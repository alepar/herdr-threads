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

CREATE INDEX lazy_recipients_pending_order ON lazy_recipients(seat_id,priority,decision_seq,ordinal) WHERE state='pending' AND decision_seq IS NOT NULL;

CREATE INDEX lazy_recipients_pending_thread_order ON lazy_recipients(seat_id,thread_id,priority,decision_seq,ordinal) WHERE state='pending' AND decision_seq IS NOT NULL;

CREATE TABLE binding_row_displays (
    binding_id TEXT NOT NULL REFERENCES occupant_bindings(binding_id),
    message_id TEXT NOT NULL CHECK(length(CAST(message_id AS BLOB)) BETWEEN 1 AND 128),
    PRIMARY KEY(binding_id, message_id)
) STRICT;
