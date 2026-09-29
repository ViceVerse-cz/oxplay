// SPDX-License-Identifier: GPL-3.0-or-later
//! Decisions shared by the compressed media range transport. No credential discovery.
use crate::{Error, Result};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serein_core::OriginHeaders;
use std::net::IpAddr;
use url::Url;

pub(crate) fn media_url(value: &str) -> Result<Url> {
    if value.len() > 32 * 1024 {
        return Err(Error::Policy);
    }
    let url = Url::parse(value).map_err(|_| Error::Policy)?;
    if url.scheme() != "https"
        || !url
            .host_str()
            .is_some_and(|host| host.ends_with(".googlevideo.com"))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::Policy);
    }
    Ok(url)
}

pub(crate) fn headers(url: &Url, source: &OriginHeaders) -> Result<HeaderMap> {
    if source.origin != url.origin().ascii_serialization() || source.fields.len() > 16 {
        return Err(Error::Policy);
    }
    let mut result = HeaderMap::new();
    let mut bytes = 0usize;
    for (name, value) in &source.fields {
        bytes = bytes.saturating_add(name.len()).saturating_add(value.len());
        if bytes > 16 * 1024 || value.bytes().any(|b| b < 32 || b == 127) {
            return Err(Error::Policy);
        }
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| Error::Policy)?;
        // This initial source is guest-only. Cookies and authorization require
        // a separate, explicitly session-bound capability, never a raw field.
        if !matches!(
            name.as_str(),
            "user-agent" | "accept" | "accept-language" | "sec-fetch-mode" | "origin" | "referer"
        ) || result.contains_key(&name)
        {
            return Err(Error::Policy);
        }
        let mut value = HeaderValue::from_str(value).map_err(|_| Error::Policy)?;
        value.set_sensitive(true);
        result.insert(name, value);
    }
    Ok(result)
}

/// Only ordinary public unicast endpoints. Reject entire special-use ranges,
/// including conservative exclusions inside otherwise global IPv6 space.
pub(crate) fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !matches!(a, 0 | 10 | 127 | 224..=255)
                && !(a == 100 && (64..=127).contains(&b))
                && !(a == 169 && b == 254)
                && !(a == 172 && (16..=31).contains(&b))
                && !(a == 192
                    && (b == 168 || (b == 0 && matches!(c, 0 | 2)) || (b == 88 && c == 99)))
                && !(a == 198 && (matches!(b, 18 | 19) || (b == 51 && c == 100)))
                && !(a == 203 && b == 0 && c == 113)
        }
        IpAddr::V6(ip) => {
            let s = ip.segments();
            (s[0] & 0xe000) == 0x2000
                && !(s[0] == 0x2001 && (s[1] < 0x0200 || s[1] == 0x0db8))
                && s[0] != 0x2002
                && !(s[0] == 0x3fff && s[1] < 0x1000)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_special_and_embedded_addresses_are_rejected() {
        for value in [
            "0.1.2.3",
            "10.0.0.1",
            "100.64.0.1",
            "127.0.0.1",
            "169.254.1.1",
            "172.16.0.1",
            "192.168.0.1",
            "192.0.2.1",
            "198.18.0.1",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "240.0.0.1",
            "::1",
            "::ffff:127.0.0.1",
            "fc00::1",
            "fe80::1",
            "2001:db8::1",
            "2002:7f00:1::",
            "3fff::1",
        ] {
            assert!(!public_ip(value.parse().unwrap()), "{value}");
        }
        for value in ["142.250.185.14", "8.8.8.8", "2607:f8b0:4007:80b::200e"] {
            assert!(public_ip(value.parse().unwrap()));
        }
    }
    #[test]
    fn origin_headers_are_bounded_and_never_accept_credentials() {
        let url = media_url("https://r1.googlevideo.com/videoplayback?secret=synthetic").unwrap();
        let mut fields = OriginHeaders {
            origin: url.origin().ascii_serialization(),
            fields: vec![("User-Agent".into(), "Synthetic test".into())],
        };
        assert!(headers(&url, &fields).is_ok());
        for name in [
            "Cookie",
            "Authorization",
            "Host",
            "Range",
            "Proxy-Authorization",
            "Connection",
        ] {
            fields.fields = vec![(name.into(), "synthetic".into())];
            assert!(headers(&url, &fields).is_err());
        }
        fields.fields = vec![("Accept".into(), "a\r\nb".into())];
        assert!(headers(&url, &fields).is_err());
        fields.fields.clear();
        fields.origin = "https://other.googlevideo.com".into();
        assert!(headers(&url, &fields).is_err());
    }
}
