-- Recipient refusal is retained separately from acceptance and service cancellation.
CREATE TABLE invitation_rejections (
    invitation_id TEXT PRIMARY KEY REFERENCES invitations(id),
    reason TEXT NOT NULL CHECK(length(CAST(reason AS BLOB)) BETWEEN 1 AND 4096),
    rejected_at INTEGER NOT NULL,
    actor_seat_id TEXT NOT NULL REFERENCES seats(id),
    generation INTEGER NOT NULL CHECK(generation >= 0),
    observation TEXT NOT NULL
) STRICT;
CREATE TRIGGER invitation_rejections_shape BEFORE INSERT ON invitation_rejections
BEGIN
    SELECT RAISE(ABORT, 'invalid invitation rejection') WHERE NOT EXISTS (
        SELECT 1 FROM invitations i WHERE i.id=NEW.invitation_id
        AND i.seat_id=NEW.actor_seat_id AND i.state='pending'
        AND NOT EXISTS(SELECT 1 FROM invitation_cancellations c WHERE c.invitation_id=i.id)
        AND NOT EXISTS(SELECT 1 FROM requirement_episodes r WHERE r.invitation_id=i.id AND r.state IN ('pending','accepted'))
    );
END;
CREATE TRIGGER invitation_rejections_immutable BEFORE UPDATE ON invitation_rejections
BEGIN SELECT RAISE(ABORT, 'invitation rejection is immutable'); END;
CREATE TRIGGER invitation_rejections_retained BEFORE DELETE ON invitation_rejections
BEGIN SELECT RAISE(ABORT, 'invitation rejection is retained'); END;
CREATE TRIGGER digest_invitation_rejected AFTER INSERT ON invitation_rejections
BEGIN
    DELETE FROM digest_pending_invitations WHERE invitation_id=NEW.invitation_id AND seat_id=NEW.actor_seat_id;
    DELETE FROM digest_open_warnings WHERE condition_kind='invitation' AND condition_id=NEW.invitation_id;
END;

-- Indexed projection only: the immutable ledger remains the rejection fact.
-- Keep rejected history out of the deadline walk's physical pending slice.
ALTER TABLE invitations ADD COLUMN reject_recorded INTEGER NOT NULL DEFAULT 0 CHECK(reject_recorded IN (0,1));
CREATE INDEX invitations_effective_pending_unwarned ON invitations(deadline_at,ordinal)
    WHERE state='pending' AND warning_message_id IS NULL AND reject_recorded=0;
CREATE TRIGGER invitations_rejection_projection_insert BEFORE INSERT ON invitations
WHEN NEW.reject_recorded<>0
BEGIN SELECT RAISE(ABORT, 'new invitation cannot claim rejection'); END;
CREATE TRIGGER invitations_rejection_projection_guard BEFORE UPDATE OF reject_recorded ON invitations
WHEN NEW.reject_recorded<>EXISTS(SELECT 1 FROM invitation_rejections r WHERE r.invitation_id=NEW.id)
BEGIN SELECT RAISE(ABORT, 'invitation rejection projection mismatch'); END;
CREATE TRIGGER invitation_rejections_project AFTER INSERT ON invitation_rejections
BEGIN UPDATE invitations SET reject_recorded=1 WHERE id=NEW.invitation_id; END;
