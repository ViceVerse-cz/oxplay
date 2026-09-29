// SPDX-License-Identifier: GPL-3.0-or-later
//! One explicit guest recovery action. Retains typed request parameters, never
//! credentials, resolved media URLs, UI search text, or account operations.
use crate::{App, UiState, catalog::Request};
use serein_core::{ProviderError, QualityCeiling, VideoId};
use serein_youtube::catalog::{CatalogCursor, CatalogRequest};
use slint::{ComponentHandle, Timer, TimerMode};
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

#[derive(Clone)]
enum Target {
    Catalog(CatalogRequest, Option<CatalogCursor>),
    Video(VideoId, QualityCeiling),
}
impl Target {
    fn from_request(request: &Request) -> Option<Self> {
        match request {
            Request::Catalog(request, cursor) => {
                Some(Self::Catalog(request.clone(), cursor.clone()))
            }
            Request::Resolve(id, quality) => Some(Self::Video(id.clone(), *quality)),
            // Comments, captions and replacements have their own ownership and
            // recovery rules. An account request cannot enter this worker.
            _ => None,
        }
    }
    fn request(self) -> Request {
        match self {
            Self::Catalog(request, cursor) => Request::Catalog(request, cursor),
            Self::Video(id, quality) => Request::Resolve(id, quality),
        }
    }
    fn label(&self) -> &'static str {
        match self {
            Self::Catalog(CatalogRequest::Search { .. }, _) => "Retry search",
            Self::Catalog(_, _) => "Retry catalog",
            Self::Video(_, _) => "Retry video",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Scope {
    generation: u64,
    session: u64,
    selection: u64,
}
struct Job {
    target: Target,
    scope: Scope,
    attempt: u8,
    failure: Option<Failure>,
}
struct Failure {
    serial: i32,
    page: i32,
    deadline: Instant,
}
#[derive(Default)]
struct Ledger {
    job: Option<Job>,
    serial: i32,
}
fn delay(error: ProviderError, attempt: u8) -> Option<Duration> {
    match error {
        ProviderError::Offline | ProviderError::Timeout | ProviderError::ExtractorFailed => {
            Some(Duration::from_secs((2_u64 << attempt.min(5)).min(60)))
        }
        // The supervised extractor currently exposes only this category, not a
        // Retry-After duration. Its shared provider cooldown remains enforced.
        ProviderError::RateLimited => Some(Duration::from_secs(60)),
        _ => None,
    }
}
pub fn retryable(error: ProviderError) -> bool {
    delay(error, 0).is_some()
}
impl Ledger {
    fn begin(&mut self, target: Option<Target>, scope: Scope) {
        self.job = target.map(|target| Job {
            target,
            scope,
            attempt: 0,
            failure: None,
        });
    }
    fn fail(
        &mut self,
        scope: Scope,
        page: i32,
        error: ProviderError,
        now: Instant,
    ) -> Option<(i32, Duration, &'static str)> {
        let job = self.job.as_mut()?;
        if job.scope != scope {
            return None;
        }
        let Some(delay) = delay(error, job.attempt) else {
            self.job = None;
            return None;
        };
        let Some(serial) = self.serial.checked_add(1) else {
            self.job = None;
            return None;
        };
        self.serial = serial;
        job.failure = Some(Failure {
            serial,
            page,
            deadline: now + delay,
        });
        Some((serial, delay, job.target.label()))
    }
    fn matches(&self, serial: i32, scope: Scope, page: i32) -> bool {
        self.job.as_ref().is_some_and(|job| {
            job.scope == scope
                && job
                    .failure
                    .as_ref()
                    .is_some_and(|failure| failure.serial == serial && failure.page == page)
        })
    }
    fn video_failure_serial(
        &self,
        id: &VideoId,
        quality: QualityCeiling,
        scope: Scope,
        page: i32,
    ) -> Option<i32> {
        let job = self.job.as_ref()?;
        let Target::Video(failed_id, failed_quality) = &job.target else {
            return None;
        };
        let serial = job.failure.as_ref()?.serial;
        (failed_id == id && *failed_quality == quality && self.matches(serial, scope, page))
            .then_some(serial)
    }
    fn ready(&self, serial: i32, scope: Scope, page: i32, now: Instant) -> bool {
        self.matches(serial, scope, page)
            && self
                .job
                .as_ref()
                .unwrap()
                .failure
                .as_ref()
                .unwrap()
                .deadline
                <= now
    }
    fn take(&mut self, serial: i32, scope: Scope, page: i32, now: Instant) -> Option<(Target, u8)> {
        if !self.ready(serial, scope, page, now) {
            return None;
        }
        let job = self.job.take().unwrap();
        Some((job.target, job.attempt.saturating_add(1).min(5)))
    }
}
#[derive(Default)]
pub struct State {
    ledger: RefCell<Ledger>,
    deadline: Timer,
}
fn scope(state: &UiState) -> Scope {
    Scope {
        generation: state.worker.borrow().generation(),
        session: state.account_ui.session_generation(),
        selection: crate::account_playback::generation(state),
    }
}
/// The transport's explicit restart shares an existing extraction failure's
/// deadline and attempt count. Callers release this ledger borrow before
/// invoking the normal retry callback, whose submission retires this job.
pub fn video_failure_serial(
    app: &App,
    state: &UiState,
    id: &VideoId,
    quality: QualityCeiling,
) -> Option<i32> {
    state.guest_recovery.ledger.borrow().video_failure_serial(
        id,
        quality,
        scope(state),
        app.get_page(),
    )
}
pub fn invalidate(app: &App, state: &UiState) {
    state.guest_recovery.deadline.stop();
    state.guest_recovery.ledger.borrow_mut().job = None;
    app.set_recovery_available(false);
    app.set_recovery_ready(false);
    app.set_recovery_serial(0);
}
pub fn failed(app: &App, state: &Rc<UiState>, generation: u64, error: ProviderError) {
    // Worker::take already rejects stale publications; retain that check here
    // too so another caller cannot retire a newer request's recovery action.
    if state.worker.borrow().generation() != generation {
        return;
    }
    let admission = state.guest_recovery.ledger.borrow_mut().fail(
        scope(state),
        app.get_page(),
        error,
        Instant::now(),
    );
    let Some((serial, wait, label)) = admission else {
        invalidate(app, state);
        return;
    };
    app.set_recovery_serial(serial);
    app.set_recovery_label(label.into());
    app.set_recovery_available(true);
    app.set_recovery_ready(false);
    app.set_status(
        format!(
            "{error} {label} in guest mode is available after a {}-second wait.",
            wait.as_secs()
        )
        .into(),
    );
    let weak = app.as_weak();
    let state_weak = Rc::downgrade(state);
    state
        .guest_recovery
        .deadline
        .start(TimerMode::SingleShot, wait, move || {
            let (Some(app), Some(state)) = (weak.upgrade(), state_weak.upgrade()) else {
                return;
            };
            let valid = state.guest_recovery.ledger.borrow().ready(
                serial,
                scope(&state),
                app.get_page(),
                Instant::now(),
            );
            if valid {
                app.set_recovery_ready(true);
            } else {
                invalidate(&app, &state);
            }
        });
}
pub fn bind(app: &App, state: &Rc<UiState>) {
    let weak = app.as_weak();
    let state_weak = Rc::downgrade(state);
    state.worker.borrow_mut().on_generation_changed(move |_| {
        let (Some(app), Some(state)) = (weak.upgrade(), state_weak.upgrade()) else {
            return;
        };
        invalidate(&app, &state);
    });
    let state_weak = Rc::downgrade(state);
    state
        .worker
        .borrow_mut()
        .on_submitted(move |generation, request| {
            let Some(state) = state_weak.upgrade() else {
                return;
            };
            // Worker is mutably borrowed by submit: use its supplied generation,
            // and never reborrow it from this synchronous observer.
            state.guest_recovery.ledger.borrow_mut().begin(
                Target::from_request(request),
                Scope {
                    generation,
                    session: state.account_ui.session_generation(),
                    selection: crate::account_playback::generation(&state),
                },
            );
        });
    let weak = app.as_weak();
    let state_weak = Rc::downgrade(state);
    app.on_retry_request(move |serial| {
        let (Some(app), Some(state)) = (weak.upgrade(), state_weak.upgrade()) else {
            return false;
        };
        if state.caption_cache.active()
            || state.library_fixture.is_some()
            || state.native_child.enabled
            || app.get_busy()
            || app.get_account_busy()
            || !state.playback_preferences.ready()
        {
            return false;
        }
        let current = scope(&state);
        if !state
            .guest_recovery
            .ledger
            .borrow()
            .matches(serial, current, app.get_page())
        {
            // A stale message dialog must not retire a newer recovery action.
            return false;
        }
        let Some((target, attempt)) = state.guest_recovery.ledger.borrow_mut().take(
            serial,
            current,
            app.get_page(),
            Instant::now(),
        ) else {
            return false;
        };
        let video = match &target {
            Target::Video(id, quality) => Some((id.clone(), *quality)),
            Target::Catalog(..) => None,
        };
        state.worker.borrow_mut().submit(target.request());
        if let Some(job) = state.guest_recovery.ledger.borrow_mut().job.as_mut() {
            job.attempt = attempt;
        }
        if let Some((id, quality)) = video {
            crate::guest_playback::retry_submitted(&state, &id, quality);
            state.focus_intent.arm(
                crate::focus_intent::Scope::Guest(state.worker.borrow().generation()),
                app.get_search_active(),
            );
        }
        app.set_busy(true);
        app.set_status("Retrying the same request in guest mode…".into());
        true
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serein_youtube::{
        ResolutionPolicy,
        catalog::{ChannelTab, SearchKind},
    };
    fn scope() -> Scope {
        Scope {
            generation: 4,
            session: 9,
            selection: 3,
        }
    }
    fn video() -> Target {
        Target::Video(
            VideoId::new("aqz-KE-bpKQ").unwrap(),
            QualityCeiling::default(),
        )
    }
    #[test]
    fn every_provider_error_has_an_explicit_recovery_classification() {
        for error in [
            ProviderError::Offline,
            ProviderError::Timeout,
            ProviderError::ExtractorFailed,
        ] {
            assert_eq!(delay(error, 0), Some(Duration::from_secs(2)));
            assert_eq!(delay(error, u8::MAX), Some(Duration::from_secs(60)));
        }
        assert_eq!(
            delay(ProviderError::RateLimited, 0),
            Some(Duration::from_secs(60))
        );
        for error in [
            ProviderError::InvalidInput,
            ProviderError::Busy,
            ProviderError::Cancelled,
            ProviderError::HelperUnavailable,
            ProviderError::UnsupportedPlatform,
            ProviderError::OutputTooLarge,
            ProviderError::MalformedOutput,
            ProviderError::Unavailable,
            ProviderError::AuthenticationRequired,
            ProviderError::ProofRequired,
            ProviderError::UnsupportedFormat,
            ProviderError::UnsafeMedia,
        ] {
            assert_eq!(delay(error, 0), None);
        }
    }
    #[test]
    fn retries_retain_typed_search_channel_playlist_and_quality_without_parsing_ui() {
        let requests = [
            CatalogRequest::Search {
                query: "actual typed request text".into(),
                kind: SearchKind::Playlists,
            },
            CatalogRequest::Channel {
                id: serein_core::ChannelId::new("UCabcdefghijklmnopqrstuv").unwrap(),
                tab: ChannelTab::Streams,
            },
            CatalogRequest::Playlist {
                id: serein_core::PlaylistId::new("PLsynthetic").unwrap(),
            },
        ];
        for request in requests {
            let saved = Target::from_request(&Request::Catalog(request.clone(), None)).unwrap();
            assert!(
                matches!(saved.request(), Request::Catalog(recovered, None) if recovered == request)
            );
        }
        let quality = QualityCeiling::from_index(2).unwrap();
        let id = VideoId::new("aqz-KE-bpKQ").unwrap();
        let saved = Target::from_request(&Request::Resolve(id.clone(), quality)).unwrap();
        assert!(
            matches!(saved.request(), Request::Resolve(recovered, retained) if recovered == id && retained == quality)
        );
        assert!(Target::from_request(&Request::Comments(id.clone(), None)).is_none());
        assert!(
            Target::from_request(&Request::ResolveQuality(
                id,
                ResolutionPolicy {
                    max_height: 720,
                    prefer_h264: true
                }
            ))
            .is_none()
        );
    }
    #[test]
    fn only_an_explicit_current_serial_after_backoff_consumes_request_once() {
        let now = Instant::now();
        let mut ledger = Ledger::default();
        ledger.begin(Some(video()), scope());
        let (serial, wait, _) = ledger
            .fail(scope(), 0, ProviderError::Offline, now)
            .unwrap();
        assert!(ledger.take(serial, scope(), 0, now).is_none());
        assert!(ledger.take(serial + 1, scope(), 0, now + wait).is_none());
        let (_, attempt) = ledger.take(serial, scope(), 0, now + wait).unwrap();
        assert_eq!(attempt, 1);
        assert!(ledger.take(serial, scope(), 0, now + wait).is_none());
    }
    #[test]
    fn cancelled_navigated_signed_out_or_new_account_selection_cannot_retry() {
        let now = Instant::now();
        let mut ledger = Ledger::default();
        ledger.begin(Some(video()), scope());
        let (serial, wait, _) = ledger
            .fail(scope(), 0, ProviderError::Timeout, now)
            .unwrap();
        for changed in [
            Scope {
                generation: 5,
                ..scope()
            },
            Scope {
                session: 10,
                ..scope()
            },
            Scope {
                selection: 4,
                ..scope()
            },
        ] {
            assert!(ledger.take(serial, changed, 0, now + wait).is_none());
        }
        assert!(ledger.take(serial, scope(), 1, now + wait).is_none());
        ledger.begin(None, scope());
        assert!(ledger.take(serial, scope(), 0, now + wait).is_none());
    }
    #[test]
    fn stale_failure_and_old_message_cannot_capture_or_consume_new_request() {
        let now = Instant::now();
        let mut ledger = Ledger::default();
        ledger.begin(Some(video()), scope());
        let (old, _, _) = ledger
            .fail(scope(), 0, ProviderError::Offline, now)
            .unwrap();
        let new_scope = Scope {
            generation: 5,
            ..scope()
        };
        ledger.begin(Some(video()), new_scope);
        let (new, wait, _) = ledger
            .fail(new_scope, 0, ProviderError::Offline, now)
            .unwrap();
        assert!(new > old);
        assert!(ledger.take(old, new_scope, 0, now + wait).is_none());
        assert!(ledger.take(new, new_scope, 0, now + wait).is_some());
        ledger.begin(Some(video()), new_scope);
        assert!(
            ledger
                .fail(scope(), 0, ProviderError::Timeout, now)
                .is_none()
        );
        assert_eq!(ledger.job.as_ref().unwrap().scope, new_scope);
        assert!(
            ledger
                .fail(new_scope, 0, ProviderError::Offline, now)
                .is_some()
        );
    }
    #[test]
    fn serial_exhaustion_disables_recovery_without_reusing_old_dialog_identity() {
        let mut ledger = Ledger {
            serial: i32::MAX,
            ..Default::default()
        };
        ledger.begin(Some(video()), scope());
        assert!(
            ledger
                .fail(scope(), 0, ProviderError::Offline, Instant::now())
                .is_none()
        );
        assert!(ledger.job.is_none());
    }
    #[test]
    fn playback_restart_matches_exact_failed_video_without_bypassing_its_deadline() {
        let mut ledger = Ledger::default();
        let now = Instant::now();
        let id = VideoId::new("aqz-KE-bpKQ").unwrap();
        let quality = QualityCeiling::default();
        ledger.begin(Some(video()), scope());
        assert_eq!(ledger.video_failure_serial(&id, quality, scope(), 2), None);
        let (serial, wait, _) = ledger
            .fail(scope(), 2, ProviderError::Offline, now)
            .unwrap();
        assert_eq!(
            ledger.video_failure_serial(&id, quality, scope(), 2),
            Some(serial)
        );
        assert!(ledger.take(serial, scope(), 2, now).is_none());
        assert_eq!(ledger.job.as_ref().unwrap().attempt, 0);
        assert_eq!(ledger.video_failure_serial(&id, quality, scope(), 0), None);
        assert_eq!(
            ledger.video_failure_serial(&VideoId::new("aaaaaaaaaaa").unwrap(), quality, scope(), 2),
            None
        );
        assert_eq!(
            ledger.video_failure_serial(&id, QualityCeiling::from_height(720).unwrap(), scope(), 2),
            None
        );
        for changed in [
            Scope {
                generation: scope().generation + 1,
                ..scope()
            },
            Scope {
                session: scope().session + 1,
                ..scope()
            },
            Scope {
                selection: scope().selection + 1,
                ..scope()
            },
        ] {
            assert_eq!(ledger.video_failure_serial(&id, quality, changed, 2), None);
        }
        let (_, attempt) = ledger.take(serial, scope(), 2, now + wait).unwrap();
        assert_eq!(attempt, 1);
        assert_eq!(ledger.video_failure_serial(&id, quality, scope(), 2), None);
    }
    #[test]
    fn manual_retries_have_bounded_state_and_capped_delays_without_automatic_submission() {
        let mut ledger = Ledger::default();
        let mut now = Instant::now();
        ledger.begin(Some(video()), scope());
        for round in 0..200 {
            let (serial, wait, _) = ledger
                .fail(scope(), 0, ProviderError::ExtractorFailed, now)
                .unwrap();
            assert!(wait <= Duration::from_secs(60));
            assert_eq!(
                wait,
                delay(ProviderError::ExtractorFailed, round.min(5)).unwrap()
            );
            assert!(ledger.job.is_some()); // elapsed time alone never consumes/submits
            now += wait;
            let (target, attempt) = ledger.take(serial, scope(), 0, now).unwrap();
            assert!(attempt <= 5);
            ledger.begin(Some(target), scope());
            ledger.job.as_mut().unwrap().attempt = attempt;
        }
    }
}
