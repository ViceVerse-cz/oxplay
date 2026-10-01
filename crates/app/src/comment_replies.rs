// SPDX-License-Identifier: GPL-3.0-or-later
//! YouTube-style reply threads for the visible page of top-level comments.
//!
//! Replies use their own worker, so opening a thread never supersedes
//! playback, search or the comment page on the shared catalog worker. Jobs run
//! one at a time in request order and each thread owns at most one job. A new
//! page, video, disabled setting, account playback or clear retires every
//! thread: queued and running jobs are cancelled and late results are dropped
//! by page generation and job identity. Only native pages carry reply cursors,
//! so extractor-fallback pages never offer a toggle. Reply portraits reuse the
//! comment avatar pipeline through a second, separately bounded instance.
use crate::{CommentReply, CommentRow, comment_avatars::Avatars, display_format};
use oxplay_core::{CancellationToken, CommentSummary, OperationContext, ProviderError, VideoId};
use oxplay_youtube::comments::{REPLY_PAGE_SIZE, ReplyCursor, ReplyPage};
use slint::{Model, ModelRc, SharedString, VecModel};
use std::{
    cell::{Cell, OnceCell, RefCell},
    collections::VecDeque,
    rc::Rc,
    sync::{Arc, Condvar, Mutex},
    thread,
};

/// Replies held across all threads of one comment page. Collapsed threads are
/// released first (and reload when reopened) when a new page needs room.
const MAX_PAGE_REPLIES: usize = 300;
/// Reply portraits are drawn at 24px; 48px covers 2x displays.
const AVATAR_EDGE: u32 = 48;
const LIMIT_REACHED: &str = "Reply browsing limit reached.";
const NO_ROOM: &str = "Hide other reply threads to load more replies.";

struct Job {
    generation: u64,
    id: u64,
    row: usize,
    video: VideoId,
    cursor: ReplyCursor,
    cancel: CancellationToken,
}
struct Ready {
    generation: u64,
    id: u64,
    row: usize,
    result: Result<ReplyPage, ProviderError>,
}
#[derive(Default)]
struct Mailbox {
    queue: VecDeque<Job>,
    active: Option<CancellationToken>,
    stop: bool,
}

type Spawn = Box<dyn FnOnce() -> thread::JoinHandle<()>>;

/// One finite FIFO worker. `Default` is idle (no thread), for tests.
#[derive(Default)]
pub struct Worker {
    mailbox: Arc<(Mutex<Mailbox>, Condvar)>,
    results: Arc<Mutex<Vec<Ready>>>,
    /// Starts the thread on the first job, so sessions that never open a
    /// reply thread pay for no extra thread.
    spawn: RefCell<Option<Spawn>>,
    thread: RefCell<Option<thread::JoinHandle<()>>>,
}
impl Worker {
    /// The transport is the resolver's shared anonymous instance, so its 429
    /// cooldown also covers reply reads. Neither the thread nor a network
    /// client exists until the first job.
    fn new(resolver: crate::resolver::SharedResolver, wake: impl Fn() + Send + 'static) -> Self {
        let mailbox = Arc::new((Mutex::new(Mailbox::default()), Condvar::new()));
        let results = Arc::new(Mutex::new(Vec::new()));
        let (work, out) = (mailbox.clone(), results.clone());
        let spawn: Spawn = Box::new(move || {
            thread::spawn(move || {
                loop {
                    let job = {
                        let (lock, ready) = &*work;
                        let mut mailbox = lock.lock().unwrap_or_else(|e| e.into_inner());
                        while mailbox.queue.is_empty() && !mailbox.stop {
                            mailbox = ready.wait(mailbox).unwrap_or_else(|e| e.into_inner());
                        }
                        if mailbox.stop {
                            break;
                        }
                        let job = mailbox.queue.pop_front().unwrap();
                        mailbox.active = Some(job.cancel.clone());
                        job
                    };
                    let operation = OperationContext {
                        request_id: job.id,
                        session_generation: 0,
                        cancel: job.cancel.clone(),
                    };
                    let result = resolver.native_on_worker().and_then(|transport| {
                        oxplay_youtube::comments::native_replies(
                            &transport,
                            &job.video,
                            &job.cursor,
                            &operation,
                        )
                    });
                    if !job.cancel.is_cancelled() {
                        out.lock().unwrap_or_else(|e| e.into_inner()).push(Ready {
                            generation: job.generation,
                            id: job.id,
                            row: job.row,
                            result,
                        });
                        wake();
                    }
                }
            })
        });
        Self {
            mailbox,
            results,
            spawn: RefCell::new(Some(spawn)),
            thread: RefCell::default(),
        }
    }
    fn submit(&self, job: Job) {
        if let Some(spawn) = self.spawn.take() {
            *self.thread.borrow_mut() = Some(spawn());
        }
        let (lock, ready) = &*self.mailbox;
        lock.lock()
            .unwrap_or_else(|e| e.into_inner())
            .queue
            .push_back(job);
        ready.notify_one();
    }
    /// Cancel queued and running work and forget published results.
    fn cancel_all(&self) {
        {
            let mut mailbox = self.mailbox.0.lock().unwrap_or_else(|e| e.into_inner());
            for job in mailbox.queue.drain(..) {
                job.cancel.cancel();
            }
            if let Some(active) = mailbox.active.take() {
                active.cancel();
            }
        }
        self.results
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }
    fn take(&self) -> Vec<Ready> {
        std::mem::take(&mut *self.results.lock().unwrap_or_else(|e| e.into_inner()))
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel_all();
        self.mailbox
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stop = true;
        self.mailbox.1.notify_one();
        if let Some(thread) = self.thread.get_mut().take() {
            let _ = thread.join();
        }
    }
}

struct Thread {
    /// Parent comment ID; results for any other parent are rejected.
    id: String,
    first: ReplyCursor,
    next: Option<ReplyCursor>,
    rows: Rc<VecModel<CommentReply>>,
    /// Portrait URLs of the published replies, in row order.
    urls: Vec<Option<String>>,
    loaded: bool,
    expanded: bool,
    pending: Option<(u64, CancellationToken)>,
}
impl Thread {
    fn held(&self) -> usize {
        self.urls.len()
            + if self.pending.is_some() {
                REPLY_PAGE_SIZE
            } else {
                0
            }
    }
}

/// UI-thread reply state for the published comment page, by top-level row.
#[derive(Default)]
pub struct Replies {
    generation: Cell<u64>,
    jobs: Cell<u64>,
    threads: RefCell<Vec<Option<Thread>>>,
    worker: Worker,
    /// Built on the first portrait request; its thread creates a runtime and
    /// TLS client, which sessions that never open replies should not pay for.
    avatars: OnceCell<Avatars>,
    start_avatars: RefCell<Option<Box<dyn FnOnce() -> Avatars>>>,
    /// (top-level row, reply index, URL) of the portraits last requested.
    avatar_slots: RefCell<Vec<(usize, usize, String)>>,
}

fn label(count: Option<u64>) -> String {
    match count {
        Some(1) => "1 reply".into(),
        Some(count) if count > 1 => format!("{} replies", display_format::compact_count(count)),
        _ => "Replies".into(),
    }
}
fn update(rows: &VecModel<CommentRow>, row: usize, change: impl FnOnce(&mut CommentRow)) {
    if let Some(mut data) = rows.row_data(row) {
        change(&mut data);
        rows.set_row_data(row, data);
    }
}

impl Replies {
    /// `wake` runs on worker threads after each reply page or portrait.
    pub fn new(
        resolver: crate::resolver::SharedResolver,
        wake: impl Fn() + Send + Sync + Clone + 'static,
    ) -> Self {
        let portraits = wake.clone();
        Self {
            worker: Worker::new(resolver, wake),
            start_avatars: RefCell::new(Some(Box::new(move || {
                Avatars::with_limits(AVATAR_EDGE, REPLY_PAGE_SIZE, portraits)
            }))),
            ..Default::default()
        }
    }
    /// Forget every thread of the page being replaced or removed.
    pub fn retire(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
        self.worker.cancel_all();
        for thread in self.threads.borrow_mut().drain(..).flatten() {
            if let Some((_, cancel)) = thread.pending {
                cancel.cancel();
            }
        }
        self.avatar_slots.borrow_mut().clear();
        if let Some(avatars) = self.avatars.get() {
            avatars.cancel();
        }
    }
    /// Register the next published top-level row (call in row order after
    /// [`Self::retire`]). Returns its toggle label (empty: no toggle) and its
    /// reply model.
    pub fn attach(
        &self,
        comment: &CommentSummary,
        cursor: Option<ReplyCursor>,
    ) -> (SharedString, ModelRc<CommentReply>) {
        let mut threads = self.threads.borrow_mut();
        let Some(first) = cursor.filter(|cursor| cursor.parent() == comment.id) else {
            threads.push(None);
            return (SharedString::new(), ModelRc::default());
        };
        let rows = Rc::new(VecModel::default());
        threads.push(Some(Thread {
            id: comment.id.clone(),
            first,
            next: None,
            rows: rows.clone(),
            urls: Vec::new(),
            loaded: false,
            expanded: false,
            pending: None,
        }));
        (label(comment.reply_count).into(), rows.into())
    }
    /// Expand (loading the first page once) or collapse one thread.
    pub fn toggle(&self, rows: &VecModel<CommentRow>, video: &VideoId, row: usize) {
        let mut threads = self.threads.borrow_mut();
        let Some(Some(thread)) = threads.get_mut(row) else {
            return;
        };
        if thread.expanded {
            thread.expanded = false;
            if let Some((_, cancel)) = thread.pending.take() {
                cancel.cancel();
            }
            update(rows, row, |data| {
                data.replies_expanded = false;
                data.replies_loading = false;
            });
            return;
        }
        thread.expanded = true;
        let first = (!thread.loaded && thread.pending.is_none()).then(|| thread.first.clone());
        update(rows, row, |data| data.replies_expanded = true);
        drop(threads);
        match first {
            Some(cursor) => self.load(rows, video, row, cursor),
            None => self.request_avatars(row),
        }
    }
    /// "Show more replies" for an expanded thread with a continuation.
    pub fn more(&self, rows: &VecModel<CommentRow>, video: &VideoId, row: usize) {
        let cursor = match self.threads.borrow().get(row) {
            Some(Some(thread)) if thread.expanded && thread.pending.is_none() => {
                thread.next.clone()
            }
            _ => None,
        };
        if let Some(cursor) = cursor {
            self.load(rows, video, row, cursor);
        }
    }
    fn load(&self, rows: &VecModel<CommentRow>, video: &VideoId, row: usize, cursor: ReplyCursor) {
        let mut threads = self.threads.borrow_mut();
        if !Self::reserve(rows, &mut threads, row) {
            update(rows, row, |data| data.replies_status = NO_ROOM.into());
            return;
        }
        let Some(Some(thread)) = threads.get_mut(row) else {
            return;
        };
        let id = self.jobs.get().wrapping_add(1);
        self.jobs.set(id);
        let cancel = CancellationToken::default();
        thread.pending = Some((id, cancel.clone()));
        self.worker.submit(Job {
            generation: self.generation.get(),
            id,
            row,
            video: video.clone(),
            cursor,
            cancel,
        });
        update(rows, row, |data| {
            data.replies_loading = true;
            data.replies_status = SharedString::new();
        });
    }
    /// Keep one more reply page within the page-wide bound, releasing
    /// collapsed idle threads if needed.
    fn reserve(rows: &VecModel<CommentRow>, threads: &mut [Option<Thread>], row: usize) -> bool {
        let held = |threads: &[Option<Thread>]| -> usize {
            threads.iter().flatten().map(Thread::held).sum()
        };
        if held(threads) + REPLY_PAGE_SIZE <= MAX_PAGE_REPLIES {
            return true;
        }
        for (index, slot) in threads.iter_mut().enumerate() {
            if let Some(thread) = slot
                && index != row
                && !thread.expanded
                && thread.pending.is_none()
                && thread.loaded
            {
                thread.rows.set_vec(Vec::new());
                thread.urls.clear();
                thread.next = None;
                thread.loaded = false;
                update(rows, index, |data| {
                    data.replies_more = false;
                    data.replies_status = SharedString::new();
                });
            }
        }
        held(threads) + REPLY_PAGE_SIZE <= MAX_PAGE_REPLIES
    }
    /// Publish finished reply pages and decoded portraits for the current page.
    pub fn receive(&self, rows: &VecModel<CommentRow>, video: &VideoId) {
        let mut published = None;
        for ready in self.worker.take() {
            if ready.generation != self.generation.get() {
                continue;
            }
            let mut threads = self.threads.borrow_mut();
            let Some(Some(thread)) = threads.get_mut(ready.row) else {
                continue;
            };
            // A collapsed/cancelled or superseded job never publishes.
            if thread.pending.as_ref().map(|(id, _)| *id) != Some(ready.id) {
                continue;
            }
            thread.pending = None;
            let status: SharedString = match ready.result {
                Ok(page) if page.video == *video && page.parent == thread.id => {
                    let replies: Vec<_> = page.replies.into_iter().take(REPLY_PAGE_SIZE).collect();
                    thread.urls.extend(
                        replies
                            .iter()
                            .map(|reply| reply.author_thumbnail_url.clone()),
                    );
                    for reply in replies {
                        thread.rows.push(reply_row(reply));
                    }
                    thread.loaded = true;
                    thread.next = page.next;
                    published = Some(ready.row);
                    if page.limit_reached {
                        LIMIT_REACHED.into()
                    } else if thread.urls.is_empty() {
                        "No replies to show.".into()
                    } else {
                        SharedString::new()
                    }
                }
                Ok(_) => "The provider returned replies for a different comment.".into(),
                Err(ProviderError::Unavailable) => {
                    "Replies are unavailable. Try again later.".into()
                }
                Err(error) => error.to_string().into(),
            };
            let more = thread.next.is_some();
            update(rows, ready.row, |data| {
                data.replies_loading = false;
                data.replies_status = status;
                data.replies_more = more;
            });
        }
        if let Some(row) = published {
            self.request_avatars(row);
        }
        self.take_avatars();
    }
    /// Replace the portrait job: missing portraits of `first`, then of the
    /// other expanded threads, within one reply page.
    fn request_avatars(&self, first: usize) {
        if self.avatars.get().is_none()
            && let Some(start) = self.start_avatars.take()
        {
            let _ = self.avatars.set(start());
        }
        let Some(avatars) = self.avatars.get() else {
            return;
        };
        let threads = self.threads.borrow();
        let order = std::iter::once(first).chain((0..threads.len()).filter(|row| *row != first));
        let mut slots = Vec::new();
        'threads: for row in order {
            let Some(Some(thread)) = threads.get(row) else {
                continue;
            };
            if !thread.expanded {
                continue;
            }
            for (index, url) in thread.urls.iter().enumerate() {
                if slots.len() >= REPLY_PAGE_SIZE {
                    break 'threads;
                }
                if let Some(url) = url
                    && thread
                        .rows
                        .row_data(index)
                        .is_some_and(|reply| !reply.avatar_ready)
                {
                    slots.push((row, index, url.clone()));
                }
            }
        }
        avatars.request(
            slots
                .iter()
                .enumerate()
                .map(|(slot, (_, _, url))| (slot, url.clone()))
                .collect(),
        );
        *self.avatar_slots.borrow_mut() = slots;
    }
    fn take_avatars(&self) {
        let Some(avatars) = self.avatars.get() else {
            return;
        };
        let slots = self.avatar_slots.borrow();
        let threads = self.threads.borrow();
        for ready in avatars.take() {
            let Some((row, index, url)) = slots.get(ready.row) else {
                continue;
            };
            // The reply must still be the one whose portrait was requested.
            let Some(Some(thread)) = threads.get(*row) else {
                continue;
            };
            if thread.urls.get(*index).and_then(Option::as_ref) != Some(url) {
                continue;
            }
            let Some(mut reply) = thread.rows.row_data(*index) else {
                continue;
            };
            let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                ready.pixels.as_raw(),
                ready.pixels.width(),
                ready.pixels.height(),
            );
            reply.avatar = slint::Image::from_rgba8(buffer);
            reply.avatar_ready = true;
            thread.rows.set_row_data(*index, reply);
        }
    }
}

fn reply_row(reply: CommentSummary) -> CommentReply {
    CommentReply {
        author: reply
            .author
            .unwrap_or_else(|| "Author unavailable".into())
            .into(),
        metadata: reply.published_text.unwrap_or_default().into(),
        likes: reply
            .like_count
            .filter(|count| *count > 0)
            .map(display_format::compact_count)
            .unwrap_or_default()
            .into(),
        creator: reply.author_is_uploader,
        content: reply.text.into(),
        avatar: slint::Image::default(),
        avatar_ready: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video() -> VideoId {
        VideoId::new("abcdefghijk").unwrap()
    }
    /// TEST FIXTURE: synthetic comments and continuation tokens only.
    fn comment(id: &str, replies: Option<u64>) -> CommentSummary {
        CommentSummary {
            id: id.into(),
            author: Some("@synthetic".into()),
            author_id: None,
            text: "Synthetic comment".into(),
            published_text: Some("1 day ago".into()),
            like_count: Some(3),
            author_is_uploader: false,
            author_thumbnail_url: None,
            reply_count: replies,
        }
    }
    fn cursor(parent: &str) -> ReplyCursor {
        ReplyCursor::synthetic(video(), parent, &format!("SYNTHETIC_{parent}")).unwrap()
    }
    fn page(parent: &str, count: usize, next: bool) -> ReplyPage {
        ReplyPage {
            video: video(),
            parent: parent.into(),
            replies: (0..count)
                .map(|n| CommentSummary {
                    reply_count: None,
                    author_thumbnail_url: Some(format!("https://yt3.ggpht.com/synthetic{n}")),
                    ..comment(&format!("{parent}.Reply{n}"), None)
                })
                .collect(),
            next: next.then(|| cursor(parent)),
            limit_reached: false,
        }
    }
    /// Attach threads to an idle (thread-less) worker, as `publish` does.
    fn attached(comments: &[(&str, Option<u64>, bool)]) -> (Replies, Rc<VecModel<CommentRow>>) {
        let replies = Replies::default();
        replies.retire();
        let rows = comments
            .iter()
            .map(|(id, count, has_cursor)| {
                let (label, model) =
                    replies.attach(&comment(id, *count), has_cursor.then(|| cursor(id)));
                CommentRow {
                    replies_label: label,
                    replies: model,
                    ..CommentRow::default()
                }
            })
            .collect::<Vec<_>>();
        (replies, Rc::new(VecModel::from(rows)))
    }
    fn queued(replies: &Replies) -> Vec<(u64, u64, usize, CancellationToken)> {
        replies
            .worker
            .mailbox
            .0
            .lock()
            .unwrap()
            .queue
            .iter()
            .map(|job| (job.generation, job.id, job.row, job.cancel.clone()))
            .collect()
    }
    fn deliver(
        replies: &Replies,
        job: (u64, u64, usize),
        result: Result<ReplyPage, ProviderError>,
    ) {
        replies.worker.results.lock().unwrap().push(Ready {
            generation: job.0,
            id: job.1,
            row: job.2,
            result,
        });
    }

    #[test]
    fn only_threads_with_a_native_continuation_offer_a_toggle() {
        let (replies, rows) = attached(&[
            ("UgA", Some(12), true),
            ("UgB", Some(1), true),
            ("UgC", Some(1_234), true),
            ("UgD", None, true),
            // Counted but without a cursor (extractor page): no toggle.
            ("UgE", Some(4), false),
        ]);
        let labels: Vec<_> = rows.iter().map(|row| row.replies_label).collect();
        assert_eq!(
            labels,
            ["12 replies", "1 reply", "1.2K replies", "Replies", ""]
        );
        replies.toggle(&rows, &video(), 4);
        assert!(queued(&replies).is_empty());
        // A cursor naming a different parent is never attached.
        let other = Replies::default();
        let (label, _) = other.attach(&comment("UgA", Some(2)), Some(cursor("UgB")));
        assert!(label.is_empty());
    }

    #[test]
    fn expanding_loads_once_collapsing_cancels_and_stale_results_are_dropped() {
        let (replies, rows) = attached(&[("UgA", Some(12), true), ("UgB", Some(3), true)]);
        replies.toggle(&rows, &video(), 0);
        let jobs = queued(&replies);
        assert_eq!(jobs.len(), 1);
        let row = rows.row_data(0).unwrap();
        assert!(row.replies_expanded && row.replies_loading);
        // Collapsing while loading cancels the thread's only job.
        replies.toggle(&rows, &video(), 0);
        assert!(jobs[0].3.is_cancelled());
        let row = rows.row_data(0).unwrap();
        assert!(!row.replies_expanded && !row.replies_loading);
        // Its late result is dropped by job identity.
        deliver(
            &replies,
            (jobs[0].0, jobs[0].1, 0),
            Ok(page("UgA", 3, false)),
        );
        replies.receive(&rows, &video());
        assert_eq!(rows.row_data(0).unwrap().replies.row_count(), 0);
        // Reopening requests the first page again.
        replies.toggle(&rows, &video(), 0);
        let (generation, id, row, _) = queued(&replies)[1].clone();
        deliver(
            &replies,
            (generation.wrapping_sub(1), id, row),
            Ok(page("UgA", 3, false)),
        );
        deliver(&replies, (generation, id, row), Ok(page("UgA", 10, true)));
        replies.receive(&rows, &video());
        let data = rows.row_data(0).unwrap();
        assert_eq!(data.replies.row_count(), 10, "stale generation dropped");
        assert!(data.replies_more && !data.replies_loading);
        assert_eq!(data.replies.row_data(0).unwrap().author, "@synthetic");
        // Collapse keeps loaded replies; reopening does not refetch them.
        replies.toggle(&rows, &video(), 0);
        replies.toggle(&rows, &video(), 0);
        assert_eq!(queued(&replies).len(), 2);
        // "Show more replies" appends the next page to the same thread.
        replies.more(&rows, &video(), 0);
        let (generation, id, row, _) = queued(&replies)[2].clone();
        let mut last = page("UgA", 2, false);
        last.limit_reached = true;
        deliver(&replies, (generation, id, row), Ok(last));
        replies.receive(&rows, &video());
        let data = rows.row_data(0).unwrap();
        assert_eq!(data.replies.row_count(), 12);
        assert!(!data.replies_more);
        assert_eq!(data.replies_status, LIMIT_REACHED);
        // Results for another parent or video, and errors, only set a status.
        replies.toggle(&rows, &video(), 1);
        let (generation, id, row, _) = queued(&replies)[3].clone();
        deliver(&replies, (generation, id, row), Ok(page("UgA", 1, false)));
        replies.receive(&rows, &video());
        let data = rows.row_data(1).unwrap();
        assert_eq!(data.replies.row_count(), 0);
        assert!(!data.replies_status.is_empty() && !data.replies_loading);
    }

    #[test]
    fn retiring_the_page_cancels_every_job_and_forgets_threads() {
        let (replies, rows) = attached(&[("UgA", Some(12), true), ("UgB", Some(3), true)]);
        replies.toggle(&rows, &video(), 0);
        replies.toggle(&rows, &video(), 1);
        let jobs = queued(&replies);
        assert_eq!(jobs.len(), 2);
        deliver(
            &replies,
            (jobs[0].0, jobs[0].1, 0),
            Ok(page("UgA", 3, false)),
        );
        replies.retire();
        assert!(jobs.iter().all(|job| job.3.is_cancelled()));
        assert!(queued(&replies).is_empty());
        assert!(replies.worker.results.lock().unwrap().is_empty());
        assert!(replies.threads.borrow().is_empty());
        // A result published after retirement never reaches the old rows.
        deliver(
            &replies,
            (jobs[1].0, jobs[1].1, 1),
            Ok(page("UgB", 3, false)),
        );
        replies.receive(&rows, &video());
        assert_eq!(rows.row_data(1).unwrap().replies.row_count(), 0);
        // A real worker with no jobs performs no I/O and joins on drop.
        let worker = Worker::new(
            crate::resolver::SharedResolver::new("relative".into(), "relative".into()),
            || {},
        );
        drop(worker);
    }

    #[test]
    fn collapsed_threads_are_released_when_the_page_bound_is_reached() {
        let threads: Vec<String> = (0..8).map(|n| format!("Ug{n}")).collect();
        let spec: Vec<_> = threads
            .iter()
            .map(|id| (id.as_str(), Some(99), true))
            .collect();
        let (replies, rows) = attached(&spec);
        let full = MAX_PAGE_REPLIES / REPLY_PAGE_SIZE;
        for (row, id) in threads.iter().enumerate().take(full) {
            replies.toggle(&rows, &video(), row);
            let (generation, job, _, _) = queued(&replies).last().unwrap().clone();
            deliver(
                &replies,
                (generation, job, row),
                Ok(page(id, REPLY_PAGE_SIZE, true)),
            );
            replies.receive(&rows, &video());
            assert_eq!(
                rows.row_data(row).unwrap().replies.row_count(),
                REPLY_PAGE_SIZE
            );
        }
        // Everything held is expanded: no room, nothing is evicted.
        replies.toggle(&rows, &video(), full);
        assert_eq!(queued(&replies).len(), full);
        assert_eq!(rows.row_data(full).unwrap().replies_status, NO_ROOM);
        // Once a thread is collapsed it is released for the new page.
        replies.toggle(&rows, &video(), full);
        replies.toggle(&rows, &video(), 0);
        replies.toggle(&rows, &video(), full);
        assert_eq!(queued(&replies).len(), full + 1);
        let released = rows.row_data(0).unwrap();
        assert_eq!(released.replies.row_count(), 0);
        assert!(!released.replies_more);
        assert!(rows.row_data(1).unwrap().replies.row_count() > 0);
    }
}
