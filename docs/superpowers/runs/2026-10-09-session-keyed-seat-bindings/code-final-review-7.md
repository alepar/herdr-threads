# super-code final review (fix round 3)

**Verdict: not ready yet.** I found no open Critical or Important product defect at b0343515. What blocks landing is that nothing measures this tip: fix round 3 changed the delivery hot path after the last green sweep, and that sweep has not run yet.

**Scope and method.** I read the epic spec (`/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/docs/superpowers/runs/2026-10-09-session-keyed-seat-bindings/2026-10-10-session-keyed-seat-bindings-design.md`) and built the review package (`.../.superpowers/sdd/ht-54r-plan/review-026ad3ab..b0343515.diff`). The diff is 238 commits and 789 files; source and migrations account for 102 files and about 13k lines, tests for about 27k. The review was static: I ran no tests. I spent the most time on the binding resolver (`src/store/bindings.rs`), migrations 0032–0035, the cursor walk, ACK settlement, the wake targets, the mod channel, and fix round 3 (ht-54r.93–.96).

## Step 1: my own findings (before reading the ledger)

**Fix round 3 checked.**
- **I1 (ht-54r.93) holds.** `StorePageSource` and `ClientPageSource` now send the binding's claim, so pages are read as that binding (`Reader::Bound`). The attention version now also hashes the binding's own unread count and newest id. `mod_binding_view` computes those per binding.
- **The mod change is safe.** `register.js` now marks every `message` ackable. Ordinary messages always carry receipts (`NotRequired` only on human-waived seats), so the change only advances the cursor over already-settled messages.
- **I2 (ht-54r.94) holds.** The offered ledger is keyed by binding id. The wake outlook adds up only the bindings a wake would reach, and treats a target with no entry as Unknown (wake allowed), which is the safe direction.
- **The earlier review's other fixes are present.**
  - The subagent read-only refusal is decided from the resolved binding's role (`control::require_top_level_binding`).
  - Plain-call reconnect only happens for process exit or migration, and needs a fingerprint and a resolved seat.
  - A lifecycle-claimed binding wins the same-process rule; plain-only bindings and launch placeholders can lose but never win.

**F1. A truncated message can block a binding's cursor (PLAUSIBLE, static trace only).**
- **What I traced.** Cursors advance only through `advance_through_displayed`, a per-call walk over the longest unbroken run of displayed messages. Neither explicit `ack` (`ack_impl`) nor `body <id>` ever advances a cursor; the only callers are `AdvanceCursors`, `CompleteInboxDelivery` and mod/hook settlement. An ordinary message over 8 KiB arrives truncated on the mod or hook-pull path and is refused (`settle::plan`). Reading it with `body <id>` then leaves that binding's cursor stuck in that thread.
- **Effect.**
  - Every later message in that thread stays in the binding's unread set: inbox re-shows it, it counts in the mod fingerprint, and it keeps the binding in the wake-target filter.
  - It clears only when a text `inbox` page shows the message whole.
  - The I1 fix makes this more visible: settled-but-unread items are now re-pushed on every drain. The mod drops them by id, so they are not re-injected, but the wire traffic repeats.
- **Related, lower impact.** A multi-page `inbox` can re-display lazy messages that sit behind an ordinary message not yet shown. The spec mandates this ("unread until its cursor passes it").
- **What is not affected.** No data loss and no spurious wake: lazy and settled items create no wake work.

**F2 (minor).** For a bound reader, the inbox page's warning frontier (`queries.rs:6316`, `:5285`), the wake path (`wake.rs:173`) and `materialization.rs:448` use the seat's newest top-level binding (`SEAT_NOTICE_BINDING_SQL`), not the reading binding. Spec §2 asks for per-binding frontiers. It affects only informational warnings.

**F3 (minor).** `link_lineage` copies the predecessor's cursors using MAX (`bindings.rs:1257`). Spec §1 says the later `pre_llm_call` links lineage "without moving cursors". It can only move cursors forward, and only in the Hermes plain-call-first window.

**Spec coverage.** I found no spec requirement left uncovered by any task.
- Migrations: 0032–0035 are additive, and released 0001–0031 are untouched.
- Human exclusivity, pane-close retirement (existing retirement path), archival scope and the 24 h migrated grace, first-contact stats, fast-forward, and per-binding mod channels with per-binding stall checks are all present.

## Step 2: ledger triage

**Clusters.**
- **[doc-drift] ×6 across 6 tasks.** The normative docs (TRUST-POLICY, agent-usage, operations) fall behind code in each fix round. It is a pipeline defect: fix tasks do not re-read those docs. The instances I checked are fixed (0a2a220c, ht-54r.80/.91/.96). One may still be stale: TRUST-POLICY.md:879 ("no C1 resume continuity"). Deferred OK, but one policy-vs-code read before landing is worth doing.
- **[missing-red-run] ×3 and [doc-formatting] ×3.** Process and cosmetic only. Deferred OK.

**Parked items: all three are resolved.**
- **ht-54r.5** (hermes/continuity hook tests timing out under load): later sweeps were green, 5526/5526 at b6bd4817. Watch for it in the coming sweep.
- **ht-54r.12** (two mod_delivery tests): settled by ht-54r.55 plus the decision taken from the spec.
- **ht-54r.19** (ignored subagent-cursor test): the test is un-ignored, and the branch adds no new `#[ignore]`.

**Minors that could have been misgraded, re-checked.**
- Already fixed by later tasks: single-target composer wake (ht-54r.82/.89), the mod-ACK disconnected gate (ht-54r.81/.87), compact presence (ht-54r.83), the fingerprint-less liveness rule (ht-54r.56), `continuity_candidates` reading only the newest row (ht-54r.78), the provenance overwrite (S5), and the failing topology test (ht-54r.39).
- Task 6's deleted operator test is covered by `operator_ux.rs:687` and `:694`.
- Task 22's human readopt path is unreachable: migrated human rows are never `migrated=1`.

## Verdict
**Not ready.** The code has no open Critical or Important finding. Landing waits on the deferred full sweep.

## Must fix before landing
1. **Run the deferred sweep on b0343515 and get it green.**
   - Steps: the full nextest run, then `scripts/check-no-leaked-processes --run-id`, `scripts/check-harness-neutral`, `scripts/check-default-features`, and the Claude mod JS suite (`integrations/claude/mod/tests/delivery.test.ts`).
   - Why: fix round 3 changed `src/cli/watch.rs`, `src/service/delivery_pull.rs`, `src/delivery/ledger.rs`, `src/scheduler/mod.rs`, `src/notification/dispatch.rs` and `register.js`. It also changed the environment of every tagged test child (`src/test_support/spawn.rs`, `src/test_support/isolation.rs`).
   - The last green sweep was b6bd4817. The merge gate only builds and runs no tests, so this tip is unmeasured.
2. **F1: fix it or record it as an accepted limit.** Either let a full `body <id>` read (and an explicit `ack` of a fully read message) advance the calling binding's cursor, or record the limit in TRUST-POLICY.md. AGENTS.md requires an accepted limit to update TRUST-POLICY in the same commit.

## Untested scope
- **No full-suite measurement of this tip** (see must-fix 1).
- **ht-54r.7 merge (BLOCKED-AUTH).** Its conflicts were resolved outside the pipeline and never seam-reviewed. Final reviews 1–6 and the latest main review covered that area by reading code only.
- **Ledger is short by one.** It counts 59 landings against 58 completions, so a missing entry there is not evidence of a clean task.
- **agy native gate.** Its 15/15 result combines a separate wake run made after a one-line fix; there was no full re-run, and none on this tip.
- **Real-harness and platform cases:**
  - real-harness payloads: Codex SubagentStop/SessionEnd, Hermes `subagent_*` and resume/compression, Claude `CLAUDE_PID` against Herdr's foreground PID with wrapper launches;
  - sandboxes hiding the ancestor PID;
  - a sibling PreToolUse hook dropping `--agent-id`;
  - Linux `/proc` paths (never compiled).
- **Known flaky tests,** never run repeatedly at base: config9, `composition_actual_new_thread_child...`, and the Hermes hook budget tests.

## Deferred OK
- F2 (per-binding warning frontier) and F3 (lineage MAX-copy).
- Performance punch list: `read_cursors` and the `advance_through_displayed` full-thread scan, the lazy probe index, per-thread first-contact queries, and the 4004-row bound in `binding_mail`.
- Fast-forward cursor jumps are not seen by the mod fingerprint.
- Stale `Entry.generation` in the mod channel status.
- Unhandled Claude rewrite shell shapes (an accepted TRUST-POLICY limit).
- Managed-launch placeholder uses the strict single-foreground read.
- Lost lifecycle check-in that is never replayed (accepted limit).
- `Outside-adapter:` trailer missing on pre-rule commits.
- About 400 evidence files under `evidence/` (follows the existing convention).
- Style, doc-formatting and missing-red-run minors.
