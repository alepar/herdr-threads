-- Final unreleased queue-wake schema; historical candidate DDL is a test fixture.

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
 ids_complete INTEGER NOT NULL DEFAULT 0 CHECK(ids_complete IN (0,1)), logical_status TEXT NOT NULL DEFAULT 'active' CHECK(logical_status IN ('active','fulfilled','scope_ended')), fulfillment_turn TEXT, fulfillment_ordinal INTEGER, arm_native_id TEXT, arm_native_ordinal INTEGER, helper_json BLOB CHECK(helper_json IS NULL OR length(helper_json)<=2048), cleanup_list_attempted INTEGER NOT NULL DEFAULT 0 CHECK(cleanup_list_attempted IN (0,1)), native_no_add_exception INTEGER NOT NULL DEFAULT 0 CHECK(native_no_add_exception IN (0,1)), add_helper_json BLOB CHECK(add_helper_json IS NULL OR length(add_helper_json)<=2048),
 armed_at_utc INTEGER,
 native_degraded_at_utc INTEGER,
 native_fallback_json BLOB CHECK(native_fallback_json IS NULL OR length(native_fallback_json)<=2048),
 UNIQUE(instance_id,attempt),
 CHECK((invitation_seq IS NULL)=(invitation_offset IS NULL)),
 CHECK((receipt_seq IS NULL)=(receipt_offset IS NULL)),
 CHECK((warning_seq IS NULL)=(warning_offset IS NULL)),
 CHECK(phase NOT IN ('armed','accepted','unknown') OR arm_nonce IS NOT NULL)
) STRICT;

CREATE UNIQUE INDEX wake_intents_unresolved_cohort ON wake_intents(instance_id,adapter,replay_scope)
 WHERE (logical_status='active' AND phase!='no_effect') OR cleanup_quarantined=1;

CREATE INDEX wake_intents_origin_seat ON wake_intents(instance_id,origin_seat,ordinal)
 WHERE (logical_status='active' AND phase!='no_effect') OR cleanup_quarantined=1;

CREATE TABLE wake_intent_ids (
 intent_ordinal INTEGER NOT NULL REFERENCES wake_intents(ordinal) ON DELETE CASCADE,
 queue_id TEXT NOT NULL CHECK(length(CAST(queue_id AS BLOB)) BETWEEN 1 AND 128), cleanup_attempted INTEGER NOT NULL DEFAULT 0 CHECK(cleanup_attempted IN (0,1)),
 PRIMARY KEY(intent_ordinal,queue_id)
) STRICT, WITHOUT ROWID;

CREATE TABLE native_turn_state (
 binding_id TEXT PRIMARY KEY REFERENCES occupant_bindings(binding_id) ON DELETE CASCADE,
 instance_id TEXT NOT NULL,
 scope_json BLOB NOT NULL CHECK(length(scope_json)<=4096),
 turn_id TEXT NOT NULL CHECK(length(CAST(turn_id AS BLOB)) BETWEEN 1 AND 128),
 phase TEXT NOT NULL CHECK(phase IN ('started','ended','end_first','unknown')),
 observation_ordinal INTEGER NOT NULL CHECK(observation_ordinal>0)
) STRICT, WITHOUT ROWID;

CREATE INDEX native_turn_state_instance ON native_turn_state(instance_id);

CREATE TRIGGER native_turn_binding_end AFTER UPDATE OF ended_at ON occupant_bindings
 WHEN NEW.ended_at IS NOT NULL BEGIN
 DELETE FROM native_turn_state WHERE binding_id=NEW.binding_id;
END;
