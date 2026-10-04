-- Materialized committed timeline activity; creation is the empty-thread fallback.
ALTER TABLE threads ADD COLUMN last_activity INTEGER NOT NULL DEFAULT 0;
UPDATE threads SET last_activity = max(created_at,
    coalesce((SELECT max(decision_at) FROM messages WHERE thread_id=threads.id), created_at),
    coalesce((SELECT max(decision_at) FROM send_manifests WHERE thread_id=threads.id), created_at));
CREATE INDEX threads_recent_activity ON threads(instance_id, last_activity DESC, ordinal DESC);
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
