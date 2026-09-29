// SPDX-License-Identifier: GPL-3.0-or-later
//! Local-only library persistence. Own this synchronous connection on a worker
//! thread, never the Slint event-loop thread. LocalStore has no account or secret
//! API. The separate vault module protects explicitly imported opaque sessions.
//! Public errors never echo SQL, credentials, or input values.
mod backup;
mod history;
mod library_transfer;
pub mod vault;
pub use history::{HistoryCursor, HistoryEntry};
pub use library_transfer::{ImportSummary, MAX_TRANSFER_BYTES};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serein_core::{
    ChannelId, PlaybackPreferences, PlaybackSpeed, Preferences, QualityCeiling, VideoId,
    VideoSummary,
};
use std::{fmt, path::Path, time::Duration};

const SCHEMA_VERSION: u32 = 5;
pub const MAX_PAGE_SIZE: u32 = 100;
const MAX_TEXT_BYTES: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageError {
    InvalidInput,
    Unavailable,
    NewerSchema,
    CorruptData,
    NotFound,
    BackupExists,
}
impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidInput => "The local library value is invalid or too large.",
            Self::Unavailable => "Local library storage is unavailable.",
            Self::NewerSchema => "This library was created by a newer application version.",
            Self::CorruptData => "Local library data is damaged or unsupported. Restore a backup.",
            Self::NotFound => "The local collection no longer exists.",
            Self::BackupExists => "Choose a new backup file; the destination already exists.",
        })
    }
}
impl std::error::Error for StorageError {}
impl From<rusqlite::Error> for StorageError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Unavailable
    }
}
pub type Result<T> = std::result::Result<T, StorageError>;

/// Separate from a remote YouTube PlaylistId; cannot be used for account writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalPlaylistId(i64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalPlaylist {
    pub id: LocalPlaylistId,
    pub name: String,
}

#[derive(Clone)]
pub struct LocalSubscription {
    pub channel_id: ChannelId,
    pub name: String,
}

/// A cursor belongs only to the query/collection that produced it. It is not an
/// offset, so deleting earlier rows does not cause later rows to be skipped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageCursor(i64);

pub struct Page<T> {
    pub items: Vec<T>,
    pub next: Option<PageCursor>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalPreferences {
    pub privacy: Preferences,
    pub volume_percent: u8,
    pub theme: Theme,
    pub playback: PlaybackPreferences,
}
impl Default for LocalPreferences {
    fn default() -> Self {
        Self {
            privacy: Preferences::default(),
            volume_percent: 100,
            theme: Theme::System,
            playback: PlaybackPreferences::default(),
        }
    }
}

pub struct LocalStore {
    connection: Connection,
}

impl LocalStore {
    /// The caller chooses an application-private path and creates its directory.
    /// This opens only a local database, never URI parameters or remote locations.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Ok(metadata) = std::fs::symlink_metadata(path)
            && !metadata.is_file()
        {
            return Err(StorageError::InvalidInput);
        }
        let connection = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
                | rusqlite::OpenFlags::SQLITE_OPEN_CREATE
                | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        Self::initialize(connection)
    }

    pub fn in_memory() -> Result<Self> {
        Self::initialize(Connection::open_in_memory()?)
    }

    fn initialize(mut connection: Connection) -> Result<Self> {
        connection.busy_timeout(Duration::from_secs(2))?;
        connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA trusted_schema=OFF; PRAGMA cache_size=-2048; PRAGMA mmap_size=0;")?;
        migrate(&mut connection)?;
        let store = Self { connection };
        store.prune_history(std::time::SystemTime::now())?;
        Ok(store)
    }

    pub fn schema_version(&self) -> Result<u32> {
        Ok(self
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))?)
    }

    pub fn create_playlist(&self, name: &str) -> Result<LocalPlaylistId> {
        validate_text(name)?;
        self.connection
            .execute("INSERT INTO local_playlists(name) VALUES (?1)", [name])?;
        Ok(LocalPlaylistId(self.connection.last_insert_rowid()))
    }

    /// One explicit local action: either both the collection and its first
    /// video commit, or neither does. A failed video insert leaves no empty
    /// collection behind, including failures after the collection insert.
    pub fn create_playlist_with_video(
        &mut self,
        name: &str,
        video: &VideoSummary,
    ) -> Result<LocalPlaylist> {
        validate_text(name)?;
        let tx = self.connection.transaction()?;
        tx.execute("INSERT INTO local_playlists(name) VALUES (?1)", [name])?;
        let id = LocalPlaylistId(tx.last_insert_rowid());
        save_video_in(&tx, id, video)?;
        tx.commit()?;
        Ok(LocalPlaylist {
            id,
            name: name.to_owned(),
        })
    }

    pub fn rename_playlist(&self, id: LocalPlaylistId, name: &str) -> Result<()> {
        validate_text(name)?;
        if self.connection.execute(
            "UPDATE local_playlists SET name=?1 WHERE id=?2",
            params![name, id.0],
        )? == 0
        {
            return Err(StorageError::NotFound);
        }
        Ok(())
    }

    /// Cascades only this local playlist's items, never remote resources.
    pub fn delete_playlist(&self, id: LocalPlaylistId) -> Result<bool> {
        Ok(self
            .connection
            .execute("DELETE FROM local_playlists WHERE id=?1", [id.0])?
            != 0)
    }

    pub fn playlists(&self, after: Option<PageCursor>, limit: u32) -> Result<Page<LocalPlaylist>> {
        let (after, fetch) = page_bounds(after, limit)?;
        let mut statement = self
            .connection
            .prepare("SELECT id,name FROM local_playlists WHERE id>?1 ORDER BY id LIMIT ?2")?;
        let rows = statement
            .query_map(params![after, fetch], |row| {
                let id = row.get(0)?;
                Ok((
                    id,
                    LocalPlaylist {
                        id: LocalPlaylistId(id),
                        name: row.get(1)?,
                    },
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(finish_page(rows, limit))
    }

    /// Saving the same video again updates its title without changing row identity.
    /// Signed/thumbnail URLs are deliberately not persisted.
    pub fn save_video(&self, playlist: LocalPlaylistId, video: &VideoSummary) -> Result<()> {
        save_video_in(&self.connection, playlist, video)
    }

    pub fn remove_video(&self, playlist: LocalPlaylistId, video: &VideoId) -> Result<bool> {
        Ok(self.connection.execute(
            "DELETE FROM local_playlist_items WHERE playlist_id=?1 AND video_id=?2",
            params![playlist.0, video.as_str()],
        )? != 0)
    }

    pub fn playlist_videos(
        &self,
        playlist: LocalPlaylistId,
        after: Option<PageCursor>,
        limit: u32,
    ) -> Result<Page<VideoSummary>> {
        let (after, fetch) = page_bounds(after, limit)?;
        let mut statement = self.connection.prepare("SELECT id,video_id,title,channel_name,channel_id,duration_seconds FROM local_playlist_items WHERE playlist_id=?1 AND id>?2 ORDER BY id LIMIT ?3")?;
        let mut query = statement.query(params![playlist.0, after, fetch])?;
        let mut rows = Vec::with_capacity(fetch as usize);
        while let Some(row) = query.next()? {
            rows.push((row.get(0)?, local_video_from_row(row)?));
        }
        Ok(finish_page(rows, limit))
    }

    /// Local Home, ordered by each video's newest surviving playlist membership.
    /// Re-saving an existing membership updates metadata without changing its
    /// order. The cursor is exclusive and descending, unlike playlist cursors.
    /// Pages are live reads, not a transaction spanning UI navigation: callers
    /// should discard their page cursors after local collection mutations.
    /// Neither history nor account/cache data participates; URLs are never read.
    pub fn recently_saved_videos(
        &self,
        after: Option<PageCursor>,
        limit: u32,
    ) -> Result<Page<VideoSummary>> {
        let (_, fetch) = page_bounds(after, limit)?;
        let upper = after.map_or(i64::MAX, |cursor| cursor.0.saturating_sub(1));
        // Scan the primary key backwards and probe the covering video/id index.
        // Filter newer memberships before applying LIMIT, including memberships
        // beyond the cursor, so old copies cannot leak onto subsequent pages.
        let mut statement = self.connection.prepare(
            "SELECT item.id,item.video_id,item.title,item.channel_name,item.channel_id,item.duration_seconds
             FROM local_playlist_items AS item
             WHERE item.id<=?1 AND NOT EXISTS (
                 SELECT 1 FROM local_playlist_items AS newer
                 WHERE newer.video_id=item.video_id AND newer.id>item.id
             ) ORDER BY item.id DESC LIMIT ?2",
        )?;
        let mut query = statement.query(params![upper, fetch])?;
        let mut rows = Vec::with_capacity(fetch as usize);
        while let Some(row) = query.next()? {
            rows.push((row.get(0)?, local_video_from_row(row)?));
        }
        Ok(finish_page(rows, limit))
    }

    /// This follows a channel locally; it never subscribes a YouTube account.
    pub fn follow_channel(&self, channel: &ChannelId, name: &str) -> Result<()> {
        validate_channel(channel)?;
        validate_text(name)?;
        self.connection.execute("INSERT INTO local_subscriptions(channel_id,name) VALUES (?1,?2) ON CONFLICT(channel_id) DO UPDATE SET name=excluded.name",params![channel.0,name])?;
        Ok(())
    }

    pub fn unfollow_channel(&self, channel: &ChannelId) -> Result<bool> {
        validate_channel(channel)?;
        Ok(self.connection.execute(
            "DELETE FROM local_subscriptions WHERE channel_id=?1",
            [&channel.0],
        )? != 0)
    }

    pub fn subscriptions(
        &self,
        after: Option<PageCursor>,
        limit: u32,
    ) -> Result<Page<LocalSubscription>> {
        let (after, fetch) = page_bounds(after, limit)?;
        let mut statement = self.connection.prepare(
            "SELECT id,channel_id,name FROM local_subscriptions WHERE id>?1 ORDER BY id LIMIT ?2",
        )?;
        let rows = statement
            .query_map(params![after, fetch], |row| {
                Ok((
                    row.get(0)?,
                    LocalSubscription {
                        channel_id: ChannelId(row.get(1)?),
                        name: row.get(2)?,
                    },
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(finish_page(rows, limit))
    }

    pub fn preferences(&self) -> Result<LocalPreferences> {
        let values = self.connection.query_row("SELECT local_history,autoplay,thumbnail_previews,background_refresh,telemetry,volume_percent,theme,quality_height,speed_millis FROM local_preferences WHERE id=1",[],|row| {
            Ok((Preferences { local_history: row.get(0)?,autoplay: row.get(1)?,thumbnail_previews: row.get(2)?,background_refresh: row.get(3)?,telemetry: row.get(4)? },row.get::<_,u8>(5)?,row.get::<_,String>(6)?,row.get::<_,u16>(7)?,row.get::<_,u16>(8)?))
        }).optional()?.ok_or(StorageError::CorruptData)?;
        Ok(LocalPreferences {
            privacy: values.0,
            volume_percent: values.1,
            theme: match values.2.as_str() {
                "system" => Theme::System,
                "light" => Theme::Light,
                "dark" => Theme::Dark,
                _ => return Err(StorageError::CorruptData),
            },
            playback: PlaybackPreferences {
                quality: QualityCeiling::from_height(values.3).ok_or(StorageError::CorruptData)?,
                speed: PlaybackSpeed::from_millis(values.4).ok_or(StorageError::CorruptData)?,
            },
        })
    }

    pub fn set_preferences(&self, prefs: LocalPreferences) -> Result<()> {
        if prefs.volume_percent > 100 {
            return Err(StorageError::InvalidInput);
        }
        let theme = match prefs.theme {
            Theme::System => "system",
            Theme::Light => "light",
            Theme::Dark => "dark",
        };
        self.connection.execute("UPDATE local_preferences SET local_history=?1,autoplay=?2,thumbnail_previews=?3,background_refresh=?4,telemetry=?5,volume_percent=?6,theme=?7,quality_height=?8,speed_millis=?9 WHERE id=1",params![prefs.privacy.local_history,prefs.privacy.autoplay,prefs.privacy.thumbnail_previews,prefs.privacy.background_refresh,prefs.privacy.telemetry,prefs.volume_percent,theme,prefs.playback.quality.height(),prefs.playback.speed.millis()])?;
        Ok(())
    }

    /// Explicit user-requested deletion of local collections and preferences.
    /// Not forensic erasure and not a YouTube account operation.
    pub fn clear_local_data(&mut self) -> Result<()> {
        let tx = self.connection.transaction()?;
        tx.execute_batch("DELETE FROM local_playlist_items; DELETE FROM local_playlists; DELETE FROM local_subscriptions; DELETE FROM local_history; DELETE FROM local_preferences; INSERT INTO local_preferences(id) VALUES(1);")?;
        tx.commit()?;
        Ok(())
    }
}

fn local_video_from_row(row: &rusqlite::Row<'_>) -> Result<VideoSummary> {
    let video_id: String = row.get(1)?;
    let seconds = row
        .get::<_, Option<i64>>(5)?
        .map(|seconds| u64::try_from(seconds).map_err(|_| StorageError::CorruptData))
        .transpose()?;
    Ok(VideoSummary {
        id: VideoId::new(&video_id).map_err(|_| StorageError::CorruptData)?,
        title: row.get(2)?,
        channel: row.get(3)?,
        channel_id: row.get::<_, Option<String>>(4)?.map(ChannelId),
        duration: seconds.map(Duration::from_secs),
        thumbnail_url: None,
    })
}

fn save_video_in(
    connection: &Connection,
    playlist: LocalPlaylistId,
    video: &VideoSummary,
) -> Result<()> {
    validate_text(&video.title)?;
    validate_text(&video.channel)?;
    if let Some(channel) = &video.channel_id {
        validate_channel(channel)?;
    }
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM local_playlists WHERE id=?1)",
        [playlist.0],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(StorageError::NotFound);
    }
    let duration = video
        .duration
        .map(|d| i64::try_from(d.as_secs()).map_err(|_| StorageError::InvalidInput))
        .transpose()?;
    connection.execute("INSERT INTO local_playlist_items(playlist_id,video_id,title,channel_name,channel_id,duration_seconds) VALUES (?1,?2,?3,?4,?5,?6)
        ON CONFLICT(playlist_id,video_id) DO UPDATE SET title=excluded.title,channel_name=excluded.channel_name,channel_id=excluded.channel_id,duration_seconds=excluded.duration_seconds",
        params![playlist.0,video.id.as_str(),video.title,video.channel,video.channel_id.as_ref().map(|c| c.0.as_str()),duration])?;
    Ok(())
}

fn validate_text(value: &str) -> Result<()> {
    if value.trim().is_empty() || value.len() > MAX_TEXT_BYTES || value.contains('\0') {
        Err(StorageError::InvalidInput)
    } else {
        Ok(())
    }
}
fn validate_channel(value: &ChannelId) -> Result<()> {
    if value.0.len() == 24
        && value.0.starts_with("UC")
        && value
            .0
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        Ok(())
    } else {
        Err(StorageError::InvalidInput)
    }
}
fn page_bounds(after: Option<PageCursor>, limit: u32) -> Result<(i64, u32)> {
    if !(1..=MAX_PAGE_SIZE).contains(&limit) {
        return Err(StorageError::InvalidInput);
    }
    Ok((after.map_or(0, |cursor| cursor.0), limit + 1))
}
fn finish_page<T>(mut rows: Vec<(i64, T)>, limit: u32) -> Page<T> {
    let has_more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next = has_more.then(|| PageCursor(rows.last().expect("nonempty bounded page").0));
    Page {
        items: rows.into_iter().map(|(_, item)| item).collect(),
        next,
    }
}

fn migrate(connection: &mut Connection) -> Result<()> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let version: u32 = tx.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version > SCHEMA_VERSION {
        return Err(StorageError::NewerSchema);
    }
    if version < 1 {
        tx.execute_batch(include_str!("schema_v1.sql"))?;
    }
    if version < 2 {
        tx.execute_batch(include_str!("schema_v2.sql"))?;
    }
    if version < 3 {
        tx.execute_batch(include_str!("schema_v3.sql"))?;
    }
    if version < 4 {
        tx.execute_batch(include_str!("schema_v4.sql"))?;
    }
    if version < 5 {
        tx.execute_batch(include_str!("schema_v5.sql"))?;
    }
    tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests;
