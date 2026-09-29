// SPDX-License-Identifier: GPL-3.0-or-later
//! Bounded keyed reconciliation for one authoritative local-library page.
use slint::{Model, VecModel};
pub(super) const MAX_ROWS: usize = 100;
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Changes {
    pub reset: bool,
    pub changed: usize,
    pub inserted: usize,
    pub removed: usize,
}

/// No UI/data borrow is held across a model notification. Keys are independent
/// of display text. Surviving unmoved rows receive no notification at all.
pub(super) fn reconcile<K: Clone + Eq, T: Clone + PartialEq + 'static>(
    model: &VecModel<T>,
    old_keys: &[K],
    rows: &[(K, T)],
    same_page: bool,
) -> Result<Changes, &'static str> {
    if rows.len() > MAX_ROWS
        || old_keys.len() > MAX_ROWS
        || model.row_count() != old_keys.len()
        || rows
            .iter()
            .enumerate()
            .any(|(i, row)| rows[..i].iter().any(|other| other.0 == row.0))
        || old_keys
            .iter()
            .enumerate()
            .any(|(i, key)| old_keys[..i].contains(key))
    {
        return Err("Invalid bounded local-library page");
    }
    let mut changes = Changes::default();
    if !same_page {
        model.set_vec(rows.iter().map(|(_, row)| row.clone()).collect::<Vec<_>>());
        changes.reset = true;
        return Ok(changes);
    }
    let mut keys = old_keys.to_vec();
    // Delete first so a single removed row never causes replacements of its
    // otherwise unchanged following siblings. Reverse order keeps indices valid.
    for index in (0..keys.len()).rev() {
        if !rows.iter().any(|(key, _)| key == &keys[index]) {
            keys.remove(index);
            model.remove(index);
            changes.removed += 1;
        }
    }
    for (index, (key, row)) in rows.iter().enumerate() {
        if keys.get(index) != Some(key) {
            if let Some(previous) = keys.iter().position(|old| old == key) {
                keys.remove(previous);
                model.remove(previous);
                changes.removed += 1;
            }
            keys.insert(index, key.clone());
            model.insert(index, row.clone());
            changes.inserted += 1;
        } else if model.row_data(index).as_ref() != Some(row) {
            model.set_row_data(index, row.clone());
            changes.changed += 1;
        }
    }
    Ok(changes)
}

#[cfg(test)]
mod tests {
    use super::*;
    // Test-only exact pinned Slint API: attach a real observer rather than
    // asserting counters recorded by the implementation under test.
    use slint::private_unstable_api::re_exports::{
        ModelChangeListener, ModelChangeListenerContainer,
    };
    use std::{cell::RefCell, pin::Pin};
    #[derive(Default)]
    struct Observer {
        events: RefCell<Vec<(&'static str, usize, usize)>>,
    }
    impl ModelChangeListener for Observer {
        fn row_changed(self: Pin<&Self>, row: usize) {
            self.events.borrow_mut().push(("change", row, 1));
        }
        fn row_added(self: Pin<&Self>, row: usize, count: usize) {
            self.events.borrow_mut().push(("insert", row, count));
        }
        fn row_removed(self: Pin<&Self>, row: usize, count: usize) {
            self.events.borrow_mut().push(("remove", row, count));
        }
        fn reset(self: Pin<&Self>) {
            self.events.borrow_mut().push(("reset", 0, 0));
        }
    }
    fn run(
        old: &[(u8, &'static str)],
        new: &[(u8, &'static str)],
    ) -> Vec<(&'static str, usize, usize)> {
        let model = VecModel::from(old.iter().map(|(_, row)| *row).collect::<Vec<_>>());
        let observer = Box::pin(ModelChangeListenerContainer::new(Observer::default()));
        model
            .model_tracker()
            .attach_peer(observer.as_ref().model_peer());
        let keys = old.iter().map(|(key, _)| *key).collect::<Vec<_>>();
        let changes = reconcile(&model, &keys, new, true).unwrap();
        assert_eq!(
            (0..model.row_count())
                .map(|i| model.row_data(i).unwrap())
                .collect::<Vec<_>>(),
            new.iter().map(|(_, row)| *row).collect::<Vec<_>>()
        );
        assert!(!changes.reset);
        observer.events.borrow().clone()
    }
    #[test]
    fn rename_remove_unfollow_and_add_emit_only_affected_notifications() {
        let original = [(1, "one"), (2, "two"), (3, "three")];
        assert_eq!(
            run(&original, &[(1, "one"), (2, "renamed"), (3, "three")]),
            [("change", 1, 1)]
        );
        assert_eq!(
            run(&original, &[(1, "one"), (3, "three")]),
            [("remove", 1, 1)]
        );
        assert_eq!(
            run(&original, &[(2, "two"), (3, "three")]),
            [("remove", 0, 1)]
        );
        assert_eq!(
            run(
                &original,
                &[(0, "new"), (1, "one"), (2, "two"), (3, "three")]
            ),
            [("insert", 0, 1)]
        );
        assert!(run(&original, &original).is_empty());
    }
    #[test]
    fn same_page_deletion_backfills_authoritative_next_row_without_reset() {
        let old: Vec<_> = (0u8..100).map(|i| (i, "unchanged")).collect();
        let new: Vec<_> = (0u8..101)
            .filter(|i| *i != 30)
            .map(|i| (i, "unchanged"))
            .collect();
        assert_eq!(run(&old, &new), [("remove", 30, 1), ("insert", 99, 1)]);
    }
    #[test]
    fn reorder_moves_only_changed_identity_and_route_change_emits_reset() {
        let old = [(1, "one"), (2, "two"), (3, "three")];
        assert_eq!(
            run(&old, &[(3, "three"), (1, "one"), (2, "two")]),
            [("remove", 2, 1), ("insert", 0, 1)]
        );
        let model = VecModel::from(vec!["one", "two", "three"]);
        let observer = Box::pin(ModelChangeListenerContainer::new(Observer::default()));
        model
            .model_tracker()
            .attach_peer(observer.as_ref().model_peer());
        let delta = reconcile(&model, &[1, 2, 3], &old, false).unwrap();
        assert!(delta.reset);
        assert_eq!(*observer.events.borrow(), [("reset", 0, 0)]);
    }
    #[test]
    fn malformed_or_oversized_page_emits_nothing_and_cannot_partially_mutate() {
        let model = VecModel::from(vec!["one"]);
        let observer = Box::pin(ModelChangeListenerContainer::new(Observer::default()));
        model
            .model_tracker()
            .attach_peer(observer.as_ref().model_peer());
        for rows in [
            vec![(1, "changed"), (1, "duplicate")],
            (0..101).map(|i| (i, "large")).collect(),
        ] {
            assert!(reconcile(&model, &[1], &rows, true).is_err());
            assert_eq!(model.row_count(), 1);
            assert_eq!(model.row_data(0), Some("one"));
            assert!(observer.events.borrow().is_empty());
        }
    }
}
