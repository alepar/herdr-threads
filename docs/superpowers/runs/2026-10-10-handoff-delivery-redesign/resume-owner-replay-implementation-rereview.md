# Owner replay ORI-1 implementation rereview

**Spec PASS; Quality PASS for the scoped source correction. ORI-1 addressed.** No remaining source finding was established in this rereview. These are static review verdicts, not runtime PASS claims.

Reviewed immutable tests-only commit `77fc3d182d2a69eba533527b4218b9c4ab1ec411` and production fix `b51959aa34cdeabbff78370b1a91da786bf43c69`, against the prior reviewed owner replay implementation `11e0b1e133277c46189fcbe60726dceeb2bdf247`. The production delta from that baseline is exactly six added lines in `src/store/invitation_content.rs`; the other delta is test coverage. Future WR9 test edits are outside this review. The previous report's general source mapping remains applicable.

## ORI-1 disposition

`src/store/invitation_content.rs:2573–2575` now finalizes the recovery bookkeeping disposition after computing `consumed`, the actual logical offered/truncated IDs:

```rust
if recovery_active && !owner_mode && consumed.is_empty() {
    owner_progress.source = crate::ports::OrdinaryPresentationSource::RecoveryControlOnly;
}
```

This fixes the identified distinction between selecting a query source and allocating a logical offer. When a previously active owner worklist is fully pruned before ordinary source selection, an empty logical allocation preserves recovery context even if Normal or LogicalDue was queried. The unchanged service finalizer therefore leaves the value found under its post-store ledger lock untouched. It does not restore a stale pre-store outlook snapshot.

The condition also preserves the intended boundaries:

- A real Normal or LogicalDue allocation has a nonempty logical `consumed` set and keeps its existing bookkeeping effects, including truncated allocations.
- Physical owner replay already has `owner_mode=true` and retains its existing OwnerReplay disposition, including empty/missing-key attempts and final exhaustion.
- Outbox replay still returns through its earlier recovery-aware path; the fix does not replace or retire the token.
- Calls without active recovery retain their previous behavior.

No source-mode selection, offer stamp, logical due continuation, query budget, normal cursor/carry/partial state, replay progress authority, protocol shape or model-visible text is changed by these six lines.

## Causal test assessment

Line references below are to the immutable `77fc3d18` tree.

`tests/service/delivery_pull.rs:10894` exercises the exact reported failure. A real old-outbox replay initializes outstanding owner debt, and its token is confirmed normally. Supported recording changes every captured stamp. The test explicitly checks both the original debt size and the subsequent `active=true`/empty remaining snapshot (`10945–10953`, `10978–10985`), so it cannot accidentally test an initially empty lease instead. A new passive invitation guarantees an output Page with no logical IDs. The matrix covers positive/zero capacity, both Boolean outlook values, and a barrier that flips the current value after the service snapshot. It compares all four requested outlook observations and unchanged stamps (`11009–11035`). On the original production branch, cases expecting true reach the unconditional false assignment; the fix prevents that assignment. This is a static causal assessment, not an observed RED/GREEN result.

The companion test at `11045` allocates a genuinely fresh ordinary ID after old recovery stamps become obsolete. It checks the new count-one offer, unchanged old count-two history and the normal `page.more` bookkeeping rule. This guards the exception for independent legitimate logical allocation.

The inherited new deferred-control test at `10768` now fills a real invitation page, verifies that Attention was actually deferred, replays/confirms the old token, then asserts a separate one-item Attention page with zero ordinary work and unchanged outlook before ordinary replay continues. This supplies the control-only coverage absent from the first reviewed checkpoint.

The remaining tests-only refinements are consistent with their intended evidence:

- The notice regression records and repeatedly checks the exact distinct pending confirming invitation; it compares replayed Notice contents, immutable outbox fields and monotone masks, permitting only canonically obsolete Attention retirement.
- Historical fallback setup now creates its helper rows before `pull_seat`, preserving the later terminal/nonaddressed/unpublished test conditions while addressing the fixture ordering collision.
- The replacement thread observes the same owner permit and its contention, with completion not yet reported before release. The comment accurately limits this evidence: the test does not instrument the internal `Mutex::lock` call. The production permit boundary remains unchanged.

## Validation boundary

The parent owns meaningful RED execution on `77fc3d18`, GREEN execution on `b51959aa`, focused regressions and required checks. No runtime result, compilation result, activation result or merge readiness is inferred here. No Cargo, tests, builds, Herdr/plugin commands, source changes, Git mutations, tracker writes or subagents were used. Only this rereview report was written.
