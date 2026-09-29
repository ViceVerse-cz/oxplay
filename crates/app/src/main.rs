// SPDX-License-Identifier: GPL-3.0-or-later
mod account;
mod account_playback;
mod account_ui;
mod caption_cache;
mod caption_files;
mod caption_ui;
mod catalog;
mod clear_smoke;
mod cli;
mod clock_ui;
mod comments_ui;
mod decode_warning;
mod feed_focus;
mod fixture_quiescence;
mod focus_intent;
mod groups;
mod guest_playback;
mod guest_recovery;
mod guest_ui;
mod handoff_smoke;
mod helper_paths;
pub mod library;
mod library_fixture;
mod library_resource_smoke;
mod library_smoke;
mod library_ui;
mod media_network;
mod model;
mod motion_smoke;
mod native_child;
mod native_child_fixture;
mod native_child_smoke;
mod picture_in_picture;
mod pip_smoke;
mod playback_preferences;
mod playback_ui;
mod preferences_smoke;
mod presentation_diagnostics;
mod recovery_smoke;
mod related_focus_smoke;
mod resolver;
mod save_smoke;
mod soak_smoke;
mod thumbnails;
use catalog::{Response, Worker};
use model::CatalogModel;
use serein_media::{GlPresenter, Player};
use slint::winit_030::{EventResult, WinitWindowAccessor, winit};
use slint::{ComponentHandle, Model, Timer, TimerMode};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};
slint::include_modules!();

struct UiState {
    pip: picture_in_picture::Controller,
    pip_exit_pending: Cell<bool>,
    presenter_generations: Cell<u64>,
    account_ui: account_ui::State,
    account_playback: account_playback::State,
    guest_playback: guest_playback::State,
    guest_recovery: guest_recovery::State,
    started: std::time::Instant,
    player: Player,
    native_child: native_child::State,
    media_network: Option<serein_network::NetworkConfig>,
    account_media_network: Option<serein_network::NetworkConfig>,
    presentation_ready: Cell<bool>,
    presentation_retry: Cell<bool>,
    progress: Timer,
    clock_ui: clock_ui::State,
    hidden: Cell<bool>,
    model: Rc<CatalogModel<VideoRow>>,
    worker: RefCell<Worker>,
    groups: groups::Groups,
    guest_ui: guest_ui::State,
    feed_focus: feed_focus::State,
    focus_intent: focus_intent::State,
    playback_preferences: playback_preferences::State,
    comments_ui: comments_ui::State,
    caption_ui: caption_ui::State,
    caption_cache: caption_cache::State,
    playback_ui: playback_ui::State,
    thumbnails: RefCell<thumbnails::Worker>,
    thumbnail_range: Cell<(usize, usize)>,
    thumbnail_attempted: RefCell<std::collections::HashSet<usize>>,
    library: library::Worker,
    library_ui: library_ui::State,
    library_fixture: Option<library_fixture::Config>,
    fixture_quiescence: fixture_quiescence::State,
    playlists: RefCell<Vec<serein_storage::LocalPlaylist>>,
    preferences: Cell<serein_storage::LocalPreferences>,
    current_video: RefCell<Option<serein_core::VideoSummary>>,
    quality_index: Cell<usize>,
    displayed_elapsed: Cell<u64>,
    displayed_remaining: Cell<u64>,
    displayed_position: Cell<f32>,
    ui_assignments: Cell<u64>,
    redraw_requests: Cell<u64>,
    draw_callbacks: Cell<u64>,
    geometry_events: Option<Cell<u64>>,
}
fn report(app: &App, result: serein_media::Result<()>) {
    if let Err(e) = result {
        app.set_status(e.to_string().into());
    }
}
fn apply_clock(app: &App, state: &UiState, values: clock_ui::Values) {
    let elapsed = values.position.max(0.) as u64;
    if state.displayed_elapsed.replace(elapsed) != elapsed {
        app.set_elapsed(format!("{}:{:02}", elapsed / 60, elapsed % 60).into());
        state.ui_assignments.set(state.ui_assignments.get() + 1);
    }
    let remaining = (values.duration - values.position).max(0.).ceil() as u64;
    if state.displayed_remaining.replace(remaining) != remaining {
        app.set_remaining(format!("{}:{:02}", remaining / 60, remaining % 60).into());
        state.ui_assignments.set(state.ui_assignments.get() + 1);
    }
    // Reading the generated property inside BeforeRendering would subscribe
    // the active draw tracker immediately before mutating it, scheduling a
    // redundant draw. All clock writes (including account clear) use this cache.
    let position = values.position as f32;
    if (position == 0. && state.displayed_position.get() != 0.)
        || (state.displayed_position.get() - position).abs() > 0.01
    {
        state.displayed_position.set(position);
        app.set_position(position);
        state.ui_assignments.set(state.ui_assignments.get() + 1);
    }
}
fn update(app: &App, state: &Rc<UiState>) {
    account_playback::observe(app, state);
    let snapshot = state.player.drain_events();
    playback_preferences::observe(app, state, &snapshot);
    caption_ui::observe(app, state, &snapshot);
    playback_ui::observe(app, state, &snapshot);
    guest_playback::observe(app, state, &snapshot);
    let playback_status = playback_ui::status(
        &snapshot,
        app.get_loaded(),
        state.player.current_load_is_active(),
    );
    if app.get_playback_status().as_str() != playback_status {
        app.set_playback_status(playback_status.into());
        state.ui_assignments.set(state.ui_assignments.get() + 1);
    }
    let can_retry = account_playback::can_retry(app, state, &snapshot)
        || guest_playback::can_retry(app, state, &snapshot);
    let account_retry_ready = account_playback::retry_ready(state, &snapshot);
    if app.get_account_retry_ready() != account_retry_ready {
        app.set_account_retry_ready(account_retry_ready);
    }
    if app.get_can_retry_playback() != can_retry {
        app.set_can_retry_playback(can_retry);
        state.ui_assignments.set(state.ui_assignments.get() + 1);
    }
    let playback_failed = app.get_loaded() && guest_playback::failed_load(&snapshot);
    if app.get_playback_failed() != playback_failed {
        app.set_playback_failed(playback_failed);
        state.ui_assignments.set(state.ui_assignments.get() + 1);
    }
    let active_video = app.get_loaded()
        && state.player.current_load_is_active()
        && snapshot.width > 0
        && snapshot.height > 0
        && !snapshot.codec.is_empty();
    let warning = decode_warning::warning(
        snapshot.diagnostic_silent_audio,
        active_video,
        &snapshot.hwdec_current,
    );
    if app.get_diagnostic_warning().as_str() != warning {
        app.set_diagnostic_warning(warning.into());
    }
    if app.get_paused() != snapshot.paused {
        app.set_paused(snapshot.paused);
        state.ui_assignments.set(state.ui_assignments.get() + 1);
    }
    if app.get_muted() != snapshot.muted {
        app.set_muted(snapshot.muted);
        state.ui_assignments.set(state.ui_assignments.get() + 1);
    }
    if app.get_mute_known() != snapshot.mute_observed {
        app.set_mute_known(snapshot.mute_observed);
        state.ui_assignments.set(state.ui_assignments.get() + 1);
    }
    // A terminal stop is asynchronous. Never republish the previous account's
    // clock after its UI metadata was cleared while the engine is stopping.
    let (position, duration) = if app.get_loaded() && state.player.current_load_is_active() {
        (snapshot.position, snapshot.duration)
    } else {
        (0., 0.)
    };
    let stage_clock = matches!(snapshot.state, serein_media::PlaybackState::Playing)
        && !snapshot.paused
        && snapshot.display_clock_active
        && state.presentation_ready.get()
        && app.get_loaded()
        && app.get_progress_visible()
        && !state.hidden.get();
    if let Some(values) = state.clock_ui.stage_or_immediate(
        clock_ui::Values { position, duration },
        state.player.clock_identity(),
        stage_clock,
    ) {
        apply_clock(app, state, values);
    }
    if app.get_duration() != duration.max(1.) as f32 {
        app.set_duration(duration.max(1.) as f32);
    }
    if let Some(error) = snapshot.error {
        app.set_status(error.into());
    }
    if app.get_show_info() {
        let info = format!(
            "Winit · FemtoVG/OpenGL · decoder: {} · {} · {}×{} @ {:.2} fps · dropped {}\nUI assignments {} · catalog changes {} / resets {} · requested draws {} / callbacks {}",
            snapshot.hwdec_current,
            snapshot.codec,
            snapshot.width,
            snapshot.height,
            snapshot.fps,
            snapshot.dropped_frames,
            state.ui_assignments.get(),
            state.model.changes.get(),
            state.model.resets.get(),
            state.redraw_requests.get(),
            state.draw_callbacks.get()
        );
        if app.get_technical().as_str() != info {
            app.set_technical(info.into());
        }
    }
    if matches!(snapshot.state, serein_media::PlaybackState::Playing)
        && !snapshot.paused
        && app.get_loaded()
        && snapshot.load_request_id != 0
        && snapshot.load_request_id == snapshot.active_load_request_id
        && state.player.current_load_is_active()
        && account_playback::authorization(state).is_none()
        && let Some(video) = state.current_video.borrow().as_ref()
    {
        library_ui::record_playback(
            state,
            video,
            Duration::from_secs(snapshot.position.max(0.) as u64),
        );
    }
    let active = matches!(snapshot.state, serein_media::PlaybackState::Playing)
        && !snapshot.paused
        && app.get_loaded()
        && app.get_page() == 2
        && app.get_progress_visible()
        && !state.hidden.get();
    if active && !state.progress.running() {
        let player = state.player.clone();
        state
            .progress
            .start(TimerMode::Repeated, Duration::from_millis(250), move || {
                let _ = player.request_progress();
            });
    } else if !active {
        state.progress.stop();
    }
    if state.native_child.enabled {
        native_child::render(app, state);
    } else if state.player.frame_pending() && !state.hidden.get() {
        state.redraw_requests.set(state.redraw_requests.get() + 1);
        // The adopted macOS CADisplayLink throttle stopped delivering draws in
        // release runs despite pending redraws. Enqueue a Winit redraw directly
        // for a media notification; this is event-driven, never a timer loop.
        #[cfg(target_os = "macos")]
        app.window().with_winit_window(|w| w.request_redraw());
        #[cfg(not(target_os = "macos"))]
        app.window().request_redraw();
    }
    playback_ui::maybe_refresh(app, state);
}
struct StartupMedia {
    local: Option<std::path::PathBuf>,
    subtitle: Option<std::path::PathBuf>,
    paused: bool,
}
fn start_local_media(app: &App, state: &Rc<UiState>, startup: &mut Option<StartupMedia>) {
    if let Some(startup) = startup.take()
        && let Some(path) = startup.local
    {
        app.set_loaded(true);
        app.set_page(2);
        app.set_video_title(
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
                .into(),
        );
        app.set_video_channel("Local file".into());
        match state.player.load_local_with_subtitle_at(
            &path,
            startup.subtitle.as_deref(),
            0.,
            startup.paused,
        ) {
            Ok(()) => focus_intent::startup_ready(app, state),
            Err(error) => {
                native_child::hide(state);
                state.focus_intent.cancel(focus_intent::Scope::LocalStartup);
                app.set_loaded(false);
                app.set_status(error.to_string().into());
            }
        }
    }
}
fn native_fullscreen(app: &App) -> bool {
    app.window().is_fullscreen()
        || app
            .window()
            .with_winit_window(|window| window.fullscreen().is_some())
            .unwrap_or(false)
}

fn request_windowed(app: &App) {
    app.window().set_fullscreen(false);
    app.window()
        .with_winit_window(|window| window.set_fullscreen(None));
    app.set_fullscreen_active(false);
}

fn exit_picture_in_picture(app: &App, state: &UiState) {
    if native_fullscreen(app) {
        state.pip_exit_pending.set(true);
        request_windowed(app);
        return;
    }
    let result = state.pip.exit(app);
    match result {
        Ok(()) => {
            state.pip_exit_pending.set(false);
            app.invoke_focus_video_mode();
        }
        Err(error) => app.set_status(error.into()),
    }
}

fn setup_presenter(
    app: &App,
    state: &Rc<UiState>,
    api: &slint::GraphicsAPI<'_>,
    startup: &mut Option<StartupMedia>,
) -> Option<GlPresenter> {
    state.presentation_retry.set(false);
    let slint::GraphicsAPI::NativeOpenGL { get_proc_address } = api else {
        app.set_status("Unsupported graphics API: OpenGL is required for this presenter".into());
        return None;
    };
    if state.native_child.enabled {
        match native_child::setup(app, state) {
            Ok(info) => {
                state.presentation_ready.set(true);
                eprintln!("native child graphics: {info}");
                app.set_technical(info.into());
                start_local_media(app, state, startup);
            }
            Err(error) => {
                state.presentation_ready.set(false);
                eprintln!("native child setup failed: {error}");
                app.set_status(error.into());
            }
        }
        return None;
    }
    // SAFETY: only called from Setup/BeforeRendering with the one owning context current.
    match unsafe { GlPresenter::new(&state.player, get_proc_address) } {
        Ok(presenter) => {
            app.set_pip_available(picture_in_picture::availability(app.window()).is_ok());
            state
                .presenter_generations
                .set(state.presenter_generations.get().saturating_add(1));
            state.presentation_ready.set(true);
            eprintln!("graphics: {}", presenter.graphics_info());
            app.set_technical(presenter.graphics_info().into());
            start_local_media(app, state, startup);
            Some(presenter)
        }
        Err(error) => {
            state.presentation_ready.set(false);
            eprintln!("presenter setup failed: {error}");
            app.set_status("Video presentation is unavailable. Activate the display to retry; details are in local diagnostics.".into());
            None
        }
    }
}
// CLI file checks happen before creating the UI or entering its event loop.
fn selected_file(value: Option<std::path::PathBuf>) -> std::io::Result<Option<std::path::PathBuf>> {
    value
        .map(|path| {
            let path = std::fs::canonicalize(path)?;
            if path.to_str().is_none() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "the native media interface requires a UTF-8 file path",
                ));
            }
            if !path.is_file() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "selected media must be a file",
                ));
            }
            Ok(path)
        })
        .transpose()
}
fn video_row(video: &serein_core::VideoSummary) -> VideoRow {
    VideoRow {
        title: video.title.clone().into(),
        channel: video.channel.clone().into(),
        id: video.id.as_str().into(),
        duration: video
            .duration
            .map(|d| format!("{}:{:02}", d.as_secs() / 60, d.as_secs() % 60))
            .unwrap_or_default()
            .into(),
        ..VideoRow::default()
    }
}
fn viewport(state: &UiState, first: usize, end: usize, visible: Option<(usize, usize)>) {
    let len = state.model.row_count();
    let (first, end) = groups::thumbnail_window(len, (first, end), visible);
    if state.thumbnail_range.replace((first, end)) == (first, end) {
        return;
    }
    state
        .thumbnail_attempted
        .borrow_mut()
        .retain(|row| (first..end).contains(row));
    for row in 0..len {
        if !(first..end).contains(&row)
            && let Some(mut item) = state.model.row_data(row)
            && item.thumbnail_ready
        {
            item.thumbnail = slint::Image::default();
            item.thumbnail_ready = false;
            state.model.set_row_data(row, item.clone());
            state.groups.update(row, item);
        }
    }
    let requests = (first..end)
        .filter_map(|row| {
            if state.thumbnail_attempted.borrow().contains(&row)
                || state.model.row_data(row)?.thumbnail_ready
            {
                return None;
            }
            Some(thumbnails::Request {
                row,
                source: if let Some(config) = &state.library_fixture {
                    thumbnails::Source::Fixture {
                        source: config.source(),
                        index: row % library_fixture::THUMBNAILS,
                    }
                } else {
                    thumbnails::Source::Remote(state.guest_ui.thumbnail(row)?)
                },
            })
        })
        .collect();
    state.thumbnails.borrow_mut().replace(requests);
}
fn bind_browsing(app: &App, state: &Rc<UiState>) {
    let weak = app.as_weak();
    let s = state.clone();
    app.on_columns_changed(move |columns| {
        if let Some(app) = weak.upgrade() {
            feed_focus::columns_changed(&app, &s, columns as usize);
            app.invoke_refresh_visible();
        }
    });
    let s = state.clone();
    app.on_visible_range(move |first, end, visible_first, visible_end| {
        let visible = (visible_first >= 0 && visible_end >= 0)
            .then_some((visible_first.max(0) as usize, visible_end.max(0) as usize));
        viewport(&s, first.max(0) as usize, end.max(0) as usize, visible)
    });
    let s = state.clone();
    let weak = app.as_weak();
    app.on_thumbnail_wake(move || {
        loop {
            let ready = s.thumbnails.borrow_mut().take();
            let Some(ready) = ready else { break };
            s.thumbnail_attempted.borrow_mut().insert(ready.row);
            let Some(pixels) = ready.pixels else { continue };
            let Some(mut row) = s.model.row_data(ready.row) else {
                continue;
            };
            let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                pixels.as_raw(),
                pixels.width(),
                pixels.height(),
            );
            row.thumbnail = slint::Image::from_rgba8(buffer);
            row.thumbnail_ready = true;
            s.model.set_row_data(ready.row, row.clone());
            s.groups.update(ready.row, row);
            s.thumbnails
                .borrow()
                .record_publication(pixels.as_raw().len());
        }
        if s.library_fixture.is_some()
            && let Some(app) = weak.upgrade()
        {
            fixture_quiescence::observe(&app, &s);
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_navigate(move |page| {
        let Some(app) = weak.upgrade() else { return };
        let previous = app.get_page();
        if previous == page {
            return;
        }
        native_child::hide(&s);
        if page == 1 {
            let tab = app.global::<LibraryUi>().get_tab();
            let tab = if tab == 2 && !app.global::<LibraryUi>().get_history_enabled() {
                0
            } else {
                tab
            };
            if !library_ui::open_tab(&app, &s, tab) {
                return;
            }
        } else {
            app.global::<LibraryUi>().set_confirmation(0);
        }
        s.worker.borrow_mut().cancel();
        app.set_busy(false);
        if page != 2 {
            let _ = s.player.set_paused(true);
        }
        app.set_page(page);
        s.thumbnail_range.set((usize::MAX, usize::MAX));
        app.invoke_refresh_visible();
        update(&app, &s);
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_open_local_tab(move |tab| {
        let Some(app) = weak.upgrade() else { return };
        native_child::hide(&s);
        if !library_ui::open_tab(&app, &s, tab) {
            return;
        }
        s.worker.borrow_mut().cancel();
        app.set_busy(false);
        let _ = s.player.set_paused(true);
        s.thumbnail_range.set((usize::MAX, usize::MAX));
        app.invoke_refresh_visible();
        update(&app, &s);
    });
    library_ui::bind(app, state);
}

// Native libavformat cannot yet enforce per-origin headers across redirects.
// These extractor browser hints are optional on the exercised guest content path;
// Origin/Referer and every unknown/credential header require the policy adapter.
fn guest_media_headers_supported(item: &serein_core::ResolvedPlayback) -> bool {
    item.guest
        && item.session_generation == 0
        && std::iter::once(&item.video_track)
            .chain(item.audio_track.iter())
            .all(|track| {
                track.headers.fields.iter().all(|(name, _)| {
                    matches!(
                        name.as_str(),
                        "user-agent" | "accept" | "accept-language" | "sec-fetch-mode"
                    )
                })
            })
}
const QUALITY_HEIGHTS: [u16; 6] = [1080, 720, 480, 360, 240, 144];

fn load_remote(
    state: &UiState,
    item: &serein_core::ResolvedPlayback,
    position: f64,
    paused: bool,
) -> Result<(), String> {
    if state.native_child.enabled {
        return Err(
            "Native-child diagnostic accepts only its explicitly selected local file.".into(),
        );
    }
    if !item.guest || item.session_generation != 0 {
        return Err("Account playback requires an explicit, current authorization.".into());
    }
    if let Some(config) = &state.media_network {
        return media_network::load(&state.player, item, config, position, paused);
    }
    if !guest_media_headers_supported(item) {
        return Err("This stream needs request headers that the direct player cannot safely scope across redirects.".into());
    }
    state
        .player
        .load_https_at(
            item.video_track.url.expose_url(),
            item.audio_track
                .as_ref()
                .map(|audio| audio.url.expose_url()),
            position,
            paused,
        )
        .map_err(|e| e.to_string())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = cli::Options::parse(std::env::args_os().skip(1))?;
    if options.help {
        println!("{}", cli::HELP);
        return Ok(());
    }
    let save_smoke_video = if options.save_smoke {
        Some(serein_core::VideoId::from_url(
            options
                .url
                .as_deref()
                .ok_or("Save diagnostic requires a public video URL")?,
        )?)
    } else {
        None
    };
    if options.soak_minutes.is_some() {
        soak_smoke::validate_environment()?;
    }
    if options.native_video_child {
        native_child::validate_environment()?;
    }
    // Fixture capabilities can only be admitted from an explicit private root
    // before UI startup. Provider metadata can never select local image files.
    let library_fixture = options
        .library_resource_fixture
        .as_deref()
        .map(library_fixture::Config::admit)
        .transpose()?;
    let mut local = if options.native_video_child || options.related_focus_check {
        options.local
    } else {
        selected_file(options.local)?
    };
    let local_startup = local.is_some();
    let subtitle = selected_file(options.subtitle)?;
    let soak_config = options.soak_minutes.map(|minutes| soak_smoke::Config {
        minutes,
        local: local.clone().expect("CLI requires local soak fixture"),
        subtitle: subtitle.clone(),
    });
    let handoff_audio = selected_file(options.handoff_audio)?;
    let handoff_video = handoff_audio.as_ref().and(local.clone());
    let smoke = options.smoke;
    let demo_related = options.demo_related;
    let url = options.url.or(options.search);
    let refresh_smoke = options.refresh_smoke;
    let captions_smoke = options.captions_smoke || options.clear_local_smoke;
    let comments_smoke = options.comments_smoke;
    let comments_snapshots = options.comments_snapshots;
    if comments_snapshots
        .as_ref()
        .is_some_and(|path| !path.is_absolute() || !path.is_dir())
    {
        return Err("--comments-snapshots requires an existing absolute directory".into());
    }
    let quality_smoke = options.quality_smoke;
    let start_paused = options.paused;
    let minimized = options.minimized;
    // An explicit root keeps native validation isolated from the normal profile.
    // Only its Serein child is app-owned; never chmod the selected root itself.
    let library_path = if let Some(root) = options.data_root {
        if !root.is_absolute() {
            return Err("--data-root requires an absolute directory".into());
        }
        if let Some(phase) = options.preferences_smoke {
            preferences_smoke::prepare_root(&root, phase)?;
        } else if options.clear_local_smoke
            || options.save_smoke
            || options.recovery_smoke
            || options.pip_smoke
            || options.library_smoke
            || options.soak_minutes.is_some()
            || options.native_video_child
            || options.related_focus_check
        {
            clear_smoke::create_root(&root)?;
        }
        root.join("Serein/library.sqlite3")
    } else {
        library::data_path()?
    };
    let account_directory = library_path
        .parent()
        .ok_or("invalid application data directory")?
        .join("sessions");
    // The fresh private root is already admitted. Only this exact copied MP4,
    // never the original path reopened after hashing, reaches the diagnostic.
    // Declared before App/Player so normal and error paths release media first.
    let _native_fixture = if options.native_video_child || options.related_focus_check {
        let root = library_path
            .parent()
            .and_then(|directory| directory.parent())
            .ok_or("invalid native diagnostic profile")?;
        let fixture = native_child_fixture::Fixture::prepare(
            local
                .as_deref()
                .ok_or("native diagnostic requires its local fixture")?,
            root,
        )?;
        local = Some(fixture.path().to_owned());
        Some(fixture)
    } else {
        None
    };
    let ui_page = options.ui_page;
    let ui_theme = options.ui_theme;
    let ui_size = options.ui_size;
    let helpers = helper_paths::HelperPaths::discover(
        &std::env::current_exe()?,
        options.yt_dlp,
        options.deno,
    )?;
    helpers.validate_media_ca()?;
    let account_media_network = if helpers.validate_dns_helper().is_ok() {
        Some(serein_network::NetworkConfig::new(
            helpers.dns_helper.clone(),
        )?)
    } else {
        None
    };
    let media_network = if options.scoped_media {
        helpers.validate_dns_helper()?;
        Some(serein_network::NetworkConfig::new(
            helpers.dns_helper.clone(),
        )?)
    } else {
        None
    };
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("femtovg".into())
        .require_opengl()
        .select()?;
    // Declare before App/Player so error-path destruction releases all media
    // leases before the caption cleanup owner joins.
    let caption_cleanup = caption_files::Worker::new(
        library_path
            .parent()
            .ok_or("invalid application data directory")?
            .join("captions"),
    )?;
    let app = App::new()?;
    app.set_native_video_child(options.native_video_child);
    app.set_cache_chrome(options.ui_cache);
    app.set_cache_search(options.search_cache);
    app.set_cache_related(options.related_cache);
    let weak = app.as_weak();
    let player = Player::new_with_ca_file(
        move || {
            let _ = weak.upgrade_in_event_loop(|app| app.invoke_media_wake());
        },
        helpers.media_ca.as_deref(),
    )?;
    if player.snapshot().diagnostic_silent_audio {
        app.set_diagnostic_warning(
            "DIAGNOSTIC: audio output is silent. This run cannot pass playback acceptance.".into(),
        );
    }
    let weak = app.as_weak();
    let resolver = resolver::SharedResolver::new(helpers.yt_dlp, helpers.deno);
    let worker = Worker::with_resolver(resolver.clone(), move || {
        let _ = weak.upgrade_in_event_loop(|app| app.invoke_worker_wake());
    });
    let weak = app.as_weak();
    let thumbnails = thumbnails::Worker::new(
        move || {
            let _ = weak.upgrade_in_event_loop(|app| app.invoke_thumbnail_wake());
        },
        library_fixture.is_some(),
    );
    let weak = app.as_weak();
    let library = library::Worker::new(library_path, move || {
        let _ = weak.upgrade_in_event_loop(|app| app.invoke_library_wake());
    });
    let account_ui = account_ui::State::new(app.as_weak(), account_directory, resolver);
    let state = Rc::new(UiState {
        pip: picture_in_picture::Controller::default(),
        pip_exit_pending: Cell::new(false),
        presenter_generations: Cell::new(0),
        account_ui,
        account_playback: account_playback::State::default(),
        guest_playback: guest_playback::State::default(),
        guest_recovery: guest_recovery::State::default(),
        started: std::time::Instant::now(),
        player,
        native_child: native_child::State::new(options.native_video_child),
        media_network,
        account_media_network,
        presentation_ready: Cell::new(false),
        presentation_retry: Cell::new(false),
        progress: Timer::default(),
        clock_ui: clock_ui::State::new(options.stage_progress),
        hidden: Cell::new(false),
        model: Rc::new(CatalogModel::default()),
        worker: RefCell::new(worker),
        groups: groups::Groups::default(),
        guest_ui: guest_ui::State::default(),
        feed_focus: feed_focus::State::default(),
        focus_intent: focus_intent::State::default(),
        playback_preferences: playback_preferences::State::default(),
        comments_ui: comments_ui::State::default(),
        caption_ui: caption_ui::State::new(caption_cleanup.client()),
        caption_cache: caption_cache::State::default(),
        playback_ui: playback_ui::State::default(),
        thumbnails: RefCell::new(thumbnails),
        thumbnail_range: Cell::new((usize::MAX, usize::MAX)),
        thumbnail_attempted: RefCell::new(std::collections::HashSet::new()),
        library,
        library_ui: library_ui::State::default(),
        library_fixture,
        fixture_quiescence: fixture_quiescence::State::default(),
        playlists: RefCell::new(Vec::new()),
        preferences: Cell::new(serein_storage::LocalPreferences::default()),
        current_video: RefCell::new(None),
        quality_index: Cell::new(0),
        displayed_elapsed: Cell::new(u64::MAX),
        displayed_remaining: Cell::new(u64::MAX),
        displayed_position: Cell::new(0.),
        ui_assignments: Cell::new(0),
        redraw_requests: Cell::new(0),
        draw_callbacks: Cell::new(0),
        geometry_events: options.diagnostics.then(|| Cell::new(0)),
    });
    app.set_videos(slint::ModelRc::from(state.model.clone()));
    app.set_groups(slint::ModelRc::from(state.groups.model.clone()));
    bind_browsing(&app, &state);
    account_ui::bind(&app, &state);
    state
        .groups
        .set_columns(app.get_columns() as usize, &state.model);
    state.library.submit(library::Request::Load);
    let weak = app.as_weak();
    let s = state.clone();
    app.on_media_wake(move || {
        if let Some(app) = weak.upgrade() {
            update(&app, &s);
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_toggle_pause(move || {
        if let Some(app) = weak.upgrade() {
            if account_playback::retry_failed(&app, &s) || guest_playback::retry_failed(&app, &s) {
                return;
            }
            report(&app, s.player.toggle_pause());
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_seek(move |position| {
        if let Some(app) = weak.upgrade() {
            report(&app, s.player.seek(position as f64));
            let _ = s.player.request_progress();
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_volume(move |volume| {
        if let Some(app) = weak.upgrade() {
            match s.player.set_volume(volume as f64) {
                Ok(()) => {
                    app.set_volume_level(volume);
                    library_ui::schedule_volume_save(&app, &s);
                }
                Err(error) => app.set_status(error.to_string().into()),
            }
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_subtitles(move || {
        if let Some(app) = weak.upgrade() {
            report(&app, s.player.cycle_subtitles());
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_relative_seek(move |amount| {
        if let Some(app) = weak.upgrade() {
            report(&app, s.player.seek_relative(amount as f64));
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_mute(move || {
        if let Some(app) = weak.upgrade() {
            report(&app, s.player.toggle_mute());
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_speed(move |speed| {
        if let Some(app) = weak.upgrade() {
            playback_preferences::request(&app, &s, speed as f64);
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_quality(move |index| {
        let Some(app) = weak.upgrade() else { return };
        // Display the active limit until a new stream has actually resolved.
        app.set_quality_index(s.quality_index.get() as i32);
        if app.get_busy() || !app.get_remote_video() {
            return;
        }
        let Some(height) = usize::try_from(index)
            .ok()
            .and_then(|i| QUALITY_HEIGHTS.get(i))
            .copied()
        else {
            return;
        };
        let Some(video) = s.current_video.borrow().clone() else {
            return;
        };
        if s.quality_index.get() == index as usize {
            return;
        }
        app.set_controls_visible(true);
        update(&app, &s);
        if !app.get_loaded()
            || s.current_video.borrow().as_ref().map(|current| &current.id) != Some(&video.id)
        {
            return;
        }
        if account_playback::authorization(&s).is_some() {
            if let Err(error) = account_playback::request_replacement(
                &app,
                &s,
                &video.id,
                serein_youtube::ResolutionPolicy {
                    max_height: height,
                    prefer_h264: true,
                },
                false,
            ) {
                app.set_status(error.into());
            }
            return;
        }
        s.worker
            .borrow_mut()
            .submit(catalog::Request::ResolveQuality(
                video.id,
                serein_youtube::ResolutionPolicy {
                    max_height: height,
                    prefer_h264: true,
                },
            ));
        app.set_busy(true);
        app.set_status(
            "Selecting the requested maximum quality… Current playback continues.".into(),
        );
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_fullscreen(move || {
        if let Some(app) = weak.upgrade() {
            if app.get_picture_in_picture() {
                exit_picture_in_picture(&app, &s);
                return;
            }
            native_child::hide(&s);
            let fullscreen = !app.window().is_fullscreen();
            app.window().set_fullscreen(fullscreen);
            app.set_fullscreen_active(fullscreen);
        }
    });
    let weak = app.as_weak();
    // Finite keyboard smoke evidence only; no recurring sampling or production
    // counter updates. Detect commands that have no visible effect when already
    // windowed, such as a wrongly intercepted Escape from the search editor.
    let fullscreen_exit_attempts = smoke.then(|| Rc::new(Cell::new(0u64)));
    let exit_attempts = fullscreen_exit_attempts.clone();
    let s = state.clone();
    app.on_exit_fullscreen(move || {
        if let Some(app) = weak.upgrade() {
            if app.get_picture_in_picture() {
                exit_picture_in_picture(&app, &s);
                return;
            }
            native_child::hide(&s);
            if let Some(counter) = &exit_attempts {
                counter.set(counter.get() + 1);
            }
            app.window().set_fullscreen(false);
            app.set_fullscreen_active(false);
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_toggle_controls(move || {
        if let Some(app) = weak.upgrade() {
            app.set_controls_visible(!app.get_controls_visible());
            update(&app, &s);
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_watch_visibility_changed(move || {
        if let Some(app) = weak.upgrade() {
            update(&app, &s);
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_picture_in_picture_toggle(move || {
        let Some(app) = weak.upgrade() else { return };
        if app.get_picture_in_picture() {
            exit_picture_in_picture(&app, &s);
            return;
        }
        if !app.get_loaded()
            || app.get_page() != 2
            || app.get_playback_overlay_open()
            || app.get_fullscreen_active()
            || s.native_child.enabled
        {
            return;
        }
        match s.pip.enter(&app) {
            Ok(()) => {
                app.invoke_focus_video_mode();
            }
            Err(error) => app.set_status(error.into()),
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_exit_picture_in_picture(move || {
        if let Some(app) = weak.upgrade() {
            exit_picture_in_picture(&app, &s);
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.window().on_close_requested(move || {
        if let Some(app) = weak.upgrade()
            && app.get_picture_in_picture()
        {
            exit_picture_in_picture(&app, &s);
            slint::CloseRequestResponse::KeepWindowShown
        } else {
            slint::CloseRequestResponse::HideWindow
        }
    });
    guest_ui::bind(&app, &state);
    feed_focus::bind(&app, &state);
    comments_ui::bind(&app, &state);
    caption_ui::bind(&app, &state);
    caption_cache::bind(&app, &state);
    playback_ui::bind(&app, &state);
    focus_intent::bind(&app, &state);
    guest_recovery::bind(&app, &state);
    native_child::bind(&app, &state);
    let weak = app.as_weak();
    let s = state.clone();
    app.on_worker_wake(move || {
        let Some(app) = weak.upgrade() else { return };
        let Some(result) = s.worker.borrow().take() else {
            return;
        };
        app.set_busy(false);
        if result.is_ok() {
            guest_recovery::invalidate(&app, &s);
        }
        match result {
            Ok(Response::Catalog(page)) => guest_ui::publish(&app, &s, *page),
            Ok(Response::Comments(video, result)) => comments_ui::publish(&app, &s, video, result),
            Ok(Response::Caption(video, index, result)) => {
                caption_ui::receive(&app, &s, video, index, result)
            }
            Ok(Response::Resolved(item, quality)) => {
                guest_ui::resolution_finished(&app, &s, s.worker.borrow().generation());
                guest_playback::receive(&app, &s, item, quality);
            }
            Ok(Response::QualityResolved(item, max_height)) => {
                playback_ui::resolved(&app, &s, item, max_height);
            }
            Err(e) => {
                s.focus_intent
                    .cancel(focus_intent::Scope::Guest(s.worker.borrow().generation()));
                app.set_status(e.to_string().into());
                guest_ui::failed(&app, &s, s.worker.borrow().generation(), e);
                guest_recovery::failed(&app, &s, s.worker.borrow().generation(), e);
                guest_playback::resolution_failed(&app, &s, s.worker.borrow().generation(), e);
            }
        }
        playback_ui::maybe_refresh(&app, &s);
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.window().on_winit_window_event(move |_, event| {
        if let Some(counter) = &s.geometry_events
            && matches!(
                event,
                winit::event::WindowEvent::Resized(_)
                    | winit::event::WindowEvent::ScaleFactorChanged { .. }
                    | winit::event::WindowEvent::Moved(_)
            )
        {
            counter.set(counter.get().saturating_add(1));
        }
        fixture_quiescence::window_event(&s, event);
        if let Some(app) = weak.upgrade() {
            if !app.get_pip_available()
                && !s.native_child.enabled
                && matches!(
                    event,
                    winit::event::WindowEvent::Focused(true)
                        | winit::event::WindowEvent::Resized(_)
                )
            {
                app.set_pip_available(picture_in_picture::availability(app.window()).is_ok());
            }
            if s.pip.active() {
                if native_fullscreen(&app) {
                    request_windowed(&app);
                } else if s.pip_exit_pending.get() {
                    exit_picture_in_picture(&app, &s);
                }
            }
            native_child::window_event(&app, &s, event);
            feed_focus::window_event(&app, &s, event);
        }
        // Match the pinned Slint backend's macOS activation workaround: Winit's
        // raw Focused value can disagree with the native window (winit#4371).
        if let winit::event::WindowEvent::Focused(raw) = event {
            let focused = if cfg!(target_os = "macos") {
                weak.upgrade()
                    .and_then(|app| app.window().with_winit_window(|w| w.has_focus()))
                    .unwrap_or(*raw)
            } else {
                *raw
            };
            s.focus_intent
                .window_event(&winit::event::WindowEvent::Focused(focused));
        } else {
            s.focus_intent.window_event(event);
        }
        if matches!(event, winit::event::WindowEvent::Focused(_))
            && let Some(app) = weak.upgrade()
        {
            focus_intent::startup_ready(&app, &s);
        }
        if !s.presentation_ready.get()
            && matches!(
                event,
                winit::event::WindowEvent::Focused(true)
                    | winit::event::WindowEvent::Occluded(false)
                    | winit::event::WindowEvent::Resized(_)
            )
        {
            s.presentation_retry.set(true);
            if let Some(app) = weak.upgrade() {
                app.window()
                    .with_winit_window(|window| window.request_redraw());
            }
        }
        if let winit::event::WindowEvent::ThemeChanged(theme) = event
            && let Some(app) = weak.upgrade()
        {
            app.set_system_dark(*theme == winit::window::Theme::Dark);
        }
        if let winit::event::WindowEvent::Occluded(hidden) = event {
            eprintln!(
                "window occluded={hidden} at_ms={}",
                s.started.elapsed().as_millis()
            );
            s.hidden.set(*hidden);
            // Window policy is separate from accepted user pause intent. An
            // observed pause property may still precede an asynchronous command.
            let result = s.player.set_occluded(*hidden);
            if let Some(app) = weak.upgrade() {
                report(&app, result);
                update(&app, &s);
                if !hidden {
                    app.window().request_redraw();
                }
            }
        }
        EventResult::Propagate
    });
    let mut presenter: Option<GlPresenter> = None;
    let mut startup_media = Some(StartupMedia {
        local,
        subtitle,
        paused: start_paused || minimized,
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.window().set_rendering_notifier(move |stage, api| {
        if matches!(stage, slint::RenderingState::RenderingTeardown) {
            native_child::teardown(&s);
            s.clock_ui.invalidate();
            s.presentation_ready.set(false);
            s.presentation_retry.set(false);
            if let Some(app) = weak.upgrade() {
                app.set_video_texture(slint::Image::default());
            }
            if let Some(p) = presenter.take() {
                eprintln!("render stats: {:?}", p.stats());
                drop(p);
            }
            return;
        }
        let Some(app) = weak.upgrade() else { return };
        match stage {
            slint::RenderingState::RenderingSetup => {
                presenter = setup_presenter(&app, &s, api, &mut startup_media);
            }
            slint::RenderingState::BeforeRendering => {
                // Explicit fixture evidence only. A completion wake may precede
                // the first valid layout; observe that existing render without
                // requesting another redraw. No presenter/worker borrow spans
                // lazy geometry evaluation, and reported generations exit early.
                if s.library_fixture.is_some() {
                    fixture_quiescence::observe(&app, &s);
                }
                // Retry once only after a native reactivation/resize event. No
                // polling loop or renderer replacement on ordinary UI updates.
                if presenter.is_none() && s.presentation_retry.replace(false) {
                    presenter = setup_presenter(&app, &s, api, &mut startup_media);
                }
                if let Some(values) = s.clock_ui.take_for_render(|| {
                    (s.presentation_ready.get()
                        && app.get_loaded()
                        && app.get_progress_visible()
                        && !s.hidden.get())
                    .then(|| s.player.clock_identity())
                    .flatten()
                }) {
                    apply_clock(&app, &s, values);
                }
                s.draw_callbacks.set(s.draw_callbacks.get() + 1);
                if s.native_child.enabled {
                    native_child::before_render(&app, &s);
                }
                if let Some(p) = presenter.as_mut() {
                    let scale = app.window().scale_factor();
                    // SAFETY: current notifier context, texture never leaves this window.
                    match unsafe {
                        p.render(
                            (app.get_video_width() * scale).max(1.) as u32,
                            (app.get_video_height() * scale).max(1.) as u32,
                            app.get_loaded(),
                        )
                    } {
                        Ok(Some(image)) => {
                            app.set_video_texture(image);
                        }
                        Ok(None) => {}
                        Err(e) => {
                            if app.get_loaded() {
                                eprintln!("presentation failed: {e}");
                                let _ = s.player.stop();
                                app.set_loaded(false);
                                app.set_video_texture(slint::Image::default());
                            }
                            app.set_status(e.to_string().into());
                        }
                    }
                }
            }
            slint::RenderingState::AfterRendering => {
                if let Some(p) = presenter.as_mut() {
                    unsafe {
                        p.after_render();
                    }
                }
            }
            slint::RenderingState::RenderingTeardown => {
                app.set_video_texture(slint::Image::default());
                if let Some(p) = presenter.take() {
                    eprintln!("render stats: {:?}", p.stats());
                    drop(p);
                }
            }
            _ => {}
        }
    })?;
    if local_startup {
        state
            .focus_intent
            .arm_local_startup(app.get_search_active());
    } else if url.is_none() {
        state
            .focus_intent
            .arm_browse_startup(app.get_search_active());
    }
    if state.library_fixture.is_some() {
        // The normal startup Library/PlaylistPage replies choose and query the
        // first real collection. Its acknowledged page populates the fixture feed.
        app.set_page(1);
        app.global::<LibraryUi>().set_tab(0);
        library_fixture::bind(&app, &state);
    }
    app.show()?;
    app.window().with_winit_window(|w| {
        w.focus_window();
        if let Some(theme) = w.theme() {
            app.set_system_dark(theme == winit::window::Theme::Dark);
        }
    });
    focus_intent::startup_ready(&app, &state);
    if minimized {
        app.window().with_winit_window(|w| w.set_minimized(true));
        state.hidden.set(true);
    }
    if let Some(url) = url {
        playback_preferences::startup(&app, &state, url.into());
    }
    let mut timers = Vec::new();
    if ui_page.is_some() || ui_theme.is_some() || ui_size.is_some() {
        // Explicit visual-validation controls. The single delayed action lets
        // initial stored preferences arrive; it never writes preferences or
        // credentials and does not simulate acceptance of account consent.
        let weak = app.as_weak();
        let timer = Timer::default();
        timer.start(TimerMode::SingleShot, Duration::from_secs(1), move || {
            if let Some(app) = weak.upgrade() {
                if let Some(page) = ui_page {
                    app.invoke_navigate(page);
                }
                if let Some(theme) = ui_theme {
                    app.set_theme(theme);
                }
                if let Some((width, height)) = ui_size {
                    app.window()
                        .set_size(slint::LogicalSize::new(width as f32, height as f32));
                }
            }
        });
        timers.push(timer);
    }
    let snapshot_thread = Rc::new(RefCell::new(None));
    if let Some(path) = options.snapshot {
        // Explicit development capture only. Never used in the media path or
        // performance runs: this one-shot operation does read pixels back.
        let weak = app.as_weak();
        let output = snapshot_thread.clone();
        let timer = Timer::default();
        // Save's explicit capture occurs after its committed acknowledgement,
        // while the shared popup is still open. Ordinary captures remain at15s.
        let seconds = if options.save_smoke {
            37
        } else if options.pip_smoke {
            6
        } else {
            15
        };
        timer.start(
            TimerMode::SingleShot,
            Duration::from_secs(seconds),
            move || {
                let Some(app) = weak.upgrade() else { return };
                match app.window().take_snapshot() {
                    Ok(pixels) => {
                        let width = pixels.width();
                        let height = pixels.height();
                        let bytes = pixels.as_bytes().to_vec();
                        let path = path.clone();
                        *output.borrow_mut() =
                            Some(std::thread::spawn(move || -> Result<(), String> {
                                let mut file = std::fs::OpenOptions::new()
                                    .create_new(true)
                                    .write(true)
                                    .open(path)
                                    .map_err(|_| "snapshot destination unavailable")?;
                                let image = image::RgbaImage::from_raw(width, height, bytes)
                                    .ok_or("invalid snapshot")?;
                                image
                                    .write_to(&mut file, image::ImageFormat::Png)
                                    .map_err(|_| "snapshot encoding failed".to_owned())
                            }));
                    }
                    Err(_) => eprintln!("diagnostic snapshot unavailable"),
                }
            },
        );
        timers.push(timer);
    }
    if minimized {
        // Initial macOS window activation can arrive after show(). Apply the
        // diagnostic state once the event loop is running, then verify it.
        let weak = app.as_weak();
        let timer = Timer::default();
        timer.start(TimerMode::SingleShot, Duration::from_secs(1), move || {
            if let Some(app) = weak.upgrade() {
                app.window().with_winit_window(|w| w.set_minimized(true));
            }
        });
        timers.push(timer);
        let weak = app.as_weak();
        let timer = Timer::default();
        timer.start(TimerMode::SingleShot, Duration::from_secs(5), move || {
            if let Some(app) = weak.upgrade() {
                let actual = app
                    .window()
                    .with_winit_window(|w| w.is_minimized())
                    .flatten();
                eprintln!("minimized diagnostic: observed={actual:?}");
                assert_eq!(
                    actual,
                    Some(true),
                    "minimized diagnostic state was not verified"
                );
            }
        });
        timers.push(timer);
    }
    if options.diagnostics {
        for seconds in [5, 10, 25, 70] {
            let s = state.clone();
            let weak = app.as_weak();
            let timer = Timer::default();
            timer.start(TimerMode::SingleShot,Duration::from_secs(seconds),move || {
                eprintln!("checkpoint seconds={seconds}: {:?}; ui_draws={} catalog_changes={} catalog_resets={}",s.player.snapshot(),s.draw_callbacks.get(),s.model.changes.get(),s.model.resets.get());
                if let Some(app) = weak.upgrade() {
                    presentation_diagnostics::record(seconds, &app, &s);
                }
            });
            timers.push(timer);
        }
    }
    if smoke || demo_related {
        state.model.replace(
            (0..30)
                .map(|i| VideoRow {
                    title: format!("TEST FIXTURE — catalog row {i}").into(),
                    channel: "Synthetic content; lifecycle test only".into(),
                    id: if options.related_focus_check {
                        format!("f{i:010}").into()
                    } else {
                        "".into()
                    },
                    ..VideoRow::default()
                })
                .collect(),
        );
        state.groups.replace(&state.model);
        app.set_status("TEST FIXTURE MODE — synthetic catalog rows for invalidation checks".into());
    }
    let soak_diagnostic = soak_config
        .map(|config| soak_smoke::Smoke::start(&app, &state, config))
        .transpose()?;
    let preferences_diagnostic = options
        .preferences_smoke
        .map(|phase| preferences_smoke::Smoke::start(&app, &state, phase));
    let library_resource_diagnostic = options
        .library_resource_smoke
        .then(|| library_resource_smoke::Smoke::start(&app, &state));
    let related_focus_diagnostic = options
        .related_focus_check
        .then(|| related_focus_smoke::Smoke::start(&app, &state));
    let save_diagnostic =
        save_smoke_video.map(|video| save_smoke::Smoke::start(&app, &state, video));
    let recovery_diagnostic = options
        .recovery_smoke
        .then(|| recovery_smoke::Smoke::start(&app, &state));
    let pip_diagnostic = options.pip_smoke.then(|| {
        app.set_diagnostic_fixture_label("TEST FIXTURE — local picture-in-picture exercise".into());
        pip_smoke::Smoke::start(&app, &state)
    });
    let motion_diagnostic = smoke.then(|| motion_smoke::Smoke::start(&app, &state));
    let focus_smoke_complete = Rc::new(Cell::new(!smoke));
    let mute_smoke_complete = Rc::new(Cell::new(!smoke));
    if smoke {
        let initial_volume = Rc::new(Cell::new(None));
        for seconds in [4, 6, 8, 10] {
            let weak = app.as_weak();
            let s = state.clone();
            let volume = initial_volume.clone();
            let complete = mute_smoke_complete.clone();
            let timer = Timer::default();
            timer.start(
                TimerMode::SingleShot,
                Duration::from_secs(seconds),
                move || {
                    let Some(app) = weak.upgrade() else {
                        return;
                    };
                    let snapshot = s.player.snapshot();
                    assert!(
                        snapshot.mute_observed && app.get_mute_known(),
                        "native mute observation unavailable"
                    );
                    assert_eq!(
                        app.get_muted(),
                        snapshot.muted,
                        "mute UI differs from native state"
                    );
                    match seconds {
                        4 => {
                            assert!(!snapshot.muted, "local smoke must start unmuted");
                            volume.set(Some(snapshot.volume));
                            app.invoke_mute();
                        }
                        6 | 8 => {
                            assert!(snapshot.muted, "mute callback was not observed");
                            assert_eq!(
                                Some(snapshot.volume),
                                volume.get(),
                                "mute changed configured volume"
                            );
                            if seconds == 8 {
                                app.invoke_mute();
                            }
                        }
                        10 => {
                            assert!(!snapshot.muted, "unmute callback was not observed");
                            assert_eq!(
                                Some(snapshot.volume),
                                volume.get(),
                                "unmute changed configured volume"
                            );
                            assert_eq!(snapshot.file_loads, 1, "mute recreated playback");
                            complete.set(true);
                        }
                        _ => unreachable!(),
                    }
                    eprintln!(
                        "mute smoke seconds={seconds} known={} muted={} volume={}",
                        snapshot.mute_observed, snapshot.muted, snapshot.volume
                    );
                },
            );
            timers.push(timer);
        }
        app.set_show_info(true);
        for (seconds, action) in [
            (3, 0),
            (5, 1),
            (7, 2),
            (8, 9),
            (9, 3),
            (11, 4),
            (12, 10),
            (13, 5),
            (14, 6),
            (16, 7),
            (18, 8),
        ] {
            let weak = app.as_weak();
            let s = state.clone();
            let exit_attempts = fullscreen_exit_attempts.clone();
            let timer = Timer::default();
            timer.start(
                TimerMode::SingleShot,
                Duration::from_secs(seconds),
                move || {
                    if let Some(app) = weak.upgrade() {
                        match action {
                            0 => {
                                assert!(app.get_player_active(), "explicit local startup did not focus the player before smoke input");
                                app.invoke_focus_player();
                                app.window().dispatch_event(
                                    slint::platform::WindowEvent::KeyPressed { text: "k".into() },
                                );
                                app.window().dispatch_event(
                                    slint::platform::WindowEvent::KeyReleased { text: "k".into() },
                                );
                            }
                            1 => report(&app, s.player.seek(20.)),
                            2 => {
                                report(&app, s.player.set_paused(false));
                                app.window().set_size(slint::LogicalSize::new(760., 600.));
                            }
                            3 => {
                                assert!(
                                    app.get_visible_related_count() > 0,
                                    "compact related rows remain unreachable"
                                );
                                assert!(app.get_watch_offset() < 0.);
                                assert!(
                                    !s.progress.running(),
                                    "offscreen transport still polls progress"
                                );
                                app.invoke_fullscreen();
                            }
                            4 => {
                                assert_eq!(
                                    app.get_watch_offset(),
                                    0.,
                                    "fullscreen inherited watch scroll"
                                );
                                // focus-player() intentionally reveals the player
                                // and clears the saved normal-window scroll. A real
                                // click focuses this already-visible fullscreen
                                // surface without erasing the offset we must restore.
                                let position = slint::LogicalPosition::new(
                                    app.get_video_window_x() + app.get_video_width() / 2.,
                                    app.get_video_window_y() + app.get_video_height() / 2.,
                                );
                                app.window().dispatch_event(
                                    slint::platform::WindowEvent::PointerPressed {
                                        position,
                                        button: slint::platform::PointerEventButton::Left,
                                    },
                                );
                                app.window().dispatch_event(
                                    slint::platform::WindowEvent::PointerReleased {
                                        position,
                                        button: slint::platform::PointerEventButton::Left,
                                    },
                                );
                                assert!(app.get_player_active(), "fullscreen video click did not focus the player");
                                let counter = exit_attempts.as_ref().expect("keyboard smoke counter");
                                let before_escape = counter.get();
                                app.window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: slint::platform::Key::Escape.into() });
                                app.window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text: slint::platform::Key::Escape.into() });
                                assert_eq!(counter.get(), before_escape + 1, "player Escape did not invoke fullscreen exit exactly once");
                                assert!(!app.get_fullscreen_active(), "player Escape did not leave fullscreen");
                            }
                            5 => {
                                app.invoke_focus_player();
                                app.window().dispatch_event(
                                    slint::platform::WindowEvent::KeyPressed { text: "/".into() },
                                );
                                app.window().dispatch_event(
                                    slint::platform::WindowEvent::KeyReleased { text: "/".into() },
                                );
                                assert!(
                                    app.get_search_active(),
                                    "search shortcut did not focus the input"
                                );
                                let counter = exit_attempts.as_ref().expect("keyboard smoke counter");
                                let before_escape = counter.get();
                                app.window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: slint::platform::Key::Escape.into() });
                                app.window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text: slint::platform::Key::Escape.into() });
                                assert!(app.get_search_active(), "Escape unexpectedly removed editor focus");
                                assert_eq!(counter.get(), before_escape, "Escape while editing invoked a playback fullscreen command");
                                eprintln!("keyboard smoke: editor Escape produced zero playback fullscreen callbacks");
                                let scope =
                                    focus_intent::Scope::Guest(s.worker.borrow().generation());
                                s.focus_intent.arm(scope, app.get_search_active());
                                app.window().dispatch_event(
                                    slint::platform::WindowEvent::KeyPressed { text: "f".into() },
                                );
                                app.window().dispatch_event(
                                    slint::platform::WindowEvent::KeyReleased { text: "f".into() },
                                );
                                assert!(
                                    !app.get_fullscreen_active(),
                                    "playback shortcut intercepted text input"
                                );
                                focus_intent::apply(&app, &s, scope);
                                assert!(app.get_search_active(), "late focus stole search typing");
                                s.focus_intent.arm(scope, app.get_search_active());
                                app.window().dispatch_event(
                                    slint::platform::WindowEvent::KeyPressed {
                                        text: slint::platform::Key::Tab.into(),
                                    },
                                );
                                app.window().dispatch_event(
                                    slint::platform::WindowEvent::KeyReleased {
                                        text: slint::platform::Key::Tab.into(),
                                    },
                                );
                                report(&app, s.player.cycle_subtitles());
                                app.set_controls_visible(false);
                                update(&app, &s);
                            }
                            6 => {
                                app.window().with_winit_window(|w| w.set_minimized(true));
                            }
                            7 => {
                                app.window().with_winit_window(|w| {
                                    w.set_minimized(false);
                                    w.focus_window();
                                });
                            }
                            8 => {
                                app.set_controls_visible(true);
                                update(&app, &s);
                            }
                            9 => {
                                assert!(app.get_related_height() >= 318.);
                                assert_eq!(
                                    app.get_visible_related_count(),
                                    0,
                                    "offscreen related rows requested thumbnails"
                                );
                                app.window().dispatch_event(
                                    slint::platform::WindowEvent::PointerScrolled {
                                        position: slint::LogicalPosition::new(150., 200.),
                                        delta_x: 0.,
                                        delta_y: -800.,
                                    },
                                );
                            }
                            10 => {
                                assert!(
                                    app.get_watch_offset() < 0.,
                                    "watch scroll was not restored"
                                );
                                assert!(app.get_visible_related_count() > 0);
                                assert_eq!(
                                    s.player.snapshot().file_loads,
                                    1,
                                    "scroll/fullscreen recreated media"
                                );
                            }
                            _ => unreachable!(),
                        }
                        for x in 0..100 {
                            app.window().dispatch_event(
                                slint::platform::WindowEvent::PointerMoved {
                                    position: slint::LogicalPosition::new(10. + x as f32, 200.),
                                },
                            );
                        }
                        assert_eq!(
                            s.model.changes.get(),
                            0,
                            "playback/pointer changed catalog rows"
                        );
                        assert_eq!(s.model.resets.get(), 1, "playback/pointer reset catalog");
                        eprintln!(
                            "smoke stage {action}: {:?}; catalog changes={} resets={}",
                            s.player.snapshot(),
                            s.model.changes.get(),
                            s.model.resets.get()
                        );
                    }
                },
            );
            timers.push(timer);
        }
        // Delayed completion exercises focus intent without a fabricated
        // provider result. Slint settles focus-change callbacks between stages.
        for milliseconds in [13_200, 13_400, 13_600, 13_800] {
            let weak = app.as_weak();
            let s = state.clone();
            let complete = focus_smoke_complete.clone();
            let timer = Timer::default();
            timer.start(
                TimerMode::SingleShot,
                Duration::from_millis(milliseconds),
                move || {
                    let Some(app) = weak.upgrade() else { return };
                    let scope = focus_intent::Scope::Guest(s.worker.borrow().generation());
                    match milliseconds {
                        13_200 => {
                            assert!(!app.get_search_active(), "Tab did not leave search");
                            assert!(
                                !app.get_player_active(),
                                "Tab unexpectedly targeted the player"
                            );
                            focus_intent::apply(&app, &s, scope);
                            assert!(!app.get_player_active(), "delayed playback stole Tab focus");
                            app.invoke_focus_search();
                        }
                        13_400 => {
                            assert!(app.get_search_active());
                            s.focus_intent.arm(scope, app.get_search_active());
                        }
                        13_600 => {
                            focus_intent::apply(&app, &s, scope);
                            assert!(
                                app.get_player_active(),
                                "unchanged playback intent did not focus player"
                            );
                            assert_eq!(
                                s.player.snapshot().file_loads,
                                1,
                                "focus recreated the player"
                            );
                        }
                        13_800 => {
                            app.invoke_focus_search();
                            focus_intent::apply(&app, &s, scope);
                            assert!(app.get_search_active(), "consumed focus intent was reused");
                            app.invoke_focus_player();
                            complete.set(true);
                        }
                        _ => unreachable!(),
                    }
                    eprintln!(
                        "focus smoke stage_ms={milliseconds} search={} player={}",
                        app.get_search_active(),
                        app.get_player_active()
                    );
                },
            );
            timers.push(timer);
        }
        // Check observed engine state after async commands have had time to complete.
        // This specifically catches the paused-seek progress regression.
        let s = state.clone();
        let timer = Timer::default();
        timer.start(TimerMode::SingleShot, Duration::from_secs(6), move || {
            let snapshot = s.player.snapshot();
            assert!(snapshot.paused, "smoke pause was not observed");
            assert!(
                (snapshot.position - 20.).abs() < 0.25,
                "paused seek did not update position"
            );
            assert!(snapshot.error.is_none(), "smoke engine returned an error");
            eprintln!("smoke assertion: paused seek reached 20 seconds");
        });
        timers.push(timer);
    }
    let minimized_before_quit = Rc::new(Cell::new(!minimized));
    let comments_diagnostic =
        comments_smoke.then(|| comments_ui::Smoke::start(&app, &state, comments_snapshots));
    let captions_diagnostic = captions_smoke.then(|| caption_ui::Smoke::start(&app, &state));
    let clear_diagnostic = options
        .clear_local_smoke
        .then(|| clear_smoke::Smoke::start(&app, &state));
    let library_diagnostic = options
        .library_smoke
        .then(|| library_smoke::Smoke::start(&app, &state));
    let native_child_diagnostic = options
        .native_video_child_smoke
        .then(|| native_child_smoke::Smoke::start(&app, &state));
    let refresh_diagnostic = refresh_smoke.then(|| playback_ui::Smoke::start(&app, &state));
    let handoff_diagnostic = handoff_audio
        .zip(handoff_video)
        .map(|(audio, video)| handoff_smoke::Smoke::start(&app, &state, video, audio));
    let quality_verified = Rc::new(Cell::new(!quality_smoke));
    if quality_smoke {
        let paused_position = Rc::new(Cell::new(0.));
        for stage in [30, 50, 55, 75] {
            let weak = app.as_weak();
            let s = state.clone();
            let position = paused_position.clone();
            let verified = quality_verified.clone();
            let timer = Timer::default();
            timer.start(
                TimerMode::SingleShot,
                Duration::from_secs(stage),
                move || {
                    let Some(app) = weak.upgrade() else { return };
                    let snapshot = s.player.snapshot();
                    assert!(snapshot.error.is_none(), "quality smoke media failure");
                    assert!(
                        app.get_remote_video() && snapshot.height > 0,
                        "quality smoke requires resolved, embedded guest video"
                    );
                    match stage {
                        30 => {
                            assert!(
                                !snapshot.paused && !app.get_busy(),
                                "initial guest playback did not become ready"
                            );
                            position.set(snapshot.position);
                            app.invoke_quality(1);
                        }
                        50 => {
                            assert_eq!(s.quality_index.get(), 1, "720p ceiling was not resolved");
                            assert!(
                                snapshot.height <= 720 && snapshot.position >= position.get(),
                                "quality change lost position or exceeded requested ceiling"
                            );
                            report(&app, s.player.set_paused(true));
                        }
                        55 => {
                            assert!(snapshot.paused, "pause was not observed");
                            position.set(snapshot.position);
                            app.invoke_quality(2);
                        }
                        _ => {
                            assert_eq!(s.quality_index.get(), 2, "480p ceiling was not resolved");
                            assert!(
                                snapshot.paused && snapshot.height <= 480,
                                "quality change did not preserve pause/ceiling"
                            );
                            assert!(
                                (snapshot.position - position.get()).abs() < 0.5,
                                "paused quality change lost position"
                            );
                            verified.set(true);
                        }
                    }
                    eprintln!(
                        "quality smoke stage={stage} height={} position={:.3} paused={} decoder={}",
                        snapshot.height, snapshot.position, snapshot.paused, snapshot.hwdec_current
                    );
                },
            );
            timers.push(timer);
        }
    }
    if let Some(seconds) = options.quit_after {
        let timer = Timer::default();
        let weak = app.as_weak();
        let verified = minimized_before_quit.clone();
        let started = state.started;
        timer.start(
            TimerMode::SingleShot,
            Duration::from_secs(seconds),
            move || {
                if minimized && let Some(app) = weak.upgrade() {
                    let actual = app
                        .window()
                        .with_winit_window(|w| w.is_minimized())
                        .flatten();
                    eprintln!(
                        "minimized diagnostic before quit: observed={actual:?} at_ms={}",
                        started.elapsed().as_millis()
                    );
                    verified.set(actual == Some(true));
                }
                let _ = slint::quit_event_loop();
            },
        );
        timers.push(timer);
    }
    slint::run_event_loop()?;
    library_ui::flush_volume_save(&app, &state);
    state.account_ui.stop_picker();
    state.library_ui.stop_picker();
    state.progress.stop();
    state.clock_ui.invalidate();
    native_child::hide(&state);
    let _ = state.player.stop();
    app.set_video_texture(slint::Image::default());
    app.hide()?;
    native_child::teardown(&state);
    if options.stage_progress {
        eprintln!("clock staging counters: {:?}", state.clock_ui.stats());
    }
    eprintln!(
        "session counters: ui={} catalog_changes={} catalog_resets={} redraw_requests={} draw_callbacks={} media={:?}",
        state.ui_assignments.get(),
        state.model.changes.get(),
        state.model.resets.get(),
        state.redraw_requests.get(),
        state.draw_callbacks.get(),
        state.player.snapshot()
    );
    if let Some(thread) = snapshot_thread.borrow_mut().take() {
        thread
            .join()
            .map_err(|_| "snapshot worker failed")?
            .map_err(std::io::Error::other)?;
    }
    if let Some(diagnostic) = comments_diagnostic {
        diagnostic.finish().map_err(std::io::Error::other)?;
    }
    if let Some(diagnostic) = captions_diagnostic {
        diagnostic.finish().map_err(std::io::Error::other)?;
    }
    if let Some(diagnostic) = clear_diagnostic {
        diagnostic.finish().map_err(std::io::Error::other)?;
    }
    if let Some(diagnostic) = library_diagnostic {
        diagnostic.finish().map_err(std::io::Error::other)?;
    }
    if let Some(diagnostic) = native_child_diagnostic {
        diagnostic.finish().map_err(std::io::Error::other)?;
    }
    if let Some(diagnostic) = related_focus_diagnostic {
        diagnostic.finish().map_err(std::io::Error::other)?;
    }
    if let Some(diagnostic) = pip_diagnostic {
        diagnostic.finish().map_err(std::io::Error::other)?;
    }
    if let Some(diagnostic) = recovery_diagnostic {
        diagnostic.finish().map_err(std::io::Error::other)?;
    }
    if let Some(diagnostic) = save_diagnostic {
        diagnostic.finish().map_err(std::io::Error::other)?;
    }
    if let Some(diagnostic) = library_resource_diagnostic {
        diagnostic.finish().map_err(std::io::Error::other)?;
    }
    if state.library_fixture.is_some() {
        let (first, end) = state.thumbnail_range.get();
        let visible_first = app.get_feed_thumbnail_first().max(0) as usize;
        let visible_end = app.get_feed_thumbnail_end().max(0) as usize;
        let visible_ready = (visible_first..visible_end)
            .filter(|row| {
                state
                    .model
                    .row_data(*row)
                    .is_some_and(|row| row.thumbnail_ready)
            })
            .count();
        eprintln!(
            "library resource fixture final: model_rows={} near_viewport_rows={} viewport_intersecting_ready_thumbnails={} actual_compositor_visibility=not_measured thumbnails={:?} catalog_changes={} catalog_resets={} group_child_changes={} library_notifications={:?}",
            state.model.row_count(),
            end.saturating_sub(first),
            visible_ready,
            state.thumbnails.borrow().statistics(),
            state.model.changes.get(),
            state.model.resets.get(),
            state.groups.child_changes.get(),
            library_ui::notification_counts(&state)
        );
    }
    if let Some(diagnostic) = refresh_diagnostic {
        diagnostic.finish().map_err(std::io::Error::other)?;
    }
    if let Some(diagnostic) = preferences_diagnostic {
        diagnostic.finish().map_err(std::io::Error::other)?;
    }
    if let Some(diagnostic) = soak_diagnostic {
        diagnostic.finish().map_err(std::io::Error::other)?;
    }
    if let Some(diagnostic) = motion_diagnostic {
        diagnostic.finish().map_err(std::io::Error::other)?;
    }
    if let Some(diagnostic) = handoff_diagnostic {
        diagnostic.finish().map_err(std::io::Error::other)?;
    }
    drop(timers);
    drop(app);
    drop(state);
    caption_cleanup.finish().map_err(std::io::Error::other)?;
    // Query before quitting: native event-loop teardown can restore the window.
    if !minimized_before_quit.get() {
        return Err("minimized state was not maintained through the diagnostic".into());
    }
    if !quality_verified.get() {
        return Err("quality smoke ended before its native assertions completed".into());
    }
    if !focus_smoke_complete.get() {
        return Err("local smoke ended before delayed-focus assertions completed".into());
    }
    if !mute_smoke_complete.get() {
        return Err("local smoke ended before observed mute/unmute assertions completed".into());
    }
    Ok(())
}
