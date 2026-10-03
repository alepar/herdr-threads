-- Catch-up release key (epic ht-1ip, spec §7 "release is a push"): the
-- decision sequence allocated when a row ends; released receipts above the
-- row's frontier take it as their attention key.
ALTER TABLE catch_up ADD COLUMN release_seq INTEGER CHECK(release_seq IS NULL OR release_seq > 0);
