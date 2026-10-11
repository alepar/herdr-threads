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
