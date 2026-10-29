-- Peer hygiene (2.8.0). Both tables are local and never replicated.
--
-- `sync_link_events`: the history `peer events` lists. One row per link
-- event; a repeat of the newest row's (kind, node, dest, detail) bumps `n` and
-- `last_at` instead of adding a row, so a backoff loop stays one row. Pruned
-- on insert to the newest 1000 rows and 30 days.
CREATE TABLE sync_link_events (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    at INTEGER NOT NULL,
    last_at INTEGER NOT NULL CHECK(last_at >= at),
    n INTEGER NOT NULL DEFAULT 1 CHECK(n >= 1),
    node_uuid TEXT CHECK(node_uuid IS NULL OR length(node_uuid) = 32),
    dest TEXT CHECK(dest IS NULL OR length(CAST(dest AS BLOB)) <= 1024),
    kind TEXT NOT NULL CHECK(kind IN ('up', 'down', 'dial_failed', 'not_installed', 'stopped', 'unforgotten')),
    origin TEXT NOT NULL CHECK(origin IN ('dialed', 'inbound')),
    detail TEXT CHECK(detail IS NULL OR length(CAST(detail AS BLOB)) <= 160)
) STRICT;
CREATE INDEX sync_link_events_node ON sync_link_events(node_uuid, seq);

-- `sync_forgotten_nodes`: dead node islands an operator pruned (`peer prune`,
-- TRUST-POLICY N16). A tombstone only: nothing of the node is deleted. The
-- coordinator neither wants nor advertises a forgotten node's logs, and the
-- views leave it out.
CREATE TABLE sync_forgotten_nodes (
    node_uuid TEXT PRIMARY KEY CHECK(length(node_uuid) = 32),
    tag TEXT NOT NULL CHECK(length(tag) = 3),
    host_label TEXT CHECK(host_label IS NULL OR length(CAST(host_label AS BLOB)) <= 255),
    forgotten_at INTEGER NOT NULL,
    provenance TEXT NOT NULL CHECK(length(provenance) BETWEEN 1 AND 128)
) STRICT;
