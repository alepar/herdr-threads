# 08 — Migration and backfill of existing data; ID aliasing

Research question 8 of the vision spec (`docs/superpowers/specs/2026-10-13-cross-node-sync-vision-design.md`). Standard-mode deep research, 2026-10-13.

## Recommendations

1. **Do not rewrite any existing public ID, seats included.** Keep every pre-link ID verbatim as a *legacy ID*. Its global key is `(origin_node, legacy_id)`, and the backfill entry that imports it binds it to an origin. This drops the spec's "re-key seats, keep old IDs as aliases" plan. The repo already follows this policy: `src/protocol/ids.rs` says "Persisted IDs with older long prefixes stay opaque and are never rewritten", and the live store still holds 53 `seat-…`, 411 `ev-…`, 116 `msg-…`, 29 `inv-…` and 7 `thr-…` long-form IDs beside the 9-character compact ones. A seat's home node is a field of its seat-existence fact, so routing never needs to parse it out of the ID.
2. **New IDs carry the node segment** (`<type><node><8 base62>`). They are unique by construction and can be told apart from legacy IDs by length or shape: compact legacy IDs are 9 characters, long legacy IDs contain `-`.
3. **Replicated payloads always name legacy entities origin-qualified.** Agents see the bare legacy ID whenever it is unambiguous. If a collision shows up on first contact, the bare form resolves to the local-origin copy and the CLI prints the qualified forms when it has to disambiguate. No replicated fact ever changes.
4. **Backfill as one synchronous migration (v47) in a single transaction**, emitting dedicated `import.*` kinds, the "migration event" pattern [S3][S2]. Do not replay synthetic fine-grained history. At the measured size this takes well under a second (see Size). Add a chunked, cursored mode only if a store is beyond about 10^5 messages, using the existing `0040`/`0043` backfill-cursor pattern.
5. **Gate linking on backfill completion.** A node advertises its own origin in its version vector only after the backfill commits, so peers never see a partial range.
6. **Downgrade = restore a pre-migration snapshot.** Write a `VACUUM INTO` copy before v47. Older binaries already refuse newer schemas (`schema.rs`: `_ => IncompatibleSchema "unsupported schema version"`), so nothing gets corrupted silently.
7. **Validate the journal against the live view in a test.** Project the import entries into an empty store, then diff the result against the migrated tables. Use a copy of a real store as the fixture.

## What exists today (measured, read-only)

The live instance (`~/.local/state/herdr/plugins/herdr-threads/instances/1ad20d92…/threads.sqlite3`) is at schema v46 and was opened with `sqlite3 -readonly`. Its history spans about 13 days (`messages.decision_at` from 1790837631607 to 1791955300174 ms).

| Table | Rows | Replicated? |
|---|---|---|
| seats | 155 (139 `retirements`) | existence and retirement only |
| threads | 63 (19 `direct_threads`) | yes |
| messages | 2730: 737 ordinary, 1862 info, 131 warn | yes |
| invitations | 118 (1 rejection) | yes |
| memberships / membership_intervals | 227 / 214 | join/leave facts |
| receipt_state | 1688 (`receipts`: 0) | ACK/settlement |
| operations (idempotency cache) | 5124 | node-local |
| binding_cursors, wake_work, warning_*, digest_* | about 2.5k | node-local |

The bodies of ordinary messages total 588 KB (average 798 B, maximum 3.6 KB). `event_json` totals 192 KB. According to `dbstat`, `operations` takes 4.3 MB and `messages` 1.7 MB. The file is 150 MB, but 34,936 of its 38,212 pages are on the freelist; that is unrelated bloat. A second instance (`9639…`, v10) is empty.

## Which rows become journal entries, in what order

Every import entry is authored by the local node, so per-origin FIFO is the only ordering rule that matters. The backfill must put every parent at a lower counter than its children:

1. **`import.seat`**: id, home node (self), role, created_at, state. A retirement is either a follow-up `import.seat_retired` entry or a field of the import. Pane mapping, structural/target columns and `wake_on` stay node-local.
2. **`import.thread`**: id, created_at, goal, managed owner, and the multi-writer registers (topic, name, archived) stamped with an HLC derived from `updated_at`. `direct_threads` pairs go in as an attribute. Deterministic 1:1 IDs (Q6) apply only to new pairs; the 19 legacy pairs keep their IDs, and the thread-union future work can merge them later.
3. **`import.invitation`**: thread, seat, inviter, episode, created_at, deadline, confirmation. The outcome (accepted with actor/generation, or rejected with reason) goes in its own entry, or inline because it has the same author.
4. **Membership intervals**: one join entry per interval and one leave entry per closed interval. `joined_seq` and `left_seq` are local thread-sequence positions, so they map to "after message X" anchors or are dropped. Under the vision's ordering, `sequence` becomes local ingest order.
5. **`import.message`**, per thread in `sequence` order: id, kind, author fields, body or event_json, decision_at, `source_message_id`/`source_invitation_id`, delivery mode, ack_required and the recipient set (from `prepared_recipients`/`send_manifests`, because remote nodes need the obligation list). Most of the info/warn rows (73% of timeline rows) carry node-local context such as `instance`, `native_session` and `binding_generation`. Replicating them as opaque data is acceptable; whether thread-scoped warns are useful remotely is a product question.
6. **`import.receipt_settlement`** for each `receipt_state` row that is acked or recipient_retired, authored by the recipient's home node, which today is always the local node. Pending rows are implied by message plus recipients plus ack_required. Pokes, offers and warning linkage stay local.

Interleaving everything globally by `decision_seq` would also work, but the per-type topological order is simpler and enough: before linking, every referenced entity is local, so no cross-origin dependency exists.

On the authoring node these entries are inserted **already-applied**, because the existing tables are the view. Only receiving nodes materialize them. Separate `import.*` kinds keep migrated aggregates distinguishable from native ones, which helps debugging [S3]. Their payload can carry final state instead of history that was never recorded: overwritten topics cannot be reconstructed.

## Size and time

Local benchmark, scratch SQLite in WAL mode with `synchronous=normal`, journal `(origin, counter)` primary key and about 850-byte payloads:

- 5.5k entries (≈ this store): **0.03 s**, about 5.6 MB.
- 550k entries (100x): **6.6 s**, and the WAL grows to about 560 MB inside a single transaction.

Real payloads are smaller on average, because most rows are about 100-byte events. A single-transaction migration is therefore right for realistic stores. Only stores far beyond the measured one need a chunked backfill with a high-water cursor (the `receipt_deadline_backfill` shape in `migrations/0040_receipt_deadline_backfill.sql`), with linking deferred until it completes.

## Every place IDs are referenced

These counts are the main argument against rewriting:

- **Columns:** about 100 columns in the live schema hold seat, thread, message or invitation IDs (53 are seat columns alone), with 87 declared FKs into `seats`, `threads`, `messages` and `invitations`.
- **JSON payloads:** 1993 of 2730 `messages.event_json` values embed seat, invitation or thread IDs (`{"action":"invite","actor_seat":"seat-e25rD3Ub","invitation":"inv-m3OwoTvd",…}`). `operations.result_json` embeds them in 2168 rows.
- **Idempotency keys:** `operations.actor_scope` is `seat:<id>` in 4836 rows. Rewriting it would make retried operations miss their cached results and re-execute.
- **Outside the store:** agent transcripts and context, skill and user notes, client-local hint files and Herdr pane metadata all hold IDs that a migration cannot reach. A re-key needs an alias lookup on every input path, permanently, and every output path must emit the canonical form. Each lookup is a single indexed probe of a few microseconds, so speed is not the issue; the cost is the size of the correctness surface.

Keeping legacy IDs removes all of this. The only new rule is the origin-scoped resolution of legacy references in replicated payloads.

## First contact between two nodes that both have legacy data

Each legacy compact ID was drawn from 62^8 ≈ 2.18×10^14 values and checked for collisions only locally (`src/store/public_ids.rs`). For N_i IDs per namespace on node i, P(any cross-node collision) ≈ Σ_{i<j} N_i·N_j / 62^8:

- 2 nodes × 3k IDs: about 4×10^-8.
- 10 nodes × 3k: about 2×10^-6.
- 10 nodes × 100k: about 2×10^-3.

The last figure is rare but not negligible, so collisions must be handled deterministically rather than assumed away. The long `seat-<uuid>` forms are effectively collision-free.

Handling:

- Journal facts never conflict, because identity is `(origin, counter)`. Only the materialized `UNIQUE(id)` can clash. Make the view's legacy key `(origin, id)`, or keep a `legacy_id_collisions` side table.
- Every node, third parties included, ingests both import entries and detects the same collision, so the outcome converges. CouchDB faces the same problem when two databases hold the same `_id`: it keeps both revisions and picks "a deterministically-chosen winner" identically on all peers [S4]. We need no winner because nothing is overwritten. Only bare-ID resolution is ambiguous, and it prefers the local-origin copy, which keeps every pre-link reference on that node correct.
- Legacy references inside an import payload always mean the importing origin's object, because everything was local before linking. Post-link entries reference legacy objects origin-qualified.

This is the same move as Matrix room upgrades: the old room stays reachable under its old ID, and a new identity links back to it through `predecessor`/tombstone [S1]. Old IDs are never reassigned. Kurrent's migration guide warns that a stream ID is "not possible to change afterwards", so the key mapping must be decided before import [S2]. That supports fixing `(origin, legacy_id)` as the permanent key now.

## Rollback and downgrade

- Before v47 runs, write `VACUUM INTO <state>/threads.pre-v47.sqlite3`. No such backup exists today. A downgrade means stopping the daemon and restoring that file. Writes made after the migration are lost, and if the node has already linked, peers will hold this origin's entries that the restored node lacks. That is exactly the "node restored from backup reusing counters" case (Q9), so a restore must bump a per-log epoch.
- The migration only adds a journal and import entries. It does not reshape existing tables, so a forward-fix needs no data rewrite.

## Limitations

- Row counts come from one developer's store (13 days, about 155 seats). Heavier users may have 10-100x as much data.
- The benchmark is synthetic and includes no index or materialization cost on receiving nodes.
- Prior-art sources are design notes and blogs, not formal studies.

## Methodology

Repo files read: the vision spec, `src/protocol/ids.rs`, `src/store/public_ids.rs`, `src/store/schema.rs` (version dispatch), `migrations/0040_receipt_deadline_backfill.sql`, and the TRUST-POLICY "Probabilistic identifiers" accepted limit.

Live store: queried read-only (`sqlite3 -readonly`) for row counts, `dbstat`, ID shapes and JSON references.

Web search: `search` CLI with the brave and exa providers. Preflight passed. 3 queries, 18 results considered, 4 original documents retrieved, no failures. Retrieval stopped once every facet was answered from repo evidence plus one prior-art source per pattern.

## Sources

- [S1] Matrix MSC1501, Room version upgrades. https://github.com/matrix-org/matrix-spec-proposals/blob/main/proposals/1501-room-version-upgrades.md
- [S2] D. Doomen, "How to gradually migrate from CRUD to Event Sourcing", Kurrent blog, 2021. https://www.kurrent.io/blog/a-recipe-for-gradually-migrating-from-crud-to-event-sourcing/
- [S3] Arkency, "The final trick when moving from Rails CRUD to Event Sourcing". https://blog.arkency.com/the-final-trick-when-moving-from-crud-to-event-sourcing/
- [S4] Apache CouchDB 3.5 docs, "Replication and conflict model". https://docs.couchdb.org/en/stable/replication/conflicts.html
