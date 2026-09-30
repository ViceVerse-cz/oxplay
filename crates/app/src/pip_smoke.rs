// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit finite local-video UI exercise, never a performance measurement.
use crate::{App, UiState};
use oxplay_media::PlaybackState;
use slint::{
    ComponentHandle, Timer, TimerMode,
    winit_030::{WinitWindowAccessor, winit},
};
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
    time::{Duration, Instant},
};

const COMPLETE: usize = 9;
const MAX_DURATION: Duration = Duration::from_secs(24);
const STEP_HOLD: Duration = Duration::from_secs(2);
const STEP_TIMEOUT: Duration = Duration::from_secs(4);

#[derive(Clone, Copy)]
struct Original {
    window: winit::window::WindowId,
    width: f64,
    height: f64,
    loads: u64,
    request: u64,
    presenters: u64,
    decorated: bool,
    frame_width: f64,
    frame_height: f64,
    seek: f64,
}

struct Driver {
    app: slint::Weak<App>,
    state: Weak<UiState>,
    timer: Timer,
    started: Instant,
    phase_started: Cell<Instant>,
    stage: Cell<usize>,
    original: Cell<Option<Original>>,
    baseline_at: Cell<Option<Instant>>,
    failure: RefCell<Option<&'static str>>,
}

pub struct Smoke(Rc<Driver>);

impl Smoke {
    pub fn start(app: &App, state: &Rc<UiState>) -> Self {
        let started = Instant::now();
        let driver = Rc::new(Driver {
            app: app.as_weak(),
            state: Rc::downgrade(state),
            timer: Timer::default(),
            started,
            phase_started: Cell::new(started),
            stage: Cell::new(0),
            original: Cell::new(None),
            baseline_at: Cell::new(None),
            failure: RefCell::new(None),
        });
        Driver::schedule(&driver);
        Self(driver)
    }

    pub fn finish(self) -> Result<(), &'static str> {
        self.0.timer.stop();
        if let Some(error) = *self.0.failure.borrow() {
            return Err(error);
        }
        if self.0.stage.get() != COMPLETE {
            return Err("Picture-in-picture exercise ended before all functional checks completed");
        }
        Ok(())
    }
}

impl Driver {
    fn schedule(driver: &Rc<Self>) {
        let weak = Rc::downgrade(driver);
        // Finite explicit UI automation only. Observe already-delivered engine
        // state; never issue recurring native property queries or redraws.
        driver.timer.start(
            TimerMode::SingleShot,
            Duration::from_millis(100),
            move || {
                if let Some(driver) = weak.upgrade() {
                    driver.tick();
                }
            },
        );
    }

    fn fail(&self, error: &'static str) {
        self.timer.stop();
        *self.failure.borrow_mut() = Some(error);
        eprintln!(
            "picture-in-picture stage={} failed: {error}",
            self.stage.get() + 1
        );
        // Normal event-loop teardown retains all existing presenter/player
        // destruction ordering; no process exit, panic or second media window.
        let _ = slint::quit_event_loop();
    }

    fn tick(self: &Rc<Self>) {
        let (Some(app), Some(state)) = (self.app.upgrade(), self.state.upgrade()) else {
            self.fail("The owning UI or player disappeared during the exercise");
            return;
        };
        let elapsed = self.started.elapsed();
        if elapsed >= MAX_DURATION {
            self.fail("Picture-in-picture did not complete within its finite deadline");
            return;
        }
        let hold = if self.stage.get() == 0 {
            Duration::from_secs(3)
        } else {
            STEP_HOLD
        };
        if self.phase_started.get().elapsed() < hold {
            Self::schedule(self);
            return;
        }
        match self.step(&app, &state) {
            Ok(true) => {
                let completed = self.stage.get() + 1;
                self.stage.set(completed);
                self.phase_started.set(Instant::now());
                eprintln!(
                    "picture-in-picture stage={completed} passed at_ms={}",
                    elapsed.as_millis()
                );
                if completed == COMPLETE {
                    eprintln!(
                        "picture-in-picture completed: same native window, media load and presenter generation; borderless compact/paused-seek/resume/resize/Escape/close-request decoration restoration; performance=not_measured"
                    );
                    return;
                }
            }
            Ok(false) => {
                let deadline = if self.stage.get() == 0 {
                    Duration::from_secs(8)
                } else {
                    STEP_TIMEOUT
                };
                if self.phase_started.get().elapsed() >= deadline {
                    self.fail(
                        "Expected native playback/window state did not settle for this stage",
                    );
                    return;
                }
            }
            Err(error) => {
                self.fail(error);
                return;
            }
        }
        Self::schedule(self);
    }

    fn step(&self, app: &App, state: &UiState) -> Result<bool, &'static str> {
        if state.native_child.enabled {
            return Err("Picture-in-picture exercise requires the regular shared-window presenter");
        }
        let observed_compact = app.get_picture_in_picture();
        let snapshot = state.player.snapshot();
        if snapshot.error.is_some() {
            return Err("The media engine reported an error during picture-in-picture");
        }
        let (window, size, decorated, frame) = app
            .window()
            .with_winit_window(|window| {
                (
                    window.id(),
                    window.inner_size().to_logical::<f64>(window.scale_factor()),
                    window.is_decorated(),
                    winit::dpi::LogicalSize::new(
                        f64::from(
                            window
                                .outer_size()
                                .width
                                .saturating_sub(window.inner_size().width),
                        ) / window.scale_factor(),
                        f64::from(
                            window
                                .outer_size()
                                .height
                                .saturating_sub(window.inner_size().height),
                        ) / window.scale_factor(),
                    ),
                )
            })
            .ok_or("The native Winit window is unavailable")?;
        let ready = app.get_loaded()
            && app.window().is_visible()
            && state.presentation_ready.get()
            && !state.hidden.get()
            && state.player.current_load_frame_ready()
            && state.player.clock_identity().is_some()
            && app.get_video_width() > 0.
            && app.get_video_height() > 0.
            && app.get_video_texture().size().width > 0;
        if self.stage.get() == 0 {
            if !ready
                || snapshot.paused
                || snapshot.state != PlaybackState::Playing
                || !app.get_pip_available()
                || !snapshot.duration.is_finite()
                || snapshot.duration < 5.
                || state.presenter_generations.get() == 0
            {
                return Ok(false);
            }
            self.original.set(Some(Original {
                window,
                width: size.width,
                height: size.height,
                loads: snapshot.file_loads,
                request: snapshot.load_request_id,
                presenters: state.presenter_generations.get(),
                decorated,
                frame_width: frame.width,
                frame_height: frame.height,
                seek: (snapshot.duration / 3.).min(20.),
            }));
            if let Some(at) = self.baseline_at.get() {
                if at.elapsed() < Duration::from_secs(1) {
                    return Ok(false);
                }
            } else {
                self.baseline_at.set(Some(Instant::now()));
                eprintln!(
                    r#"pip window probe: {{"phase":"baseline","inner_width":{},"inner_height":{},"outer_width":{},"outer_height":{},"scale":{},"decorated":{}}}"#,
                    size.width,
                    size.height,
                    size.width + frame.width,
                    size.height + frame.height,
                    app.window().scale_factor(),
                    decorated
                );
                return Ok(false);
            }
            app.invoke_focus_video_mode();
            app.window()
                .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: "p".into() });
            app.window()
                .dispatch_event(slint::platform::WindowEvent::KeyReleased { text: "p".into() });
            return Ok(true);
        }
        let original = self
            .original
            .get()
            .ok_or("Original playback state was not captured")?;
        if window != original.window
            || snapshot.file_loads != original.loads
            || snapshot.load_request_id != original.request
            || state.presenter_generations.get() != original.presenters
        {
            return Err("Picture-in-picture replaced its native window, media load or presenter");
        }
        match self.stage.get() {
            1 => {
                if !ready
                    || !app.get_picture_in_picture()
                    || decorated
                    || !near(frame.width, 0.)
                    || !near(frame.height, 0.)
                    || !near(size.width, 480.)
                    || !near(size.height, 270.)
                {
                    return Ok(false);
                }
                app.invoke_toggle_pause();
            }
            2 => {
                if !ready || !snapshot.paused || snapshot.state != PlaybackState::Paused {
                    return Ok(false);
                }
                app.invoke_seek(original.seek as f32);
            }
            3 => {
                if !ready
                    || !snapshot.paused
                    || snapshot.state != PlaybackState::Paused
                    || (snapshot.position - original.seek).abs() > 0.35
                {
                    return Ok(false);
                }
                app.invoke_toggle_pause();
            }
            4 => {
                if !ready
                    || snapshot.paused
                    || snapshot.state != PlaybackState::Playing
                    || snapshot.position < original.seek + 0.2
                {
                    return Ok(false);
                }
                app.window().with_winit_window(|window| {
                    let _ = window.request_inner_size(winit::dpi::LogicalSize::new(360., 260.));
                });
            }
            5 => {
                if !ready
                    || !app.get_picture_in_picture()
                    || decorated
                    || !near(frame.width, 0.)
                    || !near(frame.height, 0.)
                    || !near(size.width, 360.)
                    || !near(size.height, 260.)
                {
                    return Ok(false);
                }
                app.window()
                    .dispatch_event(slint::platform::WindowEvent::KeyPressed {
                        text: slint::platform::Key::Escape.into(),
                    });
                app.window()
                    .dispatch_event(slint::platform::WindowEvent::KeyReleased {
                        text: slint::platform::Key::Escape.into(),
                    });
            }
            6 => {
                if !ready
                    || app.get_picture_in_picture()
                    || decorated != original.decorated
                    || !near(frame.width, original.frame_width)
                    || !near(frame.height, original.frame_height)
                    || !near(size.width, original.width)
                    || !near(size.height, original.height)
                {
                    return Ok(false);
                }
                app.invoke_picture_in_picture_toggle();
            }
            7 => {
                if !ready
                    || !app.get_picture_in_picture()
                    || decorated
                    || !near(frame.width, 0.)
                    || !near(frame.height, 0.)
                    || !near(size.width, 360.)
                    || !near(size.height, 260.)
                {
                    return Ok(false);
                }
                // Pinned core/platform.rs documents this event as invoking the
                // actual close-request callback; core/api.rs only hides when
                // that callback permits it. Exercise return-to-window handling
                // without claiming a native OS mouse click or closing the app.
                app.window()
                    .dispatch_event(slint::platform::WindowEvent::CloseRequested);
            }
            8 => {
                if !ready
                    || app.get_picture_in_picture()
                    || decorated != original.decorated
                    || !near(frame.width, original.frame_width)
                    || !near(frame.height, original.frame_height)
                    || !near(size.width, original.width)
                    || !near(size.height, original.height)
                {
                    return Ok(false);
                }
            }
            _ => return Err("Unexpected picture-in-picture stage"),
        }
        if matches!(self.stage.get(), 1 | 8) {
            let phase = if self.stage.get() == 1 {
                "compact"
            } else {
                "restored"
            };
            eprintln!(
                r#"pip window probe: {{"phase":"{phase}","inner_width":{},"inner_height":{},"outer_width":{},"outer_height":{},"scale":{},"decorated":{}}}"#,
                size.width,
                size.height,
                size.width + frame.width,
                size.height + frame.height,
                app.window().scale_factor(),
                decorated
            );
        }
        if matches!(self.stage.get(), 1 | 5 | 6 | 7 | 8) {
            eprintln!(
                "picture-in-picture geometry stage={} compact={} decorated={} client={}x{} frame={}x{}",
                self.stage.get() + 1,
                observed_compact,
                decorated,
                size.width,
                size.height,
                frame.width,
                frame.height
            );
        }
        Ok(true)
    }
}

fn near(actual: f64, expected: f64) -> bool {
    actual.is_finite() && (actual - expected).abs() <= 2.
}
