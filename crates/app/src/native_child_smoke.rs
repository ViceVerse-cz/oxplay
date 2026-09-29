// SPDX-License-Identifier: GPL-3.0-or-later
//! Finite opt-in lifecycle evidence. Native counters are not pixel evidence.
use crate::{App, UiState};
use slint::ComponentHandle;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

const STAGES: [u64; 16] = [5, 8, 11, 14, 15, 16, 17, 20, 23, 26, 29, 32, 35, 38, 41, 44];

pub struct Smoke {
    completed: Rc<Cell<usize>>,
    failure: Rc<RefCell<Option<&'static str>>>,
    _timers: Vec<slint::Timer>,
}

impl Smoke {
    pub fn start(app: &App, state: &Rc<UiState>) -> Self {
        let completed = Rc::new(Cell::new(0));
        let failure = Rc::new(RefCell::new(None));
        let publications = Rc::new(Cell::new(0));
        let volume = Rc::new(Cell::new(0.));
        let timers = STAGES.into_iter().enumerate().map(|(index, stage)| {
            let weak = app.as_weak();
            let state = Rc::downgrade(state);
            let completed = completed.clone();
            let failure = failure.clone();
            let publications = publications.clone();
            let volume = volume.clone();
            let timer = slint::Timer::default();
            timer.start(slint::TimerMode::SingleShot, Duration::from_secs(stage), move || {
                if failure.borrow().is_some() { return; }
                let (Some(app), Some(state)) = (weak.upgrade(), state.upgrade()) else { return; };
                let result = if completed.get() != index {
                    Err("native-child lifecycle skipped a checkpoint")
                } else {
                    step(stage, &app, &state, &publications, &volume)
                };
                match result {
                    Ok(()) => {
                        completed.set(index + 1);
                        eprintln!("native-child lifecycle stage={stage} passed; compositor pixels not measured");
                    }
                    Err(error) => {
                        eprintln!("native-child lifecycle stage={stage} failed: {error}");
                        *failure.borrow_mut() = Some(error);
                        crate::native_child::hide(&state);
                        let _ = slint::quit_event_loop();
                    }
                }
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
        if let Some(error) = *self.failure.borrow() {
            return Err(error);
        }
        if self.completed.get() != STAGES.len() {
            return Err("native-child lifecycle ended before all checkpoints");
        }
        Ok(())
    }
}

#[cfg(not(target_os = "macos"))]
fn step(_: u64, _: &App, _: &UiState, _: &Cell<u64>, _: &Cell<f64>) -> Result<(), &'static str> {
    Err("native-child lifecycle is macOS-only")
}

#[cfg(target_os = "macos")]
fn step(
    stage: u64,
    app: &App,
    state: &UiState,
    publications: &Cell<u64>,
    volume: &Cell<f64>,
) -> Result<(), &'static str> {
    use slint::winit_030::WinitWindowAccessor;
    let snapshot = state.player.snapshot();
    let stats = crate::native_child::statistics(state).ok_or("native presenter was not created")?;
    if snapshot.error.is_some() {
        return Err("native media reported an error");
    }
    if snapshot.file_loads != 1 {
        return Err("lifecycle unexpectedly recreated or failed to load media");
    }
    // Require new actual publication after a hide, not merely retained counters.
    let revealed = || {
        if !stats.visible || stats.publications <= publications.get() {
            Err("child did not freshly publish after the suppression boundary")
        } else {
            Ok(())
        }
    };
    let mark_hidden = || {
        if crate::native_child::statistics(state).is_none_or(|value| value.visible) {
            Err("child was not hidden synchronously")
        } else {
            publications.set(stats.publications);
            Ok(())
        }
    };
    match stage {
        5 => {
            // Exact expected observations come from the same admitted MP4 in
            // artifacts/raster-native-v2/local.log; no requested option counts.
            if snapshot.duration < 60.
                || snapshot.width != 1920
                || snapshot.height != 1080
                || !snapshot.fps.is_finite()
                || (snapshot.fps - 60.).abs() > 0.1
                || snapshot.hwdec_current != "videotoolbox"
                || snapshot.audio_codec != "aac"
                || snapshot.audio_output != "avfoundation"
                || snapshot.audio_sample_rate != 48000
                || snapshot.diagnostic_silent_audio
                || snapshot.paused
                || !matches!(snapshot.state, serein_media::PlaybackState::Playing)
                || !snapshot.mute_observed
                || snapshot.muted
                || !app.get_mute_known()
                || app.get_muted()
                || !stats.visible
                || stats.renders == 0
                || stats.publications == 0
                || stats.backing_width == 0
                || stats.backing_height == 0
            {
                return Err(
                    "fixed fixture did not reach expected VideoToolbox 1080p60 and AVFoundation AAC/48kHz playback",
                );
            }
            volume.set(snapshot.volume);
            app.invoke_mute();
            app.invoke_toggle_pause();
        }
        8 => {
            if !snapshot.paused
                || snapshot.display_clock_active
                || !snapshot.mute_observed
                || !snapshot.muted
                || !app.get_muted()
                || snapshot.volume != volume.get()
            {
                return Err("pause/mute did not settle with volume preserved");
            }
            app.invoke_mute();
            publications.set(stats.publications);
            app.invoke_seek(20.);
        }
        11 => {
            if !snapshot.paused
                || (snapshot.position - 20.).abs() > 0.5
                || !state.player.current_load_frame_ready()
                || !snapshot.mute_observed
                || snapshot.muted
                || app.get_muted()
                || snapshot.volume != volume.get()
            {
                return Err("paused seek did not produce the requested ready frame");
            }
            revealed()?;
            app.invoke_native_child_hide();
            app.set_show_info(true);
            mark_hidden()?;
        }
        14 => {
            if stats.visible || app.get_native_child_visible() {
                return Err("Info overlay exposed the child");
            }
            app.set_show_info(false);
        }
        15 => {
            revealed()?;
            app.invoke_show_status_message();
            mark_hidden()?;
        }
        16 => {
            if !app.get_status_message_open() || stats.visible || app.get_native_child_visible() {
                return Err("readable status popup did not hide the native child");
            }
            let escape: slint::SharedString = slint::platform::Key::Escape.into();
            app.window()
                .dispatch_event(slint::platform::WindowEvent::KeyPressed {
                    text: escape.clone(),
                });
            app.window()
                .dispatch_event(slint::platform::WindowEvent::KeyReleased { text: escape });
        }
        17 => {
            if app.get_status_message_open() {
                return Err("Escape did not close the readable status popup");
            }
            revealed()?;
            app.invoke_toggle_pause();
            app.invoke_fullscreen();
            mark_hidden()?;
        }
        20 => {
            revealed()?;
            if !app.get_fullscreen_active()
                || !app.window().is_fullscreen()
                || app.get_video_height() > app.get_native_clip_height() - 57.
            {
                return Err("fullscreen did not reserve its shared Slint control strip");
            }
            if snapshot.paused || !matches!(snapshot.state, serein_media::PlaybackState::Playing) {
                return Err("resume did not restore observed playback");
            }
            app.invoke_toggle_pause();
            app.invoke_exit_fullscreen();
            mark_hidden()?;
        }
        23 => {
            revealed()?;
            if app.get_fullscreen_active() || !snapshot.paused {
                return Err("fullscreen exit/pause did not settle");
            }
            app.invoke_show_captions();
            mark_hidden()?;
        }
        26 => {
            if stats.visible || app.get_native_child_visible() {
                return Err("caption popup exposed the child");
            }
            let escape: slint::SharedString = slint::platform::Key::Escape.into();
            app.window()
                .dispatch_event(slint::platform::WindowEvent::KeyPressed {
                    text: escape.clone(),
                });
            app.window()
                .dispatch_event(slint::platform::WindowEvent::KeyReleased { text: escape });
        }
        29 => {
            revealed()?;
            app.invoke_navigate(3);
            mark_hidden()?;
        }
        32 => {
            if app.get_page() != 3 || stats.visible {
                return Err("Settings navigation did not hide video");
            }
            app.invoke_navigate(2);
            app.window().set_size(slint::LogicalSize::new(960., 700.));
        }
        35 => {
            revealed()?;
            let resized = app.window().with_winit_window(|window| {
                let size = window.inner_size().to_logical::<f64>(window.scale_factor());
                (size.width - 960.).abs() < 1. && (size.height - 700.).abs() < 1.
            }) == Some(true);
            if app.get_page() != 2
                || !resized
                || stats.backing_width == 0
                || stats.backing_height == 0
            {
                return Err("resized watch view has no native backing target");
            }
            app.invoke_native_child_hide();
            app.window()
                .with_winit_window(|window| window.set_minimized(true));
            mark_hidden()?;
        }
        38 => {
            if app
                .window()
                .with_winit_window(|window| window.is_minimized())
                .flatten()
                != Some(true)
                || stats.visible
                || snapshot.display_clock_active
            {
                return Err("minimize did not leave child hidden and media clock stopped");
            }
            app.window().with_winit_window(|window| {
                window.set_minimized(false);
                window.focus_window();
            });
        }
        41 => {
            revealed()?;
            if !snapshot.paused {
                return Err("lifecycle lost user pause");
            }
            crate::native_child::hide(state);
            state
                .player
                .stop()
                .map_err(|_| "native stop command failed")?;
            app.set_loaded(false);
            mark_hidden()?;
        }
        44 => {
            if stats.visible
                || snapshot.stop_pending
                || snapshot.display_clock_active
                || state.player.current_load_frame_ready()
            {
                return Err("stop did not settle hidden without a usable old frame");
            }
        }
        _ => return Err("unknown lifecycle checkpoint"),
    }
    eprintln!(
        "native-child lifecycle observed stage={stage} state={:?} paused={} position={:.3} decoder={} native_visible={} native_publications={} native_renders={} ui_draws={}",
        snapshot.state,
        snapshot.paused,
        snapshot.position,
        snapshot.hwdec_current,
        stats.visible,
        stats.publications,
        stats.renders,
        state.draw_callbacks.get()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn incomplete_and_failed_diagnostics_cannot_report_success() {
        let diagnostic = |completed, failure| Smoke {
            completed: Rc::new(Cell::new(completed)),
            failure: Rc::new(RefCell::new(failure)),
            _timers: Vec::new(),
        };
        assert!(diagnostic(STAGES.len() - 1, None).finish().is_err());
        assert_eq!(
            diagnostic(STAGES.len(), Some("observed failure")).finish(),
            Err("observed failure")
        );
        assert!(diagnostic(STAGES.len(), None).finish().is_ok());
    }
}
