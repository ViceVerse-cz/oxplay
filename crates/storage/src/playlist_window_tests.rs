// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

fn collections(store: &LocalStore, count: usize) -> Vec<LocalPlaylistId> {
    (0..count)
        .map(|index| {
            store
                .create_playlist(&format!("Synthetic collection {index}"))
                .unwrap()
        })
        .collect()
}
fn ids(page: &PlaylistWindowPage) -> Vec<LocalPlaylistId> {
    page.items.iter().map(|item| item.id).collect()
}

#[test]
fn created_collection_101_and_206_is_in_its_bounded_selected_window() {
    for count in [101, 206] {
        let store = LocalStore::in_memory().unwrap();
        let created = collections(&store, count);
        let newest = *created.last().unwrap();
        let page = store
            .playlist_window(PlaylistWindow::EndingAt(newest), 100)
            .unwrap();
        assert_eq!(page.items.len(), 100);
        assert_eq!(ids(&page), created[count - 100..]);
        assert_eq!(page.items.last().unwrap().id, newest);
        assert!(page.next.is_none());
        assert!(page.previous.is_some());
        // Existing forward API stays unchanged for exports and other clients.
        assert_eq!(
            store
                .playlists(None, 100)
                .unwrap()
                .items
                .iter()
                .map(|v| v.id)
                .collect::<Vec<_>>(),
            created[..100]
        );
    }
}

#[test]
fn backward_then_forward_traversal_needs_no_unvisited_cursor_history() {
    let store = LocalStore::in_memory().unwrap();
    let created = collections(&store, 206);
    let mut page = store
        .playlist_window(PlaylistWindow::EndingAt(created[205]), 100)
        .unwrap();
    let mut reverse_pages = vec![ids(&page)];
    while let Some(previous) = page.previous {
        page = store.playlist_window(previous, 100).unwrap();
        assert!(page.items.len() <= 100);
        reverse_pages.push(ids(&page));
        assert!(reverse_pages.len() <= 3);
    }
    assert_eq!(
        reverse_pages.iter().map(Vec::len).collect::<Vec<_>>(),
        [100, 100, 6]
    );
    assert_eq!(
        reverse_pages
            .iter()
            .rev()
            .flatten()
            .copied()
            .collect::<Vec<_>>(),
        created
    );
    let mut forward_pages = vec![ids(&page)];
    while let Some(next) = page.next {
        page = store.playlist_window(next, 100).unwrap();
        assert!(page.items.len() <= 100);
        forward_pages.push(ids(&page));
        assert!(forward_pages.len() <= 3);
    }
    // Returned navigation recovers every adjacent window without constructing
    // a synthetic visited-page stack for the initial jump.
    assert_eq!(
        forward_pages.into_iter().flatten().collect::<Vec<_>>(),
        created
    );
    assert_eq!(page.items.last().unwrap().id, created[205]);
}

#[test]
fn forward_and_backward_boundaries_survive_deleted_boundary_rows() {
    let store = LocalStore::in_memory().unwrap();
    let created = collections(&store, 206);
    let first = store.playlist_window(PlaylistWindow::First, 100).unwrap();
    let next = first.next.unwrap();
    store.delete_playlist(created[99]).unwrap();
    let second = store.playlist_window(next, 100).unwrap();
    assert_eq!(ids(&second), created[100..200]);
    let previous = second.previous.unwrap();
    // Delete both the old boundary and a row within the previous window.
    store.delete_playlist(created[49]).unwrap();
    let earlier = store.playlist_window(previous, 100).unwrap();
    let expected: Vec<_> = created[..100]
        .iter()
        .copied()
        .filter(|id| *id != created[49] && *id != created[99])
        .collect();
    assert_eq!(ids(&earlier), expected);
    assert!(earlier.previous.is_none());
    let again = store.playlist_window(earlier.next.unwrap(), 100).unwrap();
    assert_eq!(ids(&again), created[100..200]);
}

#[test]
fn empty_trailing_window_can_return_including_the_surviving_cursor_row() {
    let store = LocalStore::in_memory().unwrap();
    let created = collections(&store, 101);
    let first = store.playlist_window(PlaylistWindow::First, 100).unwrap();
    let next = first.next.unwrap();
    store.delete_playlist(created[100]).unwrap();
    let empty = store.playlist_window(next, 100).unwrap();
    assert!(empty.items.is_empty());
    assert!(empty.next.is_none());
    let previous = store.playlist_window(empty.previous.unwrap(), 100).unwrap();
    assert_eq!(ids(&previous), created[..100]);
    assert_eq!(previous.items.last().unwrap().id, created[99]);
    assert!(previous.previous.is_none());
    assert!(previous.next.is_none());
}

#[test]
fn empty_backward_window_can_move_forward_and_fully_empty_library_has_no_links() {
    let store = LocalStore::in_memory().unwrap();
    let created = collections(&store, 101);
    let newest = store
        .playlist_window(PlaylistWindow::EndingAt(created[100]), 100)
        .unwrap();
    let previous = newest.previous.unwrap();
    store.delete_playlist(created[0]).unwrap();
    let empty = store.playlist_window(previous, 100).unwrap();
    assert!(empty.items.is_empty());
    assert!(empty.previous.is_none());
    let next = store.playlist_window(empty.next.unwrap(), 100).unwrap();
    assert_eq!(ids(&next), created[1..]);
    for id in &created[1..] {
        store.delete_playlist(*id).unwrap();
    }
    for window in [PlaylistWindow::First, previous, empty.next.unwrap()] {
        let page = store.playlist_window(window, 100).unwrap();
        assert!(page.items.is_empty() && page.previous.is_none() && page.next.is_none());
    }
}

#[test]
fn limits_and_missing_selected_collection_are_explicit_errors() {
    let store = LocalStore::in_memory().unwrap();
    let created = collections(&store, 3);
    for limit in [0, 101, u32::MAX] {
        assert!(matches!(
            store.playlist_window(PlaylistWindow::First, limit),
            Err(StorageError::InvalidInput)
        ));
    }
    let page = store
        .playlist_window(PlaylistWindow::EndingAt(created[1]), 1)
        .unwrap();
    assert_eq!(ids(&page), [created[1]]);
    assert_eq!(
        ids(&store.playlist_window(page.previous.unwrap(), 1).unwrap()),
        [created[0]]
    );
    assert_eq!(
        ids(&store.playlist_window(page.next.unwrap(), 1).unwrap()),
        [created[2]]
    );
    store.delete_playlist(created[1]).unwrap();
    assert!(matches!(
        store.playlist_window(PlaylistWindow::EndingAt(created[1]), 100),
        Err(StorageError::NotFound)
    ));
    // A missing selected identity is not silently replaced by another playlist.
    assert_eq!(store.playlists(None, 100).unwrap().items.len(), 2);
}

#[test]
fn canonical_refresh_survives_rename_and_deletion_of_the_revealed_collection() {
    let store = LocalStore::in_memory().unwrap();
    let created = collections(&store, 206);
    let page = store
        .playlist_window(PlaylistWindow::EndingAt(created[205]), 100)
        .unwrap();
    assert!(matches!(page.current, PlaylistWindow::After(_)));
    store
        .rename_playlist(created[205], "Synthetic renamed collection")
        .unwrap();
    let renamed = store.playlist_window(page.current, 100).unwrap();
    assert_eq!(
        renamed.items.last().unwrap().name,
        "Synthetic renamed collection"
    );
    assert_eq!(ids(&renamed), ids(&page));
    store.delete_playlist(created[205]).unwrap();
    let removed = store.playlist_window(page.current, 100).unwrap();
    assert_eq!(ids(&removed), created[106..205]);
    assert_eq!(removed.current, page.current);
    assert!(removed.next.is_none());
    let earliest = store
        .playlist_window(PlaylistWindow::EndingAt(created[0]), 100)
        .unwrap();
    assert_eq!(earliest.current, PlaylistWindow::First);
}
