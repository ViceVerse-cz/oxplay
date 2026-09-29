// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit offline preference/restart diagnostic; never loads provider content.
use crate::{App, LibraryUi, UiState, playback_preferences};
use serein_core::{PlaybackPreferences, PlaybackSpeed, QualityCeiling};
use slint::ComponentHandle;
use std::{cell::Cell, io, path::Path, rc::Rc, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Write,
    Verify,
}
impl Phase {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "write" => Some(Self::Write),
            "verify" => Some(Self::Verify),
            _ => None,
        }
    }
    fn stages(self) -> &'static [u64] {
        match self {
            Self::Write => &[3, 8, 12, 20],
            Self::Verify => &[3, 6, 11, 13, 21],
        }
    }
}

const MARKER: &str = ".serein-preferences-diagnostic-v1";
const CONTENT: &[u8] = b"Serein isolated preferences diagnostic v1\n";
/// Startup-only filesystem admission, before any UI, worker, or credential work.
/// Verify can clear only a private root explicitly created by the write phase.
pub fn prepare_root(path: &Path, phase: Phase) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::{
            fs::OpenOptions,
            io::{Read, Write},
            os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
        };
        if phase == Phase::Write {
            crate::clear_smoke::create_root(path)?;
            let mut marker = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(path.join(MARKER))?;
            marker.write_all(CONTENT)?;
            marker.sync_all()?;
        }
        let root = std::fs::symlink_metadata(path)?;
        if !path.is_absolute()
            || !root.is_dir()
            || root.permissions().mode() & 0o777 != 0o700
            || root.uid() != unsafe { libc::geteuid() }
        {
            return Err(io::Error::other(
                "Preferences diagnostic requires its private isolated root",
            ));
        }
        let marker = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(path.join(MARKER))?;
        let metadata = marker.metadata()?;
        if !metadata.is_file()
            || metadata.permissions().mode() & 0o777 != 0o600
            || metadata.uid() != root.uid()
            || metadata.nlink() != 1
            || metadata.len() != CONTENT.len() as u64
        {
            return Err(io::Error::other("Preferences diagnostic marker is invalid"));
        }
        let mut bytes = Vec::new();
        marker
            .take(CONTENT.len() as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes != CONTENT {
            return Err(io::Error::other("Preferences diagnostic marker is invalid"));
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (path, phase);
        Err(io::Error::other(
            "Private preferences diagnostic is not validated on this platform",
        ))
    }
}

pub struct Smoke {
    completed: Rc<Cell<usize>>,
    phase: Phase,
    _timers: Vec<slint::Timer>,
}
fn assert_values(
    app: &App,
    state: &UiState,
    quality: QualityCeiling,
    speed: PlaybackSpeed,
    volume: u8,
) {
    let expected = PlaybackPreferences { quality, speed };
    assert!(state.playback_preferences.ready());
    assert_eq!(
        state.preferences.get().playback,
        expected,
        "worker did not acknowledge playback defaults"
    );
    assert_eq!(state.preferences.get().volume_percent, volume);
    assert_eq!(app.get_default_quality_index(), quality.index());
    assert_eq!(app.get_speed_index(), speed.index());
    assert!(!app.get_speed_busy());
    let snapshot = state.player.snapshot();
    assert!(snapshot.speed_observed);
    assert_eq!(snapshot.speed, speed.rate());
    assert!((snapshot.volume - f64::from(volume)).abs() < 0.01);
    let policy = playback_preferences::policy(state);
    assert_eq!(
        policy.max_height,
        quality.height(),
        "next-selection quality policy lost the preference"
    );
    assert!(policy.prefer_h264);
    assert!(!state.preferences.get().privacy.local_history);
}
fn change(app: &App, quality: QualityCeiling, speed: PlaybackSpeed, volume: u8) {
    app.invoke_navigate(3);
    app.invoke_default_quality(quality.index());
    app.invoke_speed(speed.rate() as f32);
    app.invoke_volume(f32::from(volume));
}
impl Smoke {
    pub fn start(app: &App, state: &Rc<UiState>, phase: Phase) -> Self {
        let completed = Rc::new(Cell::new(0));
        // Page navigation deliberately cancels old catalog work. Capture the
        // baseline after that real callback; later settings callbacks remain
        // on this page and must not submit/cancel provider work themselves.
        app.invoke_navigate(3);
        let worker_generation = state.worker.borrow().generation();
        let timers = phase.stages().iter().copied().enumerate().map(|(index, stage)| {
            let weak = app.as_weak();
            let state = Rc::downgrade(state);
            let completed = completed.clone();
            let timer = slint::Timer::default();
            timer.start(slint::TimerMode::SingleShot, Duration::from_secs(stage), move || {
                let (Some(app), Some(state)) = (weak.upgrade(), state.upgrade()) else { return };
                assert_eq!(completed.get(), index, "preferences smoke skipped a stage");
                assert!(state.playback_preferences.ready(), "preferences did not hydrate");
                assert!(!app.get_account_connected());
                assert_eq!(state.player.snapshot().file_loads, 0, "offline diagnostic loaded media");
                match (phase, stage) {
                    (Phase::Write, 3) => {
                        assert_eq!(state.preferences.get(), serein_storage::LocalPreferences::default());
                        change(&app, QualityCeiling::P720, PlaybackSpeed::OneAndHalf, 37);
                    }
                    (Phase::Write, _) | (Phase::Verify, 3) => {
                        assert_values(&app, &state, QualityCeiling::P720, PlaybackSpeed::OneAndHalf, 37);
                        assert_eq!(state.worker.borrow().generation(), worker_generation, "settings started provider work");
                    }
                    (Phase::Verify, 6) => change(&app, QualityCeiling::P480, PlaybackSpeed::Double, 62),
                    (Phase::Verify, 11) => assert_values(&app, &state, QualityCeiling::P480, PlaybackSpeed::Double, 62),
                    (Phase::Verify, 13) => {
                        app.global::<LibraryUi>().invoke_clear_local();
                        assert!(state.caption_cache.active(), "clear was not admitted");
                        app.invoke_default_quality(QualityCeiling::P144.index());
                        app.invoke_speed(0.5);
                        assert!(state.caption_cache.active(), "preference callback bypassed clear barrier");
                    }
                    (Phase::Verify, 21) => {
                        assert!(!state.caption_cache.active(), "clear did not finish");
                        assert!(app.global::<LibraryUi>().get_status().starts_with("Local library and cached captions cleared."));
                        assert_eq!(state.preferences.get(), serein_storage::LocalPreferences::default());
                        assert_values(&app, &state, QualityCeiling::P1080, PlaybackSpeed::Normal, 100);
                    }
                    _ => unreachable!(),
                }
                completed.set(index + 1);
                eprintln!("preferences smoke phase={phase:?} stage={stage} quality={} speed={} volume={} clearing={}", state.preferences.get().playback.quality.height(), state.player.snapshot().speed, state.preferences.get().volume_percent, state.caption_cache.active());
            });
            timer
        }).collect();
        Self {
            completed,
            phase,
            _timers: timers,
        }
    }
    pub fn finish(self) -> Result<(), &'static str> {
        (self.completed.get() == self.phase.stages().len())
            .then_some(())
            .ok_or("preferences smoke ended before all assertions completed")
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn phase_parser_and_finish_require_explicit_complete_run() {
        assert_eq!(Phase::parse("write"), Some(Phase::Write));
        assert_eq!(Phase::parse("verify"), Some(Phase::Verify));
        assert_eq!(Phase::parse("clear"), None);
        for phase in [Phase::Write, Phase::Verify] {
            for completed in 0..=phase.stages().len() {
                let smoke = Smoke {
                    completed: Rc::new(Cell::new(completed)),
                    phase,
                    _timers: Vec::new(),
                };
                assert_eq!(smoke.finish().is_ok(), completed == phase.stages().len());
            }
        }
    }
    #[test]
    fn diagnostic_reuse_requires_private_regular_exact_marker() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let path = std::env::temp_dir().join(format!(
            "serein-preferences-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        assert!(prepare_root(&path, Phase::Verify).is_err());
        prepare_root(&path, Phase::Write).unwrap();
        assert!(prepare_root(&path, Phase::Write).is_err());
        prepare_root(&path, Phase::Verify).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(prepare_root(&path, Phase::Verify).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let marker = path.join(MARKER);
        std::fs::write(&marker, b"invalid").unwrap();
        assert!(prepare_root(&path, Phase::Verify).is_err());
        std::fs::remove_file(&marker).unwrap();
        use std::os::unix::ffi::OsStrExt;
        let fifo = std::ffi::CString::new(marker.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        assert!(prepare_root(&path, Phase::Verify).is_err());
        std::fs::remove_file(&marker).unwrap();
        // sockaddr_un has a much shorter pathname limit than the filesystem.
        // Bind a short sibling first, then move the socket entry to the marker.
        let socket_path = path.join("s");
        let socket = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();
        std::fs::rename(&socket_path, &marker).unwrap();
        assert!(prepare_root(&path, Phase::Verify).is_err());
        drop(socket);
        std::fs::remove_file(&marker).unwrap();
        symlink("missing", &marker).unwrap();
        assert!(prepare_root(&path, Phase::Verify).is_err());
        std::fs::remove_dir_all(path).unwrap();
    }
}
