# Final review, cluster 9 (migrations), epic ht-z41e

Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168 (both verified with `git rev-parse --verify <sha>^{commit}`). Caller checkout observed at HEAD c3e9ab0be with a clean status before and after the review. Nothing was built or run: this review read the diff, the registry and the tests at head.

Cluster inputs: slices 000061-000063, which add three files: `migrations/node/0020_seat_config.sql`, `0021_side_queue_routing.sql` and `0022_binding_auxiliary_session.sql`.

## What the cluster delivers

- 0020 creates the `STRICT` table `seat_config(seat_id REFERENCES seats(id), key, value NULL, updated_at, writer_provenance, writer_binding NULL, PK(seat_id,key))`. This matches design §1 *Storage*, including a NULL `value` for the audit-only row of a column-backed key (`nudge_threshold`). `seats.id` is `NOT NULL UNIQUE` in the baseline, so the foreign key is valid. Foreign keys are on in production connections (`src/store/connection.rs:935,1104`). Seats are never deleted (no `DELETE FROM seats` in `src`), so the FK cannot block any existing path.
- 0021 adds the nullable column `wake_intents.side_sessions TEXT`. The only writer is `src/store/wake_intents.rs:261` (a JSON array, or NULL when empty). The only reader decodes it into the intent struct, and nothing branches on it, which matches the file's "diagnostic only" comment and the design's "Unchanged behavior: `wake_intents.side_sessions` shape ... unchanged".
- 0022 adds `occupant_bindings.auxiliary_session INTEGER NOT NULL DEFAULT 0 CHECK(IN (0,1))`. The only writer is `src/store/bindings.rs:1491` (`SET auxiliary_session=1 ... AND auxiliary_session=0`, so the bit is sticky and never cleared). The only reader is queue root selection (`src/store/wake_intents/selection.rs:184`). This matches the amended §2 *Root marker*: the bit is scoped to one binding row, a new row starts at 0, and `concurrent_session` is not touched.
- Registration: `src/store/schema.rs` lists NodeDelta versions 20, 21 and 22 in order with `rebuild: None`, so `NODE_LATEST_SCHEMA_VERSION` becomes 22. `main` (afeed32ac) has moved past the merge base but still ends at 0019, so the numbering does not collide.
- Downgrade: an older binary refuses the store ("store is newer than this binary", `src/store/schema.rs:453`). This is the existing one-way upgrade behavior and was not introduced here.
- The schema-wide lists include the new table: `seat_config` is on the no-kick table list (`src/service/kicks.rs`). Final retirement deletes its rows at both choke points, `insert_retirement_job` and `insert_replicated_retirement_job` in `src/store/control/retirement.rs`, as design §1 requires.
- Tests at head (`tests/store/schema.rs`): `migration_0020_seat_config_shape` checks the column shape, nullness and primary key order on both fresh and upgraded stores. `migration_0021_wake_intents_side_sessions_is_a_nullable_text_column` checks the new column. `migration_0022_binding_auxiliary_session_defaults_to_zero_and_upgrades_existing_rows` upgrades a v21 store holding a `concurrent_session=1` row: after the upgrade that row reads `auxiliary_session=0`, and the CHECK rejects the value 2.

## Requirement baseline trace (cluster scope)

- B1, B2, B6, B7 and B10 (per-seat config, closed registry S1, tiers, `nudge_threshold` stored over the existing column): this cluster provides only the storage. The table shape fits a closed registry: SQL does no key validation, the comment names `src/protocol/seat_config.rs` as the single source, and that file exists at head. `nudge_threshold` keeps its value in the column, with no data migration, as B10's "Key over existing column" requires. Tracing these rows end to end from the CLI and the daemon is the job of the CLI/service clusters and the seam reviewer.
- B8 (/side queue routing): this cluster provides the storage for the auxiliary marker and the diagnostic record of the side sessions. Tracing it from Codex hook decode through binding persistence to selection crosses into the harness and store clusters, so the seam reviewer owns it.
- B9 (side composer not empty): no migration is involved.

## Findings

- none

## Concerns

- The `CHANGELOG.md` "Unreleased" entry does not yet mention the one-way store upgrade to node schema 22 (migrations 0020-0022) or the fact that older binaries will then refuse the store. Every earlier entry that migrates the schema says so. This is release-note work for the release step, not a defect in the migrations.
- `migration_0020_seat_config_shape` does not check the `REFERENCES seats(id)` foreign key or the `STRICT` flag. Both are present in the DDL, and the schema audit replays that DDL verbatim, so a drift would still be caught there.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- The migration tests were not run in this review (no review-owned build). Pass evidence rests on the task logs under `evidence/ht-z41e.3/` and `evidence/ht-z41e.11/` and on CI.
- No test opens a real pre-upgrade store from an earlier release (v3.6.0, schema 19) and upgrades it to 22. The upgrade path is covered only by in-memory `open_with` over the delta prefixes.

## Deferred OK
- Add the one-way upgrade to schema 22 (and older binaries refusing the store) to the CHANGELOG release entry when the release is cut.
- Optionally make the 0020 shape test also assert the seat foreign key and the STRICT flag.
