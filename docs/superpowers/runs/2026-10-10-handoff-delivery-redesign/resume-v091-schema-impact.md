# Main v0.9.1 schema composition impact

Read-only investigation, 2026-10-12. Frozen inputs: previousMain `565def869ae50e47e6da7388ea333229050eaf8d`; newMain `20e7ca73e41a9449376169ab6875c0675da02edc`; feature `0b04f91ba5fbaf4f5df4819fe8ccda6a704c26a2` (`super-auto/sweep-validation`). All source comparisons use Git blobs, independent of the parent’s mutable merge. Only this ignored scratch report was written. No source, Git, tracker, Herdr, Cargo or process-lifecycle mutations; no delegation. The SQLite experiment below used memory only. This report makes no upstream allocation claim and gives no runtime/merge clearance. Parent owns composition and originalGoal interpretation.

## Result and exact migration recipe

The previous Main and feature share all 35 published migration files byte-for-byte. New Main preserves those same files byte-for-byte and adds `0036_in_turn_offer.sql`. Preserve **all canonical Main1–36 byte-exact**. Its new36 SHA-256 is `dd17b386f774d238232f2d4d945a85621eb42d6b781a185a2c389704e7662e11`.

Append the ten provisional feature additions in their existing order, incrementing physical version once:

| Feature version | Composed version | Suffix |
|---:|---:|---|
|36|37|invitation_confirmation|
|37|38|receipt_send_time|
|38|39|handoff_body_bounds|
|39|40|receipt_deadline_backfill|
|40|41|invitation_lifecycle|
|41|42|invitation_warning_origin|
|42|43|receipt_warning_origins|
|43|44|automatic_handoff_body_bounds|
|44|45|invitation_presentation|
|45|46|presentation_relevance|

Do not merge the two different migration36 meanings, append Main36 after feature45, or recognize a private provisional36 database as canonical Main36 by version stamp alone. No rewrite/automatic conversion of private historical feature databases is authorized by this report. Existing structural audits must reject the wrong shape. Keep feature SQL intent intact; update physical-version comments only where needed. New38 must retain both nullable `ack_binding_id TEXT` columns and their exact INSERT/SELECT copies in receipts and receipt_state, which the previous composition already repaired. New45/46 retain binding-keyed presentation consumers/items/proofs and their exact FKs, indexes and reclaim predicates.

A Python SQLite in-memory experiment replayed the exact source SQL in three chains with no schema substitutions: Main1–36, feature1–45, and Main1–36 + feature36–45. All succeeded. Noninternal catalog counts (name NOT LIKE sqlite_%) were:

| Chain | Tables | Indexes | Triggers |
|---|---:|---:|---:|
|Main36|85|148|198|
|Feature45|95|167|307|
|Composed46|96|168|307|

The feature suffix leaves Main’s `in_turn_offer` table and `in_turn_offer_seat` index SQL intact. This proves empty SQL composability only, not production Rust migration or data preservation.

## Registry and initializer changes

At feature `src/store/schema.rs:74–85`, insert canonical Main V36 and shift every feature V36..V45 constant/include to V37..V46; LATEST becomes46. Take Main’s `migrate_v35_to_v36`, `V36_AUDITED` and `verify_existing_v36` exactly in behavior: BEGIN IMMEDIATE, execute exact36, exact two-object audit, stamp36, commit or rollback. Run its verifier for every admitted latest/historical path before the feature suffix.

Fresh path currently creates through Main35 and stamps35 (`:198–199`), then traverses the feature suffix because original `version==0`. The least disruptive composition keeps this architecture and executes Main36 for `version <=35` (including0), audits it, then feature37 for `version <=36`. Alternatively construct Main36 in the fresh transaction and use Main’s `(1..=35)` migration condition, but do not accidentally double-apply or omit it. Historical32 migration’s clock sampling/disconnect stamp stays unchanged. Versions1–35 still traverse their real Main upgrades;36 proceeds directly into the appended feature suffix;37–46 verify the installed canonical36 before any further writes.

Shift only feature thresholds and committed stamps:

- Original topological body audit boundary `version <=37` becomes38 (`:607`); automatic-body boundary `<=42` becomes43 (`:649`). Published32–35 audit ownership thresholds remain their true numbers.
- Suffix condition/commit pairs become <=36→37, <=37→38, …, <=45→46. Rename `migrate_receipts_v36_to_v37`→`migrate_receipts_v37_to_v38`, `migrate_handoff_v37_to_v38`→`migrate_handoff_v38_to_v39`, `migrate_automatic_v42_to_v43`→`migrate_automatic_v43_to_v44`, including their internal pragma stamps.
- Presentation audit’s `version >=45` becomes>=46. All SQL-derived verifiers must follow the shifted semantic constant, especially confirmation(oldV36), receipt(oldV37), warning-origin(oldV41/V42), presentation(oldV44) and relevance(oldV45).
- Keep query-connection admission at LATEST. Do not globally replace numeric35/36: canonical Main36 and prior published versions have independent meanings.

## Fixture/catalog/version inventory

These are source anchors on the feature ref, not mutable-worktree line promises:

1. `tests/store/schema.rs`: latest literal45→46 (including explicit LATEST pin near6297, reopen near8114); exact catalog95/167/307 near7144→96/168/307; add the exact new `in_turn_offer` name to the historical22 newly-created-table exception list near5780, as Main does. Do not broaden old-rootpage exclusions. Directory fixture `.skip(31).take(11)` near7565 becomes take12 and precursor stamp42→43; following automatic rebuild is44. Presentation precursor `.take(43)` near7664 becomes take44 and stamp43→44. Old44 presentation precursor stamps near7965/8038 become45; final46 relevance upgrade/reopen expectations move accordingly. Historical test keys containing old logical versions need not be renamed merely to renumber SQL, but comments must explain actual physical prefixes.
2. `tests/store/effective.rs`: exact historical replay array near1053–1086 must insert real Main36 before renamed37–39, not merely rename includes. Its dormant precursor38 becomes39. Update origin/presentation includes near1191/1217/1227/1232 to42/43/45/46. Its declared presentation-only additive oracle remains exactly118 objects =6 tables+16 indexes+96 triggers: Main36 is a retained preexisting table/index in this fixture, not a new presentation object to exclude. Preserve its four exact automatic-body rebuild rootpage exceptions. Receipt upgrade arrays `[28,36]` and equality branches/stamps near2089/2103 and2297 become `[28,37]`; replay Main36 before feature37. The six explicitly audited receipt fence triggers are still the same additions, now migration46. Genuine published28/31 fixture versions remain genuine.
3. `tests/store/invitation_content.rs`: schema-counter probe old44/45 includes near263/272 become45/46. Full explicit prefix near392–431 must insert Main36 and run through new44; the one-ALTER epoch probe uses45 and leaves user_version44 (old43 assertion near519). Do not manufacture a historical image by resetting latest’s stamp.
4. `tests/store/topology_handoff_schema.rs`: cached prefix generator `.take(45)`→take46; `historical(45)` comparison→historical46; all latest assertions45→46. Add canonical36 as a real upgrade/reopen precursor, rather than relying only on low-version coverage.
5. `tests/store/lazy26_prerequisite.rs` (110/136/202), `tests/store/handoff_fences.rs` (420), `tests/store/topology_handoff.rs` (73/1337): latest assertions45→46. `tests/store/lazy_schema.rs` uses LATEST on the feature side; preserve it when resolving Main’s36 literal change.
6. Search all active source/test include_str paths and schema-specific version literals, not just conflicted files. Historical evidence files should retain their historical ref/version facts; do not mass-rewrite prior reports or numbers unrelated to schemas.

The existing `published_v35_upgrade_preserves_exact_ack_bindings_and_delivery_history` at schema.rs:8136 remains meaningful. Its failing receipt rebuild moves from37 to38, and the last successfully committed version from36 to37. Keep real published35 source, exact ACK identities and all existing snapshots. Add a distinct populated **canonical36** preservation case (or parameterize real35 and36 without dropping either) carrying multiple `in_turn_offer` rows on different bindings/items, alongside exact historical ACK/cursor/frontier/lazy history. Snapshot rows and table/index SQL/rootpages before upgrade; require preservation across successful46 upgrade, failed38 post-copy FK rollback, repaired retry and physical reopen. Main36’s table has no item FK; fixtures should still use actual pending message/invitation IDs to express provenance honestly.

Retain corruption controls for published guards; add missing/changed canonical36 table/index refusal before suffix writes. Include a late36 migration failure control from real35 demonstrating no partial table/version advance. A wrong private feature36 image must not be accepted as published36. Do not invent broad catalog normalization or assume a missing new table may be created into an already-latest database.

## Behavioral seams numeric renumbering does not resolve

### 1. In-turn wake evidence and lazy invitations

Main adds `AdvanceCursors.in_turn` and `invitations`, capability `inbox.in_turn_offer_v1`, and top-level-only `in_turn_offer` recording in `control::advance_cursors`; lifecycle reset clears that binding only, while retirement/unresolve/rebind clears seat records. Preserve feature’s exact admitted-display loop, direct-episode cursor baseline and coalesced `presentation_reader::note_binding_reader_change` when adding this recording. Failed/partial/foreign display cannot become wake evidence, and in-turn records do not ACK, accept, advance a private presentation token or manufacture authority. Standard hook lifecycle versus turn boundary remains distinct; V2 physical confirmation is not a standard-hook full-body completion.

Main’s wake candidate and dispatch post-reservation recheck currently call `attention::pending_invitations`. Feature introduced **pending_confirming_invitations** for wake: Lazy invitation remains passive; only NULL legacy/require_ack invitation belongs to the invitation wake set. Compose candidate all-target suppression and dispatch exact-target recheck using the same feature-confirming population as reason selection/batch construction. Otherwise a shown confirming invitation plus an unshown passive Lazy invitation spuriously makes the served binding owe a wake. Preserve saturation→not-proven-served, no-target→not-served, mixed-reason behavior, new-item retry, exact composer binding and fallback/recovery-hint recheck. The new in_turn table needs no presentation-relevance fence by default: it changes only wake eligibility, not content relevance; do not add an unneeded trigger or new invalidation fanout.

Controls: retain Main `in_turn_display_of_a_nudge_owes_no_idle_wake`, invitation counterpart, sibling lifecycle reset; store composer-target/post-reservation/mixed-reason/recovery-hint tests; CLI capability downgrade test. Add mixed confirming+Lazy invitation with only confirming shown, all-served vs unserved sibling, and passive item never independently wakes. Keep both feature modern Lazy invitation and legacy timed invitation controls.

### 2. Main join frontier directly conflicts with feature direct-accept policy

Main `read_cursors::init_for_join` now marks undisplayed published Lazy rows **above the seat’s maximum cursor across all bindings (including ended/subagent)** as `legacy_pending=1`, then raises every open binding to head. Main’s amended A8 states the first top-level display settles that pending copy seat-wide and a sibling receives no copy. Main pair tests intentionally pin this, including no ordinary redisplay after mixed opening acceptance.

Feature `TRUST-POLICY.md:823–890` explicitly requires the exact opposite for **active direct invitation acceptance**: preserve every established own cursor; seed only missing cursors at the exact invitation event sequence; invalidate each affected reader once; preserve independent sibling streams; mixed ordinary/Lazy redisplay above the floor is intentional. `control::accept` captures `direct_acceptance_floor` before membership changes and selects `init_for_direct_accept`, with generic Join/nonpair acceptance using `init_for_join`. This is implemented and covered by Q4, not a stale speculative design. The frozen Q4 episode-design report explicitly documents the mixed-mode consequence.

Recommended composition preserving feature intent: keep canonical Main’s improved generic `init_for_join` behavior for generic Join/nonpair paths, and preserve the feature’s narrow exact direct-accept override, all reader invalidation/rollback work and preaccept direct-Lazy eligibility. Merge trust text to state that exception explicitly; it cannot truthfully say both “every accept uses head/seat-pending” and “direct accept preserves independent cursors.” Parent must resolve this against originalGoal; do not silently choose Main’s new pair expectations or discard feature Q4. Keep Main’s out-of-order displayed-row guard for generic join/rejoin, and test the direct exception’s contrasting intended result openly. No schema change is needed for either policy.

Controls: actual mixed direct opening with pending ordinary and Lazy, both before and after acceptance; sibling own cursors including one displayed out of order; later invitation episode and established old cursor; missing-event refusal; failed/overflowed reader invalidation atomic rollback; accept replay idempotence; ordinary nonpair/generic Join retained head/seat-pending behavior. Main tests `pair_accept_delivers_mixed_opening_mail_once`, `pair_rejoin_with_two_bindings_pins_read_positions`, `accepting_a_pair_invitation_keeps_the_opening_messages_unread`, and `rejoin_never_reshows_a_lazy_row_displayed_out_of_order` must be assessed against the now-explicit route policy, not weakened or dropped without a stated reason. Feature policy supports a different direct acceptance oracle.

### 3. Hook, consumer invalidation and serialization integration

Main enables Codex UserPromptSubmit and in-turn marking/capability negotiation; feature hook-pull delivery and bound presentation identity remain separate. Preserve current hook registrations, contract/capability goldens, all new AdvanceCursors defaults/validation, and every Rust struct literal affected by the new fields. Main’s `state` read in generic join is legitimate only to avoid re-marking a displayed row; modern bound unread, Q4 episode admission and private carry/owner eligibility must keep their canonical cursor/legacy predicates. Consider whether generic join’s new seat-pending flag publication reaches every affected existing consumer using established canonical publication triggers; do not reset payload/carry or select an arbitrary sibling. Feature lazy completion’s exact current binding invalidation and coarse publication trigger nudge must remain.

Parent validation should run focused real-prefix schema/effective/bootstrap/invitation-content tests, then exact affected wake/hooks/direct-lazy/Q4 controls, including actual channel/hook paths where already required. Required lint/neutral/default-feature gates and leak checks remain parent-owned. No Cargo or integration test was run by this investigator, and SQL-only success is not a release qualification.
