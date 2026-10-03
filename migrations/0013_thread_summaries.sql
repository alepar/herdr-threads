-- Thread summaries for compaction survival and soft-deadline pokes (epic
-- ht-1ip, spec §13). Inert until the summary, catch-up, extension and poke
-- tasks land.

-- Spec §1. author_role is new; author_kind (0002) is untouched.
ALTER TABLE messages ADD COLUMN author_role TEXT
    CHECK(author_role IN ('human', 'agent', 'service'));
ALTER TABLE messages ADD COLUMN relays_user INTEGER NOT NULL DEFAULT 0
    CHECK(relays_user IN (0, 1));
ALTER TABLE messages ADD COLUMN author_role_backfilled INTEGER NOT NULL DEFAULT 0
    CHECK(author_role_backfilled IN (0, 1));
DROP TRIGGER messages_immutable;
UPDATE messages SET author_role_backfilled = 1, author_role = CASE
    WHEN author_kind = 'programmatic' THEN 'service'
    WHEN actor_seat_id IS NULL THEN NULL
    ELSE (SELECT CASE b.harness WHEN 'human' THEN 'human' ELSE 'agent' END
          FROM occupant_bindings b
          WHERE b.seat_id = messages.actor_seat_id
            AND b.observed_at <= messages.decision_at
            AND (b.ended_at IS NULL OR b.ended_at > messages.decision_at)
          ORDER BY b.ordinal DESC LIMIT 1)
    END;
CREATE TRIGGER messages_immutable BEFORE UPDATE ON messages
BEGIN SELECT RAISE(ABORT, 'message history is immutable'); END;

CREATE TRIGGER messages_summary_author_insert BEFORE INSERT ON messages
BEGIN
    SELECT RAISE(ABORT, 'invalid message summary authorship') WHERE
        NEW.author_role_backfilled <> 0 OR
        (NEW.author_kind = 'programmatic' AND NEW.relays_user <> 0);
END;

CREATE TABLE summary_blocks (
    id TEXT PRIMARY KEY,
    instance_id TEXT NOT NULL REFERENCES host_instances(id),
    thread_id TEXT NOT NULL REFERENCES threads(id),
    chunking_version TEXT NOT NULL CHECK(length(chunking_version) BETWEEN 1 AND 64),
    level INTEGER NOT NULL CHECK(level >= 0),
    idx INTEGER NOT NULL CHECK(idx >= 0),
    first_seq INTEGER NOT NULL CHECK(first_seq > 0),
    last_seq INTEGER NOT NULL CHECK(last_seq >= first_seq),
    children_json TEXT NOT NULL DEFAULT '[]',
    source_hash TEXT NOT NULL,
    narrative TEXT NOT NULL CHECK(length(CAST(narrative AS BLOB)) <= 65536),
    fallback INTEGER NOT NULL DEFAULT 0 CHECK(fallback IN (0, 1)),
    provenance TEXT NOT NULL DEFAULT 'derived_summary' CHECK(provenance = 'derived_summary'),
    author_seat_id TEXT NOT NULL REFERENCES seats(id),
    model TEXT NOT NULL CHECK(length(CAST(model AS BLOB)) BETWEEN 1 AND 64),
    prompt_version TEXT NOT NULL CHECK(length(CAST(prompt_version AS BLOB)) BETWEEN 1 AND 64),
    job_id TEXT,
    created_at INTEGER NOT NULL,
    UNIQUE(thread_id, chunking_version, level, idx)
) STRICT;

CREATE TRIGGER summary_blocks_immutable BEFORE UPDATE ON summary_blocks
BEGIN SELECT RAISE(ABORT, 'summary blocks are immutable'); END;

CREATE TRIGGER summary_blocks_retained BEFORE DELETE ON summary_blocks
BEGIN SELECT RAISE(ABORT, 'summary blocks are retained'); END;

CREATE TABLE summary_items (
    block_id TEXT NOT NULL REFERENCES summary_blocks(id),
    kind TEXT NOT NULL CHECK(kind IN ('user_instruction', 'decision', 'open_item', 'identifier')),
    item_id TEXT NOT NULL,
    thread_id TEXT NOT NULL REFERENCES threads(id),
    chunking_version TEXT NOT NULL,
    seq INTEGER NOT NULL CHECK(seq > 0),
    body_json TEXT NOT NULL,
    PRIMARY KEY(block_id, kind, item_id)
) STRICT;

CREATE INDEX summary_items_fold ON summary_items(thread_id, chunking_version, seq);

CREATE TABLE summary_transitions (
    block_id TEXT NOT NULL REFERENCES summary_blocks(id),
    ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
    thread_id TEXT NOT NULL REFERENCES threads(id),
    chunking_version TEXT NOT NULL,
    target_id TEXT NOT NULL,
    new_status TEXT NOT NULL CHECK(new_status IN ('done', 'resolved', 'superseded')),
    cite_seq INTEGER NOT NULL CHECK(cite_seq > 0),
    PRIMARY KEY(block_id, ordinal)
) STRICT;

CREATE INDEX summary_transitions_fold ON summary_transitions(thread_id, chunking_version, cite_seq);

CREATE TABLE summary_jobs (
    id TEXT PRIMARY KEY,
    instance_id TEXT NOT NULL REFERENCES host_instances(id),
    thread_id TEXT NOT NULL REFERENCES threads(id),
    chunking_version TEXT NOT NULL,
    level INTEGER NOT NULL CHECK(level >= 0),
    idx INTEGER NOT NULL CHECK(idx >= 0),
    first_seq INTEGER NOT NULL CHECK(first_seq > 0),
    last_seq INTEGER NOT NULL CHECK(last_seq >= first_seq),
    lease_seat_id TEXT REFERENCES seats(id),
    lease_token TEXT,
    reserved_at INTEGER,
    fetched_at INTEGER,
    lease_until INTEGER,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts >= 0),
    rejections INTEGER NOT NULL DEFAULT 0 CHECK(rejections >= 0),
    last_submit_token TEXT,
    last_submit_digest BLOB,
    last_submit_result TEXT,
    block_id TEXT REFERENCES summary_blocks(id),
    created_at INTEGER NOT NULL,
    UNIQUE(thread_id, chunking_version, level, idx),
    CHECK((lease_token IS NULL) = (lease_seat_id IS NULL))
) STRICT;

CREATE INDEX summary_jobs_live_lease ON summary_jobs(lease_until) WHERE block_id IS NULL AND lease_until IS NOT NULL;

CREATE TABLE summary_job_durations (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    instance_id TEXT NOT NULL REFERENCES host_instances(id),
    job_id TEXT NOT NULL REFERENCES summary_jobs(id),
    duration_ms INTEGER NOT NULL CHECK(duration_ms >= 0),
    recorded_at INTEGER NOT NULL
) STRICT;

CREATE INDEX summary_job_durations_recent ON summary_job_durations(instance_id, ordinal);

CREATE TABLE catch_up (
    seat_id TEXT NOT NULL REFERENCES seats(id),
    thread_id TEXT NOT NULL REFERENCES threads(id),
    frontier_seq INTEGER NOT NULL CHECK(frontier_seq >= 0),
    binding_generation INTEGER NOT NULL CHECK(binding_generation >= 0),
    execution_id TEXT NOT NULL,
    entered_at INTEGER NOT NULL,
    extension_until INTEGER,
    last_progress_at INTEGER,
    state TEXT NOT NULL CHECK(state IN ('active', 'ended')),
    end_reason TEXT CHECK(end_reason IN ('ready', 'stalled', 'superseded')),
    ended_at INTEGER,
    PRIMARY KEY(seat_id, thread_id),
    CHECK((state = 'active') = (end_reason IS NULL)),
    CHECK((end_reason IS NULL) = (ended_at IS NULL))
) STRICT;

CREATE INDEX catch_up_extension_until ON catch_up(extension_until) WHERE extension_until IS NOT NULL;

CREATE INDEX catch_up_thread_active ON catch_up(thread_id, seat_id) WHERE state = 'active';

-- Spec §10: one poke per receipt soft point, set when the host accepts it.
-- Read through the effective receipt projection (ht-1ip.13).
ALTER TABLE receipts ADD COLUMN soft_poked_at INTEGER;
ALTER TABLE receipt_state ADD COLUMN soft_poked_at INTEGER;
