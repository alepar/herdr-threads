# V090 schema and carry failure triage

Scope: the three assigned failures from `/private/tmp/ht-C-v090-final-composed-657/nextest-run.log`, against composed source `9a573c3f2a5420ff0c3fee07a3e836d189e88a83`, tree `5158b885c8988b1c3ebf5b96047751a6e4949212`. This report records source/log evidence and proposals; it does not certify a passing rerun. No Cargo, runtime test, Git, Herdr, or source/DDL mutation was used for triage. The parent subsequently authorized the two test-file repairs described below.

The actual focused run retained 651 executions: 581 passed and 70 failed (log line1954). `execution.json` separately records all six mandatory recovery cases as `ok`, and its overall exit is1. Preserve this failed execution history and the mandatory PASS evidence; none of these repairs is a waiver or a reason to remove selection rows.

## 1. Historical binding guards versus the latest full catalog

Recorded failure: `v28_upgrade_migrates_bindings_to_session_keyed_v32`, log lines1774–1777, at `tests/store/schema.rs:6239`. The left catalog has six historical archival triggers; the right catalog additionally has `presentation_fence_occupant_bindings_delete`, `presentation_fence_occupant_bindings_insert`, and `presentation_fence_occupant_bindings_update`.

Source cause: `binding_archival_guards` at6253 actually selects every trigger on `occupant_bindings`. `assert_v33_guards` at6231 assumes unchanged names and zips the before/after catalogs. Its caller initializes to `schema::LATEST_VERSION`45, rather than stopping at33 (6287–6306). Actual migration45 creates the three new guards at `migrations/0045_presentation_relevance.sql:655,664,673`. Production `verify_presentation_relevance_schema` independently reconstructs the actual migration45 objects and checks exact SQL (schema.rs4768–4836).

Proposed test-only repair: preserve the full trigger query; compare the complete after catalog against a fresh latest database (names and exact SQL, no wildcard exclusions), then look up every historical trigger by its name. Keep both owned V33 fences changed and containing `WHEN EXISTS`, and keep each of the other four historical trigger SQL strings byte-identical. Return the full after catalog so the caller's fresh-catalog comparison and missing-trigger reopen checks remain active. Update the helper comment to distinguish existing guards from newly added guards. This also repairs its main24/main25 callers without hiding the new presentation triggers.

## 2. Automatic-graph rollback expects the wrong starting version

Recorded failure: `released_main31_populated_automatic_graph_physically_upgrades_and_reopens`, log lines1835–1838, at schema.rs7594: actual42, expected38.

Source cause: the rollback copy intentionally applies sorted migrations32–42 (`skip(31).take(11)` at7563), explicitly sets `user_version`42 at7567, and inserts an orphan automatic-bootstrap key with foreign keys disabled. The rollback assertion still expects38 at7597. Actual `migrate_automatic_v42_to_v43` (src/store/schema.rs4655–4698) rebuilds the automatic graph, checks all foreign keys before setting43, rolls back on error, and restores the original foreign-keys setting. Migration38 handles the separate nonautomatic bootstrap graph; its version is unrelated to this failure fixture.

Proposed test-only repair: capture the version immediately before the failing initialize call, assert that setup is42, and require exactly that version afterward. Retain the StoreCorrupt error, foreign-keys setting restoration, complete catalog including rootpages, all retained graph row bytes, orphan repair, successful reopen, latest version, and clean FK check. Do not change the actual released31 fixture or any migration1–35 (or later DDL).

## 3. The 33-item negative oracle now fails at canonical SQL admission

Recorded failure: `carry_quantum_accounts_all_thirty_two_keys_and_refuses_a_thirty_third`, log lines1926–1927, at invitation_content.rs979. The fixture's invalid UPDATE unwrap fails with SQLite extended code275, before `presentation_pull` executes.

Source cause: actual migration44 `ordinary_pending` CHECK at68 requires <=155731 blob bytes, valid JSON object, `items` array <=32, and `prefix` array <=8. The test creates33 items then assumes that UPDATE succeeds. Independent reader guards remain: `load_raw` checks155731 before parsing (src/store/invitation_content.rs315–343), and `ordinary_carry` rejects >32 items and >8 prefixes at1456–1488. The existing successful32-item half establishes32 emitted/hydrated, <=32 seeks, >32 and <=579 key probes, and zero fresh ordinary/due-key work; keep all these assertions.

Proposed test-only repair: keep both boundaries in the same test. First verify that33-item JSON is within155731 bytes, capture private/cursor/receipt state, require the normal UPDATE to fail specifically with CHECK constraint extended code275, and require unchanged state. Then enable `ignore_check_constraints` only on this isolated fixture connection to seed the33-item corrupt private row; restore checks immediately before the production pull, including before any input helper call. Capture the corrupt state, require specifically StoreCorrupt, retain the original exact invalid-JSON preservation assertion, and compare all consumer/proof/fence/cursor/receipt rows for rollback. Production pull runs in the actual budgeted decision transaction (src/store/mod.rs1296–1311), so proof writes performed before ordinary-carry rejection must also roll back. Existing isolated corruption tests use this ON/seed/OFF pattern (tests/store/attention.rs3681–3689,3844–3848). Never disable checks in production, alter DDL, loosen32/155731, or replace the private-reader oracle with SQL refusal alone.

## Parent-owned verification

After authorized edits and scoped formatting, the parent must rerun the three exact tests plus affected `assert_v33_guards` callers (`adapter_migration_main24_to26_preserves_archival_guards_and_intent_history`, `absorption_main24_and25_to26_preserves_diagnostics_and_matches_fresh_catalog`), then the required clippy/harness-neutral checks and refreshed composed selection. Relevant mutation checks: omit or alter a V45 binding trigger; lose/change an unchanged archival trigger; fail V43 rollback or FK-setting restoration; weaken SQL carry admission; accept corrupt33-item carry; retain tentative proof progress after failed pull. Retain original failed evidence and all critical/mandatory PASS cases when refreshing qualification. No passing status is implied by these source-only proposals.

## Authorized edit handoff

The parent authorized the exact tests-only repair after receiving the source findings. Applied changes are limited to `tests/store/schema.rs` and `tests/store/invitation_content.rs` in the composed worktree. The proposals above were implemented; production Rust and every migration remain untouched by this agent. Snapshot rows use explicit table keys, including the WITHOUT ROWID binding-cursor table. Corruption seeding saves the UPDATE result, restores CHECK enforcement, then unwraps the result, so an unsuccessful seed cannot leave checks intentionally disabled.

`rustfmt --edition 2024 --config skip_children=true tests/store/schema.rs tests/store/invitation_content.rs` and the matching `rustfmt --check` both exited0. This is formatting evidence only. No compilation, clippy, harness-neutral, or runtime test claim is made. No child/helper process was started or left running; the agent is quiescent.

Final SHA256 handoff:

| Path | SHA256 |
| --- | --- |
| `tests/store/schema.rs` | `f74544bce11d0f163cd6e3e79ed661416fd086ce266aa630df52e2a7b2152b74` |
| `tests/store/invitation_content.rs` | `73a7acab3c57234cf5261560a9822749b4440c2ed2845ccac9c390aca212e291` |
