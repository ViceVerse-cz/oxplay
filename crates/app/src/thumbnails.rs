// SPDX-License-Identifier: GPL-3.0-or-later
//! Anonymous, viewport-scoped images. No disk cache, cookies, proxy inheritance,
//! redirects, or UI-thread decoding. At most four requests and eight ready images.
use std::{
    io::Cursor,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
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
}
pub struct Worker {
    command: Option<watch::Sender<Option<Batch>>>,
    results: mpsc::Receiver<Ready>,
    thread: Option<thread::JoinHandle<()>>,
    generation: u64,
    counters: Arc<Counters>,
}
impl Worker {
    pub fn new(wake: impl Fn() + Send + Sync + 'static, fixture_diagnostics: bool) -> Self {
        let (command, mut commands) = watch::channel::<Option<Batch>>(None);
        let (results_tx, results) = mpsc::channel(8);
        let wake = Arc::new(wake);
        let counters = Arc::new(Counters::default());
        let metrics = counters.clone();
        let thread = thread::spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            runtime.block_on(async move {
                let Ok(client) = reqwest::Client::builder()
                    .https_only(true).no_proxy().redirect(reqwest::redirect::Policy::none())
                    .connect_timeout(Duration::from_secs(5)).timeout(Duration::from_secs(12))
                    .pool_max_idle_per_host(4).build() else { return };
                let mut jobs = tokio::task::JoinSet::new();
                let mut pending: std::collections::VecDeque<(u64, Request)> = std::collections::VecDeque::new();
                let mut completion_wake = CompletionWake::default();
                loop {
                    while jobs.len() < 4 {
                        let Some((generation, request)) = pending.pop_front() else { break };
                        let client = client.clone();
                        let results = results_tx.clone();
                        let wake = wake.clone();
                        let metrics = metrics.clone();
                        // Count scheduled jobs before removing their pending
                        // count: settled observers must not see a false gap.
                        let active = ActiveJob::new(metrics.clone());
                        metrics.pending.store(pending.len() as u64, Ordering::SeqCst);
                        jobs.spawn(async move {
                            let mut active = active;
                            let pixels = match request.source {
                                Source::Remote(url) => {
                                    metrics.remote_started.fetch_add(1, Ordering::SeqCst);
                                    fetch(&client, &url).await
                                },
                                Source::Fixture { source, index } => source.read(index).ok().and_then(|bytes| decode(&bytes)),
                            };
                            if pixels.is_some() { metrics.decoded.fetch_add(1, Ordering::SeqCst); }
                            else { metrics.failed.fetch_add(1, Ordering::SeqCst); }
                            if let Ok(permit) = results.reserve().await {
                                metrics.ready.fetch_add(1, Ordering::SeqCst);
                                // Channel slots include this reserved permit;
                                // do not confuse delayed consumer accounting
                                // with a queue larger than its actual bound.
                                metrics.ready_peak.fetch_max((8 - results.capacity()) as u64, Ordering::SeqCst);
                                permit.send(Ready { generation, row: request.row, pixels });
                                wake();
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
                        wake();
                    }
                    tokio::select! {
                        changed = commands.changed() => {
                            if changed.is_err() { break; }
                            jobs.abort_all();
                            while jobs.join_next().await.is_some() {}
                            pending.clear();
                            if let Some(batch) = commands.borrow_and_update().clone() {
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
        }
    }
    pub fn replace(&mut self, requests: Vec<Request>) -> u64 {
        self.generation += 1;
        while self.results.try_recv().is_ok() {
            self.counters.ready.fetch_sub(1, Ordering::SeqCst);
            self.counters.stale.fetch_add(1, Ordering::SeqCst);
        }
        if let Some(command) = &self.command {
            command.send_replace(Some(Batch {
                generation: self.generation,
                requests,
            }));
        }
        self.generation
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
async fn fetch(client: &reqwest::Client, url: &str) -> Option<image::RgbaImage> {
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
        let mut worker = Worker {
            command: None,
            results,
            thread: None,
            generation: 4,
            counters: counters.clone(),
        };
        let enqueue = |generation| {
            counters.ready.fetch_add(1, Ordering::SeqCst);
            send.try_send(Ready {
                generation,
                row: 0,
                pixels: Some(image::RgbaImage::new(2, 2)),
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
}
