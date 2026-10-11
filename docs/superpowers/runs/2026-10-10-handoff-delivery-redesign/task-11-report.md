# Task 11 focused integration sweep

Status: DONE.

BASE: dd1a53d43c6adaffdcbbe3b14080969660e2788f. Own branch task-ht-0lo.5; own workspace (retained for final validation): /Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.3.2. Its directory label is historical; completed Task10 is separately archived. All Cargo commands set CARGO_TARGET_DIR to this exact workspace/target and CARGO_BUILD_JOBS=4.

Only source path changed: tests/cli/handoff_delivery.rs. Coverage catches conflating invitation acceptance and message ACK in a real canonical guarded handoff. RequireAck invitation11s, message7s: message warns at7001 while invitation remains unexpired, invitation independently warns at11001 without recipient registration. Both acceptance→ACK and ACK→accept must clear only their own conditions, retain sender attention until the other settles, and finish with passive clear and unchanged receipt duration/deadline. No production change or fabricated RED.

Initial session43725 compile-only E0502 in a closure borrow; corrected before execution. Session67560 /private/tmp/ht-task11-settlement-retry.log:19.40s build,0.681s runtime,1 test PASS exercising both orders. UUID90ff9378-290a-45b5-b345-1af26c20188d scoped cleanup PASS; sandbox ps refusal initially prevented inspection, scoped escalation succeeded.

Then tightened original sender attention assertions and removed an unnecessary orphan local stage. Final source frozen during session75926 /private/tmp/ht-task11-focused-lib.log, UUID8db193c6-1b41-4dc7-b38a-ad3d3bef49d0, targeted lib coverage of combined settlement, fair/bounded/global H-D-I/genuine upgrades/coalesced origins, canonical ordinary NULL-author origin, handoff policy/capability/native readiness, maximum bodies/all paths, fixed routing and acquired-once stdin. Final result34/34PASS, recorded below.

Parent owns final PR roast, default-feature and expensive exact-tip readiness after roast, main-tab notification and merge decision. No full suite/default-feature gate/herdr action/roast/main merge/push/stash requested or performed here.

## Focused sweep outcomes and expectation correction

- Session75926 `/private/tmp/ht-task11-focused-lib.log`: build45.51s, runtime34.345s,34 selected/34passed/0failed/0ignored. UUID8db193c6-1b41-4dc7-b38a-ad3d3bef49d0 scoped leak check exit0, no leaked test processes. This is the final handoff_delivery source, including original sender attention while one condition remains and passive final clear.
- Session41170 `/private/tmp/ht-task11-focused-integration.log`: build21.67s, runtime10.301s,17selected/17passed/0failed/0ignored across lib and integration:9 stable ownership/migration tests,5 installed canary consumer tests,3 actual elected daemon delivery/newtab/replay tests. UUIDf0eab65d-28ef-44c4-9ec0-b67d1bcbde68 cleanup exit0. The notice_wake term selected zero here because its actual registration is combined; those cases are not claimed from this run.
- Session92526 `/private/tmp/ht-task11-notice-wake.log`: combined target build8.90s/runtime1.020s,2selected/0passed/2failed/0ignored. Exact failures asserted the native original author had no candidate after its recipient became overdue (lines308 and400 in the old file). These are obsolete expectations against the settled original-sender notification contract, not an actual producer bug. UUIDd5c642ff-479f-47cb-a874-ec53a092d0ba cleanup exit0.
- Corrected already mapped `tests/integration/notice_wake.rs`: unrelated bystander remains passive; original sender must be returned by actual public StorePort wake_candidates with actionable warning and no pending receipt. Its informational notice offer cannot settle its independent origin attention. When another recipient clears, the sender remains discoverable for the still-open condition. Existing affected-seat dedup/no-rearm, final nobody-wakes/passive clear, notice delivery, exact ACK actor and state checks remain. Header now distinguishes observer from sender. Actual native guard and mod/watch paths are exercised separately by the selected existing coalesced origin/native recovery tests, not newly claimed from this store-only fixture.
- Session19122 `/private/tmp/ht-task11-notice-wake-corrected.log`:2/2PASS, build2.30s/runtime0.293s; UUIDeb5152c9-e27d-47fc-bd59-a89ff5298aaa cleanup exit0. Replaced a residual one-element observer loop with a plain block to avoid lint noise; final exact-source session84577 `/private/tmp/ht-task11-notice-wake-final.log`:2/2PASS, build2.18s/runtime0.296s; UUIDd6c38cbb-9f96-4d40-969a-37a1199d6e40 cleanup exit0.

Final focused accounting:53 distinct tests (34+17+2), all passing,0ignored. Earlier reruns of the new single test and corrected notice2 are not counted twice. No tests skipped/disabled, timeout/budget raised, receipt overlay inserted, synthetic registration/ACK/acceptance, production edit or historical fixture relabeling. Actual acceptance and ACK mutations use canonical permits and real deciding APIs.

## Reproducible focused commands

Each from own workspace with `CARGO_TARGET_DIR=$PWD/target`, `CARGO_BUILD_JOBS=4` and the unique HT_LEAK_RUN_ID recorded above, inherited by owned children:

```sh
nice cargo nextest run --locked --all-features --lib -E 'test(timed_handoff_acceptance_and_ack_clear_only_their_own_overdue_condition) | test(receipt_discovery_) | test(receipt_origin_actual_ordinary_send) | test(handoff_policy_) | test(maximum_instruction_file_freezes_through) | test(maximum_escaped_delivery) | test(frozen_v1_prompt) | test(acquired_stdin_survives_retry)'
nice cargo nextest run --locked --all-features --lib --test integration -E 'test(harness::setup_tests::stable_) | test(notice_wake::) | test(setup_canary_seam::) | test(elected_public_delivery_is_staged_without_native_effect_and_completed_retry_is_historical) | test(elected_public_new_tab_completes_one_native_create_and_start_then_replays_history) | test(elected_live_registered_phase_replays_recheck_current_caller_recipient_and_payload)'
nice cargo nextest run --locked --all-features --test combined -E 'test(notice_wake::)'
scripts/check-no-leaked-processes --run-id <exact matching UUID>
nice cargo clippy --locked --all-targets --all-features -- -D warnings
cargo fmt --check
```

Sweep covers real public handoff allroutes/maxbody/retry/capabilities/readiness, immutable fixed routing and stdin; actual NULL-native ordinary author; every original sender/coalesced native/mod recovery; H=0/H>0 global fair250+250, all four historical streams and charged visits, finite snapshots/earlier arrivals/SQL work, atomic combined reset/rollback/terminal old backlog, genuine34 migration rollback/reopen/rowid reuse; stable reinstall ownership/legacy recovery/foreign drift, actual installed canary and elected runtime gates. It reuses producer proofs rather than constructing a duplicate giant fixture. README, embedded guide and TRUST current policy audited; no prose change needed, guide remains198 lines.

## Gate and self-review

Mandatory exact-source clippy session78356 `/private/tmp/ht-task11-clippy.log` PASS exit0,19.17s. Final fmt session88743 `/private/tmp/ht-task11-fmt-final.log` PASS exit0 (silent output; no full wall timing claim). git diff --check PASS. Read full own diff, including all test changes; no production source edits. Final runtime source has only the two mapped test files. No full routine suite or default-feature readiness executed; parent owns those decisions after PR roast.

## Final handoff

HEAD:50e6a20b (Verify independent timed handoff settlement and sender warning attention). Own branch task-ht-0lo.5; git status empty after commit. BASE dd1a53d43c6adaffdcbbe3b14080969660e2788f. Exactly2 mapped test files,203 insertions/6 deletions. Only shared task-11-report.md edited outside own source checkout; no run.md/progress/bead/coordinator mutations.

No active owned child/daemon/private server remains, all six scoped run UUIDs have successful cleanup. No shared server/pane operations or user config writes. Workspace and target cache retained at the exact slot path above for coordinator rebase/merge and parent final exact-tip validation. No merge/rebase/push/stash/roast/finish/upstreamfeedback/notification performed. Fresh review and integration remain coordinator-owned. Final default-feature/readiness is explicitly deferred to parent after PR roast.

No task-local correctness blocker or new performance overrun: measured final same-path builds45.51s,21.67s,8.90s,2.30s,2.18s and lint19.17s are below60s; this does not erase earlier run-level Task7/Task10 overruns or establish full-suite budget compliance. No production change needed. New combined test passed existing implementation; obsolete notice expectations were corrected to the settled contract, not presented as production RED/GREEN.
