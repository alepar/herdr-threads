-- Open invitation-condition wake is independent of informational delivery.
-- These addresses freeze for this open episode, including historical evidence.
ALTER TABLE warning_conditions ADD COLUMN invitation_originator_instance_id TEXT REFERENCES host_instances(id);
ALTER TABLE warning_conditions ADD COLUMN invitation_originator_seat_id TEXT REFERENCES seats(id)
    CHECK((invitation_originator_instance_id IS NULL) = (invitation_originator_seat_id IS NULL));

WITH origins AS (
    SELECT c.ordinal,t.instance_id,
        coalesce(i.inviter_seat_id,(SELECT json_extract(m.event_json,'$.actor_seat') FROM messages m
            WHERE m.event_key='invite:'||i.id AND m.source_invitation_id=i.id AND m.thread_id=i.thread_id
            AND m.kind='info' AND json_extract(m.event_json,'$.action')='invite'
            AND json_extract(m.event_json,'$.invitation')=i.id AND json_extract(m.event_json,'$.seat')=i.seat_id)) AS seat_id
    FROM warning_conditions c JOIN invitations i ON i.id=c.condition_id AND i.thread_id=c.thread_id
    JOIN threads t ON t.id=i.thread_id
    JOIN messages warning ON warning.id=c.open_warning_id AND warning.kind='warn'
        AND warning.source_invitation_id=i.id AND warning.thread_id=i.thread_id
    WHERE c.condition_kind='invitation' AND c.clear_warning_id IS NULL
)
UPDATE warning_conditions SET (invitation_originator_instance_id,invitation_originator_seat_id) =
    (SELECT o.instance_id,o.seat_id FROM origins o JOIN seats s ON s.id=o.seat_id AND s.instance_id=o.instance_id
     WHERE o.ordinal=warning_conditions.ordinal)
WHERE condition_kind='invitation' AND clear_warning_id IS NULL;

CREATE INDEX warning_conditions_invitation_origin ON warning_conditions(invitation_originator_instance_id,invitation_originator_seat_id,ordinal)
    WHERE condition_kind='invitation' AND clear_warning_id IS NULL AND invitation_originator_seat_id IS NOT NULL;

CREATE TRIGGER warning_conditions_invitation_origin_immutable BEFORE UPDATE OF invitation_originator_instance_id,invitation_originator_seat_id ON warning_conditions
WHEN NEW.invitation_originator_instance_id IS NOT OLD.invitation_originator_instance_id OR NEW.invitation_originator_seat_id IS NOT OLD.invitation_originator_seat_id
BEGIN SELECT RAISE(ABORT, 'invitation warning origin is immutable'); END;
