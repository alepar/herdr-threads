-- Unlanded41 refinement: four fixed-consumer scalar cursors, no message
-- projection/backfill. Added CHECK validation may inspect each retained
-- consumer row per column (up to4S accepted upgrade work).
ALTER TABLE invitation_presentation_consumers ADD COLUMN ordinary_publication_after INTEGER NOT NULL DEFAULT 0 CHECK(ordinary_publication_after>=0);
ALTER TABLE invitation_presentation_consumers ADD COLUMN ordinary_publication_high INTEGER NOT NULL DEFAULT 0 CHECK(ordinary_publication_high>=0);
ALTER TABLE invitation_presentation_consumers ADD COLUMN ordinary_passive_generation INTEGER NOT NULL DEFAULT 0 CHECK(ordinary_passive_generation>=0);
ALTER TABLE invitation_presentation_consumers ADD COLUMN ordinary_scan_passive_generation INTEGER NOT NULL DEFAULT 0 CHECK(ordinary_scan_passive_generation>=0);

-- Exact pending invitation confirmation overlay. One-time population/index work
-- scales with retained pending invitations; no runtime repair/bootstrap walker.
CREATE TABLE presentation_confirming_invitations (
    seat_id TEXT NOT NULL,
    invitation_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK(ordinal>0),
    PRIMARY KEY(seat_id,invitation_id)
) STRICT;
CREATE INDEX presentation_confirming_invitations_seat ON presentation_confirming_invitations(seat_id,ordinal);

INSERT INTO presentation_confirming_invitations(seat_id,invitation_id,ordinal)
SELECT d.seat_id,d.invitation_id,d.ordinal FROM digest_pending_invitations d
JOIN invitations i ON i.id=d.invitation_id AND i.seat_id=d.seat_id
WHERE i.confirmation IS NULL OR i.confirmation='require_ack'
 OR EXISTS(SELECT 1 FROM requirement_episodes r WHERE r.invitation_id=i.id AND r.state IN ('pending','accepted'));

CREATE TRIGGER presentation_confirming_digest_insert AFTER INSERT ON digest_pending_invitations
BEGIN
    INSERT INTO presentation_confirming_invitations(seat_id,invitation_id,ordinal)
    SELECT d.seat_id,d.invitation_id,d.ordinal FROM invitations i
    JOIN digest_pending_invitations d ON d.seat_id=i.seat_id AND d.invitation_id=i.id
    WHERE i.id=NEW.invitation_id AND (i.confirmation IS NULL OR i.confirmation='require_ack'
        OR EXISTS(SELECT 1 FROM requirement_episodes r WHERE r.invitation_id=i.id AND r.state IN ('pending','accepted')))
    ON CONFLICT(seat_id,invitation_id) DO UPDATE SET ordinal=excluded.ordinal;
END;

CREATE TRIGGER presentation_confirming_digest_delete AFTER DELETE ON digest_pending_invitations
BEGIN
    DELETE FROM presentation_confirming_invitations WHERE seat_id=OLD.seat_id AND invitation_id=OLD.invitation_id;
END;

CREATE TRIGGER presentation_confirming_digest_update AFTER UPDATE ON digest_pending_invitations
BEGIN
    DELETE FROM presentation_confirming_invitations WHERE seat_id=OLD.seat_id AND invitation_id=OLD.invitation_id;
    INSERT INTO presentation_confirming_invitations(seat_id,invitation_id,ordinal)
    SELECT d.seat_id,d.invitation_id,d.ordinal FROM invitations i
    JOIN digest_pending_invitations d ON d.seat_id=i.seat_id AND d.invitation_id=i.id
    WHERE i.id=NEW.invitation_id AND (i.confirmation IS NULL OR i.confirmation='require_ack'
        OR EXISTS(SELECT 1 FROM requirement_episodes r WHERE r.invitation_id=i.id AND r.state IN ('pending','accepted')))
    ON CONFLICT(seat_id,invitation_id) DO UPDATE SET ordinal=excluded.ordinal;
END;

CREATE TRIGGER presentation_confirming_requirement_insert AFTER INSERT ON requirement_episodes
BEGIN
    DELETE FROM presentation_confirming_invitations WHERE invitation_id=NEW.invitation_id AND seat_id=(SELECT seat_id FROM invitations WHERE id=NEW.invitation_id);
    INSERT INTO presentation_confirming_invitations(seat_id,invitation_id,ordinal)
    SELECT d.seat_id,d.invitation_id,d.ordinal FROM invitations i
    JOIN digest_pending_invitations d ON d.seat_id=i.seat_id AND d.invitation_id=i.id
    WHERE i.id=NEW.invitation_id AND (i.confirmation IS NULL OR i.confirmation='require_ack'
        OR EXISTS(SELECT 1 FROM requirement_episodes r WHERE r.invitation_id=i.id AND r.state IN ('pending','accepted')))
    ON CONFLICT(seat_id,invitation_id) DO UPDATE SET ordinal=excluded.ordinal;
END;

CREATE TRIGGER presentation_confirming_requirement_delete AFTER DELETE ON requirement_episodes
BEGIN
    DELETE FROM presentation_confirming_invitations WHERE invitation_id=OLD.invitation_id AND seat_id=(SELECT seat_id FROM invitations WHERE id=OLD.invitation_id);
    INSERT INTO presentation_confirming_invitations(seat_id,invitation_id,ordinal)
    SELECT d.seat_id,d.invitation_id,d.ordinal FROM invitations i
    JOIN digest_pending_invitations d ON d.seat_id=i.seat_id AND d.invitation_id=i.id
    WHERE i.id=OLD.invitation_id AND (i.confirmation IS NULL OR i.confirmation='require_ack'
        OR EXISTS(SELECT 1 FROM requirement_episodes r WHERE r.invitation_id=i.id AND r.state IN ('pending','accepted')))
    ON CONFLICT(seat_id,invitation_id) DO UPDATE SET ordinal=excluded.ordinal;
END;

CREATE TRIGGER presentation_confirming_requirement_update AFTER UPDATE OF invitation_id,state ON requirement_episodes
BEGIN
    DELETE FROM presentation_confirming_invitations WHERE invitation_id=OLD.invitation_id AND seat_id=(SELECT seat_id FROM invitations WHERE id=OLD.invitation_id);
    INSERT INTO presentation_confirming_invitations(seat_id,invitation_id,ordinal)
    SELECT d.seat_id,d.invitation_id,d.ordinal FROM invitations i
    JOIN digest_pending_invitations d ON d.seat_id=i.seat_id AND d.invitation_id=i.id
    WHERE i.id=OLD.invitation_id AND (i.confirmation IS NULL OR i.confirmation='require_ack'
        OR EXISTS(SELECT 1 FROM requirement_episodes r WHERE r.invitation_id=i.id AND r.state IN ('pending','accepted')))
    ON CONFLICT(seat_id,invitation_id) DO UPDATE SET ordinal=excluded.ordinal;
    DELETE FROM presentation_confirming_invitations WHERE invitation_id=NEW.invitation_id AND seat_id=(SELECT seat_id FROM invitations WHERE id=NEW.invitation_id);
    INSERT INTO presentation_confirming_invitations(seat_id,invitation_id,ordinal)
    SELECT d.seat_id,d.invitation_id,d.ordinal FROM invitations i
    JOIN digest_pending_invitations d ON d.seat_id=i.seat_id AND d.invitation_id=i.id
    WHERE i.id=NEW.invitation_id AND (i.confirmation IS NULL OR i.confirmation='require_ack'
        OR EXISTS(SELECT 1 FROM requirement_episodes r WHERE r.invitation_id=i.id AND r.state IN ('pending','accepted')))
    ON CONFLICT(seat_id,invitation_id) DO UPDATE SET ordinal=excluded.ordinal;
END;

-- Fixed-key indexed physical ranges; judgment happens after LIMIT.
CREATE INDEX presentation_conditions_affected ON warning_conditions(affected_seat_id,ordinal) WHERE clear_warning_id IS NULL;
CREATE INDEX presentation_membership_history ON membership_intervals(thread_id,seat_id,ordinal);
CREATE INDEX presentation_prepared_warning ON prepared_unavailable_warnings(warning_id,ordinal);

-- Exactly one current bounded proof per retained seat/fixed consumer. Its JSON
-- contains 13 cursor/judgment records and one witness, never a history/map.
CREATE TABLE presentation_warning_proofs (
    seat_id TEXT NOT NULL REFERENCES seats(id) ON DELETE CASCADE,
    consumer TEXT NOT NULL CHECK(consumer IN ('hook','watch')),
    proof_json TEXT NOT NULL CHECK(length(CAST(proof_json AS BLOB))<=32768 AND json_valid(proof_json)),
    PRIMARY KEY(seat_id,consumer)
) STRICT;
CREATE TRIGGER presentation_warning_proof_reclaim AFTER DELETE ON invitation_presentation_consumers
BEGIN DELETE FROM presentation_warning_proofs WHERE seat_id=OLD.seat_id AND consumer=OLD.consumer; END;

-- Source/judgment invalidation only: NOT output/native version or wake work.
CREATE TABLE presentation_warning_fence (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    version INTEGER NOT NULL CHECK(version>=0)
) STRICT;
INSERT INTO presentation_warning_fence VALUES(1,0);

CREATE TRIGGER presentation_fence_messages_insert AFTER INSERT ON messages
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_messages_update AFTER UPDATE ON messages
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_messages_delete AFTER DELETE ON messages
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_warning_conditions_insert AFTER INSERT ON warning_conditions
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_warning_conditions_update AFTER UPDATE ON warning_conditions
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_warning_conditions_delete AFTER DELETE ON warning_conditions
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_receipt_warning_origins_insert AFTER INSERT ON receipt_warning_origins
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_receipt_warning_origins_update AFTER UPDATE ON receipt_warning_origins
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_receipt_warning_origins_delete AFTER DELETE ON receipt_warning_origins
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_warning_jobs_insert AFTER INSERT ON warning_jobs
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_warning_jobs_update AFTER UPDATE ON warning_jobs
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_warning_jobs_delete AFTER DELETE ON warning_jobs
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_warning_recipients_insert AFTER INSERT ON warning_recipients
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_warning_recipients_update AFTER UPDATE ON warning_recipients
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_warning_recipients_delete AFTER DELETE ON warning_recipients
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_prepared_unavailable_warnings_insert AFTER INSERT ON prepared_unavailable_warnings
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_prepared_unavailable_warnings_update AFTER UPDATE ON prepared_unavailable_warnings
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_prepared_unavailable_warnings_delete AFTER DELETE ON prepared_unavailable_warnings
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_work_jobs_insert AFTER INSERT ON work_jobs
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_work_jobs_update AFTER UPDATE ON work_jobs
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_work_jobs_delete AFTER DELETE ON work_jobs
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_send_preparations_insert AFTER INSERT ON send_preparations
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_send_preparations_update AFTER UPDATE ON send_preparations
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_send_preparations_delete AFTER DELETE ON send_preparations
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_send_manifests_insert AFTER INSERT ON send_manifests
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_send_manifests_update AFTER UPDATE ON send_manifests
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_send_manifests_delete AFTER DELETE ON send_manifests
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_prepared_recipients_insert AFTER INSERT ON prepared_recipients
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_prepared_recipients_update AFTER UPDATE ON prepared_recipients
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_prepared_recipients_delete AFTER DELETE ON prepared_recipients
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_receipt_state_insert AFTER INSERT ON receipt_state
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_receipt_state_update AFTER UPDATE ON receipt_state
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_receipt_state_delete AFTER DELETE ON receipt_state
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_receipts_insert AFTER INSERT ON receipts
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_receipts_update AFTER UPDATE ON receipts
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_receipts_delete AFTER DELETE ON receipts
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_receipt_send_time_cutovers_insert AFTER INSERT ON receipt_send_time_cutovers
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_receipt_send_time_cutovers_update AFTER UPDATE ON receipt_send_time_cutovers
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_receipt_send_time_cutovers_delete AFTER DELETE ON receipt_send_time_cutovers
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_invitations_insert AFTER INSERT ON invitations
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_invitations_update AFTER UPDATE ON invitations
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_invitations_delete AFTER DELETE ON invitations
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_invitation_cancellations_insert AFTER INSERT ON invitation_cancellations
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_invitation_cancellations_update AFTER UPDATE ON invitation_cancellations
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_invitation_cancellations_delete AFTER DELETE ON invitation_cancellations
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_invitation_rejections_insert AFTER INSERT ON invitation_rejections
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_invitation_rejections_update AFTER UPDATE ON invitation_rejections
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_invitation_rejections_delete AFTER DELETE ON invitation_rejections
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_requirement_episodes_insert AFTER INSERT ON requirement_episodes
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_requirement_episodes_update AFTER UPDATE ON requirement_episodes
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_requirement_episodes_delete AFTER DELETE ON requirement_episodes
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_seats_insert AFTER INSERT ON seats
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_seats_update AFTER UPDATE ON seats
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_seats_delete AFTER DELETE ON seats
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_membership_intervals_insert AFTER INSERT ON membership_intervals
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_membership_intervals_update AFTER UPDATE ON membership_intervals
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_membership_intervals_delete AFTER DELETE ON membership_intervals
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_memberships_insert AFTER INSERT ON memberships
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_memberships_update AFTER UPDATE ON memberships
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_memberships_delete AFTER DELETE ON memberships
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_occupant_bindings_insert AFTER INSERT ON occupant_bindings
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_occupant_bindings_update AFTER UPDATE ON occupant_bindings
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_occupant_bindings_delete AFTER DELETE ON occupant_bindings
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_seat_availability_insert AFTER INSERT ON seat_availability
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_seat_availability_update AFTER UPDATE ON seat_availability
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_seat_availability_delete AFTER DELETE ON seat_availability
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_human_receipt_waivers_insert AFTER INSERT ON human_receipt_waivers
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_human_receipt_waivers_update AFTER UPDATE ON human_receipt_waivers
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_human_receipt_waivers_delete AFTER DELETE ON human_receipt_waivers
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_service_notification_publications_insert AFTER INSERT ON service_notification_publications
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_service_notification_publications_update AFTER UPDATE ON service_notification_publications
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_service_notification_publications_delete AFTER DELETE ON service_notification_publications
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_service_notification_recipients_insert AFTER INSERT ON service_notification_recipients
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_service_notification_recipients_update AFTER UPDATE ON service_notification_recipients
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_service_notification_recipients_delete AFTER DELETE ON service_notification_recipients
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_digest_open_warnings_insert AFTER INSERT ON digest_open_warnings
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_digest_open_warnings_update AFTER UPDATE ON digest_open_warnings
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_digest_open_warnings_delete AFTER DELETE ON digest_open_warnings
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_digest_open_warning_recipients_insert AFTER INSERT ON digest_open_warning_recipients
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_digest_open_warning_recipients_update AFTER UPDATE ON digest_open_warning_recipients
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_digest_open_warning_recipients_delete AFTER DELETE ON digest_open_warning_recipients
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_digest_programmatic_warnings_insert AFTER INSERT ON digest_programmatic_warnings
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_digest_programmatic_warnings_update AFTER UPDATE ON digest_programmatic_warnings
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_digest_programmatic_warnings_delete AFTER DELETE ON digest_programmatic_warnings
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_warning_close_sweeps_insert AFTER INSERT ON warning_close_sweeps
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_warning_close_sweeps_update AFTER UPDATE ON warning_close_sweeps
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;

CREATE TRIGGER presentation_fence_warning_close_sweeps_delete AFTER DELETE ON warning_close_sweeps
BEGIN
    SELECT CASE WHEN COALESCE((SELECT version FROM presentation_warning_fence WHERE singleton=1),-1)<0
        THEN RAISE(ABORT,'presentation warning fence missing') END;
    SELECT CASE WHEN (SELECT version FROM presentation_warning_fence WHERE singleton=1)>=9223372036854775807
        THEN RAISE(ABORT,'presentation warning fence exhausted') END;
    UPDATE presentation_warning_fence SET version=version+1 WHERE singleton=1;
END;
