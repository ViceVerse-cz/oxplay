// SPDX-License-Identifier: GPL-3.0-or-later
//! Shared-UI account adapter. Names/capabilities enter Slint; cookies never do.
use crate::{
    AccountRow, App, UiState,
    account::{self, AccountRequest, Persistence, Response},
};
use serein_youtube::account::{
    AccountChannel, AccountCursor, AccountMutation, AccountPlaylist, AccountPlaylistItem,
    Capability, MutationOutcome,
};
use slint::ComponentHandle;
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};
#[derive(Clone)]
enum Item {
    Channel(AccountChannel),
    Playlist(AccountPlaylist),
    Video(AccountPlaylistItem),
}
#[derive(Clone)]
enum Scope {
    Subscriptions,
    Playlists,
    Playlist(serein_core::PlaylistId),
}
#[derive(Clone)]
struct RatingTarget {
    video: serein_core::VideoId,
    generation: u64,
}
impl RatingTarget {
    fn matches(&self, video: Option<&serein_core::VideoId>, generation: u64) -> bool {
        video == Some(&self.video) && self.generation == generation
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum PendingKind {
    Other,
    Connection,
    Inspection,
    Playback,
    Mutation,
}
// In-memory warning for this application run only. It is not a persisted
// reconciliation record and contains no identity or operation parameters.
#[derive(Default)]
struct WriteWarning {
    current: Cell<bool>,
    retired: Cell<bool>,
}
impl WriteWarning {
    fn retire(&self, mutation_active: bool) {
        if self.current.replace(false) || mutation_active {
            self.retired.set(true);
        }
    }
    fn status(&self, message: impl Into<String>) -> slint::SharedString {
        let mut message = message.into();
        if self.retired.get() {
            message.push_str(" A previous account change has an unconfirmed outcome. Check YouTube before repeating it. Reconnecting does not reconcile or replay that change.");
        }
        message.into()
    }
}
pub struct State {
    worker: account::Worker,
    rows: Rc<slint::VecModel<AccountRow>>,
    items: RefCell<Vec<Item>>,
    scope: RefCell<Scope>,
    cursor: RefCell<Option<AccountCursor>>,
    saved: RefCell<Option<account::SavedProfile>>,
    pending_id: Cell<Option<u64>>,
    pending_kind: Cell<PendingKind>,
    picker: RefCell<Option<slint::JoinHandle<()>>>,
    playlist_editable: Cell<bool>,
    rating_video: RefCell<Option<RatingTarget>>,
    playback_capable: Cell<bool>,
    identity_epoch: Cell<u64>,
    pending_identity_epoch: Cell<u64>,
    write_warning: WriteWarning,
}
impl State {
    pub fn new(
        weak: slint::Weak<App>,
        directory: PathBuf,
        resolver: crate::resolver::SharedResolver,
    ) -> Self {
        Self {
            worker: account::Worker::with_resolver(directory, resolver, move || {
                let _ = weak.upgrade_in_event_loop(|app| app.invoke_account_wake());
            }),
            rows: Rc::new(slint::VecModel::default()),
            items: RefCell::new(Vec::new()),
            scope: RefCell::new(Scope::Subscriptions),
            cursor: RefCell::new(None),
            saved: RefCell::new(None),
            pending_id: Cell::new(None),
            pending_kind: Cell::new(PendingKind::Other),
            picker: RefCell::new(None),
            playlist_editable: Cell::new(false),
            rating_video: RefCell::new(None),
            playback_capable: Cell::new(false),
            identity_epoch: Cell::new(0),
            pending_identity_epoch: Cell::new(0),
            write_warning: WriteWarning::default(),
        }
    }
    pub fn stop_picker(&self) {
        if let Some(picker) = self.picker.borrow_mut().take() {
            picker.abort();
        }
    }
    pub fn session_generation(&self) -> u64 {
        self.worker.control().generation()
    }
    pub fn can_playback(&self, app: &App) -> bool {
        app.get_account_connected() && !app.get_account_pending() && self.playback_capable.get()
    }
    pub fn submit_playback(
        &self,
        app: &App,
        id: serein_core::VideoId,
        policy: serein_youtube::ResolutionPolicy,
        selection_generation: u64,
    ) -> Result<(), String> {
        if !self.can_playback(app) {
            return Err("Verify a supported account identity and reconcile any pending account change first.".into());
        }
        let id = self
            .worker
            .submit(AccountRequest::ResolvePlayback {
                id,
                policy,
                selection_generation,
            })
            .map_err(|error| error.to_string())?;
        self.pending_id.set(Some(id));
        self.pending_kind.set(PendingKind::Playback);
        self.pending_identity_epoch.set(self.identity_epoch.get());
        app.set_account_busy(true);
        Ok(())
    }
    pub fn cancel_playback(&self, app: &App) {
        if let Err(error) = self.worker.cancel_playback() {
            app.set_account_status(error.to_string().into());
        }
        if self.pending_kind.get() == PendingKind::Playback {
            self.pending_kind.set(PendingKind::Other);
            self.pending_id.set(None);
            app.set_account_busy(false);
        }
    }
    fn submit(&self, app: &App, request: AccountRequest) {
        if !app.get_account_connected()
            && !matches!(
                &request,
                AccountRequest::Import { .. }
                    | AccountRequest::Reconnect(_)
                    | AccountRequest::InspectSaved
            )
        {
            app.set_account_status(
                "Connect and verify an account identity before requesting account data.".into(),
            );
            return;
        }
        let kind = match &request {
            AccountRequest::Import { .. } | AccountRequest::Reconnect(_) => PendingKind::Connection,
            AccountRequest::InspectSaved => PendingKind::Inspection,
            AccountRequest::ResolvePlayback { .. } => PendingKind::Playback,
            AccountRequest::Mutate(_) | AccountRequest::Reconcile => PendingKind::Mutation,
            _ => PendingKind::Other,
        };
        match self.worker.submit(request) {
            Ok(id) => {
                self.pending_id.set(Some(id));
                self.pending_kind.set(kind);
                self.pending_identity_epoch.set(self.identity_epoch.get());
                app.set_account_busy(true);
            }
            Err(error) => self.set_status(app, error.to_string()),
        }
    }
    fn clear_rows(&self, app: &App) {
        self.items.borrow_mut().clear();
        self.rows.set_vec(Vec::new());
        self.cursor.borrow_mut().take();
        app.set_account_more(false);
    }
    pub fn set_status(&self, app: &App, message: impl Into<String>) {
        app.set_account_status(self.write_warning.status(message));
    }
    pub fn clear_identity(&self, app: &App) {
        // Preserve only a generic warning across identity loss. Never carry a
        // pending operation's private IDs into a newly verified account.
        self.write_warning
            .retire(self.pending_kind.get() == PendingKind::Mutation);
        self.identity_epoch
            .set(self.identity_epoch.get().wrapping_add(1));
        self.playback_capable.set(false);
        self.clear_rows(app);
        self.rating_video.borrow_mut().take();
        app.set_account_connected(false);
        app.set_account_identity("".into());
        app.set_account_capabilities("".into());
        app.set_account_pending(false);
        app.set_rating_known(false);
        app.set_account_liked(false);
    }
    fn publish(&self, app: &App, items: Vec<Item>, next: Option<AccountCursor>, partial: bool) {
        let rows: Vec<_> = items
            .iter()
            .map(|item| match item {
                Item::Channel(item) => AccountRow {
                    can_account_play: false,
                    title: item.title.clone().into(),
                    detail: "YouTube subscription · Select to check remote state".into(),
                    action: "Unsubscribe on YouTube".into(),
                },
                Item::Playlist(item) => AccountRow {
                    can_account_play: false,
                    title: item.title.clone().into(),
                    detail: "YouTube account playlist".into(),
                    action: if item.editable == Some(true) && app.get_loaded() {
                        "Add playing video"
                    } else {
                        ""
                    }
                    .into(),
                },
                Item::Video(item) => AccountRow {
                    can_account_play: self.playback_capable.get(),
                    title: item.title.clone().into(),
                    detail: "Play using guest access · Account access is never automatic".into(),
                    action: if self.playlist_editable.get() && item.set_video_id.is_some() {
                        "Remove from YouTube"
                    } else {
                        ""
                    }
                    .into(),
                },
            })
            .collect();
        *self.items.borrow_mut() = items;
        self.rows.set_vec(rows);
        app.set_account_more(next.is_some());
        *self.cursor.borrow_mut() = next;
        app.set_account_status(
            if partial {
                "Some account items could not be interpreted. The supported items are shown."
            } else {
                "Account data read from YouTube. Local collections are separate."
            }
            .into(),
        );
    }
}
fn capability(value: Capability) -> &'static str {
    match value {
        Capability::Verified => "verified",
        Capability::ImplementedUnverified => "implemented; not yet verified for this session",
        Capability::Unsupported => "unsupported",
    }
}
fn capabilities(c: &serein_youtube::account::AccountCapabilities) -> String {
    format!(
        "Identity: {} · Subscriptions: {} · Playlists: {}\nSubscription writes: {} · Likes: {} · Playlist edits: {}\nAuthenticated extraction: {} · Channel switching: {}",
        capability(c.identity),
        capability(c.subscriptions),
        capability(c.playlists),
        capability(c.subscription_writes),
        capability(c.likes),
        capability(c.playlist_writes),
        capability(c.authenticated_playback),
        capability(c.channel_switching)
    )
}
pub fn bind(app: &App, state: &Rc<UiState>) {
    app.set_account_media_available(state.account_media_network.is_some());
    app.set_account_rows(slint::ModelRc::from(state.account_ui.rows.clone()));
    let weak = app.as_weak();
    let s = state.clone();
    app.on_account_import(move || {
        let Some(app) = weak.upgrade() else { return };
        if !app.get_account_consent() || app.get_account_busy() {
            return;
        }
        s.account_ui.stop_picker();
        let weak = app.as_weak();
        let remember = app.get_account_remember();
        let control = s.account_ui.worker.control();
        let generation = control.generation();
        app.set_account_busy(true);
        let picker = slint::spawn_local(async move {
            let file = rfd::AsyncFileDialog::new()
                .set_title("Select your explicit Netscape session export")
                .add_filter("Netscape cookie export", &["txt"])
                .pick_file()
                .await;
            if control.generation() != generation {
                return;
            }
            if let Some(app) = weak.upgrade() {
                app.set_account_busy(false);
                if let Some(file) = file
                    && app.get_account_consent()
                {
                    // Never choose a different file through lossy path conversion.
                    if let Some(path) = file.path().to_str() {
                        app.invoke_account_import_path(path.into(), remember);
                    } else {
                        app.set_account_status("Choose an export with a UTF-8 file name.".into());
                    }
                }
            }
        });
        match picker {
            Ok(picker) => *s.account_ui.picker.borrow_mut() = Some(picker),
            Err(_) => {
                app.set_account_busy(false);
                app.set_account_status("The native file picker is unavailable.".into());
            }
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_account_import_path(move |path, remember| {
        let Some(app) = weak.upgrade() else { return };
        if !app.get_account_consent() || app.get_account_busy() {
            return;
        }
        s.account_ui.clear_identity(&app);
        crate::account_playback::clear(&app, &s);
        app.set_account_status(
            s.account_ui
                .write_warning
                .status("Validating the selected export and verifying its YouTube identity…"),
        );
        s.account_ui.submit(
            &app,
            AccountRequest::Import {
                path: PathBuf::from(path.as_str()),
                account_index: 0,
                remember,
            },
        );
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_account_reconnect(move || {
        if let Some(app) = weak.upgrade()
            && app.get_account_consent()
            && !app.get_account_busy()
            && let Some(saved) = s.account_ui.saved.borrow().clone()
        {
            s.account_ui.clear_identity(&app);
            crate::account_playback::clear(&app, &s);
            app.set_account_status(
                s.account_ui
                    .write_warning
                    .status("Verifying the saved YouTube session…"),
            );
            s.account_ui.submit(&app, AccountRequest::Reconnect(saved));
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_account_disconnect(move || {
        let Some(app) = weak.upgrade() else { return };
        s.account_ui.stop_picker();
        crate::account_playback::clear(&app, &s);
        s.account_ui.clear_identity(&app);
        app.set_account_status(
            s.account_ui
                .write_warning
                .status("Disconnected. Removing saved credentials; local collections are kept."),
        );
        s.account_ui.pending_kind.set(PendingKind::Other);
        // This invalidates the session synchronously, before filesystem/network work.
        match s.account_ui.worker.disconnect() {
            Ok(id) => {
                s.account_ui.pending_id.set(Some(id));
                app.set_account_busy(true);
            }
            Err(error) => s.account_ui.set_status(&app, error.to_string()),
        }
        s.account_ui.saved.borrow_mut().take();
        app.set_account_saved(false);
        app.set_account_forget_needed(true);
        app.set_account_consent(false);
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_account_tab(move |tab| {
        let Some(app) = weak.upgrade() else { return };
        s.account_ui.clear_rows(&app);
        *s.account_ui.scope.borrow_mut() = if tab == 0 {
            Scope::Subscriptions
        } else {
            Scope::Playlists
        };
        s.account_ui.submit(
            &app,
            if tab == 0 {
                AccountRequest::Subscriptions(None)
            } else {
                AccountRequest::Playlists(None)
            },
        );
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_account_next(move || {
        let Some(app) = weak.upgrade() else { return };
        let Some(cursor) = s.account_ui.cursor.borrow().clone() else {
            return;
        };
        let request = match s.account_ui.scope.borrow().clone() {
            Scope::Subscriptions => AccountRequest::Subscriptions(Some(cursor)),
            Scope::Playlists => AccountRequest::Playlists(Some(cursor)),
            Scope::Playlist(id) => AccountRequest::Playlist(id, Some(cursor)),
        };
        s.account_ui.submit(&app, request);
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_account_open(move |index| {
        let Some(app) = weak.upgrade() else { return };
        let item = s.account_ui.items.borrow().get(index as usize).cloned();
        match item {
            Some(Item::Channel(item)) => s
                .account_ui
                .submit(&app, AccountRequest::SubscriptionState(item.id)),
            Some(Item::Playlist(item)) => {
                *s.account_ui.scope.borrow_mut() = Scope::Playlist(item.id.clone());
                s.account_ui
                    .submit(&app, AccountRequest::Playlist(item.id, None));
            }
            Some(Item::Video(item)) => app.invoke_search(
                format!("https://www.youtube.com/watch?v={}", item.video_id.as_str()).into(),
            ),
            None => {}
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_account_play(move |index| {
        let Some(app) = weak.upgrade() else { return };
        let item = s.account_ui.items.borrow().get(index as usize).cloned();
        if let Some(Item::Video(item)) = item
            && let Err(error) = crate::account_playback::request_initial(&app, &s, &item.video_id)
        {
            app.set_account_status(error.into());
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_account_action(move |index| {
        let Some(app) = weak.upgrade() else { return };
        if app.get_account_busy() || app.get_account_pending() {
            return;
        }
        let item = s.account_ui.items.borrow().get(index as usize).cloned();
        let mutation = match item {
            Some(Item::Channel(item)) => AccountMutation::Subscription {
                channel_id: item.id,
                subscribed: false,
            },
            Some(Item::Playlist(item)) if item.editable == Some(true) => {
                let Some(video) = s.current_video.borrow().clone() else {
                    app.set_account_status("Select a YouTube video first.".into());
                    return;
                };
                AccountMutation::AddToPlaylist {
                    playlist_id: item.id,
                    video_id: video.id,
                }
            }
            Some(Item::Video(item)) if s.account_ui.playlist_editable.get() => {
                let Scope::Playlist(id) = s.account_ui.scope.borrow().clone() else {
                    return;
                };
                let Some(set_video_id) = item.set_video_id else {
                    return;
                };
                AccountMutation::RemoveFromPlaylist {
                    playlist_id: id,
                    set_video_id,
                }
            }
            _ => return,
        };
        app.set_account_status(
            "Submitting your account change once, then checking its remote state…".into(),
        );
        s.account_ui.submit(&app, AccountRequest::Mutate(mutation));
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_account_reconcile(move || {
        if let Some(app) = weak.upgrade() {
            s.account_ui.submit(&app, AccountRequest::Reconcile);
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_account_rating(move || {
        let Some(app) = weak.upgrade() else { return };
        let Some(video) = s.current_video.borrow().clone() else {
            return;
        };
        if !app.get_account_connected() || app.get_account_busy() || app.get_account_pending() {
            return;
        }
        s.account_ui.submit(
            &app,
            if app.get_rating_known() {
                AccountRequest::Mutate(AccountMutation::Rating {
                    video_id: video.id,
                    liked: !app.get_account_liked(),
                })
            } else {
                {
                    *s.account_ui.rating_video.borrow_mut() = Some(RatingTarget {
                        video: video.id.clone(),
                        generation: s.account_ui.worker.control().generation(),
                    });
                    AccountRequest::Rating(video.id)
                }
            },
        );
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_account_wake(move || {
        let Some(app) = weak.upgrade() else { return };
        let Some(result) = s.account_ui.worker.take() else { return };
        if result.generation != s.account_ui.worker.control().generation() || s.account_ui.pending_id.get() != Some(result.request_id) { return; }
        app.set_account_busy(false); s.account_ui.pending_id.set(None);
        let kind = s.account_ui.pending_kind.replace(PendingKind::Other);
        s.account_ui.write_warning.current.set(result.unconfirmed_mutation);
        let current_identity = s.account_ui.pending_identity_epoch.get() == s.account_ui.identity_epoch.get();
        // Expiry can clear identity while an unrelated account read is in flight.
        // It must never repopulate private rows afterward. Terminal mutation
        // outcomes/errors still reach the user; no remote write is cancelled.
        if !current_identity && is_private_read(&result.result) { return; }
        if current_identity && let Some(connection) = &result.connection {
            app.set_account_capabilities(capabilities(&connection.capabilities).into());
            s.account_ui.playback_capable.set(connection.capabilities.authenticated_playback != Capability::Unsupported);
        }
        if result.playback_selection.is_some() {
            crate::account_playback::receive(&app, &s, result);
            crate::home_ui::maybe_refresh(&app, &s);
            return;
        }
        match result.result {
            Ok(Response::Connected { connection, persistence }) => {
                app.set_account_connected(true); app.set_account_identity(connection.identity.display_name.into());
                s.account_ui.saved.borrow_mut().take();
                app.set_account_saved(false);
                app.set_account_forget_needed(false);
                match persistence {
                    Persistence::SessionOnly => app.set_account_status(s.account_ui.write_warning.status("Identity verified. Session is kept in memory only. Public playback remains guest.")),
                    Persistence::Remembered(saved) => { *s.account_ui.saved.borrow_mut() = Some(saved); app.set_account_saved(true); app.set_account_status(s.account_ui.write_warning.status("Identity verified. Session saved in protected storage. Public playback remains guest.")); }
                    Persistence::SaveFailed(error) => { app.set_account_forget_needed(true); app.set_account_status(s.account_ui.write_warning.status(format!("Identity verified; using memory only. {error}"))); }
                }
            }
            Ok(Response::SavedProfile(profile)) => { app.set_account_saved(profile.is_some()); *s.account_ui.saved.borrow_mut() = profile; }
            Ok(Response::Subscriptions(page)) => s.account_ui.publish(&app, page.items.into_iter().map(Item::Channel).collect(), page.next, page.partial),
            Ok(Response::Playlists(page)) => s.account_ui.publish(&app, page.items.into_iter().map(Item::Playlist).collect(), page.next, page.partial),
            Ok(Response::Playlist(contents)) => { s.account_ui.playlist_editable.set(contents.editable); s.account_ui.publish(&app, contents.page.items.into_iter().map(Item::Video).collect(), contents.page.next, contents.page.partial); }
            Ok(Response::SubscriptionState(subscribed)) => app.set_account_status(if subscribed { "YouTube confirms this channel is subscribed." } else { "YouTube confirms this channel is not subscribed." }.into()),
            Ok(Response::Rating(liked)) => {
                let current = s.current_video.borrow();
                if s.account_ui.rating_video.borrow().as_ref().is_some_and(|target| target.matches(current.as_ref().map(|v| &v.id), result.generation)) {
                    app.set_account_liked(liked); app.set_rating_known(true);
                }
            }
            Ok(Response::PlaybackResolved { .. }) => {
                app.set_account_status("An unexpected playback response was rejected.".into());
            }
            Ok(Response::Mutation(outcome)) => {
                let pending = outcome == MutationOutcome::NeedsReconciliation;
                app.set_account_pending(pending); app.set_rating_known(false);
                app.set_account_status(if pending { "The change has an unknown outcome. Check its remote state before another write; it will not be repeated automatically." } else { "YouTube's remote state confirms the change. Refresh the account list to view it." }.into());
            }
            Ok(Response::Disconnected { forget_error }) => {
                app.set_account_forget_needed(forget_error.is_some());
                app.set_account_status(s.account_ui.write_warning.status(forget_error.map(|e| format!("Disconnected, but saved credential removal needs another attempt: {e}")).unwrap_or_else(|| "Disconnected and saved credentials removed. Local collections were kept.".into())));
            }
            Err(error) => {
                if matches!(error, account::WorkerError::Account(serein_youtube::account::AccountError::SessionExpired | serein_youtube::account::AccountError::IdentityNotVerified | serein_youtube::account::AccountError::StaleSession)) {
                    s.account_ui.clear_identity(&app);
                    crate::account_playback::clear(&app, &s);
                }
                if matches!(error, account::WorkerError::Account(serein_youtube::account::AccountError::ReconciliationRequired)) { app.set_account_pending(true); }
                app.set_account_status(s.account_ui.write_warning.status(error.to_string()));
                if kind == PendingKind::Connection { s.account_ui.submit(&app, AccountRequest::InspectSaved); }
            }
        }
    });
    app.on_open_youtube(|| {
        std::thread::spawn(|| {
            let _ = webbrowser::open("https://www.youtube.com");
        });
    });
    app.on_open_export_guide(|| {
        std::thread::spawn(|| {
            let _ = webbrowser::open(
                "https://github.com/yt-dlp/yt-dlp/wiki/Extractors#exporting-youtube-cookies",
            );
        });
    });
    // Reading this marker cannot access a Keychain item or connect an account.
    state.account_ui.submit(app, AccountRequest::InspectSaved);
}

fn is_private_read(result: &Result<Response, account::WorkerError>) -> bool {
    matches!(
        result,
        Ok(Response::Subscriptions(_)
            | Response::Playlists(_)
            | Response::Playlist(_)
            | Response::SubscriptionState(_)
            | Response::Rating(_))
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_uncertain_or_inflight_writes_leave_generic_warning_across_reconnect() {
        let warning = WriteWarning::default();
        warning.current.set(false); // verified result / ordinary account read
        warning.retire(false);
        assert_eq!(warning.status("Disconnected.").as_str(), "Disconnected.");
        warning.current.set(true); // worker reports pending mutation at expiry
        warning.retire(false);
        assert!(!warning.current.get());
        let expired = warning.status("Session expired.");
        assert!(expired.starts_with("Session expired."));
        assert!(expired.contains("Check YouTube before repeating it"));
        let removal_failure = warning.status("Saved credential removal failed.");
        assert!(removal_failure.starts_with("Saved credential removal failed."));
        assert!(removal_failure.contains("unconfirmed outcome"));
        assert_eq!(
            WriteWarning::default().status("New app run.").as_str(),
            "New app run.",
            "warning is session-only, not a persisted reconciliation record"
        );
        // A new connected session has no old pending mutation to reconcile.
        // Keep only the explicit warning; do not pretend that reconnect verified it.
        warning.current.set(false);
        let connected = warning.status("Identity verified.");
        assert!(connected.contains("Reconnecting does not reconcile or replay"));
        let active = WriteWarning::default();
        active.retire(true); // explicit sign-out can interrupt a submitted write
        assert!(
            active
                .status("Disconnected.")
                .contains("unconfirmed outcome")
        );
    }
    #[test]
    fn a_rating_result_cannot_cross_video_or_account_selection() {
        let first = serein_core::VideoId::new("aaaaaaaaaaa").unwrap();
        let second = serein_core::VideoId::new("bbbbbbbbbbb").unwrap();
        let pending = RatingTarget {
            video: first.clone(),
            generation: 7,
        };
        assert!(pending.matches(Some(&first), 7));
        assert!(!pending.matches(Some(&second), 7));
        assert!(!pending.matches(Some(&first), 8));
        assert!(!pending.matches(None, 7));
    }
    #[test]
    fn expired_identity_drops_read_publication_but_keeps_mutation_outcomes_and_errors() {
        assert!(is_private_read(&Ok(Response::Rating(true))));
        assert!(is_private_read(&Ok(Response::SubscriptionState(true))));
        assert!(!is_private_read(&Ok(Response::Mutation(
            MutationOutcome::Verified
        ))));
        assert!(!is_private_read(&Ok(Response::Mutation(
            MutationOutcome::NeedsReconciliation
        ))));
        assert!(!is_private_read(&Err(account::WorkerError::Account(
            serein_youtube::account::AccountError::SessionExpired
        ))));
        assert!(!is_private_read(&Err(account::WorkerError::Account(
            serein_youtube::account::AccountError::ReconciliationRequired
        ))));
    }
}
