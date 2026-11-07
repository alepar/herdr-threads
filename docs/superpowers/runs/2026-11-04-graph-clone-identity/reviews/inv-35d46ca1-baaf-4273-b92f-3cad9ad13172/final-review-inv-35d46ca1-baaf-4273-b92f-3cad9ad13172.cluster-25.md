# Final review, cluster 25 (tests-daemon), epic ht-ma6i

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 1b058307490f29ece8bb4e3fe7a862f8356d992f. Both verified with `git rev-parse --verify`. The caller checkout read HEAD = head with a clean status at the start and at the last check, so no caller change was observed.

Scope: the eight files and slices 000138 to 000145 that clusters.json lists for tests-daemon. Spec: `docs/superpowers/runs/2026-11-04-graph-clone-identity/2026-11-04-graph-clone-identity-design.md`.

Method: static reading of the slices at head, plus the production sources named below to check named risks. No test was executed and no worktree was created. The ledger defers the full suite to the caller, and the one behavioural claim below is a direct read of a short code path. A clone of the 92G target for a throwaway reproduction was not proportionate to its severity.

## What each file changes

- `tests/daemon/control.rs`: `begin_retirement` now returns `RetirementStart`; the fixture takes `.into_job().unwrap()`. Its seat holds no key, so the job path is the right one.
- `tests/daemon/harness_evidence.rs`: the `publication_order` store wrapper forwards the five new `StorePort` methods and takes the new `begin_retirement` return type. The `src/ports.rs` diff adds exactly those five trait methods, so nothing falls to a default and bypasses the wrapped store.
- `tests/daemon/inherited_name.rs`, `multi_host.rs`, `node_layout.rs`, `self_retire.rs`: `continuity_key: None` on the extended commands. Wire shape only, no behaviour change.
- `tests/daemon/lifecycle.rs`: `HERDR_THREADS_CONTINUITY_KEY` joins `SCRUB_NAMES_PROBE`.
- `tests/daemon/service_session_capability.rs`: `ops_v3()`, a pinned v2 refusal text, `v1_and_v2_sessions_refuse_v3_operations_and_keep_serving`, `v3_session_dispatches_every_generation`.

## Named risks checked

- Session gating (spec 4.3 Service v3, `ServiceSessionLevel`). The tests drive the real `serve_registered` in `src/daemon/transport/service_connection.rs` with a stub handler. The refusal text `operation requires <capability>` matches the asserted strings, and `required_by` is an exhaustive match that maps exactly the five spec operations to V3. `ops_v3()` covers all five. A v1 or v2 session refuses them with `Unsupported`, stays connected and never reaches the handler; a v3 session dispatches all ten. The `Recorder` maps v3 operations to `other`, so the v3 test asserts a count rather than identity. The connection forwards the operation unchanged and `tests/service/continuity_key_session.rs` and `reversible_session.rs` check the real handling, so this is adequate.
- Env scrub (spec 4.2 Hookless callers). `detached_daemon_environ_drops_child_env_scrub_list` sets the variable on the parent, but its fixture iterates `child_env_scrub_list()` and `child_env_scrub_list_is_the_union` does not chain `CONTINUITY_KEY_ENV`. Removing the name from the list would therefore pass this file. `launch_scrubs_the_continuity_key` (`tests/cli/launch.rs`) and `handoff_scrubs_the_continuity_key` (`tests/cli/handoff.rs`) pin it, so the list is guarded, only not here. `lifecycle_holds_no_host_env_literal` correctly gains the new name.
- Self retire of a key holder. Key holders are unnamed Herdr pane seats and `refused_for_pane_seat_m20` refuses self retire there, so the path cannot reach a key. No test is missing in `self_retire.rs`.
- Named and hc seats presenting a key. The cluster passes `None`; `tests/store/continuity_key_decisions.rs` covers the ignored case at store level.
- Lag host across two servers. See the items below. `key_decision` in `src/store/seats/continuity.rs` reads `lag: reconciliation_lags(tx, host)` where `host` is the claim's pane host, and `judge` then answers `Conflict(KeyLiveElsewhere)` for a bound seat reading `Resolved`. Spec 4.2 defines lag as "while a pass can still unresolve the key's seat", and that seat can sit on another server.

## Requirement baseline trace

- B1 and B2: this cluster holds the v3 gate on the real connection path, the scrub and the extended command shapes. It does not hold the reserve, reattach, reversible or undo paths; those leave the cluster and are the seam reviewer's to trace from the production entry points.
- B3: no threads change by design; nothing in this cluster bears on it. Seam reviewer's to trace.
- B4: no file in this cluster touches Herdr or changes how it is called.

No baseline row is shown undelivered from within this cluster.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- `tests/daemon/multi_host.rs` has no keyed scenario, and `tests/store/continuity_key_decisions.rs` uses a single-host fixture, so no test presents a continuity key on a server other than the one its seat sits on: neither a cross-server `Reattach` (it shares the C1 transaction that `c1_resume_naming_b_reattaches_seat_from_a` proves keyless) nor the lag case in the deferred item below.
- This review executed no tests; the cluster's new tests (`v1_and_v2_sessions_refuse_v3_operations_and_keep_serving`, `v3_session_dispatches_every_generation`) rest on the task merges' recorded check passes and the caller's deferred full-suite run.

## Deferred OK
- `src/store/seats/continuity.rs` `key_decision` judges lag on the claim's host, not the key's seat's host: with seat S resolved on server A during A's reconciliation lag and a keyed claim from an unowned pane on reconciled server B, the verdict is a durable `key_live_elsewhere` hold and alert instead of `ServiceBusy` and a kept claim, contrary to spec 4.2's "never records a false conflict" and "needs no later event"; read from the code, not reproduced, and it self-heals on the pane's next keyed event (S is then unresolved and reattaches), so an idle agent waits for that event; it needs a cross-server relaunch racing the home server's restart, the likely fix is to read lag for the seat's host as well, and the file belongs to another cluster, so the store and seam reviewers should weigh it and a follow-up bead should carry it.
