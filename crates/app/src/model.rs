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
