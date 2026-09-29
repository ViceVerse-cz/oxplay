// SPDX-License-Identifier: GPL-3.0-or-later
//! Event-driven stream replacement. A fresh native position reply is required
//! before a replacement can load; hidden controls are not a playback clock.
use crate::{App, QUALITY_HEIGHTS, UiState, account_playback, caption_ui, catalog, load_remote};
use serein_core::{RefreshBudget, ResolvedPlayback};
use serein_youtube::account::{AccountPlaybackLease, AuthorizedPlayback};
use slint::{ComponentHandle, Timer, TimerMode};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, SystemTime},
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Reason {
    Quality,
    Expiry,
}
struct Pending {
    item: Box<ResolvedPlayback>,
    height: u16,
    token: u64,
    worker_generation: u64,
    reason: Reason,
    authorization: Option<AccountPlaybackLease>,
}
#[derive(Default)]
pub struct State {
    current: RefCell<Option<ResolvedPlayback>>,
    current_load: Cell<u64>,
    budget: RefCell<RefreshBudget>,
    due: Cell<bool>,
    refresh_job: Cell<Option<u64>>,
    pending: RefCell<Option<Pending>>,
    expiry: Timer,
    position_timeout: Timer,
}
const MARGIN: Duration = Duration::from_secs(60);

/// A small event-driven transport label, independent of catalog activity and
/// detailed error messages. Never publish a prior load's terminal state while
/// a newer accepted load is still waiting for its native START_FILE event.
pub fn status(
    snapshot: &serein_media::Snapshot,
    loaded: bool,
    current_load_is_active: bool,
) -> &'static str {
    use serein_media::PlaybackState;
    if !loaded || snapshot.stop_pending || snapshot.load_request_id == 0 {
        return "";
    }
    if snapshot.failed_load_request_id == Some(snapshot.load_request_id) {
        return "Playback failed";
    }
    if !current_load_is_active {
        // END_FILE can retire the native entry before the UI consumes its
        // final state. Only the matching accepted request may supply that state.
        if snapshot.load_request_id == snapshot.active_load_request_id {
            match snapshot.state {
                PlaybackState::Ended => return "Playback ended",
                PlaybackState::Failed => return "Playback failed",
                PlaybackState::Idle => return "Playback stopped",
                _ => {}
            }
        }
        return "Loading video…";
    }
    match snapshot.state {
        PlaybackState::Buffering if snapshot.paused_for_cache || snapshot.playback_restarted => {
            "Buffering…"
        }
        PlaybackState::Buffering => "Loading video…",
        PlaybackState::Seeking => "Seeking…",
        PlaybackState::Paused => "Paused",
        PlaybackState::Ended => "Playback ended",
        PlaybackState::Failed => "Playback failed",
        PlaybackState::Idle => "Playback stopped",
        PlaybackState::Playing => "",
    }
}

impl State {
    fn clear(&self) -> Option<u64> {
        self.expiry.stop();
        self.position_timeout.stop();
        self.current.borrow_mut().take();
        self.current_load.set(0);
        self.due.set(false);
        self.refresh_job.set(None);
        *self.budget.borrow_mut() = RefreshBudget::default();
        self.pending
            .borrow_mut()
            .take()
            .map(|pending| pending.token)
    }
}

impl State {
    fn account_retry_video(
        &self,
        load: u64,
        session: u64,
        displayed: Option<&serein_core::VideoId>,
    ) -> Option<serein_core::VideoId> {
        if load == 0 || self.current_load.get() != load {
            return None;
        }
        self.current.borrow().as_ref().and_then(|item| {
            (!item.guest && item.session_generation == session && displayed == Some(&item.video.id))
                .then(|| item.video.id.clone())
        })
    }

    fn guest_retry_video(
        &self,
        load: u64,
        displayed: Option<&serein_core::VideoId>,
    ) -> Option<serein_core::VideoId> {
        if load == 0 || self.current_load.get() != load {
            return None;
        }
        self.current.borrow().as_ref().and_then(|item| {
            (item.guest && item.session_generation == 0 && displayed == Some(&item.video.id))
                .then(|| item.video.id.clone())
        })
    }
}

/// Authority is checked by the account coordinator; metadata alone grants none.
pub fn account_retry_video(
    state: &UiState,
    load: u64,
    session: u64,
) -> Option<serein_core::VideoId> {
    state.playback_ui.account_retry_video(
        load,
        session,
        state.current_video.borrow().as_ref().map(|video| &video.id),
    )
}

/// Return only the typed identity of the guest stream accepted by this exact
/// native load. The caller must separately reject any retained account lease.
pub fn guest_retry_video(state: &UiState, load: u64) -> Option<serein_core::VideoId> {
    state.playback_ui.guest_retry_video(
        load,
        state.current_video.borrow().as_ref().map(|video| &video.id),
    )
}
/// Returns whether this coordinator owned a pending UI busy state. A caller
/// must release that state without clearing unrelated guest-worker activity.
pub fn clear_local(state: &UiState) -> bool {
    state.chapters_ui.invalidate();
    if let Some(token) = state.playback_ui.clear() {
        state.player.cancel_resume_position(token);
        true
    } else {
        false
    }
}

pub fn selected(app: &App, state: &Rc<UiState>, item: &ResolvedPlayback) {
    state
        .playback_ui
        .current_load
        .set(state.player.snapshot().load_request_id);
    *state.playback_ui.budget.borrow_mut() = RefreshBudget::default();
    if let Some(pending) = state.playback_ui.pending.borrow_mut().take() {
        state.player.cancel_resume_position(pending.token);
    }
    state.playback_ui.position_timeout.stop();
    state.playback_ui.refresh_job.set(None);
    crate::chapters_ui::install(app, state, item);
    remember(app, state, item);
}
fn remember(app: &App, state: &Rc<UiState>, item: &ResolvedPlayback) {
    let s = &state.playback_ui;
    s.expiry.stop();
    s.due.set(false);
    *s.current.borrow_mut() = Some(item.clone());
    let Some(expiry) = item.expires_at else {
        return;
    };
    let delay = expiry
        .duration_since(SystemTime::now())
        .unwrap_or_default()
        .saturating_sub(MARGIN)
        .min(Duration::from_secs(24 * 60 * 60));
    let weak = app.as_weak();
    let state = Rc::downgrade(state);
    s.expiry.start(TimerMode::SingleShot, delay, move || {
        let (Some(app), Some(state)) = (weak.upgrade(), state.upgrade()) else {
            return;
        };
        let current = state.playback_ui.current.borrow().clone();
        if let Some(item) = current.filter(|item| !item.expires_within(SystemTime::now(), MARGIN)) {
            remember(&app, &state, &item);
            return;
        }
        state.playback_ui.due.set(true);
        maybe_refresh(&app, &state);
    });
}
/// Native pause observations can lag a just-accepted user pause/navigation.
/// Both intent and exact native load identity must admit automatic replacement.
fn refresh_admitted(
    snapshot: &serein_media::Snapshot,
    load: u64,
    loaded: bool,
    watching: bool,
    presentation_ready: bool,
    user_paused: bool,
) -> bool {
    loaded
        && watching
        && presentation_ready
        && load != 0
        && snapshot.load_request_id == load
        && snapshot.active_load_request_id == load
        && snapshot.failed_load_request_id != Some(load)
        && !snapshot.stop_pending
        && !user_paused
        && !snapshot.paused
        && snapshot.state == serein_media::PlaybackState::Playing
}

/// A due refresh waits for active playback and the existing single extractor.
/// No periodic wakeups or retries are introduced while paused/hidden/busy.
pub fn maybe_refresh(app: &App, state: &Rc<UiState>) {
    let s = &state.playback_ui;
    if !s.due.get() || app.get_busy() || state.hidden.get() || s.pending.borrow().is_some() {
        return;
    }
    let snapshot = state.player.snapshot();
    if !refresh_admitted(
        &snapshot,
        s.current_load.get(),
        app.get_loaded(),
        app.get_page() == 2,
        state.presentation_ready.get(),
        state.player.user_pause_intent(),
    ) {
        // Retain due: a later real resume/media event can service this once.
        // No timer/poll or automatic extraction is started while admission fails.
        return;
    }
    let current = s.current.borrow();
    let Some(item) = current.as_ref() else { return };
    if state.current_video.borrow().as_ref().map(|v| &v.id) != Some(&item.video.id) {
        return;
    }
    let guest = item.guest;
    let session_generation = if guest {
        if item.session_generation != 0 {
            return;
        }
        0
    } else {
        if app.get_account_busy() || app.get_account_pending() {
            return;
        }
        let Some(authorization) = account_playback::authorization(state) else {
            return;
        };
        if !authorization.is_valid() || authorization.generation() != item.session_generation {
            return;
        }
        authorization.generation()
    };
    // A replacement keeps its original access scope. A failed authenticated
    // request is never retried anonymously, or vice versa.
    let request = s.budget.borrow_mut().request(
        item,
        Duration::ZERO,
        session_generation,
        SystemTime::now(),
        MARGIN,
    );
    s.due.set(false);
    let Some(request) = request else { return };
    drop(current);
    if !guest {
        if let Err(error) = account_playback::request_replacement(
            app,
            state,
            &request.video_id,
            serein_youtube::ResolutionPolicy {
                max_height: QUALITY_HEIGHTS[state.quality_index.get()],
                prefer_h264: true,
            },
            true,
        ) {
            app.set_status(error.into());
        }
        return;
    }
    state
        .worker
        .borrow_mut()
        .submit(catalog::Request::ResolveQuality(
            request.video_id,
            serein_youtube::ResolutionPolicy {
                max_height: QUALITY_HEIGHTS[state.quality_index.get()],
                prefer_h264: true,
            },
        ));
    s.refresh_job.set(Some(state.worker.borrow().generation()));
    app.set_busy(true);
    app.set_status("Refreshing an expiring guest stream… Current playback continues.".into());
}
pub fn resolved(app: &App, state: &Rc<UiState>, item: Box<ResolvedPlayback>, height: u16) {
    let s = &state.playback_ui;
    let generation = state.worker.borrow().generation();
    let reason = if s.refresh_job.take() == Some(generation) {
        Reason::Expiry
    } else {
        Reason::Quality
    };
    resolved_inner(app, state, item, height, generation, reason, None);
}
pub fn resolved_authorized(
    app: &App,
    state: &Rc<UiState>,
    item: Box<AuthorizedPlayback>,
    height: u16,
    selection: u64,
    expiry: bool,
) {
    let AuthorizedPlayback {
        playback,
        authorization,
    } = *item;
    resolved_inner(
        app,
        state,
        Box::new(playback),
        height,
        selection,
        if expiry {
            Reason::Expiry
        } else {
            Reason::Quality
        },
        Some(authorization),
    );
}
fn resolved_inner(
    app: &App,
    state: &Rc<UiState>,
    item: Box<ResolvedPlayback>,
    height: u16,
    generation: u64,
    reason: Reason,
    authorization: Option<AccountPlaybackLease>,
) {
    let s = &state.playback_ui;
    let scope_valid = match &authorization {
        Some(lease) => {
            !item.guest
                && lease.is_valid()
                && lease.generation() == item.session_generation
                && account_playback::is_current(
                    state,
                    &item.video.id,
                    generation,
                    item.session_generation,
                )
        }
        None => item.guest && item.session_generation == 0,
    };
    if !state.presentation_ready.get()
        || !scope_valid
        || state.current_video.borrow().as_ref().map(|v| &v.id) != Some(&item.video.id)
    {
        return;
    }
    let token = match state.player.request_resume_position() {
        Ok(token) => token,
        Err(error) => {
            app.set_status(error.to_string().into());
            return;
        }
    };
    *s.pending.borrow_mut() = Some(Pending {
        item,
        height,
        token,
        worker_generation: generation,
        reason,
        authorization,
    });
    app.set_busy(true);
    let weak = app.as_weak();
    let state = Rc::downgrade(state);
    s.position_timeout.start(TimerMode::SingleShot, Duration::from_secs(3), move || {
        let (Some(app), Some(state)) = (weak.upgrade(), state.upgrade()) else { return };
        let s = &state.playback_ui;
        if s.pending.borrow().as_ref().is_some_and(|p| p.token == token) {
            s.pending.borrow_mut().take();
            state.player.cancel_resume_position(token);
            app.set_busy(false);
            app.set_status("The player did not return a current position. Playback was left unchanged; retry the selection.".into());
        }
    });
}
pub fn observe(app: &App, state: &Rc<UiState>, snapshot: &serein_media::Snapshot) {
    let s = &state.playback_ui;
    let Some((token, position)) = snapshot.resume_position_reply else {
        return;
    };
    if !s
        .pending
        .borrow()
        .as_ref()
        .is_some_and(|p| p.token == token)
    {
        return;
    }
    let pending = s.pending.borrow_mut().take().unwrap();
    s.position_timeout.stop();
    app.set_busy(false);
    let scope_valid = if let Some(lease) = &pending.authorization {
        lease.is_valid()
            && lease.generation() == pending.item.session_generation
            && account_playback::is_current(
                state,
                &pending.item.video.id,
                pending.worker_generation,
                pending.item.session_generation,
            )
    } else {
        pending.item.guest
            && pending.item.session_generation == 0
            && pending.worker_generation == state.worker.borrow().generation()
    };
    if !scope_valid
        || state.current_video.borrow().as_ref().map(|v| &v.id) != Some(&pending.item.video.id)
    {
        return;
    }
    if !state.presentation_ready.get() {
        app.set_status("Video presentation became unavailable. Activate the display, then retry the selection.".into());
        return;
    }
    let Some(position) = position else {
        app.set_status(
            "The current playback position is unavailable. Select the video again to restart."
                .into(),
        );
        return;
    };
    // Preserve accepted user intent across replacement. Native pause may lag
    // the last command, or reflect the independent hidden-window overlay.
    let user_paused = state.player.user_pause_intent();
    let result = if let Some(authorization) = &pending.authorization {
        match &state.account_media_network {
            Some(config) => crate::media_network::load_with_authorization(
                &state.player,
                &pending.item,
                authorization,
                config,
                position,
                user_paused,
            ),
            None => Err("The account media transport is unavailable.".into()),
        }
    } else {
        load_remote(state, &pending.item, position, user_paused)
    };
    match result {
        Ok(()) => {
            state
                .playback_ui
                .current_load
                .set(state.player.snapshot().load_request_id);
            app.set_video_texture(slint::Image::default());
            let authenticated = pending.authorization.is_some();
            if let Some(authorization) = pending.authorization {
                account_playback::install(app, state, authorization);
                caption_ui::clear_local(app, state);
                app.global::<crate::CaptionsUi>()
                    .set_status("Captions for connected playback are not supported yet.".into());
            } else {
                caption_ui::metadata(app, state, &pending.item, true);
            }
            let index = QUALITY_HEIGHTS
                .iter()
                .position(|height| *height == pending.height)
                .unwrap_or(0);
            state.quality_index.set(index);
            app.set_quality_index(index as i32);
            let saved = pending.reason != Reason::Quality
                || crate::library_ui::save_quality(
                    app,
                    state,
                    serein_core::QualityCeiling::ALL[index],
                );
            crate::chapters_ui::install(app, state, &pending.item);
            remember(app, state, &pending.item);
            app.set_status(
                match pending.reason {
                    Reason::Quality => format!(
                        "Maximum quality {}p · Reopening at the current position…{}",
                        pending.height,
                        if saved { "" } else { " This video's quality changed, but the default for future videos was not saved." }
                    ),
                    Reason::Expiry => {
                        if authenticated {
                            "Connected stream refreshed once · Resuming at the current position…"
                                .into()
                        } else {
                            "Guest stream refreshed once · Resuming at the current position…".into()
                        }
                    }
                }
                .into(),
            );
        }
        Err(error) => app.set_status(error.into()),
    }
}
pub fn bind(app: &App, state: &Rc<UiState>) {
    let weak = app.as_weak();
    let state = Rc::downgrade(state);
    state
        .upgrade()
        .unwrap()
        .worker
        .borrow_mut()
        .on_generation_changed(move |generation| {
            let (Some(app), Some(state)) = (weak.upgrade(), state.upgrade()) else {
                return;
            };
            let s = &state.playback_ui;
            crate::guest_playback::cancel(&app, &state);
            account_playback::cancel_pending(&app, &state);
            if s.pending
                .borrow()
                .as_ref()
                .is_some_and(|p| p.authorization.is_some() || p.worker_generation != generation)
            {
                if let Some(pending) = s.pending.borrow_mut().take() {
                    state.player.cancel_resume_position(pending.token);
                }
                s.position_timeout.stop();
                app.set_busy(false);
            }
            if s.refresh_job.get().is_some_and(|job| job != generation) {
                s.refresh_job.set(None);
            }
        });
}

/// Finite live diagnostic. Only this explicit mode changes expiry metadata;
/// signed provider addresses are never modified or logged.
pub struct Smoke {
    verified: Rc<Cell<bool>>,
    _timers: Vec<Timer>,
}
impl Smoke {
    pub fn start(app: &App, state: &Rc<UiState>) -> Self {
        let verified = Rc::new(Cell::new(false));
        let mut timers = Vec::new();
        let generation = Rc::new(Cell::new(0));
        let audio_token = Rc::new(Cell::new(0));
        let first_audio = Rc::new(Cell::new(None::<serein_media::AudioProbeReply>));
        for stage in [12, 16, 20, 25, 55, 60, 65] {
            let weak = app.as_weak();
            let state = state.clone();
            let verified = verified.clone();
            let generation = generation.clone();
            let audio_token = audio_token.clone();
            let first_audio = first_audio.clone();
            let timer = Timer::default();
            timer.start(
                TimerMode::SingleShot,
                Duration::from_secs(stage),
                move || {
                    let Some(app) = weak.upgrade() else { return };
                    let snapshot = state.player.snapshot();
                    assert!(snapshot.error.is_none(), "refresh smoke native error");
                    match stage {
                        12 => {
                            assert!(
                                app.get_remote_video()
                                    && !app.get_busy()
                                    && snapshot.file_loads == 1,
                                "refresh smoke requires loaded public media"
                            );
                            state.player.set_paused(true).unwrap();
                            state.player.seek(45.).unwrap();
                        }
                        16 => {
                            assert!(snapshot.paused && (snapshot.position - 45.).abs() < 0.5);
                            app.set_controls_visible(false);
                            crate::update(&app, &state);
                            state.playback_ui.expiry.stop();
                            state
                                .playback_ui
                                .current
                                .borrow_mut()
                                .as_mut()
                                .unwrap()
                                .expires_at = Some(SystemTime::now());
                            state.playback_ui.due.set(true);
                            generation.set(state.worker.borrow().generation());
                            maybe_refresh(&app, &state);
                        }
                        20 => {
                            assert_eq!(
                                state.worker.borrow().generation(),
                                generation.get(),
                                "paused expiry must not start extraction"
                            );
                            // Let playback advance with its UI clock stopped.
                            state.playback_ui.due.set(false);
                            state.player.set_paused(false).unwrap();
                        }
                        25 => {
                            assert!(
                                !snapshot.paused && !state.hidden.get(),
                                "refresh smoke window was paused/occluded"
                            );
                            assert!(!state.progress.running());
                            state.playback_ui.due.set(true);
                            maybe_refresh(&app, &state);
                        }
                        55 => {
                            assert_eq!(snapshot.file_loads, 2, "audio probe requires replacement");
                            audio_token.set(state.player.request_audio_probe().unwrap());
                        }
                        60 => {
                            let audio = snapshot.audio_probe_reply.expect("first fresh audio reply");
                            assert_eq!(audio.token, audio_token.get());
                            assert_eq!(audio.load_request_id, snapshot.active_load_request_id);
                            assert!(audio.decoder_sample_rate.is_some_and(|rate| rate > 0));
                            assert!(audio.output_sample_rate.is_some_and(|rate| rate > 0));
                            assert!(audio.audio_pts.is_some());
                            eprintln!("refresh audio first={audio:?}");
                            first_audio.set(Some(audio));
                            audio_token.set(state.player.request_audio_probe().unwrap());
                            eprintln!(
                                "refresh diagnostic state={:?} position={:.3} busy={} due={} resolving={} waiting_position={} error_present={}",
                                snapshot.state, snapshot.position, app.get_busy(),
                                state.playback_ui.due.get(), state.playback_ui.refresh_job.get().is_some(),
                                state.playback_ui.pending.borrow().is_some(), snapshot.error.is_some()
                            );
                            assert_eq!(snapshot.file_loads, 2, "one stream refresh must finish");
                            assert!(!app.get_busy());
                            assert!(
                                snapshot.position >= 49.,
                                "replacement resumed from a stale UI position"
                            );
                            generation.set(state.worker.borrow().generation());
                            state.playback_ui.expiry.stop();
                            state
                                .playback_ui
                                .current
                                .borrow_mut()
                                .as_mut()
                                .unwrap()
                                .expires_at = Some(SystemTime::now());
                            state.playback_ui.due.set(true);
                            maybe_refresh(&app, &state);
                        }
                        _ => {
                            let first = first_audio.get().expect("first audio sample retained");
                            let second = snapshot.audio_probe_reply.expect("second fresh audio reply");
                            assert_eq!(second.token, audio_token.get());
                            assert_eq!(second.load_request_id, first.load_request_id);
                            assert_eq!(second.load_request_id, snapshot.active_load_request_id);
                            assert!(second.decoder_sample_rate.is_some_and(|rate| rate > 0));
                            assert!(second.output_sample_rate.is_some_and(|rate| rate > 0));
                            assert!(second.audio_pts.unwrap() - first.audio_pts.unwrap() >= 2.,
                                "native output audio clock must advance after replacement");
                            eprintln!("refresh audio second={second:?}");
                            assert_eq!(
                                state.worker.borrow().generation(),
                                generation.get(),
                                "expiry must not cause a retry loop"
                            );
                            assert_eq!(snapshot.file_loads, 2);
                            assert_eq!(state.model.changes.get(), 0);
                            assert_eq!(state.model.resets.get(), 0);
                            verified.set(true);
                        }
                    }
                    eprintln!(
                        "refresh smoke stage={stage} loads={} position={:.3} paused={}",
                        snapshot.file_loads, snapshot.position, snapshot.paused
                    );
                },
            );
            timers.push(timer);
        }
        Self {
            verified,
            _timers: timers,
        }
    }
    pub fn finish(self) -> Result<(), &'static str> {
        if self.verified.get() {
            Ok(())
        } else {
            Err("refresh smoke ended before all assertions completed")
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn automatic_refresh_respects_pause_intent_navigation_and_exact_live_load() {
        use serein_media::{PlaybackState, Snapshot};
        let playing = Snapshot {
            load_request_id: 12,
            active_load_request_id: 12,
            state: PlaybackState::Playing,
            paused: false,
            ..Snapshot::default()
        };
        assert!(super::refresh_admitted(
            &playing, 12, true, true, true, false
        ));
        assert!(
            !super::refresh_admitted(&playing, 12, true, true, true, true),
            "an accepted pause must win before its native observation arrives"
        );
        assert!(
            !super::refresh_admitted(&playing, 12, true, false, true, false),
            "leaving watch cannot launch extraction from a stale Playing snapshot"
        );
        assert!(!super::refresh_admitted(
            &playing, 12, false, true, true, false
        ));
        assert!(!super::refresh_admitted(
            &playing, 12, true, true, false, false
        ));
        assert!(!super::refresh_admitted(
            &playing, 0, true, true, true, false
        ));
        assert!(!super::refresh_admitted(
            &playing, 13, true, true, true, false
        ));
        for stale in [
            Snapshot {
                paused: true,
                ..playing.clone()
            },
            Snapshot {
                stop_pending: true,
                ..playing.clone()
            },
            Snapshot {
                load_request_id: 13,
                ..playing.clone()
            },
            Snapshot {
                active_load_request_id: 11,
                ..playing.clone()
            },
            Snapshot {
                failed_load_request_id: Some(12),
                ..playing.clone()
            },
            Snapshot {
                state: PlaybackState::Seeking,
                ..playing.clone()
            },
            Snapshot {
                state: PlaybackState::Failed,
                ..playing.clone()
            },
        ] {
            assert!(!super::refresh_admitted(
                &stale, 12, true, true, true, false
            ));
        }
        assert!(
            super::refresh_admitted(&playing, 12, true, true, true, false),
            "the same due load can become admissible after actual resume"
        );
    }
    use super::*;
    use serein_core::{MediaTrack, MediaUrl, OriginHeaders, VideoId, VideoSummary};

    #[test]
    fn transport_label_does_not_republish_old_load_or_account_stop_state() {
        use serein_media::{PlaybackState, Snapshot};
        let mut snapshot = Snapshot {
            load_request_id: 42,
            active_load_request_id: 41,
            failed_load_request_id: Some(41),
            state: PlaybackState::Failed,
            ..Snapshot::default()
        };
        assert_eq!(status(&snapshot, true, false), "Loading video…");
        snapshot.state = PlaybackState::Ended;
        assert_eq!(status(&snapshot, true, false), "Loading video…");
        snapshot.active_load_request_id = 42;
        assert_eq!(status(&snapshot, true, false), "Playback ended");
        snapshot.failed_load_request_id = Some(42);
        assert_eq!(status(&snapshot, true, false), "Playback failed");
        assert_eq!(status(&snapshot, false, false), "");
        snapshot.stop_pending = true;
        assert_eq!(status(&snapshot, true, false), "");
        snapshot.stop_pending = false;
        snapshot.load_request_id = 0;
        assert_eq!(status(&snapshot, true, false), "");
    }

    #[test]
    fn transport_label_follows_observed_cache_and_seek_recovery() {
        use serein_media::{PlaybackState, Snapshot};
        let mut snapshot = Snapshot {
            load_request_id: 42,
            active_load_request_id: 42,
            state: PlaybackState::Buffering,
            ..Snapshot::default()
        };
        assert_eq!(status(&snapshot, true, true), "Loading video…");
        snapshot.playback_restarted = true;
        snapshot.state = PlaybackState::Playing;
        assert_eq!(status(&snapshot, true, true), "");
        snapshot.paused_for_cache = true;
        snapshot.state = PlaybackState::Buffering;
        assert_eq!(status(&snapshot, true, true), "Buffering…");
        snapshot.paused_for_cache = false;
        snapshot.state = PlaybackState::Playing;
        assert_eq!(status(&snapshot, true, true), "");
        snapshot.playback_restarted = false;
        snapshot.state = PlaybackState::Seeking;
        assert_eq!(status(&snapshot, true, true), "Seeking…");
        snapshot.paused = true;
        snapshot.playback_restarted = true;
        snapshot.state = PlaybackState::Paused;
        assert_eq!(status(&snapshot, true, true), "Paused");
    }

    fn synthetic_playback() -> ResolvedPlayback {
        ResolvedPlayback {
            video: VideoSummary {
                id: VideoId::new("aaaaaaaaaaa").unwrap(),
                title: "Synthetic private title".into(),
                channel: "Synthetic channel".into(),
                channel_id: None,
                duration: None,
                thumbnail_url: None,
            },
            details: Default::default(),
            video_track: MediaTrack {
                url: MediaUrl::parse("https://synthetic.googlevideo.com/videoplayback").unwrap(),
                codec: None,
                width: None,
                height: None,
                fps: None,
                contains_audio: true,
                headers: OriginHeaders {
                    origin: "https://synthetic.googlevideo.com".into(),
                    fields: vec![],
                },
            },
            audio_track: None,
            subtitles: vec![],
            subtitles_truncated: false,
            expires_at: None,
            session_generation: 7,
            guest: false,
        }
    }

    #[test]
    fn retry_identity_requires_current_guest_metadata_and_exact_accepted_load() {
        let s = State::default();
        let mut item = synthetic_playback();
        let id = item.video.id.clone();
        s.current_load.set(19);
        for (guest, session) in [(false, 7), (true, 7), (false, 0)] {
            item.guest = guest;
            item.session_generation = session;
            *s.current.borrow_mut() = Some(item.clone());
            assert_eq!(s.guest_retry_video(19, Some(&id)), None);
        }
        item.guest = true;
        item.session_generation = 0;
        *s.current.borrow_mut() = Some(item);
        assert_eq!(s.guest_retry_video(19, Some(&id)), Some(id.clone()));
        assert_eq!(s.guest_retry_video(20, Some(&id)), None);
        assert_eq!(s.guest_retry_video(0, Some(&id)), None);
        assert_eq!(s.guest_retry_video(19, None), None);
        assert_eq!(
            s.guest_retry_video(19, Some(&VideoId::new("bbbbbbbbbbb").unwrap())),
            None
        );
        s.clear();
        assert_eq!(s.current_load.get(), 0);
        assert_eq!(s.guest_retry_video(19, Some(&id)), None);
    }

    #[test]
    fn account_restart_metadata_never_accepts_guest_wrong_session_or_old_load() {
        let s = State::default();
        let mut item = synthetic_playback();
        let id = item.video.id.clone();
        s.current_load.set(12);
        *s.current.borrow_mut() = Some(item.clone());
        assert_eq!(s.account_retry_video(12, 7, Some(&id)), Some(id.clone()));
        assert_eq!(s.account_retry_video(11, 7, Some(&id)), None);
        assert_eq!(s.account_retry_video(12, 8, Some(&id)), None);
        assert_eq!(s.account_retry_video(12, 7, None), None);
        item.guest = true;
        *s.current.borrow_mut() = Some(item);
        assert_eq!(s.account_retry_video(12, 7, Some(&id)), None);
        s.clear();
        assert_eq!(s.account_retry_video(12, 7, Some(&id)), None);
    }

    #[test]
    fn clearing_replacement_releases_its_busy_owner_and_private_metadata_once() {
        let s = State::default();
        let item = synthetic_playback();
        *s.current.borrow_mut() = Some(item.clone());
        *s.pending.borrow_mut() = Some(Pending {
            item: Box::new(item),
            height: 720,
            token: 19,
            worker_generation: 12,
            reason: Reason::Quality,
            // This test exercises cancellation only; it creates no playback authority.
            authorization: None,
        });
        s.due.set(true);
        s.refresh_job.set(Some(12));
        assert_eq!(
            s.clear(),
            Some(19),
            "release exactly this pending busy owner"
        );
        assert!(s.current.borrow().is_none());
        assert!(s.pending.borrow().is_none());
        assert!(!s.due.get());
        assert_eq!(s.refresh_job.get(), None);
        assert_eq!(
            s.clear(),
            None,
            "do not clear another operation's busy state"
        );
    }
}
