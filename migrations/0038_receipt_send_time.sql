-- Run with foreign_keys OFF outside the containing transaction.
-- Preserve original table names, receipt ordinals/rowids and all published projections.
-- Main32 ACK binding attribution is copied verbatim; no historical binding is inferred.

CREATE TEMP TABLE receipt_send_time_saved_sequence AS SELECT seq FROM sqlite_sequence WHERE name='receipts';

CREATE TEMP TABLE receipt_send_time_saved_receipts AS SELECT * FROM receipts;

CREATE TEMP TABLE receipt_send_time_saved_state AS SELECT rowid AS preserved_rowid,* FROM receipt_state;

DROP TABLE receipts;

CREATE TABLE receipts (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    message_id TEXT NOT NULL,
    thread_id TEXT NOT NULL REFERENCES threads(id),
    seat_id TEXT NOT NULL REFERENCES seats(id),
    state TEXT NOT NULL CHECK(state IN ('pending', 'acked', 'recipient_retired')),
    frozen_duration_ms INTEGER NOT NULL CHECK(frozen_duration_ms > 0),
    available_at INTEGER,
    deadline_at INTEGER,
    ack_actor_seat_id TEXT REFERENCES seats(id),
    ack_generation INTEGER,
    ack_observation TEXT,
    acked_at INTEGER,
    warning_message_id TEXT REFERENCES messages(id),
    retired_at INTEGER, soft_poked_at INTEGER, ack_required INTEGER NOT NULL DEFAULT 1 CHECK(ack_required IN (0,1)),
    ack_binding_id TEXT,
    UNIQUE(message_id, seat_id),
    FOREIGN KEY(message_id, thread_id) REFERENCES messages(id, thread_id)
) STRICT;

INSERT INTO receipts(ordinal,message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at,ack_actor_seat_id,ack_generation,ack_observation,acked_at,warning_message_id,retired_at,soft_poked_at,ack_required,ack_binding_id) SELECT ordinal,message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at,ack_actor_seat_id,ack_generation,ack_observation,acked_at,warning_message_id,retired_at,soft_poked_at,ack_required,ack_binding_id FROM receipt_send_time_saved_receipts;

CREATE INDEX receipts_due ON receipts(deadline_at, ordinal) WHERE state = 'pending' AND deadline_at IS NOT NULL;

CREATE INDEX receipts_message_ordinal ON receipts(message_id, ordinal);

CREATE INDEX receipts_pending_unwarned ON receipts(deadline_at, ordinal) WHERE state='pending' AND deadline_at IS NOT NULL AND warning_message_id IS NULL;

CREATE INDEX receipts_required_due ON receipts(deadline_at,ordinal) WHERE state='pending' AND ack_required=1 AND deadline_at IS NOT NULL AND warning_message_id IS NULL;

CREATE INDEX receipts_required_poke ON receipts(deadline_at,message_id,seat_id) WHERE state='pending' AND ack_required=1 AND deadline_at IS NOT NULL AND available_at IS NOT NULL AND warning_message_id IS NULL AND soft_poked_at IS NULL;

CREATE INDEX receipts_required_seat_pending ON receipts(seat_id,ordinal) WHERE state='pending' AND ack_required=1;

CREATE INDEX receipts_required_thread_pending ON receipts(seat_id,thread_id,ordinal) WHERE state='pending' AND ack_required=1;

CREATE INDEX receipts_seat_state_ordinal ON receipts(seat_id, state, ordinal);

CREATE INDEX receipts_seat_thread_state_ordinal ON receipts(seat_id, thread_id, state, ordinal);

CREATE INDEX receipts_thread_pending_ordinal ON receipts(thread_id, ordinal) WHERE state='pending';

CREATE INDEX receipts_thread_seat_pending ON receipts(seat_id, thread_id, ordinal) WHERE state = 'pending';

CREATE TRIGGER archival_fence_receipts_delete AFTER DELETE ON receipts BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_receipts_insert AFTER INSERT ON receipts BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_receipts_update AFTER UPDATE ON receipts BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER digest_physical_receipt_settled AFTER UPDATE OF state ON receipts WHEN NEW.state!='pending' AND NOT EXISTS (SELECT 1 FROM send_manifests sm JOIN prepared_recipients pr ON pr.preparation_id=sm.preparation_id WHERE sm.message_id=NEW.message_id AND pr.seat_id=NEW.seat_id)
BEGIN
    DELETE FROM digest_open_warnings WHERE condition_kind='receipt' AND condition_id=length(CAST(NEW.message_id AS BLOB))||':'||NEW.message_id||':'||NEW.seat_id;
END;

CREATE TRIGGER human_receipt_physical_waived AFTER UPDATE OF ack_required ON receipts WHEN NEW.ack_required=0
BEGIN
    DELETE FROM digest_open_warnings WHERE condition_kind='receipt' AND condition_id=length(CAST(NEW.message_id AS BLOB))||':'||NEW.message_id||':'||NEW.seat_id;
END;

CREATE TRIGGER receipts_ordinal_immutable BEFORE UPDATE OF ordinal ON receipts
BEGIN SELECT RAISE(ABORT, 'receipt ordinal is immutable'); END;

CREATE TRIGGER wake_batches_clear_receipt
AFTER UPDATE OF state,ack_required ON receipts
WHEN OLD.state='pending' AND OLD.ack_required=1 AND (NEW.state!='pending' OR NEW.ack_required=0) AND NOT EXISTS(SELECT 1 FROM digest_pending_invitations WHERE seat_id=OLD.seat_id)
      AND NOT EXISTS(SELECT 1 FROM digest_pending_manifest_receipts WHERE seat_id=OLD.seat_id)
      AND NOT EXISTS(SELECT 1 FROM receipts WHERE seat_id=OLD.seat_id AND state='pending' AND ack_required=1)
BEGIN DELETE FROM wake_batches WHERE seat_id=OLD.seat_id; END;

DROP TABLE receipt_state;

CREATE TABLE receipt_state (
    message_id TEXT NOT NULL,
    seat_id TEXT NOT NULL REFERENCES seats(id),
    state TEXT NOT NULL CHECK(state IN ('pending','acked','recipient_retired')),
    available_at INTEGER,
    deadline_at INTEGER,
    ack_actor_seat_id TEXT REFERENCES seats(id),
    ack_generation INTEGER,
    ack_observation TEXT,
    acked_at INTEGER,
    retired_at INTEGER,
    warning_message_id TEXT, soft_poked_at INTEGER, ack_required INTEGER NOT NULL DEFAULT 1 CHECK(ack_required IN (0,1)),
    ack_binding_id TEXT,
    PRIMARY KEY(message_id, seat_id)
) STRICT;

INSERT INTO receipt_state(rowid,message_id,seat_id,state,available_at,deadline_at,ack_actor_seat_id,ack_generation,ack_observation,acked_at,retired_at,warning_message_id,soft_poked_at,ack_required,ack_binding_id) SELECT preserved_rowid,message_id,seat_id,state,available_at,deadline_at,ack_actor_seat_id,ack_generation,ack_observation,acked_at,retired_at,warning_message_id,soft_poked_at,ack_required,ack_binding_id FROM receipt_send_time_saved_state;

CREATE INDEX receipt_state_pending_due ON receipt_state(deadline_at, message_id, seat_id) WHERE state='pending' AND deadline_at IS NOT NULL AND warning_message_id IS NULL;

CREATE INDEX receipt_state_required_due ON receipt_state(deadline_at,message_id,seat_id) WHERE state='pending' AND ack_required=1 AND deadline_at IS NOT NULL AND warning_message_id IS NULL;

CREATE INDEX receipt_state_required_poke ON receipt_state(deadline_at,message_id,seat_id) WHERE state='pending' AND ack_required=1 AND deadline_at IS NOT NULL AND available_at IS NOT NULL AND warning_message_id IS NULL AND soft_poked_at IS NULL;

CREATE TRIGGER archival_fence_receipt_state_delete AFTER DELETE ON receipt_state BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_receipt_state_insert AFTER INSERT ON receipt_state BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_receipt_state_update AFTER UPDATE ON receipt_state BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER digest_manifest_receipt_materialized AFTER INSERT ON receipt_state WHEN NEW.state='pending'
BEGIN
    UPDATE digest_pending_manifest_receipts SET decision_seq=(SELECT decision_seq FROM send_manifests WHERE message_id=NEW.message_id) WHERE seat_id=NEW.seat_id AND preparation_id=(SELECT preparation_id FROM send_manifests WHERE message_id=NEW.message_id);
END;

CREATE TRIGGER digest_receipt_state_settled_insert AFTER INSERT ON receipt_state WHEN NEW.state!='pending'
BEGIN
    DELETE FROM digest_pending_manifest_receipts WHERE seat_id=NEW.seat_id AND preparation_id=(SELECT preparation_id FROM send_manifests WHERE message_id=NEW.message_id);
    DELETE FROM digest_open_warnings WHERE condition_kind='receipt' AND condition_id=length(CAST(NEW.message_id AS BLOB))||':'||NEW.message_id||':'||NEW.seat_id;
END;

CREATE TRIGGER digest_receipt_state_settled_update AFTER UPDATE OF state ON receipt_state WHEN NEW.state!='pending'
BEGIN
    DELETE FROM digest_pending_manifest_receipts WHERE seat_id=NEW.seat_id AND preparation_id=(SELECT preparation_id FROM send_manifests WHERE message_id=NEW.message_id);
    DELETE FROM digest_open_warnings WHERE condition_kind='receipt' AND condition_id=length(CAST(NEW.message_id AS BLOB))||':'||NEW.message_id||':'||NEW.seat_id;
END;

CREATE TRIGGER human_receipt_state_insert_waived AFTER INSERT ON receipt_state WHEN NEW.ack_required=0
BEGIN
    DELETE FROM digest_pending_manifest_receipts WHERE seat_id=NEW.seat_id AND preparation_id=(SELECT preparation_id FROM send_manifests WHERE message_id=NEW.message_id);
    DELETE FROM digest_open_warnings WHERE condition_kind='receipt' AND condition_id=length(CAST(NEW.message_id AS BLOB))||':'||NEW.message_id||':'||NEW.seat_id;
END;

CREATE TRIGGER human_receipt_state_waived AFTER UPDATE OF ack_required ON receipt_state WHEN NEW.ack_required=0
BEGIN
    DELETE FROM digest_pending_manifest_receipts WHERE seat_id=NEW.seat_id AND preparation_id=(SELECT preparation_id FROM send_manifests WHERE message_id=NEW.message_id);
    DELETE FROM digest_open_warnings WHERE condition_kind='receipt' AND condition_id=length(CAST(NEW.message_id AS BLOB))||':'||NEW.message_id||':'||NEW.seat_id;
END;

UPDATE sqlite_sequence SET seq=max(seq,COALESCE((SELECT seq FROM receipt_send_time_saved_sequence),0)) WHERE name='receipts';

DROP TABLE receipt_send_time_saved_sequence;

DROP TABLE receipt_send_time_saved_receipts;

DROP TABLE receipt_send_time_saved_state;

CREATE TABLE receipt_send_time_cutovers (instance_id TEXT PRIMARY KEY, historical_decision_seq INTEGER NOT NULL CHECK(historical_decision_seq>=0)) STRICT;

INSERT INTO receipt_send_time_cutovers(instance_id,historical_decision_seq) SELECT id,decision_seq FROM host_instances;
