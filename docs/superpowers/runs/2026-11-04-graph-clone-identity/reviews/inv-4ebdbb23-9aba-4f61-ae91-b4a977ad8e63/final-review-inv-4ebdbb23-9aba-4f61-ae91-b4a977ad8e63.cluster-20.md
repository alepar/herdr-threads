# Final review: cluster 20 (tests-daemon), epic ht-ma6i

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 0755e609de0328e6d73584dd65eb2854e1fd7bf7, both verified with `git rev-parse --verify`. The caller checkout stayed at the head with a clean status throughout. A review-owned detached worktree (seeded copy-on-write) ran the reproduction and was removed afterwards, build output included.

## What the cluster contains

Eight test files, 93 insertions and 4 deletions. Six are mechanical:

- `control.rs` and `harness_evidence.rs` follow the `RetirementStart` return type.
- `inherited_name.rs`, `multi_host.rs`, `node_layout.rs` and `self_retire.rs` add `continuity_key: None` to existing commands.

Two carry new assertions:

- `lifecycle.rs` adds the key to the K5 scrub probe.
- `service_session_capability.rs` adds the v3 routing tests.

## Analysis by file

`service_session_capability.rs` drives the production `serve_registered` loop through `ServiceSessionLevel::required_by`:

- A v1 or v2 session refuses all five v3 operations with `Unsupported`, names `service_session_v3`, stays connected and never reaches the handler.
- A v3 session dispatches every generation.
- The v1 refusal text is pinned to the same string the base hard-coded (`operation requires service_session_v2`), so the refactor changed nothing on the wire.
- Rejection of an unknown capability at registration is pinned in `src/protocol/service.rs` (`register_accepts_v1_v2_and_v3_only`).
- `required_by` is an exhaustive match, so a new operation cannot skip a level without a compile error.

`lifecycle.rs`: the probe entry sets the variable on a real detached child, and `subprocess_environ_scrub_fixture` reads `/proc/<pid>/environ`. That fixture iterates `child_env_scrub_list()`, so on its own it would not notice the name leaving the list. `launch_scrubs_the_continuity_key` in `tests/cli/launch.rs` pins list membership, so the mutation is still caught. `lifecycle_holds_no_host_env_literal` also picks up the new name.

`control.rs` and `harness_evidence.rs`: `into_job().unwrap()` is correct for the keyless fixture, and the wrapper only forwards. The trait method `begin_retirement` has no other `.begin_retirement(` call site in `src`. The two production callers of `begin_retirement_authored` that yield a start value map `Reversible` to `RetiredReversibly` in `observation.rs`.

`self_retire.rs`: `seat retire --self` is named-seat-only (M20) and key holders are unnamed, so a key cannot reach this path. `continuity_key: None` is the honest value here.

`inherited_name.rs` and `node_layout.rs`: a named or hc key is ignored, which `hc_and_named_keys_are_ignored` pins at store level. `herdr_hosts.kind` reads as a non-null string for a lazily attached host, so a keyed claim on one does not error on the kind read.

## Baseline rows

This cluster holds no production path for any row, so end-to-end tracing belongs to the seam reviewer.

- B1 and B2: the part in this cluster is service-session gating, which is delivered. `tests/service/continuity_key_session.rs` and `tests/service/reversible_session.rs` carry the operations past the transport.
- B3: no threads change is required.
- B4: nothing in this cluster touches Herdr.

## Runs performed in the review worktree

- `session_capability_tests`: 5 of 5 passed.
- `daemon::lifecycle::tests::{child_env_scrub_list_is_the_union, lifecycle_holds_no_host_env_literal, detached_daemon_environ_drops_child_env_scrub_list}`: passed.
- A throwaway store test appended to `tests/store/continuity_key_decisions.rs`, not committed: the key's seat is resolved on a second host row whose recovery marker is ahead of its reconciled marker, and the key is presented at an unowned pane on a reconciled host. Output:

```
REPRO seat_host_lagging=1 code=TargetUnresolved conflict=Some("key_live_elsewhere") conflicts=1 alerts=1
REPRO keyless_resolve=TargetUnresolved kept_claims=0
REPRO after_pass_next_event_ok=true conflict=None conflicts=0
```

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- `tests/daemon/multi_host.rs` only ever passes `continuity_key: None`, so no multi-host flow presents a key or closes a key-holding pane, and that gap hides a reproduced behaviour outside this cluster: `key_decision` in `src/store/seats/continuity.rs:588` computes `lag` from the caller pane's host, not the key seat's host, so a key whose seat is still resolved on a restarting host A, presented at an unowned pane on reconciled host B, gets a definitive `key_live_elsewhere` (`TargetUnresolved`, one alert, one hold, zero kept claims) instead of `ServiceBusy`, against spec §4.2 "while a pass can still unresolve the key's seat ... never records a false conflict" and "A keyed claim during lag needs no later event"; no accepted limit covers it, I did not rate it blocking because the single-host case is correct, it needs a cross-host relaunch inside host A's lag window, and the next event from the pane after A's pass reattaches the seat and clears the row (reproduced), though an idle agent stays seatless until that event and the kept-claim block on G5 is absent meanwhile; suggested owner action is the continuity.rs reviewer's call: also read `reconciliation_lags` for the bound seat's host plus a keyed variant of `c1_resume_naming_b_reattaches_seat_from_a`, or record an accepted limit in the spec and TRUST-POLICY.
- The mechanical edits in `control.rs`, `harness_evidence.rs`, `inherited_name.rs`, `multi_host.rs`, `node_layout.rs` and `self_retire.rs` were not executed in this review, and `tests/integration.rs` (which holds `multi_host.rs`) was not built here; the ledger defers the full-suite measurement to the caller, whose run is the evidence for them.

## Deferred OK
- `v3_session_dispatches_every_generation` asserts only the dispatch count because `Recorder` maps every v3 operation to the catch-all `"other"`, so it would not notice one operation dispatched twice in place of another; this is acceptable because `required_by` is exhaustive and `tests/service/continuity_key_session.rs` and `tests/service/reversible_session.rs` exercise each operation by kind.
- The module header of `service_session_capability.rs` still describes only the v1 refusal of v2 operations; a one-line comment refresh with no behavioural effect.
