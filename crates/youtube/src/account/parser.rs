//! Bounded normalization of reviewed InnerTube renderer shapes. Unknown shapes
//! yield partial results or an unsupported error, never synthetic account success.
use super::*;
#[cfg(test)]
use crate::renderers::duration_text;
use crate::renderers::{Parsed, text, video_lockup, video_renderer};
use std::collections::HashSet;
const MAX_NODES: usize = 50_000;
const MAX_ITEMS: usize = 200;
fn nodes(value: &Value) -> Result<Vec<(&str, &Value)>, AccountError> {
    nodes_except(value, &[])
}
/// Bounded traversal that does not descend into the values of `skip` keys.
fn nodes_except<'a>(
    value: &'a Value,
    skip: &[&str],
) -> Result<Vec<(&'a str, &'a Value)>, AccountError> {
    let mut stack = vec![value];
    let mut result = Vec::new();
    let mut visited = 0;
    while let Some(value) = stack.pop() {
        visited += 1;
        if visited > MAX_NODES {
            return Err(AccountError::ResponseTooLarge);
        }
        match value {
            Value::Object(map) => {
                if map.keys().any(|k| {
                    matches!(
                        k.as_str(),
                        "adSlotRenderer"
                            | "displayAdRenderer"
                            | "promotedVideoRenderer"
                            | "promotedSparklesWebRenderer"
                            | "inFeedAdLayoutRenderer"
                    )
                }) {
                    continue;
                }
                if visited + stack.len() + map.len() > MAX_NODES {
                    return Err(AccountError::ResponseTooLarge);
                }
                for (key, value) in map {
                    result.push((key.as_str(), value));
                    if !skip.contains(&key.as_str()) {
                        stack.push(value);
                    }
                }
            }
            Value::Array(values) => {
                if visited + stack.len() + values.len() > MAX_NODES {
                    return Err(AccountError::ResponseTooLarge);
                }
                stack.extend(values.iter().rev());
            }
            _ => {}
        }
    }
    Ok(result)
}
fn continuation(
    value: &Value,
    generation: u64,
    kind: PageKind,
) -> Result<Option<AccountCursor>, AccountError> {
    let mut tokens = HashSet::new();
    for (key, value) in nodes(value)? {
        let token = match key {
            "continuationCommand" => value.get("token").and_then(Value::as_str),
            "nextContinuationData" => value.get("continuation").and_then(Value::as_str),
            _ => None,
        };
        if let Some(token) = token {
            if token.is_empty() || token.len() > 16_384 || token.chars().any(char::is_control) {
                return Err(AccountError::UnsupportedResponse);
            }
            tokens.insert(token.to_owned());
        }
    }
    if tokens.len() > 1 {
        return Err(AccountError::UnsupportedResponse);
    }
    Ok(tokens.into_iter().next().map(|token| AccountCursor {
        token,
        generation,
        kind,
    }))
}
fn recognized_empty(value: &Value) -> Result<bool, AccountError> {
    Ok(nodes(value)?
        .iter()
        .any(|(key, _)| matches!(*key, "messageRenderer" | "backgroundPromoRenderer")))
}
pub(super) fn identities(value: &Value) -> Result<Vec<AccountIdentity>, AccountError> {
    let mut identities = Vec::new();
    for (key, item) in nodes(value)? {
        if !matches!(key, "accountItem" | "accountItemRenderer") {
            continue;
        }
        if item.get("isDisabled").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let display_name = text(item.get("accountName"));
        if display_name.is_empty() {
            return Err(AccountError::UnsupportedResponse);
        }
        let handle = text(item.get("channelHandle"));
        identities.push(AccountIdentity {
            display_name,
            handle: (!handle.is_empty()).then_some(handle),
            selected: item.get("isSelected").and_then(Value::as_bool) == Some(true),
            has_channel: item.get("hasChannel").and_then(Value::as_bool) == Some(true),
        });
        if identities.len() > 50 {
            return Err(AccountError::ResponseTooLarge);
        }
    }
    if identities.is_empty() {
        return Err(AccountError::IdentityNotVerified);
    }
    Ok(identities)
}
pub(super) fn subscriptions(
    value: &Value,
    generation: u64,
) -> Result<AccountPage<AccountChannel>, AccountError> {
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    let mut partial = false;
    for (key, item) in nodes(value)? {
        if matches!(key, "channelRenderer" | "gridChannelRenderer") {
            let Some(id) = item.get("channelId").and_then(Value::as_str) else {
                partial = true;
                continue;
            };
            let id = ChannelId(id.to_owned());
            if validate_channel(&id).is_err() {
                partial = true;
                continue;
            }
            if seen.insert(id.0.clone()) {
                items.push(AccountChannel {
                    id,
                    title: text(item.get("title")),
                });
            }
        } else if key == "lockupViewModel" {
            if item.get("contentType").and_then(Value::as_str)
                == Some("LOCKUP_CONTENT_TYPE_CHANNEL")
            {
                let Some(id) = item.get("contentId").and_then(Value::as_str) else {
                    partial = true;
                    continue;
                };
                let id = ChannelId(id.to_owned());
                if validate_channel(&id).is_err() {
                    partial = true;
                    continue;
                }
                if seen.insert(id.0.clone()) {
                    items.push(AccountChannel {
                        id,
                        title: text(item.pointer("/metadata/lockupMetadataViewModel/title")),
                    });
                }
            } else {
                partial = true;
            }
        }
    }
    if items.len() > MAX_ITEMS {
        return Err(AccountError::ResponseTooLarge);
    }
    if items.is_empty() && !recognized_empty(value)? {
        return Err(AccountError::UnsupportedResponse);
    }
    Ok(AccountPage {
        items,
        next: continuation(value, generation, PageKind::Subscriptions)?,
        partial,
    })
}
pub(super) fn playlists(
    value: &Value,
    generation: u64,
) -> Result<AccountPage<AccountPlaylist>, AccountError> {
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    let mut partial = false;
    for (key, item) in nodes(value)? {
        if matches!(key, "playlistRenderer" | "gridPlaylistRenderer") {
            let Some(id) = item.get("playlistId").and_then(Value::as_str) else {
                partial = true;
                continue;
            };
            let id = PlaylistId(id.to_owned());
            if validate_playlist(&id).is_err() {
                partial = true;
                continue;
            }
            if seen.insert(id.0.clone()) {
                items.push(AccountPlaylist {
                    id,
                    title: text(item.get("title")),
                    editable: item.get("isEditable").and_then(Value::as_bool),
                });
            }
        } else if key == "lockupViewModel" {
            if item.get("contentType").and_then(Value::as_str)
                == Some("LOCKUP_CONTENT_TYPE_PLAYLIST")
            {
                let Some(id) = item.get("contentId").and_then(Value::as_str) else {
                    partial = true;
                    continue;
                };
                let id = PlaylistId(id.to_owned());
                if validate_playlist(&id).is_err() {
                    partial = true;
                    continue;
                }
                if seen.insert(id.0.clone()) {
                    items.push(AccountPlaylist {
                        id,
                        title: text(item.pointer("/metadata/lockupMetadataViewModel/title")),
                        editable: None,
                    });
                }
            } else {
                partial = true;
            }
        }
    }
    if items.len() > MAX_ITEMS {
        return Err(AccountError::ResponseTooLarge);
    }
    if items.is_empty() && !recognized_empty(value)? {
        return Err(AccountError::UnsupportedResponse);
    }
    Ok(AccountPage {
        items,
        next: continuation(value, generation, PageKind::Playlists)?,
        partial,
    })
}
pub(super) fn playlist(
    value: &Value,
    id: &PlaylistId,
    generation: u64,
) -> Result<PlaylistContents, AccountError> {
    let mut items = Vec::new();
    let mut partial = false;
    let mut editable = false;
    let mut saw_list = false;
    for (key, item) in nodes(value)? {
        if key == "playlistVideoListRenderer" {
            if item.get("playlistId").and_then(Value::as_str) != Some(id.0.as_str()) {
                return Err(AccountError::UnsupportedResponse);
            }
            saw_list = true;
            editable = item.get("isEditable").and_then(Value::as_bool) == Some(true);
        }
        if key == "playlistVideoRenderer" {
            let Some(video) = item
                .get("videoId")
                .and_then(Value::as_str)
                .and_then(|v| VideoId::new(v).ok())
            else {
                partial = true;
                continue;
            };
            let set_video_id = item
                .get("setVideoId")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty() && s.len() <= 256)
                .map(str::to_owned);
            // Missing set IDs forbid mutation reconciliation, but the visible read remains useful.
            items.push(AccountPlaylistItem {
                video_id: video,
                title: text(item.get("title")),
                set_video_id,
            });
        }
    }
    if items.len() > MAX_ITEMS {
        return Err(AccountError::ResponseTooLarge);
    }
    if !saw_list && items.is_empty() && !recognized_empty(value)? {
        return Err(AccountError::UnsupportedResponse);
    }
    Ok(PlaylistContents {
        page: AccountPage {
            items,
            next: continuation(value, generation, PageKind::Playlist(id.0.clone()))?,
            partial,
        },
        editable,
    })
}
pub(super) fn subscription_state(value: &Value, id: &ChannelId) -> Result<bool, AccountError> {
    let states: Vec<_> = nodes(value)?
        .into_iter()
        .filter(|(key, item)| {
            *key == "subscribeButtonRenderer"
                && item.get("channelId").and_then(Value::as_str) == Some(&id.0)
        })
        .filter_map(|(_, item)| item.get("subscribed").and_then(Value::as_bool))
        .collect();
    if states.is_empty() || states.iter().any(|s| *s != states[0]) {
        return Err(AccountError::UnsupportedResponse);
    }
    Ok(states[0])
}
pub(super) fn rating(value: &Value) -> Result<bool, AccountError> {
    let mut states = Vec::new();
    for (key, item) in nodes(value)? {
        if key == "toggleButtonRenderer"
            && item
                .pointer("/defaultIcon/iconType")
                .and_then(Value::as_str)
                == Some("LIKE")
            && let Some(state) = item.get("isToggled").and_then(Value::as_bool)
        {
            states.push(state);
        }
        if key == "likeButtonViewModel"
            && let Some(state) = item
                .pointer("/likeStatusEntity/likeStatus")
                .and_then(Value::as_str)
        {
            match state {
                "LIKE" => states.push(true),
                "INDIFFERENT" | "DISLIKE" => states.push(false),
                _ => {}
            }
        }
    }
    if states.is_empty() || states.iter().any(|s| *s != states[0]) {
        return Err(AccountError::UnsupportedResponse);
    }
    Ok(states[0])
}
pub(super) fn action_rejected(value: &Value) -> bool {
    value.get("error").is_some()
        || nodes(value)
            .map(|nodes| {
                nodes.into_iter().any(|(key, item)| {
                    key == "alertRenderer"
                        && item.get("type").and_then(Value::as_str) == Some("ERROR")
                })
            })
            .unwrap_or(true)
}
/// Shelves (Shorts, news, posts), Shorts items and the topic-chip bar are not
/// part of the recommendation grid; their subtrees are neither traversed nor shown.
const FEED_SKIP: &[&str] = &[
    "richSectionRenderer",
    "reelShelfRenderer",
    "richShelfRenderer",
    "reelItemRenderer",
    "shortsLockupViewModel",
    "feedFilterChipBarRenderer",
];
/// Signed-in `FEwhat_to_watch` browse or its continuation. Only the direct item
/// arrays of the reviewed grid containers are read; nested navigation commands,
/// chip continuations and shelves never become items or cursors.
pub(super) fn recommendations(
    value: &Value,
    generation: u64,
) -> Result<AccountPage<VideoSummary>, AccountError> {
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    let mut partial = false;
    let mut saw_feed = false;
    let mut tokens = HashSet::new();
    for (key, container) in nodes_except(value, FEED_SKIP)? {
        let entries = match key {
            "richGridRenderer" => container.get("contents"),
            "appendContinuationItemsAction" | "reloadContinuationItemsCommand" => {
                container.get("continuationItems")
            }
            _ => continue,
        };
        saw_feed = true;
        let Some(entries) = entries.and_then(Value::as_array) else {
            continue;
        };
        for entry in entries {
            // Ad-badged video renderers are filtered by the shared parser.
            if crate::is_promoted(entry) {
                continue;
            }
            if let Some(next) = entry.get("continuationItemRenderer") {
                match next
                    .pointer("/continuationEndpoint/continuationCommand/token")
                    .and_then(Value::as_str)
                {
                    Some(token) => {
                        if token.is_empty()
                            || token.len() > 16_384
                            || token.chars().any(char::is_control)
                        {
                            return Err(AccountError::UnsupportedResponse);
                        }
                        tokens.insert(token.to_owned());
                    }
                    None => partial = true,
                }
                continue;
            }
            if FEED_SKIP.iter().any(|key| entry.get(*key).is_some()) {
                continue;
            }
            let Some(content) = entry.pointer("/richItemRenderer/content") else {
                partial = true;
                continue;
            };
            match feed_entry(content) {
                Parsed::Item(video) => {
                    if seen.insert(video.id.clone()) {
                        items.push(video);
                    }
                }
                Parsed::Filtered => {}
                Parsed::Unsupported => partial = true,
            }
            if items.len() > MAX_ITEMS {
                return Err(AccountError::ResponseTooLarge);
            }
        }
    }
    if !saw_feed && !recognized_empty(value)? {
        return Err(AccountError::UnsupportedResponse);
    }
    if tokens.len() > 1 {
        return Err(AccountError::UnsupportedResponse);
    }
    Ok(AccountPage {
        items,
        next: tokens.into_iter().next().map(|token| AccountCursor {
            token,
            generation,
            kind: PageKind::Recommendations,
        }),
        partial,
    })
}
fn feed_entry(content: &Value) -> Parsed<VideoSummary> {
    if let Some(video) = content.get("videoRenderer") {
        video_renderer(video)
    } else if let Some(lockup) = content.get("lockupViewModel") {
        video_lockup(lockup)
    } else if content.get("reelItemRenderer").is_some()
        || content.get("shortsLockupViewModel").is_some()
    {
        Parsed::Filtered
    } else {
        Parsed::Unsupported
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn verified_identity_needs_real_identity_shape() {
        assert!(identities(&json!({"responseContext":{}})).is_err());
        let identities=identities(&json!({"actions":[{"accountItem":{"accountName":{"simpleText":"Synthetic account"},"channelHandle":{"simpleText":"@synthetic"},"isSelected":true,"hasChannel":true}}]})).unwrap();
        assert_eq!(identities.len(), 1);
        assert!(identities[0].selected);
    }
    #[test]
    fn subscriptions_filter_promotions_and_scope_continuations() {
        let response = json!({"contents":[{"channelRenderer":{"channelId":"UCabcdefghijklmnopqrstuv","title":{"simpleText":"Synthetic channel"}}},{"promotedVideoRenderer":{"channelRenderer":{"channelId":"UCzyxwvutsrqponmlkjihgfed"}}},{"continuationItemRenderer":{"continuationEndpoint":{"continuationCommand":{"token":"synthetic-token"}}}}]});
        let page = subscriptions(&response, 42).unwrap();
        assert_eq!(page.items.len(), 1);
        let cursor = page.next.unwrap();
        assert_eq!(cursor.generation, 42);
        assert!(matches!(cursor.kind, PageKind::Subscriptions));
    }
    #[test]
    fn unknown_account_pages_are_not_successful_empty_reads() {
        assert!(subscriptions(&json!({"futureRenderer":{}}), 0).is_err());
        assert!(playlists(&json!({"futureRenderer":{}}), 0).is_err());
        assert!(
            subscriptions(
                &json!({"messageRenderer":{"text":{"simpleText":"Synthetic empty"}}}),
                0
            )
            .unwrap()
            .items
            .is_empty()
        );
    }
    #[test]
    fn duplicate_video_items_keep_distinct_set_ids() {
        let value = json!({"playlistVideoListRenderer":{"playlistId":"PLsynthetic","isEditable":true,"contents":[{"playlistVideoRenderer":{"videoId":"abcdefghijk","title":{"simpleText":"Synthetic video"},"setVideoId":"item_one"}},{"playlistVideoRenderer":{"videoId":"abcdefghijk","title":{"simpleText":"Synthetic video"},"setVideoId":"item_two"}}]}});
        let list = playlist(&value, &PlaylistId("PLsynthetic".into()), 0).unwrap();
        assert!(list.editable);
        assert_eq!(list.page.items.len(), 2);
        assert_ne!(
            list.page.items[0].set_video_id,
            list.page.items[1].set_video_id
        );
    }
    #[test]
    fn modern_lockup_metadata_is_normalized_without_inventing_editability() {
        let value = json!({"lockupViewModel":{"contentId":"PLsynthetic","contentType":"LOCKUP_CONTENT_TYPE_PLAYLIST","metadata":{"lockupMetadataViewModel":{"title":{"content":"Synthetic playlist"}}}}});
        let page = playlists(&value, 1).unwrap();
        assert_eq!(page.items[0].title, "Synthetic playlist");
        assert!(page.items[0].editable.is_none());
    }
    // TEST FIXTURES below are small synthetic shapes written for these tests.
    // They contain no real account, video, channel or session data.
    fn synthetic_video(id: &str) -> Value {
        json!({"richItemRenderer":{"content":{"videoRenderer":{
            "videoId": id,
            "navigationEndpoint": {"watchEndpoint": {"videoId": id}},
            "title": {"runs": [{"text": "Synthetic recommendation "}, {"text": id}]},
            "ownerText": {"runs": [{"text": "Synthetic channel", "navigationEndpoint": {"browseEndpoint": {"browseId": "UCabcdefghijklmnopqrstuv"}}}]},
            "lengthText": {"simpleText": "1:02:03"},
            "thumbnail": {"thumbnails": [
                {"url": "https://i.ytimg.com/vi/synthetic/default.jpg", "width": 120},
                {"url": "https://i.ytimg.com/vi/synthetic/hqdefault.jpg?sqp=synthetic", "width": 480},
                {"url": "https://i.ytimg.com/vi/synthetic/maxresdefault.jpg", "width": 1280}
            ]}
        }}}})
    }
    fn continuation_item(token: &str) -> Value {
        json!({"continuationItemRenderer":{"continuationEndpoint":{"continuationCommand":{"token": token}}}})
    }
    #[test]
    fn synthetic_home_grid_yields_videos_and_one_scoped_continuation() {
        let response = json!({
            "responseContext": {"mainAppWebResponseContext": {"loggedOut": false}},
            "contents": {"twoColumnBrowseResultsRenderer": {"tabs": [{"tabRenderer": {"content": {"richGridRenderer": {
                "header": {"feedFilterChipBarRenderer": {"contents": [{"chipCloudChipRenderer": {
                    "navigationEndpoint": {"continuationCommand": {"token": "synthetic-chip-token"}}}}]}},
                "contents": [
                    synthetic_video("aaaaaaaaaaa"),
                    {"richItemRenderer": {"content": {"adSlotRenderer": {"synthetic": true}}}},
                    {"richSectionRenderer": {"content": {"richShelfRenderer": {"contents": [
                        synthetic_video("sssssssssss"), continuation_item("synthetic-shelf-token")]}}}},
                    {"richItemRenderer": {"content": {"reelItemRenderer": {"videoId": "rrrrrrrrrrr"}}}},
                    synthetic_video("bbbbbbbbbbb"),
                    synthetic_video("aaaaaaaaaaa"),
                    continuation_item("synthetic-feed-token")
                ]
            }}}}]}}
        });
        let page = recommendations(&response, 9).unwrap();
        let ids: Vec<_> = page.items.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids, ["aaaaaaaaaaa", "bbbbbbbbbbb"]);
        assert!(!page.partial, "ads and Shorts are filtered, not failures");
        let video = &page.items[0];
        assert_eq!(video.title, "Synthetic recommendation aaaaaaaaaaa");
        assert_eq!(video.channel, "Synthetic channel");
        assert_eq!(
            video.channel_id.as_ref().map(ChannelId::as_str),
            Some("UCabcdefghijklmnopqrstuv")
        );
        assert_eq!(video.duration, Some(std::time::Duration::from_secs(3723)));
        assert_eq!(
            video.thumbnail_url.as_deref(),
            Some("https://i.ytimg.com/vi/synthetic/hqdefault.jpg?sqp=synthetic")
        );
        let cursor = page.next.unwrap();
        assert_eq!(cursor.token, "synthetic-feed-token");
        assert_eq!(cursor.generation, 9);
        assert!(cursor.kind == PageKind::Recommendations);
    }
    #[test]
    fn promoted_metadata_and_ad_badges_are_excluded_from_recommendations() {
        let mut badged = synthetic_video("ccccccccccc");
        badged["richItemRenderer"]["content"]["videoRenderer"]["badges"] =
            json!([{"metadataBadgeRenderer": {"style": "BADGE_STYLE_TYPE_AD"}}]);
        let mut metadata = synthetic_video("ddddddddddd");
        metadata["richItemRenderer"]["content"]["videoRenderer"]["adMetadata"] = json!({});
        let response = json!({"richGridRenderer": {"contents": [
            badged,
            metadata,
            {"richItemRenderer": {"content": {"promotedVideoRenderer": {"videoId": "eeeeeeeeeee"}}}},
            {"richItemRenderer": {"content": {"inFeedAdLayoutRenderer": {}}}},
            synthetic_video("fffffffffff")
        ]}});
        let page = recommendations(&response, 0).unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].id.as_str(), "fffffffffff");
        assert!(page.next.is_none());
    }
    #[test]
    fn lockup_view_models_are_normalized_and_non_video_lockups_are_partial() {
        let lockup = json!({"richItemRenderer": {"content": {"lockupViewModel": {
            "contentId": "ggggggggggg",
            "contentType": "LOCKUP_CONTENT_TYPE_VIDEO",
            "rendererContext": {"commandContext": {"onTap": {"innertubeCommand": {"watchEndpoint": {"videoId": "ggggggggggg"}}}}},
            "contentImage": {"thumbnailViewModel": {
                "image": {"sources": [{"url": "//i.ytimg.com/vi/synthetic/lockup.jpg", "width": 360}]},
                "overlays": [{"thumbnailOverlayBadgeViewModel": {"thumbnailBadges": [{"thumbnailBadgeViewModel": {"text": "12:34"}}]}}]
            }},
            "metadata": {"lockupMetadataViewModel": {
                "title": {"content": "Synthetic lockup video"},
                "image": {"decoratedAvatarViewModel": {"rendererContext": {"commandContext": {"onTap": {"innertubeCommand": {"browseEndpoint": {"browseId": "UCzyxwvutsrqponmlkjihgfe"}}}}}}},
                "metadata": {"contentMetadataViewModel": {"metadataRows": [{"metadataParts": [{"text": {"content": "Synthetic lockup channel"}}]}]}}
            }}
        }}}});
        let short = json!({"richItemRenderer": {"content": {"lockupViewModel": {
            "contentId": "hhhhhhhhhhh", "contentType": "LOCKUP_CONTENT_TYPE_VIDEO",
            "rendererContext": {"commandContext": {"onTap": {"innertubeCommand": {"reelWatchEndpoint": {"videoId": "hhhhhhhhhhh"}}}}}
        }}}});
        let mix = json!({"richItemRenderer": {"content": {"lockupViewModel": {
            "contentId": "RDsynthetic", "contentType": "LOCKUP_CONTENT_TYPE_PLAYLIST"
        }}}});
        let page = recommendations(
            &json!({"richGridRenderer": {"contents": [lockup, short, mix]}}),
            0,
        )
        .unwrap();
        assert_eq!(page.items.len(), 1);
        let video = &page.items[0];
        assert_eq!(video.id.as_str(), "ggggggggggg");
        assert_eq!(video.title, "Synthetic lockup video");
        assert_eq!(video.channel, "Synthetic lockup channel");
        assert_eq!(
            video.channel_id.as_ref().map(ChannelId::as_str),
            Some("UCzyxwvutsrqponmlkjihgfe")
        );
        assert_eq!(video.duration, Some(std::time::Duration::from_secs(754)));
        assert_eq!(
            video.thumbnail_url.as_deref(),
            Some("https://i.ytimg.com/vi/synthetic/lockup.jpg")
        );
        assert!(
            page.partial,
            "an unsupported mix lockup is reported, not hidden"
        );
    }
    #[test]
    fn continuation_responses_append_items_and_expose_the_next_token() {
        let response = json!({"onResponseReceivedActions": [{"appendContinuationItemsAction": {
            "continuationItems": [synthetic_video("iiiiiiiiiii"), continuation_item("synthetic-next-token")]
        }}]});
        let page = recommendations(&response, 3).unwrap();
        assert_eq!(page.items[0].id.as_str(), "iiiiiiiiiii");
        assert_eq!(page.next.unwrap().token, "synthetic-next-token");
        let last = json!({"onResponseReceivedActions": [{"appendContinuationItemsAction": {
            "continuationItems": [synthetic_video("jjjjjjjjjjj")]
        }}]});
        assert!(recommendations(&last, 3).unwrap().next.is_none());
    }
    #[test]
    fn malformed_recommendations_are_partial_or_rejected_never_fabricated() {
        let mut unsafe_art = synthetic_video("kkkkkkkkkkk");
        unsafe_art["richItemRenderer"]["content"]["videoRenderer"]["thumbnail"] = json!({"thumbnails": [
            {"url": "https://i.ytimg.com.attacker.invalid/vi/x.jpg", "width": 480}]});
        unsafe_art["richItemRenderer"]["content"]["videoRenderer"]["lengthText"] =
            json!({"simpleText": "LIVE"});
        let mut mismatched = synthetic_video("lllllllllll");
        mismatched["richItemRenderer"]["content"]["videoRenderer"]["navigationEndpoint"] =
            json!({"watchEndpoint": {"videoId": "mmmmmmmmmmm"}});
        let response = json!({"richGridRenderer": {"contents": [
            unsafe_art,
            mismatched,
            synthetic_video("bad/id?x=1"),
            {"richItemRenderer": {"content": {"futureRenderer": {}}}},
            {"richItemRenderer": {}},
            "not an object"
        ]}});
        let page = recommendations(&response, 0).unwrap();
        assert_eq!(page.items.len(), 1);
        assert!(page.partial);
        assert!(page.items[0].thumbnail_url.is_none());
        assert!(page.items[0].duration.is_none());
        // Unknown page shapes are not a successful empty feed.
        assert_eq!(
            recommendations(&json!({"futureRenderer": {}}), 0).err(),
            Some(AccountError::UnsupportedResponse)
        );
        assert!(
            recommendations(
                &json!({"messageRenderer": {"text": {"simpleText": "Synthetic empty"}}}),
                0
            )
            .unwrap()
            .items
            .is_empty()
        );
        for tokens in [
            vec![
                continuation_item("synthetic-one"),
                continuation_item("synthetic-two"),
            ],
            vec![continuation_item("synthetic\ncontrol")],
            vec![continuation_item("")],
        ] {
            assert_eq!(
                recommendations(&json!({"richGridRenderer": {"contents": tokens}}), 0).err(),
                Some(AccountError::UnsupportedResponse)
            );
        }
        let many: Vec<_> = (0..=MAX_ITEMS)
            .map(|index| synthetic_video(&format!("{index:011}")))
            .collect();
        assert_eq!(
            recommendations(&json!({"richGridRenderer": {"contents": many}}), 0).err(),
            Some(AccountError::ResponseTooLarge)
        );
    }
    #[test]
    fn duration_text_accepts_only_clock_forms() {
        assert_eq!(
            duration_text("0:59"),
            Some(std::time::Duration::from_secs(59))
        );
        assert_eq!(
            duration_text(" 10:00 "),
            Some(std::time::Duration::from_secs(600))
        );
        for invalid in [
            "",
            "59",
            "1:5",
            "1:60",
            "a:00",
            "1:00:00:00",
            "-1:00",
            "LIVE",
            "9999:99",
        ] {
            assert_eq!(duration_text(invalid), None, "{invalid}");
        }
    }
    #[test]
    fn missing_state_is_not_false_reconciliation() {
        assert!(rating(&json!({})).is_err());
        assert!(
            subscription_state(&json!({}), &ChannelId("UCabcdefghijklmnopqrstuv".into())).is_err()
        );
    }
}
