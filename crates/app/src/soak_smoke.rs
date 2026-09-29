// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit finite local mixed-use soak. No network, screenshots or helper scans.
use crate::{App, UiState};
use slint::winit_030::WinitWindowAccessor;
use slint::{ComponentHandle, Model, Timer, TimerMode};
use std::{
    cell::Cell,
    path::PathBuf,
    rc::{Rc, Weak},
    time::{Duration, Instant},
};

const MIN_MINUTES: u32 = 60;
const MAX_MINUTES: u32 = 240;
// Exactly one single-shot timer is armed. These are actions, not periodic
// frame/property polls; identical phase checkpoints repeat once per minute.
const PHASES: [u64; 19] = [
    5, 10, 15, 18, 20, 23, 26, 29, 31, 35, 38, 42, 45, 48, 50, 53, 55, 57, 59,
];

/// Keep the soak on the current default production path. Qualification of an
/// experimental renderer/timing mode requires a separately recorded workload.
pub fn validate_environment() -> Result<(), &'static str> {
    validate_environment_with(|name| std::env::var_os(name).is_some())
}
fn validate_environment_with(present: impl Fn(&str) -> bool) -> Result<(), &'static str> {
    if [
        "SEREIN_STABLE_VIDEO_TARGET",
        "SEREIN_VIDEO_LEAD_MS",
        "SEREIN_VIDEO_PREPARE_MS",
        "SEREIN_DIAGNOSTIC_NULL_AUDIO",
        "SEREIN_DIAGNOSTIC_AUTOSYNC",
        "SEREIN_MEDIA_TIMING",
        "SEREIN_GPU_TIMING",
    ]
    .into_iter()
    .any(present)
    {
        return Err(
            "Unset media timing, silent-audio and rendering experiment variables before the local soak",
        );
    }
    Ok(())
}

pub struct Config {
    pub minutes: u32,
    /// Host prevalidates an explicit absolute moving local fixture before UI.
    pub local: PathBuf,
    pub subtitle: Option<PathBuf>,
}
impl Config {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !(MIN_MINUTES..=MAX_MINUTES).contains(&self.minutes) {
            return Err("local soak duration must be between 60 and 240 minutes");
        }
        if !self.local.is_absolute()
            || self
                .subtitle
                .as_ref()
                .is_some_and(|path| !path.is_absolute())
        {
            return Err("local soak requires prevalidated absolute fixture paths");
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Default)]
struct PausedCounters {
    draws: u64,
    redraws: u64,
    clock_ticks: u64,
}
struct Driver {
    config: Config,
    app: slint::Weak<App>,
    state: Weak<UiState>,
    timer: Timer,
    started: Instant,
    step: Cell<usize>,
    completed_cycles: Cell<u32>,
    failure: Cell<Option<&'static str>>,
    complete: Cell<bool>,
    cleaning: Cell<bool>,
    baseline_model: (u64, u64),
    expected_loads: Cell<u64>,
    paused: Cell<PausedCounters>,
}
pub struct Smoke {
    driver: Rc<Driver>,
}
impl Smoke {
    pub fn start(app: &App, state: &Rc<UiState>, config: Config) -> Result<Self, &'static str> {
        config.validate()?;
        if state.model.row_count() != 30 || app.get_remote_video() {
            return Err(
                "local soak requires exactly 30 labeled --demo-related fixture rows and local playback",
            );
        }
        let driver = Rc::new(Driver {
            config,
            app: app.as_weak(),
            state: Rc::downgrade(state),
            timer: Timer::default(),
            started: Instant::now(),
            step: Cell::new(0),
            completed_cycles: Cell::new(0),
            failure: Cell::new(None),
            complete: Cell::new(false),
            cleaning: Cell::new(false),
            baseline_model: (state.model.changes.get(), state.model.resets.get()),
            expected_loads: Cell::new(1),
            paused: Cell::default(),
        });
        eprintln!(
            "local soak started: minutes={} related_rows=30 network=false; resource/helper evidence requires external whole-tree sampler",
            driver.config.minutes
        );
        Driver::arm(&driver);
        Ok(Self { driver })
    }
    pub fn finish(self) -> Result<(), &'static str> {
        if let Some(error) = self.driver.failure.get() {
            return Err(error);
        }
        if !self.driver.complete.get()
            || self.driver.completed_cycles.get() != self.driver.config.minutes
            || self.driver.started.elapsed()
                < Duration::from_secs(u64::from(self.driver.config.minutes) * 60)
        {
            return Err(
                "local soak ended before the full configured duration and cleanup; shortened runs do not pass",
            );
        }
        Ok(())
    }
}

fn scheduled_seconds(step: usize, minutes: u32) -> Option<u64> {
    let cycle = step / PHASES.len();
    (cycle < minutes as usize).then(|| cycle as u64 * 60 + PHASES[step % PHASES.len()])
}
impl Driver {
    fn arm(this: &Rc<Self>) {
        let deadline = scheduled_seconds(this.step.get(), this.config.minutes)
            .unwrap_or(u64::from(this.config.minutes) * 60);
        let delay = Duration::from_secs(deadline)
            .saturating_sub(this.started.elapsed())
            .max(Duration::from_millis(1));
        let weak = Rc::downgrade(this);
        this.timer.start(TimerMode::SingleShot, delay, move || {
            if let Some(this) = weak.upgrade() {
                Self::step(&this);
            }
        });
    }
    fn step(this: &Rc<Self>) {
        let (Some(app), Some(state)) = (this.app.upgrade(), this.state.upgrade()) else {
            this.failure
                .set(Some("local soak window or player was destroyed early"));
            let _ = slint::quit_event_loop();
            return;
        };
        let step = this.step.get();
        let Some(scheduled) = scheduled_seconds(step, this.config.minutes) else {
            Self::cleanup(this, &app, &state);
            return;
        };
        if this.started.elapsed() > Duration::from_secs(scheduled + 5) {
            this.failure.set(Some(
                "local soak action missed its bounded 5-second scheduling deadline",
            ));
            Self::cleanup(this, &app, &state);
            return;
        }
        let cycle = step / PHASES.len();
        let phase = PHASES[step % PHASES.len()];
        if let Err(error) = this.action(&app, &state, cycle as u32, phase) {
            this.failure.set(Some(error));
            eprintln!(
                "local soak FAILED cycle={} phase={phase}: {error}",
                cycle + 1
            );
            Self::cleanup(this, &app, &state);
            return;
        }
        this.step.set(step + 1);
        Self::arm(this);
    }
    fn action(
        &self,
        app: &App,
        state: &Rc<UiState>,
        cycle: u32,
        phase: u64,
    ) -> Result<(), &'static str> {
        let snapshot = state.player.snapshot();
        if snapshot.error.is_some() {
            return Err("local soak engine reported an error");
        }
        if (state.model.changes.get(), state.model.resets.get()) != self.baseline_model {
            return Err(
                "local soak playback/input caused unrelated catalog notifications or resets",
            );
        }
        if app.get_remote_video() || crate::account_playback::authorization(state).is_some() {
            return Err("local soak must not contain remote or account playback");
        }
        match phase {
            5 | 57 => {
                if snapshot.paused
                    || !matches!(snapshot.state, serein_media::PlaybackState::Playing)
                    || !state.player.current_load_is_active()
                    || !state.player.current_load_frame_ready()
                    || snapshot.file_loads != self.expected_loads.get()
                    || snapshot.width <= 0
                    || snapshot.height <= 0
                    || snapshot.duration < 60.
                    || app.get_video_texture().size().width == 0
                    || state.hidden.get()
                    || app.get_page() != 2
                    || app.get_fullscreen_active()
                {
                    return Err(
                        "local soak playing checkpoint has wrong load, state, visibility or video readiness",
                    );
                }
                eprintln!(
                    "local soak checkpoint cycle={} phase={phase} elapsed_ms={} loads={} load_id={} position={:.3} codec={} decoder={} size={}x{} fps={:.3} vo_drops={} decoder_drops={} events={} wakes={} media_notifications={} clock_ticks={} ui_draws={} redraw_requests={} ui_assignments={} model_changes_delta={} model_resets_delta={}",
                    cycle + 1,
                    self.started.elapsed().as_millis(),
                    snapshot.file_loads,
                    snapshot.load_request_id,
                    snapshot.position,
                    snapshot.codec,
                    snapshot.hwdec_current,
                    snapshot.width,
                    snapshot.height,
                    snapshot.fps,
                    snapshot.dropped_frames,
                    snapshot.decoder_dropped_frames,
                    snapshot.events_received,
                    snapshot.wakeups,
                    snapshot.render_notifications,
                    snapshot.display_clock_ticks,
                    state.draw_callbacks.get(),
                    state.redraw_requests.get(),
                    state.ui_assignments.get(),
                    state.model.changes.get() - self.baseline_model.0,
                    state.model.resets.get() - self.baseline_model.1
                );
                if phase == 57 {
                    self.completed_cycles.set(cycle + 1);
                }
            }
            10 => {
                if app.get_controls_visible() {
                    app.invoke_toggle_controls();
                }
            }
            15 => {
                if app.get_controls_visible() || state.progress.running() {
                    return Err("local soak hidden controls kept progress polling alive");
                }
                // Finite input burst over the same noninteractive margin.
                for x in 0..20 {
                    app.window()
                        .dispatch_event(slint::platform::WindowEvent::PointerMoved {
                            position: slint::LogicalPosition::new(2. + x as f32 / 100., 2.),
                        });
                }
            }
            18 => {
                if !app.get_controls_visible() {
                    app.invoke_toggle_controls();
                }
            }
            20 => {
                state
                    .player
                    .set_paused(true)
                    .map_err(|_| "local soak pause request failed")?;
            }
            23 => {
                if !snapshot.paused || state.progress.running() || snapshot.display_clock_active {
                    return Err(
                        "local soak settled pause retained progress or display clock activity",
                    );
                }
                self.paused.set(PausedCounters {
                    draws: state.draw_callbacks.get(),
                    redraws: state.redraw_requests.get(),
                    clock_ticks: snapshot.display_clock_ticks,
                });
            }
            26 => {
                let paused = self.paused.get();
                if snapshot.display_clock_ticks != paused.clock_ticks
                    || state.redraw_requests.get() != paused.redraws
                    || state.draw_callbacks.get().saturating_sub(paused.draws) > 1
                {
                    return Err("local soak stationary settled pause continued presentation work");
                }
                app.invoke_seek(10. + (cycle % 3) as f32 * 5.);
            }
            29 => {
                let target = 10. + f64::from(cycle % 3) * 5.;
                if !snapshot.paused || (snapshot.position - target).abs() > 0.25 {
                    return Err("local soak paused seek did not settle at requested position");
                }
            }
            31 => {
                state
                    .player
                    .set_paused(false)
                    .map_err(|_| "local soak resume failed")?;
            }
            35 => {
                app.invoke_navigate(0);
            }
            38 => {
                if app.get_page() != 0 || !snapshot.paused || state.progress.running() {
                    return Err("local soak navigation did not pause offscreen playback");
                }
                app.invoke_navigate(2);
                state
                    .player
                    .set_paused(false)
                    .map_err(|_| "local soak watch return failed")?;
            }
            42 => {
                app.window().set_size(slint::LogicalSize::new(1100., 760.));
            }
            45 => {
                app.window()
                    .with_winit_window(|window| window.set_minimized(true));
            }
            48 => {
                let minimized = app
                    .window()
                    .with_winit_window(|window| window.is_minimized())
                    .flatten();
                if minimized != Some(true)
                    || !snapshot.paused
                    || state.progress.running()
                    || snapshot.display_clock_active
                {
                    return Err("local soak minimize state/pause policy was not observed");
                }
            }
            50 => {
                app.window()
                    .with_winit_window(|window| window.set_minimized(false));
            }
            53 => {
                app.window().set_size(slint::LogicalSize::new(1320., 860.));
                if cycle.is_multiple_of(2) {
                    app.invoke_fullscreen();
                }
            }
            55 => {
                if app.get_fullscreen_active() {
                    app.invoke_exit_fullscreen();
                }
            }
            59 if cycle + 1 < self.config.minutes => {
                if (cycle + 1).is_multiple_of(5) {
                    state.clock_ui.invalidate();
                    state
                        .player
                        .load_local_with_subtitle_at(
                            &self.config.local,
                            self.config.subtitle.as_deref(),
                            0.,
                            false,
                        )
                        .map_err(|_| "local soak explicit fixture reload failed")?;
                    app.set_video_texture(slint::Image::default());
                    self.expected_loads.set(self.expected_loads.get() + 1);
                } else {
                    app.invoke_seek(0.);
                }
            }
            59 => {}
            _ => return Err("local soak internal phase is invalid"),
        }
        Ok(())
    }
    fn cleanup(this: &Rc<Self>, app: &App, state: &UiState) {
        if this.cleaning.replace(true) {
            return;
        }
        state.clock_ui.invalidate();
        if state.player.stop().is_err() && this.failure.get().is_none() {
            this.failure.set(Some("local soak terminal stop failed"));
        }
        app.set_loaded(false);
        app.set_video_texture(slint::Image::default());
        let weak = Rc::downgrade(this);
        this.timer.start(TimerMode::SingleShot, Duration::from_secs(3), move || {
            let Some(this) = weak.upgrade() else { return; };
            let stopped = this.state.upgrade().is_some_and(|state| {
                let snapshot = state.player.snapshot();
                !snapshot.stop_pending && !snapshot.display_clock_active && !state.progress.running()
            });
            if !stopped && this.failure.get().is_none() {
                this.failure.set(Some("local soak terminal stop did not settle before cleanup deadline"));
            }
            this.complete.set(this.failure.get().is_none());
            eprintln!("local soak finished: completed_cycles={} elapsed_ms={} outcome={} helper_leak_check=external_not_measured", this.completed_cycles.get(), this.started.elapsed().as_millis(), if this.complete.get() { "local-functional-pass" } else { "failed" });
            let _ = slint::quit_event_loop();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn soak_does_not_silently_measure_an_experimental_or_silent_pipeline() {
        assert!(validate_environment_with(|_| false).is_ok());
        for selected in [
            "SEREIN_STABLE_VIDEO_TARGET",
            "SEREIN_DIAGNOSTIC_NULL_AUDIO",
            "SEREIN_GPU_TIMING",
        ] {
            assert!(validate_environment_with(|name| name == selected).is_err());
        }
    }
    #[test]
    fn schedule_is_finite_ordered_and_cannot_end_before_an_hour() {
        let seconds: Vec<_> = (0..60 * PHASES.len())
            .map(|step| scheduled_seconds(step, 60).unwrap())
            .collect();
        assert_eq!(seconds[0], 5);
        assert_eq!(seconds.last(), Some(&3599));
        assert!(seconds.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(scheduled_seconds(seconds.len(), 60), None);
        assert_eq!(scheduled_seconds(240 * PHASES.len(), 240), None);
    }
    #[test]
    fn invalid_duration_and_relative_paths_are_rejected_without_io() {
        for minutes in [0, 1, 59, 241, u32::MAX] {
            assert!(
                Config {
                    minutes,
                    local: PathBuf::from("/fixture.mp4"),
                    subtitle: None
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            Config {
                minutes: 60,
                local: PathBuf::from("relative.mp4"),
                subtitle: None
            }
            .validate()
            .is_err()
        );
        assert!(
            Config {
                minutes: 60,
                local: PathBuf::from("/fixture.mp4"),
                subtitle: Some(PathBuf::from("relative.vtt"))
            }
            .validate()
            .is_err()
        );
        assert!(
            Config {
                minutes: 60,
                local: PathBuf::from("/fixture.mp4"),
                subtitle: None
            }
            .validate()
            .is_ok()
        );
    }
}
