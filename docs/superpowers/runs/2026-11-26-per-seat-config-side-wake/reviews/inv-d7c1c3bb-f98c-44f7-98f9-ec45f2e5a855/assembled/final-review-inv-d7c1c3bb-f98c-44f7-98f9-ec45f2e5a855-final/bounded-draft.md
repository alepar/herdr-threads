# Final review (bounded): ht-z41e per-seat config and Codex /side wake routing

Captured source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head 28e80bf1b0ded34349c62aeee4c3cb08281762f7 (both verified with `git rev-parse --verify`). Caller checkout observed at HEAD 28e80bf1b with a clean status; no caller file was changed by this review.

## Scope of this bounded pass

Review-input selection was unavailable (run.md:110 inv-f5f76bdd-1609-4df6-8910-d80c7bb25568: incomplete claimed collection and incomplete split bundle), so this is a bounded captured-source review, not the whole-range review. The changed-path inventory (551 paths, 347 under docs/superpowers/) was read in full. Inspected: every product source hunk under src/ named in bounded-coverage.json, the three node migrations, TRUST-POLICY.md, AGENTS.md, RELEASE.md, scripts/release-qualify, the skill and agent docs, the herdr_release_wake hunk, one scripts/wake-e2e.py hunk (enable_queue_wake), and the root design doc at head (requirement baseline B1-B10, S1). Not inspected: tests/ (other than the smoke hunk), the remaining scripts/wake-e2e*.py and selftest hunks, evidence/ logs, fixtures, and the docs/superpowers run artifacts.

## Requirement baseline trace

- B1 (replace node setting with seat level): delivered. `InstanceSettings.programmatic_wake` is now `Option<bool>`, parsed and inert; `ServiceConfig`/`QueueWakeRuntime` lost the flag (src/app.rs, src/service/config.rs, src/notification/queue.rs); `queue_wake_selection` reads `seat_config::programmatic_wake_on` in its transaction and `arm` re-reads it (src/store/wake_intents/selection.rs, src/store/wake_intents.rs); doctor prints the obsolete line and the seats with it on; release-qualify drops the node-settings probe.
- B2/S1 (generic key=value, closed typed registry): delivered. `src/protocol/seat_config.rs` REGISTRY with tier, values, default, backing; unknown keys and out-of-set values refused naming the valid set; `Table` keys need no SQL or wire change.
- B3/B4 (read/write CLI; list with descriptions): delivered. `ht config seat list|get|set|unset` (src/cli/commands.rs, src/cli/config_seat.rs); list is static, under 160 bytes per line plus a trailer naming both set spellings; `--json` form.
- B5 (prose to an agent works): delivered for the agent-tier key (`config seat set nudge_threshold N` is allowed by Claude/Codex rules) and for the human tier through `ht human config seat set`, which Claude and Codex prompt on (permission rule tests); the skill and agent-usage docs state the commands once.
- B6/B7 (tiers; nudge_threshold writable by non-human; all readable by non-human): delivered. Agent tier is the seat's own top-level binding via the existing cooperative permit (`set_agent`/`write_threshold`), subagents and read-only bindings refused; human tier is an operator decision `operator:local-user:<uid>` (`set_operator`), `--seat` only on the human verbs; `GetSeatConfig` is a claimless read for any local seat.
- B8 (wake reaches the root session in the background while /side is open, queue on): NOT delivered from the production dispatcher in the common case. Selection (`process_root`, `CodexAdapter::wake_process_root`, the `auxiliary_session` marker) and root-scoped receipt are implemented, but `src/notification/dispatch.rs:698-702` still sets `use_queue` only when `targets.len() == 1`. `wake_target_rows` (src/store/wake.rs:381-449) yields one target per connected top-level binding that owes something; with seat attention (ACK request, invitation) every binding owes, and an item sent while the side is open is unread for both root and side, so the reservation has two targets, the queue is never tried, and the wake goes native and defers until the side closes. This is the open bead ht-z41e.37, whose fix attempt is BLOCKED in the ledger (two triage clarifications, escalated). The live case `side-queue/group` that would catch it has never run (ht-z41e.31 / ht-z41e.10 open).
- B9 (no native wake typed into a side chat): delivered. `CODEX_PLACEHOLDERS` drops the side placeholder; native composer test and the herdr_release_wake smoke hunk assert deferral then wake after close (the smoke test is ignored without a private Herdr and has not been run).
- B10 (design decisions): delivered as designed for the points checked (remove node setting with no migration, own seat for agents with any seat for the human, key over the existing column). The live spikes are recorded as done in the ledger; the per-section live qualification is not.

## Cross-task seams observed

- The selection change (ht-z41e.7/.21) and the receipt change (ht-z41e.35) compose correctly with each other, but neither is reachable for a two-target reservation because the dispatcher gate (pre-existing single-target rule, untouched by any task) filters it out first. No task owned that seam until ht-z41e.37 was filed by the previous final review.
- The auxiliary-session claim field is gated per send on `seat.auxiliary_session_v1` in both hook paths (`cached_event`, `tool_boundary_check_in`); the journaled retry path (src/cli/retry.rs) is not gated, which only matters on a daemon downgrade between record and retry (ledger minor, Task 11).
- `DeliveryPullState.session_contacts` (src/service/delivery_pull.rs) is keyed by (seat, native session) and never pruned, so it grows by one entry per session ever seen for the daemon's lifetime; small, but unbounded.
- `QueueWakeRuntime.roots` clears the whole map when it reaches 256 entries (src/notification/queue.rs), which would drop the root of an in-flight attempt and make its receipt check silently skip awaiting; only reachable if the dispatcher stops taking roots.

## Ledger triage

- The recurring missing-evidence cluster (x5 across ht-z41e.1, .2, .4, .7, .33: logs only in scratch /tmp, no evidence arrays) is a process observation, not a product defect; acceptable to land with, but it means per-task test claims rest on reports, not retained logs.
- Task 1 minor [unexecuted-test] names the ignored `herdr_release_wake` side-close path: an ignored test with changed expectations, so it is regraded as must-run before landing (it belongs to ht-z41e.31).
- Task 8 minor [no-live-run] and Task 22 minor [unexercised-io-paths]: the side-queue, side-native-defer, config-prose and flag-flip driver paths have never run live; untested scope, owned by ht-z41e.31/.10.
- Task 7 minors [marker-deviation] and [mis-enrolled-side] were superseded by the redesign cycle (auxiliary_session marker; the lone-binding rule now reads both bits): resolved at head.
- Task 1 minor [doc-drift] in docs/evidence/ansi-composer/README.md:22 is fixed at head.
- Task 11 minor [retry-journal-downgrade]: deferrable (downgrade-only window).
- Remaining minors (test placement, seam tests, inline SQL, source-text selftest, docstring drift, tmux recipients staying native under --queue-proof, which RELEASE.md and AGENTS.md now state) are deferrable.
- No parked findings and no BLOCKED-AUTH lines; ht-z41e.37 is a BLOCKED task, not a parked finding, and carries the B8 must-fix.

## Verdict

not ready (B8 undelivered: dispatch still single-target-gates the queue (ht-z41e.37 blocked); live qualification and full suite not run; bounded review only)

## Must fix before landing

- B8 not delivered: src/notification/dispatch.rs:698-702 keeps `use_queue` only for `targets.len() == 1`, while wake_target_rows gives root and /side one target each when both owe, so with programmatic_wake on and a /side chat open the queue is never tried and the root is not woken in the background; land ht-z41e.37 (try the queue when every target is one harness process, as the bead says) with a real-dispatcher test.
- Run the ignored herdr_release_wake smoke (tests/integration/herdr_release_smoke.rs side-close path, changed expectation for Section 3) against a private Herdr before landing; regraded from the Task 1 [unexecuted-test] minor.
- Run the live qualification (ht-z41e.31 / ht-z41e.10): side-queue/group under --queue-proof, side-native-defer/group, the reviewed side-nudge/side-draft expectations, config-prose and flag-flip; side-queue is the only end-to-end check of B8 and has never run.

## Untested scope

- Whole-range review outstanding: selection provenance unavailable (run.md:110 inv-f5f76bdd-1609-4df6-8910-d80c7bb25568 incomplete claimed collection and incomplete split bundle); gate ht-z41e.28 is open.
- No full-suite measurement of this branch exists; the sweep is deferred to the caller (gate ht-z41e.27).
- Unreviewed in this pass: all tests/ hunks other than tests/integration/herdr_release_smoke.rs, the remaining scripts/wake-e2e.py, wake_e2e_queue.py and selftest hunks, scripts/tests and tests/release Python tests, tests/fixtures, evidence/ logs, docs/evidence/ansi-composer/README.md hunks, docs/install.md, docs/validation, CHANGELOG.md, README.md, src/cli/nudge_threshold.rs, src/cli/peer_launch.rs, src/harness/agy|opencode|pi decode one-liners, src/harness/claude.rs, src/harness/hermes.rs, src/protocol/handoff.rs, src/protocol/mod.rs, src/protocol/service.rs, src/test_support, src/store/receipts-adjacent one-liners, and the docs/superpowers run artifacts.
- No live Codex run of the auxiliary_session marker through the daemon (spike S1 evidence is PTY-only, ledger Task 3 minors); a root that emits null transcript_path is assumed to lose only queue routing.
- Stale-authority fallback and armed queued OutcomeUnknown receipt are tested at store/fixture level only (ledger Task 5 and Task 20 minors).

## Deferred OK

- Journaled retry check-in is not stripped of auxiliary_session for a daemon lacking seat.auxiliary_session_v1 (src/cli/retry.rs); downgrade-only window (Task 11 minor).
- DeliveryPullState.session_contacts grows by one entry per (seat, native session) for the daemon lifetime with no pruning.
- QueueWakeRuntime.roots clears all retained roots at 256 entries, which could drop an in-flight attempt's root; reachable only if the dispatcher stops taking roots.
- Recurring missing-evidence cluster (logs only under scratch /tmp, no evidence arrays) across ht-z41e.1, .2, .4, .7, .33.
- Test-quality minors: seam-only and redundant seam tests (Tasks 4, 21), weak integration assertions in seat_config_sweep.rs (Task 9), single-seat isolation test (Task 5), source-text selftest ordering (Task 6), regression-guard-only replay test (Task 18), inline final-retirement SQL (Task 13), wake-e2e docstring drift and timed_out assumption (Task 14).
- Tmux Codex recipients stay native under --queue-proof; documented in RELEASE.md and AGENTS.md (Tasks 6, 19).
