# Main v0.9.1 schema fixture follow-up

Source input `f39da231501cb686e361ff656000e777f984735e`. Output is only `resume-v091-schema-fixture-fix.patch` and this report in the requested ignored scratch directory. No candidate source edits, Git writes, Cargo, test/runtime execution, SQLite execution, tracker or Herdr actions. Parent supplied actual95 result:89PASS/6FAIL, scoped leaks0; the three new canonical36 controls passed. Those runtime facts are parent evidence, not independently rerun here.

## Root cause and selected repair

`tests/store/schema.rs::user_intent_schema22_fresh_and_v21_upgrade` snapshots every table root except its existing enumerated exemptions. The old root comparison gained six extra names created by feature migrations45/46. The failing result reports no displaced historical root. Frozen SQL inspection confirms none of the six CREATE TABLE names occurs in migrations1–44;45 creates invitation_context_latest/consumers/items,46 creates confirming_invitations/warning_fence/warning_proofs. The only later alteration to any of these six table definitions is46’s four explicit added consumer scalar columns. This is a stale additive-table fixture, not evidence of old-table rewriting.

The patch declares those six exact names paired with their actual45/46 source SQL. Before excluding a name from the root comparison, it asserts that name is absent from the genuine historical snapshot and compares its complete installed table SQL byte-for-byte against an independent scratch catalog built from the exact named CREATE TABLE declaration plus all four actual46 consumer ALTER statements. Expected declarations are not copied from the upgraded catalog or the initializer. Missing/altered tables and wrong ALTER counts fail. No prefix/pattern exclusions, unknown-table allowances, root normalization or production-verifier delegation are introduced.

The old adapter-table exception list, including canonical Main in_turn_offer, is unchanged. All historical root entries outside the existing exemptions still compare exactly. The original22-before/latest data comparisons, all column-null assertions, full fresh-vs-upgraded shape assertion, repeat initialize and final snapshot equality remain unchanged. No migration or production code changes. The audit is deliberately six table DDLs; the existing full-schema shape/production audit checks continue covering associated indexes/triggers.

## Additional config8 latest pin and downgrade inspection

`tests/integration/session_bindings_smoke.rs::config8_upgrade_from_a_v0_6_0_database` had the imported Main latest-version literal36. The second hunk changes only that exact assertion to46. Its binding, shown-cursor, pending Lazy/ACK message, claimless refusal and restart oracles are unchanged.

The `downgrade_to_v28` helper already constructs a genuine actual28 schema. It checkpoints and renames the current database to a separate `.current` path, removes old WAL/SHM companions, opens a **new** database at the original path, enumerates and executes exactly migration filenames1–28, stamps28, and retains that database’s own trigger SQL. It attaches the renamed latest database only as a row source. Copy destinations are enumerated exclusively from main.sqlite_master (the actual28 catalog), columns are intersected, and rekeyed delivery relations are explicitly excluded. It then detaches the old database, marks genuinely historical shown Lazy rows, restores actual28’s triggers and checks version28. There is no DROP-a-few-latest-objects/rewind-stamp conversion and no source-schema copy. In_turn_offer and feature presentation tables never become destination tables, so no new36/featureDDL removal or larger fixture rewrite is needed. Runtime data compatibility remains parent-owned.

## Static verification and handoff

Both unified patch hunks were applied to exact frozen blobs in memory; hunk context/counts matched. `rustfmt --edition2024 --config skip_children=true --emit stdout` parsed each resulting Rust file via stdin and returned exactly the proposed formatted contents. Only the named schema test body and config8’s one literal change are modified. No test name, alias, cap, timeout or counter changes.

Parent should apply the scratch patch, run the exact user_intent schema test and config8 integration case (with scoped process ownership/leak checks), then rerun the selected family/gates as planned. This source-only repair has no GREEN or regression-potency claim.
