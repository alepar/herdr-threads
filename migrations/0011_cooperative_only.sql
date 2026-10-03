-- ht-p03.2 (B4): schema v11 (renumbered from v10: main's B5 trust guards own v10), the cooperative-only skeleton.
--
-- The pre-cooperative verification layer (seat allocation by recovery
-- baseline guard, registration revocation by trusted loss evidence, native
-- caller permits and their decision fence, empty-shell/occupant-loss
-- reconciliation) is deleted from the code. The mechanical table sweep
-- (rg -w <table> src/) found no table that only those paths used:
-- allocation_decisions, recovery_baseline_releases and recovery_baseline_targets
-- are still written or read by the ordinary resolution, operator repair and
-- effective-disposition statements, so v11 drops no table.
--
-- ht-p03.12.1 (B1) appends its partial indexes to this same file, below the
-- marker line.

-- B1 (ht-p03.12.1) partial indexes
CREATE INDEX seats_live_ordinal ON seats(instance_id, ordinal) WHERE state!='retired';
CREATE INDEX work_jobs_live ON work_jobs(ordinal) WHERE status IN ('pending','failed');
ALTER TABLE work_jobs ADD COLUMN completed_at INTEGER CHECK(completed_at IS NULL OR completed_at >= 0);
CREATE INDEX work_jobs_retention ON work_jobs(kind, completed_at) WHERE status='complete';
CREATE INDEX snapshot_generations_retention ON snapshot_generations(instance_id, admission_sequence);
CREATE INDEX wake_work_reserved ON wake_work(seat_id) WHERE reservation_id IS NOT NULL;
