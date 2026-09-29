// SPDX-License-Identifier: GPL-3.0-or-later
//! Experimental adapter for Slint cf3b07d4917e6759a63b0c03913a2594ec653414.
//! This deliberately uses a pinned internal API, not a public Tooltip hook.
use std::cell::RefCell;

/// Query after all potentially lazy UI getters, before borrowing any native
/// presenter. No property access, cache, callbacks, timers or redraw requests.
/// Every Slint popup suppresses the native child, including generated tooltips.
/// Failure to borrow is also suppression; no Ref can escape this function.
pub(super) fn is_clear(window: &slint::Window) -> bool {
    use slint::private_unstable_api::re_exports::WindowInner;
    stack_is_clear(&WindowInner::from_pub(window).active_popups)
}

fn stack_is_clear<T>(stack: &RefCell<Vec<T>>) -> bool {
    stack.try_borrow().is_ok_and(|popups| popups.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_popup_must_close_before_readmission() {
        let stack = RefCell::new(vec!["tooltip", "dropdown"]);
        assert!(!stack_is_clear(&stack));
        stack.borrow_mut().pop();
        assert!(!stack_is_clear(&stack));
        stack.borrow_mut().pop();
        assert!(stack_is_clear(&stack));
        // The successful query must not retain a borrow across native/UI work.
        assert!(stack.try_borrow_mut().is_ok());
    }

    #[test]
    fn reentrant_stack_mutation_fails_closed_without_panicking() {
        let stack: RefCell<Vec<()>> = RefCell::new(Vec::new());
        let mutation = stack.borrow_mut();
        assert!(!stack_is_clear(&stack));
        drop(mutation);
        assert!(stack_is_clear(&stack));
    }
}
