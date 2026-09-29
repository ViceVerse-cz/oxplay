// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit offline-fixture evidence only. One log per admitted generation;
//! no timer, model change, image allocation, or production browsing activity.
use crate::{App, LibraryUi, UiState, thumbnails::Statistics};
use slint::{ComponentHandle, Model};
use std::cell::Cell;

#[derive(Default)]
pub struct State {
    reported_generation: Cell<u64>,
    trace_events: Cell<u64>,
}

/// Fixed event kinds only, and only for the explicitly admitted synthetic
/// fixture. Pointer motion, text, paths, URLs and provider data are never logged.
/// A finite event cap prevents a long interactive diagnostic growing its log.
pub fn trace(state: &UiState, kind: &'static str) {
    if state.library_fixture.is_none() {
        return;
    }
    let count = state
        .fixture_quiescence
        .trace_events
        .get()
        .saturating_add(1);
    state.fixture_quiescence.trace_events.set(count);
    if count > 256 {
        return;
    }
    eprintln!(
        "library fixture event: elapsed_ms={} sequence={} kind={} hidden={} catalog_changes={} catalog_resets={} library_notifications={:?}",
        state.started.elapsed().as_millis(),
        count,
        kind,
        state.hidden.get(),
        state.model.changes.get(),
        state.model.resets.get(),
        crate::library_ui::notification_counts(state)
    );
    if count == 256 {
        eprintln!("library fixture event trace capped; later event chronology unavailable");
    }
}

pub fn window_event(state: &UiState, event: &slint::winit_030::winit::event::WindowEvent) {
    if state.library_fixture.is_none() {
        return;
    }
    use slint::winit_030::winit::event::{ElementState, WindowEvent};
    let kind = match event {
        WindowEvent::Focused(true) => "focus-gained",
        WindowEvent::Focused(false) => "focus-lost",
        WindowEvent::Occluded(true) => "occluded",
        WindowEvent::Occluded(false) => "exposed",
        WindowEvent::Resized(_) => "resized",
        WindowEvent::ScaleFactorChanged { .. } => "scale-changed",
        WindowEvent::MouseInput {
            state: ElementState::Pressed,
            ..
        } => "pointer-button-pressed",
        WindowEvent::MouseWheel { .. } => "wheel-input",
        WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
            "key-pressed"
        }
        _ => return,
    };
    trace(state, kind);
}
impl State {
    fn unreported(&self, generation: u64) -> bool {
        generation != 0 && generation > self.reported_generation.get()
    }
    fn awaits(&self, generation: u64, stats: Statistics) -> bool {
        self.unreported(generation)
            && generation == stats.admitted_generation
            && stats.pending == 0
            && stats.inflight == 0
            && stats.ready == 0
            && stats.remote_started == 0
    }
    fn take(&self, generation: u64, stats: Statistics, admitted: usize, ready: usize) -> bool {
        if !self.awaits(generation, stats) || admitted == 0 || admitted > 40 || ready != admitted {
            return false;
        }
        self.reported_generation.set(generation);
        true
    }
}

fn visible_is_admitted(first: usize, end: usize, visible_first: usize, visible_end: usize) -> bool {
    first <= visible_first && visible_first < visible_end && visible_end <= end
}

pub fn observe(app: &App, state: &UiState) {
    if state.library_fixture.is_none() {
        return;
    }
    // Already-reported generations do not read lazy UI geometry or scan rows
    // on subsequent BeforeRendering callbacks. The separate statement drops
    // this borrow before any getter can notify a new thumbnail admission.
    let generation = state.thumbnails.borrow().generation();
    if !state.fixture_quiescence.unreported(generation)
        || app.get_page() != 0
        || app.global::<LibraryUi>().get_busy()
    {
        return;
    }
    // Evaluate view bindings before capturing the worker's current admission;
    // evaluating a dirty geometry binding must not leave a stale range paired
    // with a newly submitted generation in the diagnostic.
    let visible_first = app.get_feed_thumbnail_first().max(0) as usize;
    let visible_end = app.get_feed_thumbnail_end().max(0) as usize;
    let (first, end) = state.thumbnail_range.get();
    let rows = state.model.row_count();
    if rows == 0
        || rows > 100
        || first >= end
        || end > rows
        || end - first > 40
        || !visible_is_admitted(first, end, visible_first, visible_end)
    {
        return;
    }
    let (generation, stats) = {
        let worker = state.thumbnails.borrow();
        (worker.generation(), worker.statistics())
    };
    if !state.fixture_quiescence.awaits(generation, stats) {
        return;
    }
    let near_ready = (first..end)
        .filter(|row| {
            state
                .model
                .row_data(*row)
                .is_some_and(|row| row.thumbnail_ready)
        })
        .count();
    if !state
        .fixture_quiescence
        .take(generation, stats, end - first, near_ready)
    {
        return;
    }
    let visible_ready = (visible_first..visible_end)
        .filter(|row| {
            state
                .model
                .row_data(*row)
                .is_some_and(|row| row.thumbnail_ready)
        })
        .count();
    eprintln!(
        "library fixture quiescent: elapsed_ms={} generation={} model_rows={} near_viewport_first={} near_viewport_end={} near_viewport_ready={} viewport_intersecting_ready_thumbnails={} hidden={} pending={} inflight={} ready={} remote_started={} started={} decoded={} failed={} published={} published_bytes={} catalog_changes={} catalog_resets={} group_child_changes={} library_notifications={:?} compositor_visibility=not_measured",
        state.started.elapsed().as_millis(),
        generation,
        rows,
        first,
        end,
        near_ready,
        visible_ready,
        state.hidden.get(),
        stats.pending,
        stats.inflight,
        stats.ready,
        stats.remote_started,
        stats.started,
        stats.decoded,
        stats.failed,
        stats.published,
        stats.published_bytes,
        state.model.changes.get(),
        state.model.resets.get(),
        state.groups.child_changes.get(),
        crate::library_ui::notification_counts(state),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    fn settled(generation: u64) -> Statistics {
        Statistics {
            admitted_generation: generation,
            ..Statistics::default()
        }
    }
    #[test]
    fn logs_only_once_after_current_admission_worker_drain_and_all_rows_are_ready() {
        let state = State::default();
        assert!(!state.take(0, settled(0), 6, 6));
        assert!(!state.take(2, settled(1), 6, 6));
        assert!(!state.take(2, settled(2), 6, 5));
        for field in ["pending", "inflight", "ready", "remote"] {
            let mut stats = settled(2);
            match field {
                "pending" => stats.pending = 1,
                "inflight" => stats.inflight = 1,
                "ready" => stats.ready = 1,
                _ => stats.remote_started = 1,
            }
            assert!(!state.take(2, stats, 6, 6));
        }
        assert!(state.take(2, settled(2), 6, 6));
        assert!(!state.take(2, settled(2), 6, 6));
        assert!(!state.take(1, settled(1), 6, 6));
        assert!(state.take(3, settled(3), 9, 9));
    }
    #[test]
    fn empty_oversized_or_partially_published_viewports_do_not_consume_the_generation() {
        let state = State::default();
        for (admitted, ready) in [(0, 0), (41, 41), (9, 8), (9, 10)] {
            assert!(!state.take(1, settled(1), admitted, ready));
        }
        assert!(state.take(1, settled(1), 40, 40));
    }
    #[test]
    fn visible_range_must_be_nonempty_and_contained_before_consuming_the_marker() {
        for (first, end) in [(0, 0), (3, 2), (0, 5), (3, 13)] {
            assert!(!visible_is_admitted(3, 12, first, end));
        }
        assert!(visible_is_admitted(3, 12, 3, 12));
        assert!(visible_is_admitted(3, 12, 6, 9));
        let state = State::default();
        assert!(state.unreported(1));
        // Early layout rejection leaves a later completion/render observation
        // eligible; it never consumes the one admitted-generation marker.
        if visible_is_admitted(0, 9, 0, 0) {
            assert!(state.take(1, settled(1), 9, 9));
        }
        assert!(state.take(1, settled(1), 9, 9));
        assert!(!state.unreported(1));
        assert!(state.unreported(2));
    }
}
