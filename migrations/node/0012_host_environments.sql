-- Host environments (named seats, tmux wake, herdr-core terminals). One delta
-- carries every column the environments run needs; only `herdr_hosts.kind` and
-- `kind_probed_at` are written at first (Herdr attach). The rest stay unwritten
-- until the named-seat and herdr-core phases.

-- `kind`: which environment a host row is. No CHECK: validated in code against
-- the implemented-kinds set, so a later kind needs no table rebuild. A row whose
-- kind this build does not implement is attached by nobody and named by doctor.
-- `kind_probed_at`: NULL means the kind came from the default; reattach stamps it.
ALTER TABLE herdr_hosts ADD COLUMN kind TEXT NOT NULL DEFAULT 'herdr';
ALTER TABLE herdr_hosts ADD COLUMN kind_probed_at INTEGER;
CREATE UNIQUE INDEX herdr_hosts_one_named ON herdr_hosts(kind) WHERE kind = 'named';

-- `name`: the durable name of a named seat. The CHECK is a superset of the
-- grammar (1-64 bytes, [A-Za-z0-9._-]); the leading `-`/`.` and seat-id-shaped
-- rules are validated in code. The unique index also covers `unresolved` rows,
-- which `seats_live_target` does not, so an unresolved named seat is not split
-- by the next resolve of its name.
ALTER TABLE seats ADD COLUMN name TEXT
    CHECK(name IS NULL OR (length(CAST(name AS BLOB)) BETWEEN 1 AND 64
        AND name NOT GLOB '*[^A-Za-z0-9._-]*'));
CREATE UNIQUE INDEX seats_live_name ON seats(host_id, name)
    WHERE name IS NOT NULL AND is_local = 1 AND state <> 'retired';

-- Remote rows materialized before `name` existed keep the name their `seat.created` carried.
-- Display only, never authority. The predicates mirror the column CHECK plus the
-- leading-character rule; the seat-id-shaped rule cannot be stated in SQL, and such a value is
-- at worst an odd label. An invalid value stays NULL: the CHECK would abort the migration, so
-- the SET reads the same valid row the WHERE found.
UPDATE seats SET name = (
    SELECT json_extract(CAST(j.payload AS TEXT), '$.display_name') FROM journal j
    WHERE j.kind = 'seat.created' AND j.creates = seats.id
      AND json_valid(CAST(j.payload AS TEXT))
      AND typeof(json_extract(CAST(j.payload AS TEXT), '$.display_name')) = 'text'
      AND length(CAST(json_extract(CAST(j.payload AS TEXT), '$.display_name') AS BLOB)) BETWEEN 1 AND 64
      AND json_extract(CAST(j.payload AS TEXT), '$.display_name') NOT GLOB '*[^A-Za-z0-9._-]*'
      AND json_extract(CAST(j.payload AS TEXT), '$.display_name') NOT GLOB '[-.]*'
    ORDER BY j.ingest_seq LIMIT 1)
WHERE is_local = 0 AND name IS NULL;

-- `locator_hint`: pane id or `<core>:<tN>` of a named binding; a hint, never authority.
-- `readonly`: the binding's display never ACKs receipt-bearing rows.
ALTER TABLE occupant_bindings ADD COLUMN locator_hint TEXT;
ALTER TABLE occupant_bindings ADD COLUMN readonly INTEGER NOT NULL DEFAULT 0;

-- Persisted no-wake reports: one row per (observer, seat) per no-wake episode.
CREATE TABLE no_wake_reports (
    observer TEXT NOT NULL,
    seat_id TEXT NOT NULL,
    PRIMARY KEY (observer, seat_id)
) STRICT;

-- Peer launch onto a herdr-core: the gate-only record of a `peer_requests` row
-- (TRUST-POLICY N15). `gate_attempt` set makes the row one; `gate_created` is the
-- recorded `CreatedTarget`, written once the seat exists.
ALTER TABLE peer_requests ADD COLUMN gate_attempt INTEGER
    CHECK(gate_attempt IS NULL OR gate_attempt >= 1);
ALTER TABLE peer_requests ADD COLUMN gate_created TEXT
    CHECK(gate_created IS NULL OR length(CAST(gate_created AS BLOB)) <= 4096);
