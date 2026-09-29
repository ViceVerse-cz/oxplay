// SPDX-License-Identifier: GPL-3.0-or-later
//! Only column changes regroup existing rows. A thumbnail changes one child row;
//! appending data retains existing child models and scroll position.
use crate::{VideoGroup, VideoRow, model::CatalogModel};
use slint::{Model, ModelRc, VecModel};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
pub const MAX_COLUMNS: usize = 6;

/// Keep the existing 40-image limit. If leading overscan would displace a
/// visible interval that fits that limit, trim only the leading overscan.
/// Larger-than-budget viewports retain the prior deterministic bounded policy;
/// they cannot be described as fully populated resource fixtures.
pub fn thumbnail_window(
    total: usize,
    requested: (usize, usize),
    visible: Option<(usize, usize)>,
) -> (usize, usize) {
    let mut first = requested.0.min(total);
    let end = requested.1.min(total).max(first);
    if let Some((visible_first, visible_end)) = visible
        && first <= visible_first
        && visible_first <= visible_end
        && visible_end <= end
        && visible_end - visible_first <= 40
    {
        first = first.max(visible_end.saturating_sub(40)).min(visible_first);
    }
    (first, end.min(first + 40))
}
pub struct Groups {
    pub model: Rc<CatalogModel<VideoGroup>>,
    /// All child row notifications, distinct from parent regroup/reset counts.
    pub child_changes: Cell<u64>,
    columns: Cell<usize>,
    children: RefCell<Vec<Rc<VecModel<VideoRow>>>>,
}
impl Default for Groups {
    fn default() -> Self {
        Self {
            model: Rc::new(CatalogModel::default()),
            child_changes: Cell::new(0),
            columns: Cell::new(3),
            children: RefCell::new(Vec::new()),
        }
    }
}
impl Groups {
    pub fn set_columns(&self, columns: usize, source: &CatalogModel<VideoRow>) {
        let columns = columns.clamp(1, MAX_COLUMNS);
        if self.columns.replace(columns) != columns {
            self.replace(source);
        }
    }
    pub fn replace(&self, source: &CatalogModel<VideoRow>) {
        let columns = self.columns.get();
        let mut children = self.children.borrow_mut();
        children.clear();
        let mut groups = Vec::new();
        for start in (0..source.row_count()).step_by(columns) {
            let rows: Vec<_> = (start..(start + columns).min(source.row_count()))
                .filter_map(|i| source.row_data(i))
                .collect();
            let model = Rc::new(VecModel::from(rows));
            groups.push(VideoGroup {
                start: start as i32,
                items: ModelRc::from(model.clone()),
            });
            children.push(model);
        }
        drop(children);
        self.model.replace(groups);
    }
    pub fn append(&self, source: &CatalogModel<VideoRow>) {
        let columns = self.columns.get();
        let mut children = self.children.borrow_mut();
        let mut added = Vec::new();
        for group in 0..source.row_count().div_ceil(columns) {
            let start = group * columns;
            if let Some(model) = children.get(group) {
                for row in model.row_count()..columns.min(source.row_count() - start) {
                    model.push(source.row_data(start + row).unwrap());
                    self.child_changes.set(self.child_changes.get() + 1);
                }
            } else {
                let rows: Vec<_> = (start..(start + columns).min(source.row_count()))
                    .filter_map(|i| source.row_data(i))
                    .collect();
                let model = Rc::new(VecModel::from(rows));
                added.push(VideoGroup {
                    start: start as i32,
                    items: ModelRc::from(model.clone()),
                });
                children.push(model);
            }
        }
        drop(children);
        self.model.append(added);
    }
    pub fn update(&self, row: usize, data: VideoRow) {
        if let Some(child) = self.children.borrow().get(row / self.columns.get()) {
            child.set_row_data(row % self.columns.get(), data);
            self.child_changes.set(self.child_changes.get() + 1);
        }
    }

    /// Reconcile an authoritative page at the existing column count. Retain
    /// child model identities and notify only slots whose content/membership
    /// changed. A removed flat row may shift cards across group boundaries;
    /// those affected slots change, but unrelated groups never reset.
    pub fn reconcile(&self, source: &CatalogModel<VideoRow>) {
        let columns = self.columns.get();
        let mut children = self.children.borrow().clone();
        let count = source.row_count().div_ceil(columns);
        children.truncate(count);
        let mut groups = Vec::with_capacity(count);
        for group in 0..count {
            let start = group * columns;
            let length = columns.min(source.row_count() - start);
            let child = if let Some(child) = children.get(group) {
                let child = child.clone();
                while child.row_count() > length {
                    child.remove(child.row_count() - 1);
                    self.child_changes.set(self.child_changes.get() + 1);
                }
                for row in 0..length {
                    let data = source.row_data(start + row).unwrap();
                    if row == child.row_count() {
                        child.push(data);
                        self.child_changes.set(self.child_changes.get() + 1);
                    } else if child.row_data(row).as_ref() != Some(&data) {
                        child.set_row_data(row, data);
                        self.child_changes.set(self.child_changes.get() + 1);
                    }
                }
                child
            } else {
                let child = Rc::new(VecModel::from(
                    (start..start + length)
                        .map(|row| source.row_data(row).unwrap())
                        .collect::<Vec<_>>(),
                ));
                children.push(child.clone());
                child
            };
            groups.push(VideoGroup {
                start: start as i32,
                items: ModelRc::from(child),
            });
        }
        *self.children.borrow_mut() = children;
        // Starts are unique multiples of the nonzero bounded column count.
        // The source and parent models both enforce MAX_CATALOG_ROWS.
        self.model
            .reconcile(groups, |group| group.start)
            .expect("bounded unique video groups");
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
    fn same_page_reconciliation_keeps_models_and_notifies_only_changed_child() {
        let flat = CatalogModel::default();
        flat.replace((0..6).map(row).collect());
        let grouped = Groups::default();
        grouped.replace(&flat);
        let first = grouped.children.borrow()[0].clone();
        let second = grouped.children.borrow()[1].clone();
        // Test-only pinned Slint observer API verifies actual emitted events.
        let parent_events = Box::pin(ModelChangeListenerContainer::new(Observer::default()));
        let first_events = Box::pin(ModelChangeListenerContainer::new(Observer::default()));
        let second_events = Box::pin(ModelChangeListenerContainer::new(Observer::default()));
        grouped
            .model
            .model_tracker()
            .attach_peer(parent_events.as_ref().model_peer());
        first
            .model_tracker()
            .attach_peer(first_events.as_ref().model_peer());
        second
            .model_tracker()
            .attach_peer(second_events.as_ref().model_peer());
        grouped.reconcile(&flat);
        assert!(parent_events.events.borrow().is_empty());
        assert!(first_events.events.borrow().is_empty());
        assert!(second_events.events.borrow().is_empty());
        flat.set_row_data(1, row(99));
        grouped.reconcile(&flat);
        assert_eq!(*first_events.events.borrow(), [("change", 1, 1)]);
        assert!(parent_events.events.borrow().is_empty());
        assert!(second_events.events.borrow().is_empty());
        assert!(Rc::ptr_eq(&grouped.children.borrow()[0], &first));
        assert!(Rc::ptr_eq(&grouped.children.borrow()[1], &second));
    }

    #[test]
    fn same_page_membership_trims_children_and_groups_without_reset() {
        let flat = CatalogModel::default();
        flat.replace((0..6).map(row).collect());
        let grouped = Groups::default();
        grouped.replace(&flat);
        let first = grouped.children.borrow()[0].clone();
        let second = grouped.children.borrow()[1].clone();
        let parent_events = Box::pin(ModelChangeListenerContainer::new(Observer::default()));
        let first_events = Box::pin(ModelChangeListenerContainer::new(Observer::default()));
        let second_events = Box::pin(ModelChangeListenerContainer::new(Observer::default()));
        grouped
            .model
            .model_tracker()
            .attach_peer(parent_events.as_ref().model_peer());
        first
            .model_tracker()
            .attach_peer(first_events.as_ref().model_peer());
        second
            .model_tracker()
            .attach_peer(second_events.as_ref().model_peer());
        flat.reconcile((0..5).map(row).collect(), |row| row.title.clone())
            .unwrap();
        grouped.reconcile(&flat);
        assert_eq!(*second_events.events.borrow(), [("remove", 2, 1)]);
        assert!(first_events.events.borrow().is_empty());
        assert!(parent_events.events.borrow().is_empty());
        flat.reconcile((0..3).map(row).collect(), |row| row.title.clone())
            .unwrap();
        grouped.reconcile(&flat);
        assert_eq!(*parent_events.events.borrow(), [("remove", 1, 1)]);
        assert!(first_events.events.borrow().is_empty());
        assert!(Rc::ptr_eq(&grouped.children.borrow()[0], &first));
        parent_events.events.borrow_mut().clear();
        flat.reconcile((0..4).map(row).collect(), |row| row.title.clone())
            .unwrap();
        grouped.reconcile(&flat);
        assert_eq!(*parent_events.events.borrow(), [("insert", 1, 1)]);
        assert!(first_events.events.borrow().is_empty());
        assert_eq!(grouped.model.row_data(1).unwrap().start, 3);
        assert_eq!(grouped.model.resets.get(), 1);
    }

    fn row(n: usize) -> VideoRow {
        VideoRow {
            title: n.to_string().into(),
            ..VideoRow::default()
        }
    }
    #[test]
    fn image_updates_and_append_preserve_existing_row_models() {
        let flat = CatalogModel::default();
        flat.replace((0..5).map(row).collect());
        let grouped = Groups::default();
        grouped.replace(&flat);
        let first = grouped.children.borrow()[0].clone();
        let second = grouped.children.borrow()[1].clone();
        let resets = grouped.model.resets.get();
        let changes = grouped.model.changes.get();
        for _ in 0..1000 {
            grouped.set_columns(3, &flat);
        }
        assert_eq!(grouped.model.resets.get(), resets);
        grouped.update(1, row(99));
        assert_eq!(grouped.child_changes.get(), 1);
        assert_eq!(first.row_data(1).unwrap().title.as_str(), "99");
        assert_eq!(second.row_data(0).unwrap().title.as_str(), "3");
        assert_eq!(grouped.model.changes.get(), changes);
        flat.append((5..9).map(row).collect());
        grouped.append(&flat);
        assert_eq!(grouped.child_changes.get(), 2);
        assert!(Rc::ptr_eq(&grouped.children.borrow()[0], &first));
        assert!(Rc::ptr_eq(&grouped.children.borrow()[1], &second));
        assert_eq!(grouped.model.resets.get(), resets);
        assert_eq!(second.row_count(), 3);
        assert_eq!(grouped.model.row_count(), 3);
    }
    #[test]
    fn only_column_boundary_changes_regroup() {
        let flat = CatalogModel::default();
        flat.replace((0..120).map(row).collect());
        let grouped = Groups::default();
        grouped.replace(&flat);
        grouped.set_columns(4, &flat);
        assert_eq!(grouped.model.row_count(), 30);
        assert_eq!(grouped.model.row_data(29).unwrap().start, 116);
        let resets = grouped.model.resets.get();
        for _ in 0..1000 {
            grouped.set_columns(4, &flat);
        }
        assert_eq!(grouped.model.resets.get(), resets);
    }

    #[test]
    fn six_columns_keep_partial_groups_and_incremental_thumbnail_identity() {
        let flat = CatalogModel::default();
        flat.replace((0..100).map(row).collect());
        let grouped = Groups::default();
        grouped.set_columns(6, &flat);
        assert_eq!(grouped.model.row_count(), 17);
        assert_eq!(grouped.model.row_data(16).unwrap().start, 96);
        let last = grouped.children.borrow()[16].clone();
        assert_eq!(last.row_count(), 4);
        let resets = grouped.model.resets.get();
        grouped.update(99, row(999));
        assert_eq!(last.row_data(3).unwrap().title.as_str(), "999");
        assert_eq!(grouped.child_changes.get(), 1);
        for _ in 0..100 {
            grouped.set_columns(6, &flat);
        }
        assert_eq!(grouped.model.resets.get(), resets);
        assert!(Rc::ptr_eq(&last, &grouped.children.borrow()[16]));
        flat.append((100..120).map(row).collect());
        grouped.append(&flat);
        assert_eq!(last.row_count(), 6);
        assert!(Rc::ptr_eq(&last, &grouped.children.borrow()[16]));
        assert_eq!(grouped.model.row_count(), 20);
        assert_eq!(grouped.model.resets.get(), resets);
    }

    #[test]
    fn visible_thumbnails_displace_only_leading_overscan_with_unchanged_budget() {
        // Six columns, five intersecting thumbnail rows. The scroll position
        // begins in row2's text band: images18..48 are visible but the old
        // admission6..46 dropped the last two visible images.
        assert_eq!(thumbnail_window(100, (6, 54), Some((18, 48))), (8, 48));
        assert_eq!(thumbnail_window(100, (6, 48), Some((12, 42))), (6, 46));
        assert_eq!(thumbnail_window(100, (0, 36), Some((0, 30))), (0, 36));
        assert_eq!(thumbnail_window(100, (60, 108), Some((72, 100))), (60, 100));
        assert_eq!(thumbnail_window(120, (6, 54), None), (6, 46));
        assert_eq!(thumbnail_window(120, (0, 72), Some((0, 66))), (0, 40));
        // Generated bounded intervals cover partial final groups and each
        // possible leading overscan amount, without constructing UI delegates.
        for total in [1usize, 20, 100, 120] {
            for visible_first in 0..total {
                for count in 1..=40.min(total - visible_first) {
                    let visible_end = visible_first + count;
                    for leading in 0..=12 {
                        let requested = (
                            visible_first.saturating_sub(leading),
                            (visible_end + 6).min(total),
                        );
                        let (first, end) =
                            thumbnail_window(total, requested, Some((visible_first, visible_end)));
                        assert!(first >= requested.0 && end <= requested.1 && end <= total);
                        assert!(end - first <= 40);
                        assert!(first <= visible_first && end >= visible_end);
                    }
                }
            }
        }
    }
}
