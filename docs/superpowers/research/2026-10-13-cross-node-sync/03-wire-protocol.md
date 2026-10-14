# 03 — Peer wire protocol over `ssh <dest> herdr-threads peer --stdio`

Research for the cross-node threads vision (`docs/superpowers/specs/2026-10-13-cross-node-sync-vision-design.md`), research question 3. Deep mode, 2026-10-13.

## Recommendations for herdr-threads

1. **Make `peer --stdio` a thin byte pump into the remote daemon's Unix socket.** It resolves the instance, runs `ensure_running` (which already spawns a detached daemon, `src/daemon/lifecycle.rs` `spawn_detached`), connects, and splices stdin/stdout to the socket. The daemon tells a peer connection apart by its first frame (`PeerHello`), the way it already does for `WatchWireRequest`, and admits it under its own semaphore (e.g. `MAX_PEER_CONNECTIONS = 16`) [R1, E27]. The daemon stays the only SQLite writer and the only decider (A2). The pump keeps no state and exits on EOF from either side.
2. **Reuse the local framing (u32 BE length + JSON, 1 MiB cap), behind a magic preamble.** Before the first frame, the remote writes one fixed line, `HTPEER1\n`. The dialer discards up to 64 KiB of leading bytes while looking for it, then reports anything it skipped as "remote shell printed N bytes" (the rsync "is your shell clean?" failure [E21]; Syncthing uses a magic number the same way [E5]).
3. **Negotiate capabilities, not versions.** `PeerHello{magic, envelope: 1, node_id, host_key_fp, instance_uuid, epochs, caps: [..], limits: {max_frame, max_batch_bytes}}`. Unknown caps are ignored, as git v2 requires clients to ignore unknown keys [E13]. The link is refused only for an envelope it does not speak, a node-id collision, or a connection to itself. A refusal is sent as a typed `Close{reason}` frame before the link drops, as Syncthing does [E8].
4. **Keep the vector exchange symmetric, with EBT-style request skipping.** After the hello, each side sends `Have{origin -> (epoch, contiguous_counter)}`. Each side persists the last vector a peer sent and on reconnect sends only the origins that changed [E1]. With ≤10 nodes the full vector is ~200 B, so skipping is an optimisation, not a requirement.
5. **Combine pull and push on the same link.** The receiver asks for each missing range with `Want{origin, epoch, from}`, which works as a cursor, not a fixed range. The sender streams `Entries` batches in counter order. Once caught up, the subscription stays open (live tailing): each local or relayed append for that origin is pushed as soon as it is ingested. Each `Entries` batch also tells the receiver where to resume. `Want` again with the new `from` is idempotent, so resuming mid-range needs no extra state.
6. **Pull each origin from one peer at a time.** At most one link at a time is the active (`rx`) source for an origin, as in EBT [E2]. Fail over when the source stalls past a timeout [E3]. Other links get vector updates only. This removes most duplicate traffic in a mesh and is also what stops relay loops: an entry `(origin, counter)` at or below the local contiguous counter is dropped on arrival, so a cycle cannot amplify.
7. **Bound batches and use credit-based backpressure.** A batch is ≤256 KiB or ≤256 entries, and always carries at least one entry. The receiver grants credit (`Credit{bytes}`, e.g. a 1 MiB window) after committing a batch. The sender never has more than one window in flight, so neither the SSH channel buffer nor the daemon's memory grows without bound.
8. **Detect dead links in two layers.** The dialer always passes `-o ServerAliveInterval=15 -o ServerAliveCountMax=3 -o BatchMode=yes -o ControlMaster=no -o ControlPath=none` (≈45 s SSH-level detection [E15, E16]). On top, an application `Ping`/`Pong` runs every 30 s when the link is otherwise silent. A link is declared dead after 90 s with no inbound frame. Syncthing pings every 90 s when the link is idle [E7]. Mosh's SSP heartbeat interval is 3 s [E25].
9. **Reconnect with capped exponential backoff and full jitter** (AWS found Full Jitter does the least work [E23]): base 1 s, cap 5 min, reset after the link has stayed up for 60 s. The NATS loop-detection precedent applies a fixed 30 s delay [E11].
10. **Keep JSON for frames, and relay payloads byte for byte.** Journal entries keep their payload as the original canonical JSON text, carried with `serde_json::value::RawValue`, so a relay never re-encodes a kind it does not understand. At 1–10 msg/s, JSON costs at most a few KB/s. Compress only for bulk catch-up (an optional `zstd` capability), not by switching codec.

### Risks flagged against the settled decisions

- **R-A: which remote instance answers.** A node is one Herdr *instance* (`instance_dir = sha256(host_endpoint)`), but `ssh dest cmd` runs without `HERDR_SOCKET_PATH` or `HERDR_PLUGIN_STATE_DIR` [E29]. In that environment, detection falls back to defaults and then refuses named sessions or a stopped Herdr server (docs/install.md:188). A machine with two Herdr servers is ambiguous. A remote whose Herdr server is down cannot be resolved at all, so its store cannot sync even though sync needs no panes. **Mitigation:** a peer record stores the remote `--state-dir/--host-endpoint` (learned on the first successful hello). `peer --stdio` must be able to run the daemon in store-only mode while the host is down. Otherwise "converge once any path returns" depends on Herdr uptime.
- **R-B: remote `PATH`.** The command runs in place of a login shell, so profile files do not load [E20]. `~/.local/bin`, the installer's link target (README), is often missing from `PATH` there. **Mitigation:** add a per-peer `remote_command`, the same as rsync's `--rsync-path` and git's upload-pack override [E22], defaulting to `herdr-threads` with a fallback to `$HOME/.local/bin/herdr-threads`.
- **R-C: ControlMaster.** A user's `ControlMaster auto`/`ControlPersist` can attach the daemon's link to a hung master, which then "hangs forever" [E17, E18]. The dialer must disable multiplexing explicitly (rec. 8). The cost is a new SSH handshake per reconnect, which is negligible at this scale.
- **R-D: orphaned remote pumps.** Without a pty, a remote process learns that ssh died only when it writes to the pipe (SIGPIPE) or reads EOF [E19]. A quiet pump can outlive its link. That is harmless for state, but it holds a peer slot. **Mitigation:** the pump exits on stdin EOF, on any write error, and when the daemon closes the socket. The daemon's 90 s idle timeout (rec. 8) closes the socket, so the pump is reaped at the latest then.
- **R-E: "only the dialer reconnects" plus auto-discovered peers.** When A dials B and B dials A, two links exist. The design is still correct, because ingest is idempotent and the single-`rx` rule (rec. 6) avoids double transfer. Still, dedupe by `(node_id, instance_uuid)` keeps one *active* link per peer pair. Recommended rule: the link dialed by the lower node id wins, and the other is idled, not closed, so it can take over without a new SSH handshake.
- **R-F: restored-from-backup origin.** A per-origin `epoch` in the vector, as Syncthing's index ID [E6], is needed for the wire to detect counter reuse. It overlaps with research question 9.

## 1. Grounding: what exists locally

- **Framing:** `read_frame`/`write_frame` use a u32 big-endian length followed by a JSON body. `MAX_FRAME_BYTES = 1 MiB − 4`; an empty frame or one over the cap is an error (`src/daemon/transport.rs:30,73-100`) [E27].
- **Versioning:** `PROTOCOL_VERSION: u16 = 7`. The match is exact; on a mismatch the daemon sends a minimal `UnknownWireVersion` "skew reply" (`src/protocol/wire.rs:37,82`; `transport.rs:269`) [E28]. Optional additions are already gated by named capabilities through `Command::Capabilities` (`src/protocol/capabilities.rs:1-4`) [E31]. The peer link applies that capability model from the first frame. The spec's versionless rule rules out the exact version.
- **Long-lived connections:** watch connections are sniffed from the first frame and admitted by their own semaphore (`MAX_WATCH_CONNECTIONS = 64`, `src/protocol/watch.rs:70`). They define reconnect grace and exit codes (stream ended / retry / stop) [R3]. A peer connection fits this pattern.
- **Auth:** every Unix-socket connection is checked with `peer_cred().uid == owner_uid` (`transport.rs:321`). A local pump spawned by sshd runs as the same user, so the existing check admits it with no new trust code. That is consistent with "SSH is the only boundary".
- **Daemon start:** `ensure_running` probes the daemon, then spawns `daemon run --state-dir --host-endpoint` detached in its own process group, with stdin/stdout set to null (`lifecycle.rs:346-390`). That detachment matters: the daemon must not inherit the ssh session's pipes, or sshd waits on it and the link never sees EOF.
- **Sizes:** message bodies are ≤64 KiB (`src/store/messages.rs:55`) [E30]. The worst case is JSON `\u00XX` escaping of control bytes, 6×64 KiB ≈ 384 KiB, so any single message entry fits a 1 MiB frame. The authoring node should enforce an encoded-entry cap (e.g. 512 KiB) so that a future kind cannot exceed one frame.

## 2. Transport: how `peer --stdio` reaches the daemon

The prior art splits into two shapes. In git, rsync and SSB-over-ssh, a per-connection server process runs over the SSH channel and talks to the client directly [E32]. In Mosh, SSH is used only to start a server, and the server then talks over its own transport [E26]. herdr-threads already has a long-lived daemon that owns the store, so a third shape fits best: **a stateless proxy into the existing daemon.**

| Option | Pros | Cons |
|---|---|---|
| **Proxy into the daemon socket** (recommended) | One writer and one decision point (A2). Reuses admission, uid check and shutdown drain. Live tailing comes from the daemon's in-memory append notifications. | Requires a running daemon, and therefore a resolvable instance (R-A). |
| Own sync endpoint (the pump opens SQLite itself) | Works with no daemon or Herdr. | Two writers on one store. Duplicates ingest, materialisation and delivery triggers (the delivery policy must run on ingest at the recipient's home node). Live tail needs polling. |
| Mosh-style: ssh starts the daemon, which then listens on TCP | — | New listener and new auth. Breaks "SSH is the only boundary". |

Pump lifecycle:

1. Resolve the instance: `--state-dir/--host-endpoint` passed by the dialer, else default detection.
2. Call `ensure_running`, which is bounded at 5 s today.
3. Connect, write the magic line to stdout, then splice both directions. Read and write directly with no line buffering, and flush after every write. The Rust `std::io::Stdout` is line-buffered, so the pump should use raw fd copies (`tokio::io::copy_bidirectional` on the socket and `tokio::io::stdin/stdout`). The dialer reads the ssh child's stdout the same way.
4. Exit 0 on EOF from either side and non-zero on error. Stderr goes back to the dialer, which keeps the last few KiB for `doctor`/`peer status`.

On the dialing side the daemon spawns `ssh` as a child with piped stdin/stdout and a process group it owns (AGENTS.md requires that every helper process is stopped). It does **not** go through the pump: the daemon talks to the ssh child's pipes directly.

## 3. Handshake

```
dialer → remote:   [skip garbage] ← "HTPEER1\n" from remote pump
both directions:   PeerHello{
                     envelope: 1,                 // the only thing that can refuse
                     node_id: "k7q", host_key_fp, instance_uuid,
                     caps: ["journal.v1","vector.skip_v1","compress.zstd_v1", ...],
                     limits: {max_frame: 1048576, max_batch_bytes: 262144, credit: 1048576},
                     link_role: "dialer"|"dialed", link_nonce }
both:              PeerAccept{chosen: {compress: "none"}} | Close{reason}
```

- **Symmetric hellos.** Both sides send `PeerHello` immediately, which costs one round trip instead of two. EBT has the server send first so that a client can get an error before it sends a large clock [E4]. Here the hello is small and the vector is a separate frame, so ordering does not matter.
- **Refusals** are typed. `envelope_unsupported` stops the dialer's retries until a software change. `node_id_collision` is also permanent. Another case is a hello whose `node_id` and `host_key_fp` match the local node's own, as when `ssh` resolves to a localhost alias. Then `self_link` is permanent for that destination. `busy` is retryable. This mirrors the watch exit-code split (stream ended / retry / stop) [R3].
- **Capability choice** follows the git v2 rule that unknown keys are ignored [E13]. Features are the intersection. NATS leafnodes exchange `INFO` both ways and agree compression the same way [E10]. Do not use SSH `SendEnv` for negotiation (git's `GIT_PROTOCOL` approach): it needs `AcceptEnv` on the server [E14].

## 4. Vector exchange, pull, push and tailing

After `PeerAccept`, each side sends `Have{vec: {origin: [epoch, counter]}, skip_base?}`.

- **Request skipping (EBT).** Persist the last `Have` received from each peer node. On reconnect, send only origins that changed since then, flagged `partial: true`. The other side answers with any origins it needs [E1]. This is optional at 10 nodes, but cheap.
- **Want/stream.** For each origin where the peer has more, the receiver sends `Want{origin, epoch, from: local+1}` to *one* link (the `rx` source) [E2]. The sender streams `Entries{origin, epoch, entries:[...], next}` in counter order. Syncthing's delta index makes the same requirement: data must be sent in increasing sequence order for `{id, max_seq}` resumption to be sound [E6].
- **Tail.** Once a `Want` is satisfied, it stays subscribed. New appends for that origin are pushed as they are ingested, which keeps the 1–10 msg/s path at one-frame latency. Automerge's sync loop runs until neither side has anything to send. It assumes "a reliable in-order stream" [E12], which an SSH channel provides. The open subscription is the streaming form of that loop.
- **Vector updates.** `Have` deltas are sent after ingest, throttled to about 1/s, so that non-`rx` peers know who holds what and can take over.
- **Resume.** `Entries` are applied in per-origin FIFO order. Each batch is committed in one SQLite transaction, then credit is granted. After a reconnect, the next `Want.from` is simply the committed contiguous counter. Nothing in flight needs replaying, and a re-sent entry is a no-op, because `(origin, counter)` is its primary key.
- **Epoch.** If `Have.epoch` for an origin differs from the stored one (the origin was restored from backup or recreated), do not merge counters. Surface the mismatch and refuse that origin only. Syncthing generates a new index ID when the sequence resets [E6]. Policy belongs to research question 9; the wire must carry the epoch.

## 5. Batching, backpressure, compression

- A batch is ≤256 KiB encoded or ≤256 entries, and always carries ≥1 entry. A frame stays ≤1 MiB.
- **Credit window:** the sender may have at most `credit` bytes of `Entries` unacknowledged. The receiver returns `Credit{bytes}` after committing. Without this, a slow SQLite commit on a laptop lets the SSH channel and kernel pipe buffers fill, and pings queue behind bulk data. Mosh's design point is the same: keep network buffers from filling, so that interrupts stay responsive [S17]. Control frames (`Ping`, `Have`, `Close`) are never credit-gated, and the sender interleaves them between batches.
- **Compression:** an optional `compress.zstd_v1` capability compresses `Entries` frames above about 4 KiB. Syncthing compresses per message with a header flag [E9], and NATS leafnodes negotiate s2 [E10]. The user's own `ssh -C` (zlib) also works transparently. Compression matters only for first contact and backfill.

## 6. Encoding choice and size estimates

The benchmark below is the "log" dataset of rust_serialization_benchmark: small records with many strings, which is close to message metadata [E24].

| Codec | Raw | zlib | zstd |
|---|---|---|---|
| serde_json | 1.83 MB | 471 KB | 361 KB |
| ciborium (CBOR) | 1.41 MB (−23%) | 403 KB | 324 KB (−10%) |
| rmp-serde | 0.78 MB | 325 KB | 278 KB |
| postcard | 0.72 MB (−60%) | 302 KB | 253 KB (−30%) |

Estimate for herdr-threads: a message entry is about 150 B of envelope, about 250 B of payload metadata (ids, seat, flags, times) and the body (assume an average of 500 B), so ~0.9 KB in JSON. At the 10 msg/s ceiling that is ~9 KB/s, ~32 MB/h of steady-state traffic. A 100k-entry backfill is ~90 MB of raw JSON, or ~20 MB with zstd. Under compression, CBOR saves ~10% and postcard ~30%.

**Recommendation: JSON.** The codebase, its tests and its diagnostics are JSON throughout. The absolute volumes are small. postcard is not self-describing, so relaying unknown kinds would need the payload wrapped as opaque bytes anyway, and mixed-version debugging gets harder. The decision that matters is **opaque payload relay**: store each entry's payload exactly as authored and forward those bytes (`RawValue`). Then a relay running an older version cannot alter or drop fields of a kind it does not know, and any later content hash stays stable.

## 7. Keepalive, dead-link detection, SSH pitfalls

- `ServerAliveInterval` defaults to 0 (off). With 15 s and the default count of 3, ssh exits after ~45 s of silence. These probes travel inside the encrypted channel and cannot be spoofed, unlike `TCPKeepAlive` [E15]. The dialer passes them explicitly; it does not rely on the user's config.
- `BatchMode=yes`: a background daemon must never block on a password or host-key prompt [E16]. A prompt failure is reported as `peer status: auth needed` and retried with backoff.
- `ControlMaster=no`, `ControlPath=none`: users report sessions hanging forever on a stale master [E17, E18].
- Application `Ping` every 30 s when idle, and the link is dead after 90 s with no inbound frame. This catches a wedged remote daemon that SSH-level probes cannot see, because the SSH channel stays alive while the daemon behind the pump does not answer. The same 90 s rule applies on the dialed side.
- Remote-side reaping depends on EOF or SIGPIPE, not SIGHUP, because there is no pty [E19] (R-D).
- Shell noise on stdout is handled by the magic preamble [E21]. Shell noise on stderr is harmless.

## 8. Reconnect and backoff

Only the dialer reconnects (settled). The delay is `sleep = rand(0, min(cap, base·2^n))` with base 1 s and cap 300 s [E23]. Reset `n` after 60 s of healthy link. Permanent refusals (`envelope_unsupported`, `node_id_collision`, `self_link`) stop dialing until `peer add`/an upgrade/a `doctor` repair. Authentication failures are retried with the cap. A hint, such as a Herdr remote attach being observed again (research question 7), may cut the current backoff short.

## 9. Multiple links, relays and loops

- **Duplicate links to one node.** Simultaneous dials, two SSH aliases for one host, or auto-discovery from both sides can each produce a second link. Identify a peer by `(node_id, instance_uuid)` from the hello, not by SSH destination. Keep one active link: the link dialed by the lower node id. The other moves to standby (pings only), so it can take over without a new SSH handshake. Data correctness does not depend on this, because ingest is idempotent.
- **Relay loops.** Transitive relay over a full mesh with cycles is safe by construction. An entry is forwarded only after it is ingested in contiguous order, and an already-held `(origin, counter)` is dropped. The single-`rx`-source rule [E2] limits duplicate transfer to the failover window. NATS needs explicit loop-detection subjects [E11] because leafnode routing is subscription-based, not idempotent by ID. herdr-threads does not need that machinery, but should keep a counter of dropped duplicates per link as a diagnostic.
- **Source choice:** prefer the origin itself, if it is linked, then the peer with the highest advertised counter. Switch away from a source when it makes no progress for 30 s while it advertises more [E3].

## Limitations

- The autossh man page and NATS client ping defaults could not be retrieved; the autossh fetch failed and the NATS pages returned navigation only. The keepalive numbers rest on ssh_config(5), Syncthing and Mosh instead.
- There is no empirical measurement of SSH channel buffering or of SQLite commit latency under backfill. The window and batch sizes are engineering defaults to validate with a test.
- The size estimate assumes a 500 B average body. Real data from an existing store should be measured during migration (research question 8).

## Methodology and retrieval stop

Tools: the `search` CLI (preflight passed; the perplexity and serper providers failed on quota, so later queries were pinned to brave, exa and tavily); `curl` through r.jina.ai and direct fetches for the original documents; local repository reads. 16 query attempts, ~160 result entries considered, 17 external documents retrieved and used, plus 6 repository files. Failures: autossh(1) (empty), the first Server Fault fetch (re-fetched successfully), NATS ping/pong and the client protocol page (navigation only), and the Syncthing relay spec (stub). Ledgers are in the session scratchpad (`r3/ledger/`). Retrieval stopped when every facet listed in the topic was covered by a primary spec, man page or the repository, and the remaining gaps were judged low priority.

## Bibliography

- [S1] ssbc/epidemic-broadcast-trees README — https://github.com/ssbc/epidemic-broadcast-trees
- [S11] ssbc/ssb-ebt README — https://github.com/ssbc/ssb-ebt
- [S2] Syncthing, Block Exchange Protocol v1 — https://docs.syncthing.net/specs/bep-v1.html
- [S3] NATS, Leafnode protocol (2.11) — https://docs.nats.io/reference/2.11/protocols/leafnode
- [S4] Automerge `sync` module docs — https://automerge.org/automerge/automerge/sync/index.html
- [S5] gitprotocol-v2 — https://git-scm.com/docs/gitprotocol-v2
- [S20] gitprotocol-pack — https://git-scm.com/docs/gitprotocol-pack
- [S8] ssh_config(5), OpenBSD — https://man.openbsd.org/ssh_config
- [S10] rust_serialization_benchmark — https://github.com/djkoloski/rust_serialization_benchmark
- [S13] Server Fault 1002279, SSH multiplexing hangs — https://serverfault.com/questions/1002279
- [S18] Server Fault 1012465, stuck ControlMaster sockets — https://serverfault.com/questions/1012465
- [S14] Unix SE 39605, ssh command continues after ssh terminates — https://unix.stackexchange.com/questions/39605
- [S15] Server Fault 351731, PATH of an ssh remote command — https://serverfault.com/questions/351731
- [S16] AWS Architecture Blog, Exponential Backoff and Jitter — https://aws.amazon.com/blogs/architecture/exponential-backoff-and-jitter/
- [S17] Winstein & Balakrishnan, Mosh (USENIX ATC 2012) — https://mosh.org/mosh-paper.pdf
- [S22] mosh.org — https://mosh.org/
- [S21] rsync(1) — https://download.samba.org/pub/rsync/rsync.1
- [R1–R5] repository: `src/daemon/transport.rs`, `src/protocol/wire.rs`, `src/protocol/watch.rs`, `src/protocol/capabilities.rs`, `src/daemon/paths.rs`, `src/daemon/lifecycle.rs`, `docs/install.md`, `src/store/messages.rs`

Evidence IDs (E1–E32) refer to `evidence.jsonl` in the run ledger. Each E-id maps to one source above: E1–E4 → S1; E5–E9 → S2; E10–E11 → S3; E12 → S4; E13–E14 → S5; E15–E16 → S8; E17 → S18; E18 → S13; E19 → S14; E20 → S15; E21–E22 → S21; E23 → S16; E24 → S10; E25 → S17; E26 → S22; E27 → R1; E28 → R2; E29 → R4; E30 → R5; E31 → R3; E32 → S20.
