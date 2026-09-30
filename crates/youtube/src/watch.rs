// SPDX-License-Identifier: GPL-3.0-or-later
//! Native guest watch page from the public InnerTube `next` endpoint: video
//! metadata, full description, chapter starts, the real watch-next (related)
//! list and the first comments continuation. Anonymous and read-only through
//! [`GuestTransport`]; stream resolution stays with the supervised extractor.
//!
//! Shapes were verified against live guest `next` responses on 2026-09-30 (see
//! docs/provider.md). Every value is normalized and bounded here; unknown or
//! malformed shapes yield `MalformedOutput`, never a fabricated page, so the
//! caller can keep the extractor-provided values instead.
use crate::{
    channel_avatar::safe_avatar,
    comments::CommentCursor,
    innertube::GuestTransport,
    renderers::{self, Parsed},
};
use serde_json::{Value, json};
use oxplay_core::{
    CatalogItem, ChannelId, MAX_VIDEO_CHAPTERS, OperationContext, ProviderError, VideoDetails,
    VideoId,
};
use std::{collections::HashSet, time::Duration};

/// Related rows kept from one `next` response (live pages carry ~26).
pub const MAX_RELATED: usize = 30;
const MAX_ENTRIES: usize = 128;
const MAX_DESCRIPTION_CHARS: usize = 20_000;

/// Normalized public watch-page data. Absent values stay absent; nothing here
/// is provider JSON.
#[derive(Clone)]
pub struct WatchPage {
    pub video: VideoId,
    pub title: Option<String>,
    pub channel: Option<String>,
    pub channel_id: Option<ChannelId>,
    /// Exact public avatar host only (`yt3.ggpht.com` / `yt3.googleusercontent.com`).
    pub channel_avatar_url: Option<String>,
    pub description: Option<String>,
    pub upload_date: Option<String>,
    pub view_count: Option<u64>,
    pub like_count: Option<u64>,
    pub comment_count: Option<u64>,
    pub channel_subscriber_count: Option<u64>,
    chapter_starts: Result<Vec<(Option<String>, Duration)>, ProviderError>,
    /// Watch-next videos in provider order; ads, Shorts, mixes, playlists and
    /// channels are excluded. The selected video itself never appears.
    pub related: Vec<CatalogItem>,
    /// An entry had an unrecognized shape and was skipped.
    pub related_partial: bool,
    /// First-page comments continuation (top comments), when offered.
    pub comments: Option<CommentCursor>,
}

impl WatchPage {
    /// A page carrying no values for `video`; fields are filled by the parser
    /// (or explicitly by callers composing their own synthetic fixtures).
    pub fn new(video: VideoId) -> Self {
        Self {
            video,
            title: None,
            channel: None,
            channel_id: None,
            channel_avatar_url: None,
            description: None,
            upload_date: None,
            view_count: None,
            like_count: None,
            comment_count: None,
            channel_subscriber_count: None,
            chapter_starts: Ok(Vec::new()),
            related: Vec::new(),
            related_partial: false,
            comments: None,
        }
    }
    /// True when chapter markers were present (valid or not).
    pub fn has_chapter_markers(&self) -> bool {
        !matches!(&self.chapter_starts, Ok(starts) if starts.is_empty())
    }
    /// Native values take precedence field by field; values the native page
    /// lacks (or chapters it cannot close without a duration) come from the
    /// extractor's already-resolved details.
    pub fn merge_details(
        &self,
        resolved: &VideoDetails,
        duration: Option<Duration>,
    ) -> VideoDetails {
        let native_chapters = match &self.chapter_starts {
            Ok(starts) if !starts.is_empty() => crate::chapters::from_starts(starts, duration).ok(),
            _ => None,
        };
        let (chapters, chapters_unavailable) = match native_chapters {
            Some(chapters) => (chapters, false),
            None => (resolved.chapters.clone(), resolved.chapters_unavailable),
        };
        VideoDetails {
            description: self
                .description
                .clone()
                .or_else(|| resolved.description.clone()),
            upload_date: self
                .upload_date
                .clone()
                .or_else(|| resolved.upload_date.clone()),
            view_count: self.view_count.or(resolved.view_count),
            like_count: self.like_count.or(resolved.like_count),
            comment_count: self.comment_count.or(resolved.comment_count),
            channel_subscriber_count: self
                .channel_subscriber_count
                .or(resolved.channel_subscriber_count),
            chapters,
            chapters_unavailable,
        }
    }
}

/// Errors after which the extractor's result should be used instead of the
/// native adapter's: unsupported/malformed shapes and unexpected server
/// responses. Cancellation, rate limits, offline/timeouts and genuine
/// unavailability are reported as-is rather than repeated through the helper.
pub fn fallback_eligible(error: ProviderError) -> bool {
    matches!(
        error,
        ProviderError::MalformedOutput
            | ProviderError::ExtractorFailed
            | ProviderError::OutputTooLarge
            | ProviderError::UnsupportedFormat
    )
}

/// One anonymous `next` request. Blocking; call on a worker thread.
pub fn watch_page(
    transport: &GuestTransport,
    video: &VideoId,
    operation: &OperationContext,
) -> Result<WatchPage, ProviderError> {
    let value = transport.post("next", request(video), operation)?;
    if operation.cancel.is_cancelled() {
        return Err(ProviderError::Cancelled);
    }
    parse(&value, video, operation.session_generation)
}

pub(crate) fn request(video: &VideoId) -> Value {
    json!({"videoId": video.as_str(), "racyCheckOk": true, "contentCheckOk": true})
}

/// The response must name the requested video before any field is trusted.
pub(crate) fn check_video(value: &Value, video: &VideoId) -> Result<(), ProviderError> {
    match value
        .pointer("/currentVideoEndpoint/watchEndpoint/videoId")
        .and_then(Value::as_str)
    {
        Some(id) if id == video.as_str() => Ok(()),
        _ => Err(ProviderError::MalformedOutput),
    }
}

fn contents(value: &Value) -> Result<&[Value], ProviderError> {
    let list = value
        .pointer("/contents/twoColumnWatchNextResults/results/results/contents")
        .and_then(Value::as_array)
        .ok_or(ProviderError::MalformedOutput)?;
    if list.len() > MAX_ENTRIES {
        return Err(ProviderError::OutputTooLarge);
    }
    Ok(list)
}

fn panels(value: &Value) -> impl Iterator<Item = &Value> {
    value
        .get("engagementPanels")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(32)
        .filter_map(|panel| panel.get("engagementPanelSectionListRenderer"))
}

pub(crate) fn parse(
    value: &Value,
    video: &VideoId,
    generation: u64,
) -> Result<WatchPage, ProviderError> {
    check_video(value, video)?;
    let contents = contents(value)?;
    let primary = contents
        .iter()
        .find_map(|item| item.get("videoPrimaryInfoRenderer"));
    let secondary = contents
        .iter()
        .find_map(|item| item.get("videoSecondaryInfoRenderer"));
    if primary.is_none() && secondary.is_none() {
        return Err(ProviderError::MalformedOutput);
    }
    let owner = secondary.and_then(|s| s.pointer("/owner/videoOwnerRenderer"));
    let (related, related_partial) = related(value, video)?;
    Ok(WatchPage {
        video: video.clone(),
        title: primary.and_then(|p| clean(&renderers::text(p.get("title")), 500)),
        channel: owner.and_then(|o| clean(&renderers::text(o.get("title")), 200)),
        channel_id: owner.and_then(|o| {
            o.pointer("/title/runs/0/navigationEndpoint/browseEndpoint/browseId")
                .or_else(|| o.pointer("/navigationEndpoint/browseEndpoint/browseId"))
                .and_then(Value::as_str)
                .and_then(|id| ChannelId::new(id).ok())
        }),
        channel_avatar_url: owner.and_then(|o| avatar(o.pointer("/thumbnail/thumbnails"))),
        description: description(secondary, value, video),
        upload_date: primary.and_then(|p| date(&renderers::text(p.get("dateText")))),
        view_count: primary.and_then(view_count),
        like_count: primary.and_then(like_count),
        comment_count: comment_count(value, contents),
        channel_subscriber_count: owner
            .and_then(|o| count(&renderers::text(o.get("subscriberCountText")))),
        chapter_starts: chapter_starts(value),
        related,
        related_partial,
        comments: comment_section_token(value)
            .ok()
            .map(|token| CommentCursor::native_start(video.clone(), generation, token)),
    })
}

/// Control characters other than line breaks/tabs are removed; blank is absent.
pub(crate) fn clean(value: &str, limit: usize) -> Option<String> {
    let text: String = value
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .take(limit)
        .collect();
    (!text.trim().is_empty()).then_some(text)
}

fn avatar(thumbnails: Option<&Value>) -> Option<String> {
    // Prefer the largest rendition up to 176px; the app resizes to 88px.
    let list = thumbnails?.as_array()?;
    list.iter()
        .take(16)
        .filter_map(|t| {
            Some((
                t.get("width").and_then(Value::as_u64).unwrap_or(0),
                t.get("url")?.as_str()?,
            ))
        })
        .filter(|(width, _)| *width <= 176)
        .max_by_key(|(width, _)| *width)
        .and_then(|(_, url)| safe_avatar(url))
}

/// A public count such as `2,261,131 views`, `4.72M subscribers`, `2.8K` or
/// `like this video along with 57,644 other people`. English (`hl=en`) only;
/// a decimal point without a K/M/B suffix is ambiguous and rejected.
pub(crate) fn count(text: &str) -> Option<u64> {
    let token = text
        .split_whitespace()
        .find(|token| token.starts_with(|c: char| c.is_ascii_digit()))?
        .trim_end_matches(|c: char| !c.is_ascii_alphanumeric());
    let (number, multiplier) = match token.chars().last()? {
        'K' | 'k' => (&token[..token.len() - 1], 1_000u64),
        'M' | 'm' => (&token[..token.len() - 1], 1_000_000),
        'B' | 'b' => (&token[..token.len() - 1], 1_000_000_000),
        _ => (token, 1),
    };
    let number = number.replace(',', "");
    let (whole, fraction) = number.split_once('.').unwrap_or((&number, ""));
    if whole.is_empty()
        || whole.len() > 15
        || fraction.len() > 3
        || (multiplier == 1 && !fraction.is_empty())
        || !whole
            .bytes()
            .chain(fraction.bytes())
            .all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let whole: u64 = whole.parse().ok()?;
    let scale = 10u64.pow(fraction.len() as u32);
    let fraction: u64 = if fraction.is_empty() {
        0
    } else {
        fraction.parse().ok()?
    };
    whole
        .checked_mul(multiplier)?
        .checked_add(fraction.checked_mul(multiplier)? / scale)
}

/// `Sep 29, 2026`, possibly prefixed (`Premiered …`, `Streamed live on …`).
/// Relative texts (`3 hours ago`) are absent rather than guessed.
pub(crate) fn date(text: &str) -> Option<String> {
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let tokens: Vec<&str> = text.split_whitespace().take(16).collect();
    tokens.windows(3).find_map(|window| {
        let month = window[0].to_ascii_lowercase();
        let month = MONTHS
            .iter()
            .position(|m| month.len() >= 3 && month.starts_with(m))?
            + 1;
        let day = window[1].strip_suffix(',')?;
        let year = window[2].trim_end_matches(|c: char| !c.is_ascii_digit());
        if !(1..=2).contains(&day.len())
            || year.len() != 4
            || !day.bytes().chain(year.bytes()).all(|b| b.is_ascii_digit())
        {
            return None;
        }
        let day: u32 = day.parse().ok()?;
        (1..=31)
            .contains(&day)
            .then(|| format!("{year}-{month:02}-{day:02}"))
    })
}

fn view_count(primary: &Value) -> Option<u64> {
    let renderer = primary.pointer("/viewCount/videoViewCountRenderer")?;
    if renderer.get("isLive").and_then(Value::as_bool) == Some(true) {
        return None; // A concurrent viewer count is not a total.
    }
    // The displayed total is authoritative: live pages send `originalViewCount`
    // as "0" for older videos, so it is only a nonzero last resort.
    count(&renderers::text(renderer.get("viewCount"))).or_else(|| {
        renderer
            .get("originalViewCount")
            .and_then(Value::as_str)
            .filter(|exact| exact.len() <= 20)
            .and_then(|exact| exact.parse::<u64>().ok())
            .filter(|count| *count > 0)
    })
}

fn like_count(primary: &Value) -> Option<u64> {
    primary
        .pointer("/videoActions/menuRenderer/topLevelButtons")?
        .as_array()?
        .iter()
        .take(16)
        .find_map(|button| {
            let model = button.pointer(
                "/segmentedLikeDislikeButtonViewModel/likeButtonViewModel/likeButtonViewModel/toggleButtonViewModel/toggleButtonViewModel/defaultButtonViewModel/buttonViewModel",
            )?;
            model
                .get("accessibilityText")
                .and_then(Value::as_str)
                .and_then(count)
                .or_else(|| model.get("title").and_then(Value::as_str).and_then(count))
        })
}

fn comment_count(value: &Value, contents: &[Value]) -> Option<u64> {
    contents
        .iter()
        .filter_map(|item| item.pointer("/itemSectionRenderer/contents")?.as_array())
        .flatten()
        .take(MAX_ENTRIES)
        .find_map(|item| {
            count(&renderers::text(
                item.pointer("/commentsEntryPointHeaderRenderer/commentCount"),
            ))
        })
        .or_else(|| {
            panels(value)
                .find(|panel| {
                    panel.get("panelIdentifier").and_then(Value::as_str)
                        == Some("engagement-panel-comments-section")
                })
                .and_then(|panel| {
                    count(&renderers::text(panel.pointer(
                        "/header/engagementPanelTitleHeaderRenderer/contextualInfo",
                    )))
                })
        })
}

/// The attributed description with YouTube's redirect links restored to their
/// real targets (the display text of long links is truncated with `...`).
/// Legacy `description.runs` and the structured-description panel are
/// fallbacks. Rendering remains plain text.
fn description(secondary: Option<&Value>, value: &Value, video: &VideoId) -> Option<String> {
    let attributed = secondary
        .and_then(|s| s.get("attributedDescription"))
        .or_else(|| {
            panels(value)
                .filter_map(|panel| {
                    panel
                        .pointer("/content/structuredDescriptionContentRenderer/items")?
                        .as_array()
                })
                .flatten()
                .take(16)
                .find_map(|item| {
                    item.pointer(
                        "/expandableVideoDescriptionBodyRenderer/attributedDescriptionBodyText",
                    )
                })
        });
    if let Some(attributed) = attributed {
        return attributed_text(attributed, video)
            .and_then(|text| clean(&text, MAX_DESCRIPTION_CHARS));
    }
    let runs = secondary?.pointer("/description/runs")?.as_array()?;
    let text: String = runs
        .iter()
        .take(4096)
        .filter_map(|run| run.get("text")?.as_str())
        .collect();
    clean(&text, MAX_DESCRIPTION_CHARS)
}

/// The URL a description link stands for, and whether it applies only to a
/// rendered link chip (display text padded with no-break spaces). Plain
/// `@mentions`, `#hashtags` and same-video timestamps keep their typed text.
fn link_target(command: &Value, video: &VideoId) -> Option<(String, bool)> {
    if let Some(url) = command.pointer("/urlEndpoint/url") {
        return Some((redirect_target(url.as_str()?)?, false));
    }
    if let Some(path) = command.pointer("/browseEndpoint/canonicalBaseUrl") {
        let path = path.as_str()?;
        let safe = path.len() <= 256
            && path.starts_with('/')
            && !path.starts_with("//")
            && path
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/@_-.%".contains(&b));
        return safe.then(|| (format!("https://www.youtube.com{path}"), true));
    }
    let id = VideoId::new(command.pointer("/watchEndpoint/videoId")?.as_str()?).ok()?;
    (id != *video).then(|| (id.watch_url(), true))
}

fn attributed_text(value: &Value, video: &VideoId) -> Option<String> {
    let content = value.get("content")?.as_str()?;
    let units: Vec<u16> = content.encode_utf16().collect();
    let mut runs: Vec<(usize, usize, String, bool)> = value
        .get("commandRuns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(512)
        .filter_map(|run| {
            let start = usize::try_from(run.get("startIndex")?.as_u64()?).ok()?;
            let length = usize::try_from(run.get("length")?.as_u64()?).ok()?;
            let (target, chip_only) = link_target(run.pointer("/onTap/innertubeCommand")?, video)?;
            Some((start, start.checked_add(length)?, target, chip_only))
        })
        .collect();
    runs.sort_by_key(|run| run.0);
    let boundary = |index: usize| {
        index == 0
            || index == units.len()
            || (index < units.len() && !(0xDC00..=0xDFFF).contains(&units[index]))
    };
    let mut text = String::with_capacity(content.len());
    let mut cursor = 0;
    for (start, end, target, chip_only) in runs {
        if start < cursor || end > units.len() || start >= end || !boundary(start) || !boundary(end)
        {
            continue; // Overlapping or out-of-range runs keep their display text.
        }
        let display = String::from_utf16_lossy(&units[start..end]);
        if chip_only && !display.contains('\u{a0}') {
            continue;
        }
        text.push_str(&String::from_utf16_lossy(&units[cursor..start]));
        // Untruncated URL text already names the target (the redirect adds a
        // normalizing slash); truncated links and link chips are restored.
        if display.trim_end_matches('/') == target.trim_end_matches('/') {
            text.push_str(&display);
        } else {
            text.push_str(&target);
        }
        cursor = end;
    }
    text.push_str(&String::from_utf16_lossy(&units[cursor..]));
    Some(text)
}

/// Only YouTube's own `/redirect?q=` wrapper is unwrapped, to an http(s) URL.
fn redirect_target(url: &str) -> Option<String> {
    let url = url::Url::parse(url).ok()?;
    if url.scheme() != "https"
        || url.host_str() != Some("www.youtube.com")
        || url.path() != "/redirect"
    {
        return None;
    }
    let target = url.query_pairs().find(|(key, _)| key == "q")?.1;
    let parsed = url::Url::parse(&target).ok()?;
    (matches!(parsed.scheme(), "http" | "https")
        && target.len() <= 2048
        && !target.chars().any(|c| c.is_whitespace() || c.is_control()))
    .then(|| target.into_owned())
}

/// Start offsets from the decorated player bar (current `multiMarkersPlayerBarRenderer`
/// or legacy `chapteredPlayerBarRenderer`), else the chapters engagement panel.
fn chapter_starts(value: &Value) -> Result<Vec<(Option<String>, Duration)>, ProviderError> {
    let bar = value.pointer(
        "/playerOverlays/playerOverlayRenderer/decoratedPlayerBarRenderer/decoratedPlayerBarRenderer/playerBar",
    );
    let from_bar = bar.and_then(|bar| {
        bar.pointer("/multiMarkersPlayerBarRenderer/markersMap")
            .and_then(Value::as_array)
            .and_then(|map| {
                let chapters = |key: &str| {
                    map.iter().take(16).find_map(|entry| {
                        (entry.get("key")?.as_str()? == key)
                            .then(|| entry.pointer("/value/chapters"))
                            .flatten()
                    })
                };
                chapters("DESCRIPTION_CHAPTERS").or_else(|| chapters("AUTO_CHAPTERS"))
            })
            .or_else(|| bar.pointer("/chapteredPlayerBarRenderer/chapters"))
    });
    if let Some(list) = from_bar {
        let list = list.as_array().ok_or(ProviderError::MalformedOutput)?;
        return starts(list, |chapter| {
            let chapter = chapter.get("chapterRenderer")?;
            let millis = chapter.get("timeRangeStartMillis")?.as_u64()?;
            Some((
                renderers::text(chapter.get("title")),
                Duration::from_millis(millis),
            ))
        });
    }
    let panel = panels(value).find_map(|panel| {
        panel
            .pointer("/content/macroMarkersListRenderer/contents")?
            .as_array()
    });
    match panel {
        Some(list) => starts(list, |item| {
            let item = item.get("macroMarkersListItemRenderer")?;
            Some((
                renderers::text(item.get("title")),
                // `M:SS` / `H:MM:SS` only; other forms reject the set.
                renderers::duration_text(&renderers::text(item.get("timeDescription")))?,
            ))
        }),
        None => Ok(Vec::new()),
    }
}

fn starts(
    list: &[Value],
    read: impl Fn(&Value) -> Option<(String, Duration)>,
) -> Result<Vec<(Option<String>, Duration)>, ProviderError> {
    if list.len() > MAX_VIDEO_CHAPTERS {
        return Err(ProviderError::OutputTooLarge);
    }
    let mut result = Vec::with_capacity(list.len());
    let mut previous: Option<Duration> = None;
    for entry in list {
        let (title, start) = read(entry).ok_or(ProviderError::MalformedOutput)?;
        if previous.is_some_and(|previous| start <= previous) {
            return Err(ProviderError::MalformedOutput);
        }
        previous = Some(start);
        result.push(((!title.trim().is_empty()).then_some(title), start));
    }
    Ok(result)
}

/// Watch-next secondary results. Known promotions/ads, Shorts shelves, mixes,
/// playlists, channels and continuation rows never become items.
fn related(value: &Value, video: &VideoId) -> Result<(Vec<CatalogItem>, bool), ProviderError> {
    let Some(list) = value
        .pointer("/contents/twoColumnWatchNextResults/secondaryResults/secondaryResults/results")
    else {
        return Ok((Vec::new(), false));
    };
    let list = list.as_array().ok_or(ProviderError::MalformedOutput)?;
    // Older layouts wrap the list in one item section; flatten exactly one level.
    let entries: Vec<&Value> = list
        .iter()
        .take(MAX_ENTRIES)
        .flat_map(
            |entry| match entry.pointer("/itemSectionRenderer/contents") {
                Some(Value::Array(inner)) => inner.iter().take(MAX_ENTRIES).collect(),
                _ => vec![entry],
            },
        )
        .take(MAX_ENTRIES)
        .collect();
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    let mut partial = false;
    for entry in entries {
        if crate::is_promoted(entry) || has_ad_badge(entry) {
            continue;
        }
        let parsed = if let Some(lockup) = entry.get("lockupViewModel") {
            match lockup.get("contentType").and_then(Value::as_str) {
                Some("LOCKUP_CONTENT_TYPE_VIDEO") => renderers::video_lockup(lockup),
                // Mixes/radio and playlists are not offered by this list yet.
                Some(_) => Parsed::Filtered,
                None => Parsed::Unsupported,
            }
        } else if let Some(compact) = entry.get("compactVideoRenderer") {
            renderers::video_renderer(compact)
        } else if [
            "continuationItemRenderer",
            "reelShelfRenderer",
            "compactRadioRenderer",
            "compactPlaylistRenderer",
            "compactMovieRenderer",
            "compactChannelRenderer",
            "relatedChipCloudRenderer",
            "shortsLockupViewModel",
        ]
        .iter()
        .any(|key| entry.get(*key).is_some())
        {
            Parsed::Filtered
        } else {
            Parsed::Unsupported
        };
        match parsed {
            Parsed::Item(summary) => {
                if summary.id != *video && seen.insert(summary.id.clone()) {
                    items.push(CatalogItem::Video(summary));
                }
            }
            Parsed::Filtered => {}
            Parsed::Unsupported => partial = true,
        }
        if items.len() >= MAX_RELATED {
            break;
        }
    }
    Ok((items, partial))
}

fn has_ad_badge(entry: &Value) -> bool {
    entry
        .pointer("/compactVideoRenderer/badges")
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

/// Opaque continuation token: printable ASCII, bounded.
pub(crate) fn token(value: &Value) -> Option<&str> {
    value
        .as_str()
        .filter(|t| !t.is_empty() && t.len() <= 16_384 && t.bytes().all(|b| b.is_ascii_graphic()))
}

/// The first comments continuation of a watch page: the `comment-item-section`
/// item section (as used by yt-dlp), else the comments engagement panel. A
/// section carrying only a message (comments turned off) is `Unavailable`.
pub(crate) fn comment_section_token(value: &Value) -> Result<String, ProviderError> {
    let sections = contents(value)?
        .iter()
        .filter_map(|item| item.get("itemSectionRenderer"))
        .chain(panels(value).filter_map(|panel| {
            (panel.get("panelIdentifier").and_then(Value::as_str)
                == Some("engagement-panel-comments-section"))
            .then(|| panel.pointer("/content/sectionListRenderer/contents/0/itemSectionRenderer"))
            .flatten()
        }))
        .filter(|section| {
            section.get("sectionIdentifier").and_then(Value::as_str) == Some("comment-item-section")
        });
    let mut disabled = false;
    for section in sections {
        let items = section
            .get("contents")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(16);
        for item in items {
            if let Some(next) = item.get("continuationItemRenderer") {
                let raw = next
                    .pointer("/continuationEndpoint/continuationCommand/token")
                    .ok_or(ProviderError::MalformedOutput)?;
                return token(raw)
                    .map(str::to_owned)
                    .ok_or(ProviderError::MalformedOutput);
            }
            if item.get("messageRenderer").is_some() {
                disabled = true;
            }
        }
    }
    Err(if disabled {
        ProviderError::Unavailable
    } else {
        ProviderError::MalformedOutput
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIDEO: &str = "abcdefghijk";
    const CHANNEL: &str = "UCabcdefghijklmnopqrstuv";

    /// TEST FIXTURE: synthetic, minimal `next` response in the shape observed
    /// live on 2026-09-30. No real titles, authors or tokens.
    fn fixture() -> Value {
        json!({
            "currentVideoEndpoint": {"watchEndpoint": {"videoId": VIDEO}},
            "contents": {"twoColumnWatchNextResults": {
                "results": {"results": {"contents": [
                    {"videoPrimaryInfoRenderer": {
                        "title": {"runs": [{"text": "Synthetic "}, {"text": "title"}]},
                        "viewCount": {"videoViewCountRenderer": {
                            "viewCount": {"simpleText": "2,261,131 views"},
                            // Observed live: "0" on older videos; never trusted over the text.
                            "originalViewCount": "0"}},
                        "dateText": {"simpleText": "Sep 29, 2026"},
                        "videoActions": {"menuRenderer": {"topLevelButtons": [
                            {"segmentedLikeDislikeButtonViewModel": {"likeButtonViewModel": {"likeButtonViewModel": {
                                "toggleButtonViewModel": {"toggleButtonViewModel": {"defaultButtonViewModel": {
                                    "buttonViewModel": {"title": "57K",
                                        "accessibilityText": "like this video along with 57,644 other people"}}}}}}}}
                        ]}}
                    }},
                    {"videoSecondaryInfoRenderer": {
                        "owner": {"videoOwnerRenderer": {
                            "title": {"runs": [{"text": "Synthetic channel",
                                "navigationEndpoint": {"browseEndpoint": {"browseId": CHANNEL}}}]},
                            "thumbnail": {"thumbnails": [
                                {"url": "https://yt3.ggpht.com/synthetic=s48", "width": 48},
                                {"url": "https://yt3.ggpht.com/synthetic=s88", "width": 88},
                                {"url": "https://yt3.ggpht.com/synthetic=s176", "width": 176},
                                {"url": "https://yt3.ggpht.com/synthetic=s900", "width": 900}]},
                            "subscriberCountText": {"simpleText": "4.72M subscribers"}}},
                        "attributedDescription": {
                            "content": "Merch 😀 https://example.test/lo... and 0:00 intro",
                            "commandRuns": [
                                {"startIndex": 9, "length": 26, "onTap": {"innertubeCommand": {"urlEndpoint": {
                                    "url": "https://www.youtube.com/redirect?event=video_description&redir_token=SYNTHETIC&q=https%3A%2F%2Fexample.test%2Flong%2Fpath"}}}},
                                {"startIndex": 40, "length": 4, "onTap": {"innertubeCommand": {"watchEndpoint": {
                                    "videoId": VIDEO, "startTimeSeconds": 0}}}}
                            ]}
                    }},
                    {"itemSectionRenderer": {"sectionIdentifier": "comment-item-section", "contents": [
                        {"continuationItemRenderer": {"continuationEndpoint": {"continuationCommand": {
                            "token": "SYNTHETIC_SECTION_TOKEN"}}}}]}}
                ]}},
                "secondaryResults": {"secondaryResults": {"results": [
                    {"lockupViewModel": lockup("lockupvideo")},
                    {"adSlotRenderer": {"fulfillmentContent": {}}},
                    {"compactVideoRenderer": {
                        "videoId": "compactvide", "title": {"simpleText": "Compact synthetic"},
                        "shortBylineText": {"runs": [{"text": "Compact channel",
                            "navigationEndpoint": {"browseEndpoint": {"browseId": CHANNEL}}}]},
                        "lengthText": {"simpleText": "1:02:03"},
                        "thumbnail": {"thumbnails": [{"url": "https://i.ytimg.com/vi/compactvide/hqdefault.jpg", "width": 336}]}}},
                    {"compactVideoRenderer": {"videoId": "advertvideo", "badges": [
                        {"metadataBadgeRenderer": {"style": "BADGE_STYLE_TYPE_AD"}}]}},
                    {"reelShelfRenderer": {"items": [{"shortsLockupViewModel": {}}]}},
                    {"lockupViewModel": {"contentId": "RDsynthetic", "contentType": "LOCKUP_CONTENT_TYPE_PLAYLIST"}},
                    {"lockupViewModel": lockup(VIDEO)},
                    {"lockupViewModel": lockup("lockupvideo")},
                    {"futureRenderer": {}},
                    {"continuationItemRenderer": {"continuationEndpoint": {"continuationCommand": {"token": "RELATED"}}}}
                ]}}
            }},
            "playerOverlays": {"playerOverlayRenderer": {"decoratedPlayerBarRenderer": {"decoratedPlayerBarRenderer": {
                "playerBar": {"multiMarkersPlayerBarRenderer": {"markersMap": [
                    {"key": "DESCRIPTION_CHAPTERS", "value": {"chapters": [
                        {"chapterRenderer": {"title": {"simpleText": "Intro"}, "timeRangeStartMillis": 0}},
                        {"chapterRenderer": {"title": {"simpleText": "Middle"}, "timeRangeStartMillis": 105000}}
                    ]}}]}}}}}},
            "engagementPanels": [
                {"engagementPanelSectionListRenderer": {
                    "panelIdentifier": "engagement-panel-comments-section",
                    "header": {"engagementPanelTitleHeaderRenderer": {"contextualInfo": {"runs": [{"text": "2.8K"}]}}}}}
            ]
        })
    }
    fn lockup(id: &str) -> Value {
        json!({
            "contentId": id, "contentType": "LOCKUP_CONTENT_TYPE_VIDEO",
            "rendererContext": {"commandContext": {"onTap": {"innertubeCommand": {"watchEndpoint": {"videoId": id}}}}},
            "contentImage": {"thumbnailViewModel": {
                "image": {"sources": [{"url": format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg"), "width": 336}]},
                "overlays": [{"thumbnailBottomOverlayViewModel": {"badges": [{"thumbnailBadgeViewModel": {"text": "28:02"}}]}}]}},
            "metadata": {"lockupMetadataViewModel": {
                "title": {"content": "Lockup synthetic"},
                "image": {"decoratedAvatarViewModel": {"rendererContext": {"commandContext": {"onTap": {
                    "innertubeCommand": {"browseEndpoint": {"browseId": CHANNEL}}}}}}},
                "metadata": {"contentMetadataViewModel": {"metadataRows": [
                    {"metadataParts": [{"text": {"content": "Lockup channel"}}]}]}}}}
        })
    }
    fn page() -> WatchPage {
        parse(&fixture(), &VideoId::new(VIDEO).unwrap(), 0).unwrap()
    }

    #[test]
    fn primary_and_secondary_info_are_normalized() {
        let page = page();
        assert_eq!(page.title.as_deref(), Some("Synthetic title"));
        assert_eq!(page.view_count, Some(2_261_131));
        assert_eq!(page.upload_date.as_deref(), Some("2026-09-29"));
        assert_eq!(page.like_count, Some(57_644));
        assert_eq!(page.channel.as_deref(), Some("Synthetic channel"));
        assert_eq!(
            page.channel_id.as_ref().map(ChannelId::as_str),
            Some(CHANNEL)
        );
        assert_eq!(
            page.channel_avatar_url.as_deref(),
            Some("https://yt3.ggpht.com/synthetic=s176")
        );
        assert_eq!(page.channel_subscriber_count, Some(4_720_000));
        assert_eq!(page.comment_count, Some(2_800));
    }

    #[test]
    fn description_runs_restore_redirect_targets_in_utf16_offsets() {
        // The emoji is two UTF-16 units: offsets must not be byte/char indices.
        assert_eq!(
            page().description.as_deref(),
            Some("Merch 😀 https://example.test/long/path and 0:00 intro")
        );
        let foreign = json!({"content": "see link", "commandRuns": [{"startIndex": 4, "length": 4,
            "onTap": {"innertubeCommand": {"urlEndpoint": {"url": "https://evil.test/redirect?q=https%3A%2F%2Fx.test"}}}}]});
        assert_eq!(
            attributed_text(&foreign, &VideoId::new(VIDEO).unwrap()).as_deref(),
            Some("see link")
        );
        let script = json!({"content": "see link", "commandRuns": [{"startIndex": 4, "length": 4,
            "onTap": {"innertubeCommand": {"urlEndpoint": {"url": "https://www.youtube.com/redirect?q=javascript%3Aalert(1)"}}}}]});
        assert_eq!(
            attributed_text(&script, &VideoId::new(VIDEO).unwrap()).as_deref(),
            Some("see link")
        );
        let hostile = json!({"content": "ab", "commandRuns": [{"startIndex": 1, "length": 99,
            "onTap": {"innertubeCommand": {"urlEndpoint": {"url": "https://www.youtube.com/redirect?q=https%3A%2F%2Fx.test"}}}}]});
        assert_eq!(
            attributed_text(&hostile, &VideoId::new(VIDEO).unwrap()).as_deref(),
            Some("ab")
        );
        // Link chips (no-break-space padded) become their URLs; typed
        // #hashtags, @mentions, timestamps and complete URLs stay as typed.
        let nb = '\u{a0}';
        let content = format!("{nb}/{nb}@chan{nb} {nb}•{nb}Other{nb} #tag 1:05 https://x.test");
        let runs = [
            (
                0,
                9,
                json!({"browseEndpoint": {"canonicalBaseUrl": format!("/channel/{CHANNEL}")}}),
            ),
            (10, 9, json!({"watchEndpoint": {"videoId": "otherother1"}})),
            (
                20,
                4,
                json!({"browseEndpoint": {"canonicalBaseUrl": "/hashtag/tag"}}),
            ),
            (25, 4, json!({"watchEndpoint": {"videoId": VIDEO}})),
            (
                30,
                14,
                json!({"urlEndpoint": {"url": "https://www.youtube.com/redirect?q=https%3A%2F%2Fx.test%2F"}}),
            ),
        ];
        let chips = json!({"content": content, "commandRuns": runs.iter().map(|(start, length, command)| json!({
            "startIndex": start, "length": length, "onTap": {"innertubeCommand": command}})).collect::<Vec<_>>()});
        assert_eq!(
            attributed_text(&chips, &VideoId::new(VIDEO).unwrap()).as_deref(),
            Some(format!(
                "https://www.youtube.com/channel/{CHANNEL} https://www.youtube.com/watch?v=otherother1 #tag 1:05 https://x.test"
            ).as_str())
        );
        // Legacy runs and bounds.
        let mut value = fixture();
        let secondary = &mut value["contents"]["twoColumnWatchNextResults"]["results"]["results"]["contents"]
            [1]["videoSecondaryInfoRenderer"];
        secondary
            .as_object_mut()
            .unwrap()
            .remove("attributedDescription");
        secondary["description"] =
            json!({"runs": [{"text": "a\u{0000}b\n"}, {"text": "x".repeat(30_000)}]});
        let description = parse(&value, &VideoId::new(VIDEO).unwrap(), 0)
            .unwrap()
            .description
            .unwrap();
        assert!(description.starts_with("ab\n"));
        assert_eq!(description.chars().count(), MAX_DESCRIPTION_CHARS);
    }

    #[test]
    fn chapters_close_with_the_resolved_duration_and_fall_back_otherwise() {
        let page = page();
        assert!(page.has_chapter_markers());
        let merged = page.merge_details(&VideoDetails::default(), Some(Duration::from_secs(300)));
        assert_eq!(merged.chapters.len(), 2);
        assert_eq!(merged.chapters[0].end, Duration::from_secs(105));
        assert_eq!(merged.chapters[1].title.as_deref(), Some("Middle"));
        assert_eq!(merged.chapters[1].end, Duration::from_secs(300));
        // Without a duration (or with an inconsistent one) the extractor's set wins.
        let resolved = VideoDetails {
            chapters_unavailable: true,
            ..Default::default()
        };
        let merged = page.merge_details(&resolved, None);
        assert!(merged.chapters.is_empty() && merged.chapters_unavailable);
        let merged = page.merge_details(&resolved, Some(Duration::from_secs(60)));
        assert!(merged.chapters.is_empty() && merged.chapters_unavailable);
        // Engagement-panel markers are read when the player bar has none.
        let mut value = fixture();
        value.as_object_mut().unwrap().remove("playerOverlays");
        value["engagementPanels"].as_array_mut().unwrap().push(json!({
            "engagementPanelSectionListRenderer": {"content": {"macroMarkersListRenderer": {"contents": [
                {"macroMarkersListItemRenderer": {"title": {"simpleText": "A"}, "timeDescription": {"simpleText": "0:00"}}},
                {"macroMarkersListItemRenderer": {"title": {"simpleText": "B"}, "timeDescription": {"simpleText": "1:02:03"}}}
            ]}}}}));
        let panel = parse(&value, &VideoId::new(VIDEO).unwrap(), 0).unwrap();
        let merged = panel.merge_details(&VideoDetails::default(), Some(Duration::from_secs(4000)));
        assert_eq!(merged.chapters[1].start, Duration::from_secs(3723));
        // Out-of-order markers invalidate the native set as a whole.
        let mut value = fixture();
        value["playerOverlays"]["playerOverlayRenderer"]["decoratedPlayerBarRenderer"]["decoratedPlayerBarRenderer"]
            ["playerBar"]["multiMarkersPlayerBarRenderer"]["markersMap"][0]["value"]["chapters"]
            [1]["chapterRenderer"]["timeRangeStartMillis"] = json!(0);
        let broken = parse(&value, &VideoId::new(VIDEO).unwrap(), 0).unwrap();
        assert!(broken.has_chapter_markers());
        assert!(
            broken
                .merge_details(&VideoDetails::default(), Some(Duration::from_secs(300)))
                .chapters
                .is_empty()
        );
    }

    #[test]
    fn related_keeps_videos_and_filters_ads_shorts_mixes_self_and_duplicates() {
        let page = page();
        let ids: Vec<_> = page
            .related
            .iter()
            .map(|item| match item {
                CatalogItem::Video(video) => video.id.as_str().to_owned(),
                _ => panic!("only videos are related"),
            })
            .collect();
        assert_eq!(ids, ["lockupvideo", "compactvide"]);
        assert!(
            page.related_partial,
            "unknown renderer is reported as partial"
        );
        let CatalogItem::Video(first) = &page.related[0] else {
            unreachable!()
        };
        assert_eq!(first.title, "Lockup synthetic");
        assert_eq!(first.channel, "Lockup channel");
        assert_eq!(first.duration, Some(Duration::from_secs(28 * 60 + 2)));
        assert_eq!(
            first.thumbnail_url.as_deref(),
            Some("https://i.ytimg.com/vi/lockupvideo/hqdefault.jpg")
        );
        let CatalogItem::Video(compact) = &page.related[1] else {
            unreachable!()
        };
        assert_eq!(
            compact.channel_id.as_ref().map(ChannelId::as_str),
            Some(CHANNEL)
        );
        assert_eq!(compact.duration, Some(Duration::from_secs(3723)));
        // Unsafe artwork is dropped, not the row; a bounded list stays bounded.
        let mut value = fixture();
        let results =
            value["contents"]["twoColumnWatchNextResults"]["secondaryResults"]["secondaryResults"]
                ["results"]
                .as_array_mut()
                .unwrap();
        results.clear();
        for n in 0..50 {
            let mut item = lockup(&format!("video{n:06}"));
            item["contentImage"]["thumbnailViewModel"]["image"]["sources"][0]["url"] =
                json!("https://evil.test/a.jpg");
            results.push(json!({"lockupViewModel": item}));
        }
        let page = parse(&value, &VideoId::new(VIDEO).unwrap(), 0).unwrap();
        assert_eq!(page.related.len(), MAX_RELATED);
        assert!(matches!(&page.related[0], CatalogItem::Video(v) if v.thumbnail_url.is_none()));
    }

    #[test]
    fn comment_token_is_extracted_and_disabled_sections_are_unavailable() {
        assert!(page().comments.is_some());
        let mut value = fixture();
        let section = &mut value["contents"]["twoColumnWatchNextResults"]["results"]["results"]["contents"]
            [2]["itemSectionRenderer"]["contents"];
        *section =
            json!([{"messageRenderer": {"text": {"simpleText": "Comments are turned off."}}}]);
        assert_eq!(
            comment_section_token(&value),
            Err(ProviderError::Unavailable)
        );
        assert!(
            parse(&value, &VideoId::new(VIDEO).unwrap(), 0)
                .unwrap()
                .comments
                .is_none()
        );
        value["contents"]["twoColumnWatchNextResults"]["results"]["results"]["contents"][2] = json!({"itemSectionRenderer": {"sectionIdentifier": "comment-item-section", "contents": [
                {"continuationItemRenderer": {"continuationEndpoint": {"continuationCommand": {"token": "bad token\n"}}}}]}});
        assert_eq!(
            comment_section_token(&value),
            Err(ProviderError::MalformedOutput)
        );
    }

    #[test]
    fn foreign_or_unrecognized_pages_are_malformed_never_empty_success() {
        let id = VideoId::new(VIDEO).unwrap();
        let mut foreign = fixture();
        foreign["currentVideoEndpoint"]["watchEndpoint"]["videoId"] = json!("zyxwvutsrqp");
        for value in [
            foreign,
            json!({}),
            json!({"currentVideoEndpoint": {"watchEndpoint": {"videoId": VIDEO}}}),
            json!({"currentVideoEndpoint": {"watchEndpoint": {"videoId": VIDEO}},
                "contents": {"twoColumnWatchNextResults": {"results": {"results": {"contents": [{"futureRenderer": {}}]}}}}}),
        ] {
            assert_eq!(
                parse(&value, &id, 0).err(),
                Some(ProviderError::MalformedOutput)
            );
        }
        // Optional fields may be absent without breaking the page.
        let mut sparse = fixture();
        sparse["contents"]["twoColumnWatchNextResults"]["results"]["results"]["contents"][0] = json!({"videoPrimaryInfoRenderer": {"viewCount": {"videoViewCountRenderer": {
                "isLive": true, "viewCount": {"runs": [{"text": "1,234"}, {"text": " watching now"}]}}},
                "dateText": {"simpleText": "Started streaming 3 hours ago"}}});
        let page = parse(&sparse, &id, 0).unwrap();
        assert_eq!(page.title, None);
        assert_eq!(page.view_count, None);
        assert_eq!(page.upload_date, None);
        assert_eq!(page.like_count, None);
    }

    #[test]
    fn counts_and_dates_accept_only_reviewed_english_forms() {
        for (text, expected) in [
            ("2,261,131 views", Some(2_261_131)),
            ("4.72M subscribers", Some(4_720_000)),
            ("2.8K", Some(2_800)),
            ("5K likes", Some(5_000)),
            ("1.2B", Some(1_200_000_000)),
            (
                "like this video along with 57,644 other people",
                Some(57_644),
            ),
            ("No views", None),
            ("1.5 views", None),
            ("", None),
            ("99999999999999999999K", None),
        ] {
            assert_eq!(count(text), expected, "{text}");
        }
        assert_eq!(
            date("Premiered Nov 10, 2014").as_deref(),
            Some("2014-11-10")
        );
        assert_eq!(
            date("Streamed live on Jan 2, 2020").as_deref(),
            Some("2020-01-02")
        );
        assert_eq!(date("11 hours ago"), None);
        assert_eq!(date("Sep 32, 2026"), None);
    }

    #[test]
    fn merge_prefers_native_values_and_keeps_extractor_gaps() {
        let mut value = fixture();
        value["contents"]["twoColumnWatchNextResults"]["results"]["results"]["contents"][0]
            ["videoPrimaryInfoRenderer"]
            .as_object_mut()
            .unwrap()
            .remove("dateText");
        let page = parse(&value, &VideoId::new(VIDEO).unwrap(), 0).unwrap();
        let resolved = VideoDetails {
            description: Some("extractor description".into()),
            upload_date: Some("2026-09-28".into()),
            view_count: Some(1),
            ..Default::default()
        };
        let merged = page.merge_details(&resolved, Some(Duration::from_secs(300)));
        assert!(merged.description.unwrap().starts_with("Merch"));
        assert_eq!(merged.upload_date.as_deref(), Some("2026-09-28"));
        assert_eq!(merged.view_count, Some(2_261_131));
    }

    #[test]
    fn fallback_classes_exclude_cancellation_limits_and_real_unavailability() {
        for error in [
            ProviderError::MalformedOutput,
            ProviderError::ExtractorFailed,
            ProviderError::OutputTooLarge,
        ] {
            assert!(fallback_eligible(error));
        }
        for error in [
            ProviderError::Cancelled,
            ProviderError::RateLimited,
            ProviderError::Offline,
            ProviderError::Timeout,
            ProviderError::Unavailable,
        ] {
            assert!(!fallback_eligible(error));
        }
    }

    #[test]
    fn transport_errors_propagate_from_the_fixture() {
        let transport = GuestTransport::with_fixture(vec![
            Ok(json!({"error": {"status": "INTERNAL"}})),
            Ok(fixture()),
        ]);
        let id = VideoId::new(VIDEO).unwrap();
        let operation = OperationContext {
            request_id: 1,
            session_generation: 0,
            cancel: Default::default(),
        };
        let error = watch_page(&transport, &id, &operation).err().unwrap();
        assert!(fallback_eligible(error));
        let page = watch_page(&transport, &id, &operation).unwrap();
        assert_eq!(page.related.len(), 2);
        let calls = transport.fixture.as_ref().unwrap().calls.lock().unwrap();
        assert_eq!(calls[1].0, "next");
        assert_eq!(calls[1].1["videoId"], VIDEO);
    }
}
