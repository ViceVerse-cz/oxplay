// SPDX-License-Identifier: GPL-3.0-or-later
//! Startup-only argument validation. Errors never echo supplied values.
use std::{collections::HashSet, ffi::OsString, path::PathBuf};

#[derive(Default)]
pub struct Options {
    pub help: bool,
    pub local: Option<PathBuf>,
    pub subtitle: Option<PathBuf>,
    pub handoff_audio: Option<PathBuf>,
    pub yt_dlp: Option<PathBuf>,
    pub deno: Option<PathBuf>,
    pub data_root: Option<PathBuf>,
    pub snapshot: Option<PathBuf>,
    pub comments_snapshots: Option<PathBuf>,
    pub url: Option<String>,
    pub search: Option<String>,
    pub ui_page: Option<i32>,
    pub ui_theme: Option<i32>,
    pub ui_size: Option<(u32, u32)>,
    pub quit_after: Option<u64>,
    pub soak_minutes: Option<u32>,
    pub preferences_smoke: Option<crate::preferences_smoke::Phase>,
    pub library_resource_fixture: Option<PathBuf>,
    pub library_resource_smoke: bool,
    pub related_focus_check: bool,
    pub save_smoke: bool,
    pub recovery_smoke: bool,
    pub pip_smoke: bool,
    pub home_smoke: bool,
    pub paused: bool,
    pub minimized: bool,
    pub smoke: bool,
    pub quality_smoke: bool,
    pub comments_smoke: bool,
    pub captions_smoke: bool,
    pub clear_local_smoke: bool,
    pub library_smoke: bool,
    pub library_keyboard_smoke: bool,
    pub collection_window_smoke: bool,
    pub refresh_smoke: bool,
    pub demo_related: bool,
    pub ui_cache: bool,
    pub search_cache: bool,
    pub related_cache: bool,
    pub stage_progress: bool,
    pub diagnostics: bool,
    pub scoped_media: bool,
    pub native_video_child: bool,
    pub native_video_child_smoke: bool,
}

impl Options {
    pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self, &'static str> {
        let mut options = Self::default();
        let mut args = args.into_iter();
        let mut seen = HashSet::new();
        while let Some(argument) = args.next() {
            let key = argument.to_str().ok_or("Option names must be UTF-8")?;
            if !seen.insert(argument.clone()) {
                return Err("An option was supplied more than once");
            }
            match key {
                "--help" => options.help = true,
                "--paused" => options.paused = true,
                "--minimized" => options.minimized = true,
                "--smoke-test" => options.smoke = true,
                "--quality-smoke-test" => options.quality_smoke = true,
                "--comments-smoke-test" => options.comments_smoke = true,
                "--captions-smoke-test" => options.captions_smoke = true,
                "--clear-local-smoke-test" => options.clear_local_smoke = true,
                "--library-smoke-test" => options.library_smoke = true,
                "--library-keyboard-smoke-test" => options.library_keyboard_smoke = true,
                "--collection-window-smoke-test" => options.collection_window_smoke = true,
                "--library-resource-smoke-test" => options.library_resource_smoke = true,
                "--related-focus-check" => options.related_focus_check = true,
                "--save-smoke-test" => options.save_smoke = true,
                "--recovery-smoke-test" => options.recovery_smoke = true,
                "--pip-smoke-test" => options.pip_smoke = true,
                "--home-smoke-test" => options.home_smoke = true,
                "--refresh-smoke-test" => options.refresh_smoke = true,
                "--demo-related" => options.demo_related = true,
                "--ui-cache" => options.ui_cache = true,
                "--search-cache" => options.search_cache = true,
                "--related-cache" => options.related_cache = true,
                "--stage-progress" => options.stage_progress = true,
                "--no-ui-cache" => {}
                "--diagnostics" => options.diagnostics = true,
                "--scoped-media" => options.scoped_media = true,
                "--native-video-child" => options.native_video_child = true,
                "--native-video-child-smoke-test" => options.native_video_child_smoke = true,
                "--local"
                | "--subtitle"
                | "--handoff-audio"
                | "--yt-dlp"
                | "--deno"
                | "--data-root"
                | "--library-resource-fixture"
                | "--snapshot"
                | "--comments-snapshots"
                | "--url"
                | "--search"
                | "--ui-page"
                | "--ui-theme"
                | "--ui-size"
                | "--quit-after"
                | "--soak-minutes"
                | "--preferences-smoke-test" => {
                    let value = args.next().ok_or("An option is missing its value")?;
                    if value.is_empty() || value.to_string_lossy().starts_with("--") {
                        return Err(
                            "An option is missing its value; use ./ for paths beginning with --",
                        );
                    }
                    match key {
                        "--local" => options.local = Some(value.into()),
                        "--subtitle" => options.subtitle = Some(value.into()),
                        "--handoff-audio" => options.handoff_audio = Some(value.into()),
                        "--yt-dlp" => options.yt_dlp = Some(value.into()),
                        "--deno" => options.deno = Some(value.into()),
                        "--data-root" => options.data_root = Some(value.into()),
                        "--library-resource-fixture" => {
                            options.library_resource_fixture = Some(value.into())
                        }
                        "--snapshot" => options.snapshot = Some(value.into()),
                        "--comments-snapshots" => options.comments_snapshots = Some(value.into()),
                        _ => {
                            let value = value
                                .into_string()
                                .map_err(|_| "Text option values must be UTF-8")?;
                            match key {
                                "--url" => options.url = Some(value),
                                "--search" => options.search = Some(value),
                                "--ui-page" => {
                                    options.ui_page = Some(match value.as_str() {
                                        "browse" => 0,
                                        "library" => 1,
                                        "settings" => 3,
                                        "account" => 4,
                                        _ => {
                                            return Err(
                                                "--ui-page expects browse, library, settings or account",
                                            );
                                        }
                                    })
                                }
                                "--ui-theme" => {
                                    options.ui_theme = Some(match value.as_str() {
                                        "system" => 0,
                                        "light" => 1,
                                        "dark" => 2,
                                        _ => {
                                            return Err("--ui-theme expects system, light or dark");
                                        }
                                    })
                                }
                                "--ui-size" => options.ui_size = Some(parse_size(&value)?),
                                "--preferences-smoke-test" => {
                                    options.preferences_smoke = Some(
                                        crate::preferences_smoke::Phase::parse(&value).ok_or(
                                            "--preferences-smoke-test expects write or verify",
                                        )?,
                                    );
                                }
                                "--soak-minutes" => {
                                    let minutes = value
                                        .parse::<u32>()
                                        .map_err(|_| "--soak-minutes expects whole minutes")?;
                                    if !(60..=240).contains(&minutes) {
                                        return Err(
                                            "--soak-minutes must be between 60 and 240 minutes",
                                        );
                                    }
                                    options.soak_minutes = Some(minutes);
                                }
                                "--quit-after" => {
                                    let seconds = value
                                        .parse::<u64>()
                                        .map_err(|_| "--quit-after expects whole seconds")?;
                                    if !(1..=86_400).contains(&seconds) {
                                        return Err(
                                            "--quit-after must be between 1 and 86400 seconds",
                                        );
                                    }
                                    options.quit_after = Some(seconds);
                                }
                                _ => unreachable!(),
                            }
                        }
                    }
                }
                _ => return Err("Unknown option or unexpected positional argument; see --help"),
            }
        }
        if options.ui_cache && seen.contains(std::ffi::OsStr::new("--no-ui-cache")) {
            return Err("--ui-cache and --no-ui-cache cannot be combined");
        }
        if [
            options.ui_cache,
            options.search_cache,
            options.related_cache,
        ]
        .into_iter()
        .filter(|enabled| *enabled)
        .count()
            > 1
        {
            return Err(
                "Choose only one diagnostic cache scope: --ui-cache, --search-cache, or --related-cache",
            );
        }
        if options.help {
            return Ok(options);
        }
        if options.home_smoke {
            const ALLOWED: &[&str] = &[
                "--home-smoke-test",
                "--data-root",
                "--quit-after",
                "--ui-size",
                "--ui-theme",
                "--snapshot",
            ];
            if options
                .data_root
                .as_ref()
                .is_none_or(|root| !root.is_absolute())
                || options.quit_after.is_some_and(|seconds| seconds != 40)
                || seen.iter().any(|key| {
                    !ALLOWED
                        .iter()
                        .any(|allowed| key == std::ffi::OsStr::new(allowed))
                })
            {
                return Err(
                    "--home-smoke-test requires a NEW absolute --data-root, a 40-second watchdog and no network, media or other diagnostic inputs",
                );
            }
            options.quit_after = Some(40);
        }
        if options.pip_smoke {
            const ALLOWED: &[&str] = &[
                "--pip-smoke-test",
                "--local",
                "--subtitle",
                "--data-root",
                "--quit-after",
                "--ui-size",
                "--ui-theme",
                "--snapshot",
            ];
            if options.local.is_none()
                || options
                    .data_root
                    .as_ref()
                    .is_none_or(|root| !root.is_absolute())
                || options.quit_after.is_some_and(|seconds| seconds != 25)
                || seen.iter().any(|key| {
                    !ALLOWED
                        .iter()
                        .any(|allowed| key == std::ffi::OsStr::new(allowed))
                })
            {
                return Err(
                    "--pip-smoke-test requires local video and a NEW absolute --data-root, with a 25-second watchdog and no online or other diagnostic inputs",
                );
            }
            options.quit_after = Some(25);
        }
        if options.recovery_smoke {
            const ALLOWED: &[&str] = &[
                "--recovery-smoke-test",
                "--yt-dlp",
                "--data-root",
                "--quit-after",
                "--ui-size",
                "--ui-theme",
                "--diagnostics",
                "--snapshot",
            ];
            if !cfg!(unix)
                || options
                    .data_root
                    .as_ref()
                    .is_none_or(|root| !root.is_absolute())
                || options.yt_dlp.as_deref().map(std::path::Path::as_os_str)
                    != Some(std::ffi::OsStr::new("/usr/bin/false"))
                || options.quit_after.is_some_and(|seconds| seconds != 20)
                || options.snapshot.as_ref().is_some_and(|path| {
                    path.parent() != options.data_root.as_deref()
                        || path.file_name().is_none()
                        || path.extension().is_none_or(|extension| extension != "png")
                })
                || seen.iter().any(|key| {
                    !ALLOWED
                        .iter()
                        .any(|allowed| key == std::ffi::OsStr::new(allowed))
                })
            {
                return Err(
                    "--recovery-smoke-test requires exactly --yt-dlp /usr/bin/false and NEW absolute --data-root, with a 20-second watchdog and no other content, account, helper or diagnostic inputs",
                );
            }
            options.quit_after = Some(20);
        }
        if options.collection_window_smoke {
            const ALLOWED: &[&str] = &[
                "--collection-window-smoke-test",
                "--data-root",
                "--quit-after",
                "--ui-size",
                "--ui-theme",
                "--snapshot",
            ];
            if !cfg!(unix)
                || options
                    .data_root
                    .as_ref()
                    .is_none_or(|root| !root.is_absolute())
                || options.quit_after.is_some_and(|seconds| seconds != 58)
                || options.snapshot.as_ref().is_some_and(|path| {
                    path.parent() != options.data_root.as_deref()
                        || path.extension().is_none_or(|extension| extension != "png")
                })
                || seen.iter().any(|key| {
                    !ALLOWED
                        .iter()
                        .any(|allowed| key == std::ffi::OsStr::new(allowed))
                })
            {
                return Err(
                    "--collection-window-smoke-test requires NEW absolute --data-root and its exact 58-second watchdog; no network, media, helpers or other diagnostics",
                );
            }
            options.quit_after = Some(58);
        }
        if options.library_keyboard_smoke {
            const ALLOWED: &[&str] = &[
                "--library-keyboard-smoke-test",
                "--data-root",
                "--quit-after",
                "--ui-size",
                "--ui-theme",
                "--snapshot",
            ];
            if !cfg!(unix)
                || options
                    .data_root
                    .as_ref()
                    .is_none_or(|root| !root.is_absolute())
                || options.quit_after.is_some_and(|seconds| seconds != 38)
                || options.snapshot.as_ref().is_some_and(|path| {
                    path.parent() != options.data_root.as_deref()
                        || path.extension().is_none_or(|extension| extension != "png")
                })
                || seen.iter().any(|key| {
                    !ALLOWED
                        .iter()
                        .any(|allowed| key == std::ffi::OsStr::new(allowed))
                })
            {
                return Err(
                    "--library-keyboard-smoke-test requires only NEW absolute --data-root and its exact38-second watchdog; no media, network, helper or other diagnostic inputs",
                );
            }
            options.quit_after = Some(38);
        }
        if options.save_smoke {
            const ALLOWED: &[&str] = &[
                "--save-smoke-test",
                "--url",
                "--data-root",
                "--quit-after",
                "--ui-size",
                "--ui-theme",
                "--diagnostics",
                "--snapshot",
            ];
            if !cfg!(unix)
                || options
                    .data_root
                    .as_ref()
                    .is_none_or(|root| !root.is_absolute())
                || options
                    .url
                    .as_deref()
                    .is_none_or(|url| serein_core::VideoId::from_url(url).is_err())
                || seen.iter().any(|key| {
                    !ALLOWED
                        .iter()
                        .any(|allowed| key == std::ffi::OsStr::new(allowed))
                })
                || options.quit_after.is_some_and(|seconds| seconds != 75)
                || options.snapshot.as_ref().is_some_and(|path| {
                    path.parent() != options.data_root.as_deref()
                        || path.file_name().is_none()
                        || path.extension().is_none_or(|extension| extension != "png")
                })
            {
                return Err(
                    "--save-smoke-test requires only a public video --url and NEW absolute --data-root; its watchdog is 75 seconds, with no other diagnostics, helpers or transport overrides",
                );
            }
            options.quit_after = Some(75);
        }
        if options.related_focus_check {
            const ALLOWED: &[&str] = &[
                "--related-focus-check",
                "--local",
                "--demo-related",
                "--data-root",
                "--quit-after",
            ];
            if !cfg!(unix)
                || options.local.is_none()
                || !options.demo_related
                || options
                    .data_root
                    .as_ref()
                    .is_none_or(|root| !root.is_absolute())
                || seen.iter().any(|key| {
                    !ALLOWED
                        .iter()
                        .any(|allowed| key == std::ffi::OsStr::new(allowed))
                })
                || options.quit_after.is_some_and(|seconds| seconds != 42)
            {
                return Err(
                    "--related-focus-check requires only the exact local synthetic MP4, --demo-related and NEW absolute --data-root; its watchdog is 42 seconds",
                );
            }
            options.quit_after = Some(42);
        }
        if options.native_video_child_smoke && !options.native_video_child {
            return Err("--native-video-child-smoke-test requires --native-video-child");
        }
        if options.native_video_child_smoke && options.paused {
            return Err(
                "Native-child lifecycle starts playing and controls its own pause checkpoints",
            );
        }
        if options.native_video_child {
            if !cfg!(target_os = "macos") {
                return Err("--native-video-child is an unqualified macOS-only diagnostic");
            }
            if options.local.is_none()
                || options
                    .data_root
                    .as_ref()
                    .is_none_or(|root| !root.is_absolute())
                || options.url.is_some()
                || options.search.is_some()
                || options.scoped_media
                || options.soak_minutes.is_some()
                || options.ui_cache
                || options.search_cache
                || options.related_cache
                || options.stage_progress
                || options.smoke
                || options.snapshot.is_some()
                || options.handoff_audio.is_some()
                || options.quality_smoke
                || options.comments_smoke
                || options.captions_smoke
                || options.clear_local_smoke
                || options.library_smoke
                || options.refresh_smoke
                || options.preferences_smoke.is_some()
                || options.library_resource_fixture.is_some()
                || options.library_resource_smoke
                || options.comments_snapshots.is_some()
                || options.ui_page.is_some()
                || options.minimized
                || options.subtitle.is_some()
                || options.yt_dlp.is_some()
                || options.deno.is_some()
            {
                return Err(
                    "--native-video-child requires the pinned local MP4 and NEW absolute --data-root; arbitrary subtitles, other content, helpers, transport, capture, lifecycle and rendering experiments are disabled",
                );
            }
        }
        if options.library_resource_smoke && options.library_resource_fixture.is_none() {
            return Err(
                "--library-resource-smoke-test requires an explicit prepared --library-resource-fixture",
            );
        }
        if let Some(root) = &options.library_resource_fixture {
            if !root.is_absolute() || options.data_root.as_ref().is_some_and(|data| data != root) {
                return Err(
                    "--library-resource-fixture requires an absolute prepared root matching any --data-root",
                );
            }
            if options.local.is_some()
                || options.url.is_some()
                || options.search.is_some()
                || options.subtitle.is_some()
                || options.handoff_audio.is_some()
                || options.smoke
                || options.quality_smoke
                || options.comments_smoke
                || options.captions_smoke
                || options.clear_local_smoke
                || options.library_smoke
                || options.refresh_smoke
                || options.preferences_smoke.is_some()
                || options.soak_minutes.is_some()
                || options.demo_related
                || options.paused
                || options.ui_page.is_some()
                || options.scoped_media
                || options.diagnostics
                || options.comments_snapshots.is_some()
                || options.stage_progress
                || options.yt_dlp.is_some()
                || options.deno.is_some()
            {
                return Err(
                    "The offline library resource fixture cannot combine content, helpers, playback diagnostics or a startup page override",
                );
            }
            if options.library_resource_smoke && (options.minimized || options.snapshot.is_some()) {
                return Err(
                    "The library resource traversal requires a visible window without snapshot work",
                );
            }
            options.data_root = Some(root.clone());
        }
        let sources = [
            options.local.is_some(),
            options.url.is_some(),
            options.search.is_some(),
        ];
        if sources.into_iter().filter(|source| *source).count() > 1 {
            return Err("Choose only one of --local, --url or --search");
        }
        if (options.subtitle.is_some()
            || options.handoff_audio.is_some()
            || options.paused
            || options.smoke
            || options.demo_related)
            && options.local.is_none()
        {
            return Err("Local playback diagnostics and subtitles require --local");
        }
        let guest_smokes = [
            options.quality_smoke,
            options.comments_smoke,
            options.captions_smoke,
            options.refresh_smoke,
            options.clear_local_smoke,
        ];
        let guest_count = guest_smokes.into_iter().filter(|enabled| *enabled).count();
        if guest_count > 0 && options.url.is_none() {
            return Err("Guest playback smoke tests require an explicit --url");
        }
        if guest_count + usize::from(options.smoke) + usize::from(options.handoff_audio.is_some())
            > 1
        {
            return Err("Run one playback smoke test at a time");
        }
        if options.handoff_audio.is_some() && (options.demo_related || options.subtitle.is_some()) {
            return Err(
                "The frame handoff diagnostic requires plain local video and audio fixtures",
            );
        }
        if options.comments_snapshots.is_some() && !options.comments_smoke {
            return Err("--comments-snapshots requires --comments-smoke-test");
        }
        if options.clear_local_smoke && options.data_root.is_none() {
            return Err(
                "--clear-local-smoke-test requires --data-root pointing to a new directory",
            );
        }
        if options.library_smoke
            && (options.data_root.is_none()
                || sources.into_iter().any(|source| source)
                || guest_count > 0
                || options.smoke
                || options.minimized
                || options.ui_page.is_some())
        {
            return Err(
                "--library-smoke-test requires NEW --data-root and cannot combine startup content, another smoke, --minimized or --ui-page",
            );
        }
        if options.preferences_smoke.is_some()
            && (options.data_root.is_none()
                || options
                    .data_root
                    .as_ref()
                    .is_some_and(|root| !root.is_absolute())
                || sources.into_iter().any(|source| source)
                || guest_count > 0
                || options.smoke
                || options.library_smoke
                || options.handoff_audio.is_some()
                || options.soak_minutes.is_some()
                || options.snapshot.is_some()
                || options.comments_snapshots.is_some()
                || options.minimized
                || options.paused
                || options.ui_page.is_some()
                || options.ui_theme.is_some()
                || options.ui_size.is_some()
                || options.diagnostics
                || options.demo_related
                || options.scoped_media
                || options.ui_cache
                || options.search_cache
                || options.related_cache
                || options.stage_progress)
        {
            return Err(
                "--preferences-smoke-test requires its explicit absolute --data-root and cannot combine content, another diagnostic or startup UI overrides",
            );
        }
        if let Some(minutes) = options.soak_minutes {
            if options.local.is_none()
                || !options.demo_related
                || options.data_root.is_none()
                || options
                    .data_root
                    .as_ref()
                    .is_some_and(|root| !root.is_absolute())
                || guest_count > 0
                || options.smoke
                || options.handoff_audio.is_some()
                || options.library_smoke
                || options.snapshot.is_some()
                || options.comments_snapshots.is_some()
                || options.paused
                || options.minimized
                || options.ui_page.is_some()
                || options.ui_theme.is_some()
                || options.ui_size.is_some()
                || options.diagnostics
                || options.scoped_media
                || options.ui_cache
                || options.search_cache
                || options.related_cache
                || options.stage_progress
                || options.preferences_smoke.is_some()
            {
                return Err(
                    "--soak-minutes requires --local, --demo-related and NEW absolute --data-root; do not combine other diagnostics, remote content or startup UI overrides",
                );
            }
            let watchdog = u64::from(minutes) * 60 + 15;
            if options
                .quit_after
                .is_some_and(|seconds| seconds != watchdog)
            {
                return Err("Local soak --quit-after must equal minutes * 60 + 15 (or omit it)");
            }
            options.quit_after = Some(watchdog);
        }
        let smoke_duration = if options.native_video_child_smoke {
            Some(46)
        } else if options.library_resource_smoke {
            Some(44)
        } else if options.preferences_smoke.is_some() {
            Some(24)
        } else if options.smoke {
            Some(20)
        } else if options.handoff_audio.is_some() {
            Some(30)
        } else if options.clear_local_smoke {
            Some(85)
        } else if options.library_smoke {
            Some(28)
        } else if options.quality_smoke {
            Some(80)
        } else if guest_count > 0 {
            Some(70)
        } else {
            None
        };
        if let Some(required) = smoke_duration {
            if options.quit_after.is_some_and(|seconds| seconds < required) {
                return Err("--quit-after cannot end a smoke test before its checks finish");
            }
            options.quit_after.get_or_insert(required);
        }
        if options.minimized
            && (options.local.is_some() || options.url.is_some() || options.search.is_some())
        {
            return Err(
                "--minimized is an idle diagnostic; do not combine it with startup media or search",
            );
        }
        Ok(options)
    }
}

fn parse_size(value: &str) -> Result<(u32, u32), &'static str> {
    let (width, height) = value
        .split_once('x')
        .ok_or("--ui-size expects WIDTHxHEIGHT")?;
    let width = width.parse().map_err(|_| "Invalid diagnostic width")?;
    let height = height.parse().map_err(|_| "Invalid diagnostic height")?;
    if !(760..=3840).contains(&width) || !(600..=2160).contains(&height) {
        return Err("Diagnostic size must be between 760x600 and 3840x2160");
    }
    Ok((width, height))
}

pub const HELP: &str = "Serein experimental native client
  --local PATH      Explicit local media
  --subtitle PATH   Explicit local subtitle (requires --local)
  --yt-dlp PATH     Absolute helper path
  --deno PATH       Absolute JavaScript runtime path
  --url URL         Resolve a public YouTube URL
  --search TEXT     Search public YouTube metadata
  --paused          Start local playback paused
  --minimized       Minimized idle diagnostic
  --data-root DIR   Store local data under DIR/Serein (absolute path)
  --ui-page PAGE    Diagnostic: browse/library/settings/account
  --ui-theme THEME  Diagnostic: system/light/dark (not saved)
  --ui-size WxH     Diagnostic logical window size
  --snapshot PATH   One diagnostic PNG at 15 seconds (not for benchmarks)
  --smoke-test      20-second native lifecycle test (requires --local)
  --handoff-audio PATH  30-second local video/audio-only frame-isolation test (requires --local)
  --captions-smoke-test 70-second guest caption/quality test (requires --url)
  --clear-local-smoke-test 85-second caption/cache deletion test (requires --url and NEW --data-root)
  --library-smoke-test 28-second offline local-navigation test (requires NEW --data-root)
  --library-keyboard-smoke-test 38-second offline injected-key playlist test (requires NEW --data-root)
  --collection-window-smoke-test 58-second offline playlist-window test (requires NEW --data-root)
  --library-resource-fixture ROOT  Prepared, labeled offline 10,000-item fixture (uses ROOT/Serein)
  --library-resource-smoke-test  Five 100-row page/input checks plus keyboard/resize checks (44s; requires prepared fixture; no resource qualification)
  --comments-smoke-test 70-second guest details/comments test (requires --url)
  --comments-snapshots DIR  Two PNGs (requires comments smoke)
  --quality-smoke-test 80-second guest quality/resume test (requires --url)
  --refresh-smoke-test 70-second controlled expiry/resume test (requires --url)
  --preferences-smoke-test write|verify  Offline preference/restart test (requires isolated --data-root)
  --soak-minutes N  Explicit 60–240 minute local lifecycle soak (requires --local, --demo-related, NEW --data-root)
  --related-focus-check  Finite offline related keyboard/resize/fullscreen checks (42s; exact fixture and NEW data root)
  --home-smoke-test Finite offline local Home navigation/mutation checks (40s; NEW absolute --data-root)
  --pip-smoke-test  Finite offline picture-in-picture lifecycle checks (25s; local video and NEW absolute --data-root)
  --recovery-smoke-test  Finite synthetic failure/retry checks (20s; --yt-dlp /usr/bin/false and NEW private --data-root)
  --save-smoke-test  Finite real guest/local Save checks (75s; public --url and NEW private --data-root)
  --demo-related    Labeled static fixture rows (requires --local)
  --ui-cache        Diagnostic cached-chrome candidate (unqualified)
  --search-cache    Diagnostic cache of only the search input (unqualified)
  --related-cache   Diagnostic cache of watch-page related cards (unqualified)
  --stage-progress  Diagnostic cosmetic-clock staging during rendering (unqualified)
  --no-ui-cache     Force cached chrome off (default)
  --scoped-media    Experimental in-process HTTPS transport (unqualified)
  --native-video-child  macOS child-surface diagnostic (requires pinned synthetic --local MP4 and NEW absolute --data-root; no subtitles, accounts, online actions or texture screenshots)
  --native-video-child-smoke-test  Finite 46-second child lifecycle checks; requires --native-video-child and a hardware-decodable local clip of at least 60 seconds
  --diagnostics     Finite playback snapshot checkpoints
  --quit-after N    Exit after N seconds, from 1 to 86400
  --help            Show this help without opening a window
No network requests occur until search or URL submission.";

#[cfg(test)]
mod tests {
    #[test]
    fn home_diagnostic_forbids_network_existing_modes_and_missing_private_root() {
        let base = ["--home-smoke-test", "--data-root", "/synthetic/new-profile"];
        let options = parse(&base).unwrap();
        assert!(options.home_smoke);
        assert_eq!(options.quit_after, Some(40));
        assert!(parse(&["--home-smoke-test"]).is_err());
        for extra in [
            vec!["--url", "https://www.youtube.com/watch?v=abcdefghijk"],
            vec!["--local", "/synthetic/clip.mp4"],
            vec!["--ui-page", "account"],
            vec!["--minimized"],
            vec!["--pip-smoke-test"],
            vec!["--quit-after", "5"],
        ] {
            let mut args = base.to_vec();
            args.extend(extra);
            assert!(parse(&args).is_err());
        }
    }
    #[test]
    fn pip_diagnostic_is_offline_isolated_and_has_a_finite_watchdog() {
        let base = [
            "--pip-smoke-test",
            "--local",
            "/synthetic/clip.mp4",
            "--data-root",
            "/synthetic/profile",
        ];
        let parse =
            |args: &[&str]| super::Options::parse(args.iter().map(std::ffi::OsString::from));
        let options = parse(&base).unwrap();
        assert!(options.pip_smoke);
        assert_eq!(options.quit_after, Some(25));
        for extra in [
            vec!["--url", "https://www.youtube.com/watch?v=abcdefghijk"],
            vec!["--paused"],
            vec!["--native-video-child"],
            vec!["--smoke-test"],
            vec!["--quit-after", "5"],
        ] {
            let mut args = base.to_vec();
            args.extend(extra);
            assert!(parse(&args).is_err());
        }
        assert!(parse(&["--pip-smoke-test", "--local", "/synthetic/clip.mp4"]).is_err());
        assert!(parse(&["--pip-smoke-test", "--data-root", "/synthetic/profile"]).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn recovery_diagnostic_requires_exact_inert_helper_and_isolated_inputs() {
        let base = [
            "--recovery-smoke-test",
            "--yt-dlp",
            "/usr/bin/false",
            "--data-root",
            "/synthetic/new",
        ];
        assert_eq!(parse(&base).unwrap().quit_after, Some(20));
        let mut with_snapshot = base.to_vec();
        with_snapshot.extend(["--snapshot", "/synthetic/new/error.png"]);
        assert!(parse(&with_snapshot).is_ok());
        for extra in [
            vec!["--url", "https://www.youtube.com/watch?v=aqz-KE-bpKQ"],
            vec!["--search", "query"],
            vec!["--local", "/tmp/clip"],
            vec!["--deno", "/usr/bin/false"],
            vec!["--snapshot", "/synthetic/other/out.png"],
            vec!["--ui-page", "account"],
            vec!["--scoped-media"],
            vec!["--save-smoke-test"],
            vec!["--native-video-child"],
            vec!["--quit-after", "19"],
        ] {
            let mut args = base.to_vec();
            args.extend(extra);
            assert!(parse(&args).is_err());
        }
        for helper in [
            "/bin/false",
            "/usr/bin/true",
            "relative",
            "/usr/bin/../bin/false",
            "/usr/bin/false/",
            "/usr//bin/false",
            "/usr/bin/./false",
        ] {
            assert!(
                parse(&[
                    "--recovery-smoke-test",
                    "--yt-dlp",
                    helper,
                    "--data-root",
                    "/synthetic/new"
                ])
                .is_err()
            );
        }
        assert!(parse(&["--recovery-smoke-test", "--yt-dlp", "/usr/bin/false"]).is_err());
        assert!(
            parse(&[
                "--recovery-smoke-test",
                "--yt-dlp",
                "/usr/bin/false",
                "--data-root",
                "relative"
            ])
            .is_err()
        );
    }
    #[cfg(not(unix))]
    #[test]
    fn recovery_diagnostic_is_unavailable_without_unix_supervision() {
        assert!(
            parse(&[
                "--recovery-smoke-test",
                "--yt-dlp",
                "/usr/bin/false",
                "--data-root",
                "/synthetic/new"
            ])
            .is_err()
        );
    }
    #[test]
    fn save_diagnostic_requires_explicit_new_profile_and_one_public_video_scope() {
        let base = [
            "--save-smoke-test",
            "--url",
            "https://www.youtube.com/watch?v=abcdefghijk",
            "--data-root",
            "/synthetic/new",
        ];
        let parsed = parse(&base).unwrap();
        assert!(parsed.save_smoke);
        assert_eq!(parsed.quit_after, Some(75));
        for extra in [
            "--scoped-media",
            "--comments-smoke-test",
            "--native-video-child",
            "--ui-cache",
            "--minimized",
        ] {
            let mut values = base.to_vec();
            values.push(extra);
            assert!(parse(&values).is_err());
        }
        for pair in [
            ["--quit-after", "74"],
            ["--quit-after", "76"],
            ["--yt-dlp", "/synthetic/helper"],
            ["--deno", "/synthetic/runtime"],
            ["--snapshot", "/synthetic/output.png"],
            ["--snapshot", "/synthetic/new/../output.png"],
            ["--snapshot", "/synthetic/new/nested/output.png"],
            ["--snapshot", "/synthetic/new/output.jpg"],
            ["--snapshot", "relative.png"],
        ] {
            let mut values = base.to_vec();
            values.extend(pair);
            assert!(parse(&values).is_err());
        }
        let mut capture = base.to_vec();
        capture.extend(["--snapshot", "/synthetic/new/save.png"]);
        assert!(parse(&capture).is_ok());
        assert!(
            parse(&[
                "--save-smoke-test",
                "--url",
                "https://www.youtube.com/watch?v=abcdefghijk"
            ])
            .is_err()
        );
        assert!(
            parse(&[
                "--save-smoke-test",
                "--url",
                "https://example.invalid/",
                "--data-root",
                "/synthetic/new"
            ])
            .is_err()
        );
        assert!(
            parse(&[
                "--save-smoke-test",
                "--url",
                "https://www.youtube.com/watch?v=abcdefghijk",
                "--data-root",
                "relative"
            ])
            .is_err()
        );
    }
    #[test]
    fn native_child_lifecycle_requires_its_mode_and_complete_deadline() {
        assert!(parse(&["--native-video-child-smoke-test"]).is_err());
        let base = [
            "--native-video-child",
            "--native-video-child-smoke-test",
            "--local",
            "fixture.mp4",
            "--data-root",
            "/synthetic/new",
        ];
        if cfg!(target_os = "macos") {
            assert_eq!(parse(&base).unwrap().quit_after, Some(46));
            let mut too_short = base.to_vec();
            too_short.extend(["--quit-after", "45"]);
            assert!(parse(&too_short).is_err());
            let mut capture = base.to_vec();
            capture.extend(["--snapshot", "/synthetic/capture.png"]);
            assert!(parse(&capture).is_err());
            let mut paused = base.to_vec();
            paused.push("--paused");
            assert!(parse(&paused).is_err());
        } else {
            assert!(parse(&base).is_err());
        }
    }
    use super::*;
    fn parse(args: &[&str]) -> Result<Options, &'static str> {
        Options::parse(args.iter().map(OsString::from))
    }
    #[test]
    fn related_focus_diagnostic_is_exact_offline_bounded_and_isolated() {
        let base = [
            "--related-focus-check",
            "--local",
            "fixture.mp4",
            "--demo-related",
            "--data-root",
            "/new-synthetic-root",
        ];
        if cfg!(unix) {
            assert_eq!(parse(&base).unwrap().quit_after, Some(42));
            for extra in [
                vec!["--search", "test"],
                vec!["--native-video-child"],
                vec!["--smoke-test"],
                vec!["--yt-dlp", "/helper"],
                vec!["--quit-after", "43"],
                vec!["--quit-after", "41"],
            ] {
                let mut args = base.to_vec();
                args.extend(extra);
                assert!(parse(&args).is_err());
            }
        }
        assert!(parse(&["--related-focus-check"]).is_err());
        assert!(
            parse(&[
                "--related-focus-check",
                "--local",
                "fixture.mp4",
                "--demo-related",
                "--data-root",
                "relative"
            ])
            .is_err()
        );
    }
    #[test]
    fn native_child_diagnostic_is_explicit_local_isolated_and_exclusive() {
        let base = [
            "--native-video-child",
            "--local",
            "fixture.mp4",
            "--data-root",
            "/synthetic/new",
        ];
        assert_eq!(parse(&base).is_ok(), cfg!(target_os = "macos"));
        for extra in [
            vec!["--url", "https://www.youtube.com/watch?v=BaW_jenozKc"],
            vec!["--search", "synthetic"],
            vec!["--scoped-media"],
            vec!["--smoke-test"],
            vec!["--snapshot", "/synthetic/capture.png"],
            vec!["--ui-cache"],
            vec!["--related-cache"],
            vec!["--search-cache"],
            vec!["--stage-progress"],
            vec!["--soak-minutes", "60"],
            vec!["--ui-page", "account"],
            vec!["--yt-dlp", "/synthetic/helper"],
            vec!["--subtitle", "fixture.srt"],
        ] {
            let mut args = base.to_vec();
            args.extend(extra);
            assert!(parse(&args).is_err());
        }
        assert!(parse(&["--native-video-child", "--local", "fixture.mp4"]).is_err());
        assert!(
            parse(&[
                "--native-video-child",
                "--local",
                "fixture.mp4",
                "--data-root",
                "relative"
            ])
            .is_err()
        );
        let mut allowed = base.to_vec();
        allowed.extend([
            "--paused",
            "--demo-related",
            "--diagnostics",
            "--quit-after",
            "30",
            "--ui-size",
            "760x600",
            "--ui-theme",
            "dark",
        ]);
        assert_eq!(parse(&allowed).is_ok(), cfg!(target_os = "macos"));
    }
    #[test]
    fn missing_unknown_and_duplicate_arguments_fail_without_echoing_values() {
        for args in [
            vec!["--url"],
            vec!["--url", "--diagnostics"],
            vec!["--url", ""],
            vec!["--unknown=private-value"],
            vec!["private-value"],
            vec!["--url", "private-value", "--url", "another-value"],
            vec!["--diagnostics", "--diagnostics"],
        ] {
            let error = parse(&args).err().expect("must reject malformed arguments");
            assert!(!error.contains("private-value"));
            assert!(!error.contains("another-value"));
        }
    }
    #[test]
    fn option_shaped_values_cannot_activate_unrequested_flags() {
        let options = parse(&["--search", "two words --minimized", "--ui-theme", "dark"]).unwrap();
        assert!(!options.minimized);
        assert_eq!(options.search.as_deref(), Some("two words --minimized"));
        assert_eq!(options.ui_theme, Some(2));
        assert!(parse(&["--search", "--minimized"]).is_err());
        assert!(parse(&["--ui-cache", "--no-ui-cache"]).is_err());
        assert!(parse(&["--ui-cache", "--search-cache"]).is_err());
    }
    #[test]
    fn progress_staging_is_explicit_and_does_not_enable_caching() {
        assert!(!parse(&[]).unwrap().stage_progress);
        let options = parse(&["--stage-progress"]).unwrap();
        assert!(options.stage_progress);
        assert!(!options.ui_cache && !options.search_cache && !options.related_cache);
        assert!(parse(&["--stage-progress", "--stage-progress"]).is_err());
    }
    #[test]
    fn related_cache_is_explicit_and_cannot_mix_experiment_scopes() {
        assert!(!parse(&[]).unwrap().related_cache);
        let options = parse(&["--related-cache"]).unwrap();
        assert!(options.related_cache);
        assert!(!options.ui_cache && !options.search_cache);
        for flags in [
            ["--related-cache", "--ui-cache"],
            ["--search-cache", "--related-cache"],
            ["--related-cache", "--related-cache"],
        ] {
            assert!(parse(&flags).is_err());
        }
        assert!(
            parse(&["--related-cache", "--no-ui-cache"])
                .unwrap()
                .related_cache
        );
    }
    #[test]
    fn library_diagnostic_is_finite_isolated_and_cannot_start_remote_or_media_work() {
        assert!(parse(&["--library-smoke-test"]).is_err());
        let base = ["--library-smoke-test", "--data-root", "/new-root"];
        let options = parse(&base).unwrap();
        assert!(options.library_smoke);
        assert_eq!(options.quit_after, Some(28));
        assert!(!parse(&[]).unwrap().library_smoke);
        for extra in [
            vec!["--quit-after", "25"],
            vec!["--local", "video.mp4"],
            vec!["--url", "public"],
            vec!["--search", "public"],
            vec!["--minimized"],
            vec!["--ui-page", "library"],
            vec!["--smoke-test"],
            vec!["--clear-local-smoke-test"],
        ] {
            let mut args = base.to_vec();
            args.extend(extra);
            assert!(parse(&args).is_err());
        }
    }
    #[test]
    fn collection_window_diagnostic_is_finite_and_isolated() {
        assert!(parse(&["--collection-window-smoke-test"]).is_err());
        let base = ["--collection-window-smoke-test", "--data-root", "/new-root"];
        let parsed = parse(&base);
        if cfg!(unix) {
            let options = parsed.unwrap();
            assert!(options.collection_window_smoke);
            assert_eq!(options.quit_after, Some(58));
        } else {
            assert!(parsed.is_err());
        }
        for extra in [
            vec!["--quit-after", "57"],
            vec!["--quit-after", "59"],
            vec!["--local", "clip"],
            vec!["--search", "query"],
            vec!["--url", "https://youtu.be/aqz-KE-bpKQ"],
            vec!["--yt-dlp", "/usr/bin/false"],
            vec!["--library-keyboard-smoke-test"],
            vec!["--home-smoke-test"],
            vec!["--minimized"],
            vec!["--ui-page", "library"],
            vec!["--snapshot", "/elsewhere/p.png"],
            vec!["--soak-minutes", "60"],
        ] {
            let mut args = base.to_vec();
            args.extend(extra);
            assert!(parse(&args).is_err());
        }
        assert!(parse(&["--collection-window-smoke-test", "--data-root", "relative"]).is_err());
    }
    #[test]
    fn keyboard_library_diagnostic_rejects_every_unrelated_input() {
        assert!(parse(&["--library-keyboard-smoke-test"]).is_err());
        assert!(parse(&["--library-keyboard-smoke-test", "--data-root", "relative"]).is_err());
        let base = ["--library-keyboard-smoke-test", "--data-root", "/new-root"];
        let result = parse(&base);
        if cfg!(unix) {
            let options = result.unwrap();
            assert!(options.library_keyboard_smoke);
            assert_eq!(options.quit_after, Some(38));
        } else {
            assert!(result.is_err());
        }
        for extra in [
            vec!["--quit-after", "37"],
            vec!["--quit-after", "39"],
            vec!["--local", "clip"],
            vec!["--url", "https://youtu.be/aqz-KE-bpKQ"],
            vec!["--search", "fixture"],
            vec!["--yt-dlp", "/usr/bin/false"],
            vec!["--deno", "/usr/bin/false"],
            vec!["--library-smoke-test"],
            vec!["--home-smoke-test"],
            vec!["--minimized"],
            vec!["--diagnostics"],
            vec!["--snapshot", "/elsewhere/image.png"],
            vec!["--ui-page", "library"],
            vec!["--ui-cache"],
            vec!["--scoped-media"],
            vec!["--paused"],
        ] {
            let mut args = base.to_vec();
            args.extend(extra);
            assert!(parse(&args).is_err());
        }
    }
    #[test]
    fn prepared_library_resource_mode_is_explicit_offline_and_has_no_default_timer() {
        let base = ["--library-resource-fixture", "/prepared-root"];
        let idle = parse(&base).unwrap();
        assert_eq!(
            idle.data_root.as_deref(),
            Some(std::path::Path::new("/prepared-root"))
        );
        assert_eq!(idle.quit_after, None);
        assert!(!idle.library_resource_smoke);
        assert!(parse(&["--library-resource-smoke-test"]).is_err());
        assert!(parse(&["--library-resource-fixture", "relative"]).is_err());
        for extra in [
            vec!["--data-root", "/different"],
            vec!["--local", "clip"],
            vec!["--url", "public"],
            vec!["--search", "public"],
            vec!["--library-smoke-test"],
            vec!["--ui-page", "account"],
            vec!["--preferences-smoke-test", "verify"],
            vec!["--scoped-media"],
            vec!["--diagnostics"],
            vec!["--soak-minutes", "60"],
        ] {
            let mut args = base.to_vec();
            args.extend(extra);
            assert!(parse(&args).is_err());
        }
        let mut finite = base.to_vec();
        finite.push("--library-resource-smoke-test");
        assert_eq!(parse(&finite).unwrap().quit_after, Some(44));
        for extra in [
            vec!["--minimized"],
            vec!["--snapshot", "/snapshot.png"],
            vec!["--quit-after", "29"],
        ] {
            let mut args = finite.clone();
            args.extend(extra);
            assert!(parse(&args).is_err());
        }
        let mut same_root = base.to_vec();
        same_root.extend(["--data-root", "/prepared-root"]);
        assert!(parse(&same_root).is_ok());
    }
    #[test]
    fn finite_tests_cannot_silently_skip_checks_or_combine_workloads() {
        assert!(parse(&["--handoff-audio", "audio.wav"]).is_err());
        assert_eq!(
            parse(&["--local", "video.mp4", "--handoff-audio", "audio.wav"])
                .unwrap()
                .quit_after,
            Some(30)
        );
        for extra in [
            vec!["--quit-after", "20"],
            vec!["--smoke-test"],
            vec!["--demo-related"],
            vec!["--subtitle", "caption.vtt"],
        ] {
            let mut args = vec!["--local", "video.mp4", "--handoff-audio", "audio.wav"];
            args.extend(extra);
            assert!(parse(&args).is_err());
        }
        assert!(parse(&["--url", "public", "--clear-local-smoke-test"]).is_err());
        assert_eq!(
            parse(&[
                "--url",
                "public",
                "--clear-local-smoke-test",
                "--data-root",
                "/new-root"
            ])
            .unwrap()
            .quit_after,
            Some(85)
        );
        assert!(
            parse(&[
                "--url",
                "public",
                "--clear-local-smoke-test",
                "--data-root",
                "/new-root",
                "--quit-after",
                "70"
            ])
            .is_err()
        );
        assert_eq!(
            parse(&["--url", "public", "--captions-smoke-test"])
                .unwrap()
                .quit_after,
            Some(70)
        );
        assert!(
            parse(&[
                "--url",
                "public",
                "--captions-smoke-test",
                "--quit-after",
                "5"
            ])
            .is_err()
        );
        assert!(
            parse(&[
                "--url",
                "public",
                "--quality-smoke-test",
                "--captions-smoke-test"
            ])
            .is_err()
        );
        assert!(parse(&["--local", "clip", "--url", "public"]).is_err());
        assert!(parse(&["--search", "public", "--comments-smoke-test"]).is_err());
        assert!(parse(&["--paused"]).is_err());
        for invalid in [
            "0",
            "-1",
            "1.5",
            "86401",
            "18446744073709551616",
            "private-value",
        ] {
            assert!(parse(&["--quit-after", invalid]).is_err());
        }
        assert_eq!(
            parse(&["--quit-after", "3600"]).unwrap().quit_after,
            Some(3600)
        );
    }
    #[test]
    fn preferences_diagnostic_is_explicit_isolated_and_finite() {
        assert!(parse(&[]).unwrap().preferences_smoke.is_none());
        for phase in ["write", "verify"] {
            let base = [
                "--preferences-smoke-test",
                phase,
                "--data-root",
                "/isolated",
            ];
            assert_eq!(parse(&base).unwrap().quit_after, Some(24));
            for extra in [
                vec!["--quit-after", "23"],
                vec!["--local", "fixture.mp4"],
                vec!["--url", "public"],
                vec!["--search", "public"],
                vec!["--library-smoke-test"],
                vec!["--minimized"],
                vec!["--snapshot", "/file.png"],
                vec!["--ui-page", "settings"],
                vec!["--stage-progress"],
                vec!["--diagnostics"],
                vec!["--soak-minutes", "60"],
            ] {
                let mut args = base.to_vec();
                args.extend(extra);
                assert!(parse(&args).is_err());
            }
        }
        assert!(
            parse(&[
                "--preferences-smoke-test",
                "invalid",
                "--data-root",
                "/isolated"
            ])
            .is_err()
        );
        assert!(parse(&["--preferences-smoke-test", "write"]).is_err());
        assert!(
            parse(&[
                "--preferences-smoke-test",
                "write",
                "--data-root",
                "relative"
            ])
            .is_err()
        );
    }
    #[test]
    fn soak_requires_full_duration_fresh_profile_workload_and_fixed_watchdog() {
        assert!(parse(&[]).unwrap().soak_minutes.is_none());
        let base = [
            "--soak-minutes",
            "60",
            "--local",
            "fixture.mp4",
            "--demo-related",
            "--data-root",
            "/new-root",
        ];
        let options = parse(&base).unwrap();
        assert_eq!(options.soak_minutes, Some(60));
        assert_eq!(options.quit_after, Some(3615));
        for extra in [
            vec!["--quit-after", "3600"],
            vec!["--quit-after", "86400"],
            vec!["--smoke-test"],
            vec!["--handoff-audio", "fixture.wav"],
            vec!["--library-smoke-test"],
            vec!["--snapshot", "/out.png"],
            vec!["--paused"],
            vec!["--minimized"],
            vec!["--ui-size", "1320x860"],
            vec!["--ui-page", "browse"],
            vec!["--ui-theme", "dark"],
            vec!["--diagnostics"],
            vec!["--ui-cache"],
            vec!["--search-cache"],
            vec!["--related-cache"],
            vec!["--stage-progress"],
            vec!["--preferences-smoke-test", "write"],
            vec!["--scoped-media"],
            vec!["--url", "public"],
        ] {
            let mut args = base.to_vec();
            args.extend(extra);
            assert!(parse(&args).is_err());
        }
        for duration in ["0", "59", "241", "1.5", "4294967296"] {
            let mut args = base;
            args[1] = duration;
            assert!(parse(&args).is_err());
        }
        assert!(
            parse(&[
                "--soak-minutes",
                "60",
                "--local",
                "fixture.mp4",
                "--demo-related"
            ])
            .is_err()
        );
        assert!(
            parse(&[
                "--soak-minutes",
                "60",
                "--local",
                "fixture.mp4",
                "--data-root",
                "/new-root"
            ])
            .is_err()
        );
        let mut relative = base;
        relative[6] = "relative-root";
        assert!(parse(&relative).is_err());
        let mut args = base.to_vec();
        args.extend(["--quit-after", "3615"]);
        assert!(parse(&args).is_ok());
        let mut maximum = base;
        maximum[1] = "240";
        assert_eq!(parse(&maximum).unwrap().quit_after, Some(14415));
    }
    #[cfg(unix)]
    #[test]
    fn native_paths_preserve_bytes_and_non_utf8_text_fails_without_panic() {
        use std::os::unix::ffi::OsStringExt;
        let path = OsString::from_vec(b"/tmp/local-\xff".to_vec());
        let options = Options::parse(["--data-root".into(), path.clone()]).unwrap();
        assert_eq!(options.data_root.unwrap().into_os_string(), path);
        assert!(Options::parse(["--search".into(), path.clone()]).is_err());
        assert!(Options::parse([path]).is_err());
    }
}
