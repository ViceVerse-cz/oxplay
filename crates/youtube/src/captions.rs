// SPDX-License-Identifier: GPL-3.0-or-later
//! Selected-only guest timedtext access. Signed addresses never enter diagnostics.
use crate::{RateLimit, YtDlp};
use serde_json::Value;
use serein_core::{CaptionUrl, OperationContext, ProviderError, SubtitleTrack, VideoId};
use std::time::{Duration, SystemTime};
const MAX_TRACKS: usize = 64;
const MAX_CAPTION_BYTES: usize = 2 * 1024 * 1024;
pub struct CaptionData(Vec<u8>);
impl CaptionData {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}
pub(crate) fn tracks(value: &Value, video: &VideoId) -> (Vec<SubtitleTrack>, bool) {
    let mut result = Vec::new();
    let mut truncated = false;
    for (key, automatic) in [("subtitles", false), ("automatic_captions", true)] {
        let Some(languages) = value.get(key).and_then(Value::as_object) else {
            continue;
        };
        let mut entries: Vec<_> = languages.iter().collect();
        // Original automatic captions precede optional translated variants.
        entries.sort_by_key(|(language, _)| !language.ends_with("-orig"));
        for (language, formats) in entries {
            if language.is_empty()
                || language.len() > 64
                || !language
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
            {
                continue;
            }
            let Some(formats) = formats.as_array() else {
                continue;
            };
            let Some((format, url)) = formats
                .iter()
                .filter(|entry| entry.get("ext").and_then(Value::as_str) == Some("vtt"))
                .find_map(|entry| {
                    Some((
                        entry,
                        CaptionUrl::parse(entry.get("url")?.as_str()?, video).ok()?,
                    ))
                })
            else {
                continue;
            };
            if result.len() >= MAX_TRACKS {
                truncated = true;
                continue;
            }
            let label = format
                .get("name")
                .and_then(Value::as_str)
                .map(|name| {
                    name.chars()
                        .filter(|c| !c.is_control())
                        .take(200)
                        .collect::<String>()
                })
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| language.clone());
            result.push(SubtitleTrack {
                language: language.clone(),
                label,
                automatic,
                url,
                format: "vtt".into(),
            });
        }
    }
    (result, truncated)
}
fn validate_vtt(bytes: Vec<u8>) -> Result<CaptionData, ProviderError> {
    if bytes.len() > MAX_CAPTION_BYTES {
        return Err(ProviderError::OutputTooLarge);
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| ProviderError::UnsupportedFormat)?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    if !text
        .strip_prefix("WEBVTT")
        .is_some_and(|rest| rest.starts_with(['\n', '\r', ' ', '\t']))
        || text
            .bytes()
            .any(|b| b < 0x20 && !matches!(b, b'\n' | b'\r' | b'\t'))
        || text.lines().any(|line| line.len() > 16_384)
        || !text.lines().any(|line| line.contains(" --> "))
    {
        return Err(ProviderError::UnsupportedFormat);
    }
    Ok(CaptionData(bytes))
}
impl YtDlp {
    /// Blocking worker API. No credentials, redirect following, proxy inheritance,
    /// automatic retry, browser fingerprint impersonation, or challenge bypass.
    pub fn caption(
        &self,
        track: &SubtitleTrack,
        video: &VideoId,
        operation: &OperationContext,
    ) -> Result<CaptionData, ProviderError> {
        if operation.cancel.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        if track.url.video_id() != video || track.format != "vtt" {
            return Err(ProviderError::InvalidInput);
        }
        if self
            .cooldown
            .lock()
            .map_err(|_| ProviderError::ExtractorFailed)?
            .is_some_and(RateLimit::active)
        {
            return Err(ProviderError::RateLimited);
        }
        let _active = self.active.acquire(&operation.cancel, false)?;
        if self
            .cooldown
            .lock()
            .map_err(|_| ProviderError::ExtractorFailed)?
            .is_some_and(RateLimit::active)
        {
            return Err(ProviderError::RateLimited);
        }
        let client = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .pool_max_idle_per_host(0)
            .user_agent("Serein/0.1 (experimental native YouTube client)")
            .build()
            .map_err(|_| ProviderError::Offline)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| ProviderError::Offline)?;
        runtime.block_on(async {
            tokio::select! {
                result = async {
                    let mut response = client.get(track.url.expose_url()).header("Accept", "text/vtt").send().await.map_err(network_error)?;
                    match response.status().as_u16() {
                        200 => {},
                        300..=399 => return Err(ProviderError::UnsafeMedia),
                        401 => return Err(ProviderError::AuthenticationRequired),
                        403 => return Err(ProviderError::ProofRequired),
                        429 => {
                            let retry = response.headers().get("retry-after").and_then(|h| h.to_str().ok());
                            let delay = retry_delay(retry);
                            *self.cooldown.lock().map_err(|_| ProviderError::ExtractorFailed)? = Some(RateLimit::after(delay));
                            return Err(ProviderError::RateLimited);
                        },
                        _ => return Err(ProviderError::Unavailable),
                    }
                    if response.content_length().is_some_and(|n| n > MAX_CAPTION_BYTES as u64) { return Err(ProviderError::OutputTooLarge); }
                    let mut bytes = Vec::new();
                    while let Some(chunk) = response.chunk().await.map_err(network_error)? {
                        if bytes.len()+chunk.len() > MAX_CAPTION_BYTES { return Err(ProviderError::OutputTooLarge); }
                        bytes.extend_from_slice(&chunk);
                    }
                    if operation.cancel.is_cancelled() { return Err(ProviderError::Cancelled); }
                    validate_vtt(bytes)
                } => result,
                _ = async { loop { tokio::time::sleep(Duration::from_millis(20)).await; if operation.cancel.is_cancelled() { break; } } } => Err(ProviderError::Cancelled),
            }
        })
    }
}
fn retry_delay(value: Option<&str>) -> Duration {
    let minimum = Duration::from_secs(60);
    value
        .map(str::trim)
        .and_then(|value| {
            if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
                Some(Duration::from_secs(value.parse().unwrap_or(u64::MAX)))
            } else {
                httpdate::parse_http_date(value)
                    .ok()?
                    .duration_since(SystemTime::now())
                    .ok()
            }
        })
        .unwrap_or(minimum)
        .max(minimum)
}
fn network_error(error: reqwest::Error) -> ProviderError {
    if error.is_timeout() {
        ProviderError::Timeout
    } else {
        ProviderError::Offline
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn tracks_keep_manual_and_auto_identity_and_reject_foreign_urls() {
        let video = VideoId::new("abcdefghijk").unwrap();
        let value = json!({"subtitles":{"en":[{"ext":"vtt","name":"English","url":"https://www.youtube.com/api/timedtext?v=abcdefghijk&fmt=vtt&sig=secret"}],"fr":[{"ext":"vtt","url":"https://evil.invalid/api/timedtext?v=abcdefghijk&fmt=vtt"}]},"automatic_captions":{"de-orig":[{"ext":"vtt","url":"https://www.youtube.com/api/timedtext?v=abcdefghijk&fmt=vtt"}]}});
        let (tracks, limited) = tracks(&value, &video);
        assert_eq!(tracks.len(), 2);
        assert!(!limited);
        assert!(!tracks[0].automatic);
        assert!(tracks[1].automatic);
        assert_eq!(tracks[0].label, "English");
        assert!(!format!("{:?} {}", tracks[0].url, tracks[0].url).contains("secret"));
    }
    #[test]
    fn retry_after_is_never_silently_shortened() {
        assert_eq!(retry_delay(Some("90000")), Duration::from_secs(90000));
        assert_eq!(
            retry_delay(Some("999999999999999999999999")),
            Duration::from_secs(u64::MAX)
        );
        assert!(matches!(
            RateLimit::after(retry_delay(Some("999999999999999999999999"))),
            RateLimit::Indefinite
        ));
    }
    #[test]
    fn vtt_rejects_empty_html_binary_and_unbounded_payloads() {
        for data in [
            vec![],
            b"<html>challenge</html>".to_vec(),
            b"WEBVTT\n\n00:00.000 --> 00:01.000\n\0".to_vec(),
            vec![b'x'; MAX_CAPTION_BYTES + 1],
        ] {
            assert!(validate_vtt(data).is_err());
        }
        let text = b"WEBVTT\n\n00:00.000 --> 00:01.000\nSynthetic text\n".to_vec();
        assert_eq!(validate_vtt(text.clone()).unwrap().as_bytes(), text);
    }
    #[test]
    fn generated_caption_urls_enforce_exact_video_origin_and_format() {
        for index in 0..128 {
            let id = VideoId::new(&format!("{index:011}")).unwrap();
            let query = format!("v={}&fmt=vtt&sig=secret{index}", id.as_str());
            let accepted = CaptionUrl::parse(
                &format!("https://www.youtube.com/api/timedtext?{query}"),
                &id,
            )
            .unwrap();
            assert_eq!(accepted.video_id(), &id);
            for authority in [
                "www.youtube.com.evil.invalid",
                "www.youtube.com@evil.invalid",
                "user@www.youtube.com",
                "www.youtube.com:444",
                "r1.googlevideo.com",
            ] {
                assert!(
                    CaptionUrl::parse(&format!("https://{authority}/api/timedtext?{query}"), &id)
                        .is_err()
                );
            }
            for query in [
                format!("{query}&v={}", id.as_str()),
                format!("{query}&fmt=srv3"),
                "v=abcdefghijk&fmt=json3".into(),
                "fmt=vtt".into(),
            ] {
                assert!(
                    CaptionUrl::parse(
                        &format!("https://www.youtube.com/api/timedtext?{query}"),
                        &id
                    )
                    .is_err()
                );
            }
            assert!(!format!("{accepted:?}").contains("secret"));
        }
    }
    #[test]
    fn captions_are_bounded_and_cancel_before_transport() {
        let video = VideoId::new("abcdefghijk").unwrap();
        let mut languages = serde_json::Map::new();
        for index in 0..100 {
            languages.insert(format!("lang-{index}"), json!([{"ext":"vtt","name":"x".repeat(1000),"url":"https://www.youtube.com/api/timedtext?v=abcdefghijk&fmt=vtt"}]));
        }
        let (tracks, limited) = tracks(&json!({"subtitles":languages}), &video);
        assert_eq!(tracks.len(), 64);
        assert!(limited);
        assert!(tracks.iter().all(|track| track.label.len() == 200));
        let operation = OperationContext {
            request_id: 1,
            session_generation: 0,
            cancel: Default::default(),
        };
        operation.cancel.cancel();
        let provider = YtDlp::new("/does/not/exist").unwrap();
        assert!(matches!(
            provider.caption(&tracks[0], &video, &operation),
            Err(ProviderError::Cancelled)
        ));
    }
}
