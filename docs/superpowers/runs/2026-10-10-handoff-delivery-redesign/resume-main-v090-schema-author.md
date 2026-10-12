# Main v0.9.0 schema lane author report

Authored in shared checkout `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/upstream-composition-v090` under parent-approved schema plan. Inputs: candidate `f330bdab3abee8535213228e5465b085b65be750` plus Main `b6dd5dd122772599a02e2cc7f76b20e5492f0af3`. No Cargo/product runtime, Git stage/commit, subagents, Herdr/seat mutations or user-config writes. Source edits are now quiescent; parent owns compiled validation and integration.

## Plan and approved interface

Plan sent before source edits: write actual published35 ACK-preservation test first; renumber only unpublished candidate32..41 to36..45; preserve both published nullable ACK binding columns and exact copied values in new37; retain Main32..35 functions/audits/clock semantics and compose addon suffix/fresh paths; resolve owned test conflicts and honest historical fixture versions; run file-only formatting/static reference checks and hand off to parent.

Parent explicitly approved Dscope schema keys after agreement with `impl_watch_recovery`: consumers and warning proofs PK `(binding_id,consumer)`; items PK `(binding_id,invitation_id,consumer)`; retain `seat_id`; canonical FK `occupant_bindings(binding_id)`; item revisit/cleanup indexes begin binding/consumer; consumer deletion reclaims proofs using exactly `OLD.binding_id` and `OLD.consumer`. Canonical invitation/content/context/confirmation projections stay seat/thread keyed. These new tables are first created at44/45 atop publishedMain35; no guessed migration or deletion of private historical D rows is added.

## Authored changes

`src/store/schema.rs`: Main constants32..35 retained, addons registered36..45, LATEST45. Fresh initialization constructs published35 then runs the shared addon suffix; historical paths run Main session migrations once before addons. Published32 disconnect-clock stamp and its exact audits, Main33 trigger replacement ownership, Main34 catch-up/lazy audits and Main35 per-binding frontier audits remain. Exact addon audits now use the appropriate renamed SQL. Body-bound schema guards use37/42 as pre-upgrade thresholds, and query-connection admission follows LATEST45. No whole-catalog or old-object rootpage normalization was introduced.

Migration renames:

|Old candidate|New physical version|Logical suffix|
|---:|---:|---|
|32|36|invitation_confirmation|
|33|37|receipt_send_time|
|34|38|handoff_body_bounds|
|35|39|receipt_deadline_backfill|
|36|40|invitation_lifecycle|
|37|41|invitation_warning_origin|
|38|42|receipt_warning_origins|
|39|43|automatic_handoff_body_bounds|
|40|44|invitation_presentation|
|41|45|presentation_relevance|

New37 intentionally preserves `ack_binding_id TEXT` in BOTH rebuilt `receipts` and `receipt_state`, and includes it in each explicit INSERT/SELECT copy list. No historical value is derived or replaced. All other published receipt row values, rowids and sequence high water remain covered by the existing rebuild logic and tests. New44/45 implement the agreed binding keys/FKs/indexes/reclaim predicate.

Owned test files adapted: `tests/store/schema.rs`, `effective.rs`, `lazy26_prerequisite.rs`, `handoff_fences.rs`, `topology_handoff.rs`, `topology_handoff_schema.rs`. Restored the Main-only existing `unpublished_staging_is_invisible_and_first_later_anchor_is_immutable` key, lost by the preliminary shared merge: it still checks first observed availability1200 despite a later backdated500 event, and now separately requires the new frozen send deadline1300 before/after availability. Current-version assertions use45/LATEST where appropriate; published step32..35 precursor assertions retain their actual meanings. Hand-built addon precursors include the actual Main32..35 chain, with private candidate logical32/33 checkpoints now36/37. Actual released v31 physical fixture and existing test keys retained. Directory-count fixtures reach actual43 rather than former39, and automatic-bound rollback fixtures reach42 rather than former38. The bootstrap historical image helper caches all45 genuine prefixes.

The dormant catalog oracle starts after actualMain32..35 plus addon36..38; preserves exact old object membership, DDL and rootpages, with only its four explicitly rebuilt automatic body relations as the prior allowed exception. Its exact118 new44/45 object oracle is derived from their SQL, and only those declared new allocation names have unconstrained page numbers. Invitation row snapshots explicitly gain one exact44 INTEGER0; published pre32 receipt/message snapshots explicitly gain their one nullable binding attribution field. There is no broad object-prefix exclusion or old-page normalization. The separate bootstrap upgrade root oracle admits only the exact Main32/34/35 table-owned rebuilds a given real historical precursor still needs; Main35+ predecessors continue comparing those pages exactly.

## Test-first regression and evidence

Added before production edits: `store::schema::tests::published_v35_upgrade_preserves_exact_ack_bindings_and_delivery_history`.

Builds actualMain35 from an existing populated real Main34 fixture plus immutable35 SQL, stores distinct non-NULL historical ACK binding IDs in physical and sparse receipts and sender attribution, then VACUUMs to a private physical file. With FK false/true it forces new37's post-copy FK check to fail, checks exact receipt catalog/rootpages and all captured published rows roll back at actual36, repairs only the child obstruction, upgrades, checks both original ACK binding IDs, and compares all complete rows in occupant bindings/cursors/catch-up/warning/notice/lazy/message/receipt relations. Physical reopen/latest initialization must preserve all rows and avoid resampling migration time. Existing keys are unchanged.

Compiled Rust RED/GREEN remains parent-owned because this lane was forbidden Cargo. The minimal isolated Python in-memory SQL experiment was run with actualMain1..35 and non-NULL physical/sparse ACK values:

- Original candidate old33 rebuild: expected RED, `no such column: ack_binding_id`.
- Corrected new37 rebuild: exact non-NULL values `first-binding`/`second-binding` and entire message/receipt/state rows preserved.

The same SQL-only experiment with all45 source migrations succeeds and yields exact noninternal catalog counts95 tables,167 indexes,307 triggers; these literals replace the conflicted latest-catalog count assertion. This is source/SQL evidence only, not a Rust passing-test claim.

## Completed static checks and handoff

File-only `rustfmt --edition 2024 --config skip_children=true` succeeded for the seven owned Rust files. Static inspections: no owned conflict markers; no missing owned `include_str` target; exactly one SQL file at each version1..45; every existing test key from both Main and candidate retained. All35 published Main migration files were individually byte-compared with their pinned Main Git blobs: zero differences.

`src/store/mod.rs`: made no edits. Module exports were already clean; remaining conflicts belonged to forwarding methods. Handed the quiescent file to `compose_v090_store_binding`, cc parent, for binding/reconnect API forwarding; no further schema-lane mod writes.

Pending parent checks: compile all affected targets; run the new actual35 regression and exact mutation RED with an in-memory/or safely restored unpublished37 omission; focused schema/effective/lazy/bootstrap tests; integration matrix; required fmt/clippy/neutral/default gates and scoped process checks where applicable. Other lanes own presentation runtime/test adaptation, capabilities, policy and service coverage. No pass/merge readiness claim is made here.
