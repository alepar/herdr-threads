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
2. `NodeDelta` gains `rebuild` and a Rust `post` step. Delta 0005 rebuilds `journal`/`journal_forked` with `foreign_keys` off outside the transaction (the SQLite-documented procedure). The Rust step copies every row while computing `raw` from the projection, and aborts with `StoreCorrupt` unless `SHA-256(raw) == hash` for every row, which makes the migration provably exact. No placeholder and no trigger exemption for `raw` exist.
3. One reader: a row decodes to `StoredEntry::Known` only when `raw` carries the HTJRNL01 magic (its projection is then present and its canonical encoding equals `raw`). Every other row decodes to `StoredEntry::Opaque` from `raw`, whether its projection is NULL or filled by promotion. Chain checks, verification, fork evidence, re-issue and serving therefore never re-hash a foreign layout as HTJRNL01. Apply reads the projection columns directly. Serving uses `raw` for opaque items, and the known wire shape is unchanged (built from the projection, exactly as today).
4. Unknown-layout rows use a new `apply_state='parked_layout'`, not `parked_unknown`, so existing readers stay correct. Promotion is a projection update, allowed by the trigger only from an all-NULL projection on `journal` rows. It runs at store open and in the apply pass for rows whose magic the binary now parses (including a test-support understood-layout hook), sets `apply_state='pending'` and clears the log's `opaque_layout` block in one transaction, so successors apply through the normal pass. A promoted entry is served Known only if its HTJRNL01 encoding equals `raw`, and otherwise opaque. ht-evlm closes as superseded.

## Decision points, by section

### 1. Row shape

Recommended: `journal(ingest_seq, log_ix, counter, prev_hash, hash, signature, raw, hlc?, kind?, kind_version?, deps?, creates?, needs?, payload?, ingested_at, apply_state, applied_seq, parked_on, apply_error)`, where `?` marks columns that are NULL for an unknown layout.

Constraints:
- A CHECK that the projection is all-present or all-null.
- `length(raw) <= RAW_MAX` where `RAW_MAX = 786432` (768 KiB), a fixed number written into the DDL. It covers the largest valid HTJRNL01 entry (68-byte prefix + hlc + kind (≤64) + kind_version + DEPS_MAX×28 ≈ 114.7 KB deps + creates (≤162) + NEEDS_MAX×(2+160) ≈ 41.5 KB needs + PAYLOAD_MAX 524288 + length prefixes ≈ 681 KB), with headroom. It is also far above OPAQUE_MAX_BYTES (384 KiB), which still bounds unknown-layout raw at ingest. A const assertion in `envelope.rs` checks that the computed maximal canonical size is ≤ RAW_MAX.
- `CHECK(length(raw) >= 68)`.
- The existing per-column checks, now applied only when the column is non-NULL.
- `UNIQUE(log_ix, counter)` kept.

`journal_forked` mirrors this shape (raw, nullable projection, the same CHECK). The guard triggers become:
- the full trigger set is listed in §3 (`journal_envelope_immutable` keeps `ingested_at`; `journal_applied_terminal` and the forked triggers are recreated).

The prefix fields (`log_ix`, `counter`, `prev_hash`) stay as columns: they are needed for indexes and chain continuity, and the CHECK ties them to `raw` through an ingest-time verification (EntryPrefix).

Discarded:
- Keeping `journal_opaque` (two tables, union readers, promotion impossible without new FKs).
- A sentinel projection for unknown rows (violates the checks, and lies about the data).
- Storing only `raw` with no projection columns (every query would parse blobs; indexes on kind and creates would be lost).

### 2. Unknown layout apply semantics

Recommended: an unknown-layout row is inserted with a NULL projection and a new `apply_state='parked_layout'` (the CHECK list changes anyway, since the table is rebuilt). The user's brief said `parked_unknown`; the design roast showed that reusing it breaks existing readers that read `kind`/`kind_version` as non-null for every `parked_unknown` row (`replay_candidates` at startup and operator replay, the unapplied status page) and silently changes the `parked_unknown` counters. A distinct state keeps every existing reader correct and gives status its own counter.

The apply pass stops a log at a `parked_layout` row and writes `sync_logs.blocked_on = 'opaque_layout'`, keeping the 2.3.0 status surface and wording; dependents in other logs keep the existing `deps <node8>/<inc>@<c>` blocked state. `sync status` reports `parked_layout` as its own count.

Reader audit: classification in SQL uses the same rule as the decoder, the magic of `raw` (`substr(raw,1,8) = CAST('HTJRNL01' AS BLOB)`), never projection presence. The rules are:
- known-entry predicates (incarnations `pending_reissues`, the known re-issue candidate arm, `kind <> ?` filters) add the HTJRNL01-magic condition;
- every former `journal_opaque` arm (the opaque re-issue candidates, `cannot_reissue`, fork evidence) selects the non-HTJRNL01 rows, promoted or not, so a promoted foreign-layout row in a frozen own log counts as `cannot_reissue`, never as a known candidate;
- aggregates such as `MAX(hlc)` ignore NULL (unchanged);
- the genesis read requires HTJRNL01 (genesis always is).

Apply and materialisation are the only readers that use projection presence. Task 3 owns the inventory, with one line per reader in its report.

### 3. Migration (delta 0005, rewritten)

Recommended: a table rebuild, done the SQLite-documented way, driven from Rust with no placeholder and no exemption.

- **Runner.** `NodeDelta` becomes `{ version, sql, rebuild: Option<Rebuild> }`, where `Rebuild { copy: fn(&Connection) -> Result<(), ApiError>, sql_after: &'static str }`. For a rebuild delta the runner sets `PRAGMA foreign_keys=OFF` *before* `BEGIN IMMEDIATE`, then runs `sql` (creates `journal_new`, `journal_forked_new`), `copy` (row copy only, no DDL), then `sql_after` (DROP old tables, RENAME new ones, CREATE the indexes and the full trigger set). It then queries `PRAGMA foreign_key_check` from Rust and aborts unless it returns zero rows, commits, and restores `foreign_keys` on every path. This fixes the deferred-counter COMMIT failure: with foreign keys on inside the transaction, the DROP of a referenced parent fails at COMMIT even though `foreign_key_check` is empty.
- **Schema audit.** Every DDL statement stays in replayable SQL. The open-time expected-object replay (`expected_objects`) runs `sql` and then `sql_after` for a rebuild delta. `copy` is a no-op on the empty in-memory database, so the replay performs the same create→drop→rename sequence, and `verify_at` sees identical `sqlite_master` text, including SQLite's quoting of a renamed table (`CREATE TABLE "journal"(…)`). Nobody hand-writes the post-rename DDL. A test runs `verify_at` after migrating both a fresh store and a populated schema-4 store.
- **Steps (row copy).** `copy` reads every old row, builds `UnsignedEnvelope` from the projection plus the log's node and incarnation, computes `raw = canonical_bytes()`, checks `SHA-256(raw) == hash` (abort with `StoreCorrupt` otherwise) and inserts the complete row into the new table. No placeholder ever exists, so the `length(raw) >= 68` CHECK holds row by row and nothing ever UPDATEs `raw`.
- **Index set after migration** (recreated by `sql_after`, since DROP TABLE removes them): `journal_applied_seq` (UNIQUE, partial), `journal_unapplied`, `journal_creates`, `journal_parked_on`, exactly as in the baseline. `journal_forked`'s uniqueness stays a table constraint. A test compares the post-migration index list against this list.
- **Trigger set after migration**, asserted by a test that compares `sqlite_master`:
  - `journal_envelope_immutable`: `log_ix, counter, prev_hash, hash, signature, raw, ingested_at` are never changed; projection columns may change only when every OLD projection column is NULL (promotion);
  - `journal_append_only`;
  - `journal_applied_terminal`;
  - `journal_forked_immutable` (unchanged: no UPDATE at all);
  - `journal_forked_append_only`.
- **Failure.** Any mismatch or error rolls back, leaving the store at schema 4.
- **No exemption marker.** No trigger text carries a migration bypass, because `raw` is written only by INSERT.

`journal_forked` rows are never promoted. Readers treat a NULL projection there as opaque fork evidence, and promotion touches only `journal`.

The 2.3.0 delta (`journal_opaque`) is replaced wholesale, keeping the number 0005, since nothing released carries it.

Discarded:
- Defer-foreign-keys inside the transaction (it fails at COMMIT on any store with `thread_registers`/`sync_reissues` rows).
- A placeholder plus a raw UPDATE through a marker (it violates the CHECK, and the marker would become a permanent bypass in audited trigger text).
- Pure-SQL canonical encoding.
- Lazy backfill on read.

### 4. Reads, serving and relay

Recommended: one row decoder (`journal::entries_from` and its siblings) yields `StoredEntry`, classified by `raw`'s magic, not by projection presence (Key decision 3). A promoted foreign-layout row stays `Opaque` for every chain, verification and serving path, while apply and materialisation read its projection. `CHAIN_ROWS` and every union is deleted, and the readers go back to plain `journal`/`journal_forked` queries. Unchanged in behaviour and tests:
- serving (`WireItem::Known` from the projection, byte-identical to today; `WireItem::Opaque` from `raw`, only toward `envelope.opaque.v1`);
- the capability gate, the withheld-log parking and `withheld_logs`;
- the oversize path (OPAQUE_MAX_BYTES applies to unknown-layout `raw`);
- fork evidence (forks between known and unknown rows now live in `journal_forked` with nullable projection);
- resync salvage (an old backup has no `raw` column: salvage detects schema 4 and reads the projection, recomputing canonical bytes as the migration does).

Known-layout ingest stores `raw = canonical_bytes()` of the verified envelope.

Discarded: serving known entries from `raw` by re-parsing (no benefit; the projection is authoritative for known layouts, and their bytes are equal by hash).

### 5. Promotion

Recommended: `journal::promote(tx, log, counter)` (journal rows only) parses `raw` with the layouts the binary understands (production: HTJRNL01 only; test-support: an understood-layout registry the synthetic HTJRNL99 builder can register). On success it:
- writes the projection columns (allowed by the trigger only from all-NULL);
- sets `apply_state='pending'` (from `parked_layout`);
- clears `sync_logs.blocked_on` when it reads `opaque_layout`.

All of this happens in one transaction, and the normal apply pass then applies the entry and its successors (ingest_seq is the row's own, so FKs from materialisation work unchanged). Triggers: at store open (after migrations), and in the apply pass when the head row is projection-NULL and parseable. The promoted entry's `raw` and hash are untouched. Serving and verification rule (one place, the row decoder's magic classification): an entry is `Known`, and served as `WireItem::Known`, only for HTJRNL01 `raw`. Any other entry, promoted or not, is `Opaque`: it is verified by `SHA-256(raw) == hash` plus the signature, and served as `WireItem::Opaque` from `raw`, gated by `envelope.opaque.v1`. A promoted non-HTJRNL01 entry is therefore still served opaque and withheld from 2.1.1 peers, consistent with the TRUST-POLICY accepted limit. The §7 promotion test asserts what a capability peer and a non-capability peer receive after promotion.

Failure: a parse error, shape check or trigger refusal during promotion leaves the row `parked_layout` with its log stalled, records a diagnostic (daemon log line plus the sync-status cannot-promote count), and never fails store open or the apply pass.

Discarded: promotion by moving rows between tables (the 2.3.0 dead end).

### 6. Kept from 2.3.0 unchanged

The following stay as they are: the capability `envelope.opaque.v1`, the WireItem decode, EntryPrefix and OpaqueEntry verification, golden fixtures and `scripts/freeze-peer-wire`, the 2.1.1 shadow decoder, `scripts/check-wire-compat` and its baseline, the CI tag fetch with `HT_REQUIRE_TAGS`, the prefix/envelope=1/genesis pins, the security fix for authenticated oversize claims, TRUST-POLICY's accepted-limit text (wording adjusted: entries are stored in the journal itself), and the docs (operations: "opaque positions stay stalled until promoted by an upgrade that understands their layout").

### 7. Tests

Required:
- A schema-4 store with representative rows migrates. The rows cover every builtin fact kind, a forked pair, applied and parked rows, and populated `thread_registers` and `sync_reissues` child rows, so the FK-off runner is exercised. After migration:
  - every row's `raw` equals its canonical bytes and `SHA-256(raw) == hash`;
  - serving every log produces frames byte-identical to those served from the schema-4 store before migration (captured before upgrading);
  - `PRAGMA foreign_key_check` is empty;
  - `verify_at` passes on reopen.
- A fresh store reaches schema 5 and passes `verify_at`.
- The `sqlite_master` trigger and index sets after migration equal the lists in §3.
- The migration fixture includes a maximal-size HTJRNL01 row (PAYLOAD_MAX payload, DEPS_MAX deps, NEEDS_MAX needs) that copies under RAW_MAX, plus a known-layout ingest of the same entry after migration.
- A promoted HTJRNL99 row in a frozen own log counts as `cannot_reissue`, and the re-issue pass does not fail.
- A corrupted projection makes the migration abort with the store left at schema 4.
- An unknown-magic (HTJRNL99) entry is stored with a NULL projection and `parked_layout`, relayed byte-exactly to a capability peer, withheld from a non-capability peer, and stalls its log.
- After the test-hook upgrade, the entry is promoted: projection filled, raw/hash unchanged, the stalled successors apply, and `blocked_on` clears. It still decodes as `Opaque` and verifies by raw hash and signature. A capability peer still receives it as `WireItem::Opaque`, and a non-capability peer still does not receive it.
- A malformed body under a registered magic leaves the row `parked_layout`; store open and the apply pass succeed, and a diagnostic is recorded.
- The trigger refuses any projection change on a row whose projection is already filled, and any raw/hash change.
- Existing 2.3.0 suites (`opaque_relay`, `peer_wire`, `legacy_wire`, `journal_port_contract`, resync, incarnations) stay green.

### 8. Rollout

Ships as 2.3.0 (minor: no wire change, a local schema delta, downgrade refused as today). ht-evlm closes as superseded. The 2.3.0 branch's `journal_opaque` design notes are superseded by this spec, recorded in that spec's Post-Implementation Notes.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*

### 2026-10-19: integration sweep (ht-76u6.8)

Swept the integrated tree (tasks 1, 2, 4, 5, 7, 9, 10 plus the task 6 relay and promotion tests) against the goal. No gap needed an inline fix.

- Static checks: migrations `0001`-`0004` are untouched. `journal_opaque`, `CHAIN_ROWS`, `BRANCH_ROWS` and `insert_opaque` no longer appear in `src/` or `migrations/`. No peer wire fixture was added or rewritten by this epic (`tests/fixtures/peer_wire/2.0.4` and `2.1.1` come from ht-j87z.6).
- Gates green: `cargo clippy --locked --all-targets --all-features -- -D warnings`, `check-harness-neutral`, `check-wire-compat` (0 violations, 0 stale), `check-default-features`, `cargo fmt --check`.
- Full `cargo nextest run --locked --all-targets --all-features`: 7856 passed, 0 failed. This covers the migration round-trip (fresh and populated schema 4 to 5, corrupted projection and corrupted forked row abort, maximal entry, trigger and index set), the relay and promotion suites (`opaque_relay`, `opaque_promotion`), wire byte-identity and `peer_wire_fixtures`, `legacy_wire_v211` and `journal_port_contract`. The leak checks found no leaked processes or temp files.
- Divergences from the design as written: delta 0005 ships as two SQL files (`0005_bytes_first_journal.sql` and `0005_bytes_first_journal_after.sql`) around the Rust copy step; the `apply_state` CHECK admits `parked_layout`, as design roast r1 requires. The task 2 bridge definitions of `CHAIN_ROWS` and `BRANCH_ROWS` were removed again by the reader audit (task 10).
- Timing: the run took 371 s, over the 300 s budget, because `package manifest::failpoints_are_compiled_out_of_release_builds` did a cold release build (335 s) in this fresh worktree. It does not depend on this change.
