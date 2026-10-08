-- Mode is recorded independently of intent, attribution and receipt evidence.
ALTER TABLE messages ADD COLUMN delivery_mode TEXT NOT NULL DEFAULT 'ordinary' CHECK(delivery_mode IN ('ordinary','lazy'));
ALTER TABLE send_preparations ADD COLUMN delivery_mode TEXT NOT NULL DEFAULT 'ordinary' CHECK(delivery_mode IN ('ordinary','lazy'));

CREATE TRIGGER lazy_preparation_mode_immutable BEFORE UPDATE OF delivery_mode ON send_preparations
WHEN NEW.delivery_mode IS NOT OLD.delivery_mode
BEGIN SELECT RAISE(ABORT, 'delivery mode is immutable'); END;

CREATE TRIGGER lazy_message_mode_immutable BEFORE UPDATE OF delivery_mode ON messages
WHEN NEW.delivery_mode IS NOT OLD.delivery_mode
BEGIN SELECT RAISE(ABORT, 'recorded message mode is immutable'); END;

CREATE TRIGGER lazy_message_shape BEFORE INSERT ON messages
WHEN NEW.delivery_mode='lazy' AND NEW.kind<>'ordinary'
BEGIN SELECT RAISE(ABORT, 'lazy message must be ordinary content'); END;

-- Message IDs precede publication, so only preparation/thread/seat are foreign
-- keys here. The manifest guard pins the eventual published message identity.
CREATE TABLE lazy_recipients (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    preparation_id TEXT NOT NULL REFERENCES send_preparations(id),
    message_id TEXT NOT NULL CHECK(length(CAST(message_id AS BLOB)) BETWEEN 1 AND 128),
    thread_id TEXT NOT NULL REFERENCES threads(id),
    seat_id TEXT NOT NULL REFERENCES seats(id),
    state TEXT NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','displayed')),
    UNIQUE(preparation_id,seat_id),
    UNIQUE(message_id,seat_id),
    CHECK(message_id IS CASE
        WHEN substr(preparation_id,1,5)='prep-' AND length(preparation_id)>5 THEN 'msg-'||substr(preparation_id,6)
        WHEN length(preparation_id)=9 AND substr(preparation_id,1,1)='p' AND substr(preparation_id,2) NOT GLOB '*[^A-Za-z0-9]*' THEN 'm'||substr(preparation_id,2)
        ELSE NULL END)
) STRICT;

CREATE INDEX lazy_recipients_pending_seat_ordinal ON lazy_recipients(seat_id,ordinal) WHERE state='pending';

CREATE INDEX lazy_recipients_preparation_ordinal ON lazy_recipients(preparation_id,ordinal);

CREATE TRIGGER lazy_recipient_identity_immutable BEFORE UPDATE OF ordinal,preparation_id,message_id,thread_id,seat_id ON lazy_recipients
BEGIN SELECT RAISE(ABORT, 'lazy recipient identity is immutable'); END;

CREATE TRIGGER lazy_recipient_progress_forward BEFORE UPDATE OF state ON lazy_recipients
WHEN OLD.state='displayed' AND NEW.state<>'displayed'
BEGIN SELECT RAISE(ABORT, 'lazy delivery progress cannot regress'); END;

CREATE TRIGGER lazy_recipient_source BEFORE INSERT ON lazy_recipients
BEGIN
    SELECT RAISE(ABORT, 'lazy recipient source mismatch') WHERE NEW.state<>'pending' OR NOT EXISTS (
        SELECT 1 FROM send_preparations p JOIN seats s ON s.id=NEW.seat_id AND s.instance_id=p.instance_id
        WHERE p.id=NEW.preparation_id AND p.thread_id=NEW.thread_id AND p.delivery_mode='lazy' AND p.status='building'
    ) OR EXISTS(SELECT 1 FROM send_manifests WHERE preparation_id=NEW.preparation_id);
END;

CREATE TRIGGER lazy_recipient_display_published BEFORE UPDATE OF state ON lazy_recipients
WHEN NEW.state='displayed' AND NOT EXISTS (
    SELECT 1 FROM send_manifests sm JOIN messages m ON m.id=sm.message_id
    WHERE sm.preparation_id=OLD.preparation_id AND sm.message_id=OLD.message_id AND sm.thread_id=OLD.thread_id AND m.delivery_mode='lazy'
)
BEGIN SELECT RAISE(ABORT, 'unpublished lazy delivery cannot complete'); END;

CREATE TRIGGER lazy_recipient_published_retained BEFORE DELETE ON lazy_recipients
WHEN EXISTS(SELECT 1 FROM send_manifests WHERE preparation_id=OLD.preparation_id)
BEGIN SELECT RAISE(ABORT, 'published lazy delivery is retained'); END;

CREATE TRIGGER lazy_manifest_source BEFORE INSERT ON send_manifests
BEGIN
    SELECT RAISE(ABORT, 'lazy manifest source mismatch') WHERE
        (SELECT delivery_mode FROM send_preparations WHERE id=NEW.preparation_id) IS NOT (SELECT delivery_mode FROM messages WHERE id=NEW.message_id)
        OR ((SELECT delivery_mode FROM send_preparations WHERE id=NEW.preparation_id)='lazy' AND NEW.message_id IS NOT CASE WHEN substr(NEW.preparation_id,1,5)='prep-' AND length(NEW.preparation_id)>5 THEN 'msg-'||substr(NEW.preparation_id,6) WHEN length(NEW.preparation_id)=9 AND substr(NEW.preparation_id,1,1)='p' AND substr(NEW.preparation_id,2) NOT GLOB '*[^A-Za-z0-9]*' THEN 'm'||substr(NEW.preparation_id,2) ELSE NULL END)
        OR ((SELECT delivery_mode FROM messages WHERE id=NEW.message_id)='lazy' AND NEW.warning_count<>0);
END;
