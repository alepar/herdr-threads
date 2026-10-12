# Task 15 proof helper independent source review

Reviewed HEAD: `e9ef2150fa7ba6883297052fa527f16060fca768` (tree `477dafa6502c3c00ed0d8e442ec6a12ec58ebca3`), against `ee1e598c3ee96566ddc9b397185cdc97c8bf6218`.

Spec Issues

Quality NeedsFixes

## Important finding

`tests/store/invitation_content.rs:7` imports `herdr_threads::protocol::MessageId`, which does not exist at this reviewed HEAD. `src/protocol/mod.rs` declares `pub mod ids` and contains no MessageId re-export; the public definition is `src/protocol/ids.rs:39`. Change the import to `herdr_threads::protocol::ids::MessageId`. The existing import prevents the registered combined test target from compiling. This is established directly from source, without running Cargo.

## Exact scope and provenance

Read the complete brief, including its authoritative combined-registration supersession, report, and complete patch. Independently verified the artifact is byte-identical to the immutable baseline-to-HEAD Git diff: 6836 bytes, SHA256 `f43e3d0619590fc17fce6bdccc6e20aa13bd97556f5c868b79a893f4ce2c2ea1`. The clean workspace HEAD and truthful Outside-adapter trailer match the report. Only the new 139-line `tests/store/invitation_content.rs` and two registration lines in existing `tests/combined.rs` change. All existing registrations remain. No production code, private accessor, private lib registration, new test binary, process-global state, test child, model-visible output, or dependency changes are introduced.

## Assertion and API assessment

The serializer witness uses actual serde_json, a valid 128-byte ASCII-graphic ID alternating quote and backslash, and 8191 raw NUL bytes. The independently derived size is seven structural bytes plus 256 escaped key bytes plus 49146 escaped body bytes, totaling 49409. Exact roundtrip strings, cardinality, and raw NUL bytes are asserted. The separate 8192-byte witness totals 49415 and demonstrates why serialized-cap admission cannot replace the independent raw-length guard. These are meaningful serializer characterizations, not private implementation mirrors or claims that private guards were exercised. Private invalid-ID, cardinality, corruption, raw-length and serialized-overflow guards remain D-owned through actual canonical operations.

Inspected the actual pinned rusqlite 0.39.0 source: `Statement::get_status(&self, StatementStatus) -> i32` and `StatementStatus::VmStep` are available without an additional feature. Cargo.toml selects its bundled SQLite. The helper uses those linked-project APIs, not system SQLite or elapsed-time estimates. No other source-level API mismatch was found.

The default-column test executes ALTER on independently populated 0-, 64-, and 4096-row fixtures, requires an active positive counter, compares its work across row counts, and verifies retained topics and epoch defaults. CHECK-bearing ALTER uses separate fixtures. Bundled SQLite's ALTER path invokes nested `pragma_quick_check`; comments and report correctly avoid treating the outer statement counter as total nested work. The separately executed direct quick_check growth assertions are explicitly a scan-work proxy. They do not certify total CHECK ALTER cost. The last-row cross-column CHECK violation supplies a behavioral existing-row validation witness and verifies failed ALTER removes the new column and retains all rows.

## Execution boundary

This is source-only review. No Cargo, tests, migration, source/Git mutations, tracker, Herdr, or child-agent operations were performed. Formatting and Git whitespace evidence in the helper report do not establish compilation or passing tests. After the import correction and refreshed immutable review evidence, D must compile and run the three actually registered combined-suite tests at its composed source under the sole Cargo lease, retaining actual counts and linked SQLite diagnostics before claiming proof. Populated canonical migration 39-to-40, rollback/FK/catalog/index costs, D whole-task review and composed schema/interface review remain mandatory and outside this helper verdict.
