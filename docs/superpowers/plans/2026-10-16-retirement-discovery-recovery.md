# Retirement discovery and recovery implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox syntax for tracking.

**Goal:** Make normal retirement sweeps proportional to pending jobs, with terminal-inclusive health recovery performed only as bounded reads when needed.

**Architecture:** Normal discovery uses the existing `(status, ordinal)` index and returns pending jobs. A separate terminal-inclusive reader preserves exact-job completion evidence and failure-tracking-gap coverage. Health requests that reader only while it retains named retirement failures or a tracking gap; the recovery reader never advances jobs or acquires the fair writer.

**Tech Stack:** Rust, SQLite, existing scheduler and worker-health state.

**Spec:** none — coordinator messages mg5Iw0LWR and m9vbZQB7V in the established task thread are the confirmed requirements.

## Global constraints

- Keep the original real-store latency regression.
- A normal sweep over N terminal plus k pending jobs makes about k advancement and writer calls, independently of N.
- Recovery still clears a named error for a job completed elsewhere and a tracking gap, without writer use.
- Preserve bounded pages, frozen high-water coverage, error backoff, fair writer admission, cancellation, and conservative health recovery.
- No live store writes, daemon/server restart, real user config writes, push, stash, or full test suite.
- Per-change checks: clippy all targets/features with warnings denied, harness-neutral, default features, fmt, focused tests and run-scoped leak checks.
- Commit on fix/retirement-stuck with an Outside-adapter trailer listing all changed paths.

## Task 1: Split retirement discovery and conditional recovery

**Files:** `src/store/queries.rs`, `src/store/mod.rs`, `src/ports.rs`, `src/scheduler/deadlines.rs`, `src/service/workers.rs`; tests under `tests/scheduler/`, `tests/store/`, `tests/service/` as appropriate. Avoid unrelated edits.

**Interfaces:**

```rust
// StorePort and DeadlinePort: read-only terminal-inclusive recovery reader.
fn retirement_recovery_jobs(
    &self, page: PageRequest, budget: &CallBudget,
) -> Result<Page<RetirementStatus>, ApiError>;
```

A neutral default may forward to `pending_retirement_jobs` for existing qualified doubles. Production `SqliteStore` overrides it with the current public retirement-jobs query; `ScheduledStore` forwards without `FairWriter`. Production `pending_retirement_jobs` uses an internal indexed pending-only query, with a distinct cursor filter digest. Public CLI retirement history remains terminal-inclusive.

```sql
SELECT r.ordinal,r.id,r.seat_id,r.cutover_at,r.phase,
       r.processed_units,r.status,r.last_error,s.instance_id
FROM retirements r INDEXED BY retirements_status_ordinal
JOIN seats s ON s.id=r.seat_id
WHERE r.status='pending' AND r.ordinal>?1 AND r.ordinal<=?2
ORDER BY r.ordinal LIMIT 1;
```

Normal discovery must not count as terminal-inclusive health coverage. Keep a distinct coverage marker (or separate outcome) so an empty pending page cannot clear a tracking gap. Exact normal job progress/completion may still clear its own named error.

Health owns a separate bounded recovery cursor/retry gate. It reads a bounded terminal-inclusive page only while named retirement failures or tracking gaps exist, and observes each terminal row as affirmative read evidence. A complete row clears its exact named error; a fresh, fully completed error-free walk clears a tracking gap. Pending/failed rows invalidate gap coverage. Reset the recovery cursor when recovery is no longer needed. Query errors back off and never manufacture completion. Recovery never calls `advance_retirement` and must not cause normal due/work writer passes for each historical record. A bounded recovery page per ordinary tick is acceptable; it need not immediately drain all history.

If normal discovery unexpectedly returns a completed row (for example a qualified double), treat its terminal state as read evidence and do not call advancement. Reject inconsistent terminal state conservatively.

- [x] **Write and run failing cost tests.** Retain the in-progress `completed_retirement_history_never_enters_the_advancement_writer` test and the original `retirement_backlog_drains_without_a_safety_tick_per_completed_job`. Add a real-store cost case using N=143 and N=10,000 terminal records, k=0 and a small nonzero k. Seed rows inside a fixture transaction. Measure advancement calls and/or the durable writer decision count, requiring an upper bound depending on k, not N.

```rust
assert_eq!(completed_job_advances, 0);
assert_eq!(pending_job_advances, k);
assert!(writer_decisions <= 4 * k + 8);
```

The exact constant must be derived from the real due-phase writes; record it in the test comment. Do not assert on mock behavior alone: use real store data for the proportional-cost result.

- [x] **Write and run failing recovery tests.** Record a named retirement error in WorkerStatus, complete that exact job through the fixture store, then run a health recovery read and assert the named error clears with zero advancement/writer calls. Create a real bounded tracking gap by overflowing its 64-key capacity, provide terminal records for a fresh complete walk, and assert the gap clears. A page containing a pending job must not clear it. Keep existing conservative partial/error coverage tests.

```rust
assert!(status.health().unwrap().tracking_gap);
// Drive bounded, read-only recovery pages over terminal records.
assert!(status.health().is_none());
assert_eq!(writer_decisions_after, writer_decisions_before);
```

- [x] **Implement the reader split and health recovery.** Follow the interfaces and state rules above; use the existing indexed/status query helpers and output fitting. No schema migration is needed.
- [x] **Run focused tests.** `nice cargo test --locked --all-features --lib retirement`, `nice cargo test --locked --all-features --lib scheduler::deadlines::tests`, plus any directly affected worker-health tests. Socket tests may need approved outside-sandbox execution. Use `HT_LEAK_RUN_ID=acfe6a59-b246-4599-9615-694972d86db2`.
- [x] **Review the complete diff.** Obtain a task/spec and quality review, then a final whole-branch review before reporting readiness. Fix substantive findings and rerun covering tests.
- [x] **Verify and commit.** Run required per-change checks and leak checks. Commit the follow-up on the existing branch, leave the worktree clean, and report exact costs, tests, caveats, branch and commits to main.

## Result

Implemented pending-only indexed discovery and conditional read-only recovery. Normal sweeps over both 143 and 10,000 terminal jobs use 1 fair-writer admission with k=0 and 7 with k=3; terminal rows receive no advancement. Current focused tests pass: 92 retirement, 30 deadline-driver, 10 service-health. Spec/quality, whole-branch and final fixture reviews approved without outstanding findings. Clippy all targets/features with warnings denied, harness-neutral, default features, fmt, whitespace and run-scoped process/temp leak checks pass. Full suite remains assigned to main, as requested.
