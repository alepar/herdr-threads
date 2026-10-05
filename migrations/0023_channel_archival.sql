-- Automatic cleanup is a daemon lifecycle decision. None of these records
-- changes seat continuity, membership, invitation or receipt authority.
CREATE TABLE archival_instances (
    instance_id TEXT PRIMARY KEY REFERENCES host_instances(id),
    runtime_boot TEXT NOT NULL,
    evidence_epoch INTEGER NOT NULL DEFAULT 0,
    mutation_revision INTEGER NOT NULL DEFAULT 0,
    policy_ms INTEGER NOT NULL DEFAULT 3600000 CHECK(policy_ms>=0),
    last_mono INTEGER,
    last_utc INTEGER,
    coherent_since INTEGER,
    host_generation INTEGER NOT NULL DEFAULT 0,
    bootstrap_veto INTEGER NOT NULL DEFAULT 1 CHECK(bootstrap_veto IN (0,1)),
    bootstrap_source TEXT,
    seed_cursor INTEGER NOT NULL DEFAULT 0,
    seat_seed_cursor INTEGER NOT NULL DEFAULT 0,
    validating_snapshot TEXT,
    snapshot_cursor TEXT NOT NULL DEFAULT '',
    snapshot_checked INTEGER NOT NULL DEFAULT 0 CHECK(snapshot_checked IN (0,1))
) STRICT;
CREATE TABLE channel_archival (
    thread_id TEXT PRIMARY KEY REFERENCES threads(id),
    instance_id TEXT NOT NULL REFERENCES host_instances(id),
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
CREATE INDEX channel_archival_due ON channel_archival(instance_id,due_mono,thread_id) WHERE enabled=1;
CREATE TABLE seat_archival (
    seat_id TEXT PRIMARY KEY REFERENCES seats(id),
    instance_id TEXT NOT NULL REFERENCES host_instances(id),
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
CREATE INDEX seat_archival_due ON seat_archival(instance_id,next_mono,seat_id) WHERE next_mono IS NOT NULL;
CREATE INDEX seat_archival_target ON seat_archival(instance_id,target_id) WHERE next_mono IS NOT NULL;
CREATE TABLE channel_handoff_fences (
    instance_id TEXT NOT NULL REFERENCES host_instances(id),
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
CREATE INDEX channel_handoff_live ON channel_handoff_fences(thread_id,compound) WHERE state='live';
CREATE INDEX channel_handoff_unattached ON channel_handoff_fences(instance_id,compound) WHERE state='live' AND thread_id IS NULL;
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
CREATE INDEX archival_invitations_thread ON digest_pending_invitations(thread_id,ordinal);
CREATE INDEX archival_preparations_thread ON send_preparations(thread_id,status,id);
CREATE INDEX archival_service_preparations_thread ON service_notification_preparations(thread_id,status,ordinal);
CREATE INDEX archival_summary_leases ON summary_jobs(thread_id,lease_until) WHERE block_id IS NULL AND lease_token IS NOT NULL;

CREATE TRIGGER archival_thread_insert AFTER INSERT ON threads BEGIN
 INSERT INTO channel_archival(thread_id,instance_id,enabled) VALUES(NEW.id,NEW.instance_id,1-NEW.archived);
 UPDATE archival_instances SET mutation_revision=mutation_revision+1 WHERE instance_id=NEW.instance_id;
END;
CREATE TRIGGER archival_thread_activity AFTER UPDATE OF archived,topic,goal,name,membership_revision,timeline_revision ON threads BEGIN
 UPDATE channel_archival SET activity_revision=activity_revision+1,quiet_mono=NULL,scan_revision=NULL,due_mono=0,enabled=1-NEW.archived WHERE thread_id=NEW.id;
 UPDATE archival_instances SET mutation_revision=mutation_revision+1 WHERE instance_id=NEW.instance_id;
END;
CREATE TRIGGER archival_seat_insert AFTER INSERT ON seats BEGIN
 INSERT INTO seat_archival(seat_id,instance_id) VALUES(NEW.id,NEW.instance_id);
END;
-- A newly decided accountable operation is activity; historical replay has
-- no INSERT and reads cannot claim an action. This is O(1), never member fanout.
CREATE TRIGGER archival_operation_activity AFTER INSERT ON operations WHEN NEW.actor_scope LIKE 'seat:%' BEGIN
 UPDATE seat_archival SET activity_revision=activity_revision+1,idle_mono=NULL,samples=0,next_mono=0 WHERE seat_id=substr(NEW.actor_scope,6);
 UPDATE archival_instances SET mutation_revision=mutation_revision+1 WHERE instance_id=(SELECT instance_id FROM seats WHERE id=substr(NEW.actor_scope,6));
END;

-- Canonical certificate invalidation inventory (bookkeeping is excluded).
CREATE TRIGGER archival_fence_seats_insert AFTER INSERT ON seats BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;
CREATE TRIGGER archival_fence_seats_update AFTER UPDATE ON seats BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;
CREATE TRIGGER archival_fence_seats_delete AFTER DELETE ON seats BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;
CREATE TRIGGER archival_fence_occupant_bindings_insert AFTER INSERT ON occupant_bindings BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;
CREATE TRIGGER archival_fence_occupant_bindings_update AFTER UPDATE ON occupant_bindings BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;
CREATE TRIGGER archival_fence_occupant_bindings_delete AFTER DELETE ON occupant_bindings BEGIN
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
CREATE TRIGGER archival_fence_memberships_update AFTER UPDATE ON memberships BEGIN
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
CREATE TRIGGER archival_fence_receipts_insert AFTER INSERT ON receipts BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;
CREATE TRIGGER archival_fence_receipts_update AFTER UPDATE ON receipts BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;
CREATE TRIGGER archival_fence_receipts_delete AFTER DELETE ON receipts BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;
CREATE TRIGGER archival_fence_receipt_state_insert AFTER INSERT ON receipt_state BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;
CREATE TRIGGER archival_fence_receipt_state_update AFTER UPDATE ON receipt_state BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;
CREATE TRIGGER archival_fence_receipt_state_delete AFTER DELETE ON receipt_state BEGIN
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
CREATE TRIGGER archival_fence_catch_up_insert AFTER INSERT ON catch_up BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;
CREATE TRIGGER archival_fence_catch_up_update AFTER UPDATE ON catch_up BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;
CREATE TRIGGER archival_fence_catch_up_delete AFTER DELETE ON catch_up BEGIN
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
CREATE TRIGGER archival_seat_activity AFTER UPDATE OF state,generation,target_generation,target_id ON seats BEGIN
 UPDATE seat_archival SET activity_revision=activity_revision+1,idle_mono=NULL,samples=0,next_mono=0 WHERE seat_id=NEW.id;
END;
CREATE TRIGGER archival_binding_activity AFTER UPDATE OF registered_at,ended_at,generation,harness,native_session,execution_id,target_id,host_boot,host_epoch,observation_provenance ON occupant_bindings BEGIN
 UPDATE seat_archival SET activity_revision=activity_revision+1,idle_mono=NULL,samples=0,next_mono=0 WHERE seat_id=NEW.seat_id;
END;
CREATE TRIGGER archival_host_invalidation AFTER UPDATE OF host_boot,host_epoch,invalidation_revision,baseline_hold_unclaimed ON host_instances
WHEN NEW.host_boot IS NOT OLD.host_boot OR NEW.host_epoch!=OLD.host_epoch OR NEW.invalidation_revision!=OLD.invalidation_revision OR NEW.baseline_hold_unclaimed!=OLD.baseline_hold_unclaimed BEGIN
 UPDATE archival_instances SET evidence_epoch=evidence_epoch+1,coherent_since=NULL,mutation_revision=mutation_revision+1 WHERE instance_id=NEW.id;
END;
-- Current-target nonidle/unknown evidence invalidates positive samples. A
-- structural snapshot with Unknown UI is not an inspected composer sample.
CREATE TRIGGER archival_current_negative_insert AFTER INSERT ON observed_targets WHEN NEW.ui_state!='idle' BEGIN
 UPDATE seat_archival SET idle_mono=NULL,samples=0,next_mono=0 WHERE instance_id=NEW.instance_id AND target_id=NEW.target_id AND next_mono IS NOT NULL;
 UPDATE archival_instances SET mutation_revision=mutation_revision+1 WHERE instance_id=NEW.instance_id;
END;
CREATE TRIGGER archival_current_negative_update AFTER UPDATE ON observed_targets WHEN NEW.ui_state!='idle' BEGIN
 UPDATE seat_archival SET idle_mono=NULL,samples=0,next_mono=0 WHERE instance_id=NEW.instance_id AND target_id=NEW.target_id AND next_mono IS NOT NULL;
 UPDATE archival_instances SET mutation_revision=mutation_revision+1 WHERE instance_id=NEW.instance_id;
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
CREATE INDEX archival_preparations_page ON send_preparations(thread_id,id);
CREATE INDEX archival_service_preparations_page ON service_notification_preparations(thread_id,ordinal);

-- Positive UI never excuses a changed identity after a member was scanned.
-- Lookups touch only the current positively qualified target, not seat history.
CREATE INDEX archival_positive_targets ON seat_archival(instance_id,target_id) WHERE idle_mono IS NOT NULL;
CREATE TRIGGER archival_current_structure_insert AFTER INSERT ON observed_targets
WHEN EXISTS(SELECT 1 FROM seat_archival a WHERE a.instance_id=NEW.instance_id AND a.target_id=NEW.target_id AND a.idle_mono IS NOT NULL AND
 (a.target_generation!=NEW.generation OR a.terminal IS NOT NEW.terminal_id OR a.incarnation IS NOT NEW.incarnation OR a.host_boot!=NEW.host_boot OR a.host_epoch!=NEW.epoch)) BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1 WHERE instance_id=NEW.instance_id;
 UPDATE seat_archival SET idle_mono=NULL,samples=0,next_mono=0 WHERE instance_id=NEW.instance_id AND target_id=NEW.target_id AND idle_mono IS NOT NULL;
END;
CREATE TRIGGER archival_current_structure_update AFTER UPDATE ON observed_targets
WHEN EXISTS(SELECT 1 FROM seat_archival a WHERE a.instance_id=NEW.instance_id AND a.target_id=NEW.target_id AND a.idle_mono IS NOT NULL AND
 (a.target_generation!=NEW.generation OR a.terminal IS NOT NEW.terminal_id OR a.incarnation IS NOT NEW.incarnation OR a.host_boot!=NEW.host_boot OR a.host_epoch!=NEW.epoch)) BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1 WHERE instance_id=NEW.instance_id;
 UPDATE seat_archival SET idle_mono=NULL,samples=0,next_mono=0 WHERE instance_id=NEW.instance_id AND target_id=NEW.target_id AND idle_mono IS NOT NULL;
END;
-- A conflicting candidate snapshot may conservatively veto even if publication
-- is later abandoned. Matching structural Unknown snapshots grant no evidence
-- and do not churn the certificate revision.
CREATE TRIGGER archival_snapshot_structure AFTER INSERT ON snapshot_targets
WHEN EXISTS(SELECT 1 FROM snapshot_generations g JOIN seat_archival a ON a.instance_id=g.instance_id AND a.target_id=NEW.target_id WHERE g.id=NEW.generation_id AND a.idle_mono IS NOT NULL AND
 (a.target_generation!=NEW.generation OR a.terminal IS NOT NEW.terminal_id OR a.incarnation IS NOT g.incarnation OR a.host_boot!=g.host_boot OR a.host_epoch!=g.epoch OR NEW.ui_state NOT IN ('idle','unknown'))) BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1 WHERE instance_id=(SELECT instance_id FROM snapshot_generations WHERE id=NEW.generation_id);
 UPDATE seat_archival SET idle_mono=NULL,samples=0,next_mono=0 WHERE instance_id=(SELECT instance_id FROM snapshot_generations WHERE id=NEW.generation_id) AND target_id=NEW.target_id AND idle_mono IS NOT NULL;
END;

CREATE INDEX archival_positive_seats ON seat_archival(instance_id,seat_id) WHERE idle_mono IS NOT NULL;

-- Constant-work membership existence for bounded composer reservations.
CREATE INDEX memberships_archival_joined_seat ON memberships(seat_id) WHERE state='joined';

-- O(1) queue notifications: inactive retained rows remain parked until canonical
-- continuity/membership changes. Eligibility is rechecked in the deciding turn.
CREATE TRIGGER archival_binding_queue_insert AFTER INSERT ON occupant_bindings BEGIN
 UPDATE seat_archival SET next_mono=0 WHERE seat_id=NEW.seat_id;
END;
CREATE TRIGGER archival_binding_queue_delete AFTER DELETE ON occupant_bindings BEGIN
 UPDATE seat_archival SET next_mono=0 WHERE seat_id=OLD.seat_id;
END;
CREATE TRIGGER archival_membership_queue_delete AFTER DELETE ON memberships BEGIN
 UPDATE seat_archival SET next_mono=0 WHERE seat_id=OLD.seat_id;
END;
