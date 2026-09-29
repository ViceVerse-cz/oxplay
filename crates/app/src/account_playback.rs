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
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Initial,
    Replacement { expiry: bool },
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
    if state.account_playback.active.borrow_mut().take().is_some() {
        // The guest load was accepted, but its first frame may not exist yet.
        // Do not retain a private account frame under the new guest identity.
        app.set_video_texture(slint::Image::default());
    }
    app.set_account_playback_active(false);
}
pub fn clear(app: &App, state: &Rc<UiState>) {
    cancel_pending(app, state);
    state.account_playback.expiry.stop();
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
        app.set_account_status("The account session expired or was disconnected. Reconnect explicitly to play with this account.".into());
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
    app.set_account_status(
        "Resolving this video with the explicitly selected account session…".into(),
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
            app.set_account_status(error.to_string().into());
            app.set_status(error.to_string().into());
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
        app.set_account_status(message.into());
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
    if let Err(error) = crate::media_network::load_with_authorization(
        &state.player,
        &playback.playback,
        &playback.authorization,
        config,
        0.,
        false,
    ) {
        state.focus_intent.cancel(focus_scope);
        app.set_status(error.into());
        return;
    }
    app.set_video_texture(slint::Image::default());
    install(app, state, playback.authorization);
    let item = playback.playback;
    crate::caption_ui::clear_local(app, state);
    crate::comments_ui::clear_local(app, state);
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
    *state.current_video.borrow_mut() = Some(item.video.clone());
    crate::playback_ui::selected(app, state, &item);
    app.set_page(2);
    app.set_loaded(true);
    crate::focus_intent::apply(app, state, focus_scope);
    app.set_status("Account playback · Experimental ad filtering · Account captions and private local history are unavailable".into());
    app.set_account_status(
        "Playing with this account. Disconnect stops playback and clears its private metadata."
            .into(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
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
