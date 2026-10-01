-- A person's own pane identity (`herdr-threads me init`) is an occupant with
-- harness 'human' and 'operator_human' provenance. SQLite cannot alter a
-- CHECK, so the table is rebuilt with identical columns, rows, ordinals and
-- indexes; only the accepted harness set grows. No trigger, view or foreign
-- key refers to occupant_bindings.
CREATE TABLE occupant_bindings_v9 (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    seat_id TEXT NOT NULL REFERENCES seats(id),
    generation INTEGER NOT NULL CHECK(generation >= 0),
    target_generation INTEGER NOT NULL DEFAULT 0 CHECK(target_generation >= 0),
    target_id TEXT NOT NULL,
    terminal_id TEXT,
    incarnation TEXT,
    host_boot TEXT NOT NULL,
    host_epoch INTEGER NOT NULL CHECK(host_epoch >= 0),
    harness TEXT NOT NULL CHECK(harness IN ('codex', 'claude', 'human')),
    native_session TEXT NOT NULL,
    execution_id TEXT NOT NULL,
    observation_provenance TEXT NOT NULL,
    observed_at INTEGER NOT NULL,
    registered_at INTEGER,
    ended_at INTEGER,
    UNIQUE(seat_id, generation)
) STRICT;
INSERT INTO occupant_bindings_v9 (ordinal, seat_id, generation, target_generation, target_id, terminal_id, incarnation, host_boot, host_epoch, harness, native_session, execution_id, observation_provenance, observed_at, registered_at, ended_at)
SELECT ordinal, seat_id, generation, target_generation, target_id, terminal_id, incarnation, host_boot, host_epoch, harness, native_session, execution_id, observation_provenance, observed_at, registered_at, ended_at FROM occupant_bindings;
DROP TABLE occupant_bindings;
ALTER TABLE occupant_bindings_v9 RENAME TO occupant_bindings;
CREATE UNIQUE INDEX occupant_bindings_current ON occupant_bindings(seat_id) WHERE ended_at IS NULL;
CREATE INDEX occupant_bindings_history ON occupant_bindings(seat_id, ordinal);
CREATE INDEX occupant_bindings_execution ON occupant_bindings(seat_id, execution_id);
