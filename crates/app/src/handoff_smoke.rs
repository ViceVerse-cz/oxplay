// SPDX-License-Identifier: GPL-3.0-or-later
//! Finite local-only presentation diagnostic. No credentials or network requests.
use crate::{App, UiState};
use slint::{ComponentHandle, Timer, TimerMode};
use std::{cell::Cell, path::PathBuf, rc::Rc, time::Duration};

pub struct Smoke {
    verified: Rc<Cell<bool>>,
    _timers: Vec<Timer>,
}
impl Smoke {
    pub fn start(app: &App, state: &Rc<UiState>, video: PathBuf, audio: PathBuf) -> Self {
        let verified = Rc::new(Cell::new(false));
        let mut timers = Vec::new();
        for stage in [5, 8, 10, 15, 18, 21, 23, 28] {
            let weak = app.as_weak();
            let state = Rc::downgrade(state);
            let video = video.clone();
            let audio = audio.clone();
            let verified = verified.clone();
            let timer = Timer::default();
            timer.start(TimerMode::SingleShot, Duration::from_secs(stage), move || {
                let (Some(app), Some(state)) = (weak.upgrade(), state.upgrade()) else { return };
                let snapshot = state.player.snapshot();
                assert!(snapshot.error.is_none(), "handoff diagnostic engine failure");
                match stage {
                    5 => {
                        assert_eq!(snapshot.file_loads, 1);
                        assert!(state.player.current_load_frame_ready());
                        assert!(app.get_video_texture().size().width > 0);
                        state.player.load_local_at(&audio, 0., true).unwrap();
                        app.set_video_texture(slint::Image::default());
                        app.set_video_title("TEST FIXTURE — audio-only handoff".into());
                        app.window().set_size(slint::LogicalSize::new(960., 700.));
                    }
                    8 => {
                        assert_eq!(snapshot.file_loads, 2);
                        assert!(snapshot.paused);
                        assert!(state.player.current_load_is_active());
                        assert_eq!((snapshot.width, snapshot.height), (0, 0));
                        assert!(snapshot.codec.is_empty());
                        assert!(snapshot.hwdec_current.is_empty() || snapshot.hwdec_current == "no");
                        assert!(!app.get_diagnostic_warning().contains("Software video"));
                        assert!(!state.player.current_load_frame_ready(), "audio-only load inherited video readiness");
                        assert_eq!(app.get_video_texture().size().width, 0, "audio-only resize exposed the previous video");
                    }
                    10 | 23 => {
                        state.player.load_local_at(&video, 1., true).unwrap();
                        app.set_video_texture(slint::Image::default());
                        app.set_loaded(true);
                        app.set_video_title("TEST FIXTURE — paused video handoff".into());
                        app.window().set_size(slint::LogicalSize::new(1100., 760.));
                    }
                    15 | 28 => {
                        assert_eq!(snapshot.file_loads, if stage == 15 { 3 } else { 4 });
                        assert!(snapshot.paused);
                        assert!(state.player.current_load_frame_ready(), "paused first frame was never admitted");
                        assert!(app.get_video_texture().size().width > 0);
                        if stage == 28 {
                            assert_eq!(state.model.changes.get(), 0);
                            assert_eq!(state.model.resets.get(), 0);
                            verified.set(true);
                        }
                    }
                    18 => {
                        state.player.stop().unwrap();
                        app.set_loaded(false);
                        app.set_video_texture(slint::Image::default());
                        app.set_video_title("".into());
                        app.set_video_channel("".into());
                        crate::update(&app, &state);
                    }
                    21 => {
                        assert!(!snapshot.stop_pending);
                        assert!(!state.player.current_load_frame_ready());
                        assert_eq!(app.get_video_texture().size().width, 0);
                        assert_eq!(app.get_position(), 0.);
                        assert_eq!(app.get_duration(), 1.);
                        assert_eq!(app.get_elapsed().as_str(), "0:00");
                        assert_eq!(app.get_remaining().as_str(), "0:00");
                    }
                    _ => unreachable!(),
                }
                eprintln!("handoff smoke stage={stage} loads={} paused={} frame_ready={} texture_width={}",
                    snapshot.file_loads, snapshot.paused,
                    state.player.current_load_frame_ready(), app.get_video_texture().size().width);
            });
            timers.push(timer);
        }
        Self {
            verified,
            _timers: timers,
        }
    }
    pub fn finish(self) -> Result<(), &'static str> {
        self.verified
            .get()
            .then_some(())
            .ok_or("handoff diagnostic ended before all assertions completed")
    }
}
