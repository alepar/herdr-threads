# Bytes-first journal — design

Run: [2026-10-18-bytes-first-journal](.) · Mode B (one-shot) · base `main` @ v2.2.1 · amends [peer wire compat hardening](../2026-10-18-peer-wire-compat-hardening/2026-10-18-peer-wire-compat-hardening-design.md) (§2, §4, promotion) before it ships as 2.3.0

## Goal

Every journal row stores the entry's original signed canonical bytes, its hash and its signature as the source of truth, with the parsed fields as a derived projection. An entry in a layout this node cannot parse is stored with an empty projection and relayed byte-exactly. After an upgrade that understands the layout, such an entry is promoted by re-deriving its projection, and the log's stalled successors apply. All of this ships without any wire change and with no separate opaque table.

## Problem description

The peer-wire-compat-hardening branch (unreleased) added `journal_opaque`, a second table for unknown-layout entries, with union readers (`CHAIN_ROWS`) at every chain-defining read site. Promotion to an applied entry was deferred (ht-evlm), because materialising a fact needs a `journal` row: `thread_registers.src_ingest_seq` and `sync_reissues.target_ingest_seq` reference `journal(ingest_seq)`, and apply bookkeeping keys on journal rows. The user chose (main-tab message mQ0V2b8xmSWQ) to make storage bytes-first before 2.3.0: one table, raw bytes canonical, projection derived. Relay is then opaque by default and promotion is a projection update.

## Main challenges

(a) SQLite cannot relax `NOT NULL` on existing columns. Making the projection columns nullable needs a table rebuild under foreign-key references from `thread_registers` and `sync_reissues`, and the guard triggers and indexes must be recreated. (b) Node deltas are SQL-only today, but recomputing HTJRNL01 canonical bytes is easiest in Rust; the delta mechanism needs a post-SQL step. (c) The 2.3.0 branch already ships StoredEntry, the capability gate, the oversize path, fork evidence and apply stalls built on two tables. Those must move onto one table without changing their behaviour or wire output. (d) Promotion must not break the immutability of signed data while it updates the projection.

## Key decisions made

1. `journal` and `journal_forked` gain `raw BLOB NOT NULL` (canonical bytes). Their projection columns become nullable, all-or-none per row (`hlc, kind, kind_version, deps, needs, payload`; `creates` stays optional). `journal_opaque` and `CHAIN_ROWS` are removed. Delta 0005 is rewritten in place (unreleased; no store has applied it outside test fixtures).
2. `NodeDelta` gains an optional Rust `post` step run in the same transaction as the delta's SQL. 0005's post step backfills `raw` for every pre-existing row by recomputing HTJRNL01 canonical bytes from the projection, and refuses with `StoreCorrupt` unless `SHA-256(raw) == hash` for every row. That check is what makes the migration provably exact.
3. One reader: rows decode to `StoredEntry::Known` (projection present) or `StoredEntry::Opaque` (projection NULL) from the single table. Serving uses `raw` for opaque items, and the known wire shape is unchanged (built from the projection, exactly as today).
4. Promotion is a projection update guarded by a trigger exemption that allows writing projection columns only when they were NULL. It runs at store open and in the apply pass for rows whose magic the binary now parses (including a test-support understood-layout hook). It sets `apply_state='pending'` and clears the log's `blocked_on` in the same transaction, so successors apply through the normal pass. ht-evlm closes as superseded.

## Decision points, by section

### 1. Row shape

Recommended: `journal(ingest_seq, log_ix, counter, prev_hash, hash, signature, raw, hlc?, kind?, kind_version?, deps?, creates?, needs?, payload?, ingested_at, apply_state, applied_seq, parked_on, apply_error)`, where `?` marks columns that are NULL for an unknown layout.

Constraints:
- A CHECK that the projection is all-present or all-null.
- `length(raw) <= 524288 + header bound`.
- `CHECK(length(raw) >= 68)`.
- The existing per-column checks, now applied only when the column is non-NULL.
- `UNIQUE(log_ix, counter)` kept.

`journal_forked` mirrors this shape (raw, nullable projection, the same CHECK). The guard triggers become:
- `journal_immutable` forbids changing `log_ix, counter, prev_hash, hash, signature, raw, ingest_seq` always, and the projection columns unless every old value is NULL (promotion);
- `journal_append_only` is unchanged.

The prefix fields (`log_ix`, `counter`, `prev_hash`) stay as columns: they are needed for indexes and chain continuity, and the CHECK ties them to `raw` through an ingest-time verification (EntryPrefix).

Discarded:
- Keeping `journal_opaque` (two tables, union readers, promotion impossible without new FKs).
- A sentinel projection for unknown rows (violates the checks, and lies about the data).
- Storing only `raw` with no projection columns (every query would parse blobs; indexes on kind and creates would be lost).

### 2. Unknown layout apply semantics

Recommended: an unknown-layout row is inserted with `apply_state='parked_unknown'` and a NULL projection. Unlike a known-kind `parked_unknown` row, which advances `applied_through`, the apply pass stops a log at a row whose projection is NULL. It writes `sync_logs.blocked_on = 'opaque_layout'`, keeping the 2.3.0 status surface and wording. Dependents in other logs keep the existing `deps <node8>/<inc>@<c>` blocked state. The distinction is made in one place: the apply head scan reads the projection-NULL flag.

Discarded: a new `apply_state` value (it would change the CHECK list and every status counter for no gain).

### 3. Migration (delta 0005, rewritten)

Recommended: delta 0005 does the following in one transaction:
1. With `PRAGMA defer_foreign_keys=ON`, create `journal_new` and `journal_forked_new`.
2. `INSERT … SELECT` all rows with `raw` set to an empty placeholder, rejected later.
3. Drop the old tables, rename the new ones and recreate the indexes and triggers.
4. Run `PRAGMA foreign_key_check`, which must be empty.

The Rust post step then computes canonical bytes for every row through the existing `UnsignedEnvelope::canonical_bytes` (a projection plus the log's node and incarnation from `sync_logs`). It sets `raw` through a migration-only exemption marker, and asserts `SHA-256(raw) == hash` and that `verify` passes against the log's genesis public key where the genesis is known. Any mismatch aborts the migration with `StoreCorrupt` and the store stays at schema 4. The placeholder is never visible after commit: the post step's final query asserts that no row has an empty `raw`.

`NodeDelta { version, sql, post: Option<fn(&Transaction) -> Result<(), ApiError>> }` is the one extension point. The 2.3.0 delta (journal_opaque) is replaced wholesale, keeping the number 0005, since nothing released carries it.

Discarded:
- Pure-SQL canonical encoding with `unhex`/`printf` and a recursive CTE for `needs` (fragile, and duplicates the encoder).
- Lazy backfill on read (two code paths forever).

### 4. Reads, serving and relay

Recommended: one row decoder (`journal::entries_from` and its siblings) yields `StoredEntry`. `CHAIN_ROWS` and every union is deleted, and the readers go back to plain `journal`/`journal_forked` queries. Unchanged in behaviour and tests:
- serving (`WireItem::Known` from the projection, byte-identical to today; `WireItem::Opaque` from `raw`, only toward `envelope.opaque.v1`);
- the capability gate, the withheld-log parking and `withheld_logs`;
- the oversize path (OPAQUE_MAX_BYTES applies to unknown-layout `raw`);
- fork evidence (forks between known and unknown rows now live in `journal_forked` with nullable projection);
- resync salvage (an old backup has no `raw` column: salvage detects schema 4 and reads the projection, recomputing canonical bytes as the migration does).

Known-layout ingest stores `raw = canonical_bytes()` of the verified envelope.

Discarded: serving known entries from `raw` by re-parsing (no benefit; the projection is authoritative for known layouts, and their bytes are equal by hash).

### 5. Promotion

Recommended: `journal::promote(tx, log, counter)` parses `raw` with the layouts the binary understands (production: HTJRNL01 only; test-support: an understood-layout registry the synthetic HTJRNL99 builder can register). On success it:
- writes the projection columns (allowed by the trigger only from all-NULL);
- sets `apply_state='pending'`;
- clears `sync_logs.blocked_on` when it reads `opaque_layout`.

All of this happens in one transaction, and the normal apply pass then applies the entry and its successors (ingest_seq is the row's own, so FKs from materialisation work unchanged). Triggers: at store open (after migrations), and in the apply pass when the head row is projection-NULL and parseable. The promoted entry's `raw` and hash are untouched. It is then served as `Known` to every peer, because a known projection exists and its canonical bytes equal `raw` by hash. If the projection's re-encoding were not equal to `raw` for a future layout, that layout's own serving rules apply; this release understands only HTJRNL01 in production.

Discarded: promotion by moving rows between tables (the 2.3.0 dead end).

### 6. Kept from 2.3.0 unchanged

The following stay as they are: the capability `envelope.opaque.v1`, the WireItem decode, EntryPrefix and OpaqueEntry verification, golden fixtures and `scripts/freeze-peer-wire`, the 2.1.1 shadow decoder, `scripts/check-wire-compat` and its baseline, the CI tag fetch with `HT_REQUIRE_TAGS`, the prefix/envelope=1/genesis pins, the security fix for authenticated oversize claims, TRUST-POLICY's accepted-limit text (wording adjusted: entries are stored in the journal itself), and the docs (operations: "opaque positions stay stalled until promoted by an upgrade that understands their layout").

### 7. Tests

Required:
- A schema-4 store with representative rows (every builtin fact kind, a forked pair, applied and parked rows) migrates. Every row's `raw` equals canonical bytes, `SHA-256(raw) == hash`, and serving every log produces frames byte-identical to the frames served from the schema-4 store before migration (captured before upgrading).
- A corrupted projection makes the migration abort with the store left at schema 4.
- An unknown-magic (HTJRNL99) entry is stored with a NULL projection, relayed byte-exactly to a capability peer, withheld from a non-capability peer, and stalls its log.
- After the test-hook upgrade, the entry is promoted: projection filled, raw/hash unchanged, the stalled successors apply, and `blocked_on` clears.
- The trigger refuses any projection change on a known row and any raw/hash change.
- Existing 2.3.0 suites (`opaque_relay`, `peer_wire`, `legacy_wire`, `journal_port_contract`, resync, incarnations) stay green.

### 8. Rollout

Ships as 2.3.0 (minor: no wire change, a local schema delta, downgrade refused as today). ht-evlm closes as superseded. The 2.3.0 branch's `journal_opaque` design notes are superseded by this spec, recorded in that spec's Post-Implementation Notes.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
