-- SPDX-License-Identifier: GPL-3.0-or-later
-- Ambient-mode glow preferences. The glow is on by default; the size index
-- follows oxplay_core::GlowSize::ALL (0 small, 1 medium, 2 large, 3 extra large).
ALTER TABLE local_preferences ADD COLUMN ambient_mode INTEGER NOT NULL DEFAULT 1 CHECK(ambient_mode IN (0,1));
ALTER TABLE local_preferences ADD COLUMN glow_size INTEGER NOT NULL DEFAULT 1 CHECK(glow_size BETWEEN 0 AND 3);
