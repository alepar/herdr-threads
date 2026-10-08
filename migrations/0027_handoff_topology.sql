-- Allocated topology27, registered after the actual lazy delivery26 migration.
-- Bootstrap status is retained state, never native submission permission.
CREATE TABLE bootstrap_handoffs (
    id INTEGER PRIMARY KEY,
    instance_id TEXT NOT NULL REFERENCES host_instances(id),
    state_dir TEXT NOT NULL,
    host_endpoint TEXT NOT NULL,
    actor_scope TEXT NOT NULL,
    compound TEXT NOT NULL,
    digest TEXT NOT NULL CHECK(length(digest)=64),
    identity_json BLOB NOT NULL CHECK(length(identity_json)<=131072),
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
CREATE INDEX bootstrap_live_thread ON bootstrap_handoffs(thread_id,compound) WHERE state NOT IN ('completed','cancelled');
CREATE INDEX bootstrap_live_unattached ON bootstrap_handoffs(instance_id,compound) WHERE thread_id IS NULL AND state NOT IN ('completed','cancelled');
CREATE TABLE bootstrap_child_keys (
    parent_id INTEGER NOT NULL,
    instance_id TEXT NOT NULL,
    state_dir TEXT NOT NULL,
    host_endpoint TEXT NOT NULL,
    actor_scope TEXT NOT NULL,
    operation_key TEXT NOT NULL,
    role TEXT NOT NULL CHECK(role IN ('compound','begin','create','invite','send','complete','handoff','resolve','attach','linked_complete','reserve','record','check','recovery')),
    attempt INTEGER NOT NULL CHECK(attempt BETWEEN 0 AND 4294967295),
    PRIMARY KEY(instance_id,state_dir,host_endpoint,actor_scope,operation_key),
    UNIQUE(parent_id,role,attempt),
    FOREIGN KEY(parent_id,instance_id,state_dir,host_endpoint,actor_scope) REFERENCES bootstrap_handoffs(id,instance_id,state_dir,host_endpoint,actor_scope),
    CHECK((role IN ('reserve','record','check','recovery'))=(attempt>0))
) STRICT;
-- Current-key replay must not walk all historical attempt keys for a parent.
CREATE INDEX bootstrap_keys_attempt ON bootstrap_child_keys(parent_id,attempt,role);
CREATE TABLE bootstrap_attempts (
    parent_id INTEGER NOT NULL REFERENCES bootstrap_handoffs(id),
    attempt INTEGER NOT NULL CHECK(attempt BETWEEN 1 AND 4294967295),
    state TEXT NOT NULL CHECK(state IN ('prepared','possible_creation','not_submitted','outcome_unknown','created')),
    reserve_key TEXT NOT NULL,
    record_key TEXT NOT NULL,
    check_key TEXT NOT NULL,
    reserved_administrative_revision INTEGER CHECK(reserved_administrative_revision>=0),
    creation_json BLOB CHECK(length(creation_json)<=131072),
    PRIMARY KEY(parent_id,attempt),
    CHECK((state='created')=(creation_json IS NOT NULL)),
    CHECK(reserve_key!=record_key AND reserve_key!=check_key AND record_key!=check_key)
) STRICT;
CREATE TABLE bootstrap_recovery_decisions (
    parent_id INTEGER NOT NULL REFERENCES bootstrap_handoffs(id),
    attempt INTEGER NOT NULL,
    operation TEXT NOT NULL,
    result_json BLOB NOT NULL CHECK(length(result_json)<=262144),
    PRIMARY KEY(parent_id,operation),
    UNIQUE(parent_id,attempt),
    FOREIGN KEY(parent_id,attempt) REFERENCES bootstrap_attempts(parent_id,attempt)
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
CREATE TRIGGER bootstrap_attempt_identity BEFORE UPDATE ON bootstrap_attempts
WHEN NEW.parent_id!=OLD.parent_id OR NEW.attempt!=OLD.attempt OR NEW.reserve_key!=OLD.reserve_key
 OR NEW.record_key!=OLD.record_key OR NEW.check_key!=OLD.check_key
 OR (OLD.creation_json IS NOT NULL AND NEW.creation_json IS NOT OLD.creation_json)
BEGIN SELECT RAISE(ABORT,'immutable bootstrap attempt identity'); END;
CREATE TRIGGER bootstrap_attempt_retained BEFORE DELETE ON bootstrap_attempts
BEGIN SELECT RAISE(ABORT,'bootstrap attempts retained'); END;
CREATE TRIGGER bootstrap_attempt_keys AFTER INSERT ON bootstrap_attempts BEGIN
 INSERT INTO bootstrap_child_keys SELECT NEW.parent_id,instance_id,state_dir,host_endpoint,actor_scope,NEW.reserve_key,'reserve',NEW.attempt FROM bootstrap_handoffs WHERE id=NEW.parent_id;
 INSERT INTO bootstrap_child_keys SELECT NEW.parent_id,instance_id,state_dir,host_endpoint,actor_scope,NEW.record_key,'record',NEW.attempt FROM bootstrap_handoffs WHERE id=NEW.parent_id;
 INSERT INTO bootstrap_child_keys SELECT NEW.parent_id,instance_id,state_dir,host_endpoint,actor_scope,NEW.check_key,'check',NEW.attempt FROM bootstrap_handoffs WHERE id=NEW.parent_id;
END;
CREATE TRIGGER bootstrap_recovery_key AFTER INSERT ON bootstrap_recovery_decisions BEGIN
 INSERT INTO bootstrap_child_keys SELECT NEW.parent_id,instance_id,state_dir,host_endpoint,actor_scope,NEW.operation,'recovery',NEW.attempt FROM bootstrap_handoffs WHERE id=NEW.parent_id;
END;
CREATE TRIGGER bootstrap_keys_immutable BEFORE UPDATE ON bootstrap_child_keys
BEGIN SELECT RAISE(ABORT,'immutable bootstrap child key'); END;
CREATE TRIGGER bootstrap_keys_retained BEFORE DELETE ON bootstrap_child_keys
BEGIN SELECT RAISE(ABORT,'bootstrap child keys retained'); END;
CREATE TRIGGER bootstrap_recovery_immutable BEFORE UPDATE ON bootstrap_recovery_decisions
BEGIN SELECT RAISE(ABORT,'immutable bootstrap recovery decision'); END;
CREATE TRIGGER bootstrap_recovery_retained BEFORE DELETE ON bootstrap_recovery_decisions
BEGIN SELECT RAISE(ABORT,'bootstrap recovery decisions retained'); END;
CREATE TRIGGER bootstrap_attachment_immutable BEFORE UPDATE ON bootstrap_attachments
BEGIN SELECT RAISE(ABORT,'immutable bootstrap attachment'); END;
CREATE TRIGGER bootstrap_attachment_retained BEFORE DELETE ON bootstrap_attachments
BEGIN SELECT RAISE(ABORT,'bootstrap attachment retained'); END;
CREATE TRIGGER bootstrap_report_immutable BEFORE UPDATE ON bootstrap_reports
BEGIN SELECT RAISE(ABORT,'immutable bootstrap completion'); END;
CREATE TRIGGER bootstrap_report_retained BEFORE DELETE ON bootstrap_reports
BEGIN SELECT RAISE(ABORT,'bootstrap completion retained'); END;
