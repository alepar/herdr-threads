-- NULL is the historical timed invitation. Never reconstruct an unknown inviter.
ALTER TABLE invitations ADD COLUMN confirmation TEXT CHECK(confirmation IN ('lazy','require_ack'));
ALTER TABLE invitations ADD COLUMN inviter_seat_id TEXT REFERENCES seats(id);

CREATE INDEX invitations_confirmation_due ON invitations(deadline_at,ordinal)
    WHERE state='pending' AND warning_message_id IS NULL AND reject_recorded=0
    AND (confirmation IS NULL OR confirmation='require_ack');

CREATE TRIGGER invitations_confirmation_origin BEFORE INSERT ON invitations
WHEN NEW.confirmation IS NOT NULL AND NEW.inviter_seat_id IS NULL
BEGIN SELECT RAISE(ABORT, 'modern invitation requires original inviter'); END;

CREATE TRIGGER invitations_confirmation_immutable BEFORE UPDATE OF confirmation,inviter_seat_id ON invitations
WHEN NEW.confirmation IS NOT OLD.confirmation OR NEW.inviter_seat_id IS NOT OLD.inviter_seat_id
BEGIN SELECT RAISE(ABORT, 'invitation policy and origin are immutable'); END;
