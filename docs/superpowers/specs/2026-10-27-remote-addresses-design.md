# Addresses on other machines (`@MACHINE`)

Status: design, awaiting review. Builds on cross-node sync
(`2026-10-13-cross-node-sync-vision-design.md`, TRUST-POLICY "Cross-node
sync" N1-N13), the unified seat selector
(`../runs/2026-10-15-unified-seat-selector/`) and node status / seat
directory (`2026-10-27-node-status-and-seat-directory-design.md`).

User request (verbatim): "let's make sure that all cli commands that accept a
seat/tab, accept it on another machine. we already have notation like
[<space_name>/]<tab_name>[/<pane_name>], so lets extend it to
[<space_name>/]<tab_name>[/<pane_name>][@<machine_name>]".

User decision on machine identity (binding): "machines are identified by uuid
but also have their name taken from hostname and/or herdr's name, and follow
the same resolution protocol we have for space/tab/pane names: if its unique
it's accepted, otherwise it complains flagging all machines that match, and
asks the caller to explicitly choose by uuid".

## 1. Problem and what exists

- Every SEAT argument and every `--space/--tab/--pane` selector resolves names
  only against the **local** live Herdr topology (`seat_selector.rs`,
  `panes.rs`). A remote seat is reachable only by its exact ID.
- Remote seats are replicated (`seats.state='remote'`, `home_node`), so
  `send --seat <remote id>` already works; the home node delivers.
- `handoff --existing --seat <remote>` is refused **client-side only**
  (`handoff_delivery.rs:324` `inspect()` demands a Resolved local seat); the
  daemon's `eligible_delivery_recipient` (`seats.rs:5077`) already accepts a
  `remote` recipient.
- `--new-tab` / `launch` are driven by the CLI against its own Herdr socket;
  every handoff/bootstrap `validate()` pins `claim.host`/`claim.instance` to
  the frozen namespace (`protocol/handoff.rs:87`, `:781`).
- There is **no request/response frame** between peers: the link carries
  journal replication plus the one-way `NodeInfo` and `SeatStatus` gossip.

Findings from the laptop session, verified:

1. "Nodes have no name yet" — **partly wrong.** Every node signs its hostname
   into `node.genesis` (`GenesisPayload.host_label`, `app.rs:1428`, <=255
   bytes); it is materialized in `node_directory.host_label` on every node and
   already shown as `machine` by `seat list` (`seat_directory.rs:57`) and
   matched by `seat list --on` (case-insensitive, `seat_directory.rs:126`).
   Herdr has **no** instance or machine name: a Herdr server is identified
   only by its socket endpoint. So no new name publication is needed.
2. Remote seats replicated, actions on a host fail — confirmed (above).
3. Remote labels unavailable — superseded in 2.4.0: labels travel on
   `seat_status` gossip into `remote_seat_status`, but only for seats that
   exist, advisory and up to 3 min stale, and TRUST-POLICY N13 forbids them
   from feeding seat resolution.
4. A cross-node launch is a new attribution path — confirmed; §7.

## 2. Decisions

**D1. Machine identity is the node.** A machine in an address is a node
(one daemon and store per machine and OS user). Its identifiers, all already
replicated:

| Form | Matches | Uniqueness |
|---|---|---|
| `node_uuid`, 32 lowercase hex | exactly that node | unique |
| uuid prefix, 8-31 hex | nodes whose uuid starts with it | unique if one match |
| tag, exactly 3 base62 (case-sensitive) | the node with that tag | unique by the retag rule |
| name | `host_label` equal ignoring ASCII case, or its first DNS label (`blacky` matches `blacky.dayton`) | unique if one match |

All forms are tried together and pooled (the selector's "all readings" rule):
one distinct node → accepted; several → refused listing every match as
`<uuid> <tag> <host_label>` and the hint to pass `@<uuid>`. Our own node is a
candidate like any other; an address that resolves to it is resolved locally,
exactly as today. "herdr's name" has no source today; if Herdr gains a
machine name it becomes one more pooled reading (follow-up, not built).
Accepted limit: `host_label` is captured at genesis, so a later hostname
change is not reflected.

**D2. Grammar.** `ADDRESS := LOCATION ['@' MACHINE]`, where `LOCATION` is
today's seat argument (exact seat ID, `SPACE/TAB/PANE` readings, nickname
`PREFIX/SEAT_ID`, pane ID). Splitting:

- *Raw form* (no `path:` prefix): if the string contains `@`, it has two
  readings — the whole string, local (today's meaning), and split at the
  **last** `@` into `LOCATION@MACHINE` (both parts nonempty). Both are
  resolved; if both hit, the result is `Ambiguous` and lists both, exactly as
  raw and decoded readings already collide today. A machine suffix that names
  no known node contributes no reading (so `user@laptop` as a pane label on a
  network without a `laptop` node keeps working). If no reading hits and the
  suffix matched no node, the error names the unknown machine.
- *Suffix names a known node that cannot answer* (unreachable, cap missing,
  timeout, `HostUnavailable`): the remote reading is unevaluated, so the
  ambiguity rule cannot be applied. The address is **refused** with the
  remote reason and, if the local reading hit, the hint to escape:
  `cannot check PATH@M: <reason>; for the local name use 'path:...\@...'`
  (fixed part 52 bytes). Accepting the local hit would be the silent
  fallback D7 forbids. The cost is a refusal for a local `@` label whose
  suffix happens to be an offline node's name, with an escape that always
  works. Consequence: every raw address whose suffix names a known remote
  node costs one round trip, which is the point of writing `@M`.
- *`path:` form*: `\@` is always a literal `@`, and an unescaped `@` after
  the last component is the machine separator, at most one. The escape set
  becomes `\/ \\ \@`; any other backslash sequence is still an error. The
  same dual reading as the raw form applies to an unescaped `@` (whole
  string local, split remote), so an existing `path:` address with a literal
  `@` in a label keeps its meaning while its suffix names no known node: no
  breaking change, no version impact beyond the feature's minor bump.
- An exact seat ID is still tried first, on the whole raw string. `SEAT_ID@M`
  is accepted: it is the seat if its `home_node` is M, else refused
  (`seat S lives on <tag host>, not M`).
- `MACHINE` has no escapes: uuid, tag and hostnames contain no `@`, `/` or
  `\`.
- `--space`, `--tab` and `--pane` accept the same `@MACHINE` suffix (label or
  ID, then `@`). Every address in one invocation that targets a node must
  name the same node (narrowing, recipients of one `send`, rebind seat and
  pane); otherwise refused (`addresses name different machines: A, B`).
  A selector with no `@` next to one with `@M` inherits M (`--seat x@laptop
  --tab main` narrows on laptop).
- Parent inheritance on M: locally, a lone `--pane`/`--tab` inherits the
  caller pane's space and tab (`panes::resolve_parents`). On M the caller has
  no pane, so a remote `--pane P@M` or `--tab T@M` without the parent flags
  pools globally over every workspace and tab on M (tier `any`): one match is
  accepted, several are `Ambiguous` and listed, as a SEAT name already is.

**D3. Remote resolution: the owning daemon resolves.** Rejected
alternative: resolve against the `remote_seat_status` label view. It is
advisory gossip (N13), up to three minutes stale, lists only panes that
already hold a seat, carries no workspace list (so `--new-tab --space` could
never resolve) and no tab/pane IDs, and would make every node a second judge
of another node's topology. Chosen: the CLI sends the unresolved address to
its daemon, which forwards it over the authenticated link to the home node;
that daemon runs the **same matcher** against its own live Herdr topology
(every attached host) and its own seat table, and answers with the outcome.
Consequences:

- One rule, one judge (A2): the node that owns the panes decides.
- Fresh: no staleness window; unbound panes, tabs and workspaces resolve.
- Cost: resolving a name on M needs M reachable over a direct link. An exact
  seat ID needs nothing and works offline, which the errors say.
- Proximity tiers do not apply (the caller has no pane on M): every
  candidate is tier `any`, so a second candidate is ambiguous.
- The pure resolution code moves into a neutral module
  (`src/protocol/address/`) so the service can call it: from
  `src/cli/seat_selector.rs` the `Selector`, readings, `reading_matches`,
  tiering and `Nickname::verify`; from `src/cli/panes.rs` the selector
  locator (`resolve_selector`, `resolve_parents` with the caller pane made
  optional) and the `looks_like_pane_id`/`looks_like_tab_id` shape tests. All
  of it takes a `HostTopology` and an optional caller pane, and none of it
  touches the CLI host or the daemon socket. The CLI keeps argument
  collection, the `SeatInspect`/`Seats` lookups and rendering. Pure move, no
  behaviour change, its own commit (the existing selector and panes tests
  move with it unchanged).
- On a multi-host M, resolution reads every attached host within the 3 s
  budget; if any host fails to read, the outcome is `HostUnavailable` unless
  the hosts that did read already make it `Ambiguous` (a partial read could
  otherwise give a false unique match or a false `NoMatch`).

**D4. Resolution yields a seat ID; the command then behaves as if given that
ID.** A remote resolve is read-only on M and attributes nothing. Once the CLI
holds the seat ID, every existing rule applies unchanged: the local daemon
decides the command (A2), a send is authored by the caller's home node and
delivered by M (N3/N4), lookup slots on remote seats behave as with the exact
ID today. A location that resolves to a pane on M with no seat is refused in
phase 1 for every slot, Allocate included (allocation is a mutation on M;
phase 3).

**D5. Remote actions are requests to the home node, never remote facts.**
Phase 2 adds one remote mutation, *launch* (new tab or existing pane), which
M's daemon executes on behalf of the linked peer and records with a new
provenance (§7). Everything else in a handoff (thread, invite, task message,
ACK request) stays authored by the caller's node as today, addressed to the
seat the launch produced. No node ever authors a fact for another node's
seat (N5 stays).

**D6. Direct links only.** A request travels only over a live direct link to
M; relayed requests are a follow-up. A mesh of 2-10 SSH-linked nodes is
almost always directly linked.

**D7. Capability per phase, clear refusal.** Each phase's frames are gated by
their own cap. A peer that lacks it gets no frame; the caller gets a typed
refusal naming the machine, the missing cap and the peer's last known
version. Never a silent local fallback.

## 3. Affected arguments

From `src/cli/commands.rs` (13 SEAT help strings plus the pane selectors):

| Command | Argument | Slot | Phase |
|---|---|---|---|
| `inbox`, `pending-receipts`, `diagnostics`, `warnings`, `thread list` | `--seat`, and `--space/--tab/--pane` | Lookup | 1 |
| `seat inspect` | `SEAT` | Lookup | 1 |
| `send` | `--seat` (1:1), `--require-ack SEAT`, `--require-ack-pane PANE`, `--space/--tab` | Allocate (existing seat only in 1) | 1 |
| `invite THREAD` | `--seat`, `--space/--tab/--pane` | Allocate (existing seat only in 1) | 1 |
| any `--seat` narrowing | `--space/--tab` | narrowing | 1 |
| `handoff --existing` | `--seat`, `--pane` | Lookup | 2 |
| `handoff` (launch into pane) | `--pane` | launch target | 2 |
| `handoff --new-tab LABEL` | `--space SPACE@M` picks the machine; `LABEL` stays a label | launch target | 2 |
| `launch` | `--pane` | launch target | 2 |
| `seat resolve` | `--space/--tab/--pane` | operator, allocate | 3 |
| `seat rebind` | `SEAT`, `--replace NEW`, `--pane` | operator | 3 |
| `seat retire` | `SEAT` | operator | 3 |

Not addresses, unchanged: `--cooperative-target`, `--cooperative-seat`
(hook-internal, local by definition), `_topology-recover --created-pane`,
`seat list --node/--on` (already machine filters), `peer forget`,
`sync status --log`. Remote `--new-tab` needs `--space` (no caller
workspace exists on M): `remote --new-tab needs --space SPACE@MACHINE`.

Help text: the 13 strings become `SEAT: exact ID or SPACE/TAB/PANE[@MACHINE];
single-quote path: names with \/ \\ \@ escapes` (+8 bytes each); the pane
selector help gains `[@MACHINE]`. SKILL.md gains one example line in the
existing addressing section (~70 bytes).

## 4. Phase 1: names and addressing for reads and send

### 4.1 Machine resolution (daemon)

`Command::ResolveMachine { raw }` is not a separate round trip; machine
resolution is part of the resolve request (4.3) so the daemon decides it
against the canonical `node_directory` (A2). A pure function
`node_directory::match_machine(rows, raw) -> MachineMatch { One(row) |
Many(rows) | None }` implements D1 and is unit-tested.

### 4.2 Wire

```rust
pub const ADDR_RESOLVE_CAP: &str = "addr.resolve.v1";

// cap: ADDR_RESOLVE_CAP
AddrResolve(AddrResolve),
// cap: ADDR_RESOLVE_CAP
AddrResolved(AddrResolved),

pub struct AddrResolve {
    pub id: Uuid,              // correlation, fresh per request
    #[serde(default)]
    pub target: AddrTarget,    // what to resolve (below)
}
pub enum AddrTarget {          // #[serde(tag = "k")], #[serde(other)] Unknown
    Seat { location: String, decoded: bool },   // SEAT argument without @M
    Pane { space: Option<String>, tab: Option<String>, pane: Option<String> },
    Narrow { seat: String, space: Option<String>, tab: Option<String> },
}
pub struct AddrResolved {
    pub id: Uuid,
    #[serde(default)]
    pub outcome: AddrOutcome,
}
pub enum AddrOutcome {         // #[serde(tag = "k")], #[serde(other)] Unknown
    // `at`: the counter of the seat's `seat.created` fact in M's own log
    // (origin M, its current incarnation), so the caller can wait until the
    // seat row has replicated (4.3 step 5).
    Seat { seat: SeatId, path: String, pane: String, #[serde(default)] at: Option<FactPos> },
    NoSeat { path: String, pane: String },       // pane resolved, holds no seat
    NoMatch,
    Ambiguous { candidates: Vec<AddrCandidate>, omitted: u32 },  // <= 32
    HostUnavailable,                             // no live Herdr on M
    Refused { detail: String },                  // <= 256 bytes
}
```

Bounds: `location` and each label <= 512 bytes, enforced on both ends; a
reply over 64 KiB is refused. `Unknown` target → `Refused{"unsupported
target"}`; `Unknown` outcome → caller error `machine M answered in an
unknown form`. Both frames pass `scripts/check-wire-compat` (every field
`serde(default)`, `// cap:` markers); `scripts/freeze-peer-wire` gains a
fixture set for the release that ships this, and the 2.1.1/2.4.0 fixtures
stay byte-exact (the hello gains only the cap string).

### 4.3 Flow

1. CLI parses each address into `(location, machine?)` per D2. No machine,
   or a machine the daemon reports as self: today's path.
2. Otherwise the CLI sends `Command::RemoteResolve { machine, target }` to its
   daemon (a read; no claim needed beyond today's read rules; subagents may
   use it).
3. The daemon resolves the machine (4.1), checks the direct link is `Active`
   and its negotiated caps include `ADDR_RESOLVE_CAP`, sends `AddrResolve`,
   and awaits `AddrResolved` with the same id for at most
   `REMOTE_RESOLVE_TIMEOUT = 10 s`. Pending requests live in memory in the
   coordinator keyed by id; a link drop fails them at once.
4. M's coordinator hands the request to a `RemoteResolver` service task
   (bounded: 8 in flight per link, else `Refused{"busy"}`), which reads the
   topology of every attached host through `HostSessions` (3 s budget), runs
   the matcher with global tiering and maps the pane to its seat through the
   seat table (local seats only).
5. Before answering the CLI, the caller's daemon waits until its applied
   vector covers `at` (the seat row exists locally), bounded by
   `REPLICA_WAIT = 5 s`; past it the CLI gets `seat S on <tag> has not
   replicated here yet; retry` (51 bytes + ids). Without this the follow-on
   command (and phase 2's launch-then-handoff) could find no local seat row.
6. The CLI renders the outcome with `@<tag>` appended to each path, writes
   the usual `resolved: ... seat=<id> (tier: any) @<tag> <host>` line, and
   continues with the seat ID (D4).

Call budgets: today's cooperative CLI→daemon budget is 5 s
(`cli/mod.rs` `cooperative_budget`). `RemoteResolve` gets its own budget of
`REMOTE_RESOLVE_TIMEOUT + REPLICA_WAIT + 5 s` = 20 s, so the daemon's typed
errors always reach the caller before the client gives up. Several remote
addresses in one invocation resolve concurrently (one request each). Name
resolution never runs inside a hook: hooks and check-ins act on exact IDs,
so hook latency is unaffected.

### 4.4 Errors (caller-visible, sizes are fixed parts)

| Case | Text | Bytes |
|---|---|---|
| no node matches | `no machine M; known: <tag> <host>, ...` (<= 8 listed) | 21 + list |
| several nodes | `ambiguous machine M: N matches` then one line each `  <uuid> <tag> <host>`, then `pass @<uuid>` | 29 + 52/line + 12 |
| not directly linked / down | `machine <tag> <host> is unreachable (since T); pass an exact seat ID` | 54 + names |
| peer lacks cap | `machine <tag> <host> cannot resolve names (needs addr.resolve.v1; version V)` | 60 + names |
| timeout | `machine <tag> <host> did not answer in 10s; pass an exact seat ID` | 52 + names |
| `NoSeat` | `no seat at PATH@<tag> (pane P); a seat appears once an agent there checks in` | 64 + path |
| `NoMatch` | today's NoMatch text with `@<tag>` on the selector | +4 |
| `Ambiguous` | today's candidate list, each path suffixed `@<tag>` | +4/line |
| `HostUnavailable` | `machine <tag> <host>: seat names need its live Herdr topology` | 46 + names |
| different machines | `addresses name different machines: <tag>, <tag>` | 36 + tags |
| seat on other node | `seat S lives on <tag> <host>, not M` | 17 + names |

Agent-facing; all one line except the ambiguity lists, which reuse today's
bounded format (32 lines + `(N omitted)`).

### 4.5 Trust (phase 1)

TRUST-POLICY gains **N14 Remote resolution**: a name qualified by `@M` is
resolved by M's daemon against its own live topology and seat table, over an
authenticated direct link (N6), as a read that writes and attributes nothing
on M; the caller gets only a seat ID, and the command is then decided by the
caller's daemon exactly as for that exact ID. N13 is unchanged: cached remote
labels still never feed resolution. Accepted limit "a linked node can answer
a resolve falsely" joins "Remote presence is advisory" (cooperative, same
user). No provenance value: nothing is attributed.

## 5. Phase 2: remote handoff and launch

### 5.1 `handoff --existing` to a remote seat

No new remote mutation. Resolution (phase 1, or the exact ID) yields a remote
seat; the client-side `inspect()` accepts `ContinuityStatus::Remote` when a
fresh owner check says the seat is eligible: a new `AddrTarget::Check {
seat }` variant inside the phase-1 frames, sent only to a peer that also
advertises `PEER_LAUNCH_CAP` (an older peer gets the cap refusal, never a
frame it would answer `unsupported target`), answered by M with
`AddrOutcome::Eligible` or `Ineligible{reason}` from the same predicate the
local inspect uses (resolved, nonretired, unheld, target present). M unreachable → refused
(`cannot confirm seat S on <tag> <host>: unreachable; use send --require-ack`,
~70 bytes): a handoff promises a live recipient, so we do not hand off blind.
The thread, invite, task and ACK request are then authored by the caller's
node; M delivers and wakes per the unchanged delivery contract.

### 5.2 Remote launch (`--new-tab`, `--pane P@M`, `launch --pane P@M`)

```rust
pub const PEER_LAUNCH_CAP: &str = "peer.launch.v1";
// cap: PEER_LAUNCH_CAP
LaunchRequest(PeerLaunchRequest),   // id, requester seat (claimed), frozen plan
// cap: PEER_LAUNCH_CAP
LaunchReply(PeerLaunchReply),       // id, Launched{seat, pane, tab} | Refused{code, detail} | Pending
```

- The frozen plan is today's `LaunchRequest` / bootstrap identity minus the
  caller-host fields: target (`NewTab{space, label}` or `Pane{selector}`),
  harness id, profile, argv, `cwd` (an absolute path **on M**, checked on M),
  agent name. The requester's local `--cwd` default (its own cwd) does not
  apply; remote launch requires an explicit `--cwd` (`remote launch needs
  --cwd DIR on M`, 34 bytes).
- **Request id.** R is derived, not random: the caller freezes the handoff
  or launch intent in its CLI intent journal (`src/cli/journal.rs`, as every
  handoff already is) and R is the intent's launch child operation key. A
  retry of the same intent (link drop, CLI crash, caller daemon restart, the
  existing handoff retry path) re-sends the same R; a new deliberate launch
  is a new intent and so a new R, and never collides.
- M's daemon records the request in a new local table
  `peer_requests(id PK, node_uuid, kind, state, result, attempt,
  created_at, updated_at)` before acting, states `running → done | failed`;
  a repeated id returns the recorded result, or `Pending` while `running`.
  This is the idempotency key across link drops and caller retries, so no
  double tab.
- **Crash recovery on M.** The `_peer-launch` child records its progress in
  the existing bootstrap attempt records (`ReserveBootstrapAttempt`,
  `RecordBootstrapCreated`), keyed by R. On daemon start, and when a child
  exits without a result, a `running` row is settled from those records:
  no tab created → `failed{interrupted}`; tab created → the existing
  bootstrap recovery (`Bootstrap creation attempts and administrative
  recovery` in TRUST-POLICY) adopts or refuses it exactly as for a local
  interrupted bootstrap, and the row takes that outcome. A row still
  `running` after `PEER_REQUEST_DEADLINE = 10 min` is failed the same way.
  `done` and `failed` rows are kept 7 days for retries and audit, then
  deleted by the daemon's existing periodic maintenance.
- M executes the launch with the existing code path, run by the daemon as a
  child process of its own binary (`herdr-threads _peer-launch --request ID`,
  hidden, owner UID, spawned with the daemon's environment) so the CLI-side
  tab creation, `agent.start` and `ObservedStartup` stay one implementation.
  The child acts as a new `OriginalActor::Peer { node }`; the handoff and
  bootstrap `validate()` functions accept it in place of the cooperative
  claim only for the bootstrap/launch commands, with the namespace taken
  from M's own daemon (its instance and the selected local host).
- C2 holds, retirement and the `ResolveBootstrapSeat` guards apply unchanged:
  a held target is refused.
- Reply after the seat is resolved, or `Refused`. The CLI does not hold one
  long call: it sends `RemoteLaunch{R}` (returns at once with `Pending`,
  `Launched` or `Refused`) and then polls `RemoteLaunch{R}` every 2 s, each a
  normal short call, up to `REMOTE_LAUNCH_TIMEOUT = 120 s`; the caller's
  daemon re-sends R over a re-established link. Past the timeout the CLI
  reports `launch on <tag> still pending: request R; rerun to resume`
  (~60 bytes) and exits non-zero; the intent stays journaled, and the
  existing handoff retry (or re-running the command) resumes with the same R.
- `Launched` carries the new seat's `at` position; the caller waits for it
  to replicate as in 4.3 step 5, then runs today's `--existing` delivery
  against it (5.1), so the handoff message is authored by the caller's node.
  If that delivery fails after the launch succeeded, the intent records the
  launch as done and the retry resumes at delivery, never relaunching.

### 5.3 Trust (phase 2)

- New A3 provenance **`peer_request:<node_uuid>`** (launch and bootstrap
  audit rows on the executing node only, never bindings or receipts): the
  linked peer node `<node_uuid>`, authenticated by its link (N6), asked this
  node to launch; the requesting seat travels as a claim and is recorded as
  claimed, never as actor. The binding the launch opens keeps
  `managed_launch`.
- A5 table gains the row "launch on behalf of a linked peer node | the
  executing node's daemon, on an authenticated direct link with
  `peer.launch.v1`".
- N5 is restated, not weakened: the remote seat is never an actor on M; the
  peer node is.
- Accepted limit: any linked node can make this node open a tab and start an
  agent (same user, cooperative; it is what `ssh M herdr-threads launch`
  could do).

## 6. Phase 3: remote operator repair

`seat resolve`, `seat rebind`, `seat retire` with `@M`: an `OperatorRequest`
frame under `peer.operator.v1`, executed by M's daemon as
`OperatorActor::Peer{node}` with audit label `operator:peer:<node_uuid>`
(new A3 value). N10 holds: a rebind whose seat and pane name different
machines is refused before any frame is sent. Designed here for completeness;
specified in detail and reviewed before phase 3 starts.

## 7. Failure modes (all phases)

| Failure | Behaviour |
|---|---|
| M unknown / ambiguous | refused locally, no frame (4.4) |
| M is self | local resolution, no frame |
| no direct link, link down, peer offline | refused with `unreachable`; exact seat IDs still work for lookups and send |
| older peer (cap missing) | refused naming the cap and peer version; no frame sent |
| link drops mid-request | pending requests fail at once; launch retried by id (5.2) |
| peer answers late | late reply dropped (id no longer pending) |
| remote Herdr down | `HostUnavailable` |
| seat retired between resolve and use | decided by the existing rules for that exact ID |
| resolved seat not yet replicated here | caller daemon waits up to 5 s for `at`, then `has not replicated here yet; retry` |
| raw `@` suffix names a known node that cannot answer | refused with the reason and the `\@` escape hint (D2) |
| one host of a multi-host M fails to read | `HostUnavailable` unless already `Ambiguous` |
| M's daemon or `_peer-launch` dies mid-launch | `running` row settled from bootstrap records at restart/child exit; deadline 10 min |
| caller CLI or daemon dies mid-launch | intent journal keeps R; retry re-sends R, gets the recorded result |
| launch done, delivery failed | retry resumes at delivery, no second launch |
| request flood | 8 in flight per link, `Refused{busy}` |

## 8. Tests

Two-node harness `test_support::multinode::Cluster` with `FakeHerdr` hosts
(tests/multinode/). Additions to test support:
- `FakeHerdr`: labels on workspaces, tabs and panes (today every pane's
  label is its id and workspaces/tabs have none), and, for phase 2,
  `tab.create` and `agent.start` answering like Herdr (a new pane in a new
  tab, an idle agent whose startup the launcher observes). If the
  `agent.start`/`ObservedStartup` fake proves too large, the phase-2 new-tab
  case runs only on `IsolatedHerdr` and the fake covers `--existing`.
- a test-support env `HT_TEST_HOST_LABEL` overriding the genesis hostname
  (all cluster nodes share one real hostname, which is itself the ambiguity
  fixture);
- `ClusterOptions.withhold_caps` dropping named caps from one node's hello
  (older-peer case);
- a failpoint holding `AddrResolved` on the answering node (timeout and late
  reply).

Phase 1:
- unit: grammar (`@` split, last-`@` rule, raw dual reading, `path:` with
  `\@`, `SEAT@M`), `match_machine` (uuid, prefix, tag, name, first DNS label,
  case, self, ambiguity listing).
- `send --seat 'ws/tab/pane@b'` from a reaches b's seat; `inbox --seat
  ...@b` reads it.
- same host label on both → `ambiguous machine`, then `@<uuid>` works.
- `withhold_caps=[addr.resolve.v1]` on b → cap refusal, no fallback.
- partition → `unreachable`; exact seat ID still sends.
- pane without seat → `NoSeat`; label containing `@` still resolves locally
  (suffix names no node), in raw and `path:` form.
- local `user@b` label with `b` known: both readings hit → `Ambiguous`; with
  b partitioned → refused with the escape hint; `path:user\@b` → local.
- held `AddrResolved` → timeout error within 20 s; the late reply is dropped.
- 9 concurrent resolves on one link → one `busy`.
- `send` with recipients on two different machines in one address set →
  `addresses name different machines`.
- b with no attached host → `HostUnavailable`.
- remote `--pane P@b` with no parents, P unique on b → resolved; P in two
  tabs → `Ambiguous`.
- frame fixtures: wire-compat, freeze fixture, 2.1.1/2.4.0 unchanged.

Phase 2:
- `handoff --existing --seat x@b`: task delivered on b, ACK returns to a.
- `handoff --new-tab L --space w1@b --cwd D`: tab created on b's fake Herdr,
  seat resolved on b, handoff delivered; repeated id after a dropped link
  creates no second tab.
- older peer (`withhold_caps=[peer.launch.v1]`) refused; held target refused.
- b's daemon killed while `running` → after restart the row settles from the
  bootstrap records; the caller's retry gets that outcome, no second tab.
- caller CLI killed mid-poll → re-run resumes with the same R.
- delivery failure after `Launched` → retry delivers without relaunching.
- one isolated real-Herdr case for `--new-tab` on the receiving node
  (`IsolatedHerdr`), skipped when Herdr is absent.

## 9. Acceptance criteria

- AC1 Every argument in §3 for the phase accepts `@MACHINE`; without `@` all
  existing selector tests pass unchanged.
- AC2 Machine resolution follows D1, including the multi-match refusal that
  lists uuid, tag and host and points at `@<uuid>`.
- AC3 Names are resolved only by the home node's daemon; cached remote labels
  are never consulted (N13 test stays).
- AC4 A peer without the phase's cap gets no frame and the caller gets the
  cap refusal; no local fallback.
- AC5 Peer offline / link down / timeout produce the §4.4 errors within
  10 s (resolve) and never hang.
- AC6 Phase 2: remote `--existing` and `--new-tab` hand off end to end on
  the two-node harness; launch is idempotent by request id.
- AC7 TRUST-POLICY gains N14 (phase 1) and `peer_request:<node_uuid>` plus
  the A5 row and accepted limit (phase 2) in the commits that implement them.
- AC8 clippy (all features, -D warnings), fmt, check-harness-neutral,
  check-wire-compat, check-default-features pass; focused tests plus one
  unfiltered full suite with the leak checks.

## 10. Phases and commits

1. Move the matcher to `src/protocol/address.rs` (no behaviour change).
2. Grammar: `@` split, `\@` escape, help strings.
3. `match_machine` + `RemoteResolve` command + `AddrResolve` frames +
   `RemoteResolver`; CLI wiring for the phase-1 rows; N14; tests.
4. (phase 2) eligibility check, remote `--existing`.
5. (phase 2) `peer_requests`, `PEER_LAUNCH_CAP` frames, `_peer-launch`,
   `OriginalActor::Peer`, TRUST-POLICY provenance; tests.
6. (phase 3) separate design addendum, then implementation.

Each phase merges on its own: phase 1 changes no mutation path; phase 2 adds
only the launch path.

## Post-Implementation Notes

> *Append dated notes here as the design is implemented and diverges.*

### 2026-10-27, phase 1 as built

- **Scope.** Phase 1 covers the SEAT arguments: `--seat` (reads, `invite`,
  direct `send`), `--require-ack SEAT`, and the positional `SEAT` of `seat
  inspect|rebind|retire` (rebind and retire of a remote seat stay refused by
  N5 once resolved, as with its exact ID). The `--space/--tab/--pane` flags
  keep their local meaning in phase 1 (`@` stays a literal character there);
  `@MACHINE` on them moves to phase 2, where `launch`/`handoff --pane P@M`
  need it anyway. Every remote pane is already addressable as
  `--seat 'SPACE/TAB/PANE@MACHINE'`. `--space/--tab` narrowing of a remote
  `--seat` address is refused (`--space/--tab cannot narrow a remote address;
  qualify its path instead`, 71 bytes). `AddrTarget::Pane` and the remote
  parent rule (`protocol::address::resolve_parts`) are on the wire and in the
  resolver now, so phase 2 adds no frame.
- **Nicknames.** `SEAT_ID@M` is checked by M (its own non-retired seat, else
  `seat S does not live on this machine`); `PREFIX/SEAT_ID@M` is not resolved
  remotely (no match there).
- **Replication wait.** `AddrOutcome::Seat` carries no `at` position: the
  asking daemon polls for the seat row with `home_node = M` (every 100 ms, up
  to `REPLICA_WAIT_MS` = 5 s), which also enforces that the seat is M's.
- **Fallback.** An unknown machine suffix, or a local daemon that cannot
  resolve machines, falls back to the local reading only when the whole
  string names a live local pane; otherwise the unknown machine is reported.
- **Resolved line.** `resolved: PATH@TAG PANE seat=ID (tier: any)`.
- **Old peer.** A peer is "older" when its hello lacks `addr.resolve.v1`;
  test-support daemons drop it with `HT_TEST_WITHHOLD_CAPS=addr.resolve.v1`.
- **Test harness.** The multinode dial shim now sources the destination
  node's own environment (`cluster/env/<node>`), so a daemon a peer's dial
  starts no longer inherits the dialer's; `FakeHerdr` panes take `ID=LABEL`;
  `HT_TEST_HOST_LABEL` stands in for the hostname at genesis.
- **Host label shown.** `peer list` node rows end in ` host=LABEL` (6 bytes
  plus the label), the name `@MACHINE` matches.
- **Deferred to phase 2.** The D2 rule that every address of one invocation
  names the same machine (`addresses name different machines`) is not built:
  in phase 1 each SEAT argument resolves on its own machine, and recipients on
  different machines in one `send --require-ack` are legitimate. Phase 2
  enforces it where it matters: narrowing, and a rebind's seat and pane.
- **Budgets (PR roast round 1).** The daemon transport gives `resolve
  address` its own bound, `REMOTE_ADDRESS_DAEMON_BOUND_MS` = 10 s home-node
  wait + 5 s replica wait + 2 s; the CLI waits 3 s longer, so the typed
  refusal always reaches it. AC5 reads accordingly: a refusal within the
  20 s CLI budget, never a hang. Timeout, late reply and the 8-request bound
  are tested at the coordinator (`remote_resolve_times_out_and_bounds_inbound_requests`),
  multi-host pooling and the host-failure rule at the resolver (`pool`), and
  the asker's N14 seat check at the service (`resolve_address_matches_machines_and_takes_only_the_named_nodes_seat`);
  the multinode test covers the end-to-end paths.
- **Error classes.** Unreachable, no cap, timeout, not replicated and no
  live Herdr are `host_unavailable` (retryable); a peer's request-shaped
  refusal (`seat S does not live on this machine`, invalid path, a pane with
  two seats) and an unknown answer are `invalid_request`. Only a daemon
  without sync administration (`unsupported`) counts as knowing no machines;
  every other failure of `resolve address` is passed through as a refusal.
