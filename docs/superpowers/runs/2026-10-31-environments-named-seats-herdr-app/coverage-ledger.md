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
C2-1 · r2 · GAP · R28 TRUST-POLICY/AGENTS.md N rows · applied — same-commit TRUST-POLICY owns lines + acceptance on .21 (C1 named continuity, cooperative_named_seat, cross-kind refusal), .26 (cooperative_self_retire, Operator replication), .23 (D19, M7), .24 (readonly no-ACK), .27 (D1), .22 (D12), .28 (M9)
C2-2 · r2 · GAP · Claude mod channel on a Named site · applied — ht-kqrt.17 owns register.js spawn env forwarding of seat name/readonly and no stop on no_pane under a name; node test with no HERDR_PANE_ID
C2-3 · r2 · GAP · environment-agnostic core enforced mechanically · applied — new leaf ht-kqrt.65 (host-neutrality source scan, pattern-based with allowlist; deps .3, .10); dep of gates .34, .62, .63, .64; full scripts/check-host-neutral (G6) stays follow-on per spec
C2-4 · r2 · ORPHAN · H3 leaves ht-kqrt.53-.58, .64 · kept — the spec puts H3 in scope (constraint 3, Phase H3 and its acceptance); the coverage tool reads the goal from the spec's ## Goal, which is not edited
C2-5 · r2 · UNOWNED-SEAM · daemon launch-gate authorization handler (.53 -> .54/.57) · applied — ht-kqrt.57 owns the daemon LaunchGate handler (match (core,tN) vs CreatedTarget/HostWitness and terminal.get, stage bootstrap Send before reply, bounded-timeout refusal) with a fake_hc test
C2-6 · r2 · GAP · Terminal-site instance discovery beside Herdr · applied — ht-kqrt.46 acceptance (Herdr command group's store, no TUI --host-endpoint for hc, doctor flags two stores); dep on ht-kqrt.18
C2-7 · r2 · GAP · Herdr desktop-app pane gets full messaging · applied — ht-kqrt.59 acceptance with a fake_hc herdr-app fixture (seat, send, inbox, ACK, hook delivery, <session>:tN); dep on ht-kqrt.43
C2-8 · r2 · GAP · R39 M20 self-retire refusal from a Terminal site · applied — ht-kqrt.48 owns and tests the M20 refusal
C2-9 · r2 · GAP · R10 F behavioural gate ht-kqrt.13 blocks nothing · applied — ht-kqrt.13 is now a dep of .15, .40, .34, .63
C2-10 · r2 · GAP · whole-tree gate with every phase merged · applied — ht-kqrt.64 is the final gate (retitled; runs on the fully merged run branch); deps on .62, .63, .59 (and .65)
C2-11 · r2 · GAP · .59 configuration smoke behind no gate · applied — covered by C2-10: ht-kqrt.59 is a dep of ht-kqrt.64
C2-12 · r2 · GAP · R18 unresolved named row vs liveness of other named bindings · applied — ht-kqrt.25 owns it with a store/reconcile test
C2-13 · r2 · UNOWNED-SEAM · consumer filters F4-audit (.6 -> .50, .34) · applied — ht-kqrt.6 owns dialled-only consumer filters per 07 §5.3
C2-14 · r2 · UNOWNED-SEAM · hc labels H1c-labels (.43 -> .60, .52) · applied — ht-kqrt.43 owns seat_labels/live_topology output, herdr-app doc-name parsing/cache, ambiguous bare tN refusal
C2-15 · r2 · UNOWNED-SEAM · HcHost::server_process (.47 <-> .27) · applied — ht-kqrt.47 owns HcHost::server_process and LocalEndpointWitness peer-pid recording
C2-16 · r2 · GAP · Hermes tmux idle wake · rejected — spec scopes turn_state_edge to Codex/agy plus Claude report-only Stop; default None means no tmux wake for Hermes by design
C2-17 · r2 · NEEDS-SPEC · ht-kqrt (N2) Hermes tmux wake · rejected — spec settles it: default turn_state_edge None, no tmux wake for Hermes
C2-18 · r2 · NEEDS-SPEC · ht-kqrt (root) goal lacks H3 outcome · rejected — duplicate of C2-4; H3 is in scope per spec
C2-19 · r2 · GAP · cross-node hc case in the .59 smoke · rejected — owned and tested by ht-kqrt.60 (named cross-node by .33); a second end-to-end copy fills no gap
C3-1 · amend-design-1 · unstated · 16 stale edges to superseded beads · applied — removed (reason lines already re-pointed to replacements)
C3-2 · amend-design-1 · citation · ht-kqrt.59/.50 cited superseded ht-kqrt.39/.22 · applied — re-pointed to ht-kqrt.69/.71
C3-3 · amend-design-1 · GAP · R27/R32 idle exit vs ht-kqrt.69 keep-alive · applied — R27/R32 rewritten to the spec keep-alive; ht-kqrt.69 acceptance covers both directions (wake-armed stays up, dead/unhinted idle-exits)
C3-4 · amend-design-1 · GAP · R34 tmux docs and evidence after ht-kqrt.39 superseded · applied — ht-kqrt.69 owns: contract table, AGENTS.md rule 6 + tmux line, TRUST-POLICY N2 A4 row incl. keep-alive and pane-mode limits, T1/T3/T5 (T2 .70, T4 .38, all recorded by .62); .39 already closed
C3-5 · amend-design-1 · GAP · R38 C2 rotation, status:null, doctor source, TRUST D10/C2/wiped-core rows after ht-kqrt.42 superseded · applied — ht-kqrt.67 owns: line + fake_hc acceptance (C2 on rotation, status:null Unknown, keep/reason rows, TRUST rows rewritten); .42 already closed
C3-6 · amend-design-1 · GAP · Codex/agy turn_state_edge after ht-kqrt.36 superseded · applied — ht-kqrt.70 owns Codex/agy implementations (src/harness/codex*, agy/ only) with hook-driven tests
C3-7 · amend-design-1 · GAP · R28/R17 S5 named half, D12, D1, E1 after ht-kqrt.22/.27 superseded · applied — ht-kqrt.71 owns D12 row, S5 named half, nameless-hold limit; ht-kqrt.72 owns D1 row (same-name limit, revised M6) and E1 Linux/macOS reader acceptance
C3-8 · amend-design-1 · GAP · R22 vs ht-kqrt.71 nameless hold · applied — R22 rewritten to the spec rule (non-retired collision, name NULL hold, inspect/doctor show it); ht-kqrt.71 acceptance quotes it
C3-9 · amend-design-1 · GAP · R38 exit semantics vs ht-kqrt.68 keep:false · applied — R38 split on keep; R48 (launched keep:false worker retires) added, served by ht-kqrt.68
C3-10 · amend-design-1 · GAP · join/accept-invitation for named and hc seats · applied — join + accept (text accept backlog print from the site) acceptance on ht-kqrt.34 (Named) and ht-kqrt.48 (Terminal)
C3-11 · amend-design-1 · ORPHAN · superseded ht-kqrt.22/.27/.36/.39/.42 · applied — owns carried to .71/.72/.70/.69/.67 (C3-4..C3-7); all five already closed; no edges from open beads to them remain (.39 -> .36 is from a closed bead, kept)
C3-12 · amend-design-1 · UNOWNED-SEAM · labels N4-inherit/N3-restore/H1c-closure/N2-wake/N2-turn on ht-kqrt.67-.74 · applied — owns:/consumes:/Key lines on .67-.74 (labels .72/.71/.67/.69/.70, new H3-launch-final .68, F1-detach .73, F8-wall .74); consumer lines already name labels, none named old ids
C3-13 · amend-design-1 · UNOWNED-SEAM · ht-kqrt.68 <-> .57 LaunchGate handler · applied — dep ht-kqrt.68 -> .57; .68 consumes the LaunchGate handler (H3d2)
C3-14 · amend-design-1 · UNOWNED-SEAM · terminal.run keep parameter · applied — ht-kqrt.55 owns schema-checked keep: false with fake_hc assertion; R43 amended; .68 consumes it
C3-15 · amend-design-1 · UNOWNED-SEAM · idle-exit predicate .6/.69 · applied — dep ht-kqrt.69 -> .6; .69 owns the combined predicate (N2-idle-exit) with a dialled-hosts x wake-armed-bindings matrix; .50 runs its idle-exit cases against it (dep .50 -> .69)
C3-16 · amend-design-1 · UNOWNED-SEAM · foreign_terminal_process_names() move into host::hc · applied — ht-kqrt.46 owns the move and consumes F8-wall from .74 (dep .46 -> .74); .74 owns the F const list
C3-17 · amend-design-1 · UNOWNED-SEAM · ScreenIdle emission .69/.70 · applied — .70 owns screen_turn_running and the ScreenIdle turn-state entry; .69 owns the two-read (60 s busy, 5 s apart) emission and the only-path test; .70's reverse consume of the composer read removed
C3-18 · amend-design-1 · UNOWNED-SEAM · spawn_detached .2/.73 · applied — ht-kqrt.73 owns the R9 assertion on the reparented daemon's environ, consuming F1-scrub from .2
C3-19 · amend-design-1 · NEEDS-SPEC · N2 keep-alive vs R27/R32 · rejected — spec settles it; handled as C3-3
C3-20 · amend-design-1 · NEEDS-SPEC · R38 vs .68 and R22 vs .71 · rejected — spec settles both; handled as C3-8 and C3-9
G-1 · graph · GRAPH-EDGE · ht-kqrt.15 <- ht-kqrt.13 · applied — F gate is evidence-only; ordering kept via .34/.63/.66
G-2 · graph · GRAPH-EDGE · ht-kqrt.40 <- ht-kqrt.13 · applied — same; .63 <- .13 kept
G-3 · graph · GRAPH-EDGE · 20 critical edges · kept — carry real artifacts
G-4 · graph · GRAPH-EDGE · seam-contract .35/.38 and .32/.35 · parked — proposals are never applied unattended
