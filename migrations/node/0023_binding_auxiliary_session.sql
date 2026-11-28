-- Sticky auxiliary-session mark of a binding (ht-z41e.21). A harness adapter reports hook evidence
-- that a session is auxiliary (Codex: a null `transcript_path`, which every `/side` session emits);
-- the store sets this bit on the binding row and never clears it. Only queue root selection reads
-- it; `concurrent_session` stays set by an independent fork alone, so liveness, dormant-target and
-- successor logic are untouched. A new binding row starts at 0.
ALTER TABLE occupant_bindings ADD COLUMN auxiliary_session INTEGER NOT NULL DEFAULT 0
    CHECK(auxiliary_session IN (0,1));
