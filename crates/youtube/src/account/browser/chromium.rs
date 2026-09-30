// SPDX-License-Identifier: GPL-3.0-or-later
//! Chromium-family cookie reading and value decryption (macOS "v10" scheme).
//!
//! Values are AES-128-CBC encrypted with a key derived by
//! `PBKDF2-HMAC-SHA1(password, "saltysalt", 1003)` where the password is the
//! `<Vendor> Safe Storage` Keychain generic password; the IV is 16 spaces and a
//! `v10` prefix marks encrypted values. Databases at meta `version >= 24`
//! prepend a 32-byte SHA-256 of the host to the plaintext, which is stripped.
use super::{
    AccountError, BrowserKeychain, BrowserProfile, ChromiumInfo, SessionCookies, host_filter,
    host_params,
};
use crate::account::cookies::RawCookie;
use aes::cipher::{BlockModeDecrypt, KeyIvInit, block_padding::Pkcs7};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

/// The base "User Data" directory that holds the browser's profile folders.
fn base_dir(home: &Path, info: &ChromiumInfo) -> PathBuf {
    home.join("Library/Application Support")
        .join(info.app_support)
}

/// A directory is a selectable profile when it is named `Default` or
/// `Profile N` and contains a cookie database.
pub(super) fn discover_profiles(home: &Path, info: &ChromiumInfo) -> Vec<BrowserProfile> {
    let base = base_dir(home, info);
    let Ok(entries) = std::fs::read_dir(&base) else {
        return Vec::new();
    };
    let names = profile_names(&base);
    let mut profiles = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if name != "Default" && !name.starts_with("Profile ") {
            continue;
        }
        if profile_cookie_db(&base, &name).is_some() {
            let display_name = names
                .iter()
                .find(|(dir, _)| *dir == name)
                .map(|(_, label)| label.clone())
                .unwrap_or_else(|| name.clone());
            profiles.push(BrowserProfile {
                display_name,
                id: name,
            });
        }
    }
    // Default profile first, remaining profiles in stable name order.
    profiles.sort_by(|a, b| {
        (a.id != "Default")
            .cmp(&(b.id != "Default"))
            .then_with(|| a.id.cmp(&b.id))
    });
    profiles
}

/// Human profile names from the browser's `Local State` file
/// (`profile.info_cache.<dir>.name`). This is non-secret profile metadata; a
/// missing, oversized or malformed file simply falls back to directory names.
fn profile_names(base: &Path) -> Vec<(String, String)> {
    let Some(text) = super::read_small_text(&base.join("Local State")) else {
        return Vec::new();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let Some(cache) = json
        .get("profile")
        .and_then(|profile| profile.get("info_cache"))
        .and_then(serde_json::Value::as_object)
    else {
        return Vec::new();
    };
    cache
        .iter()
        .filter_map(|(dir, info)| {
            let label = info.get("name")?.as_str()?.trim();
            let label: String = label.chars().filter(|c| !c.is_control()).take(64).collect();
            (!label.is_empty()).then(|| (dir.clone(), label))
        })
        .collect()
}

/// Newer Chromium stores cookies at `<profile>/Network/Cookies`; older builds
/// used `<profile>/Cookies`.
fn profile_cookie_db(base: &Path, profile: &str) -> Option<PathBuf> {
    let network = base.join(profile).join("Network").join("Cookies");
    if network.is_file() {
        return Some(network);
    }
    let legacy = base.join(profile).join("Cookies");
    legacy.is_file().then_some(legacy)
}

pub(super) fn cookie_db(home: &Path, info: &ChromiumInfo, profile: &str) -> Option<PathBuf> {
    profile_cookie_db(&base_dir(home, info), profile)
}

pub(super) fn import(
    db: &Path,
    info: &ChromiumInfo,
    now_unix: u64,
    keychain: &dyn BrowserKeychain,
) -> Result<SessionCookies, AccountError> {
    let temp = super::copy_locked_sqlite(db)?;
    let conn = super::open_readonly(temp.path())?;
    let meta_version: i64 = conn
        .query_row("SELECT value FROM meta WHERE key='version'", [], |row| {
            row.get::<_, String>(0)
                .map(|v| v.parse::<i64>().unwrap_or(0))
        })
        .unwrap_or(0);
    let strip_hash = meta_version >= 24;

    // Only rows already restricted to the allowed hosts are selected, so no
    // unrelated cookie value is ever decrypted or returned.
    let sql = format!(
        "SELECT host_key, name, path, is_secure, is_httponly, expires_utc, encrypted_value, value \
         FROM cookies WHERE {}",
        host_filter("host_key")
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|_| AccountError::BrowserUnavailable)?;
    let mut rows = stmt
        .query(rusqlite::params_from_iter(host_params()))
        .map_err(|_| AccountError::BrowserUnavailable)?;

    // Derive the profile key lazily, only if an encrypted value is present, so
    // reading a profile with no relevant cookies never prompts for Keychain.
    let mut key: Option<Zeroizing<[u8; 16]>> = None;
    let mut entries = Vec::new();
    while let Some(row) = rows.next().map_err(|_| AccountError::BrowserUnavailable)? {
        let host: String = row.get(0).map_err(|_| AccountError::BrowserUnavailable)?;
        let name: String = row.get(1).map_err(|_| AccountError::BrowserUnavailable)?;
        let path: String = row.get(2).map_err(|_| AccountError::BrowserUnavailable)?;
        let is_secure: i64 = row.get(3).map_err(|_| AccountError::BrowserUnavailable)?;
        let is_httponly: i64 = row.get(4).map_err(|_| AccountError::BrowserUnavailable)?;
        let expires_utc: i64 = row.get(5).map_err(|_| AccountError::BrowserUnavailable)?;
        let encrypted: Vec<u8> = row.get(6).map_err(|_| AccountError::BrowserUnavailable)?;
        let plaintext: String = row.get(7).map_err(|_| AccountError::BrowserUnavailable)?;

        let value = if !encrypted.is_empty() {
            if key.is_none() {
                let password =
                    keychain.safe_storage_password(info.keychain_service, info.keychain_account)?;
                key = Some(derive_key(&password));
            }
            match decrypt_value(&encrypted, key.as_ref().expect("key derived"), strip_hash) {
                Some(value) => value,
                None => continue, // undecryptable/corrupt cookie: skip, never fail the import
            }
        } else if !plaintext.is_empty() {
            Zeroizing::new(plaintext)
        } else {
            continue;
        };

        entries.push(RawCookie {
            domain: host.clone(),
            subdomains: host.starts_with('.'),
            path,
            secure: is_secure != 0,
            http_only: is_httponly != 0,
            expires: chromium_expiry(expires_utc),
            name,
            value,
        });
    }
    drop(rows);
    drop(stmt);
    SessionCookies::import_entries(entries, now_unix)
}

fn derive_key(password: &[u8]) -> Zeroizing<[u8; 16]> {
    let mut key = Zeroizing::new([0u8; 16]);
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password, b"saltysalt", 1003, key.as_mut_slice());
    key
}

/// Decrypt a single `v10` value. Non-`v10` values are treated as plaintext.
fn decrypt_value(encrypted: &[u8], key: &[u8; 16], strip_hash: bool) -> Option<Zeroizing<String>> {
    let Some(ciphertext) = encrypted.strip_prefix(b"v10") else {
        // No known encryption prefix: interpret as UTF-8 plaintext if possible.
        return String::from_utf8(encrypted.to_vec())
            .ok()
            .map(Zeroizing::new);
    };
    if ciphertext.is_empty() || ciphertext.len() % 16 != 0 {
        return None;
    }
    let iv = [0x20u8; 16]; // 16 ASCII spaces
    let mut buffer = Zeroizing::new(ciphertext.to_vec());
    let plaintext = Aes128CbcDec::new_from_slices(key, &iv)
        .ok()?
        .decrypt_padded::<Pkcs7>(&mut buffer)
        .ok()?;
    // Chrome M118+ (meta version >= 24) prepends a 32-byte SHA-256 of the host.
    let plaintext = if strip_hash {
        plaintext.get(32..)?
    } else {
        plaintext
    };
    String::from_utf8(plaintext.to_vec())
        .ok()
        .map(Zeroizing::new)
}

/// Chromium stores expiry as microseconds since 1601-01-01 UTC. Convert to unix
/// seconds; `0` means a session cookie, and any pre-epoch value is reported as
/// expired.
fn chromium_expiry(expires_utc: i64) -> u64 {
    if expires_utc == 0 {
        return 0;
    }
    let unix = expires_utc / 1_000_000 - 11_644_473_600;
    if unix <= 0 { 1 } else { unix as u64 }
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn expiry_conversion_marks_epochs_correctly() {
        assert_eq!(chromium_expiry(0), 0);
        // 2001-01-01 in Chromium epoch microseconds -> a positive unix time.
        assert_eq!(chromium_expiry(12_622_780_800_000_000), 978_307_200);
        assert_eq!(chromium_expiry(1), 1); // pre-epoch collapses to "expired"
    }

    #[test]
    fn roundtrip_v10_with_and_without_host_hash() {
        use aes::cipher::{BlockModeEncrypt, KeyIvInit};
        type Enc = cbc::Encryptor<aes::Aes128>;
        let key = derive_key(b"synthetic-safe-storage-password");
        let iv = [0x20u8; 16];
        let make = |value: &str, prefix_hash: bool| -> Vec<u8> {
            let mut plain = Vec::new();
            if prefix_hash {
                plain.extend_from_slice(&[0u8; 32]);
            }
            plain.extend_from_slice(value.as_bytes());
            let ct = Enc::new_from_slices(&*key, &iv)
                .unwrap()
                .encrypt_padded_vec::<Pkcs7>(&plain);
            let mut out = b"v10".to_vec();
            out.extend_from_slice(&ct);
            out
        };
        let no_hash = make("SYNTHETIC_SAPISID", false);
        assert_eq!(
            decrypt_value(&no_hash, &key, false)
                .as_deref()
                .map(String::as_str),
            Some("SYNTHETIC_SAPISID")
        );
        let with_hash = make("SYNTHETIC_SAPISID", true);
        assert_eq!(
            decrypt_value(&with_hash, &key, true)
                .as_deref()
                .map(String::as_str),
            Some("SYNTHETIC_SAPISID")
        );
        // Wrong strip flag yields hash bytes / truncation, not the real value.
        assert_ne!(
            decrypt_value(&with_hash, &key, false)
                .as_deref()
                .map(String::as_str),
            Some("SYNTHETIC_SAPISID")
        );
    }
}
