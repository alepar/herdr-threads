-- Per-binding delivery state (ht-54r, spec §2 'What happens to today's per-seat delivery state').
-- Executed in one transaction by schema::migrate_v33_to_v34.
-- Lazy display state moves to the binding cursors; lazy rows still pending at upgrade are carried
-- as legacy pending (a cursor cannot say "displayed ordinary 5 and 6 but not lazy 4").
ALTER TABLE lazy_recipients ADD COLUMN legacy_pending INTEGER NOT NULL DEFAULT 0 CHECK(legacy_pending IN (0,1));
UPDATE lazy_recipients SET legacy_pending=1 WHERE state='pending' AND EXISTS(SELECT 1 FROM send_manifests sm WHERE sm.preparation_id=lazy_recipients.preparation_id);
CREATE INDEX lazy_recipients_legacy ON lazy_recipients(seat_id, ordinal) WHERE legacy_pending=1;
CREATE INDEX lazy_recipients_seat_ordinal ON lazy_recipients(seat_id, ordinal);

-- Catch-up rows are keyed by binding. Each existing row moves to the seat's open migrated binding,
-- else to its newest binding row; a seat with no binding row at all loses its rows.
CREATE TABLE catch_up_v34 (
    binding_id TEXT NOT NULL REFERENCES occupant_bindings(binding_id),
    seat_id TEXT NOT NULL REFERENCES seats(id),
    thread_id TEXT NOT NULL REFERENCES threads(id),
    frontier_seq INTEGER NOT NULL CHECK(frontier_seq >= 0),
    entered_at INTEGER NOT NULL,
    extension_until INTEGER,
    last_progress_at INTEGER,
    state TEXT NOT NULL CHECK(state IN ('active', 'ended')),
    end_reason TEXT CHECK(end_reason IN ('ready', 'stalled', 'superseded')),
    ended_at INTEGER,
    release_seq INTEGER CHECK(release_seq IS NULL OR release_seq > 0),
    PRIMARY KEY(binding_id, thread_id),
    CHECK((state = 'active') = (end_reason IS NULL)),
    CHECK((end_reason IS NULL) = (ended_at IS NULL))
) STRICT;
INSERT INTO catch_up_v34(binding_id, seat_id, thread_id, frontier_seq, entered_at, extension_until, last_progress_at, state, end_reason, ended_at, release_seq)
SELECT (SELECT b.binding_id FROM occupant_bindings b WHERE b.seat_id=c.seat_id ORDER BY (b.migrated=1 AND b.ended_at IS NULL) DESC, b.ordinal DESC LIMIT 1),
       c.seat_id, c.thread_id, c.frontier_seq, c.entered_at, c.extension_until, c.last_progress_at, c.state, c.end_reason, c.ended_at, c.release_seq
FROM catch_up c WHERE EXISTS(SELECT 1 FROM occupant_bindings b WHERE b.seat_id=c.seat_id);
DROP TABLE catch_up;
ALTER TABLE catch_up_v34 RENAME TO catch_up;
CREATE INDEX catch_up_extension_until ON catch_up(extension_until) WHERE extension_until IS NOT NULL;
CREATE INDEX catch_up_thread_active ON catch_up(thread_id, seat_id) WHERE state = 'active';
CREATE INDEX catch_up_seat ON catch_up(seat_id, thread_id, state);

-- The rebuild drops table-owned archival triggers; retain the exact v23 guards.
CREATE TRIGGER archival_fence_catch_up_insert AFTER INSERT ON catch_up BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;
CREATE TRIGGER archival_fence_catch_up_update AFTER UPDATE ON catch_up BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;
CREATE TRIGGER archival_fence_catch_up_delete AFTER DELETE ON catch_up BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;
