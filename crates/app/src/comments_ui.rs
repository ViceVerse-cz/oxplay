// SPDX-License-Identifier: GPL-3.0-or-later
//! Bounded guest comments with one deferred initial request and explicit paging.
use crate::{App, CommentRow, CommentsUi, UiState, catalog::Request, display_format};
use oxplay_core::{ProviderError, VideoDetails, VideoId};
use oxplay_youtube::comments::{CommentCursor, CommentPage};
use slint::ComponentHandle;
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};
#[derive(Default)]
pub struct State {
    pending: Cell<Option<u64>>,
    video: RefCell<Option<VideoId>>,
    rows: Rc<slint::VecModel<CommentRow>>,
    cursor: RefCell<Option<CommentCursor>>,
    next: RefCell<Option<CommentCursor>>,
    previous: RefCell<Vec<Option<CommentCursor>>>,
    auto_load: slint::Timer,
    auto_generation: Cell<Option<u64>>,
    auto_requested: Cell<bool>,
    reveal_page: Cell<bool>,
    owner: RefCell<Weak<UiState>>,
    // Created in `bind`, where the application handle exists.
    avatars: RefCell<Option<crate::comment_avatars::Avatars>>,
    /// Portrait URLs of the rows currently published, in row order.
    avatar_urls: RefCell<Vec<Option<String>>>,
}
impl State {
    /// Retire portraits for the page being replaced or removed. Late results are
    /// dropped by generation, so a stale page can never decorate a new one.
    fn retire_avatars(&self) {
        self.avatar_urls.borrow_mut().clear();
        if let Some(avatars) = self.avatars.borrow().as_ref() {
            avatars.cancel();
        }
    }
    fn supersede(&self, generation: u64) -> bool {
        if self
            .auto_generation
            .get()
            .is_some_and(|expected| expected != generation)
        {
            self.auto_load.stop();
            self.auto_generation.set(None);
        }
        if self
            .pending
            .get()
            .is_some_and(|pending| pending != generation)
        {
            self.pending.set(None);
            true
        } else {
            false
        }
    }
}
pub fn details(app: &App, state: &UiState, video: &VideoId, details: &VideoDetails) {
    set_details(app, state, video, details, true);
}
/// Already-resolved account metadata never enables an anonymous comment request.
pub fn account_details(app: &App, state: &UiState, video: &VideoId, details: &VideoDetails) {
    set_details(app, state, video, details, false);
}
fn set_details(
    app: &App,
    state: &UiState,
    video: &VideoId,
    details: &VideoDetails,
    guest_comments: bool,
) {
    let s = &state.comments_ui;
    s.auto_load.stop();
    s.auto_generation.set(None);
    s.auto_requested.set(false);
    s.reveal_page.set(false);
    *s.video.borrow_mut() = guest_comments.then(|| video.clone());
    s.pending.set(None);
    s.retire_avatars();
    s.rows.set_vec(Vec::new());
    s.cursor.borrow_mut().take();
    s.next.borrow_mut().take();
    s.previous.borrow_mut().clear();
    let ui = app.global::<CommentsUi>();
    ui.set_enabled(crate::library_ui::desired_preferences(state).comments_enabled);
    ui.set_request_active(false);
    ui.set_next(false);
    ui.set_previous(false);
    ui.set_description_expanded(false);
    ui.set_available(guest_comments);
    ui.set_has_loaded(false);
    ui.set_status(
        if guest_comments {
            ""
        } else {
            "Comments are unavailable for account playback."
        }
        .into(),
    );
    present(app, details);
    if guest_comments {
        schedule_auto(app, state);
    }
}
/// Late native watch-page details for the same accepted guest video: update the
/// description/metadata presentation only. Comment rows, cursors, pending work
/// and the reader's expanded/collapsed choice are left untouched.
pub fn refresh_details(app: &App, state: &UiState, video: &VideoId, details: &VideoDetails) {
    if state.comments_ui.video.borrow().as_ref() != Some(video) {
        return;
    }
    present(app, details);
}
fn present(app: &App, details: &VideoDetails) {
    let ui = app.global::<CommentsUi>();
    let description = details
        .description
        .as_deref()
        .unwrap_or("No description was provided.");
    let preview = description.lines().take(3).collect::<Vec<_>>().join("\n");
    let preview = preview.chars().take(300).collect::<String>();
    ui.set_description_expandable(preview != description);
    ui.set_description_preview(preview.into());
    ui.set_description(description.into());
    ui.set_heading(
        details
            .comment_count
            .map(|count| format!("{} comments", display_format::grouped_count(count)))
            .unwrap_or_else(|| "Comments".into())
            .into(),
    );
    let mut metadata = Vec::new();
    if let Some(count) = details.view_count {
        metadata.push(format!("{} views", display_format::grouped_count(count)));
    }
    if let Some(date) = &details.upload_date {
        metadata.push(display_format::date(date));
    }
    if let Some(count) = details.like_count {
        metadata.push(format!("{} likes", display_format::compact_count(count)));
    }
    ui.set_metadata(metadata.join(" · ").into());
    app.set_watch_channel_subscribers(
        details
            .channel_subscriber_count
            .map(|count| format!("{} subscribers", display_format::compact_count(count)))
            .unwrap_or_default()
            .into(),
    );
}
/// Settings are admitted/persisted through the existing library worker.
pub fn sync_preferences(app: &App, state: &UiState) {
    let enabled = crate::library_ui::desired_preferences(state).comments_enabled;
    let ui = app.global::<CommentsUi>();
    ui.set_enabled(enabled);
    if enabled {
        schedule_auto(app, state);
    } else {
        state.comments_ui.auto_load.stop();
        state.comments_ui.auto_generation.set(None);
        cancel_owned(app, state);
        state.comments_ui.retire_avatars();
        state.comments_ui.rows.set_vec(Vec::new());
        state.comments_ui.auto_requested.set(false);
        ui.set_has_loaded(false);
        ui.set_next(false);
        ui.set_previous(false);
    }
}
fn schedule_auto(app: &App, state: &UiState) {
    let s = &state.comments_ui;
    if !app.global::<CommentsUi>().get_enabled()
        || s.auto_requested.get()
        || s.pending.get().is_some()
    {
        return;
    }
    let Some(video) = s.video.borrow().clone() else {
        return;
    };
    let generation = state.worker.borrow().generation();
    let weak = app.as_weak();
    let owner = s.owner.borrow().clone();
    s.auto_generation.set(Some(generation));
    // Resolve publication first commits current_video, playback, focus and busy.
    // A finite next-event handoff avoids re-entering the resolving worker borrow
    // or superseding a new search/caption/video request made in the meantime.
    s.auto_load.start(
        slint::TimerMode::SingleShot,
        std::time::Duration::ZERO,
        move || {
            let (Some(app), Some(state)) = (weak.upgrade(), owner.upgrade()) else {
                return;
            };
            let s = &state.comments_ui;
            s.auto_generation.set(None);
            if !automatic_admitted(&app, &state, &video, generation) {
                return;
            }
            s.auto_requested.set(true);
            s.previous.borrow_mut().clear();
            s.reveal_page.set(false);
            submit(&app, &state, None);
        },
    );
}
fn automatic_admitted(app: &App, state: &UiState, video: &VideoId, generation: u64) -> bool {
    app.global::<CommentsUi>().get_enabled()
        && !app.get_busy()
        && app.get_loaded()
        && app.get_remote_video()
        && !app.get_account_playback_active()
        && state.worker.borrow().generation() == generation
        && state.current_video.borrow().as_ref().map(|v| &v.id) == Some(video)
        && state.comments_ui.video.borrow().as_ref() == Some(video)
}
fn cancel_owned(app: &App, state: &UiState) {
    if let Some(generation) = state.comments_ui.pending.take() {
        state.worker.borrow_mut().cancel_generation(generation);
    }
    // Comments never own application-wide busy state. A concurrently
    // admitted UI operation must retain its own admission indicator.
    app.global::<CommentsUi>().set_request_active(false);
}
/// Clear descriptions and retire only this feature's work. Never stop a newer
/// resolver request or reveal authenticated metadata to the guest reader.
pub fn clear_local(app: &App, state: &UiState) {
    let s = &state.comments_ui;
    s.auto_load.stop();
    s.auto_generation.set(None);
    s.auto_requested.set(false);
    cancel_owned(app, state);
    s.video.borrow_mut().take();
    s.retire_avatars();
    s.rows.set_vec(Vec::new());
    s.cursor.borrow_mut().take();
    s.next.borrow_mut().take();
    s.previous.borrow_mut().clear();
    let ui = app.global::<CommentsUi>();
    ui.set_next(false);
    ui.set_previous(false);
    ui.set_description_expanded(false);
    ui.set_available(false);
    ui.set_has_loaded(false);
    ui.set_heading("Comments".into());
    ui.set_description_expandable(false);
    ui.set_description_preview("Description unavailable for this playback mode.".into());
    ui.set_description("Description unavailable for this playback mode.".into());
    ui.set_metadata("".into());
    ui.set_status("".into());
    app.set_watch_channel_subscribers("".into());
}
fn submit(app: &App, state: &UiState, cursor: Option<CommentCursor>) {
    if app.get_busy()
        || state.comments_ui.pending.get().is_some()
        || !app.global::<CommentsUi>().get_enabled()
        || app.get_account_playback_active()
    {
        return;
    }
    let Some(video) = state.current_video.borrow().as_ref().map(|v| v.id.clone()) else {
        return;
    };
    if state.comments_ui.video.borrow().as_ref() != Some(&video) {
        return;
    }
    // The first page reuses the native watch page's comments continuation when
    // it already arrived, saving one `next` request. It is the page-one cursor
    // that Previous returns to.
    let cursor = cursor.or_else(|| crate::watch_meta::comment_start(state, &video));
    *state.comments_ui.cursor.borrow_mut() = cursor.clone();
    state
        .worker
        .borrow_mut()
        .submit(Request::Comments(video, cursor));
    state
        .comments_ui
        .pending
        .set(Some(state.worker.borrow().generation()));
    state.comments_ui.retire_avatars();
    state.comments_ui.rows.set_vec(Vec::new());
    state.comments_ui.next.borrow_mut().take();
    let ui = app.global::<CommentsUi>();
    ui.set_next(false);
    ui.set_previous(!state.comments_ui.previous.borrow().is_empty());
    // Loading comments is local to this section. Playback/navigation stay
    // interactive and may supersede this lower-priority shared-worker job.
    ui.set_request_active(true);
    ui.set_status("".into());
}
pub fn publish(
    app: &App,
    state: &UiState,
    video: VideoId,
    result: Result<CommentPage, ProviderError>,
) {
    if !app.global::<CommentsUi>().get_enabled()
        || app.get_account_playback_active()
        || state.current_video.borrow().as_ref().map(|v| &v.id) != Some(&video)
        || state.comments_ui.video.borrow().as_ref() != Some(&video)
    {
        return;
    }
    state.comments_ui.pending.set(None);
    let ui = app.global::<CommentsUi>();
    ui.set_request_active(false);
    match result {
        Ok(page) if page.video == video => {
            ui.set_has_loaded(true);
            let comments: Vec<_> = page.comments.into_iter().take(20).collect();
            let urls: Vec<Option<String>> = comments
                .iter()
                .map(|comment| comment.author_thumbnail_url.clone())
                .collect();
            let rows = comments
                .into_iter()
                .map(|comment| CommentRow {
                    author: comment
                        .author
                        .unwrap_or_else(|| "Author unavailable".into())
                        .into(),
                    metadata: comment.published_text.unwrap_or_default().into(),
                    likes: comment
                        .like_count
                        .filter(|count| *count > 0)
                        .map(display_format::compact_count)
                        .unwrap_or_default()
                        .into(),
                    creator: comment.author_is_uploader,
                    content: comment.text.into(),
                    avatar: slint::Image::default(),
                    avatar_ready: false,
                })
                .collect::<Vec<_>>();
            let empty = rows.is_empty();
            state.comments_ui.retire_avatars();
            state.comments_ui.rows.set_vec(rows);
            if let Some(avatars) = state.comments_ui.avatars.borrow().as_ref() {
                avatars.request(
                    urls.iter()
                        .enumerate()
                        .filter_map(|(row, url)| Some((row, url.clone()?)))
                        .collect(),
                );
            }
            *state.comments_ui.avatar_urls.borrow_mut() = urls;
            *state.comments_ui.next.borrow_mut() = page.next;
            ui.set_next(state.comments_ui.next.borrow().is_some());
            ui.set_previous(!state.comments_ui.previous.borrow().is_empty());
            ui.set_status(
                if page.limit_reached {
                    "200-comment browsing limit reached."
                } else if empty {
                    "No public comments to show."
                } else {
                    ""
                }
                .into(),
            );
            // Automatic publication never scrolls away from the playing video.
            // Only deliberate refresh/pagination may reveal its resulting page.
            if state.comments_ui.reveal_page.replace(false) {
                ui.set_page_epoch(ui.get_page_epoch().wrapping_add(1));
            }
        }
        Ok(_) => ui.set_status("The provider returned comments for a different video.".into()),
        Err(error) => ui.set_status(if error == ProviderError::Unavailable {
            "Comments are unavailable. Try again later.".into()
        } else {
            error.to_string().into()
        }),
    }
}
pub fn bind(app: &App, state: &Rc<UiState>) {
    *state.comments_ui.owner.borrow_mut() = Rc::downgrade(state);
    let ui = app.global::<CommentsUi>();
    ui.set_rows(state.comments_ui.rows.clone().into());
    ui.set_enabled(true);
    let weak = app.as_weak();
    *state.comments_ui.avatars.borrow_mut() =
        Some(crate::comment_avatars::Avatars::new(move || {
            let _ =
                weak.upgrade_in_event_loop(|app| app.global::<CommentsUi>().invoke_avatar_wake());
        }));
    let owner = Rc::downgrade(state);
    ui.on_avatar_wake(move || {
        let Some(state) = owner.upgrade() else { return };
        let ready = state
            .comments_ui
            .avatars
            .borrow()
            .as_ref()
            .map(|avatars| avatars.take())
            .unwrap_or_default();
        for ready in ready {
            // The row must still be the one whose portrait URL was requested.
            let Some(mut row) = slint::Model::row_data(&*state.comments_ui.rows, ready.row) else {
                continue;
            };
            if state
                .comments_ui
                .avatar_urls
                .borrow()
                .get(ready.row)
                .is_none_or(|url| url.is_none())
            {
                continue;
            }
            let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                ready.pixels.as_raw(),
                ready.pixels.width(),
                ready.pixels.height(),
            );
            row.avatar = slint::Image::from_rgba8(buffer);
            row.avatar_ready = true;
            slint::Model::set_row_data(&*state.comments_ui.rows, ready.row, row);
        }
    });
    let weak = app.as_weak();
    let owner = Rc::downgrade(state);
    state
        .worker
        .borrow_mut()
        .on_generation_changed(move |generation| {
            let (Some(app), Some(state)) = (weak.upgrade(), owner.upgrade()) else {
                return;
            };
            if state.comments_ui.supersede(generation) {
                let ui = app.global::<CommentsUi>();
                ui.set_request_active(false);
                ui.set_status("Comment loading was interrupted. Reload to try again.".into());
            }
        });
    let weak = app.as_weak();
    let owner = Rc::downgrade(state);
    ui.on_set_enabled(move |enabled| {
        let (Some(app), Some(state)) = (weak.upgrade(), owner.upgrade()) else {
            return;
        };
        if crate::library_ui::set_comments_enabled(&app, &state, enabled) {
            sync_preferences(&app, &state);
        }
    });
    let weak = app.as_weak();
    let owner = Rc::downgrade(state);
    ui.on_load(move || {
        let (Some(app), Some(state)) = (weak.upgrade(), owner.upgrade()) else {
            return;
        };
        if app.get_busy() || state.comments_ui.pending.get().is_some() {
            return;
        }
        state.comments_ui.auto_load.stop();
        state.comments_ui.auto_generation.set(None);
        state.comments_ui.previous.borrow_mut().clear();
        state.comments_ui.reveal_page.set(true);
        submit(&app, &state, None);
    });
    let weak = app.as_weak();
    let owner = Rc::downgrade(state);
    ui.on_page(move |forward| {
        let (Some(app), Some(state)) = (weak.upgrade(), owner.upgrade()) else {
            return;
        };
        if app.get_busy() || state.comments_ui.pending.get().is_some() {
            return;
        }
        let cursor = if forward {
            let Some(next) = state.comments_ui.next.borrow().clone() else {
                return;
            };
            let mut previous = state.comments_ui.previous.borrow_mut();
            if previous.len() >= 10 {
                return;
            }
            previous.push(state.comments_ui.cursor.borrow().clone());
            Some(next)
        } else {
            let Some(previous) = state.comments_ui.previous.borrow_mut().pop() else {
                return;
            };
            previous
        };
        state.comments_ui.reveal_page.set(true);
        submit(&app, &state, cursor);
    });
    let weak = app.as_weak();
    let owner = Rc::downgrade(state);
    ui.on_cancel(move || {
        let (Some(app), Some(state)) = (weak.upgrade(), owner.upgrade()) else {
            return;
        };
        cancel_owned(&app, &state);
        app.global::<CommentsUi>()
            .set_status("Comment loading cancelled.".into());
    });
}
/// Explicit finite native diagnostic; separate from the one-shot product fetch.
pub struct Smoke {
    verified: Rc<Cell<bool>>,
    _timers: Vec<slint::Timer>,
    captures: Captures,
}
impl Smoke {
    pub fn start(app: &App, state: &Rc<UiState>, directory: Option<std::path::PathBuf>) -> Self {
        let verified = Rc::new(Cell::new(false));
        let captures = Rc::new(RefCell::new(Vec::new()));
        let mut timers = Vec::new();
        for stage in [15, 17, 20, 40, 60, 62, 65] {
            let weak = app.as_weak();
            let state = state.clone();
            let verified = verified.clone();
            let captures = captures.clone();
            let directory = directory.clone();
            let timer = slint::Timer::default();
            timer.start(
                slint::TimerMode::SingleShot,
                std::time::Duration::from_secs(stage),
                move || {
                    let Some(app) = weak.upgrade() else { return };
                    let ui = app.global::<CommentsUi>();
                    match stage {
                        15 => {
                            assert!(
                                app.get_remote_video() && app.get_loaded(),
                                "comments smoke needs resolved guest playback"
                            );
                            assert!(
                                !ui.get_description().is_empty(),
                                "description was not populated"
                            );
                            ui.set_description_expanded(true);
                            app.invoke_show_video_details();
                        }
                        17 => capture(&app, directory.as_deref(), "description.png", &captures),
                        20 => {
                            if !ui.get_has_loaded() && !ui.get_request_active() {
                                ui.invoke_load();
                                assert!(ui.get_request_active(), "comment request did not start");
                            }
                        }
                        40 => {
                            assert!(
                                !app.get_busy() && !ui.get_request_active(),
                                "first comment request did not finish"
                            );
                            assert_eq!(
                                slint::Model::row_count(&*state.comments_ui.rows),
                                20,
                                "expected 20 real comments"
                            );
                            assert!(ui.get_next(), "public fixture has no second comment page");
                            ui.invoke_page(true);
                        }
                        60 => {
                            assert!(
                                !app.get_busy() && !ui.get_request_active(),
                                "second comment request did not finish"
                            );
                            let count = slint::Model::row_count(&*state.comments_ui.rows);
                            assert!(
                                count > 0 && count <= 20,
                                "second page must contain bounded real comments"
                            );
                            assert!(ui.get_previous(), "previous-page cursor was lost");
                            capture(&app, directory.as_deref(), "comments.png", &captures);
                            eprintln!("comments smoke second page rows={count}");
                        }
                        62 => {
                            ui.invoke_load();
                            assert!(ui.get_request_active());
                            ui.invoke_cancel();
                            assert!(!app.get_busy() && !ui.get_request_active());
                        }
                        _ => {
                            assert!(!ui.get_request_active());
                            assert!(state.comments_ui.pending.get().is_none());
                            verified.set(true);
                        }
                    }
                    eprintln!("comments smoke stage={stage} completed");
                },
            );
            timers.push(timer);
        }
        Self {
            verified,
            _timers: timers,
            captures,
        }
    }
    pub fn finish(self) -> Result<(), String> {
        for worker in self.captures.borrow_mut().drain(..) {
            worker
                .join()
                .map_err(|_| "comments snapshot worker failed")??;
        }
        if !self.verified.get() {
            return Err("comments smoke ended before its assertions completed".into());
        }
        Ok(())
    }
}
type Captures = Rc<RefCell<Vec<std::thread::JoinHandle<Result<(), String>>>>>;
fn capture(app: &App, directory: Option<&std::path::Path>, name: &str, workers: &Captures) {
    let Some(directory) = directory else { return };
    // Explicit diagnostic readback only, never part of playback/performance tests.
    let pixels = app
        .window()
        .take_snapshot()
        .expect("comments snapshot unavailable");
    let width = pixels.width();
    let height = pixels.height();
    let bytes = pixels.as_bytes().to_vec();
    let path = directory.join(name);
    workers.borrow_mut().push(std::thread::spawn(move || {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .map_err(|_| "snapshot destination unavailable")?;
        let image = image::RgbaImage::from_raw(width, height, bytes).ok_or("invalid snapshot")?;
        image
            .write_to(&mut file, image::ImageFormat::Png)
            .map_err(|_| "snapshot encoding failed".into())
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deferred_initial_fetch_retires_when_a_new_worker_generation_wins() {
        i_slint_backend_testing::init_no_event_loop();
        let state = State::default();
        state.auto_generation.set(Some(7));
        state.auto_load.start(
            slint::TimerMode::SingleShot,
            std::time::Duration::ZERO,
            || panic!("retired auto fetch must not execute"),
        );
        assert!(state.auto_load.running());
        assert!(!state.supersede(7));
        assert!(state.auto_load.running());
        assert!(!state.supersede(8));
        assert!(!state.auto_load.running());
        assert_eq!(state.auto_generation.get(), None);
    }
    #[test]
    fn loading_ownership_clears_on_supersession_without_a_busy_transition() {
        let state = State::default();
        state.pending.set(Some(7));
        assert!(!state.supersede(7));
        assert_eq!(state.pending.get(), Some(7));
        assert!(state.supersede(8));
        assert_eq!(state.pending.get(), None);
        assert!(!state.supersede(9));
    }
}
