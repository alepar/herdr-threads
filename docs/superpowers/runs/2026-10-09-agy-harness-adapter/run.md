# super-auto run — 2026-10-09-agy-harness-adapter

flags: planOneShot=false skipPlanRoast=false skipCodeRoast=false autonomous=true
resumeChange: 2026-10-09 · "let's /super-auto this. Fully autonomouns from now, both roasts on" · autonomous on after interactive root design approval; nested brainstorms Mode B; both roasts on (unchanged)
resumeChange: 2026-10-10 · "I think this is way too much text… marks everything as 'untrusted'… I want that corrected… create a new bead task under our epic and handle it ourselves" · added leaf for compact cooperative framing (R17) after the Claude re-fit; supersedes R9 byte-identity for the mod's model-visible frame text only
resumeChange: 2026-10-10 · "the nudge should probably say something minimal, like 'wake up' and then let hook inject the actual message" · added leaf for adapter-declared minimal wake text (R18): agy and Claude mod wake with 'wake up', bodies injected at turn start
resumeChange: 2026-10-10 · "also let's add to AGENTS.md that when designing comms, we strive to design the protocols to be token efficient as much as possible" · folded into ht-poh.13: AGENTS.md 'Token-efficient communication' rule (no dedicated token-efficiency research found in ~/Documents)
phase: finish
codeMechanism: Workflow

spec: 2026-10-09-agy-harness-adapter-design.md
epic: ht-poh
idea: let's pick up adding support for agy. we implemented a prototype of routing agy via openrouter to avoid gemini subscription for testing, so lets use that. /superpowers:super-auto interactive design where we do the briefing, look around, brainstorming and finalizing design together, fully autonomous after that, roasts on. One of the goals - prove our new shiny adapter model. So one of the things I want us to be on the lookout for - how much we will need to touch code outside the adapter. Will we have to extend it scope? Rework existing implementations? callers? -> that can be a final report at the end
branch: super-auto/agy-harness-adapter
base: main

roastDesignRound: 2
roastCodeRound: 2
roastCodeExit: converged
roast-code: 2026-10-10-agy-harness-adapter-roast-pr-1.md, 2026-10-10-agy-harness-adapter-roast-pr-2.md
roast-design: 2026-10-10-agy-harness-adapter-roast-design-1.md, 2026-10-10-agy-harness-adapter-roast-design-2.md

stepBackDesign-round-1: patch — two findings share one cause (transcript-confirm read only at next PreInvocation; Stop used only for error-string detection); cluster C1 rule applied in root + subspecs; executable-identity source patched separately


baseAbsorbed: f3431b31 → d342add2, 14 conflicted files (main v0.6.0 absorbed mid-phase-3: remaining agy delivery work depends on main's lazy hook delivery and permission core; follow-ups ht-poh.16, ht-poh.17, ht-poh.18)

baseAbsorbed: 7c40c83c → cc885360, 1 conflicted files (main v0.6.1/v0.6.2 after the fix loop exited; SKILL.md resolved, mechanical)

feedback: herdr prompt to superpowers workspace pane wB:p8 (2026-10-10), draft upstream-feedback-draft.md
friction: 6 events

codeBuckets:
  completed: every task bead in the tree (root ht-poh closed, stopReason root-closed); ht-poh.13 and ht-poh.27 closed as superseded
  escalated: none (ht-poh.13 superseded by ht-poh.41 after the human ran the golden generator; gate ht-poh.19 released)
  pendingRetry:
  parked:
  stalled: false
  review: not ready (invocation 8 final review: no Critical; Important S1 agy informational notices only at lifecycle check-ins; unmeasured branch; flaky ht-01s + two unclassified flakes) — code-final-review-8.md
  sweep: 417c534 — 4904 passed, 0 failed, 0 errors, 44 skipped; failing: none; command: nice cargo nextest run --locked --all-targets --all-features (272 s, host quiet; an earlier run under other tabs' build load timed out manifest::failpoints_are_compiled_out_of_release_builds at 340 s, which passed alone in 98 s) @ 417c534f
  slowness: round 1 allowlist hot-file cap raised to 4; round 2 graph-bound, edge audit armed; audit 1 left ht-poh.5 <- ht-poh.4.5 drop for an operator (parked)
  worktreesKept: task-ht-poh.13 — uncommitted partial R17 work (BLOCKED-AUTH), task-ht-poh.4.5 — unmerged (blocked on agy binary, since resolved)
  processSweep: stopped 0 · survived 0

stepBackCode-round-1: patch — round 1, no recurrence; 24 findings mostly independent; clusters agy-state-files, ledger-monotonic, hook-silent-failures, attention-block, stale-docs
scopeFilter-round-1: [Should-fix] src/store/lazy_delivery.rs:249 in-scope — Missed wake for a wake_on: all seat is incorrect behavior in goal-named R5/R11 wake.
scopeFilter-round-1: [Should-fix] src/service/delivery_pull.rs:294 in-scope — Missing test for the attention block in the goal-named R4 hook-pull delivery path.
scopeFilter-round-1: [Nit] src/delivery/ledger.rs:137 punch-list — Rotation race only causes a duplicate offer under at-least-once delivery; extra hardening, not a correctness defect.
scopeFilter-round-1: [Nit] src/delivery/ledger.rs:96 punch-list — Needs more than 288 pending items plus failing confirms; cost-only hardening of the offer cap.
scopeFilter-round-1: [Nit] src/harness/agy/delivery.rs:873; src/harness/agy/delivery.rs:880 punch-list — Lock-free read-modify-write of a self-healing hint file; hardening or accepted limit.
scopeFilter-round-1: [Nit] src/harness/agy/delivery.rs:647; src/harness/agy/delivery.rs:644 punch-list — Prune is delayed, not lost, on A2 hint files; housekeeping quality.
scopeFilter-round-1: [Nit] src/harness/agy/delivery.rs:830 punch-list — Performance cost of an unneeded fsync; no behavioral incorrectness.
scopeFilter-round-1: [Nit] src/cli/hook.rs:2960 punch-list — Duplicate journal scan is a performance improvement; behavior is correct.
scopeFilter-round-1: [Nit] scripts/agy-sandbox/fetch-agy:124 punch-list — Manual-only script hardening against a path-component version; same-user, no new reach.
scopeFilter-round-1: [Nit] src/cli/hook.rs:2878 punch-list — Missing stderr diagnostics on fail-open paths is observability polish; delivery behavior is correct.
scopeFilter-round-1: [Nit] src/cli/topology_handoff.rs:192 punch-list — Only the error message on an older daemon is unclean; no working flow regresses.
scopeFilter-round-1: [Nit] src/service/delivery_pull.rs:384 punch-list — Doc-vs-behavior mismatch on replay, an accepted limit; wording fix only.
scopeFilter-round-1: [Nit] src/harness/agy/delivery.rs:842 in-scope — Probation that never clears keeps downgrading delivery, contradicting the design's one-turn rule in the goal-named R4 path, and no test covers it.
scopeFilter-round-1: [Nit] src/harness/agy/decode.rs:98 punch-list — About 140-byte overshoot of a bound that the outer hook cap already enforces; minor hardening.
scopeFilter-round-1: [Nit] scripts/agy-relay/budget.py:71 in-scope — The R12 live-smoke tier's hard budget can be overrun by concurrent requests, so it does not do what it claims.
scopeFilter-round-1: [Nit] src/harness/agy/plugin.rs:60 punch-list — Documentation gap about rollback; not named by the goal.
scopeFilter-round-1: [Nit] src/protocol/delivery.rs:1; src/protocol/delivery.rs:3 punch-list — Doc comment cleanup of the shipped state.
scopeFilter-round-1: [Nit] src/harness/agy/delivery.rs:325 punch-list — Doctor not seeing state write failures is observability hardening beyond the listed doctor conditions.
scopeFilter-round-1: [Nit] src/harness/agy/delivery.rs:909; tests/harness/agy_delivery.rs:794 punch-list — Test-quality and reachability cleanup; the covered behavior is correct.
scopeFilter-round-1: [Nit] TRUST-POLICY.md:211 punch-list — Doc list update, quality only.
scopeFilter-round-1: [FYI] src/store/schema.rs:411; src/store/schema.rs:409 punch-list — Forward-only migration is the established repo convention and pre-existing.
scopeFilter-round-1: [FYI] src/harness/agy/setup.rs:379 punch-list — Comment wording only.
scopeFilter-round-1: [FYI] src/service/delivery_pull.rs:297 punch-list — cluster override: Pre-existing bare-marker convention copied from watch.rs and scoped by design to truncation markers only; not a correctness defect in the new behavior.
scopeFilter-round-1: [FYI] src/service/delivery_pull.rs:176 punch-list — Pre-existing Busy-mapping idiom; observability improvement.
cluster dropped: agy-state-files — send every agy adapter-state file write (conversation state and downgrade.json) through one helper. That helper reuses `harness::manifest::write_private` with a parent-create option instead of the duplicate. It holds an advisory flock around load-mutate-save, skips the write when the serialized bytes are unchanged, does not fsync these A2 hint files, and has a prune pass that rotates its scan start and also removes `.tmp*` leftovers. Every caller in delivery.rs moves onto it: confirm/remove_downgrade, pre_invocation probation, stop, and prune_if_new. If a lock is not wanted, the race goes into the design's accepted limits instead.
cluster dropped: ledger-monotonic — no ledger mutation may lose offer accounting. A write from an older generation is ignored and never wipes a newer entry. An id that has reached MAX_OFFERS_PER_GENERATION is kept as a parked marker instead of being evicted. Apply this in `seat_mut` and `SeatOffers::record`, and check that every other ledger write site follows the same rule.
cluster dropped: hook-silent-failures — fail-open means empty stdout, not lost signal. `HookStep::run` returns an optional diagnostic that is threaded into `HookOutcome.diagnostic`. Every swallowed error in the agy step reports through that channel or through a last-failure record the doctor reads: no daemon, no claim, refused pull, failed confirm, and state or downgrade write failures.
cluster dropped: stale-docs — make one pass over every doc and comment the branch touched, plus any that name a harness list or a capability state, and make each describe shipped behaviour. That covers advertised capabilities, `ANTIGRAVITY_AGENT` in the marker lists (also docs/agent-usage.md:5/:79, docs/operations.md:221, and the ~/.gemini entry in AGENTS.md:10), the not-atomic-CAS wording, confirm replay answering not_offered, the agy downgrade note in install.md and the agy README, and the misplaced or garbled doc comments.
scope-filter: 4 in-scope · 20 punch-listed

parked:
- super-code invocation 1 · graph-change · "ht-poh.5 <- ht-poh.4.5 drop (edge audit, safe no: .5 declares IdleWake::Native but does not call the composer)"
- graph-pass · graph-change · "ht-poh.2.7 <- ht-poh.7 drop (safe no: 2.7 notes snapshots follow ht-poh.7's audit fixes) — with the 4.4 drop, depth 11→10"
- graph-pass · graph-change · "ht-poh.4.4 <- ht-poh.3.6 drop (safe no: 4.4 consumes the SKILL_MD wake-on section; would need a placeholder golden)"
- graph-pass · graph-change · "proposal: seam-contract agy doctor extension point (delivery::doctor_lines) between ht-poh.4.7 and ht-poh.5"
- graph-pass · graph-change · "proposal: split-file src/cli/hook.rs runner-helper boundary between ht-poh.2.6 and ht-poh.4.1"
- 2026-10-10-agy-harness-adapter-roast-design-1.md · escalation · "R5/contract rule 4: Herdr may report agy idle while a subagent generates (herdr issue #2354, open per 2 seats; PR #4278 status and fullyIdle semantics unverified), so a Native wake could type a nudge into a busy agy. Patched in-design: agy ComposerPolicy reads the subagent 'Generating' status as Unreadable (no wake); human to confirm on Herdr 0.9.1."

graph-pass: depth 11→11 · width 3.5→3.5 · applied 2 · parked 4

approvals:
- top-split · auto · ht-poh.1 LEAF, ht-poh.7 LEAF, ht-poh.2 PROMOTE, ht-poh.3 PROMOTE, ht-poh.4 PROMOTE, ht-poh.5 LEAF, ht-poh.6 PROMOTE
- coverage-round-1 · c1 auto applied, c2 auto applied, c3 auto applied, c4 auto applied, c5 auto rejected, c6 auto applied, c7 auto applied, c8 auto applied, c9 auto applied, c10 auto rejected, c11 auto applied, c12 auto rejected, c13 auto applied, c14 auto applied, c15 auto rejected · R1–R13 canonical (+R14 R15 R16 from r-new) · requirements: 13 · mapped: 12 · unmapped: 1 (R13)
- coverage-round-2 · c16–c28 auto applied, c29 auto rejected, c30 auto applied, c31 auto rejected, c32 auto rejected, c33–c35 auto applied, c36 auto rejected, c37 auto applied · requirements: 16 · mapped: 16 · unmapped: 0 · divergence: findings 14 → 26, novel 85%, widening: yes (round 1 inputs hid owns lines and mis-matched epic goals; fixed for round 2) · integration sweep ht-poh.11
