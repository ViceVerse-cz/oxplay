-- SPDX-License-Identifier: GPL-3.0-or-later
ALTER TABLE local_preferences ADD COLUMN quality_height INTEGER NOT NULL DEFAULT 1080 CHECK(quality_height IN (1080,720,480,360,240,144));
ALTER TABLE local_preferences ADD COLUMN speed_millis INTEGER NOT NULL DEFAULT 1000 CHECK(speed_millis IN (500,1000,1500,2000));
