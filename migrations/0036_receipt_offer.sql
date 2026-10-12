-- Per-binding receipt presentation (ht-705). Executed in one transaction by
-- schema::migrate_v35_to_v36.
-- One row per (binding, message): a top-level binding's own display (`AdvanceCursors`) showed the
-- message whole while the seat's receipt for it was pending. The receipt stays pending (no ACK);
-- the row only tells the wake candidate that this occupant was already served in-turn, so an idle
-- wake is not owed to it for that receipt (delivery contract rule 4). A lifecycle check-in of the
-- binding starts it over, as it does the warning offer frontier.
CREATE TABLE receipt_offer (
    binding_id TEXT NOT NULL REFERENCES occupant_bindings(binding_id),
    seat_id TEXT NOT NULL REFERENCES seats(id),
    message_id TEXT NOT NULL,
    PRIMARY KEY (binding_id, message_id)
) STRICT;
CREATE INDEX receipt_offer_seat ON receipt_offer(seat_id);
