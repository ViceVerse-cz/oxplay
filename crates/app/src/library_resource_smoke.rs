// SPDX-License-Identifier: GPL-3.0-or-later
//! Finite, explicitly selected offline fixture traversal. Never used by settled
//! library resource runs; it does not synthesize provider or account results.
use crate::{App, LibraryUi, UiState};
use slint::{ComponentHandle, Model};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

const STAGES: [u64; 17] = [
    5, 6, 11, 12, 17, 18, 23, 24, 29, 30, 31, 32, 34, 36, 38, 40, 42,
];
const PAGE_STAGES: usize = 10;
const PAGES: [usize; 5] = [0, 1, 2, 1, 0];
const PAGE_SIZE: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Counters {
    catalog: (u64, u64),
    groups: (u64, u64, u64),
    library: (u64, u64),
    assignments: u64,
    redraw_requests: u64,
    thumbnail_work: (u64, u64, u64, u64, u64),
}
impl Counters {
    fn capture(state: &UiState) -> Self {
        let images = state.thumbnails.borrow().statistics();
        Self {
            catalog: (state.model.changes.get(), state.model.resets.get()),
            groups: (
                state.groups.model.changes.get(),
                state.groups.model.resets.get(),
                state.groups.child_changes.get(),
            ),
            library: crate::library_ui::notification_counts(state),
            assignments: state.ui_assignments.get(),
            redraw_requests: state.redraw_requests.get(),
            thumbnail_work: (
                images.started,
                images.remote_started,
                images.decoded,
                images.published,
                images.published_bytes,
            ),
        }
    }
}

#[derive(Default)]
struct Pages {
    // Three bounded acknowledged pages, never the entire 10,000-row library.
    ids: [Option<Vec<slint::SharedString>>; 3],
}

#[derive(Default)]
struct Keyboard {
    original_size: Option<slint::PhysicalSize>,
    original_columns: i32,
    page_down_target: usize,
}
impl Keyboard {
    fn key(app: &App, key: slint::platform::Key) {
        let text: slint::SharedString = key.into();
        app.window()
            .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
        app.window()
            .dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
    }
    fn focused(app: &App, state: &UiState, index: usize) -> Result<(), &'static str> {
        if app.get_feed_focused_index() != index as i32
            || app.get_feed_parking_active()
            || index < app.get_feed_thumbnail_first().max(0) as usize
            || index >= app.get_feed_thumbnail_end().max(0) as usize
            || state
                .model
                .row_data(index)
                .is_none_or(|row| row.id.as_str() != format!("f{index:010}"))
        {
            eprintln!(
                "library keyboard focus mismatch: expected={index} focused={} parking={} target={} request={} delivery={} visible_first={} visible_end={}",
                app.get_feed_focused_index(),
                app.get_feed_parking_active(),
                app.get_feed_focus_target(),
                app.get_feed_focus_serial(),
                app.get_feed_focus_delivery(),
                app.get_feed_thumbnail_first(),
                app.get_feed_thumbnail_end()
            );
            return Err(
                "virtualized keyboard focus did not reach the expected acknowledged fixture identity",
            );
        }
        Ok(())
    }
    fn step(&mut self, app: &App, state: &UiState, seconds: u64) -> Result<(), &'static str> {
        use slint::platform::Key;
        if state.model.row_count() != PAGE_SIZE
            || state.player.snapshot().file_loads != 0
            || state.thumbnails.borrow().statistics().remote_started != 0
        {
            return Err("keyboard fixture left its bounded offline page");
        }
        match seconds {
            31 => {
                self.original_size = Some(app.window().size());
                self.original_columns = app.get_columns();
                app.invoke_feed_focus_entry();
            }
            32 => {
                Self::focused(app, state, 0)?;
                Self::key(app, Key::End);
                eprintln!(
                    "library keyboard End dispatch: focused={} parking={} target={} request={} delivery={} visible_first={} visible_end={}",
                    app.get_feed_focused_index(),
                    app.get_feed_parking_active(),
                    app.get_feed_focus_target(),
                    app.get_feed_focus_serial(),
                    app.get_feed_focus_delivery(),
                    app.get_feed_thumbnail_first(),
                    app.get_feed_thumbnail_end()
                );
                if !app.get_feed_parking_active() || app.get_feed_focus_target() != 99 {
                    return Err(
                        "keyboard fixture did not exercise a genuinely deferred offscreen target",
                    );
                }
                Self::key(app, Key::LeftArrow);
                if app.get_feed_focus_target() != 98 {
                    return Err("rapid parked navigation lost its logical target");
                }
                Self::key(app, Key::RightArrow);
                if app.get_feed_focus_target() != 99 {
                    return Err("rapid parked navigation failed to coalesce");
                }
            }
            34 => {
                Self::focused(app, state, PAGE_SIZE - 1)?;
                Self::key(app, Key::Tab);
                if app.get_feed_focused_index() >= 0 {
                    return Err("Tab at the final feed item trapped keyboard focus");
                }
                Self::key(app, Key::Backtab);
                Self::focused(app, state, PAGE_SIZE - 1)?;
                // 760 and 1280 yield different column counts with the shared sidebar.
                let width = if self.original_columns == 2 {
                    1280.
                } else {
                    760.
                };
                app.window().set_size(slint::LogicalSize::new(width, 600.));
            }
            36 => {
                Self::focused(app, state, PAGE_SIZE - 1)?;
                if app.get_columns() == self.original_columns {
                    return Err("keyboard fixture resize did not cross a column boundary");
                }
                Self::key(app, Key::Home);
            }
            38 => {
                Self::focused(app, state, 0)?;
                self.page_down_target =
                    (app.get_columns() * app.get_feed_page_rows()).max(1) as usize;
                Self::key(app, Key::PageDown);
            }
            40 => {
                Self::focused(app, state, self.page_down_target.min(PAGE_SIZE - 1))?;
                Self::key(app, Key::End);
                // The deferred delivery timer cannot run between these calls;
                // changing focus must invalidate its pending claim, regardless
                // of whether Slint constructs the distant target synchronously.
                app.invoke_focus_search();
            }
            42 => {
                if !app.get_search_active()
                    || app.get_feed_focused_index() != -1
                    || app.get_feed_focus_serial() != 0
                {
                    return Err("late virtualized construction stole unrelated search focus");
                }
                if let Some(size) = self.original_size {
                    app.window().set_size(size);
                }
                eprintln!(
                    "library resource fixture keyboard: End/Home/PageDown reached actual bounded identities; column resize retained item99; unrelated search focus cancelled delayed handoff; screen_reader=not_tested"
                );
            }
            _ => return Err("unexpected keyboard fixture stage"),
        }
        eprintln!(
            "library keyboard stage={seconds}: focused={} parking={} target={} request={} delivery={} visible_first={} visible_end={} columns={}",
            app.get_feed_focused_index(),
            app.get_feed_parking_active(),
            app.get_feed_focus_target(),
            app.get_feed_focus_serial(),
            app.get_feed_focus_delivery(),
            app.get_feed_thumbnail_first(),
            app.get_feed_thumbnail_end(),
            app.get_columns()
        );
        Ok(())
    }
}
impl Pages {
    fn observe(&mut self, page: usize, ids: Vec<slint::SharedString>) -> Result<(), &'static str> {
        if page >= self.ids.len() || ids.len() != PAGE_SIZE {
            return Err("fixture traversal requires acknowledged 100-row pages");
        }
        if ids.iter().enumerate().any(|(i, id)| ids[..i].contains(id)) {
            return Err("fixture page contains duplicate identities");
        }
        if let Some(previous) = &self.ids[page] {
            if previous != &ids {
                return Err("returning to a fixture page changed its identities or order");
            }
        } else {
            if self
                .ids
                .iter()
                .flatten()
                .any(|previous| ids.iter().any(|id| previous.contains(id)))
            {
                return Err("successive fixture pages overlapped");
            }
            self.ids[page] = Some(ids);
        }
        Ok(())
    }
}

pub struct Smoke {
    completed: Rc<Cell<usize>>,
    failure: Rc<Cell<Option<&'static str>>>,
    _timers: Vec<slint::Timer>,
}
impl Smoke {
    pub fn start(app: &App, state: &Rc<UiState>) -> Self {
        let completed = Rc::new(Cell::new(0));
        let failure = Rc::new(Cell::new(None));
        let pages = Rc::new(RefCell::new(Pages::default()));
        let counters = Rc::new(Cell::new(None));
        let range = Rc::new(Cell::new((0, 0)));
        let keyboard = Rc::new(RefCell::new(Keyboard::default()));
        let timers = STAGES.into_iter().enumerate().map(|(index, seconds)| {
            let app = app.as_weak();
            let state = Rc::downgrade(state);
            let completed = completed.clone();
            let failure = failure.clone();
            let pages = pages.clone();
            let counters = counters.clone();
            let range = range.clone();
            let keyboard = keyboard.clone();
            let timer = slint::Timer::default();
            timer.start(slint::TimerMode::SingleShot, Duration::from_secs(seconds), move || {
                if failure.get().is_some() { return; }
                let (Some(app), Some(state)) = (app.upgrade(), state.upgrade()) else { return; };
                let result = (|| -> Result<(), &'static str> {
                    if completed.get() != index { return Err("fixture traversal missed a scheduled stage"); }
                    let ui = app.global::<LibraryUi>();
                    if ui.get_busy() || app.get_page() != 0 || state.hidden.get() {
                        return Err("fixture feed was not settled and visible at the finite checkpoint");
                    }
                    if state.player.snapshot().file_loads != 0 || app.get_account_connected() {
                        return Err("offline fixture unexpectedly started media or connected an account");
                    }
                    if index >= PAGE_STAGES { return keyboard.borrow_mut().step(&app, &state, seconds); }
                    let videos = crate::library_ui::fixture_videos(&state)
                        .ok_or("fixture worker has no acknowledged video page")?;
                    let ids: Vec<_> = (0..state.model.row_count()).filter_map(|row| state.model.row_data(row).map(|row| row.id)).collect();
                    if videos.len() != PAGE_SIZE || ids.len() != PAGE_SIZE || videos.iter().zip(&ids).any(|(video, id)| video.id.as_str() != id.as_str()) {
                        return Err("fixture feed does not match its actual SQLite page");
                    }
                    pages.borrow_mut().observe(PAGES[index / 2], ids)?;
                    if videos.first().map(|v| v.id.as_str()) != Some(format!("f{:010}", PAGES[index / 2] * PAGE_SIZE).as_str()) {
                        return Err("fixture page did not begin at the expected real SQLite cursor");
                    }
                    {
                        let worker = state.thumbnails.borrow();
                        let images = worker.statistics();
                        if images.remote_started != 0 { return Err("offline fixture initiated a remote thumbnail request"); }
                        if images.admitted_generation != worker.generation() || images.pending != 0 || images.inflight != 0 || images.ready != 0 {
                            return Err("fixture thumbnail worker did not settle before the finite input checkpoint");
                        }
                    }
                    let (first, end) = state.thumbnail_range.get();
                    if first >= end || end > PAGE_SIZE || end - first > 40 || (first..end).any(|row| !state.model.row_data(row).is_some_and(|row| row.thumbnail_ready)) {
                        return Err("fixture near-viewport thumbnails did not finish before the input checkpoint");
                    }
                    let visible_first = app.get_feed_thumbnail_first().max(0) as usize;
                    let visible_end = app.get_feed_thumbnail_end().max(0) as usize;
                    if visible_first >= visible_end || visible_first < first || visible_end > end {
                        return Err("visible fixture thumbnails were outside the admitted viewport");
                    }
                    if index % 2 == 0 {
                        // Focus changes are explicit and may redraw their local widget.
                        // The baseline is sampled afterwards; raw pointer coordinates
                        // must not alter application models, clock or redraw requests.
                        app.invoke_focus_search();
                        if !app.get_search_active() { return Err("fixture search focus was not acquired"); }
                        counters.set(Some(Counters::capture(&state)));
                        range.set((first, end));
                        for step in 0..40 {
                            app.window().dispatch_event(slint::platform::WindowEvent::PointerMoved {
                                position: slint::LogicalPosition::new(2. + step as f32 / 100., 2.),
                            });
                        }
                        // Enter and move within the same unselected sidebar
                        // action. Toolkit hover drawing is permitted; application
                        // models and thumbnail work must remain unchanged.
                        for step in 0..40 {
                            app.window().dispatch_event(slint::platform::WindowEvent::PointerMoved {
                                position: slint::LogicalPosition::new(32. + step as f32 / 100., 140.),
                            });
                        }
                    } else {
                        if counters.get() != Some(Counters::capture(&state)) || range.get() != (first, end) || !app.get_search_active() {
                            return Err("irrelevant cursor motion changed fixture models, viewport, focus or application redraw work");
                        }
                        app.window().dispatch_event(slint::platform::WindowEvent::PointerMoved { position: slint::LogicalPosition::new(2., 2.) });
                        if counters.get() != Some(Counters::capture(&state)) { return Err("leaving the hover target changed application models or thumbnail work"); }
                        eprintln!("library resource fixture checkpoint={} page={} sqlite_rows={} near_viewport_ready={} catalog_changes={} catalog_resets={} group_child_changes={} input_model_delta=0 media_loads=0 account_connected=false viewport_intersecting_ready_thumbnails={} compositor_visibility=not_measured", index / 2 + 1, PAGES[index / 2] + 1, videos.len(), end - first, state.model.changes.get(), state.model.resets.get(), state.groups.child_changes.get(), visible_end - visible_first);
                        if index + 1 < PAGE_STAGES {
                            let forward = index < 4;
                            if (forward && !ui.get_next()) || (!forward && !ui.get_previous()) {
                                return Err("fixture page cursor was unavailable");
                            }
                            ui.invoke_page(forward);
                            if !ui.get_busy() { return Err("actual library page request was not admitted"); }
                        }
                    }
                    Ok(())
                })();
                if let Err(reason) = result {
                    failure.set(Some(reason));
                    eprintln!("library resource fixture failed: {reason}");
                    let _ = slint::quit_event_loop();
                    return;
                }
                completed.set(index + 1);
                if index + 1 == STAGES.len() { let _ = slint::quit_event_loop(); }
            });
            timer
        }).collect();
        Self {
            completed,
            failure,
            _timers: timers,
        }
    }
    pub fn finish(self) -> Result<(), &'static str> {
        if let Some(reason) = self.failure.get() {
            return Err(reason);
        }
        (self.completed.get() == STAGES.len())
            .then_some(())
            .ok_or("fixture traversal ended before all finite checks completed")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ids(page: usize) -> Vec<slint::SharedString> {
        (page * PAGE_SIZE..(page + 1) * PAGE_SIZE)
            .map(|n| n.to_string().into())
            .collect()
    }
    #[test]
    fn bounded_page_evidence_rejects_overlap_reordering_and_short_pages() {
        let mut pages = Pages::default();
        for page in PAGES {
            pages.observe(page, ids(page)).unwrap();
        }
        let mut reordered = ids(0);
        reordered.swap(0, 1);
        assert!(pages.observe(0, reordered).is_err());
        assert!(Pages::default().observe(0, vec![]).is_err());
        let mut overlap = Pages::default();
        overlap.observe(0, ids(0)).unwrap();
        assert!(overlap.observe(1, ids(0)).is_err());
        let mut repeated = ids(0);
        repeated[1] = repeated[0].clone();
        assert!(Pages::default().observe(0, repeated).is_err());
        assert!(Pages::default().observe(3, ids(3)).is_err());
    }
    #[test]
    fn early_exit_and_recorded_failure_cannot_pass() {
        for complete in 0..=STAGES.len() {
            for failed in [false, true] {
                let smoke = Smoke {
                    completed: Rc::new(Cell::new(complete)),
                    failure: Rc::new(Cell::new(failed.then_some("fixture failure"))),
                    _timers: vec![],
                };
                assert_eq!(smoke.finish().is_ok(), complete == STAGES.len() && !failed);
            }
        }
    }
}
