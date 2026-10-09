L1 · r1 · flag-sweep · ht-4ne.1 · kept as leaf — sp:demoted-by-session overrules a PROMOTE (size); contract pinned in spec D2a and the work re-split into ht-4ne.1 (inert seam) + ht-4ne.4 (behavior); re-review judged both LEAF
L2 · r1 · GAP · R2 · applied — ht-4ne.4 acceptance: first seated turn after bare SessionStart gets lifecycle check-in + context, parity with launch-created seat; ht-4ne.3 case 1 asserts context; root epic text corrected (qualification is not launch-bound)
L3 · r1 · GAP · R6 · applied — ht-4ne.4 acceptance: /new reuses seat, Clear on same seat
L4 · r1 · GAP · R7 · applied — ht-4ne.4 acceptance: SessionStart path silent outside Herdr
L5 · r1 · GAP · R9 · applied — ht-4ne.4 acceptance: launch-created seat unchanged, launch/hermes tests green (not in smoke: native launch needs a real hermes)
L6 · r1 · GAP · R5 · rejected — ht-4ne.1 line already says "with Claude/Codex unchanged"; its tests require Claude/Codex enrollment tests unchanged and green
L7 · r1 · GAP · R10 · applied — ht-4ne.4 acceptance: null/absent witness = launch-only, never refused; ht-4ne.3 configuration 7 legacy envelope
L8 · r1 · GAP · bridge refresh · applied — ht-4ne.1 owns Hermes bridge install/refresh via setup (new bridge written, stale generation reported); ht-4ne.3 loads the setup-installed bridge
L9 · r1 · GAP · real Hermes · applied in part — ht-4ne.3 fixtures cite Hermes 0.21.5 source for each callback's kwargs; a live run stays out of the tree (fresh HERMES_HOME syncs against the real install root, violating the no-~/.hermes-writes rule); follow-on issue filed
L10 · r1 · GAP · parity · applied — merged into L2
L11 · r1 · GAP · profile without plugin · rejected — no herdr-threads hook runs in a profile without the bridge; nothing to build
L12 · r1 · UNOWNED-SEAM · Hermes bridge install/refresh via setup · applied — owned by ht-4ne.1, consumed by ht-4ne.2
L13 · r1 · UNOWNED-SEAM · TRUST-POLICY Hermes text · applied — ht-4ne.2 consumes ht-4ne.4's TRUST-POLICY text; grep covers TRUST-POLICY.md
L14 · r1 · UNOWNED-SEAM · check-in qualification for non-launched seats · applied — owned by ht-4ne.4
L15 · r2 · GAP · R4 · applied — ht-4ne.4 tests: restore hold refused, earlier-session seat reused via resolver (A4 replaces binding), fallback same guards, provenance cooperative_top_level
L16 · r2 · GAP · pane reuse · applied — ht-4ne.4 tests: exit then Hermes again / exit then Claude, same seat (A4)
L17 · r2 · GAP · resume · applied — ht-4ne.4 test and ht-4ne.3 configuration 9; ht-4ne.4 owns resume and wake limit text
L18 · r2 · GAP · daemon start · applied — ht-4ne.4: SessionStart calls ensure_running (spec D3); ht-4ne.3 configuration 8
L19 · r2 · GAP · stale bridge hint · rejected — the only runtime sink is model context; an old bridge in an unseated pane is today's normal state; docs plus setup-status carry it
L20 · r2 · UNOWNED-SEAM · bridge install/refresh · applied — ht-4ne.3 consumes ht-4ne.1 refresh
L21 · r2 · GAP · inert label · applied — ht-4ne.1 retitled; setup/status named as its only user-visible changes
L22 · r2 · GAP · profile resolution · rejected — setup profile selection is unchanged by this tree (spec D7); existing setup tests cover it
L23 · r2 · GAP · duplicate check-in · applied — ht-4ne.4 test: only the first qualified turn checks in
L24 · r2 · GAP · race · applied — ht-4ne.4 test: concurrent start and turn → one seat, one check-in with context; fallback checks in same callback
L25 · r2 · GAP · disabled plugin diagnosis · rejected in part — setup-status already reports enablement (configured_enabled); ht-4ne.2 states per-profile install/enable
L26 · r2 · GAP · R10 launched pane · applied — ht-4ne.4 null-witness test on a launch-created seat checks in with context
L27 · r2 · GAP · version skew · rejected — premise unverified in the inputs; the spec's bridge spawns the absolute executable pinned in bridge_config.json at setup, not PATH
L28 · r2 · UNOWNED-SEAM · resume and wake limits · applied — merged into L17
L29 · r2 · UNOWNED-SEAM · cli_session_start meaning · applied — ht-4ne.1 acceptance follows spec D2a combos and refusals
G1 · graph · GRAPH-EDGE · ht-4ne.4 <- ht-4ne.1 · kept — consumes the enrollment seam and envelope parse; shared files
G2 · graph · GRAPH-EDGE · ht-4ne.3 <- ht-4ne.4 · kept — smoke needs the shipped enrollment behavior
G3 · graph · GRAPH-EDGE · ht-4ne.2 <- ht-4ne.4 · kept — docs source limits from ht-4ne.4's TRUST-POLICY text
