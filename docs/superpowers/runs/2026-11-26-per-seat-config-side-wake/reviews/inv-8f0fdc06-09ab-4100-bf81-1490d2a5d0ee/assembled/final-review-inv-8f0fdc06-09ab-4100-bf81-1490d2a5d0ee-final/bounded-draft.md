# Final whole-epic review (bounded) — ht-z41e, super-auto/per-seat-config-side-wake

## Route and source

- Review source bound: invocation inv-8f0fdc06-09ab-4100-bf81-1490d2a5d0ee, base 236c77a660c3bb19fb31472007d8b0c670f3b907, head d919d40fa95af9a597e4c816984d591df028b3e9 (both verified as commits).
- Selection sidecar review-inputs.json: status unavailable, sourceAvailable true, wholeRangeReady false, 914 selected, 0 omitted, 2 retained provenance gaps (run.md:130, inv-f5f76bdd collection and split bundle). This is a bounded captured-source review only; a whole-range review remains outstanding.
- Inspected from captured Git: 43 changed paths, 193 hunks (the core production paths of all three sections: registry, CLI, store, service, protocol, selection, Codex adapter and composer, host queue admission, queue runtime, dispatcher, scheduler, contact tracking, bridge evidence path, ports, permission rules, skill, plus the queue-section hunk of TRUST-POLICY and the sweep failure evidence). Coverage is listed in bounded-coverage.json.
- The caller checkout was not switched, edited or committed; no worktree was created.

## Requirement baseline trace

- B1 (replace node setting with seat level): delivered. `InstanceSettings.programmatic_wake` is now `Option<bool>`, parsed and inert (src/daemon/settings.rs); `ServiceConfig` and `QueueWakeRuntime` lost the flag; selection reads `seat_config::programmatic_wake_on` inside the selection transaction and again at arm (src/store/wake_intents/selection.rs, src/store/wake_intents.rs). doctor reports the obsolete key and the seats with it on (src/cli/doctor.rs). No migration, per B10.
- B2 + S1 (generic key=value, closed typed registry): delivered. `REGISTRY` in src/protocol/seat_config.rs; table-backed keys need no schema change; unknown keys and out-of-range values are refused naming the valid set.
- B3 (read/write CLI): delivered. `ht config seat get|set|unset` and `ht human config seat set|unset [--seat S]` (src/cli/config_seat.rs, src/cli/commands.rs, src/cli/mod.rs), gated by `seat.config_v1`.
- B4 (list with descriptions): delivered. `ht config seat list` is static (no daemon), one line per key plus a trailer naming both spellings; `--json` carries the same fields.
- B5 (prose instruction works): delivered at the documentation layer. The skill states the commands once; the agent refusal for a human-tier key prints the exact `ht human config seat set KEY VALUE` spelling as typed; Claude and Codex permission rules ask on `human config seat` and allow the agent verbs. A live config-prose case exists in the driver (not inspected here).
- B6/B7 (reads non-human, low-risk writes non-human, gated writes human): delivered. `nudge_threshold` is tier agent and column-backed through the shared `write_column` (audit row + `note_new_attention`); `programmatic_wake` is tier human; the CLI refuses an agent write of a human-tier key and `--seat` on an agent write; the daemon independently refuses a human-tier key on the agent path, refuses subagents and read-only bindings, and decides the human tier only as an operator decision of the elected owner's peer.
- B8 (wake reaches the top-level session while /side is open): delivered in code, with an orchestrator amendment awaiting the person. Root naming via `HarnessAdapter::wake_process_root` (neutral default None; Codex implementation in src/harness/codex.rs), sticky `auxiliary_session` evidence carried on every event claim including the tool-boundary path (`event_claim`, `tool_boundary_check_in` in src/harness/bridge.rs; the earlier inv-f5f76bdd B8 defect is closed at head), root-scoped queue admission (`admit_queue_wake`) and root-scoped receipt (`take_root`, `contacted_since_session`). The amended admission refuses the queue wake whenever a side draft is typed; the design records that choice as an orchestrator decision with human confirmation parked.
- B9 (no native wake typed into a side chat): delivered. `Ask a follow-up question` removed from `CODEX_PLACEHOLDERS`; the side-by-side layout reader returns the side pane's read when the root pane is empty, so native typing refuses while a side chat is displayed in either layout.
- B10 (brainstorm decisions): own seat for agents, human may name any local seat; no migration; threshold key over the existing column; spikes recorded. Delivered.

## Cross-task integration notes

- The lazy-frontier widening from ht-z41e.43 (a7d1d92e0) makes every lazy-only wake reserve every connected top-level binding of the seat, not only the queue case it was written for. That composes wrong with the pre-existing per-binding contract exercised by tests/hook_entrypoint.rs, which still expects a lazy candidate to target only the binding with the row unread. The full sweep caught it; the scoped fix (ht-z41e.60, "lazy widening scoped to one process") never landed.
- `StorePort::seat_config_operator` was added as a trait default returning `ErrorCode::Unsupported`, which the repository's own ports guard forbids. Fix ht-z41e.59 (make it a required method) never landed.
- The final live qualification bead ht-z41e.10 depends on both fixes per the planner's edges, so the live evidence on record predates the code that will land.
- Native-gate equivalence: the removed `(composer_ui && ui != Idle)` reset term in `observe_target_with_composer` is covered by `read_composer`'s own reset on any non-Empty or failed read, so native dwell behaviour is unchanged; the queue admission uses `read_composer_text` and never touches `empty_windows`.
- Fail-closed paths check out: unreadable seat config reads off at selection and at arm; a root that turns Busy/Unknown between selection and arm is refused with no native fallback in the same attempt; an admission refusal falls back to the native gate, which refuses non-empty composers.

## Findings

- [important, CONFIRMED by sweep] src/ports.rs:3727 `StorePort::seat_config_operator` trait default returns `ErrorCode::Unsupported`; `ports::allocation_tests::ports_have_no_unsupported_defaults_or_admitted_duplicates` fails at src/ports.rs:5887 ("trait default returns Unsupported near byte 136088"). Sweep at 46cf2eb48, whose code is identical to head (d919d40fa adds only run.md and sweep evidence). Fix bead ht-z41e.59 BLOCKED (toolbox broken, no test run).
- [important, CONFIRMED by sweep] src/store/wake.rs `load_candidate`/`reserve` count `frontier.lazy` as seat attention, so a lazy-only candidate's targets widen to every connected binding; `native_hook_auto_ack_preserves_sibling_unread_and_delivery_wake` fails at tests/hook_entrypoint.rs:2439 (left [a, b], right [b]). Fix bead ht-z41e.60 BLOCKED.
- [important] Final live qualification (ht-z41e.10) at the final SHA is outstanding; the spec's Qualification section requires `side-queue/group` (and the focused busy-side run) at the final SHA, and both pending fixes change code the live cases exercise.
- [needs human decision] Queue admission refuses the root's queue wake on any typed side draft (design "Queue admission (amended 2026-11-27, code cycle 2)": "orchestrator decision of the cycle-2 coverage round; human confirmation parked"; ledger Task 33 changed the side-draft queue expectation to match). B8's promise holds only with no side draft typed.
- [minor] src/service/delivery_pull.rs `session_contacts` gains one entry per (seat, native session) that ever made a claim-bearing call, subagent child sessions included, and nothing evicts it; growth is bounded only by daemon idle exit.
- [minor] Marker-2 gap: with a Codex that persists side rollouts, a side binding first enrolled by a non-SessionStart event (non-null `transcript_path`) is neither concurrent nor auxiliary; while the root is present the process is rootless (native, safe), but if the root binding is gone `process_root`'s single-binding rule targets the side session. TRUST-POLICY's "a session whose hook has run is either marked or a root" overstates this for marker 2 (ledger Task 7 raised the same edge before marker 2 existed).

## Ledger triage

- Recurring missing-evidence (x10 across 9 tasks) and recurring cleanup-inspection (x3; stop-run-processes --check exit 4): process quality, not product defects; they leave test runs unverifiable by reviewers and leave the process sweep with one unknown root (/tmp/sp-6cd888ee). Grouped under untested scope.
- Task 23 (ht-z41e.31) [confirmed-live-fail] was misgraded as minor (it names known product defects). The live FAILs were filed as ht-z41e.51-.54 and commit 38d142f9e records them resolved; re-qualification at the final SHA is the outstanding item above.
- Task 33 (ht-z41e.54) [existing-test-changed]: the side-draft queue refusal is the human-decision item above.
- Task 1 [doc-drift] README and earlier final-review doc drifts (install.md, evidence README): fixed at head (inspected).
- Task 7 [mis-enrolled-side]: narrowed by the lone-binding rule reading both bits, but reopened in a new form by marker 2 (finding above).
- Task 11 [retry-journal-downgrade], Task 24 [root-text-proxy], Task 26 [extra-host-round-trip], Task 13 [inline-sql-duplication], Task 5/9 weak integration coverage, Task 6 tmux recipients native under --queue-proof (documented in AGENTS.md): acceptable to land.
- BLOCKED-AUTH Task 32 (ht-z41e.53): `git reset --hard` refused; the bead's commits were already merged and it was later closed on re-entry without a new review.
- Parked: none. Fix passes: ht-z41e.59 and ht-z41e.60 BLOCKED on a broken build container.

## Verdict
not ready (2 full-suite failures at the code-identical 46cf2eb; fixes ht-z41e.59/.60 blocked; final-SHA live qualification and the side-draft human decision outstanding; bounded review only)

## Must fix before landing
- Fix `ports::allocation_tests::ports_have_no_unsupported_defaults_or_admitted_duplicates`: src/ports.rs:3727 `StorePort::seat_config_operator` default returns ErrorCode::Unsupported (land ht-z41e.59 and run the named tests).
- Fix `hook_entrypoint native_hook_auto_ack_preserves_sibling_unread_and_delivery_wake` (tests/hook_entrypoint.rs:2439): the ht-z41e.43 lazy seat-attention widening in src/store/wake.rs reserves every binding; scope it to the one-process queue case (ht-z41e.60) or justify and update the test, then run it with a_lazy_wake_reserves_a_sibling_that_already_displayed_the_row.
- Run a full-suite sweep that passes on the landing SHA (no passing full-suite measurement of this branch exists; the only sweep failed 2 of 10077).
- Run the final live qualification (ht-z41e.10) at the landing SHA, including side-queue/group with a focused pane and a busy side turn, side-native-defer/group, and the existing side-nudge/side-draft cases, since both pending fixes change exercised code.
- Get the person's decision on the amended queue admission that refuses the root's queue wake when a side draft is typed (design marks human confirmation parked); B8 is otherwise delivered only without a side draft.

## Untested scope
- Bounded review: 871 of 914 changed paths not inspected, including scripts/wake-e2e.py (+1168), scripts/wake_e2e_selftest.py, scripts/wake_e2e_queue.py, scripts/wake_e2e_queue_selftest.py, scripts/release-qualify, scripts/wake-e2e-herdr, all tests/** (seat_config, queue_side_dispatch, wake_queue_cost, wake_receipt and others), RELEASE.md, AGENTS.md, CHANGELOG.md, docs/install.md, the 698 run-evidence files and every TRUST-POLICY.md hunk except @@ -4318,6 +4369,84 @@.
- Uninspected hunks of inspected files: src/host/native.rs @@ -6630,6 +7074,15 @@, src/harness/codex/composer.rs @@ -141,6 +212,164 @@ (split-pane tests), src/cli/mod.rs run_cooperative hunks @@ -5355, -5402, -5596, docs/evidence/ansi-composer/README.md @@ -30,5 +30,6 @@.
- Provenance gap: d919d40fa:docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/run.md:130 inv-f5f76bdd-1609-4df6-8910-d80c7bb25568: incomplete claimed collection.
- Provenance gap: same invocation: incomplete split bundle, missing cluster report or slices.
- No passing full-suite measurement of this branch; the caller's sweep after this invocation has not run.
- Fix passes ht-z41e.59 and ht-z41e.60 ran no tests (build toolbox broken); their outcomes are BLOCKED.
- BLOCKED-AUTH Task 32 (ht-z41e.53): coverage lost to a refused `git reset --hard`; the bead closed on re-entry without a new review.
- Leak checks across tasks were manual (stop-run-processes --check exit 4); final process sweep left one unknown root, /tmp/sp-6cd888ee.
- Recurring missing-evidence (x10 across 9 tasks): test logs kept only in /tmp scratch, not reproducible by reviewers.
- Self-test only, never live: flag-flip never_armed verdict branch (Task 33); armed queued OutcomeUnknown receipt path (Task 20).
- scripts/check-default-features not recorded at head (Task 26 left it to merge time).

## Deferred OK
- src/service/delivery_pull.rs session_contacts has no eviction (one entry per seat and native session, subagents included); file a follow-up to bound or prune it.
- Marker-2 lone-side gap: a side enrolled by a non-SessionStart event with a persisted transcript path, with its root binding gone, is queue-targeted; file a follow-up and reword the TRUST-POLICY "either marked or a root" sentence.
- Task 11 retry journal keeps auxiliary_session against a downgraded daemon between record and retry (deny_unknown_fields refusal).
- Task 24 queue_text_ok accepts any WAKE_UP target rather than the root specifically; correct because selection names only a cooperative_top_level root.
- Task 26 lone working root reaches capture and prepare before the arm-time admission refuses it (per-retry cost, accepted by the amended design).
- Accepted limit: a main-composer draft exactly equal to the side placeholder admits a queue wake (TRUST-POLICY limit 2).
- Task 13 final-retirement check is inline SQL in src/store/seat_config.rs.
- Task 5 and Task 9 weak integration coverage (per-seam tests exist).
- Task 6 tmux Codex recipients stay native under --queue-proof (documented in AGENTS.md).
- Task 23 evidence bulk (about 87 MB of results.json and commands.jsonl) in the repository; consider pruning before publish.
