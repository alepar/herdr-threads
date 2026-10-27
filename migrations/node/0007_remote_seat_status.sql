-- Seat status (ht-xg88): each remote node's latest advisory snapshot of its
-- own seats, as relayed on `seat_status` frames. Display data only: nothing
-- routes, wakes or attributes by it. One origin row per node; its seats are
-- replaced as a whole by a newer snapshot.
CREATE TABLE remote_seat_origin (
    origin TEXT PRIMARY KEY CHECK(length(origin) = 32),
    boot TEXT NOT NULL CHECK(length(boot) = 36),
    rev INTEGER NOT NULL CHECK(rev >= 0),
    built_at INTEGER NOT NULL CHECK(built_at >= 0),
    version TEXT CHECK(version IS NULL OR length(version) BETWEEN 1 AND 32),
    other_boot_at INTEGER CHECK(other_boot_at IS NULL OR other_boot_at >= 0),
    updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE remote_seat_status (
    seat_id TEXT PRIMARY KEY REFERENCES seats(id),
    origin TEXT NOT NULL REFERENCES remote_seat_origin(origin),
    presence TEXT NOT NULL CHECK(length(presence) BETWEEN 1 AND 16),
    harness TEXT CHECK(harness IS NULL OR length(harness) BETWEEN 1 AND 16),
    activity_at INTEGER CHECK(activity_at IS NULL OR activity_at >= 0),
    workspace TEXT CHECK(workspace IS NULL OR length(CAST(workspace AS BLOB)) <= 64),
    tab TEXT CHECK(tab IS NULL OR length(CAST(tab AS BLOB)) <= 64),
    pane TEXT CHECK(pane IS NULL OR length(CAST(pane AS BLOB)) <= 64)
) STRICT;
CREATE INDEX remote_seat_status_origin ON remote_seat_status(origin);
