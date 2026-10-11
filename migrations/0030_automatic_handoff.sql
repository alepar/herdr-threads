-- Automatic mode is canonical; legacy fence slots remain storage-only.
CREATE TABLE automatic_handoff_companions (
    instance_id TEXT NOT NULL,
    actor_scope TEXT NOT NULL,
    compound TEXT NOT NULL,
    state_dir TEXT NOT NULL,
    host_endpoint TEXT NOT NULL,
    original_json BLOB NOT NULL CHECK(length(original_json)<=131072),
    begin_key TEXT NOT NULL,
    ensure_key TEXT NOT NULL,
    send_key TEXT NOT NULL,
    complete_key TEXT NOT NULL,
    PRIMARY KEY(instance_id,actor_scope,compound),
    FOREIGN KEY(instance_id,actor_scope,compound) REFERENCES channel_handoff_fences(instance_id,actor_scope,compound),
    CHECK(begin_key!=ensure_key AND begin_key!=send_key AND begin_key!=complete_key AND ensure_key!=send_key AND ensure_key!=complete_key AND send_key!=complete_key)
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
CREATE TRIGGER automatic_handoff_immutable BEFORE UPDATE ON automatic_handoff_companions
BEGIN SELECT RAISE(ABORT,'immutable automatic handoff original'); END;
CREATE TRIGGER automatic_handoff_retained BEFORE DELETE ON automatic_handoff_companions
BEGIN SELECT RAISE(ABORT,'automatic handoff originals retained'); END;
CREATE TRIGGER automatic_handoff_keys_immutable BEFORE UPDATE ON automatic_handoff_keys
BEGIN SELECT RAISE(ABORT,'immutable automatic handoff key'); END;
CREATE TRIGGER automatic_handoff_keys_retained BEFORE DELETE ON automatic_handoff_keys
BEGIN SELECT RAISE(ABORT,'automatic handoff keys retained'); END;
CREATE TRIGGER automatic_handoff_attachment BEFORE UPDATE OF thread_id ON channel_handoff_fences
WHEN OLD.thread_id IS NOT NULL AND NEW.thread_id IS NOT OLD.thread_id
 AND EXISTS(SELECT 1 FROM automatic_handoff_companions WHERE instance_id=OLD.instance_id AND actor_scope=OLD.actor_scope AND compound=OLD.compound)
BEGIN SELECT RAISE(ABORT,'immutable automatic handoff attachment'); END;
-- Additive V2 topology retention; every child key names its canonical parent.
CREATE TABLE automatic_bootstrap_parents (
    id INTEGER PRIMARY KEY,
    instance_id TEXT NOT NULL REFERENCES host_instances(id),
    actor_scope TEXT NOT NULL,
    compound TEXT NOT NULL,
    state_dir TEXT NOT NULL,
    host_endpoint TEXT NOT NULL,
    original_json BLOB NOT NULL CHECK(length(original_json)<=131072),
    state TEXT NOT NULL CHECK(state IN ('prepared','possible_creation','created','attached','completed','cancelled')),
    current_attempt INTEGER NOT NULL CHECK(current_attempt>0),
    administrative_revision INTEGER NOT NULL DEFAULT 0 CHECK(administrative_revision>=0),
    thread_id TEXT REFERENCES threads(id),
    UNIQUE(instance_id,actor_scope,compound)
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
CREATE TABLE automatic_bootstrap_attachments (
    parent_id INTEGER PRIMARY KEY REFERENCES automatic_bootstrap_parents(id),
    attempt INTEGER NOT NULL,
    attachment_json BLOB NOT NULL CHECK(length(attachment_json)<=131072),
    FOREIGN KEY(parent_id,attempt) REFERENCES automatic_bootstrap_attempts(parent_id,attempt)
) STRICT;
CREATE TABLE automatic_bootstrap_reports (
    parent_id INTEGER PRIMARY KEY REFERENCES automatic_bootstrap_parents(id),
    report_json BLOB NOT NULL CHECK(length(report_json)<=2097152)
) STRICT;
CREATE INDEX automatic_bootstrap_parent_keys ON automatic_bootstrap_keys(parent_id,attempt);
CREATE TRIGGER automatic_bootstrap_original BEFORE UPDATE OF instance_id,actor_scope,compound,state_dir,host_endpoint,original_json ON automatic_bootstrap_parents
BEGIN SELECT RAISE(ABORT,'immutable automatic bootstrap original'); END;
CREATE TRIGGER automatic_bootstrap_terminal BEFORE UPDATE ON automatic_bootstrap_parents
WHEN (OLD.state IN ('completed','cancelled') AND NEW.state IS NOT OLD.state) OR (OLD.thread_id IS NOT NULL AND NEW.thread_id IS NOT OLD.thread_id)
BEGIN SELECT RAISE(ABORT,'absorbing automatic bootstrap attachment'); END;
CREATE TRIGGER automatic_bootstrap_retained BEFORE DELETE ON automatic_bootstrap_parents
BEGIN SELECT RAISE(ABORT,'automatic bootstrap retained'); END;
CREATE TRIGGER automatic_bootstrap_keys_immutable BEFORE UPDATE ON automatic_bootstrap_keys
BEGIN SELECT RAISE(ABORT,'immutable automatic bootstrap keys'); END;
CREATE TRIGGER automatic_bootstrap_keys_retained BEFORE DELETE ON automatic_bootstrap_keys
BEGIN SELECT RAISE(ABORT,'automatic bootstrap keys retained'); END;
CREATE TRIGGER automatic_bootstrap_attempt_identity BEFORE UPDATE ON automatic_bootstrap_attempts
WHEN NEW.parent_id IS NOT OLD.parent_id OR NEW.attempt IS NOT OLD.attempt OR (OLD.creation_json IS NOT NULL AND NEW.creation_json IS NOT OLD.creation_json)
BEGIN SELECT RAISE(ABORT,'immutable automatic creation evidence'); END;
CREATE TRIGGER automatic_bootstrap_attempts_retained BEFORE DELETE ON automatic_bootstrap_attempts
BEGIN SELECT RAISE(ABORT,'automatic attempts retained'); END;
CREATE TRIGGER automatic_bootstrap_attachment_immutable BEFORE UPDATE ON automatic_bootstrap_attachments
BEGIN SELECT RAISE(ABORT,'immutable automatic attachment'); END;
CREATE TRIGGER automatic_bootstrap_attachment_retained BEFORE DELETE ON automatic_bootstrap_attachments
BEGIN SELECT RAISE(ABORT,'automatic attachment retained'); END;
CREATE TRIGGER automatic_bootstrap_report_immutable BEFORE UPDATE ON automatic_bootstrap_reports
BEGIN SELECT RAISE(ABORT,'immutable automatic report'); END;
CREATE TRIGGER automatic_bootstrap_report_retained BEFORE DELETE ON automatic_bootstrap_reports
BEGIN SELECT RAISE(ABORT,'automatic report retained'); END;

CREATE TABLE automatic_bootstrap_recoveries (
 parent_id INTEGER NOT NULL REFERENCES automatic_bootstrap_parents(id),
 attempt INTEGER NOT NULL,
 decision_kind TEXT NOT NULL CHECK(decision_kind IN ('recovery','cancellation')),
 operation TEXT NOT NULL CHECK(length(operation) BETWEEN 1 AND 256),
 revision INTEGER NOT NULL CHECK(revision>0),
 result_json BLOB NOT NULL CHECK(length(result_json)<=262144),
 PRIMARY KEY(parent_id,attempt,decision_kind),
 UNIQUE(parent_id,operation), UNIQUE(parent_id,revision),
 FOREIGN KEY(parent_id,attempt) REFERENCES automatic_bootstrap_attempts(parent_id,attempt)
) STRICT;
CREATE TRIGGER automatic_bootstrap_recovery_immutable BEFORE UPDATE ON automatic_bootstrap_recoveries
BEGIN SELECT RAISE(ABORT,'immutable automatic recovery'); END;
CREATE TRIGGER automatic_bootstrap_recovery_retained BEFORE DELETE ON automatic_bootstrap_recoveries
BEGIN SELECT RAISE(ABORT,'automatic recovery retained'); END;
