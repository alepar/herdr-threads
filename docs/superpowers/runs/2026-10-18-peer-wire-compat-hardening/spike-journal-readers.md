# Spike: journal row readers and the opaque-entry seam

Base SHA: `01c324b11b9dbe1f44589e0b1ea192365a667fb9` (all `file:line` references are at this SHA). Task ht-j87z.1; design `2026-10-18-peer-wire-compat-hardening-design.md` section 4 and Key abstractions. Read-only investigation: no source file changed.

## Go/no-go

single seam: no
union reader function needed: yes (expected)
refactor needed: yes

Meaning. The `journal` table is read by SQL in about 20 places across `src/store/{journal,apply,incarnations,sync_status}.rs`, and entries are consumed as `Envelope` values in about 30 places across `src/sync/**`. Nothing funnels them: `RawEnvelope::read` is the only row-to-`Envelope` constructor (4 call sites, all in `src/store/journal.rs`), but chain-position questions (hash at a counter, max counter, predecessor hash, branch walk, "is this (counter,hash) held") are answered by hand-written SQL in five files. A `journal_opaque` table therefore cannot be added behind one reader; every chain-defining reader below must read the union of `journal` and `journal_opaque`, which is a semantic change (design section 4), not a move-only refactor.

Readers ht-j87z.11 must port to the one union reader function (the "Chain-row reader"), in the order a port should take them (each step keeps the suite green because with an empty `journal_opaque` the union equals `journal`):

1. `hash_at` (`src/store/journal.rs:899`) and `envelope_at` (`:914`): the leaf lookups; `stored_hash` (`:979`), ingest's duplicate check (`:1357`) and the `Conflict` held side (`:1411`) sit on them.
2. `serve` (`:997`) and `batch_journal_range` (`:1077`): the main-chain range read; returns `StoredEntry`.
3. `serve_branch_within` (`:1138`): the `forked_prev` walk (`:1158`) and the branch row read (`:1200`); needs `journal_opaque.branch` rows beside `journal_forked`.
4. `verify_invariants` (`:199`): contiguous max (`:204`) and head hash at contiguous (`:205`).
5. `next_ready_head` (`src/store/apply.rs:863`): the apply head scan (inner JOIN at `:873`); this is where a log stops with `blocked_on = opaque_layout`.
6. `sync_status::verify` J2 prev-link check (`src/store/sync_status.rs:314`).
7. Incarnations/fork repair: `record_fork` held check (`src/store/incarnations.rs:103`), `forked_ready` (`:374`), `upsert_branch_tip` (`:999`), `chains_on_known` (`:1227`), `pending_reissues` (`:1548`), `CANDIDATES` (`:239`/`:244`), `check_invariants` I2 (`:1898`).
8. `MemoryJournal` (`src/sync/memory_journal.rs`): same surface over `Vec<StoredEntry>`; its contract suite `tests/sync/journal_port_contract.rs` is the equivalence proof.

Move-only preparation (optional, recommended, no behaviour change; can be the first commit of ht-j87z.11): (a) move the three hand-written chain-position queries outside `journal.rs` into `journal.rs` helpers next to `hash_at`: `record_fork` (`incarnations.rs:103`), `forked_ready` (`:374`), `chains_on_known` (`:1227`) all ask "is (log_ix, counter, hash) held in journal or journal_forked", one helper `held_at(conn, log_ix, counter, hash, include_branches)`; (b) route `store/incarnations.rs::entry_at` (`:632`) through the same helper instead of `journal::serve(limit 1)`. Nothing else needs a pre-move: `RawEnvelope` is already private to `journal.rs`, and the apply head scan can take the union as a SQL `UNION ALL` subquery in place.

Two verified deviations from the roast r1 expectation list: `replay` (`apply.rs:1012`/`replay_candidates:1039`) is known-only (it selects by `apply_state IN ('parked_unknown','parked_dep')`, which an opaque position never has; a promoted entry re-enters as a `pending` `journal` row and is found by `next_ready_head`), and the two apply-time SQL sites on `creates`/`needs` are known-only. All other expected members (verify_invariants, hash_at, next_ready_head, sync_status, incarnations/resync, serve/serve_branch) are union-required. `resync.rs::salvage` reads through `journal::vector` and `journal::serve`, so it is union-required transitively (it copies a backup's entries into a new store; with opaque entries it must carry them across, or it silently truncates a log at the first opaque position).

Facts that the later tasks must know (found while reading):

- `next_ready_head` joins `journal` on `counter = applied_through + 1` with an inner JOIN (`apply.rs:873`). An opaque next position yields no row, so the log is skipped silently: `blocked_on` is never written. The `opaque_layout` marker therefore needs a second statement over `journal_opaque`; it does not fall out of the existing scan.
- The scan orders by `j.ingest_seq` (`apply.rs:874`); `journal_opaque` rows have no `ingest_seq` in the design's column list. Opaque heads are not candidates for apply, so ordering is unaffected; the marker pass needs no order.
- `verify_invariants` compares `contiguous_counter` to `MAX(counter)` over `journal` only (`journal.rs:204`, `:227-231`). With an opaque head it reports `StoreCorrupt "journal invariant: ... contiguous_counter ... != stored max"` and the daemon refuses to start, so the union is a startup-safety requirement, not a nicety.
- `sync_status::verify` J2 (`sync_status.rs:314`) LEFT JOINs the predecessor in `journal`: an opaque predecessor would be reported as a false J2 violation, and an opaque `j` is never examined.
- `journal.rs:312` (`open`, HLC resume) and `incarnations.rs:947` (`restore_chain`) read `hlc` of rows at/after the head; an opaque row has no readable `hlc`. Both stay known-only (the join simply yields no row for an opaque head) and that is acceptable, but a test should pin it.
- `held_genesis_pubkey` (`coordinator.rs:2452`) serves counter 1 and parses its payload; counter 1 is always HTJRNL01 (design 1b), so it matches `StoredEntry::Known` and treats `Opaque` as unreachable-corrupt.
- All journal tables kick lanes by name: `APPLY_TABLES = ["journal"]`, `SYNC_TABLES = ["journal","sync_logs","sync_dial_targets"]` (`src/service/kicks.rs:119`, `:123`), and `journal_forked` is listed as a passive table (`:237`). `journal_opaque` writes always also update `sync_logs` (contiguous head), which already kicks the sync lane; the apply lane, however, is kicked only by `journal`, so a promotion (task 13) that inserts into `journal` kicks it, and an opaque insert correctly does not. The kick registry still needs a `journal_opaque` entry (passive) so the table-name coverage test does not fail.
- Own logs: ingest of an own-log entry that the node does not understand (restore from a future version of itself) goes through `AheadOfSelf`/`restore_rotate` (`incarnations.rs:668`), `Job::Restore { entry: Box<Envelope> }` (`sink.rs:72`, `worker.rs:832`). An opaque own-log entry has to be reported "cannot re-issue" there, never replayed.

## Readers

Rows read from `journal` or `journal_forked`. "Env?" is whether the reader builds an `Envelope` from a row (`RawEnvelope::read` then `decode`). `U` is union-required (defines or walks chain positions, so a `journal_opaque` row must be visible), `K` is known-only (needs kind, payload, deps, creates, needs or apply state of a known entry, so an opaque row is correctly invisible).

| file:line | function (header line) | columns read | Env? | class | reason |
|---|---|---|---|---|---|
| `src/store/journal.rs:204-205` | `verify_invariants` (199) | `MAX(counter)`, `hash` at `contiguous_counter` | no | U | the startup invariant is defined over chain positions; an opaque head fails it today (see facts) |
| `src/store/journal.rs:904` | `hash_at` (899) | `hash` | no | U | leaf of every continuity check (`stored_hash`, ingest duplicate check, branch walk) |
| `src/store/journal.rs:922` | `envelope_at` (914) | `ENVELOPE_COLUMNS` | yes (`RawEnvelope::read` 924) | U | the held side of a fork `Conflict` (`:1411`) may be opaque; must return `StoredEntry` |
| `src/store/journal.rs:1012` | `serve` (997) | `ENVELOPE_COLUMNS` | yes (1019) | U | serves chain positions in counter order to peers, `entry_at` and salvage |
| `src/store/journal.rs:1087` | `batch_journal_range` (1077) | `ENVELOPE_COLUMNS` | yes (1098) | U | main-chain part of a served branch |
| `src/store/journal.rs:1160` | `serve_branch_within` closure `forked_prev` (1158) | `prev_hash` of `journal_forked` | no | U | walks a fork branch by prev link; an opaque branch row lives in `journal_opaque` |
| `src/store/journal.rs:1203` | `serve_branch_within` (1138) | `ENVELOPE_COLUMNS` of `journal_forked` | yes (1211) | U | branch rows served verbatim |
| `src/store/journal.rs:251` | `report` (247) | `COUNT(*)` by `apply_state` | no | K | counts parked/failed states; opaque rows have no apply state |
| `src/store/journal.rs:312` | `open` (277) | `MAX(hlc)` at head of self logs | no | K | HLC resume; an opaque head has no readable hlc (see facts) |
| `src/store/apply.rs:217`, `:219` | `delivery_suppressed` (211) | `creates` of `journal`, `journal_forked` | no | K | settlement keys exist only on known kinds |
| `src/store/apply.rs:382` | `load_entry` (361) | `log_ix,counter,hlc,kind,kind_version,creates,needs,payload` by `ingest_seq` | no (own `EntryRow`) | K | builds a handler input; opaque is never applied |
| `src/store/apply.rs:583`, `:593` | `apply_entry` (554) | `creates`, `apply_state` | no | K | `needs` resolution |
| `src/store/apply.rs:873` | `next_ready_head` (863) | `log_ix,node,inc,blocked_on,ingest_seq,deps` at `applied_through+1` | no | U | the head scan; opaque at the next position must stop the log (see facts) |
| `src/store/apply.rs:933` | `PARKED_READY` (933) | `creates`, `apply_state` | no | K | parked-dependency readiness |
| `src/store/apply.rs:941` | `next_reattemptable` (938) | `ingest_seq` | no | K | parked-dependency re-attempt |
| `src/store/apply.rs:1059` | `replay_candidates` (1039) | `ingest_seq,apply_state,kind,kind_version` | no | K | replay selects parked states only (deviation from the roast list, see above) |
| `src/store/sync_status.rs:82` | `per_log` (65) | `apply_state` counts | no | K | parked/failed counts per log |
| `src/store/sync_status.rs:180` | `unapplied_page` (170) | `ingest_seq,counter,kind,apply_state,parked_on,apply_error` | no | K | lists unapplied known entries |
| `src/store/sync_status.rs:228` | `unapplied_page` (170) | `COUNT(*)` by state | no | K | same |
| `src/store/sync_status.rs:314-316` | `verify` (288) | `counter`, `prev_hash`, predecessor `hash` | no | U | J2 prev-link continuity check over chain positions |
| `src/store/incarnations.rs:103` | `record_fork` (81) | existence of `(log_ix,counter,hash)` | no | U | decides which fork sides are missing from the main chain |
| `src/store/incarnations.rs:239`, `:244` | `CANDIDATES` (235), used by `reissue_pass_ungated` (277) | `counter,hash,prev_hash,kind` of frozen own logs, `journal` and `journal_forked` | no | U | the re-issue work list; an opaque tail is "cannot re-issue, report", not silently absent |
| `src/store/incarnations.rs:374-375` | `forked_ready` (372) | existence of predecessor `(counter-1, prev_hash)` | no | U | prev-link continuity of a forked candidate |
| `src/store/incarnations.rs:837` | `own_geneses` (833) | `incarnation,hlc,payload` of counter 1 | no | K | counter 1 is always HTJRNL01 (design 1b) |
| `src/store/incarnations.rs:947` | `restore_chain` (894) | `MAX(hlc)` of the active log | no | K | lineage rule over hlc; opaque has none (see facts) |
| `src/store/incarnations.rs:999` | `upsert_branch_tip` (972) | existence in `journal_forked` | no | U | "is this branch tip already held"; an opaque branch tip lives in `journal_opaque` |
| `src/store/incarnations.rs:1227-1228` | `chains_on_known` (1217) | existence of `(counter-1, hash)` in `journal` or `journal_forked` | no | U | first entry of a pulled branch must chain on a known predecessor |
| `src/store/incarnations.rs:1548`, `:1551` | `pending_reissues` (1544) | `COUNT(*)` of candidates | no | U | status count of the `CANDIDATES` rule; must include opaque as "unreissuable" |
| `src/store/incarnations.rs:1836` | `record_own_unknown_newer` (1815) | `hlc` of counter 1 | no | K | genesis only |
| `src/store/incarnations.rs:1898-1899` | `check_invariants` I2 (1893) | `journal_forked` rows at or below freeze | no | U | invariant over fork-branch positions; an opaque branch row must obey I2 too |
| `src/store/incarnations.rs:1927`, `:1929`, `:1931` | `check_invariants` I3 (1893) | `kind` of re-issue sources/targets | no | K | I3 is "no genesis is re-issued"; an opaque entry is never counter 1 |
| `src/store/materialize.rs:187` | `journaled_route` (169) | existence by `creates` | no | K | creating-fact test |
| `src/store/control/thread.rs:488` | `has_creating_fact` (486) | existence by `creates` | no | K | same |
| `src/test_support/multinode.rs:1215` | `kinds` helper | `kind` counts | no | K | test helper over kinds |
| `src/test_support/multinode.rs:1234` | `journal_entry` helper | `kind,hash,apply_state` | no | K | test helper; may gain an opaque twin later |
| `src/test_support/journal_oracle.rs:602` | `origin_of` | `node,inc,counter` by `ingest_seq` | no | K | maps an applied row back to its position |
| `src/test_support/journal_oracle.rs:633` | `positions` | `counter, applied_seq` | no | K | applied-position oracle |

Scalar readers of `sync_logs` that answer chain-position questions without reading journal rows (not affected by `journal_opaque`; `contiguous_counter` and `head_hash` already describe the union if ingest maintains them for opaque appends):

| file:line | function | what it reads |
|---|---|---|
| `src/store/journal.rs:106` | `active_self_log` (103) | `contiguous_counter`, `head_hash` of the active self log |
| `src/store/journal.rs:146` | `check_self_applied` (143) | `applied_through = contiguous_counter` of the active self log |
| `src/store/journal.rs:166` | `applied_vector` (163) | `applied_through` per log |
| `src/store/journal.rs:861` | `load_log` (857) | `pubkey, contiguous_counter, head_hash, state, frozen_at_counter` (feeds `HeldLog`) |
| `src/store/journal.rs:937` | `vector` (934) | `contiguous_counter, head_hash, state, frozen_at_counter` (the have-heads source) |
| `src/store/sync_status.rs:103` | `per_log` (65) | `contiguous_counter, head_hash, applied_through, frozen_at_counter` |
| `src/store/apply.rs:838` | `unmet_dep` (828) | `applied_through, frozen_at_counter` for a dependency's log |
| `src/store/incarnations.rs:602` | `active_self` (599) | `contiguous_counter, head_hash` |
| `src/store/incarnations.rs:980` | `upsert_branch_tip` (972) | `frozen_at_counter` |
| `src/test_support/multinode.rs:1086` | `vector` helper | `contiguous_counter, applied_through` |

## Writers

| file:line | function | write | notes for opaque |
|---|---|---|---|
| `src/store/journal.rs:493` | `insert_pending` (480) | `INSERT INTO journal` (known entry, `pending`) | callers: `begin_incarnation_with` (own genesis), `author_encoded` (own facts), `reissue` (own re-issue), ingest `Genesis`/`Append` (`:1443`). An opaque append gets a sibling insert into `journal_opaque`; the cross-table guard keeps the position unique |
| `src/store/journal.rs:455` | `begin_incarnation_with` (380) | `INSERT INTO sync_logs` | own genesis only |
| `src/store/journal.rs:1518` | `insert_log` (1506) | `INSERT INTO sync_logs` (empty row for a verified foreign genesis) | genesis stays HTJRNL01 |
| `src/store/journal.rs:657`, `:729` | `author_encoded` (605), `reissue` (691) | `UPDATE sync_logs SET contiguous_counter, head_hash` | own-log appends, known only |
| `src/store/journal.rs:1461` | `ingest_batch` (1333) | `UPDATE sync_logs SET contiguous_counter, head_hash` after appends | the one place a batch with opaque entries moves the head |
| `src/store/journal.rs:1595` | `store_forked` (1569) | `INSERT OR IGNORE INTO journal_forked` | opaque branch entries go to `journal_opaque` with `branch = 1` |
| `src/store/journal.rs:1645` | `freeze` (1625) | `UPDATE sync_logs SET state='frozen', frozen_at_counter` | scalar only |
| `src/store/apply.rs:451` | `record_unapplied` (440) | `UPDATE journal SET apply_state, parked_on, apply_error` | known only |
| `src/store/apply.rs:461` | `advance_head` (458) | `UPDATE sync_logs SET applied_through, blocked_on=NULL` | the opaque stall writes `blocked_on='opaque_layout'` through a new statement |
| `src/store/apply.rs:628` | `apply_entry` (554) | `UPDATE journal SET apply_state='applied', applied_seq` | known only |
| `src/store/apply.rs:919` | `next_ready_head` (863) | writes `blocked_on` for unmet deps | gains the `opaque_layout` case |
| `src/store/incarnations.rs:121`, `:1423` | `record_fork` (81), `adopt_declared_freeze` (1407) | `INSERT OR IGNORE INTO sync_forks` | stores hashes and counters only; opaque proofs fit as is |
| `src/store/incarnations.rs:346-355` | `reissue_pass_ungated` (277) | `INSERT INTO sync_reissues` after `journal::reissue` | opaque candidates must be skipped and reported, never reissued |

## Fork-freeze

How a conflicting entry at an existing counter, or a broken prev link, is detected and recorded, and which row fields it reads.

- Detection is `chain::check_next` (`src/sync/chain.rs:66`), a pure function of a `HeldLog` value (`:9-24`) and the incoming `Envelope`. It reads, from the entry: `counter`, `hash`, `prev_hash`, `verify(pubkey)` (hash recomputation plus signature) and, for a genesis only, `genesis_pubkey()`. It reads, from the store, only scalars: `contiguous`, `head_hash`, `state`, `frozen_at`, `is_active_self` (all from `sync_logs`, loaded by `load_log`, `journal.rs:857`) and `stored_hash`, the hash at the entry's counter if `counter <= contiguous` (filled by `hash_at`, `journal.rs:1357`).
- Verdicts (`chain.rs:34-53`): `Duplicate` (same hash held), `Conflict` (different validly signed entry at a held counter `:92`, or a `prev_hash` that does not equal `head_hash` at `contiguous + 1` `:106`), `BeyondFrozen` (`:94-98`), `AheadOfSelf` (own active log, `:79-86`), `Gap`, `Genesis`, `Append`, `Corrupt`. A mismatched hash or signature is `Corrupt`, never a fork (`signed_conflict`, `:112`).
- Recording is in two steps. Ingest (`journal.rs:1399-1416`) returns `IngestOutcome::Fork { evidence: Conflict { held, conflicting } }` and stores nothing for the conflict; the held side is read back in full by `envelope_at` (`journal.rs:922`). Then `sink.on_fork` (`sink.rs:529`) and the incarnations worker `handle_fork`/`act_on_pair` (`worker.rs:759`, `:776`) classify the pair with `classify_pair` (`src/sync/incarnations.rs:111`) into a `Proof { log, x, held, conflicting }` and call, in one store transaction, `record_fork` (`src/store/incarnations.rs:81`): `journal::freeze(tx, log, x-1)` (`journal.rs:1625`, updates `sync_logs.state='frozen'`, `frozen_at_counter`), then `store_forked` (`journal.rs:1569`, inserts the sides not already in `journal` into `journal_forked`), then `INSERT OR IGNORE INTO sync_forks`. Later branch pulls (`branch_progress`, `incarnations.rs:1251`) freeze lower-only at the divergence (`:1336`) and store off-chain entries.
- Freeze state is entirely in `sync_logs.state` / `frozen_at_counter`; `chain.rs:94`, `journal.rs:1395`, `apply.rs:838`/`:846` (`unmet_dep` caps a dependency at the freeze) read it as a scalar.
- Fields read from rows: `counter`, `hash`, `prev_hash` (and `signature` through `verify`), all of which are the 68-byte prefix plus hash plus signature. Nothing in fork detection reads `kind`, `payload` or `deps`, except `reissue`/`CANDIDATES` (re-issue after a fork), which needs kind, payload, deps, creates, needs and is known-only. So fork detection and freeze work for opaque entries once `check_next` takes `&StoredEntry`; only the held-side read (`envelope_at`), `store_forked` and the proof types (`Proof.held/conflicting`, `ForkEvidence::{Conflict,AheadOfSelf}`) carry a full entry.

## Have-heads

How the `have` frame's per-log `counter` and `head` are produced, and on which path.

- Source: `JournalPort::vector` (`journal_port.rs:117`) over `SqliteStore` = `store::journal::vector` (`journal.rs:934`): one `SELECT node_uuid,incarnation,contiguous_counter,head_hash,state,frozen_at_counter FROM sync_logs` (`:937`). No `journal` row is read; the head hash is the stored `sync_logs.head_hash`, which ingest maintains (`journal.rs:1461`), `author_encoded` (`:657`) and `reissue` (`:729`). `MemoryJournal::vector` (`memory_journal.rs:245`) computes it from the entries (`head_of`, `:220`: `entries.len()` and `entries.last().hash`).
- Coordinator path: `Actor::request_local` (`coordinator.rs:1620`) runs `journal.vector` on a blocking task; `on_local` (`:1639`) stores it in `self.local` and answers owed `have`s (`send_have`); `flush_have_deltas` (`serve.rs:264`) diffs `self.local` against `last_have_sent` at most once a second per link; `have_log` (`serve.rs:311`) maps a `LogHead` to the wire `HaveLog { node, inc, counter, head: hex, frozen_at }`.
- Peer side: `have_head` (`coordinator.rs:2379`) decodes a received row to a `LogHead`; the incarnations worker `handle_have` (`worker.rs:894`) compares it with `log_meta` (`:921`) and `stored_hash` (`:932`) to detect a tip it does not hold.
- Consequence for opaque entries: have-heads need no row reads, so a stored opaque entry counts toward the head exactly as design 2 requires provided ingest advances `sync_logs.contiguous_counter` and `head_hash` for opaque appends. The two row-level checks that remain are `verify_invariants` (`journal.rs:204-205`, must read the union, else the startup invariant refuses a store with an opaque head) and `stored_hash` (`hash_at`, must read the union for the `handle_have` comparison at `worker.rs:932` and for the duplicate check at `journal.rs:1357`).

## Prev-link continuity

Where an incoming entry's `prev_hash` is compared with the stored predecessor, for `SqliteStore` and `MemoryJournal`.

- The one statement of the rule is `chain::check_next`, shared by both ports (comment at `chain.rs:61-62`): for an append at `contiguous + 1`, `entry.prev_hash != held.head_hash` is `Conflict` (`chain.rs:106`). The predecessor hash is not read from a row; it is `HeldLog.head_hash`, the cached `sync_logs.head_hash` (`load_log`, `journal.rs:890`; `MemoryJournal` builds the view at `memory_journal.rs:443` and calls `check_next` at `:459`). For counters at or below `contiguous`, the comparison is by hash at that counter: `HeldLog.stored_hash` (SQLite: `hash_at`, `journal.rs:1356-1359`; memory: the vec element at counter-1).
- Branch and repair code compares prev links against rows, in SQL, in five more places: `serve_branch_within` (`journal.rs:1190`, `hash_at(counter-1) == prev` joins a forked branch to the main chain; memory twin `memory_journal.rs:360`), `forked_ready` (`incarnations.rs:372`), `chains_on_known` (`incarnations.rs:1217`), `sync_status::verify` J2 (`sync_status.rs:314-320`, `p.hash <> j.prev_hash`), and `branch_progress`'s `stored_hash` common-prefix loop (`incarnations.rs:1324-1330`). `classify_ahead_of_self` (`src/sync/incarnations.rs:159`) compares `entry.prev_hash` with the head hash passed in.
- Reissue and author compute `prev_hash` from `ActiveLog.head_hash` (`journal.rs:103`), a `sync_logs` scalar.
- For opaque entries: the ingest path needs no change beyond `check_next(&StoredEntry)`; the row-level comparisons in the five places above are the union-required readers listed in "Go/no-go".

## JournalPort surface

Trait `src/sync/journal_port.rs:115`. "Env" is whether the method returns or consumes `Envelope` (and so becomes `StoredEntry`).

| method | trait line | `SqliteStore` impl (`journal.rs`) | `MemoryJournal` impl | Env |
|---|---|---|---|---|
| `vector` | 117 | 1656 -> `vector` 934 | 245 | no |
| `stored_hash` | 120 | 1661 -> `stored_hash` 979 -> `hash_at` 899 | 265 | no (hash only; union internally) |
| `serve` | 128 | 1671 -> `serve` 997 | 280 | returns `Vec<Envelope>` -> `Vec<StoredEntry>` |
| `serve_branch` | 145 | 1682 -> `serve_branch_within` 1138 | 310 | returns `Option<Vec<Envelope>>` -> `Option<Vec<StoredEntry>>` |
| `log_meta` | 159 | 1698 -> `log_meta` 1224 | 392 | no |
| `ingest` | 164 | 1706 -> `ingest` 1301 / `ingest_batch` 1333 | 414 | consumes `Vec<Envelope>` -> `Vec<StoredEntry>`; `IngestOutcome::Fork` carries `ForkEvidence` (`journal_port.rs:48`) -> `StoredEntry` |
| `apply_pending` | 174 | 1716 -> `apply::apply_pass` (`apply.rs:980`) | 559 | no |

Other `JournalPort` implementers that will need the signature change: `ForgedGenesis` (`coordinator.rs:2940`), `VectorStub` (`:3040`), `TestJournal` (`:4509`), `CorruptBranches` (`tests/sync/incarnations_scenarios.rs:916`). Port consumers: `coordinator.rs:1632`, `:2460`, `:2468`; `serve.rs:173`, `:394`; `ingest.rs:137`; `worker.rs:310`, `:711`, `:921`, `:932`, `:1093`, `:1152`, `:1302`; `sink.rs:443`, `:460`; test support `journal_oracle.rs`, `fact_exchange.rs`, `sync_node.rs`. Direct (non-port) users of the store functions: `src/sync/resync.rs:466` (`journal::vector`) and `:488` (`journal::serve`) in `salvage`; `src/store/incarnations.rs:334` (`serve_branch`), `:633` (`serve`), `:698` and `:1325` (`stored_hash`).

## StoredEntry match sites

Every place an `Envelope` read from the journal (or carried by `ForkEvidence`/`Proof`) is consumed. `P` = needs only log, counter, prev_hash, hash, signature, size (one `StoredEntry` method each); `K` = needs kind, payload, deps, hlc, creates or needs (stays known-only, or matches `Known`/`Opaque` explicitly).

Serve path (outbound):

| file:line | what it reads off the entry | class |
|---|---|---|
| `src/sync/coordinator/serve.rs:173` -> `on_served` (186) -> `:223-226` | `WireEnvelope::from(&env)` per entry; this is where `WireItem::{Known,Opaque}` is built | P (wire conversion is a `StoredEntry` method) |
| `src/sync/coordinator/serve.rs:55-76` `encode_halved` | `last.counter`, encoded size via the built frame | P |
| `src/sync/coordinator/serve.rs:229-253` (cursor advance in `on_served`) | `last_counter + 1` from `encode_halved` | P |
| the capability gate (new) | `is_opaque()` per entry, between `journal.serve` result and `encode_halved` | P |
| `src/sync/coordinator/serve.rs:394` -> `on_branch_served` (415) -> `send_branch_entries` (575-581) | `WireEnvelope::from`, `e.counter < want.tip.counter` | P |
| `src/store/journal.rs:1029`, `:1065` (`serve`, `Batch::push`) | `envelope.canonical_bytes().len()` for batch limits | P (size: opaque uses `raw.len()`) |
| `src/sync/memory_journal.rs:299`, `:382` | `canonical_bytes().len()` | P |

Ingest path (inbound):

| file:line | what it reads | class |
|---|---|---|
| `src/sync/coordinator/ingest.rs:173-174` | `Envelope::try_from(WireEnvelope)`; becomes `WireItem` -> `StoredEntry` | P |
| `src/sync/coordinator/ingest.rs:37`, `:125` | `IngestJob.entries: Vec<Envelope>` | P (type only) |
| `src/store/journal.rs:1324` | `entry.log != *log` | P |
| `src/store/journal.rs:1365` -> `chain.rs:66-115` | `counter`, `hash`, `prev_hash`, `verify`, `genesis_pubkey` | P, except `genesis_pubkey` (K, counter 1 only; opaque at counter 1 is `Corrupt(NotGenesis)`) |
| `src/store/journal.rs:1385`, `:1412` | `entry.clone()` into `ForkEvidence` | P |
| `src/store/journal.rs:1420` | `entry.validate_shape()` | K (opaque shape was checked at `OpaqueEntry` construction) |
| `src/store/journal.rs:1428` | `entry.genesis_pubkey()` | K |
| `src/store/journal.rs:1443` -> `insert_pending` (480-515) | all columns incl. kind, deps, payload | K (opaque appends go to `journal_opaque` instead) |
| `src/store/journal.rs:1444-1445`, `:1451` | `counter`, `hash`; `entry.hlc` folded into the clock | P for counter/hash; hlc fold is K (an opaque entry cannot fold the HLC; document and test) |
| `src/sync/memory_journal.rs:414-558` | the same loop over `Vec<StoredEntry>` | mirror |
| `src/sync/resync.rs:443`, `:488-503` (`salvage`) | `batch.last().counter`, `batch.len()`, passes the batch to ingest | P |

Fork and repair path:

| file:line | what it reads | class |
|---|---|---|
| `src/sync/journal_port.rs:48-62` `ForkEvidence` | `entry: Box<Envelope>`, `held`, `conflicting` | P (type changes) |
| `src/sync/incarnations/sink.rs:531-532` `on_fork`, `:438-` `on_ahead_of_self` | `entry.verify(pubkey)` via `classify_ahead_of_self`; `entry.clone()` into `Job::Restore` (`:72`) | P; `Restore` replays the entry through `restore_rotate` and is K |
| `src/sync/incarnations/sink.rs:460` | `journal.serve(...)` one entry, `e.counter` | P |
| `src/sync/incarnations.rs:111` `classify_pair` | `counter`, `hash`, `prev_hash`, `verify(pubkey)`, builds `Proof` | P |
| `src/sync/incarnations.rs:159-190` `classify_ahead_of_self` | `entry.counter`, `.prev_hash`, `.hash`, `mine.log`, `mine.hash`; `Proof.held/conflicting` | P |
| `src/sync/incarnations.rs:66-67` `Proof` | `held: Envelope`, `conflicting: Envelope` | P (type changes) |
| `src/sync/incarnations/worker.rs:759`, `:776` | `held`, `other` passed to `classify_pair` | P |
| `src/sync/incarnations/worker.rs:832-880` `handle_restore`, `after_rotation` | `entry` to `restore_rotate`; `conflicting.hash` | `restore_rotate` is K (replays an own-log entry), `.hash` is P |
| `src/sync/incarnations/worker.rs:1080-1096` `local_entry` | `serve_branch` one entry, `e.counter`, `e.hash` | P |
| `src/sync/incarnations/worker.rs:1102-1165` `resolve_fork` | `<[Envelope;2]>`, `stored_hash` for `e.counter,e.hash`, `(counter,hash)` ordering | P |
| `src/sync/incarnations/worker.rs:1171-1215` `handle_branch`, `:1548` `decode_wire` | wire entries decoded to `Envelope` | P (`WireItem`) |
| `src/sync/incarnations/worker.rs:1294-1305` `local_main_entry` | `serve`, `e.counter` | P |
| `src/sync/incarnations/worker.rs:1307-` `on_pull_reply`, `:1566` `fork_ref` | `e.counter`, `e.hash` | P |
| `src/store/incarnations.rs:632-646` `entry_at` | `serve` limit 1, `entry.counter` | P |
| `src/store/incarnations.rs:116` `record_fork` -> `store_forked` (`journal.rs:1569-1616`) | every column for `journal_forked` | K (opaque goes to `journal_opaque` with `branch = 1`) |
| `src/store/incarnations.rs:1251-1360` `branch_progress`, `verify_reply` | `counter`, `hash`, `prev_hash`, `verify`; `store_forked(off_chain)` | P for the checks, K for the store |
| `src/store/incarnations.rs:334-346` `reissue_pass_ungated` -> `journal::reissue` (`journal.rs:691-752`) | `source.deps`, `.hlc`, `.kind`, `.kind_version`, `.creates`, `.needs`, `.payload`, `.log` | K (opaque source: "cannot re-issue, report") |
| `src/sync/coordinator.rs:2452-2475` `held_genesis_pubkey` | `entry.kind == GENESIS_KIND`, `entry.payload` | K (counter 1, always Known) |

Test consumers that construct or inspect journal `Envelope`s through the port and will need a `StoredEntry` helper: `src/test_support/journal_oracle.rs`, `src/test_support/fact_exchange.rs`, `src/test_support/sync_node.rs`, `src/test_support/multinode.rs`, `tests/sync/journal_port_contract.rs`, `tests/sync/incarnations_scenarios.rs`.

## Excluded grep hits

Hits of the step-1 greps that are not journal-row readers or writers, one line per file with its hit lines. They are either scalar `sync_logs` fields, wire/diagnostic field names, definitions, comments, or code that only moves a value that a reader above produced.

- `src/cli/commands.rs` lines 1333: doc comment of the `sync status` flag; renders scalar `applied_through`, no row read.
- `src/daemon/transport.rs` lines 250: unrelated `fn serve` (daemon socket server).
- `src/daemon/transport/watch_connection.rs` lines 85: unrelated `fn serve` (watch connection).
- `src/protocol/output_compact.rs` lines 1187, 1191, 1236: renders the scalar `applied_through` / `frozen_at` fields of the sync status rows; no row read.
- `src/protocol/sync_admin.rs` lines 144, 148, 184: wire struct fields `applied_through` / `frozen_at` of the status protocol; no row read.
- `src/service/sync_admin.rs` lines 257: copies `repaired.frozen_at` into a reply; no row read.
- `src/service/workers.rs` lines 19, 1287: imports `JournalPort` and a doc comment that the apply worker holds the concrete store; no row read.
- `src/store/apply.rs` lines 16, 18, 266, 270, 272, 827, 846, 847, 874, 971, 1008: doc comments (16, 18, 266-272, 827, 971, 1008), `unmet_dep` arithmetic on scalars already read at 838 (846, 847), and the ORDER BY of the `next_ready_head` statement already listed at 873 (874).
- `src/store/incarnations.rs` lines 64, 136, 138, 145, 240, 665, 765, 811, 965, 978, 987, 1004, 1007, 1096, 1113, 1123, 1155, 1244, 1361, 1465, 1544, 1554, 1566, 1897, 1913, 1919: struct fields, doc comments and arguments of `frozen_at` (64, 665, 765, 811, 965, 1096, 1113, 1244, 1361, 1465); `sync_logs.frozen_at_counter` scalar reads (136, 138, 145, 978, 987, 1004, 1007, 1123, 1155, 1566); the continuation line of the `CANDIDATES` SQL (240); parameters and header of `pending_reissues` (1544, 1554); result mapping of the `check_invariants` I2 query (1897, 1913, 1919).
- `src/store/journal.rs` lines 44, 50, 126, 134, 161, 167, 191, 202, 229, 240, 313, 762, 776, 777, 780, 841, 843, 879, 889, 892, 924, 955, 967, 970, 1019, 1098, 1127, 1211, 1240, 1368, 1369, 1376, 1382, 1390, 1394, 1395, 1396, 1399, 1410, 1417, 1427, 1482, 1538, 1637, 1655, 1671, 1682, 1729: imports (44, 50); `sync_logs` scalar columns and error texts (126, 134, 161, 167, 191, 202, 229, 240, 313 continuation of 312, 955, 967, 970); the `RawEnvelope` definition and `ENVELOPE_COLUMNS` (762, 776, 777, 780) whose callers are listed above, and their call arguments (924, 1019, 1098, 1211); `LogRow` / `load_log` decoding (841, 843, 879, 889, 892); `serve_branch` and `log_meta` bodies already covered by their inner reads (1127, 1240); the `ChainStep` match arms inside `ingest_batch`, covered under Fork-freeze (1368, 1369, 1376, 1382, 1390, 1394-1396, 1399, 1410, 1417, 1427); `LogHead` constructors (1482, 1538); `freeze` and the `JournalPort` impl header lines, covered by the surface table (1637, 1655, 1671, 1682, 1729).
- `src/store/sync_status.rs` lines 7, 104, 151, 155: module doc (7) and `per_log` mapping of `sync_logs` scalars (104, 151, 155); no row read.
- `src/sync/chain.rs` lines 7, 32, 69, 75, 76, 90, 100, 105, 107, 114, 115, 200, 213, 216, 225, 235, 236, 241, 242, 254, 255, 266, 274, 277, 278, 285, 295, 296, 305, 306, 316, 317, 326, 336, 346, 347, 356, 358, 361, 365, 373, 381, 390, 391, 402, 403: pure `check_next` (no store access; its entry reads are listed under Fork-freeze) and its unit tests (200-403).
- `src/sync/coordinator.rs` lines 14, 50, 76, 195, 314, 457, 2380, 2381, 2386, 2391, 2407, 2548, 2645, 2649, 2655, 2940, 2949, 2963, 2972, 3040, 3055, 3064, 3158, 3306, 3312, 3372, 4509, 4521, 4537, 5095, 5096, 5276, 5327, 5354: actor wiring and module docs (14, 50, 76, 457), the `ForkSink::on_fork` signature and its no-op impl, whose `ForkEvidence` parameter changes type but reads no entry (195, 314), `have_head` mapping (2380-2391), `admit` signature (2407), and test doubles / fixtures that implement or call `JournalPort` (2548-5354); production row reads of the coordinator are only 2460 and 2468, listed above.
- `src/sync/coordinator/ingest.rs` lines 12, 53, 118, 120: module doc and `ingest_batch` header (12, 53, 118, 120); the port call is 137, listed above.
- `src/sync/coordinator/routing.rs` lines 99, 207, 240, 247: reads `LogHead.frozen_at` from the cached local vector to decide what is wantable (99, 207, 240, 247); no row read.
- `src/sync/coordinator/serve.rs` lines 7, 13, 317: module docs (7, 13) and `have_log` mapping of `LogHead.frozen_at` (317); no row read.
- `src/sync/facts/node_genesis.rs` lines 10, 29, 35, 116: `frozen_at` is a field of the `node.genesis` fact payload (a kind-level fact), not a journal column.
- `src/sync/frame.rs` lines 99, 123, 130, 439, 446, 702, 773: wire frame fields `HaveLog.frozen_at` / `ForkRef.frozen_at` and their tests; no row read.
- `src/sync/incarnations/sink.rs` lines 17, 179, 189, 201, 529, 715, 716, 722, 796, 847, 853, 893, 964, 1010, 1032, 1037, 1038, 1039: imports, module docs, `ForkSink` struct (17, 179, 189, 201), `on_fork` match whose entry use is listed under StoredEntry match sites (529), and unit-test builders of `ForkEvidence` (715-1039).
- `src/sync/incarnations/worker.rs` lines 14, 15, 57, 97, 703, 809, 1576: imports and module docs (14, 15, 57, 97), `ingest` comment (703), `fork_ref` use (809, 1576); the port calls are listed above.
- `src/sync/journal_port.rs` lines 31, 34, 83, 94, 107, 128, 145: `IngestOutcome::Frozen` and `LogMeta.frozen_at` definitions and trait doc lines (31, 34, 83, 94, 107); trait methods 128 and 145 are in the surface table.
- `src/sync/memory_journal.rs` lines 1, 8, 12, 31, 36, 38, 44, 220, 225, 244, 280, 310, 398, 403, 408: header docs and imports (1-44), `head_of` (220, 225), trait impl and method headers listed in the surface table (244, 280, 310), `log_meta` freeze mapping (398-408).
- `src/sync/mod.rs` lines 72, 75: `LogHead.frozen_at` field and doc (72, 75).
- `src/sync/resync.rs` lines 15, 441, 1034, 1090: doc comments and test helper (15, 441, 1034, 1090); the production reads are 466 and 488, listed above.
- `src/sync/sync_store.rs` lines 9: module doc mentioning `JournalPort`; no row read.
- `src/test_support/counting_client.rs` lines 149: `fn serve` of a scripted client backend (serves a CLI `Command`, unrelated to the journal).
- `src/test_support/fact_exchange.rs` lines 5, 20: doc and import of `JournalPort`; goes through the port.
- `src/test_support/journal_oracle.rs` lines 14, 52, 319: doc, import and `JournalPort` trait-object use (14, 52, 319); goes through the port.
- `src/test_support/multinode.rs` lines 1079: doc of the `vector` helper (1079); its scalar read is listed above.
- `src/test_support/sync_node.rs` lines 50, 113, 122: import and `Arc<dyn JournalPort>` construction (50, 113, 122); goes through the port.
