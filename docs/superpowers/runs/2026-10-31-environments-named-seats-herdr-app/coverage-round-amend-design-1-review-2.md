requirements:
R1 → ht-kqrt.10, ht-kqrt.11, ht-kqrt.13, ht-kqrt.56, ht-kqrt.66
R2 → ht-kqrt.7
R3 → ht-kqrt.3, ht-kqrt.5, ht-kqrt.6
R4 → ht-kqrt.4, ht-kqrt.44
R5 → ht-kqrt.9
R6 → ht-kqrt.10, ht-kqrt.9, ht-kqrt.56
R7 → ht-kqrt.10, ht-kqrt.11, ht-kqrt.74
R8 → ht-kqrt.11
R9 → ht-kqrt.2, ht-kqrt.73
R10 → ht-kqrt.13
R11 → ht-kqrt.15, ht-kqrt.16, ht-kqrt.20, ht-kqrt.21
R12 → ht-kqrt.18, ht-kqrt.19
R13 → ht-kqrt.18
R14 → ht-kqrt.23, ht-kqrt.24
R15 → ht-kqrt.24
R16 → ht-kqrt.18, ht-kqrt.30, ht-kqrt.34
R17 → ht-kqrt.72
R18 → ht-kqrt.21, ht-kqrt.25
R19 → ht-kqrt.28
R20 → ht-kqrt.8, ht-kqrt.25, ht-kqrt.31, ht-kqrt.32
R21 → ht-kqrt.26
R22 → ht-kqrt.71 (text does not match .71's "nameless hold", see GAP 4)
R23 → ht-kqrt.33
R24 → ht-kqrt.20, ht-kqrt.30
R25 → ht-kqrt.30
R26 → ht-kqrt.15, ht-kqrt.17, ht-kqrt.29, ht-kqrt.30
R27 → ht-kqrt.6, ht-kqrt.25 (put at risk by ht-kqrt.69's keep-alive, see GAP 3)
R28 → ht-kqrt.17, ht-kqrt.21, ht-kqrt.23, ht-kqrt.24, ht-kqrt.25, ht-kqrt.26, ht-kqrt.28, ht-kqrt.32, ht-kqrt.72 (D12 row and named half of S5 were owned by superseded ht-kqrt.22, see GAP 5)
R29 → ht-kqrt.38, ht-kqrt.69, ht-kqrt.70
R30 → ht-kqrt.37, ht-kqrt.69, ht-kqrt.70
R31 → ht-kqrt.31, ht-kqrt.35, ht-kqrt.38
R32 → (unmapped as stated; ht-kqrt.69 delivers the lane registration but adds an idle-exit keep-alive that contradicts the idle-exit half)
R33 → ht-kqrt.70
R34 → ht-kqrt.38 (leak recognition), ht-kqrt.62 (T1-T5 evidence); contract table, AGENTS.md tmux line and TRUST-POLICY N2 A4 → (unmapped: owned only by superseded ht-kqrt.39)
R35 → ht-kqrt.41, ht-kqrt.46, ht-kqrt.48
R36 → ht-kqrt.47, ht-kqrt.72
R37 → ht-kqrt.4, ht-kqrt.44
R38 → ht-kqrt.41, ht-kqrt.67; C2 on core-id rotation, status:null → Unknown, doctor status source, TRUST-POLICY D10/C2/wiped-core rows → (unmapped: owned only by superseded ht-kqrt.42)
R39 → ht-kqrt.43, ht-kqrt.46, ht-kqrt.48, ht-kqrt.74
R40 → ht-kqrt.6, ht-kqrt.50
R41 → ht-kqrt.40, ht-kqrt.45, ht-kqrt.49, ht-kqrt.51, ht-kqrt.52
R42 → ht-kqrt.53, ht-kqrt.54, ht-kqrt.57
R43 → ht-kqrt.55, ht-kqrt.57
R44 → ht-kqrt.53, ht-kqrt.54, ht-kqrt.56
R45 → ht-kqrt.57, ht-kqrt.58
R46 → ht-kqrt.60
R47 → ht-kqrt.65
R-new: A gate-shim-launched hc worker (keep:false) retires its seat through D10 when it exits; no launched worker is left as a held seat → ht-kqrt.68
R-new: The spawned daemon is reparented through a double fork (internal daemon-detach, setsid) and carries the scrubbed env → ht-kqrt.73
R-new: The LocateCaller ancestry walk stops at a foreign terminal root (herdr-core/hc), and its cache key is stamped with the process start time, so a reused pid never hits a stale entry → ht-kqrt.74
R-new: On restore, a named row whose name collides with a non-retired seat is held nameless and is never placed or bound → ht-kqrt.71
R-new: A wake-armed named binding keeps the daemon alive (ActivityProbe::active), and tmux wake is gated on pane mode → ht-kqrt.69 (conflicts with R27/R32, see GAP 3)

- **type:** GAP
- **subject:** R34 (tmux delivery-contract table, AGENTS.md rule 6 Native wording and tmux "What we do today" line, TRUST-POLICY N2 A4 row and accepted limits, T1-T5 record in the run report)
- **evidence:** Ledger C1-4 assigned these to ht-kqrt.39 ("ht-kqrt.39 owns contract table, AGENTS.md rule 6/tmux line, TRUST-POLICY N2 A4, T1-T5 record"). The changes file says ht-kqrt.69 supersedes ht-kqrt.39. The owns: line of .39 still lists them. The visible Owns text of .69 lists only "ComposerTraits::TMUX; the built-in session wake lane (pacer, reachability, Health); the new ActivityProbe::active term …", and .69 has no structured owns: line. ht-kqrt.62 consumes "all N2 leaves" and does not own the docs. Once .39 is closed, nothing owns R34's docs, and AGENTS.md requires same-commit TRUST-POLICY changes for wake.
- **proposed fix:** Add to ht-kqrt.69 an owns: line carrying all of .39's documentation and evidence owns (contract table in the commit message, AGENTS.md rule 6 and tmux line, TRUST-POLICY N2 A4 row including the new keep-alive and pane-mode limits, T1-T5), with acceptance lines. Then close ht-kqrt.39.

- **type:** GAP
- **subject:** R38 (C2 holds on core-id rotation; status:null reads Unknown with the source shown only in doctor; TRUST-POLICY H1 D10 closure table, C2 row, wiped/cloned-core accepted limit)
- **evidence:** These are on the owns: line of ht-kqrt.42, which ht-kqrt.67 supersedes ("D10 rewritten on herdr-core Registry::finish (keep, reason)"). The visible Owns text of .67 covers "enumerate_targets keeping finished local records with reason and keep … the D1…" and nothing on C2 rotation, status:null or the doctor source. .67 has no owns: line. ht-kqrt.63 replaced its dep on .42 with .67, so the gate no longer reaches whatever only .42 delivered.
- **proposed fix:** Give ht-kqrt.67 a structured owns: line that adds C2 on core-id rotation, status:null → Unknown with the doctor status source, and the TRUST-POLICY D10/C2/wiped-core rows rewritten for keep/reason, with tests against fake_hc. Then close ht-kqrt.42.

- **type:** GAP
- **subject:** R32 / R27 (idle exit with only the named host exits, even with tmux-hinted bindings present)
- **evidence:** R32 says "idle exit with only the named host still exits with tmux-hinted bindings present" and R27 says "Idle exit with only the named host exits". ht-kqrt.69 is titled "…idle-exit keep-alive…" and owns "the new ActivityProbe::active term counting wake-armed named bindi[ngs]", which keeps the daemon alive when only the named host holds wake-armed bindings. ht-kqrt.6 owns the idle-exit consumer filter ("idle exit counts only dialled hosts holding a resolved local seat"). The amendment changed the behaviour, but the canonical requirement was not amended.
- **proposed fix:** Amend R32 (and R27 if affected) to the keep-alive semantics, including when a wake-armed binding stops counting. Add acceptance on .69 for both directions: it stays alive while wake-armed, and it still exits once no binding is wake-armed. Or drop the keep-alive.

- **type:** GAP
- **subject:** R22 (restore on a name collision)
- **evidence:** R22 says a seat with display_name comes back unresolved "or stays for the operator when a live seat holds that name". ht-kqrt.71 is a "nameless hold on name collision", and it triggers on collision "with a non-retired seat", not only a live one. The changes file lists "Unchanged behavior (new)" and amended sections but no amended requirement. The canonical R22 and the replacement bead disagree on the trigger (live vs non-retired) and the outcome (operator hold vs nameless hold).
- **proposed fix:** Rewrite R22 to .71's rule, naming the trigger set and how the operator sees the held row, and make .71's acceptance quote it.

- **type:** GAP
- **subject:** R28 (TRUST-POLICY D12 restore row; named half of spike S5) and R17/R28 (TRUST-POLICY D1 inherited-name row; E1 Linux reader)
- **evidence:** Ledger C1-3 says "ht-kqrt.22 (S5)" and C2-1 says ".22 (D12)". The owns: line of .22 is "restore placement rules; TRUST-POLICY D12 restore placement row". The owns: line of .27 has "TRUST-POLICY D1 inherited-name refusal row". The replacements .71 and .72 have no owns: lines, and their visible text names neither the TRUST-POLICY rows nor S5.
- **proposed fix:** Carry the TRUST-POLICY D12 row and the S5 named-half acceptance onto ht-kqrt.71. Carry the D1 row, revised for the same-name limit and the new M6 text, plus the E1 Linux/macOS reader acceptance onto ht-kqrt.72.

- **type:** GAP
- **subject:** tmux turn_state_edge implementations for Codex and agy (ledger: C1-11)
- **evidence:** C1-11 put "Codex/agy turn_state_edge with hook-driven tests" on ht-kqrt.36, and .36's owns: line has "turn_state_edge implementations for Codex and agy over their existing hook events". ht-kqrt.70 supersedes .36. Its visible Owns text names the trait, TurnEdge (including ScreenIdle), daemon turn state and the Claude Stop/StopFailure, but not the Codex/agy adapter implementations. Without them only Claude can wake under tmux, and R29 is not met for Codex/agy.
- **proposed fix:** Add the Codex/agy implementations (src/harness/codex, src/harness/agy only) and their hook-driven tests to an owns: line on ht-kqrt.70.

- **type:** GAP
- **subject:** R38 vs ht-kqrt.68 (exit semantics)
- **evidence:** R38 says "exit/signal/server_stop/host_shutdown unresolve and resume reattaches". ht-kqrt.68 says "a gate-shim-launched worker (keep: false…) retires its seat through D10 when it exits". The canonical requirement does not distinguish keep:false, so .68's acceptance contradicts R38 as written.
- **proposed fix:** Amend R38 to split on keep (keep:true: exit unresolves; keep:false: exit retires), or add the R-new above and qualify R38.

- **type:** ORPHAN
- **subject:** ht-kqrt.22, ht-kqrt.27, ht-kqrt.36, ht-kqrt.39, ht-kqrt.42 (superseded beads still in the tree)
- **evidence:** All five are still listed as tasks with live owns: lines. No task depends on them any more: .34 deps .71/.72, .50 → .71, .52/.63 → .67, .59/.61/.62 → .69. ht-kqrt.66 omits them. Only the internal edge .39 → .36 remains, so .36 and .39 can still come up in `bd ready`. Their owns: lines duplicate the replacements' owns (ComposerTraits::TMUX on both .39 and .69; turn_state_edge on .36 and .70; restore rules on .22 and .71; server environ reader/locator_hint/M6 on .27 and .72). The result is two owners per seam, and duplicate work if one is picked up.
- **proposed fix:** Move every owns: item to the replacement (GAPs 1, 2, 5, 6), then close the five beads as superseded and drop the .39 → .36 edge.

- **type:** UNOWNED-SEAM
- **subject:** No structured owns:/consumes: lines on ht-kqrt.67 through ht-kqrt.74, while existing consumers name their labels
- **evidence:** ht-kqrt.28 consumes "locator_hint on the resolve command (N4-inherit)" and ht-kqrt.47 consumes "server environ reader (N4-inherit)". N4-inherit was .27's label, now .72, which has no owns: line. ht-kqrt.50 consumes "restore rules (N3-restore)" (now .71). ht-kqrt.52 consumes "finish-reason closure (H1c-closure)" (now .67). ht-kqrt.59, .61 and .62 consume "tmux wake (N2-wake)" and "per-binding turn state and turn_state_edge (N2-turn)" (now .69 and .70). The owns appear only as free-text "Owns:" in descriptions, so the graph's owns: lines no longer name these boundaries.
- **proposed fix:** Add owns: (and consumes:) lines to .67-.74 that reuse the labels N4-inherit, N3-restore, H1c-closure, N2-wake and N2-turn, and update the consumers' references.

- **type:** UNOWNED-SEAM
- **subject:** `keep` parameter of terminal.run, exchanged between ht-kqrt.55 / ht-kqrt.54 and ht-kqrt.68 / ht-kqrt.67
- **evidence:** ht-kqrt.68 relies on "a gate-shim-launched worker (`keep: false`…)". ht-kqrt.55 owns "terminal.run client method and its parameter set (no_notify, outlive_parent, no_parent, labels; no caller, no first_message; each checked against the vendored hc-0.1.15 schema)", and that set has no `keep`. R43 lists the same set. ht-kqrt.67 reads `keep` from finished records. No task owns sending keep:false, or checking it against the schema.
- **proposed fix:** Add `keep: false` (schema-checked) to .55's owned parameter set and to R43, with a fake_hc assertion. Have .68 consume it.

- **type:** UNOWNED-SEAM
- **subject:** Idle-exit predicate, shared by ht-kqrt.6 and ht-kqrt.69
- **evidence:** .6 owns "dialled-only consumer filters per 07 §5.3 (idle exit, …)". .69 owns "the new ActivityProbe::active term counting wake-armed named bindi[ngs]", which changes the same idle-exit decision. .69 does not depend on .6, and neither bead names the combined predicate. ht-kqrt.50 tests "seatless core idle exit" against .6's filter only.
- **proposed fix:** Make .69 depend on .6 and own the combined idle-exit predicate, with a test matrix (dialled hosts × wake-armed named bindings). Re-run .50's idle-exit cases against it.

- **type:** UNOWNED-SEAM
- **subject:** `host::foreign_terminal_process_names()` moving from src/host/mod.rs (ht-kqrt.74) into host::hc (H1: ht-kqrt.41 / ht-kqrt.46)
- **evidence:** .74 says "a const list in src/host/mod.rs with herdr-core and hc, moving into host::hc in H1". No H1 bead names the move: .41 owns host::hc::ENV_VARS and .46 owns the full LocateCaller. The wall rule that .46 must keep is not in its consumes.
- **proposed fix:** Add to ht-kqrt.41 (or .46) "owns: move of foreign_terminal_process_names into host::hc" and "consumes: terminal-root wall rule (ht-kqrt.74)", with a dep on .74.

- **type:** UNOWNED-SEAM
- **subject:** ScreenIdle edge, exchanged between ht-kqrt.69 (capture-pane screen read) and ht-kqrt.70 (TurnEdge::ScreenIdle, turn state)
- **evidence:** .70 owns "TurnEdge (including ScreenIdle)" and "screen-idle clear". The screen read that observes idleness is .69's capture-pane, and .69 depends on .70, not the other way round. No owns: line says who emits ScreenIdle into the per-binding turn state, or on what screen evidence (ComposerPolicy::screen_blocked from ht-kqrt.37?).
- **proposed fix:** State on .69 "owns: ScreenIdle emission from capture-pane into turn state" and "consumes: TurnEdge::ScreenIdle (.70), screen_blocked (.37)". Add a test that a missing Stop is cleared only by that path.

- **type:** UNOWNED-SEAM
- **subject:** spawn_detached, shared by ht-kqrt.2 and ht-kqrt.73
- **evidence:** .2 is titled "K5 env scrub: child_env_scrub_list() and spawn_detached". .73 is "spawn_detached reparents the daemon (double fork through internal daemon-detach)" and owns daemon-detach "with the scrubbed env". R9's scrub test has to hold across the extra hop, and neither bead says which one owns the end-to-end assertion that the daemon env is scrubbed after the reparent.
- **proposed fix:** Put the R9 assertion against the reparented daemon's /proc environ on .73, and have it consume child_env_scrub_list (F1-scrub) from .2.

NEEDS-SPEC: ht-kqrt (N2) — whether wake-armed named bindings keep the daemon alive contradicts canonical R27/R32 (.69); the spec must settle which wins.
NEEDS-SPEC: ht-kqrt (H1/H3) — R38's "exit unresolves" vs .68's keep:false "retires on exit", and R22's operator hold vs .71's nameless hold: canonical requirements were not amended with the beads.
