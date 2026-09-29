//! Stable, bounded catalog model; all notification sites are counted.
use slint::{Model, ModelNotify, ModelTracker};
use std::cell::{Cell, RefCell};
pub const MAX_CATALOG_ROWS: usize = 120;
pub struct CatalogModel<T> {
    rows: RefCell<Vec<T>>,
    notify: ModelNotify,
    pub changes: Cell<u64>,
    pub resets: Cell<u64>,
}
impl<T> Default for CatalogModel<T> {
    fn default() -> Self {
        Self {
            rows: RefCell::new(Vec::new()),
            notify: ModelNotify::default(),
            changes: Cell::new(0),
            resets: Cell::new(0),
        }
    }
}
impl<T: Clone + 'static> CatalogModel<T> {
    pub fn replace(&self, rows: Vec<T>) {
        *self.rows.borrow_mut() = rows.into_iter().take(MAX_CATALOG_ROWS).collect();
        self.resets.set(self.resets.get() + 1);
        self.notify.reset();
    }
    pub fn append(&self, rows: Vec<T>) {
        let mut data = self.rows.borrow_mut();
        let start = data.len();
        let count = rows.len().min(MAX_CATALOG_ROWS - start);
        data.extend(rows.into_iter().take(count));
        drop(data);
        if count > 0 {
            self.changes.set(self.changes.get() + 1);
            self.notify.row_added(start, count);
        }
    }

    /// Reconcile one bounded authoritative page without resetting its model.
    /// Keys must identify rows independently of their displayed metadata. Reject
    /// ambiguous/oversized inputs before publishing any partial mutation.
    pub fn reconcile<K: Eq>(&self, rows: Vec<T>, key: impl Fn(&T) -> K) -> Result<(), &'static str>
    where
        T: PartialEq,
    {
        if rows.len() > MAX_CATALOG_ROWS {
            return Err("Catalog page exceeds its row limit");
        }
        let original = self.rows.borrow().clone();
        let mut keys: Vec<_> = original.iter().map(&key).collect();
        let incoming: Vec<_> = rows.iter().map(&key).collect();
        if keys.iter().enumerate().any(|(i, k)| keys[..i].contains(k))
            || incoming
                .iter()
                .enumerate()
                .any(|(i, k)| incoming[..i].contains(k))
        {
            return Err("Catalog page contains duplicate row identities");
        }
        // Remove disappeared identities first: following unchanged siblings
        // must not each receive a replacement notification for a single delete.
        for index in (0..keys.len()).rev() {
            if !incoming.contains(&keys[index]) {
                keys.remove(index);
                self.remove(index);
            }
        }
        for (index, (row, identity)) in rows.into_iter().zip(incoming).enumerate() {
            if keys.get(index) != Some(&identity) {
                if let Some(previous) = keys.iter().position(|old| old == &identity) {
                    keys.remove(previous);
                    self.remove(previous);
                }
                keys.insert(index, identity);
                self.rows.borrow_mut().insert(index, row);
                self.changes.set(self.changes.get() + 1);
                self.notify.row_added(index, 1);
            } else if self.row_data(index).as_ref() != Some(&row) {
                self.set_row_data(index, row);
            }
        }
        Ok(())
    }

    fn remove(&self, index: usize) {
        self.rows.borrow_mut().remove(index);
        self.changes.set(self.changes.get() + 1);
        self.notify.row_removed(index, 1);
    }
}
impl<T: Clone + 'static> Model for CatalogModel<T> {
    type Data = T;
    fn row_count(&self) -> usize {
        self.rows.borrow().len()
    }
    fn row_data(&self, row: usize) -> Option<T> {
        self.rows.borrow().get(row).cloned()
    }
    fn set_row_data(&self, row: usize, data: T) {
        let mut rows = self.rows.borrow_mut();
        if let Some(target) = rows.get_mut(row) {
            *target = data;
            drop(rows);
            self.changes.set(self.changes.get() + 1);
            self.notify.row_changed(row);
        }
    }
    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use slint::private_unstable_api::re_exports::{
        ModelChangeListener, ModelChangeListenerContainer,
    };
    use std::pin::Pin;

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

    #[test]
    fn keyed_refresh_notifies_only_actual_metadata_and_membership_changes() {
        let model = CatalogModel::default();
        let original = vec![(1, "one"), (2, "two"), (3, "three")];
        model.replace(original.clone());
        let observer = Box::pin(ModelChangeListenerContainer::new(Observer::default()));
        model
            .model_tracker()
            .attach_peer(observer.as_ref().model_peer());
        model.reconcile(original, |row| row.0).unwrap();
        assert!(observer.events.borrow().is_empty());
        model
            .reconcile(vec![(1, "one"), (2, "renamed"), (3, "three")], |row| row.0)
            .unwrap();
        assert_eq!(*observer.events.borrow(), [("change", 1, 1)]);
        observer.events.borrow_mut().clear();
        model
            .reconcile(vec![(1, "one"), (3, "three")], |row| row.0)
            .unwrap();
        assert_eq!(*observer.events.borrow(), [("remove", 1, 1)]);
        observer.events.borrow_mut().clear();
        model
            .reconcile(vec![(0, "new"), (1, "one"), (3, "three")], |row| row.0)
            .unwrap();
        assert_eq!(*observer.events.borrow(), [("insert", 0, 1)]);
        assert_eq!(model.resets.get(), 1);
        assert_eq!(model.row_data(2), Some((3, "three")));
    }

    #[test]
    fn keyed_moves_and_page_backfill_preserve_bounded_model() {
        let model = CatalogModel::default();
        model.replace((0..MAX_CATALOG_ROWS).collect());
        let observer = Box::pin(ModelChangeListenerContainer::new(Observer::default()));
        model
            .model_tracker()
            .attach_peer(observer.as_ref().model_peer());
        let next: Vec<_> = (0..=MAX_CATALOG_ROWS).filter(|n| *n != 30).collect();
        model.reconcile(next.clone(), |row| *row).unwrap();
        assert_eq!(
            *observer.events.borrow(),
            [("remove", 30, 1), ("insert", 119, 1)]
        );
        observer.events.borrow_mut().clear();
        let mut moved = next;
        let last = moved.pop().unwrap();
        moved.insert(0, last);
        model.reconcile(moved.clone(), |row| *row).unwrap();
        assert_eq!(
            *observer.events.borrow(),
            [("remove", 119, 1), ("insert", 0, 1)]
        );
        assert_eq!(
            (0..model.row_count())
                .map(|i| model.row_data(i).unwrap())
                .collect::<Vec<_>>(),
            moved
        );
        assert_eq!(model.row_count(), MAX_CATALOG_ROWS);
    }

    #[test]
    fn malformed_reconciliation_is_rejected_before_any_notification() {
        let model = CatalogModel::default();
        model.replace(vec![1]);
        let observer = Box::pin(ModelChangeListenerContainer::new(Observer::default()));
        model
            .model_tracker()
            .attach_peer(observer.as_ref().model_peer());
        for rows in [vec![2, 2], (0..=MAX_CATALOG_ROWS).collect()] {
            assert!(model.reconcile(rows, |row| *row).is_err());
            assert_eq!(model.row_data(0), Some(1));
            assert_eq!(model.row_count(), 1);
            assert!(observer.events.borrow().is_empty());
        }
        model.replace(vec![1, 1]);
        observer.events.borrow_mut().clear();
        assert!(model.reconcile(vec![1], |row| *row).is_err());
        assert_eq!(model.row_count(), 2);
        assert!(observer.events.borrow().is_empty());
    }
    #[test]
    fn bounded_incremental_rows() {
        let model = CatalogModel::default();
        model.replace(vec![0; 100]);
        model.append(vec![1; 100]);
        assert_eq!(model.row_count(), 120);
        model.set_row_data(2, 7);
        assert_eq!(model.row_data(2), Some(7));
        assert_eq!(model.resets.get(), 1);
        assert_eq!(model.changes.get(), 2);
    }
}
