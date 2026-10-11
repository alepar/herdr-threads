-- Session-keyed seat bindings (ht-54r, spec §1 and §4 Migration V29). Executed in one
-- transaction by schema::migrate_v28_to_v29, which then stamps the upgrade time into the
-- disconnected_at of each open migrated binding.
CREATE TABLE harness_binding_sequence_v29(seq INTEGER NOT NULL);
INSERT INTO harness_binding_sequence_v29 SELECT COALESCE((SELECT seq FROM sqlite_sequence WHERE name='occupant_bindings'),0);
CREATE TABLE occupant_bindings_v29 (
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
    binding_id TEXT NOT NULL UNIQUE DEFAULT (lower(hex(randomblob(16)))) CHECK(length(CAST(binding_id AS BLOB)) BETWEEN 1 AND 128 AND binding_id NOT GLOB '*[^!-~]*'),
    role TEXT NOT NULL DEFAULT 'top_level' CHECK(role IN ('top_level','subagent')),
    parent_binding TEXT,
    agent_id TEXT NOT NULL DEFAULT '' CHECK(length(CAST(agent_id AS BLOB)) <= 128 AND agent_id NOT GLOB '*[^!-~]*'),
    lineage_from TEXT,
    liveness TEXT NOT NULL DEFAULT 'connected' CHECK(liveness IN ('connected','disconnected')),
    disconnected_at INTEGER,
    disconnect_reason TEXT CHECK(disconnect_reason IS NULL OR (length(disconnect_reason) BETWEEN 1 AND 64 AND disconnect_reason NOT GLOB '*[^a-z_]*')),
    liveness_pid INTEGER CHECK(liveness_pid IS NULL OR liveness_pid > 0),
    liveness_start INTEGER,
    migrated INTEGER NOT NULL DEFAULT 0 CHECK(migrated IN (0,1)),
    migrated_session TEXT,
    CHECK(role='subagent' OR parent_binding IS NULL),
    CHECK((liveness='connected') = (disconnected_at IS NULL AND disconnect_reason IS NULL)),
    CHECK((liveness_pid IS NULL) = (liveness_start IS NULL)),
    CHECK(migrated=1 OR migrated_session IS NULL)
) STRICT;
INSERT INTO occupant_bindings_v29 (ordinal, seat_id, generation, target_generation, target_id, terminal_id, incarnation, host_boot, host_epoch, harness, native_session, execution_id, observation_provenance, observed_at, registered_at, ended_at, binding_id, role, agent_id, liveness, disconnected_at, disconnect_reason, migrated, migrated_session)
SELECT ordinal, seat_id, generation, target_generation, target_id, terminal_id, incarnation, host_boot, host_epoch, harness, native_session, execution_id, observation_provenance, observed_at, registered_at, ended_at,
       lower(hex(randomblob(16))), 'top_level', '',
       CASE WHEN ended_at IS NULL AND harness='human' THEN 'connected' ELSE 'disconnected' END,
       CASE WHEN ended_at IS NULL AND harness='human' THEN NULL ELSE COALESCE(ended_at, observed_at) END,
       CASE WHEN ended_at IS NULL AND harness='human' THEN NULL ELSE 'migrated' END,
       CASE WHEN ended_at IS NULL AND harness<>'human' THEN 1 ELSE 0 END,
       CASE WHEN ended_at IS NULL AND harness<>'human' AND native_session<>''
                 AND substr(native_session,1,15)<>'plugin_context:' AND substr(native_session,1,7)<>'launch:'
            THEN native_session END
FROM occupant_bindings;
UPDATE sqlite_sequence SET seq=MAX(seq,(SELECT seq FROM harness_binding_sequence_v29)) WHERE name='occupant_bindings_v29';
INSERT INTO sqlite_sequence(name,seq) SELECT 'occupant_bindings_v29',seq FROM harness_binding_sequence_v29 WHERE NOT EXISTS(SELECT 1 FROM sqlite_sequence WHERE name='occupant_bindings_v29');
DROP TABLE harness_binding_sequence_v29;
DROP TABLE occupant_bindings;
ALTER TABLE occupant_bindings_v29 RENAME TO occupant_bindings;
CREATE UNIQUE INDEX occupant_bindings_session_key ON occupant_bindings(seat_id, harness, native_session, agent_id) WHERE ended_at IS NULL;
CREATE INDEX occupant_bindings_open ON occupant_bindings(seat_id) WHERE ended_at IS NULL;
CREATE INDEX occupant_bindings_history ON occupant_bindings(seat_id, ordinal);
CREATE INDEX occupant_bindings_execution ON occupant_bindings(seat_id, execution_id);

-- The rebuild drops table-owned archival triggers; retain the exact v23 guards.
CREATE TRIGGER archival_fence_occupant_bindings_insert AFTER INSERT ON occupant_bindings BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;
CREATE TRIGGER archival_fence_occupant_bindings_update AFTER UPDATE ON occupant_bindings BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;
CREATE TRIGGER archival_fence_occupant_bindings_delete AFTER DELETE ON occupant_bindings BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;
CREATE TRIGGER archival_binding_activity AFTER UPDATE OF registered_at,ended_at,generation,harness,native_session,execution_id,target_id,host_boot,host_epoch,observation_provenance ON occupant_bindings BEGIN
 UPDATE seat_archival SET activity_revision=activity_revision+1,idle_mono=NULL,samples=0,next_mono=0 WHERE seat_id=NEW.seat_id;
END;
CREATE TRIGGER archival_binding_queue_insert AFTER INSERT ON occupant_bindings BEGIN
 UPDATE seat_archival SET next_mono=0 WHERE seat_id=NEW.seat_id;
END;
CREATE TRIGGER archival_binding_queue_delete AFTER DELETE ON occupant_bindings BEGIN
 UPDATE seat_archival SET next_mono=0 WHERE seat_id=OLD.seat_id;
END;

CREATE TABLE binding_cursors (
    binding_id TEXT NOT NULL REFERENCES occupant_bindings(binding_id),
    thread_id TEXT NOT NULL REFERENCES threads(id),
    seq INTEGER NOT NULL CHECK(seq >= 0),
    PRIMARY KEY(binding_id, thread_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX binding_cursors_thread ON binding_cursors(thread_id, binding_id);
INSERT INTO binding_cursors(binding_id, thread_id, seq)
SELECT b.binding_id, m.thread_id, COALESCE((SELECT MAX(x.sequence) FROM messages x WHERE x.thread_id=m.thread_id), 0)
FROM occupant_bindings b JOIN memberships m ON m.seat_id=b.seat_id AND m.state='joined'
WHERE b.ended_at IS NULL;
ALTER TABLE receipts ADD COLUMN ack_binding_id TEXT;
ALTER TABLE receipt_state ADD COLUMN ack_binding_id TEXT;
ALTER TABLE messages ADD COLUMN sender_binding_id TEXT;
