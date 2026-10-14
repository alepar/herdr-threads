# Cross-node herdr-threads sync — design

Seed: [vision spec](../../specs/2026-10-13-cross-node-sync-vision-design.md) (settled decisions, approved by the user 2026-10-13/14) and research [01-09](../../research/2026-10-13-cross-node-sync/). This spec makes those decisions buildable: it fills the gaps the vision left, fixes concrete shapes, and is the root of the run's task tree.

## Goal

herdr-threads daemons on SSH-linked machines (one node per machine and OS user) share one thread space: an agent on any node can create and join threads, invite, send, receive and ACK with seats on any other node, through partitions of any length, and every node converges to the same set of facts once any path between them returns, with no dependence on a Herdr client or server being up.

## Problem description

herdr-threads today is one daemon and one SQLite store per Herdr server, on one machine. Every write is decided by that daemon against its own canonical view, every ordering counter assumes a single writer, and identity (instance, seat, pane) is local. Herdr's own remote support lets a client see remote panes only while the client is attached, so agents on different machines cannot message each other durably. The user wants cross-machine messaging that keeps working through long disconnections (AP, PA/EL), at 2-10 nodes and bursts of ~1-10 msg/s, with SQLite kept as local storage.

## Main challenges

- **Single-writer assumptions everywhere.** `decision_seq`, `threads.next_sequence`, `*_revision`, cursors, `offered_through` and AUTOINCREMENT ordinals all assume one writer. They must become purely local positions while the replicated facts get a separate global identity.
- **Identity is per Herdr server, not per machine.** The store, socket and instance UUID are keyed by `sha256(host_endpoint)`; the daemon cannot run without its Herdr server, and a machine with several Herdr servers is several instances. The vision requires one node per machine+user that runs and syncs with zero Herdr servers up.
- **Facts arrive late, out of cross-origin order, and possibly twice.** Transitive relay pulls each origin independently, so an entry can arrive before the entities it references. Apply must be causal, idempotent and must never stall relay.
- **Restore, clones and corruption.** A node restored from backup reuses counters; cloned VMs share identity. These must be detected and healed without losing facts.
- **The delivery contract must survive replication.** Remote facts must wake/notify exactly as local ones do (lazy by default, nudges only for idle sessions, coalesced), and catch-up of settled history must not wake anyone.
- **Every write path changes.** Thread create, direct pairs, invite/accept/reject, join/leave, send, ACK, topic/name/archive and seat lifecycle each need an authoring side (decide, build a fact) and an apply side (materialize a fact from any origin).

## Key decisions made

The vision's decisions stand (node = machine+user with an Ed25519 identity; signed hash-chained per-origin journal; causal apply via deps vectors; version-vector anti-entropy with transitive relay; fork recovery by incarnation; clean-start store; deterministic 1:1 IDs; local arrival order; LWW only for topic/name/archived; nudges coalesce; ACK clock on the sender). This spec adds: the journal is the source of truth and **every** replicated mutation, local or remote, is materialized by one `apply(entry)` path; derived timeline events (info/warn) are **not** replicated, each node derives them on apply; overdue decisions stay with the deciding (sender's or inviter's) node; remote seats are directory rows with no pane; automatic archival never touches a thread with a remote member; the peer link is a stateless `peer --stdio` pump into the remote daemon over the user's own `ssh`; and a fork freezes the old incarnation at the last common counter while the owner republishes the union of every branch's tail under a new incarnation.

## Non-goals

- Consistency stronger than causal; a global message order.
- Protection against a node's own compromised key or root on a node.
- Deleting, compacting or partially replicating data (partial replication is a later scaling path).
- Migrating pre-existing stores (clean start).
- Herdr-level cross-node pane control; a node never wakes or types into another node's panes.
- Thread union (future work in the vision).
- OS service installation (launchd/systemd) for the daemon; a node daemon starts on local use or on an inbound peer link.

## Design

### 1. Node, store and daemon layout

**Decision.** A node is one daemon and store per `(machine, OS user, state dir)`. The node lives in `<state_dir>/node/`:

| File | Content |
|---|---|
| `node.key` | Ed25519 secret key, mode 0600, created once; never rewritten |
| `node.json` | `{node_uuid, tag, created_at}` (derived, regenerable from the key plus the store's tag record) |
| `herdr-threads.db` | the store (new file; clean start) |
| `daemon.sock`, `owner.lock`, logs, settings | as today under the instance dir |

- `node_uuid` = lowercase hex of the first 16 bytes of SHA-256(public key). It is the node's replication identity and never changes while the key exists.
- `daemon run --state-dir <dir>` no longer requires `--host-endpoint`. The CLI selects the daemon by state dir only (flag, `HERDR_PLUGIN_STATE_DIR`, default). The host endpoint becomes part of the caller's claim (§3), not the daemon's address.
- The daemon runs with zero Herdr servers attached and keeps running while it has a peer link or a dial target, even if no Herdr server is reachable (`store-only` mode). Liveness/idle-exit rules that today end the daemon when its host goes away are replaced by: exit only on explicit stop or when it has no local clients, no attached host, no peer links and no dial targets for the existing idle period.
- **Clean start.** The new daemon never opens `<state_dir>/instances/`. It is left in place; `doctor` reports it as legacy data with the removal command (`herdr-threads legacy purge`, operator-gated, which deletes only that directory). Hooks installed by an older `setup` keep working because `--state-dir` still resolves the node; `--host-endpoint` on old hook argv is accepted and used as the claim's host.

**Considered.** Keeping one node per Herdr server (rejected by the user: sync would depend on a Herdr server being up, and one machine would be several nodes). Deriving identity from the SSH host key (rejected: ambiguous key types, rotation, clones, absent on some machines — research 05). Auto-deleting the legacy directory (rejected: destructive; an explicit purge costs nothing).

### 2. Node identity, tags and public IDs

**Decision.**
- `tag`: 3 base62 characters (alphabet `0-9A-Za-z`, case-sensitive), `tag_k = base62(SHA-256(node_uuid ‖ k))[0..3]` with `k = 0` first. Collision odds ≈2e-4 at 10 nodes.
- A tag collision (two genesis records with the same tag and different `node_uuid`, seen by any node, including through relay) is resolved deterministically: the node with the **higher** `node_uuid` moves to the next `k` and authors a `node.retag` fact. IDs it minted under the old tag stay valid forever (they are opaque strings); only newly minted IDs use the new tag. Both nodes resolve the collision without coordination.
- Public IDs: `<kind prefix><tag><8 base62>` — e.g. a seat `sK3qAb3k3Fq` (prefix `s`, tag `K3q`). Prefixes are today's (`s`, `t`, `i`, `m`, `n`, `p`, …). Uniqueness across nodes is by construction (tag) plus the existing local collision check; a residual collision within one tag is impossible because only one node mints under a tag at a time.
- Direct (1:1) thread IDs: `d` + 10 base62 characters of SHA-256(`"herdr-threads/direct/v1" ‖ len‖seat_lo ‖ len‖seat_hi`) where `seat_lo < seat_hi` bytewise (research 06). Node IDs are not hashed in (seat IDs already embed their tag, and a re-tag must not split a pair). The ID parser accepts `d…` wherever a thread ID is accepted.

**Considered.** 2-character tags (1.2% collision at 10 nodes, rejected); raw concatenation for 1:1 IDs (~28 chars of model-visible text, rejected); `node_uuid` inside 1:1 IDs (rejected per research 06).

### 3. Multiple Herdr servers per node

**Decision.**
- The schema splits today's `host_instances` responsibilities: **node scope** (`decision_seq`, `send_eligibility_revision`, `duration_config_revision`, filter revisions, thread/message ownership) stays on a single node row; **Herdr-server scope** (boot, epoch, observation and admission sequences, snapshot generations, recovery baseline, `lifecycle_revision`, reachability) moves to a new `herdr_hosts` table keyed by a node-local `host_id`, with the host endpoint locator as a unique attribute.
- Seats that sit in a pane carry `host_id`; `seats_live_target` becomes unique on `(host_id, target_id)`.
- The daemon runs one **host session** per attached Herdr server: its `NativeCli`, reachability flag, observation worker, wake dispatcher, archival sampler and handoff namespace. A host session is created when a caller's claim names a host endpoint the daemon has not attached yet, or at startup for every `herdr_hosts` row whose endpoint socket exists. A host session that cannot reach its server marks that host unreachable without affecting other hosts or sync.
- `CallerClaim` gains the host locator; store entry points check `claim.host` against an attached host instead of `claim.instance == self.instance`.
- `HostPort` methods take a host selector (or are reached through a per-host handle); the wake dispatcher routes by the seat's `host_id`.

**Considered.** Supporting only one Herdr server per node in v1 (rejected: the node-per-machine decision means every user with two Herdr sessions would otherwise lose one).

### 4. Journal, envelope, signing and clocks

**Decision.** Tables (clean-start schema; exact DDL is the journal-core subepic's, following research 02 §2):
- `sync_logs(log_ix PK, node_uuid, incarnation, is_self, contiguous_counter, head_hash, applied_through, blocked_on, state ∈ {active, frozen}, frozen_at_counter, genesis_hash, pubkey)` — one row per `(node_uuid, incarnation)`; the vector is `{(node_uuid, incarnation) → (contiguous_counter, head_hash)}`.
- `journal(ingest_seq PK AUTOINCREMENT, log_ix, counter, prev_hash, hash, hlc, kind, kind_version, deps BLOB, creates, needs, payload BLOB, signature, apply_state ∈ {applied, pending, parked_unknown, parked_dep, failed}, applied_seq, apply_error, UNIQUE(log_ix, counter))`, append-only (triggers forbid UPDATE of envelope columns and DELETE).
- `journal_forked(...)` — same envelope columns; holds entries of a frozen incarnation beyond its frozen counter (nothing is deleted).
- `thread_registers(thread_id, field ∈ {topic, name, archived}, value, hlc, node_uuid, PK(thread_id, field))` — no FK to threads.

**Envelope.** `origin = (node_uuid, incarnation)`, `counter` (gapless from 1 per log), `prev_hash` (32 zero bytes at counter 1), `hlc`, `kind`, `kind_version`, `deps` (the author's applied vector at write time: `{log → counter}` over every log it has applied, encoded compactly), `creates` (entity ID created, optional), `needs` (entity IDs referenced, list), `payload` (exact authored JSON bytes, ≤512 KiB), `hash`, `signature`.
- `hash = SHA-256(canonical_bytes)` where canonical bytes are a fixed, length-prefixed binary concatenation of every envelope field except `hash` and `signature` (not JSON canonicalization). `signature = Ed25519(node key, hash)`.
- Counter 1 of every incarnation is a `node.genesis` entry carrying the public key, tag, incarnation, predecessor incarnation (if any) and a display host label. A receiver verifies `SHA-256(pubkey)[0..16] == node_uuid` there, then every later entry's signature with that key.

**HLC.** In-tree, u64 = 48-bit Unix ms ‖ 16-bit logical counter (carry into ms on overflow). Local events take `max(wall, last+1)`; ingest folds a remote HLC into the local clock only when it is ≤60 s ahead of wall time; larger skew is stored, accepted, and reported (`sync status`, log warning). On startup the clock resumes from `max(wall, max HLC in the self log)`. The ACK/invitation deadline clock is the deciding node's wall clock (§7).

**Considered.** `uhlc` crate (rejected by research 04: small in-tree code, no extra dependency); JSON canonicalization for hashing (rejected: fragile across serde versions); per-fact dependency lists only (rejected for deps vectors per the user: one full vector is simpler and works for unknown kinds; `needs` is kept as a precise, kind-agnostic block reason).

### 5. Ingest and apply

**Decision.**
- **Ingest** (per batch, one `BEGIN IMMEDIATE` transaction, ≤256 entries/≤256 KiB): for each entry, in counter order per log: the counter is `contiguous+1` (else drop the rest of the batch and re-request), `prev_hash == head_hash` (else §10 fork), hash recomputes, signature verifies (a genesis entry establishes the key). Store with `apply_state = pending`, advance `contiguous_counter/head_hash` in the same transaction. Ingest never looks at payloads, so unknown kinds relay unchanged. Serving to peers only reads committed rows; the writer runs with `synchronous=FULL`.
- **Apply** (separate loop in the writer, after ingest commits and after every local append): an entry is ready when its log's `applied_through = counter-1` and the local applied vector covers its `deps`. It is applied in its own SAVEPOINT by the kind's handler. Outcomes: `applied`; `parked_unknown` (unknown kind or newer major `kind_version`), which counts as processed for causality; `parked_dep` (an entity in `needs` was created by a parked entry); `failed` (handler error, recorded with a bounded message, counts as processed). Parked/failed entries are replayed in `ingest_seq` order after an upgrade (startup pass) and by `sync unapplied replay`.
- **Local authoring uses the same path.** A local command decides (A2) in its transaction, appends the self-log entry, and applies it through the same handler in the same transaction, so local and remote materialization cannot drift. Test oracle: replaying the journal into an empty store reproduces every replicated table.
- **Idempotent by fact ID.** Handlers key on public fact IDs (message, invitation, seat, thread, receipt `(message, seat)`), so a fact re-issued under a new incarnation (§10) or authored twice (direct-pair create) is a no-op the second time. Natural-key UNIQUE constraints stay as loud guards; only the LWW register upsert uses `ON CONFLICT`.
- **Local positions** (`messages.sequence`, `decision_seq`, `*_revision`, ordinals, cursors, membership interval seqs) are assigned at apply. `messages.sent_at` is the origin's HLC time; the local arrival order is the apply order.

**Considered.** Triggers for materialization (rejected: cannot run Rust delivery policy or park unknown kinds — research 02 §3). Rejecting entries with dangling references at ingest (rejected: would stall the origin's relay; the user accepted causal apply).

### 6. Fact kinds (v1)

Every replicated mutation is one of these kinds. Derived timeline events (info: joined, left, topic changed, archived…; warn: overdue, rejection) are **not** facts: each node's apply handler appends them locally with the existing event writers, so they get local sequences and cost no replication.

| Kind | Authored by | Payload (fact IDs and origin data only) | Apply effect |
|---|---|---|---|
| `node.genesis` | each node, counter 1 of each incarnation | pubkey, tag, incarnation, predecessor, host label | `sync_logs` key/genesis; node directory row |
| `node.retag` | the higher-uuid node of a tag collision | new tag, k | node directory tag |
| `seat.created` | seat's home node | seat ID, role (agent/human), display name, created hlc | local seat (if home) or remote seat directory row |
| `seat.retired` | seat's home node | seat ID, reason | seat retired; its open invitations/receipts settle `recipient_retired` on every node |
| `thread.created` | creator's home node | thread ID, name, topic, goal, creator seat | thread row; initial register values via the register upsert with this entry's HLC |
| `thread.direct` | either pair seat's home node | `d…` thread ID, seat_lo, seat_hi | idempotent create; creator = earliest `(hlc, node_uuid)`; default topic is not written to the register |
| `thread.register` | any node whose local seat set topic/name/archived | thread ID, field, value | LWW upsert on `(hlc, node_uuid)`; local write uses `max(clock, register.hlc+1)`; winner copied to `threads` |
| `invitation.created` | inviter's home node | invitation ID, thread, inviter, invitee, deadline_at, content, required flag | invitation row; invitee's home node runs delivery; if the invitee is already joined, settles `already_joined` with no wake |
| `invitation.answered` | invitee's home node | invitation ID, accepted/rejected, reason | invitation state; accept also creates the invitee's membership |
| `membership.joined` / `membership.left` | the seat's home node | thread, seat | membership row and local interval |
| `message.sent` | sender's home node | message ID, thread, author seat, body, flags (lazy/nudge/require_ack), recipients `[{seat, receipt: none\|ack, deadline_at}]`, sent_at | message row; for each **local** recipient the existing delivery tail (§7); remote recipients only get receipt rows |
| `receipt.settled` | recipient's home node | message ID, seat, how (acked, displayed, mod_delivered, policy, retired), settled_at | receipt state on every node; the sender's node clears pending overdue tracking |

- The **author fixes the audience**: `message.sent` carries the recipient set and per-recipient receipt obligation decided by the sender's node from its membership view. Receivers never recompute it.
- Membership transitions of one seat are authored only by that seat's home node, so they are totally ordered per seat; an invitation is the inviter's fact and its answer is the invitee's.
- Receivers accept facts the author's view allowed even if their own view differs (e.g. a message into a thread they see as archived is stored and delivered; archive does not reject posts on apply).
- Each kind has `kind_version = 1`. An additive payload field is a minor change (older handlers ignore unknown fields); a breaking change bumps the major and older nodes park it.

**Considered.** Replicating info/warn events (rejected: 73% of timeline rows today, all derivable); recomputing recipients on each node (rejected: membership views diverge — research 02 R5).

### 7. Delivery, deadlines and catch-up

**Decision.**
- **Local recipients of a replicated fact** go through the same tail as a local send: `send_manifests` + prepared/lazy recipient rows + the `send_attention` work job + `lazy_delivery::note_published` + `wake_work`, all written by the apply handler so the table-driven lane kicks (`service/kicks.rs`) fire as they do today. Invitations to local invitees write the same invitation attention rows. The delivery contract is unchanged; pending nudges for one seat coalesce into one wake, including during bulk catch-up.
- **Catch-up suppression.** When an apply finds the fact already settled for a local recipient (a `receipt.settled` or `invitation.answered` from the recipient's home node — which is this node — is already in the journal), no attention, wake or receipt work is created. Facts ingested as part of a node's own old incarnation (wipe recovery, §10) are applied with delivery suppressed for receipts that log already settled.
- **Deadlines.** The sender's node computes `deadline_at` from its wall clock at send and carries it in `message.sent`; only the sender's node scans receipts it authored for overdue and emits the overdue warning to its sender seat. The inviter's node does the same for invitation deadlines. Recipient nodes never emit overdue for remote senders. A node resuming from sleep (wall clock jumped by more than the deadline-lane period) holds overdue emission until it completes its first sync round with every linked peer or 60 s pass, whichever comes first.
- **Remote seats** never get wake work, liveness, composer sampling or binding rows locally.

**Considered.** Re-delivering late history as new work (rejected: wakes every seat after a wipe — research 09 R3).

### 8. Remote seats, threads and archival

**Decision.**
- `seats` gains `home_node` (node_uuid; equal to self for local seats) and a `remote` state. Remote seat rows have `host_id`, `target_id`, structural columns and bindings NULL; observation, reconciliation, liveness and retirement-by-observation skip them. Only `seat.retired` from the home node retires a remote seat.
- Thread and message rows are node-scoped (the node row), not host-scoped.
- **Automatic archival** decides only for threads whose every non-retired member is a local seat; threads with a remote member are archived only explicitly (`thread.register archived`). Explicit archive/reopen replicate as LWW.
- Agent-visible output stays ID-based: seat IDs embed the tag, so no extra marker is printed per row. `thread participants` and `thread show` add one compact `node <tag> unreachable since <time>` line per unreachable home node (not per seat).

**Considered.** Sampling remote members' idleness via replicated activity (rejected for v1: needs a new replicated signal; explicit archive suffices).

### 9. Peer links

**Decision.**
- **Remote side:** `herdr-threads peer --stdio [--state-dir X]` resolves the node dir, `ensure_running`s the daemon (store-only is fine), connects to `daemon.sock`, writes `HTPEER1\n` to stdout, then splices stdio ⇄ socket with raw fd copies until EOF on either side or a write error. It keeps no state. The daemon recognizes a peer connection by its first frame and admits it under `MAX_PEER_CONNECTIONS = 16`; the existing peer-uid check applies.
- **Dialing side:** the daemon spawns `ssh -o BatchMode=yes -o ServerAliveInterval=15 -o ServerAliveCountMax=3 -o ControlMaster=no -o ControlPath=none -T <dest> '<remote command>'` as an owned child (own process group; killed and reaped on link teardown). The remote command defaults to `exec "$(command -v herdr-threads || echo "$HOME/.local/bin/herdr-threads")" peer --stdio`, overridable per dial target (`remote_command`). Tests and power users can replace the whole transport command with `sync.dial_command` (a template with `{dest}`), which is how multi-node tests run without real ssh.
- **Framing:** u32 BE length + JSON, 1 MiB cap, after the `HTPEER1\n` preamble (the dialer skips ≤64 KiB of leading noise and reports it).
- **Handshake:** both sides send `PeerHello{envelope: 1, node_uuid, tag, pubkey, caps, limits, nonce}`; each answers with `PeerAuth{signature over (own hello, peer hello, peer nonce)}`; then `PeerAccept` or typed `Close{reason ∈ envelope_unsupported, node_collision, self_link, busy, auth_failed}`. `envelope_unsupported`, `self_link` and `auth_failed` stop dialing that target until it is changed; `busy` retries. Unknown caps are ignored.
- **Sync:** each side sends `Have{(node_uuid, incarnation) → (counter, head_hash)}` (full vector; partial after reconnect is an optional cap). For every log where the peer is ahead, the receiver sends `Want{log, from}` to exactly one link (the `rx` source: the origin itself if linked, else the peer with the highest counter; fail over after 30 s without progress). The sender streams `Entries{log, entries}` batches (≤256 entries/≤256 KiB, ≥1) in counter order under a credit window (`Credit{bytes}`, 1 MiB, returned after commit). A satisfied `Want` stays subscribed and new appends are pushed as ingested (live tail). `Have` deltas go to all links after ingest, throttled to ~1/s. Duplicates at or below `contiguous` are dropped, so relay cycles cannot amplify.
- **Liveness:** app `Ping`/`Pong` every 30 s of silence; dead after 90 s without an inbound frame; reconnect with full-jitter backoff (base 1 s, cap 5 min, reset after 60 s up). Only the dialer reconnects.
- **Duplicate links:** peers are identified by `node_uuid`; when two links connect the same pair, the one dialed by the lower `node_uuid` stays active and the other idles (pings only) as standby.

**Considered.** A sync endpoint that opens SQLite outside the daemon (rejected: two writers, duplicated delivery); a TCP listener (rejected: new auth surface); reusing Herdr's ControlMaster (rejected: internal to Herdr); CBOR/postcard (rejected: 10-30% saving, worse debuggability — research 03 §6).

### 10. Forks, restore and corruption

**Decision.**
- **Detection.** (a) At handshake/`Have`: a peer advertises my own `(node_uuid, incarnation)` at a counter above mine, or a different `head_hash` at a counter both hold (the peer further ahead compares its stored hash at the shorter counter). (b) At ingest: an entry whose `prev_hash` differs from the held hash at `counter-1` for an otherwise valid signature. Two validly signed different entries at one `(origin, incarnation, counter)` are proof the key holder forked the log.
- **Third-party nodes** keep the branch they hold; on detection they mark the log `frozen` at the last common counter, store the conflicting entries in `journal_forked`, stop pulling that log beyond the frozen counter, and record fork evidence (both conflicting entries) that `sync status` shows and links forward in `Have` so other nodes freeze too.
- **The owner** (detects (a) about itself, or receives fork evidence about its own log) mints a new incarnation (genesis naming the predecessor and the frozen counter), pulls every branch of the old incarnation's tail from peers into `journal_forked`, and re-issues under the new incarnation, in original order, every fact past the frozen counter from **all** branches (its own and the peers'), each re-signed with its original public fact IDs. Idempotent apply makes re-issues no-ops where already applied, so every node ends with the union of facts and the old incarnation frozen everywhere. The owner never authors under the old incarnation again.
- **Restore without a fork** (peers hold my log beyond my head with matching hashes): rotate incarnation, pull my old tail back and apply it with delivery suppression (§7). A wiped store with the key intact is the same case from counter 0. A node whose key was also lost is a new node; its old seats stay as remote seats of the old node.
- **Corrupt bytes** (hash or signature failure): reject the batch, count an error against that peer for that log, prefer another source. Local DB corruption keeps today's `StoreCorrupt` path; `sync resync --from <peer>` rebuilds under a new incarnation after a `VACUUM INTO` backup.
- **Disk full:** ingest stops below a free-space headroom (default 256 MiB) so local authoring keeps working; batches are atomic, so a failed batch leaves the vector unadvanced.

**Considered.** First-seen-wins (rejected: permanent divergence, SSB); operator-only re-issue in v1 (rejected: the user's vision has the forking node resolve it; automatic re-issue is deterministic and idempotent).

### 11. Discovery and operator commands

**Decision.**
- **Dial targets** (`sync_dial_targets`): `ssh_dest` (verbatim ssh destination), source (`explicit` | `herdr_machine` | `herdr_remote_process`), enabled, learned `node_uuid`, backoff state, last error, `remote_command`.
- **Discovery:** while at least one Herdr host is attached, the daemon polls `herdr machine list --json` every 60 s and upserts every enabled machine's `target` as a `herdr_machine` dial target; it also adds `herdr --remote <target>` clients seen in the existing process-table scan. Discovery is on by default and disabled by `sync.discover = false`. A target whose remote binary is missing (exit 127 / not found) is shown as `not installed` and retried every 30 min instead of the normal backoff.
- **CLI** (all compact, one row per item, ids not prose): `peer add <ssh-dest> [--remote-command C]`, `peer forget <dest|node>` (stops dialing; deletes nothing), `peer list`, `sync status [--log <node>]` (per log: counter, head hash prefix, applied_through, parked/failed counts, frozen state, lag, skew; per link: state, rx logs, last error; store size and headroom), `sync verify` (re-hash the self chain and every stored chain), `sync unapplied [list|replay]`, `sync resync --from <node> [--log <node>]`, `legacy purge`. Mutating commands are operator-gated and record `operator:local-user:<uid>` provenance.

**Considered.** Watching Herdr's internal `endpoints.json` (rejected: internal file); requiring opt-in per machine (rejected: the user wants automatic linking; the kill switch and `peer forget` cover opt-out).

### 12. Trust policy

TRUST-POLICY.md is amended in the trust bead (same commit as the first code that relies on it): node identity and key custody; origin signatures verified by every receiver as an invariant; provenance `replicated_from:<node_uuid>` for materialized remote facts; A2 applies on the authoring node and receivers apply signed facts as given; a remote seat is never moved, merged or retired except by its home node's `seat.retired`; fork quarantine and re-issue as defined repairs; accepted limits (root on a node can steal its key; clock skew affects LWW and shown times only).

### 13. Testing and configurations

- **Multi-node harness** (test-support): N nodes in one test, each its own state dir and fake Herdr host (`FakeHost`), linked through `sync.dial_command` pointing at a shim that runs `herdr-threads peer --stdio --state-dir <other>` directly; a partition is the shim refusing/killing links. Every daemon and shim is spawned through `test_support::spawn` and stopped at test end.
- **Configuration smoke** (enumerated configurations, each run once end to end): 2 nodes direct; 3 nodes in a line A–B–C (relay); partition and heal with writes on both sides (direct pair created on both sides); node restored from a store backup (fork, re-issue); mixed versions (an unknown kind injected and relayed through a node that parks it); zero Herdr hosts on one node (store-only); two Herdr hosts on one node.
- The pre-release wake e2e stays single-node; a cross-node wake case is follow-on work outside this run.

## Follow-on (outside this tree)

- A real two-machine run over real ssh (needs two machines; operational).
- Cross-node cases in the pre-release wake e2e.
- Thread union; partial replication; launchd/systemd units.

## Key abstractions

**NodeIdentity** — new struct (module `src/node/`), the node's key custody and identity: the one place that knows the node's own `node_uuid`, tag and signing key.
- Owns: loading or creating `node.key` (0600, created once), deriving `node_uuid`, signing hashes, the current tag and the retag rule (`tag_k`, the higher uuid moves to the next k), and regenerating `node.json`.
- Hides: Ed25519, the tag derivation, and which node moves on a collision.
- Must not: rewrite or regenerate an existing key it cannot read (it refuses to start), or verify other nodes' entries (Envelope does, with the pubkey from that log's genesis).
- Relationships: Journal (signing) and PublicIds (tag) use it; it depends on nothing in store or sync.
- Lifecycle: loaded once at daemon start, held as a read-only `Arc` for the daemon's life; the tag is re-read from the store's tag record after a `node.retag` applies.
- Errors: key missing, unreadable or with bad permissions refuses the start, distinguishable from store errors; signing does not fail.

**Envelope** — new struct (`src/sync/envelope.rs`), one journal entry's signed wire and storage form.
- Owns: the fixed length-prefixed canonical bytes, `hash`, signature verification against a given pubkey, the genesis self-check (`SHA-256(pubkey)[0..16] == node_uuid`), and the compact deps encoding.
- Hides: the byte layout; JSON canonicalization is never used.
- Must not: look at the payload's meaning or touch the store.
- Relationships: a value type shared by Journal, PeerLink frames and FactRegistry (payload bytes).
- Errors: `HashMismatch` and `BadSignature`, distinguishable from a malformed shape.

**Hlc** — new struct, the node's hybrid logical clock (48-bit ms + 16-bit logical).
- Owns: the local tick (`max(wall, last+1)`), folding a remote HLC under the 60 s skew cap, recording skew for `sync status`, resuming from the self log's maximum.
- Hides: bit packing, overflow carry, the skew rule.
- Must not: decide deadlines; ACK and invitation deadlines use the deciding node's wall `Clock` (§7).
- Relationships: wraps the existing `Clock` trait; lives inside `StoreContext`; Journal is its only mutator.
- Lifecycle: one per daemon, behind a mutex, touched only under the writer; rebuilt from the store at start.

**Journal** — new module (`src/store/journal.rs`), the single owner of `sync_logs`, `journal` and `journal_forked`: append, ingest, serve and the vector.
- Owns: local authoring (HLC tick, next counter, prev_hash, sign, insert, then hand to ApplyEngine in the same transaction), batch ingest (contiguity, prev_hash, hash and signature checks, advancing `contiguous_counter`/`head_hash`, the disk-headroom stop), serving committed rows for a `(log, from)` range, the version vector, and the append-only triggers.
- Hides: the per-log chain rules, and that ingest never reads payloads (unknown kinds relay unchanged).
- Must not: materialize anything (ApplyEngine), choose peers (SyncCoordinator) or resolve forks (Incarnations); it only reports them.
- Relationships: composes NodeIdentity, Hlc and Envelope; delegates to ApplyEngine. SyncCoordinator reaches it through a narrow `JournalPort` trait implemented by `SqliteStore` and an in-memory fake for link and routing tests (the fake is the second implementation); `StorePort` is not extended.
- Lifecycle: state in SQLite; functions run in writer transactions (foreground for authoring, background quanta for ingest).
- Errors: typed ingest outcomes callers must tell apart: `Gap` (drop the rest of the batch, re-`Want`), `Fork{evidence}` (to Incarnations), `Corrupt{log}` (penalize this source), `Headroom` (pause pulls, keep authoring), store errors (roll back, nothing advanced). An authoring failure aborts the command as an `ApiError` and is never recorded as `failed`.

**FactKind / FactRegistry** — new trait plus a static registry (`src/sync/facts/`, one module per kind), what each replicated fact means.
- Owns: per kind, the name, major `kind_version`, typed payload, `creates`/`needs` extraction and the apply handler; the registry maps `(kind, major)` to an erased decode-and-apply function.
- Hides: payload schemas, and "unknown kind or newer major means park" (a registry miss is the park decision).
- Must not: recompute audiences, reject a fact the author allowed from the local membership view (§6), or emit wakes directly (local delivery goes through DeliveryTail; derived info/warn events through the existing `append_event_once*` writers).
- Relationships: shared interface; command code authors through a typed `FactKind`; ApplyEngine dispatches through the registry.
- Lifecycle: `OnceLock` static, immutable, like `harness::registry::builtins()`.
- Errors: a handler returns a fact-level rejection (recorded `failed`, counted as processed) or a store error (aborts the savepoint and the quantum, retried); callers keep them apart.

**ApplyEngine** — new module, the one `apply(entry)` path for local and remote entries.
- Owns: readiness (`applied_through = counter-1` and the applied vector covering `deps`), one SAVEPOINT per entry, recording the outcome (`applied`, `parked_unknown`, `parked_dep`, `failed`), assigning local positions (sequence, decision_seq, revisions, ordinals), the startup and `sync unapplied replay` passes, and building the `ApplyContext` handlers get (origin, hlc, `replicated_from` provenance, whether the seat is local, delivery suppression).
- Hides: causal ordering and the catch-up suppression rule (already settled by its home node, or own old incarnation).
- Must not: verify signatures (Journal) or know any kind's semantics.
- Relationships: delegates to FactRegistry; Journal calls it inline for local appends; a new `Lane::Apply` quantum runs it after ingest commits.
- Lifecycle: stateless code; the Apply lane worker is owned by the daemon worker slot and stopped by `factory_stop`.

**DeliveryTail** — new module, extracted from `messages::insert_publication` and the invitation writers: the local-recipient delivery tail.
- Owns: `send_manifests`, prepared and lazy recipient rows, the `send_attention` job, `lazy_delivery::note_published`, `wake_work` and invitation attention rows, for local seats only.
- Hides: the delivery-contract table writes the kicks key on, so a remote fact wakes exactly as a local one does.
- Must not: create work for remote seats or suppressed facts; it does not decide suppression (ApplyContext does).
- Relationships: called by the `message.sent` and `invitation.created` handlers; local and replicated sends share it.

**Incarnations** — new module, fork, restore and corruption repair.
- Owns: fork detection at `Have` time and from ingest evidence, freezing a log at the last common counter, moving the tail to `journal_forked`, linking fork evidence forward, and the owner path (mint a new incarnation, pull every branch, re-issue the union in order under the original fact IDs); restore-without-fork rotation and `sync resync`.
- Hides: the frozen-incarnation policy and the re-issue order.
- Must not: delete entries or author under a frozen incarnation.
- Relationships: called by SyncCoordinator and Journal ingest outcomes; re-issues through Journal authoring; relies on ApplyEngine idempotence.
- Lifecycle: state in `sync_logs`; the re-issue job's progress is persisted and runs as writer quanta.

**PeerLink** — new struct (`src/sync/link.rs`), one peer connection's protocol lifetime, generic over AsyncRead/AsyncWrite so the ssh child's stdio and an accepted Unix socket run the same code.
- Owns: the `HTPEER1` preamble (skipping ≤64 KiB noise), u32 framing at 1 MiB, the Hello/Auth/Accept handshake, the credit window, Ping/Pong with the 90 s dead timer.
- Hides: the wire. `PeerFrame` is a closed enum; `CloseReason` is a closed enum whose one method classifies stop-dialing vs retry.
- Must not: choose which log to pull from, or write the store.
- Relationships: owned by SyncCoordinator; reports Have, Entries and Close upward.
- Lifecycle: a tokio task in the coordinator's JoinSet; ends on EOF, the dead timer or shutdown; the owning Dialer kills the ssh child's process group on teardown.
- Errors: io/decode errors mean the link is dead (Dialer backs off); `Close{reason}` is typed (`envelope_unsupported`, `self_link`, `auth_failed` stop dialing; `busy` retries).

**SyncCoordinator** — new struct (`src/sync/coordinator.rs`), the node-wide replication brain.
- Owns: active links by `node_uuid` (lower dialer's link active, duplicate standby), the `rx` source per log with 30 s failover, Want subscriptions and live-tail pushes, `Have` deltas at ~1/s, per-peer error counts.
- Hides: routing and relay policy.
- Must not: verify or apply entries (Journal, ApplyEngine), or spawn processes (Dialer).
- Relationships: composes PeerLinks; uses `JournalPort` and Incarnations; a new `Lane::Sync` kick on journal commits drives live tail and Have.
- Lifecycle: created in `run_elected_impl`, lives for the daemon; shutdown cancels links.

**Dialer** — new struct, the dial targets' runtime.
- Owns: `sync_dial_targets` state (full-jitter backoff, the `not installed` 30 min retry, last error, learned uuid), and building and spawning the owned transport child (ssh argv by default, or `sync.dial_command` with `{dest}`).
- Hides: ssh options and the dial_command template.
- Must not: discover machines (the discovery worker upserts targets) or route logs.
- Relationships: hands established streams to SyncCoordinator.
- Lifecycle: daemon-lived; owns and reaps every child.

**HostSession / HostSessions** — new struct plus registry, one attached Herdr server.
- Owns: per host, its `Arc<dyn HostPort>` (`NativeCli`), `HostReachability`, observation worker, wake dispatcher, archival sampler and handoff namespace. The registry maps `host_id` to its session, attaches at startup for `herdr_hosts` rows whose socket exists, and lazily when a claim names an unattached endpoint.
- Hides: how many Herdr servers exist; it is the one place that routes by a seat's `host_id`.
- Must not: affect sync or other hosts when its host is unreachable; never addressed for remote seats.
- Relationships: composition; the wake lane, identity and observation get a per-host handle from it, so `HostPort` keeps its single-host shape (§3's per-host handle option).
- Lifecycle: registry lives for the daemon; a session lives until the daemon stops; unreachable only marks it.

**Store transactions** (`execute_accountable_transaction` / `execute_budgeted_idempotent_transaction_with_constraints`) — extended: the apply phase decides and authors facts through Journal, which applies them in the same transaction; the `operations` replay table stays local; `cooperative_instance` checks `claim.host` against HostSessions instead of `claim.instance == self`.

**`host_instances` → node row + `herdr_hosts`** — extended: node-scope revisions stay on the node row; Herdr-scope epochs and sequences move to `herdr_hosts`.

**seats** — extended with `home_node`; one local-seat predicate (view or helper) is used by observation, reconciliation, liveness, wake and retirement-by-observation; callers never test `home_node` inline.

**PublicIds (`store/public_ids.rs`)** — extended: the minter takes the current tag; plus the pure deterministic `d…` direct-thread ID function.

**Daemon transport** — extended: first-frame sniffing gains a peer-connection kind under its own `MAX_PEER_CONNECTIONS = 16` semaphore.

**Lane (`service/kicks.rs`)** — extended: the closed enum gains `Apply` (kicked on `journal` commits) and `Sync`.

**Deadline lane** — extended: scans only receipts and invitations authored by this node; holds overdue emission after a wall-clock jump until the first sync round completes or 60 s pass.

**InstancePaths → NodePaths** — extended: resolved by state dir only; legacy `instances/` is reported, never opened.

**Daemon idle-exit** — extended: the daemon stays up while it has peer links or dial targets.

Variation: fact kinds are added across releases and unknown kinds must park → each kind implements `FactKind`, and `FactRegistry` is the one place kinds are chosen (a miss or newer major means park). Herdr servers per node vary at runtime → HostSessions is the one routing point, keyed on `host_id`. Transport commands vary (ssh, dial_command, remote_command) only as data in one Dialer function, no trait. Peer frames, Close reasons, discovery sources and apply outcomes are fixed sets → closed enums with `match`.

Walk-through: an agent on A sends to a seat on B. Dispatch reaches `publish_send` in the accountable transaction (`claim.host` resolves through HostSessions); the decide step builds a `message.sent` payload with recipients and deadlines; `Journal::author` ticks the Hlc, chains, signs with NodeIdentity and inserts; ApplyEngine applies it through the registry handler in the same transaction (message row, local sequence, receipt rows; DeliveryTail does nothing, the recipient is remote). The commit kicks `Lane::Sync`; SyncCoordinator pushes to B's subscribed Want. B's PeerLink yields Entries; `Journal::ingest` verifies and commits, kicking `Lane::Apply`; B's ApplyEngine finds deps covered and runs the handler; the seat is local and unsettled, so DeliveryTail writes `wake_work`, the Wakes lane kicks, and the HostSession for the seat's `host_id` wakes the pane if idle. When B's seat ACKs, B authors `receipt.settled`; A applies it and clears overdue tracking. A future `thread.union` kind touches one new `FactKind` module and one registry entry; older nodes park it, and Journal, links and ApplyEngine are untouched.

Per spec §5/§7: one apply path and the handler writes recipient rows, but today's two-phase send stages `prepared_recipients` across bounded preparation transactions — the `message.sent` handler reuses a local preparation's staged rows, keyed by message ID, instead of rewriting them (a deliberate local-origin branch inside the one path; otherwise local sends write their audience twice).
Per spec §9: the remote side is a stateless `peer --stdio` pump, so the peer protocol lives entirely in the daemon transport and PeerLink; the CLI pump stays a plain splice, with no abstraction.

Digest:
- NodeIdentity: owns key custody, node_uuid, tag and signing; tag changes arrive only via applied `node.retag`;
  callers never read `node.key` or derive tags themselves.
- Envelope: owns canonical bytes, hash and signature verification; envelope fields are added only here;
  callers never hash JSON or verify ad hoc.
- Hlc: owns the node's HLC tick, remote fold and skew record; clock rules change only here;
  callers never use HLC for deadlines.
- Journal: owns self-log append, verified ingest, serving and the vector; new journal tables or checks go here;
  callers never write `journal`/`sync_logs` rows or materialize facts directly.
- FactKind/FactRegistry: owns per-kind payload, needs/creates and apply; a new fact kind is one module plus one registry entry;
  callers never `match` on kind strings.
- ApplyEngine: owns readiness, savepoints, outcomes, local positions and catch-up suppression; new apply rules go here;
  handlers never decide readiness or suppression, and commands never materialize outside it.
- DeliveryTail: owns local-recipient delivery rows for messages and invitations; delivery-table changes go here;
  handlers never write `wake_work`/manifests directly or for remote seats.
- Incarnations: owns fork freeze, `journal_forked`, re-issue and restore rotation; repair policy changes go here;
  callers never delete entries or author under a frozen incarnation.
- PeerLink: owns preamble, framing, handshake, credit and liveness of one connection; new frames are `PeerFrame` variants here;
  callers never read raw frames or choose retries from Close reasons themselves.
- SyncCoordinator: owns links, rx-source choice, Want/Have routing, live tail and duplicate-link standby; relay policy changes go here;
  links and Journal never pick sources.
- Dialer: owns dial targets' backoff, transport-command construction and child processes; new transport templates are data here;
  nothing else spawns ssh or dial_command children.
- HostSession/HostSessions: owns per-Herdr-server workers and the host_id routing; new per-host workers go into HostSession;
  callers never hold a node-wide `HostPort` or route wakes for remote seats.

## Post-Implementation Notes

> *As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
