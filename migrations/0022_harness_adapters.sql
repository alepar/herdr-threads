-- Open lexical storage does not grant cooperative authority: registry recognition
-- and exact canonical binding checks remain Rust decision requirements.
-- No legacy evidence backfill or reinterpretation. Executed in one transaction.
CREATE TABLE harness_binding_sequence_v22(seq INTEGER NOT NULL);
INSERT INTO harness_binding_sequence_v22 SELECT COALESCE((SELECT seq FROM sqlite_sequence WHERE name='occupant_bindings'),0);
CREATE TABLE occupant_bindings_v22 (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    seat_id TEXT NOT NULL REFERENCES seats(id),
    generation INTEGER NOT NULL CHECK(generation >= 0),
    target_generation INTEGER NOT NULL DEFAULT 0 CHECK(target_generation >= 0),
    target_id TEXT NOT NULL,
    terminal_id TEXT,
    incarnation TEXT,
    host_boot TEXT NOT NULL,
    host_epoch INTEGER NOT NULL CHECK(host_epoch >= 0),
    harness TEXT NOT NULL CHECK(length(CAST(harness AS BLOB)) BETWEEN 1 AND 64 AND instr(harness,char(0))=0 AND substr(harness,1,1) GLOB '[a-z]' AND harness NOT GLOB '*[^a-z0-9_-]*'),
    native_session TEXT NOT NULL,
    execution_id TEXT NOT NULL,
    observation_provenance TEXT NOT NULL,
    observed_at INTEGER NOT NULL,
    registered_at INTEGER,
    ended_at INTEGER,
    UNIQUE(seat_id, generation)
) STRICT;
INSERT INTO occupant_bindings_v22 (ordinal, seat_id, generation, target_generation, target_id, terminal_id, incarnation, host_boot, host_epoch, harness, native_session, execution_id, observation_provenance, observed_at, registered_at, ended_at)
SELECT ordinal, seat_id, generation, target_generation, target_id, terminal_id, incarnation, host_boot, host_epoch, harness, native_session, execution_id, observation_provenance, observed_at, registered_at, ended_at FROM occupant_bindings;
UPDATE sqlite_sequence SET seq=MAX(seq,(SELECT seq FROM harness_binding_sequence_v22)) WHERE name='occupant_bindings_v22';
INSERT INTO sqlite_sequence(name,seq) SELECT 'occupant_bindings_v22',seq FROM harness_binding_sequence_v22 WHERE NOT EXISTS(SELECT 1 FROM sqlite_sequence WHERE name='occupant_bindings_v22');
DROP TABLE harness_binding_sequence_v22;
DROP TABLE occupant_bindings;
ALTER TABLE occupant_bindings_v22 RENAME TO occupant_bindings;
CREATE UNIQUE INDEX occupant_bindings_current ON occupant_bindings(seat_id) WHERE ended_at IS NULL;
CREATE INDEX occupant_bindings_history ON occupant_bindings(seat_id, ordinal);
CREATE INDEX occupant_bindings_execution ON occupant_bindings(seat_id, execution_id);

CREATE TABLE harness_version_evidence_v22(
  harness TEXT NOT NULL CHECK(length(CAST(harness AS BLOB)) BETWEEN 1 AND 64 AND instr(harness,char(0))=0 AND substr(harness,1,1) GLOB '[a-z]' AND harness NOT GLOB '*[^a-z0-9_-]*' AND harness<>'human'),
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
INSERT INTO harness_version_evidence_v22 SELECT * FROM harness_version_evidence;
DROP TABLE harness_version_evidence;
ALTER TABLE harness_version_evidence_v22 RENAME TO harness_version_evidence;
CREATE INDEX harness_version_evidence_seen ON harness_version_evidence(harness, last_seen_at);
CREATE TABLE harness_unattributed_v22(
  harness TEXT PRIMARY KEY CHECK(length(CAST(harness AS BLOB)) BETWEEN 1 AND 64 AND instr(harness,char(0))=0 AND substr(harness,1,1) GLOB '[a-z]' AND harness NOT GLOB '*[^a-z0-9_-]*' AND harness<>'human'),
  reason TEXT NOT NULL,
  at INTEGER NOT NULL
) WITHOUT ROWID;
INSERT INTO harness_unattributed_v22 SELECT * FROM harness_unattributed;
DROP TABLE harness_unattributed;
ALTER TABLE harness_unattributed_v22 RENAME TO harness_unattributed;

CREATE TABLE harness_runtime_identities (
    harness TEXT NOT NULL CHECK(length(CAST(harness AS BLOB)) BETWEEN 1 AND 64 AND instr(harness,char(0))=0 AND substr(harness,1,1) GLOB '[a-z]' AND harness NOT GLOB '*[^a-z0-9_-]*' AND harness<>'human'),
    identity_key TEXT NOT NULL CHECK(instr(identity_key,char(0))=0 AND length(CAST(identity_key AS BLOB)) BETWEEN 1 AND 136 AND (identity_key GLOB 'release:[0-9]*.[0-9]*.[0-9]*' OR (substr(identity_key,1,6)='build:' AND length(identity_key)=70 AND substr(identity_key,7) NOT GLOB '*[^0-9a-f]*'))),
    descriptor_json TEXT NOT NULL CHECK(length(CAST(descriptor_json AS BLOB)) BETWEEN 1 AND 4096 AND json_valid(descriptor_json) AND json_type(descriptor_json)='object'),
    last_seen_at INTEGER NOT NULL,
    PRIMARY KEY(harness, identity_key)
) STRICT, WITHOUT ROWID;
CREATE TABLE harness_contract_evidence_v2 (
    harness TEXT NOT NULL,
    identity_key TEXT NOT NULL,
    domain TEXT NOT NULL CHECK(instr(domain,char(0))=0 AND length(domain) BETWEEN 1 AND 32 AND substr(domain,1,1) GLOB '[a-z]' AND domain NOT GLOB '*[^a-z0-9_]*'),
    origin TEXT NOT NULL CHECK(origin IN ('native_payload','native_shape_observation','bridge_envelope')),
    contract_id TEXT NOT NULL CHECK(instr(contract_id,char(0))=0 AND length(contract_id)=16 AND contract_id NOT GLOB '*[^0-9a-f]*'),
    first_seen_at INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL,
    milestones_json TEXT NOT NULL CHECK(length(CAST(milestones_json AS BLOB)) BETWEEN 1 AND 4096 AND json_valid(milestones_json) AND json_type(milestones_json)='object'),
    violation_at INTEGER,
    violation_event TEXT CHECK(violation_event IS NULL OR (instr(violation_event,char(0))=0 AND length(CAST(violation_event AS BLOB)) BETWEEN 1 AND 63 AND substr(violation_event,1,1) GLOB '[A-Za-z]' AND violation_event NOT GLOB '*[^A-Za-z0-9_]*')),
    violation_field TEXT CHECK(violation_field IS NULL OR length(CAST(violation_field AS BLOB)) BETWEEN 1 AND 256),
    CHECK((violation_at IS NULL AND violation_event IS NULL AND violation_field IS NULL) OR (violation_at IS NOT NULL AND violation_event IS NOT NULL AND violation_field IS NOT NULL)),
    PRIMARY KEY(harness, identity_key, domain, origin, contract_id),
    FOREIGN KEY(harness, identity_key) REFERENCES harness_runtime_identities(harness, identity_key)
) STRICT, WITHOUT ROWID;
CREATE INDEX harness_contract_evidence_v2_seen ON harness_contract_evidence_v2(harness,last_seen_at);
CREATE TABLE harness_unattributed_v2 (
    harness TEXT NOT NULL CHECK(length(CAST(harness AS BLOB)) BETWEEN 1 AND 64 AND instr(harness,char(0))=0 AND substr(harness,1,1) GLOB '[a-z]' AND harness NOT GLOB '*[^a-z0-9_-]*' AND harness<>'human'),
    domain TEXT NOT NULL CHECK(instr(domain,char(0))=0 AND length(domain) BETWEEN 1 AND 32 AND substr(domain,1,1) GLOB '[a-z]' AND domain NOT GLOB '*[^a-z0-9_]*'),
    origin TEXT NOT NULL CHECK(origin IN ('native_payload','native_shape_observation','bridge_envelope')),
    reason TEXT NOT NULL CHECK(length(CAST(reason AS BLOB)) BETWEEN 1 AND 256),
    at INTEGER NOT NULL,
    PRIMARY KEY(harness, domain, origin)
) STRICT, WITHOUT ROWID;
