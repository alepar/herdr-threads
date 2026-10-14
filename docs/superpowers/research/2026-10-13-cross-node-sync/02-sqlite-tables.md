# 02: SQLite table shapes for cross-node sync

Research question 2 of the vision spec (`docs/superpowers/specs/2026-10-13-cross-node-sync-vision-design.md`). Deep mode, 2026-10-13. It covers the journal, the version vector, unapplied entries, peers, LWW registers, materialization, dual identity, indexing/WAL and storage growth. Source IDs `[Sxx]` are external, `[Lx]` are repo files, `[Bx]` are local benchmarks and `[Xx]` are sibling reports in this directory.

## Recommendations for herdr-threads

1. **The journal is a rowid table: `ingest_seq INTEGER PRIMARY KEY AUTOINCREMENT` is local arrival order, and `UNIQUE(log_ix, counter)` is the global identity.** Payloads are about 1 KB. SQLite's guidance limits WITHOUT ROWID to rows under about 1/20 of a page [S07]. The two layouts measured within about 10% of each other in size and speed [B1]. The rowid gives arrival order for free. eventsourcing's SQLite recorder uses the same pair: PK `(originator_id, originator_version)` for identity and rowid for the local notification order [S15].
2. **Key logs as `(node_id, epoch)`, interned to a small integer `log_ix`.** cr-sqlite and Corrosion do the same with site ordinals [S04]. With the epoch in the key, a restored node that starts a new epoch begins a new log, so its counters cannot collide with the old ones (consistent with [X2] R-F).
3. **Accept entries contiguously only (`counter = contiguous + 1`).** Then the version vector is one column per log, and no gap, sequence or buffer bookkeeping is needed. Corrosion needs three tables for that bookkeeping (`__corro_bookkeeping_gaps`, `__corro_seq_bookkeeping`, `__corro_buffered_changes`) only because it accepts gossip out of order [S04][S21].
4. **Separate the stored frontier from the apply cursor.** The vector advertises what is *stored*, so relay never waits on apply. Each log has its own `applied_through` cursor, and apply runs head-of-line in counter order within each log, which keeps per-origin FIFO in the view.
5. **Materialize with an apply loop in the daemon's writer, not with triggers.** Each batch runs as one `BEGIN IMMEDIATE` transaction, with a SAVEPOINT per entry so a failing handler marks only that entry `failed` and the origin's stream is not wedged. The roughly 359 existing triggers keep maintaining derived projections from the base tables, which the loop writes.
6. **Put local and remote facts through the same `apply(entry)` handler.** The authoring path decides (A2) and builds the entry, then applies it in the same transaction. Until every write path is converted, add an equivalence test: replaying the journal into an empty store must reproduce the replicated tables.
7. **Payloads are exact JSON bytes stored in a `BLOB`, never re-encoded.** The digest is the first 16 bytes of SHA-256 over the canonical envelope plus payload; `sha2` and `serde_json` are already dependencies. Do not use SQLite JSONB as the stored or wire form: SQLite says it is "not intended as an external format" [S17].
8. **All local positions are assigned at apply time and never replicated.** That covers `ingest_seq`, `decision_seq`, `threads.next_sequence`→`messages.sequence`, `*_revision`, AUTOINCREMENT `ordinal`s, `binding_cursors.seq` and `membership_intervals.*_seq`. Payloads carry only IDs and origin facts such as HLC, send time and deadline.
9. **LWW registers get their own table with no FK to `threads`,** so a topic set that arrives before the thread's create still converges. The update is a row-value upsert `WHERE (excluded.hlc, excluded.node) > (hlc, node)`; I verified it on SQLite 3.51. The winning value is then copied into `threads`.
10. **Transactions:** at most 256 entries and 256 KiB per transaction, the same as the wire batch in [X2]. Commit, then grant credit, then yield the writer. Keep `synchronous=FULL`. Add a `journal_size_limit` and a passive checkpoint after catch-up bursts, as Corrosion does with WAL truncation [S22][S23].

## Risks to settled decisions (flagged separately)

- **R1. Per-origin FIFO does not order references across origins.** B's message in A's thread, C's ACK of A's message and a join of a remote seat can reach a third node before their referents, because transitive relay pulls each origin independently. Today's FKs (`memberships.seat_id REFERENCES seats(id)`, `messages.thread_id REFERENCES threads(id)` [L1]) would reject them. *Mitigation:* add two envelope fields that are readable without understanding the kind: `creates` (the entity ID the entry creates, if any) and `deps` (the entity IDs it needs). Apply then blocks that origin head-of-line until the dependencies are present. A dependency whose creator is an unknown kind parks the dependent entry instead of blocking, so a newer kind cannot wedge an older node. The dependencies are causal (the author had seen them), so this cannot deadlock. The fields extend the spec's envelope `(origin, counter, hlc, kind, kind_version, payload)`.
- **R2. Remote ingest will invalidate local sends.** `publish_send` rejects with `Conflict "send preparation snapshot changed"` when `threads.timeline_revision` (among other values) moved since preparation [L2, messages.rs:967-975], and every published message bumps it [L2:882]. With remote traffic in the same thread, local sends will conflict more often; near 10 msg/s in one thread they could livelock. *Verify* what the timeline fence protects. Replicated ordinary messages probably should not bump it, or the fence should narrow to the warnings and sequence base it guards.
- **R3. Growth.** "Nothing replicated is deleted" plus full mesh plus 10 msg/s *sustained* comes to about 0.9 TB per node per year (§6). The observed rate is three orders of magnitude lower, so v1 is fine, but the spec should name the sustained rate it targets and when partial replication becomes necessary.
- **R4. Remote seats need `seats` CHECK changes.** `state` and `role` CHECKs [L1] cannot hold a remote seat with no pane, and SQLite cannot alter a CHECK in place. That means a full table rebuild of `seats`, which many FKs reference, or a separate `seat_directory` table that FKs are repointed to. Plan it in the migration (Q8).
- **R5. Membership intervals diverge per node.** `joined_seq`/`left_seq` are local sequence positions, so "was X a member when message M arrived" can differ between nodes. The audience of a message, meaning its receipt obligations, must therefore be decided by the author and carried in the payload, as [X1] already proposes. Never recompute it from local intervals.

## 1. Grounding: today's single-writer counters

- `next_decision_seq` increments `host_instances.decision_seq` inside the writer transaction, and a bounded batch shares one value [L3]. `insert_publication` reads `threads.next_sequence` as `base` and inserts the message with `sequence=base`. It also inserts K warning events at `base+1..base+K` and `event_offset` 1..K, sets `next_sequence=base+K+1` and bumps `timeline_revision` [L2].
- `messages_instance_logical UNIQUE(instance_id, decision_seq, event_offset)` and `UNIQUE(thread_id, sequence)` are the local identity [L1]. `binding_cursors(binding_id, thread_id, seq)` is a reading position in `sequence` [L5].
- The writer uses WAL with `synchronous=FULL` and a 2 s busy timeout, and every decision runs under `BEGIN IMMEDIATE` [L4].
- One send can produce several timeline rows. The journal therefore needs the pair (entry, `event_offset`) to identify a row, and the existing `event_offset` column already provides it.

## 2. Proposed DDL (node-local sync bookkeeping plus the journal)

```sql
-- One row per (node, epoch) log. A new epoch is a new log (restore-from-backup safety).
CREATE TABLE sync_logs (
    log_ix INTEGER PRIMARY KEY,
    node_id TEXT NOT NULL CHECK(length(node_id) BETWEEN 2 AND 4),
    epoch INTEGER NOT NULL CHECK(epoch >= 0),
    is_self INTEGER NOT NULL DEFAULT 0 CHECK(is_self IN (0,1)),
    contiguous_counter INTEGER NOT NULL DEFAULT 0 CHECK(contiguous_counter >= 0), -- version vector (stored)
    applied_through INTEGER NOT NULL DEFAULT 0 CHECK(applied_through <= contiguous_counter), -- apply cursor
    blocked_on TEXT,              -- entity ID awaited by the head entry (R1); NULL = not blocked
    hlc_last INTEGER,             -- self row only: last issued HLC ([X3] rec. 5)
    first_seen_at INTEGER NOT NULL,
    UNIQUE(node_id, epoch)
) STRICT;
CREATE UNIQUE INDEX sync_logs_self ON sync_logs(is_self) WHERE is_self = 1;

CREATE TABLE journal (
    ingest_seq INTEGER PRIMARY KEY AUTOINCREMENT,   -- local arrival order; never reused
    log_ix INTEGER NOT NULL REFERENCES sync_logs(log_ix),
    counter INTEGER NOT NULL CHECK(counter > 0),
    hlc INTEGER NOT NULL CHECK(hlc >= 0),           -- 48-bit ms | 16-bit logical ([X3])
    kind TEXT NOT NULL CHECK(length(kind) BETWEEN 1 AND 64),
    kind_version INTEGER NOT NULL CHECK(kind_version > 0),
    creates TEXT,                                   -- envelope: entity created (R1)
    deps TEXT,                                      -- envelope: JSON array of entity IDs (R1)
    payload BLOB NOT NULL CHECK(length(payload) <= 524288),  -- exact authored JSON bytes
    digest BLOB NOT NULL CHECK(length(digest) = 16),         -- SHA-256/128 of canonical envelope+payload
    ingested_at INTEGER NOT NULL,
    apply_state INTEGER NOT NULL CHECK(apply_state IN (0,1,2,3,4)),
        -- 0 applied, 1 pending, 2 parked_unknown (kind or major kind_version), 3 parked_dep, 4 failed
    applied_seq INTEGER UNIQUE,      -- decision_seq of the applying decision (reproducible local order)
    applied_by INTEGER,              -- apply-logic version, for targeted re-projection after upgrade
    apply_error TEXT CHECK(apply_error IS NULL OR length(apply_error) <= 512),
    UNIQUE(log_ix, counter)
) STRICT;
CREATE UNIQUE INDEX journal_creates ON journal(creates) WHERE creates IS NOT NULL;
CREATE INDEX journal_unapplied ON journal(apply_state, ingest_seq) WHERE apply_state <> 0;
CREATE TRIGGER journal_immutable BEFORE UPDATE OF log_ix,counter,hlc,kind,kind_version,creates,deps,payload,digest ON journal
BEGIN SELECT RAISE(ABORT, 'journal entries are immutable'); END;
CREATE TRIGGER journal_append_only BEFORE DELETE ON journal
BEGIN SELECT RAISE(ABORT, 'journal entries are never deleted'); END;

-- Dial targets (peer add / observed attach). Forget = dial 0; nothing is deleted.
CREATE TABLE sync_dial_targets (
    ssh_dest TEXT PRIMARY KEY,
    node_id TEXT,                    -- learned at handshake
    source TEXT NOT NULL CHECK(source IN ('explicit','observed_attach')),
    dial INTEGER NOT NULL DEFAULT 1 CHECK(dial IN (0,1)),
    remote_command TEXT,             -- [X2] R-B
    added_at INTEGER NOT NULL,
    forgotten_at INTEGER,
    backoff_step INTEGER NOT NULL DEFAULT 0,
    next_dial_at INTEGER,
    last_connected_at INTEGER,
    last_error TEXT CHECK(last_error IS NULL OR length(last_error) <= 512)
) STRICT;

-- Known nodes (dialed or inbound) and the last vector each advertised (hint, not authority).
CREATE TABLE sync_peers (
    node_id TEXT PRIMARY KEY,
    instance_uuid TEXT,
    host_key_fp TEXT,
    caps_json TEXT,
    last_seen_at INTEGER,
    forgotten INTEGER NOT NULL DEFAULT 0 CHECK(forgotten IN (0,1))
) STRICT;
CREATE TABLE sync_peer_vectors (
    node_id TEXT NOT NULL REFERENCES sync_peers(node_id),
    log_ix INTEGER NOT NULL REFERENCES sync_logs(log_ix),
    peer_has INTEGER NOT NULL CHECK(peer_has >= 0),
    updated_at INTEGER NOT NULL,
    PRIMARY KEY(node_id, log_ix)
) STRICT, WITHOUT ROWID;

-- Multi-writer LWW registers. No FK: may precede the thread's create.
CREATE TABLE thread_registers (
    thread_id TEXT NOT NULL,
    field TEXT NOT NULL CHECK(field IN ('topic','name','archived')),
    value ANY,
    hlc INTEGER NOT NULL,
    node_id TEXT NOT NULL,
    src_ingest_seq INTEGER NOT NULL REFERENCES journal(ingest_seq),
    PRIMARY KEY(thread_id, field)
) STRICT, WITHOUT ROWID;

-- Provenance on materialized replicated rows (repeat for invitations, receipt_state,
-- memberships, seats/seat_directory as each kind is converted).
ALTER TABLE messages ADD COLUMN src_ingest_seq INTEGER REFERENCES journal(ingest_seq);
CREATE UNIQUE INDEX messages_src ON messages(src_ingest_seq, event_offset) WHERE src_ingest_seq IS NOT NULL;
```

Notes:

- **Field layout.** `kind` is TEXT, about 15 B per row, which is cheap at this scale and readable in `sqlite3`. `creates` and `deps` are envelope columns, not payload fields, so an older node can index them for kinds it does not understand. A row-per-field register is extensible: a new LWW field needs no migration, which fits the versionless rule. The register upsert is `INSERT … ON CONFLICT(thread_id, field) DO UPDATE SET … WHERE (excluded.hlc, excluded.node_id) > (thread_registers.hlc, thread_registers.node_id)`. A thread-create entry writes its initial topic and name through the same upsert, stamped with its own HLC.
- **Version vector.** The vector is `SELECT node_id, epoch, contiguous_counter FROM sync_logs`. At startup, assert `contiguous_counter = (SELECT MAX(counter) FROM journal WHERE log_ix=?)`; it is one index probe per log.
- **Local append and duplicates.** A local append takes `contiguous_counter + 1` of the self log in the decision transaction, the same way `next_decision_seq` works. For an inbound entry with `counter ≤ contiguous`, compare digests: a match is a duplicate and is dropped, a mismatch is a fork (Q9). An entry with `counter > contiguous + 1` is dropped, and the link re-requests from the cursor ([X2] rec. 5).
- **Per-peer state.** Only the advertised vectors are persisted, for `peer status` and to skip unchanged origins on reconnect ([X2] rec. 4). In-flight credit and subscriptions stay in memory.

## 3. Materialization: apply loop versus triggers

| | Triggers on `journal` insert | Apply loop in the writer (recommended) |
|---|---|---|
| Payload decoding | `json_extract` in SQL; kind and version dispatch in `CASE` | Rust serde types; `kind_version` dispatch in code |
| Unknown kind / newer major version | Hard to express; risks aborting the insert, which would stall relay | `apply_state=2`; the entry is stored and relayed but not applied |
| Delivery policy (wake, nudge, lazy), A2 checks | Cannot run Rust adapters | Runs in the same transaction, as today |
| Poison entry | Aborts the whole batch transaction | SAVEPOINT per entry; on error, `ROLLBACK TO` and mark `failed` |
| Replay after upgrade | Needs DDL changes per version | A startup pass over `journal_unapplied` |

Triggers stay where they are now: maintaining derived projections from base tables, plus the journal's immutability guards. The apply loop writes the base tables, so the existing triggers keep working.

**Idempotence.** An entry is applied exactly once: its effects and `apply_state=0` commit in the same transaction, and `UNIQUE(log_ix, counter)` rejects re-ingest. This is the same "tracking row in the projection transaction" pattern eventsourcing uses (`UPDATE tracking SET notification_id … WHERE tracking.notification_id < :notification_id` [S15]). Keep the natural-key UNIQUE constraints (message `id`, `messages_src`) as loud guards. Do not hide double-apply with `ON CONFLICT DO NOTHING`; the LWW upsert is the only exception.

**Apply order.** Take logs in `ingest_seq` order of their next entry. Within a log, start at `applied_through+1`:

- A known entry with all dependencies present: apply it and advance.
- An unknown kind: park it (2) and advance.
- A dependency created by a parked entry: park it (3) and advance.
- A dependency that is absent: set `blocked_on` and stop this log.

When an entry with `creates=X` applies, clear `blocked_on=X` on any log waiting for it.

**Replay after upgrade.** Three cases:

- (a) Entries parked as unknown whose kind is now known: apply them in `ingest_seq` order. They get new local sequences, so they appear at the bottom, which matches the spec's late-arrival rule.
- (b) Apply logic now extracts more fields from a known kind: run a migration that re-reads payloads `WHERE kind=? AND applied_by < N` and patches columns, without re-applying.
- (c) Full rebuild, for tests and repair: replay into an empty store in `applied_seq` order, which reproduces local sequences exactly.

Synapse is the precedent for keeping arrival order separate from replayable history. Its `stream_ordering` is arrival order, and backfilled history gets negative, decrementing values [S14].

## 4. Dual identity: how each counter maps

| Counter | Scope after sync | Replicated? |
|---|---|---|
| `(node_id, epoch, counter)` | global identity of a fact | yes (envelope) |
| entity IDs (`m_…`, `t_…`) | global, with the origin embedded | yes (payload) |
| `hlc` | origin time for LWW and "sent at" | yes |
| `journal.ingest_seq` | local arrival | no |
| `decision_seq` + `event_offset` | local decision clock. A replicated entry gets a fresh one at apply, stored as `journal.applied_seq`. Wake cursors (`last_*_seq`) and `offered_through_seq` keep their meaning | no |
| `messages.sequence` / `next_sequence` | local per-thread ingest order (spec) | no |
| `*_revision` | local cache and fence tokens; bumped on apply (see R2) | no |
| AUTOINCREMENT `ordinal`s | local tie-break and scan order | no |
| `binding_cursors.seq`, `membership_intervals.*_seq` | local positions (R5) | no |
| `decision_at` | for replicated rows, the origin's send time (spec: show the original send time). The origin computes `deadline_at` and carries it in the payload, so the ACK clock starts on the sender's node | via payload |

`instance_id` columns keep meaning "this store". A replicated row gets the local instance, and its origin is recorded in the journal through `src_ingest_seq`. Do not create `host_instances` rows for remote nodes.

## 5. Indexing, WAL and transaction sizing

- **Access paths.** A range pull uses `WHERE log_ix=? AND counter>? ORDER BY counter LIMIT 256` through the UNIQUE index, then a rowid fetch. The apply queue uses the partial index `journal_unapplied`, which stays tiny. Dependency resolution uses `journal_creates`. Nothing reads the journal by HLC or by kind on a hot path.
- **Benchmarks** (Python `sqlite3`, SQLite 3.53.4, macOS, WAL, `synchronous=FULL` [B2]). The test applied each entry as journal insert + `decision_seq` + `next_sequence` + `messages` insert + state flip:

  | Batch size | Entries/s | Transaction p50 | Transaction p99 |
  |---|---|---|---|
  | 1 | 4.1k | — | — |
  | 64 | 10.3k | 1.9 ms | 36 ms |
  | 256 | 21.5k | 6.8 ms | 97 ms |
  | 2048 | — | 66 ms | — |

  Caveat: Python's `synchronous=FULL` on macOS does not issue `F_FULLFSYNC` (the 0.1 ms p50 at batch size 1 shows it), so absolute numbers are optimistic. The ratios still hold. 256 entries per transaction keeps the writer hold well under the 2 s busy timeout [L4], and local commands interleave between batches.
- **Catch-up throughput.** First contact with about 10^5 entries is about 400 transactions, a few seconds of writer time. Run it as a background lane that yields between batches.
- **WAL.** "WAL works best with smaller transactions", and a long-running read transaction can stop the checkpointer [S06]. Corrosion saw the WAL "grow to multiple gigabytes" under write pressure. It runs a periodic `wal_checkpoint(TRUNCATE)` and sets `journal_size_limit=1 GiB` [S22][S23]. For herdr-threads, set `journal_size_limit` to something like 64 MiB, run `wal_checkpoint(PASSIVE)` after a catch-up burst, and keep query connections short-lived so they do not pin the WAL.
- **Durability.** Keep `synchronous=FULL` on the writer, as today. Corrosion uses `NORMAL` [S23], but here a lost local commit means a counter that peers may already hold with different content. That is exactly the fork case (Q9).

## 6. Storage growth

Measured: a journal row with a JSON payload of about 530 B takes 682–690 B including indexes [B1]. A 4 KB body takes about 4.6 KB. The live store holds 2,730 timeline rows over about 13 days, ordinary bodies average 798 B, and the `messages` table is 1.7 MB per `dbstat` [X1].

Per ordinary message the estimate is about 3 KB in total: a journal entry of about 1.4 KB, a materialized row of about 0.9 KB (the body is stored twice), plus receipt rows and ACK entries.

| Scenario | Rate | Per node per year |
|---|---|---|
| Today ×10 nodes (≈2.1k rows/day) | 0.024/s | ≈2–3 GB |
| 1 msg/s sustained | 86k/day | ≈90 GB |
| 10 msg/s sustained (spec ceiling) | 864k/day | ≈0.9 TB |

SQLite's own limits (about 281 TB [S12]) are irrelevant here; disk space and first-contact transfer time are what bind. For v1, accept storing the body twice. If growth matters later, materialized `messages.body` can be NULL for rows whose body lives in `journal.payload`, read through `src_ingest_seq`. Info events are 73% of timeline rows [X1], so whether they are journaled at all is the larger lever.

## 7. Prior art

- **cr-sqlite (`crsql_changes`).** It is history-free: the clock table's PK `(key, col_name)` *replaces* superseded entries, so a `db_version` can lose seqs [S04]. `crsql_changes` stores current state, "rather than the full history" [S03]. It therefore cannot be a relay log that keeps every entry and every unknown kind, and it would also add a native extension. The ideas worth borrowing are interned site ordinals and a per-site version table (`crsql_db_versions(site_id, db_version)`) [S04].
- **Corrosion.** Gossip arrives out of order, so it keeps gap ranges `(actor_id, start, end)`, per-version seq ranges, and buffered partial transactions [S21]. Contiguous-only ingest over point-to-point links avoids all three. Its operational lessons (WAL truncation, `journal_size_limit`) do carry over [S22][S23].
- **SQLite session extension.** It records physical row diffs keyed on the PRIMARY KEY [S05]. When a target table is incompatible, the changes for that table are skipped with only a log warning [S11]. Both points disqualify it here: the PKs are local AUTOINCREMENT `ordinal`s that would collide across nodes, and skipping a table silently contradicts "unknown kinds stay unapplied until replay". It is also not compiled into the `bundled` rusqlite feature the repo uses.
- **Event sourcing on SQLite.** The pattern is an aggregate-version PK plus a rowid notification order plus a tracking row in the projection transaction [S15]. That is the recommended shape, with `(log, counter)` in the aggregate role.
- **Matrix Synapse.** It keeps arrival order (`stream_ordering`) separate from DAG order and gives backfill its own ordering [S14]. This supports the spec's choice of local-arrival `sequence`.

## Methodology and limitations

- **Tools.** `search` 0.9.0 ran 12 queries and considered 120 result entries; its serper and perplexity providers failed (credits/quota) and the others answered. Fetching was direct HTTP. 20 original documents were retrieved, plus a shallow clone of `superfly/corrosion` at `9408f65`. The repo was read directly.
- **Benchmarks.** Synthetic, Python `sqlite3` 3.53.4 [B1][B2].
- **Retrieval failures.** GitHub code search returned 401, so Corrosion was cloned instead. Reading the live store was denied by the permission classifier, so live-store figures come from sibling report 08 [X1].
- **Limitations.** The storage estimates are extrapolations. R2 needs code-level confirmation of what `timeline_revision` guards. The scratch ledgers (queries, sources, evidence, claims, coverage) are in the session scratchpad.
- **Stop rule.** Every facet in scope is covered; the remaining open items are design choices, not missing evidence.

## Bibliography

- [S03] vlcn.io, "crsql_changes". https://vlcn.io/docs/cr-sqlite/api-methods/crsql_changes
- [S04] superfly/cr-sqlite README (Fly.io fork, v0.17). https://github.com/superfly/cr-sqlite
- [S05] SQLite, "The Session Extension". https://sqlite.org/sessionintro.html
- [S06] SQLite, "Write-Ahead Logging". https://www.sqlite.org/wal.html
- [S07] SQLite, "Clustered Indexes and the WITHOUT ROWID Optimization". https://www.sqlite.org/withoutrowid.html
- [S11] SQLite, "sqlite3changeset_apply". https://sqlite.org/session/sqlite3changeset_apply.html
- [S12] SQLite, "Implementation Limits". https://www.sqlite.org/limits.html
- [S14] Synapse, "Room DAG concepts". https://github.com/matrix-org/synapse/blob/develop/docs/development/room-dag-concepts.md
- [S15] eventsourcing 9.4.6, `eventsourcing.sqlite`. https://eventsourcing.readthedocs.io/en/v9.4.6/_modules/eventsourcing/sqlite.html
- [S17] SQLite, "The SQLite JSONB Format". https://sqlite.org/jsonb.html
- [S21] Corrosion `crates/corro-types/src/agent.rs` @9408f65 (`init_migration`). https://github.com/superfly/corrosion
- [S22] Corrosion `crates/corro-agent/src/agent/handlers.rs` @9408f65 (`wal_checkpoint`).
- [S23] Corrosion `crates/corro-types/src/sqlite.rs` @9408f65 (pragmas).
- [L1] `migrations/0001_initial.sql`; [L2] `src/store/messages.rs:786-905, 954-975`; [L3] `src/store/schema.rs:3316-3338, 3422-3434`; [L4] `src/store/connection.rs:238-264, 398-470`; [L5] `migrations/0032_session_bindings.sql:81-91`.
- [B1] Local benchmark: journal bytes per row, rowid vs WITHOUT ROWID. [B2] Local benchmark: apply-loop batch latency.
- [X1] `08-migration-backfill.md`; [X2] `03-wire-protocol.md`; [X3] `04-hlc-lww.md` (this directory).
