// SPDX-License-Identifier: GPL-3.0-or-later
//! Shared bounded normalization of reviewed InnerTube video renderer shapes.
//! Used by the account parser and the guest watch-page (`next`) adapter.
use serde_json::Value;
use serein_core::{ChannelId, VideoId, VideoSummary};

pub(crate) enum VideoEntry {
    Video(VideoSummary),
    /// Deliberately omitted (advertising or Shorts); not an interpretation failure.
    Filtered,
    Unsupported,
}

/// Plain text from a string, `{simpleText}`, view-model `{content}` or `{runs}`,
/// bounded to 500 characters. Missing values are empty.
pub(crate) fn text(value: Option<&Value>) -> String {
    let Some(value) = value else {
        return String::new();
    };
    if let Some(value) = value
        .as_str()
        .or_else(|| value.get("simpleText")?.as_str())
        .or_else(|| value.get("content")?.as_str())
    {
        return value.chars().take(500).collect();
    }
    value
        .get("runs")
        .and_then(Value::as_array)
        .map(|runs| {
            runs.iter()
                .filter_map(|r| r.get("text")?.as_str())
                .flat_map(str::chars)
                .take(500)
                .collect()
        })
        .unwrap_or_default()
}

/// `videoRenderer` and the watch-next `compactVideoRenderer` share this shape.
pub(crate) fn video_renderer(video: &Value) -> VideoEntry {
    if video
        .pointer("/navigationEndpoint/reelWatchEndpoint")
        .is_some()
    {
        return VideoEntry::Filtered;
    }
    let Some(id) = video
        .get("videoId")
        .and_then(Value::as_str)
        .and_then(|id| VideoId::new(id).ok())
    else {
        return VideoEntry::Unsupported;
    };
    if let Some(target) = video.pointer("/navigationEndpoint/watchEndpoint/videoId")
        && target.as_str() != Some(id.as_str())
    {
        return VideoEntry::Unsupported;
    }
    let byline = ["ownerText", "shortBylineText", "longBylineText"]
        .iter()
        .find_map(|key| video.get(*key));
    VideoEntry::Video(VideoSummary {
        id,
        title: title_or_default(text(video.get("title"))),
        channel: channel_or_default(text(byline)),
        channel_id: byline
            .and_then(|b| b.pointer("/runs/0/navigationEndpoint/browseEndpoint/browseId"))
            .and_then(Value::as_str)
            .and_then(|id| ChannelId::new(id).ok()),
        duration: duration_text(&text(video.get("lengthText"))),
        thumbnail_url: thumbnail(video.pointer("/thumbnail/thumbnails")),
    })
}

pub(crate) fn video_lockup(lockup: &Value) -> VideoEntry {
    if lockup.get("contentType").and_then(Value::as_str) != Some("LOCKUP_CONTENT_TYPE_VIDEO") {
        // Mixes, playlists and other lockups are not playable single videos here.
        return VideoEntry::Unsupported;
    }
    let command = lockup.pointer("/rendererContext/commandContext/onTap/innertubeCommand");
    if command.is_some_and(|c| c.get("reelWatchEndpoint").is_some()) {
        return VideoEntry::Filtered;
    }
    let Some(id) = lockup
        .get("contentId")
        .and_then(Value::as_str)
        .and_then(|id| VideoId::new(id).ok())
    else {
        return VideoEntry::Unsupported;
    };
    if let Some(target) = command.and_then(|c| c.pointer("/watchEndpoint/videoId"))
        && target.as_str() != Some(id.as_str())
    {
        return VideoEntry::Unsupported;
    }
    let metadata = lockup.pointer("/metadata/lockupMetadataViewModel");
    let duration = lockup
        .pointer("/contentImage/thumbnailViewModel/overlays")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(8)
        .flat_map(|overlay| {
            overlay
                .pointer("/thumbnailOverlayBadgeViewModel/thumbnailBadges")
                .or_else(|| overlay.pointer("/thumbnailBottomOverlayViewModel/badges"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .take(8)
        })
        .find_map(|badge| {
            duration_text(
                badge
                    .pointer("/thumbnailBadgeViewModel/text")
                    .and_then(Value::as_str)?,
            )
        });
    VideoEntry::Video(VideoSummary {
        id,
        title: title_or_default(text(metadata.and_then(|m| m.get("title")))),
        channel: channel_or_default(text(metadata.and_then(|m| {
            m.pointer("/metadata/contentMetadataViewModel/metadataRows/0/metadataParts/0/text")
        }))),
        channel_id: metadata
            .and_then(|m| {
                m.pointer("/image/decoratedAvatarViewModel/rendererContext/commandContext/onTap/innertubeCommand/browseEndpoint/browseId")
            })
            .and_then(Value::as_str)
            .and_then(|id| ChannelId::new(id).ok()),
        duration,
        thumbnail_url: thumbnail(lockup.pointer("/contentImage/thumbnailViewModel/image/sources")),
    })
}
fn title_or_default(title: String) -> String {
    if title.trim().is_empty() {
        "Untitled video".into()
    } else {
        title
    }
}
fn channel_or_default(channel: String) -> String {
    if channel.trim().is_empty() {
        "Unknown channel".into()
    } else {
        channel.chars().take(200).collect()
    }
}
/// Card-sized candidate from a bounded list; only reviewed HTTPS artwork hosts.
pub(crate) fn thumbnail(value: Option<&Value>) -> Option<String> {
    let mut best: Option<(u64, &str)> = None;
    for candidate in value?.as_array()?.iter().take(16) {
        let Some(url) = candidate.get("url").and_then(Value::as_str) else {
            continue;
        };
        let width = candidate.get("width").and_then(Value::as_u64).unwrap_or(0);
        let better = match best {
            None => true,
            Some((current, _)) if current > 720 => width < current,
            Some((current, _)) => width <= 720 && width > current,
        };
        if better {
            best = Some((width, url));
        }
    }
    let url = best?.1;
    if url.len() > 2048 {
        return None;
    }
    match url.strip_prefix("//") {
        Some(rest) => crate::safe_thumbnail(&format!("https://{rest}")),
        None => crate::safe_thumbnail(url),
    }
}
/// `M:SS` or `H:MM:SS`; anything else (LIVE, localized words) is unknown.
pub(crate) fn duration_text(value: &str) -> Option<std::time::Duration> {
    let parts: Vec<_> = value.trim().split(':').collect();
    if !(2..=3).contains(&parts.len()) {
        return None;
    }
    let mut total = 0u64;
    for (index, part) in parts.iter().enumerate() {
        if part.is_empty() || part.len() > 4 || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let number: u64 = part.parse().ok()?;
        if index > 0 && (number >= 60 || part.len() != 2) {
            return None;
        }
        total = total * 60 + number;
    }
    (total <= 31_536_000).then(|| std::time::Duration::from_secs(total))
}
