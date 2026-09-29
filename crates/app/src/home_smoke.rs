// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit offline functional exercise. Fixture creation runs before the UI;
//! all later database operations use the ordinary library worker. Synthetic
//! identifiers never enter a provider or player request.
use crate::{App, LibraryUi, UiState, home_ui, library};
use serein_core::{VideoId, VideoSummary};
use serein_storage::{LocalPlaylistId, LocalStore};
use slint::{ComponentHandle, Model, Timer, TimerMode};
use std::{
    cell::{Cell, RefCell},
    fs::OpenOptions,
    io::{self, Write},
    path::{Path, PathBuf},
    rc::{Rc, Weak},
    time::{Duration, Instant},
};

const COUNT: usize = 205;
const NAME: &str = "TEST FIXTURE — offline Home saved videos";
const IMPORT_NAME: &str = "TEST FIXTURE — offline Home import";
const COMPLETE: usize = 11;
const DEADLINE: Duration = Duration::from_secs(35);

pub struct Fixture {
    playlist: LocalPlaylistId,
    import: PathBuf,
}

/// Must run before creating the Slint application or its library worker. Refuse
/// every existing root; neither the normal profile nor an arbitrary preexisting
/// database can become this diagnostic's mutation target.
pub fn prepare_root(root: &Path) -> io::Result<Fixture> {
    crate::clear_smoke::create_root(root)?;
    let directory = root.join("Serein");
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().mode(0o700).create(&directory)?;
    }
    #[cfg(not(unix))]
    return Err(io::Error::other(
        "Private Home fixture creation is not validated here",
    ));
    let store = LocalStore::open(directory.join("library.sqlite3")).map_err(io::Error::other)?;
    let playlist = store.create_playlist(NAME).map_err(io::Error::other)?;
    for index in 0..COUNT {
        store
            .save_video(playlist, &video(index))
            .map_err(io::Error::other)?;
    }
    drop(store);

    let source = LocalStore::in_memory().map_err(io::Error::other)?;
    let imported = source
        .create_playlist(IMPORT_NAME)
        .map_err(io::Error::other)?;
    source
        .save_video(imported, &video(COUNT + 1))
        .map_err(io::Error::other)?;
    let bytes = source.export_library_json().map_err(io::Error::other)?;
    if bytes.len() > 4096 {
        return Err(io::Error::other(
            "Home import fixture exceeded its fixed byte limit",
        ));
    }
    let import = root.join("home-import.json");
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(&import)?.write_all(&bytes)?;
    Ok(Fixture { playlist, import })
}

fn video(index: usize) -> VideoSummary {
    VideoSummary {
        id: VideoId::new(&format!("h{index:010}")).expect("fixed valid fixture identifier"),
        title: format!("TEST FIXTURE — saved Home video {index:03}"),
        channel: "TEST FIXTURE — offline local collection".into(),
        channel_id: None,
        duration: Some(Duration::from_secs(60 + index as u64)),
        thumbnail_url: None,
    }
}

struct Driver {
    app: slint::Weak<App>,
    state: Weak<UiState>,
    fixture: Fixture,
    timer: Timer,
    started: Instant,
    phase_started: Cell<Instant>,
    stage: Cell<usize>,
    unchanged: Cell<Option<(u64, u64, u64)>>,
    failure: RefCell<Option<&'static str>>,
}

pub struct Smoke(Rc<Driver>);

impl Smoke {
    pub fn start(app: &App, state: &Rc<UiState>, fixture: Fixture) -> Self {
        app.set_diagnostic_fixture_label("TEST FIXTURE — offline saved-video Home exercise".into());
        let started = Instant::now();
        let driver = Rc::new(Driver {
            app: app.as_weak(),
            state: Rc::downgrade(state),
            fixture,
            timer: Timer::default(),
            started,
            phase_started: Cell::new(started),
            stage: Cell::new(0),
            unchanged: Cell::new(None),
            failure: RefCell::new(None),
        });
        // No synthetic video selection is necessary to validate the local feed.
        // Fail visibly instead of forwarding accidental fixture interactions.
        let weak = Rc::downgrade(&driver);
        app.on_select_video(move |_| {
            if let Some(driver) = weak.upgrade() {
                driver.fail("Synthetic Home selection must not reach a provider");
            }
        });
        let weak = Rc::downgrade(&driver);
        app.on_search(move |_| {
            if let Some(driver) = weak.upgrade() {
                driver.fail("The offline Home exercise does not admit network search");
            }
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
            return Err("Offline Home exercise ended before all functional stages completed");
        }
        Ok(())
    }
}

impl Driver {
    fn schedule(driver: &Rc<Self>) {
        let weak = Rc::downgrade(driver);
        // Explicit finite UI automation; observes delivered worker replies and
        // issues no recurring database reads, redraws or resource sampling.
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
            "offline-home stage={} failed: {error}",
            self.stage.get() + 1
        );
        let _ = slint::quit_event_loop();
    }

    fn tick(self: &Rc<Self>) {
        let (Some(app), Some(state)) = (self.app.upgrade(), self.state.upgrade()) else {
            self.fail("The owning UI disappeared during the Home exercise");
            return;
        };
        if self.started.elapsed() >= DEADLINE {
            self.fail("Offline Home exercise exceeded its finite deadline");
            return;
        }
        if self.phase_started.get().elapsed() < Duration::from_secs(1) {
            Self::schedule(self);
            return;
        }
        match self.step(&app, &state) {
            Ok(true) => {
                let completed = self.stage.get() + 1;
                self.stage.set(completed);
                self.phase_started.set(Instant::now());
                eprintln!(
                    "offline-home stage={completed} passed at_ms={}",
                    self.started.elapsed().as_millis()
                );
                if completed == COMPLETE {
                    eprintln!(
                        "offline-home completed: startup/pages/unchanged-refresh/navigation/save/remove/import/delete; synthetic local data only; no playback or resource measurement"
                    );
                    return;
                }
            }
            Ok(false) => {
                if self.phase_started.get().elapsed() >= Duration::from_secs(8) {
                    self.fail("The expected committed Home page did not arrive");
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
        if state.player.snapshot().file_loads != 0
            || app.get_loaded()
            || state.current_video.borrow().is_some()
        {
            return Err("Offline Home unexpectedly started playback");
        }
        if state.preferences.get().privacy.local_history
            || state.thumbnails.borrow().statistics().remote_started != 0
        {
            return Err("Offline Home changed privacy defaults or started remote thumbnails");
        }
        if app.get_home_error() {
            return Err("Home reported a storage error");
        }
        if app.get_page() != 0
            || !app.get_home_active()
            || app.get_home_loading()
            || app.global::<LibraryUi>().get_busy()
        {
            return Ok(false);
        }
        let Some(rows) = home_ui::acknowledged_videos(state) else {
            return Ok(false);
        };
        let expected: Vec<usize> = match self.stage.get() {
            2 | 4 => (5..105).rev().collect(),
            3 => (0..5).rev().collect(),
            6 => (106..206).rev().collect(),
            8 => std::iter::once(206).chain((106..205).rev()).collect(),
            _ => (105..205).rev().collect(),
        };
        if !matches_page(&rows, &expected) {
            return Ok(false);
        }
        if state.model.row_count() != rows.len() {
            return Err("Home model and committed page lengths disagree");
        }
        for (index, video) in rows.iter().enumerate() {
            let row = state
                .model
                .row_data(index)
                .ok_or("Home model omitted a committed row")?;
            if row.id.as_str() != video.id.as_str()
                || row.title.as_str() != video.title
                || row.kind != "Video"
            {
                return Err("Home displayed a row from another catalog or stale page");
            }
        }
        match self.stage.get() {
            0 | 9 => {
                let ticket = home_ui::accepted_read(state)
                    .ok_or("Home has no acknowledged database read")?;
                self.unchanged.set(Some((
                    ticket,
                    state.model.changes.get(),
                    state.model.resets.get(),
                )));
                app.invoke_navigate(0);
            }
            1 | 10 => {
                let (old_ticket, changes, resets) = self
                    .unchanged
                    .get()
                    .ok_or("Unchanged Home checkpoint missing")?;
                if home_ui::accepted_read(state) == Some(old_ticket) {
                    return Ok(false);
                }
                if state.model.changes.get() != changes || state.model.resets.get() != resets {
                    return Err("An unchanged Home read unnecessarily changed the catalog model");
                }
                if self.stage.get() == 1 {
                    app.invoke_more();
                }
            }
            2 => app.invoke_more(),
            3 => app.invoke_guest_previous(),
            4 => {
                app.invoke_navigate(3);
                if app.get_page() != 3 {
                    return Err("Settings navigation was not admitted");
                }
                app.invoke_navigate(0);
            }
            5 => self.submit(
                state,
                library::Request::Save(library::VideoSave {
                    serial: u64::MAX,
                    destination: library::SaveDestination::Existing(self.fixture.playlist),
                    video: video(COUNT),
                }),
            )?,
            6 => self.submit(
                state,
                library::Request::Remove(self.fixture.playlist, video(COUNT).id),
            )?,
            7 => self.submit(state, library::Request::Import(self.fixture.import.clone()))?,
            8 => {
                let imported = state
                    .playlists
                    .borrow()
                    .iter()
                    .find(|p| p.name == IMPORT_NAME)
                    .map(|p| p.id);
                let Some(imported) = imported else {
                    return Ok(false);
                };
                self.submit(state, library::Request::Delete(imported))?;
            }
            _ => return Err("Unexpected offline Home stage"),
        }
        Ok(true)
    }

    fn submit(&self, state: &UiState, request: library::Request) -> Result<(), &'static str> {
        // Deliberate fixture mutation through the production worker. This does
        // not claim a Save dialog or native file-picker interaction.
        state
            .library
            .submit(request)
            .then_some(())
            .ok_or("The local mutation queue rejected the fixture operation")
    }
}

fn matches_page(rows: &[VideoSummary], expected: &[usize]) -> bool {
    rows.len() == expected.len()
        && rows.iter().zip(expected).all(|(row, index)| {
            let expected = video(*index);
            row.id == expected.id
                && row.title == expected.title
                && row.channel == expected.channel
                && row.duration == expected.duration
                && row.channel_id.is_none()
                && row.thumbnail_url.is_none()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn exact_page_identity_rejects_reordered_stale_and_remote_rows() {
        let mut rows = vec![video(2), video(1)];
        assert!(matches_page(&rows, &[2, 1]));
        assert!(!matches_page(&rows, &[1, 2]));
        assert!(!matches_page(&rows, &[2]));
        rows[0].thumbnail_url = Some("https://example.invalid/fixture".into());
        assert!(!matches_page(&rows, &[2, 1]));
    }

    #[test]
    #[cfg(unix)]
    fn fresh_private_fixture_uses_real_pages_and_import_without_reusing_roots() {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "serein-home-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let fixture = prepare_root(&root).unwrap();
        assert!(prepare_root(&root).is_err());
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&fixture.import)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let mut store = LocalStore::open(root.join("Serein/library.sqlite3")).unwrap();
        let first = store.recently_saved_videos(None, 100).unwrap();
        assert!(matches_page(
            &first.items,
            &(105..205).rev().collect::<Vec<_>>()
        ));
        let second = store.recently_saved_videos(first.next, 100).unwrap();
        assert!(matches_page(
            &second.items,
            &(5..105).rev().collect::<Vec<_>>()
        ));
        let third = store.recently_saved_videos(second.next, 100).unwrap();
        assert!(matches_page(
            &third.items,
            &(0..5).rev().collect::<Vec<_>>()
        ));
        assert!(third.next.is_none());
        let imported = store
            .import_library_json(&std::fs::read(&fixture.import).unwrap())
            .unwrap();
        assert_eq!(imported.videos_saved, 1);
        assert_eq!(
            store.recently_saved_videos(None, 1).unwrap().items[0].id,
            video(206).id
        );
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }
}
