-- Finite pending-confirmation snapshots advance by immutable ordinal, never
-- by deadline, so new earlier deadlines cannot stretch an existing traversal.
CREATE INDEX invitations_confirmation_scan ON invitations(ordinal)
    WHERE state='pending' AND warning_message_id IS NULL AND reject_recorded=0
    AND (confirmation IS NULL OR confirmation='require_ack');

-- Timed wake windows exclude lazy rows before LIMIT; lazy backlog cannot hide
-- the timed invitation that needs guarded native/mod delivery.
CREATE INDEX invitations_confirmation_pending_seat ON invitations(seat_id,created_decision_seq,ordinal)
    WHERE state='pending' AND (confirmation IS NULL OR confirmation='require_ack');

CREATE INDEX invitations_confirmation_pending_thread ON invitations(seat_id,thread_id,created_decision_seq,ordinal)
    WHERE state='pending' AND (confirmation IS NULL OR confirmation='require_ack');
