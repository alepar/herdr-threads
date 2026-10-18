-- Opaque journal entries (ht-j87z, peer wire compat hardening, design section 4):
-- verified entries whose signed layout this node does not understand. A row
-- is a chain position (branch = 0) or a conflicting-branch entry (branch = 1,
-- as journal_forked holds for known entries). Guard triggers, the promotion
-- marker and the store API come with ht-j87z.14.
CREATE TABLE journal_opaque (
    log_ix INTEGER NOT NULL REFERENCES sync_logs(log_ix),
    counter INTEGER NOT NULL CHECK(counter >= 2),
    hash BLOB NOT NULL CHECK(length(hash) = 32),
    prev_hash BLOB NOT NULL CHECK(length(prev_hash) = 32),
    signature BLOB NOT NULL CHECK(length(signature) = 64),
    raw BLOB NOT NULL CHECK(length(raw) BETWEEN 68 AND 393216),
    branch INTEGER NOT NULL CHECK(branch IN (0,1)),
    PRIMARY KEY(log_ix, counter, hash)
) STRICT, WITHOUT ROWID;

CREATE UNIQUE INDEX journal_opaque_position ON journal_opaque(log_ix, counter) WHERE branch = 0;

-- One row per in-flight promotion, inside promote_opaque's transaction only;
-- always empty at rest (verify_invariants checks it).
CREATE TABLE journal_opaque_promoting (
    log_ix INTEGER NOT NULL, counter INTEGER NOT NULL, hash BLOB NOT NULL,
    PRIMARY KEY(log_ix, counter, hash)
) STRICT, WITHOUT ROWID;

CREATE TRIGGER journal_opaque_immutable BEFORE UPDATE ON journal_opaque
BEGIN SELECT RAISE(ABORT, 'journal_opaque_immutable'); END;

CREATE TRIGGER journal_opaque_append_only BEFORE DELETE ON journal_opaque
WHEN NOT EXISTS (SELECT 1 FROM journal_opaque_promoting p
                 WHERE p.log_ix = OLD.log_ix AND p.counter = OLD.counter AND p.hash = OLD.hash)
BEGIN SELECT RAISE(ABORT, 'journal_opaque_append_only'); END;

CREATE TRIGGER journal_opaque_position_taken BEFORE INSERT ON journal_opaque
WHEN NEW.branch = 0 AND EXISTS (SELECT 1 FROM journal j WHERE j.log_ix = NEW.log_ix AND j.counter = NEW.counter)
BEGIN SELECT RAISE(ABORT, 'journal_opaque_position_taken'); END;

CREATE TRIGGER journal_position_taken_by_opaque BEFORE INSERT ON journal
WHEN EXISTS (SELECT 1 FROM journal_opaque o WHERE o.log_ix = NEW.log_ix AND o.counter = NEW.counter AND o.branch = 0)
BEGIN SELECT RAISE(ABORT, 'journal_opaque_position_taken'); END;
