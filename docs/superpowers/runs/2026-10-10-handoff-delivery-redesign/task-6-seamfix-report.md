# Task 6 bounded rebase seam candidate

Status: READY_FOR_EXACT_TIP_GATE at corrected `f48857d73b18c76b8ee35b0f8cab5589a6f4ea5c`. Fresh scoped rereview1 is Spec CLEAN/Quality CLEAN, I1/I2/I3 ADDRESSED; corrected focused43tests pass and scoped leakcheck is clean. Initial candidate/static CLEAN remain superseded history. Coordinator merge_2 owns final exact-tip clippy/fmt gates.

## Exact range and disposition

Own worktree: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.2.2`.
Own branch: `task-ht-0lo.2.2`.
Integration BASE: `eac576409d850720b62ac3363598ed6edb0fe730`.
Frozen clean HEAD: `3f9167bd90cf19ddbd0b75f1282674628f607e43`.
Original approved range: `a04d1a41838fdcfee18b8cb12ba0c1595cd103f3..19afa478b8b71141ea958d96278302a808ef9ed0`.

All three original commits survive in order, followed by one bounded composition commit:

- `ae922aba` → `782a46cfc0a0c99a690d36c2daba815a19074787`, Activate immutable invitation confirmation lifecycle.
- `50c397f4` → `2186877fe93e93d0f004c7c3b1eccc30857f1f6d`, Preserve open inviter wake after informational offer.
- `19afa478` → `97b485fb7d9567f459f29add86979ac552d0cd84`, Discover open inviter conditions after notice settlement.
- `3f9167bd90cf19ddbd0b75f1282674628f607e43`, Compose invitation lifecycle after receipt and handoff migrations.

Conflict inventory: three conflicted file occurrences across two paused commits, two distinct files. Commit1 conflicted in `src/store/schema.rs` and `tests/protocol/capabilities.rs`; commit2 conflicted in `src/store/schema.rs`; commit3 applied cleanly. Initial five overlap files were TRUST-POLICY.md, src/store/effective.rs, src/store/schema.rs, tests/protocol/capabilities.rs, tests/store/schema.rs. TRUST/effective/schema-test overlaps auto-composed; they were inspected rather than treated as approval by Git alone.

## Migration and contract composition

Immutable global migrations1..32 remain byte-for-byte equal to BASE: invitation29, receipt30, handoff31, receipt-backfill32. Approved Task6 SQL is renamed without any byte change: local0030_invitation_lifecycle.sql becomes0033_invitation_lifecycle.sql; local0031_invitation_warning_origin.sql becomes0034_invitation_warning_origin.sql. Python Git-object comparisons checked all34 files against their proper integration/original references.

Latest is34. Fresh initialization still commits0001..0029 at version29. Receipt30 and handoff31 remain independent sequential FK-off wrappers; a direct function-body comparison confirms `migrate_receipts_v29_to_v30` and `migrate_handoff_v30_to_v31` retain exact BASE bytes. Backfill32 retains its table/marker transaction and audit. Additive33 runs only for original version<=32, with its lifecycle-index audit before version33 commit. Additive34 runs only for original version<=33, with origin audit before version34 commit. Existing32/33/34 cannot recapture receipt30 cutover or recreate backfill32 progress. No nesting or FK-policy changes were introduced.

TRUST retains the send-time receipt policy and accepted mixed-version limit, plus immutable invitation lifecycle and independent open-inviter waking under existing guards. All approved non-schema Task6 source added/deleted lines were compared to the original approved range and remain identical, including effective.rs's exact-origin helper; inherited Task5 producer code is untouched. Lazy/RequireAck/legacy lifecycle, finite scanner semantics, original inviter attribution, actual discovery probe, mod/native offer independence and admission checks remain the approved implementation. No notice/guard/origin heuristic changes.

Daemon capability composition preserves send-time receipts in ADVERTISED and dynamically adds active invitation confirmation. Both conflicting tests survive. Bare-daemon exact expected rows add the dynamic invitation flag while retaining receipt capability; recorder-backed expected rows use the same dynamic append. The ADVERTISED constant inventory remains unchanged because invitation is dynamically chained. Existing active invitation lifecycle tests remain present.

## Fixture audit and admitted surfaces

Controller explicitly admitted latest-only fixture updates in tests/store/topology_handoff.rs, topology_handoff_schema.rs, lazy26_prerequisite.rs, lazy_schema.rs, handoff_fences.rs and actual tests/integration/channel_activation.rs. They now assert34 after initialization. Topology's genuine migration-image cache replays through34 and additionally exercises true32/33 inputs; true historical28/29/30/31 and older cutpoints remain real prefixes. No latest database was relabeled as historical.

`tests/store/schema.rs` already auto-composed the exact fresh catalog guards to76 tables/144 indexes/184 triggers. Independent SQLite replay confirms these counts. Existing historical schema/rootpage assertions remain intact; the new migrations add no table/rebuild. `tests/store/effective.rs` uses LATEST_VERSION for current-version assertions and retains true28/29/31 rollback assertions. Its prior31→latest preservation test now excludes exactly the three new lifecycle indexes and origin index/trigger alongside its preexisting backfill exclusion. It derives the exact expected warning_conditions DDL by executing the real old table declaration plus only migration34's two ALTER COLUMN additions on a private in-memory connection; it compares the original rootpage and every other old catalog definition exactly. Failed32 still compares the unmodified old catalog. Existing row snapshots, cutover12 and backfill marker snapshots remain unchanged.

`invitation_confirmation_prior_schema_upgrade_preserves_exact_open_origins` now builds real global0001..0033, asserts33 after physical reopen, and upgrades through public writer initialization to34. Actual lifecycle indexes/columns exist before upgrade. Stored/exact canonical legacy/unknown origin assertions, unchanged old event bytes and fresh episode's distinct origin remain intact.

New `invitation_confirmation_actual32_upgrade_rolls_back_reopens_and_preserves_receipt_progress` executes real0001..0032, seeds host before receipt30 so cutover12 is real, changes current decision sequence99, and retains a modern invitation plus nonzero physical/prepared repair progress2/7 and3/9. A conflicting second lifecycle index rolls back33 including the first created index and leaves actual32. A conflicting origin index fails after both ALTERs, rolls back those columns and leaves committed33. Each case physically reopens its failure state, removes its owned obstruction, initializes/repeats/reopens34 and verifies original invitation timing/policy/origin, cutover12 and exact progress. FK ON/autocommit remain asserted. No old SQL is modified and no worker/snapshot runtime behavior changes.

## Fresh evidence and limits

- Python SQLite replay of real1..34 succeeds with foreign_keys OFF and ON.
- Independent SQLite statement-level obstruction/rollback/retry checks pass at both32 and33, preserving cutover12 and marker `(1,2,7,3,9,0)`.
- Independent catalog count proof:76 tables,144 indexes,184 triggers.
- Git-object immutable SQL comparison: all1..32 equal BASE; renamed33/34 equal approved Task6 originals.
- Approved non-schema source added/deleted-line comparison succeeds, including effective overlap.
- Existing receipt/handoff FK wrapper function bodies equal integration BASE exactly.
- Direct rustfmt --edition2024 checks of all ten seam-modified Rust files exit0; full-range git diff --check exits0.
- `git status --porcelain=v1` is empty at frozen HEAD.

These are static/SQLite checks, not compiled Rust evidence. Original Task6 historical GREEN and round2 Spec CLEAN/Quality CLEAN/I1 ADDRESSED belong to the original approved tip; this rebased candidate and new32→34 regression remain uncompiled/unrun. No fresh lifecycle/native/discovery/clippy/default-feature/full-suite claim is made. Coordinator must obtain its fresh seam review and focused exact-tip Cargo validation before integration readiness. Task7 shared fairness/coalesced receipt discovery, Task8 client attention rendering, and Task10 linked frozen-policy replacement remain downstream responsibilities.

## Review package

Generated from this exact own worktree with the installed helper via bash and explicit output:

`bash /Users/alepar/.codex/skills/subagent-driven-development/scripts/review-package /Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/task-6-brief.md eac576409d850720b62ac3363598ed6edb0fe730 HEAD /Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/task-6-seam-review-package.diff`

HEAD resolved to the frozen hash above. Package: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/task-6-seam-review-package.diff`,167136bytes,4commits, complete diff with10context lines and commit/stat inventory. Net32files,1826insertions/108deletions. Shared integration writes are only this report and the requested review package; no run-state or integration-ref commit.

All owned commands ran synchronously and exited. No test daemon, Herdr server, helper child or other persistent process was started. No Cargo, shared Herdr/thread action, real configuration write, new/tmp target, cache copying, merge, push or stash occurred. CLI-only Git escalation was used for own worktree staging/rebase/commit; no approval rejection. Existing own persistent target remains the only eventual Cargo target.


## Bounded seam correction after actual build exposed lost runtime hunks

The preceding initial candidate history remains recorded, but its preservation/completion claim is superseded. Initial seam static Spec CLEAN/Quality CLEAN was withdrawn after actual compilation exposed a real unused `invitation_originator` warning. The author incorrectly resolved schema conflicts using the full integration stage2 file plus migration blocks, dropping nonconflicting approved runtime changes. The earlier source comparison excluded schema and therefore did not detect this loss. This is a bounded rebase seam correction, not an additional original-task implementation round. No lint was weakened.

Corrected clean HEAD: `f48857d73b18c76b8ee35b0f8cab5589a6f4ea5c`.
BASE remains `eac576409d850720b62ac3363598ed6edb0fe730`.
Fix range: `3f9167bd90cf19ddbd0b75f1282674628f607e43..f48857d73b18c76b8ee35b0f8cab5589a6f4ea5c`.
Commit: Restore approved invitation warning runtime after schema composition. Exactly2files: schema.rs24insertions/8deletions, control.rs161new test/helper lines. All original3commits and prior seam commit retained.

### Actual detection and RED provenance

On the prior clean3f9167bd, exported `CARGO_TARGET_DIR="$PWD/target"`, `CARGO_BUILD_JOBS=4`, and unique `HT_LEAK_RUN_ID=1fa55930-e193-402e-a941-2eebe8d334b3`. Exact original39expression was stored in `/private/tmp/ht-task6-seam-filterset`, with29fully qualified `test(=NAME)` terms and one anchored suffix regex selecting10admission tests. Ran:

`nice cargo nextest list --locked --all-features --lib --test combined --test integration -E "$HT_SEAM_FILTER" --message-format json > /private/tmp/ht-task6-seam-list.json`

Exit0; compilation2m44s, exceeding the1minute build budget. The sandbox printed `nice: setpriority: Operation not permitted`; Cargo continued and was not restarted. Compilation emitted real dead_code warning for effective::invitation_originator in lib and lib-test. JSON confirmed exactly39matched:lib24,combined14,integration1. This was discovery/build evidence, not39tests GREEN.

Under explicitly authorized cached RED, before any source change, ran with the same own target/jobs/run-ID and CLI-only escalation:

`nice cargo nextest run --locked --all-features --lib -E 'test(=store::invitation_due::tests::lazy_invitation_is_discoverable_but_never_due_or_waking) | test(=store::control::tests::invitation_confirmation_discovery_survives_committed_notice_offer_and_completed_attribution)'`

Exit100; cached build0.06s; nextest runtime0.202s;2selected/0passed/2failed/0ignored (3646filtered). Meaningful Lazy failure at invitation_due.rs424: direct late overdue gateway retrospectively warned Lazy. Origin regression failed at control.rs10861: expected1original inviter notice recipient, actual0; this failure precedes its downstream actual discovery assertion and is not claimed as reaching that later cutpoint. These failures establish the lost schema effects. Full39run was paused. Scoped `scripts/check-no-leaked-processes --run-id 1fa55930-e193-402e-a941-2eebe8d334b3 --root "$PWD"` exited0, no leaked test processes. Lane released before source work/review.

### Complete original→rebased overlap mapping

All original schema diff hunks were enumerated, not only helper references: four migration/version hunks and six runtime hunks. The6runtime hunks were extracted from the approved original range into an owned5175byte patch, checked then applied atop inherited integration source without changing their added/deleted lines.

| Overlap | Original approved delta | Corrected composition and proof |
| --- | --- | --- |
| TRUST-POLICY.md | Immutable Lazy/RequireAck/required-service paragraph; A7 original-inviter independent waking/passive clear | Every original added/deleted line equals corrected BASE→HEAD delta. Existing integration receipt policy and accepted mixed-version limit remain inherited. |
| src/store/effective.rs | One exact canonical invitation_originator helper | Every original added/deleted line equals corrected BASE→HEAD delta; inherited Task5 code untouched. Corrected schema now calls it in both runtime producer paths. |
| src/store/schema.rs constants | Local lifecycle30,origin31,latest31 | Byte-identical SQL renamed33/34; global1..32 inherited; latest34. |
| src/store/schema.rs fresh branch | Original local fresh chain included lifecycle30/origin31 | Intentionally composed fresh1..29 transaction, then existing receipt30/handoff31 FK-off wrappers/backfill32, then additive33/34. Required wrapper/cutover boundaries preserved. |
| src/store/schema.rs supported range | Original18..31 | Composed18..34. Genuine historical inputs retain real prefixes. |
| src/store/schema.rs additive migrations/audits | Original lifecycle30 and origin31 transaction/version/audits | Mapped to33/34 with original-version<=32/<=33 guards. Both approved audit function bodies compare exactly after function-name and V30→V33/V31→V34 substitutions. |
| src/store/schema.rs runtime clear1 | Select condition_id with open condition | Exact original hunk restored. |
| src/store/schema.rs runtime clear2 | Original inviter passive clear recipient after enqueue | Exact original hunk restored, no wake enqueue. |
| src/store/schema.rs overdue1/2 | Invitation deadline Option with CASE Lazy→NULL; tuple passes Option | Both exact original hunks restored, preventing retrospective warning from direct settlement routes. |
| src/store/schema.rs overdue3 | Resolve exact canonical origin and INSERT frozen origin instance/seat | Exact original hunk restored. |
| src/store/schema.rs overdue4 | Original inviter warning recipient and wake_work enqueue | Exact original hunk restored. |
| tests/protocol/capabilities.rs | Active invitation capability regression | Original test body exact; integration receipt status guard retained. Only composed bare/recorder expected dynamic rows added, with receipt rows unchanged; ADVERTISED constant inventory retained. |
| tests/store/schema.rs | Exact index140→144 and trigger183→184 guards | Every original added/deleted line equals corrected BASE→HEAD delta; inherited table76 guard untouched. |

Automated comparison confirms all6schema runtime hunks' added/deleted lines equal the original approved range,24insertions/8deletions. The preceding4schema hunks are the explicit intentional version/migration adaptations enumerated above. Every other approved source delta was already compared, and none changes in this correction. Existing SQL/schema fixture/catalog checks are unchanged; no redundant full SQL replay was performed during correction.

### Narrow actual late Lazy regression coverage

New lib control tests use the existing canonical bound create/invite fixture, actual Lazy policy and time400000 beyond inherited physical deadline:

- `invitation_confirmation_late_lazy_accept_has_no_overdue_condition_or_wake` invokes actual accountable accept with exact invitation permit; asserts accepted state/time/actor/generation and joined membership.
- `invitation_confirmation_late_lazy_retirement_has_no_overdue_condition_or_wake` invokes actual canonical `begin_retirement` with the fixture's fresh effective target proof, then bounded `advance_retirement` until complete; asserts recipient_retired/time, retired seat and no synthetic acceptance.

Both assert no invitation overdue condition, no warn event sourced to that invitation, no origin wake_work, no origin warning attention and zero mod other_pending. They do not replace the actual existing late Lazy rejection or shared direct-overdue regression. Compile-sensitive setup was traced: hostbootb/epoch1, currents2targetgeneration1, no active snapshot so fresh target is NewerCurrentTarget; pending invited membership is traversed by the real retirement worker. No bare test children or shared Herdr usage.

Expanded future focused selection is43: original39plus existing directLazy and actual lateLazyReject2, plus new actual lateLazyAccept/Retire2. The39names/targets are recorded in the coordinator handoff; revised exact expression is in `/private/tmp/ht-task6-seam-filterset`. Corrected source is uncompiled/unrun; a fresh nextest list must confirm43before running after fresh review and lane grant. Required clippy/fmt exact-tip gates belong to merge_2 afterward. No additional Cargo was run after correction.

Direct rustfmt --edition2024 --check on both correction files and full-range git diff --check exit0; own git status empty. These establish format/static preservation only. No corrected lifecycle GREEN claim.

### Refreshed packages

Installed bash review-package helper generated both explicit outputs at corrected clean HEAD:

- Full BASE→corrected HEAD: `task-6-seam-review-package-1.diff`,179583bytes,5commits, complete10context diff/inventory.
- Prior candidate→corrected HEAD: `task-6-seam-fix-review-package.diff`,13435bytes,1commit, complete10context diff/inventory.

Both reside in `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/`. The prior167136byte package is retained as superseded history. Only report/packages are shared integration writes. All owned commands/processes exited; no shared-server/config changes, merge/push/stash, new target/cache or default-feature/full-suite work.


## Corrected exact-tip focused GREEN

No source changed after fresh scoped rereview1 approved the corrected candidate (Spec CLEAN/Quality CLEAN; I1/I2/I3 ADDRESSED). The coordinator granted sole Cargo lane after Task9 released its completed setup83GREEN run. All following Cargo calls used CLI-only require_escalated so nice priority and local test socket permissions were honored, with exact own persistent target/jobs4 and no cache/target copying.

Frozen HEAD before and after validation: `f48857d73b18c76b8ee35b0f8cab5589a6f4ea5c`; git status empty both times.
Fresh unique leak run ID: `4177790e-09b4-4b91-a1d0-0dce1d3f7839`.

Exact environment and commands, run from the exact own worktree:

```sh
export CARGO_TARGET_DIR="$PWD/target" CARGO_BUILD_JOBS=4 HT_LEAK_RUN_ID=4177790e-09b4-4b91-a1d0-0dce1d3f7839
HT_SEAM_FILTER="$(cat /private/tmp/ht-task6-seam-filterset)"
nice cargo nextest list --locked --all-features --lib --test combined --test integration -E "$HT_SEAM_FILTER" --message-format json > /private/tmp/ht-task6-seam-list-corrected.json
# Parse every testcase filter-match.status and require exactly43 matches before run.
nice cargo nextest run --locked --all-features --lib --test combined --test integration -E "$HT_SEAM_FILTER"
scripts/check-no-leaked-processes --run-id 4177790e-09b4-4b91-a1d0-0dce1d3f7839 --root "$PWD"
```

Exact filter expression (the owned temporary file is only a transport, this literal records the full selection):

```text
test(=store::invitation_due::tests::invitation_confirmation_prior_schema_upgrade_preserves_exact_open_origins) | test(=store::invitation_due::tests::invitation_confirmation_actual32_upgrade_rolls_back_reopens_and_preserves_receipt_progress) | test(=store::effective::tests::dormant_upgrade_actual_v31_backfill32_reopens_and_rolls_back_without_recapture) | test(=store::connection::tests::absorption_main24_and25_to26_preserves_diagnostics_and_matches_fresh_catalog) | test(=protocol::capabilities::tests::bare_daemon_advertises_legacy_capabilities_without_v2_recorder) | test(=protocol::capabilities::tests::v2_capability_is_negotiated_only_with_recorder) | test(=protocol::capabilities::tests::invitation_confirmation_capability_is_advertised_with_active_lifecycle) | test(=protocol::capabilities::tests::public_operation_status_cannot_bypass_send_time_capability_guard) | test(=store::control::tests::invitation_confirmation_native_open_condition_survives_committed_notice_offer) | test(=store::control::tests::invitation_confirmation_mod_open_condition_survives_committed_notice_offer) | test(=store::control::tests::invitation_confirmation_discovery_survives_committed_notice_offer_and_completed_attribution) | test(=store::discovery_cost::every_access_path_prepares_indexed_by) | test(=store::discovery_cost::wake_discovery_is_flat_in_settled_history) | test(=store::discovery_cost::wake_discovery_scales_with_live_seats_only) | test(=topology_handoff::schema_tests::registered28_fresh_and_genuine_historical_chains_reopen) | test(=topology_handoff::schema_tests::registered28_preserves25_and26_history_and_rolls_back_late_ddl_failure) | test(=topology_handoff::schema_tests::handoff_body_bounds_upgrade_preserves_existing_identity_bytes_and_keys) | test(=topology_handoff::schema_tests::handoff_bounds_upgrade_preserves_populated_completed_and_recovery_history) | test(=topology_handoff::schema_tests::handoff_bounds_upgrade_failure_restores_fk_and_retries_without_recapturing_receipt_cutover) | test(=topology_handoff::begin_installs_attempt_one_and_thread_protection_without_delivery) | test(=topology_handoff::current_main_fresh_and_canonical26_upgrade_to_topology28) | test(=lazy26_prerequisite::fresh_store_installs26_and_reopens) | test(=lazy26_prerequisite::historical25_store_upgrade_preserves_evidence_and_defaults_modes) | test(=lazy26_prerequisite::reopen_rejects_missing_and_altered_lazy_triggers) | test(=lazy26_prerequisite::reopen_rejects_missing_and_altered_lazy_table_and_indexes) | test(=lazy26_prerequisite::reopen_rejects_missing_or_weakened_mode_columns) | test(=lazy_schema::lazy_schema_upgrade25_defaults_existing_messages_to_ordinary) | test(=handoff_fences::handoff_actual22_upgrade_imports_already_committed_create_then_replays_without_unattached_fence) | test(=channel_activation::channel_activation_real_daemon_loads_fresh_21_and_22_and_owns_archival_lane) | test(/::(typed_draft_defers_poke_and_ordinary_wake|cooperative_overdue_warning_wakes_idle_native_agent_exactly_once|blocked_or_human_input_target_is_never_prompted|pre_send_errors_are_refusals_by_class_and_never_reach_submit|unresolved_recovery_hold_blocks_candidate_continuity_even_with_fresh_target|human_bound_seat_.*)$/) | test(=store::invitation_due::tests::lazy_invitation_is_discoverable_but_never_due_or_waking) | test(=store::control::tests::invitation_confirmation_lazy_rejection_still_publishes_attributed_notice) | test(=store::control::tests::invitation_confirmation_late_lazy_accept_has_no_overdue_condition_or_wake) | test(=store::control::tests::invitation_confirmation_late_lazy_retirement_has_no_overdue_condition_or_wake)
```

List exited0, compile54.88s with zero diagnostics, meeting the1minute incremental budget for the correction. JSON discovery counted lib28/combined14/integration1, exactly43matches. Selected run exited0, cached build0.05s, runtime4.157s;43passed/0failed/0selected ignored;4400filtered/skipped. No full suite/default-feature/clippy was run. The original2cached REDcases both passed at the corrected tip. Earlier2m44s broken-candidate build overrun remains disclosed above.

The new actual late Lazy accept and retirement fixtures each reached their truthful terminal assertions and no-condition/no-warn/no-native/mod-wake assertions. Existing actual late Lazy rejection and common direct-overdue gateway passed. Real32→34 obstruction/rollback/reopen/progress, real33→34 origins/events/fresh episode, prior31→latest exact catalog/rootpages and both capability list compositions passed. Actual native discovery after settled notice/completed attribution, native load/reserve, mod fingerprint and passive clear assertions passed alongside the unchanged10admission checks and indexed cost suite. This establishes only the selected bounded seam contracts, not downstream Task7 global discovery/fairness or Task8/10 obligations.

Exact discovered-and-executed test inventory:

herdr-threads (28):

- `protocol::capabilities::tests::bare_daemon_advertises_legacy_capabilities_without_v2_recorder`
- `protocol::capabilities::tests::invitation_confirmation_capability_is_advertised_with_active_lifecycle`
- `protocol::capabilities::tests::public_operation_status_cannot_bypass_send_time_capability_guard`
- `protocol::capabilities::tests::v2_capability_is_negotiated_only_with_recorder`
- `scheduler::poke_flow::human_bound_seat_costs_no_reservation`
- `scheduler::tests::blocked_or_human_input_target_is_never_prompted`
- `scheduler::tests::cooperative_overdue_warning_wakes_idle_native_agent_exactly_once`
- `scheduler::tests::pre_send_errors_are_refusals_by_class_and_never_reach_submit`
- `scheduler::tests::typed_draft_defers_poke_and_ordinary_wake`
- `store::connection::tests::absorption_main24_and25_to26_preserves_diagnostics_and_matches_fresh_catalog`
- `store::control::tests::invitation_confirmation_discovery_survives_committed_notice_offer_and_completed_attribution`
- `store::control::tests::invitation_confirmation_late_lazy_accept_has_no_overdue_condition_or_wake`
- `store::control::tests::invitation_confirmation_late_lazy_retirement_has_no_overdue_condition_or_wake`
- `store::control::tests::invitation_confirmation_lazy_rejection_still_publishes_attributed_notice`
- `store::control::tests::invitation_confirmation_mod_open_condition_survives_committed_notice_offer`
- `store::control::tests::invitation_confirmation_native_open_condition_survives_committed_notice_offer`
- `store::discovery_cost::every_access_path_prepares_indexed_by`
- `store::discovery_cost::wake_discovery_is_flat_in_settled_history`
- `store::discovery_cost::wake_discovery_scales_with_live_seats_only`
- `store::effective::tests::dormant_upgrade_actual_v31_backfill32_reopens_and_rolls_back_without_recapture`
- `store::invitation_due::tests::invitation_confirmation_actual32_upgrade_rolls_back_reopens_and_preserves_receipt_progress`
- `store::invitation_due::tests::invitation_confirmation_prior_schema_upgrade_preserves_exact_open_origins`
- `store::invitation_due::tests::lazy_invitation_is_discoverable_but_never_due_or_waking`
- `store::poke::tests::human_bound_seat_is_not_a_candidate`
- `store::wake_tests::human_bound_seat_never_in_a_candidate_page`
- `store::wake_tests::human_bound_seat_with_agent_in_pane_gets_no_wake_authority`
- `store::wake_tests::human_bound_seat_with_pending_acks_is_never_a_wake_candidate`
- `store::wake_tests::unresolved_recovery_hold_blocks_candidate_continuity_even_with_fresh_target`

herdr-threads::combined (14):

- `handoff_fences::handoff_actual22_upgrade_imports_already_committed_create_then_replays_without_unattached_fence`
- `lazy26_prerequisite::fresh_store_installs26_and_reopens`
- `lazy26_prerequisite::historical25_store_upgrade_preserves_evidence_and_defaults_modes`
- `lazy26_prerequisite::reopen_rejects_missing_and_altered_lazy_table_and_indexes`
- `lazy26_prerequisite::reopen_rejects_missing_and_altered_lazy_triggers`
- `lazy26_prerequisite::reopen_rejects_missing_or_weakened_mode_columns`
- `lazy_schema::lazy_schema_upgrade25_defaults_existing_messages_to_ordinary`
- `topology_handoff::begin_installs_attempt_one_and_thread_protection_without_delivery`
- `topology_handoff::current_main_fresh_and_canonical26_upgrade_to_topology28`
- `topology_handoff::schema_tests::handoff_body_bounds_upgrade_preserves_existing_identity_bytes_and_keys`
- `topology_handoff::schema_tests::handoff_bounds_upgrade_failure_restores_fk_and_retries_without_recapturing_receipt_cutover`
- `topology_handoff::schema_tests::handoff_bounds_upgrade_preserves_populated_completed_and_recovery_history`
- `topology_handoff::schema_tests::registered28_fresh_and_genuine_historical_chains_reopen`
- `topology_handoff::schema_tests::registered28_preserves25_and26_history_and_rolls_back_late_ddl_failure`

herdr-threads::integration (1):

- `channel_activation::channel_activation_real_daemon_loads_fresh_21_and_22_and_owns_archival_lane`

Scoped leakcheck exited0 and reported `no leaked test processes`. Final exact HEAD/status reconfirmed clean. All owned commands and children exited, lane explicitly released to merge_2 before this report-only write. No source change, integration reference/docs commit, shared Herdr/config mutation, push/stash, full suite or additional Cargo. Full and fix review packages remain exact for unchanged corrected HEAD:179583bytes and13435bytes respectively. Coordinator merge_2 owns mandatory clippy/fmt exact-tip gate next.
