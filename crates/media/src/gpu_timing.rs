// SPDX-License-Identifier: GPL-3.0-or-later
//! Diagnostic-only GPU timeline queries. No query/result wait, glFinish, timer,
//! redraw request, or pixel readback. See ARB_timer_query revision13.
use glow::HasContext;
use std::{
    collections::VecDeque,
    ffi::{CStr, c_void},
    rc::Rc,
};
const SLOTS: usize = 16;
const GPU_DISJOINT_EXT: u32 = 0x8FBB;
type GetQuery = unsafe extern "system" fn(u32, u32, *mut i32);
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Timestamps,
    UiElapsed,
}
#[derive(Debug, Default, Clone, Copy)]
pub struct GpuTimingStats {
    pub requested: bool,
    pub supported: bool,
    pub query_api_available: bool,
    pub counter_bits: u32,
    pub elapsed_counter_bits: u32,
    pub ui_elapsed_requested: bool,
    pub ui_elapsed_samples: u64,
    pub ui_elapsed_ns: u64,
    pub max_ui_elapsed_ns: u64,
    /// Any foreign elapsed query or missing matching end disables sampling.
    pub elapsed_conflicts: u64,
    pub allocation_failed: bool,
    pub disjoint_supported: bool,
    pub frame_samples: u64,
    pub media_samples: u64,
    pub frame_ns: u64,
    pub media_ns: u64,
    pub ui_after_media_ns: u64,
    pub max_frame_ns: u64,
    pub max_media_ns: u64,
    pub max_ui_after_media_ns: u64,
    pub dropped_samples: u64,
    pub invalid_samples: u64,
    pub disjoint_events: u64,
    pub pending_samples: u32,
}
struct Slot {
    queries: Vec<glow::NativeQuery>,
    media: bool,
}
pub(super) struct GpuTiming {
    gl: Rc<glow::Context>,
    slots: Vec<Slot>,
    free: Vec<usize>,
    pending: VecDeque<usize>,
    active: Option<usize>,
    mode: Mode,
    get_query: Option<GetQuery>,
    stats: GpuTimingStats,
}
impl GpuTiming {
    /// Owning desktop GL >=3.3 context must be current through Drop.
    pub unsafe fn new(
        gl: Rc<glow::Context>,
        get_proc: &dyn Fn(&CStr) -> *const c_void,
        ui_elapsed: bool,
    ) -> Self {
        let mut timer = Self {
            gl,
            slots: Vec::with_capacity(SLOTS),
            free: Vec::with_capacity(SLOTS),
            pending: VecDeque::with_capacity(SLOTS),
            active: None,
            mode: if ui_elapsed {
                Mode::UiElapsed
            } else {
                Mode::Timestamps
            },
            get_query: None,
            stats: GpuTimingStats {
                requested: true,
                ui_elapsed_requested: ui_elapsed,
                ..Default::default()
            },
        };
        // glow0.18 exposes GetQueryObject but not GetQueryiv(target,...).
        let function = get_proc(c"glGetQueryiv");
        if function.is_null() {
            return timer;
        }
        timer.stats.query_api_available = true;
        let query_bits: GetQuery = unsafe { std::mem::transmute(function) };
        timer.get_query = Some(query_bits);
        let mut bits = 0;
        unsafe {
            query_bits(glow::TIMESTAMP, glow::QUERY_COUNTER_BITS, &mut bits);
        }
        timer.stats.counter_bits = bits.max(0) as u32;
        // Do not surround libmpv with TIME_ELAPSED:
        // the engine uses that non-nestable query target internally.
        let mut elapsed_bits = 0;
        unsafe {
            query_bits(
                glow::TIME_ELAPSED,
                glow::QUERY_COUNTER_BITS,
                &mut elapsed_bits,
            );
        }
        timer.stats.elapsed_counter_bits = elapsed_bits.max(0) as u32;
        // ARB permits zero useful bits. At least30 is required when nonzero.
        let selected_bits = if ui_elapsed { elapsed_bits } else { bits };
        if !(30..=64).contains(&selected_bits) {
            return timer;
        }
        timer.stats.disjoint_supported = timer
            .gl
            .supported_extensions()
            .contains("GL_EXT_disjoint_timer_query");
        for index in 0..SLOTS {
            let count = if ui_elapsed { 1 } else { 4 };
            let mut queries = Vec::with_capacity(count);
            for _ in 0..count {
                match unsafe { timer.gl.create_query() } {
                    Ok(q) => queries.push(q),
                    Err(_) => {
                        timer.stats.allocation_failed = true;
                        for q in queries {
                            unsafe { timer.gl.delete_query(q) }
                        }
                        return timer;
                    }
                }
            }
            timer.slots.push(Slot {
                queries,
                media: false,
            });
            timer.free.push(index);
        }
        timer.stats.supported = true;
        timer
    }
    pub fn stats(&self) -> GpuTimingStats {
        let mut stats = self.stats;
        stats.pending_samples = (self.pending.len() + usize::from(self.active.is_some())) as u32;
        stats
    }
    pub unsafe fn begin_frame(&mut self) {
        if self.mode == Mode::UiElapsed {
            // A missing AfterRendering must never carry an elapsed query across
            // the following mpv render. End only our own query, then disable.
            if self.active.is_some() {
                unsafe { self.cancel_elapsed() };
                self.stats.elapsed_conflicts += 1;
                self.stats.supported = false;
            }
            if self.stats.supported {
                unsafe { self.collect() };
            }
            return;
        }
        if !self.stats.supported {
            return;
        }
        unsafe {
            self.collect();
        }
        if let Some(index) = self.active.take() {
            self.stats.dropped_samples += 1;
            self.free.push(index);
        }
        let Some(index) = self.free.pop() else {
            self.stats.dropped_samples += 1;
            return;
        };
        self.slots[index].media = false;
        unsafe {
            self.gl
                .query_counter(self.slots[index].queries[0], glow::TIMESTAMP);
        }
        self.active = Some(index);
    }
    pub unsafe fn begin_media(&mut self) {
        if self.mode != Mode::Timestamps {
            return;
        }
        if let Some(index) = self.active {
            unsafe {
                self.gl
                    .query_counter(self.slots[index].queries[1], glow::TIMESTAMP);
            }
        }
    }
    pub unsafe fn end_media(&mut self) {
        if self.mode != Mode::Timestamps {
            return;
        }
        if let Some(index) = self.active {
            unsafe {
                self.gl
                    .query_counter(self.slots[index].queries[2], glow::TIMESTAMP);
            }
            self.slots[index].media = true;
        }
    }
    pub unsafe fn end_frame(&mut self) {
        if self.mode == Mode::UiElapsed {
            if let Some(index) = self.active.take() {
                let query = self.slots[index].queries[0];
                if owns_elapsed(query.0.get(), unsafe { self.current_elapsed() }) {
                    unsafe { self.gl.end_query(glow::TIME_ELAPSED) };
                    self.pending.push_back(index);
                } else {
                    // A foreign renderer changed the non-nestable target. Never
                    // end its query or treat the incomplete interval as ours.
                    self.stats.elapsed_conflicts += 1;
                    self.stats.invalid_samples += 1;
                    self.stats.supported = false;
                    self.free.push(index);
                }
            }
            return;
        }
        if let Some(index) = self.active.take() {
            unsafe {
                self.gl
                    .query_counter(self.slots[index].queries[3], glow::TIMESTAMP);
            }
            self.pending.push_back(index);
        }
        // Only existing frame callbacks collect results; paused/hidden windows
        // never acquire a background polling loop to finish diagnostics.
    }
    /// Called at the end of BeforeRendering, after mpv and state restoration,
    /// on both media and UI-only frames. Does nothing in timestamp mode.
    pub unsafe fn begin_ui(&mut self) {
        if self.mode != Mode::UiElapsed || !self.stats.supported {
            return;
        }
        if unsafe { self.current_elapsed() } != 0 {
            self.stats.elapsed_conflicts += 1;
            self.stats.supported = false;
            return;
        }
        let Some(index) = self.free.pop() else {
            self.stats.dropped_samples += 1;
            return;
        };
        unsafe {
            self.gl
                .begin_query(glow::TIME_ELAPSED, self.slots[index].queries[0])
        };
        self.active = Some(index);
    }
    unsafe fn current_elapsed(&self) -> u32 {
        let mut current = 0;
        if let Some(get_query) = self.get_query {
            unsafe { get_query(glow::TIME_ELAPSED, glow::CURRENT_QUERY, &mut current) };
        }
        current as u32
    }
    unsafe fn cancel_elapsed(&mut self) {
        if let Some(index) = self.active.take() {
            if owns_elapsed(self.slots[index].queries[0].0.get(), unsafe {
                self.current_elapsed()
            }) {
                unsafe { self.gl.end_query(glow::TIME_ELAPSED) };
            }
            self.stats.invalid_samples += 1;
            self.free.push(index);
        }
    }
    unsafe fn collect(&mut self) {
        if self.stats.disjoint_supported
            && unsafe { self.gl.get_parameter_i32(GPU_DISJOINT_EXT) } != 0
        {
            self.stats.disjoint_events += 1;
            self.stats.invalid_samples += self.pending.len() as u64;
            self.free.extend(self.pending.drain(..));
            return;
        }
        let previous = self.stats;
        let mut collected = 0u64;
        while let Some(&index) = self.pending.front() {
            let slot = &self.slots[index];
            let final_query = slot.queries[if self.mode == Mode::UiElapsed { 0 } else { 3 }];
            // A later timestamp being available guarantees all preceding
            // timestamps are available (ARB_timer_query lines235–238).
            if unsafe {
                self.gl
                    .get_query_parameter_u32(final_query, glow::QUERY_RESULT_AVAILABLE)
            } == 0
            {
                break;
            }
            if self.mode == Mode::UiElapsed {
                let value = unsafe {
                    self.gl
                        .get_query_parameter_u64(final_query, glow::QUERY_RESULT)
                };
                if valid_elapsed_result(value, self.stats.elapsed_counter_bits) {
                    self.stats.ui_elapsed_samples += 1;
                    self.stats.ui_elapsed_ns = self.stats.ui_elapsed_ns.saturating_add(value);
                    self.stats.max_ui_elapsed_ns = self.stats.max_ui_elapsed_ns.max(value);
                } else {
                    self.stats.invalid_samples += 1;
                }
                self.pending.pop_front();
                self.free.push(index);
                collected += 1;
                continue;
            }
            let start = unsafe {
                self.gl
                    .get_query_parameter_u64(slot.queries[0], glow::QUERY_RESULT)
            };
            let end = unsafe {
                self.gl
                    .get_query_parameter_u64(slot.queries[3], glow::QUERY_RESULT)
            };
            let frame_delta = elapsed(start, end, self.stats.counter_bits);
            let media = if slot.media {
                let a = unsafe {
                    self.gl
                        .get_query_parameter_u64(slot.queries[1], glow::QUERY_RESULT)
                };
                let b = unsafe {
                    self.gl
                        .get_query_parameter_u64(slot.queries[2], glow::QUERY_RESULT)
                };
                elapsed(a, b, self.stats.counter_bits).zip(elapsed(b, end, self.stats.counter_bits))
            } else {
                Some((0, frame_delta.unwrap_or(0)))
            };
            if let (Some(frame), Some((mpv, ui))) = (frame_delta, media) {
                if mpv <= frame && ui <= frame && mpv.saturating_add(ui) <= frame {
                    self.stats.frame_samples += 1;
                    self.stats.frame_ns = self.stats.frame_ns.saturating_add(frame);
                    self.stats.max_frame_ns = self.stats.max_frame_ns.max(frame);
                    if slot.media {
                        self.stats.media_samples += 1;
                        self.stats.media_ns = self.stats.media_ns.saturating_add(mpv);
                        self.stats.max_media_ns = self.stats.max_media_ns.max(mpv);
                    }
                    self.stats.ui_after_media_ns = self.stats.ui_after_media_ns.saturating_add(ui);
                    self.stats.max_ui_after_media_ns = self.stats.max_ui_after_media_ns.max(ui);
                } else {
                    self.stats.invalid_samples += 1;
                }
            } else {
                self.stats.invalid_samples += 1;
            }
            self.pending.pop_front();
            self.free.push(index);
            collected += 1;
        }
        if self.stats.disjoint_supported
            && unsafe { self.gl.get_parameter_i32(GPU_DISJOINT_EXT) } != 0
        {
            // Discontinuity during result collection invalidates the batch too.
            self.stats = previous;
            self.stats.disjoint_events += 1;
            self.stats.invalid_samples += collected + self.pending.len() as u64;
            self.free.extend(self.pending.drain(..));
        }
    }
}
impl Drop for GpuTiming {
    fn drop(&mut self) {
        if self.mode == Mode::UiElapsed {
            unsafe { self.cancel_elapsed() };
        }
        let current = if self.mode == Mode::UiElapsed {
            unsafe { self.current_elapsed() }
        } else {
            0
        };
        for slot in &self.slots {
            for &q in &slot.queries {
                // Even an unexpected external reuse of one of our names must
                // not end its active query. Context destruction reclaims it.
                if q.0.get() == current {
                    continue;
                }
                unsafe {
                    self.gl.delete_query(q);
                }
            }
        }
    }
}
fn owns_elapsed(owned: u32, current: u32) -> bool {
    owned != 0 && owned == current
}
fn valid_elapsed_result(value: u64, bits: u32) -> bool {
    (30..=64).contains(&bits) && value < 500_000_000
}
fn elapsed(start: u64, end: u64, bits: u32) -> Option<u64> {
    if !(30..=64).contains(&bits) {
        return None;
    }
    let delta = if bits == 64 {
        end.checked_sub(start)?
    } else {
        end.wrapping_sub(start) & ((1u64 << bits) - 1)
    };
    // Reject stalls/discontinuities >=0.5s rather than treating wrap/reset as
    // valid performance data. Core timers have no universal disjoint signal.
    (delta < 500_000_000).then_some(delta)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    #[derive(Default)]
    struct Mock {
        next: u32,
        current: u32,
        ended: Vec<u32>,
        deleted: Vec<u32>,
        ready: bool,
        reads: usize,
        invalid_gl: bool,
    }
    thread_local! { static MOCK: RefCell<Mock> = RefCell::default(); }
    unsafe extern "system" fn get_string(_: u32) -> *const u8 {
        c"3.3 mock".as_ptr().cast()
    }
    unsafe extern "system" fn get_integer(_: u32, value: *mut i32) {
        unsafe { *value = 0 };
    }
    unsafe extern "system" fn get_query(target: u32, parameter: u32, value: *mut i32) {
        let result = if parameter == glow::CURRENT_QUERY {
            MOCK.with_borrow(|m| m.current as i32)
        } else if target == glow::TIME_ELAPSED {
            32
        } else {
            0
        };
        unsafe { *value = result };
    }
    unsafe extern "system" fn generate(count: i32, names: *mut u32) {
        MOCK.with_borrow_mut(|m| {
            for index in 0..count {
                m.next += 1;
                unsafe { *names.add(index as usize) = m.next };
            }
        });
    }
    unsafe extern "system" fn delete(count: i32, names: *const u32) {
        MOCK.with_borrow_mut(|m| {
            for index in 0..count {
                let query = unsafe { *names.add(index as usize) };
                m.invalid_gl |= query == m.current;
                m.deleted.push(query);
            }
        });
    }
    unsafe extern "system" fn begin(_: u32, name: u32) {
        MOCK.with_borrow_mut(|m| {
            m.invalid_gl |= m.current != 0;
            m.current = name;
        });
    }
    unsafe extern "system" fn end(_: u32) {
        MOCK.with_borrow_mut(|m| {
            m.invalid_gl |= m.current == 0;
            m.ended.push(m.current);
            m.current = 0;
        });
    }
    unsafe extern "system" fn available(_: u32, _: u32, value: *mut u32) {
        unsafe { *value = MOCK.with_borrow(|m| u32::from(m.ready)) };
    }
    unsafe extern "system" fn result(_: u32, _: u32, value: *mut u64) {
        MOCK.with_borrow_mut(|m| {
            m.invalid_gl |= !m.ready;
            m.reads += 1;
        });
        unsafe { *value = 12_345 };
    }
    fn loader(name: &CStr) -> *const c_void {
        match name.to_bytes() {
            b"glGetString" => get_string as *const c_void,
            b"glGetIntegerv" => get_integer as *const c_void,
            b"glGetQueryiv" => get_query as *const c_void,
            b"glGenQueries" => generate as *const c_void,
            b"glDeleteQueries" => delete as *const c_void,
            b"glBeginQuery" => begin as *const c_void,
            b"glEndQuery" => end as *const c_void,
            b"glGetQueryObjectuiv" => available as *const c_void,
            b"glGetQueryObjectui64v" => result as *const c_void,
            _ => std::ptr::null(),
        }
    }
    fn timer(ui_elapsed: bool) -> GpuTiming {
        MOCK.with_borrow_mut(|m| *m = Mock::default());
        let gl = unsafe { glow::Context::from_loader_function_cstr(loader) };
        unsafe { GpuTiming::new(Rc::new(gl), &loader, ui_elapsed) }
    }
    #[test]
    fn timestamp_delta_handles_bounded_wrap_and_rejects_invalid_intervals() {
        assert_eq!(elapsed(100, 160, 64), Some(60));
        assert_eq!(elapsed((1 << 30) - 5, 5, 30), Some(10));
        assert_eq!(elapsed(160, 100, 64), None);
        assert_eq!(elapsed(0, 500_000_000, 64), None);
        assert_eq!(elapsed(0, 0, 0), None);
    }
    #[test]
    fn elapsed_ring_never_waits_or_wraps_media_and_keeps_separate_totals() {
        let mut timer = timer(true);
        for _ in 0..SLOTS + 1 {
            unsafe {
                timer.begin_frame();
                timer.begin_media();
                assert_eq!(MOCK.with_borrow(|m| m.current), 0);
                timer.end_media();
                timer.begin_ui();
                timer.end_frame();
            }
        }
        assert_eq!(timer.stats().pending_samples as usize, SLOTS);
        assert_eq!(timer.stats().dropped_samples, 1);
        assert_eq!(MOCK.with_borrow(|m| m.reads), 0);
        MOCK.with_borrow_mut(|m| m.ready = true);
        unsafe { timer.begin_frame() };
        let stats = timer.stats();
        assert_eq!(stats.ui_elapsed_samples, SLOTS as u64);
        assert_eq!(stats.ui_elapsed_ns, SLOTS as u64 * 12_345);
        assert_eq!(
            stats.frame_samples + stats.media_samples + stats.frame_ns + stats.media_ns,
            0
        );
        drop(timer);
        MOCK.with_borrow(|m| {
            assert!(!m.invalid_gl);
            assert_eq!(m.deleted.len(), SLOTS);
        });
    }
    #[test]
    fn missing_after_render_and_drop_end_only_owned_query() {
        let mut timer = timer(true);
        unsafe {
            timer.begin_frame();
            timer.begin_ui();
            timer.begin_frame();
        }
        assert!(!timer.stats().supported);
        assert_eq!(timer.stats().elapsed_conflicts, 1);
        assert_eq!(MOCK.with_borrow(|m| m.current), 0);
        drop(timer);
        let mut timer = self::timer(true);
        unsafe { timer.begin_ui() };
        drop(timer);
        MOCK.with_borrow(|m| {
            assert_eq!(m.current, 0);
            assert_eq!(m.ended.len(), 1);
            assert!(!m.invalid_gl);
        });
    }
    #[test]
    fn foreign_elapsed_queries_are_never_nested_ended_or_deleted() {
        let mut timer = timer(true);
        MOCK.with_borrow_mut(|m| m.current = 999);
        unsafe { timer.begin_ui() };
        assert!(!timer.stats().supported);
        drop(timer);
        MOCK.with_borrow(|m| {
            assert_eq!(m.current, 999);
            assert!(m.ended.is_empty());
            assert!(!m.deleted.contains(&999));
            assert!(!m.invalid_gl);
        });
        let mut timer = self::timer(true);
        unsafe { timer.begin_ui() };
        MOCK.with_borrow_mut(|m| m.current = 999);
        unsafe { timer.end_frame() };
        assert_eq!(timer.stats().invalid_samples, 1);
        drop(timer);
        MOCK.with_borrow(|m| {
            assert_eq!(m.current, 999);
            assert!(m.ended.is_empty());
            assert!(!m.invalid_gl);
        });
    }
    #[test]
    fn elapsed_mode_is_explicit_and_long_or_unsupported_results_are_rejected() {
        let timer = timer(false);
        assert!(!timer.stats().supported);
        assert!(!timer.stats().ui_elapsed_requested);
        assert_eq!(MOCK.with_borrow(|m| m.next), 0);
        assert!(valid_elapsed_result(12_345, 32));
        assert!(!valid_elapsed_result(500_000_000, 32));
        assert!(!valid_elapsed_result(12_345, 0));
    }
}
