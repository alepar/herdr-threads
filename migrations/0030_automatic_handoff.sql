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
