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

**D1. No new heartbeat for liveness.** Ping/Pong plus any frame already
guarantee inbound traffic at least every 30 s on a healthy link, `Seen` relays
contact every 10 s, and `reach` already turns that into reachable/unreachable.
A heartbeat frame would duplicate Ping and change 2.1.1 traffic for nothing.
The Q0V incident was not a liveness failure (pings flowed); it was missing
*position* and *time* observability, which D3 and D7 fix.

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

`LinkEntry` gains `last_rx: UtcMillis` (set in `on_frame` and in
`poll_contacts` when `inbound_frames` grows, i.e. pings count) and
`peer_version: Option<String>`. `LinkStatus` exposes `since`, `last_rx`,
`subscriptions` (already present), `peer_version`, `caps`, and
`peer_has_ours: Option<u64>` = `peer_vector[own active LogKey].counter`.

### 4.3 Storage (migration `0006_peer_status.sql`)

```sql
CREATE TABLE sync_peer_status (
    node_uuid TEXT PRIMARY KEY,
    version TEXT,                 -- last NodeInfo, NULL if never sent
    caps TEXT NOT NULL,           -- negotiated, space-separated
    boot TEXT NOT NULL,           -- last hello boot
    linked_since INTEGER,         -- NULL while no active link
    last_linked_at INTEGER NOT NULL,
    peer_has_ours INTEGER,        -- their acknowledged counter of our active log
    updated_at INTEGER NOT NULL
) STRICT;
```

Written by the coordinator through `SyncStorePort`: on activation, on link
end (`linked_since = NULL`), and when `peer_has_ours` advanced, at most every
10 s per node (the `ContactBook` cadence). Last contact stays in
`sync_node_contact`; `rx` positions come from `sync_logs`. One row per node,
bounded by `node_directory`.

### 4.4 Node state (one rule)

Computed in `store::sync_status` from `reach` plus `linked_since`:

- `linked`: an active direct link exists (`linked_since` set);
- `reachable`: no direct link, contact fresh (relayed);
- `unreachable since <last_contact>` or `unreachable never`.

### 4.5 CLI

`peer list` compact rows (one per known node, self excluded):

```
peers
peer Q0V linked seen=4s v=2.4.0 rx=567/567 tx=1180/1234 since=09:12
peer K7m unreachable since 09:40 v=2.3.2 rx=88/88 tx=902/1234
```

- `rx=A/B`: we hold A of their head B as last advertised; `tx=A/B`: they
  acknowledged A of our head B. Unknown parts print `-`.
- Linked row: 67 bytes for this example (fixed part ~43 bytes plus tag,
  version and counters); unreachable row ~62 bytes.

`sync status` link rows add `since`, `last_rx`, `subs`, `peer_lag`:

```
link Q0V active dialer rx_logs=3 unsolicited=0 since=09:12 last_rx=2s subs=3 peer_lag=54
```

+34 bytes per link row (88 total in the example). `LinkRow` and `PeerRow`
gain `#[serde(default)]` fields (`src/protocol/sync_admin.rs` is a wire file).

### 4.6 Daemon log timestamps

The stderr sink (`logs.rs:84-90`) prefixes `2026-10-27T09:12:03.481Z ` (25
bytes, UTC, ms). daemon.log is operator-facing, not model-visible.

## 5. Phase 2: seat status propagation

### 5.1 What a home node publishes

For each local, nonretired, published seat:

| Field | Source | Bound |
| --- | --- | --- |
| `seat` | id | |
| `presence` | `active` = connected binding and effective `ui_state` in {`active_turn`, `approval_or_question`, `human_input`}; `idle` = connected binding otherwise; `inactive` = no connected binding | enum |
| `harness` | harness of the open top-level binding, else latest | 16 bytes, adapter id |
| `activity_age_ms` | now − max(`record_contact`, latest binding transition) | u64 |
| `workspace`, `tab`, `pane` | `SeatHostLabels` for the seat's target, matched on terminal and incarnation as `peer_locations` does | 64 bytes each, one line, else truncated |

`machine` is not sent: it is `node_directory.host_label`, already replicated.

The home daemon reads labels with one `api snapshot` (the same call and
parser the CLI uses) on the publish tick, only while at least one linked peer
negotiated `SEAT_STATUS_CAP`. No change to the reconcile/observation lane.

### 5.2 Wire

```rust
pub const SEAT_STATUS_CAP: &str = "seat.status.v1";

// in PeerFrame
/// One origin node's full advisory seat snapshot, relayed as is.
// cap: SEAT_STATUS_CAP
SeatStatus(SeatStatusFrame),

pub struct SeatStatusFrame {
    #[serde(default)] pub origin: NodeUuid,
    #[serde(default)] pub boot: Uuid,      // origin daemon boot
    #[serde(default)] pub rev: u64,        // per boot, +1 per change
    #[serde(default)] pub age_ms: u64,     // since origin built it, accumulated by relays
    #[serde(default)] pub version: String, // origin daemon version (relayed nodes learn it here)
    #[serde(default)] pub seats: Vec<SeatStatusEntry>, // <= 512
}
pub struct SeatStatusEntry {
    #[serde(default)] pub seat: SeatId,
    #[serde(default)] pub presence: String, // unknown values render as `-`
    #[serde(default)] pub harness: Option<String>,
    #[serde(default)] pub activity_age_ms: Option<u64>,
    #[serde(default)] pub workspace: Option<String>,
    #[serde(default)] pub tab: Option<String>,
    #[serde(default)] pub pane: Option<String>,
}
```

- **Publish:** the origin rebuilds its snapshot every 60 s and on any presence or
  label change (coalesced 5 s). It sends it to every active cap peer: when
  something changed, on each link activation, and at least every 60 s so that
  freshness (D5) keeps advancing. With ~20 seats this is about 2 KiB per link
  per minute.
- **Relay:** a receiver keeps the newest `(boot, rev, built_at)` per origin.
  It forwards a newer snapshot to its other cap peers with
  `age_ms += local hold time`, the same idea as relative ages in `Seen`.
  A snapshot with the same `(boot, rev)` is only re-forwarded if its
  `built_at` is newer.
- **Consistency guards** (not adversarial verification):
  - Drop entries whose seat has no local row, or whose local row has
    `home_node != origin`. The next refresh re-sends them.
  - Drop a frame whose `origin` is this node.
  - If the frame size exceeds `MAX_FRAME`, the snapshot is truncated in seat
    order and marked by `seats.len()` (bounded by 512).

### 5.3 Storage (migration `0007_remote_seat_status.sql`)

```sql
CREATE TABLE remote_seat_status (
    seat_id TEXT PRIMARY KEY REFERENCES seats(id),
    origin TEXT NOT NULL,        -- = seats.home_node
    boot TEXT NOT NULL,
    rev INTEGER NOT NULL,
    built_at INTEGER NOT NULL,   -- local ms = received_at - age_ms
    presence TEXT NOT NULL,
    harness TEXT,
    activity_at INTEGER,         -- local ms
    workspace TEXT, tab TEXT, pane TEXT
) STRICT;
CREATE TABLE remote_seat_origin (
    origin TEXT PRIMARY KEY, boot TEXT NOT NULL, rev INTEGER NOT NULL,
    built_at INTEGER NOT NULL, version TEXT
) STRICT;
```

A newer snapshot from an origin replaces that origin's rows in one
transaction; seats absent from it are deleted (they retired or went
unpublished). Writes happen only when the content changed. A refresh with an
unchanged `rev` updates only `remote_seat_origin.built_at`.
`remote_seat_origin.version` also gives the version of relayed-only nodes in
`peer list`.

### 5.4 Freshness (rendering rule, in one store function)

```
remote_presence(seat) =
  match reach(origin):
    Unreachable(since) -> Unreachable(since)
    Reachable if now - origin.built_at > UNREACHABLE_AFTER_MS -> Stale(built_at)
    Reachable -> Fresh(presence, activity_at)
  no row -> Unknown   (origin lacks the cap or has not published yet)
```

Local seats render from local state as today.

## 6. Phase 3: directory and surfaces

### 6.1 `seat list`

New flags: `--node TAG`, `--machine NAME` (host label or tag), `--harness ID`,
`--label TEXT` (case-insensitive substring of workspace/tab/pane), `--local`.
Filters are AND, evaluated in the daemon (paging stays correct). Remote seats
are already in `seats()`; they now carry `directory: Option<SeatDirectory>`.

New compact form (today it falls back to the generic encoder):

```
seats
snSGbl096JZZ blk active claude last=4s: herdr-threads / dev / claude
sQ0VnTDasAbY Q0V idle claude last=12m: office / main / claude
sQ0Vx1aB2cD3 Q0V unreachable since 09:40 codex last=3d: office / api / codex
sK7m0aaaBBBB K7m stale 09:58 - last=-: -
next: herdr-threads seat list --cursor ...
```

- Row grammar: `SEAT TAG STATE HARNESS last=AGE[: WS / TAB / PANE]`.
  - `STATE` is one of `active|idle|inactive|unreachable since T|stale T|-`.
  - Peer-chosen labels come only after the first `": "` and go through
    `one_line()` (output_compact convention).
- Size: 31-33 fixed bytes plus `STATE` (4-24 bytes), harness, and labels
  (at most 3×64 + 6 bytes).
  - Typical rows run 60-80 bytes.
  - Paging stays at the existing default of 20 rows / 16 KiB.
- Human table: a `NODE` column is added and `SPACE TAB PANE` is filled for
  remote rows.
  - The existing advisory preface stays.
  - Unreachable rows show `unreachable since T` in `PRESENCE`.

### 6.2 `thread participants` / `thread show`

- The compact remote row changes:
  - before: `SEAT joined remote` (24 bytes)
  - after, fresh: `SEAT joined remote idle claude last=12m` (+17-25 bytes)
  - unreachable home: stays `SEAT joined remote`. The existing one-per-node
    line `node Q0V unreachable since 09:40` already says it (no per-seat
    repeat).
- The location suffix for remote seats becomes
  `location SEAT: office / main / claude [advisory]`, served by
  `ParticipantLocations` from `remote_seat_status`, the same shape as local
  seats. This replaces `remote; labels unavailable` (the line was 46 bytes
  and stays roughly the same length).
  - Stale or unreachable: the labels are still shown, with
    `[advisory, last known]` (+12 bytes).
- No new line for local seats. Default sends, check-ins and deliveries are
  untouched: none of them render participants.

### 6.3 `seat inspect`

It gains one line for remote seats:
`directory: Q0V idle claude last=12m seen=4s: office / main / claude`
(~67 bytes). `seen` is the snapshot age. Unreachable:
`directory: Q0V unreachable since 09:40 claude last=3d: office / main / claude`.

### 6.4 Protocol

`SeatSummary.directory`, `Participant.directory` and
`ParticipantLocation.remote_labels` are added as `#[serde(default,
skip_serializing_if = "Option::is_none")]` fields. A new daemon capability
`seats.directory_v1` (in `src/protocol/capabilities.rs`) lets a newer CLI
use the filters only against a daemon that has them; an older daemon gets
the plain list and the CLI says `filters need a newer daemon`. Harness ids
are opaque adapter ids from the binding. The core does not branch on them
(harness-neutral).

## 7. Timeouts and constants

| Constant | Value | Origin |
| --- | --- | --- |
| `PING_AFTER` / `DEAD_AFTER` | 30 s / 90 s | existing |
| `UNREACHABLE_AFTER_MS` | 180 s | existing; also the snapshot staleness bound |
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
- **Store:** migrations 0006 and 0007 are additive. A downgrade leaves the
  two tables unused.
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
- `peer_has_ours` tracks the peer's `have` for our key, and `last_rx`
  advances on pings.
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
   `seat list --node <tag1>` shows the seat as `idle`/`active` with its
   harness and labels.
3. Cut the link (`partition(0,1)`, or `hold_frames_from(1)` with `gated`).
   Then use `age_contact(0, node1, 200 s)` so the test does not wait 180 s.
4. On node 0:
   - `peer list` shows `unreachable since <T>` with the last-known version
     and positions;
   - `seat list` shows the seat as `unreachable since <T>`;
   - no row says `active` or `idle`;
   - `thread participants` shows `node <tag> unreachable since`.
5. Heal. Presence returns within one refresh.
6. Restart node 0 while it is still partitioned. The status is served from
   the tables, not vanished.

**Stale path:** hold only `SeatStatus` (a failpoint suppresses publishing)
while pings continue. After the bound, the seat shows `stale`.

All tests run under `nice -n 19` with cluster fixtures and private state
directories only. Every test spawns through `test_support::spawn`.

## 11. Phases and commits

1. **Node status:** `NodeInfo` frame and cap, `LinkEntry` fields,
   `sync_peer_status`, `peer list`/`sync status` rows, log timestamps.
   This closes ht-b3k6.
2. **Seat status:** publisher (labels read, presence), the `SeatStatus` frame,
   relay, `remote_seat_status`, the freshness rule, and the TRUST-POLICY
   amendment.
3. **Directory:** `seat list` filters and compact form, participants,
   locations, inspect, the `seats.directory_v1` daemon cap, the skill line
   (once per session, in SKILL.md), and the multinode e2e.

## 12. Open questions for review

1. **Presence vocabulary.** Should it be `active|idle|inactive`? Today's
   local `active` means "connected binding", but here it would mean
   "connected and mid-turn". The alternative `busy|idle|offline` avoids
   overloading the word. I lean towards `busy|idle|offline` and leaving the
   existing `active/inactive` column alone.
2. **Label reads.** The home daemon would make one extra `api snapshot` call
   every 60 s, only while a peer with the cap is linked. Alternatively,
   labels could be lifted out of the reconcile enumeration that already
   fetches the snapshot. That touches the observation lane, so I kept it
   separate.
3. **Pane cwd.** Should it be published too? It is useful for finding the
   right agent, but it is more than the brief asked for. Default: no.
