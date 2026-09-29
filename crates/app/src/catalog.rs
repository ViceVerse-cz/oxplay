//! A single owned worker and a latest-request slot. Replaced jobs are cancelled.
use serein_core::{CancellationToken, OperationContext, ProviderError, ResolvedPlayback, VideoId};
use serein_youtube::{
    ResolutionPolicy,
    catalog::{CatalogCursor, CatalogPage, CatalogRequest},
    comments::{CommentCursor, CommentPage},
};
use std::sync::{Arc, Condvar, Mutex};

pub enum Request {
    Catalog(CatalogRequest, Option<CatalogCursor>),
    Resolve(VideoId, serein_core::QualityCeiling),
    Comments(VideoId, Option<CommentCursor>),
    Caption(
        VideoId,
        usize,
        Box<serein_core::SubtitleTrack>,
        crate::caption_files::Client,
    ),
    ResolveQuality(VideoId, ResolutionPolicy),
}
pub enum Response {
    Catalog(Box<CatalogPage>),
    Resolved(Box<ResolvedPlayback>, serein_core::QualityCeiling),
    Comments(VideoId, Result<CommentPage, ProviderError>),
    Caption(
        VideoId,
        usize,
        Result<Arc<crate::caption_files::Lease>, CaptionError>,
    ),
    QualityResolved(Box<ResolvedPlayback>, u16),
}
pub enum CaptionError {
    Provider(ProviderError),
    PrivateFile(&'static str),
}
impl std::fmt::Display for CaptionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Provider(error) => write!(f, "{error}"),
            Self::PrivateFile(message) => f.write_str(message),
        }
    }
}
type Submitted = Box<dyn Fn(u64, &Request)>;
type Wake = Arc<Mutex<Box<dyn Fn() + Send>>>;
type ResultSlot = Option<(u64, Result<Response, ProviderError>)>;
struct State {
    next: Option<(OperationContext, Request)>,
    active: Option<CancellationToken>,
    stop: bool,
}
pub struct Worker {
    shared: Arc<(Mutex<State>, Condvar)>,
    result: Arc<Mutex<ResultSlot>>,
    thread: Option<std::thread::JoinHandle<()>>,
    generation: u64,
    blocked: bool,
    wake: Wake,
    generation_changed: Vec<Box<dyn Fn(u64)>>,
    submitted: Vec<Submitted>,
}
impl Worker {
    #[cfg(test)]
    pub fn new(
        helper: std::path::PathBuf,
        deno: std::path::PathBuf,
        wake: impl Fn() + Send + 'static,
    ) -> Self {
        Self::with_resolver(crate::resolver::SharedResolver::new(helper, deno), wake)
    }
    pub fn with_resolver(
        resolver: crate::resolver::SharedResolver,
        wake: impl Fn() + Send + 'static,
    ) -> Self {
        let wake: Wake = Arc::new(Mutex::new(Box::new(wake)));
        let worker_wake = wake.clone();
        let shared = Arc::new((
            Mutex::new(State {
                next: None,
                active: None,
                stop: false,
            }),
            Condvar::new(),
        ));
        let result = Arc::new(Mutex::new(None));
        let work = shared.clone();
        let out = result.clone();
        let thread = std::thread::spawn(move || {
            loop {
                let (op, request) = {
                    let (lock, cv) = &*work;
                    let mut state = lock.lock().unwrap();
                    while state.next.is_none() && !state.stop {
                        state = cv.wait(state).unwrap();
                    }
                    if state.stop {
                        break;
                    }
                    let job = state.next.take().unwrap();
                    state.active = Some(job.0.cancel.clone());
                    job
                };
                let provider = resolver.get_on_worker();
                let response = match &provider {
                    Ok(provider) => match request {
                        Request::Catalog(request, cursor) => provider
                            .catalog(&request, cursor.as_ref(), &op)
                            .map(|page| Response::Catalog(Box::new(page))),
                        Request::Comments(id, cursor) => Ok(Response::Comments(
                            id.clone(),
                            provider.comments(&id, cursor.as_ref(), &op),
                        )),
                        Request::Caption(id, index, track, files) => {
                            let result = provider
                                .caption(&track, &id, &op)
                                .map_err(CaptionError::Provider)
                                .and_then(|data| {
                                    if op.cancel.is_cancelled() {
                                        return Err(CaptionError::Provider(
                                            ProviderError::Cancelled,
                                        ));
                                    }
                                    files
                                        .create(data.as_bytes())
                                        .map_err(CaptionError::PrivateFile)
                                });
                            Ok(Response::Caption(id, index, result))
                        }
                        Request::Resolve(id, quality) => provider
                            .resolve_with_policy(
                                &id,
                                ResolutionPolicy {
                                    max_height: quality.height(),
                                    prefer_h264: true,
                                },
                                &op,
                            )
                            .map(|r| Response::Resolved(Box::new(r), quality)),
                        Request::ResolveQuality(id, policy) => provider
                            .resolve_with_policy(&id, policy, &op)
                            .map(|r| Response::QualityResolved(Box::new(r), policy.max_height)),
                    },
                    Err(e) => match request {
                        Request::Comments(id, _) => Ok(Response::Comments(id, Err(*e))),
                        Request::Caption(id, index, _, _) => Ok(Response::Caption(
                            id,
                            index,
                            Err(CaptionError::Provider(*e)),
                        )),
                        _ => Err(*e),
                    },
                };
                if !op.cancel.is_cancelled() {
                    *out.lock().unwrap() = Some((op.request_id, response));
                    worker_wake.lock().unwrap()();
                }
            }
        });
        Self {
            shared,
            result,
            thread: Some(thread),
            generation: 0,
            blocked: false,
            wake,
            generation_changed: Vec::new(),
            submitted: Vec::new(),
        }
    }
    /// Supersede all work even when the replacement input fails validation.
    pub fn cancel(&mut self) {
        self.generation += 1;
        let (lock, _) = &*self.shared;
        let mut state = lock.lock().unwrap();
        if let Some(cancel) = state.active.take() {
            cancel.cancel();
        }
        if let Some((op, _)) = state.next.take() {
            op.cancel.cancel();
        }
        *self.result.lock().unwrap() = None;
        for changed in &self.generation_changed {
            changed(self.generation);
        }
    }
    pub fn on_generation_changed(&mut self, changed: impl Fn(u64) + 'static) {
        self.generation_changed.push(Box::new(changed));
    }
    pub fn on_submitted(&mut self, submitted: impl Fn(u64, &Request) + 'static) {
        self.submitted.push(Box::new(submitted));
    }
    pub fn set_blocked(&mut self, blocked: bool) {
        self.blocked = blocked;
        if blocked {
            self.cancel();
        }
    }
    pub fn submit(&mut self, request: Request) {
        self.cancel();
        if self.blocked {
            *self.result.lock().unwrap() = Some((self.generation, Err(ProviderError::Busy)));
            self.wake.lock().unwrap()();
            return;
        }
        for submitted in &self.submitted {
            submitted(self.generation, &request);
        }
        let (lock, cv) = &*self.shared;
        let mut state = lock.lock().unwrap();
        state.next = Some((
            OperationContext {
                request_id: self.generation,
                session_generation: 0,
                cancel: CancellationToken::default(),
            },
            request,
        ));
        cv.notify_one();
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    /// A transient comments panel must not cancel a newer playback/search job.
    pub fn cancel_generation(&mut self, generation: u64) -> bool {
        if self.generation != generation {
            return false;
        }
        self.cancel();
        true
    }
    pub fn take(&self) -> Option<Result<Response, ProviderError>> {
        self.result
            .lock()
            .unwrap()
            .take()
            .and_then(|(id, result)| (id == self.generation).then_some(result))
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let (lock, cv) = &*self.shared;
        {
            let mut state = lock.lock().unwrap();
            state.stop = true;
            if let Some(cancel) = state.active.take() {
                cancel.cancel();
            }
            if let Some((op, _)) = state.next.take() {
                op.cancel.cancel();
            }
        }
        cv.notify_one();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_configuration_errors_are_delivered_by_worker_without_running_a_helper() {
        let (notify, receive) = std::sync::mpsc::channel();
        let mut worker = Worker::new(
            "relative-helper".into(),
            "relative-deno".into(),
            move || {
                let _ = notify.send(());
            },
        );
        worker.submit(Request::Resolve(
            VideoId::new("aqz-KE-bpKQ").unwrap(),
            Default::default(),
        ));
        receive
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        assert!(matches!(
            worker.take(),
            Some(Err(ProviderError::InvalidInput))
        ));
    }

    // Construct an idle worker directly to test mailbox interleavings without
    // starting subprocesses, network operations, or a native event loop.
    fn idle_worker() -> Worker {
        Worker {
            shared: Arc::new((
                Mutex::new(State {
                    next: None,
                    active: None,
                    stop: false,
                }),
                Condvar::new(),
            )),
            result: Arc::new(Mutex::new(None)),
            thread: None,
            generation: 7,
            blocked: false,
            wake: Arc::new(Mutex::new(Box::new(|| {}))),
            generation_changed: Vec::new(),
            submitted: Vec::new(),
        }
    }

    #[test]
    fn cancel_invalidates_active_pending_and_published_results() {
        let mut worker = idle_worker();
        let active = CancellationToken::default();
        let pending = CancellationToken::default();
        {
            let mut state = worker.shared.0.lock().unwrap();
            state.active = Some(active.clone());
            state.next = Some((
                OperationContext {
                    request_id: 7,
                    session_generation: 0,
                    cancel: pending.clone(),
                },
                Request::Catalog(
                    CatalogRequest::Search {
                        query: "pending".into(),
                        kind: serein_youtube::catalog::SearchKind::All,
                    },
                    None,
                ),
            ));
        }
        *worker.result.lock().unwrap() = Some((7, Err(ProviderError::Unavailable)));
        worker.cancel();
        assert!(active.is_cancelled());
        assert!(pending.is_cancelled());
        assert_eq!(worker.generation, 8);
        assert!(worker.shared.0.lock().unwrap().next.is_none());
        assert!(worker.shared.0.lock().unwrap().active.is_none());
        assert!(worker.take().is_none());

        // Simulate the old worker publishing after its last cancellation check.
        *worker.result.lock().unwrap() = Some((7, Err(ProviderError::Unavailable)));
        assert!(worker.take().is_none());
    }
    #[test]
    fn supersession_notifies_all_independent_feature_observers() {
        let mut worker = idle_worker();
        let notifications = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        for feature in ["comments", "captions"] {
            let notifications = notifications.clone();
            worker.on_generation_changed(move |generation| {
                notifications.borrow_mut().push((feature, generation));
            });
        }
        worker.cancel();
        assert_eq!(
            &*notifications.borrow(),
            &[("comments", 8), ("captions", 8)]
        );
    }

    #[test]
    fn admitted_submission_observer_runs_after_retirement_and_never_for_blocked_work() {
        let mut worker = idle_worker();
        let trace = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let retired = trace.clone();
        worker.on_generation_changed(move |generation| {
            retired.borrow_mut().push((generation, "retired"))
        });
        let admitted = trace.clone();
        worker.on_submitted(move |generation, request| {
            assert!(matches!(request, Request::Resolve(id, quality)
                if id.as_str() == "aqz-KE-bpKQ" && *quality == serein_core::QualityCeiling::default()));
            assert_eq!(admitted.borrow().last(), Some(&(generation, "retired")));
            admitted.borrow_mut().push((generation, "admitted"));
        });
        worker.submit(Request::Resolve(
            VideoId::new("aqz-KE-bpKQ").unwrap(),
            Default::default(),
        ));
        assert_eq!(&*trace.borrow(), &[(8, "retired"), (8, "admitted")]);
        worker.set_blocked(true);
        worker.submit(Request::Comments(
            VideoId::new("aqz-KE-bpKQ").unwrap(),
            None,
        ));
        assert_eq!(
            &*trace.borrow(),
            &[
                (8, "retired"),
                (8, "admitted"),
                (9, "retired"),
                (10, "retired")
            ]
        );
        assert!(worker.shared.0.lock().unwrap().next.is_none());
    }

    #[test]
    fn stale_comment_cancel_cannot_stop_new_video_selection() {
        let mut worker = idle_worker();
        let video = VideoId::new("abcdefghijk").unwrap();
        worker.submit(Request::Comments(video.clone(), None));
        let comments_generation = worker.generation();
        worker.submit(Request::Resolve(video, Default::default()));
        assert!(!worker.cancel_generation(comments_generation));
        assert!(worker.shared.0.lock().unwrap().next.is_some());
        assert!(worker.cancel_generation(worker.generation()));
        assert!(worker.shared.0.lock().unwrap().next.is_none());
    }
    #[test]
    fn clearing_blocks_new_search_and_stale_results_until_admission_resumes() {
        let mut worker = idle_worker();
        let request = || {
            Request::Catalog(
                CatalogRequest::Search {
                    query: "synthetic blocked search".into(),
                    kind: serein_youtube::catalog::SearchKind::All,
                },
                None,
            )
        };
        worker.submit(request());
        let old = worker.generation();
        worker.set_blocked(true);
        worker.submit(request());
        assert!(worker.shared.0.lock().unwrap().next.is_none());
        assert!(matches!(worker.take(), Some(Err(ProviderError::Busy))));
        *worker.result.lock().unwrap() = Some((old, Err(ProviderError::Unavailable)));
        assert!(worker.take().is_none());
        worker.set_blocked(false);
        worker.submit(request());
        assert!(worker.shared.0.lock().unwrap().next.is_some());
    }

    #[test]
    fn submit_cancels_replaced_jobs_and_only_accepts_current_results() {
        let mut worker = idle_worker();
        worker.submit(Request::Catalog(
            CatalogRequest::Search {
                query: "first".into(),
                kind: serein_youtube::catalog::SearchKind::All,
            },
            None,
        ));
        let first = worker
            .shared
            .0
            .lock()
            .unwrap()
            .next
            .as_ref()
            .unwrap()
            .0
            .clone();
        worker.submit(Request::Catalog(
            CatalogRequest::Search {
                query: "second".into(),
                kind: serein_youtube::catalog::SearchKind::All,
            },
            None,
        ));
        assert!(first.cancel.is_cancelled());
        let latest_id = worker
            .shared
            .0
            .lock()
            .unwrap()
            .next
            .as_ref()
            .unwrap()
            .0
            .request_id;
        assert_eq!(latest_id, worker.generation);
        assert_ne!(latest_id, first.request_id);
        *worker.result.lock().unwrap() = Some((first.request_id, Err(ProviderError::Unavailable)));
        assert!(worker.take().is_none());
        *worker.result.lock().unwrap() = Some((latest_id, Err(ProviderError::RateLimited)));
        assert!(matches!(
            worker.take(),
            Some(Err(ProviderError::RateLimited))
        ));
        assert!(worker.take().is_none());
    }
}
