# Cross-node threads: vision

## Goal

herdr-threads instances on SSH-linked machines share one thread space: agents on any node send, invite, join and ACK as if all were local, through prolonged partitions (AP; PA/EL), and converge once any path between nodes returns. Sized for 2-10 nodes and bursts of ~1-10 msg/s in total.

This is a vision spec: it fixes requirements and the shape of the model. The per-topic research behind the amendments below is in `docs/superpowers/research/2026-10-13-cross-node-sync/` (01 prior art … 09 failure modes); the detailed design builds on both. It amends two earlier positions: the original design's non-goal "No cross-machine federation" (`docs/design/herdr-threads/2026-09-27-herdr-threads-design.md:24`) and TRUST-POLICY's "same local user, same machine" assumption.

## Context: what changes from today

- Today an instance is one Herdr server (`instance_dir = sha256(host_endpoint)`), and its daemon cannot serve without that server; a seat is bound to a local pane.
- `host_instances.decision_seq`, `threads.next_sequence`, `*_revision` counters, AUTOINCREMENT ordinals, cursors and `offered_through` all assume a single writer. They stay as local positions, assigned at apply time and never replicated.
- Public IDs are `<prefix><8 base62>`, unique only by probability and only checked locally.
- **Clean start:** cross-node herdr-threads starts from a fresh store; the existing database is wiped. No migration, backfill, legacy IDs or aliases.

## Decisions

### Node

- A node is one herdr-threads daemon and store per machine and OS user. It serves every Herdr server of that user on the machine and runs (and syncs) with no Herdr server up. Seats record which Herdr server (host endpoint) their pane belongs to.
- Identity is an Ed25519 keypair minted with the store. `node_uuid` = hash of the public key. A 3-character base62 `node_tag` (collision odds ~2e-4 at 10 nodes) prefixes public IDs; it is display/ID material only, never the replication key. On a tag collision the node with the higher `node_uuid` picks a new tag; IDs already minted stay valid.
- Not derived from the SSH host key: it is ambiguous (several key types), rotates, is shared by cloned VMs, and absent on some machines.

### Replication scope

Full mesh: every node holds every thread, seat directory entry and message, including archived threads. Partial replication (directory everywhere, bodies only to nodes with a participant) is a later scaling path.

### Journal sharded by origin

- Every replicated mutation is a signed journal entry: `origin (node_uuid), incarnation, counter, prev_hash, hlc, deps (author's applied version vector at write time), kind, kind_version, payload, signature`.
- Each node appends only to its own log; `counter` is gapless and monotonic per `(origin, incarnation)`. `prev_hash` chains each origin's log.
- Every fact has exactly one authoring node: the acting seat's home node (sender for a message, recipient for an ACK, actor for join/leave/accept/reject, inviter for an invitation, home node for seat existence/retirement). Replicated facts never conflict; partitions cause only staleness and late arrival.
- The journal is the source of truth for replicated facts; the existing tables become a materialized view of it.
- Nothing replicated is deleted. Storage is sized for bursty agent traffic; a sustained 10 msg/s for a year would be ~0.3-0.9 TB per node, which is outside the expected load.

| Replicated (journal) | Node-local |
|---|---|
| thread create, topic, name, archive/reopen | pane bindings, liveness, host observations |
| messages and info/warn timeline events, each with its recipient set | wake work, delivery ledgers, `offered_through` |
| invitations, accept/reject, join/leave | read cursors, catch-up |
| ACKs and receipt settlement | archival composer sampling |
| seat existence and retirement (not its pane mapping) | handoff/launch topology |

### Ingest and apply

- **Ingest** verifies only the signature and the chain (`prev_hash` matches the held head of that origin), then stores and relays the entry. It never checks references, so relay never waits on apply.
- **Apply** is causal: an entry is applied once the node's applied vector covers the entry's `deps` vector. An entry from B that references a thread A created waits until A's log is applied up to the create, without stalling B's relay.
- An entry of an unknown kind, or one that fails to apply, is parked (counts as processed for causal purposes, its effects absent) and replayed after an upgrade.
- Apply is idempotent by fact ID, and the vector advances in the same transaction that commits the entries.
- The author's node fixes each message's recipient set in the payload; receiving nodes do not recompute it from their own membership view.

### Multi-writer state

Only thread topic, name and archived flag are written by more than one node: last-writer-wins registers on `(hlc, node_uuid)`. A local write takes `max(clock, register.hlc + 1)`, so a far-future value cannot block later edits.

### Clock

An in-tree HLC packed into a u64 (48-bit Unix ms + 16-bit counter). A replicated entry is never rejected for a far-ahead HLC; the local clock follows a remote HLC only within a bounded skew (e.g. 60 s), larger skew is measured at handshake and warned about. On startup the clock resumes from the journal's maximum HLC.

### IDs

- Public IDs: kind letter + `node_tag` + 8 base62 (12 chars), unique by construction.
- A 1:1 thread's ID is `d` + 10 base62 of SHA-256 over the domain-tagged, sorted pair of seat IDs (11 chars). Both sides of a split that open the pair's thread create the same thread; on heal their messages merge. Concurrent creates are idempotent: the earliest `(hlc, node_uuid)` is the creator, a default topic does not write the LWW register, crossed invites settle as already joined without a nudge.
- Same ID means same thread; different IDs are different threads. Names are labels and may repeat.

### Ordering

- Each node shows a thread in local arrival order (the dense per-thread `sequence`, assigned at apply), interleaved by ingest time. A late message appears at the bottom, new and unread, with its original send time.
- Entries from one origin are ingested in counter order. Within an origin nothing is silently lost, only delayed.
- Different nodes may order the same thread differently. No global order is promised.

### Sync

- A sync exchange carries each side's full version vector with head hashes: `{(origin, incarnation) -> (counter, head_hash)}`. "Here is my vector; send what you have newer, and I'll send what I have newer than yours."
- Relay is transitive (store-and-forward): any node serves every origin's log it holds, so any surviving path heals everyone.
- Each origin is pulled from one peer at a time, in bounded batches; caught-up links stay subscribed for live entries. Duplicates are dropped by `(origin, incarnation, counter)`, so relay loops cannot amplify.
- First contact exchanges full history, chunked and resumable.

### Forks (restore from backup)

- Detection: for each origin in an exchange, compare hashes at the lower of the two counters; a mismatch, or an ingested entry whose `prev_hash` is not the held head, is a fork. Two validly signed different entries at one `(origin, incarnation, counter)` prove that the key holder forked its log.
- Resolution belongs to the forking node, which sees on contact a branch of its own log it does not hold: it adopts the peers' branch (recovering entries lost in the restore), bumps its incarnation, and re-signs and republishes its divergent entries under the new incarnation. Dedupe by fact ID prevents duplicates. Other nodes keep the branch they already had and hold the conflicting one until the owner republishes.
- Operators get inspect/verify/resync/quarantine commands for an origin.

### Links

- Peers are discovered by polling `herdr machine list --json` (Herdr has no attach hook or event) and added explicitly with `peer add <ssh-dest>`.
- The daemon owns the link: `ssh <dest> herdr-threads peer --stdio`, using the user's ssh config, agent and keys, non-interactive, without connection sharing, with keepalives. The remote `peer --stdio` is a stateless pump into the remote daemon, starting it if needed; the daemon stays the only SQLite writer.
- Handshake: mutual challenge-response with node keys, envelope version, capabilities (unknown ones ignored), typed refusals (envelope, tag/uuid collision, self-link, busy). Duplicate links to one node are deduplicated by `node_uuid`.
- Only the dialing side reconnects, with jittered backoff, indefinitely. `peer forget <node>` stops dialing and deletes nothing; that node's seats show as remote and unreachable.

### Version skew

Peer links negotiate capabilities, never an exact version, and refuse only an incompatible envelope version. Unknown kinds are relayed and parked, so a mixed-version mesh keeps relaying and per-origin order intact. This follows the versionless rule in AGENTS.md.

### Trust

- SSH is the link boundary: the same user across SSH-linked nodes, cooperative. No adversarial verification of local callers.
- Node-level authenticity is new: every entry is signed by its origin node and verified by every receiver, however it was relayed, so a link into the mesh cannot forge another node's facts. Root on a machine can steal its key; that is out of scope.
- TRUST-POLICY.md is amended in the implementing change: origin signatures as an invariant, a provenance value for replicated facts (e.g. `replicated_from:<node>`), and A2 applied on the authoring node, with receivers applying its signed facts as given.

### Delivery

- The recipient seat's home node runs the existing delivery policy when a fact is applied, local or replicated. The delivery contract is unchanged.
- Nudges always deliver, however late; pending nudges for a session coalesce into one wake, including during bulk catch-up.
- The ACK clock starts at send time on the sender's node, and the sender's node decides overdue. Overdue means overdue regardless of cause. A node waking from sleep holds overdue checks until its first sync.

## Future work (not v1)

- Thread union: a node-authored "t_x merged into t_y" fact (lower ID survives, so concurrent unions converge), e.g. to merge two separately created `psa-global` threads.
- Partial replication of message bodies if traffic grows.

## Non-goals

- Consistency stronger than causal; global message order.
- Protection against a compromised node's own key or root on a node.
- Deleting or compacting replicated data.
- Migrating pre-existing stores.
- Herdr-level cross-node pane control.

## Post-Implementation Notes

> *As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
