// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit account actions on one sleeping worker. No browser discovery, automatic
//! connection, unsolicited account writes, or credentials in UI responses.
use crate::resolver::SharedResolver;
use oxplay_core::{
    CancellationToken, ChannelId, OperationContext, PlaylistId, ProviderError, VideoId,
    VideoSummary,
};
use oxplay_storage::vault::{ProtectedSessionStore, SessionProfile, VaultError};
use oxplay_youtube::ResolutionPolicy;
use oxplay_youtube::account::{
    AccountChannel, AccountClient, AccountCursor, AccountError, AccountMutation, AccountPage,
    AccountPlaylist, AuthenticatedResolveError, AuthorizedPlayback, BrowserKind, ConnectionInfo,
    MAX_COOKIE_BYTES, MutationOutcome, PlaylistContents, SessionControl, SessionCookies,
    import_browser_session,
};
use std::{
    fmt,
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};
use zeroize::Zeroizing;

/// Nonsecret reference only. Possessing this value does not authorize reconnect.
#[derive(Clone)]
pub struct SavedProfile {
    pub profile: SessionProfile,
    pub account_index: u8,
}

pub enum AccountRequest {
    /// The caller must first obtain explicit import/risk consent and a user-selected file.
    Import {
        path: PathBuf,
        account_index: u8,
        remember: bool,
    },
    /// Sign in by reading ONLY the YouTube/Google session cookies of one
    /// user-selected installed browser profile. Requires the same explicit
    /// risk consent as file import; runs the identical verification/storage path.
    ImportBrowser {
        browser: BrowserKind,
        profile: String,
        account_index: u8,
        remember: bool,
    },
    /// Explicit Reconnect, or the single launch-time restore of a session the
    /// user chose to remember (never for finite diagnostics or after failure).
    Reconnect(SavedProfile),
    /// Reads a bounded nonsecret marker and cleans stale app-owned extractor jars
    /// once; never opens Keychain or the network. Call during worker startup.
    InspectSaved,
    Subscriptions(Option<AccountCursor>),
    Playlists(Option<AccountCursor>),
    Playlist(PlaylistId, Option<AccountCursor>),
    /// Signed-in YouTube home feed. Submit only for explicit Home navigation/Refresh.
    Recommendations(Option<AccountCursor>),
    SubscriptionState(ChannelId),
    Rating(VideoId),
    /// Submit only for the corresponding explicit UI action; never retry automatically.
    Mutate(AccountMutation),
    Reconcile,
    /// Only an explicit authenticated-playback action may submit this request.
    ResolvePlayback {
        id: VideoId,
        policy: ResolutionPolicy,
        selection_generation: u64,
    },
}

pub enum Persistence {
    SessionOnly,
    Remembered(SavedProfile),
    /// Identity was verified but persistence failed. Display this; never imply it was saved.
    SaveFailed(WorkerError),
}

pub enum Response {
    Connected {
        connection: ConnectionInfo,
        persistence: Persistence,
    },
    SavedProfile(Option<SavedProfile>),
    Subscriptions(AccountPage<AccountChannel>),
    Playlists(AccountPage<AccountPlaylist>),
    Playlist(PlaylistContents),
    Recommendations(AccountPage<VideoSummary>),
    SubscriptionState(bool),
    Rating(oxplay_youtube::account::VideoRating),
    Mutation(MutationOutcome),
    PlaybackResolved {
        playback: Box<AuthorizedPlayback>,
        selection_generation: u64,
    },
    /// Authentication is already invalidated even when local deletion needs retry.
    Disconnected {
        forget_error: Option<WorkerError>,
    },
}
pub struct AccountResponse {
    pub request_id: u64,
    pub generation: u64,
    /// Present for playback failures as well as successes, allowing precise stale
    /// selection rejection without dropping unrelated account-operation responses.
    pub playback_selection: Option<u64>,
    /// Latest verified identity/capabilities; None after expiry or disconnection.
    pub connection: Option<ConnectionInfo>,
    /// Generic unknown-write warning; contains no account/playlist/video data.
    pub unconfirmed_mutation: bool,
    /// Nonsecret remaining provider cooldown, measured after this operation.
    pub retry_after: Option<std::time::Duration>,
    pub result: Result<Response, WorkerError>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerError {
    Account(AccountError),
    Vault(VaultError),
    Resolver(ProviderError),
    InvalidFile,
    NoSavedProfile,
    Unavailable,
}
impl fmt::Display for WorkerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Account(error) => error.fmt(f),
            Self::Vault(error) => error.fmt(f),
            Self::Resolver(error) => error.fmt(f),
            Self::InvalidFile => f.write_str("Choose a readable regular Netscape cookie export. No browser profiles are inspected."),
            Self::NoSavedProfile => f.write_str("No saved session exists. Explicitly import a new browser-session export."),
            Self::Unavailable => f.write_str("The account worker is unavailable."),
        }
    }
}
impl From<AccountError> for WorkerError {
    fn from(value: AccountError) -> Self {
        Self::Account(value)
    }
}
impl From<VaultError> for WorkerError {
    fn from(value: VaultError) -> Self {
        Self::Vault(value)
    }
}
impl From<AuthenticatedResolveError> for WorkerError {
    fn from(value: AuthenticatedResolveError) -> Self {
        match value {
            AuthenticatedResolveError::Account(error) => Self::Account(error),
            AuthenticatedResolveError::Resolver(error) => Self::Resolver(error),
        }
    }
}

struct Work {
    operation: OperationContext,
    action: Action,
}
enum Action {
    Request(AccountRequest),
    Disconnect,
}
impl Action {
    fn playback_selection(&self) -> Option<u64> {
        match self {
            Self::Request(AccountRequest::ResolvePlayback {
                selection_generation,
                ..
            }) => Some(*selection_generation),
            _ => None,
        }
    }
}
struct Active {
    operation: OperationContext,
    playback_selection: Option<u64>,
}
#[derive(Default)]
struct State {
    next_id: u64,
    pending: Option<Work>,
    active: Option<Active>,
    result: Option<AccountResponse>,
    closing: bool,
}
struct Shared {
    state: Mutex<State>,
    ready: Condvar,
    control: SessionControl,
}
pub struct Worker {
    shared: Arc<Shared>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Worker {
    /// `directory` is an application-owned vault directory. Constructor performs no I/O.
    #[cfg(test)]
    pub fn new(directory: PathBuf, wake: impl Fn() + Send + 'static) -> Self {
        Self::start(directory, wake, || {})
    }
    pub fn with_resolver(
        directory: PathBuf,
        resolver: SharedResolver,
        wake: impl Fn() + Send + 'static,
    ) -> Self {
        Self::start_with_resolver(directory, Some(resolver), wake, || {})
    }
    #[cfg(test)]
    fn start(
        directory: PathBuf,
        wake: impl Fn() + Send + 'static,
        started: impl FnOnce() + Send + 'static,
    ) -> Self {
        Self::start_with_resolver(directory, None, wake, started)
    }
    fn start_with_resolver(
        directory: PathBuf,
        resolver: Option<SharedResolver>,
        wake: impl Fn() + Send + 'static,
        started: impl FnOnce() + Send + 'static,
    ) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            ready: Condvar::new(),
            control: SessionControl::default(),
        });
        let background = shared.clone();
        let thread = thread::spawn(move || {
            started();
            let mut engine = Engine {
                directory,
                client: None,
                saved: None,
                directory_lock: None,
                cleanup_checked: false,
                control: background.control.clone(),
                resolver,
            };
            loop {
                let work = {
                    let Ok(mut state) = background.state.lock() else {
                        break;
                    };
                    while state.pending.is_none() && !state.closing {
                        state = match background.ready.wait(state) {
                            Ok(state) => state,
                            Err(_) => return,
                        };
                    }
                    if state.closing
                        && !matches!(
                            state.pending.as_ref().map(|work| &work.action),
                            Some(Action::Disconnect)
                        )
                    {
                        break;
                    }
                    let work = state.pending.take().expect("pending operation");
                    state.active = Some(Active {
                        operation: work.operation.clone(),
                        playback_selection: work.action.playback_selection(),
                    });
                    work
                };
                let playback_selection = work.action.playback_selection();
                let result = match work.action {
                    Action::Request(request) => engine.handle(request, &work.operation),
                    Action::Disconnect => {
                        if let Some(client) = engine.client.as_mut() {
                            client.disconnect();
                        }
                        Ok(Response::Disconnected {
                            forget_error: engine.forget().err(),
                        })
                    }
                };
                let Ok(mut state) = background.state.lock() else {
                    break;
                };
                state.active = None;
                if !state.closing && current(&background.control, &work.operation).is_ok() {
                    state.result = Some(AccountResponse {
                        request_id: work.operation.request_id,
                        generation: work.operation.session_generation,
                        playback_selection,
                        connection: engine.client.as_ref().and_then(AccountClient::connection),
                        unconfirmed_mutation: engine
                            .client
                            .as_ref()
                            .is_some_and(AccountClient::has_unconfirmed_mutation),
                        retry_after: engine
                            .client
                            .as_ref()
                            .and_then(AccountClient::cooldown_remaining),
                        result,
                    });
                    drop(state);
                    wake();
                }
            }
            // Dropping the client clears in-memory credentials; remembered sessions stay
            // encrypted across ordinary application shutdown, until explicit disconnect.
        });
        Self {
            shared,
            thread: Some(thread),
        }
    }
    pub fn control(&self) -> SessionControl {
        self.shared.control.clone()
    }
    /// One bounded operation/result slot. A full slot is visible Busy, never a dropped write.
    pub fn submit(&self, request: AccountRequest) -> Result<u64, WorkerError> {
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| WorkerError::Unavailable)?;
        if state.closing {
            return Err(WorkerError::Unavailable);
        }
        if state.pending.is_some() || state.active.is_some() || state.result.is_some() {
            return Err(AccountError::Busy.into());
        }
        if matches!(
            request,
            AccountRequest::Import { .. }
                | AccountRequest::ImportBrowser { .. }
                | AccountRequest::Reconnect(_)
        ) {
            self.shared.control.invalidate();
        }
        let operation = new_operation(&mut state, self.shared.control.generation());
        let id = operation.request_id;
        state.pending = Some(Work {
            operation,
            action: Action::Request(request),
        });
        self.shared.ready.notify_one();
        Ok(id)
    }
    /// Cancel only authenticated extraction. A cancelled active job retains its
    /// capacity until its helper has been reaped; unrelated writes/reads and their
    /// results remain intact. Session authorization itself is not revoked.
    pub fn cancel_playback(&self) -> Result<(), WorkerError> {
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| WorkerError::Unavailable)?;
        if let Some(active) = &state.active
            && active.playback_selection.is_some()
        {
            active.operation.cancel.cancel();
        }
        if state
            .pending
            .as_ref()
            .is_some_and(|work| work.action.playback_selection().is_some())
            && let Some(work) = state.pending.take()
        {
            work.operation.cancel.cancel();
        }
        if state
            .result
            .as_ref()
            .is_some_and(|result| result.playback_selection.is_some())
        {
            state.result = None;
        }
        Ok(())
    }
    /// Invalidation is synchronous, before waiting for network/file/Keychain teardown.
    /// Pending commands and undelivered account data are discarded. Always allowed while busy.
    pub fn disconnect(&self) -> Result<u64, WorkerError> {
        let generation = self.shared.control.invalidate();
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| WorkerError::Unavailable)?;
        if state.closing {
            return Err(WorkerError::Unavailable);
        }
        if let Some(active) = &state.active {
            active.operation.cancel.cancel();
        }
        state.pending = None;
        state.result = None;
        let operation = new_operation(&mut state, generation);
        let id = operation.request_id;
        state.pending = Some(Work {
            operation,
            action: Action::Disconnect,
        });
        self.shared.ready.notify_one();
        Ok(id)
    }
    pub fn take(&self) -> Option<AccountResponse> {
        let result = self.shared.state.lock().ok()?.result.take()?;
        (result.generation == self.shared.control.generation()).then_some(result)
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.shared.control.invalidate();
        if let Ok(mut state) = self.shared.state.lock() {
            state.closing = true;
            if let Some(active) = &state.active {
                active.operation.cancel.cancel();
            }
            if !matches!(
                state.pending.as_ref().map(|work| &work.action),
                Some(Action::Disconnect)
            ) {
                state.pending = None;
            }
            state.result = None;
            self.shared.ready.notify_one();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn new_operation(state: &mut State, generation: u64) -> OperationContext {
    state.next_id += 1;
    OperationContext {
        request_id: state.next_id,
        session_generation: generation,
        cancel: CancellationToken::default(),
    }
}
fn current(control: &SessionControl, operation: &OperationContext) -> Result<(), WorkerError> {
    if operation.cancel.is_cancelled() {
        return Err(AccountError::Cancelled.into());
    }
    if control.generation() != operation.session_generation {
        return Err(AccountError::StaleSession.into());
    }
    Ok(())
}

struct Engine {
    directory: PathBuf,
    client: Option<AccountClient>,
    saved: Option<SavedProfile>,
    directory_lock: Option<File>,
    cleanup_checked: bool,
    control: SessionControl,
    resolver: Option<SharedResolver>,
}
impl Engine {
    fn client(&mut self) -> Result<&mut AccountClient, WorkerError> {
        if self.client.is_none() {
            self.client = Some(AccountClient::new(self.control.clone())?);
        }
        Ok(self.client.as_mut().expect("initialized account client"))
    }
    fn handle(
        &mut self,
        request: AccountRequest,
        operation: &OperationContext,
    ) -> Result<Response, WorkerError> {
        current(&self.control, operation)?;
        match request {
            AccountRequest::Import {
                path,
                account_index,
                remember,
            } => {
                if let Some(client) = self.client.as_mut() {
                    client.disconnect();
                }
                if account_index > 9 {
                    return Err(AccountError::InvalidInput.into());
                }
                let bytes = read_import(&path)?;
                current(&self.control, operation)?;
                let cookies = SessionCookies::import_netscape(bytes, now()?)?;
                self.connect_and_persist(cookies, account_index, remember, operation)
            }
            AccountRequest::ImportBrowser {
                browser,
                profile,
                account_index,
                remember,
            } => {
                if let Some(client) = self.client.as_mut() {
                    client.disconnect();
                }
                if account_index > 9 {
                    return Err(AccountError::InvalidInput.into());
                }
                // Reads ONLY YouTube/Google session cookies from the chosen
                // profile, then verifies identity exactly like file import.
                let cookies = import_browser_session(browser, &profile, now()?)?;
                current(&self.control, operation)?;
                self.connect_and_persist(cookies, account_index, remember, operation)
            }
            AccountRequest::Reconnect(saved) => {
                if let Some(client) = self.client.as_mut() {
                    client.disconnect();
                }
                if saved.account_index > 9 {
                    return Err(AccountError::InvalidInput.into());
                }
                self.claim_directory()?;
                let marker = read_marker(&self.directory)?.ok_or(WorkerError::NoSavedProfile)?;
                if marker.profile != saved.profile || marker.account_index != saved.account_index {
                    return Err(AccountError::InvalidInput.into());
                }
                self.saved = Some(saved.clone());
                let store = ProtectedSessionStore::new(&self.directory, saved.profile.clone())?;
                let secret = store.load()?.ok_or(WorkerError::NoSavedProfile)?;
                current(&self.control, operation)?;
                let cookies = SessionCookies::import_netscape(
                    Zeroizing::new(secret.expose().to_vec()),
                    now()?,
                )?;
                let connection = self
                    .client()?
                    .connect(cookies, saved.account_index, operation)?;
                Ok(Response::Connected {
                    connection,
                    persistence: Persistence::Remembered(saved),
                })
            }
            AccountRequest::InspectSaved => {
                if !self.cleanup_checked {
                    AccountClient::cleanup_stale_extractor_sessions()?;
                    self.cleanup_checked = true;
                }
                Ok(Response::SavedProfile(read_marker(&self.directory)?))
            }
            AccountRequest::Subscriptions(cursor) => Ok(Response::Subscriptions(
                self.client()?.subscriptions(cursor.as_ref(), operation)?,
            )),
            AccountRequest::Playlists(cursor) => Ok(Response::Playlists(
                self.client()?.playlists(cursor.as_ref(), operation)?,
            )),
            AccountRequest::Playlist(id, cursor) => Ok(Response::Playlist(
                self.client()?.playlist(&id, cursor.as_ref(), operation)?,
            )),
            AccountRequest::Recommendations(cursor) => Ok(Response::Recommendations(
                self.client()?.recommendations(cursor.as_ref(), operation)?,
            )),
            AccountRequest::SubscriptionState(id) => Ok(Response::SubscriptionState(
                self.client()?.subscription_state(&id, operation)?,
            )),
            AccountRequest::Rating(id) => {
                Ok(Response::Rating(self.client()?.rating(&id, operation)?))
            }
            AccountRequest::Mutate(mutation) => Ok(Response::Mutation(
                self.client()?.apply_user_mutation(mutation, operation)?,
            )),
            AccountRequest::Reconcile => Ok(Response::Mutation(
                self.client()?.reconcile_pending(operation)?,
            )),
            AccountRequest::ResolvePlayback {
                id,
                policy,
                selection_generation,
            } => {
                let resolver = self
                    .resolver
                    .as_ref()
                    .ok_or(WorkerError::Resolver(ProviderError::HelperUnavailable))?
                    .get_on_worker()
                    .map_err(WorkerError::Resolver)?;
                let playback = self
                    .client()?
                    .resolve_authenticated_with_policy(&resolver, &id, policy, operation)?;
                Ok(Response::PlaybackResolved {
                    playback: Box::new(playback),
                    selection_generation,
                })
            }
        }
    }
    /// Shared tail of every connection path (file or browser import). Discards
    /// any previously saved credential, verifies identity, and optionally saves
    /// the session to protected storage. Identical for both import sources.
    fn connect_and_persist(
        &mut self,
        cookies: SessionCookies,
        account_index: u8,
        remember: bool,
        operation: &OperationContext,
    ) -> Result<Response, WorkerError> {
        // An explicit replacement must not strand an old saved credential.
        self.forget()?;
        current(&self.control, operation)?;
        let connection = self.client()?.connect(cookies, account_index, operation)?;
        let persistence = if remember {
            match self.remember(account_index, operation) {
                Ok(saved) => Persistence::Remembered(saved),
                Err(error) => Persistence::SaveFailed(error),
            }
        } else {
            Persistence::SessionOnly
        };
        Ok(Response::Connected {
            connection,
            persistence,
        })
    }
    fn remember(
        &mut self,
        account_index: u8,
        operation: &OperationContext,
    ) -> Result<SavedProfile, WorkerError> {
        current(&self.control, operation)?;
        self.claim_directory()?;
        let saved = SavedProfile {
            profile: SessionProfile::random()?,
            account_index,
        };
        let store = ProtectedSessionStore::new(&self.directory, saved.profile.clone())?;
        prepare_directory(&self.directory)?;
        let secret = self.client()?.export_for_vault()?;
        current(&self.control, operation)?;
        // Keep the reference before I/O so a racing disconnect can retry deletion.
        self.saved = Some(saved.clone());
        // Record the nonsecret recovery reference before allocating a Keychain key.
        // A crash between key creation and envelope commit remains forgettable.
        let outcome = save_with_recovery_marker(
            &self.directory,
            &saved,
            || current(&self.control, operation),
            || store.save(&secret).map_err(WorkerError::from),
        );
        if let Err(error) = outcome {
            // Cancellation after a successful Keychain write must not leave a new login behind.
            self.forget()?;
            return Err(error);
        }
        Ok(saved)
    }
    fn claim_directory(&mut self) -> Result<(), WorkerError> {
        if self.directory_lock.is_some() {
            return Ok(());
        }
        prepare_directory(&self.directory)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
        }
        let file = options
            .open(self.directory.join(".account-worker.lock"))
            .map_err(|_| VaultError::UnsafePath)?;
        let metadata = file
            .metadata()
            .map_err(|_| VaultError::StorageUnavailable)?;
        if !metadata.is_file() {
            return Err(VaultError::UnsafePath.into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.mode() & 0o077 != 0
                || metadata.nlink() != 1
                || metadata.uid() != unsafe { libc::geteuid() }
            {
                return Err(VaultError::UnsafePath.into());
            }
        }
        file.try_lock().map_err(|_| VaultError::InUse)?;
        self.directory_lock = Some(file);
        Ok(())
    }
    fn forget(&mut self) -> Result<(), WorkerError> {
        if self
            .directory
            .try_exists()
            .map_err(|_| VaultError::StorageUnavailable)?
        {
            self.claim_directory()?;
        }
        let saved = match self.saved.clone() {
            Some(saved) => Some(saved),
            None => read_marker(&self.directory)?,
        };
        if let Some(saved) = saved {
            self.saved = Some(saved.clone());
            ProtectedSessionStore::new(&self.directory, saved.profile)?.delete()?;
            remove_marker(&self.directory)?;
            self.saved = None;
        }
        Ok(())
    }
}
fn now() -> Result<u64, WorkerError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_secs())
        .map_err(|_| WorkerError::Unavailable)
}

/// Canonicalize only the explicitly selected file; never enumerate a directory.
fn read_import(path: &Path) -> Result<Zeroizing<Vec<u8>>, WorkerError> {
    let canonical = path.canonicalize().map_err(|_| WorkerError::InvalidFile)?;
    #[cfg(unix)]
    let file = {
        use std::os::unix::fs::OpenOptionsExt;
        OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(canonical)
            .map_err(|_| WorkerError::InvalidFile)?
    };
    // The handle itself must be a regular file, never a reparse point.
    #[cfg(windows)]
    let file = oxplay_storage::windows_private::open_no_follow(&canonical, false, false)
        .map_err(|_| WorkerError::InvalidFile)?;
    #[cfg(not(any(unix, windows)))]
    {
        let _ = canonical;
        return Err(WorkerError::InvalidFile);
    }
    #[cfg(any(unix, windows))]
    {
        let metadata = file.metadata().map_err(|_| WorkerError::InvalidFile)?;
        if !metadata.is_file() {
            return Err(WorkerError::InvalidFile);
        }
        if metadata.len() > MAX_COOKIE_BYTES as u64 {
            return Err(AccountError::ImportTooLarge.into());
        }
        let mut bytes = Zeroizing::new(Vec::new());
        file.take(MAX_COOKIE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| WorkerError::InvalidFile)?;
        if bytes.len() > MAX_COOKIE_BYTES {
            return Err(AccountError::ImportTooLarge.into());
        }
        Ok(bytes)
    }
}

const MARKER: &str = "saved-profile";
const MARKER_MAGIC: &str = "oxplay-session-v1";
/// The recovery reference precedes credential allocation. On any failure the
/// caller attempts vault deletion, retaining this reference if deletion fails.
fn save_with_recovery_marker(
    directory: &Path,
    saved: &SavedProfile,
    validate: impl Fn() -> Result<(), WorkerError>,
    save: impl FnOnce() -> Result<(), WorkerError>,
) -> Result<(), WorkerError> {
    write_marker(directory, saved)?;
    validate()?;
    save()?;
    validate()
}
fn prepare_directory(directory: &Path) -> Result<(), WorkerError> {
    if !directory.is_absolute() {
        return Err(VaultError::UnsafePath.into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt};
        if !directory.exists() {
            let parent = directory.parent().ok_or(VaultError::UnsafePath)?;
            std::fs::create_dir_all(parent).map_err(|_| VaultError::StorageUnavailable)?;
            match std::fs::DirBuilder::new().mode(0o700).create(directory) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => return Err(VaultError::StorageUnavailable.into()),
            }
        }
        let metadata =
            std::fs::symlink_metadata(directory).map_err(|_| VaultError::StorageUnavailable)?;
        if !metadata.is_dir()
            || metadata.mode() & 0o077 != 0
            || metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err(VaultError::UnsafePath.into());
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        let parent = directory.parent().ok_or(VaultError::UnsafePath)?;
        std::fs::create_dir_all(parent).map_err(|_| VaultError::StorageUnavailable)?;
        oxplay_storage::windows_private::ensure_private_directory(directory).map_err(|error| {
            if error.kind() == std::io::ErrorKind::InvalidInput {
                VaultError::UnsafePath
            } else {
                VaultError::StorageUnavailable
            }
        })?;
        Ok(())
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(VaultError::UnsupportedPlatform.into())
    }
}
/// Persist a directory entry change. std cannot open Windows directory handles
/// for flushing; NTFS journals the rename after the file itself was synced.
fn sync_directory(directory: &Path) -> Result<(), WorkerError> {
    #[cfg(not(windows))]
    File::open(directory)
        .and_then(|file| file.sync_all())
        .map_err(|_| VaultError::StorageUnavailable)?;
    #[cfg(windows)]
    let _ = directory;
    Ok(())
}
fn read_marker(directory: &Path) -> Result<Option<SavedProfile>, WorkerError> {
    if !directory
        .try_exists()
        .map_err(|_| VaultError::StorageUnavailable)?
    {
        return Ok(None);
    }
    prepare_directory(directory)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_OPEN_REPARSE_POINT: open a link itself, never its target.
        options.custom_flags(0x0020_0000);
    }
    let file = match options.open(directory.join(MARKER)) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(VaultError::UnsafePath.into()),
    };
    let metadata = file
        .metadata()
        .map_err(|_| VaultError::StorageUnavailable)?;
    if !metadata.is_file() || metadata.len() > 64 {
        return Err(VaultError::InvalidEnvelope.into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.mode() & 0o077 != 0
            || metadata.nlink() != 1
            || metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err(VaultError::UnsafePath.into());
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // FILE_ATTRIBUTE_REPARSE_POINT: the marker handle must not be a link.
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(VaultError::UnsafePath.into());
        }
    }
    let mut text = String::new();
    file.take(65)
        .read_to_string(&mut text)
        .map_err(|_| VaultError::InvalidEnvelope)?;
    if text.len() > 64 {
        return Err(VaultError::InvalidEnvelope.into());
    }
    let mut lines = text.lines();
    if lines.next() != Some(MARKER_MAGIC) {
        return Err(VaultError::InvalidEnvelope.into());
    }
    let profile = SessionProfile::parse(lines.next().ok_or(VaultError::InvalidEnvelope)?)?;
    let account_index = lines
        .next()
        .and_then(|v| v.parse::<u8>().ok())
        .filter(|v| *v <= 9)
        .ok_or(VaultError::InvalidEnvelope)?;
    if lines.next().is_some() {
        return Err(VaultError::InvalidEnvelope.into());
    }
    Ok(Some(SavedProfile {
        profile,
        account_index,
    }))
}
fn write_marker(directory: &Path, saved: &SavedProfile) -> Result<(), WorkerError> {
    prepare_directory(directory)?;
    let temporary = directory.join(format!(".saved-{}", SessionProfile::random()?.as_str()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let result = (|| {
        let mut file = options
            .open(&temporary)
            .map_err(|_| VaultError::StorageUnavailable)?;
        write!(
            file,
            "{MARKER_MAGIC}\n{}\n{}\n",
            saved.profile.as_str(),
            saved.account_index
        )
        .map_err(|_| VaultError::StorageUnavailable)?;
        file.sync_all()
            .map_err(|_| VaultError::StorageUnavailable)?;
        std::fs::rename(&temporary, directory.join(MARKER))
            .map_err(|_| VaultError::StorageUnavailable)?;
        sync_directory(directory)
    })();
    let _ = std::fs::remove_file(temporary);
    result
}
fn remove_marker(directory: &Path) -> Result<(), WorkerError> {
    match std::fs::remove_file(directory.join(MARKER)) {
        Ok(()) => sync_directory(directory),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(VaultError::StorageUnavailable.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::mpsc, time::Duration};

    struct TestDirectory(PathBuf);
    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "oxplay-account-synthetic-{}",
                SessionProfile::random().unwrap().as_str()
            ));
            prepare_directory(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn dormant() -> Worker {
        Worker {
            shared: Arc::new(Shared {
                state: Mutex::new(State::default()),
                ready: Condvar::new(),
                control: SessionControl::default(),
            }),
            thread: None,
        }
    }
    fn engine(directory: &Path) -> Engine {
        Engine {
            directory: directory.to_owned(),
            client: None,
            saved: None,
            directory_lock: None,
            cleanup_checked: true,
            control: SessionControl::default(),
            resolver: None,
        }
    }
    fn playback_request(selection_generation: u64) -> AccountRequest {
        AccountRequest::ResolvePlayback {
            id: VideoId::new("aqz-KE-bpKQ").unwrap(),
            policy: ResolutionPolicy::default(),
            selection_generation,
        }
    }
    fn activate(worker: &Worker) -> OperationContext {
        let mut state = worker.shared.state.lock().unwrap();
        let work = state.pending.take().unwrap();
        let operation = work.operation.clone();
        state.active = Some(Active {
            operation: work.operation,
            playback_selection: work.action.playback_selection(),
        });
        operation
    }
    #[test]
    fn playback_cancellation_removes_pending_without_changing_session() {
        let worker = dormant();
        worker.submit(playback_request(42)).unwrap();
        let token = worker
            .shared
            .state
            .lock()
            .unwrap()
            .pending
            .as_ref()
            .unwrap()
            .operation
            .cancel
            .clone();
        worker.cancel_playback().unwrap();
        assert!(token.is_cancelled());
        assert_eq!(worker.control().generation(), 0);
        assert!(worker.shared.state.lock().unwrap().pending.is_none());
        assert!(worker.submit(AccountRequest::Reconcile).is_ok());
    }
    #[test]
    fn active_playback_retains_capacity_until_cancelled_helper_finishes() {
        let worker = dormant();
        worker.submit(playback_request(42)).unwrap();
        let operation = activate(&worker);
        worker.cancel_playback().unwrap();
        assert!(operation.cancel.is_cancelled());
        assert!(matches!(
            current(&worker.control(), &operation),
            Err(WorkerError::Account(AccountError::Cancelled))
        ));
        assert!(matches!(
            worker.submit(AccountRequest::Reconcile),
            Err(WorkerError::Account(AccountError::Busy))
        ));
        // Mirrors the worker's post-reap transition. Cancellation does not publish
        // a fake success/error acknowledgment into the single response slot.
        worker.shared.state.lock().unwrap().active = None;
        assert!(worker.take().is_none());
        assert!(worker.submit(AccountRequest::Reconcile).is_ok());
    }
    #[test]
    fn playback_cancellation_preserves_queued_and_active_account_operations() {
        for request in [
            AccountRequest::Mutate(AccountMutation::Rating {
                video_id: VideoId::new("aqz-KE-bpKQ").unwrap(),
                rating: oxplay_youtube::account::VideoRating::Like,
            }),
            AccountRequest::Reconcile,
            AccountRequest::Subscriptions(None),
            AccountRequest::Recommendations(None),
            AccountRequest::InspectSaved,
        ] {
            let worker = dormant();
            worker.submit(request).unwrap();
            worker.cancel_playback().unwrap();
            assert!(worker.shared.state.lock().unwrap().pending.is_some());
            let operation = activate(&worker);
            worker.cancel_playback().unwrap();
            assert!(!operation.cancel.is_cancelled());
            assert!(current(&worker.control(), &operation).is_ok());
            assert!(worker.shared.state.lock().unwrap().active.is_some());
        }
    }
    #[test]
    fn playback_cancellation_discards_published_errors_but_preserves_account_results() {
        let worker = dormant();
        for result in [
            Ok(Response::Mutation(MutationOutcome::NeedsReconciliation)),
            Err(WorkerError::Account(AccountError::ReconciliationRequired)),
        ] {
            worker.shared.state.lock().unwrap().result = Some(AccountResponse {
                request_id: 1,
                generation: 0,
                playback_selection: None,
                connection: None,
                unconfirmed_mutation: false,
                retry_after: None,
                result,
            });
            worker.cancel_playback().unwrap();
            assert!(worker.take().is_some());
        }
        worker.shared.state.lock().unwrap().result = Some(AccountResponse {
            request_id: 2,
            generation: 0,
            playback_selection: Some(42),
            connection: None,
            unconfirmed_mutation: false,
            retry_after: None,
            result: Err(WorkerError::Resolver(ProviderError::Offline)),
        });
        worker.cancel_playback().unwrap();
        assert!(worker.take().is_none());
        assert!(worker.submit(AccountRequest::Reconcile).is_ok());
    }
    #[test]
    fn playback_cancellation_preserves_priority_disconnect() {
        let worker = dormant();
        worker.submit(playback_request(42)).unwrap();
        let operation = activate(&worker);
        let id = worker.disconnect().unwrap();
        worker.cancel_playback().unwrap();
        assert!(operation.cancel.is_cancelled());
        let state = worker.shared.state.lock().unwrap();
        assert!(matches!(
            state.pending.as_ref().unwrap().action,
            Action::Disconnect
        ));
        assert_eq!(state.pending.as_ref().unwrap().operation.request_id, id);
    }
    #[test]
    fn uninjected_resolver_reports_selection_without_network_or_vault_access() {
        let directory = TestDirectory::new();
        let path = directory.0.join("unused-vault");
        let (notify, receive) = mpsc::channel();
        let worker = Worker::new(path.clone(), move || {
            let _ = notify.send(());
        });
        let id = worker.submit(playback_request(42)).unwrap();
        receive.recv_timeout(Duration::from_secs(3)).unwrap();
        let response = worker.take().unwrap();
        assert_eq!(response.request_id, id);
        assert_eq!(response.playback_selection, Some(42));
        assert!(response.connection.is_none());
        assert!(matches!(
            response.result,
            Err(WorkerError::Resolver(ProviderError::HelperUnavailable))
        ));
        assert!(!path.exists());
    }
    #[test]
    fn shared_resolver_cannot_start_authenticated_helper_without_verified_identity() {
        let directory = TestDirectory::new();
        let path = directory.0.join("unused-vault");
        let resolver = SharedResolver::new(
            directory.0.join("nonexistent-yt-dlp"),
            directory.0.join("nonexistent-deno"),
        );
        let (notify, receive) = mpsc::channel();
        let worker = Worker::with_resolver(path.clone(), resolver, move || {
            let _ = notify.send(());
        });
        worker.submit(playback_request(42)).unwrap();
        receive.recv_timeout(Duration::from_secs(3)).unwrap();
        let response = worker.take().unwrap();
        assert_eq!(response.playback_selection, Some(42));
        assert!(response.connection.is_none());
        assert!(matches!(
            response.result,
            Err(WorkerError::Account(AccountError::IdentityNotVerified))
        ));
        assert!(!path.exists());
    }
    #[test]
    fn unverified_worker_rejects_home_recommendations_without_network_or_vault() {
        let directory = TestDirectory::new();
        let path = directory.0.join("unused-vault");
        let (notify, receive) = mpsc::channel();
        let worker = Worker::new(path.clone(), move || {
            let _ = notify.send(());
        });
        let id = worker
            .submit(AccountRequest::Recommendations(None))
            .unwrap();
        receive.recv_timeout(Duration::from_secs(3)).unwrap();
        let response = worker.take().unwrap();
        assert_eq!(response.request_id, id);
        assert!(response.playback_selection.is_none());
        assert!(response.connection.is_none());
        assert!(matches!(
            response.result,
            Err(WorkerError::Account(AccountError::IdentityNotVerified))
        ));
        assert!(!path.exists());
    }
    #[test]
    fn explicit_file_import_is_bounded_and_rejects_directories() {
        let directory = TestDirectory::new();
        assert!(matches!(
            read_import(&directory.0),
            Err(WorkerError::InvalidFile)
        ));
        let path = directory.0.join("synthetic-cookie-export");
        let file = File::create(&path).unwrap();
        file.set_len(MAX_COOKIE_BYTES as u64 + 1).unwrap();
        assert!(matches!(
            read_import(&path),
            Err(WorkerError::Account(AccountError::ImportTooLarge))
        ));
        std::fs::write(&path, "# Synthetic invalid export; no credentials\n").unwrap();
        assert!(read_import(&path).unwrap().starts_with(b"# Synthetic"));
    }
    #[test]
    fn invalid_export_never_constructs_account_transport() {
        let directory = TestDirectory::new();
        let path = directory.0.join("synthetic-invalid-export");
        std::fs::write(&path, "This is intentionally not a cookie export.").unwrap();
        let mut engine = engine(&directory.0);
        let operation = OperationContext {
            request_id: 1,
            session_generation: 0,
            cancel: CancellationToken::default(),
        };
        assert!(matches!(
            engine.handle(
                AccountRequest::Import {
                    path,
                    account_index: 0,
                    remember: false
                },
                &operation
            ),
            Err(WorkerError::Account(AccountError::InvalidCookieFile))
        ));
        assert!(engine.client.is_none());
        assert!(engine.saved.is_none());
    }
    #[test]
    fn browser_import_rejects_invalid_index_before_touching_any_browser() {
        // The account-index guard runs before any browser discovery/read, so an
        // out-of-range request fails closed without inspecting real browsers.
        let directory = TestDirectory::new();
        let mut engine = engine(&directory.0);
        let operation = OperationContext {
            request_id: 1,
            session_generation: 0,
            cancel: CancellationToken::default(),
        };
        assert!(matches!(
            engine.handle(
                AccountRequest::ImportBrowser {
                    browser: oxplay_youtube::account::BrowserKind::Chrome,
                    profile: "Default".to_owned(),
                    account_index: 10,
                    remember: false,
                },
                &operation,
            ),
            Err(WorkerError::Account(AccountError::InvalidInput))
        ));
        assert!(engine.client.is_none());
        assert!(engine.saved.is_none());
    }
    #[test]
    fn marker_round_trip_is_nonsecret_private_and_does_not_connect() {
        let directory = TestDirectory::new();
        let saved = SavedProfile {
            profile: SessionProfile::random().unwrap(),
            account_index: 2,
        };
        assert!(read_marker(&directory.0).unwrap().is_none());
        write_marker(&directory.0, &saved).unwrap();
        let read = read_marker(&directory.0).unwrap().unwrap();
        assert_eq!(read.profile, saved.profile);
        assert_eq!(read.account_index, 2);
        let mut engine = engine(&directory.0);
        let operation = OperationContext {
            request_id: 1,
            session_generation: 0,
            cancel: CancellationToken::default(),
        };
        assert!(matches!(
            engine.handle(AccountRequest::InspectSaved, &operation),
            Ok(Response::SavedProfile(Some(_)))
        ));
        assert!(engine.client.is_none());
        assert!(engine.saved.is_none());
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(
                std::fs::metadata(directory.0.join(MARKER)).unwrap().mode() & 0o777,
                0o600
            );
        }
        remove_marker(&directory.0).unwrap();
        assert!(read_marker(&directory.0).unwrap().is_none());
    }
    #[cfg(unix)]
    #[test]
    fn marker_rejects_symlinks_and_unprivate_files() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let directory = TestDirectory::new();
        let saved = SavedProfile {
            profile: SessionProfile::random().unwrap(),
            account_index: 0,
        };
        write_marker(&directory.0, &saved).unwrap();
        let marker = directory.0.join(MARKER);
        std::fs::set_permissions(&marker, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(
            read_marker(&directory.0),
            Err(WorkerError::Vault(VaultError::UnsafePath))
        ));
        std::fs::remove_file(&marker).unwrap();
        symlink("missing-target", &marker).unwrap();
        assert!(matches!(
            read_marker(&directory.0),
            Err(WorkerError::Vault(VaultError::UnsafePath))
        ));
    }
    #[test]
    fn directory_lock_excludes_concurrent_credential_mutation() {
        let directory = TestDirectory::new();
        let mut first = engine(&directory.0);
        let mut second = engine(&directory.0);
        first.claim_directory().unwrap();
        assert!(matches!(
            second.claim_directory(),
            Err(WorkerError::Vault(VaultError::InUse))
        ));
        drop(first);
        second.claim_directory().unwrap();
    }
    #[test]
    fn queue_is_bounded_and_disconnect_has_priority_and_cancels_immediately() {
        let worker = dormant();
        worker.submit(AccountRequest::InspectSaved).unwrap();
        assert!(matches!(
            worker.submit(AccountRequest::InspectSaved),
            Err(WorkerError::Account(AccountError::Busy))
        ));
        let operation = {
            let mut state = worker.shared.state.lock().unwrap();
            let operation = state.pending.take().unwrap().operation;
            state.active = Some(Active {
                operation: operation.clone(),
                playback_selection: None,
            });
            operation
        };
        let prior = worker.control().generation();
        let disconnected = worker.disconnect().unwrap();
        assert_eq!(worker.control().generation(), prior + 1);
        assert!(operation.cancel.is_cancelled());
        let state = worker.shared.state.lock().unwrap();
        assert!(matches!(
            state.pending.as_ref().unwrap().action,
            Action::Disconnect
        ));
        assert_eq!(
            state.pending.as_ref().unwrap().operation.request_id,
            disconnected
        );
        assert!(state.result.is_none());
    }
    #[test]
    fn account_switch_invalidates_and_stale_result_never_reaches_ui() {
        let worker = dormant();
        worker
            .submit(AccountRequest::Reconnect(SavedProfile {
                profile: SessionProfile::random().unwrap(),
                account_index: 0,
            }))
            .unwrap();
        assert_eq!(worker.control().generation(), 1);
        {
            let mut state = worker.shared.state.lock().unwrap();
            state.pending = None;
            state.result = Some(AccountResponse {
                request_id: 1,
                generation: 1,
                playback_selection: None,
                connection: None,
                unconfirmed_mutation: false,
                retry_after: None,
                result: Ok(Response::SavedProfile(None)),
            });
        }
        worker.control().invalidate();
        assert!(worker.take().is_none());
        assert!(worker.submit(AccountRequest::InspectSaved).is_ok());
    }
    #[test]
    fn worker_inspection_and_signout_publish_without_account_network_or_keychain() {
        let directory = TestDirectory::new();
        // Keep the vault absent so disconnect is a no-op for credential storage.
        let path = directory.0.join("unused-vault");
        let (notify, receive) = mpsc::channel();
        let worker = Worker::new(path.clone(), move || {
            let _ = notify.send(());
        });
        worker.submit(AccountRequest::InspectSaved).unwrap();
        receive.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(matches!(
            worker.take().unwrap().result,
            Ok(Response::SavedProfile(None))
        ));
        let request_id = worker.disconnect().unwrap();
        let generation = worker.control().generation();
        receive.recv_timeout(Duration::from_secs(3)).unwrap();
        let response = worker.take().unwrap();
        assert_eq!(response.request_id, request_id);
        assert_eq!(response.generation, generation);
        assert!(matches!(
            response.result,
            Ok(Response::Disconnected { forget_error: None })
        ));
        assert!(!path.exists());
    }
    #[test]
    fn failed_save_keeps_nonsecret_recovery_reference_and_checks_post_save_generation() {
        let directory = TestDirectory::new();
        let saved = SavedProfile {
            profile: SessionProfile::random().unwrap(),
            account_index: 0,
        };
        let failure = save_with_recovery_marker(
            &directory.0,
            &saved,
            || Ok(()),
            || Err(VaultError::KeychainUnavailable.into()),
        );
        assert_eq!(
            failure,
            Err(WorkerError::Vault(VaultError::KeychainUnavailable))
        );
        // If OS deletion also fails, the caller can retain this marker and retry
        // forget after restart. This test never opens a real credential store.
        let restored = read_marker(&directory.0).unwrap().unwrap();
        assert_eq!(restored.profile, saved.profile);
        let contents = std::fs::read_to_string(directory.0.join(MARKER)).unwrap();
        assert_eq!(
            contents,
            format!("{MARKER_MAGIC}\n{}\n0\n", saved.profile.as_str())
        );
        let control = SessionControl::default();
        let operation = OperationContext {
            request_id: 1,
            session_generation: 0,
            cancel: CancellationToken::default(),
        };
        let cancelled = save_with_recovery_marker(
            &directory.0,
            &saved,
            || current(&control, &operation),
            || {
                control.invalidate();
                Ok(())
            },
        );
        assert_eq!(
            cancelled,
            Err(WorkerError::Account(AccountError::StaleSession))
        );
        assert_eq!(
            read_marker(&directory.0).unwrap().unwrap().profile,
            saved.profile
        );
    }
    #[test]
    fn immediate_shutdown_completes_queued_disconnect_without_reading_credentials() {
        let directory = TestDirectory::new();
        let (release, begin) = mpsc::channel();
        let worker = Worker::start(
            directory.0.clone(),
            || {},
            move || {
                let _ = begin.recv();
            },
        );
        let shared = worker.shared.clone();
        worker.disconnect().unwrap();
        let closing = thread::spawn(move || drop(worker));
        // The actual worker is held before its loop. Wait for Drop to run first,
        // proving this tests a queued sign-out rather than one already executing.
        let guard = shared.state.lock().unwrap();
        let (guard, timeout) = shared
            .ready
            .wait_timeout_while(guard, Duration::from_secs(3), |state| !state.closing)
            .unwrap();
        assert!(!timeout.timed_out());
        assert!(matches!(
            guard.pending.as_ref().map(|work| &work.action),
            Some(Action::Disconnect)
        ));
        drop(guard);
        release.send(()).unwrap();
        closing.join().unwrap();
        // No saved marker exists, so forget cannot read or delete a Keychain item.
        // Its directory claim is an observable effect of completed cleanup.
        assert!(directory.0.join(".account-worker.lock").is_file());
        assert!(!directory.0.join(MARKER).exists());
    }
}
