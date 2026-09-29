// SPDX-License-Identifier: GPL-3.0-or-later
//! Guest selection crosses an account boundary only after terminal media stop.
use crate::{App, UiState, account_playback, caption_ui, comments_ui, load_remote, playback_ui};
use serein_core::ResolvedPlayback;
use slint::{ComponentHandle, Timer, TimerMode};
use std::{cell::RefCell, rc::Rc, time::Duration};

pub fn failed_load(snapshot: &serein_media::Snapshot) -> bool {
    snapshot.load_request_id != 0
        && (snapshot.failed_load_request_id == Some(snapshot.load_request_id)
            || (snapshot.active_load_request_id == snapshot.load_request_id
                && snapshot.state == serein_media::PlaybackState::Failed))
}

/// Availability describes the current failed guest file, independently of a
/// concurrent resolver's busy state. The action rechecks all admission guards.
pub fn can_retry(app: &App, state: &UiState, snapshot: &serein_media::Snapshot) -> bool {
    app.get_loaded()
        && !snapshot.stop_pending
        && failed_load(snapshot)
        && state.guest_playback.retry.borrow().blocked_load != Some(snapshot.load_request_id)
        // Even an expired lease remains account-scoped; never fall back to guest.
        && account_playback::authorization(state).is_none()
        && playback_ui::guest_retry_video(state, snapshot.load_request_id).is_some()
}

/// True means the terminal action was handled, including blocked admission.
/// Never let a busy/clearing failed file fall through to a native pause command.
pub fn retry_failed(app: &App, state: &Rc<UiState>) -> bool {
    let snapshot = state.player.snapshot();
    if !failed_load(&snapshot) {
        return false;
    }
    if !can_retry(app, state, &snapshot) {
        return true;
    }
    if app.get_busy()
        || app.get_account_busy()
        || state.caption_cache.active()
        || state.library_fixture.is_some()
        || state.native_child.enabled
        || !state.playback_preferences.ready()
        || app.get_page() != 2
    {
        return true;
    }
    if !state.presentation_ready.get() {
        app.set_status("Activate the display before retrying playback.".into());
        return true;
    }
    let Some(id) = playback_ui::guest_retry_video(state, snapshot.load_request_id) else {
        return true;
    };
    let Some(quality) = i32::try_from(state.quality_index.get())
        .ok()
        .and_then(serein_core::QualityCeiling::from_index)
    else {
        return true;
    };
    if let Some(serial) = crate::guest_recovery::video_failure_serial(app, state, &id, quality) {
        if app.invoke_retry_request(serial) {
            app.set_status(
                "Resolving a fresh guest stream… Playback will restart from the beginning.".into(),
            );
        }
        return true;
    }
    // Retain the acknowledged metadata until the normal initial-load response
    // succeeds. No signed URL, native position, or account credential is reused.
    state
        .worker
        .borrow_mut()
        .submit(crate::catalog::Request::Resolve(id.clone(), quality));
    retry_submitted(state, &id, quality);
    state.focus_intent.arm(
        crate::focus_intent::Scope::Guest(state.worker.borrow().generation()),
        app.get_search_active(),
    );
    app.set_busy(true);
    app.set_status(
        "Resolving a fresh guest stream… Playback will restart from the beginning.".into(),
    );
    true
}

#[derive(Default)]
struct RetryState {
    attempt: Option<(u64, u64)>, // worker generation, accepted native load
    blocked_load: Option<u64>,
}
impl RetryState {
    fn failed(&mut self, generation: u64, load: u64, error: serein_core::ProviderError) {
        if self.attempt != Some((generation, load)) {
            return;
        }
        self.attempt = None;
        if !crate::guest_recovery::retryable(error) {
            self.blocked_load = Some(load);
        }
    }
}

/// Called only after an explicit restart/recovery submission has finished its
/// synchronous generation callbacks. Ordinary account work cannot enter here.
pub fn retry_submitted(
    state: &UiState,
    id: &serein_core::VideoId,
    quality: serein_core::QualityCeiling,
) {
    let snapshot = state.player.snapshot();
    if failed_load(&snapshot)
        && !snapshot.stop_pending
        && account_playback::authorization(state).is_none()
        && playback_ui::guest_retry_video(state, snapshot.load_request_id).as_ref() == Some(id)
        && usize::try_from(quality.index()).ok() == Some(state.quality_index.get())
    {
        state.guest_playback.retry.borrow_mut().attempt =
            Some((state.worker.borrow().generation(), snapshot.load_request_id));
    }
}

pub fn resolution_failed(
    app: &App,
    state: &UiState,
    generation: u64,
    error: serein_core::ProviderError,
) {
    if state.worker.borrow().generation() != generation {
        return;
    }
    let snapshot = state.player.snapshot();
    state
        .guest_playback
        .retry
        .borrow_mut()
        .failed(generation, snapshot.load_request_id, error);
    // The failed native file may produce no further wake; retire UI admission
    // in the same terminal worker callback rather than waiting for one.
    app.set_can_retry_playback(can_retry(app, state, &snapshot));
}

struct Pending {
    generation: u64,
    item: Box<ResolvedPlayback>,
    quality: serein_core::QualityCeiling,
}
#[derive(Default)]
pub struct State {
    pending: RefCell<Option<Pending>>,
    retry: RefCell<RetryState>,
    deadline: Timer,
}
pub fn cancel(app: &App, state: &UiState) {
    state.guest_playback.retry.borrow_mut().attempt = None;
    state.guest_playback.deadline.stop();
    if let Some(pending) = state.guest_playback.pending.borrow_mut().take() {
        state
            .focus_intent
            .cancel(crate::focus_intent::Scope::Guest(pending.generation));
        app.set_busy(false);
    }
}
pub fn receive(
    app: &App,
    state: &Rc<UiState>,
    item: Box<ResolvedPlayback>,
    quality: serein_core::QualityCeiling,
) {
    if !item.guest || item.session_generation != 0 {
        state.focus_intent.cancel(crate::focus_intent::Scope::Guest(
            state.worker.borrow().generation(),
        ));
        app.set_status("A non-guest result was rejected by guest playback.".into());
        return;
    }
    if account_playback::authorization(state).is_some() {
        account_playback::clear(app, state);
    }
    if !state.player.snapshot().stop_pending {
        publish(app, state, *item, quality);
        return;
    }
    cancel(app, state);
    let generation = state.worker.borrow().generation();
    *state.guest_playback.pending.borrow_mut() = Some(Pending {
        generation,
        item,
        quality,
    });
    app.set_busy(true);
    app.set_status("Stopping previous playback before starting guest playback…".into());
    let weak = app.as_weak();
    let state_weak = Rc::downgrade(state);
    state.guest_playback.deadline.start(TimerMode::SingleShot, Duration::from_secs(3), move || {
        let (Some(app), Some(state)) = (weak.upgrade(), state_weak.upgrade()) else { return };
        if state.guest_playback.pending.borrow().as_ref().is_some_and(|job| job.generation == generation) {
            cancel(&app, &state);
            app.set_status("The previous player has not finished stopping. Select the video again to retry.".into());
        }
    });
}
pub fn observe(app: &App, state: &Rc<UiState>, snapshot: &serein_media::Snapshot) {
    if state.guest_playback.pending.borrow().is_none() {
        return;
    }
    if snapshot.stop_pending {
        return;
    }
    let pending = state.guest_playback.pending.borrow_mut().take().unwrap();
    state.guest_playback.deadline.stop();
    app.set_busy(false);
    if pending.generation == state.worker.borrow().generation() && !state.caption_cache.active() {
        publish(app, state, *pending.item, pending.quality);
    }
}
fn publish(
    app: &App,
    state: &Rc<UiState>,
    item: ResolvedPlayback,
    quality: serein_core::QualityCeiling,
) {
    let focus_scope = crate::focus_intent::Scope::Guest(state.worker.borrow().generation());
    if !state.presentation_ready.get() {
        state.focus_intent.cancel(focus_scope);
        app.set_status(
            "Video presentation is unavailable. Activate the display, then select the video again."
                .into(),
        );
        return;
    }
    if let Err(error) = load_remote(state, &item, 0., false) {
        state.focus_intent.cancel(focus_scope);
        app.set_status(error.into());
        return;
    }
    account_playback::leave_for_guest(app, state);
    app.set_video_texture(slint::Image::default());
    app.set_rating_known(false);
    app.set_remote_video(true);
    app.set_quality_index(quality.index());
    state.quality_index.set(quality.index() as usize);
    app.set_video_title(item.video.title.clone().into());
    app.set_video_channel(item.video.channel.clone().into());
    comments_ui::details(app, state, &item.video.id, &item.details);
    caption_ui::metadata(app, state, &item, false);
    *state.current_video.borrow_mut() = Some(item.video.clone());
    playback_ui::selected(app, state, &item);
    app.set_page(2);
    app.set_loaded(true);
    crate::focus_intent::apply(app, state, focus_scope);
    app.set_status("Guest playback · Ad filtering is experimental and may miss some ads".into());
}

#[cfg(test)]
mod tests {
    use super::*;
    use serein_core::ProviderError;
    use serein_media::{PlaybackState, Snapshot};

    #[test]
    fn only_the_current_loads_actual_terminal_failure_can_restart() {
        let mut snapshot = Snapshot {
            load_request_id: 8,
            active_load_request_id: 7,
            failed_load_request_id: Some(7),
            state: PlaybackState::Failed,
            error: Some("A prior command failed".into()),
            ..Snapshot::default()
        };
        assert!(!failed_load(&snapshot));
        snapshot.failed_load_request_id = Some(8);
        assert!(
            failed_load(&snapshot),
            "current load rejection before START_FILE"
        );
        snapshot.failed_load_request_id = None;
        snapshot.active_load_request_id = 8;
        assert!(
            failed_load(&snapshot),
            "current file's terminal native failure"
        );
        for state in [
            PlaybackState::Idle,
            PlaybackState::Buffering,
            PlaybackState::Playing,
            PlaybackState::Paused,
            PlaybackState::Seeking,
            PlaybackState::Ended,
        ] {
            snapshot.state = state;
            assert!(
                !failed_load(&snapshot),
                "generic error cannot restart {state:?}"
            );
        }
        snapshot.load_request_id = 0;
        snapshot.active_load_request_id = 0;
        snapshot.failed_load_request_id = Some(0);
        snapshot.state = PlaybackState::Failed;
        assert!(!failed_load(&snapshot), "stopped has no retryable load");
    }

    #[test]
    fn terminal_resolution_policy_blocks_only_its_exact_restart_load() {
        for error in [
            ProviderError::AuthenticationRequired,
            ProviderError::ProofRequired,
            ProviderError::UnsupportedFormat,
            ProviderError::UnsafeMedia,
            ProviderError::Unavailable,
        ] {
            let mut retry = RetryState {
                attempt: Some((4, 19)),
                ..Default::default()
            };
            retry.failed(3, 19, error);
            retry.failed(4, 18, error);
            assert_eq!(retry.attempt, Some((4, 19)));
            assert_eq!(retry.blocked_load, None);
            retry.failed(4, 19, error);
            assert_eq!(retry.attempt, None);
            assert_eq!(retry.blocked_load, Some(19));
            // Unrelated navigation cancellation never reauthorizes this file;
            // a genuinely new native load has a distinct token.
            retry.attempt = None;
            retry.failed(5, 19, ProviderError::Offline);
            assert_eq!(retry.blocked_load, Some(19));
            assert_ne!(retry.blocked_load, Some(20));
        }
    }

    #[test]
    fn recoverable_resolution_errors_preserve_the_existing_manual_retry_path() {
        for error in [
            ProviderError::Offline,
            ProviderError::Timeout,
            ProviderError::ExtractorFailed,
            ProviderError::RateLimited,
        ] {
            let mut retry = RetryState {
                attempt: Some((7, 23)),
                ..Default::default()
            };
            retry.failed(7, 23, error);
            assert_eq!(retry.attempt, None);
            assert_eq!(retry.blocked_load, None);
        }
        let mut cancelled = RetryState::default();
        cancelled.failed(7, 23, ProviderError::AuthenticationRequired);
        assert_eq!(cancelled.blocked_load, None);
    }
}
