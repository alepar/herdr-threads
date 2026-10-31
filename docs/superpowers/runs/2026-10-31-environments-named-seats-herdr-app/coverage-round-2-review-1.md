requirements:
R1 → ht-kqrt.10, ht-kqrt.11, ht-kqrt.13 (H3 regression: ht-kqrt.56, ht-kqrt.64)
R2 → ht-kqrt.7
R3 → ht-kqrt.5, ht-kqrt.6, ht-kqrt.8
R4 → ht-kqrt.4, ht-kqrt.3
R5 → ht-kqrt.9
R6 → ht-kqrt.10, ht-kqrt.9
R7 → ht-kqrt.10, ht-kqrt.11
R8 → ht-kqrt.11
R9 → ht-kqrt.2
R10 → ht-kqrt.13
R11 → ht-kqrt.16, ht-kqrt.20, ht-kqrt.18, ht-kqrt.34
R12 → ht-kqrt.19, ht-kqrt.18
R13 → ht-kqrt.18
R14 → ht-kqrt.23, ht-kqrt.24
R15 → ht-kqrt.24
R16 → ht-kqrt.34, ht-kqrt.30, ht-kqrt.18
R17 → ht-kqrt.27
R18 → ht-kqrt.21, ht-kqrt.25
R19 → ht-kqrt.28
R20 → ht-kqrt.31, ht-kqrt.32, ht-kqrt.25, ht-kqrt.8
R21 → ht-kqrt.26
R22 → ht-kqrt.22
R23 → ht-kqrt.33
R24 → ht-kqrt.30
R25 → ht-kqrt.30
R26 → ht-kqrt.17 (M14/M15/M16), ht-kqrt.30 (M11), ht-kqrt.29 (M10), ht-kqrt.15 (corpus)
R27 → ht-kqrt.25, ht-kqrt.6
R28 → ht-kqrt.17 (K1, Hermes bridge, AGENTS line, S3), ht-kqrt.32 (K2/K3, tripwire, S6, A4), ht-kqrt.21 (S1), ht-kqrt.25 (S2), ht-kqrt.26 (S4), ht-kqrt.22 (S5), ht-kqrt.27 (E1); TRUST-POLICY N rows other than A4 → (unmapped, see GAP 1)
R29 → ht-kqrt.39, ht-kqrt.38
R30 → ht-kqrt.39, ht-kqrt.37, ht-kqrt.36
R31 → ht-kqrt.38, ht-kqrt.35
R32 → ht-kqrt.39
R33 → ht-kqrt.36
R34 → ht-kqrt.39, ht-kqrt.38, ht-kqrt.62
R35 → ht-kqrt.48, ht-kqrt.46, ht-kqrt.41
R36 → ht-kqrt.47
R37 → ht-kqrt.44, ht-kqrt.4
R38 → ht-kqrt.42, ht-kqrt.41
R39 → ht-kqrt.46, ht-kqrt.43, ht-kqrt.48 (M20 from Terminal site, implied only)
R40 → ht-kqrt.50
R41 → ht-kqrt.45, ht-kqrt.49, ht-kqrt.40, ht-kqrt.51, ht-kqrt.52, ht-kqrt.63
R42 → ht-kqrt.53, ht-kqrt.54, ht-kqrt.57
R43 → ht-kqrt.55, ht-kqrt.57
R44 → ht-kqrt.54, ht-kqrt.56, ht-kqrt.53
R45 → ht-kqrt.58, ht-kqrt.57, ht-kqrt.64
R-new: hc terminal seats are addressable and displayed across nodes (`<session>:tN@node`) → ht-kqrt.60
R-new: core neutrality is enforced: routing, delivery, receipts and binding contain no branch on HostKind::Named/Hc or on host-specific env, checked by a source scan or check script (the goal says the core "never learn[s] which environment a seat lives in") → (unmapped)
R-new: Claude mod channel delivery and channel wake work from a Named site (plain/tmux process), as well as from a Terminal site → ht-kqrt.48 (Terminal only); Named → (unmapped)
R-new: an agent in a Herdr desktop-app pane, as distinct from a bare `hc run` terminal, gets a seat with full messaging → ht-kqrt.43 (doc names only); end to end → (unmapped)
R-new: tmux idle wake for Codex/agy named seats (turn_state_edge), plus a real-agent proof → ht-kqrt.36, ht-kqrt.61
R-new: Terminal-site instance discovery when Herdr is also installed (hooks baked with a TUI --host-endpoint) resolves to one store, the hc analogue of R13 → (unmapped)
R-new: handoff and launch into hc terminals and app spaces (the H3 outcome said to be added to the epic goal under C1-13) → ht-kqrt.53..58, ht-kqrt.64

- **type:** GAP
- **subject:** R28 "TRUST-POLICY/AGENTS.md carry the N rows" (ledger: C1-10, which covered H1/H3 only)
- **evidence:** C1-10 added TRUST-POLICY owns lines on .41/.42/.46/.48/.54/.55/.57, all H1/H3. In N, only ht-kqrt.32 names TRUST-POLICY (A4, no-wake). The N leaves that add new continuity or attribution rules name no TRUST-POLICY row on their owns lines: ht-kqrt.21 (owns "named continuity C1 path; cross-kind rebind refusal; Closed guard"; R18 names provenance `cooperative_named_seat`), ht-kqrt.26 (R21 names `cooperative_self_retire`, Operator replication), ht-kqrt.23 (live-holder guard D19, M7), ht-kqrt.24 (readonly bindings never ACK), ht-kqrt.27 (inherited-name refusal D1), ht-kqrt.28 (second-locator M9). AGENTS.md: "a new way to attribute an action needs a provenance value defined in the policy" and "A change that weakens an invariant ... updates TRUST-POLICY.md in the same commit."
- **proposed fix:** Add owns lines in the "(same commit)" style C1-10 used: .21 gets the C1 named-continuity row and the `cooperative_named_seat` provenance; .26 gets `cooperative_self_retire`; .23 gets the D19 live-holder row; .24 gets the readonly/no-ACK receipt row; .27 gets the D1 inherited-name row; .28 gets the M9 row.

- **type:** GAP
- **subject:** R-new: Claude mod channel on a Named site (R20 "live-mod Claude reports no block" for named seats; goal "channel delivery" for the plain/tmux agent) (ledger: C1-7, Terminal only)
- **evidence:** ht-kqrt.48 owns "mod watch/channel on a Terminal site (register.js spawn env carries HC_TERMINAL_ID/HC_TERMINAL_SOCKET; never stops on no_pane)". That implies register.js today stops on `no_pane` and needs a change before it works off-pane. ht-kqrt.17 owns only "watch on the Named site" (CLI watch, S3), with no register.js or no_pane change. No task makes the mod keep running under `HERDR_THREADS_SEAT_NAME` with no pane, so the channel delivery and channel wake that R20 relies on have no owner for named seats.
- **proposed fix:** Extend ht-kqrt.17's owns line with "mod watch/channel on a Named site (register.js carries HERDR_THREADS_SEAT_NAME/READONLY, never stops on no_pane)" and an acceptance test with the live mod. Alternatively move the no_pane change into one shared leaf that .17 and .48 both consume.

- **type:** GAP
- **subject:** R-new: environment-agnostic core is enforced
- **evidence:** The goal says "routing, delivery, receipts and binding logic never learn which environment a seat lives in." R6 limits caller-location env literals to caller_location.rs, which covers the CLI only. R40 tests dialled-only consumers with two hosts. `scripts/check-harness-neutral` covers harness names, not host kinds. No task owns a check that src/store, src/service, src/delivery and the scheduler never match on HostKind::Named/Hc or read HC_*/TMUX. Premortem: every leaf passes its own tests while a store leaf (.21/.25/.31) or wake_status (.8) branches on HostKind::Named, and nothing catches it.
- **proposed fix:** Add a source-scan test (or extend check-harness-neutral with a host-kind allowlist) to ht-kqrt.13, or as a small leaf gated by .34 and .63, asserting that HostKind variants and host env names appear only under src/host/<kind>/, in registration and in caller_location.rs.

- **type:** ORPHAN
- **subject:** ht-kqrt.53, .54, .55, .56, .57, .58, .64 (ledger: C1-13)
- **evidence:** The changes file says "Amended: ht-kqrt epic goal (C1-13 H3 outcome)". The Goals section in coverage-round-2-inputs.md still reads only seats, messaging, cross-env/cross-node addressing, the agnostic core and idle wake. It has no handoff or launch outcome and no "coordinators create workers (D7)". Against the goal text as provided, the H3 leaves serve no stated outcome, so the C1-13 fix is not visible in the inputs.
- **proposed fix:** Confirm the amendment is in the epic's goal field and that the coverage-inputs generator reads it. If it is missing, add the sentence: "Handoff and launch create workers in each environment, including hc terminals and app spaces (D7)."

- **type:** UNOWNED-SEAM
- **subject:** daemon-side launch-gate authorization handler between ht-kqrt.53 and ht-kqrt.54/ht-kqrt.57 (ledger: C1-17)
- **evidence:** ht-kqrt.53 is a "Seam contract:" leaf that owns "launch-gate authorization command and reply (bootstrap <compound>:<attempt>, (core, tN) witness, bounded timeout, refusal), its local capability, and the rule that the bootstrap Send is staged before the reply". ht-kqrt.54 owns the client-side shim. ht-kqrt.57 "consumes ... launch-gate authorization command and reply, bootstrap Send staged before the reply (H3a)". No owns line names the daemon handler that matches the shim's HC_TERMINAL_ID against the CreatedTarget/HostWitness from terminal.run (.55), enforces the timeout and stages the Send. A seam-contract leaf usually defines types, not the service implementation, so the behaviour R42 tests ("waiting for the daemon's (core,tN) authorization before exec") may fall between .53 and .57.
- **proposed fix:** State explicitly on ht-kqrt.57 (or .54) "owns: daemon LaunchGate handler: witness match, timeout, refusal, Send staged before reply", or confirm that .53's acceptance covers the handler implementation.

- **type:** GAP
- **subject:** R-new: Terminal-site instance discovery beside Herdr (hc analogue of R13)
- **evidence:** ht-kqrt.18 implements SA1 discovery for "non-Pane sites" but lands in N, before CallerLocator::Terminal exists (.46). ht-kqrt.49 covers only "an hc-only machine (... no `herdr` on PATH)". On a machine with both Herdr and hc, hooks are baked with the TUI `--host-endpoint` (R12, "with herdr present, install output and hooks are byte-identical"). No task owns how an hc terminal caller (including a desktop-app pane) picks its state dir and ignores the baked TUI endpoint, and no requirement plays R13's role (one store, doctor flags two stores) for hc. ht-kqrt.46 consumes .18 nowhere: its deps are .11, .16, .41, .44.
- **proposed fix:** Add to ht-kqrt.46 (or .49) acceptance: "Terminal site on a Herdr-installed machine uses the Herdr command group's store; the baked TUI --host-endpoint is not used for hc; doctor flags two stores." Add a dep on .18.

- **type:** GAP
- **subject:** R-new: Herdr desktop-app pane gets a seat with full messaging
- **evidence:** The goal names "a herdr-core terminal (`hc run` or a Herdr desktop-app pane)". Only ht-kqrt.43 mentions the app ("names from doc `herdr-app`"), and only for labels. ht-kqrt.59 lists "hc terminal sites" without saying whether one is an app-created terminal (with herdr.space/herdr.tab labels and the app doc). No acceptance runs messaging from a pane shaped like a desktop-app pane, so a difference in env or labels between app panes and `hc run` terminals would go unseen.
- **proposed fix:** Add a fake_hc desktop-app-pane fixture (app doc plus herdr.space/tab labels) to ht-kqrt.48 or ht-kqrt.59 acceptance: seat, send, inbox, ACK, hook delivery and `<session>:tN` label.

- **type:** GAP
- **subject:** idle wake "as far as each host allows" for Hermes in tmux (ledger: C1-11, Codex/agy only)
- **evidence:** ht-kqrt.36: "default `None` = no tmux wake for that harness", and it owns turn_state_edge implementations "for Codex and agy ... only", with Claude as report-only Stop. Hermes has a turn-start hook (`pre_llm_call`, per AGENTS.md), but no task gives it a turn_state_edge, so a Hermes named seat in tmux never wakes even though the host allows it. This is either a gap or an intended limit; neither is recorded.
- **proposed fix:** Either add a Hermes turn_state_edge to .36, or record "Hermes: no tmux wake" as an accepted limit in .39's TRUST-POLICY N2 A4 row and the contract table.

- **type:** GAP
- **subject:** R39 "self-retire from a Terminal site is refused with M20"
- **evidence:** ht-kqrt.26 owns `seat retire --self`, and R21 refuses it "for pane seats (M20)". ht-kqrt.46 owns M4/M17 only. ht-kqrt.48 consumes self-retire but no owns line or acceptance names the Terminal-site M20 refusal.
- **proposed fix:** Add "M20 refusal from a Terminal site" to ht-kqrt.46 (or .48) acceptance.

- **type:** GAP
- **subject:** cross-phase configuration smoke ht-kqrt.59 is behind no gate (ledger: C1-8, "no root sweep")
- **evidence:** ht-kqrt.59 deps .16, .34, .39, .48 and is not a dep of .62, .63 or .64. ht-kqrt.60 (cross-node hc) is in .63, but no gate depends on .59, which carries the C1-12 hc<->named and hc<->pane acceptance. Premortem: every phase gate is green and the mixed-environment smoke has never run on the final tree.
- **proposed fix:** Make ht-kqrt.64 (the last gate) depend on ht-kqrt.59, or add .59 to the H1 gate .63, since it needs only N2 and H1 leaves.

- **type:** UNOWNED-SEAM
- **subject:** HcHost::server_process between ht-kqrt.47 and ht-kqrt.27
- **evidence:** ht-kqrt.47 says "`HcHost::server_process` = the core socket's peer pid from the `LocalEndpointWitness` the client records" but has no owns line. ht-kqrt.27 consumes "HostPort server_process answer (F2)". ht-kqrt.40 owns the rpc client, but its owns line does not name recording the LocalEndpointWitness or the peer pid. The peer-pid capture in the hc client that .47 depends on has no owner.
- **proposed fix:** Add to ht-kqrt.47 "owns: HcHost::server_process", and to ht-kqrt.40 "owns: LocalEndpointWitness peer-pid recording in the hc client" (or move that into .47).

Previous round's fixes, checked:
- C1-1/C1-2 (.30): closed.
- C1-3: closed.
- C1-4: closed. T1-T5 are recorded in both .39 and .62; a duplicate, harmless.
- C1-5: closed.
- C1-6 (.60): closed. Added R-new above.
- C1-7: closes Terminal only. Named gap re-reported above.
- C1-8: phase gates are well wired transitively. .59 is outside every gate (finding above).
- C1-9 (.61): consumes .38's helper only through .39. Fine transitively.
- C1-10: H1/H3 only. N rows re-reported above.
- C1-11: Hermes left undecided (finding above).
- C1-12: closed, but ungated (see the .59 finding).
- C1-13: amendment not visible in the Goals input (ORPHAN above).
- C1-14, C1-15, C1-16, C1-18, C1-19, C1-20, C1-21, C1-22: closed.
- C1-17: the handler implementation is still ambiguous (seam above).

NEEDS-SPEC: ht-kqrt (N2) — whether Hermes gets tmux idle wake or an accepted no-wake limit is not settled by the inputs.
