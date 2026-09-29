-- SPDX-License-Identifier: GPL-3.0-or-later
ALTER TABLE local_preferences ADD COLUMN history_retention_days INTEGER NOT NULL DEFAULT 30 CHECK(history_retention_days BETWEEN 1 AND 365);
CREATE TABLE local_history (
    video_id TEXT PRIMARY KEY CHECK(length(video_id)=11),
    title TEXT NOT NULL CHECK(length(CAST(title AS BLOB)) BETWEEN 1 AND 1024),
    channel_name TEXT NOT NULL CHECK(length(CAST(channel_name AS BLOB)) BETWEEN 1 AND 1024),
    channel_id TEXT CHECK(channel_id IS NULL OR length(channel_id)=24),
    duration_seconds INTEGER CHECK(duration_seconds IS NULL OR duration_seconds>=0),
    position_seconds INTEGER NOT NULL CHECK(position_seconds>=0),
    watched_at INTEGER NOT NULL CHECK(watched_at>=0)
) STRICT;
CREATE INDEX local_history_page ON local_history(watched_at DESC,video_id);
-- Disabling local history also clears rows and their SQLite index entries in
-- the same preferences transaction. No persisted derived recommendations exist.
CREATE TRIGGER disable_local_history AFTER UPDATE OF local_history ON local_preferences
WHEN NEW.local_history=0 BEGIN DELETE FROM local_history; END;
