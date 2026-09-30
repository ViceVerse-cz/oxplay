// SPDX-License-Identifier: GPL-3.0-or-later
//! Firefox cookie reading. Values are stored in plaintext in `cookies.sqlite`;
//! the live database is copied (with its `-wal`) before opening because Firefox
//! keeps it locked.
use super::{
    AccountError, BrowserProfile, SessionCookies, host_filter, host_params, read_small_text,
};
use crate::account::cookies::RawCookie;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

fn base_dir(home: &Path) -> PathBuf {
    home.join("Library/Application Support/Firefox")
}

/// Discover Firefox profiles from `profiles.ini`, default profile first, keeping
/// only those whose `cookies.sqlite` exists.
pub(super) fn discover_profiles(home: &Path) -> Vec<BrowserProfile> {
    let base = base_dir(home);
    let Some(text) = read_small_text(&base.join("profiles.ini")) else {
        return Vec::new();
    };
    let sections = parse_ini(&text);
    // The [Install*] section names the default profile's path.
    let default_path = sections
        .iter()
        .find(|(name, _)| name.starts_with("Install"))
        .and_then(|(_, kv)| lookup(kv, "Default"));

    let mut profiles = Vec::new();
    for (name, kv) in &sections {
        if !name.starts_with("Profile") {
            continue;
        }
        let Some(path) = lookup(kv, "Path") else {
            continue;
        };
        if !cookie_db(home, &path).is_file() {
            continue;
        }
        let display = lookup(kv, "Name").unwrap_or_else(|| path.clone());
        let is_default = default_path.as_deref() == Some(path.as_str())
            || lookup(kv, "Default").as_deref() == Some("1");
        profiles.push((
            is_default,
            BrowserProfile {
                id: path,
                display_name: display,
            },
        ));
    }
    profiles.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
    profiles.into_iter().map(|(_, profile)| profile).collect()
}

/// Resolve a profile's cookie database. `Path` entries are relative to the
/// Firefox base directory (IsRelative=1, the norm) but an absolute path is
/// honored as-is.
pub(super) fn cookie_db(home: &Path, profile_path: &str) -> PathBuf {
    let candidate = Path::new(profile_path);
    let profile_dir = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        base_dir(home).join(candidate)
    };
    profile_dir.join("cookies.sqlite")
}

pub(super) fn import(db: &Path, now_unix: u64) -> Result<SessionCookies, AccountError> {
    let temp = super::copy_locked_sqlite(db)?;
    let conn = super::open_readonly(temp.path())?;
    let sql = format!(
        "SELECT host, name, path, isSecure, isHttpOnly, expiry, value \
         FROM moz_cookies WHERE {}",
        host_filter("host")
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|_| AccountError::BrowserUnavailable)?;
    let mut rows = stmt
        .query(rusqlite::params_from_iter(host_params()))
        .map_err(|_| AccountError::BrowserUnavailable)?;
    let mut entries = Vec::new();
    while let Some(row) = rows.next().map_err(|_| AccountError::BrowserUnavailable)? {
        let host: String = row.get(0).map_err(|_| AccountError::BrowserUnavailable)?;
        let name: String = row.get(1).map_err(|_| AccountError::BrowserUnavailable)?;
        let path: String = row.get(2).map_err(|_| AccountError::BrowserUnavailable)?;
        let is_secure: i64 = row.get(3).map_err(|_| AccountError::BrowserUnavailable)?;
        let is_httponly: i64 = row.get(4).map_err(|_| AccountError::BrowserUnavailable)?;
        let expiry: i64 = row.get(5).map_err(|_| AccountError::BrowserUnavailable)?;
        let value: String = row.get(6).map_err(|_| AccountError::BrowserUnavailable)?;
        if value.is_empty() {
            continue;
        }
        entries.push(RawCookie {
            domain: host.clone(),
            subdomains: host.starts_with('.'),
            path,
            secure: is_secure != 0,
            http_only: is_httponly != 0,
            expires: if expiry <= 0 { 0 } else { expiry as u64 },
            name,
            value: Zeroizing::new(value),
        });
    }
    drop(rows);
    drop(stmt);
    SessionCookies::import_entries(entries, now_unix)
}

/// Minimal INI parse: ordered `(section, [(key, value)])` pairs.
fn parse_ini(text: &str) -> Vec<(String, Vec<(String, String)>)> {
    let mut sections: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(['#', ';']) {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            sections.push((name.to_owned(), Vec::new()));
        } else if let Some((key, value)) = line.split_once('=')
            && let Some((_, kv)) = sections.last_mut()
        {
            kv.push((key.trim().to_owned(), value.trim().to_owned()));
        }
    }
    sections
}

fn lookup(kv: &[(String, String)], key: &str) -> Option<String> {
    kv.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v.clone())
}
