# coverage ledger — 2026-10-15-unified-seat-selector
c1 · r1 · GAP · R13 every candidate listed · applied — root goal/D3 amended: list every candidate, safety cap 32 + omitted count; ht-u3ym.1, ht-u3ym.3.2 updated
c2 · r1 · GAP · R12 seat-argument inventory · applied — spec D8b; ht-u3ym.2 acceptance adds clap-tree audit test with reasoned exclusion (--cooperative-seat)
c3 · r1 · UNOWNED-SEAM · docs/help goldens ordering · applied — ht-u3ym.5 now blocked by ht-u3ym.3.3; help/golden updates from the parser flip owned by ht-u3ym.3.2
c4 · r1 · GAP · R9 regression tests · applied — ht-u3ym.4 adds --pane/--require-ack-pane regression rows
c5 · r1 · GAP · R15 resolved line full path · applied — D6 amended; ht-u3ym.1 renderer emits full live path, tested per alias
c6 · r1 · GAP · nickname Unverifiable mapping · applied — D2 amended (conflict, never not_found); ht-u3ym.3.2, ht-u3ym.4 updated
c7 · r1 · GAP · trust: seat repair by name · applied — D9 amended; ht-u3ym.5 adds TRUST-POLICY repair-by-name sentence
c8 · r1 · GAP · R14 narrowing with exact ID/nickname · applied — D2 amended (verify live target inside narrowing; no Herdr read without narrowing); ht-u3ym.3.2, ht-u3ym.4 updated
c9 · r1 · GAP · nickname round-trip from read output · applied — ht-u3ym.4 adds round-trip test (agent nick as read/follow renders it)
c10 · r1 · UNOWNED-SEAM · HostTopology input model · applied — boundary is an existing type (src/host/observation.rs incl. agent_names); consumes line on ht-u3ym.1, builder named on ht-u3ym.3.2; no seam beads needed
c11 · r1 · UNOWNED-SEAM · parse-time checks on placeholder slots · rejected — already owned: ht-u3ym.3.2 skips placeholders at parse time (nested promotion review fix), ht-u3ym.3.3 re-runs post-freeze
c12 · r2 · GAP · R3 host_unavailable owner/test · applied — ht-u3ym.3.2 states the mapping for every name/nickname read; ht-u3ym.4 adds a Herdr-down row
c13 · r2 · GAP · Lookup pane without seat · applied — spec D4 already decides no_seat_for_pane; restated on ht-u3ym.3.2, row on ht-u3ym.4
c14 · r2 · GAP · path round-trip / escaping · applied — spec D6 paste-back rule (unsafe labels render as IDs, no escape grammar); ht-u3ym.1 round-trip test, ht-u3ym.4 row
c15 · r2 · GAP · multiple resolved lines · applied — spec D6 one line per distinct name; ht-u3ym.3.2, ht-u3ym.4
c16 · r2 · UNOWNED-SEAM · handoff --existing narrowing meaning · applied — spec D8a; ht-u3ym.3.3
c17 · r2 · GAP · ID-shaped strings / labels equal to seat IDs · applied — spec D2 'ID-shaped strings' accepted limit; ht-u3ym.3.2, ht-u3ym.4, ht-u3ym.5
c18 · r2 · GAP · structured JSON candidates · applied (decision: message-only, no wire change) — spec D6 'Machine output'; ht-u3ym.4 JSON row
c19 · r2 · GAP · output size budget · applied — spec D6 'Sizes'; ht-u3ym.1 single trailing hint line
c20 · r2 · UNOWNED-SEAM · nickname no-read wording vs host down · applied — spec D2 'Which forms read Herdr'; ht-u3ym.3.2
c21 · r2 · UNOWNED-SEAM · TRUST-POLICY same-commit · rejected — the AGENTS.md same-commit rule applies to weakening an invariant or adding an accepted limit; spec D9 decides no invariant is weakened and the sentences are clarifying, so they stay with the docs bead
c22 · r2 · UNOWNED-SEAM · agy plugin skill copy · applied — ht-u3ym.5 owns confirming and updating any derived copy
c23 · r2 · GAP · tiers end to end / no caller · applied — ht-u3ym.4 tier rows
g1 · graph · GRAPH-EDGE · ht-u3ym.3.3 <- ht-u3ym.3.2 · kept — consumes seat_resolution.rs stage and parser flip
g2 · graph · GRAPH-EDGE · ht-u3ym.3.2 <- ht-u3ym.3.1 · kept — consumes extracted helpers
g3 · graph · GRAPH-EDGE · ht-u3ym.3.2 <- ht-u3ym.2 · kept — consumes SeatSlot/SeatMapping
g4 · graph · GRAPH-EDGE · ht-u3ym.3.2 <- ht-u3ym.1 · kept — consumes seat_selector API
g5 · graph · GRAPH-EDGE · ht-u3ym.5 <- ht-u3ym.3.3 · parked — narrow to ht-u3ym.3.2 (safe no; lets docs overlap 3.3, not on the critical path)
g6 · graph · GRAPH-EDGE · ht-u3ym.4 <- ht-u3ym.3.3 · kept — tests what 3.3 delivers
g7 · graph · PROPOSAL · ht-u3ym.3.1, ht-u3ym.3.2 · parked — fold the move-only helper extraction into 3.2 (depth 5→4), never safe
