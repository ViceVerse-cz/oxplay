// SPDX-License-Identifier: GPL-3.0-or-later
//! Windows release builds use the GUI subsystem so launching Oxplay does not
//! open a console window. Command-line diagnostics started from a terminal
//! reattach to that console; a startup failure that would otherwise vanish
//! with no console is also shown in a native message box.

/// Reattach standard output/error to the launching terminal, if any. A GUI
/// launch (Explorer, Start menu) has no parent console and stays silent.
pub fn attach_parent_console() {
    #[cfg(not(debug_assertions))]
    // SAFETY: plain Win32 call; failure (no parent console) is expected.
    unsafe {
        windows_sys::Win32::System::Console::AttachConsole(
            windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS,
        );
    }
}

/// Show a startup error that ends the process before any window exists.
pub fn report_startup_error(error: &dyn std::error::Error) {
    #[cfg(not(debug_assertions))]
    {
        let _ = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title("Oxplay could not start")
            .set_description(error.to_string())
            .set_buttons(rfd::MessageButtons::Ok)
            .show();
    }
    #[cfg(debug_assertions)]
    let _ = error;
}
