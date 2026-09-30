//! Guest YouTube catalog and resolution. Invoke blocking methods on a worker thread.
//! No account import or authenticated capabilities are implied by this adapter.
pub mod account;
mod activity;
pub mod captions;
pub mod catalog;
mod channel_avatar;
mod chapters;
pub mod comments;
pub mod innertube;
pub mod suggestions;
mod supervisor;
use serde_json::Value;
use serein_core::{
    ChannelId, MediaTrack, MediaUrl, OperationContext, OriginHeaders, ProviderError,
    ResolvedPlayback, VideoId, VideoSummary,
};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant, SystemTime},
};
use url::Url;

const PAGE_SIZE: usize = 20;
const MAX_SEARCH_RESULTS: usize = 200;
/// Upper bound for one yt-dlp JSON document. Popular videos list ~180
/// machine-translated caption languages (observed 11.2 MiB of the 11.7 MiB
/// document for one guest video on 2026-09-30); `skip=translated_subs` did not
/// reduce it. The bound stays finite and the parsed value is dropped after use.
pub const MAX_EXTRACTOR_JSON_BYTES: usize = 32 * 1024 * 1024;
/// Opaque provider-owned cursor. Search terms intentionally have no Debug formatting.
#[derive(Clone)]
pub struct SearchCursor {
    query: String,
    offset: usize,
}
pub struct SearchPage {
    pub videos: Vec<VideoSummary>,
    pub next: Option<SearchCursor>,
}

/// A quality ceiling and codec preference, not a hardware-decoder guarantee.
#[derive(Clone, Copy, Debug)]
pub struct ResolutionPolicy {
    pub max_height: u16,
    /// At equal resolution and frame rate, prefer H.264. Never reduce either to get H.264.
    pub prefer_h264: bool,
}
impl Default for ResolutionPolicy {
    fn default() -> Self {
        Self {
            max_height: 1080,
            prefer_h264: true,
        }
    }
}
impl ResolutionPolicy {
    fn arguments(self) -> Result<Vec<String>, ProviderError> {
        if !(144..=2160).contains(&self.max_height) {
            return Err(ProviderError::InvalidInput);
        }
        let height = self.max_height;
        Ok(vec![
            "--format".into(),
            format!(
                "bestvideo[protocol=https][height<={height}]+bestaudio[protocol=https]/best[protocol=https][height<={height}]"
            ),
            "--format-sort".into(),
            if self.prefer_h264 {
                "height,fps,vcodec:h264".into()
            } else {
                "height,fps".into()
            },
        ])
    }
}

#[derive(Clone, Copy)]
enum RateLimit {
    Until(Instant),
    Indefinite,
}
impl RateLimit {
    fn active(self) -> bool {
        match self {
            Self::Until(until) => until > Instant::now(),
            Self::Indefinite => true,
        }
    }
    fn after(delay: Duration) -> Self {
        Instant::now()
            .checked_add(delay)
            .map(Self::Until)
            .unwrap_or(Self::Indefinite)
    }
}
pub struct YtDlp {
    binary: PathBuf,
    deno: Option<PathBuf>,
    active: activity::Gate,
    cooldown: Mutex<Option<RateLimit>>,
    timeout: Duration,
    resolution: ResolutionPolicy,
}
impl YtDlp {
    pub fn new(binary: impl AsRef<Path>) -> Result<Self, ProviderError> {
        if !binary.as_ref().is_absolute() {
            return Err(ProviderError::InvalidInput);
        }
        Ok(Self {
            binary: binary.as_ref().to_owned(),
            deno: None,
            active: activity::Gate::default(),
            cooldown: Mutex::new(None),
            timeout: Duration::from_secs(45),
            resolution: ResolutionPolicy::default(),
        })
    }
    /// Opt in to one installed, reviewed runtime. Runtime downloads remain disabled.
    pub fn with_deno(mut self, path: impl AsRef<Path>) -> Result<Self, ProviderError> {
        if !path.as_ref().is_absolute() {
            return Err(ProviderError::InvalidInput);
        }
        self.deno = Some(path.as_ref().to_owned());
        Ok(self)
    }
    pub fn with_resolution_policy(
        mut self,
        policy: ResolutionPolicy,
    ) -> Result<Self, ProviderError> {
        policy.arguments()?;
        self.resolution = policy;
        Ok(self)
    }
    fn run(&self, extra: &[String], operation: &OperationContext) -> Result<Value, ProviderError> {
        self.run_guarded(extra, operation, &|| false)
    }
    fn run_guarded(
        &self,
        extra: &[String],
        operation: &OperationContext,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Value, ProviderError> {
        self.run_with_priority(extra, operation, cancelled, false)
    }
    fn run_with_priority(
        &self,
        extra: &[String],
        operation: &OperationContext,
        cancelled: &dyn Fn() -> bool,
        background: bool,
    ) -> Result<Value, ProviderError> {
        if operation.cancel.is_cancelled() || cancelled() {
            return Err(ProviderError::Cancelled);
        }
        if self
            .cooldown
            .lock()
            .map_err(|_| ProviderError::ExtractorFailed)?
            .is_some_and(RateLimit::active)
        {
            return Err(ProviderError::RateLimited);
        }
        let _active = self.active.acquire(&operation.cancel, background)?;
        if operation.cancel.is_cancelled() || cancelled() {
            return Err(ProviderError::Cancelled);
        }
        // A preempted background operation may establish cooldown while the
        // foreground worker waits for its supervised teardown.
        if self
            .cooldown
            .lock()
            .map_err(|_| ProviderError::ExtractorFailed)?
            .is_some_and(RateLimit::active)
        {
            return Err(ProviderError::RateLimited);
        }
        let mut args: Vec<String> = [
            "--ignore-config",
            "--no-config-locations",
            "--no-plugin-dirs",
            "--no-cache-dir",
            "--no-js-runtimes",
            "--no-remote-components",
            "--no-mark-watched",
            "--no-cookies-from-browser",
            "--no-progress",
            "--no-warnings",
            "--skip-download",
            "--socket-timeout",
            "15",
            "--retries",
            "0",
            "--extractor-retries",
            "0",
            "--dump-single-json",
            "--proxy",
            "",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        if let Some(deno) = &self.deno {
            args.push("--js-runtimes".into());
            args.push(format!("deno:{}", deno.display()));
        }
        args.extend_from_slice(extra);
        let output = supervisor::run_guarded(
            &self.binary,
            &args,
            operation,
            self.timeout,
            MAX_EXTRACTOR_JSON_BYTES,
            64 * 1024,
            cancelled,
        )?;
        if !output.success {
            let error = classify_failure(&output.stderr);
            if error == ProviderError::RateLimited {
                *self
                    .cooldown
                    .lock()
                    .map_err(|_| ProviderError::ExtractorFailed)? =
                    Some(RateLimit::after(Duration::from_secs(60)));
            }
            return Err(error);
        }
        serde_json::from_slice(&output.stdout).map_err(|_| ProviderError::MalformedOutput)
    }
    pub fn search(
        &self,
        query: &str,
        cursor: Option<&SearchCursor>,
        operation: &OperationContext,
    ) -> Result<SearchPage, ProviderError> {
        let query = query.trim();
        if query.is_empty() || query.chars().count() > 200 || query.chars().any(char::is_control) {
            return Err(ProviderError::InvalidInput);
        }
        let start = match cursor {
            Some(c) if c.query == query => c.offset,
            Some(_) => return Err(ProviderError::InvalidInput),
            None => 0,
        };
        if start >= MAX_SEARCH_RESULTS {
            return Err(ProviderError::InvalidInput);
        }
        let end = start + PAGE_SIZE;
        let response = self.run(
            &[
                "--flat-playlist".into(),
                "--playlist-items".into(),
                format!("{}:{end}", start + 1),
                "--".into(),
                format!("ytsearch{end}:{query}"),
            ],
            operation,
        )?;
        let entries = response
            .get("entries")
            .and_then(Value::as_array)
            .ok_or(ProviderError::MalformedOutput)?;
        let videos = entries
            .iter()
            .filter(|v| !is_promoted(v))
            .filter_map(|v| summary(v).ok())
            .take(PAGE_SIZE)
            .collect();
        let next = (entries.len() >= PAGE_SIZE && end < MAX_SEARCH_RESULTS).then(|| SearchCursor {
            query: query.to_owned(),
            offset: end,
        });
        Ok(SearchPage { videos, next })
    }
    pub fn resolve(
        &self,
        id: &VideoId,
        operation: &OperationContext,
    ) -> Result<ResolvedPlayback, ProviderError> {
        self.resolve_with_policy(id, self.resolution, operation)
    }
    /// Reuse the same supervised helper, cancellation and rate-limit budget when
    /// the user deliberately changes the maximum playback quality.
    pub fn resolve_with_policy(
        &self,
        id: &VideoId,
        policy: ResolutionPolicy,
        operation: &OperationContext,
    ) -> Result<ResolvedPlayback, ProviderError> {
        let mut arguments = policy.arguments()?;
        arguments.extend(["--no-playlist".into(), "--".into(), id.watch_url()]);
        let response = self.run(&arguments, operation)?;
        let playback = parse_playback(&response, operation.session_generation)?;
        if playback.video.id != *id {
            return Err(ProviderError::MalformedOutput);
        }
        Ok(playback)
    }
    fn resolve_authorized(
        &self,
        id: &VideoId,
        policy: ResolutionPolicy,
        cookie_path: &Path,
        operation: &OperationContext,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<ResolvedPlayback, ProviderError> {
        let cookie_path = cookie_path.to_str().ok_or(ProviderError::InvalidInput)?;
        let mut arguments = policy.arguments()?;
        arguments.extend([
            "--cookies".into(),
            cookie_path.to_owned(),
            "--no-playlist".into(),
            "--".into(),
            id.watch_url(),
        ]);
        let response = self.run_guarded(&arguments, operation, cancelled)?;
        let mut playback = parse_playback(&response, operation.session_generation)?;
        if playback.video.id != *id {
            return Err(ProviderError::MalformedOutput);
        }
        playback.guest = false;
        Ok(playback)
    }
}

fn summary(value: &Value) -> Result<VideoSummary, ProviderError> {
    let id = VideoId::new(
        value
            .get("id")
            .and_then(Value::as_str)
            .ok_or(ProviderError::MalformedOutput)?,
    )?;
    let title = value
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("Untitled video")
        .chars()
        .take(500)
        .collect();
    let channel = value
        .get("channel")
        .or_else(|| value.get("uploader"))
        .and_then(Value::as_str)
        .unwrap_or("Unknown channel")
        .chars()
        .take(200)
        .collect();
    let thumbnail_url = value
        .get("thumbnail")
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .get("thumbnails")?
                .as_array()?
                .last()?
                .get("url")?
                .as_str()
        })
        .and_then(safe_thumbnail);
    Ok(VideoSummary {
        id,
        title,
        channel,
        channel_id: value
            .get("channel_id")
            .and_then(Value::as_str)
            .filter(|s| s.len() <= 100)
            .map(|s| ChannelId(s.to_owned())),
        duration: value
            .get("duration")
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite() && *v >= 0.0 && *v <= 31_536_000.0)
            .map(Duration::from_secs_f64),
        thumbnail_url,
    })
}
fn safe_thumbnail(value: &str) -> Option<String> {
    let u = Url::parse(value).ok()?;
    let host = u.host_str()?;
    (u.scheme() == "https"
        && u.username().is_empty()
        && u.password().is_none()
        && u.port().is_none()
        && (host == "i.ytimg.com"
            || host.ends_with(".ytimg.com")
            || host == "yt3.ggpht.com"
            || host == "yt3.googleusercontent.com"))
        .then(|| u.into())
}
/// Known advertising shapes are dropped before normalization. Unknown fields alone
/// do not identify an ad; this is not a guarantee about inseparable in-stream ads.
fn is_promoted(value: &Value) -> bool {
    const KEYS: &[&str] = &[
        "adSlotRenderer",
        "displayAdRenderer",
        "promotedVideoRenderer",
        "promotedSparklesWebRenderer",
        "promotedSparklesTextSearchRenderer",
        "inFeedAdLayoutRenderer",
        "adPlacementRenderer",
        "adMetadata",
    ];
    match value {
        Value::Object(map) => map.iter().any(|(key, value)| {
            KEYS.contains(&key.as_str())
                || ((key == "is_ad" || key == "is_promoted") && value.as_bool() == Some(true))
                || is_promoted(value)
        }),
        Value::Array(values) => values.iter().any(is_promoted),
        _ => false,
    }
}
fn reject_required_cookies(value: &Value) -> Result<(), ProviderError> {
    // yt-dlp 2026.08.19 YoutubeDL._calc_headers removes Cookie from headers and
    // emits scoped requirements separately. Never silently drop that authority.
    match value.get("cookies") {
        None => Ok(()),
        Some(Value::String(value)) if value.is_empty() => Ok(()),
        _ => Err(ProviderError::UnsafeMedia),
    }
}
fn track(value: &Value) -> Result<MediaTrack, ProviderError> {
    reject_required_cookies(value)?;
    if value.get("protocol").and_then(Value::as_str) != Some("https") {
        return Err(ProviderError::UnsupportedFormat);
    }
    let url = MediaUrl::parse(
        value
            .get("url")
            .and_then(Value::as_str)
            .ok_or(ProviderError::MalformedOutput)?,
    )?;
    let codec = value
        .get("vcodec")
        .and_then(Value::as_str)
        .filter(|s| *s != "none")
        .or_else(|| value.get("acodec").and_then(Value::as_str))
        .map(|s| s.chars().take(80).collect());
    let headers = guest_headers(value, &url)?;
    Ok(MediaTrack {
        url,
        headers,
        codec,
        width: value
            .get("width")
            .and_then(Value::as_u64)
            .and_then(|x| u32::try_from(x).ok()),
        height: value
            .get("height")
            .and_then(Value::as_u64)
            .and_then(|x| u32::try_from(x).ok()),
        fps: value.get("fps").and_then(Value::as_f64),
        contains_audio: value
            .get("acodec")
            .and_then(Value::as_str)
            .is_some_and(|s| s != "none"),
    })
}
fn guest_headers(value: &Value, media: &MediaUrl) -> Result<OriginHeaders, ProviderError> {
    let origin = Url::parse(media.expose_url())
        .map_err(|_| ProviderError::UnsafeMedia)?
        .origin()
        .ascii_serialization();
    let mut headers = OriginHeaders {
        origin,
        fields: Vec::new(),
    };
    if let Some(map) = value.get("http_headers").and_then(Value::as_object) {
        for (key, value) in map {
            let name = key.to_ascii_lowercase();
            if ["cookie", "authorization", "proxy-authorization"].contains(&name.as_str()) {
                return Err(ProviderError::UnsafeMedia);
            }
            if ![
                "user-agent",
                "accept",
                "accept-language",
                "sec-fetch-mode",
                "referer",
                "origin",
            ]
            .contains(&name.as_str())
            {
                // Unknown fields may be required by the provider. Never silently
                // downgrade a track by dropping an unrecognized request header.
                return Err(ProviderError::UnsafeMedia);
            }
            let value = value.as_str().ok_or(ProviderError::MalformedOutput)?;
            if name == "sec-fetch-mode" && value != "navigate" {
                return Err(ProviderError::UnsafeMedia);
            }
            if value.len() > 2048 || value.bytes().any(|c| c < 0x20 || c == 0x7f) {
                return Err(ProviderError::UnsafeMedia);
            }
            if name == "referer" || name == "origin" {
                let url = Url::parse(value).map_err(|_| ProviderError::UnsafeMedia)?;
                if url.scheme() != "https"
                    || url.host_str() != Some("www.youtube.com")
                    || !url.username().is_empty()
                    || url.password().is_some()
                    || url.port().is_some()
                    || url.query().is_some()
                    || url.fragment().is_some()
                {
                    return Err(ProviderError::UnsafeMedia);
                }
            }
            headers.fields.push((name, value.to_owned()));
        }
    }
    Ok(headers)
}

fn parse_playback(value: &Value, generation: u64) -> Result<ResolvedPlayback, ProviderError> {
    reject_required_cookies(value)?;
    if is_promoted(value) {
        return Err(ProviderError::Unavailable);
    }
    let (video_track, audio_track) =
        if let Some(formats) = value.get("requested_formats").and_then(Value::as_array) {
            let video = formats
                .iter()
                .find(|v| {
                    v.get("vcodec")
                        .and_then(Value::as_str)
                        .is_some_and(|c| c != "none")
                })
                .ok_or(ProviderError::UnsupportedFormat)?;
            let audio = formats.iter().find(|v| {
                v.get("vcodec").and_then(Value::as_str) == Some("none")
                    && v.get("acodec")
                        .and_then(Value::as_str)
                        .is_some_and(|c| c != "none")
            });
            (track(video)?, audio.map(track).transpose()?)
        } else {
            (track(value)?, None)
        };
    let expires_at = [
        video_track.url.expires_at(),
        audio_track.as_ref().and_then(|t| t.url.expires_at()),
    ]
    .into_iter()
    .flatten()
    .min();
    if expires_at.is_some_and(|expiry| expiry <= SystemTime::now()) {
        return Err(ProviderError::Unavailable);
    }
    let video = summary(value)?;
    let (subtitles, subtitles_truncated) = captions::tracks(value, &video.id);
    Ok(ResolvedPlayback {
        video,
        details: comments::details(value),
        video_track,
        audio_track,
        subtitles,
        subtitles_truncated,
        expires_at,
        session_generation: generation,
        guest: true,
    })
}
fn classify_failure(stderr: &[u8]) -> ProviderError {
    // Raw stderr is never returned or logged: it can contain signed URLs and titles.
    let message = String::from_utf8_lossy(stderr).to_lowercase();
    if message.contains("429") || message.contains("too many requests") {
        ProviderError::RateLimited
    } else if message.contains("po token")
        || message.contains("javascript runtime")
        || message.contains("challenge solver")
    {
        ProviderError::ProofRequired
    } else if message.contains("sign in")
        || message.contains("login required")
        || message.contains("cookies")
    {
        ProviderError::AuthenticationRequired
    } else if message.contains("requested format is not available") {
        ProviderError::UnsupportedFormat
    } else if message.contains("not available")
        || message.contains("private video")
        || message.contains("removed")
    {
        ProviderError::Unavailable
    } else if message.contains("network is unreachable")
        || message.contains("name resolution")
        || message.contains("connection refused")
        || message.contains("temporary failure in name")
    {
        ProviderError::Offline
    } else {
        ProviderError::ExtractorFailed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn format_policy_keeps_resolution_and_fps_before_codec_preference() {
        let args = ResolutionPolicy::default().arguments().unwrap();
        assert!(args[1].contains("height<=1080"));
        assert_eq!(args[3], "height,fps,vcodec:h264");
        assert!(
            ResolutionPolicy {
                max_height: 0,
                prefer_h264: false
            }
            .arguments()
            .is_err()
        );
    }
    #[test]
    fn guest_headers_reject_credentials_injection_and_cross_origin_referers() {
        let url = MediaUrl::parse("https://r1.googlevideo.com/video?token=secret").unwrap();
        for headers in [
            json!({"Cookie":"sensitive"}),
            json!({"Authorization":"sensitive"}),
            json!({"User-Agent":"agent\r\nCookie: sensitive"}),
            json!({"Referer":"https://evil.example/"}),
            json!({"X-Provider-Required":"opaque"}),
            json!({"Sec-Fetch-Mode":"cors"}),
        ] {
            assert!(guest_headers(&json!({"http_headers":headers}), &url).is_err());
        }
        let result = guest_headers(&json!({"http_headers":{"User-Agent":"Example Agent", "Referer":"https://www.youtube.com/", "Sec-Fetch-Mode":"navigate"}}), &url).unwrap();
        assert_eq!(result.origin, "https://r1.googlevideo.com");
        assert_eq!(result.fields.len(), 3);
        assert!(!format!("{result:?}").contains("Example"));
    }

    #[test]
    fn separate_cookie_requirements_are_rejected_at_common_and_selected_track_levels() {
        let base = json!({"id":"abcdefghijk", "url":"https://r1.googlevideo.com/video",
            "protocol":"https", "vcodec":"avc1", "acodec":"aac"});
        assert!(parse_playback(&base, 0).is_ok());
        for cookie in [
            json!("SID=synthetic; Domain=.googlevideo.com"),
            json!(" "),
            Value::Null,
            json!({}),
            json!([]),
            json!(false),
            json!(1),
        ] {
            let mut common = base.clone();
            common["cookies"] = cookie.clone();
            assert!(matches!(
                parse_playback(&common, 0),
                Err(ProviderError::UnsafeMedia)
            ));
            let mut track = base.clone();
            track["cookies"] = cookie;
            let selected = json!({"id":"abcdefghijk", "requested_formats":[track]});
            assert!(matches!(
                parse_playback(&selected, 0),
                Err(ProviderError::UnsafeMedia)
            ));
        }
        let mut empty = base.clone();
        empty["cookies"] = json!("");
        assert!(parse_playback(&empty, 0).is_ok());
        let audio = json!({"url":"https://r2.googlevideo.com/audio", "protocol":"https", "vcodec":"none", "acodec":"opus", "cookies":"synthetic"});
        assert!(matches!(
            parse_playback(
                &json!({"id":"abcdefghijk", "requested_formats":[base,audio]}),
                0
            ),
            Err(ProviderError::UnsafeMedia)
        ));
    }
    #[test]
    fn known_promotions_are_filtered() {
        for value in [
            json!({"promotedVideoRenderer": {}}),
            json!({"wrapper":{"adSlotRenderer":{}}}),
            json!({"id":"abcdefghijk","is_ad":true}),
        ] {
            assert!(is_promoted(&value));
        }
        assert!(!is_promoted(
            &json!({"id":"abcdefghijk","title":"An ad in a title is not a promoted renderer", "unknown": 3})
        ));
    }
    #[test]
    fn optional_metadata_and_unknown_fields_are_tolerated() {
        let video = summary(&json!({"id":"abcdefghijk", "future":{"value":1}})).unwrap();
        assert_eq!(video.title, "Untitled video");
        assert!(video.duration.is_none());
        assert!(summary(&json!({"title":"Missing ID"})).is_err());
    }
    #[test]
    fn unsafe_protocols_and_hosts_fail_closed() {
        assert!(track(&json!({"protocol":"file","url":"file:///etc/passwd"})).is_err());
        assert!(track(&json!({"protocol":"https","url":"https://localhost/media"})).is_err());
    }
    #[test]
    fn errors_never_include_provider_secrets() {
        let error = classify_failure(b"ERROR https://secret.googlevideo.com/?token=private 429");
        assert_eq!(error, ProviderError::RateLimited);
        assert!(!error.to_string().contains("private"));
    }
}
