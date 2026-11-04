-- Atomic remote handoff (ht-8lem, TRUST-POLICY N4/N5/N15).
-- The sender node's ledger of signed handoff requests: one row per request id, recorded before the
-- first send with the signed LaunchRequest frozen in `request_bytes`; every send replays those bytes.
-- Node-local, never replicated. Overdue is columns, never a state.
CREATE TABLE handoff_obligations (
    request TEXT PRIMARY KEY CHECK(length(request) = 36),
    target_node TEXT NOT NULL CHECK(length(target_node) = 32),
    author TEXT NOT NULL REFERENCES seats(id),
    message TEXT NOT NULL UNIQUE,
    requested_at INTEGER NOT NULL,
    deadline_at INTEGER,
    request_bytes BLOB NOT NULL,
    seat TEXT,
    state TEXT NOT NULL CHECK(state IN ('sent','launched','linked','expired','failed')),
    answer TEXT,
    overdue_seq INTEGER,
    thread_warned_at INTEGER,
    alert_shown_at INTEGER,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;
-- The seat-addressed overdue alert (Q4): pending while overdue, not yet warned in the pair thread and
-- unshown, unless the row is linked (the receipt's own overdue takes over).
CREATE INDEX handoff_obligations_alert ON handoff_obligations(author, overdue_seq)
    WHERE overdue_seq IS NOT NULL AND thread_warned_at IS NULL AND alert_shown_at IS NULL AND state!='linked';
-- Target side: the staged pair thread and message (set together by staging) and the prompt frozen in decide.
ALTER TABLE peer_requests ADD COLUMN handoff_thread TEXT
    CHECK(handoff_thread IS NULL OR length(CAST(handoff_thread AS BLOB)) <= 1024);
ALTER TABLE peer_requests ADD COLUMN handoff_message TEXT
    CHECK(handoff_message IS NULL OR length(CAST(handoff_message AS BLOB)) <= 1024);
ALTER TABLE peer_requests ADD COLUMN handoff_prompt TEXT
    CHECK(handoff_prompt IS NULL OR length(CAST(handoff_prompt AS BLOB)) <= 1024);
-- N5: the node that owns a handoff message's deadline (the block's sender_node); NULL otherwise.
ALTER TABLE messages ADD COLUMN deadline_node TEXT
    CHECK(deadline_node IS NULL OR length(deadline_node) = 32);
-- N5: the receipt due index follows the deadline owner, COALESCE(deadline_node, origin_node). The Rust macro
-- schema::deadline_owner_sql! is the source of truth, pinned by deadline_owner_triggers_use_the_fragment.
DROP TRIGGER receipt_due_sparse_insert;
DROP TRIGGER receipt_due_sparse_update;

CREATE TRIGGER receipt_due_sparse_insert AFTER INSERT ON receipt_state
 BEGIN
 DELETE FROM receipt_due_sparse WHERE message_id=NEW.message_id AND seat_id=NEW.seat_id
  AND NOT (NEW.state='pending' AND NEW.ack_required=1 AND NEW.deadline_at IS NOT NULL AND NEW.warning_message_id IS NULL);
 INSERT INTO receipt_due_sparse(message_id,seat_id,deadline_at)
 SELECT NEW.message_id,NEW.seat_id,NEW.deadline_at
 WHERE NEW.state='pending' AND NEW.ack_required=1 AND NEW.deadline_at IS NOT NULL AND NEW.warning_message_id IS NULL
  AND COALESCE((SELECT COALESCE(deadline_node,origin_node) FROM messages WHERE id=NEW.message_id),(SELECT id FROM local_node))=(SELECT id FROM local_node)
 ON CONFLICT(message_id,seat_id) DO UPDATE SET deadline_at=excluded.deadline_at;
 END;

CREATE TRIGGER receipt_due_sparse_update AFTER UPDATE OF state,ack_required,deadline_at,warning_message_id ON receipt_state
 BEGIN
 DELETE FROM receipt_due_sparse WHERE message_id=NEW.message_id AND seat_id=NEW.seat_id
  AND NOT (NEW.state='pending' AND NEW.ack_required=1 AND NEW.deadline_at IS NOT NULL AND NEW.warning_message_id IS NULL);
 INSERT INTO receipt_due_sparse(message_id,seat_id,deadline_at)
 SELECT NEW.message_id,NEW.seat_id,NEW.deadline_at
 WHERE NEW.state='pending' AND NEW.ack_required=1 AND NEW.deadline_at IS NOT NULL AND NEW.warning_message_id IS NULL
  AND COALESCE((SELECT COALESCE(deadline_node,origin_node) FROM messages WHERE id=NEW.message_id),(SELECT id FROM local_node))=(SELECT id FROM local_node)
 ON CONFLICT(message_id,seat_id) DO UPDATE SET deadline_at=excluded.deadline_at;
 END;

