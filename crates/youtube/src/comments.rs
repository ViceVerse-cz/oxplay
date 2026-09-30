// SPDX-License-Identifier: GPL-3.0-or-later
//! Bounded guest comments. The native path reads the public InnerTube `next`
//! comments continuation directly (real remote continuation tokens); the
//! reviewed yt-dlp extraction remains the fallback, whose offset pages replay
//! the prefix rather than continue remotely.
use crate::{YtDlp, innertube::GuestTransport, watch};
use serde_json::{Value, json};
use oxplay_core::{
    ChannelId, CommentSummary, OperationContext, ProviderError, VideoDetails, VideoId,
};
use std::{collections::HashMap, sync::Arc};

const PAGE_SIZE: usize = 20;
const MAX_COMMENTS: usize = 200;
#[derive(Clone)]
pub struct CommentCursor {
    video: VideoId,
    generation: u64,
    offset: usize,
    // Prefix replay must preserve every previously published identity, not just
    // the page boundary. Arc keeps UI Back-stack cursor clones inexpensive. For
    // native cursors these are the published IDs used to drop repeats.
    prefix_ids: Arc<[String]>,
    /// Native InnerTube continuation; `None` identifies an extractor replay cursor.
    token: Option<Arc<str>>,
}
impl CommentCursor {
    /// First native page from a watch page's comments-section continuation.
    pub(crate) fn native_start(video: VideoId, generation: u64, token: String) -> Self {
        Self {
            video,
            generation,
            offset: 0,
            prefix_ids: Arc::from([]),
            token: Some(token.into()),
        }
    }
    pub fn video(&self) -> &VideoId {
        &self.video
    }
    /// True for a native continuation (as opposed to extractor prefix replay).
    pub fn is_native(&self) -> bool {
        self.token.is_some()
    }
}
pub struct CommentPage {
    pub video: VideoId,
    pub comments: Vec<CommentSummary>,
    pub next: Option<CommentCursor>,
    pub limit_reached: bool,
}
fn text(value: &Value, key: &str, limit: usize) -> Option<String> {
    value
        .get(key)?
        .as_str()
        .map(|s| {
            s.chars()
                .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
                .take(limit)
                .collect::<String>()
        })
        .filter(|s| !s.trim().is_empty())
}
pub(crate) fn details(value: &Value) -> VideoDetails {
    let chapters = crate::chapters::parse(value);
    let chapters_unavailable = chapters.is_err();
    VideoDetails {
        description: text(value, "description", 20_000),
        upload_date: value
            .get("upload_date")
            .and_then(Value::as_str)
            .filter(|s| s.len() == 8 && s.bytes().all(|c| c.is_ascii_digit()))
            .map(|s| format!("{}-{}-{}", &s[..4], &s[4..6], &s[6..])),
        view_count: value.get("view_count").and_then(Value::as_u64),
        like_count: value.get("like_count").and_then(Value::as_u64),
        comment_count: value.get("comment_count").and_then(Value::as_u64),
        channel_subscriber_count: value.get("channel_follower_count").and_then(Value::as_u64),
        chapters: chapters.unwrap_or_default(),
        chapters_unavailable,
    }
}
impl YtDlp {
    /// Explicit read-only guest action. Never imports an account session.
    pub fn comments(
        &self,
        video: &VideoId,
        cursor: Option<&CommentCursor>,
        operation: &OperationContext,
    ) -> Result<CommentPage, ProviderError> {
        let start = match cursor {
            None => 0,
            Some(c)
                if c.video == *video
                    && c.generation == operation.session_generation
                    && c.offset < MAX_COMMENTS
                    && c.token.is_none() =>
            {
                c.offset
            }
            Some(_) => return Err(ProviderError::InvalidInput),
        };
        let count = start + PAGE_SIZE + 1;
        let response = self.run(
            &[
                "--get-comments".into(),
                "--no-ignore-errors".into(),
                "--ignore-no-formats-error".into(),
                "--no-playlist".into(),
                "--extractor-args".into(),
                format!("youtube:comment_sort=top;max_comments={count},{count},0,0,1"),
                "--".into(),
                video.watch_url(),
            ],
            operation,
        )?;
        if operation.cancel.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        parse_page(&response, video, cursor, operation.session_generation)
    }
}
fn parse_comment(value: &Value) -> Result<CommentSummary, ProviderError> {
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| valid_id(id))
        .ok_or(ProviderError::MalformedOutput)?;
    if value.get("parent").and_then(Value::as_str) != Some("root") {
        return Err(ProviderError::MalformedOutput);
    }
    Ok(CommentSummary {
        id: id.to_owned(),
        author: text(value, "author", 200),
        author_id: value
            .get("author_id")
            .and_then(Value::as_str)
            .and_then(|id| ChannelId::new(id).ok()),
        text: text(value, "text", 10_000).ok_or(ProviderError::MalformedOutput)?,
        published_text: text(value, "_time_text", 100),
        like_count: value.get("like_count").and_then(Value::as_u64),
        author_is_uploader: value
            .get("author_is_uploader")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        author_thumbnail_url: value
            .get("author_thumbnail")
            .and_then(Value::as_str)
            .and_then(crate::channel_avatar::safe_avatar),
    })
}
fn parse_page(
    value: &Value,
    video: &VideoId,
    cursor: Option<&CommentCursor>,
    generation: u64,
) -> Result<CommentPage, ProviderError> {
    if value.get("id").and_then(Value::as_str) != Some(video.as_str()) {
        return Err(ProviderError::MalformedOutput);
    }
    let entries = match value.get("comments") {
        Some(Value::Null) => return Err(ProviderError::Unavailable),
        Some(Value::Array(entries)) => entries,
        _ => return Err(ProviderError::MalformedOutput),
    };
    let start = cursor.map_or(0, |c| c.offset);
    if entries.len() > start + PAGE_SIZE + 1 {
        return Err(ProviderError::OutputTooLarge);
    }
    let mut seen = std::collections::HashSet::new();
    let normalized = entries
        .iter()
        .map(parse_comment)
        .collect::<Result<Vec<_>, _>>()?;
    if normalized.iter().any(|item| !seen.insert(item.id.as_str())) {
        return Err(ProviderError::MalformedOutput);
    }
    // Comments can be reordered/deleted between requests. Checking only the last
    // ID misses a seen comment moved past that boundary. Require the complete
    // published prefix to match before exposing another page; text may change.
    if let Some(cursor) = cursor
        && (cursor.prefix_ids.len() != start
            || normalized.len() < start
            || normalized[..start]
                .iter()
                .map(|comment| comment.id.as_str())
                .ne(cursor.prefix_ids.iter().map(String::as_str)))
    {
        return Err(ProviderError::Unavailable);
    }
    let more = normalized.len() > start + PAGE_SIZE;
    let end = normalized.len().min(start + PAGE_SIZE);
    let next = if more && end < MAX_COMMENTS {
        Some(CommentCursor {
            video: video.clone(),
            generation,
            offset: end,
            prefix_ids: normalized[..end]
                .iter()
                .map(|comment| comment.id.clone())
                .collect(),
            token: None,
        })
    } else {
        None
    };
    let comments = normalized.into_iter().skip(start).take(PAGE_SIZE).collect();
    Ok(CommentPage {
        video: video.clone(),
        comments,
        next,
        limit_reached: more && end >= MAX_COMMENTS,
    })
}
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

/// Guest comments: native InnerTube first, the supervised extractor as the
/// fallback. A first page (no cursor, or a native start cursor) falls back to
/// the extractor on unsupported/malformed native responses; later pages keep
/// the path their cursor came from, so a native continuation never silently
/// turns into a differently ordered replay. No credentials are ever consulted.
pub fn guest_comments(
    native: Option<&GuestTransport>,
    extractor: Option<&YtDlp>,
    video: &VideoId,
    cursor: Option<&CommentCursor>,
    operation: &OperationContext,
) -> Result<CommentPage, ProviderError> {
    if let Some(cursor) = cursor.filter(|c| c.token.is_none()) {
        return extractor.ok_or(ProviderError::HelperUnavailable)?.comments(
            video,
            Some(cursor),
            operation,
        );
    }
    let first = cursor.is_none_or(|c| c.offset == 0);
    let result = match native {
        Some(transport) => native_comments(transport, video, cursor, operation),
        None => Err(ProviderError::ExtractorFailed),
    };
    match (result, extractor) {
        (Err(error), Some(extractor)) if first && watch::fallback_eligible(error) => {
            extractor.comments(video, None, operation)
        }
        (result, _) => result,
    }
}

/// One native page. Without a cursor, the watch page is requested first to
/// obtain the comments-section continuation; the default sort is Top.
pub fn native_comments(
    transport: &GuestTransport,
    video: &VideoId,
    cursor: Option<&CommentCursor>,
    operation: &OperationContext,
) -> Result<CommentPage, ProviderError> {
    let (token, offset, seen): (Arc<str>, usize, Arc<[String]>) = match cursor {
        None => {
            let page = transport.post("next", watch::request(video), operation)?;
            watch::check_video(&page, video)?;
            (
                watch::comment_section_token(&page)?.into(),
                0,
                Arc::from([]),
            )
        }
        Some(c)
            if c.video == *video
                && c.generation == operation.session_generation
                && c.offset < MAX_COMMENTS =>
        {
            let token = c.token.clone().ok_or(ProviderError::InvalidInput)?;
            (token, c.offset, c.prefix_ids.clone())
        }
        Some(_) => return Err(ProviderError::InvalidInput),
    };
    let value = transport.post("next", json!({"continuation": &*token}), operation)?;
    if operation.cancel.is_cancelled() {
        return Err(ProviderError::Cancelled);
    }
    parse_native_page(&value, video, offset, &seen, operation.session_generation)
}

const MAX_CONTINUATION_ITEMS: usize = 100;
const MAX_MUTATIONS: usize = 2_000;

fn parse_native_page(
    value: &Value,
    video: &VideoId,
    offset: usize,
    seen: &Arc<[String]>,
    generation: u64,
) -> Result<CommentPage, ProviderError> {
    let endpoints = value
        .get("onResponseReceivedEndpoints")
        .and_then(Value::as_array)
        .ok_or(ProviderError::MalformedOutput)?;
    if endpoints.len() > 8 {
        return Err(ProviderError::OutputTooLarge);
    }
    let mut items = Vec::new();
    for endpoint in endpoints {
        for action in [
            "reloadContinuationItemsCommand",
            "appendContinuationItemsAction",
        ] {
            if let Some(list) = endpoint
                .get(action)
                .and_then(|action| action.get("continuationItems"))
            {
                items.extend(list.as_array().ok_or(ProviderError::MalformedOutput)?);
            }
        }
        if items.len() > MAX_CONTINUATION_ITEMS {
            return Err(ProviderError::OutputTooLarge);
        }
    }
    let mut entities: HashMap<&str, &Value> = HashMap::new();
    if let Some(mutations) = value.pointer("/frameworkUpdates/entityBatchUpdate/mutations") {
        let mutations = mutations.as_array().ok_or(ProviderError::MalformedOutput)?;
        if mutations.len() > MAX_MUTATIONS {
            return Err(ProviderError::OutputTooLarge);
        }
        for mutation in mutations {
            if let (Some(key), Some(payload)) = (
                mutation.get("entityKey").and_then(Value::as_str),
                mutation.get("payload"),
            ) {
                entities.insert(key, payload);
            }
        }
    }
    let mut comments: Vec<CommentSummary> = Vec::new();
    let mut tokens = std::collections::HashSet::new();
    let (mut recognized, mut threads, mut disabled) = (false, 0usize, false);
    for item in &items {
        let parsed = if let Some(thread) = item.get("commentThreadRenderer") {
            threads += 1;
            thread
                .pointer("/commentViewModel/commentViewModel")
                .map(|model| view_model_comment(model, &entities))
                .or_else(|| {
                    thread
                        .pointer("/comment/commentRenderer")
                        .map(legacy_comment)
                })
        } else if let Some(model) = item.get("commentViewModel") {
            threads += 1;
            Some(view_model_comment(model, &entities))
        } else if let Some(renderer) = item.get("commentRenderer") {
            threads += 1;
            Some(legacy_comment(renderer))
        } else if let Some(next) = item.get("continuationItemRenderer") {
            recognized = true;
            let raw = next
                .pointer("/continuationEndpoint/continuationCommand/token")
                .or_else(|| {
                    next.pointer("/button/buttonRenderer/command/continuationCommand/token")
                })
                .ok_or(ProviderError::MalformedOutput)?;
            tokens.insert(watch::token(raw).ok_or(ProviderError::MalformedOutput)?);
            None
        } else if item.get("commentsHeaderRenderer").is_some() {
            recognized = true;
            None
        } else if item.get("messageRenderer").is_some() {
            recognized = true;
            disabled = true;
            None
        } else {
            None // Unknown rows are skipped; an all-unknown page is rejected below.
        };
        // A thread whose entity is missing or malformed is skipped, not invented.
        if let Some(Ok(comment)) = parsed
            && !seen.contains(&comment.id)
            && comments.iter().all(|c| c.id != comment.id)
        {
            comments.push(comment);
        }
    }
    if tokens.len() > 1 {
        return Err(ProviderError::MalformedOutput);
    }
    if threads > 0 && comments.is_empty() && seen.is_empty() {
        return Err(ProviderError::MalformedOutput);
    }
    if !recognized && threads == 0 {
        if items.is_empty() && offset > 0 {
            // A finished continuation may legitimately return nothing.
            return Ok(CommentPage {
                video: video.clone(),
                comments: Vec::new(),
                next: None,
                limit_reached: false,
            });
        }
        return Err(ProviderError::MalformedOutput);
    }
    if disabled && threads == 0 && offset == 0 {
        return Err(ProviderError::Unavailable);
    }
    // YouTube serves 20 top-level threads per continuation; the published page
    // stays within the existing 20-row contract.
    comments.truncate(PAGE_SIZE);
    let end = offset + comments.len();
    let token = tokens.into_iter().next();
    // A page consisting only of already-published comments means the remote
    // stream is looping; stop rather than follow it indefinitely.
    let more = token.is_some() && !comments.is_empty();
    let next = match token {
        Some(token) if more && end < MAX_COMMENTS => Some(CommentCursor {
            video: video.clone(),
            generation,
            offset: end,
            prefix_ids: seen
                .iter()
                .cloned()
                .chain(comments.iter().map(|comment| comment.id.clone()))
                .collect(),
            token: Some(token.into()),
        }),
        _ => None,
    };
    Ok(CommentPage {
        video: video.clone(),
        comments,
        next,
        limit_reached: more && end >= MAX_COMMENTS,
    })
}

fn bounded(value: Option<&str>, limit: usize) -> Option<String> {
    value.and_then(|value| watch::clean(value, limit))
}

/// Modern shape: the thread's `commentViewModel` names entity keys whose
/// `commentEntityPayload` (and optional toolbar state) carry the comment.
fn view_model_comment(
    model: &Value,
    entities: &HashMap<&str, &Value>,
) -> Result<CommentSummary, ProviderError> {
    let payload = model
        .get("commentKey")
        .and_then(Value::as_str)
        .and_then(|key| entities.get(key))
        .and_then(|payload| payload.get("commentEntityPayload"))
        .ok_or(ProviderError::MalformedOutput)?;
    let properties = payload
        .get("properties")
        .ok_or(ProviderError::MalformedOutput)?;
    let id = properties
        .get("commentId")
        .and_then(Value::as_str)
        .filter(|id| valid_id(id))
        .ok_or(ProviderError::MalformedOutput)?;
    if model
        .get("commentId")
        .and_then(Value::as_str)
        .is_some_and(|model_id| model_id != id)
        || properties
            .get("replyLevel")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            != 0
    {
        return Err(ProviderError::MalformedOutput);
    }
    let author = payload.get("author");
    let toolbar = payload.get("toolbar");
    Ok(CommentSummary {
        id: id.to_owned(),
        author: bounded(
            author
                .and_then(|a| a.get("displayName"))
                .and_then(Value::as_str),
            200,
        ),
        author_id: author
            .and_then(|a| a.get("channelId"))
            .and_then(Value::as_str)
            .and_then(|id| ChannelId::new(id).ok()),
        text: bounded(
            properties
                .pointer("/content/content")
                .and_then(Value::as_str),
            10_000,
        )
        .ok_or(ProviderError::MalformedOutput)?,
        published_text: bounded(properties.get("publishedTime").and_then(Value::as_str), 100),
        like_count: ["likeCountNotliked", "likeCountA11y"]
            .iter()
            .find_map(|key| {
                toolbar
                    .and_then(|t| t.get(*key))
                    .and_then(Value::as_str)
                    .and_then(watch::count)
            }),
        author_is_uploader: author
            .and_then(|a| a.get("isCreator"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        author_thumbnail_url: author
            .and_then(|a| a.get("avatarThumbnailUrl"))
            .or_else(|| payload.pointer("/avatar/image/sources/0/url"))
            .and_then(Value::as_str)
            .and_then(crate::channel_avatar::safe_avatar),
    })
}

/// Legacy `commentRenderer` shape (still accepted when no entities are sent).
fn legacy_comment(renderer: &Value) -> Result<CommentSummary, ProviderError> {
    let id = renderer
        .get("commentId")
        .and_then(Value::as_str)
        .filter(|id| valid_id(id))
        .ok_or(ProviderError::MalformedOutput)?;
    let text = |key: &str, limit| {
        crate::innertube::text(renderer.get(key)?).and_then(|t| watch::clean(&t, limit))
    };
    Ok(CommentSummary {
        id: id.to_owned(),
        author: text("authorText", 200),
        author_id: renderer
            .pointer("/authorEndpoint/browseEndpoint/browseId")
            .and_then(Value::as_str)
            .and_then(|id| ChannelId::new(id).ok()),
        text: text("contentText", 10_000).ok_or(ProviderError::MalformedOutput)?,
        published_text: text("publishedTimeText", 100),
        like_count: text("voteCount", 32).as_deref().and_then(watch::count),
        author_is_uploader: renderer
            .get("authorIsChannelOwner")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        author_thumbnail_url: renderer
            .pointer("/authorThumbnail/thumbnails")
            .and_then(Value::as_array)
            .and_then(|list| list.iter().take(16).next_back())
            .and_then(|thumbnail| thumbnail.get("url")?.as_str())
            .and_then(crate::channel_avatar::safe_avatar),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture(count: usize) -> Value {
        json!({"id":"abcdefghijk","comments": (0..count).map(|i| json!({"id":format!("UgSynthetic{i}"),"parent":"root","text":"Synthetic comment\nsecond line","author":"Synthetic author"})).collect::<Vec<_>>()})
    }
    #[test]
    fn pages_are_bounded_and_preserve_real_comment_identity() {
        let id = VideoId::new("abcdefghijk").unwrap();
        let first = parse_page(&fixture(21), &id, None, 7).unwrap();
        assert_eq!(first.comments.len(), 20);
        let second = parse_page(&fixture(41), &id, first.next.as_ref(), 7).unwrap();
        assert_eq!(second.comments[0].id, "UgSynthetic20");
        let mut changed = fixture(41);
        changed["comments"][19]["id"] = json!("ChangedOrder");
        assert!(matches!(
            parse_page(&changed, &id, first.next.as_ref(), 7),
            Err(ProviderError::Unavailable)
        ));
    }
    #[test]
    fn unchanged_page_boundary_cannot_hide_a_reordered_published_prefix() {
        let id = VideoId::new("abcdefghijk").unwrap();
        let first = parse_page(&fixture(21), &id, None, 7).unwrap();
        let cursor = first.next.as_ref().unwrap();
        for previously_seen in 0..19 {
            let mut changed = fixture(41);
            changed["comments"]
                .as_array_mut()
                .unwrap()
                .swap(previously_seen, 20);
            assert_eq!(changed["comments"][19]["id"], "UgSynthetic19");
            assert!(matches!(
                parse_page(&changed, &id, Some(cursor), 7),
                Err(ProviderError::Unavailable)
            ));
        }
        let mut edited_text = fixture(41);
        edited_text["comments"][0]["text"] = json!("Edited public comment");
        let next = parse_page(&edited_text, &id, Some(cursor), 7).unwrap();
        assert_eq!(next.comments[0].id, "UgSynthetic20");
        assert!(
            next.comments
                .iter()
                .all(|comment| { first.comments.iter().all(|seen| seen.id != comment.id) })
        );
    }
    #[test]
    fn continuation_prefixes_are_bounded_shared_and_checked_on_every_page() {
        let id = VideoId::new("abcdefghijk").unwrap();
        let mut cursor = None;
        for start in (0..MAX_COMMENTS).step_by(PAGE_SIZE) {
            let page =
                parse_page(&fixture(start + PAGE_SIZE + 1), &id, cursor.as_ref(), 7).unwrap();
            assert_eq!(page.comments.len(), PAGE_SIZE);
            assert_eq!(page.comments[0].id, format!("UgSynthetic{start}"));
            if let Some(next) = page.next.as_ref() {
                assert_eq!(next.prefix_ids.len(), start + PAGE_SIZE);
                assert!(next.prefix_ids.len() <= MAX_COMMENTS - PAGE_SIZE);
                assert!(next.prefix_ids.iter().all(|id| id.len() <= 256));
                assert!(std::sync::Arc::ptr_eq(
                    &next.prefix_ids,
                    &next.clone().prefix_ids
                ));
                let mut changed = fixture(next.offset + PAGE_SIZE + 1);
                changed["comments"]
                    .as_array_mut()
                    .unwrap()
                    .swap(0, next.offset);
                assert!(matches!(
                    parse_page(&changed, &id, Some(next), 7),
                    Err(ProviderError::Unavailable)
                ));
            } else {
                assert_eq!(start + PAGE_SIZE, MAX_COMMENTS);
                assert!(page.limit_reached);
            }
            cursor = page.next;
        }
    }
    #[test]
    fn disabled_missing_foreign_and_oversize_comments_never_look_successful() {
        let id = VideoId::new("abcdefghijk").unwrap();
        for value in [
            json!({"id":"abcdefghijk","comments":null}),
            json!({"id":"abcdefghijk"}),
            json!({"id":"zyxwvutsrqp","comments":[]}),
            fixture(22),
        ] {
            assert!(parse_page(&value, &id, None, 0).is_err());
        }
        assert!(
            parse_page(&fixture(0), &id, None, 0)
                .unwrap()
                .comments
                .is_empty()
        );
    }
    #[test]
    fn metadata_and_comment_text_are_bounded_plain_text() {
        let info = details(
            &json!({"description":"a".repeat(25_000), "upload_date":"20260929","view_count":9,"like_count":-1}),
        );
        assert_eq!(info.description.unwrap().len(), 20_000);
        assert_eq!(info.upload_date.as_deref(), Some("2026-09-29"));
        assert_eq!(info.like_count, None);
        let comment = parse_comment(
            &json!({"id":"UgSynthetic","parent":"root","text":"a\u{0000}b","author_id":"UC../bad"}),
        )
        .unwrap();
        assert_eq!(comment.text, "ab");
        assert!(comment.author_id.is_none());
    }
    #[test]
    fn author_thumbnail_is_kept_only_for_exact_public_avatar_hosts() {
        let comment = |url: Value| {
            parse_comment(
                &json!({"id":"UgSynthetic","parent":"root","text":"hello","author_thumbnail":url}),
            )
            .unwrap()
            .author_thumbnail_url
        };
        assert_eq!(
            comment(json!(
                "https://yt3.ggpht.com/synthetic=s48-c-k-c0x00ffffff-no-rj"
            ))
            .as_deref(),
            Some("https://yt3.ggpht.com/synthetic=s48-c-k-c0x00ffffff-no-rj")
        );
        assert!(comment(json!("https://yt3.googleusercontent.com/synthetic")).is_some());
        for hostile in [
            json!("http://yt3.ggpht.com/a"),
            json!("https://yt3.ggpht.com.evil.test/a"),
            json!("https://user@yt3.ggpht.com/a"),
            json!("https://yt3.ggpht.com:444/a"),
            json!("https://i.ytimg.com/vi/a/hqdefault.jpg"),
            json!("https://example.com/a"),
            json!(format!("https://yt3.ggpht.com/{}", "a".repeat(5000))),
            json!(42),
            Value::Null,
        ] {
            assert_eq!(comment(hostile.clone()), None, "{hostile}");
        }
        // A missing field is simply "no portrait", never a parse failure.
        let bare = parse_comment(&json!({"id":"UgSynthetic","parent":"root","text":"hello"}));
        assert!(bare.unwrap().author_thumbnail_url.is_none());
    }
    #[test]
    fn final_page_stops_at_ceiling_and_foreign_cursors_fail_before_spawn() {
        let id = VideoId::new("abcdefghijk").unwrap();
        let cursor = CommentCursor {
            video: id.clone(),
            generation: 7,
            offset: 180,
            prefix_ids: (0..180).map(|i| format!("UgSynthetic{i}")).collect(),
            token: None,
        };
        let final_page = parse_page(&fixture(201), &id, Some(&cursor), 7).unwrap();
        assert_eq!(final_page.comments.len(), 20);
        assert!(final_page.next.is_none());
        assert!(final_page.limit_reached);
        let provider = YtDlp::new("/does/not/exist").unwrap();
        let operation = OperationContext {
            request_id: 1,
            session_generation: 8,
            cancel: Default::default(),
        };
        assert!(matches!(
            provider.comments(&id, Some(&cursor), &operation),
            Err(ProviderError::InvalidInput)
        ));
        let foreign = VideoId::new("zyxwvutsrqp").unwrap();
        let operation = OperationContext {
            session_generation: 7,
            ..operation
        };
        assert!(matches!(
            provider.comments(&foreign, Some(&cursor), &operation),
            Err(ProviderError::InvalidInput)
        ));
    }
    #[test]
    fn generated_json_mutations_preserve_bounds_types_and_plain_text() {
        let id = VideoId::new("abcdefghijk").unwrap();
        let atoms = [
            Value::Null,
            json!(false),
            json!(-1),
            json!({"unexpected":"shape"}),
            json!(["array"]),
            json!("<script>literal text</script>\n\u{0000}é"),
        ];
        for i in 0..256 {
            let atom = &atoms[i % atoms.len()];
            let source = json!({"description": atom, "view_count": atom, "upload_date": atom});
            let normalized = details(&source);
            assert_eq!(normalized.view_count, atom.as_u64());
            match atom.as_str() {
                Some(text) => {
                    let actual = normalized.description.unwrap();
                    assert_eq!(actual, text.replace('\0', ""));
                    assert!(actual.chars().count() <= 20_000);
                    // Markup stays ordinary text; rendering uses Text/TextEdit,
                    // never an HTML renderer or a URL execution path.
                    assert!(actual.starts_with("<script>"));
                }
                None => assert!(normalized.description.is_none()),
            }
            let mut page = fixture(i % 23);
            if i % 23 > 0 {
                page["comments"][0]["author"] = atom.clone();
                page["comments"][0]["text"] = json!("é\u{0000}\n".repeat(i * 20));
            }
            match parse_page(&page, &id, None, i as u64) {
                Ok(page) => {
                    assert_eq!(page.video, id);
                    assert!(page.comments.len() <= 20);
                    assert_eq!(page.next.is_some(), i % 23 == 21);
                    for comment in page.comments {
                        assert!(comment.text.chars().count() <= 10_000);
                        assert!(!comment.text.contains('\0'));
                        assert!(
                            comment
                                .author
                                .as_ref()
                                .is_none_or(|author| author.chars().count() <= 200)
                        );
                    }
                }
                Err(error) => assert!(i % 23 == 22 && error == ProviderError::OutputTooLarge),
            }
        }
    }

    // TEST FIXTURE: synthetic native continuation responses in the shapes
    // observed live on 2026-09-30. No real comments, authors or tokens.
    const CHANNEL: &str = "UCabcdefghijklmnopqrstuv";
    fn thread(id: &str) -> Value {
        json!({"commentThreadRenderer": {"commentViewModel": {"commentViewModel": {
            "commentId": id, "commentKey": format!("key-{id}"), "toolbarStateKey": format!("toolbar-{id}")}}}})
    }
    fn entity(id: &str, n: usize) -> Value {
        json!({"entityKey": format!("key-{id}"), "payload": {"commentEntityPayload": {
            "properties": {"commentId": id, "replyLevel": 0,
                "content": {"content": format!("Synthetic comment {n}\nline")},
                "publishedTime": "7 hours ago (edited)"},
            "author": {"channelId": CHANNEL, "displayName": "@synthetic",
                "avatarThumbnailUrl": "https://yt3.ggpht.com/synthetic=s88-c-k", "isCreator": n == 0},
            "toolbar": {"likeCountNotliked": if n == 0 { "5K" } else { "12" }, "likeCountA11y": "12 likes"}}}})
    }
    fn native(ids: std::ops::Range<usize>, token: Option<&str>, reload: bool) -> Value {
        let names: Vec<String> = ids.clone().map(|i| format!("UgNative{i}")).collect();
        let mut items: Vec<Value> = names.iter().map(|id| thread(id)).collect();
        if let Some(token) = token {
            items.push(
                json!({"continuationItemRenderer": {"continuationEndpoint": {
                "continuationCommand": {"token": token}}}}),
            );
        }
        let mutations: Vec<Value> = names
            .iter()
            .zip(ids)
            .flat_map(|(id, n)| {
                [
                    entity(id, n),
                    json!({"entityKey": format!("toolbar-{id}"), "payload": {
                        "engagementToolbarStateEntityPayload": {"heartState": "TOOLBAR_HEART_STATE_UNHEARTED"}}}),
                ]
            })
            .collect();
        let action = if reload {
            json!([
                {"reloadContinuationItemsCommand": {"continuationItems": [
                    {"commentsHeaderRenderer": {"countText": {"runs": [{"text": "2,824"}, {"text": " Comments"}]}}}]}},
                {"reloadContinuationItemsCommand": {"continuationItems": items}}
            ])
        } else {
            json!([{"appendContinuationItemsAction": {"continuationItems": items}}])
        };
        json!({"onResponseReceivedEndpoints": action,
            "frameworkUpdates": {"entityBatchUpdate": {"mutations": mutations}}})
    }
    fn watch_next(video: &str) -> Value {
        json!({"currentVideoEndpoint": {"watchEndpoint": {"videoId": video}},
            "contents": {"twoColumnWatchNextResults": {"results": {"results": {"contents": [
                {"itemSectionRenderer": {"sectionIdentifier": "comment-item-section", "contents": [
                    {"continuationItemRenderer": {"continuationEndpoint": {"continuationCommand": {
                        "token": "SYNTHETIC_SECTION"}}}}]}}]}}}}})
    }
    fn op() -> OperationContext {
        OperationContext {
            request_id: 1,
            session_generation: 0,
            cancel: Default::default(),
        }
    }

    #[test]
    fn native_view_model_threads_join_their_entity_mutations() {
        let id = VideoId::new("abcdefghijk").unwrap();
        let page = parse_native_page(
            &native(0..20, Some("NEXT"), true),
            &id,
            0,
            &Arc::from([]),
            0,
        )
        .unwrap();
        assert_eq!(page.comments.len(), 20);
        let first = &page.comments[0];
        assert_eq!(first.id, "UgNative0");
        assert_eq!(first.text, "Synthetic comment 0\nline");
        assert_eq!(first.author.as_deref(), Some("@synthetic"));
        assert_eq!(
            first.author_id.as_ref().map(ChannelId::as_str),
            Some(CHANNEL)
        );
        assert_eq!(first.like_count, Some(5_000));
        assert_eq!(
            first.published_text.as_deref(),
            Some("7 hours ago (edited)")
        );
        assert!(first.author_is_uploader && !page.comments[1].author_is_uploader);
        assert_eq!(
            first.author_thumbnail_url.as_deref(),
            Some("https://yt3.ggpht.com/synthetic=s88-c-k")
        );
        let next = page.next.expect("continuation");
        assert!(next.is_native());
        assert_eq!(next.offset, 20);
        assert_eq!(next.prefix_ids.len(), 20);
        assert_eq!(next.token.as_deref(), Some("NEXT"));
    }

    #[test]
    fn legacy_comment_renderer_threads_are_still_accepted() {
        let id = VideoId::new("abcdefghijk").unwrap();
        let value = json!({"onResponseReceivedEndpoints": [{"appendContinuationItemsAction": {"continuationItems": [
            {"commentThreadRenderer": {"comment": {"commentRenderer": {
                "commentId": "UgLegacy1", "contentText": {"runs": [{"text": "Legacy "}, {"text": "text"}]},
                "authorText": {"simpleText": "Legacy author"},
                "authorEndpoint": {"browseEndpoint": {"browseId": CHANNEL}},
                "publishedTimeText": {"runs": [{"text": "1 day ago"}]},
                "voteCount": {"simpleText": "1.2K"}, "authorIsChannelOwner": true,
                "authorThumbnail": {"thumbnails": [
                    {"url": "https://yt3.ggpht.com/legacy=s48"}, {"url": "https://yt3.ggpht.com/legacy=s88"}]}}}}},
            {"commentRenderer": {"commentId": "UgLegacy2", "contentText": {"simpleText": "flat"},
                "authorThumbnail": {"thumbnails": [{"url": "https://example.test/a.png"}]}}}
        ]}}]});
        let page = parse_native_page(&value, &id, 20, &Arc::from([]), 0).unwrap();
        assert_eq!(page.comments.len(), 2);
        let legacy = &page.comments[0];
        assert_eq!(legacy.text, "Legacy text");
        assert_eq!(legacy.author.as_deref(), Some("Legacy author"));
        assert_eq!(legacy.like_count, Some(1_200));
        assert!(legacy.author_is_uploader);
        assert_eq!(
            legacy.author_thumbnail_url.as_deref(),
            Some("https://yt3.ggpht.com/legacy=s88")
        );
        assert_eq!(page.comments[1].author_thumbnail_url, None);
        assert!(page.next.is_none(), "no continuation, final page");
    }

    #[test]
    fn malformed_native_pages_never_look_like_successful_empty_pages() {
        let id = VideoId::new("abcdefghijk").unwrap();
        let none: Arc<[String]> = Arc::from([]);
        let mut orphan = native(0..3, Some("NEXT"), false);
        orphan["frameworkUpdates"]["entityBatchUpdate"]["mutations"] = json!([]);
        let mut two_tokens = native(0..2, Some("A"), false);
        two_tokens["onResponseReceivedEndpoints"][0]["appendContinuationItemsAction"]
            ["continuationItems"]
            .as_array_mut()
            .unwrap()
            .push(json!({"continuationItemRenderer": {"continuationEndpoint": {"continuationCommand": {"token": "B"}}}}));
        let mut reply = native(0..1, None, false);
        reply["frameworkUpdates"]["entityBatchUpdate"]["mutations"][0]["payload"]["commentEntityPayload"]
            ["properties"]["replyLevel"] = json!(1);
        for value in [
            json!({}),
            json!({"onResponseReceivedEndpoints": [{"appendContinuationItemsAction": {"continuationItems": [{"futureRenderer": {}}]}}]}),
            json!({"onResponseReceivedEndpoints": "text"}),
            orphan,
            two_tokens,
            reply,
        ] {
            assert_eq!(
                parse_native_page(&value, &id, 0, &none, 0).err(),
                Some(ProviderError::MalformedOutput)
            );
        }
        let disabled = json!({"onResponseReceivedEndpoints": [{"reloadContinuationItemsCommand": {"continuationItems": [
            {"messageRenderer": {"text": {"simpleText": "Comments are turned off."}}}]}}]});
        assert_eq!(
            parse_native_page(&disabled, &id, 0, &none, 0).err(),
            Some(ProviderError::Unavailable)
        );
        // Zero public comments with a recognized header is a genuine empty page.
        let empty = json!({"onResponseReceivedEndpoints": [{"reloadContinuationItemsCommand": {"continuationItems": [
            {"commentsHeaderRenderer": {}}]}}]});
        let page = parse_native_page(&empty, &id, 0, &none, 0).unwrap();
        assert!(page.comments.is_empty() && page.next.is_none());
        // Text is bounded plain text; oversized responses are rejected.
        let mut long = native(0..1, None, false);
        long["frameworkUpdates"]["entityBatchUpdate"]["mutations"][0]["payload"]["commentEntityPayload"]
            ["properties"]["content"]["content"] = json!("é\u{0000}".repeat(20_000));
        let page = parse_native_page(&long, &id, 0, &none, 0).unwrap();
        assert_eq!(page.comments[0].text.chars().count(), 10_000);
        assert!(!page.comments[0].text.contains('\0'));
        assert_eq!(
            parse_native_page(&native(0..101, None, false), &id, 0, &none, 0).err(),
            Some(ProviderError::OutputTooLarge)
        );
    }

    #[test]
    fn native_paging_uses_real_continuations_drops_repeats_and_stops_at_the_ceiling() {
        let id = VideoId::new("abcdefghijk").unwrap();
        let transport = GuestTransport::with_fixture(vec![
            Ok(watch_next("abcdefghijk")),
            Ok(native(0..20, Some("PAGE2"), true)),
            // Page two repeats the last published comment (pinned repeat).
            Ok(native(19..40, Some("PAGE3"), false)),
            // A page of only repeats means the remote stream is looping.
            Ok(native(0..20, Some("PAGE4"), false)),
        ]);
        let first = guest_comments(Some(&transport), None, &id, None, &op()).unwrap();
        assert_eq!(first.comments.len(), 20);
        let second =
            guest_comments(Some(&transport), None, &id, first.next.as_ref(), &op()).unwrap();
        assert_eq!(second.comments.len(), 20);
        assert_eq!(second.comments[0].id, "UgNative20");
        assert_eq!(second.next.as_ref().unwrap().offset, 40);
        let looping =
            guest_comments(Some(&transport), None, &id, second.next.as_ref(), &op()).unwrap();
        assert!(looping.comments.is_empty() && looping.next.is_none() && !looping.limit_reached);
        {
            let calls = transport.fixture.as_ref().unwrap().calls.lock().unwrap();
            assert_eq!(calls.len(), 4);
            assert_eq!(calls[0].1["videoId"], "abcdefghijk");
            assert_eq!(calls[1].1["continuation"], "SYNTHETIC_SECTION");
            assert_eq!(calls[2].1["continuation"], "PAGE2");
            assert!(calls.iter().all(|(endpoint, _)| endpoint == "next"));
        }
        // The 200-comment ceiling ends browsing with an explicit limit.
        let near = CommentCursor {
            video: id.clone(),
            generation: 0,
            offset: 180,
            prefix_ids: Arc::from([]),
            token: Some("NEAR".into()),
        };
        let transport =
            GuestTransport::with_fixture(vec![Ok(native(180..200, Some("MORE"), false))]);
        let last = guest_comments(Some(&transport), None, &id, Some(&near), &op()).unwrap();
        assert!(last.next.is_none() && last.limit_reached);
        // Cursors stay scoped to their video and session generation.
        let foreign = VideoId::new("zyxwvutsrqp").unwrap();
        assert_eq!(
            native_comments(&transport, &foreign, Some(&near), &op()).err(),
            Some(ProviderError::InvalidInput)
        );
        let provider = YtDlp::new("/does/not/exist").unwrap();
        assert_eq!(
            provider.comments(&id, Some(&near), &op()).err(),
            Some(ProviderError::InvalidInput),
            "a native token is never replayed through the extractor"
        );
    }

    #[test]
    fn only_unsupported_first_pages_fall_back_to_the_extractor() {
        let id = VideoId::new("abcdefghijk").unwrap();
        // The extractor path is observable: this helper does not exist.
        let extractor = YtDlp::new("/does/not/exist").unwrap();
        let transport = GuestTransport::with_fixture(vec![
            Ok(json!({"error": {"status": "INTERNAL"}})),
            Ok(watch_next("zyxwvutsrqp")),
            Ok(json!({"error": {"status": "RESOURCE_EXHAUSTED"}})),
        ]);
        for _ in 0..2 {
            assert_eq!(
                guest_comments(Some(&transport), Some(&extractor), &id, None, &op()).err(),
                Some(ProviderError::HelperUnavailable),
                "unsupported/foreign native response falls back"
            );
        }
        assert_eq!(
            guest_comments(Some(&transport), Some(&extractor), &id, None, &op()).err(),
            Some(ProviderError::RateLimited),
            "rate limits are not repeated through the helper"
        );
        assert_eq!(
            guest_comments(None, Some(&extractor), &id, None, &op()).err(),
            Some(ProviderError::HelperUnavailable),
            "no native transport uses the extractor"
        );
        let later = CommentCursor {
            video: id.clone(),
            generation: 0,
            offset: 20,
            prefix_ids: Arc::from([]),
            token: Some("LATER".into()),
        };
        let transport = GuestTransport::with_fixture(vec![Ok(json!({"unexpected": true}))]);
        assert_eq!(
            guest_comments(Some(&transport), Some(&extractor), &id, Some(&later), &op()).err(),
            Some(ProviderError::MalformedOutput),
            "a native continuation never becomes a differently ordered replay"
        );
        let cancelled = op();
        cancelled.cancel.cancel();
        let transport = GuestTransport::with_fixture(vec![]);
        assert_eq!(
            guest_comments(Some(&transport), Some(&extractor), &id, None, &cancelled).err(),
            Some(ProviderError::Cancelled)
        );
    }
}
