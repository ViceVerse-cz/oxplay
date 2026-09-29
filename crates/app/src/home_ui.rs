// SPDX-License-Identifier: GPL-3.0-or-later
//! A bounded local-only Home page. No network, account authority or SQL here.
use crate::{App, UiState, library};
use serein_core::VideoSummary;
use serein_storage::{Page, PageCursor};
use std::cell::{Cell, RefCell};

#[derive(Clone, Default)]
struct Pages {
    current: Option<PageCursor>,
    next: Option<PageCursor>,
    previous: Vec<Option<PageCursor>>,
}
#[derive(Default)]
struct Reads {
    serial: u64,
    pending: Option<(u64, Pages)>,
}
impl Reads {
    fn start(&mut self, pages: Pages) -> Option<u64> {
        self.serial = self.serial.checked_add(1)?;
        self.pending = Some((self.serial, pages));
        Some(self.serial)
    }
    fn finish(&mut self, ticket: u64) -> Option<Pages> {
        if self.pending.as_ref().is_some_and(|(id, _)| *id == ticket) {
            self.pending.take().map(|(_, pages)| pages)
        } else {
            None
        }
    }
}
#[derive(Default)]
pub struct State {
    reads: RefCell<Reads>,
    pages: RefCell<Pages>,
    accepted: Cell<Option<u64>>,
    dirty: Cell<bool>,
}

impl State {
    fn cancel_read(&self) {
        // An admitted refresh may have observed a committed mutation. If its
        // result is retired for playback, retry once when that operation ends.
        // A rejected/completed read has no pending ticket and stays quiescent.
        if self.reads.borrow_mut().pending.take().is_some() {
            self.dirty.set(true);
        }
    }
}
/// Retire only this local read; accepted writes on the shared worker still finish.
pub fn cancel(app: &App, state: &UiState) {
    state.home_ui.cancel_read();
    app.set_home_loading(false);
}
fn header(app: &App) {
    app.set_catalog_title("Home".into());
    app.set_catalog_subtitle("Recently saved · From your local playlists on this device".into());
    app.set_catalog_empty_title("Make yourself at home".into());
    app.set_catalog_empty_message("Save videos to a local playlist and they’ll appear here.\nSearch YouTube above when you’re ready to explore.".into());
    app.set_guest_scope(4);
    app.set_guest_can_back(false);
    app.set_guest_can_follow(false);
}
fn query(app: &App, state: &UiState, pages: Pages) {
    if state.caption_cache.active() || state.home_ui.reads.borrow().pending.is_some() {
        return;
    }
    state.home_ui.dirty.set(false);
    let after = pages.current;
    let Some(ticket) = state.home_ui.reads.borrow_mut().start(pages) else {
        app.set_status("Local Home navigation is unavailable. Restart the application.".into());
        return;
    };
    if state
        .library
        .submit(library::Request::Home { ticket, after })
    {
        app.set_home_loading(true);
        app.set_home_error(false);
    } else {
        state.home_ui.reads.borrow_mut().finish(ticket);
        app.set_home_error(true);
        app.set_status("The local library is busy. Use Refresh to try Home again.".into());
    }
}
pub fn open(app: &App, state: &UiState) {
    if state.caption_cache.active() {
        app.set_status("Wait for local-data clearing to finish.".into());
        return;
    }
    // Generation observers also retire pending authenticated extraction without
    // cancelling accepted account writes or reclassifying the active media.
    state.worker.borrow_mut().cancel();
    app.set_busy(false);
    crate::native_child::hide(state);
    if state.native_child.enabled {
        let _ = state.player.set_paused(true);
    }
    cancel(app, state);
    let same = app.get_home_active() && state.home_ui.pages.borrow().current.is_none();
    if !same {
        crate::guest_ui::begin_home(app, state);
    }
    app.set_home_active(true);
    app.set_search_kind(0);
    app.set_page(0);
    header(app);
    app.set_status(
        "Local Home · Saved videos only · Choose a video to connect for playback".into(),
    );
    query(app, state, Pages::default());
}
pub fn refresh(app: &App, state: &UiState) {
    if !app.get_home_active() || app.get_page() != 0 || app.get_busy() {
        return;
    }
    query(app, state, Pages::default());
}
pub fn page(app: &App, state: &UiState, forward: bool) {
    if !app.get_home_active() || app.get_page() != 0 || app.get_busy() {
        return;
    }
    let mut pages = state.home_ui.pages.borrow().clone();
    if forward {
        let Some(next) = pages.next else { return };
        if pages.previous.len() == 1024 {
            pages.previous.remove(0);
        }
        pages.previous.push(pages.current);
        pages.current = Some(next);
    } else {
        let Some(previous) = pages.previous.pop() else {
            return;
        };
        pages.current = previous;
    }
    query(app, state, pages);
}
pub fn receive(
    app: &App,
    state: &UiState,
    ticket: u64,
    result: Result<Page<VideoSummary>, String>,
) {
    let Some(mut pages) = state.home_ui.reads.borrow_mut().finish(ticket) else {
        return;
    };
    app.set_home_loading(false);
    if !app.get_home_active() || app.get_page() != 0 || state.caption_cache.active() {
        return;
    }
    let page = match result {
        Ok(page) => page,
        Err(error) => {
            app.set_home_error(true);
            app.set_status(error.into());
            return;
        }
    };
    let same = state.home_ui.pages.borrow().current == pages.current;
    if let Err(error) = crate::guest_ui::publish_home(app, state, page.items, same) {
        app.set_home_error(true);
        app.set_status(error.into());
        return;
    }
    pages.next = page.next;
    app.set_has_more(pages.next.is_some());
    app.set_has_previous(!pages.previous.is_empty());
    *state.home_ui.pages.borrow_mut() = pages;
    state.home_ui.accepted.set(Some(ticket));
    header(app);
    app.set_home_error(false);
}
/// A committed mutation must not navigate away from search, playback or account.
pub fn changed(app: &App, state: &UiState) {
    cancel(app, state);
    state.home_ui.dirty.set(true);
    {
        let mut pages = state.home_ui.pages.borrow_mut();
        pages.next = None;
        pages.previous.clear();
    }
    if app.get_home_active() {
        app.set_has_more(false);
        app.set_has_previous(false);
    }
    maybe_refresh(app, state);
}
/// Consume one deferred invalidation on a terminal operation/event, never poll.
pub fn maybe_refresh(app: &App, state: &UiState) {
    if app.get_home_active()
        && app.get_page() == 0
        && !app.get_busy()
        && !state.caption_cache.active()
        && state.home_ui.dirty.get()
        && state.home_ui.reads.borrow().pending.is_none()
    {
        query(app, state, Pages::default());
    }
}
pub fn accepted_read(state: &UiState) -> Option<u64> {
    state.home_ui.accepted.get()
}
pub fn acknowledged_videos(state: &UiState) -> Option<Vec<VideoSummary>> {
    if state.home_ui.reads.borrow().pending.is_some() {
        return None;
    }
    crate::guest_ui::local_videos(state)
}
/// Drop every Home-derived reference, including the shared related-video view.
pub fn cleared(app: &App, state: &UiState) {
    cancel(app, state);
    *state.home_ui.pages.borrow_mut() = Pages::default();
    state.home_ui.accepted.set(None);
    state.home_ui.dirty.set(false);
    if app.get_home_active() {
        crate::guest_ui::begin_home(app, state);
        header(app);
        app.set_home_error(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retiring_admitted_refresh_keeps_invalidation_but_rejected_read_does_not_retry() {
        let state = State::default();
        let ticket = state.reads.borrow_mut().start(Pages::default()).unwrap();
        state.cancel_read(); // a selection supersedes the admitted mutation refresh
        assert!(state.dirty.get());
        assert!(state.reads.borrow_mut().finish(ticket).is_none());
        state.dirty.set(false); // one deferred query attempt consumes admission
        let rejected = state.reads.borrow_mut().start(Pages::default()).unwrap();
        state.reads.borrow_mut().finish(rejected); // bounded queue rejected it
        state.cancel_read();
        assert!(
            !state.dirty.get(),
            "cancel must not rearm rejected/completed work"
        );
    }
    #[test]
    fn stale_local_read_cannot_finish_new_navigation_or_reappear_after_clear() {
        let mut reads = Reads::default();
        let first = reads.start(Pages::default()).unwrap();
        reads.pending = None; // search/navigation/clear barrier
        let second = reads.start(Pages::default()).unwrap();
        assert!(reads.finish(first).is_none());
        assert!(reads.pending.is_some());
        assert!(reads.finish(second).is_some());
        assert!(reads.finish(second).is_none());
        reads.serial = u64::MAX;
        assert!(reads.start(Pages::default()).is_none());
    }
}
