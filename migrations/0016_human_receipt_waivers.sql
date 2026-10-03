-- A human check-in waives unacknowledged receipt obligations through its
-- decision sequence. This is a cutoff, not an ACK; historical ACK provenance
-- remains untouched. A cutoff covers the human check-in decision; recorded
-- binding intervals and recipient availability cover later human sends while
-- preserving mail for an agent bound before its first availability decision.
CREATE TABLE human_receipt_waivers (
    seat_id TEXT PRIMARY KEY REFERENCES seats(id),
    through_decision_seq INTEGER NOT NULL CHECK(through_decision_seq >= 0),
    human_generation INTEGER NOT NULL CHECK(human_generation >= 0),
    decided_at INTEGER NOT NULL
) STRICT;

-- Keep the original state and ACK fields intact. This independent obligation
-- bit also makes waived pending history absent from bounded pending/due walks.
ALTER TABLE prepared_recipients ADD COLUMN ack_required INTEGER NOT NULL DEFAULT 1 CHECK(ack_required IN (0,1));
ALTER TABLE receipt_state ADD COLUMN ack_required INTEGER NOT NULL DEFAULT 1 CHECK(ack_required IN (0,1));
ALTER TABLE receipts ADD COLUMN ack_required INTEGER NOT NULL DEFAULT 1 CHECK(ack_required IN (0,1));

INSERT INTO human_receipt_waivers(seat_id,through_decision_seq,human_generation,decided_at)
SELECT b.seat_id, a.decision_seq, b.generation, a.decision_at
FROM occupant_bindings b
JOIN seat_availability a ON a.seat_id=b.seat_id AND a.binding_generation=b.generation
WHERE b.harness='human'
  AND a.decision_seq=(SELECT MIN(first.decision_seq) FROM seat_availability first
                      WHERE first.seat_id=b.seat_id AND first.binding_generation=b.generation)
ON CONFLICT(seat_id) DO UPDATE SET
    through_decision_seq=MAX(human_receipt_waivers.through_decision_seq,excluded.through_decision_seq),
    human_generation=CASE WHEN excluded.through_decision_seq>human_receipt_waivers.through_decision_seq
                          THEN excluded.human_generation ELSE human_receipt_waivers.human_generation END,
    decided_at=CASE WHEN excluded.through_decision_seq>human_receipt_waivers.through_decision_seq
                    THEN excluded.decided_at ELSE human_receipt_waivers.decided_at END;

-- Open human bindings can cover older unanchored rows too. There is no later
-- agent in this state, so the current decision high-water is a safe cutoff.
INSERT INTO human_receipt_waivers(seat_id,through_decision_seq,human_generation,decided_at)
SELECT b.seat_id,h.decision_seq,b.generation,b.observed_at
FROM occupant_bindings b
JOIN seats s ON s.id=b.seat_id
JOIN host_instances h ON h.id=s.instance_id
WHERE b.harness='human' AND b.ended_at IS NULL
ON CONFLICT(seat_id) DO UPDATE SET
    through_decision_seq=MAX(human_receipt_waivers.through_decision_seq,excluded.through_decision_seq),
    human_generation=excluded.human_generation,
    decided_at=excluded.decided_at;

-- A closed binding may have been temporarily unregistered at send time. Its
-- recipient provenance is then NULL; the recorded publication time within
-- the binding interval is the durable evidence of human ownership. The end
-- boundary is exclusive so a managed-launch agent's mail stays owed.
UPDATE prepared_recipients AS pr SET ack_required=0
WHERE pr.availability_provenance='operator_human'
   OR EXISTS (
       SELECT 1 FROM send_manifests sm
       WHERE sm.preparation_id=pr.preparation_id AND (
           EXISTS(SELECT 1 FROM human_receipt_waivers w WHERE w.seat_id=pr.seat_id AND sm.decision_seq<=w.through_decision_seq)
           OR EXISTS(SELECT 1 FROM occupant_bindings b WHERE b.seat_id=pr.seat_id AND b.harness='human'
                     AND sm.decision_at>=b.observed_at AND (b.ended_at IS NULL OR sm.decision_at<b.ended_at))
       )
   );

UPDATE receipts AS r SET ack_required=0
WHERE state='pending' AND EXISTS (
    SELECT 1 FROM messages m WHERE m.id=r.message_id AND (
        EXISTS(SELECT 1 FROM human_receipt_waivers w WHERE w.seat_id=r.seat_id AND m.decision_seq<=w.through_decision_seq)
        OR EXISTS(SELECT 1 FROM occupant_bindings b WHERE b.seat_id=r.seat_id AND b.harness='human'
                  AND m.decision_at>=b.observed_at AND (b.ended_at IS NULL OR m.decision_at<b.ended_at))
    )
);

UPDATE receipt_state AS rs SET ack_required=0
WHERE state='pending' AND EXISTS (
    SELECT 1 FROM send_manifests sm JOIN prepared_recipients pr ON pr.preparation_id=sm.preparation_id
    WHERE sm.message_id=rs.message_id AND pr.seat_id=rs.seat_id AND pr.ack_required=0
);

DELETE FROM digest_pending_manifest_receipts
WHERE EXISTS(SELECT 1 FROM prepared_recipients pr WHERE pr.preparation_id=digest_pending_manifest_receipts.preparation_id
             AND pr.seat_id=digest_pending_manifest_receipts.seat_id AND pr.ack_required=0);

DELETE FROM digest_open_warnings
WHERE condition_kind='receipt' AND (
    EXISTS(SELECT 1 FROM receipts r WHERE r.ack_required=0
           AND condition_id=length(CAST(r.message_id AS BLOB))||':'||r.message_id||':'||r.seat_id)
    OR EXISTS(SELECT 1 FROM prepared_recipients pr JOIN send_manifests sm ON sm.preparation_id=pr.preparation_id
              WHERE pr.ack_required=0 AND condition_id=length(CAST(sm.message_id AS BLOB))||':'||sm.message_id||':'||pr.seat_id)
);

CREATE TRIGGER human_receipt_prepared_waived AFTER UPDATE OF ack_required ON prepared_recipients WHEN NEW.ack_required=0
BEGIN
    DELETE FROM digest_pending_manifest_receipts WHERE preparation_id=NEW.preparation_id AND seat_id=NEW.seat_id;
    DELETE FROM digest_open_warnings WHERE condition_kind='receipt' AND condition_id IN
        (SELECT length(CAST(sm.message_id AS BLOB))||':'||sm.message_id||':'||NEW.seat_id
         FROM send_manifests sm WHERE sm.preparation_id=NEW.preparation_id);
END;

CREATE TRIGGER human_receipt_state_waived AFTER UPDATE OF ack_required ON receipt_state WHEN NEW.ack_required=0
BEGIN
    DELETE FROM digest_pending_manifest_receipts WHERE seat_id=NEW.seat_id AND preparation_id=(SELECT preparation_id FROM send_manifests WHERE message_id=NEW.message_id);
    DELETE FROM digest_open_warnings WHERE condition_kind='receipt' AND condition_id=length(CAST(NEW.message_id AS BLOB))||':'||NEW.message_id||':'||NEW.seat_id;
END;

CREATE TRIGGER human_receipt_state_insert_waived AFTER INSERT ON receipt_state WHEN NEW.ack_required=0
BEGIN
    DELETE FROM digest_pending_manifest_receipts WHERE seat_id=NEW.seat_id AND preparation_id=(SELECT preparation_id FROM send_manifests WHERE message_id=NEW.message_id);
    DELETE FROM digest_open_warnings WHERE condition_kind='receipt' AND condition_id=length(CAST(NEW.message_id AS BLOB))||':'||NEW.message_id||':'||NEW.seat_id;
END;

CREATE TRIGGER human_receipt_physical_waived AFTER UPDATE OF ack_required ON receipts WHEN NEW.ack_required=0
BEGIN
    DELETE FROM digest_open_warnings WHERE condition_kind='receipt' AND condition_id=length(CAST(NEW.message_id AS BLOB))||':'||NEW.message_id||':'||NEW.seat_id;
END;

CREATE INDEX receipts_required_seat_pending ON receipts(seat_id,ordinal) WHERE state='pending' AND ack_required=1;
CREATE INDEX prepared_recipients_required_seat ON prepared_recipients(seat_id,ordinal) WHERE ack_required=1;
CREATE INDEX receipts_required_thread_pending ON receipts(seat_id,thread_id,ordinal) WHERE state='pending' AND ack_required=1;
CREATE INDEX receipts_required_due ON receipts(deadline_at,ordinal) WHERE state='pending' AND ack_required=1 AND deadline_at IS NOT NULL AND warning_message_id IS NULL;
CREATE INDEX receipt_state_required_due ON receipt_state(deadline_at,message_id,seat_id) WHERE state='pending' AND ack_required=1 AND deadline_at IS NOT NULL AND warning_message_id IS NULL;
CREATE INDEX receipts_required_poke ON receipts(deadline_at,message_id,seat_id) WHERE state='pending' AND ack_required=1 AND deadline_at IS NOT NULL AND available_at IS NOT NULL AND warning_message_id IS NULL AND soft_poked_at IS NULL;
CREATE INDEX receipt_state_required_poke ON receipt_state(deadline_at,message_id,seat_id) WHERE state='pending' AND ack_required=1 AND deadline_at IS NOT NULL AND available_at IS NOT NULL AND warning_message_id IS NULL AND soft_poked_at IS NULL;

CREATE TABLE human_receipt_reconciliation_bounds (
    seat_id TEXT PRIMARY KEY REFERENCES seats(id),
    prepared_high_water INTEGER NOT NULL CHECK(prepared_high_water >= 0),
    physical_high_water INTEGER NOT NULL CHECK(physical_high_water >= 0),
    decision_seq INTEGER NOT NULL CHECK(decision_seq >= 0)
) STRICT;

-- A human check-in only writes a cutoff and one job. The existing work queue
-- reconciles its retained rows in bounded units after the deciding commit.
ALTER TABLE work_jobs RENAME TO work_jobs_v15;
CREATE TABLE work_jobs (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL CHECK(kind IN ('warning_attribution','send_attention','receipt_timer_materialization','preparation_cleanup','human_receipt_reconciliation')),
    subject_id TEXT NOT NULL,
    position INTEGER NOT NULL DEFAULT 0 CHECK(position >= 0),
    high_water INTEGER NOT NULL CHECK(high_water >= 0),
    completed_units INTEGER NOT NULL DEFAULT 0 CHECK(completed_units >= 0),
    status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','complete','failed')),
    last_error TEXT CHECK(last_error IS NULL OR length(CAST(last_error AS BLOB)) <= 512),
    completed_at INTEGER CHECK(completed_at IS NULL OR completed_at >= 0),
    UNIQUE(kind,subject_id)
) STRICT;
INSERT INTO work_jobs(ordinal,id,kind,subject_id,position,high_water,completed_units,status,last_error,completed_at)
SELECT ordinal,id,kind,subject_id,position,high_water,completed_units,status,last_error,completed_at FROM work_jobs_v15;
DROP TABLE work_jobs_v15;
CREATE INDEX work_jobs_ready ON work_jobs(status, kind, ordinal);
CREATE INDEX work_jobs_live ON work_jobs(ordinal) WHERE status IN ('pending','failed');
CREATE INDEX work_jobs_retention ON work_jobs(kind, completed_at) WHERE status='complete';
