-- herdr-threads node store baseline (application_id 0x48544E44 'HTND', user_version 1).
--
-- Produced from the v46 migration chain (ht-5fr.1.4): the objects of a fresh
-- v46 store in sqlite_master rowid order (tables, then indexes, then views, then
-- triggers; views precede triggers because an INSTEAD OF trigger needs its view),
-- each statement verbatim, then the rows a fresh store holds. The identity marker
-- is 'herdr-threads-node-v1'. No PRAGMAs here: `schema::initialize` sets
-- application_id and user_version in the same transaction.
--
-- Cutover rule: this file is edited in place until the first `v*` release tag
-- that ships the node store. After that it is frozen: every schema change is an
-- additive `migrations/node/NNNN_<slug>.sql` (NNNN = the `user_version` it
-- produces, from 0002) listed in NODE_DELTAS in src/store/schema.rs, and an
-- applied delta is never edited.
--
-- Herdr-scope child tables (snapshot_generations, observed_targets,
-- recovery_baseline_releases, recovery_baseline_targets, recovery_holds,
-- allocation_decisions) are keyed by host_id (ht-5fr.2.4); target ids are unique
-- only within one Herdr server. Archival is one node-wide lane
-- (archival_instances keyed by instance_id) with per-host evidence in
-- archival_hosts, and each seat_archival row names its seat's host (ht-5fr.2.9).
--
-- Node row delta (ht-5fr.1.5): v46's host_instances is split into `local_node`
-- (the node's own identity, tag and the node-scope counters decision_seq,
-- send_eligibility_revision, duration_config_revision) and `herdr_hosts` (one row
-- per attached Herdr server, holding every Herdr-scope epoch, sequence and
-- snapshot pointer); `node_directory` is the node-name table (this store writes
-- only the self row at initialization). Every instance_id column keeps its name
-- and now references local_node(id).
--
-- A fresh store has no legacy rows, so the two receipt backfill singletons start
-- with completed = 1 (the v46 chain started them at 0 for its backfill workers).
--
-- `schema::verify_baseline` audits every open store against this file: it is the
-- one schema check.

CREATE TABLE schema_identity (
    marker TEXT PRIMARY KEY CHECK(marker = 'herdr-threads-node-v1')
) STRICT;

CREATE TABLE local_node (
    id TEXT PRIMARY KEY CHECK(length(id) = 32 AND id NOT GLOB '*[^0-9a-f]*'),
    singleton INTEGER NOT NULL DEFAULT 1 UNIQUE CHECK(singleton = 1),
    created_at INTEGER NOT NULL,
    tag TEXT NOT NULL CHECK(length(tag) = 3 AND tag NOT GLOB '*[^0-9A-Za-z]*'),
    tag_k INTEGER NOT NULL DEFAULT 0 CHECK(tag_k >= 0),
    decision_seq INTEGER NOT NULL DEFAULT 0 CHECK(decision_seq >= 0),
    send_eligibility_revision INTEGER NOT NULL DEFAULT 0 CHECK(send_eligibility_revision >= 0),
    duration_config_revision INTEGER NOT NULL DEFAULT 0 CHECK(duration_config_revision >= 0),
    -- The log this node authors into; NULL until journal::open bootstraps (ht-5fr.3.4).
    active_log_ix INTEGER REFERENCES sync_logs(log_ix)
) STRICT;

CREATE TABLE herdr_hosts (
    host_id INTEGER PRIMARY KEY AUTOINCREMENT,
    node_id TEXT NOT NULL REFERENCES local_node(id),
    endpoint TEXT NOT NULL UNIQUE CHECK(length(CAST(endpoint AS BLOB)) BETWEEN 1 AND 4096),
    created_at INTEGER NOT NULL,
    last_attached_at INTEGER,
    host_boot TEXT,
    host_epoch INTEGER NOT NULL DEFAULT 0 CHECK(host_epoch >= 0),
    observation_sequence INTEGER NOT NULL DEFAULT 0 CHECK(observation_sequence >= 0),
    recovery_boot TEXT,
    recovery_epoch INTEGER,
    lifecycle_revision INTEGER NOT NULL DEFAULT 0 CHECK(lifecycle_revision >= 0),
    active_snapshot_id TEXT REFERENCES snapshot_generations(id),
    recovery_baseline_generation_id TEXT REFERENCES snapshot_generations(id),
    baseline_hold_unclaimed INTEGER NOT NULL DEFAULT 0 CHECK(baseline_hold_unclaimed IN (0,1)),
    invalidation_revision INTEGER NOT NULL DEFAULT 0 CHECK(invalidation_revision >= 0),
    observation_admission_sequence INTEGER NOT NULL DEFAULT 0 CHECK(observation_admission_sequence >= 0),
    observation_decided_sequence INTEGER NOT NULL DEFAULT 0 CHECK(observation_decided_sequence >= 0 AND observation_decided_sequence <= observation_admission_sequence),
    reconciled_boot TEXT,
    reconciled_epoch INTEGER CHECK(reconciled_epoch IS NULL OR reconciled_epoch >= 0)
) STRICT;

CREATE TABLE node_directory (
    node_uuid TEXT PRIMARY KEY CHECK(length(node_uuid) = 32 AND node_uuid NOT GLOB '*[^0-9a-f]*'),
    tag TEXT NOT NULL CHECK(length(tag) = 3 AND tag NOT GLOB '*[^0-9A-Za-z]*'),
    tag_k INTEGER NOT NULL CHECK(tag_k >= 0),
    host_label TEXT CHECK(host_label IS NULL OR length(CAST(host_label AS BLOB)) <= 255),
    first_seen_at INTEGER NOT NULL,
    tag_hlc INTEGER
) STRICT;

CREATE INDEX node_directory_tag ON node_directory(tag);

-- Journal core (ht-5fr.3.4, journal-core design section 4). Code owners: the
-- Journal (ht-5fr.3.6) and ThreadRegisters (ht-5fr.3.11).
CREATE TABLE sync_logs (
    log_ix INTEGER PRIMARY KEY,
    node_uuid TEXT NOT NULL CHECK(length(node_uuid) = 32 AND node_uuid NOT GLOB '*[^0-9a-f]*'),
    incarnation INTEGER NOT NULL CHECK(incarnation BETWEEN 1 AND 4294967295),
    is_self INTEGER NOT NULL CHECK(is_self IN (0,1)),
    pubkey BLOB NOT NULL CHECK(length(pubkey) = 32),
    genesis_hash BLOB NOT NULL CHECK(length(genesis_hash) = 32),
    contiguous_counter INTEGER NOT NULL CHECK(contiguous_counter >= 0),
    head_hash BLOB NOT NULL CHECK(length(head_hash) = 32),
    applied_through INTEGER NOT NULL DEFAULT 0 CHECK(applied_through BETWEEN 0 AND contiguous_counter),
    blocked_on TEXT CHECK(blocked_on IS NULL OR length(blocked_on) <= 200),
    state TEXT NOT NULL DEFAULT 'active' CHECK(state IN ('active','frozen')),
    frozen_at_counter INTEGER CHECK((state = 'frozen') = (frozen_at_counter IS NOT NULL)),
    skew_ahead_ms INTEGER NOT NULL DEFAULT 0 CHECK(skew_ahead_ms >= 0),
    first_seen_at INTEGER NOT NULL,
    UNIQUE(node_uuid, incarnation)
) STRICT;

CREATE TABLE journal (
    ingest_seq INTEGER PRIMARY KEY AUTOINCREMENT,
    log_ix INTEGER NOT NULL REFERENCES sync_logs(log_ix),
    counter INTEGER NOT NULL CHECK(counter >= 1),
    prev_hash BLOB NOT NULL CHECK(length(prev_hash) = 32),
    hash BLOB NOT NULL CHECK(length(hash) = 32),
    hlc INTEGER NOT NULL CHECK(hlc >= 0),
    kind TEXT NOT NULL CHECK(length(kind) BETWEEN 1 AND 64),
    kind_version INTEGER NOT NULL CHECK(kind_version BETWEEN 1 AND 65535),
    deps BLOB NOT NULL,
    creates TEXT CHECK(creates IS NULL OR length(creates) BETWEEN 1 AND 160),
    needs TEXT NOT NULL DEFAULT '',
    payload BLOB NOT NULL CHECK(length(payload) <= 524288),
    signature BLOB NOT NULL CHECK(length(signature) = 64),
    ingested_at INTEGER NOT NULL,
    apply_state TEXT NOT NULL CHECK(apply_state IN ('applied','pending','parked_unknown','parked_dep','failed')),
    applied_seq INTEGER CHECK((apply_state = 'applied') = (applied_seq IS NOT NULL)),
    parked_on TEXT CHECK((apply_state = 'parked_dep') = (parked_on IS NOT NULL)),
    apply_error TEXT CHECK(apply_error IS NULL OR length(CAST(apply_error AS BLOB)) <= 512),
    UNIQUE(log_ix, counter)
) STRICT;

CREATE UNIQUE INDEX journal_applied_seq ON journal(applied_seq) WHERE applied_seq IS NOT NULL;
CREATE INDEX journal_unapplied ON journal(apply_state, ingest_seq) WHERE apply_state <> 'applied';
CREATE INDEX journal_creates ON journal(creates) WHERE creates IS NOT NULL;
CREATE INDEX journal_parked_on ON journal(parked_on) WHERE apply_state = 'parked_dep';

-- Sync coordinator tables (ht-5fr.5.4, peer-link design sections 12 and 13).
-- Code owners: store::sync_dial_targets and store::sync_contacts.
CREATE TABLE sync_dial_targets (
    target_id INTEGER PRIMARY KEY,
    ssh_dest TEXT NOT NULL UNIQUE CHECK(length(ssh_dest) BETWEEN 1 AND 1024 AND substr(ssh_dest,1,1) <> '-'),
    source TEXT NOT NULL CHECK(source IN ('explicit','herdr_machine','herdr_remote_process')),
    enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0,1)),
    remote_command TEXT CHECK(remote_command IS NULL OR length(remote_command) BETWEEN 1 AND 1024),
    learned_node_uuid TEXT CHECK(learned_node_uuid IS NULL OR length(learned_node_uuid) = 32),
    state TEXT NOT NULL DEFAULT 'idle' CHECK(state IN ('idle','dialing','linked','backoff','not_installed','stopped')),
    stopped_reason TEXT CHECK(stopped_reason IS NULL OR stopped_reason IN ('envelope_unsupported','self_link','auth_failed','node_collision')),
    stopped_version TEXT,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts >= 0),
    next_attempt_at INTEGER,
    last_error TEXT CHECK(last_error IS NULL OR length(last_error) <= 512),
    last_error_at INTEGER,
    last_linked_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    CHECK((state = 'stopped') = (stopped_reason IS NOT NULL))
) STRICT;

CREATE TABLE sync_node_contact (
    node_uuid TEXT PRIMARY KEY CHECK(length(node_uuid) = 32),
    last_contact_at INTEGER NOT NULL,
    via TEXT NOT NULL CHECK(via IN ('direct','relayed')),
    updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE journal_forked (
    forked_ix INTEGER PRIMARY KEY AUTOINCREMENT,
    log_ix INTEGER NOT NULL REFERENCES sync_logs(log_ix),
    counter INTEGER NOT NULL CHECK(counter >= 1),
    prev_hash BLOB NOT NULL CHECK(length(prev_hash) = 32),
    hash BLOB NOT NULL CHECK(length(hash) = 32),
    hlc INTEGER NOT NULL CHECK(hlc >= 0),
    kind TEXT NOT NULL CHECK(length(kind) BETWEEN 1 AND 64),
    kind_version INTEGER NOT NULL CHECK(kind_version BETWEEN 1 AND 65535),
    deps BLOB NOT NULL,
    creates TEXT CHECK(creates IS NULL OR length(creates) BETWEEN 1 AND 160),
    needs TEXT NOT NULL DEFAULT '',
    payload BLOB NOT NULL CHECK(length(payload) <= 524288),
    signature BLOB NOT NULL CHECK(length(signature) = 64),
    source_node TEXT CHECK(source_node IS NULL OR length(source_node) = 32),
    received_at INTEGER NOT NULL,
    UNIQUE(log_ix, counter, hash)
) STRICT;

CREATE TABLE thread_registers (
    thread_id TEXT NOT NULL,
    field TEXT NOT NULL CHECK(field IN ('topic','name','archived')),
    value TEXT,
    hlc INTEGER NOT NULL CHECK(hlc >= 0),
    node_uuid TEXT NOT NULL CHECK(length(node_uuid) = 32 AND node_uuid NOT GLOB '*[^0-9a-f]*'),
    src_ingest_seq INTEGER NOT NULL REFERENCES journal(ingest_seq),
    PRIMARY KEY(thread_id, field)
) STRICT, WITHOUT ROWID;

CREATE TRIGGER journal_envelope_immutable BEFORE UPDATE OF log_ix, counter, prev_hash, hash, hlc, kind, kind_version, deps, creates, needs, payload, signature, ingested_at ON journal
BEGIN SELECT RAISE(ABORT, 'journal_envelope_immutable'); END;

CREATE TRIGGER journal_append_only BEFORE DELETE ON journal
BEGIN SELECT RAISE(ABORT, 'journal_append_only'); END;

CREATE TRIGGER journal_applied_terminal BEFORE UPDATE OF apply_state, applied_seq ON journal
WHEN OLD.apply_state = 'applied'
BEGIN SELECT RAISE(ABORT, 'journal_applied_terminal'); END;

CREATE TRIGGER journal_forked_immutable BEFORE UPDATE ON journal_forked
BEGIN SELECT RAISE(ABORT, 'journal_forked_immutable'); END;

CREATE TRIGGER journal_forked_append_only BEFORE DELETE ON journal_forked
BEGIN SELECT RAISE(ABORT, 'journal_forked_append_only'); END;

CREATE TRIGGER sync_logs_identity_immutable BEFORE UPDATE OF node_uuid, incarnation, is_self, pubkey, genesis_hash ON sync_logs
BEGIN SELECT RAISE(ABORT, 'sync_logs_identity_immutable'); END;

CREATE TRIGGER sync_logs_append_only BEFORE DELETE ON sync_logs
BEGIN SELECT RAISE(ABORT, 'sync_logs_append_only'); END;

CREATE TRIGGER sync_logs_monotone BEFORE UPDATE ON sync_logs
WHEN NEW.contiguous_counter < OLD.contiguous_counter
  OR NEW.applied_through < OLD.applied_through
  OR (OLD.state = 'frozen' AND NEW.state <> 'frozen')
BEGIN SELECT RAISE(ABORT, 'sync_logs_monotone'); END;

CREATE TRIGGER sync_logs_self_flag BEFORE INSERT ON sync_logs
WHEN NEW.is_self <> (NEW.node_uuid = (SELECT id FROM local_node))
BEGIN SELECT RAISE(ABORT, 'sync_logs_self_flag'); END;

CREATE TRIGGER thread_registers_lww BEFORE UPDATE ON thread_registers
WHEN (NEW.hlc, NEW.node_uuid) <= (OLD.hlc, OLD.node_uuid)
BEGIN SELECT RAISE(ABORT, 'thread_registers_lww'); END;

CREATE TRIGGER thread_registers_append_only BEFORE DELETE ON thread_registers
BEGIN SELECT RAISE(ABORT, 'thread_registers_append_only'); END;

-- Fork repair state (ht-5fr.6.4, incarnations design section 2). Code owner:
-- IncarnationStore (store::incarnations).
CREATE TABLE sync_forks (
    fork_ix INTEGER PRIMARY KEY,
    log_ix INTEGER NOT NULL REFERENCES sync_logs(log_ix),
    origin TEXT NOT NULL CHECK(origin IN ('evidence','declared')),
    proof_counter INTEGER CHECK(proof_counter IS NULL OR proof_counter >= 1),
    held_counter INTEGER,
    held_hash BLOB CHECK(held_hash IS NULL OR length(held_hash) = 32),
    conflicting_counter INTEGER,
    conflicting_hash BLOB CHECK(conflicting_hash IS NULL OR length(conflicting_hash) = 32),
    frozen_at INTEGER NOT NULL CHECK(frozen_at >= 0),
    source_node TEXT CHECK(source_node IS NULL OR length(source_node) = 32),
    detected_at INTEGER NOT NULL,
    CHECK((origin = 'evidence') = (proof_counter IS NOT NULL AND held_hash IS NOT NULL AND conflicting_hash IS NOT NULL)),
    UNIQUE(log_ix, held_counter, held_hash, conflicting_counter, conflicting_hash)
) STRICT;

CREATE TABLE sync_branches (
    branch_ix INTEGER PRIMARY KEY,
    log_ix INTEGER NOT NULL REFERENCES sync_logs(log_ix),
    tip_counter INTEGER NOT NULL CHECK(tip_counter >= 1),
    tip_hash BLOB NOT NULL CHECK(length(tip_hash) = 32),
    next_from INTEGER NOT NULL CHECK(next_from >= 1),
    search_span INTEGER NOT NULL DEFAULT 0 CHECK(search_span >= 0),
    state TEXT NOT NULL CHECK(state IN ('pulling','complete','unavailable')),
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt_at INTEGER,
    last_error TEXT CHECK(last_error IS NULL OR length(last_error) <= 512),
    first_seen_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(log_ix, tip_hash)
) STRICT;

CREATE TABLE sync_reissues (
    source_log_ix INTEGER NOT NULL REFERENCES sync_logs(log_ix),
    source_counter INTEGER NOT NULL,
    source_hash BLOB NOT NULL CHECK(length(source_hash) = 32),
    target_ingest_seq INTEGER NOT NULL REFERENCES journal(ingest_seq),
    reissued_at INTEGER NOT NULL,
    PRIMARY KEY(source_log_ix, source_counter, source_hash)
) STRICT, WITHOUT ROWID;

-- Clone reports and own_unknown_newer / lineage_branch logs (ht-5fr.6.7, incarnations
-- design section 2). Rows are never deleted; a repeat report of an open row advances
-- only last_seen_at, and clearing sets cleared_at/cleared_by once.
CREATE TABLE sync_clone_evidence (
    ev_ix INTEGER PRIMARY KEY,
    subject TEXT NOT NULL CHECK(length(subject) = 32),
    kind TEXT NOT NULL CHECK(kind IN ('handshake','peer_close','third_party','own_unknown_newer','lineage_branch')),
    log_ix INTEGER REFERENCES sync_logs(log_ix),
    detail TEXT NOT NULL CHECK(length(detail) <= 512),
    source_node TEXT CHECK(source_node IS NULL OR length(source_node) = 32),
    detected_at INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL,
    cleared_at INTEGER,
    cleared_by TEXT CHECK(cleared_by IS NULL OR length(cleared_by) <= 128),
    CHECK((kind IN ('own_unknown_newer','lineage_branch')) = (log_ix IS NOT NULL)),
    CHECK((cleared_at IS NULL) = (cleared_by IS NULL))
) STRICT;

CREATE UNIQUE INDEX sync_clone_evidence_open ON sync_clone_evidence(subject, kind, coalesce(log_ix, -1))
WHERE cleared_at IS NULL;

CREATE INDEX journal_forked_creates ON journal_forked(creates) WHERE creates IS NOT NULL;

CREATE TRIGGER sync_logs_freeze_lowers_only BEFORE UPDATE OF frozen_at_counter ON sync_logs
WHEN OLD.state = 'frozen' AND NEW.frozen_at_counter > OLD.frozen_at_counter
BEGIN SELECT RAISE(ABORT, 'sync_logs_freeze_lowers_only'); END;

CREATE TRIGGER sync_forks_append_only BEFORE DELETE ON sync_forks
BEGIN SELECT RAISE(ABORT, 'sync_forks_append_only'); END;

CREATE TRIGGER sync_clone_evidence_append_only BEFORE DELETE ON sync_clone_evidence
BEGIN SELECT RAISE(ABORT, 'sync_clone_evidence_append_only'); END;

-- Only last_seen_at (forward) on an open row and the one-time clear may change.
CREATE TRIGGER sync_clone_evidence_set_once BEFORE UPDATE ON sync_clone_evidence
WHEN OLD.cleared_at IS NOT NULL
  OR NEW.ev_ix IS NOT OLD.ev_ix OR NEW.subject IS NOT OLD.subject OR NEW.kind IS NOT OLD.kind
  OR NEW.log_ix IS NOT OLD.log_ix OR NEW.detail IS NOT OLD.detail
  OR NEW.source_node IS NOT OLD.source_node OR NEW.detected_at IS NOT OLD.detected_at
  OR NEW.last_seen_at < OLD.last_seen_at
BEGIN SELECT RAISE(ABORT, 'sync_clone_evidence_set_once'); END;

CREATE TRIGGER sync_reissues_append_only BEFORE DELETE ON sync_reissues
BEGIN SELECT RAISE(ABORT, 'sync_reissues_append_only'); END;

CREATE TRIGGER sync_reissues_immutable BEFORE UPDATE ON sync_reissues
BEGIN SELECT RAISE(ABORT, 'sync_reissues_immutable'); END;

CREATE TABLE snapshot_generations (
    id TEXT PRIMARY KEY,
    host_id INTEGER NOT NULL REFERENCES herdr_hosts(host_id),
    host_boot TEXT NOT NULL,
    epoch INTEGER NOT NULL CHECK(epoch > 0),
    observation_sequence INTEGER NOT NULL CHECK(observation_sequence > 0),
    incarnation TEXT NOT NULL CHECK(length(CAST(incarnation AS BLOB)) BETWEEN 1 AND 128),
    expected_targets INTEGER NOT NULL CHECK(expected_targets >= 0),
    staged_targets INTEGER NOT NULL DEFAULT 0 CHECK(staged_targets >= 0 AND staged_targets <= expected_targets),
    status TEXT NOT NULL CHECK(status IN ('building','sealed','published','discarded')),
    captured_active_id TEXT,
    captured_lifecycle_revision INTEGER NOT NULL CHECK(captured_lifecycle_revision >= 0),
    captured_invalidation_revision INTEGER NOT NULL CHECK(captured_invalidation_revision >= 0),
    captured_recovery_epoch INTEGER CHECK(captured_recovery_epoch >= 0),
    admission_sequence INTEGER NOT NULL DEFAULT 0 CHECK(admission_sequence >= 0),
    published_invalidation_revision INTEGER CHECK(published_invalidation_revision >= 0),
    created_at INTEGER NOT NULL,
    UNIQUE(host_id,host_boot,epoch,observation_sequence),
    CHECK(status NOT IN ('sealed','published') OR staged_targets=expected_targets)
) STRICT;

CREATE TABLE snapshot_targets (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    generation_id TEXT NOT NULL REFERENCES snapshot_generations(id),
    target_id TEXT NOT NULL,
    terminal_id TEXT,
    generation INTEGER NOT NULL CHECK(generation >= 0),
    observation_sequence INTEGER NOT NULL CHECK(observation_sequence > 0),
    connection_epoch INTEGER CHECK(connection_epoch IS NULL OR connection_epoch >= 0),
    incarnation_source_kind TEXT CHECK(incarnation_source_kind IN ('native_current_target','coherent_enumeration')),
    occupancy TEXT NOT NULL CHECK(occupancy IN ('empty_shell','occupied','unknown')),
    ui_state TEXT NOT NULL CHECK(ui_state IN ('idle','active_turn','approval_or_question','human_input','unknown')),
    observed_at INTEGER NOT NULL,
    verified_execution TEXT,
    top_level_occupant INTEGER NOT NULL DEFAULT 0 CHECK(top_level_occupant IN (0,1)),
    UNIQUE(generation_id,target_id),
    CHECK((connection_epoch IS NULL) = (incarnation_source_kind IS NULL))
) STRICT;

CREATE TABLE recovery_baseline_releases (
    host_id INTEGER NOT NULL REFERENCES herdr_hosts(host_id),
    baseline_generation_id TEXT NOT NULL REFERENCES snapshot_generations(id),
    target_id TEXT NOT NULL,
    decision_seq INTEGER NOT NULL CHECK(decision_seq > 0),
    PRIMARY KEY(host_id,baseline_generation_id,target_id)
) STRICT;

CREATE TABLE observed_targets (
    host_id INTEGER NOT NULL REFERENCES herdr_hosts(host_id),
    target_id TEXT NOT NULL,
    host_boot TEXT NOT NULL,
    epoch INTEGER NOT NULL CHECK(epoch >= 0),
    generation INTEGER NOT NULL CHECK(generation >= 0),
    observation_sequence INTEGER NOT NULL DEFAULT 0 CHECK(observation_sequence >= 0),
    connection_epoch INTEGER CHECK(connection_epoch IS NULL OR connection_epoch >= 0),
    incarnation TEXT,
    incarnation_source_kind TEXT CHECK(incarnation_source_kind IN ('native_current_target','coherent_enumeration')),
    observed_at INTEGER NOT NULL,
    provenance TEXT NOT NULL CHECK(provenance IN ('fresh', 'enumeration', 'cache')),
    suggested_seat_id TEXT,
    terminal_id TEXT,
    occupancy TEXT NOT NULL DEFAULT 'unknown' CHECK(occupancy IN ('empty_shell','occupied','unknown')),
    ui_state TEXT NOT NULL DEFAULT 'unknown' CHECK(ui_state IN ('idle','active_turn','approval_or_question','human_input','unknown')),
    verified_execution TEXT,
    top_level_occupant INTEGER NOT NULL DEFAULT 0 CHECK(top_level_occupant IN (0,1)),
    PRIMARY KEY(host_id, target_id),
    CHECK((incarnation IS NULL AND incarnation_source_kind IS NULL AND connection_epoch IS NULL)
       OR (incarnation IS NOT NULL AND incarnation_source_kind IS NOT NULL AND connection_epoch IS NOT NULL))
) STRICT;

CREATE TABLE recovery_baseline_targets (
    host_id INTEGER NOT NULL REFERENCES herdr_hosts(host_id),
    target_id TEXT NOT NULL,
    baseline_boot TEXT NOT NULL,
    baseline_epoch INTEGER NOT NULL CHECK(baseline_epoch >= 0),
    captured_at INTEGER NOT NULL,
    disposition TEXT NOT NULL CHECK(disposition IN (
        'unambiguous_unclaimed', 'created_after_baseline', 'held_for_repair', 'already_owned', 'unknown'
    )),
    PRIMARY KEY(host_id, target_id)
) STRICT;

CREATE TABLE recovery_holds (
    host_id INTEGER NOT NULL REFERENCES herdr_hosts(host_id),
    target_id TEXT NOT NULL,
    baseline_boot TEXT NOT NULL,
    baseline_epoch INTEGER NOT NULL CHECK(baseline_epoch >= 0),
    reason TEXT NOT NULL CHECK(length(reason) <= 512),
    released_at INTEGER,
    PRIMARY KEY(host_id, target_id)
) STRICT;

CREATE TABLE seats (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    instance_id TEXT NOT NULL REFERENCES local_node(id),
    state TEXT NOT NULL CHECK(state IN ('resolved', 'unresolved', 'retired', 'remote')),
    unresolved_reason TEXT CHECK(unresolved_reason IN ('host_invalidation','other')),
    unresolved_from_generation_id TEXT REFERENCES snapshot_generations(id),
    unresolved_prior_binding_generation INTEGER CHECK(unresolved_prior_binding_generation >= 0),
    role TEXT NOT NULL CHECK(role IN ('native', 'operator_fresh')),
    target_id TEXT,
    generation INTEGER NOT NULL CHECK(generation >= 0),
    target_generation INTEGER NOT NULL DEFAULT 0 CHECK(target_generation >= 0),
    structural_terminal_id TEXT,
    structural_incarnation TEXT,
    structural_incarnation_kind TEXT CHECK(structural_incarnation_kind IN ('native_current_target','coherent_enumeration')),
    structural_host_boot TEXT,
    structural_host_epoch INTEGER CHECK(structural_host_epoch >= 0),
    structural_connection_epoch INTEGER CHECK(structural_connection_epoch >= 0),
    structural_observation_sequence INTEGER CHECK(structural_observation_sequence > 0),
    created_at INTEGER NOT NULL,
    retired_at INTEGER,
    retired_seq INTEGER CHECK(retired_seq > 0),
    unavailability_episode INTEGER NOT NULL DEFAULT 1 CHECK(unavailability_episode > 0),
    unavailability_open INTEGER NOT NULL DEFAULT 1 CHECK(unavailability_open IN (0,1)), wake_on TEXT NOT NULL DEFAULT 'nudge' CHECK(wake_on IN ('nudge','all')),
    nudge_threshold INTEGER NOT NULL DEFAULT 0 CHECK(nudge_threshold BETWEEN -1 AND 9),
    home_node TEXT NOT NULL CHECK(length(home_node)=32 AND home_node NOT GLOB '*[^0-9a-f]*'),
    is_local INTEGER NOT NULL CHECK(is_local IN (0,1)),
    remote_kind TEXT CHECK(remote_kind IN ('agent','human')),
    host_id INTEGER REFERENCES herdr_hosts(host_id),
    published INTEGER NOT NULL DEFAULT 0 CHECK(published IN (0,1)),
    CHECK(is_local=1 OR published=1),
    CHECK((is_local=0) = (remote_kind IS NOT NULL)),
    CHECK((is_local=1) = (host_id IS NOT NULL)),
    CHECK(is_local=1 OR (target_id IS NULL AND structural_terminal_id IS NULL AND state IN ('remote','retired'))),
    CHECK(state<>'remote' OR is_local=0),
    CHECK((state = 'retired') = (retired_at IS NOT NULL)),
    CHECK(state = 'unresolved' OR unresolved_reason IS NULL),
    CHECK((structural_terminal_id IS NULL AND structural_incarnation IS NULL AND structural_incarnation_kind IS NULL AND structural_host_boot IS NULL AND structural_host_epoch IS NULL AND structural_connection_epoch IS NULL AND structural_observation_sequence IS NULL)
       OR (target_id IS NOT NULL AND structural_terminal_id IS NOT NULL AND structural_incarnation IS NOT NULL AND structural_incarnation_kind IS NOT NULL AND structural_host_boot IS NOT NULL AND structural_host_epoch IS NOT NULL AND structural_connection_epoch IS NOT NULL AND structural_observation_sequence IS NOT NULL)),
    CHECK(unresolved_reason IS 'host_invalidation' OR
          (unresolved_from_generation_id IS NULL AND unresolved_prior_binding_generation IS NULL))
) STRICT;

CREATE TABLE threads (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    instance_id TEXT NOT NULL REFERENCES local_node(id),
    topic TEXT NOT NULL CHECK(length(CAST(topic AS BLOB)) BETWEEN 1 AND 1024),
    goal TEXT NOT NULL CHECK(length(CAST(goal AS BLOB)) <= 1024),
    archived INTEGER NOT NULL DEFAULT 0 CHECK(archived IN (0, 1)),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    next_sequence INTEGER NOT NULL DEFAULT 1 CHECK(next_sequence > 0),
    topic_revision INTEGER NOT NULL DEFAULT 0 CHECK(topic_revision >= 0),
    directory_revision INTEGER NOT NULL DEFAULT 0 CHECK(directory_revision >= 0),
    membership_revision INTEGER NOT NULL DEFAULT 0 CHECK(membership_revision >= 0),
    timeline_revision INTEGER NOT NULL DEFAULT 0 CHECK(timeline_revision >= 0)
, managed_owner_author_id TEXT REFERENCES service_authors(id), name TEXT, last_activity INTEGER NOT NULL DEFAULT 0,
    journaled INTEGER NOT NULL CHECK(journaled IN (0,1))) STRICT;

CREATE TABLE memberships (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id TEXT NOT NULL REFERENCES threads(id),
    seat_id TEXT NOT NULL REFERENCES seats(id),
    episode INTEGER NOT NULL DEFAULT 1 CHECK(episode > 0),
    state TEXT NOT NULL CHECK(state IN ('invited', 'joined', 'left', 'retired')),
    joined_at INTEGER,
    left_at INTEGER,
    retired_at INTEGER, voluntary_state TEXT
    CHECK(voluntary_state IN ('absent','invited','joined','left','retired')),
    home_transition INTEGER NOT NULL DEFAULT 0,
    last_joined_n INTEGER NOT NULL DEFAULT 0,
    UNIQUE(thread_id, seat_id)
) STRICT;

CREATE TABLE membership_intervals (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id TEXT NOT NULL REFERENCES threads(id),
    seat_id TEXT NOT NULL REFERENCES seats(id),
    episode INTEGER NOT NULL CHECK(episode > 0),
    joined_seq INTEGER NOT NULL CHECK(joined_seq > 0),
    left_seq INTEGER CHECK(left_seq > joined_seq),
    UNIQUE(thread_id, seat_id, episode)
) STRICT;

CREATE TABLE seat_availability (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    seat_id TEXT NOT NULL REFERENCES seats(id),
    decision_seq INTEGER NOT NULL CHECK(decision_seq > 0),
    decision_at INTEGER NOT NULL,
    binding_generation INTEGER NOT NULL CHECK(binding_generation >= 0),
    observation_provenance TEXT NOT NULL CHECK(length(CAST(observation_provenance AS BLOB)) BETWEEN 1 AND 512),
    UNIQUE(seat_id, decision_seq)
) STRICT;

CREATE TABLE invitations (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    thread_id TEXT NOT NULL REFERENCES threads(id),
    seat_id TEXT NOT NULL REFERENCES seats(id),
    episode INTEGER NOT NULL CHECK(episode > 0),
    state TEXT NOT NULL CHECK(state IN ('pending', 'accepted', 'recipient_retired', 'already_joined')),
    created_decision_seq INTEGER NOT NULL CHECK(created_decision_seq > 0),
    created_at INTEGER NOT NULL,
    frozen_duration_ms INTEGER NOT NULL CHECK(frozen_duration_ms > 0),
    deadline_at INTEGER NOT NULL,
    accepted_at INTEGER,
    accepted_actor_seat_id TEXT REFERENCES seats(id),
    accepted_generation INTEGER CHECK(accepted_generation >= 0),
    accepted_observation TEXT,
    warning_message_id TEXT REFERENCES messages(id),
    retired_at INTEGER, reject_recorded INTEGER NOT NULL DEFAULT 0 CHECK(reject_recorded IN (0,1)), confirmation TEXT CHECK(confirmation IN ('lazy','require_ack')), inviter_seat_id TEXT REFERENCES seats(id), command_context_epoch INTEGER NOT NULL DEFAULT 0,
    origin_node TEXT NOT NULL CHECK(length(origin_node)=32 AND origin_node NOT GLOB '*[^0-9a-f]*'),
    invitee_n INTEGER NOT NULL DEFAULT 0 CHECK(invitee_n >= 0),
    inviter_kind TEXT NOT NULL DEFAULT 'seat' CHECK(inviter_kind IN ('seat','service','operator')),
    UNIQUE(thread_id, seat_id, episode),
    CHECK(state <> 'accepted' OR (accepted_at IS NOT NULL AND accepted_actor_seat_id IS NOT NULL AND accepted_generation IS NOT NULL AND accepted_observation IS NOT NULL))
) STRICT;

CREATE TABLE messages (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    instance_id TEXT NOT NULL REFERENCES local_node(id),
    thread_id TEXT NOT NULL REFERENCES threads(id),
    sequence INTEGER NOT NULL CHECK(sequence > 0),
    kind TEXT NOT NULL CHECK(kind IN ('ordinary', 'info', 'warn')),
    event_key TEXT UNIQUE,
    actor_seat_id TEXT REFERENCES seats(id),
    actor_label TEXT,
    native_observation TEXT,
    decision_seq INTEGER NOT NULL CHECK(decision_seq > 0),
    event_offset INTEGER NOT NULL DEFAULT 0 CHECK(event_offset >= 0),
    body TEXT CHECK(body IS NULL OR length(CAST(body AS BLOB)) <= 65536),
    event_json TEXT CHECK(event_json IS NULL OR length(CAST(event_json AS BLOB)) <= 4096),
    decision_at INTEGER NOT NULL,
    source_message_id TEXT REFERENCES messages(id),
    source_invitation_id TEXT REFERENCES invitations(id), author_kind TEXT
    CHECK(author_kind IN ('native', 'programmatic', 'built_in')), author_service_id TEXT REFERENCES service_authors(id), author_role TEXT
    CHECK(author_role IN ('human', 'agent', 'service')), relays_user INTEGER NOT NULL DEFAULT 0
    CHECK(relays_user IN (0, 1)), author_role_backfilled INTEGER NOT NULL DEFAULT 0
    CHECK(author_role_backfilled IN (0, 1)), user_intent TEXT
    CHECK (user_intent IN ('query', 'request', 'rule')), delivery_mode TEXT NOT NULL DEFAULT 'ordinary' CHECK(delivery_mode IN ('ordinary','lazy')), sender_binding_id TEXT,
    priority INTEGER NOT NULL DEFAULT 0 CHECK(priority BETWEEN 0 AND 9),
    origin_node TEXT CHECK(origin_node IS NULL OR length(origin_node)=32),
    UNIQUE(id, thread_id),
    UNIQUE(thread_id, sequence),
    CHECK((kind = 'ordinary' AND body IS NOT NULL AND event_key IS NULL) OR
          (kind IN ('info', 'warn') AND event_json IS NOT NULL AND body IS NULL))
) STRICT;

CREATE TABLE send_preparations (
    id TEXT PRIMARY KEY,
    instance_id TEXT NOT NULL REFERENCES local_node(id),
    operation_scope TEXT NOT NULL,
    operation_key TEXT NOT NULL,
    digest BLOB NOT NULL CHECK(length(digest)=32),
    thread_id TEXT NOT NULL REFERENCES threads(id),
    captured_membership_revision INTEGER NOT NULL CHECK(captured_membership_revision >= 0),
    captured_lifecycle_revision INTEGER NOT NULL CHECK(captured_lifecycle_revision >= 0),
    captured_eligibility_revision INTEGER NOT NULL CHECK(captured_eligibility_revision >= 0),
    captured_timeline_revision INTEGER NOT NULL CHECK(captured_timeline_revision >= 0),
    captured_config_revision INTEGER NOT NULL CHECK(captured_config_revision >= 0),
    interval_high_water INTEGER NOT NULL CHECK(interval_high_water >= 0),
    recipient_high_water INTEGER NOT NULL CHECK(recipient_high_water >= 0),
    recipient_cursor INTEGER NOT NULL DEFAULT 0 CHECK(recipient_cursor >= 0),
    recipient_count INTEGER NOT NULL DEFAULT 0 CHECK(recipient_count >= 0),
    warning_count INTEGER NOT NULL DEFAULT 0 CHECK(warning_count >= 0),
    earliest_lease_deadline INTEGER,
    status TEXT NOT NULL CHECK(status IN ('building','sealed','discarded')), prepared_at INTEGER, delivery_mode TEXT NOT NULL DEFAULT 'ordinary' CHECK(delivery_mode IN ('ordinary','lazy')),
    priority INTEGER NOT NULL DEFAULT 0 CHECK(priority BETWEEN 0 AND 9),
    origin TEXT NOT NULL DEFAULT 'local' CHECK(origin IN ('local','replicated')),
    UNIQUE(instance_id, operation_scope, operation_key)
) STRICT;

CREATE TABLE prepared_recipients (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    preparation_id TEXT NOT NULL REFERENCES send_preparations(id),
    thread_id TEXT NOT NULL REFERENCES threads(id),
    seat_id TEXT NOT NULL REFERENCES seats(id),
    receipt_ordinal INTEGER NOT NULL CHECK(receipt_ordinal > 0),
    frozen_duration_ms INTEGER NOT NULL CHECK(frozen_duration_ms > 0),
    eligible_at_snapshot INTEGER NOT NULL CHECK(eligible_at_snapshot IN (0,1)),
    availability_provenance TEXT, ack_required INTEGER NOT NULL DEFAULT 1 CHECK(ack_required IN (0,1)),
    UNIQUE(preparation_id, seat_id),
    UNIQUE(preparation_id, receipt_ordinal)
) STRICT;

CREATE TABLE prepared_unavailable_warnings (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    preparation_id TEXT NOT NULL REFERENCES send_preparations(id),
    warning_key TEXT NOT NULL,
    warning_id TEXT NOT NULL,
    affected_seat_id TEXT NOT NULL REFERENCES seats(id),
    unavailability_episode INTEGER NOT NULL CHECK(unavailability_episode > 0),
    warning_offset INTEGER NOT NULL CHECK(warning_offset > 0),
    event_json TEXT NOT NULL CHECK(length(CAST(event_json AS BLOB)) <= 4096),
    UNIQUE(preparation_id, warning_key),
    UNIQUE(preparation_id, warning_offset),
    UNIQUE(preparation_id, warning_id)
) STRICT;

CREATE TABLE send_manifests (
    preparation_id TEXT PRIMARY KEY REFERENCES send_preparations(id),
    message_id TEXT NOT NULL UNIQUE REFERENCES messages(id),
    instance_id TEXT NOT NULL REFERENCES local_node(id),
    thread_id TEXT NOT NULL REFERENCES threads(id),
    decision_seq INTEGER NOT NULL CHECK(decision_seq > 0),
    decision_at INTEGER NOT NULL,
    base_sequence INTEGER NOT NULL CHECK(base_sequence > 0),
    interval_high_water INTEGER NOT NULL CHECK(interval_high_water >= 0),
    recipient_count INTEGER NOT NULL CHECK(recipient_count >= 0),
    warning_count INTEGER NOT NULL CHECK(warning_count >= 0),
    suppressed_seats TEXT
) STRICT;

CREATE TABLE operations (
    actor_scope TEXT NOT NULL,
    operation_key TEXT NOT NULL,
    digest BLOB NOT NULL CHECK(length(digest) = 32),
    result_json TEXT NOT NULL,
    decided_at INTEGER NOT NULL,
    PRIMARY KEY(actor_scope, operation_key)
) STRICT;

CREATE TABLE wake_work (
    seat_id TEXT PRIMARY KEY REFERENCES seats(id),
    reason_bits INTEGER NOT NULL DEFAULT 0 CHECK(reason_bits >= 0),
    attention_version INTEGER NOT NULL DEFAULT 0 CHECK(attention_version >= 0),
    checkpoint_version INTEGER NOT NULL DEFAULT 0 CHECK(checkpoint_version >= 0),
    binding_generation INTEGER,
    retry_step INTEGER NOT NULL DEFAULT 0 CHECK(retry_step >= 0),
    reservation_id TEXT,
    reservation_boot TEXT,
    reserved_at_utc INTEGER,
    completed_at_utc INTEGER,
    minimum_delay_ms INTEGER NOT NULL DEFAULT 0 CHECK(minimum_delay_ms >= 0),
    effective_delay_ms INTEGER NOT NULL DEFAULT 0 CHECK(effective_delay_ms >= 0),
    last_reservation_id TEXT,
    last_reservation_boot TEXT,
    last_reserved_at_utc INTEGER,
    last_invitation_seq INTEGER CHECK(last_invitation_seq IS NULL OR last_invitation_seq > 0),
    last_invitation_offset INTEGER CHECK(last_invitation_offset IS NULL OR last_invitation_offset >= 0),
    last_receipt_seq INTEGER CHECK(last_receipt_seq IS NULL OR last_receipt_seq > 0),
    last_receipt_offset INTEGER CHECK(last_receipt_offset IS NULL OR last_receipt_offset >= 0),
    last_warning_seq INTEGER CHECK(last_warning_seq IS NULL OR last_warning_seq > 0),
    last_warning_offset INTEGER CHECK(last_warning_offset IS NULL OR last_warning_offset >= 0),
    last_outcome TEXT CHECK(last_outcome IS NULL OR length(last_outcome) <= 128), last_lazy_ordinal INTEGER CHECK(last_lazy_ordinal IS NULL OR last_lazy_ordinal>0),
    CHECK((last_invitation_seq IS NULL) = (last_invitation_offset IS NULL)),
    CHECK((last_receipt_seq IS NULL) = (last_receipt_offset IS NULL)),
    CHECK((last_warning_seq IS NULL) = (last_warning_offset IS NULL))
) STRICT;

CREATE TABLE warning_jobs (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    warning_id TEXT NOT NULL UNIQUE,
    event_seq INTEGER NOT NULL CHECK(event_seq > 0),
    thread_id TEXT NOT NULL REFERENCES threads(id),
    interval_high_water INTEGER NOT NULL CHECK(interval_high_water >= 0),
    affected_seat_id TEXT REFERENCES seats(id),
    condition_kind TEXT NOT NULL CHECK(condition_kind IN ('invitation','receipt','unavailable')),
    condition_id TEXT NOT NULL,
    cursor_ordinal INTEGER NOT NULL DEFAULT 0 CHECK(cursor_ordinal >= 0),
    phase TEXT NOT NULL DEFAULT 'intervals' CHECK(phase IN ('intervals','affected','finalize','complete')),
    processed_units INTEGER NOT NULL DEFAULT 0 CHECK(processed_units >= 0),
    status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','complete','failed')),
    last_error TEXT CHECK(last_error IS NULL OR length(CAST(last_error AS BLOB)) <= 512)
, recipient_cutoff_seq INTEGER CHECK(recipient_cutoff_seq IS NULL OR recipient_cutoff_seq >= 0)) STRICT;

CREATE TABLE warning_recipients (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    warning_id TEXT NOT NULL,
    seat_id TEXT NOT NULL REFERENCES seats(id),
    generation INTEGER NOT NULL CHECK(generation > 0),
    UNIQUE(warning_id, seat_id)
) STRICT;

CREATE TABLE retirements (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    seat_id TEXT NOT NULL UNIQUE REFERENCES seats(id),
    cutover_at INTEGER NOT NULL,
    closure_boot TEXT,
    closure_epoch INTEGER CHECK(closure_epoch >= 0),
    closure_target TEXT,
    closure_generation INTEGER CHECK(closure_generation >= 0),
    origin TEXT NOT NULL DEFAULT 'local' CHECK(origin IN ('local','replicated')),
    thread_ordinal INTEGER NOT NULL DEFAULT 0 CHECK(thread_ordinal >= 0),
    phase TEXT NOT NULL DEFAULT 'select_thread' CHECK(phase IN ('select_thread', 'invitations', 'receipts', 'logical_receipts', 'audit', 'complete')),
    obligation_ordinal INTEGER NOT NULL DEFAULT 0 CHECK(obligation_ordinal >= 0),
    high_water_ordinal INTEGER NOT NULL DEFAULT 0 CHECK(high_water_ordinal >= 0),
    processed_units INTEGER NOT NULL DEFAULT 0 CHECK(processed_units >= 0),
    warnings_added INTEGER NOT NULL DEFAULT 0 CHECK(warnings_added >= 0),
    obligations_retired INTEGER NOT NULL DEFAULT 0 CHECK(obligations_retired >= 0),
    audits_added INTEGER NOT NULL DEFAULT 0 CHECK(audits_added >= 0),
    status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending', 'complete')),
    last_error TEXT CHECK(last_error IS NULL OR length(CAST(last_error AS BLOB)) <= 512),
    CHECK((origin='local') = (closure_boot IS NOT NULL))
) STRICT;

CREATE TABLE retirement_audits (
    job_id TEXT NOT NULL REFERENCES retirements(id),
    thread_id TEXT NOT NULL REFERENCES threads(id),
    message_id TEXT NOT NULL UNIQUE REFERENCES messages(id),
    PRIMARY KEY(job_id, thread_id)
) STRICT;

CREATE TABLE filter_revisions (
    instance_id TEXT NOT NULL REFERENCES local_node(id),
    scope_kind TEXT NOT NULL CHECK(scope_kind IN ('directory', 'inbox', 'topic')),
    scope_key TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 0 CHECK(revision >= 0),
    PRIMARY KEY(instance_id, scope_kind, scope_key)
) STRICT;

CREATE TABLE delivery_observations (
    seat_id TEXT NOT NULL REFERENCES seats(id),
    thread_id TEXT NOT NULL REFERENCES threads(id),
    attempted INTEGER NOT NULL DEFAULT 0 CHECK(attempted >= 0),
    submitted INTEGER NOT NULL DEFAULT 0 CHECK(submitted >= 0),
    read_count INTEGER NOT NULL DEFAULT 0 CHECK(read_count >= 0),
    acked INTEGER NOT NULL DEFAULT 0 CHECK(acked >= 0),
    PRIMARY KEY(seat_id, thread_id)
) STRICT;

CREATE TABLE service_authors (
    id TEXT PRIMARY KEY CHECK(length(CAST(id AS BLOB)) BETWEEN 1 AND 128),
    instance_id TEXT NOT NULL UNIQUE REFERENCES local_node(id),
    reserved_name TEXT NOT NULL DEFAULT 'herdr-graph' CHECK(reserved_name = 'herdr-graph'),
    created_at INTEGER NOT NULL
) STRICT;

CREATE TABLE requirement_episodes (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE CHECK(length(CAST(id AS BLOB)) BETWEEN 1 AND 128),
    thread_id TEXT NOT NULL REFERENCES threads(id),
    seat_id TEXT NOT NULL REFERENCES seats(id),
    issuer_author_id TEXT NOT NULL REFERENCES service_authors(id),
    invitation_id TEXT NOT NULL UNIQUE REFERENCES invitations(id),
    revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0),
    state TEXT NOT NULL CHECK(state IN ('pending', 'accepted', 'released', 'retired')),
    created_decision_seq INTEGER NOT NULL CHECK(created_decision_seq > 0),
    created_at INTEGER NOT NULL,
    accepted_by_seat_id TEXT REFERENCES seats(id),
    accepted_generation INTEGER CHECK(accepted_generation >= 0),
    accepted_observation TEXT CHECK(accepted_observation IS NULL OR length(CAST(accepted_observation AS BLOB)) BETWEEN 1 AND 512),
    accepted_at INTEGER,
    released_at INTEGER,
    retired_at INTEGER,
    CHECK(state <> 'accepted' OR (
        accepted_by_seat_id IS NOT NULL AND accepted_by_seat_id = seat_id
        AND accepted_generation IS NOT NULL
        AND accepted_observation IS NOT NULL AND accepted_at IS NOT NULL
    )),
    CHECK(state <> 'pending' OR (
        accepted_by_seat_id IS NULL AND accepted_generation IS NULL
        AND accepted_observation IS NULL AND accepted_at IS NULL
        AND released_at IS NULL AND retired_at IS NULL
    )),
    CHECK((accepted_at IS NULL AND accepted_by_seat_id IS NULL
        AND accepted_generation IS NULL AND accepted_observation IS NULL)
        OR (accepted_at IS NOT NULL AND accepted_by_seat_id IS NOT NULL
        AND accepted_by_seat_id = seat_id
        AND accepted_generation IS NOT NULL AND accepted_observation IS NOT NULL)),
    CHECK(state <> 'released' OR released_at IS NOT NULL),
    CHECK(state <> 'retired' OR retired_at IS NOT NULL)
) STRICT;

CREATE TABLE invitation_cancellations (
    invitation_id TEXT PRIMARY KEY REFERENCES invitations(id),
    requirement_id TEXT NOT NULL UNIQUE REFERENCES requirement_episodes(id),
    cancelled_at INTEGER NOT NULL
) STRICT;

CREATE TABLE service_notification_preparations (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    instance_id TEXT NOT NULL REFERENCES local_node(id),
    author_id TEXT NOT NULL REFERENCES service_authors(id),
    operation_key TEXT NOT NULL,
    digest BLOB NOT NULL CHECK(length(digest)=32),
    thread_id TEXT NOT NULL REFERENCES threads(id),
    membership_revision INTEGER NOT NULL CHECK(membership_revision>=0),
    lifecycle_revision INTEGER NOT NULL CHECK(lifecycle_revision>=0),
    interval_high_water INTEGER NOT NULL CHECK(interval_high_water>=0),
    requirement_high_water INTEGER NOT NULL CHECK(requirement_high_water>=0),
    interval_cursor INTEGER NOT NULL DEFAULT 0 CHECK(interval_cursor>=0),
    requirement_cursor INTEGER NOT NULL DEFAULT 0 CHECK(requirement_cursor>=0),
    recipient_count INTEGER NOT NULL DEFAULT 0 CHECK(recipient_count>=0),
    status TEXT NOT NULL CHECK(status IN ('building','sealed','discarded','published'))
) STRICT;

CREATE TABLE service_notification_recipients (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    preparation_id TEXT NOT NULL REFERENCES service_notification_preparations(id),
    seat_id TEXT NOT NULL REFERENCES seats(id),
    UNIQUE(preparation_id,seat_id)
) STRICT;

CREATE TABLE service_notification_publications (
    preparation_id TEXT PRIMARY KEY REFERENCES service_notification_preparations(id),
    message_id TEXT NOT NULL UNIQUE REFERENCES messages(id),
    decision_seq INTEGER NOT NULL CHECK(decision_seq>0),
    recipient_count INTEGER NOT NULL CHECK(recipient_count>=0)
) STRICT;

CREATE TABLE digest_open_warnings (
    source TEXT NOT NULL CHECK(source IN ('job','prepared')),
    source_ordinal INTEGER NOT NULL,
    warning_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    affected_seat_id TEXT,
    condition_kind TEXT NOT NULL CHECK(condition_kind IN ('invitation','receipt','unavailable')),
    condition_id TEXT NOT NULL,
    episode INTEGER,
    PRIMARY KEY(source, source_ordinal)
) STRICT;

CREATE TABLE digest_pending_invitations (
    seat_id TEXT NOT NULL,
    invitation_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    created_decision_seq INTEGER NOT NULL,
    ordinal INTEGER NOT NULL,
    PRIMARY KEY(seat_id, invitation_id)
) STRICT;

CREATE TABLE digest_pending_manifest_receipts (
    seat_id TEXT NOT NULL,
    preparation_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL,
    decision_seq INTEGER,
    PRIMARY KEY(seat_id, preparation_id)
) STRICT;

CREATE TABLE digest_open_warning_recipients (
    seat_id TEXT NOT NULL,
    warning_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    source TEXT NOT NULL,
    source_ordinal INTEGER NOT NULL,
    PRIMARY KEY(seat_id, warning_id)
) STRICT;

CREATE TABLE digest_programmatic_warnings (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    seat_id TEXT NOT NULL,
    warning_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    event_seq INTEGER NOT NULL,
    event_offset INTEGER NOT NULL,
    UNIQUE(seat_id, warning_id)
) STRICT;

CREATE TABLE "allocation_decisions" (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    host_id INTEGER NOT NULL REFERENCES herdr_hosts(host_id),
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

CREATE TABLE summary_blocks (
    id TEXT PRIMARY KEY,
    instance_id TEXT NOT NULL REFERENCES local_node(id),
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

CREATE TABLE summary_transitions (
    block_id TEXT NOT NULL REFERENCES summary_blocks(id),
    ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
    thread_id TEXT NOT NULL REFERENCES threads(id),
    chunking_version TEXT NOT NULL,
    target_id TEXT NOT NULL,
    new_status TEXT NOT NULL CHECK(new_status IN ('done', 'resolved', 'superseded')),
    cite_seq INTEGER NOT NULL CHECK(cite_seq > 0), rule_change TEXT
    CHECK (rule_change IN ('withdrawn', 'replaced')),
    PRIMARY KEY(block_id, ordinal)
) STRICT;

CREATE TABLE summary_jobs (
    id TEXT PRIMARY KEY,
    instance_id TEXT NOT NULL REFERENCES local_node(id),
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
    created_at INTEGER NOT NULL, fetched_bundle_json TEXT,
    UNIQUE(thread_id, chunking_version, level, idx),
    CHECK((lease_token IS NULL) = (lease_seat_id IS NULL))
) STRICT;

CREATE TABLE summary_job_durations (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    instance_id TEXT NOT NULL REFERENCES local_node(id),
    job_id TEXT NOT NULL REFERENCES summary_jobs(id),
    duration_ms INTEGER NOT NULL CHECK(duration_ms >= 0),
    recorded_at INTEGER NOT NULL
) STRICT;

CREATE TABLE human_receipt_waivers (
    seat_id TEXT PRIMARY KEY REFERENCES seats(id),
    through_decision_seq INTEGER NOT NULL CHECK(through_decision_seq >= 0),
    human_generation INTEGER NOT NULL CHECK(human_generation >= 0),
    decided_at INTEGER NOT NULL
) STRICT;

CREATE TABLE human_receipt_reconciliation_bounds (
    seat_id TEXT PRIMARY KEY REFERENCES seats(id),
    prepared_high_water INTEGER NOT NULL CHECK(prepared_high_water >= 0),
    physical_high_water INTEGER NOT NULL CHECK(physical_high_water >= 0),
    decision_seq INTEGER NOT NULL CHECK(decision_seq >= 0)
) STRICT;

CREATE TABLE wake_batches (
    seat_id TEXT PRIMARY KEY REFERENCES seats(id),
    deadline_at INTEGER NOT NULL
) STRICT;

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
    cleared_seq INTEGER, invitation_originator_instance_id TEXT REFERENCES local_node(id), invitation_originator_seat_id TEXT REFERENCES seats(id)
    CHECK((invitation_originator_instance_id IS NULL) = (invitation_originator_seat_id IS NULL)),
    CHECK((clear_warning_id IS NULL) = (cleared_seq IS NULL))
) STRICT;

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

CREATE TABLE invitation_rejections (
    invitation_id TEXT PRIMARY KEY REFERENCES invitations(id),
    reason TEXT NOT NULL CHECK(length(CAST(reason AS BLOB)) BETWEEN 1 AND 4096),
    rejected_at INTEGER NOT NULL,
    actor_seat_id TEXT NOT NULL REFERENCES seats(id),
    generation INTEGER NOT NULL CHECK(generation >= 0),
    observation TEXT NOT NULL
) STRICT;

CREATE TABLE archival_instances (
    instance_id TEXT PRIMARY KEY REFERENCES local_node(id),
    runtime_boot TEXT NOT NULL,
    evidence_epoch INTEGER NOT NULL DEFAULT 0,
    mutation_revision INTEGER NOT NULL DEFAULT 0,
    policy_ms INTEGER NOT NULL DEFAULT 3600000 CHECK(policy_ms>=0),
    last_mono INTEGER,
    last_utc INTEGER,
    bootstrap_veto INTEGER NOT NULL DEFAULT 1 CHECK(bootstrap_veto IN (0,1)),
    bootstrap_source TEXT,
    seed_cursor INTEGER NOT NULL DEFAULT 0,
    seat_seed_cursor INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE TABLE archival_hosts (
    host_id INTEGER PRIMARY KEY REFERENCES herdr_hosts(host_id),
    evidence_epoch INTEGER NOT NULL DEFAULT 0,
    host_generation INTEGER NOT NULL DEFAULT 0,
    coherent_since INTEGER,
    validating_snapshot TEXT,
    snapshot_cursor TEXT NOT NULL DEFAULT '',
    snapshot_checked INTEGER NOT NULL DEFAULT 0 CHECK(snapshot_checked IN (0,1))
) STRICT;

CREATE TABLE channel_archival (
    thread_id TEXT PRIMARY KEY REFERENCES threads(id),
    instance_id TEXT NOT NULL REFERENCES local_node(id),
    activity_revision INTEGER NOT NULL DEFAULT 1,
    quiet_mono INTEGER,
    quiet_utc INTEGER,
    runtime_boot TEXT,
    evidence_epoch INTEGER,
    due_mono INTEGER NOT NULL DEFAULT 0,
    scan_revision INTEGER,
    scan_phase INTEGER NOT NULL DEFAULT 0,
    scan_cursor INTEGER NOT NULL DEFAULT 0,
    scan_key TEXT NOT NULL DEFAULT '',
    scan_expiry INTEGER,
    receipt_physical_after INTEGER NOT NULL DEFAULT 0,
    receipt_manifest_after INTEGER NOT NULL DEFAULT 0,
    receipt_physical_high INTEGER,
    receipt_manifest_high INTEGER,
    receipt_next_manifest INTEGER NOT NULL DEFAULT 0,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0,1))
) STRICT;

CREATE TABLE seat_archival (
    seat_id TEXT PRIMARY KEY REFERENCES seats(id),
    instance_id TEXT NOT NULL REFERENCES local_node(id),
    host_id INTEGER NOT NULL REFERENCES herdr_hosts(host_id),
    host_evidence_epoch INTEGER NOT NULL DEFAULT 0,
    activity_revision INTEGER NOT NULL DEFAULT 0,
    runtime_boot TEXT,
    evidence_epoch INTEGER,
    idle_mono INTEGER,
    idle_utc INTEGER,
    last_mono INTEGER,
    samples INTEGER NOT NULL DEFAULT 0,
    next_mono INTEGER,
    binding_generation INTEGER,
    target_generation INTEGER,
    target_id TEXT,
    host_boot TEXT,
    host_epoch INTEGER,
    terminal TEXT,
    incarnation TEXT,
    harness TEXT,
    native_session TEXT,
    execution TEXT,
    observation_sequence INTEGER,
    connection_epoch INTEGER
) STRICT;

CREATE TABLE channel_handoff_fences (
    instance_id TEXT NOT NULL REFERENCES local_node(id),
    actor_scope TEXT NOT NULL,
    compound TEXT NOT NULL,
    digest TEXT NOT NULL CHECK(length(digest)=64),
    claim_json TEXT NOT NULL,
    recipient TEXT NOT NULL,
    create_key TEXT NOT NULL,
    invite_key TEXT NOT NULL,
    send_key TEXT NOT NULL,
    original_thread TEXT,
    thread_id TEXT REFERENCES threads(id),
    origin TEXT NOT NULL CHECK(origin IN ('cooperative_pending_claim','legacy_local_journal_hint')),
    state TEXT NOT NULL CHECK(state IN ('live','completed')),
    created_at INTEGER NOT NULL,
    completed_at INTEGER,
    source_identity TEXT,
    PRIMARY KEY(instance_id,actor_scope,compound),
    UNIQUE(instance_id,actor_scope,create_key),
    UNIQUE(instance_id,actor_scope,invite_key),
    UNIQUE(instance_id,actor_scope,send_key),
    CHECK((state='completed')=(completed_at IS NOT NULL))
) STRICT;

CREATE TABLE lazy_recipients (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    preparation_id TEXT NOT NULL REFERENCES send_preparations(id),
    message_id TEXT NOT NULL CHECK(length(CAST(message_id AS BLOB)) BETWEEN 1 AND 128),
    thread_id TEXT NOT NULL REFERENCES threads(id),
    seat_id TEXT NOT NULL REFERENCES seats(id),
    state TEXT NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','displayed')), legacy_pending INTEGER NOT NULL DEFAULT 0 CHECK(legacy_pending IN (0,1)),
    priority INTEGER NOT NULL DEFAULT 1 CHECK(priority BETWEEN 0 AND 9),
    decision_seq INTEGER CHECK(decision_seq IS NULL OR decision_seq>0),
    UNIQUE(preparation_id,seat_id),
    UNIQUE(message_id,seat_id),
    CHECK(message_id IS CASE
        WHEN substr(preparation_id,1,5)='prep-' AND length(preparation_id)>5 THEN 'msg-'||substr(preparation_id,6)
        WHEN length(preparation_id)=12 AND substr(preparation_id,1,1)='p' AND substr(preparation_id,2) NOT GLOB '*[^A-Za-z0-9]*' THEN 'm'||substr(preparation_id,2)
        ELSE NULL END)
) STRICT;

CREATE TABLE "harness_version_evidence"(
  harness TEXT NOT NULL CHECK(length(CAST(harness AS BLOB)) BETWEEN 1 AND 64 AND instr(harness,char(0))=0 AND substr(harness,1,1) GLOB '[a-z]' AND harness NOT GLOB '*[^a-z0-9_-]*' AND harness<>'human'),
  version TEXT NOT NULL,
  contract_id TEXT NOT NULL,
  first_seen_at INTEGER NOT NULL,
  lifecycle_ok_at INTEGER,
  tool_ok_at INTEGER,
  violation_at INTEGER,
  violation_event TEXT,
  violation_field TEXT,
  last_seen_at INTEGER NOT NULL,
  PRIMARY KEY(harness, version, contract_id)
) WITHOUT ROWID;

CREATE TABLE "harness_unattributed"(
  harness TEXT PRIMARY KEY CHECK(length(CAST(harness AS BLOB)) BETWEEN 1 AND 64 AND instr(harness,char(0))=0 AND substr(harness,1,1) GLOB '[a-z]' AND harness NOT GLOB '*[^a-z0-9_-]*' AND harness<>'human'),
  reason TEXT NOT NULL,
  at INTEGER NOT NULL
) WITHOUT ROWID;

CREATE TABLE harness_runtime_identities (
    harness TEXT NOT NULL CHECK(length(CAST(harness AS BLOB)) BETWEEN 1 AND 64 AND instr(harness,char(0))=0 AND substr(harness,1,1) GLOB '[a-z]' AND harness NOT GLOB '*[^a-z0-9_-]*' AND harness<>'human'),
    identity_key TEXT NOT NULL CHECK(instr(identity_key,char(0))=0 AND length(CAST(identity_key AS BLOB)) BETWEEN 1 AND 136 AND (identity_key GLOB 'release:[0-9]*.[0-9]*.[0-9]*' OR (substr(identity_key,1,6)='build:' AND length(identity_key)=70 AND substr(identity_key,7) NOT GLOB '*[^0-9a-f]*'))),
    descriptor_json TEXT NOT NULL CHECK(length(CAST(descriptor_json AS BLOB)) BETWEEN 1 AND 4096 AND json_valid(descriptor_json) AND json_type(descriptor_json)='object'),
    last_seen_at INTEGER NOT NULL,
    PRIMARY KEY(harness, identity_key)
) STRICT, WITHOUT ROWID;

CREATE TABLE harness_contract_evidence_v2 (
    harness TEXT NOT NULL,
    identity_key TEXT NOT NULL,
    domain TEXT NOT NULL CHECK(instr(domain,char(0))=0 AND length(domain) BETWEEN 1 AND 32 AND substr(domain,1,1) GLOB '[a-z]' AND domain NOT GLOB '*[^a-z0-9_]*'),
    origin TEXT NOT NULL CHECK(origin IN ('native_payload','native_shape_observation','bridge_envelope')),
    contract_id TEXT NOT NULL CHECK(instr(contract_id,char(0))=0 AND length(contract_id)=16 AND contract_id NOT GLOB '*[^0-9a-f]*'),
    first_seen_at INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL,
    milestones_json TEXT NOT NULL CHECK(length(CAST(milestones_json AS BLOB)) BETWEEN 1 AND 4096 AND json_valid(milestones_json) AND json_type(milestones_json)='object'),
    violation_at INTEGER,
    violation_event TEXT CHECK(violation_event IS NULL OR (instr(violation_event,char(0))=0 AND length(CAST(violation_event AS BLOB)) BETWEEN 1 AND 63 AND substr(violation_event,1,1) GLOB '[A-Za-z]' AND violation_event NOT GLOB '*[^A-Za-z0-9_]*')),
    violation_field TEXT CHECK(violation_field IS NULL OR length(CAST(violation_field AS BLOB)) BETWEEN 1 AND 256),
    CHECK((violation_at IS NULL AND violation_event IS NULL AND violation_field IS NULL) OR (violation_at IS NOT NULL AND violation_event IS NOT NULL AND violation_field IS NOT NULL)),
    PRIMARY KEY(harness, identity_key, domain, origin, contract_id),
    FOREIGN KEY(harness, identity_key) REFERENCES harness_runtime_identities(harness, identity_key)
) STRICT, WITHOUT ROWID;

CREATE TABLE harness_unattributed_v2 (
    harness TEXT NOT NULL CHECK(length(CAST(harness AS BLOB)) BETWEEN 1 AND 64 AND instr(harness,char(0))=0 AND substr(harness,1,1) GLOB '[a-z]' AND harness NOT GLOB '*[^a-z0-9_-]*' AND harness<>'human'),
    domain TEXT NOT NULL CHECK(instr(domain,char(0))=0 AND length(domain) BETWEEN 1 AND 32 AND substr(domain,1,1) GLOB '[a-z]' AND domain NOT GLOB '*[^a-z0-9_]*'),
    origin TEXT NOT NULL CHECK(origin IN ('native_payload','native_shape_observation','bridge_envelope')),
    reason TEXT NOT NULL CHECK(length(CAST(reason AS BLOB)) BETWEEN 1 AND 256),
    at INTEGER NOT NULL,
    PRIMARY KEY(harness, domain, origin)
) STRICT, WITHOUT ROWID;

CREATE TABLE "harness_contract_diagnostics" (
    harness TEXT NOT NULL CHECK (length(harness) BETWEEN 1 AND 64 AND harness GLOB '[a-z]*' AND harness NOT GLOB '*[^a-z0-9_-]*' AND harness <> 'human'),
    session_id TEXT NOT NULL CHECK (length(CAST(session_id AS BLOB)) BETWEEN 1 AND 256),
    contract_id TEXT NOT NULL CHECK (length(contract_id) = 16 AND contract_id NOT GLOB '*[^0-9a-f]*'),
    event TEXT NOT NULL CHECK (length(event) BETWEEN 1 AND 63 AND event NOT GLOB '*[^a-zA-Z0-9_]*'),
    field TEXT NOT NULL CHECK (length(CAST(field AS BLOB)) BETWEEN 1 AND 128),
    first_seen_at INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL,
    PRIMARY KEY (harness, session_id, contract_id)
) WITHOUT ROWID;

CREATE TABLE bootstrap_child_keys (
    parent_id INTEGER NOT NULL,
    instance_id TEXT NOT NULL,
    state_dir TEXT NOT NULL,
    host_endpoint TEXT NOT NULL,
    actor_scope TEXT NOT NULL,
    operation_key TEXT NOT NULL,
    role TEXT NOT NULL CHECK(role IN ('compound','begin','create','invite','send','complete','handoff','resolve','attach','linked_complete','reserve','record','check','not_submitted','recovery','cancel')),
    attempt INTEGER NOT NULL CHECK(attempt BETWEEN 0 AND 4294967295),
    PRIMARY KEY(instance_id,state_dir,host_endpoint,actor_scope,operation_key),
    UNIQUE(parent_id,role,attempt),
    FOREIGN KEY(parent_id,instance_id,state_dir,host_endpoint,actor_scope) REFERENCES bootstrap_handoffs(id,instance_id,state_dir,host_endpoint,actor_scope),
    CHECK((role IN ('reserve','record','check','not_submitted','recovery','cancel'))=(attempt>0))
) STRICT;

CREATE TABLE bootstrap_attempts (
    parent_id INTEGER NOT NULL REFERENCES bootstrap_handoffs(id),
    attempt INTEGER NOT NULL CHECK(attempt BETWEEN 1 AND 4294967295),
    state TEXT NOT NULL CHECK(state IN ('prepared','possible_creation','not_submitted','outcome_unknown','created')),
    reserve_key TEXT NOT NULL,
    record_key TEXT NOT NULL,
    check_key TEXT NOT NULL,
    not_submitted_key TEXT NOT NULL,
    reserved_administrative_revision INTEGER CHECK(reserved_administrative_revision>=0),
    creation_json BLOB CHECK(length(creation_json)<=131072),
    PRIMARY KEY(parent_id,attempt),
    CHECK((state='created')=(creation_json IS NOT NULL)),
    CHECK(reserve_key!=record_key AND reserve_key!=check_key AND record_key!=check_key
      AND not_submitted_key!=reserve_key AND not_submitted_key!=record_key AND not_submitted_key!=check_key)
) STRICT;

CREATE TABLE bootstrap_attachments (
    parent_id INTEGER PRIMARY KEY REFERENCES bootstrap_handoffs(id),
    attempt INTEGER NOT NULL,
    attachment_json BLOB NOT NULL CHECK(length(attachment_json)<=131072),
    FOREIGN KEY(parent_id,attempt) REFERENCES bootstrap_attempts(parent_id,attempt)
) STRICT;

CREATE TABLE bootstrap_reports (
    parent_id INTEGER PRIMARY KEY REFERENCES bootstrap_attachments(parent_id),
    completed_json BLOB NOT NULL CHECK(length(completed_json)<=2097152)
) STRICT;

CREATE TABLE direct_threads (
    instance_id TEXT NOT NULL REFERENCES local_node(id),
    seat_a TEXT NOT NULL REFERENCES seats(id),
    seat_b TEXT NOT NULL REFERENCES seats(id),
    thread_id TEXT NOT NULL UNIQUE REFERENCES threads(id),
    created_hlc INTEGER NOT NULL,
    created_node TEXT NOT NULL CHECK(length(created_node)=32 AND created_node NOT GLOB '*[^0-9a-f]*'),
    CHECK(seat_a < seat_b),
    UNIQUE(instance_id, seat_a, seat_b)
) STRICT;

-- Local attribution of a command's own fact (ht-5fr.4.15): staged by the command, taken by the
-- materializer in the same transaction. Never replicated, never keyed by an EntityKey.
CREATE TABLE fact_attribution_staging (
    staging_key TEXT PRIMARY KEY,
    binding_id TEXT,
    binding_generation INTEGER NOT NULL CHECK(binding_generation >= 0),
    observation TEXT NOT NULL CHECK(length(CAST(observation AS BLOB)) <= 4096)
) STRICT;

CREATE TABLE automatic_handoff_keys (
    instance_id TEXT NOT NULL,
    actor_scope TEXT NOT NULL,
    compound TEXT NOT NULL,
    operation_key TEXT NOT NULL CHECK(length(operation_key) BETWEEN 1 AND 256),
    role TEXT NOT NULL CHECK(role IN ('compound','begin','ensure','send','complete')),
    PRIMARY KEY(instance_id,actor_scope,operation_key),
    UNIQUE(instance_id,actor_scope,compound,role),
    FOREIGN KEY(instance_id,actor_scope,compound) REFERENCES automatic_handoff_companions(instance_id,actor_scope,compound)
) STRICT;

CREATE TABLE automatic_bootstrap_keys (
    instance_id TEXT NOT NULL,
    actor_scope TEXT NOT NULL,
    operation_key TEXT NOT NULL CHECK(length(operation_key) BETWEEN 1 AND 256),
    parent_id INTEGER NOT NULL REFERENCES automatic_bootstrap_parents(id),
    role TEXT NOT NULL CHECK(role IN ('compound','begin','resolve','attach','complete','child','child_begin','ensure','send','child_complete','reserve','record','check','not_submitted','unknown','recovery','cancellation')),
    attempt INTEGER NOT NULL CHECK(attempt>=0),
    PRIMARY KEY(instance_id,actor_scope,operation_key),
    UNIQUE(parent_id,role,attempt)
) STRICT;

CREATE TABLE automatic_bootstrap_attempts (
    parent_id INTEGER NOT NULL REFERENCES automatic_bootstrap_parents(id),
    attempt INTEGER NOT NULL CHECK(attempt>0),
    state TEXT NOT NULL CHECK(state IN ('prepared','possible_creation','outcome_unknown','not_submitted','created')),
    reserved_revision INTEGER CHECK(reserved_revision>=0),
    creation_json BLOB CHECK(length(creation_json)<=131072),
    PRIMARY KEY(parent_id,attempt),
    CHECK((state='created')=(creation_json IS NOT NULL))
) STRICT;

CREATE TABLE automatic_bootstrap_reports (
    parent_id INTEGER PRIMARY KEY REFERENCES automatic_bootstrap_parents(id),
    report_json BLOB NOT NULL CHECK(length(report_json)<=2097152)
) STRICT;

CREATE TABLE "occupant_bindings" (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    seat_id TEXT NOT NULL REFERENCES seats(id),
    generation INTEGER NOT NULL CHECK(generation >= 0),
    target_generation INTEGER NOT NULL DEFAULT 0 CHECK(target_generation >= 0),
    target_id TEXT NOT NULL,
    terminal_id TEXT,
    incarnation TEXT,
    host_boot TEXT NOT NULL,
    host_epoch INTEGER NOT NULL CHECK(host_epoch >= 0),
    harness TEXT NOT NULL CHECK(length(CAST(harness AS BLOB)) BETWEEN 1 AND 64 AND instr(harness,char(0))=0 AND substr(harness,1,1) GLOB '[a-z]' AND harness NOT GLOB '*[^a-z0-9_-]*'),
    native_session TEXT NOT NULL,
    execution_id TEXT NOT NULL,
    observation_provenance TEXT NOT NULL,
    observed_at INTEGER NOT NULL,
    registered_at INTEGER,
    ended_at INTEGER,
    binding_id TEXT NOT NULL UNIQUE DEFAULT (lower(hex(randomblob(16)))) CHECK(length(CAST(binding_id AS BLOB)) BETWEEN 1 AND 128 AND binding_id NOT GLOB '*[^!-~]*'),
    role TEXT NOT NULL DEFAULT 'top_level' CHECK(role IN ('top_level','subagent')),
    parent_binding TEXT,
    agent_id TEXT NOT NULL DEFAULT '' CHECK(length(CAST(agent_id AS BLOB)) <= 128 AND agent_id NOT GLOB '*[^!-~]*'),
    lineage_from TEXT,
    liveness TEXT NOT NULL DEFAULT 'connected' CHECK(liveness IN ('connected','disconnected')),
    disconnected_at INTEGER,
    disconnect_reason TEXT CHECK(disconnect_reason IS NULL OR (length(disconnect_reason) BETWEEN 1 AND 64 AND disconnect_reason NOT GLOB '*[^a-z_]*')),
    liveness_pid INTEGER CHECK(liveness_pid IS NULL OR liveness_pid > 0),
    liveness_start INTEGER,
    migrated INTEGER NOT NULL DEFAULT 0 CHECK(migrated IN (0,1)),
    migrated_session TEXT,
    CHECK(role='subagent' OR parent_binding IS NULL),
    CHECK((liveness='connected') = (disconnected_at IS NULL AND disconnect_reason IS NULL)),
    CHECK((liveness_pid IS NULL) = (liveness_start IS NULL)),
    CHECK(migrated=1 OR migrated_session IS NULL)
) STRICT;

CREATE TABLE binding_cursors (
    binding_id TEXT NOT NULL REFERENCES occupant_bindings(binding_id),
    thread_id TEXT NOT NULL REFERENCES threads(id),
    seq INTEGER NOT NULL CHECK(seq >= 0),
    PRIMARY KEY(binding_id, thread_id)
) STRICT, WITHOUT ROWID;

CREATE TABLE "catch_up" (
    binding_id TEXT NOT NULL REFERENCES occupant_bindings(binding_id),
    seat_id TEXT NOT NULL REFERENCES seats(id),
    thread_id TEXT NOT NULL REFERENCES threads(id),
    frontier_seq INTEGER NOT NULL CHECK(frontier_seq >= 0),
    entered_at INTEGER NOT NULL,
    extension_until INTEGER,
    last_progress_at INTEGER,
    state TEXT NOT NULL CHECK(state IN ('active', 'ended')),
    end_reason TEXT CHECK(end_reason IN ('ready', 'stalled', 'superseded')),
    ended_at INTEGER,
    release_seq INTEGER CHECK(release_seq IS NULL OR release_seq > 0),
    PRIMARY KEY(binding_id, thread_id),
    CHECK((state = 'active') = (end_reason IS NULL)),
    CHECK((end_reason IS NULL) = (ended_at IS NULL))
) STRICT;

CREATE TABLE "warning_offer" (
    binding_id TEXT PRIMARY KEY REFERENCES occupant_bindings(binding_id),
    seat_id TEXT NOT NULL REFERENCES seats(id),
    offered_through_seq INTEGER NOT NULL DEFAULT 0 CHECK(offered_through_seq >= 0)
) STRICT;

CREATE TABLE "digest_notice_offer" (
    binding_id TEXT PRIMARY KEY REFERENCES occupant_bindings(binding_id),
    seat_id TEXT NOT NULL REFERENCES seats(id),
    offered_ordinal INTEGER NOT NULL CHECK(offered_ordinal >= 0)
) STRICT;

CREATE TABLE in_turn_offer (
    binding_id TEXT NOT NULL REFERENCES occupant_bindings(binding_id),
    seat_id TEXT NOT NULL REFERENCES seats(id),
    item_id TEXT NOT NULL,
    PRIMARY KEY (binding_id, item_id)
) STRICT;

CREATE TABLE receipts (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    message_id TEXT NOT NULL,
    thread_id TEXT NOT NULL REFERENCES threads(id),
    seat_id TEXT NOT NULL REFERENCES seats(id),
    state TEXT NOT NULL CHECK(state IN ('pending', 'acked', 'recipient_retired')),
    frozen_duration_ms INTEGER NOT NULL CHECK(frozen_duration_ms > 0),
    available_at INTEGER,
    deadline_at INTEGER,
    ack_actor_seat_id TEXT REFERENCES seats(id),
    ack_generation INTEGER,
    ack_observation TEXT,
    acked_at INTEGER,
    warning_message_id TEXT REFERENCES messages(id),
    retired_at INTEGER, soft_poked_at INTEGER, ack_required INTEGER NOT NULL DEFAULT 1 CHECK(ack_required IN (0,1)),
    ack_binding_id TEXT,
    UNIQUE(message_id, seat_id),
    FOREIGN KEY(message_id, thread_id) REFERENCES messages(id, thread_id)
) STRICT;

CREATE TABLE receipt_state (
    message_id TEXT NOT NULL,
    seat_id TEXT NOT NULL REFERENCES seats(id),
    state TEXT NOT NULL CHECK(state IN ('pending','acked','recipient_retired')),
    available_at INTEGER,
    deadline_at INTEGER,
    ack_actor_seat_id TEXT REFERENCES seats(id),
    ack_generation INTEGER,
    ack_observation TEXT,
    acked_at INTEGER,
    retired_at INTEGER,
    warning_message_id TEXT, soft_poked_at INTEGER, ack_required INTEGER NOT NULL DEFAULT 1 CHECK(ack_required IN (0,1)),
    ack_binding_id TEXT,
    settle_batch TEXT CHECK(settle_batch IS NULL OR length(settle_batch) <= 64),
    PRIMARY KEY(message_id, seat_id)
) STRICT;

CREATE TABLE receipt_send_time_cutovers (instance_id TEXT PRIMARY KEY, historical_decision_seq INTEGER NOT NULL CHECK(historical_decision_seq>=0)) STRICT;

CREATE TABLE bootstrap_handoffs (
    id INTEGER PRIMARY KEY,
    instance_id TEXT NOT NULL REFERENCES local_node(id),
    state_dir TEXT NOT NULL,
    host_endpoint TEXT NOT NULL,
    actor_scope TEXT NOT NULL,
    compound TEXT NOT NULL,
    digest TEXT NOT NULL CHECK(length(digest)=64),
    identity_json BLOB NOT NULL CHECK(length(identity_json)<=524288),
    create_key TEXT NOT NULL,
    original_thread TEXT REFERENCES threads(id),
    thread_id TEXT REFERENCES threads(id),
    state TEXT NOT NULL CHECK(state IN ('prepared','possible_creation','created','attached','completed','cancelled')),
    current_attempt INTEGER NOT NULL CHECK(current_attempt BETWEEN 1 AND 4294967295),
    administrative_revision INTEGER NOT NULL DEFAULT 0 CHECK(administrative_revision>=0),
    latest_recovery_operation TEXT,
    created_at INTEGER NOT NULL,
    terminal_at INTEGER,
    UNIQUE(instance_id,state_dir,host_endpoint,actor_scope,compound),
    UNIQUE(id,instance_id,state_dir,host_endpoint,actor_scope),
    FOREIGN KEY(id,current_attempt) REFERENCES bootstrap_attempts(parent_id,attempt) DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(id,latest_recovery_operation) REFERENCES bootstrap_recovery_decisions(parent_id,operation) DEFERRABLE INITIALLY DEFERRED,
    CHECK((state IN ('completed','cancelled'))=(terminal_at IS NOT NULL)),
    CHECK(original_thread IS NULL OR thread_id IS original_thread)
) STRICT;

CREATE TABLE bootstrap_recovery_decisions (
    parent_id INTEGER NOT NULL REFERENCES bootstrap_handoffs(id),
    attempt INTEGER NOT NULL,
    operation TEXT NOT NULL,
    decision_kind TEXT NOT NULL DEFAULT 'recovery' CHECK(decision_kind IN ('recovery','cancellation')),
    result_json BLOB NOT NULL CHECK(length(result_json)<=1048576),
    PRIMARY KEY(parent_id,operation),
    UNIQUE(parent_id,attempt,decision_kind),
    FOREIGN KEY(parent_id,attempt) REFERENCES bootstrap_attempts(parent_id,attempt)
) STRICT;

CREATE TABLE receipt_deadline_backfill (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    physical_after INTEGER NOT NULL CHECK(physical_after>=0),
    physical_high_water INTEGER NOT NULL CHECK(physical_high_water>=physical_after),
    prepared_after INTEGER NOT NULL CHECK(prepared_after>=0),
    prepared_high_water INTEGER NOT NULL CHECK(prepared_high_water>=prepared_after),
    completed INTEGER NOT NULL CHECK(completed IN (0,1)),
    CHECK(completed=0 OR (physical_after=physical_high_water AND prepared_after=prepared_high_water))
) STRICT;

CREATE TABLE receipt_warning_origins (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    instance_id TEXT NOT NULL REFERENCES local_node(id),
    seat_id TEXT NOT NULL REFERENCES seats(id),
    warning_id TEXT NOT NULL REFERENCES messages(id),
    source_message_id TEXT NOT NULL REFERENCES messages(id),
    UNIQUE(warning_id,seat_id)
) STRICT;

CREATE TABLE receipt_due_sparse (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    message_id TEXT NOT NULL,
    seat_id TEXT NOT NULL REFERENCES seats(id),
    deadline_at INTEGER NOT NULL,
    UNIQUE(message_id,seat_id)
) STRICT;

CREATE TABLE receipt_discovery_backfill (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    physical_after INTEGER NOT NULL CHECK(physical_after>=0),
    physical_high_water INTEGER NOT NULL CHECK(physical_high_water>=physical_after),
    sparse_after INTEGER NOT NULL CHECK(sparse_after>=0),
    sparse_high_water INTEGER NOT NULL CHECK(sparse_high_water>=sparse_after),
    completed INTEGER NOT NULL CHECK(completed IN (0,1)),
    CHECK(completed=0 OR (physical_after=physical_high_water AND sparse_after=sparse_high_water))
) STRICT;

CREATE TABLE automatic_handoff_companions (
    instance_id TEXT NOT NULL,
    actor_scope TEXT NOT NULL,
    compound TEXT NOT NULL,
    state_dir TEXT NOT NULL,
    host_endpoint TEXT NOT NULL,
    original_json BLOB NOT NULL CHECK(length(original_json)<=524288),
    begin_key TEXT NOT NULL,
    ensure_key TEXT NOT NULL,
    send_key TEXT NOT NULL,
    complete_key TEXT NOT NULL,
    PRIMARY KEY(instance_id,actor_scope,compound),
    FOREIGN KEY(instance_id,actor_scope,compound) REFERENCES channel_handoff_fences(instance_id,actor_scope,compound),
    CHECK(begin_key!=ensure_key AND begin_key!=send_key AND begin_key!=complete_key AND ensure_key!=send_key AND ensure_key!=complete_key AND send_key!=complete_key)
) STRICT;

CREATE TABLE automatic_bootstrap_parents (
    id INTEGER PRIMARY KEY,
    instance_id TEXT NOT NULL REFERENCES local_node(id),
    actor_scope TEXT NOT NULL,
    compound TEXT NOT NULL,
    state_dir TEXT NOT NULL,
    host_endpoint TEXT NOT NULL,
    original_json BLOB NOT NULL CHECK(length(original_json)<=524288),
    state TEXT NOT NULL CHECK(state IN ('prepared','possible_creation','created','attached','completed','cancelled')),
    current_attempt INTEGER NOT NULL CHECK(current_attempt>0),
    administrative_revision INTEGER NOT NULL DEFAULT 0 CHECK(administrative_revision>=0),
    thread_id TEXT REFERENCES threads(id),
    UNIQUE(instance_id,actor_scope,compound)
) STRICT;

CREATE TABLE automatic_bootstrap_attachments (
    parent_id INTEGER PRIMARY KEY REFERENCES automatic_bootstrap_parents(id),
    attempt INTEGER NOT NULL,
    attachment_json BLOB NOT NULL CHECK(length(attachment_json)<=524288),
    FOREIGN KEY(parent_id,attempt) REFERENCES automatic_bootstrap_attempts(parent_id,attempt)
) STRICT;

CREATE TABLE automatic_bootstrap_recoveries (
 parent_id INTEGER NOT NULL REFERENCES automatic_bootstrap_parents(id),
 attempt INTEGER NOT NULL,
 decision_kind TEXT NOT NULL CHECK(decision_kind IN ('recovery','cancellation')),
 operation TEXT NOT NULL CHECK(length(operation) BETWEEN 1 AND 256),
 revision INTEGER NOT NULL CHECK(revision>0),
 result_json BLOB NOT NULL CHECK(length(result_json)<=2097152),
 PRIMARY KEY(parent_id,attempt,decision_kind),
 UNIQUE(parent_id,operation), UNIQUE(parent_id,revision),
 FOREIGN KEY(parent_id,attempt) REFERENCES automatic_bootstrap_attempts(parent_id,attempt)
) STRICT;

CREATE TABLE invitation_context_latest (
    thread_id TEXT PRIMARY KEY REFERENCES threads(id),
    instance_id TEXT NOT NULL REFERENCES local_node(id),
    last_change_seq INTEGER NOT NULL CHECK(last_change_seq > 0),
    context_revision INTEGER NOT NULL CHECK(context_revision > 0)
) STRICT;

CREATE TABLE invitation_presentation_consumers (
    binding_id TEXT NOT NULL REFERENCES occupant_bindings(binding_id),
    seat_id TEXT NOT NULL REFERENCES seats(id),
    consumer TEXT NOT NULL CHECK(consumer IN ('hook','watch')),
    instance_id TEXT NOT NULL REFERENCES local_node(id),
    generation INTEGER NOT NULL CHECK(generation>=0),
    execution TEXT NOT NULL CHECK(length(CAST(execution AS BLOB)) BETWEEN 1 AND 128),
    render_key TEXT NOT NULL CHECK(length(render_key) IN (0,64)),
    initial_after INTEGER NOT NULL CHECK(initial_after>=0),
    initial_high INTEGER NOT NULL CHECK(initial_high>=0),
    context_after_seq INTEGER NOT NULL CHECK(context_after_seq>=0),
    context_after_thread TEXT NOT NULL,
    context_high INTEGER NOT NULL CHECK(context_high>=0),
    context_thread TEXT NOT NULL,
    context_selected_seq INTEGER NOT NULL CHECK(context_selected_seq>=0),
    context_invitation_after INTEGER NOT NULL CHECK(context_invitation_after>=0),
    context_invitation_high INTEGER NOT NULL CHECK(context_invitation_high>=0),
    revisit_after_bytes INTEGER NOT NULL CHECK(revisit_after_bytes>=0),
    revisit_after_ordinal INTEGER NOT NULL CHECK(revisit_after_ordinal>=0),
    revisit_high_bytes INTEGER NOT NULL CHECK(revisit_high_bytes>=0),
    revisit_high_ordinal INTEGER NOT NULL CHECK(revisit_high_ordinal>=0),
    cleanup_after INTEGER NOT NULL CHECK(cleanup_after>=0),
    dirty_after INTEGER NOT NULL CHECK(dirty_after>=0),
    dirty_high INTEGER NOT NULL CHECK(dirty_high>=0),
    rotation INTEGER NOT NULL CHECK(rotation BETWEEN 0 AND 4),
    mod_generation INTEGER NOT NULL CHECK(mod_generation>=0),
    mod_execution TEXT NOT NULL CHECK(length(CAST(mod_execution AS BLOB))<=128),
    mod_initial_after INTEGER NOT NULL CHECK(mod_initial_after>=0),
    mod_initial_high INTEGER NOT NULL CHECK(mod_initial_high>=0),
    mod_context_after_seq INTEGER NOT NULL CHECK(mod_context_after_seq>=0),
    mod_context_high INTEGER NOT NULL CHECK(mod_context_high>=0),
    mod_context_selected_seq INTEGER NOT NULL CHECK(mod_context_selected_seq>=0),
    mod_context_invitation_after INTEGER NOT NULL CHECK(mod_context_invitation_after>=0),
    mod_context_invitation_high INTEGER NOT NULL CHECK(mod_context_invitation_high>=0),
    mod_cleanup_after INTEGER NOT NULL CHECK(mod_cleanup_after>=0),
    mod_cleanup_high INTEGER NOT NULL CHECK(mod_cleanup_high>=0),
    mod_version INTEGER NOT NULL CHECK(mod_version>=0),
    mod_context_after_thread TEXT NOT NULL,
    mod_context_thread TEXT NOT NULL,
    ordinary_version TEXT NOT NULL CHECK(length(ordinary_version)<=20),
    ordinary_done INTEGER NOT NULL CHECK(ordinary_done IN (0,1)),
    ordinary_cursor TEXT CHECK(length(CAST(ordinary_cursor AS BLOB))<=8192),
    ordinary_pending TEXT NOT NULL CHECK(length(CAST(ordinary_pending AS BLOB))<=155731 AND json_valid(ordinary_pending) AND json_type(ordinary_pending)='object' AND json_type(ordinary_pending,'$.items')='array' AND json_array_length(ordinary_pending,'$.items')<=32 AND json_type(ordinary_pending,'$.prefix')='array' AND json_array_length(ordinary_pending,'$.prefix')<=8),
    ordinary_skip_body TEXT CHECK(length(CAST(ordinary_skip_body AS BLOB))<=128),
    ordinary_partial TEXT NOT NULL CHECK(length(CAST(ordinary_partial AS BLOB))<=49415 AND json_valid(ordinary_partial) AND json_type(ordinary_partial)='object'),
    outbox_retired_mask INTEGER NOT NULL CHECK(outbox_retired_mask BETWEEN 0 AND 4294967295),
    outbox_confirmed_mask INTEGER NOT NULL CHECK(outbox_confirmed_mask BETWEEN 0 AND 4294967295 AND (outbox_confirmed_mask & outbox_retired_mask)=outbox_confirmed_mask),
    last_descriptors TEXT NOT NULL CHECK(length(CAST(last_descriptors AS BLOB))<=4096 AND json_valid(last_descriptors) AND json_type(last_descriptors)='array' AND json_array_length(last_descriptors)<=32),
    last_confirmed_mask INTEGER NOT NULL CHECK(last_confirmed_mask BETWEEN 0 AND 4294967295),
    control_state TEXT NOT NULL CHECK(length(CAST(control_state AS BLOB))<=4096 AND json_valid(control_state) AND json_type(control_state)='object'),
    outbox_selection TEXT NOT NULL CHECK(length(CAST(outbox_selection AS BLOB))<=128),
    outbox_codec_key TEXT NOT NULL CHECK(length(outbox_codec_key)<=64),
    outbox_render_key TEXT NOT NULL CHECK(length(outbox_render_key)<=64),
    outbox_prefix TEXT NOT NULL CHECK(length(CAST(outbox_prefix AS BLOB))<=24601 AND json_valid(outbox_prefix) AND json_type(outbox_prefix)='array' AND json_array_length(outbox_prefix)<=8),
    outbox_token TEXT CHECK(outbox_token IS NULL OR (length(outbox_token)=32 AND outbox_token NOT GLOB '*[^0-9a-f]*')),
    last_token TEXT CHECK(last_token IS NULL OR (length(last_token)=32 AND last_token NOT GLOB '*[^0-9a-f]*')),
    outbox_text TEXT NOT NULL CHECK(length(CAST(outbox_text AS BLOB))<=65536),
    outbox_descriptors TEXT NOT NULL CHECK(length(CAST(outbox_descriptors AS BLOB))<=262144 AND json_valid(outbox_descriptors) AND json_type(outbox_descriptors)='array' AND json_array_length(outbox_descriptors)<=32),
    outbox_version TEXT NOT NULL CHECK(length(outbox_version)<=20),
    outbox_more INTEGER NOT NULL CHECK(outbox_more IN (0,1)), ordinary_publication_after INTEGER NOT NULL DEFAULT 0 CHECK(ordinary_publication_after>=0), ordinary_publication_high INTEGER NOT NULL DEFAULT 0 CHECK(ordinary_publication_high>=0), ordinary_passive_generation INTEGER NOT NULL DEFAULT 0 CHECK(ordinary_passive_generation>=0), ordinary_scan_passive_generation INTEGER NOT NULL DEFAULT 0 CHECK(ordinary_scan_passive_generation>=0),
    PRIMARY KEY(binding_id,consumer)
) STRICT;

CREATE TABLE invitation_presentation_items (
    binding_id TEXT NOT NULL REFERENCES occupant_bindings(binding_id),
    seat_id TEXT NOT NULL REFERENCES seats(id),
    invitation_id TEXT NOT NULL REFERENCES invitations(id),
    consumer TEXT NOT NULL CHECK(consumer IN ('hook','watch')),
    generation INTEGER NOT NULL CHECK(generation>=0),
    execution TEXT NOT NULL CHECK(length(CAST(execution AS BLOB)) BETWEEN 1 AND 128),
    context_key TEXT NOT NULL CHECK(length(CAST(context_key AS BLOB))<=4096),
    render_key TEXT NOT NULL CHECK(length(render_key) IN (0,64)),
    output_class INTEGER NOT NULL CHECK(output_class BETWEEN 2 AND 4),
    partial_count INTEGER NOT NULL CHECK(partial_count IN (0,1)),
    complete_count INTEGER NOT NULL CHECK(complete_count IN (0,1)),
    required_render_bytes INTEGER NOT NULL CHECK(required_render_bytes>=0),
    revisit_ordinal INTEGER NOT NULL CHECK(revisit_ordinal>0),
    mod_generation INTEGER NOT NULL DEFAULT 0 CHECK(mod_generation>=0),
    mod_execution TEXT NOT NULL DEFAULT '' CHECK(length(CAST(mod_execution AS BLOB))<=128),
    mod_key TEXT NOT NULL CHECK(length(CAST(mod_key AS BLOB))<=4096),
    PRIMARY KEY(binding_id,invitation_id,consumer)
) STRICT;

CREATE TABLE presentation_confirming_invitations (
    seat_id TEXT NOT NULL,
    invitation_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK(ordinal>0),
    PRIMARY KEY(seat_id,invitation_id)
) STRICT;

CREATE TABLE presentation_warning_proofs (
    binding_id TEXT NOT NULL REFERENCES occupant_bindings(binding_id) ON DELETE CASCADE,
    seat_id TEXT NOT NULL REFERENCES seats(id) ON DELETE CASCADE,
    consumer TEXT NOT NULL CHECK(consumer IN ('hook','watch')),
    proof_json TEXT NOT NULL CHECK(length(CAST(proof_json AS BLOB))<=32768 AND json_valid(proof_json)),
    PRIMARY KEY(binding_id,consumer)
) STRICT;

CREATE TABLE presentation_warning_fence (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    version INTEGER NOT NULL CHECK(version>=0)
) STRICT;

CREATE INDEX snapshot_generations_status ON snapshot_generations(host_id,status,id);

CREATE INDEX snapshot_generations_captured_active ON snapshot_generations(captured_active_id,status);

CREATE UNIQUE INDEX snapshot_targets_terminal ON snapshot_targets(generation_id,terminal_id) WHERE terminal_id IS NOT NULL;

CREATE INDEX snapshot_targets_generation_ordinal ON snapshot_targets(generation_id,ordinal);

CREATE UNIQUE INDEX seats_live_target ON seats(host_id, target_id) WHERE state = 'resolved' AND target_id IS NOT NULL;

CREATE INDEX seats_home_node ON seats(home_node, state);

CREATE INDEX seats_instance_ordinal ON seats(instance_id, ordinal);

CREATE INDEX seats_instance_state_ordinal ON seats(instance_id,state,ordinal);

CREATE INDEX seats_unresolved_generation ON seats(unresolved_from_generation_id) WHERE unresolved_from_generation_id IS NOT NULL;

CREATE INDEX threads_instance_ordinal ON threads(instance_id, ordinal);

CREATE INDEX threads_instance_topic_ordinal ON threads(instance_id, topic, ordinal);

CREATE INDEX memberships_seat_ordinal ON memberships(seat_id, ordinal);

CREATE INDEX memberships_thread_ordinal ON memberships(thread_id, ordinal);

CREATE INDEX memberships_thread_state ON memberships(thread_id, state, ordinal);

CREATE INDEX membership_intervals_thread_ordinal ON membership_intervals(thread_id, ordinal);

CREATE INDEX membership_intervals_snapshot ON membership_intervals(thread_id, seat_id, joined_seq);

CREATE UNIQUE INDEX membership_intervals_open ON membership_intervals(thread_id, seat_id) WHERE left_seq IS NULL;

CREATE INDEX seat_availability_after ON seat_availability(seat_id, decision_seq);

CREATE INDEX invitations_due ON invitations(deadline_at, ordinal) WHERE state = 'pending';

CREATE INDEX invitations_due_origin ON invitations(origin_node, deadline_at, ordinal) WHERE state = 'pending';

CREATE INDEX invitations_seat_thread_pending ON invitations(seat_id, thread_id, ordinal) WHERE state = 'pending';

CREATE INDEX invitations_thread_seat_episode ON invitations(thread_id, seat_id, episode);

CREATE INDEX invitations_seat_decision ON invitations(seat_id, created_decision_seq, ordinal);

CREATE INDEX invitations_pending_unwarned ON invitations(deadline_at, ordinal) WHERE state='pending' AND warning_message_id IS NULL;

CREATE INDEX messages_thread_sequence ON messages(thread_id, sequence);

CREATE INDEX messages_kind_ordinal ON messages(kind, ordinal);

CREATE INDEX messages_origin_ordinary ON messages(origin_node) WHERE kind='ordinary';

CREATE UNIQUE INDEX messages_instance_logical ON messages(instance_id, decision_seq, event_offset);

CREATE INDEX messages_instance_ordinary_logical ON messages(instance_id, decision_seq, event_offset) WHERE kind='ordinary';

CREATE INDEX messages_instance_warning_logical ON messages(instance_id, decision_seq, event_offset) WHERE kind='warn';

CREATE INDEX send_preparations_status ON send_preparations(status, id);

CREATE INDEX prepared_recipients_seat_ordinal ON prepared_recipients(seat_id, ordinal);

CREATE INDEX prepared_recipients_seat_thread_ordinal ON prepared_recipients(seat_id, thread_id, ordinal);

CREATE INDEX prepared_recipients_preparation_ordinal ON prepared_recipients(preparation_id, ordinal);

CREATE INDEX prepared_recipients_thread_ordinal ON prepared_recipients(thread_id, ordinal);

CREATE INDEX prepared_unavailable_warnings_key ON prepared_unavailable_warnings(warning_key, warning_id);

CREATE INDEX prepared_unavailable_warnings_ordinal ON prepared_unavailable_warnings(ordinal);

CREATE INDEX send_manifests_thread_sequence ON send_manifests(thread_id, base_sequence);

CREATE INDEX send_manifests_decision ON send_manifests(decision_seq, message_id);

CREATE UNIQUE INDEX send_manifests_instance_decision ON send_manifests(instance_id, decision_seq);

CREATE INDEX send_manifests_warning_decision ON send_manifests(instance_id, decision_seq) WHERE warning_count>0;

CREATE UNIQUE INDEX wake_work_active_attempt ON wake_work(reservation_id) WHERE reservation_id IS NOT NULL;

CREATE INDEX warning_jobs_ready ON warning_jobs(status, ordinal);

CREATE INDEX warning_recipients_seat_generation ON warning_recipients(seat_id, generation);

CREATE INDEX warning_recipients_warning_ordinal ON warning_recipients(warning_id, ordinal);

CREATE INDEX retirements_status_ordinal ON retirements(status, ordinal);

CREATE INDEX retirements_progress ON retirements(status, thread_ordinal, ordinal);

CREATE INDEX threads_managed_owner ON threads(managed_owner_author_id, ordinal)
    WHERE managed_owner_author_id IS NOT NULL;

CREATE INDEX messages_author_service ON messages(author_service_id, ordinal)
    WHERE author_service_id IS NOT NULL;

CREATE UNIQUE INDEX requirement_episodes_effective
    ON requirement_episodes(thread_id, seat_id)
    WHERE state IN ('pending', 'accepted');

CREATE INDEX requirement_episodes_seat_pending
    ON requirement_episodes(seat_id, ordinal)
    WHERE state = 'pending';

CREATE INDEX requirement_episodes_thread_seat
    ON requirement_episodes(thread_id, seat_id, ordinal);

CREATE INDEX requirement_episodes_thread_ordinal
    ON requirement_episodes(thread_id,ordinal);

CREATE INDEX service_notification_preparations_key
    ON service_notification_preparations(instance_id,author_id,operation_key,ordinal);

CREATE INDEX service_notification_recipients_page
    ON service_notification_recipients(preparation_id,ordinal);

CREATE INDEX retirements_failed_pending ON retirements(ordinal)
WHERE status='pending' AND last_error IS NOT NULL;

CREATE INDEX service_notification_recipients_seat ON service_notification_recipients(seat_id);

CREATE INDEX warning_jobs_affected_seat ON warning_jobs(affected_seat_id);

CREATE INDEX prepared_unavailable_warnings_affected_seat ON prepared_unavailable_warnings(affected_seat_id);

CREATE INDEX prepared_unavailable_warnings_warning ON prepared_unavailable_warnings(warning_id);

CREATE INDEX membership_intervals_seat_thread ON membership_intervals(seat_id, thread_id);

CREATE INDEX messages_thread_warning ON messages(thread_id, sequence) WHERE kind='warn';

CREATE INDEX send_manifests_thread_warning ON send_manifests(thread_id, base_sequence) WHERE warning_count>0;

CREATE INDEX digest_open_warnings_affected ON digest_open_warnings(affected_seat_id, source, source_ordinal);

CREATE INDEX digest_open_warnings_affected_thread ON digest_open_warnings(affected_seat_id, thread_id, source, source_ordinal);

CREATE INDEX digest_open_warnings_warning ON digest_open_warnings(warning_id);

CREATE INDEX digest_open_warnings_condition ON digest_open_warnings(condition_kind, condition_id);

CREATE INDEX digest_pending_invitations_seat ON digest_pending_invitations(seat_id, created_decision_seq, ordinal);

CREATE INDEX digest_pending_invitations_thread ON digest_pending_invitations(seat_id, thread_id, created_decision_seq, ordinal);

CREATE INDEX digest_pending_manifest_receipts_thread ON digest_pending_manifest_receipts(thread_id, ordinal);

CREATE INDEX digest_pending_manifest_receipts_seat ON digest_pending_manifest_receipts(seat_id, decision_seq, ordinal);

CREATE INDEX digest_pending_manifest_receipts_seat_thread ON digest_pending_manifest_receipts(seat_id, thread_id, decision_seq, ordinal);

CREATE INDEX digest_open_warning_recipients_seat ON digest_open_warning_recipients(seat_id, source, source_ordinal);

CREATE INDEX digest_open_warning_recipients_thread ON digest_open_warning_recipients(seat_id, thread_id, source, source_ordinal);

CREATE INDEX digest_open_warning_recipients_warning ON digest_open_warning_recipients(warning_id);

CREATE INDEX digest_programmatic_warnings_seat ON digest_programmatic_warnings(seat_id, ordinal);

CREATE INDEX digest_programmatic_warnings_thread ON digest_programmatic_warnings(seat_id, thread_id, ordinal);

CREATE INDEX allocation_decisions_target ON allocation_decisions(host_id, target_id, ordinal);

CREATE INDEX allocation_decisions_seat_history ON allocation_decisions(seat_id, ordinal);

CREATE INDEX seats_live_ordinal ON seats(instance_id, ordinal) WHERE state!='retired';

CREATE INDEX snapshot_generations_retention ON snapshot_generations(host_id, admission_sequence);

CREATE INDEX wake_work_reserved ON wake_work(seat_id) WHERE reservation_id IS NOT NULL;

CREATE INDEX summary_items_fold ON summary_items(thread_id, chunking_version, seq);

CREATE INDEX summary_transitions_fold ON summary_transitions(thread_id, chunking_version, cite_seq);

CREATE INDEX summary_jobs_live_lease ON summary_jobs(lease_until) WHERE block_id IS NULL AND lease_until IS NOT NULL;

CREATE INDEX summary_job_durations_recent ON summary_job_durations(instance_id, ordinal);

CREATE INDEX send_preparations_retention ON send_preparations(prepared_at, id)
WHERE prepared_at IS NOT NULL AND status IN ('building','sealed');

CREATE INDEX prepared_recipients_required_seat ON prepared_recipients(seat_id,ordinal) WHERE ack_required=1;

CREATE INDEX warning_conditions_active ON warning_conditions(condition_kind,thread_id,condition_id,ordinal) WHERE clear_warning_id IS NULL;

CREATE INDEX warning_conditions_affected ON warning_conditions(affected_seat_id,condition_kind,ordinal) WHERE clear_warning_id IS NULL;

CREATE INDEX warning_conditions_close_scope ON warning_conditions(affected_seat_id,condition_kind,episode,ordinal) WHERE clear_warning_id IS NULL;

CREATE INDEX warning_conditions_thread ON warning_conditions(thread_id,ordinal);

CREATE INDEX warning_conditions_unavailable_reopen ON warning_conditions(thread_id,affected_seat_id,episode,ordinal) WHERE condition_kind='unavailable' AND clear_warning_id IS NULL;

CREATE INDEX warning_close_sweeps_cover ON warning_close_sweeps(condition_kind,affected_seat_id,episode,through_ordinal,after_ordinal);

CREATE INDEX work_jobs_ready ON work_jobs(status, kind, ordinal);

CREATE INDEX work_jobs_live ON work_jobs(ordinal) WHERE status IN ('pending','failed');

CREATE INDEX work_jobs_retention ON work_jobs(kind, completed_at) WHERE status='complete';

CREATE INDEX threads_instance_name ON threads(instance_id, name);

CREATE INDEX threads_recent_activity ON threads(instance_id, last_activity DESC, ordinal DESC);

CREATE INDEX invitations_effective_pending_unwarned ON invitations(deadline_at,ordinal)
    WHERE state='pending' AND warning_message_id IS NULL AND reject_recorded=0;

CREATE INDEX channel_archival_due ON channel_archival(instance_id,due_mono,thread_id) WHERE enabled=1;

CREATE INDEX seat_archival_due ON seat_archival(next_mono,seat_id) WHERE next_mono IS NOT NULL;

CREATE INDEX seat_archival_target ON seat_archival(host_id,target_id) WHERE next_mono IS NOT NULL;

CREATE INDEX channel_handoff_live ON channel_handoff_fences(thread_id,compound) WHERE state='live';

CREATE INDEX channel_handoff_unattached ON channel_handoff_fences(instance_id,compound) WHERE state='live' AND thread_id IS NULL;

CREATE INDEX archival_invitations_thread ON digest_pending_invitations(thread_id,ordinal);

CREATE INDEX archival_preparations_thread ON send_preparations(thread_id,status,id);

CREATE INDEX archival_service_preparations_thread ON service_notification_preparations(thread_id,status,ordinal);

CREATE INDEX archival_summary_leases ON summary_jobs(thread_id,lease_until) WHERE block_id IS NULL AND lease_token IS NOT NULL;

CREATE INDEX archival_preparations_page ON send_preparations(thread_id,id);

CREATE INDEX archival_service_preparations_page ON service_notification_preparations(thread_id,ordinal);

CREATE INDEX archival_positive_targets ON seat_archival(host_id,target_id) WHERE idle_mono IS NOT NULL;

CREATE INDEX archival_positive_seats ON seat_archival(host_id,seat_id) WHERE idle_mono IS NOT NULL;

CREATE INDEX memberships_archival_joined_seat ON memberships(seat_id) WHERE state='joined';

CREATE INDEX lazy_recipients_pending_seat_ordinal ON lazy_recipients(seat_id,ordinal) WHERE state='pending';

CREATE INDEX lazy_recipients_preparation_ordinal ON lazy_recipients(preparation_id,ordinal);

CREATE INDEX harness_version_evidence_seen ON harness_version_evidence(harness, last_seen_at);

CREATE INDEX harness_contract_evidence_v2_seen ON harness_contract_evidence_v2(harness,last_seen_at);

CREATE INDEX harness_contract_diagnostics_recent ON harness_contract_diagnostics(harness, last_seen_at DESC, first_seen_at DESC, session_id, contract_id);

CREATE INDEX bootstrap_keys_attempt ON bootstrap_child_keys(parent_id,attempt,role);

CREATE INDEX bootstrap_keys_legacy_lookup ON bootstrap_child_keys(instance_id,actor_scope,operation_key);

CREATE INDEX automatic_bootstrap_parent_keys ON automatic_bootstrap_keys(parent_id,attempt);

CREATE UNIQUE INDEX occupant_bindings_session_key ON occupant_bindings(seat_id, harness, native_session, agent_id) WHERE ended_at IS NULL;

CREATE INDEX occupant_bindings_open ON occupant_bindings(seat_id) WHERE ended_at IS NULL;

CREATE INDEX occupant_bindings_history ON occupant_bindings(seat_id, ordinal);

CREATE INDEX occupant_bindings_execution ON occupant_bindings(seat_id, execution_id);

CREATE INDEX binding_cursors_thread ON binding_cursors(thread_id, binding_id);

CREATE INDEX lazy_recipients_legacy ON lazy_recipients(seat_id, ordinal) WHERE legacy_pending=1;

CREATE INDEX lazy_recipients_seat_ordinal ON lazy_recipients(seat_id, ordinal);

CREATE INDEX catch_up_extension_until ON catch_up(extension_until) WHERE extension_until IS NOT NULL;

CREATE INDEX catch_up_thread_active ON catch_up(thread_id, seat_id) WHERE state = 'active';

CREATE INDEX catch_up_seat ON catch_up(seat_id, thread_id, state);

CREATE INDEX warning_offer_seat ON warning_offer(seat_id);

CREATE INDEX digest_notice_offer_seat ON digest_notice_offer(seat_id);

CREATE INDEX in_turn_offer_seat ON in_turn_offer(seat_id);

CREATE INDEX invitations_confirmation_due ON invitations(deadline_at,ordinal)
    WHERE state='pending' AND warning_message_id IS NULL AND reject_recorded=0
    AND (confirmation IS NULL OR confirmation='require_ack');

CREATE INDEX receipts_due ON receipts(deadline_at, ordinal) WHERE state = 'pending' AND deadline_at IS NOT NULL;

CREATE INDEX receipts_message_ordinal ON receipts(message_id, ordinal);

CREATE INDEX receipts_pending_unwarned ON receipts(deadline_at, ordinal) WHERE state='pending' AND deadline_at IS NOT NULL AND warning_message_id IS NULL;

CREATE INDEX receipts_required_due ON receipts(deadline_at,ordinal) WHERE state='pending' AND ack_required=1 AND deadline_at IS NOT NULL AND warning_message_id IS NULL;

CREATE INDEX receipts_required_poke ON receipts(deadline_at,message_id,seat_id) WHERE state='pending' AND ack_required=1 AND deadline_at IS NOT NULL AND available_at IS NOT NULL AND warning_message_id IS NULL AND soft_poked_at IS NULL;

CREATE INDEX receipts_required_seat_pending ON receipts(seat_id,ordinal) WHERE state='pending' AND ack_required=1;

CREATE INDEX receipts_required_thread_pending ON receipts(seat_id,thread_id,ordinal) WHERE state='pending' AND ack_required=1;

CREATE INDEX receipts_seat_state_ordinal ON receipts(seat_id, state, ordinal);

CREATE INDEX receipts_seat_thread_state_ordinal ON receipts(seat_id, thread_id, state, ordinal);

CREATE INDEX receipts_thread_pending_ordinal ON receipts(thread_id, ordinal) WHERE state='pending';

CREATE INDEX receipts_thread_seat_pending ON receipts(seat_id, thread_id, ordinal) WHERE state = 'pending';

CREATE INDEX receipt_state_pending_due ON receipt_state(deadline_at, message_id, seat_id) WHERE state='pending' AND deadline_at IS NOT NULL AND warning_message_id IS NULL;

CREATE INDEX receipt_state_required_due ON receipt_state(deadline_at,message_id,seat_id) WHERE state='pending' AND ack_required=1 AND deadline_at IS NOT NULL AND warning_message_id IS NULL;

CREATE INDEX receipt_state_required_poke ON receipt_state(deadline_at,message_id,seat_id) WHERE state='pending' AND ack_required=1 AND deadline_at IS NOT NULL AND available_at IS NOT NULL AND warning_message_id IS NULL AND soft_poked_at IS NULL;

CREATE INDEX bootstrap_live_thread ON bootstrap_handoffs(thread_id,compound) WHERE state NOT IN ('completed','cancelled');

CREATE INDEX bootstrap_live_unattached ON bootstrap_handoffs(instance_id,compound) WHERE thread_id IS NULL AND state NOT IN ('completed','cancelled');

CREATE INDEX invitations_confirmation_scan ON invitations(ordinal)
    WHERE state='pending' AND warning_message_id IS NULL AND reject_recorded=0
    AND (confirmation IS NULL OR confirmation='require_ack');

CREATE INDEX invitations_confirmation_pending_seat ON invitations(seat_id,created_decision_seq,ordinal)
    WHERE state='pending' AND (confirmation IS NULL OR confirmation='require_ack');

CREATE INDEX invitations_confirmation_pending_thread ON invitations(seat_id,thread_id,created_decision_seq,ordinal)
    WHERE state='pending' AND (confirmation IS NULL OR confirmation='require_ack');

CREATE INDEX warning_conditions_invitation_origin ON warning_conditions(invitation_originator_instance_id,invitation_originator_seat_id,ordinal)
    WHERE condition_kind='invitation' AND clear_warning_id IS NULL AND invitation_originator_seat_id IS NOT NULL;

CREATE INDEX receipt_warning_origins_seat ON receipt_warning_origins(instance_id,seat_id,ordinal);

CREATE INDEX receipt_warning_origins_exact ON receipt_warning_origins(instance_id,seat_id,warning_id);

CREATE INDEX receipt_warning_origins_warning ON receipt_warning_origins(warning_id,ordinal);

CREATE INDEX receipts_required_due_ordinal ON receipts(ordinal)
 WHERE state='pending' AND ack_required=1 AND deadline_at IS NOT NULL AND warning_message_id IS NULL;

CREATE INDEX invitation_content_initial ON digest_pending_invitations(seat_id,ordinal);

CREATE INDEX invitation_content_thread ON digest_pending_invitations(seat_id,thread_id,ordinal);

CREATE INDEX invitation_context_changed ON invitation_context_latest(instance_id,last_change_seq,thread_id);

CREATE UNIQUE INDEX invitation_presentation_token ON invitation_presentation_consumers(outbox_token) WHERE outbox_token IS NOT NULL;

CREATE UNIQUE INDEX invitation_presentation_last_token ON invitation_presentation_consumers(last_token) WHERE last_token IS NOT NULL;

CREATE INDEX invitation_presentation_revisit ON invitation_presentation_items(binding_id,consumer,generation,required_render_bytes,revisit_ordinal);

CREATE UNIQUE INDEX invitation_presentation_cleanup ON invitation_presentation_items(binding_id,consumer,revisit_ordinal);

CREATE INDEX presentation_confirming_invitations_seat ON presentation_confirming_invitations(seat_id,ordinal);

CREATE INDEX presentation_conditions_affected ON warning_conditions(affected_seat_id,ordinal) WHERE clear_warning_id IS NULL;

CREATE INDEX presentation_membership_history ON membership_intervals(thread_id,seat_id,ordinal);

CREATE INDEX presentation_prepared_warning ON prepared_unavailable_warnings(warning_id,ordinal);

CREATE TRIGGER host_active_snapshot_published BEFORE UPDATE OF active_snapshot_id ON herdr_hosts
WHEN NEW.active_snapshot_id IS NOT NULL
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1 FROM snapshot_generations g
        WHERE g.id=NEW.active_snapshot_id AND g.host_id=NEW.host_id
          AND g.status='published' AND g.staged_targets=g.expected_targets
    ) THEN RAISE(ABORT,'active snapshot is not complete and published') END;
END;

CREATE TRIGGER host_baseline_snapshot_published BEFORE UPDATE OF recovery_baseline_generation_id ON herdr_hosts
WHEN NEW.recovery_baseline_generation_id IS NOT NULL
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1 FROM snapshot_generations g
        WHERE g.id=NEW.recovery_baseline_generation_id AND g.host_id=NEW.host_id
          AND g.status='published' AND g.staged_targets=g.expected_targets
    ) THEN RAISE(ABORT,'baseline snapshot is not complete and published') END;
END;

CREATE TRIGGER seats_home_node_insert BEFORE INSERT ON seats
WHEN NEW.is_local IS NOT (NEW.home_node = (SELECT id FROM local_node))
BEGIN SELECT RAISE(ABORT, 'seat home mismatch'); END;

CREATE TRIGGER seats_home_node_update BEFORE UPDATE OF home_node, is_local, state ON seats
WHEN NEW.is_local IS NOT (NEW.home_node = (SELECT id FROM local_node))
BEGIN SELECT RAISE(ABORT, 'seat home mismatch'); END;

CREATE TRIGGER seats_ordinal_immutable BEFORE UPDATE OF ordinal ON seats
BEGIN SELECT RAISE(ABORT, 'seat ordinal is immutable'); END;

CREATE TRIGGER threads_ordinal_immutable BEFORE UPDATE OF ordinal ON threads
BEGIN SELECT RAISE(ABORT, 'thread ordinal is immutable'); END;

CREATE TRIGGER memberships_ordinal_immutable BEFORE UPDATE OF ordinal ON memberships
BEGIN SELECT RAISE(ABORT, 'membership ordinal is immutable'); END;

CREATE TRIGGER invitations_ordinal_immutable BEFORE UPDATE OF ordinal ON invitations
BEGIN SELECT RAISE(ABORT, 'invitation ordinal is immutable'); END;

CREATE TRIGGER invitations_decision_immutable BEFORE UPDATE OF created_decision_seq ON invitations
BEGIN SELECT RAISE(ABORT, 'invitation decision is immutable'); END;

CREATE TRIGGER messages_instance_thread BEFORE INSERT ON messages
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1 FROM threads t WHERE t.id=NEW.thread_id AND t.instance_id=NEW.instance_id
    ) THEN RAISE(ABORT, 'message instance does not match thread') END;
END;

CREATE TRIGGER manifests_instance_source BEFORE INSERT ON send_manifests
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1 FROM send_preparations sp JOIN threads t ON t.id=NEW.thread_id
        JOIN messages m ON m.id=NEW.message_id
        WHERE sp.id=NEW.preparation_id AND sp.instance_id=NEW.instance_id
          AND t.instance_id=NEW.instance_id AND m.instance_id=NEW.instance_id
          AND m.thread_id=NEW.thread_id AND m.decision_seq=NEW.decision_seq
          AND m.event_offset=0 AND m.kind='ordinary'
    ) THEN RAISE(ABORT, 'manifest instance or source mismatch') END;
END;

CREATE TRIGGER send_manifests_immutable BEFORE UPDATE ON send_manifests
BEGIN SELECT RAISE(ABORT, 'published manifest is immutable'); END;

CREATE TRIGGER messages_retained BEFORE DELETE ON messages
BEGIN SELECT RAISE(ABORT, 'message history is retained'); END;

CREATE TRIGGER prepared_recipients_ordinal_immutable BEFORE UPDATE OF ordinal ON prepared_recipients
BEGIN SELECT RAISE(ABORT, 'prepared recipient ordinal is immutable'); END;

CREATE TRIGGER seats_retirement_terminal BEFORE UPDATE OF state, retired_at ON seats
WHEN OLD.state = 'retired' AND (NEW.state <> 'retired' OR NEW.retired_at <> OLD.retired_at)
BEGIN SELECT RAISE(ABORT, 'retired seat is terminal'); END;

CREATE TRIGGER retirements_provenance_immutable BEFORE UPDATE OF ordinal, id, seat_id, cutover_at,
    closure_boot, closure_epoch, closure_target, closure_generation ON retirements
BEGIN SELECT RAISE(ABORT, 'retirement provenance is immutable'); END;

CREATE TRIGGER service_authors_immutable BEFORE UPDATE ON service_authors
BEGIN SELECT RAISE(ABORT, 'service author is immutable'); END;

CREATE TRIGGER service_authors_retained BEFORE DELETE ON service_authors
BEGIN SELECT RAISE(ABORT, 'service author is retained'); END;

CREATE TRIGGER threads_managed_owner_instance_insert
BEFORE INSERT ON threads
WHEN NEW.managed_owner_author_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'managed owner instance mismatch')
    WHERE NOT EXISTS (
        SELECT 1 FROM service_authors a
        WHERE a.id = NEW.managed_owner_author_id AND a.instance_id = NEW.instance_id
    );
END;

CREATE TRIGGER threads_managed_owner_instance_update
BEFORE UPDATE OF managed_owner_author_id, instance_id ON threads
WHEN NEW.managed_owner_author_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'managed owner instance mismatch')
    WHERE NOT EXISTS (
        SELECT 1 FROM service_authors a
        WHERE a.id = NEW.managed_owner_author_id AND a.instance_id = NEW.instance_id
    );
END;

CREATE TRIGGER threads_managed_owner_immutable
BEFORE UPDATE OF managed_owner_author_id ON threads
WHEN OLD.managed_owner_author_id IS NOT NULL
    AND NEW.managed_owner_author_id IS NOT OLD.managed_owner_author_id
BEGIN
    SELECT RAISE(ABORT, 'managed owner is immutable');
END;

CREATE TRIGGER messages_author_shape_insert
BEFORE INSERT ON messages
BEGIN
    SELECT RAISE(ABORT, 'invalid message author') WHERE
        (NEW.author_kind = 'programmatic' AND (
            NEW.author_service_id IS NULL OR NEW.actor_seat_id IS NOT NULL OR
            NOT EXISTS (SELECT 1 FROM service_authors a
                WHERE a.id = NEW.author_service_id AND a.instance_id = NEW.instance_id)
        )) OR
        (NEW.author_kind IS NOT NULL AND NEW.author_kind <> 'programmatic'
            AND NEW.author_service_id IS NOT NULL) OR
        (NEW.author_kind = 'native' AND NEW.actor_seat_id IS NULL) OR
        (NEW.author_kind = 'built_in' AND NEW.actor_seat_id IS NOT NULL) OR
        (NEW.author_kind IS NULL AND NEW.author_service_id IS NOT NULL);
END;

CREATE TRIGGER requirement_episodes_owner_insert BEFORE INSERT ON requirement_episodes
BEGIN
    SELECT RAISE(ABORT, 'requirement relation mismatch') WHERE NOT EXISTS (
        SELECT 1 FROM threads t
        JOIN seats s ON s.id = NEW.seat_id AND s.instance_id = t.instance_id
        JOIN invitations i ON i.id = NEW.invitation_id
            AND i.thread_id = NEW.thread_id AND i.seat_id = NEW.seat_id
        WHERE t.id = NEW.thread_id AND t.managed_owner_author_id = NEW.issuer_author_id
            AND s.state <> 'retired'
    );
END;

CREATE TRIGGER requirement_episodes_identity_immutable
BEFORE UPDATE OF id, thread_id, seat_id, issuer_author_id, invitation_id,
    created_decision_seq, created_at ON requirement_episodes
BEGIN
    SELECT RAISE(ABORT, 'requirement identity immutable');
END;

CREATE TRIGGER requirement_episodes_revision_forward
BEFORE UPDATE ON requirement_episodes
WHEN NEW.revision <= OLD.revision
BEGIN
    SELECT RAISE(ABORT, 'requirement revision must advance');
END;

CREATE TRIGGER requirement_episodes_acceptance_provenance
BEFORE UPDATE ON requirement_episodes
WHEN (
    OLD.accepted_at IS NULL AND
    (NEW.accepted_by_seat_id IS NOT NULL OR NEW.accepted_generation IS NOT NULL OR
     NEW.accepted_observation IS NOT NULL OR NEW.accepted_at IS NOT NULL) AND
    NOT (OLD.state = 'pending' AND NEW.state = 'accepted')
) OR (
    OLD.accepted_at IS NOT NULL AND
    (NEW.accepted_by_seat_id IS NOT OLD.accepted_by_seat_id OR
     NEW.accepted_generation IS NOT OLD.accepted_generation OR
     NEW.accepted_observation IS NOT OLD.accepted_observation OR
     NEW.accepted_at IS NOT OLD.accepted_at)
)
BEGIN
    SELECT RAISE(ABORT, 'requirement acceptance provenance is immutable');
END;

CREATE TRIGGER requirement_episodes_state_forward
BEFORE UPDATE ON requirement_episodes
WHEN OLD.state IN ('released', 'retired') OR
    (OLD.state = 'accepted' AND NEW.state NOT IN ('accepted', 'released', 'retired'))
BEGIN
    SELECT RAISE(ABORT, 'requirement state is terminal or cannot reverse');
END;

CREATE TRIGGER requirement_episodes_retained BEFORE DELETE ON requirement_episodes
BEGIN SELECT RAISE(ABORT, 'requirement episode is retained'); END;

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

CREATE TRIGGER digest_invitation_created AFTER INSERT ON invitations WHEN NEW.state='pending'
BEGIN
    INSERT OR IGNORE INTO digest_pending_invitations(seat_id, invitation_id, thread_id, created_decision_seq, ordinal) VALUES (NEW.seat_id, NEW.id, NEW.thread_id, NEW.created_decision_seq, NEW.ordinal);
END;

CREATE TRIGGER digest_invitation_settled AFTER UPDATE OF state ON invitations WHEN NEW.state!='pending'
BEGIN
    DELETE FROM digest_pending_invitations WHERE seat_id=OLD.seat_id AND invitation_id=OLD.id;
    DELETE FROM digest_open_warnings WHERE condition_kind='invitation' AND condition_id=OLD.id;
END;

CREATE TRIGGER digest_invitation_cancelled AFTER INSERT ON invitation_cancellations
BEGIN
    DELETE FROM digest_pending_invitations WHERE invitation_id=NEW.invitation_id AND seat_id=(SELECT seat_id FROM invitations WHERE id=NEW.invitation_id);
    DELETE FROM digest_open_warnings WHERE condition_kind='invitation' AND condition_id=NEW.invitation_id;
END;

CREATE TRIGGER digest_manifest_receipt_staged AFTER INSERT ON prepared_recipients
BEGIN
    INSERT OR IGNORE INTO digest_pending_manifest_receipts(seat_id, preparation_id, thread_id, ordinal) VALUES (NEW.seat_id, NEW.preparation_id, NEW.thread_id, NEW.ordinal);
END;

CREATE TRIGGER digest_manifest_receipt_unstaged AFTER DELETE ON prepared_recipients
BEGIN
    DELETE FROM digest_pending_manifest_receipts WHERE seat_id=OLD.seat_id AND preparation_id=OLD.preparation_id;
END;

CREATE TRIGGER digest_open_warning_job AFTER INSERT ON warning_jobs WHEN NEW.condition_kind IN ('invitation','receipt')
BEGIN
    INSERT OR IGNORE INTO digest_open_warnings(source, source_ordinal, warning_id, thread_id, affected_seat_id, condition_kind, condition_id, episode) VALUES ('job', NEW.ordinal, NEW.warning_id, NEW.thread_id, NEW.affected_seat_id, NEW.condition_kind, NEW.condition_id, NULL);
END;

CREATE TRIGGER digest_open_warning_prepared AFTER INSERT ON prepared_unavailable_warnings
BEGIN
    INSERT OR IGNORE INTO digest_open_warnings(source, source_ordinal, warning_id, thread_id, affected_seat_id, condition_kind, condition_id, episode) SELECT 'prepared', NEW.ordinal, NEW.warning_id, p.thread_id, NEW.affected_seat_id, 'unavailable', NEW.warning_key, NEW.unavailability_episode FROM send_preparations p JOIN seats s ON s.id=NEW.affected_seat_id WHERE p.id=NEW.preparation_id AND s.state!='retired' AND s.unavailability_open=1 AND s.unavailability_episode=NEW.unavailability_episode;
END;

CREATE TRIGGER digest_open_warning_unprepared AFTER DELETE ON prepared_unavailable_warnings
BEGIN
    DELETE FROM digest_open_warnings WHERE source='prepared' AND source_ordinal=OLD.ordinal;
END;

CREATE TRIGGER digest_unavailability_settled AFTER UPDATE OF state, unavailability_open, unavailability_episode ON seats WHEN NEW.state='retired' OR NEW.unavailability_open=0 OR NEW.unavailability_episode!=OLD.unavailability_episode
BEGIN
    DELETE FROM digest_open_warnings WHERE affected_seat_id=NEW.id AND condition_kind='unavailable' AND (NEW.state='retired' OR NEW.unavailability_open=0 OR episode!=NEW.unavailability_episode);
END;

CREATE TRIGGER digest_open_warning_recipient_projected AFTER INSERT ON warning_recipients
BEGIN
    INSERT OR IGNORE INTO digest_open_warning_recipients(seat_id, warning_id, thread_id, source, source_ordinal) SELECT NEW.seat_id, d.warning_id, d.thread_id, d.source, d.source_ordinal FROM digest_open_warnings d WHERE d.warning_id=NEW.warning_id;
END;

CREATE TRIGGER digest_open_warning_closed AFTER DELETE ON digest_open_warnings
BEGIN
    DELETE FROM digest_open_warning_recipients WHERE warning_id=OLD.warning_id AND source=OLD.source AND source_ordinal=OLD.source_ordinal;
END;

CREATE TRIGGER digest_programmatic_warning_projected AFTER INSERT ON warning_recipients WHEN EXISTS (SELECT 1 FROM service_notification_publications p JOIN messages m ON m.id=p.message_id WHERE p.message_id=NEW.warning_id AND m.kind='warn' AND m.author_kind='programmatic')
BEGIN
    INSERT OR IGNORE INTO digest_programmatic_warnings(seat_id, warning_id, thread_id, event_seq, event_offset) SELECT NEW.seat_id, m.id, m.thread_id, m.decision_seq, m.event_offset FROM messages m WHERE m.id=NEW.warning_id;
END;

CREATE TRIGGER messages_immutable BEFORE UPDATE ON messages
BEGIN SELECT RAISE(ABORT, 'message history is immutable'); END;

CREATE TRIGGER messages_summary_author_insert BEFORE INSERT ON messages
BEGIN
    SELECT RAISE(ABORT, 'invalid message summary authorship') WHERE
        NEW.author_role_backfilled <> 0 OR
        (NEW.author_kind = 'programmatic' AND NEW.relays_user <> 0);
END;

CREATE TRIGGER summary_blocks_immutable BEFORE UPDATE ON summary_blocks
BEGIN SELECT RAISE(ABORT, 'summary blocks are immutable'); END;

CREATE TRIGGER summary_blocks_retained BEFORE DELETE ON summary_blocks
BEGIN SELECT RAISE(ABORT, 'summary blocks are retained'); END;

CREATE TRIGGER send_manifest_preparation_published AFTER INSERT ON send_manifests
BEGIN
    UPDATE send_preparations SET prepared_at=NULL WHERE id=NEW.preparation_id;
END;

CREATE TRIGGER human_receipt_prepared_waived AFTER UPDATE OF ack_required ON prepared_recipients WHEN NEW.ack_required=0
BEGIN
    DELETE FROM digest_pending_manifest_receipts WHERE preparation_id=NEW.preparation_id AND seat_id=NEW.seat_id;
    DELETE FROM digest_open_warnings WHERE condition_kind='receipt' AND condition_id IN
        (SELECT length(CAST(sm.message_id AS BLOB))||':'||sm.message_id||':'||NEW.seat_id
         FROM send_manifests sm WHERE sm.preparation_id=NEW.preparation_id);
END;

CREATE TRIGGER wake_batches_clear_invitation
AFTER DELETE ON digest_pending_invitations
WHEN NOT EXISTS(SELECT 1 FROM digest_pending_invitations WHERE seat_id=OLD.seat_id)
      AND NOT EXISTS(SELECT 1 FROM digest_pending_manifest_receipts WHERE seat_id=OLD.seat_id)
      AND NOT EXISTS(SELECT 1 FROM receipts WHERE seat_id=OLD.seat_id AND state='pending' AND ack_required=1)
BEGIN DELETE FROM wake_batches WHERE seat_id=OLD.seat_id; END;

CREATE TRIGGER wake_batches_clear_manifest
AFTER DELETE ON digest_pending_manifest_receipts
WHEN NOT EXISTS(SELECT 1 FROM digest_pending_invitations WHERE seat_id=OLD.seat_id)
      AND NOT EXISTS(SELECT 1 FROM digest_pending_manifest_receipts WHERE seat_id=OLD.seat_id)
      AND NOT EXISTS(SELECT 1 FROM receipts WHERE seat_id=OLD.seat_id AND state='pending' AND ack_required=1)
BEGIN DELETE FROM wake_batches WHERE seat_id=OLD.seat_id; END;

CREATE TRIGGER wake_batches_clear_retired
AFTER UPDATE OF state ON seats
WHEN NEW.state='retired'
BEGIN DELETE FROM wake_batches WHERE seat_id=NEW.id; END;

CREATE TRIGGER threads_recent_insert AFTER INSERT ON threads BEGIN
    UPDATE threads SET last_activity=NEW.created_at WHERE id=NEW.id;
    INSERT INTO filter_revisions(instance_id,scope_kind,scope_key,revision) VALUES(NEW.instance_id,'directory','recent:all',1)
    ON CONFLICT(instance_id,scope_kind,scope_key) DO UPDATE SET revision=revision+1;
END;

CREATE TRIGGER threads_recent_update AFTER UPDATE OF timeline_revision,name,topic,archived ON threads BEGIN
    UPDATE threads SET last_activity=max(last_activity,NEW.updated_at) WHERE id=NEW.id AND NEW.timeline_revision!=OLD.timeline_revision;
    INSERT INTO filter_revisions(instance_id,scope_kind,scope_key,revision) VALUES(NEW.instance_id,'directory','recent:all',1)
    ON CONFLICT(instance_id,scope_kind,scope_key) DO UPDATE SET revision=revision+1;
END;

CREATE TRIGGER invitation_rejections_shape BEFORE INSERT ON invitation_rejections
BEGIN
    SELECT RAISE(ABORT, 'invalid invitation rejection') WHERE NOT EXISTS (
        SELECT 1 FROM invitations i WHERE i.id=NEW.invitation_id
        AND i.seat_id=NEW.actor_seat_id AND i.state='pending'
        AND NOT EXISTS(SELECT 1 FROM invitation_cancellations c WHERE c.invitation_id=i.id)
        AND NOT EXISTS(SELECT 1 FROM requirement_episodes r WHERE r.invitation_id=i.id AND r.state IN ('pending','accepted'))
    );
END;

CREATE TRIGGER invitation_rejections_immutable BEFORE UPDATE ON invitation_rejections
BEGIN SELECT RAISE(ABORT, 'invitation rejection is immutable'); END;

CREATE TRIGGER invitation_rejections_retained BEFORE DELETE ON invitation_rejections
BEGIN SELECT RAISE(ABORT, 'invitation rejection is retained'); END;

CREATE TRIGGER digest_invitation_rejected AFTER INSERT ON invitation_rejections
BEGIN
    DELETE FROM digest_pending_invitations WHERE invitation_id=NEW.invitation_id AND seat_id=NEW.actor_seat_id;
    DELETE FROM digest_open_warnings WHERE condition_kind='invitation' AND condition_id=NEW.invitation_id;
END;

CREATE TRIGGER invitations_rejection_projection_insert BEFORE INSERT ON invitations
WHEN NEW.reject_recorded<>0
BEGIN SELECT RAISE(ABORT, 'new invitation cannot claim rejection'); END;

CREATE TRIGGER invitations_rejection_projection_guard BEFORE UPDATE OF reject_recorded ON invitations
WHEN NEW.reject_recorded<>EXISTS(SELECT 1 FROM invitation_rejections r WHERE r.invitation_id=NEW.id)
BEGIN SELECT RAISE(ABORT, 'invitation rejection projection mismatch'); END;

CREATE TRIGGER invitation_rejections_project AFTER INSERT ON invitation_rejections
BEGIN UPDATE invitations SET reject_recorded=1 WHERE id=NEW.invitation_id; END;

CREATE TRIGGER messages_user_intent_insert
BEFORE INSERT ON messages
WHEN NEW.user_intent IS NOT NULL
BEGIN
    SELECT CASE WHEN COALESCE(
        NEW.kind = 'ordinary' AND
        (NEW.author_role = 'human' OR
         (NEW.author_role = 'agent' AND NEW.relays_user = 1)), 0
    ) = 0 THEN RAISE(ABORT, 'invalid user intent attribution') END;
END;

CREATE TRIGGER channel_handoff_terminal BEFORE UPDATE ON channel_handoff_fences
WHEN OLD.state='completed' AND (NEW.state!='completed' OR NEW.thread_id IS NOT OLD.thread_id OR NEW.completed_at IS NOT OLD.completed_at)
BEGIN SELECT RAISE(ABORT,'completed handoff is absorbing'); END;

CREATE TRIGGER channel_handoff_identity BEFORE UPDATE ON channel_handoff_fences
WHEN NEW.instance_id!=OLD.instance_id OR NEW.actor_scope!=OLD.actor_scope OR NEW.compound!=OLD.compound
 OR NEW.digest!=OLD.digest OR NEW.claim_json!=OLD.claim_json OR NEW.recipient!=OLD.recipient
 OR NEW.create_key!=OLD.create_key OR NEW.invite_key!=OLD.invite_key OR NEW.send_key!=OLD.send_key
 OR NEW.original_thread IS NOT OLD.original_thread
BEGIN SELECT RAISE(ABORT,'immutable handoff identity'); END;

CREATE TRIGGER channel_handoff_retained BEFORE DELETE ON channel_handoff_fences
BEGIN SELECT RAISE(ABORT,'handoff identities are retained'); END;

CREATE TRIGGER archival_thread_insert AFTER INSERT ON threads BEGIN
 INSERT INTO channel_archival(thread_id,instance_id,enabled) VALUES(NEW.id,NEW.instance_id,1-NEW.archived);
 UPDATE archival_instances SET mutation_revision=mutation_revision+1 WHERE instance_id=NEW.instance_id;
END;

CREATE TRIGGER archival_thread_activity AFTER UPDATE OF archived,topic,goal,name,membership_revision,timeline_revision ON threads BEGIN
 UPDATE channel_archival SET activity_revision=activity_revision+1,quiet_mono=NULL,scan_revision=NULL,due_mono=0,enabled=1-NEW.archived WHERE thread_id=NEW.id;
 UPDATE archival_instances SET mutation_revision=mutation_revision+1 WHERE instance_id=NEW.instance_id;
END;

CREATE TRIGGER archival_seat_insert AFTER INSERT ON seats WHEN NEW.host_id IS NOT NULL BEGIN
 INSERT INTO seat_archival(seat_id,instance_id,host_id) VALUES(NEW.id,NEW.instance_id,NEW.host_id);
END;

CREATE TRIGGER archival_operation_activity AFTER INSERT ON operations WHEN NEW.actor_scope LIKE 'seat:%' BEGIN
 UPDATE seat_archival SET activity_revision=activity_revision+1,idle_mono=NULL,samples=0,next_mono=0 WHERE seat_id=substr(NEW.actor_scope,6);
 UPDATE archival_instances SET mutation_revision=mutation_revision+1 WHERE instance_id=(SELECT instance_id FROM seats WHERE id=substr(NEW.actor_scope,6));
END;

CREATE TRIGGER archival_fence_seats_delete AFTER DELETE ON seats BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_seat_availability_insert AFTER INSERT ON seat_availability BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_seat_availability_update AFTER UPDATE ON seat_availability BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_seat_availability_delete AFTER DELETE ON seat_availability BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_memberships_insert AFTER INSERT ON memberships BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_memberships_delete AFTER DELETE ON memberships BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_membership_intervals_insert AFTER INSERT ON membership_intervals BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_membership_intervals_update AFTER UPDATE ON membership_intervals BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_membership_intervals_delete AFTER DELETE ON membership_intervals BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_invitations_insert AFTER INSERT ON invitations BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_invitations_update AFTER UPDATE ON invitations BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_invitations_delete AFTER DELETE ON invitations BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_invitation_cancellations_insert AFTER INSERT ON invitation_cancellations BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_invitation_cancellations_update AFTER UPDATE ON invitation_cancellations BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_invitation_cancellations_delete AFTER DELETE ON invitation_cancellations BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_invitation_rejections_insert AFTER INSERT ON invitation_rejections BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_invitation_rejections_update AFTER UPDATE ON invitation_rejections BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_invitation_rejections_delete AFTER DELETE ON invitation_rejections BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_human_receipt_waivers_insert AFTER INSERT ON human_receipt_waivers BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_human_receipt_waivers_update AFTER UPDATE ON human_receipt_waivers BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_human_receipt_waivers_delete AFTER DELETE ON human_receipt_waivers BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_requirement_episodes_insert AFTER INSERT ON requirement_episodes BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_requirement_episodes_update AFTER UPDATE ON requirement_episodes BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_requirement_episodes_delete AFTER DELETE ON requirement_episodes BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_send_preparations_insert AFTER INSERT ON send_preparations BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_send_preparations_update AFTER UPDATE ON send_preparations BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_send_preparations_delete AFTER DELETE ON send_preparations BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_prepared_recipients_insert AFTER INSERT ON prepared_recipients BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_prepared_recipients_update AFTER UPDATE ON prepared_recipients BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_prepared_recipients_delete AFTER DELETE ON prepared_recipients BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_send_manifests_insert AFTER INSERT ON send_manifests BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_send_manifests_update AFTER UPDATE ON send_manifests BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_send_manifests_delete AFTER DELETE ON send_manifests BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_service_notification_preparations_insert AFTER INSERT ON service_notification_preparations BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_service_notification_preparations_update AFTER UPDATE ON service_notification_preparations BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_service_notification_preparations_delete AFTER DELETE ON service_notification_preparations BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_service_notification_publications_insert AFTER INSERT ON service_notification_publications BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_service_notification_publications_update AFTER UPDATE ON service_notification_publications BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_service_notification_publications_delete AFTER DELETE ON service_notification_publications BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_summary_jobs_insert AFTER INSERT ON summary_jobs BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_summary_jobs_update AFTER UPDATE ON summary_jobs BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_summary_jobs_delete AFTER DELETE ON summary_jobs BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_recovery_holds_insert AFTER INSERT ON recovery_holds BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_recovery_holds_update AFTER UPDATE ON recovery_holds BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_recovery_holds_delete AFTER DELETE ON recovery_holds BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_recovery_baseline_releases_insert AFTER INSERT ON recovery_baseline_releases BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_recovery_baseline_releases_update AFTER UPDATE ON recovery_baseline_releases BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_recovery_baseline_releases_delete AFTER DELETE ON recovery_baseline_releases BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_channel_handoff_fences_insert AFTER INSERT ON channel_handoff_fences BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_channel_handoff_fences_update AFTER UPDATE ON channel_handoff_fences BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_channel_handoff_fences_delete AFTER DELETE ON channel_handoff_fences BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_seat_activity AFTER UPDATE OF state,generation,target_generation,target_id,host_id ON seats BEGIN
 UPDATE seat_archival SET host_id=COALESCE(NEW.host_id,host_id),activity_revision=activity_revision+1,idle_mono=NULL,samples=0,next_mono=0 WHERE seat_id=NEW.id;
END;

CREATE TRIGGER archival_host_invalidation AFTER UPDATE OF host_boot,host_epoch,invalidation_revision,baseline_hold_unclaimed ON herdr_hosts
WHEN NEW.host_boot IS NOT OLD.host_boot OR NEW.host_epoch!=OLD.host_epoch OR NEW.invalidation_revision!=OLD.invalidation_revision OR NEW.baseline_hold_unclaimed!=OLD.baseline_hold_unclaimed BEGIN
 UPDATE archival_hosts SET evidence_epoch=evidence_epoch+1,coherent_since=NULL WHERE host_id=NEW.host_id;
 UPDATE archival_instances SET mutation_revision=mutation_revision+1 WHERE instance_id=NEW.node_id;
END;

CREATE TRIGGER archival_current_negative_insert AFTER INSERT ON observed_targets WHEN NEW.ui_state!='idle' BEGIN
 UPDATE seat_archival SET idle_mono=NULL,samples=0,next_mono=0 WHERE host_id=NEW.host_id AND target_id=NEW.target_id AND next_mono IS NOT NULL;
 UPDATE archival_instances SET mutation_revision=mutation_revision+1 WHERE instance_id=(SELECT node_id FROM herdr_hosts WHERE host_id=NEW.host_id);
END;

CREATE TRIGGER archival_current_negative_update AFTER UPDATE ON observed_targets WHEN NEW.ui_state!='idle' BEGIN
 UPDATE seat_archival SET idle_mono=NULL,samples=0,next_mono=0 WHERE host_id=NEW.host_id AND target_id=NEW.target_id AND next_mono IS NOT NULL;
 UPDATE archival_instances SET mutation_revision=mutation_revision+1 WHERE instance_id=(SELECT node_id FROM herdr_hosts WHERE host_id=NEW.host_id);
END;

CREATE TRIGGER archival_membership_activity_insert AFTER INSERT ON memberships BEGIN
 UPDATE seat_archival SET next_mono=0 WHERE seat_id=NEW.seat_id;
 UPDATE channel_archival SET activity_revision=activity_revision+1,quiet_mono=NULL,scan_revision=NULL,due_mono=0 WHERE thread_id=NEW.thread_id;
END;

CREATE TRIGGER archival_membership_activity_update AFTER UPDATE ON memberships BEGIN
 UPDATE seat_archival SET next_mono=0 WHERE seat_id IN (OLD.seat_id,NEW.seat_id);
 UPDATE channel_archival SET activity_revision=activity_revision+1,quiet_mono=NULL,scan_revision=NULL,due_mono=0 WHERE thread_id=NEW.thread_id;
END;

CREATE TRIGGER archival_message_activity AFTER INSERT ON messages BEGIN
 UPDATE channel_archival SET activity_revision=activity_revision+1,quiet_mono=NULL,scan_revision=NULL,due_mono=0 WHERE thread_id=NEW.thread_id;
 UPDATE archival_instances SET mutation_revision=mutation_revision+1 WHERE instance_id=NEW.instance_id;
END;

CREATE TRIGGER archival_current_structure_insert AFTER INSERT ON observed_targets
WHEN EXISTS(SELECT 1 FROM seat_archival a WHERE a.host_id=NEW.host_id AND a.target_id=NEW.target_id AND a.idle_mono IS NOT NULL AND
 (a.target_generation!=NEW.generation OR a.terminal IS NOT NEW.terminal_id OR a.incarnation IS NOT NEW.incarnation OR a.host_boot!=NEW.host_boot OR a.host_epoch!=NEW.epoch)) BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1 WHERE instance_id=(SELECT node_id FROM herdr_hosts WHERE host_id=NEW.host_id);
 UPDATE seat_archival SET idle_mono=NULL,samples=0,next_mono=0 WHERE host_id=NEW.host_id AND target_id=NEW.target_id AND idle_mono IS NOT NULL;
END;

CREATE TRIGGER archival_current_structure_update AFTER UPDATE ON observed_targets
WHEN EXISTS(SELECT 1 FROM seat_archival a WHERE a.host_id=NEW.host_id AND a.target_id=NEW.target_id AND a.idle_mono IS NOT NULL AND
 (a.target_generation!=NEW.generation OR a.terminal IS NOT NEW.terminal_id OR a.incarnation IS NOT NEW.incarnation OR a.host_boot!=NEW.host_boot OR a.host_epoch!=NEW.epoch)) BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1 WHERE instance_id=(SELECT node_id FROM herdr_hosts WHERE host_id=NEW.host_id);
 UPDATE seat_archival SET idle_mono=NULL,samples=0,next_mono=0 WHERE host_id=NEW.host_id AND target_id=NEW.target_id AND idle_mono IS NOT NULL;
END;

CREATE TRIGGER archival_snapshot_structure AFTER INSERT ON snapshot_targets
WHEN EXISTS(SELECT 1 FROM snapshot_generations g JOIN seat_archival a ON a.host_id=g.host_id AND a.target_id=NEW.target_id WHERE g.id=NEW.generation_id AND a.idle_mono IS NOT NULL AND
 (a.target_generation!=NEW.generation OR a.terminal IS NOT NEW.terminal_id OR a.incarnation IS NOT g.incarnation OR a.host_boot!=g.host_boot OR a.host_epoch!=g.epoch OR NEW.ui_state NOT IN ('idle','unknown'))) BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1 WHERE instance_id=(SELECT h.node_id FROM snapshot_generations g JOIN herdr_hosts h ON h.host_id=g.host_id WHERE g.id=NEW.generation_id);
 UPDATE seat_archival SET idle_mono=NULL,samples=0,next_mono=0 WHERE host_id=(SELECT g.host_id FROM snapshot_generations g WHERE g.id=NEW.generation_id) AND target_id=NEW.target_id AND idle_mono IS NOT NULL;
END;

CREATE TRIGGER archival_membership_queue_delete AFTER DELETE ON memberships BEGIN
 UPDATE seat_archival SET next_mono=0 WHERE seat_id=OLD.seat_id;
END;

CREATE TRIGGER digest_transition_warning_projected AFTER INSERT ON warning_recipients WHEN EXISTS (SELECT 1 FROM warning_conditions c WHERE c.open_warning_id=NEW.warning_id OR c.clear_warning_id=NEW.warning_id)
BEGIN
    INSERT OR IGNORE INTO digest_programmatic_warnings(seat_id,warning_id,thread_id,event_seq,event_offset) SELECT NEW.seat_id,m.id,m.thread_id,m.decision_seq,m.event_offset FROM messages m WHERE m.id=NEW.warning_id UNION ALL SELECT NEW.seat_id,w.warning_id,sm.thread_id,sm.decision_seq,w.warning_offset FROM prepared_unavailable_warnings w JOIN send_manifests sm ON sm.preparation_id=w.preparation_id WHERE w.warning_id=NEW.warning_id AND NOT EXISTS(SELECT 1 FROM messages m WHERE m.id=w.warning_id);
    -- Delete the shared legacy parent, not just this recipient's projection:
    -- a later-running legacy recipient trigger then has nothing to reinsert.
    -- Other recipients awaiting attribution remain in warning_jobs backlog.
    DELETE FROM digest_open_warnings WHERE warning_id=NEW.warning_id AND EXISTS(SELECT 1 FROM digest_programmatic_warnings d WHERE d.seat_id=NEW.seat_id AND d.warning_id=NEW.warning_id);
END;

CREATE TRIGGER lazy_preparation_mode_immutable BEFORE UPDATE OF delivery_mode ON send_preparations
WHEN NEW.delivery_mode IS NOT OLD.delivery_mode
BEGIN SELECT RAISE(ABORT, 'delivery mode is immutable'); END;

CREATE TRIGGER lazy_message_mode_immutable BEFORE UPDATE OF delivery_mode ON messages
WHEN NEW.delivery_mode IS NOT OLD.delivery_mode
BEGIN SELECT RAISE(ABORT, 'recorded message mode is immutable'); END;

CREATE TRIGGER lazy_message_shape BEFORE INSERT ON messages
WHEN NEW.delivery_mode='lazy' AND NEW.kind<>'ordinary'
BEGIN SELECT RAISE(ABORT, 'lazy message must be ordinary content'); END;

CREATE TRIGGER lazy_recipient_identity_immutable BEFORE UPDATE OF ordinal,preparation_id,message_id,thread_id,seat_id ON lazy_recipients
BEGIN SELECT RAISE(ABORT, 'lazy recipient identity is immutable'); END;

CREATE TRIGGER lazy_recipient_progress_forward BEFORE UPDATE OF state ON lazy_recipients
WHEN OLD.state='displayed' AND NEW.state<>'displayed'
BEGIN SELECT RAISE(ABORT, 'lazy delivery progress cannot regress'); END;

CREATE TRIGGER lazy_recipient_source BEFORE INSERT ON lazy_recipients
BEGIN
    SELECT RAISE(ABORT, 'lazy recipient source mismatch') WHERE NEW.state<>'pending' OR NEW.decision_seq IS NOT NULL OR NOT EXISTS (
        SELECT 1 FROM send_preparations p JOIN seats s ON s.id=NEW.seat_id AND s.instance_id=p.instance_id
        WHERE p.id=NEW.preparation_id AND p.thread_id=NEW.thread_id AND p.status='building' AND p.priority=NEW.priority
    ) OR EXISTS(SELECT 1 FROM send_manifests WHERE preparation_id=NEW.preparation_id);
END;

CREATE TRIGGER lazy_recipient_display_published BEFORE UPDATE OF state ON lazy_recipients
WHEN NEW.state='displayed' AND NOT EXISTS (
    SELECT 1 FROM send_manifests sm JOIN messages m ON m.id=sm.message_id
    WHERE sm.preparation_id=OLD.preparation_id AND sm.message_id=OLD.message_id AND sm.thread_id=OLD.thread_id
)
BEGIN SELECT RAISE(ABORT, 'unpublished lazy delivery cannot complete'); END;

CREATE TRIGGER lazy_recipient_published_retained BEFORE DELETE ON lazy_recipients
WHEN EXISTS(SELECT 1 FROM send_manifests WHERE preparation_id=OLD.preparation_id)
BEGIN SELECT RAISE(ABORT, 'published lazy delivery is retained'); END;

CREATE TRIGGER lazy_manifest_source BEFORE INSERT ON send_manifests
BEGIN
    SELECT RAISE(ABORT, 'lazy manifest source mismatch') WHERE
        (SELECT delivery_mode FROM send_preparations WHERE id=NEW.preparation_id) IS NOT (SELECT delivery_mode FROM messages WHERE id=NEW.message_id)
        OR (SELECT priority FROM send_preparations WHERE id=NEW.preparation_id) IS NOT (SELECT priority FROM messages WHERE id=NEW.message_id)
        OR (((SELECT delivery_mode FROM send_preparations WHERE id=NEW.preparation_id)='lazy' OR EXISTS(SELECT 1 FROM lazy_recipients WHERE preparation_id=NEW.preparation_id)) AND NEW.message_id IS NOT CASE WHEN substr(NEW.preparation_id,1,5)='prep-' AND length(NEW.preparation_id)>5 THEN 'msg-'||substr(NEW.preparation_id,6) WHEN length(NEW.preparation_id)=12 AND substr(NEW.preparation_id,1,1)='p' AND substr(NEW.preparation_id,2) NOT GLOB '*[^A-Za-z0-9]*' THEN 'm'||substr(NEW.preparation_id,2) ELSE NULL END)
        OR (NEW.warning_count<>0 AND (SELECT priority FROM messages WHERE id=NEW.message_id)>=1 AND NOT EXISTS(SELECT 1 FROM prepared_recipients WHERE preparation_id=NEW.preparation_id));
END;

CREATE TRIGGER bootstrap_attempt_identity BEFORE UPDATE ON bootstrap_attempts
WHEN NEW.parent_id!=OLD.parent_id OR NEW.attempt!=OLD.attempt OR NEW.reserve_key!=OLD.reserve_key
 OR NEW.record_key!=OLD.record_key OR NEW.check_key!=OLD.check_key OR NEW.not_submitted_key!=OLD.not_submitted_key
 OR (OLD.creation_json IS NOT NULL AND NEW.creation_json IS NOT OLD.creation_json)
BEGIN SELECT RAISE(ABORT,'immutable bootstrap attempt identity'); END;

CREATE TRIGGER bootstrap_attempt_retained BEFORE DELETE ON bootstrap_attempts
BEGIN SELECT RAISE(ABORT,'bootstrap attempts retained'); END;

CREATE TRIGGER bootstrap_attempt_keys AFTER INSERT ON bootstrap_attempts BEGIN
 INSERT INTO bootstrap_child_keys SELECT NEW.parent_id,instance_id,state_dir,host_endpoint,actor_scope,NEW.reserve_key,'reserve',NEW.attempt FROM bootstrap_handoffs WHERE id=NEW.parent_id;
 INSERT INTO bootstrap_child_keys SELECT NEW.parent_id,instance_id,state_dir,host_endpoint,actor_scope,NEW.record_key,'record',NEW.attempt FROM bootstrap_handoffs WHERE id=NEW.parent_id;
 INSERT INTO bootstrap_child_keys SELECT NEW.parent_id,instance_id,state_dir,host_endpoint,actor_scope,NEW.check_key,'check',NEW.attempt FROM bootstrap_handoffs WHERE id=NEW.parent_id;
 INSERT INTO bootstrap_child_keys SELECT NEW.parent_id,instance_id,state_dir,host_endpoint,actor_scope,NEW.not_submitted_key,'not_submitted',NEW.attempt FROM bootstrap_handoffs WHERE id=NEW.parent_id;
END;

CREATE TRIGGER bootstrap_keys_immutable BEFORE UPDATE ON bootstrap_child_keys
BEGIN SELECT RAISE(ABORT,'immutable bootstrap child key'); END;

CREATE TRIGGER bootstrap_keys_retained BEFORE DELETE ON bootstrap_child_keys
BEGIN SELECT RAISE(ABORT,'bootstrap child keys retained'); END;

CREATE TRIGGER bootstrap_attachment_immutable BEFORE UPDATE ON bootstrap_attachments
BEGIN SELECT RAISE(ABORT,'immutable bootstrap attachment'); END;

CREATE TRIGGER bootstrap_attachment_retained BEFORE DELETE ON bootstrap_attachments
BEGIN SELECT RAISE(ABORT,'bootstrap attachment retained'); END;

CREATE TRIGGER bootstrap_report_immutable BEFORE UPDATE ON bootstrap_reports
BEGIN SELECT RAISE(ABORT,'immutable bootstrap completion'); END;

CREATE TRIGGER bootstrap_report_retained BEFORE DELETE ON bootstrap_reports
BEGIN SELECT RAISE(ABORT,'bootstrap completion retained'); END;

CREATE TRIGGER automatic_handoff_keys_immutable BEFORE UPDATE ON automatic_handoff_keys
BEGIN SELECT RAISE(ABORT,'immutable automatic handoff key'); END;

CREATE TRIGGER automatic_handoff_keys_retained BEFORE DELETE ON automatic_handoff_keys
BEGIN SELECT RAISE(ABORT,'automatic handoff keys retained'); END;

CREATE TRIGGER automatic_handoff_attachment BEFORE UPDATE OF thread_id ON channel_handoff_fences
WHEN OLD.thread_id IS NOT NULL AND NEW.thread_id IS NOT OLD.thread_id
 AND EXISTS(SELECT 1 FROM automatic_handoff_companions WHERE instance_id=OLD.instance_id AND actor_scope=OLD.actor_scope AND compound=OLD.compound)
BEGIN SELECT RAISE(ABORT,'immutable automatic handoff attachment'); END;

CREATE TRIGGER automatic_bootstrap_keys_immutable BEFORE UPDATE ON automatic_bootstrap_keys
BEGIN SELECT RAISE(ABORT,'immutable automatic bootstrap keys'); END;

CREATE TRIGGER automatic_bootstrap_keys_retained BEFORE DELETE ON automatic_bootstrap_keys
BEGIN SELECT RAISE(ABORT,'automatic bootstrap keys retained'); END;

CREATE TRIGGER automatic_bootstrap_attempt_identity BEFORE UPDATE ON automatic_bootstrap_attempts
WHEN NEW.parent_id IS NOT OLD.parent_id OR NEW.attempt IS NOT OLD.attempt OR (OLD.creation_json IS NOT NULL AND NEW.creation_json IS NOT OLD.creation_json)
BEGIN SELECT RAISE(ABORT,'immutable automatic creation evidence'); END;

CREATE TRIGGER automatic_bootstrap_attempts_retained BEFORE DELETE ON automatic_bootstrap_attempts
BEGIN SELECT RAISE(ABORT,'automatic attempts retained'); END;

CREATE TRIGGER automatic_bootstrap_report_immutable BEFORE UPDATE ON automatic_bootstrap_reports
BEGIN SELECT RAISE(ABORT,'immutable automatic report'); END;

CREATE TRIGGER automatic_bootstrap_report_retained BEFORE DELETE ON automatic_bootstrap_reports
BEGIN SELECT RAISE(ABORT,'automatic report retained'); END;

CREATE TRIGGER archival_fence_occupant_bindings_delete AFTER DELETE ON occupant_bindings BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_binding_activity AFTER UPDATE OF registered_at,ended_at,generation,harness,native_session,execution_id,target_id,host_boot,host_epoch,observation_provenance ON occupant_bindings BEGIN
 UPDATE seat_archival SET activity_revision=activity_revision+1,idle_mono=NULL,samples=0,next_mono=0 WHERE seat_id=NEW.seat_id;
END;

CREATE TRIGGER archival_binding_queue_insert AFTER INSERT ON occupant_bindings BEGIN
 UPDATE seat_archival SET next_mono=0 WHERE seat_id=NEW.seat_id;
END;

CREATE TRIGGER archival_binding_queue_delete AFTER DELETE ON occupant_bindings BEGIN
 UPDATE seat_archival SET next_mono=0 WHERE seat_id=OLD.seat_id;
END;

CREATE TRIGGER archival_fence_seats_update AFTER UPDATE OF state,retired_at,target_id ON seats
WHEN EXISTS(SELECT 1 FROM memberships WHERE seat_id=NEW.id AND state='joined')
  OR EXISTS(SELECT 1 FROM invitations WHERE seat_id=NEW.id AND state='pending')
  OR EXISTS(SELECT 1 FROM receipts WHERE seat_id=NEW.id AND state='pending')
  OR EXISTS(SELECT 1 FROM prepared_recipients WHERE seat_id=NEW.id)
BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1 WHERE instance_id=NEW.instance_id;
END;

CREATE TRIGGER archival_fence_memberships_update AFTER UPDATE OF state ON memberships BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_occupant_bindings_insert AFTER INSERT ON occupant_bindings
WHEN EXISTS(SELECT 1 FROM memberships WHERE seat_id=NEW.seat_id AND state='joined')
  OR EXISTS(SELECT 1 FROM invitations WHERE seat_id=NEW.seat_id AND state='pending')
  OR EXISTS(SELECT 1 FROM receipts WHERE seat_id=NEW.seat_id AND state='pending')
  OR EXISTS(SELECT 1 FROM prepared_recipients WHERE seat_id=NEW.seat_id)
BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1 WHERE instance_id=(SELECT instance_id FROM seats WHERE id=NEW.seat_id);
END;

CREATE TRIGGER archival_fence_occupant_bindings_update AFTER UPDATE OF liveness,ended_at ON occupant_bindings
WHEN EXISTS(SELECT 1 FROM memberships WHERE seat_id=NEW.seat_id AND state='joined')
  OR EXISTS(SELECT 1 FROM invitations WHERE seat_id=NEW.seat_id AND state='pending')
  OR EXISTS(SELECT 1 FROM receipts WHERE seat_id=NEW.seat_id AND state='pending')
  OR EXISTS(SELECT 1 FROM prepared_recipients WHERE seat_id=NEW.seat_id)
BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1 WHERE instance_id=(SELECT instance_id FROM seats WHERE id=NEW.seat_id);
END;

CREATE TRIGGER archival_fence_catch_up_insert AFTER INSERT ON catch_up BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_catch_up_update AFTER UPDATE ON catch_up BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_catch_up_delete AFTER DELETE ON catch_up BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER invitations_confirmation_origin BEFORE INSERT ON invitations
WHEN NEW.confirmation IS NOT NULL AND NEW.inviter_seat_id IS NULL
BEGIN SELECT RAISE(ABORT, 'modern invitation requires original inviter'); END;

CREATE TRIGGER invitations_inviter_required BEFORE INSERT ON invitations
WHEN NEW.inviter_seat_id IS NULL AND NEW.inviter_kind='seat'
  AND (SELECT journaled FROM threads WHERE id=NEW.thread_id)=1
BEGIN SELECT RAISE(ABORT, 'journaled invitation requires its inviter seat'); END;

CREATE TRIGGER invitations_confirmation_immutable BEFORE UPDATE OF confirmation,inviter_seat_id ON invitations
WHEN NEW.confirmation IS NOT OLD.confirmation OR NEW.inviter_seat_id IS NOT OLD.inviter_seat_id
BEGIN SELECT RAISE(ABORT, 'invitation policy and origin are immutable'); END;

CREATE TRIGGER archival_fence_receipts_delete AFTER DELETE ON receipts BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_receipts_insert AFTER INSERT ON receipts BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_receipts_update AFTER UPDATE ON receipts BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER digest_physical_receipt_settled AFTER UPDATE OF state ON receipts WHEN NEW.state!='pending' AND NOT EXISTS (SELECT 1 FROM send_manifests sm JOIN prepared_recipients pr ON pr.preparation_id=sm.preparation_id WHERE sm.message_id=NEW.message_id AND pr.seat_id=NEW.seat_id)
BEGIN
    DELETE FROM digest_open_warnings WHERE condition_kind='receipt' AND condition_id=length(CAST(NEW.message_id AS BLOB))||':'||NEW.message_id||':'||NEW.seat_id;
END;

CREATE TRIGGER human_receipt_physical_waived AFTER UPDATE OF ack_required ON receipts WHEN NEW.ack_required=0
BEGIN
    DELETE FROM digest_open_warnings WHERE condition_kind='receipt' AND condition_id=length(CAST(NEW.message_id AS BLOB))||':'||NEW.message_id||':'||NEW.seat_id;
END;

CREATE TRIGGER receipts_ordinal_immutable BEFORE UPDATE OF ordinal ON receipts
BEGIN SELECT RAISE(ABORT, 'receipt ordinal is immutable'); END;

CREATE TRIGGER wake_batches_clear_receipt
AFTER UPDATE OF state,ack_required ON receipts
WHEN OLD.state='pending' AND OLD.ack_required=1 AND (NEW.state!='pending' OR NEW.ack_required=0) AND NOT EXISTS(SELECT 1 FROM digest_pending_invitations WHERE seat_id=OLD.seat_id)
      AND NOT EXISTS(SELECT 1 FROM digest_pending_manifest_receipts WHERE seat_id=OLD.seat_id)
      AND NOT EXISTS(SELECT 1 FROM receipts WHERE seat_id=OLD.seat_id AND state='pending' AND ack_required=1)
BEGIN DELETE FROM wake_batches WHERE seat_id=OLD.seat_id; END;

CREATE TRIGGER archival_fence_receipt_state_delete AFTER DELETE ON receipt_state BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_receipt_state_insert AFTER INSERT ON receipt_state BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER archival_fence_receipt_state_update AFTER UPDATE ON receipt_state BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;

CREATE TRIGGER digest_manifest_receipt_materialized AFTER INSERT ON receipt_state WHEN NEW.state='pending'
BEGIN
    UPDATE digest_pending_manifest_receipts SET decision_seq=(SELECT decision_seq FROM send_manifests WHERE message_id=NEW.message_id) WHERE seat_id=NEW.seat_id AND preparation_id=(SELECT preparation_id FROM send_manifests WHERE message_id=NEW.message_id);
END;

CREATE TRIGGER digest_receipt_state_settled_insert AFTER INSERT ON receipt_state WHEN NEW.state!='pending'
BEGIN
    DELETE FROM digest_pending_manifest_receipts WHERE seat_id=NEW.seat_id AND preparation_id=(SELECT preparation_id FROM send_manifests WHERE message_id=NEW.message_id);
    DELETE FROM digest_open_warnings WHERE condition_kind='receipt' AND condition_id=length(CAST(NEW.message_id AS BLOB))||':'||NEW.message_id||':'||NEW.seat_id;
END;

CREATE TRIGGER digest_receipt_state_settled_update AFTER UPDATE OF state ON receipt_state WHEN NEW.state!='pending'
BEGIN
    DELETE FROM digest_pending_manifest_receipts WHERE seat_id=NEW.seat_id AND preparation_id=(SELECT preparation_id FROM send_manifests WHERE message_id=NEW.message_id);
    DELETE FROM digest_open_warnings WHERE condition_kind='receipt' AND condition_id=length(CAST(NEW.message_id AS BLOB))||':'||NEW.message_id||':'||NEW.seat_id;
END;

CREATE TRIGGER human_receipt_state_insert_waived AFTER INSERT ON receipt_state WHEN NEW.ack_required=0
BEGIN
    DELETE FROM digest_pending_manifest_receipts WHERE seat_id=NEW.seat_id AND preparation_id=(SELECT preparation_id FROM send_manifests WHERE message_id=NEW.message_id);
    DELETE FROM digest_open_warnings WHERE condition_kind='receipt' AND condition_id=length(CAST(NEW.message_id AS BLOB))||':'||NEW.message_id||':'||NEW.seat_id;
END;

CREATE TRIGGER human_receipt_state_waived AFTER UPDATE OF ack_required ON receipt_state WHEN NEW.ack_required=0
BEGIN
    DELETE FROM digest_pending_manifest_receipts WHERE seat_id=NEW.seat_id AND preparation_id=(SELECT preparation_id FROM send_manifests WHERE message_id=NEW.message_id);
    DELETE FROM digest_open_warnings WHERE condition_kind='receipt' AND condition_id=length(CAST(NEW.message_id AS BLOB))||':'||NEW.message_id||':'||NEW.seat_id;
END;

CREATE TRIGGER bootstrap_identity BEFORE UPDATE ON bootstrap_handoffs
WHEN NEW.id!=OLD.id OR NEW.instance_id!=OLD.instance_id OR NEW.state_dir!=OLD.state_dir
 OR NEW.host_endpoint!=OLD.host_endpoint OR NEW.actor_scope!=OLD.actor_scope OR NEW.compound!=OLD.compound
 OR NEW.digest!=OLD.digest OR NEW.identity_json!=OLD.identity_json OR NEW.create_key!=OLD.create_key
 OR NEW.original_thread IS NOT OLD.original_thread OR NEW.created_at!=OLD.created_at
BEGIN SELECT RAISE(ABORT,'immutable bootstrap identity'); END;

CREATE TRIGGER bootstrap_terminal BEFORE UPDATE ON bootstrap_handoffs
WHEN OLD.state IN ('completed','cancelled') AND (NEW.state!=OLD.state OR NEW.current_attempt!=OLD.current_attempt
 OR NEW.thread_id IS NOT OLD.thread_id OR NEW.administrative_revision!=OLD.administrative_revision
 OR NEW.latest_recovery_operation IS NOT OLD.latest_recovery_operation OR NEW.terminal_at IS NOT OLD.terminal_at)
BEGIN SELECT RAISE(ABORT,'terminal bootstrap is absorbing'); END;

CREATE TRIGGER bootstrap_retained BEFORE DELETE ON bootstrap_handoffs
BEGIN SELECT RAISE(ABORT,'bootstrap identity retained'); END;

CREATE TRIGGER bootstrap_recovery_key AFTER INSERT ON bootstrap_recovery_decisions BEGIN
 INSERT INTO bootstrap_child_keys SELECT NEW.parent_id,instance_id,state_dir,host_endpoint,actor_scope,NEW.operation,CASE NEW.decision_kind WHEN 'cancellation' THEN 'cancel' ELSE 'recovery' END,NEW.attempt FROM bootstrap_handoffs WHERE id=NEW.parent_id;
END;

CREATE TRIGGER bootstrap_recovery_immutable BEFORE UPDATE ON bootstrap_recovery_decisions
BEGIN SELECT RAISE(ABORT,'immutable bootstrap recovery decision'); END;

CREATE TRIGGER bootstrap_recovery_retained BEFORE DELETE ON bootstrap_recovery_decisions
BEGIN SELECT RAISE(ABORT,'bootstrap recovery decisions retained'); END;

CREATE TRIGGER warning_conditions_invitation_origin_immutable BEFORE UPDATE OF invitation_originator_instance_id,invitation_originator_seat_id ON warning_conditions
WHEN NEW.invitation_originator_instance_id IS NOT OLD.invitation_originator_instance_id OR NEW.invitation_originator_seat_id IS NOT OLD.invitation_originator_seat_id
BEGIN SELECT RAISE(ABORT, 'invitation warning origin is immutable'); END;

CREATE TRIGGER receipt_warning_origin_attention AFTER INSERT ON receipt_warning_origins
 BEGIN
 INSERT INTO warning_recipients(warning_id,seat_id,generation)
 SELECT NEW.warning_id,s.id,m.decision_seq FROM seats s JOIN messages m ON m.id=NEW.warning_id
 WHERE s.id=NEW.seat_id AND s.instance_id=NEW.instance_id AND s.state!='retired' ON CONFLICT(warning_id,seat_id) DO NOTHING;
 INSERT INTO wake_work(seat_id,reason_bits,attention_version)
 SELECT id,4,1 FROM seats WHERE id=NEW.seat_id AND instance_id=NEW.instance_id AND state!='retired'
 ON CONFLICT(seat_id) DO UPDATE SET reason_bits=reason_bits|4,attention_version=attention_version+1;
 END;

CREATE TRIGGER receipt_warning_origin_receipts_insert AFTER INSERT ON receipts
 WHEN NEW.warning_message_id IS NOT NULL
 BEGIN
 INSERT INTO receipt_warning_origins(instance_id,seat_id,warning_id,source_message_id)
 SELECT m.instance_id,m.actor_seat_id,c.open_warning_id,m.id FROM receipts r
 JOIN messages m ON m.id=r.message_id AND (m.author_kind='native' OR m.author_kind IS NULL) AND m.author_service_id IS NULL
 JOIN seats s ON s.id=m.actor_seat_id AND s.instance_id=m.instance_id
 JOIN warning_conditions c ON c.open_warning_id=r.warning_message_id
    AND c.condition_kind='receipt' AND c.clear_warning_id IS NULL
    AND c.thread_id=m.thread_id AND c.affected_seat_id=r.seat_id
 WHERE r.message_id=NEW.message_id AND r.seat_id=NEW.seat_id ON CONFLICT(warning_id,seat_id) DO NOTHING;
 END;

CREATE TRIGGER receipt_warning_origin_receipts_update AFTER UPDATE OF warning_message_id ON receipts
 WHEN NEW.warning_message_id IS NOT NULL
 BEGIN
 INSERT INTO receipt_warning_origins(instance_id,seat_id,warning_id,source_message_id)
 SELECT m.instance_id,m.actor_seat_id,c.open_warning_id,m.id FROM receipts r
 JOIN messages m ON m.id=r.message_id AND (m.author_kind='native' OR m.author_kind IS NULL) AND m.author_service_id IS NULL
 JOIN seats s ON s.id=m.actor_seat_id AND s.instance_id=m.instance_id
 JOIN warning_conditions c ON c.open_warning_id=r.warning_message_id
    AND c.condition_kind='receipt' AND c.clear_warning_id IS NULL
    AND c.thread_id=m.thread_id AND c.affected_seat_id=r.seat_id
 WHERE r.message_id=NEW.message_id AND r.seat_id=NEW.seat_id ON CONFLICT(warning_id,seat_id) DO NOTHING;
 END;

CREATE TRIGGER receipt_warning_origin_receipt_state_insert AFTER INSERT ON receipt_state
 WHEN NEW.warning_message_id IS NOT NULL
 BEGIN
 INSERT INTO receipt_warning_origins(instance_id,seat_id,warning_id,source_message_id)
 SELECT m.instance_id,m.actor_seat_id,c.open_warning_id,m.id FROM receipt_state r
 JOIN messages m ON m.id=r.message_id AND (m.author_kind='native' OR m.author_kind IS NULL) AND m.author_service_id IS NULL
 JOIN send_manifests sm ON sm.message_id=r.message_id AND sm.instance_id=m.instance_id AND sm.thread_id=m.thread_id
 JOIN prepared_recipients pr ON pr.preparation_id=sm.preparation_id AND pr.seat_id=r.seat_id AND pr.thread_id=m.thread_id
 JOIN seats s ON s.id=m.actor_seat_id AND s.instance_id=m.instance_id
 JOIN warning_conditions c ON c.open_warning_id=r.warning_message_id
    AND c.condition_kind='receipt' AND c.clear_warning_id IS NULL
    AND c.thread_id=m.thread_id AND c.affected_seat_id=r.seat_id
 WHERE r.message_id=NEW.message_id AND r.seat_id=NEW.seat_id ON CONFLICT(warning_id,seat_id) DO NOTHING;
 END;

CREATE TRIGGER receipt_warning_origin_receipt_state_update AFTER UPDATE OF warning_message_id ON receipt_state
 WHEN NEW.warning_message_id IS NOT NULL
 BEGIN
 INSERT INTO receipt_warning_origins(instance_id,seat_id,warning_id,source_message_id)
 SELECT m.instance_id,m.actor_seat_id,c.open_warning_id,m.id FROM receipt_state r
 JOIN messages m ON m.id=r.message_id AND (m.author_kind='native' OR m.author_kind IS NULL) AND m.author_service_id IS NULL
 JOIN send_manifests sm ON sm.message_id=r.message_id AND sm.instance_id=m.instance_id AND sm.thread_id=m.thread_id
 JOIN prepared_recipients pr ON pr.preparation_id=sm.preparation_id AND pr.seat_id=r.seat_id AND pr.thread_id=m.thread_id
 JOIN seats s ON s.id=m.actor_seat_id AND s.instance_id=m.instance_id
 JOIN warning_conditions c ON c.open_warning_id=r.warning_message_id
    AND c.condition_kind='receipt' AND c.clear_warning_id IS NULL
    AND c.thread_id=m.thread_id AND c.affected_seat_id=r.seat_id
 WHERE r.message_id=NEW.message_id AND r.seat_id=NEW.seat_id ON CONFLICT(warning_id,seat_id) DO NOTHING;
 END;

CREATE TRIGGER receipt_warning_origin_condition AFTER INSERT ON warning_conditions
 WHEN NEW.condition_kind='receipt' AND NEW.clear_warning_id IS NULL
 BEGIN
INSERT INTO receipt_warning_origins(instance_id,seat_id,warning_id,source_message_id)
SELECT m.instance_id,m.actor_seat_id,c.open_warning_id,m.id FROM receipts r
 JOIN messages m ON m.id=r.message_id AND (m.author_kind='native' OR m.author_kind IS NULL) AND m.author_service_id IS NULL
 JOIN seats s ON s.id=m.actor_seat_id AND s.instance_id=m.instance_id
 JOIN warning_conditions c ON c.open_warning_id=r.warning_message_id
    AND c.condition_kind='receipt' AND c.clear_warning_id IS NULL
    AND c.thread_id=m.thread_id AND c.affected_seat_id=r.seat_id
 WHERE r.message_id=(SELECT source_message_id FROM messages WHERE id=NEW.open_warning_id) AND r.seat_id=NEW.affected_seat_id ON CONFLICT(warning_id,seat_id) DO NOTHING;
INSERT INTO receipt_warning_origins(instance_id,seat_id,warning_id,source_message_id)
SELECT m.instance_id,m.actor_seat_id,c.open_warning_id,m.id FROM receipt_state r
 JOIN messages m ON m.id=r.message_id AND (m.author_kind='native' OR m.author_kind IS NULL) AND m.author_service_id IS NULL
 JOIN send_manifests sm ON sm.message_id=r.message_id AND sm.instance_id=m.instance_id AND sm.thread_id=m.thread_id
 JOIN prepared_recipients pr ON pr.preparation_id=sm.preparation_id AND pr.seat_id=r.seat_id AND pr.thread_id=m.thread_id
 JOIN seats s ON s.id=m.actor_seat_id AND s.instance_id=m.instance_id
 JOIN warning_conditions c ON c.open_warning_id=r.warning_message_id
    AND c.condition_kind='receipt' AND c.clear_warning_id IS NULL
    AND c.thread_id=m.thread_id AND c.affected_seat_id=r.seat_id
 WHERE r.message_id=(SELECT source_message_id FROM messages WHERE id=NEW.open_warning_id) AND r.seat_id=NEW.affected_seat_id ON CONFLICT(warning_id,seat_id) DO NOTHING;
END;

CREATE TRIGGER receipt_warning_origin_clear AFTER UPDATE OF clear_warning_id ON warning_conditions
 WHEN NEW.condition_kind='receipt' AND NEW.clear_warning_id IS NOT NULL
 BEGIN
 INSERT INTO warning_recipients(warning_id,seat_id,generation)
 SELECT NEW.clear_warning_id,o.seat_id,NEW.cleared_seq FROM receipt_warning_origins o
 JOIN seats s ON s.id=o.seat_id AND s.state!='retired' WHERE o.warning_id=NEW.open_warning_id ON CONFLICT(warning_id,seat_id) DO NOTHING;
 DELETE FROM receipt_warning_origins WHERE warning_id=NEW.open_warning_id;
 END;

CREATE TRIGGER receipt_due_sparse_insert AFTER INSERT ON receipt_state
 BEGIN
 DELETE FROM receipt_due_sparse WHERE message_id=NEW.message_id AND seat_id=NEW.seat_id
  AND NOT (NEW.state='pending' AND NEW.ack_required=1 AND NEW.deadline_at IS NOT NULL AND NEW.warning_message_id IS NULL);
 INSERT INTO receipt_due_sparse(message_id,seat_id,deadline_at)
 SELECT NEW.message_id,NEW.seat_id,NEW.deadline_at
 WHERE NEW.state='pending' AND NEW.ack_required=1 AND NEW.deadline_at IS NOT NULL AND NEW.warning_message_id IS NULL
  AND COALESCE((SELECT origin_node FROM messages WHERE id=NEW.message_id),(SELECT id FROM local_node))=(SELECT id FROM local_node)
 ON CONFLICT(message_id,seat_id) DO UPDATE SET deadline_at=excluded.deadline_at;
 END;

CREATE TRIGGER receipt_due_sparse_update AFTER UPDATE OF state,ack_required,deadline_at,warning_message_id ON receipt_state
 BEGIN
 DELETE FROM receipt_due_sparse WHERE message_id=NEW.message_id AND seat_id=NEW.seat_id
  AND NOT (NEW.state='pending' AND NEW.ack_required=1 AND NEW.deadline_at IS NOT NULL AND NEW.warning_message_id IS NULL);
 INSERT INTO receipt_due_sparse(message_id,seat_id,deadline_at)
 SELECT NEW.message_id,NEW.seat_id,NEW.deadline_at
 WHERE NEW.state='pending' AND NEW.ack_required=1 AND NEW.deadline_at IS NOT NULL AND NEW.warning_message_id IS NULL
  AND COALESCE((SELECT origin_node FROM messages WHERE id=NEW.message_id),(SELECT id FROM local_node))=(SELECT id FROM local_node)
 ON CONFLICT(message_id,seat_id) DO UPDATE SET deadline_at=excluded.deadline_at;
 END;

CREATE TRIGGER receipt_due_sparse_delete AFTER DELETE ON receipt_state
 BEGIN DELETE FROM receipt_due_sparse WHERE message_id=OLD.message_id AND seat_id=OLD.seat_id; END;

CREATE TRIGGER automatic_handoff_immutable BEFORE UPDATE ON automatic_handoff_companions
BEGIN SELECT RAISE(ABORT,'immutable automatic handoff original'); END;

CREATE TRIGGER automatic_handoff_retained BEFORE DELETE ON automatic_handoff_companions
BEGIN SELECT RAISE(ABORT,'automatic handoff originals retained'); END;

CREATE TRIGGER automatic_bootstrap_original BEFORE UPDATE OF instance_id,actor_scope,compound,state_dir,host_endpoint,original_json ON automatic_bootstrap_parents
BEGIN SELECT RAISE(ABORT,'immutable automatic bootstrap original'); END;

CREATE TRIGGER automatic_bootstrap_terminal BEFORE UPDATE ON automatic_bootstrap_parents
WHEN (OLD.state IN ('completed','cancelled') AND NEW.state IS NOT OLD.state) OR (OLD.thread_id IS NOT NULL AND NEW.thread_id IS NOT OLD.thread_id)
BEGIN SELECT RAISE(ABORT,'absorbing automatic bootstrap attachment'); END;

CREATE TRIGGER automatic_bootstrap_retained BEFORE DELETE ON automatic_bootstrap_parents
BEGIN SELECT RAISE(ABORT,'automatic bootstrap retained'); END;

CREATE TRIGGER automatic_bootstrap_attachment_immutable BEFORE UPDATE ON automatic_bootstrap_attachments
BEGIN SELECT RAISE(ABORT,'immutable automatic attachment'); END;

CREATE TRIGGER automatic_bootstrap_attachment_retained BEFORE DELETE ON automatic_bootstrap_attachments
BEGIN SELECT RAISE(ABORT,'automatic attachment retained'); END;

CREATE TRIGGER automatic_bootstrap_recovery_immutable BEFORE UPDATE ON automatic_bootstrap_recoveries
BEGIN SELECT RAISE(ABORT,'immutable automatic recovery'); END;

CREATE TRIGGER automatic_bootstrap_recovery_retained BEFORE DELETE ON automatic_bootstrap_recoveries
BEGIN SELECT RAISE(ABORT,'automatic recovery retained'); END;

CREATE TRIGGER invitation_context_instance_insert BEFORE INSERT ON invitation_context_latest
WHEN NOT EXISTS (SELECT 1 FROM threads t WHERE t.id=NEW.thread_id AND t.instance_id=NEW.instance_id)
BEGIN SELECT RAISE(ABORT, 'invitation context instance mismatch'); END;

CREATE TRIGGER invitation_context_instance_update BEFORE UPDATE ON invitation_context_latest
WHEN NEW.thread_id IS NOT OLD.thread_id OR NEW.instance_id IS NOT OLD.instance_id
BEGIN SELECT RAISE(ABORT, 'invitation context identity immutable'); END;

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

CREATE TRIGGER presentation_warning_proof_reclaim AFTER DELETE ON invitation_presentation_consumers
BEGIN DELETE FROM presentation_warning_proofs WHERE binding_id=OLD.binding_id AND consumer=OLD.consumer; END;

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

-- Message delivery priority (ht-yha, folded from v47; reconcile C8).

CREATE TRIGGER preparation_priority_immutable BEFORE UPDATE OF priority ON send_preparations
WHEN NEW.priority IS NOT OLD.priority
BEGIN SELECT RAISE(ABORT, 'preparation priority is immutable'); END;

CREATE TRIGGER lazy_recipient_priority_immutable BEFORE UPDATE OF priority ON lazy_recipients
WHEN NEW.priority IS NOT OLD.priority
BEGIN SELECT RAISE(ABORT, 'delivery row priority is immutable'); END;

CREATE TRIGGER lazy_recipient_decision_seq_once BEFORE UPDATE OF decision_seq ON lazy_recipients
WHEN OLD.decision_seq IS NOT NULL AND NEW.decision_seq IS NOT OLD.decision_seq
BEGIN SELECT RAISE(ABORT, 'delivery row publication is immutable'); END;

CREATE TABLE binding_row_displays (
    binding_id TEXT NOT NULL REFERENCES occupant_bindings(binding_id),
    message_id TEXT NOT NULL CHECK(length(CAST(message_id AS BLOB)) BETWEEN 1 AND 128),
    PRIMARY KEY(binding_id, message_id)
) STRICT;

CREATE TABLE binding_pending (
    binding_id TEXT NOT NULL REFERENCES occupant_bindings(binding_id),
    ordinal INTEGER NOT NULL REFERENCES lazy_recipients(ordinal),
    thread_id TEXT NOT NULL,
    priority INTEGER NOT NULL CHECK(priority BETWEEN 0 AND 9),
    decision_seq INTEGER NOT NULL CHECK(decision_seq>0),
    PRIMARY KEY(binding_id, ordinal)
) STRICT;

CREATE INDEX binding_pending_order ON binding_pending(binding_id,priority,decision_seq,ordinal);

CREATE INDEX binding_pending_thread_order ON binding_pending(binding_id,thread_id,priority,decision_seq,ordinal);

CREATE INDEX lazy_recipients_legacy_order ON lazy_recipients(seat_id,priority,decision_seq,ordinal) WHERE legacy_pending=1 AND decision_seq IS NOT NULL;

CREATE TRIGGER binding_pending_publish AFTER UPDATE OF decision_seq ON lazy_recipients
WHEN OLD.decision_seq IS NULL AND NEW.decision_seq IS NOT NULL
BEGIN
    INSERT OR IGNORE INTO binding_pending(binding_id,ordinal,thread_id,priority,decision_seq)
    SELECT b.binding_id,NEW.ordinal,NEW.thread_id,NEW.priority,NEW.decision_seq FROM occupant_bindings b
    WHERE b.seat_id=NEW.seat_id AND (b.ended_at IS NULL AND b.liveness='connected')
      AND (EXISTS(SELECT 1 FROM binding_cursors lc WHERE lc.binding_id=b.binding_id AND lc.thread_id=NEW.thread_id) OR EXISTS(SELECT 1 FROM memberships lm WHERE lm.thread_id=NEW.thread_id AND lm.seat_id=NEW.seat_id AND (lm.state='joined' OR (lm.state='invited' AND COALESCE(lm.voluntary_state,lm.state)='invited' AND EXISTS(SELECT 1 FROM direct_threads ld JOIN seats ls ON ls.id=NEW.seat_id JOIN invitations li ON li.thread_id=ld.thread_id AND li.seat_id=ls.id AND li.episode=lm.episode WHERE ld.thread_id=NEW.thread_id AND ld.instance_id=(SELECT si.instance_id FROM seats si WHERE si.id=NEW.seat_id) AND (ls.id=ld.seat_a OR ls.id=ld.seat_b) AND ls.instance_id=(SELECT si.instance_id FROM seats si WHERE si.id=NEW.seat_id) AND ls.state<>'retired' AND ls.retired_at IS NULL AND li.state='pending' AND NEW.decision_seq>li.created_decision_seq)))));
END;

CREATE TRIGGER binding_pending_legacy_set AFTER UPDATE OF legacy_pending ON lazy_recipients
WHEN OLD.legacy_pending=0 AND NEW.legacy_pending=1 AND NEW.decision_seq IS NOT NULL
BEGIN
    INSERT OR IGNORE INTO binding_pending(binding_id,ordinal,thread_id,priority,decision_seq)
    SELECT b.binding_id,NEW.ordinal,NEW.thread_id,NEW.priority,NEW.decision_seq FROM occupant_bindings b
    WHERE b.seat_id=NEW.seat_id AND (b.ended_at IS NULL AND b.liveness='connected') AND b.role='top_level';
END;

CREATE TRIGGER binding_pending_legacy_clear AFTER UPDATE OF legacy_pending ON lazy_recipients
WHEN OLD.legacy_pending=1 AND NEW.legacy_pending=0
BEGIN
    DELETE FROM binding_pending WHERE ordinal=NEW.ordinal
      AND (binding_id IN (SELECT c.binding_id FROM binding_cursors c WHERE c.thread_id=NEW.thread_id AND c.seq>=(SELECT m.sequence FROM messages m WHERE m.id=NEW.message_id))
        OR binding_id IN (SELECT d.binding_id FROM binding_row_displays d WHERE d.message_id=NEW.message_id));
END;

CREATE TRIGGER binding_pending_cursor_insert AFTER INSERT ON binding_cursors
BEGIN
    DELETE FROM binding_pending WHERE binding_id=NEW.binding_id AND thread_id=NEW.thread_id
      AND (SELECT m.sequence FROM lazy_recipients r JOIN messages m ON m.id=r.message_id WHERE r.ordinal=binding_pending.ordinal)<=NEW.seq
      AND NOT EXISTS(SELECT 1 FROM lazy_recipients r JOIN occupant_bindings ob ON ob.binding_id=NEW.binding_id WHERE r.ordinal=binding_pending.ordinal AND r.legacy_pending=1 AND ob.role='top_level');
END;

CREATE TRIGGER binding_pending_cursor_advance AFTER UPDATE OF seq ON binding_cursors
WHEN NEW.seq>OLD.seq
BEGIN
    DELETE FROM binding_pending WHERE binding_id=NEW.binding_id AND thread_id=NEW.thread_id
      AND (SELECT m.sequence FROM lazy_recipients r JOIN messages m ON m.id=r.message_id WHERE r.ordinal=binding_pending.ordinal)<=NEW.seq
      AND NOT EXISTS(SELECT 1 FROM lazy_recipients r JOIN occupant_bindings ob ON ob.binding_id=NEW.binding_id WHERE r.ordinal=binding_pending.ordinal AND r.legacy_pending=1 AND ob.role='top_level');
END;

CREATE TRIGGER binding_pending_row_display AFTER INSERT ON binding_row_displays
BEGIN
    DELETE FROM binding_pending WHERE binding_id=NEW.binding_id AND ordinal IN (
        SELECT r.ordinal FROM lazy_recipients r JOIN occupant_bindings ob ON ob.binding_id=NEW.binding_id AND ob.seat_id=r.seat_id
        WHERE r.message_id=NEW.message_id AND NOT (r.legacy_pending=1 AND ob.role='top_level'));
END;

CREATE TRIGGER binding_pending_binding_end AFTER UPDATE OF ended_at ON occupant_bindings
WHEN OLD.ended_at IS NULL AND NEW.ended_at IS NOT NULL
BEGIN
    DELETE FROM binding_pending WHERE binding_id=NEW.binding_id;
END;

-- Health reads only open holds in each host's current recovery epoch,
-- independently of released targets and open holds retained from previous
-- incarnations (main schema 48, folded in place).
CREATE INDEX recovery_holds_current_open ON recovery_holds(host_id,baseline_boot,baseline_epoch,target_id) WHERE released_at IS NULL;

INSERT INTO "schema_identity"("marker") VALUES('herdr-threads-node-v1');
INSERT INTO "receipt_deadline_backfill"("singleton","physical_after","physical_high_water","prepared_after","prepared_high_water","completed") VALUES(1,0,0,0,0,1);
INSERT INTO "receipt_discovery_backfill"("singleton","physical_after","physical_high_water","sparse_after","sparse_high_water","completed") VALUES(1,0,0,0,0,1);
INSERT INTO "presentation_warning_fence"("singleton","version") VALUES(1,0);
