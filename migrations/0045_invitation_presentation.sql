-- ADD COLUMN preserves retained invitation bytes without rebuilding.
-- The existing STRICT table invokes named-table quick_check over invitation
-- rows/types/existing CHECKs and invitation index pages/bytes, plus catalog/schema
-- work. No added epoch CHECK: canonical readers/writers check range/exhaustion.
ALTER TABLE invitations ADD COLUMN command_context_epoch INTEGER NOT NULL DEFAULT 0;

-- Build only over the already canonical pending projection; no backfill or
-- retained-history rewrite. Upgrade index work scales with pending rows.
CREATE INDEX invitation_content_initial ON digest_pending_invitations(seat_id,ordinal);
CREATE INDEX invitation_content_thread ON digest_pending_invitations(seat_id,thread_id,ordinal);

CREATE TABLE invitation_context_latest (
    thread_id TEXT PRIMARY KEY REFERENCES threads(id),
    instance_id TEXT NOT NULL REFERENCES host_instances(id),
    last_change_seq INTEGER NOT NULL CHECK(last_change_seq > 0),
    context_revision INTEGER NOT NULL CHECK(context_revision > 0)
) STRICT;
CREATE INDEX invitation_context_changed ON invitation_context_latest(instance_id,last_change_seq,thread_id);
CREATE TRIGGER invitation_context_instance_insert BEFORE INSERT ON invitation_context_latest
WHEN NOT EXISTS (SELECT 1 FROM threads t WHERE t.id=NEW.thread_id AND t.instance_id=NEW.instance_id)
BEGIN SELECT RAISE(ABORT, 'invitation context instance mismatch'); END;
CREATE TRIGGER invitation_context_instance_update BEFORE UPDATE ON invitation_context_latest
WHEN NEW.thread_id IS NOT OLD.thread_id OR NEW.instance_id IS NOT OLD.instance_id
BEGIN SELECT RAISE(ABORT, 'invitation context identity immutable'); END;

CREATE TABLE invitation_presentation_consumers (
    binding_id TEXT NOT NULL REFERENCES occupant_bindings(binding_id),
    seat_id TEXT NOT NULL REFERENCES seats(id),
    consumer TEXT NOT NULL CHECK(consumer IN ('hook','watch')),
    instance_id TEXT NOT NULL REFERENCES host_instances(id),
    generation INTEGER NOT NULL CHECK(generation>=0),
    execution TEXT NOT NULL CHECK(length(CAST(execution AS BLOB)) BETWEEN 1 AND 128),
    render_key TEXT NOT NULL CHECK(length(render_key) IN (0,64)),
    initial_after INTEGER NOT NULL CHECK(initial_after>=0),
    initial_high INTEGER NOT NULL CHECK(initial_high>=0),
    context_after_seq INTEGER NOT NULL CHECK(context_after_seq>=0),
    context_after_thread TEXT NOT NULL,
    context_high INTEGER NOT NULL CHECK(context_high>=0),
    context_thread TEXT NOT NULL,
    context_selected_seq INTEGER NOT NULL CHECK(context_selected_seq>=0),
    context_invitation_after INTEGER NOT NULL CHECK(context_invitation_after>=0),
    context_invitation_high INTEGER NOT NULL CHECK(context_invitation_high>=0),
    revisit_after_bytes INTEGER NOT NULL CHECK(revisit_after_bytes>=0),
    revisit_after_ordinal INTEGER NOT NULL CHECK(revisit_after_ordinal>=0),
    revisit_high_bytes INTEGER NOT NULL CHECK(revisit_high_bytes>=0),
    revisit_high_ordinal INTEGER NOT NULL CHECK(revisit_high_ordinal>=0),
    cleanup_after INTEGER NOT NULL CHECK(cleanup_after>=0),
    dirty_after INTEGER NOT NULL CHECK(dirty_after>=0),
    dirty_high INTEGER NOT NULL CHECK(dirty_high>=0),
    rotation INTEGER NOT NULL CHECK(rotation BETWEEN 0 AND 4),
    mod_generation INTEGER NOT NULL CHECK(mod_generation>=0),
    mod_execution TEXT NOT NULL CHECK(length(CAST(mod_execution AS BLOB))<=128),
    mod_initial_after INTEGER NOT NULL CHECK(mod_initial_after>=0),
    mod_initial_high INTEGER NOT NULL CHECK(mod_initial_high>=0),
    mod_context_after_seq INTEGER NOT NULL CHECK(mod_context_after_seq>=0),
    mod_context_high INTEGER NOT NULL CHECK(mod_context_high>=0),
    mod_context_selected_seq INTEGER NOT NULL CHECK(mod_context_selected_seq>=0),
    mod_context_invitation_after INTEGER NOT NULL CHECK(mod_context_invitation_after>=0),
    mod_context_invitation_high INTEGER NOT NULL CHECK(mod_context_invitation_high>=0),
    mod_cleanup_after INTEGER NOT NULL CHECK(mod_cleanup_after>=0),
    mod_cleanup_high INTEGER NOT NULL CHECK(mod_cleanup_high>=0),
    mod_version INTEGER NOT NULL CHECK(mod_version>=0),
    mod_context_after_thread TEXT NOT NULL,
    mod_context_thread TEXT NOT NULL,
    ordinary_version TEXT NOT NULL CHECK(length(ordinary_version)<=20),
    ordinary_done INTEGER NOT NULL CHECK(ordinary_done IN (0,1)),
    ordinary_cursor TEXT CHECK(length(CAST(ordinary_cursor AS BLOB))<=8192),
    ordinary_pending TEXT NOT NULL CHECK(length(CAST(ordinary_pending AS BLOB))<=155731 AND json_valid(ordinary_pending) AND json_type(ordinary_pending)='object' AND json_type(ordinary_pending,'$.items')='array' AND json_array_length(ordinary_pending,'$.items')<=32 AND json_type(ordinary_pending,'$.prefix')='array' AND json_array_length(ordinary_pending,'$.prefix')<=8),
    ordinary_skip_body TEXT CHECK(length(CAST(ordinary_skip_body AS BLOB))<=128),
    ordinary_partial TEXT NOT NULL CHECK(length(CAST(ordinary_partial AS BLOB))<=49415 AND json_valid(ordinary_partial) AND json_type(ordinary_partial)='object'),
    outbox_retired_mask INTEGER NOT NULL CHECK(outbox_retired_mask BETWEEN 0 AND 4294967295),
    outbox_confirmed_mask INTEGER NOT NULL CHECK(outbox_confirmed_mask BETWEEN 0 AND 4294967295 AND (outbox_confirmed_mask & outbox_retired_mask)=outbox_confirmed_mask),
    last_descriptors TEXT NOT NULL CHECK(length(CAST(last_descriptors AS BLOB))<=4096 AND json_valid(last_descriptors) AND json_type(last_descriptors)='array' AND json_array_length(last_descriptors)<=32),
    last_confirmed_mask INTEGER NOT NULL CHECK(last_confirmed_mask BETWEEN 0 AND 4294967295),
    control_state TEXT NOT NULL CHECK(length(CAST(control_state AS BLOB))<=4096 AND json_valid(control_state) AND json_type(control_state)='object'),
    outbox_selection TEXT NOT NULL CHECK(length(CAST(outbox_selection AS BLOB))<=128),
    outbox_codec_key TEXT NOT NULL CHECK(length(outbox_codec_key)<=64),
    outbox_render_key TEXT NOT NULL CHECK(length(outbox_render_key)<=64),
    outbox_prefix TEXT NOT NULL CHECK(length(CAST(outbox_prefix AS BLOB))<=24601 AND json_valid(outbox_prefix) AND json_type(outbox_prefix)='array' AND json_array_length(outbox_prefix)<=8),
    outbox_token TEXT CHECK(outbox_token IS NULL OR (length(outbox_token)=32 AND outbox_token NOT GLOB '*[^0-9a-f]*')),
    last_token TEXT CHECK(last_token IS NULL OR (length(last_token)=32 AND last_token NOT GLOB '*[^0-9a-f]*')),
    outbox_text TEXT NOT NULL CHECK(length(CAST(outbox_text AS BLOB))<=65536),
    outbox_descriptors TEXT NOT NULL CHECK(length(CAST(outbox_descriptors AS BLOB))<=262144 AND json_valid(outbox_descriptors) AND json_type(outbox_descriptors)='array' AND json_array_length(outbox_descriptors)<=32),
    outbox_version TEXT NOT NULL CHECK(length(outbox_version)<=20),
    outbox_more INTEGER NOT NULL CHECK(outbox_more IN (0,1)),
    PRIMARY KEY(binding_id,consumer)
) STRICT;
CREATE UNIQUE INDEX invitation_presentation_token ON invitation_presentation_consumers(outbox_token) WHERE outbox_token IS NOT NULL;
CREATE UNIQUE INDEX invitation_presentation_last_token ON invitation_presentation_consumers(last_token) WHERE last_token IS NOT NULL;

CREATE TABLE invitation_presentation_items (
    binding_id TEXT NOT NULL REFERENCES occupant_bindings(binding_id),
    seat_id TEXT NOT NULL REFERENCES seats(id),
    invitation_id TEXT NOT NULL REFERENCES invitations(id),
    consumer TEXT NOT NULL CHECK(consumer IN ('hook','watch')),
    generation INTEGER NOT NULL CHECK(generation>=0),
    execution TEXT NOT NULL CHECK(length(CAST(execution AS BLOB)) BETWEEN 1 AND 128),
    context_key TEXT NOT NULL CHECK(length(CAST(context_key AS BLOB))<=4096),
    render_key TEXT NOT NULL CHECK(length(render_key) IN (0,64)),
    output_class INTEGER NOT NULL CHECK(output_class BETWEEN 2 AND 4),
    partial_count INTEGER NOT NULL CHECK(partial_count IN (0,1)),
    complete_count INTEGER NOT NULL CHECK(complete_count IN (0,1)),
    required_render_bytes INTEGER NOT NULL CHECK(required_render_bytes>=0),
    revisit_ordinal INTEGER NOT NULL CHECK(revisit_ordinal>0),
    mod_generation INTEGER NOT NULL DEFAULT 0 CHECK(mod_generation>=0),
    mod_execution TEXT NOT NULL DEFAULT '' CHECK(length(CAST(mod_execution AS BLOB))<=128),
    mod_key TEXT NOT NULL CHECK(length(CAST(mod_key AS BLOB))<=4096),
    PRIMARY KEY(binding_id,invitation_id,consumer)
) STRICT;
CREATE INDEX invitation_presentation_revisit ON invitation_presentation_items(binding_id,consumer,generation,required_render_bytes,revisit_ordinal);
CREATE UNIQUE INDEX invitation_presentation_cleanup ON invitation_presentation_items(binding_id,consumer,revisit_ordinal);
