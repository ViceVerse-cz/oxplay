// SPDX-License-Identifier: GPL-3.0-or-later
//! One foreground public channel image, never persisted or account-derived.
use crate::{App, UiState};
use serein_core::{CancellationToken, ChannelId, OperationContext, VideoId, VideoSummary};
use slint::ComponentHandle;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use tokio::sync::watch;

#[derive(Clone)]
struct Job {
    generation: u64,
    channel: ChannelId,
    cancel: CancellationToken,
    cancelled: watch::Receiver<bool>,
}
struct Ready {
    generation: u64,
    pixels: Option<image::RgbaImage>,
}
#[derive(Clone, PartialEq, Eq)]
struct Selection {
    video: VideoId,
    channel: ChannelId,
}

pub struct State {
    selection: RefCell<Option<Selection>>,
    attempted: Cell<bool>,
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
                let url = resolver
                    .get_on_worker()
                    .ok()
                    .and_then(|provider| provider.channel_avatar(&job.channel, &op).ok().flatten());
                let pixels = match url.filter(|_| !op.cancel.is_cancelled()) {
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
                    // At most one decoded result. No video-frame transport.
                    *out.lock().unwrap() = Some(Ready {
                        generation: job.generation,
                        pixels,
                    });
                    let _ = app.upgrade_in_event_loop(|app| app.invoke_channel_avatar_wake());
                }
            }
        });
        Self {
            selection: RefCell::new(None),
            attempted: Cell::new(false),
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
    }
    fn request(&self, channel: ChannelId) {
        self.cancel();
        let cancel = CancellationToken::default();
        let (sender, cancelled) = watch::channel(false);
        *self.cancellation.borrow_mut() = Some((cancel.clone(), sender));
        if let Some(command) = &self.command {
            command.send_replace(Some(Job {
                generation: self.generation.get(),
                channel,
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

fn reset_image(app: &App) {
    app.set_watch_channel_avatar_ready(false);
    app.set_watch_channel_avatar(slint::Image::default());
}

pub fn clear(app: &App, state: &UiState) {
    app.set_watch_channel_available(false);
    state.channel_avatar.cancel();
    state.channel_avatar.selection.borrow_mut().take();
    state.channel_avatar.attempted.set(false);
    reset_image(app);
}

/// Only the acknowledged guest playback path may admit metadata here.
pub fn selected_guest(app: &App, state: &UiState, video: &VideoSummary) {
    clear(app, state);
    app.set_watch_channel_available(video.channel_id.is_some());
    *state.channel_avatar.selection.borrow_mut() =
        video.channel_id.clone().map(|channel| Selection {
            video: video.id.clone(),
            channel,
        });
    observe(app, state);
}

fn relevant(app: &App, state: &UiState) -> bool {
    let selected = state.channel_avatar.selection.borrow();
    let current = state.current_video.borrow();
    selected.as_ref().is_some_and(|selected| {
        current.as_ref().is_some_and(|video| {
            selected.video == video.id && video.channel_id.as_ref() == Some(&selected.channel)
        })
    }) && app.get_page() == 2
        && app.get_loaded()
        && app.get_remote_video()
        && !app.get_account_playback_active()
        && crate::account_playback::authorization(state).is_none()
        && !state.caption_cache.active()
        && !state.hidden.get()
        && !app.get_picture_in_picture()
        && !app.get_fullscreen_active()
}

pub fn observe(app: &App, state: &UiState) {
    let avatar = &state.channel_avatar;
    if !relevant(app, state) {
        // Navigation and compact playback hide the creator row but do not
        // change its identity. Retain the one accepted avatar; cancel only an
        // unfinished request so invisible navigation cannot initiate work.
        if !app.get_watch_channel_avatar_ready() && avatar.attempted.replace(false) {
            avatar.cancel();
        }
        return;
    }
    if !avatar.attempted.replace(true)
        && let Some(selected) = avatar.selection.borrow().as_ref()
    {
        avatar.request(selected.channel.clone());
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
        if ready.generation != state.channel_avatar.generation.get() || !relevant(&app, &state) {
            return;
        }
        let Some(pixels) = ready.pixels else { return };
        let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
            pixels.as_raw(),
            pixels.width(),
            pixels.height(),
        );
        app.set_watch_channel_avatar(slint::Image::from_rgba8(buffer));
        app.set_watch_channel_avatar_ready(true);
    });
}
