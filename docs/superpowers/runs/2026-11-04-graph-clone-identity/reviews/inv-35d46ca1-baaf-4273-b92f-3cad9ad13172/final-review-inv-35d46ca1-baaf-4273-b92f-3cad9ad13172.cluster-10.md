# Final review, cluster 10 (scripts), epic ht-ma6i

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 1b058307490f29ece8bb4e3fe7a862f8356d992f. Both verified with `git rev-parse --verify`. The review source was bound before this file was written.

## Reviewed input

The cluster is one file, `scripts/wire-compat-baseline.txt` (slice 000057), with six added lines and nothing removed. It is the only change under `scripts/` in the range, so `scripts/check-wire-compat` itself is unchanged.

Added entries:
- `field SeatRetiredReversiblePayload.{reason,retired_at,seat}`
- `field SeatUnretiredPayload.{retired_at,seat,unretired_at}`

## Checks against the baseline policy

- New types only. The file header and AGENTS.md let the baseline grow only by new types, in the commit that introduces them. Neither type exists at the base. Commit 8945c48ca adds both structs to `src/sync/facts/seat.rs` and the six baseline lines together, and it is the only commit in the range that touches the file.
- Entries match the tree. At head the two structs have exactly these fields, with no extras and no stale lines. `src/sync/facts/seat.rs` is on the checker's scanned list, so the entries are enforced.
- No existing type grew. The diff of the scanned wire files in the range touches only `src/sync/facts/seat.rs`, and only by adding these types and their `FactKind` impls. `src/sync/frame.rs` is untouched, so there is no new gated variant and no `// cap:` marker is needed. `src/sync/facts/mod.rs` also changes, to register the kinds; it is not a scanned file and adds no wire fields.
- Checker run at head, against the base and against the default `main` (which equals the base): `check-wire-compat: ok — 0 violations, 0 stale, 456 fields, 24 variants, 274 baseline entries`. The R4 delta lists exactly the six lines. The `--base` run exited 0. The default run's exit status was masked by a pipe to `tail`, so its evidence is the summary line.
- Self-test: `python3 -m unittest scripts/tests/test_check_wire_compat.py` ran 36 tests, all passing, including `RealTreeTests.test_current_tree_passes` against this tree.
- Merge safety: the merge-base with `main` is the base, and `main` has not touched the baseline since, so there is no sort or merge conflict to carry.

## Spec conformance

Spec 4.3 requires `seat.retired_reversible{seat, reason, retired_at}` and `seat.unretired{seat, retired_at, unretired_at}`. The baseline lines match those shapes field for field.

Spec 4.3 also rejects a per-link capability, because the fact registry is the gate. An old node parks an unknown kind as `parked_unknown`. That is consistent with no `// cap:` marker being present here. The R3 marker rule applies only to gated `PeerFrame` and `WireItem` variants, and these are fact payloads.

Spec 4.3 calls both kinds held kinds. I opened `src/store/materialize.rs` to check this, and `HELD_KINDS` lists `seat.retired_reversible` and `seat.unretired`.

The required payload fields carry no `#[serde(default)]`, which fits the policy: R2 exempts baselined fields, and these are new types in their introducing commit. They mirror the existing `SeatRetiredPayload`.

## Requirement baseline trace

The spec has a `## Requirement baseline` section with rows B1 to B4 and no human scope changes.

- B1 (continuity key): this cluster adds no key wire shape. The key is local by spec 4.3. The trace leaves this cluster and is the seam reviewer's.
- B2 (pane close retires, undo reverses): these baseline lines only permit the two cross-node payloads to exist. The cluster delivers no behaviour here. Tracing from the production entry point through the service, store and fact materialization leaves this cluster and is the seam reviewer's.
- B3 (agent exit keeps the seat): the spec says this needs no threads change, and nothing in this cluster affects it.
- B4 (Herdr unchanged): this cluster changes no Herdr file or interface.

No row is contradicted by this cluster, and none can be judged undelivered from it.

## Observed state

The caller checkout stayed on 1b058307490f29ece8bb4e3fe7a862f8356d992f with a clean `git status` throughout. No caller files were edited, and no review worktree was needed. One of my probes, passing `--base-file` through process substitution, was refused by the checker as a usage error (exit 2). It tested nothing and was replaced by the `--base` run above.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- none

## Deferred OK
- none
