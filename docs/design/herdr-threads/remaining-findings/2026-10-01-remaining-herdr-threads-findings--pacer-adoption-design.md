# B2 Pacer adoption: daemon loops, cancellation and wake ladder (design)

Parent spec: [2026-10-01-remaining-herdr-threads-findings-design.md](./2026-10-01-remaining-herdr-threads-findings-design.md)
(§B2, §Sequencing, §Configurations to exercise) · Bead: `ht-p03.9` (epic, promoted at the root split) ·
Root epic: `ht-p03` · Mode B (autonomous, caller-invoked).

## Goal

Every daemon loop runs on the `Pacer` (ht-p03.7). A committed change kicks exactly the lanes whose inputs
changed, so a send is picked up in under 100 ms. An idle daemon wakes at most once per 5 s safety tick and
its deadline and wake lanes and request paths make no durable commits. The retention lane makes none when
nothing is published, and ≤ 1 per 60 s tick while observation publishes at idle. With Herdr stopped,
the observation lane costs at most one durable commit per backoff step, capped at 30 s (this bound is
the observation lane's; the wake lane's refused-wake commits are D4's scope note). Cancellation wakes waiters instead of being polled. A pre-send wake refusal
retries on backoff and does not climb the 30 s → 300 s reminder ladder.

Ancestor goal (root `## Goal`, verbatim prefix): *herdr-threads at the integration tip is ready to tag
v0.1.0: the daemon stays fast and quiet over weeks of retained history and through Herdr outages, …*

## Problem description

All three daemon lanes in `src/service/workers.rs` (deadlines, wakes, host observations) are `std::thread`
loops. Each one runs a pass and then sleeps 5 × 20 ms, so every lane wakes about 10 times per second for
the life of the daemon. The deadline driver adds a 1 s tick gate (`TICK_MILLIS`). Nothing kicks it after
a commit: `DeadlineDriver::after_committed_change` exists, but production never calls it
(`workers.rs:857`). The result is roughly one job per second, and real work waits for the tick.

When Herdr is down, `ObservationLane::discard` (`reconcile.rs:885`) marks the lane dirty. That makes
`snapshot_due` true straight away, so each 100 ms turn runs a full capture: one `begin_host_observation`
admission commit, then one `invalidate_host_observation` commit. That is about 20 durable commits per
second, all recording the same failure.

Async cancellation is polled in a `while !is_cancelled() { sleep(10 ms) }` loop in three places:
`daemon/transport.rs:68` (plus the 10 ms drain loop at `:254`), `client/local.rs:21` and
`client/service.rs:461`. `SessionLease::drop` (`service_connection.rs:213`) takes the `LiveServiceGate`
std mutex on the current-thread runtime. That mutex is also held by `decision_guard` for the length of a
store write transaction, so the drop can block the runtime for a whole transaction.

The wake ladder (W9-1) advances at reservation commit: `RetryGuard::reserve` and `store/wake.rs:424` set
`retry_step+1`. The target-readiness checks run after that, inside `NativeWakeDispatcher::attempt_wake`.
So a target that was never sent anything ("not idle yet", host unavailable) still uses up a ladder step
and pushes the next try out to 60 / 120 / 300 s. Worse, the reservation also overwrites the seat's
offered frontier (`last_*_seq/offset`), so a refused warning wake counts as offered.

## Main challenges

- **The lanes are threads, the primitive is Notify.** A `tokio::sync::Notify` cannot be awaited from a
  `std::thread` without a runtime handle, so a lane needs a way to block until kicked, retry-due or
  cancelled. The lanes have to stay threads, because they make blocking SQLite and Herdr calls.
- **Kicks must fire after the commit is visible.** SQLite's commit hook runs *before* the commit
  completes. A kick fired from there wakes a lane that reads the old WAL snapshot and goes back to sleep
  for 5 s.
- **Lanes commit too.** If a lane kicks itself on its own commits, it busy-loops. Kicking only on
  foreground commits would miss the edges between lanes (for example, a deadline-lane warning that should
  wake the wake lane).
- **About 30 commit sites.** `tx.commit()` appears in `connection.rs`, `messages.rs`, `seats.rs`,
  `service_events.rs`, `materialization.rs`, `control.rs`, `queries.rs` and others. Wiring each site by
  hand would rot.
- **"Idle: no durable commits" conflicts with "unchanged reconciliation cadence".** Every 5 s observation
  cycle takes a durable admission (`observation_admission_sequence+1`) and may publish a snapshot
  generation. That is identity/B5-adjacent fencing, and this epic must not change it.
- **The refusal fix has to leave the durable ladder exactly as it was** (step, delays, ever-reserved
  marker, offered frontier). It must also stay correct across a crash between reservation and completion.

## Key decisions made

1. **Table-level change capture on the single writer connection.** SQLite's update hook records which
   tables changed, the commit and rollback hooks seal or discard that set, and the writer turn takes
   the sealed set while it still holds the writer mutex guard and kicks after releasing it. A static `table → lane` map turns the set into `Pacer::kick()`
   calls, minus the lane that made the commit. The map has an exhaustive classification test.
2. **Lanes stay `std::thread`s.** They block in the Pacer's blocking wait, which takes a safety tick:
   5 s for deadlines and wakes. The observation lane keeps its 5 s snapshot cadence. A pass that reports
   more work runs again immediately. A kicked pass resets the lane's safety tick, so an idle wake lane does
   at most one pass per 5 s even with observation kicks. A failed pass waits out the Pacer backoff (100 ms × 2ⁿ, cap 30 s,
   ±20 %).
3. **One cancellation type.** ht-p03.7 upgrades `protocol::time::Cancellation` itself to be
   Notify-backed, with async `cancelled()` and a blocking wait (consumer contract agreed with the
   coordinator on 2026-10-01). This epic adopts it: every 10 ms poll helper is deleted, and
   `SessionLease::drop` uses `try_lock` and otherwise hands the revoke to the blocking pool.
4. **A stopped Herdr is quiet.** A failed capture schedules its retry on the observation lane's backoff,
   not on the dirty flag. An explicit target capture (an operator
   command) requests one lane capture that runs even inside a backoff wait (`Pacer::kick_explicit`,
   ht-p03.104). If it fails, it counts as the next backoff step. Commit kicks never shorten a backoff. A repeat failure with the same invalidation reason, no publication since the
   last durable invalidation, and that invalidation's unresolved-marking pass completed, skips the
   invalidation write and returns a dedicated `InvalidationRepeated` outcome. It is counted in memory
   instead. That leaves the observation lane one admission commit per backoff step.
5. **W9-1: refusals undo their reservation's ladder effect.** Pre-send refusals become a distinct
   `WakeOutcome::Refused(cause)`. Completing a refused attempt restores the pre-reservation ladder fields
   and offered frontier inside the completion transaction. The seat then retries on a per-seat
   in-memory backoff with the Pacer schedule. Only `Submitted` and `OutcomeUnknown` (a wake that was
   delivered or may have been) keep the advanced step.
6. **Health shows retries without new lines.** The lane's existing Health line gains a suffix,
   `retrying (attempt N, next ≤ Xs)`, read from the lane's Pacer. The line count does not change, so the
   ht-p03.27 line budget is unaffected.

## Consumed contracts

- **From ht-p03.7 (Pacer API)**: `Pacer::kick()`, `on_failure()` / `on_success()`, `attempts()`,
  `next_retry_at()`, the "lane went idle" test hook, and a **blocking wait usable from a `std::thread`**
  that returns on kick, retry-due, safety tick or cancellation (for example
  `wait_blocking(tick) -> Wake { Kicked, RetryDue, Tick, Cancelled }`). The W9-1 leaf also needs the
  backoff schedule as a **standalone value** that can be kept per seat (for example a `Backoff` struct).
  It also needs **`protocol::time::Cancellation` itself upgraded to Notify-backed**, with
  `async cancelled()` and `wait_blocking(timeout)`, and accepted by the Pacer wait.
- The coordinator appended all three requirements to ht-p03.7's description as a consumer contract on
  2026-10-01, so this design assumes .7 delivers them.
- Fallback, if .7 lands short: the consuming leaf adds a thin adapter inside its own file
  (`workers.rs`, `scheduler/mod.rs` or `protocol/time.rs`), never in `pacer.rs`. There is no separate
  Cancellation bead.
- **From ht-p03.3 (post-collapse surface)**: `DeadlinePort`'s blanket impl and fake-friendly defaults are
  gone, scheduler tests run on `SqliteStore`, the `identity/reconcile.rs` forwarding traits are direct
  calls, and `StorePort` has no `Unsupported` defaults. The W9-1 completion change and the lane loops
  are written against that surface, not the pre-collapse one.
- **From ht-p03.1 (isolated named Herdr session fixture)**: the Herdr-stopped and Herdr-restarted tests.
  The shared Herdr server is never touched (root §Hard constraints).

## Decision points

### D1. Commit → lane kick plumbing (closes `workers.rs:857`)

**Recommended: capture changes at the connection level, flush after the writer turn.**

- At `open_writer`, `StoreContext` installs three hooks on the one domain writer connection:
  - an **update hook** that ORs the changed table's lane set into a per-connection `pending` bitset;
  - a **commit hook** that moves `pending` into `sealed` and returns 0, so the commit is never vetoed;
  - a **rollback hook** that clears `pending`.
- Install them with rusqlite's `hooks` feature on the same pinned 0.39.0, or through the `ffi` calls
  already used for the progress handler. The implementer picks one.
- `SqliteStore::writer()` returns a `WriterTurn` that wraps the `MutexGuard`. `WriterTurn::drop` first
  **takes `sealed` (swaps it for an empty set) while the guard is still held**, then releases the
  guard, then calls `CommitKicks::kick(taken − origin)` (amended by design roast round 1).
  - Why the order matters: only the guard holder can commit on the writer connection, so the set taken
    under the guard is exactly this turn's commits. Taking it after release let another thread's turn
    (for example the wake lane, origin `Wakes`) commit in the gap, OR its tables into the same `sealed`,
    and flush the combined set minus *its* origin — dropping a request commit's `Wakes` kick as a
    "self-kick" and leaving the send to the 5 s safety tick.
  - The kick still fires after the commit is visible (the commit statement has returned before the
    turn drops) and still never runs under the writer mutex.
- `origin` is a thread-local `Lane` that each lane thread sets once at start (`kicks::enter_lane`). It is
  empty on request threads.
- Raw `BEGIN…COMMIT` sites, such as `queries.rs:89`, are covered automatically because they run on the
  same connection.
- A commit that changes no rows fires no update hook, so it produces no kick. This is what keeps the
  empty due-scan transactions of an idle deadline lane silent.

**Mapping** (`src/service/kicks.rs`, one `match` on the table name):

| Lane | Kicked by changes to | Why |
|---|---|---|
| `Wakes` | `wake_work`, `seats`, `occupant_bindings`, `seat_availability`, `warning_recipients`, `warning_offer` | Wake candidates come from `wake_work` reason bits joined to seat/binding state. A binding change can make a refused seat sendable. (`snapshot_generations` removed by design roast round 2: one observation publish is several separate commits on it — begin, one stage commit per target chunk, seal, publish — so it kicked the wake lane several times per idle 5 s cycle. Wake discovery, `store/wake.rs`, never reads it.) |
| `Deadlines` | `work_jobs`, `warning_jobs`, `retirements`, `invitations`, `invitation_cancellations`, `receipts`, `receipt_state` | Newly enqueued jobs need an immediate quantum. Invitation and receipt rows can create, cancel or settle obligations. The kick also calls `DeadlineDriver::after_committed_change()` so the tick gate does not skip the pass. |
| `Observation` | none | Host-driven. Its cadence and host-event dirty hints are unchanged. |
| none (explicit) | `host_instances` (holds `observation_admission_sequence` and `observation_decided_sequence`, migrations/0001_initial.sql:22-23; written by the admission fence in `store/seats.rs`) | Explicitly NOT in the Wakes or Retention kick sets. Observation-lane admission commits must not kick the wake lane, or the 5 s observation cadence would wake it every cycle and defeat the idle acceptance. `snapshot_generations` and `snapshot_targets` (observation stage, seal and publish) are classified "none" too (design roast round 2). A publication that changes seat or binding state kicks Wakes through `seats` and `occupant_bindings`. A publication whose only wake-relevant effect is the new active snapshot pointer (`host_instances`) reaches the wake lane at its 5 s safety tick or at a refused seat's D5 `next_due_at`, whichever is first. |
| `Retention` (B1, ht-p03.12) | no table (ht-p03.12 D4) — Amended by coverage r2, 2026-10-01: ~~added by ht-p03.12 when its lane lands (expected: `snapshot_generations`)~~ superseded; the kick row is empty and Retention-origin commits kick no lane | The variant exists so B1 only registers an empty-set lane; no table maps to it, so kicks to it never fire. |
| `AdmissionObserver` (ht-p03.9.6) | no kicked tables — Amended by coverage r2, 2026-10-01 | Lane::AdmissionObserver is the harness-admission observer loop on the Pacer (L6); its row is empty and the lane has its own per-origin counter key (lane wiring contract ht-p03.39). Its commits, if any, kick no lane. |
| — | every other table | Classified explicitly as "no lane" in the same `match`. |

The **exhaustive classification test** reads `sqlite_master` from a freshly migrated store and asserts
that every table appears in the `match`. A future migration that adds a table fails the test until
someone classifies it.

Considered:
- *Per-call-site `kick(lanes)`*: rejected. There are about 30 commit sites, and nothing would catch a
  forgotten one.
- *Kick every lane on every commit*: rejected. Lane commits would cause self-feedback and cross-lane
  ping-pong, and the idle acceptance would fail.
- *Kick from the commit hook*: rejected because of the pre-visibility race described in Main challenges.
- *A channel carrying work items*: rejected by the parent spec (it would duplicate the durable queue).

### D2. Lane loops and per-loop parameters

All lanes use the Pacer default schedule: **base 100 ms, factor 2, cap 30 s, ±20 % jitter, reset on
first success**. The Pacer wait also takes the lane's `Cancellation`, so shutdown wakes a blocked lane
right away.

| Lane | Safety tick | Re-run immediately when | Failure that backs off | Notes |
|---|---|---|---|---|
| Deadlines | **5 s** (was 100 ms turns plus a 1 s gate) | `due_continuation`, or retirement/work progressed with more pending | `drive_deadlines` returns `Err` | `TICK_MILLIS` becomes the 5 s safety tick. Time-driven expiries (invitation/receipt deadlines, minute-scale with a 300 s default) can fire up to 5 s late. That is documented in `docs/operations.md`. Per-job `RETIREMENT_RETRY_MILLIS` / `DUE_PHASE_RETRY_MILLIS` gates stay as job-local retry gates. |
| Wakes | **5 s**, or earlier at `WakeDriveOutcome::next_due_at` (D5) | `has_more` | `drive_wakes` returns `Err` with no callback-classified cause | Per-seat refusal backoff is D5's and is separate from lane backoff. |
| Observation | the 5 s snapshot cadence (unchanged) | a reconciliation continuation page is pending | capture `Err`, `Invalidated` or `InvalidationRepeated` (D4) | Recovers to the 5 s cadence on the first `Published`. |
| Retention (B1) | owned by ht-p03.12 (60 s; idle bound owned by ht-p03.12.6) | — | — | Consumes D1 and the same loop shape. |
| AdmissionObserver (Amended by coverage r2, 2026-10-01; ht-p03.9.6) | today's admission cadence, never below **5 s** | — | the admission observation returns `Err` (Pacer default schedule, reset on success) | `Lane::AdmissionObserver`, no kicked tables; a std thread under `kicks::enter_lane`, Notify-backed `Cancellation`, `WorkerStatus` attached; success/failure recorded through the lane wiring contract (ht-p03.39). ht-p03.23's re-observation runs on this tick. |

**Safety tick vs kicks.** A pass triggered by a kick resets the lane's safety tick (the wait restarts at the full tick). At idle no lane receives kicks from the observation lane: `observation_admission_sequence` and the publication pointer live in `host_instances`, and the stage/seal/publish commits write `snapshot_generations` and `snapshot_targets`, none of which is in any kick set (D1 table, design roast round 2). So an idle wake or deadline lane wakes only on its safety tick.

**Health.** `WorkerStatus` gains `attach_pacer(Arc<Pacer>)` and `retry() -> Option<(attempts,
next_retry_at)>`. `RedactedWorkerHealth::summary` appends `; retrying (attempt N, next ≤ Xs)` while
`attempts > 0`. There are no new Health lines, so this needs no `health.rs` change. ht-p03.11's
rate-limited logging reads the same Pacer counters.

Considered:
- *Convert lanes to tokio tasks*: rejected. They make blocking SQLite and host calls, and moving them
  would mean rewriting `HostPort` and the writer.
- *Keep the 1 s deadline tick*: rejected. The idle acceptance allows ≤ 1 wakeup per 5 s.
- *Compute the exact next-due instant from the store*: rejected for now. It needs a new indexed `MIN()`
  query per table, for minute-scale deadlines.

### D3. Cancellation migration (closes `transport.rs:67/68`, `service_connection.rs:213`)

**Recommended:**
- ht-p03.7 delivers `protocol::time::Cancellation` as the one Notify-backed type. It keeps its public
  name and `cancel()` / `is_cancelled()`, and adds `async cancelled()` (arm `notified()`, check the
  flag, await: no lost wakeup) and `wait_blocking(timeout)`.
- Every `CallBudget` and every Pacer share that one object. This epic does not redesign it. It adopts
  it everywhere a poll exists today.
- The poll helpers `daemon/transport.rs::cancelled`, `client/local.rs::cancelled` and
  `client/service.rs::Self::cancelled` are deleted, and call sites use `.cancelled()`.
- The accept-drain loop (`transport.rs:254`) waits on task completion (`JoinSet` / `select!` over the
  next finishing task and the drain deadline) instead of sleeping 10 ms.

**`SessionLease::drop`:**
- `try_lock` the gate. On success, revoke inline.
- On `WouldBlock` (a decision guard holds it during a store transaction), first cancel the session's
  `Cancellation`, which is non-blocking and stops further frames. Then hand `revoke_exact` to
  `tokio::task::spawn_blocking`, the path `revoke_after_disconnect` already uses. If there is no runtime
  (`Handle::try_current` fails), use a detached `std::thread`.
- Ordering does not change: the revoke still runs after the in-flight decision completes, which is what
  a blocking lock gave before.

The parent spec says "a release message processed by the owning task". The blocking pool *is* that
owner for blocking work, and an mpsc to the connection task would add a second shutdown path for no
benefit.

Considered: *keep `AtomicBool` and add a separate Notify beside each token* (rejected: two sources of
truth); *`async` mutex for the gate* (rejected: `decision_guard` is held from synchronous store code).

### D4. Herdr-down observation lane (closes `reconcile.rs:885`)

**Recommended:**
- `ObservationLane::discard` stops setting `dirty`. It sets `retry_after_failure`, so
  `snapshot_due(now)` is gated by the lane Pacer's `next_retry_at()` and not by "now".
- Host-event invalidation hints still set `dirty`. A real host event is not a failure retry.
- (ht-p03.2 deletes the P10 EmptyShell / verified-execution branches and `decision_fence`. D4 is
  written against the remaining `ReconfirmStructure`-only continuity path.)
- In `observe_and_publish`'s failure path, a failure may **skip** `invalidate_host_observation` and the
  unresolved-marking pages. The skip is a complete contract (amended by design roast round 1):
  - **Predicate.** All three must hold: (1) the failure would durably invalidate with the **same
    reason** as the last durable invalidation; (2) no publication has happened since; (3) that
    invalidation's **unresolved-marking pass completed** — every reconciliation page of its
    continuation committed. The lane remembers `(last_invalidation_reason, published_since)` in memory.
    `last_invalidation_reason` is set only when the last page of the continuation armed by an
    `Invalidated` outcome commits (`next_after_ordinal` is `None`); any page error (stale fence,
    `CursorStale`, a bounded-read, store or budget error) **clears** it. So a failure after an
    interrupted marking pass re-runs the full invalidation from ordinal 0, which is today's
    self-healing behaviour. After a daemon restart the field starts empty, so the first failure always
    invalidates in full.
  - **Outcome.** A skip returns a dedicated `ObservationOutcome::InvalidationRepeated { reason, cause }`
    — never `Superseded` (which would clear the Health failure while Herdr is still down), never
    `Invalidated` (which would arm a continuation and re-run the skipped pages), never `Err` (which
    would reclassify the failure as `ObservationCapture`). It is handled like `Invalidated` for
    reporting: `WorkerStatus::observe_capture` records `ObservationInvalidated(reason)` and the lane
    calls `host_evidence.record_invalidated(reason, cause)`. It differs in control flow: it arms **no**
    continuation, and the lane counts it as a backoff failure (`on_failure()`, the same as
    `Invalidated` and capture `Err` in D2). It increments the in-memory `repeat_failures` counter,
    reported through the lane's Pacer attempts.
- Each backoff step after the first failure therefore costs the observation lane exactly one durable
  commit: the admission. The fence still advances, so B5 fencing semantics are untouched.
- The first `Published` result resets the Pacer and the counter, clears `last_invalidation_reason`, and
  cadence returns to 5 s.

**Scope of "≤ 1 commit per backoff step"** (stated by design roast round 1; this does not resolve the
open escalation). The bound covers the **observation lane only**. It does not bound the wake lane's
durable commits while Herdr is down. Under D5, each refused wake attempt costs a reserve commit plus a
fenced completion commit, once per refused seat per refusal-backoff step (100 ms × 2ⁿ, cap 30 s).
Whether that wake-lane cost is acceptable as specified, or needs its own bound and an outage-test
assertion, is the round-1 escalation "pacer D5 vs root §B2 D2: wake-lane durable commits while Herdr is
down" (material dissent). It **remains parked for the human** (run.md `parked:`). This spec neither
accepts nor rejects that cost. Cross-reference (design roast round 2): the task tree pins the cost as
currently specified. ht-p03.9.4's outage test asserts **≤ 2 durable commits per seat per
refusal-backoff step** (reservation + restoring completion). That criterion records D5's cost, not a
decision to accept it. If the human resolves the escalation with a different bound, the criterion is
revised to match.

Considered:
- *Skip the admission too while failing*: rejected. The admission sequence is the fence that orders
  concurrent observers, and that is B5-frozen.
- *Durable failure counter column*: rejected. It is a schema change for an in-memory concern.

**Acceptance reconciliation.** The parent acceptance says "idle daemon: no durable commits", but parent
Decision 1 keeps the reconciliation cadence. Every 5 s observation cycle commits an admission and a
snapshot generation (B1 prunes those). This design resolves the conflict this way:
- An idle daemon makes **zero durable commits from the deadline and wake lanes and from request paths**.
  The **retention** lane makes zero when nothing is published, and **≤ 1 commit per 60 s tick while
  observation publishes at idle**: each idle 5 s cycle publishes a snapshot generation, which leaves a
  prunable superseded generation. Skipping unchanged snapshots is deferred (below).
- The observation lane makes **no more commits than one cycle's worth per 5 s cadence step**.
- Each lane makes **≤ 1 wakeup per safety tick**; a kicked pass resets that lane's tick, so an idle wake lane runs at most one pass per 5 s. Observation commits kick nothing: `host_instances`, `snapshot_generations` and `snapshot_targets` are unmapped (D1, as amended by design roast round 2). The L5 idle test asserts that an unchanged-host cycle makes zero commits touching a Wakes-mapped table: reconciliation pages whose transitions are all `Unchanged` are expected to write no seat or binding row. If a page turns out to rewrite unchanged rows, L5 records that as a deviation, because the write is B5-owned. It does not relax the bound.
- With Herdr down, the **observation lane** costs **≤ 1 commit (the admission fence) per backoff step**
  (wake-lane scope: see the scope note above; parked escalation).
- Test hook: a per-origin-lane commit counter (owned by L1) lets the idle test assert zero commits from deadline, wake and request origins while observation commits.

Removing the idle observation commit needs a "skip unchanged snapshot" protocol change in identity
(B5-adjacent). It is recorded as a deferral below.

### D5. W9-1 wake ladder (closes W9-1)

**Recommended:**
- `WakeOutcome` gains `Refused(RefusalCause)`, where `RefusalCause` is `Unsafe`, `Unavailable` or
  `TimedOut`.
- `NativeWakeDispatcher::attempt_wake` returns it for **every exit before `submit_prompt` is called**.
  That covers the boot/epoch mismatch, the budget checks, the identity/basis/terminal mismatches, and
  errors from `observe_current_target` / `safe_wake_target` / `is_current`, which are mapped to the
  matching cause rather than propagated with `?`.
- Errors raised by `submit_prompt` keep today's mapping in `scheduler/mod.rs:519-525`, because a prompt
  may already have been sent.

On completion, the scheduler passes the pre-reservation ladder state it reserved from: the candidate's
`retry_step`, `minimum_delay_ms`, `effective_delay_ms`, `last_reservation_id`/boot/at and
`last_reserved_frontier`.
- `store::wake::complete` writes `last_outcome` with the cause's existing string. The new outcome adds
  no new disposition strings.
- For `Refused`, the same fenced `UPDATE` also restores those fields to their prior values. The fence is
  `reservation_id` and `reservation_boot` matching.
- In memory, `DispatchState` restores the seat's `RetryGuard` to its prior durable state, so the refusal
  does not re-anchor. It also advances a per-seat refusal `Backoff` (the Pacer schedule: 100 ms → 30 s
  cap, ±20 %). `can_reserve` requires both the ladder and the refusal backoff to be eligible.
- Amended by coverage r2, 2026-10-01: `unsubmitted` is carried in `WakeDriveOutcome`; `last_outcome` keeps the existing `OutcomeUnknown` string (no new disposition string is written).
- `Submitted` resets the seat's refusal backoff. `OutcomeUnknown`, `TimedOut` (post-send) and
  `Cancelled` keep today's ladder behaviour.
- `WakeDriveOutcome` gains `next_due_at: Option<MonoInstant>`, the earliest ladder or refusal instant
  among seats this runner tracks. The wake lane (D2) wakes at `min(safety tick, next_due_at)`, so
  refusal retries keep sub-second precision.

**Crash safety.** A crash between reservation and completion leaves the advanced step. Recovery settles
it as abandoned (`recover_abandoned`), as it does today. This errs toward the slower ladder and never
toward a wake storm.

**Fence miss** (amended by design roast round 1). Some existing store paths clear `reservation_id`
without rolling back `retry_step` or the offered frontier: host-invalidation mark-unresolved
(`store/seats.rs` ~1583), registration loss (`seats.rs` ~2472) and `store/control.rs` ~137. If one
of them commits between the reservation and a `Refused` completion, the fenced restore matches 0 rows.
The outcome is **accepted, not repaired**:
- The durable row keeps the one advanced step and the overwritten frontier, the same direction as the
  crash case: one slower ladder step, never a storm. The window is a few milliseconds.
- `store::wake::complete` returns whether the fenced update matched a row. `DispatchState` restores the
  seat's in-memory `RetryGuard` **only when it did**, so memory and the durable row never disagree.
  The per-seat refusal `Backoff` still advances, because the refusal happened either way.
- The clearing paths are not changed to roll back ladder fields. They belong to B5-adjacent
  invalidation and registration code, and the cost of the miss is one ladder step.

The sequence of `retry_step` values for a refused wake therefore depends only on whether a concurrent
clear happened, never on timing between memory and store.

**Offered frontier.** Restoring `last_*_seq/offset` means a refused warning is not counted as offered,
so `warning_offered_for_current_occupant()` stays false and the warning is retried.

Considered:
- *Readiness probe before reservation*: rejected as the only fix. The post-reservation recheck still
  has to exist, and its refusals would still climb the ladder.
- *New `wake_work` columns for the prior state*: rejected. The candidate already carries it, and a
  crash conservatively keeps the advanced step.
- *Decrementing `retry_step`*: rejected. It is wrong for a first reservation (`ever_reserved`) and
  leaves the frontier overwritten.

## Leaves (decomposition)

Children of `ht-p03.9`. Edges are leaf-level and sparse, and each one is paired with a `blocked-by` reason line in the dependent bead.

| Leaf | Scope | Owns | Consumes (edge) |
|---|---|---|---|
| L1 `ht-p03.9.1` — Lane registry: commit-change kicks and Health retry publication | D1 hooks, `WriterTurn` flush, `kicks.rs` map and exhaustive test, thread-local origin; `WorkerStatus::attach_pacer` / `retry()` and the summary suffix (D2 Health) | `Lane` enum, `lanes_for_table`, `CommitKicks` installed on `SqliteStore`, `kicks::enter_lane`, `WorkerStatus::attach_pacer`/`retry()` | Pacer `kick()`/`attempts()`/`next_retry_at()` (ht-p03.7) |
| L2 `ht-p03.9.2` — Event-driven transport/client cancellation and non-blocking lease drop | D3 (adoption: delete the poll helpers and the drain sleep; `SessionLease::drop`) | — (non-gating) | Notify-backed `Cancellation::cancelled()` (ht-p03.7) |
| L3 `ht-p03.9.3` — W9-1 pre-send refusals retry on backoff | D5 | `WakeOutcome::Refused`, refused-completion restore, per-seat refusal backoff, `WakeDriveOutcome::next_due_at` | Pacer standalone backoff (ht-p03.7); post-collapse `WakePort`/`StorePort` (ht-p03.3) |
| L4 `ht-p03.9.4` — Deadline and wake lanes on the Pacer | D2 for deadlines and wakes, kick subscription, `after_committed_change` on kick | — | L1 (kick map, origin, attach_pacer), L3 (`next_due_at`), ht-p03.3 (DeadlinePort/ScheduledStore surface); Pacer wait and Cancellation (ht-p03.7) transitively via L1 |
| L5 `ht-p03.9.5` — Observation lane backoff and quiet Herdr-down | D4 and D2 for observation | — | L1 (`Lane::Observation`, attach_pacer), ht-p03.1 (fixture), ht-p03.3 (reconcile direct calls); Pacer wait and Cancellation (ht-p03.7) transitively via L1 |
| L6 `ht-p03.9.6` — Admission-observer tick on the Pacer and daemon-loop inventory (Amended by coverage r2, 2026-10-01) | D2 `AdmissionObserver` row (`Lane::AdmissionObserver`, no kicked tables, safety tick ≥ 5 s); the remaining-periodic-loop inventory and its test-asserted allowlist; the epic's whole-tree `sleep(Duration::from_millis((10\|20)))` rg | the admission-observer loop on the Pacer (consumed by ht-p03.23 re-observation); the loop-inventory allowlist | Pacer wait/backoff and Cancellation (ht-p03.7); L1 (`Lane`, `enter_lane`, `attach_pacer`); lane wiring contract (ht-p03.39); L2, L4, L5 (inventory runs after the transport, deadline/wake and observation loops are converted) |

Critical path inside the epic: ht-p03.7 → {L1, L3} → {L4, L5}, which is 2 rounds after .7. L2 runs
alongside them and gates nothing.
`workers.rs` is shared by L1, L4, L5 and by root siblings ht-p03.11 / ht-p03.27. Each leaf touches
disjoint functions in it, and the merge gate handles the rest.

## Acceptance (epic level, distributed to leaves)

- A committed send is visible to the wake lane and attempted in **< 100 ms** with no tick wait (L4,
  needs L1).
- **Idle daemon** (isolated store, no traffic): over 30 s, deadline, wake and request paths make zero
  durable commits (the update-hook counter stays 0), and each lane wakes ≤ 1 time per 5 s tick (Pacer
  idle hook). Observation-origin commits kick no lane, so the wake lane receives no kicks at idle (D1,
  design roast round 2). The observation lane makes ≤ one cycle's commits per 5 s step (L4 and L5). The retention
  lane makes zero commits when nothing is published and ≤ 1 per 60 s tick while observation publishes at
  idle (skip-unchanged deferred; that retention idle bound is asserted by ht-p03.12.6).
- **Herdr stopped mid-run** (isolated named session, ht-p03.1): the observation lane makes ≤ 1 durable
  commit per backoff step, reaches the 30 s cap, and Health shows `retrying (attempt N …)`. After
  restart, the first publish returns it to the 5 s cadence (L5). This asserts the observation lane
  only (D4 scope note; the wake-lane bound is the parked escalation, and the ≤ 2 commits per seat per
  step that L4 asserts records D5's cost as specified).
- **Skip contract** (L5): a first failure whose marking pages error part-way leaves
  `last_invalidation_reason` clear, so the next same-reason failure re-runs the full invalidation from
  ordinal 0 and marks the seats past the abort point unresolved. A skipped failure returns
  `InvalidationRepeated`: Health keeps `ObservationInvalidated(reason)`, host evidence records the
  invalidation, no continuation is armed, and the Pacer attempt count increases.
- **Commit-kick attribution** (L1): a request commit on `wake_work` whose turn is paused between
  releasing the guard and kicking, while a `Wakes`-origin turn commits a `Wakes`-mapped table in that
  gap, still kicks `Wakes` exactly once from the request turn. The `Wakes` turn kicks nothing for its
  own origin.
- **Fence miss** (L3): a reservation cleared by a host invalidation between reserve and a `Refused`
  completion gives a 0-row restore, no error, `retry_step` advanced by exactly one, and no in-memory
  guard restore.
- **Pre-send refusal**: a fake host that refuses k times then accepts gives retries on 100 ms × 2ⁿ
  backoff and `retry_step` unchanged; then `Submitted` advances exactly one step and the offered
  frontier is restored after each refusal (L3).
- `SessionLease::drop` while a decision guard is held does not block a current-thread runtime: another
  task on the same runtime makes progress (L2).
- `rg 'sleep\(Duration::from_millis\((10|20)\)\)' src/daemon src/client src/service` finds nothing
  (L2, L4 and L5: L5 replaces the observation lane's `spawn_observation_loop` 20 ms turns).

## Configurations exercised

From the parent's Herdr up/down table:
- **Herdr stopped while daemon runs**: L5's backoff and commit-count test.
- **Herdr restarted under a running daemon**: L5's recovery-to-cadence test.
- **Herdr up**: L4's latency and idle tests.

All of these run on the ht-p03.1 isolated session, never on the shared server.

## Explicit deferrals

| Item | Reason |
|---|---|
| Idle observation commit (admission + generation every 5 s with an unchanged host) | Needs a skip-unchanged-snapshot change in the identity fencing protocol (B5-adjacent). B1 bounds the generation growth. |
| Exact next-due wake-up for the deadline lane | Minute-scale deadlines; a ≤ 5 s lateness is documented. A store `MIN(deadline)` query can follow real need. |

## Post-Implementation Notes

> *As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
