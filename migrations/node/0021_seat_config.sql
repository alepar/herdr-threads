-- Per-seat config (ht-z41e.2). Node-local, never replicated. A Table-backed key has a row only
-- while set explicitly (absence = registry default). A column-backed key (nudge_threshold) keeps
-- only an audit row: value NULL, its value lives in seats.nudge_threshold. The store never
-- validates a key in SQL: the registry (src/protocol/seat_config.rs) is the single source.
CREATE TABLE seat_config (
    seat_id TEXT NOT NULL REFERENCES seats(id),
    key TEXT NOT NULL,
    value TEXT,
    updated_at INTEGER NOT NULL,
    writer_provenance TEXT NOT NULL,
    writer_binding TEXT,
    PRIMARY KEY (seat_id, key)
) STRICT;
