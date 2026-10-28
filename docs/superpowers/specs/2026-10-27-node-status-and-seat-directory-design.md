# Node status and seat directory (ht-xg88)

Status: design, awaiting review. Ships in 2.4.0 (functional, user-visible).
Folds in ht-b3k6 (sync observability). Builds on the cross-node sync design
(`docs/superpowers/runs/2026-10-14-cross-node-sync/`) and TRUST-POLICY.md
"Cross-node sync".

## 1. Problem

- A remote seat renders as `remote; labels unavailable` (`src/cli/peer_locations.rs:146`)
  and `active:false` with no bindings: `presence()` reads only local
  `occupant_bindings`, and labels are read by the CLI from the local Herdr
  snapshot only. From blacky we cannot tell whether `sQ0VnTDasAbY` (laptop)
  is online, idle or gone.
- Node health is opaque. On 2026-10-27 the Q0V link read `active lag=0` while
  the laptop had written nothing for ~3 days:
  - `lag` is `advertised - ours` clamped at 0 (`src/service/sync_admin.rs:218`):
    it says only how far *we* are behind, never how far the peer is behind on
    our log;
  - `LinkStatus.since`, `throttled`, `subscriptions` exist in the coordinator
    snapshot but `LinkRow` drops them; there is no last-frame-received time;
  - daemon.log lines carry no timestamp (`src/daemon/logs.rs:84-90`);
  - the peer's daemon version is not on the wire at all (`PeerHello` is
    versionless by design) and nothing about a peer survives its link except
    `sync_node_contact`.

## 2. What exists and is reused

| Need | Existing mechanism | Reused as |
| --- | --- | --- |
| Liveness | `Ping` after 30 s silence, link `Dead` at 90 s (`link.rs:46-48`); every decoded inbound frame resets it | unchanged |
| Last seen | `ContactBook` (direct on every frame, relayed via `Seen` every 10 s), flushed to `sync_node_contact`; `sync_contacts::reach`, `UNREACHABLE_AFTER_MS = 180 s` | **the** reachability rule for node status and for seat-status freshness |
| Their view of our log | peer's `have` already lists our `LogKey`; merged into `LinkEntry.peer_vector` (`coordinator.rs:1587`) | `tx` position, `peer_lag` |
| Our view of theirs | local vector / `sync_logs.contiguous_counter` | `rx` position |
| Caps | hello intersection in `PeerInfo.caps` (memory only) | persisted for display |
| Machine label | `node.genesis.host_label` → `node_directory.host_label` (already replicated) | `machine` |
| Workspace/tab/pane labels | `normalize_seat_labels` over Herdr `api snapshot` (`src/host/observation.rs:295`) | read by the home daemon, not only the CLI |
| Harness | `occupant_bindings.harness` | `harness` |
| Seat activity | `DeliveryPullState::record_contact` (`src/service/delivery_pull.rs:132`, last daemon call claiming the seat) | `last activity` |
| Ephemeral gossip precedent | `PeerFrame::Seen` (relative ages, transitive, in memory + throttled table) | template for the seat-status frame |

## 3. Decisions

**D1. Transport liveness is not node health; add application-level
progress, no new heartbeat frame.** Pong is answered inside the link I/O task
(`link.rs:738`), so a peer whose coordinator or store is wedged still pings
fine and today reads `active`. The Q0V root cause is unknown (laptop-side
debug pending); the design must not depend on it and must not show such a
peer as plain `linked`. Node status therefore has three layers:

1. *frame contact* (`seen`): any decoded frame, pings included (existing
   `ContactBook` / `reach`);
2. *application contact* (`app_seen`): a frame emitted by the peer's
   coordinator actor, i.e. anything that reaches `on_frame` (`Have`, `Want`,
   `Entries`, `Seen`, `NodeInfo`, `SeatStatus`, ...), never Ping/Pong/Credit;
3. *progress*: each direction's position advances while there is something
   to move (§4.4 stall rules).

A dedicated heartbeat frame is not needed for layer 2: `Seen` is already sent
by the coordinator actor every 10 s on each active link
(`coordinator.rs:132`, `:2126`), so a live coordinator produces app-level
traffic at least every 10 s and a wedged actor stops it. A wedged *store* with
a live actor still sends `Seen` from memory, which layer 3 catches: the peer's
acknowledged position of our log (from its `have`) stops advancing while we
are ahead. A new frame would duplicate `Seen` and still miss the store case.

**D2. Status survives the link.** Everything shown about a peer is persisted
(new `sync_peer_status` table) and rendered from the store, so a peer we
cannot reach shows `unreachable since <last contact>` with its last-known
version and positions, never vanishes. In-memory coordinator state is the
source while a link is up; the table is its throttled shadow.

**D3. Peer version on a new cap-gated frame, not in the hello.** `PeerHello`
must stay byte-identical toward 2.1.1 except for the cap strings (that is the
negotiation itself; `tests/sync/forward_compat.rs` pins its own cap set at
:43-45 and is unaffected). After `Accept`, a node sends one
`PeerFrame::NodeInfo { version }` to a peer whose caps include
`NODE_INFO_CAP = "node.info.v1"`. Version is display/diagnostic only (AGENTS.md
"Harness and Herdr versions"): nothing compares or gates on it.

**D4. Seat status is link gossip, not a journal fact.** The journal is
append-only forever and replicates to every peer regardless of caps (a new
fact kind is only *parked* on old peers, never withheld). Presence changes
every turn; journaling it would grow every node's store without bound for a
value that is meaningless an hour later. Instead each node publishes a full
snapshot of its own seats on a new frame `PeerFrame::SeatStatus`, gated by
`SEAT_STATUS_CAP = "seat.status.v1"`, relayed transitively like `Seen`.
Receivers persist the latest snapshot per origin (`remote_seat_status`) so
labels and last-known presence survive restarts and partitions.

**D5. Freshness is bounded by node status.** A remote seat's presence is
shown only when (a) its home node is reachable per `reach` and (b) the
snapshot holding it is younger than `UNREACHABLE_AFTER_MS` (origin re-publishes
every 60 s). Otherwise the row shows `unreachable since X` (a) or `stale` (b)
in place of presence; labels and harness stay, as last-known advisory text.
A stale `active` is never printed.

**D6. Advisory only.** Remote status rows are display data. They feed no
wake, route, deadline, availability, catch-up, attribution or seat
resolution; remote `availability` stays `Remote`. Seat resolution by name
never consults remote labels. (TRUST-POLICY C1, N5; §8.)

**D7. ht-b3k6 fold-in.** Timestamp every daemon.log line; show per-link
`since`, `last_rx`, `subs`, and `peer_lag` (their lag on our log). Out of
scope here: per-subscription listing (count only; the log keys stay in
`--machine` output), lag history/trends, and the live Q0V root cause (the
separate debug track).

## 4. Phase 1: node status

### 4.1 Wire

```rust
pub const NODE_INFO_CAP: &str = "node.info.v1";

// in PeerFrame
/// Sender's daemon version, display only. Sent once after Accept.
// cap: NODE_INFO_CAP
NodeInfo(NodeInfo),

pub struct NodeInfo {
    #[serde(default)]
    pub version: String, // CARGO_PKG_VERSION, <= 32 bytes, else dropped
}
```

- Sent only if the negotiated caps include `NODE_INFO_CAP`; a peer without it
  sees no new frame. Received from a peer that did not advertise it: ignored
  (same rule as fork frames, `coordinator.rs:1653`).
- Hello gains the cap string(s) only. `scripts/freeze-peer-wire 2.1.1 --check`
  stays green; a new fixture is not frozen until 2.4.0 ships.

### 4.2 Coordinator state (memory)

`LinkEntry` gains:

- `last_rx: UtcMillis`: set in `on_frame` and in `poll_contacts` when
  `inbound_frames` grows (pings count);
- `app_rx: UtcMillis`: set in `on_frame` only (coordinator-originated frames);
- `tx_progress_at: UtcMillis`: last time `peer_has_ours` advanced, or the
  peer was caught up (`peer_has_ours >= our head`); reset on activation;
- `rx_progress_at: UtcMillis`: last time our contiguous counter of the
  peer's active log advanced, or we were caught up with its advertised head;
- `peer_version: Option<String>`.

`LinkStatus` exposes `since`, `last_rx`, `app_rx`, `subscriptions` (already
present), `peer_version`, `caps`, `peer_has_ours: Option<u64>` =
`peer_vector[own active LogKey].counter`, `tx_progress_at`, `rx_progress_at`.

### 4.3 Storage (migration `0006_peer_status.sql`)

```sql
CREATE TABLE sync_peer_status (
    node_uuid TEXT PRIMARY KEY,
    version TEXT,                 -- last node_info of this boot
    boot TEXT NOT NULL,           -- the peer boot the values belong to
    tx_acked INTEGER, tx_head INTEGER,  -- their acknowledged view of our active log
    rx_have INTEGER, rx_head INTEGER,   -- our view of their active log
    app_seen_at INTEGER,          -- last coordinator-originated frame
    updated_at INTEGER NOT NULL
) STRICT;
```

Only what a peer without a live link is shown with: whether it is linked,
and since when, comes from the live coordinator alone, so no stored row can
claim a link a previous daemon run had. Written by the coordinator through
`SyncStorePort::record_peer_status` (`src/sync/coordinator/peer_status.rs`,
store `src/store/sync_peer_status.rs`):

- a node not yet written, or a new version or boot: at the next tick;
- changed positions: at most every 10 s; app contact only once it advanced
  30 s (the `ContactBook` advance rule), so a healthy link costs a write
  about every 30 s, not every `seen`;
- a failed write backs off 10 s and is logged once per run of failures (and
  once on recovery); a write still in flight after 60 s is logged once;
- at shutdown, while the links still exist, the write in flight is joined
  and every row that differs from the store is written, each step bounded by
  `CLOSE_WRITE_TIMEOUT`.

Merge rule: a row of a new boot replaces every value (the old boot's
positions describe another run); within a boot a missing version or
position keeps the stored one and app contact only moves forward. Counters
past `i64::MAX` (a peer may advertise any `u64`) are stored clamped, so
display data never fails a write. Last contact stays in `sync_node_contact`.
One row per node, bounded by `node_directory`. The progress clocks are live
state only.

### 4.4 Node state (one rule)

`store::sync_status::node_state`, from the live link the coordinator
reports and `reach`:

1. A node with a live active link is `stalled` with the parts that hold, in
   the order `app`, `tx`, `rx`, else `linked`. The live link wins over the
   throttled stored contact, so a just-linked node is never shown
   unreachable.
   - `app`: `now - app_rx > APP_SILENT_AFTER` (30 s = three missed `seen`);
     transport alive, coordinator silent. The threshold must exceed the 10 s
     `seen` period, or a healthy link reads stalled between two `seen`s;
   - `tx`: our active log's head is ahead of what the peer acknowledges and
     that acknowledgement has not grown for `STALL_AFTER` (120 s);
   - `rx`: the peer advertises its active log ahead of what we hold and we
     have ingested none of it for `STALL_AFTER`.
2. Without one: `unreachable since <last_contact>` / `unreachable never` by
   `reach`, else `reachable` (fresh relayed contact).

If the coordinator does not answer `peer list` within 2 s (its own deadline,
inside the call budget), no node is live: the rows come from the store.

Progress is clocked on the 1 s housekeeping tick (`track_progress`): a
direction that is caught up, whose held counter grew, or whose active log
changed (a new incarnation) made progress at that tick. A direction is
stalled only while it still holds the counter of the same log the last tick
recorded, so a catch-up followed by new entries between two ticks never
reads as a stall. Stall is diagnostic only: it changes no link, rank or dial
decision (a follow-up may let a stalled active link yield to a standby; not
here).

`node_info` is kept once per link and only when it looks like a version
(1..=32 bytes of `[0-9A-Za-z.+-]`).

Clone evidence: when `sync_clone_evidence` has an open (uncleared) row whose
`subject` is the peer, its row adds ` clone_suspected` (16 bytes); the state
is computed as above, never hidden.

### 4.5 CLI

`peer list` keeps its dial-target rows (`peer <target_id> ...`) and adds one
row per known node (`node_directory`, self excluded) after them:

```
peers
peer 1 linked explicit node=Q0V reach=reachable attempts=0: laptop
node Q0V linked seen=4s app=6s v=2.4.0 rx=567/567 tx=1180/1234 since=06:12Z
node Q0V stalled app,tx seen=4s app=6s v=2.4.0 rx=567/567 tx=1180/1234 since=06:12Z clone_suspected
node K7m unreachable since 09:11Z v=2.4.0 rx=567/567 tx=1180/1234
node Pq7 unreachable never v=- rx=- tx=-
```

- `seen` is frame contact, `app` application contact (§D1).
- `rx=A/B`: we hold A of their head B as last advertised; `tx=A/B`: they
  acknowledged A of our head B; last-known values once the link is gone.
  Unknown parts print `-`. The version is peer text, escaped to one token.
- Sizes (the stalled and link rows asserted in `tests/service/sync_admin.rs`):
  the linked row is 75 bytes in this example, the stalled one 99,
  unreachable 65, never-heard 40; they grow only with the counters' digits
  and the version text (at most 32 bytes).
- JSON: `SyncReply::Peers.nodes: Vec<NodeRow>` (`#[serde(default)]`, omitted
  when empty); an older daemon's reply has none and prints no node rows.
- `empty` is printed only when there is neither a dial target nor a known
  node.

`sync status` link rows add `since`, `last_rx`, `app_rx`, `subs`, `peer_lag`
and, on a stalled link, `stalled=`:

```
link Q0V active dialer rx_logs=3 unsolicited=0 since=09:11Z last_rx=2s app_rx=6s subs=3 peer_lag=54 stalled=tx
```

+58 bytes per link row in this example (110 total; `stalled=` only when set).
An older daemon's reply lacks the fields and prints the row as before.
`LinkRow` gains `#[serde(default)]` fields (`src/protocol/sync_admin.rs` is a
wire file). Agent-facing only on explicit `peer list` / `sync status` calls;
no delivery, hook or check-in text changes.

### 4.6 Daemon log timestamps

The daemon child's `daemon.log` sink (`StampedLog` in `src/daemon/mod.rs`)
prefixes each line start with `2026-10-27T09:12:03.481Z ` (25 bytes, UTC,
ms, wall clock): the output reader's bytes (everything the process writes to
stdout/stderr, panics and child output included) and the daemon's own
writes (owner errors, drain notes, the final diagnostic replay) alike. The
line state is shared, so a write continuing a line another began is not
stamped again. CLI stderr is untouched. daemon.log is operator-facing, not
model-visible. The 1 MiB rotation is unchanged, so the stamp costs about a
fifth of the retained history; accepted for now.

### 4.7 Left out of ht-b3k6

- Per-subscription listing: `subs` is a count; the log keys are not printed.
- Lag history or trends: only the current positions and the stall verdict.
- The live Q0V root cause (separate debug track).

## 5. Phase 2: seat status propagation

### 5.1 What a home node publishes

`service::seat_publisher` (one task beside discovery, ended by the stop
token) builds, for each local, published, nonretired seat
(`store::local_seat_status::local_seats`, at most 512, newest first, so past
the cap the oldest are left out):

| Field | Source | Bound |
| --- | --- | --- |
| `seat` | id | 64 bytes |
| `presence` | `busy` = active (`bindings::seat_active`) and effective `ui_state` in {`active_turn`, `approval_or_question`, `human_input`}; `idle` = active otherwise; `offline` = not active. The local `active/inactive` column is unchanged. | 16 bytes |
| `harness` | the open top-level binding's harness, else the latest binding's; opaque text (no harness branch in core) | 16 bytes; a longer id is dropped, never cut |
| `activity_age_ms` | now − max(`DeliveryPullState::last_contact`, latest binding registration or disconnect) | u64 |
| `workspace`, `tab`, `pane` | `SeatHostLabels` of the seat's host and target whose terminal is the seat's structural terminal (and whose incarnation, when the host reports one, is the seat's); the label, else the Herdr id | 64 bytes each, cut on a char boundary |

`machine` is not sent: it is `node_directory.host_label`, already replicated.

Cadence: every 5 s it rebuilds from the store and publishes when anything
but the ages changed (a publish the coordinator's queue refused is retried
on the next check); every 60 s it re-reads the labels (one `api snapshot`
per attached host through the new `HostPort::seat_labels`, default none;
`NativeCli` answers with `participant_labels`; one 3 s budget for all hosts;
a host that does not answer keeps its last labels) and publishes again even
unchanged, which keeps receivers fresh. Read failures are logged once per
run. `busy` uses the wake path's own rule (`effective::turn_in_progress`). It runs only while some active link
negotiated `SEAT_STATUS_CAP` (`SyncHandle::seat_status_wanted`); the first
listener gets a fresh snapshot at once. No change to the reconcile or
observation lane. Test-support daemons take `HT_TEST_SEAT_CHECK_MS` and
`HT_TEST_SEAT_REFRESH_MS`.

### 5.2 Wire

```rust
pub const SEAT_STATUS_CAP: &str = "seat.status.v1";

// in PeerFrame
/// One origin node's whole advisory seat snapshot, relayed as is.
// cap: SEAT_STATUS_CAP
SeatStatus(SeatStatusFrame),

pub struct SeatStatusFrame {   // every field #[serde(default)]
    pub origin: NodeUuid,
    pub boot: Uuid,      // origin daemon boot
    pub rev: u64,        // per boot, +1 per changed snapshot
    pub age_ms: u64,     // since the origin built it, accumulated by relays
    pub version: String, // origin daemon version (relayed nodes learn it here)
    pub seats: Vec<SeatStatusEntry>, // <= 512
}
pub struct SeatStatusEntry {   // every field #[serde(default)]
    pub seat: String,
    pub presence: String, // unknown values render as `-`
    pub harness: Option<String>,
    pub activity_age_ms: Option<u64>,
    pub workspace: Option<String>,
    pub tab: Option<String>,
    pub pane: Option<String>,
}
```

Coordinator module `src/sync/coordinator/seat_status.rs`:

- **Publish:** `SyncHandle::publish_seat_status` hands the entries to the
  actor (never waits; `false` when its queue is full). Every publish,
  refreshes included, is the next `rev`. Each publish goes to every ready
  link (active, ranked, open, announced, with the cap); a newly ready link
  first gets the own snapshot and every held one (except its own node's).
- **Frame bound:** a frame over the link's negotiated `max_frame` (as small
  as 4 KiB) is cut to the share of seats that fits, from the end.
- **Ordering:** one held snapshot per origin. Same boot: only a higher `rev`
  replaces it, so a copy coming back round a relay loop always dies. Another
  boot: replaces it only when built more than 1 s later (`built_at` =
  receipt time − `age_ms`). Dropped snapshots are never relayed.
- **Clone suspicion:** per origin and boot, the first and last build seen
  (build times, not arrival, so a stale relayed copy moves nothing); boots
  are forgotten 180 s after their last build. Another boot counts as live
  only when it was built after the held boot's first build: two boots alive
  at once, never a plain restart (whose old builds all precede the new). The
  stored row then carries `other_boot_at` and the seats render
  `clone_suspected`.
- **Relay:** an accepted snapshot goes to every other ready link except the
  sender and the origin's own node, with `age_ms` recomputed from `built_at`.
- **Guards:** a frame from a link without the cap, with this node as origin,
  or for an origin this node does not know (no log of it held and no contact,
  direct or relayed by `seen`) is dropped; entries are bounded on receipt as
  on publish (an empty presence becomes `-`).
- **Writes:** always the whole snapshot, the newest per origin, one batch in
  flight through `SyncStorePort::record_remote_seat_status`; a failure keeps
  the updates pending unless newer ones arrived, backs off 10 s and is logged
  once per run (and on recovery); a write in flight 60 s is logged once;
  shutdown flushes what is pending, bounded.

### 5.3 Storage (migration `0007_remote_seat_status.sql`)

```sql
CREATE TABLE remote_seat_origin (
    origin TEXT PRIMARY KEY, boot TEXT NOT NULL, rev INTEGER NOT NULL,
    built_at INTEGER NOT NULL,   -- local ms = received_at - age_ms
    version TEXT, other_boot_at INTEGER, updated_at INTEGER NOT NULL
) STRICT;
CREATE TABLE remote_seat_status (
    seat_id TEXT PRIMARY KEY REFERENCES seats(id),
    origin TEXT NOT NULL REFERENCES remote_seat_origin(origin),
    presence TEXT NOT NULL, harness TEXT,
    activity_at INTEGER,         -- local ms
    workspace TEXT, tab TEXT, pane TEXT
) STRICT;
```

A snapshot replaces its origin's seat rows in one transaction; a seat is kept
only when this node knows it as a remote seat (`state='remote'`) whose
`home_node` is the origin (N4 guard, in the store), so a seat learned after a
snapshot is taken by the next refresh. Rows equal to the stored ones are not
rewritten (a refresh then moves only the origin row). `get` reads only seats
still `remote`. `remote_seat_origin.version` gives phase 3 the version of
relayed-only nodes.

### 5.4 Freshness (one store function)

`store::remote_seat_status::presence_of(row, origin_reach, origin_live, now)`:

```
  home node unreachable and no live link   -> Unreachable(since)
  no row                                   -> Unknown   (no cap, or nothing yet)
  another boot published within 180 s      -> CloneSuspected
  built more than 180 s ago                -> Stale(built_at)
  else                                     -> Fresh(presence, activity_at)
```

A live link to the home node wins over its stored reach (as in §4.4). A
`stalled` home node still shows fresh presence while its snapshots arrive:
the snapshot is itself coordinator traffic. If its coordinator is silent,
snapshots stop and the seat goes `stale` within the bound. Local seats
render from local state as today.

## 6. Phase 3: directory and surfaces

One store function builds every entry: `store::seat_directory::directory`
(`SeatDirectory`: node tag, machine, state, since, harness, activity,
snapshot build, labels). A local seat's comes from its bindings and
observation (`local_seat_status`: presence by the same rule as the
publisher, the latest binding change as activity) and carries no labels:
labels of local seats stay the CLI's own host read (`--human`). A remote
seat's comes from its home node's stored snapshot through
`remote_seat_status::presence_of`: `busy`/`idle`/`offline`, else
`unreachable` (since the last contact), `stale` (since the build),
`clone_suspected` or `-`; harness, activity and labels stay last known.

### 6.1 `seat list`

New flags: `--node TAG`, `--on MACHINE` (host label or tag; `--machine` is
the global machine-output flag), `--harness ID`, `--label TEXT`
(case-insensitive substring of a seat's workspace/tab/pane: a remote seat's
from its home node's snapshot, a local seat's from the daemon's own host read,
made only for `--label`, ht-hldp), `--local`.
Filters are AND, case-insensitive, evaluated in the daemon while it pages
(rows examined count toward the page's work bound; locality is the
`is_local` column, so a retired remote seat is never `--local`), kept in the
cursor digest and in the `next:` command as `--flag=VALUE`. The CLI sends
them only to a daemon advertising `seats.directory_v1` (an older one refuses
the unknown field and never answers), else: `seat list filters unavailable;
daemon requires seats.directory_v1`. A pane lookup (`SeatsQuery.target`, the
hook's and caller resolution's path) builds no directory entry.

Compact form (new; it was the generic encoder):

```
seats
snSGbl096JZZ blk idle claude last=4s
sQ0VnTDasAbY Q0V idle claude last=12m: office / main / claude
sQ0Vx1aB2cD3 Q0V unreachable since 08:42Z claude last=12m: office / main / claude
sQ0Vtab0nTab Q0V stale 09:07Z claude last=12m: office / a: b\nc / claude
next: herdr-threads seat list --cursor ...
```

- Row grammar: `SEAT TAG STATE HARNESS last=AGE[ unresolved| retired][: WS /
  TAB / PANE]`; `STATE` as above with `unreachable since T` / `unreachable
  never` / `stale T`. Peer labels only after `": "`, each escaped with
  `one_line`. Peer presence is one of `busy`/`idle`/`offline` (anything else
  is stored as `-`) and a peer harness id `[A-Za-z0-9._-]{1,16}`, so neither
  can fake row punctuation. `--json` keeps every field.
- Sizes (asserted): a local row 36 bytes; the fresh remote row 61, the
  unreachable one 81 for 6-byte labels; labels add at most 3 x 64 + 6.
  Paging stays 20 rows / 16 KiB. An older daemon's row has no entry:
  `SEAT CONTINUITY active|inactive`.
- Human table: a `NODE` column; a remote row shows its state (`unreachable
  since T`, `stale since T`) under `PRESENCE` and its labels under
  `SPACE TAB PANE`. The advisory preface stays.

### 6.2 `thread participants` / `thread show`

- A remote row ends with ` STATE HARNESS last=AGE` when its home node's word
  is current (`b joined remote idle claude last=4s`, +20 bytes asserted,
  17-25 typical), after the existing flags, ` stale` or
  ` clone_suspected` alone, and nothing when the home node is unreachable or
  never published: the existing one-per-node `node TAG unreachable since T`
  line says it.
- The CLI location hint for a remote seat becomes `WS / TAB / PANE
  [advisory]`, or `[advisory, last known]` once the presence is not current
  (+12 bytes), from `ParticipantLocation.remote`; `remote; labels unavailable`
  stays for a seat whose node published none and with an older daemon.
- Local rows and every delivery, hook and check-in are unchanged.

### 6.3 `seat inspect`

A remote seat gets one line, `directory: TAG STATE HARNESS last=AGE seen=AGE[:
WS / TAB / PANE]` (67 bytes for the asserted example), in place of the raw
`directory` field; `seen` is the snapshot's age.

### 6.4 Protocol

`SeatSummary.directory` and `Participant.directory` (`Option<Box<_>>`),
`SeatInspection.directory` and `ParticipantLocation.remote`, and
`SeatsQuery.filter` (`SeatFilter`), all `#[serde(default,
skip_serializing_if)]`. Daemon capability `seats.directory_v1`. Harness ids
are opaque text from the binding; the core does not branch on them.

### 6.5 Skill

`integrations/skill/SKILL.md` gains one paragraph on the `seat list` row and
filters (about 480 bytes, once per session), including that remote presence
and labels are advisory and never an address.

### 6.6 Accepted limits of phase 3

- Remote rows read `presence_of` with no live-link input: a link that just
  came up shows its node unreachable until the contact flush (at most ~10 s).
- `--label` reads the local hosts' labels within at most 1 s of the call's
  budget; a host that does not answer in time leaves its seats matching no
  label (ht-hldp; previously `--label` matched remote seats only).

## 7. Timeouts and constants

| Constant | Value | Origin |
| --- | --- | --- |
| `PING_AFTER` / `DEAD_AFTER` | 30 s / 90 s | existing |
| `UNREACHABLE_AFTER_MS` | 180 s | existing; also the snapshot staleness bound |
| `APP_SILENT_AFTER` | 30 s | new; three missed `Seen` (10 s) |
| `STALL_AFTER` | 120 s | new; a direction with work pending and no progress |
| `SEAT_STATUS_REFRESH` | 60 s | new; < 180 s / 2, so one missed refresh is never stale |
| `SEAT_STATUS_COALESCE` | 5 s | new |
| `PEER_STATUS_FLUSH` | 10 s | new, mirrors the `ContactBook` cadence |
| `MAX_SEAT_STATUS_SEATS` / label | 512 / 64 bytes | new |

## 8. Trust-policy impact

There are no new provenance values: these rows attribute no action and are
never written to bindings, receipts or events (A3 unchanged).
TRUST-POLICY.md gains, in the same commit as phase 2:

- **Header amendment:** "Amended 2026-10-xx for node status and seat directory
  (ht-xg88): advisory peer status and remote seat status."
- **N-rule (Cross-node sync), Advisory seat status:**
  - Only the home node describes its own seats. Receivers keep an entry only
    when the seat's `home_node` is the origin, which is a consistency guard
    (N4 author rule) and not caller verification.
  - Status is relayed as is and never journaled.
  - Remote presence and labels never feed wake, routing, deadlines,
    availability, catch-up, seat resolution, attribution or retirement (C1,
    N5). Remote `availability` stays `remote`.
- **Accepted limit (Cross-node sync), Remote presence is advisory and lags:**
  - A remote seat's presence is at most `UNREACHABLE_AFTER_MS` old when shown,
    can be wrong within that window, and is replaced by `unreachable since`
    or `stale` past it.
  - Labels are cooperative peer text: shown last-known, never identity.
  - The peer version is a self-report: diagnostics only.
  - `stalled` and `clone_suspected` are diagnostics: they change no link,
    dial, seat or binding decision.
- A2: decisions are still made in the daemon against the canonical view. The
  directory reads no client-local file, and the client CLI performs no
  filtering.

## 9. Migration and compatibility

- **Old peers (2.1.1 – 2.3.x)** do not advertise either cap, so they get no
  `NodeInfo` and no `SeatStatus` frames. Their traffic is byte-identical except
  for the two new cap strings in our hello, which they ignore.
  - In `peer list`, they show with `v=-` and positions.
  - Their seats show `-` presence plus `location SEAT: remote; labels
    unavailable` (the old text is kept for exactly this case).
- **A newer peer sending a frame we do not know** decodes as
  `PeerFrame::Unknown` (unchanged).
- **Store:** migrations 0006 and 0007 are additive, but every migration
  bumps the store's schema version and an older binary refuses a newer store
  ("store is newer than this binary"), so a rollback below the release that
  carries 0006 needs a newer binary back (as for every earlier delta).
- **Wire-compat:** two new variants carry `// cap:`, and the baseline grows
  only by the new types. `scripts/check-wire-compat` and
  `scripts/freeze-peer-wire 2.1.1 --check` must pass. A 2.4.0 fixture is
  frozen at release.
- **Model-visible deltas:** these are all opt-in reads (`seat list`,
  `participants`, `seat inspect`, `peer list`, `sync status`). No delivery,
  hook or check-in text changes.

## 10. Test plan

**Unit and in-process tests (coordinator `Rig`):**
- The `NodeInfo` frame is sent only when the cap is negotiated. It is ignored
  from a peer that did not advertise the cap. The version text is bounded.
- `peer_has_ours` tracks the peer's `have` for our key; `last_rx`
  advances on pings, `app_rx` does not.
- Stall rules (`Rig`, mock clock): tx stall after `STALL_AFTER` with our head
  ahead and no `have` advance; no stall when caught up or when the peer has
  nothing to take; rx stall when the peer advertises ahead and we ingest
  nothing; `stalled app` after `APP_SILENT_AFTER` of pings-only traffic;
  each clears on the next progress.
- Cross-boot ordering: two boots of one origin alternating snapshots →
  the shown snapshot follows `built_at`, never flips on boot id, and seats
  render `clone_suspected`; one boot going quiet for the bound → back to
  normal.
- `SeatStatus` relay:
  - a newer `(boot, rev)` replaces the old one;
  - an older one is dropped;
  - ages accumulate across relays;
  - self-origin frames are dropped;
  - entries whose `home_node` is not the origin are dropped;
  - over-bound frames are truncated.

**Store tests:**
- The `remote_presence` rule covers four cases: fresh, stale, unreachable
  (via an aged contact) and unknown.
- Replacing a snapshot deletes seats that are absent from it.

**Wire tests:**
- `forward_compat` stays green.
- A 2.1.1 shadow decoder decodes `NodeInfo` and `SeatStatus` as `Unknown`.
- `check-wire-compat`.

**Output tests:**
- Compact and human golden rows for `seat list`, `participants`,
  `seat inspect`, `peer list` and `sync status`, with byte sizes asserted.
- Peer label escaping, including a label that contains `: ` or a newline.

**Two-node e2e (`tests/multinode/seat_directory.rs`, `support::cluster(.., 2, &[(0,1)], &[0,1])`):**
1. Converge. Node 1 has a hosted seat with a connected binding.
2. On node 0, `peer list` shows node 1 as `linked v=<ver>` with `tx`/`rx`.
   `seat list --node <tag1>` shows the seat as `idle`/`busy` with its
   harness and labels.
3. Cut the link (`partition(0,1)`, or `hold_frames_from(1)` with `gated`).
   Then use `age_contact(0, node1, 200 s)` so the test does not wait 180 s.
4. On node 0:
   - `peer list` shows `unreachable since <T>` with the last-known version
     and positions;
   - `seat list` shows the seat as `unreachable since <T>`;
   - no row says `busy` or `idle`;
   - `thread participants` shows `node <tag> unreachable since`.
5. Heal. Presence returns within one refresh.
6. Restart node 0 while it is still partitioned. The status is served from
   the tables, not vanished.

**Stale path:** hold only `SeatStatus` (a failpoint suppresses publishing)
while pings continue. After the bound, the seat shows `stale`.

**Wedged-peer e2e (`tests/multinode/node_stall.rs`, gated cluster, the case
behind D1):** two linked nodes; node 0 authors messages so it has entries to
offer. The relay then drops node 1's coordinator frames (`Have`, `Want`,
`Entries`, `Seen`, `NodeInfo`, `SeatStatus`) while passing Ping/Pong, so the
transport stays alive (a frame-class filter added to the existing
`hold_frames_from` relay; the link must not die at 90 s). On node 0, with
`STALL_AFTER` / `APP_SILENT_AFTER` shortened through the test-support
config:
- `peer list` shows `stalled` (`app` and/or `tx`) with `seen` fresh and
  `app` aging, and never plain `linked`, sampled every second for the
  whole hold;
- `sync status` link row shows `stalled=`;
- node 1's seats go `stale`, never `busy`/`idle` past the bound.
Release the filter: the row returns to `linked` within one `Seen` period
plus one `have`. A second variant keeps `Seen` flowing and holds only
`Have`/`Entries` (wedged store) and expects `stalled tx`.

All tests run under `nice -n 19` with cluster fixtures and private state
directories only. Every test spawns through `test_support::spawn`.

## 11. Phases and commits

1. **Node status:** `NodeInfo` frame and cap, `LinkEntry` fields, app-level contact and stall rules,
   `sync_peer_status`, `peer list`/`sync status` rows, log timestamps.
   This closes ht-b3k6.
2. **Seat status:** publisher (labels read, presence), the `SeatStatus` frame,
   relay, `remote_seat_status`, the freshness rule, and the TRUST-POLICY
   amendment.
3. **Directory:** `seat list` filters and compact form, participants,
   locations, inspect, the `seats.directory_v1` daemon cap, the skill line
   (once per session, in SKILL.md), and the multinode e2e.

## 12. Resolved in review (2026-10-27)

1. Presence vocabulary: `busy|idle|offline`; the local `active/inactive`
   column stays.
2. Labels: a separate 60 s `api snapshot` on the home daemon; the observation
   lane is untouched.
3. Pane cwd: not published.
4. Liveness: application-level progress (`app_seen`, stall rules) added;
   no heartbeat frame, justified against the wedged-coordinator/store case
   (D1), with a reproducing e2e (§10).
5. Clone evidence: cross-boot ordering by `built_at`, `clone_suspected` on
   peer rows and seats.
6. Release: phases 1-3 ship together as 2.4.0 unless the user splits phase 1
   off as a patch.
