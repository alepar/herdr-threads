CREATE TABLE wake_batches (
    seat_id TEXT PRIMARY KEY REFERENCES seats(id),
    deadline_at INTEGER NOT NULL
) STRICT;

CREATE TRIGGER wake_batches_clear_invitation
AFTER DELETE ON digest_pending_invitations
WHEN NOT EXISTS(SELECT 1 FROM digest_pending_invitations WHERE seat_id=OLD.seat_id)
      AND NOT EXISTS(SELECT 1 FROM digest_pending_manifest_receipts WHERE seat_id=OLD.seat_id)
      AND NOT EXISTS(SELECT 1 FROM receipts WHERE seat_id=OLD.seat_id AND state='pending' AND ack_required=1)
BEGIN DELETE FROM wake_batches WHERE seat_id=OLD.seat_id; END;

CREATE TRIGGER wake_batches_clear_manifest
AFTER DELETE ON digest_pending_manifest_receipts
WHEN NOT EXISTS(SELECT 1 FROM digest_pending_invitations WHERE seat_id=OLD.seat_id)
      AND NOT EXISTS(SELECT 1 FROM digest_pending_manifest_receipts WHERE seat_id=OLD.seat_id)
      AND NOT EXISTS(SELECT 1 FROM receipts WHERE seat_id=OLD.seat_id AND state='pending' AND ack_required=1)
BEGIN DELETE FROM wake_batches WHERE seat_id=OLD.seat_id; END;

CREATE TRIGGER wake_batches_clear_receipt
AFTER UPDATE OF state,ack_required ON receipts
WHEN OLD.state='pending' AND OLD.ack_required=1 AND (NEW.state!='pending' OR NEW.ack_required=0) AND NOT EXISTS(SELECT 1 FROM digest_pending_invitations WHERE seat_id=OLD.seat_id)
      AND NOT EXISTS(SELECT 1 FROM digest_pending_manifest_receipts WHERE seat_id=OLD.seat_id)
      AND NOT EXISTS(SELECT 1 FROM receipts WHERE seat_id=OLD.seat_id AND state='pending' AND ack_required=1)
BEGIN DELETE FROM wake_batches WHERE seat_id=OLD.seat_id; END;

CREATE TRIGGER wake_batches_clear_retired
AFTER UPDATE OF state ON seats
WHEN NEW.state='retired'
BEGIN DELETE FROM wake_batches WHERE seat_id=NEW.id; END;
