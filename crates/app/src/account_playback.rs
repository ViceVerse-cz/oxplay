// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit account playback coordination. No guest-to-account fallback, no
//! credentials in UI state, and no periodic expiry polling.
use crate::{
    App, CaptionsUi, UiState,
    account::{AccountResponse, Response, WorkerError},
};
use serein_core::VideoId;
use serein_youtube::{
    ResolutionPolicy,
    account::{AccountError, AccountPlaybackLease},
};
use slint::{ComponentHandle, Timer, TimerMode};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Initial,
    Restart { load: u64 },
    Replacement { expiry: bool },
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct RestartScope {
    load: u64,
    session: u64,
}
struct RestartFailure {
    scope: RestartScope,
    attempts: u8,
    deadline: Option<Instant>, // None is a terminal policy rejection.
}
#[derive(Default)]
struct RestartState {
    failure: Option<RestartFailure>,
}
fn restart_delay(error: WorkerError, attempt: u8) -> Option<Duration> {
    match error {
        WorkerError::Resolver(error) if crate::guest_recovery::retryable(error) => {
            Some(if error == serein_core::ProviderError::RateLimited {
                Duration::from_secs(60)
            } else {
                Duration::from_secs((2u64 << attempt.min(5)).min(60))
            })
        }
        WorkerError::Account(
            AccountError::Offline | AccountError::Timeout | AccountError::ServiceUnavailable,
        ) => Some(Duration::from_secs((2u64 << attempt.min(5)).min(60))),
        WorkerError::Account(AccountError::RateLimited) => Some(Duration::from_secs(60)),
        _ => None,
    }
}
impl RestartState {
    fn available(&self, scope: RestartScope) -> bool {
        !self
            .failure
            .as_ref()
            .is_some_and(|failure| failure.scope == scope && failure.deadline.is_none())
    }
    fn ready(&self, scope: RestartScope, now: Instant) -> bool {
        self.available(scope)
            && self
                .failure
                .as_ref()
                .filter(|failure| failure.scope == scope)
                .is_none_or(|failure| failure.deadline.is_some_and(|deadline| deadline <= now))
    }
    fn remaining(&self, scope: RestartScope, now: Instant) -> Option<Duration> {
        self.failure
            .as_ref()
            .filter(|failure| failure.scope == scope)
            .and_then(|failure| failure.deadline)
            .map(|deadline| deadline.saturating_duration_since(now))
    }
    fn failed(
        &mut self,
        scope: RestartScope,
        error: WorkerError,
        retry_after: Option<Duration>,
        now: Instant,
    ) -> Option<Duration> {
        let attempts = self
            .failure
            .as_ref()
            .filter(|failure| failure.scope == scope)
            .map_or(0, |failure| failure.attempts);
        let wait =
            restart_delay(error, attempts).map(|delay| delay.max(retry_after.unwrap_or_default()));
        let deadline = wait.and_then(|wait| now.checked_add(wait));
        self.failure = Some(RestartFailure {
            scope,
            attempts: attempts.saturating_add(1).min(5),
            deadline,
        });
        wait.filter(|_| deadline.is_some())
    }
}
#[derive(Debug, PartialEq, Eq)]
enum InstallRejection {
    Stale,
    Expired,
    Clearing,
    Presentation,
}
fn admission(
    scope_matches: bool,
    valid: bool,
    clearing: bool,
    presentation: bool,
) -> Result<(), InstallRejection> {
    if !scope_matches {
        Err(InstallRejection::Stale)
    } else if !valid {
        Err(InstallRejection::Expired)
    } else if clearing {
        Err(InstallRejection::Clearing)
    } else if !presentation {
        Err(InstallRejection::Presentation)
    } else {
        Ok(())
    }
}
struct Job {
    id: VideoId,
    policy: ResolutionPolicy,
    kind: Kind,
    selection: u64,
    session: u64,
}
impl Job {
    fn accepts_response(
        &self,
        selection: Option<u64>,
        session: u64,
        current_selection: u64,
        current_session: u64,
    ) -> bool {
        selection == Some(self.selection)
            && session == self.session
            && current_selection == self.selection
            && current_session == self.session
    }
    fn matches(&self, selection: u64, session: u64, video: &VideoId) -> bool {
        self.selection == selection && self.session == session && self.id == *video
    }
}
#[derive(Default)]
pub struct State {
    selection: Cell<u64>,
    pending: RefCell<Option<Job>>,
    active: RefCell<Option<AccountPlaybackLease>>,
    expiry: Timer,
    restart: RefCell<RestartState>,
    restart_deadline: Timer,
}
pub fn authorization(state: &UiState) -> Option<AccountPlaybackLease> {
    // Preserve an invalid lease until observe/clear handles it. Returning None
    // for invalid authority could incorrectly classify private playback as guest.
    state.account_playback.active.borrow().clone()
}
pub fn generation(state: &UiState) -> u64 {
    state.account_playback.selection.get()
}
pub fn is_current(state: &UiState, video: &VideoId, selection: u64, session: u64) -> bool {
    generation(state) == selection
        && state.account_ui.session_generation() == session
        && state
            .current_video
            .borrow()
            .as_ref()
            .is_some_and(|item| item.id == *video)
        && authorization(state)
            .is_some_and(|lease| lease.generation() == session && lease.is_valid())
}
pub fn cancel_pending(app: &App, state: &Rc<UiState>) {
    crate::guest_recovery::invalidate(app, state);
    let s = &state.account_playback;
    state.focus_intent.cancel_account();
    s.selection.set(s.selection.get().wrapping_add(1));
    if s.pending.borrow_mut().take().is_some() {
        app.set_busy(false);
    }
    state.account_ui.cancel_playback(app);
}
pub fn leave_for_guest(app: &App, state: &Rc<UiState>) {
    cancel_pending(app, state);
    state.account_playback.expiry.stop();
    state.account_playback.restart_deadline.stop();
    *state.account_playback.restart.borrow_mut() = RestartState::default();
    if state.account_playback.active.borrow_mut().take().is_some() {
        // The guest load was accepted, but its first frame may not exist yet.
        // Do not retain a private account frame under the new guest identity.
        app.set_video_texture(slint::Image::default());
    }
    app.set_account_playback_active(false);
}
pub fn clear(app: &App, state: &Rc<UiState>) {
    crate::share_ui::clear_account(app, state);
    cancel_pending(app, state);
    state.account_playback.expiry.stop();
    state.account_playback.restart_deadline.stop();
    *state.account_playback.restart.borrow_mut() = RestartState::default();
    let was_active = state.account_playback.active.borrow_mut().take().is_some();
    app.set_account_playback_active(false);
    // Cancelling an initial account request must leave existing guest media alone.
    if !was_active {
        return;
    }
    state.clock_ui.invalidate();
    if let Err(error) = state.player.stop() {
        app.set_status(error.to_string().into());
    }
    if crate::playback_ui::clear_local(state) {
        app.set_busy(false);
    }
    crate::caption_ui::clear_local(app, state);
    crate::chapters_ui::clear(app, state);
    crate::comments_ui::clear_local(app, state);
    app.global::<CaptionsUi>()
        .set_status("Account playback stopped; no guest captions were requested.".into());
    state.current_video.borrow_mut().take();
    state.progress.stop();
    app.set_loaded(false);
    app.set_remote_video(false);
    app.set_video_texture(slint::Image::default());
    app.set_video_title("".into());
    app.set_video_channel("".into());
    app.set_rating_known(false);
    crate::apply_clock(
        app,
        state,
        crate::clock_ui::Values {
            position: 0.,
            duration: 0.,
        },
    );
    app.set_duration(1.);
}
pub fn install(app: &App, state: &Rc<UiState>, lease: AccountPlaybackLease) {
    state.account_playback.expiry.stop();
    *state.account_playback.active.borrow_mut() = Some(lease.clone());
    app.set_account_playback_active(true);
    let Some(delay) = lease.valid_for() else {
        return;
    };
    let delay = delay.min(Duration::from_secs(24 * 60 * 60));
    let weak = app.as_weak();
    let state_weak = Rc::downgrade(state);
    state
        .account_playback
        .expiry
        .start(TimerMode::SingleShot, delay, move || {
            let (Some(app), Some(state)) = (weak.upgrade(), state_weak.upgrade()) else {
                return;
            };
            let current = authorization(&state);
            if let Some(current) = current {
                if !current.is_valid() {
                    observe(&app, &state);
                } else {
                    install(&app, &state, current);
                }
            }
        });
}
pub fn observe(app: &App, state: &Rc<UiState>) {
    if authorization(state).is_some_and(|lease| !lease.is_valid()) {
        clear(app, state);
        state.account_ui.clear_identity(app);
        state.account_ui.set_status(app, "The account session expired or was disconnected. Reconnect explicitly to play with this account.");
        app.set_status("Account playback stopped because its authorization ended.".into());
    }
}
fn ready(app: &App, state: &UiState) -> Result<(), String> {
    if !state.playback_preferences.ready() {
        return Err("Wait for local playback settings to finish loading.".into());
    }
    if !state.account_ui.can_playback(app) {
        return Err(
            "Verify a supported account identity and reconcile any pending account change first."
                .into(),
        );
    }
    if state.account_media_network.is_none() {
        return Err(
            "Authenticated media is unavailable because the protected network path is unavailable."
                .into(),
        );
    }
    if state.caption_cache.active() {
        return Err("Wait for local-data clearing to finish.".into());
    }
    if !state.presentation_ready.get() {
        return Err("Activate the display before starting account playback.".into());
    }
    if app.get_account_busy() || app.get_busy() {
        return Err("Another operation is active. Try again after it finishes.".into());
    }
    Ok(())
}
fn submit(
    app: &App,
    state: &Rc<UiState>,
    id: &VideoId,
    policy: ResolutionPolicy,
    kind: Kind,
) -> Result<(), String> {
    cancel_pending(app, state);
    let selection = generation(state);
    let session = state.account_ui.session_generation();
    state
        .account_ui
        .submit_playback(app, id.clone(), policy, selection)?;
    *state.account_playback.pending.borrow_mut() = Some(Job {
        id: id.clone(),
        policy,
        kind,
        selection,
        session,
    });
    app.set_busy(true);
    state.account_ui.set_status(
        app,
        "Resolving this video with the explicitly selected account session…",
    );
    Ok(())
}
pub fn request_initial(app: &App, state: &Rc<UiState>, id: &VideoId) -> Result<(), String> {
    ready(app, state)?;
    // Guest generation observers may cancel account work. Run them before the
    // new job/selection epoch is installed, never after its submission.
    state.worker.borrow_mut().cancel();
    submit(
        app,
        state,
        id,
        crate::playback_preferences::policy(state),
        Kind::Initial,
    )?;
    state.focus_intent.arm(
        crate::focus_intent::Scope::Account {
            selection: generation(state),
            session: state.account_ui.session_generation(),
        },
        app.get_search_active(),
    );
    Ok(())
}
pub fn request_replacement(
    app: &App,
    state: &Rc<UiState>,
    id: &VideoId,
    policy: ResolutionPolicy,
    expiry: bool,
) -> Result<(), String> {
    ready(app, state)?;
    let session = state.account_ui.session_generation();
    if !is_current(state, id, generation(state), session) {
        return Err("Account playback changed or its authorization ended.".into());
    }
    submit(app, state, id, policy, Kind::Replacement { expiry })
}

fn restart_identity(
    state: &UiState,
    snapshot: &serein_media::Snapshot,
) -> Option<(VideoId, RestartScope)> {
    if !crate::guest_playback::failed_load(snapshot) || snapshot.stop_pending {
        return None;
    }
    let lease = authorization(state)?;
    let session = state.account_ui.session_generation();
    if !lease.is_valid() || lease.generation() != session {
        return None;
    }
    let id = crate::playback_ui::account_retry_video(state, snapshot.load_request_id, session)?;
    Some((
        id,
        RestartScope {
            load: snapshot.load_request_id,
            session,
        },
    ))
}

/// Account authority is retained even on failure. Never reinterpret it as guest.
pub fn can_retry(app: &App, state: &UiState, snapshot: &serein_media::Snapshot) -> bool {
    app.get_loaded()
        && state.account_ui.can_playback(app)
        && restart_identity(state, snapshot)
            .is_some_and(|(_, scope)| state.account_playback.restart.borrow().available(scope))
}

pub fn retry_ready(state: &UiState, snapshot: &serein_media::Snapshot) -> bool {
    restart_identity(state, snapshot).is_none_or(|(_, scope)| {
        state
            .account_playback
            .restart
            .borrow()
            .ready(scope, Instant::now())
    })
}

/// Handle account failures before the guest transport callback. Restart is an
/// explicit new authenticated extraction, using the protected initial-load path.
pub fn retry_failed(app: &App, state: &Rc<UiState>) -> bool {
    let snapshot = state.player.snapshot();
    if authorization(state).is_none() || !crate::guest_playback::failed_load(&snapshot) {
        return false;
    }
    if !can_retry(app, state, &snapshot) || app.get_page() != 2 {
        return true;
    }
    let Some((id, scope)) = restart_identity(state, &snapshot) else {
        return true;
    };
    if !state
        .account_playback
        .restart
        .borrow()
        .ready(scope, Instant::now())
    {
        return true;
    }
    if let Err(error) = ready(app, state) {
        app.set_status(error.into());
        return true;
    }
    let Some(quality) = i32::try_from(state.quality_index.get())
        .ok()
        .and_then(serein_core::QualityCeiling::from_index)
    else {
        return true;
    };
    // Retire guest work before installing the new account selection. The same
    // AccountClient and resolver remain alive, preserving their cooldowns.
    state.worker.borrow_mut().cancel();
    match submit(
        app,
        state,
        &id,
        ResolutionPolicy {
            max_height: quality.height(),
            prefer_h264: true,
        },
        Kind::Restart { load: scope.load },
    ) {
        Ok(()) => {
            state.focus_intent.arm(
                crate::focus_intent::Scope::Account {
                    selection: generation(state),
                    session: scope.session,
                },
                app.get_search_active(),
            );
            app.set_status("Resolving a fresh stream with this account… Playback will restart from the beginning.".into());
        }
        Err(error) => app.set_status(error.into()),
    }
    true
}

fn restart_failed(
    app: &App,
    state: &Rc<UiState>,
    scope: RestartScope,
    error: WorkerError,
    retry_after: Option<Duration>,
) {
    let wait = state.account_playback.restart.borrow_mut().failed(
        scope,
        error,
        retry_after,
        Instant::now(),
    );
    state.account_playback.restart_deadline.stop();
    if let Some(wait) = wait {
        app.set_status(
            format!(
                "{error} Account playback retry is available after a {}-second wait.",
                wait.as_secs()
                    .saturating_add(u64::from(wait.subsec_nanos() != 0))
            )
            .into(),
        );
        arm_restart_readiness(app, state, scope, wait);
    }
    // A failed file can be quiescent: immediately retire/disable its control.
    publish_retry(app, state, &state.player.snapshot());
}

fn arm_restart_readiness(app: &App, state: &Rc<UiState>, scope: RestartScope, wait: Duration) {
    let weak = app.as_weak();
    let state_weak = Rc::downgrade(state);
    state.account_playback.restart_deadline.start(
        TimerMode::SingleShot,
        crate::guest_recovery::deadline_wakeup_delay(wait),
        move || {
            let (Some(app), Some(state)) = (weak.upgrade(), state_weak.upgrade()) else {
                return;
            };
            let snapshot = state.player.snapshot();
            if snapshot.load_request_id == scope.load
                && state.account_ui.session_generation() == scope.session
            {
                let remaining = state
                    .account_playback
                    .restart
                    .borrow()
                    .remaining(scope, Instant::now());
                match remaining {
                    Some(Duration::ZERO) => publish_retry(&app, &state, &snapshot),
                    Some(wait) => arm_restart_readiness(&app, &state, scope, wait),
                    None => {}
                }
            }
        },
    );
}

fn publish_retry(app: &App, state: &UiState, snapshot: &serein_media::Snapshot) {
    app.set_can_retry_playback(
        can_retry(app, state, snapshot) || crate::guest_playback::can_retry(app, state, snapshot),
    );
    app.set_account_retry_ready(retry_ready(state, snapshot));
}

pub fn receive(app: &App, state: &Rc<UiState>, response: AccountResponse) {
    let accepts = state
        .account_playback
        .pending
        .borrow()
        .as_ref()
        .is_some_and(|job| {
            job.accepts_response(
                response.playback_selection,
                response.generation,
                generation(state),
                state.account_ui.session_generation(),
            )
        });
    if !accepts {
        return;
    }
    let job = state
        .account_playback
        .pending
        .borrow_mut()
        .take()
        .expect("matched playback job");
    let focus_scope = crate::focus_intent::Scope::Account {
        selection: job.selection,
        session: job.session,
    };
    app.set_busy(false);
    if let Kind::Restart { load } = job.kind
        && state.player.snapshot().load_request_id != load
    {
        state.focus_intent.cancel(focus_scope);
        return;
    }
    let retry_after = response.retry_after;
    let (playback, selection) = match response.result {
        Ok(Response::PlaybackResolved {
            playback,
            selection_generation,
        }) => (playback, selection_generation),
        Err(error) => {
            state.focus_intent.cancel(focus_scope);
            if matches!(
                error,
                WorkerError::Account(
                    AccountError::SessionExpired
                        | AccountError::IdentityNotVerified
                        | AccountError::StaleSession
                )
            ) {
                clear(app, state);
                state.account_ui.clear_identity(app);
            }
            state.account_ui.set_status(app, error.to_string());
            app.set_status(error.to_string().into());
            if let Kind::Restart { load } = job.kind
                && state.player.snapshot().load_request_id == load
            {
                restart_failed(
                    app,
                    state,
                    RestartScope {
                        load,
                        session: job.session,
                    },
                    error,
                    retry_after,
                );
            }
            return;
        }
        _ => {
            state.focus_intent.cancel(focus_scope);
            app.set_status("An unexpected account playback response was rejected.".into());
            return;
        }
    };
    let scope_matches = job.matches(
        selection,
        playback.playback.session_generation,
        &playback.playback.video.id,
    ) && !playback.playback.guest
        && playback.authorization.generation() == job.session;
    if let Err(rejection) = admission(
        scope_matches,
        playback.authorization.is_valid(),
        state.caption_cache.active(),
        state.presentation_ready.get(),
    ) {
        state.focus_intent.cancel(focus_scope);
        let message = match rejection {
            InstallRejection::Stale => {
                "The account playback response no longer matches this selection."
            }
            InstallRejection::Expired => {
                // Initial extraction has no installed lease for observe() to see.
                // Expiry during the worker-to-UI handoff must still clear identity.
                clear(app, state);
                state.account_ui.clear_identity(app);
                "The account session expired before playback could start. Reconnect explicitly."
            }
            InstallRejection::Clearing => {
                "Account playback was cancelled while local data is being cleared."
            }
            InstallRejection::Presentation => {
                "Video presentation became unavailable. Activate the display, then select the account video again."
            }
        };
        state.account_ui.set_status(app, message);
        app.set_status(message.into());
        return;
    }
    if let Kind::Replacement { expiry } = job.kind {
        state.focus_intent.cancel(focus_scope);
        crate::playback_ui::resolved_authorized(
            app,
            state,
            playback,
            job.policy.max_height,
            job.selection,
            expiry,
        );
        return;
    }
    let Some(config) = &state.account_media_network else {
        state.focus_intent.cancel(focus_scope);
        app.set_status("The protected account media transport is unavailable.".into());
        return;
    };
    let restarting = matches!(job.kind, Kind::Restart { .. });
    if let Kind::Restart { load } = job.kind {
        let snapshot = state.player.snapshot();
        // Extraction ownership alone cannot authorize replacing a stopped or
        // newer native file. Retain the exact failed load and live account
        // lease through the final handoff, just as at retry admission.
        let expected = RestartScope {
            load,
            session: job.session,
        };
        if !restart_identity(state, &snapshot)
            .is_some_and(|(id, scope)| id == job.id && scope == expected)
        {
            state.focus_intent.cancel(focus_scope);
            app.set_status(
                "Account playback changed while retrying. The old retry was discarded.".into(),
            );
            return;
        }
    }
    let paused = restarting && state.player.user_pause_intent();
    if let Err(error) = crate::media_network::load_with_authorization(
        &state.player,
        &playback.playback,
        &playback.authorization,
        config,
        0.,
        paused,
    ) {
        state.focus_intent.cancel(focus_scope);
        app.set_status(error.into());
        if let Kind::Restart { load } = job.kind
            && state.player.snapshot().load_request_id == load
        {
            restart_failed(
                app,
                state,
                RestartScope {
                    load,
                    session: job.session,
                },
                WorkerError::Resolver(serein_core::ProviderError::UnsupportedFormat),
                None,
            );
        }
        return;
    }
    app.set_video_texture(slint::Image::default());
    state.account_playback.restart_deadline.stop();
    *state.account_playback.restart.borrow_mut() = RestartState::default();
    install(app, state, playback.authorization);
    let item = playback.playback;
    crate::caption_ui::clear_local(app, state);
    crate::comments_ui::account_details(app, state, &item.video.id, &item.details);
    app.global::<CaptionsUi>().set_status(
        "Account captions are not supported yet. No guest caption request will be made.".into(),
    );
    app.set_video_title(item.video.title.clone().into());
    app.set_video_channel(item.video.channel.clone().into());
    app.set_rating_known(false);
    app.set_remote_video(true);
    let quality =
        serein_core::QualityCeiling::from_height(job.policy.max_height).unwrap_or_default();
    app.set_quality_index(quality.index());
    state.quality_index.set(quality.index() as usize);
    crate::share_ui::clear(app, state);
    *state.current_video.borrow_mut() = Some(item.video.clone());
    crate::playback_ui::selected(app, state, &item);
    app.set_page(2);
    app.set_loaded(true);
    crate::focus_intent::apply(app, state, focus_scope);
    app.set_status("Account playback · Experimental ad filtering · Account captions and private local history are unavailable".into());
    state.account_ui.set_status(
        app,
        "Playing with this account. Disconnect stops playback and clears its private metadata.",
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fractional_account_cooldown_retains_readiness_until_its_exact_deadline() {
        let now = Instant::now();
        let scope = RestartScope {
            load: 9,
            session: 3,
        };
        let mut restart = RestartState::default();
        let delay = Duration::from_secs(61) + Duration::from_nanos(123_456);
        assert_eq!(
            restart.failed(
                scope,
                WorkerError::Account(AccountError::RateLimited),
                Some(delay),
                now
            ),
            Some(delay)
        );
        let early = now + delay - Duration::from_nanos(1);
        assert_eq!(
            restart.remaining(scope, early),
            Some(Duration::from_nanos(1))
        );
        assert!(!restart.ready(scope, early));
        assert!(restart.available(scope));
        assert_eq!(restart.remaining(scope, now + delay), Some(Duration::ZERO));
        assert!(restart.ready(scope, now + delay));
        assert_eq!(
            restart.remaining(
                RestartScope {
                    session: 4,
                    ..scope
                },
                early
            ),
            None
        );
        assert_eq!(
            restart.remaining(RestartScope { load: 10, ..scope }, early),
            None
        );
    }
    #[test]
    fn explicit_account_retries_keep_capped_attempt_delay_and_actual_provider_cooldown() {
        let mut restart = RestartState::default();
        let scope = RestartScope {
            load: 11,
            session: 4,
        };
        let mut now = Instant::now();
        for expected in [2, 4, 8, 16, 32, 60, 60] {
            let wait = restart
                .failed(
                    scope,
                    WorkerError::Account(AccountError::Offline),
                    None,
                    now,
                )
                .unwrap();
            assert_eq!(wait, Duration::from_secs(expected));
            assert!(restart.available(scope));
            assert!(!restart.ready(scope, now));
            now += wait;
            assert!(restart.ready(scope, now));
        }
        let wait = restart
            .failed(
                scope,
                WorkerError::Account(AccountError::RateLimited),
                Some(Duration::from_secs(123)),
                now,
            )
            .unwrap();
        assert_eq!(wait, Duration::from_secs(123));
        assert!(!restart.ready(scope, now + Duration::from_secs(122)));
        assert!(restart.ready(scope, now + wait));
        assert!(restart.ready(RestartScope { load: 12, ..scope }, now));
        assert!(restart.ready(
            RestartScope {
                session: 5,
                ..scope
            },
            now
        ));
        assert!(
            !restart.ready(scope, now),
            "checking other scopes must not reset this deadline"
        );
    }

    #[test]
    fn nonretryable_account_rejections_and_indefinite_cooldowns_stay_blocked() {
        use serein_core::ProviderError;
        let scope = RestartScope {
            load: 22,
            session: 6,
        };
        for error in [
            WorkerError::Account(AccountError::SessionExpired),
            WorkerError::Account(AccountError::IdentityNotVerified),
            WorkerError::Account(AccountError::StaleSession),
            WorkerError::Account(AccountError::UnsupportedAccount),
            WorkerError::Account(AccountError::ChallengeRequired),
            WorkerError::Account(AccountError::ReconciliationRequired),
            WorkerError::Resolver(ProviderError::AuthenticationRequired),
            WorkerError::Resolver(ProviderError::ProofRequired),
            WorkerError::Resolver(ProviderError::UnsafeMedia),
            WorkerError::Resolver(ProviderError::UnsupportedFormat),
        ] {
            let now = Instant::now();
            let mut restart = RestartState::default();
            assert_eq!(restart.failed(scope, error, None, now), None);
            assert!(!restart.available(scope));
            assert!(!restart.ready(scope, now + Duration::from_secs(3600)));
        }
        let mut restart = RestartState::default();
        assert_eq!(
            restart.failed(
                scope,
                WorkerError::Account(AccountError::RateLimited),
                Some(Duration::MAX),
                Instant::now()
            ),
            None
        );
        assert!(
            !restart.available(scope),
            "unrepresentable provider deadline cannot become a short retry or overflow a UI timer"
        );
    }

    #[test]
    fn restart_responses_share_initial_identity_guards_but_retain_old_native_load() {
        let id = VideoId::new("aaaaaaaaaaa").unwrap();
        let job = Job {
            id: id.clone(),
            policy: ResolutionPolicy {
                max_height: 720,
                prefer_h264: true,
            },
            kind: Kind::Restart { load: 37 },
            selection: 11,
            session: 4,
        };
        assert!(job.kind == Kind::Restart { load: 37 });
        assert_eq!(job.policy.max_height, 720);
        assert!(job.accepts_response(Some(11), 4, 11, 4));
        assert!(!job.accepts_response(Some(11), 4, 12, 4));
        assert!(!job.accepts_response(Some(11), 4, 11, 5));
        assert!(!job.matches(11, 5, &id));
        assert!(!job.matches(11, 4, &VideoId::new("bbbbbbbbbbb").unwrap()));
    }

    #[test]
    fn expired_initial_handoff_is_distinct_from_stale_or_unavailable_presentation() {
        assert_eq!(
            admission(true, false, false, true),
            Err(InstallRejection::Expired)
        );
        assert_eq!(
            admission(true, false, false, false),
            Err(InstallRejection::Expired)
        );
        // A foreign stale lease must never clear the currently connected identity.
        assert_eq!(
            admission(false, false, false, true),
            Err(InstallRejection::Stale)
        );
        assert_eq!(
            admission(true, true, false, false),
            Err(InstallRejection::Presentation)
        );
        assert_eq!(
            admission(true, true, true, true),
            Err(InstallRejection::Clearing)
        );
        assert_eq!(admission(true, true, false, true), Ok(()));
    }
    #[test]
    fn playback_routing_requires_exact_video_selection_and_session() {
        let first = VideoId::new("aaaaaaaaaaa").unwrap();
        let second = VideoId::new("bbbbbbbbbbb").unwrap();
        let job = Job {
            id: first.clone(),
            policy: ResolutionPolicy::default(),
            kind: Kind::Initial,
            selection: 9,
            session: 4,
        };
        assert!(job.matches(9, 4, &first));
        assert!(!job.matches(10, 4, &first));
        assert!(!job.matches(9, 5, &first));
        assert!(!job.matches(9, 4, &second));
        assert!(job.accepts_response(Some(9), 4, 9, 4));
        assert!(!job.accepts_response(None, 4, 9, 4));
        assert!(!job.accepts_response(Some(8), 4, 9, 4));
        assert!(!job.accepts_response(Some(9), 3, 9, 4));
        assert!(!job.accepts_response(Some(9), 4, 10, 4));
        assert!(!job.accepts_response(Some(9), 4, 9, 5));
    }
}
