# Final review: cluster 28 (tests-host), epic ht-ma6i

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 1b058307490f29ece8bb4e3fe7a862f8356d992f. Both verified with `git rev-parse --verify`. The caller checkout stayed at head and clean throughout. Source binding succeeded.

## Cluster contents

One file, `tests/host/bootstrap_recovery.rs`, one hunk (slice 000152), from commit 8945c48ca (the mechanical sweep that gave `ResolveSeat`, `ContinuityCheckIn`, `CheckIn` and `SeatInspection` literals their `None` fields). The hunk adds `continuity_key: None,` to the `ResolveSeat` literal the test passes to `with_bootstrap_observation`.

## Analysis

- Shape: at head `ResolveSeat` has six fields (`target`, `host`, `operation`, `locator_hint`, `tmux_hint`, `continuity_key`), and the literal now names all six.
- Fidelity to production: the four production callers of `with_bootstrap_observation` (`src/service/dispatch.rs` lines 667, 758, 1235, 1311) build the same literal with `continuity_key: None`, so the test still mirrors what production sends. Spec 4.2 has `ResolveSeat` present a key only for the caller's own pane; automatic bootstrap resolves a created pane on the plan's behalf, so `None` is correct and no keyed variant of this path exists to cover.
- Test strength: every assertion in the test is untouched, and `BootstrapAttachmentGuard::try_new` passes the request to `OrdinaryResolutionGuard::try_new` as before. No assertion was weakened and no case was removed.
- Formatting: the added line and the closing `},` sit at column zero. The file is pulled in by `include!` inside `native::tests` (`src/host/native.rs:7096`), so `cargo fmt --all -- --check` in CI does not reach it. Standalone `rustfmt --edition 2024 --check` also reports no difference at base or at head, so this is cosmetic and cannot fail CI. The same column-zero form appears in `tests/service/graph_d2.rs` and `tests/service/graph_send.rs`, which belong to other clusters.

## Requirement baseline trace

The spec has a `## Requirement baseline` section (B1 to B4, no human scope changes). This cluster holds one keyless test literal and no production path, so it neither delivers nor lowers any row. B1 (continuity key), B2 (pane close retire and undo) and B3 (agent exit keeps the seat) are traced by the reviewers of the clusters and seams that hold those paths. B4 (Herdr unchanged) is not touched here: the test uses its own socket server inside `native::tests`.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- This review did not execute `bootstrap_recovery`; correctness of the hunk rests on reading the struct definition and the production literals, and the full-suite run at head is recorded in the ledger as deferred to the caller.

## Deferred OK
- `tests/host/bootstrap_recovery.rs:479-480`: `continuity_key: None,` and `},` are at column zero, left over from the mechanical sweep; cosmetic, and rustfmt does not flag it because the file is reached only through `include!`, so re-indent whenever the file is next edited.
