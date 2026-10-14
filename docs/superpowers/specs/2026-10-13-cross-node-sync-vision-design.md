# Cross-node threads: vision

## Goal

herdr-threads instances on SSH-linked machines share one thread space: agents on any node send, invite, join and ACK as if all were local, through prolonged partitions (AP; PA/EL), and converge once any path between nodes returns. Sized for 2-10 nodes and ~1-10 msg/s in total.

This is a vision spec: it fixes requirements and the shape of the model so a `/deep-research` pass can work out storage, sync protocol, API and table shapes (see "Research questions"). It amends two earlier positions: the original design's non-goal "No cross-machine federation" (`docs/design/herdr-threads/2026-09-27-herdr-threads-design.md:24`) and TRUST-POLICY's "same local user, same machine" assumption.

## Context: what assumes a single writer today

- `host_instances.decision_seq` is the instance-wide monotonic clock; `threads.next_sequence` allocates a dense, unique per-thread message `sequence`; `*_revision` counters, AUTOINCREMENT ordinals, `binding_cursors.seq`, `membership_intervals.joined_seq/left_seq` and `warning_offer.offered_through_seq` are positions in those orders.
- Public IDs are `<prefix><8 base62>` from CSPRNG bits (~2^47.6), collision-checked only against local tables (`src/protocol/ids.rs`, `src/store/public_ids.rs`).
- An instance is one Herdr server (`instance_dir = sha256(host_endpoint)`); a seat is bound to a local pane.
- Messages, receipts and threads are never deleted; archival only flips a flag.

## Decisions

### Node

A node is a herdr-threads instance (one store, one Herdr server), not a machine. It has a 2-4 character node ID derived from its SSH host key fingerprint; a collision is detected and refused at link handshake.

### Replication scope

Full mesh: every node holds every thread, seat directory entry and message (including archived and pre-link history). Partial replication (directory everywhere, bodies only to nodes with a participant) is a later scaling path, not v1.

### Journal sharded by origin

- Every replicated mutation is a journal entry `(origin_node, counter, hlc, kind, kind_version, payload)`.
- Each node appends only to its own log; `counter` is monotonic per origin. A fact's global identity is `(origin_node, counter)`.
- Every fact has exactly one authoring node: the acting seat's home node (sender for a message, recipient for an ACK, actor for join/leave/accept/reject, inviter for an invitation, home node for seat existence/retirement). Replicated facts therefore never conflict; partitions cause only staleness and late arrival.
- The journal is the source of truth for replicated facts; today's tables become a materialized view of it.
- A node stores and relays every entry, including kinds it does not understand; unknown kinds stay unapplied until an upgrade can replay them.
- Nothing replicated is deleted.

| Replicated (journal) | Node-local (unchanged) |
|---|---|
| thread create, topic, name, archive/reopen | pane bindings, liveness, host observations |
| messages and info/warn timeline events | wake work, delivery ledgers, `offered_through` |
| invitations, accept/reject, join/leave | read cursors, catch-up |
| ACKs and receipt settlement | archival composer sampling |
| seat existence and retirement (not its pane mapping) | handoff/launch topology |

### Multi-writer state

Only thread topic, name and archived flag are written by more than one node; they are last-writer-wins registers on `(hlc, node)`.

### IDs

- Seat, thread, message and invitation IDs embed the origin node ID, so they are unique by construction. Agents read IDs constantly, so the added length stays minimal (2-4 chars).
- A 1:1 thread's ID is deterministic from its sorted node IDs and sorted seat IDs (possibly a fixed-length hash of that tuple). Two sides of a split that each open the pair's thread create the same thread, and on heal both sides' messages simply merge.
- Same ID means same thread, whenever created; different IDs are different threads. Names are labels and may repeat (as today).
- Existing seat IDs are re-keyed with the local node prefix on upgrade; the old IDs remain as aliases that resolve to the new ones.

### Ordering

- Each node shows a thread in local arrival order: the existing dense per-thread `sequence` becomes local ingest order, interleaved by ingest time. A message that arrives late after a partition appears at the bottom, new and unread, with its original send time.
- Entries from one origin are always ingested in counter order (per-origin FIFO). Within an origin nothing is silently lost, only delayed; loss is possible only for facts that never left a node that is gone.
- Different nodes may order the same thread differently. No global order is promised.

### Links

- When herdr-threads observes a Herdr client attaching to a remote server, the local daemon records that SSH destination as a peer; `peer add <ssh-dest>` does the same explicitly (any node may dial any destination it can reach).
- The daemon owns the link: `ssh <dest> herdr-threads peer --stdio`, using the user's ssh config, agent and keys. Herdr clients and servers can come and go.
- Only the dialing side reconnects, with backoff, indefinitely. The dialed side only accepts.
- `peer forget <node>` stops dialing; it deletes nothing. The forgotten node's seats show as remote and unreachable; ACK-required messages to them go overdue normally.

### Sync

- Each node keeps a version vector `{origin -> highest contiguous counter}`.
- Peers exchange vectors on connect and on change; a node behind on an origin pulls the missing range from any linked peer that has it.
- Relay is transitive (store-and-forward): any node serves every origin's log it holds, so any surviving path heals everyone (laptop linking `box1` and `box2` relays between them).
- First contact exchanges full history. Upgrading an existing install backfills its local threads, messages, memberships, invitations and receipts as journal entries authored by the local node.

### Version skew

Peer links negotiate capabilities, never an exact version, and refuse only an incompatible envelope version. Together with relaying unknown kinds, a mixed-version cluster keeps relaying and FIFO intact; an older node just cannot show newer features. This follows the versionless rule in AGENTS.md.

### Trust

- SSH is the only boundary: the same user across SSH-linked nodes, cooperative. No extra admission step.
- TRUST-POLICY.md is amended in the implementing change: remote facts carry a new provenance value (e.g. `replicated_from:<node>`); A2 (decide in the daemon against the canonical view) applies on the authoring node, and receiving nodes apply its facts as given. No adversarial node verification.

### Delivery

- The recipient seat's home node runs the existing delivery policy when a fact is ingested, local or replicated. The delivery contract is unchanged.
- Nudges always deliver, however late; pending nudges for a session coalesce into one wake (e.g. after a reconnect).
- The ACK clock starts at send time on the sender's node. Overdue means overdue regardless of cause: the requested work has not started either way, and the sender must find a workaround.

## Future work (not v1)

- Thread union: a node-authored "t_x merged into t_y" fact (lower ID survives, so concurrent unions converge), e.g. to merge two nodes' separately created `psa-global` after first link.
- Partial replication of message bodies if traffic grows.

## Non-goals

- Consistency stronger than causal-per-origin; global message order.
- Adversarial or untrusted nodes.
- Deleting or compacting replicated data.
- Herdr-level cross-node pane control.

## Research questions (for `/deep-research`)

1. Prior art and pitfalls for per-origin append-only logs with version-vector anti-entropy: Secure Scuttlebutt, Matrix event DAG, CouchDB/PouchDB sequences, cr-sqlite, Automerge sync, Bayou, Dynamo-style anti-entropy, NATS JetStream mirrors, SQLite session extension/changesets, Litestream/LiteFS, Corrosion.
2. SQLite table shapes: journal, per-origin high-water marks, unapplied entries, materialization (triggers vs. apply loop), dual identity (local `sequence` plus `(origin, counter)`).
3. Wire protocol over SSH stdio: framing, vector exchange, range pull, batching, backpressure, resume mid-range.
4. HLC choice and clock-skew bounds for the LWW fields.
5. Node ID derivation and length from the SSH host key; collision detection.
6. Fixed-length encoding of the deterministic 1:1 thread ID.
7. Detecting a Herdr remote attach (hook, event or observation).
8. Migration and backfill of existing data; seat ID aliasing.
9. Failure modes: node restored from backup reusing counters (per-log epoch?), lost journal, corrupt entry.

## Post-Implementation Notes

> *As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
