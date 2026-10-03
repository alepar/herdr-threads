-- ht-xoc.4: schema v12, harness version evidence.
--
-- One row per (harness, version, contract_id) the hooks have reported on,
-- with the first lifecycle and tool payloads that matched the contract and a
-- sticky first violation. `harness_unattributed` keeps the latest reason a
-- payload could not be attributed to a version (no evidence row is written).
CREATE TABLE harness_version_evidence(
  harness TEXT NOT NULL CHECK(harness IN ('claude','codex')),
  version TEXT NOT NULL,
  contract_id TEXT NOT NULL,
  first_seen_at INTEGER NOT NULL,
  lifecycle_ok_at INTEGER,
  tool_ok_at INTEGER,
  violation_at INTEGER,
  violation_event TEXT,
  violation_field TEXT,
  last_seen_at INTEGER NOT NULL,
  PRIMARY KEY(harness, version, contract_id)
) WITHOUT ROWID;
CREATE INDEX harness_version_evidence_seen ON harness_version_evidence(harness, last_seen_at);
CREATE TABLE harness_unattributed(
  harness TEXT PRIMARY KEY CHECK(harness IN ('claude','codex')),
  reason TEXT NOT NULL,
  at INTEGER NOT NULL
) WITHOUT ROWID;
