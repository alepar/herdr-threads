-- Swap the tables journal::copy_rows_with_raw filled into place and recreate
-- every index and trigger of the journal tables (ht-76u6).
DROP TABLE journal;
DROP TABLE journal_forked;
ALTER TABLE journal_new RENAME TO journal;
ALTER TABLE journal_forked_new RENAME TO journal_forked;

CREATE UNIQUE INDEX journal_applied_seq ON journal(applied_seq) WHERE applied_seq IS NOT NULL;
CREATE INDEX journal_unapplied ON journal(apply_state, ingest_seq) WHERE apply_state <> 'applied';
CREATE INDEX journal_creates ON journal(creates) WHERE creates IS NOT NULL;
CREATE INDEX journal_parked_on ON journal(parked_on) WHERE apply_state = 'parked_dep';
CREATE INDEX journal_forked_creates ON journal_forked(creates) WHERE creates IS NOT NULL;

-- Signed data never changes; the projection may be written only while every
-- projection column is NULL (promotion, design section 5).
CREATE TRIGGER journal_envelope_immutable BEFORE UPDATE OF log_ix, counter, prev_hash, hash, signature, raw, ingested_at, hlc, kind, kind_version, deps, creates, needs, payload ON journal
WHEN NEW.log_ix IS NOT OLD.log_ix OR NEW.counter IS NOT OLD.counter OR NEW.prev_hash IS NOT OLD.prev_hash
  OR NEW.hash IS NOT OLD.hash OR NEW.signature IS NOT OLD.signature OR NEW.raw IS NOT OLD.raw
  OR NEW.ingested_at IS NOT OLD.ingested_at
  OR OLD.hlc IS NOT NULL OR OLD.kind IS NOT NULL OR OLD.kind_version IS NOT NULL OR OLD.deps IS NOT NULL
  OR OLD.creates IS NOT NULL OR OLD.needs IS NOT NULL OR OLD.payload IS NOT NULL
BEGIN SELECT RAISE(ABORT, 'journal_envelope_immutable'); END;

CREATE TRIGGER journal_append_only BEFORE DELETE ON journal
BEGIN SELECT RAISE(ABORT, 'journal_append_only'); END;

CREATE TRIGGER journal_applied_terminal BEFORE UPDATE OF apply_state, applied_seq ON journal
WHEN OLD.apply_state = 'applied'
BEGIN SELECT RAISE(ABORT, 'journal_applied_terminal'); END;

CREATE TRIGGER journal_forked_immutable BEFORE UPDATE ON journal_forked
BEGIN SELECT RAISE(ABORT, 'journal_forked_immutable'); END;

CREATE TRIGGER journal_forked_append_only BEFORE DELETE ON journal_forked
BEGIN SELECT RAISE(ABORT, 'journal_forked_append_only'); END;
