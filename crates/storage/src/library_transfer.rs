// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use serde::{Deserialize, Serialize};

const TRANSFER_VERSION: u32 = 1;
pub const MAX_TRANSFER_BYTES: usize = 16 * 1024 * 1024;
const MAX_PLAYLISTS: usize = 1000;
const MAX_VIDEOS: usize = 50_000;
const MAX_SUBSCRIPTIONS: usize = 10_000;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Transfer {
    serein_local_library_version: u32,
    subscriptions: Vec<Subscription>,
    playlists: Vec<Playlist>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Subscription {
    channel_id: String,
    name: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Playlist {
    name: String,
    videos: Vec<Video>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Video {
    id: String,
    title: String,
    channel: String,
    channel_id: Option<String>,
    duration_seconds: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportSummary {
    pub playlists_created: usize,
    pub videos_saved: usize,
    pub channels_followed: usize,
}

impl LocalStore {
    /// Explicit local-only export, bounded to 16 MiB, 1,000 playlists, 50,000
    /// videos and 10,000 subscriptions. Contains browsing interests. No account
    /// material, history or preferences are exported. Call on a worker thread.
    pub fn export_library_json(&self) -> Result<Vec<u8>> {
        let snapshot = self.connection.unchecked_transaction()?;
        let mut transfer = Transfer {
            serein_local_library_version: TRANSFER_VERSION,
            subscriptions: Vec::new(),
            playlists: Vec::new(),
        };
        let mut cursor = None;
        let mut retained_bytes = 0;
        loop {
            let page = self.subscriptions(cursor, MAX_PAGE_SIZE)?;
            for subscription in page.items {
                retained_bytes += subscription.channel_id.0.len() + subscription.name.len();
                transfer.subscriptions.push(Subscription {
                    channel_id: subscription.channel_id.0,
                    name: subscription.name,
                });
            }
            if transfer.subscriptions.len() > MAX_SUBSCRIPTIONS
                || retained_bytes > MAX_TRANSFER_BYTES
            {
                return Err(StorageError::InvalidInput);
            }
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        let mut count = 0;
        loop {
            let page = self.playlists(cursor, MAX_PAGE_SIZE)?;
            for local_playlist in page.items {
                let mut playlist = Playlist {
                    name: local_playlist.name,
                    videos: Vec::new(),
                };
                retained_bytes += playlist.name.len();
                let mut video_cursor = None;
                loop {
                    let videos =
                        self.playlist_videos(local_playlist.id, video_cursor, MAX_PAGE_SIZE)?;
                    for video in videos.items {
                        count += 1;
                        retained_bytes += video.id.as_str().len()
                            + video.title.len()
                            + video.channel.len()
                            + video.channel_id.as_ref().map_or(0, |id| id.0.len());
                        if count > MAX_VIDEOS || retained_bytes > MAX_TRANSFER_BYTES {
                            return Err(StorageError::InvalidInput);
                        }
                        playlist.videos.push(Video {
                            id: video.id.as_str().to_owned(),
                            title: video.title,
                            channel: video.channel,
                            channel_id: video.channel_id.map(|id| id.0),
                            duration_seconds: video.duration.map(|d| d.as_secs()),
                        });
                    }
                    video_cursor = videos.next;
                    if video_cursor.is_none() {
                        break;
                    }
                }
                transfer.playlists.push(playlist);
                if transfer.playlists.len() > MAX_PLAYLISTS || retained_bytes > MAX_TRANSFER_BYTES {
                    return Err(StorageError::InvalidInput);
                }
            }
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        // serde's writer is capped before growth; escaped text cannot evade cap.
        struct Bounded(Vec<u8>);
        impl std::io::Write for Bounded {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if bytes.len() > MAX_TRANSFER_BYTES - self.0.len() {
                    return Err(std::io::Error::other("local export size limit"));
                }
                self.0.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut output = Bounded(Vec::new());
        serde_json::to_writer(&mut output, &transfer).map_err(|_| StorageError::InvalidInput)?;
        snapshot.commit()?;
        Ok(output.0)
    }

    /// Imports reviewed local-only data atomically. Creates new local playlists
    /// and merges follows by channel ID; never enables history or sends requests.
    /// The caller must bound file reads to 16 MiB before allocating `bytes`.
    pub fn import_library_json(&mut self, bytes: &[u8]) -> Result<ImportSummary> {
        if bytes.is_empty() || bytes.len() > MAX_TRANSFER_BYTES {
            return Err(StorageError::InvalidInput);
        }
        let transfer: Transfer =
            serde_json::from_slice(bytes).map_err(|_| StorageError::InvalidInput)?;
        if transfer.serein_local_library_version != TRANSFER_VERSION
            || transfer.playlists.len() > MAX_PLAYLISTS
            || transfer.subscriptions.len() > MAX_SUBSCRIPTIONS
        {
            return Err(StorageError::InvalidInput);
        }
        let mut count = 0usize;
        let mut channels = std::collections::HashSet::new();
        for subscription in &transfer.subscriptions {
            if !channels.insert(&subscription.channel_id) {
                return Err(StorageError::InvalidInput);
            }
            validate_channel(&ChannelId(subscription.channel_id.clone()))?;
            validate_text(&subscription.name)?;
        }
        for playlist in &transfer.playlists {
            let mut videos = std::collections::HashSet::new();
            validate_text(&playlist.name)?;
            count += playlist.videos.len();
            if count > MAX_VIDEOS {
                return Err(StorageError::InvalidInput);
            }
            for video in &playlist.videos {
                if !videos.insert(&video.id) {
                    return Err(StorageError::InvalidInput);
                }
                VideoId::new(&video.id).map_err(|_| StorageError::InvalidInput)?;
                validate_text(&video.title)?;
                validate_text(&video.channel)?;
                if let Some(id) = &video.channel_id {
                    validate_channel(&ChannelId(id.clone()))?;
                }
                if video
                    .duration_seconds
                    .is_some_and(|seconds| seconds > i64::MAX as u64)
                {
                    return Err(StorageError::InvalidInput);
                }
            }
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        for subscription in &transfer.subscriptions {
            tx.execute("INSERT INTO local_subscriptions(channel_id,name) VALUES (?1,?2) ON CONFLICT(channel_id) DO UPDATE SET name=excluded.name",params![subscription.channel_id,subscription.name])?;
        }
        for playlist in &transfer.playlists {
            tx.execute(
                "INSERT INTO local_playlists(name) VALUES (?1)",
                [&playlist.name],
            )?;
            let id = tx.last_insert_rowid();
            for video in &playlist.videos {
                tx.execute("INSERT INTO local_playlist_items(playlist_id,video_id,title,channel_name,channel_id,duration_seconds) VALUES (?1,?2,?3,?4,?5,?6)
                    ON CONFLICT(playlist_id,video_id) DO UPDATE SET title=excluded.title,channel_name=excluded.channel_name,channel_id=excluded.channel_id,duration_seconds=excluded.duration_seconds",
                    params![id,video.id,video.title,video.channel,video.channel_id,video.duration_seconds.map(|s| s as i64)])?;
            }
        }
        tx.commit()?;
        Ok(ImportSummary {
            playlists_created: transfer.playlists.len(),
            videos_saved: count,
            channels_followed: transfer.subscriptions.len(),
        })
    }
}
