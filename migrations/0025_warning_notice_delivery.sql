-- Built-in condition transitions use the same bounded, occupant-scoped
-- informational delivery page as programmatic service warnings. Preserve the
-- original projection name/indexes and all immutable warning history.
CREATE TRIGGER digest_transition_warning_projected AFTER INSERT ON warning_recipients WHEN EXISTS (SELECT 1 FROM warning_conditions c WHERE c.open_warning_id=NEW.warning_id OR c.clear_warning_id=NEW.warning_id)
BEGIN
    INSERT OR IGNORE INTO digest_programmatic_warnings(seat_id,warning_id,thread_id,event_seq,event_offset) SELECT NEW.seat_id,m.id,m.thread_id,m.decision_seq,m.event_offset FROM messages m WHERE m.id=NEW.warning_id UNION ALL SELECT NEW.seat_id,w.warning_id,sm.thread_id,sm.decision_seq,w.warning_offset FROM prepared_unavailable_warnings w JOIN send_manifests sm ON sm.preparation_id=w.preparation_id WHERE w.warning_id=NEW.warning_id AND NOT EXISTS(SELECT 1 FROM messages m WHERE m.id=w.warning_id);
    -- Delete the shared legacy parent, not just this recipient's projection:
    -- a later-running legacy recipient trigger then has nothing to reinsert.
    -- Other recipients awaiting attribution remain in warning_jobs backlog.
    DELETE FROM digest_open_warnings WHERE warning_id=NEW.warning_id AND EXISTS(SELECT 1 FROM digest_programmatic_warnings d WHERE d.seat_id=NEW.seat_id AND d.warning_id=NEW.warning_id);
END;

-- The old global decision watermark cannot prove which bounded events were
-- carried. Retain its legacy use, but start these new delivery rows unoffered.
INSERT OR IGNORE INTO digest_programmatic_warnings(seat_id,warning_id,thread_id,event_seq,event_offset) SELECT wr.seat_id,m.id,m.thread_id,m.decision_seq,m.event_offset FROM warning_recipients wr JOIN messages m ON m.id=wr.warning_id WHERE EXISTS (SELECT 1 FROM warning_conditions c WHERE c.open_warning_id=m.id OR c.clear_warning_id=m.id) ORDER BY m.decision_seq,m.event_offset,wr.seat_id;

INSERT OR IGNORE INTO digest_programmatic_warnings(seat_id,warning_id,thread_id,event_seq,event_offset) SELECT wr.seat_id,w.warning_id,sm.thread_id,sm.decision_seq,w.warning_offset FROM warning_recipients wr JOIN prepared_unavailable_warnings w ON w.warning_id=wr.warning_id JOIN send_manifests sm ON sm.preparation_id=w.preparation_id WHERE EXISTS (SELECT 1 FROM warning_conditions c WHERE c.open_warning_id=w.warning_id) AND NOT EXISTS(SELECT 1 FROM messages m WHERE m.id=w.warning_id) ORDER BY sm.decision_seq,w.warning_offset,wr.seat_id;

-- Backfilled canonical transitions no longer belong to legacy pending walks.
-- Keep legacy conditions and all immutable warning/recipient/history rows.
DELETE FROM digest_open_warnings WHERE warning_id IN (SELECT warning_id FROM digest_programmatic_warnings) AND EXISTS(SELECT 1 FROM warning_conditions c WHERE c.open_warning_id=digest_open_warnings.warning_id OR c.clear_warning_id=digest_open_warnings.warning_id);
