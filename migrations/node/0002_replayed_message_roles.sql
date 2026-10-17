-- v2.0.0 message.sent omitted author_role. Replay may backfill only a native
-- ordinary message with no acting local binding and the existing journal
-- attribution. Current sends and programmatic relays retain their guards.
DROP TRIGGER messages_summary_author_insert;
CREATE TRIGGER messages_summary_author_insert BEFORE INSERT ON messages
BEGIN
    SELECT RAISE(ABORT, 'invalid message summary authorship') WHERE
        (NEW.author_role_backfilled <> 0 AND COALESCE(
            NEW.kind = 'ordinary' AND NEW.actor_seat_id IS NOT NULL AND
            NEW.author_service_id IS NULL AND NEW.sender_binding_id IS NULL AND
            CASE WHEN json_valid(NEW.native_observation) THEN
                json_extract(NEW.native_observation, '$.provenance') IN
                    ('local_journal', 'replicated_from:' || NEW.origin_node)
            ELSE 0 END, 0) = 0) OR
        (NEW.author_kind = 'programmatic' AND NEW.relays_user <> 0);
END;
