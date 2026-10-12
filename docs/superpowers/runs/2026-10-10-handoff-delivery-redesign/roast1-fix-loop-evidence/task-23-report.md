# Task ht-0lo.15 source implementation

Status: IMPLEMENTED (source-only implementation; parent causal RED and GREEN measured, combined gates pending).

Base: `bdae610212ed26a095ca100bd1dc7aa1bd02fac2`.
Branch: `super-auto/roast1-ht-0lo-15`.

## Implementation

Both V2 Pull and Confirm acquire their existing exact owner permit through one budget-aware helper. It checks the caller budget before attempting the mutex and again after acquisition, recovers the guard from poisoning as before, and waits on the cancellation token for at most the smaller of 10 ms and the remaining deadline. The returned guard occupies the existing route scope, retaining the owner fence across admission, canonical store work and successful progress installation. No permit identity, public wire, store, capacity, trust or canonical-history change is introduced. Existing budget errors are reused; no new model-visible string is added.

The registered `tests/service.rs` module uses real temporary SQLite storage, canonical recipient and sender lifecycle check-ins, a canonical Lazy invitation with a genuine outstanding token, and the real channel registry's owner mutex. A narrow forwarding observer signals acquisition of that exact permit; it does not replace admission, canonical work or replay state. Cancellation/deadline is triggered after this signal while the owner is held. The result is captured with a bounded timeout before owner release; cleanup releases and joins the worker before the intended RED assertion. Snapshots cover private presentation, binding, reader, receipt, invitation, membership and wake state. Successful contention controls verify no completion or canonical change while held, then a real page/confirmation after release. A Watch replay-application assertion checks the mutex is still held through successful progress installation. Poisoned-owner controls exercise both routes.

## Commits

- Tests-only: `aed4b720084511464ca7065b8ff1b8fca059d263`.
- Tests-only canonical-token fixture correction: `36c3988fe8fd0e46861170c182d1160afda00fc4`.
- Tests-only public peer factory correction: `741984b9d6d9ca8459416a5ef82db1dd2a812591`.
- Production: `3b1113f0d1938ee065324f39d7590b250a4058a7`.

The fixture corrections precede production in branch ancestry. Parent provenance retains the first tests-only SHA. The superseded production object `4d194c63` has the identical production diff but is not the final branch head.

## Files changed

- `tests/service.rs`
- `tests/service/presentation_owner_budget_controls.rs`
- `src/service/delivery_presentation.rs`

## Test selectors and intended values

Command for the owner module: `nice cargo test --locked --all-features --test service presentation_owner_budget_controls`.

Regression selectors:

- `presentation_pull_cancelled_owner_waiter_returns_before_release_without_effects`
- `presentation_pull_expired_owner_waiter_returns_before_release_without_effects`
- `presentation_confirm_cancelled_owner_waiter_returns_before_release_without_effects`
- `presentation_confirm_expired_owner_waiter_returns_before_release_without_effects`

Intended RED on the corrected tests-only stack: each fails at `exhausted waiter must return BEFORE owner release` because the blocking production mutex does not respond while held. Each loops Hook before Watch, so a baseline Hook assertion failure leaves its Watch iteration and later no-effect assertions unmeasured. Intended GREEN with production: the waiter returns `Cancelled` or `DeadlineExceeded` respectively before owner release; snapshots are equal; confirming the original token afterward returns `Confirmed` for both consumers.

Control selectors:

- `presentation_live_owner_waiters_serialize_and_succeed_after_release`: intended PASS before and after production; every Hook/Watch Pull/Confirm waiter remains pending and leaves snapshots unchanged while held, then returns the original Page token or `Confirmed` after release.
- `presentation_poisoned_owner_recovers_for_pull_and_confirm`: intended PASS before and after production; Hook/Watch Pull yields Page and Confirm yields `Confirmed` after isolated owner poisoning.

## Evidence and runtime limits

I ran no Cargo, build, test, daemon, Node/plugin, socket, Herdr or tracker mutation command, per parent runtime override. I ran `rustfmt --edition 2024 --config skip_children=true` only on changed Rust files and `git diff --check`; these are formatting/static checks, not runtime evidence. Own worktree is clean at the reported head.

Parent supplied an actual diagnostic build at the initial tests-only SHA, before the peer-factory correction: `/private/tmp/ht-roast1-rust-stage2-aed4b720/list.stderr` reports E0624 at the private `PeerIdentity::from_kernel(501)` call and exits 101 before tests. This is not causal RED evidence. The correction uses existing public `herdr_threads::test_support::peer_identity(501)` and adds no authority API. Parent reported 98.564 seconds and zero process leaks for run `1ec36781`. Corrected causal RED/GREEN and required gate logs remain parent-owned and will be appended when supplied.

## Test hygiene

No runtime/helper/background process started by this implementer. No runtime temp state created. Parent reported zero leaks for the diagnostic build above. Other parent-run hygiene awaits parent evidence.

## Assumptions and concerns

Current bead description and frozen Task 23 agree; no task amendment/comment changes apply. Missing brief setup was corrected by the parent using the shipped task-brief helper. TRUST-POLICY was read. Test fixture storage is per test under the parent-configured TMPDIR; no real home/config or shared Herdr server is used.

The bounded owner wait polls release every at most 10 ms while cancellation can wake it immediately, following existing budget-aware lane style. This preserves correctness and adds no caller budget extension. Runtime correctness and gate success are deliberately unclaimed until the parent supplies actual output. The canonical-invitation fixture correction was identified by source inspection: empty joined threads have no invitation/control descriptors and cannot guarantee an outstanding Confirm token.

## Parent causal RED evidence

Parent ran the corrected tests-only stack against unchanged owner production on owned validation commit `046100d70eedc1076279426e64c5949464e09f48`. Exact command is retained verbatim in `run.argv` in `/private/tmp/ht-roast1-rust-stage2-corrected-741984b9/execution.json`; it is `nice cargo nextest run --locked --all-features --lib --test service --no-fail-fast -E` with the 13 exact selected tests listed there (the six owner selectors above and seven independent source/migration controls). Output is `/private/tmp/ht-roast1-rust-stage2-corrected-741984b9/run.log`.

Measured owner result: four expected FAIL, two positive PASS. Each of the four regression tests reached its real held exact-owner permit signal, then failed at the before-release bounded receive, after releasing/joining its worker:

```
exhausted waiter must return BEFORE owner release: Timeout
PASS presentation_poisoned_owner_recovers_for_pull_and_confirm
PASS presentation_live_owner_waiters_serialize_and_succeed_after_release
Summary [10.592s] 13 tests run: 9 passed, 4 failed, 5028 skipped
```

This is causal RED for both Pull/Confirm cancellation/deadline wait behavior. Hook runs first; Watch and post-timeout no-effect assertions remain unmeasured on RED, as noted above. Parent execution JSON records list/build 15.178825 seconds, run 10.818728 seconds and exit 100, with process leak check exit 0 for `ea337674-33a7-4cd0-9b47-18f05aa0b118`. All seven independent controls passed. Parent is now applying production for GREEN; no GREEN or gate claim is made here yet.

## Parent GREEN evidence

Parent ran the production and corrected tests on owned validation commit `1bf42f80ffa2cb7ea2438fb05d12b7c26f02d57a`. I checked `git diff 1bf42f80 3b1113f0 -- src/service/delivery_presentation.rs tests/service.rs tests/service/presentation_owner_budget_controls.rs`; empty output confirms all three task-file blobs exactly match this task's final head. No source changes followed this verification.

Exact command (also retained as `run.argv` in `/private/tmp/ht-roast1-owner-green-3b1113f0/execution.json`):

```
nice cargo nextest run --locked --all-features --test service --no-fail-fast -E 'test(=presentation_owner_budget_controls::presentation_pull_cancelled_owner_waiter_returns_before_release_without_effects) | test(=presentation_owner_budget_controls::presentation_pull_expired_owner_waiter_returns_before_release_without_effects) | test(=presentation_owner_budget_controls::presentation_confirm_cancelled_owner_waiter_returns_before_release_without_effects) | test(=presentation_owner_budget_controls::presentation_confirm_expired_owner_waiter_returns_before_release_without_effects) | test(=presentation_owner_budget_controls::presentation_live_owner_waiters_serialize_and_succeed_after_release) | test(=presentation_owner_budget_controls::presentation_poisoned_owner_recovers_for_pull_and_confirm)'
```

Actual output from `/private/tmp/ht-roast1-owner-green-3b1113f0/run.log`:

```
PASS presentation_confirm_expired_owner_waiter_returns_before_release_without_effects
PASS presentation_pull_cancelled_owner_waiter_returns_before_release_without_effects
PASS presentation_confirm_cancelled_owner_waiter_returns_before_release_without_effects
PASS presentation_poisoned_owner_recovers_for_pull_and_confirm
PASS presentation_pull_expired_owner_waiter_returns_before_release_without_effects
PASS presentation_live_owner_waiters_serialize_and_succeed_after_release
Summary [1.209s] 6 tests run: 6 passed, 93 skipped
```

Every regression completed both Hook and Watch iterations, including budget-specific errors before release, equal no-effect snapshots and successful subsequent confirmation of the original token. Successful contention and poison controls passed. Execution JSON records list/build 11.074633 seconds, run 1.416863 seconds, exit 0; leak check exit 0 for `9d10c393-8aa2-499c-83e0-4b04eaa83aa5`. Parent combined gates are still pending; no merge/gate success claim is made here.

Source-only end: base `bdae610212ed26a095ca100bd1dc7aa1bd02fac2`; tests `aed4b720084511464ca7065b8ff1b8fca059d263` plus corrections `36c3988fe8fd0e46861170c182d1160afda00fc4`, `741984b9d6d9ca8459416a5ef82db1dd2a812591`; production/head `3b1113f0d1938ee065324f39d7590b250a4058a7`; measured RED four before-release timeouts and two passing controls; measured GREEN six passing tests, both Hook/Watch budget-specific errors before release with no effects; selector `--test service presentation_owner_budget_controls`; no implementer runtime execution.
