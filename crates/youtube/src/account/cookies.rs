//! Explicit Netscape-file import only. No browser/profile discovery.
use super::AccountError;
use std::{collections::BTreeMap, fmt};
use zeroize::Zeroizing;

pub const MAX_COOKIE_BYTES: usize = 4 * 1024 * 1024;
const MAX_COOKIE_ENTRIES: usize = 10_000;
const AUTH_NAMES: &[&str] = &[
    "SAPISID",
    "APISID",
    "SID",
    "HSID",
    "SSID",
    "LOGIN_INFO",
    "__Secure-1PAPISID",
    "__Secure-3PAPISID",
    "__Secure-1PSID",
    "__Secure-3PSID",
    "__Secure-1PSIDTS",
    "__Secure-3PSIDTS",
    "__Secure-1PSIDCC",
    "__Secure-3PSIDCC",
];
struct Cookie {
    domain: String,
    subdomains: bool,
    path: String,
    secure: bool,
    http_only: bool,
    expires: u64,
    name: String,
    value: Zeroizing<String>,
}
/// A parsed candidate credential jar. Possession does not mean account connection.
/// Values are wiped on drop; transport libraries can retain temporary copies.
pub struct SessionCookies {
    cookies: Vec<Cookie>,
}
impl fmt::Debug for SessionCookies {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "SessionCookies([redacted]; {} retained entries)",
            self.cookies.len()
        )
    }
}
impl SessionCookies {
    /// Conservative playback lifetime: every retained session cookie must still
    /// be current, and SAPISID must cover the actual HTTPS YouTube watch path.
    /// Returning an expiry conveys no secret; session cookies (expiry 0) remain
    /// revocable by explicit disconnect/provider rejection.
    pub(super) fn playback_expiry(
        &self,
        now: std::time::SystemTime,
    ) -> Result<Option<std::time::SystemTime>, AccountError> {
        let now_unix = now
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| AccountError::InvalidInput)?
            .as_secs();
        if self
            .cookies
            .iter()
            .any(|cookie| cookie.expires != 0 && cookie.expires <= now_unix)
            || !self
                .matching("/watch", now_unix)
                .iter()
                .any(|cookie| cookie.name == "SAPISID")
        {
            return Err(AccountError::SessionExpired);
        }
        self.cookies
            .iter()
            .filter(|cookie| cookie.expires != 0)
            .map(|cookie| cookie.expires)
            .min()
            .map(|expiry| {
                std::time::UNIX_EPOCH
                    .checked_add(std::time::Duration::from_secs(expiry))
                    .ok_or(AccountError::InvalidCookieFile)
            })
            .transpose()
    }
    pub fn import_netscape(bytes: Zeroizing<Vec<u8>>, now_unix: u64) -> Result<Self, AccountError> {
        if bytes.len() > MAX_COOKIE_BYTES {
            return Err(AccountError::ImportTooLarge);
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| AccountError::InvalidCookieFile)?;
        if !(text.starts_with("# Netscape HTTP Cookie File")
            || text.starts_with("# HTTP Cookie File"))
        {
            return Err(AccountError::InvalidCookieFile);
        }
        let mut retained = BTreeMap::new();
        let mut entries = 0;
        for raw in text.lines() {
            let raw = raw.strip_suffix('\r').unwrap_or(raw);
            let (line, http_only) = if let Some(line) = raw.strip_prefix("#HttpOnly_") {
                (line, true)
            } else {
                (raw, false)
            };
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            entries += 1;
            if entries > MAX_COOKIE_ENTRIES {
                return Err(AccountError::ImportTooLarge);
            }
            if line.len() > 16_384 {
                return Err(AccountError::InvalidCookieFile);
            }
            let fields: Vec<_> = line.split('\t').collect();
            if fields.len() != 7 {
                return Err(AccountError::InvalidCookieFile);
            }
            let domain = fields[0].strip_prefix('.').unwrap_or(fields[0]);
            if !domain.is_ascii() || domain.is_empty() || domain.contains(['/', ':', '@', '\\']) {
                return Err(AccountError::InvalidCookieFile);
            }
            let domain = domain.to_ascii_lowercase();
            let subdomains = parse_bool(fields[1])?;
            let secure = parse_bool(fields[3])?;
            let expires = fields[4]
                .parse::<u64>()
                .map_err(|_| AccountError::InvalidCookieFile)?;
            // Unrelated Google cookies are never imported or sent to YouTube.
            if !["youtube.com", "www.youtube.com"].contains(&domain.as_str())
                || !AUTH_NAMES.contains(&fields[5])
            {
                continue;
            }
            if !fields[2].starts_with('/')
                || fields[2].len() > 2048
                || !fields[2].is_ascii()
                || fields[2]
                    .bytes()
                    .any(|c| c <= 0x20 || c == 0x7f || c == b';')
            {
                return Err(AccountError::InvalidCookieFile);
            }
            if fields[5].starts_with("__Secure-") && !secure {
                return Err(AccountError::InvalidCookieFile);
            }
            if fields[6].is_empty()
                || fields[6].len() > 4096
                || fields[6]
                    .bytes()
                    .any(|c| !(0x21..=0x7e).contains(&c) || matches!(c, b';' | b'"' | b'\\' | b','))
            {
                return Err(AccountError::InvalidCookieFile);
            }
            if expires != 0 && expires <= now_unix {
                continue;
            }
            let key = (domain.clone(), fields[2].to_owned(), fields[5].to_owned());
            if retained.contains_key(&key) {
                return Err(AccountError::AmbiguousCookies);
            }
            retained.insert(
                key,
                Cookie {
                    domain,
                    subdomains,
                    path: fields[2].to_owned(),
                    secure,
                    http_only,
                    expires,
                    name: fields[5].to_owned(),
                    value: Zeroizing::new(fields[6].to_owned()),
                },
            );
        }
        let jar = Self {
            cookies: retained.into_values().collect(),
        };
        if jar.cookies.is_empty() {
            return Err(AccountError::MissingSessionCookies);
        }
        Ok(jar)
    }
    /// Explicit export of filtered credentials for the protected vault. Never log or persist plainly.
    pub fn export_for_vault(&self) -> Zeroizing<Vec<u8>> {
        let mut bytes = Zeroizing::new(b"# Netscape HTTP Cookie File\n".to_vec());
        for cookie in &self.cookies {
            if cookie.http_only {
                bytes.extend_from_slice(b"#HttpOnly_");
            }
            if cookie.subdomains {
                bytes.push(b'.');
            }
            bytes.extend_from_slice(cookie.domain.as_bytes());
            bytes.extend_from_slice(if cookie.subdomains {
                b"\tTRUE\t"
            } else {
                b"\tFALSE\t"
            });
            bytes.extend_from_slice(cookie.path.as_bytes());
            bytes.extend_from_slice(if cookie.secure {
                b"\tTRUE\t"
            } else {
                b"\tFALSE\t"
            });
            bytes.extend_from_slice(cookie.expires.to_string().as_bytes());
            bytes.push(b'\t');
            bytes.extend_from_slice(cookie.name.as_bytes());
            bytes.push(b'\t');
            bytes.extend_from_slice(cookie.value.as_bytes());
            bytes.push(b'\n');
        }
        bytes
    }
    fn matching(&self, path: &str, now: u64) -> Vec<&Cookie> {
        let mut cookies: Vec<_> = self
            .cookies
            .iter()
            .filter(|c| {
                let domain_ok =
                    c.domain == "www.youtube.com" || (c.domain == "youtube.com" && c.subdomains);
                let path_ok = path == c.path
                    || (path.starts_with(&c.path)
                        && (c.path.ends_with('/')
                            || path.as_bytes().get(c.path.len()) == Some(&b'/')));
                domain_ok && path_ok && (c.expires == 0 || c.expires > now)
            })
            .collect();
        cookies.sort_by_key(|c| std::cmp::Reverse(c.path.len()));
        cookies
    }
    pub(super) fn request_values(
        &self,
        path: &str,
        now: u64,
    ) -> Result<(Zeroizing<String>, Zeroizing<String>), AccountError> {
        // This method only constructs cookies for HTTPS www.youtube.com, never arbitrary destinations.
        let matching = self.matching(path, now);
        let sid = matching
            .iter()
            .find(|c| c.name == "SAPISID")
            .ok_or(AccountError::SessionExpired)?;
        let mut header = Zeroizing::new(String::new());
        for cookie in &matching {
            if !header.is_empty() {
                header.push_str("; ");
            }
            header.push_str(&cookie.name);
            header.push('=');
            header.push_str(&cookie.value);
        }
        if header.len() > 64 * 1024 {
            return Err(AccountError::ImportTooLarge);
        }
        Ok((header, Zeroizing::new(sid.value.to_string())))
    }
}
fn parse_bool(value: &str) -> Result<bool, AccountError> {
    match value {
        "TRUE" => Ok(true),
        "FALSE" => Ok(false),
        _ => Err(AccountError::InvalidCookieFile),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(lines: &str) -> Result<SessionCookies, AccountError> {
        SessionCookies::import_netscape(
            Zeroizing::new(format!("# Netscape HTTP Cookie File\n{lines}").into_bytes()),
            100,
        )
    }
    #[test]
    fn filters_domains_expiry_names_and_preserves_rules() {
        let jar = parse("#HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t200\tSAPISID\tsynthetic\n.google.com\tTRUE\t/\tTRUE\t200\tSID\texcluded\n.youtube.com\tTRUE\t/\tTRUE\t99\tSID\texpired\n.youtube.com\tTRUE\t/\tTRUE\t200\tPREF\texcluded\n").unwrap();
        assert_eq!(jar.cookies.len(), 1);
        assert_eq!(
            &*jar.request_values("/youtubei/v1/browse", 100).unwrap().0,
            "SAPISID=synthetic"
        );
        assert!(jar.request_values("/youtubei/v1/browse", 201).is_err());
        let exported = jar.export_for_vault();
        assert!(!String::from_utf8_lossy(&exported).contains("excluded"));
        assert_eq!(
            SessionCookies::import_netscape(exported, 100)
                .unwrap()
                .cookies
                .len(),
            1
        );
        assert!(!format!("{jar:?}").contains("synthetic"));
    }
    #[test]
    fn rejects_injection_duplicates_and_bad_flags() {
        for line in [
            ".youtube.com\tTRUE\t/\tTRUE\t200\tSAPISID\ta; SID=bad",
            ".youtube.com\tTRUE\t/\tFALSE\t200\t__Secure-3PSID\tfake",
            ".youtube.com\tmaybe\t/\tTRUE\t200\tSAPISID\tfake",
            ".youtube.com\tTRUE\t/\tTRUE\t200\tSAPISID\tfake\rbroken",
        ] {
            assert!(parse(line).is_err());
        }
        let duplicate = ".youtube.com\tTRUE\t/\tTRUE\t200\tSAPISID\tfake\n.youtube.com\tTRUE\t/\tTRUE\t200\tSAPISID\tfake";
        assert!(matches!(
            parse(duplicate),
            Err(AccountError::AmbiguousCookies)
        ));
    }
    #[test]
    fn host_only_and_path_boundaries_are_not_widened() {
        let jar = parse("youtube.com\tFALSE\t/\tTRUE\t200\tSAPISID\tfake").unwrap();
        assert!(jar.request_values("/youtubei/v1/browse", 100).is_err());
        let jar = parse("www.youtube.com\tFALSE\t/youtubei\tTRUE\t200\tSAPISID\tfake").unwrap();
        assert!(jar.request_values("/youtubei/v1/browse", 100).is_ok());
        assert!(jar.request_values("/youtubeix/v1/browse", 100).is_err());
    }
    #[test]
    fn input_bounds_are_enforced() {
        assert!(matches!(
            SessionCookies::import_netscape(Zeroizing::new(vec![0; MAX_COOKIE_BYTES + 1]), 100),
            Err(AccountError::ImportTooLarge)
        ));
        let entries =
            "example.com\tFALSE\t/\tFALSE\t0\tignored\tfake\n".repeat(MAX_COOKIE_ENTRIES + 1);
        assert!(matches!(parse(&entries), Err(AccountError::ImportTooLarge)));
    }
    #[test]
    fn generated_import_mutations_preserve_retained_cookie_invariants() {
        // Bounded deterministic mutation corpus, not real credentials. Includes
        // valid candidates as well as malformed UTF-8, tabs, line boundaries,
        // expiry, origin, flag, name and value changes.
        let original = b"# Netscape HTTP Cookie File\n.youtube.com\tTRUE\t/\tTRUE\t200\tSAPISID\tsynthetic-generated-value\n";
        let mut accepted = 0;
        let mut rejected = 0;
        for seed in 0..1024u64 {
            let mut bytes = original.to_vec();
            let mut random = seed + 1;
            for _ in 0..seed % 7 {
                random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
                let index = random as usize % bytes.len();
                bytes[index] = (random >> 32) as u8;
            }
            let Ok(jar) = SessionCookies::import_netscape(Zeroizing::new(bytes), 100) else {
                rejected += 1;
                continue;
            };
            accepted += 1;
            for cookie in &jar.cookies {
                assert!(["youtube.com", "www.youtube.com"].contains(&cookie.domain.as_str()));
                assert!(AUTH_NAMES.contains(&cookie.name.as_str()));
                assert!(cookie.expires == 0 || cookie.expires > 100);
                assert!(cookie.path.starts_with('/') && cookie.path.is_ascii());
                assert!(cookie.value.len() <= 4096 && !cookie.value.is_empty());
                assert!(cookie.value.bytes().all(
                    |b| (0x21..=0x7e).contains(&b) && !matches!(b, b';' | b'"' | b'\\' | b',')
                ));
                assert!(!cookie.name.starts_with("__Secure-") || cookie.secure);
            }
            assert!(!format!("{jar:?}").contains("synthetic-generated-value"));
        }
        assert!(
            accepted > 100 && rejected > 100,
            "corpus must exercise both acceptance and rejection"
        );
    }
}
