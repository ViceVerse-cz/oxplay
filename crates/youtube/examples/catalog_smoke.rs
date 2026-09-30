// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit public-network check. Prints structural counts only, never titles/queries/URLs.
use oxplay_core::{CancellationToken, CatalogItem, OperationContext};
use oxplay_youtube::{
    YtDlp,
    catalog::{CatalogRequest, ChannelTab, SearchKind},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        return Err(
            "usage: catalog_smoke /absolute/yt-dlp /absolute/deno public-search-query".into(),
        );
    }
    let provider = YtDlp::new(&args[0])?.with_deno(&args[1])?;
    let operation = OperationContext {
        request_id: 1,
        session_generation: 0,
        cancel: CancellationToken::default(),
    };
    for kind in [SearchKind::Channels, SearchKind::Playlists, SearchKind::All] {
        let request = CatalogRequest::Search {
            query: args[2].clone(),
            kind,
        };
        let page = provider.catalog(&request, None, &operation)?;
        let videos = page
            .items
            .iter()
            .filter(|v| matches!(v, CatalogItem::Video(_)))
            .count();
        let channels = page
            .items
            .iter()
            .filter(|v| matches!(v, CatalogItem::Channel(_)))
            .count();
        let playlists = page
            .items
            .iter()
            .filter(|v| matches!(v, CatalogItem::Playlist(_)))
            .count();
        println!(
            "search_kind={} videos={videos} channels={channels} playlists={playlists} partial={} more={}",
            match kind {
                SearchKind::Channels => "channels",
                SearchKind::Playlists => "playlists",
                _ => "all",
            },
            page.partial,
            page.next.is_some()
        );
        if let Some(cursor) = page.next {
            let second = provider.catalog(&request, Some(&cursor), &operation)?;
            println!(
                "second_page_items={} partial={}",
                second.items.len(),
                second.partial
            );
        }
        let request = match page.items.into_iter().next() {
            Some(CatalogItem::Channel(channel)) => Some(CatalogRequest::Channel {
                id: channel.id,
                tab: ChannelTab::Videos,
            }),
            Some(CatalogItem::Playlist(playlist)) => {
                Some(CatalogRequest::Playlist { id: playlist.id })
            }
            _ => None,
        };
        if let Some(request) = request {
            let page = provider.catalog(&request, None, &operation)?;
            println!(
                "browse_items={} partial={} more={}",
                page.items.len(),
                page.partial,
                page.next.is_some()
            );
        }
    }
    Ok(())
}
