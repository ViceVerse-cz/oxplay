// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit guest-only comments; independent bounded model, no playback timer.
use crate::{App, CommentRow, CommentsUi, UiState, catalog::Request};
use serein_core::{ProviderError, VideoDetails, VideoId};
use serein_youtube::comments::{CommentCursor, CommentPage};
use slint::ComponentHandle;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
#[derive(Default)]
pub struct State {
    pending: Cell<Option<u64>>,
    video: RefCell<Option<VideoId>>,
    rows: Rc<slint::VecModel<CommentRow>>,
    cursor: RefCell<Option<CommentCursor>>,
    next: RefCell<Option<CommentCursor>>,
    previous: RefCell<Vec<Option<CommentCursor>>>,
}
impl State {
    fn supersede(&self, generation: u64) -> bool {
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
    *state.comments_ui.video.borrow_mut() = Some(video.clone());
    state.comments_ui.pending.set(None);
    state.comments_ui.rows.set_vec(Vec::new());
    state.comments_ui.cursor.borrow_mut().take();
    state.comments_ui.next.borrow_mut().take();
    state.comments_ui.previous.borrow_mut().clear();
    let ui = app.global::<CommentsUi>();
    ui.set_request_active(false);
    ui.set_next(false);
    ui.set_previous(false);
    ui.set_tab(0);
    ui.set_status("Comments load only when requested. Guest access; read-only.".into());
    ui.set_description(
        details
            .description
            .clone()
            .unwrap_or_else(|| "No description was provided.".into())
            .into(),
    );
    let mut metadata = Vec::new();
    if let Some(date) = &details.upload_date {
        metadata.push(format!("Uploaded {date}"));
    }
    if let Some(count) = details.view_count {
        metadata.push(format!("{count} views"));
    }
    if let Some(count) = details.like_count {
        metadata.push(format!("{count} likes"));
    }
    if let Some(count) = details.comment_count {
        metadata.push(format!("{count} comments reported by YouTube"));
    }
    ui.set_metadata(metadata.join(" · ").into());
}
/// Drop account-associated descriptions, identifiers, and comments without
/// issuing a guest request for a previously authenticated video.
pub fn clear_local(app: &App, state: &UiState) {
    let s = &state.comments_ui;
    s.pending.set(None);
    s.video.borrow_mut().take();
    s.rows.set_vec(Vec::new());
    s.cursor.borrow_mut().take();
    s.next.borrow_mut().take();
    s.previous.borrow_mut().clear();
    let ui = app.global::<CommentsUi>();
    ui.set_request_active(false);
    ui.set_next(false);
    ui.set_previous(false);
    ui.set_tab(0);
    ui.set_description("".into());
    ui.set_metadata("".into());
    ui.set_status("Account comments are not supported by the guest comments reader.".into());
}
fn submit(app: &App, state: &UiState, cursor: Option<CommentCursor>) {
    if app.get_busy() {
        return;
    }
    let Some(video) = state.current_video.borrow().as_ref().map(|v| v.id.clone()) else {
        return;
    };
    if state.comments_ui.video.borrow().as_ref() != Some(&video) {
        return;
    }
    *state.comments_ui.cursor.borrow_mut() = cursor.clone();
    state
        .worker
        .borrow_mut()
        .submit(Request::Comments(video, cursor));
    state
        .comments_ui
        .pending
        .set(Some(state.worker.borrow().generation()));
    state.comments_ui.rows.set_vec(Vec::new());
    state.comments_ui.next.borrow_mut().take();
    app.global::<CommentsUi>().set_next(false);
    app.global::<CommentsUi>()
        .set_previous(!state.comments_ui.previous.borrow().is_empty());
    app.set_busy(true);
    app.global::<CommentsUi>().set_request_active(true);
    app.global::<CommentsUi>()
        .set_status("Loading public comments…".into());
}
pub fn publish(
    app: &App,
    state: &UiState,
    video: VideoId,
    result: Result<CommentPage, ProviderError>,
) {
    if state.current_video.borrow().as_ref().map(|v| &v.id) != Some(&video)
        || state.comments_ui.video.borrow().as_ref() != Some(&video)
    {
        return;
    }
    state.comments_ui.pending.set(None);
    let ui = app.global::<CommentsUi>();
    ui.set_request_active(false);
    match result {
        Ok(page) if page.video == video => {
            let rows = page
                .comments
                .into_iter()
                .take(20)
                .map(|comment| {
                    let mut metadata = Vec::new();
                    if let Some(time) = comment.published_text {
                        metadata.push(time);
                    }
                    if let Some(likes) = comment.like_count {
                        metadata.push(format!("{likes} likes"));
                    }
                    if comment.author_is_uploader {
                        metadata.push("Creator".into());
                    }
                    CommentRow {
                        author: comment
                            .author
                            .unwrap_or_else(|| "Author unavailable".into())
                            .into(),
                        metadata: metadata.join(" · ").into(),
                        content: comment.text.into(),
                    }
                })
                .collect::<Vec<_>>();
            let empty = rows.is_empty();
            state.comments_ui.rows.set_vec(rows);
            *state.comments_ui.next.borrow_mut() = page.next;
            ui.set_next(state.comments_ui.next.borrow().is_some());
            ui.set_previous(!state.comments_ui.previous.borrow().is_empty());
            ui.set_status(
                if page.limit_reached {
                    "200-comment browsing limit reached."
                } else if empty {
                    "YouTube returned no public comments on this page."
                } else {
                    "Guest comments · Up to 20 per page · Replies are not loaded."
                }
                .into(),
            );
        }
        Ok(_) => ui.set_status("The provider returned comments for a different video.".into()),
        Err(error) => ui.set_status(if error == ProviderError::Unavailable {
            "Comments are unavailable or their order changed. Try Load / refresh.".into()
        } else {
            error.to_string().into()
        }),
    }
}
pub fn bind(app: &App, state: &Rc<UiState>) {
    app.global::<CommentsUi>()
        .set_rows(state.comments_ui.rows.clone().into());
    // This runs synchronously on worker supersession, even if busy remains true.
    // Weak ownership prevents Worker -> UiState -> Worker reference cycles.
    let weak = app.as_weak();
    let state_weak = Rc::downgrade(state);
    state
        .worker
        .borrow_mut()
        .on_generation_changed(move |generation| {
            let (Some(app), Some(state)) = (weak.upgrade(), state_weak.upgrade()) else {
                return;
            };
            if state.comments_ui.supersede(generation) {
                let ui = app.global::<CommentsUi>();
                ui.set_request_active(false);
                ui.set_status("Comment request cancelled by another action.".into());
            }
        });
    let weak = app.as_weak();
    let s = state.clone();
    app.global::<CommentsUi>().on_load(move || {
        let Some(app) = weak.upgrade() else { return };
        if app.get_busy() {
            return;
        }
        s.comments_ui.previous.borrow_mut().clear();
        submit(&app, &s, None);
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.global::<CommentsUi>().on_page(move |forward| {
        let Some(app) = weak.upgrade() else { return };
        if app.get_busy() {
            return;
        }
        let cursor = if forward {
            let Some(next) = s.comments_ui.next.borrow().clone() else {
                return;
            };
            let mut previous = s.comments_ui.previous.borrow_mut();
            if previous.len() >= 10 {
                return;
            }
            previous.push(s.comments_ui.cursor.borrow().clone());
            Some(next)
        } else {
            let Some(previous) = s.comments_ui.previous.borrow_mut().pop() else {
                return;
            };
            previous
        };
        submit(&app, &s, cursor);
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.global::<CommentsUi>().on_cancel(move || {
        let Some(app) = weak.upgrade() else { return };
        let Some(generation) = s.comments_ui.pending.take() else {
            return;
        };
        if !s.worker.borrow_mut().cancel_generation(generation) {
            return;
        }
        app.set_busy(false);
        app.global::<CommentsUi>().set_request_active(false);
        app.global::<CommentsUi>()
            .set_status("Comment request cancelled.".into());
    });
}
/// Explicit finite native diagnostic only; no timers or auto-fetch in normal use.
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
                                app.get_remote_video() && app.get_loaded() && !app.get_busy(),
                                "comments smoke needs resolved guest playback"
                            );
                            assert!(
                                !ui.get_description().is_empty(),
                                "description was not populated"
                            );
                            ui.set_tab(0);
                            app.invoke_show_video_details();
                        }
                        17 => capture(&app, directory.as_deref(), "description.png", &captures),
                        20 => {
                            ui.set_tab(1);
                            ui.invoke_load();
                            assert!(ui.get_request_active(), "comment request did not start");
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
