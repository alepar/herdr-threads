-- Opaque journal entries (ht-j87z, peer wire compat hardening, design section 4):
-- verified entries whose signed layout this node does not understand. A row
-- is a chain position (branch = 0) or a conflicting-branch entry (branch = 1,
-- as journal_forked holds for known entries). Guard triggers, the promotion
-- marker and the store API come with ht-j87z.3 in this same delta.
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
