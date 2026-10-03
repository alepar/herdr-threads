# B1 Retention and live-row discovery: partial indexes, retention lane, query costs (design)

Parent spec: [2026-10-01-remaining-herdr-threads-findings-design.md](./2026-10-01-remaining-herdr-threads-findings-design.md)
(§B1, §Sequencing) · Bead: `ht-p03.12` (epic, promoted at the root split) · Root epic: `ht-p03` ·
Sibling spec consumed: [pacer adoption](./2026-10-01-remaining-herdr-threads-findings--pacer-adoption-design.md)
(`ht-p03.9`) · Mode B (autonomous, caller-invoked).

## Goal

Every background discovery pass (observation walk, work-job discovery, wake discovery and its attention
rebuild) does work proportional to *live* rows only, proven by a cost-flatness test per path as settled
history grows 10×. A retention lane on the Pacer keeps the snapshot tables bounded (current + previous
generation plus pinned rows) across 1,000 publishes and prunes completed work jobs after 24 h, in
transactions of at most 256 rows. Paged reads size their pages in linear time, and the human `read`
and `show`/`participants` paths make a bounded number of daemon calls per page.

Promotion rationale (seed): five index predicates, the attention rebuild rewrite, the retention lane, a
13-site page-sizing helper, W6-R1 and Wave 27. The parent's "snapshot generations = current + at most one
staged" cannot be a static partial-index predicate, so this design decides how to express it.

Ancestor goal (root `## Goal`, verbatim prefix): *herdr-threads at the integration tip is ready to tag
v0.1.0: the daemon stays fast and quiet over weeks of retained history and through Herdr outages, …*

## Problem description

The findings doc (§B1, 8 items) and the code agree. Four background paths and three read paths walk
whole tables:

- **Observation walk** (`store/seats.rs:1217`, `snapshot_seat_page`): pages `seats WHERE instance_id=?
  AND ordinal BETWEEN …`, retired seats included, 16 per page, every 5 s cycle.
- **Work-job discovery** (`store/mod.rs:689`): walks `work_jobs` by ordinal over *all* rows and filters
  `status IN ('pending','failed')` in Rust. The `work_jobs_ready(status, kind, ordinal)` index is unused.
- **Wake discovery** (`store/mod.rs:838`, `wake_candidates_page`): walks every seat of the instance
  (retired too). For each one it runs `effective::scan_effective_seat_attention`. That scan walks the
  seat's whole invitation and receipt history, then **every warning message in the database**
  (`effective.rs:493`: `messages WHERE kind='warn'` and `prepared_unavailable_warnings` by ordinal),
  judging each for recipiency. Cost is O(seats × all warnings) per pass. Seats with a `wake_work` row
  are returned as candidates whatever their attention (`historical`).
- **Human seats (Wave 18)**: a seat whose current binding is `harness='human'` has pending ACKs, so it
  becomes a candidate every pass. Only later does `wake.rs:218` (`current_authority`) refuse it.
- **Snapshots (`seats.rs:702`)**: `publish_snapshot_stage` leaves the superseded generation at
  `status='published'` for ever, together with its `snapshot_targets` rows: one generation plus one
  target row per pane every 5 s. `discard_snapshot_stage` (`seats.rs:1393`) deletes the target rows of
  failed stages, but only one ≤ 16-row quantum (`reconcile.rs:770`: `let _ = …`). It never deletes the
  generation row. The "durable cleanup worker" its comment mentions does not exist.
- **Page sizing (`queries.rs:990` and siblings)**: each paged query clones the item vector, pushes the
  candidate and re-encodes the **whole page** (`encode_selected`) per candidate. That is O(n²) per page,
  across 14 `let required = encode_selected(…)` sites in `queries.rs`. Three more `internal_page_bytes`
  pop-and-re-encode loops sit in `store/mod.rs` (595, 745, 937).
- **W6-R1**: `show`/`participants` mark the caller's own row through `CallerNeed::SelfMarker →
  pane_seat` (`cli/mod.rs:1035`). That opens a **second daemon connection** and walks every seat page
  through `collect_pane_seats`.
- **Wave 27**: the human `read` transcript (`cli/follow.rs`, `NickCache`, `LiveLookup`) issues a
  `SeatInspect` and a `pane_names` call per author, plus one `Message` body fetch per clipped preview.

## Main challenges

- **The generation predicate is not static.** "Current published" is defined by a pointer
  (`host_instances.active_snapshot_id`), not by a column value. A superseded generation keeps
  `status='published'`, so no `WHERE` clause can tell the current one from a superseded one. Adding a
  status value would mean rebuilding `snapshot_generations`, because SQLite cannot alter a `CHECK`.
  Four foreign keys and two triggers reference that table (`host_instances.active_snapshot_id`,
  `recovery_baseline_generation_id`, `snapshot_targets.generation_id`,
  `recovery_baseline_releases.baseline_generation_id`, `seats.unresolved_from_generation_id`).
- **Generations are pinned by more than "current".** The recovery baseline, any unresolved seat's
  `unresolved_from_generation_id`, and recovery-baseline releases all reference older generations, and
  the FKs (`foreign_keys=ON`, `connection.rs:87`) would reject deleting them.
- **Settled rows are often load-bearing markers, not garbage.** A completed `preparation_cleanup` job is
  read as a completion marker (`messages.rs:190`). The existence of a `warning_jobs` row means "already
  attributed" to the backlog walk (`attention.rs:711`). The effective warning readers also read
  `warning_jobs` for `interval_high_water`/condition (`effective.rs:1431, 1956, 1996`). Pruning those
  rows would change behaviour.
- **The wake frontier must stay exact.** The wake candidate carries invitation/receipt/warning frontiers
  that the ladder and the offered-frontier comparison use. A cheaper attention rebuild must produce the
  same values, or wakes are skipped or repeated.
- **Idle commits.** The Pacer epic promises zero idle commits from the retention lane. But at idle the
  observation lane still publishes a generation every 5 s (deferred skip-unchanged, B5-adjacent), so
  there is always something to prune.
- **Page fitting must be byte-identical.** Text output is not additive per item (columns, field
  selection, the `CachedCheckInPage` escaping). A cheaper sizer must choose exactly the page boundary the
  quadratic loop chose.

## Key decisions made

1. **Generations: pointer joins plus pruning, no live partial index.** Every reader already reaches live
   generations by primary-key pointer joins (`h.active_snapshot_id`, `s.unresolved_from_generation_id`).
   No discovery query scans generations. "Current + at most one staged" is therefore enforced by
   *bounding the table*, not by indexing it. The retention lane keeps {active, previous published,
   recovery baseline, pinned by unresolved seats or baseline releases, in-flight stages}. Retention finds
   its candidates through a new plain index `snapshot_generations_retention(instance_id,
   admission_sequence)`. There is no schema rebuild and no change to the publish path.
2. **Two new live partial indexes, and nothing for the classes already covered.** `seats_live_ordinal
   (instance_id, ordinal) WHERE state!='retired'` serves the observation walk, wake discovery and the
   publish nonretired probe. `work_jobs_live(ordinal) WHERE status IN ('pending','failed')` serves
   work-job discovery. Pending receipts/ACKs and unresolved warnings already have pending-only access
   paths: the v1 partial indexes and the v8 digest projections, which triggers keep in the writers'
   transactions. The rewritten paths use those with `INDEXED BY`.
3. **Wake attention is derived from the v8 pending projections.** It walks live, non-human seats and
   computes each seat's frontiers with `attention.rs`'s pending-source walks and canonical judges. A
   seat is examined only when one of its O(1) pending probes hits, and it is emitted only if it has
   actionable work (`wake_work` is not probed; see D1).
   `scan_effective_seat_attention` leaves the production wake path and stays as the test oracle.
4. **Retention is a fifth lane, time-driven, on the Pacer.** It has a 60 s safety tick and an empty kick
   set (the reserved `Lane::Retention` row in `kicks.rs` maps no table). It prunes snapshot generations
   and targets, completed `send_attention` / `warning_attribution` / `receipt_timer_materialization`
   jobs after 24 h, and dead recovery-baseline releases. Each transaction holds ≤ 256 rows and lasts
   ≤ 5 ms. Warnings, receipts, retired seats and `preparation_cleanup` jobs are **retained**.
5. **One page-fit helper.** It estimates each item's size once (one single-item encode), keeps a running
   byte budget, and then does an exact boundary check that costs O(log n) full encodes. A differential
   test proves it returns the same boundary as the old greedy loop.
6. **CLI read costs.** `pane_seat` reuses the caller's connection and asks one target-filtered seat
   page. The human transcript resolves names once per page: one `participants` call per thread, one
   `pane_names` per page, and `SeatInspect` only for authors absent from both. Clipped bodies arrive
   inline through a new optional `HistoryQuery.full_bodies` flag.

## Consumed contracts

- **ht-p03.2 (v10 migration skeleton)**: `migrations/0010_*.sql`, registered in `src/store/schema.rs`.
  B1 appends its statements to that file and extends the v10 startup verification. If .2 drops
  `recovery_baseline_releases` / `recovery_baseline_generation_id` as verification-only, the matching
  pin and prune clauses below are simply omitted.
- **ht-p03.3 (post-collapse surface)**: `StorePort::wake_candidates` / work discovery without
  `Unsupported` defaults. Scheduler tests run on `SqliteStore`.
- **ht-p03.6 (per-test cost counter)**: the per-test VM-unit counter context that replaces the global
  `UNITS` in `tests/store/attention.rs:277`. Every flatness test here uses it.
- **ht-p03.7 (Pacer)**: `wait_blocking(tick) -> {Kicked, RetryDue, Tick, Cancelled}`,
  `on_failure`/`on_success`, `attempts`/`next_retry_at`, the idle hook, and the Notify-backed
  `Cancellation`.
- **ht-p03.9.1 (lane registry)**: `Lane::Retention` (a reserved variant), `lanes_for_table` (B1 leaves
  every table out of Retention's set: see D4), `kicks::enter_lane`, `WorkerStatus::attach_pacer`, and
  the per-origin commit counter. B1 does **not** redefine the `Lane` enum or the kick map. It only
  confirms that the Retention row stays empty.

## Decision points

### D1. Live predicate per table, and the v10 contents B1 appends

| Table | Live (discovery reads) | Settled | Discovery access path | Retention |
|---|---|---|---|---|
| `seats` | `state!='retired'` | retired | **new** `seats_live_ordinal(instance_id, ordinal) WHERE state!='retired'` | keep (user-visible history; B5 tombstones) |
| `work_jobs` | `status IN ('pending','failed')` | `complete` | **new** `work_jobs_live(ordinal) WHERE status IN ('pending','failed')` | prune `complete` after 24 h, three kinds (D4) |
| `snapshot_generations` | active, previous, baseline, pinned, in-flight | everything else | PK pointer joins (unchanged) | prune (D3) via **new** `snapshot_generations_retention(instance_id, admission_sequence)` |
| `snapshot_targets` | rows of a live generation | rows of a pruned one | `snapshot_targets_generation_ordinal` (existing) | deleted with their generation |
| `receipts` / `receipt_state` | `state='pending'` | settled | existing partial `receipts_*_pending*`, `receipt_state_pending_due`, `digest_pending_manifest_receipts` | keep |
| `invitations` | `state='pending'`, not cancelled | settled | existing `invitations_due`, `invitations_*_pending*`, `digest_pending_invitations` | keep |
| warnings (`messages kind='warn'`, `warning_jobs`, `warning_recipients`) | condition open | condition settled | v8 `digest_open_warnings`, `digest_open_warning_recipients`, `digest_programmatic_warnings`; `warning_jobs_ready` for the backlog | **keep** (D4) |
| `wake_work` | not a discovery predicate (see note below) | n/a | read by PK in `wake::load_candidate` only for seats a pending probe selected; reservations are found by the recovery walk (`store/mod.rs:533`; Amended by coverage r2, 2026-10-01: rewritten by ht-p03.12.9 to read `wake_work_reserved` only) | keep (one row per seat) |
| `recovery_baseline_releases` (if it survives .2) | `baseline_generation_id = h.recovery_baseline_generation_id` | older baselines | PK prefix | prune non-current |

**`wake_work` has no live predicate (verified against code, 2026-10-01).** The proposed
`reservation_id IS NOT NULL OR reason_bits!=0 OR attention_version!=checkpoint_version` is rejected because
it degenerates to the old `historical` rule (every seat that ever had a `wake_work` row):
- `checkpoint_version` is never written. Its only occurrences are the column default 0
  (`migrations/0001_initial.sql:458`), the read at `store/wake.rs:101` and an equality check at
  `store/wake.rs:409`. Meanwhile `attention_version` is incremented on every attention bump
  (`store/materialization.rs:467`, `store/control.rs:1308`, `store/control.rs:1449`,
  `store/service_controls.rs:417`). So `attention_version!=checkpoint_version` stays true forever once any
  bump happens.
- `reason_bits` is set (`|1`) on invitation (`control.rs:1308`, `control.rs:1449`,
  `service_controls.rs:417`). It is cleared only at retirement (`control.rs:137`). No scheduler,
  notification or service code reads it.
- `reservation_id IS NOT NULL` is already covered by the recovery walk (`store/mod.rs:533`, a
  `LEFT JOIN wake_work` over all seats — Amended by coverage r2, 2026-10-01: superseded, the walk reads the reserved set through `wake_work_reserved`, ht-p03.12.9) and by in-memory completions (`scheduler/mod.rs:570`). It is not
  covered by candidate discovery: `wake::reserve` refuses a candidate whose `reservation_id` is set
  (`store/wake.rs:405`).
- The settle ("clearing") pass that the `historical` rule appeared to provide is a no-op in production.
  Today a seat with a `wake_work` row and no actionable attention is emitted (`store/mod.rs:910-917`).
  Its only consumer, `WakeRunner::try_candidate`, returns `Ok(None)` before touching any durable or
  in-memory state when `AttentionSnapshot::select()` is `None` (`scheduler/mod.rs:407-415`;
  `select` = `has_actionable_work` minus the retired test, `notification/policy.rs:36-48`). So dropping
  settled seats from discovery changes no observable behavior. A probe-free seat never needs to be
  examined to "clear" anything.

**v10 statements B1 appends** (after .2's drops). They are verified at startup by normalized
`CREATE` comparison, the same way `verify_existing_v7` does it today:

```sql
CREATE INDEX seats_live_ordinal ON seats(instance_id, ordinal) WHERE state!='retired';
CREATE INDEX work_jobs_live ON work_jobs(ordinal) WHERE status IN ('pending','failed');
ALTER TABLE work_jobs ADD COLUMN completed_at INTEGER CHECK(completed_at IS NULL OR completed_at >= 0);
CREATE INDEX work_jobs_retention ON work_jobs(completed_at) WHERE status='complete';
CREATE INDEX snapshot_generations_retention ON snapshot_generations(instance_id, admission_sequence);
```

**Amended by coverage r2, 2026-10-01 (ht-p03.12.1 owns the list now; the block above is superseded where it differs):**

```sql
-- ht-p03.12.1: five indexes plus the column; exact SQL is the contract
CREATE INDEX seats_live_ordinal ON seats(instance_id, ordinal) WHERE state!='retired';
CREATE INDEX work_jobs_live ON work_jobs(ordinal) WHERE status IN ('pending','failed');
ALTER TABLE work_jobs ADD COLUMN completed_at INTEGER CHECK(completed_at IS NULL OR completed_at >= 0);
CREATE INDEX work_jobs_retention ON work_jobs(kind, completed_at) WHERE status='complete';
CREATE INDEX snapshot_generations_retention ON snapshot_generations(instance_id, admission_sequence);
CREATE INDEX wake_work_reserved ON wake_work(seat_id) WHERE reservation_id IS NOT NULL;
```

- `work_jobs_retention` is keyed by `kind` first (~~`work_jobs_retention(completed_at)`~~ superseded): retained `preparation_cleanup` rows fall outside the pruned kinds' ranges.
- `wake_work_reserved` (coverage r1; consumed by the recovery walk, ht-p03.12.9) is new: the recovery walk reads `wake_work INDEXED BY wake_work_reserved WHERE reservation_id IS NOT NULL` and joins seats by id, in seat-ordinal order.
- `occupant_bindings_current` is verified, not created, with the `(seat_id, ended_at IS NULL)` shape the human-seat exclusion in D2 needs, together with the six pending probes ht-p03.12.5 uses (`digest_pending_invitations_seat`, `digest_pending_manifest_receipts_seat`, `receipts_seat_state_ordinal`, `digest_open_warning_recipients_seat`, `digest_open_warnings_affected`, `digest_programmatic_warnings_seat`); any missing one is created in v10 and added to ht-p03.12.1's owned list.

- `completed_at` is written by the Rust write path that sets `status='complete'` (`store/work.rs:111` and
  any `materialization.rs` sibling), using the injected `Clock`. No SQL clock is used, so tests stay
  deterministic.
- Rows completed before v10 keep `NULL`, and retention treats `NULL` as already past 24 h.
- The `ALTER … ADD COLUMN` needs no table rebuild.

**Queries name their index** (`INDEXED BY`), so a missing or altered index fails at prepare time:

- Observation walk: `… FROM seats INDEXED BY seats_live_ordinal WHERE instance_id=?1 AND state!='retired'
  AND ordinal>?2 AND ordinal<=?3 ORDER BY ordinal LIMIT ?4`. The high-water stays `MAX(ordinal)`, so
  cursor semantics are unchanged.
- Publish nonretired probe (`seats.rs:664`): the same index with `LIMIT 1`.
- Work discovery: `… FROM work_jobs INDEXED BY work_jobs_live WHERE status IN ('pending','failed') AND
  ordinal>?1 AND ordinal<=?2 ORDER BY ordinal LIMIT 100`.
  - The 100-row visit cap now counts live rows only.
  - The ascending ordinal cursor and the `WorkJobs` cursor scope are unchanged.
  - The Rust status filter becomes an assertion.

**Considered**:
- *Use `work_jobs_ready(status, kind, ordinal)` as the parent suggested*: rejected. It orders by status
  first, so one ascending-ordinal stream over pending ∪ failed needs a two-cursor merge, and the
  continuation cursor would change shape.
- *A `superseded` status value on generations*: rejected. It needs a `CHECK` rebuild of an FK-referenced
  table, plus an extra write in the B5-adjacent publish fence.
- *A nullable `superseded_at` column flipped at publish*: rejected for the same publish-path reason, and
  it is unnecessary because `admission_sequence` already orders generations per instance.

### D2. Wake discovery and attention rebuild (closes `mod.rs:838`, `effective.rs:493`, Wave 18)

`wake_candidates_page` is rewritten. The cursor scope (`WakeCandidates`), the `high_water` = `MAX(ordinal)`
of the instance's seats and the 100-unit examined cap stay.

1. **Seat walk**: `seats INDEXED BY seats_live_ordinal WHERE instance_id=?1 AND state!='retired' AND
   ordinal>?2 AND ordinal<=?3 AND NOT EXISTS (SELECT 1 FROM occupant_bindings INDEXED BY
   occupant_bindings_current WHERE seat_id=seats.id AND ended_at IS NULL AND harness='human')`.
   - Retired seats are never visited.
   - Human-bound seats are never visited (Wave 18). Their mail waits for a manual ACK, and
     `current_authority` would refuse them anyway, so no candidate, reservation or ladder step is ever
     produced for them.
2. **Pending probe** per seat: up to six O(1) `EXISTS` probes on seat-leading pending indexes.
   - `digest_pending_invitations_seat`.
   - `digest_pending_manifest_receipts_seat`.
   - `receipts_seat_state_ordinal` with `state='pending'`.
   - `digest_open_warning_recipients_seat`.
   - `digest_open_warnings_affected`.
   - `digest_programmatic_warnings_seat` above `notice_frontier`.
   - The seat goes on to step 3 only if a probe hits. `wake_work` is **not** probed (see the D1 note:
     every candidate `wake_work` predicate degenerates to "ever woken", and the settle pass is a no-op).
   - **Emission rule**: an examined seat is put in the page iff `candidate.has_actionable_work()`. This
     replaces `has_actionable_work() || historical` (`store/mod.rs:910-917`).
   - Settle behavior, now pinned by a test: once a seat's attention settles, it is never again in a
     candidate page. Its `wake_work` row (retry ladder, `last_reserved_frontier`) is untouched, so the
     next actionable episode resumes the ladder exactly as today. This is equivalent to today, because
     `try_candidate` discards non-actionable candidates without side effects (`scheduler/mod.rs:407-415`).
     A seat that is mid-reservation when it settles is still reached by the recovery walk
     (`store/mod.rs:533`) and by completions (`scheduler/mod.rs:570`).
   - **Recovery walk** (Amended by coverage r2, 2026-10-01, ht-p03.12.9): `wake_recovery_candidates` is rewritten to walk `FROM wake_work INDEXED BY wake_work_reserved WHERE reservation_id IS NOT NULL` and join seats by id, keeping seat-ordinal order and the old result set, so its cost no longer grows with settled or retired history. Its continuation cursor is the recovery cursor delivered by the seam ht-p03.12.10, round-tripped by ht-p03.12.11.
   - **Cursor seams** (ht-p03.12.10 contract, ht-p03.12.11 integration): the post-D2 wake cursor, the work cursor and the recovery cursor are Rust types with `longest_cursor_bytes()` bounds that feed the `PageFit` base; a pre-rewrite wake cursor carrying the removed attention fields is `InvalidCursor` once ht-p03.12.5 switches the rejection on. The counting fake `LocalClient` seam for the D6 read-cost tests is ht-p03.12.12 (contract) and ht-p03.12.13 (integration).
3. **Attention**: a new `attention::wake_seat_attention(db, seat)` computes the same struct that
   `scan_effective_seat_attention` returns to the wake path. That is `has_pending_invitation`,
   `invitation_frontier`, `has_pending_receipt`, `receipt_frontier_seq` and `actionable_warning` (seq,
   offset), plus the decision-seq position.
   - It uses `attention.rs`'s existing per-source newest-first walks (`pending_invitations`,
     `pending_receipts`, `seat_pending_warnings` / `projected_pending_warnings`, `warning_backlog`).
   - It uses the same judges: `effective_receipt`, `is_warning_recipient`, `warning_condition_actionable`.
   - Each frontier is the maximum over the judged items of its source. Keyed sources (invitations,
     manifest receipts) walk in logical-key order, so the first accepted item is the exact maximum.
     Ordinal-ordered sources (physical receipts, warnings) carry the newest ordinal first.
   - It is exact except for the edge case `attention.rs`'s header documents: a preparation staged before,
     and published after, a full window of newer rows. That edge is accepted here exactly as the digest
     accepts it.
   - `has_*` flags are exact whenever any pending row exists.
   - Cost is O(`WINDOW` × sources) per examined seat, independent of history.
   - **Decision-seq position**: it is not derived from the walks' contents. It is `host_instances.decision_seq`
     for the seat's instance, read once with
     `SELECT h.decision_seq FROM seats s JOIN host_instances h ON h.id=s.instance_id WHERE s.id=?1` inside
     the page's `BEGIN DEFERRED` read transaction (`store/mod.rs:817-819`), before the walks. That is
     exactly what `scan_effective_seat_attention` does today (`store/effective.rs:370-374`). It is the
     snapshot revision that the walks' results are valid at, not a maximum over walked rows.
     `wake::load_candidate` stamps it into the `WakeAttentionWitness` (`store/wake.rs:158`).
     `wake::reserve` re-reads `h.decision_seq` in its write transaction and refuses the reservation if
     anything was decided since (`store/wake.rs:398-401` with `WakeAttentionWitness::valid_at`,
     `ports.rs:2066-2079`). Using the maximum walked `decision_seq` instead would wrongly accept a
     reservation after an unrelated decision that changed this seat's attention without adding a newer
     row (for example, an ACK settling a receipt).
   - `wake_seat_attention` completes within the call (bounded by `WINDOW`), so there is no mid-seat
     continuation. The new path never emits the cursor's `attention` / `last_examined_key` /
     `scope_revision` fields (`store/mod.rs:809-850`). Wake cursors live only in the scheduler's in-memory
     scan state (`scheduler/mod.rs:205-230`), so no cursor crosses a daemon version. A cursor that carries
     those fields is rejected as `InvalidCursor`. Each examined seat counts as one unit toward the
     100-unit cap.
4. `wake::load_candidate` is unchanged. The page-byte fit at the tail is D5's.

**Per-pass cost**: O(live non-human seats × sources) probes, plus O(`WINDOW` × sources) per seat with
pending attention. It is independent of retired seats, settled obligations and the global warning count.

**Considered**:
- *Patch only the warnings fold in `scan_effective_seat_attention`*: rejected. Its invitation and receipt
  scans still walk the seat's settled history.
- *A new `wake_live_seats` projection kept by triggers*: rejected. It needs reference counting across
  six source tables, and the per-seat probes are already O(1) over live seats.
- *A partial index excluding humans*: impossible. "Human" lives on the current binding, not on
  `seats`, so the `NOT EXISTS` probe on the unique `occupant_bindings_current` is O(1).

### D3. Snapshot retention (closes `seats.rs:702`): how "current + at most one staged" is expressed

For each instance `h`, a generation `g` is **pinned** (never pruned) when any of these holds:

- `g.id = h.active_snapshot_id` (current).
- `g` is the **previous** published generation, meaning the published `g` with the greatest
  `admission_sequence` below the active one's. This is a grace for readers that captured the old pointer.
- `g.id = h.recovery_baseline_generation_id`.
- An unresolved seat references it (`seats_unresolved_generation`, partial, existing).
- A `recovery_baseline_releases` row for the *current* baseline references it.
- `g.status IN ('building','sealed') AND g.admission_sequence > h.observation_decided_sequence`
  (in-flight stage).

Everything else is **prunable**:

- Superseded published generations older than "previous".
- Every `discarded` generation.
- **Dead stages**: `building`/`sealed` with `admission_sequence ≤ observation_decided_sequence`.
  `publish_snapshot_stage` rejects such a stage (`seats.rs:648`, `admission_sequence <= decided →
  StaleHostObservation`), so it can never publish. Pruning it changes no fence (B5-safe).

The retained set is therefore {current, previous, baseline, pins, in-flight}, independent of the number
of publishes. This is the bounded form of "current + at most one staged".

**Prune step** (`store/retention.rs`, new, one writer transaction, ≤ 256 rows, ≤ 5 ms quantum like
`discard_snapshot_stage`):

1. Select up to `k` candidate generations by `snapshot_generations_retention` (lowest
   `admission_sequence` first) and re-check the pin predicate inside the transaction.
2. Mark `published`/`building`/`sealed` candidates `discarded`.
3. Delete their `snapshot_targets` by `snapshot_targets_generation_ordinal`, up to the remaining row
   budget.
4. Delete the generation row once it has no targets left. The FKs enforce the order.
5. Delete `recovery_baseline_releases` rows whose baseline is not current. Those rows are consulted only
   with the current baseline (`effective.rs:169`, `seats.rs:3309-3325`).

The cleanup turn that already runs after a failed publish (`reconcile.rs:770`) is unchanged. Retention
finishes whatever it leaves.

**Upgrade backlog**: a weeks-old v9 database may hold about 10⁵ superseded generations. The lane drains
them one ≤ 256-row transaction at a time and re-runs immediately while `has_more`, releasing the writer
between batches. The fair writer interleaves request writes, so the upgrade never takes the writer for
more than one batch.

### D4. Retention lane, policy and Pacer wiring

**Policy**: fixed constants in `src/store/retention.rs`, not user-configurable in v0.1.0 (parent
deferral).

| Constant | Value | Applies to |
|---|---|---|
| `RETENTION_BATCH_ROWS` | 256 | every delete transaction (all tables together) |
| `RETENTION_QUANTUM_MS` | 5 | early stop inside a batch |
| `WORK_JOB_RETENTION` | 24 h | `work_jobs` with `status='complete'`, `kind IN ('send_attention','warning_attribution','receipt_timer_materialization')`, `completed_at ≤ now − 24 h` or `NULL` |
| snapshot keep set | D3 | `snapshot_generations`, `snapshot_targets` |

**Retained on purpose** (decided against the parent's "settled warnings after 7 days"):

- **Warnings.** `messages kind='warn'` is user-visible thread history. A `warning_jobs` row is the
  attribution record that `attention.rs:711` and `effective.rs:1431/1956/1996` read, and
  `warning_recipients` is the recipient record. Discovery already skips settled warnings: the v8 digest
  projections are pure pending sets, and their triggers delete rows on settlement. Pruning would change
  behaviour for no discovery gain.
- **`preparation_cleanup` jobs.** Their completed row is the marker `messages.rs:190` reads when an
  operation key is reused.
- **Retired seats, settled receipts and invitations** (parent decision; B5 relies on them).
- **`wake_work`** (one row per seat).

Why the three pruned job kinds are safe:

- `warning_attribution` and `send_attention` are created in the same transaction as their subject's own
  unique row: `warning_jobs.warning_id UNIQUE` and the published preparation.
- `receipt_timer_materialization` is keyed by a `seat_availability` AUTOINCREMENT ordinal
  (`seats.rs:2355, 3721`).
- So a pruned job's subject can never be enqueued again. Each kind gets a test that re-runs its producer
  after the prune and asserts it does not re-enqueue.

**Lane**: `start_retention_worker` in `src/service/workers.rs`, shaped like the Pacer-adoption lanes.

- It calls `kicks::enter_lane(Lane::Retention)` and owns an `Arc<Pacer>` attached to its `WorkerStatus`.
- It blocks in `pacer.wait_blocking(60 s)` with the lane `Cancellation`.
- Each pass runs `retention::prune_once(budget)`. That function runs the snapshot step and then the
  work-job step, each one transaction of ≤ 256 rows, and returns `has_more`.
- `has_more` triggers an immediate re-run. `Err` calls `on_failure()` and backs off (100 ms × 2ⁿ, 30 s
  cap). Success calls `on_success()`.
- **Kick set: empty.** `lanes_for_table` maps no table to `Lane::Retention`. The reserved row stays, but
  it is empty by decision, not left out by accident.
- Why not kick on `snapshot_generations`, as the .9 spec expected: the observation lane publishes every
  5 s at idle, so that kick would cost a retention commit every 5 s. On a 60 s tick, one transaction
  prunes the ~12 generations superseded in that minute.
- **Idle reconciliation** with the .9 acceptance ("zero idle commits from the retention lane"):
  - A store with no publication since the last pass and nothing past 24 h gets **zero** retention
    commits. A no-op transaction is not counted (.9.1 counter).
  - While the observation lane publishes at idle, retention makes **≤ 1 commit per 60 s tick**. That
    commit is caused by observation commits, which .9 already allows.
  - Accepted by the coordinator (2026-10-01). The ht-p03.9 acceptance, the root spec §B2 acceptance and
    the pacer spec D4 now read: zero from deadline, wake and request paths; retention: zero absent
    publication, ≤ 1 commit per 60 s tick while observation publishes at idle (skip-unchanged deferred).
- **Health and logs**: no new Health line (ht-p03.27 budget). On the first failure and on each new
  backoff step the lane logs one `warn` line; the Pacer backoff itself rate-limits it to ≤ 1 per 30 s at
  the cap. There is no dependency on ht-p03.11.
  (Amended by coverage r2, 2026-10-01: superseded — lane errors report through the ht-p03.39 `WorkerStatus::record_failure` → `LaneErrorLog::record(lane, &Error)` hook, not through the leaf's own `warn` line; ht-p03.11 implements the rate-limited logger behind that hook, and ht-p03.12.6 only calls `record_failure`.)
- **Shutdown**: cancellation wakes the blocked wait. A pass in progress finishes its current
  transaction and does not start another.

### D5. Page-fit helper (closes `queries.rs:990` and the 13 sibling sites)

`src/store/page_fit.rs` (new):

```rust
pub(crate) struct PageFit<'a> { output: &'a OutputSpec, max: usize, /* … */ }
impl PageFit<'_> {
    /// `render(k)` builds the CommandResult for the first k accepted items with the
    /// continuation cursor of item k (exactly what the old loop encoded).
    pub fn fit<I>(&mut self, items: &[I], render: impl Fn(usize) -> Result<CommandResult, ApiError>)
        -> Result<Fit, ApiError>; // Fit { accepted: usize, stop: Rows|Bytes, required_minimum: Option<u32> }
}
```

- **Estimate**: `base` is the length of `render(0)` with the longest possible cursor. Each item adds its
  single-item delta, `len(render_one(i)) − base` plus the separator, to a running total. That is one
  small encode per item, and it gives the estimated cut `k̂`.
- **Exact boundary**: encode `render(k̂)`.
  - If it fits, gallop upward (k̂+1, k̂+2, k̂+4, …) until a page does not fit. If it does not fit, gallop
    downward.
  - Then binary-search between the last fitting and the first non-fitting `k`.
  - That costs O(log n) full encodes. Because page length is monotone in `k`, the result equals the
    greedy loop's maximal prefix, in JSON and in text, with field selection and escaping.
- **Single item that cannot fit**: the same `InvalidBudget` / `required_minimum_bytes` errors as today,
  so error texts are unchanged.
- **Sites**: every `let required = encode_selected(…)` loop in `queries.rs` (14 by `rg` today; the
  findings doc says 13), the `internal_page_bytes` pop loops in `store/mod.rs` (595, 745, 937), and the
  `ensure_fit` callers that follow a loop. `ensure_fit` itself stays as the final assertion.
- **Ownership split with D2**: in `wake_candidates_page`, D2 owns the discovery loop and this leaf owns
  only the fit tail (from `let mut result = wake_page_at(` onward).

**Considered**: *a pure additive estimate with no exact pass*: rejected, because text output is not
additive and pages would differ. *Caching the encoded prefix*: rejected, because JSON arrays and text
tables cannot be extended in place without re-rendering the envelope.

### D6. CLI read costs (closes W6-R1 and Wave 27)

D6 ships as three leaves (split at this level, coordinator 2026-10-01, overruling a size-only PROMOTE):
W6-R1 (`ht-p03.12.3`), Wave 27 names (`ht-p03.12.7`) and Wave 27 bodies (`ht-p03.12.8`).

**W6-R1** (`ht-p03.12.3`):
- `pane_seat` takes the already-open `client` (one connection per CLI invocation). The `SelfMarker`
  path asks one `SeatsQuery { target: Some(pane), page: { limit: 2, … } }`. The target filter at
  `queries.rs:990` already probes `seats_live_target` and the unresolved index.
- `collect_pane_seats`'s full page walk is used only where every seat is genuinely needed. None of
  `show`, `participants` or `CallerNeed::SeatDefault` needs it.
- The `limit: 2` keeps the "maps to more than one seat" error.

**Wave 27** (human `read`, `cli/follow.rs`):
- **Names** (`ht-p03.12.7`): before rendering a history page, `NickCache` fetches one `Participants` page per thread
  and reads one `pane_names` snapshot per page (when stale), so not once per author. It calls
  `SeatInspect` only for an author found in neither, then caches the result across pages.
- **Bodies** (`ht-p03.12.8`): `HistoryQuery` gains `#[serde(default, skip_serializing_if = "is_false")] full_bodies:
  bool`.
  - When it is set, the daemon puts complete bodies into the page, still under the page byte budget and
    sized by D5's helper.
  - A body that does not fit even alone keeps today's clipped preview plus body cursor. The CLI fetches
    only those through `Message`. That is rare and bounded by one fetch per oversize message.
  - The field is omitted from the wire when false, so existing request bytes are unchanged.
  - Same-install CLI and daemon always agree. Against an older daemon, the CLI falls back to today's
    per-preview fetch when the reply classifies as the `VersionSkew` error class (ht-p03.10). It never
    string-matches an `InvalidRequest` detail, and an `InvalidRequest` of any other class is surfaced.
  - Amended by coverage r2, 2026-10-01: the `VersionSkew` fallback just described is superseded. The older-daemon fallback is the ht-p03.43 handshake capability: the daemon's hello reply advertises `history.full_bodies` (discovered through a separate `Command::Capabilities` request), `protocol_version` is not bumped, and the CLI sends `full_bodies` only when the capability is present; an older daemon reads as no capabilities and the CLI uses per-preview `Message` fetches without ever sending the field.
  - The daemon branch sizes the page through D5's `PageFit` helper, which owns the history fit loop
    (`ht-p03.12.2`). The bodies leaf therefore depends on `ht-p03.12.2` and `ht-p03.10`.

Acceptance counts daemon calls with a counting fake `LocalClient`. B10 (presentation) is not touched:
the rendered transcript bytes are identical before and after.

## Leaves (decomposition)

Children of `ht-p03.12`. Edges are leaf-level and sparse, and each one is paired with a `blocked-by`
reason line.

| Leaf | Scope | Owns | Consumes (edge) |
|---|---|---|---|
| L1 `ht-p03.12.1` — v10 B1 schema: live and retention indexes, `work_jobs.completed_at` | D1 statements, startup verification, `completed_at` stamping, v9→v10 migration test | ~~the four index names~~ the five index names (`seats_live_ordinal`, `work_jobs_live`, `work_jobs_retention` keyed by kind, `snapshot_generations_retention`, `wake_work_reserved`) and their exact SQL, any pending-probe index L1 had to add, the `completed_at` column and its writer (coverage r2) | v10 skeleton file + registration (ht-p03.2) |
| L2 `ht-p03.12.4` — Observation and work-job discovery read live rows only | D1 queries for `seats.rs:1217`, publish probe, `mod.rs:689`; flatness tests | — | L1 (`seats_live_ordinal`, `work_jobs_live`); per-test cost counter (ht-p03.6) |
| L3 `ht-p03.12.5` — Wake discovery from pending projections; human seats excluded | D2 | `attention::wake_seat_attention` | L1 (`seats_live_ordinal`); ht-p03.6; post-collapse `wake_candidates` surface (ht-p03.3) |
| L4 `ht-p03.12.6` — Retention lane: snapshot and work-job pruning on the Pacer | D3, D4 | `src/store/retention.rs` (policy constants, `prune_once`), `start_retention_worker`, the empty Retention kick row | L1 (`snapshot_generations_retention`, `work_jobs_retention`, `completed_at`); Pacer (ht-p03.7); lane registry (ht-p03.9.1); post-deletion tables (ht-p03.2, transitively via L1) |
| L5 `ht-p03.12.2` — Linear page fitting through one helper | D5 | `src/store/page_fit.rs` | — |
| L6 `ht-p03.12.3` — CLI self-marker seat page reuses the caller connection (W6-R1) | D6 W6-R1 | — | — |
| L7 `ht-p03.12.7` — Batched human read: resolve names once per page (Wave 27 NickCache) | D6 Wave 27 names | — | — |
| L8 `ht-p03.12.8` — History `full_bodies`: wire field, daemon branch via the D5 page helper, older-daemon fallback | D6 Wave 27 bodies | `HistoryQuery.full_bodies` | L5 (`PageFit` helper owning the history fit loop); ~~`VersionSkew` error class (ht-p03.10)~~ superseded by the ht-p03.43 `history.full_bodies` capability (Amended by coverage r2, 2026-10-01) |
| L9 `ht-p03.12.9` — Wake recovery walk reads reserved seats only (Amended by coverage r2, 2026-10-01) | D1 `wake_work` note, D2 recovery walk | the rewritten `wake_recovery_candidates` | L1 (`wake_work_reserved`); ht-p03.6; ht-p03.3; seam ht-p03.12.10 |
| Seams (Amended by coverage r2, 2026-10-01) | ht-p03.12.10 (contract: wake, work and recovery cursor types, `longest_cursor_bytes()`, removed-field rejection switch, `page_fit.rs` stub) and ht-p03.12.11 (integration: cursors round-trip across pages and match the PageFit oracle); ht-p03.12.12 (contract: `CountingLocalClient`, scripted older-daemon capabilities via ht-p03.43) and ht-p03.12.13 (integration: per-invocation read-cost counts) | the boundary types and the integration tests | L2, L3, L5, L9; L6, L7, L8 |

Critical path inside the epic: ht-p03.2 → L1 → {L2, L3, L4}, which is 2 rounds after .2 (L4 also waits
for .7 and .9.1). L5, L6 and L7 run from round 1, and only L5 gates anything (L8). L8 waits for L5 and
ht-p03.10.

Shared files:
- `store/mod.rs`: L2 (work discovery fn), L3 (wake discovery loop), L5 (fit tails). These are disjoint
  functions or regions.
- `store/seats.rs`: L2 (walk and probe), L4 (none; retention lives in a new file).
- `queries.rs`: L5 (fit loops), L8 (history `full_bodies` branch). L8 adds a branch inside the history
  fit loop that L5 converts, so L8 is blocked by L5 (artifact: the `PageFit` helper).
- `cli/follow.rs`: L7 (`NickCache`) and L8 (body fetch and fallback). These are disjoint regions, and the
  merge gate handles them; there is no artifact edge.

## Acceptance (epic level, distributed to leaves)

- **Cost flatness** (per-test counter, ht-p03.6), one test per path. Each test grows settled history 10×
  and asserts that per-pass VM units stay within 1.1× + a fixed constant of the base size.
  - Settled history means: retired seats, completed jobs, superseded generations, settled
    invitations/receipts and settled warnings.
  - Paths: observation walk (L2), work discovery (L2), wake discovery (L3).
- **Wake equivalence** (L3): on fixtures below `WINDOW`, `wake_seat_attention` equals
  `scan_effective_seat_attention` field by field (the oracle). A human-bound seat with pending ACKs is
  never a candidate and never reserved. A seat whose attention settles is never again in a candidate page, and its `wake_work` row is unchanged.
- **Migration** (L1): a v9 fixture database migrates to v10. ~~The four indexes~~ (superseded, Amended by coverage r2, 2026-10-01: the ht-p03.12.1 list — `seats_live_ordinal`, `work_jobs_live`, `work_jobs_retention` keyed by kind, `snapshot_generations_retention`, `wake_work_reserved` — plus the verified `occupant_bindings_current` and pending probes) match their exact SQL,
  every `INDEXED BY` statement prepares, and pre-v10 completed jobs have `completed_at IS NULL`.
- **Bounded snapshots** (L4): after 1,000 publishes the generation count is ≤ |keep set| + 1 and the
  target count is ≤ (|keep set| + 1) × panes.
  - Every pin case survives: baseline, unresolved seat, current-baseline release, in-flight stage.
  - A dead stage is pruned, and a stage above `decided` is not.
  - Every retention transaction touches ≤ 256 rows.
  - A 10⁴-generation upgrade backlog drains while a concurrent request write completes within one batch.
- **Work-job retention** (L4): completed jobs of the three kinds are gone 24 h after `completed_at`
  (fake clock) or at the first pass when `NULL`. `preparation_cleanup` jobs and pending or failed jobs
  stay. Re-running each pruned kind's producer does not enqueue it again.
- **Recovery walk** (L9, Amended by coverage r2, 2026-10-01): per-pass recovery-walk VM units stay within 1.1× + a constant while retired seats and settled unreserved `wake_work` rows grow 10×, using `wake_work_reserved`; the new walk returns the old walk's candidates in the same order, and a multi-page walk round-trips its recovery cursor (seam ht-p03.12.10/.11).
- **Idle** (L4, .9.1 counter): no publication and nothing due gives zero Retention-origin commits over
  2 ticks. Observation publishing at idle gives ≤ 1 Retention-origin commit per 60 s tick.
- **Page fit** (L5): a differential test over randomized item sizes, in JSON and text, with field
  selection, gives a page boundary and bytes identical to the old greedy loop at every site. Encode calls
  per page are ≤ n + 2·⌈log₂ n⌉ + 2.
- **CLI** (L6, L7, L8):
  - L6: `participants`/`show` with a caller pane make 1 daemon connection and ≤ 2 calls (1 seats page +
    the read).
  - L7: a 100-message human `read` page with 10 authors makes ≤ 1 participants + 1 `pane_names` call
    and 0 `SeatInspect` calls when every author resolves from those two.
  - L8: with `full_bodies`, the same page with 5 clipped previews makes 1 history call and 0 `Message`
    calls. ~~A `VersionSkew` reply falls back to per-preview fetches.~~ Superseded (Amended by coverage r2, 2026-10-01): against a daemon that does not advertise `history.full_bodies` (ht-p03.43 capability, no `VersionSkew` reply involved) the CLI never sends the field and falls back to per-preview fetches.
  - The rendered transcript bytes are unchanged (L7, L8).

## Configurations exercised

This epic needs none of the parent's runtime matrices: no harness, no Herdr up/down and no release
target is specific to B1. Herdr down is exercised indirectly: dead and discarded stages from failed
captures are pruned (L4 test, fake host).

## Explicit deferrals and deviations from the parent

| Item | Disposition | Reason |
|---|---|---|
| "Settled warnings pruned after 7 days" (parent §B1 D2) | **Not done**: warnings retained | Warning rows are user-visible history and load-bearing attribution markers (D4 evidence). Discovery is already pending-only through the v8 projections. |
| "Work-job discovery uses `work_jobs_ready`" (parent closes-list) | **Replaced** by `work_jobs_live` | Keeps the ascending-ordinal cursor (D1 considered). |
| "Snapshot generations = current + at most one staged" as a partial index | **Expressed as** pointer joins + bounded keep set | Not a static predicate (D3). |
| Retention kick on `snapshot_generations` (expected in the .9 spec) | **Empty kick set** | Avoids a retention commit per 5 s idle publish (D4). |
| `preparation_cleanup` job pruning | Deferred | Its completion row is the operation-key reuse marker. Pruning needs a separate marker or a reuse window. |
| `observed_targets`, `send_preparations` (discarded), `wake_work` of retired seats | Not pruned | Each is bounded by distinct panes, operations or seats, not by time. No finding names them. |
| User-configurable retention | Deferred (parent) | Fixed defaults for v0.1.0. |

## Post-Implementation Notes

> *As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
