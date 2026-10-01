-- A required-only pending invitation is cancelled on release without
-- rewriting the invitation's accepted/retired history or its deadline.
CREATE TABLE invitation_cancellations (
    invitation_id TEXT PRIMARY KEY REFERENCES invitations(id),
    requirement_id TEXT NOT NULL UNIQUE REFERENCES requirement_episodes(id),
    cancelled_at INTEGER NOT NULL
) STRICT;
CREATE TRIGGER invitation_cancellations_shape BEFORE INSERT ON invitation_cancellations
BEGIN
    SELECT RAISE(ABORT, 'invalid invitation cancellation') WHERE NOT EXISTS (
        SELECT 1 FROM invitations i JOIN requirement_episodes r ON r.invitation_id=i.id
        WHERE i.id=NEW.invitation_id AND r.id=NEW.requirement_id
            AND i.state='pending' AND r.state='released'
            AND r.created_decision_seq=i.created_decision_seq
    );
END;
CREATE TRIGGER invitation_cancellations_immutable BEFORE UPDATE ON invitation_cancellations
BEGIN SELECT RAISE(ABORT, 'invitation cancellation is immutable'); END;
CREATE TRIGGER invitation_cancellations_retained BEFORE DELETE ON invitation_cancellations
BEGIN SELECT RAISE(ABORT, 'invitation cancellation is retained'); END;
