# Final review: cluster 37 (tests-store), epic ht-z41e

Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168 (both verified with `git rev-parse --verify`). Caller checkout clean at review start (`git status --short` empty, branch super-auto/per-seat-config-side-wake). Review source bound through dispatch-text.mjs bind-review-source (exit 0).

## Scope and method

The cluster is the 40 files under `tests/store/` (slices 000203-000242). Most of them (31 files) add only `auxiliary_session: false` to `CallerClaim` literals, which the new field requires. Six files carry the substance: `seat_config.rs` (new, 795 lines), `wake_queue_cost.rs` (+360), `cooperative_checkin.rs` (+229), `schema.rs` (+82), `wake_activation_recovery.rs` (+73), `reversible_retirement.rs` (+54), `wake.rs` (+44) and `wake_intents.rs` (+32). I checked them against the design doc `docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-26-per-seat-config-side-wake-design.md` (§Testing, §Selection "Root marker", §Error handling). Where a test's power depends on production code, I opened that code: `src/store/wake_intents/selection.rs` (`process_root`, `queue_wake_selection`, `select_binding`), `src/harness/codex.rs` (`wake_process_root`), `src/store/bindings.rs` (`mark_auxiliary`), `src/store/seat_config.rs` (the final-retirement check) and `tests/fixtures/wake_intent.rs` (`fixture_with`, `enroll_payload`, `set_programmatic_wake`).

This was a static review. I did not build or run the tests: the full-suite run at this head is deferred to the caller (progress.md, Recovery `full-suite measurement deferred to caller`), and building in a fresh worktree would mean a full rebuild.

The mounting is correct. `seat_config.rs`, `wake_queue_cost.rs`, `wake.rs` and `cooperative_checkin.rs` are `#[path]` unit modules of `src/store/seat_config.rs`, `src/store/wake_intents/selection.rs` and `src/store/mod.rs`. `wake_intents.rs` and `wake_activation_recovery.rs` are modules of `tests/combined.rs`. The side-payload fixtures named by `include_bytes!` all exist at head under `tests/fixtures/codex-0.163.0-side/`.

## What the cluster covers against the design's test list

- Registry validation, refusal text and no-write-on-refusal: `unknown_key_and_bad_values_are_refused_naming_the_valid_set` (exact strings, `total_changes` unchanged).
- Default vs explicit, table row deleted on unset, same-value set/unset reports `changed=false`: `fresh_seat_reads_every_key_at_its_default` and `table_key_set_reads_explicit_and_unset_returns_to_default`.
- Column-backed `nudge_threshold`: `nudge_threshold_is_backed_by_the_seats_column` covers the column as the source of truth, default by equality, the NULL-valued audit row and the `wake_on` mirror. `column_unset_writes_the_default_and_upserts_the_audit_row` covers unset. `aliases_upsert_the_audit_row_and_keep_their_clamp` covers the strict range on the generic path against the alias clamp.
- Every change to the column value notes new attention (agent, human, set, unset, alias), with no bump when the value is unchanged, plus the table key: `every_column_value_change_notes_new_attention`.
- Tier rules: an agent is refused on a human key (under 100 bytes, `ht human` spelling), on another seat, as a subagent and as read-only, and the tier must agree with the entry point: `agent_tier_refuses_human_keys_other_seats_subagents_and_readonly`.
- Replay idempotence on both tiers, and a changed payload conflicts: `agent_replay_returns_the_recorded_result_and_a_changed_payload_conflicts`.
- A remote seat is refused before any write: `remote_seat_is_refused_before_any_write`.
- A registry entry alone works end to end (S1, B2): `a_new_table_key_needs_no_schema_change`.
- Retirement: a reversibly retired seat keeps its rows through revival, final retirement drops them, and an operator retire drops them at once: `seat_config_survives_reversible_retirement_and_revival_and_final_retirement_drops_it`. An operator write to a finally retired seat is refused with `seat retired: <seat>` and leaves no row or replay record: `operator_writes_to_a_finally_retired_seat_are_refused_and_leave_no_row`.
- Migrations 0020, 0021 and 0022: DDL shape, and the upgrade never marks an existing row: `schema.rs`.
- `load_candidate` witness carries `programmatic_wake`: `wake.rs`.
- Per-seat flag read on each selection, a toggle without restart, and seat isolation: `per_seat_flag_selects_queue_only_for_the_flagged_seat` and `flag_toggle_applies_on_the_next_selection`. An unreadable seat config fails closed: `seat_config_read_failure_selects_native`. A flag flipped off before arm takes the stale-authority path: `intent_armed_after_the_seat_flips_off_takes_the_stale_authority_fallback`.
- Root marker through real Codex payload decoding, never raw flag writes, for the positive cases:
  - side first seen by SessionStart fork, UserPromptSubmit, PreToolUse or Stop sets only `auxiliary_session`;
  - the mark is sticky;
  - a new binding starts at 0;
  - root plus one or two sides selects the root and names the sides;
  - a later non-null event keeps the mark;
  - a non-`/side` fork keeps the process rootless;
  - a cross-process binding stays native;
  - the census bound is enforced.
- Arrival (R5): a side turn on its own binding leaves the queued intent Active, and a root turn fulfils it: `queued_root_wake_is_fulfilled_by_the_root_turn_and_not_by_a_side_turn`.

Busy-side, receipt-scope and composer tests are outside this cluster. They exist elsewhere in the diff (for example `a_busy_side_binding_does_not_refuse_the_root_wake_and_a_busy_root_does`, `a_queued_root_wake_with_only_side_contact_is_unreceived_and_not_retyped`).

## Why the lone-side test cannot fail

The design requires, in §Selection and §Testing: "A lone connected binding that is `concurrent_session` or `auxiliary_session` is never a queue target ... native fallback with the side alone." This was a cycle-1 roast defect ("a lone such side was queue-targeted"). Production implements it in `process_root`: `[only] => (!only.concurrent && !only.auxiliary).then(...)`. The base selected any lone binding.

The only test of the rule is `a_lone_side_whose_root_is_gone_is_never_a_target` in `tests/store/wake_queue_cost.rs`, and it does not discriminate. `seat_with` runs `qualify(&store, &r, &p)`, which records Busy/Idle native turns only for the fixture root binding (`p.target.scope.binding = "binding"`). `select_binding` returns `None` unless the chosen binding has its own native turn row with phase `Ended` and a matching scope:

```rust
let qualified = super::super::native_turn::load(db, &store.instance, &scope.binding)?
    .is_some_and(|turn| turn.scope == scope && turn.phase == NativeTurnPhase::Ended);
```

The enrolled side has no such row; `wake_activation_recovery.rs` has to record side turns explicitly for that reason. So if `[only]` again returned the lone side, `select_binding` would still refuse it as unqualified, and `native(&select(..))` would still pass. Removing the guard would not turn this test red.

`a_root_that_emits_a_null_transcript_path_makes_the_process_rootless` has a similar weakness, though I have not confirmed it. `enroll_payload` checks SESSION_ROOT in under a new execution id, and `qualify` then reports turns with the fixture's `execution`. Nothing shows that the root would be qualified and selected without its auxiliary mark, so the test has no positive control. `plain_call_bindings_carry_no_evidence_and_stay_native` has the same gap for a "first plain candidate wins" mutation, because the first row under `ORDER BY ordinal DESC` is the unqualified side.

## Requirement baseline trace (this cluster's share)

- B1 (per-seat queue wake replaces the node setting): store selection reads `seat_config::programmatic_wake_on` per attempt (`src/store/wake_intents/selection.rs:174`, `src/store/wake_intents.rs:285`, `src/store/wake.rs:272`); the per-seat, toggle and fail-closed tests above cover it. The removal of the node setting and its doctor line belong to other clusters (seam reviewer).
- B2/S1 (generic closed typed registry): covered by `a_new_table_key_needs_no_schema_change` and the unknown-key refusal.
- B3/B6/B7 (read/write, tiered; nudge threshold writable by an agent): the store entry points `set_agent`, `unset_agent`, `set_operator`, `unset_operator` and `get` are reached from production dispatch (`src/store/mod.rs:1746,1749,2102,2105`, `src/store/queries.rs:409`). The CLI to service path is the seam reviewer's to trace.
- B8 (root routing beside `/side`): positive store selection is covered through real payload decoding. The lone-side negative is covered only vacuously (see above). The production rule exists, so the behaviour is delivered; the gap is regression protection.
- B4, B5, B9 and B10 have no store-test share in this cluster.

## Findings

- important: `tests/store/wake_queue_cost.rs` `a_lone_side_whose_root_is_gone_is_never_a_target` is vacuous. The side binding is never qualified (no `Ended` native turn for its scope), so `select_binding` refuses it whatever `process_root`'s `[only]` arm decides. The design-required "never queue-target a lone concurrent/auxiliary side" rule, a cycle-1 roast regression, has no test that fails when the guard is removed. Fix: record Busy/Idle native turns for the side's own scope (as `queued_root_wake_is_fulfilled_by_the_root_turn_and_not_by_a_side_turn` does), and add a control case (same setup, plain lone binding) that selects, so only the bit check refuses.
- should fix: the native-fallback tests in `wake_queue_cost.rs` (`a_root_that_emits_a_null_transcript_path_makes_the_process_rootless`, `plain_call_bindings_carry_no_evidence_and_stay_native`) assert `binding.is_none()` with no positive control. An unqualified or scope-mismatched candidate gives `None` for reasons unrelated to the rule under test. Add a sibling case that differs only in the factor under test and selects the root, or assert on `process_root` directly.
- nit: `a_non_naming_adapter_stays_native_beside_a_second_binding` would stay native even under the Codex rule. Its second binding is `concurrent_session=1, auxiliary_session=0` (a real fork, which Codex also refuses) and unqualified, and Claude has no `wake_control`. It therefore cannot show that the neutral default `wake_process_root` names no root. Give the second binding `auxiliary_session=1` and qualify the first, so only the adapter default yields `None`.
- nit: `tests/store/cooperative_checkin.rs` `auxiliary_evidence_marks_no_subagent_or_person_binding` discards the subagent check-in result (`let _ = check_in(..)`), so the test passes trivially if that check-in is refused before `mark_auxiliary`. The `Harness::Human` (person) guard named in the test title is never exercised. Assert that the subagent check-in reached a binding, and add a person-claim case.
- nit: `tests/store/seat_config.rs` `operator_write_to_a_reversibly_retired_seat_still_writes` claims to kill a check on `seats.state='retired'`, but it never sets `seats.state='retired'` (a real reversible close does). A check of `retirements OR state='retired'` would pass both retirement tests and still refuse a real reversibly retired seat. Drive it through the real close (as `reversible_retirement.rs`'s `f.close` does), or set the state too.
- nit: `lone_root_selects_with_no_side_sessions` and `a_side_with_no_hook_event_yet_is_invisible_and_the_root_is_targeted` exercise the same state, an empty `seat_with`. The second adds only a binding count.
- nit: `intent_armed_after_the_seat_flips_off_takes_the_stale_authority_fallback` asserts only `arm == None` and phase `Prepared`. It does not observe the `queue wake no longer authorized` NoEffect outcome its comment names.

## Verdict
not ready (the lone concurrent/auxiliary side guard, a cycle-1 regression the design requires tested, has only a vacuous test)

## Must fix before landing
- tests/store/wake_queue_cost.rs a_lone_side_whose_root_is_gone_is_never_a_target: qualify the side binding's own native turn (Busy then Idle on its scope) and add a selecting control, so removing the `!only.concurrent && !only.auxiliary` guard in selection.rs process_root turns the test red

## Untested scope
- Tests in this cluster were not executed by this review; the full-suite run at c3e9ab0b is deferred to the caller per progress.md
- No discriminating test for the lone concurrent/auxiliary binding rule in process_root (see Must fix)
- Neutral default wake_process_root returning None beside a second auxiliary binding is not shown by a discriminating test
- mark_auxiliary's Harness::Human and non-top-level binding guards are not exercised; the subagent case may be vacuous
- Operator write after a real reversible close (seats.state='retired') is not exercised
- Whether the codex-0.163.0-side fixture payloads match live Codex shapes is the fixtures cluster's or seam reviewer's to judge (model and paths are synthetic)

## Deferred OK
- Positive controls for the other native-fallback selection tests (root-null, plain-call)
- Redundant lone-root and no-hook-yet tests could merge
- Stale-authority arm test could assert the NoEffect outcome text
