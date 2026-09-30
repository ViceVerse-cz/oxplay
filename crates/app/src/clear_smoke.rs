// SPDX-License-Identifier: GPL-3.0-or-later
//! Destructive diagnostic confined to an explicitly selected, newly created root.
use crate::{App, CaptionsUi, LibraryUi, UiState};
use slint::{ComponentHandle, Model};
use std::{cell::Cell, path::Path, rc::Rc, time::Duration};

pub fn create_root(path: &Path) -> std::io::Result<()> {
    if !path.is_absolute() {
        return Err(std::io::Error::other(
            "Clear diagnostic requires an absolute new data root",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        // No recursive create and no reuse: an existing directory/symlink/file
        // fails before any application data or media operation is possible.
        std::fs::DirBuilder::new().mode(0o700).create(path)
    }
    #[cfg(not(unix))]
    {
        Err(std::io::Error::other(
            "Private clear diagnostic is not validated on this platform",
        ))
    }
}

pub struct Smoke {
    verified: Rc<Cell<bool>>,
    _timers: Vec<slint::Timer>,
}
impl Smoke {
    pub fn start(app: &App, state: &Rc<UiState>) -> Self {
        let verified = Rc::new(Cell::new(false));
        let before_clear = Rc::new(Cell::new(0));
        let timers = [5, 67, 80]
            .into_iter()
            .map(|stage| {
                let weak = app.as_weak();
                let state = Rc::downgrade(state);
                let verified = verified.clone();
                let before_clear = before_clear.clone();
                let timer = slint::Timer::default();
                timer.start(
                    slint::TimerMode::SingleShot,
                    Duration::from_secs(stage),
                    move || {
                        let (Some(app), Some(state)) = (weak.upgrade(), state.upgrade()) else {
                            return;
                        };
                        let library = app.global::<LibraryUi>();
                        match stage {
                            5 => {
                                assert!(!library.get_busy(), "isolated library did not initialize");
                                library.invoke_create(
                                    "TEST FIXTURE — clear-local-data diagnostic".into(),
                                );
                            }
                            67 => {
                                assert_eq!(
                                    state.playlists.borrow().len(),
                                    1,
                                    "fixture collection was not persisted"
                                );
                                assert!(
                                    state.player.snapshot().subtitle_id.is_some(),
                                    "native caption not selected before clear"
                                );
                                before_clear.set(state.player.snapshot().file_loads);
                                library.invoke_clear_local();
                                assert!(state.caption_cache.active(), "clear was not admitted");
                                // Exercise the catalog admission barrier through the
                                // same shared callback as a user submitting a search.
                                app.invoke_search(
                                    "TEST FIXTURE — this blocked search must not reach a provider"
                                        .into(),
                                );
                                assert!(
                                    state.caption_cache.active(),
                                    "search bypassed the clear barrier"
                                );
                            }
                            _ => {
                                assert!(!state.caption_cache.active(), "clear did not finish");
                                assert!(
                                    library.get_status().starts_with(
                                        "Local library, cached artwork and captions cleared."
                                    ),
                                    "clear did not report confirmed success"
                                );
                                assert!(
                                    !app.get_loaded() && !app.get_remote_video(),
                                    "clearing left playback active"
                                );
                                assert!(
                                    !state.player.snapshot().stop_pending,
                                    "native stop did not complete"
                                );
                                assert!(
                                    app.get_video_title().is_empty()
                                        && app.get_video_channel().is_empty()
                                );
                                assert_eq!(
                                    app.get_video_texture().size().width,
                                    0,
                                    "old texture was republished"
                                );
                                assert_eq!(
                                    app.get_position(),
                                    0.,
                                    "old media position was republished"
                                );
                                assert_eq!(
                                    app.get_duration(),
                                    1.,
                                    "old media duration was republished"
                                );
                                assert_eq!(app.get_elapsed().as_str(), "0:00");
                                assert_eq!(app.get_remaining().as_str(), "0:00");
                                assert_eq!(
                                    state.player.snapshot().file_loads,
                                    before_clear.get(),
                                    "blocked search started another file"
                                );
                                assert_eq!(
                                    app.global::<CaptionsUi>().get_tracks().row_count(),
                                    1,
                                    "caption tracks survived clearing"
                                );
                                assert!(
                                    state.playlists.borrow().is_empty(),
                                    "local collection survived clearing"
                                );
                                assert_eq!(
                                    state.preferences.get(),
                                    oxplay_storage::LocalPreferences::default(),
                                    "privacy defaults were not installed before admission reopened"
                                );
                                verified.set(true);
                            }
                        }
                        eprintln!(
                            "clear-local smoke stage={stage} active={} loads={}",
                            state.caption_cache.active(),
                            state.player.snapshot().file_loads
                        );
                    },
                );
                timer
            })
            .collect();
        Self {
            verified,
            _timers: timers,
        }
    }
    pub fn finish(self) -> Result<(), &'static str> {
        self.verified
            .get()
            .then_some(())
            .ok_or("clear-local smoke ended before its assertions completed")
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn destructive_diagnostic_refuses_existing_data_and_symlinks() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let root = std::env::temp_dir().join(format!(
            "oxplay-clear-scope-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        create_root(&root).unwrap();
        assert_eq!(
            std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        std::fs::write(root.join("must-remain"), b"existing data").unwrap();
        assert!(create_root(&root).is_err());
        let link = root.join("alias");
        symlink(&root, &link).unwrap();
        assert!(create_root(&link).is_err());
        assert_eq!(
            std::fs::read(root.join("must-remain")).unwrap(),
            b"existing data"
        );
        assert!(create_root(Path::new("relative")).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
