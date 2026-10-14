# Cross-node sync: journal core — design

Parent: [root spec](2026-10-14-cross-node-sync-design.md) · Bead: ht-5fr.3

## Goal

Every replicated mutation a node makes is a signed, hash-chained entry in its own journal, and it is materialized through one `apply(entry)` path. That path is the same for entries authored locally and entries ingested from other nodes. The journal also takes verified batches of other nodes' entries, serves committed ranges and reports a version vector. Its observable outcomes: an entry authored on store A, served, and ingested into store B is verified, stored, and then applied on B exactly once, in causal order, with a recorded outcome (`applied`, `parked_unknown`, `parked_dep` or `failed`). Replaying a journal into an empty store reproduces the replicated tables.

## Problem description

The store today has one writer and no notion of a replicated fact. Every command decides and writes base tables directly inside `execute_budgeted_decision_with_constraints` (`src/store/connection.rs`), wrapped by `execute_budgeted_idempotent_transaction_with_constraints` and `execute_accountable_transaction` (`src/store/schema.rs`). Every ordering value (`next_decision_seq`, `threads.next_sequence`, revisions, ordinals) is a local counter. The root spec (§4, §5) makes a per-origin journal the source of truth for replicated facts. It fixes the envelope fields, the hash and signature rule, the HLC and the ingest/apply split. It leaves the concrete DDL, the byte layout, the deps encoding, the Rust surfaces, the transaction boundaries, the outcome bookkeeping, the lane wiring and the error taxonomy to this subepic. Peer links (ht-5fr.5), fact kinds for threads, seats and messages (ht-5fr.4) and fork repair (ht-5fr.6) all build on what this subepic ships. So this subepic has to be complete enough that each of them adds only its own module.

## Main challenges

Several requirements pull against each other. Canonical bytes must be stable across serde versions, Rust versions and node versions, so hashing JSON is out. The payload must still relay byte for byte through nodes that do not understand its kind. Ingest must never read payloads, yet the genesis entry has to supply the public key that verifies everything after it. Apply must be causal (a deps vector) and must never stall relay. Two cases need care here:
- an entity whose creator was parked;
- a log that is later frozen at a counter below what an author had applied.

Local authoring must go through the same handler as remote apply, in the command's own transaction. A handler that rejects a local fact must abort the command and must not be recorded as `failed`, while the same rejection of a remote fact must be recorded and counted as processed. Handlers sometimes need to author a follow-up fact (a retag after a tag collision), and that follow-up's deps must include the entry that triggered it. Finally, the new lane must not kick itself forever, and must not cost a writer turn on every local command, because every local command now writes `journal`. The 8-bit lane bitset also has no room left once `Apply` and `Sync` are added.

## Key decisions made

- **Canonical bytes.** They are a fixed big-endian, length-prefixed binary layout with an 8-byte magic. `deps` is a sorted binary vector of `(node_uuid, incarnation, counter)` triples, and the same bytes are stored in `journal.deps`. The wire form is JSON: binary fields are lowercase hex and the payload is a `serde_json::value::RawValue`, so its exact bytes survive relay.
- **Log identity.** Logs are keyed by `(node_uuid, incarnation)` and interned to a local `log_ix`. The log a node authors into is named by a new `local_node.active_log_ix` column, not by a unique "self" flag, so ingesting the node's own older incarnations is just ordinary ingest.
- **Ingest.** Ingest keeps the valid prefix of a batch on `Gap`, `Fork` and `Frozen`, and stores nothing on `Corrupt` or `Headroom`.
- **Readiness.** An entry is ready when its log's `applied_through = counter-1` and every deps triple is covered by `applied_through`, capped at `frozen_at_counter` for a frozen log. Every entity in `needs` is then looked up through the indexed, kind-agnostic `journal.creates` column. A missing or unapplied creator parks the entry as `parked_dep` with `parked_on`. When a creator applies, every apply pass re-attempts the entries parked on it, so no in-memory state is needed.
- **Error classification.** Handlers return `FactError::{Rejected(ApiError), Store(ApiError)}`. An `ApiError` is classified as a store error only for the busy, full, corrupt, cancelled and deadline codes.
- **Follow-up facts.** A follow-up is authored inside the triggering entry's savepoint, after that entry's `applied_through` has advanced.
- **Positions and provenance.** ApplyEngine allocates one `decision_seq` per applied entry and stores it as `applied_seq`. `ApplyContext` carries provenance, suppression, the origin time and the follow-up queue.
- **Lanes.** `Lane::Apply` is kicked by `journal` commits and is never kicked by its own commits. `LaneSet` widens to `u16`.
- **Shared chain rules.** The chain rules are one pure function shared by `SqliteStore` and the in-memory `MemoryJournal`, and one contract test runs against both.
- **Node kinds.** This subepic owns the `node.genesis` and `node.retag` kinds completely, including the collision check and the authoring of retags.

## Decision points

### 1. Module layout and ownership

**Recommended.** New core modules (all under `scripts/check-harness-neutral`'s scan, naming no harness):

| File | Contents |
|---|---|
| `src/sync/mod.rs` | `LogKey { node: NodeUuid, incarnation: u32 }`, `LogHead { counter: u64, head_hash: [u8;32], state: LogState, frozen_at: Option<u64> }`, `LogState::{Active, Frozen}`, `Vector(BTreeMap<LogKey, LogHead>)`, `BatchLimits { entries: 256, bytes: 256 KiB }`, `EntityKey` |
| `src/sync/envelope.rs` | `Envelope`, `UnsignedEnvelope`, `Deps`, `WireEnvelope`, `EnvelopeError`, canonical bytes, hash, verify, genesis check |
| `src/sync/hlc.rs` | `HlcStamp`, `Hlc`, `Fold` |
| `src/sync/chain.rs` | pure `check_next(held: Option<&HeldLog>, entry: &Envelope) -> ChainStep`, shared by both `JournalPort` implementations |
| `src/sync/journal_port.rs` | `JournalPort` trait, `IngestOutcome`, `ForkEvidence`, `ApplyProgress` |
| `src/sync/memory_journal.rs` | `MemoryJournal` (`#[cfg(any(test, feature = "test-support"))]`), the second `JournalPort` implementation |
| `src/sync/facts/mod.rs` | `FactKind`, `FactError`, `FactRegistry`, `registry()`, `ApplyContext` re-export |
| `src/sync/facts/node_genesis.rs`, `src/sync/facts/node_retag.rs` | the two v1 node kinds |
| `src/store/journal.rs` | `open`, `author`, `author_above`, `ingest`, `serve`, `vector`, `applied_vector`, `stored_hash`, `store_forked`, `begin_incarnation`, startup invariants; `impl JournalPort for SqliteStore` |
| `src/store/apply.rs` | `ApplyEngine` functions, `ApplyContext`, readiness, savepoints, outcomes, replay |
| `src/store/thread_registers.rs` | `current`, `upsert` (LWW) |
| `migrations/node/0001_node_baseline.sql` | the DDL of §4, edited in place (ht-5fr.1's "editable until released" rule) |

`src/lib.rs` exports `herdr_threads::sync`. `Cargo.toml` enables `serde_json`'s `raw_value` feature, at the same pinned version. Verification uses ht-5fr.1's `ed25519-dalek` (`VerifyingKey::verify_strict`). There are no other new dependencies.

**Considered.**
- Putting `Journal` in `src/sync/`: that would split one table owner across two trees, and the root names `src/store/journal.rs`.
- A separate `ApplyPort` trait for the lane worker: it would have one implementation. The worker reaches apply through `JournalPort::apply_pending` instead (§9).

### 2. Envelope: canonical bytes, deps, wire form

**Recommended.** `Envelope { log: LogKey, counter: u64, prev_hash: [u8;32], hlc: HlcStamp, kind: String, kind_version: u32, deps: Deps, creates: Option<EntityKey>, needs: Vec<EntityKey>, payload: Vec<u8>, hash: [u8;32], signature: [u8;64] }`.

Canonical bytes (all integers big-endian; `hash` and `signature` excluded):

| # | Field | Encoding |
|---|---|---|
| 1 | magic | 8 bytes `HTJRNL01` |
| 2 | node_uuid | 16 bytes |
| 3 | incarnation | u32 |
| 4 | counter | u64 |
| 5 | prev_hash | 32 bytes (32 zero bytes at counter 1) |
| 6 | hlc | u64 |
| 7 | kind | u16 length + UTF-8 |
| 8 | kind_version | u32 |
| 9 | deps | u32 length + deps bytes |
| 10 | creates | u8 0 or 1, then (if 1) u16 length + bytes |
| 11 | needs | u16 count, then per key u16 length + bytes |
| 12 | payload | u32 length + bytes |

- `hash = SHA-256(canonical_bytes)`, `signature = Ed25519(node key, hash)` via `NodeIdentity::sign`. The magic separates this signature domain from the handshake signatures of ht-5fr.5.
- **Deps bytes:** u16 count, then count × (node_uuid 16 ‖ incarnation u32 ‖ counter u64), strictly ascending by `(node_uuid bytes, incarnation)`. Every counter is ≥ 1. The entry's own log is never listed, because `counter-1` already covers it; the origin's other incarnations are listed. The author's applied vector is taken at write time, with logs at `applied_through = 0` omitted. `Deps::decode` rejects unsorted input, duplicates, a zero counter, trailing bytes and the own log (`Malformed{field: "deps"}`). The canonical form is therefore unique, and `journal.deps` stores exactly these bytes. Size: 28 bytes per log, so about 300 bytes at 10 nodes.
- **EntityKey grammar:** 1..160 bytes over `[0-9A-Za-z._:/-]`. Plain public IDs (`t…`, `m…`, `s…`, `i…`, `d…`) are keys. Composite keys joined by `/` name settlement records, so that catch-up suppression stays kind-agnostic (§8). Examples: `m<id>/s<id>` for a receipt settlement and `i<id>/answer` for an invitation answer.
- **Shape limits** (`Envelope::validate_shape`, envelope v1 contract): `kind` 1..64 bytes matching `^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+$`; `kind_version` 1..=65535; at most 256 `needs`, sorted and unique; at most 4096 deps; payload ≤ 524288 bytes and valid JSON; `counter` ≤ `i64::MAX`. Counter 1 must be `node.genesis` v1, and `node.genesis` may appear only at counter 1.
- **Methods:**
  - `UnsignedEnvelope::seal(self, &NodeIdentity) -> Envelope`
  - `Envelope::compute_hash()`
  - `Envelope::verify(&self, pubkey: &[u8;32]) -> Result<(), EnvelopeError>`: recompute the hash, then `HashMismatch`; verify the signature, then `BadSignature`
  - `Envelope::genesis_pubkey(&self) -> Result<[u8;32], EnvelopeError>`: parse only `pubkey` from the genesis payload, then check `SHA-256(pubkey)[0..16] == log.node` (`UuidMismatch`)
  - `EnvelopeError::{Malformed{field}, HashMismatch, BadSignature, UuidMismatch, NotGenesis}`: shape errors stay distinct from cryptographic ones
- **Wire form** `WireEnvelope` (serde, used inside ht-5fr.5's `Entries` frames): `{node, inc, counter, prev, hlc, kind, ver, deps, creates?, needs, payload, hash, sig}`. Binary fields are lowercase hex and `payload` is `Box<RawValue>`. `From<&Envelope>` and `TryFrom<WireEnvelope>` (the latter runs `validate_shape`) never re-encode the payload.

**Considered.**
- JSON canonicalization (root rejected it).
- CBOR (a new dependency).
- Base64 payloads on the wire (no base64 crate in tree, and 33% overhead).
- Listing only the logs that differ from the previous entry's deps: smaller, but every receiver would then need the previous entry to decode, which breaks single-entry verification and replay.
- Making `needs` an envelope field the root had not fixed: the root already lists it.

### 3. HLC

**Recommended.** In `src/sync/hlc.rs`:
- `HlcStamp(u64)`: 48-bit Unix ms ‖ 16-bit logical, with `ms()`, `logical()` and `from_parts(ms, logical)`. Wall time is clamped to `0..2^48`. `Ord` is the u64 order, and the LWW tie-break pair `(HlcStamp, NodeUuid)` is compared as a tuple.
- `Hlc { last: HlcStamp }`:
  - `tick(wall) -> HlcStamp = max(wall << 16, last + 1)`. A u64 increment carries the logical overflow into ms. It saturates at `u64::MAX`, which is unreachable but means no panic.
  - `tick_above(wall, floor) -> HlcStamp = max(tick(wall), floor + 1)`. `last` takes this value only when `floor.ms() ≤ wall + 60 000`, so a far-future register cannot drag the node clock.
  - `fold(wall, remote) -> Fold`: either `Absorbed` (when `remote.ms() ≤ wall + MAX_ABSORB_MS`, which sets `last = max(last, remote)`) or `Skewed { ahead_ms }`, which leaves `last` untouched.
  - `resume(wall, self_max: Option<HlcStamp>)`: `last = max(wall << 16, min(self_max, (wall + 60 000) << 16))`.
- `MAX_ABSORB_MS = 60_000`.

The resume rule is root §4's `max(wall, max HLC in the self log)` with one refinement: values written by `tick_above` above the absorb cap are capped at resume. Otherwise a restart would absorb a poisoned register floor that the running clock deliberately did not absorb. `self_max` is the head entry's `hlc` of every log with `node = self` (one indexed probe per self log). Self-log HLCs are not monotone in counter, because `tick_above` can go higher, and nothing relies on them being monotone: ordering is by counter, and HLC is used only for LWW and "sent at".

`StoreContext` gains `sync: OnceLock<SyncState>`, where `SyncState { identity: Arc<NodeIdentity>, hlc: Mutex<Hlc>, registry: &'static FactRegistry, headroom_bytes: AtomicU64 }`. It is installed once by `journal::open`. Before that, any authoring or ingest call returns `ErrorCode::Unsupported` "journal not open", which is a programming error. Only `src/store/journal.rs` locks `hlc`, and only inside a writer transaction. A rolled-back transaction leaves `last` advanced, which is harmless because the clock only moves forward.

Skew: when ingest folds a `Skewed` entry, Journal sets `sync_logs.skew_ahead_ms` to the largest `ahead_ms` seen in that batch, and to 0 when the batch had none. It also writes one daemon-log line per log per hour (`sync: log <uuid8>/<inc> hlc ahead of wall by <n> ms; not absorbed`). ht-5fr.8's `sync status` reads the column.

**Considered.**
- The `uhlc` crate (root rejected it).
- Persisting `hlc_last` in a column updated on every append: an extra write per command, while the head probe at start is cheap.
- Letting `tick_above` update `last` unconditionally: the poisoned-register case in research 04, R2.

### 4. Tables (baseline DDL)

**Recommended.** Added to `migrations/node/0001_node_baseline.sql`, all tables `STRICT`, in this order:

```sql
-- local_node (ht-5fr.1) gains:
--   active_log_ix INTEGER REFERENCES sync_logs(log_ix)   -- NULL until journal::open bootstraps

CREATE TABLE sync_logs (
  log_ix INTEGER PRIMARY KEY,
  node_uuid TEXT NOT NULL CHECK(length(node_uuid)=32 AND node_uuid NOT GLOB '*[^0-9a-f]*'),
  incarnation INTEGER NOT NULL CHECK(incarnation BETWEEN 1 AND 4294967295),
  is_self INTEGER NOT NULL CHECK(is_self IN (0,1)),
  pubkey BLOB NOT NULL CHECK(length(pubkey)=32),
  genesis_hash BLOB NOT NULL CHECK(length(genesis_hash)=32),
  contiguous_counter INTEGER NOT NULL CHECK(contiguous_counter>=0),
  head_hash BLOB NOT NULL CHECK(length(head_hash)=32),
  applied_through INTEGER NOT NULL DEFAULT 0 CHECK(applied_through BETWEEN 0 AND contiguous_counter),
  blocked_on TEXT CHECK(blocked_on IS NULL OR length(blocked_on)<=200),
  state TEXT NOT NULL DEFAULT 'active' CHECK(state IN ('active','frozen')),
  frozen_at_counter INTEGER CHECK((state='frozen')=(frozen_at_counter IS NOT NULL)),
  skew_ahead_ms INTEGER NOT NULL DEFAULT 0 CHECK(skew_ahead_ms>=0),
  first_seen_at INTEGER NOT NULL,
  UNIQUE(node_uuid, incarnation)
) STRICT;

CREATE TABLE journal (
  ingest_seq INTEGER PRIMARY KEY AUTOINCREMENT,
  log_ix INTEGER NOT NULL REFERENCES sync_logs(log_ix),
  counter INTEGER NOT NULL CHECK(counter>=1),
  prev_hash BLOB NOT NULL CHECK(length(prev_hash)=32),
  hash BLOB NOT NULL CHECK(length(hash)=32),
  hlc INTEGER NOT NULL CHECK(hlc>=0),
  kind TEXT NOT NULL CHECK(length(kind) BETWEEN 1 AND 64),
  kind_version INTEGER NOT NULL CHECK(kind_version BETWEEN 1 AND 65535),
  deps BLOB NOT NULL,
  creates TEXT CHECK(creates IS NULL OR length(creates) BETWEEN 1 AND 160),
  needs TEXT NOT NULL DEFAULT '',          -- '\n'-joined sorted EntityKeys
  payload BLOB NOT NULL CHECK(length(payload)<=524288),
  signature BLOB NOT NULL CHECK(length(signature)=64),
  ingested_at INTEGER NOT NULL,
  apply_state TEXT NOT NULL CHECK(apply_state IN ('applied','pending','parked_unknown','parked_dep','failed')),
  applied_seq INTEGER CHECK((apply_state='applied')=(applied_seq IS NOT NULL)),
  parked_on TEXT CHECK((apply_state='parked_dep')=(parked_on IS NOT NULL)),
  apply_error TEXT CHECK(apply_error IS NULL OR length(CAST(apply_error AS BLOB))<=512),
  UNIQUE(log_ix, counter)
) STRICT;
CREATE UNIQUE INDEX journal_applied_seq ON journal(applied_seq) WHERE applied_seq IS NOT NULL;
CREATE INDEX journal_unapplied ON journal(apply_state, ingest_seq) WHERE apply_state<>'applied';
CREATE INDEX journal_creates ON journal(creates) WHERE creates IS NOT NULL;
CREATE INDEX journal_parked_on ON journal(parked_on) WHERE apply_state='parked_dep';

CREATE TABLE journal_forked (
  forked_ix INTEGER PRIMARY KEY AUTOINCREMENT,
  log_ix INTEGER NOT NULL REFERENCES sync_logs(log_ix),
  counter INTEGER NOT NULL CHECK(counter>=1),
  -- same envelope columns and CHECKs as journal: prev_hash … signature
  source_node TEXT CHECK(source_node IS NULL OR length(source_node)=32),
  received_at INTEGER NOT NULL,
  UNIQUE(log_ix, counter, hash)
) STRICT;

CREATE TABLE thread_registers (
  thread_id TEXT NOT NULL,
  field TEXT NOT NULL CHECK(field IN ('topic','name','archived')),
  value TEXT,
  hlc INTEGER NOT NULL CHECK(hlc>=0),
  node_uuid TEXT NOT NULL CHECK(length(node_uuid)=32 AND node_uuid NOT GLOB '*[^0-9a-f]*'),
  src_ingest_seq INTEGER NOT NULL REFERENCES journal(ingest_seq),
  PRIMARY KEY(thread_id, field)
) STRICT, WITHOUT ROWID;
```

Triggers, each `RAISE(ABORT, '<name>')` so the failing guard is named:
- `journal_envelope_immutable`: BEFORE UPDATE OF every envelope column (`log_ix … signature`, `ingested_at`).
- `journal_append_only`: BEFORE DELETE.
- `journal_applied_terminal`: BEFORE UPDATE OF `apply_state`, `applied_seq` WHEN `OLD.apply_state='applied'`.
- `journal_forked_immutable` and `journal_forked_append_only`: the same two rules on `journal_forked`.
- `sync_logs_identity_immutable`: BEFORE UPDATE OF `node_uuid`, `incarnation`, `is_self`, `pubkey`, `genesis_hash`.
- `sync_logs_append_only`: BEFORE DELETE.
- `sync_logs_monotone`: BEFORE UPDATE WHEN `NEW.contiguous_counter < OLD.contiguous_counter OR NEW.applied_through < OLD.applied_through OR (OLD.state='frozen' AND NEW.state<>'frozen')`.
- `sync_logs_self_flag`: BEFORE INSERT WHEN `NEW.is_self <> (NEW.node_uuid=(SELECT id FROM local_node))`.
- `thread_registers_lww`: BEFORE UPDATE WHEN `(NEW.hlc, NEW.node_uuid) <= (OLD.hlc, OLD.node_uuid)`.
- `thread_registers_append_only`: BEFORE DELETE.

`thread_registers` has no FK to `threads` (root §4). It is written only by `thread_registers::upsert(tx, thread, field, value, hlc, node, ingest_seq) -> RegisterOutcome::{Won, Lost}`: an `INSERT … ON CONFLICT(thread_id, field) DO UPDATE … WHERE (excluded.hlc, excluded.node_uuid) > (hlc, node_uuid)`, the one `ON CONFLICT` the root allows. `thread_registers::current(tx, thread, field) -> Option<(HlcStamp, NodeUuid)>` gives authors the floor for `author_above` (§5). ht-5fr.4's `thread.created`/`thread.register` handlers call these and copy winners into `threads`.

Kicks (`src/service/kicks.rs`): `journal` maps to `Lane::Apply`. `sync_logs`, `journal_forked` and `thread_registers` go into `KNOWN_UNMAPPED`, as do `local_node` and `node_directory` if ht-5fr.1 has not classified them. ht-5fr.5 re-maps `sync_logs`/`journal` for `Lane::Sync`.

**Considered.**
- Research 02's `UNIQUE` index on `creates`: re-issued facts (ht-5fr.6) and `thread.direct` created by both pair members repeat a `creates` key.
- An `INTEGER` apply_state: it is unreadable in `sqlite3`, and the root names text states.
- A unique "active self" index on `sync_logs`: ingesting this node's older incarnations (restore pull-back) also has `is_self=1`, and `local_node.active_log_ix` states the authoring log directly.
- An FK from `thread_registers` to `threads`: root §4 forbids it.

### 5. Journal: open, author, serve, vector

**Recommended.** `src/store/journal.rs`:

- `open(ctx: &StoreContext, conn, identity: Arc<NodeIdentity>, host_label: &str, registry: &'static FactRegistry) -> Result<OpenReport, ApiError>`. It runs once in `run_elected_impl`, after ht-5fr.1's start step 4 (`node.json`) and before host attach. In one `BEGIN IMMEDIATE`:
  1. **Invariants.** For every log, `contiguous_counter = MAX(counter)` and `head_hash` equals the stored hash at that counter (zero hash at 0). For the active self log, `applied_through = contiguous_counter`. A violation is `StoreCorrupt` "journal invariant: <log> <what>" and refuses the start through the existing `startup_failure_text` path.
  2. **Resume** the HLC (§3).
  3. **Bootstrap.** If `local_node.active_log_ix` is NULL, call `begin_incarnation(tx, 1, None)` (amended by [incarnations design §8](2026-10-14-cross-node-sync--incarnations-design.md#8-journal-changes): the incarnation is drawn at random through `next_incarnation`, never 1). That builds and seals the genesis entry (payload `{"pubkey":"<hex64>","tag":"<tag>","tag_k":<k>,"incarnation":1,"predecessor":null,"host_label":"<≤255 bytes>"}`), inserts the `sync_logs` row (`is_self=1`, `pubkey`, `genesis_hash`, `contiguous_counter=1`, `head_hash=genesis hash`), inserts the journal row, sets `active_log_ix`, and applies it inline (§7). `host_label` is `gethostname()` truncated at a char boundary.

  `begin_incarnation(tx, incarnation, predecessor: Option<(u32 /*inc*/, u64 /*frozen_at*/)>)` is public for ht-5fr.6's rotation (amended by [incarnations design §8](2026-10-14-cross-node-sync--incarnations-design.md#8-journal-changes): `predecessor: Option<Predecessor { incarnation: u32, frozen_at: Option<u64> }>`, and the genesis payload's `predecessor` is `null | {incarnation, frozen_at: null | n}`; `open` also refuses a store whose active self log is frozen, I1). It refuses (`Conflict`) if a `sync_logs` row for `(self, incarnation)` exists.
  `OpenReport { self_log: LogKey, counter, parked_unknown, parked_dep, failed }` is logged once.
- `author::<K: FactKind>(tx, ctx, decision: DecisionInstant, payload: &K::Payload) -> Result<Authored, ApiError>` and `author_above::<K>(…, floor: HlcStamp)` (LWW writes):
  1. HLC: `tick(decision.utc)` or `tick_above`.
  2. Read the active self log: `counter = contiguous+1`, `prev_hash = head_hash`.
  3. `deps = applied_vector(tx)` minus the active self log.
  4. `payload = serde_json::to_vec`. Over 512 KiB is `InvalidRequest` "fact payload exceeds 512 KiB".
  5. `creates = K::creates`, `needs = K::needs` (sorted, deduplicated).
  6. `validate_shape`, then `seal`.
  7. Insert `apply_state='pending'` and advance `contiguous_counter`/`head_hash`.
  8. `apply::apply_authored(tx, ctx, ingest_seq, decision)` (§7).

  The result is `Authored { ingest_seq, counter, hlc, hash }`. A handler rejection, or any outcome other than `applied` (for example a `parked_dep` because the command referenced an entity whose creator is parked), is returned as that `ApiError` (`Conflict` "fact <kind> not applicable: <reason>" for parked outcomes). The caller's transaction then rolls back, so the entry, the counter and the HLC-stamped row all disappear and nothing is recorded `failed`.
- `serve(conn: &Connection, log: &LogKey, from: u64, limits: BatchLimits) -> Result<Vec<Envelope>, ApiError>` runs on a query connection (committed rows only). It returns `counter >= from ORDER BY counter`, stopping at 256 entries or when the next entry would pass 256 KiB of canonical size. It always returns at least one entry when `from ≤ contiguous`, even an oversized one. An unknown log or `from > contiguous` gives an empty vec.
- `vector(conn) -> Vector` covers every `sync_logs` row (`contiguous_counter`, `head_hash`, `state`, `frozen_at_counter`). `applied_vector(conn) -> Vec<(LogKey, u64)>` gives `applied_through > 0` rows. `stored_hash(conn, log, counter) -> Option<[u8;32]>` supports ht-5fr.6's Have-time comparison.
- `store_forked(tx, log, entries: &[Envelope], source: Option<NodeUuid>)` inserts verified entries into `journal_forked`, ignoring exact duplicates by `UNIQUE(log_ix, counter, hash)`. `freeze(tx, log, at: u64)` sets `state='frozen'`, `frozen_at_counter=at` (amended by [incarnations design §8](2026-10-14-cross-node-sync--incarnations-design.md#8-journal-changes): lower-only, returns `FreezeOutcome::{Frozen, Lowered, Unchanged}`, and refuses the active self log with `Conflict`). Both exist for ht-5fr.6 and are never called by ingest itself, because Journal reports forks and does not resolve them.

**Considered.**
- Changing the `apply` closure signature of the transaction wrappers to pass an authoring handle: it churns 77 call sites for no safety gain, since every apply closure already has `tx`, `context` and `decision` (§10).
- Authoring the genesis inside `schema::initialize`: the store would need the signing key, and ht-5fr.1 keeps `initialize` free of `NodeIdentity`.

### 6. Ingest

**Recommended.** `ingest(ctx, conn, log: &LogKey, entries: Vec<Envelope>, budget) -> Result<IngestOutcome, ApiError>`. The caller holds a FairWriter background turn.

1. **Before BEGIN:**
   - **Headroom.** Free space comes from `libc::statvfs` on the database directory (`f_bavail × f_frsize`). Below `headroom_bytes` (default `INGEST_HEADROOM_BYTES = 256 MiB`; test-support setter `StoreContext::set_ingest_headroom`) → `Headroom`, with nothing opened.
   - **Batch bounds.** More than 256 entries, or any `entry.log != log`, → `Corrupt{reason: Malformed}`.
2. **In one `BEGIN IMMEDIATE`:** load `HeldLog` (row, pubkey, contiguous, head_hash, state, `is active self`). For each entry in order, `chain::check_next` returns a `ChainStep`, which Journal acts on:

| Situation | ChainStep | Action |
|---|---|---|
| `log` is this node's active self log | `AheadOfSelf` | stop; `Fork{evidence: AheadOfSelf{log, my_head, entry}}` |
| no row, `counter != 1` | `Gap{expected: 1}` | stop |
| no row, `counter == 1` | `Genesis` | `genesis_pubkey` + `verify` (failure → Corrupt); insert `sync_logs` row; store |
| `counter ≤ contiguous`, same hash as stored | `Duplicate` | skip |
| `counter ≤ contiguous`, different hash, signature valid | `Conflict` | stop; `Fork{evidence: Conflict{held, conflicting}}` |
| frozen and `counter > frozen_at` | `BeyondFrozen` | stop; `Frozen{frozen_at}` |
| `counter > contiguous+1` | `Gap{expected}` | stop |
| `counter == contiguous+1`, `prev_hash ≠ head_hash`, signature valid | `Conflict` | stop; `Fork{…}` with the held entry at `counter-1` |
| `counter == contiguous+1`, `verify` fails | `Corrupt(err)` | roll back the whole transaction |
| `counter == contiguous+1`, valid | `Append` | store |

   A different-hash or prev-mismatch entry whose signature does not verify is `Corrupt`, not a fork. Store means: insert the journal row with `apply_state='pending'` and `ingested_at = decision.utc`, `Hlc::fold`, and advance `contiguous_counter`/`head_hash`. After the loop, update `skew_ahead_ms` and commit. The stored prefix stays committed on `Gap`, `Fork` and `Frozen`.
3. **Outcome** `IngestOutcome`:
   - `Stored{stored, duplicates, head}`
   - `Gap{stored, expected}`
   - `Fork{stored, evidence}`
   - `Frozen{stored, frozen_at}`
   - `Corrupt{log, counter, reason: EnvelopeError}`: nothing stored
   - `Headroom`: nothing stored

   `Err(ApiError)` is reserved for store errors (rolled back, nothing advanced). A `DiskFull` from SQLite inside the transaction is rolled back and returned as `Headroom`, not as an error, so ht-5fr.5 pauses pulls instead of penalizing the peer.
4. **Cost.** Signature verification runs inside the writer at about 50 µs per entry, so about 13 ms per 256-entry batch. That is accepted: it is below a single apply quantum, and verification needs the pubkey from the held row.

The payload is never parsed except in `genesis_pubkey`. Unknown kinds are stored and served unchanged.

**Considered.**
- Rejecting the whole batch on `Gap`: it wastes the prefix and re-requests it.
- Treating every hash mismatch as a fork: a corrupted transfer would then freeze a healthy log (root §10 separates corrupt from fork).
- Verifying signatures before taking the writer turn: it needs a pubkey cache outside the store and a second contiguity check, for about 13 ms saved per batch.

### 7. ApplyEngine: readiness, savepoints, outcomes, positions

**Recommended.** `src/store/apply.rs`.

- **Pass** `apply_pass(tx, ctx, decision, limits: ApplyLimits { entries: 64, wall_ms: 100 }) -> ApplyProgress { applied, parked, failed, replayed, has_more, self_tag_k: Option<u32> }`:
  1. Load the applied map from `sync_logs` (`applied_through`, `state`, `frozen_at_counter`).
  2. **Candidates:** for each log with `applied_through < contiguous_counter`, the entry at `applied_through+1`.
  3. **Ready** when every deps triple `(L, c)` has `applied(L) ≥ min(c, frozen_at(L))` (`frozen_at` = ∞ when active), and `L` has a row. A not-ready head sets that log's `blocked_on` to `deps <uuid8>/<inc>@<c>` (written only when it changes) and is skipped.
  3a. **Own-history hold** (root §7). While `SyncState.own_history.active(now)`, heads of logs whose `node_uuid` is not this node are not candidates (no `blocked_on` write; they stay `pending`). Own-log heads and `apply_authored` are unaffected. `OwnHistoryHold` holds two flags, `Links` (set by SyncCoordinator while an active link advertises an own log beyond its local `contiguous_counter`) and `Branches` (set by ht-5fr.6's IncarnationsSink while an own `sync_branches` row is `pulling`), plus the instant the hold began; `active(now)` is true while either flag is set and the hold is younger than 5 minutes (`OWN_HISTORY_HOLD_MAX`). Clearing the last flag, or the cap expiring, sets `apply_hint` and kicks `Lane::Apply`; expiry logs one line `sync: own-history hold expired after 5 min; applying foreign entries`. Nothing sets a flag on a fresh node (no own log advertised), so it never holds. The hold is in-memory: at start both flags are clear until the coordinator and sink re-derive them.
  4. Apply the ready candidate with the smallest `ingest_seq` (§ below), update the map, and repeat until no candidate is ready or a limit is reached. `has_more` reports a hit limit.
  5. **Re-attempt parked-on-created:** `SELECT p.ingest_seq FROM journal p WHERE p.apply_state='parked_dep' AND EXISTS (SELECT 1 FROM journal c WHERE c.creates=p.parked_on AND c.apply_state='applied') ORDER BY p.ingest_seq`, within the same limits. These entries are already counted in `applied_through` and are re-attempted through the needs check and the handler only.
- **Applying one entry** (`apply_one(tx, ctx, ingest_seq, mode)`):
  1. **Registry miss** for `(kind, kind_version)` → `parked_unknown`. This includes a newer major on a node that knows only older majors.
  2. **Needs:** for each key, if a `journal` row with `creates = key` and `apply_state='applied'` exists, the key is satisfied. Otherwise the outcome is `parked_dep`, with `parked_on` set to the first unsatisfied key and `apply_error` set to `needs <key>: creator <parked_unknown|parked_dep|failed|absent>`.
  3. `SAVEPOINT apply_entry`.
  4. `decision_seq = next_decision_seq(tx, node)`.
  5. Build the `ApplyContext` and call the handler.
  6. On `Ok`: set `apply_state='applied'`, `applied_seq=decision_seq`, `apply_error=NULL`. If the entry is the log head (not a replay), set `applied_through=counter` and clear `blocked_on`. Then author any queued follow-ups (below), then `RELEASE`.
  7. On `Err(FactError::Rejected(e))`: `ROLLBACK TO apply_entry; RELEASE`. Then set `apply_state='failed'` and `apply_error` to the first 512 bytes, cut at a char boundary, of `<code>: <detail>`.
  8. On `Err(FactError::Store(e))`: `ROLLBACK TO apply_entry; RELEASE`, then return `Err(e)`. The caller rolls back the whole quantum, and the Pacer retries with backoff.

  Parked and failed head entries still advance `applied_through` (root: they count as processed).
- **Error classification.** The erased handler maps an `ApiError` from store helpers with `FactError::from_api`. `StoreBusy`, `StoreFull`, `StoreCorrupt`, `IncompatibleSchema`, `Cancelled` and `DeadlineExceeded` map to `Store`. Every other code maps to `Rejected`, including `Conflict`, which `store_error` produces for constraint violations, so a natural-key UNIQUE guard firing on a remote fact is recorded loudly as `failed`. A payload that fails to decode is `Rejected(InvalidRequest "payload decode: <serde msg>")`.
- **Local authoring** (`apply_authored`) runs the same `apply_one` on the just-inserted head. Readiness holds by construction. Any outcome other than `applied` returns `Err` to `author` (§5). It never writes `failed`.
- **Follow-ups.** `ApplyContext::follow_up::<K>(payload)` queues a typed fact. After the triggering entry's `applied_through`/`apply_state` update, still inside its savepoint, ApplyEngine authors each queued fact through `journal::author` (which applies it inline). The follow-up's deps therefore include the trigger. A follow-up that fails rolls the trigger back to `failed` with `follow-up <kind>: <error>`.
- **ApplyContext** (passed `&mut` to handlers):
  - `origin: LogKey`, `counter`, `ingest_seq`, `hlc: HlcStamp`, `origin_ms() = hlc.ms()` (the origin's "sent at")
  - `decision: DecisionInstant` (the local apply time, or the command's decision when local)
  - `decision_seq` (the one local position allocated per entry; handlers use it and never call `next_decision_seq`)
  - `local_node: NodeUuid`
  - `provenance() -> Provenance::{Local, ReplicatedFrom(NodeUuid)}`, where `as_str()` gives `replicated_from:<uuid hex>` per the TRUST-POLICY amendment (ht-5fr.9). Entries whose origin node is this node, in any incarnation, are `Local`.
  - `is_local_origin() -> bool` (the origin node is self)
  - `is_local_seat(tx, &SeatId)` (delegates to ht-5fr.1's `seats::is_local_seat`)
  - `delivery_suppressed(tx, settle_key: &EntityKey) -> bool` (§8)
  - `follow_up::<K>(payload)`
  - `set_self_tag_k(k)` (post-commit, §11)

  Every other local position (message sequence, revisions, ordinals, cursors, membership interval seqs) is assigned by the handler's existing store writers inside the apply transaction. Payloads never carry them; this is FactKind review rule 1 (§9).
- **Replay.**
  - `replay(tx, ctx, decision, scope: ReplayScope::{Startup, Unapplied{states: &[ApplyState]}}, limits) -> ApplyProgress` iterates `journal_unapplied` in `ingest_seq` order.
  - `Startup` re-attempts `parked_unknown` entries whose `(kind, kind_version)` the registry now knows (one `SELECT DISTINCT kind, kind_version` decides which), plus re-attemptable `parked_dep` entries.
  - `Unapplied` is ht-5fr.8's `sync unapplied replay`, which re-attempts every entry in the named states, `failed` included.
  - Replays never move `applied_through`.
  - A replayed entry that applies gets a fresh `applied_seq` and therefore late local positions (root §5: local arrival order).
- **Catch-up is not special.** Bulk catch-up is many passes. Coalescing of nudges stays with DeliveryTail and the Wakes lane (root §7).

**Considered.**
- Research 02's `blocked_on` as a hard stop when a needed entity is absent: with deps covered, an absent creator can only be parked, failed or on a frozen tail, so blocking would wedge the log forever. Parking with `parked_on` and re-attempting is the root's `parked_dep` and never stalls.
- Keeping an in-memory "awaiting X" map: it is lost on restart, while the indexed query is durable and cheap.
- Allocating `decision_seq` only in handlers that need it: `applied_seq` would then be NULL for some applied entries, and the oracle could not reproduce local order.

### 8. Catch-up suppression in ApplyContext

**Recommended.** `ApplyContext::delivery_suppressed(tx, settle_key) -> bool` is true when either of these holds:
- (a) the entry's origin is this node in an incarnation other than `local_node.active_log_ix`'s (own old incarnation, root §7 wipe recovery);
- (b) a `journal` or `journal_forked` row with `creates = settle_key` exists in a log whose `node_uuid` is this node, in any apply state. The settlement was already authored by the recipient's home node, which is this node; after a fork it may sit on a branch the owner pulled into `journal_forked` (ht-5fr.6).

(b) only works when the node's own settlements are ingested before the foreign fact applies. After a wipe or restore they arrive in the node's own old logs, pulled independently of the foreign logs, so §7's own-history hold keeps foreign heads unapplied until the own logs are ingested.

The ApplyEngine never interprets the key. ht-5fr.4's `receipt.settled` kind sets `creates = "<message id>/<seat id>"`, and `invitation.answered` sets `creates = "<invitation id>/answer"`. Their `message.sent` and `invitation.created` handlers ask with the same keys before calling DeliveryTail. The check reads ingested rows, not applied ones, because a settlement can be ingested before the fact it settles is applied (`needs` holds it back).

**Considered.**
- A per-kind settled table written on apply: it misses settlements that were ingested but not yet applied, which is exactly the catch-up case.
- A new envelope field for settlement keys: the root fixes the envelope, and `creates` already is the kind-agnostic, indexed "this fact establishes X" column.

### 9. FactKind and FactRegistry

**Recommended.** In `src/sync/facts/mod.rs`:

```rust
pub trait FactKind: 'static {
    const NAME: &'static str;          // e.g. "node.retag"
    const MAJOR: u32;                  // envelope kind_version
    type Payload: Serialize + DeserializeOwned;
    fn creates(p: &Self::Payload) -> Option<EntityKey>;
    fn needs(p: &Self::Payload) -> Vec<EntityKey>;
    fn apply(tx: &Transaction<'_>, ctx: &mut ApplyContext<'_>, p: Self::Payload) -> Result<(), FactError>;
}
pub enum FactError { Rejected(ApiError), Store(ApiError) }
pub struct FactRegistry { handlers: BTreeMap<(&'static str, u32), ErasedApply> }
type ErasedApply = fn(&Transaction<'_>, &mut ApplyContext<'_>, &[u8]) -> Result<(), FactError>;
```

- `FactRegistry::builtins()` is built once in a `static REGISTRY: OnceLock<FactRegistry>`, the same pattern as `harness::registry::builtins()`. `registry()` returns it.
- `FactRegistry::builder().with::<K>()` builds a registry. A duplicate `(NAME, MAJOR)` panics at build time.
- Test support: `FactRegistry::builtins_plus(|b| b.with::<TestKind>())` returns a leaked `&'static` registry that `journal::open` accepts, so tests can add kinds and simulate an upgrade by reopening with a larger registry.
- **Version rules:** an additive payload field is a minor change. Payload structs use `#[serde(default)]` for new fields and never `deny_unknown_fields`. A breaking change adds a new `MAJOR` handler next to the old one. Old majors stay registered forever, because the journal is never deleted.
- **Review rules for every kind** (stated in the module doc, checked in review and by the oracle test):
  1. Payloads carry fact IDs and origin data only, never local positions.
  2. `needs` names only keys created by some fact's `creates`. Entities that ht-5fr.4 has not converted yet are left out.
  3. Handlers never author through `journal::author` directly; they use `follow_up`.
  4. Handlers never recompute audiences or reject facts based on the local membership view (root §6).
  5. Handlers never write `wake_work` and the other delivery tables directly. They use DeliveryTail (ht-5fr.4).
- **Kinds shipped here:**
  - **`node.genesis` v1:** payload `{pubkey, tag, tag_k, incarnation, predecessor: Option<{incarnation, frozen_at}>, host_label}` (amended by [incarnations design §8](2026-10-14-cross-node-sync--incarnations-design.md#8-journal-changes): `frozen_at` is itself optional, and a `Some` freeze is adopted on apply), creates none, needs none. Apply:
    1. Check `payload.incarnation == origin.incarnation` and `tag == tag_for(origin.node, tag_k)`, else `Rejected`.
    2. Upsert `node_directory(node_uuid, tag, tag_k, host_label, first_seen_at, tag_hlc)` through ht-5fr.1's `NodeDirectory`. The tag fields change only when `hlc > tag_hlc`.
    3. If `origin.node != self`, run the collision check (§11).
  - **`node.retag` v1:** payload `{k, tag}`, creates none, needs none. Apply:
    1. Check `tag == tag_for(origin.node, k)` and `k > 0`, else `Rejected`.
    2. Upsert `node_directory` if `hlc > tag_hlc`.
    3. If `origin.node == self`: `UPDATE local_node SET tag, tag_k` (only when `k > tag_k`), `ctx.set_self_tag_k(k)`, then run the collision check for the new self tag against all known nodes.
    4. Otherwise run the collision check for that node's new tag.

**Considered.**
- `inventory`-style distributed registration: a new dependency, and the explicit list is the root's "one registry entry" per kind.
- `dyn Any` payloads: the erased `fn` over bytes is enough and keeps decode errors inside `Rejected`.
- Making node kinds part of ht-5fr.4: the bead scopes them here, and ht-5fr.1's walk-through attributed retag to ht-5fr.4. This spec takes them, which narrows ht-5fr.4's list. ht-5fr.4 consumes `node_directory` as filled here.

### 10. Store transaction wrapper extension

**Recommended.** Commands author by calling `journal::author::<K>(tx, &context, decision, &payload)` (or `author_above`) inside their existing `apply` closure. That closure is the decide step (A2): it runs after `validate` and before commit. Every wrapper already passes `tx` and `decision`, so no closure signature changes. The wrapper changes are:

1. **Commit guard.** `StoreContext::execute_budgeted_decision_with_constraints` and `execute_decision` (`src/store/connection.rs`), the two writers under every wrapper, call `journal::check_self_applied(&tx)` after `apply` returns and before `tx.commit()`. It is one primary-key read: if the journal is open and the active self log has `applied_through ≠ contiguous_counter`, the call returns `StoreCorrupt` "self journal entry left unapplied" and the transaction rolls back. A local entry can therefore never commit without its materialization. Before `journal::open` it is a no-op.
2. **Idempotent replay.** `execute_budgeted_idempotent_transaction_with_constraints` replays a stored `operations` result without calling `apply`, so a retried command never authors twice. `operations` stays local and unreplicated, as the root says. This is pinned by a test.
3. **Writer pragma.** `open_writer` adds `PRAGMA journal_size_limit = 67108864` (64 MiB, research 02 §5). The Apply lane runs `PRAGMA wal_checkpoint(PASSIVE)` after a drain in which at least 1024 entries were applied since the last checkpoint.
4. **No claim check here.** The `cooperative_instance` → `claim.host` change in `execute_accountable_transaction` belongs to ht-5fr.2 and is not touched.

Ingest and apply quanta use `execute_budgeted_decision` directly, with a trivial `validate`, under the lane's background writer turn.

**Considered.**
- A thread-local or `StoreContext`-held "current fact scope": hidden state across 77 call sites.
- Changing the closure signature to a `FactWriter`: wide churn, and ht-5fr.4 converts call sites one kind at a time anyway.

### 11. Tag collision handling and post-commit identity update

**Recommended.** `collision_check(tx, ctx, other: NodeUuid, other_tag: NodeTag)` reads `local_node.(id, tag, tag_k)` and calls ht-5fr.1's `on_collision(self, self_k, other, other_tag)`. On `MoveTo{k, tag}` it queues `ctx.follow_up::<NodeRetag>({k, tag})`. After the self retag applies, its handler checks `node_directory::colliding(new_tag)` and queues another retag if this node is higher again, so a chain of k values converges. Only the higher node itself ever authors, and third parties never do.

Retags are only ever authored as follow-ups inside apply passes, never by a command. `ApplyProgress.self_tag_k` carries the new k out of the transaction. After commit, one post-commit helper, `node_retag::publish_committed_tag(store, identity, node_json)`, reads the committed `local_node.tag_k` on a query connection and, when it is above `identity.tag_k()`, calls `identity.set_tag_k(k)` and ht-5fr.1's NodeJson regenerate (idempotent otherwise). Both paths call it: the Apply lane worker after a pass whose `self_tag_k` is `Some`, and `run_elected_impl` once after `journal::open` commits. `OpenReport` has no `self_tag_k`: `open` applies only its own genesis inline, which never changes `tag_k`, and startup replay runs on the Apply lane, so open has no retag to carry out; the open-path call covers a crash between a retag's commit and its NodeJson regenerate. Minting already reads `local_node.tag` inside its own transaction, so the cache is never on the correctness path.

**Considered.** Applying the identity update from inside the transaction: a rollback would then leave the in-memory tag ahead of the store.

### 12. JournalPort and MemoryJournal

**Recommended.** In `src/sync/journal_port.rs`:

```rust
pub trait JournalPort: Send + Sync {
    fn vector(&self, budget: &CallBudget) -> Result<Vector, ApiError>;
    fn stored_hash(&self, log: &LogKey, counter: u64, budget: &CallBudget) -> Result<Option<[u8; 32]>, ApiError>;
    fn serve(&self, log: &LogKey, from: u64, limits: BatchLimits, budget: &CallBudget) -> Result<Vec<Envelope>, ApiError>;
    /// Caller holds a FairWriter background turn.
    fn ingest(&self, log: &LogKey, entries: Vec<Envelope>, budget: &CallBudget) -> Result<IngestOutcome, ApiError>;
    /// Caller holds a FairWriter background turn.
    fn apply_pending(&self, budget: &CallBudget) -> Result<ApplyProgress, ApiError>;
}
```

`SqliteStore` implements it. The read methods use `open_query`, and the write methods use the writer connection through `execute_budgeted_decision`. `StorePort` is not extended (root).

`MemoryJournal::new(identity)` keeps `BTreeMap<LogKey, Vec<Envelope>>` plus pubkeys and frozen marks, and runs the same `chain::check_next` and `Envelope::verify`. It never skips the signature invariant (TRUST-POLICY amendment, ht-5fr.9). Its `apply_pending` marks entries processed with no handlers, and `author_raw(kind, payload)` lets link tests create entries. ht-5fr.5 uses it for link and routing tests without SQLite.

**Considered.** A fake that skips verification: link tests would then pass on data that real nodes reject.

### 13. Lane::Apply and its worker

**Recommended.** In `src/service/kicks.rs`:
- `Lane` gains `Apply` (bit 6), and `COUNT`/`INTERNAL_ALL` become 7. `LaneSet` widens from `u8` to `u16`, because the `from_bits` mask `(1 << len) - 1` overflows `u8` at 8 lanes once ht-5fr.5 adds `Sync`.
- The literal `6`s in `CommitKicks.pacers`, `LaneFaults` (`src/app.rs`) and their tests become `Lane::COUNT`.
- `name()` returns `"apply"`.
- A new `pub fn kicks_for_commit(origin: Option<Lane>, sealed: LaneSet) -> LaneSet` replaces the `origin_discards_kicks` call in `KickHooks::on_commit` (`src/store/connection.rs`): Retention-origin commits still discard everything, and Apply-origin commits drop `Lane::Apply`, so the lane never kicks itself. `has_more` continues a drain.

Worker: `service::workers::start_apply_worker(store: Arc<SqliteStore>, post_commit: ApplyPostCommit, writer: Arc<FairWriter>, pacer: Arc<Pacer>, cancellation, status) -> io::Result<JoinHandle<()>>` runs on thread `herdr-apply` with `enter_lane(Lane::Apply)`. The worker does not go through `JournalPort`: that trait is ht-5fr.5's ingest/serve seam, and the worker needs more than it offers. It takes the concrete store handle, which exposes startup replay (`ReplayScope::Startup`), `apply_pending` and a passive WAL checkpoint, plus `post_commit: ApplyPostCommit` (a `Box<dyn Fn(&ApplyProgress) + Send>`) built by `app` around §11's `publish_committed_tag`, so the retag effects (`set_tag_k` + NodeJson regenerate) stay owned by the node kinds. It is the same shape as the retention worker:
- Its first pass is `ReplayScope::Startup` followed by `apply_pending`.
- It then waits on the Pacer with a 60 s safety tick.
- Each pass takes `writer.enter_background` with a 5 s budget.
- On `Ok`, `post_commit(&progress)` runs after the commit (it acts only when `self_tag_k` is `Some`, §11).
- After a drain that applied ≥ 1024 entries since the last checkpoint, it runs `PRAGMA wal_checkpoint(PASSIVE)` (§10 item 3).
- On a failure it calls `pacer.on_failure()` + `status.record_failure(Lane::Apply, &e)`. Health then shows `lane apply failed: <code> retrying (attempt n, …)`, one line of about 60 bytes, the existing format.

`app::start_apply_lane(kicks, clock, stop, journal, writer, status)` registers the Pacer and starts the worker from `run_elected_impl` right after `journal::open`, under `factory_stop`. A local command's `journal` commit kicks the lane. A pass that finds nothing ready writes nothing, so it commits no change and kicks nothing further. The lane's cost on local-only commits is bounded: a kicked pass first checks a cheap in-memory pending flag (`SyncState.apply_hint: AtomicBool`, set after commit by `ingest` when it stored at least one pending row, set at startup for the replay, and re-set on `has_more`; authoring applies inline and leaves it clear), and when the flag is clear it returns without taking a writer turn.

**Considered.**
- Mapping `sync_logs` instead of `journal` to `Apply`: ingest and authoring both write both tables, so it saves nothing.
- Applying inline at the end of each ingest transaction: the root separates ingest and apply so that relay never waits on apply.
- An explicit kick call from Journal: it bypasses the one table-driven kick mechanism.

## Testing

All store-level tests build stores with test identities: `NodeIdentity::from_seed([n; 32])`, a `#[cfg(any(test, feature = "test-support"))]` constructor added to ht-5fr.1's `src/node/identity.rs`. Each test passes them to `journal::open`, and two stores in one test act as two nodes. The test constructors of `SqliteStore` open the journal automatically with seed 1, so ht-5fr.4's converted writers work in every existing store test.

- **`src/sync/envelope.rs` unit tests:**
  - a golden canonical-bytes hex and hash for a fixed seed and fixed fields;
  - mutating any single field changes the hash;
  - `verify` returns `Ok`, `HashMismatch` (flipped payload byte) and `BadSignature` (another key);
  - `genesis_pubkey` returns `UuidMismatch` and `NotGenesis`;
  - `Deps` round trip, plus rejection of unsorted, duplicate, zero-counter, trailing-byte and own-log input;
  - every shape limit at its boundary;
  - `WireEnvelope` round trip preserves payload bytes exactly, including whitespace and key order.
- **`src/sync/hlc.rs` unit tests:**
  - `tick` stays strictly increasing when wall time steps back;
  - the logical counter carries over at 0xFFFF;
  - `fold` absorbs at +60 000 ms and returns `Skewed` at +60 001 ms;
  - `tick_above` returns `floor+1` and leaves `last` untouched when the floor is beyond the cap;
  - `resume` caps the self maximum at wall + 60 s.
- **`src/sync/chain.rs` unit tests:** one case per row of the §6 table.
- **`tests/store/journal.rs`** (`#[cfg(test)] #[path]` module of `src/store/journal.rs`):
  - `open` bootstraps the genesis at counter 1, applied, with `active_log_ix` set;
  - a second `open` is a no-op;
  - tampered `contiguous_counter` and tampered `head_hash` each refuse with the named invariant;
  - `author` chains `prev_hash` and gapless counters, carries the applied vector as `deps`, and rejects an oversized payload with nothing stored;
  - a handler rejection during `author` rolls everything back, leaving no journal row and an unchanged counter;
  - serve limits: 256 entries, 256 KiB, and ≥ 1 oversized entry;
  - every trigger aborts with its name: payload UPDATE, DELETE, applied→pending, `sync_logs` regression, unfreeze, self-flag mismatch, register regression.
- **Ingest tests** (same module, two stores A and B):
  - happy path `Stored`, rows `pending`;
  - duplicates are skipped;
  - `Gap` keeps the prefix;
  - a flipped byte gives `Corrupt` with nothing stored;
  - a foreign key gives `Corrupt(BadSignature)`;
  - two validly signed entries at one counter give `Fork{Conflict}` with the prefix kept;
  - an entry for B's own active log gives `Fork{AheadOfSelf}`;
  - a frozen log gives `Frozen`;
  - headroom set to `u64::MAX` gives `Headroom` with nothing opened;
  - ingesting B's own old incarnation is ordinary ingest;
  - a `Skewed` entry sets `skew_ahead_ms` and leaves the HLC unchanged.
- **`tests/store/apply.rs`** (module of `src/store/apply.rs`), using test kinds registered via `builtins_plus`:
  - deps readiness: B's entry deps-on A@2 waits, with `blocked_on` set, until A's log arrives, then applies;
  - a frozen dep is capped at `frozen_at`;
  - an unknown kind and a newer major give `parked_unknown`, which still advances `applied_through`;
  - `parked_dep` with `parked_on` is re-attempted in the same pass once its creator applies;
  - a creator that failed parks its dependents;
  - `Rejected` gives `failed` with a bounded message, and the handler's rows are gone (savepoint);
  - a `Store` error aborts the quantum with nothing changed;
  - a constraint violation in a handler becomes `failed`, not a wedge;
  - `applied_seq` is unique and increasing;
  - provenance is `replicated_from:<uuid>` for B's entry on A and `Local` for A's own;
  - `delivery_suppressed` is true for an own-old-incarnation origin and for a self-authored settle key in `journal` or `journal_forked`, and false otherwise;
  - own-history hold: with `Links` set, a ready head of B's log stays `pending` while an own-log head applies; clearing the flag applies B's head in the next pass; a hold older than 5 minutes (paused clock) no longer filters; **wipe-then-pull**: store B authors a `receipt.settled` for A's message, B is wiped (same seed), A's log is ingested first with the hold set, then B's old log; A's `message.sent` applies only after the hold clears and `delivery_suppressed` is then true for the settle key;
  - a follow-up's deps include its trigger, and a failed follow-up marks the trigger `failed`;
  - `ReplayScope::Startup` after reopening with a larger registry applies the old `parked_unknown` entries in `ingest_seq` order with late `applied_seq`;
  - `Unapplied{failed}` re-attempts.
- **`tests/store/node_facts.rs`:**
  - genesis fills `node_directory`;
  - two stores whose tags are forced equal (tests set `local_node.tag` and use `tag_for` collision fixtures): the higher uuid authors `node.retag`, the lower does not, and after exchange both directories agree;
  - the next `public_ids::fresh` on the higher store mints under the new tag;
  - `set_tag_k` is called post-commit;
  - a retag with a mismatched tag gives `failed`.
- **`tests/store/thread_registers.rs`:** all 24 arrival orders of four writes converge to the same winner; an equal `(hlc, node)` loses; the trigger refuses a regression.
- **Oracle:** `test_support::journal_oracle::{replay_into_empty(src) -> Store, replicated_projection(store) -> BTreeMap<table, rows>}`. This subepic covers `node_directory`, `thread_registers` and the test kinds' tables. ht-5fr.4 adds each converted table's replicated columns.
  - The replay store has its own seed identity (not src's seed 1), so its own genesis and self log are distinct from every log of src.
  - `replay_into_empty` ingests src's logs one origin log at a time (logs in `LogKey` order, entries in counter order, through `JournalPort::ingest`), then drains `apply_pending`; the engine applies by readiness and `ingest_seq`.
  - The projection excludes rows that originate from the replaying node (its own genesis and its self `node_directory` row) and every local-position column (`applied_seq`, `decision_seq`, `sequence`, `ingest_seq`, `src_ingest_seq`, revisions and ordinals). Rows are keyed by their replicated columns, so the comparison is order-insensitive.
  - This does not reproduce src's applied order: any causal linear extension can result. Order-insensitive comparison suffices because FactKind review rules make the replicated columns a function of the applied fact set (LWW by `(hlc, node)`, needs/creates gating, no local positions in payloads), which is exactly the property the oracle checks. A parked-then-applied entry in src may apply directly in the replay; the projection is the same.
  - Strict mode compares local positions by relative order, not raw values: for each local-position column it checks that rows whose originating entries are causally ordered (same log by counter, deps, or needs → creates) are ordered the same way in both stores, since every causal apply order preserves those pairs. The replay store's position counters start from a high offset, so a kind that copies src's position from its payload shows up as a value outside the replay store's own range and fails strict mode.
- **Transaction wrapper:**
  - a command that authors through `execute_accountable_transaction` and is retried with the same operation key replays from `operations` and leaves exactly one journal entry;
  - a forced unapplied self entry trips `check_self_applied` and rolls back.
- **JournalPort contract** (`tests/sync/journal_port_contract.rs`, a module of `tests/combined.rs` with `required-features = ["test-support"]` already on that target): one parameterized suite over `SqliteStore` and `MemoryJournal` covering the vector, serve limits, every ingest outcome and the stored hash.
- **Kicks:**
  - `tests/service/kicks_contract.rs`: the table-classification test passes with the new tables; `journal` kicks `Apply`; `kicks_for_commit(Some(Apply), {Apply, Wakes}) = {Wakes}`; `LaneSet` holds 8 lanes.
  - The apply worker (unit test in `src/service/workers.rs`): ingest into a store, kick, and the entry is applied within one pass. An injected lane fault degrades Health with the `lane apply failed` line, and it recovers on success.
- **Checks:** `nice cargo clippy --locked --all-targets --all-features -- -D warnings`, `scripts/check-harness-neutral`, `nice scripts/check-default-features`, and focused `nice cargo test --locked --all-features journal apply envelope hlc chain node_facts thread_registers kicks`. No daemon is spawned by these tests.

Suggested task cut (files):
1. Envelope, Deps, wire form and chain rules: `src/sync/{mod,envelope,chain}.rs`, `Cargo.toml` (`raw_value`), `src/lib.rs`.
2. Hlc: `src/sync/hlc.rs`, `src/store/connection.rs` (`SyncState` field).
3. DDL and triggers: `migrations/node/0001_node_baseline.sql`, `src/service/kicks.rs` (classification only).
4. Journal open/author/serve/vector and the wrapper guard: `src/store/journal.rs`, `src/store/connection.rs`, `src/store/schema.rs` (re-export), `src/store/mod.rs`.
5. FactKind/FactRegistry and ApplyEngine: `src/sync/facts/mod.rs`, `src/store/apply.rs`.
6. Ingest and JournalPort, plus MemoryJournal and the contract suite: `src/store/journal.rs`, `src/sync/journal_port.rs`, `src/sync/memory_journal.rs`, `tests/sync/journal_port_contract.rs`, `tests/combined.rs`.
7. Node kinds and collision: `src/sync/facts/node_{genesis,retag}.rs`, `src/node/identity.rs` (`from_seed`).
8. Thread registers: `src/store/thread_registers.rs`.
9. Lane::Apply, the worker and daemon wiring: `src/service/kicks.rs`, `src/service/workers.rs`, `src/app.rs`, `src/store/connection.rs` (`kicks_for_commit`, `journal_size_limit`).
10. The oracle harness: `src/test_support/journal_oracle.rs`.

Order: 1 → 2 → 3 → 4 → 5 → (6, 7, 8) → 9 → 10.

## Key abstractions

**LogKey / LogHead / Vector** — new value structs (`src/sync/mod.rs`), the name and replication state of one origin log.
- Owns: `LogKey { node, incarnation }` as the only identity of a log, ordered by `(node bytes, incarnation)` (the deps order); `LogHead` (contiguous counter, head hash, `LogState::{Active, Frozen}`, `frozen_at`); `Vector` as the node's full per-log head map; `BatchLimits`.
- Hides: the local `log_ix` interning. Only Journal maps `LogKey` to `log_ix`, and it never leaves `src/store/journal.rs` and `src/store/apply.rs`.
- Must not: carry an "is self" meaning. Self-ness comes from `local_node.active_log_ix` and the node uuid, so an older own incarnation is an ordinary log.
- Relationships: leaf types under Envelope, ChainRules, JournalPort and SyncCoordinator. They depend only on `NodeUuid`.

**EntityKey** — new value struct (`src/sync/mod.rs`), the kind-agnostic name of something a fact establishes or depends on.
- Owns: the 1..160-byte `[0-9A-Za-z._:/-]` grammar, its validating constructor and its `Ord` (sorted, unique `needs`), and the `/`-joined composite form for settlement records (`m<id>/s<id>`, `i<id>/answer`).
- Hides: nothing beyond the grammar. ApplyEngine and Journal compare keys as opaque strings and never split them.
- Must not: be parsed by the engine to learn a kind or an entity type. Only the kinds that build composite keys know their parts.
- Relationships: produced by `FactKind::creates`/`needs`. Stored in `journal.creates` and `journal.needs`. Read by readiness (`parked_on`) and by `ApplyContext::delivery_suppressed`.
- Errors: construction rejects bad length or characters as `EnvelopeError::Malformed{field}`.

**ChainRules** — new pure module (`src/sync/chain.rs`): `check_next` over `HeldLog` → `ChainStep`, the one statement of the per-log chain rules.
- Owns: the decision table of §6 (`AheadOfSelf`, `Gap{expected}`, `Genesis`, `Duplicate`, `Conflict`, `BeyondFrozen`, `Append`, `Corrupt(EnvelopeError)`), including "a mismatched hash with a bad signature is `Corrupt`, never a fork".
- Hides: the order in which contiguity, freeze, prev-hash and signature are checked.
- Must not: touch the store or the clock, act on a step, or read the payload (except `Envelope::genesis_pubkey` on `Genesis`).
- Relationships: called by Journal's ingest and by `MemoryJournal`, which act on the step. It depends on Envelope and LogKey only. It is the reason the two `JournalPort` implementations cannot disagree.
- Errors: none of its own. `ChainStep` is a closed enum, and callers `match` it exhaustively.

**SyncState** — new struct (`src/store/connection.rs`, in `StoreContext.sync: OnceLock`), the journal's per-daemon runtime installed once by `journal::open`.
- Owns: the `Arc<NodeIdentity>`, the `Mutex<Hlc>`, the `&'static FactRegistry` and the ingest headroom threshold, the "journal is open" gate, and `own_history: OwnHistoryHold` (§7 step 3a: two atomic flags and a start instant, set by SyncCoordinator and the incarnations sink, read by the apply pass; it takes no lock).
- Hides: whether a given store has a journal. Before `open`, authoring and ingest return `Unsupported "journal not open"`, and `check_self_applied` is a no-op.
- Must not: be locked outside `src/store/journal.rs`. Only Journal ticks or folds the Hlc, and only inside a writer transaction. It must not be replaced after install (registry upgrades are a reopen).
- Relationships: owned by StoreContext. Read by Journal, ApplyEngine (registry) and the commit guard.
- Lifecycle: set once per StoreContext. It lives as long as the store, and tests install a larger registry by reopening.
- Errors: "journal not open" is a programming error, distinct from every store and fact error.

**ThreadRegisters** — new store module (`src/store/thread_registers.rs`), the only owner of `thread_registers`.
- Owns: the LWW upsert keyed by `(thread, field)` with the `(HlcStamp, NodeUuid)` tie-break, its `RegisterOutcome::{Won, Lost}`, and `current` (the floor for `author_above`). The register field set (`topic`, `name`, `archived`) is a closed enum matching the CHECK.
- Hides: the `ON CONFLICT … WHERE` form and the monotone trigger.
- Must not: write `threads` (ht-5fr.4's handlers copy winners), reference `threads` by FK (root §4), or read the local clock.
- Relationships: called by future `thread.*` handlers and by authors choosing a floor. It depends on HlcStamp and NodeUuid only.
- Errors: losing is `Lost`, not an error. A trigger abort means a broken caller and surfaces as a store error.

**Envelope** — extended: concrete v1 canonical layout with the `HTJRNL01` magic, `UnsignedEnvelope::seal`, `validate_shape` (the envelope v1 limits), `Deps` (unique sorted triples, own log excluded), `WireEnvelope` (hex fields plus a `RawValue` payload that is never re-encoded) and the closed `EnvelopeError::{Malformed{field}, HashMismatch, BadSignature, UuidMismatch, NotGenesis}`.

**Hlc** — extended: `HlcStamp` with `tick_above` (LWW floor, absorbing only under the 60 s cap), `Fold::{Absorbed, Skewed{ahead_ms}}` and the capped `resume`. It lives in SyncState.

**Journal** — extended: `open` (startup invariants, HLC resume, genesis bootstrap through `begin_incarnation`), `author`/`author_above` generic over `FactKind`, the commit guard `check_self_applied`, the headroom and batch-bound checks before BEGIN, and `store_forked`/`freeze` exposed for Incarnations. The concrete `IngestOutcome::{Stored, Gap, Fork{ForkEvidence}, Frozen, Corrupt, Headroom}` keeps "nothing stored" (`Corrupt`, `Headroom`) apart from "prefix kept" (`Gap`, `Fork`, `Frozen`), and `Err` stays reserved for store errors.

**JournalPort / MemoryJournal** — extended: the port gains `apply_pending` so the Apply lane worker needs no second trait. `MemoryJournal` runs the same ChainRules and verification, with a handler-less `apply_pending` and `author_raw` for link tests. Per spec §1/§12, the port is therefore wider than "narrow for SyncCoordinator"; the alternative was a single-implementation `ApplyPort`.

**FactKind / FactRegistry** — extended: the concrete trait (`NAME`, `MAJOR`, typed `Payload`, `creates`, `needs`, `apply`), the erased `fn` over payload bytes, a builder that panics on a duplicate `(NAME, MAJOR)`, test-only `builtins_plus`, and `FactError::{Rejected, Store}` with `from_api` as the one classifier of `ApiError` codes. The first two kinds, `node.genesis` and `node.retag`, are added with the collision check, which acts on `on_collision` through `follow_up`.

**ApplyEngine** — extended: `apply_pass` (head candidates in `ingest_seq` order, deps capped at `frozen_at`, `blocked_on`, foreign heads held while the own-history hold is active), `apply_one` with the registry-miss → needs → savepoint → handler order, `parked_on` re-attempt by the indexed `creates` query (no in-memory wait map), `apply_authored` for local entries (never writes `failed`), `replay` with the closed `ReplayScope::{Startup, Unapplied{states}}`, `ApplyProgress`, and one `decision_seq` per applied entry stored as `applied_seq`. Per spec §7, follow-ups call back into `journal::author` from inside `apply_one`, so `store::apply` and `store::journal` depend on each other. The cycle is deliberate: it puts the follow-up's deps after its trigger.

**ApplyContext** — extended: origin, counter, `ingest_seq`, hlc, decision, `decision_seq`, the closed `Provenance::{Local, ReplicatedFrom(NodeUuid)}` (any own incarnation is `Local`), `is_local_seat`, `delivery_suppressed` (kind-agnostic, over ingested own `creates` keys in `journal` or `journal_forked`, plus own-old-incarnation), the follow-up queue and `set_self_tag_k`, which carries a tag change out to be applied after commit.

**Lane (`service/kicks.rs`)** — extended: `Apply` (bit 6), `LaneSet` widened to `u16`, and `kicks_for_commit` as the one rule for which lanes a commit kicks, so Apply never kicks itself.

**Store transactions** — extended: the two base writers call `check_self_applied` before commit. Closures author through `journal::author` with no signature change. Idempotent replay never re-authors.

Variation: fact kinds are the only open-ended set. Each is a `FactKind` module, and `FactRegistry::builtins()` is the one place kinds are chosen; a miss means `parked_unknown`. `JournalPort` has exactly the two implementations the spec names (`SqliteStore`, `MemoryJournal`), and `ChainRules` is shared so they cannot diverge. Everything else is fixed and is a closed enum handled with `match`: `ChainStep`, `IngestOutcome`, `ForkEvidence`, `EnvelopeError`, `FactError`, `Fold`, `LogState`, apply states, `ReplayScope`, `Provenance`, `RegisterOutcome` and the register field. Envelope layout changes are a new magic, not a trait.

Walk-through: a command on A calls `journal::author::<K>` inside its accountable transaction. Journal ticks the Hlc in SyncState, reads the active self log, takes `applied_vector` as `Deps`, seals the Envelope with NodeIdentity and inserts the row as `pending`. ApplyEngine's `apply_authored` runs `apply_one`: the registry hit, needs met by applied `creates` rows, a savepoint, one `decision_seq`, then the handler with an ApplyContext whose provenance is `Local`. The commit guard sees `applied_through = contiguous`. The `journal` commit kicks `Lane::Apply`, whose pass finds nothing ready. Later, B's PeerLink hands A's batch to `JournalPort::ingest`. Journal checks headroom and bounds, ChainRules returns `Append` for each entry, the rows are stored `pending` with the Hlc folded, and the commit kicks Apply. B's worker calls `apply_pending`: deps are covered, the handler runs with `ReplicatedFrom(A)`, and the outcome is recorded. Likely future change: ht-5fr.4 adds `thread.register`. That is one `FactKind` module whose handler calls `ThreadRegisters::upsert` and copies the winner to `threads`, plus one `.with::<ThreadRegister>()` in `builtins()`. The command authors with `author_above` and the floor from `ThreadRegisters::current`. Journal, ChainRules, ApplyEngine and JournalPort are untouched; only the test oracle's projection gains that table's columns.

Digest:
- LogKey/LogHead/Vector: owns log identity, head state and the per-log vector; new per-log replicated state goes in `LogHead`;
  callers never use `log_ix` outside Journal/ApplyEngine or treat a log as "self" by flag.
- EntityKey: owns the creates/needs key grammar and ordering; new entity or settlement keys are new composite spellings built in their kind;
  the engine never splits or interprets a key.
- ChainRules: owns the per-log append/duplicate/gap/fork/frozen/corrupt decision; a new chain rule is a `ChainStep` variant here;
  Journal and MemoryJournal never re-derive chain checks themselves.
- SyncState: owns the installed identity, Hlc, registry and headroom, the "journal open" gate and the own-history hold; new journal runtime state goes here;
  nothing outside `store::journal` locks the Hlc, and nothing swaps the registry after open.
- ThreadRegisters: owns `thread_registers` and the LWW tie-break; new LWW fields are new register-field variants here;
  handlers never write the table or compare `(hlc, node)` themselves.

## Post-Implementation Notes

> *As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
