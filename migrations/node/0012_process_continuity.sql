-- C1 process continuity (ht-hsyg, TRUST-POLICY C1): a seat rebind the daemon decides on its own
-- process evidence is recorded in seat history as kind 'structural_process_continuity'. This file
-- creates the replacement table with the extended kind list; seats::copy_allocation_decisions
-- copies every row and 0012_process_continuity_after.sql swaps it in.
CREATE TABLE allocation_decisions_new (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    host_id INTEGER NOT NULL REFERENCES herdr_hosts(host_id),
    target_id TEXT NOT NULL,
    seat_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('ordinary', 'operator_fresh', 'operator_rebind', 'operator_retire', 'operator_human_override', 'cooperative_continuity', 'structural_process_continuity')),
    decided_at INTEGER NOT NULL,
    host_boot TEXT NOT NULL,
    epoch INTEGER NOT NULL CHECK(epoch >= 0),
    generation INTEGER NOT NULL CHECK(generation >= 0),
    operator_label TEXT,
    continuity_diagnostic TEXT CHECK(continuity_diagnostic IS NULL OR continuity_diagnostic IN ('match', 'mismatch', 'absent', 'read_error'))
) STRICT;
