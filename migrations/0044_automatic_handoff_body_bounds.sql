-- Additive bounds for exact escaped handoff bodies. Independent creation/report bounds remain unchanged.

CREATE TEMP TABLE ht39_automatic_handoff_companions AS SELECT * FROM automatic_handoff_companions;

CREATE TEMP TABLE ht39_automatic_bootstrap_parents AS SELECT * FROM automatic_bootstrap_parents;

CREATE TEMP TABLE ht39_automatic_bootstrap_attachments AS SELECT * FROM automatic_bootstrap_attachments;

CREATE TEMP TABLE ht39_automatic_bootstrap_recoveries AS SELECT * FROM automatic_bootstrap_recoveries;

DROP TABLE automatic_bootstrap_recoveries;

DROP TABLE automatic_bootstrap_attachments;

DROP TABLE automatic_bootstrap_parents;

DROP TABLE automatic_handoff_companions;

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

CREATE TRIGGER automatic_handoff_immutable BEFORE UPDATE ON automatic_handoff_companions
BEGIN SELECT RAISE(ABORT,'immutable automatic handoff original'); END;

CREATE TRIGGER automatic_handoff_retained BEFORE DELETE ON automatic_handoff_companions
BEGIN SELECT RAISE(ABORT,'automatic handoff originals retained'); END;

CREATE TABLE automatic_bootstrap_parents (
    id INTEGER PRIMARY KEY,
    instance_id TEXT NOT NULL REFERENCES host_instances(id),
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

CREATE TRIGGER automatic_bootstrap_recovery_immutable BEFORE UPDATE ON automatic_bootstrap_recoveries
BEGIN SELECT RAISE(ABORT,'immutable automatic recovery'); END;

CREATE TRIGGER automatic_bootstrap_recovery_retained BEFORE DELETE ON automatic_bootstrap_recoveries
BEGIN SELECT RAISE(ABORT,'automatic recovery retained'); END;

INSERT INTO automatic_handoff_companions SELECT * FROM ht39_automatic_handoff_companions;

INSERT INTO automatic_bootstrap_parents SELECT * FROM ht39_automatic_bootstrap_parents;

INSERT INTO automatic_bootstrap_attachments SELECT * FROM ht39_automatic_bootstrap_attachments;

INSERT INTO automatic_bootstrap_recoveries SELECT * FROM ht39_automatic_bootstrap_recoveries;

DROP TABLE ht39_automatic_handoff_companions;

DROP TABLE ht39_automatic_bootstrap_parents;

DROP TABLE ht39_automatic_bootstrap_attachments;

DROP TABLE ht39_automatic_bootstrap_recoveries;
