# Final whole-epic review (bounded): ht-z41e per-seat config and Codex /side queue wake routing

Captured source: base 236c77a660c3bb19fb31472007d8b0c670f3b907, head c693861ce61741135e7adeaebb939ecb6921fe33
(both verified with `git rev-parse --verify <sha>^{commit}`). Read only from captured Git (`git diff`, `git show`);
no review worktree was created, nothing was built or run. The caller checkout was at HEAD c693861ce, status clean
at the start of this review; no caller commit or status change was observed.

## Route and scope

Bounded route: `review-inputs.json` reports status unavailable, sourceAvailable true, wholeRangeReady false,
924 selected paths, 0 omitted, 2 gaps (run.md:133 inv-f5f76bdd-1609-4df6-8910-d80c7bb25568: incomplete claimed
collection; incomplete split bundle: missing cluster report or slices). The failed assembly (diffstat.txt,
generated-input-provenance.md, review-inputs.json, empty slices/) was preserved untouched.

The range is 924 files, +205416/-593; 707 of them are run archives under docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/.
This pass read the full diff of the 41 product source files listed in bounded-coverage.json (every hunk of each), the
root spec docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-26-per-seat-config-side-wake-design.md at
head, selected head-tree context (src/host/native.rs idle gate and composer reads, src/store/wake.rs
validate_reservation/current_authority/cooperative_authority, src/ports.rs same_scope, the dispatcher's queue
branch), the qualification.md at head, the open child beads, and then the ledger. Docs, scripts and tests were not
read hunk by hunk (only grepped where noted), so they are listed as unreviewed.

## Requirement baseline trace (spec `## Requirement baseline`, B1 to B10, S1)

- B1 (replace node setting with seat level): `InstanceSettings.programmatic_wake` is now `Option<bool>`, parsed and
  unused; `ServiceConfig` and `QueueWakeRuntime` lost the flag; `queue_wake_selection` reads
  `seat_config::programmatic_wake_on` in its transaction and `wake_intents::arm` re-reads it; doctor prints the
  obsolete line. Delivered from the daemon's real wake path.
- B2 with S1 (generic key = value, closed typed registry): `src/protocol/seat_config.rs` REGISTRY with name,
  description, values, default, tier, backing; `seat_config` table (migration 0021) with no key in SQL. Delivered.
- B3 (CLI reads/writes): `ht config seat get|set|unset` and `ht human config seat set|unset`, routed in
  src/cli/mod.rs (selected and unselected paths) to src/cli/config_seat.rs, gated on `seat.config_v1`, decided by
  `DomainService::seat_config_write` (agent tier as a cooperative permit mutation, human tier as an operator decision
  matched against the elected owner). Delivered.
- B4 (list with description, values, default): `config seat list` is static, one line per key plus a trailer naming
  both spellings; `--json` carries the same fields. Delivered.
- B5 (prose instruction works): the list trailer names the exact human spelling and the agent-path refusal prints it;
  qualification.md records config-prose/group with calls `config seat list` then
  `human config seat set --seat <seat> programmatic_wake on` and store value `on` (row failed only on a native-mode
  transport check). Delivered, live evidence predates head.
- B6 (tiered writes, all reads by non-human): agent tier refused for human keys client-side
  (`refuse_agent_config_write`) and daemon-side (`agent_write`), `--seat` refused on the agent path, reads open to any
  caller and subagents; the Claude and Codex permission rules prompt on `human config seat` and allow the agent verbs.
  Delivered (the accepted limit for harnesses without an ask rule is recorded in TRUST-POLICY).
- B7 (nudge threshold settable via non-human path): `nudge_threshold` is agent tier, column-backed through the shared
  `seats::write_column` (wake re-judged, audit row upserted); the `nudge-threshold`/`wake-on` aliases keep their
  path. Delivered.
- B8 (wake to the top-level session beside /side): Codex `wake_process_root` names the plain binding when every other
  binding is auxiliary; `process_root` requires one process; the queue admission `admit_queue_wake` is root scoped;
  receipt is root-session scoped. qualification.md records side-queue-focused/group and shared-server side-queue PASS
  after the ht-z41e.43/.53 fixes. Delivered when `programmatic_wake` is on; with it off the wake defers natively,
  as the spec decided. Live evidence predates head.
- B9 (stop native typing into a side chat): `CODEX_PLACEHOLDERS` drops the side placeholder; a side composer reads
  Text and the native gate refuses. Delivered.
- B10 (brainstorm decisions): remove with no migration, own seat for agents and any seat for the human path, key over
  the existing column, spikes S1-S4 recorded. Delivered.

No baseline row was found undelivered at head by static trace.

## Integration seams examined

- Selection to dispatcher: the dispatcher's early ActiveTurn/HumanInput exemption is limited to a queue candidate
  (`one_queue_process` and `queue_capable`: reserved `programmatic_wake` and a harness wake control); a native path
  that follows still goes through `submit_prompt_if_owed`'s idle gate, which refuses a non-ready status and a
  non-empty composer, so the exemption cannot type into a busy or drafted pane.
- Queue runtime to host: the early `guard()` call before capture was removed; the only host admission is
  `GuardFor::Queue` inside `authorize` right before arm, and `GuardFor::NativeFallback` keeps `check_idle_wake`.
  `admit_queue_wake` reads `read_composer_text` (no window reset) and never touches `empty_windows`.
- Native observation: `reset_empty` dropped `composer_ui && ui != Idle`; `read_composer` already resets the window on
  any non-Empty read or error, and a missing policy reads Unreadable at the gate, so the native dwell is unchanged in
  effect.
- Receipt: `record_contact` now records per (seat, native session); the scheduler awaits the root session only when
  the transport stayed Queue and `take_queue_root` returned the root; a native fallback keeps seat scope.
- Auxiliary marker: carried on `CallerClaim.auxiliary_session` (skip-serialized when false), gated by
  `seat.auxiliary_session_v1` at both hook send sites, stored sticky by `mark_auxiliary` in
  `resolve_claim_with_hooks`, read only by queue selection.
- Retirement: `delete_for_final_retirement` runs in both final-retirement inserts (local and replicated), so the
  `seat_config.seat_id REFERENCES seats(id)` row cannot dangle; operator writes refuse a finally retired seat.

## Ledger triage

- Recurring minor `missing-evidence` (x10 across 9 tasks) and recurring `cleanup-inspection` (x3,
  stop-run-processes --check exit 4): process-class; neither names a product defect. The cleanup tool failure means
  leak checks were manual across the run.
- Task 23 (ht-z41e.31) `confirmed-live-fail`: the listed live FAILs were taken through the fix path afterwards
  (ht-z41e.51 side-native-defer driver close, ht-z41e.52 resume-idle driver, ht-z41e.53 split-pane composer read,
  ht-z41e.54 side-nudge/side-draft/flag-flip driver); qualification.md at head records each as fixed with live PASS
  runs, and busy-invite as a flake (3/3 rerun). Resolved, but every one of those runs predates ht-z41e.59/.60.
- Task 33 (ht-z41e.54) `existing-test-changed` (side-draft queue expectation now `none`): consistent with the amended
  spec, where typed text in the root or a side draft refuses a queue wake; the spec's notes say human confirmation of
  that rule is parked.
- Task 11 (ht-z41e.21) `retry-journal-downgrade`: src/cli/retry.rs is not in the range; a journaled check-in carrying
  the auxiliary bit replayed against a downgraded daemon is refused. Downgrade-only; acceptable to defer.
- Task 35 (ht-z41e.60) `scope-inconsistency`: confirmed by reading; see Deferred OK.
- Task 24 `root-text-proxy`, Task 26 `extra-host-round-trip`: re-read at head; both behave as stated and are safe.
- Task 32 (ht-z41e.53) BLOCKED-AUTH (`git reset --hard a9ec9544e47fc71f3e387defe1e8072640d76011` refused): the bead was
  later closed as already merged, with no new review of that landing.
- Task 34/35 (ht-z41e.59/.60) first passes were BLOCKED on the broken toolbox; their re-entry reports record focused
  tests passing (ports::allocation_tests 13 passed; lib 518 passed plus the named wake tests).

## Verdict
not ready (bounded review; no final-SHA live qualification, no full-suite run, whole-range review outstanding)

## Must fix before landing
- ht-z41e.10 is open: no live qualification exists at c693861ce; every live PASS in qualification.md ran before ht-z41e.59/.60 landed, and ht-z41e.60 changes the wake target set (lazy Process scope) that side-queue and side-nudge exercise, so the Codex --queue-proof matrix, side-queue, side-nudge, side-draft, side-native-defer and the default native matrix must run at the final SHA.
- No full-suite measurement of this head exists (gate ht-z41e.27 open); the caller's sweep must pass before the branch lands.
- The typed-draft queue refusal (a root or side draft refuses a queue wake, replacing the S4-based admit) is an orchestrator narrowing whose human confirmation is parked per the spec's Post-Implementation Notes (ht-z41e.42, ht-z41e.45); it needs the person's decision before landing.

## Untested scope
- Whole-range review outstanding (gate ht-z41e.28): this bounded pass read 41 product source files only.
- Provenance gap: docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/run.md:133 inv-f5f76bdd-1609-4df6-8910-d80c7bb25568 incomplete claimed collection.
- Provenance gap: docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/run.md:133 inv-f5f76bdd-1609-4df6-8910-d80c7bb25568 incomplete split bundle (missing cluster report or slices).
- Unreviewed docs: AGENTS.md, CHANGELOG.md, README.md, RELEASE.md, TRUST-POLICY.md (only grepped for the A5 exception and accepted limits), docs/agent-usage.md, docs/evidence/ansi-composer/README.md, docs/install.md, docs/superpowers/specs/INDEX.md, docs/validation/herdr-release-matrix.md, integrations/codex/README.md, integrations/skill/SKILL.md.
- Unreviewed scripts: scripts/wake-e2e.py (+1168), scripts/wake_e2e_selftest.py, scripts/wake_e2e_queue.py, scripts/wake_e2e_queue_selftest.py, scripts/release-qualify (only grepped), scripts/tests/test_release_qualify.py, scripts/wake-e2e-herdr.
- Unreviewed source: src/app.rs, src/cli/peer_launch.rs, src/harness/agy/decode.rs, src/harness/claude.rs, src/harness/hermes.rs, src/harness/mod.rs, src/harness/opencode/decode.rs, src/harness/pi/decode.rs, src/ports/wake_intents.rs, src/protocol/handoff.rs, src/protocol/mod.rs, src/protocol/results.rs, src/protocol/service.rs, src/service/kicks.rs, src/store/archival.rs, src/store/invitation_content.rs, src/store/queries.rs, src/store/schema.rs, src/store/seats/continuity.rs, src/test_support/history.rs, src/test_support/mod.rs.
- Unreviewed tests: all 127 changed paths under tests/ (including tests/store/seat_config.rs, tests/service/seat_config.rs, tests/cli/config_seat.rs, tests/scheduler/queue_side_dispatch.rs, tests/store/wake_queue_cost.rs and the codex-0.163.0-side fixtures).
- Unreviewed evidence: the 9 paths under evidence/ and the 707 run-archive paths under docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/.
- Task 32 (ht-z41e.53) lost review coverage to a permission refusal (`git reset --hard a9ec9544e47fc71f3e387defe1e8072640d76011`) and was closed as already merged without a new review.
- Leak checks across the run were manual (stop-run-processes --check exit 4, git worktree inventory failed).
- scripts/check-default-features was not run for ht-z41e.42 (deferred to merge time).

## Deferred OK
- src/store/wake.rs validate_reservation builds the current authority with `TargetScope::of(.., false)` while reserve and wake_text_evidence_current use the Process scope for a lazy-only wake; `same_scope` compares `binding_generation`, which is set when any placed row is registered, so a lazy reservation whose only registered row is a side sibling would fail validation. Narrow (root and side of one Codex process are normally both hook-registered); worth aligning in a follow-up.
- src/service/delivery_pull.rs `session_contacts` keys on (seat, native session) and is never pruned, so it grows by one entry per session for the daemon's lifetime.
- src/cli/retry.rs does not strip `auxiliary_session` from a journaled check-in replayed against a daemon without `seat.auxiliary_session_v1` (downgrade only).
- src/harness/codex.rs classifies SessionStart, UserPromptSubmit, PreToolUse and Stop; the spec also names PostToolUse, which Codex's declared hooks do not include. Spec wording drift only.
- The spec's Post-Implementation Notes do not record the second Codex marker (SessionStart fork whose rollout head says `thread_source: tui_side_chat`, ht-z41e.43) or the side-by-side split-pane composer read (ht-z41e.43/.53); TRUST-POLICY documents the marker. Doc drift.
- src/notification/dispatch.rs `queue_text_ok` accepts any target whose text is WAKE_UP rather than the selected root; correct because selection names only a cooperative_top_level root (ledger Task 24).
- A lone working root now reaches capture and prepare before the arm-time admission refuses it: one extra round trip per retry (ledger Task 26).
- Recurring process-class minors (missing evidence records, manual leak checks) carry no product defect.
