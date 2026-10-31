requirements:
R1 → ht-kqrt.10, ht-kqrt.11, ht-kqrt.13
R2 → ht-kqrt.7
R3 → ht-kqrt.5, ht-kqrt.6, ht-kqrt.8
R4 → ht-kqrt.4, ht-kqrt.44
R5 → ht-kqrt.9
R6 → ht-kqrt.10, ht-kqrt.9
R7 → ht-kqrt.10, ht-kqrt.11
R8 → ht-kqrt.11
R9 → ht-kqrt.2, ht-kqrt.73
R10 → ht-kqrt.13
R11 → ht-kqrt.15, ht-kqrt.16, ht-kqrt.20, ht-kqrt.21
R12 → ht-kqrt.18, ht-kqrt.19
R13 → ht-kqrt.18
R14 → ht-kqrt.23, ht-kqrt.24
R15 → ht-kqrt.24
R16 → ht-kqrt.34, ht-kqrt.30, ht-kqrt.18
R17 → ht-kqrt.72 (was ht-kqrt.27)
R18 → ht-kqrt.20, ht-kqrt.21, ht-kqrt.25
R19 → ht-kqrt.28
R20 → ht-kqrt.31, ht-kqrt.32, ht-kqrt.25
R21 → ht-kqrt.26
R22 → ht-kqrt.71 (was ht-kqrt.22)
R23 → ht-kqrt.33
R24 → ht-kqrt.30, ht-kqrt.20
R25 → ht-kqrt.30
R26 → ht-kqrt.15, ht-kqrt.17, ht-kqrt.29, ht-kqrt.30
R27 → ht-kqrt.25, ht-kqrt.6 (conflicts with ht-kqrt.69 keep-alive, see finding 1)
R28 → ht-kqrt.17, ht-kqrt.32, ht-kqrt.21, ht-kqrt.23, ht-kqrt.24, ht-kqrt.25, ht-kqrt.26, ht-kqrt.28, ht-kqrt.71 (?S5), ht-kqrt.72 (?E1, D1 row)
R29 → ht-kqrt.69, ht-kqrt.70, ht-kqrt.38
R30 → ht-kqrt.69, ht-kqrt.70, ht-kqrt.37
R31 → ht-kqrt.38, ht-kqrt.35, ht-kqrt.31
R32 → ht-kqrt.69 (contradicted by its keep-alive, see finding 1)
R33 → ht-kqrt.70
R34 → ht-kqrt.38 (leak recognition), ht-kqrt.62 (evidence); contract table, AGENTS.md tmux line and T1-T5 record were owned by ht-kqrt.39, now superseded, and are not visibly owned by ht-kqrt.69
R35 → ht-kqrt.41, ht-kqrt.46, ht-kqrt.48
R36 → ht-kqrt.47, ht-kqrt.72
R37 → ht-kqrt.40, ht-kqrt.44
R38 → ht-kqrt.67; the C2/status-null/doctor parts were owned by superseded ht-kqrt.42 (see finding 3)
R39 → ht-kqrt.46, ht-kqrt.74, ht-kqrt.43, ht-kqrt.48
R40 → ht-kqrt.50, ht-kqrt.6, ht-kqrt.71
R41 → ht-kqrt.40, ht-kqrt.45, ht-kqrt.49, ht-kqrt.51, ht-kqrt.52
R42 → ht-kqrt.53, ht-kqrt.54, ht-kqrt.57
R43 → ht-kqrt.55, ht-kqrt.57
R44 → ht-kqrt.53, ht-kqrt.54, ht-kqrt.56
R45 → ht-kqrt.57, ht-kqrt.58
R46 → ht-kqrt.60
R47 → ht-kqrt.65
R-new: a named seat and an hc terminal seat can join a thread, and accept and receive invitations (the goal lists "join" first; no R names join or accept for non-Pane sites) → ht-kqrt.34, ht-kqrt.48, ht-kqrt.59 (only if their acceptance says so)
R-new: a launched hc worker retires its seat when it exits (keep:false finality); the new ht-kqrt.68 serves this and no R states it → ht-kqrt.68
R-new: the daemon is reparented through a double fork (internal daemon-detach); ht-kqrt.73 serves this and no R states it → ht-kqrt.73

- **type:** GAP
  - **subject:** R32 / R27 (idle exit) vs ht-kqrt.69
  - **evidence:** ht-kqrt.69 "Owns: ... idle-exit keep-alive ... the new `ActivityProbe::active` term counting wake-armed named bindi[ngs]". R32: "idle exit with only the named host still exits with tmux-hinted bindings present". R27: "Idle exit with only the named host exits". The amendment added a keep-alive, but the canonical requirements were not amended. Either the requirements are stale or ht-kqrt.69 breaks them.
  - **proposed fix:** Amend R32 and R27 to state the keep-alive condition (wake-armed named bindings keep the daemon up, plain tmux-hinted or unarmed bindings do not). Give ht-kqrt.69 an acceptance test for both sides.

- **type:** GAP
  - **subject:** R34 (tmux docs and evidence); ledger: C1-4
  - **evidence:** C1-4 placed "contract table, AGENTS.md rule 6/tmux line, TRUST-POLICY N2 A4, T1-T5 record" on ht-kqrt.39. ht-kqrt.39 is superseded by ht-kqrt.69. The visible "Owns:" of ht-kqrt.69 lists ComposerTraits::TMUX, the wake lane, the ActivityProbe term, pane-mode gate, capture-pane and send-keys. No docs, contract table or T1-T5 ownership is visible. ht-kqrt.39's old owns line still claims them.
  - **proposed fix:** Move those four owns items explicitly onto ht-kqrt.69 (or ht-kqrt.62 for T1-T5) and close ht-kqrt.39.

- **type:** GAP
  - **subject:** R38 (C2 on core-id rotation, status:null → Unknown, doctor status source, TRUST-POLICY D10/C2/wiped-core rows); ledger: C1-10
  - **evidence:** ht-kqrt.42 owns "C2 holds on core-id rotation; status: null → Unknown and the doctor status source; TRUST-POLICY H1 D10 closure table, C2 on core-id rotation and the wiped/cloned-core accepted limit". ht-kqrt.67 supersedes it, and its visible owns cover enumerate_targets keeping finished records with reason/keep plus "the D1[0]...". ht-kqrt.63 (H1 gate) and ht-kqrt.52 now depend on ht-kqrt.67, not ht-kqrt.42. Nothing shows ht-kqrt.67 carrying the C2, status-null, doctor or TRUST rows.
  - **proposed fix:** Copy the remaining ht-kqrt.42 owns items onto ht-kqrt.67, with acceptance for core-id rotation C2 holds, status:null → Unknown and the TRUST-POLICY rows. Close ht-kqrt.42.

- **type:** GAP
  - **subject:** C1-11 (Codex/agy turn_state_edge) after ht-kqrt.36 → ht-kqrt.70; R29/R30
  - **evidence:** ht-kqrt.36 owns "turn_state_edge implementations for Codex and agy over their existing hook events (src/harness/codex, src/harness/agy/ only)". The visible ht-kqrt.70 owns are turn_state_edge/TurnEdge (ScreenIdle), daemon per-binding turn state, Claude report-only Stop and StopFailure. Codex and agy implementations are not visible. Without them only Claude gets tmux wake, and the ht-kqrt.61 wake-e2e on Codex would fail.
  - **proposed fix:** State on ht-kqrt.70 that it owns the Codex and agy turn_state_edge implementations, with hook-driven tests.

- **type:** GAP
  - **subject:** R28 (spike S5 named half, TRUST-POLICY D12 row, D1 row, E1 Linux reader); ledger: C1-3, C2-1
  - **evidence:** C1-3 put the S5 acceptance on ht-kqrt.22. C2-1 put the D12 row on ht-kqrt.22 and the D1 row on ht-kqrt.27. Both are superseded by ht-kqrt.71 and ht-kqrt.72. The visible ht-kqrt.72 owns include the server environ reader, locator_hint and M6. The D1 TRUST row and S5/D12 on ht-kqrt.71 are not visible in the tree.
  - **proposed fix:** Confirm or add S5, the D12 TRUST row and the nameless-hold accepted limit as owns on ht-kqrt.71, and the D1 TRUST row and same-name limit on ht-kqrt.72.

- **type:** GAP
  - **subject:** R22 text vs ht-kqrt.71 nameless hold
  - **evidence:** R22: "stays for the operator when a live seat holds that name". ht-kqrt.71: "on collision with a non-retired seat of [that name] ... nameless hold". A non-retired but unresolved seat is a collision under ht-kqrt.71 but not under R22, so the requirement and the task diverge.
  - **proposed fix:** Amend R22 to the nameless-hold rule (collision with any non-retired seat, then a nameless hold) and test it on ht-kqrt.71.

- **type:** GAP
  - **subject:** R-new join/accept for named and hc seats
  - **evidence:** Goal: "full messaging: join, send, inbox, ACK, turn-hook and channel delivery". R16 and R35 list send, inbox, ACK and hook delivery but never join, invite or accept. The text accept prints the pending backlog and depends on the caller site, so it is not trivially covered.
  - **proposed fix:** Add join and accept-invitation acceptance to ht-kqrt.34 (named) and ht-kqrt.48 or ht-kqrt.59 (hc).

- **type:** ORPHAN
  - **subject:** ht-kqrt.22, ht-kqrt.27, ht-kqrt.36, ht-kqrt.39, ht-kqrt.42 (superseded)
  - **evidence:** The changes file says each is superseded by ht-kqrt.71, .72, .70, .69 and .67. C3-1 removed their dependent edges, and the ht-kqrt.66 sweep deps omit all five. They remain in the tree with live owns lines that duplicate their replacements, for example ComposerTraits::TMUX on both ht-kqrt.39 and .69, turn_state_edge on both .36 and .70, and the server environ reader on both .27 and .72. That is double ownership, and a ready-queue executor could still pick them up.
  - **proposed fix:** Close each one as superseded (or drop it from the tree) after moving any owns item its replacement lacks (findings 2-5).

- **type:** UNOWNED-SEAM
  - **subject:** labels N2-wake / N2-turn (ht-kqrt.69/.70 → ht-kqrt.59, ht-kqrt.61)
  - **evidence:** ht-kqrt.59 "consumes: ... tmux wake (N2-wake)". ht-kqrt.61 "consumes: tmux composer wake (N2-wake); per-binding turn state and turn_state_edge (N2-turn)". Those labels were defined by ht-kqrt.39 and ht-kqrt.36. The replacements ht-kqrt.67-.74 have no structured owns:/consumes: lines (ownership is only inside the description text), so these consumed labels resolve to superseded owners.
  - **proposed fix:** Give ht-kqrt.67-.74 structured owns: and consumes: lines, and re-point the N2-wake and N2-turn consumers to ht-kqrt.69 and ht-kqrt.70.

- **type:** UNOWNED-SEAM
  - **subject:** ht-kqrt.68 ↔ ht-kqrt.57: the daemon LaunchGate handler and the gate-shim handoff path
  - **evidence:** ht-kqrt.68: "a gate-shim-launched worker (`keep: false`, harness exec'd as terminal root) retires its seat through D10". The launch that creates that worker exists end to end only once ht-kqrt.57's "daemon LaunchGate handler" lands. ht-kqrt.68 deps are .54, .55 and .67, not .57.
  - **proposed fix:** Add a ht-kqrt.68 → ht-kqrt.57 dependency and name the LaunchGate handler in ht-kqrt.68's consumes.

- **type:** UNOWNED-SEAM
  - **subject:** host::foreign_terminal_process_names() moving from F (ht-kqrt.74) into host::hc (H1)
  - **evidence:** ht-kqrt.74: "a const list in src/host/mod.rs with `herdr-core` and `hc`, moving into `host::hc` in H1". No H1 task's owns line names the move. ht-kqrt.41 owns host::hc::ENV_VARS only, and ht-kqrt.46 (full LocateCaller) has no direct dependency on ht-kqrt.74.
  - **proposed fix:** Put the move on ht-kqrt.41 or ht-kqrt.46 (owns line plus a direct dependency on ht-kqrt.74), and have ht-kqrt.46 consume the terminal-root wall and the start-stamped cache key.

- **type:** UNOWNED-SEAM
  - **subject:** idle-exit rule shared by ht-kqrt.6 (dialled-only idle exit filter) and ht-kqrt.69 (ActivityProbe::active keep-alive term)
  - **evidence:** ht-kqrt.6 owns "dialled-only consumer filters per 07 §5.3 (idle exit, ...)". ht-kqrt.69 adds a new idle-exit keep-alive term. Neither task names the combined rule, and ht-kqrt.69 has no direct dependency on ht-kqrt.6 (only transitively, through .38 → .15 → .13).
  - **proposed fix:** Make ht-kqrt.69 own the combined idle-exit predicate (consumes ht-kqrt.6) and test it together with R27/R32 (finding 1).

NEEDS-SPEC: ht-kqrt (N2) — the idle-exit keep-alive for wake-armed named bindings in ht-kqrt.69 contradicts R27/R32; the spec must say which wins.
