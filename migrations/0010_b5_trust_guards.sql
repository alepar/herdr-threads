-- B5 trust policy guards (TRUST-POLICY C1-C3). One migration for the epic.
-- Reconciliation marker: the recovery boot/epoch whose saved-seat pass
-- finished with no refused transition (decision 1 of the B5 design).
ALTER TABLE host_instances ADD COLUMN reconciled_boot TEXT;
ALTER TABLE host_instances ADD COLUMN reconciled_epoch INTEGER CHECK(reconciled_epoch IS NULL OR reconciled_epoch >= 0);
-- SQLite cannot alter a CHECK: rebuild allocation_decisions with identical
-- rows, ordinals and indexes; the kind set grows and a nullable diagnostic
-- column records the cooperative-continuity Herdr comparison. No trigger,
-- view or foreign key refers to allocation_decisions.
CREATE TABLE allocation_decisions_v10 (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    instance_id TEXT NOT NULL REFERENCES host_instances(id),
    target_id TEXT NOT NULL,
    seat_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('ordinary', 'operator_fresh', 'operator_rebind', 'operator_retire', 'operator_human_override', 'cooperative_continuity')),
    decided_at INTEGER NOT NULL,
    host_boot TEXT NOT NULL,
    epoch INTEGER NOT NULL CHECK(epoch >= 0),
    generation INTEGER NOT NULL CHECK(generation >= 0),
    operator_label TEXT,
    continuity_diagnostic TEXT CHECK(continuity_diagnostic IS NULL OR continuity_diagnostic IN ('match', 'mismatch', 'absent', 'read_error'))
) STRICT;
INSERT INTO allocation_decisions_v10 (ordinal, instance_id, target_id, seat_id, kind, decided_at, host_boot, epoch, generation, operator_label)
SELECT ordinal, instance_id, target_id, seat_id, kind, decided_at, host_boot, epoch, generation, operator_label FROM allocation_decisions;
DROP TABLE allocation_decisions;
ALTER TABLE allocation_decisions_v10 RENAME TO allocation_decisions;
CREATE INDEX allocation_decisions_target ON allocation_decisions(instance_id, target_id, ordinal);
CREATE INDEX allocation_decisions_seat_history ON allocation_decisions(seat_id, ordinal);
