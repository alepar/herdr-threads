-- Pending-only access paths for the seat attention digest, check-in and inbox
-- (schema v8). Every projection except the programmatic one is a pure pending
-- set: a trigger inserts a row when the pending source row appears and
-- deletes it on settlement, in the writer's own transaction. Programmatic
-- notices are pending above the seat's current occupant's offered notice
-- frontier (`digest_notice_offer`, occupant-scoped and monotone): a check-in
-- advances it over the capped page it carries, one row write, and deletes
-- nothing (wave-2 fix2 root decision (a)). Each read is a set of per-source
-- `LIMIT cap+1` walks over these projections' seat-leading indexes, in
-- logical publication order where the key is known at insertion (digest
-- fix5). Statements are separated by blank lines; startup compares every
-- CREATE with sqlite_master.

CREATE TABLE digest_open_warnings (
    source TEXT NOT NULL CHECK(source IN ('job','prepared')),
    source_ordinal INTEGER NOT NULL,
    warning_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    affected_seat_id TEXT,
    condition_kind TEXT NOT NULL CHECK(condition_kind IN ('invitation','receipt','unavailable')),
    condition_id TEXT NOT NULL,
    episode INTEGER,
    PRIMARY KEY(source, source_ordinal)
) STRICT;

CREATE INDEX digest_open_warnings_affected ON digest_open_warnings(affected_seat_id, source, source_ordinal);

CREATE INDEX digest_open_warnings_affected_thread ON digest_open_warnings(affected_seat_id, thread_id, source, source_ordinal);

CREATE INDEX digest_open_warnings_warning ON digest_open_warnings(warning_id);

CREATE INDEX digest_open_warnings_condition ON digest_open_warnings(condition_kind, condition_id);

CREATE TABLE digest_pending_invitations (
    seat_id TEXT NOT NULL,
    invitation_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    created_decision_seq INTEGER NOT NULL,
    ordinal INTEGER NOT NULL,
    PRIMARY KEY(seat_id, invitation_id)
) STRICT;

CREATE INDEX digest_pending_invitations_seat ON digest_pending_invitations(seat_id, created_decision_seq, ordinal);

CREATE INDEX digest_pending_invitations_thread ON digest_pending_invitations(seat_id, thread_id, created_decision_seq, ordinal);

CREATE TRIGGER digest_invitation_created AFTER INSERT ON invitations WHEN NEW.state='pending'
BEGIN
    INSERT OR IGNORE INTO digest_pending_invitations(seat_id, invitation_id, thread_id, created_decision_seq, ordinal) VALUES (NEW.seat_id, NEW.id, NEW.thread_id, NEW.created_decision_seq, NEW.ordinal);
END;

CREATE TRIGGER digest_invitation_settled AFTER UPDATE OF state ON invitations WHEN NEW.state!='pending'
BEGIN
    DELETE FROM digest_pending_invitations WHERE seat_id=OLD.seat_id AND invitation_id=OLD.id;
    DELETE FROM digest_open_warnings WHERE condition_kind='invitation' AND condition_id=OLD.id;
END;

CREATE TRIGGER digest_invitation_cancelled AFTER INSERT ON invitation_cancellations
BEGIN
    DELETE FROM digest_pending_invitations WHERE invitation_id=NEW.invitation_id AND seat_id=(SELECT seat_id FROM invitations WHERE id=NEW.invitation_id);
    DELETE FROM digest_open_warnings WHERE condition_kind='invitation' AND condition_id=NEW.invitation_id;
END;

CREATE TABLE digest_pending_manifest_receipts (
    seat_id TEXT NOT NULL,
    preparation_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL,
    decision_seq INTEGER,
    PRIMARY KEY(seat_id, preparation_id)
) STRICT;

CREATE INDEX digest_pending_manifest_receipts_thread ON digest_pending_manifest_receipts(thread_id, ordinal);

CREATE INDEX digest_pending_manifest_receipts_seat ON digest_pending_manifest_receipts(seat_id, decision_seq, ordinal);

CREATE INDEX digest_pending_manifest_receipts_seat_thread ON digest_pending_manifest_receipts(seat_id, thread_id, decision_seq, ordinal);

CREATE TRIGGER digest_manifest_receipt_staged AFTER INSERT ON prepared_recipients
BEGIN
    INSERT OR IGNORE INTO digest_pending_manifest_receipts(seat_id, preparation_id, thread_id, ordinal) VALUES (NEW.seat_id, NEW.preparation_id, NEW.thread_id, NEW.ordinal);
END;

CREATE TRIGGER digest_manifest_receipt_unstaged AFTER DELETE ON prepared_recipients
BEGIN
    DELETE FROM digest_pending_manifest_receipts WHERE seat_id=OLD.seat_id AND preparation_id=OLD.preparation_id;
END;

CREATE TRIGGER digest_manifest_receipt_materialized AFTER INSERT ON receipt_state WHEN NEW.state='pending'
BEGIN
    UPDATE digest_pending_manifest_receipts SET decision_seq=(SELECT decision_seq FROM send_manifests WHERE message_id=NEW.message_id) WHERE seat_id=NEW.seat_id AND preparation_id=(SELECT preparation_id FROM send_manifests WHERE message_id=NEW.message_id);
END;

CREATE TRIGGER digest_receipt_state_settled_insert AFTER INSERT ON receipt_state WHEN NEW.state!='pending'
BEGIN
    DELETE FROM digest_pending_manifest_receipts WHERE seat_id=NEW.seat_id AND preparation_id=(SELECT preparation_id FROM send_manifests WHERE message_id=NEW.message_id);
    DELETE FROM digest_open_warnings WHERE condition_kind='receipt' AND condition_id=length(CAST(NEW.message_id AS BLOB))||':'||NEW.message_id||':'||NEW.seat_id;
END;

CREATE TRIGGER digest_receipt_state_settled_update AFTER UPDATE OF state ON receipt_state WHEN NEW.state!='pending'
BEGIN
    DELETE FROM digest_pending_manifest_receipts WHERE seat_id=NEW.seat_id AND preparation_id=(SELECT preparation_id FROM send_manifests WHERE message_id=NEW.message_id);
    DELETE FROM digest_open_warnings WHERE condition_kind='receipt' AND condition_id=length(CAST(NEW.message_id AS BLOB))||':'||NEW.message_id||':'||NEW.seat_id;
END;

CREATE TRIGGER digest_physical_receipt_settled AFTER UPDATE OF state ON receipts WHEN NEW.state!='pending' AND NOT EXISTS (SELECT 1 FROM send_manifests sm JOIN prepared_recipients pr ON pr.preparation_id=sm.preparation_id WHERE sm.message_id=NEW.message_id AND pr.seat_id=NEW.seat_id)
BEGIN
    DELETE FROM digest_open_warnings WHERE condition_kind='receipt' AND condition_id=length(CAST(NEW.message_id AS BLOB))||':'||NEW.message_id||':'||NEW.seat_id;
END;

CREATE TRIGGER digest_open_warning_job AFTER INSERT ON warning_jobs WHEN NEW.condition_kind IN ('invitation','receipt')
BEGIN
    INSERT OR IGNORE INTO digest_open_warnings(source, source_ordinal, warning_id, thread_id, affected_seat_id, condition_kind, condition_id, episode) VALUES ('job', NEW.ordinal, NEW.warning_id, NEW.thread_id, NEW.affected_seat_id, NEW.condition_kind, NEW.condition_id, NULL);
END;

CREATE TRIGGER digest_open_warning_prepared AFTER INSERT ON prepared_unavailable_warnings
BEGIN
    INSERT OR IGNORE INTO digest_open_warnings(source, source_ordinal, warning_id, thread_id, affected_seat_id, condition_kind, condition_id, episode) SELECT 'prepared', NEW.ordinal, NEW.warning_id, p.thread_id, NEW.affected_seat_id, 'unavailable', NEW.warning_key, NEW.unavailability_episode FROM send_preparations p JOIN seats s ON s.id=NEW.affected_seat_id WHERE p.id=NEW.preparation_id AND s.state!='retired' AND s.unavailability_open=1 AND s.unavailability_episode=NEW.unavailability_episode;
END;

CREATE TRIGGER digest_open_warning_unprepared AFTER DELETE ON prepared_unavailable_warnings
BEGIN
    DELETE FROM digest_open_warnings WHERE source='prepared' AND source_ordinal=OLD.ordinal;
END;

CREATE TRIGGER digest_unavailability_settled AFTER UPDATE OF state, unavailability_open, unavailability_episode ON seats WHEN NEW.state='retired' OR NEW.unavailability_open=0 OR NEW.unavailability_episode!=OLD.unavailability_episode
BEGIN
    DELETE FROM digest_open_warnings WHERE affected_seat_id=NEW.id AND condition_kind='unavailable' AND (NEW.state='retired' OR NEW.unavailability_open=0 OR episode!=NEW.unavailability_episode);
END;

CREATE TABLE digest_open_warning_recipients (
    seat_id TEXT NOT NULL,
    warning_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    source TEXT NOT NULL,
    source_ordinal INTEGER NOT NULL,
    PRIMARY KEY(seat_id, warning_id)
) STRICT;

CREATE INDEX digest_open_warning_recipients_seat ON digest_open_warning_recipients(seat_id, source, source_ordinal);

CREATE INDEX digest_open_warning_recipients_thread ON digest_open_warning_recipients(seat_id, thread_id, source, source_ordinal);

CREATE INDEX digest_open_warning_recipients_warning ON digest_open_warning_recipients(warning_id);

CREATE TRIGGER digest_open_warning_recipient_projected AFTER INSERT ON warning_recipients
BEGIN
    INSERT OR IGNORE INTO digest_open_warning_recipients(seat_id, warning_id, thread_id, source, source_ordinal) SELECT NEW.seat_id, d.warning_id, d.thread_id, d.source, d.source_ordinal FROM digest_open_warnings d WHERE d.warning_id=NEW.warning_id;
END;

CREATE TRIGGER digest_open_warning_closed AFTER DELETE ON digest_open_warnings
BEGIN
    DELETE FROM digest_open_warning_recipients WHERE warning_id=OLD.warning_id AND source=OLD.source AND source_ordinal=OLD.source_ordinal;
END;

CREATE TABLE digest_programmatic_warnings (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    seat_id TEXT NOT NULL,
    warning_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    event_seq INTEGER NOT NULL,
    event_offset INTEGER NOT NULL,
    UNIQUE(seat_id, warning_id)
) STRICT;

CREATE INDEX digest_programmatic_warnings_seat ON digest_programmatic_warnings(seat_id, ordinal);

CREATE INDEX digest_programmatic_warnings_thread ON digest_programmatic_warnings(seat_id, thread_id, ordinal);

CREATE TABLE digest_notice_offer (
    seat_id TEXT PRIMARY KEY,
    binding_generation INTEGER NOT NULL CHECK(binding_generation >= 0),
    execution_id TEXT NOT NULL,
    offered_ordinal INTEGER NOT NULL CHECK(offered_ordinal >= 0)
) STRICT;

CREATE TRIGGER digest_programmatic_warning_projected AFTER INSERT ON warning_recipients WHEN EXISTS (SELECT 1 FROM service_notification_publications p JOIN messages m ON m.id=p.message_id WHERE p.message_id=NEW.warning_id AND m.kind='warn' AND m.author_kind='programmatic')
BEGIN
    INSERT OR IGNORE INTO digest_programmatic_warnings(seat_id, warning_id, thread_id, event_seq, event_offset) SELECT NEW.seat_id, m.id, m.thread_id, m.decision_seq, m.event_offset FROM messages m WHERE m.id=NEW.warning_id;
END;

INSERT OR IGNORE INTO digest_pending_invitations(seat_id, invitation_id, thread_id, created_decision_seq, ordinal) SELECT i.seat_id, i.id, i.thread_id, i.created_decision_seq, i.ordinal FROM invitations i WHERE i.state='pending' AND NOT EXISTS (SELECT 1 FROM invitation_cancellations c WHERE c.invitation_id=i.id);

INSERT OR IGNORE INTO digest_pending_manifest_receipts(seat_id, preparation_id, thread_id, ordinal, decision_seq) SELECT pr.seat_id, pr.preparation_id, pr.thread_id, pr.ordinal, CASE WHEN rs.state='pending' THEN sm.decision_seq END FROM prepared_recipients pr LEFT JOIN send_manifests sm ON sm.preparation_id=pr.preparation_id LEFT JOIN receipt_state rs ON rs.message_id=sm.message_id AND rs.seat_id=pr.seat_id WHERE rs.state IS NULL OR rs.state='pending';

INSERT OR IGNORE INTO digest_open_warnings(source, source_ordinal, warning_id, thread_id, affected_seat_id, condition_kind, condition_id, episode) SELECT 'job', j.ordinal, j.warning_id, j.thread_id, j.affected_seat_id, j.condition_kind, j.condition_id, NULL FROM warning_jobs j LEFT JOIN messages m ON m.id=j.warning_id WHERE (j.condition_kind='invitation' AND EXISTS (SELECT 1 FROM invitations i WHERE i.id=j.condition_id AND i.state='pending' AND NOT EXISTS (SELECT 1 FROM invitation_cancellations c WHERE c.invitation_id=i.id))) OR (j.condition_kind='receipt' AND NOT EXISTS (SELECT 1 FROM receipt_state rs WHERE rs.message_id=m.source_message_id AND rs.seat_id=j.affected_seat_id AND rs.state!='pending') AND NOT EXISTS (SELECT 1 FROM receipts r WHERE r.message_id=m.source_message_id AND r.seat_id=j.affected_seat_id AND r.state!='pending' AND NOT EXISTS (SELECT 1 FROM send_manifests sm JOIN prepared_recipients pr ON pr.preparation_id=sm.preparation_id WHERE sm.message_id=r.message_id AND pr.seat_id=r.seat_id)));

INSERT OR IGNORE INTO digest_open_warnings(source, source_ordinal, warning_id, thread_id, affected_seat_id, condition_kind, condition_id, episode) SELECT 'prepared', w.ordinal, w.warning_id, p.thread_id, w.affected_seat_id, 'unavailable', w.warning_key, w.unavailability_episode FROM prepared_unavailable_warnings w JOIN send_preparations p ON p.id=w.preparation_id JOIN seats s ON s.id=w.affected_seat_id WHERE s.state!='retired' AND s.unavailability_open=1 AND s.unavailability_episode=w.unavailability_episode;

INSERT OR IGNORE INTO digest_open_warning_recipients(seat_id, warning_id, thread_id, source, source_ordinal) SELECT wr.seat_id, d.warning_id, d.thread_id, d.source, d.source_ordinal FROM digest_open_warnings d JOIN warning_recipients wr ON wr.warning_id=d.warning_id;

INSERT OR IGNORE INTO digest_programmatic_warnings(seat_id, warning_id, thread_id, event_seq, event_offset) SELECT wr.seat_id, m.id, m.thread_id, m.decision_seq, m.event_offset FROM warning_recipients wr JOIN service_notification_publications p ON p.message_id=wr.warning_id JOIN messages m ON m.id=wr.warning_id WHERE m.kind='warn' AND m.author_kind='programmatic' ORDER BY m.decision_seq, m.event_offset, wr.seat_id;

INSERT OR IGNORE INTO digest_notice_offer(seat_id, binding_generation, execution_id, offered_ordinal) SELECT o.seat_id, o.binding_generation, o.execution_id, (SELECT COALESCE(MAX(d.ordinal),0) FROM digest_programmatic_warnings d WHERE d.seat_id=o.seat_id AND d.event_seq<=o.offered_through_seq) FROM warning_offer o JOIN seats s ON s.id=o.seat_id JOIN occupant_bindings b ON b.seat_id=s.id AND b.generation=s.generation AND b.ended_at IS NULL WHERE o.binding_generation=s.generation AND o.execution_id=b.execution_id;
