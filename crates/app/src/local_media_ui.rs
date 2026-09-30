// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit local selection, bounded worker validation, and an event-driven
//! stop-before-load handoff. Local paths never enter storage or provider models.
use crate::{App, UiState};
use oxplay_media::Snapshot;
use slint::{ComponentHandle, Timer, TimerMode};
use std::{
    cell::{Cell, RefCell},
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, mpsc},
    thread::JoinHandle,
    time::Duration,
};

const VIDEO_EXTENSIONS: &[&str] = &["mp4", "m4v", "mov", "mkv", "webm"];
const SUBTITLE_EXTENSIONS: &[&str] = &["srt", "vtt"];
const MAX_SUBTITLE: u64 = 2 * 1024 * 1024;

#[derive(Clone, Copy)]
enum Kind {
    Video,
    Subtitle,
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct Scope {
    load: u64,
    catalog: u64,
    account: u64,
}
impl Scope {
    fn current(state: &UiState) -> Self {
        Self {
            load: state.player.snapshot().load_request_id,
            catalog: state.worker.borrow().generation(),
            account: crate::account_playback::generation(state),
        }
    }
}
#[derive(Clone, Copy)]
struct Operation {
    serial: u64,
    scope: Scope,
    kind: Kind,
}
struct Request {
    operation: Operation,
    path: PathBuf,
}
struct Response {
    operation: Operation,
    result: Result<PathBuf, &'static str>,
}
struct Worker {
    requests: Option<mpsc::SyncSender<Request>>,
    results: Option<mpsc::Receiver<Response>>,
    thread: Option<JoinHandle<()>>,
}
impl Worker {
    fn new(weak: slint::Weak<App>) -> std::io::Result<Self> {
        let (requests, input) = mpsc::sync_channel::<Request>(1);
        let (output, results) = mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("local-media-validation".into())
            .spawn(move || {
                while let Ok(request) = input.recv() {
                    let result = validate(&request.path, request.operation.kind);
                    if output
                        .send(Response {
                            operation: request.operation,
                            result,
                        })
                        .is_err()
                    {
                        break;
                    }
                    let _ = weak.upgrade_in_event_loop(|app| app.invoke_local_file_wake());
                }
            })?;
        Ok(Self {
            requests: Some(requests),
            results: Some(results),
            thread: Some(thread),
        })
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.results.take();
        self.requests.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
struct Handoff {
    path: PathBuf,
    scope: Scope,
}
struct Subtitle {
    path: Arc<PathBuf>,
    load: u64,
}
pub struct State {
    worker: Worker,
    picker: RefCell<Option<slint::JoinHandle<()>>>,
    serial: Cell<u64>,
    operation: Cell<Option<Operation>>,
    handoff: RefCell<Option<Handoff>>,
    subtitle: RefCell<Option<Subtitle>>,
    deadline: Timer,
}
impl State {
    pub fn new(weak: slint::Weak<App>) -> std::io::Result<Self> {
        Ok(Self {
            worker: Worker::new(weak)?,
            picker: RefCell::new(None),
            serial: Cell::new(0),
            operation: Cell::new(None),
            handoff: RefCell::new(None),
            subtitle: RefCell::new(None),
            deadline: Timer::default(),
        })
    }
    pub fn stop_picker(&self) {
        if let Some(picker) = self.picker.borrow_mut().take() {
            picker.abort();
        }
        self.operation.set(None);
        self.handoff.borrow_mut().take();
        self.subtitle.borrow_mut().take();
        self.deadline.stop();
    }
}

fn validate(path: &Path, kind: Kind) -> Result<PathBuf, &'static str> {
    let path = path
        .canonicalize()
        .map_err(|_| "The selected file could not be opened.")?;
    let text = path.to_str().ok_or("Select a file with a UTF-8 path.")?;
    if text.len() > 4096 || !path.is_absolute() {
        return Err("The selected path is unsupported.");
    }
    let extension = path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let allowed = match kind {
        Kind::Video => VIDEO_EXTENSIONS,
        Kind::Subtitle => SUBTITLE_EXTENSIONS,
    };
    if !allowed.contains(&extension.as_str()) {
        return Err("Select a supported video or subtitle file, not a playlist or link.");
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    let mut file: File = options
        .open(&path)
        .map_err(|_| "The selected file could not be opened.")?;
    let metadata = file
        .metadata()
        .map_err(|_| "The selected file could not be inspected.")?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err("Select a non-empty regular file.");
    }
    match kind {
        Kind::Video => {
            let mut header = [0u8; 16];
            file.read_exact(&mut header)
                .map_err(|_| "The selected video is incomplete.")?;
            let valid = match extension.as_str() {
                "mp4" | "m4v" | "mov" => &header[4..8] == b"ftyp",
                "mkv" | "webm" => header[..4] == [0x1a, 0x45, 0xdf, 0xa3],
                _ => false,
            };
            if !valid {
                return Err(
                    "The file does not match a supported MP4/MOV or Matroska/WebM container.",
                );
            }
        }
        Kind::Subtitle => {
            if metadata.len() > MAX_SUBTITLE {
                return Err("Subtitle files must be no larger than 2 MiB.");
            }
            let mut bytes = Vec::with_capacity(metadata.len() as usize);
            file.take(MAX_SUBTITLE + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| "The subtitle file could not be read.")?;
            if bytes.len() as u64 > MAX_SUBTITLE {
                return Err("Subtitle files must be no larger than 2 MiB.");
            }
            let text = std::str::from_utf8(&bytes)
                .map_err(|_| "Select a UTF-8 SRT or WebVTT subtitle file.")?;
            let text = text.trim_start_matches('\u{feff}').trim_start();
            let mut lines = text.lines();
            let first = lines.next().unwrap_or("");
            let valid = match extension.as_str() {
                "vtt" => {
                    first == "WEBVTT"
                        || first.starts_with("WEBVTT ")
                        || first.starts_with("WEBVTT\t")
                }
                "srt" => {
                    !first.is_empty()
                        && first.bytes().all(|c| c.is_ascii_digit())
                        && lines.next().is_some_and(|line| line.contains(" --> "))
                }
                _ => false,
            };
            if !valid || text.contains('\0') {
                return Err("The file does not look like an SRT or WebVTT subtitle file.");
            }
        }
    }
    Ok(path)
}

fn finish(app: &App, state: &UiState, message: &str) {
    state.local_media.operation.set(None);
    state.local_media.deadline.stop();
    app.set_local_file_busy(false);
    app.set_local_file_cancellable(false);
    if !message.is_empty() {
        app.set_status(message.into());
    }
}
fn subtitle_allowed(app: &App, state: &UiState) -> bool {
    !state.caption_cache.active()
        && state.playback_preferences.ready()
        && app.get_loaded()
        && !app.get_remote_video()
        && !app.get_native_video_child()
        && state.player.current_load_is_active()
}
pub fn cancel(app: &App, state: &UiState) {
    state.local_media.stop_picker();
    finish(app, state, "");
}
fn begin(app: &App, state: &Rc<UiState>, kind: Kind) {
    if state.caption_cache.active() {
        return;
    }
    if !state.playback_preferences.ready() {
        app.set_status("Wait for saved playback settings before opening a file.".into());
        return;
    }
    if app.get_native_video_child()
        || app.get_local_file_busy()
        || (matches!(kind, Kind::Subtitle) && !subtitle_allowed(app, state))
    {
        return;
    }
    let Some(serial) = state.local_media.serial.get().checked_add(1) else {
        app.set_status("File selection identifiers are exhausted. Restart Oxplay.".into());
        return;
    };
    state.local_media.stop_picker();
    state.local_media.serial.set(serial);
    let operation = Operation {
        serial,
        scope: Scope::current(state),
        kind,
    };
    state.local_media.operation.set(Some(operation));
    app.set_local_file_busy(true);
    app.set_local_file_cancellable(true);
    let weak = app.as_weak();
    let state_weak = Rc::downgrade(state);
    let task = slint::spawn_local(async move {
        let picker = rfd::AsyncFileDialog::new();
        let picker = match kind {
            Kind::Video => picker
                .set_title("Open a local video file")
                .add_filter("Video files", VIDEO_EXTENSIONS),
            Kind::Subtitle => picker
                .set_title("Load a subtitle file for this video")
                .add_filter("UTF-8 subtitles", SUBTITLE_EXTENSIONS),
        };
        let file = picker.pick_file().await;
        let (Some(app), Some(state)) = (weak.upgrade(), state_weak.upgrade()) else {
            return;
        };
        if state
            .local_media
            .operation
            .get()
            .is_none_or(|active| active.serial != serial)
        {
            return;
        }
        if state.caption_cache.active() || !state.playback_preferences.ready() {
            cancel(&app, &state);
            return;
        }
        if Scope::current(&state) != operation.scope {
            finish(
                &app,
                &state,
                "File selection cancelled because playback changed.",
            );
            return;
        }
        let Some(file) = file else {
            finish(&app, &state, "");
            return;
        };
        let request = Request {
            operation,
            path: file.path().to_path_buf(),
        };
        if state
            .local_media
            .worker
            .requests
            .as_ref()
            .is_none_or(|worker| worker.try_send(request).is_err())
        {
            finish(
                &app,
                &state,
                "File validation is busy. Try opening the file again.",
            );
        } else {
            app.set_status("Checking the selected file…".into());
        }
    });
    match task {
        Ok(task) => *state.local_media.picker.borrow_mut() = Some(task),
        Err(_) => finish(app, state, "The native file picker is unavailable."),
    }
}

fn deadline(app: &App, state: &Rc<UiState>) {
    let weak = app.as_weak();
    let state_weak = Rc::downgrade(state);
    state
        .local_media
        .deadline
        .start(TimerMode::SingleShot, Duration::from_secs(15), move || {
            let (Some(app), Some(state)) = (weak.upgrade(), state_weak.upgrade()) else {
                return;
            };
            state.local_media.handoff.borrow_mut().take();
            state.local_media.subtitle.borrow_mut().take();
            finish(
                &app,
                &state,
                "The media operation did not finish. Try opening the file again.",
            );
        });
}
fn selected_video(app: &App, state: &Rc<UiState>, path: PathBuf) {
    // Revoke remote work before publishing any local metadata or media.
    crate::guest_playback::cancel(app, state);
    state.worker.borrow_mut().cancel();
    crate::account_playback::clear(app, state);
    crate::playback_ui::clear_local(state);
    crate::share_ui::clear(app, state);
    crate::watch_context::clear(app, state);
    crate::channel_avatar::clear(app, state);
    crate::caption_ui::clear_local(app, state);
    crate::comments_ui::clear_local(app, state);
    crate::chapters_ui::clear(app, state);
    state.current_video.borrow_mut().take();
    state.clock_ui.invalidate();
    state.progress.stop();
    app.set_busy(false);
    app.set_loaded(false);
    app.set_remote_video(false);
    app.set_video_texture(slint::Image::default());
    app.set_video_title("".into());
    app.set_video_channel("".into());
    app.set_rating_known(false);
    if let Err(error) = state.player.stop() {
        finish(app, state, &error.to_string());
        return;
    }
    *state.local_media.handoff.borrow_mut() = Some(Handoff {
        path,
        scope: Scope::current(state),
    });
    // The original picker scope is intentionally retired at this handoff.
    state.local_media.operation.set(None);
    app.set_status("Stopping previous playback before opening the local file…".into());
    deadline(app, state);
    crate::update(app, state);
}
fn selected_subtitle(app: &App, state: &Rc<UiState>, path: PathBuf, load: u64) {
    if !subtitle_allowed(app, state) || state.player.snapshot().load_request_id != load {
        finish(
            app,
            state,
            "Subtitle selection cancelled because playback changed.",
        );
        return;
    }
    let path = Arc::new(path);
    if let Err(error) = state
        .player
        .add_subtitle(&path, "Local subtitle", "", path.clone())
    {
        finish(app, state, &error.to_string());
        return;
    }
    state.local_media.operation.set(None);
    app.set_local_file_cancellable(false);
    *state.local_media.subtitle.borrow_mut() = Some(Subtitle { path, load });
    app.set_status("Loading the selected subtitle…".into());
    deadline(app, state);
}
pub fn observe(app: &App, state: &Rc<UiState>, snapshot: &Snapshot) {
    if state.caption_cache.active() {
        cancel(app, state);
        if app.get_local_subtitle_available() {
            app.set_local_subtitle_available(false);
        }
        return;
    }
    let available = subtitle_allowed(app, state);
    if app.get_local_subtitle_available() != available {
        app.set_local_subtitle_available(available);
    }
    let stale = state
        .local_media
        .handoff
        .borrow()
        .as_ref()
        .is_some_and(|handoff| Scope::current(state) != handoff.scope);
    if stale {
        state.local_media.handoff.borrow_mut().take();
        finish(app, state, "Local playback cancelled by a newer selection.");
    }
    if !snapshot.stop_pending && state.local_media.handoff.borrow().is_some() {
        let handoff = state.local_media.handoff.borrow_mut().take().unwrap();
        if !state.presentation_ready.get() {
            finish(
                app,
                state,
                "Video presentation is unavailable. Restore the window and open the file again.",
            );
        } else {
            match state.player.load_local(&handoff.path) {
                Ok(()) => {
                    app.set_video_title(
                        handoff
                            .path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned()
                            .into(),
                    );
                    app.set_video_channel("Local file · Not saved to history".into());
                    app.set_page(2);
                    app.set_loaded(true);
                    crate::watch_loading::local_finished(app, state);
                    app.invoke_focus_video_mode();
                    finish(
                        app,
                        state,
                        "Local playback · No YouTube request was made for this file.",
                    );
                }
                Err(error) => finish(app, state, &error.to_string()),
            }
        }
    }
    let subtitle = state.local_media.subtitle.borrow();
    if let Some(pending) = subtitle.as_ref() {
        let stale = !available || snapshot.load_request_id != pending.load;
        let selected = !stale && state.player.subtitle_matches(&pending.path);
        if stale || selected {
            drop(subtitle);
            state.local_media.subtitle.borrow_mut().take();
            finish(
                app,
                state,
                if selected {
                    "Local subtitle selected."
                } else {
                    "Subtitle selection cancelled because playback changed."
                },
            );
        }
    }
}
pub fn bind(app: &App, state: &Rc<UiState>) {
    let weak = app.as_weak();
    let state_weak = Rc::downgrade(state);
    app.on_cancel_local_file(move || {
        if let (Some(app), Some(state)) = (weak.upgrade(), state_weak.upgrade())
            && app.get_local_file_cancellable()
        {
            state.local_media.stop_picker();
            finish(&app, &state, "File selection cancelled.");
        }
    });
    for (kind, callback) in [(Kind::Video, true), (Kind::Subtitle, false)] {
        let weak = app.as_weak();
        let state_weak = Rc::downgrade(state);
        let handler = move || {
            if let (Some(app), Some(state)) = (weak.upgrade(), state_weak.upgrade()) {
                begin(&app, &state, kind);
            }
        };
        if callback {
            app.on_open_video_file(handler);
        } else {
            app.on_load_subtitle_file(handler);
        }
    }
    let weak = app.as_weak();
    let state_weak = Rc::downgrade(state);
    app.on_local_file_wake(move || {
        let (Some(app), Some(state)) = (weak.upgrade(), state_weak.upgrade()) else {
            return;
        };
        while let Some(response) = state
            .local_media
            .worker
            .results
            .as_ref()
            .and_then(|results| results.try_recv().ok())
        {
            let operation = response.operation;
            if state
                .local_media
                .operation
                .get()
                .is_none_or(|active| active.serial != operation.serial)
            {
                continue;
            }
            if state.caption_cache.active() || !state.playback_preferences.ready() {
                cancel(&app, &state);
                continue;
            }
            if Scope::current(&state) != operation.scope {
                finish(
                    &app,
                    &state,
                    "File selection cancelled because playback changed.",
                );
                continue;
            }
            match response.result {
                Ok(path) => match operation.kind {
                    Kind::Video => selected_video(&app, &state, path),
                    Kind::Subtitle => selected_subtitle(&app, &state, path, operation.scope.load),
                },
                Err(error) => finish(&app, &state, error),
            }
        }
    });
}
