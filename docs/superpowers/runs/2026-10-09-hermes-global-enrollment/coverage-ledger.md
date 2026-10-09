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
