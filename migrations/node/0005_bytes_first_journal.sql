-- Bytes-first journal (ht-76u6): every journal row stores the original canonical
-- bytes (raw) as truth; the parsed fields are a nullable projection (all present,
-- or all NULL for a layout this node does not understand). This file creates the
-- replacement tables; the Rust copy journal::copy_rows_with_raw fills them
-- (raw computed and hash-checked) and 0005_bytes_first_journal_after.sql swaps
-- them in.
CREATE TABLE journal_new (
    ingest_seq INTEGER PRIMARY KEY AUTOINCREMENT,
    log_ix INTEGER NOT NULL REFERENCES sync_logs(log_ix),
    counter INTEGER NOT NULL CHECK(counter >= 1),
    prev_hash BLOB NOT NULL CHECK(length(prev_hash) = 32),
    hash BLOB NOT NULL CHECK(length(hash) = 32),
    signature BLOB NOT NULL CHECK(length(signature) = 64),
    raw BLOB NOT NULL CHECK(length(raw) BETWEEN 68 AND 786432),
    hlc INTEGER CHECK(hlc IS NULL OR hlc >= 0),
    kind TEXT CHECK(kind IS NULL OR length(kind) BETWEEN 1 AND 64),
    kind_version INTEGER CHECK(kind_version IS NULL OR kind_version BETWEEN 1 AND 65535),
    deps BLOB,
    creates TEXT CHECK(creates IS NULL OR length(creates) BETWEEN 1 AND 160),
    needs TEXT DEFAULT '',
    payload BLOB CHECK(payload IS NULL OR length(payload) <= 524288),
    ingested_at INTEGER NOT NULL,
    apply_state TEXT NOT NULL CHECK(apply_state IN ('applied','pending','parked_unknown','parked_dep','failed','parked_layout')),
    applied_seq INTEGER CHECK((apply_state = 'applied') = (applied_seq IS NOT NULL)),
    parked_on TEXT CHECK((apply_state = 'parked_dep') = (parked_on IS NOT NULL)),
    apply_error TEXT CHECK(apply_error IS NULL OR length(CAST(apply_error AS BLOB)) <= 512),
    -- The projection is all-present or all-NULL (an unknown layout).
    CHECK((hlc IS NULL) = (kind IS NULL) AND (kind_version IS NULL) = (kind IS NULL)
      AND (deps IS NULL) = (kind IS NULL) AND (needs IS NULL) = (kind IS NULL)
      AND (payload IS NULL) = (kind IS NULL)),
    CHECK(kind IS NOT NULL OR creates IS NULL),
    -- parked_layout is exactly the empty-projection state (promotion moves both at once).
    CHECK((apply_state = 'parked_layout') = (kind IS NULL)),
    UNIQUE(log_ix, counter)
) STRICT;

CREATE TABLE journal_forked_new (
    forked_ix INTEGER PRIMARY KEY AUTOINCREMENT,
    log_ix INTEGER NOT NULL REFERENCES sync_logs(log_ix),
    counter INTEGER NOT NULL CHECK(counter >= 1),
    prev_hash BLOB NOT NULL CHECK(length(prev_hash) = 32),
    hash BLOB NOT NULL CHECK(length(hash) = 32),
    signature BLOB NOT NULL CHECK(length(signature) = 64),
    raw BLOB NOT NULL CHECK(length(raw) BETWEEN 68 AND 786432),
    hlc INTEGER CHECK(hlc IS NULL OR hlc >= 0),
    kind TEXT CHECK(kind IS NULL OR length(kind) BETWEEN 1 AND 64),
    kind_version INTEGER CHECK(kind_version IS NULL OR kind_version BETWEEN 1 AND 65535),
    deps BLOB,
    creates TEXT CHECK(creates IS NULL OR length(creates) BETWEEN 1 AND 160),
    needs TEXT DEFAULT '',
    payload BLOB CHECK(payload IS NULL OR length(payload) <= 524288),
    source_node TEXT CHECK(source_node IS NULL OR length(source_node) = 32),
    received_at INTEGER NOT NULL,
    CHECK((hlc IS NULL) = (kind IS NULL) AND (kind_version IS NULL) = (kind IS NULL)
      AND (deps IS NULL) = (kind IS NULL) AND (needs IS NULL) = (kind IS NULL)
      AND (payload IS NULL) = (kind IS NULL)),
    CHECK(kind IS NOT NULL OR creates IS NULL),
    UNIQUE(log_ix, counter, hash)
) STRICT;
