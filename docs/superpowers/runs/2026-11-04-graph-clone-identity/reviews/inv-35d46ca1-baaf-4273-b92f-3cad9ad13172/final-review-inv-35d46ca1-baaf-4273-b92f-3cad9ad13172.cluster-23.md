# Final review: cluster 23 (tests-cli), epic ht-ma6i

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 1b058307490f29ece8bb4e3fe7a862f8356d992f, both verified with `git rev-parse --verify`. The caller checkout was at the head and clean throughout. No review-owned worktree was needed; everything was read through `git show` and the assembled slices 000119 to 000134.

## Scope read

Sixteen files under `tests/cli/`. Twelve carry only the mechanical `continuity_key: None` and `retirement_state: None` field additions (`cooperative`, `hook_local_evidence`, `hook_neutral`, `hook_pull_lazy`, `nudge_threshold`, `read_cost_names`, `summary`, `topology_handoff`, `topology_recover`, `watch`, plus the same additions in parts of `hook` and `journal`). The behavioural tests are in `hook.rs` (`continuity_gate` and `registered_resume`), `journal.rs`, `launch.rs`, `handoff.rs`, `human.rs` and `caller_location.rs`.

## Requirement baseline trace

- B1 (continuity key), the client half in this cluster: `HookEnv::from_process` reads `CallerEnv::from_process`, `seat_filter` admits the key only at the process entry, `PaneCall::continuity_key` gates on top-level and non-subagent, and `continuity_key_presentation` gates on `seat.continuity_key_v1`. `key_sent_only_when_advertised` pins the key on all three commands on a production hook path through a real socket fixture. The watch claim takes `CallerEnv::for_site` at `src/cli/watch.rs:1291`. Daemon judgment and the service reservation leave this cluster and are the seam reviewer's to trace.
- B2 (pane close, undo): only the display part lands here, `state: retired (reversible)` through `compact_and_human_show_retired_reversible`. The fence and undo are the seam reviewer's.
- B3 and B4: nothing in this cluster touches membership lifetime or Herdr. Every new test runs against stubs or the fixture service, with no real Herdr contact.

## Behaviour checked against production

- The conflict fixture's detail string matches `KeyDecision::refuse` plus `key_conflict_cause` in `src/store/seats/continuity.rs`, including the `KEY_CONFLICT_CAUSE` prefix `api_failure` tests, so `conflict_leaves_hook_successful_and_silent` exercises the real quieting rule.
- `ErrorCode::TargetUnresolved` is the code production uses for hold-bearing kinds, and `TargetAlreadyOwned` for the alert-only kinds; the hook's resolved-pane branch at `src/cli/hook.rs:2082` matches on the code first, so an alert-only refusal takes the ordinary check-in, and `resolve_observed` lets the follow-up keyed `ResolveSeat` proceed for those kinds. By inspection the spec 4.2 row "T's check-in proceeds unbound" holds.
- `keyed_start_on_a_current_mapping_takes_the_ordinary_check_in` asserts on `requests()`, which counts `ContinuityCheckIn` only, so its steady-turn and keyless assertions are real.
- `changed_key_does_not_reuse_the_intent` and `a_keyless_intent_is_not_reused_by_a_keyed_claim` match the `*continuity_key == key` comparison in `continuity_intent`.
- `launch_scrubs_the_continuity_key` and `handoff_scrubs_the_continuity_key` name lists production actually uses: `src/daemon/lifecycle.rs:458`, `scrub_names` and `peer_launcher::child_env`.
- The enroll-only route (`enroll_pane`) always returns empty stdout, so the `stdout.is_empty()` in `failed_capability_read_on_enroll_only_start_ends_quiet` is trivially true there; its `diagnostic` and `enrollment_counts` assertions carry that test, and they are meaningful.
- Repository test rules hold: temp paths through `test_support::scratch::temp_path`, in-process hook tests use `CallerEnv::from_lookup`, no bare spawns, and `launch_gate::prepare` only builds a command.

## Execution evidence

I did not run these tests. The head's tree (43572f92d) is identical to task commit 227af9625, whose owner recorded a passing merge check, and the full-suite measurement is recorded in the ledger as the caller's. Re-running the same tree would duplicate that work. The observations below rest on reading the fixtures and production code, not on mutation runs.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- `src/cli/hook.rs:2160-2165` (a keyed `ResolveSeat` answered `ServiceBusy` ends `Failure::Quiet`) has no test in this cluster: the `registered_resume` fixture's `ResolveSeat` arm can never answer `ServiceBusy`, so removing the branch survives and a keyed lifecycle event whose claim declined (no session or fingerprint failure) at an owned pane during lag would print `seat enrollment: ServiceBusy` to the model; add a `key-busy-resolve.json` control and assert empty stdout and no check-in.
- The spec 4.2 row "bound to S, resolved owner T != S: T's check-in proceeds unbound" is not joined at hook level: `key-conflict.json` answers only `TargetUnresolved` and `keyed_start_on_a_current_mapping_takes_the_ordinary_check_in` answers `NotFound`, so no hook test feeds `TargetAlreadyOwned` with a `continuity key conflict` detail, while `tests/store/continuity_key_decisions.rs` covers only the store half and `tests/clone_identity_sweep.rs` has no `pane_owned_by_other_seat` case; a reorder of `api_failure` or the resolved-pane match would go unnoticed.
- `launch_resolving_another_pane_sends_no_key` in `tests/cli/launch.rs` cannot kill the mutant its comment names: `DaemonSeatResolver` holds no caller env, the test sets no key, and `CallerEnv::from_process` hides the key from a library caller through `seat_filter`, so a resolver changed to read the caller's key would still record `None`; only the literal `None` at `src/cli/launch.rs:421` protects the rule today.
- `key_bound_elsewhere_mints_no_seat_on_plain_cli` in `tests/cli/launch.rs` drives `resolve_result` with a stub answering `ContinuityKeyPending`, which is the row for a key bound to an unresolved seat rather than one live elsewhere, and it never passes through the plain CLI call site at `src/cli/mod.rs:4041`; that site's wiring is covered only by the helper tests in `src/cli/site_capability.rs`, and the sweep's `seat resolve --pane` runs without a key.
- The tests in this cluster were not executed by this review; the evidence is the owner's merge check on the identical tree 43572f92d, and the full-suite run stays with the caller.

## Deferred OK
- Spec 4.7 states the `key` line as 4 bytes plus key and owner and `retired (reversible)` as 9 bytes more than `retired`, while `src/protocol/output_compact.rs` documents 6 bytes and a separate 28-byte line, the latter pinned by `compact_and_human_show_retired_reversible`; the text is operator-facing and the code is self-consistent, so a dated post-implementation note in the design is enough.
- `intent_recorded_before_the_upgrade_decodes_as_keyless` and `replay_presents_the_key` cover `ContinuityCheckIn` and `ResolveSeat` but not `CooperativeCheckIn`; the field carries `#[serde(default)]`, `to_command` clones it, and `key_sent_only_when_advertised` covers the first submission, so the remaining gap is a pre-upgrade decode assertion of one line.
- `invalid_key_sends_nothing_and_prints_nothing_new` compares stdout lengths rather than contents, which is weaker than its name but sufficient given its separate assertion that no command carried a key.
