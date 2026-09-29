// SPDX-License-Identifier: GPL-3.0-or-later
//! In-process libmpv control and a single-window OpenGL presenter.
//! No decoded pixels cross this interface. Commands are asynchronous; snapshots
//! contain only observed, sanitized properties. This is an experimental adapter.
mod audio_probe;
mod commands;
pub use audio_probe::AudioProbeReply;
mod ffi;
mod gpu_timing;
pub use gpu_timing::GpuTimingStats;
#[cfg(test)]
mod caption_tests;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod native_child;
#[cfg(target_os = "macos")]
pub use native_child::{NativeChildGeometry, NativeChildPresenter, NativeChildStats, NativeRect};
mod pause_intent;
mod presenter;
mod seek_confirmation;
pub mod streams;
#[cfg(test)]
mod tls_tests;
pub use presenter::{GlPresenter, RenderStats};
use std::{
    cell::{Cell, RefCell},
    ffi::{CStr, CString, c_void},
    fmt,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaError(pub String);
impl fmt::Display for MediaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for MediaError {}
pub type Result<T> = std::result::Result<T, MediaError>;
// libmpv0.41 client.h enum mpv_error. These are approximate engine categories,
// not diagnoses of HTTP status, credentials or a particular device. Never
// forward engine logs, filenames or signed stream URLs into a UI error.
fn playback_error(operation: &str, code: i32) -> String {
    let explanation = match code {
        -1 => "The media engine's command queue is full",
        -2 => "The media engine could not allocate memory",
        -3 => "The media engine is not initialized",
        -4 => "The media engine rejected an invalid argument",
        -12 => "The media engine could not complete the requested action",
        -13 => "The media file or stream could not be opened",
        -14 => "Audio output could not start; check the selected output device",
        -15 => "Video output could not start",
        -16 => "No audio or video track is available for playback",
        -17 => "The media format is unrecognized or the content is damaged",
        -18 => "This operation is unsupported by the available media system",
        -19 => "This operation is not implemented by the media engine",
        _ => "The media engine reported an unspecified error",
    };
    format!("{operation}: {explanation} (engine error {code}).")
}
fn checked(code: i32, operation: &str) -> Result<()> {
    if code < 0 {
        Err(MediaError(format!(
            "{operation} failed (libmpv error {code})"
        )))
    } else {
        Ok(())
    }
}
fn cstring(s: &str) -> Result<CString> {
    CString::new(s).map_err(|_| MediaError("Invalid NUL in media argument".into()))
}
fn start_value(position: f64) -> Result<String> {
    if !position.is_finite() || position < 0. {
        return Err(MediaError("Invalid playback start position".into()));
    }
    // mpv 0.41 manual: loadfile's fourth argument is per-file options; third
    // insertion index is required since 0.38 even with the replace action.
    Ok(position.to_string())
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    #[default]
    Idle,
    Buffering,
    Playing,
    Paused,
    Seeking,
    Ended,
    Failed,
}
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// True until reserved stop replies and a fresh native query confirms that
    /// the old video output is destroyed. Caption leases still follow END_FILE.
    pub stop_pending: bool,
    pub state: PlaybackState,
    pub paused: bool,
    /// Last observed native paused-for-cache value; separate from user pause.
    pub paused_for_cache: bool,
    /// Current load has completed native playback restart since its last seek.
    pub playback_restarted: bool,
    pub position: f64,
    pub duration: f64,
    pub volume: f64,
    /// Native global mute observation, independent of volume and persistence.
    pub muted: bool,
    pub mute_observed: bool,
    /// Observed global mpv speed, not merely the last requested setting.
    pub speed: f64,
    pub speed_observed: bool,
    pub speed_updates: u64,
    pub hwdec_current: String,
    pub codec: String,
    pub audio_codec: String,
    pub audio_output: String,
    /// Positive native track ID only; no subtitle text/path enters diagnostics.
    pub subtitle_id: Option<i64>,
    pub subtitle_selection_observed: bool,
    pub subtitle_updates: u64,
    pub diagnostic_silent_audio: bool,
    /// Experimental AO clock smoothing; zero is the unchanged production default.
    pub autosync_factor: u32,
    pub audio_sample_rate: i64,
    pub width: i64,
    pub height: i64,
    pub fps: f64,
    pub dropped_frames: i64,
    pub error: Option<String>,
    pub events_received: u64,
    /// Advances only for actual FILE_LOADED events, not load submissions or seeks.
    pub file_loads: u64,
    pub file_starts: u64,
    /// Latest accepted loadfile submission; zero means none/stopped.
    pub load_request_id: u64,
    /// Request identified by native START_FILE playlist entry; zero is unknown.
    pub active_load_request_id: u64,
    pub failed_load_request_id: Option<u64>,
    /// Dedicated asynchronous time-pos reply, independent of progress polling.
    /// None position means unavailable/failed, never a guessed zero.
    pub resume_position_reply: Option<(u64, Option<f64>)>,
    /// Explicit finite diagnostic only; ordinary playback never requests this.
    pub audio_probe_reply: Option<AudioProbeReply>,
    pub wakeups: u64,
    pub render_notifications: u64,
    pub decoder_dropped_frames: i64,
    pub timing_lead_ms: u32,
    pub prepare_ms: u32,
    pub display_clock_ticks: u64,
    pub display_clock_starts: u64,
    pub display_clock_stops: u64,
    pub display_clock_active: bool,
    pub prevents_display_sleep: bool,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            stop_pending: false,
            state: PlaybackState::Idle,
            paused: true,
            paused_for_cache: false,
            playback_restarted: false,
            position: 0.,
            duration: 0.,
            volume: 100.,
            muted: false,
            mute_observed: false,
            speed: 1.,
            speed_observed: false,
            speed_updates: 0,
            hwdec_current: "unverified".into(),
            codec: String::new(),
            audio_codec: String::new(),
            audio_output: String::new(),
            subtitle_id: None,
            subtitle_selection_observed: false,
            subtitle_updates: 0,
            diagnostic_silent_audio: false,
            autosync_factor: 0,
            audio_sample_rate: 0,
            width: 0,
            height: 0,
            fps: 0.,
            dropped_frames: 0,
            error: None,
            events_received: 0,
            file_loads: 0,
            file_starts: 0,
            load_request_id: 0,
            active_load_request_id: 0,
            failed_load_request_id: None,
            resume_position_reply: None,
            audio_probe_reply: None,
            wakeups: 0,
            render_notifications: 0,
            decoder_dropped_frames: 0,
            timing_lead_ms: 0,
            prepare_ms: 0,
            display_clock_ticks: 0,
            display_clock_starts: 0,
            display_clock_stops: 0,
            display_clock_active: false,
            prevents_display_sleep: false,
        }
    }
}
struct Wake {
    queued: AtomicBool,
    frame: AtomicBool,
    due: AtomicBool,
    count: AtomicU64,
    render_count: AtomicU64,
    timing_origin: Option<std::time::Instant>,
    callback_time_ns: AtomicU64,
    #[cfg(target_os = "macos")]
    clock: macos::ClockSignals,
    callback: Box<dyn Fn() + Send + Sync>,
}
impl Wake {
    fn notify(&self) {
        if !self.queued.swap(true, Ordering::AcqRel) {
            self.count.fetch_add(1, Ordering::Relaxed);
            // Never unwind through libmpv's C ABI, even if the host wake closure panics.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (self.callback)()));
        }
    }
}
unsafe extern "C" fn event_wake(data: *mut c_void) {
    // SAFETY: pointer references an Arc allocation kept alive through core destruction.
    unsafe {
        (&*data.cast::<Wake>()).notify();
    }
}
unsafe extern "C" fn render_wake(data: *mut c_void) {
    // SAFETY: presenter/core own this allocation until callback removal + context free.
    let wake = unsafe { &*data.cast::<Wake>() };
    if let Some(origin) = wake.timing_origin {
        // No clock access when timing is disabled. The newest callback is
        // measured; coalesced callbacks make this a lower bound on queue age.
        wake.callback_time_ns.store(
            (origin.elapsed().as_nanos() as u64).saturating_add(1),
            Ordering::Release,
        );
    }
    wake.render_count.fetch_add(1, Ordering::Relaxed);
    wake.frame.store(true, Ordering::Release);
    #[cfg(target_os = "macos")]
    if wake.clock.enabled.load(Ordering::Acquire) && wake.clock.running.load(Ordering::Acquire) {
        // The active display clock will signal the next valid presentation
        // opportunity. Avoid a redundant UI wake for every decoded frame.
        return;
    }
    wake.notify();
}
fn selected_path(path: &std::path::Path) -> Result<&str> {
    if !path.is_absolute() {
        return Err(MediaError("Select an absolute local media path".into()));
    }
    path.to_str()
        .ok_or_else(|| MediaError("Media path is not UTF-8".into()))
}
struct SubtitleCommand {
    generation: u64,
    entry: i64,
    path: String,
    lease: Arc<dyn Send + Sync>,
}
type SubtitleLeases = std::collections::BTreeMap<u64, SubtitleCommand>;
type PlaybackSubtitleLeases = std::collections::BTreeMap<(i64, String), Arc<dyn Send + Sync>>;
struct Inner {
    raw: *mut ffi::Handle,
    wake: Arc<Wake>,
    snapshot: RefCell<Snapshot>,
    progress_pending: Cell<bool>,
    progress_refresh_pending: Cell<bool>,
    renderer_attached: Cell<bool>,
    #[cfg(target_os = "macos")]
    presentation_clock: RefCell<Option<macos::PresentationClock>>,
    pending_commands: Cell<usize>,
    pause_intent: RefCell<pause_intent::PauseIntent>,
    pending_loads: Cell<usize>,
    stop_requested: Cell<bool>,
    stop_inflight: Cell<bool>,
    stop_barrier: Cell<bool>,
    stop_failed: Cell<bool>,
    stop_waiting_vo: Cell<bool>,
    stop_probe_pending: Cell<bool>,
    stop_probe_again: Cell<bool>,
    seek_pending: Cell<bool>,
    latest_seek: Cell<Option<f64>>,
    streams: Box<streams::Registry>,
    subtitle_leases: RefCell<SubtitleLeases>,
    playback_subtitle_leases: RefCell<PlaybackSubtitleLeases>,
    next_subtitle_reply: Cell<u64>,
    playback_generation: Cell<u64>,
    clock_transport_epoch: Cell<u64>,
    clock_awaiting_seek_event: Cell<bool>,
    seek_confirmation: RefCell<seek_confirmation::Seeks>,
    seek_timer: slint::Timer,
    subtitles_off: Cell<bool>,
    caption_paths: RefCell<std::collections::BTreeSet<String>>,
    selected_caption: RefCell<Option<String>>,
    resume_request: Cell<Option<(u64, u64)>>,
    resume_reply: Cell<Option<(u64, Option<f64>)>>,
    next_resume_request: Cell<u64>,
    audio_probes: RefCell<audio_probe::AudioProbes>,
    audio_probe_reply: Cell<Option<AudioProbeReply>>,
    next_load_request: Cell<u64>,
    active_load_request: Cell<u64>,
    load_entries: RefCell<std::collections::BTreeMap<i64, u64>>,
    active_playlist_entry: Cell<Option<i64>>,
    restarted_entry: Cell<Option<i64>>,
    frame_probe: Cell<Option<(u64, i64)>>,
    frame_ready_load: Cell<u64>,
}
impl Drop for Inner {
    fn drop(&mut self) {
        self.seek_timer.stop();
        // Presenter holds Rc<Inner>, so its GL render context is freed first.
        self.streams.clear();
        // Registration userdata remains owned until every stream has closed.
        unsafe {
            ffi::mpv_set_wakeup_callback(self.raw, None, std::ptr::null_mut());
            ffi::mpv_terminate_destroy(self.raw);
        }
    }
}
/// Nonsecret identity for deferred cosmetic UI updates. A transport operation
/// changes the epoch before its asynchronous native command can complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockIdentity {
    pub load_request_id: u64,
    pub transport_epoch: u64,
}
/// UI-thread-only player; clones share one engine. Never recreate for control updates.
#[derive(Clone)]
pub struct Player {
    inner: Rc<Inner>,
}
impl Player {
    /// `wake` must enqueue UI work, never render, mutate the UI, or call libmpv.
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Result<Self> {
        Self::new_with_ca_file(wake, None)
    }
    /// TLS peer verification is mandatory. The host may provide its reviewed,
    /// prevalidated CA resource; this constructor performs no filesystem IO.
    /// Without an explicit resource, libavformat uses its system CA discovery.
    pub fn new_with_ca_file(
        wake: impl Fn() + Send + Sync + 'static,
        ca_file: Option<&std::path::Path>,
    ) -> Result<Self> {
        let ca_file = ca_file.map(selected_path).transpose()?;
        // Diagnostic A/B control, never imported from mpv user configuration.
        // macOS uses the source-reviewed 50ms lead with display-clock gating.
        // Other unqualified platforms retain the initial zero-lead behavior.
        let timing_lead_ms = match std::env::var("SEREIN_VIDEO_LEAD_MS") {
            Ok(value) => value
                .parse::<u32>()
                .ok()
                .filter(|v| *v <= 100)
                .ok_or_else(|| {
                    MediaError("SEREIN_VIDEO_LEAD_MS must be an integer from 0 to 100".into())
                })?,
            Err(std::env::VarError::NotPresent) => {
                if cfg!(target_os = "macos") {
                    50
                } else {
                    0
                }
            }
            Err(_) => return Err(MediaError("Invalid SEREIN_VIDEO_LEAD_MS value".into())),
        };
        let prepare_ms = match std::env::var("SEREIN_VIDEO_PREPARE_MS") {
            Ok(value) => value
                .parse::<u32>()
                .ok()
                .filter(|v| *v <= 16 && *v <= timing_lead_ms)
                .ok_or_else(|| {
                    MediaError(
                        "SEREIN_VIDEO_PREPARE_MS must be 0..16 and no greater than VIDEO_LEAD_MS"
                            .into(),
                    )
                })?,
            Err(std::env::VarError::NotPresent) => 0,
            Err(_) => return Err(MediaError("Invalid SEREIN_VIDEO_PREPARE_MS value".into())),
        };
        let diagnostic_silent_audio =
            std::env::var_os("SEREIN_DIAGNOSTIC_NULL_AUDIO").is_some_and(|v| v == "1");
        if diagnostic_silent_audio {
            eprintln!(
                "DIAGNOSTIC: timed null audio output, no audible audio; this run cannot pass playback acceptance"
            );
        }
        // mpv v0.41.0 player/video.c:update_avsync_before_frame smooths
        // ao_get_delay() by this factor. Its manual explicitly suggests 30 for
        // inaccurate AO delay, with a 1–2 second response to abrupt A/V offsets.
        // Diagnostic only: frame-rate-dependent smoothing requires A/V testing.
        let autosync_factor =
            if std::env::var_os("SEREIN_DIAGNOSTIC_AUTOSYNC").is_some_and(|v| v == "1") {
                30
            } else {
                0
            };
        if autosync_factor != 0 {
            eprintln!(
                "DIAGNOSTIC: autosync=30 AO clock smoothing; abrupt A/V offsets may take 1–2 seconds to settle; default remains 0"
            );
        }
        let autosync = autosync_factor.to_string();
        let timing_offset = (f64::from(timing_lead_ms) / 1000.).to_string();
        let raw = unsafe { ffi::mpv_create() };
        if raw.is_null() {
            return Err(MediaError("Could not create libmpv".into()));
        }
        let inner = Rc::new(Inner {
            raw,
            wake: Arc::new(Wake {
                queued: AtomicBool::new(false),
                frame: AtomicBool::new(false),
                due: AtomicBool::new(false),
                count: AtomicU64::new(0),
                render_count: AtomicU64::new(0),
                timing_origin: std::env::var_os("SEREIN_MEDIA_TIMING")
                    .is_some_and(|v| v == "1")
                    .then(std::time::Instant::now),
                callback_time_ns: AtomicU64::new(0),
                #[cfg(target_os = "macos")]
                clock: macos::ClockSignals::default(),
                callback: Box::new(wake),
            }),
            snapshot: RefCell::new(Snapshot {
                timing_lead_ms,
                prepare_ms,
                diagnostic_silent_audio,
                autosync_factor,
                ..Snapshot::default()
            }),
            progress_pending: Cell::new(false),
            progress_refresh_pending: Cell::new(false),
            renderer_attached: Cell::new(false),
            #[cfg(target_os = "macos")]
            presentation_clock: RefCell::new(None),
            pending_commands: Cell::new(0),
            pause_intent: RefCell::default(),
            pending_loads: Cell::new(0),
            stop_requested: Cell::new(false),
            stop_inflight: Cell::new(false),
            stop_barrier: Cell::new(false),
            stop_failed: Cell::new(false),
            stop_waiting_vo: Cell::new(false),
            stop_probe_pending: Cell::new(false),
            stop_probe_again: Cell::new(false),
            seek_pending: Cell::new(false),
            latest_seek: Cell::new(None),
            streams: Box::default(),
            subtitle_leases: RefCell::default(),
            playback_subtitle_leases: RefCell::default(),
            next_subtitle_reply: Cell::new(1000),
            playback_generation: Cell::new(0),
            clock_transport_epoch: Cell::new(0),
            clock_awaiting_seek_event: Cell::new(false),
            seek_confirmation: RefCell::default(),
            seek_timer: slint::Timer::default(),
            subtitles_off: Cell::new(false),
            caption_paths: RefCell::default(),
            selected_caption: RefCell::default(),
            resume_request: Cell::new(None),
            resume_reply: Cell::new(None),
            next_resume_request: Cell::new(1 << 63),
            audio_probes: RefCell::default(),
            audio_probe_reply: Cell::new(None),
            next_load_request: Cell::new(1 << 62),
            active_load_request: Cell::new(0),
            load_entries: RefCell::default(),
            active_playlist_entry: Cell::new(None),
            restarted_entry: Cell::new(None),
            frame_probe: Cell::new(None),
            frame_ready_load: Cell::new(0),
        });
        // No inherited scripts, configs, extractor, playlists or ambient cookies.
        for (name, value) in [
            ("config", "no"),
            ("access-references", "no"),
            ("load-scripts", "no"),
            ("video-latency-hacks", "no"),
            ("force-window", "no"),
            // load-scripts does not disable bundled Lua scripts in mpv 0.41.
            ("load-stats-overlay", "no"),
            ("load-console", "no"),
            ("load-select", "no"),
            ("load-positioning", "no"),
            ("load-commands", "no"),
            ("load-context-menu", "no"),
            ("load-auto-profiles", "no"),
            ("ytdl", "no"),
            ("terminal", "no"),
            ("msg-level", "all=no"),
            ("vo", "libmpv"),
            ("hwdec", "auto-safe"),
            ("idle", "yes"),
            ("keep-open", "yes"),
            ("input-default-bindings", "no"),
            ("input-vo-keyboard", "no"),
            ("osc", "no"),
            ("osd-level", "0"),
            ("sub-auto", "no"),
            ("audio-file-auto", "no"),
            ("video-timing-offset", timing_offset.as_str()),
            ("video-sync", "audio"),
            ("autosync", autosync.as_str()),
            ("demuxer-max-bytes", "32MiB"),
            ("demuxer-max-back-bytes", "8MiB"),
            ("network-timeout", "15"),
            ("cookies", "no"),
            // mpv 0.41 stream_lavf.c defaults this option to false; never inherit it.
            ("tls-verify", "yes"),
        ] {
            unsafe {
                checked(
                    ffi::mpv_set_option_string(
                        raw,
                        cstring(name)?.as_ptr(),
                        cstring(value)?.as_ptr(),
                    ),
                    "Media option",
                )?;
            }
        }
        if let Some(ca_file) = ca_file {
            unsafe {
                checked(
                    ffi::mpv_set_option_string(
                        raw,
                        c"tls-ca-file".as_ptr(),
                        cstring(ca_file)?.as_ptr(),
                    ),
                    "Configure media trust store",
                )?;
            }
        }
        if diagnostic_silent_audio {
            unsafe {
                checked(
                    ffi::mpv_set_option_string(raw, c"ao".as_ptr(), c"null".as_ptr()),
                    "Select diagnostic silent audio",
                )?;
            }
        }
        unsafe {
            checked(ffi::mpv_initialize(raw), "Media initialization")?;
            // mpv0.41 emits this after idle_loop destroys the prior VO. It is
            // only a wake hint; a fresh current-vo query supplies the proof.
            checked(ffi::mpv_request_event(raw, 11, 1), "Observe native idle")?;
            inner.streams.install(raw)?;
        }
        for (id, name, format) in [
            (1, "pause", 3),
            (2, "duration", 5),
            (3, "hwdec-current", 1),
            (4, "video-codec", 1),
            (5, "width", 4),
            (6, "height", 4),
            (7, "container-fps", 5),
            (8, "frame-drop-count", 4),
            (9, "volume", 5),
            (10, "paused-for-cache", 3),
            (11, "eof-reached", 3),
            (12, "audio-codec-name", 1),
            (13, "audio-params/samplerate", 4),
            (14, "decoder-frame-drop-count", 4),
            (15, "current-ao", 1),
            (16, "sid", 1),
            (17, "current-tracks/sub/external-filename", 1),
            (18, "speed", 5),
            (19, "mute", 3),
        ] {
            unsafe {
                checked(
                    ffi::mpv_observe_property(raw, id, cstring(name)?.as_ptr(), format),
                    "Observe media property",
                )?;
            }
        }
        unsafe {
            ffi::mpv_set_wakeup_callback(
                raw,
                Some(event_wake),
                Arc::as_ptr(&inner.wake).cast_mut().cast(),
            );
        }
        Ok(Self { inner })
    }
    fn command(&self, args: &[&str]) -> Result<()> {
        self.command_with_reply(args, 100)
    }
    fn command_with_reply(&self, args: &[&str], reply: u64) -> Result<()> {
        // libmpv's event queue is bounded, and we also bound pending commands.
        if self.inner.pending_commands.get() >= 64 {
            return Err(MediaError("Media command queue busy".into()));
        }
        self.submit_command(args, reply)?;
        self.inner
            .pending_commands
            .set(self.inner.pending_commands.get() + 1);
        Ok(())
    }
    fn submit_command(&self, args: &[&str], reply: u64) -> Result<()> {
        let values: Vec<CString> = args.iter().map(|s| cstring(s)).collect::<Result<_>>()?;
        let mut pointers: Vec<_> = values.iter().map(|s| s.as_ptr()).collect();
        pointers.push(std::ptr::null());
        unsafe {
            checked(
                ffi::mpv_command_async(self.inner.raw, reply, pointers.as_ptr()),
                "Media command",
            )?;
        }
        Ok(())
    }
    /// Requires an absolute, host-prevalidated local path originating from explicit
    /// user selection, never provider metadata. Perform filesystem validation on a
    /// worker or before opening the UI; this method performs no filesystem I/O.
    pub fn load_local(&self, path: &std::path::Path) -> Result<()> {
        self.load_local_at(path, 0., false)
    }
    /// Start position is a loadfile-scoped option, so it cannot race a later
    /// FILE_LOADED notification or affect the following unrelated selection.
    pub fn load_local_at(&self, path: &std::path::Path, position: f64, paused: bool) -> Result<()> {
        self.load_local_with_subtitle_at(path, None, position, paused)
    }
    /// The initial subtitle belongs to this exact load, including rapid file
    /// replacement. Both paths have the host-prevalidated absolute-path contract.
    pub fn load_local_with_subtitle_at(
        &self,
        path: &std::path::Path,
        subtitle: Option<&std::path::Path>,
        position: f64,
        paused: bool,
    ) -> Result<()> {
        self.load_file(
            selected_path(path)?,
            None,
            subtitle.map(selected_path).transpose()?,
            position,
            paused,
        )
    }
    /// Accept only policy-validated anonymous direct HTTPS media URLs. Redirect and
    /// manifest policy remains a qualification blocker; never attach account cookies.
    pub fn load_https(&self, video: &str, audio: Option<&str>) -> Result<()> {
        self.load_https_at(video, audio, 0., false)
    }
    pub fn load_https_at(
        &self,
        video: &str,
        audio: Option<&str>,
        position: f64,
        paused: bool,
    ) -> Result<()> {
        if !valid_direct_url(video) || audio.is_some_and(|s| !valid_direct_url(s)) {
            return Err(MediaError(
                "Media URL rejected by anonymous playback policy".into(),
            ));
        }
        self.load_file(video, audio, None, position, paused)
    }
    /// Load policy-owned compressed streams. Sources never expose URLs/headers to
    /// libmpv. Registrations and cancellation are owned through engine shutdown.
    pub fn load_streams_at(
        &self,
        video: Arc<dyn streams::StreamFactory>,
        audio: Option<Arc<dyn streams::StreamFactory>>,
        position: f64,
        paused: bool,
    ) -> Result<()> {
        let pair = self.inner.streams.prepare(video, audio)?;
        let result =
            self.submit_load_file(&pair.video, pair.audio.as_deref(), None, position, paused);
        if result.is_ok() {
            self.inner.streams.commit(&pair);
        } else {
            self.inner.streams.rollback(&pair);
        }
        result
    }
    fn load_file(
        &self,
        path: &str,
        audio: Option<&str>,
        subtitle: Option<&str>,
        position: f64,
        paused: bool,
    ) -> Result<()> {
        self.submit_load_file(path, audio, subtitle, position, paused)?;
        self.inner.streams.clear();
        Ok(())
    }
    fn submit_load_file(
        &self,
        path: &str,
        audio: Option<&str>,
        subtitle: Option<&str>,
        position: f64,
        paused: bool,
    ) -> Result<()> {
        if self.inner.stop_requested.get() {
            return Err(MediaError(
                "Playback is stopping; retry after stop completes".into(),
            ));
        }
        // mpv0.41 mp_add_external_file can attach after playback restarts while
        // its asynchronous open runs. Do not replace until its reply is drained.
        if !self.inner.subtitle_leases.borrow().is_empty() {
            return Err(MediaError(
                "A caption change is still pending; retry playback shortly".into(),
            ));
        }
        let start = start_value(position)?;
        let generation = self
            .inner
            .playback_generation
            .get()
            .checked_add(1)
            .ok_or_else(|| MediaError("Playback identifiers exhausted".into()))?;
        let request = self.inner.next_load_request.get();
        let next_request = request
            .checked_add(1)
            .filter(|value| *value < (1 << 63))
            .ok_or_else(|| MediaError("Load request identifiers exhausted".into()))?;
        // Reserve this load plus at most one following serialized pause command.
        // All admission happens on this UI-thread owner, never in a callback.
        if self.inner.pending_commands.get() > 62 {
            return Err(MediaError("Media command queue busy".into()));
        }
        unsafe {
            commands::loadfile(self.inner.raw, request, path, &start, audio, subtitle)?;
        }
        self.inner
            .pending_loads
            .set(self.inner.pending_loads.get() + 1);
        self.inner.playback_generation.set(generation);
        self.inner.next_load_request.set(next_request);
        self.inner.active_load_request.set(request);
        self.inner.clock_awaiting_seek_event.set(false);
        self.cancel_seek_confirmation();
        self.inner.frame_ready_load.set(0);
        self.inner.frame_probe.set(None);
        {
            let mut snapshot = self.inner.snapshot.borrow_mut();
            snapshot.paused_for_cache = false;
            snapshot.playback_restarted = false;
            snapshot.load_request_id = request;
            snapshot.failed_load_request_id = None;
            snapshot.position = 0.;
            snapshot.duration = 0.;
        }
        self.inner.resume_request.set(None);
        self.inner.resume_reply.set(None);
        self.invalidate_audio_probe();
        self.inner.caption_paths.borrow_mut().clear();
        self.inner.selected_caption.borrow_mut().take();
        self.inner
            .pending_commands
            .set(self.inner.pending_commands.get() + 1);
        self.inner.latest_seek.set(None);
        self.inner.pause_intent.borrow_mut().new_load(paused);
        self.invalidate_resume_position();
        self.apply_pause_intent()
    }
    pub fn set_paused(&self, paused: bool) -> Result<()> {
        self.inner.pause_intent.borrow_mut().user(paused);
        self.invalidate_resume_position();
        self.apply_pause_intent()
    }
    pub fn toggle_pause(&self) -> Result<()> {
        if self.inner.snapshot.borrow().state == PlaybackState::Ended
            && self.inner.pause_intent.borrow().eof_unsettled()
        {
            return Err(MediaError(
                "End-of-file pause is still settling; try Play again shortly".into(),
            ));
        }
        let paused = !self.inner.pause_intent.borrow().desired();
        if !paused && self.inner.snapshot.borrow().state == PlaybackState::Ended {
            // mpv keep-open does not replay on pause=no: seek explicitly, then
            // defer unpause until our existing seek confirmation settles.
            self.seek(0.)?;
        }
        self.set_paused(paused)
    }
    /// Latest accepted user intent (or confirmed native keep-open EOF hold),
    /// excluding the window-occlusion overlay.
    /// This is intentionally distinct from observed Snapshot.paused. Preserve
    /// it when replacing a stream; a delayed property must not undo user input.
    pub fn user_pause_intent(&self) -> bool {
        self.inner.pause_intent.borrow().desired()
    }
    /// Window policy overlays accepted user intent; it never infers whether to
    /// resume from a possibly stale observed pause property. Repeated events are
    /// idempotent. Snapshot.paused continues to report only native observations.
    pub fn set_occluded(&self, hidden: bool) -> Result<()> {
        if !self.inner.pause_intent.borrow_mut().occluded(hidden) {
            return Ok(());
        }
        self.invalidate_resume_position();
        self.apply_pause_intent()
    }
    fn apply_pause_intent(&self) -> Result<()> {
        if let Err(error) = self.submit_pause_intent() {
            // Submission failure is recoverable through an explicit new load,
            // but must never leave an unconfirmed pause playing invisibly.
            let _ = self.stop();
            return Err(error);
        }
        Ok(())
    }
    fn submit_pause_intent(&self) -> Result<()> {
        if self.inner.pending_commands.get() >= 64 {
            // Constant-sized desired state waits for an already queued reply;
            // drain_events services it after slots become available.
            return Ok(());
        }
        let command = self.inner.pause_intent.borrow().prepare()?;
        if let Some(command) = command {
            if !command.paused
                && (self.inner.pause_intent.borrow().eof_unsettled()
                    || self.inner.seek_confirmation.borrow().token().is_some())
            {
                return Ok(());
            }
            self.command_with_reply(
                &["set", "pause", if command.paused { "yes" } else { "no" }],
                command.token,
            )?;
            self.inner.pause_intent.borrow_mut().submitted(command);
        }
        Ok(())
    }
    fn submit_eof_probe(&self) -> Result<()> {
        let entry_matches = self.inner.active_playlist_entry.get().is_some_and(|entry| {
            self.inner.load_entries.borrow().get(&entry).copied()
                == Some(self.inner.active_load_request.get())
        });
        if self.inner.stop_requested.get()
            || self.inner.active_load_request.get() == 0
            || self.inner.pending_loads.get() != 0
            || !entry_matches
            || self.inner.seek_confirmation.borrow().token().is_some()
            || !self.inner.pause_intent.borrow_mut().prepare_eof_probe()
        {
            return Ok(());
        }
        let code =
            unsafe { ffi::mpv_get_property_async(self.inner.raw, 104, c"eof-reached".as_ptr(), 3) };
        if code < 0 {
            let _ = self.inner.pause_intent.borrow_mut().eof_reply(None);
        }
        checked(code, "Confirm native end-of-file pause")
    }
    pub fn seek(&self, seconds: f64) -> Result<()> {
        if !seconds.is_finite() {
            return Err(MediaError("Invalid seek position".into()));
        }
        if self.inner.seek_pending.get() {
            self.inner.latest_seek.set(Some(seconds.max(0.)));
            self.inner.pause_intent.borrow_mut().invalidate_eof();
            self.invalidate_resume_position();
            return Ok(());
        }
        if self.inner.seek_confirmation.borrow().token().is_some() {
            return Err(MediaError(
                "A relative seek is still settling; try again shortly".into(),
            ));
        }
        if !self.current_load_is_active() || !self.inner.snapshot.borrow().playback_restarted {
            return Err(MediaError(
                "Wait for current playback before seeking".into(),
            ));
        }
        self.submit_absolute_seek(seconds.max(0.))
    }
    // Called after a previous seek fully confirms, including from drain_events.
    // This must not borrow Snapshot because the caller may own its mutable guard.
    fn submit_absolute_seek(&self, seconds: f64) -> Result<()> {
        let token = self
            .inner
            .seek_confirmation
            .borrow_mut()
            .begin(self.seek_context())?;
        if let Err(error) =
            self.command_with_reply(&["seek", &seconds.to_string(), "absolute+exact"], token)
        {
            self.cancel_seek_confirmation();
            return Err(error);
        }
        self.inner.seek_pending.set(true);
        self.inner.pause_intent.borrow_mut().invalidate_eof();
        self.inner.clock_awaiting_seek_event.set(true);
        self.invalidate_resume_position();
        self.arm_seek_watchdog(token);
        Ok(())
    }
    pub fn seek_relative(&self, seconds: f64) -> Result<()> {
        if !seconds.is_finite() {
            return Err(MediaError("Invalid seek position".into()));
        }
        if !self.current_load_is_active()
            || self.inner.seek_pending.get()
            || self.inner.latest_seek.get().is_some()
            || !self.inner.snapshot.borrow().playback_restarted
        {
            return Err(MediaError(
                "Wait for current playback before seeking".into(),
            ));
        }
        let token = self
            .inner
            .seek_confirmation
            .borrow_mut()
            .begin(self.seek_context())?;
        if let Err(error) =
            self.command_with_reply(&["seek", &seconds.to_string(), "relative"], token)
        {
            self.cancel_seek_confirmation();
            return Err(error);
        }
        self.inner.pause_intent.borrow_mut().invalidate_eof();
        self.inner.clock_awaiting_seek_event.set(true);
        self.invalidate_resume_position();
        self.arm_seek_watchdog(token);
        Ok(())
    }
    fn arm_seek_watchdog(&self, token: u64) {
        let weak = Rc::downgrade(&self.inner);
        // Remote media can legitimately buffer during a seek. This bounds an
        // unconfirmed transport without confusing queue acceptance with native
        // completion or adding a polling loop. The callback only upgrades Weak.
        self.inner.seek_timer.start(
            slint::TimerMode::SingleShot,
            std::time::Duration::from_secs(30),
            move || {
                if let Some(inner) = weak.upgrade() {
                    Player { inner }.seek_timeout(token);
                }
            },
        );
    }
    fn seek_context(&self) -> seek_confirmation::Context {
        seek_confirmation::Context {
            load: self.inner.active_load_request.get(),
            entry: self.inner.active_playlist_entry.get(),
        }
    }
    fn cancel_seek_confirmation(&self) {
        self.inner.seek_timer.stop();
        self.inner.seek_confirmation.borrow_mut().cancel();
        self.inner.clock_awaiting_seek_event.set(false);
        self.inner.seek_pending.set(false);
        self.inner.latest_seek.set(None);
    }
    fn finish_seek_confirmation(&self, decision: seek_confirmation::Decision) -> Result<()> {
        if matches!(
            decision,
            seek_confirmation::Decision::Completed | seek_confirmation::Decision::Failed
        ) {
            self.inner.seek_timer.stop();
            self.inner.clock_awaiting_seek_event.set(false);
            self.inner.seek_pending.set(false);
            if let Some(position) = self.inner.latest_seek.take() {
                self.submit_absolute_seek(position)?;
            }
        }
        Ok(())
    }
    fn seek_timeout(&self, token: u64) {
        let timed_out = self.inner.seek_confirmation.borrow_mut().timeout(token);
        if !timed_out {
            return;
        }
        let stop = self.stop();
        let mut snapshot = self.inner.snapshot.borrow_mut();
        snapshot.error = Some(
            if stop.is_ok() {
                "Seek did not settle; playback was stopped. Reload to retry."
            } else {
                "Seek did not settle and stopping failed. Stop or reload playback to retry."
            }
            .into(),
        );
        drop(snapshot);
        self.inner.wake.notify();
    }
    pub fn set_volume(&self, volume: f64) -> Result<()> {
        if !volume.is_finite() {
            return Err(MediaError("Invalid volume".into()));
        }
        self.command(&["set", "volume", &volume.clamp(0., 100.).to_string()])
    }
    pub fn set_speed(&self, speed: f64) -> Result<()> {
        if !speed.is_finite() || !(0.25..=4.).contains(&speed) {
            return Err(MediaError("Invalid playback speed".into()));
        }
        self.command(&["set", "speed", &speed.to_string()])
    }
    pub fn toggle_mute(&self) -> Result<()> {
        self.command(&["cycle", "mute"])
    }
    /// Attach a host-prevalidated local caption. The host checks playback/session
    /// generation before submitting and retains its file for the playback's full
    /// lifetime. This extra lease survives asynchronous command completion or
    /// engine destruction, even if that playback is replaced while adding.
    /// Lease Drop must only queue worker cleanup, never perform UI-thread I/O.
    pub fn add_subtitle(
        &self,
        path: &std::path::Path,
        title: &str,
        language: &str,
        lease: Arc<dyn Send + Sync>,
    ) -> Result<()> {
        let entry = self
            .inner
            .active_playlist_entry
            .get()
            .ok_or_else(|| MediaError("Wait for playback before selecting captions".into()))?;
        {
            let snapshot = self.inner.snapshot.borrow();
            if snapshot.load_request_id == 0
                || snapshot.load_request_id != snapshot.active_load_request_id
                || !matches!(
                    snapshot.state,
                    PlaybackState::Playing | PlaybackState::Paused | PlaybackState::Seeking
                )
            {
                return Err(MediaError(
                    "Wait for the selected video before choosing captions".into(),
                ));
            }
        }
        if self.inner.subtitle_leases.borrow().len() >= 4 {
            return Err(MediaError("Caption command queue busy".into()));
        }
        if title.len() > 256 || language.len() > 64 {
            return Err(MediaError("Caption metadata exceeds its limit".into()));
        }
        let path = selected_path(path)?;
        if path.len() > 4096
            || (self.inner.caption_paths.borrow().len() >= 8
                && !self.inner.caption_paths.borrow().contains(path))
        {
            return Err(MediaError("Caption file limit reached".into()));
        }
        let mut registered: std::collections::BTreeSet<_> = self
            .inner
            .playback_subtitle_leases
            .borrow()
            .keys()
            .cloned()
            .collect();
        registered.extend(
            self.inner
                .subtitle_leases
                .borrow()
                .values()
                .map(|caption| (caption.entry, caption.path.clone())),
        );
        if registered.len() >= 8 && !registered.contains(&(entry, path.to_owned())) {
            return Err(MediaError("Caption playback lease limit reached".into()));
        }
        let reply = self.inner.next_subtitle_reply.get();
        let next = reply
            .checked_add(1)
            .filter(|value| *value < seek_confirmation::FIRST)
            .ok_or_else(|| MediaError("Caption command identifiers exhausted".into()))?;
        self.command_with_reply(&["sub-add", path, "cached", title, language], reply)?;
        self.inner.subtitle_leases.borrow_mut().insert(
            reply,
            SubtitleCommand {
                generation: self.inner.playback_generation.get(),
                entry,
                path: path.to_owned(),
                lease,
            },
        );
        self.inner
            .caption_paths
            .borrow_mut()
            .insert(path.to_owned());
        self.inner.next_subtitle_reply.set(next);
        self.inner.subtitles_off.set(false);
        Ok(())
    }
    /// Match only a registered local caption path against actual engine state.
    /// Paths remain private and never enter Snapshot or diagnostic formatting.
    pub fn subtitle_matches(&self, path: &std::path::Path) -> bool {
        self.inner.selected_caption.borrow().as_deref() == path.to_str()
            && self.inner.selected_caption.borrow().is_some()
    }
    pub fn disable_subtitles(&self) -> Result<()> {
        self.command(&["set", "sid", "no"])?;
        self.inner.subtitles_off.set(true);
        Ok(())
    }
    pub fn cycle_subtitles(&self) -> Result<()> {
        self.command(&["cycle", "sid"])
    }
    /// Revoke registered streams immediately and request a terminal native stop.
    /// One reserved command bypasses ordinary queue saturation; repeated calls
    /// coalesce. Older load commands are fenced by their replies followed by a
    /// final stop, because libmpv does not promise async command ordering.
    /// New loads remain rejected until a post-reply query confirms no current
    /// VO. Native idle events wake this check without polling. A native error
    /// keeps admission closed until explicit retry; caption leases retain their
    /// separate exact END_FILE boundary.
    pub fn stop(&self) -> Result<()> {
        self.inner.pause_intent.borrow_mut().stop();
        self.cancel_seek_confirmation();
        self.invalidate_audio_probe();
        self.inner.active_load_request.set(0);
        self.inner.clock_awaiting_seek_event.set(false);
        self.inner.frame_ready_load.set(0);
        self.inner.frame_probe.set(None);
        {
            let mut snapshot = self.inner.snapshot.borrow_mut();
            snapshot.paused_for_cache = false;
            snapshot.playback_restarted = false;
            snapshot.load_request_id = 0;
            snapshot.failed_load_request_id = None;
            snapshot.position = 0.;
            snapshot.duration = 0.;
        }
        self.inner.resume_request.set(None);
        self.inner.resume_reply.set(None);
        self.inner.caption_paths.borrow_mut().clear();
        self.inner.selected_caption.borrow_mut().take();
        self.inner
            .playback_generation
            .set(self.inner.playback_generation.get().saturating_add(1));
        self.inner.streams.clear();
        self.inner.latest_seek.set(None);
        self.inner.stop_requested.set(true);
        if self.inner.pending_loads.get() != 0 {
            self.inner.stop_barrier.set(true);
        }
        // One reserved asynchronous slot, independent of the ordinary 64 slots.
        // Repeated sign-out/clear/stop calls cannot grow the native command queue.
        if !self.inner.stop_inflight.get()
            && (!self.inner.stop_waiting_vo.get() || self.inner.stop_failed.get())
        {
            self.inner.stop_failed.set(false);
            self.inner.stop_waiting_vo.set(false);
            self.submit_stop()?;
        }
        Ok(())
    }
    fn submit_stop(&self) -> Result<()> {
        if let Err(error) = self.submit_command(&["stop"], 102) {
            self.inner.stop_failed.set(true);
            return Err(error);
        }
        self.inner.stop_inflight.set(true);
        Ok(())
    }
    fn advance_stop(&self) -> Result<()> {
        if self.inner.stop_requested.get()
            && !self.inner.stop_inflight.get()
            && !self.inner.stop_failed.get()
            && self.inner.pending_loads.get() == 0
        {
            if self.inner.stop_barrier.replace(false) {
                // client.h permits arbitrary async reordering. cmd_loadfile
                // completes after inserting its playlist entry; cmd_stop clears
                // that playlist. Dispatch the final stop only AFTER every older
                // load reply and the immediate stop reply, never by queue order.
                self.submit_stop()?;
            } else {
                self.inner.stop_waiting_vo.set(true);
                self.request_stop_probe()?;
            }
        }
        Ok(())
    }
    fn request_stop_probe(&self) -> Result<()> {
        if !self.inner.stop_waiting_vo.get() || self.inner.stop_failed.get() {
            return Ok(());
        }
        if self.inner.stop_probe_pending.get() {
            self.inner.stop_probe_again.set(true);
            return Ok(());
        }
        let result = unsafe {
            checked(
                ffi::mpv_get_property_async(self.inner.raw, 103, c"current-vo".as_ptr(), 1),
                "Verify stopped video output",
            )
        };
        if result.is_ok() {
            self.inner.stop_probe_pending.set(true);
        } else {
            self.inner.stop_failed.set(true);
        }
        result
    }
    /// Request a fresh position for quality/expiry replacement, including paused
    /// or hidden controls. One request may be pending. Results are tokened and
    /// invalidated by load/stop; no polling timer or synchronous query is used.
    pub fn request_resume_position(&self) -> Result<u64> {
        if self.inner.seek_pending.get()
            || self.inner.latest_seek.get().is_some()
            || self.inner.clock_awaiting_seek_event.get()
            || self.inner.seek_confirmation.borrow().token().is_some()
        {
            return Err(MediaError(
                "A seek is still pending; retry after playback settles".into(),
            ));
        }
        {
            let snapshot = self.inner.snapshot.borrow();
            if !self.inner.pause_intent.borrow().settled(snapshot.paused) {
                return Err(MediaError(
                    "Pause is still pending; retry after playback settles".into(),
                ));
            }
            if matches!(
                snapshot.state,
                PlaybackState::Seeking | PlaybackState::Buffering
            ) {
                return Err(MediaError(
                    "Media is still settling; retry after playback is ready".into(),
                ));
            }
            if snapshot.load_request_id != 0
                && snapshot.load_request_id != snapshot.active_load_request_id
            {
                return Err(MediaError(
                    "Media is changing files; resume position is not yet available".into(),
                ));
            }
        }
        if self.inner.resume_request.get().is_some() {
            return Err(MediaError("Resume position request already pending".into()));
        }
        let token = self.inner.next_resume_request.get();
        let next = token
            .checked_add(1)
            .ok_or_else(|| MediaError("Resume position identifiers exhausted".into()))?;
        self.inner.resume_reply.set(None);
        self.inner
            .resume_request
            .set(Some((token, self.inner.playback_generation.get())));
        let code =
            unsafe { ffi::mpv_get_property_async(self.inner.raw, token, c"time-pos".as_ptr(), 5) };
        if code < 0 {
            self.inner.resume_request.set(None);
        }
        checked(code, "Request resume position")?;
        self.inner.next_resume_request.set(next);
        Ok(token)
    }
    /// Forget only this operation; late native replies cannot affect a newer one.
    pub fn cancel_resume_position(&self, token: u64) {
        if self
            .inner
            .resume_request
            .get()
            .is_some_and(|(id, _)| id == token)
        {
            self.inner.resume_request.set(None);
        }
        if self
            .inner
            .resume_reply
            .get()
            .is_some_and(|(id, _)| id == token)
        {
            self.inner.resume_reply.set(None);
        }
    }
    fn advance_clock_epoch(&self) {
        self.inner
            .clock_transport_epoch
            .set(self.inner.clock_transport_epoch.get().saturating_add(1));
    }
    fn invalidate_resume_position(&self) {
        self.advance_clock_epoch();
        self.invalidate_audio_probe();
        let token = self
            .inner
            .resume_request
            .take()
            .map(|(token, _)| token)
            .or_else(|| self.inner.resume_reply.get().map(|(token, _)| token));
        if let Some(token) = token {
            self.inner.resume_reply.set(Some((token, None)));
        }
    }
    /// Request exactly three fresh native audio diagnostics. One probe may be
    /// outstanding; there is no timer or implicit retry. Values are correlated
    /// to this playback generation and native entry, not cached observations.
    pub fn request_audio_probe(&self) -> Result<u64> {
        if !self.current_load_is_active()
            || self.inner.seek_pending.get()
            || self.inner.latest_seek.get().is_some()
            || self.inner.clock_awaiting_seek_event.get()
            || self.inner.seek_confirmation.borrow().token().is_some()
            || !self.inner.snapshot.borrow().playback_restarted
            || !matches!(
                self.inner.snapshot.borrow().state,
                PlaybackState::Playing | PlaybackState::Paused
            )
        {
            return Err(MediaError(
                "Wait for current playback before probing audio".into(),
            ));
        }
        let token = self
            .inner
            .audio_probes
            .borrow_mut()
            .begin(self.audio_probe_context())?;
        self.inner.audio_probe_reply.set(None);
        for (field, (name, format)) in [
            (c"audio-params/samplerate", 4),
            (c"audio-out-params/samplerate", 4),
            (c"audio-pts", 5),
        ]
        .into_iter()
        .enumerate()
        {
            let result = unsafe {
                checked(
                    ffi::mpv_get_property_async(
                        self.inner.raw,
                        token + field as u64,
                        name.as_ptr(),
                        format,
                    ),
                    "Read audio diagnostic",
                )
            };
            if let Err(error) = result {
                self.inner.audio_probes.borrow_mut().abort_submission();
                return Err(error);
            }
            self.inner.audio_probes.borrow_mut().submitted(field as u8);
        }
        Ok(token)
    }
    /// Invalidate only this token. Admission remains occupied until already
    /// submitted native replies drain, so cancellation cannot flood the queue.
    pub fn cancel_audio_probe(&self, token: u64) {
        self.inner.audio_probes.borrow_mut().cancel(token);
        if self
            .inner
            .audio_probe_reply
            .get()
            .is_some_and(|reply| reply.token == token)
        {
            self.inner.audio_probe_reply.set(None);
        }
    }
    fn invalidate_audio_probe(&self) {
        self.inner.audio_probes.borrow_mut().invalidate();
        self.inner.audio_probe_reply.set(None);
    }
    fn audio_probe_context(&self) -> audio_probe::Context {
        audio_probe::Context {
            generation: self.inner.playback_generation.get(),
            load_request_id: self.inner.active_load_request.get(),
            entry: self.inner.active_playlist_entry.get(),
        }
    }
    /// Called at ~4 Hz by the host only while progress is visible and playing.
    pub fn request_progress(&self) -> Result<()> {
        if self.inner.progress_pending.replace(true) {
            return Ok(());
        }
        let code =
            unsafe { ffi::mpv_get_property_async(self.inner.raw, 20, c"time-pos".as_ptr(), 5) };
        if code < 0 {
            self.inner.progress_pending.set(false);
        }
        checked(code, "Read progress")
    }
    pub fn frame_pending(&self) -> bool {
        let frame = self.inner.wake.frame.load(Ordering::Acquire);
        #[cfg(target_os = "macos")]
        let frame = frame
            && (!self.inner.wake.clock.enabled.load(Ordering::Acquire)
                || self.inner.wake.clock.ready.load(Ordering::Acquire));
        frame || self.inner.wake.due.load(Ordering::Acquire)
    }
    pub fn snapshot(&self) -> Snapshot {
        let mut snapshot = self.inner.snapshot.borrow().clone();
        snapshot.stop_pending = self.inner.stop_requested.get();
        snapshot.resume_position_reply = self.inner.resume_reply.get();
        snapshot.audio_probe_reply = self.inner.audio_probe_reply.get();
        snapshot
    }
    /// Cheap admission check for UI texture publication, without cloning the
    /// diagnostic strings once per frame. This is load identity, not frame age.
    pub fn current_load_is_active(&self) -> bool {
        let snapshot = self.inner.snapshot.borrow();
        !self.inner.stop_requested.get()
            && snapshot.load_request_id != 0
            && snapshot.load_request_id == snapshot.active_load_request_id
    }
    /// Cheap, UI-thread-only identity for deferred progress display. No strings
    /// or diagnostic snapshot are cloned. Saturation disables staging instead
    /// of allowing an epoch to wrap and accept a stale value.
    pub fn clock_identity(&self) -> Option<ClockIdentity> {
        let epoch = self.inner.clock_transport_epoch.get();
        if epoch == u64::MAX
            || self.inner.seek_pending.get()
            || self.inner.latest_seek.get().is_some()
            || self.inner.clock_awaiting_seek_event.get()
            || self.inner.seek_confirmation.borrow().token().is_some()
            || !self.current_load_is_active()
            || !self.inner.snapshot.borrow().playback_restarted
        {
            return None;
        }
        Some(ClockIdentity {
            load_request_id: self.inner.active_load_request.get(),
            transport_epoch: epoch,
        })
    }
    /// Restart and decoded-video admission for ordinary video presentation.
    /// This is not a universal privacy proof (restart can also mean video EOF).
    /// Account-boundary transitions MUST await stop_pending=false, which proves
    /// destruction of the old native VO, before accepting another load.
    pub fn current_load_frame_ready(&self) -> bool {
        self.current_load_is_active()
            && self.inner.frame_ready_load.get() == self.inner.active_load_request.get()
    }
    fn request_frame_probe(&self) -> Result<()> {
        let request = self.inner.active_load_request.get();
        let Some(entry) = self.inner.active_playlist_entry.get() else {
            return Ok(());
        };
        if request == 0
            || self.inner.stop_requested.get()
            || self.inner.restarted_entry.get() != Some(entry)
            || self.inner.load_entries.borrow().get(&entry) != Some(&request)
            || self.inner.frame_probe.get().is_some()
            || self.inner.frame_ready_load.get() == request
        {
            return Ok(());
        }
        // This property has no container-dimension fallback, unlike width or
        // video-params/w; audio-only playback has no video decoder/VO chain.
        unsafe {
            checked(
                ffi::mpv_get_property_async(
                    self.inner.raw,
                    request,
                    c"video-dec-params/w".as_ptr(),
                    4,
                ),
                "Verify first video frame",
            )?;
        }
        self.inner.frame_probe.set(Some((request, entry)));
        Ok(())
    }
    /// Drain events after each coalesced wake. No synchronous mpv property reads.
    pub fn drain_events(&self) -> Snapshot {
        self.inner.wake.queued.store(false, Ordering::Release);
        let mut snapshot = self.inner.snapshot.borrow_mut();
        let mut pause_failed = false;
        loop {
            // timeout=0 is expressly safe on the mpv rendering thread.
            let event = unsafe { &*ffi::mpv_wait_event(self.inner.raw, 0.) };
            if event.id == 0 {
                break;
            }
            snapshot.events_received += 1;
            match event.id {
                3 | 22 => {
                    if event.id == 3 && event.userdata == 104 {
                        let value = if event.error >= 0 && !event.data.is_null() {
                            let property = unsafe { &*event.data.cast::<ffi::Property>() };
                            (property.format == 3 && !property.data.is_null())
                                .then(|| unsafe { *property.data.cast::<i32>() != 0 })
                        } else {
                            None
                        };
                        let result = self.inner.pause_intent.borrow_mut().eof_reply(value);
                        match result {
                            Ok(Some(true)) => {
                                snapshot.state = PlaybackState::Ended;
                                self.invalidate_resume_position();
                            }
                            Err(error) => {
                                snapshot.error = Some(error.to_string());
                                pause_failed = true;
                            }
                            _ => {}
                        }
                        continue;
                    }
                    if event.id == 3 && audio_probe::owns_userdata(event.userdata) {
                        let value = if event.error >= 0 && !event.data.is_null() {
                            let property = unsafe { &*event.data.cast::<ffi::Property>() };
                            if property.data.is_null() {
                                audio_probe::Value::Unavailable
                            } else {
                                match property.format {
                                    4 => audio_probe::Value::Integer(unsafe {
                                        *property.data.cast::<i64>()
                                    }),
                                    5 => audio_probe::Value::Float(unsafe {
                                        *property.data.cast::<f64>()
                                    }),
                                    _ => audio_probe::Value::Unavailable,
                                }
                            }
                        } else {
                            audio_probe::Value::Unavailable
                        };
                        if let Some(reply) = self.inner.audio_probes.borrow_mut().receive(
                            event.userdata,
                            value,
                            self.audio_probe_context(),
                        ) {
                            self.inner.audio_probe_reply.set(Some(reply));
                        }
                        continue;
                    }
                    if event.id == 3 && event.userdata == 103 {
                        self.inner.stop_probe_pending.set(false);
                        if self.inner.stop_waiting_vo.get() {
                            if event.error == -10 {
                                // MPV_ERROR_PROPERTY_UNAVAILABLE
                                self.inner.stop_requested.set(false);
                                self.inner.stop_waiting_vo.set(false);
                                self.inner.stop_probe_again.set(false);
                            } else if event.error < 0 {
                                self.inner.stop_failed.set(true);
                                snapshot.error = Some(format!(
                                    "Stopped video output verification failed ({})",
                                    event.error
                                ));
                            } else if self.inner.stop_probe_again.replace(false)
                                && let Err(error) = self.request_stop_probe()
                            {
                                snapshot.error = Some(error.to_string());
                            }
                        }
                        continue;
                    }
                    if event.id == 3 && ((1 << 62)..(1 << 63)).contains(&event.userdata) {
                        if let Some((request, entry)) = self.inner.frame_probe.get()
                            && request == event.userdata
                        {
                            self.inner.frame_probe.set(None);
                            let valid_width = event.error >= 0
                                && !event.data.is_null()
                                && unsafe {
                                    let property = &*event.data.cast::<ffi::Property>();
                                    property.format == 4
                                        && !property.data.is_null()
                                        && *property.data.cast::<i64>() > 0
                                };
                            if valid_width
                                && request == self.inner.active_load_request.get()
                                && !self.inner.stop_requested.get()
                                && self.inner.active_playlist_entry.get() == Some(entry)
                                && self.inner.restarted_entry.get() == Some(entry)
                            {
                                self.inner.frame_ready_load.set(request);
                                self.inner.wake.due.store(true, Ordering::Release);
                            }
                        }
                        continue;
                    }
                    if event.id == 3 && event.userdata >= (1 << 63) {
                        if let Some((token, generation)) = self.inner.resume_request.get()
                            && token == event.userdata
                        {
                            self.inner.resume_request.set(None);
                            if generation == self.inner.playback_generation.get() {
                                let position = if event.error >= 0 && !event.data.is_null() {
                                    let property = unsafe { &*event.data.cast::<ffi::Property>() };
                                    if property.format == 5 && !property.data.is_null() {
                                        let value = unsafe { *property.data.cast::<f64>() };
                                        (value.is_finite() && value >= 0.).then_some(value)
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                };
                                self.inner.resume_reply.set(Some((token, position)));
                            }
                        }
                        continue;
                    }
                    if event.userdata == 17 {
                        self.inner.selected_caption.borrow_mut().take();
                        if !event.data.is_null() {
                            let property = unsafe { &*event.data.cast::<ffi::Property>() };
                            if property.format == 1 && !property.data.is_null() {
                                let pointer =
                                    unsafe { *property.data.cast::<*const std::ffi::c_char>() };
                                if !pointer.is_null() {
                                    let bytes = unsafe { CStr::from_ptr(pointer) }.to_bytes();
                                    if bytes.len() <= 4096
                                        && let Ok(path) = std::str::from_utf8(bytes)
                                        && self.inner.caption_paths.borrow().contains(path)
                                    {
                                        *self.inner.selected_caption.borrow_mut() =
                                            Some(path.to_owned());
                                    }
                                }
                            }
                        }
                        snapshot.subtitle_updates = snapshot.subtitle_updates.saturating_add(1);
                    }
                    if event.userdata == 20 {
                        self.inner.progress_pending.set(false);
                        if self.inner.progress_refresh_pending.replace(false)
                            && let Err(error) = self.request_progress()
                        {
                            snapshot.error = Some(error.to_string());
                        }
                    }
                    if !event.data.is_null() {
                        unsafe {
                            update_property(
                                &mut snapshot,
                                event.userdata,
                                &*event.data.cast::<ffi::Property>(),
                            );
                        }
                        if event.userdata == 1 {
                            let property = unsafe { &*event.data.cast::<ffi::Property>() };
                            if property.format == 3 && !property.data.is_null() {
                                self.inner
                                    .pause_intent
                                    .borrow_mut()
                                    .observe(snapshot.paused);
                            }
                        }
                        if event.userdata == 11 {
                            let property = unsafe { &*event.data.cast::<ffi::Property>() };
                            if property.format == 3
                                && !property.data.is_null()
                                && !self.inner.stop_requested.get()
                                && snapshot.load_request_id != 0
                            {
                                // The notification may precede its load reply.
                                // Keep a generation-tagged candidate; the fresh
                                // query waits for exact native entry admission.
                                let eof = unsafe { *property.data.cast::<i32>() != 0 };
                                self.inner.pause_intent.borrow_mut().eof(eof);
                            }
                        }
                    }
                }
                5 => {
                    if event.userdata == 102 {
                        self.inner.stop_inflight.set(false);
                        if event.error < 0 {
                            self.inner.stop_failed.set(true);
                            snapshot.error =
                                Some(format!("Stop playback failed ({})", event.error));
                        }
                        if let Err(error) = self.advance_stop() {
                            snapshot.error = Some(error.to_string());
                        }
                        continue;
                    }
                    if pause_intent::owns(event.userdata) {
                        self.inner
                            .pending_commands
                            .set(self.inner.pending_commands.get().saturating_sub(1));
                        if self
                            .inner
                            .pause_intent
                            .borrow_mut()
                            .reply(event.userdata, event.error >= 0)
                        {
                            snapshot.error = Some(format!(
                                "Pause intent could not be applied ({}); stopping playback",
                                event.error,
                            ));
                            pause_failed = true;
                        }
                        continue;
                    }
                    // sub-add may run on mpv's worker; only its reply or complete
                    // engine destruction ends the callback's file lease.
                    let caption = self
                        .inner
                        .subtitle_leases
                        .borrow_mut()
                        .remove(&event.userdata);
                    if event.error >= 0
                        && let Some(caption) = &caption
                        && self.inner.active_playlist_entry.get() == Some(caption.entry)
                    {
                        // A successful sub-add may retain its demuxer for seeks.
                        // Exact END_FILE follows uninit_demuxer; an earlier stop
                        // command reply or idle-active property is not sufficient.
                        self.inner
                            .playback_subtitle_leases
                            .borrow_mut()
                            .insert((caption.entry, caption.path.clone()), caption.lease.clone());
                    }
                    self.inner
                        .pending_commands
                        .set(self.inner.pending_commands.get().saturating_sub(1));
                    let seek_reply = seek_confirmation::owns(event.userdata);
                    let seek_current = if seek_reply {
                        let decision = self.inner.seek_confirmation.borrow_mut().reply(
                            event.userdata,
                            event.error >= 0,
                            self.seek_context(),
                        );
                        if let Err(error) = self.finish_seek_confirmation(decision) {
                            snapshot.error = Some(error.to_string());
                        }
                        decision != seek_confirmation::Decision::Stale
                    } else {
                        true
                    };
                    // A slow sub-add can finish after the user's Off command.
                    // Reapply the latest Off intent once its last pending add
                    // completes; this is event-driven, never a retry timer.
                    if caption.is_some()
                        && self.inner.subtitles_off.get()
                        && self.inner.subtitle_leases.borrow().is_empty()
                        && let Err(error) = self.command(&["set", "sid", "no"])
                    {
                        snapshot.error = Some(error.to_string());
                    }
                    let load_reply = ((1 << 62)..(1 << 63)).contains(&event.userdata);
                    if load_reply {
                        self.inner
                            .pending_loads
                            .set(self.inner.pending_loads.get().saturating_sub(1));
                        if let Err(error) = self.advance_stop() {
                            snapshot.error = Some(error.to_string());
                        }
                    }
                    let current_load = seek_current
                        && (!load_reply || event.userdata == self.inner.active_load_request.get());
                    if load_reply && event.error >= 0 {
                        if let Some(entry) = unsafe { command_playlist_entry(event.data) } {
                            let mut entries = self.inner.load_entries.borrow_mut();
                            if entries.len() >= 8
                                && let Some(old) = entries
                                    .keys()
                                    .copied()
                                    .find(|id| Some(*id) != self.inner.active_playlist_entry.get())
                            {
                                entries.remove(&old);
                            }
                            entries.insert(entry, event.userdata);
                            if self.inner.active_playlist_entry.get() == Some(entry) {
                                snapshot.active_load_request_id = event.userdata;
                                snapshot.playback_restarted = current_load
                                    && !self.inner.stop_requested.get()
                                    && self.inner.restarted_entry.get() == Some(entry);
                                recover_cache_phase(&mut snapshot);
                            }
                            drop(entries);
                            if let Err(error) = self.request_frame_probe() {
                                snapshot.error = Some(error.to_string());
                            }
                        } else if current_load {
                            snapshot.failed_load_request_id = Some(event.userdata);
                            snapshot.error =
                                Some("Media engine did not identify the queued file".into());
                        }
                    }
                    if load_reply && current_load && event.error < 0 {
                        snapshot.failed_load_request_id = Some(event.userdata);
                    }
                    if event.error < 0
                        && current_load
                        && caption.as_ref().is_none_or(|caption| {
                            caption.generation == self.inner.playback_generation.get()
                        })
                    {
                        snapshot.error =
                            Some(playback_error("Playback command failed", event.error));
                    }
                }
                6 => {
                    self.cancel_seek_confirmation();
                    let entry = if event.data.is_null() {
                        None
                    } else {
                        Some(unsafe { (*event.data.cast::<ffi::StartFile>()).playlist_entry_id })
                    };
                    self.inner.active_playlist_entry.set(entry);
                    self.invalidate_audio_probe();
                    self.inner.restarted_entry.set(None);
                    self.inner.frame_ready_load.set(0);
                    self.inner.frame_probe.set(None);
                    snapshot.active_load_request_id = entry
                        .and_then(|id| self.inner.load_entries.borrow().get(&id).copied())
                        .unwrap_or(0);
                    snapshot.paused_for_cache = false;
                    snapshot.playback_restarted = false;
                    snapshot.file_starts = snapshot.file_starts.saturating_add(1);
                    self.inner.selected_caption.borrow_mut().take();
                    snapshot.subtitle_id = None;
                    snapshot.subtitle_selection_observed = false;
                    snapshot.state = PlaybackState::Buffering;
                    snapshot.error = None;
                    snapshot.position = 0.;
                    snapshot.duration = 0.;
                }
                8 | 21 => {
                    if event.id == 8 {
                        snapshot.file_loads = snapshot.file_loads.saturating_add(1);
                    }
                    // mpv playloop.c publishes PLAYBACK_RESTART after updating
                    // playback_pts, including paused seeks. Query once even
                    // when the normal progress timer is stopped. If an earlier
                    // seek-time query is in flight, refresh after its reply.
                    if event.id == 21 {
                        self.inner
                            .restarted_entry
                            .set(self.inner.active_playlist_entry.get());
                        snapshot.playback_restarted = !self.inner.stop_requested.get()
                            && snapshot.load_request_id != 0
                            && snapshot.load_request_id == snapshot.active_load_request_id;
                        if snapshot.playback_restarted {
                            let decision = self
                                .inner
                                .seek_confirmation
                                .borrow_mut()
                                .restart(self.seek_context());
                            if let Err(error) = self.finish_seek_confirmation(decision) {
                                snapshot.error = Some(error.to_string());
                            }
                        }
                        if let Err(error) = self.request_frame_probe() {
                            snapshot.error = Some(error.to_string());
                        }
                        if self.inner.progress_pending.get() {
                            self.inner.progress_refresh_pending.set(true);
                        } else if let Err(error) = self.request_progress() {
                            snapshot.error = Some(error.to_string());
                        }
                    }
                    // FILE_LOADED alone does not finish startup buffering.
                    // Cache recovery is independent of PLAYBACK_RESTART.
                    if event.id == 21 {
                        snapshot.state = running_phase(&snapshot);
                    }
                }
                11 => {
                    if let Err(error) = self.request_stop_probe() {
                        snapshot.error = Some(error.to_string());
                    }
                }
                20 => {
                    self.inner
                        .seek_confirmation
                        .borrow_mut()
                        .seek(self.seek_context());
                    self.inner.clock_awaiting_seek_event.set(false);
                    self.advance_clock_epoch();
                    self.invalidate_audio_probe();
                    snapshot.playback_restarted = false;
                    self.inner.restarted_entry.set(None);
                    snapshot.state = PlaybackState::Seeking;
                }
                7 => {
                    if !event.data.is_null() {
                        let end = unsafe { &*event.data.cast::<ffi::EndFile>() };
                        self.inner
                            .playback_subtitle_leases
                            .borrow_mut()
                            .retain(|(entry, _), _| *entry != end.playlist_entry_id);
                        if self.inner.active_playlist_entry.get() != Some(end.playlist_entry_id) {
                            continue;
                        }
                        self.cancel_seek_confirmation();
                        self.inner.active_playlist_entry.set(None);
                        self.invalidate_audio_probe();
                        self.inner.restarted_entry.set(None);
                        self.inner.frame_ready_load.set(0);
                        self.inner.frame_probe.set(None);
                        self.inner
                            .load_entries
                            .borrow_mut()
                            .remove(&end.playlist_entry_id);
                        snapshot.paused_for_cache = false;
                        snapshot.playback_restarted = false;
                        snapshot.state = if end.reason == 4 {
                            snapshot.error = Some(playback_error("Playback failed", end.error));
                            PlaybackState::Failed
                        } else if end.reason == 0 {
                            PlaybackState::Ended
                        } else {
                            PlaybackState::Idle
                        };
                    }
                }
                24 => {
                    snapshot.error = Some("Media event queue overflow; reload playback".into());
                    snapshot.playback_restarted = false;
                    snapshot.state = PlaybackState::Failed;
                }
                _ => {}
            }
        }
        if let Err(error) = self.submit_pause_intent() {
            snapshot.error = Some(error.to_string());
            pause_failed = true;
        }
        if let Err(error) = self.submit_eof_probe() {
            snapshot.error = Some(error.to_string());
            pause_failed = true;
        }
        if pause_failed {
            // stop() owns snapshot mutation. Release this drain borrow before
            // taking its reserved command path; no callback reentrancy needed.
            drop(snapshot);
            if let Err(error) = self.stop() {
                self.inner.snapshot.borrow_mut().error = Some(error.to_string());
            }
            return self.snapshot();
        }
        #[cfg(target_os = "macos")]
        if let Some(clock) = self.inner.presentation_clock.borrow().as_ref()
            && let Err(error) =
                clock.service(!snapshot.paused && snapshot.state == PlaybackState::Playing)
        {
            snapshot.state = PlaybackState::Failed;
            snapshot.error = Some(error.to_string());
        }
        #[cfg(target_os = "macos")]
        {
            snapshot.display_clock_ticks = self.inner.wake.clock.ticks.load(Ordering::Relaxed);
            snapshot.display_clock_starts = self.inner.wake.clock.starts.load(Ordering::Relaxed);
            snapshot.display_clock_stops = self.inner.wake.clock.stops.load(Ordering::Relaxed);
            snapshot.display_clock_active = self.inner.wake.clock.running.load(Ordering::Acquire);
            snapshot.prevents_display_sleep = self
                .inner
                .presentation_clock
                .borrow()
                .as_ref()
                .is_some_and(|clock| clock.prevents_display_sleep());
        }
        snapshot.wakeups = self.inner.wake.count.load(Ordering::Relaxed);
        snapshot.render_notifications = self.inner.wake.render_count.load(Ordering::Relaxed);
        snapshot.resume_position_reply = self.inner.resume_reply.get();
        snapshot.audio_probe_reply = self.inner.audio_probe_reply.get();
        snapshot.stop_pending = self.inner.stop_requested.get();
        snapshot.clone()
    }
}
// mpv0.41 cmd_loadfile returns a bounded map containing playlist_entry_id.
// Borrow only while consuming COMMAND_REPLY; never retain provider/native nodes.
unsafe fn command_playlist_entry(data: *mut c_void) -> Option<i64> {
    if data.is_null() {
        return None;
    }
    let node = unsafe { &(*data.cast::<ffi::CommandReply>()).result };
    if node.format != 8 {
        return None;
    }
    let list = unsafe { node.data.list };
    if list.is_null() {
        return None;
    }
    let list = unsafe { &*list };
    if !(1..=16).contains(&list.num) || list.keys.is_null() || list.values.is_null() {
        return None;
    }
    for index in 0..list.num as usize {
        let key = unsafe { *list.keys.add(index) };
        if !key.is_null() && unsafe { CStr::from_ptr(key) }.to_bytes() == b"playlist_entry_id" {
            let value = unsafe { &*list.values.add(index) };
            if value.format == 4 {
                let id = unsafe { value.data.int64 };
                return (id >= 0).then_some(id);
            }
        }
    }
    None
}
// No credentials or arbitrary schemes. Provider owns host allowlisting.
fn valid_direct_url(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("https://") else {
        return false;
    };
    let authority = rest.split('/').next().unwrap_or_default();
    (authority == "googlevideo.com" || authority.ends_with(".googlevideo.com"))
        && authority
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        && !value.chars().any(char::is_control)
}
fn running_phase(s: &Snapshot) -> PlaybackState {
    if s.paused_for_cache {
        PlaybackState::Buffering
    } else if s.paused {
        PlaybackState::Paused
    } else {
        PlaybackState::Playing
    }
}
fn recover_cache_phase(s: &mut Snapshot) {
    if !s.paused_for_cache
        && s.state == PlaybackState::Buffering
        && s.playback_restarted
        && s.load_request_id != 0
        && s.load_request_id == s.active_load_request_id
        && s.failed_load_request_id != Some(s.load_request_id)
    {
        s.state = running_phase(s);
    }
}
unsafe fn update_property(s: &mut Snapshot, id: u64, p: &ffi::Property) {
    if p.format == 0 || p.data.is_null() {
        clear_unavailable_media_property(s, id);
        return;
    }
    match (id, p.format) {
        (1, 3) => {
            s.paused = unsafe { *p.data.cast::<i32>() != 0 };
            if matches!(s.state, PlaybackState::Playing | PlaybackState::Paused) {
                s.state = if s.paused {
                    PlaybackState::Paused
                } else {
                    PlaybackState::Playing
                };
            }
        }
        (2, 5) => s.duration = unsafe { *p.data.cast::<f64>() },
        (3 | 4 | 12 | 15, 1) => {
            let ptr = unsafe { *p.data.cast::<*const std::ffi::c_char>() };
            if !ptr.is_null() {
                let text = unsafe { CStr::from_ptr(ptr) }
                    .to_string_lossy()
                    .into_owned();
                match id {
                    3 => s.hwdec_current = text,
                    4 => s.codec = text,
                    12 => s.audio_codec = text,
                    _ => s.audio_output = text,
                }
            } else {
                clear_unavailable_media_property(s, id);
            }
        }
        (5, 4) => s.width = unsafe { *p.data.cast::<i64>() },
        (6, 4) => s.height = unsafe { *p.data.cast::<i64>() },
        (7, 5) => s.fps = unsafe { *p.data.cast::<f64>() },
        (8, 4) => s.dropped_frames = unsafe { *p.data.cast::<i64>() },
        (9, 5) => s.volume = unsafe { *p.data.cast::<f64>() },
        (19, 3) => {
            s.muted = unsafe { *p.data.cast::<i32>() != 0 };
            s.mute_observed = true;
        }
        (19, _) => clear_unavailable_media_property(s, id),
        (10, 3) => {
            s.paused_for_cache = unsafe { *p.data.cast::<i32>() } != 0;
            // mpv cache recovery updates this property without another restart
            // event. Preserve terminal/startup/seek phases when no current-load
            // restart has confirmed that playback is ready to continue.
            if s.paused_for_cache {
                if matches!(s.state, PlaybackState::Playing | PlaybackState::Paused) {
                    s.state = PlaybackState::Buffering;
                }
            } else {
                recover_cache_phase(s);
            }
        }
        (11, 3) => {
            if unsafe { *p.data.cast::<i32>() } != 0 {
                s.state = PlaybackState::Ended;
            }
        }
        (13, 4) => s.audio_sample_rate = unsafe { *p.data.cast::<i64>() },
        (14, 4) => s.decoder_dropped_frames = unsafe { *p.data.cast::<i64>() },
        (18, 5) => {
            let speed = unsafe { *p.data.cast::<f64>() };
            s.speed_observed = speed.is_finite() && (0.25..=4.).contains(&speed);
            if s.speed_observed {
                s.speed = speed;
            }
            s.speed_updates = s.speed_updates.saturating_add(1);
        }
        (16, 1) => {
            let ptr = unsafe { *p.data.cast::<*const std::ffi::c_char>() };
            if !ptr.is_null() {
                let text = unsafe { CStr::from_ptr(ptr) }.to_bytes();
                s.subtitle_id = std::str::from_utf8(text)
                    .ok()
                    .and_then(|s| s.parse::<i64>().ok())
                    .filter(|id| *id > 0);
                s.subtitle_selection_observed = true;
                s.subtitle_updates = s.subtitle_updates.saturating_add(1);
            }
        }
        (20, 5) => s.position = unsafe { *p.data.cast::<f64>() },
        _ => {}
    }
}
// An unavailable observed property is new information, not permission to keep
// the previous file's decoder/format diagnostics. Leave unrelated state intact.
fn clear_unavailable_media_property(s: &mut Snapshot, id: u64) {
    match id {
        2 => s.duration = 0.,
        3 => s.hwdec_current.clear(),
        4 => s.codec.clear(),
        5 => s.width = 0,
        6 => s.height = 0,
        7 => s.fps = 0.,
        12 => s.audio_codec.clear(),
        13 => s.audio_sample_rate = 0,
        15 => s.audio_output.clear(),
        18 => {
            s.speed_observed = false;
            s.speed_updates = s.speed_updates.saturating_add(1);
        }
        19 => {
            s.muted = false;
            s.mute_observed = false;
        }
        _ => {}
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn playback_failure_preserves_engine_category_without_inventing_network_cause() {
        let unavailable = super::playback_error("Playback failed", -13);
        assert!(unavailable.contains("file or stream could not be opened"));
        assert!(!unavailable.contains("expired"));
        assert!(!unavailable.contains("authentication"));
        let audio = super::playback_error("Playback failed", -14);
        let video = super::playback_error("Playback failed", -15);
        assert!(audio.contains("Audio output could not start"));
        assert!(video.contains("Video output could not start"));
        let unknown = super::playback_error("Playback failed", -999);
        assert!(unknown.contains("unspecified error"));
        assert!(unknown.contains("-999"));
    }

    use super::*;
    #[test]
    fn mute_observation_distinguishes_false_true_and_unavailable_without_changing_volume() {
        let mut snapshot = Snapshot {
            volume: 37.,
            ..Snapshot::default()
        };
        for muted in [0i32, 1, 0, 1] {
            let mut flag = muted;
            let property = ffi::Property {
                name: std::ptr::null(),
                format: 3,
                data: std::ptr::from_mut(&mut flag).cast(),
            };
            unsafe { update_property(&mut snapshot, 19, &property) };
            assert!(snapshot.mute_observed);
            assert_eq!(snapshot.muted, muted != 0);
            assert_eq!(snapshot.volume, 37.);
        }
        for format in [0, 3, 5] {
            snapshot.muted = true;
            snapshot.mute_observed = true;
            let mut flag = 1i32;
            let property = ffi::Property {
                name: std::ptr::null(),
                format,
                data: if format == 3 {
                    std::ptr::null_mut()
                } else {
                    std::ptr::from_mut(&mut flag).cast()
                },
            };
            unsafe { update_property(&mut snapshot, 19, &property) };
            assert!(!snapshot.mute_observed);
            assert!(!snapshot.muted);
            assert_eq!(snapshot.volume, 37.);
        }
    }
    #[test]
    fn native_mute_toggle_preserves_volume_and_survives_a_file_load() {
        struct Fixture(std::path::PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let fixture = Fixture(std::env::temp_dir().join(format!(
                "serein-mute-{}-{}.wav",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )));
        std::fs::write(&fixture.0, silent_wav()).unwrap();
        let player = Player::new(|| {}).unwrap();
        player.command(&["set", "ao", "null"]).unwrap();
        player.command(&["set", "vo", "null"]).unwrap();
        player.set_volume(37.).unwrap();
        let wait = |wanted: bool, file_loaded: bool| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                let snapshot = player.drain_events();
                assert!(snapshot.error.is_none(), "{:?}", snapshot.error);
                if snapshot.mute_observed
                    && snapshot.muted == wanted
                    && (snapshot.volume - 37.).abs() < 0.001
                    && (!file_loaded
                        || snapshot.file_loads == 1
                            && snapshot.paused
                            && snapshot.audio_output == "null")
                {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "native mute observation timed out"
                );
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        };
        wait(false, false);
        player.toggle_mute().unwrap();
        wait(true, false);
        player.load_local_at(&fixture.0, 0., true).unwrap();
        wait(true, true);
        player.toggle_mute().unwrap();
        wait(false, true);
        assert_eq!(player.snapshot().file_loads, 1);
    }
    #[test]
    fn native_relative_command_failure_and_watchdog_release_ownership() {
        let player = Player::new(|| {}).unwrap();
        // Exercise a real asynchronous native failure: valid seek syntax on an
        // idle engine. Public admission correctly disallows this operation.
        let token = player
            .inner
            .seek_confirmation
            .borrow_mut()
            .begin(player.seek_context())
            .unwrap();
        player
            .command_with_reply(&["seek", "1", "relative"], token)
            .unwrap();
        player.inner.clock_awaiting_seek_event.set(true);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while player.inner.seek_confirmation.borrow().token().is_some() {
            player.drain_events();
            assert!(
                std::time::Instant::now() < deadline,
                "failed relative command was not reaped"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!player.inner.clock_awaiting_seek_event.get());
        assert!(player.snapshot().error.is_some());
        // Trigger the same handler a single-shot timeout uses, without adding a
        // GUI backend just for a timer test. It must take the reserved stop path.
        let expired = player
            .inner
            .seek_confirmation
            .borrow_mut()
            .begin(player.seek_context())
            .unwrap();
        player.inner.clock_awaiting_seek_event.set(true);
        player.seek_timeout(expired);
        assert!(player.snapshot().stop_pending);
        assert!(
            player
                .snapshot()
                .error
                .as_deref()
                .is_some_and(|e| e.contains("did not settle"))
        );
        assert_eq!(player.inner.seek_confirmation.borrow().token(), None);
        assert!(!player.inner.clock_awaiting_seek_event.get());
        let timeout_error = player.snapshot().error;
        drain_until_stopped_with_error(&player, timeout_error.as_deref());
        assert!(!player.snapshot().stop_pending);
        player.seek_timeout(expired); // stale timeout is a no-op
        assert!(!player.snapshot().stop_pending);
        assert_eq!(player.snapshot().error, timeout_error);
    }
    #[test]
    fn speed_observation_requires_finite_supported_native_values_and_counts_invalidations() {
        let mut snapshot = Snapshot::default();
        for (rate, valid) in [(1.5, true), (f64::NAN, false), (0., false), (2., true)] {
            let previous = snapshot.speed_updates;
            let mut rate = rate;
            let property = ffi::Property {
                name: std::ptr::null(),
                format: 5,
                data: std::ptr::from_mut(&mut rate).cast(),
            };
            unsafe { update_property(&mut snapshot, 18, &property) };
            assert_eq!(snapshot.speed_observed, valid);
            assert_eq!(snapshot.speed_updates, previous + 1);
            if valid {
                assert_eq!(snapshot.speed, rate);
            }
        }
        let previous = snapshot.speed_updates;
        clear_unavailable_media_property(&mut snapshot, 18);
        assert!(!snapshot.speed_observed);
        assert_eq!(snapshot.speed_updates, previous + 1);
    }
    fn cache_property(snapshot: &mut Snapshot, paused: bool) {
        let mut value = i32::from(paused);
        let property = ffi::Property {
            name: std::ptr::null(),
            format: 3,
            data: std::ptr::from_mut(&mut value).cast(),
        };
        unsafe { update_property(snapshot, 10, &property) };
    }
    #[test]
    fn cache_recovery_restores_current_running_phase_without_another_restart() {
        for paused in [false, true] {
            let mut snapshot = Snapshot {
                paused,
                playback_restarted: true,
                load_request_id: 7,
                active_load_request_id: 7,
                state: if paused {
                    PlaybackState::Paused
                } else {
                    PlaybackState::Playing
                },
                ..Snapshot::default()
            };
            cache_property(&mut snapshot, true);
            assert!(snapshot.paused_for_cache);
            assert_eq!(snapshot.state, PlaybackState::Buffering);
            cache_property(&mut snapshot, false);
            assert!(!snapshot.paused_for_cache);
            assert_eq!(
                snapshot.state,
                if paused {
                    PlaybackState::Paused
                } else {
                    PlaybackState::Playing
                }
            );
        }
    }
    #[test]
    fn late_load_identity_can_complete_already_observed_cache_recovery() {
        let mut snapshot = Snapshot {
            state: PlaybackState::Buffering,
            paused: false,
            load_request_id: 7,
            ..Snapshot::default()
        };
        cache_property(&mut snapshot, false);
        assert_eq!(snapshot.state, PlaybackState::Buffering);
        // A correlated loadfile command reply may follow START/RESTART events.
        snapshot.active_load_request_id = 7;
        snapshot.playback_restarted = true;
        recover_cache_phase(&mut snapshot);
        assert_eq!(snapshot.state, PlaybackState::Playing);
    }
    #[test]
    fn cache_updates_cannot_resurrect_terminal_startup_seeking_or_stale_loads() {
        for phase in [
            PlaybackState::Idle,
            PlaybackState::Ended,
            PlaybackState::Failed,
            PlaybackState::Seeking,
        ] {
            let mut snapshot = Snapshot {
                state: phase,
                playback_restarted: true,
                load_request_id: 7,
                active_load_request_id: 7,
                ..Snapshot::default()
            };
            cache_property(&mut snapshot, true);
            cache_property(&mut snapshot, false);
            assert_eq!(snapshot.state, phase);
        }
        for (restarted, request, active, failed) in [
            (false, 7, 7, None),
            (true, 0, 0, None),
            (true, 8, 7, None),
            (true, 7, 7, Some(7)),
        ] {
            let mut snapshot = Snapshot {
                state: PlaybackState::Buffering,
                playback_restarted: restarted,
                load_request_id: request,
                active_load_request_id: active,
                failed_load_request_id: failed,
                ..Snapshot::default()
            };
            cache_property(&mut snapshot, true);
            cache_property(&mut snapshot, false);
            assert_eq!(snapshot.state, PlaybackState::Buffering);
            assert!(!snapshot.paused_for_cache);
        }
    }
    #[test]
    fn unavailable_properties_clear_only_the_corresponding_media_metadata() {
        let mut null_string: *const std::ffi::c_char = std::ptr::null();
        for (id, format) in [
            (2, 5),
            (3, 1),
            (4, 1),
            (5, 4),
            (6, 4),
            (7, 5),
            (12, 1),
            (13, 4),
            (15, 1),
        ] {
            let mut cases = vec![
                ffi::Property {
                    name: std::ptr::null(),
                    format: 0,
                    data: std::ptr::from_mut(&mut null_string).cast(),
                },
                ffi::Property {
                    name: std::ptr::null(),
                    format,
                    data: std::ptr::null_mut(),
                },
            ];
            if format == 1 {
                cases.push(ffi::Property {
                    name: std::ptr::null(),
                    format,
                    data: std::ptr::from_mut(&mut null_string).cast(),
                });
            }
            for property in cases {
                let mut snapshot = Snapshot {
                    duration: 90.,
                    hwdec_current: "videotoolbox".into(),
                    codec: "H264".into(),
                    width: 1920,
                    height: 1080,
                    fps: 60.,
                    audio_codec: "aac".into(),
                    audio_sample_rate: 48_000,
                    audio_output: "avfoundation".into(),
                    volume: 37.,
                    paused: true,
                    ..Snapshot::default()
                };
                unsafe {
                    update_property(&mut snapshot, id, &property);
                }
                assert_eq!(snapshot.duration == 0., id == 2);
                assert_eq!(snapshot.hwdec_current.is_empty(), id == 3);
                assert_eq!(snapshot.codec.is_empty(), id == 4);
                assert_eq!(snapshot.width == 0, id == 5);
                assert_eq!(snapshot.height == 0, id == 6);
                assert_eq!(snapshot.fps == 0., id == 7);
                assert_eq!(snapshot.audio_codec.is_empty(), id == 12);
                assert_eq!(snapshot.audio_sample_rate == 0, id == 13);
                assert_eq!(snapshot.audio_output.is_empty(), id == 15);
                assert_eq!(snapshot.volume, 37.);
                assert!(snapshot.paused);
            }
        }
    }
    fn silent_wav() -> Vec<u8> {
        let mut wav = b"RIFF".to_vec();
        wav.extend(96_036u32.to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16u32.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(48_000u32.to_le_bytes());
        wav.extend(96_000u32.to_le_bytes());
        wav.extend(2u16.to_le_bytes());
        wav.extend(16u16.to_le_bytes());
        wav.extend(b"data");
        wav.extend(96_000u32.to_le_bytes());
        wav.resize(96_044, 0);
        wav
    }
    fn drain_until_stopped(player: &Player) {
        drain_until_stopped_with_error(player, None);
    }
    fn drain_until_stopped_with_error(player: &Player, expected_error: Option<&str>) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let snapshot = player.drain_events();
            assert_eq!(snapshot.error.as_deref(), expected_error);
            if !snapshot.stop_pending && player.inner.pending_commands.get() == 0 {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "stop reply timed out");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(player.inner.pending_loads.get(), 0);
        assert!(!player.inner.stop_inflight.get());
        assert!(!player.inner.stop_barrier.get());
    }
    #[test]
    fn reserved_stop_survives_saturated_queue_and_coalesces_until_reply() {
        let player = Player::new(|| {}).unwrap();
        for _ in 0..64 {
            player.set_volume(37.).unwrap();
        }
        assert!(player.set_volume(38.).is_err());
        for _ in 0..1000 {
            player
                .stop()
                .expect("reserved stop bypasses ordinary admission");
        }
        assert!(player.snapshot().stop_pending);
        assert!(player.inner.stop_inflight.get());
        assert_eq!(player.inner.pending_commands.get(), 64);
        assert!(
            player
                .load_local_at(std::path::Path::new("/unused.wav"), 0., true)
                .is_err()
        );
        drain_until_stopped(&player);
        player.set_volume(38.).unwrap();
    }
    #[test]
    fn native_pause_intent_survives_occlusion_before_observation_and_hidden_reload() {
        struct Fixture(std::path::PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let fixture = Fixture(std::env::temp_dir().join(format!(
            "serein-pause-intent-{}-{}.wav", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
        )));
        std::fs::write(&fixture.0, silent_wav()).unwrap();
        let player = Player::new(|| {}).unwrap();
        player.command(&["set", "ao", "null"]).unwrap();
        player.command(&["set", "vo", "null"]).unwrap();
        let settle = |paused, loads| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                let snapshot = player.drain_events();
                assert!(snapshot.error.is_none(), "{:?}", snapshot.error);
                if snapshot.paused == paused
                    && snapshot.file_loads == loads
                    && player.inner.pending_commands.get() == 0
                {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "pause intent did not settle"
                );
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        };
        player.load_local_at(&fixture.0, 0., true).unwrap();
        settle(true, 1);
        // User resume/pause are both submitted before any event drain, then
        // fullscreen-style repeated hide/show must preserve the newest pause.
        player.toggle_pause().unwrap();
        player.toggle_pause().unwrap();
        player.set_occluded(true).unwrap();
        player.set_occluded(true).unwrap();
        player.set_occluded(false).unwrap();
        player.set_occluded(false).unwrap();
        settle(true, 1);
        player.set_occluded(true).unwrap();
        player.toggle_pause().unwrap(); // user intends playing, still hidden
        settle(true, 1);
        assert!(!player.inner.pause_intent.borrow().desired());
        player.stop().unwrap();
        drain_until_stopped(&player);
        assert!(player.inner.pause_intent.borrow().desired());
        player.load_local_at(&fixture.0, 0., false).unwrap();
        settle(true, 2);
        player.set_occluded(false).unwrap();
        settle(false, 2);
        player.stop().unwrap();
        drain_until_stopped(&player);
    }

    #[test]
    fn native_correlated_pause_failure_stops_without_retrying() {
        let player = Player::new(|| {}).unwrap();
        player.inner.pause_intent.borrow_mut().user(true);
        let command = player
            .inner
            .pause_intent
            .borrow()
            .prepare()
            .unwrap()
            .unwrap();
        player
            .command_with_reply(&["set", "pause", "invalid-value"], command.token)
            .unwrap();
        player.inner.pause_intent.borrow_mut().submitted(command);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let snapshot = player.drain_events();
            if snapshot
                .error
                .as_deref()
                .is_some_and(|error| error.starts_with("Pause intent could not be applied"))
                && !snapshot.stop_pending
                && player.inner.pending_commands.get() == 0
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "pause failure did not terminate"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            player
                .inner
                .pause_intent
                .borrow()
                .prepare()
                .unwrap()
                .is_none()
        );
        player.set_occluded(true).unwrap();
        player.set_occluded(false).unwrap();
        assert_eq!(player.inner.pending_commands.get(), 0);
    }
    #[test]
    fn pause_intent_waits_for_bounded_command_admission_without_losing_latest_value() {
        let player = Player::new(|| {}).unwrap();
        for _ in 0..64 {
            player.set_volume(37.).unwrap();
        }
        player.set_paused(false).unwrap();
        player.set_paused(true).unwrap();
        player.set_occluded(true).unwrap();
        player.set_occluded(false).unwrap();
        assert_eq!(player.inner.pending_commands.get(), 64);
        assert!(player.inner.pause_intent.borrow().desired());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let snapshot = player.drain_events();
            assert!(snapshot.error.is_none(), "{:?}", snapshot.error);
            assert!(player.inner.pending_commands.get() <= 64);
            if snapshot.paused && player.inner.pending_commands.get() == 0 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "coalesced pause was not admitted"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    #[test]
    fn native_keep_open_eof_play_and_new_unpaused_file_do_not_reuse_stale_pause_cache() {
        struct Fixture(std::path::PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let fixture = Fixture(std::env::temp_dir().join(format!(
                "serein-pause-eof-{}-{}.wav",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )));
        std::fs::write(&fixture.0, silent_wav()).unwrap();
        let player = Player::new(|| {}).unwrap();
        player.command(&["set", "ao", "null"]).unwrap();
        player.command(&["set", "vo", "null"]).unwrap();
        let wait = |ended, loads| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                let snapshot = player.drain_events();
                assert!(snapshot.error.is_none(), "{:?}", snapshot.error);
                if snapshot.file_loads == loads
                    && player.inner.pending_commands.get() == 0
                    && if ended {
                        snapshot.state == PlaybackState::Ended
                            && snapshot.paused
                            && player.inner.pause_intent.borrow().settled(snapshot.paused)
                    } else {
                        snapshot.state == PlaybackState::Playing && !snapshot.paused
                    }
                {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "EOF/play transition did not settle: {snapshot:?}"
                );
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        };
        player.load_local_at(&fixture.0, 0., false).unwrap();
        // Let the native core finish this one-second file before the first
        // drain. EOF notification/load and pause replies may already be queued.
        std::thread::sleep(std::time::Duration::from_millis(1500));
        wait(true, 1);
        assert!(player.user_pause_intent());
        player.set_occluded(true).unwrap();
        player.set_occluded(false).unwrap();
        wait(true, 1);
        player.toggle_pause().unwrap();
        player.toggle_pause().unwrap();
        assert!(player.user_pause_intent());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let snapshot = player.drain_events();
            assert!(snapshot.error.is_none(), "{:?}", snapshot.error);
            if snapshot.paused
                && snapshot.state == PlaybackState::Paused
                && player.inner.pending_commands.get() == 0
                && player.inner.seek_confirmation.borrow().token().is_none()
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "rapid EOF double toggle lost pause intent"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        player.toggle_pause().unwrap();
        assert!(!player.user_pause_intent());
        wait(false, 1);
        wait(true, 1);
        assert!(player.user_pause_intent());
        player.toggle_pause().unwrap();
        wait(false, 1);
        wait(true, 1);
        player.load_local_at(&fixture.0, 0., false).unwrap();
        wait(false, 2);
        assert!(!player.user_pause_intent());
        player.stop().unwrap();
        drain_until_stopped(&player);
    }
    #[test]
    fn stop_barrier_clears_older_queued_loads_before_accepting_another_load() {
        struct Fixture(std::path::PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let fixture = Fixture(std::env::temp_dir().join(format!(
            "serein-stop-{}-{}.wav", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        )));
        std::fs::write(&fixture.0, silent_wav()).unwrap();
        let player = Player::new(|| {}).unwrap();
        player.command(&["set", "ao", "null"]).unwrap();
        player.command(&["set", "vo", "null"]).unwrap();
        player.load_local_at(&fixture.0, 0., true).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let snapshot = player.drain_events();
            assert!(snapshot.error.is_none(), "{:?}", snapshot.error);
            if snapshot.file_loads == 1 && player.inner.pending_commands.get() == 0 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "initial audio load timed out"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(player.inner.active_playlist_entry.get().is_some());
        assert!(player.current_load_is_active());
        assert!(
            !player.current_load_frame_ready(),
            "audio must never publish video"
        );
        player.load_local_at(&fixture.0, 0., true).unwrap();
        player.load_local_at(&fixture.0, 0., true).unwrap();
        assert_eq!(player.inner.pending_loads.get(), 2);
        while player.inner.pending_commands.get() < 64 {
            player.set_volume(0.).unwrap();
        }
        player.stop().unwrap();
        assert!(!player.current_load_is_active());
        assert!(!player.current_load_frame_ready());
        assert!(player.inner.stop_barrier.get());
        assert!(player.load_local_at(&fixture.0, 0., true).is_err());
        drain_until_stopped(&player);
        assert!(!player.current_load_is_active());
        unsafe extern "C" {
            fn mpv_get_property(
                raw: *mut ffi::Handle,
                name: *const std::ffi::c_char,
                format: i32,
                value: *mut std::ffi::c_void,
            ) -> i32;
        }
        let mut count = -1i64;
        assert!(
            unsafe {
                mpv_get_property(
                    player.inner.raw,
                    c"playlist-count".as_ptr(),
                    4,
                    std::ptr::from_mut(&mut count).cast(),
                )
            } >= 0
        );
        assert_eq!(
            count, 0,
            "final stop must clear all earlier loadfile entries"
        );
        // Subsequent explicit intent is admitted after the actual final reply.
        player.load_local_at(&fixture.0, 0., true).unwrap();
        player.stop().unwrap();
        drain_until_stopped(&player);
    }
    #[test]
    fn paused_video_requires_correlated_restart_and_failed_replacement_stays_closed() {
        struct Fixture(std::path::PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
                let _ = std::fs::remove_file(self.0.with_extension("wav"));
            }
        }
        let fixture = Fixture(std::env::temp_dir().join(format!(
                "serein-frame-fence-{}-{}.y4m",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )));
        let mut image = b"YUV4MPEG2 W16 H16 F1:1 Ip A1:1 C420jpeg\nFRAME\n".to_vec();
        image.extend([100u8; 256]);
        image.extend([128u8; 128]);
        std::fs::write(&fixture.0, image).unwrap();
        let player = Player::new(|| {}).unwrap();
        player.command(&["set", "vo", "null"]).unwrap();
        player.command(&["set", "ao", "null"]).unwrap();
        player.load_local_at(&fixture.0, 0., true).unwrap();
        assert!(!player.current_load_frame_ready());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !player.current_load_frame_ready() {
            let snapshot = player.drain_events();
            assert!(snapshot.error.is_none(), "{:?}", snapshot.error);
            assert!(
                std::time::Instant::now() < deadline,
                "paused video fence timed out"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!player.snapshot().codec.is_empty());
        assert_eq!(player.snapshot().width, 16);
        assert_eq!(player.snapshot().height, 16);
        let first_clock = player
            .clock_identity()
            .expect("observed paused video identity");
        player.set_volume(50.).unwrap();
        assert_eq!(player.clock_identity(), Some(first_clock));
        player.set_paused(true).unwrap();
        assert_ne!(player.clock_identity(), Some(first_clock));
        player.seek(0.).unwrap();
        assert!(
            player.seek_relative(1.).is_err(),
            "relative seek overlapped an unconfirmed absolute seek"
        );
        assert!(player.request_resume_position().is_err());
        assert!(player.request_audio_probe().is_err());
        player.seek(0.).unwrap(); // coalesced transport work also closes staging
        assert_eq!(player.clock_identity(), None);
        let seek_deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while player.clock_identity().is_none() {
            let snapshot = player.drain_events();
            assert!(snapshot.error.is_none(), "{:?}", snapshot.error);
            assert!(
                std::time::Instant::now() < seek_deadline,
                "clock seek did not settle"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let settled_clock = player.clock_identity().unwrap();
        assert_eq!(settled_clock.load_request_id, first_clock.load_request_id);
        assert!(settled_clock.transport_epoch > first_clock.transport_epoch);
        player.seek_relative(0.).unwrap();
        let relative_token = player.inner.seek_confirmation.borrow().token();
        assert!(
            player.seek_relative(1.).is_err(),
            "two relative seeks were admitted at once"
        );
        assert!(
            player.seek(0.).is_err(),
            "absolute seek superseded an unconfirmed relative seek"
        );
        assert_eq!(
            player.inner.seek_confirmation.borrow().token(),
            relative_token
        );
        assert_eq!(player.clock_identity(), None);
        assert!(
            player.request_audio_probe().is_err(),
            "relative seek admitted a pre-seek audio probe"
        );
        assert!(
            player.request_resume_position().is_err(),
            "relative seek admitted a pre-seek resume position"
        );
        let relative_deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while player.clock_identity().is_none() {
            let snapshot = player.drain_events();
            assert!(snapshot.error.is_none(), "{:?}", snapshot.error);
            assert!(
                std::time::Instant::now() < relative_deadline,
                "relative clock seek did not settle"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(player.clock_identity().unwrap().transport_epoch > settled_clock.transport_epoch);
        let audio = fixture.0.with_extension("wav");
        std::fs::write(&audio, silent_wav()).unwrap();
        player.load_local_at(&audio, 0., true).unwrap();
        assert_eq!(player.clock_identity(), None);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let snapshot = player.drain_events();
            assert!(snapshot.error.is_none(), "{:?}", snapshot.error);
            assert!(!player.current_load_frame_ready());
            if snapshot.file_loads == 2
                && !snapshot.audio_codec.is_empty()
                && snapshot.audio_sample_rate == 48_000
                && snapshot.codec.is_empty()
                && snapshot.width == 0
                && snapshot.height == 0
                && snapshot.fps == 0.
            {
                // mpv may report "no" for inactive hwdec rather than unavailable.
                assert!(snapshot.hwdec_current.is_empty() || snapshot.hwdec_current == "no");
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "audio-only metadata did not replace video metadata"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let audio_token = player.request_audio_probe().unwrap();
        assert!(
            player.request_audio_probe().is_err(),
            "only one native probe is admitted"
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let snapshot = player.drain_events();
            if let Some(reply) = snapshot.audio_probe_reply {
                assert_eq!(reply.token, audio_token);
                assert_eq!(reply.load_request_id, snapshot.load_request_id);
                assert_eq!(reply.decoder_sample_rate, Some(48_000));
                assert_eq!(reply.output_sample_rate, Some(48_000));
                assert!(reply.audio_pts.is_some());
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "native audio diagnostic timed out"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let previous = player.snapshot().load_request_id;
        player
            .load_local_at(&fixture.0.with_extension("missing"), 0., true)
            .unwrap();
        assert!(!player.current_load_frame_ready());
        assert!(player.snapshot().audio_probe_reply.is_none());
        // A real stale native GET_PROPERTY reply cannot reopen an old load gate.
        assert!(
            unsafe {
                ffi::mpv_get_property_async(
                    player.inner.raw,
                    previous,
                    c"video-dec-params/w".as_ptr(),
                    4,
                )
            } >= 0
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let snapshot = player.drain_events();
            assert!(!player.current_load_frame_ready());
            if snapshot.state == PlaybackState::Failed {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "missing-file observation timed out"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    #[test]
    fn callbacks_coalesce_without_losing_frame_state() {
        let called = Arc::new(AtomicU64::new(0));
        let clone = called.clone();
        let wake = Wake {
            queued: AtomicBool::new(false),
            frame: AtomicBool::new(false),
            due: AtomicBool::new(false),
            count: AtomicU64::new(0),
            render_count: AtomicU64::new(0),
            timing_origin: None,
            callback_time_ns: AtomicU64::new(0),
            #[cfg(target_os = "macos")]
            clock: macos::ClockSignals::default(),
            callback: Box::new(move || {
                clone.fetch_add(1, Ordering::Relaxed);
            }),
        };
        for _ in 0..1000 {
            unsafe {
                render_wake(std::ptr::from_ref(&wake).cast_mut().cast());
            }
        }
        assert_eq!(called.load(Ordering::Relaxed), 1);
        assert!(wake.frame.load(Ordering::Acquire));
        assert_eq!(wake.callback_time_ns.load(Ordering::Relaxed), 0);
        wake.queued.store(false, Ordering::Release);
        wake.notify();
        assert_eq!(called.load(Ordering::Relaxed), 2);
    }
    #[test]
    fn engine_initializes_and_async_commands_are_observed() {
        let player =
            Player::new(|| {}).expect("installed libmpv must initialize with isolated options");
        player.set_paused(true).unwrap();
        player.set_volume(37.).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let snapshot = player.drain_events();
            assert!(snapshot.error.is_none(), "{:?}", snapshot.error);
            if snapshot.paused && (snapshot.volume - 37.).abs() < 0.1 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "async property observation timed out"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(player.seek(f64::NAN).is_err());
        assert!(player.set_volume(f64::INFINITY).is_err());
        assert!(player.set_speed(0.).is_err());
        assert!(
            player
                .load_local_at(
                    std::path::Path::new("/synthetic-unused.mp4"),
                    f64::NAN,
                    true
                )
                .is_err()
        );
        assert!(
            player
                .load_https_at("https://r1.googlevideo.com/video", None, -1., true)
                .is_err()
        );
        assert!(
            player
                .load_local(std::path::Path::new("relative.mp4"))
                .is_err()
        );
        assert!(
            player
                .load_local_with_subtitle_at(
                    &std::env::current_dir().unwrap().join("unopened.mp4"),
                    Some(std::path::Path::new("relative.srt")),
                    0.,
                    false,
                )
                .is_err()
        );
        assert_eq!(player.inner.pending_commands.get(), 0);
    }
    #[test]
    fn tls_verification_is_enabled_and_relative_ca_paths_are_rejected() {
        unsafe extern "C" {
            fn mpv_get_property(
                raw: *mut ffi::Handle,
                name: *const std::ffi::c_char,
                format: i32,
                value: *mut std::ffi::c_void,
            ) -> i32;
        }
        let player = Player::new(|| {}).unwrap();
        let mut verify = 0i32;
        let code = unsafe {
            mpv_get_property(
                player.inner.raw,
                c"options/tls-verify".as_ptr(),
                3,
                std::ptr::from_mut(&mut verify).cast(),
            )
        };
        assert!(code >= 0);
        assert_eq!(verify, 1);
        assert!(
            Player::new_with_ca_file(|| {}, Some(std::path::Path::new("relative.pem"))).is_err()
        );
    }
    #[test]
    fn url_rejects_secrets_and_unsafe_schemes() {
        for value in [
            "file:///etc/passwd",
            "https://user:secret@host/video",
            "http://host/video",
            "https://host/\nvideo",
            "https://",
            "https://localhost/video",
            "https://127.0.0.1/video",
            "https://googlevideo.com.evil.test/video",
            "https://googlevideo.com:8443/video",
        ] {
            assert!(!valid_direct_url(value));
        }
        assert!(valid_direct_url(
            "https://video.googlevideo.com/videoplayback?expire=123"
        ));
    }
}
