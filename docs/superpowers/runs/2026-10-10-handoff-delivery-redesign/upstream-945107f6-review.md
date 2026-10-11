# Upstream topology fixture composition review

Spec CLEAN
Quality CLEAN

Scope: read-only review of commit `945107f6acd8b05ade7b5e2d9f8b293282aa06f4` relative to its parent, and composition with integration tip `a7a6970243e8057a0fc3c896573c388da83b31a9`. This is a fixture correctness and composition review, not a full branch roast or runtime verification.

## Mandatory test change audit

`git show --stat` reports exactly two test files, 36 insertions and 18 deletions: `tests/handoff_topology_cli.rs` and `tests/integration/topology_activation.rs`, each 18 insertions and 9 deletions. Inspected both complete diff hunks. Each replaces one staging call with a loop over the unvisited suffix and adds a positive progress assertion. Seal, publish, and downstream behavioral assertions remain intact. No skips, ignored tests, weakened assertions, timeout increases, writer quantum increases, or overall budget increases.

## Store contract and fixture correctness

`src/store/seats.rs::stage_snapshot_targets` requires a nonempty slice of at most the admitted 16 targets, validates the supplied offset against the durable staged count, starts its 5 ms quantum after acquiring the writer, and commits at least one target before checking the quantum cutoff. Its returned `visited` is the exact committed prefix length. Each subsequent loop call therefore uses the correct cumulative offset and remaining suffix; the positive progress assertion prevents a silent nonadvancing loop. The same immutable `CallBudget` is reused throughout, retaining the original 5000 ms end-to-end deadline. Each call is a new bounded writer turn; the admission value is Copy and safely reused. Empty snapshots skip staging and proceed to seal rather than passing a forbidden empty slice.

These scripted fixture snapshots contain two initial panes and at most one added pane, so their remaining slices stay below the 16-target admission. The production reconcile path also batches larger snapshots and verifies additional returned metadata; this patch does not broaden these fixtures to arbitrary snapshots.

## Composition instruction

The current integration `handoff_topology_cli.rs` fixture already has a stronger compatible loop: it batches to 16 and asserts both positive/bounded visitation and exact cumulative staged count. Preserve that existing loop when resolving the overlapping upstream hunk; replacing it with the upstream loop would discard useful assertions without adding behavior. Apply the upstream continuation loop to `integration/topology_activation.rs`, whose current fixture still stages only once. This is a straightforward semantic composition, not a reason to overwrite the full file.

The remaining integration differences in these two files are the two invitation `confirmation: None` fields in `topology_activation.rs`; preserve them. Comparison of upstream parent with current integration shows no other differences in `handoff_topology_cli.rs`, so its Task4 body, quoting, frozen-payload and historical retry tests remain present unchanged. No runtime commands or Cargo checks were run; final combined-tip verification belongs to the parent readiness gate.
