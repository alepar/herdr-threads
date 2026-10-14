R1 One node per machine+OS user: the daemon and store are addressed by state dir only, run and sync with zero Herdr servers attached, and serve several Herdr servers at once.
R2 Node identity: an Ed25519 key, node_uuid and 3-char tag (collision resolved by deterministic retag); public IDs carry the tag; 1:1 thread IDs are deterministic from the two seat IDs.
R3 Every replicated mutation is authored as a signed, hash-chained per-origin journal entry and materialized by one apply path for local and remote facts.
R4 Ingest verifies signature and chain and relays entries without reading payloads; apply is causal (deps vector), idempotent by fact ID, parks unknown or failed entries and replays them after an upgrade.
R5 Agents on different nodes can create and join threads, invite and answer, send, receive and ACK each other's messages, with LWW topic, name and archived flag.
R6 Remote facts reach local recipients through the existing delivery contract (lazy by default, nudges coalesce), and already-settled history never wakes anyone.
R7 ACK and invitation overdue are decided only on the deciding node from send time, held after a wall-clock jump until the first sync round.
R8 Peer links run over the user's own ssh (peer --stdio pump) with a mutually authenticated handshake, version-vector anti-entropy, transitive relay, live tail, credit backpressure, keepalive and reconnect backoff, independent of any Herdr client.
R9 After partitions of any length every node converges to the same fact set once any path returns, including a direct pair opened on both sides.
R10 Forks and restores are detected and repaired (frozen incarnation, re-issue of the union, wipe pull-back, corrupt bytes penalize the source, operator resync).
R11 Peers are discovered (herdr machine list, process table) or added explicitly, and operators have peer add/forget/list, sync status/verify/unapplied/resync and legacy purge.
R12 Remote seats exist without panes, are excluded from node-local workers, unreachable home nodes show one compact marker, and automatic archival skips threads with remote members.
R13 The store starts clean; legacy instance data is reported and purgeable but never opened.
R14 TRUST-POLICY is amended (origin signatures, replicated provenance, accepted limits) and the agent skill and user docs describe cross-node use.
R15 Mixed-version nodes interoperate: link capabilities are negotiated and unknown kinds are relayed and parked.
R16 A multi-node test harness exists and each enumerated configuration (2-node, 3-node relay, partition+heal, restore fork, mixed versions, store-only node, two Herdr hosts) is exercised end to end.
