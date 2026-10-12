# Main v0.9.0 schema composition impact

Read-only source/Git inspection and isolated Python in-memory SQLite experiments; no source, SQL, Git, Cargo, Herdr or process-lifecycle mutations. Only this report was written.

Main baseline: `b6dd5dd122772599a02e2cc7f76b20e5492f0af3` (parent identifies published v0.9.0 session bindings). Candidate: `f330bdab3abee8535213228e5465b085b65be750`. Candidate path: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/upstream-composition-478ec4a9`. All source reads in the inventory/experiments were pinned with `git show REF:path`, so later workspace edits cannot change this assessment.

## Decisive result

Main publishes migrations1–35, including four genuinely new binding migrations32–35. Candidate shares only the exact1–31 prefix and has ten different unpublished migrations32–41. It does not contain the Main binding migrations under alternate numbers, nor their registration or schema columns. **Preserve Main1–35 byte-for-byte; append candidate logical additions as36–45. Numeric renaming alone is unsafe:** candidate receipt-send-time SQL rebuilds both receipt relations and drops Main's published `ack_binding_id` columns. Its new37 rebuild must preserve their definitions and exact historical values. This is a composition defect, not permission to modify the published32 migration.

## Exact SQL identity and registration

Enumerated both refs' migration trees and compared byte contents. Exactly31 common filenames, all byte-identical. None of candidate32–41 has an exact SQL counterpart anywhere in Main1–35. Candidate SQL contains no `binding_id`, `binding_cursors`, `legacy_pending` or `liveness_pid` witnesses; no older session-binding migration was imported as a prerequisite.

Main `src/store/schema.rs:70–74` registers session32, scope33, delivery34, attention35 and `LATEST_VERSION=35`. Its ordinary initialize suffix (`:653–669`) performs the four upgrades in order with exact verification after each. `migrate_v31_to_v32(:874)` samples the migration clock, executes32, stamps current upgrade time into `disconnected_at` for open migrated bindings, verifies and commits version32. This clock/stamp and its rollback behavior must survive composition; raw SQL replay is not the complete production migration.

Main33 retains its exact trigger replacement audit and the explicit `V33_SUPERSEDED_V23_TRIGGERS` list so the historical23 validator accepts only the declared later ownership. Main34 audits its rebuilt catch-up relation and exact default-zero lazy legacy column. Main35 audits both rebuilt per-binding frontier tables and indexes. Preserve those validators; add the new feature validators after them.

Candidate `src/store/schema.rs:70–81` registers invitation32, send-time33, body34, backfill35, lifecycle36, invitation-origin37, receipt-origin38, automatic-body39, presentation40 and relevance41; `LATEST_VERSION=41`. Its fresh branch directly applies through its own32 and stamps32 (`:180–187`); its suffix continues its own33..41 (`:679–789`). Merge the registry/initialize logic semantically: fresh and every historical path must execute Main32..35 once, then new36..45 once, preserving per-migration atomicity, FK restore, verification and clock behavior. Update range guards, stamps, names, includes, version-dependent verification and query-connection bounds together. A global string/number replacement would corrupt Main's newly published32..35 meanings.

Published Main additions and SHA256:

|Published SQL|SHA256|
|---|---|
|0032_session_bindings.sql|8c64d36344aceefb7fbbf82487d24d5a93d33da9d20297ad4547bf48abcd1033|
|0033_archival_scan_scope.sql|8cab27994ee53b25a5e08fb75b5c8dd18a5d0ac5ea48e06029abc85974f1309a|
|0034_binding_delivery_state.sql|64636bb15a05555a37f418c1041bcc84782aba881e0fa17ca4c7b78d3d7e2d3b|
|0035_binding_attention_frontiers.sql|628f64be5f80b7813445f79f2b747dcdffd2d6a3b64d463ff5c9a9cbc6dea960|

Candidate logical renumbering and pre-adaptation SHA256:

|Current candidate number|Appended number|Logical filename suffix|Current SQL SHA256|
|---:|---:|---|---|
|32|36|invitation_confirmation.sql|09af42e476d5c958e38f50256f9f16545d9ce85aeb695ce3d7adacaca1616229|
|33|37|receipt_send_time.sql|19fd3f7663b37ded6611b49a290915d9f6e3f3e09a3ecc9ac439ed6ccdcb8019|
|34|38|handoff_body_bounds.sql|ffd9e64a89546fb07be9e72627626ca704afa91c6f0092b8485e1194a7f4c37f|
|35|39|receipt_deadline_backfill.sql|770689f3a6a3d846ead9632d0e066e4ca8535e6515f03b12d82068423c8675eb|
|36|40|invitation_lifecycle.sql|206f7da7e8077fbd371c62d5155c23cfaece23a27e21d67294b51e2fc93204be|
|37|41|invitation_warning_origin.sql|59dc40257c4a69d5f8db9ef85d584a45211cacba9db2b0e8531261bf266ff569|
|38|42|receipt_warning_origins.sql|f75b12aa854d074560b65457bf570f3256634329dae648b55bdf9826c9a48333|
|39|43|automatic_handoff_body_bounds.sql|6cf21a42a889eb3df777d9f12f2dd3681447e06520b788ea406903dadb49e981|
|40|44|invitation_presentation.sql|a3ba9411ab1c516d200c52a2be089d8428051e531f0926d95a66fbbc360ea8b5|
|41|45|presentation_relevance.sql|0142aa5a67146fbc5bd77380609a86b7f892b32704fb4a2f32f36ba35f3523c7|

Retain suffixes for traceability. Most unpublished SQL can be moved unchanged apart from number-specific comments/helper object names where appropriate; old33/new37 requires an intentional content adaptation preserving Main ACK binding columns. Never rewrite a published1–35 file to manufacture the desired combined state.

## Published shape changes that remain authoritative

- Main32 rebuilds `occupant_bindings`: adds canonical binding ID, role/parent/agent/lineage, liveness/disconnect/process identity and migration metadata; replaces the old one-open-binding shape with session-key uniqueness. Preserves sequence high water and re-creates owned archival triggers. Creates `binding_cursors` keyed by binding/thread, seeded from joined history. Adds `receipts.ack_binding_id`, `receipt_state.ack_binding_id` and `messages.sender_binding_id`.
- Main33 narrows archival fences to relevant seat lifecycle and binding liveness; drops the general seat-insert fence and replaces four other triggers. Reinstalling old broad fences or requiring old23 text would undo the published behavior.
- Main34 adds checked default-zero `lazy_recipients.legacy_pending`, carries pending published lazy rows, adds two indexes, and rebuilds `catch_up` with `binding_id`/thread primary key, replacing old generation/execution identity. Preserves exact archival trigger ownership.
- Main35 rebuilds `warning_offer` and `digest_notice_offer` per binding; their old generation/execution columns are removed. Historical migration associates only exact prior occupant evidence, with unmatched obsolete frontiers dropped by the published rule. Two sessions of one seat must retain separate frontiers.

These are not additive empty-column changes alone: exact historical fixture snapshots and old table/rootpage assertions must model the declared rebuilds and data transformation explicitly.

## Confirmed renumber-only data-loss hazard

Executed Main1..35 SQL, then all candidate32..41 SQL in a fresh Python in-memory SQLite connection. All statements execute on an empty schema (no Rust validators/version stamping/clock path invoked). Immediately after Main35 both receipt tables contain `ack_binding_id TEXT`; immediately after candidate old33 both columns are absent. Remaining candidate SQL finishes normally. This proves SQL success alone cannot be used as composition validation.

Causal source: `0033_receipt_send_time.sql` saves the old rows with `SELECT *`, then its replacement `CREATE TABLE receipts` and `CREATE TABLE receipt_state` omit `ack_binding_id`; its explicit INSERT column and SELECT projection lists omit it too. Any non-NULL published ACK binding attribution would be discarded, and Main's ACK paths would subsequently target absent columns. Retain the exact nullable TEXT shape and append it to both copy lists in the unpublished new37 SQL; preserve its original ordinal/rowid, actor, generation, observation, time, deadline and binding value. This does not warrant changing Main32 or substituting the current binding for missing historical attribution. New37's exact verifier must require the combined schema rather than the old candidate-only relation.

Main32's sender binding column survives the candidate SQL experiment because no candidate addon rebuilds `messages`. Candidate new38/new43 rebuild only their explicitly documented bootstrap/automatic body-bearing tables, so those bodies' known rebuild exceptions remain separate from Main's32/34/35 rebuild exceptions.

## Historical test and oracle changes

Published Main historical tests must retain the meaning of32–35. `tests/store/schema.rs` at Main:6512/6514,6567/6574 directly apply32; :6593/6596 apply33; :6923/6926 apply34. Their migration names and precursor stamps remain unchanged. `LATEST_VERSION` completion expectations naturally advance to45, but exact published-step stamps must not be mass-rewritten.

Candidate references requiring logical renumbering:

- `tests/store/effective.rs`: :1060–1068 replays its old32..34, :1175 old37, :1201 old38, :1211/:1216 old40/41; :2069 and:2202 create old32 precursors. Convert those explicit addon includes and matching stamped versions to36..38,41,42,44/45 and36 respectively. The constructed v28/v31 historical precursor cannot jump straight to36: include actual Main32..35 before addon36 so a stamped36 store is honestly derived from the published path.
- `tests/store/invitation_content.rs`: :261/:270 use actual consumer40/relevance41 DDL; :418–425 replay old32..39; :427 selects old40's epoch ALTER; :439/:513 stamp/assert old39. New physical precursor is43, epoch ALTER44, consumer44 and relevance45. Include Main32..35 before addons36..43. Update diagnostics and names that falsely label experimental old39 as a released schema. Published Main release evidence means35, not the branch's former39.
- `tests/store/schema.rs`: :7035/:7081/:7154 explicit old40 includes become44; old39/40/41 fixture names and stamps become43/44/45. Directory-position fixtures are a particular trap: `presentation_populate_v39(:6776–6787)` uses sorted `.take(39)`, which after composition stops at39 backfill rather than43 automatic body bounds. The test at:6688 uses `.skip(31).take(7)` to reach old38; after inserting Main's four files it needs the actual combined chain through42 (eleven migrations after31), not merely renamed SQL references. Inspect every `.take`, `.skip`, source-path version and `PRAGMA user_version` against the logical migration it intends; `.take(28)`/`.take(31)` remain the published prefixes.
- Candidate store fixtures still writing `catch_up(binding_generation,execution_id,...)` or `warning_offer(seat_id,binding_generation,execution_id,...)` into latest databases must adapt to authoritative per-binding Main shapes. Examples in candidate `tests/store/schema.rs`: :4725/:4946/:4966 catch-up and :549 warning-offer. Old historical fixtures before the Main upgrade should retain the actual old shape and assert the published transformation instead of pretending it was additive.

Do not weaken exact catalog/original-row checks. Start upgrade addon tests from an actual published35 checkpoint when the purpose is addon preservation. For older real v28/v31 upgrades, derive explicit expected transformations for Main32/34/35: canonical binding IDs/liveness and migration time, added nullable receipt/message attribution, default-zero/legacy lazy carry, rebuilt catch-up and exact per-binding frontiers, and the exact Main33 trigger changes. Keep exact old definitions/rootpages for objects not declared rebuilt. Add focused full-row checks ensuring non-NULL published ACK binding attribution survives new37's receipt rebuild and physical reopen. Separate the new44 default-zero invitation epoch from the published32 nullable ACK columns; neither justifies dropping columns from snapshots.

Existing C schema triage remains valid for the prior f330 branch, but its statement that only39 can rebuild old objects cannot be carried unchanged to the new Main composition. Its118 presentation definitions become new44/45 and still need exact set/DDL checks. Main's four published migrations add their own explicit, independently audited allowed changes.

## Scope and required parent-owned verification

No composition was implemented and no Cargo was run. The smallest next meaningful causal check is a published35 populated fixture containing canonical per-binding frontiers/cursors, both physical and sparse ACKs with non-NULL `ack_binding_id`, sender binding identity and lazy/catch-up state. Run the new36..45 path, prove exact preserved published attribution and permitted deadline/body/presentation transformations, test induced rollback and reopen, then latest no-op verification. Older actual v31 upgrade coverage must also exercise Main's clock-stamped session migration before addons.

Schema additions keyed only by `(seat_id,consumer)` in candidate invitation presentation/proof tables are visible structural differences from Main's now per-binding delivery/frontier policy. Their coexistence is not resolved merely by renumbering; parent must review the intended binding ownership/admission semantics against Main's published policy. This report identifies the schema boundary and makes no unsupported claim that every such private row must automatically be rekeyed.

A database stamped with the old unpublished candidate32–41 history is not a published Main32–35 database: identical version integers now denote different schemas. Do not reinterpret stamps, skip published steps or infer historical binding identity from table names/local hints. Any support for private experimental databases needs an explicit separately designed conversion; recreating isolated test fixtures is simpler. Published Main1–35 bytes are the immutable production prerequisite.
