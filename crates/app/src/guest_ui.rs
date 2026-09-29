// SPDX-License-Identifier: GPL-3.0-or-later
//! Typed guest catalog adapter. Shared card models retain their identity;
//! row replacement occurs only for an explicit catalog/page navigation.
use crate::{App, UiState, VideoRow, catalog::Request};
use serein_core::{CatalogItem, ChannelId, ChannelSummary, PlaylistId, ProviderError, VideoId};
use serein_youtube::catalog::{
    CatalogCursor, CatalogHeader, CatalogPage, CatalogRequest, ChannelTab, SearchKind,
};
use slint::{ComponentHandle, Model};
use std::{cell::RefCell, rc::Rc};

#[derive(Clone)]
struct Location {
    request: CatalogRequest,
    cursor: Option<CatalogCursor>,
}
struct NavigationEntry {
    location: Location,
    previous: Vec<Option<CatalogCursor>>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RequestKind {
    Search,
    Channel,
    Playlist,
    Video,
}
impl RequestKind {
    fn from_request(request: &Request) -> Option<Self> {
        match request {
            Request::Catalog(CatalogRequest::Search { .. }, _) => Some(Self::Search),
            Request::Catalog(CatalogRequest::Channel { .. }, _) => Some(Self::Channel),
            Request::Catalog(CatalogRequest::Playlist { .. }, _) => Some(Self::Playlist),
            Request::Resolve(..) => Some(Self::Video),
            _ => None,
        }
    }
    fn loading(self) -> &'static str {
        match self {
            Self::Search => "Searching public YouTube metadata…",
            Self::Channel => "Loading public channel metadata…",
            Self::Playlist => "Loading public playlist metadata…",
            Self::Video => "Resolving a public video…",
        }
    }
    fn failed(self) -> &'static str {
        match self {
            Self::Search => "Couldn’t load search results",
            Self::Channel => "Couldn’t load this channel",
            Self::Playlist => "Couldn’t load this playlist",
            Self::Video => "Couldn’t open this video",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Idle,
    Loading {
        generation: u64,
        kind: RequestKind,
    },
    Failed,
}
#[derive(Default)]
struct Presentation {
    phase: Phase,
    // A genuine empty page is also acknowledged. Item count cannot answer this.
    acknowledged: bool,
}
impl Presentation {
    fn begin(&mut self, generation: u64, kind: RequestKind) -> bool {
        self.phase = Phase::Loading { generation, kind };
        if kind != RequestKind::Video {
            self.acknowledged = false;
        }
        !self.acknowledged
    }
    fn failed(&mut self, generation: u64) -> Option<RequestKind> {
        let Phase::Loading {
            generation: expected,
            kind,
        } = self.phase
        else {
            return None;
        };
        if generation != expected {
            return None;
        }
        self.phase = Phase::Failed;
        (!self.acknowledged).then_some(kind)
    }
    fn cancelled(&mut self) -> bool {
        let loading = matches!(self.phase, Phase::Loading { .. });
        if loading {
            self.phase = Phase::Idle;
        }
        loading && !self.acknowledged
    }
    fn resolved(&mut self, generation: u64) -> bool {
        if self.phase
            != (Phase::Loading {
                generation,
                kind: RequestKind::Video,
            })
        {
            return false;
        }
        self.phase = Phase::Idle;
        !self.acknowledged
    }
    fn published(&mut self) {
        self.phase = Phase::Idle;
        self.acknowledged = true;
    }
}
#[derive(Default)]
pub struct State {
    presentation: RefCell<Presentation>,
    items: RefCell<Vec<CatalogItem>>,
    current: RefCell<Option<Location>>,
    next: RefCell<Option<CatalogCursor>>,
    previous: RefCell<Vec<Option<CatalogCursor>>>,
    navigation: RefCell<Vec<NavigationEntry>>,
    channel: RefCell<Option<ChannelSummary>>,
}
impl State {
    fn remember(&self) {
        let previous = std::mem::take(&mut *self.previous.borrow_mut());
        if let Some(location) = self.current.borrow().clone() {
            bounded_push(
                &mut self.navigation.borrow_mut(),
                NavigationEntry { location, previous },
                32,
            );
        }
    }
    fn back(&self) -> Option<Location> {
        let entry = self.navigation.borrow_mut().pop()?;
        *self.previous.borrow_mut() = entry.previous;
        Some(entry.location)
    }
    pub fn thumbnail(&self, row: usize) -> Option<String> {
        match self.items.borrow().get(row)? {
            CatalogItem::Video(item) => item.thumbnail_url.clone(),
            CatalogItem::Channel(item) => item.thumbnail_url.clone(),
            CatalogItem::Playlist(item) => item.thumbnail_url.clone(),
        }
    }
}
fn kind(index: i32) -> SearchKind {
    match index {
        1 => SearchKind::Videos,
        2 => SearchKind::Channels,
        3 => SearchKind::Playlists,
        _ => SearchKind::All,
    }
}
enum Input {
    Video(VideoId),
    Catalog(CatalogRequest),
}
fn request_from_input(input: &str, search_kind: SearchKind) -> Result<Input, ProviderError> {
    let input = input.trim();
    if input.is_empty() || input.len() > 2048 || input.chars().any(char::is_control) {
        return Err(ProviderError::InvalidInput);
    }
    if input.contains("://") || input.starts_with("https:") {
        // A watch URL remains a video action even when it contains a playlist.
        if let Ok(id) = VideoId::from_url(input) {
            return Ok(Input::Video(id));
        }
        if let Ok(id) = ChannelId::from_url(input) {
            return Ok(Input::Catalog(CatalogRequest::Channel {
                id,
                tab: ChannelTab::Videos,
            }));
        }
        if let Ok(id) = PlaylistId::from_url(input) {
            return Ok(Input::Catalog(CatalogRequest::Playlist { id }));
        }
        return Err(ProviderError::InvalidInput);
    }
    if input.chars().count() > 200 {
        return Err(ProviderError::InvalidInput);
    }
    Ok(Input::Catalog(CatalogRequest::Search {
        query: input.to_owned(),
        kind: search_kind,
    }))
}
fn row(item: &CatalogItem) -> VideoRow {
    match item {
        CatalogItem::Video(video) => {
            let mut row = crate::video_row(video);
            row.kind = "Video".into();
            row
        }
        CatalogItem::Channel(channel) => VideoRow {
            title: channel.title.clone().into(),
            channel: channel
                .subscriber_count
                .map(|n| format!("{n} subscribers"))
                .unwrap_or_else(|| "Public YouTube channel".into())
                .into(),
            id: channel.id.as_str().into(),
            kind: "Channel".into(),
            ..VideoRow::default()
        },
        CatalogItem::Playlist(playlist) => VideoRow {
            title: playlist.title.clone().into(),
            channel: match (&playlist.channel, playlist.video_count) {
                (Some(channel), Some(count)) => format!("{channel} · {count} videos"),
                (Some(channel), None) => channel.clone(),
                (None, Some(count)) => format!("{count} videos"),
                _ => "Public YouTube playlist".into(),
            }
            .into(),
            id: playlist.id.as_str().into(),
            kind: "Playlist".into(),
            ..VideoRow::default()
        },
    }
}
fn bounded_push<T>(list: &mut Vec<T>, item: T, limit: usize) {
    if list.len() == limit {
        list.remove(0);
    }
    list.push(item);
}
fn load(app: &App, s: &UiState, location: Location, remember: bool) {
    crate::home_ui::cancel(app, s);
    app.set_home_active(false);
    if remember {
        s.guest_ui.remember();
    }
    s.guest_ui.channel.borrow_mut().take();
    app.set_guest_can_follow(false);
    match &location.request {
        CatalogRequest::Search { query, kind } => {
            app.set_search_kind(match kind {
                SearchKind::All => 0,
                SearchKind::Videos => 1,
                SearchKind::Channels => 2,
                SearchKind::Playlists => 3,
            });
            app.set_catalog_title(format!("Results for “{query}”").into());
            app.set_catalog_subtitle(
                "Public YouTube results · Choose a video, channel or playlist".into(),
            );
            app.set_guest_scope(0);
        }
        CatalogRequest::Channel { tab, .. } => {
            app.set_catalog_title("YouTube channel".into());
            app.set_catalog_subtitle("Loading public channel metadata…".into());
            app.set_guest_scope(1);
            app.set_channel_tab(match tab {
                ChannelTab::Videos => 0,
                ChannelTab::Shorts => 1,
                ChannelTab::Streams => 2,
                ChannelTab::Playlists => 3,
            });
        }
        CatalogRequest::Playlist { .. } => {
            app.set_catalog_title("YouTube playlist".into());
            app.set_catalog_subtitle("Loading public playlist metadata…".into());
            app.set_guest_scope(2);
        }
    }
    s.worker.borrow_mut().submit(Request::Catalog(
        location.request.clone(),
        location.cursor.clone(),
    ));
    *s.guest_ui.current.borrow_mut() = Some(location);
    s.guest_ui.next.borrow_mut().take();
    s.guest_ui.items.borrow_mut().clear();
    crate::feed_focus::reset(app, s);
    s.model.replace(Vec::new());
    s.groups.replace(&s.model);
    s.thumbnail_attempted.borrow_mut().clear();
    s.thumbnail_range.set((usize::MAX, usize::MAX));
    app.set_has_more(false);
    app.set_has_previous(!s.guest_ui.previous.borrow().is_empty());
    app.set_guest_can_back(!s.guest_ui.navigation.borrow().is_empty());
    app.set_page(0);
    let _ = s.player.set_paused(true);
    app.set_busy(true);
    app.set_status("Contacting YouTube in guest mode…".into());
    app.invoke_refresh_visible();
}
/// Called only for current guest worker errors, irrespective of retry eligibility.
pub fn failed(app: &App, state: &UiState, generation: u64, error: ProviderError) {
    let kind = state.guest_ui.presentation.borrow_mut().failed(generation);
    let Some(kind) = kind else { return };
    app.set_catalog_subtitle(kind.failed().into());
    app.set_catalog_empty_title(kind.failed().into());
    // Fixed typed error descriptions are bounded. Backoff/detail text remains
    // in the full status message, not an unrelated empty-feed binding.
    app.set_catalog_empty_message(error.to_string().into());
}
pub fn resolution_finished(app: &App, state: &UiState, generation: u64) {
    let reset = state
        .guest_ui
        .presentation
        .borrow_mut()
        .resolved(generation);
    if reset {
        app.set_catalog_title("Find your next watch".into());
        app.set_catalog_subtitle("Search YouTube, or paste a video link above.".into());
        app.set_guest_scope(0);
        app.set_catalog_empty_title("Watch with intention".into());
        app.set_catalog_empty_message(
            "Start with a search or a YouTube link.\nNo autoplay. No endless recommended feed."
                .into(),
        );
    }
}
pub fn publish(app: &App, s: &UiState, page: CatalogPage) {
    s.guest_ui.presentation.borrow_mut().published();
    match page.header {
        CatalogHeader::Search => app.set_catalog_subtitle(
            "Public YouTube results · Choose a video, channel or playlist".into(),
        ),
        CatalogHeader::Channel(channel) => {
            *s.guest_ui.channel.borrow_mut() = Some(channel.clone());
            app.set_guest_can_follow(true);
            app.set_catalog_title(channel.title.into());
            app.set_catalog_subtitle(
                channel
                    .description
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "Public channel · Guest browsing".into())
                    .into(),
            );
        }
        CatalogHeader::Playlist(playlist) => {
            app.set_catalog_title(playlist.title.into());
            app.set_catalog_subtitle(
                playlist
                    .description
                    .filter(|s| !s.is_empty())
                    .or(playlist.channel)
                    .unwrap_or_else(|| "Public playlist · Guest browsing".into())
                    .into(),
            );
        }
    }
    let items: Vec<_> = page.items.into_iter().take(20).collect();
    app.set_catalog_empty_title("No public results".into());
    app.set_catalog_empty_message(if page.partial {
        "No supported public items remain on this page. Some entries were unavailable or unsupported."
    } else { "No public results on this page. Try another search or choose another collection." }.into());
    s.model.append(items.iter().map(row).collect());
    *s.guest_ui.items.borrow_mut() = items;
    s.groups.append(&s.model);
    *s.guest_ui.next.borrow_mut() = page.next;
    app.set_has_more(s.guest_ui.next.borrow().is_some());
    s.thumbnail_attempted.borrow_mut().clear();
    s.thumbnail_range.set((usize::MAX, usize::MAX));
    app.invoke_refresh_visible();
    app.set_status(
        if page.limit_reached {
            "Catalog safety limit reached. Narrow the search or choose another collection."
        } else if page.partial {
            "Public results · Some unavailable or unsupported entries were omitted."
        } else if s.guest_ui.items.borrow().is_empty() {
            "No public results on this page."
        } else {
            "Public YouTube catalog · Guest mode · Up to 20 records per page"
        }
        .into(),
    );
}
/// Retire provider navigation and thumbnail generations before any local rows.
pub fn begin_home(app: &App, state: &UiState) {
    state.thumbnails.borrow_mut().replace(Vec::new());
    state.thumbnail_attempted.borrow_mut().clear();
    state.thumbnail_range.set((usize::MAX, usize::MAX));
    state.guest_ui.current.borrow_mut().take();
    state.guest_ui.next.borrow_mut().take();
    state.guest_ui.previous.borrow_mut().clear();
    state.guest_ui.navigation.borrow_mut().clear();
    state.guest_ui.channel.borrow_mut().take();
    state.guest_ui.items.borrow_mut().clear();
    state.guest_ui.presentation.borrow_mut().published();
    crate::feed_focus::reset(app, state);
    state.model.replace(Vec::new());
    state.groups.replace(&state.model);
    app.set_has_more(false);
    app.set_has_previous(false);
    app.set_guest_can_back(false);
    app.set_guest_can_follow(false);
    app.invoke_reset_feed_scroll();
}
pub fn local_videos(state: &UiState) -> Option<Vec<serein_core::VideoSummary>> {
    state
        .guest_ui
        .items
        .borrow()
        .iter()
        .map(|item| match item {
            CatalogItem::Video(video) => Some(video.clone()),
            _ => None,
        })
        .collect()
}
pub fn publish_home(
    app: &App,
    state: &UiState,
    videos: Vec<serein_core::VideoSummary>,
    same_page: bool,
) -> Result<(), &'static str> {
    if videos.len() > serein_storage::MAX_PAGE_SIZE as usize
        || videos.iter().any(|video| video.thumbnail_url.is_some())
        || videos
            .iter()
            .enumerate()
            .any(|(i, video)| videos[..i].iter().any(|old| old.id == video.id))
    {
        return Err("Invalid local Home page. Use Refresh to try again.");
    }
    let items: Vec<_> = videos.into_iter().map(CatalogItem::Video).collect();
    let rows: Vec<_> = items.iter().map(row).collect();
    let changed = state.model.row_count() != rows.len()
        || rows
            .iter()
            .enumerate()
            .any(|(i, row)| state.model.row_data(i).as_ref() != Some(row));
    if same_page && changed {
        crate::feed_focus::reconcile_home(app, state, || {
            state.model.reconcile(rows, |row| row.id.clone())?;
            state.groups.reconcile(&state.model);
            Ok(())
        })?;
    } else if !same_page {
        crate::feed_focus::reset(app, state);
        state.model.replace(rows);
        state.groups.replace(&state.model);
        app.invoke_reset_feed_scroll();
    }
    *state.guest_ui.items.borrow_mut() = items;
    state.guest_ui.presentation.borrow_mut().published();
    state.thumbnail_attempted.borrow_mut().clear();
    state.thumbnail_range.set((usize::MAX, usize::MAX));
    app.invoke_refresh_visible();
    Ok(())
}
pub fn bind(app: &App, state: &Rc<UiState>) {
    let weak = app.as_weak();
    let state_weak = Rc::downgrade(state);
    state.worker.borrow_mut().on_generation_changed(move |_| {
        let (Some(app), Some(state)) = (weak.upgrade(), state_weak.upgrade()) else {
            return;
        };
        let cancelled = state.guest_ui.presentation.borrow_mut().cancelled();
        if cancelled {
            app.set_catalog_subtitle("Request cancelled.".into());
            app.set_catalog_empty_title("Request cancelled".into());
            app.set_catalog_empty_message("Search again or choose another page.".into());
        }
    });
    let weak = app.as_weak();
    let state_weak = Rc::downgrade(state);
    state
        .worker
        .borrow_mut()
        .on_submitted(move |generation, request| {
            let (Some(app), Some(state)) = (weak.upgrade(), state_weak.upgrade()) else {
                return;
            };
            let Some(kind) = RequestKind::from_request(request) else {
                return;
            };
            crate::home_ui::cancel(&app, &state);
            let visible = state
                .guest_ui
                .presentation
                .borrow_mut()
                .begin(generation, kind);
            if visible {
                if kind == RequestKind::Video {
                    app.set_catalog_title("YouTube video".into());
                    app.set_guest_scope(3); // No unrelated channel/search filter for a video error.
                    app.set_guest_can_follow(false);
                }
                app.set_catalog_subtitle(kind.loading().into());
                app.set_catalog_empty_title(
                    if kind == RequestKind::Video {
                        "Opening video…"
                    } else {
                        "Loading YouTube…"
                    }
                    .into(),
                );
                app.set_catalog_empty_message(kind.loading().into());
            }
        });

    let weak = app.as_weak();
    let s = state.clone();
    app.on_search(move |query| {
        let Some(app) = weak.upgrade() else { return };
        if !crate::playback_preferences::admit_search(&app, &s) {
            return;
        }
        s.worker.borrow_mut().cancel();
        app.set_busy(false);
        // Cancelling work does not replace the acknowledged catalog page.
        // Invalid input and failed video extraction must retain its navigation;
        // load() clears it only when an actual catalog navigation is admitted.
        match request_from_input(query.as_str(), kind(app.get_search_kind())) {
            Ok(Input::Video(id)) => {
                let quality = crate::library_ui::desired_preferences(&s).playback.quality;
                s.worker.borrow_mut().submit(Request::Resolve(id, quality));
                s.focus_intent.arm(
                    crate::focus_intent::Scope::Guest(s.worker.borrow().generation()),
                    app.get_search_active(),
                );
                app.set_busy(true);
                app.set_status("Resolving selected content stream…".into());
            }
            Ok(Input::Catalog(request)) => load(
                &app,
                &s,
                Location {
                    request,
                    cursor: None,
                },
                true,
            ),
            Err(error) => app.set_status(error.to_string().into()),
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_more(move || {
        let Some(app) = weak.upgrade() else { return };
        if app.get_home_active() {
            crate::home_ui::page(&app, &s, true);
            return;
        }
        if app.get_busy() {
            return;
        }
        let Some(cursor) = s.guest_ui.next.borrow().clone() else {
            return;
        };
        let Some(mut current) = s.guest_ui.current.borrow().clone() else {
            return;
        };
        bounded_push(
            &mut s.guest_ui.previous.borrow_mut(),
            current.cursor.clone(),
            1024,
        );
        current.cursor = Some(cursor);
        load(&app, &s, current, false);
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_guest_previous(move || {
        let Some(app) = weak.upgrade() else { return };
        if app.get_home_active() {
            crate::home_ui::page(&app, &s, false);
            return;
        }
        if app.get_busy() {
            return;
        }
        let Some(cursor) = s.guest_ui.previous.borrow_mut().pop() else {
            return;
        };
        let Some(mut current) = s.guest_ui.current.borrow().clone() else {
            return;
        };
        current.cursor = cursor;
        load(&app, &s, current, false);
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_guest_back(move || {
        let Some(app) = weak.upgrade() else { return };
        let Some(location) = s.guest_ui.back() else {
            return;
        };
        load(&app, &s, location, false);
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_search_kind_changed(move |index| {
        let Some(app) = weak.upgrade() else { return };
        app.set_search_kind(index);
        let current = s.guest_ui.current.borrow().clone();
        if let Some(Location {
            request: CatalogRequest::Search { query, .. },
            ..
        }) = current
        {
            load(
                &app,
                &s,
                Location {
                    request: CatalogRequest::Search {
                        query,
                        kind: kind(index),
                    },
                    cursor: None,
                },
                true,
            );
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_channel_tab_changed(move |index| {
        let Some(app) = weak.upgrade() else { return };
        let current = s.guest_ui.current.borrow().clone();
        if let Some(Location {
            request: CatalogRequest::Channel { id, .. },
            ..
        }) = current
        {
            let tab = match index {
                1 => ChannelTab::Shorts,
                2 => ChannelTab::Streams,
                3 => ChannelTab::Playlists,
                _ => ChannelTab::Videos,
            };
            load(
                &app,
                &s,
                Location {
                    request: CatalogRequest::Channel { id, tab },
                    cursor: None,
                },
                true,
            );
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_follow_guest_channel(move || {
        let Some(app) = weak.upgrade() else { return };
        if app.get_busy() || !app.get_guest_can_follow() {
            return;
        }
        let Some(channel) = s.guest_ui.channel.borrow().clone() else {
            return;
        };
        crate::library_ui::follow_channel(&app, &s, &channel.id, &channel.title);
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_select_video(move |index| {
        let Some(app) = weak.upgrade() else { return };
        if app.get_busy() {
            return;
        }
        if !crate::playback_preferences::admit_search(&app, &s) {
            return;
        }
        let item = s.guest_ui.items.borrow().get(index as usize).cloned();
        match item {
            Some(CatalogItem::Video(video)) => {
                let quality = crate::library_ui::desired_preferences(&s).playback.quality;
                s.worker
                    .borrow_mut()
                    .submit(Request::Resolve(video.id, quality));
                s.focus_intent.arm(
                    crate::focus_intent::Scope::Guest(s.worker.borrow().generation()),
                    app.get_search_active(),
                );
                app.set_busy(true);
                app.set_status("Resolving selected content stream…".into());
            }
            Some(CatalogItem::Channel(channel)) => load(
                &app,
                &s,
                Location {
                    request: CatalogRequest::Channel {
                        id: channel.id,
                        tab: ChannelTab::Videos,
                    },
                    cursor: None,
                },
                true,
            ),
            Some(CatalogItem::Playlist(playlist)) => load(
                &app,
                &s,
                Location {
                    request: CatalogRequest::Playlist { id: playlist.id },
                    cursor: None,
                },
                true,
            ),
            None => {}
        }
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn presentation_rejects_stale_terminal_events_and_cancellation_clears_loading() {
        let mut state = Presentation::default();
        assert!(state.begin(7, RequestKind::Channel));
        assert!(state.begin(8, RequestKind::Playlist));
        assert_eq!(state.failed(7), None);
        assert_eq!(
            state.phase,
            Phase::Loading {
                generation: 8,
                kind: RequestKind::Playlist
            }
        );
        assert_eq!(state.failed(8), Some(RequestKind::Playlist));
        assert_eq!(state.phase, Phase::Failed);
        assert!(!state.cancelled());
        assert!(state.begin(9, RequestKind::Video));
        assert!(state.cancelled());
        assert_eq!(state.failed(9), None);
        assert_eq!(state.phase, Phase::Idle);
        state.begin(10, RequestKind::Video);
        assert!(!state.resolved(9));
        assert_eq!(
            state.phase,
            Phase::Loading {
                generation: 10,
                kind: RequestKind::Video
            }
        );
        assert!(state.resolved(10));
        assert_eq!(state.phase, Phase::Idle);
    }
    #[test]
    fn genuine_empty_catalog_is_preserved_across_video_failure_and_cancel() {
        let mut state = Presentation::default();
        state.begin(1, RequestKind::Search);
        state.published(); // Acknowledged empty/nonempty pages have identical ownership.
        assert!(!state.begin(2, RequestKind::Video));
        assert_eq!(state.failed(2), None);
        assert!(state.acknowledged);
        assert!(!state.begin(3, RequestKind::Video));
        assert!(!state.cancelled());
        assert!(state.acknowledged);
        assert!(state.begin(4, RequestKind::Channel));
        assert!(!state.acknowledged);
        assert_eq!(state.failed(4), Some(RequestKind::Channel));
        assert!(state.begin(5, RequestKind::Video));
        assert_eq!(state.failed(5), Some(RequestKind::Video));
    }
    #[test]
    fn only_catalog_and_initial_resolve_own_empty_feed_presentation() {
        let id = VideoId::new("aqz-KE-bpKQ").unwrap();
        assert_eq!(
            RequestKind::from_request(&Request::Resolve(id.clone(), Default::default())),
            Some(RequestKind::Video)
        );
        assert_eq!(
            RequestKind::from_request(&Request::Comments(id.clone(), None)),
            None
        );
        assert_eq!(
            RequestKind::from_request(&Request::ResolveQuality(
                id,
                serein_youtube::ResolutionPolicy {
                    max_height: 720,
                    prefer_h264: true
                }
            )),
            None
        );
    }
    #[test]
    fn urls_route_to_real_typed_targets_and_reject_unknown_hosts() {
        assert!(matches!(
            request_from_input(
                "https://www.youtube.com/channel/UCabcdefghijklmnopqrstuv",
                SearchKind::All
            ),
            Ok(Input::Catalog(CatalogRequest::Channel { .. }))
        ));
        assert!(matches!(
            request_from_input(
                "https://www.youtube.com/playlist?list=PLsynthetic",
                SearchKind::All
            ),
            Ok(Input::Catalog(CatalogRequest::Playlist { .. }))
        ));
        assert!(matches!(
            request_from_input(
                "https://www.youtube.com/watch?v=abcdefghijk&list=PLsynthetic",
                SearchKind::All
            ),
            Ok(Input::Video(_))
        ));
        assert!(
            request_from_input(
                "https://evil.example/playlist?list=PLsynthetic",
                SearchKind::All
            )
            .is_err()
        );
    }
    #[test]
    fn rows_keep_channel_and_playlist_kinds_without_fake_video_summaries() {
        let channel = CatalogItem::Channel(serein_core::ChannelSummary {
            id: ChannelId::new("UCabcdefghijklmnopqrstuv").unwrap(),
            title: "Synthetic channel".into(),
            description: None,
            thumbnail_url: None,
            subscriber_count: None,
        });
        let row = row(&channel);
        assert_eq!(row.kind.as_str(), "Channel");
        assert!(row.duration.is_empty());
        let playlist = CatalogItem::Playlist(serein_core::PlaylistSummary {
            id: PlaylistId::new("PLsynthetic").unwrap(),
            title: "Synthetic playlist".into(),
            description: None,
            channel: None,
            channel_id: None,
            thumbnail_url: None,
            video_count: None,
        });
        assert_eq!(super::row(&playlist).kind.as_str(), "Playlist");
    }
    #[test]
    fn back_restores_the_previous_page_history_for_that_catalog() {
        let state = State::default();
        let request = CatalogRequest::Search {
            query: "Synthetic".into(),
            kind: SearchKind::All,
        };
        *state.current.borrow_mut() = Some(Location {
            request: request.clone(),
            cursor: None,
        });
        // None is the real first-page cursor. The next/previous cursor's private
        // token does not need to be fabricated to exercise history ownership.
        state.previous.borrow_mut().push(None);
        state.remember();
        assert!(state.previous.borrow().is_empty());
        *state.current.borrow_mut() = Some(Location {
            request: CatalogRequest::Playlist {
                id: PlaylistId::new("PLsynthetic").unwrap(),
            },
            cursor: None,
        });
        let restored = state.back().unwrap();
        assert!(restored.request == request);
        assert_eq!(state.previous.borrow().len(), 1);
        assert!(state.previous.borrow()[0].is_none());
        assert!(state.back().is_none());
        assert_eq!(state.previous.borrow().len(), 1);
    }
    #[test]
    fn generated_external_routes_keep_identity_and_reject_authority_mutations() {
        for index in 0..128 {
            let video = format!("{index:011}");
            let channel = format!("UC{index:022}");
            let playlist = format!("PLsynthetic{index}");
            match request_from_input(
                &format!("https://www.youtube.com/watch?v={video}&list={playlist}"),
                SearchKind::All,
            )
            .unwrap()
            {
                Input::Video(id) => assert_eq!(id.as_str(), video),
                _ => panic!("watch URL lost its video identity"),
            }
            assert!(
                matches!(request_from_input(&format!("https://www.youtube.com/channel/{channel}"), SearchKind::All), Ok(Input::Catalog(CatalogRequest::Channel { id, .. })) if id.as_str() == channel)
            );
            assert!(
                matches!(request_from_input(&format!("https://www.youtube.com/playlist?list={playlist}"), SearchKind::All), Ok(Input::Catalog(CatalogRequest::Playlist { id })) if id.as_str() == playlist)
            );
            for authority in [
                format!("www.youtube.com.attacker{index}.invalid"),
                format!("www.youtube.com@attacker{index}.invalid"),
                "www.youtube.com:444".into(),
                "user@www.youtube.com".into(),
            ] {
                for path in [
                    format!("/watch?v={video}"),
                    format!("/channel/{channel}"),
                    format!("/playlist?list={playlist}"),
                ] {
                    assert!(
                        request_from_input(&format!("https://{authority}{path}"), SearchKind::All)
                            .is_err()
                    );
                }
            }
            assert!(
                request_from_input(
                    &format!("https://www.youtube.com/watch?v={video}\0"),
                    SearchKind::All
                )
                .is_err()
            );
        }
    }
    #[test]
    fn navigation_storage_is_bounded() {
        let mut data = Vec::new();
        for i in 0..2000 {
            bounded_push(&mut data, i, 32);
        }
        assert_eq!(data.len(), 32);
        assert_eq!(data[0], 1968);
    }
}
