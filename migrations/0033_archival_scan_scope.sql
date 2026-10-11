-- Archival scan scope (ht-54r.15, spec §3 Archival / decision 11). Executed in one transaction
-- by schema::migrate_v32_to_v33. Migration 0023 (re-created verbatim by 0027 and 0032 for the
-- rebuilt occupant_bindings table) bumped archival_instances.mutation_revision on ANY seat or
-- binding write, so every fingerprint refresh restarted every archival scan. The revision now
-- moves only for membership changes, member liveness and seat lifecycle, and the seat- and
-- binding-level triggers fire only for a seat that appears in a scanned source: a joined
-- member, a pending invitation, a pending receipt or a prepared (logical) receipt recipient.
-- Kept unchanged: every other 0023 fence, the *_insert and *_delete fences of memberships and
-- occupant_bindings, and archival_fence_seats_delete.
DROP TRIGGER archival_fence_seats_insert;
DROP TRIGGER archival_fence_seats_update;
DROP TRIGGER archival_fence_memberships_update;
DROP TRIGGER archival_fence_occupant_bindings_insert;
DROP TRIGGER archival_fence_occupant_bindings_update;

-- A new seat is in no channel, so its INSERT is dropped without replacement.
CREATE TRIGGER archival_fence_seats_update AFTER UPDATE OF state,retired_at,target_id ON seats
WHEN EXISTS(SELECT 1 FROM memberships WHERE seat_id=NEW.id AND state='joined')
  OR EXISTS(SELECT 1 FROM invitations WHERE seat_id=NEW.id AND state='pending')
  OR EXISTS(SELECT 1 FROM receipts WHERE seat_id=NEW.id AND state='pending')
  OR EXISTS(SELECT 1 FROM prepared_recipients WHERE seat_id=NEW.id)
BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1 WHERE instance_id=NEW.instance_id;
END;
CREATE TRIGGER archival_fence_memberships_update AFTER UPDATE OF state ON memberships BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1;
END;
CREATE TRIGGER archival_fence_occupant_bindings_insert AFTER INSERT ON occupant_bindings
WHEN EXISTS(SELECT 1 FROM memberships WHERE seat_id=NEW.seat_id AND state='joined')
  OR EXISTS(SELECT 1 FROM invitations WHERE seat_id=NEW.seat_id AND state='pending')
  OR EXISTS(SELECT 1 FROM receipts WHERE seat_id=NEW.seat_id AND state='pending')
  OR EXISTS(SELECT 1 FROM prepared_recipients WHERE seat_id=NEW.seat_id)
BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1 WHERE instance_id=(SELECT instance_id FROM seats WHERE id=NEW.seat_id);
END;
CREATE TRIGGER archival_fence_occupant_bindings_update AFTER UPDATE OF liveness,ended_at ON occupant_bindings
WHEN EXISTS(SELECT 1 FROM memberships WHERE seat_id=NEW.seat_id AND state='joined')
  OR EXISTS(SELECT 1 FROM invitations WHERE seat_id=NEW.seat_id AND state='pending')
  OR EXISTS(SELECT 1 FROM receipts WHERE seat_id=NEW.seat_id AND state='pending')
  OR EXISTS(SELECT 1 FROM prepared_recipients WHERE seat_id=NEW.seat_id)
BEGIN
 UPDATE archival_instances SET mutation_revision=mutation_revision+1 WHERE instance_id=(SELECT instance_id FROM seats WHERE id=NEW.seat_id);
END;
