# Final review, cluster 22 (src-test-support), epic ht-ma6i

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 1b058307490f29ece8bb4e3fe7a862f8356d992f (both verified with `git rev-parse --verify`). Caller checkout observed at the head SHA with a clean status at the start and end of the review.

## What the cluster changes

One slice, `slices/000118.diff`: six added lines in `src/test_support/mod.rs`, from commit 06329d8a3. It adds `pub fn operator_actor() -> OperatorActor`, which builds `OperatorActor::from_peer(PeerIdentity::from_kernel(501), 501)`. The commit's `Outside-adapter` trailer names the path.

## Trust boundary

`OperatorActor` is an authority type whose constructor and `PeerIdentity::from_kernel` are `pub(crate)`. A public function returning one would be a forgery path out of the crate if it reached a production build. It does not:

- `src/lib.rs` declares `pub mod test_support` under `#[cfg(any(test, feature = "test-support"))]`, so the function is absent from ordinary builds.
- `PeerIdentity::from_kernel` carries the same gate (`src/protocol/authority.rs`), so the helper cannot compile without it, and the feature is already the boundary for the other public fixtures in this module.
- `from_peer` is unchanged: it is the same owner-uid equality check production dispatch uses. The helper passes matching uids, so the `expect` cannot fire.

TRUST-POLICY is not affected by this cluster: no provenance, invariant or accepted limit is added.

## Callers and feature gating

Three call sites, all finalizing a retirement through `seats::mutate_operator`:

- `tests/store/fact_convergence_seat.rs` and `tests/sync/facts_seat_reversible.rs`, modules of `tests/combined.rs`, whose target has `required-features = ["test-support"]`.
- `tests/store/reversible_facts.rs`, compiled through `#[cfg(all(test, feature = "test-support"))]` in `src/store/seats/publication.rs`.

No default-feature target reaches the helper. These calls take the actor directly and do not compare uid 501 against a daemon owner, so the fixed uid is harmless. It matches the literal the rest of the suite uses.

## Duplication

The same one-line construction exists as local helpers in `tests/service/sync_admin.rs`, `tests/store/control.rs`, `tests/store/cooperative_checkin.rs` and others, all predating this helper or local to a suite inside the crate. The new helper exists because an out-of-crate suite cannot call the `pub(crate)` constructor, so it is the minimum for the need; nothing here calls for a sweep. The doc comment says "outside the crate" while `reversible_facts.rs` is an in-crate user; this is cosmetic.

## Requirement baseline

Rows B1 to B4 of the root spec describe production behaviour; this cluster holds a test fixture only and delivers no row on its own. It cannot weaken B1 to B4 because it is compiled out of production. B2's finalize and undo paths, which these tests exercise, leave this cluster and are the seam reviewer's and the store clusters' to trace.

## Verification performed

Static review of the diff, the feature gates, the authority type and each call site at the head SHA. I did not run the build or tests for this cluster: the helper's compile and behaviour are exercised by the callers' suites, which the caller runs once on the integration tree.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- none

## Deferred OK
- none
