//! Explicit, user-driven browser-session account adapter. No credential discovery.
//! All methods that use the network are blocking worker APIs with cancellable I/O.
mod authorization;
mod browser;
mod cookies;
pub use authorization::AccountPlaybackLease;
pub use browser::{
    BrowserKind, BrowserProfile, InstalledBrowser, detect_browsers, import_browser_session,
};
mod ephemeral;
mod http;
mod parser;
pub use cookies::{MAX_COOKIE_BYTES, SessionCookies};
use oxplay_core::{ChannelId, OperationContext, PlaylistId, VideoId, VideoSummary};
use serde_json::{Value, json};
use std::{
    fmt,
    sync::{Arc, atomic::Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountError {
    SecureTemporaryStorage,
    ImportTooLarge,
    InvalidCookieFile,
    AmbiguousCookies,
    MissingSessionCookies,
    SessionExpired,
    ChallengeRequired,
    Cancelled,
    StaleSession,
    Offline,
    ServiceUnavailable,
    Timeout,
    RateLimited,
    RedirectRejected,
    ResponseTooLarge,
    UnsupportedResponse,
    UnsupportedAccount,
    IdentityNotVerified,
    InvalidInput,
    RemoteRejected,
    ReconciliationRequired,
    StateMismatch,
    Busy,
    /// Browser sign-in is not implemented for this platform or browser yet.
    BrowserUnsupported,
    /// The selected browser profile's cookie data could not be read.
    BrowserUnavailable,
    /// The OS denied access to the browser's data or saved keys.
    BrowserPermissionDenied,
}
impl fmt::Display for AccountError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::SecureTemporaryStorage => "A private temporary session file could not be created or cleaned safely. Authenticated extraction is unavailable.",
        Self::ImportTooLarge=>"The selected cookie export exceeds the import limit.",
        Self::InvalidCookieFile=>"The selected file is not a valid supported Netscape cookie export.",
        Self::AmbiguousCookies=>"The export contains duplicate session cookies. Export a single dedicated browser session again.",
        Self::MissingSessionCookies=>"The export has no supported unexpired YouTube session cookies.",
        Self::SessionExpired=>"The YouTube session expired or was rejected. Disconnect and import a fresh export.",
        Self::ChallengeRequired=>"YouTube rejected the account request or requires a browser challenge. Open YouTube directly; this app does not bypass challenges.",
        Self::Cancelled=>"Account operation cancelled.", Self::StaleSession=>"The account session changed. This result was discarded.",
        Self::Offline=>"The account request could not reach YouTube.",
            Self::ServiceUnavailable=>"YouTube is temporarily unavailable. A pending write must be reconciled before retrying.", Self::Timeout=>"The account request timed out. A pending write must be reconciled before retrying.",
        Self::RateLimited=>"YouTube is limiting account requests. Wait before trying again.",
        Self::RedirectRejected=>"An authenticated request attempted a redirect and was stopped.",
        Self::ResponseTooLarge=>"The account response exceeded the safety limit.",
        Self::UnsupportedResponse=>"YouTube returned an account response this version cannot safely interpret.",
        Self::UnsupportedAccount=>"This account or channel-selection configuration is not supported yet.",
        Self::IdentityNotVerified=>"Connect and verify a YouTube identity before accessing account data.",
        Self::InvalidInput=>"The account action has an invalid identifier or stale continuation.",
        Self::RemoteRejected=>"YouTube rejected the requested account action.",
        Self::ReconciliationRequired=>"A previous account action has an unknown outcome. Check its remote state before another write.",
        Self::StateMismatch=>"The remote account state does not match the requested change.",
        Self::Busy=>"Another account operation is active.",
        Self::BrowserUnsupported=>"Signing in from a browser is not supported yet on this platform or for this browser. Use the session file import instead.",
        Self::BrowserUnavailable=>"That browser profile's YouTube sign-in could not be read. Make sure the browser is installed and you are signed in to YouTube in it.",
        Self::BrowserPermissionDenied=>"Oxplay needs permission to read the selected browser's data. For Safari, grant Oxplay Full Disk Access in System Settings > Privacy & Security, then try again. For Chrome and other Chromium browsers, allow the keychain access prompt.",
    })
    }
}
impl std::error::Error for AccountError {}
#[derive(Debug)]
pub enum AuthenticatedResolveError {
    Account(AccountError),
    Resolver(oxplay_core::ProviderError),
}
impl From<AccountError> for AuthenticatedResolveError {
    fn from(error: AccountError) -> Self {
        Self::Account(error)
    }
}
impl fmt::Display for AuthenticatedResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Account(error) => error.fmt(f),
            Self::Resolver(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for AuthenticatedResolveError {}

#[derive(Clone, Default)]
pub struct SessionControl(Arc<authorization::Control>);
impl SessionControl {
    pub fn generation(&self) -> u64 {
        self.0.generation.load(Ordering::Acquire)
    }
    /// Call on the UI thread before sign-out/account switching; active HTTP is aborted.
    pub fn invalidate(&self) -> u64 {
        self.0.invalidate()
    }
    fn check(&self, operation: &OperationContext) -> Result<(), AccountError> {
        if operation.cancel.is_cancelled() {
            return Err(AccountError::Cancelled);
        }
        if self.generation() != operation.session_generation {
            return Err(AccountError::StaleSession);
        }
        Ok(())
    }
}
/// Successful explicit account extraction plus revocable session authority.
/// URLs remain redacted domain values; the lease contains no credentials.
pub struct AuthorizedPlayback {
    pub playback: oxplay_core::ResolvedPlayback,
    pub authorization: AccountPlaybackLease,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capability {
    Unsupported,
    ImplementedUnverified,
    Verified,
}
#[derive(Clone, Debug)]
pub struct AccountCapabilities {
    pub identity: Capability,
    pub subscriptions: Capability,
    pub playlists: Capability,
    pub recommendations: Capability,
    pub subscription_writes: Capability,
    pub likes: Capability,
    pub playlist_writes: Capability,
    pub authenticated_playback: Capability,
    pub channel_switching: Capability,
}
impl Default for AccountCapabilities {
    fn default() -> Self {
        Self {
            identity: Capability::ImplementedUnverified,
            subscriptions: Capability::ImplementedUnverified,
            playlists: Capability::ImplementedUnverified,
            recommendations: Capability::ImplementedUnverified,
            subscription_writes: Capability::ImplementedUnverified,
            likes: Capability::ImplementedUnverified,
            playlist_writes: Capability::ImplementedUnverified,
            authenticated_playback: Capability::ImplementedUnverified,
            channel_switching: Capability::Unsupported,
        }
    }
}
#[derive(Clone)]
pub struct AccountIdentity {
    pub display_name: String,
    pub handle: Option<String>,
    pub selected: bool,
    pub has_channel: bool,
}
#[derive(Clone)]
pub struct ConnectionInfo {
    pub identity: AccountIdentity,
    pub available_identities: Vec<AccountIdentity>,
    pub generation: u64,
    pub capabilities: AccountCapabilities,
}
#[derive(Clone)]
pub struct AccountChannel {
    pub id: ChannelId,
    pub title: String,
}
#[derive(Clone)]
pub struct AccountPlaylist {
    pub id: PlaylistId,
    pub title: String,
    pub editable: Option<bool>,
}
#[derive(Clone)]
pub struct AccountPlaylistItem {
    pub video_id: VideoId,
    pub title: String,
    pub set_video_id: Option<String>,
}
#[derive(Clone, PartialEq, Eq)]
enum PageKind {
    Subscriptions,
    Playlists,
    Playlist(String),
    Recommendations,
}
#[derive(Clone)]
pub struct AccountCursor {
    token: String,
    generation: u64,
    kind: PageKind,
}
pub struct AccountPage<T> {
    pub items: Vec<T>,
    pub next: Option<AccountCursor>,
    pub partial: bool,
}
pub struct PlaylistContents {
    pub page: AccountPage<AccountPlaylistItem>,
    pub editable: bool,
}
/// The connected identity's own rating of a video. YouTube publishes no
/// dislike count; this is only the account's private state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VideoRating {
    #[default]
    None,
    Like,
    Dislike,
}
/// Only construct in response to the user's explicit account action. Import/read
/// operations never call this API and never imply authorization to write.
#[derive(Clone)]
pub enum AccountMutation {
    Subscription {
        channel_id: ChannelId,
        subscribed: bool,
    },
    Rating {
        video_id: VideoId,
        rating: VideoRating,
    },
    AddToPlaylist {
        playlist_id: PlaylistId,
        video_id: VideoId,
    },
    RemoveFromPlaylist {
        playlist_id: PlaylistId,
        set_video_id: String,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MutationOutcome {
    Verified,
    NeedsReconciliation,
}
struct VerifiedSession {
    expired: std::cell::Cell<bool>,
    playback_authorization: std::cell::RefCell<Option<AccountPlaybackLease>>,
    cookies: SessionCookies,
    info: ConnectionInfo,
    account_index: u8,
}
impl VerifiedSession {
    fn expire(&self) {
        self.expired.set(true);
        if let Some(lease) = self.playback_authorization.borrow().as_ref() {
            lease.revoke();
        }
    }
}
impl Drop for VerifiedSession {
    fn drop(&mut self) {
        if let Some(lease) = self.playback_authorization.get_mut().take() {
            lease.revoke();
        }
    }
}
struct PendingMutation {
    mutation: AccountMutation,
    generation: u64,
    baseline_ids: Vec<String>,
}
pub struct AccountClient {
    http: http::Transport,
    control: SessionControl,
    session: Option<VerifiedSession>,
    pending: Option<PendingMutation>,
}
impl AccountClient {
    pub fn new(control: SessionControl) -> Result<Self, AccountError> {
        Ok(Self {
            http: http::Transport::new()?,
            control,
            session: None,
            pending: None,
        })
    }
    /// Cleanup only app-owned stale extractor jars. Call once on an app worker
    /// at startup; errors should be surfaced, never treated as forensic erasure.
    pub fn cleanup_stale_extractor_sessions() -> Result<(), AccountError> {
        ephemeral::cleanup_stale()
    }
    /// Explicit account playback only. Public playback must continue to use the
    /// guest resolver unless the user deliberately requests authenticated access.
    pub fn resolve_authenticated(
        &mut self,
        resolver: &crate::YtDlp,
        id: &VideoId,
        operation: &OperationContext,
    ) -> Result<AuthorizedPlayback, AuthenticatedResolveError> {
        self.resolve_authenticated_with_policy(resolver, id, resolver.resolution, operation)
    }
    pub fn resolve_authenticated_with_policy(
        &mut self,
        resolver: &crate::YtDlp,
        id: &VideoId,
        policy: crate::ResolutionPolicy,
        operation: &OperationContext,
    ) -> Result<AuthorizedPlayback, AuthenticatedResolveError> {
        self.control.check(operation)?;
        if self.http.cooldown_remaining().is_some() {
            return Err(AccountError::RateLimited.into());
        }
        let session = self
            .session
            .as_ref()
            .ok_or(AccountError::IdentityNotVerified)?;
        if session.info.generation != operation.session_generation {
            return Err(AccountError::StaleSession.into());
        }
        if session.expired.get() {
            return Err(AccountError::SessionExpired.into());
        }
        // No reviewed extractor selector binds other account slots or delegated channels.
        if session.account_index != 0 || session.info.available_identities.len() != 1 {
            return Err(AccountError::UnsupportedAccount.into());
        }
        let expiry = match session
            .cookies
            .playback_expiry(std::time::SystemTime::now())
        {
            Ok(expiry) => expiry,
            Err(error) => {
                session.expire();
                return Err(error.into());
            }
        };
        let authorization = if let Some(lease) = session.playback_authorization.borrow().as_ref() {
            lease.clone()
        } else {
            self.control.0.issue(operation.session_generation, expiry)?
        };
        if !authorization.is_valid() {
            session.expire();
            return Err(AccountError::SessionExpired.into());
        }
        *session.playback_authorization.borrow_mut() = Some(authorization.clone());
        let jar = ephemeral::CookieFile::new(&session.cookies)?;
        let result = resolver.resolve_authorized(id, policy, jar.path(), operation, &|| {
            self.control.generation() != operation.session_generation || !authorization.is_valid()
        });
        // Reaping precedes private jar deletion, including cancellation and expiry.
        let cleanup = jar.close();
        self.control.check(operation)?;
        cleanup?;
        if session
            .cookies
            .playback_expiry(std::time::SystemTime::now())
            .is_err()
            || !authorization.is_valid()
        {
            session.expire();
            return Err(AccountError::SessionExpired.into());
        }
        if matches!(result, Err(oxplay_core::ProviderError::RateLimited)) {
            self.http.limit_requests(None);
        }
        let playback = result.map_err(AuthenticatedResolveError::Resolver)?;
        Ok(AuthorizedPlayback {
            playback,
            authorization,
        })
    }
    pub fn cooldown_remaining(&self) -> Option<std::time::Duration> {
        self.http.cooldown_remaining()
    }
    pub fn control(&self) -> SessionControl {
        self.control.clone()
    }
    /// Import is a candidate until a real authenticated identity response succeeds.
    /// Account index selects a Google account slot, not an arbitrary delegated channel.
    pub fn connect(
        &mut self,
        cookies: SessionCookies,
        account_index: u8,
        operation: &OperationContext,
    ) -> Result<ConnectionInfo, AccountError> {
        self.control.check(operation)?;
        if account_index > 9 {
            return Err(AccountError::InvalidInput);
        }
        self.session = None;
        self.pending = None;
        let response=self.http.post(&cookies,account_index,"account/accounts_list",json!({"requestType":"ACCOUNTS_LIST_REQUEST_TYPE_CHANNEL_SWITCHER","callCircumstance":"SWITCHING_USERS_FULL"}),false,&self.control,operation)?;
        let identities = parser::identities(&response)?;
        let selected: Vec<_> = identities.iter().filter(|i| i.selected).collect();
        if selected.len() != 1 {
            return Err(AccountError::UnsupportedAccount);
        }
        let authenticated_playback = if account_index == 0 && identities.len() == 1 {
            Capability::ImplementedUnverified
        } else {
            Capability::Unsupported
        };
        let info = ConnectionInfo {
            identity: selected[0].clone(),
            available_identities: identities,
            generation: operation.session_generation,
            capabilities: AccountCapabilities {
                identity: Capability::Verified,
                authenticated_playback,
                ..AccountCapabilities::default()
            },
        };
        self.control.check(operation)?;
        self.session = Some(VerifiedSession {
            expired: std::cell::Cell::new(false),
            playback_authorization: std::cell::RefCell::new(None),
            cookies,
            info: info.clone(),
            account_index,
        });
        Ok(info)
    }
    /// Invalidate via SessionControl immediately, then call here on the owning worker
    /// to drop account credentials and pending state. Local collections are untouched.
    pub fn disconnect(&mut self) {
        // The UI may already have advanced the generation while this worker was
        // cancelling I/O. Do not invalidate a newly queued import a second time.
        if self
            .session
            .as_ref()
            .is_some_and(|s| s.info.generation == self.control.generation())
        {
            self.control.invalidate();
        }
        self.session = None;
        self.pending = None;
    }
    /// A mutation may have reached YouTube without a verified outcome. This
    /// nonsecret flag survives expiry; it never permits replay or transfers
    /// pending private identifiers to another account/session.
    pub fn has_unconfirmed_mutation(&self) -> bool {
        self.pending.is_some()
    }
    pub fn connection(&self) -> Option<ConnectionInfo> {
        self.session
            .as_ref()
            .filter(|s| s.info.generation == self.control.generation() && !s.expired.get())
            .map(|s| s.info.clone())
    }
    /// Explicit filtered credential export for an OS-protected vault, never ordinary storage.
    pub fn export_for_vault(&self) -> Result<zeroize::Zeroizing<Vec<u8>>, AccountError> {
        let session = self
            .session
            .as_ref()
            .filter(|s| s.info.generation == self.control.generation() && !s.expired.get())
            .ok_or(AccountError::IdentityNotVerified)?;
        Ok(session.cookies.export_for_vault())
    }
    fn request(
        &self,
        path: &str,
        payload: Value,
        tv: bool,
        operation: &OperationContext,
    ) -> Result<Value, AccountError> {
        self.control.check(operation)?;
        let session = self
            .session
            .as_ref()
            .ok_or(AccountError::IdentityNotVerified)?;
        if session.info.generation != operation.session_generation {
            return Err(AccountError::StaleSession);
        }
        if session.expired.get() {
            return Err(AccountError::SessionExpired);
        }
        let result = self.http.post(
            &session.cookies,
            session.account_index,
            path,
            payload,
            tv,
            &self.control,
            operation,
        );
        if matches!(result, Err(AccountError::SessionExpired)) {
            session.expire();
        }
        result
    }
    fn browse(
        &self,
        id: &str,
        cursor: Option<&AccountCursor>,
        kind: &PageKind,
        operation: &OperationContext,
    ) -> Result<Value, AccountError> {
        let payload = if let Some(cursor) = cursor {
            if cursor.generation != operation.session_generation || &cursor.kind != kind {
                return Err(AccountError::InvalidInput);
            }
            json!({"continuation":cursor.token})
        } else {
            json!({"browseId":id})
        };
        self.request("browse", payload, false, operation)
    }
    pub fn subscriptions(
        &mut self,
        cursor: Option<&AccountCursor>,
        operation: &OperationContext,
    ) -> Result<AccountPage<AccountChannel>, AccountError> {
        let value = self.browse("FEchannels", cursor, &PageKind::Subscriptions, operation)?;
        let page = parser::subscriptions(&value, operation.session_generation)?;
        self.control.check(operation)?;
        if let Some(s) = &mut self.session {
            s.info.capabilities.subscriptions = Capability::Verified;
        }
        Ok(page)
    }
    pub fn playlists(
        &mut self,
        cursor: Option<&AccountCursor>,
        operation: &OperationContext,
    ) -> Result<AccountPage<AccountPlaylist>, AccountError> {
        let value = self.browse(
            "FEplaylist_aggregation",
            cursor,
            &PageKind::Playlists,
            operation,
        )?;
        let page = parser::playlists(&value, operation.session_generation)?;
        self.control.check(operation)?;
        if let Some(s) = &mut self.session {
            s.info.capabilities.playlists = Capability::Verified;
        }
        Ok(page)
    }
    /// The signed-in YouTube home feed. Read only on an explicit user action;
    /// promoted items and Shorts shelves are excluded before normalization.
    pub fn recommendations(
        &mut self,
        cursor: Option<&AccountCursor>,
        operation: &OperationContext,
    ) -> Result<AccountPage<VideoSummary>, AccountError> {
        let value = self.browse(
            "FEwhat_to_watch",
            cursor,
            &PageKind::Recommendations,
            operation,
        )?;
        let page = parser::recommendations(&value, operation.session_generation)?;
        self.control.check(operation)?;
        if let Some(s) = &mut self.session {
            s.info.capabilities.recommendations = Capability::Verified;
        }
        Ok(page)
    }
    pub fn playlist(
        &self,
        id: &PlaylistId,
        cursor: Option<&AccountCursor>,
        operation: &OperationContext,
    ) -> Result<PlaylistContents, AccountError> {
        validate_playlist(id)?;
        let value = self.browse(
            &format!("VL{}", id.0),
            cursor,
            &PageKind::Playlist(id.0.clone()),
            operation,
        )?;
        parser::playlist(&value, id, operation.session_generation)
    }
    pub fn subscription_state(
        &self,
        id: &ChannelId,
        operation: &OperationContext,
    ) -> Result<bool, AccountError> {
        validate_channel(id)?;
        let value = self.request("browse", json!({"browseId":id.0}), false, operation)?;
        parser::subscription_state(&value, id)
    }
    pub fn rating(
        &self,
        id: &VideoId,
        operation: &OperationContext,
    ) -> Result<VideoRating, AccountError> {
        let value = self.request("next", json!({"videoId":id.as_str()}), false, operation)?;
        parser::rating(&value)
    }
    /// One POST at most. Unknown outcomes remain pending; never automatically repeat a write.
    pub fn apply_user_mutation(
        &mut self,
        mutation: AccountMutation,
        operation: &OperationContext,
    ) -> Result<MutationOutcome, AccountError> {
        self.control.check(operation)?;
        let session = self
            .session
            .as_ref()
            .ok_or(AccountError::IdentityNotVerified)?;
        if session.info.generation != operation.session_generation {
            return Err(AccountError::StaleSession);
        }
        if session.expired.get() {
            return Err(AccountError::SessionExpired);
        }
        if self.pending.is_some() {
            return Err(AccountError::ReconciliationRequired);
        }
        let (path, payload, tv) = mutation_request(&mutation)?;
        let baseline_ids = match &mutation {
            AccountMutation::AddToPlaylist { playlist_id, .. }
            | AccountMutation::RemoveFromPlaylist { playlist_id, .. } => {
                let (editable, items) = self.playlist_snapshot(playlist_id, operation)?;
                if !editable {
                    return Err(AccountError::RemoteRejected);
                }
                if let AccountMutation::RemoveFromPlaylist { set_video_id, .. } = &mutation
                    && !items
                        .iter()
                        .any(|i| i.set_video_id.as_ref() == Some(set_video_id))
                {
                    return Err(AccountError::StateMismatch);
                }
                items.into_iter().filter_map(|i| i.set_video_id).collect()
            }
            _ => vec![],
        };
        self.pending = Some(PendingMutation {
            mutation,
            generation: operation.session_generation,
            baseline_ids,
        });
        match self.request(path, payload, tv, operation) {
            Ok(value) => {
                if parser::action_rejected(&value) {
                    self.pending = None;
                    return Err(AccountError::RemoteRejected);
                }
            }
            Err(
                error @ (AccountError::RemoteRejected
                | AccountError::SessionExpired
                | AccountError::ChallengeRequired
                | AccountError::RateLimited
                | AccountError::InvalidInput),
            ) => {
                self.pending = None;
                return Err(error);
            }
            Err(AccountError::Cancelled | AccountError::StaleSession) => {
                return Err(AccountError::Cancelled);
            }
            Err(_) => return Ok(MutationOutcome::NeedsReconciliation),
        }
        self.reconcile_pending(operation)
    }
    pub fn reconcile_pending(
        &mut self,
        operation: &OperationContext,
    ) -> Result<MutationOutcome, AccountError> {
        self.control.check(operation)?;
        let pending = self.pending.as_ref().ok_or(AccountError::InvalidInput)?;
        if pending.generation != operation.session_generation {
            return Err(AccountError::StaleSession);
        }
        let matches = match &pending.mutation {
            AccountMutation::Subscription {
                channel_id,
                subscribed,
            } => self
                .subscription_state(channel_id, operation)
                .map(|s| s == *subscribed),
            AccountMutation::Rating { video_id, rating } => {
                self.rating(video_id, operation).map(|s| s == *rating)
            }
            AccountMutation::AddToPlaylist {
                playlist_id,
                video_id,
            } => self
                .playlist_snapshot(playlist_id, operation)
                .map(|(_, items)| {
                    items.iter().any(|i| {
                        i.video_id == *video_id
                            && i.set_video_id
                                .as_ref()
                                .is_some_and(|id| !pending.baseline_ids.contains(id))
                    })
                }),
            AccountMutation::RemoveFromPlaylist {
                playlist_id,
                set_video_id,
            } => self
                .playlist_snapshot(playlist_id, operation)
                .map(|(_, items)| {
                    !items
                        .iter()
                        .any(|i| i.set_video_id.as_ref() == Some(set_video_id))
                }),
        };
        match matches {
            Ok(true) => {
                let mutation = &self
                    .pending
                    .as_ref()
                    .ok_or(AccountError::InvalidInput)?
                    .mutation;
                if let Some(session) = &mut self.session {
                    match mutation {
                        AccountMutation::Subscription { .. } => {
                            session.info.capabilities.subscription_writes = Capability::Verified
                        }
                        AccountMutation::Rating { .. } => {
                            session.info.capabilities.likes = Capability::Verified
                        }
                        _ => session.info.capabilities.playlist_writes = Capability::Verified,
                    }
                }
                self.pending = None;
                Ok(MutationOutcome::Verified)
            }
            Ok(false) => Ok(MutationOutcome::NeedsReconciliation),
            // A write can have succeeded before its verification read loses
            // authentication. Preserve the pending outcome, but surface loss
            // of identity so the UI removes private rows and account playback.
            // Treating it as an ordinary uncertain outcome leaves Connected
            // visible despite request() having expired/revoked the session.
            Err(error @ (AccountError::SessionExpired | AccountError::IdentityNotVerified)) => {
                Err(error)
            }
            Err(AccountError::Cancelled | AccountError::StaleSession) => {
                Err(AccountError::Cancelled)
            }
            Err(_) => Ok(MutationOutcome::NeedsReconciliation),
        }
    }
    fn playlist_snapshot(
        &self,
        id: &PlaylistId,
        operation: &OperationContext,
    ) -> Result<(bool, Vec<AccountPlaylistItem>), AccountError> {
        let mut cursor = None;
        let mut items = Vec::new();
        let mut editable = false;
        for page_index in 0..25 {
            let page = self.playlist(id, cursor.as_ref(), operation)?;
            if page.page.partial
                || page
                    .page
                    .items
                    .iter()
                    .any(|item| item.set_video_id.is_none())
            {
                return Err(AccountError::UnsupportedResponse);
            }
            if page_index == 0 {
                editable = page.editable;
            }
            items.extend(page.page.items);
            if items.len() > 2_000 {
                return Err(AccountError::ResponseTooLarge);
            }
            cursor = page.page.next;
            if cursor.is_none() {
                return Ok((editable, items));
            }
        }
        Err(AccountError::ResponseTooLarge)
    }
}
fn validate_channel(id: &ChannelId) -> Result<(), AccountError> {
    if id.0.starts_with("UC")
        && id.0.len() == 24
        && id
            .0
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
    {
        Ok(())
    } else {
        Err(AccountError::InvalidInput)
    }
}
fn validate_playlist(id: &PlaylistId) -> Result<(), AccountError> {
    if (2..=100).contains(&id.0.len())
        && id
            .0
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
    {
        Ok(())
    } else {
        Err(AccountError::InvalidInput)
    }
}
fn mutation_request(
    mutation: &AccountMutation,
) -> Result<(&'static str, Value, bool), AccountError> {
    Ok(match mutation {
        AccountMutation::Subscription {
            channel_id,
            subscribed,
        } => {
            validate_channel(channel_id)?;
            (
                if *subscribed {
                    "subscription/subscribe"
                } else {
                    "subscription/unsubscribe"
                },
                json!({"channelIds":[channel_id.0],"params":if *subscribed{"EgIIAhgA"}else{"CgIIAhgA"}}),
                false,
            )
        }
        AccountMutation::Rating { video_id, rating } => (
            match rating {
                VideoRating::Like => "like/like",
                VideoRating::Dislike => "like/dislike",
                // Removes either a like or a dislike.
                VideoRating::None => "like/removelike",
            },
            json!({"target":{"videoId":video_id.as_str()}}),
            true,
        ),
        AccountMutation::AddToPlaylist {
            playlist_id,
            video_id,
        } => {
            validate_playlist(playlist_id)?;
            (
                "browse/edit_playlist",
                json!({"playlistId":playlist_id.0,"actions":[{"action":"ACTION_ADD_VIDEO","addedVideoId":video_id.as_str()}]}),
                false,
            )
        }
        AccountMutation::RemoveFromPlaylist {
            playlist_id,
            set_video_id,
        } => {
            validate_playlist(playlist_id)?;
            if set_video_id.is_empty()
                || set_video_id.len() > 256
                || set_video_id
                    .bytes()
                    .any(|c| !c.is_ascii_alphanumeric() && c != b'_' && c != b'-')
            {
                return Err(AccountError::InvalidInput);
            };
            (
                "browse/edit_playlist",
                json!({"playlistId":playlist_id.0,"actions":[{"action":"ACTION_REMOVE_VIDEO","setVideoId":set_video_id}]}),
                false,
            )
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplay_core::CancellationToken;
    fn context() -> OperationContext {
        OperationContext {
            request_id: 1,
            session_generation: 0,
            cancel: CancellationToken::default(),
        }
    }
    fn identity_response() -> Value {
        json!({"accountItem":{"accountName":{"simpleText":"Synthetic identity"},"isSelected":true,"hasChannel":true}})
    }
    fn fixture_client(replies: Vec<Result<Value, AccountError>>) -> AccountClient {
        let mut client = AccountClient::new(SessionControl::default()).unwrap();
        client.http.fixture = Some(http::Fixture {
            replies: std::cell::RefCell::new(replies.into()),
            calls: std::cell::RefCell::new(Vec::new()),
        });
        let cookies=SessionCookies::import_netscape(zeroize::Zeroizing::new(b"# Netscape HTTP Cookie File\n.youtube.com\tTRUE\t/\tTRUE\t0\tSAPISID\tsynthetic\n".to_vec()),0).unwrap();
        client.connect(cookies, 0, &context()).unwrap();
        client
    }
    #[test]
    fn connected_playback_capability_matches_supported_identity_scope() {
        for (index, multiple, expected) in [
            (0, false, Capability::ImplementedUnverified),
            (1, false, Capability::Unsupported),
            (0, true, Capability::Unsupported),
        ] {
            let response = if multiple {
                json!({"items": [identity_response(), {"accountItem": {
                    "accountName": {"simpleText": "Second synthetic identity"},
                    "isSelected": false, "hasChannel": true
                }}]})
            } else {
                identity_response()
            };
            let mut client = AccountClient::new(SessionControl::default()).unwrap();
            client.http.fixture = Some(http::Fixture {
                replies: std::cell::RefCell::new(vec![Ok(response)].into()),
                calls: std::cell::RefCell::new(Vec::new()),
            });
            let cookies = SessionCookies::import_netscape(zeroize::Zeroizing::new(
                b"# Netscape HTTP Cookie File\n.youtube.com\tTRUE\t/\tTRUE\t0\tSAPISID\tsynthetic\n".to_vec()), 0).unwrap();
            let info = client.connect(cookies, index, &context()).unwrap();
            assert_eq!(info.capabilities.identity, Capability::Verified);
            assert_eq!(info.capabilities.authenticated_playback, expected);
            if expected == Capability::Unsupported {
                let resolver =
                    crate::YtDlp::new(crate::test_absolute("/never-spawned-helper")).unwrap();
                assert!(matches!(
                    client.resolve_authenticated(
                        &resolver,
                        &VideoId::new("abcdefghijk").unwrap(),
                        &context()
                    ),
                    Err(AuthenticatedResolveError::Account(
                        AccountError::UnsupportedAccount
                    ))
                ));
            }
        }
    }
    fn subscription_response(subscribed: bool) -> Value {
        json!({"subscribeButtonRenderer":{"channelId":"UCabcdefghijklmnopqrstuv","subscribed":subscribed}})
    }
    fn playlist_response(ids: &[&str]) -> Value {
        json!({"playlistVideoListRenderer":{"playlistId":"PLsynthetic","isEditable":true,"contents":ids.iter().map(|id|json!({"playlistVideoRenderer":{"videoId":"abcdefghijk","setVideoId":id,"title":{"simpleText":"Synthetic"}}})).collect::<Vec<_>>()}})
    }
    #[cfg(unix)]
    #[test]
    fn authenticated_extractor_gets_only_a_protected_cookie_path() {
        use std::os::unix::fs::PermissionsExt;
        let mut client = fixture_client(vec![Ok(identity_response())]);
        let synthetic=SessionCookies::import_netscape(zeroize::Zeroizing::new(b"# Netscape HTTP Cookie File\n.youtube.com\tTRUE\t/\tTRUE\t0\tSAPISID\tsynthetic-only\n".to_vec()),0).unwrap();
        let fixture_files = ephemeral::CookieFile::new(&synthetic).unwrap();
        let binary = fixture_files
            .path()
            .parent()
            .unwrap()
            .join("synthetic-helper");
        let script = r#"#!/bin/sh
printf x >> "$0.invocations"
cookie=
policy=
previous=
for argument do
  if [ "$previous" = "--cookies" ]; then cookie="$argument"; fi
  case "$argument" in *synthetic-only*) exit 91;; *'height<=720'*) policy=yes;; esac
  previous="$argument"
done
[ -n "$cookie" ] && [ -f "$cookie" ] || exit 92
[ "$policy" = yes ] || exit 93
printf '%s' '{"id":"abcdefghijk","title":"Synthetic helper video","url":"https://r1.googlevideo.com/video?expire=9999999999","protocol":"https","vcodec":"avc1","acodec":"aac"}'
"#;
        std::fs::write(&binary, script).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let resolver = crate::YtDlp::new(&binary).unwrap();
        let invocations = binary.with_file_name("synthetic-helper.invocations");
        let mut unverified = AccountClient::new(SessionControl::default()).unwrap();
        assert!(matches!(
            unverified.resolve_authenticated(
                &resolver,
                &VideoId::new("abcdefghijk").unwrap(),
                &context()
            ),
            Err(AuthenticatedResolveError::Account(
                AccountError::IdentityNotVerified
            ))
        ));
        assert!(
            !invocations.exists(),
            "unverified identity must never launch the helper"
        );
        let item = client
            .resolve_authenticated_with_policy(
                &resolver,
                &VideoId::new("abcdefghijk").unwrap(),
                crate::ResolutionPolicy {
                    max_height: 720,
                    prefer_h264: true,
                },
                &context(),
            )
            .unwrap();
        assert!(!item.playback.guest);
        assert!(item.authorization.is_valid());
        assert_eq!(item.playback.session_generation, 0);
        assert_eq!(item.playback.video.title, "Synthetic helper video");
        let replacement = client
            .resolve_authenticated_with_policy(
                &resolver,
                &VideoId::new("abcdefghijk").unwrap(),
                crate::ResolutionPolicy {
                    max_height: 720,
                    prefer_h264: true,
                },
                &context(),
            )
            .unwrap();
        assert!(
            item.authorization.is_valid(),
            "replacement must preserve playing authority"
        );
        assert!(replacement.authorization.is_valid());
        assert_eq!(
            std::fs::read(&invocations).unwrap(),
            b"xx",
            "explicit repeat performs fresh extraction"
        );
        client.http.limit_requests(Some("120"));
        assert!(client.cooldown_remaining().unwrap() > std::time::Duration::from_secs(100));
        assert!(matches!(
            client.resolve_authenticated(
                &resolver,
                &VideoId::new("abcdefghijk").unwrap(),
                &context()
            ),
            Err(AuthenticatedResolveError::Account(
                AccountError::RateLimited
            ))
        ));
        assert_eq!(
            std::fs::read(&invocations).unwrap(),
            b"xx",
            "same client preserves server cooldown across explicit retries"
        );
        client.disconnect();
        assert!(!item.authorization.is_valid());
        assert!(!replacement.authorization.is_valid());
        assert!(matches!(
            client.resolve_authenticated(
                &resolver,
                &VideoId::new("abcdefghijk").unwrap(),
                &context()
            ),
            Err(AuthenticatedResolveError::Account(
                AccountError::StaleSession
            ))
        ));
        assert_eq!(
            std::fs::read(&invocations).unwrap(),
            b"xx",
            "sign-out rejects stale restart before helper launch"
        );
        std::fs::remove_file(invocations).unwrap();
        std::fs::remove_file(binary).unwrap();
        fixture_files.close().unwrap();
    }
    #[test]
    fn retained_cookie_expiry_rejects_auth_extraction_before_helper_start() {
        let mut client = fixture_client(vec![Ok(identity_response())]);
        client.session.as_mut().unwrap().cookies = SessionCookies::import_netscape(
            zeroize::Zeroizing::new(b"# Netscape HTTP Cookie File\n.youtube.com\tTRUE\t/\tTRUE\t1\tSAPISID\tsynthetic-expired\n".to_vec()), 0).unwrap();
        let resolver =
            crate::YtDlp::new(crate::test_absolute("/never-spawned-expired-helper")).unwrap();
        assert!(matches!(
            client.resolve_authenticated(
                &resolver,
                &VideoId::new("abcdefghijk").unwrap(),
                &context()
            ),
            Err(AuthenticatedResolveError::Account(
                AccountError::SessionExpired
            ))
        ));
        assert!(client.connection().is_none());
        assert_eq!(
            client.control.generation(),
            0,
            "terminal response must remain current"
        );
    }
    #[test]
    fn provider_expiry_and_client_drop_revoke_playback_without_generation_loss() {
        let mut client = fixture_client(vec![
            Ok(identity_response()),
            Err(AccountError::SessionExpired),
        ]);
        let lease = client.control.0.issue(0, None).unwrap();
        *client
            .session
            .as_ref()
            .unwrap()
            .playback_authorization
            .borrow_mut() = Some(lease.clone());
        assert!(matches!(
            client.subscriptions(None, &context()),
            Err(AccountError::SessionExpired)
        ));
        assert!(!lease.is_valid());
        assert_eq!(client.control.generation(), 0);
        let client = fixture_client(vec![Ok(identity_response())]);
        let lease = client.control.0.issue(0, None).unwrap();
        *client
            .session
            .as_ref()
            .unwrap()
            .playback_authorization
            .borrow_mut() = Some(lease.clone());
        drop(client);
        assert!(!lease.is_valid());
    }
    #[test]
    fn nondefault_account_slot_cannot_leak_into_an_unbound_extractor_session() {
        let mut client = fixture_client(vec![Ok(identity_response())]);
        client.session.as_mut().unwrap().account_index = 1;
        let resolver = crate::YtDlp::new(crate::test_absolute("/never-spawned-helper")).unwrap();
        assert!(matches!(
            client.resolve_authenticated(
                &resolver,
                &VideoId::new("abcdefghijk").unwrap(),
                &context()
            ),
            Err(AuthenticatedResolveError::Account(
                AccountError::UnsupportedAccount
            ))
        ));
    }
    #[test]
    fn dislike_and_removal_are_single_writes_verified_by_a_rating_reread() {
        let status = |value: &str| {
            Ok(json!({"likeButtonViewModel": {"likeStatusEntity": {"likeStatus": value}}}))
        };
        let mut client = fixture_client(vec![
            Ok(identity_response()),
            Ok(json!({})),
            status("DISLIKE"),
            Ok(json!({})),
            status("LIKE"),
        ]);
        let video = VideoId::new("abcdefghijk").unwrap();
        assert_eq!(
            client.apply_user_mutation(
                AccountMutation::Rating {
                    video_id: video.clone(),
                    rating: VideoRating::Dislike,
                },
                &context()
            ),
            Ok(MutationOutcome::Verified)
        );
        assert_eq!(
            client.connection().unwrap().capabilities.likes,
            Capability::Verified
        );
        // A reread that disagrees stays pending; it is never retried.
        assert_eq!(
            client.apply_user_mutation(
                AccountMutation::Rating {
                    video_id: video,
                    rating: VideoRating::None,
                },
                &context()
            ),
            Ok(MutationOutcome::NeedsReconciliation)
        );
        assert!(client.has_unconfirmed_mutation());
        assert_eq!(
            &*client.http.fixture.as_ref().unwrap().calls.borrow(),
            &[
                "account/accounts_list",
                "like/dislike",
                "next",
                "like/removelike",
                "next"
            ]
        );
    }
    #[test]
    fn writes_require_verified_identity_before_creating_pending_state() {
        let mut client = AccountClient::new(SessionControl::default()).unwrap();
        let mutation = AccountMutation::Rating {
            video_id: VideoId::new("abcdefghijk").unwrap(),
            rating: VideoRating::Like,
        };
        assert_eq!(
            client.apply_user_mutation(mutation, &context()),
            Err(AccountError::IdentityNotVerified)
        );
        assert!(client.pending.is_none());
    }
    #[test]
    fn expired_session_loses_connected_capabilities_and_vault_export() {
        let mut client = fixture_client(vec![
            Ok(identity_response()),
            Err(AccountError::SessionExpired),
        ]);
        assert!(matches!(
            client.subscriptions(None, &context()),
            Err(AccountError::SessionExpired)
        ));
        assert!(client.connection().is_none());
        assert!(client.export_for_vault().is_err());
        assert!(matches!(
            client.subscriptions(None, &context()),
            Err(AccountError::SessionExpired)
        ));
        assert_eq!(
            client.http.fixture.as_ref().unwrap().calls.borrow().len(),
            2
        );
    }
    #[test]
    fn expired_reconciliation_revokes_identity_and_preserves_unknown_write_without_replay() {
        for write_reply in [Ok(json!({})), Err(AccountError::Timeout)] {
            let uncertain_submission = write_reply.is_err();
            let mut client = fixture_client(vec![
                Ok(identity_response()),
                write_reply,
                Err(AccountError::SessionExpired),
            ]);
            let lease = client.control.0.issue(0, None).unwrap();
            *client
                .session
                .as_ref()
                .unwrap()
                .playback_authorization
                .borrow_mut() = Some(lease.clone());
            let mutation = AccountMutation::Rating {
                video_id: VideoId::new("abcdefghijk").unwrap(),
                rating: VideoRating::Like,
            };
            let submitted = client.apply_user_mutation(mutation.clone(), &context());
            if uncertain_submission {
                assert_eq!(submitted, Ok(MutationOutcome::NeedsReconciliation));
                assert_eq!(
                    client.reconcile_pending(&context()),
                    Err(AccountError::SessionExpired)
                );
            } else {
                assert_eq!(submitted, Err(AccountError::SessionExpired));
            }
            assert!(
                client.has_unconfirmed_mutation(),
                "expiry must not claim the write failed or succeeded"
            );
            assert!(client.connection().is_none());
            assert!(client.export_for_vault().is_err());
            assert!(
                !lease.is_valid(),
                "a previously issued media authority must be revoked"
            );
            assert_eq!(
                client.control.generation(),
                0,
                "the terminal expiry result must remain publishable"
            );
            assert_eq!(
                client.reconcile_pending(&context()),
                Err(AccountError::SessionExpired)
            );
            assert_eq!(
                client.apply_user_mutation(mutation, &context()),
                Err(AccountError::SessionExpired)
            );
            assert_eq!(
                &*client.http.fixture.as_ref().unwrap().calls.borrow(),
                &["account/accounts_list", "like/like", "next"]
            );
        }
    }
    #[test]
    fn reconciliation_without_identity_is_terminal_but_transient_read_failure_is_not() {
        let mut client = fixture_client(vec![
            Ok(identity_response()),
            Err(AccountError::Timeout),
            Err(AccountError::Offline),
        ]);
        let mutation = AccountMutation::Rating {
            video_id: VideoId::new("abcdefghijk").unwrap(),
            rating: VideoRating::Like,
        };
        assert_eq!(
            client.apply_user_mutation(mutation.clone(), &context()),
            Ok(MutationOutcome::NeedsReconciliation)
        );
        assert_eq!(
            client.reconcile_pending(&context()),
            Ok(MutationOutcome::NeedsReconciliation)
        );
        assert!(client.connection().is_some());
        assert_eq!(
            client.apply_user_mutation(mutation, &context()),
            Err(AccountError::ReconciliationRequired)
        );
        client.session.take();
        assert_eq!(
            client.reconcile_pending(&context()),
            Err(AccountError::IdentityNotVerified)
        );
        assert!(client.pending.is_some());
        assert_eq!(
            &*client.http.fixture.as_ref().unwrap().calls.borrow(),
            &["account/accounts_list", "like/like", "next"]
        );
    }
    #[test]
    fn expired_candidate_never_becomes_connected() {
        let mut client = AccountClient::new(SessionControl::default()).unwrap();
        client.http.fixture = Some(http::Fixture {
            replies: std::cell::RefCell::new(vec![Err(AccountError::SessionExpired)].into()),
            calls: std::cell::RefCell::new(Vec::new()),
        });
        let cookies=SessionCookies::import_netscape(zeroize::Zeroizing::new(b"# Netscape HTTP Cookie File\n.youtube.com\tTRUE\t/\tTRUE\t0\tSAPISID\tsynthetic\n".to_vec()),0).unwrap();
        assert!(matches!(
            client.connect(cookies, 0, &context()),
            Err(AccountError::SessionExpired)
        ));
        assert!(client.connection().is_none());
        assert!(client.export_for_vault().is_err());
    }
    #[test]
    fn timed_out_write_cannot_repeat_before_remote_reconciliation() {
        let mut client = fixture_client(vec![
            Ok(identity_response()),
            Err(AccountError::Timeout),
            Ok(subscription_response(true)),
        ]);
        let mutation = AccountMutation::Subscription {
            channel_id: ChannelId("UCabcdefghijklmnopqrstuv".into()),
            subscribed: true,
        };
        assert_eq!(
            client
                .apply_user_mutation(mutation.clone(), &context())
                .unwrap(),
            MutationOutcome::NeedsReconciliation
        );
        assert_eq!(
            client.apply_user_mutation(mutation, &context()),
            Err(AccountError::ReconciliationRequired)
        );
        assert_eq!(
            client.reconcile_pending(&context()).unwrap(),
            MutationOutcome::Verified
        );
        let calls = client.http.fixture.as_ref().unwrap().calls.borrow();
        assert_eq!(
            calls
                .iter()
                .filter(|path| path.as_str() == "subscription/subscribe")
                .count(),
            1
        );
        assert_eq!(
            client
                .connection()
                .unwrap()
                .capabilities
                .subscription_writes,
            Capability::Verified
        );
    }
    #[test]
    fn playlist_add_reconciliation_requires_a_new_set_item_identity() {
        let mut client = fixture_client(vec![
            Ok(identity_response()),
            Ok(playlist_response(&["existing"])),
            Err(AccountError::Timeout),
            Ok(playlist_response(&["existing"])),
            Ok(playlist_response(&["existing", "new_item"])),
        ]);
        let mutation = AccountMutation::AddToPlaylist {
            playlist_id: PlaylistId("PLsynthetic".into()),
            video_id: VideoId::new("abcdefghijk").unwrap(),
        };
        assert_eq!(
            client.apply_user_mutation(mutation, &context()).unwrap(),
            MutationOutcome::NeedsReconciliation
        );
        assert_eq!(
            client.reconcile_pending(&context()).unwrap(),
            MutationOutcome::NeedsReconciliation
        );
        assert_eq!(
            client.reconcile_pending(&context()).unwrap(),
            MutationOutcome::Verified
        );
    }
    #[test]
    fn disconnect_blocks_stale_reads_before_transport_and_removes_credentials() {
        let mut client = fixture_client(vec![Ok(identity_response())]);
        let disconnected_generation = client.control().invalidate();
        assert!(matches!(
            client.subscriptions(None, &context()),
            Err(AccountError::StaleSession)
        ));
        assert!(client.connection().is_none());
        assert!(client.export_for_vault().is_err());
        client.disconnect();
        assert_eq!(client.control().generation(), disconnected_generation);
        assert!(client.session.is_none());
        assert_eq!(
            client.http.fixture.as_ref().unwrap().calls.borrow().len(),
            1
        );
    }
    // TEST FIXTURE: synthetic home-feed shape; no real account data.
    fn recommendation_response(id: &str, token: Option<&str>) -> Value {
        let mut contents = vec![json!({"richItemRenderer":{"content":{"videoRenderer":{
            "videoId": id, "title": {"simpleText": "Synthetic recommendation"}}}}})];
        if let Some(token) = token {
            contents.push(json!({"continuationItemRenderer":{"continuationEndpoint":{"continuationCommand":{"token":token}}}}));
        }
        json!({"richGridRenderer":{"contents":contents}})
    }
    #[test]
    fn recommendations_use_the_verified_session_and_scope_their_cursor() {
        let mut client = fixture_client(vec![
            Ok(identity_response()),
            Ok(recommendation_response(
                "abcdefghijk",
                Some("synthetic-token"),
            )),
            Ok(
                json!({"channelRenderer":{"channelId":"UCabcdefghijklmnopqrstuv"},
                "continuationItemRenderer":{"continuationEndpoint":{"continuationCommand":{"token":"synthetic-sub"}}}}),
            ),
            Ok(recommendation_response("bcdefghijkl", None)),
        ]);
        assert_eq!(
            client.connection().unwrap().capabilities.recommendations,
            Capability::ImplementedUnverified
        );
        let first = client.recommendations(None, &context()).unwrap();
        assert_eq!(first.items[0].id.as_str(), "abcdefghijk");
        assert_eq!(
            client.connection().unwrap().capabilities.recommendations,
            Capability::Verified
        );
        let subscriptions = client.subscriptions(None, &context()).unwrap();
        // A cursor from another collection cannot continue the home feed.
        assert_eq!(
            client
                .recommendations(subscriptions.next.as_ref(), &context())
                .err(),
            Some(AccountError::InvalidInput)
        );
        let second = client
            .recommendations(first.next.as_ref(), &context())
            .unwrap();
        assert_eq!(second.items[0].id.as_str(), "bcdefghijkl");
        assert!(second.next.is_none());
        assert_eq!(
            &*client.http.fixture.as_ref().unwrap().calls.borrow(),
            &["account/accounts_list", "browse", "browse", "browse"]
        );
        // Sign-out invalidates before transport; the retained cursor is stale.
        client.control().invalidate();
        assert_eq!(
            client.recommendations(None, &context()).err(),
            Some(AccountError::StaleSession)
        );
        let later = OperationContext {
            request_id: 2,
            session_generation: client.control().generation(),
            cancel: CancellationToken::default(),
        };
        assert!(client.recommendations(first.next.as_ref(), &later).is_err());
        assert_eq!(
            client.http.fixture.as_ref().unwrap().calls.borrow().len(),
            4
        );
    }
    #[test]
    fn guest_client_and_expired_session_never_read_recommendations() {
        let mut guest = AccountClient::new(SessionControl::default()).unwrap();
        assert_eq!(
            guest.recommendations(None, &context()).err(),
            Some(AccountError::IdentityNotVerified)
        );
        let mut client = fixture_client(vec![
            Ok(identity_response()),
            Ok(json!({"responseContext":{"mainAppWebResponseContext":{"loggedOut":true}}})),
        ]);
        assert_eq!(
            client.recommendations(None, &context()).err(),
            Some(AccountError::SessionExpired)
        );
        assert!(client.connection().is_none());
        assert_eq!(
            client.recommendations(None, &context()).err(),
            Some(AccountError::SessionExpired)
        );
        assert_eq!(
            client.http.fixture.as_ref().unwrap().calls.borrow().len(),
            2
        );
    }
    #[test]
    fn signout_invalidates_all_prior_contexts() {
        let control = SessionControl::default();
        let op = OperationContext {
            request_id: 1,
            session_generation: 0,
            cancel: CancellationToken::default(),
        };
        assert!(control.check(&op).is_ok());
        control.invalidate();
        assert_eq!(control.check(&op), Err(AccountError::StaleSession));
    }
    #[test]
    fn mutation_payload_is_typed_and_does_not_retry() {
        let video = VideoId::new("abcdefghijk").unwrap();
        for (rating, expected) in [
            (VideoRating::None, "like/removelike"),
            (VideoRating::Like, "like/like"),
            (VideoRating::Dislike, "like/dislike"),
        ] {
            let (path, payload, tv) = mutation_request(&AccountMutation::Rating {
                video_id: video.clone(),
                rating,
            })
            .unwrap();
            assert_eq!(path, expected);
            assert_eq!(payload["target"]["videoId"], "abcdefghijk");
            assert!(tv);
        }
        assert!(
            mutation_request(&AccountMutation::Subscription {
                channel_id: ChannelId("file:///bad".into()),
                subscribed: true
            })
            .is_err()
        );
    }
}
