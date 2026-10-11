-- Canonical derived addresses of all native authors associated with an open
-- shared receipt condition. No membership, current binding or offered notice
-- can establish an origin. Historical rows are bootstrapped in bounded turns.
CREATE TABLE receipt_warning_origins (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    instance_id TEXT NOT NULL REFERENCES host_instances(id),
    seat_id TEXT NOT NULL REFERENCES seats(id),
    warning_id TEXT NOT NULL REFERENCES messages(id),
    source_message_id TEXT NOT NULL REFERENCES messages(id),
    UNIQUE(warning_id,seat_id)
) STRICT;
CREATE INDEX receipt_warning_origins_seat ON receipt_warning_origins(instance_id,seat_id,ordinal);
CREATE INDEX receipt_warning_origins_exact ON receipt_warning_origins(instance_id,seat_id,warning_id);
CREATE INDEX receipt_warning_origins_warning ON receipt_warning_origins(warning_id,ordinal);

-- receipt_state has a reusable implicit rowid. This maintained candidate
-- projection gives every newly eligible episode a monotonic identity instead.
CREATE TABLE receipt_due_sparse (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    message_id TEXT NOT NULL,
    seat_id TEXT NOT NULL REFERENCES seats(id),
    deadline_at INTEGER NOT NULL,
    UNIQUE(message_id,seat_id)
) STRICT;
CREATE INDEX receipts_required_due_ordinal ON receipts(ordinal)
 WHERE state='pending' AND ack_required=1 AND deadline_at IS NOT NULL AND warning_message_id IS NULL;

CREATE TABLE receipt_discovery_backfill (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    physical_after INTEGER NOT NULL CHECK(physical_after>=0),
    physical_high_water INTEGER NOT NULL CHECK(physical_high_water>=physical_after),
    sparse_after INTEGER NOT NULL CHECK(sparse_after>=0),
    sparse_high_water INTEGER NOT NULL CHECK(sparse_high_water>=sparse_after),
    completed INTEGER NOT NULL CHECK(completed IN (0,1)),
    CHECK(completed=0 OR (physical_after=physical_high_water AND sparse_after=sparse_high_water))
) STRICT;
-- Both extrema use their integer rowid/primary-key tree. No history is visited.
INSERT INTO receipt_discovery_backfill
 VALUES(1,0,COALESCE((SELECT MAX(ordinal) FROM receipts),0),
          0,COALESCE((SELECT MAX(rowid) FROM receipt_state),0),0);

CREATE TRIGGER receipt_warning_origin_attention AFTER INSERT ON receipt_warning_origins
 BEGIN
 INSERT INTO warning_recipients(warning_id,seat_id,generation)
 SELECT NEW.warning_id,s.id,m.decision_seq FROM seats s JOIN messages m ON m.id=NEW.warning_id
 WHERE s.id=NEW.seat_id AND s.instance_id=NEW.instance_id AND s.state!='retired' ON CONFLICT(warning_id,seat_id) DO NOTHING;
 INSERT INTO wake_work(seat_id,reason_bits,attention_version)
 SELECT id,4,1 FROM seats WHERE id=NEW.seat_id AND instance_id=NEW.instance_id AND state!='retired'
 ON CONFLICT(seat_id) DO UPDATE SET reason_bits=reason_bits|4,attention_version=attention_version+1;
 END;
CREATE TRIGGER receipt_warning_origin_receipts_insert AFTER INSERT ON receipts
 WHEN NEW.warning_message_id IS NOT NULL
 BEGIN
 INSERT INTO receipt_warning_origins(instance_id,seat_id,warning_id,source_message_id)
 SELECT m.instance_id,m.actor_seat_id,c.open_warning_id,m.id FROM receipts r
 JOIN messages m ON m.id=r.message_id AND m.author_kind='native'
 JOIN seats s ON s.id=m.actor_seat_id AND s.instance_id=m.instance_id
 JOIN warning_conditions c ON c.open_warning_id=r.warning_message_id
    AND c.condition_kind='receipt' AND c.clear_warning_id IS NULL
    AND c.thread_id=m.thread_id AND c.affected_seat_id=r.seat_id
 WHERE r.message_id=NEW.message_id AND r.seat_id=NEW.seat_id ON CONFLICT(warning_id,seat_id) DO NOTHING;
 END;
CREATE TRIGGER receipt_warning_origin_receipts_update AFTER UPDATE OF warning_message_id ON receipts
 WHEN NEW.warning_message_id IS NOT NULL
 BEGIN
 INSERT INTO receipt_warning_origins(instance_id,seat_id,warning_id,source_message_id)
 SELECT m.instance_id,m.actor_seat_id,c.open_warning_id,m.id FROM receipts r
 JOIN messages m ON m.id=r.message_id AND m.author_kind='native'
 JOIN seats s ON s.id=m.actor_seat_id AND s.instance_id=m.instance_id
 JOIN warning_conditions c ON c.open_warning_id=r.warning_message_id
    AND c.condition_kind='receipt' AND c.clear_warning_id IS NULL
    AND c.thread_id=m.thread_id AND c.affected_seat_id=r.seat_id
 WHERE r.message_id=NEW.message_id AND r.seat_id=NEW.seat_id ON CONFLICT(warning_id,seat_id) DO NOTHING;
 END;
CREATE TRIGGER receipt_warning_origin_receipt_state_insert AFTER INSERT ON receipt_state
 WHEN NEW.warning_message_id IS NOT NULL
 BEGIN
 INSERT INTO receipt_warning_origins(instance_id,seat_id,warning_id,source_message_id)
 SELECT m.instance_id,m.actor_seat_id,c.open_warning_id,m.id FROM receipt_state r
 JOIN messages m ON m.id=r.message_id AND m.author_kind='native'
 JOIN send_manifests sm ON sm.message_id=r.message_id AND sm.instance_id=m.instance_id AND sm.thread_id=m.thread_id
 JOIN prepared_recipients pr ON pr.preparation_id=sm.preparation_id AND pr.seat_id=r.seat_id AND pr.thread_id=m.thread_id
 JOIN seats s ON s.id=m.actor_seat_id AND s.instance_id=m.instance_id
 JOIN warning_conditions c ON c.open_warning_id=r.warning_message_id
    AND c.condition_kind='receipt' AND c.clear_warning_id IS NULL
    AND c.thread_id=m.thread_id AND c.affected_seat_id=r.seat_id
 WHERE r.message_id=NEW.message_id AND r.seat_id=NEW.seat_id ON CONFLICT(warning_id,seat_id) DO NOTHING;
 END;
CREATE TRIGGER receipt_warning_origin_receipt_state_update AFTER UPDATE OF warning_message_id ON receipt_state
 WHEN NEW.warning_message_id IS NOT NULL
 BEGIN
 INSERT INTO receipt_warning_origins(instance_id,seat_id,warning_id,source_message_id)
 SELECT m.instance_id,m.actor_seat_id,c.open_warning_id,m.id FROM receipt_state r
 JOIN messages m ON m.id=r.message_id AND m.author_kind='native'
 JOIN send_manifests sm ON sm.message_id=r.message_id AND sm.instance_id=m.instance_id AND sm.thread_id=m.thread_id
 JOIN prepared_recipients pr ON pr.preparation_id=sm.preparation_id AND pr.seat_id=r.seat_id AND pr.thread_id=m.thread_id
 JOIN seats s ON s.id=m.actor_seat_id AND s.instance_id=m.instance_id
 JOIN warning_conditions c ON c.open_warning_id=r.warning_message_id
    AND c.condition_kind='receipt' AND c.clear_warning_id IS NULL
    AND c.thread_id=m.thread_id AND c.affected_seat_id=r.seat_id
 WHERE r.message_id=NEW.message_id AND r.seat_id=NEW.seat_id ON CONFLICT(warning_id,seat_id) DO NOTHING;
 END;
-- Only the exact first source is needed at condition creation. Later
-- coalesced sources are inserted by their exact receipt marker triggers.
CREATE TRIGGER receipt_warning_origin_condition AFTER INSERT ON warning_conditions
 WHEN NEW.condition_kind='receipt' AND NEW.clear_warning_id IS NULL
 BEGIN
INSERT INTO receipt_warning_origins(instance_id,seat_id,warning_id,source_message_id)
SELECT m.instance_id,m.actor_seat_id,c.open_warning_id,m.id FROM receipts r
 JOIN messages m ON m.id=r.message_id AND m.author_kind='native'
 JOIN seats s ON s.id=m.actor_seat_id AND s.instance_id=m.instance_id
 JOIN warning_conditions c ON c.open_warning_id=r.warning_message_id
    AND c.condition_kind='receipt' AND c.clear_warning_id IS NULL
    AND c.thread_id=m.thread_id AND c.affected_seat_id=r.seat_id
 WHERE r.message_id=(SELECT source_message_id FROM messages WHERE id=NEW.open_warning_id) AND r.seat_id=NEW.affected_seat_id ON CONFLICT(warning_id,seat_id) DO NOTHING;
INSERT INTO receipt_warning_origins(instance_id,seat_id,warning_id,source_message_id)
SELECT m.instance_id,m.actor_seat_id,c.open_warning_id,m.id FROM receipt_state r
 JOIN messages m ON m.id=r.message_id AND m.author_kind='native'
 JOIN send_manifests sm ON sm.message_id=r.message_id AND sm.instance_id=m.instance_id AND sm.thread_id=m.thread_id
 JOIN prepared_recipients pr ON pr.preparation_id=sm.preparation_id AND pr.seat_id=r.seat_id AND pr.thread_id=m.thread_id
 JOIN seats s ON s.id=m.actor_seat_id AND s.instance_id=m.instance_id
 JOIN warning_conditions c ON c.open_warning_id=r.warning_message_id
    AND c.condition_kind='receipt' AND c.clear_warning_id IS NULL
    AND c.thread_id=m.thread_id AND c.affected_seat_id=r.seat_id
 WHERE r.message_id=(SELECT source_message_id FROM messages WHERE id=NEW.open_warning_id) AND r.seat_id=NEW.affected_seat_id ON CONFLICT(warning_id,seat_id) DO NOTHING;
END;
CREATE TRIGGER receipt_warning_origin_clear AFTER UPDATE OF clear_warning_id ON warning_conditions
 WHEN NEW.condition_kind='receipt' AND NEW.clear_warning_id IS NOT NULL
 BEGIN
 INSERT INTO warning_recipients(warning_id,seat_id,generation)
 SELECT NEW.clear_warning_id,o.seat_id,NEW.cleared_seq FROM receipt_warning_origins o
 JOIN seats s ON s.id=o.seat_id AND s.state!='retired' WHERE o.warning_id=NEW.open_warning_id ON CONFLICT(warning_id,seat_id) DO NOTHING;
 DELETE FROM receipt_warning_origins WHERE warning_id=NEW.open_warning_id;
 END;
CREATE TRIGGER receipt_due_sparse_insert AFTER INSERT ON receipt_state
 BEGIN
 DELETE FROM receipt_due_sparse WHERE message_id=NEW.message_id AND seat_id=NEW.seat_id
  AND NOT (NEW.state='pending' AND NEW.ack_required=1 AND NEW.deadline_at IS NOT NULL AND NEW.warning_message_id IS NULL);
 INSERT INTO receipt_due_sparse(message_id,seat_id,deadline_at)
 SELECT NEW.message_id,NEW.seat_id,NEW.deadline_at
 WHERE NEW.state='pending' AND NEW.ack_required=1 AND NEW.deadline_at IS NOT NULL AND NEW.warning_message_id IS NULL
 ON CONFLICT(message_id,seat_id) DO UPDATE SET deadline_at=excluded.deadline_at;
 END;
CREATE TRIGGER receipt_due_sparse_update AFTER UPDATE OF state,ack_required,deadline_at,warning_message_id ON receipt_state
 BEGIN
 DELETE FROM receipt_due_sparse WHERE message_id=NEW.message_id AND seat_id=NEW.seat_id
  AND NOT (NEW.state='pending' AND NEW.ack_required=1 AND NEW.deadline_at IS NOT NULL AND NEW.warning_message_id IS NULL);
 INSERT INTO receipt_due_sparse(message_id,seat_id,deadline_at)
 SELECT NEW.message_id,NEW.seat_id,NEW.deadline_at
 WHERE NEW.state='pending' AND NEW.ack_required=1 AND NEW.deadline_at IS NOT NULL AND NEW.warning_message_id IS NULL
 ON CONFLICT(message_id,seat_id) DO UPDATE SET deadline_at=excluded.deadline_at;
 END;
CREATE TRIGGER receipt_due_sparse_delete AFTER DELETE ON receipt_state
 BEGIN DELETE FROM receipt_due_sparse WHERE message_id=OLD.message_id AND seat_id=OLD.seat_id; END;
