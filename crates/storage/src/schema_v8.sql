-- SPDX-License-Identifier: GPL-3.0-or-later
-- Remote search completions have their own visible preference (initially on).
-- Submitted plain-text searches share the opt-in local-history switch,
-- retention and deletion. At most 50 are kept; the key is case-folded.
ALTER TABLE local_preferences ADD COLUMN search_suggestions INTEGER NOT NULL DEFAULT 1 CHECK(search_suggestions IN (0,1));
CREATE TABLE local_search_history (
    id INTEGER PRIMARY KEY,
    query_key TEXT NOT NULL UNIQUE CHECK(length(CAST(query_key AS BLOB)) BETWEEN 1 AND 4096),
    query TEXT NOT NULL CHECK(length(query) BETWEEN 1 AND 200 AND length(CAST(query AS BLOB)) <= 1024),
    searched_at INTEGER NOT NULL CHECK(searched_at>=0)
) STRICT;
-- Disabling local history deletes stored searches (and their UNIQUE index
-- entries) in the same preferences transaction as watch history.
CREATE TRIGGER disable_local_search_history AFTER UPDATE OF local_history ON local_preferences
WHEN NEW.local_history=0 BEGIN DELETE FROM local_search_history; END;
