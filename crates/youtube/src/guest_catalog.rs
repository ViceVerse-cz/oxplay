// SPDX-License-Identifier: GPL-3.0-or-later
//! Native guest search, channel and public playlist pages over the anonymous
//! InnerTube transport. Only the reviewed item containers of each response are
//! read: header engagement panels, chip bars and shelves never become items or
//! continuation tokens. Every page carries at most 20 records; extra records of
//! one response are buffered in the opaque cursor for the next page.
use crate::{
    catalog::{
        CatalogCursor, CatalogHeader, CatalogPage, CatalogRequest, CatalogSource, ChannelTab,
        PAGE_SIZE, SearchKind,
    },
    channel_avatar::ChannelProfile,
    innertube::GuestTransport,
    renderers::{self, Parsed, text},
};
use oxplay_core::{
    CatalogItem, ChannelHandle, ChannelId, ChannelSummary, OperationContext, PlaylistId,
    PlaylistSummary, ProviderError,
};
use serde_json::{Value, json};
use std::{collections::HashSet, sync::Arc};

/// Continuation requests one page may issue to fill 20 rows after filtering.
const MAX_CONTINUATIONS_PER_PAGE: usize = 3;
/// Playlist responses carry up to 100 rows; the remainder waits in the cursor.
const MAX_BUFFERED: usize = 256;
/// Visited list entries per response (containers included).
const MAX_ENTRIES: usize = 5_000;
const MAX_ITEMS_PER_RESPONSE: usize = 200;
const MAX_DEPTH: u8 = 4;

/// Opaque native continuation state. Deliberately has no Debug: the token and
/// the buffered records are provider-owned data. Buffers are shared between
/// successive cursors instead of being copied into each saved Back location.
#[derive(Clone)]
pub(crate) struct Continuation {
    token: Option<Arc<str>>,
    buffer: Arc<[CatalogItem]>,
    header: Arc<CatalogHeader>,
}

/// InnerTube `SearchFilter { type }` params, base64 without URL encoding
/// (the web client's `sp=EgIQAQ%3D%3D` query value, decoded). Verified live
/// 2026-09-30: each filter returned only that result kind.
pub(crate) fn search_params(kind: SearchKind) -> Option<&'static str> {
    match kind {
        SearchKind::All => None,
        SearchKind::Videos => Some("EgIQAQ=="),
        SearchKind::Channels => Some("EgIQAg=="),
        SearchKind::Playlists => Some("EgIQAw=="),
    }
}

/// Channel tab params as advertised by the live channel tab endpoints
/// (`tabRenderer.endpoint.browseEndpoint.params`, URL-decoded) on 2026-09-30.
/// The response's selected tab is checked, so a silent change fails closed.
pub(crate) fn tab_params(tab: ChannelTab) -> &'static str {
    match tab {
        ChannelTab::Videos => "EgZ2aWRlb3PyBgQKAjoA",
        ChannelTab::Shorts => "EgZzaG9ydHPyBgUKA5oBAA==",
        ChannelTab::Streams => "EgdzdHJlYW1z8gYECgJ6AA==",
        ChannelTab::Playlists => "EglwbGF5bGlzdHPyBgoKCEIGCgIQaCIA",
    }
}

fn tab_path(tab: ChannelTab) -> &'static str {
    match tab {
        ChannelTab::Videos => "/videos",
        ChannelTab::Shorts => "/shorts",
        ChannelTab::Streams => "/streams",
        ChannelTab::Playlists => "/playlists",
    }
}

#[derive(Clone, Copy)]
enum Scope<'a> {
    Search,
    Channel {
        owner: &'a ChannelSummary,
        tab: ChannelTab,
    },
    Playlist(&'a PlaylistId),
}
impl<'a> Scope<'a> {
    fn of(header: &'a CatalogHeader, request: &CatalogRequest) -> Self {
        match (header, request) {
            (CatalogHeader::Channel(owner), CatalogRequest::Channel { tab, .. }) => {
                Self::Channel { owner, tab: *tab }
            }
            (CatalogHeader::Playlist(playlist), _) => Self::Playlist(&playlist.id),
            _ => Self::Search,
        }
    }
}

#[derive(Default)]
struct Chunk {
    items: Vec<CatalogItem>,
    token: Option<String>,
    partial: bool,
    visited: usize,
}
impl Chunk {
    fn token(&mut self, value: Option<&Value>) -> Result<(), ProviderError> {
        let Some(token) = value.and_then(Value::as_str) else {
            self.partial = true;
            return Ok(());
        };
        if token.is_empty() || token.len() > 16_384 || token.chars().any(char::is_control) {
            return Err(ProviderError::MalformedOutput);
        }
        match &self.token {
            Some(existing) if existing != token => Err(ProviderError::MalformedOutput),
            _ => {
                self.token = Some(token.to_owned());
                Ok(())
            }
        }
    }
    fn push(&mut self, item: CatalogItem, request: &CatalogRequest) -> Result<(), ProviderError> {
        if !request.accepts(&item) {
            self.partial = true;
            return Ok(());
        }
        self.items.push(item);
        if self.items.len() > MAX_ITEMS_PER_RESPONSE {
            return Err(ProviderError::OutputTooLarge);
        }
        Ok(())
    }
}

/// Shelves, chips and query hints: containers of duplicates or navigation,
/// deliberately not shown. Their subtrees are never traversed.
const SKIPPED: &[&str] = &[
    "shelfRenderer",
    "reelShelfRenderer",
    "richShelfRenderer",
    "richSectionRenderer",
    "gridShelfViewModel",
    "horizontalCardListRenderer",
    "universalWatchCardRenderer",
    "showingResultsForRenderer",
    "didYouMeanRenderer",
    "includingResultsForRenderer",
    "feedFilterChipBarRenderer",
    "chipCloudRenderer",
    // Recognized empty-list notices.
    "messageRenderer",
    "backgroundPromoRenderer",
];

fn collect(
    list: Option<&Value>,
    scope: Scope,
    request: &CatalogRequest,
    chunk: &mut Chunk,
    depth: u8,
) -> Result<(), ProviderError> {
    let Some(entries) = list.and_then(Value::as_array) else {
        chunk.partial = true;
        return Ok(());
    };
    for entry in entries {
        entry_one(entry, scope, request, chunk, depth)?;
    }
    Ok(())
}

fn entry_one(
    entry: &Value,
    scope: Scope,
    request: &CatalogRequest,
    chunk: &mut Chunk,
    depth: u8,
) -> Result<(), ProviderError> {
    chunk.visited += 1;
    if chunk.visited > MAX_ENTRIES {
        return Err(ProviderError::OutputTooLarge);
    }
    let Some(map) = entry.as_object() else {
        chunk.partial = true;
        return Ok(());
    };
    let Some((key, value)) = map
        .iter()
        .find(|(key, _)| !matches!(key.as_str(), "trackingParams" | "clickTrackingParams"))
    else {
        chunk.partial = true;
        return Ok(());
    };
    let key = key.as_str();
    if SKIPPED.contains(&key) {
        return Ok(());
    }
    let container = matches!(
        key,
        "itemSectionRenderer" | "gridRenderer" | "playlistVideoListRenderer" | "richItemRenderer"
    );
    if container && depth >= MAX_DEPTH {
        chunk.partial = true;
        return Ok(());
    }
    match key {
        "continuationItemRenderer" => {
            chunk.token(value.pointer("/continuationEndpoint/continuationCommand/token"))
        }
        "continuationItemViewModel" => chunk.token(
            value.pointer("/continuationCommand/innertubeCommand/continuationCommand/token"),
        ),
        "richItemRenderer" => match value.get("content") {
            Some(content) => entry_one(content, scope, request, chunk, depth + 1),
            None => {
                chunk.partial = true;
                Ok(())
            }
        },
        "itemSectionRenderer" => collect(value.get("contents"), scope, request, chunk, depth + 1),
        "gridRenderer" => collect(value.get("items"), scope, request, chunk, depth + 1),
        "playlistVideoListRenderer" => {
            if let (Scope::Playlist(id), Some(list_id)) = (scope, value.get("playlistId"))
                && list_id.as_str() != Some(id.as_str())
            {
                return Err(ProviderError::MalformedOutput);
            }
            collect(value.get("contents"), scope, request, chunk, depth + 1)
        }
        // Known advertising/promoted shapes anywhere inside one item drop that
        // item. Containers are not tested as a whole: an ad slot next to real
        // results must not hide its section.
        _ if crate::is_promoted(entry) => Ok(()),
        _ => match item(key, value, scope) {
            Parsed::Item(item) => chunk.push(item, request),
            Parsed::Filtered => Ok(()),
            Parsed::Unsupported => {
                chunk.partial = true;
                Ok(())
            }
        },
    }
}

fn map<T>(parsed: Parsed<T>, wrap: fn(T) -> CatalogItem) -> Parsed<CatalogItem> {
    match parsed {
        Parsed::Item(item) => Parsed::Item(wrap(item)),
        Parsed::Filtered => Parsed::Filtered,
        Parsed::Unsupported => Parsed::Unsupported,
    }
}

fn item(key: &str, value: &Value, scope: Scope) -> Parsed<CatalogItem> {
    let shorts_tab = matches!(
        scope,
        Scope::Channel {
            tab: ChannelTab::Shorts,
            ..
        }
    );
    let parsed = match key {
        "videoRenderer" | "gridVideoRenderer" => {
            map(renderers::video_renderer(value), CatalogItem::Video)
        }
        "playlistVideoRenderer" => {
            if value.get("isPlayable").and_then(Value::as_bool) == Some(false) {
                return Parsed::Unsupported;
            }
            map(renderers::video_renderer(value), CatalogItem::Video)
        }
        "lockupViewModel" => match value.get("contentType").and_then(Value::as_str) {
            Some("LOCKUP_CONTENT_TYPE_VIDEO") => {
                map(renderers::video_lockup(value), CatalogItem::Video)
            }
            Some("LOCKUP_CONTENT_TYPE_PLAYLIST") => {
                map(renderers::playlist_lockup(value), CatalogItem::Playlist)
            }
            _ => Parsed::Unsupported,
        },
        "channelRenderer" | "gridChannelRenderer" => {
            map(renderers::channel_renderer(value), CatalogItem::Channel)
        }
        "playlistRenderer" | "gridPlaylistRenderer" => {
            map(renderers::playlist_renderer(value), CatalogItem::Playlist)
        }
        // Shorts are listed only on a channel's Shorts tab.
        "shortsLockupViewModel" if shorts_tab => {
            map(renderers::short(value, false), CatalogItem::Video)
        }
        "reelItemRenderer" if shorts_tab => map(renderers::short(value, true), CatalogItem::Video),
        "shortsLockupViewModel" | "reelItemRenderer" => Parsed::Filtered,
        _ => Parsed::Unsupported,
    };
    match (parsed, scope) {
        (Parsed::Item(item), Scope::Channel { owner, .. }) => Parsed::Item(owned(item, owner)),
        (parsed, _) => parsed,
    }
}

/// Channel-tab lockups omit the byline (their first row is often the view
/// count), so the validated page owner names records that name no other channel.
fn owned(item: CatalogItem, owner: &ChannelSummary) -> CatalogItem {
    match item {
        CatalogItem::Video(mut video)
            if video.channel_id.as_ref().is_none_or(|id| *id == owner.id) =>
        {
            video.channel = owner.title.clone();
            video.channel_id = Some(owner.id.clone());
            CatalogItem::Video(video)
        }
        CatalogItem::Playlist(mut playlist)
            if playlist
                .channel_id
                .as_ref()
                .is_none_or(|id| *id == owner.id) =>
        {
            playlist.channel = Some(owner.title.clone());
            playlist.channel_id = Some(owner.id.clone());
            CatalogItem::Playlist(playlist)
        }
        item => item,
    }
}

struct First {
    request: CatalogRequest,
    header: CatalogHeader,
    chunk: Chunk,
}

fn first_page(
    transport: &GuestTransport,
    request: &CatalogRequest,
    operation: &OperationContext,
) -> Result<First, ProviderError> {
    match request {
        CatalogRequest::Search { query, kind } => {
            let mut payload = json!({"query": query.trim()});
            if let Some(params) = search_params(*kind) {
                payload["params"] = json!(params);
            }
            let value = transport.post("search", payload, operation)?;
            let chunk = search_chunk(&value, request)?;
            Ok(First {
                request: request.clone(),
                header: CatalogHeader::Search,
                chunk,
            })
        }
        CatalogRequest::Channel { id, tab } => channel_first(transport, id, *tab, operation),
        CatalogRequest::ChannelHandle { handle, tab } => {
            let id = resolve_handle(transport, handle, operation)?;
            channel_first(transport, &id, *tab, operation)
        }
        CatalogRequest::Playlist { id } => {
            let value = transport.post(
                "browse",
                json!({"browseId": format!("VL{}", id.as_str())}),
                operation,
            )?;
            let (header, chunk) = playlist_response(&value, id, request)?;
            Ok(First {
                request: request.clone(),
                header: CatalogHeader::Playlist(header),
                chunk,
            })
        }
    }
}

fn search_chunk(value: &Value, request: &CatalogRequest) -> Result<Chunk, ProviderError> {
    let list = value
        .pointer(
            "/contents/twoColumnSearchResultsRenderer/primaryContents/sectionListRenderer/contents",
        )
        .filter(|list| list.is_array())
        .ok_or(ProviderError::MalformedOutput)?;
    let mut chunk = Chunk::default();
    collect(Some(list), Scope::Search, request, &mut chunk, 0)?;
    Ok(chunk)
}

/// `@handle` → canonical channel ID. The handle is never followed afterwards.
fn resolve_handle(
    transport: &GuestTransport,
    handle: &ChannelHandle,
    operation: &OperationContext,
) -> Result<ChannelId, ProviderError> {
    let value = transport.post(
        "navigation/resolve_url",
        json!({"url": handle.browse_url()}),
        operation,
    )?;
    let endpoint = value
        .get("endpoint")
        .ok_or(ProviderError::MalformedOutput)?;
    if let Some(kind) = endpoint.pointer("/commandMetadata/webCommandMetadata/webPageType")
        && kind.as_str() != Some("WEB_PAGE_TYPE_CHANNEL")
    {
        return Err(ProviderError::MalformedOutput);
    }
    endpoint
        .pointer("/browseEndpoint/browseId")
        .and_then(Value::as_str)
        .and_then(|id| ChannelId::new(id).ok())
        .ok_or(ProviderError::MalformedOutput)
}

fn channel_first(
    transport: &GuestTransport,
    id: &ChannelId,
    tab: ChannelTab,
    operation: &OperationContext,
) -> Result<First, ProviderError> {
    let value = transport.post(
        "browse",
        json!({"browseId": id.as_str(), "params": tab_params(tab)}),
        operation,
    )?;
    let request = CatalogRequest::Channel {
        id: id.clone(),
        tab,
    };
    let (header, chunk) = channel_response(&value, id, tab, &request)?;
    Ok(First {
        request,
        header: CatalogHeader::Channel(header),
        chunk,
    })
}

fn channel_response(
    value: &Value,
    id: &ChannelId,
    tab: ChannelTab,
    request: &CatalogRequest,
) -> Result<(ChannelSummary, Chunk), ProviderError> {
    let header = channel_header(value, id)?;
    let tabs = value
        .pointer("/contents/twoColumnBrowseResultsRenderer/tabs")
        .and_then(Value::as_array)
        .ok_or(ProviderError::MalformedOutput)?;
    let path = |tab: &Value| {
        tab.pointer("/tabRenderer/endpoint/commandMetadata/webCommandMetadata/url")
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let suffix = tab_path(tab);
    let selected = tabs.iter().take(32).find(|tab| {
        tab.pointer("/tabRenderer/selected")
            .and_then(Value::as_bool)
            == Some(true)
    });
    let mut chunk = Chunk::default();
    match selected {
        Some(selected) if path(selected).is_some_and(|url| url.ends_with(suffix)) => {
            let content = selected
                .pointer("/tabRenderer/content")
                .ok_or(ProviderError::MalformedOutput)?;
            let list = content
                .pointer("/richGridRenderer/contents")
                .or_else(|| content.pointer("/sectionListRenderer/contents"))
                .filter(|list| list.is_array())
                .ok_or(ProviderError::MalformedOutput)?;
            let scope = Scope::Channel {
                owner: &header,
                tab,
            };
            collect(Some(list), scope, request, &mut chunk, 0)?;
        }
        // A channel without this tab is served its home tab instead: a real,
        // empty listing rather than an interpretation failure.
        _ if !tabs
            .iter()
            .take(32)
            .any(|tab| path(tab).is_some_and(|url| url.ends_with(suffix))) => {}
        _ => return Err(ProviderError::MalformedOutput),
    }
    Ok((header, chunk))
}

/// Header metadata for exactly the requested channel.
fn channel_header(value: &Value, id: &ChannelId) -> Result<ChannelSummary, ProviderError> {
    let metadata = value
        .pointer("/metadata/channelMetadataRenderer")
        .ok_or(ProviderError::MalformedOutput)?;
    if metadata.get("externalId").and_then(Value::as_str) != Some(id.as_str()) {
        return Err(ProviderError::MalformedOutput);
    }
    let title = text(metadata.get("title"));
    if title.trim().is_empty() {
        return Err(ProviderError::MalformedOutput);
    }
    let description = renderers::bounded_text(metadata.get("description"), 4000);
    let rows = value
        .pointer("/header/pageHeaderRenderer/content/pageHeaderViewModel/metadata/contentMetadataViewModel/metadataRows")
        .and_then(Value::as_array);
    let subscriber_count = rows
        .into_iter()
        .flatten()
        .take(8)
        .flat_map(|row| {
            row.get("metadataParts")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .take(8)
        })
        .find_map(|part| renderers::count_text(&text(part.get("text")), "subscriber"))
        .or_else(|| {
            renderers::count_text(
                &text(value.pointer("/header/c4TabbedHeaderRenderer/subscriberCountText")),
                "subscriber",
            )
        });
    Ok(ChannelSummary {
        id: id.clone(),
        title: title.chars().take(200).collect(),
        description: (!description.trim().is_empty()).then_some(description),
        thumbnail_url: renderers::avatar(metadata.pointer("/avatar/thumbnails")),
        subscriber_count,
    })
}

fn playlist_response(
    value: &Value,
    id: &PlaylistId,
    request: &CatalogRequest,
) -> Result<(PlaylistSummary, Chunk), ProviderError> {
    let Some(list) = value.pointer(
        "/contents/twoColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents",
    ) else {
        // Private, deleted or region-restricted playlists carry only an alert.
        let alert = value
            .get("alerts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(8)
            .any(|alert| {
                alert.pointer("/alertRenderer/type").and_then(Value::as_str) == Some("ERROR")
                    || alert.pointer("/alertWithButtonRenderer/type").and_then(Value::as_str)
                        == Some("ERROR")
            });
        return Err(if alert {
            ProviderError::Unavailable
        } else {
            ProviderError::MalformedOutput
        });
    };
    let header = playlist_header(value, id)?;
    let mut chunk = Chunk::default();
    collect(Some(list), Scope::Playlist(id), request, &mut chunk, 0)?;
    Ok((header, chunk))
}

fn playlist_header(value: &Value, id: &PlaylistId) -> Result<PlaylistSummary, ProviderError> {
    let canonical = value
        .pointer("/microformat/microformatDataRenderer/urlCanonical")
        .and_then(Value::as_str)
        .and_then(|url| url::Url::parse(url).ok())
        .and_then(|url| {
            url.query_pairs()
                .find(|(key, _)| key == "list")
                .map(|(_, list)| list.into_owned())
        })
        .or_else(|| {
            value
                .pointer("/header/playlistHeaderRenderer/playlistId")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
    if canonical.as_deref() != Some(id.as_str()) {
        return Err(ProviderError::MalformedOutput);
    }
    let sidebar: Vec<&Value> = value
        .pointer("/sidebar/playlistSidebarRenderer/items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(4)
        .collect();
    let primary = sidebar
        .iter()
        .find_map(|item| item.get("playlistSidebarPrimaryInfoRenderer"));
    let owner = sidebar.iter().find_map(|item| {
        item.pointer("/playlistSidebarSecondaryInfoRenderer/videoOwner/videoOwnerRenderer")
    });
    let legacy = value.pointer("/header/playlistHeaderRenderer");
    let title = Some(text(
        value.pointer("/metadata/playlistMetadataRenderer/title"),
    ))
    .filter(|title| !title.trim().is_empty())
    .or_else(|| Some(text(primary.and_then(|p| p.get("title")))))
    .filter(|title| !title.trim().is_empty())
    .ok_or(ProviderError::MalformedOutput)?;
    let description = renderers::bounded_text(
        value.pointer("/metadata/playlistMetadataRenderer/description"),
        4000,
    );
    let owner_text = owner
        .and_then(|owner| owner.get("title"))
        .or_else(|| legacy.and_then(|header| header.get("ownerText")));
    let channel_id = owner_text
        .and_then(|owner| owner.pointer("/runs/0/navigationEndpoint/browseEndpoint/browseId"))
        .or_else(|| {
            owner.and_then(|owner| owner.pointer("/navigationEndpoint/browseEndpoint/browseId"))
        })
        .and_then(Value::as_str)
        .and_then(|id| ChannelId::new(id).ok());
    let channel = text(owner_text);
    let video_count = primary
        .and_then(|p| p.get("stats"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(4)
        .find_map(|stat| renderers::count_text(&text(Some(stat)), "video"))
        .or_else(|| {
            renderers::count_text(
                &text(legacy.and_then(|header| header.get("numVideosText"))),
                "video",
            )
        });
    let art = primary.and_then(|p| p.get("thumbnailRenderer"));
    Ok(PlaylistSummary {
        id: id.clone(),
        title,
        description: (!description.trim().is_empty()).then_some(description),
        channel: (!channel.trim().is_empty()).then(|| channel.chars().take(200).collect()),
        channel_id,
        thumbnail_url: renderers::thumbnail(art.and_then(|art| {
            art.pointer("/playlistVideoThumbnailRenderer/thumbnail/thumbnails")
                .or_else(|| art.pointer("/playlistCustomThumbnailRenderer/thumbnail/thumbnails"))
        })),
        video_count,
    })
}

fn continuation_chunk(
    value: &Value,
    scope: Scope,
    request: &CatalogRequest,
) -> Result<Chunk, ProviderError> {
    let actions = value
        .get("onResponseReceivedActions")
        .or_else(|| value.get("onResponseReceivedCommands"));
    let Some(actions) = actions.and_then(Value::as_array) else {
        // The last playlist continuation is an otherwise empty response.
        let terminal = value.as_object().is_some_and(|map| {
            map.keys()
                .all(|key| matches!(key.as_str(), "responseContext" | "trackingParams"))
        });
        return if terminal {
            Ok(Chunk::default())
        } else {
            Err(ProviderError::MalformedOutput)
        };
    };
    let mut chunk = Chunk::default();
    let mut recognized = false;
    for action in actions.iter().take(8) {
        let items = action
            .pointer("/appendContinuationItemsAction/continuationItems")
            .or_else(|| action.pointer("/reloadContinuationItemsCommand/continuationItems"));
        if let Some(items) = items {
            recognized = true;
            collect(Some(items), scope, request, &mut chunk, 0)?;
        }
    }
    if !recognized {
        return Err(ProviderError::MalformedOutput);
    }
    Ok(chunk)
}

fn identity(item: &CatalogItem) -> (u8, String) {
    match item {
        CatalogItem::Video(video) => (0, video.id.as_str().to_owned()),
        CatalogItem::Channel(channel) => (1, channel.id.as_str().to_owned()),
        CatalogItem::Playlist(playlist) => (2, playlist.id.as_str().to_owned()),
    }
}

/// Appends records. Search results repeated across continuation responses are
/// shown once per page (earlier pages are not tracked); playlists may
/// legitimately contain one video several times, so browse lists are kept as-is.
fn append(buffer: &mut Vec<CatalogItem>, items: Vec<CatalogItem>, search: bool) {
    if !search {
        buffer.extend(items);
        return;
    }
    let mut seen: HashSet<_> = buffer.iter().map(identity).collect();
    buffer.extend(items.into_iter().filter(|item| seen.insert(identity(item))));
}

/// One native page. `cursor` must already be validated for this request and
/// session generation and must carry native state.
pub(crate) fn page(
    transport: &GuestTransport,
    request: &CatalogRequest,
    cursor: Option<&CatalogCursor>,
    operation: &OperationContext,
) -> Result<CatalogPage, ProviderError> {
    let (canonical, header, mut buffer, mut token, served, mut partial) = match cursor {
        Some(cursor) => {
            let state = cursor.native.as_ref().ok_or(ProviderError::InvalidInput)?;
            (
                cursor.request.clone(),
                state.header.clone(),
                state.buffer.to_vec(),
                state.token.as_deref().map(str::to_owned),
                cursor.offset,
                false,
            )
        }
        None => {
            let first = first_page(transport, request, operation)?;
            let mut items = Vec::new();
            let search = matches!(first.request, CatalogRequest::Search { .. });
            append(&mut items, first.chunk.items, search);
            (
                first.request,
                Arc::new(first.header),
                items,
                first.chunk.token,
                0,
                first.chunk.partial,
            )
        }
    };
    let search = matches!(canonical, CatalogRequest::Search { .. });
    let endpoint = if search { "search" } else { "browse" };
    let mut continuations = 0;
    // Browse lists also look ahead when exactly 20 rows are pending: YouTube
    // returns a token even at the end of a list (its reply is then empty), and
    // an empty final page behind a Next control would misrepresent the list.
    // Search results are practically unbounded, so no extra request there.
    let wanted = |pending: usize| pending < PAGE_SIZE || (!search && pending == PAGE_SIZE);
    while wanted(buffer.len()) && continuations < MAX_CONTINUATIONS_PER_PAGE {
        let Some(current) = token.take() else { break };
        continuations += 1;
        let result = transport
            .post(endpoint, json!({"continuation": current}), operation)
            .and_then(|value| {
                continuation_chunk(&value, Scope::of(&header, &canonical), &canonical)
            });
        match result {
            Ok(chunk) => {
                partial |= chunk.partial;
                append(&mut buffer, chunk.items, search);
                if buffer.len() > MAX_BUFFERED {
                    return Err(ProviderError::OutputTooLarge);
                }
                token = chunk.token;
            }
            Err(ProviderError::Cancelled) => return Err(ProviderError::Cancelled),
            // Records already received stay useful; the failed continuation is
            // retried by the next page (and falls back there if it persists).
            Err(error) if buffer.is_empty() => return Err(error),
            Err(_) => {
                token = Some(current);
                break;
            }
        }
    }
    if operation.cancel.is_cancelled() {
        return Err(ProviderError::Cancelled);
    }
    let rest = buffer.split_off(buffer.len().min(PAGE_SIZE));
    let items = buffer;
    if items.is_empty() && partial {
        // An entirely unsupported response is not a successful empty page.
        return Err(ProviderError::MalformedOutput);
    }
    let more = !rest.is_empty() || token.is_some();
    let served = served + items.len();
    let limit_reached = more && served >= canonical.limit();
    let next = (more && !limit_reached).then(|| CatalogCursor {
        request: canonical.clone(),
        offset: served,
        generation: operation.session_generation,
        native: Some(Continuation {
            token: token.map(Arc::from),
            buffer: rest.into(),
            header: header.clone(),
        }),
    });
    Ok(CatalogPage {
        header: (*header).clone(),
        items,
        next,
        partial,
        limit_reached,
        source: CatalogSource::Native,
    })
}

/// Public channel header metadata (avatar and subscriber count) for one channel.
pub(crate) fn channel_profile(
    transport: &GuestTransport,
    id: &ChannelId,
    operation: &OperationContext,
) -> Result<ChannelProfile, ProviderError> {
    let value = transport.post(
        "browse",
        json!({"browseId": id.as_str(), "params": tab_params(ChannelTab::Videos)}),
        operation,
    )?;
    let header = channel_header(&value, id)?;
    Ok(ChannelProfile {
        avatar_url: header.thumbnail_url,
        subscriber_count: header.subscriber_count,
    })
}

#[cfg(test)]
mod tests {
    //! TEST FIXTURES: every response below is a small synthetic JSON shape
    //! written for these tests after the reviewed live structure. They contain
    //! no real video, channel, playlist, token or session data.
    use super::*;
    use crate::YtDlp;
    use oxplay_core::CancellationToken;

    const OWNER: &str = "UCsyntheticchannel000001";
    const OTHER: &str = "UCsyntheticchannel000002";

    fn operation(generation: u64) -> OperationContext {
        OperationContext {
            request_id: 1,
            session_generation: generation,
            cancel: CancellationToken::default(),
        }
    }
    fn video_id(n: usize) -> String {
        format!("synthv{n:05}")
    }
    fn video(n: usize) -> Value {
        let id = video_id(n);
        json!({"videoRenderer": {
            "videoId": id,
            "navigationEndpoint": {"watchEndpoint": {"videoId": id}},
            "title": {"runs": [{"text": "Synthetic video "}, {"text": id}]},
            "ownerText": {"runs": [{"text": "Synthetic owner", "navigationEndpoint": {"browseEndpoint": {"browseId": OTHER}}}]},
            "lengthText": {"simpleText": "7:45"},
            "thumbnail": {"thumbnails": [{"url": "https://i.ytimg.com/vi/synthetic/hq720.jpg", "width": 720}]}
        }})
    }
    fn video_lockup(n: usize, first_row: &str) -> Value {
        let id = video_id(n);
        json!({"lockupViewModel": {
            "contentId": id,
            "contentType": "LOCKUP_CONTENT_TYPE_VIDEO",
            "rendererContext": {"commandContext": {"onTap": {"innertubeCommand": {"watchEndpoint": {"videoId": id}}}}},
            "contentImage": {"thumbnailViewModel": {
                "image": {"sources": [{"url": "https://i.ytimg.com/vi/synthetic/lockup.jpg", "width": 360}]},
                "overlays": [{"thumbnailBottomOverlayViewModel": {"badges": [{"thumbnailBadgeViewModel": {"text": "1:02:03"}}]}}]
            }},
            "metadata": {"lockupMetadataViewModel": {
                "title": {"content": "Synthetic lockup"},
                "metadata": {"contentMetadataViewModel": {"metadataRows": [
                    {"metadataParts": [{"text": {"content": first_row}}]}
                ]}}
            }}
        }})
    }
    fn playlist_lockup(id: &str) -> Value {
        json!({"lockupViewModel": {
            "contentId": id,
            "contentType": "LOCKUP_CONTENT_TYPE_PLAYLIST",
            "contentImage": {"collectionThumbnailViewModel": {"primaryThumbnail": {"thumbnailViewModel": {
                "image": {"sources": [{"url": "https://i.ytimg.com/vi/synthetic/playlist.jpg", "width": 480}]},
                "overlays": [{"thumbnailOverlayBadgeViewModel": {"thumbnailBadges": [{"thumbnailBadgeViewModel": {"text": "17 videos"}}]}}]
            }}}},
            "metadata": {"lockupMetadataViewModel": {
                "title": {"content": "Synthetic playlist"},
                "metadata": {"contentMetadataViewModel": {"metadataRows": [{"metadataParts": [
                    {"text": {"content": "Synthetic curator", "commandRuns": [{"onTap": {"innertubeCommand": {"browseEndpoint": {"browseId": OTHER}}}}]}},
                    {"text": {"content": "Playlist"}}
                ]}]}}
            }}
        }})
    }
    fn channel_renderer(id: &str) -> Value {
        json!({"channelRenderer": {
            "channelId": id,
            "navigationEndpoint": {"browseEndpoint": {"browseId": id}},
            "title": {"simpleText": "Synthetic channel"},
            "descriptionSnippet": {"runs": [{"text": "Synthetic description"}]},
            "thumbnail": {"thumbnails": [{"url": "//yt3.ggpht.com/synthetic=s176", "width": 176}]},
            "videoCountText": {"simpleText": "577K subscribers"},
            "subscriberCountText": {"simpleText": "@synthetic"}
        }})
    }
    fn continuation(token: &str) -> Value {
        json!({"continuationItemRenderer": {"continuationEndpoint": {"continuationCommand": {"token": token}}}})
    }
    fn search_response(items: Vec<Value>, token: Option<&str>) -> Value {
        let mut sections = vec![json!({"itemSectionRenderer": {"contents": items}})];
        sections.extend(token.map(continuation));
        json!({"responseContext": {}, "estimatedResults": "1",
            "contents": {"twoColumnSearchResultsRenderer": {"primaryContents": {"sectionListRenderer": {"contents": sections}}}}})
    }
    fn search_continuation(items: Vec<Value>, token: Option<&str>) -> Value {
        let mut sections = vec![json!({"itemSectionRenderer": {"contents": items}})];
        sections.extend(token.map(continuation));
        json!({"onResponseReceivedCommands": [{"appendContinuationItemsAction": {"continuationItems": sections}}]})
    }
    fn tabs(selected: &str, content: Value) -> Value {
        let tabs: Vec<Value> = ["featured", "videos", "shorts", "streams", "playlists"]
            .iter()
            .map(|name| {
                let mut tab = json!({"tabRenderer": {"endpoint": {"commandMetadata": {"webCommandMetadata": {"url": format!("/@synthetic/{name}")}}}}});
                if *name == selected {
                    tab["tabRenderer"]["selected"] = json!(true);
                    tab["tabRenderer"]["content"] = content.clone();
                }
                tab
            })
            .collect();
        json!({"twoColumnBrowseResultsRenderer": {"tabs": tabs}})
    }
    fn channel_response(id: &str, selected: &str, content: Value) -> Value {
        json!({
            "metadata": {"channelMetadataRenderer": {
                "externalId": id, "title": "Synthetic channel", "description": "Synthetic about text",
                "avatar": {"thumbnails": [{"url": "https://yt3.googleusercontent.com/synthetic=s900", "width": 900}]}
            }},
            "header": {"pageHeaderRenderer": {"content": {"pageHeaderViewModel": {
                "metadata": {"contentMetadataViewModel": {"metadataRows": [
                    {"metadataParts": [{"text": {"content": "@synthetic"}}]},
                    {"metadataParts": [{"text": {"content": "1.2M subscribers"}}, {"text": {"content": "284 videos"}}]}
                ]}},
                // An engagement panel continuation is never a listing cursor.
                "description": {"descriptionPreviewViewModel": {"rendererContext": {"commandContext": {"onTap": {"innertubeCommand": {
                    "showEngagementPanelEndpoint": {"engagementPanel": {"contents": [continuation("synthetic-about-panel-token")]}}}}}}}}
            }}}},
            "contents": tabs(selected, content)
        })
    }
    fn grid(items: Vec<Value>, token: Option<&str>) -> Value {
        let mut contents: Vec<Value> = items
            .into_iter()
            .map(|item| json!({"richItemRenderer": {"content": item}}))
            .collect();
        contents.extend(token.map(continuation));
        json!({"richGridRenderer": {"contents": contents}})
    }
    fn playlist_response(id: &str, items: Vec<Value>, token: Option<&str>) -> Value {
        let mut sections = vec![json!({"itemSectionRenderer": {"contents": items}})];
        sections.extend(token.map(|token| json!({"continuationItemViewModel": {"continuationCommand": {"innertubeCommand": {"continuationCommand": {"token": token}}}}})));
        json!({
            "microformat": {"microformatDataRenderer": {"urlCanonical": format!("http://www.youtube.com/playlist?list={id}")}},
            "metadata": {"playlistMetadataRenderer": {"title": "Synthetic playlist", "description": "Synthetic playlist description"}},
            "sidebar": {"playlistSidebarRenderer": {"items": [
                {"playlistSidebarPrimaryInfoRenderer": {"stats": [{"runs": [{"text": "1,024"}, {"text": " videos"}]}, {"simpleText": "9 views"}]}},
                {"playlistSidebarSecondaryInfoRenderer": {"videoOwner": {"videoOwnerRenderer": {
                    "title": {"runs": [{"text": "Synthetic owner", "navigationEndpoint": {"browseEndpoint": {"browseId": OWNER}}}]}}}}}
            ]}},
            "contents": {"twoColumnBrowseResultsRenderer": {"tabs": [{"tabRenderer": {"content": {"sectionListRenderer": {"contents": sections}}}}]}}
        })
    }
    fn search(kind: SearchKind) -> CatalogRequest {
        CatalogRequest::Search {
            query: "synthetic query".into(),
            kind,
        }
    }
    fn calls(transport: &GuestTransport) -> Vec<(String, Value)> {
        transport
            .fixture
            .as_ref()
            .unwrap()
            .calls
            .lock()
            .unwrap()
            .clone()
    }

    #[test]
    fn mixed_search_normalizes_known_kinds_and_filters_shelves_shorts_and_ads() {
        let mut badged = video(90);
        badged["videoRenderer"]["badges"] =
            json!([{"metadataBadgeRenderer": {"style": "BADGE_STYLE_TYPE_AD"}}]);
        let mut short = video(91);
        short["videoRenderer"]["navigationEndpoint"] =
            json!({"reelWatchEndpoint": {"videoId": video_id(91)}});
        let response = search_response(
            vec![
                playlist_lockup("PLsyntheticplaylist01"),
                video(1),
                channel_renderer(OWNER),
                json!({"playlistRenderer": {"playlistId": "PLsyntheticlegacy01", "title": {"simpleText": "Synthetic legacy"},
                    "videoCount": "3", "thumbnails": [{"thumbnails": [{"url": "https://i.ytimg.com/vi/synthetic/legacy.jpg"}]}],
                    "shortBylineText": {"runs": [{"text": "Synthetic owner", "navigationEndpoint": {"browseEndpoint": {"browseId": OWNER}}}]}}}),
                video_lockup(2, "Synthetic lockup owner"),
                json!({"gridShelfViewModel": {"contents": [video(80)]}}),
                json!({"reelShelfRenderer": {"items": [video(81)]}}),
                json!({"shortsLockupViewModel": {"onTap": {"innertubeCommand": {"reelWatchEndpoint": {"videoId": video_id(82)}}}}}),
                json!({"adSlotRenderer": {"synthetic": true}}),
                json!({"searchPyvRenderer": {"ads": [video(83)]}}),
                badged,
                short,
                playlist_lockup("RDsyntheticmix0001"),
            ],
            Some("synthetic-search-token"),
        );
        let request = search(SearchKind::All);
        let chunk = search_chunk(&response, &request).unwrap();
        assert!(!chunk.partial, "filtered shapes are deliberate omissions");
        assert_eq!(chunk.token.as_deref(), Some("synthetic-search-token"));
        let kinds: Vec<_> = chunk.items.iter().map(identity).collect();
        assert_eq!(
            kinds,
            [
                (2, "PLsyntheticplaylist01".to_owned()),
                (0, video_id(1)),
                (1, OWNER.to_owned()),
                (2, "PLsyntheticlegacy01".to_owned()),
                (0, video_id(2)),
            ]
        );
        let CatalogItem::Playlist(playlist) = &chunk.items[0] else {
            panic!("kind")
        };
        assert_eq!(playlist.video_count, Some(17));
        assert_eq!(playlist.channel.as_deref(), Some("Synthetic curator"));
        assert_eq!(
            playlist.channel_id.as_ref().map(ChannelId::as_str),
            Some(OTHER)
        );
        let CatalogItem::Video(video) = &chunk.items[1] else {
            panic!("kind")
        };
        assert_eq!(video.duration, Some(std::time::Duration::from_secs(465)));
        assert_eq!(
            video.channel_id.as_ref().map(ChannelId::as_str),
            Some(OTHER)
        );
        let CatalogItem::Channel(channel) = &chunk.items[2] else {
            panic!("kind")
        };
        assert_eq!(channel.subscriber_count, Some(577_000));
        assert_eq!(
            channel.thumbnail_url.as_deref(),
            Some("https://yt3.ggpht.com/synthetic=s176")
        );
        let CatalogItem::Playlist(legacy) = &chunk.items[3] else {
            panic!("kind")
        };
        assert_eq!(legacy.video_count, Some(3));
        let CatalogItem::Video(lockup) = &chunk.items[4] else {
            panic!("kind")
        };
        assert_eq!(lockup.duration, Some(std::time::Duration::from_secs(3723)));
        assert_eq!(lockup.channel, "Synthetic lockup owner");
    }

    #[test]
    fn search_filters_send_verified_params_and_unknown_kinds_mark_partial() {
        for (kind, params, rows) in [
            (SearchKind::All, None, 2),
            (SearchKind::Videos, Some("EgIQAQ=="), 1),
            (SearchKind::Channels, Some("EgIQAg=="), 1),
            (SearchKind::Playlists, Some("EgIQAw=="), 0),
        ] {
            let transport = GuestTransport::with_fixture(vec![Ok(search_response(
                vec![
                    video(1),
                    channel_renderer(OWNER),
                    json!({"movieRenderer": {"videoId": video_id(2)}}),
                ],
                None,
            ))]);
            let result = page(&transport, &search(kind), None, &operation(0));
            let call = &calls(&transport)[0];
            assert_eq!(call.0, "search");
            assert_eq!(call.1["query"], "synthetic query");
            assert_eq!(call.1["context"]["client"]["clientName"], "WEB");
            assert_eq!(call.1.get("params").and_then(Value::as_str), params);
            if rows == 0 {
                // Nothing acceptable: not a successful empty page.
                assert_eq!(result.err(), Some(ProviderError::MalformedOutput));
                continue;
            }
            let result = result.unwrap();
            // Unknown result kinds and kinds outside the filter are reported.
            assert!(result.partial);
            assert!(result.next.is_none());
            assert_eq!(result.items.len(), rows);
        }
        // A recognized "no results" notice is a real empty page.
        let transport = GuestTransport::with_fixture(vec![Ok(search_response(
            vec![json!({"backgroundPromoRenderer": {"title": {"simpleText": "No results"}}})],
            None,
        ))]);
        let empty = page(
            &transport,
            &search(SearchKind::Channels),
            None,
            &operation(0),
        )
        .unwrap();
        assert!(empty.items.is_empty() && !empty.partial && empty.next.is_none());
    }

    #[test]
    fn continuation_pages_fill_twenty_rows_buffer_the_rest_and_stop_at_the_end() {
        let first = search_response((0..15).map(video).collect(), Some("synthetic-a"));
        // One result repeated by the continuation is shown once.
        let second = search_continuation((14..25).map(video).collect(), Some("synthetic-b"));
        let last = search_continuation((25..28).map(video).collect(), None);
        let transport = GuestTransport::with_fixture(vec![Ok(first), Ok(second), Ok(last)]);
        let request = search(SearchKind::Videos);
        let one = page(&transport, &request, None, &operation(4)).unwrap();
        assert_eq!(one.items.len(), PAGE_SIZE);
        assert_eq!(one.source, CatalogSource::Native);
        let cursor = one.next.unwrap();
        assert_eq!(cursor.generation, 4);
        assert_eq!(cursor.offset, 20);
        let state = cursor.native.as_ref().unwrap();
        assert_eq!(state.buffer.len(), 5);
        assert_eq!(state.token.as_deref(), Some("synthetic-b"));
        let two = page(&transport, &request, Some(&cursor), &operation(4)).unwrap();
        let ids: Vec<_> = two.items.iter().map(|i| identity(i).1).collect();
        assert_eq!(ids, (20..28).map(video_id).collect::<Vec<_>>());
        assert!(two.next.is_none() && !two.limit_reached);
        let calls = calls(&transport);
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[1].1["continuation"], "synthetic-a");
        assert_eq!(calls[2].1["continuation"], "synthetic-b");
        assert!(calls[1].1.get("query").is_none());
    }

    #[test]
    fn buffered_pages_need_no_request_and_the_search_cap_is_explicit() {
        let transport = GuestTransport::with_fixture(vec![]);
        let request = search(SearchKind::All);
        let buffered = |offset| CatalogCursor {
            request: request.clone(),
            offset,
            generation: 0,
            native: Some(Continuation {
                token: Some(Arc::from("synthetic-more")),
                buffer: (0..25)
                    .map(
                        |n| match renderers::video_renderer(&video(n)["videoRenderer"]) {
                            Parsed::Item(video) => CatalogItem::Video(video),
                            _ => unreachable!(),
                        },
                    )
                    .collect(),
                header: Arc::new(CatalogHeader::Search),
            }),
        };
        let page_two = page(&transport, &request, Some(&buffered(20)), &operation(0)).unwrap();
        assert_eq!(page_two.items.len(), 20);
        assert!(page_two.next.is_some());
        let capped = page(&transport, &request, Some(&buffered(180)), &operation(0)).unwrap();
        assert!(capped.limit_reached);
        assert!(capped.next.is_none());
        assert!(
            calls(&transport).is_empty(),
            "served from the cursor buffer"
        );
    }

    #[test]
    fn channel_grid_uses_header_owner_real_metadata_and_only_grid_continuations() {
        let id = ChannelId::new(OWNER).unwrap();
        let request = CatalogRequest::Channel {
            id: id.clone(),
            tab: ChannelTab::Videos,
        };
        let response = channel_response(
            OWNER,
            "videos",
            grid(
                vec![
                    video_lockup(1, "29K views"),
                    video_lockup(2, "Synthetic collab"),
                    json!({"futureLockup": {}}),
                    json!({"richSectionRenderer": {"content": {"richShelfRenderer": {"contents": [continuation("synthetic-shelf-token")]}}}}),
                ],
                Some("synthetic-grid-token"),
            ),
        );
        let transport =
            GuestTransport::with_fixture(vec![Ok(response), Ok(json!({"responseContext": {}}))]);
        let result = page(&transport, &request, None, &operation(0)).unwrap();
        let calls = calls(&transport);
        assert_eq!(calls[0].1["browseId"], OWNER);
        assert_eq!(calls[0].1["params"], "EgZ2aWRlb3PyBgQKAjoA");
        // Only the grid token was followed; the about-panel and shelf tokens never were.
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1].1["continuation"], "synthetic-grid-token");
        let CatalogHeader::Channel(header) = &result.header else {
            panic!("header")
        };
        assert!(header.id == id);
        assert_eq!(header.title, "Synthetic channel");
        assert_eq!(header.description.as_deref(), Some("Synthetic about text"));
        assert_eq!(header.subscriber_count, Some(1_200_000));
        assert_eq!(
            header.thumbnail_url.as_deref(),
            Some("https://yt3.googleusercontent.com/synthetic=s900")
        );
        assert_eq!(result.items.len(), 2);
        assert!(result.partial, "unknown grid item is reported");
        for item in &result.items {
            let CatalogItem::Video(video) = item else {
                panic!("kind")
            };
            assert_eq!(video.channel, "Synthetic channel");
            assert!(video.channel_id.as_ref() == Some(&id));
            assert_eq!(video.duration, Some(std::time::Duration::from_secs(3723)));
        }
        assert!(result.next.is_none());
    }

    #[test]
    fn channel_identity_tab_selection_and_missing_tabs_fail_closed_or_empty() {
        let id = ChannelId::new(OWNER).unwrap();
        let request = |tab| CatalogRequest::Channel {
            id: id.clone(),
            tab,
        };
        let listing = || grid(vec![video_lockup(1, "x")], None);
        // A different channel's metadata is rejected.
        let transport =
            GuestTransport::with_fixture(vec![Ok(channel_response(OTHER, "videos", listing()))]);
        assert_eq!(
            page(
                &transport,
                &request(ChannelTab::Videos),
                None,
                &operation(0)
            )
            .err(),
            Some(ProviderError::MalformedOutput)
        );
        // Another existing tab selected instead of the requested one.
        let transport =
            GuestTransport::with_fixture(vec![Ok(channel_response(OWNER, "featured", listing()))]);
        assert_eq!(
            page(
                &transport,
                &request(ChannelTab::Videos),
                None,
                &operation(0)
            )
            .err(),
            Some(ProviderError::MalformedOutput)
        );
        // A channel without that tab at all is served its home tab: empty listing.
        let mut missing = channel_response(OWNER, "featured", listing());
        missing["contents"]["twoColumnBrowseResultsRenderer"]["tabs"]
            .as_array_mut()
            .unwrap()
            .retain(|tab| {
                tab.pointer("/tabRenderer/endpoint/commandMetadata/webCommandMetadata/url")
                    .and_then(Value::as_str)
                    != Some("/@synthetic/streams")
            });
        let transport = GuestTransport::with_fixture(vec![Ok(missing)]);
        let empty = page(
            &transport,
            &request(ChannelTab::Streams),
            None,
            &operation(0),
        )
        .unwrap();
        assert!(empty.items.is_empty() && !empty.partial && empty.next.is_none());
        assert!(matches!(empty.header, CatalogHeader::Channel(_)));
    }

    #[test]
    fn shorts_and_playlist_tabs_parse_their_own_shapes() {
        let id = ChannelId::new(OWNER).unwrap();
        let shorts = grid(
            vec![json!({"shortsLockupViewModel": {
                "onTap": {"innertubeCommand": {"reelWatchEndpoint": {"videoId": video_id(7)}}},
                "overlayMetadata": {"primaryText": {"content": "Synthetic short"}},
                "thumbnailViewModel": {"thumbnailViewModel": {"image": {"sources": [{"url": "https://i.ytimg.com/vi/synthetic/sardefault.jpg", "width": 405}]}}}
            }})],
            None,
        );
        let transport =
            GuestTransport::with_fixture(vec![Ok(channel_response(OWNER, "shorts", shorts))]);
        let result = page(
            &transport,
            &CatalogRequest::Channel {
                id: id.clone(),
                tab: ChannelTab::Shorts,
            },
            None,
            &operation(0),
        )
        .unwrap();
        assert_eq!(calls(&transport)[0].1["params"], "EgZzaG9ydHPyBgUKA5oBAA==");
        let [CatalogItem::Video(short)] = result.items.as_slice() else {
            panic!("one short")
        };
        assert_eq!(short.title, "Synthetic short");
        assert!(short.channel_id.as_ref() == Some(&id));
        assert!(short.thumbnail_url.is_some());
        let playlists = json!({"sectionListRenderer": {"contents": [{"itemSectionRenderer": {"contents": [
            {"gridRenderer": {"items": [playlist_lockup("PLsyntheticplaylist02"), video_lockup(3, "x")]}}
        ]}}]}});
        let transport =
            GuestTransport::with_fixture(vec![Ok(channel_response(OWNER, "playlists", playlists))]);
        let result = page(
            &transport,
            &CatalogRequest::Channel {
                id: id.clone(),
                tab: ChannelTab::Playlists,
            },
            None,
            &operation(0),
        )
        .unwrap();
        assert_eq!(result.items.len(), 1);
        assert!(
            result.partial,
            "a video in the playlists tab is not accepted"
        );
        let CatalogItem::Playlist(playlist) = &result.items[0] else {
            panic!("kind")
        };
        // A byline naming another validated channel is kept.
        assert_eq!(
            playlist.channel_id.as_ref().map(ChannelId::as_str),
            Some(OTHER)
        );
    }

    #[test]
    fn handles_resolve_once_and_cursors_pin_the_canonical_channel() {
        let resolved = json!({"endpoint": {
            "commandMetadata": {"webCommandMetadata": {"webPageType": "WEB_PAGE_TYPE_CHANNEL"}},
            "browseEndpoint": {"browseId": OWNER}}});
        let transport = GuestTransport::with_fixture(vec![
            Ok(resolved),
            Ok(channel_response(
                OWNER,
                "videos",
                grid((0..25).map(|n| video_lockup(n, "x")).collect(), None),
            )),
        ]);
        let request = CatalogRequest::ChannelHandle {
            handle: ChannelHandle::new("@synthetic").unwrap(),
            tab: ChannelTab::Videos,
        };
        let result = page(&transport, &request, None, &operation(3)).unwrap();
        let calls = calls(&transport);
        assert_eq!(calls[0].0, "navigation/resolve_url");
        assert_eq!(calls[0].1["url"], "https://www.youtube.com/@synthetic");
        assert_eq!(calls[1].1["browseId"], OWNER);
        let cursor = result.next.unwrap();
        assert!(
            matches!(&cursor.request, CatalogRequest::Channel { id, tab: ChannelTab::Videos } if id.as_str() == OWNER)
        );
        // Non-channel resolution is not followed.
        let transport = GuestTransport::with_fixture(vec![Ok(json!({"endpoint": {
            "commandMetadata": {"webCommandMetadata": {"webPageType": "WEB_PAGE_TYPE_WATCH"}},
            "browseEndpoint": {"browseId": OWNER}}}))]);
        assert_eq!(
            page(&transport, &request, None, &operation(3)).err(),
            Some(ProviderError::MalformedOutput)
        );
    }

    #[test]
    fn playlist_pages_read_header_items_and_view_model_continuations() {
        let id = PlaylistId::new("PLsyntheticplaylist03").unwrap();
        let request = CatalogRequest::Playlist { id: id.clone() };
        let mut unplayable = video(50);
        unplayable["videoRenderer"]["isPlayable"] = json!(false);
        let legacy = json!({"playlistVideoListRenderer": {"playlistId": id.as_str(), "contents": [
            {"playlistVideoRenderer": video(40)["videoRenderer"].clone()},
            {"playlistVideoRenderer": unplayable["videoRenderer"].clone()},
        ]}});
        let mut items: Vec<Value> = (0..18)
            .map(|n| video_lockup(n, "Synthetic uploader"))
            .collect();
        // A video listed twice is a real playlist entry and stays.
        items.push(video_lockup(0, "Synthetic uploader"));
        items.push(legacy);
        let transport = GuestTransport::with_fixture(vec![
            Ok(playlist_response(
                id.as_str(),
                items,
                Some("synthetic-list-token"),
            )),
            // Terminal continuation: an otherwise empty response.
            Ok(json!({"responseContext": {}, "trackingParams": "synthetic"})),
        ]);
        let one = page(&transport, &request, None, &operation(0)).unwrap();
        let calls = calls(&transport);
        assert_eq!(calls[0].1["browseId"], "VLPLsyntheticplaylist03");
        // Exactly 20 pending rows: one lookahead finds the end of the list.
        assert_eq!(calls[1].1["continuation"], "synthetic-list-token");
        let CatalogHeader::Playlist(header) = &one.header else {
            panic!("header")
        };
        assert_eq!(header.title, "Synthetic playlist");
        assert_eq!(
            header.description.as_deref(),
            Some("Synthetic playlist description")
        );
        assert_eq!(header.video_count, Some(1_024));
        assert_eq!(header.channel.as_deref(), Some("Synthetic owner"));
        assert_eq!(
            header.channel_id.as_ref().map(ChannelId::as_str),
            Some(OWNER)
        );
        assert_eq!(one.items.len(), 20);
        assert!(one.partial, "the unplayable entry is reported");
        assert!(one.next.is_none(), "no empty final page");
        // Identity, and restricted playlists.
        let transport = GuestTransport::with_fixture(vec![Ok(playlist_response(
            "PLsyntheticotherlist",
            vec![video_lockup(1, "x")],
            None,
        ))]);
        assert_eq!(
            page(&transport, &request, None, &operation(0)).err(),
            Some(ProviderError::MalformedOutput)
        );
        let transport = GuestTransport::with_fixture(vec![Ok(json!({"alerts": [
            {"alertRenderer": {"type": "ERROR", "text": {"simpleText": "Synthetic unavailable"}}}]}))]);
        assert_eq!(
            page(&transport, &request, None, &operation(0)).err(),
            Some(ProviderError::Unavailable)
        );
    }

    #[test]
    fn malformed_shapes_tokens_ids_and_art_are_rejected_or_partial() {
        let request = search(SearchKind::All);
        for response in [
            json!({"contents": {"futureResultsRenderer": {}}}),
            json!({"contents": {"twoColumnSearchResultsRenderer": {"primaryContents": {"sectionListRenderer": {"contents": {}}}}}}),
            search_response(
                vec![video(1), continuation("synthetic-one")],
                Some("synthetic-two"),
            ),
            search_response(vec![video(1)], Some("synthetic\ncontrol")),
            search_response(vec![video(1)], Some("")),
        ] {
            let transport = GuestTransport::with_fixture(vec![Ok(response)]);
            assert_eq!(
                page(&transport, &request, None, &operation(0)).err(),
                Some(ProviderError::MalformedOutput)
            );
        }
        let mut unsafe_art = video(2);
        unsafe_art["videoRenderer"]["thumbnail"] = json!({"thumbnails": [
            {"url": "https://i.ytimg.com.attacker.invalid/vi/x.jpg", "width": 480}]});
        let mut mismatched = video(3);
        mismatched["videoRenderer"]["navigationEndpoint"] =
            json!({"watchEndpoint": {"videoId": video_id(4)}});
        let mut ytimg_avatar = channel_renderer(OWNER);
        ytimg_avatar["channelRenderer"]["thumbnail"] =
            json!({"thumbnails": [{"url": "https://i.ytimg.com/vi/synthetic/avatar.jpg"}]});
        let transport = GuestTransport::with_fixture(vec![Ok(search_response(
            vec![
                unsafe_art,
                mismatched,
                json!({"lockupViewModel": {"contentType": "LOCKUP_CONTENT_TYPE_VIDEO", "contentId": "bad/id?x=1"}}),
                channel_renderer("UCshort"),
                ytimg_avatar,
                json!({"playlistRenderer": {"playlistId": "bad list", "title": {"simpleText": "x"}}}),
                json!("not an object"),
                json!({"richItemRenderer": {}}),
            ],
            None,
        ))]);
        let result = page(&transport, &request, None, &operation(0)).unwrap();
        assert!(result.partial);
        assert_eq!(result.items.len(), 2);
        let CatalogItem::Video(first) = &result.items[0] else {
            panic!("kind")
        };
        assert!(first.thumbnail_url.is_none(), "look-alike host rejected");
        let CatalogItem::Channel(channel) = &result.items[1] else {
            panic!("kind")
        };
        assert!(
            channel.thumbnail_url.is_none(),
            "avatars only from portrait hosts"
        );
        // Oversized responses are bounded.
        let many = search_response((0..=MAX_ITEMS_PER_RESPONSE).map(video).collect(), None);
        let transport = GuestTransport::with_fixture(vec![Ok(many)]);
        assert_eq!(
            page(&transport, &request, None, &operation(0)).err(),
            Some(ProviderError::OutputTooLarge)
        );
    }

    #[test]
    fn a_failed_continuation_keeps_received_rows_and_its_token() {
        let transport = GuestTransport::with_fixture(vec![
            Ok(search_response(
                (0..5).map(video).collect(),
                Some("synthetic-retry"),
            )),
            Err(ProviderError::Timeout),
        ]);
        let result = page(&transport, &search(SearchKind::All), None, &operation(0)).unwrap();
        assert_eq!(result.items.len(), 5);
        let cursor = result.next.unwrap();
        assert_eq!(
            cursor.native.as_ref().unwrap().token.as_deref(),
            Some("synthetic-retry")
        );
    }

    fn provider(replies: Vec<Result<Value, ProviderError>>) -> YtDlp {
        YtDlp::new("/synthetic/nonexistent-helper")
            .unwrap()
            .with_guest_transport(GuestTransport::with_fixture(replies))
    }
    fn extractor_attempted<T>(result: Result<T, ProviderError>) -> bool {
        // The synthetic helper cannot start, so reaching it is observable.
        matches!(
            result.err(),
            Some(ProviderError::HelperUnavailable | ProviderError::UnsupportedPlatform)
        )
    }

    #[test]
    fn fallback_to_the_extractor_happens_only_for_unsupported_response_classes() {
        let request = search(SearchKind::All);
        for error in [
            ProviderError::MalformedOutput,
            ProviderError::ExtractorFailed,
            ProviderError::OutputTooLarge,
            ProviderError::UnsupportedFormat,
            ProviderError::ProofRequired,
        ] {
            let provider = provider(vec![Err(error)]);
            assert!(
                extractor_attempted(provider.catalog(&request, None, &operation(0))),
                "{error:?} falls back"
            );
        }
        // A syntactically valid but unrecognized response also falls back.
        let provider = self::provider(vec![Ok(json!({"responseContext": {}}))]);
        assert!(extractor_attempted(provider.catalog(
            &request,
            None,
            &operation(0)
        )));
        for error in [
            ProviderError::Cancelled,
            ProviderError::RateLimited,
            ProviderError::Offline,
            ProviderError::Timeout,
            ProviderError::InvalidInput,
            ProviderError::Unavailable,
        ] {
            let provider = self::provider(vec![Err(error)]);
            assert_eq!(
                provider.catalog(&request, None, &operation(0)).err(),
                Some(error),
                "{error:?} is returned without an extractor attempt"
            );
        }
        // Success never touches the extractor.
        let provider = self::provider(vec![Ok(search_response(vec![video(1)], None))]);
        let result = provider.catalog(&request, None, &operation(0)).unwrap();
        assert_eq!(result.source, CatalogSource::Native);
    }

    #[test]
    fn extractor_cursors_stay_on_the_extractor_and_foreign_cursors_fail_first() {
        let request = search(SearchKind::All);
        // No fixture reply is queued: any native request would panic.
        let provider = provider(vec![]);
        let extractor_cursor = CatalogCursor {
            request: request.clone(),
            offset: 20,
            generation: 0,
            native: None,
        };
        assert!(extractor_attempted(provider.catalog(
            &request,
            Some(&extractor_cursor),
            &operation(0)
        )));
        assert_eq!(
            provider
                .catalog(&request, Some(&extractor_cursor), &operation(1))
                .err(),
            Some(ProviderError::InvalidInput)
        );
        assert_eq!(
            provider
                .catalog(
                    &search(SearchKind::Videos),
                    Some(&extractor_cursor),
                    &operation(0)
                )
                .err(),
            Some(ProviderError::InvalidInput)
        );
    }

    #[test]
    fn compatibility_search_is_videos_only_and_channel_profile_is_native_first() {
        let provider = provider(vec![
            Ok(search_response((0..21).map(video).collect(), None)),
            Ok(channel_response(OWNER, "videos", grid(vec![], None))),
            Err(ProviderError::MalformedOutput),
            Err(ProviderError::RateLimited),
        ]);
        let result = provider
            .search(" synthetic query ", None, &operation(0))
            .unwrap();
        assert_eq!(result.videos.len(), 20);
        let cursor = result.next.unwrap();
        assert!(matches!(
            provider.search("another query", Some(&cursor), &operation(0)),
            Err(ProviderError::InvalidInput)
        ));
        let id = ChannelId::new(OWNER).unwrap();
        let profile = provider.channel_profile(&id, &operation(0)).unwrap();
        assert_eq!(profile.subscriber_count, Some(1_200_000));
        assert!(profile.avatar_url.is_some());
        assert!(extractor_attempted(
            provider.channel_profile(&id, &operation(0))
        ));
        assert!(matches!(
            provider.channel_profile(&id, &operation(0)),
            Err(ProviderError::RateLimited)
        ));
        let calls = calls(provider.guest().unwrap());
        assert_eq!(calls[0].1["params"], "EgIQAQ==");
        assert_eq!(calls[1].1["params"], "EgZ2aWRlb3PyBgQKAjoA");
    }
}
