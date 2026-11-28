# Final review: cluster 34 (tests-root), epic ht-z41e

Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168 (both verified with `git rev-parse --verify`). The review-source binding was recorded with `dispatch-text.mjs bind-review-source` (exit 0). The caller checkout was not modified, and no review worktree was needed.

## Scope

The cluster is three files and three slices (000146, 000147, 000176):

- `tests/combined.rs`: adds `#[path = "integration/seat_config_sweep.rs"] mod seat_config_sweep;` next to `priority_sweep`.
- `tests/contracts.rs`: two `CallerClaim` literals gain `auxiliary_session: false` (`registration_request_binds_operation_and_selected_offer_context` and `operator_boundary_only_accepts_three_administrative_commands`).
- `tests/lifecycle_ux.rs`: one `CallerClaim` literal gains `auxiliary_session: false` (`stopped_daemon_is_unavailable_for_every_descriptor_read`).

## Analysis

Registering the sweep in `combined.rs`:
- AGENTS.md says a suite that keeps no process-wide state belongs in `tests/combined.rs` as a `#[path]` module. `seat_config_sweep.rs` has no `set_var`/`remove_var`, umask change, panic hook, failpoint, bare `std::env::temp_dir()` or bare `Command::spawn`. It drives isolated daemons through `super::lazy_config_smoke::World` and `super::priority_config_smoke` helpers, which are already modules of the same binary (combined.rs lines 201-202 and 216-217), so the `super::` paths resolve.
- `combined` already declares `required-features = ["test-support"]` in Cargo.toml, so the sweep's use of `herdr_threads::test_support` cannot break `scripts/check-default-features`.
- No other target registers the sweep. `git grep seat_config_sweep` at head finds only combined.rs:229-230, so it is not compiled twice.
- Evidence at `evidence/ht-z41e.11/seat_config_sweep.log`: the three sweep tests passed in the combined binary in 7.69 s. That run was on the ht-z41e.11 task worktree, not on this head. It still covers the module's placement, because the later commits that touch this cluster only add the claim field.

The `CallerClaim` fixture updates:
- At head, `CallerClaim.auxiliary_session` is `#[serde(default, skip_serializing_if = "std::ops::Not::not")]` (src/protocol/authority.rs:33-34). `false` is left out when serialized, so these fixtures encode the same bytes they did before the field existed. No hash, payload binding or golden shape that these contract tests check can change.
- Every `CallerClaim {` literal in the two files has the new field (3 of 3: contracts.rs:370, contracts.rs:1762, lifecycle_ux.rs:961). These are struct literals, so a missed one would be a compile error, and the merge check (`cargo check --locked --all-targets --all-features`) would catch it.
- `false` is the right value here. These are legacy-fixture or root claims, not `/side` sessions.

Requirement baseline (B1-B10, S1):
- No production behaviour lives in this cluster. Its only link to the baseline is that it registers the end-to-end sweep for B3-B7 and B9, so that the full-suite run executes it.
- Tracing each row from its production entry point is the job of the clusters that own `src/` and of the seam reviewer. That includes the `ht config seat` CLI, the human-tier write split, `nudge_threshold` as a seat key, `/side` root targeting and the native side-composer deferral.
- Nothing in this cluster lowers or contradicts a baseline row.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- The three updated fixtures only pin `auxiliary_session: false`. Behaviour with `true` (sticky binding bit, capability-gated drop in `cli::site_capability::drop_unadvertised_auxiliary`, root selection) is tested only outside this cluster; the seam reviewer traces it.
- For this head, the only run of `seat_config_sweep` inside the combined binary on record is the ht-z41e.11 task log. The integration branch's full-suite sweep was deferred to the caller (progress.md Recovery record), so no run at c3e9ab0be is on record yet.

## Deferred OK
- none
