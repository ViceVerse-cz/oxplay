// SPDX-License-Identifier: GPL-3.0-or-later
//! Finite offline playlist form checks in a native window using injected Slint
//! key events. This does not test macOS event delivery, IME or accessibility APIs.
use crate::{App, LibraryUi, UiState};
use oxplay_storage::LocalPlaylistId;
use slint::{
    ComponentHandle, Model, SharedString, Timer, TimerMode,
    platform::{Key, WindowEvent},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

const STAGES: [u64; 22] = [
    3, 5, 8, 11, 14, 17, 20, 23, 26, 29, 32, 35, 38, 41, 44, 47, 50, 53, 56, 59, 62, 65,
];
const CREATED: &str = "TEST FIXTURE keyboard /mf";
const DRAFT: &str = "TEST FIXTURE separate create draft";
const RENAMED: &str = "TEST FIXTURE renamed";
const AWAY: &str = "TEST FIXTURE renamed while away";
const PAGE_DRAFT: &str = "TEST FIXTURE create draft retained across video pages";
const SAVES: usize = 101;
const SAVE_BATCH: usize = 20;

fn fixture_video(index: usize) -> oxplay_core::VideoSummary {
    oxplay_core::VideoSummary {
        metadata: None,
        id: oxplay_core::VideoId::new(&format!("k{index:010}")).unwrap(),
        title: format!("TEST FIXTURE keyboard pagination video {index:03}"),
        channel: "TEST FIXTURE offline local channel".into(),
        channel_id: None,
        duration: None,
        thumbnail_url: None,
    }
}
fn saved_page(state: &UiState, start: usize, end: usize) -> bool {
    crate::library_ui::acknowledged_video_page(state).is_some_and(|items| {
        items.len() == end - start
            && items.iter().zip(start..end).all(|(item, index)| {
                let expected = fixture_video(index);
                item.id == expected.id
                    && item.title == expected.title
                    && item.channel == expected.channel
                    && item.channel_id.is_none()
                    && item.duration.is_none()
                    && item.thumbnail_url.is_none()
            })
    })
}

#[derive(Default)]
struct Progress {
    completed: Cell<usize>,
    failure: RefCell<Option<&'static str>>,
    playlist: Cell<Option<LocalPlaylistId>>,
}
impl Progress {
    fn fail(&self, error: &'static str) {
        self.failure.borrow_mut().get_or_insert(error);
    }
    fn finish(&self) -> Result<(), &'static str> {
        if let Some(error) = *self.failure.borrow() {
            return Err(error);
        }
        if self.completed.get() != STAGES.len() {
            return Err("Playlist keyboard check did not finish every stage");
        }
        Ok(())
    }
}
pub struct Smoke {
    progress: Rc<Progress>,
    _timers: Vec<Timer>,
}

fn event(app: &App, event: WindowEvent) -> Result<(), &'static str> {
    app.window()
        .dispatch_event_with_result(event)
        .map(|_| ())
        .map_err(|_| "Slint key dispatch failed")
}
fn key(app: &App, text: impl Into<SharedString>) -> Result<(), &'static str> {
    let text = text.into();
    event(app, WindowEvent::KeyPressed { text: text.clone() })?;
    event(app, WindowEvent::KeyReleased { text })
}
fn type_text(app: &App, text: &str) -> Result<(), &'static str> {
    for character in text.chars() {
        key(app, character.to_string())?;
    }
    Ok(())
}
fn replace_text(app: &App, text: &str) -> Result<(), &'static str> {
    // The public Slint event carries Slint's semantic Control modifier. Winit's
    // macOS Command mapping is deliberately outside this injected-event check.
    event(
        app,
        WindowEvent::KeyPressed {
            text: Key::Control.into(),
        },
    )?;
    let result = key(app, "a");
    event(
        app,
        WindowEvent::KeyReleased {
            text: Key::Control.into(),
        },
    )?;
    result?;
    key(app, Key::Backspace)?;
    type_text(app, text)
}
fn focus(app: &App, target: i32) -> Result<(), &'static str> {
    // Observe the actual current control, not a hard-coded global Tab count.
    for _ in 0..32 {
        if app.global::<LibraryUi>().get_name_focus_target() == target {
            return Ok(());
        }
        key(app, Key::Tab)?;
    }
    Err("The playlist form control was not keyboard reachable")
}
/// Each fresh Library page starts with its editing tools collapsed. Activate
/// the real disclosure through its keyboard path before testing the name form.
fn open_editor(app: &App) -> Result<(), &'static str> {
    focus(app, 8)?;
    key(app, " ")?;
    check(
        app.global::<LibraryUi>().get_name_focus_target() == 1,
        "The playlist editing disclosure did not reveal and focus the name editor",
    )
}
fn check(condition: bool, error: &'static str) -> Result<(), &'static str> {
    condition.then_some(()).ok_or(error)
}
fn selected(app: &App, state: &UiState) -> Option<(LocalPlaylistId, String)> {
    state
        .playlists
        .borrow()
        .get(usize::try_from(app.global::<LibraryUi>().get_selected()).ok()?)
        .map(|playlist| (playlist.id, playlist.name.clone()))
}
fn selection_display_matches(app: &App) -> bool {
    let ui = app.global::<LibraryUi>();
    usize::try_from(ui.get_selected())
        .ok()
        .and_then(|index| ui.get_collections().row_data(index))
        .is_some_and(|name| {
            ui.get_collection_visible_index() == ui.get_selected()
                && ui.get_collection_visible_value() == name
        })
}
/// Exercise the component's rejected selection path without pretending the real
/// worker queue is saturated. Restore the production callback before returning.
fn reject_selection(app: &App) -> Result<(), &'static str> {
    focus(app, 7)?;
    let ui = app.global::<LibraryUi>();
    let acknowledged = ui.get_selected();
    check(
        ui.get_collections().row_count() == 2,
        "Selection rejection needs two actual playlists",
    )?;
    let attempted = Rc::new(Cell::new(None));
    let observed = attempted.clone();
    ui.on_choose(move |index| observed.set(Some(index)));
    let result = key(
        app,
        if acknowledged > 0 {
            Key::UpArrow
        } else {
            Key::DownArrow
        },
    );
    let weak = app.as_weak();
    ui.on_choose(move |index| {
        if let Some(app) = weak.upgrade() {
            app.invoke_choose_playlist(index);
        }
    });
    result?;
    check(
        attempted.get()
            == Some(if acknowledged > 0 {
                acknowledged - 1
            } else {
                1
            }),
        "Native component did not attempt a different keyboard selection",
    )?;
    check(
        ui.get_selected() == acknowledged && selection_display_matches(app) && !ui.get_busy(),
        "Rejected component selection changed its display or acknowledged route",
    )?;
    eprintln!(
        "library keyboard selector: injected_rejection=true real_queue_saturation=false acknowledged_display_restored=true"
    );
    Ok(())
}
fn settled(app: &App) -> Result<(), &'static str> {
    check(
        !app.global::<LibraryUi>().get_busy(),
        "The real local write/read did not settle before its checkpoint",
    )
}
fn stage(
    index: usize,
    app: &App,
    state: &UiState,
    progress: &Progress,
) -> Result<(), &'static str> {
    let ui = app.global::<LibraryUi>();
    match index {
        0 => {
            settled(app)?;
            check(
                state.playlists.borrow().is_empty(),
                "The keyboard check requires an empty private library",
            )?;
            app.invoke_open_local_tab(0);
        }
        1 => {
            settled(app)?;
            open_editor(app)?;
            focus(app, 1)?;
            key(app, Key::Return)?;
            check(
                !ui.get_busy()
                    && ui.get_name_draft().is_empty()
                    && !ui.get_name_status().is_empty(),
                "Empty Enter did not retain a local validation error",
            )?;
            check(
                ui.get_name_focus_target() == 1,
                "Validation moved focus away from the editor",
            )?;
            type_text(app, CREATED)?;
            check(
                ui.get_name_draft() == CREATED && !app.get_search_active(),
                "Playlist typing escaped the editor",
            )?;
            key(app, Key::Return)?;
            check(ui.get_busy(), "Enter did not admit a real create")?;
        }
        2 => {
            settled(app)?;
            let (id, name) = selected(app, state).ok_or("Created playlist was not selected")?;
            check(
                name == CREATED && state.playlists.borrow().len() == 1,
                "Create acknowledgment did not publish one exact playlist",
            )?;
            progress.playlist.set(Some(id));
            check(
                selection_display_matches(app),
                "First Create dropdown differs from acknowledged selection",
            )?;
            check(
                ui.get_name_draft().is_empty() && ui.get_name_focus_target() == 1,
                "Create completion lost editor focus or kept its committed draft",
            )?;
            type_text(app, DRAFT)?;
            focus(app, 3)?;
            key(app, " ")?;
            check(
                ui.get_renaming()
                    && ui.get_name_draft() == CREATED
                    && ui.get_name_focus_target() == 1,
                "Rename did not prefill and focus the actual selected name",
            )?;
            type_text(app, "TEST FIXTURE cancelled rename")?;
            check(
                ui.get_name_draft() == "TEST FIXTURE cancelled rename",
                "Rename prefill was not selected for replacement",
            )?;
            key(app, Key::Escape)?;
            check(
                !ui.get_renaming()
                    && ui.get_name_draft() == DRAFT
                    && ui.get_name_focus_target() == 1
                    && !ui.get_busy(),
                "Escape did not restore the separate create draft without a write",
            )?;
        }
        3 => {
            settled(app)?;
            check(
                selected(app, state).is_some_and(|(id, name)| {
                    Some(id) == progress.playlist.get() && name == CREATED
                }),
                "Cancelled rename changed the saved playlist",
            )?;
            focus(app, 3)?;
            key(app, " ")?;
            type_text(app, RENAMED)?;
            key(app, Key::Return)?;
            check(
                ui.get_busy() && ui.get_renaming(),
                "Enter did not admit rename",
            )?;
            key(app, Key::Escape)?;
            check(
                ui.get_renaming() && ui.get_busy(),
                "Escape cancelled an already accepted name write",
            )?;
        }
        4 => {
            settled(app)?;
            check(
                selected(app, state).is_some_and(|(id, name)| {
                    Some(id) == progress.playlist.get() && name == RENAMED
                }),
                "Rename did not keep the same real playlist identity",
            )?;
            check(
                ui.get_name_draft() == DRAFT && ui.get_name_focus_target() == 1,
                "Rename completion lost the create draft or editor focus",
            )?;
            focus(app, 4)?;
            key(app, " ")?;
            check(
                ui.get_confirmation() == 1 && ui.get_name_focus_target() == 5,
                "Keyboard Delete did not focus the safe confirmation action",
            )?;
            // Keep the real panel open across the independent 15-second capture.
        }
        5 => {
            settled(app)?;
            check(
                ui.get_confirmation() == 1 && ui.get_name_focus_target() == 5,
                "Confirmation did not retain its safe action focus",
            )?;
            key(app, Key::Escape)?;
            check(
                ui.get_confirmation() == 0 && !ui.get_busy() && ui.get_name_focus_target() == 4,
                "Escape did not cancel confirmation and restore Delete focus safely",
            )?;
            check(
                state.playlists.borrow().len() == 1,
                "Cancelled confirmation deleted the playlist",
            )?;
            focus(app, 3)?;
            key(app, " ")?;
            type_text(app, AWAY)?;
            key(app, Key::Return)?;
            check(
                ui.get_busy(),
                "Navigation case did not start an actual rename write",
            )?;
            // Route changes may retire focus, but must not cancel accepted SQL.
            app.invoke_navigate(3);
            app.invoke_focus_search();
            check(
                app.get_search_active(),
                "Could not focus search after leaving the editor",
            )?;
        }
        6 => {
            settled(app)?;
            check(
                app.get_page() == 3 && app.get_search_active(),
                "Late rename completion stole focus or navigated back",
            )?;
            app.invoke_open_local_tab(0);
        }
        7 => {
            settled(app)?;
            check(
                selected(app, state)
                    .is_some_and(|(id, name)| Some(id) == progress.playlist.get() && name == AWAY),
                "Accepted rename was lost after navigation",
            )?;
            open_editor(app)?;
            focus(app, 1)?;
            replace_text(app, "")?;
            key(app, Key::Return)?;
            check(
                !ui.get_busy()
                    && ui.get_name_draft().is_empty()
                    && !ui.get_name_status().is_empty(),
                "Empty name validation submitted a write",
            )?;
            replace_text(app, &"x".repeat(1025))?;
            key(app, Key::Return)?;
            check(
                !ui.get_busy()
                    && ui.get_name_draft().len() == 1025
                    && !ui.get_name_status().is_empty()
                    && ui.get_name_focus_target() == 1,
                "Too-long name validation lost draft/focus or admitted a write",
            )?;
            key(app, Key::Escape)?;
            check(
                ui.get_name_draft().len() == 1025,
                "Escape destructively cleared a create draft",
            )?;
            replace_text(app, DRAFT)?;
        }
        8 => {
            settled(app)?;
            check(
                state.playlists.borrow().len() == 1,
                "Invalid names changed the stored collection count",
            )?;
            focus(app, 2)?;
            key(app, " ")?;
            check(
                ui.get_busy(),
                "Visible Create action was not keyboard activated",
            )?;
        }
        9 => {
            settled(app)?;
            check(
                state.playlists.borrow().len() == 2
                    && selected(app, state).is_some_and(|(id, name)| {
                        Some(id) != progress.playlist.get() && name == DRAFT
                    }),
                "Visible Create did not commit a second selected playlist",
            )?;
            check(
                ui.get_name_draft().is_empty() && ui.get_name_focus_target() == 1,
                "Visible Create did not restore a ready editor",
            )?;
            check(
                selection_display_matches(app),
                "Second Create dropdown retained a severed old selection binding",
            )?;
            reject_selection(app)?;
        }
        10 => {
            settled(app)?;
            // A coalesced current-value change must not reverse the rejected
            // current-index rollback on the next event-loop turn.
            check(
                selected(app, state)
                    .is_some_and(|(id, name)| Some(id) != progress.playlist.get() && name == DRAFT)
                    && selection_display_matches(app),
                "Rejected selection changed after the next event-loop turn",
            )?;
            key(
                app,
                if ui.get_selected() > 0 {
                    Key::UpArrow
                } else {
                    Key::DownArrow
                },
            )?;
            check(
                ui.get_busy(),
                "Production keyboard selection did not admit its real page read",
            )?;
        }
        11 => {
            settled(app)?;
            check(
                selected(app, state)
                    .is_some_and(|(id, name)| Some(id) == progress.playlist.get() && name == AWAY)
                    && selection_display_matches(app),
                "Production selection acknowledgment did not select and display the first renamed playlist",
            )?;
            app.invoke_navigate(0);
            check(
                app.get_page() == 0 && app.get_home_active() && state.model.row_count() == 0,
                "Empty local collections did not return to local Home",
            )?;
            check(
                state.playlists.borrow().len() == 2
                    && state.player.snapshot().file_loads == 0
                    && !app.get_account_connected()
                    && state.thumbnails.borrow().statistics().remote_started == 0,
                "Offline keyboard flow admitted unrelated media/account/network work",
            )?;
        }
        12 => {
            settled(app)?;
            app.invoke_open_local_tab(0);
        }
        13..=18 => {
            settled(app)?;
            let id = progress
                .playlist
                .get()
                .ok_or("Missing original playlist identity")?;
            check(
                app.get_page() == 1
                    && ui.get_tab() == 0
                    && selected(app, state)
                        .is_some_and(|(selected, name)| selected == id && name == AWAY),
                "Fixture saves must remain on the original Library route",
            )?;
            let first = (index - 13) * SAVE_BATCH;
            // Bounded synthetic input to the real worker; do not steal its
            // response receiver from the ordinary application adapter. The
            // subsequent FIFO read verifies every committed item and order.
            for item in first..(first + SAVE_BATCH).min(SAVES) {
                check(
                    state.library.submit(crate::library::Request::Save(
                        crate::library::VideoSave {
                            serial: (1_u64 << 60) + item as u64,
                            destination: crate::library::SaveDestination::Existing(id),
                            video: fixture_video(item),
                        },
                    )),
                    "The bounded diagnostic save batch was not admitted",
                )?;
            }
            if index == 18 {
                // Same-tab navigation intentionally no-ops. Reopen through a
                // non-Home route so no Home read or thumbnail request is added.
                app.invoke_navigate(3);
                app.invoke_open_local_tab(0);
                check(
                    ui.get_busy(),
                    "The committed fixture page read was not admitted",
                )?;
            }
        }
        19 => {
            settled(app)?;
            check(
                saved_page(state, 0, 100) && ui.get_next() && !ui.get_previous(),
                "The worker did not publish the exact first 100 saved videos",
            )?;
            open_editor(app)?;
            focus(app, 1)?;
            replace_text(app, PAGE_DRAFT)?;
            focus(app, 3)?;
            key(app, " ")?;
            type_text(app, "TEST FIXTURE rename abandoned by video Next")?;
            check(
                ui.get_renaming()
                    && ui.get_name_draft() == "TEST FIXTURE rename abandoned by video Next",
                "The pagination case did not start an actual keyboard rename draft",
            )?;
            ui.invoke_page(true);
            check(
                ui.get_busy() && !ui.get_renaming() && ui.get_name_draft() == PAGE_DRAFT,
                "Accepted video Next did not immediately cancel Rename and restore the Create draft",
            )?;
        }
        20 => {
            settled(app)?;
            check(
                saved_page(state, 100, SAVES)
                    && ui.get_previous()
                    && !ui.get_next()
                    && !ui.get_renaming()
                    && ui.get_name_draft() == PAGE_DRAFT
                    && selected(app, state).is_some_and(|(id, name)| {
                        Some(id) == progress.playlist.get() && name == AWAY
                    }),
                "Video Next changed the playlist name/identity or published the wrong final page",
            )?;
            ui.invoke_page(false);
            check(ui.get_busy(), "Video Previous did not admit its real read")?;
        }
        21 => {
            settled(app)?;
            check(
                saved_page(state, 0, 100)
                    && !ui.get_previous()
                    && ui.get_next()
                    && !ui.get_renaming()
                    && ui.get_name_draft() == PAGE_DRAFT
                    && selection_display_matches(app)
                    && selected(app, state).is_some_and(|(id, name)| {
                        Some(id) == progress.playlist.get() && name == AWAY
                    }),
                "Video Previous did not restore the original page and unchanged name context",
            )?;
            check(
                state.playlists.borrow().len() == 2
                    && state.player.snapshot().file_loads == 0
                    && !app.get_account_connected()
                    && state.thumbnails.borrow().statistics().remote_started == 0,
                "Offline pagination admitted unrelated media/account/network work",
            )?;
        }
        _ => return Err("Unexpected keyboard diagnostic stage"),
    }
    Ok(())
}
impl Smoke {
    pub fn start(app: &App, state: &Rc<UiState>) -> Self {
        app.set_diagnostic_fixture_label("TEST FIXTURE — offline playlist keyboard check".into());
        let progress = Rc::new(Progress::default());
        let weak = Rc::downgrade(&progress);
        app.on_search(move |_| {
            if let Some(progress) = weak.upgrade() {
                progress.fail("Offline playlist keys unexpectedly submitted search");
            }
        });
        let weak = Rc::downgrade(&progress);
        app.on_select_video(move |_| {
            if let Some(progress) = weak.upgrade() {
                progress.fail("Offline playlist keys unexpectedly selected media");
            }
        });
        let timers=STAGES.into_iter().enumerate().map(|(index,seconds)| {
            let app=app.as_weak(); let state=Rc::downgrade(state); let progress=progress.clone();
            let timer=Timer::default();
            timer.start(TimerMode::SingleShot,Duration::from_secs(seconds),move || {
                if progress.failure.borrow().is_some() { return; }
                let (Some(app),Some(state))=(app.upgrade(),state.upgrade()) else { progress.fail("Keyboard diagnostic window ended early"); return; };
                if progress.completed.get()!=index { progress.fail("Keyboard diagnostic skipped a checkpoint"); return; }
                if let Err(error)=stage(index,&app,&state,&progress) { progress.fail(error); eprintln!("library keyboard failed: stage={} reason={error}",index+1); return; }
                if progress.failure.borrow().is_some() { return; }
                progress.completed.set(index+1);
                eprintln!("library keyboard stage={} injected_slint_keys=true os_keyboard_delivery=false",index+1);
            });
            timer
        }).collect();
        Self {
            progress,
            _timers: timers,
        }
    }
    pub fn finish(self) -> Result<(), &'static str> {
        self.progress.finish()?;
        eprintln!(
            "library keyboard complete: stages={} saved_fixture_videos=101 injected_slint_keys=true remote_thumbnail_starts=0 media_loads=0 account_connected=false",
            STAGES.len()
        );
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn incomplete_or_failed_stages_never_report_success() {
        let progress = Progress::default();
        for count in 0..STAGES.len() {
            progress.completed.set(count);
            assert!(progress.finish().is_err());
        }
        progress.completed.set(STAGES.len());
        assert!(progress.finish().is_ok());
        progress.fail("first failure");
        progress.fail("later failure");
        assert_eq!(progress.finish(), Err("first failure"));
        assert!(STAGES.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(STAGES.last().unwrap() < &68);
        const {
            assert!(SAVE_BATCH <= 20);
            assert!(SAVES.div_ceil(SAVE_BATCH) == 6);
        }
    }
}
