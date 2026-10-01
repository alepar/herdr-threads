-- Required-only invitations keep a compatibility membership row for stable
-- ordinal paging. This marker records the independent voluntary fact.
ALTER TABLE memberships ADD COLUMN voluntary_state TEXT
    CHECK(voluntary_state IN ('absent','invited','joined','left','retired'));

-- Frozen C2 v3 could leave a required-only invitation in the raw invited
-- state. Its invitation decision sequence identifies that origin exactly.
-- A previous left interval supplies the prior state; the matching leave event
-- supplies its erased timestamp. Migration preflight rejects missing evidence.
UPDATE memberships AS m SET
    voluntary_state=CASE WHEN EXISTS (
        SELECT 1 FROM membership_intervals mi
        WHERE mi.thread_id=m.thread_id AND mi.seat_id=m.seat_id AND mi.left_seq IS NOT NULL
    ) THEN 'left' ELSE 'absent' END,
    left_at=CASE WHEN EXISTS (
        SELECT 1 FROM membership_intervals mi
        WHERE mi.thread_id=m.thread_id AND mi.seat_id=m.seat_id AND mi.left_seq IS NOT NULL
    ) THEN (
        SELECT msg.decision_at FROM membership_intervals mi
        JOIN messages msg ON msg.thread_id=mi.thread_id AND msg.decision_seq=mi.left_seq
        WHERE mi.thread_id=m.thread_id AND mi.seat_id=m.seat_id
          AND mi.left_seq IS NOT NULL
          AND json_extract(msg.event_json,'$.action')='leave'
          AND json_extract(msg.event_json,'$.seat')=m.seat_id
        ORDER BY mi.episode DESC LIMIT 1
    ) ELSE m.left_at END
WHERE m.state='invited' AND EXISTS (
    SELECT 1 FROM requirement_episodes r
    JOIN invitations i ON i.id=r.invitation_id
    WHERE r.thread_id=m.thread_id AND r.seat_id=m.seat_id
      AND r.created_decision_seq=i.created_decision_seq
      AND i.episode=m.episode
);
