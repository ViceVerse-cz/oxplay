-- SPDX-License-Identifier: GPL-3.0-or-later
CREATE TABLE local_playlists (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL CHECK(length(CAST(name AS BLOB)) BETWEEN 1 AND 1024)
) STRICT;
CREATE TABLE local_playlist_items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    playlist_id INTEGER NOT NULL REFERENCES local_playlists(id) ON DELETE CASCADE,
    video_id TEXT NOT NULL CHECK(length(video_id)=11),
    title TEXT NOT NULL CHECK(length(CAST(title AS BLOB)) BETWEEN 1 AND 1024),
    channel_name TEXT NOT NULL CHECK(length(CAST(channel_name AS BLOB)) BETWEEN 1 AND 1024),
    channel_id TEXT CHECK(channel_id IS NULL OR length(channel_id)=24),
    duration_seconds INTEGER CHECK(duration_seconds IS NULL OR duration_seconds>=0),
    UNIQUE(playlist_id,video_id)
) STRICT;
CREATE INDEX local_playlist_item_page ON local_playlist_items(playlist_id,id);
CREATE TABLE local_subscriptions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    channel_id TEXT NOT NULL UNIQUE CHECK(length(channel_id)=24),
    name TEXT NOT NULL CHECK(length(CAST(name AS BLOB)) BETWEEN 1 AND 1024)
) STRICT;
