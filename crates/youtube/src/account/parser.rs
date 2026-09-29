//! Bounded normalization of reviewed InnerTube renderer shapes. Unknown shapes
//! yield partial results or an unsupported error, never synthetic account success.
use super::*;
use std::collections::HashSet;
const MAX_NODES: usize = 50_000;
const MAX_ITEMS: usize = 200;
fn nodes(value: &Value) -> Result<Vec<(&str, &Value)>, AccountError> {
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
                    stack.push(value);
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
fn text(value: Option<&Value>) -> String {
    let Some(value) = value else {
        return String::new();
    };
    if let Some(value) = value
        .as_str()
        .or_else(|| value.get("simpleText")?.as_str())
        .or_else(|| value.get("content")?.as_str())
    {
        return value.chars().take(500).collect();
    }
    value
        .get("runs")
        .and_then(Value::as_array)
        .map(|runs| {
            runs.iter()
                .filter_map(|r| r.get("text")?.as_str())
                .flat_map(str::chars)
                .take(500)
                .collect()
        })
        .unwrap_or_default()
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
    #[test]
    fn missing_state_is_not_false_reconciliation() {
        assert!(rating(&json!({})).is_err());
        assert!(
            subscription_state(&json!({}), &ChannelId("UCabcdefghijklmnopqrstuv".into())).is_err()
        );
    }
}
