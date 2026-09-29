// SPDX-License-Identifier: GPL-3.0-or-later
//! Guest channel/playlist/search pages via the supervised yt-dlp adapter.
//! Cursor offsets are application-owned, not exposed InnerTube continuation tokens.
use crate::{YtDlp, is_promoted, safe_thumbnail, summary};
use serde_json::Value;
use serein_core::{
    CatalogItem, ChannelHandle, ChannelId, ChannelSummary, OperationContext, PlaylistId,
    PlaylistSummary, ProviderError,
};
use url::Url;

const PAGE_SIZE: usize = 20;
const SEARCH_LIMIT: usize = 200;
const BROWSE_LIMIT: usize = 10_000;
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SearchKind {
    All,
    Videos,
    Channels,
    Playlists,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ChannelTab {
    Videos,
    Shorts,
    Streams,
    Playlists,
}
#[derive(Clone, PartialEq, Eq)]
pub enum CatalogRequest {
    Search {
        query: String,
        kind: SearchKind,
    },
    Channel {
        id: ChannelId,
        tab: ChannelTab,
    },
    ChannelHandle {
        handle: ChannelHandle,
        tab: ChannelTab,
    },
    Playlist {
        id: PlaylistId,
    },
}
#[derive(Clone)]
pub struct CatalogCursor {
    request: CatalogRequest,
    offset: usize,
    generation: u64,
}
pub enum CatalogHeader {
    Search,
    Channel(ChannelSummary),
    Playlist(PlaylistSummary),
}
pub struct CatalogPage {
    pub header: CatalogHeader,
    pub items: Vec<CatalogItem>,
    pub next: Option<CatalogCursor>,
    /// Unknown/unavailable entries were skipped. Never implies a complete empty result.
    pub partial: bool,
    /// The explicit catalog safety limit was reached, not the remote end of the list.
    pub limit_reached: bool,
}
impl CatalogRequest {
    fn url(&self) -> Result<String, ProviderError> {
        match self {
            Self::Search { query, kind } => {
                if query.trim().is_empty()
                    || query.chars().count() > 200
                    || query.chars().any(char::is_control)
                {
                    return Err(ProviderError::InvalidInput);
                }
                let mut url =
                    Url::parse("https://www.youtube.com/results").expect("fixed HTTPS URL");
                url.query_pairs_mut()
                    .append_pair("search_query", query.trim());
                // SearchFilter { filters(field2): { type(field2): 1/2/3 }}.
                // YouTube.js encodes the base64 value with encodeURIComponent;
                // SearchURLIE forwards the parsed sp value as InnerTube params.
                let params = match kind {
                    SearchKind::All => None,
                    SearchKind::Videos => Some("EgIQAQ%3D%3D"),
                    SearchKind::Channels => Some("EgIQAg%3D%3D"),
                    SearchKind::Playlists => Some("EgIQAw%3D%3D"),
                };
                if let Some(params) = params {
                    url.query_pairs_mut().append_pair("sp", params);
                }
                Ok(url.into())
            }
            Self::Channel { tab, .. } | Self::ChannelHandle { tab, .. } => {
                let base = match self {
                    Self::Channel { id, .. } => ChannelId::new(id.as_str())?.browse_url(),
                    Self::ChannelHandle { handle, .. } => handle.browse_url(),
                    _ => unreachable!(),
                };
                let tab = match tab {
                    ChannelTab::Videos => "videos",
                    ChannelTab::Shorts => "shorts",
                    ChannelTab::Streams => "streams",
                    ChannelTab::Playlists => "playlists",
                };
                Ok(format!("{base}/{tab}"))
            }
            Self::Playlist { id } => Ok(PlaylistId::new(id.as_str())?.browse_url()),
        }
    }
    fn limit(&self) -> usize {
        if matches!(self, Self::Search { .. }) {
            SEARCH_LIMIT
        } else {
            BROWSE_LIMIT
        }
    }
    fn accepts(&self, item: &CatalogItem) -> bool {
        if matches!(
            self,
            Self::Search {
                kind: SearchKind::All,
                ..
            }
        ) {
            return true;
        }
        match item {
            CatalogItem::Video(_) => matches!(
                self,
                Self::Search {
                    kind: SearchKind::Videos,
                    ..
                } | Self::Channel {
                    tab: ChannelTab::Videos | ChannelTab::Shorts | ChannelTab::Streams,
                    ..
                } | Self::ChannelHandle {
                    tab: ChannelTab::Videos | ChannelTab::Shorts | ChannelTab::Streams,
                    ..
                } | Self::Playlist { .. }
            ),
            CatalogItem::Channel(_) => matches!(
                self,
                Self::Search {
                    kind: SearchKind::Channels,
                    ..
                }
            ),
            CatalogItem::Playlist(_) => matches!(
                self,
                Self::Search {
                    kind: SearchKind::Playlists,
                    ..
                } | Self::Channel {
                    tab: ChannelTab::Playlists,
                    ..
                } | Self::ChannelHandle {
                    tab: ChannelTab::Playlists,
                    ..
                }
            ),
        }
    }
}
impl YtDlp {
    /// Dedicated worker only. Uses guest configuration even if another account is connected.
    pub fn catalog(
        &self,
        request: &CatalogRequest,
        cursor: Option<&CatalogCursor>,
        operation: &OperationContext,
    ) -> Result<CatalogPage, ProviderError> {
        let url = request.url()?;
        let start = match cursor {
            Some(cursor)
                if cursor.request == *request
                    && cursor.generation == operation.session_generation =>
            {
                cursor.offset
            }
            Some(_) => return Err(ProviderError::InvalidInput),
            None => 0,
        };
        if start >= request.limit() {
            return Err(ProviderError::InvalidInput);
        }
        // One bounded lookahead distinguishes a full final page from more results.
        // Lazy extraction avoids downloading a whole remote collection merely for its count.
        let response = self.run(
            &[
                "--flat-playlist".into(),
                "--lazy-playlist".into(),
                "--playlist-items".into(),
                format!("{}:{}", start + 1, start + PAGE_SIZE + 1),
                "--".into(),
                url,
            ],
            operation,
        )?;
        if operation.cancel.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        parse_page(&response, request, start, operation.session_generation)
    }
}
fn text(value: &Value, key: &str, limit: usize) -> Option<String> {
    value.get(key)?.as_str().filter(|s| !s.is_empty()).map(|s| {
        s.chars()
            .filter(|c| !c.is_control() || *c == '\n')
            .take(limit)
            .collect()
    })
}
fn thumbnail(value: &Value) -> Option<String> {
    value
        .get("thumbnail")
        .and_then(Value::as_str)
        .and_then(safe_thumbnail)
        .or_else(|| {
            value
                .get("thumbnails")?
                .as_array()?
                .iter()
                .rev()
                .find_map(|v| v.get("url")?.as_str().and_then(safe_thumbnail))
        })
}
fn channel(value: &Value) -> Result<ChannelSummary, ProviderError> {
    let id = value
        .get("channel_id")
        .or_else(|| value.get("id"))
        .and_then(Value::as_str)
        .ok_or(ProviderError::MalformedOutput)?;
    Ok(ChannelSummary {
        id: ChannelId::new(id).map_err(|_| ProviderError::MalformedOutput)?,
        title: text(value, "channel", 500)
            .or_else(|| text(value, "title", 500))
            .ok_or(ProviderError::MalformedOutput)?,
        description: text(value, "description", 4000),
        thumbnail_url: thumbnail(value),
        subscriber_count: value.get("channel_follower_count").and_then(Value::as_u64),
    })
}
fn playlist(value: &Value) -> Result<PlaylistSummary, ProviderError> {
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .ok_or(ProviderError::MalformedOutput)?;
    Ok(PlaylistSummary {
        id: PlaylistId::new(id).map_err(|_| ProviderError::MalformedOutput)?,
        title: text(value, "title", 500).ok_or(ProviderError::MalformedOutput)?,
        description: text(value, "description", 4000),
        channel: text(value, "channel", 200).or_else(|| text(value, "uploader", 200)),
        channel_id: value
            .get("channel_id")
            .and_then(Value::as_str)
            .and_then(|id| ChannelId::new(id).ok()),
        thumbnail_url: thumbnail(value),
        video_count: value.get("playlist_count").and_then(Value::as_u64),
    })
}
fn item(value: &Value) -> Result<CatalogItem, ProviderError> {
    let key = value.get("ie_key").and_then(Value::as_str).unwrap_or("");
    let raw_url = value
        .get("url")
        .and_then(Value::as_str)
        .ok_or(ProviderError::MalformedOutput)?;
    if key == "Youtube" {
        let video = summary(value)?;
        if serein_core::VideoId::from_url(raw_url)? != video.id {
            return Err(ProviderError::MalformedOutput);
        }
        return Ok(CatalogItem::Video(video));
    }
    if key != "YoutubeTab" {
        return Err(ProviderError::MalformedOutput);
    }
    if let Ok(id) = ChannelId::from_url(raw_url) {
        let channel = channel(value)?;
        if id != channel.id {
            return Err(ProviderError::MalformedOutput);
        }
        return Ok(CatalogItem::Channel(channel));
    }
    if let Ok(id) = PlaylistId::from_url(raw_url) {
        let playlist = playlist(value)?;
        if id != playlist.id {
            return Err(ProviderError::MalformedOutput);
        }
        return Ok(CatalogItem::Playlist(playlist));
    }
    Err(ProviderError::MalformedOutput)
}
fn parse_page(
    value: &Value,
    request: &CatalogRequest,
    start: usize,
    generation: u64,
) -> Result<CatalogPage, ProviderError> {
    if value.get("_type").and_then(Value::as_str) != Some("playlist") {
        return Err(ProviderError::MalformedOutput);
    }
    let header = match request {
        CatalogRequest::Search { .. } => CatalogHeader::Search,
        CatalogRequest::Channel { id, .. } => {
            let item = channel(value)?;
            if item.id != *id {
                return Err(ProviderError::MalformedOutput);
            }
            CatalogHeader::Channel(item)
        }
        CatalogRequest::ChannelHandle { .. } => CatalogHeader::Channel(channel(value)?),
        CatalogRequest::Playlist { id } => {
            let item = playlist(value)?;
            if item.id != *id {
                return Err(ProviderError::MalformedOutput);
            }
            CatalogHeader::Playlist(item)
        }
    };
    let entries = value
        .get("entries")
        .and_then(Value::as_array)
        .ok_or(ProviderError::MalformedOutput)?;
    if entries.len() > PAGE_SIZE + 1 {
        return Err(ProviderError::OutputTooLarge);
    }
    let mut items = Vec::with_capacity(PAGE_SIZE);
    let mut partial = false;
    for raw in entries.iter().take(PAGE_SIZE) {
        if is_promoted(raw) {
            continue;
        }
        match item(raw) {
            Ok(item) if request.accepts(&item) => items.push(item),
            _ => partial = true,
        }
    }
    if items.is_empty() && partial {
        return Err(ProviderError::MalformedOutput);
    }
    let more = entries.len() > PAGE_SIZE;
    let limit_reached = more && start + PAGE_SIZE >= request.limit();
    // Resolve mutable handles once; continuation follows the real UC identity.
    let canonical = match (request, &header) {
        (CatalogRequest::ChannelHandle { tab, .. }, CatalogHeader::Channel(channel)) => {
            CatalogRequest::Channel {
                id: channel.id.clone(),
                tab: *tab,
            }
        }
        _ => request.clone(),
    };
    let next = (more && !limit_reached).then(|| CatalogCursor {
        request: canonical,
        offset: start + PAGE_SIZE,
        generation,
    });
    Ok(CatalogPage {
        header,
        items,
        next,
        partial,
        limit_reached,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use serein_core::CancellationToken;
    fn video() -> Value {
        json!({"_type":"url","ie_key":"Youtube","url":"https://www.youtube.com/watch?v=abcdefghijk","id":"abcdefghijk","title":"Synthetic video","channel":"Synthetic channel"})
    }
    fn channel_item() -> Value {
        json!({"_type":"url","ie_key":"YoutubeTab","url":"https://www.youtube.com/channel/UCabcdefghijklmnopqrstuv","id":"UCabcdefghijklmnopqrstuv","channel_id":"UCabcdefghijklmnopqrstuv","title":"Synthetic channel","channel_follower_count":12,"thumbnails":[{"url":"https://yt3.googleusercontent.com/synthetic"}]})
    }
    fn playlist_item() -> Value {
        json!({"_type":"url","ie_key":"YoutubeTab","url":"https://www.youtube.com/playlist?list=PLsynthetic","id":"PLsynthetic","title":"Synthetic playlist"})
    }
    fn request() -> CatalogRequest {
        CatalogRequest::Search {
            query: "synthetic".into(),
            kind: SearchKind::All,
        }
    }
    #[test]
    fn search_filters_match_verified_protobuf_and_url_encoding() {
        for (kind, param) in [
            (SearchKind::Videos, "EgIQAQ%3D%3D"),
            (SearchKind::Channels, "EgIQAg%3D%3D"),
            (SearchKind::Playlists, "EgIQAw%3D%3D"),
        ] {
            let raw = CatalogRequest::Search {
                query: "a & b".into(),
                kind,
            }
            .url()
            .unwrap();
            let url = Url::parse(&raw).unwrap();
            assert_eq!(
                url.query_pairs()
                    .find(|(k, _)| k == "search_query")
                    .unwrap()
                    .1,
                "a & b"
            );
            assert_eq!(url.query_pairs().find(|(k, _)| k == "sp").unwrap().1, param);
        }
    }
    #[test]
    fn typed_results_preserve_kind_and_absent_metadata() {
        let page = parse_page(
            &json!({"_type":"playlist","entries":[video(),channel_item(),playlist_item()]}),
            &request(),
            0,
            0,
        )
        .unwrap();
        assert_eq!(page.items.len(), 3);
        assert!(!page.partial);
        assert!(matches!(&page.items[0], CatalogItem::Video(_)));
        let CatalogItem::Channel(channel) = &page.items[1] else {
            panic!("wrong kind")
        };
        assert_eq!(channel.subscriber_count, Some(12));
        assert!(channel.thumbnail_url.is_some());
        let CatalogItem::Playlist(playlist) = &page.items[2] else {
            panic!("wrong kind")
        };
        assert_eq!(playlist.video_count, None);
        assert!(playlist.channel.is_none());
    }
    #[test]
    fn unknown_entries_are_not_successful_empty_results_and_promotions_are_excluded() {
        assert!(parse_page(&json!({"_type":"playlist","entries":[{"_type":"url","ie_key":"Unknown","url":"https://evil.example/"}]}),&request(),0,0).is_err());
        let page=parse_page(&json!({"_type":"playlist","entries":[video(),{"is_ad":true,"id":"ad"},{"newShape":{}}]}),&request(),0,0).unwrap();
        assert_eq!(page.items.len(), 1);
        assert!(page.partial);
        assert!(parse_page(&json!({"entries":[]}), &request(), 0, 0).is_err());
    }
    #[test]
    fn lookahead_cursor_is_scope_and_generation_bound_and_limits_are_explicit() {
        let response = json!({"_type":"playlist","entries":vec![video();21]});
        let page = parse_page(&response, &request(), 0, 7).unwrap();
        assert_eq!(page.items.len(), 20);
        assert!(page.next.is_some());
        let operation = OperationContext {
            request_id: 2,
            session_generation: 8,
            cancel: CancellationToken::default(),
        };
        let provider = YtDlp::new("/synthetic/nonexistent-helper").unwrap();
        assert!(matches!(
            provider.catalog(&request(), page.next.as_ref(), &operation),
            Err(ProviderError::InvalidInput)
        ));
        let other = CatalogRequest::Search {
            query: "other".into(),
            kind: SearchKind::All,
        };
        assert!(matches!(
            provider.catalog(&other, page.next.as_ref(), &operation),
            Err(ProviderError::InvalidInput)
        ));
        let capped = parse_page(&response, &request(), 180, 7).unwrap();
        assert!(capped.limit_reached);
        assert!(capped.next.is_none());
        let final_page = parse_page(
            &json!({"_type":"playlist","entries":vec![video();20]}),
            &request(),
            20,
            7,
        )
        .unwrap();
        assert!(final_page.next.is_none());
    }
    #[test]
    fn mismatched_identity_and_unsafe_entry_targets_fail_closed() {
        let req = CatalogRequest::Channel {
            id: ChannelId::new("UCabcdefghijklmnopqrstuv").unwrap(),
            tab: ChannelTab::Videos,
        };
        assert!(parse_page(&json!({"_type":"playlist","id":"UCzzzzzzzzzzzzzzzzzzzzzz","title":"Other","entries":[]}),&req,0,0).is_err());
        let mut raw = video();
        raw["url"] = json!("https://evil.example/watch?v=abcdefghijk");
        assert!(item(&raw).is_err());
        let mut raw = playlist_item();
        raw["id"] = json!("PLdifferent");
        assert!(item(&raw).is_err());
        let mut raw = channel_item();
        raw["thumbnails"] = json!([{"url":"https://yt3.googleusercontent.com.evil.example/a"}]);
        let CatalogItem::Channel(parsed) = item(&raw).unwrap() else {
            panic!("wrong kind")
        };
        assert!(parsed.thumbnail_url.is_none());
    }
    #[test]
    fn channel_and_playlist_headers_are_real_bounded_metadata() {
        let id = ChannelId::new("UCabcdefghijklmnopqrstuv").unwrap();
        let req = CatalogRequest::Channel {
            id: id.clone(),
            tab: ChannelTab::Videos,
        };
        assert_eq!(req.url().unwrap(), format!("{}/videos", id.browse_url()));
        let mut outer = channel_item();
        outer["_type"] = json!("playlist");
        outer["entries"] = json!([video()]);
        let page = parse_page(&outer, &req, 0, 0).unwrap();
        assert!(matches!(page.header, CatalogHeader::Channel(_)));
        assert_eq!(page.items.len(), 1);
        let req = CatalogRequest::Playlist {
            id: PlaylistId::new("PLsynthetic").unwrap(),
        };
        let mut outer = playlist_item();
        outer["_type"] = json!("playlist");
        outer["entries"] = json!([video()]);
        outer["playlist_count"] = json!(7);
        let page = parse_page(&outer, &req, 0, 0).unwrap();
        let CatalogHeader::Playlist(header) = page.header else {
            panic!("wrong header")
        };
        assert_eq!(header.video_count, Some(7));
    }

    #[test]
    fn handle_resolution_requires_real_identity_and_pins_continuation_to_it() {
        let request = CatalogRequest::ChannelHandle {
            handle: ChannelHandle::new("@synthetic-channel").unwrap(),
            tab: ChannelTab::Videos,
        };
        assert_eq!(
            request.url().unwrap(),
            "https://www.youtube.com/@synthetic-channel/videos"
        );
        let mut response = channel_item();
        response["_type"] = json!("playlist");
        response["entries"] = json!(vec![video(); 21]);
        let page = parse_page(&response, &request, 0, 7).unwrap();
        let cursor = page.next.unwrap();
        assert!(
            matches!(cursor.request, CatalogRequest::Channel { ref id, tab: ChannelTab::Videos }
            if id.as_str() == "UCabcdefghijklmnopqrstuv")
        );
        assert_eq!(cursor.generation, 7);
        assert_eq!(cursor.offset, PAGE_SIZE);
        response["channel_id"] = json!("@synthetic-channel");
        response["id"] = json!("@synthetic-channel");
        assert!(parse_page(&response, &request, 0, 7).is_err());
    }
}
