-- Swap the table seats::copy_allocation_decisions filled into place and recreate its indexes
-- (ht-hsyg).
DROP TABLE allocation_decisions;
ALTER TABLE allocation_decisions_new RENAME TO allocation_decisions;

CREATE INDEX allocation_decisions_target ON allocation_decisions(host_id, target_id, ordinal);
CREATE INDEX allocation_decisions_seat_history ON allocation_decisions(seat_id, ordinal);
