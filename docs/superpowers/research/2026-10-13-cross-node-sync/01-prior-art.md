# 01 — Prior art and pitfalls: per-origin append-only logs with version-vector anti-entropy

Research question 1 of the [cross-node sync vision](../../specs/2026-10-13-cross-node-sync-vision-design.md). Deep-mode pass, 2026-10-13. The decisions in the spec are taken as settled; risks the evidence raises against them are listed separately and labeled.

## Recommendations for herdr-threads

1. **Use Bayou as the algorithmic template and SSB EBT as the wire template.** Bayou's anti-entropy uses per-server accept-stamps, a prefix property and version vectors, which is our design almost exactly. It shows that the vector alone is a complete, resumable checkpoint: "The prefix property enables the use of version vectors to compactly represent the set of writes known to a server" ([Bayou, Petersen et al. SOSP'97](https://www.cs.princeton.edu/courses/archive/fall16/cos418/papers/flexible.pdf)). SSB's EBT supplies the per-feed "notes" exchange, the rule to request a feed from only one peer at a time, and clock caching across reconnects ([epidemic-broadcast-trees](https://github.com/ssbc/epidemic-broadcast-trees)). Corrosion is the closest *implementation* (Rust, SQLite, per-actor versions), but it is mostly a source of cautionary tales (see 4).
2. **Keep the prefix property strict and never record "have" before commit.** Advance `vv[origin]` only in the same SQLite transaction that durably appends the contiguous entries. Drop or buffer anything past the first gap and re-pull it; never mark it as seen. Corrosion lost data permanently when a failed batch had already been entered in its `seen` cache, so "subsequent gossip rebroadcasts of the missing changes from other peers are filtered out as duplicates" ([corrosion#577](https://github.com/superfly/corrosion/issues/577)).
3. **Make counter reuse detectable. This is the top pitfall across systems.** Every system with per-writer counters breaks when a writer restarts from old state: SSB forks "irrecoverably break your account" ([Manyverse](https://www.manyver.se/blog/2022-03-update/)), duplicate Yjs ClientIDs mean "the document might get permanently corrupted" ([Yjs FAQ](https://docs.yjs.dev/api/faq)), and a copied cr-sqlite file keeps its site id ([cr-sqlite#145](https://github.com/vlcn-io/cr-sqlite/issues/145)). Recommendations:
   - Each entry carries a content hash, and preferably `prev_hash` (the SSB/Hypercore chain).
   - The vector exchange carries `(counter, hash_at_counter)` per origin, like LiteFS's position of TXID plus rolling checksum ([LiteFS position](https://docs.fly.io/litefs/position)). Divergence then shows up at handshake, not weeks later.
   - Add a per-origin **epoch/incarnation** to the origin key. On startup, if the local journal may be behind (restored backup, lost DB), a node must either resync its own log from peers before authoring (Manyverse's "Resync" fix, [manyverse#111](https://github.com/staltz/manyverse/issues/111)) or open a new epoch.
4. **Bound catch-up.** Pull bounded ranges, cap the bytes in flight, cap concurrent syncs per node, and apply in chunked transactions. CouchDB: "It is RECOMMENDED to process the feed in small chunks" ([CouchDB replication protocol](https://docs.couchdb.org/en/stable/replication/protocol.html)). Corrosion's queue grew huge when "a node has been off for a while and then starts syncing millions of changes" ([corrosion#198](https://github.com/superfly/corrosion/issues/198)), and its handshake can answer `MaxConcurrencyReached` ([sync.rs](https://github.com/superfly/corrosion/blob/main/crates/corro-types/src/sync.rs)). First contact with full history is exactly this case.
5. **Validate every wire range and count before using it.** Corrosion's tasks panicked on inverted ranges ([corrosion#548](https://github.com/superfly/corrosion/issues/548)).
6. **Model node identity changes explicitly, as Bayou creation and retirement facts.** Bayou servers announce themselves with a creation write and leave with a retirement write. A peer "removes the corresponding entry from all the server's version vectors" only after it has the retiring server's full prefix ([Bayou §4.3](https://www.cs.princeton.edu/courses/archive/fall16/cos418/papers/flexible.pdf)). Without this, a joining node waits forever on gaps from removed actors ([corrosion#581](https://github.com/superfly/corrosion/issues/581)). We never delete, but a "node retired, final counter = N" fact lets readiness checks and UIs stop waiting on a dead origin.
7. **Do not truncate vectors, and skip Merkle trees.** With ≤10 nodes, plus extra origins from reinstalls, the vectors stay small. Dynamo's size cap drops the oldest pair, and Dynamo admits the cap makes "descendant relationships cannot be derived accurately" ([Dynamo §4.4](https://www.allthingsdistributed.com/files/amazon-dynamo-sosp2007.pdf)). Merkle-tree anti-entropy (Dynamo, Cassandra) exists for unordered key spaces; prefix-closed logs do not need it. An optional periodic per-origin hash comparison catches silent corruption cheaply.
8. **Reuse crates for plumbing, ideas for protocol.** `rusqlite` (already in use), `uhlc` for the HLC, and `rangemap` only if the design ever allows gaps. Write the sync engine in-tree: no crate implements per-origin-log anti-entropy over an arbitrary stream for SQLite.

## Risks in settled decisions (flagged)

- **RISK A: node ID from the SSH host key versus "a node is an instance".** Two herdr-threads instances (two Herdr servers) on one machine share a host key, so they would derive the same ID and be refused at handshake. Cloned VMs and containers often share host keys too, and reinstalling or rotating the host key silently changes a node's ID. The evidence on identity (Yjs, cr-sqlite#145, SSB) consistently shows that ids tied to a copyable artifact cause collisions or forks. Suggestion: derive from host key *plus* instance (`instance_dir`), and persist the ID once it is chosen. Owned by researcher 05.
- **RISK B: no per-log epoch for restore or lost-journal recovery.** The spec says "counter is monotonic per origin" and "nothing silently lost", but it gives no rule for a node restored from a backup. Without an epoch and hash check, the restored node re-issues counters already seen elsewhere. Peers then either silently keep the old entry (dedup by `(origin, counter)`), which loses the new one, or diverge. This is the SSB failure mode exactly (Recommendation 3).
- **RISK C: unknown kinds and per-origin FIFO apply.** "Unknown kinds stay unapplied" needs a rule for whether a later *known* entry from the same origin may apply past an unapplied unknown one. Blocking stalls the whole origin on an old node. Skipping is safe only if kinds are designed so that later known entries never depend on earlier unknown ones (for example, ACK of a message whose creation kind is unknown). This should be an explicit invariant, with the journal (not the materialized view) as the replay source.
- **RISK D: first contact means full history over one SSH stdio stream.** Without range-bounded pulls and resumable checkpoints (which come free from the vector, if Recommendation 2 holds), a partition-heal storm or a large backfill monopolizes the link and the apply loop. That is a lower risk at 1–10 msg/s, but it applies on day one, because the upgrade backfill turns all existing history into journal entries.

## Survey

### Secure Scuttlebutt (feeds, EBT)

**Maps:** one signed append-only feed per identity, where "Each message (except the first one) references the ID of the previous message" and sequence numbers start at 1 ([protocol guide](https://ssbc.github.io/scuttlebutt-protocol-guide/)). EBT exchanges a vector clock of per-feed sequences, packing each entry as "{ feed: (seq === -1 ? -1 : seq << 1 | !rx) }" ([EBT](https://github.com/ssbc/epidemic-broadcast-trees)). That is our `{origin -> counter}` plus a "send me this" bit. Relay is transitive: any peer serves any feed it holds, the same as our store-and-forward.

**Reusable ideas:**
- Peers cache the last clock exchanged: "Each peer remembers the vector clock _sent_ by the other peer". On reconnect only the changed entries are sent.
- "only request new messages using rx for a feed from one of the nodes". This avoids duplicate streams in a mesh, with a timeout to switch peers when one stalls.

**Pitfalls:**
- Forks. Restoring an identity and posting before resyncing created a new root message; the real feed "is silently ignored", leaving the account read-only ([manyverse#111](https://github.com/staltz/manyverse/issues/111)). The fix was a mandatory resync before authoring ([Manyverse 2022-03](https://www.manyver.se/blog/2022-03-update/)).
- One identity per device is unavoidable: "Each device still uses it's own identity" ([guide, metafeeds](https://ssbc.github.io/scuttlebutt-protocol-guide/)). That matches our one-origin-per-node design.

**Crates:** `kuska-ssb` 0.4.0, last updated in 2022 ([crates.io](https://crates.io/api/v1/crates)). Use the ideas only.

### Matrix (event DAG, federation)

**What we avoid:** each event names `prev_events` and forms a room DAG; gaps are filled by "a breadth-first walk of the `prev_events` for the `latest_events`" via `/get_missing_events` ([S2S spec](https://spec.matrix.org/latest/server-server-api/)). Room state is computed by state resolution when "two branches in the event graph merge". Years of "state resets" followed: "users being re-added into a room they have left". Room v12 changed the algorithm again ([Project Hydra](https://matrix.org/blog/2025/08/project-hydra-improving-state-res/)). Our single-author facts plus three LWW fields avoid this class of bug by construction.

**Maps:** v12 makes "the room ID equal to the hash of the room's creation event". That supports content-derived, deterministic IDs such as our 1:1 thread ID.

### CouchDB / PouchDB

**Maps:** a pull replicator reads `_changes` since a checkpoint, sends `_revs_diff`, and stores checkpoints in `_local` docs on both ends. Checkpoint validity hinges on `session_id` agreement: on mismatch it searches history for a common session, otherwise "the Replicator MUST run Full Replication" ([protocol](https://docs.couchdb.org/en/stable/replication/protocol.html)).

**Lessons:**
- A per-peer checkpoint can disagree with reality after a restore. Our vector is computed from our own journal, so it cannot go stale relative to our data, which is better.
- Process the changes feed "in small chunks".
- Per-peer sequence numbers are local, not global. That mirrors our local `sequence` versus the global `(origin, counter)`.

### cr-sqlite and Corrosion (Fly.io)

**cr-sqlite:** state-based CRDT tables. Each transaction gets a database-wide `db_version`, and peers pull with "`WHERE db_version > x AND site_id != some_site_id`" ([cr-sqlite](https://github.com/vlcn-io/cr-sqlite)). Its README says it "Keeps no history / only keeps the current state" and that causal event logs are planned for v2. That is a different model from our journal (row merge versus an immutable fact log). Pitfall: copying the DB file copies `site_id` ([#145](https://github.com/vlcn-io/cr-sqlite/issues/145)).

**Corrosion:**
- Built on cr-sqlite, with SWIM membership and QUIC, using LWW on logical timestamps ([Corrosion blog](https://fly.io/blog/corrosion/), [CRDTs](https://superfly.github.io/corrosion/crdts.html)).
- Its sync state is the closest code-level analogue of our vector exchange: `heads: HashMap<ActorId, DbVersion>` plus `need: HashMap<ActorId, Vec<RangeInclusive<…>>>` and `partial_need` ([sync.rs](https://github.com/superfly/corrosion/blob/main/crates/corro-types/src/sync.rs)).
- Corrosion *allows gaps*, because broadcast delivers out of order, and keeps gap bookkeeping tables. That is the source of its failures:
  - dropped batches masked by the seen-cache ([#577](https://github.com/superfly/corrosion/issues/577));
  - gaps from removed actors that "no current peer appeared able to fill" ([#581](https://github.com/superfly/corrosion/issues/581));
  - unbounded queues on catch-up ([#198](https://github.com/superfly/corrosion/issues/198));
  - panics on inverted ranges ([#548](https://github.com/superfly/corrosion/issues/548)).
- Operations: a fleet-wide deadlock ("every proxy in our fleet had locked up hard") and retry storms that were "literally saturating our uplinks" ([blog](https://fly.io/blog/corrosion/)).

**Takeaway:** keep per-origin FIFO (no gaps), and keep the handshake's concurrency rejection and range validation. Corrosion is a service, not a library; `corro-types` on crates.io is stale.

### Automerge / Yjs

- **Yjs** state vectors denote "the next expected clock from each client", and a sync sends the diff against the peer's vector ([Yjs updates](https://docs.yjs.dev/api/document-updates)). That is identical in shape to our pull. Yjs mints a new ClientID per session because a reused one corrupts documents ([FAQ](https://docs.yjs.dev/api/faq)), which is a cheap answer to restore-reuse that we cannot copy directly, since our origin is a stable node.
- **Automerge** syncs hash-DAG heads with Bloom filters and "assumes a reliable in-order stream" ([sync.rs](https://github.com/automerge/automerge/blob/main/rust/automerge/src/sync.rs)). Hash heads are robust to equivocation; vectors are not. That is the case for adding entry hashes (Recommendation 3).
- **Crates:** `automerge` 0.12 and `yrs` 0.28 are mature but model a document CRDT. Ideas only.

### Bayou

The closest algorithmic match:
- per-server monotonic accept-stamps;
- the prefix property, which makes vectors sufficient;
- one-pass sending of the writes "not covered by that vector";
- incremental progress across interruptions, with "writes need not be resent";
- an ordered transport is required ([paper §2–3](https://www.cs.princeton.edu/courses/archive/fall16/cos418/papers/flexible.pdf)).

It also predicts our log-growth tradeoff. "a single, long disconnected replica can cause the write-logs at all other replicas to grow indefinitely"; Bayou chose truncation with full-state transfer as the fallback. Our no-delete stance accepts growth instead, which is fine at 1–10 msg/s. Creation and retirement writes (§4.3) are the model for node lifecycle facts (Recommendation 6).

### Dynamo / Cassandra

- **Dynamo:** vector clocks per object, truncated at about 10 entries, with admitted imprecision ([§4.4](https://www.allthingsdistributed.com/files/amazon-dynamo-sosp2007.pdf)). Merkle trees per key range need recalculation "when a node joins or leaves".
- **Cassandra:** incremental repair never re-checks data marked repaired, so full repairs are still needed ([repair docs](https://cassandra.apache.org/doc/latest/cassandra/managing/operating/repair.html)).
- **Relevance:** low. Merkle trees reconcile unordered sets; we have ordered per-origin logs. The lesson is to verify periodically even when the bookkeeping says "in sync".

### NATS JetStream mirrors and sources

A mirror keeps the upstream sequence; a source aggregate "gives them fresh sequence numbers as they arrive", and "across upstreams there's no ordering guarantee" ([docs](https://docs.nats.io/nats-concepts/jetstream/source_and_mirror)). That is exactly our dual identity: local ingest `sequence` versus the origin `(origin, counter)`, with no global order. The fetched page does not cover cycles or resume (ADR-59 was not read).

### SQLite session extension

Changesets are per-row INSERT/UPDATE/DELETE with conflict callbacks, and they require a declared PRIMARY KEY ([sessionintro](https://www.sqlite.org/sessionintro.html)). They carry no ordering or origin metadata in the fetched docs. Not a fit: we replicate *facts*, not row diffs, and the materialized tables stay derived. `rusqlite` exposes it behind a `session` feature, but there is no reason to use it.

### Litestream / LiteFS

These are single-writer physical replication systems.
- **LiteFS:** a replica detects split-brain when "its checksum is different even though its transaction ID could be the same", and falls back to a snapshot ([architecture](https://github.com/superfly/litefs/blob/main/docs/ARCHITECTURE.md)).
- **Litestream:** older versions tracked state with random "generation" IDs, which v0.5 replaced with TXID-based LTX files ([how it works](https://litestream.io/how-it-works/)).
- **Ideas:** position = counter plus checksum, and an epoch on restore (Recommendation 3). Neither is usable directly (single primary, file level).

### Hypercore

Hypercore is a signed Merkle append-only log. `truncate()` bumps a monotonic **fork id** ("Per default this will update the fork id of the core to + 1"), and the signable state includes key, tree hash, length and fork ([hypercore](https://github.com/holepunchto/hypercore)). It tracks `contiguousLength` per peer ([Pear docs](https://docs.pears.com/p2p/reference/building-blocks/hypercore/)), which is our contiguous high-water mark. The fork id is the precedent for a per-log epoch. The Rust `hypercore` 0.16 crate has low adoption; use the ideas only.

### Git

Protocol v2 negotiates with `have` and `want`: the server returns "ACK obj-id" for common objects, then "ready" ([protocol-v2](https://git-scm.com/docs/protocol-v2)). Ancestry search is needed because history is a DAG. With per-origin counters, our "have" collapses to one integer per origin, a one-round-trip negotiation, which is the point of the design.

## Closest templates

1. **Bayou anti-entropy** for semantics: prefix property, vectors as the checkpoint, ordered stream, incremental resume, creation and retirement writes.
2. **SSB EBT** for operations: a per-feed vector "notes" exchange on connect and on change, cached peer clocks, one upstream per origin with a stall timeout, and transitive relay. It comes with SSB's hard lesson on forks, so add Hypercore/LiteFS-style hash or epoch checks.

Use Corrosion's code and issue tracker as a checklist of what goes wrong when gaps, seen-caches and unbounded queues creep in.

## Limitations

- Fetched pages did not cover NATS source cycles (ADR-59), Automerge sync-state persistence, or reader behaviour on a Hypercore fork.
- The Kleppmann/Howard BEC paper ([arXiv 2012.00472](https://arxiv.org/abs/2012.00472)) was read at the abstract level only. Its argument for hash graphs over vectors is inferred from the design, not quoted.
- Crate maturity comes from crates.io metadata (version, last update, recent downloads), not from code review.

## Methodology

- **Mode:** deep, one retrieval round.
- **Tools:**
  - `search` CLI: preflight OK; the serper and perplexity providers failed for lack of credits or quota, the other providers answered.
  - WebFetch: 27 fetches.
  - WebSearch: 1 query.
  - `gh`: Corrosion source and issues, Manyverse and cr-sqlite issues.
  - curl: crates.io API and the Bayou PDF; pdftotext for the Dynamo and Bayou PDFs.
- **Searches:** 10 search-CLI queries, plus 1 WebSearch and 1 `gh search issues` query (6 to 25 result entries each, roughly 90 considered).
- **Documents:** 31 distinct original documents registered.
- **Failures:**
  - web.archive.org is blocked.
  - automerge.org returned 404 for the merge-rules page.
  - The Dynamo PDF could not be read through WebFetch, so it was extracted locally.
- **Retrieval stop:** budget-exhausted, with all high-priority facets supported. The ledgers (coverage, queries, sources, evidence, claims, manifest) are in the researcher's scratch run directory.

## Bibliography

- Petersen, Spreitzer, Terry, Theimer, Demers. *Flexible Update Propagation for Weakly Consistent Replication*, SOSP 1997. https://www.cs.princeton.edu/courses/archive/fall16/cos418/papers/flexible.pdf
- DeCandia et al. *Dynamo*, SOSP 2007. https://www.allthingsdistributed.com/files/amazon-dynamo-sosp2007.pdf
- SSB: https://github.com/ssbc/epidemic-broadcast-trees ; https://ssbc.github.io/scuttlebutt-protocol-guide/ ; https://github.com/staltz/manyverse/issues/111 ; https://www.manyver.se/blog/2022-03-update/
- Matrix: https://spec.matrix.org/latest/server-server-api/ ; https://matrix.org/blog/2025/08/project-hydra-improving-state-res/
- CouchDB: https://docs.couchdb.org/en/stable/replication/protocol.html
- cr-sqlite: https://github.com/vlcn-io/cr-sqlite ; https://github.com/vlcn-io/cr-sqlite/issues/145
- Corrosion: https://fly.io/blog/corrosion/ ; https://superfly.github.io/corrosion/crdts.html ; https://github.com/superfly/corrosion/blob/main/crates/corro-types/src/sync.rs ; issues #577, #548, #198, #581 at https://github.com/superfly/corrosion/issues/
- Yjs: https://docs.yjs.dev/api/faq ; https://docs.yjs.dev/api/document-updates
- Automerge: https://github.com/automerge/automerge/blob/main/rust/automerge/src/sync.rs ; https://arxiv.org/abs/2012.00472
- Cassandra: https://cassandra.apache.org/doc/latest/cassandra/managing/operating/repair.html
- NATS: https://docs.nats.io/nats-concepts/jetstream/source_and_mirror
- SQLite: https://www.sqlite.org/sessionintro.html
- LiteFS/Litestream: https://github.com/superfly/litefs/blob/main/docs/ARCHITECTURE.md ; https://docs.fly.io/litefs/position ; https://litestream.io/how-it-works/
- Hypercore: https://github.com/holepunchto/hypercore ; https://docs.pears.com/p2p/reference/building-blocks/hypercore/
- Git: https://git-scm.com/docs/protocol-v2
- crates.io API (automerge, yrs, hypercore, foca, rusqlite, kuska-ssb, ruma, uhlc, rangemap, negentropy, corro-types): https://crates.io/api/v1/crates/<name>
