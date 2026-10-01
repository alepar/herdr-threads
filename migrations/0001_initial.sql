CREATE TABLE schema_identity (
    marker TEXT PRIMARY KEY CHECK(marker = 'herdr-threads-shared-v1-r7')
) STRICT;
INSERT INTO schema_identity(marker) VALUES ('herdr-threads-shared-v1-r7');

CREATE TABLE host_instances (
    id TEXT PRIMARY KEY,
    created_at INTEGER NOT NULL,
    host_boot TEXT,
    host_epoch INTEGER NOT NULL DEFAULT 0 CHECK(host_epoch >= 0),
    observation_sequence INTEGER NOT NULL DEFAULT 0 CHECK(observation_sequence >= 0),
    recovery_boot TEXT,
    recovery_epoch INTEGER,
    lifecycle_revision INTEGER NOT NULL DEFAULT 0 CHECK(lifecycle_revision >= 0),
    decision_seq INTEGER NOT NULL DEFAULT 0 CHECK(decision_seq >= 0),
    send_eligibility_revision INTEGER NOT NULL DEFAULT 0 CHECK(send_eligibility_revision >= 0),
    duration_config_revision INTEGER NOT NULL DEFAULT 0 CHECK(duration_config_revision >= 0),
    active_snapshot_id TEXT REFERENCES snapshot_generations(id),
    recovery_baseline_generation_id TEXT REFERENCES snapshot_generations(id),
    baseline_hold_unclaimed INTEGER NOT NULL DEFAULT 0 CHECK(baseline_hold_unclaimed IN (0,1)),
    invalidation_revision INTEGER NOT NULL DEFAULT 0 CHECK(invalidation_revision >= 0),
    observation_admission_sequence INTEGER NOT NULL DEFAULT 0 CHECK(observation_admission_sequence >= 0),
    observation_decided_sequence INTEGER NOT NULL DEFAULT 0 CHECK(observation_decided_sequence >= 0 AND observation_decided_sequence <= observation_admission_sequence)
) STRICT;

-- Each captured namespace is built invisibly, then published by one pointer
-- update. The first complete recovery generation remains the baseline even
-- after later active generations supersede it.
CREATE TABLE snapshot_generations (
    id TEXT PRIMARY KEY,
    instance_id TEXT NOT NULL REFERENCES host_instances(id),
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
    UNIQUE(instance_id,host_boot,epoch,observation_sequence),
    CHECK(status NOT IN ('sealed','published') OR staged_targets=expected_targets)
) STRICT;
CREATE INDEX snapshot_generations_status ON snapshot_generations(instance_id,status,id);
CREATE INDEX snapshot_generations_captured_active ON snapshot_generations(captured_active_id,status);

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
CREATE UNIQUE INDEX snapshot_targets_terminal ON snapshot_targets(generation_id,terminal_id) WHERE terminal_id IS NOT NULL;
CREATE INDEX snapshot_targets_generation_ordinal ON snapshot_targets(generation_id,ordinal);

CREATE TABLE recovery_baseline_releases (
    instance_id TEXT NOT NULL REFERENCES host_instances(id),
    baseline_generation_id TEXT NOT NULL REFERENCES snapshot_generations(id),
    target_id TEXT NOT NULL,
    decision_seq INTEGER NOT NULL CHECK(decision_seq > 0),
    PRIMARY KEY(instance_id,baseline_generation_id,target_id)
) STRICT;

CREATE TRIGGER host_active_snapshot_published BEFORE UPDATE OF active_snapshot_id ON host_instances
WHEN NEW.active_snapshot_id IS NOT NULL
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1 FROM snapshot_generations g
        WHERE g.id=NEW.active_snapshot_id AND g.instance_id=NEW.id
          AND g.status='published' AND g.staged_targets=g.expected_targets
    ) THEN RAISE(ABORT,'active snapshot is not complete and published') END;
END;
CREATE TRIGGER host_baseline_snapshot_published BEFORE UPDATE OF recovery_baseline_generation_id ON host_instances
WHEN NEW.recovery_baseline_generation_id IS NOT NULL
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1 FROM snapshot_generations g
        WHERE g.id=NEW.recovery_baseline_generation_id AND g.instance_id=NEW.id
          AND g.status='published' AND g.staged_targets=g.expected_targets
    ) THEN RAISE(ABORT,'baseline snapshot is not complete and published') END;
END;

-- Observation and repair state is independent of whether a durable seat exists.
CREATE TABLE observed_targets (
    instance_id TEXT NOT NULL REFERENCES host_instances(id),
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
    PRIMARY KEY(instance_id, target_id),
    CHECK((incarnation IS NULL AND incarnation_source_kind IS NULL AND connection_epoch IS NULL)
       OR (incarnation IS NOT NULL AND incarnation_source_kind IS NOT NULL AND connection_epoch IS NOT NULL))
) STRICT;
CREATE TABLE recovery_baseline_targets (
    instance_id TEXT NOT NULL REFERENCES host_instances(id),
    target_id TEXT NOT NULL,
    baseline_boot TEXT NOT NULL,
    baseline_epoch INTEGER NOT NULL CHECK(baseline_epoch >= 0),
    captured_at INTEGER NOT NULL,
    disposition TEXT NOT NULL CHECK(disposition IN (
        'unambiguous_unclaimed', 'created_after_baseline', 'held_for_repair', 'already_owned', 'unknown'
    )),
    PRIMARY KEY(instance_id, target_id)
) STRICT;
CREATE TABLE recovery_holds (
    instance_id TEXT NOT NULL REFERENCES host_instances(id),
    target_id TEXT NOT NULL,
    baseline_boot TEXT NOT NULL,
    baseline_epoch INTEGER NOT NULL CHECK(baseline_epoch >= 0),
    reason TEXT NOT NULL CHECK(length(reason) <= 512),
    released_at INTEGER,
    PRIMARY KEY(instance_id, target_id)
) STRICT;
CREATE TABLE allocation_decisions (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    instance_id TEXT NOT NULL REFERENCES host_instances(id),
    target_id TEXT NOT NULL,
    seat_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('ordinary', 'operator_fresh', 'operator_rebind')),
    decided_at INTEGER NOT NULL,
    host_boot TEXT NOT NULL,
    epoch INTEGER NOT NULL CHECK(epoch >= 0),
    generation INTEGER NOT NULL CHECK(generation >= 0),
    operator_label TEXT
) STRICT;
CREATE INDEX allocation_decisions_target ON allocation_decisions(instance_id, target_id, ordinal);
CREATE INDEX allocation_decisions_seat_history ON allocation_decisions(seat_id, ordinal);

CREATE TABLE seats (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    instance_id TEXT NOT NULL REFERENCES host_instances(id),
    state TEXT NOT NULL CHECK(state IN ('resolved', 'unresolved', 'retired')),
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
    unavailability_open INTEGER NOT NULL DEFAULT 1 CHECK(unavailability_open IN (0,1)),
    CHECK((state = 'retired') = (retired_at IS NOT NULL)),
    CHECK(state = 'unresolved' OR unresolved_reason IS NULL),
    CHECK((structural_terminal_id IS NULL AND structural_incarnation IS NULL AND structural_incarnation_kind IS NULL AND structural_host_boot IS NULL AND structural_host_epoch IS NULL AND structural_connection_epoch IS NULL AND structural_observation_sequence IS NULL)
       OR (target_id IS NOT NULL AND structural_terminal_id IS NOT NULL AND structural_incarnation IS NOT NULL AND structural_incarnation_kind IS NOT NULL AND structural_host_boot IS NOT NULL AND structural_host_epoch IS NOT NULL AND structural_connection_epoch IS NOT NULL AND structural_observation_sequence IS NOT NULL)),
    CHECK(unresolved_reason IS 'host_invalidation' OR
          (unresolved_from_generation_id IS NULL AND unresolved_prior_binding_generation IS NULL))
) STRICT;
CREATE UNIQUE INDEX seats_live_target ON seats(instance_id, target_id) WHERE state = 'resolved' AND target_id IS NOT NULL;
CREATE INDEX seats_instance_ordinal ON seats(instance_id, ordinal);
CREATE INDEX seats_instance_state_ordinal ON seats(instance_id,state,ordinal);
CREATE INDEX seats_unresolved_generation ON seats(unresolved_from_generation_id) WHERE unresolved_from_generation_id IS NOT NULL;

CREATE TABLE occupant_bindings (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    seat_id TEXT NOT NULL REFERENCES seats(id),
    generation INTEGER NOT NULL CHECK(generation >= 0),
    target_generation INTEGER NOT NULL DEFAULT 0 CHECK(target_generation >= 0),
    target_id TEXT NOT NULL,
    terminal_id TEXT,
    incarnation TEXT,
    host_boot TEXT NOT NULL,
    host_epoch INTEGER NOT NULL CHECK(host_epoch >= 0),
    harness TEXT NOT NULL CHECK(harness IN ('codex', 'claude')),
    native_session TEXT NOT NULL,
    execution_id TEXT NOT NULL,
    observation_provenance TEXT NOT NULL,
    observed_at INTEGER NOT NULL,
    registered_at INTEGER,
    ended_at INTEGER,
    UNIQUE(seat_id, generation)
) STRICT;
CREATE UNIQUE INDEX occupant_bindings_current ON occupant_bindings(seat_id) WHERE ended_at IS NULL;
CREATE INDEX occupant_bindings_history ON occupant_bindings(seat_id, ordinal);
CREATE INDEX occupant_bindings_execution ON occupant_bindings(seat_id, execution_id);

CREATE TABLE threads (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    instance_id TEXT NOT NULL REFERENCES host_instances(id),
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
) STRICT;
CREATE INDEX threads_instance_ordinal ON threads(instance_id, ordinal);
CREATE INDEX threads_instance_topic_ordinal ON threads(instance_id, topic, ordinal);

CREATE TABLE memberships (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id TEXT NOT NULL REFERENCES threads(id),
    seat_id TEXT NOT NULL REFERENCES seats(id),
    episode INTEGER NOT NULL DEFAULT 1 CHECK(episode > 0),
    state TEXT NOT NULL CHECK(state IN ('invited', 'joined', 'left', 'retired')),
    joined_at INTEGER,
    left_at INTEGER,
    retired_at INTEGER,
    UNIQUE(thread_id, seat_id)
) STRICT;
CREATE INDEX memberships_seat_ordinal ON memberships(seat_id, ordinal);
CREATE INDEX memberships_thread_ordinal ON memberships(thread_id, ordinal);
CREATE INDEX memberships_thread_state ON memberships(thread_id, state, ordinal);

CREATE TABLE membership_intervals (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id TEXT NOT NULL REFERENCES threads(id),
    seat_id TEXT NOT NULL REFERENCES seats(id),
    episode INTEGER NOT NULL CHECK(episode > 0),
    joined_seq INTEGER NOT NULL CHECK(joined_seq > 0),
    left_seq INTEGER CHECK(left_seq > joined_seq),
    UNIQUE(thread_id, seat_id, episode)
) STRICT;
CREATE INDEX membership_intervals_thread_ordinal ON membership_intervals(thread_id, ordinal);
CREATE INDEX membership_intervals_snapshot ON membership_intervals(thread_id, seat_id, joined_seq);
CREATE UNIQUE INDEX membership_intervals_open ON membership_intervals(thread_id, seat_id) WHERE left_seq IS NULL;

CREATE TABLE seat_availability (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    seat_id TEXT NOT NULL REFERENCES seats(id),
    decision_seq INTEGER NOT NULL CHECK(decision_seq > 0),
    decision_at INTEGER NOT NULL,
    binding_generation INTEGER NOT NULL CHECK(binding_generation >= 0),
    observation_provenance TEXT NOT NULL CHECK(length(CAST(observation_provenance AS BLOB)) BETWEEN 1 AND 512),
    UNIQUE(seat_id, decision_seq)
) STRICT;
CREATE INDEX seat_availability_after ON seat_availability(seat_id, decision_seq);

CREATE TABLE invitations (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    thread_id TEXT NOT NULL REFERENCES threads(id),
    seat_id TEXT NOT NULL REFERENCES seats(id),
    episode INTEGER NOT NULL CHECK(episode > 0),
    state TEXT NOT NULL CHECK(state IN ('pending', 'accepted', 'recipient_retired')),
    created_decision_seq INTEGER NOT NULL CHECK(created_decision_seq > 0),
    created_at INTEGER NOT NULL,
    frozen_duration_ms INTEGER NOT NULL CHECK(frozen_duration_ms > 0),
    deadline_at INTEGER NOT NULL,
    accepted_at INTEGER,
    accepted_actor_seat_id TEXT REFERENCES seats(id),
    accepted_generation INTEGER CHECK(accepted_generation >= 0),
    accepted_observation TEXT,
    warning_message_id TEXT REFERENCES messages(id),
    retired_at INTEGER,
    UNIQUE(thread_id, seat_id, episode),
    CHECK(state <> 'accepted' OR (accepted_at IS NOT NULL AND accepted_actor_seat_id IS NOT NULL AND accepted_generation IS NOT NULL AND accepted_observation IS NOT NULL))
) STRICT;
CREATE INDEX invitations_due ON invitations(deadline_at, ordinal) WHERE state = 'pending';
CREATE INDEX invitations_seat_thread_pending ON invitations(seat_id, thread_id, ordinal) WHERE state = 'pending';
CREATE INDEX invitations_thread_seat_episode ON invitations(thread_id, seat_id, episode);
CREATE INDEX invitations_seat_decision ON invitations(seat_id, created_decision_seq, ordinal);
CREATE INDEX invitations_pending_unwarned ON invitations(deadline_at, ordinal) WHERE state='pending' AND warning_message_id IS NULL;

CREATE TABLE messages (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    instance_id TEXT NOT NULL REFERENCES host_instances(id),
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
    source_invitation_id TEXT REFERENCES invitations(id),
    UNIQUE(id, thread_id),
    UNIQUE(thread_id, sequence),
    CHECK((kind = 'ordinary' AND body IS NOT NULL AND event_key IS NULL) OR
          (kind IN ('info', 'warn') AND event_json IS NOT NULL AND body IS NULL))
) STRICT;
CREATE INDEX messages_thread_sequence ON messages(thread_id, sequence);
CREATE INDEX messages_kind_ordinal ON messages(kind, ordinal);
CREATE UNIQUE INDEX messages_instance_logical ON messages(instance_id, decision_seq, event_offset);
CREATE INDEX messages_instance_ordinary_logical ON messages(instance_id, decision_seq, event_offset) WHERE kind='ordinary';
CREATE INDEX messages_instance_warning_logical ON messages(instance_id, decision_seq, event_offset) WHERE kind='warn';

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
    retired_at INTEGER,
    UNIQUE(message_id, seat_id),
    FOREIGN KEY(message_id, thread_id) REFERENCES messages(id, thread_id),
    CHECK((available_at IS NULL) = (deadline_at IS NULL))
) STRICT;
CREATE INDEX receipts_due ON receipts(deadline_at, ordinal) WHERE state = 'pending' AND deadline_at IS NOT NULL;
CREATE INDEX receipts_seat_state_ordinal ON receipts(seat_id, state, ordinal);
CREATE INDEX receipts_seat_thread_state_ordinal ON receipts(seat_id, thread_id, state, ordinal);
CREATE INDEX receipts_thread_seat_pending ON receipts(seat_id, thread_id, ordinal) WHERE state = 'pending';
CREATE INDEX receipts_pending_unwarned ON receipts(deadline_at, ordinal) WHERE state='pending' AND deadline_at IS NOT NULL AND warning_message_id IS NULL;
CREATE INDEX receipts_message_ordinal ON receipts(message_id, ordinal);
CREATE INDEX receipts_thread_pending_ordinal ON receipts(thread_id, ordinal) WHERE state='pending';

CREATE TABLE send_preparations (
    id TEXT PRIMARY KEY,
    instance_id TEXT NOT NULL REFERENCES host_instances(id),
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
    status TEXT NOT NULL CHECK(status IN ('building','sealed','discarded')),
    UNIQUE(instance_id, operation_scope, operation_key)
) STRICT;
CREATE INDEX send_preparations_status ON send_preparations(status, id);

CREATE TABLE prepared_recipients (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    preparation_id TEXT NOT NULL REFERENCES send_preparations(id),
    thread_id TEXT NOT NULL REFERENCES threads(id),
    seat_id TEXT NOT NULL REFERENCES seats(id),
    receipt_ordinal INTEGER NOT NULL CHECK(receipt_ordinal > 0),
    frozen_duration_ms INTEGER NOT NULL CHECK(frozen_duration_ms > 0),
    eligible_at_snapshot INTEGER NOT NULL CHECK(eligible_at_snapshot IN (0,1)),
    availability_provenance TEXT,
    UNIQUE(preparation_id, seat_id),
    UNIQUE(preparation_id, receipt_ordinal)
) STRICT;
CREATE INDEX prepared_recipients_seat_ordinal ON prepared_recipients(seat_id, ordinal);
CREATE INDEX prepared_recipients_seat_thread_ordinal ON prepared_recipients(seat_id, thread_id, ordinal);
CREATE INDEX prepared_recipients_preparation_ordinal ON prepared_recipients(preparation_id, ordinal);
CREATE INDEX prepared_recipients_thread_ordinal ON prepared_recipients(thread_id, ordinal);

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
CREATE INDEX prepared_unavailable_warnings_key ON prepared_unavailable_warnings(warning_key, warning_id);
CREATE INDEX prepared_unavailable_warnings_ordinal ON prepared_unavailable_warnings(ordinal);

CREATE TABLE send_manifests (
    preparation_id TEXT PRIMARY KEY REFERENCES send_preparations(id),
    message_id TEXT NOT NULL UNIQUE REFERENCES messages(id),
    instance_id TEXT NOT NULL REFERENCES host_instances(id),
    thread_id TEXT NOT NULL REFERENCES threads(id),
    decision_seq INTEGER NOT NULL CHECK(decision_seq > 0),
    decision_at INTEGER NOT NULL,
    base_sequence INTEGER NOT NULL CHECK(base_sequence > 0),
    interval_high_water INTEGER NOT NULL CHECK(interval_high_water >= 0),
    recipient_count INTEGER NOT NULL CHECK(recipient_count >= 0),
    warning_count INTEGER NOT NULL CHECK(warning_count >= 0)
) STRICT;
CREATE INDEX send_manifests_thread_sequence ON send_manifests(thread_id, base_sequence);
CREATE INDEX send_manifests_decision ON send_manifests(decision_seq, message_id);
CREATE UNIQUE INDEX send_manifests_instance_decision ON send_manifests(instance_id, decision_seq);
CREATE INDEX send_manifests_warning_decision ON send_manifests(instance_id, decision_seq) WHERE warning_count>0;

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
    warning_message_id TEXT,
    PRIMARY KEY(message_id, seat_id),
    CHECK((available_at IS NULL) = (deadline_at IS NULL))
) STRICT;
CREATE INDEX receipt_state_pending_due ON receipt_state(deadline_at, message_id, seat_id) WHERE state='pending' AND deadline_at IS NOT NULL AND warning_message_id IS NULL;

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
    last_outcome TEXT CHECK(last_outcome IS NULL OR length(last_outcome) <= 128),
    CHECK((last_invitation_seq IS NULL) = (last_invitation_offset IS NULL)),
    CHECK((last_receipt_seq IS NULL) = (last_receipt_offset IS NULL)),
    CHECK((last_warning_seq IS NULL) = (last_warning_offset IS NULL))
) STRICT;
CREATE UNIQUE INDEX wake_work_active_attempt ON wake_work(reservation_id) WHERE reservation_id IS NOT NULL;

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
) STRICT;
CREATE INDEX warning_jobs_ready ON warning_jobs(status, ordinal);

CREATE TABLE warning_recipients (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    warning_id TEXT NOT NULL,
    seat_id TEXT NOT NULL REFERENCES seats(id),
    generation INTEGER NOT NULL CHECK(generation > 0),
    UNIQUE(warning_id, seat_id)
) STRICT;
CREATE INDEX warning_recipients_seat_generation ON warning_recipients(seat_id, generation);
CREATE INDEX warning_recipients_warning_ordinal ON warning_recipients(warning_id, ordinal);

CREATE TABLE warning_offer (
    seat_id TEXT PRIMARY KEY REFERENCES seats(id),
    binding_generation INTEGER NOT NULL CHECK(binding_generation >= 0),
    execution_id TEXT NOT NULL,
    offered_through_seq INTEGER NOT NULL DEFAULT 0 CHECK(offered_through_seq >= 0)
) STRICT;

CREATE TABLE work_jobs (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL CHECK(kind IN ('warning_attribution','send_attention','receipt_timer_materialization','preparation_cleanup')),
    subject_id TEXT NOT NULL,
    position INTEGER NOT NULL DEFAULT 0 CHECK(position >= 0),
    high_water INTEGER NOT NULL CHECK(high_water >= 0),
    completed_units INTEGER NOT NULL DEFAULT 0 CHECK(completed_units >= 0),
    status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','complete','failed')),
    last_error TEXT CHECK(last_error IS NULL OR length(CAST(last_error AS BLOB)) <= 512),
    UNIQUE(kind, subject_id)
) STRICT;
CREATE INDEX work_jobs_ready ON work_jobs(status, kind, ordinal);

CREATE TABLE retirements (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    seat_id TEXT NOT NULL UNIQUE REFERENCES seats(id),
    cutover_at INTEGER NOT NULL,
    closure_boot TEXT NOT NULL,
    closure_epoch INTEGER NOT NULL CHECK(closure_epoch >= 0),
    closure_target TEXT NOT NULL,
    closure_generation INTEGER NOT NULL CHECK(closure_generation >= 0),
    thread_ordinal INTEGER NOT NULL DEFAULT 0 CHECK(thread_ordinal >= 0),
    phase TEXT NOT NULL DEFAULT 'select_thread' CHECK(phase IN ('select_thread', 'invitations', 'receipts', 'logical_receipts', 'audit', 'complete')),
    obligation_ordinal INTEGER NOT NULL DEFAULT 0 CHECK(obligation_ordinal >= 0),
    high_water_ordinal INTEGER NOT NULL DEFAULT 0 CHECK(high_water_ordinal >= 0),
    processed_units INTEGER NOT NULL DEFAULT 0 CHECK(processed_units >= 0),
    warnings_added INTEGER NOT NULL DEFAULT 0 CHECK(warnings_added >= 0),
    obligations_retired INTEGER NOT NULL DEFAULT 0 CHECK(obligations_retired >= 0),
    audits_added INTEGER NOT NULL DEFAULT 0 CHECK(audits_added >= 0),
    status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending', 'complete')),
    last_error TEXT CHECK(last_error IS NULL OR length(CAST(last_error AS BLOB)) <= 512)
) STRICT;
CREATE INDEX retirements_status_ordinal ON retirements(status, ordinal);
CREATE INDEX retirements_progress ON retirements(status, thread_ordinal, ordinal);

CREATE TABLE retirement_audits (
    job_id TEXT NOT NULL REFERENCES retirements(id),
    thread_id TEXT NOT NULL REFERENCES threads(id),
    message_id TEXT NOT NULL UNIQUE REFERENCES messages(id),
    PRIMARY KEY(job_id, thread_id)
) STRICT;

CREATE TABLE filter_revisions (
    instance_id TEXT NOT NULL REFERENCES host_instances(id),
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

-- These constant-size guards keep keyset pagination and cutover classification
-- stable across later state updates. They never visit a retirement child row.
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
CREATE TRIGGER messages_immutable BEFORE UPDATE ON messages
BEGIN SELECT RAISE(ABORT, 'message history is immutable'); END;
CREATE TRIGGER send_manifests_immutable BEFORE UPDATE ON send_manifests
BEGIN SELECT RAISE(ABORT, 'published manifest is immutable'); END;
CREATE TRIGGER messages_retained BEFORE DELETE ON messages
BEGIN SELECT RAISE(ABORT, 'message history is retained'); END;
CREATE TRIGGER receipts_ordinal_immutable BEFORE UPDATE OF ordinal ON receipts
BEGIN SELECT RAISE(ABORT, 'receipt ordinal is immutable'); END;
CREATE TRIGGER prepared_recipients_ordinal_immutable BEFORE UPDATE OF ordinal ON prepared_recipients
BEGIN SELECT RAISE(ABORT, 'prepared recipient ordinal is immutable'); END;
CREATE TRIGGER seats_retirement_terminal BEFORE UPDATE OF state, retired_at ON seats
WHEN OLD.state = 'retired' AND (NEW.state <> 'retired' OR NEW.retired_at <> OLD.retired_at)
BEGIN SELECT RAISE(ABORT, 'retired seat is terminal'); END;
CREATE TRIGGER retirements_provenance_immutable BEFORE UPDATE OF ordinal, id, seat_id, cutover_at,
    closure_boot, closure_epoch, closure_target, closure_generation ON retirements
BEGIN SELECT RAISE(ABORT, 'retirement provenance is immutable'); END;
