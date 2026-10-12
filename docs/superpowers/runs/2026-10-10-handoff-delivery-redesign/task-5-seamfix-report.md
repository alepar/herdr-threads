# Task5 schema rebase seam report

Status: fresh scoped Spec CLEAN / Quality CLEAN accepted by the controller; all seven authorized focused selections passed on the unchanged clean candidate. Cargo lane released. Controller owns the remaining exact-tip integration gate. Initial seam authoring used no Cargo; the later explicitly granted focused validation is recorded below.

Exact integration BASE: `630d6948c2e77826a1b90dce98feef1e263965cc`.
Exact final HEAD: `3d6685a6cddd5ef6eb05951d163902c7483cbf9f` on `task-ht-0lo.1.3`.
Original approved Task5 BASE: `58da51c13aa630fa512b6bdd20209468ae0c1adc`.
Original approved Task5 HEAD: `82a625163fc251dcf38e7b7bd2b802c351f8a26e`.
Own cwd: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.1.3`.
Own persistent future target: that exact cwd plus `/target`. No new target/cache or copied active cache.

The paused rebase's sole conflict/overlap was `src/store/schema.rs`. Continued the authorized own-branch rebase after resolving it, producing `2f005b781053262e42c512c3158364fc47620051`; the controller-approved current-fixture guard correction is the second commit `3d6685a6cddd5ef6eb05951d163902c7483cbf9f`. No integration branch/document commit, main merge, push or stash occurred.

## Final composition

Invitation29 remains followed by the canonical receipt30 FK-off wrapper and handoff31 FK-off wrapper, each invoked sequentially outside an enclosing transaction and each retaining its own restoration/rollback behavior. Fresh ordinary0001..0029 still commits version29 first. The new additive receipt deadline backfill migration is renamed from the approved provisional `0031_receipt_deadline_backfill.sql` to `0032_receipt_deadline_backfill.sql` with identical bytes. Latest schema/version acceptance is32. After handoff31 audit, version<=31 executes backfill32 and advances version in its own transaction; current32 only audits the marker/table. Receipt30 never reruns from an actual30/31/32 precursor and its existing cutover is never recaptured. The handoff31 migration/audit remains unchanged.

The bounded repair producer, effective receipt projection and pending ACK race fix have unchanged approved deltas. An automated added/deleted-line comparison over original BASE..approvedHEAD versus integration BASE..candidate proves exact equal deltas for `src/store/effective.rs`, `src/store/materialization.rs`, `src/store/receipts.rs` and `tests/store/schema.rs`, ignoring only hunk coordinates. No routing/shared-driver/fairness change, new provenance, authority change or policy weakening was introduced.

## Bounded fixture surfaces and final audit

Additional files were reported before edits and explicitly approved by the controller. Actual historical prefixes/cutpoints are retained; only current/latest expectations advance.

- `tests/store/effective.rs`: existing current upgrade checks already use `schema::LATEST_VERSION`, now truthfully32. Original actual28 setup/reopen assertions, actual29 invitation setup and receipt30-failure rollback29 remain unchanged. Added one test `dormant_upgrade_actual_v31_backfill32_reopens_and_rolls_back_without_recapture` in this existing suite. It starts with the real populated/reopened v28 fixture, replays immutable29/30/31 with explicit version stamps, then closes/reopens actual31 before initialize. It covers initial FK OFF/ON, an obstructing TEMP view that fails INSERT after main CREATE, rollback to actual31 with no new main table, reopen/retry32, exact old receipt/preparation/manifest/invitation/cutover rows and every prior catalog object/rootpage, captured physical/prepared high waters1/2, original cutover12 despite advanced sequence99/1000, repeated reopen/initialize and FK cleanliness. These new Rust assertions have not yet compiled or run.
- `tests/store/schema.rs`: all current guards use LATEST_VERSION. Exact table count remains the approved76, indexes140 and triggers183. Rootpage preservation excludes only the approved newly added backfill table alongside already-new/rebuilt objects. Its historical migration replay/version/refusal cutpoints remain unchanged.
- `tests/store/topology_handoff.rs`: exactly two post-current-initialize latest guards31→32. Genuine old26 setup and old27 setup/refusal/rollback checks remain unchanged.
- `tests/store/topology_handoff_schema.rs`: current replay ceiling/reference and successful latest guards advance31→32; current-chain inputs now additionally include genuine31. Actual historical prefix checks and old28/29/30 lists for handoff31 rebuild/populated/recovery/failure remain unchanged. Forced31 failure still expects actual30 and canonical historical30 bootstrap DDL; cutover30 conditionals remain historical. Existing non-rebuilt rootpage checks retain every old root; additive32 contributes no excluded old object.
- `tests/store/lazy26_prerequisite.rs`: exactly three current store/latest guards28→32 (fresh store, successful old25 upgrade, rejection on current reopen) and one stale latest-schema comment. Genuine historical25 precursor and rollback/refusal25 assertions remain unchanged.
- `tests/store/lazy_schema.rs`: exactly one successful actual25→current guard28→32; migration replay ceiling25 and actual25 seed unchanged.
- `tests/store/handoff_fences.rs`: exactly one successful SqliteStore initialization of actual22→current guard28→32 plus its stale latest comment; genuine22 setup/retained operation behavior unchanged.
- `tests/integration/channel_activation.rs`: exactly one daemon current initialization guard28→32 plus its stale latest comment; fresh0/actual21/actual22 inputs unchanged. No fixture process/control behavior was edited.

Final repository-wide `user_version`/LATEST_VERSION and multiline version-assertion scan found these current guards and no further stale current28/29/30/31 assertions outside the admitted historical cases. Separate exact catalog-count scan found only the schema fixture's76/140/183 guards. No broad cleanup, removed/ignored test, softened timing or assertion weakening. Existing true-legacy CHECK migrations are byte-immutable.

## Static verification

`python3 /tmp/ht-task5-seam-sql-proof.py` completed exit0. Captured final output at `/tmp/ht-task5-seam-sql-proof.log`; task5 proof total14.615s including the reused receipt/task4 proofs. It executes repository migrations statement-by-statement with explicit transaction control, avoiding executescript implicit commit. It proves:

- Every existing migration0001..0031 byte-identical to integration BASE630d6948; renamed32 byte-identical to approved82a62516's provisional31.
- Fresh0 ordinary prefix through29, then separate30/31/32; latest32 exact catalog76tables/140indexes/183triggers and empty high-water marker.
- Actual28/29/30/31 populated precursor × initial FK OFF/ON, physical file reopen, immutable receipt/invitation/bootstrap/recovery histories, exact prior catalog/rootpages and canonical cutover retained.
-32 CREATE-then-INSERT obstruction fails and rolls back to31/no table, same old graph/catalog/FK setting; physical reopen/retry32 succeeds, fixed repair high waters persist on later reopen, advanced host sequence never recaptures cutover.
- Reused task4 proof additionally covers true28/29/30 handoff31 rollback/retry and exact1MiB/max+1 recovery bounds; reused task1 proof covers true28/29 receipt30 old CHECK rollback/retry and immutable invitation origin/policy.

Direct `rustfmt --edition 2024 --config skip_children=true --check` across all12 changed Rust files exits0. `git diff --check BASE HEAD` exits0; final task `git status --short` is empty. Direct formatter used because this pass explicitly prohibits Cargo. No new SQL/production source change followed the successful proof; the final second commit changes only latest fixture literals/comments. SQL proof is not Rust runtime evidence or an independent scoped review verdict. Original272 focused executions and lint are reported in task-5-report.md for82a62516 and are not claimed as fresh candidate validation.

## Review package and changed files

Complete package generated from the exact task cwd using:

`bash /Users/alepar/.codex/skills/subagent-driven-development/scripts/review-package /Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/task-5-brief.md 630d6948c2e77826a1b90dce98feef1e263965cc 3d6685a6cddd5ef6eb05951d163902c7483cbf9f /Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/task-5-seam-review-package.diff`

Package: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/task-5-seam-review-package.diff`,86562bytes,2commits, full diff with10lines context plus commit/stat inventory. Net13files,1068insertions/40deletions:

`migrations/0032_receipt_deadline_backfill.sql`, `src/store/effective.rs`, `src/store/materialization.rs`, `src/store/receipts.rs`, `src/store/schema.rs`, `tests/integration/channel_activation.rs`, `tests/store/effective.rs`, `tests/store/handoff_fences.rs`, `tests/store/lazy26_prerequisite.rs`, `tests/store/lazy_schema.rs`, `tests/store/schema.rs`, `tests/store/topology_handoff.rs`, `tests/store/topology_handoff_schema.rs`.

## Pending focused validation and cleanup

After fresh literal CLEAN review and explicit exclusive lane grant, suggested sequential focused commands are below. Every command uses `CARGO_BUILD_JOBS=4`, `CARGO_TARGET_DIR=/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.1.3/target`, nice priority and authorized scoped direct CLI escalation. Do not create a new target or copy an active cache. These commands were pending at seam-review freeze; their subsequently authorized observed results are recorded below.

1. `nice cargo test --locked --all-features --lib store::effective::tests` — covers the new actual31→32 test, original dormant repair/rollback/ACK cases and current guards; expected46 tests, runtime previously0.93s before added case. First rebuild estimate1–3minutes from recent cross-seam evidence, an explicit risk against the1-minute incremental budget, not a claimed measurement.
2. `nice cargo test --locked --all-features --lib store::connection::tests` — schema migrations/catalog/audits/current32, previous130tests/2.01s runtime. Warm profile expected near0s if prior command compiled unchanged source.
3. `nice cargo test --locked --all-features --test combined topology_handoff` — topology current32 and true old cutpoints; previous74tests/2.13s runtime. Combined target rebuild may again take1–3minutes; not measured here.
4. `nice cargo test --locked --all-features --test combined lazy26_prerequisite` — six existing fresh/old25/reopen/audit cases, expectedseconds.
5. `nice cargo test --locked --all-features --test combined lazy_schema_upgrade25_defaults_existing_messages_to_ordinary` — exact one updated current guard, expectedsubsecond runtime.
6. `nice cargo test --locked --all-features --test combined handoff_actual22_upgrade_imports_already_committed_create_then_replays_without_unattached_fence` — exact one updated current guard, expectedsubsecond runtime.
7. `nice cargo test --locked --all-features --test integration channel_activation_real_daemon_loads_fresh_21_and_22_and_owns_archival_lane` — exact one current-init test across0/21/22, expectedseconds but actual new runtime unknown. It uses private fake-host/state/owned daemon lifecycle; verify owned children exit and run scoped leak check before release. No full integration sweep is requested.

Controller owns the mandatory exact-tip gate; no duplicate lint/default-feature/full-suite runs here. Combined global repair/fairness/warning-origin semantics remain Task7-owned and unverified by this seam producer proof.

All owned synchronous Git/formatter/Python commands exited. The SQL proof creates/removes only its exact owned temporary SQLite files; read-only escalated `ps` confirmed no owned proof process remained after normal sandbox ps refusal. No daemon/helper/test child, shared Herdr interaction/thread mutation, real config write, cache clone or Cargo invocation occurred. Own worktree remains clean at the exact final HEAD above. Reports/package only were written into the shared integration run; no shared run-state files were changed.


## Fresh review and focused Rust GREEN

Controller reported fresh seam review literal Spec CLEAN / Quality CLEAN with no findings, then explicitly granted the next exclusive Cargo lane for all seven exact selections after Task6's actual-discovery RED released it. Frozen HEAD stayed `3d6685a6cddd5ef6eb05951d163902c7483cbf9f` throughout, with no source mutation, recompilation of an altered fixture, cache copy or new target.

All Cargo commands used the exact own cwd and environment prefix already recorded above: `CARGO_BUILD_JOBS=4`, the existing absolute own `CARGO_TARGET_DIR`, `nice`, locked dependencies and all features. Commands ran sequentially under scoped direct CLI escalation. Results:

| Selection | Build/profile time | Passed/failed/ignored | Runtime | Session/result |
| --- | --- | --- | --- | --- |
| `--lib store::effective::tests` | 1m56s | 46/0/0 | 4.15s | 39083, exit0; 3586 filtered |
| `--lib store::connection::tests` | 0.04s | 130/0/0 | 6.18s | 23324, exit0; 3502 filtered |
| `--test combined topology_handoff` | 2m39s | 74/0/0 | 3.52s | 45047, exit0; 531 filtered |
| `--test combined lazy26_prerequisite` | 0.09s | 6/0/0 | 1.09s | 51634, exit0; 599 filtered |
| `--test combined lazy_schema_upgrade25_defaults_existing_messages_to_ordinary` | 0.06s | 1/0/0 | 0.10s | exit0; 604 filtered |
| `--test combined handoff_actual22_upgrade_imports_already_committed_create_then_replays_without_unattached_fence` | 0.05s | 1/0/0 | 0.23s | exit0; 604 filtered |
| `--test integration channel_activation_real_daemon_loads_fresh_21_and_22_and_owns_archival_lane` | 19.43s | 1/0/0 | 3.36s | 49695, exit0; 186 filtered |

Total 259 tests passed, zero failed/ignored, no compiler diagnostics. The newly added actual31→32 populated physical reopen/CREATE-then-INSERT rollback/retry/marker/cutover/rootpage assertions ran and passed, along with all original effective repair/ACK cases. Connection/catalog/current32 and topology true-old/current guards passed. Six lazy26 fresh/old25/reopen/audit cases, exact old25 lazy upgrade, exact old22 handoff replay and real daemon channel initialization from0/21/22 passed. Lib artifact: own target/debug/deps/herdr_threads-9111c9f9be8710f5; combined: own target/debug/deps/combined-c319dcf33c89ec96; integration: own target/debug/deps/integration-5d5ea72dd0efdef0. No foreign or stale artifact execution.

Fourth-selection infrastructure detail: its first escalation failed before process creation because automatic permission review did not finish before its deadline. The exact rejection said not to infer unsafe action from the timeout and allowed one retry. Controller was informed; the single identical retry was approved and produced the passing result above. No test ran or failed in the timed-out approval attempt, and no duplicate Cargo process was spawned. This resolved infrastructure timeout leaves no blocked action.

Budget evidence: fresh lib rebuild1m56s and combined rebuild2m39s exceed the one-minute incremental budget. Warm selections and final integration profile19.43s were within it. No timeout, assertions, profile, jobs limit, target/cache or speed budget was changed to mask those overruns. No eight-minute build diagnostic threshold was approached. These are fresh exact-candidate measurements, superseding runtime estimates above without erasing the original task's prior budget history.

Before the process-bearing channel selection, exported unique `HT_LEAK_RUN_ID=BF7CA484-6AF1-40A6-BDC5-B8A2D17270DA`. After its exit, `scripts/check-no-leaked-processes --root /Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.1.3 --run-id BF7CA484-6AF1-40A6-BDC5-B8A2D17270DA --grace 0` exited0 and printed `no leaked test processes`. No kill or foreign process action. All owned Cargo/test sessions completed. Immediately released the lane to the controller before this report bookkeeping.

Final `git rev-parse HEAD` remains3d6685a6cddd5ef6eb05951d163902c7483cbf9f and `git status --short` is empty. Review package remains the exact two-commit86562-byte artifact recorded above; no further source commit. No duplicate clippy, default-feature check, full suite, shared Herdr mutation or real-user config write. Controller merge operator owns the remaining exact-tip mandatory gate; Task7's combined global fairness/origin semantics remain outside this focused producer/seam proof.
