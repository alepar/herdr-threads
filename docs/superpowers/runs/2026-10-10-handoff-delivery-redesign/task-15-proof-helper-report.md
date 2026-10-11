# Task 15 source-only proof helper

Status: PREPARED FOR INDEPENDENT SOURCE REVIEW; uncompiled and unexecuted. No Cargo command, production edit, test process, migration application, merge, tracker change or messaging action was performed. This is not a task-completion or RED/GREEN claim. D must apply only after review, execute the unique combined-suite selection at its actual source under its sole Cargo lease, and retain --nocapture diagnostics.

Owned workspace: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/js-epoch-helper-15`. Branch `helper-ht-0lo.8.4-schema-proofs`. Base `ee1e598c3ee96566ddc9b397185cdc97c8bf6218`; helper HEAD `e9ef2150fa7ba6883297052fa527f16060fca768`; tree `477dafa6502c3c00ed0d8e442ec6a12ec58ebca3`. Clean helper branch preserves the prior JS branch. Scope is exactly NEW `tests/store/invitation_content.rs` plus two-line existing `tests/combined.rs` registration. Registration follows the authoritative parent-directed brief supersession; no new private lib test module or accessor.

Full baseline-to-HEAD patch: `task-15-proof-helper.diff`; 6836 bytes; SHA256 `f43e3d0619590fc17fce6bdccc6e20aa13bd97556f5c868b79a893f4ce2c2ea1`. Commit includes truthful Outside-adapter trailer. New model-visible strings: 0 bytes; test-only diagnostic strings are not product delivery output.

## Three actual registered test names (not yet selected or run)

- `invitation_content::ordinary_partial_worst_escaped_witness_preserves_exact_bytes`
- `invitation_content::linked_sqlite_default_column_alter_has_row_independent_vm_work`
- `invitation_content::linked_sqlite_checked_column_validates_existing_rows_and_rolls_back`

Expected eventual selection is `--test combined invitation_content:: -- --nocapture`, with D's normal locked/all-features/nice command and actual compilation/count evidence. I did not run it. `rustfmt --check tests/store/invitation_content.rs` and `git diff --check` exited zero; these establish formatting only.

## Serializer evidence and limits

The test uses actual serde_json serialization and roundtrips the exact HashMap strings, including raw NUL bytes. Its one valid ASCII-graphic MessageId is 128 bytes alternating quote/backslash: both characters need two JSON bytes, so the key occupies 256. A body of 8191 NUL bytes requires six bytes each, totaling 49146. One single-entry object adds seven structural bytes (`{`, key quotes, colon, value quotes, `}`), giving an independently hand-derived expected length **49409**. The 49415 admission ceiling leaves six bytes for this valid partial body. The fixture with 8192 NUL bytes occupies exactly **49415**, showing why serialized-cap admission alone cannot reject raw body length >=8192. That semantic guard is separately necessary.

These numbers are analytical expectations embedded in unexecuted assertions, not observed serializer output yet. No unsupported Unicode-ID assumption, complete >8KiB ordinary display promise, private guard mirror or guard-execution claim is introduced. Private serialized-overflow/corrupt JSON/invalid-ID/cardinality/raw-length guards remain D author scope through real canonical operations in EXISTING tests/service/delivery_pull.rs. Existing canonical control-heavy chunk and 65536 cursor regressions remain D-owned and are not replaced by this witness.

## Linked SQLite measurement and limits

Cargo.toml uses exact rusqlite 0.39.0 with bundled SQLite; the same locked project dependency will execute these tests. Inspected local rusqlite 0.39.0 `src/statement.rs` exposes `Statement::get_status(StatementStatus::VmStep)` without extra features. No dependency or feature changes. No system Python SQLite, timing guesses, writable_schema or table rebuild.

For independently populated in-memory fixtures of 0, 64 and 4096 rows, the test prepares and executes the exact uncomplicated `ALTER TABLE invitations ADD COLUMN epoch INTEGER NOT NULL DEFAULT 0`, records actual positive VM counters, and asserts equality across row counts. It also verifies every retained row still has its topic and reads default epoch zero. Actual sqlite_version and each numeric counter print with --nocapture. Neither version nor numeric counter has been observed yet.

A CHECK-bearing ALTER is executed on separate identical fixtures and its OUTER VM counter is printed. The bundled SQLite source's ALTER finish path uses nested `pragma_quick_check` for CHECK validation; the outer StatementStatus counter does not provide a total nested-work guarantee. Accordingly no claim/assertion of CHECK ALTER total counter growth is made. Direct `PRAGMA quick_check(invitations)` on each checked fixture records a separate scan-work counter and asserts strict growth with row count; this is explicitly a separately executed scan characterization, not total ALTER cost. A final test places the only cross-column violation in the last of 64 existing rows, expects actual CHECK refusal, and verifies the failed ALTER left no epoch column and preserved all rows.

This is narrow evidence for the default-column assumption and existing-row CHECK validation. Full populated canonical migration39→40 atomic upgrade/rollback/FK/catalog/queryplan/index startup-cost coverage stays D author/schema reviewer scope. If the eventual linked execution fails, report the actual failure and investigate before claiming any proof.

## FIX ROUND 1 FINAL — authoritative current source provenance

Fresh independent source review found one Important compile defect: nonexistent `herdr_threads::protocol::MessageId` import. Corrected exactly that import to public `herdr_threads::protocol::ids::MessageId`; no assertion, fixture, registration, production or dependency change. Original e9ef provenance above is historical and superseded by this appendix. This source defect is NOT a behavioral RED result.

Final helper HEAD `c2553281a22bdcb293cd71df69f160868159ded8`; tree `a4f1f6acb288f5c4e142ee576528eedfa8534ef9`. Updated complete base-to-HEAD `task-15-proof-helper.diff`: 6841 bytes, SHA256 `2e46713f1c230e47a4abcf5462f624e24c39efcae185cf8aba539382eab1d1d9`. Separate e9ef-to-final `task-15-proof-helper-fix-round-1.diff`: 391 bytes, SHA256 `9ab34f5516b542849e0fdd3e29cf1dfaaa095b973f0976ae25809d7cfb365270`.

Re-inspected all imports/APIs/static fixtures once: `src/lib.rs` publicly exports protocol; protocol/mod.rs publicly exports ids; ids.rs provides public MessageId::parse/as_str with the ASCII-graphic 128-byte guard. rusqlite publicly exports Connection/StatementStatus; actual 0.39.0 sources expose open_in_memory, execute_batch, execute, prepare, query_row, Statement::execute/query_map/get_status and VmStep. serde_json is an existing pinned public dependency. SQL fixtures and hand-derived serializer expectations remain as previously reviewed. These source checks do not establish successful compilation.

`rustfmt --check tests/store/invitation_content.rs` and `git diff --check` exited zero after correction; no Cargo/build/test commands were run. All three registered tests remain uncompiled/unexecuted. Expected new model-visible strings remain 0 bytes. Fix round 1/5: entered with one Important finding, corrected one, awaiting fresh scoped re-review. D later owns actual unique three-test execution, numeric linked-SQLite diagnostics and behavior evidence. Stop before application/merge.
