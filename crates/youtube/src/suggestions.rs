// SPDX-License-Identifier: GPL-3.0-or-later
//! Anonymous YouTube search completions for text the user is typing.
//! One exact HTTPS origin, no cookies, redirects, proxies or retries, short
//! timeouts and a bounded response. The caller owns scheduling/cancellation.
use oxplay_core::ProviderError;
use serde_json::Value;
use std::time::Duration;
use url::Url;

/// Exact completion origin. Observed 2026-09-30: with `client=firefox` it
/// answers a plain JSON array `["query",["completion",…],[],{…}]`; `oe=utf-8`
/// selects UTF-8 (without it the body is ISO-8859-1).
pub const SUGGESTION_HOST: &str = "suggestqueries-clients6.youtube.com";
pub const MAX_RESPONSE_BYTES: usize = 64 * 1024;
pub const MAX_SUGGESTIONS: usize = 10;
pub const MAX_QUERY_CHARS: usize = 200;

/// Returns `None` for text that must not leave the device as a completion
/// query: empty, oversized, control characters, or anything URL-like (a
/// pasted link can carry session or tracking tokens).
pub fn request_url(query: &str) -> Option<Url> {
    let query = query.trim();
    let count = query.chars().count();
    if count == 0 || count > MAX_QUERY_CHARS || query.chars().any(char::is_control) {
        return None;
    }
    let lower = query.to_lowercase();
    if lower.contains("://")
        || lower.starts_with("www.")
        || lower.starts_with("https:")
        || lower.starts_with("http:")
        || lower.contains("youtube.com/")
        || lower.contains("youtu.be/")
    {
        return None;
    }
    let mut url = Url::parse(&format!("https://{SUGGESTION_HOST}/complete/search")).ok()?;
    url.query_pairs_mut()
        .append_pair("client", "firefox")
        .append_pair("ds", "yt")
        .append_pair("oe", "utf-8")
        .append_pair("ie", "utf-8")
        .append_pair("q", query);
    allowed(&url).then_some(url)
}

fn allowed(url: &Url) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some(SUGGESTION_HOST)
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && url.path() == "/complete/search"
}

/// A dedicated anonymous client: no cookie store (reqwest's cookie feature is
/// not enabled in this workspace), no ambient proxy, no redirects/retries.
pub fn client() -> Result<reqwest::Client, ProviderError> {
    reqwest::Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(5))
        .pool_max_idle_per_host(1)
        .user_agent("Oxplay/0.1 (experimental native YouTube client)")
        .build()
        .map_err(|_| ProviderError::Offline)
}

/// Dropping this future cancels the request.
pub async fn fetch(client: &reqwest::Client, query: &str) -> Result<Vec<String>, ProviderError> {
    let url = request_url(query).ok_or(ProviderError::InvalidInput)?;
    let mut response = client
        .get(url)
        .header("Accept", "application/json, text/javascript")
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                ProviderError::Timeout
            } else {
                ProviderError::Offline
            }
        })?;
    match response.status().as_u16() {
        200 => {}
        429 => return Err(ProviderError::RateLimited),
        // Includes redirects, which are never followed.
        _ => return Err(ProviderError::Unavailable),
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_RESPONSE_BYTES as u64)
    {
        return Err(ProviderError::OutputTooLarge);
    }
    let latin1 = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().contains("iso-8859-1"));
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| ProviderError::Offline)? {
        if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(ProviderError::OutputTooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    parse(&bytes, latin1)
}

/// Accepts the JSON form (`["q",["a","b"],…]`) and, defensively, the JSONP
/// `window.google.ac.h(["q",[["a",0,[…]],…],…])` form. Invalid entries are
/// skipped; at most ten trimmed, case-insensitively unique completions remain.
pub fn parse(bytes: &[u8], latin1: bool) -> Result<Vec<String>, ProviderError> {
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(ProviderError::OutputTooLarge);
    }
    let text = if latin1 {
        bytes.iter().map(|&byte| char::from(byte)).collect()
    } else {
        String::from_utf8(bytes.to_vec()).map_err(|_| ProviderError::MalformedOutput)?
    };
    let mut body = text.trim();
    if let Some(inner) = body
        .strip_prefix("window.google.ac.h(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        body = inner;
    }
    let value: Value = serde_json::from_str(body).map_err(|_| ProviderError::MalformedOutput)?;
    let items = value
        .as_array()
        .and_then(|array| array.get(1))
        .and_then(Value::as_array)
        .ok_or(ProviderError::MalformedOutput)?;
    let mut seen = std::collections::HashSet::new();
    let mut suggestions = Vec::new();
    for item in items {
        let text = match item {
            Value::String(text) => Some(text.as_str()),
            Value::Array(parts) => parts.first().and_then(Value::as_str),
            _ => None,
        };
        let Some(text) = text.map(str::trim) else {
            continue;
        };
        let count = text.chars().count();
        if count == 0 || count > MAX_QUERY_CHARS || text.chars().any(char::is_control) {
            continue;
        }
        if seen.insert(text.to_lowercase()) {
            suggestions.push(text.to_owned());
            if suggestions.len() == MAX_SUGGESTIONS {
                break;
            }
        }
    }
    Ok(suggestions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_is_exact_https_origin_and_encodes_the_query() {
        let url = request_url("  rust & lang?  ").unwrap();
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host_str(), Some(SUGGESTION_HOST));
        assert_eq!(
            url.as_str(),
            "https://suggestqueries-clients6.youtube.com/complete/search?client=firefox&ds=yt&oe=utf-8&ie=utf-8&q=rust+%26+lang%3F"
        );
        assert!(
            request_url("příliš")
                .unwrap()
                .as_str()
                .ends_with("&q=p%C5%99%C3%ADli%C5%A1")
        );
    }

    #[test]
    fn urls_controls_and_oversized_text_are_never_sent() {
        for rejected in [
            "",
            "   ",
            "https://www.youtube.com/watch?v=aqz-KE-bpKQ&si=secret",
            "http:example",
            "www.youtube.com/watch?v=x",
            "youtu.be/aqz-KE-bpKQ",
            "m.youtube.com/watch?v=x",
            "synthetic\nquery",
            "synthetic\u{0}query",
        ] {
            assert!(request_url(rejected).is_none(), "{rejected:?}");
        }
        assert!(request_url(&"a".repeat(MAX_QUERY_CHARS)).is_some());
        assert!(request_url(&"a".repeat(MAX_QUERY_CHARS + 1)).is_none());
    }

    #[test]
    fn parses_observed_json_and_jsonp_forms() {
        let json = br#"["rust lang",["rust language","rust lang","rust language tutorial"],[],{"google:suggestsubtypes":[[512],[512],[512]]}]"#;
        assert_eq!(
            parse(json, false).unwrap(),
            ["rust language", "rust lang", "rust language tutorial"]
        );
        let jsonp = br#"window.google.ac.h(["rust lang",[["rust language",0,[512]],["rust lang",0,[512]],["rust language \u0634\u0631\u062D",0,[22,30]]],{"k":1}])"#;
        assert_eq!(
            parse(jsonp, false).unwrap(),
            ["rust language", "rust lang", "rust language شرح"]
        );
    }

    #[test]
    fn decodes_latin1_bodies_and_rejects_invalid_utf8() {
        // Observed without oe=utf-8: raw 0xED for "í" plus \u escapes.
        let body = b"[\"\\u0159\xEDzek\",[\"\\u0159\xEDzek minecraft\"],[],{}]";
        assert_eq!(parse(body, true).unwrap(), ["řízek minecraft"]);
        assert_eq!(parse(body, false), Err(ProviderError::MalformedOutput));
    }

    #[test]
    fn malformed_input_is_rejected_or_skipped_defensively() {
        for malformed in [
            &b""[..],
            b"null",
            b"{}",
            b"[]",
            b"[\"q\"]",
            b"[\"q\",{}]",
            b"[\"q\",\"not a list\"]",
            b"window.google.ac.h([\"q\",[\"a\"]]",
            b"[\"q\",[\"a\"]] trailing",
            b"<!doctype html><html></html>",
        ] {
            assert_eq!(
                parse(malformed, false),
                Err(ProviderError::MalformedOutput),
                "{}",
                String::from_utf8_lossy(malformed)
            );
        }
        let mixed = br#"["q",[1,null,"",["   "],[],{"x":"y"},"ok","OK","  spaced  ","bad\u0007bell",["nested",0]]]"#;
        assert_eq!(parse(mixed, false).unwrap(), ["ok", "spaced", "nested"]);
        let long = format!(r#"["q",["{}","{}"]]"#, "a".repeat(201), "b".repeat(200));
        assert_eq!(parse(long.as_bytes(), false).unwrap(), ["b".repeat(200)]);
        let deep = format!("{}{}", "[".repeat(10_000), "]".repeat(10_000));
        assert_eq!(
            parse(deep.as_bytes(), false),
            Err(ProviderError::MalformedOutput)
        );
    }

    #[test]
    fn response_and_result_counts_are_bounded() {
        let many = format!(
            r#"["q",[{}]]"#,
            (0..50)
                .map(|index| format!("\"synthetic {index}\""))
                .collect::<Vec<_>>()
                .join(",")
        );
        let parsed = parse(many.as_bytes(), false).unwrap();
        assert_eq!(parsed.len(), MAX_SUGGESTIONS);
        assert_eq!(parsed[9], "synthetic 9");
        let oversized = format!(r#"["q",["{}"]]"#, "a".repeat(MAX_RESPONSE_BYTES));
        assert_eq!(
            parse(oversized.as_bytes(), false),
            Err(ProviderError::OutputTooLarge)
        );
    }
}
