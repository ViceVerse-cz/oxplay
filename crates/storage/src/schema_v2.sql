-- SPDX-License-Identifier: GPL-3.0-or-later
CREATE TABLE local_preferences (
    id INTEGER PRIMARY KEY CHECK(id=1),
    local_history INTEGER NOT NULL DEFAULT 0 CHECK(local_history IN (0,1)),
    autoplay INTEGER NOT NULL DEFAULT 0 CHECK(autoplay IN (0,1)),
    thumbnail_previews INTEGER NOT NULL DEFAULT 0 CHECK(thumbnail_previews IN (0,1)),
    background_refresh INTEGER NOT NULL DEFAULT 0 CHECK(background_refresh IN (0,1)),
    telemetry INTEGER NOT NULL DEFAULT 0 CHECK(telemetry IN (0,1)),
    volume_percent INTEGER NOT NULL DEFAULT 100 CHECK(volume_percent BETWEEN 0 AND 100),
    theme TEXT NOT NULL DEFAULT 'system' CHECK(theme IN ('system','light','dark'))
) STRICT;
INSERT INTO local_preferences(id) VALUES(1);
