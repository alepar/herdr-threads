# Publication scalar CHECK proof helper: fix round 1

Spec CLEAN

Quality CLEAN

Source-only verdict for the complete revised 10,462-byte block `task-15-publication-check-proof-block.rs`, SHA-256 `23038740284131385450df6c739fbc386c230cbbf606466804fdf648f50b8ff5`. Read the full cumulative block, full refreshed report, and complete 8,936-byte delta SHA-256 `ea13f917f47f5daf7f3739303b591c032888aaf5a7326c688fff6acd69aeb833`. No Cargo, tests, live source edits, Git mutations, or system-SQLite measurements were performed. This approves the reviewed source correction, not successful execution or whole D.

## Original finding: ADDRESSED

The original Important finding in `task-15-publication-check-proof-review.md` concerned mixed consumer comparison paths within the exact-affine samples. The revision uses two independent homogeneous series, hook and watch, each containing 1/32/128 rows. Fixture and matched control receive the same explicit consumer. Work vectors and slope assertions are allocated inside the consumer loop, so neither data nor deltas cross the two paths. Diagnostics identify the consumer. Both supported kinds remain covered rather than dropping one to obtain a simpler measurement.

I checked the other actual V40 CHECK inputs, not only consumer. Within each series every constrained integer, mask, version/selection string, JSON envelope, and render key is identical across retained rows. The four nullable cursor/token columns are always NULL. All JSON arrays have the same empty shape and all object inputs have the same content. The mask subset, length, range, IN, JSON type/length, and NULL-short-circuit conditions therefore take the same branch per row. All four added values are initially zero. Only `seat_id` varies: it is non-NULL fixed-width `seat-0000` through `seat-0127`, with no content-dependent V40 CHECK. Foreign keys resolve to private existing parents. Unique token index entries remain absent because tokens are NULL. Implicit row IDs and B-tree contents vary with population, but this source does not translate their C implementation work into progress ticks or claim all CPU work is affine.

There is no remaining source-level mixture of constraint paths underlying the selected per-series slope assertion. Exact affine behavior remains an actual linked-runtime assertion to execute, not a proven numeric result. If SQLite's execution produces another nonaffine component, D must investigate that result; this source verdict does not authorize changing a failed assertion into a pass or claiming measurements in advance. No VM coefficient, row-visit-to-tick equality, or `ticks=4*S` is asserted here.

## Preservation and quality

All original checks remain: real full consumer-table/index DDL, exact four ALTERs, CHECK-only matched controls, positive active outer counters, checked work exceeding control work, constant unchecked progress across sizes, positive checked deltas, complete old-column snapshots after ALTER and negative refusal, zero defaults for every row, and independent negative scalar rejection. Each series repeats all four columns and all sizes. No old assertion was weakened or removed. The report now explicitly calls the fixture SQL-schema-valid and excludes canonical runtime-decoder/full-upgrade proof.

The raw FFI and RAII code is unchanged from the complete original audit: stable Box<Cell> context, synchronous private connection, saturating callback without SQLite reentry or unwinding, handler installed after prepare, and guard unregistration before counter drop on success/error/unwind. Existing module imports, local imports, relative include paths, linked rusqlite API use, unique test name, and append-only registration scope remain consistent. No new dependency, binary, model-visible string, or production authority is introduced.

## Provenance and execution gate

Verified current target and prerequisites still match the report:

- D `tests/store/invitation_content.rs`: `1a5bd5d0580e8ffd611a806fda9c07b214c992529c8449e3ea126912bf9c1c0f`.
- D `migrations/0040_invitation_presentation.sql`: `83e7348a425907dc7ad55021960812e933f4126cdaacefbb7f90d28382118326`.
- D `migrations/0041_presentation_relevance.sql`: `0142aa5a67146fbc5bd77380609a86b7f892b32704fb4a2f32f36ba35f3523c7`.
- Original archived block remains exactly 9,865 bytes, SHA-256 `ab28a17aca691b26dd77748e72a6ba9635f6998009366036959287147cfac459`; original archived report hash matches `fb94c50b342914b67706cc30164416e24246c13c648a5ca3f9388eed8294aac0`.

Assembly still requires positive D quiescent ACK and exact target baseline verification. D must compile and execute the actual locked/all-features selected combined test, retain the nonzero selection count and raw measurements for both consumers/all four columns/all three sizes, and complete remaining whole migration/interface/behavior gates. No executed, green, complete, or merge-ready claim is made by this review.
