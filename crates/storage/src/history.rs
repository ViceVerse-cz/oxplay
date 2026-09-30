// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use std::time::SystemTime;

/// Scoped to history ordering, never a provider/account continuation token.
#[derive(Clone)]
pub struct HistoryCursor {
    watched_at: i64,
    video_id: String,
}
pub struct HistoryEntry {
    pub video: VideoSummary,
    pub position: Duration,
    pub watched_at: SystemTime,
}

impl LocalStore {
    pub fn history_retention_days(&self) -> Result<u16> {
        Ok(self.connection.query_row(
            "SELECT history_retention_days FROM local_preferences WHERE id=1",
            [],
            |row| row.get(0),
        )?)
    }

    pub fn set_history_retention_days(&mut self, days: u16, now: SystemTime) -> Result<()> {
        if !(1..=365).contains(&days) {
            return Err(StorageError::InvalidInput);
        }
        let cutoff = timestamp(now)?.saturating_sub(i64::from(days) * 86400);
        let tx = self.connection.transaction()?;
        tx.execute(
            "UPDATE local_preferences SET history_retention_days=?1 WHERE id=1",
            [days],
        )?;
        tx.execute("DELETE FROM local_history WHERE watched_at<?1", [cutoff])?;
        tx.execute(
            "DELETE FROM local_search_history WHERE searched_at<?1",
            [cutoff],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Only records when the persisted history opt-in is true. Call periodically
    /// from a bounded worker schedule, not every decoded frame; URLs are omitted.
    pub fn record_history(
        &mut self,
        video: &VideoSummary,
        position: Duration,
        now: SystemTime,
    ) -> Result<bool> {
        if !self.preferences()?.privacy.local_history {
            return Ok(false);
        }
        validate_text(&video.title)?;
        validate_text(&video.channel)?;
        if let Some(id) = &video.channel_id {
            validate_channel(id)?;
        }
        let watched_at = timestamp(now)?;
        let position = i64::try_from(position.as_secs()).map_err(|_| StorageError::InvalidInput)?;
        let duration = video
            .duration
            .map(|d| i64::try_from(d.as_secs()).map_err(|_| StorageError::InvalidInput))
            .transpose()?;
        let cutoff = watched_at.saturating_sub(i64::from(self.history_retention_days()?) * 86400);
        let tx = self.connection.transaction()?;
        tx.execute("DELETE FROM local_history WHERE watched_at<?1", [cutoff])?;
        // Recheck opt-in in the statement, so another connection disabling it
        // cannot race a history write into a disabled database.
        let written = tx.execute("INSERT INTO local_history(video_id,title,channel_name,channel_id,duration_seconds,position_seconds,watched_at)
            SELECT ?1,?2,?3,?4,?5,?6,?7 WHERE (SELECT local_history FROM local_preferences WHERE id=1)=1
            ON CONFLICT(video_id) DO UPDATE SET title=excluded.title,channel_name=excluded.channel_name,channel_id=excluded.channel_id,duration_seconds=excluded.duration_seconds,position_seconds=excluded.position_seconds,watched_at=excluded.watched_at",
            params![video.id.as_str(),video.title,video.channel,video.channel_id.as_ref().map(|id| id.0.as_str()),duration,position,watched_at])?;
        tx.commit()?;
        Ok(written != 0)
    }

    /// Deletes expired entries even if no new playback has been recorded.
    /// Stored searches share the watch-history retention period.
    pub fn prune_history(&self, now: SystemTime) -> Result<usize> {
        let cutoff =
            timestamp(now)?.saturating_sub(i64::from(self.history_retention_days()?) * 86400);
        let searches = self.connection.execute(
            "DELETE FROM local_search_history WHERE searched_at<?1",
            [cutoff],
        )?;
        Ok(self
            .connection
            .execute("DELETE FROM local_history WHERE watched_at<?1", [cutoff])?
            + searches)
    }

    pub fn history(
        &self,
        after: Option<&HistoryCursor>,
        limit: u32,
        now: SystemTime,
    ) -> Result<(Vec<HistoryEntry>, Option<HistoryCursor>)> {
        page_bounds(None, limit)?;
        self.prune_history(now)?;
        if !self.preferences()?.privacy.local_history {
            return Ok((Vec::new(), None));
        }
        let mut statement = self.connection.prepare("SELECT video_id,title,channel_name,channel_id,duration_seconds,position_seconds,watched_at FROM local_history
            WHERE watched_at<?1 OR (watched_at=?1 AND video_id>?2) ORDER BY watched_at DESC,video_id ASC LIMIT ?3")?;
        let mut query = statement.query(params![
            after.map_or(i64::MAX, |c| c.watched_at),
            after.map_or("", |c| c.video_id.as_str()),
            limit + 1
        ])?;
        let mut rows = Vec::with_capacity((limit + 1) as usize);
        while let Some(row) = query.next()? {
            let id: String = row.get(0)?;
            let watched_at = row.get::<_, i64>(6)?;
            let seconds = u64::try_from(watched_at).map_err(|_| StorageError::CorruptData)?;
            let duration = row
                .get::<_, Option<i64>>(4)?
                .map(|s| {
                    u64::try_from(s)
                        .map(Duration::from_secs)
                        .map_err(|_| StorageError::CorruptData)
                })
                .transpose()?;
            let position =
                u64::try_from(row.get::<_, i64>(5)?).map_err(|_| StorageError::CorruptData)?;
            rows.push((
                HistoryCursor {
                    watched_at,
                    video_id: id.clone(),
                },
                HistoryEntry {
                    video: VideoSummary {
                        id: VideoId::new(&id).map_err(|_| StorageError::CorruptData)?,
                        title: row.get(1)?,
                        channel: row.get(2)?,
                        channel_id: row.get::<_, Option<String>>(3)?.map(ChannelId),
                        duration,
                        thumbnail_url: None,
                    },
                    position: Duration::from_secs(position),
                    watched_at: SystemTime::UNIX_EPOCH
                        .checked_add(Duration::from_secs(seconds))
                        .ok_or(StorageError::CorruptData)?,
                },
            ));
        }
        let more = rows.len() > limit as usize;
        rows.truncate(limit as usize);
        let next = more.then(|| rows.last().expect("nonempty history page").0.clone());
        Ok((rows.into_iter().map(|(_, entry)| entry).collect(), next))
    }
    pub fn delete_history_video(&self, id: &VideoId) -> Result<bool> {
        Ok(self
            .connection
            .execute("DELETE FROM local_history WHERE video_id=?1", [id.as_str()])?
            != 0)
    }
    /// Deletes watch history and submitted-search history together.
    pub fn clear_history(&self) -> Result<()> {
        self.connection
            .execute_batch("DELETE FROM local_history; DELETE FROM local_search_history;")?;
        Ok(())
    }

    /// Only records normalized plain-text searches while the persisted history
    /// opt-in is true; URL-like input is rejected. Keeps the newest 50 entries,
    /// deduplicated case-insensitively (the newest spelling wins).
    pub fn record_search(&mut self, query: &str, now: SystemTime) -> Result<bool> {
        if !self.preferences()?.privacy.local_history {
            return Ok(false);
        }
        let query = normalize_search_query(query).ok_or(StorageError::InvalidInput)?;
        let key = search_query_key(&query);
        let searched_at = timestamp(now)?;
        let cutoff = searched_at.saturating_sub(i64::from(self.history_retention_days()?) * 86400);
        let tx = self.connection.transaction()?;
        tx.execute(
            "DELETE FROM local_search_history WHERE searched_at<?1 OR query_key=?2",
            params![cutoff, key],
        )?;
        // Recheck opt-in in the statement, like watch history.
        let written = tx.execute(
            "INSERT INTO local_search_history(query_key,query,searched_at)
            SELECT ?1,?2,?3 WHERE (SELECT local_history FROM local_preferences WHERE id=1)=1",
            params![key, query, searched_at],
        )?;
        tx.execute(
            "DELETE FROM local_search_history WHERE id NOT IN
            (SELECT id FROM local_search_history ORDER BY id DESC LIMIT ?1)",
            [MAX_SEARCH_HISTORY as i64],
        )?;
        tx.commit()?;
        Ok(written != 0)
    }

    /// Newest first; empty while history is disabled. Expired rows are pruned.
    pub fn search_history(&self, now: SystemTime) -> Result<Vec<String>> {
        self.prune_history(now)?;
        if !self.preferences()?.privacy.local_history {
            return Ok(Vec::new());
        }
        let mut statement = self
            .connection
            .prepare("SELECT query FROM local_search_history ORDER BY id DESC LIMIT ?1")?;
        let rows = statement
            .query_map([MAX_SEARCH_HISTORY as i64], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|query| normalize_search_query(&query).ok_or(StorageError::CorruptData))
            .collect()
    }

    /// Removes one stored search by its case-insensitive identity.
    pub fn delete_search(&self, query: &str) -> Result<bool> {
        let Some(query) = normalize_search_query(query) else {
            return Ok(false);
        };
        Ok(self.connection.execute(
            "DELETE FROM local_search_history WHERE query_key=?1",
            [search_query_key(&query)],
        )? != 0)
    }
}

/// Bound for stored and in-memory submitted searches.
pub const MAX_SEARCH_HISTORY: usize = 50;
/// Same limit as a typed catalog search.
pub const MAX_SEARCH_QUERY_CHARS: usize = 200;

/// Trims and collapses whitespace. Rejects empty, oversized, control-character
/// and URL-like input, so pasted links (which may carry tokens) are never kept.
pub fn normalize_search_query(query: &str) -> Option<String> {
    if query.len() > 4 * 1024 || query.chars().any(char::is_control) {
        return None;
    }
    let query = query.split_whitespace().collect::<Vec<_>>().join(" ");
    let count = query.chars().count();
    if count == 0 || count > MAX_SEARCH_QUERY_CHARS || query.len() > MAX_TEXT_BYTES {
        return None;
    }
    let lower = query.to_lowercase();
    let url_like = lower.contains("://")
        || lower.starts_with("www.")
        || lower.starts_with("https:")
        || lower.starts_with("http:")
        || ["youtube.com/", "youtu.be/", "youtube-nocookie.com/"]
            .iter()
            .any(|host| lower.contains(host));
    (!url_like).then_some(query)
}

/// Case-insensitive identity of an already normalized query.
pub fn search_query_key(normalized: &str) -> String {
    normalized.to_lowercase()
}
fn timestamp(time: SystemTime) -> Result<i64> {
    i64::try_from(
        time.duration_since(SystemTime::UNIX_EPOCH)
            .map_err(|_| StorageError::InvalidInput)?
            .as_secs(),
    )
    .map_err(|_| StorageError::InvalidInput)
}
