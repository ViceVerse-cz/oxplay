// SPDX-License-Identifier: GPL-3.0-or-later
fn main() {
    #[cfg(target_os = "macos")]
    std::process::exit(serein_network::run_helper());
    #[cfg(not(target_os = "macos"))]
    std::process::exit(2);
}
