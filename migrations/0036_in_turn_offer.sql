-- Per-binding in-turn presentation (ht-705). Executed in one transaction by
-- schema::migrate_v35_to_v36.
-- One row per (binding, item): a top-level binding's own in-turn display (`AdvanceCursors` with
-- `in_turn`) showed the item whole while it was pending: a message whose seat receipt was pending,
-- or a pending invitation. Nothing is settled (no ACK, no accept); the row only tells the wake
-- candidate that this occupant was already served in-turn, so an idle wake is not owed to it for
-- that item (delivery contract rule 4). A lifecycle check-in of the binding starts it over, as it
-- does the warning offer frontier.
CREATE TABLE in_turn_offer (
    binding_id TEXT NOT NULL REFERENCES occupant_bindings(binding_id),
    seat_id TEXT NOT NULL REFERENCES seats(id),
    item_id TEXT NOT NULL,
    PRIMARY KEY (binding_id, item_id)
) STRICT;
CREATE INDEX in_turn_offer_seat ON in_turn_offer(seat_id);
