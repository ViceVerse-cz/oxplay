// SPDX-License-Identifier: GPL-3.0-or-later
//! Display-driven scheduling for the single owning macOS video window.
//! CVDisplayLink is deprecated in the macOS 15 SDK, but its behavior is verified
//! against the failing native-swap path and functioning SDL3 baseline. It is
//! isolated here so a qualified CADisplayLink replacement need not alter the UI.
use crate::{MediaError, Result, Wake};
use std::{
    cell::Cell,
    ffi::c_void,
    marker::PhantomData,
    ptr::NonNull,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

#[derive(Default)]
pub(crate) struct ClockSignals {
    pub enabled: AtomicBool,
    pub ready: AtomicBool,
    pub running: AtomicBool,
    pub ticks: AtomicU64,
    pub starts: AtomicU64,
    pub stops: AtomicU64,
}

impl ClockSignals {
    /// A stopped clock cannot supply a presentation tick. Paused/control
    /// notifications must remain consumable without starting a polling clock.
    pub(crate) fn waiting_for_tick(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
            && self.running.load(Ordering::Acquire)
            && !self.ready.load(Ordering::Acquire)
    }
}

type DisplayCallback = unsafe extern "C" fn(
    *mut c_void,
    *const c_void,
    *const c_void,
    u64,
    *mut u64,
    *mut c_void,
) -> i32;
#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    fn CVDisplayLinkCreateWithActiveCGDisplays(link: *mut *mut c_void) -> i32;
    fn CVDisplayLinkSetOutputCallback(
        link: *mut c_void,
        callback: Option<DisplayCallback>,
        data: *mut c_void,
    ) -> i32;
    fn CVDisplayLinkSetCurrentCGDisplayFromOpenGLContext(
        link: *mut c_void,
        context: *mut c_void,
        format: *mut c_void,
    ) -> i32;
    fn CVDisplayLinkStart(link: *mut c_void) -> i32;
    #[cfg(feature = "native-rendering")]
    fn CVDisplayLinkSetCurrentCGDisplay(link: *mut c_void, display: u32) -> i32;
    fn CVDisplayLinkStop(link: *mut c_void) -> i32;
    fn CVDisplayLinkRelease(link: *mut c_void);
}
#[link(name = "OpenGL", kind = "framework")]
unsafe extern "C" {
    fn CGLGetCurrentContext() -> *mut c_void;
    fn CGLGetPixelFormat(context: *mut c_void) -> *mut c_void;
    fn CGLGetParameter(context: *mut c_void, parameter: u32, value: *mut i32) -> i32;
    fn CGLSetParameter(context: *mut c_void, parameter: u32, value: *const i32) -> i32;
}
// Installed SDK OpenGL/CGLTypes.h: kCGLCPSwapInterval.
const SWAP_INTERVAL: u32 = 222;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOPMAssertionCreateWithName(
        kind: *const c_void,
        level: u32,
        name: *const c_void,
        id: *mut u32,
    ) -> i32;
    fn IOPMAssertionRelease(id: u32) -> i32;
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringCreateWithCString(
        allocator: *const c_void,
        text: *const i8,
        encoding: u32,
    ) -> *const c_void;
    fn CFRelease(value: *const c_void);
}
/// Prevents *idle* display sleep only during observed playback. It never wakes
/// a sleeping screen, changes system preferences, or prevents explicit sleep.
#[derive(Default)]
struct PlaybackAssertion(Cell<Option<u32>>);
impl PlaybackAssertion {
    fn set_playing(&self, playing: bool) -> Result<()> {
        if playing && self.0.get().is_none() {
            // Installed IOPMLib.h: assertion string, level 255, uint32_t ID.
            // CoreFoundation kCFStringEncodingUTF8 = 0x08000100.
            unsafe {
                let kind = CFStringCreateWithCString(
                    std::ptr::null(),
                    c"PreventUserIdleDisplaySleep".as_ptr(),
                    0x08000100,
                );
                let name = CFStringCreateWithCString(
                    std::ptr::null(),
                    c"Oxplay video playback".as_ptr(),
                    0x08000100,
                );
                if kind.is_null() || name.is_null() {
                    if !kind.is_null() {
                        CFRelease(kind);
                    }
                    if !name.is_null() {
                        CFRelease(name);
                    }
                    return Err(MediaError(
                        "Cannot allocate playback power assertion".into(),
                    ));
                }
                let mut id = 0;
                let result = IOPMAssertionCreateWithName(kind, 255, name, &mut id);
                CFRelease(kind);
                CFRelease(name);
                checked(result, "prevent display idle sleep during playback")?;
                self.0.set(Some(id));
            }
        } else if !playing && let Some(id) = self.0.get() {
            unsafe {
                checked(IOPMAssertionRelease(id), "release playback power assertion")?;
            }
            self.0.set(None);
        }
        Ok(())
    }
}
impl Drop for PlaybackAssertion {
    fn drop(&mut self) {
        if let Err(error) = self.set_playing(false) {
            eprintln!("{error}");
        }
    }
}
fn checked(code: i32, action: &str) -> Result<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(MediaError(format!("macOS {action} failed ({code})")))
    }
}
unsafe extern "C" fn tick(
    _: *mut c_void,
    _: *const c_void,
    _: *const c_void,
    _: u64,
    _: *mut u64,
    data: *mut c_void,
) -> i32 {
    // The clock owns Arc<Wake> until Stop has joined callback work and Release
    // has destroyed the native object. No GL, mpv, or UI calls on this thread.
    let wake = unsafe { &*data.cast::<Wake>() };
    wake.clock.ticks.fetch_add(1, Ordering::Relaxed);
    if wake.frame.load(Ordering::Acquire) && !wake.clock.ready.swap(true, Ordering::AcqRel) {
        wake.notify();
    }
    0
}

/// Main-thread owner. The OpenGL comparison path also borrows its CGL context;
/// native GPU presentation associates the clock directly with the window's display.
pub(crate) struct PresentationClock {
    link: NonNull<c_void>,
    context: Option<NonNull<c_void>>,
    #[cfg(feature = "native-rendering")]
    native_display: Cell<Option<u32>>,
    previous_interval: i32,
    wake: Arc<Wake>,
    running: Cell<bool>,
    playback_assertion: PlaybackAssertion,
    _thread_bound: PhantomData<Rc<()>>,
}
impl PresentationClock {
    pub unsafe fn new(wake: Arc<Wake>) -> Result<Self> {
        let context = NonNull::new(unsafe { CGLGetCurrentContext() })
            .ok_or_else(|| MediaError("No current CGL context for presentation".into()))?;
        let mut previous_interval = 0;
        unsafe {
            checked(
                CGLGetParameter(context.as_ptr(), SWAP_INTERVAL, &mut previous_interval),
                "read swap interval",
            )?;
        }
        let clock = Self::create(wake, Some(context), previous_interval)?;
        unsafe {
            clock.update_display()?;
            checked(
                CGLSetParameter(context.as_ptr(), SWAP_INTERVAL, &0),
                "disable native swap wait",
            )?;
            let mut observed = -1;
            checked(
                CGLGetParameter(context.as_ptr(), SWAP_INTERVAL, &mut observed),
                "verify swap interval",
            )?;
            if observed != 0 {
                return Err(MediaError(
                    "macOS did not disable native swap waiting".into(),
                ));
            }
        }
        clock.wake.clock.enabled.store(true, Ordering::Release);
        Ok(clock)
    }
    /// Native GPU presentation has no CGL context or swap-interval mutation.
    #[cfg(feature = "native-rendering")]
    pub fn new_native(wake: Arc<Wake>, display: Option<u32>) -> Result<Self> {
        let clock = Self::create(wake, None, 0)?;
        if let Some(display) = display {
            clock.set_native_display(display)?;
        }
        clock.wake.clock.enabled.store(true, Ordering::Release);
        Ok(clock)
    }
    fn create(
        wake: Arc<Wake>,
        context: Option<NonNull<c_void>>,
        previous_interval: i32,
    ) -> Result<Self> {
        let mut link = std::ptr::null_mut();
        unsafe {
            checked(
                CVDisplayLinkCreateWithActiveCGDisplays(&mut link),
                "create display clock",
            )?;
        }
        let link = NonNull::new(link)
            .ok_or_else(|| MediaError("macOS returned an empty display clock".into()))?;
        let clock = Self {
            link,
            context,
            #[cfg(feature = "native-rendering")]
            native_display: Cell::new(None),
            previous_interval,
            wake,
            running: Cell::new(false),
            playback_assertion: PlaybackAssertion::default(),
            _thread_bound: PhantomData,
        };
        unsafe {
            checked(
                CVDisplayLinkSetOutputCallback(
                    link.as_ptr(),
                    Some(tick),
                    Arc::as_ptr(&clock.wake).cast_mut().cast(),
                ),
                "set display callback",
            )?;
        }
        Ok(clock)
    }
    #[cfg(feature = "native-rendering")]
    pub fn set_native_display(&self, display: u32) -> Result<()> {
        if self.context.is_some() || self.native_display.get() == Some(display) {
            return Ok(());
        }
        unsafe {
            checked(
                CVDisplayLinkSetCurrentCGDisplay(self.link.as_ptr(), display),
                "associate native display clock",
            )?;
        }
        self.native_display.set(Some(display));
        Ok(())
    }
    /// Must run on the UI thread with the owning context current, after resize.
    pub unsafe fn update_display(&self) -> Result<()> {
        let Some(context) = self.context else {
            return Ok(());
        };
        unsafe {
            checked(
                CVDisplayLinkSetCurrentCGDisplayFromOpenGLContext(
                    self.link.as_ptr(),
                    context.as_ptr(),
                    CGLGetPixelFormat(context.as_ptr()),
                ),
                "associate display clock",
            )
        }
    }
    /// UI-thread service after a coalesced wake. Starting/stopping a native
    /// display clock never waits for a future frame or performs rendering.
    pub fn service(&self, playing: bool) -> Result<()> {
        self.playback_assertion.set_playing(playing)?;
        if !playing {
            // Paused seeks may deliver one final frame after the clock stops.
            // Preserve readiness without a display callback or background loop.
            self.stop()?;
            self.wake
                .clock
                .ready
                .store(self.wake.frame.load(Ordering::Acquire), Ordering::Release);
            return Ok(());
        }
        // Keep phase while the engine reports Playing. A brief gap in engine
        // notifications is not an idle state: stopping/restarting CVDisplayLink
        // loses phase under scheduling jitter. The callback does not poll mpv
        // or wake/redraw the UI unless an engine notification is pending.
        if self.wake.frame.load(Ordering::Acquire)
            && !self.running.get()
            && !self.wake.clock.ready.load(Ordering::Acquire)
        {
            unsafe {
                checked(
                    CVDisplayLinkStart(self.link.as_ptr()),
                    "start display clock",
                )?;
            }
            self.running.set(true);
            self.wake.clock.running.store(true, Ordering::Release);
            self.wake.clock.starts.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }
    pub fn prevents_display_sleep(&self) -> bool {
        self.playback_assertion.0.get().is_some()
    }
    fn stop(&self) -> Result<()> {
        if self.running.get() {
            unsafe {
                checked(CVDisplayLinkStop(self.link.as_ptr()), "stop display clock")?;
            }
            self.running.set(false);
            self.wake.clock.running.store(false, Ordering::Release);
            self.wake.clock.ready.store(false, Ordering::Release);
            self.wake.clock.stops.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }
}
impl Drop for PresentationClock {
    fn drop(&mut self) {
        self.wake.clock.enabled.store(false, Ordering::Release);
        if let Err(error) = self.stop() {
            eprintln!("{error}");
        }
        unsafe {
            // Release the native callback source before dropping its Arc data.
            CVDisplayLinkRelease(self.link.as_ptr());
            if let Some(context) = self.context {
                debug_assert_eq!(CGLGetCurrentContext(), context.as_ptr());
                let result =
                    CGLSetParameter(context.as_ptr(), SWAP_INTERVAL, &self.previous_interval);
                if result != 0 {
                    eprintln!("macOS restore swap interval failed ({result})");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wake() -> Arc<Wake> {
        Arc::new(Wake {
            queued: AtomicBool::new(false),
            frame: AtomicBool::new(false),
            due: AtomicBool::new(false),
            count: AtomicU64::new(0),
            render_count: AtomicU64::new(0),
            timing_origin: None,
            callback_time_ns: AtomicU64::new(0),
            clock: ClockSignals::default(),
            callback: Box::new(|| {}),
        })
    }
    fn pulse(wake: &Wake) {
        unsafe {
            tick(
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                std::ptr::from_ref(wake).cast_mut().cast(),
            );
        }
    }
    #[test]
    fn display_ticks_without_pending_media_never_wake_or_mark_ready() {
        let wake = wake();
        wake.clock.enabled.store(true, Ordering::Release);
        wake.clock.running.store(true, Ordering::Release);
        for _ in 0..600 {
            pulse(&wake);
        }
        assert_eq!(wake.count.load(Ordering::Relaxed), 0);
        assert!(!wake.clock.ready.load(Ordering::Acquire));
        assert_eq!(wake.clock.ticks.load(Ordering::Relaxed), 600);
        // The first real engine notification still produces exactly one wake
        // on the following display tick, without a clock restart.
        unsafe {
            crate::render_wake(Arc::as_ptr(&wake).cast_mut().cast());
        }
        pulse(&wake);
        assert_eq!(wake.count.load(Ordering::Relaxed), 1);
        assert!(wake.clock.ready.load(Ordering::Acquire));
    }
    #[test]
    fn stopped_clock_admits_a_paused_render_notification_without_a_tick() {
        let wake = wake();
        wake.clock.enabled.store(true, Ordering::Release);
        unsafe { crate::render_wake(Arc::as_ptr(&wake).cast_mut().cast()) };
        assert!(wake.frame.load(Ordering::Acquire));
        assert_eq!(wake.count.load(Ordering::Relaxed), 1);
        assert!(!wake.clock.waiting_for_tick());
        // Active playback still waits for its real display opportunity.
        wake.clock.running.store(true, Ordering::Release);
        assert!(wake.clock.waiting_for_tick());
        pulse(&wake);
        assert!(!wake.clock.waiting_for_tick());
        // Stopping before a subsequent tick must never strand that frame.
        wake.clock.ready.store(false, Ordering::Release);
        wake.clock.running.store(false, Ordering::Release);
        assert!(!wake.clock.waiting_for_tick());
    }
    #[test]
    fn active_clock_coalesces_engine_and_display_wakeups() {
        let wake = wake();
        wake.clock.enabled.store(true, Ordering::Release);
        wake.clock.running.store(true, Ordering::Release);
        unsafe {
            crate::render_wake(Arc::as_ptr(&wake).cast_mut().cast());
        }
        assert!(wake.frame.load(Ordering::Acquire));
        assert_eq!(wake.count.load(Ordering::Relaxed), 0);
        pulse(&wake);
        assert!(wake.clock.ready.load(Ordering::Acquire));
        assert_eq!(wake.count.load(Ordering::Relaxed), 1);
        wake.queued.store(false, Ordering::Release);
        for _ in 0..20 {
            pulse(&wake);
        }
        assert_eq!(wake.count.load(Ordering::Relaxed), 1);
        // Consuming a frame permits exactly one wake for the next frame.
        wake.clock.ready.store(false, Ordering::Release);
        pulse(&wake);
        assert_eq!(wake.count.load(Ordering::Relaxed), 2);
    }
}
