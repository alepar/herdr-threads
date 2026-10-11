-- Per-binding attention frontiers (ht-54r, spec §2 Hook notifications). Executed in one
-- transaction by schema::migrate_v31_to_v32.
-- The warning offer frontier and the informational notice frontier were one row per seat, scoped
-- to the seat's single occupant (binding generation and execution). Each is now one row per
-- binding, so two sessions of one seat each keep their own frontier and a check-in by one never
-- settles or resets the other's. The binding id replaces the generation/execution pair.
-- An old row moves to the binding of the occupant it named (same generation and execution):
-- the open migrated binding when there is one, else the newest such binding row. A row whose
-- occupant has no binding row is dropped: the old rules already treated a frontier recorded for
-- another occupant as covering nothing.
CREATE TABLE warning_offer_v32 (
    binding_id TEXT PRIMARY KEY REFERENCES occupant_bindings(binding_id),
    seat_id TEXT NOT NULL REFERENCES seats(id),
    offered_through_seq INTEGER NOT NULL DEFAULT 0 CHECK(offered_through_seq >= 0)
) STRICT;
INSERT INTO warning_offer_v32(binding_id, seat_id, offered_through_seq)
SELECT b.binding_id, o.seat_id, o.offered_through_seq
FROM warning_offer o JOIN occupant_bindings b ON b.binding_id=(SELECT x.binding_id FROM occupant_bindings x WHERE x.seat_id=o.seat_id AND x.generation=o.binding_generation AND x.execution_id=o.execution_id ORDER BY (x.migrated=1 AND x.ended_at IS NULL) DESC, x.ordinal DESC LIMIT 1);
DROP TABLE warning_offer;
ALTER TABLE warning_offer_v32 RENAME TO warning_offer;
CREATE INDEX warning_offer_seat ON warning_offer(seat_id);

CREATE TABLE digest_notice_offer_v32 (
    binding_id TEXT PRIMARY KEY REFERENCES occupant_bindings(binding_id),
    seat_id TEXT NOT NULL REFERENCES seats(id),
    offered_ordinal INTEGER NOT NULL CHECK(offered_ordinal >= 0)
) STRICT;
INSERT INTO digest_notice_offer_v32(binding_id, seat_id, offered_ordinal)
SELECT b.binding_id, o.seat_id, o.offered_ordinal
FROM digest_notice_offer o JOIN occupant_bindings b ON b.binding_id=(SELECT x.binding_id FROM occupant_bindings x WHERE x.seat_id=o.seat_id AND x.generation=o.binding_generation AND x.execution_id=o.execution_id ORDER BY (x.migrated=1 AND x.ended_at IS NULL) DESC, x.ordinal DESC LIMIT 1);
DROP TABLE digest_notice_offer;
ALTER TABLE digest_notice_offer_v32 RENAME TO digest_notice_offer;
CREATE INDEX digest_notice_offer_seat ON digest_notice_offer(seat_id);
