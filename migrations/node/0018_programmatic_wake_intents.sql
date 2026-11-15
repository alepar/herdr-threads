-- Frozen queue-effect history survives origin retirement and binding replacement.
CREATE TABLE wake_intents (
 ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
 instance_id TEXT NOT NULL,
 attempt TEXT NOT NULL,
 adapter TEXT NOT NULL,
 replay_scope TEXT NOT NULL CHECK(length(CAST(replay_scope AS BLOB)) BETWEEN 1 AND 128),
 origin_seat TEXT NOT NULL,
 origin_host INTEGER NOT NULL,
 origin_host_boot TEXT NOT NULL,
 origin_host_epoch INTEGER NOT NULL CHECK(origin_host_epoch>=0),
 origin_target TEXT NOT NULL,
 origin_target_generation INTEGER NOT NULL CHECK(origin_target_generation>=0),
 target_json BLOB NOT NULL CHECK(length(target_json)<=4096),
 descriptor BLOB NOT NULL CHECK(length(descriptor)<=16384),
 correlation TEXT NOT NULL CHECK(length(CAST(correlation AS BLOB)) BETWEEN 1 AND 128),
 invitation_seq INTEGER,
 invitation_offset INTEGER,
 receipt_seq INTEGER,
 receipt_offset INTEGER,
 warning_seq INTEGER,
 warning_offset INTEGER,
 lazy_ordinal INTEGER,
 phase TEXT NOT NULL CHECK(phase IN ('prepared','armed','accepted','unknown','no_effect')),
 revision INTEGER NOT NULL CHECK(revision>0),
 owner_boot TEXT NOT NULL,
 arm_nonce TEXT,
 created_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL,
 cleanup_quarantined INTEGER NOT NULL CHECK(cleanup_quarantined IN (0,1)),
 diagnostic TEXT NOT NULL CHECK(length(CAST(diagnostic AS BLOB))<=2048),
 ids_complete INTEGER NOT NULL DEFAULT 0 CHECK(ids_complete IN (0,1)),
 UNIQUE(instance_id,attempt),
 CHECK((invitation_seq IS NULL)=(invitation_offset IS NULL)),
 CHECK((receipt_seq IS NULL)=(receipt_offset IS NULL)),
 CHECK((warning_seq IS NULL)=(warning_offset IS NULL)),
 CHECK(phase NOT IN ('armed','accepted','unknown') OR arm_nonce IS NOT NULL)
) STRICT;
CREATE UNIQUE INDEX wake_intents_unresolved_cohort ON wake_intents(instance_id,adapter,replay_scope)
 WHERE phase!='no_effect' OR cleanup_quarantined=1;
CREATE INDEX wake_intents_origin_seat ON wake_intents(instance_id,origin_seat,ordinal)
 WHERE phase!='no_effect' OR cleanup_quarantined=1;
CREATE TABLE wake_intent_ids (
 intent_ordinal INTEGER NOT NULL REFERENCES wake_intents(ordinal) ON DELETE CASCADE,
 queue_id TEXT NOT NULL CHECK(length(CAST(queue_id AS BLOB)) BETWEEN 1 AND 128),
 PRIMARY KEY(intent_ordinal,queue_id)
) STRICT, WITHOUT ROWID;
