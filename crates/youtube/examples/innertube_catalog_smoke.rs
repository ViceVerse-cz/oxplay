// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit public-network check of the native guest catalog against the
//! supervised yt-dlp listing. Prints structural counts and timings only, never
//! titles, URLs, tokens or provider JSON. No cookies or account session.
//!
//! usage: innertube_catalog_smoke /abs/yt-dlp /abs/deno 'search query' @handle [PLAYLIST_ID]
use oxplay_core::{CancellationToken, CatalogItem, ChannelHandle, OperationContext, PlaylistId};
use oxplay_youtube::{
    YtDlp,
    catalog::{CatalogCursor, CatalogHeader, CatalogPage, CatalogRequest, ChannelTab, SearchKind},
};
use std::time::Instant;

type Fetch<'a> = dyn Fn(&CatalogRequest, Option<&CatalogCursor>) -> Result<CatalogPage, oxplay_core::ProviderError>
    + 'a;

fn report(label: &str, fetch: &Fetch, request: &CatalogRequest) -> Option<CatalogPage> {
    let started = Instant::now();
    let page = match fetch(request, None) {
        Ok(page) => page,
        Err(error) => {
            println!(
                "{label} error={error:?} ms={}",
                started.elapsed().as_millis()
            );
            return None;
        }
    };
    let first_ms = started.elapsed().as_millis();
    let count = |f: fn(&CatalogItem) -> bool| page.items.iter().filter(|i| f(i)).count();
    let durations = page
        .items
        .iter()
        .filter(|i| matches!(i, CatalogItem::Video(v) if v.duration.is_some()))
        .count();
    let thumbnails = page
        .items
        .iter()
        .filter(|i| match i {
            CatalogItem::Video(v) => v.thumbnail_url.is_some(),
            CatalogItem::Channel(c) => c.thumbnail_url.is_some(),
            CatalogItem::Playlist(p) => p.thumbnail_url.is_some(),
        })
        .count();
    let header = match &page.header {
        CatalogHeader::Search => "search".to_owned(),
        CatalogHeader::Channel(c) => format!(
            "channel(subscribers={} avatar={} description={})",
            c.subscriber_count.is_some(),
            c.thumbnail_url.is_some(),
            c.description.is_some()
        ),
        CatalogHeader::Playlist(p) => format!(
            "playlist(owner={} owner_id={} count={:?})",
            p.channel.is_some(),
            p.channel_id.is_some(),
            p.video_count
        ),
    };
    let mut line = format!(
        "{label} source={:?} ms={first_ms} items={} videos={} channels={} playlists={} durations={durations} thumbnails={thumbnails} partial={} more={} header={header}",
        page.source,
        page.items.len(),
        count(|i| matches!(i, CatalogItem::Video(_))),
        count(|i| matches!(i, CatalogItem::Channel(_))),
        count(|i| matches!(i, CatalogItem::Playlist(_))),
        page.partial,
        page.next.is_some(),
    );
    if let Some(cursor) = &page.next {
        let canonical = match (&page.header, request) {
            (CatalogHeader::Channel(c), CatalogRequest::ChannelHandle { tab, .. }) => {
                CatalogRequest::Channel {
                    id: c.id.clone(),
                    tab: *tab,
                }
            }
            _ => request.clone(),
        };
        let started = Instant::now();
        match fetch(&canonical, Some(cursor)) {
            Ok(second) => {
                let first: Vec<_> = page.items.iter().map(id).collect();
                let overlap = second
                    .items
                    .iter()
                    .filter(|i| first.contains(&id(i)))
                    .count();
                line += &format!(
                    " | page2 source={:?} ms={} items={} overlap={overlap} partial={} more={}",
                    second.source,
                    started.elapsed().as_millis(),
                    second.items.len(),
                    second.partial,
                    second.next.is_some()
                );
            }
            Err(error) => {
                line += &format!(
                    " | page2 error={error:?} ms={}",
                    started.elapsed().as_millis()
                )
            }
        }
    }
    println!("{line}");
    Some(page)
}

fn id(item: &CatalogItem) -> String {
    match item {
        CatalogItem::Video(v) => v.id.as_str().to_owned(),
        CatalogItem::Channel(c) => c.id.as_str().to_owned(),
        CatalogItem::Playlist(p) => p.id.as_str().to_owned(),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !(4..=5).contains(&args.len()) {
        return Err(
            "usage: innertube_catalog_smoke /abs/yt-dlp /abs/deno 'query' @handle [PLAYLIST_ID]"
                .into(),
        );
    }
    let provider = YtDlp::new(&args[0])?.with_deno(&args[1])?;
    let operation = OperationContext {
        request_id: 1,
        session_generation: 0,
        cancel: CancellationToken::default(),
    };
    let native = |r: &CatalogRequest, c: Option<&CatalogCursor>| provider.catalog(r, c, &operation);
    let extractor = |r: &CatalogRequest, c: Option<&CatalogCursor>| {
        provider.catalog_with_extractor(r, c, &operation)
    };
    let handle = ChannelHandle::new(&args[3])?;
    let search = CatalogRequest::Search {
        query: args[2].clone(),
        kind: SearchKind::All,
    };
    let channel = CatalogRequest::ChannelHandle {
        handle,
        tab: ChannelTab::Videos,
    };
    report("native search_all", &native, &search);
    for (label, kind) in [
        ("native search_videos", SearchKind::Videos),
        ("native search_channels", SearchKind::Channels),
        ("native search_playlists", SearchKind::Playlists),
    ] {
        let request = CatalogRequest::Search {
            query: args[2].clone(),
            kind,
        };
        report(label, &native, &request);
    }
    let channel_page = report("native channel_handle_videos", &native, &channel);
    let mut playlist = args.get(4).map(|id| PlaylistId::new(id)).transpose()?;
    if let Some(CatalogHeader::Channel(owner)) = channel_page.as_ref().map(|p| &p.header) {
        let started = Instant::now();
        let profile = provider.channel_profile(&owner.id, &operation);
        println!(
            "native channel_profile ms={} avatar={} subscribers={}",
            started.elapsed().as_millis(),
            profile.as_ref().is_ok_and(|p| p.avatar_url.is_some()),
            profile.as_ref().is_ok_and(|p| p.subscriber_count.is_some()),
        );
        for (label, tab) in [
            ("native channel_shorts", ChannelTab::Shorts),
            ("native channel_streams", ChannelTab::Streams),
            ("native channel_playlists", ChannelTab::Playlists),
        ] {
            let request = CatalogRequest::Channel {
                id: owner.id.clone(),
                tab,
            };
            let page = report(label, &native, &request);
            if playlist.is_none()
                && let Some(CatalogItem::Playlist(p)) =
                    page.and_then(|p| p.items.into_iter().next())
            {
                playlist = Some(p.id);
            }
        }
    }
    let playlist = playlist.map(|id| CatalogRequest::Playlist { id });
    if let Some(playlist) = &playlist {
        report("native playlist", &native, playlist);
    }
    report("extractor search_all", &extractor, &search);
    report("extractor channel_handle_videos", &extractor, &channel);
    if let Some(playlist) = &playlist {
        report("extractor playlist", &extractor, playlist);
    }
    Ok(())
}
