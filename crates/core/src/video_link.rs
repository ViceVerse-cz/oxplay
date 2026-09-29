// SPDX-License-Identifier: GPL-3.0-or-later
//! Initial playback intent from a public YouTube link. Never a media address.
use crate::{ProviderError, VideoId, parse_url};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VideoStart(u32);
impl VideoStart {
    pub fn seconds(self) -> u32 {
        self.0
    }
    fn parse(input: &str) -> Result<Self, ProviderError> {
        if input.is_empty() || input.len() > 64 || !input.is_ascii() {
            return Err(ProviderError::InvalidInput);
        }
        if input.bytes().all(|b| b.is_ascii_digit()) {
            return input
                .parse()
                .map(Self)
                .map_err(|_| ProviderError::InvalidInput);
        }
        let mut remaining = input;
        let mut previous = 3601;
        let mut total = 0_u32;
        while !remaining.is_empty() {
            let digits = remaining.bytes().take_while(u8::is_ascii_digit).count();
            if digits == 0 || digits == remaining.len() {
                return Err(ProviderError::InvalidInput);
            }
            let amount: u32 = remaining[..digits]
                .parse()
                .map_err(|_| ProviderError::InvalidInput)?;
            let unit = match remaining.as_bytes()[digits] {
                b'h' => 3600,
                b'm' => 60,
                b's' => 1,
                _ => return Err(ProviderError::InvalidInput),
            };
            if unit >= previous {
                return Err(ProviderError::InvalidInput);
            }
            previous = unit;
            total = amount
                .checked_mul(unit)
                .and_then(|part| total.checked_add(part))
                .ok_or(ProviderError::InvalidInput)?;
            remaining = &remaining[digits + 1..];
        }
        Ok(Self(total))
    }
}

pub struct VideoLink {
    pub id: VideoId,
    pub start: VideoStart,
}
impl VideoLink {
    pub fn from_url(input: &str) -> Result<Self, ProviderError> {
        let id = VideoId::from_url(input)?;
        let url = parse_url(input, ProviderError::InvalidInput)?;
        let mut start = None;
        for (key, value) in url.query_pairs() {
            if matches!(key.as_ref(), "t" | "start") {
                if start.is_some() {
                    return Err(ProviderError::InvalidInput);
                }
                start = Some(VideoStart::parse(&value)?);
            }
        }
        if let Some(fragment) = url.fragment()
            && fragment.starts_with("t=")
        {
            if start.is_some() || fragment.contains('&') {
                return Err(ProviderError::InvalidInput);
            }
            // Decode exactly the fragment key/value as URL query syntax, so
            // encoded signs or separators cannot evade the numeric parser.
            let (_, value) = url::form_urlencoded::parse(fragment.as_bytes())
                .next()
                .ok_or(ProviderError::InvalidInput)?;
            start = Some(VideoStart::parse(&value)?);
        }
        Ok(Self {
            id,
            start: start.unwrap_or_default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_links_retain_an_exact_bounded_initial_position() {
        for (suffix, expected) in [
            ("", 0),
            ("&t=90", 90),
            ("&t=1h2m3s", 3723),
            ("&start=2m", 120),
            ("#t=45s", 45),
            ("&t=0", 0),
        ] {
            let link = VideoLink::from_url(&format!(
                "https://www.youtube.com/watch?v=abcdefghijk{suffix}"
            ))
            .unwrap();
            assert_eq!(link.id.as_str(), "abcdefghijk");
            assert_eq!(link.start.seconds(), expected);
        }
        assert_eq!(
            VideoLink::from_url("https://youtu.be/abcdefghijk?t=90")
                .unwrap()
                .start
                .seconds(),
            90
        );
    }
    #[test]
    fn ambiguity_and_invalid_times_are_rejected_not_silently_reset() {
        for suffix in [
            "&t=",
            "&t=-1",
            "&t=1.5",
            "&t=1e3",
            "&t=1m2h",
            "&t=1m2m",
            "&t=4294967296",
            "&t=4294967295h",
            "&t=1&t=1",
            "&t=1&start=1",
            "&t=1#t=1",
            "#t=1&t=2",
            "&t=%2B90",
            "&t=1h2",
        ] {
            assert!(
                VideoLink::from_url(&format!(
                    "https://www.youtube.com/watch?v=abcdefghijk{suffix}"
                ))
                .is_err(),
                "{suffix}"
            );
        }
        assert!(VideoLink::from_url("https://evil.example/watch?v=abcdefghijk&t=90").is_err());
    }
}
