# Vision: herdr-threads on herdr-core / the Herdr desktop app

Status: draft for user review. Decisions marked **[decide]** are recommendations and are not approved yet.
Revised after the roast (2026-10-31): spike 2C is resolved, and closure, identity, verification, wake and launch
were rewritten. The roast table is in `docs/superpowers/specs/2026-10-31-environments-proposal.md`, Part 4.
Inputs: 01-herdr-app-research.md, 03-ht-host-coupling.md, 04-mac-spikes.md (hc 0.1.15, herdr-app 0.1.10).
herdr-core source references (`input.rs`, `terminals.rs`, `api/run.rs`, `notes.rs`, ...) point into the
herdr-core tree that the roast read.

## Goal

An agent in a herdr-core terminal gets a durable seat and uses herdr-threads the way an agent in a Herdr TUI pane
does. The terminal can come from `hc run` or be a pane of the Herdr desktop app. The agent can join, send, read its
inbox, ACK, receive turn-hook or channel delivery, and message seats in Herdr TUI panes, named seats and linked
nodes. Idle wake is the one capability that waits on an upstream primitive (decision 4).

## What the new environment is

- **herdr-core is the host and the app is a view.** Each app pane is one core terminal (`tN`), and the core stores
  the layout (doc `herdr-app`). herdr-threads therefore integrates with herdr-core. The app adds only labels and,
  later, optional UI. Everything works whether or not the app is open.
- **No plugin system.** herdr-threads installs standalone: the release archive plus `herdr-threads setup`. Named
  seats need the same path on a machine without Herdr.
- **Different identity.** A terminal's env carries `HC_TERMINAL_ID=tN`, `HC_SESSION`, `HC_TERMINAL_SOCKET` and
  `HC_SCOPE`, and has no `HERDR_PANE_ID` of its own. `tN` is unique only within one core id. A wiped or cloned
  core gets a new id and starts again at t1 (herdr-core server.rs:88-118, terminals.rs:246-249).
- **The core env leaks.** A terminal gets the env of the core process, and that env is frozen from whichever
  process autostarted the core. It includes `HERDR_PANE_ID`, `CLAUDE_CODE_*` and any `HERDR_THREADS_SEAT_NAME`
  (04 step 3, Spike 2B). `terminal.run` can add or overwrite variables but cannot unset them.
- **Primitives.** The core offers a JSON-RPC socket with `events.subscribe` (snapshot, seq, heartbeat, `missed`).
  Claude Code and Codex report their status natively over OSC 7501. `terminal.send` accepts `if: idle|working`.
  `if: idle` is **not** draft-safe. Spike 2C showed that a typed draft is appended to and submitted, and that both
  agents report `idle` while a draft is present (04:80-84). hc's quiet rule then waits out a person's typing and
  submits at their first 3 s pause (input.rs:105-118, 155-200).

## Decisions (recommended)

1. **Seat = (core id, tN). Closure is keyed on the finish reason, not on keep.**
   - The seat maps to `(hc-core:<core id>, tN)`.
   - Keep recovery (a fresh shell on the same `tN`) keeps the seat.
   - `killed` (app `pane.close`, `hc kill`) retires the seat through the pane-close fence. So does `removed` while
     the seat is still resolved on that terminal (herdr-app window.rs:2774-2786; runtime.rs:656-704).
   - A finish from program exit, a signal, `server_stop` or `host_shutdown` makes the seat **unresolved**: its
     bindings end with `seat_unresolved`, and C1 resume of the same native session in a new terminal reattaches it.
     Otherwise the operator retires it.
   - "Later sessions in that terminal bind the same seat" holds only for shell-rooted terminals (app panes). For
     `hc run <agent>` the agent is the root program, so the terminal ends when the agent does.
   - `lost` and `restarted` leave the seat unchanged.
2. **Host identity is the core id; the socket is only a dial address.**
   - The boot/incarnation is `hc-core:<core.json id>`, read through `hello` on every attach.
   - If a known locator answers with a different core id, the old seats get C2 treatment (unresolved, holds) and are
     never matched by `tN`.
   - A known core id that answers at a new path is treated as a new host, and the old row's seats go unresolved.
     This is rare because the session dir is fixed (inferred).
   - Kept `tN` surviving a core stop and start is structural evidence for hc only. It is a C1 row amendment.
3. **Caller location: cooperative, with a consistency guard only when env is contradictory.**
   - A single host family in env is accepted unverified, as A1 accepts today.
   - Only when env names targets on two families (`HERDR_PANE_ID` and `HC_TERMINAL_ID`) does the daemon pick the
     candidate whose host-reported root pid is the nearest ancestor of the caller's fingerprinted process. When
     none matches, the call is refused and never falls back.
   - The guard corrects stale env. It is not attestation, and no binding or receipt records "verified".
   - Accepted limits: a double-forked or setsid descendant cannot be checked (04 2A(3)), and neither can Codex
     without `--no-daemon` (04 step 6).
   - **Inherited seat name:** if `HERDR_THREADS_SEAT_NAME` is present beside a host candidate and the host server
     process (the hc `serve` process, or the Herdr server) carries the same value, the name came from the host and
     the call is refused with one line. This only refuses and moves nothing. Proposal Part 5, D1.
4. **Wake: none on hc until a draft-safe primitive exists.** **[decide: D6]**
   - In v1, hc terminals have `WakeCapability::None`. This is the degradation named seats have: `wake=no_wake` is
     reported once, and Claude with the mod keeps Channel wake.
   - Composer wake is enabled only when `hello.features`/`rpc.discover` reports an upstream draft-safe send, for
     example `send{if_input_empty}` or `quiet:"refuse"` checked atomically inside the core. The ask also includes
     `if:[idle,done]` (decision 4b).
   - A screen pre-read followed by `send` is rejected: hc's quiet rule submits a draft that a person types after
     our read, and the plain-text read cannot tell Claude's prompt suggestion from a draft (01 R5; TRUST-POLICY
     styled-read rule). This is also why we never fall back to a plain `send`.
   - 4b. **Outcome table** for when the wake ships:

     | Status | Outcome |
     |---|---|
     | `working` | no block; retry on the next `status`→idle event |
     | `blocked` | `WakeBlock::Blocked`, overdue warning only |
     | starting or `lost` | retry after `restarted` |
     | never reported (`started:null`) | lasting `no_wake` once per (sender, seat) |
     | `done` | wake-ready only if upstream accepts `if:[idle,done]` (spike W2); otherwise a recorded `StatusDone` block, which is an accepted limit |

     The wake budget is at least quiet + settle + the started window (about 10 s; input.rs:65-72). A transport
     timeout after the request was sent counts as "submitted, unknown": it enters `AwaitingReceipt` and is never
     retyped. `started:false` is a candidate for `unreceived`.
5. **Events are dirty hints and closure comes from a coherent capture.**
   - `status`, `turn`, `finished`, `removed` and `missed` trigger a `hello` plus a one-shot snapshot of the same
     core id, and decisions are made from that capture.
   - Addresses containing `/` are remote mirrors and are ignored (remote.rs:208-211, 440-464).
   - Binding liveness stays on the OS fingerprint (A3). Host-reported liveness would need a new provenance value.
6. **Labels from the app, unambiguous across cores.**
   - `herdr.space`/`herdr.tab` labels and the `herdr-app` doc supply the WORKSPACE/TAB columns.
   - Labels are keyed by `(host_id, target)`.
   - A bare hc terminal is displayed and selected as `<HC_SESSION>:tN`, or as bare `tN` only when exactly one core
     is attached. A bare `t1` that matches several cores is refused with the list of candidates.
   - Topology served to the RemoteResolver includes the session name.
7. **Handoff/launch into hc: phase 2 (H3).** **[decide: D7]** When it lands:
   - `terminal.run` with `no_parent: true` (or `outlive_parent` plus `no_notify`) and `caller` omitted
     (api/run.rs:109-139);
   - the prompt is delivered through herdr-threads, never hc's `first_message`;
   - leaked `HERDR_*`, `HERDR_THREADS_SEAT_NAME`, `CLAUDE*`, `CLAUDECODE` and `AISW*` are overwritten with empty
     values or run through the gate shim;
   - per-harness argv such as Codex `--no-daemon` comes from a `HarnessAdapter` launch policy with a neutral default;
   - remote launch uses a cap-gated `LaunchTarget` variant.
8. **Remote through ht peers, not hc machine links.** The local daemon attaches only local cores.
9. **App mod panel: later.**
10. **Versionless.** Capabilities are probed (`hello.features`, `rpc.discover`, reported status) and only the
    missing one degrades.
11. **Coexist with hc's own agent messaging.** Our skill states the split once. Because decision 7 uses
    `no_parent`/`no_notify`, our own launches never produce hc notes. Only children that the user makes with hc
    produce notes, and that is the user's configuration. **[decide: D8]**
12. **Guard the TUI path now (phase F).** When env carries both `HERDR_PANE_ID` and `HC_TERMINAL_ID`, the Herdr
    candidate is checked by nearest ancestor (decision 3) before binding. Otherwise the call is refused with
    "runs in herdr-core terminal tN; not supported yet".
13. **TUI may later run on herdr-core** (herdr-app#63). `NativeCli` and `HcHost` stay independent behind
    `HostPort`.
14. **Attach lazily and keep the daemon able to idle-exit.**
    - A core is attached when a claim names its socket, and remembered rows that still hold local seats are
      re-attached.
    - The scan of `$XDG_STATE_HOME/herdr-core/*/` is for diagnostics and `hello` liveness only, and it never runs
      `hc` (`hc server status` autostarts a core).
    - An hc host with no resolved seat does not count as idle-exit activity and does not count against
      `MAX_ATTACHED_HOSTS` (idle_exit.rs:37-42; host_sessions.rs:42).

## Trust policy (same commit as the code)

These amendments to TRUST-POLICY.md land in the same commit as the code they cover:

- **Vocabulary.** Boot/incarnation is defined per host kind; for hc it is the core id.
- **C1.** The structural row covers kept `tN` across a core stop and start, for hc only.
- **Pane-close retirement.** The hc wording covers the `killed` and `removed` finish reasons.
- **A1.** The two-family consistency guard and the inherited-name refusal are added.
- **A3.** `managed_launch` is either extended to "host-guarded launch observed by its host" (the gate shim and the
  terminal's root pid fingerprint), or hc launches record no binding until check-in.
- **C4.** Bootstrap staging gains the hc launch.
- **Accepted limits:**
  - inherited core env;
  - unverifiable detached descendants and Codex daemon mode;
  - no wake on hc until the upstream primitive exists;
  - `done` not being wake-ready.

## Testing

- **Fake core.** `test_support::fake_hc` is a JSON-RPC line server implementing `hello`,
  `terminal.get/list/read/send` and events with `missed` and heartbeat.
- **Isolated cores.** An isolated-core helper, analogous to `scripts/lib/isolated-herdr.sh`, gives each test a
  private `HC_SESSION` and `XDG_STATE_HOME` and passes USER, LOGNAME, SHELL and TMPDIR. It never calls
  `hc server status`.
- **Leak checks.** `check-no-leaked-processes` learns to recognise `herdr-core serve` and `hold`.
- **Wake e2e.** Opt-in wake-e2e cases run under `--host hc`. In v1 they cover `no_wake` once, Channel wake with
  the mod, and leaked-`HERDR_PANE_ID` and leaked-name precedence. Once the upstream primitive exists, they add
  send-p0, silent send-p1, draft refusal and the status after a turn.

## Out of scope (v1)

- App mod UI.
- Attaching remote cores directly.
- Replacing hc's own agent messaging.
- Windows-specific paths.
- hc composer wake (decision 4).
- hc handoff (decision 7, phase H3).
