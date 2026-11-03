# Named seats: herdr-threads outside Herdr

Bead: ht-kqrt. Status: draft. Vision and design agreed with the user section by section (Mode A).

## Goal

An agent session started with `HERDR_THREADS_SEAT_NAME=<name>` in its environment, anywhere on a
machine (no Herdr server at all, beside Herdr, headless, or on a node linked to Herdr machines), gets a
stable seat of that name and can join, send, read, ACK and receive delivery through its turn hooks or
channel exactly as a pane seat does.

## Use cases (all in scope)

1. **Machine without Herdr**: plain terminal, tmux or IDE; the daemon runs with no Herdr host.
2. **Beside Herdr**: Herdr runs locally, some agents live outside it and talk to pane seats.
3. **Headless / scripted**: `claude -p`, `codex exec`, cron loops that act under a fixed name and exit.
4. **Remote via peer link**: a Herdr-less node linked to Herdr nodes; its named seats are addressable as
   `name@node`.

## Decisions

| Topic | Decision |
|---|---|
| Identity | The name is a role. Any later or concurrent session with the same name on the same node binds the same seat and inherits its threads, unread mail and ACK obligations (concurrent sessions are bindings of one seat, as in a pane). Seats are never merged. |
| Creation | Automatic on first use (any CLI call or hook carrying a new name), with one stderr line `herdr-threads: created seat <id> (<name>)`. |
| Retirement | Explicit only; never on idleness or other heuristics. Operator: `human seat retire SEAT --operator`. Own agent: `seat retire --self` (below). A retired name used again later mints a new seat; the retired seat is never revived. |
| Precedence | `HERDR_THREADS_SEAT_NAME` (non-empty) wins over `HERDR_PANE_ID`; the pane is ignored entirely (no pane target, no Native wake, no pane labels), so a named seat behaves identically inside and outside Herdr. |
| Namespace | Per node. `reviewer` on node A and on node B are different seats; cross-node addressing is `reviewer@B`. No cross-node name ownership. |
| Names | 1–64 bytes of `[A-Za-z0-9._-]`; must not contain `/` or `@`, must not parse as a seat ID, must not start with `-` or `.`. An invalid name is a hard error naming the variable; it never falls back to the pane. |
| Wake | Named seats have no Native wake. Harnesses whose idle wake is `Channel` (Claude with the mod: `watch` runs under the same env and the mod submits its own turn) keep idle wake. Others get turn-hook delivery and the pending hint only. |
| Sender warnings | The existing one-time `warning: <seat> wake=no_wake` per (sender, seat). No retries, no `unreceived`/`refused`, no repeated warnings. |
| Handoff / launch | Targeting a named seat is refused in v1 (`named seat <name> has no pane to launch into`). A named seat may still hand off to Herdr panes when a Herdr host is reachable. |

## Architecture: the named host

Every node has one built-in, always-attached host that is not Herdr: the **named host**. Its targets are
seat names. It plugs into the existing host port, so seats, bindings, claims, mapping checks, liveness,
operator retire, selectors and the remote resolver keep their current code paths.

- **Locator**: a reserved canonical path `<state_dir>/node/named-host` (never a socket; `HostLocator`
  stays an absolute path, so `CallerClaim` needs no new field). One `herdr_hosts` row, created at daemon
  start, attached in-process (no `attach_locked` socket dial), counted outside `MAX_ATTACHED_HOSTS` and
  outside idle-exit's host count (a node with only the named host is store-only for idle exit).
- **Observation**: `observe_current_target(name)` returns a present target for every *valid* name. The
  target id is the name; terminal id and incarnation are derived from the name and the named host's
  boot, which is fixed per node (it never restarts like a Herdr server, so host-epoch reconfirmation
  never fires for it). This observation is a recorded claim ("a caller presented this name"), not
  Herdr evidence; see Trust policy.
- **Topology**: the set of non-retired local named seats, labelled by name. Selector resolution and the
  home node's `RemoteResolver` read it like any host topology, so a bare `reviewer` and `reviewer@B`
  resolve with no new frame. Bare names are pooled with pane-path readings; ambiguity is refused with
  the existing error (exact seat IDs always work).
- **Composer / wake**: none. The named host's wake worker is never started; `wake_status` reports
  `NoWake` for a named seat whose harness idle wake is `Native`, and no block for `Channel`.
- **Closure**: the named host never reports a target closed; retirement only comes from the two
  explicit paths.

### Caller side (CLI and hooks)

- `HERDR_THREADS_SEAT_NAME` set and valid: the caller claim is `host = named-host locator, target = name`.
  `derive_selection`, `located_pane_seat`, hook `connect_pane`, `check_in`, `report_binding_event` and
  `foreign_session` take the named branch; `HERDR_ENV=1` and `HERDR_PANE_ID` are not required.
- Unknown name: the ordinary resolve (`ResolveSeat` → `OrdinaryIdentity::resolve`) enrolls it like a new
  pane, then prints the creation line.
- Discovery: with the name set, the state dir defaults to `$XDG_STATE_HOME`/`~/.local/state` +
  `herdr/plugins/herdr-threads` and is created when missing; no `herdr plugin list` or
  `herdr status server` subprocess runs, and the Herdr host endpoint is resolved only when a command
  needs a pane target (handoff into a pane). The daemon autostart path is unchanged.
- `setup` must install hooks with `--state-dir` and no `herdr` binary on PATH (verify; fix if not).
- Liveness fingerprint is unchanged (local process table). `liveness_candidates` already joins
  `herdr_hosts`; the named host has a row, so named bindings are judged like pane bindings.

### Self-retire

`herdr-threads seat retire --self`: acted by the caller's own top-level binding of a **named** seat.
Refused for subagents, for pane seats (they retire on pane close or by the operator) and without a
resolved caller. Recorded with provenance `cooperative_self_retire`. Effect equals operator retire:
bindings end, pending receipts settle `Retired`, `seat.retired` is published. The output lists what was
abandoned (open ACK obligations, joined threads) in compact rows.

### Peer wire

- `seat.created.display_name` (reserved, always null in v1) carries the name for named seats; older nodes
  ignore it (`materialize_seat_created` does not read it). `SeatKind` stays `agent`/`human`.
- `SeatStatusEntry` gains `name: Option<String>` with `#[serde(default)]`. No new frame, item variant or
  capability; `scripts/check-wire-compat` baseline grows only by that field.
- Remote nodes show `name@node` for a named remote seat (from `display_name`, or the status entry).

### Displays

`seat list`, `thread participants`, `thread show`, delivery `from` headers and `seat inspect` show the
name (`reviewer`, or `reviewer@B` remotely) where a pane path would be, with workspace/tab/pane empty.
`SeatSummary` and `SeatDirectory` gain an optional `name` sourced from a new nullable `seats.name`
column (migration: add column; unique among non-retired local seats on the named host).

## Trust policy (same commit as the code)

- New provenance values in the A3 table: `cooperative_named_seat` (binding observed through the named
  host) and `cooperative_self_retire`.
- New rule: a named-host observation is a recorded cooperative claim, never Herdr evidence; it never
  moves, rebinds or merges a pane seat, and a pane observation never moves a named seat.
- Accepted limit: any same-user process with the variable set acts as that seat (the analogue of an
  inherited `HERDR_PANE_ID`); a typo mints a stray seat that needs explicit retire.
- Continuity: name equality on one node is the continuity evidence; retire ends it for good.

## Delivery contract check

Rules 1–3 unchanged (priorities, threshold, in-turn delivery via hooks/channel). Rule 4: a named seat
gets no nudge at all unless its harness wakes itself. Rule 6: `Channel` wake is unchanged; `Native` is
the one degraded capability, reported once via `wake=no_wake`. Invariant 2 does not hold for a named
Native-wake seat; this is the documented degradation (TRUST-POLICY accepted limit + skill note).

## Testing

- Unit: name validation; named-host observation/topology; precedence (both env vars set).
- Integration (isolated state dir, no Herdr): create on first call; second session binds the same seat;
  concurrent sessions; send/inbox/ACK between two named seats; hook check-in and delivery (Claude,
  Codex stand-ins); one `wake=no_wake` per pair; retire --operator and --self; reuse after retire mints
  a new seat; handoff to a named seat refused; daemon idle exit with only the named host.
- Beside Herdr (isolated Herdr session): pane seat ↔ named seat messaging; pane agent with the name set
  uses the named seat; selector ambiguity between a tab name and a seat name is refused.
- Multinode (`test_support::multinode`): `reviewer@B` addressing and display; old-node decode of
  `display_name` and the new status field.
- Checks: clippy all-features, check-harness-neutral, check-wire-compat, check-default-features, full
  suite + leak check.

## Out of scope (v1)

Handoff/launch into a named seat; mesh-global names; idle-based expiry; Native wake for named seats.

## Post-Implementation Notes

> *As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
