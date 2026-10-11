-- Widen only body-bearing bootstrap envelopes. Called with foreign keys off
-- outside a transaction for an existing populated database. Recreate original
-- names directly: never rename a referenced table or rewrite retained JSON.
CREATE TEMP TABLE ht29_handoffs AS SELECT * FROM bootstrap_handoffs;
CREATE TEMP TABLE ht29_recovery AS SELECT * FROM bootstrap_recovery_decisions;
DROP TABLE bootstrap_recovery_decisions;
DROP TABLE bootstrap_handoffs;
CREATE TABLE bootstrap_handoffs (
    id INTEGER PRIMARY KEY,
    instance_id TEXT NOT NULL REFERENCES host_instances(id),
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
INSERT INTO bootstrap_handoffs SELECT * FROM ht29_handoffs;
INSERT INTO bootstrap_recovery_decisions SELECT * FROM ht29_recovery;
CREATE INDEX bootstrap_live_thread ON bootstrap_handoffs(thread_id,compound) WHERE state NOT IN ('completed','cancelled');
CREATE INDEX bootstrap_live_unattached ON bootstrap_handoffs(instance_id,compound) WHERE thread_id IS NULL AND state NOT IN ('completed','cancelled');
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
DROP TABLE ht29_handoffs;
DROP TABLE ht29_recovery;
