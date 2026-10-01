-- Service identity is independent of native seats and their bindings.
CREATE TABLE service_authors (
    id TEXT PRIMARY KEY CHECK(length(CAST(id AS BLOB)) BETWEEN 1 AND 128),
    instance_id TEXT NOT NULL UNIQUE REFERENCES host_instances(id),
    reserved_name TEXT NOT NULL DEFAULT 'herdr-graph' CHECK(reserved_name = 'herdr-graph'),
    created_at INTEGER NOT NULL
) STRICT;
CREATE TRIGGER service_authors_immutable BEFORE UPDATE ON service_authors
BEGIN SELECT RAISE(ABORT, 'service author is immutable'); END;
CREATE TRIGGER service_authors_retained BEFORE DELETE ON service_authors
BEGIN SELECT RAISE(ABORT, 'service author is retained'); END;

ALTER TABLE threads ADD COLUMN managed_owner_author_id TEXT REFERENCES service_authors(id);
CREATE INDEX threads_managed_owner ON threads(managed_owner_author_id, ordinal)
    WHERE managed_owner_author_id IS NOT NULL;

CREATE TRIGGER threads_managed_owner_instance_insert
BEFORE INSERT ON threads
WHEN NEW.managed_owner_author_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'managed owner instance mismatch')
    WHERE NOT EXISTS (
        SELECT 1 FROM service_authors a
        WHERE a.id = NEW.managed_owner_author_id AND a.instance_id = NEW.instance_id
    );
END;
CREATE TRIGGER threads_managed_owner_instance_update
BEFORE UPDATE OF managed_owner_author_id, instance_id ON threads
WHEN NEW.managed_owner_author_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'managed owner instance mismatch')
    WHERE NOT EXISTS (
        SELECT 1 FROM service_authors a
        WHERE a.id = NEW.managed_owner_author_id AND a.instance_id = NEW.instance_id
    );
END;
CREATE TRIGGER threads_managed_owner_immutable
BEFORE UPDATE OF managed_owner_author_id ON threads
WHEN OLD.managed_owner_author_id IS NOT NULL
    AND NEW.managed_owner_author_id IS NOT OLD.managed_owner_author_id
BEGIN
    SELECT RAISE(ABORT, 'managed owner is immutable');
END;

-- A nullable explicit kind lets preexisting native/built-in writers retain
-- their original actor fields. Readers derive those legacy kinds from the
-- immutable actor seat when this field is NULL.
ALTER TABLE messages ADD COLUMN author_kind TEXT
    CHECK(author_kind IN ('native', 'programmatic', 'built_in'));
ALTER TABLE messages ADD COLUMN author_service_id TEXT REFERENCES service_authors(id);
DROP TRIGGER messages_immutable;
UPDATE messages SET author_kind = CASE
    WHEN actor_seat_id IS NOT NULL THEN 'native' ELSE 'built_in' END;
CREATE TRIGGER messages_immutable BEFORE UPDATE ON messages
BEGIN SELECT RAISE(ABORT, 'message history is immutable'); END;
CREATE INDEX messages_author_service ON messages(author_service_id, ordinal)
    WHERE author_service_id IS NOT NULL;
CREATE TRIGGER messages_author_shape_insert
BEFORE INSERT ON messages
BEGIN
    SELECT RAISE(ABORT, 'invalid message author') WHERE
        (NEW.author_kind = 'programmatic' AND (
            NEW.author_service_id IS NULL OR NEW.actor_seat_id IS NOT NULL OR
            NOT EXISTS (SELECT 1 FROM service_authors a
                WHERE a.id = NEW.author_service_id AND a.instance_id = NEW.instance_id)
        )) OR
        (NEW.author_kind IS NOT NULL AND NEW.author_kind <> 'programmatic'
            AND NEW.author_service_id IS NOT NULL) OR
        (NEW.author_kind = 'native' AND NEW.actor_seat_id IS NULL) OR
        (NEW.author_kind = 'built_in' AND NEW.actor_seat_id IS NOT NULL) OR
        (NEW.author_kind IS NULL AND NEW.author_service_id IS NOT NULL);
END;

-- An episode is separate from memberships, so joined voluntary membership
-- can coexist with a pending required confirmation.
CREATE TABLE requirement_episodes (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE CHECK(length(CAST(id AS BLOB)) BETWEEN 1 AND 128),
    thread_id TEXT NOT NULL REFERENCES threads(id),
    seat_id TEXT NOT NULL REFERENCES seats(id),
    issuer_author_id TEXT NOT NULL REFERENCES service_authors(id),
    invitation_id TEXT NOT NULL UNIQUE REFERENCES invitations(id),
    revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0),
    state TEXT NOT NULL CHECK(state IN ('pending', 'accepted', 'released', 'retired')),
    created_decision_seq INTEGER NOT NULL CHECK(created_decision_seq > 0),
    created_at INTEGER NOT NULL,
    accepted_by_seat_id TEXT REFERENCES seats(id),
    accepted_generation INTEGER CHECK(accepted_generation >= 0),
    accepted_observation TEXT CHECK(accepted_observation IS NULL OR length(CAST(accepted_observation AS BLOB)) BETWEEN 1 AND 512),
    accepted_at INTEGER,
    released_at INTEGER,
    retired_at INTEGER,
    CHECK(state <> 'accepted' OR (
        accepted_by_seat_id IS NOT NULL AND accepted_by_seat_id = seat_id
        AND accepted_generation IS NOT NULL
        AND accepted_observation IS NOT NULL AND accepted_at IS NOT NULL
    )),
    CHECK(state <> 'pending' OR (
        accepted_by_seat_id IS NULL AND accepted_generation IS NULL
        AND accepted_observation IS NULL AND accepted_at IS NULL
        AND released_at IS NULL AND retired_at IS NULL
    )),
    CHECK((accepted_at IS NULL AND accepted_by_seat_id IS NULL
        AND accepted_generation IS NULL AND accepted_observation IS NULL)
        OR (accepted_at IS NOT NULL AND accepted_by_seat_id IS NOT NULL
        AND accepted_by_seat_id = seat_id
        AND accepted_generation IS NOT NULL AND accepted_observation IS NOT NULL)),
    CHECK(state <> 'released' OR released_at IS NOT NULL),
    CHECK(state <> 'retired' OR retired_at IS NOT NULL)
) STRICT;
CREATE UNIQUE INDEX requirement_episodes_effective
    ON requirement_episodes(thread_id, seat_id)
    WHERE state IN ('pending', 'accepted');
CREATE INDEX requirement_episodes_seat_pending
    ON requirement_episodes(seat_id, ordinal)
    WHERE state = 'pending';
CREATE INDEX requirement_episodes_thread_seat
    ON requirement_episodes(thread_id, seat_id, ordinal);

CREATE TRIGGER requirement_episodes_owner_insert BEFORE INSERT ON requirement_episodes
BEGIN
    SELECT RAISE(ABORT, 'requirement relation mismatch') WHERE NOT EXISTS (
        SELECT 1 FROM threads t
        JOIN seats s ON s.id = NEW.seat_id AND s.instance_id = t.instance_id
        JOIN invitations i ON i.id = NEW.invitation_id
            AND i.thread_id = NEW.thread_id AND i.seat_id = NEW.seat_id
        WHERE t.id = NEW.thread_id AND t.managed_owner_author_id = NEW.issuer_author_id
            AND s.state <> 'retired'
    );
END;
CREATE TRIGGER requirement_episodes_identity_immutable
BEFORE UPDATE OF id, thread_id, seat_id, issuer_author_id, invitation_id,
    created_decision_seq, created_at ON requirement_episodes
BEGIN
    SELECT RAISE(ABORT, 'requirement identity immutable');
END;
CREATE TRIGGER requirement_episodes_revision_forward
BEFORE UPDATE ON requirement_episodes
WHEN NEW.revision <= OLD.revision
BEGIN
    SELECT RAISE(ABORT, 'requirement revision must advance');
END;
-- The first complete native acceptance tuple belongs to the pending→accepted
-- decision. Later revisions and terminal transitions retain that exact tuple.
CREATE TRIGGER requirement_episodes_acceptance_provenance
BEFORE UPDATE ON requirement_episodes
WHEN (
    OLD.accepted_at IS NULL AND
    (NEW.accepted_by_seat_id IS NOT NULL OR NEW.accepted_generation IS NOT NULL OR
     NEW.accepted_observation IS NOT NULL OR NEW.accepted_at IS NOT NULL) AND
    NOT (OLD.state = 'pending' AND NEW.state = 'accepted')
) OR (
    OLD.accepted_at IS NOT NULL AND
    (NEW.accepted_by_seat_id IS NOT OLD.accepted_by_seat_id OR
     NEW.accepted_generation IS NOT OLD.accepted_generation OR
     NEW.accepted_observation IS NOT OLD.accepted_observation OR
     NEW.accepted_at IS NOT OLD.accepted_at)
)
BEGIN
    SELECT RAISE(ABORT, 'requirement acceptance provenance is immutable');
END;
CREATE TRIGGER requirement_episodes_state_forward
BEFORE UPDATE ON requirement_episodes
WHEN OLD.state IN ('released', 'retired') OR
    (OLD.state = 'accepted' AND NEW.state NOT IN ('accepted', 'released', 'retired'))
BEGIN
    SELECT RAISE(ABORT, 'requirement state is terminal or cannot reverse');
END;
CREATE TRIGGER requirement_episodes_retained BEFORE DELETE ON requirement_episodes
BEGIN SELECT RAISE(ABORT, 'requirement episode is retained'); END;
