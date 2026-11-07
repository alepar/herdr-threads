# Final review, cluster 9 (migrations-node), epic ht-ma6i

## Source and scope

Reviewed base cf13ed76c1c2871b321ad290d03229cb785c400e and head 1b058307490f29ece8bb4e3fe7a862f8356d992f, both verified with `git rev-parse --verify`. The caller checkout was at the reviewed head with a clean status when read.

Files: `migrations/node/0016_graph_clone_identity.sql` and `migrations/node/0016_graph_clone_identity_after.sql` (slices 000055 and 000056). Other source was opened only to check named risks: `src/store/schema.rs`, `src/store/seats/reversible.rs`, `src/store/seats/continuity.rs`, `src/store/continuity_keys.rs`, `src/store/queries.rs`, `tests/store/schema.rs`.

## Spec conformance

- Spec 4.1, 4.2, 4.3 and 4.4 column lists all match: `seat_shell_fingerprints`, `continuity_keys`, `continuity_key_conflicts`, `reversible_retirements`, `restored_target_absence`. `continuity_alerts` has the fields the feed needs.
- Spec 4.2 adds `kept_continuity_claims.continuity_key`, spec 4.4 adds `retirements.absence_evidence`; both present, nullable, with CHECKs.
- Spec 4.5: `allocation_decisions` is rebuilt the 0012 way with all four new kinds, every earlier kind kept. Delta 16 is registered in `NODE_DELTAS` with `copy_allocation_decisions` and the after file.
- Spec 4.3: the terminal trigger is relaxed only for a seat with a row in state `reversible`.

## Migration mechanics

- Only 0001, 0012 and 0016 touch `allocation_decisions`, and no trigger body references it, so the copied column list is current and dropping the table loses no trigger. The after file recreates both indexes.
- No production `DELETE FROM allocation_decisions` exists, so copied ordinals keep the AUTOINCREMENT sequence correct.
- The rebuild runs with foreign keys off under `BEGIN IMMEDIATE` and ends with `foreign_key_check`, as `apply_delta` documents; `copy` is DML only and all DDL is in replayable SQL.
- Independent replay of all 16 deltas on SQLite 3.51.2 in memory: integrity and foreign key checks clean, no `allocation_decisions_new` left, every foreign key parent resolves and refuses an unknown parent, and a NULL primary key is refused in these STRICT tables.

## Constraint and writer agreement

- `ContinuityKey::parse` admits 1 to 128 printable ASCII bytes, matching every key CHECK. `HostTargetId::parse` caps at 128 bytes, inside the 512-byte target CHECK.
- `RetireReason` has only Closed and Operator, matching the reason CHECK. `ContinuityConflictKind` matches both kind lists, pinned by a serde-name test.
- `identity::shell_continuity::read` yields only a session leader with a terminal (`session == pid`), and `record_shell_fingerprint` range-checks the u64 fields and selects non-null structural columns, so a CHECK failure is not reachable from it.
- Decision kinds are read as plain strings in seat history (`src/store/queries.rs`), so a new kind cannot read as corrupt.
- `allocation_decision` in `reversible.rs` selects `host_id` from the seat, which a remote seat lacks, but it runs only under key-owner or operator finalization of a local row, and the fact paths never call it.
- No production `DELETE FROM seats`, `herdr_hosts` or `service_authors` exists, so the new restricting foreign keys block nothing. `restored_pane_watch` appears in no inventory that omits the new tables; `src/service/kicks.rs` lists all six.

## Trigger relaxation

- Every production write to `seats.state` or `seats.retired_at` is guarded by `state!='retired'`, by a specific prior state, or is one of the two un-retire statements, so relaxing the trigger exposes no accidental resurrection path.
- Both un-retire paths (`unretire_local`, `unretire_by_fact`) update the seat before changing the row, which is the order the trigger needs. The remote form (`state='remote'`, `retired_at=NULL`) satisfies the `seats` table CHECKs and home-node trigger.
- `finalize` and `finalize_by_fact` delete the row first and then change only `retired_seq`, which the trigger does not watch.

## Requirement baseline

- B1: tables and kept-claim column exist with production writers (`continuity_keys.rs`, `kept_continuity.rs`, `continuity.rs`). The end-to-end path from the hook and service leaves this cluster and is the seam reviewer's to trace.
- B2: the reversible row, relaxed trigger, `absence_evidence` and decision kinds are in place with production writers (`retirement.rs`, `reversible.rs`, `observation.rs`). Same seam note for the undo path.
- B3: no schema change is needed, since memberships stay on the seat; the delta does not touch them.
- B4: nothing in this cluster concerns Herdr.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- `graph_clone_identity_tables_enforce_their_checks` states it kills a dropped CHECK on the new tables, but it does not exercise the CHECKs of `seat_shell_fingerprints` (pid, start, session, tty), `restored_target_absence` (seat_generation, absent_observed_ms), the `kept_continuity_claims.continuity_key` length, the `continuity_key_conflicts` and `continuity_alerts` key and target lengths, or `reversible_retirements` reason and closure ranges; production writers pass only parsed or range-checked values, so the exposure is limited to a later edit dropping one of them unnoticed.
- This review replayed the SQL on an empty in-memory store and read the populated-store upgrade test without rerunning it; it did not upgrade a real populated v15 store.

## Deferred OK
- `retirements.absence_evidence` is outside the `retirements_provenance_immutable` trigger, as `closure_evidence` has been since 0009; no production statement updates either and `tests/store/schema.rs` relies on the column being updatable, so immutability here is by convention until a later delta extends the trigger.
- The 0016 header says "Local tables, never replicated", yet a peer also gets a `reversible_retirements` row from the replicated `seat.retired_reversible` fact (`insert_fact_row`); the table's own comment acknowledges peer rows and the schema is correct, so this is comment precision only.
