-- An independent lifecycle fork is a concurrent session, not a same-process
-- replacement. Persist this fact through reconnects and daemon restarts.
ALTER TABLE occupant_bindings ADD COLUMN concurrent_session INTEGER NOT NULL DEFAULT 0
    CHECK(concurrent_session IN (0,1) AND (concurrent_session=0 OR (role='top_level' AND harness<>'human')));
