// SPDX-License-Identifier: GPL-3.0-or-later
//! Bounded resolved chapter navigation; no clock subscriptions or provider work.
use crate::{App, ChaptersUi, UiState, account_playback};
use serein_core::{MAX_VIDEO_CHAPTERS, ResolvedPlayback, VideoChapter, VideoId};
use slint::ComponentHandle;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

struct Selection {
    video: VideoId,
    load: u64,
    session: Option<u64>,
    chapters: Vec<VideoChapter>,
}
#[derive(Default)]
pub struct State {
    epoch: Cell<i32>,
    selection: RefCell<Option<Selection>>,
    rows: Rc<slint::VecModel<slint::language::StandardListViewItem>>,
}
impl State {
    pub fn invalidate(&self) {
        self.selection.borrow_mut().take();
        self.rows.set_vec(Vec::new());
        self.epoch.set(self.epoch.get().saturating_add(1));
    }
}
pub fn clear(app: &App, state: &UiState) {
    state.chapters_ui.invalidate();
    let ui = app.global::<ChaptersUi>();
    ui.set_epoch(state.chapters_ui.epoch.get());
    ui.set_available(false);
    ui.set_status("No chapter metadata is available for this video.".into());
}
fn time(seconds: u64) -> String {
    if seconds >= 3600 {
        format!(
            "{}:{:02}:{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    } else {
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
}
pub fn install(app: &App, state: &UiState, item: &ResolvedPlayback) {
    clear(app, state);
    let s = &state.chapters_ui;
    let ui = app.global::<ChaptersUi>();
    if item.details.chapters_unavailable || item.details.chapters.len() > MAX_VIDEO_CHAPTERS {
        ui.set_status("Chapter metadata was malformed or exceeded the supported limit.".into());
        return;
    }
    if s.epoch.get() == i32::MAX || item.details.chapters.is_empty() {
        return;
    }
    let session = if item.guest && item.session_generation == 0 {
        if account_playback::authorization(state).is_some() {
            return;
        }
        None
    } else if !item.guest
        && account_playback::authorization(state)
            .is_some_and(|lease| lease.is_valid() && lease.generation() == item.session_generation)
    {
        Some(item.session_generation)
    } else {
        return;
    };
    let load = state.player.snapshot().load_request_id;
    if load == 0 {
        return;
    }
    let rows: Vec<slint::language::StandardListViewItem> = item
        .details
        .chapters
        .iter()
        .enumerate()
        .map(|(index, chapter)| {
            let title = chapter
                .title
                .clone()
                .unwrap_or_else(|| format!("Chapter {}", index + 1));
            slint::language::StandardListViewItem::from(slint::SharedString::from(format!(
                "{} · {title}",
                time(chapter.start.as_secs())
            )))
        })
        .collect();
    *s.selection.borrow_mut() = Some(Selection {
        video: item.video.id.clone(),
        load,
        session,
        chapters: item.details.chapters.clone(),
    });
    s.rows.set_vec(rows);
    ui.set_available(true);
    ui.set_status("Select a chapter, then Jump. Playback stays paused if already paused.".into());
}
fn load_admitted(snapshot: &serein_media::Snapshot, load: u64) -> bool {
    load != 0
        && snapshot.load_request_id == load
        && snapshot.active_load_request_id == load
        && !snapshot.stop_pending
        && snapshot.failed_load_request_id != Some(load)
        && snapshot.playback_restarted
        && !matches!(
            snapshot.state,
            serein_media::PlaybackState::Idle | serein_media::PlaybackState::Failed
        )
}
pub fn bind(app: &App, state: &Rc<UiState>) {
    app.global::<ChaptersUi>()
        .set_rows(state.chapters_ui.rows.clone().into());
    let weak = app.as_weak();
    let state = Rc::downgrade(state);
    app.global::<ChaptersUi>().on_jump(move |index, epoch| {
        let (Some(app), Some(state)) = (weak.upgrade(), state.upgrade()) else {
            return;
        };
        if epoch != state.chapters_ui.epoch.get()
            || app.get_page() != 2
            || !app.get_loaded()
            || !app.get_remote_video()
            || app.get_busy()
            || state.caption_cache.active()
            || !state.presentation_ready.get()
        {
            return;
        }
        let selection = state.chapters_ui.selection.borrow();
        let Some(selection) = selection.as_ref() else {
            return;
        };
        if state.current_video.borrow().as_ref().map(|video| &video.id) != Some(&selection.video)
            || !load_admitted(&state.player.snapshot(), selection.load)
        {
            return;
        }
        let authority = account_playback::authorization(&state);
        let authorized = match selection.session {
            None => authority.is_none(),
            Some(session) => {
                state.account_ui.session_generation() == session
                    && authority
                        .is_some_and(|lease| lease.is_valid() && lease.generation() == session)
            }
        };
        if !authorized {
            return;
        }
        let Some(chapter) = usize::try_from(index)
            .ok()
            .and_then(|index| selection.chapters.get(index))
        else {
            return;
        };
        match state.player.seek(chapter.start.as_secs_f64()) {
            Ok(()) => {
                let _ = state.player.request_progress();
                app.global::<ChaptersUi>()
                    .set_status(format!("Seeking to {}…", time(chapter.start.as_secs())).into());
            }
            Err(error) => app
                .global::<ChaptersUi>()
                .set_status(error.to_string().into()),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chapters_cannot_seek_a_new_failed_stopped_or_not_started_native_load() {
        let snapshot = serein_media::Snapshot {
            load_request_id: 7,
            active_load_request_id: 7,
            playback_restarted: true,
            state: serein_media::PlaybackState::Paused,
            ..Default::default()
        };
        assert!(load_admitted(&snapshot, 7));
        assert!(!load_admitted(&snapshot, 6));
        for rejected in [
            serein_media::Snapshot {
                stop_pending: true,
                ..snapshot.clone()
            },
            serein_media::Snapshot {
                load_request_id: 8,
                ..snapshot.clone()
            },
            serein_media::Snapshot {
                active_load_request_id: 6,
                ..snapshot.clone()
            },
            serein_media::Snapshot {
                failed_load_request_id: Some(7),
                ..snapshot.clone()
            },
            serein_media::Snapshot {
                playback_restarted: false,
                ..snapshot.clone()
            },
        ] {
            assert!(!load_admitted(&rejected, 7));
        }
    }
}
