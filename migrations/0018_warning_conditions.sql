-- Only transitions decided after this migration enter this ledger. Older
-- warning events keep their source-based actionability and gain no clear.
CREATE TABLE warning_conditions (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    condition_kind TEXT NOT NULL CHECK(condition_kind IN ('receipt','invitation','unavailable')),
    thread_id TEXT NOT NULL REFERENCES threads(id),
    condition_id TEXT NOT NULL,
    affected_seat_id TEXT NOT NULL REFERENCES seats(id),
    episode INTEGER,
    open_warning_id TEXT NOT NULL UNIQUE,
    opened_seq INTEGER NOT NULL CHECK(opened_seq > 0),
    clear_warning_id TEXT UNIQUE,
    cleared_seq INTEGER,
    CHECK((clear_warning_id IS NULL) = (cleared_seq IS NULL))
) STRICT;

-- A close sweep can logically close an old row before its physical clear event
-- is projected. A later open of the same condition key may coexist meanwhile.
CREATE INDEX warning_conditions_active ON warning_conditions(condition_kind,thread_id,condition_id,ordinal) WHERE clear_warning_id IS NULL;
CREATE INDEX warning_conditions_affected ON warning_conditions(affected_seat_id,condition_kind,ordinal) WHERE clear_warning_id IS NULL;
CREATE INDEX warning_conditions_close_scope ON warning_conditions(affected_seat_id,condition_kind,episode,ordinal) WHERE clear_warning_id IS NULL;
CREATE INDEX warning_conditions_thread ON warning_conditions(thread_id,ordinal);
CREATE INDEX warning_conditions_unavailable_reopen ON warning_conditions(thread_id,affected_seat_id,episode,ordinal) WHERE condition_kind='unavailable' AND clear_warning_id IS NULL;

-- One deciding write captures the closure boundary. The work job cursor drains
-- its rows by warning_conditions.ordinal; rows opened later are never swept.
CREATE TABLE warning_close_sweeps (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    condition_kind TEXT NOT NULL CHECK(condition_kind IN ('receipt','unavailable')),
    affected_seat_id TEXT NOT NULL REFERENCES seats(id),
    episode INTEGER,
    after_ordinal INTEGER NOT NULL CHECK(after_ordinal >= 0),
    through_ordinal INTEGER NOT NULL CHECK(through_ordinal > 0),
    close_decision_seq INTEGER NOT NULL CHECK(close_decision_seq >= 0),
    interval_high_water INTEGER NOT NULL CHECK(interval_high_water >= 0),
    decision_at INTEGER NOT NULL CHECK(decision_at >= 0),
    CHECK((condition_kind='unavailable') = (episode IS NOT NULL)),
    CHECK(after_ordinal < through_ordinal)
) STRICT;
CREATE INDEX warning_close_sweeps_cover ON warning_close_sweeps(condition_kind,affected_seat_id,episode,through_ordinal,after_ordinal);

-- Physical publication may lag the canonical close. Fanout uses this frozen
-- cutoff rather than the clear message's later decision sequence.
ALTER TABLE warning_jobs ADD COLUMN recipient_cutoff_seq INTEGER CHECK(recipient_cutoff_seq IS NULL OR recipient_cutoff_seq >= 0);

-- Keep closure work in the existing fair, budgeted work lane.
ALTER TABLE work_jobs RENAME TO work_jobs_v17;
CREATE TABLE work_jobs (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL CHECK(kind IN ('warning_attribution','warning_condition_close','send_attention','receipt_timer_materialization','preparation_cleanup','human_receipt_reconciliation')),
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
SELECT ordinal,id,kind,subject_id,position,high_water,completed_units,status,last_error,completed_at FROM work_jobs_v17;
DROP TABLE work_jobs_v17;
CREATE INDEX work_jobs_ready ON work_jobs(status, kind, ordinal);
CREATE INDEX work_jobs_live ON work_jobs(ordinal) WHERE status IN ('pending','failed');
CREATE INDEX work_jobs_retention ON work_jobs(kind, completed_at) WHERE status='complete';
