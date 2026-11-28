# Final whole-epic review (bounded): ht-z41e per-seat config and Codex /side queue wake routing

Captured source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head 03025d02fbba386d8f4999799b81ffd9b5d968e6 (both verified with `git rev-parse --verify`). Caller checkout observed clean on branch super-auto/per-seat-config-side-wake at the head SHA; nothing was switched or restored.

## Scope of this bounded pass

The review-inputs sidecar reports status unavailable, sourceAvailable true, wholeRangeReady false, 562 selected paths, 0 omitted, and two retained provenance gaps (run.md:110, inv-f5f76bdd-1609-4df6-8910-d80c7bb25568: incomplete claimed collection; incomplete split bundle, missing cluster report or slices). This pass read the full changed-path inventory, then inspected every hunk of 59 paths from captured Git: all product Rust paths except eleven one-line or test-support edits, the three node migrations, TRUST-POLICY.md, the Codex and Claude permission rules, the skill and Codex README text, scripts/release-qualify, four Codex /side hook fixtures and the root design. It traced the baseline rows from production entry points beyond the diff where needed (src/host/native.rs `check_idle_wake`/`idle_wake_gate`, src/notification/queue.rs guard call sites, src/cli/actor_route.rs). The remaining 503 paths are listed as unreviewed below. No build, test or whole-range package was run.

## Requirement baseline trace

- B1 (replace node setting with seat level): delivered. `InstanceSettings.programmatic_wake` is `Option<bool>` with no effect; `QueueWakeRuntime` lost its flag; `queue_wake_selection` reads `seat_config::programmatic_wake_on` in its transaction and `wake_intents::arm` re-reads it; doctor prints the obsolete line and the seats with it on; release-qualify selects queue proof only by `--queue-proof`.
- B2 with S1 (generic key = value, closed typed registry): delivered. `src/protocol/seat_config.rs` REGISTRY; unknown keys and out-of-type values refused naming the valid set; Table-backed keys need no schema change.
- B3 (CLI reads/writes): delivered. `ht config seat get|set|unset` and `ht human config seat set|unset`, gated by `seat.config_v1`.
- B4 (list with descriptions, values): delivered. `config seat list` is static (no daemon) with values, default, tier, description and a trailer naming both spellings; `--json` carries the same fields.
- B5 (telling an agent in prose works): delivered in code and skill text (SKILL.md "Per-seat settings" line, list trailer, human-tier refusal naming the exact `ht human` spelling). The live config-prose wake e2e case has not run.
- B6 (tiers: human-only writes for gated keys, all reads non-human, low-risk keys writable by agents): delivered. CLI refuses `--seat` and human-tier keys on the agent path before sending; the daemon re-checks tier, own seat, subagent and read-only; the human tier is an operator decision of the elected owner's uid; Claude and Codex ask on `human` (accepted limit for other harnesses recorded in TRUST-POLICY).
- B7 (nudge threshold settable by the non-human path): delivered. `nudge_threshold` is tier agent, column-backed, shares `write_threshold`/`write_column` with the aliases, upserts the audit row and re-judges wake.
- B8 (wake reaches the root session in the background while /side is open, queue on): not delivered under the condition the request promises. See the integration seam below: with a /side chat displayed, the queue path's own idle guard reads the displayed side composer, which section 3 now classifies as non-empty, so the queue attempt is refused before it is armed and the root wakes only after the side chat closes.
- B9 (stop native wake typing into a side chat): delivered. `CODEX_PLACEHOLDERS` holds only the main placeholder; `idle_wake_gate` refuses `composer is nonempty`; covered by the native.rs host test.
- B10 (brainstorm decisions): delivered as decided (node setting removed without migration; own seat for agents, any seat for the human tier; key over the existing column; spikes run on an owned PTY rather than an isolated Herdr session, disclosed).

## Cross-task integration seams

- Section 2 composes wrong with section 3 (B8). `NativeWakeDispatcher` passes `guard = || self.host.check_idle_wake(&target, context)` to `QueueWakePort::attempt` (src/notification/dispatch.rs, queue branch). `QueueWakeRuntime::attempt` calls `guard()?` after root selection, and again inside `authorize` before arm (src/notification/queue.rs). On the native host `check_idle_wake` is `idle_wake_gate` plus a budget check, and `idle_wake_gate` reads the pane's composer (`read_composer`, `agent read --source detection`) and returns `TargetUnsafe "composer is nonempty"` for anything but `ComposerRead::Empty` (src/host/native.rs). Before this branch the side placeholder read Empty, so the guard passed; after ht-z41e.1 a displayed side composer reads `Text("Ask a follow-up question")`, so the queue attempt is refused with nothing armed, the dispatcher maps the error to a refusal and the ladder retries until the side chat closes. That is the native-defer behaviour, applied to the queue path too, which contradicts the design goal ("with programmatic wake on whose /side chat is open still gets its idle wake in the root session"), TRUST-POLICY "Queue wake target with /side sessions present" ("the root is targeted while the side view is displayed") and the `side-queue/group` pass criteria. No test catches it: tests/scheduler/queue_side_dispatch.rs and the queue-runtime tests use host fakes whose guard does not read a side composer; the S2/S4 spikes ran with no Herdr host or daemon; the 2026-11-14 queue evidence predates the placeholder change. The live `side-queue/group` run (open bead ht-z41e.31/ht-z41e.10) would be the first to exercise it. When the /side view has been closed but its binding is still connected, the composer reads empty and the queue path selects the root, so only the displayed-side case (the one the user asked about) is lost.
- Queue receipt scope (ht-z41e.22) composes with the side's distinct native session: the fixtures show a distinct `session_id` per /side session, so `contacted_since_session` keyed on the root UUID keeps side contact from settling a queued wake. Correct as traced.
- Auxiliary evidence propagation (ht-z41e.21/.33): `event_claim` is the only place the bit reaches a claim; `drop_unadvertised_auxiliary` gates both cached-event and tool-boundary sends; `decode_request` keeps the frozen bit on replay. The journaled retry path in src/cli/retry.rs was not traced (ledger minor, below).
- The ht-z41e.37 dispatch gate (`one_queue_process`, `queue_text_ok`) is consistent with selection's full-fingerprint `process_root` check; a root without `cooperative_top_level` yields no selection and the wake goes native with the reduced text, as stated.
- `DeliveryPullState.session_contacts` gains one entry per (seat, native session) ever seen and is never pruned, so a long-lived daemon grows it with every resumed or cleared session. Small per entry; no eviction exists. The seat-scoped map had the same shape per seat, so the per-session one is the new growth.
- `QueueWakeRuntime.roots` clears the whole map at 256 entries; an attempt whose root was cleared before `take_root` is then not awaited (no `unreceived` judgement for it). Fail-open on receipt bookkeeping only, not on targeting.

## Ledger triage

- The recurring missing-evidence cluster (x6 across ht-z41e.1, .2, .4, .7, .33, .37) is a process observation about retained logs and structured evidence records. It does not change product behaviour; it does mean per-task test counts cannot be re-read from the repo. Retained as deferred, each instance keeps its own identity.
- Task 1 (ht-z41e.1) [unexecuted-test]: the ignored `herdr_release_wake` side-close path in tests/integration/herdr_release_smoke.rs and the inverted /side expectation (with the codex.js Ctrl-C change) compiled but never ran. A minor naming an ignored test is misgraded; it is must-fix before landing. Its run is in the scope of the open bead ht-z41e.31.
- Task 8 (ht-z41e.8) [no-live-run] and Task 22 (ht-z41e.30) [unexercised-io-paths]: the live side-queue, side-native-defer, config-prose and flag-flip cases have never run. This is untested scope owned by the open live gates ht-z41e.10 and ht-z41e.31, and the root acceptance requires it. Given the B8 seam above, side-queue is expected to fail as built.
- Task 7 (ht-z41e.7) [marker-deviation] and [mis-enrolled-side]: superseded by the redesign (ht-z41e.21 transcript_path marker, auxiliary_session bit, lone concurrent or auxiliary binding never a target). Resolved at head as traced in selection.rs and codex.rs.
- Task 7 [queue-level-side-composer-test]: this is the gap that hides the B8 seam: no queue-runtime-level test reads a side composer through the real guard. Promoted into the must-fix item below rather than deferred.
- Task 11 (ht-z41e.21) [retry-journal-downgrade]: a journaled check-in carrying `auxiliary_session` is not stripped when retried against a daemon that lacks `seat.auxiliary_session_v1`; it fails only across a daemon downgrade between record and retry. Deferred OK as a follow-up.
- Task 24 (ht-z41e.37) [root-text-proxy]: `queue_text_ok` accepts any target with WAKE_UP text; correct only through selection's `cooperative_top_level` root requirement. Deferred OK; a direct root-text check would remove the coupling.
- Task 6 (ht-z41e.5) [missing-edge-case-coverage] and Task 19 (ht-z41e.34) [unverified-claim]: tmux Codex recipients stay native under `--queue-proof`; documented by ht-z41e.34 but not traced here (scripts/wake-e2e.py unreviewed). Deferred OK.
- Remaining minors (test placement, redundant or seam-only tests, source-text test fragility, inline retirement SQL, doc drift in wake-e2e docstrings, single-trial spike evidence, stale-test-tree on a docs-only delta, stop-run-processes inventory failures): no product defect, data-loss path or skipped test named. Deferred OK.
- No `parked`, `parked finding` or `BLOCKED-AUTH` lines exist in the ledger.

## Verdict

not ready (B8 not delivered: the queue guard reads the displayed /side composer and refuses; live qualification, full suite and whole-range review not done)

## Must fix before landing

- B8 not delivered (baseline row B8): with programmatic_wake on and a /side chat displayed, `QueueWakeRuntime::attempt` calls the dispatcher's guard (`check_idle_wake` -> `idle_wake_gate`, src/host/native.rs), which reads the displayed side composer, now non-Empty after ht-z41e.1, and refuses `composer is nonempty` before arm, so the root is not woken until the side closes; make the queue path's idle check judge the root's own state without requiring the displayed /side composer to read empty (keeping identity, status, epoch and budget checks, and keeping native admission unchanged), and add a queue-runtime or dispatcher test whose host fake reads the real side capture through the real gate.
- Run the ignored `herdr_release_wake` side-close path in tests/integration/herdr_release_smoke.rs with its inverted /side expectation and the codex.js Ctrl-C change (ledger Task 1 [unexecuted-test], misgraded minor naming an ignored test); in scope of open bead ht-z41e.31.
- Complete the root acceptance live-qualification row (open beads ht-z41e.10 and ht-z41e.31): side-queue/group under --queue-proof, side-native-defer/group, config-prose, flag-flip and the existing queue-proof cases with per-seat enablement, at the final SHA after the B8 fix.

## Untested scope

- No full-suite measurement of this branch exists yet (gate ht-z41e.27 open; the caller runs the sweep after this invocation); the branch is not treated as tested.
- Live wake e2e cases side-queue/group, side-native-defer/group, config-prose, flag-flip and herdr_release_wake side paths have never run against real Codex or Herdr.
- Spikes S2/S4 ran on an owned PTY without a Herdr host or daemon, so no live evidence covers the queue guard with a displayed /side composer.
- Provenance gap (retained): docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/run.md:110 inv-f5f76bdd-1609-4df6-8910-d80c7bb25568 incomplete claimed collection.
- Provenance gap (retained): docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/run.md:110 inv-f5f76bdd-1609-4df6-8910-d80c7bb25568 incomplete split bundle (missing cluster report or slices); review-inputs exit 3, wholeRangeReady false.
- A whole-range review remains outstanding (gate ht-z41e.28 open); this bounded pass does not replace it.
- Unreviewed product and tooling paths: scripts/wake-e2e.py, scripts/wake_e2e_queue.py, scripts/wake_e2e_queue_selftest.py, scripts/wake_e2e_selftest.py, scripts/tests/test_release_qualify.py.
- Unreviewed small Rust edits: src/cli/peer_launch.rs, src/harness/agy/decode.rs, src/harness/claude.rs, src/harness/hermes.rs, src/harness/opencode/decode.rs, src/harness/pi/decode.rs, src/protocol/handoff.rs, src/protocol/mod.rs, src/protocol/service.rs, src/test_support/history.rs, src/test_support/mod.rs.
- Unreviewed docs: AGENTS.md, CHANGELOG.md, README.md, RELEASE.md, docs/agent-usage.md, docs/evidence/ansi-composer/README.md, docs/install.md, docs/validation/herdr-release-matrix.md.
- Unreviewed tests: 118 changed paths under tests/ (all except the four inspected codex-0.163.0-side fixtures), including tests/scheduler/queue_side_dispatch.rs beyond its header and fixture use, tests/store/seat_config.rs, tests/service/seat_config.rs, tests/cli/config_seat.rs beyond its test names, tests/store/wake_queue_cost.rs and tests/integration/seat_config_sweep.rs.
- Unreviewed run records: 354 changed paths under docs/superpowers/ other than the root design (roast reports, review bundles, spikes, run.md), and 7 paths under evidence/.
- src/cli/retry.rs journaled check-in replay of `auxiliary_session` against an older daemon was not traced.

## Deferred OK

- Recurring missing-evidence cluster (x6: ht-z41e.1, .2, .4, .7, .33, .37): test logs and structured evidence not retained in the repo; process only.
- Task 11 (ht-z41e.21) [retry-journal-downgrade]: journaled auxiliary_session not stripped on retry against a downgraded daemon; follow-up.
- Task 24 (ht-z41e.37) [root-text-proxy]: queue_text_ok checks any target's text, relying on selection's cooperative_top_level root; follow-up to check the root's text directly.
- `DeliveryPullState.session_contacts` grows by one entry per (seat, native session) with no eviction; bound or prune it in a follow-up.
- `QueueWakeRuntime.roots` clears all retained roots at 256; a cleared attempt is not awaited for receipt; acceptable bound, worth a per-entry eviction later.
- Task 6 (ht-z41e.5) and Task 19 (ht-z41e.34): tmux Codex recipients stay native under --queue-proof; documented.
- Task 13 (ht-z41e.24) [inline-sql-duplication]: final-retirement check is inline SQL in src/store/seat_config.rs.
- Remaining task minors (test placement, redundant or seam-only tests, source-text test fragility, wake-e2e docstring drift, REFUSED_OUTCOMES assumption, single-trial spike evidence, stale-test-tree on a docs-only delta, stop-run-processes inventory failures): no product defect.
