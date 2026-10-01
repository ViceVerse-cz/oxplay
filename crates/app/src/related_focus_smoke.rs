// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit finite offline keyboard diagnostic; not an accessibility or resource gate.
use crate::{App, UiState};
use slint::{ComponentHandle, Model};
use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

const STAGES: [u64; 16] = [4, 6, 8, 10, 12, 14, 16, 19, 22, 24, 26, 28, 30, 33, 36, 39];
// Capture settled state before the checkpoint dispatches its next input.
const CAPTURE_STAGES: [u64; 12] = [8, 10, 12, 14, 16, 19, 22, 24, 26, 30, 36, 39];
type Captures = Rc<RefCell<Vec<std::thread::JoinHandle<Result<(), &'static str>>>>>;

fn capture_path(base: &Path, seconds: u64) -> Result<PathBuf, &'static str> {
    let stem = base
        .file_stem()
        .ok_or("related snapshot basename unavailable")?;
    let mut name = stem.to_os_string();
    name.push(format!("-stage-{seconds:02}-before.png"));
    Ok(base.with_file_name(name))
}

fn capture(app: &App, base: &Path, seconds: u64, workers: &Captures) -> Result<(), &'static str> {
    // Explicit diagnostic readback only; ordinary checks and playback never
    // call take_snapshot or start image encoding workers.
    let pixels = app
        .window()
        .take_snapshot()
        .map_err(|_| "related snapshot unavailable")?;
    let width = pixels.width();
    let height = pixels.height();
    let bytes = pixels.as_bytes().to_vec();
    let path = capture_path(base, seconds)?;
    eprintln!(
        "related focus snapshot stage={seconds} phase=before_input width={width} height={height} focused={} parking={} target={} fullscreen={}",
        app.get_feed_focused_index(),
        app.get_related_parking_active(),
        app.get_feed_focus_target(),
        app.get_fullscreen_active()
    );
    workers.borrow_mut().push(std::thread::spawn(move || {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .map_err(|_| "related snapshot destination unavailable")?;
        let image =
            image::RgbaImage::from_raw(width, height, bytes).ok_or("invalid related snapshot")?;
        image
            .write_to(&mut file, image::ImageFormat::Png)
            .map_err(|_| "related snapshot encoding failed")
    }));
    Ok(())
}
#[derive(Default)]
struct Checks {
    counters: Option<(u64, u64, u64)>,
    compact_offset: f32,
}
fn key(app: &App, key: slint::platform::Key) {
    let text: slint::SharedString = key.into();
    app.window()
        .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
    app.window()
        .dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
}
fn focused(app: &App, state: &UiState, expected: usize) -> Result<(), &'static str> {
    if app.get_feed_focused_index() != expected as i32
        || app.get_related_parking_active()
        || !app.get_related_focused_visible()
        || state
            .watch_context
            .model
            .row_data(expected)
            .is_none_or(|row| row.id.as_str() != format!("f{expected:010}"))
    {
        return Err("related identity did not acquire actual visible card focus");
    }
    Ok(())
}
impl Checks {
    fn step(&mut self, app: &App, state: &UiState, seconds: u64) -> Result<(), &'static str> {
        use slint::platform::Key;
        if app.get_page() != 2
            || !app.get_loaded()
            || state.hidden.get()
            || app.get_busy()
            || state.player.snapshot().file_loads != 1
            || state.model.row_count() != 30
            || state.watch_context.model.row_count() != 30
            || app.get_account_connected()
            || state.thumbnails.borrow().statistics().remote_started != 0
        {
            return Err("offline related fixture is not settled, visible and isolated");
        }
        let counters = (
            state.model.changes.get(),
            state.model.resets.get(),
            state.groups.child_changes.get(),
        );
        if self.counters.is_some_and(|previous| previous != counters) {
            return Err("related input emitted unrelated catalog notifications");
        }
        self.counters.get_or_insert(counters);
        match seconds {
            4 => app.window().set_size(slint::LogicalSize::new(1320., 860.)),
            6 => app.invoke_feed_focus_entry(),
            8 => {
                focused(app, state, 0)?;
                key(app, Key::End);
                if !app.get_related_parking_active() || app.get_feed_focus_target() != 29 {
                    return Err("End did not exercise a deferred offscreen related target");
                }
                key(app, Key::UpArrow);
                key(app, Key::DownArrow);
                if app.get_feed_focus_target() != 29 {
                    return Err("rapid related navigation lost its target");
                }
            }
            10 => {
                focused(app, state, 29)?;
                key(app, Key::Tab);
                if app.get_feed_focused_index() >= 0 {
                    return Err("related Tab boundary trapped focus");
                }
                key(app, Key::Backtab);
                focused(app, state, 29)?;
                app.window().set_size(slint::LogicalSize::new(760., 600.));
            }
            12 => {
                key(app, Key::Home);
            }
            14 => {
                focused(app, state, 0)?;
                // Exercise ordinary Slint Tab traversal, not the parking API.
                app.invoke_focus_player();
                for _ in 0..32 {
                    key(app, Key::Tab);
                    if app.get_feed_focused_index() >= 0 {
                        break;
                    }
                }
                if app.get_feed_focused_index() != 0 {
                    return Err("ordinary Tab did not enter the compact related list");
                }
            }
            16 => {
                focused(app, state, 0)?;
                self.compact_offset = app.get_watch_offset();
                if self.compact_offset >= -1. {
                    return Err("compact ordinary Tab did not reveal the outer watch viewport");
                }
                app.invoke_fullscreen();
            }
            19 => {
                if !app.get_fullscreen_active() || !app.window().is_fullscreen() {
                    return Err("fullscreen entry was not observed");
                }
                app.invoke_exit_fullscreen();
            }
            22 => {
                if app.get_fullscreen_active()
                    || app.window().is_fullscreen()
                    || (app.get_watch_offset() - self.compact_offset).abs() > 2.
                {
                    return Err("ordinary Tab watch offset did not survive fullscreen restoration");
                }
                app.invoke_feed_focus_entry();
            }
            24 => {
                focused(app, state, 0)?;
                key(app, Key::End);
                app.invoke_focus_search();
            }
            26 => {
                if !app.get_search_active()
                    || app.get_feed_focused_index() != -1
                    || app.get_feed_focus_serial() != 0
                {
                    return Err("late related handoff stole search focus");
                }
                app.window().set_size(slint::LogicalSize::new(1320., 860.));
            }
            28 => app.invoke_feed_focus_entry(),
            30 => {
                focused(app, state, 0)?;
                key(app, Key::End);
                app.invoke_fullscreen();
                // Slint changed-property callbacks run after this input stack.
                // The delivery claim also checks the active surface immediately.
            }
            33 => {
                eprintln!(
                    "related fullscreen retirement: player={} parking={} serial={}",
                    app.get_player_active(),
                    app.get_related_parking_active(),
                    app.get_feed_focus_serial()
                );
                if app.get_feed_focus_serial() != 0
                    || app.get_related_parking_active()
                    || !app.get_player_active()
                {
                    return Err("fullscreen did not retire pending related delivery");
                }
                if !app.get_fullscreen_active() || !app.window().is_fullscreen() {
                    return Err("second fullscreen entry failed");
                }
                app.invoke_exit_fullscreen();
                app.invoke_focus_search();
            }
            36 => {
                if app.get_fullscreen_active() || app.window().is_fullscreen() {
                    return Err("second fullscreen exit was not observed");
                }
                if !app.get_search_active() || app.get_feed_focus_serial() != 0 {
                    return Err("fullscreen-retired timer stole restored search focus");
                }
            }
            39 => {
                if !app.get_search_active() {
                    return Err("related focus did not remain cancelled");
                }
                eprintln!(
                    "related focus check complete: wide_compact=true ordinary_tab=true fullscreen_offset=true search_cancel=true fullscreen_cancel=true catalog_delta=0 loads=1 screen_reader=not_tested compositor_visibility=not_measured"
                );
            }
            _ => return Err("unknown related diagnostic stage"),
        }
        Ok(())
    }
}

pub struct Smoke {
    completed: Rc<Cell<usize>>,
    failure: Rc<Cell<Option<&'static str>>>,
    captures: Captures,
    expected_captures: usize,
    _timers: Vec<slint::Timer>,
}
impl Smoke {
    pub fn start(app: &App, state: &Rc<UiState>, snapshot: Option<PathBuf>) -> Self {
        // Explicit diagnostic mode seeds both independent surfaces. Thirty
        // synthetic rows exercise off-screen focus beyond the production
        // related-page cap; they contain no provider/account data or URLs.
        state.watch_context.model.replace(
            (0..state.model.row_count())
                .filter_map(|index| state.model.row_data(index))
                .collect(),
        );
        state.thumbnails.borrow_mut().replace(Vec::new());
        state.thumbnail_attempted.borrow_mut().clear();
        state.thumbnail_range.set((usize::MAX, usize::MAX));
        // A fresh profile plus exact copied fixture is admitted before startup.
        // Block explicit online callbacks too; this does not claim an OS egress sandbox.
        macro_rules! block {
            ($callback:ident, ($($argument:ident),*)) => {{
                app.$callback(move |$($argument),*| { $(let _ = $argument;)* });
            }};
        }
        block!(on_search, (query));
        block!(on_select_video, (index));
        block!(on_select_related, (index));
        block!(on_open_watch_channel, ());
        block!(on_guest_back, ());
        block!(on_more, ());
        block!(on_guest_previous, ());
        block!(on_search_kind_changed, (index));
        block!(on_channel_tab_changed, (index));
        block!(on_follow_guest_channel, ());
        block!(on_account_import, ());
        block!(on_account_import_path, (path, remember));
        block!(on_account_reconnect, ());
        block!(on_account_disconnect, ());
        block!(on_account_tab, (tab));
        block!(on_account_open, (index));
        block!(on_account_action, (index));
        block!(on_account_play, (index));
        block!(on_account_next, ());
        block!(on_account_reconcile, ());
        block!(on_account_rating, (like));
        block!(on_open_youtube, ());
        block!(on_open_export_guide, ());
        let completed = Rc::new(Cell::new(0));
        let failure = Rc::new(Cell::new(None));
        let checks = Rc::new(RefCell::new(Checks::default()));
        let captures = Captures::default();
        let expected_captures = if snapshot.is_some() {
            CAPTURE_STAGES.len()
        } else {
            0
        };
        let mut timers = Vec::new();
        for (index, seconds) in STAGES.into_iter().enumerate() {
            let weak = app.as_weak();
            let state = Rc::downgrade(state);
            let completed = completed.clone();
            let failure = failure.clone();
            let checks = checks.clone();
            let captures = captures.clone();
            let snapshot = snapshot.clone();
            let timer = slint::Timer::default();
            timer.start(slint::TimerMode::SingleShot, Duration::from_secs(seconds), move || {
                if failure.get().is_some() { return; }
                let result = (|| {
                    let app = weak.upgrade().ok_or("related diagnostic lost its window")?;
                    let state = state.upgrade().ok_or("related diagnostic lost its core")?;
                    if completed.get() != index { return Err("related diagnostic missed a finite stage"); }
                    if CAPTURE_STAGES.contains(&seconds) && let Some(base) = snapshot.as_deref() {
                        capture(&app, base, seconds, &captures)?;
                    }
                    let result = checks.borrow_mut().step(&app, &state, seconds);
                    eprintln!("related focus stage={seconds} focused={} parking={} target={} serial={} visible={} offset={} fullscreen={} result={:?}", app.get_feed_focused_index(), app.get_related_parking_active(), app.get_feed_focus_target(), app.get_feed_focus_serial(), app.get_related_focused_visible(), app.get_watch_offset(), app.get_fullscreen_active(), result);
                    result
                })();
                match result {
                    Ok(()) => completed.set(index + 1),
                    Err(reason) => { failure.set(Some(reason)); eprintln!("related focus failed: {reason}"); let _ = slint::quit_event_loop(); }
                }
            });
            timers.push(timer);
        }
        Self {
            completed,
            failure,
            captures,
            expected_captures,
            _timers: timers,
        }
    }
    pub fn finish(self) -> Result<(), &'static str> {
        let capture_count = self.captures.borrow().len();
        let mut capture_failure = None;
        for worker in self.captures.borrow_mut().drain(..) {
            if let Err(reason) = worker
                .join()
                .unwrap_or(Err("related snapshot worker failed"))
            {
                capture_failure.get_or_insert(reason);
            }
        }
        if let Some(reason) = self.failure.get() {
            return Err(reason);
        }
        if let Some(reason) = capture_failure {
            return Err(reason);
        }
        if capture_count != self.expected_captures {
            return Err("related focus check did not complete every requested snapshot");
        }
        if self.completed.get() == STAGES.len() {
            Ok(())
        } else {
            Err("related focus check ended before every finite stage completed")
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn early_exit_or_failed_final_stage_never_passes() {
        for count in 0..=STAGES.len() {
            for failed in [false, true] {
                let smoke = Smoke {
                    completed: Rc::new(Cell::new(count)),
                    failure: Rc::new(Cell::new(failed.then_some("fixture failure"))),
                    captures: Captures::default(),
                    expected_captures: 0,
                    _timers: vec![],
                };
                assert_eq!(smoke.finish().is_ok(), count == STAGES.len() && !failed);
            }
        }
    }

    #[test]
    fn snapshot_names_stay_beside_the_admitted_base_and_stages_are_finite() {
        let base = Path::new("/new-synthetic-root/focus.png");
        let mut names = std::collections::HashSet::new();
        for stage in CAPTURE_STAGES {
            assert!(STAGES.contains(&stage));
            let path = capture_path(base, stage).unwrap();
            assert_eq!(path.parent(), base.parent());
            assert_eq!(path.extension().unwrap(), "png");
            assert_ne!(path, base);
            assert!(names.insert(path));
        }
        assert_eq!(names.len(), 12);
    }

    #[test]
    fn completed_input_checks_cannot_hide_missing_or_failed_snapshot_outputs() {
        let complete = || Smoke {
            completed: Rc::new(Cell::new(STAGES.len())),
            failure: Rc::new(Cell::new(None)),
            captures: Captures::default(),
            expected_captures: 1,
            _timers: Vec::new(),
        };
        assert!(complete().finish().is_err());
        let failed = complete();
        failed
            .captures
            .borrow_mut()
            .push(std::thread::spawn(|| Err("synthetic encoding failure")));
        assert_eq!(failed.finish(), Err("synthetic encoding failure"));
        let succeeded = complete();
        succeeded
            .captures
            .borrow_mut()
            .push(std::thread::spawn(|| Ok(())));
        assert!(succeeded.finish().is_ok());
    }
}
