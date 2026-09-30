// SPDX-License-Identifier: GPL-3.0-or-later
//! Bounded local-library pages. SQLite and selected-file I/O stay on the worker.
use crate::{App, LibraryRow, LibraryUi, SaveUi, UiState, library};
use serein_core::{ChannelId, VideoSummary};
use serein_storage::{
    HistoryCursor, HistoryEntry, LocalPlaylistId, LocalSubscription, PageCursor, PlaylistWindow,
};
use slint::{ComponentHandle, Model};
#[path = "library_model.rs"]
mod library_model;
#[path = "library_organization.rs"]
mod organization;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, SystemTime},
};

#[derive(Clone)]
enum Cursor {
    Page(PageCursor),
    History(HistoryCursor),
}
enum Item {
    Video(VideoSummary),
    Follow(LocalSubscription),
    History(HistoryEntry),
}
#[derive(Clone, PartialEq, Eq)]
enum ItemKey {
    Video(serein_core::VideoId),
    Follow(ChannelId),
    History(serein_core::VideoId),
}
impl Item {
    fn key(&self) -> ItemKey {
        match self {
            Self::Video(v) => ItemKey::Video(v.id.clone()),
            Self::Follow(v) => ItemKey::Follow(v.channel_id.clone()),
            Self::History(v) => ItemKey::History(v.video.id.clone()),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum Route {
    Collections(u64),
    Videos(LocalPlaylistId, u64),
    Subscriptions(u64),
    History(u64),
    Empty(u64),
}
impl Route {
    fn accepts(&self, result: &library::PageResult) -> bool {
        matches!(
            (self, result),
            (Self::Collections(_), library::PageResult::Collections(_))
                | (
                    Self::Subscriptions(_),
                    library::PageResult::Subscriptions(_)
                )
                | (Self::History(_), library::PageResult::History { .. })
        ) || matches!((self,result), (Self::Videos(expected,_), library::PageResult::Videos(actual,_)) if expected==actual)
    }
}
#[derive(Default)]
struct Reads {
    serial: u64,
    pending: Option<(u64, Route)>,
}
impl Reads {
    fn start(&mut self, route: Route) -> Option<u64> {
        if self.pending.is_some() {
            return None;
        }
        self.serial = self.serial.checked_add(1)?;
        self.pending = Some((self.serial, route));
        Some(self.serial)
    }
    fn finish(&mut self, ticket: u64) -> Option<Route> {
        if self.pending.as_ref().is_some_and(|(id, _)| *id == ticket) {
            self.pending.take().map(|(_, route)| route)
        } else {
            None
        }
    }
}
#[derive(Clone)]
struct SaveTarget {
    video: VideoSummary,
    load: u64,
    collections: Vec<serein_storage::LocalPlaylist>,
}
impl SaveTarget {
    fn matches(&self, video: &VideoSummary, snapshot: &serein_media::Snapshot) -> bool {
        self.video.id == video.id && self.load == snapshot.load_request_id && savable_load(snapshot)
    }
}
struct PendingSave {
    serial: u64,
    collection_name: String,
}
struct RenameTarget {
    id: LocalPlaylistId,
    epoch: u64,
    create_draft: slint::SharedString,
    context_valid: bool,
}
impl RenameTarget {
    fn matches(&self, selected: Option<LocalPlaylistId>, epoch: u64, visible: bool) -> bool {
        self.context_valid && visible && selected == Some(self.id) && epoch == self.epoch
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum NameKind {
    Create,
    Rename,
}
#[derive(Clone, Copy)]
struct CreateContext {
    route_epoch: u64,
    collection_epoch: u64,
    visible: bool,
}
impl CreateContext {
    fn matches(self, route_epoch: u64, collection_epoch: u64, visible: bool) -> bool {
        self.visible
            && visible
            && self.route_epoch == route_epoch
            && self.collection_epoch == collection_epoch
    }
}
struct CreatedSelection {
    ticket: u64,
    id: LocalPlaylistId,
    context: CreateContext,
}
struct PendingFilter {
    ticket: u64,
    origin: Route,
    filter: String,
}
#[derive(Clone, Default)]
struct CollectionWindow {
    current: PlaylistWindow,
    previous: Option<PlaylistWindow>,
    next: Option<PlaylistWindow>,
}
impl CollectionWindow {
    fn advance(&mut self, forward: bool) -> bool {
        let Some(query) = (if forward { self.next } else { self.previous }) else {
            return false;
        };
        self.current = query;
        true
    }
}
struct NameWrite {
    kind: NameKind,
    draft: slint::SharedString,
    rename_identity: Option<(LocalPlaylistId, u64)>,
    create_context: Option<CreateContext>,
}
fn playlist_name(draft: &str) -> Result<String, &'static str> {
    let name = draft.trim();
    if name.is_empty() {
        Err("Enter a name for the playlist.")
    } else if draft.chars().any(char::is_control) {
        Err("Playlist names cannot contain line breaks or control characters.")
    } else if name.len() > 1024 {
        Err("That playlist name is too long. Try a shorter name.")
    } else {
        Ok(name.to_owned())
    }
}
fn savable_load(snapshot: &serein_media::Snapshot) -> bool {
    !snapshot.stop_pending
        && snapshot.load_request_id != 0
        && snapshot.playback_restarted
        && snapshot.load_request_id == snapshot.active_load_request_id
        && snapshot.failed_load_request_id != Some(snapshot.load_request_id)
        && matches!(
            snapshot.state,
            serein_media::PlaybackState::Playing
                | serein_media::PlaybackState::Paused
                | serein_media::PlaybackState::Buffering
                | serein_media::PlaybackState::Ended
        )
}
#[derive(Clone, Default)]
struct Pagination {
    current: Option<Cursor>,
    next: Option<Cursor>,
    previous: Vec<Option<Cursor>>,
}
impl Pagination {
    fn advance(&mut self, forward: bool) -> bool {
        if forward {
            let Some(next) = self.next.clone() else {
                return false;
            };
            if self.previous.len() == 1024 {
                self.previous.remove(0);
            }
            self.previous.push(self.current.clone());
            self.current = Some(next);
        } else {
            let Some(previous) = self.previous.pop() else {
                return false;
            };
            self.current = previous;
        }
        true
    }
    fn page(&self) -> Option<PageCursor> {
        match &self.current {
            Some(Cursor::Page(c)) => Some(*c),
            _ => None,
        }
    }
    fn history(&self) -> Option<HistoryCursor> {
        match &self.current {
            Some(Cursor::History(c)) => Some(c.clone()),
            _ => None,
        }
    }
}
#[derive(Default)]
pub struct State {
    organization: organization::State,
    filter: RefCell<String>,
    pending_filter: RefCell<Option<PendingFilter>>,
    rename_target: RefCell<Option<RenameTarget>>,
    name_write: RefCell<Option<NameWrite>>,
    rows: Rc<slint::VecModel<LibraryRow>>,
    collection_names: Rc<slint::VecModel<slint::SharedString>>,
    published_route: RefCell<Option<Route>>,
    published_collection_epoch: Cell<Option<u64>>,
    route_epoch: Cell<u64>,
    collection_epoch: Cell<u64>,
    reads: RefCell<Reads>,
    last_page_read: Cell<Option<u64>>,
    collection_origin: RefCell<Option<(u64, CollectionWindow)>>,
    created_selection: RefCell<Option<CreatedSelection>>,
    row_changes: Cell<u64>,
    collection_changes: Cell<u64>,
    row_resets: Cell<u64>,
    collection_replacements: Cell<u64>,
    items: RefCell<Vec<Item>>,
    pages: RefCell<Pagination>,
    collections: RefCell<CollectionWindow>,
    pending: Cell<bool>,
    requested_preferences: Cell<Option<library::PreferenceWrite>>,
    preference_serial: Cell<u64>,
    volume_save: slint::Timer,
    picker: RefCell<Option<slint::JoinHandle<()>>>,
    last_record: RefCell<Option<(serein_core::VideoId, u64)>>,
    save_target: RefCell<Option<SaveTarget>>,
    pending_save: RefCell<Option<PendingSave>>,
    save_serial: Cell<u64>,
}
/// Counts application-owned model mutations; not Slint draws or item instances.
pub fn notification_counts(state: &UiState) -> (u64, u64) {
    (
        state.library_ui.row_resets.get() + state.library_ui.row_changes.get(),
        state.library_ui.collection_replacements.get() + state.library_ui.collection_changes.get(),
    )
}
/// A single acknowledged page, never all 10,000 fixture entries.
pub fn fixture_videos(state: &UiState) -> Option<Vec<VideoSummary>> {
    state.library_fixture.as_ref()?;
    acknowledged_video_page(state)
}
/// The currently published bounded worker page, used by finite diagnostics to
/// verify committed reads without opening SQLite on the event-loop thread.
pub fn acknowledged_video_page(state: &UiState) -> Option<Vec<VideoSummary>> {
    if state.library_ui.pending.get() {
        return None;
    }
    let items = state.library_ui.items.borrow();
    if items.len() > 100 {
        return None;
    }
    items
        .iter()
        .map(|item| match item {
            Item::Video(video) => Some(video.clone()),
            _ => None,
        })
        .collect()
}
impl State {
    fn desired_preferences(
        &self,
        committed: serein_storage::LocalPreferences,
    ) -> serein_storage::LocalPreferences {
        self.requested_preferences
            .get()
            .map(|write| write.value)
            .unwrap_or(committed)
    }
    fn finish_video_save(&self, serial: u64) -> Option<PendingSave> {
        let mut pending = self.pending_save.borrow_mut();
        if pending.as_ref().is_some_and(|save| save.serial == serial) {
            pending.take()
        } else {
            None
        }
    }
    fn finish_preferences(&self, write: library::PreferenceWrite) -> bool {
        if self.requested_preferences.get() == Some(write) {
            self.requested_preferences.set(None);
            true
        } else {
            false
        }
    }
    pub fn stop_picker(&self) {
        if let Some(task) = self.picker.borrow_mut().take() {
            task.abort();
        }
    }
}
/// Coalesce slider/keyboard changes; SQL never runs on the event thread.
pub fn schedule_volume_save(app: &App, state: &UiState) {
    if state.caption_cache.active() {
        return;
    }
    let weak = app.as_weak();
    state.library_ui.volume_save.start(
        slint::TimerMode::SingleShot,
        Duration::from_millis(250),
        move || {
            if let Some(app) = weak.upgrade() {
                app.invoke_preferences_changed();
            }
        },
    );
}
pub fn flush_volume_save(app: &App, state: &UiState) {
    if state.library_ui.volume_save.running() {
        state.library_ui.volume_save.stop();
        app.invoke_preferences_changed();
    }
}
fn status(app: &App, message: impl Into<slint::SharedString>) {
    let message = message.into();
    app.global::<LibraryUi>().set_status(message.clone());
    app.set_status(message);
}
fn name_status(app: &App, message: impl Into<slint::SharedString>) {
    let message = message.into();
    app.global::<LibraryUi>().set_name_status(message.clone());
    status(app, message);
}
fn cancel_name(app: &App, state: &UiState) {
    let target = state.library_ui.rename_target.borrow_mut().take();
    let ui = app.global::<LibraryUi>();
    if let Some(target) = target {
        ui.set_name_draft(target.create_draft);
        ui.set_name_status("".into());
    }
    ui.set_renaming(false);
}
fn finish_name(app: &App, state: &UiState, kind: NameKind) -> Option<NameWrite> {
    let completed = {
        let mut pending = state.library_ui.name_write.borrow_mut();
        if pending.as_ref().is_some_and(|write| write.kind == kind) {
            pending.take()
        } else {
            None
        }
    };
    let write = completed?;
    let ui = app.global::<LibraryUi>();
    if kind == NameKind::Rename {
        let current = state
            .library_ui
            .rename_target
            .borrow()
            .as_ref()
            .map(|target| (target.id, target.epoch));
        if current != write.rename_identity {
            return Some(write);
        }
        cancel_name(app, state);
    } else if ui.get_name_draft() == write.draft {
        ui.set_name_draft("".into());
    }
    name_status(
        app,
        if kind == NameKind::Create {
            "Playlist created."
        } else {
            "Playlist renamed."
        },
    );
    Some(write)
}
pub fn desired_preferences(state: &UiState) -> serein_storage::LocalPreferences {
    state
        .library_ui
        .desired_preferences(state.preferences.get())
}
fn theme_index(theme: serein_storage::Theme) -> i32 {
    match theme {
        serein_storage::Theme::System => 0,
        serein_storage::Theme::Light => 1,
        serein_storage::Theme::Dark => 2,
    }
}
fn save_preferences(app: &App, state: &UiState, value: serein_storage::LocalPreferences) -> bool {
    if state.caption_cache.active() || !state.playback_preferences.ready() {
        status(
            app,
            "Wait for local settings to finish loading or clearing.",
        );
        return false;
    }
    let id = state.library_ui.preference_serial.get().wrapping_add(1);
    let write = library::PreferenceWrite { id, value };
    if !state.library.submit(library::Request::Preferences(write)) {
        status(app, "The library is busy. The preference was not saved.");
        return false;
    }
    state.library_ui.preference_serial.set(id);
    state.library_ui.requested_preferences.set(Some(write));
    app.set_default_quality_index(value.playback.quality.index());
    true
}
pub fn set_comments_enabled(app: &App, state: &UiState, enabled: bool) -> bool {
    let mut prefs = desired_preferences(state);
    prefs.comments_enabled = enabled;
    save_preferences(app, state, prefs)
}
pub fn set_search_suggestions(app: &App, state: &UiState, enabled: bool) -> bool {
    let mut prefs = desired_preferences(state);
    prefs.search_suggestions = enabled;
    save_preferences(app, state, prefs)
}
pub fn save_quality(app: &App, state: &UiState, quality: serein_core::QualityCeiling) -> bool {
    let mut prefs = desired_preferences(state);
    prefs.playback.quality = quality;
    save_preferences(app, state, prefs)
}
pub fn save_speed(app: &App, state: &UiState, speed: serein_core::PlaybackSpeed) -> bool {
    let mut prefs = desired_preferences(state);
    prefs.playback.speed = speed;
    save_preferences(app, state, prefs)
}

fn thumbnail_cache_mib(index: i32) -> Option<u16> {
    match index {
        0 => Some(0),
        1 => Some(32),
        2 => Some(128),
        3 => Some(256),
        _ => None,
    }
}

fn thumbnail_cache_index(mib: u16) -> Option<i32> {
    match mib {
        0 => Some(0),
        32 => Some(1),
        128 => Some(2),
        256 => Some(3),
        _ => None,
    }
}

/// Apply only a committed or freshly hydrated preference. The thumbnail worker
/// performs disk IO asynchronously; the UI callback merely sends its config.
fn apply_thumbnail_cache(app: &App, state: &UiState, mib: u16) {
    app.set_thumbnail_cache_index(thumbnail_cache_index(mib).unwrap_or(0));
    let result = state.thumbnails.borrow_mut().set_cache_limit(mib);
    if let Err(error) = result {
        status(app, error);
    }
}

/// Called by the existing thumbnail wake after worker IO completes. Ignore a
/// superseded setting's failure; it cannot describe the current saved policy.
pub fn thumbnail_cache_result(
    app: &App,
    state: &UiState,
    mib: u16,
    result: Result<(), &'static str>,
) {
    if state.preferences.get().thumbnail_cache_mib == mib
        && let Err(error) = result
    {
        status(app, error);
    }
}
fn selected(app: &App, s: &UiState) -> Option<LocalPlaylistId> {
    s.playlists
        .borrow()
        .get(app.get_selected_playlist() as usize)
        .map(|p| p.id)
}
fn submit(app: &App, s: &UiState, request: library::Request) -> bool {
    let trace_kind = match &request {
        library::Request::ReadPage {
            page: library::PageQuery::Videos(..) | library::PageQuery::FilteredVideos(..),
            ..
        } => "video-page-queued",
        library::Request::ReadPage {
            page: library::PageQuery::Collections(..),
            ..
        } => "collection-page-queued",
        library::Request::ReadPage {
            page: library::PageQuery::Subscriptions(..),
            ..
        } => "subscriptions-page-queued",
        library::Request::ReadPage {
            page: library::PageQuery::History(..),
            ..
        } => "history-page-queued",
        _ => "other-library-request-queued",
    };
    if s.caption_cache.active() {
        status(app, "Local-data clearing is still in progress.");
        return false;
    }
    if s.library_ui.pending.get() || s.library_ui.pending_save.borrow().is_some() {
        status(app, "The library is busy. Try again shortly.");
        return false;
    }
    if !s.library.submit(request) {
        status(app, "The library queue is full. Try again shortly.");
        return false;
    }
    s.library_ui.pending.set(true);
    crate::fixture_quiescence::trace(s, trace_kind);
    app.global::<LibraryUi>().set_busy(true);
    true
}
/// Explicit local action only. This worker has no account mutation capability.
pub fn follow_channel(app: &App, state: &UiState, id: &ChannelId, name: &str) {
    let Ok(id) = ChannelId::new(id.as_str()) else {
        status(app, "This channel has no supported canonical identity.");
        return;
    };
    if submit(app, state, library::Request::Follow(id, name.to_owned())) {
        status(
            app,
            "Saving a local follow. YouTube subscriptions are unchanged.",
        );
    }
}
fn follow_target(item: &LocalSubscription) -> Result<String, serein_core::ProviderError> {
    Ok(ChannelId::new(item.channel_id.as_str())?.browse_url())
}
fn complete(app: &App, s: &UiState) {
    let outstanding = s.library_ui.pending_save.borrow().is_some()
        || s.library_ui.reads.borrow().pending.is_some()
        || s.library_ui.organization.pending();
    s.library_ui.pending.set(outstanding);
    app.global::<LibraryUi>()
        .set_busy(s.caption_cache.active() || outstanding);
}
fn current_route(app: &App, state: &UiState) -> Route {
    let epoch = state.library_ui.route_epoch.get();
    match app.global::<LibraryUi>().get_tab() {
        1 => Route::Subscriptions(epoch),
        2 => Route::History(epoch),
        _ => selected(app, state).map_or(Route::Empty(epoch), |id| Route::Videos(id, epoch)),
    }
}
fn next_epoch(app: &App, value: &Cell<u64>) -> Option<u64> {
    let next = value.get().checked_add(1);
    if next.is_none() {
        status(
            app,
            "Local navigation is unavailable. Restart the application.",
        );
    }
    next
}
fn read_page(app: &App, state: &UiState, route: Route, page: library::PageQuery) -> bool {
    let Some(ticket) = state.library_ui.reads.borrow_mut().start(route) else {
        status(app, "The local library is busy. Try again shortly.");
        return false;
    };
    if submit(app, state, library::Request::ReadPage { ticket, page }) {
        true
    } else {
        state.library_ui.reads.borrow_mut().finish(ticket);
        false
    }
}

fn save_status(app: &App, message: impl Into<slint::SharedString>) {
    let message = message.into();
    app.global::<SaveUi>().set_status(message.clone());
    status(app, message);
}

fn current_savable_video(
    app: &App,
    state: &UiState,
) -> Option<(VideoSummary, serein_media::Snapshot)> {
    if !app.get_remote_video()
        || !app.get_loaded()
        || app.get_busy()
        || app.get_native_video_child()
        || app.get_account_playback_active()
        || crate::account_playback::authorization(state).is_some()
        || state.caption_cache.active()
        || state.library_fixture.is_some()
    {
        return None;
    }
    let video = state.current_video.borrow().clone()?;
    let snapshot = state.player.snapshot();
    savable_load(&snapshot).then_some((video, snapshot))
}

fn begin_save(app: &App, state: &UiState) -> bool {
    if state.library_ui.pending_save.borrow().is_some() {
        return true;
    }
    if state.library_ui.pending.get() {
        status(app, "The local library is busy. Try Save again shortly.");
        return false;
    }
    let Some((video, snapshot)) = current_savable_video(app, state) else {
        status(
            app,
            "Save is available for the current guest YouTube video after playback starts.",
        );
        return false;
    };
    let collections = state.playlists.borrow().clone();
    let ui = app.global::<SaveUi>();
    ui.set_collections(slint::ModelRc::new(slint::VecModel::from(
        collections
            .iter()
            .map(|item| item.name.clone().into())
            .collect::<Vec<slint::SharedString>>(),
    )));
    ui.set_video_title(video.title.clone().into());
    ui.set_busy(false);
    ui.set_saved(false);
    ui.set_destinations_paged(
        app.global::<LibraryUi>().get_collections_previous()
            || app.global::<LibraryUi>().get_collections_next(),
    );
    ui.set_status("Choose a playlist or create one below.".into());
    *state.library_ui.save_target.borrow_mut() = Some(SaveTarget {
        video,
        load: snapshot.load_request_id,
        collections,
    });
    true
}

fn enqueue_video_save(app: &App, state: &UiState, existing: Option<i32>, name: Option<String>) {
    if state.library_ui.pending_save.borrow().is_some() || app.global::<SaveUi>().get_saved() {
        return;
    }
    let target = state.library_ui.save_target.borrow().clone();
    let current = current_savable_video(app, state);
    let Some((target, (_, snapshot))) = target
        .zip(current)
        .filter(|(target, (video, snapshot))| target.matches(video, snapshot))
    else {
        save_status(
            app,
            "Playback changed or is unavailable. Close this dialog and choose Save again.",
        );
        return;
    };
    // The popup owns this bounded destination snapshot. A refreshed library
    // model must not redirect its selected index to a different collection.
    let (destination, collection_name) = if let Some(index) = existing {
        let Some(collection) = usize::try_from(index)
            .ok()
            .and_then(|index| target.collections.get(index))
        else {
            save_status(app, "Choose a local playlist before saving.");
            return;
        };
        (
            library::SaveDestination::Existing(collection.id),
            collection.name.clone(),
        )
    } else {
        let name = match playlist_name(&name.unwrap_or_default()) {
            Ok(name) => name,
            Err(error) => {
                save_status(app, error);
                return;
            }
        };
        (library::SaveDestination::New(name.clone()), name)
    };
    let Some(serial) = state.library_ui.save_serial.get().checked_add(1) else {
        save_status(
            app,
            "Saving is unavailable. Restart the application and try again.",
        );
        return;
    };
    // The native identity above is checked immediately before this bounded
    // worker admission. Once accepted, the explicit write completes even if
    // playback changes or the popup closes; its response retains this serial.
    debug_assert_eq!(target.load, snapshot.load_request_id);
    if submit(
        app,
        state,
        library::Request::Save(library::VideoSave {
            serial,
            destination,
            video: target.video,
        }),
    ) {
        state.library_ui.save_serial.set(serial);
        *state.library_ui.pending_save.borrow_mut() = Some(PendingSave {
            serial,
            collection_name,
        });
        app.global::<SaveUi>().set_busy(true);
        save_status(app, "Saving on this device…");
    } else {
        app.global::<SaveUi>()
            .set_status("The local library is busy. Nothing was queued; try again shortly.".into());
    }
}
pub fn clear_after_caption_purge(app: &App, s: &UiState) -> bool {
    if !s.library.submit(library::Request::ClearLocalData) {
        return false;
    }
    s.library_ui.pending.set(true);
    app.global::<LibraryUi>().set_busy(true);
    true
}
fn unix_seconds(time: SystemTime) -> i64 {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .and_then(|value| i64::try_from(value.as_secs()).ok())
        .unwrap_or(0)
}
fn clock(seconds: u64) -> String {
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
fn history_row(entry: &HistoryEntry, heading: String) -> LibraryRow {
    let position = entry.position.as_secs();
    let duration = entry
        .video
        .duration
        .map(|value| value.as_secs())
        .filter(|value| *value > 0);
    let detail = match duration {
        Some(duration) if position >= duration => "Watched".into(),
        Some(duration) => format!("Watched {} of {}", clock(position), clock(duration)),
        None if position > 0 => format!("Watched {}", clock(position)),
        None => String::new(),
    };
    LibraryRow {
        title: entry.video.title.clone().into(),
        channel: entry.video.channel.clone().into(),
        detail: detail.into(),
        day_heading: heading.into(),
        duration: duration.map(clock).unwrap_or_default().into(),
        progress: duration.map_or(0.0, |duration| {
            (position as f64 / duration as f64).min(1.0) as f32
        }),
        action: "Remove from history".into(),
        ..LibraryRow::default()
    }
}
/// A cache miss stays offline; local history never synthesizes a CDN URL.
pub fn thumbnail_source(state: &UiState, index: usize) -> Option<crate::thumbnails::Source> {
    let items = state.library_ui.items.borrow();
    let Item::History(entry) = items.get(index)? else {
        return None;
    };
    (!state.library_ui.rows.row_data(index)?.thumbnail_ready)
        .then(|| crate::thumbnails::Source::CachedVideo(entry.video.id.clone()))
}
pub fn publish_thumbnail(
    state: &UiState,
    index: usize,
    id: &serein_core::VideoId,
    image: slint::Image,
) -> bool {
    let matches = matches!(state.library_ui.items.borrow().get(index), Some(Item::History(entry)) if &entry.video.id == id);
    if !matches {
        return false;
    }
    let Some(mut row) = state.library_ui.rows.row_data(index) else {
        return false;
    };
    row.thumbnail = image;
    row.thumbnail_ready = true;
    state.library_ui.rows.set_row_data(index, row);
    state
        .library_ui
        .row_changes
        .set(state.library_ui.row_changes.get() + 1);
    true
}
pub fn release_thumbnails(state: &UiState, keep: std::ops::Range<usize>) {
    for index in 0..state.library_ui.rows.row_count() {
        if !keep.contains(&index)
            && let Some(mut row) = state.library_ui.rows.row_data(index)
            && row.thumbnail_ready
        {
            row.thumbnail = slint::Image::default();
            row.thumbnail_ready = false;
            state.library_ui.rows.set_row_data(index, row);
            state
                .library_ui
                .row_changes
                .set(state.library_ui.row_changes.get() + 1);
        }
    }
}
pub fn thumbnail_count(state: &UiState) -> usize {
    state.library_ui.rows.row_count()
}
/// Determine actual visible indices from the bounded variable-height row page.
/// Group headings are inside their video row, so callbacks retain video indices.
pub fn thumbnail_viewport(
    state: &UiState,
    offset: f32,
    height: f32,
    narrow: bool,
) -> (usize, usize) {
    let top = offset.max(0.0);
    let bottom = top + height.max(0.0);
    let count = state.library_ui.rows.row_count();
    let mut first = count;
    let mut end = count;
    let mut y = 0.0;
    for index in 0..count {
        let Some(row) = state.library_ui.rows.row_data(index) else {
            break;
        };
        let row_height = (if narrow { 92.0 } else { 112.0 })
            + if row.day_heading.is_empty() {
                0.0
            } else {
                36.0
            };
        if y + row_height > top && first == count {
            first = index;
        }
        if y >= bottom {
            end = index;
            break;
        }
        y += row_height;
    }
    (first, end.max(first))
}
fn publish(app: &App, s: &UiState, items: Vec<Item>, next: Option<Cursor>) -> bool {
    crate::fixture_quiescence::trace(
        s,
        if items.is_empty() {
            "publish-empty-library-page"
        } else {
            "publish-library-page"
        },
    );
    let now = unix_seconds(SystemTime::now());
    let mut previous_day = None;
    let rows: Vec<LibraryRow> = items
        .iter()
        .map(|item| match item {
            Item::Video(v) => LibraryRow {
                title: v.title.clone().into(),
                detail: v.channel.clone().into(),
                action: "Remove".into(),
                ..LibraryRow::default()
            },
            Item::Follow(v) => LibraryRow {
                title: v.name.clone().into(),
                detail: "Local follow · Open public channel".into(),
                action: "Unfollow".into(),
                ..LibraryRow::default()
            },
            Item::History(v) => {
                let day = crate::display_format::history_day(unix_seconds(v.watched_at), now);
                let heading = if previous_day.as_ref() == Some(&day) {
                    String::new()
                } else {
                    day.clone()
                };
                previous_day = Some(day);
                let mut row = history_row(v, heading);
                // A same-page deletion/metadata change must not discard cached
                // artwork held by surviving row identities.
                if s.library_ui.published_route.borrow().as_ref() == Some(&current_route(app, s)) {
                    let previous = s
                        .library_ui
                        .items
                        .borrow()
                        .iter()
                        .position(|old| old.key() == item.key());
                    if let Some(old) = previous.and_then(|index| s.library_ui.rows.row_data(index))
                    {
                        row.thumbnail = old.thumbnail;
                        row.thumbnail_ready = old.thumbnail_ready;
                    }
                }
                row
            }
        })
        .collect();
    let route = current_route(app, s);
    let old_keys: Vec<_> = s.library_ui.items.borrow().iter().map(Item::key).collect();
    let keyed: Vec<_> = items.iter().map(Item::key).zip(rows).collect();
    let same_page = s.library_ui.published_route.borrow().as_ref() == Some(&route);
    let changes = match library_model::reconcile(&s.library_ui.rows, &old_keys, &keyed, same_page) {
        Ok(changes) => changes,
        Err(error) => {
            status(app, error);
            return false;
        }
    };
    s.library_ui
        .row_resets
        .set(s.library_ui.row_resets.get() + u64::from(changes.reset));
    s.library_ui.row_changes.set(
        s.library_ui.row_changes.get()
            + (changes.changed + changes.inserted + changes.removed) as u64,
    );
    *s.library_ui.published_route.borrow_mut() = Some(route);
    *s.library_ui.items.borrow_mut() = items;
    let (has_next, has_previous) = {
        let mut pages = s.library_ui.pages.borrow_mut();
        pages.next = next;
        (pages.next.is_some(), !pages.previous.is_empty())
    };
    let ui = app.global::<LibraryUi>();
    ui.set_next(has_next);
    ui.set_previous(has_previous);
    if let Some(videos) = fixture_videos(s) {
        crate::library_fixture::publish(app, s, &videos);
    }
    if app.get_page() == 1 && ui.get_tab() == 2 {
        // A deletion/page change may preserve the numeric viewport while its
        // row identities change. Cancel/re-admit cache-only work immediately.
        s.thumbnail_attempted.borrow_mut().clear();
        s.thumbnail_range.set((usize::MAX, usize::MAX));
        app.invoke_refresh_visible();
    }
    true
}
fn refresh(app: &App, s: &UiState) -> bool {
    let route = current_route(app, s);
    let page = match &route {
        Route::Subscriptions(_) => {
            library::PageQuery::Subscriptions(s.library_ui.pages.borrow().page())
        }
        Route::History(_) => library::PageQuery::History(s.library_ui.pages.borrow().history()),
        Route::Videos(id, _) => video_query(
            *id,
            s.library_ui.pages.borrow().page(),
            &s.library_ui.filter.borrow(),
        ),
        _ => {
            publish(app, s, Vec::new(), None);
            return false;
        }
    };
    read_page(app, s, route, page)
}
fn video_query(id: LocalPlaylistId, after: Option<PageCursor>, filter: &str) -> library::PageQuery {
    if filter.is_empty() {
        library::PageQuery::Videos(id, after)
    } else {
        library::PageQuery::FilteredVideos(id, after, filter.to_owned())
    }
}
fn reset_filter(app: &App, state: &UiState) {
    state.library_ui.filter.borrow_mut().clear();
    state.library_ui.pending_filter.borrow_mut().take();
    let ui = app.global::<LibraryUi>();
    ui.set_filter_draft("".into());
    ui.set_active_filter("".into());
}
fn apply_filter(app: &App, state: &UiState, draft: &str) {
    let ui = app.global::<LibraryUi>();
    if app.get_page() != 1
        || ui.get_tab() != 0
        || ui.get_confirmation() != 0
        || state.library_ui.pending.get()
    {
        return;
    }
    let Some(id) = selected(app, state) else {
        return;
    };
    if draft.len() > serein_storage::MAX_PLAYLIST_FILTER_BYTES
        || draft.chars().any(char::is_control)
    {
        status(
            app,
            "Use a shorter playlist search without control characters (up to 256 bytes).",
        );
        return;
    }
    let filter = draft.trim();
    let Some(epoch) = next_epoch(app, &state.library_ui.route_epoch) else {
        return;
    };
    let origin = current_route(app, state);
    if !read_page(
        app,
        state,
        Route::Videos(id, epoch),
        video_query(id, None, filter),
    ) {
        return;
    }
    let ticket = state
        .library_ui
        .reads
        .borrow()
        .pending
        .as_ref()
        .map(|(ticket, _)| *ticket);
    if let Some(ticket) = ticket {
        *state.library_ui.pending_filter.borrow_mut() = Some(PendingFilter {
            ticket,
            origin,
            filter: filter.to_owned(),
        });
    }
}
/// Finite diagnostics request the same bounded production refresh and track
/// its terminal ticket independently of model invalidation counts.
pub fn refresh_current_page(app: &App, state: &UiState) -> Option<u64> {
    if !refresh(app, state) {
        return None;
    }
    state
        .library_ui
        .reads
        .borrow()
        .pending
        .as_ref()
        .map(|(ticket, _)| *ticket)
}
pub fn accepted_page_read(state: &UiState) -> Option<u64> {
    state.library_ui.last_page_read.get()
}
fn collections(app: &App, s: &UiState) {
    let window = s.library_ui.collections.borrow().current;
    read_page(
        app,
        s,
        Route::Collections(s.library_ui.collection_epoch.get()),
        library::PageQuery::Collections(window),
    );
}
fn create_context(app: &App, state: &UiState) -> CreateContext {
    CreateContext {
        route_epoch: state.library_ui.route_epoch.get(),
        collection_epoch: state.library_ui.collection_epoch.get(),
        visible: app.get_page() == 1 && app.global::<LibraryUi>().get_tab() == 0,
    }
}
fn context_matches(app: &App, state: &UiState, expected: CreateContext) -> bool {
    let current = create_context(app, state);
    expected.matches(
        current.route_epoch,
        current.collection_epoch,
        current.visible,
    )
}
fn take_created_selection(state: &State, ticket: u64) -> Option<CreatedSelection> {
    let mut pending = state.created_selection.borrow_mut();
    if pending
        .as_ref()
        .is_some_and(|selection| selection.ticket == ticket)
    {
        pending.take()
    } else {
        None
    }
}
/// Admit the read before changing window state. The old model and selected ID
/// stay actionable only after success/failure releases the existing busy gate.
fn reveal_created(app: &App, state: &UiState, id: LocalPlaylistId) -> bool {
    let Some(epoch) = next_epoch(app, &state.library_ui.collection_epoch) else {
        return false;
    };
    let original = state.library_ui.collections.borrow().clone();
    if !read_page(
        app,
        state,
        Route::Collections(epoch),
        library::PageQuery::Collections(PlaylistWindow::EndingAt(id)),
    ) {
        return false;
    }
    *state.library_ui.collection_origin.borrow_mut() =
        Some((state.library_ui.collection_epoch.get(), original));
    state.library_ui.collection_epoch.set(epoch);
    state.library_ui.collections.borrow_mut().current = PlaylistWindow::EndingAt(id);
    let ticket = state
        .library_ui
        .reads
        .borrow()
        .pending
        .as_ref()
        .expect("admitted collection read")
        .0;
    *state.library_ui.created_selection.borrow_mut() = Some(CreatedSelection {
        ticket,
        id,
        context: create_context(app, state),
    });
    true
}
fn restore_collection_page(state: &State, origin: Option<(u64, CollectionWindow)>) {
    if let Some((epoch, page)) = origin {
        state.collection_epoch.set(epoch);
        *state.collections.borrow_mut() = page;
    }
}
fn collection_selection(
    items: &[serein_storage::LocalPlaylist],
    prefer: Option<LocalPlaylistId>,
) -> i32 {
    prefer
        .and_then(|id| items.iter().position(|item| item.id == id))
        .map(|index| index as i32)
        .unwrap_or(if items.is_empty() { -1 } else { 0 })
}
fn publish_collections(
    app: &App,
    s: &UiState,
    items: Vec<serein_storage::LocalPlaylist>,
    prefer: Option<LocalPlaylistId>,
) -> bool {
    crate::fixture_quiescence::trace(s, "publish-collections");
    let previous = selected(app, s);
    let prefer = prefer.or(previous);
    let index = collection_selection(&items, prefer);
    let changed_selection = previous != items.get(index as usize).map(|item| item.id);
    let cancel_edit = changed_selection && s.library_ui.rename_target.borrow().is_some();
    if cancel_edit {
        cancel_name(app, s);
    }
    let next_route = if changed_selection && app.global::<LibraryUi>().get_tab() == 0 {
        let Some(epoch) = next_epoch(app, &s.library_ui.route_epoch) else {
            return false;
        };
        Some(epoch)
    } else {
        None
    };
    let old_keys: Vec<_> = s.playlists.borrow().iter().map(|item| item.id).collect();
    let names: Vec<_> = items
        .iter()
        .map(|item| (item.id, slint::SharedString::from(item.name.as_str())))
        .collect();
    let epoch = s.library_ui.collection_epoch.get();
    let changes = match library_model::reconcile(
        &s.library_ui.collection_names,
        &old_keys,
        &names,
        s.library_ui.published_collection_epoch.get() == Some(epoch),
    ) {
        Ok(changes) => changes,
        Err(error) => {
            status(app, error);
            return false;
        }
    };
    s.library_ui.published_collection_epoch.set(Some(epoch));
    s.library_ui
        .collection_replacements
        .set(s.library_ui.collection_replacements.get() + u64::from(changes.reset));
    s.library_ui.collection_changes.set(
        s.library_ui.collection_changes.get()
            + (changes.changed + changes.inserted + changes.removed) as u64,
    );
    *s.playlists.borrow_mut() = items;
    if let Some(epoch) = next_route {
        s.library_ui.route_epoch.set(epoch);
        *s.library_ui.pages.borrow_mut() = Pagination::default();
        reset_filter(app, s);
    }
    app.set_selected_playlist(index);
    app.global::<LibraryUi>().set_selected(index);
    if next_route.is_some() {
        // A subsequent video read can fail. Retire the old playlist's rows now,
        // before that failure could make them actionable under a new identity.
        publish(app, s, Vec::new(), None);
    }
    true
}
/// Call on media position notifications; no timer is created for history.
/// The stored opt-in is rechecked inside SQLite, and writes are coalesced to 30s.
pub fn record_playback(s: &UiState, video: &VideoSummary, position: Duration) {
    if s.caption_cache.active()
        || crate::account_playback::authorization(s).is_some()
        || !s.preferences.get().privacy.local_history
    {
        return;
    }
    let bucket = position.as_secs() / 30;
    if s.library_ui
        .last_record
        .borrow()
        .as_ref()
        .is_some_and(|(id, last)| id == &video.id && *last == bucket)
    {
        return;
    }
    if s.library
        .submit(library::Request::RecordHistory(video.clone(), position))
    {
        *s.library_ui.last_record.borrow_mut() = Some((video.id.clone(), bucket));
    }
}
pub fn bind(app: &App, state: &Rc<UiState>) {
    organization::bind(app, state);
    let names = slint::ModelRc::from(state.library_ui.collection_names.clone());
    app.set_playlists(names.clone());
    app.global::<LibraryUi>().set_collections(names);
    app.global::<LibraryUi>()
        .set_rows(slint::ModelRc::from(state.library_ui.rows.clone()));
    let weak = app.as_weak();
    let s = state.clone();
    app.on_library_wake(move || {
        let Some(app) = weak.upgrade() else { return };
        while let Some(response) = s.library.take() {
            match response {
                library::Response::Organization { ticket, result } => organization::receive(&app, &s, ticket, result),
                library::Response::Home { ticket, result } => crate::home_ui::receive(&app, &s, ticket, result),
                library::Response::Library(items, prefs, chosen) => {
                    crate::home_ui::changed(&app, &s);
                    complete(&app, &s);
                    if s.library_ui.reads.borrow().pending.is_none() {
                        let was_later_page=s.library_ui.collections.borrow().current != PlaylistWindow::First;
                        if was_later_page {
                            let Some(epoch)=next_epoch(&app,&s.library_ui.collection_epoch) else { continue; };
                            s.library_ui.collection_epoch.set(epoch);
                        }
                        *s.library_ui.collections.borrow_mut()=CollectionWindow::default();
                        publish_collections(&app, &s, items, chosen);
                    }
                    if !s.playback_preferences.ready() {
                      s.preferences.set(prefs);
                    crate::comments_ui::sync_preferences(&app, &s);
                    crate::search_suggestions::sync_preferences(&app, &s);
                      app.set_theme(match prefs.theme {
                        serein_storage::Theme::System => 0,
                        serein_storage::Theme::Light => 1,
                        serein_storage::Theme::Dark => 2,
                      });
                      app.set_volume_level(prefs.volume_percent as f32);
                      let _ = s.player.set_volume(prefs.volume_percent as f64);
                      app.global::<LibraryUi>()
                        .set_history_enabled(prefs.privacy.local_history);
                      crate::playback_preferences::hydrate(&app, &s, prefs.playback);
                      apply_thumbnail_cache(&app, &s, prefs.thumbnail_cache_mib);
                    }
                    // Startup summary is followed by a correlated page.
                    if s.library_ui.reads.borrow().pending.is_none() {
                        collections(&app, &s);
                    }
                }
                library::Response::Created(id) => {
                    crate::home_ui::changed(&app, &s);
                    let write = finish_name(&app, &s, NameKind::Create);
                    complete(&app, &s);
                    if s.caption_cache.active() { continue; }
                    let reveal = write.and_then(|write| write.create_context)
                        .is_some_and(|context| context_matches(&app, &s, context));
                    if reveal {
                        if !reveal_created(&app, &s, id) {
                            status(&app, "Playlist created, but its page could not be loaded. Reopen local playlists.");
                        }
                    } else {
                        // Keep the current selection/window when a committed
                        // creation finishes after its author left the editor.
                        collections(&app, &s);
                    }
                }
                library::Response::PageRead { ticket, result } => {
                    // An old success OR failure cannot complete a newer request.
                    let route = s.library_ui.reads.borrow_mut().finish(ticket);
                    let Some(route) = route else { continue; };
                    let filter_change = if s.library_ui.pending_filter.borrow().as_ref().is_some_and(|pending| pending.ticket == ticket) {
                        s.library_ui.pending_filter.borrow_mut().take()
                    } else { None };
                    let collection_origin=if matches!(route,Route::Collections(_)) {
                        s.library_ui.collection_origin.borrow_mut().take()
                    } else { None };
                    let created = take_created_selection(&s.library_ui, ticket);
                    complete(&app,&s);
                    let current = if matches!(route,Route::Collections(_)) {
                        Route::Collections(s.library_ui.collection_epoch.get())
                    } else if let Some(change) = &filter_change {
                        if change.origin != current_route(&app,&s) || app.get_page() != 1 {
                            continue;
                        }
                        route.clone()
                    } else { current_route(&app,&s) };
                    if route != current || s.caption_cache.active() { continue; }
                    if created.as_ref().is_some_and(|selection| !context_matches(&app, &s, selection.context)) {
                        restore_collection_page(&s.library_ui, collection_origin);
                        collections(&app, &s);
                        continue;
                    }
                    let result = match result { Ok(page) if route.accepts(&page) => page,
                        Ok(_) => { restore_collection_page(&s.library_ui,collection_origin); status(&app,"Unexpected local-library page. Please reopen this view."); continue; },
                        Err(error) => { restore_collection_page(&s.library_ui,collection_origin); status(&app,if created.is_some() { format!("Playlist created, but its page could not be loaded. {error}") } else { error }); continue; } };
                    if let Some(change) = filter_change {
                        let Route::Videos(_, epoch) = route else { continue };
                        s.library_ui.route_epoch.set(epoch);
                        cancel_name(&app, &s);
                        *s.library_ui.pages.borrow_mut() = Pagination::default();
                        let ui = app.global::<LibraryUi>();
                        ui.set_active_filter(change.filter.as_str().into());
                        ui.set_filter_draft(change.filter.as_str().into());
                        *s.library_ui.filter.borrow_mut() = change.filter;
                        status(&app, "Playlist search updated. Only saved titles and channel names on this device are searched.");
                    }
                    let published = match result {
                        library::PageResult::Collections(page) => {
                            let prefer = created.map(|selection| selection.id);
                            if prefer.is_some_and(|id| !page.items.iter().any(|item| item.id == id)) {
                                restore_collection_page(&s.library_ui,collection_origin);
                                status(&app,"Playlist was created, but its page no longer contains it. Reopen local playlists.");
                                continue;
                            }
                            if !publish_collections(&app,&s,page.items,prefer) { restore_collection_page(&s.library_ui,collection_origin); continue; }
                            let (has_next,has_previous)={
                                let mut pages=s.library_ui.collections.borrow_mut();
                                pages.current=page.current;
                                pages.next=page.next;
                                pages.previous=page.previous;
                                (pages.next.is_some(),pages.previous.is_some())
                            };
                            let ui=app.global::<LibraryUi>();
                            ui.set_collections_next(has_next); ui.set_collections_previous(has_previous);
                            if app.get_page()==1 { refresh(&app,&s); }
                            true
                        }
                        library::PageResult::Videos(_,page) => publish(&app,&s,page.items.into_iter().map(Item::Video).collect(),page.next.map(Cursor::Page)),
                        library::PageResult::Subscriptions(page) => publish(&app,&s,page.items.into_iter().map(Item::Follow).collect(),page.next.map(Cursor::Page)),
                        library::PageResult::History {entries,next,retention_days} => {
                            app.global::<LibraryUi>().set_retention(retention_days as i32);
                            publish(&app,&s,entries.into_iter().map(Item::History).collect(),next.map(Cursor::History))
                        }
                    };
                    if published { s.library_ui.last_page_read.set(Some(ticket)); }
                }
                library::Response::HistoryRecorded(_) | library::Response::SearchHistoryWritten => {}
                library::Response::SearchHistory { ticket, result } => {
                    crate::search_suggestions::loaded(&app, &s, ticket, result);
                }
                library::Response::BackgroundError(error) => status(&app, error),
                library::Response::PreferencesFailed(write, error) => {
                    let latest = s.library_ui.finish_preferences(write);
                    crate::comments_ui::sync_preferences(&app, &s);
                    crate::search_suggestions::sync_preferences(&app, &s);
                    app.set_thumbnail_cache_index(thumbnail_cache_index(s.preferences.get().thumbnail_cache_mib).unwrap_or(0));
                    if latest {
                        crate::playback_preferences::save_failed(&app, &s, write.value.playback);
                    }
                    if latest && write.value.theme != s.preferences.get().theme {
                        app.set_theme(theme_index(desired_preferences(&s).theme));
                    }
                    app.set_default_quality_index(desired_preferences(&s).playback.quality.index());
                    app.global::<LibraryUi>()
                        .set_history_enabled(s.preferences.get().privacy.local_history);
                    status(&app, if latest && write.value.playback != s.preferences.get().playback {
                        format!("{error} Playback defaults were not saved; previous defaults remain in use.")
                    } else { error });
                }
                library::Response::PreferencesSaved(write) => {
                    s.library_ui.finish_preferences(write);
                    let prefs = write.value;
                    let history_changed = s.preferences.get().privacy.local_history != prefs.privacy.local_history;
                    let thumbnail_changed = s.preferences.get().thumbnail_cache_mib != prefs.thumbnail_cache_mib;
                    let theme_changed = s.preferences.get().theme != prefs.theme;
                    s.preferences.set(prefs);
                    crate::comments_ui::sync_preferences(&app, &s);
                    crate::search_suggestions::sync_preferences(&app, &s);
                    app.set_thumbnail_cache_index(thumbnail_cache_index(prefs.thumbnail_cache_mib).unwrap_or(0));
                    // Keep newer admitted previews; unrelated writes must not
                    // override a session-only diagnostic appearance.
                    if theme_changed { app.set_theme(theme_index(desired_preferences(&s).theme)); }
                    app.set_default_quality_index(desired_preferences(&s).playback.quality.index());
                    app.global::<LibraryUi>()
                        .set_history_enabled(prefs.privacy.local_history);
                    status(
                        &app,
                        if !history_changed {
                            "Preferences saved on this device."
                        } else if prefs.privacy.local_history {
                            "Preferences saved. Local history is enabled."
                        } else {
                            "Preferences saved. Local history is off."
                        },
                    );
                    if thumbnail_changed { apply_thumbnail_cache(&app, &s, prefs.thumbnail_cache_mib); }
                    if history_changed && app.get_page() == 1 && app.global::<LibraryUi>().get_tab() == 2 {
                        if prefs.privacy.local_history {
                            refresh(&app, &s);
                        } else {
                            // SQLite committed the disable-and-clear operation;
                            // remove its corresponding UI metadata/artwork too.
                            publish(&app, &s, Vec::new(), None);
                        }
                    }
                }
                library::Response::Saved => {
                    crate::home_ui::changed(&app, &s);
                    let _ = finish_name(&app,&s,NameKind::Rename);
                    complete(&app, &s);
                    status(&app, "Saved on this device.");
                    if app.global::<LibraryUi>().get_tab() == 0 {
                        collections(&app, &s);
                    } else {
                        refresh(&app, &s);
                    }
                }
                library::Response::VideoSaved(serial) => {
                    crate::home_ui::changed(&app, &s);
                    if let Some(saved) = s.library_ui.finish_video_save(serial) {
                        complete(&app, &s);
                        app.global::<SaveUi>().set_busy(false);
                        app.global::<SaveUi>().set_saved(true);
                        app.global::<SaveUi>().set_status("Saved on this device.".into());
                        status(&app, format!("Saved to “{}” on this device.", saved.collection_name));
                        // Refresh the same bounded collection page only after
                        // SQLite has committed; no optimistic empty playlist.
                        collections(&app, &s);
                    }
                }
                library::Response::VideoSaveFailed(serial, error) => {
                    if s.library_ui.finish_video_save(serial).is_some() {
                        complete(&app, &s);
                        app.global::<SaveUi>().set_busy(false);
                        app.global::<SaveUi>().set_saved(false);
                        save_status(&app, format!("{error} Nothing was saved. You can try again."));
                    }
                }
                library::Response::Error(error) => {
                    if s.library_ui.name_write.borrow_mut().take().is_some() {
                        app.global::<LibraryUi>().set_name_status(error.clone().into());
                    }
                    complete(&app, &s);
                    if !s.playback_preferences.ready() {
                        crate::playback_preferences::hydrate(&app, &s, Default::default());
                        status(&app, format!("{error} Using default playback settings for this session."));
                    } else {
                        status(&app, error);
                    }
                }
                library::Response::ClearFailed(error) => {
                    complete(&app, &s);
                    if !crate::caption_cache::library_finished(&app, &s, Err(&error)) {
                        status(&app, error);
                    }
                }
                library::Response::Imported(summary) => {
                    crate::home_ui::changed(&app, &s);
                    complete(&app, &s);
                    status(
                        &app,
                        format!(
                            "Imported {} collections, {} videos and {} follows.",
                            summary.playlists_created,
                            summary.videos_saved,
                            summary.channels_followed
                        ),
                    );
                    *s.library_ui.collections.borrow_mut() = CollectionWindow::default();
                    if let Some(epoch)=next_epoch(&app,&s.library_ui.collection_epoch) { s.library_ui.collection_epoch.set(epoch); }
                    collections(&app, &s);
                }
                library::Response::Exported => {
                    complete(&app, &s);
                    status(
                        &app,
                        "Local library exported. Account credentials are excluded.",
                    );
                }
                library::Response::BackedUp => {
                    complete(&app, &s);
                    status(
                        &app,
                        "Local library backup created. Account credentials are excluded.",
                    );
                }
                library::Response::Cleared(prefs) => {
                    crate::guest_ui::clear_cached_catalog(&app, &s);
                    crate::home_ui::cleared(&app, &s);
                    cancel_name(&app,&s);
                    reset_filter(&app, &s);
                    organization::reset(&app, &s);
                    s.library_ui.name_write.borrow_mut().take();
                    app.global::<LibraryUi>().set_name_draft("".into());
                    // Install the committed defaults before reopening admission:
                    // a theme/volume event must not restore an old history opt-in.
                    s.preferences.set(prefs);
                    s.library_ui.requested_preferences.set(None);
                    crate::comments_ui::sync_preferences(&app, &s);
                    crate::search_suggestions::local_data_cleared(&app, &s);
                    s.library_ui.last_record.borrow_mut().take();
                    s.library_ui.save_target.borrow_mut().take();
                    app.global::<SaveUi>().set_video_title("".into());
                    app.global::<SaveUi>().set_collections(slint::ModelRc::default());
                    app.global::<SaveUi>().set_saved(false);
                    app.global::<SaveUi>().set_status("Local data was cleared. Close this dialog.".into());
                    crate::playback_preferences::reset(&app, &s, prefs.playback);
                    app.set_theme(0);
                    app.set_volume_level(prefs.volume_percent as f32);
                    let _ = s.player.set_volume(prefs.volume_percent as f64);
                    app.global::<LibraryUi>()
                        .set_history_enabled(prefs.privacy.local_history);
                    *s.library_ui.collections.borrow_mut() = CollectionWindow::default();
                    *s.library_ui.pages.borrow_mut() = Pagination::default();
                    s.library_ui.reads.borrow_mut().pending=None;
                    s.library_ui.last_page_read.set(None);
                    s.library_ui.collection_origin.borrow_mut().take();
                    s.library_ui.created_selection.borrow_mut().take();
                    complete(&app, &s);
                    if let Some(epoch)=next_epoch(&app,&s.library_ui.collection_epoch) { s.library_ui.collection_epoch.set(epoch); }
                    if let Some(epoch)=next_epoch(&app,&s.library_ui.route_epoch) { s.library_ui.route_epoch.set(epoch); }
                    app.global::<LibraryUi>().set_collections_next(false);
                    app.global::<LibraryUi>().set_collections_previous(false);
                    app.global::<LibraryUi>().set_retention(30);
                    publish_collections(&app, &s, Vec::new(), None);
                    publish(&app, &s, Vec::new(), None);
                    crate::caption_cache::library_finished(&app, &s, Ok(()));
                    apply_thumbnail_cache(&app, &s, prefs.thumbnail_cache_mib);
                }
            }
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.global::<LibraryUi>().on_change_tab(move |tab| {
        let Some(app) = weak.upgrade() else { return };
        open_tab(&app, &s, tab);
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.global::<LibraryUi>().on_filter(move |draft| {
        if let Some(app) = weak.upgrade() {
            apply_filter(&app, &s, &draft);
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_choose_playlist(move |index| {
        let Some(app) = weak.upgrade() else { return };
        if s.library_ui.pending.get() || app.global::<LibraryUi>().get_tab() != 0 {
            return;
        }
        let id = s.playlists.borrow().get(index as usize).map(|item| item.id);
        let Some(id) = id else {
            return;
        };
        if selected(&app, &s) == Some(id) {
            return;
        }
        let Some(epoch) = next_epoch(&app, &s.library_ui.route_epoch) else {
            return;
        };
        // Queue admission precedes navigation. A full worker queue must not
        // leave a new cursor/selection attached to the old displayed rows.
        if !read_page(
            &app,
            &s,
            Route::Videos(id, epoch),
            library::PageQuery::Videos(id, None),
        ) {
            return;
        }
        s.library_ui.route_epoch.set(epoch);
        cancel_name(&app, &s);
        reset_filter(&app, &s);
        organization::reset(&app, &s);
        app.set_selected_playlist(index);
        app.global::<LibraryUi>().set_selected(index);
        *s.library_ui.pages.borrow_mut() = Pagination::default();
        publish(&app, &s, Vec::new(), None);
    });
    let weak = app.as_weak();
    app.global::<LibraryUi>().on_choose(move |i| {
        if let Some(app) = weak.upgrade() {
            app.invoke_choose_playlist(i);
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.global::<LibraryUi>().on_page(move |next| {
        let Some(app) = weak.upgrade() else { return };
        if !s.library_ui.pending.get() {
            let Some(epoch) = next_epoch(&app, &s.library_ui.route_epoch) else {
                return;
            };
            let mut page = s.library_ui.pages.borrow().clone();
            if !page.advance(next) {
                return;
            }
            let (route, query) = match current_route(&app, &s) {
                Route::Videos(id, _) => (
                    Route::Videos(id, epoch),
                    video_query(id, page.page(), &s.library_ui.filter.borrow()),
                ),
                Route::Subscriptions(_) => (
                    Route::Subscriptions(epoch),
                    library::PageQuery::Subscriptions(page.page()),
                ),
                Route::History(_) => (
                    Route::History(epoch),
                    library::PageQuery::History(page.history()),
                ),
                _ => return,
            };
            if !read_page(&app, &s, route, query) {
                return;
            }
            s.library_ui.route_epoch.set(epoch);
            // Page navigation retires the captured rename epoch. Retire its
            // visible editor at the same time, rather than leaving a Save
            // control whose stale target can only fail and discard the edit.
            cancel_name(&app, &s);
            *s.library_ui.pages.borrow_mut() = page;
            publish(&app, &s, Vec::new(), None);
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.global::<LibraryUi>().on_collection_page(move |next| {
        let Some(app) = weak.upgrade() else { return };
        if !s.library_ui.pending.get() {
            let Some(epoch) = next_epoch(&app, &s.library_ui.collection_epoch) else {
                return;
            };
            let original = s.library_ui.collections.borrow().clone();
            let mut page = original.clone();
            if !page.advance(next) {
                return;
            }
            if !read_page(
                &app,
                &s,
                Route::Collections(epoch),
                library::PageQuery::Collections(page.current),
            ) {
                return;
            }
            *s.library_ui.collection_origin.borrow_mut() =
                Some((s.library_ui.collection_epoch.get(), original));
            s.library_ui.collection_epoch.set(epoch);
            *s.library_ui.collections.borrow_mut() = page;
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_create_playlist(move |name| {
        if let Some(app) = weak.upgrade() {
            if !s.playback_preferences.ready() {
                name_status(&app, "Wait for the local library to finish loading.");
                return;
            }
            if s.library_ui.rename_target.borrow().is_some() {
                name_status(
                    &app,
                    "Save or cancel the current rename before creating a playlist.",
                );
                return;
            }
            let value = match playlist_name(&name) {
                Ok(value) => value,
                Err(error) => {
                    name_status(&app, error);
                    return;
                }
            };
            if submit(&app, &s, library::Request::Create(value)) {
                *s.library_ui.name_write.borrow_mut() = Some(NameWrite {
                    kind: NameKind::Create,
                    draft: name,
                    rename_identity: None,
                    create_context: Some(create_context(&app, &s)),
                });
                name_status(&app, "Creating playlist…");
            }
        }
    });
    let weak = app.as_weak();
    app.global::<LibraryUi>().on_create(move |name| {
        if let Some(app) = weak.upgrade() {
            app.invoke_create_playlist(name);
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.global::<LibraryUi>().on_begin_rename(move || {
        let Some(app) = weak.upgrade() else {
            return false;
        };
        if app.get_page() != 1
            || app.global::<LibraryUi>().get_tab() != 0
            || s.library_ui.pending.get()
            || !s.playback_preferences.ready()
            || s.caption_cache.active()
            || s.library_ui.name_write.borrow().is_some()
            || s.library_ui.rename_target.borrow().is_some()
        {
            return false;
        }
        let item = s
            .playlists
            .borrow()
            .get(app.get_selected_playlist() as usize)
            .cloned();
        let Some(item) = item else {
            return false;
        };
        let ui = app.global::<LibraryUi>();
        *s.library_ui.rename_target.borrow_mut() = Some(RenameTarget {
            id: item.id,
            epoch: s.library_ui.route_epoch.get(),
            create_draft: ui.get_name_draft(),
            context_valid: true,
        });
        ui.set_name_draft(item.name.into());
        ui.set_renaming(true);
        ui.set_name_status("Edit this playlist’s name, then choose Save or press Enter.".into());
        true
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.global::<LibraryUi>().on_cancel_name(move || {
        if s.library_ui.name_write.borrow().is_some() {
            return;
        }
        if let Some(app) = weak.upgrade() {
            cancel_name(&app, &s);
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.global::<LibraryUi>().on_name_context_opened(move || {
        let Some(app) = weak.upgrade() else {
            return;
        };
        organization::reset(&app, &s);
        if let Some(write) = s.library_ui.name_write.borrow_mut().as_mut() {
            write.create_context = None;
            if let Some(target) = s.library_ui.rename_target.borrow_mut().as_mut() {
                target.context_valid = false;
            }
        } else {
            cancel_name(&app, &s);
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.global::<LibraryUi>().on_rename(move |name| {
        let Some(app) = weak.upgrade() else {
            return;
        };
        if s.library_ui.name_write.borrow().is_some() {
            return;
        }
        let id = s
            .library_ui
            .rename_target
            .borrow()
            .as_ref()
            .filter(|target| {
                target.matches(
                    selected(&app, &s),
                    s.library_ui.route_epoch.get(),
                    app.get_page() == 1 && app.global::<LibraryUi>().get_tab() == 0,
                )
            })
            .map(|target| target.id);
        let Some(id) = id else {
            cancel_name(&app, &s);
            name_status(&app, "The selection changed. Choose Rename again.");
            return;
        };
        let value = match playlist_name(&name) {
            Ok(value) => value,
            Err(error) => {
                name_status(&app, error);
                return;
            }
        };
        if submit(&app, &s, library::Request::Rename(id, value)) {
            *s.library_ui.name_write.borrow_mut() = Some(NameWrite {
                kind: NameKind::Rename,
                draft: name,
                rename_identity: Some((id, s.library_ui.route_epoch.get())),
                create_context: None,
            });
            name_status(&app, "Saving playlist name…");
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.global::<LibraryUi>().on_delete_collection(move || {
        if let Some(app) = weak.upgrade()
            && let Some(id) = selected(&app, &s)
        {
            submit(&app, &s, library::Request::Delete(id));
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.global::<SaveUi>()
        .on_begin(move || weak.upgrade().is_some_and(|app| begin_save(&app, &s)));
    let weak = app.as_weak();
    let s = state.clone();
    app.global::<SaveUi>().on_create_and_save(move |name| {
        if let Some(app) = weak.upgrade() {
            enqueue_video_save(&app, &s, None, Some(name.to_string()));
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_save_to_playlist(move |index| {
        let Some(app) = weak.upgrade() else { return };
        enqueue_video_save(&app, &s, Some(index), None);
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.global::<LibraryUi>().on_open(move |index| {
        let Some(app) = weak.upgrade() else { return };
        let query = {
            let items = s.library_ui.items.borrow();
            match items.get(index as usize) {
                Some(Item::Video(v)) => {
                    format!("https://www.youtube.com/watch?v={}", v.id.as_str())
                }
                Some(Item::History(v)) => {
                    format!("https://www.youtube.com/watch?v={}", v.video.id.as_str())
                }
                Some(Item::Follow(v)) => match follow_target(v) {
                    Ok(url) => url,
                    Err(_) => {
                        status(
                            &app,
                            "This saved follow has no supported canonical channel identity.",
                        );
                        return;
                    }
                },
                None => return,
            }
        };
        app.invoke_search(query.into());
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.global::<LibraryUi>().on_remove(move |index| {
        let Some(app) = weak.upgrade() else { return };
        let request = {
            let items = s.library_ui.items.borrow();
            match items.get(index as usize) {
                Some(Item::Video(v)) => {
                    let Some(id) = selected(&app, &s) else { return };
                    library::Request::Remove(id, v.id.clone())
                }
                Some(Item::History(v)) => library::Request::DeleteHistory(v.video.id.clone()),
                Some(Item::Follow(v)) => library::Request::Unfollow(v.channel_id.clone()),
                None => return,
            }
        };
        submit(&app, &s, request);
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.global::<LibraryUi>().on_follow_current(move || {
        let Some(app) = weak.upgrade() else { return };
        if crate::account_playback::authorization(&s).is_some() {
            status(
                &app,
                "Following an account playback channel locally is not supported yet.",
            );
            return;
        }
        let current = s.current_video.borrow().clone();
        if let Some(video) = current
            && let Some(id) = video.channel_id
        {
            follow_channel(&app, &s, &id, &video.channel);
        } else {
            status(
                &app,
                "Play a YouTube video with a known channel before following locally.",
            );
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.global::<LibraryUi>().on_history(move |enabled| {
        let Some(app) = weak.upgrade() else { return };
        if s.caption_cache.active() {
            return;
        }
        let mut prefs = desired_preferences(&s);
        prefs.privacy.local_history = enabled;
        if save_preferences(&app, &s, prefs) {
            status(&app, "Saving your local history preference…");
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.global::<LibraryUi>().on_set_retention(move |days| {
        if let Some(app) = weak.upgrade()
            && submit(&app, &s, library::Request::HistoryRetention(days as u16))
        {
            // Retention also prunes stored searches; re-read that list.
            crate::search_suggestions::reload(&s);
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.global::<LibraryUi>().on_clear_history(move || {
        if let Some(app) = weak.upgrade()
            && submit(&app, &s, library::Request::ClearHistory)
        {
            // The same request deletes stored searches; forget them now too.
            crate::search_suggestions::history_cleared(&app, &s);
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.global::<LibraryUi>().on_clear_local(move || {
        if let Some(app) = weak.upgrade() {
            if s.library_ui.pending.get()
                || s.library_ui.pending_save.borrow().is_some()
                || s.caption_cache.active()
            {
                status(&app, "The library is busy. Try clearing again shortly.");
                return;
            }
            s.library_ui.stop_picker();
            s.library_ui.volume_save.stop();
            s.library_ui.requested_preferences.set(None);
            s.library_ui.last_record.borrow_mut().take();
            crate::playback_preferences::cancel_for_clear(&app, &s);
            crate::caption_cache::begin(&app, &s);
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_theme_changed(move |index| {
        let Some(app) = weak.upgrade() else { return };
        let theme = match index {
            0 => serein_storage::Theme::System,
            1 => serein_storage::Theme::Light,
            2 => serein_storage::Theme::Dark,
            _ => return,
        };
        let mut prefs = desired_preferences(&s);
        if prefs.theme != theme {
            prefs.theme = theme;
            save_preferences(&app, &s, prefs);
        }
        // Preview only an admitted write; rejected admission retains the prior
        // accepted choice. Correlated responses above resolve persistence.
        app.set_theme(theme_index(desired_preferences(&s).theme));
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_preferences_changed(move || {
        let Some(app) = weak.upgrade() else { return };
        if s.caption_cache.active() {
            return;
        }
        let mut prefs = desired_preferences(&s);
        // Appearance has its own admission callback. Reading app.theme here
        // would persist a session-only --ui-theme override during volume save.
        prefs.volume_percent = app.get_volume_level().clamp(0., 100.) as u8;
        save_preferences(&app, &s, prefs);
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_default_quality(move |index| {
        let Some(app) = weak.upgrade() else { return };
        if let Some(quality) = serein_core::QualityCeiling::from_index(index) {
            save_quality(&app, &s, quality);
        }
        app.set_default_quality_index(desired_preferences(&s).playback.quality.index());
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_thumbnail_cache_changed(move |index| {
        let Some(app) = weak.upgrade() else { return };
        if let Some(mib) = thumbnail_cache_mib(index) {
            let mut prefs = desired_preferences(&s);
            prefs.thumbnail_cache_mib = mib;
            save_preferences(&app, &s, prefs);
        }
        // The selector reflects the committed policy until its write succeeds.
        app.set_thumbnail_cache_index(
            thumbnail_cache_index(s.preferences.get().thumbnail_cache_mib).unwrap_or(0),
        );
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.global::<LibraryUi>().on_transfer(move |kind| {
        let Some(app) = weak.upgrade() else { return };
        if s.library_ui.pending.get() {
            return;
        }
        s.library_ui.stop_picker();
        let weak = app.as_weak();
        let state = Rc::downgrade(&s);
        let task = slint::spawn_local(async move {
            let picker = rfd::AsyncFileDialog::new().set_title(match kind {
                0 => "Import local library JSON",
                1 => "Export local library JSON",
                _ => "Create SQLite library backup",
            });
            let file = if kind == 0 {
                picker.add_filter("JSON", &["json"]).pick_file().await
            } else {
                picker
                    .set_file_name(if kind == 1 {
                        "serein-library.json"
                    } else {
                        "serein-library.sqlite3"
                    })
                    .save_file()
                    .await
            };
            let (Some(file), Some(app), Some(s)) = (file, weak.upgrade(), state.upgrade()) else {
                return;
            };
            // Export and backup refuse replacement; choosing an existing file is
            // never treated as permission to destroy it merely by selecting it.
            let request = match kind {
                0 => library::Request::Import(file.path().to_path_buf()),
                1 => library::Request::Export {
                    path: file.path().to_path_buf(),
                    overwrite: false,
                },
                _ => library::Request::Backup(file.path().to_path_buf()),
            };
            submit(&app, &s, request);
        });
        match task {
            Ok(task) => *s.library_ui.picker.borrow_mut() = Some(task),
            Err(_) => status(&app, "The native file picker could not be opened."),
        }
    });
}

fn destination_admitted(tab: i32, history_enabled: bool, blocked: bool) -> bool {
    !blocked && (0..=2).contains(&tab) && (tab != 2 || history_enabled)
}

/// Select a local destination and submit its first page as one operation. In
/// particular, do not activate the old tab before changing to the requested tab.
pub fn open_tab(app: &App, state: &UiState, tab: i32) -> bool {
    crate::fixture_quiescence::trace(state, "open-local-tab-requested");
    let ui = app.global::<LibraryUi>();
    if !destination_admitted(
        tab,
        state.preferences.get().privacy.local_history,
        state.library_ui.pending.get()
            || state.caption_cache.active()
            || ui.get_confirmation() != 0,
    ) {
        return false;
    }
    if app.get_page() == 1 && ui.get_tab() == tab {
        return false;
    }
    let route_changed = ui.get_tab() != tab
        || state.library_ui.pages.borrow().current.is_some()
        || !state.library_ui.filter.borrow().is_empty();
    let epoch = if route_changed {
        let Some(epoch) = next_epoch(app, &state.library_ui.route_epoch) else {
            return false;
        };
        epoch
    } else {
        state.library_ui.route_epoch.get()
    };
    let request = match tab {
        1 => Some((
            Route::Subscriptions(epoch),
            library::PageQuery::Subscriptions(None),
        )),
        2 => Some((Route::History(epoch), library::PageQuery::History(None))),
        _ => selected(app, state).map(|id| {
            (
                Route::Videos(id, epoch),
                library::PageQuery::Videos(id, None),
            )
        }),
    };
    if let Some((route, query)) = request
        && !read_page(app, state, route, query)
    {
        return false;
    }
    state.library_ui.route_epoch.set(epoch);
    cancel_name(app, state);
    reset_filter(app, state);
    organization::reset(app, state);
    ui.set_tab(tab);
    if route_changed {
        *state.library_ui.pages.borrow_mut() = Pagination::default();
    }
    if route_changed || state.library_ui.published_route.borrow().is_none() {
        publish(app, state, Vec::new(), None);
    }
    app.set_page(1);
    // Internal library tabs can change without a page change notification. End
    // old history artwork work even when navigation stays on page one.
    app.invoke_refresh_visible();
    true
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_rows_format_long_positions_without_raw_ids_or_timestamps() {
        let entry = HistoryEntry {
            video: VideoSummary {
                id: serein_core::VideoId::new("abcdefghijk").unwrap(),
                title: "Synthetic title".into(),
                channel: "Synthetic creator".into(),
                channel_id: None,
                duration: Some(Duration::from_secs(7_200)),
                thumbnail_url: None,
            },
            position: Duration::from_secs(3_723),
            watched_at: SystemTime::UNIX_EPOCH,
        };
        let row = history_row(&entry, "Today".into());
        assert_eq!(row.detail.as_str(), "Watched 1:02:03 of 2:00:00");
        assert_eq!(row.duration.as_str(), "2:00:00");
        assert_eq!(row.channel.as_str(), "Synthetic creator");
        assert_eq!(row.day_heading.as_str(), "Today");
        assert!((row.progress - 0.517_083_35).abs() < 0.000_001);
        assert!(!row.thumbnail_ready);
        let completed = history_row(
            &HistoryEntry {
                position: Duration::MAX,
                ..entry
            },
            String::new(),
        );
        assert_eq!(completed.detail.as_str(), "Watched");
        assert_eq!(completed.progress, 1.0);
        assert!(completed.day_heading.is_empty());
    }
    #[test]
    fn created_selection_requires_its_exact_read_and_live_submission_context() {
        let store = serein_storage::LocalStore::in_memory().unwrap();
        let id = store.create_playlist("Synthetic created").unwrap();
        let context = CreateContext {
            route_epoch: 3,
            collection_epoch: 7,
            visible: true,
        };
        assert!(context.matches(3, 7, true));
        for (route, collection, visible) in [(4, 7, true), (3, 8, true), (3, 7, false)] {
            assert!(!context.matches(route, collection, visible));
        }
        assert!(
            !CreateContext {
                visible: false,
                ..context
            }
            .matches(3, 7, true)
        );
        let state = State::default();
        *state.created_selection.borrow_mut() = Some(CreatedSelection {
            ticket: 42,
            id,
            context,
        });
        assert!(take_created_selection(&state, 41).is_none());
        assert_eq!(
            state.created_selection.borrow().as_ref().unwrap().ticket,
            42
        );
        assert_eq!(take_created_selection(&state, 42).unwrap().id, id);
        assert!(take_created_selection(&state, 42).is_none());
    }
    #[test]
    fn collection_windows_move_both_directions_after_a_jump_without_a_cursor_stack() {
        let store = serein_storage::LocalStore::in_memory().unwrap();
        let ids: Vec<_> = (0..206)
            .map(|i| store.create_playlist(&format!("Synthetic {i}")).unwrap())
            .collect();
        let last = store
            .playlist_window(PlaylistWindow::EndingAt(ids[205]), 100)
            .unwrap();
        let mut window = CollectionWindow {
            current: last.current,
            previous: last.previous,
            next: last.next,
        };
        assert!(!window.advance(true));
        assert!(window.advance(false));
        let previous = store.playlist_window(window.current, 100).unwrap();
        assert_eq!(previous.items.first().unwrap().id, ids[6]);
        assert_eq!(previous.items.last().unwrap().id, ids[105]);
        window = CollectionWindow {
            current: previous.current,
            previous: previous.previous,
            next: previous.next,
        };
        assert!(window.advance(true));
        let again = store.playlist_window(window.current, 100).unwrap();
        assert_eq!(again.items, last.items);
    }
    #[test]
    fn thumbnail_cache_selector_admits_only_supported_persisted_limits() {
        for (index, mib) in [(0, 0), (1, 32), (2, 128), (3, 256)] {
            assert_eq!(thumbnail_cache_mib(index), Some(mib));
            assert_eq!(thumbnail_cache_index(mib), Some(index));
        }
        for index in [i32::MIN, -1, 4, i32::MAX] {
            assert_eq!(thumbnail_cache_mib(index), None);
        }
        for mib in [1, 31, 127, 257, u16::MAX] {
            assert_eq!(thumbnail_cache_index(mib), None);
        }
    }
    #[test]
    fn rename_target_rejects_selection_navigation_and_reset_changes() {
        let store = serein_storage::LocalStore::in_memory().unwrap();
        let first = store.create_playlist("Synthetic first").unwrap();
        let second = store.create_playlist("Synthetic second").unwrap();
        let mut target = RenameTarget {
            id: first,
            epoch: 4,
            create_draft: "Unsubmitted new playlist".into(),
            context_valid: true,
        };
        assert!(target.matches(Some(first), 4, true));
        assert!(!target.matches(Some(second), 4, true));
        assert!(!target.matches(None, 4, true));
        assert!(!target.matches(Some(first), 5, true));
        assert!(!target.matches(Some(first), 4, false));
        target.context_valid = false;
        assert!(!target.matches(Some(first), 4, true));
        assert_eq!(target.create_draft.as_str(), "Unsubmitted new playlist");
    }
    #[test]
    fn playlist_names_validate_trimmed_utf8_bytes_without_losing_the_input() {
        assert_eq!(playlist_name("  Watch later  ").unwrap(), "Watch later");
        assert_eq!(playlist_name(&"é".repeat(512)).unwrap().len(), 1024);
        for value in [" ".into(), "a\nb".into(), "a\0b".into(), "é".repeat(513)] {
            assert!(playlist_name(&value).is_err());
        }
    }
    #[test]
    fn collection_read_failure_restores_epoch_and_next_cursor_without_model_changes() {
        let store = serein_storage::LocalStore::in_memory().unwrap();
        store.create_playlist("Synthetic first").unwrap();
        store.create_playlist("Synthetic second").unwrap();
        let cursor = store.playlists(None, 1).unwrap().next.unwrap();
        let original = CollectionWindow {
            next: Some(PlaylistWindow::After(cursor)),
            ..Default::default()
        };
        let state = State::default();
        state.collection_epoch.set(8);
        let mut requested = original.clone();
        assert!(requested.advance(true));
        *state.collections.borrow_mut() = requested;
        restore_collection_page(&state, Some((7, original)));
        assert_eq!(state.collection_epoch.get(), 7);
        let page = state.collections.borrow();
        assert_eq!(page.current, PlaylistWindow::First);
        assert_eq!(page.next, Some(PlaylistWindow::After(cursor)));
        assert!(page.previous.is_none());
        assert_eq!(state.collection_replacements.get(), 0);
        assert_eq!(state.collection_changes.get(), 0);
    }
    #[test]
    fn late_page_success_failure_or_duplicate_cannot_complete_newer_read() {
        let mut reads = Reads::default();
        let first = reads.start(Route::Subscriptions(10)).unwrap();
        assert!(reads.start(Route::History(11)).is_none());
        assert_eq!(reads.finish(first), Some(Route::Subscriptions(10)));
        let current = reads.start(Route::History(11)).unwrap();
        // Both failure and success use the same terminal ticket gate, before
        // either their payload or error can reach the UI.
        for _ in 0..2 {
            assert_eq!(reads.finish(first), None);
        }
        assert_eq!(reads.pending, Some((current, Route::History(11))));
        assert_eq!(reads.finish(current), Some(Route::History(11)));
        assert_eq!(reads.finish(current), None);
        reads.serial = u64::MAX;
        assert!(reads.start(Route::History(12)).is_none());
        assert!(reads.pending.is_none());
    }
    #[test]
    fn page_identity_checks_route_epoch_kind_and_playlist_id() {
        let store = serein_storage::LocalStore::in_memory().unwrap();
        let first = store.create_playlist("Synthetic first").unwrap();
        let second = store.create_playlist("Synthetic second").unwrap();
        let result = library::PageResult::Videos(
            first,
            serein_storage::Page {
                items: Vec::new(),
                next: None,
            },
        );
        assert!(Route::Videos(first, 7).accepts(&result));
        assert!(!Route::Videos(second, 7).accepts(&result));
        assert!(!Route::Subscriptions(7).accepts(&result));
        assert!(!Route::History(7).accepts(&result));
        assert!(!Route::Collections(7).accepts(&result));
        assert_ne!(Route::Videos(first, 7), Route::Videos(first, 8));
        assert_ne!(Route::Collections(7), Route::Collections(8));
    }
    #[test]
    fn collection_selection_survives_rename_and_deletion_of_an_earlier_sibling() {
        let store = serein_storage::LocalStore::in_memory().unwrap();
        let first = store.create_playlist("Synthetic first").unwrap();
        let second = store.create_playlist("Synthetic second").unwrap();
        let mut items = store.playlists(None, 100).unwrap().items;
        assert_eq!(collection_selection(&items, Some(second)), 1);
        items[1].name = "Changed display name".into();
        assert_eq!(collection_selection(&items, Some(second)), 1);
        items.remove(0);
        assert_eq!(collection_selection(&items, Some(second)), 0);
        assert_eq!(collection_selection(&items, Some(first)), 0);
        assert_eq!(collection_selection(&[], Some(second)), -1);
    }
    #[test]
    fn save_target_rejects_replacement_failed_and_stopped_loads() {
        let video = VideoSummary {
            id: serein_core::VideoId::new("abcdefghijk").unwrap(),
            title: "Synthetic title".into(),
            channel: "Synthetic channel".into(),
            channel_id: None,
            duration: None,
            thumbnail_url: None,
        };
        let target = SaveTarget {
            video: video.clone(),
            load: 42,
            collections: Vec::new(),
        };
        let snapshot = serein_media::Snapshot {
            state: serein_media::PlaybackState::Playing,
            playback_restarted: true,
            load_request_id: 42,
            active_load_request_id: 42,
            ..Default::default()
        };
        assert!(target.matches(&video, &snapshot));
        let mut renamed = video.clone();
        renamed.title = "Updated actual metadata".into();
        assert!(target.matches(&renamed, &snapshot));
        renamed.id = serein_core::VideoId::new("lmnopqrstuv").unwrap();
        assert!(!target.matches(&renamed, &snapshot));
        for (requested, active) in [(0, 42), (43, 42), (43, 43), (42, 0)] {
            let changed = serein_media::Snapshot {
                load_request_id: requested,
                active_load_request_id: active,
                ..snapshot.clone()
            };
            assert!(!target.matches(&video, &changed));
        }
        for state in [
            serein_media::PlaybackState::Idle,
            serein_media::PlaybackState::Failed,
            serein_media::PlaybackState::Seeking,
        ] {
            assert!(!target.matches(
                &video,
                &serein_media::Snapshot {
                    state,
                    ..snapshot.clone()
                }
            ));
        }
        assert!(!target.matches(
            &video,
            &serein_media::Snapshot {
                stop_pending: true,
                ..snapshot.clone()
            }
        ));
        assert!(!target.matches(
            &video,
            &serein_media::Snapshot {
                playback_restarted: false,
                ..snapshot.clone()
            }
        ));
        assert!(!target.matches(
            &video,
            &serein_media::Snapshot {
                failed_load_request_id: Some(42),
                ..snapshot
            }
        ));
    }
    #[test]
    fn unrelated_and_repeated_save_responses_do_not_acknowledge_the_current_write() {
        let state = State::default();
        *state.pending_save.borrow_mut() = Some(PendingSave {
            serial: 7,
            collection_name: "Synthetic playlist".into(),
        });
        assert!(state.finish_video_save(6).is_none());
        assert!(state.pending_save.borrow().is_some());
        assert_eq!(
            state.finish_video_save(7).unwrap().collection_name,
            "Synthetic playlist"
        );
        assert!(state.finish_video_save(7).is_none());
        *state.pending_save.borrow_mut() = Some(PendingSave {
            serial: 8,
            collection_name: "Another explicit save".into(),
        });
        assert!(state.finish_video_save(7).is_none());
        assert!(state.pending_save.borrow().is_some());
    }
    #[test]
    fn identical_preference_values_do_not_let_an_old_failure_discard_a_new_write() {
        let state = State::default();
        let value = serein_storage::LocalPreferences {
            playback: serein_core::PlaybackPreferences {
                quality: serein_core::QualityCeiling::P720,
                speed: serein_core::PlaybackSpeed::OneAndHalf,
            },
            ..Default::default()
        };
        let old = library::PreferenceWrite { id: 4, value };
        let new = library::PreferenceWrite { id: 5, value };
        state.requested_preferences.set(Some(new));
        assert!(!state.finish_preferences(old));
        assert_eq!(state.requested_preferences.get(), Some(new));
        assert!(state.finish_preferences(new));
        assert_eq!(state.requested_preferences.get(), None);
        assert!(!state.finish_preferences(new));
    }
    #[test]
    fn theme_preview_keeps_newer_admission_and_rolls_back_only_its_failed_write() {
        use serein_storage::{LocalPreferences, Theme};
        let state = State::default();
        let mut committed = LocalPreferences::default();
        let light = library::PreferenceWrite {
            id: 8,
            value: LocalPreferences {
                theme: Theme::Light,
                ..committed
            },
        };
        let dark = library::PreferenceWrite {
            id: 9,
            value: LocalPreferences {
                theme: Theme::Dark,
                ..committed
            },
        };
        state.requested_preferences.set(Some(dark));
        assert_eq!(state.desired_preferences(committed).theme, Theme::Dark);
        // Whether the earlier Light write fails or commits, its completion
        // cannot end the newer Dark preview.
        assert!(!state.finish_preferences(light));
        assert_eq!(state.desired_preferences(committed).theme, Theme::Dark);
        committed = light.value;
        assert_eq!(state.desired_preferences(committed).theme, Theme::Dark);
        // A failed matching Dark write restores the latest actual SQL state.
        assert!(state.finish_preferences(dark));
        assert_eq!(state.desired_preferences(committed).theme, Theme::Light);
        // A later successful write becomes the committed choice.
        state.requested_preferences.set(Some(dark));
        assert!(state.finish_preferences(dark));
        committed = dark.value;
        assert_eq!(state.desired_preferences(committed).theme, Theme::Dark);
    }
    #[test]
    fn destination_admission_requires_opt_in_and_never_bypasses_a_pending_operation() {
        for tab in -4..8 {
            for enabled in [false, true] {
                assert!(!destination_admitted(tab, enabled, true));
                assert_eq!(
                    destination_admitted(tab, enabled, false),
                    (0..=2).contains(&tab) && (tab != 2 || enabled)
                );
            }
        }
    }
    #[test]
    fn local_follow_navigation_uses_the_canonical_id_not_its_display_name() {
        let follow = LocalSubscription {
            channel_id: ChannelId::new("UCabcdefghijklmnopqrstuv").unwrap(),
            name: "Unrelated display name".into(),
        };
        assert_eq!(
            follow_target(&follow).unwrap(),
            "https://www.youtube.com/channel/UCabcdefghijklmnopqrstuv"
        );
        let invalid = LocalSubscription {
            channel_id: ChannelId("UCbad/../../account".into()),
            name: "Synthetic".into(),
        };
        assert!(follow_target(&invalid).is_err());
    }
    #[test]
    fn page_navigation_retains_previous_and_bounds_history() {
        let store = serein_storage::LocalStore::in_memory().unwrap();
        store.create_playlist("Synthetic one").unwrap();
        store.create_playlist("Synthetic two").unwrap();
        let cursor = store.playlists(None, 1).unwrap().next.unwrap();
        let mut page = Pagination::default();
        assert!(!page.advance(false));
        assert!(!page.advance(true));
        for _ in 0..1100 {
            page.next = Some(Cursor::Page(cursor));
            assert!(page.advance(true));
        }
        assert_eq!(page.previous.len(), 1024);
        assert!(page.advance(false));
        assert_eq!(page.page(), Some(cursor));
    }
}
