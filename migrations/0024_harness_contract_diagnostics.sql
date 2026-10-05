-- Advisory unavailable-runtime failures: no version/build or authority key.
CREATE TABLE harness_contract_diagnostics (
    harness TEXT NOT NULL CHECK (harness IN ('claude', 'codex')),
    session_id TEXT NOT NULL CHECK (length(CAST(session_id AS BLOB)) BETWEEN 1 AND 256),
    contract_id TEXT NOT NULL CHECK (length(contract_id) = 16 AND contract_id NOT GLOB '*[^0-9a-f]*'),
    event TEXT NOT NULL CHECK (length(event) BETWEEN 1 AND 63 AND event NOT GLOB '*[^a-zA-Z0-9]*'),
    field TEXT NOT NULL CHECK (length(CAST(field AS BLOB)) BETWEEN 1 AND 128),
    first_seen_at INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL,
    PRIMARY KEY (harness, session_id, contract_id)
) WITHOUT ROWID;
CREATE INDEX harness_contract_diagnostics_recent ON harness_contract_diagnostics(harness, last_seen_at DESC, first_seen_at DESC, session_id, contract_id);
