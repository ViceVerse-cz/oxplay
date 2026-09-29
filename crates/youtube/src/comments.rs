// SPDX-License-Identifier: GPL-3.0-or-later
//! Bounded guest comments using the reviewed yt-dlp extraction implementation.
//! Offset pages replay the prefix; they are not remote continuation tokens.
use crate::YtDlp;
use serde_json::Value;
use serein_core::{
    ChannelId, CommentSummary, OperationContext, ProviderError, VideoDetails, VideoId,
};

const PAGE_SIZE: usize = 20;
const MAX_COMMENTS: usize = 200;
#[derive(Clone)]
pub struct CommentCursor {
    video: VideoId,
    generation: u64,
    offset: usize,
    // Prefix replay must preserve every previously published identity, not just
    // the page boundary. Arc keeps UI Back-stack cursor clones inexpensive.
    prefix_ids: std::sync::Arc<[String]>,
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
                    && c.offset < MAX_COMMENTS =>
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
        .filter(|id| {
            !id.is_empty()
                && id.len() <= 256
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
        })
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
    fn final_page_stops_at_ceiling_and_foreign_cursors_fail_before_spawn() {
        let id = VideoId::new("abcdefghijk").unwrap();
        let cursor = CommentCursor {
            video: id.clone(),
            generation: 7,
            offset: 180,
            prefix_ids: (0..180).map(|i| format!("UgSynthetic{i}")).collect(),
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
}
