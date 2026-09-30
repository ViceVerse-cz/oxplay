-- SPDX-License-Identifier: GPL-3.0-or-later
-- Widen the maximum-quality ceiling to 1440p and 2160p. SQLite cannot alter
-- a column CHECK, so copy into a replacement column; saved heights survive.
ALTER TABLE local_preferences ADD COLUMN quality_height_v9 INTEGER NOT NULL DEFAULT 1080 CHECK(quality_height_v9 IN (2160,1440,1080,720,480,360,240,144));
UPDATE local_preferences SET quality_height_v9=quality_height;
ALTER TABLE local_preferences DROP COLUMN quality_height;
ALTER TABLE local_preferences RENAME COLUMN quality_height_v9 TO quality_height;
