// SPDX-License-Identifier: GPL-3.0-or-later
//! Finite offline collection-window callbacks against a newly created private
//! database. No helper, media, account or resource-measurement admission.
use crate::{App, LibraryUi, UiState};
use oxplay_storage::{LocalPlaylistId, LocalStore};
use slint::{ComponentHandle, Model, Timer, TimerMode};
use std::{
    cell::{Cell, RefCell},
    io,
    path::Path,
    rc::Rc,
    time::Duration,
};

const STAGES: [u64; 14] = [3, 6, 10, 14, 18, 22, 26, 30, 34, 38, 42, 46, 50, 54];
const CREATED: &str = "TEST FIXTURE collection206";
const RENAMED: &str = "TEST FIXTURE renamed206";
const AWAY: &str = "TEST FIXTURE collection207 away";
fn name(index: usize) -> String {
    format!("TEST FIXTURE collection{index:03}")
}
pub struct Fixture {
    seed: Vec<LocalPlaylistId>,
}
pub fn prepare_root(root: &Path) -> io::Result<Fixture> {
    crate::clear_smoke::create_root(root)?;
    let directory = root.join("Oxplay");
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().mode(0o700).create(&directory)?;
    }
    #[cfg(not(unix))]
    return Err(io::Error::other(
        "Private collection fixture is unsupported on this platform",
    ));
    let store = LocalStore::open(directory.join("library.sqlite3")).map_err(io::Error::other)?;
    let seed = (1..=205)
        .map(|index| store.create_playlist(&name(index)))
        .collect::<Result<Vec<_>, _>>()
        .map_err(io::Error::other)?;
    drop(store);
    Ok(Fixture { seed })
}
struct Progress {
    fixture: Fixture,
    completed: Cell<usize>,
    failure: RefCell<Option<&'static str>>,
    created: Cell<Option<LocalPlaylistId>>,
    away: Cell<Option<LocalPlaylistId>>,
    previous_selection: Cell<Option<LocalPlaylistId>>,
}
impl Progress {
    fn fail(&self, error: &'static str) {
        self.failure.borrow_mut().get_or_insert(error);
    }
    fn finish(&self) -> Result<(), &'static str> {
        if let Some(error) = *self.failure.borrow() {
            return Err(error);
        }
        check(
            self.completed.get() == STAGES.len(),
            "Collection-window check ended before every stage completed",
        )
    }
}
pub struct Smoke {
    progress: Rc<Progress>,
    _timers: Vec<Timer>,
}
fn check(value: bool, error: &'static str) -> Result<(), &'static str> {
    value.then_some(()).ok_or(error)
}
fn settled(app: &App) -> Result<(), &'static str> {
    check(
        !app.global::<LibraryUi>().get_busy(),
        "Local collection request did not settle",
    )
}
fn selected(app: &App, state: &UiState) -> Option<LocalPlaylistId> {
    let index = usize::try_from(app.global::<LibraryUi>().get_selected()).ok()?;
    state.playlists.borrow().get(index).map(|item| item.id)
}
fn shown_selection(app: &App) -> bool {
    let ui = app.global::<LibraryUi>();
    usize::try_from(ui.get_selected())
        .ok()
        .and_then(|index| ui.get_collections().row_data(index))
        .is_some_and(|name| {
            ui.get_collection_visible_index() == ui.get_selected()
                && ui.get_collection_visible_value() == name
        })
}
fn expected(progress: &Progress, first: usize, last: usize) -> Vec<(LocalPlaylistId, String)> {
    (first..=last)
        .map(|index| (progress.fixture.seed[index - 1], name(index)))
        .collect()
}
fn window(
    app: &App,
    state: &UiState,
    items: &[(LocalPlaylistId, String)],
    previous: bool,
    next: bool,
) -> Result<(), &'static str> {
    check(
        items.len() <= 100,
        "Expected collection window exceeded its fixed bound",
    )?;
    let actual = state.playlists.borrow();
    check(
        actual.len() == items.len()
            && actual
                .iter()
                .zip(items)
                .all(|(a, (id, name))| a.id == *id && a.name == *name),
        "Acknowledged collection window has wrong members or ordering",
    )?;
    drop(actual);
    let ui = app.global::<LibraryUi>();
    check(
        ui.get_collections().row_count() == items.len()
            && ui.get_collections_previous() == previous
            && ui.get_collections_next() == next,
        "Collection window controls do not match acknowledged bounds",
    )
}
fn last_created(
    progress: &Progress,
    renamed: bool,
) -> Result<Vec<(LocalPlaylistId, String)>, &'static str> {
    let mut rows = expected(progress, 107, 205);
    rows.push((
        progress
            .created
            .get()
            .ok_or("Created identity was not captured")?,
        if renamed { RENAMED } else { CREATED }.into(),
    ));
    Ok(rows)
}
fn last_away(progress: &Progress) -> Result<Vec<(LocalPlaylistId, String)>, &'static str> {
    let mut rows = expected(progress, 107, 205);
    rows.push((
        progress
            .away
            .get()
            .ok_or("Away-created identity was not captured")?,
        AWAY.into(),
    ));
    Ok(rows)
}
fn page(app: &App, next: bool) -> Result<(), &'static str> {
    app.global::<LibraryUi>().invoke_collection_page(next);
    check(
        app.global::<LibraryUi>().get_busy(),
        "Collection page navigation was not admitted",
    )
}
fn create(app: &App, name: &str) -> Result<(), &'static str> {
    let ui = app.global::<LibraryUi>();
    ui.set_name_draft(name.into());
    ui.invoke_create(name.into());
    check(ui.get_busy(), "Create did not admit its real storage write")
}
fn stage(index: usize, app: &App, state: &UiState, p: &Progress) -> Result<(), &'static str> {
    settled(app)?;
    let ui = app.global::<LibraryUi>();
    match index {
        0 => {
            app.invoke_open_local_tab(0);
        }
        1 => {
            window(app, state, &expected(p, 1, 100), false, true)?;
            check(
                shown_selection(app),
                "Initial selector disagrees with acknowledged collection",
            )?;
            create(app, CREATED)?;
        }
        2 => {
            let rows = state.playlists.borrow();
            let item = rows.last().ok_or("Created page is empty")?;
            check(
                item.name == CREATED && !p.fixture.seed.contains(&item.id),
                "Created row was not found beyond the first100",
            )?;
            p.created.set(Some(item.id));
            drop(rows);
            window(app, state, &last_created(p, false)?, true, false)?;
            check(
                selected(app, state) == p.created.get() && shown_selection(app),
                "Created collection was not selected and displayed",
            )?;
            page(app, false)?;
        }
        3 => {
            window(app, state, &expected(p, 7, 106), true, true)?;
            page(app, false)?;
        }
        4 => {
            window(app, state, &expected(p, 1, 6), false, true)?;
            page(app, true)?;
        }
        5 => {
            window(app, state, &expected(p, 7, 106), true, true)?;
            page(app, true)?;
        }
        6 => {
            window(app, state, &last_created(p, false)?, true, false)?;
            ui.invoke_choose(99);
            check(
                ui.get_busy() || selected(app, state) == p.created.get(),
                "Created collection could not be selected after paging",
            )?;
        }
        7 => {
            check(
                selected(app, state) == p.created.get() && shown_selection(app),
                "Paging lost the created identity",
            )?;
            check(
                ui.invoke_begin_rename(),
                "Rename was not admitted for selected collection",
            )?;
            ui.set_name_draft(RENAMED.into());
            ui.invoke_rename(RENAMED.into());
            check(ui.get_busy(), "Rename did not admit its actual write")?;
        }
        8 => {
            window(app, state, &last_created(p, true)?, true, false)?;
            check(
                selected(app, state) == p.created.get(),
                "Rename changed selection identity",
            )?;
            ui.invoke_delete_collection();
            check(ui.get_busy(), "Delete did not admit its actual write")?;
        }
        9 => {
            window(app, state, &expected(p, 107, 205), true, false)?;
            p.previous_selection.set(selected(app, state));
            check(
                p.previous_selection.get().is_some(),
                "Delete left no acknowledged selection",
            )?;
            create(app, AWAY)?;
            app.invoke_navigate(3);
            app.invoke_focus_search();
            check(
                app.get_page() == 3 && app.get_search_active(),
                "Could not leave pending Create for Settings/search",
            )?;
        }
        10 => {
            check(
                app.get_page() == 3 && app.get_search_active(),
                "Late Create stole navigation or keyboard focus",
            )?;
            check(
                selected(app, state) == p.previous_selection.get(),
                "Late Create changed the acknowledged prior selection",
            )?;
            let rows = state.playlists.borrow();
            let item = rows.last().ok_or("Away refresh lost the collection page")?;
            check(
                item.name == AWAY
                    && !p.fixture.seed.contains(&item.id)
                    && Some(item.id) != p.created.get(),
                "Away Create was not committed/refreshed in the current window",
            )?;
            p.away.set(Some(item.id));
            drop(rows);
            window(app, state, &last_away(p)?, true, false)?;
            app.invoke_open_local_tab(0);
        }
        11 => {
            window(app, state, &last_away(p)?, true, false)?;
            check(
                selected(app, state) == p.previous_selection.get() && shown_selection(app),
                "Returning from Settings changed acknowledged selection",
            )?;
            page(app, false)?;
        }
        12 => {
            window(app, state, &expected(p, 7, 106), true, true)?;
            page(app, true)?;
        }
        13 => {
            window(app, state, &last_away(p)?, true, false)?;
            check(
                shown_selection(app),
                "Final selector lost its acknowledged display",
            )?;
            check(
                state.player.snapshot().file_loads == 0
                    && !app.get_account_connected()
                    && state.thumbnails.borrow().statistics().remote_started == 0,
                "Collection-only exercise admitted unrelated media/account/network work",
            )?;
        }
        _ => return Err("Unexpected collection-window stage"),
    }
    Ok(())
}
impl Smoke {
    pub fn start(app: &App, state: &Rc<UiState>, fixture: Fixture) -> Self {
        app.set_diagnostic_fixture_label("TEST FIXTURE — offline collection-window check".into());
        let progress = Rc::new(Progress {
            fixture,
            completed: Cell::new(0),
            failure: RefCell::new(None),
            created: Cell::new(None),
            away: Cell::new(None),
            previous_selection: Cell::new(None),
        });
        let weak = Rc::downgrade(&progress);
        app.on_search(move |_| {
            if let Some(progress) = weak.upgrade() {
                progress.fail("Offline collection diagnostic submitted search");
            }
        });
        let weak = Rc::downgrade(&progress);
        app.on_select_video(move |_| {
            if let Some(progress) = weak.upgrade() {
                progress.fail("Offline collection diagnostic selected media");
            }
        });
        let timers=STAGES.into_iter().enumerate().map(|(index,seconds)|{
            let app=app.as_weak();let state=Rc::downgrade(state);let progress=progress.clone();let timer=Timer::default();
            timer.start(TimerMode::SingleShot,Duration::from_secs(seconds),move||{
                if progress.failure.borrow().is_some(){return;}
                let (Some(app),Some(state))=(app.upgrade(),state.upgrade())else{progress.fail("Collection diagnostic window ended early");return;};
                if progress.completed.get()!=index{progress.fail("Collection diagnostic skipped a stage");return;}
                if let Err(error)=stage(index,&app,&state,&progress){progress.fail(error);eprintln!("collection window failed: stage={} reason={error}",index+1);return;}
                if progress.failure.borrow().is_some(){return;}
                progress.completed.set(index+1);
                eprintln!("collection window stage={} collection_rows={} page={} real_worker_callbacks=true",index+1,state.playlists.borrow().len(),app.get_page());
            });timer
        }).collect();
        Self {
            progress,
            _timers: timers,
        }
    }
    pub fn finish(self) -> Result<(), &'static str> {
        self.progress.finish()?;
        eprintln!(
            "collection window complete: stages=14 seeded_playlists=205 bounded_window=100 remote_thumbnail_starts=0 media_loads=0 account_connected=false"
        );
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completion_requires_all_finite_stages_and_retains_first_error() {
        let p = Progress {
            fixture: Fixture { seed: Vec::new() },
            completed: Cell::new(0),
            failure: RefCell::new(None),
            created: Cell::new(None),
            away: Cell::new(None),
            previous_selection: Cell::new(None),
        };
        for count in 0..STAGES.len() {
            p.completed.set(count);
            assert!(p.finish().is_err());
        }
        p.completed.set(STAGES.len());
        assert!(p.finish().is_ok());
        p.fail("first");
        p.fail("second");
        assert_eq!(p.finish(), Err("first"));
        assert!(STAGES.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(STAGES.last().unwrap() < &58);
    }
    #[cfg(unix)]
    #[test]
    fn fixture_is_bounded_empty_and_refuses_an_existing_root() {
        static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        struct Root(std::path::PathBuf);
        impl Drop for Root {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let root = Root(std::env::temp_dir().canonicalize().unwrap().join(format!(
            "oxplay-collection-window-fixture-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        )));
        let fixture = prepare_root(&root.0).unwrap();
        assert_eq!(fixture.seed.len(), 205);
        assert!(prepare_root(&root.0).is_err());
        let store = LocalStore::open(root.0.join("Oxplay/library.sqlite3")).unwrap();
        let first = store.playlists(None, 100).unwrap();
        let second = store.playlists(first.next, 100).unwrap();
        let third = store.playlists(second.next, 100).unwrap();
        assert_eq!(
            (first.items.len(), second.items.len(), third.items.len()),
            (100, 100, 5)
        );
        assert!(third.next.is_none());
        for (index, id) in fixture.seed.iter().enumerate() {
            assert!(
                store
                    .playlist_videos(*id, None, 1)
                    .unwrap()
                    .items
                    .is_empty()
            );
            assert_eq!(
                first.items.get(index).map(|p| p.id),
                if index < 100 { Some(*id) } else { None }
            );
        }
        assert!(
            store
                .recently_saved_videos(None, 100)
                .unwrap()
                .items
                .is_empty()
        );
    }
}
