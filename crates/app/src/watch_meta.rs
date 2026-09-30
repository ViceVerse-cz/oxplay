// SPDX-License-Identifier: GPL-3.0-or-later
//! Native guest watch-page data (InnerTube `next`), fetched on its own worker
//! in parallel with the extractor's stream resolve for accepted guest
//! selections only. It supplies the real related list, metadata/description,
//! chapters, the creator's public portrait/subscriber count and the first
//! comments continuation. Any failure keeps the existing fallbacks: the
//! extractor's resolved details and the captured guest-page related rows.
use crate::{App, UiState};
use serein_core::{CancellationToken, OperationContext, ProviderError, ResolvedPlayback, VideoId};
use serein_youtube::{comments::CommentCursor, watch::WatchPage};
use slint::ComponentHandle;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{Arc, Condvar, Mutex},
    thread,
};

struct Job {
    generation: u64,
    video: VideoId,
    cancel: CancellationToken,
}
type Ready = (u64, VideoId, Result<WatchPage, ProviderError>);
#[derive(Default)]
struct Mailbox {
    next: Option<Job>,
    stop: bool,
}

pub struct State {
    generation: Cell<u64>,
    video: RefCell<Option<VideoId>>,
    page: RefCell<Option<Rc<WatchPage>>>,
    active: RefCell<Option<CancellationToken>>,
    mailbox: Arc<(Mutex<Mailbox>, Condvar)>,
    result: Arc<Mutex<Option<Ready>>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl State {
    /// The worker owns no network client until its first job; the transport is
    /// the resolver's shared anonymous instance (shared 429 cooldown).
    pub fn new(app: slint::Weak<App>, resolver: crate::resolver::SharedResolver) -> Self {
        let mailbox = Arc::new((Mutex::new(Mailbox::default()), Condvar::new()));
        let result = Arc::new(Mutex::new(None));
        let (work, out) = (mailbox.clone(), result.clone());
        let thread = thread::spawn(move || {
            loop {
                let job = {
                    let (lock, ready) = &*work;
                    let mut mailbox = lock.lock().unwrap_or_else(|e| e.into_inner());
                    while mailbox.next.is_none() && !mailbox.stop {
                        mailbox = ready.wait(mailbox).unwrap_or_else(|e| e.into_inner());
                    }
                    if mailbox.stop {
                        break;
                    }
                    mailbox.next.take().unwrap()
                };
                let operation = OperationContext {
                    request_id: job.generation,
                    session_generation: 0,
                    cancel: job.cancel.clone(),
                };
                let page = resolver.native_on_worker().and_then(|transport| {
                    serein_youtube::watch::watch_page(&transport, &job.video, &operation)
                });
                if !job.cancel.is_cancelled() {
                    *out.lock().unwrap_or_else(|e| e.into_inner()) =
                        Some((job.generation, job.video, page));
                    let _ = app.upgrade_in_event_loop(|app| app.invoke_watch_meta_wake());
                }
            }
        });
        Self {
            generation: Cell::new(0),
            video: RefCell::new(None),
            page: RefCell::new(None),
            active: RefCell::new(None),
            mailbox,
            result,
            thread: Some(thread),
        }
    }
    fn retire(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
        if let Some(cancel) = self.active.borrow_mut().take() {
            cancel.cancel();
        }
        if let Some(job) = self
            .mailbox
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .next
            .take()
        {
            job.cancel.cancel();
        }
        self.result.lock().unwrap_or_else(|e| e.into_inner()).take();
        self.video.borrow_mut().take();
        self.page.borrow_mut().take();
    }
    fn page_for(&self, video: &VideoId) -> Option<Rc<WatchPage>> {
        self.page
            .borrow()
            .as_ref()
            .filter(|page| page.video == *video)
            .cloned()
    }
    fn request(&self, video: &VideoId) {
        if self.video.borrow().as_ref() == Some(video) && self.page.borrow().is_some() {
            return; // Same selection (e.g. a failed-file retry) keeps its page.
        }
        self.retire();
        let cancel = CancellationToken::default();
        *self.active.borrow_mut() = Some(cancel.clone());
        *self.video.borrow_mut() = Some(video.clone());
        let (lock, ready) = &*self.mailbox;
        lock.lock().unwrap_or_else(|e| e.into_inner()).next = Some(Job {
            generation: self.generation.get(),
            video: video.clone(),
            cancel,
        });
        ready.notify_one();
    }
    /// The worker's result only if it still belongs to the current request.
    fn take_current(&self) -> Option<(VideoId, Result<WatchPage, ProviderError>)> {
        let (generation, video, result) = self
            .result
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()?;
        if generation != self.generation.get() || self.video.borrow().as_ref() != Some(&video) {
            return None;
        }
        self.active.borrow_mut().take();
        Some((video, result))
    }
    fn merge(&self, item: &mut ResolvedPlayback) {
        if !item.guest || item.session_generation != 0 {
            return;
        }
        if let Some(page) = self.page_for(&item.video.id) {
            item.details = page.merge_details(&item.details, item.video.duration);
        }
    }
    fn hint(
        &self,
        video: &serein_core::VideoSummary,
    ) -> Option<crate::channel_avatar::NativeProfile> {
        let page = self.page_for(&video.id)?;
        (page.channel_id.is_some() && page.channel_id == video.channel_id).then(|| {
            crate::channel_avatar::NativeProfile {
                avatar_url: page.channel_avatar_url.clone(),
                subscriber_count: page.channel_subscriber_count,
            }
        })
    }
}
impl Drop for State {
    fn drop(&mut self) {
        self.retire();
        self.mailbox
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stop = true;
        self.mailbox.1.notify_one();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Start the native read for an accepted guest selection. Supersedes any
/// earlier page; the resolve worker and its generation are unaffected.
pub fn request(state: &UiState, video: &VideoId) {
    state.watch_meta.request(video);
}

/// Account/local playback and explicit clearing: no guest page survives.
pub fn clear(state: &UiState) {
    state.watch_meta.retire();
}

/// Replace extractor details with the merged native view (field by field).
/// Only guest items with an accepted page for the same video are changed.
pub fn merge_into(state: &UiState, item: &mut ResolvedPlayback) {
    state.watch_meta.merge(item);
}

/// The first comments continuation from the accepted page, if any.
pub fn comment_start(state: &UiState, video: &VideoId) -> Option<CommentCursor> {
    state.watch_meta.page_for(video)?.comments.clone()
}

/// Public creator portrait/subscriber count for the resolved channel only.
pub fn channel_hint(
    state: &UiState,
    video: &serein_core::VideoSummary,
) -> Option<crate::channel_avatar::NativeProfile> {
    state.watch_meta.hint(video)
}

fn receive(app: &App, state: &Rc<UiState>) {
    let s = &state.watch_meta;
    let Some((video, result)) = s.take_current() else {
        return;
    };
    // Guest-only: account authority never receives anonymous metadata.
    if app.get_account_playback_active() || crate::account_playback::authorization(state).is_some()
    {
        return;
    }
    // Failure keeps the extractor details and the captured related snapshot.
    let Ok(page) = result else { return };
    let page = Rc::new(page);
    *s.page.borrow_mut() = Some(page.clone());
    if !page.related.is_empty() {
        crate::watch_context::replace_native(app, state, &page.related);
    }
    let published = app.get_loaded()
        && app.get_remote_video()
        && state.current_video.borrow().as_ref().map(|v| &v.id) == Some(&video);
    if !published {
        // Still resolving: fill placeholders the selection could not provide.
        if app.get_watch_loading() && app.get_page() == 2 {
            if app.get_video_title().is_empty()
                && let Some(title) = &page.title
            {
                app.set_video_title(title.as_str().into());
            }
            if app.get_video_channel().is_empty()
                && let Some(channel) = &page.channel
            {
                app.set_video_channel(channel.as_str().into());
            }
            if let Some(count) = page.channel_subscriber_count {
                app.set_watch_channel_subscribers(
                    format!(
                        "{} subscribers",
                        crate::display_format::compact_count(count)
                    )
                    .into(),
                );
            }
        }
        return;
    }
    // Published before the native page arrived: upgrade the same item in place.
    if let Some(details) = crate::playback_ui::upgrade_guest_details(app, state, &video, |item| {
        page.merge_details(&item.details, item.video.duration)
    }) {
        crate::comments_ui::refresh_details(app, state, &video, &details);
    }
}

pub fn bind(app: &App, state: &Rc<UiState>) {
    let weak = app.as_weak();
    let state = Rc::downgrade(state);
    app.on_watch_meta_wake(move || {
        if let (Some(app), Some(state)) = (weak.upgrade(), state.upgrade()) {
            receive(&app, &state);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serein_core::{ChannelId, MediaTrack, MediaUrl, VideoDetails, VideoSummary};

    /// No worker thread, network or event loop: the mailbox is inspected directly.
    fn idle() -> State {
        State {
            generation: Cell::new(0),
            video: RefCell::new(None),
            page: RefCell::new(None),
            active: RefCell::new(None),
            mailbox: Arc::new((Mutex::new(Mailbox::default()), Condvar::new())),
            result: Arc::new(Mutex::new(None)),
            thread: None,
        }
    }
    fn id(n: u8) -> VideoId {
        VideoId::new(&format!("synthetic{n:02}")).unwrap()
    }
    fn channel() -> ChannelId {
        ChannelId::new("UCabcdefghijklmnopqrstuv").unwrap()
    }
    /// TEST FIXTURE: synthetic extractor result; never a real signed address.
    fn resolved(video: VideoId, guest: bool) -> ResolvedPlayback {
        ResolvedPlayback {
            video: VideoSummary {
                id: video,
                title: "Synthetic".into(),
                channel: "Synthetic channel".into(),
                channel_id: Some(channel()),
                duration: None,
                thumbnail_url: None,
            },
            details: VideoDetails {
                description: Some("extractor".into()),
                like_count: Some(7),
                ..Default::default()
            },
            video_track: MediaTrack {
                url: MediaUrl::parse("https://r1.googlevideo.com/synthetic").unwrap(),
                codec: None,
                width: None,
                height: None,
                fps: None,
                contains_audio: true,
                headers: Default::default(),
            },
            audio_track: None,
            subtitles: Vec::new(),
            subtitles_truncated: false,
            expires_at: None,
            session_generation: if guest { 0 } else { 3 },
            guest,
        }
    }
    fn native(video: VideoId) -> WatchPage {
        let mut page = WatchPage::new(video);
        page.description = Some("native".into());
        page.channel_id = Some(channel());
        page.channel_avatar_url = Some("https://yt3.ggpht.com/synthetic=s176".into());
        page.channel_subscriber_count = Some(4_720_000);
        page
    }

    #[test]
    fn superseded_requests_are_cancelled_and_their_results_dropped() {
        let state = idle();
        state.request(&id(1));
        let first = state.mailbox.0.lock().unwrap().next.take().unwrap();
        state.request(&id(2));
        assert!(
            first.cancel.is_cancelled(),
            "a new selection cancels the old read"
        );
        let second = state.mailbox.0.lock().unwrap().next.take().unwrap();
        assert_ne!(first.generation, second.generation);
        *state.result.lock().unwrap() = Some((first.generation, id(1), Ok(native(id(1)))));
        assert!(state.take_current().is_none(), "stale generation");
        *state.result.lock().unwrap() = Some((second.generation, id(1), Ok(native(id(1)))));
        assert!(state.take_current().is_none(), "foreign video");
        *state.result.lock().unwrap() = Some((second.generation, id(2), Ok(native(id(2)))));
        assert!(state.take_current().is_some());
        assert!(state.active.borrow().is_none());
        // Clearing (account/local playback) retires in-flight work and pages.
        state.request(&id(3));
        let third = state.mailbox.0.lock().unwrap().next.take().unwrap();
        *state.active.borrow_mut() = Some(third.cancel.clone());
        state.retire();
        assert!(third.cancel.is_cancelled());
        assert!(state.video.borrow().is_none() && state.page.borrow().is_none());
    }

    #[test]
    fn accepted_page_is_reused_for_the_same_selection_only() {
        let state = idle();
        state.request(&id(1));
        *state.page.borrow_mut() = Some(Rc::new(native(id(1))));
        let generation = state.generation.get();
        state.request(&id(1));
        assert_eq!(state.generation.get(), generation, "retry keeps its page");
        assert!(state.page_for(&id(1)).is_some());
        assert!(state.page_for(&id(2)).is_none());
        state.request(&id(2));
        assert!(
            state.page_for(&id(1)).is_none(),
            "a new video drops the old page"
        );
    }

    #[test]
    fn merge_and_avatar_hint_are_scoped_to_the_same_guest_video_and_channel() {
        let state = idle();
        state.request(&id(1));
        *state.page.borrow_mut() = Some(Rc::new(native(id(1))));
        let mut guest = resolved(id(1), true);
        state.merge(&mut guest);
        assert_eq!(guest.details.description.as_deref(), Some("native"));
        assert_eq!(
            guest.details.like_count,
            Some(7),
            "extractor fills native gaps"
        );
        let mut account = resolved(id(1), false);
        state.merge(&mut account);
        assert_eq!(account.details.description.as_deref(), Some("extractor"));
        let mut other = resolved(id(2), true);
        state.merge(&mut other);
        assert_eq!(other.details.description.as_deref(), Some("extractor"));
        let hint = state.hint(&guest.video).unwrap();
        assert!(hint.avatar_url.is_some());
        assert_eq!(hint.subscriber_count, Some(4_720_000));
        let mut foreign = guest.video.clone();
        foreign.channel_id = Some(ChannelId::new("UCzzzzzzzzzzzzzzzzzzzzzz").unwrap());
        assert!(
            state.hint(&foreign).is_none(),
            "never another channel's portrait"
        );
    }
}
