-- SPDX-License-Identifier: GPL-3.0-or-later
-- Local Home deduplicates newest surviving playlist memberships without
-- loading the whole library or consulting history/account metadata.
CREATE INDEX local_playlist_item_video ON local_playlist_items(video_id,id);
