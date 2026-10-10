-- Immutable canonical automatic pair identity; ordinary channels are not adopted.
-- Restrictive FKs retain the pair while its seats and conversation history exist.
CREATE TABLE direct_threads (
    instance_id TEXT NOT NULL REFERENCES host_instances(id),
    seat_a TEXT NOT NULL REFERENCES seats(id),
    seat_b TEXT NOT NULL REFERENCES seats(id),
    thread_id TEXT NOT NULL UNIQUE REFERENCES threads(id),
    CHECK(seat_a < seat_b),
    UNIQUE(instance_id, seat_a, seat_b)
) STRICT;
