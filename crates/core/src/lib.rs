//! In-process domain values. No UI, provider JSON, credentials, or graphics handles.
use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime},
};
use url::Url;

mod video_link;
pub use video_link::{VideoLink, VideoStart};

const MAX_URL_BYTES: usize = 16_384;

// Bound untrusted text before the URL parser allocates or canonicalizes it.
// URL parsers may silently discard raw controls; reject those rather than give
// a pasted/provider address a different meaning. Percent-encoded data remains
// subject to each typed address policy below.
fn parse_url(value: &str, error: ProviderError) -> Result<Url, ProviderError> {
    if value.len() > MAX_URL_BYTES || value.chars().any(char::is_control) {
        return Err(error);
    }
    let url = Url::parse(value).map_err(|_| error)?;
    if url.as_str().len() > MAX_URL_BYTES {
        return Err(error);
    }
    Ok(url)
}

#[derive(Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);
impl CancellationToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}
#[derive(Clone)]
pub struct OperationContext {
    pub request_id: u64,
    pub session_generation: u64,
    pub cancel: CancellationToken,
}
impl OperationContext {
    pub fn is_current(&self, request_id: u64, session_generation: u64) -> bool {
        !self.cancel.is_cancelled()
            && self.request_id == request_id
            && self.session_generation == session_generation
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct VideoId(String);
impl VideoId {
    pub fn new(value: &str) -> Result<Self, ProviderError> {
        if value.len() == 11
            && value
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        {
            Ok(Self(value.to_owned()))
        } else {
            Err(ProviderError::InvalidInput)
        }
    }
    pub fn from_url(input: &str) -> Result<Self, ProviderError> {
        let url = parse_url(input, ProviderError::InvalidInput)?;
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
        {
            return Err(ProviderError::InvalidInput);
        }
        match url.host_str() {
            Some("youtu.be") => Self::new(url.path().trim_start_matches('/')),
            Some("youtube.com" | "www.youtube.com" | "m.youtube.com") => {
                if url.path() == "/watch" {
                    let mut ids = url.query_pairs().filter(|(key, _)| key == "v");
                    let id = ids.next().ok_or(ProviderError::InvalidInput)?;
                    if ids.next().is_some() {
                        return Err(ProviderError::InvalidInput);
                    }
                    Self::new(&id.1)
                } else if let Some(id) = url
                    .path()
                    .strip_prefix("/shorts/")
                    .or_else(|| url.path().strip_prefix("/live/"))
                {
                    Self::new(id)
                } else {
                    Err(ProviderError::InvalidInput)
                }
            }
            _ => Err(ProviderError::InvalidInput),
        }
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn watch_url(&self) -> String {
        format!("https://www.youtube.com/watch?v={}", self.0)
    }
}
impl fmt::Debug for VideoId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("VideoId([redacted])")
    }
}
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ChannelId(pub String);
/// Public channel address, never a verified/stable channel identity.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ChannelHandle(String);
impl ChannelHandle {
    pub fn new(value: &str) -> Result<Self, ProviderError> {
        let value = value.strip_prefix('@').ok_or(ProviderError::InvalidInput)?;
        // This is a bounded address parser, not YouTube's registration policy:
        // mixed-script length and eligibility remain the provider's decision.
        if value.is_empty()
            || value.len() > 400
            || value.chars().count() > 100
            || value.starts_with(['_', '-', '.', '·'])
            || value.ends_with(['_', '-', '.', '·'])
            || value.chars().any(|c| {
                c.is_control()
                    || c.is_whitespace()
                    || (c.is_ascii() && !c.is_ascii_alphanumeric() && !"_.-".contains(c))
                    || matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}')
            })
        {
            return Err(ProviderError::InvalidInput);
        }
        Ok(Self(value.to_owned()))
    }
    pub fn from_url(value: &str) -> Result<Self, ProviderError> {
        // Reject path rewrites before URL parsing normalizes dot segments or
        // treats backslashes as separators in special-scheme URLs.
        if value.len() > MAX_URL_BYTES
            || value.contains('\\')
            || value.split('/').any(|part| {
                matches!(
                    part.replace("%2e", ".").replace("%2E", ".").as_str(),
                    "." | ".."
                )
            })
        {
            return Err(ProviderError::InvalidInput);
        }
        let url = catalog_url(value)?;
        if url.query().is_some() || url.fragment().is_some() {
            return Err(ProviderError::InvalidInput);
        }
        let mut parts = url.path().strip_prefix('/').unwrap_or("").split('/');
        let encoded = parts.next().ok_or(ProviderError::InvalidInput)?;
        if let Some(tab) = parts.next()
            && !["", "videos", "shorts", "streams", "playlists", "featured"].contains(&tab)
        {
            return Err(ProviderError::InvalidInput);
        }
        if parts.next().is_some() || encoded.len() > 1203 {
            return Err(ProviderError::InvalidInput);
        }
        let mut decoded = Vec::with_capacity(encoded.len());
        let mut bytes = encoded.bytes();
        while let Some(byte) = bytes.next() {
            if byte == b'%' {
                let high = bytes.next().and_then(|b| (b as char).to_digit(16));
                let low = bytes.next().and_then(|b| (b as char).to_digit(16));
                let (Some(high), Some(low)) = (high, low) else {
                    return Err(ProviderError::InvalidInput);
                };
                decoded.push((high * 16 + low) as u8);
            } else {
                decoded.push(byte);
            }
        }
        let decoded = std::str::from_utf8(&decoded).map_err(|_| ProviderError::InvalidInput)?;
        Self::new(decoded)
    }
    pub fn browse_url(&self) -> String {
        let mut url = Url::parse("https://www.youtube.com/").expect("fixed HTTPS URL");
        url.set_path(&format!("/@{}", self.0));
        url.into()
    }
}
impl fmt::Debug for ChannelHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ChannelHandle([redacted])")
    }
}
#[cfg(test)]
mod channel_handle_tests {
    use super::*;

    #[test]
    fn handles_roundtrip_unicode_without_allowing_address_rewrites() {
        for input in [
            "@synthetic-channel",
            "@日本語",
            "@český·kanál",
            "@a\u{301}bc",
        ] {
            let handle = ChannelHandle::new(input).unwrap();
            assert_eq!(
                ChannelHandle::from_url(&handle.browse_url()).unwrap(),
                handle
            );
        }
        for input in [
            "https://youtube.com.evil/@synthetic",
            "https://user@youtube.com/@synthetic",
            "http://youtube.com/@synthetic",
            "https://youtube.com:444/@synthetic",
            "https://youtube.com/@synthetic?secret=value",
            "https://youtube.com/@synthetic#fragment",
            "https://youtube.com/@synthetic%2fother",
            "https://youtube.com/@synthetic%00",
            "https://youtube.com/@synthetic/../@other",
            "https://youtube.com/@synthetic/%2e%2e/@other",
            "https://youtube.com/@synthetic\\videos",
            "https://youtube.com/@synthetic/videos/extra",
            "https://youtube.com/@synthetic%FF",
            "https://youtube.com/@synthetic%zz",
        ] {
            assert!(ChannelHandle::from_url(input).is_err(), "{input}");
        }
        assert!(ChannelHandle::new("@name tutorial").is_err());
        assert!(ChannelHandle::new("@name\u{202e}").is_err());
        assert!(ChannelHandle::new(&format!("@{}", "x".repeat(101))).is_err());
    }
}
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PlaylistId(pub String);
impl ChannelId {
    pub fn new(value: &str) -> Result<Self, ProviderError> {
        if value.len() == 24
            && value.starts_with("UC")
            && value
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        {
            Ok(Self(value.to_owned()))
        } else {
            Err(ProviderError::InvalidInput)
        }
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn from_url(value: &str) -> Result<Self, ProviderError> {
        let url = catalog_url(value)?;
        if url.query().is_some() || url.fragment().is_some() {
            return Err(ProviderError::InvalidInput);
        }
        let path = url
            .path()
            .strip_prefix("/channel/")
            .ok_or(ProviderError::InvalidInput)?;
        let mut parts = path.split('/');
        let id = Self::new(parts.next().ok_or(ProviderError::InvalidInput)?)?;
        if let Some(tab) = parts.next()
            && !["", "videos", "shorts", "streams", "playlists", "featured"].contains(&tab)
        {
            return Err(ProviderError::InvalidInput);
        }
        if parts.next().is_some() {
            return Err(ProviderError::InvalidInput);
        }
        Ok(id)
    }
    pub fn browse_url(&self) -> String {
        format!("https://www.youtube.com/channel/{}", self.0)
    }
}
impl PlaylistId {
    pub fn new(value: &str) -> Result<Self, ProviderError> {
        if (2..=128).contains(&value.len())
            && value
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        {
            Ok(Self(value.to_owned()))
        } else {
            Err(ProviderError::InvalidInput)
        }
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn from_url(value: &str) -> Result<Self, ProviderError> {
        let url = catalog_url(value)?;
        if !["/playlist", "/watch"].contains(&url.path()) || url.fragment().is_some() {
            return Err(ProviderError::InvalidInput);
        }
        let mut ids = url.query_pairs().filter(|(key, _)| key == "list");
        let id = ids.next().ok_or(ProviderError::InvalidInput)?;
        if ids.next().is_some() {
            return Err(ProviderError::InvalidInput);
        }
        Self::new(&id.1)
    }
    pub fn browse_url(&self) -> String {
        format!("https://www.youtube.com/playlist?list={}", self.0)
    }
}
fn catalog_url(value: &str) -> Result<Url, ProviderError> {
    let url = parse_url(value, ProviderError::InvalidInput)?;
    if url.scheme() != "https"
        || !matches!(
            url.host_str(),
            Some("www.youtube.com" | "youtube.com" | "m.youtube.com")
        )
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return Err(ProviderError::InvalidInput);
    }
    Ok(url)
}
#[derive(Clone)]
pub struct ChannelSummary {
    pub id: ChannelId,
    pub title: String,
    pub description: Option<String>,
    pub thumbnail_url: Option<String>,
    pub subscriber_count: Option<u64>,
}
#[derive(Clone)]
pub struct PlaylistSummary {
    pub id: PlaylistId,
    pub title: String,
    pub description: Option<String>,
    pub channel: Option<String>,
    pub channel_id: Option<ChannelId>,
    pub thumbnail_url: Option<String>,
    pub video_count: Option<u64>,
}
#[derive(Clone)]
pub enum CatalogItem {
    Video(VideoSummary),
    Channel(ChannelSummary),
    Playlist(PlaylistSummary),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProfileId(pub u64);
#[derive(Clone)]
pub struct VideoSummary {
    pub id: VideoId,
    pub title: String,
    pub channel: String,
    pub channel_id: Option<ChannelId>,
    pub duration: Option<Duration>,
    pub thumbnail_url: Option<String>,
}

/// Short-lived media address. Explicit access is required to pass it to a media engine.
#[derive(Clone)]
pub struct MediaUrl(String);
impl MediaUrl {
    /// Initial guest policy: HTTPS content hosted by Google's video CDN only.
    /// This validates the initial address; the transport must validate redirects separately.
    pub fn parse(value: &str) -> Result<Self, ProviderError> {
        let url = parse_url(value, ProviderError::UnsafeMedia)?;
        let host = url.host_str().ok_or(ProviderError::UnsafeMedia)?;
        if url.scheme() != "https"
            || !host.ends_with(".googlevideo.com")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
            || url.fragment().is_some()
        {
            return Err(ProviderError::UnsafeMedia);
        }
        Ok(Self(url.into()))
    }
    pub fn expose_url(&self) -> &str {
        &self.0
    }
    pub fn expires_at(&self) -> Option<SystemTime> {
        let url = Url::parse(&self.0).ok()?;
        let value = url
            .query_pairs()
            .find(|(k, _)| k == "expire")?
            .1
            .parse::<u64>()
            .ok()?;
        SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(value))
    }
}
impl fmt::Debug for MediaUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MediaUrl([redacted])")
    }
}
impl fmt::Display for MediaUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted media URL]")
    }
}
/// Headers are valid only for the exact origin of their associated track.
/// Guest adapters must never place account credentials here.
#[derive(Clone, Default)]
pub struct OriginHeaders {
    pub origin: String,
    pub fields: Vec<(String, String)>,
}
impl fmt::Debug for OriginHeaders {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "OriginHeaders([redacted]; {} fields)", self.fields.len())
    }
}
#[derive(Clone, Debug)]
pub struct MediaTrack {
    pub url: MediaUrl,
    pub codec: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub contains_audio: bool,
    pub headers: OriginHeaders,
}
#[derive(Clone)]
pub struct SubtitleTrack {
    pub language: String,
    pub label: String,
    pub automatic: bool,
    pub url: CaptionUrl,
    pub format: String,
}
/// Signed caption address, deliberately separate from the content-CDN policy.
#[derive(Clone)]
pub struct CaptionUrl {
    url: Url,
    video: VideoId,
}
impl CaptionUrl {
    pub fn parse(value: &str, video: &VideoId) -> Result<Self, ProviderError> {
        let url = parse_url(value, ProviderError::UnsafeMedia)?;
        if url.scheme() != "https"
            || url.host_str() != Some("www.youtube.com")
            || url.path() != "/api/timedtext"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
            || url.fragment().is_some()
        {
            return Err(ProviderError::UnsafeMedia);
        }
        let pairs: Vec<_> = url.query_pairs().collect();
        if pairs.len() > 64
            || pairs.iter().filter(|(key, _)| key == "v").count() != 1
            || pairs.iter().filter(|(key, _)| key == "fmt").count() != 1
            || !pairs
                .iter()
                .any(|(key, value)| key == "v" && value == video.as_str())
            || !pairs
                .iter()
                .any(|(key, value)| key == "fmt" && value == "vtt")
        {
            return Err(ProviderError::UnsafeMedia);
        }
        Ok(Self {
            url,
            video: video.clone(),
        })
    }
    pub fn expose_url(&self) -> &str {
        self.url.as_str()
    }
    pub fn video_id(&self) -> &VideoId {
        &self.video
    }
}
impl fmt::Debug for CaptionUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CaptionUrl([redacted])")
    }
}
impl fmt::Display for CaptionUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted caption URL]")
    }
}
#[derive(Clone)]
pub struct ResolvedPlayback {
    pub video: VideoSummary,
    pub details: VideoDetails,
    pub video_track: MediaTrack,
    pub audio_track: Option<MediaTrack>,
    pub subtitles: Vec<SubtitleTrack>,
    pub subtitles_truncated: bool,
    pub expires_at: Option<SystemTime>,
    pub session_generation: u64,
    /// Guest resolver never supplies cookies or arbitrary extractor-provided headers.
    pub guest: bool,
}
pub const MAX_VIDEO_CHAPTERS: usize = 200;
#[derive(Clone, Debug)]
pub struct VideoChapter {
    pub title: Option<String>,
    pub start: Duration,
    pub end: Duration,
}
/// Optional metadata; absence never implies zero counts or an empty description.
#[derive(Clone, Debug, Default)]
pub struct VideoDetails {
    pub description: Option<String>,
    pub upload_date: Option<String>,
    pub view_count: Option<u64>,
    pub like_count: Option<u64>,
    pub comment_count: Option<u64>,
    pub channel_subscriber_count: Option<u64>,
    pub chapters: Vec<VideoChapter>,
    /// Present but malformed/oversized chapter metadata was rejected as a set.
    pub chapters_unavailable: bool,
}
/// Read-only public top-level comment. No provider JSON or account write capability.
#[derive(Clone)]
pub struct CommentSummary {
    pub id: String,
    pub author: Option<String>,
    pub author_id: Option<ChannelId>,
    pub text: String,
    pub published_text: Option<String>,
    pub like_count: Option<u64>,
    pub author_is_uploader: bool,
    /// Provider-supplied public portrait URL, already restricted by the provider
    /// adapter to an HTTPS Google avatar host. The app still applies its own
    /// exact-host fetch policy; absence means "show the placeholder".
    pub author_thumbnail_url: Option<String>,
}
/// A single controlled refresh attempt owned by one playback lifecycle. Preserve
/// this budget when replacing expired stream URLs; reset only for a new selection.
#[derive(Default)]
pub struct RefreshBudget {
    attempted: bool,
}
#[derive(Clone)]
pub struct StreamRefreshRequest {
    pub video_id: VideoId,
    pub resume_position: Duration,
    pub session_generation: u64,
    pub authenticated: bool,
}
impl ResolvedPlayback {
    pub fn expires_within(&self, now: SystemTime, margin: Duration) -> bool {
        let threshold = now.checked_add(margin).unwrap_or(now);
        self.expires_at.is_some_and(|expiry| expiry <= threshold)
    }
}
impl RefreshBudget {
    /// Unknown expiry and arbitrary HTTP403/errors never authorize a retry.
    pub fn request(
        &mut self,
        item: &ResolvedPlayback,
        position: Duration,
        current_generation: u64,
        now: SystemTime,
        margin: Duration,
    ) -> Option<StreamRefreshRequest> {
        if self.attempted
            || item.session_generation != current_generation
            || !item.expires_within(now, margin)
        {
            return None;
        }
        self.attempted = true;
        Some(StreamRefreshRequest {
            video_id: item.video.id.clone(),
            resume_position: position,
            session_generation: current_generation,
            authenticated: !item.guest,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderError {
    InvalidInput,
    Busy,
    Cancelled,
    Timeout,
    HelperUnavailable,
    UnsupportedPlatform,
    OutputTooLarge,
    MalformedOutput,
    Offline,
    Unavailable,
    AuthenticationRequired,
    RateLimited,
    ProofRequired,
    UnsupportedFormat,
    UnsafeMedia,
    ExtractorFailed,
}
impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidInput => "Enter a supported HTTPS YouTube URL or a search of 1–200 characters.",
            Self::Busy => "The previous request is still stopping. Try again shortly.",
            Self::Cancelled => "Request cancelled.", Self::Timeout => "YouTube took too long to respond. Try again.",
            Self::HelperUnavailable => "The configured yt-dlp helper is unavailable.",
            Self::UnsupportedPlatform => "Safe extractor process supervision is not yet implemented on this platform.",
            Self::OutputTooLarge => "The provider response exceeded the safety limit.",
            Self::MalformedOutput => "The extractor returned an unsupported response.",
            Self::Offline => "Cannot reach YouTube. Check your connection and retry.",
            Self::Unavailable => "The requested YouTube content is unavailable or restricted.",
            Self::AuthenticationRequired => "YouTube requires an authorized account session. Guest requests are not automatically authenticated.",
            Self::RateLimited => "YouTube is limiting requests. Wait before trying again.",
            Self::ProofRequired => "This stream requires unsupported challenge or proof-token tooling.",
            Self::UnsupportedFormat => "No supported direct HTTPS media format is available.",
            Self::UnsafeMedia => "The provider returned a media address outside the supported security policy.",
            Self::ExtractorFailed => "YouTube extraction failed. Retry or check the installed helper version.",
        })
    }
}
impl std::error::Error for ProviderError {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Preferences {
    pub local_history: bool,
    pub autoplay: bool,
    pub thumbnail_previews: bool,
    pub background_refresh: bool,
    pub telemetry: bool,
}

/// Supported user-selected ceilings; this does not promise source availability.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum QualityCeiling {
    #[default]
    P1080,
    P720,
    P480,
    P360,
    P240,
    P144,
}
impl QualityCeiling {
    pub const ALL: [Self; 6] = [
        Self::P1080,
        Self::P720,
        Self::P480,
        Self::P360,
        Self::P240,
        Self::P144,
    ];
    pub const fn height(self) -> u16 {
        match self {
            Self::P1080 => 1080,
            Self::P720 => 720,
            Self::P480 => 480,
            Self::P360 => 360,
            Self::P240 => 240,
            Self::P144 => 144,
        }
    }
    pub fn from_height(height: u16) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|quality| quality.height() == height)
    }
    pub fn from_index(index: i32) -> Option<Self> {
        usize::try_from(index)
            .ok()
            .and_then(|index| Self::ALL.get(index).copied())
    }
    pub fn index(self) -> i32 {
        Self::ALL
            .iter()
            .position(|quality| *quality == self)
            .unwrap() as i32
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlaybackSpeed {
    Half,
    #[default]
    Normal,
    OneAndHalf,
    Double,
}
impl PlaybackSpeed {
    pub const ALL: [Self; 4] = [Self::Half, Self::Normal, Self::OneAndHalf, Self::Double];
    pub const fn millis(self) -> u16 {
        match self {
            Self::Half => 500,
            Self::Normal => 1000,
            Self::OneAndHalf => 1500,
            Self::Double => 2000,
        }
    }
    pub fn rate(self) -> f64 {
        f64::from(self.millis()) / 1000.
    }
    pub fn from_millis(millis: u16) -> Option<Self> {
        Self::ALL.into_iter().find(|speed| speed.millis() == millis)
    }
    pub fn from_rate(rate: f64) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|speed| (speed.rate() - rate).abs() < 0.000_001)
    }
    pub fn index(self) -> i32 {
        Self::ALL.iter().position(|speed| *speed == self).unwrap() as i32
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlaybackPreferences {
    pub quality: QualityCeiling,
    pub speed: PlaybackSpeed,
}

#[cfg(test)]
mod url_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn playback_presets_roundtrip_and_reject_unsupported_values() {
        assert_eq!(PlaybackPreferences::default().quality.height(), 1080);
        assert_eq!(PlaybackPreferences::default().speed.rate(), 1.);
        for quality in QualityCeiling::ALL {
            assert_eq!(QualityCeiling::from_height(quality.height()), Some(quality));
            assert_eq!(QualityCeiling::from_index(quality.index()), Some(quality));
        }
        for speed in PlaybackSpeed::ALL {
            assert_eq!(PlaybackSpeed::from_millis(speed.millis()), Some(speed));
            assert_eq!(PlaybackSpeed::from_rate(speed.rate()), Some(speed));
        }
        for height in [0, 143, 145, 2160, u16::MAX] {
            assert_eq!(QualityCeiling::from_height(height), None);
        }
        for speed in [f64::NAN, f64::INFINITY, 0., 0.25, 1.25, 4.] {
            assert_eq!(PlaybackSpeed::from_rate(speed), None);
        }
        assert_eq!(QualityCeiling::from_index(-1), None);
        assert_eq!(QualityCeiling::from_index(6), None);
    }
    #[test]
    fn urls_are_restricted() {
        assert!(
            ChannelId::from_url("https://www.youtube.com/channel/UCabcdefghijklmnopqrstuv/videos")
                .is_ok()
        );
        assert!(PlaylistId::from_url("https://www.youtube.com/playlist?list=PLsynthetic").is_ok());
        assert!(
            PlaylistId::from_url("https://www.youtube.com/playlist?list=PLone&list=PLtwo").is_err()
        );
        assert!(
            ChannelId::from_url(
                "https://www.youtube.com/channel/UCabcdefghijklmnopqrstuv/videos/extra"
            )
            .is_err()
        );
        assert!(
            ChannelId::from_url("https://youtube.com.evil/channel/UCabcdefghijklmnopqrstuv")
                .is_err()
        );
        assert!(PlaylistId::new("PLunsafe&secret").is_err());
        assert!(VideoId::from_url("https://youtu.be/abcdefghijk").is_ok());
        for url in [
            "file:///etc/passwd",
            "https://youtube.com.evil/watch?v=abcdefghijk",
            "https://user@youtube.com/watch?v=abcdefghijk",
            "https://127.0.0.1/watch?v=abcdefghijk",
        ] {
            assert!(VideoId::from_url(url).is_err());
        }
        assert!(MediaUrl::parse("https://r1.googlevideo.com/videoplayback?secret=hidden").is_ok());
        assert!(MediaUrl::parse("https://r1.googlevideo.com.evil/video").is_err());
        assert!(MediaUrl::parse("http://r1.googlevideo.com/video").is_err());
    }
    #[test]
    fn media_debug_redacts_secrets() {
        let url = MediaUrl::parse("https://r1.googlevideo.com/video?secret=hidden").unwrap();
        assert!(!format!("{url:?} {url}").contains("hidden"));
    }
    #[test]
    fn stale_or_cancelled_results_are_rejected() {
        let op = OperationContext {
            request_id: 1,
            session_generation: 2,
            cancel: CancellationToken::default(),
        };
        assert!(op.is_current(1, 2));
        assert!(!op.is_current(2, 2));
        assert!(!op.is_current(1, 3));
        op.cancel.cancel();
        assert!(!op.is_current(1, 2));
    }
    #[test]
    fn refresh_is_once_only_known_expiry_and_generation_scoped() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let mut item = ResolvedPlayback {
            details: VideoDetails::default(),
            video: VideoSummary {
                id: VideoId::new("abcdefghijk").unwrap(),
                title: String::new(),
                channel: String::new(),
                channel_id: None,
                duration: None,
                thumbnail_url: None,
            },
            video_track: MediaTrack {
                url: MediaUrl::parse("https://r1.googlevideo.com/video").unwrap(),
                codec: None,
                width: None,
                height: None,
                fps: None,
                contains_audio: true,
                headers: OriginHeaders::default(),
            },
            audio_track: None,
            subtitles: Vec::new(),
            subtitles_truncated: false,
            expires_at: None,
            session_generation: 7,
            guest: true,
        };
        let mut budget = RefreshBudget::default();
        assert!(
            budget
                .request(&item, Duration::ZERO, 7, now, Duration::from_secs(30))
                .is_none()
        );
        item.expires_at = Some(now);
        assert!(
            budget
                .request(&item, Duration::ZERO, 8, now, Duration::from_secs(30))
                .is_none()
        );
        let request = budget
            .request(&item, Duration::from_secs(42), 7, now, Duration::ZERO)
            .unwrap();
        assert_eq!(request.resume_position, Duration::from_secs(42));
        assert!(!request.authenticated);
        assert!(
            budget
                .request(&item, Duration::ZERO, 7, now, Duration::ZERO)
                .is_none()
        );
    }
    #[test]
    fn private_defaults() {
        assert_eq!(
            Preferences::default(),
            Preferences {
                local_history: false,
                autoplay: false,
                thumbnail_previews: false,
                background_refresh: false,
                telemetry: false
            }
        );
    }
}
