-- SPDX-License-Identifier: GPL-3.0-or-later
-- Public thumbnail bytes are kept separately from the library. Zero opts out
-- and requests cache clearing; migrations preserve every existing local item.
ALTER TABLE local_preferences ADD COLUMN thumbnail_cache_mib INTEGER NOT NULL DEFAULT 256 CHECK(thumbnail_cache_mib IN (0,32,128,256));
