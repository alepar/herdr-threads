# Final review, cluster 23 (tests-host), epic ht-ma6i

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 0755e609de0328e6d73584dd65eb2854e1fd7bf7, both verified with `git rev-parse --verify`. The caller checkout stayed at head with a clean status throughout; no commit or status change was observed.

## Scope of the cluster

One file, `tests/host/bootstrap_recovery.rs`, one hunk (slices/000114.diff). Commit 8945c48ca added `continuity_key: None` to the single `ResolveSeat` literal the fixture passes to `with_bootstrap_observation` (line 473 at head). The commit message describes it as part of a mechanical sweep of 182 struct literals. No assertion changed.

## Analysis

The value matches production. Both `with_bootstrap_observation` callers in `src/service/dispatch.rs` I read (the literals ending at lines 673 and 764) build their `ResolveSeat` with `continuity_key: None`, so the fixture still mirrors the path it stands in for.

The field is inert on this path. `with_bootstrap_observation` (`src/identity/repair.rs:816`) reads `request.target` and hands the request to `BootstrapAttachmentGuard::try_new` (`src/ports.rs:1604`), and no `.continuity_key` read exists in `src/ports.rs`. The key is read in `src/store/seats/resolution.rs` (the digest at line 119, and lines 448 and 476), and `repair.rs:1149` reads it from `ContinuityCheckIn`, which is a different request type. The recovery assertions and the `frozen` byte comparison of the topology request therefore keep their prior meaning, and the keyless resolve digest stays in its original domain.

Formatting: the added line and the closing brace sit at column zero. This fails no gate. `cargo fmt --all -- --check` exits 0 at head, because the file reaches the crate only through `include!` in `src/host/native.rs:7096`. Standalone `rustfmt --check` on the file also exits 0 with no output, which suggests rustfmt declines to touch that body, though I did not establish why. Neighbouring code in the same function is hand-shaped as well, so the result is cosmetic.

## Requirement baseline trace

The root spec has a `## Requirement baseline` section with rows B1 to B4. This cluster delivers no production behaviour for any of them. It is a test fixture kept compiling against the new field, and it neither lowers nor weakens a row. Tracing B1, B2 and B3 from a production entry point leaves this cluster and belongs to the seam reviewer. B4 is not engaged by this file.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- `bootstrap_recovery` was not executed by this review, and no pass at exact head was found: the fixture's compile and pass at 0755e609d rest on the caller's pending full-suite run (progress.md defers it to the caller under runTag graph-clone-identity-20261105T074145), so this verdict is static.
- This fixture only drives a keyless resolve through recovery's observation path, so TRUST-POLICY's rule that a standing key conflict refuses bootstrap creation is not exercised here; I did not establish coverage elsewhere, and a search for `bootstrap` beside `continuity_key` in `tests/`, `src/store/topology_handoff` and `src/store/seats` found no same-line match, so this is for the seam reviewer to trace.

## Deferred OK
- `tests/host/bootstrap_recovery.rs:480-481` has `continuity_key: None,` and `},` at column zero, left by the mechanical sweep; cosmetic only since `cargo fmt --all -- --check` exits 0 at head, and worth re-indenting by hand the next time the file is touched.
