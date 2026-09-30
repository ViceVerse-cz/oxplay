// SPDX-License-Identifier: GPL-3.0-or-later
//! Tiny in-memory memory of decoded thumbnails that just left the viewport.
//!
//! Rows leaving the request window release their Slint image so a long feed
//! never retains more than the viewport plus overscan. Scrolling back used to
//! depend entirely on a fresh asynchronous fetch/decode whose in-flight jobs are
//! cancelled by every later range change, leaving cards blank until the scroll
//! settled (or indefinitely after a transient failure). Keeping the most recent
//! few released images lets an immediate return repaint synchronously.
//!
//! The bound is `CAPACITY` images (at most 320x180 RGBA, about 0.22 MiB each,
//! about 7 MiB total). Entries are keyed by row kind and identity, never by
//! URL, live only for one thumbnail surface and are cleared whenever the surface
//! changes or public/local/account catalog data is cleared.
use std::collections::VecDeque;

pub const CAPACITY: usize = 32;

pub struct Retention<T> {
    capacity: usize,
    entries: VecDeque<(String, T)>,
}
impl<T> Default for Retention<T> {
    fn default() -> Self {
        Self::new(CAPACITY)
    }
}
impl<T> Retention<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: VecDeque::new(),
        }
    }
    /// Remember a released image. The newest entry is the last to be evicted.
    pub fn put(&mut self, key: String, value: T) {
        self.entries.retain(|(existing, _)| *existing != key);
        self.entries.push_back((key, value));
        while self.entries.len() > self.capacity {
            self.entries.pop_front();
        }
    }
    /// Move a retained image back to the model; it is no longer retained here.
    pub fn take(&mut self, key: &str) -> Option<T> {
        let index = self
            .entries
            .iter()
            .position(|(existing, _)| existing == key)?;
        self.entries.remove(index).map(|(_, value)| value)
    }
    pub fn clear(&mut self) {
        self.entries.clear();
    }
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Stable identity independent of URL and row index.
pub fn key(kind: &str, id: &str) -> String {
    format!("{kind}\u{0}{id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Models the scroll-away/scroll-back sequence: a window of rows shows
    /// images, the window moves and releases them, and coming back must find
    /// the recently released rows without another network/disk request.
    #[test]
    fn scrolling_away_and_back_restores_recent_rows_within_a_fixed_bound() {
        let mut retained = Retention::<usize>::default();
        // Rows 0..12 were visible; rows 0..9 leave the window, nearest last.
        for row in 0..9 {
            retained.put(key("Video", &format!("v{row}")), row);
        }
        assert_eq!(retained.len(), 9);
        for row in 0..9 {
            assert_eq!(retained.take(&key("Video", &format!("v{row}"))), Some(row));
        }
        assert_eq!(retained.len(), 0, "restored images leave the retained set");
        assert_eq!(retained.take(&key("Video", "v0")), None);
    }

    #[test]
    fn memory_is_bounded_and_the_farthest_rows_are_dropped_first() {
        let mut retained = Retention::<usize>::default();
        for row in 0..CAPACITY + 10 {
            retained.put(key("Video", &row.to_string()), row);
        }
        assert_eq!(retained.len(), CAPACITY);
        assert_eq!(retained.take(&key("Video", "0")), None);
        assert_eq!(retained.take(&key("Video", "9")), None);
        assert_eq!(retained.take(&key("Video", "10")), Some(10));
        assert_eq!(
            retained.take(&key("Video", &(CAPACITY + 9).to_string())),
            Some(CAPACITY + 9)
        );
    }

    #[test]
    fn identity_includes_kind_and_reput_replaces_instead_of_growing() {
        let mut retained = Retention::<&str>::new(2);
        retained.put(key("Video", "same"), "video");
        retained.put(key("Channel", "same"), "channel");
        retained.put(key("Video", "same"), "video again");
        assert_eq!(retained.len(), 2);
        assert_eq!(retained.take(&key("Channel", "same")), Some("channel"));
        assert_eq!(retained.take(&key("Video", "same")), Some("video again"));
    }

    #[test]
    fn clear_forgets_everything() {
        let mut retained = Retention::<u8>::default();
        retained.put(key("Video", "a"), 1);
        retained.clear();
        assert_eq!(retained.len(), 0);
        assert_eq!(retained.take(&key("Video", "a")), None);
    }
}
