// SPDX-License-Identifier: GPL-3.0-or-later
//! Anonymous author portraits for the one visible page of comments (at most 20),
//! and, through a second instance, for the reply threads opened on that page.
//!
//! Reuses the bounded thumbnail fetch policy (`thumbnails::fetch`: exact Google
//! image hosts only, HTTPS, no proxy, cookies or redirects, 2 MiB input, bounded
//! decode). One job owns the whole page: at most four requests run at once, every
//! result is resized to at most 88x88 before it reaches Slint, and a new page,
//! video, disabled setting or clear supersedes the job and drops late results.
use crate::thumbnails::fetch_sized;
use std::{
    cell::{Cell, RefCell},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use tokio::sync::watch;

pub const MAX_AVATARS: usize = 20;
const MAX_EDGE: u32 = 88;
const CONCURRENT: usize = 4;

pub struct Ready {
    pub generation: u64,
    pub row: usize,
    pub pixels: image::RgbaImage,
}
#[derive(Clone)]
struct Job {
    generation: u64,
    urls: Vec<(usize, String)>,
    cancelled: watch::Receiver<bool>,
}

/// Keep only results from the current generation, preserving arrival order.
fn drain_current(results: &mut Vec<Ready>, generation: u64) -> Vec<Ready> {
    results
        .drain(..)
        .filter(|ready| ready.generation == generation)
        .collect()
}
pub struct Avatars {
    generation: Cell<u64>,
    max_avatars: usize,
    command: Option<watch::Sender<Option<Job>>>,
    cancellation: RefCell<Option<watch::Sender<bool>>>,
    results: Arc<Mutex<Vec<Ready>>>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Avatars {
    /// `wake` is invoked from the worker after each published result.
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self::with_limits(MAX_EDGE, MAX_AVATARS, wake)
    }
    /// Same pipeline with a different per-job count and decoded edge bound.
    pub fn with_limits(
        max_edge: u32,
        max_avatars: usize,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let (command, mut commands) = watch::channel::<Option<Job>>(None);
        let results = Arc::new(Mutex::new(Vec::<Ready>::new()));
        let out = results.clone();
        let thread = thread::spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            let _runtime_context = runtime.enter();
            let Ok(client) = reqwest::Client::builder()
                .https_only(true)
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(12))
                .pool_max_idle_per_host(2)
                .build()
            else {
                return;
            };
            while runtime.block_on(commands.changed()).is_ok() {
                let Some(mut job) = commands.borrow_and_update().clone() else {
                    continue;
                };
                runtime.block_on(async {
                    let mut set = tokio::task::JoinSet::new();
                    let mut urls = job.urls.drain(..);
                    loop {
                        while set.len() < CONCURRENT
                            && !*job.cancelled.borrow()
                            && let Some((row, url)) = urls.next()
                        {
                            let client = client.clone();
                            set.spawn(async move {
                                (row, fetch_sized(&client, &url, max_edge, max_edge).await)
                            });
                        }
                        if set.is_empty() {
                            break;
                        }
                        tokio::select! {
                            biased;
                            _ = job.cancelled.changed() => {
                                set.abort_all();
                                break;
                            }
                            done = set.join_next() => {
                                if let Some(Ok((row, Some(pixels)))) = done
                                    && !*job.cancelled.borrow()
                                {
                                    out.lock().unwrap().push(Ready {
                                        generation: job.generation,
                                        row,
                                        pixels,
                                    });
                                    wake();
                                }
                            }
                        }
                    }
                });
            }
        });
        Self {
            generation: Cell::new(0),
            max_avatars,
            command: Some(command),
            cancellation: RefCell::new(None),
            results,
            thread: Some(thread),
        }
    }
    #[cfg(test)]
    pub fn generation(&self) -> u64 {
        self.generation.get()
    }
    /// Retire the current job and any published-but-unconsumed results.
    pub fn cancel(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
        if let Some(sender) = self.cancellation.borrow_mut().take() {
            sender.send_replace(true);
        }
        if let Some(command) = &self.command {
            command.send_replace(None);
        }
        self.results.lock().unwrap().clear();
    }
    /// Replace the job with portraits for at most one page of rows.
    pub fn request(&self, mut urls: Vec<(usize, String)>) {
        self.cancel();
        urls.truncate(self.max_avatars);
        if urls.is_empty() {
            return;
        }
        let (sender, cancelled) = watch::channel(false);
        *self.cancellation.borrow_mut() = Some(sender);
        if let Some(command) = &self.command {
            command.send_replace(Some(Job {
                generation: self.generation.get(),
                urls,
                cancelled,
            }));
        }
    }
    /// Results for the current generation only.
    pub fn take(&self) -> Vec<Ready> {
        drain_current(&mut self.results.lock().unwrap(), self.generation.get())
    }
}
impl Drop for Avatars {
    fn drop(&mut self) {
        self.cancel();
        self.command.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready(generation: u64, row: usize) -> Ready {
        Ready {
            generation,
            row,
            pixels: image::RgbaImage::new(2, 2),
        }
    }

    #[test]
    fn stale_page_results_are_never_handed_to_the_current_page() {
        let mut results = vec![ready(3, 0), ready(4, 1), ready(3, 2), ready(4, 3)];
        let current = drain_current(&mut results, 4);
        assert_eq!(current.iter().map(|r| r.row).collect::<Vec<_>>(), [1, 3]);
        assert!(results.is_empty(), "stale results are discarded, not kept");
    }

    #[test]
    fn cancel_and_new_requests_supersede_without_network_or_hostile_hosts() {
        let wakes = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let observed = wakes.clone();
        let avatars = Avatars::new(move || {
            observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        assert_eq!(avatars.generation(), 0);
        // Hosts outside the exact anonymous image policy fail before any I/O.
        avatars.request(vec![
            (0, "https://example.com/a".into()),
            (1, "http://yt3.ggpht.com/a".into()),
            (2, "https://yt3.ggpht.com.evil.test/a".into()),
        ]);
        let first = avatars.generation();
        avatars.request(Vec::new());
        assert!(
            avatars.generation() > first,
            "a new page supersedes the last"
        );
        avatars.cancel();
        std::thread::sleep(Duration::from_millis(200));
        assert!(avatars.take().is_empty());
        assert_eq!(wakes.load(std::sync::atomic::Ordering::SeqCst), 0);
        // The request bound is one page.
        avatars.request(
            (0..50)
                .map(|i| (i, "https://example.com/a".into()))
                .collect(),
        );
        drop(avatars); // cancels and joins the worker thread
    }
}
