// SPDX-License-Identifier: GPL-3.0-or-later
//! One bounded guest-only related page with ownership independent of browsing.
//! The real watch-next list from the native `next` response replaces the
//! captured browsing snapshot when it arrives; the snapshot is the fallback.
//! Account/private metadata never enters this model; the selected video's other
//! fields remain owned by the existing playback/details adapters.
use crate::{App, UiState, VideoRow, model::CatalogModel};
use serein_core::{CatalogItem, VideoId, VideoSummary};
use slint::Model;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

const MAX_RELATED_ROWS: usize = serein_youtube::watch::MAX_RELATED;
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
/// Publish the native watch-next list for the accepted guest selection. Row
/// indices are positions in this list (what select-related and feed focus
/// resolve); focus resets as for any explicit catalog replacement. Decoded
/// images already shown for the same item are kept. Thumbnail jobs are retired
/// only when the related surface currently owns the shared thumbnail worker.
pub fn replace_native(app: &App, state: &UiState, related: &[CatalogItem]) {
    let items = native_items(related);
    if items.is_empty() {
        return;
    }
    let previous = &state.watch_context.model;
    let previous: Vec<VideoRow> = (0..previous.row_count())
        .filter_map(|row| previous.row_data(row))
        .collect();
    let rows = native_rows(&previous, &items);
    let owns_thumbnails = state.thumbnail_surface.get() == 1;
    if owns_thumbnails {
        state.thumbnails.borrow_mut().replace(Vec::new());
        state.thumbnail_attempted.borrow_mut().clear();
    }
    *state.watch_context.items.borrow_mut() = items;
    state.watch_context.local_only.set(false);
    crate::feed_focus::reset(app, state);
    state.watch_context.model.replace(rows);
    if owns_thumbnails {
        state.thumbnail_range.set((usize::MAX, usize::MAX));
        app.invoke_refresh_visible();
    }
}
fn native_items(related: &[CatalogItem]) -> Vec<CatalogItem> {
    related_candidates(related.to_vec())
        .into_iter()
        .map(|(_, item)| item)
        .take(MAX_RELATED_ROWS)
        .collect()
}
/// One row per item, in item order; a decoded image already shown for the same
/// item identity is reused instead of being fetched again.
fn native_rows(previous: &[VideoRow], items: &[CatalogItem]) -> Vec<VideoRow> {
    items
        .iter()
        .map(|item| {
            let mut fresh = crate::guest_ui::row(item);
            if let Some(shown) = previous
                .iter()
                .find(|row| row.thumbnail_ready && row.kind == fresh.kind && row.id == fresh.id)
            {
                fresh.thumbnail = shown.thumbnail.clone();
                fresh.thumbnail_ready = true;
            }
            fresh
        })
        .collect()
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
    crate::watch_meta::clear(state);
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
            metadata: None,
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

    #[test]
    fn native_related_rows_match_item_indices_and_keep_only_same_identity_images() {
        // TEST FIXTURE: synthetic watch-next list (a stray channel row included).
        let mut related: Vec<CatalogItem> = (0..40).map(video).collect();
        related.insert(3, channel());
        let items = native_items(&related);
        assert_eq!(items.len(), MAX_RELATED_ROWS);
        assert!(
            items
                .iter()
                .all(|item| matches!(item, CatalogItem::Video(_)))
        );
        // A previously shown row for the same video keeps its decoded image;
        // a row whose ID differs never lends its image to another item.
        let mut shown = crate::guest_ui::row(&video(2));
        shown.thumbnail_ready = true;
        let mut other = crate::guest_ui::row(&video(99));
        other.thumbnail_ready = true;
        let rows = native_rows(&[other, shown], &items);
        assert_eq!(rows.len(), items.len());
        for (row, item) in rows.iter().zip(&items) {
            assert_eq!(row.id.as_str(), item_id(item), "row index names its item");
        }
        assert!(rows[2].thumbnail_ready);
        assert_eq!(rows.iter().filter(|row| row.thumbnail_ready).count(), 1);
    }
}
