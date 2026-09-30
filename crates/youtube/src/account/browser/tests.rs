// SPDX-License-Identifier: GPL-3.0-or-later
//! Synthetic-only tests. No real browser, profile, cookie or Keychain is
//! touched; every fixture database/file is generated in a temp directory and is
//! clearly labeled with SYNTHETIC, non-credential values.
use super::*;
use aes::cipher::{BlockModeEncrypt, KeyIvInit, block_padding::Pkcs7};
use std::fs;
use std::path::Path;

type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;

const SYNTHETIC_PASSWORD: &[u8] = b"SYNTHETIC-safe-storage-not-a-real-key";

struct FakeKeychain {
    deny: bool,
}
impl BrowserKeychain for FakeKeychain {
    fn safe_storage_password(
        &self,
        _service: &str,
        _account: &str,
    ) -> Result<Zeroizing<Vec<u8>>, AccountError> {
        if self.deny {
            Err(AccountError::BrowserPermissionDenied)
        } else {
            Ok(Zeroizing::new(SYNTHETIC_PASSWORD.to_vec()))
        }
    }
}

fn derive(password: &[u8]) -> [u8; 16] {
    let mut key = [0u8; 16];
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password, b"saltysalt", 1003, &mut key);
    key
}

/// Encrypt a synthetic value into a Chromium "v10" blob, optionally with the
/// 32-byte host-hash prefix used at meta version >= 24.
fn v10(key: &[u8; 16], value: &str, with_hash: bool) -> Vec<u8> {
    let iv = [0x20u8; 16];
    let mut plain = Vec::new();
    if with_hash {
        plain.extend_from_slice(&[0u8; 32]);
    }
    plain.extend_from_slice(value.as_bytes());
    let ct = Aes128CbcEnc::new_from_slices(key, &iv)
        .unwrap()
        .encrypt_padded_vec::<Pkcs7>(&plain);
    let mut out = b"v10".to_vec();
    out.extend_from_slice(&ct);
    out
}

fn chromium_micros(unix: i64) -> i64 {
    (unix + 11_644_473_600) * 1_000_000
}

struct ChromiumRow {
    host: &'static str,
    name: &'static str,
    encrypted: Vec<u8>,
    expires_unix: i64,
}

fn write_chromium_db(path: &Path, meta_version: i64, rows: &[ChromiumRow]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE meta(key TEXT NOT NULL, value TEXT);\
         CREATE TABLE cookies(host_key TEXT, name TEXT, path TEXT, is_secure INTEGER, \
         is_httponly INTEGER, expires_utc INTEGER, encrypted_value BLOB, value TEXT);",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO meta(key, value) VALUES('version', ?1)",
        [meta_version.to_string()],
    )
    .unwrap();
    for row in rows {
        conn.execute(
            "INSERT INTO cookies(host_key, name, path, is_secure, is_httponly, expires_utc, \
             encrypted_value, value) VALUES(?1, ?2, '/', 1, 1, ?3, ?4, '')",
            rusqlite::params![
                row.host,
                row.name,
                chromium_micros(row.expires_unix),
                row.encrypted
            ],
        )
        .unwrap();
    }
}

fn chrome_cookie_path(home: &Path) -> std::path::PathBuf {
    home.join("Library/Application Support/Google/Chrome/Default/Network/Cookies")
}

#[test]
fn chromium_decrypts_only_youtube_cookies_and_drops_expired() {
    let home = tempfile::tempdir().unwrap();
    let key = derive(SYNTHETIC_PASSWORD);
    let now: u64 = 2_000_000_000;
    let rows = [
        ChromiumRow {
            host: ".youtube.com",
            name: "SAPISID",
            encrypted: v10(&key, "SYNTHETIC_SAPISID", true),
            expires_unix: 2_500_000_000,
        },
        ChromiumRow {
            host: ".youtube.com",
            name: "__Secure-3PSID",
            encrypted: v10(&key, "SYNTHETIC_EXPIRED", true),
            expires_unix: 1_000, // already expired relative to `now`
        },
        ChromiumRow {
            host: ".google.com",
            name: "SID",
            encrypted: v10(&key, "SYNTHETIC_GOOGLE", true),
            expires_unix: 2_500_000_000,
        },
        ChromiumRow {
            host: "notyoutube.com",
            name: "SAPISID",
            encrypted: v10(&key, "SYNTHETIC_EVIL", true),
            expires_unix: 2_500_000_000,
        },
    ];
    write_chromium_db(&chrome_cookie_path(home.path()), 24, &rows);

    let jar = import_in(
        home.path(),
        BrowserKind::Chrome,
        "Default",
        now,
        &FakeKeychain { deny: false },
    )
    .unwrap();
    // Only the unexpired YouTube SAPISID survives; Google, expired and unrelated
    // hosts are never returned.
    let (header, _) = jar.request_values("/youtubei/v1/browse", now).unwrap();
    assert_eq!(&*header, "SAPISID=SYNTHETIC_SAPISID");
    assert!(!format!("{jar:?}").contains("SYNTHETIC_SAPISID"));
}

#[test]
fn chromium_version_below_24_has_no_host_hash() {
    let home = tempfile::tempdir().unwrap();
    let key = derive(SYNTHETIC_PASSWORD);
    let now: u64 = 2_000_000_000;
    let rows = [ChromiumRow {
        host: ".youtube.com",
        name: "SAPISID",
        encrypted: v10(&key, "SYNTHETIC_LEGACY", false),
        expires_unix: 2_500_000_000,
    }];
    write_chromium_db(&chrome_cookie_path(home.path()), 23, &rows);
    let jar = import_in(
        home.path(),
        BrowserKind::Chrome,
        "Default",
        now,
        &FakeKeychain { deny: false },
    )
    .unwrap();
    assert_eq!(
        &*jar.request_values("/youtubei/v1/browse", now).unwrap().0,
        "SAPISID=SYNTHETIC_LEGACY"
    );
}

#[test]
fn chromium_keychain_denial_maps_to_permission_error() {
    let home = tempfile::tempdir().unwrap();
    let key = derive(SYNTHETIC_PASSWORD);
    let rows = [ChromiumRow {
        host: ".youtube.com",
        name: "SAPISID",
        encrypted: v10(&key, "SYNTHETIC_SAPISID", true),
        expires_unix: 2_500_000_000,
    }];
    write_chromium_db(&chrome_cookie_path(home.path()), 24, &rows);
    assert!(matches!(
        import_in(
            home.path(),
            BrowserKind::Chrome,
            "Default",
            2_000_000_000,
            &FakeKeychain { deny: true },
        ),
        Err(AccountError::BrowserPermissionDenied)
    ));
}

#[test]
fn keychain_status_codes_map_to_typed_errors() {
    // User cancel, auth failure and "interaction not allowed" are denials; a
    // missing Safe Storage item (errSecItemNotFound) is simply unavailable.
    for status in [-128, -25293, -25308] {
        assert!(matches!(
            keychain_error(status),
            AccountError::BrowserPermissionDenied
        ));
    }
    assert!(matches!(
        keychain_error(-25300),
        AccountError::BrowserUnavailable
    ));
}

#[test]
fn unknown_profile_id_is_unavailable() {
    let home = tempfile::tempdir().unwrap();
    let key = derive(SYNTHETIC_PASSWORD);
    let rows = [ChromiumRow {
        host: ".youtube.com",
        name: "SAPISID",
        encrypted: v10(&key, "SYNTHETIC_SAPISID", true),
        expires_unix: 2_500_000_000,
    }];
    write_chromium_db(&chrome_cookie_path(home.path()), 24, &rows);
    assert!(matches!(
        import_in(
            home.path(),
            BrowserKind::Chrome,
            "Profile 99",
            2_000_000_000,
            &FakeKeychain { deny: false },
        ),
        Err(AccountError::BrowserUnavailable)
    ));
}

fn write_firefox_db(home: &Path, profile_dir: &str, name: &str) {
    let base = home.join("Library/Application Support/Firefox");
    let profile = base.join(profile_dir);
    fs::create_dir_all(&profile).unwrap();
    let ini = format!(
        "[Install0]\nDefault={profile_dir}\n\n[Profile0]\nName={name}\nIsRelative=1\nPath={profile_dir}\nDefault=1\n"
    );
    fs::write(base.join("profiles.ini"), ini).unwrap();
    let conn = rusqlite::Connection::open(profile.join("cookies.sqlite")).unwrap();
    conn.execute_batch(
        "CREATE TABLE moz_cookies(host TEXT, name TEXT, value TEXT, path TEXT, expiry INTEGER, \
         isSecure INTEGER, isHttpOnly INTEGER);",
    )
    .unwrap();
    let insert = |host: &str, name: &str, value: &str, expiry: i64| {
        conn.execute(
            "INSERT INTO moz_cookies(host, name, value, path, expiry, isSecure, isHttpOnly) \
             VALUES(?1, ?2, ?3, '/', ?4, 1, 1)",
            rusqlite::params![host, name, value, expiry],
        )
        .unwrap();
    };
    insert(
        ".youtube.com",
        "SAPISID",
        "SYNTHETIC_FFSAPISID",
        2_500_000_000,
    );
    insert(
        ".youtube.com",
        "__Secure-3PSID",
        "SYNTHETIC_FFEXPIRED",
        1_000,
    );
    insert(".google.com", "SID", "SYNTHETIC_FFGOOGLE", 2_500_000_000);
}

#[test]
fn firefox_reads_plaintext_youtube_cookies() {
    let home = tempfile::tempdir().unwrap();
    write_firefox_db(home.path(), "abcd.default-release", "default");
    let now: u64 = 2_000_000_000;
    let jar = import_in(
        home.path(),
        BrowserKind::Firefox,
        "abcd.default-release",
        now,
        &FakeKeychain { deny: false },
    )
    .unwrap();
    assert_eq!(
        &*jar.request_values("/youtubei/v1/browse", now).unwrap().0,
        "SAPISID=SYNTHETIC_FFSAPISID"
    );
}

#[test]
fn firefox_reads_uncheckpointed_wal_while_the_browser_holds_the_database() {
    let home = tempfile::tempdir().unwrap();
    let base = home.path().join("Library/Application Support/Firefox");
    let profile = base.join("Profiles/wal.default-release");
    fs::create_dir_all(&profile).unwrap();
    fs::write(
        base.join("profiles.ini"),
        "[Profile0]\nName=default-release\nIsRelative=1\nPath=Profiles/wal.default-release\nDefault=1\n",
    )
    .unwrap();
    // Simulate the running browser: WAL mode, no checkpoint, connection open.
    let live = rusqlite::Connection::open(profile.join("cookies.sqlite")).unwrap();
    live.pragma_update(None, "journal_mode", "WAL").unwrap();
    live.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
    live.execute_batch(
        "CREATE TABLE moz_cookies(host TEXT, name TEXT, value TEXT, path TEXT, expiry INTEGER, \
         isSecure INTEGER, isHttpOnly INTEGER);\
         INSERT INTO moz_cookies VALUES('.youtube.com', 'SAPISID', 'SYNTHETIC_WAL', '/', \
         2500000000, 1, 0);",
    )
    .unwrap();
    assert!(profile.join("cookies.sqlite-wal").exists());
    let now: u64 = 2_000_000_000;
    let jar = import_in(
        home.path(),
        BrowserKind::Firefox,
        "Profiles/wal.default-release",
        now,
        &FakeKeychain { deny: false },
    )
    .unwrap();
    assert_eq!(
        &*jar.request_values("/youtubei/v1/browse", now).unwrap().0,
        "SAPISID=SYNTHETIC_WAL"
    );
    // The live database is left untouched and still usable by the "browser".
    let count: i64 = live
        .query_row("SELECT COUNT(*) FROM moz_cookies", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

/// Encode one Safari cookie record matching the parser's expected layout.
fn safari_cookie(
    domain: &str,
    name: &str,
    path: &str,
    value: &str,
    flags: u32,
    expiration_unix: i64,
) -> Vec<u8> {
    let mac_time = (expiration_unix - 978_307_200) as f64;
    let header_len = 56u32;
    let url_off = header_len;
    let name_off = url_off + domain.len() as u32 + 1;
    let path_off = name_off + name.len() as u32 + 1;
    let value_off = path_off + path.len() as u32 + 1;
    let total = value_off + value.len() as u32 + 1;
    let mut record = Vec::new();
    record.extend_from_slice(&total.to_le_bytes()); // 0: size
    record.extend_from_slice(&0u32.to_le_bytes()); // 4
    record.extend_from_slice(&flags.to_le_bytes()); // 8: flags
    record.extend_from_slice(&0u32.to_le_bytes()); // 12
    record.extend_from_slice(&url_off.to_le_bytes()); // 16
    record.extend_from_slice(&name_off.to_le_bytes()); // 20
    record.extend_from_slice(&path_off.to_le_bytes()); // 24
    record.extend_from_slice(&value_off.to_le_bytes()); // 28
    record.extend_from_slice(&0u64.to_le_bytes()); // 32: end marker
    record.extend_from_slice(&mac_time.to_le_bytes()); // 40: expiration
    record.extend_from_slice(&0f64.to_le_bytes()); // 48: creation
    for text in [domain, name, path, value] {
        record.extend_from_slice(text.as_bytes());
        record.push(0);
    }
    assert_eq!(record.len() as u32, total);
    record
}

fn safari_binarycookies(cookies: &[Vec<u8>]) -> Vec<u8> {
    // One page holding all cookies.
    let mut page = Vec::new();
    page.extend_from_slice(&[0x00, 0x00, 0x01, 0x00]); // page header
    page.extend_from_slice(&(cookies.len() as u32).to_le_bytes());
    let records_start = 12 + cookies.len() as u32 * 4;
    let mut cursor = records_start;
    for cookie in cookies {
        page.extend_from_slice(&cursor.to_le_bytes());
        cursor += cookie.len() as u32;
    }
    page.extend_from_slice(&0u32.to_le_bytes()); // footer
    for cookie in cookies {
        page.extend_from_slice(cookie);
    }
    let mut file = Vec::new();
    file.extend_from_slice(b"cook");
    file.extend_from_slice(&1u32.to_be_bytes()); // page count
    file.extend_from_slice(&(page.len() as u32).to_be_bytes()); // page size
    file.extend_from_slice(&page);
    file
}

fn write_safari(home: &Path, bytes: &[u8]) -> std::path::PathBuf {
    let dir = home.join("Library/Containers/com.apple.Safari/Data/Library/Cookies");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Cookies.binarycookies");
    fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn safari_parses_binarycookies_and_filters_hosts() {
    let home = tempfile::tempdir().unwrap();
    let bytes = safari_binarycookies(&[
        safari_cookie(
            ".youtube.com",
            "SAPISID",
            "/",
            "SYNTHETIC_SFSAPISID",
            0x1,
            2_500_000_000,
        ),
        safari_cookie(
            ".google.com",
            "SID",
            "/",
            "SYNTHETIC_SFGOOGLE",
            0x1,
            2_500_000_000,
        ),
        safari_cookie(
            ".evil.com",
            "SAPISID",
            "/",
            "SYNTHETIC_SFEVIL",
            0x1,
            2_500_000_000,
        ),
    ]);
    write_safari(home.path(), &bytes);
    // The parser itself keeps only the exact allowed hosts (values of other
    // hosts are never extracted); the shared filter then keeps only YouTube.
    let extra = safari_binarycookies(&[
        safari_cookie(
            "mail.google.com",
            "SID",
            "/",
            "SYNTHETIC_SFMAIL",
            0x1,
            2_500_000_000,
        ),
        safari_cookie(
            "accounts.google.com",
            "LSID",
            "/",
            "SYNTHETIC_SFACCOUNTS",
            0x5,
            2_500_000_000,
        ),
    ]);
    let hosts: Vec<_> = safari::parse(&bytes)
        .unwrap()
        .into_iter()
        .chain(safari::parse(&extra).unwrap())
        .map(|cookie| (cookie.domain, cookie.secure, cookie.http_only))
        .collect();
    assert_eq!(
        hosts,
        [
            (".youtube.com".to_owned(), true, false),
            (".google.com".to_owned(), true, false),
            ("accounts.google.com".to_owned(), true, true),
        ]
    );
    let now: u64 = 2_000_000_000;
    let jar = import_in(
        home.path(),
        BrowserKind::Safari,
        "",
        now,
        &FakeKeychain { deny: false },
    )
    .unwrap();
    assert_eq!(
        &*jar.request_values("/youtubei/v1/browse", now).unwrap().0,
        "SAPISID=SYNTHETIC_SFSAPISID"
    );
}

#[test]
fn safari_permission_error_is_surfaced() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if unsafe { libc::geteuid() } == 0 {
            return; // root ignores permission bits; skip
        }
        let home = tempfile::tempdir().unwrap();
        let path = write_safari(home.path(), b"cook");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        assert!(matches!(
            import_in(
                home.path(),
                BrowserKind::Safari,
                "",
                2_000_000_000,
                &FakeKeychain { deny: false }
            ),
            Err(AccountError::BrowserPermissionDenied)
        ));
    }
}

#[test]
fn discovery_lists_installed_browsers_and_profiles_default_first() {
    let home = tempfile::tempdir().unwrap();
    let key = derive(SYNTHETIC_PASSWORD);
    let row = |name: &'static str| ChromiumRow {
        host: ".youtube.com",
        name,
        encrypted: v10(&key, "SYNTHETIC_SAPISID", true),
        expires_unix: 2_500_000_000,
    };
    // Chrome with two profiles (created out of order to prove Default sorts first).
    write_chromium_db(
        &home
            .path()
            .join("Library/Application Support/Google/Chrome/Profile 1/Network/Cookies"),
        24,
        &[row("SAPISID")],
    );
    write_chromium_db(&chrome_cookie_path(home.path()), 24, &[row("SAPISID")]);
    // A profile-like directory without a cookie database is not offered.
    fs::create_dir_all(
        home.path()
            .join("Library/Application Support/Google/Chrome/Profile 2"),
    )
    .unwrap();
    // Non-secret profile names come from the synthetic Local State metadata.
    fs::write(
        home.path()
            .join("Library/Application Support/Google/Chrome/Local State"),
        r#"{"profile":{"info_cache":{"Default":{"name":"SYNTHETIC Personal"},"Profile 1":{"name":"SYNTHETIC Work"}}}}"#,
    )
    .unwrap();
    write_firefox_db(home.path(), "abcd.default-release", "default");
    let applications = home.path().join("Applications");
    fs::create_dir_all(applications.join("Safari.app")).unwrap();

    let detected = detect_in(home.path(), &applications);
    let chrome = detected
        .iter()
        .find(|b| b.kind == BrowserKind::Chrome)
        .expect("chrome detected");
    assert_eq!(chrome.profiles.len(), 2);
    assert_eq!(chrome.profiles[0].id, "Default");
    assert_eq!(chrome.profiles[0].display_name, "SYNTHETIC Personal");
    assert_eq!(chrome.profiles[1].id, "Profile 1");
    assert_eq!(chrome.profiles[1].display_name, "SYNTHETIC Work");
    let firefox = detected
        .iter()
        .find(|b| b.kind == BrowserKind::Firefox)
        .expect("firefox detected");
    assert_eq!(firefox.profiles[0].id, "abcd.default-release");
    // Safari is detected from the installed application, without its container.
    assert!(detected.iter().any(|b| b.kind == BrowserKind::Safari));
    // A browser that is not installed never appears, and order follows ALL.
    assert!(!detected.iter().any(|b| b.kind == BrowserKind::Brave));
    assert_eq!(detected[0].kind, BrowserKind::Chrome);

    // Without Safari.app and with no other data, nothing is offered.
    let empty = tempfile::tempdir().unwrap();
    assert!(detect_in(empty.path(), &empty.path().join("Applications")).is_empty());
}

#[test]
fn firefox_default_profile_sorts_first() {
    let home = tempfile::tempdir().unwrap();
    let base = home.path().join("Library/Application Support/Firefox");
    for dir in ["Profiles/aaaa.other", "Profiles/zzzz.default-release"] {
        let profile = base.join(dir);
        fs::create_dir_all(&profile).unwrap();
        // Discovery only checks the cookie database exists; it never opens it.
        fs::write(profile.join("cookies.sqlite"), b"SYNTHETIC").unwrap();
    }
    fs::write(
        base.join("profiles.ini"),
        "[Profile1]\nName=other\nIsRelative=1\nPath=Profiles/aaaa.other\n\n\
         [Profile0]\nName=default-release\nIsRelative=1\nPath=Profiles/zzzz.default-release\n\n\
         [Install4F96D1932A9F858E]\nDefault=Profiles/zzzz.default-release\nLocked=1\n\n\
         [Profile2]\nName=missing\nIsRelative=1\nPath=Profiles/missing\n",
    )
    .unwrap();
    let profiles = firefox::discover_profiles(home.path());
    let ids: Vec<_> = profiles.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(
        ids,
        ["Profiles/zzzz.default-release", "Profiles/aaaa.other"]
    );
    assert_eq!(profiles[0].display_name, "default-release");
}

#[test]
fn chromium_filters_hosts_before_decryption_and_keychain() {
    struct PanicKeychain;
    impl BrowserKeychain for PanicKeychain {
        fn safe_storage_password(
            &self,
            _service: &str,
            _account: &str,
        ) -> Result<Zeroizing<Vec<u8>>, AccountError> {
            panic!("Keychain must not be queried without an allowed encrypted cookie");
        }
    }
    let home = tempfile::tempdir().unwrap();
    let key = derive(SYNTHETIC_PASSWORD);
    // Other sites, other Google subdomains and look-alike hosts are excluded by
    // the SQL host filter, so none of them is decrypted (the Keychain is not
    // even queried) or returned.
    let row = |host: &'static str| ChromiumRow {
        host,
        name: "SAPISID",
        encrypted: v10(&key, "SYNTHETIC_OTHER", true),
        expires_unix: 2_500_000_000,
    };
    let rows = [
        row(".example.com"),
        row("mail.google.com"),
        row("m.youtube.com"),
        row("youtube.com.evil.example"),
        row("notyoutube.com"),
    ];
    write_chromium_db(&chrome_cookie_path(home.path()), 24, &rows);
    assert!(matches!(
        import_in(
            home.path(),
            BrowserKind::Chrome,
            "Default",
            2_000_000_000,
            &PanicKeychain,
        ),
        Err(AccountError::MissingSessionCookies)
    ));
}

#[test]
fn missing_or_corrupt_sources_are_typed_errors() {
    let home = tempfile::tempdir().unwrap();
    // Safari: no cookie file at all.
    assert!(matches!(
        import_in(
            home.path(),
            BrowserKind::Safari,
            "",
            2_000_000_000,
            &FakeKeychain { deny: false }
        ),
        Err(AccountError::BrowserUnavailable)
    ));
    // Safari: a truncated/corrupt file.
    write_safari(home.path(), b"cook\x00\x00\x00\x05SYNTHETIC");
    assert!(matches!(
        import_in(
            home.path(),
            BrowserKind::Safari,
            "",
            2_000_000_000,
            &FakeKeychain { deny: false }
        ),
        Err(AccountError::BrowserUnavailable)
    ));
    // Chromium: a file that is not a SQLite database.
    let db = chrome_cookie_path(home.path());
    fs::create_dir_all(db.parent().unwrap()).unwrap();
    fs::write(&db, b"SYNTHETIC not a database").unwrap();
    assert!(matches!(
        import_in(
            home.path(),
            BrowserKind::Chrome,
            "Default",
            2_000_000_000,
            &FakeKeychain { deny: false }
        ),
        Err(AccountError::BrowserUnavailable)
    ));
}
