// SPDX-License-Identifier: GPL-3.0-or-later
//! Two bounded public channel portraits: accepted watch creator and channel page.
use crate::{App, UiState};
use serein_core::{
    CancellationToken, ChannelId, ChannelSummary, OperationContext, VideoId, VideoSummary,
};
use slint::ComponentHandle;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use tokio::sync::watch;

#[derive(Clone, PartialEq, Eq)]
enum Selection {
    Watch { video: VideoId, channel: ChannelId },
    Profile(ChannelId),
}
impl Selection {
    fn channel(&self) -> &ChannelId {
        match self {
            Self::Watch { channel, .. } | Self::Profile(channel) => channel,
        }
    }
}
/// Public creator portrait/subscriber count already read from the native watch
/// page for the same channel. With a portrait URL, the extractor's channel
/// metadata run is skipped; without one, that run remains the fallback.
#[derive(Clone)]
pub struct NativeProfile {
    pub avatar_url: Option<String>,
    pub subscriber_count: Option<u64>,
}
#[derive(Clone)]
struct Job {
    generation: u64,
    selection: Selection,
    hint: Option<NativeProfile>,
    cancel: CancellationToken,
    cancelled: watch::Receiver<bool>,
}
struct Ready {
    generation: u64,
    selection: Selection,
    pixels: Option<image::RgbaImage>,
    subscribers: Option<u64>,
}

pub struct State {
    selection: RefCell<Option<Selection>>,
    watch_hint: RefCell<Option<NativeProfile>>,
    profile: RefCell<Option<ChannelId>>,
    attempted: Cell<bool>,
    profile_attempted: Cell<bool>,
    active: RefCell<Option<Selection>>,
    generation: Cell<u64>,
    command: Option<watch::Sender<Option<Job>>>,
    cancellation: RefCell<Option<(CancellationToken, watch::Sender<bool>)>>,
    result: Arc<Mutex<Option<Ready>>>,
    thread: Option<thread::JoinHandle<()>>,
}
impl State {
    pub fn new(app: slint::Weak<App>, resolver: crate::resolver::SharedResolver) -> Self {
        let (command, mut commands) = watch::channel::<Option<Job>>(None);
        let result = Arc::new(Mutex::new(None));
        let out = result.clone();
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
                .pool_max_idle_per_host(1)
                .build()
            else {
                return;
            };
            while runtime.block_on(commands.changed()).is_ok() {
                let Some(mut job) = commands.borrow_and_update().clone() else {
                    continue;
                };
                let op = OperationContext {
                    request_id: job.generation,
                    session_generation: 0,
                    cancel: job.cancel.clone(),
                };
                if op.cancel.is_cancelled() {
                    continue;
                }
                let (avatar_url, subscribers) = match job.hint.take() {
                    Some(hint) if hint.avatar_url.is_some() => {
                        (hint.avatar_url, hint.subscriber_count)
                    }
                    _ => resolver
                        .get_on_worker()
                        .ok()
                        .and_then(|provider| {
                            provider.channel_profile(job.selection.channel(), &op).ok()
                        })
                        .map_or((None, None), |profile| {
                            (profile.avatar_url, profile.subscriber_count)
                        }),
                };
                let pixels = match avatar_url.filter(|_| !op.cancel.is_cancelled()) {
                    Some(url) => runtime.block_on(async {
                        tokio::select! {
                            biased;
                            _ = job.cancelled.changed() => None,
                            pixels = crate::thumbnails::fetch(&client, &url) => pixels,
                        }
                    }),
                    None => None,
                };
                if !op.cancel.is_cancelled() {
                    // One replaceable decoded result; never video-frame transport.
                    *out.lock().unwrap() = Some(Ready {
                        generation: job.generation,
                        selection: job.selection,
                        pixels,
                        subscribers,
                    });
                    let _ = app.upgrade_in_event_loop(|app| app.invoke_channel_avatar_wake());
                }
            }
        });
        Self {
            selection: RefCell::new(None),
            watch_hint: RefCell::new(None),
            profile: RefCell::new(None),
            attempted: Cell::new(false),
            profile_attempted: Cell::new(false),
            active: RefCell::new(None),
            generation: Cell::new(0),
            command: Some(command),
            cancellation: RefCell::new(None),
            result,
            thread: Some(thread),
        }
    }
    fn cancel(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
        if let Some((token, sender)) = self.cancellation.borrow_mut().take() {
            token.cancel();
            sender.send_replace(true);
        }
        if let Some(command) = &self.command {
            command.send_replace(None);
        }
        self.result.lock().unwrap().take();
        self.active.borrow_mut().take();
    }
    fn request(&self, selection: Selection) {
        self.cancel();
        let cancel = CancellationToken::default();
        let (sender, cancelled) = watch::channel(false);
        *self.cancellation.borrow_mut() = Some((cancel.clone(), sender));
        *self.active.borrow_mut() = Some(selection.clone());
        let hint = match selection {
            Selection::Watch { .. } => self.watch_hint.borrow().clone(),
            Selection::Profile(..) => None,
        };
        if let Some(command) = &self.command {
            command.send_replace(Some(Job {
                generation: self.generation.get(),
                selection,
                hint,
                cancel,
                cancelled,
            }));
        }
    }
}
impl Drop for State {
    fn drop(&mut self) {
        self.cancel();
        self.command.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn clear(app: &App, state: &UiState) {
    app.set_watch_channel_available(false);
    if matches!(
        *state.channel_avatar.active.borrow(),
        Some(Selection::Watch { .. })
    ) {
        state.channel_avatar.cancel();
    }
    state.channel_avatar.selection.borrow_mut().take();
    state.channel_avatar.watch_hint.borrow_mut().take();
    state.channel_avatar.attempted.set(false);
    app.set_watch_channel_avatar_ready(false);
    app.set_watch_channel_avatar(slint::Image::default());
    app.set_watch_channel_subscribers("".into());
}

pub fn clear_profile(app: &App, state: &UiState) {
    if matches!(
        *state.channel_avatar.active.borrow(),
        Some(Selection::Profile(..))
    ) {
        state.channel_avatar.cancel();
    }
    state.channel_avatar.profile.borrow_mut().take();
    state.channel_avatar.profile_attempted.set(false);
    app.set_channel_profile_avatar_ready(false);
    app.set_channel_profile_avatar(slint::Image::default());
    app.set_channel_profile_subscribers("".into());
}

pub fn selected_profile(app: &App, state: &UiState, channel: &ChannelSummary) {
    if state.channel_avatar.profile.borrow().as_ref() != Some(&channel.id) {
        clear_profile(app, state);
        *state.channel_avatar.profile.borrow_mut() = Some(channel.id.clone());
    }
    if let Some(count) = channel.subscriber_count {
        app.set_channel_profile_subscribers(
            format!(
                "{} subscribers",
                crate::display_format::compact_count(count)
            )
            .into(),
        );
    }
    observe(app, state);
}

/// Only acknowledged guest playback may admit public metadata here. A native
/// watch-page hint must already be scoped to this video's resolved channel.
pub fn selected_guest(
    app: &App,
    state: &UiState,
    video: &VideoSummary,
    hint: Option<NativeProfile>,
) {
    clear(app, state);
    *state.channel_avatar.watch_hint.borrow_mut() = hint;
    app.set_watch_channel_available(video.channel_id.is_some());
    *state.channel_avatar.selection.borrow_mut() =
        video.channel_id.clone().map(|channel| Selection::Watch {
            video: video.id.clone(),
            channel,
        });
    observe(app, state);
}

fn relevant(app: &App, state: &UiState) -> Option<Selection> {
    if state.hidden.get() || app.get_picture_in_picture() || app.get_fullscreen_active() {
        return None;
    }
    let avatar = &state.channel_avatar;
    if app.get_page() == 0 && app.get_guest_scope() == 1 && !app.get_home_active() {
        return avatar.profile.borrow().clone().map(Selection::Profile);
    }
    if app.get_page() != 2
        || !app.get_loaded()
        || !app.get_remote_video()
        || app.get_account_playback_active()
        || crate::account_playback::authorization(state).is_some()
        || state.caption_cache.active()
    {
        return None;
    }
    let selected = avatar.selection.borrow();
    let current = state.current_video.borrow();
    selected
        .as_ref()
        .filter(|selected| match selected {
            Selection::Watch { video, channel } => current.as_ref().is_some_and(|current| {
                *video == current.id && current.channel_id.as_ref() == Some(channel)
            }),
            Selection::Profile(..) => false,
        })
        .cloned()
}

pub fn observe(app: &App, state: &UiState) {
    let avatar = &state.channel_avatar;
    let desired = relevant(app, state);
    let active = avatar.active.borrow().clone();
    if active.is_some() && active != desired {
        // Retain accepted portraits, cancel only unfinished invisible work.
        match active {
            Some(Selection::Watch { .. }) => avatar.attempted.set(false),
            Some(Selection::Profile(..)) => avatar.profile_attempted.set(false),
            None => {}
        }
        avatar.cancel();
    }
    let Some(selection) = desired else { return };
    let attempted = match selection {
        Selection::Watch { .. } => &avatar.attempted,
        Selection::Profile(..) => &avatar.profile_attempted,
    };
    if !attempted.replace(true) {
        avatar.request(selection);
    }
}

pub fn bind(app: &App, state: &Rc<UiState>) {
    let weak = app.as_weak();
    let state = Rc::downgrade(state);
    app.on_channel_avatar_wake(move || {
        let (Some(app), Some(state)) = (weak.upgrade(), state.upgrade()) else {
            return;
        };
        let Some(ready) = state.channel_avatar.result.lock().unwrap().take() else {
            return;
        };
        if ready.generation != state.channel_avatar.generation.get()
            || relevant(&app, &state).as_ref() != Some(&ready.selection)
        {
            return;
        }
        state.channel_avatar.active.borrow_mut().take();
        let portrait = ready.pixels.map(|pixels| {
            let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                pixels.as_raw(),
                pixels.width(),
                pixels.height(),
            );
            slint::Image::from_rgba8(buffer)
        });
        let subscribers = ready.subscribers.map(|count| {
            format!(
                "{} subscribers",
                crate::display_format::compact_count(count)
            )
        });
        match ready.selection {
            Selection::Watch { .. } => {
                if let Some(subscribers) = subscribers {
                    app.set_watch_channel_subscribers(subscribers.into());
                }
                if let Some(portrait) = portrait {
                    app.set_watch_channel_avatar(portrait);
                    app.set_watch_channel_avatar_ready(true);
                }
            }
            Selection::Profile(..) => {
                if let Some(subscribers) = subscribers {
                    app.set_channel_profile_subscribers(subscribers.into());
                }
                if let Some(portrait) = portrait {
                    app.set_channel_profile_avatar(portrait);
                    app.set_channel_profile_avatar_ready(true);
                }
            }
        }
    });
}
