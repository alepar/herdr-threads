# Cross-node sync: peer link and sync coordinator — design

Parent: [root spec](2026-10-14-cross-node-sync-design.md) · Bead: ht-5fr.5

## Goal

Two node daemons connected only by the user's `ssh` (or by `sync.dial_command` in tests) authenticate each other by node key and keep their journals converged. Each pulls every origin log the other is ahead on, pushes new entries within one frame of their commit, and relays third-party logs, so a line A–B–C converges. Links survive and recover from partitions: a dead link is detected within 90 s, redialed with full-jitter backoff, and a node with no Herdr server, no clients and nothing to sync is the only daemon that exits on its own.

## Problem description

The journal core (ht-5fr.3) can author, verify, ingest and serve signed entries, but nothing moves them between machines. Root §9 fixes the transport outline: a stateless `peer --stdio` pump on the remote side, an ssh child on the dialing side, u32 framing behind an `HTPEER1` preamble, a mutually signed handshake, `Have`/`Want`/`Entries` anti-entropy with one `rx` source per log, live tail, a credit window, Ping/Pong, backoff and duplicate-link standby. Several things are left open: the exact frames and their encoding, how the handshake signatures bind both hellos, how the daemon transport hands a sniffed peer connection to the coordinator, the coordinator's concurrency model around blocking `JournalPort` calls and FairWriter turns, how each `IngestOutcome` drives routing, failover rules, the shape and ownership of `sync_dial_targets`, ssh argv quoting, process-group ownership and reaping, and the `sync.dial_command` template. Two sibling signals are also undefined: ht-5fr.13 needs a "first sync round complete" signal and per-node reachability readable from store queries. Finally, the root makes this subepic own daemon idle-exit, but today's daemon has no idle exit at all (`src/app.rs` and `src/daemon/lifecycle.rs` run until `Stop`; docs/install.md documents no idle period).

## Main challenges

The link must be safe against everything a same-user peer or a noisy shell can produce without adding adversarial verification. The rules: a bounded frame, bounded noise before the preamble, bounded hello, a typed close, and signatures checked only where the policy needs them (node identity at the handshake, entries in Journal). Relay must not amplify in a cycle or duplicate transfer in a mesh, but correctness must never depend on dedup, since ingest is idempotent. Backpressure has to cover both the ssh pipe and the receiver's writer, which is shared with local commands through FairWriter background turns. Pings must never queue behind a megabyte of entries. `JournalPort` is synchronous and must not block the tokio runtime that also serves local IPC. Failover must make progress when a source stalls or serves corrupt bytes, and must not flap between healthy sources. Both ends must agree on which of two duplicate links is active without an extra round trip. The dialer owns child processes that must always be killed and reaped (AGENTS.md), including on daemon shutdown and on test teardown. Reachability through relays must not depend on cross-node clock agreement. An idle exit must never stop a daemon that serves a Herdr server or a peer.

## Key decisions made

The remote pump is two blocking copy threads over raw fds 0/1 and the daemon socket, with no tokio and no frame parsing. The daemon transport sniffs a `PeerFrame::Hello` first frame and hands the stream plus the frame bytes to a `PeerIntake` channel, under its own 16-slot semaphore and outside `MAX_CONNECTIONS`. `PeerFrame` is one serde enum tagged by `"t"` with an `Unknown` catch-all that receivers ignore, so later frames are added behind caps. The handshake signs a domain-separated SHA-256 over both raw hello frames plus the peer's nonce. `self_link` and `node_collision` are told apart by the daemon boot id in the hello. Credit is counted in encoded `Entries` frame bytes and enforced inside PeerLink, which keeps a separate control queue that is always written first. The coordinator is one tokio actor task that owns all routing state. Blocking store work runs through `spawn_blocking`, and ingest goes through one sequential ingest worker, so per-log order holds and there is one background writer turn at a time. The active one of two duplicate links is the minimum of `(dialer node_uuid, dialer hello nonce)`, which both ends compute locally. `Unwant` is added to the frame set, so failover cancels the old subscription. Reachability is gossiped as relative ages (`Seen`, every 10 s), never as absolute timestamps. It is persisted in a new `sync_node_contact` table that ht-5fr.13's queries read. The round-complete signal is in memory (`SyncSignals`). Dial targets live in `sync_dial_targets` with explicit-beats-discovered precedence, persisted backoff, and a persisted `stopped` state that is cleared by re-adding the target or by a different local software version. Idle exit is new: only a store-only daemon with no peer links, no enabled dial targets and no open local connections exits, after 10 continuous minutes.

## Decision points

### 1. Module layout

**Recommended.** New core files (all scanned by `scripts/check-harness-neutral`, naming no harness):

| File | Contents |
|---|---|
| `src/sync/frame.rs` | `PeerFrame`, `PeerHello`, `HaveLog`, `SeenAge`, `CloseReason`, `CloseClass`, frame encode/decode, preamble constants |
| `src/sync/link.rs` | `PeerLink` handshake and runtime: `LocalHello`, `Role`, `Established`, `LinkHandle`, `LinkEvent`, `LinkError`, preamble reader, credit window, Ping/Pong, dead timer |
| `src/sync/coordinator.rs` | `SyncCoordinator` actor, `SyncHandle`, `PeerIntake`, rx selection, subscriptions, ingest worker, Have/Seen scheduling |
| `src/sync/signals.rs` | `SyncSignals` (round-complete), `ContactBook` (in-memory reachability) |
| `src/sync/dialer.rs` | `Dialer`, `DialTarget`, `DialTransition`, `FullJitter`, `TransportCommand`, child ownership |
| `src/sync/sync_store.rs` | `SyncStorePort` trait and `MemorySyncStore` (`#[cfg(any(test, feature = "test-support"))]`) |
| `src/store/sync_dial_targets.rs` | the only owner of `sync_dial_targets`: `add_explicit`, `upsert_discovered`, `forget`, `list`, `enabled`, `record_transition`, `clear_stale_stops` |
| `src/store/sync_contacts.rs` | the only owner of `sync_node_contact`: `record`, `reach`, `UNREACHABLE_AFTER_MS` |
| `src/cli/peer.rs` | `peer --stdio` (ht-5fr.8 adds `peer add/forget/list` to the same file) |
| `src/daemon/idle_exit.rs` | `IdleExit` monitor and `ActivityProbe` |

Changed: `src/daemon/transport.rs` (peer sniff, `ConnectionGauge`), `src/daemon/transport/peer_connection.rs` (new; the sniffed hand-off), `src/service/kicks.rs` (`Lane::Sync`, table map), `src/daemon/settings.rs` (`sync.dial_command`), `src/app.rs` (composition, shutdown, idle exit), `src/service/config.rs` (test-support overrides), `src/main.rs` and `src/cli/mod.rs` (route `peer --stdio`), `migrations/node/0001_node_baseline.sql` (two tables, editable until release), `src/store/mod.rs` (`impl SyncStorePort for SqliteStore`).

No new dependencies. `rand`-like jitter uses the existing xorshift pattern of `service::pacer::Backoff`. Signing and verification use ht-5fr.1's `NodeIdentity::sign` and `ed25519-dalek::VerifyingKey::verify_strict`.

**Considered.** Putting frames in `src/protocol/`: that module is the local CLI↔daemon wire with an exact `PROTOCOL_VERSION`, and peer frames are versionless (envelope plus caps), so mixing them would invite the exact-version rule. Extending `StorePort` for dial targets: the root keeps `StorePort` unextended, and a narrow port gets a fake for coordinator tests.

### 2. `peer --stdio`: the stateless pump

**Recommended.** `herdr-threads [--state-dir X] peer --stdio`, in `src/cli/peer.rs::run_stdio(context) -> ExitCode`:
1. Resolve `RuntimeContext` like every CLI command (state dir from the flag, `HERDR_PLUGIN_STATE_DIR` or the default; the host is optional and passed to `ensure_running` only as the attach hint when one resolves).
2. `lifecycle::ensure_running(context, current_exe, clock)` (5 s bound; a store-only start is fine). On failure, print one stderr line `herdr-threads peer: daemon unavailable: <detail>` and exit 3. No preamble is written, so the dialer sees EOF before the preamble.
3. Connect a `std::os::unix::net::UnixStream` to the descriptor's socket path.
4. Write exactly `HTPEER1\n` to fd 1 with `write_all` on a `ManuallyDrop<File>` built with `File::from_raw_fd(1)`. `std::io::Stdout` is not used, because it is line-buffered.
5. Splice with two std threads and no tokio:
   - `up`: `io::copy(stdin_fd0_file, socket)`. On EOF it calls `socket.shutdown(Write)` and returns.
   - `down`: `io::copy(socket_clone, stdout_fd1_file)`. On EOF the process exits 0.
   - Any read or write error exits 1. SIGPIPE is already ignored by the Rust runtime, so a dead ssh channel surfaces as `EPIPE`.
6. The main thread joins `down` and then `process::exit`s, which ends a blocked `up`.

The pump never parses a frame, keeps no file and writes no log. The daemon-side 90 s dead timer closes the socket, so an orphaned pump exits at the latest then (research 03 R-D). The existing `peer_cred().uid == owner_uid` check admits it, because sshd runs it as the same user.

**Considered.**
- `tokio::io::copy_bidirectional` over `tokio::io::stdin`: tokio's stdin is a blocking-pool reader with its own buffering, which costs a runtime for no gain.
- Passing `--state-dir/--host-endpoint` learned from a hello in the remote command: the node is addressed by state dir alone (ht-5fr.1), and `remote_command` already covers a non-default state dir.
- Writing the preamble before `ensure_running`: the dialer could then send its hello into a pump with no socket yet. Writing it after the connect means the hello always has a reader.

### 3. Transport: first-frame peer sniffing and admission

**Recommended.** In `serve_connection` (`src/daemon/transport.rs`), after the uid check and the first `read_frame`, and after the watch sniff, the transport tries `PeerFrame::decode_hello(&frame)`. It succeeds only for a JSON object whose `"t"` is `"hello"`; `WireRequest` and `ServiceWireRequest` can never match it.
- The ordinary permit is dropped, as for watch.
- A peer permit comes from a new `peer_slots = Semaphore::new(MAX_PEER_CONNECTIONS)` where `pub const MAX_PEER_CONNECTIONS: usize = 16` (in `src/sync/frame.rs`).
- `peer_connection::hand_off(stream, frame, permit, intake: Option<PeerIntake>)`:
  - The intake is absent (coordinator not started, or stopped): write one `Close{reason: busy, detail: "sync not running"}` frame (1 s bound) and return.
  - No permit: write `Close{busy, "peer connections full (16)"}` and return.
  - Otherwise `intake.offer(PeerStream { stream, first_frame, permit })` over a bounded `mpsc` (capacity 16). A full or closed channel gives the same `busy` close. The transport task then returns. The link's lifetime belongs to the coordinator's `JoinSet`, and the permit travels with the stream, so the 250 ms transport drain is unaffected.
- `serve_with_gate` gains `peers: Option<PeerIntake>` and `gauge: ConnectionGauge`. `serve` passes `None` and a fresh gauge, so existing tests compile unchanged.
- `ConnectionGauge(Arc<AtomicUsize>)` counts accepted connection tasks that are not peer connections (ordinary, watch and service). It is incremented at spawn and decremented by a drop guard. Idle exit (§15) reads it.

The 5 s first-frame timeout (`ORDINARY_TIMEOUT`) also applies to peers: the dialer sends its hello as soon as it reads the preamble, which takes one ssh round trip.

**Considered.**
- A second listener or socket for peers: it is a new path to secure and to clean up, and sniffing is the existing pattern (watch).
- Running the link inside the transport task: the transport drain would then wait on long-lived links, and links would live in two `JoinSet`s.

### 4. Wire: preamble, framing, `PeerFrame`, `CloseReason`

**Recommended.**
- **Preamble.** The remote writes `HTPEER1\n` once. The dialer reads byte-wise into a 64 KiB window until it finds the 8-byte line. Outcomes:
  - found after skipping `n` bytes: if `n > 0`, log once per target per daemon run `sync peer <dest>: remote printed <n> bytes before preamble`;
  - more than 64 KiB without it: `LinkError::PreambleNoise`;
  - 30 s without it: `PreambleTimeout`;
  - EOF first: `PreambleEof` (the dialer then classifies the child's exit status, §13).
- **Framing.** `transport::read_frame_with_limit`/`write_frame` (u32 BE length plus JSON, non-empty, `MAX_FRAME_BYTES` = 1 MiB − 4). The same functions are used on both sides.
- **`PeerFrame`** (serde, internally tagged `#[serde(tag = "t", rename_all = "snake_case")]`). Every struct tolerates unknown fields, so peers can add fields compatibly:

| `t` | Fields | Notes |
|---|---|---|
| `hello` | `envelope: u32` (=1), `node: NodeUuid`, `tag: NodeTag`, `pubkey: hex64`, `boot: Uuid`, `caps: Vec<String>` (≤32 × ≤64 B), `limits: {max_frame, batch_entries, batch_bytes, credit}`, `nonce: hex64` (32 random bytes), `role: dialer\|dialed` | ≤8 KiB, else `protocol_error` |
| `auth` | `sig: hex128` | §5 |
| `accept` | none | |
| `close` | `reason: CloseReason`, `detail: Option<String>` (≤256 B) | allowed in any phase |
| `have` | `full: bool`, `ask: bool`, `logs: [HaveLog{node, inc, counter, head: hex64, frozen_at: Option<u64>}]` (≤4096) | `full` replaces the stored peer vector; otherwise a delta |
| `want` | `node, inc, from` | `from ≥ 1` |
| `unwant` | `node, inc` | ends a subscription |
| `entries` | `node, inc, entries: [WireEnvelope]` (≥1, ≤256) | credit-gated; every entry's log must equal the frame's |
| `credit` | `bytes: u32` | |
| `ping` / `pong` | `n: u64` | |
| `seen` | `ages: [SeenAge{node, age_ms: u64}]` (≤4096) | §12 |
| other | `#[serde(other)] Unknown` | ignored and counted; senders send a non-base frame only when the peer advertised its cap |

  The base cap is `"sync.v1"` (every frame above). ht-5fr.6 adds its fork frames behind `"fork.v1"`.
- **`CloseReason`** is a closed enum: `envelope_unsupported`, `node_collision`, `self_link`, `auth_failed`, `busy`, `shutdown`, `protocol_error`, and `#[serde(other)] unknown`. Its one method is `class() -> CloseClass::{StopDialing, Retry}`:
  - `StopDialing`: `envelope_unsupported`, `self_link`, `auth_failed`, `node_collision`;
  - `Retry`: everything else.

  Root §9 lists the first five for the handshake. `shutdown` and `protocol_error` are post-handshake closes, and `unknown` keeps a newer peer's reason from breaking decode. `node_collision` stops dialing because it means the same key runs in two daemons, which only an operator repairs (research 09 F6). The node that refuses with it in its own handshake, or receives it, also reports clone evidence about itself (§8 `ForkSink::on_clone`), which stops its automatic identity actions until the operator clears it (root §10 clone gate).

**Considered.** Externally tagged enums (`{"hello":{…}}`): they are harder to sniff and to read in captures. A binary frame header: root §9 fixed JSON.

### 5. Handshake and mutual authentication

**Recommended.** `link::handshake(io, me: &LocalHello, role, first_frame: Option<Vec<u8>>, deadline) -> Result<Established, LinkError>`. `LocalHello` holds `Arc<NodeIdentity>`, the boot id, caps and limits.
1. The dialer sends `Hello` right after the preamble. The dialed side already holds the dialer's hello as the sniffed first frame and answers with its own `Hello`. Each side keeps both hellos' **raw frame bytes**.
2. On receiving the peer hello, each side checks in this order and sends `Close` on the first failure:
   - `envelope != 1` → `envelope_unsupported`;
   - `SHA-256(pubkey)[0..16] != node` → `auth_failed`;
   - `node == my node`: `boot == my boot` → `self_link`, else `node_collision`;
   - a hello frame over 8 KiB, or more than 32 caps → `protocol_error`.
3. Each side sends `Auth{sig}` with `sig = NodeIdentity::sign(SHA-256("HTPEER1-AUTH\0" ‖ u32 len ‖ own_hello_bytes ‖ u32 len ‖ peer_hello_bytes ‖ peer_nonce))`. The domain prefix keeps this signature apart from Envelope's `HTJRNL01` domain. Signing the peer's fresh nonce and both exact hellos binds the key to this link and prevents replay.
4. On the peer's `Auth`, each side verifies with the peer's hello pubkey over `(peer_hello_bytes, own_hello_bytes, own_nonce)`, using `verify_strict`. Failure → `Close{auth_failed}`.
5. Each side sends `Accept`. The link is `Established` once both `Accept`s are seen.
   - Effective limits: `max_frame = min(both)`, `credit = min(both, 1 MiB)`, `batch_entries = min(both, 256)`, `batch_bytes = min(both, 256 KiB)`.
   - Effective caps: the intersection.
6. A 15 s handshake deadline covers steps 1-5. Expiry is `LinkError::HandshakeTimeout` (retry class).

`Established { peer: PeerInfo { node, tag, pubkey, boot, caps, limits, role, nonce }, reader, writer }`. The coordinator then checks the hello's `pubkey` against the genesis pubkey of any held log for that node. A mismatch is impossible unless SHA-256 collides, so it is treated as `auth_failed`.

Unknown caps are ignored. No version is compared anywhere (AGENTS.md versionless rule).

The boot id is a random `Uuid` drawn once per daemon process, so the `self_link` versus `node_collision` split assumes each copy of a cloned node runs its own daemon process started after the copy. A memory-preserving clone (a live VM snapshot or a fork of a running daemon) keeps the boot id; it is out of scope (root §12).

**Considered.**
- Signing only the peer nonce: it does not bind the peer's advertised caps and limits.
- A challenge–response round before the hellos: one more round trip over ssh for no added property.
- Comparing `boot` only for `self_link`: without the boot, a cloned VM looks like a self link and is silently stopped, instead of being reported as a collision.

### 6. Credit window and liveness

**Recommended.**
- **Credit** is counted in the encoded byte length of `entries` frames. The sender keeps `outstanding`. It may write an `entries` frame when `outstanding + len ≤ credit`, or when `outstanding == 0`, so that a single oversized frame always goes out. Each received `Credit{bytes}` reduces `outstanding` (saturating at 0).
- The receiver returns `Credit{len}` for each `entries` frame once that frame is finished: ingested and committed, dropped as unsolicited, rejected as corrupt, or refused by `Headroom`. Control frames are never credit-gated.
- **Frame size.** A served batch is ≤256 entries and ≤256 KiB canonical (Journal `serve`). The sender encodes it, and if the JSON exceeds `max_frame`, splits it in halves recursively. A single entry always fits: payload ≤512 KiB, hex deps ≤4096 × 56 B, needs ≤256 × 160 B, plus fixed fields, which totals under 800 KiB.
- **Liveness.** Each side tracks `last_inbound` and `last_ping_sent`. When 30 s pass with no inbound frame and no ping sent in those 30 s, it sends `Ping{n}`. A `Ping` is answered with `Pong{n}` at once on the control queue. Any inbound frame resets `last_inbound`. At 90 s without an inbound frame the link ends with `LinkError::Dead`. Both roles run this, so the dialed daemon closes its socket and the remote pump exits. ssh's own `ServerAliveInterval=15 × 3` catches a dead TCP path at about 45 s.

**Considered.** Counting credit in entries rather than bytes: the memory and pipe pressure is bytes. Granting credit on receipt instead of after commit: that would let a slow writer buffer without bound (research 03 §5).

### 7. PeerLink runtime

**Recommended.** After `Established`, `link::spawn(established, link_id, events: mpsc::Sender<LinkEvent>, clock, cancel) -> LinkHandle` starts two tokio tasks in the coordinator's `JoinSet`:
- **reader:** decodes frames, handles `ping`, `pong`, `credit` and `unknown` itself, and forwards every other frame as `LinkEvent::Frame{link_id, frame, wire_len}`. It runs the dead timer. A decode error or a frame received in the wrong phase (`hello`/`auth`/`accept` after establishment) ends the link with `Close{protocol_error}`.
- **writer:** two queues. `control` (bounded 256) holds every frame except `entries`. `data` (bounded 4 frames, pre-encoded bytes) holds `entries`. The writer always drains `control` first, then writes one `data` frame if credit allows, and emits `LinkEvent::CreditFreed{link_id}` when space or credit frees.

`LinkHandle` methods (non-blocking, all return `Err(LinkGone)` after the end): `send(frame)`, `try_send_entries(bytes) -> Result<(), Full>`, `grant(bytes)` (queues `Credit`), `close(reason)` (writes `Close` within 1 s, then drops both halves).

The link ends with `LinkEvent::Ended{link_id, why: LinkError, up_for: Duration}`. `LinkError` is a closed enum: `Io`, `Decode`, `PreambleNoise`, `PreambleTimeout`, `PreambleEof`, `HandshakeTimeout`, `Dead`, `PeerClosed(CloseReason)`, `Refused(CloseReason)`, `Shutdown`, plus `class()` delegating to `CloseReason::class()` for the two close variants, and `Retry` otherwise. For a dialed link the coordinator only logs it. For a dialer link the Dialer turns it into a transition (§13).

PeerLink is generic over `AsyncRead + AsyncWrite + Unpin + Send`. The dialer passes the ssh child's `ChildStdout`/`ChildStdin`, the transport passes the `UnixStream` halves, and tests pass `tokio::io::duplex`.

**Considered.** One task per link with `select!` over read and write: a large write would delay `pong` answers and the dead timer. Coordinator-side credit accounting: the root assigns the credit window to PeerLink.

### 8. SyncCoordinator: actor, links, duplicate standby

**Recommended.** `SyncCoordinator::start(deps: CoordinatorDeps) -> (SyncHandle, PeerIntake, JoinHandle<()>)`. `CoordinatorDeps` holds:
- `journal: Arc<dyn JournalPort>`, `sync_store: Arc<dyn SyncStorePort>`, `identity: Arc<NodeIdentity>`, `boot: Uuid`;
- `writer: Arc<FairWriter>`, `clock: Arc<dyn Clock>`, `pacer: Arc<Pacer>` (the `Lane::Sync` pacer), `status: Arc<LaneStatus>`, `log: Arc<dyn Fn(&str)>`;
- `fork_sink: Arc<dyn ForkSink>` (§10), `dial: DialConfig`, `cancel: Cancellation`.

It runs one actor task on the daemon's tokio runtime and owns all state. Inputs are an `mpsc` of `LinkEvent`s, `PeerIntake` offers, Dialer hand-offs (`DialedStream { target_id, dest, stdout, stdin, report }`), ingest-worker results, `pacer.wait(1 s)` (kicks), and a 1 s housekeeping tick.

**Dialed-link reports.** `report` is the per-dial channel back to the DialTask: a bounded `mpsc::Sender<DialReport>` (capacity 2), `enum DialReport { Established{ node_uuid: NodeUuid }, LinkEnded{ target_id, why: LinkError, up_for, peer_node: Option<NodeUuid> } }`. The actor sends `Established` once, right after the handshake completes (before the link is ranked active or standby), and `LinkEnded` exactly once, last, whether the handshake failed or the established link ended; a handshake failure sends only `LinkEnded` with `peer_node = None`. A dropped sender without `LinkEnded` (actor stopped) reads as `LinkEnded{why: Shutdown}`. The DialTask records `Linked` on `Established` (§13) and classifies on `LinkEnded`.

State:
- `links: BTreeMap<LinkId, LinkState>`. `LinkState` holds `{ handle, peer: PeerInfo, origin: Inbound | Dialed{target_id}, mode: Active | Standby, peer_vector: Vector, subs_out: BTreeMap<LogKey, u64 /*next counter to push*/>, errors: BTreeMap<LogKey, PenaltyUntil>, round: RoundState, last_have_sent: Vector }`.
- `by_node: BTreeMap<NodeUuid, Vec<LinkId>>`.
- `rx: BTreeMap<LogKey, RxSource { link, requested_from, last_progress: MonoInstant }>`.
- `local: Vector` (the last read of `journal.vector()`).
- `contacts: ContactBook` and `signals: SyncSignals` (§12).

**Duplicate links.** For a node with several established links, the active one is the link minimising `(dialer node_uuid, dialer hello nonce)`, where "dialer" is whichever side has `role = dialer` on that link. Both ends compute this from the hellos alone, so they agree without a frame. All others are `Standby`: they keep Ping/Pong and `Seen` only, and never send or accept `have`/`want`/`entries`.
- When an active link ends, the next link by the same order is promoted locally. It sends `Have{full, ask: true}` and re-issues `Want`s.
- A `have` or `want` received on a link this side thinks is standby is taken as the peer's promotion: that link becomes active here too, and the formerly active link is demoted.
- When a third link to the same node is established, the coordinator closes the newest standby beyond two with `Close{busy}` (retry class, so its dialer backs off).
- **Clone at a third node.** Duplicates are links whose hellos carry one `node_uuid` and one `boot`. When a newly established link carries a `node_uuid` that an existing link carries with a **different** `boot`, the coordinator first probes the older link: it sends `Ping` and waits up to 10 s. No inbound frame means the older link is a leftover of a peer daemon that restarted: it is ended with `LinkError::Dead` and the new link is ranked normally. An answer means two live daemons hold one key: the coordinator closes the newer link with `Close{node_collision}` (stop-dialing class on whichever side dialed it), keeps the older one, and calls `fork_sink.on_clone(CloneReport{subject: node, kind: ThirdParty, boots: [old, new], source: self})` so `sync status` shows it. The closed clone receives `PeerClosed(node_collision)`; the coordinator on that side, and on either side of a handshake that refused with `node_collision`, calls `on_clone(CloneReport{subject: self, kind: PeerClose | Handshake, …})`. `ForkSink::on_clone` is a default no-op method added here; `LogOnlyForkSink` writes one log line per subject per hour, and ht-5fr.6's sink records the evidence (incarnations §12). The probe runs in the actor through the link's ping and dead timer, never blocking it.
- **Own-history signal.** After every `peer_vector` merge, link end and ingest of an own log, the coordinator sets `own_history.set(Links, on)` (`CoordinatorDeps.own_history: Arc<OwnHistoryHold>`, the one in SyncState, journal-core §7 step 3a), where `on` = some active link advertises a log whose `node_uuid` is this node's at a counter above the local `contiguous_counter` (for a frozen own log, only up to `frozen_at`; beyond it the branches are ht-5fr.6's flag). It is a pure function of `peer_vector`s and `local`. A fresh node, which no peer advertises beyond itself, never sets it.

**Considered.**
- Closing duplicates outright: a later failover would then need a new ssh handshake (research 03 R-E).
- Locks shared between link tasks instead of an actor: per-log routing decisions span links, and one owner avoids lock ordering.

### 9. Vector exchange, rx source, Want, live tail and failover

**Recommended.**
- **On active:** send `Have{full: true, ask: false, logs: local}` and start the round (§12). On `have`, merge into `peer_vector` (`full` replaces it, a delta upserts) and run `reselect` for the logs that changed. On `Have{ask: true}`, reply with a full `Have{ask: false}`.
- **Wantable log** `L`: some active link advertises `counter > local[L].contiguous`, `L` is not paused (fork, headroom), and, when `local[L]` is frozen, `contiguous < frozen_at`. Frozen-log pulls beyond `frozen_at` are ht-5fr.6's frames.
- **`reselect(L)`** picks the rx source from the candidates: active links advertising more than local, with no live penalty for `L`. The order is:
  1. the origin itself (`link.peer.node == L.node`);
  2. the highest advertised counter;
  3. the lower `node_uuid`.

  The current source is kept while it is still a candidate and has made progress in the last 30 s, except that a newly linked origin always takes over. On a switch, the coordinator sends `Unwant{L}` to the old source (if live) and `Want{L, from: local[L].contiguous + 1}` to the new one.
- **Failover:** on the 1 s tick, any rx source with no ingest progress for `L` for 30 s while it still advertises more is penalized for `L` for 30 s, and `reselect(L)` runs. If it is the only candidate, the coordinator re-`Want`s it from `contiguous+1`, since `Want` is idempotent.
- **Serving `want{L, from}`:** record `subs_out[L] = from` and run `push(link, L)`, which loops while the link's data queue accepts:
  - `spawn_blocking(journal.serve(L, next, limits))` (query connection, committed rows only);
  - convert with `WireEnvelope::from`, encode and split (§6), `try_send_entries`;
  - advance `next` past the last counter.

  An empty serve leaves the subscription armed (live tail). `CreditFreed` and every vector change re-run `push` for armed subscriptions whose `next ≤ local[L].contiguous`. `unwant{L}` removes the subscription. Wants for logs this node does not hold are kept armed and served once the log appears.
- **Receiving `entries`:**
  - Not from the current rx source for that log, or a log never wanted on that link: drop, `grant(len)`, count `unsolicited`.
  - Otherwise `TryFrom<WireEnvelope>` per entry. A shape error is handled as `Corrupt` for that log.
  - Otherwise the batch is queued to the ingest worker (§10) with `(link_id, len)`.

Duplicates at or below `contiguous` are skipped by ChainRules (`Duplicate`), so a relay cycle cannot amplify. Entries are relayed only after they are committed, because serve reads committed rows.

**Considered.**
- Pulling every log from every peer: N× duplicate transfer in a mesh.
- Persisting the last `Have` received from each peer for partial Haves: optional at ≤10 nodes (root §9). It stays a future cap; v1 always sends a full Have on activation.
- Sticky sources with no penalty: a source serving corrupt bytes would be retried forever.

### 10. Ingest worker and outcome handling

**Recommended.** There is one ingest worker task with a FIFO (bounded only by credit, ≤16 links × 1 MiB). For each batch:

```
spawn_blocking(move || {
    let _o = enter_lane(Lane::Sync);
    let turn = writer.enter_background(budget(5 s))?;
    journal.ingest(&log, entries, &budget)
})
```

The result goes back to the actor, which always calls `grant(len)` on that link and then acts on the outcome:

| Outcome | Action |
|---|---|
| `Stored{head, ..}` | `local[L] = head`; mark rx progress; contact `L.node` and the link's node (§12); schedule Have delta; `push` to other subscribers of `L` |
| `Gap{expected, ..}` | re-`Want{L, from: expected}` on the same source |
| `Fork{evidence, ..}` | `fork_sink.on_fork(L, evidence, link.peer.node)`; on `ForkAck::Paused` pause `L` (no Want) until the sink resumes it; `ForkAck::Corrupt` is handled as `Corrupt` (amended by [incarnations design §12](2026-10-14-cross-node-sync--incarnations-design.md#12-runtime-sink-worker-coordinator-changes)) |
| `Frozen{frozen_at, ..}` | `Unwant{L}`; `L` is no longer wantable past `frozen_at` |
| `Corrupt{reason, ..}` | penalize `(link, L)` for 10 min, count an error, `Unwant`, `reselect(L)`; with no other source, re-Want the same source after the penalty expires |
| `Headroom` | pause **all** wants: `Unwant` every rx subscription and set `headroom_until = now + 60 s`, after which reselect runs again (a still-full disk answers `Headroom` again) |
| `Err(store)` | `status.record_failure(Lane::Sync, &e)` (Health: `lane sync failed: …`); re-Want from `contiguous+1` after the Lane::Sync pacer's backoff |

`ForkSink` is a trait with `on_fork(&self, log, evidence, source) -> ForkAck` (amended by [incarnations design §12](2026-10-14-cross-node-sync--incarnations-design.md#12-runtime-sink-worker-coordinator-changes): it returns `ForkAck::{Paused, Corrupt}`, not `()`; `LogOnlyForkSink` returns `Paused`) and `resumed(&self) -> Vec<LogKey>` (polled on the 1 s tick). ht-5fr.6 implements it with Incarnations. Until then, `LogOnlyForkSink` writes one log line per log per hour, `sync: fork on <uuid8>/<inc> from <uuid8>; log paused`, and never resumes. The coordinator itself never freezes or re-issues anything (root: SyncCoordinator must not resolve forks). It is the second implementation of a trait ht-5fr.6 needs anyway, so it is not speculative.

**Considered.**
- One ingest per link in parallel: FairWriter serializes them anyway, and per-log order across a source switch could interleave.
- Ingest inline in the actor: it would block the runtime threads that also serve local IPC.

### 11. Have deltas and `Lane::Sync`

**Recommended.**
- `src/service/kicks.rs`: `Lane::Sync` at bit 7, `COUNT`/`INTERNAL_ALL` = 8, `name() = "sync"`. `LaneSet` is already `u16` from ht-5fr.3.
- Table map (`SYNC_TABLES`): `journal` → `Apply | Sync`, `sync_logs` → `Sync`, `sync_dial_targets` → `Sync`. `sync_node_contact` goes into `KNOWN_UNMAPPED`.
- `kicks_for_commit` keeps `Sync` for every origin except Retention (whose commits discard all kicks today), including Sync-origin commits. An ingest that stored entries must kick Sync to reach other subscribers, and the actor's reaction to a no-change kick is one vector read. Apply-origin commits only `UPDATE journal.apply_state`, so they kick Sync too. The cost is one indexed `sync_logs` read per apply pass, and the pacer coalesces them.
- **On a kick** (Pacer wait returns `Kicked`), the actor reads `journal.vector()` via `spawn_blocking`, diffs against `local`, and for changed logs:
  - runs `push` for armed subscriptions;
  - adds the logs to each active link's pending delta.

  It also tells the Dialer to reload targets (§13), rate-limited to once per second.
- **Have delta:** at most once per second per link, an active link with a non-empty pending delta gets `Have{full: false, logs: changed}`, so non-rx peers learn who holds what (root §9).

**Considered.** An explicit notify call from Journal: the root keeps kicks table-driven (journal-core §13 made the same choice).

### 12. Reachability and the round-complete signal (for ht-5fr.13)

**Recommended.**

**Contacts.** `ContactBook` (`src/sync/signals.rs`) holds `node → (last_contact_wall_ms, via: Direct | Relayed)`:
- Any inbound frame on an established link (active or standby) records `Direct` contact with that link's node at the local wall time.
- Every 10 s, and once on activation, each active link gets `Seen{ages}`: for every node in the book, `age_ms = now − last_contact`; self is reported with age 0; the receiving link's own node is excluded.
- On receiving `Seen`, a candidate contact is `now − age_ms` (`Relayed`) for each entry other than self. It is taken only when newer than the held contact.

Ages are relative, so no cross-node clock agreement is needed, and an echo can never make a contact newer than it was. A line of nine relays lags at most about 90 s.

**Persisted view.** New table in the baseline:

```sql
CREATE TABLE sync_node_contact (
  node_uuid TEXT PRIMARY KEY CHECK (length(node_uuid) = 32),
  last_contact_at INTEGER NOT NULL,          -- unix ms, local wall clock
  via TEXT NOT NULL CHECK (via IN ('direct','relayed')),
  updated_at INTEGER NOT NULL
) STRICT;
```

The actor flushes through `SyncStorePort::record_contacts(&[NodeContact])` (one background writer quantum, an upsert keeping `MAX(last_contact_at)`). A node's row is written when its contact advanced by ≥30 s since the last write, or when it crossed the reachable/unreachable threshold. Flushes happen at most every 10 s. `store::sync_contacts::reach(conn, node, now_ms) -> NodeReach::{Reachable, Unreachable{since: Option<u64>}}` is the one rule: `Reachable` when `now − last_contact_at ≤ UNREACHABLE_AFTER_MS` (180 000), else `Unreachable{since: Some(last_contact_at)}`; no row gives `since: None`. The self node is always `Reachable`. ht-5fr.13 calls `reach` from `src/store/queries.rs` and renders its marker line. Persisting it means "unreachable since" survives restarts and is readable without the daemon.

**Round complete.** `SyncSignals` is `Arc`-shared and lock-protected. A link's round:
- starts at activation, or when `request_round(at)` is called (the actor then sends `Have{full, ask: true}` on every active link);
- completes once a full `Have` from the peer has arrived after the start, and every log it advertised ahead of us has local `contiguous ≥` the advertised counter or became unwantable (paused, frozen or penalized).

`SyncSignals::round_state(since: MonoInstant) -> RoundState::{NoLinks, Complete, Pending{links: u32}}` is `Complete` when every active link has completed a round started at or after `since`. ht-5fr.13 calls `request_round(now)` at daemon start (once the coordinator is up) and on a wall-clock jump, then holds overdue emission while the state is `Pending`, or `NoLinks` while an enabled dial target exists (its link is still being dialed), for at most 60 s (root §7 overdue hold). `SyncHandle::signals()` hands the `Arc` to the deadline-lane composition in `app.rs`.

**Considered.**
- Absolute timestamps in gossip: cross-node skew would make nodes look reachable or unreachable wrongly (root §4 accepts skew only for LWW and shown times).
- In-memory-only reachability: the store queries that render it (ht-5fr.13) run in the service and on daemon-less `doctor`, and a restart would forget when a node was last seen.
- Deriving reachability from log progress: a quiet node authors nothing and would look dead.

### 13. Dialer: targets, transport command, backoff, child ownership

**Recommended.**

**Table** (baseline):

```sql
CREATE TABLE sync_dial_targets (
  target_id INTEGER PRIMARY KEY,
  ssh_dest TEXT NOT NULL UNIQUE CHECK (length(ssh_dest) BETWEEN 1 AND 1024 AND substr(ssh_dest,1,1) <> '-'),
  source TEXT NOT NULL CHECK (source IN ('explicit','herdr_machine','herdr_remote_process')),
  enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0,1)),
  remote_command TEXT CHECK (remote_command IS NULL OR length(remote_command) BETWEEN 1 AND 1024),
  learned_node_uuid TEXT CHECK (learned_node_uuid IS NULL OR length(learned_node_uuid) = 32),
  state TEXT NOT NULL DEFAULT 'idle' CHECK (state IN ('idle','dialing','linked','backoff','not_installed','stopped')),
  stopped_reason TEXT CHECK (stopped_reason IS NULL OR stopped_reason IN ('envelope_unsupported','self_link','auth_failed','node_collision')),
  stopped_version TEXT,
  attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
  next_attempt_at INTEGER,
  last_error TEXT CHECK (last_error IS NULL OR length(last_error) <= 512),
  last_error_at INTEGER,
  last_linked_at INTEGER,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  CHECK ((state = 'stopped') = (stopped_reason IS NOT NULL))
) STRICT;
```

`ssh_dest` and `remote_command` reject control characters (checked in `store::sync_dial_targets` before insert; `InvalidRequest` "dial target contains control characters").

**Store API** (`src/store/sync_dial_targets.rs`, consumed by ht-5fr.7 and ht-5fr.8):
- `add_explicit(tx, dest, remote_command, now) -> TargetId`: upserts with `source='explicit'`, `enabled=1`, and resets `state='idle'`, `attempts=0`, `stopped_*`, `next_attempt_at=NULL`. Re-adding a target is the operator's "changed".
- `upsert_discovered(tx, dest, source, now)`: inserts if absent. It never changes `source='explicit'` rows, `enabled`, `remote_command` or any state.
- `forget(tx, dest_or_node) -> usize`: sets `enabled=0` and deletes nothing (root §11).
- `list(conn)`, `enabled(conn)`.
- `record_transition(tx, target_id, DialTransition, now)`.
- `clear_stale_stops(tx, version)`: run once at start. It resets `stopped` rows whose `stopped_version != CARGO_PKG_VERSION` to `idle`, because a local upgrade is a "change" that may fix `envelope_unsupported`.

More than `MAX_DIAL_TARGETS = 64` enabled rows makes `add_explicit` return `InvalidRequest` "too many dial targets (64)". `upsert_discovered` skips the insert and returns `Skipped`.

**`SyncStorePort`** (`src/sync/sync_store.rs`) wraps these for the runtime: `enabled_targets`, `record_transition`, `record_contacts`. Its write methods take a FairWriter background turn internally, because they are rare and small. `MemorySyncStore` is the test fake.

**Runtime.** On start and on each rate-limited reload, `Dialer` diffs `enabled_targets()` against its tasks:
- a new target gets a `DialTask` (a tokio task);
- a removed or disabled one is cancelled, which closes its link and kills its child;
- a changed `remote_command` restarts the task.

`DialTask` loop:
1. Wait until `next_attempt_at` (immediately if NULL or past), unless the state is `stopped`. A stopped task idles until a reload changes the row.
2. `record_transition(Dialing)`. Build the `TransportCommand` and spawn it with `tokio::process::Command`: `stdin`/`stdout` piped, `stderr` piped into a 4 KiB ring buffer task, `process_group(0)`, `kill_on_drop(true)`, environment inherited from the daemon.
3. Read the preamble (§4) and hand `DialedStream` to the coordinator, which runs the handshake with `role = dialer`. The coordinator reports back over the same per-dial `report` channel (§8): first `Established{node_uuid}` once the handshake completes, then `LinkEnded{target_id, why, up_for, peer_node}` when the link ends. On `Established` the DialTask records `Linked{node}` (setting `learned_node_uuid`) at once, so `sync status` and `peer list` show the target linked while the link is up, not only after it ends. A changed uuid logs `sync peer <dest>: node changed <old8> -> <new8>`. A handshake failure delivers only `LinkEnded`, so no `Linked` is recorded.
4. **Teardown:** `killpg(pgid, SIGTERM)`, wait up to 2 s, `killpg(SIGKILL)`, then `child.wait()` to reap. This also runs on cancellation and daemon shutdown.
5. **Classify:**

| Result | Transition |
|---|---|
| `why.class() == StopDialing` | `Stopped{reason, version}` |
| child exited 127 before the preamble | `NotInstalled`, `next_attempt_at = now + 30 min` |
| anything else | `Backoff`: if `up_for ≥ 60 s`, `attempts = 0`; `delay = FullJitter::delay(attempts)`; `attempts += 1` |

   `last_error` is set from the `LinkError` display or, before the preamble, the last non-empty stderr line (≤512 bytes).

`FullJitter::delay(n) = uniform(0, min(300 s, 1 s × 2^n))`, using an xorshift seeded per task. A test seed is passed through `DialConfig`.

**`TransportCommand::build(target, dial_command: Option<&DialTemplate>) -> (program, argv)`:**
- Default: `ssh -o BatchMode=yes -o ServerAliveInterval=15 -o ServerAliveCountMax=3 -o ControlMaster=no -o ControlPath=none -o ConnectTimeout=20 -T -- <ssh_dest> <remote_command>`. `ConnectTimeout` is added to root §9's list so that a blackholed host fails in 20 s instead of the kernel's TCP timeout. `--` keeps a destination from being parsed as an option.
- `remote_command` defaults to `exec "$(command -v herdr-threads || echo "$HOME/.local/bin/herdr-threads")" peer --stdio`. ssh hands it to the remote login shell as one string.
- With `sync.dial_command`: `/bin/sh -c <template>`, where every `{dest}` is replaced by the destination single-quoted for sh (`'` → `'\''`). The target's `remote_command` is ignored in this mode. This is how ht-5fr.10 links nodes without ssh (template e.g. `exec /path/herdr-threads --state-dir {dest} peer --stdio`).

Nothing else in the daemon spawns these children (root Dialer digest).

**Considered.**
- Deleting forgotten targets: root §11 says forget deletes nothing, and the disabled row keeps discovery from re-adding it.
- Retrying stopped targets daily: root §9 says stop "until it is changed".
- Splitting `dial_command` into argv without a shell: there is no shell-words crate in tree, and the template is operator or test data, not untrusted input.
- An in-process (no child) dial for tests: it would skip the real pump.

### 14. Settings: `sync.dial_command`

**Recommended.** `InstanceSettings` (`src/daemon/settings.rs`) gains `pub sync: SyncSettings` with `#[serde(default, deny_unknown_fields)] struct SyncSettings { dial_command: Option<String> }`. ht-5fr.7 adds `discover` to the same struct. `ServiceConfig::from_settings` validates it into `DialTemplate`:
- at most 4096 bytes;
- contains `{dest}` at least once;
- no NUL or newline.

A failure refuses the start with the existing "names the file" error. The setting is read once at start, and the module doc lists the key. No other `sync.*` keys are added here.

**Considered.** An environment variable: tests already write per-node `settings.json`, and the root names the setting.

### 15. Daemon composition, shutdown and idle exit

**Recommended.**
- **Composition** (`src/app.rs::run_elected_impl`, after `journal::open` and HostSessions start, before the listener serves):
  1. register a `Lane::Sync` Pacer with `CommitKicks`;
  2. run `clear_stale_stops`;
  3. `SyncCoordinator::start(deps)`, which also constructs the Dialer inside the actor;
  4. pass `Some(peer_intake)` and the `ConnectionGauge` into `serve_with_gate`;
  5. store the coordinator's tokio `JoinHandle` in a new `sync_slot`, and add the coordinator's `LaneStatus` to `lane_statuses`.
- **Shutdown** (the existing stop closure): after `worker_stop.cancel()`, await the coordinator handle with a 3 s bound. On cancel, the actor sends `Close{shutdown}` on every link (1 s), ends the link tasks, cancels every DialTask (each runs its kill-and-reap teardown), and flushes contacts once (best effort). A handle still running at 3 s is aborted and logged. The children are already in their own process groups and `kill_on_drop`, and teardown kills by group.
- **Idle exit** (new; `src/daemon/idle_exit.rs`). There is no idle exit today (verified in `src/app.rs` and `src/daemon/lifecycle.rs`), so root §1's "existing idle period" is fixed here as `IDLE_EXIT_AFTER = 10 min`, and the rule stays narrower than root §1's wording allows. A daemon exits only when **all** of these hold at every 30 s check for 10 continuous minutes:
  - `HostSessions::attached_count() == 0` (store-only);
  - `SyncHandle::link_count() == 0` (established or handshaking, inbound or dialed);
  - `SyncHandle::enabled_target_count() == 0` (owned by the Dialer leaf, ht-5fr.5.7);
  - `ConnectionGauge == 0`.

  Any failed check resets the timer. On exit it logs one line, `idle exit: store-only, no peer links, no dial targets, no clients for 600 s`, and cancels the same `shutdown` the `Stop` path uses, so the descriptor, the lock and the drain behave as on `daemon stop`.

  A daemon with any attached Herdr server never idle-exits (today's lifetime), and neither does a daemon with a dial target or a link. The next local CLI use or the next inbound `peer --stdio` starts it again through `ensure_running`. Test-support `ServiceConfig::with_idle_exit_after(Duration)` shortens the period, and `Duration::ZERO` disables it.

**Considered.**
- No idle exit at all: then a store-only daemon started by one inbound link would live forever on a machine that is otherwise unused. That is not harmful but is wasteful, and the bead owns the rule.
- Exiting with an attached but idle Herdr server: wakes need a running daemon, and nothing local would restart it before a needed wake.
- Including pending apply work in the check: parked entries are permanent, so "nothing pending" may never hold, and a store-only daemon with no links finishes its apply backlog in seconds.

### 16. Surfaces for sibling subepics

**Recommended.** `SyncHandle` (cloneable) exposes:
- `snapshot() -> SyncSnapshot { links: Vec<LinkStatus{ node, tag, role, mode, dest: Option<String>, since, rx_logs: u32, unsolicited: u64, last_error: Option<String> }>, paused_logs, headroom_until }`, for ht-5fr.8's `sync status`. It is bounded at 64 links and answered by the actor within the caller's budget, or `StoreBusy`.
- `signals() -> Arc<SyncSignals>`, for ht-5fr.13.
- `link_count()`.
- `reload_targets()` (owned by the Dialer leaf, ht-5fr.5.7), an explicit nudge that ht-5fr.8's `peer add` does not need, because the table kick covers it, but which ht-5fr.7 may call after a discovery batch.

ht-5fr.6 extends `PeerFrame` behind `fork.v1`, implements `ForkSink`, and may request a branch pull through a new coordinator method. Those are its additions. ht-5fr.10 consumes `sync.dial_command` and `peer --stdio` as specified.

## Testing

- **Frames** (`src/sync/frame.rs` unit):
  - every `PeerFrame` round-trips;
  - an unknown `t` decodes to `Unknown`, and unknown fields are tolerated;
  - `decode_hello` rejects non-hello frames and oversized hellos;
  - the `CloseReason::class` table;
  - an unknown reason → `unknown`/`Retry`.
- **Link** (`src/sync/link.rs` unit, `tokio::io::duplex`, paused time):
  - preamble found after 0 and 1000 noise bytes (noise count reported);
  - over 64 KiB → `PreambleNoise`; 30 s → `PreambleTimeout`;
  - a successful handshake in both roles with intersected caps and min limits;
  - each refusal (`envelope_unsupported`, `auth_failed` for a bad sig and for a uuid/pubkey mismatch, `self_link` for the same identity and boot, `node_collision` for the same identity and a different boot, `busy`) with the correct `LinkError`;
  - a replayed `Auth` from another link fails;
  - credit: two 400 KiB frames go, the third waits for `Credit`, a single 900 KiB frame goes when `outstanding == 0`, and `ping` is answered while data is blocked;
  - a ping after 30 s of silence and `Dead` at 90 s;
  - a handshake frame after establishment → `protocol_error`.
- **Coordinator** (`src/sync/coordinator.rs` tests, `MemoryJournal` + `MemorySyncStore` + duplex links, no processes):
  - two nodes converge (vectors equal);
  - the line A–B–C relays, and an entry `author_raw`'d on A after convergence reaches C through live tail with no new `Want`;
  - the rx source prefers the origin;
  - failover after 30 s with no progress sends `Unwant`/`Want` to the other source;
  - a triangle ingests each entry exactly once per node (no amplification);
  - two A–B links: only the minimal one carries Have/Want, ending it promotes the standby with no new handshake, and a third link is closed `busy`;
  - clone at a third node: two links carrying B's `node_uuid` with different boots and a live older link → the newer is closed `node_collision` and `on_clone(ThirdParty)` is recorded; with a silent older link (paused time, no pong within 10 s) → the older ends `Dead` and the newer is ranked active with no report; a link refused or closed with `node_collision` reports `on_clone(subject: self)`;
  - own-history signal: a peer advertising this node's other incarnation beyond local sets `Links`; ingesting up to the advertised counter, or that link ending, clears it; no own log advertised never sets it;
  - Have deltas are throttled to ≤1/s;
  - `Corrupt` penalizes and switches source;
  - `Headroom` pauses every want for 60 s;
  - `Fork` calls the sink and pauses the log;
  - an unsolicited `entries` frame is dropped and its credit granted;
  - `round_state` goes `Pending` → `Complete`, and `request_round` resets it; a `request_round` before any link is up reports `NoLinks` until the first link activates, then `Pending` until its round completes;
  - `Seen` relative ages give C a relayed contact for A in a line, and the contact flush writes only on a ≥30 s advance or a threshold crossing.
- **Dialer** (`src/sync/dialer.rs` unit):
  - exact ssh argv including `--` and the default remote command;
  - `dial_command` quoting of a dest containing `'` and spaces;
  - `FullJitter` bounds with a fixed seed;
  - the attempts reset after 60 s up;
  - exit 127 → `NotInstalled` with +30 min;
  - StopDialing → `Stopped`, which survives reload until `add_explicit` or a version change clears it.
- **Dialer process ownership** (`tests/daemon/sync_dialer.rs`, test-support): with `dial_command` `exec sh -c 'printf "HTPEER1\n"; exec sleep 600'`, the handshake times out, teardown kills the group and reaps it, and cancellation mid-dial leaves no process. Afterwards `scripts/check-no-leaked-processes --run-id "$HT_LEAK_RUN_ID"` finds nothing.
- **Store** (`tests/store/sync_dial_targets.rs`, `tests/store/sync_contacts.rs`, modules of `tests/combined.rs`):
  - the CHECKs (leading `-`, control chars, stopped ↔ reason);
  - explicit beats discovered, and discovery neither re-enables nor overwrites;
  - forget disables only;
  - the 64-target cap;
  - `clear_stale_stops`;
  - `reach` thresholds and `since`;
  - the `kicks` exhaustive classification test passes with the new tables.
- **Transport** (`tests/daemon/transport.rs`):
  - a hello first frame reaches `PeerIntake` without consuming an ordinary permit;
  - the 17th concurrent peer gets `Close{busy}`;
  - a `None` intake gives `busy`;
  - the uid check still runs first;
  - `ConnectionGauge` counts ordinary and watch connections but not peer connections.
- **`peer --stdio`** (`tests/cli/peer_stdio.rs`, test-support, own `[[test]]` only if it needs process-wide state, else in `tests/integration.rs`):
  - the pump spawned through `test_support::spawn` prints `HTPEER1\n` first;
  - a dialer-role PeerLink over its stdio completes the handshake with the daemon;
  - stdin EOF exits the pump 0 and the daemon logs the link down;
  - `daemon stop` makes the pump exit;
  - with no daemon startable (unsafe state dir) it exits 3 with no stdout bytes.
- **Two daemons** (`tests/daemon/sync_link.rs`, test-support; both daemons and pumps owned and stopped):
  - with `sync.dial_command = "exec <bin> --state-dir {dest} peer --stdio"` and an explicit target inserted before start, the nodes exchange genesis logs and their vectors converge;
  - killing the remote daemon gives a backoff transition, and restarting it reconnects;
  - a store-only daemon with no target idle-exits with a 2 s test period, and one with a target does not.
- **Checks:** `nice cargo clippy --locked --all-targets --all-features -- -D warnings`, `scripts/check-harness-neutral`, `nice scripts/check-default-features`, focused `nice cargo test --locked --all-features sync`.

Suggested task cut (files touched):
1. Frames and CloseReason: `src/sync/frame.rs`.
2. PeerLink handshake, preamble, credit and liveness: `src/sync/link.rs`.
3. Baseline tables and store modules: `migrations/node/0001_node_baseline.sql`, `src/store/sync_dial_targets.rs`, `src/store/sync_contacts.rs`, `src/store/mod.rs`, `src/sync/sync_store.rs`.
4. `Lane::Sync` and the table map: `src/service/kicks.rs`, `src/app.rs` (pacer arrays).
5. SyncCoordinator, ingest worker and signals: `src/sync/coordinator.rs`, `src/sync/signals.rs`.
6. Dialer and transport command: `src/sync/dialer.rs`.
7. Settings `sync.dial_command`: `src/daemon/settings.rs`, `src/service/config.rs`.
8. Transport sniff, `PeerIntake` hand-off and `ConnectionGauge`: `src/daemon/transport.rs`, `src/daemon/transport/peer_connection.rs`.
9. `peer --stdio`: `src/cli/peer.rs`, `src/cli/mod.rs`, `src/main.rs`.
10. Composition, shutdown and idle exit: `src/app.rs`, `src/daemon/idle_exit.rs`.
11. Integration tests: `tests/daemon/sync_link.rs`, `tests/daemon/sync_dialer.rs`, `tests/cli/peer_stdio.rs`.

Order: 1 → 2 → (3, 4, 7) → 5 → 6 → 8 → 9 → 10 → 11.

## Key abstractions

**PeerIntake** — new struct (`src/sync/coordinator.rs`), the bounded hand-off from the daemon transport to the coordinator for sniffed inbound peer connections.
- Owns: the 16-slot `mpsc` sender, and the `PeerStream { stream, first_frame, permit }` value that carries the peer permit with the stream into the coordinator's `JoinSet`.
- Hides: that the receiver is an actor; the transport only offers and either hands over or gets "busy".
- Must not: run the handshake or hold the stream after a successful offer; the transport task returns at once, so the 250 ms drain never waits on a link.
- Relationships: created by `SyncCoordinator::start`, passed as `Option` into `serve_with_gate`; `peer_connection::hand_off` is its only caller. `None` (coordinator not started or stopped) and a full or closed channel both end in `Close{busy}`.
- Lifecycle: lives as long as the coordinator; the transport holds a clone until it stops serving.
- Errors: a refused offer is one outcome (busy), not an error the transport tells apart further.

**SyncStorePort / MemorySyncStore** — new trait plus test fake (`src/sync/sync_store.rs`), the coordinator's narrow store port for its own two tables.
- Owns: `enabled_targets`, `record_transition` and `record_contacts` for the runtime; each write takes its own FairWriter background turn.
- Hides: SQLite, the background turn and the store modules behind it.
- Must not: grow into a general store API; `StorePort` and `JournalPort` stay unextended. Operator writes (`add_explicit`, `forget`) and discovery (`upsert_discovered`) go straight to `store::sync_dial_targets` from ht-5fr.8 and ht-5fr.7, not through this port.
- Relationships: implemented by `SqliteStore` (delegating to `store::sync_dial_targets` and `store::sync_contacts`) and by `MemorySyncStore` for coordinator and dialer tests (the spec's named second implementation).
- Errors: store errors only; the Dialer logs a failed `record_transition` and keeps its in-memory state, and a failed contact flush retries at the next flush.

**DialTargets (`store::sync_dial_targets`)** — new store module, the only owner of `sync_dial_targets`.
- Owns: the row rules: control-character and leading-`-` rejection, explicit-beats-discovered precedence, forget-disables-only, the 64-target cap, the `stopped` ↔ reason invariant, `clear_stale_stops` by `CARGO_PKG_VERSION`, and applying a `DialTransition` (closed enum: `Dialing`, `Linked{node}`, `Backoff{attempts, next_at, error}`, `NotInstalled`, `Stopped{reason, version}`) to state, attempts, `next_attempt_at` and error columns.
- Hides: the table shape and the precedence `ON CONFLICT` forms.
- Must not: compute backoff delays or classify link errors (the Dialer decides the transition; this module only records it); must never delete a row.
- Relationships: used by SyncStorePort's SQLite impl, by ht-5fr.8's `peer add/forget/list` and by ht-5fr.7's discovery. The `CloseReason` subset it stores in `stopped_reason` is the `StopDialing` class.
- Errors: `InvalidRequest` (control characters, cap reached) is kept apart from `Skipped` (discovery over the cap, not an error) and from store errors.

**ContactBook / SyncContacts (`store::sync_contacts`)** — new struct (`src/sync/signals.rs`) plus store module, per-node reachability: the in-memory relative-age gossip and the one persisted reach rule.
- Owns: ContactBook holds `node → (last_contact_wall_ms, Direct | Relayed)`, records direct contact from inbound frames, builds `Seen` ages (self at 0, the receiving link's node excluded), merges received ages only when newer, and decides which rows are due for a flush (≥30 s advance or a threshold crossing, at most every 10 s). SyncContacts owns `sync_node_contact`, the `MAX(last_contact_at)` upsert, `UNREACHABLE_AFTER_MS` and `reach() -> NodeReach::{Reachable, Unreachable{since}}`.
- Hides: that gossip carries ages and never absolute times; the threshold.
- Must not: compare wall clocks across nodes; ContactBook must not be shared outside the actor (only the table is read elsewhere).
- Relationships: the actor owns the ContactBook and flushes through SyncStorePort; ht-5fr.13's `store::queries` and daemon-less `doctor` call `reach` directly.
- Lifecycle: ContactBook is rebuilt empty on each start; rows persist.

**SyncSignals** — new struct (`src/sync/signals.rs`), the in-memory "first sync round complete" signal.
- Owns: per active link the round start and completion, `request_round(at)` and `round_state(since) -> RoundState::{NoLinks, Complete, Pending{links}}`.
- Hides: the completion rule (a full `Have` after the start, and every advertised-ahead log caught up or unwantable).
- Must not: persist anything or block; it is a lock-protected snapshot the actor writes and the deadline lane reads.
- Relationships: written only by the SyncCoordinator actor; `SyncHandle::signals()` hands the `Arc` to the deadline-lane composition in `app.rs` (ht-5fr.13). `request_round` records the request and wakes the actor, which sends `Have{full, ask: true}`.
- Lifecycle: created by `SyncCoordinator::start`, shared as `Arc` for the daemon's life.

**ForkSink** — new trait (`src/sync/coordinator.rs`), where the coordinator reports a fork and learns that a paused log may resume.
- Owns: the two-call contract `on_fork(log, evidence, source) -> ForkAck::{Paused, Corrupt}` (amended by [incarnations design §12](2026-10-14-cross-node-sync--incarnations-design.md#12-runtime-sink-worker-coordinator-changes)) and `resumed()` (polled on the 1 s tick), plus the clone report `on_clone(CloneReport)` (default no-op, §8).
- Hides: fork and clone policy entirely; the coordinator only pauses and unpauses wants and closes a clone's newer link.
- Must not: be called for anything but `IngestOutcome::Fork`, the fork frames ht-5fr.6 adds, and clone reports; the coordinator never freezes, re-issues or acts on clone evidence (root SyncCoordinator "must not").
- Relationships: `LogOnlyForkSink` (rate-limited log line, never resumes) now; ht-5fr.6's Incarnations implementation next. Chosen once in `CoordinatorDeps` in `app.rs`.
- Errors: none; implementations log their own failures, and an unresolved fork stays paused.

**PeerLink** — extended: concrete `PeerFrame` (internally tagged by `"t"`, with `#[serde(other)] Unknown` ignored and counted, so it is closed for matching but open on the wire behind caps), `PeerHello`, `CloseReason` with `class() -> CloseClass::{StopDialing, Retry}` (plus `unknown`), the closed `LinkError` whose `class()` delegates to `CloseReason`, the domain-separated two-hello Auth signature, `LocalHello`/`Established`/`PeerInfo`, a `LinkHandle` (control queue before byte-credited data queue) and `LinkEvent` upward. It handles `ping`, `pong`, `credit` and `unknown` itself and forwards all other frames; it decides nothing about routing. Generic over `AsyncRead + AsyncWrite` (ssh child stdio, accepted `UnixStream` halves, `duplex` in tests), not a trait object.

**SyncCoordinator** — extended: a single tokio actor owning `links`, `by_node`, `rx`, `local`, the ContactBook and SyncSignals writes; `CoordinatorDeps` as its one composition point; a sequential ingest worker (one FairWriter background turn at a time, per-log order kept) and `spawn_blocking` for every `JournalPort`/SyncStorePort call; one exhaustive `match` on `IngestOutcome`; duplicate-link order `(dialer node_uuid, dialer nonce)` computed locally on both ends; third-node clone detection (same `node_uuid`, different boot, live older link → `Close{node_collision}` on the newer and `ForkSink::on_clone`); the own-history `Links` flag; `Unwant`. `SyncHandle` (cloneable facade) exposes `snapshot`, `signals`, `link_count`, `reload_targets` and the enabled-target count. `reselect` and the duplicate-link order are pure functions over actor state so they test without links.

**Dialer** — extended: constructed inside the actor and driven by its reload diff; one `DialTask` per enabled target owns its child (`process_group(0)`, `kill_on_drop`, killpg TERM→KILL, reap) on every exit path including cancellation; `TransportCommand::build` is the one place argv is built (ssh default or `DialTemplate` via `/bin/sh -c` with sh-quoted `{dest}`); `FullJitter` reuses `service::pacer::Backoff`'s xorshift; the only place a `LinkError`/child exit becomes a `DialTransition`.

**Daemon transport** — extended: the hello sniff after the watch sniff, the `MAX_PEER_CONNECTIONS` semaphore, `peer_connection::hand_off` into PeerIntake, and `ConnectionGauge` (an atomic count of non-peer connection tasks with a drop guard), which only idle exit reads.

**Daemon idle-exit** — extended (per spec §15 it is new code: no idle exit exists today, so the root's "existing idle period" is created as `IDLE_EXIT_AFTER = 10 min`): `IdleExit` in `src/daemon/idle_exit.rs` checks every 30 s that hosts, links, enabled targets and `ConnectionGauge` are all zero for 10 continuous minutes, then cancels the same `shutdown` as `Stop`. `ActivityProbe` is a plain struct over `HostSessions`, `SyncHandle` and `ConnectionGauge`, not a trait. It reads the target count through `SyncHandle`, because the Dialer lives inside the actor (spec §15 writes `Dialer::enabled_targets()`).

**Lane (`service/kicks.rs`)** — extended: `Sync` at bit 7; `journal`, `sync_logs`, `sync_dial_targets` map to it; `sync_node_contact` is in `KNOWN_UNMAPPED`; `kicks_for_commit` keeps `Sync` for Sync- and Apply-origin commits.

**InstanceSettings / ServiceConfig** — extended: `SyncSettings { dial_command }` validated once into `DialTemplate`; test-support `with_idle_exit_after`.

Variation: what carries a link's bytes varies (ssh child, accepted socket, test duplex) → PeerLink is generic over the stream, and the Dialer and transport are the only two places a link starts. Fork handling varies (log-only now, Incarnations in ht-5fr.6) → the `ForkSink` trait, chosen once in `CoordinatorDeps`. The sync store backend varies between SQLite and tests → `SyncStorePort` with its two named implementations. Transport commands vary only as data (`TransportCommand::build`, `DialTemplate`), with no trait. Peer frames grow across releases but each node matches a fixed set → closed `PeerFrame` with an `Unknown` catch-all, and new frames are variants gated by a cap string. Close reasons, link errors, dial transitions, dial sources, target states, ingest outcomes, `NodeReach` and `RoundState` are fixed → closed enums or CHECKs with `match`. `peer --stdio` is a plain two-thread splice with no abstraction (root, spec §2).

Walk-through: A has an explicit target `b-host`. At start `clear_stale_stops` runs, `SyncCoordinator::start` builds the Dialer, and it reads `enabled_targets` through SyncStorePort and spawns a DialTask. The task records `Dialing`, spawns `ssh … -- b-host 'exec … peer --stdio'` in its own process group and reads the preamble. On B, the pump runs `ensure_running`, connects and writes `HTPEER1\n`. B's transport sniffs A's hello, counts it outside `ConnectionGauge`, and offers a `PeerStream` to B's PeerIntake. Both sides run the PeerLink handshake, check the two-hello Auth signatures, and reach `Established`. Each actor ranks the link as active and sends a full `Have`. A sees B ahead on log L, and `reselect` picks B (the origin) and sends `Want`. B's actor serves committed rows through `spawn_blocking` and `try_send_entries` within the credit window. A's ingest worker takes one background turn, and `JournalPort::ingest` returns `Stored`. The actor grants credit, advances `local`, records contact in the ContactBook, and pushes to other subscribers of L. The commit kicks `Lane::Sync` for Have deltas. The round completes, so SyncSignals reports `Complete`. Later, killing B gives `LinkError::Dead` or `Io`. The coordinator reports `LinkEnded` to the DialTask, which kills and reaps the group and records a `Backoff` transition with `FullJitter`. Likely future change: ht-5fr.6 resolves forks. That is one new `IncarnationsForkSink` implementing `ForkSink` plus one line in `CoordinatorDeps` in `app.rs`. Its branch-pull frames are new `PeerFrame` variants behind `"fork.v1"` (spec §16 also allows one new coordinator method). PeerLink framing, the Dialer, DialTargets, SyncStorePort and the ingest worker are untouched.

Per spec §15: the root lists daemon idle-exit as "extended", but no idle exit exists, so this subepic creates it under the root's name.
Per spec §8/§10: the actor plus one ingest worker serializes all ingest node-wide, which is deliberate because FairWriter would serialize it anyway.

Digest:
- PeerIntake: owns the bounded transport→coordinator hand-off of sniffed peer streams with their permits; inbound link admission rules go here and in `peer_connection::hand_off`;
  the transport never runs a handshake or keeps a stream it offered.
- SyncStorePort/MemorySyncStore: owns the coordinator's narrow writes and reads of dial targets and contacts, with their FairWriter turns; new runtime sync-table access is a method here;
  the coordinator never touches `SqliteStore`, `StorePort` or SQL directly.
- DialTargets (`store::sync_dial_targets`): owns `sync_dial_targets` rows, precedence, cap, stop clearing and recording transitions; new target columns or sources go here;
  callers never write the table directly or delete rows.
- ContactBook/SyncContacts: own relative-age reachability gossip and the persisted `reach` rule; new reachability rules go in `store::sync_contacts::reach`;
  nothing gossips absolute times or computes reachable/unreachable itself.
- SyncSignals: owns the round-complete state and `request_round`; new sync readiness signals go here;
  readers never inspect coordinator link state to decide whether sync caught up.
- ForkSink: owns the fork hand-off contract; new fork handling is a new implementation chosen in `CoordinatorDeps`;
  the coordinator never freezes, re-issues or resumes a log on its own.

## Post-Implementation Notes

> *As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*

- 2026-10-16 (ht-5fr.54): §12's round_state counted only active links. Root spec §7 (and its Deadline-lane line) requires every linked peer and every enabled dial target, and wins. round_state is now Complete only when every active link completed a round since `since` and every enabled dial target's learned node is among those links; an unlinked target counts in Pending.links. The Dialer publishes its enabled targets and learned nodes into SyncSignals. A target that never links holds overdue emission until the 60 s cap.
