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
    let (items, local_only) = state.guest_ui.watch_snapshot();
    let rows = items
        .iter()
        .enumerate()
        .take(MAX_RELATED_ROWS)
        .map(|(index, item)| {
            state
                .model
                .row_data(index)
                .filter(|row| row.id.as_str() == item_id(item))
                .unwrap_or_else(|| crate::guest_ui::row(item))
        })
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
    state.watch_context.items.borrow_mut().clear();
    state.watch_context.local_only.set(false);
    if state.watch_context.model.row_count() != 0 {
        crate::feed_focus::reset(app, state);
        state.watch_context.model.replace(Vec::new());
    }
    state.thumbnail_range.set((usize::MAX, usize::MAX));
}
