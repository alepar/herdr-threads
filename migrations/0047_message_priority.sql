-- Message delivery priority (ht-yha, spec 2026-10-13-message-priority-delivery). One transaction.
-- delivery_mode, wake_on and confirmation stay, frozen for history and old-reader audit.
DROP TRIGGER messages_immutable;

ALTER TABLE messages ADD COLUMN priority INTEGER NOT NULL DEFAULT 0 CHECK(priority BETWEEN 0 AND 9);

ALTER TABLE send_preparations ADD COLUMN priority INTEGER NOT NULL DEFAULT 0 CHECK(priority BETWEEN 0 AND 9);

UPDATE messages SET priority=1 WHERE delivery_mode='lazy';

UPDATE send_preparations SET priority=1 WHERE delivery_mode='lazy';

CREATE TRIGGER messages_immutable BEFORE UPDATE ON messages
BEGIN SELECT RAISE(ABORT, 'message history is immutable'); END;

CREATE TRIGGER message_priority_immutable BEFORE UPDATE OF priority ON messages
WHEN NEW.priority IS NOT OLD.priority
BEGIN SELECT RAISE(ABORT, 'message priority is immutable'); END;

CREATE TRIGGER preparation_priority_immutable BEFORE UPDATE OF priority ON send_preparations
WHEN NEW.priority IS NOT OLD.priority
BEGIN SELECT RAISE(ABORT, 'preparation priority is immutable'); END;

ALTER TABLE lazy_recipients ADD COLUMN priority INTEGER NOT NULL DEFAULT 1 CHECK(priority BETWEEN 0 AND 9);

ALTER TABLE lazy_recipients ADD COLUMN decision_seq INTEGER CHECK(decision_seq IS NULL OR decision_seq>0);

UPDATE lazy_recipients SET decision_seq=(SELECT sm.decision_seq FROM send_manifests sm WHERE sm.preparation_id=lazy_recipients.preparation_id AND sm.message_id=lazy_recipients.message_id) WHERE EXISTS(SELECT 1 FROM send_manifests sm WHERE sm.preparation_id=lazy_recipients.preparation_id AND sm.message_id=lazy_recipients.message_id);

CREATE TRIGGER lazy_recipient_priority_immutable BEFORE UPDATE OF priority ON lazy_recipients
WHEN NEW.priority IS NOT OLD.priority
BEGIN SELECT RAISE(ABORT, 'delivery row priority is immutable'); END;

CREATE TRIGGER lazy_recipient_decision_seq_once BEFORE UPDATE OF decision_seq ON lazy_recipients
WHEN OLD.decision_seq IS NOT NULL AND NEW.decision_seq IS NOT OLD.decision_seq
BEGIN SELECT RAISE(ABORT, 'delivery row publication is immutable'); END;

DROP TRIGGER lazy_recipient_source;

DROP TRIGGER lazy_recipient_display_published;

DROP TRIGGER lazy_manifest_source;

CREATE TRIGGER lazy_recipient_source BEFORE INSERT ON lazy_recipients
BEGIN
    SELECT RAISE(ABORT, 'lazy recipient source mismatch') WHERE NEW.state<>'pending' OR NEW.decision_seq IS NOT NULL OR NOT EXISTS (
        SELECT 1 FROM send_preparations p JOIN seats s ON s.id=NEW.seat_id AND s.instance_id=p.instance_id
        WHERE p.id=NEW.preparation_id AND p.thread_id=NEW.thread_id AND p.status='building'
    ) OR EXISTS(SELECT 1 FROM send_manifests WHERE preparation_id=NEW.preparation_id);
END;

CREATE TRIGGER lazy_recipient_display_published BEFORE UPDATE OF state ON lazy_recipients
WHEN NEW.state='displayed' AND NOT EXISTS (
    SELECT 1 FROM send_manifests sm JOIN messages m ON m.id=sm.message_id
    WHERE sm.preparation_id=OLD.preparation_id AND sm.message_id=OLD.message_id AND sm.thread_id=OLD.thread_id
)
BEGIN SELECT RAISE(ABORT, 'unpublished lazy delivery cannot complete'); END;

CREATE TRIGGER lazy_manifest_source BEFORE INSERT ON send_manifests
BEGIN
    SELECT RAISE(ABORT, 'lazy manifest source mismatch') WHERE
        (SELECT delivery_mode FROM send_preparations WHERE id=NEW.preparation_id) IS NOT (SELECT delivery_mode FROM messages WHERE id=NEW.message_id)
        OR (SELECT priority FROM send_preparations WHERE id=NEW.preparation_id) IS NOT (SELECT priority FROM messages WHERE id=NEW.message_id)
        OR (((SELECT delivery_mode FROM send_preparations WHERE id=NEW.preparation_id)='lazy' OR EXISTS(SELECT 1 FROM lazy_recipients WHERE preparation_id=NEW.preparation_id)) AND NEW.message_id IS NOT CASE WHEN substr(NEW.preparation_id,1,5)='prep-' AND length(NEW.preparation_id)>5 THEN 'msg-'||substr(NEW.preparation_id,6) WHEN length(NEW.preparation_id)=9 AND substr(NEW.preparation_id,1,1)='p' AND substr(NEW.preparation_id,2) NOT GLOB '*[^A-Za-z0-9]*' THEN 'm'||substr(NEW.preparation_id,2) ELSE NULL END)
        OR (NEW.warning_count<>0 AND (SELECT priority FROM messages WHERE id=NEW.message_id)>=1 AND NOT EXISTS(SELECT 1 FROM prepared_recipients WHERE preparation_id=NEW.preparation_id));
END;

ALTER TABLE seats ADD COLUMN nudge_threshold INTEGER NOT NULL DEFAULT 0 CHECK(nudge_threshold BETWEEN -1 AND 9);

UPDATE seats SET nudge_threshold=-1 WHERE wake_on='all';

CREATE TABLE binding_row_displays (
    binding_id TEXT NOT NULL REFERENCES occupant_bindings(binding_id),
    message_id TEXT NOT NULL CHECK(length(CAST(message_id AS BLOB)) BETWEEN 1 AND 128),
    PRIMARY KEY(binding_id, message_id)
) STRICT;

-- ht-yha.44 per-binding pending views (also the repair tail of a pre-amendment v47 store).
DROP INDEX IF EXISTS lazy_recipients_pending_order;

DROP INDEX IF EXISTS lazy_recipients_pending_thread_order;

CREATE TABLE binding_pending (
    binding_id TEXT NOT NULL REFERENCES occupant_bindings(binding_id),
    ordinal INTEGER NOT NULL REFERENCES lazy_recipients(ordinal),
    thread_id TEXT NOT NULL,
    priority INTEGER NOT NULL CHECK(priority BETWEEN 0 AND 9),
    decision_seq INTEGER NOT NULL CHECK(decision_seq>0),
    PRIMARY KEY(binding_id, ordinal)
) STRICT;

CREATE INDEX binding_pending_order ON binding_pending(binding_id,priority,decision_seq,ordinal);

CREATE INDEX binding_pending_thread_order ON binding_pending(binding_id,thread_id,priority,decision_seq,ordinal);

CREATE INDEX lazy_recipients_legacy_order ON lazy_recipients(seat_id,priority,decision_seq,ordinal) WHERE legacy_pending=1 AND decision_seq IS NOT NULL;

CREATE TRIGGER binding_pending_publish AFTER UPDATE OF decision_seq ON lazy_recipients
WHEN OLD.decision_seq IS NULL AND NEW.decision_seq IS NOT NULL
BEGIN
    INSERT OR IGNORE INTO binding_pending(binding_id,ordinal,thread_id,priority,decision_seq)
    SELECT b.binding_id,NEW.ordinal,NEW.thread_id,NEW.priority,NEW.decision_seq FROM occupant_bindings b
    WHERE b.seat_id=NEW.seat_id AND (b.ended_at IS NULL AND b.liveness='connected')
      AND (EXISTS(SELECT 1 FROM binding_cursors c WHERE c.binding_id=b.binding_id AND c.thread_id=NEW.thread_id)
        OR EXISTS(SELECT 1 FROM memberships mb WHERE mb.thread_id=NEW.thread_id AND mb.seat_id=NEW.seat_id AND mb.state IN ('joined','invited')));
END;

CREATE TRIGGER binding_pending_legacy_set AFTER UPDATE OF legacy_pending ON lazy_recipients
WHEN OLD.legacy_pending=0 AND NEW.legacy_pending=1 AND NEW.decision_seq IS NOT NULL
BEGIN
    INSERT OR IGNORE INTO binding_pending(binding_id,ordinal,thread_id,priority,decision_seq)
    SELECT b.binding_id,NEW.ordinal,NEW.thread_id,NEW.priority,NEW.decision_seq FROM occupant_bindings b
    WHERE b.seat_id=NEW.seat_id AND (b.ended_at IS NULL AND b.liveness='connected') AND b.role='top_level';
END;

CREATE TRIGGER binding_pending_legacy_clear AFTER UPDATE OF legacy_pending ON lazy_recipients
WHEN OLD.legacy_pending=1 AND NEW.legacy_pending=0
BEGIN
    DELETE FROM binding_pending WHERE ordinal=NEW.ordinal
      AND (binding_id IN (SELECT c.binding_id FROM binding_cursors c WHERE c.thread_id=NEW.thread_id AND c.seq>=(SELECT m.sequence FROM messages m WHERE m.id=NEW.message_id))
        OR binding_id IN (SELECT d.binding_id FROM binding_row_displays d WHERE d.message_id=NEW.message_id));
END;

CREATE TRIGGER binding_pending_cursor_insert AFTER INSERT ON binding_cursors
BEGIN
    DELETE FROM binding_pending WHERE binding_id=NEW.binding_id AND thread_id=NEW.thread_id
      AND (SELECT m.sequence FROM lazy_recipients r JOIN messages m ON m.id=r.message_id WHERE r.ordinal=binding_pending.ordinal)<=NEW.seq
      AND NOT EXISTS(SELECT 1 FROM lazy_recipients r JOIN occupant_bindings ob ON ob.binding_id=NEW.binding_id WHERE r.ordinal=binding_pending.ordinal AND r.legacy_pending=1 AND ob.role='top_level');
END;

CREATE TRIGGER binding_pending_cursor_advance AFTER UPDATE OF seq ON binding_cursors
WHEN NEW.seq>OLD.seq
BEGIN
    DELETE FROM binding_pending WHERE binding_id=NEW.binding_id AND thread_id=NEW.thread_id
      AND (SELECT m.sequence FROM lazy_recipients r JOIN messages m ON m.id=r.message_id WHERE r.ordinal=binding_pending.ordinal)<=NEW.seq
      AND NOT EXISTS(SELECT 1 FROM lazy_recipients r JOIN occupant_bindings ob ON ob.binding_id=NEW.binding_id WHERE r.ordinal=binding_pending.ordinal AND r.legacy_pending=1 AND ob.role='top_level');
END;

CREATE TRIGGER binding_pending_row_display AFTER INSERT ON binding_row_displays
BEGIN
    DELETE FROM binding_pending WHERE binding_id=NEW.binding_id AND ordinal IN (
        SELECT r.ordinal FROM lazy_recipients r JOIN occupant_bindings ob ON ob.binding_id=NEW.binding_id AND ob.seat_id=r.seat_id
        WHERE r.message_id=NEW.message_id AND NOT (r.legacy_pending=1 AND ob.role='top_level'));
END;

CREATE TRIGGER binding_pending_binding_end AFTER UPDATE OF ended_at ON occupant_bindings
WHEN OLD.ended_at IS NULL AND NEW.ended_at IS NOT NULL
BEGIN
    DELETE FROM binding_pending WHERE binding_id=NEW.binding_id;
END;
