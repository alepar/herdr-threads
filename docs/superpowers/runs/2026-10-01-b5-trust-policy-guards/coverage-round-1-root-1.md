requirements:
R1 → ht-rzi.1
R2 → ht-rzi.1
R3 → ht-rzi.1
R4 → ht-rzi.1
R5 → ht-rzi.2
R6 → ht-rzi.5
R7 → ht-rzi.5
R8 → ht-rzi.3
R9 → ht-rzi.3
R10 → ht-rzi.4
R11 → ht-rzi.4
R12 → ht-rzi.6
R13 → ht-rzi.4
R14 → ht-rzi.5
R15 → ht-rzi.6
R16 → ht-rzi.1, ht-rzi.3
R-new: TRUST-POLICY.md marks the required guards implemented once shipped → (unmapped)
R-new: hold-lift predicate runs in every seat-resolving transaction listed in decision 1 → ht-rzi.1, ht-rzi.2
R-new: end-to-end F6 restore scenario tested (incarnation change → held → resumed session reattaches → collision via retire/--replace → hold lifts) → (unmapped)

findings:
1. GAP · docs for ht-rzi.2/.4/.5 behaviour and README unowned (R16 partial) — fix: docs-sweep leaf or add docs to .2/.4/.5
2. GAP · ht-rzi.2 weakens decision 3 hint rule to "consider"; no acceptance for hint mismatch, absent hint, fresh session_start_source — fix: amend .2
3. UNOWNED-SEAM · hold-lift predicate ht-rzi.1 → ht-rzi.2 consumed conditionally, no edge — fix: edge .2 blocked-by .1, unconditional call, acceptance test
4. GAP · ht-rzi.1 acceptance tests hold lift only after rebind/retire; decision 1 lists fresh seat, replace, reconciliation retirement — fix: amend .1 acceptance
5. GAP · no end-to-end F6 restore test (walking skeleton) — fix: integration sweep leaf depending on .1 .2 (.5)
6. UNOWNED-SEAM · provenance of open binding after cooperative continuity undecided; .3/.4 key on cooperative_top_level — fix: binding stays cooperative_top_level, continuity only in seat history; acceptance in .2 and .3
7. GAP · TRUST-POLICY.md status ("not yet implemented", "today ...") not updated by any task — fix: docs sweep
8. UNEXERCISED-CONFIGURATION · per-harness coverage: .3 lacks CODEX_* and Herdr-agent cases; .2 lacks per-harness (Claude, Codex w/o hint) tests; .4 W9-2 both directions — fix: extend acceptance
