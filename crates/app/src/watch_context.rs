// SPDX-License-Identifier: GPL-3.0-or-later
//! One bounded guest-only related page with ownership independent of browsing.
//! Account/private metadata never enters this model; the selected video's other
//! fields remain owned by the existing playback/details adapters.
use crate::{App, UiState, VideoRow, model::CatalogModel};
use serein_core::{CatalogItem, VideoId, VideoSummary};
use slint::Model;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

const MAX_RELATED_ROWS: usize = 20;
#[derive(Default)]
pub struct State {
    pub model: Rc<CatalogModel<VideoRow>>,
    items: RefCell<Vec<CatalogItem>>,
    local_only: Cell<bool>,
}
impl State {
    pub fn item(&self, row: usize) -> Option<CatalogItem> {
        self.items.borrow().get(row).cloned()
    }
    pub fn video_summary(&self, id: &VideoId) -> Option<VideoSummary> {
        self.items.borrow().iter().find_map(|item| match item {
            CatalogItem::Video(video) if &video.id == id => Some(video.clone()),
            _ => None,
        })
    }
    pub fn thumbnail_source(&self, row: usize) -> Option<crate::thumbnails::Source> {
        crate::guest_ui::thumbnail_source(self.items.borrow().get(row)?, self.local_only.get())
    }
}

/// Call before a new guest selection clears its previous watch presentation.
/// Related selection retains the exact visible page, including its typed indices;
/// a selection from browsing captures only its currently acknowledged guest page.
pub fn capture_guest(app: &App, state: &UiState, selected: &VideoId) {
    if app.get_page() == 2 && state.watch_context.video_summary(selected).is_some() {
        return;
    }
    let (snapshot, local_only) = state.guest_ui.watch_snapshot();
    // Keep each candidate's browsing index so a retained row (and its decoded
    // image) is reused only when it still names the same item.
    let candidates = related_candidates(snapshot);
    let rows = candidates
        .iter()
        .take(MAX_RELATED_ROWS)
        .map(|(source_index, item)| {
            state
                .model
                .row_data(*source_index)
                .filter(|row| row.id.as_str() == item_id(item))
                .unwrap_or_else(|| crate::guest_ui::row(item))
        })
        .collect();
    let items: Vec<CatalogItem> = candidates
        .into_iter()
        .take(MAX_RELATED_ROWS)
        .map(|(_, item)| item)
        .collect();
    // Retire old row-index jobs before publishing a replacement dataset.
    state.thumbnails.borrow_mut().replace(Vec::new());
    state.thumbnail_attempted.borrow_mut().clear();
    *state.watch_context.items.borrow_mut() = items;
    state.watch_context.local_only.set(local_only);
    crate::feed_focus::reset(app, state);
    state.watch_context.model.replace(rows);
    state.thumbnail_range.set((usize::MAX, usize::MAX));
}
/// The watch page recommends videos and playlists. Channel rows (including the
/// creator's own channel, which already has a dedicated action beside the title)
/// never enter the related list. The returned pairs keep each item's index in
/// the source page; the surviving order defines related row indices, which is
/// also what `item()`/select-related and feed focus use.
fn related_candidates(items: Vec<CatalogItem>) -> Vec<(usize, CatalogItem)> {
    items
        .into_iter()
        .enumerate()
        .filter(|(_, item)| !matches!(item, CatalogItem::Channel(_)))
        .collect()
}
fn item_id(item: &CatalogItem) -> &str {
    match item {
        CatalogItem::Video(video) => video.id.as_str(),
        CatalogItem::Channel(channel) => channel.id.as_str(),
        CatalogItem::Playlist(playlist) => playlist.id.as_str(),
    }
}
/// Use for account/local selections and explicit local-data clearing. No account
/// teardown is required here because authenticated results are never captured.
pub fn clear(app: &App, state: &UiState) {
    if app.get_page() == 2 {
        state.thumbnails.borrow_mut().replace(Vec::new());
        state.thumbnail_attempted.borrow_mut().clear();
    }
    state.thumbnail_retained.borrow_mut().clear();
    state.watch_context.items.borrow_mut().clear();
    state.watch_context.local_only.set(false);
    if state.watch_context.model.row_count() != 0 {
        crate::feed_focus::reset(app, state);
        state.watch_context.model.replace(Vec::new());
    }
    state.thumbnail_range.set((usize::MAX, usize::MAX));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serein_core::{ChannelId, ChannelSummary, PlaylistId, PlaylistSummary};

    fn video(n: u8) -> CatalogItem {
        CatalogItem::Video(VideoSummary {
            id: VideoId::new(&format!("{n:011}")).unwrap(),
            title: format!("video {n}"),
            channel: "Creator".into(),
            channel_id: Some(ChannelId::new("UC0123456789012345678901").unwrap()),
            duration: None,
            thumbnail_url: None,
        })
    }
    fn channel() -> CatalogItem {
        CatalogItem::Channel(ChannelSummary {
            id: ChannelId::new("UC0123456789012345678901").unwrap(),
            title: "Creator".into(),
            description: None,
            thumbnail_url: None,
            subscriber_count: None,
        })
    }
    fn playlist() -> CatalogItem {
        CatalogItem::Playlist(PlaylistSummary {
            id: PlaylistId::new("PL0123456789012345678901234567890AB").unwrap(),
            title: "Mix".into(),
            description: None,
            channel: None,
            channel_id: None,
            thumbnail_url: None,
            video_count: None,
        })
    }

    #[test]
    fn related_list_drops_channel_rows_and_keeps_typed_indices_consistent() {
        // Search-like page: channel first, then videos, another channel, a playlist.
        let page = vec![
            channel(),
            video(1),
            video(2),
            channel(),
            playlist(),
            video(3),
        ];
        let kept = related_candidates(page);
        assert_eq!(
            kept.iter().map(|(source, _)| *source).collect::<Vec<_>>(),
            [1, 2, 4, 5]
        );
        assert!(
            kept.iter()
                .all(|(_, item)| !matches!(item, CatalogItem::Channel(_)))
        );
        // Related indices are positions in the surviving list, and the row a
        // card displays is the same item select-related will resolve.
        let state = State::default();
        *state.items.borrow_mut() = kept.into_iter().map(|(_, item)| item).collect();
        assert!(matches!(state.item(0), Some(CatalogItem::Video(v)) if v.title == "video 1"));
        assert!(matches!(state.item(2), Some(CatalogItem::Playlist(_))));
        assert!(matches!(state.item(3), Some(CatalogItem::Video(v)) if v.title == "video 3"));
        assert!(state.item(4).is_none());
        assert_eq!(item_id(&state.item(0).unwrap()), "00000000001");
    }
}
