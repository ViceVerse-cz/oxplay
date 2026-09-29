-- SPDX-License-Identifier: GPL-3.0-or-later
-- Explicit product preference: public comments after guest selection, never at launch.
ALTER TABLE local_preferences ADD COLUMN comments_enabled INTEGER NOT NULL DEFAULT 1 CHECK(comments_enabled IN (0,1));
