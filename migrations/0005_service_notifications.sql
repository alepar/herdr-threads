-- Publication is one row; audience capture and wake projection are bounded.
CREATE INDEX requirement_episodes_thread_ordinal
    ON requirement_episodes(thread_id,ordinal);
CREATE TABLE service_notification_preparations (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    instance_id TEXT NOT NULL REFERENCES host_instances(id),
    author_id TEXT NOT NULL REFERENCES service_authors(id),
    operation_key TEXT NOT NULL,
    digest BLOB NOT NULL CHECK(length(digest)=32),
    thread_id TEXT NOT NULL REFERENCES threads(id),
    membership_revision INTEGER NOT NULL CHECK(membership_revision>=0),
    lifecycle_revision INTEGER NOT NULL CHECK(lifecycle_revision>=0),
    interval_high_water INTEGER NOT NULL CHECK(interval_high_water>=0),
    requirement_high_water INTEGER NOT NULL CHECK(requirement_high_water>=0),
    interval_cursor INTEGER NOT NULL DEFAULT 0 CHECK(interval_cursor>=0),
    requirement_cursor INTEGER NOT NULL DEFAULT 0 CHECK(requirement_cursor>=0),
    recipient_count INTEGER NOT NULL DEFAULT 0 CHECK(recipient_count>=0),
    status TEXT NOT NULL CHECK(status IN ('building','sealed','discarded','published'))
) STRICT;
CREATE INDEX service_notification_preparations_key
    ON service_notification_preparations(instance_id,author_id,operation_key,ordinal);
CREATE TABLE service_notification_recipients (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    preparation_id TEXT NOT NULL REFERENCES service_notification_preparations(id),
    seat_id TEXT NOT NULL REFERENCES seats(id),
    UNIQUE(preparation_id,seat_id)
) STRICT;
CREATE INDEX service_notification_recipients_page
    ON service_notification_recipients(preparation_id,ordinal);
CREATE TABLE service_notification_publications (
    preparation_id TEXT PRIMARY KEY REFERENCES service_notification_preparations(id),
    message_id TEXT NOT NULL UNIQUE REFERENCES messages(id),
    decision_seq INTEGER NOT NULL CHECK(decision_seq>0),
    recipient_count INTEGER NOT NULL CHECK(recipient_count>=0)
) STRICT;
