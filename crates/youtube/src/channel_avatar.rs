// SPDX-License-Identifier: GPL-3.0-or-later
//! Public channel artwork only; never infer identity artwork from a video image.
use crate::YtDlp;
use serde_json::Value;
use oxplay_core::{ChannelId, OperationContext, ProviderError};

pub struct ChannelProfile {
    pub avatar_url: Option<String>,
    pub subscriber_count: Option<u64>,
}

impl YtDlp {
    /// Public channel header metadata from the native guest InnerTube reader;
    /// one metadata-only extraction through the isolated, supervised guest
    /// runner only for `catalog::falls_back` errors. `playlist-items=0` avoids
    /// enumerating a channel's uploads.
    pub fn channel_profile(
        &self,
        id: &ChannelId,
        operation: &OperationContext,
    ) -> Result<ChannelProfile, ProviderError> {
        if let Some(transport) = self.guest() {
            match crate::guest_catalog::channel_profile(transport, id, operation) {
                Err(error) if crate::catalog::falls_back(error) => {}
                result => return result,
            }
        }
        if operation.cancel.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        let value = self.run_with_priority(
            &[
                "--flat-playlist".into(),
                "--playlist-items".into(),
                "0".into(),
                "--".into(),
                format!("{}/videos", id.browse_url()),
            ],
            operation,
            &|| false,
            true,
        )?;
        if operation.cancel.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        profile(&value, id)
    }
}

fn profile(value: &Value, id: &ChannelId) -> Result<ChannelProfile, ProviderError> {
    Ok(ChannelProfile {
        avatar_url: avatar(value, id)?,
        subscriber_count: value.get("channel_follower_count").and_then(Value::as_u64),
    })
}

fn avatar(value: &Value, id: &ChannelId) -> Result<Option<String>, ProviderError> {
    if value.get("channel_id").and_then(Value::as_str) != Some(id.as_str())
        || value.get("_type").and_then(Value::as_str) != Some("playlist")
    {
        return Err(ProviderError::MalformedOutput);
    }
    let Some(thumbnails) = value.get("thumbnails").and_then(Value::as_array) else {
        return Ok(None);
    };
    if thumbnails.len() > 64 {
        return Err(ProviderError::OutputTooLarge);
    }
    // yt-dlp YoutubeTabIE marks the channelMetadataRenderer avatar explicitly.
    // Its list also contains banners, so selecting the last image is incorrect.
    let Some(uncropped) = thumbnails.iter().find_map(|item| {
        (item.get("id")?.as_str()? == "avatar_uncropped").then(|| item.get("url")?.as_str())?
    }) else {
        return Ok(None);
    };
    let base = uncropped.split('=').next();
    // Prefer the original bounded rendition of that same proven avatar.
    let original = thumbnails.iter().find_map(|item| {
        let url = item.get("url")?.as_str()?;
        (url != uncropped
            && url.split('=').next() == base
            && item.get("preference").and_then(Value::as_i64).unwrap_or(0) >= 0)
            .then_some(url)
    });
    Ok(original.or(Some(uncropped)).and_then(safe_avatar))
}

/// Shared public-portrait URL policy (channel profiles and comment authors).
pub(crate) fn safe_avatar(value: &str) -> Option<String> {
    if value.len() > 4096 {
        return None;
    }
    let url = url::Url::parse(value).ok()?;
    (url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.fragment().is_none()
        && matches!(
            url.host_str(),
            Some("yt3.ggpht.com" | "yt3.googleusercontent.com")
        ))
    .then(|| url.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn subscriber_count_is_optional_and_requires_the_matching_channel() {
        let id = ChannelId::new("UCabcdefghijklmnopqrstuv").unwrap();
        let mut value =
            json!({"_type":"playlist", "channel_id":id.as_str(), "channel_follower_count":12345});
        let channel = profile(&value, &id).unwrap();
        assert_eq!(channel.subscriber_count, Some(12345));
        assert_eq!(channel.avatar_url, None);
        value["channel_follower_count"] = json!(-1);
        assert_eq!(profile(&value, &id).unwrap().subscriber_count, None);
        value["channel_id"] = json!("UCzzzzzzzzzzzzzzzzzzzzzz");
        assert!(profile(&value, &id).is_err());
    }

    #[test]
    fn only_explicit_matching_channel_avatar_is_accepted() {
        let id = ChannelId::new("UCabcdefghijklmnopqrstuv").unwrap();
        let mut value = json!({"_type":"playlist", "channel_id":id.as_str(), "thumbnails":[
            {"url":"https://yt3.googleusercontent.com/channel=s88"},
            {"id":"avatar_uncropped", "url":"https://yt3.googleusercontent.com/channel=s0"},
            {"id":"banner_uncropped", "url":"https://yt3.googleusercontent.com/banner=s0"}
        ]});
        assert_eq!(
            avatar(&value, &id).unwrap().as_deref(),
            Some("https://yt3.googleusercontent.com/channel=s88")
        );
        value["thumbnails"] = json!([{"url":"https://i.ytimg.com/vi/abcdefghijk/default.jpg"}]);
        assert_eq!(avatar(&value, &id).unwrap(), None);
        value["channel_id"] = json!("UCzzzzzzzzzzzzzzzzzzzzzz");
        assert!(avatar(&value, &id).is_err());
        assert!(safe_avatar("https://yt3.googleusercontent.com.evil.test/a").is_none());
    }
}
