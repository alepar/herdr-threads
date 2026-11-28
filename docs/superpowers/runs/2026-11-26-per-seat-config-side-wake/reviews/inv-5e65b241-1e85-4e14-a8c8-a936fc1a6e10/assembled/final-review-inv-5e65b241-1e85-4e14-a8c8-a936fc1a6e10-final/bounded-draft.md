# Final review (bounded): epic ht-z41e, per-seat config and Codex /side wake

Captured source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head cc35007af2b884809ef14c0c3d93a4d62c776180 (both verified with `git rev-parse --verify <sha>^{commit}`). The caller checkout was at the head SHA with a clean tree when this review started. This review is bounded: the review-inputs sidecar reports status unavailable, sourceAvailable true, wholeRangeReady false, 0 omitted, with two retained provenance gaps (run.md:110 inv-f5f76bdd: incomplete claimed collection; incomplete split bundle). No whole-range package was built.

## Scope and method

- Read the full changed-path inventory: 543 paths, 205 of them product, test, script or doc paths outside `docs/superpowers/runs` and `.superpowers`.
- Read the whole `src/` and `migrations/` diff (61 paths), the design doc with its Requirement baseline (B1 to B10, S1), the AGENTS.md, TRUST-POLICY.md, SKILL.md and agent-usage.md hunks, and four test diffs (queue_runtime, scheduler dispatch, herdr_release_smoke, codex.js stand-in).
- Traced the B8 path beyond the diff: `src/notification/dispatch.rs` (queue gate), `src/store/wake.rs` (`load_candidate`, `wake_target_rows`, `cooperative_authority`), `src/ports.rs` (`ReservedWakeAuthority::targets`), `src/store/read_cursors.rs` (`has_unread`), `src/store/bindings.rs` (`resolve_claim_with_hooks`, `mark_auxiliary`), and the `side-queue/group` driver case in `scripts/wake-e2e.py`.
- Then read the ledger (`progress.md`) for deferred minors, parked lines and recurring clusters. It has no parked or BLOCKED-AUTH lines.

## Requirement trace

- B1, replace the node setting with a seat setting: delivered. `programmatic_wake` is a human-tier `Table` key. It is read in `queue_wake_selection` and again at `arm`, failing closed to native. The node key parses as `Option<bool>` and has no effect, and doctor prints the obsolete-setting line. `QueueWakeRuntime` and `ServiceConfig` no longer carry the flag.
- B2/S1, a closed typed registry: delivered (`src/protocol/seat_config.rs`). An unknown key or value is refused with the valid set, and a `Table` key needs no migration.
- B3/B4, a read/write CLI and a list with descriptions: delivered. `config seat list` (static, `--json`), `get`, `set`, `unset`, and the `human` verbs.
- B5/B6/B7, the tier split: delivered. An agent write of a human-tier key is refused on the client and in the daemon with the exact `ht human config seat ...` spelling. `nudge_threshold` is an agent-tier key over the existing column, and every write records an audit row and re-judges the wake. Human-tier writes are operator decisions, gated by the installed ask rules; the gap on other harnesses is recorded in TRUST-POLICY.
- B9, no native wake typed into a side chat: delivered. The side placeholder is removed from `CODEX_PLACEHOLDERS`; the native test and the ladder test show it defers and retries.
- B8, the wake still reaches the root session while a side chat is open: not delivered from the production entry point in the normal case (see Must fix). The new root selection in `src/store/wake_intents/selection.rs` (`process_root`, `CodexAdapter::wake_process_root`, the sticky `auxiliary_session` bit) is correct in isolation. The dispatcher only reaches it when the reservation has exactly one target, though, and targets are per binding.
- B10: the design choices are implemented as answered. The node setting is removed with no migration, agents write their own seat while a human may name any, the threshold is a key over the existing column, and the spikes ran live on the codex-2 credential (evidence run-2/run-3, spike-side-identity.md).

## Cross-task seams

- Wake dispatch to queue selection, B8. `NotificationDispatcher` computes `use_queue = text == WAKE_UP && poke.is_none() && !using_poke && targets.len() == 1 && !targets[0].dormant` at `src/notification/dispatch.rs:701` (unchanged from base). `targets` comes from `ReservedWakeAuthority::Cooperative.targets`, built by `cooperative_authority` from `wake_target_rows`. That function returns one `WakeTarget` per connected top-level binding on the pane that owes something: all of them when `seat_attention` is set (pending invitation, receipt or warning), else each binding with its own `has_unread`. A `/side` binding shares seat memberships but keeps its own cursor, so any item sent while the side is connected is unread for both bindings. The reservation then has two targets, `use_queue` is false, `QueueWakeRuntime::attempt` returns `Native` before selection runs, and with section 3 the native wake defers until the side closes. The tests miss this. `tests/notification/queue_runtime.rs` calls `runtime.attempt(.., use_queue = true, ..)` directly, the scheduler receipt tests use a fake `QueueNotifier`, and `tests/integration/seat_config_sweep.rs` asserts only the stored value (ledger Task 9 minor). The only end-to-end check is the live `side-queue/group` case, which has not been run (ht-z41e.10 and ht-z41e.31 are open).
- Receipt scope: consistent. `record_contact` stores (seat, native session). `AwaitingReceipt.root_session` is filled only when `take_transport == Queue` and the runtime kept a root, which happens only for Submitted or Unknown. A fallback to native gets no root, so its receipt stays seat-scoped. A `programmatic_wake` flip lifts the unreceived suppression through the witness.
- Auxiliary evidence propagation: consistent at head. `event_claim` copies the bit in `prepare_event_as`, `prepare_qualified_turn` and `tool_boundary_check_in` (the ht-z41e.33 fix), and `drop_unadvertised_auxiliary` gates it on `seat.auxiliary_session_v1`. `decode_request` keeps the frozen bit, and the daemon sets it only on top-level non-human claims in `resolve_claim_with_hooks`.
- Wire compatibility: the new commands and fields are additive with `#[serde(default)]`, capabilities `seat.config_v1` and `seat.auxiliary_session_v1` are advertised, and `WakeAttentionWitness.programmatic_wake` is `#[serde(default)]`.

## Ledger triage

- Recurring minor `[process-class: missing-evidence]` (x5 across ht-z41e.1, .2, .4, .7, .33): a process cluster about logs kept only in scratch, not a product defect. Deferred.
- Task 1 `[unexecuted-test]`: `herdr_release_wake` is an ignored test (it needs a private Herdr), and its `/side` expectation was inverted by this branch. A minor naming an ignored test is misgraded, so it is moved to must fix.
- Task 7 `[marker-deviation]` and `[mis-enrolled-side]`: superseded at head. The marker is now `transcript_path` nullness persisted as `auxiliary_session` (ht-z41e.21), and `process_root` refuses a lone concurrent or auxiliary binding.
- Task 8 `[no-live-run]`: the live side cases are still unrun. This is the gap that hides the dispatcher seam above. Untested scope.
- Task 1 `[doc-drift]` in the ansi-composer README: coherent at head. Task 14 doc-drift and the remaining test-strength minors: deferred.

## Verdict
not ready (B8 queue routing to the root is unreachable whenever a /side binding also owes the item; bounded review, live qualification and full suite outstanding)

## Must fix before landing
- B8 not delivered: `src/notification/dispatch.rs:701` keeps `use_queue` only when `targets.len() == 1`, but `cooperative_authority`/`wake_target_rows` (`src/store/wake.rs`) give one target per connected top-level binding that owes something. A `/side` binding owes every item sent while it is open (shared membership, its own cursor; all bindings when an invitation, receipt or warning is pending), so the reservation has two targets, the queue is never tried, and the wake defers natively until the side closes. Fix: let the queue be tried when every target belongs to one harness process (same harness and fingerprint pid, as `ComposerChoice::Process`), and add a test through the real dispatcher with root and side targets; the runtime and scheduler tests bypass this gate.
- Run the ignored `herdr_release_wake` test (`tests/integration/herdr_release_smoke.rs`, needs a private Herdr) before landing. Its `/side` expectation was inverted by this branch (ledger Task 1 `[unexecuted-test]`, misgraded as minor); it is scheduled in ht-z41e.31.

## Untested scope
- No full-suite measurement of this head exists; the caller runs the sweep after this invocation (gate ht-z41e.27 open).
- Live qualification has not run: `side-queue/group` and `side-native-defer/group` against real Codex, the queue-proof matrix with per-seat enablement, and the config-prose and flag-flip cases (ht-z41e.10, ht-z41e.31 open; driver additions in ht-z41e.30 not yet on this head).
- Bounded review: the test diffs other than the four read here were not inspected, including tests/store/seat_config.rs, tests/service/seat_config.rs, tests/cli/config_seat.rs, tests/hook_entrypoint.rs, tests/store/cooperative_checkin.rs, tests/store/wake_queue_cost.rs, tests/scheduler/wake_receipt.rs, tests/integration/seat_config_sweep.rs, tests/store/schema.rs, tests/store/bindings.rs and the codex-0.163.0-side fixtures.
- Bounded review: scripts not inspected as diffs (scripts/release-qualify, scripts/wake-e2e.py beyond the side-queue case, scripts/wake_e2e_queue.py, scripts/wake_e2e_selftest.py, scripts/wake_e2e_queue_selftest.py, scripts/tests/test_release_qualify.py, tests/release/release_qualify_test.py).
- Bounded review: docs not inspected (CHANGELOG.md, README.md, RELEASE.md, docs/install.md, docs/evidence/ansi-composer/README.md diff, integrations/codex/README.md, docs/validation/herdr-release-matrix.md, docs/superpowers/specs/INDEX.md), the evidence/ logs, and about 338 run-record and .superpowers paths.
- Provenance gap kept from the sidecar: run.md:110 inv-f5f76bdd-1609-4df6-8910-d80c7bb25568, incomplete claimed collection.
- Provenance gap kept from the sidecar: run.md:110 inv-f5f76bdd-1609-4df6-8910-d80c7bb25568, incomplete split bundle (missing cluster report or slices).
- The unbounded whole-epic review with complete provenance is still outstanding (gate ht-z41e.28 open).

## Deferred OK
- `DeliveryPullState.session_contacts` (src/service/delivery_pull.rs) gains one entry per (seat, native session) and is never pruned. Daemon memory grows slowly across many sessions; idle exit bounds it.
- `QueueWakeRuntime.roots` clears every retained root when it reaches 256 (`ROOTS_MAX`), so an untaken attempt is dropped rather than evicted oldest-first. Only queue-delivered attempts that are never taken are affected.
- `GetSeatConfig` checks `seats.instance_id` before `require_local_seat_setting` (src/store/queries.rs), so a remote-homed seat may read `seat not found` instead of the homed-on-node line. Not verified by a test here.
- Ledger Task 11 `[retry-journal-downgrade]`: a journaled retry keeps `auxiliary_session` against a daemon downgraded between record and retry (src/cli/retry.rs). Narrow edge.
- Ledger Task 18 `[regression-guard-only]` and `[missing-evidence]`, Task 20 `[missing-edge-case-test]` (armed queued OutcomeUnknown), Task 5 `[weak-isolation-test]` and `[test-level-gap]`: test-strength gaps with no product defect found.
- Ledger Task 6 `[missing-edge-case-coverage]`: tmux Codex seats stay native under `--queue-proof`, now stated in AGENTS.md and RELEASE.md (Task 19 `[unverified-claim]` not traced here).
- Recurring `[process-class: missing-evidence]` cluster: process hygiene, not product behaviour.
