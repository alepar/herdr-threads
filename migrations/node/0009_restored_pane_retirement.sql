-- Restored-pane retirement (ht-wpq4, TRUST-POLICY C2): the pane a coherent
-- publication showed at an unresolved (reason other) seat's saved target while
-- that target was held in the recovery baseline, watched until a coherent
-- publication of the same incarnation shows neither the target nor its
-- terminal for long enough. Local, never replicated; one row per seat. A row
-- of another incarnation or seat generation is ignored and replaced.
CREATE TABLE restored_pane_watch (
    seat_id TEXT PRIMARY KEY REFERENCES seats(id),
    host_id INTEGER NOT NULL REFERENCES herdr_hosts(host_id),
    host_boot TEXT NOT NULL,
    incarnation TEXT NOT NULL,
    target_id TEXT NOT NULL,
    terminal_id TEXT NOT NULL,
    target_generation INTEGER NOT NULL CHECK(target_generation > 0),
    seat_generation INTEGER NOT NULL CHECK(seat_generation >= 0),
    recorded_at INTEGER NOT NULL,
    absent_seen_at INTEGER,
    absent_observed_ms INTEGER NOT NULL DEFAULT 0 CHECK(absent_observed_ms >= 0)
) STRICT;
-- The evidence a local retirement rests on; NULL for every path but the
-- restored-pane rule (same-incarnation pane close, operator, replicated).
ALTER TABLE retirements ADD COLUMN closure_evidence TEXT
    CHECK(closure_evidence IS NULL OR closure_evidence IN ('restored_pane_closed'));
