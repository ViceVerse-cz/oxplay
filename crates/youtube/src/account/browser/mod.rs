// SPDX-License-Identifier: GPL-3.0-or-later
//! "Sign in with your browser": read ONLY YouTube/Google sign-in cookies from a
//! single, user-selected installed browser profile and convert them into the
//! same [`SessionCookies`](crate::account::SessionCookies) representation the
//! Netscape file import produces, so both reach identical identity verification.
//!
//! This module never scans unrelated cookies, never touches other browsers than
//! the one the user picked, and only reads rows already restricted to the
//! allowed hosts before any Chromium value is decrypted. Decrypted values and
//! keys are held in zeroizing buffers. Real browser databases are never read in
//! tests; every test uses synthetic fixtures generated in a temporary directory.
// The readers compile (and are tested) everywhere, but only macOS wires them up.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]
use super::AccountError;
use super::cookies::SessionCookies;
use std::ffi::OsString;
use std::fs::File;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

mod chromium;
mod firefox;
mod safari;

#[cfg(test)]
mod tests;

/// A browser Oxplay knows how to read a YouTube session from on macOS. Other
/// platforms compile but report the feature as not supported yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowserKind {
    Chrome,
    Brave,
    Edge,
    Arc,
    Chromium,
    Vivaldi,
    Firefox,
    Safari,
}

/// One selectable profile inside an installed browser. `id` is an opaque token
/// (a Chromium profile directory name, a Firefox relative path, or empty for
/// Safari) that is always re-validated against fresh discovery before use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserProfile {
    pub id: String,
    pub display_name: String,
}

/// A detected browser and its readable profiles (default profile first).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledBrowser {
    pub kind: BrowserKind,
    pub display_name: String,
    pub profiles: Vec<BrowserProfile>,
}

/// Chromium-family layout metadata: where the profiles live and which macOS
/// Keychain generic password protects their value-encryption key.
struct ChromiumInfo {
    /// Sub-path under `~/Library/Application Support`.
    app_support: &'static str,
    /// Keychain generic-password service, e.g. `"Chrome Safe Storage"`.
    keychain_service: &'static str,
    /// Keychain generic-password account, e.g. `"Chrome"`.
    keychain_account: &'static str,
}

enum Family {
    Chromium(ChromiumInfo),
    Firefox,
    Safari,
}

impl BrowserKind {
    pub const ALL: [BrowserKind; 8] = [
        BrowserKind::Chrome,
        BrowserKind::Brave,
        BrowserKind::Edge,
        BrowserKind::Arc,
        BrowserKind::Chromium,
        BrowserKind::Vivaldi,
        BrowserKind::Firefox,
        BrowserKind::Safari,
    ];
    pub fn display_name(self) -> &'static str {
        match self {
            BrowserKind::Chrome => "Google Chrome",
            BrowserKind::Brave => "Brave",
            BrowserKind::Edge => "Microsoft Edge",
            BrowserKind::Arc => "Arc",
            BrowserKind::Chromium => "Chromium",
            BrowserKind::Vivaldi => "Vivaldi",
            BrowserKind::Firefox => "Firefox",
            BrowserKind::Safari => "Safari",
        }
    }
    /// Stable identifier for logging/marshaling. Not a secret.
    pub fn as_str(self) -> &'static str {
        match self {
            BrowserKind::Chrome => "chrome",
            BrowserKind::Brave => "brave",
            BrowserKind::Edge => "edge",
            BrowserKind::Arc => "arc",
            BrowserKind::Chromium => "chromium",
            BrowserKind::Vivaldi => "vivaldi",
            BrowserKind::Firefox => "firefox",
            BrowserKind::Safari => "safari",
        }
    }
    fn family(self) -> Family {
        match self {
            BrowserKind::Chrome => Family::Chromium(ChromiumInfo {
                app_support: "Google/Chrome",
                keychain_service: "Chrome Safe Storage",
                keychain_account: "Chrome",
            }),
            BrowserKind::Brave => Family::Chromium(ChromiumInfo {
                app_support: "BraveSoftware/Brave-Browser",
                keychain_service: "Brave Safe Storage",
                keychain_account: "Brave",
            }),
            BrowserKind::Edge => Family::Chromium(ChromiumInfo {
                app_support: "Microsoft Edge",
                keychain_service: "Microsoft Edge Safe Storage",
                keychain_account: "Microsoft Edge",
            }),
            BrowserKind::Arc => Family::Chromium(ChromiumInfo {
                app_support: "Arc/User Data",
                keychain_service: "Arc Safe Storage",
                keychain_account: "Arc",
            }),
            BrowserKind::Chromium => Family::Chromium(ChromiumInfo {
                app_support: "Chromium",
                keychain_service: "Chromium Safe Storage",
                keychain_account: "Chromium",
            }),
            BrowserKind::Vivaldi => Family::Chromium(ChromiumInfo {
                app_support: "Vivaldi",
                keychain_service: "Vivaldi Safe Storage",
                keychain_account: "Vivaldi",
            }),
            BrowserKind::Firefox => Family::Firefox,
            BrowserKind::Safari => Family::Safari,
        }
    }
}

/// Only these hosts are ever read from a cookie database, filtered in SQL (or,
/// for Safari, while parsing) before any decryption. Everything else is ignored.
const ALLOWED_HOSTS: [&str; 7] = [
    "youtube.com",
    ".youtube.com",
    "www.youtube.com",
    "google.com",
    ".google.com",
    "www.google.com",
    "accounts.google.com",
];
/// Cookie databases are small; refuse to copy anything implausibly large.
const MAX_DB_BYTES: u64 = 256 * 1024 * 1024;
/// Small metadata text files (profiles.ini, Chromium `Local State`) have a
/// tight bound; anything larger falls back to directory names.
const MAX_TEXT_BYTES: u64 = 4 * 1024 * 1024;

/// Reads the `<Vendor> Safe Storage` Keychain password used to derive a
/// Chromium profile's cookie key. Behind a trait so tests inject a synthetic
/// password and never touch the real Keychain (which cannot run in CI).
pub(crate) trait BrowserKeychain {
    fn safe_storage_password(
        &self,
        service: &str,
        account: &str,
    ) -> Result<Zeroizing<Vec<u8>>, AccountError>;
}

#[cfg(target_os = "macos")]
struct SystemBrowserKeychain;
#[cfg(target_os = "macos")]
impl BrowserKeychain for SystemBrowserKeychain {
    fn safe_storage_password(
        &self,
        service: &str,
        account: &str,
    ) -> Result<Zeroizing<Vec<u8>>, AccountError> {
        // macOS shows its own access prompt here; denial and cancellation are
        // reported as a permission error, never a silent failure. A missing
        // item (errSecItemNotFound, the browser never stored a key) and any
        // other failure are reported as unavailable.
        match security_framework::passwords::get_generic_password(service, account) {
            Ok(bytes) => Ok(Zeroizing::new(bytes)),
            Err(error) => Err(keychain_error(error.code())),
        }
    }
}

/// Map a Security framework `OSStatus` from reading the Safe Storage item.
fn keychain_error(status: i32) -> AccountError {
    const ERR_SEC_USER_CANCELED: i32 = -128;
    const ERR_SEC_AUTH_FAILED: i32 = -25293;
    const ERR_SEC_INTERACTION_NOT_ALLOWED: i32 = -25308;
    match status {
        ERR_SEC_USER_CANCELED | ERR_SEC_AUTH_FAILED | ERR_SEC_INTERACTION_NOT_ALLOWED => {
            AccountError::BrowserPermissionDenied
        }
        _ => AccountError::BrowserUnavailable,
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

/// Detect installed, readable browsers and their profiles. Filesystem probing
/// of profile metadata only: no cookie database is opened, no cookie value is
/// read, Safari's protected container is not touched, and no Keychain or
/// network access occurs. Other platforms report nothing.
pub fn detect_browsers() -> Vec<InstalledBrowser> {
    #[cfg(target_os = "macos")]
    {
        match home() {
            Some(home) => detect_in(&home, Path::new("/Applications")),
            None => Vec::new(),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

/// `applications` is the system applications folder (`/Applications`), used
/// only to decide whether Safari is installed without probing its container.
fn detect_in(home: &Path, applications: &Path) -> Vec<InstalledBrowser> {
    let mut detected = Vec::new();
    for kind in BrowserKind::ALL {
        let profiles = match kind.family() {
            Family::Chromium(info) => chromium::discover_profiles(home, &info),
            Family::Firefox => firefox::discover_profiles(home),
            Family::Safari => safari::discover_profiles(applications),
        };
        if !profiles.is_empty() {
            detected.push(InstalledBrowser {
                kind,
                display_name: kind.display_name().to_owned(),
                profiles,
            });
        }
    }
    detected
}

/// Import a YouTube session from one explicitly selected browser profile.
/// Runs the same filtering/validation as file import and returns candidate
/// cookies to be verified by the caller. macOS only.
pub fn import_browser_session(
    kind: BrowserKind,
    profile_id: &str,
    now_unix: u64,
) -> Result<SessionCookies, AccountError> {
    #[cfg(target_os = "macos")]
    {
        let home = home().ok_or(AccountError::BrowserUnavailable)?;
        import_in(&home, kind, profile_id, now_unix, &SystemBrowserKeychain)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (kind, profile_id, now_unix);
        Err(AccountError::BrowserUnsupported)
    }
}

fn import_in(
    home: &Path,
    kind: BrowserKind,
    profile_id: &str,
    now_unix: u64,
    keychain: &dyn BrowserKeychain,
) -> Result<SessionCookies, AccountError> {
    match kind.family() {
        Family::Chromium(info) => {
            // Re-validate the requested profile against fresh discovery so a
            // crafted id cannot escape the browser's own profile directory.
            let profile = chromium::discover_profiles(home, &info)
                .into_iter()
                .find(|profile| profile.id == profile_id)
                .ok_or(AccountError::BrowserUnavailable)?;
            let db = chromium::cookie_db(home, &info, &profile.id)
                .ok_or(AccountError::BrowserUnavailable)?;
            chromium::import(&db, &info, now_unix, keychain)
        }
        Family::Firefox => {
            let profile = firefox::discover_profiles(home)
                .into_iter()
                .find(|profile| profile.id == profile_id)
                .ok_or(AccountError::BrowserUnavailable)?;
            let db = firefox::cookie_db(home, &profile.id);
            firefox::import(&db, now_unix)
        }
        Family::Safari => {
            let path = safari::cookie_path(home).ok_or(AccountError::BrowserUnavailable)?;
            safari::import(&path, now_unix)
        }
    }
}

fn map_fs_err(error: std::io::Error) -> AccountError {
    match error.kind() {
        ErrorKind::PermissionDenied => AccountError::BrowserPermissionDenied,
        _ => AccountError::BrowserUnavailable,
    }
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(OsString::from(suffix));
    PathBuf::from(name)
}

fn bounded_copy(src: &Path, dst: &Path, cap: u64) -> Result<(), AccountError> {
    let file = File::open(src).map_err(map_fs_err)?;
    let mut reader = file.take(cap + 1);
    let mut out = File::create(dst).map_err(|_| AccountError::BrowserUnavailable)?;
    let copied = std::io::copy(&mut reader, &mut out).map_err(map_fs_err)?;
    if copied > cap {
        return Err(AccountError::BrowserUnavailable);
    }
    Ok(())
}

/// A private temporary copy of a (possibly locked) SQLite cookie database and
/// its write-ahead/shared-memory sidecars, deleted when dropped.
struct TempDb {
    _dir: tempfile::TempDir,
    path: PathBuf,
}
impl TempDb {
    fn path(&self) -> &Path {
        &self.path
    }
}

/// Copy a cookie SQLite DB (and any `-wal`/`-shm` sidecars) into a fresh private
/// temp directory before opening, since the browser holds a lock on the live
/// files. The temp copy is removed on drop.
fn copy_locked_sqlite(db: &Path) -> Result<TempDb, AccountError> {
    let dir = tempfile::Builder::new()
        .prefix("oxplay-browser-")
        .tempdir()
        .map_err(|_| AccountError::BrowserUnavailable)?;
    let target = dir.path().join("Cookies.sqlite");
    bounded_copy(db, &target, MAX_DB_BYTES)?;
    for suffix in ["-wal", "-shm"] {
        let side = with_suffix(db, suffix);
        if side.exists() {
            // Best effort: a missing/oversized sidecar just means we read the
            // committed database without the uncheckpointed tail.
            let _ = bounded_copy(&side, &with_suffix(&target, suffix), MAX_DB_BYTES);
        }
    }
    Ok(TempDb {
        _dir: dir,
        path: target,
    })
}

fn open_readonly(path: &Path) -> Result<rusqlite::Connection, AccountError> {
    use rusqlite::OpenFlags;
    rusqlite::Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| AccountError::BrowserUnavailable)
}

/// `<column> IN (?, ?, ...)` for the allowed hosts, matched by exact bind
/// parameters so no `LIKE` wildcard can widen the host scope.
fn host_filter(column: &str) -> String {
    let marks = vec!["?"; ALLOWED_HOSTS.len()].join(",");
    format!("{column} IN ({marks})")
}

fn host_params() -> impl Iterator<Item = &'static str> {
    ALLOWED_HOSTS.into_iter()
}

/// Read a small metadata text file with a tight size bound.
fn read_small_text(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let mut text = String::new();
    file.take(MAX_TEXT_BYTES + 1)
        .read_to_string(&mut text)
        .ok()?;
    if text.len() as u64 > MAX_TEXT_BYTES {
        return None;
    }
    Some(text)
}
