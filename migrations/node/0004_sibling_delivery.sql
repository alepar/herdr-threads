-- Sibling delivery omits the exact sender from publication and legacy views.

DROP TRIGGER binding_pending_publish;
CREATE TRIGGER binding_pending_publish AFTER UPDATE OF decision_seq ON lazy_recipients
WHEN OLD.decision_seq IS NULL AND NEW.decision_seq IS NOT NULL
BEGIN
    INSERT OR IGNORE INTO binding_pending(binding_id,ordinal,thread_id,priority,decision_seq)
    SELECT b.binding_id,NEW.ordinal,NEW.thread_id,NEW.priority,NEW.decision_seq FROM occupant_bindings b
    WHERE b.seat_id=NEW.seat_id AND b.binding_id IS NOT (SELECT m.sender_binding_id FROM messages m WHERE m.id=NEW.message_id) AND (b.ended_at IS NULL AND b.liveness='connected')
      AND (EXISTS(SELECT 1 FROM binding_cursors lc WHERE lc.binding_id=b.binding_id AND lc.thread_id=NEW.thread_id) OR EXISTS(SELECT 1 FROM memberships lm WHERE lm.thread_id=NEW.thread_id AND lm.seat_id=NEW.seat_id AND (lm.state='joined' OR (lm.state='invited' AND COALESCE(lm.voluntary_state,lm.state)='invited' AND EXISTS(SELECT 1 FROM direct_threads ld JOIN seats ls ON ls.id=NEW.seat_id JOIN invitations li ON li.thread_id=ld.thread_id AND li.seat_id=ls.id AND li.episode=lm.episode WHERE ld.thread_id=NEW.thread_id AND ld.instance_id=(SELECT si.instance_id FROM seats si WHERE si.id=NEW.seat_id) AND (ls.id=ld.seat_a OR ls.id=ld.seat_b) AND ls.instance_id=(SELECT si.instance_id FROM seats si WHERE si.id=NEW.seat_id) AND ls.state<>'retired' AND ls.retired_at IS NULL AND li.state='pending' AND NEW.decision_seq>li.created_decision_seq)))));
END;

DROP TRIGGER binding_pending_legacy_set;
CREATE TRIGGER binding_pending_legacy_set AFTER UPDATE OF legacy_pending ON lazy_recipients
WHEN OLD.legacy_pending=0 AND NEW.legacy_pending=1 AND NEW.decision_seq IS NOT NULL
BEGIN
    INSERT OR IGNORE INTO binding_pending(binding_id,ordinal,thread_id,priority,decision_seq)
    SELECT b.binding_id,NEW.ordinal,NEW.thread_id,NEW.priority,NEW.decision_seq FROM occupant_bindings b
    WHERE b.seat_id=NEW.seat_id AND b.binding_id IS NOT (SELECT m.sender_binding_id FROM messages m WHERE m.id=NEW.message_id) AND (b.ended_at IS NULL AND b.liveness='connected') AND b.role='top_level';
END;
