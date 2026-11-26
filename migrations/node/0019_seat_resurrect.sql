-- Seat resurrect (ht-eu3v, spec 2026-11-25). Local tables, never replicated, except that
-- obligation_expiries mirrors the seat.obligations_expired facts this node applied. This file
-- creates the replacement decision table with the extended kind list; seats::copy_allocation_decisions
-- copies every row and 0019_seat_resurrect_after.sql swaps it in.
CREATE TABLE allocation_decisions_new (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    host_id INTEGER NOT NULL REFERENCES herdr_hosts(host_id),
    target_id TEXT NOT NULL,
    seat_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('ordinary', 'operator_fresh', 'operator_rebind', 'operator_retire', 'operator_human_override', 'cooperative_continuity', 'structural_process_continuity', 'structural_shell_continuity', 'cooperative_continuity_key', 'service_unretire', 'service_finalize', 'operator_unretire', 'window_finalize', 'service_rebind')),
    decided_at INTEGER NOT NULL,
    host_boot TEXT NOT NULL,
    epoch INTEGER NOT NULL CHECK(epoch >= 0),
    generation INTEGER NOT NULL CHECK(generation >= 0),
    operator_label TEXT,
    continuity_diagnostic TEXT CHECK(continuity_diagnostic IS NULL OR continuity_diagnostic IN ('match', 'mismatch', 'absent', 'read_error'))
) STRICT;

-- When a timed (keyless) reversible row's obligations expired on its home node. NULL: not yet.
ALTER TABLE reversible_retirements ADD COLUMN obligations_expired_at INTEGER;
-- When a row was replayed into this store from a fact (own-log restore or a peer); NULL for the
-- fence's own row. A replayed local row's clocks run from here, so the rest of its own history
-- (a later seat.unretired) has a full window to arrive before the lane acts on it.
ALTER TABLE reversible_retirements ADD COLUMN timed_from INTEGER;

-- One row per expiry of a seat's obligations, on every node that applied it: an obligation to the
-- seat first written later with created_at <= expired_at settles at write. Never pruned: bounded
-- by the seat's retirement episodes.
CREATE TABLE obligation_expiries (
    seat_id TEXT NOT NULL REFERENCES seats(id),
    retired_at INTEGER NOT NULL,
    expired_at INTEGER NOT NULL,
    PRIMARY KEY(seat_id, retired_at),
    CHECK(expired_at >= retired_at)
) STRICT;
