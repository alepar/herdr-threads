C1-1 · r1 · GAP · R25 named-caller handoff recovery hint · applied — ht-kqrt.30 owns the Named-site env_assignments recovery hint, acceptance + dep on ht-kqrt.16
C1-2 · r1 · GAP · R26 M11 handoff into a named seat · applied — ht-kqrt.30 owns the M11 refusal (NamedHost port answer) with golden acceptance
C1-3 · r1 · GAP · R28 spikes S2, S4, named half of S5 · applied — acceptance lines already present on ht-kqrt.25 (S2), ht-kqrt.26 (S4), ht-kqrt.22 (S5); Spec refs added to .25/.26
C1-4 · r1 · GAP · R34 tmux docs and evidence · applied — ht-kqrt.39 owns contract table, AGENTS.md rule 6/tmux line, TRUST-POLICY N2 A4, T1-T5 record; ht-kqrt.38 owns -L ht-test-* leak recognition
C1-5 · r1 · GAP · R45 L1-L4 · applied — ht-kqrt.57 acceptance answers L1, L2/L3, L4 into the run report
C1-6 · r1 · GAP · cross-node addressing of hc terminal seats · applied — new leaf ht-kqrt.60 (deps .43, .33, .48)
C1-7 · r1 · GAP · Claude mod channel from Terminal sites · applied — ht-kqrt.48 owns mod watch/channel on Terminal sites, dep on ht-kqrt.17
C1-8 · r1 · GAP · per-phase integration sweep after F · applied — ht-kqrt.34 extended as N gate (+13 N deps); new gates ht-kqrt.62 (N2), ht-kqrt.63 (H1), ht-kqrt.64 (H3); no root sweep
C1-9 · r1 · GAP · real-agent tmux wake e2e · applied — new opt-in leaf ht-kqrt.61 (tmux-named wake-e2e case; deps .39, .36)
C1-10 · r1 · GAP · TRUST-POLICY and AGENTS.md H1/H3 rows · applied — owns lines on .41 (vocab, C1), .42 (D10, C2, wiped core), .46 (A1+limits), .48 (A4, AGENTS hc line), .54 (managed_launch), .55 (ht.bootstrap), .57 (C4); isolated-core line stays on .40
C1-11 · r1 · GAP · tmux turn_state_edge for Codex and agy · applied — ht-kqrt.36 owns Codex/agy turn_state_edge with hook-driven tests
C1-12 · r1 · GAP · cross-environment messaging including hc · applied — ht-kqrt.59 acceptance covers hc<->named and hc<->pane in one daemon
C1-13 · r1 · ORPHAN · H3 leaves ht-kqrt.53..58 · kept — they serve goal outcome "full herdr-threads functionality in a herdr-core terminal; handoff is how coordinators create workers (proposal D7)"; epic goal amended with the H3 handoff/launch outcome
C1-14 · r1 · UNOWNED-SEAM · seat.created.display_name semantics · applied — ht-kqrt.33 owns; ht-kqrt.22 consumes via display_name fixture fact
C1-15 · r1 · UNOWNED-SEAM · hc label vocabulary and <session>:tN · applied — ht-kqrt.41 owns; .43, .55, .57 consume
C1-16 · r1 · UNOWNED-SEAM · default standalone state dir · applied — ht-kqrt.18 owns the path function; ht-kqrt.19 consumes, dep on .18, equality acceptance
C1-17 · r1 · UNOWNED-SEAM · launch-gate shim to daemon (core,tN) authorization · applied — ht-kqrt.53 owns the command/reply and staging rule; ht-kqrt.54 (new dep on .53) and .57 consume
C1-18 · r1 · UNOWNED-SEAM · readonly field on the resolve command · applied — ht-kqrt.24 owns the readonly resolve-command field
C1-19 · r1 · UNOWNED-SEAM · tmux hint field vs locator_hint/M9 · applied — ht-kqrt.38 owns (dep on .28, no-locator_hint/no-M9 test); .39 consumes; .28 notes the rule
C1-20 · r1 · UNOWNED-SEAM · HcDiscovery API and core state paths · applied — owns moved from ht-kqrt.41 to ht-kqrt.45
C1-21 · r1 · UNOWNED-SEAM · site-capability gate for seat.terminal_sites_v1 · applied — ht-kqrt.48 consumes the F9 gate, direct dep on ht-kqrt.12
C1-22 · r1 · UNOWNED-SEAM · HcHost terminal.run request/reply · applied — ht-kqrt.55 owns terminal.run client method, params and fake_hc terminal.run
C1-23 · r1 · GAP · hc idle wake as far as host allows · rejected — H2 hc composer wake is out of scope (run constraint 3); no_wake is intended
C1-24 · r1 · GAP · user-facing README/install docs · rejected — not asked by goal or spec; doc surfaces are skill, AGENTS.md, TRUST-POLICY
C1-25 · r1 · ORPHAN · ht-kqrt.52 hc-smoke · rejected — backs R41 (hc-smoke status recorded), spec defines the follow-on fallback
C1-26 · r1 · NEEDS-SPEC · ht-kqrt.40/.48 hc wake decision · rejected — spec answers it: hc composer wake is H2, out of scope
