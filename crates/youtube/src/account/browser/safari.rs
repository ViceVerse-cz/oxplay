// SPDX-License-Identifier: GPL-3.0-or-later
//! Safari `Cookies.binarycookies` reading. The format is plaintext but the file
//! lives inside Safari's container and requires Full Disk Access; a permission
//! error is reported so the UI can tell the user how to grant access.
use super::{ALLOWED_HOSTS, AccountError, BrowserProfile, SessionCookies, map_fs_err};
use crate::account::cookies::RawCookie;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

/// Read at most this many bytes from the cookie file.
const MAX_BINARYCOOKIES_BYTES: u64 = 32 * 1024 * 1024;
/// Seconds between the Unix epoch (1970) and the Mac absolute epoch (2001).
const MAC_EPOCH_OFFSET: i64 = 978_307_200;

fn container_path(home: &Path) -> PathBuf {
    home.join("Library/Containers/com.apple.Safari/Data/Library/Cookies/Cookies.binarycookies")
}

fn legacy_path(home: &Path) -> PathBuf {
    home.join("Library/Cookies/Cookies.binarycookies")
}

/// Safari is a single profile. It is offered when the application is installed;
/// detection deliberately does not probe Safari's protected container (which
/// could trigger a macOS privacy prompt before the user asked for anything).
/// An unreadable cookie file is reported at sign-in with Full Disk Access
/// guidance.
pub(super) fn discover_profiles(applications: &Path) -> Vec<BrowserProfile> {
    if applications.join("Safari.app").is_dir() {
        vec![BrowserProfile {
            id: String::new(),
            display_name: "Safari".to_owned(),
        }]
    } else {
        Vec::new()
    }
}

pub(super) fn cookie_path(home: &Path) -> Option<PathBuf> {
    let container = container_path(home);
    if container.exists() {
        return Some(container);
    }
    let legacy = legacy_path(home);
    if legacy.exists() {
        return Some(legacy);
    }
    // Prefer the container path for the read attempt so a permission error is
    // surfaced with actionable guidance rather than a bare "not found".
    Some(container)
}

pub(super) fn import(path: &Path, now_unix: u64) -> Result<SessionCookies, AccountError> {
    let file = File::open(path).map_err(map_fs_err)?;
    // The raw file holds every Safari cookie value; wipe it once parsed. Reserve
    // the expected size up front so growth does not leave unwiped copies.
    let expected = file
        .metadata()
        .map(|meta| meta.len().min(MAX_BINARYCOOKIES_BYTES) as usize)
        .unwrap_or(0);
    let mut bytes = Zeroizing::new(Vec::with_capacity(expected + 1));
    file.take(MAX_BINARYCOOKIES_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(map_fs_err)?;
    if bytes.len() as u64 > MAX_BINARYCOOKIES_BYTES {
        return Err(AccountError::BrowserUnavailable);
    }
    let entries = parse(&bytes)?;
    SessionCookies::import_entries(entries, now_unix)
}

fn be_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}
fn le_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}
fn le_f64(bytes: &[u8], at: usize) -> Option<f64> {
    Some(f64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

/// Parse the whole file into candidate cookies restricted to allowed hosts.
/// Malformed structure is a typed error; individual unparseable cookies are
/// skipped rather than aborting the import.
pub(super) fn parse(bytes: &[u8]) -> Result<Vec<RawCookie>, AccountError> {
    let malformed = || AccountError::BrowserUnavailable;
    if bytes.len() < 8 || &bytes[0..4] != b"cook" {
        return Err(malformed());
    }
    let page_count = be_u32(bytes, 4).ok_or_else(malformed)? as usize;
    if page_count > 100_000 {
        return Err(malformed());
    }
    let mut offset = 8;
    let mut page_sizes = Vec::with_capacity(page_count);
    for _ in 0..page_count {
        page_sizes.push(be_u32(bytes, offset).ok_or_else(malformed)? as usize);
        offset += 4;
    }
    let mut entries = Vec::new();
    let mut page_start = offset;
    for size in page_sizes {
        let end = page_start.checked_add(size).ok_or_else(malformed)?;
        let page = bytes.get(page_start..end).ok_or_else(malformed)?;
        parse_page(page, &mut entries);
        page_start = end;
    }
    Ok(entries)
}

fn parse_page(page: &[u8], entries: &mut Vec<RawCookie>) {
    // Page header 0x00000100, then a little-endian cookie count and offsets.
    let Some(count) = le_u32(page, 4) else { return };
    let count = count as usize;
    if count > 100_000 {
        return;
    }
    for index in 0..count {
        let Some(cookie_offset) = le_u32(page, 8 + index * 4) else {
            return;
        };
        if let Some(cookie) = parse_cookie(page, cookie_offset as usize) {
            entries.push(cookie);
        }
    }
}

fn parse_cookie(page: &[u8], start: usize) -> Option<RawCookie> {
    let record = page.get(start..)?;
    let size = le_u32(record, 0)? as usize;
    let record = record.get(..size)?;
    let flags = le_u32(record, 8)?;
    let url_off = le_u32(record, 16)? as usize;
    let name_off = le_u32(record, 20)? as usize;
    let path_off = le_u32(record, 24)? as usize;
    let value_off = le_u32(record, 28)? as usize;
    let expiration = le_f64(record, 40)?;

    let domain = cstr(record, url_off)?;
    if !host_allowed(&domain) {
        return None;
    }
    let name = cstr(record, name_off)?;
    let path = cstr(record, path_off)?;
    let value = cstr(record, value_off)?;
    if value.is_empty() {
        return None;
    }
    let expires = if expiration <= 0.0 {
        0
    } else {
        let unix = expiration as i64 + MAC_EPOCH_OFFSET;
        if unix <= 0 { 1 } else { unix as u64 }
    };
    Some(RawCookie {
        domain: domain.clone(),
        subdomains: domain.starts_with('.'),
        path,
        secure: flags & 0x1 != 0,
        http_only: flags & 0x4 != 0,
        expires,
        name,
        value: Zeroizing::new(value),
    })
}

/// Null-terminated ASCII string starting at `at` within a cookie record.
fn cstr(record: &[u8], at: usize) -> Option<String> {
    let slice = record.get(at..)?;
    let end = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
    let text = std::str::from_utf8(&slice[..end]).ok()?;
    Some(text.to_owned())
}

/// The same exact host set the SQL filters use for Chromium/Firefox; other
/// Google subdomains (mail, drive, ...) are never read.
fn host_allowed(domain: &str) -> bool {
    ALLOWED_HOSTS.contains(&domain.to_ascii_lowercase().as_str())
}
