// SPDX-License-Identifier: GPL-3.0-or-later
//! Windows display-sleep assertion during observed playback.
//!
//! With `vo=libmpv` mpv owns no window, so its own screensaver inhibition never
//! runs. Mirror the macOS assertion: request the display only while playback
//! is observed Playing and unpaused, and release it when paused, stopped,
//! failed or when the player is destroyed. `SetThreadExecutionState` is
//! per-thread; the non-Send player always calls it from the UI thread.
use std::cell::Cell;
use windows_sys::Win32::System::Power::{
    ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED, SetThreadExecutionState,
};

#[derive(Default)]
pub(crate) struct DisplayRequest {
    active: Cell<bool>,
}
impl DisplayRequest {
    /// Idempotent and event-driven: the OS is called only on transitions.
    pub(crate) fn update(&self, playing: bool) -> bool {
        if self.active.get() != playing {
            let flags = if playing {
                ES_CONTINUOUS | ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED
            } else {
                ES_CONTINUOUS
            };
            // SAFETY: plain Win32 call without pointers. A zero return means
            // the request was not applied; report it as inactive.
            let applied = unsafe { SetThreadExecutionState(flags) } != 0;
            self.active.set(playing && applied);
        }
        self.active.get()
    }
}
impl Drop for DisplayRequest {
    fn drop(&mut self) {
        self.update(false);
    }
}
