// SPDX-License-Identifier: GPL-3.0-or-later
//! Bounded Home page. Saved rows come from the local SQLite worker. When a
//! verified account is connected, an explicit Home visit/Refresh may also read
//! the account's YouTube recommendations through the account worker. No SQL,
//! network or credentials here; guests never trigger a remote Home request.
use crate::{App, UiState, account::WorkerError, library};
use serein_core::VideoSummary;
use serein_storage::{Page, PageCursor};
use serein_youtube::account::{AccountCursor, AccountPage};
use std::cell::{Cell, RefCell};

/// Rows per visible recommendation page, matching the other catalogs.
pub const RECOMMENDATION_PAGE: usize = 20;
/// Retained account rows for one Home visit; memory only, cleared on sign-out.
const MAX_RECOMMENDATIONS: usize = 200;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Source {
    #[default]
    Saved,
    Recommended,
}
impl Source {
    fn index(self) -> i32 {
        match self {
            Self::Saved => 0,
            Self::Recommended => 1,
        }
    }
    fn from_index(index: i32) -> Option<Self> {
        match index {
            0 => Some(Self::Saved),
            1 => Some(Self::Recommended),
            _ => None,
        }
    }
}
/// Only an explicit Home visit by a verified account reads recommendations.
/// Startup, guests and background refresh always show the local Saved feed.
fn initial_source(explicit: bool, connected: bool, preferred: Option<Source>) -> Source {
    if explicit && connected {
        preferred.unwrap_or(Source::Recommended)
    } else {
        Source::Saved
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fetch {
    First,
    Next,
}
enum Step<C> {
    Show,
    Fetch(C),
    Stay,
}
/// Account-derived Home rows. One pending account request at a time, bound to
/// its worker request ID and account session generation. The cursor type is
/// the provider's opaque session-scoped token (generic only for tests).
struct Recommendations<C = AccountCursor> {
    pending: Option<(u64, Fetch, u64)>,
    videos: Vec<VideoSummary>,
    next: Option<C>,
    page: usize,
    session: Option<u64>,
    partial: bool,
    limited: bool,
}
impl<C> Default for Recommendations<C> {
    fn default() -> Self {
        Self {
            pending: None,
            videos: Vec::new(),
            next: None,
            page: 0,
            session: None,
            partial: false,
            limited: false,
        }
    }
}
impl<C: Clone> Recommendations<C> {
    fn retained(&self, session: u64) -> bool {
        self.session == Some(session) && self.pending.is_none() && !self.videos.is_empty()
    }
    fn loading_first(&self) -> bool {
        matches!(self.pending, Some((_, Fetch::First, _)))
    }
    fn visible(&self) -> &[VideoSummary] {
        let start = (self.page * RECOMMENDATION_PAGE).min(self.videos.len());
        let end = (start + RECOMMENDATION_PAGE).min(self.videos.len());
        &self.videos[start..end]
    }
    fn has_more(&self) -> bool {
        self.videos.len() > (self.page + 1) * RECOMMENDATION_PAGE || self.next.is_some()
    }
    fn has_previous(&self) -> bool {
        self.page > 0
    }
    /// Pages are fixed 20-row windows; a short window is shown only when YouTube
    /// has no further continuation.
    fn forward(&mut self) -> Step<C> {
        let target = self.page + 1;
        let full = self.videos.len() >= (target + 1) * RECOMMENDATION_PAGE;
        let last = self.videos.len() > target * RECOMMENDATION_PAGE && self.next.is_none();
        if full || last {
            self.page = target;
            Step::Show
        } else if let Some(cursor) = self.next.clone() {
            Step::Fetch(cursor)
        } else {
            Step::Stay
        }
    }
    fn back(&mut self) -> bool {
        if self.page == 0 {
            return false;
        }
        self.page -= 1;
        true
    }
    fn start(&mut self, request: u64, fetch: Fetch, session: u64) {
        self.pending = Some((request, fetch, session));
    }
    fn finish(&mut self, request: u64) -> Option<(Fetch, u64)> {
        match self.pending {
            Some((id, fetch, session)) if id == request => {
                self.pending = None;
                Some((fetch, session))
            }
            _ => None,
        }
    }
    /// Returns whether the visible page changed. Duplicate IDs across provider
    /// pages are dropped; the retained total is bounded.
    fn accept(
        &mut self,
        fetch: Fetch,
        session: u64,
        items: Vec<VideoSummary>,
        next: Option<C>,
        partial: bool,
    ) -> bool {
        if fetch == Fetch::First {
            *self = Self::default();
        } else if self.session != Some(session) {
            // A continuation never extends another session's rows.
            return false;
        }
        self.session = Some(session);
        self.partial = partial;
        for video in items {
            if self.videos.len() >= MAX_RECOMMENDATIONS {
                self.limited = true;
                break;
            }
            if !self.videos.iter().any(|old| old.id == video.id) {
                self.videos.push(video);
            }
        }
        self.next = if self.limited { None } else { next };
        if fetch == Fetch::First {
            return true;
        }
        // Same fixed-window rule as forward(): the next page must be full
        // unless YouTube has no further continuation.
        let target = self.page + 1;
        if self.videos.len() >= (target + 1) * RECOMMENDATION_PAGE
            || (self.videos.len() > target * RECOMMENDATION_PAGE && self.next.is_none())
        {
            self.page = target;
            true
        } else {
            false
        }
    }
}

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
    source: Cell<Source>,
    /// The user's explicit chip choice for the current account identity.
    preferred: Cell<Option<Source>>,
    recommendations: RefCell<Recommendations>,
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
/// Retire only this Home read; accepted writes on the shared workers still
/// finish. A retired account read completes on its worker and is discarded.
pub fn cancel(app: &App, state: &UiState) {
    state.home_ui.cancel_read();
    state.home_ui.recommendations.borrow_mut().pending = None;
    app.set_home_loading(false);
}
fn header(app: &App) {
    app.set_home_source(Source::Saved.index());
    app.set_catalog_title("Home".into());
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
/// `explicit` is true only for a user's Home navigation. Startup passes false,
/// so a clean launch never reads account data even if a session is connected.
pub fn open(app: &App, state: &UiState, explicit: bool) {
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
    // Repeating Home while its first recommendation page is loading keeps that
    // single account request instead of queueing a second one.
    if explicit
        && app.get_account_connected()
        && app.get_home_active()
        && state.home_ui.source.get() == Source::Recommended
        && state.home_ui.recommendations.borrow().loading_first()
    {
        return;
    }
    cancel(app, state);
    match initial_source(
        explicit,
        app.get_account_connected(),
        state.home_ui.preferred.get(),
    ) {
        Source::Recommended => open_recommended(app, state, false),
        Source::Saved => open_saved(app, state),
    }
}
fn open_saved(app: &App, state: &UiState) {
    let same = app.get_home_active()
        && state.home_ui.source.get() == Source::Saved
        && !state.guest_ui.account_home()
        && state.home_ui.pages.borrow().current.is_none();
    if !same {
        crate::guest_ui::begin_home(app, state);
    }
    state.home_ui.source.set(Source::Saved);
    app.set_home_active(true);
    app.set_search_kind(0);
    app.set_page(0);
    header(app);
    app.set_status(
        "Local Home · Saved videos only · Choose a video to connect for playback".into(),
    );
    query(app, state, Pages::default());
}
fn recommended_header(app: &App) {
    app.set_home_source(Source::Recommended.index());
    app.set_catalog_title("Home".into());
    app.set_catalog_empty_title("Recommendations aren’t loaded".into());
    app.set_catalog_empty_message(
        "Use Refresh to read your YouTube home feed.\nYour saved videos stay under Saved.".into(),
    );
    app.set_guest_scope(4);
    app.set_guest_can_back(false);
    app.set_guest_can_follow(false);
}
/// `reuse` shows retained rows of the same account session without a request.
fn open_recommended(app: &App, state: &UiState, reuse: bool) {
    state.home_ui.source.set(Source::Recommended);
    app.set_home_active(true);
    app.set_search_kind(0);
    app.set_page(0);
    recommended_header(app);
    let session = state.account_ui.session_generation();
    if reuse && state.home_ui.recommendations.borrow().retained(session) {
        state.home_ui.recommendations.borrow_mut().page = 0;
        show_recommendations(app, state);
        app.set_status(
            "Recommended by YouTube for your account · Use Refresh for a new set".into(),
        );
        return;
    }
    *state.home_ui.recommendations.borrow_mut() = Recommendations::default();
    crate::guest_ui::begin_account_home(app, state);
    recommended_header(app);
    fetch_recommendations(app, state, Fetch::First, None);
}
fn fetch_recommendations(app: &App, state: &UiState, fetch: Fetch, cursor: Option<AccountCursor>) {
    if state.home_ui.recommendations.borrow().pending.is_some() {
        return;
    }
    let session = state.account_ui.session_generation();
    match state.account_ui.submit_recommendations(app, cursor) {
        Ok(request) => {
            state
                .home_ui
                .recommendations
                .borrow_mut()
                .start(request, fetch, session);
            app.set_home_loading(true);
            app.set_home_error(false);
            app.set_status("Reading recommendations from your YouTube account…".into());
        }
        Err(message) => {
            app.set_home_error(true);
            app.set_catalog_empty_message(message.clone().into());
            app.set_status(format!("Couldn’t load recommendations. {message}").into());
        }
    }
}
fn show_recommendations(app: &App, state: &UiState) {
    let (visible, more, previous) = {
        let recommendations = state.home_ui.recommendations.borrow();
        (
            recommendations.visible().to_vec(),
            recommendations.has_more(),
            recommendations.has_previous(),
        )
    };
    if let Err(error) = crate::guest_ui::publish_account_home(app, state, &visible) {
        app.set_home_error(true);
        app.set_status(error.into());
        return;
    }
    app.set_has_more(more);
    app.set_has_previous(previous);
}
/// Chip selection on Home: 0 Saved, 1 Recommended (verified accounts only).
pub fn select_source(app: &App, state: &UiState, index: i32) {
    let Some(target) = Source::from_index(index) else {
        return;
    };
    if !app.get_home_active()
        || app.get_page() != 0
        || app.get_busy()
        || state.caption_cache.active()
        || state.home_ui.source.get() == target
        || (target == Source::Recommended && !app.get_account_connected())
    {
        app.set_home_source(state.home_ui.source.get().index());
        return;
    }
    state.home_ui.preferred.set(Some(target));
    cancel(app, state);
    match target {
        Source::Saved => open_saved(app, state),
        Source::Recommended => open_recommended(app, state, true),
    }
}
pub fn refresh(app: &App, state: &UiState) {
    if !app.get_home_active() || app.get_page() != 0 || app.get_busy() {
        return;
    }
    if state.home_ui.source.get() == Source::Recommended {
        if !app.get_account_connected() {
            open_saved(app, state);
            return;
        }
        // Keep acknowledged rows visible until the fresh first page arrives.
        fetch_recommendations(app, state, Fetch::First, None);
        return;
    }
    query(app, state, Pages::default());
}
pub fn page(app: &App, state: &UiState, forward: bool) {
    if !app.get_home_active() || app.get_page() != 0 || app.get_busy() {
        return;
    }
    if state.home_ui.source.get() == Source::Recommended {
        recommendation_page(app, state, forward);
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
fn recommendation_page(app: &App, state: &UiState, forward: bool) {
    if state.home_ui.recommendations.borrow().pending.is_some() || !app.get_account_connected() {
        return;
    }
    let step = if forward {
        state.home_ui.recommendations.borrow_mut().forward()
    } else if state.home_ui.recommendations.borrow_mut().back() {
        Step::Show
    } else {
        Step::Stay
    };
    match step {
        Step::Show => show_recommendations(app, state),
        Step::Fetch(cursor) => fetch_recommendations(app, state, Fetch::Next, Some(cursor)),
        Step::Stay => {}
    }
}
/// Account worker completion for a Home request. Errors are typed, fixed
/// messages; provider JSON and credentials never reach this function.
pub fn receive_recommendations(
    app: &App,
    state: &UiState,
    request: u64,
    result: Result<AccountPage<VideoSummary>, WorkerError>,
) {
    let Some((fetch, session)) = state.home_ui.recommendations.borrow_mut().finish(request) else {
        return;
    };
    app.set_home_loading(false);
    if session != state.account_ui.session_generation()
        || !app.get_account_connected()
        || !app.get_home_active()
        || app.get_page() != 0
        || state.home_ui.source.get() != Source::Recommended
        || state.caption_cache.active()
    {
        return;
    }
    let page = match result {
        Ok(page) => page,
        Err(error) => {
            app.set_home_error(true);
            app.set_catalog_empty_message(error.to_string().into());
            app.set_status(format!("Couldn’t load recommendations. {error}").into());
            return;
        }
    };
    let changed = state.home_ui.recommendations.borrow_mut().accept(
        fetch,
        session,
        page.items,
        page.next,
        page.partial,
    );
    let (empty, partial, limited) = {
        let recommendations = state.home_ui.recommendations.borrow();
        (
            recommendations.videos.is_empty(),
            recommendations.partial,
            recommendations.limited,
        )
    };
    app.set_home_error(false);
    if changed {
        recommended_header(app);
        if empty {
            app.set_catalog_empty_title("No recommendations right now".into());
            app.set_catalog_empty_message("YouTube returned no recommendations for this account. If watch history is paused on YouTube, this feed can be empty.".into());
        }
        show_recommendations(app, state);
    } else {
        let recommendations = state.home_ui.recommendations.borrow();
        app.set_has_more(recommendations.has_more());
        app.set_has_previous(recommendations.has_previous());
    }
    app.set_status(
        if limited {
            "Recommendation limit reached for this visit. Use Refresh for a new set."
        } else if !changed {
            "YouTube returned no further recommendations on that page."
        } else if partial {
            "Recommended by YouTube · Some entries, such as mixes or unsupported items, are not shown."
        } else {
            "Recommended by YouTube for your account · Videos play with guest access"
        }
        .into(),
    );
}
/// Identity loss (sign-out, expiry, replacement import): drop every retained
/// account row before another identity can connect, then show Saved.
pub fn account_cleared(app: &App, state: &UiState) {
    *state.home_ui.recommendations.borrow_mut() = Recommendations::default();
    state.home_ui.preferred.set(None);
    let was_recommended = state.home_ui.source.replace(Source::Saved) == Source::Recommended;
    app.set_home_source(Source::Saved.index());
    if !was_recommended && !state.guest_ui.account_home() {
        return;
    }
    app.set_home_loading(false);
    app.set_home_error(false);
    app.set_has_more(false);
    app.set_has_previous(false);
    crate::guest_ui::begin_home(app, state);
    if app.get_home_active() {
        header(app);
        state.home_ui.dirty.set(true);
        maybe_refresh(app, state);
    }
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
    if !app.get_home_active()
        || app.get_page() != 0
        || state.caption_cache.active()
        || state.home_ui.source.get() != Source::Saved
    {
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
    // A local mutation never retires an account recommendation read.
    state.home_ui.cancel_read();
    let saved = state.home_ui.source.get() == Source::Saved;
    if saved {
        app.set_home_loading(false);
    }
    state.home_ui.dirty.set(true);
    {
        let mut pages = state.home_ui.pages.borrow_mut();
        pages.next = None;
        pages.previous.clear();
    }
    if app.get_home_active() && saved {
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
        && state.home_ui.source.get() == Source::Saved
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
    // Clearing also retires visible account rows and their decoded artwork;
    // they are not reloaded until the user explicitly refreshes.
    *state.home_ui.recommendations.borrow_mut() = Recommendations::default();
    if app.get_home_active() && state.home_ui.source.get() == Source::Recommended {
        crate::guest_ui::begin_account_home(app, state);
        recommended_header(app);
        app.set_has_more(false);
        app.set_has_previous(false);
        app.set_home_error(false);
    } else if app.get_home_active() {
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

    // Synthetic TEST FIXTURE videos; IDs are generated, not real content.
    fn videos(range: std::ops::Range<usize>) -> Vec<VideoSummary> {
        range
            .map(|index| VideoSummary {
                metadata: None,
                id: serein_core::VideoId::new(&format!("{index:011}")).unwrap(),
                title: "Synthetic recommendation".into(),
                channel: "Synthetic channel".into(),
                channel_id: None,
                duration: None,
                thumbnail_url: None,
            })
            .collect()
    }
    /// Synthetic opaque cursor; the real provider cursor cannot be forged here.
    type Pager = Recommendations<u8>;
    fn ids(recommendations: &Pager) -> Vec<String> {
        recommendations
            .visible()
            .iter()
            .map(|video| video.id.as_str().to_owned())
            .collect()
    }
    #[test]
    fn home_defaults_to_recommendations_only_for_explicit_connected_visits() {
        assert_eq!(initial_source(true, true, None), Source::Recommended);
        assert_eq!(
            initial_source(true, true, Some(Source::Saved)),
            Source::Saved,
            "an explicit Saved choice is kept for this identity"
        );
        // Clean launch and guests never request account recommendations.
        for preferred in [None, Some(Source::Recommended), Some(Source::Saved)] {
            assert_eq!(initial_source(false, true, preferred), Source::Saved);
            assert_eq!(initial_source(true, false, preferred), Source::Saved);
            assert_eq!(initial_source(false, false, preferred), Source::Saved);
        }
        assert_eq!(Source::from_index(0), Some(Source::Saved));
        assert_eq!(Source::from_index(1), Some(Source::Recommended));
        assert_eq!(Source::from_index(2), None);
        assert_eq!(Source::from_index(-1), None);
    }
    #[test]
    fn recommendation_pages_are_bounded_windows_over_retained_rows() {
        let mut state = Pager::default();
        // Without a cursor the second page is a genuine short final page.
        assert!(state.accept(Fetch::First, 4, videos(0..25), None, false));
        assert_eq!(state.visible().len(), RECOMMENDATION_PAGE);
        assert!(state.has_more() && !state.has_previous());
        assert!(matches!(state.forward(), Step::Show));
        assert_eq!(ids(&state), ids_of(20..25));
        assert!(!state.has_more() && state.has_previous());
        assert!(matches!(state.forward(), Step::Stay));
        assert!(state.back());
        assert_eq!(ids(&state), ids_of(0..20));
        assert!(!state.back());
    }
    fn ids_of(range: std::ops::Range<usize>) -> Vec<String> {
        range.map(|index| format!("{index:011}")).collect()
    }
    #[test]
    fn continuation_fetch_fills_the_next_window_and_drops_duplicates() {
        let mut state = Pager::default();
        state.start(7, Fetch::First, 3);
        assert!(state.loading_first());
        assert!(!state.retained(3), "a pending read is not reusable");
        assert!(
            state.finish(6).is_none(),
            "another request cannot complete it"
        );
        assert_eq!(state.finish(7), Some((Fetch::First, 3)));
        assert!(state.finish(7).is_none(), "a ticket completes once");
        // 25 rows plus a continuation: Next must fetch rather than show a
        // 5-row window, and the fetch carries the provider's opaque cursor.
        assert!(state.accept(Fetch::First, 3, videos(0..25), Some(1), false));
        assert!(state.retained(3) && !state.retained(4));
        state.start(8, Fetch::Next, 3);
        assert!(!state.loading_first() && !state.retained(3));
        assert_eq!(state.finish(8), Some((Fetch::Next, 3)));
        assert!(state.has_more());
        assert!(matches!(state.forward(), Step::Fetch(1)));
        assert_eq!(state.page, 0, "the page moves only after rows arrive");
        // Duplicate IDs from the continuation are dropped.
        assert!(state.accept(Fetch::Next, 3, videos(20..45), Some(2), true));
        assert_eq!(state.page, 1);
        assert_eq!(ids(&state), ids_of(20..40));
        assert_eq!(state.videos.len(), 45);
        assert!(state.partial);
        // A continuation that only repeats rows does not move to a short page
        // while YouTube still offers another continuation.
        assert!(matches!(state.forward(), Step::Fetch(2)));
        assert!(!state.accept(Fetch::Next, 3, videos(0..10), Some(3), false));
        assert_eq!(state.page, 1);
        assert!(state.has_more());
        // Without a further continuation the short final window is shown.
        assert!(matches!(state.forward(), Step::Fetch(3)));
        assert!(state.accept(Fetch::Next, 3, Vec::new(), None, false));
        assert_eq!(ids(&state), ids_of(40..45));
        assert!(!state.has_more());
        assert!(state.back());
        // A continuation completing for another account session never
        // extends or replaces these rows.
        assert!(!state.accept(Fetch::Next, 4, videos(50..55), Some(9), false));
        assert_eq!(state.session, Some(3));
        assert_eq!(state.videos.len(), 45);
        assert_eq!(ids(&state), ids_of(20..40));
    }
    #[test]
    fn retained_recommendations_are_bounded_and_stop_offering_more() {
        let mut state = Pager::default();
        state.accept(Fetch::First, 1, videos(0..150), Some(1), false);
        state.page = 6;
        assert!(state.accept(Fetch::Next, 1, videos(150..260), Some(2), false));
        assert_eq!(state.videos.len(), MAX_RECOMMENDATIONS);
        assert!(state.limited);
        assert!(state.next.is_none(), "no cursor is offered past the bound");
        assert_eq!(state.page, 7);
        assert!(state.visible().len() <= RECOMMENDATION_PAGE);
        // Refresh (First) discards the old set entirely.
        state.accept(Fetch::First, 1, videos(0..3), None, false);
        assert_eq!(state.videos.len(), 3);
        assert_eq!(state.page, 0);
        assert!(!state.limited && !state.has_more());
        // A genuinely empty feed is not reused; switching back reads again.
        state.accept(Fetch::First, 1, Vec::new(), None, false);
        assert!(!state.retained(1));
    }
}
