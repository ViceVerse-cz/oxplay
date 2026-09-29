// SPDX-License-Identifier: GPL-3.0-or-later
//! Anonymous, viewport-scoped images. Guest video artwork has a bounded cache.
//! No cookies, proxy inheritance,
//! redirects, or UI-thread decoding. At most four requests and eight ready images.
use serein_core::VideoId;
use serein_storage::artwork::{ArtworkCache, CacheLimit};
use std::{
    io::Cursor,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};
use tokio::sync::{mpsc, watch};

#[derive(Clone)]
pub struct Request {
    pub row: usize,
    pub source: Source,
}
#[derive(Clone)]
pub enum Source {
    Remote(String),
    /// Only a public guest video catalog row may admit this source.
    RemoteGuestVideo {
        id: VideoId,
        url: String,
    },
    /// Disk-only lookup: a miss must never cause a network request.
    CachedVideo(VideoId),
    /// Only an explicitly admitted offline developer fixture can create this.
    Fixture {
        source: Arc<crate::library_fixture::FixtureSource>,
        index: usize,
    },
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Statistics {
    pub admitted_generation: u64,
    pub pending: u64,
    pub inflight: u64,
    pub ready: u64,
    pub ready_peak: u64,
    pub started: u64,
    pub remote_started: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub cache_errors: u64,
    pub decoded: u64,
    pub failed: u64,
    pub cancelled: u64,
    pub stale: u64,
    /// Actual UI publication, recorded after constructing the Slint image.
    pub published: u64,
    pub published_bytes: u64,
}
#[derive(Default)]
struct Counters {
    admitted_generation: AtomicU64,
    pending: AtomicU64,
    inflight: AtomicU64,
    ready: AtomicU64,
    ready_peak: AtomicU64,
    started: AtomicU64,
    remote_started: AtomicU64,
    cache_hits: AtomicU64,
    cache_misses: AtomicU64,
    cache_errors: AtomicU64,
    decoded: AtomicU64,
    failed: AtomicU64,
    cancelled: AtomicU64,
    stale: AtomicU64,
    published: AtomicU64,
    published_bytes: AtomicU64,
}
impl Counters {
    fn snapshot(&self) -> Statistics {
        Statistics {
            admitted_generation: self.admitted_generation.load(Ordering::SeqCst),
            pending: self.pending.load(Ordering::SeqCst),
            inflight: self.inflight.load(Ordering::SeqCst),
            ready: self.ready.load(Ordering::SeqCst),
            ready_peak: self.ready_peak.load(Ordering::SeqCst),
            started: self.started.load(Ordering::SeqCst),
            remote_started: self.remote_started.load(Ordering::SeqCst),
            cache_hits: self.cache_hits.load(Ordering::SeqCst),
            cache_misses: self.cache_misses.load(Ordering::SeqCst),
            cache_errors: self.cache_errors.load(Ordering::SeqCst),
            decoded: self.decoded.load(Ordering::SeqCst),
            failed: self.failed.load(Ordering::SeqCst),
            cancelled: self.cancelled.load(Ordering::SeqCst),
            stale: self.stale.load(Ordering::SeqCst),
            published: self.published.load(Ordering::SeqCst),
            published_bytes: self.published_bytes.load(Ordering::SeqCst),
        }
    }
}
struct ActiveJob {
    counters: Arc<Counters>,
    completed: bool,
}
impl ActiveJob {
    fn new(counters: Arc<Counters>) -> Self {
        counters.inflight.fetch_add(1, Ordering::SeqCst);
        counters.started.fetch_add(1, Ordering::SeqCst);
        Self {
            counters,
            completed: false,
        }
    }
}
impl Drop for ActiveJob {
    fn drop(&mut self) {
        self.counters.inflight.fetch_sub(1, Ordering::SeqCst);
        if !self.completed {
            self.counters.cancelled.fetch_add(1, Ordering::SeqCst);
        }
    }
}
#[derive(Clone)]
struct Batch {
    generation: u64,
    requests: Vec<Request>,
}
#[derive(Default)]
struct CompletionWake {
    generation: u64,
}
impl CompletionWake {
    fn take(&mut self, generation: u64, pending: usize, jobs: usize) -> bool {
        if generation == 0 || generation <= self.generation || pending != 0 || jobs != 0 {
            return false;
        }
        self.generation = generation;
        true
    }
}
pub struct Ready {
    pub generation: u64,
    pub row: usize,
    pub pixels: Option<image::RgbaImage>,
    pub video_id: Option<VideoId>,
}
enum Control {
    Purge(u64),
    EndPurge(u64),
}
#[derive(Default)]
struct Acknowledgments {
    purge: Option<(u64, Result<(), &'static str>)>,
    limit: Option<(u16, Result<(), &'static str>)>,
}
type Cache = Arc<Mutex<Option<ArtworkCache>>>;

/// One queued UI handoff regardless of how many images complete together.
/// The UI acknowledges before draining, then schedules a continuation only
/// while ready work remains. This is not a periodic image/upload timer.
struct Wake {
    queued: AtomicBool,
    notify: Box<dyn Fn() + Send + Sync>,
}
impl Wake {
    fn new(notify: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            queued: AtomicBool::new(false),
            notify: Box::new(notify),
        }
    }
    fn send(&self) {
        if !self.queued.swap(true, Ordering::AcqRel) {
            (self.notify)();
        }
    }
    fn acknowledge(&self) {
        self.queued.store(false, Ordering::Release);
    }
}

pub struct Worker {
    command: Option<watch::Sender<Option<Batch>>>,
    results: mpsc::Receiver<Ready>,
    thread: Option<thread::JoinHandle<()>>,
    generation: u64,
    counters: Arc<Counters>,
    controls: mpsc::Sender<Control>,
    limits: watch::Sender<Option<CacheLimit>>,
    acknowledgments: Arc<Mutex<Acknowledgments>>,
    purge: Option<u64>,
    purge_serial: u64,
    wake: Arc<Wake>,
}
impl Worker {
    pub fn new(
        cache_path: PathBuf,
        initial_limit: Option<CacheLimit>,
        wake: impl Fn() + Send + Sync + 'static,
        fixture_diagnostics: bool,
    ) -> Self {
        let (command, mut commands) = watch::channel::<Option<Batch>>(None);
        let (results_tx, results) = mpsc::channel(8);
        let (controls, mut control_rx) = mpsc::channel(2);
        let (limits, mut limit_rx) = watch::channel(initial_limit);
        let acknowledgments = Arc::new(Mutex::new(Acknowledgments::default()));
        let acks = acknowledgments.clone();
        let wake = Arc::new(Wake::new(wake));
        let worker_wake = wake.clone();
        let counters = Arc::new(Counters::default());
        let metrics = counters.clone();
        let thread = thread::spawn(move || {
            let wake = worker_wake;
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            let cache = Arc::new(Mutex::new(
                initial_limit.and_then(|limit| ArtworkCache::open(&cache_path, limit).ok()),
            ));
            runtime.block_on(async move {
                let Ok(client) = reqwest::Client::builder()
                    .https_only(true).no_proxy().redirect(reqwest::redirect::Policy::none())
                    .connect_timeout(Duration::from_secs(5)).timeout(Duration::from_secs(12))
                    .pool_max_idle_per_host(4).build() else { return };
                let mut jobs = tokio::task::JoinSet::new();
                let mut pending: std::collections::VecDeque<(u64, Request)> = std::collections::VecDeque::new();
                let mut completion_wake = CompletionWake::default();
                let mut purging = None;
                let mut deferred_limit = None;
                loop {
                    while jobs.len() < 4 {
                        let Some((generation, request)) = pending.pop_front() else { break };
                        let client = client.clone();
                        let results = results_tx.clone();
                        let wake = wake.clone();
                        let metrics = metrics.clone();
                        // Count scheduled jobs before removing their pending
                        // count: settled observers must not see a false gap.
                        let cache = cache.clone();
                        let active = ActiveJob::new(metrics.clone());
                        metrics.pending.store(pending.len() as u64, Ordering::SeqCst);
                        jobs.spawn(async move {
                            let mut active = active;
                            let (pixels, video_id) = resolve_source(&client, request.source, &cache, &metrics).await;
                            if pixels.is_some() { metrics.decoded.fetch_add(1, Ordering::SeqCst); }
                            else { metrics.failed.fetch_add(1, Ordering::SeqCst); }
                            if let Ok(permit) = results.reserve().await {
                                metrics.ready.fetch_add(1, Ordering::SeqCst);
                                // Channel slots include this reserved permit;
                                // do not confuse delayed consumer accounting
                                // with a queue larger than its actual bound.
                                metrics.ready_peak.fetch_max((8 - results.capacity()) as u64, Ordering::SeqCst);
                                permit.send(Ready { generation, row: request.row, pixels, video_id });
                                wake.send();
                            }
                            active.completed = true;
                        });
                    }
                    // An image's wake can reach the UI before ActiveJob drops.
                    // Explicit fixture diagnostics need one final handoff after
                    // the scheduler has reaped every job, including an empty
                    // admitted batch. Ready images are drained by that UI wake.
                    // Normal browsing gains no extra completion notification.
                    if fixture_diagnostics && completion_wake.take(
                        metrics.admitted_generation.load(Ordering::SeqCst), pending.len(), jobs.len(),
                    ) {
                        wake.send();
                    }
                    tokio::select! {
                        biased;
                        control = control_rx.recv() => {
                            let Some(control) = control else { break };
                            match control {
                                Control::Purge(id) => {
                                    purging = Some(id);
                                    // UI blocks replace synchronously before the next event.
                                    // Drop only pre-barrier batches here, never post-End work.
                                    commands.borrow_and_update();
                                    jobs.abort_all();
                                    while jobs.join_next().await.is_some() {}
                                    pending.clear();
                                    metrics.pending.store(0, Ordering::SeqCst);
                                    // No job can retain a cache write across this barrier.
                                    let result = clear_cache(&cache, &cache_path);
                                    acks.lock().unwrap().purge = Some((id, result));
                                    wake.send();
                                }
                                Control::EndPurge(id) if purging == Some(id) => {
                                    purging = None;
                                    if let Some(limit) = deferred_limit.take() {
                                        let result = configure_cache(&cache, &cache_path, limit);
                                        acks.lock().unwrap().limit = Some((limit.mib(), result));
                                        wake.send();
                                    }
                                }
                                Control::EndPurge(_) => {}
                            }
                        }
                        changed = limit_rx.changed() => {
                            if changed.is_err() { break; }
                            let Some(limit) = *limit_rx.borrow_and_update() else { continue; };
                            if purging.is_some() {
                                deferred_limit = Some(limit);
                            } else {
                                let result = configure_cache(&cache, &cache_path, limit);
                                acks.lock().unwrap().limit = Some((limit.mib(), result));
                                wake.send();
                            }
                        }
                        changed = commands.changed() => {
                            if changed.is_err() { break; }
                            jobs.abort_all();
                            while jobs.join_next().await.is_some() {}
                            pending.clear();
                            let batch = commands.borrow_and_update().clone();
                            // Apply a coalesced preference before admitting any image jobs.
                            if limit_rx.has_changed().unwrap_or(false) {
                                let Some(limit) = *limit_rx.borrow_and_update() else { continue; };
                                if purging.is_some() {
                                    deferred_limit = Some(limit);
                                } else {
                                    let result = configure_cache(&cache, &cache_path, limit);
                                    acks.lock().unwrap().limit = Some((limit.mib(), result));
                                    wake.send();
                                }
                            }
                            if purging.is_none() && let Some(batch) = batch {
                                pending.extend(batch.requests.into_iter().take(40).map(|r| (batch.generation, r)));
                                metrics.pending.store(pending.len() as u64, Ordering::SeqCst);
                                metrics.admitted_generation.store(batch.generation, Ordering::SeqCst);
                            }
                            metrics.pending.store(pending.len() as u64, Ordering::SeqCst);
                        }
                        _ = jobs.join_next(), if !jobs.is_empty() => {}
                    }
                }
                jobs.abort_all();
                while jobs.join_next().await.is_some() {}
                metrics.pending.store(0, Ordering::SeqCst);
            });
        });
        Self {
            command: Some(command),
            results,
            thread: Some(thread),
            generation: 0,
            counters,
            controls,
            limits,
            acknowledgments,
            purge: None,
            purge_serial: 0,
            wake,
        }
    }
    pub fn begin_wake(&self) {
        self.wake.acknowledge();
    }
    /// Call after a bounded UI drain to continue on a fresh event-loop turn.
    pub fn continue_wake(&self) {
        if !self.results.is_empty() {
            self.wake.send();
        }
    }
    pub fn replace(&mut self, mut requests: Vec<Request>) -> u64 {
        requests.truncate(40);
        self.generation += 1;
        while self.results.try_recv().is_ok() {
            self.counters.ready.fetch_sub(1, Ordering::SeqCst);
            self.counters.stale.fetch_add(1, Ordering::SeqCst);
        }
        if self.purge.is_none()
            && let Some(command) = &self.command
        {
            command.send_replace(Some(Batch {
                generation: self.generation,
                requests,
            }));
        }
        self.generation
    }
    pub fn set_cache_limit(&mut self, mib: u16) -> Result<(), &'static str> {
        let limit = CacheLimit::from_mib(mib).ok_or("Invalid artwork cache limit.")?;
        self.limits
            .send(Some(limit))
            .map_err(|_| "Artwork worker is unavailable.")
    }
    pub fn take_cache_limit_result(&self) -> Option<(u16, Result<(), &'static str>)> {
        self.acknowledgments.lock().unwrap().limit.take()
    }
    pub fn begin_purge(&mut self) -> Result<u64, &'static str> {
        if self.purge.is_some() {
            return Err("Artwork cleanup is already in progress.");
        }
        let id = self
            .purge_serial
            .checked_add(1)
            .ok_or("Artwork cleanup is unavailable.")?;
        self.controls
            .try_send(Control::Purge(id))
            .map_err(|_| "Artwork worker is busy or unavailable.")?;
        self.purge_serial = id;
        self.purge = Some(id);
        self.replace(Vec::new()); // synchronous publication invalidation, no new batch
        Ok(id)
    }
    pub fn purge_result(&self, id: u64) -> Option<Result<(), &'static str>> {
        self.acknowledgments
            .lock()
            .unwrap()
            .purge
            .filter(|(actual, _)| *actual == id)
            .map(|(_, result)| result)
    }
    pub fn end_purge(&mut self, id: u64) -> Result<(), &'static str> {
        if self.purge != Some(id) {
            return Err("Artwork cleanup identity changed.");
        }
        if self.purge_result(id).is_none() {
            return Err("Artwork cleanup has not completed.");
        }
        self.controls
            .try_send(Control::EndPurge(id))
            .map_err(|_| "Artwork worker is busy or unavailable.")?;
        self.purge = None;
        Ok(())
    }
    pub fn take(&mut self) -> Option<Ready> {
        while let Ok(result) = self.results.try_recv() {
            self.counters.ready.fetch_sub(1, Ordering::SeqCst);
            if result.generation == self.generation {
                return Some(result);
            }
            self.counters.stale.fetch_add(1, Ordering::SeqCst);
        }
        None
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn statistics(&self) -> Statistics {
        self.counters.snapshot()
    }
    pub fn record_publication(&self, rgba_bytes: usize) {
        self.counters.published.fetch_add(1, Ordering::SeqCst);
        self.counters
            .published_bytes
            .fetch_add(rgba_bytes as u64, Ordering::SeqCst);
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.command.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn configure_cache(
    cache: &Cache,
    path: &std::path::Path,
    limit: CacheLimit,
) -> Result<(), &'static str> {
    let mut cache = cache.lock().unwrap();
    if let Some(cache) = cache.as_mut() {
        cache
            .set_limit(limit)
            .map_err(|_| "Artwork cache configuration failed.")
    } else {
        *cache =
            Some(ArtworkCache::open(path, limit).map_err(|_| "Artwork cache is unavailable.")?);
        Ok(())
    }
}
fn clear_cache(cache: &Cache, path: &std::path::Path) -> Result<(), &'static str> {
    let mut cache = cache.lock().unwrap();
    match cache.as_mut() {
        Some(cache) => cache.clear(),
        None => ArtworkCache::clear_if_absent(path),
    }
    .map_err(|_| "Artwork cache could not be cleared.")
}
fn cached(cache: &Cache, id: &VideoId, counters: &Counters) -> Option<image::RgbaImage> {
    let bytes = match cache.lock().unwrap().as_mut().map(|cache| cache.get(id)) {
        Some(Ok(Some(bytes))) => bytes,
        Some(Ok(None)) => {
            counters.cache_misses.fetch_add(1, Ordering::SeqCst);
            return None;
        }
        _ => {
            counters.cache_errors.fetch_add(1, Ordering::SeqCst);
            return None;
        }
    };
    let pixels = decode(&bytes);
    if pixels.is_some() {
        counters.cache_hits.fetch_add(1, Ordering::SeqCst);
    } else {
        counters.cache_errors.fetch_add(1, Ordering::SeqCst);
    }
    pixels
}
fn store(cache: &Cache, id: &VideoId, pixels: &image::RgbaImage, counters: &Counters) {
    // This is small normalized artwork, never decoded-video frame transport.
    let mut png = Cursor::new(Vec::new());
    let result = pixels
        .write_to(&mut png, image::ImageFormat::Png)
        .ok()
        .filter(|_| png.get_ref().len() <= 512 * 1024)
        .and_then(|_| cache.lock().unwrap().as_mut()?.put(id, png.get_ref()).ok());
    if result.is_none() {
        counters.cache_errors.fetch_add(1, Ordering::SeqCst);
    }
}
async fn resolve_source(
    client: &reqwest::Client,
    source: Source,
    cache: &Cache,
    counters: &Counters,
) -> (Option<image::RgbaImage>, Option<VideoId>) {
    match source {
        Source::CachedVideo(id) => (cached(cache, &id, counters), Some(id)),
        Source::RemoteGuestVideo { id, url } => {
            // Remote requests are explicitly admitted public browsing. A hit can
            // avoid that request, but only this source may populate the cache.
            if let Some(pixels) = cached(cache, &id, counters) {
                return (Some(pixels), Some(id));
            }
            counters.remote_started.fetch_add(1, Ordering::SeqCst);
            let pixels = fetch(client, &url).await;
            if let Some(pixels) = &pixels {
                store(cache, &id, pixels, counters);
            }
            (pixels, Some(id))
        }
        Source::Remote(url) => {
            counters.remote_started.fetch_add(1, Ordering::SeqCst);
            (fetch(client, &url).await, None)
        }
        Source::Fixture { source, index } => (
            source.read(index).ok().and_then(|bytes| decode(&bytes)),
            None,
        ),
    }
}
fn allowed(url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(url) else {
        return false;
    };
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.fragment().is_none()
        && matches!(
            url.host_str(),
            Some(
                "i.ytimg.com"
                    | "i1.ytimg.com"
                    | "i2.ytimg.com"
                    | "i3.ytimg.com"
                    | "i4.ytimg.com"
                    | "yt3.ggpht.com"
                    | "yt3.googleusercontent.com"
            )
        )
}
pub(crate) async fn fetch(client: &reqwest::Client, url: &str) -> Option<image::RgbaImage> {
    if !allowed(url) {
        return None;
    }
    let mut response = client.get(url).send().await.ok()?.error_for_status().ok()?;
    const MAX_BYTES: usize = 2 * 1024 * 1024;
    if response
        .content_length()
        .is_some_and(|n| n > MAX_BYTES as u64)
    {
        return None;
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if bytes.len().checked_add(chunk.len())? > MAX_BYTES {
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    // The dedicated worker owns decoding too. Small, bounded input avoids an
    // unbounded blocking-pool queue; cancellation is checked before publication.
    decode(&bytes)
}
fn decode(bytes: &[u8]) -> Option<image::RgbaImage> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(16 * 1024 * 1024);
    reader.limits(limits);
    Some(reader.decode().ok()?.thumbnail(320, 180).into_rgba8())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn image_completion_bursts_queue_one_handoff_until_acknowledged() {
        let notifications = Arc::new(AtomicU64::new(0));
        let observed = notifications.clone();
        let wake = Wake::new(move || {
            observed.fetch_add(1, Ordering::SeqCst);
        });
        for _ in 0..40 {
            wake.send();
        }
        assert_eq!(notifications.load(Ordering::SeqCst), 1);
        wake.acknowledge();
        // A completion racing with a bounded-drain continuation still queues
        // only one next callback. Idle state never schedules another wake.
        wake.send();
        wake.send();
        assert_eq!(notifications.load(Ordering::SeqCst), 2);
        wake.acknowledge();
        assert_eq!(notifications.load(Ordering::SeqCst), 2);
    }
    #[test]
    fn anonymous_origin_policy_is_exact() {
        assert!(allowed("https://i.ytimg.com/vi/test/hqdefault.jpg"));
        assert!(allowed("https://yt3.ggpht.com/avatar"));
        assert!(allowed("https://yt3.googleusercontent.com/avatar"));
        assert!(!allowed(
            "https://yt3.googleusercontent.com.evil.test/avatar"
        ));
        for url in [
            "http://i.ytimg.com/a",
            "https://i.ytimg.com.evil.test/a",
            "https://user@i.ytimg.com/a",
            "https://example.com/a",
            "https://i.ytimg.com:444/a",
            "https://i.ytimg.com/a#x",
        ] {
            assert!(!allowed(url));
        }
    }
    #[test]
    fn decoding_is_bounded_and_resized_before_handoff() {
        let source = image::RgbImage::new(1280, 720);
        let mut png = Cursor::new(Vec::new());
        source.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let result = decode(png.get_ref()).unwrap();
        assert_eq!(result.dimensions(), (320, 180));
        assert!(decode(b"not an image").is_none());
    }
    #[test]
    fn offscreen_and_replaced_page_completions_cannot_repopulate_new_rows() {
        let (send, results) = mpsc::channel(8);
        let counters = Arc::new(Counters::default());
        let (controls, _control_rx) = mpsc::channel(2);
        let (limits, _limit_rx) = watch::channel(Some(CacheLimit::Off));
        let mut worker = Worker {
            command: None,
            results,
            thread: None,
            generation: 4,
            counters: counters.clone(),
            controls,
            limits,
            acknowledgments: Arc::new(Mutex::new(Acknowledgments::default())),
            purge: None,
            purge_serial: 0,
            wake: Arc::new(Wake::new(|| {})),
        };
        let enqueue = |generation| {
            counters.ready.fetch_add(1, Ordering::SeqCst);
            send.try_send(Ready {
                generation,
                row: 0,
                pixels: Some(image::RgbaImage::new(2, 2)),
                video_id: None,
            })
            .unwrap();
        };
        enqueue(3);
        enqueue(4);
        assert_eq!(worker.take().unwrap().generation, 4);
        assert!(worker.take().is_none());
        assert_eq!(worker.statistics().stale, 1);
        assert_eq!(worker.statistics().ready, 0);
        enqueue(4);
        let next = worker.replace(Vec::new());
        assert_eq!(next, 5);
        // A blocking image decode may finish just after cancellation. Even an
        // in-range row index from the old page must never be handed to Slint.
        enqueue(4);
        assert!(worker.take().is_none());
        assert_eq!(worker.statistics().stale, 3);
        assert_eq!(worker.statistics().ready, 0);
        assert_eq!(worker.statistics().published, 0);
        enqueue(5);
        let ready = worker.take().unwrap();
        worker.record_publication(ready.pixels.unwrap().as_raw().len());
        assert_eq!(worker.statistics().published, 1);
        assert_eq!(worker.statistics().published_bytes, 16);
    }
    #[test]
    fn job_cancellation_and_completion_release_inflight_counts() {
        let counters = Arc::new(Counters::default());
        {
            let _cancelled = ActiveJob::new(counters.clone());
            let mut completed = ActiveJob::new(counters.clone());
            completed.completed = true;
            assert_eq!(counters.snapshot().inflight, 2);
        }
        let stats = counters.snapshot();
        assert_eq!(stats.inflight, 0);
        assert_eq!(stats.started, 2);
        assert_eq!(stats.cancelled, 1);
    }
    #[test]
    fn final_fixture_wake_waits_for_reaped_jobs_and_is_once_per_admitted_generation() {
        let mut wake = CompletionWake::default();
        assert!(!wake.take(0, 0, 0));
        assert!(!wake.take(1, 1, 0));
        assert!(!wake.take(1, 0, 1)); // Result wake can precede this final join.
        assert!(wake.take(1, 0, 0));
        assert!(!wake.take(1, 0, 0));
        assert!(!wake.take(2, 0, 1)); // Cancellation still owns the old task.
        assert!(wake.take(3, 0, 0)); // Coalesced replacement may skip generation2.
        assert!(!wake.take(2, 0, 0));
        assert!(wake.take(4, 0, 0)); // Empty batch still needs an admission wake.
    }
    #[test]
    fn cache_only_miss_keeps_identity_and_never_admits_http() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let id = VideoId::new("aqz-KE-bpKQ").unwrap();
        let metrics = Counters::default();
        let (pixels, identity) = runtime.block_on(resolve_source(
            &reqwest::Client::builder().no_proxy().build().unwrap(),
            Source::CachedVideo(id.clone()),
            &Arc::new(Mutex::new(None)),
            &metrics,
        ));
        assert!(pixels.is_none());
        assert!(identity == Some(id));
        assert_eq!(metrics.snapshot().remote_started, 0);
        assert_eq!(metrics.snapshot().cache_errors, 1);
    }

    #[cfg(unix)]
    #[test]
    fn real_cache_purge_blocks_publication_and_immediate_post_end_batch_survives() {
        use std::os::unix::fs::DirBuilderExt;
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        struct Directory(PathBuf);
        impl Drop for Directory {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let directory = Directory(std::env::temp_dir().join(format!(
            "serein-artwork-worker-test-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::SeqCst),
        )));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory.0)
            .unwrap();
        let path = directory.0.canonicalize().unwrap().join("cache");
        let id = VideoId::new("aqz-KE-bpKQ").unwrap();
        let mut png = Cursor::new(Vec::new());
        image::RgbaImage::new(16, 9)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        {
            let mut cache = ArtworkCache::open(&path, CacheLimit::Mib32).unwrap();
            cache.put(&id, png.get_ref()).unwrap();
        }
        let (wake, wakes) = std::sync::mpsc::channel();
        let mut worker = Worker::new(
            path.clone(),
            None,
            move || {
                let _ = wake.send(());
            },
            false,
        );
        let request = || {
            vec![Request {
                row: 3,
                source: Source::CachedVideo(id.clone()),
            }]
        };
        worker.replace(request());
        let wait = |worker: &Worker| {
            wakes.recv_timeout(Duration::from_secs(5)).unwrap();
            worker.begin_wake();
        };
        loop {
            if let Some(ready) = worker.take() {
                assert!(ready.pixels.is_none());
                break;
            }
            wait(&worker);
        }
        // Unconfigured startup must neither open nor purge the preexisting cache.
        worker.set_cache_limit(32).unwrap();
        loop {
            if let Some((limit, result)) = worker.take_cache_limit_result() {
                assert_eq!(limit, 32);
                result.unwrap();
                break;
            }
            wait(&worker);
        }
        worker.replace(request());
        let first = loop {
            if let Some(ready) = worker.take() {
                break ready;
            }
            wait(&worker);
        };
        assert_eq!(first.row, 3);
        assert!(first.video_id == Some(id.clone()));
        assert_eq!(first.pixels.unwrap().dimensions(), (320, 180));
        let purge = worker.begin_purge().unwrap();
        assert!(worker.begin_purge().is_err());
        worker.replace(request()); // suppressed while purge owns admission
        loop {
            if let Some(result) = worker.purge_result(purge) {
                result.unwrap();
                break;
            }
            wait(&worker);
        }
        assert!(worker.take().is_none());
        assert_eq!(worker.statistics().started, 2);
        assert_eq!(worker.statistics().inflight, 0);
        assert!(worker.end_purge(purge + 1).is_err());
        worker.end_purge(purge).unwrap();
        // Deliberately queue immediately: End must not discard this fresh batch.
        let generation = worker.replace(request());
        let second = loop {
            if let Some(ready) = worker.take() {
                break ready;
            }
            wait(&worker);
        };
        assert_eq!(second.generation, generation);
        assert!(second.video_id == Some(id.clone()));
        assert!(second.pixels.is_none());
        assert_eq!(worker.statistics().remote_started, 0);
        assert_eq!(worker.statistics().cache_hits, 1);
        assert_eq!(worker.statistics().cache_misses, 1);
        drop(worker);
        let mut cache = ArtworkCache::open(&path, CacheLimit::Mib32).unwrap();
        assert!(cache.get(&id).unwrap().is_none());
    }
}
