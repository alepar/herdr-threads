# Task 5: dormant historical receipt upgrade

Status: DONE_WITH_CONCERNS. Implemented, committed, focused GREEN and required lint/format passed. Earlier compile times exceeded the incremental budget; final GREEN build and lint are below one minute. Integration/default-feature validation remain coordinator-owned.

## Workspace and authorization

- Task: `ht-0lo.1.3`; branch `task-ht-0lo.1.3`.
- BASE: `58da51c13aa630fa512b6bdd20209468ae0c1adc`.
- Exact worktree: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.1.3`.
- Exact existing target: the worktree's absolute `/target`; inherited by coordinator-controlled APFS clone after prerequisite gates. No alternate target or other checkout source is used.
- Parent authorizes autonomous implementation, own-branch commits, jobs4/nice with one shared heavy Cargo lane, focused all-feature tests and required lint/format. Parent owns final default-feature check, review, serial schema integration and bead closure.

Read the implementer template, TDD skill and good-test reference, verification-before-completion skill, repository AGENTS/TRUST-POLICY, Task5 brief, fixture-preflight/execution-seams, Task1 original/seam-fix/seam-review reports and authoritative upgrade-aware refinement. No subagents, Herdr thread mutations, real config edits, shared server management, push or stash.

## Audit and fixture preflight

Confirmed actual production gaps before changing code: the physical effective receipt branch exposes a dormant stored NULL deadline unchanged; completed historical manifests can have no physical `receipt_state` and no remaining send-attention/timer job, making their logically derived deadline absent from due indexes. Task1's immutable migration0030/cutover and sparse historical deadline reconstruction remain prerequisites and must not be recaptured/replaced.

Four initial regressions in `tests/store/effective.rs` use real immutable migrations0001..0028, assert user_version28 and FK cleanliness, populate dormant physical and committed-manifest rows, VACUUM INTO a unique owned temp database and physically reopen before upgrade. They cover physical projection1300, indexed warning discovery from both sources, 250+250 old candidates with restart and continuing earlier-deadline arrivals, and running/terminal/waived/ACK-NULL preservation plus retained operation/result bytes. The source of expectations is immutable send1000 + frozen duration300, rather than the projection helper under test.

Static SQLite preflight ran the actual old migration chain and500-row physical/manifest graph with FK checks. It caught the need to advance `threads.next_sequence` before warning writes; a later source audit corrected arrival host decision sequence separately from thread sequence to avoid canonical event-offset collisions. This is SQL/fixture evidence only, not Rust test evidence.

## Pending producer contract

Coordinator/planner accepted provisional new migration0031 and `materialization::backfill_receipt_deadlines(&mut Connection,limit<=100)` returning `{inspected,more,completed}`. Persistent finite physical/prepared ordinal snapshots are captured once; EVERY visited row counts, including staged/unpublished, terminal, future and waived. Only published pending required obligations are projected. Backfill must preserve canonical cutover and retained bytes, and resume without recapture on restart.

The internal helper runs in `scan_due`'s transaction. Repair uses at most ceil(limit/2), ordinary due uses the rest (at least50 at limit100), and unused repair allowance is lent to ordinary due. `more` includes repair continuation. One-shot `completed` resets ordinary traversal before same-call selection; repair progress, reset decision and warning effects commit/rollback together. H0 empty completion spends no row allowance. Normal due finite traversal, quota lending across physical/sparse streams and combined driver fairness are Task7-owned.

Use the authoritative `upgrade-discovery-refinement.md`: H counts all actually inspected repair rows; D counts the final fresh ordinary receipt snapshot including repaired rows/finite arrivals; I counts the invitation snapshot. Task5 local tests do not establish the combined global Q-bound. For H>0, R=ceil(H/50)+2 and the global bound is5000+2max(R+ceil(D/100)+1,ceil(I/100)+1)Q. H0 retains the original two-class formula.

## TDD and commands so far

Command (exact own cwd/target): `CARGO_BUILD_JOBS=4 CARGO_TARGET_DIR=<absolute own target> nice cargo test --locked --all-features --lib dormant_upgrade`.

- First normal-sandbox command emitted `nice: setpriority: Operation not permitted` while starting compilation; canceled only owned session55784, exit130. This is environment setup, not RED. Parent authorizes subsequent direct CLI-only escalation for established nice/socket sandbox limitations.
- First escalated compile failed E0277 at two fixture counter parameters: `usize: ToSql` is behind rusqlite's disabled `fallible_uint` feature. No tests selected, not RED. Changed only those two parameters to explicit i64; no dependency/feature changes.
- Corrected four-test RED finished exit101: build2m17s, tests0.51s, selected4/failed4/ignored0,3607filtered. No compiler diagnostics. Artifact: own `target/debug/deps/herdr_threads-9111c9f9be8710f5`. Failures were the intended behavior: physical deadlineNone versus1300; source-pair warnings0 versus2;500-row backlog inspected0 versus100; preserved-state matrix dormant warnings0 versus2. Production source was unchanged until these failures were observed. Cargo lane released immediately.

## Implemented candidate, awaiting GREEN

Added provisional immutable migration0031 `receipt_deadline_backfill`: singleton constrained cursor with independent physical/prepared after and captured high water plus completed0/1. Schema version31 adds/audits the table; no old migration changes, cutover recapture or unbounded startup rewrite. SQLite actual-migration static proof produces76tables/140indexes/183triggers and exact schema audit SQL. EXPLAIN shows both repair streams use bounded integer-primary-key ranges and manifest lookup uses its preparation unique index with LEFT JOIN, without sorting/filtering an unbounded staged stream.

Implemented public `materialization::backfill_receipt_deadlines(&mut Connection,u16)->Result<ReceiptBackfillResult,ApiError>` and its in-transaction helper, result public fields inspected/more/completed. Physical source visits precede prepared source under one total allowance; every visited row counts. Effective canonical state gates repair; physical rows shadowed by a manifest are left alone. Pending physical NULL deadlines derive send+duration, existing physical deadlines and nonpending/waived evidence remain unchanged. Manifest projection reuses Task1 effective semantics and skips already indexed/terminal/waived rows. Empty tails advance to frozen bounds with no candidate charge; completion is durable and one-shot, including H0.

`scan_due` runs repair in its existing decision transaction, at most ceil(limit/2), reuses all unspent allowance for due rows, totals inspected counts, reports repair.more and resets the due cursor once before same-call SQL when completed, carrying extension state. Error drops both repair and warning effects and leaves the caller's cursor unchanged. For limit1, a fully spent repair allowance reports more so due work gets a continuation. Task7 still owns normal due-query high-water enforcement/full physical+sparse lending and the combined scheduler proof.

Expanded producer regressions: all-terminal500 rows plus unpublished prepared row count exactly501 and emit completion once; trigger-induced due failure rolls back projections/cursor/completion and survives reopen; preadvanced due key is retained during repair and reset on completion; removed captured tails finalize with zero row allowance; empty snapshot keeps full100 ordinary due capacity. Original500-row arrival/restart case now checks exact fixed repair highs250/251 and early new due progress.

Self-review found ACK-before-repair loses the newly derived original deadline on existing pendingNULL physical/sparse rows. Parent approved this narrow race within scope but required another observed RED. A10th selector test reuses the actualv28/reopened fixture and complete canonical target/unregistered-binding graph, matching public ACK claim/permit. Original1300 is distinct from catch-up effective6000; at clock5000 ACK must persist1300, leave extension6000 and create no warning. Existing source ACK updates are deliberately unchanged pending that test. The10-test frozen candidate started in owned escalated Cargo session10322 under the second explicit lane grant. No result is claimed yet.

Direct rustfmt (edition2024, skip_children) and git diff--check passed before freeze; required Cargo fmt and clippy remain pending lane authorization.

Second run: exit101, build1m48s,10selected/4passed/6failed/0ignored,3607filtered, tests0.78s, no compiler diagnostics. Physical projection, removed tails, all-terminal501 accounting and500-row bounded/restart/arrivals passed. The public ACK regression reached the real physical settlement path and failed as intended: terminal deadlineNone versus original1300 at clock5000 with effective extension6000. Because the test loops physical first, this RED did not reach the sparse branch; final GREEN must run both. The lane was released immediately after results.

The other five failures were erroneous event-count expectations, diagnosed from actual failed databases and existing `schema::record_overdue_inner`: open receipt warnings intentionally coalesce by (thread,recipient seat), so all2/100/500 physical/sparse obligations correctly had the SAME durable warning association while only1 event was inserted. No production warning behavior was changed. Parent approved correcting assertions to1 coalesced event AND exact association for EVERY intended obligation, with canonical open receipt condition/thread/seat checks and retained aggregate marker counts. This strengthens row-level discovery evidence instead of substituting an event-count assertion. Initial RED still proves missing deadline/index behavior (zero warning associations before implementation).

After meaningful ACK RED, applied the approved narrow pending-only fix in `settle_pending`: physical state transition persists `COALESCE(stored_deadline,original_projected_deadline)` only for canonical Pending; NotRequired suppliesNone. Sparse conflict update preserves original projected deadline only for canonical Pending and existing stored deadlines win. Existing terminal rows remain outside the update, actor/generation/proof/history columns retain normal settlement/replay behavior, and catch-up extension is never passed as original deadline. No production ACK fix preceded its observed RED. Candidate formatted/frozen again for final focused GREEN and broader required checks, pending explicit lane grant.

No full suite/default-feature gate or test child process has been started. Final cleanup, exact HEAD/files, verification and self-review are pending.

Off-lane cleanup: removed exactly the10 individually recorded owned failed-test SQLite fixture files after preserving diagnosis above. No glob/shared-temp cleanup, no foreign processes stopped. Frozen source remains unchanged pending GREEN lane.

## Final verification and commit

HEAD: `82a625163fc251dcf38e7b7bd2b802c351f8a26e` (`Repair dormant historical receipt deadlines in bounded pages`). BASE remains `58da51c13aa630fa512b6bdd20209468ae0c1adc`. Own branch is clean after commit. Seven files changed,898 insertions/20 deletions:

- `migrations/0031_receipt_deadline_backfill.sql` (new; provisional serial number).
- `src/store/schema.rs`.
- `src/store/effective.rs`.
- `src/store/materialization.rs`.
- `src/store/receipts.rs`.
- `tests/store/effective.rs`.
- `tests/store/schema.rs`.

Final frozen candidate was verified under a new explicit exclusive lane. Every Cargo command used the exact cwd above and this prefix:

```sh
CARGO_BUILD_JOBS=4 CARGO_TARGET_DIR=/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.1.3/target nice
```

| Command after prefix | Build/profile time | Selected/passed | Test time | Result |
| --- | --- | --- | --- | --- |
| `cargo test --locked --all-features --lib dormant_upgrade` |42.62s|10/10|0.94s|exit0;3607 filtered|
| `cargo test --locked --all-features --lib store::effective::tests` |0.03s|45/45|0.93s|exit0;3572 filtered|
| `cargo test --locked --all-features --lib store::connection::tests` |0.04s|130/130|2.01s|exit0;3487 filtered|
| `cargo test --locked --all-features --lib store::materialization::tests` |0.05s|22/22|0.36s|exit0;3595 filtered|
| `cargo test --locked --all-features --lib store::receipts::tests` |0.04s|65/65|0.90s|exit0;3552 filtered|
| `cargo clippy --locked --all-targets --all-features -- -D warnings` |56.64s|all targets/features|n/a|exit0; no diagnostics|

All selected tests had0 failed/0 ignored. There were272 passing executions (262 distinct tests; the initial10 are included again in effective45). Final ACK regression completed both loop branches: physical and existing sparse pendingNULL each persisted original1300, kept effective catch-up6000 and actor/generation/observation evidence, and emitted no premature warning. Every intended2/100/500 discovered obligation has its exact canonical warning association, with1 expected coalesced event. Running/terminal/waived evidence and ACKNULL are separately preserved by the matrix/restart tests.

`cargo fmt --all --check` (same cwd/target/jobs, without nice) exited0 with no output. `git diff --check` was clean before source commit. There were no compiler/lint diagnostics in the successful final runs. Artifact provenance is own absolute target, lib binary `target/debug/deps/herdr_threads-9111c9f9be8710f5`; no cross-worktree binary/source or new cache was used. No full suite or default-feature gate was run; parent owns final default-feature validation.

Honest budget evidence: the first corrected RED build took2m17s and the second producer/ACK RED build1m48s, both over the1-minute incremental budget. The final frozen build42.62s and full required lint56.64s meet it. No test timeout, compiler profile, speed budget or assertion was relaxed. Event-count expectations were corrected only after documenting existing warning coalescing and adding per-obligation canonical association checks.

## Task7 producer integration and review notes

Public signature: `materialization::backfill_receipt_deadlines(&mut rusqlite::Connection, limit:u16) -> Result<ReceiptBackfillResult,ApiError>`, with public `inspected:u16`, `more:bool`, `completed:bool`. Valid limit1..=100. For combined due driving, call existing `receipts::scan_due`, which uses the internal producer in the same decision transaction, rather than independently consuming standalone completion. Its existing result.more now explicitly includes repair continuation. Its inspected is repair+ordinary actual visited rows, bounded by requested limit.

Migration31 snapshots all physical/prepared high ordinals once. New arrivals are excluded from repair H; no count filtered by obligation state or publication disguises work. Actual tested mixed snapshot highs250/251 include250 physical,250 published prepared and1 unpublished prepared row; the all-terminal test inspects exactly501. Restart reuses these highs. Removed tails produce zero-charge finalization. Empty H completes in the same call and lends all100 slots to ordinary due discovery. During nonempty repair at most50/100 slots are used for repair, permitting new due arrivals to be marked before repair exhaustion. The ordinary due cursor remains intact for intermediate pages; only durable completion resets it before same-call due selection. Failure-trigger/restart test proves projection/cursor/completion/warnings roll back together; completion is emitted once.

Task7 must retain the accepted H/D/I bound and own the existing ordinary scanner SQL high-water and physical/sparse lending fixes. Task5 does not claim the combined driver Q-bound from local tests. Serial schema integration must renumber this provisional0031 against independent Task4/Task6 migrations and review latest-version/catalog seams freshly; immutable existing migrations0001..0030 are untouched. No current-binding heuristic, authority change, new provenance or TRUST-POLICY weakening was introduced.

Self-review checked exact finite ordinal traversal and every visited-row charge, published-only projection, physical shadow handling, pending-only original clock persistence, extension separation, no terminal/waived mutation, one-shot atomic reset, rollback/restart and retained bytes. No further source changes were made after final validation.

## Cleanup

Ten individually recorded owned failed-fixture databases were removed after diagnosis; passing tests remove their own files. No children/daemon/Herdr process was spawned by these in-process store tests. The final `scripts/check-no-leaked-processes --root <exact own worktree> --grace 0` initially encountered sandbox ps denial; identical read-only CLI escalation exited0 and printed `no leaked test processes`. No process was killed, no foreign workspace/process touched, and no owned Cargo/test process remains. Cargo lane was released immediately after clippy completed, before report/commit work. Git index writes required scoped escalation after sandbox index.lock denial; the authorized own-branch stage/commit succeeded. No push, stash, rebase or merge performed.
