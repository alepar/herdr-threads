# Task 6: Invitation acceptance, due and delivery lifecycle

Status: DONE for Task6 owned implementation; fresh scoped rereview pending after round2 (no CLEAN claim). Downstream integration obligations remain parent-owned. Assigned branch task-ht-0lo.2.2. Final clean committed HEAD after review fix round2: 19afa478b8b71141ea958d96278302a808ef9ed0 (differs from BASE). Initial implementation commit remains ae922aba412bf69f457818ecba8341e224f339b4. Full clean BASE confirmed before edits: a04d1a41838fdcfee18b8cb12ba0c1595cd103f3.

## Scope and seams

Own worktree: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.2.2`.

Read task brief, appended execution seams, root and invitation specifications, task-2 report/review, AGENTS.md and TRUST-POLICY.md. Applied implementer/TDD/verification workflow; no subagents spawned and no Herdr messaging, acceptance, ACK or lifecycle operations performed. Parent explicitly approved additional central `src/store/attention.rs` and `src/store/effective.rs` paths, additive result metadata plus mechanical output/fixture plumbing, migration30, and a temporary linked-handoff guard with its production-validator regression. Integration/merges/bead closure belong to parent.

The public scanner seam remains `scan_invitation_due_batch(context, conn, Option<InvitationDueCursor>, limit)` -> `InvitationDueBatch { inspected, warnings_added, next }`. Limit is 1..=100. Cursor fields remain `high_water_ordinal`, `after_deadline`, `after_ordinal`; after_deadline is informational, not a traversal key. First scan captures max ordinal. Selection seeks ordinal > after_ordinal and <= high_water_ordinal, ordered by ordinal in partial index `invitations_confirmation_scan`. Its predicate is pending, warning_message_id NULL, reject_recorded=0, and confirmation NULL or require_ack. Future/cancelled physical candidates count as inspected; canonical eligibility/deadline are reread under the deciding write transaction. Full slices return Some cursor, including an exact-full final slice; a following empty call returns None. New ordinals cannot stretch the finite snapshot. Task7 owns immediate continuation and shared receipt/invitation scheduling, not this task.

Provisional migration: `migrations/0030_invitation_lifecycle.sql`. No old migration bytes changed. It adds the bounded ordinal scan index and seat/thread-leading confirming-only attention indexes; integration must renumber with other tasks' additive migrations.

## Initial TDD RED

No production source was changed until this actual runtime RED completed. Private target metadata preflight confirmed workspace_root and target_directory both point to this task's worktree.

Command (assigned exclusive lane, CLI-only escalation for nice/socket sandbox restrictions):

```
CARGO_TARGET_DIR="$PWD/target" CARGO_BUILD_JOBS=1 nice cargo test --locked --all-features --lib invitation_
```

Exit101. Compile7m17s; tests1.65s. 67 selected:60 passed,6 failed,1 pre-existing ignored. Meaningful owned failures:

- `protocol::capabilities::tests::invitation_confirmation_capability_is_advertised_with_active_lifecycle`: capability absent.
- `store::control::tests::invitation_confirmation_creates_frozen_policy_and_only_timed_wake`: modern Invite returns Unsupported, "invitation confirmation lifecycle is not active".
- `store::control::tests::topology_attachment::actual_main_linked_phase_rejects_unfrozen_invitation_confirmation`: actual selected child validator returns Ok(()) for unfrozen Some policy instead of OperationPayloadMismatch.
- `store::invitation_due::tests::lazy_invitation_is_discoverable_but_never_due_or_waking`: inspected/warned=(1,1), expected(0,0).
- `store::invitation_due::tests::timed_invitation_warns_original_inviter_after_leaving_and_clear_is_passive`: original inviter is not a warning recipient.
- `store::invitation_due::tests::invitation_due_index_bounds_physical_work_across_lazy_and_future_backlogs`:300 inspected instead of251; lazy rows incorrectly consume physical scan pages.

Pre-existing ignored identity: `store::attention::tests::pending_invitation_axis_is_flat`, ignored in baseline because it writes10^4 items through real writers (60-180s debug under parallel load; release-only). No test newly ignored, skipped, or disabled.

The initial250-arrival fixture passed baseline because all initial deadlines were500 and inserted200 deadlines fell before the retained deadline cursor. It was strengthened to first100 initial deadlines100, remaining150 deadline900, and continuing arrivals at200, placing arrivals after the old cursor and before unfinished old rows. This correction is disclosed rather than claiming the initial passing fixture established a regression. The separately failing lazy/future index regression already established the missing bounded scanner behavior before production changes.

Cargo lane was released immediately after process exit, while production was implemented; parent assigned Task1 next. No spawned test daemons/private Herdr servers or helper children remained.

## Verification log

Independent Python SQLite preflight executed all30 migrations with foreign_keys=ON and compared every new lifecycle index SQL with sqlite_master exactly: exit0, "All 30 immutable/additive migrations execute; lifecycle index catalog audit matches." Cargo fmt/check and git diff --check passed for the current package before the next build.

Second assigned exclusive lane used parent-authorized warm-cache `CARGO_BUILD_JOBS=4`, same persistent private target, and `nice cargo test --locked --all-features --lib invitation_`: exit101, compile1m18s, runtime1.72s. 72 selected:69 passed,2 failed,1 pre-existing ignored (same identity above),3534 filtered. All six original RED causes pass. Compilation improved substantially from jobs1 cold/private-cache preparation but still exceeds the1-minute incremental budget by18s; no claim that this budget is met.

New meaningful RED, added before the corresponding source changes:

- `store::control::tests::invitation_confirmation_required_service_gets_fresh_timed_episode_without_mutating_lazy`: required service invitation reused the Lazy invitation ID, violating immutable voluntary policy and timed required membership.
- `store::invitation_due::tests::timed_invitation_warns_original_inviter_after_leaving_and_clear_is_passive`: open warning origin projection now passes, but the new explicit passive-clear assertion fails because the former inviter lacks its clear notice.

Cargo lane released immediately on these failures. Source corrections then create a separate timed service episode when the existing voluntary episode is Lazy (old policy, origin, keyed outcome and history retained), and project the original inviter as a passive clear recipient without inserting wake work. Their subsequent GREEN is recorded below. The combined metadata regression remains unrun and metadata production remains untouched.


Third assigned Cargo lane: `CARGO_TARGET_DIR="$PWD/target" CARGO_BUILD_JOBS=4 nice cargo test --locked --all-features --lib invitation_` exits0. Compile31.37s, runtime1.61s;72 selected,71 passed,0 failed,1 pre-existing ignored,3534 filtered. Same ignored benchmark identity above; no new skips or diagnostics. Both service-episode and passive-clear regressions now pass, together with all original failures. Incremental compile for this correction meets1-minute budget.

Downstream Task8 obligation explicitly assigned by parent: `src/cli/watch.rs:550` currently marks all InboxBatchV2 invitations as saw_attention; `src/cli/hook_inbox.rs:105` similarly treats all invitations as attention. Task6 exports optional modern policy/deadline metadata for these consumers; Task8 must use it to keep modern Lazy discovery passive. Task6 does not claim client attention interpretation complete. Canonical native/mod server producers in Task6 exclude Lazy via confirming-only indexes/projection.

Combined metadata TDD RED under the same third lane: `CARGO_TARGET_DIR="$PWD/target" CARGO_BUILD_JOBS=4 nice cargo test --locked --all-features --test combined invitation_confirmation_inbox_metadata_distinguishes_lazy_timed_and_legacy`: exit101, compile1m09s, runtime0.12s.1 selected,0 passed,1 failed,0 ignored,597 filtered. Failure at tests/store/lazy_inbox.rs:1228: serialized confirmation Null vs expected lazy. No compiler diagnostics. This build exceeds incremental1-minute budget by9s. Lane released immediately before metadata source changes.

Optional additive metadata implementation now exists in both `InboxBatchItem::Invitation` and `InboxBatchV2Item::Invitation`: `confirmation: Option<commands::InvitationConfirmation>`, `deadline_at: Option<UtcMillis>`, both serde default + omit None. Historical policy-absent rows omit both fields exactly; modern Lazy reports policy and omits deadline; modern RequireAck reports original frozen deadline. Both bounded query producers point-read these immutable columns by invitation ID after existing bounded selection. `output_compact` carries fields through its V2-to-V1 conversion; display behavior stays with Task8. Two existing test constructors in tests/cli/watch.rs and tests/cli/lazy_display.rs mechanically initialize None. Subsequent metadata GREEN and lint evidence are recorded below.

## Canonical wake and warning trace

`control::invite` records original caller and policy only at episode creation, skips wake_work for Lazy, and returns the existing pending episode before any duplicate creation/policy updates. Central `schema::record_overdue_inner` makes Lazy deadline absent even when invoked outside the scanner. `attention::pending_confirming_invitations` uses confirming-only partial indexes before bounded LIMIT; `attention::wake_seat_attention`, `wake::oldest_ordinary_at`/batching and `seats::mod_seat_view` use that projection. Natural digest/inbox discovery keeps all pending invitations, including Lazy.

On a timed invitation warning, `schema::record_overdue_inner` point-reads immutable inviter, inserts warning_recipients for the nonretired origin with the warning publication decision sequence, and enqueues bit4 wake work. Projection triggers place this recipient in the bounded programmatic warning set, even after leaving. `attention::warning_wakes_seat` checks the uncleared invitation condition and exact immutable origin independently of membership and informational-offer frontier; existing guards still decide launch/availability. Pending notice selection retains existing per-binding committed-offer settlement semantics. The warning's open condition keeps it waking after a prior informational offer. Other observers stay passive.

On terminal acceptance/rejection/cancellation/retirement, the central clear path uses the original condition invitation ID to project the origin's clear notice directly, without wake_work; `warning_wakes_seat` sees no open condition for the clear event. Existing audience projections still provide passive observer notices. Native launch/registration never accepts or restarts the original invitation timer. Regression exercises both unavailable and real registered recipient paths, due equality and late explicit acceptance, left original inviter open/clear behavior, duplicate caller policy freeze, Lazy rejection, finite250 backlog continuation amid earlier-deadline arrivals, physical future/Lazy bounds, restart progress, and actual linked public validation.

## Temporary linked policy guard

`src/store/topology_handoff/attachment.rs` accepts linked historical Invite only when confirmation is None, in the existing exact payload predicate. Ordinary canonical Invite Some is activated separately. This guard must be replaced by Task10 frozen policy comparison; until then no newly advertised capability silently authorizes unfrozen linked policy. The real main linked-phase validation test checks both Some modes fail OperationPayloadMismatch before total_changes changes.

Fourth assigned lane metadata GREEN: `CARGO_TARGET_DIR="$PWD/target" CARGO_BUILD_JOBS=4 nice cargo test --locked --all-features --test combined invitation_confirmation_inbox_metadata_distinguishes_lazy_timed_and_legacy`: exit0, compile1m06s, runtime0.10s,1 selected/1 passed/0 failed/0 ignored/597 filtered. Both V1 and V2 producers exercised in the single regression. Build exceeds1-minute budget by6s.

Compatibility selection: `CARGO_TARGET_DIR="$PWD/target" CARGO_BUILD_JOBS=4 nice cargo test --locked --all-features --lib protocol::output::tests::`: exit0, compile1m28s, runtime0.01s,31 selected/31 passed/0 failed/0 ignored/3575 filtered. Covers historical missing optional invitation metadata deserialization, compact rendering, JSON envelopes and routing. No compiler diagnostics. Build exceeds1-minute budget by28s; only a documentation placement correction separates this source from metadata GREEN. No unrelated lifecycle rerun since its current71-pass run. Subsequent required clippy evidence is recorded below.

## Remaining integration obligations and limits

- Task7 consumes the cursor API for immediate successful-page continuation and shared fair driver; its cross-class250+ formula/fairness/native-blocked scenarios are not claimed by these isolated invitation store tests.
- Task8 must interpret the new optional metadata in the two identified client attention predicates and implement user-facing defaults/presentation. Producer support is active, client semantics integrate downstream.
- Task10 replaces the temporary linked confirmation-is-None guard with frozen policy comparison.
- Migration0030 is provisional; parent must map/renumber it with other additive task migrations and review schema hot-file integration.
- Default-feature/final branch readiness and post-roast integration checks are parent-owned; no full suite run per ht-zo4 instruction.
- Compile timings honestly exceed the1-minute budget on some rebuilt artifacts (reported above); the focused lifecycle correction warm run was31.37s. Required lint timing is reported separately when finished. No test timeout/budget was weakened.

Required lint: `CARGO_TARGET_DIR="$PWD/target" CARGO_BUILD_JOBS=4 nice cargo clippy --locked --all-targets --all-features -- -D warnings`: exit0,39.50s, no diagnostics, meets1-minute lint budget. `cargo fmt --check`: exit0. Final `git diff --check`: exit0. Cargo lane released before report/commit bookkeeping. All Cargo sessions exited; no owned daemon, Herdr test server or persistent helper children were spawned. The output compatibility subprocess test completed its zsh invocation normally. No foreign process was stopped, real config touched, full suite run, push/stash, merge, or bead closure performed.

## Changed files

- `TRUST-POLICY.md`: immutable modern episode/timing contract, fresh required-over-Lazy episode and A7 origin audience.
- `migrations/0030_invitation_lifecycle.sql`: three additive partial indexes.
- `src/store/schema.rs`: provisional migration/audit, effective Lazy deadline, original inviter open/clear projections.
- `src/store/control.rs`: canonical modern policy activation, frozen origin, event/wake semantics.
- `src/store/invitation_due.rs`: ordinal finite high-water API and bounded indexed traversal.
- `src/store/attention.rs`: confirming-only windows and original inviter open-condition wake predicate.
- `src/store/effective.rs`: exact historical/native origin lookup.
- `src/store/wake.rs`: confirming-only native batching/oldest timing.
- `src/store/seats.rs`: confirming-only mod fingerprint attention.
- `src/store/service_controls.rs`: required-over-Lazy fresh timed episode.
- `src/store/queries.rs`: absent Lazy diagnostic deadline; optional V1/V2 producer metadata.
- `src/protocol/results.rs`: additive omitted-by-default invitation policy/deadline result fields.
- `src/protocol/output_compact.rs`: mechanical metadata carry-through/default rendering.
- `src/daemon/control.rs`: invitation capability advertised through existing daemon chain.
- `src/store/topology_handoff/attachment.rs`: temporary linked Some refusal pending Task10.
- `tests/protocol/capabilities.rs`: active daemon capability regression.
- `tests/store/control.rs`: canonical create/registration/late acceptance/rejection/duplicate/service episode tests.
- `tests/store/invitation_due.rs`: Lazy, inviter, bounded physical work, finite arrivals and restart tests.
- `tests/store/lazy_inbox.rs`: both producer versions' modern/legacy metadata regression.
- `tests/store/schema.rs`: mechanical additive index catalog count.
- `tests/store/topology_attachment_guard.rs`: real linked public validation before-effects regression.
- `tests/cli/watch.rs`, `tests/cli/lazy_display.rs`: mechanical None constructor adaptation only.

Self-review checked full owned diff, scope, exact scanner SQL predicate and ordinal ORDER/LIMIT, duplicate origin freeze, legacy omission, warning open/clear wake distinction, service cancellation ownership, canonical-only historical attribution, capability/linked activation boundary, migration catalog exactness and all mechanical constructors. New tests use existing isolated fixtures and no new test target/process-wide state. All observed owned failures have subsequent GREEN evidence. Integration obligations and timing limitations are retained above for fresh review.

Final commit: `ae922aba412bf69f457818ecba8341e224f339b4` (`Activate immutable invitation confirmation lifecycle`),23 files changed. Own branch remains `task-ht-0lo.2.2`; final `git status --porcelain=v1` output empty. Sandbox initially denied own worktree index.lock; exact CLI-only git add/commit escalation completed successfully under preauthorization. Report is outside task branch at the parent-authorized integration run path and is intentionally not included in this commit. No merge or bead close performed.

## Review fix round1: ADDRESSED I1

Fresh review `task-6-review.md` found one Important defect: informational offer settlement removes projected inviter warning candidates from actual native/mod attention before the correct `warning_wakes_seat` predicate can act. Earlier tests covered pre-offer attention and passive clears, and therefore did not establish the documented post-offer wake independence. Read receiving-code-review skill and full review; verified `seat_pending_warnings` notice-frontier filtering and native `load_candidate` warning_offer suppression. This report corrects the earlier overclaim.

Parent authorized additive `migrations/0031_invitation_warning_origin.sql` and admitted schema/attention/wake/effective surfaces for a bounded canonical open-origin projection. Origin freezes per newly opened episode; backfill only currently open invitation conditions, using the source invitation's stored inviter or exact canonical creation event actor with same-instance evidence. Unknown origins stay null. No current membership inference, ACK creation, changed informational settlement, historical event rewrite or old migration rewrite.

Tests-only RED package: actual native and mod post-committed-notice-offer regressions in tests/store/control.rs, and actual1..30 historical migration-prefix upgrade regression in tests/store/invitation_due.rs. Native fixture checks actual wake_seat_attention, load_candidate.has_actionable_work and cooperative reserve admission after exact committed informational settlement; mod fixture checks fingerprint.other_pending. Both retain a left original inviter, assert already offered notices stay settled, use explicit late accountable recipient acceptance, and require native/mod attention to disappear while a fresh passive clear stays deliverable. Prior-schema fixture covers stored origin, exact legacy event origin, unknown absence, unchanged event bytes and a fresh episode with different origin. No production source changes preceded assigned RED.

Fix-round1 actual RED (before production): `CARGO_TARGET_DIR="$PWD/target" CARGO_BUILD_JOBS=4 nice cargo test --locked --all-features --lib open_condition_survives_committed_notice_offer`: exit101, compile1m27s, runtime0.19s;2 selected/0 passed/2 failed/0 ignored/3607 filtered. Native actual latest_warning_seq was absent; mod actual other_pending was0 instead of1. `... --lib prior_schema_upgrade_preserves_exact_open_origins`: exit101, build0.04s, runtime0.83s;1 selected/0 passed/1 failed/0 ignored/3608 filtered. The real schema1..30 reopen lost stored origin's open condition attention. No setup/compile failures or weakened assertions. Lane released before production; no owned children.

Fix production adds0031 with nullable paired invitation_originator_instance_id/invitation_originator_seat_id frozen on each warning_conditions open episode, partial index `(invitation_originator_instance_id,invitation_originator_seat_id,ordinal)` WHERE invitation kind, clear NULL, origin seat NOT NULL, and immutable-origin trigger. Backfill only open conditions with exact source invitation and canonical warning source/thread; source origin is stored inviter or exact matching native invitation-creation actor, same-instance seat only. Unknown stays NULL. Migration changes no historical events or old SQL migration bytes. New episodes resolve their own origin at central creation; old closed episode addresses remain immutable.

`attention::open_invitation_origin_warnings` point-reads a live seat's instance, then selects one WINDOW-sized index range by instance/origin/ordinal, joining only bounded message primary keys for publication offsets. `wake_warnings` merges this separate source after narrowing the informational pending set, so both actual wake_seat_attention and mod_seat_view retain it independently of notice_frontier. `wake::load_candidate` suppresses legacy warning_offer gating when the independent origin source is still open. Exact informational settlement/page behavior remains unchanged. Clearing automatically removes the condition from the partial index; clear notice stays passive. No repeated informational notice or ACK obligation was added.

Independent SQLite preflight executed all31 migrations foreign_keysON; exact new index/trigger sql catalog comparison passes. EXPLAIN actual bounded origin selection shows SEARCH warning_conditions USING INDEX warning_conditions_invitation_origin(instance=? AND seat=?), bounded derived row scan, messages primary-key lookup; no TEMP B-TREE/sort. New changed files this round: migrations/0031_invitation_warning_origin.sql, src/store/{schema,attention,wake}.rs, TRUST-POLICY.md, tests/store/{control,invitation_due,schema}.rs. All mapped/approved. GREEN and normal gate pending.

Origin immutability distinction: `warning_conditions_invitation_origin_immutable` is BEFORE UPDATE OF the paired origin fields and rejects NEW instance/seat differing from OLD, whether the existing condition is open or closed. It never rewrites an old episode. The canonical new condition INSERT resolves `source_invitation` through `effective::invitation_originator`; fresh episode creation is an INSERT with its own resolved address, not mutation of the old address. The prior-schema regression closes the old stored-origin condition, creates a new invitation with different recorded inviter, runs the real due scanner, and checks the new condition's origin is that new inviter. Unknown backfilled evidence stays paired NULL. No state from the old episode is copied to the new origin.

Fix-round1 GREEN assigned lane, frozen production:

- `... nice cargo test --locked --all-features --lib open_condition_survives_committed_notice_offer`: exit0; compile1m12s, runtime0.16s,2 selected/2 passed/0 failed/0 ignored/3607 filtered. Includes actual native candidate and cooperative reserve admission, actual mod fingerprint, exactly settled old notice and passive late-accept clear. Compilation exceeds incremental budget12s.
- `... --lib prior_schema_upgrade_preserves_exact_open_origins`: exit0; build0.05s, runtime0.33s,1 selected/1 passed/0 failed/0 ignored/3608 filtered. Real historical prefix1..30, stored/legacy/unknown origin handling, original event bytes and fresh episode origin all pass.
- `... --lib invitation_`: exit0; build0.03s, runtime1.77s,75 selected/74 passed/0 failed/1 pre-existing ignored/3534 filtered. Same `pending_invitation_axis_is_flat` identity as above; no new skip.
- `... --lib store::attention::tests::`: exit0; build0.04s, runtime1.36s,32 selected/17 passed/0 failed/15 pre-existing ignored/3577 filtered. Active projection/oracle/capped count/saturated notice/flat-cost checks pass. Existing ignored identities all under store::attention::tests: axes_are_flat_at_a_hundred_thousand; check_in_reads_are_flat_at_a_hundred_thousand_acked_receipts; check_in_reads_are_flat_at_a_hundred_thousand_settled_warnings; check_in_reads_are_flat_in_production_settled_warnings; check_in_reads_are_flat_in_production_shaped_acked_receipts; delivered_programmatic_axis_is_flat; digest_cost_is_flat_at_a_hundred_thousand_acked_receipts; digest_cost_is_flat_from_a_thousand_to_a_hundred_thousand_threads; offer_settlement_writes_are_constant; offer_settlement_writes_are_constant_at_three_hundred_thousand; pending_invitation_axis_is_flat; pending_receipt_axis_is_flat; pending_warning_count_is_flat_at_a_hundred_thousand_threads; scale_invariance_inventory_of_canonical_effective_scans; undelivered_programmatic_axis_is_flat. These release/report-only benchmark ignores are unchanged baseline; no newly ignored tests.

No compiler diagnostics, weakened assertions or owned children. Metadata compatibility and required normal gate still running. Mechanical latest catalog counts become144 indexes/184 triggers (one new origin index/immutable trigger),74 tables unchanged; actual historical prefix migrations remain untouched.

Remaining fix-round focused verification:

- `CARGO_TARGET_DIR="$PWD/target" CARGO_BUILD_JOBS=4 nice cargo test --locked --all-features --test combined invitation_confirmation_inbox_metadata_distinguishes_lazy_timed_and_legacy`: exit0; compile22.76s, runtime0.10s,1 selected/1 passed/0 failed/0 ignored/597 filtered. Legacy omission and modern Lazy deadline absence/RequireAck frozen deadline remain intact with schema31.
- After metadata compile exited, corrected only the admitted latest catalog trigger literal183->184. Audited all current/latest local30/31 table/index/trigger/version guards before the next compile:74 tables,144 indexes,184 triggers; LATEST_VERSION31; real historical prefixes retained.
- `... nice cargo test --locked --all-features --lib absorption_main24_and25_to26_preserves_diagnostics_and_matches_fresh_catalog`: exit0; compile17.32s, runtime0.44s,1 selected/1 passed/0 failed/0 ignored/3608 filtered. Actual published historical24/25 upgrade matches fresh latest catalog without diagnostic/history rewrite. No serial missed-literal rebuild/failure.
- Independent Python SQLite check executes all31 migrations with foreign_keysON, inserts an open condition, verifies changing its inviter fails with exact immutable-origin error, closes it, inserts a fresh condition with distinct inviter, and confirms old/new retained origins. Exit0. The real Rust prior-schema/scanner regression separately establishes canonical source resolution for a fresh episode.

Required frozen all-target/all-feature Clippy currently running; no source edits during compilation.

First fix-round required Clippy exited101 with exactly one owned style diagnostic (`field_reassign_with_default`, src/store/attention.rs Gather.steps after Default construction); both lib/lib-test emitted that same cause. The failed compiler did not print final elapsed, so no unmeasured duration is claimed. All focused behavioral/catalog checks above had passed. Once all Clippy jobs exited, parent explicitly retained/regranted the lane for only the equivalent Gather initializer rewrite (steps set within struct initializer) and corrected mandatory lint/fmt. No lint suppression, semantic changes, extra behavioral reruns, or assertions weakened. Corrected `cargo fmt --check` exits0. Final corrected lint evidence follows.

Final corrected mandatory lint: `CARGO_TARGET_DIR="$PWD/target" CARGO_BUILD_JOBS=4 nice cargo clippy --locked --all-targets --all-features -- -D warnings` exits0 in30.20s with zero diagnostics, meeting1-minute lint budget. `cargo fmt --check` and final `git diff --check` exit0. Only the equivalent Gather initializer rewrite follows focused GREEN; parent explicitly requested no unnecessary behavior rerun. Cargo lane released immediately after final lint, before report/commit bookkeeping. No owned child/server/daemon was spawned; all Cargo processes exited.

ADDRESSED I1 committed as `50c397f4196002cca4e8ee65557da049f974fd02` (`Preserve open inviter wake after informational offer`),8 approved files changed. Final own `git status --porcelain=v1` empty; branch task-ht-0lo.2.2 remains isolated, no rebase/merge/bead closure. Original BASE remains a04d1a41838fdcfee18b8cb12ba0c1595cd103f3. Self-review rechecked complete production diff, exact origin backfill/source/thread/instance predicates, immutable trigger, bounded query plan and notice/admission separation; real tests now cover the previously missed post-offer cutpoint. Fresh scoped rereview is controller-owned and has not been claimed CLEAN by the implementer. Migration0031 is provisional and must be renumbered in serial integration with0030; unchanged historical1..30 fixture remains truthful at this local reviewed tip and must preserve its exact historical prefix semantics during integration. Task7/8/10 downstream obligations and prior timing disclosures remain unchanged.

## Review fix round2: I1 remains open (tests-only preparation)

Fresh scoped `task-6-rereview-1.md` found the preceding ADDRESSED I1 implementation still incomplete: ordinary native discovery first calls seat_has_pending_rows, whose six old WAKE_PROBES omit the new origin source. Thus direct native/mod attention and reserve tests passed while production native discovery could skip the otherwise-idle inviter. Read the full final review and verified actual discover_wake_candidates gate; no other finding was reported. Previous commits/base retained.

New tests-only regression `invitation_confirmation_discovery_survives_committed_notice_offer_and_completed_attribution` reuses canonical create/invite/left-origin fixture, drains the actual warning-attribution work job to complete, settles the exact committed informational notice, then calls actual discover_wake_candidates before direct native/load/reserve checks. It requires the original inviter be discovered with no unrelated pending work and disappear after accountable late acceptance while the passive clear stays deliverable. Existing native/mod regression assertions remain. No production edits before assigned actual RED. Planned fix is only one bounded instance/seat-leading open-origin EXISTS probe, retaining current host/composer/human/native admission guards and informational delivery settlement; no migration or ACK changes. Mechanical access-path inventory test count update requested for tests/store/wake_discovery_cost.rs.

Round2 RED evidence: first exact filter compile failed E0277 because the test read SQLite MAX(ordinal) directly as u64 (rusqlite FromSql supports i64). This was a test-only setup error, not behavioral evidence; no production changed. Under explicit parent authorization, corrected only i64 read + checked u64 conversion, audited fixture types/prerequisites, then reran `CARGO_TARGET_DIR="$PWD/target" CARGO_BUILD_JOBS=4 nice cargo test --locked --all-features --lib discovery_survives_committed_notice_offer_and_completed_attribution`: exit101, compile27.22s, runtime0.12s,1 selected/0 passed/1 failed/0 ignored/3609 filtered. Meaningful failure is the exact actual native-discovery missing-origin assertion after completed canonical warning attribution and exact committed informational settlement. No compiler diagnostics on corrected build, no assertion weakening or new ignore, no owned children. Cargo lane released immediately before production changes.

Round2 production is one extra WAKE_PROBES entry pinned to warning_conditions_invitation_origin: EXISTS WHERE origin instance=(seat's canonical PK instance lookup), origin seat=?1, invitation kind, clear NULL, origin NOT NULL. Discovery remains seven constant indexed probes, stopping on first hit, with unchanged live/human seat walk,100 examined-seat cap, canonical actionability/continuity/admission, informational notice settlement, and all existing host/composer guards. No migration or ACK contract changed. Mechanical tests/store/wake_discovery_cost.rs inventory changes8->9 paths (seven pending probes plus live/human paths), with budgets/flat-cost assertions preserved; source/test expansion explicitly approved by parent.

Independent Python SQLite EXPLAIN on all31 migrations shows SEARCH warning_conditions USING INDEX warning_conditions_invitation_origin(instance=? AND seat=?), scalar seats PK lookup by id, and no retained-history scan/temp sort. fmt/check and diff/check pass. Frozen final GREEN selection requested for all actual native/mod post-offer/accept regressions, exact indexed-plan/flat-cost suite, unchanged working/draft/human/hold guards and mandatory lint. No further Cargo launched before grant.


### Round2 final GREEN and ADDRESSED I1

The preceding round1 ADDRESSED claim was disproved by fresh scoped review; it remains NOT ADDRESSED at50c397f4. Round2 closes the actual native discovery gate in addition to the already verified direct native/mod paths. Source stayed frozen through the following final checks. All test commands use `CARGO_TARGET_DIR="$PWD/target" CARGO_BUILD_JOBS=4 nice cargo test --locked --all-features --lib <filter>` in the existing own target, with authorized scoped CLI escalation. No profile/cache changes, default-feature gate or full suite.

| Filter | Passed / failed / ignored | Build | Runtime |
| --- | --- | --- | --- |
| committed_notice_offer |3 /0 /0 |55.03s |0.38s |
| store::discovery_cost:: |3 /0 /0 |0.08s |1.25s |
| typed_draft_defers_poke_and_ordinary_wake |1 /0 /0 |0.04s |0.00s |
| cooperative_overdue_warning_wakes_idle_native_agent_exactly_once |1 /0 /0 |0.05s |1.05s |
| blocked_or_human_input_target_is_never_prompted |1 /0 /0 |0.05s |0.00s |
| pre_send_errors_are_refusals_by_class_and_never_reach_submit |1 /0 /0 |0.06s |0.00s |
| unresolved_recovery_hold_blocks_candidate_continuity_even_with_fresh_target |1 /0 /0 |0.04s |0.03s |
| human_bound_seat_ |5 /0 /0 |0.04s |0.16s |

Total16 selected/16 passed/0 failed/0 ignored. No newly skipped tests; earlier pre-existing benchmark ignore identities remain unchanged and are not claimed as executed round2 coverage. The first filter includes all three actual discovery/native/mod post-offer regressions, finished warning-attribution work, left original inviter and late accountable acceptance with passive clear. Cost selection includes every_access_path_prepares_indexed_by, wake_discovery_is_flat_in_settled_history and wake_discovery_scales_with_live_seats_only; original budgets unchanged. The existing cooperative scheduler test exercises idle/done versus working/blocked/shell; blocked-target test includes ActiveTurn, ApprovalOrQuestion, HumanInput and Unknown. The pre-send-errors test covers HostUnavailable/DeadlineExceeded and other refusal classes before submit. Typed draft, recovery hold and all five human-seat matches establish the unchanged admission guards with positive executed counts.

Interruption/recovery: completed committed_notice_offer evidence was recovered without rerunning it. The first discovery_cost launch was rejected by automatic approval review due account usage limit; it did not execute and was not behavioral evidence or a safety rejection. After explicit resume, audited own HEAD and the exact three modified files; read-only process audit found no build/test process with this task target path. Resumed the remaining checks under the parent-regranted exclusive lane. No foreign process was stopped or modified.

Mandatory `CARGO_TARGET_DIR="$PWD/target" CARGO_BUILD_JOBS=4 nice cargo clippy --locked --all-targets --all-features -- -D warnings` exits0 in32.62s, zero diagnostics. `cargo fmt --check` and `git diff --check` exit0. Final build55.03s and lint32.62s meet their respective1-minute budgets; earlier overruns remain disclosed above. Cargo lane released immediately after final results, before commit/report bookkeeping, for Task9. No owned persistent children/servers/daemons; all owned Cargo/test commands exited.

Round2 committed as `19afa478b8b71141ea958d96278302a808ef9ed0` (`Discover open inviter conditions after notice settlement`), exactly three approved files: src/store/attention.rs, tests/store/control.rs, tests/store/wake_discovery_cost.rs. Full HEAD differs from original BASE; prior commits50c397f4 and ae922aba retained without rebase. Final git status --porcelain=v1 empty. No production changes after GREEN, no additional migration or notice settlement/admission changes. Report only is written at the explicitly authorized parent run path outside own worktree. Fresh scoped rereview and integration remain controller-owned; ADDRESSED I1 here is the implementation/evidence claim pending independent CLEAN review. Existing downstream Task7/8/10 obligations and provisional migration renumbering remain unchanged.
