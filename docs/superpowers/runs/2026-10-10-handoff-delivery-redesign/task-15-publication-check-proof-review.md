# Publication scalar CHECK proof helper review

Spec Issues

Quality NeedsFixes

Reviewed the complete 9,865-byte block `task-15-publication-check-proof-block.rs`, SHA-256 `ab28a17aca691b26dd77748e72a6ba9635f6998009366036959287147cfac459`, and its complete source-only report. This is a narrow test-helper review before assembly. No Cargo, tests, source/Git/Herdr mutations, or system-SQLite substitute measurements were performed. Only this review artifact was written.

## Important finding

**1. The exact affine slope assertion mixes different per-row execution paths.** `task-15-publication-check-proof-block.rs:96,146,207` alternates hook/watch rows, measures counts 1/32/128, then requires `first_delta * 96 == second_delta * 31`. The one-row sample contains only hook; the other two samples contain equal numbers of hook and watch. Actual V40 DDL includes `CHECK(consumer IN ('hook','watch'))`. In the project's cached linked SQLite source, `sqlite3FindInIndex` selects comparison-based IN evaluation for a two-element literal list (`libsqlite3-sys-0.37.0/sqlite3/sqlite3.c:114849`), the IN expression requests that optimization at line 115505, and its comparison loop at line 115561 emits an early equality jump to `labelOk`. A hook match skips the later comparison that a watch match executes. The progress counter intentionally includes this nested validation work.

Thus constant work per retained row is not established for the chosen mixed population. A per-watch additional term changes the measured series by `floor(S/2)`; its deltas over 31 and 96 added rows are 16 and 48, whose cross-products are 1536 and 1488. This is source evidence of a data-dependent path, not an invented numeric VM measurement or a claimed executed failure. Exact affine equality can reject a correct migration. The report's claim that this fixture proves that equality is not yet supported.

Use the same supported consumer kind throughout each measured 1/32/128 series, and exercise the other kind in a separate homogeneous series or a separate preservation witness. Alternatively use a meaningful bounded-growth assertion whose bounds are actually justified for the heterogeneous fixture. Do not simply delete the positive-growth/control assertions or explain away a failing exact identity. Refresh the report and block hash, obtain a scoped rereview, then have D compile and execute the actual selected combined test. This finding blocks source approval of the current block.

## Spec compliance: remaining inspected scope

The exact target test file and both migrations match every reported full-file hash. The actual V40 consumer-table-plus-two-index slice is 5,397 bytes with SHA-256 `845102cb2b834812c5811b5763d0191b704cc09e28b1ddeb0514b9f7a1c0d30f`; the four V41 ALTER lines are 607 bytes with SHA-256 `a215c5c7a56cf13a912fcd18e2030df3ae6beeb60203f6350d769d7bb9b71bac`. The parser selects those exact current statements in the correct order. Splitting each at ` CHECK(` and restoring the semicolon removes only its CHECK suffix. The full DDL includes both unique token indexes and preserves its real old constraints; minimal private parent tables satisfy the referenced keys with foreign keys enabled. It does not execute the complete canonical 39-to-41 upgrade or later migration projections.

The fixture fills all old columns, snapshots all old values ordered by seat/consumer, and checks exact value equality after each ALTER and each failed negative write. It checks every new scalar defaults to zero for every row, rejects a negative value, and remains zero after rejection. The nonzero old integers satisfy the current SQL checks, including rotation and confirmed-mask subset; JSON envelopes and nullable token/cursor slots satisfy the actual DDL. This is a SQL-schema-valid full-shape witness, not a canonical runtime-decoder witness: for example `{}` control_state and generic `retained` version/selection strings are not proof of real application-produced state. The report's bounded statement that this is not the full populated canonical migration must remain explicit; whole D's canonical upgrade evidence is still required.

There is no private production API, new binary, dependency/feature, global configuration mutation, or model-visible product string. The eventual include paths are correct relative to `tests/store/invitation_content.rs`; this standalone artifact itself is not represented as a compile target. Existing `Connection` and `StatementStatus` imports are present. `params_from_iter`, `Value`, C types, and Cell are block-local imports. The selected function name does not collide with the existing three tests. No old test assertion is removed or weakened by the proposed append.

## Code quality: unsafe lifetime and measurement audit

The raw connection-local progress handler has a sound lifetime at this source. Its context points to stable `Box<Cell<u64>>` storage; the callback performs only Cell access and saturating arithmetic, returns zero, does not reenter SQLite, allocate, or unwind. All operations occur synchronously on a private, unshared in-memory connection. `ProgressGuard` unregisters before count storage is dropped both on explicit success and on error/unwind: the guard is declared after the counter. The statement can finalize later because the handler is already removed. No fallible operation intervenes between successful handler installation and construction of the guard. Unregistration's handle call operates on the still-live borrowed connection. No other handler is overwritten on these newly created private connections.

The project's cached rusqlite 0.39 API exposes `pub use libsqlite3_sys as ffi`, unsafe `Connection::handle`, and `Statement::get_status(StatementStatus) -> i32`; the local type/signature use appears compile-ready. Compilation is nevertheless unverified and belongs to D. The raw FFI path needs no progress-handler feature. Installing after prepare deliberately excludes initial preparation work; nested execution is observed through the same linked library, while outer VmStep is reported separately. Measurements do not cover all C implementation work, memory/page costs, or prove row visits equal VM ticks. The report correctly avoids `ticks=4*S`, CHECK-constant-work, or outer-counter-only claims.

The positive checked-versus-unchecked comparison, active outer counters, constant matched-control work across row counts, retained-value checks, and negative writes are meaningful checks worth preserving during the fixture correction. The exact-affine issue above is the sole blocking finding from this complete narrow review.

## Provenance and remaining gates

Active D workspace inspected read-only: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.3.2`.

- Target `tests/store/invitation_content.rs`: `1a5bd5d0580e8ffd611a806fda9c07b214c992529c8449e3ea126912bf9c1c0f`.
- `migrations/0040_invitation_presentation.sql`: `83e7348a425907dc7ad55021960812e933f4126cdaacefbb7f90d28382118326`.
- `migrations/0041_presentation_relevance.sql`: `0142aa5a67146fbc5bd77380609a86b7f892b32704fb4a2f32f36ba35f3523c7`.

No assembly approval is implied. After correction, require fresh exact block/report review, positive D quiescent ACK and target-file baseline check, then actual locked/all-features execution with nonzero selected-test count and raw measurements retained. Whole D migration, interface, behavior, and package review remain mandatory. No green, compiled, complete, or ready-for-merge claim is made.
