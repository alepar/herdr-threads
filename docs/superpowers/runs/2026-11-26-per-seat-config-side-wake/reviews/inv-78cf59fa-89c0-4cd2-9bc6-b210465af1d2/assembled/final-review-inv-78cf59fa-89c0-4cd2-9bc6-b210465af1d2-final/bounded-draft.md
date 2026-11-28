# Final review (bounded): ht-z41e per-seat config and Codex /side queue routing

Captured source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head 34d7719a25052191836e0bd8e65d84cf1374053d (both verified with `git rev-parse --verify`). Caller checkout observed clean at the head; it was not switched or restored.

## Route and scope

Bounded route: review-inputs.json reports sourceAvailable true, wholeRangeReady false, status unavailable, no omissions, one retained gap (`run.md:31: malformed collection record: record outside approvals or invalid prefix`). No whole-range package was built. I read the full changed-path inventory (108 paths) and inspected 43 product, doc and test-fixture paths from captured Git; bounded-coverage.json lists every inspected hunk. The tests, the wake e2e driver scripts, the CLI grammar files and the run's roast and coverage records were not read hunk by hunk (listed below as unreviewed).

## Requirement baseline trace

- B1 (replace the node setting with a seat setting): `QueueWakeRuntime` no longer has the process-wide flag (`src/notification/queue.rs`, `src/app.rs`). `queue_wake_selection` reads `programmatic_wake_on` inside its transaction and `arm` reads it again. The node key is parsed as `Option<bool>`, has no effect, and doctor reports it as obsolete. Delivered.
- B2/S1 (generic key=value capability, closed typed registry): `src/protocol/seat_config.rs` holds the `REGISTRY` with type, default, tier and backing. The `seat_config` table (0020) holds the values. Unknown keys and values outside a key's type are refused, naming the valid set. Delivered.
- B3/B4 (read/write CLI and a listing with descriptions): `ht config seat list|get|set|unset` and `ht human config seat set|unset` are in `src/cli/config_seat.rs`. `list` is static, and its trailer names both write spellings. Delivered.
- B5 (telling an agent in prose works): SKILL.md "Per-seat settings" names list/get/set and the human spelling. The agent-tier refusal names `ht human config seat set KEY VALUE`. Delivered.
- B6/B7 (all keys readable by non-human callers; low-risk keys writable by non-human callers; nudge_threshold an agent-tier key): `GetSeatConfig` takes no claim, so any caller can read. Agent-tier writes go through `write_threshold` on the caller's own seat only. Human-tier writes are operator decisions, and the Claude and Codex permission rules ask before `human config seat`. Delivered.
- B8 (wakes reach the root session while /side is open; spikes): spike ht-z41e.6 ran live (S1-S4). Selection now targets the process root named by `wake_process_root`. Codex names the root by `concurrent_session`, not the spike's `transcript_path`; this is disclosed and fails closed when two roots read. Delivered in code. The required live qualification (ht-z41e.10) is still open.
- B9 (remove the side placeholder from the empty-composer list): `CODEX_PLACEHOLDERS` now holds only the main placeholder. Native wake refuses before typing; this is tested in `host::native`. Delivered. The changed release-matrix expectation has never been run (see Must fix).
- B10 (brainstorm decisions): node setting removed with no migration; the human path may name any seat; the threshold is a key over the existing column (`write_column`, audit row). Delivered.

## Cross-task seams checked

- Wake re-judgement: a table-backed change calls `note_new_attention` in `apply`; a column-backed change calls it in `write_column`. The scheduler's `AwaitingReceipt` lifts the unreceived suppression when the witness's `programmatic_wake` flips (`src/store/wake.rs` `load_candidate`). This is consistent across ht-z41e.2, .4 and .7.
- Arm-time recheck plus selection-time read: a flip to off between selection and arm takes the existing stale-authority fallback. This is covered at store level only (ledger minor).
- Final retirement deletes `seat_config` rows in both the local and the replicated retirement-job paths. Reversible retirement keeps them.
- Wire: the new command fields carry `#[serde(default)]` where optional, `seat.config_v1` is advertised, and `PermitMutation::try_from` admits only agent tier with a claim.
- Fail-closed rules: an unreadable flag reads off. A census above 64 bindings, bindings across processes, and an adapter with no named root all select native.

## Concerns

- PLAUSIBLE: Codex root identification counts every binding that has `concurrent_session` set (SessionStart `source: fork`) as a side session. Codex also emits `fork` for a conversation fork the user then works in. If a TUI fork leaves the original session's binding connected in the same process, `wake_process_root` names the original session, which is no longer displayed. The queue then wakes that background thread instead of the displayed conversation. Before this change two bindings always selected native. No test or spike covers a non-/side fork (`src/harness/codex.rs` `wake_process_root`, `src/store/wake_intents/selection.rs` `process_root`).
- PLAUSIBLE (ledger Task 7 minor, retained): a lone side binding first enrolled by a non-SessionStart event, with the root binding gone, reads as non-concurrent and becomes the queue target. That contradicts "side sessions are never targets". The window is narrow, and no marker is persisted that could detect it (`transcript_path` is not stored on the binding row).
- The Codex marker deviates from the spike: the persisted `concurrent_session` is used instead of `transcript_path`. This is disclosed in TRUST-POLICY and AGENTS, and it fails closed only for the case of two roots, not for a mis-enrolled lone side.

## Ledger triage

- Recurring minor `missing-evidence` (×3 across ht-z41e.1, .2, .4): a process-class cluster about test logs kept only in scratch. It does not affect delivery. Deferred, but a full-suite and live run at the final SHA supersede it.
- Task 1 `unexecuted-test`: `tests/integration/herdr_release_smoke.rs` `herdr_release_wake` now expects no wake under /side and a wake after Ctrl-C closes the side chat. It also relies on new Ctrl-C handling in `tests/fixtures/herdr-matrix/codex.js`. The test is ignored (it needs a private Herdr), and no run of it is recorded. Under the triage rule a minor that names an ignored test is graded must-fix.
- Task 8 `no-live-run` and the open bead ht-z41e.10: `side-queue/group` and `side-native-defer/group` have never run against a real Codex. The root-acceptance row `live-qualification` is requiredLive true.
- Task 1 `doc-drift`: line 22 of `docs/evidence/ansi-composer/README.md` still says the /side placeholder "is added to the known placeholders", which contradicts line 21 and the code. Small doc fix.
- Task 5 `weak-isolation-test`, `test-level-gap`; Task 9 `weak-integration-coverage` ×2, `redundant-assertion`; Task 7 `queue-level-side-composer-test`, `missing-red-run`: test-depth gaps. The seams are covered by per-seam tests. Acceptable to land.
- Task 6 `missing-edge-case-coverage`: tmux Codex recipients in the wake e2e never call `enable_queue_wake`, so they stay native under `--queue-proof`. Before this change the node-wide flag queued them. This narrows release queue-proof coverage and should be documented in AGENTS.md. Acceptable to land.
- Task 3 spike deviations (owned PTY, single trial, stripped screen text) and Task 2/4 fixture and evidence notes: disclosed. Acceptable to land.

## Verdict

not ready (bounded review only; the ignored release-smoke /side test has never run; live qualification ht-z41e.10 and the full suite are still outstanding)

## Must fix before landing

- Run the ignored `herdr_release_wake` test (`tests/integration/herdr_release_smoke.rs`, through `scripts/herdr-release-smoke` with a private Herdr) at least once on the final SHA. Its /side expectation was inverted and a Ctrl-C side-close was added to the `codex.js` stand-in in ht-z41e.1, and neither has ever executed (ledger Task 1 `unexecuted-test`; graded must-fix because it names an ignored test).
- Complete the root-acceptance `live-qualification` row (bead ht-z41e.10, requiredLive true): the Codex `--queue-proof` matrix with per-seat enablement, plus `side-queue/group`, `side-nudge/group` and `side-draft/group`, at the final SHA with the codex-2 auth copy. Until then B8's queue routing to the root has no end-to-end evidence through the product path.

## Untested scope

- No full-suite measurement of this branch exists: the sweep (`cargo nextest run --locked --all-targets --all-features`) is deferred to the caller and has not run on head 34d7719a2.
- Live wake e2e cases `side-queue/group` and `side-native-defer/group` have never run against a real Codex (ledger Task 8 `no-live-run`; ht-z41e.10 open).
- No spike or test covers a Codex non-/side fork (SessionStart `source: fork` for a conversation the user then works in) while the original session's binding stays connected; queue root selection in that state is unverified.
- Provenance gap: review-inputs.json selection is unavailable because of `34d7719a25052191836e0bd8e65d84cf1374053d:docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/run.md:31: malformed collection record: record outside approvals or invalid prefix`; a whole-range review remains outstanding.
- Changed paths not inspected in this bounded review: CHANGELOG.md, README.md, RELEASE.md, docs/agent-usage.md, docs/install.md, docs/specs INDEX.md, docs/validation/herdr-release-matrix.md, integrations/codex/README.md, the run's roast, coverage, findings, friction and tree records, spike evidence (side_spike.py, run-2 and run-3 driver.log and side-results.json, leak-check.txt), evidence/ht-z41e.3, .8 and .11 logs, scripts/wake-e2e.py, scripts/wake_e2e_queue.py, scripts/wake_e2e_queue_selftest.py, scripts/wake_e2e_selftest.py, scripts/tests/test_release_qualify.py, tests/release/release_qualify_test.py, src/cli/commands.rs, src/cli/mod.rs, src/cli/site_capability.rs, src/protocol/mod.rs, src/protocol/results.rs, src/store/queries.rs, src/store/schema.rs, and every changed file under tests/ except tests/integration/herdr_release_smoke.rs and tests/fixtures/herdr-matrix/codex.js.
- TRUST-POLICY.md hunks @@ -4202,7, @@ -4215,6 and @@ -4299,10 (queue wake target with /side sessions, stale authority) were not inspected.
- The stale-authority fallback after a programmatic_wake flip to off is tested at store arm level only, not through the queue runtime (ledger Task 5 `test-level-gap`).

## Deferred OK

- Fix line 22 of docs/evidence/ansi-composer/README.md, which still says the /side placeholder is added to the known placeholders (ledger Task 1 `doc-drift`).
- Document in AGENTS.md that tmux Codex recipients in the wake e2e stay native under `--queue-proof` (ledger Task 6 `missing-edge-case-coverage`).
- Codex root marker uses persisted `concurrent_session` rather than the spike's `transcript_path`; disclosed, and it fails closed when two roots read (ledger Task 7 `marker-deviation`).
- A lone side binding mis-enrolled through a later event, with the root binding gone, can be queue-targeted; the window is narrow, and the follow-up is to persist `transcript_path` nullness on the binding (ledger Task 7 `mis-enrolled-side`).
- Recurring `missing-evidence` cluster (ht-z41e.1, .2, .4): test logs kept only in scratch; superseded by final-SHA suite and live evidence.
- Test-depth gaps: per-seat isolation asserted without a second bound seat, flow 1 and flow 3 sweep assertions at the store and composer level, a redundant assertion in the sweep, no queue-level side-composer test, no recorded RED run (ledger Tasks 5, 7, 9).
- Spike method deviations (owned PTY instead of Herdr, one trial each, stripped screen text) and Task 2/4 fixture and evidence notes; all disclosed.
- Wake e2e selftest ordering check through `inspect.getsource` is fragile to refactors (ledger Task 6 `source-text-test`).
