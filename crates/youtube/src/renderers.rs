// SPDX-License-Identifier: GPL-3.0-or-later
//! Bounded normalization of reviewed public InnerTube item renderers, shared by
//! the signed-in feed parser and the native guest catalog. Every identifier is
//! validated by its core constructor and every image URL by the exact-host
//! thumbnail/avatar policies; unknown shapes are reported, never invented.
use oxplay_core::{ChannelId, ChannelSummary, PlaylistId, PlaylistSummary, VideoId, VideoSummary};
use serde_json::Value;
use std::time::Duration;

/// Outcome of interpreting one item renderer.
pub(crate) enum Parsed<T> {
    Item(T),
    /// Deliberately omitted (advertising, Shorts in a mixed list, mixes); not
    /// an interpretation failure.
    Filtered,
    Unsupported,
}

/// Plain bounded text from a string, `{simpleText}`, `{content}` or `{runs}`.
pub(crate) fn text(value: Option<&Value>) -> String {
    bounded_text(value, 500)
}

pub(crate) fn bounded_text(value: Option<&Value>, limit: usize) -> String {
    let Some(value) = value else {
        return String::new();
    };
    let keep = |c: &char| !c.is_control() || *c == '\n';
    if let Some(value) = value
        .as_str()
        .or_else(|| value.get("simpleText")?.as_str())
        .or_else(|| value.get("content")?.as_str())
    {
        return value.chars().filter(keep).take(limit).collect();
    }
    value
        .get("runs")
        .and_then(Value::as_array)
        .map(|runs| {
            runs.iter()
                .take(256)
                .filter_map(|r| r.get("text")?.as_str())
                .flat_map(str::chars)
                .filter(keep)
                .take(limit)
                .collect()
        })
        .unwrap_or_default()
}

fn optional_text(value: Option<&Value>, limit: usize) -> Option<String> {
    let text = bounded_text(value, limit);
    (!text.trim().is_empty()).then_some(text)
}

pub(crate) fn title_or_default(title: String) -> String {
    if title.trim().is_empty() {
        "Untitled video".into()
    } else {
        title
    }
}

pub(crate) fn channel_or_default(channel: String) -> String {
    if channel.trim().is_empty() {
        "Unknown channel".into()
    } else {
        channel.chars().take(200).collect()
    }
}

/// Card-sized candidate from a bounded list, passed through `policy`.
fn best_image(value: Option<&Value>, policy: fn(&str) -> Option<String>) -> Option<String> {
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
        Some(rest) => policy(&format!("https://{rest}")),
        None => policy(url),
    }
}

/// Only reviewed HTTPS artwork hosts (`crate::safe_thumbnail`).
pub(crate) fn thumbnail(value: Option<&Value>) -> Option<String> {
    best_image(value, crate::safe_thumbnail)
}

/// Only the exact public portrait hosts (`channel_avatar::safe_avatar`).
pub(crate) fn avatar(value: Option<&Value>) -> Option<String> {
    best_image(value, crate::channel_avatar::safe_avatar)
}

/// `M:SS` or `H:MM:SS`; anything else (LIVE, localized words) is unknown.
pub(crate) fn duration_text(value: &str) -> Option<Duration> {
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
    (total <= 31_536_000).then(|| Duration::from_secs(total))
}

/// English display counts from the fixed `hl=en` guest context, e.g.
/// "577K subscribers", "1.2M subscribers", "1,234 videos", "No videos".
/// The text must name `unit`; abbreviated values are necessarily approximate.
pub(crate) fn count_text(value: &str, unit: &str) -> Option<u64> {
    let value = value.trim().to_ascii_lowercase();
    if value.len() > 64 || !value.contains(unit) {
        return None;
    }
    if value.starts_with("no ") {
        return Some(0);
    }
    let number = value.split_whitespace().next()?;
    let (digits, multiplier) = match number.chars().last()? {
        'k' => (&number[..number.len() - 1], 1_000u64),
        'm' => (&number[..number.len() - 1], 1_000_000),
        'b' => (&number[..number.len() - 1], 1_000_000_000),
        _ => (number, 1),
    };
    let digits = digits.replace(',', "");
    if digits.is_empty()
        || digits.len() > 16
        || !digits.bytes().all(|b| b.is_ascii_digit() || b == b'.')
    {
        return None;
    }
    let (whole, fraction) = digits.split_once('.').unwrap_or((&digits, ""));
    if whole.is_empty() || fraction.contains('.') || (multiplier == 1 && !fraction.is_empty()) {
        return None;
    }
    let whole: u64 = whole.parse().ok()?;
    let mut total = whole.checked_mul(multiplier)?;
    let mut scale = multiplier;
    for digit in fraction.bytes().take(9) {
        scale /= 10;
        total = total.checked_add(u64::from(digit - b'0') * scale)?;
    }
    (total <= 100_000_000_000_000).then_some(total)
}

fn browse_channel(endpoint: Option<&Value>) -> Option<ChannelId> {
    endpoint?
        .pointer("/browseEndpoint/browseId")
        .and_then(Value::as_str)
        .and_then(|id| ChannelId::new(id).ok())
}

/// A bounded `BADGE_STYLE_TYPE_AD` marker on a video renderer.
pub(crate) fn has_ad_badge(renderer: &Value) -> bool {
    renderer
        .get("badges")
        .and_then(Value::as_array)
        .is_some_and(|badges| {
            badges.iter().take(16).any(|badge| {
                badge
                    .pointer("/metadataBadgeRenderer/style")
                    .and_then(Value::as_str)
                    == Some("BADGE_STYLE_TYPE_AD")
            })
        })
}

/// `videoRenderer`, and the equivalent `playlistVideoRenderer` fields.
pub(crate) fn video_renderer(video: &Value) -> Parsed<VideoSummary> {
    if video
        .pointer("/navigationEndpoint/reelWatchEndpoint")
        .is_some()
    {
        return Parsed::Filtered;
    }
    if has_ad_badge(video) {
        return Parsed::Filtered;
    }
    let Some(id) = video
        .get("videoId")
        .and_then(Value::as_str)
        .and_then(|id| VideoId::new(id).ok())
    else {
        return Parsed::Unsupported;
    };
    if let Some(target) = video.pointer("/navigationEndpoint/watchEndpoint/videoId")
        && target.as_str() != Some(id.as_str())
    {
        return Parsed::Unsupported;
    }
    let byline = ["ownerText", "shortBylineText", "longBylineText"]
        .iter()
        .find_map(|key| video.get(*key));
    Parsed::Item(VideoSummary {
        id,
        title: title_or_default(text(video.get("title"))),
        channel: channel_or_default(text(byline)),
        channel_id: byline
            .and_then(|b| b.pointer("/runs/0/navigationEndpoint"))
            .and_then(|endpoint| browse_channel(Some(endpoint))),
        duration: duration_text(&text(video.get("lengthText"))),
        thumbnail_url: thumbnail(video.pointer("/thumbnail/thumbnails")),
        metadata: display_metadata(
            [
                text(
                    video
                        .get("shortViewCountText")
                        .or_else(|| video.get("viewCountText")),
                ),
                text(video.get("publishedTimeText")),
            ]
            .into_iter(),
        ),
    })
}

/// Joins short display parts ("733K", "4d ago") into one bounded line; a bare
/// abbreviated count gains " views" so related rows read like search results.
pub(crate) fn display_metadata(parts: impl Iterator<Item = String>) -> Option<String> {
    let parts: Vec<String> = parts
        .map(|part| part.trim().to_owned())
        .filter(|part| !part.is_empty() && part.chars().count() <= 40)
        .map(|part| {
            let bare = part
                .trim_end_matches(['K', 'M', 'B'])
                .chars()
                .all(|c| c.is_ascii_digit() || c == '.' || c == ',');
            if bare { format!("{part} views") } else { part }
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn first_metadata_part(metadata: Option<&Value>) -> Option<&Value> {
    metadata?.pointer("/metadata/contentMetadataViewModel/metadataRows/0/metadataParts/0/text")
}

/// The byline's command target, only when it is a validated channel.
fn metadata_channel(part: Option<&Value>) -> Option<ChannelId> {
    browse_channel(part?.pointer("/commandRuns/0/onTap/innertubeCommand"))
}

pub(crate) fn video_lockup(lockup: &Value) -> Parsed<VideoSummary> {
    if lockup.get("contentType").and_then(Value::as_str) != Some("LOCKUP_CONTENT_TYPE_VIDEO") {
        // Mixes, playlists and other lockups are not playable single videos here.
        return Parsed::Unsupported;
    }
    let command = lockup.pointer("/rendererContext/commandContext/onTap/innertubeCommand");
    if command.is_some_and(|c| c.get("reelWatchEndpoint").is_some()) {
        return Parsed::Filtered;
    }
    let Some(id) = lockup
        .get("contentId")
        .and_then(Value::as_str)
        .and_then(|id| VideoId::new(id).ok())
    else {
        return Parsed::Unsupported;
    };
    if let Some(target) = command.and_then(|c| c.pointer("/watchEndpoint/videoId"))
        && target.as_str() != Some(id.as_str())
    {
        return Parsed::Unsupported;
    }
    let metadata = lockup.pointer("/metadata/lockupMetadataViewModel");
    let byline = first_metadata_part(metadata);
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
    Parsed::Item(VideoSummary {
        id,
        title: title_or_default(text(metadata.and_then(|m| m.get("title")))),
        channel: channel_or_default(text(byline)),
        channel_id: metadata
            .and_then(|m| {
                m.pointer(
                    "/image/decoratedAvatarViewModel/rendererContext/commandContext/onTap/innertubeCommand",
                )
            })
            .and_then(|command| browse_channel(Some(command)))
            .or_else(|| metadata_channel(byline)),
        duration,
        thumbnail_url: thumbnail(lockup.pointer("/contentImage/thumbnailViewModel/image/sources")),
        // Row 0 is the byline; row 1 carries views and age.
        metadata: display_metadata(
            metadata
                .and_then(|m| {
                    m.pointer("/metadata/contentMetadataViewModel/metadataRows/1/metadataParts")
                })
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .take(3)
                .map(|part| text(part.get("text"))),
        ),
    })
}

/// A Short in a channel's Shorts tab (`shortsLockupViewModel` or the older
/// `reelItemRenderer`). It has no duration or byline; the caller supplies the owner.
pub(crate) fn short(value: &Value, legacy: bool) -> Parsed<VideoSummary> {
    let (id, title, sources) = if legacy {
        (
            value.get("videoId"),
            value.get("headline"),
            value.pointer("/thumbnail/thumbnails"),
        )
    } else {
        (
            value.pointer("/onTap/innertubeCommand/reelWatchEndpoint/videoId"),
            value.pointer("/overlayMetadata/primaryText"),
            value
                .pointer("/thumbnailViewModel/thumbnailViewModel/image/sources")
                .or_else(|| value.pointer("/thumbnail/thumbnailViewModel/image/sources")),
        )
    };
    let Some(id) = id
        .and_then(Value::as_str)
        .and_then(|id| VideoId::new(id).ok())
    else {
        return Parsed::Unsupported;
    };
    Parsed::Item(VideoSummary {
        metadata: None,
        id,
        title: title_or_default(text(title)),
        channel: channel_or_default(String::new()),
        channel_id: None,
        duration: None,
        thumbnail_url: thumbnail(sources),
    })
}

pub(crate) fn channel_renderer(value: &Value) -> Parsed<ChannelSummary> {
    let Some(id) = value
        .get("channelId")
        .and_then(Value::as_str)
        .and_then(|id| ChannelId::new(id).ok())
    else {
        return Parsed::Unsupported;
    };
    if let Some(target) = value.pointer("/navigationEndpoint/browseEndpoint/browseId")
        && target.as_str() != Some(id.as_str())
    {
        return Parsed::Unsupported;
    }
    let title = text(value.get("title"));
    if title.trim().is_empty() {
        return Parsed::Unsupported;
    }
    // Current search results place "577K subscribers" in videoCountText and
    // the handle in subscriberCountText; accept whichever names subscribers.
    let subscriber_count = ["videoCountText", "subscriberCountText"]
        .iter()
        .find_map(|key| count_text(&text(value.get(*key)), "subscriber"));
    Parsed::Item(ChannelSummary {
        id,
        title: title.chars().take(200).collect(),
        description: optional_text(value.get("descriptionSnippet"), 4000),
        thumbnail_url: avatar(value.pointer("/thumbnail/thumbnails")),
        subscriber_count,
    })
}

fn playlist_id(value: Option<&Value>) -> Result<PlaylistId, bool> {
    let id = value
        .and_then(Value::as_str)
        .and_then(|id| PlaylistId::new(id).ok())
        .ok_or(false)?;
    // Generated mixes ("RD…") are endless radio sequences, not browsable
    // public playlists; omitting them is deliberate, not a parse failure.
    if id.as_str().starts_with("RD") {
        return Err(true);
    }
    Ok(id)
}

pub(crate) fn playlist_lockup(lockup: &Value) -> Parsed<PlaylistSummary> {
    if lockup.get("contentType").and_then(Value::as_str) != Some("LOCKUP_CONTENT_TYPE_PLAYLIST") {
        return Parsed::Unsupported;
    }
    let id = match playlist_id(lockup.get("contentId")) {
        Ok(id) => id,
        Err(true) => return Parsed::Filtered,
        Err(false) => return Parsed::Unsupported,
    };
    let metadata = lockup.pointer("/metadata/lockupMetadataViewModel");
    let title = text(metadata.and_then(|m| m.get("title")));
    if title.trim().is_empty() {
        return Parsed::Unsupported;
    }
    let image = lockup
        .pointer("/contentImage/collectionThumbnailViewModel/primaryThumbnail/thumbnailViewModel");
    let video_count = image
        .and_then(|image| image.get("overlays"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(8)
        .flat_map(|overlay| {
            overlay
                .pointer("/thumbnailOverlayBadgeViewModel/thumbnailBadges")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .take(8)
        })
        .find_map(|badge| {
            count_text(
                badge
                    .pointer("/thumbnailBadgeViewModel/text")
                    .and_then(Value::as_str)?,
                "video",
            )
        });
    // Only a byline whose command opens a validated channel names the owner.
    let byline = first_metadata_part(metadata);
    let channel_id = metadata_channel(byline);
    Parsed::Item(PlaylistSummary {
        id,
        title,
        description: None,
        channel: channel_id.as_ref().and(optional_text(byline, 200)),
        channel_id,
        thumbnail_url: thumbnail(image.and_then(|image| image.pointer("/image/sources"))),
        video_count,
    })
}

/// Older `playlistRenderer` / `gridPlaylistRenderer` shapes.
pub(crate) fn playlist_renderer(value: &Value) -> Parsed<PlaylistSummary> {
    let id = match playlist_id(value.get("playlistId")) {
        Ok(id) => id,
        Err(true) => return Parsed::Filtered,
        Err(false) => return Parsed::Unsupported,
    };
    let title = text(value.get("title"));
    if title.trim().is_empty() {
        return Parsed::Unsupported;
    }
    let byline = ["shortBylineText", "longBylineText"]
        .iter()
        .find_map(|key| value.get(*key));
    let channel_id = byline
        .and_then(|b| b.pointer("/runs/0/navigationEndpoint"))
        .and_then(|endpoint| browse_channel(Some(endpoint)));
    let video_count = value
        .get("videoCount")
        .and_then(Value::as_str)
        .filter(|s| s.len() <= 12)
        .and_then(|s| s.replace(',', "").parse().ok())
        .or_else(|| {
            ["videoCountText", "videoCountShortText"]
                .iter()
                .find_map(|key| count_text(&text(value.get(*key)), "video"))
        });
    Parsed::Item(PlaylistSummary {
        id,
        title,
        description: None,
        channel: channel_id.as_ref().and(optional_text(byline, 200)),
        channel_id,
        thumbnail_url: thumbnail(
            value
                .pointer("/thumbnails/0/thumbnails")
                .or_else(|| value.pointer("/thumbnail/thumbnails")),
        ),
        video_count,
    })
}

#[cfg(test)]
mod metadata_tests {
    use super::display_metadata;
    #[test]
    fn metadata_joins_bounded_parts_and_labels_bare_counts() {
        let join = |parts: &[&str]| display_metadata(parts.iter().map(|p| p.to_string()));
        assert_eq!(
            join(&["733K", "4d ago"]).as_deref(),
            Some("733K views · 4d ago")
        );
        assert_eq!(
            join(&["10M views", "7 years ago"]).as_deref(),
            Some("10M views · 7 years ago")
        );
        assert_eq!(join(&["", "  "]), None);
        assert_eq!(
            join(&[&"x".repeat(41), "1h ago"]).as_deref(),
            Some("1h ago")
        );
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    // Clock-form duration coverage lives with the account feed tests.
    #[test]
    fn display_counts_parse_only_named_english_quantities() {
        for (input, unit, expected) in [
            ("577K subscribers", "subscriber", Some(577_000)),
            ("1.2M subscribers", "subscriber", Some(1_200_000)),
            ("2.45B subscribers", "subscriber", Some(2_450_000_000)),
            ("1 subscriber", "subscriber", Some(1)),
            ("1,234 subscribers", "subscriber", Some(1_234)),
            ("No subscribers", "subscriber", Some(0)),
            ("17 videos", "video", Some(17)),
            ("1,024 videos", "video", Some(1_024)),
            ("@synthetic-handle", "subscriber", None),
            ("577K views", "subscriber", None),
            ("1.5 videos", "video", None),
            ("1..2K subscribers", "subscriber", None),
            ("K subscribers", "subscriber", None),
            ("-3 videos", "video", None),
            ("99999999999999999999 videos", "video", None),
        ] {
            assert_eq!(count_text(input, unit), expected, "{input}");
        }
    }

    #[test]
    fn text_is_bounded_and_strips_controls() {
        let value = serde_json::json!({"runs": [{"text": "a\u{0007}b"}, {"text": "\nc"}]});
        assert_eq!(text(Some(&value)), "ab\nc");
        let long = serde_json::json!({"simpleText": "x".repeat(900)});
        assert_eq!(text(Some(&long)).chars().count(), 500);
    }
}
