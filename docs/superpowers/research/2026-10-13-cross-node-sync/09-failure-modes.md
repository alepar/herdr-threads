# 09 — Failure modes and their handling

Topic 9 of the cross-node sync research (vision: `docs/superpowers/specs/2026-10-13-cross-node-sync-vision-design.md`, cited as [L1]). Mode: deep-research, DEEP. Sources are cited inline as [S#] (web) and [L#] (repo). The bibliography is at the end.

## Recommendations

Build these into v1. Together they are cheap: about 40 bytes per entry, one nonce per store, and a few columns.

1. **Per-entry content hash and per-origin hash chain.** Each entry carries `hash = H(origin_key, incarnation, counter, hlc, kind, kind_version, payload, prev_hash)`. Vector exchange sends `{origin -> (counter, head_hash)}`, and the peer that is further ahead checks its stored hash at the shorter counter. A counter alone cannot detect a rollback once the restored node has counted past the point its peers saw. Active Directory documents that case as undetected and long-lived: "a range of USNs applies to two different sets of changes. This can continue for extended periods without being detected" [S2]. SSB, Hypercore and Matrix all chain or hash entries for this reason [S7][S10][S21].
2. **Incarnation (epoch) in the origin key.** Replication identity becomes `(origin_key, incarnation, counter)`. The store mints a random 64-bit incarnation when it is created, and mints a new one whenever rollback or loss is suspected. This is AD's invocationID pattern: on a restore "the domain controller resets the Invocation ID … thereby preventing USN reuse", and partners then request "everything from DC1 associated with InvocationID B" [S1]. Riak fixed its kv679 silent-loss bug the same way, with a new actor epoch whenever an actor might reissue a dot [S3][S4]. A new incarnation can never collide, so an incarnation that is rotated without need costs one vector entry and nothing else.
3. **A genesis entry per incarnation** (counter 1). It holds the full host-key fingerprint, the instance discriminator, the incarnation and the predecessor incarnation, if any. Vector entries carry the genesis hash. Any node that sees two different genesis entries under the same short node ID has found a late collision, even if it never shook hands with either node.
4. **Key the journal internally by a long origin key, not the 2–4 char node ID.** The short ID is used only for display and public IDs. A short-ID collision then makes IDs ambiguous to read but cannot corrupt sync.
5. **Ingest FIFO is separate from apply.** Storing, relaying and advancing the vector never depend on whether an entry can be applied. Unknown kinds, entries that fail to decode, and entries whose dependencies are parked go into an `unapplied` table and are replayed in `(origin, counter)` order after an upgrade. Never drop such an entry: AWS warns that a DLQ breaks FIFO order [S20].
6. **Apply is idempotent by public fact ID** (message, invitation, receipt or ACK key), not by `(origin, counter)`. This lets a fact be safely re-issued under a new incarnation after a fork, and makes backfill and migration replays safe.
7. **Durability rule.** An entry is served to peers only after its commit is durable. With WAL and `synchronous=NORMAL`, "a transaction committed … might roll back following a power loss" [S17]. That rollback would turn an ordinary crash into a fork. Production already commits with `FULL`; only tests relax it (`src/store/connection.rs`). The sync code should assert `FULL`.
8. **Bounded atomic ingest batches.** One transaction covers the journal rows, the vector advance and either materialization or the apply cursor. On `SQLITE_FULL`/`IOERR`, issue an explicit `ROLLBACK` [S15] and retry with backoff. Keep a free-space headroom so local authoring keeps working when relaying has filled the disk.
9. **Restore self-check at startup and at every handshake** (detailed below). On suspicion: rotate incarnation, pull your own old tail back from peers, and do not author under the old incarnation again.
10. **Fork quarantine instead of "first seen wins".** SSB's history shows that silently keeping whichever branch arrived first leaves nodes divergent for good [S8][S9].
11. **Operator CLI** (§ Operator tools). Each repair records `operator:local-user:<uid>` provenance, which TRUST-POLICY already defines [L4].
12. **Suppress delivery during bootstrap and backfill** for facts that are already settled (see risk R3).

Defer to later versions: automatic fork re-issue (v1 only alerts and provides an operator command), automated detection of cloned VMs, verification of signed origins, and partial replication.

## Risks in the settled decisions (flagged separately)

- **R1 — `(origin_node, counter)` as global identity, with no incarnation and no content hash.** After a backup restore the node reissues counters it already used. Peers that already hold those counters treat the new entries as "already have", so data is lost silently. This is exactly what Riak kv679 did: a vnode that forgot its history "re-issues some event" and "the tombstone has managed to silently delete the new value" [S4]. A NATS stream restored from backup also "resets its sequence state to the backup point" [S25]. The spec lists "per-log epoch?" as an open question. The evidence says it should be mandatory, together with the hash (Rec. 1–2).
- **R2 — node ID from the SSH host key.** (a) Cloned VM images share host keys [S11]. AD's documented undetectable case is the same image running in two places at once [S2], and Syncthing maintainers said they "can't distinguish between same device migrating between networks and two devices with same id" [S11]. (b) The spec defines a node as an *instance* (one Herdr server), yet two instances on one machine share a host key and would collide at handshake. The internal origin key needs an instance discriminator and the incarnation (Rec. 3–4). Topic 05 owns derivation; this is the failure-mode angle.
- **R3 — "Nudges always deliver, however late"** combined with first-contact full-history exchange, recovery of a wiped node's own log, or a returning node that was offline for months. The recipient's home node "runs the existing delivery policy when a fact is ingested" [L1]. Re-ingesting history whose receipts are already settled could wake every local seat. Coalescing into one wake (as the spec says) is necessary but not enough. Facts whose receipt is already settled in the journal, or that predate the seat's binding, must not be offered as new work. That also follows delivery-contract rules 1–2 and the token-efficiency rules.
- **R4 — Nothing deleted, full mesh, up to 10 msg/s.** My arithmetic, at about 1 KiB per entry: 1 msg/s ≈ 86 MB/day ≈ 31 GB/year on *every* node; 10 msg/s ≈ 860 MB/day ≈ 310 GB/year. At the top of the stated range, a full disk is a normal operating state, not an edge case. Rec. 8 headroom plus size reporting (`sync status`) are needed in v1, and the vision should state the expected average rate.
- **R5 — LWW on HLC with forward clock jumps.** A node whose clock jumped forward writes topic, name or archive with a far-future HLC, and that value wins against every later write that is causally unaware of it. The HLC paper's remedy is to "ignore reception of messages that cause l value to diverge too much" [S18], and CockroachDB's is to crash [S19]. Neither is allowed here: entries cannot be dropped, and the system is AP. Mitigation is under F5.

## Failure modes

Each mode is given as detection, recovery, what agents and users see, and what to build in v1 versus later.

### F1. Node restored from backup or snapshot reuses counters (forked origin log)

**Detection.**
- (a) *Local sidecar.* Atomically write `(incarnation, counter, head_hash)` to a small file beside the DB, after commits and periodically. At startup, a DB head behind the sidecar, or a DB with no sidecar, means it was rolled back or copied. This catches the common "copied `herdr.db` back" case. A whole-disk or VM snapshot restores the sidecar too, so it does not catch that.
- (b) *Peer evidence at handshake.* A peer reports, for *my* origin and incarnation, a counter higher than mine, or a different hash at a shared counter. Riak treats "an incoming clock with the same actor+epoch but greater counter" as "a hint that a byzantine failure occurred" [S3]. AD quarantines when a partner presents USNs it has already seen [S2].
- (c) *Third-party fork.* Node X ingests origin O from two peers and gets two different entries for the same `(O, inc, k)`, or an entry whose `prev_hash` does not match. SSB describes this exact way to fork: "If some nodes have not seen the original n_0 then they will accept the new one" [S8].
- Counter comparison alone misses the case where the restored node has already counted past the peers' view [S2]. Rec. 1 closes that gap.

**Recovery.**
- *Self-detected, (a) or (b).* Before authoring anything, mint a new incarnation (genesis entry naming its predecessor). Then pull the old incarnation's tail back from peers and apply it normally. That brings back the facts lost by the restore, as with AD, where updates lost by the rollback "replicate back" from partners [S1]. Entries the restored node authored *before* it noticed are re-issued under the new incarnation with their original public IDs. Idempotent apply (Rec. 6) makes those no-ops wherever they already landed.
- *Third-party detection, (c).* Quarantine the origin incarnation at the last common counter: stop advancing it, keep both branches' entries (nothing deleted), keep whatever was already applied, store the other branch unapplied, and relay a small *fork-evidence* record so other nodes freeze too. SSB proposed but never confirmed shipping a fork proof [S8]. The origin node, once it sees the evidence, does the self-detected recovery, which un-sticks everyone because the new incarnation is a fresh log.
- *Avoid* SSB's outcome, where a restored identity "creates a new root message" and the real history "is silently ignored", leaving a "defunct, read-only state" [S9]. Also avoid the Automerge wedge, where a duplicate `(actor, seq)` was resent forever and was only cured by deleting the document [S13].

**What agents see.** Usually nothing. Messages written during the forked window may arrive late, which is ordinary late-arrival semantics. Operators see a fork alert in `sync status` and the daemon log.

**v1 vs later.** v1: hash chain, incarnation, sidecar, handshake check, quarantine with alert, `sync rotate-incarnation`, and re-issue by operator command. Later: automatic re-issue and fork-evidence gossip tuning.

### F2. Lost or wiped store

**Detection.** On a fresh store, the host key still maps to the same node ID, and peers hold incarnations for that node ID that the local store lacks.

**Recovery.** Because the incarnation is minted at store creation, a wiped node can never reuse old `(origin, inc, counter)` slots. Fresh identity and recovery stop being an either-or: the node keeps its node ID, starts a new incarnation, and on first contact pulls everything, including its own old incarnations. It applies those as its own history; they are authored by the node they claim, so this is not heuristic. Seat *existence* returns with the history. Pane bindings are node-local and are re-established only through normal repair (TRUST-POLICY: never move or merge seats, never end a binding on heuristic evidence). Prior art without incarnations fails here. Matrix: a re-provisioned server "cannot re-join any rooms it was previously participating in" [S22], and "wiping your database does not have a supported recovery method" [S23]. Manyverse forked on restore [S9].

**What agents see.** Seats on that node reappear once sync completes. Bootstrap must not re-nudge settled items (R3). Until the first sync, the node is a near-empty island; that is expected.

**v1 vs later.** v1: incarnation at creation and full pull. A "wait for first sync before authoring" gate is *not* recommended, because it violates AP and incarnations make it unnecessary.

### F3. Corrupt or undecodable entries; a peer serving a truncated log

**Detection.**
- Recompute the hash on receipt. A mismatch means bytes were damaged in transit or in the peer's store. Matrix redacts on hash failure [S21]; Kafka catches and logs `CorruptRecordException` instead of crashing the fetcher [S24].
- A `prev_hash` mismatch at contiguous ingest means a fork (F1).
- A valid hash with a payload that fails to decode is a version or bug issue.
- Local corruption: `PRAGMA integrity_check` already runs when the schema opens and maps to `StoreCorrupt` [L3].

**Recovery.**
- Hash mismatch: reject the batch, keep nothing from it, try another peer for the range, and count errors per peer.
- Undecodable with a valid hash: store, relay and park unapplied, never blocking (Rec. 5).
- Truncated serve: a peer that ends a range early is normal, since it may only hold that much. A peer that advertised counter N but cannot serve it, or whose vector goes backwards between sessions, is marked stale for that origin, and other peers are preferred. Kafka's KIP-101/320 exist because a bare offset "can tell something is wrong, [but] can't tell where in the log the divergence started" [S5]. The fix there, carrying the last epoch in each fetch [S6], is what the incarnation plus head hash gives this design.
- Local corruption: do a `node reset --from-peer` rebuild under a new incarnation (F2), after keeping a `VACUUM INTO` copy of the damaged DB [S16].

**What agents see.** Nothing, unless corruption is local, in which case the existing `StoreCorrupt` error appears.

**v1 vs later.** v1: hashes, peer error counters, the unapplied table and rebuild. Later: per-peer scoring.

### F4. Disk full mid-ingest; partially applied batches

**Detection.** `SQLITE_FULL` or `SQLITE_IOERR`. SQLite says the transaction "may or may not be rolled back automatically" [S15], so the safe response is an explicit ROLLBACK.

**Recovery.** One transaction per bounded batch (Rec. 8) means no batch is ever partially applied. The vector does not advance, so peers resend later. If apply runs as a separate loop, keep a per-origin `applied_through` cursor in the same transaction as each apply. Stop *ingesting* below a headroom threshold, so local sends keep working and keep reporting the existing `StoreFull`, "Nothing was accepted; stored history is intact" [L2]. Never take a backup by file copy during a transaction [S16]; recommend `VACUUM INTO` or the backup API.

**What agents see.** Local sends fail with `StoreFull` only when local authoring itself is out of space. Relay stalls show up as delay.

**v1 vs later.** v1 throughout.

### F5. Clock jumps

**Backward jumps.** At startup, initialise the HLC to `max(wall clock, max HLC in own journal)` so the clock is monotonic across restarts.

**Forward jumps.**
- Accept and store the entry; FIFO and no-loss rule this out of being dropped.
- Log a warning when an entry's HLC exceeds local time by more than Δ (for example 5 minutes). The HLC paper says such actions should "log the offending entries for inspection and … notify the administrator" [S18].
- For the LWW registers, write `hlc = max(local_hlc, current_register_hlc) + 1`. Then any write made *after seeing* a poisoned value still wins, without letting one bad clock drag every node's clock forward.
- Shown send times may look odd. The ACK clock runs on the sender's node only, so a jump there can make deadlines fire early or late. Accept and document this, since overdue is defined as a local fact.

**v1 vs later.** v1: monotonic init, Δ warning, causal LWW bump. Later: a skew column in `sync status`.

### F6. Node ID collision discovered late; cloned VMs

**Late collision.**
- Detection: genesis-hash mismatch under the same short ID (Rec. 3). It works even when the colliding pair is bridged only through relays.
- Recovery: the internal origin key keeps the logs separate (Rec. 4), so the problem is cosmetic. Refuse new links per spec, warn, and offer re-keying of the short ID later.
- This differs from Yjs, where "two Y.Doc instances with the same ClientID" can leave the document "permanently corrupted" [S12].

**Cloned VM.**
- The host key, store and incarnation are all copied, so both copies extend the same log. That is F1(c) detection, and it can only be noticed when the branches meet [S2][S11].
- cr-sqlite found no reliable way to tell "no longer the same file" [S14]. Proxmox and Syncthing rely on documentation [S11].
- Recovery: an operator regenerates the host key and runs `node reset --from-peer` on the clone. AD's guidance is the same: a clone running under the source identity "should be removed from the network" [S1].
- Later: optional hints (machine-id, hypervisor generation ID where exposed) recorded in the sidecar and used only to *suggest* incarnation rotation.

### F7. A stuck unknown-kind entry blocking FIFO

Prevented by design (Rec. 5).
- Ingest order is strict; apply order skips unknown kinds.
- Entries that reference an unapplied entry (for example a message into a thread created by a new kind) are parked with `depends_on`.
- An upgrade replays parked entries in order.
- Never re-request or retry-loop on apply failure; the q2 wedge was a poison change "resent… on every subsequent attempt" [S13].
- Agents on older nodes simply do not see the newer feature.

### F8. Node offline for months

**Volume.** Up to about 26 GB at 10 msg/s × 30 days (R4).

**Mechanism.**
- Range pull with resume: the vector advances per batch, so a broken link loses at most one batch.
- Pull from any peer and spread origins across peers.
- Prioritise origins whose seats share threads with local seats, for faster useful convergence.
- Report per-origin lag (entries behind, and the HLC of the last ingested entry) in `sync status`.
- Agents get at most one compact marker per thread when a member's home node lags past a threshold. Per-message markers would break the token-efficiency rules.
- Corrosion retains per-actor gaps and clears them "only when certain that the old actor cannot come back" [S26]. Likewise, `peer forget` leaves the vector entry as is.

**v1 vs later.** v1: resume, lag report, R3 suppression. Later: lag-aware prioritisation.

## Operator tools (v1)

| Command | Purpose |
|---|---|
| `sync status [--origin]` | Vector per origin incarnation (counter, head hash, genesis), each peer's view, lag, unapplied and quarantined counts, store size and free headroom |
| `sync verify` | Re-hash the local journal chain |
| `sync resync --from <peer> [--origin]` | Re-pull a range, or everything |
| `sync quarantine <origin-inc>` / `release` | Manual freeze or unfreeze, the analogue of AD's replication disable and event 2095 [S2] |
| `sync rotate-incarnation` | Force a new incarnation |
| `sync fork resolve <origin-inc>` | Re-issue the local branch under a new incarnation |
| `sync unapplied list` / `replay` | Inspect and retry parked entries |
| `node reset --from-peer` | Keep a `VACUUM INTO` copy, start an empty store under a new incarnation, pull everything |

All of these are `--operator` gated, are recorded with `operator:local-user:<uid>` [L4], and delete nothing replicated. The TRUST-POLICY amendment should list fork quarantine and re-issue as operator repairs.

## Methodology and limitations

- Tools: `search` CLI 0.9.0 (preflight `ok`; every query returned `partial_success`, meaning usable results with some component-provider failures), WebFetch, and local `pdftotext`. 27 query attempts, 270 result entries considered, and 26 distinct original documents retrieved (partial scope, excerpts with locators).
- 3 failed fetches:
  - russelldb kv679 write-up: HTTP 404, not recovered; covered instead by [S3][S4].
  - Hypercore on npm: 403, recovered via the GitHub README.
  - CockroachDB docs: cross-host redirect, recovered.
- The repo was read for [L1]–[L4].
- Ledgers are in the researcher's scratch directory (`coverage.json`, `queries.jsonl`, `sources.jsonl`, `evidence.jsonl`, `retrievals.jsonl`, `run_manifest.json`). They are not committed.
- Retrieval stop: `budget-exhausted`. Every high-priority facet had direct evidence after round 2, but the two post-readiness low-yield probe rounds were not run.
- Gaps:
  - SSB's fork proof was never confirmed as implemented [S8].
  - The storage-growth figures are my arithmetic, not sourced.
  - The fork-resolution protocol (quarantine plus re-issue) is a synthesis of AD, Riak and SSB evidence, not a system copied whole.

## Bibliography

- [S1] Microsoft Learn, Virtualized Domain Controller Architecture. https://learn.microsoft.com/en-us/windows-server/identity/ad-ds/get-started/virtual-dc/virtualized-domain-controller-architecture
- [S2] Microsoft Learn (archived), Running Domain Controllers in Hyper-V: USN and USN Rollback. https://learn.microsoft.com/en-us/previous-versions/windows/it-pro/windows-server-2008-R2-and-2008/dd363553(v=ws.10)
- [S3] basho/riak_kv PR #1070, per-key actor epoch (kv679). https://github.com/basho/riak_kv/pull/1070
- [S4] Per-Key Actor Epochs In Riak. https://wombat.me/posts/per-key-epochs/
- [S5] Apache Kafka KIP-101. https://cwiki.apache.org/confluence/display/KAFKA/KIP-101+-+Alter+Replication+Protocol+to+use+Leader+Epoch+rather+than+High+Watermark+for+Truncation
- [S6] Apache Kafka KIP-320. https://cwiki.apache.org/confluence/pages/viewpage.action?pageId=87296688
- [S7] Scuttlebutt Protocol Guide. https://ssbc.github.io/scuttlebutt-protocol-guide/
- [S8] ssbc/ssb-db issue #157, network reaction to a forked feed. https://github.com/ssbc/ssb-db/issues/157
- [S9] staltz/manyverse issue #111, restore results in a forked feed. https://github.com/staltz/manyverse/issues/111
- [S10] holepunchto/hypercore README. https://github.com/holepunchto/hypercore/blob/main/README.md
- [S11] syncthing issue #3997, duplicate device ID in a cloned VM. https://github.com/syncthing/syncthing/issues/3997
- [S12] Yjs FAQ. https://docs.yjs.dev/api/faq
- [S13] quarto-dev/q2 PR #689, Automerge actor-id collision self-heal. https://github.com/quarto-dev/q2/pull/689
- [S14] vlcn-io/cr-sqlite issue #145. https://github.com/vlcn-io/cr-sqlite/issues/145
- [S15] SQLite, Transaction. http://www3.sqlite.org/lang_transaction.html
- [S16] SQLite, How To Corrupt An SQLite Database File. https://www.sqlite.org/howtocorrupt.html
- [S17] SQLite, PRAGMA synchronous. https://www.sqlite.org/pragma.html
- [S18] Kulkarni et al., Logical Physical Clocks and Consistent Snapshots (HLC), §4.1. https://cse.buffalo.edu/tech-reports/2014-04.pdf
- [S19] CockroachDB, Transaction Layer. https://docs.cockroachlabs.com/docs/stable/architecture/transaction-layer
- [S20] AWS, Using dead-letter queues in Amazon SQS. https://docs.aws.amazon.com/AWSSimpleQueueService/latest/SQSDeveloperGuide/sqs-dead-letter-queues.html
- [S21] Matrix Server-Server API v1.16. https://spec.matrix.org/v1.16/server-server-api/
- [S22] matrix-org/synapse issue #11373. https://github.com/matrix-org/synapse/issues/11373
- [S23] matrix-org/synapse issue #10424. https://github.com/matrix-org/synapse/issues/10424
- [S24] Apache Kafka commit for KAFKA-18723. https://www.mail-archive.com/commits@kafka.apache.org/msg29019.html
- [S25] Synadia, NATS consumer sequence ahead of stream sequence. https://www.synadia.com/insights/checks/nats-consumer-sequence-ahead-of-stream-sequence
- [S26] superfly/corrosion issue #581. https://github.com/superfly/corrosion/issues/581
- [L1] `docs/superpowers/specs/2026-10-13-cross-node-sync-vision-design.md`
- [L2] `src/protocol/results.rs:395` (`StoreFull`)
- [L3] `src/store/schema.rs:3285` (`check_integrity`)
- [L4] `TRUST-POLICY.md:194` (`operator:local-user:<uid>`)
