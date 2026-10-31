# Environments proposal: named seats and herdr-core/herdr-app behind one host seam

Status: for user review. Bead: ht-kqrt (named seats). Tree: `.worktrees/external-seat` at badccf481.
Sources: the run files in the Appendix. The roasted and revised drafts are `05-herdr-app-vision.md` and
`07-unified-code-design.md`. The approved named-seats spec is not edited; Part 1 lists the amendments it needs.
"(inferred)" marks a claim not traced end to end.

## Goal

An agent session gets a durable seat in any of three environments, through the same CLI, hooks, mod channel,
daemon, store and peer wire:

1. a Herdr TUI pane (today);
2. a plain process with `HERDR_THREADS_SEAT_NAME=<name>` (no Herdr, beside Herdr, headless, or remote);
3. a herdr-core terminal (`hc run`, or a pane of the Herdr desktop app).

In every environment the agent can join, send, read its inbox, ACK, get turn-hook or channel delivery, and message
seats across environments and linked nodes. Idle wake works as far as the host allows.

## Summary

- **One seam.** Each environment is a host behind the existing `HostPort`:
  - `NativeCli` for Herdr;
  - a built-in, never-dialled `NamedHost`;
  - a dialled `HcHost`.

  Core reads capability answers (`wake_capability`, `closure_authority`, `archival_evidence`,
  `native_launch_capability`) and does not branch on the environment for policy. The few places that still name an
  environment are listed in Part 3, "Where core still knows its environment". `CallerClaim` and the peer wire are
  unchanged. Harness adapters change only where they own env: the Hermes bridge passes the new variables (N/H1),
  and H3 adds neutral-default launch methods. There is one store migration.
- **Phases.**
  - **F**: the shared foundation, with Herdr-only behaviour. It fixes cross-core label mixing (K4: the CLI joins
    labels by target only, src/cli/human.rs:52-61, so two hosts' `t1` cross) and the daemon env scrub (K5: a
    daemon autostarted inside an hc terminal inherits `HC_SCOPE` and is swept when that terminal ends;
    `spawn_detached` today removes only `HERDR_BIN_PATH`, src/daemon/lifecycle.rs:385-389), and adds the TUI leak
    guard.
  - **N**: named seats.
  - **H1**: hc observation and messaging, with no wake.
  - **H2**: hc composer wake, blocked on upstream.
  - **H3**: hc launch and handoff.

  N and H1 are independent after F. N merges first.
- **What the roast changed.** 58 of 64 non-nit findings survived adversarial refutation (6 refuted), and every one is resolved in the drafts or listed as a
  spec amendment:
  1. **Inherited seat name.** The name leaks through the hc core env, and through any server started from a named
     shell, which would fold unrelated agents into one seat. The fix: refuse a name that the host server process
     also carries, make second-locator bindings visible, and scrub the variable from everything herdr-threads
     launches.
  2. **Named host never closes.** Its enumeration publishes targets but cannot retire or unresolve them. Named
     continuity and restore become explicit TRUST-POLICY C1 rules.
  3. **hc wake is off until herdr-core ships a draft-safe send.** `hc send --if idle` submits typed drafts, and hc's
     quiet rule waits out a typing person and then submits their draft. A screen pre-read cannot close that gap.
  4. **hc closure is keyed on the finish reason.** Agent exit makes the seat unresolved, so resume reattaches it.
     A pane close retires it.
  5. **Delivery contract, stated honestly.** Seats with no wake break rules 5 and 6, not rule 2. Their no-wake
     warnings are persisted, so a sender sees each once.
  6. **Mechanics.** The kind probe runs before the host row is written. G4 now means "this build has a
     constructor". The capability answers are separated, and the local protocol gets a skew gate.
- **The user's decisions** are in Part 5. The ones that matter most are D1 (inherited name), D3 (read-only
  bindings), D6 (hc wake) and D10 (hc closure).

## Part 0. Research and spike findings (01, 04)

Trees read: herdr-core `dc276b7` (0.1.15), herdr-app `0c1a102` (0.1.10), Herdr TUI `ee5bfb50` (0.9.5) (01:5-9).
Spikes ran on a real install on the user's macOS laptop (hc 0.1.15, herdr-app 0.1.10, Claude Code 2.1.316,
Codex 0.165) in isolated cores; where a spike and the research disagree, the spike wins (04; 07:3).

### Plugin system: Herdr TUI vs herdr-core / herdr-app

| | Herdr TUI | herdr-core / herdr-app |
|---|---|---|
| Extension model | `herdr-plugin.toml`: build, `[[startup]]`, actions, events, overlay panes, link handlers (01:78) | None in herdr-core. herdr-app has "mods": trusted QuickJS/GPUI `main.js` + `mod.json`, dropped into `~/.config/herdr-app/mods/<name>/`, no manifest, installer, startup hook or event subscription; `exec` capped at 30 s / 8 MiB (01:24, 01:78; 04:49-50) |
| Install | `herdr plugin link`, registry `~/.config/herdr/plugins.json` (01:79, 01:90) | Nothing to register. herdr-threads installs standalone (release archive + `setup`) |
| Daemon start | `[[startup]]` runs `view.sh ensure` at server start (01:91) | No equivalent. A mod cannot host a long-running process (30 s cap). herdr-threads' own lazy CLI/hook start is used (06 6.3) |
| State dir | `HERDR_PLUGIN_STATE_DIR` (01:94) | None; herdr-threads keeps its existing default dir (06 6.3, 07 4.4.1) |
| Operator view | overlay pane `operator` | later: a mod panel or a labelled hc terminal (05 D9) |

### Pane identification

| | Herdr TUI pane | herdr-core terminal / app pane |
|---|---|---|
| Env | `HERDR_ENV`, `HERDR_PANE_ID`, `HERDR_SOCKET_PATH`, ... set last, harness vars stripped (01:67) | `HC_TERMINAL_ID=tN`, `HC_TERMINAL_SOCKET`, `HC_SCOPE`, `HC_SESSION`; base env is the **core's**, frozen from whoever autostarted it, no stripping (01:67; 04:13, 04:16) |
| Id | `<ws>:p<n>`; changes on a cross-workspace move (01:69-70) | `tN`, never reused in a core's state; same across attach/detach, keep recovery, `server restart`, stop+start (04:14) |
| Host identity | server process (peer-pid witness, src/host/native.rs:104-130) | core id in `core.json`, read by `hello`; stable across restart and stop+start (04:92); not in env; a wiped or cloned core restarts at `t1` under a new id (01:28) |
| App pane | n/a | one app pane = one core terminal; the layout (pane `p…` ↔ `tN`) lives in core doc `herdr-app`; pane ids exist in no env or label (04:47-48, 04:102) |
| Topology | workspaces/tabs/panes | none in core ("a runtime, not a multiplexer"); app spaces/tabs are labels `herdr.space`/`herdr.tab` plus the doc (01:29, 01:72) |
| Status | screen detection only for Claude, Codex, Hermes, agy (OSC title, OSC 9;4 progress, screen regexes, PTY activity; exit reads Idle); their hooks report session identity only; no OSC 7501 support (TUI ee5bfb50) | self-reported OSC 7501 only (04:21); can report `blocked kind=auth`, null at the folder-trust dialog, `idle` with a draft (04:29, 04:84) |
| Input | raw type, keys, `agent.prompt` | `terminal.send` = paste + Enter always; `if: idle\|working`; 3 s human-quiet wait; no raw type (01:74; 04:20) |
| Multiple instances | one server per session | several cores per user at once; no RPC enumerates them; discover via `~/.local/state/herdr-core/*/core.json` + `hello` (04:88-89) |

### Spike results that shaped the design (04)

1. **OSC 7501 is native** in Claude Code and Codex (no hooks needed), which resolves 01's biggest unknown (01:30;
   04:31-32). Codex must run `--no-daemon` or its commands run outside hc (04:35).
2. **`hc send --if idle` submits typed drafts.** Both agents report `idle` with a draft; the wake was appended and
   submitted (04:82-83). This is the basis of D6.
3. **The core env leaks.** A core autostarted from a Herdr pane gave every terminal all seven `HERDR_*` vars and 14
   `CLAUDE*`/`AISW*` vars (04:16, 04:76-77). This is the basis of the TUI leak guard (F) and of D1.
4. **herdr-threads 2.8.0 has no seat in an hc terminal:** `inbox` refuses for want of `HERDR_PANE_ID`, and with a
   leaked pane id it would bind the foreign pane's seat (04:37-43).
5. **No proof for detached descendants:** a double-forked `setsid` process keeps only the inherited env (04:73). This
   bounds the nearest-ancestor guard (A1 accepted limits).
6. **Operational hazards for tests and discovery:** `hc server status` autostarts a core on macOS (04:9); a core
   started with only `HOME`/`PATH` leaves Claude logged out (04:85).

Pre-existing TUI defects found on the way (01 R8: cross-workspace `pane.move` breaks a bound seat, plugin actions get
the focused pane's `HERDR_PANE_ID`, live handoff reallocates `terminal_id`, Hermes managed launch always refuses
upstream) are out of scope here; F's env scrub covers R8b only. The rest are follow-up beads (not yet filed).

## Part 1. Vision: named seats

### Approved spec, summarized (specs/2026-10-31-named-seats-design.md)

- **Identity.** `HERDR_THREADS_SEAT_NAME` names a role. Concurrent sessions with the same name on one node are
  bindings of one seat and inherit its threads, mail and ACK obligations. Seats are never merged.
- **Lifecycle.** A seat is created automatically on first use, with one stderr line. It is retired only
  explicitly, by the operator or by its own agent (`seat retire --self`). A retired name used again mints a new
  seat.
- **Precedence and names.** The name wins over `HERDR_PANE_ID`. Names are per node (`name@node` remotely), 1–64
  bytes of `[A-Za-z0-9._-]`. An invalid name is a hard error.
- **Wake.** Named seats have no Native wake; Channel wake (Claude with the mod) is kept. Senders get a one-time
  `wake=no_wake`. Handoff into a named seat is refused.
- **Architecture.** A built-in named host has targets equal to names. It has a reserved locator, a fixed boot, a
  topology of the live named seats, no composer and no closure.
- **Data and wire.** There is a `seats.name` column. `seat.created.display_name` carries the name. Trust policy
  gains `cooperative_named_seat` and `cooperative_self_retire`.

### Proposed amendments (spec not edited; each needs approval)

| # | Amendment | Findings |
|---|---|---|
| SA1 | **Precedence narrows.** The name replaces only the caller's *target*, not instance discovery. Discovery order: flag, `HERDR_PLUGIN_STATE_DIR`, hook-baked `--state-dir`, today's discovery when Herdr env is present, the existing default dir; the default dir is created only when no Herdr is configured or on PATH (07 4.4.1). `doctor` flags two live stores. | N-T7, N-D6, N-A7 |
| SA2 | **Inherited-name refusal.** When a host candidate sits beside the name and the host's server process env carries the same value, refuse with one line (about 110 B). **Observability:** each named binding records a `locator_hint`; the first binding from a second distinct locator prints one line (about 60 B); `seat inspect` lists bindings by locator. **Scrub:** the variable is dropped from everything ht spawns. Setup and skill text say to set the name per command. The accepted limit is restated to cover inheritance through tmux, IDE or shell parents. | N-T1, N-D1, N-A1 |
| SA3 | **The named host never closes.** Its enumeration covers `host_id = named` and is `NonClosing`. Its rows are tagged `cooperative_named_seat`, never CoherentVerified. A store guard refuses `RetireReason::Closed` on a named host. The host has a fixed incarnation and boot, and S1/S2 are gates before approval. | N-T2, N-A2 |
| SA4 | **New C1 row, named continuity.** An ordinary resolve on the named host whose name equals an unresolved named seat's name, with no live seat of that name, reattaches it. **Restore:** a seat with `display_name` goes onto the named host, unresolved, at target = name, and is left for the operator if a live seat with that name exists. Others are placed only when the node's dialled hosts are all of one kind; otherwise they are held. The named host never satisfies `wait:host`. | N-T3, N-A5, U-T2 |
| SA5 | **Named host is outside `attached()`.** Consumers that opt in are listed (07 5.3): the `operator_host` fallback, RemoteResolver, the seat publisher and `wake_status`. A test asserts it is never in `attached()`. | N-A5 |
| SA6 | **Self-retire.** The wire carries `RetireReason::Operator` (closed enum, seat.rs:72-79), with `cooperative_self_retire` only in the local label. That is an accepted limit; a cap-gated `SelfRetired` is a follow-up. Self-retire is refused while another connected top-level binding exists, unless `--force`. | N-T4, N-A3, U-T4 |
| SA7 | **Role semantics.** ACK obligations belong to the role, and the first display discharges them. Parallel workers use distinct names (`ci-bot-$N`). An optional read-only binding (`HERDR_THREADS_SEAT_READONLY=1`, D3) reads and sends but never ACKs. A test runs two concurrent bindings, one of them headless. | N-T5, N-D4 |
| SA8 | **Use case 3.** "Cron loops" is reworded to "a named seat needs a harness session (`claude -p`, `codex exec`)". Named person seats are deferred. | N-T6 |
| SA9 | **Pane-form allocation** of a pane whose named binding hinted that pane is refused: `address it by name`. | N-T8 |
| SA10 | **Sender warnings.** No-wake reports for no-wake hosts are persisted per (observer, seat): once ever, across restarts and channel flaps. Overdue lines read `wake=no_wake`. | N-D2, U-D1 |
| SA11 | **Delivery-contract check rewritten.** Rules 5 and 6 are degraded: p0 ACKs, invitations and warnings arrive on the next turn, and nobody wakes the seat without a live mod. Pokes and deadline reminders are absent. `seat threshold` is inert and says so. AGENTS.md rules 5 and 6 are amended, as well as TRUST-POLICY. | N-D3, N-A4, U-D4 |
| SA12 | **Wake classification.** It is `idle_wake_on(harness, host capability, mod_suppressed)`. Hooks-only Claude classifies as `None`. Spike S3 (watch env inheritance, never `NoPane`) is a gate. | N-A4, U-D2 |
| SA13 | **Peer wire.** `SeatStatusEntry.name` is dropped (O1). `cooperative_named_seat` names the named-host observation; binding provenance stays the caller's. | N-A3 |
| SA14 | **Selectors.** A bare token is a name only when no pane-path reading matches. Auto-creation happens only on seat-acting calls. `HostTopologies` is listed as a local protocol change. | N-A6 |
| SA15 | **Locator.** The fixed reserved path `/.herdr-threads/named-host` (O2). | 07 O2 |
| SA16 | **Live-holder guard** (narrows the Identity decision; D19). When a named seat would gain or reconnect a top-level binding, the daemon checks the seat's other connected top-level bindings with `LivenessFingerprint::is_alive` (pid + start time). It refuses `seat_name_in_use` (`herdr-threads: seat <n> is held by live pid <p>; use another name`, about 60 B) when a holder is alive, differs from the caller and neither is in the other's parent chain. Allowed: the same process, its ancestors/descendants, subagents (agent_id, as in a pane), D3 read-only bindings, Codex forks and Hermes under their existing rules. A dead or reused pid frees the name at once. Needs a ProcessInfo port in the enroll transaction (new plumbing). Complements SA2 (sequential folds through the host) rather than replacing it; makes SA6's self-retire check and SA7's concurrency test mostly moot. Today's panes have no such guard: enrollment never ends or refuses another binding (TRUST-POLICY.md:456-457). | follow-up review |
| SA17 | **Named presence follows the pane precedent** (pane open, no agent). A named seat goes offline only when its bindings disconnect (`liveness_end_event`, ProcessExit, Superseded) and then shows exactly what a pane seat shows: "inactive, unavailable since T" in seat list, participants and inspect; `offline` to peers; one `recipient_unavailable` per episode for ACK and p0 sends; overdue lines per SA10. No new model-visible text. Fixes needed for that to hold: (b) the named host never arms `baseline_hold_unclaimed` or recovery holds (today one unresolved row on a host silences liveness for the whole host, seats.rs:1037-1047, 2195-2208); (c) the named host's reconcile pass stamps its fixed structural boot/incarnation so the C4 carry keeps live named bindings available across a daemon restart; (d) synthetic workspace/tab labels render `-`, the name once. Optional for all seats: `presence: offline` in overdue payloads (`wake=no_wake offline`, 8 B). | follow-up review |
| SA18 | **N2: tmux wake for named seats** (D20). Phase after N, independent of H1-H3. Caller location records `TMUX`/`TMUX_PANE` as hint `tmux:<socket>:<server_pid>:<pane>`; the daemon verifies it (`tmux -S <sock> display -p -t <pane> '#{pane_pid} #{pid}'` + nearest-ancestor walk) at bind and at every wake; a failed check only disables wake, never moves a seat. NamedHost reports `Composer` for bindings with a verified hint. Composer: `capture-pane -p` at `#{pane_width}` into the existing ComposerPolicy; focus = pane_active && window_active && session_attached. Typing: `send-keys -l` + Enter with the existing read-back/clear. Idle/busy (tmux drops OSC 7501, inferred): hook turn state UserPromptSubmit→Stop (Codex has Stop; Claude needs one) plus a harness-owned screen recognizer for blocked (new ComposerPolicy method, neutral `None` = no wake). tmux as a fourth host is rejected: pane ids are reused, no incarnation witness, competes with named identity. Spikes T1-T5 (Part 6). | follow-up review |

## Part 2. Vision: herdr-core / herdr-app (05, revised)

- **The host is herdr-core and the app is a view.**
  - Each app pane is a core terminal `tN`, and the layout lives in core doc `herdr-app`.
  - There is no plugin system, so herdr-threads installs standalone.
  - `HC_TERMINAL_ID` and `HC_TERMINAL_SOCKET` locate the caller.
  - The core env leaks, because terminals inherit the env frozen from whoever autostarted the core (04 step 3, 2B).
- **Decisions.** **[decide]** items point to Part 5.
  1. **Seat = (core id, tN), with closure by finish reason.**
     - `killed`, or `removed` of a resolved seat, retires it.
     - Exit, signal, `server_stop` and `host_shutdown` make it unresolved, and C1 resume reattaches it.
     - `lost` and `restarted` change nothing.
     - Only shell-rooted terminals give pane-like continuity across sessions. **[D10]**
  2. **Boot = `hc-core:<core id>` from `hello`.** The socket is only a dial address. A changed id at a known path
     puts the old seats under C2 holds.
  3. **Cooperative caller location.** A single host family is accepted unverified. When both families are present,
     the nearest-ancestor guard decides. An inherited seat name is refused (D1). Nothing records "verified".
  4. **No hc composer wake until upstream ships a draft-safe send.** **[D6]**
     - Detection is through `hello.features`.
     - When it lands, the outcome table applies (working, blocked, starting or lost, `started:null`, `done`). The
       budget is at least 10 s, and a timeout after send means "submitted, unknown".
  5. **Events are dirty hints.** Closure comes from a coherent capture of the same core id, and `machine/tN`
     mirrors are ignored.
  6. **Labels** come from the app's labels and doc. A terminal is shown and selected as `<HC_SESSION>:tN`, and an
     ambiguous bare `t1` is refused. **[D11]**
  7. **hc handoff in H3**, with `no_parent`/`no_notify`, the prompt through ht, leaked env blanked, per-harness argv
     from the adapter, and a cap-gated remote `LaunchTarget`. **[D7]**
  8. **Remote** goes through ht peers.
  9. **Mod panel** later.
  10. **Versionless.**
  11. **Coexist with hc notes.** Only user-made children produce notes. **[D8]**
  12. **TUI leak guard in F.**
  13. **NativeCli and HcHost stay independent.**
  14. **Lazy attach.** The scan is for diagnostics only and never runs `hc`. A core with no seats does not block
      idle exit. 05 D14 also says such a core does not count against `MAX_ATTACHED_HOSTS`; 07 5.3 keeps the cap
      unchanged for dialled hosts (host_sessions.rs:42, 16 hosts). The 07 ruling stands; 05's sentence is
      superseded (inferred harmless at 16).
- **Trust policy section.** It covers vocabulary per host kind, C1 kept `tN`, pane-close for `killed`/`removed`, the
  A1 guard, `managed_launch` extended to host-guarded launch, C4 staging, and the accepted limits.
- **Testing.** The plan uses `fake_hc`, an isolated-core helper (private `HC_SESSION` and `XDG_STATE_HOME`, USER,
  LOGNAME, SHELL and TMPDIR passed, never `hc server status`), leak-check recognition of `herdr-core serve`/`hold`,
  and opt-in `--host hc` wake-e2e cases.

## Part 3. Unified code design (07, condensed)

### Environment model

A **host** is one attached environment server, not a daemon: a Herdr TUI server, one herdr-core core, or the built-in named host. One daemon per node (machine, OS user, state dir) attaches all of them (up to 16 dialled hosts plus the built-in one), so seats in every environment share one store and message each other directly. Seat identity cannot collide across hosts: seat IDs are node-minted, and a target is scoped by its host row (pane `w3:p1`, terminal `t1` and name `t1` are different seats). Only a typed selector can be ambiguous across environments, and it is refused as today.


| | Herdr TUI | Named | herdr-core |
|---|---|---|---|
| Host | dialled socket | built-in, in-process | dialled JSON-RPC |
| Target | pane id | name | `tN` |
| Boot | server process | fixed per node | core id |
| Caller env | `HERDR_ENV`, `HERDR_PANE_ID`, `HERDR_SOCKET_PATH` | `HERDR_THREADS_SEAT_NAME` | `HC_TERMINAL_ID`, `HC_TERMINAL_SOCKET` |
| Wake | composer | none (Channel with live mod) | none until upstream; then composer |
| Closure | absence from complete snapshot | never (explicit retire) | finish reason (above) |
| Launch | in place + new tab | refused | new terminal (H3); in place refused |

Seats, bindings, `CallerClaim{host,target}`, the cooperative mapping, the OS liveness fingerprint, reconcile,
RemoteResolver, receipts, delivery and the peer wire are shared and unchanged.

### Shared abstractions

```rust
// src/ports.rs — HostPort gains defaulted methods only; the 40 existing impls compile unchanged.
pub enum HostKind { Herdr, Named /* N */, Hc /* H1 */, Unknown }   // G4: Unknown = no constructor in this build
pub enum WakeCapability { None, Composer(ComposerTraits) }
pub struct ComposerTraits { pub submit_budget: Duration }           // H2 adds focus_known, stash
pub enum ArchivalEvidence { Supported, NoEvidence }
pub enum ClosureAuthority { Closing, NonClosing }
trait HostPort {
    fn kind(&self) -> HostKind { HostKind::Herdr }
    fn wake_capability(&self) -> WakeCapability { WakeCapability::Composer(ComposerTraits::HERDR_TUI) }
    fn target_root_process(&self, t: &HostTargetId, cx: &HostCallContext) -> Result<Option<u32>, ApiError> { Ok(None) }
    fn server_process(&self) -> Option<u32> { None }               // inherited-name check (E1)
    fn archival_evidence(&self) -> ArchivalEvidence { ArchivalEvidence::Supported }
    fn closure_authority(&self) -> ClosureAuthority { ClosureAuthority::Closing }
}

// src/service/host_sessions.rs
pub struct HostSessions { sessions: RwLock<BTreeMap<HostId, Arc<HostSession>>>, builtin: OnceLock<Arc<HostSession>>, .. }
// attach_builtin at daemon start; all_for_reads() only for the audited readers; wake lane started iff capability != None.
// H1: factory = probe(locator, budget) -> HostKind (hello vs ping), run BEFORE store.attach_host, then build(kind, locator).

// src/harness/registry.rs — has_idle_wake stays as the oracle
pub fn idle_wake_on(&self, harness: &str, host: WakeCapability, channel_live /* mod_suppressed */: bool) -> Option<IdleWakeOn>;

// src/cli/caller_location.rs — the only reader of caller-location env
pub enum CallerLocator { Pane(HostTargetId), Named(SeatName), Terminal { socket: PathBuf, id: HostTargetId } }
pub enum Located { One(CallerLocator), Ambiguous(Vec<CallerLocator>) }
impl CallerSite { fn env_assignments(&self) -> Vec<String>; fn host_scoped_lookup(&self) -> bool; /* ... */ }
```

**Precedence.**

1. **A valid name replaces the target.** Discovery is unchanged (SA1). The daemon refuses a name that the host's
   `server_process` env also carries (SA2).
2. **One family** is accepted unverified (A1).
3. **Both families** go to `LocateCaller`, where the nearest-ancestor root pid wins.
4. **Neither verifies:** the call is refused.
5. **Phase F guard:** only the Herdr candidate is checked.

### Per-environment answers

| Answer | Herdr | Named | hc |
|---|---|---|---|
| `wake_capability` | Composer(2 s) | None | None; Composer(15 s) once upstream feature detected |
| `closure_authority` | Closing | NonClosing | Closing (finish reason) |
| `archival_evidence` | Supported | NoEvidence | NoEvidence |
| `native_launch_capability` (in place) | unchanged | Unsupported | Unsupported (creation = `CreateTargetPort`, H3) |
| `target_root_process` | `shell_pid` | None | `terminal.get.pid` |
| `server_process` | socket peer pid; the incarnation witness already records it (src/host/native.rs:104-130) | None | `serve` peer pid (inferred) |
| labels | unchanged | pane = name | `herdr.space/tab` + doc; `<session>:tN` |

The audit of node-wide `attached()` consumers (07 5.3) names each consumer and whether it includes or skips each
host:

- idle exit counts dialled hosts that hold a resolved seat;
- archival filters on `archival_evidence`;
- the bootstrap runtime is gated by the in-place launch capability;
- `wake_status`, RemoteResolver and the seat publisher go through `all_for_reads`;
- the discovery gate considers Herdr only.

### API changes

| Area | Changes |
|---|---|
| CLI | `seat retire --self`. New lines, all sized under rule (f): creation (45 B + name), named launch refusal (45 B), inherited-name refusal (110 B), second-locator notice (60 B), pane-form refusal (50 B), inert threshold (60 B), old-daemon gate (60 B). The hook guard line (110 B) rides lifecycle check-ins only, and Hermes gets it on its first delivering hook. Handoff recovery hints are built through `env_assignments`. |
| Local protocol | `SeatsQuery.host`, sent only for named and terminal sites. `Command::HostTopologies`, `SeatDirectory.name`, `SelfRetire` and `LocateCaller`. Capability `seat.host_sites_v1`. `WakeBlock::{NoStatus, StatusDone}` (H2). `CallerClaim` is unchanged. |
| Daemon | Env scrub builds on `lifecycle::scrub_env(command, names)` + `host::PANE_SCOPED_ENV_VARS` from fix/tui-defects 5e19e9bce (ht-tod8, R8b); F adds `host::hc::ENV_VARS` and the seat name. `attach_builtin`, the capability-gated wake lane, the probe before the host row is written, persisted no-wake reports, and an env scrub of every host's `ENV_VARS` plus `HC_SESSION`. |

### Schema and wire

- **Schema: one NodeDelta.**
  - `herdr_hosts.kind`, plus `kind_probed_at`.
  - `seats.name`, with a unique index over live local rows, including unresolved ones.
  - `occupant_bindings.locator_hint`, and `readonly` if D3 is accepted.
  - The table `no_wake_reports(observer, seat_id)`.
  - Only the kinds this build implements are attachable.
- **Peer wire: no change.**
  - `seat.created.display_name` (already baselined) carries the name, and an invalid value is stored as NULL.
  - `SeatStatusEntry.name` is dropped. There is no new frame or capability in v1.
  - A future `LaunchTarget::NewTerminal` carries `// cap:`.

### TRUST-POLICY changes (same commit as the code)

1. **Vocabulary.** Host boot and incarnation are defined per kind. A changed hc core id is a new boot.
2. **A3.** `cooperative_named_seat` applies to the named observation. `cooperative_self_retire` is a local label;
   the wire reads Operator, which is an accepted limit.
3. **Named rule.** A named claim is not host evidence, and named seats never close by absence. New C1 row: named
   continuity. The restore rule is SA4. Role ACK semantics are SA7.
4. **A1 addendum.**
   - The two-family nearest-ancestor guard.
   - The inherited-name refusal, as cooperative consistency, not anti-spoofing.
   - Accepted limits: single-family stale env; unverifiable double-forked processes and Codex daemon mode; a name
     inherited through a non-host parent, which is only observable; same-user use of the variable; typo seats.
5. **Closure (hc).** Pane-close retirement covers `killed` and `removed` of a resolved seat. Exit unresolves the
   seat. The C1 structural row covers kept `tN` across a core stop and start. Accepted limit: a wiped or cloned
   core leads to C2 holds.
6. **A4 wake.** Named seats and hc seats have no composer wake. Rules 5 and 6 are degraded, pokes and reminders
   are absent, and the threshold is inert. hc H2 conditions: process-group target, self-reported status, no focus
   signal, and `done` not wake-ready.
7. **Launch (H3).** `managed_launch` is extended to host-guarded launch, and C4 staging covers the hc launch.
8. **AGENTS.md.** Delivery-contract rules 5 and 6 gain the exception for hosts with no wake.

No adversarial verification is added. Every decision is made in the daemon against the canonical view (A2).

### Install, discovery and daemon start without a plugin manifest

Named seats and hc terminals have no `[[startup]]` hook and no `HERDR_PLUGIN_STATE_DIR` (Part 0), so:

- **State dir.** One dir for every environment, the existing `$XDG_STATE_HOME/herdr/plugins/herdr-threads`, so a
  TUI plugin, hc terminals and named seats on one machine share one node (06 6.3). Discovery order with a name is
  SA1 (07 4.4.1). `ensure_default_state_dir` creates it 0700 for Named and Terminal sites only when no Herdr is
  configured or on PATH.
- **Install.** `scripts/install.sh` today skips harness setup unless the plugin is registered with Herdr
  (scripts/install.sh:909-911). It changes to run `setup` whenever it can, keep `herdr plugin link` only when
  `herdr` is present, and write a standalone marker `<state>/node/install.json`, so the leftover check
  (src/cli/instance.rs:271-290) does not read a standalone install as a leftover (06 6.3). `setup` bakes
  `--state-dir`, and no TUI `--host-endpoint` on hc-only machines.
- **Daemon start.** The existing lazy CLI/hook start, with the K5 scrub. No daemon in an `hc run --keep` terminal:
  it would inherit `HC_TERMINAL_ID`, and keep restores a shell, not the daemon (06 6.3).
- **Core discovery** (`HcDiscovery`, H1): list `herdr-core/*/core.json`, derive the socket by the core's rules,
  `hello`, require the core id to match, dedupe by id. It never runs `hc` (`hc server status` autostarts a core,
  04:9) and is used only when no caller site names a core (06 V7).
- **Skill.** One environment-neutral sentence plus the hc/ht split, about 160 bytes once per session (06 6.3).

### Where core still knows its environment

The goal: herdr-threads' general logic (message routing, delivery, receipts, seat bindings, liveness, peer sync) is unaware of the environment the recipient or caller runs in. That holds: none of the sites below is in routing, delivery, receipts or binding logic. Environment knowledge lives in host backends, the one caller-location module, and these bounded spots. These are the remaining sites in core
(AGENTS.md's core includes `src/cli`, `src/store`, `src/daemon`), each with why it stays:

| Site | What it knows | Why it stays / what bounds it |
|---|---|---|
| `herdr_hosts.kind` + cross-kind rebind refusal (store) | host family | trust rule (a named seat never rebinds onto a pane and vice versa); persisted, never read for policy (07 3.1) |
| peer-discovery gate `any_reachable` (src/app.rs:2277-2283, host_sessions.rs:500) | `kind()==Herdr` | it feeds `herdr machine list`, which is Herdr's own peer lister; hc remote goes through ht peers (Part 2 item 8) |
| store literal `'named'` | the built-in host | the unique index `herdr_hosts_one_named` and the restore/rebind rules only (07 7) |
| `CallerLocator {Pane, Named, Terminal}` in `src/cli/caller_location.rs` | the three env shapes | the only env reader; its per-variant answers (`moved_target`, `relative_pane`, `person_origin_allowed`, `enrolls_on_first_use`, `attempts_continuity`, `host_scoped_lookup`, `env_assignments`) are a closed match in one module (07 4.4). A `dyn CallerSite` was rejected for three variants (07 1) |
| daemon spawn scrub (src/daemon/lifecycle.rs) | union of each host's `ENV_VARS`, plus a literal `HC_SESSION` | env names come from the host modules (G3); `HC_SESSION` is the one literal (inferred: it could move into `host::hc::ENV_VARS`) |
| instance discovery step 4 (src/cli/instance.rs:57-81) | `HERDR_ENV`/`HERDR_SESSION`/`HERDR_CONFIG_PATH` | pre-existing; the state dir is still Herdr-plugin-rooted |
| `ParticipantLocation.remote`, `SeatHostLabels`, `HostTopology` | Herdr-shaped workspace/tab/pane | frozen shapes; named and hc fill them with reserved synthetic ids (07 4.5; 03 4) |

No mechanical check enforces this list; `scripts/check-host-neutral` is a follow-up (G6, D18 O8).

### Delivery-contract and harness-neutral checks

- **Rules 1–4.** Rules 1–3 are unchanged in every environment. Rule 4 holds, because a seat with no wake is never
  nudged.
- **Rules 5 and 6.** Named seats and hc seats until H2 degrade rules 5 and 6. That degradation is reported once
  per sender and seat, and documented in AGENTS.md, TRUST-POLICY and one SKILL.md sentence (about 90 B, once per
  session).
- **Tripwire.** A test fails if any host returns `None` while SKILL.md lacks `wake=no_wake`.
- **Harness neutrality.** No harness id enters core. Per-harness inputs are existing adapter methods, or neutral
  defaults added in H3 (`child_scrub_vars`, `host_launch_argv`). The Hermes bridge `PANE_ENV` must pass the new
  variables (adapter-owned; inferred). `check-harness-neutral` needs no allowlist entry (inferred).

### Change inventory and phasing

| Phase | Main modules | Size |
|---|---|---|
| F | ports, host_sessions, app, registry, dispatch/archival/peer_launcher, store and the NodeDelta, seat_publisher/remote_resolver, `cli/caller_location.rs`, cli mod/hook/watch (L, TRUST-POLICY-sensitive), human/peer_locations/follow/seat_resolution, instance/setup, `locate.rs` (Herdr candidate only), lifecycle scrub | about 4–5k lines (inferred) |
| N | `host/named.rs`, seats.rs (name, continuity, restore, self-retire, the Closed guard), inherited-name check, locator hints, pane-form refusal, `display_name` sync, recovery hints | M–L |
| H1 | `host/hc/*`, probe, full LocateCaller, lazy attach, finish-reason closure, labels, install/setup, `fake_hc`, isolated-core helper | L |
| H2 | the hc wake behind the upstream feature: outcome table, budget, `process_group` | M |
| H3 | `CreateTargetPort`, V3 journal, launch gate, `host_client` | L |

**Exit gates.**

- **F:** the acceptance criteria (07 2), the full suite, the leak check, and the Herdr wake e2e. The hook.rs
  refactor must show that nothing observable changed.
- **N:** S1–S6 (Part 6 gates all six on N) and E1 for the Herdr server, since N ships the inherited-name refusal.
- **H1:** D1, E1 for `herdr-core serve`, and H-C1.
- **H2:** the upstream feature exists, plus W1–W5.
- **H3:** L1–L4.

## Part 4. Roast results

Nine critics (three lenses on each of the two visions and the unified design) raised 71 findings. The 64 that were not nits each went to two skeptics, one checking the facts against the source and spikes, one checking scope and prior decisions. 58 survived and 6 were refuted. Doc keys:

- N = vision-named (the spec);
- H = vision-hc (05);
- U = unified-design (07).

Lens keys: T = trust-identity, D = delivery-wake, A = architecture. "Spec" means a proposed amendment from Part 1;
05 and 07 were edited in place.

| ID | Sev | Finding | Resolution |
|---|---|---|---|
| N-T1 | B | Inherited name sends every child terminal to one seat | SA2; 07 4.4 inherited-name refusal + locator hints + scrub |
| N-T2 | B | Named observation becomes verified evidence, retires by absence | SA3; 07 4.1/4.7 `NonClosing`, `host_id`-keyed enumeration, Closed guard |
| N-T3 | B | Name continuity and restore missing from C1 | SA4; 07 5.2, 9.3 C1 row + restore rules |
| N-T4 | S | Self-retire replicates as operator; one session retires all | SA6; 07 5.2 refusal unless `--force`; 9.4 limit |
| N-T5 | S | Short-lived sessions absorb ACKs on display | SA7; D3 read-only bindings |
| N-T6 | S | Cron loops have no attribution path | SA8; D9 |
| N-T7 | S | Name ignores pane's command group (wrong daemon) | SA1; 07 4.4.1 |
| N-T8 | S | Addressing a named agent's pane mints a dead seat | SA9; 07 4.4 pane-form refusal |
| N-D1 | B | Leaked name takes over panes and hc terminals | SA2 (as N-T1) |
| N-D2 | S | `wake=no_wake` re-arms on restart and channel flaps | SA10; 07 4.3 persisted reports |
| N-D3 | S | Contract check omits rule 5 and pokes | SA11; 07 10 |
| N-D4 | S | Concurrent sessions fan every item into every context | SA7; test; distinct names |
| N-D5 | B | hc pre-read fallback cannot be built safely | 05 D4, 07 10.3: no hc wake until upstream |
| N-D6 | S | Beside-Herdr discovery splits stores | SA1 |
| N-A1 | B | Name-wins plus env inheritance merges agents | SA2 (server-env refusal chosen over a root-marker launcher, D1) |
| N-A2 | B | Named port contradicts snapshot/incarnation invariants | SA3; S1/S2 as gates |
| N-A3 | S | Wire/provenance statements break wire-compat and adapters | SA13, SA6 |
| N-A4 | S | Wake classification undefined; breach documented in wrong place | SA11, SA12; AGENTS.md amendment |
| N-A5 | S | "Keeps current code paths" false for host-set consumers | SA4, SA5; 07 5.3 |
| N-A6 | S | Auto-created names break others' selectors | SA14; 07 4.5; D4 |
| N-A7 | S | State-dir discovery can split stores | SA1 |
| H-T1 | B | Closure retires on agent exit, leaves pane close unretired | 05 D1, 07 4.7 finish-reason table; D10 |
| H-T2 | B | Host identity must be core id, not socket | 05 D2; 07 4.2 probe reads core id; 9.1 |
| H-T3 | B | Seat name leaks through core env | 05 D3, 07 4.4 refusal; H3 scrub |
| H-T4 | B | "Verified for every caller" is adversarial | 05 D3 rewritten: two-family guard only |
| H-T5 | S | `lost` as closure; host events as liveness | 05 D5; 07 4.7 |
| H-T6 | S | No TRUST-POLICY section | 05 "Trust policy" section added |
| H-T7 | S | Bare `tN` not unique per node | 05 D6, 07 4.5 `<session>:tN`; D11 |
| H-D1 | B | Decision 4 ignores 2C; screen check races | 05 D4, 07 10.3 (D6) |
| H-D2 | B | `terminal.run` parents worker, types notes, steers to hc | 05 D7, 07 4.6 `no_parent`/`no_notify`, prompt via ht |
| H-D3 | S | `if: idle` excludes `done` | 05 D4 table; upstream ask `if:[idle,done]`; W2 |
| H-D4 | S | No-status degrade has no warning design | 05 D4 outcome table |
| H-D5 | S | Send can outlast wake budget | 05 D4, 07 10.3: ≥10 s budget, timeout = submitted-unknown |
| H-A1 | B | `send --if idle` not race-free | as H-D1 |
| H-A2 | B | `done` agents never woken | as H-D3; spike W2 |
| H-A3 | B | No host-kind discriminator | 07 4.2 probe before attach; `kind` column; D1 spike |
| H-A4 | S | Seat key vs endpoint row; closure vs hc lifecycle | 05 D1/D2; 07 4.7, 9.1 |
| H-A5 | S | Ancestry verification conflicts with policy | 05 D3; accepted limits; spike H-C1 |
| H-A6 | S | hc handoff inherits env, parent notes, argv, wire | 05 D7 moved to H3 with all four fixes (D7) |
| H-A7 | S | Discovery scan pins daemon, wastes host budget | 05 D14; 07 5.3 idle-exit row |
| H-A8 | S | No testing plan | 05 "Testing" section |
| U-T1 | B | Name precedence plus frozen core env folds agents | 07 4.4 (server-env check, not root env, so a per-terminal name still works) |
| U-T2 | B | Restore sends hc seats to TUI host | 07 5.2 restore rule; spike S5 extended |
| U-T3 | S | Recovery hints rebuild site as `HERDR_PANE_ID=` | 07 4.4.1 `env_assignments` |
| U-T4 | S | Self-retire recorded as operator, no limit listed | 07 9.4 limit; inspect/doctor show label |
| U-T5 | S | Named closure path can retire as "closed" | 07 4.7 `NonClosing`; Closed guard |
| U-D1 | S | No-repeat warnings false (memory, idle exit) | 07 4.3 persisted reports |
| U-D2 | S | `has_live` ignores stall | 07 4.3 uses `mod_suppressed` |
| U-D3 | S | `no_wake` reachable with no skill text; tripwire lost | 07 10 skill sentence + tripwire |
| U-D4 | S | Contract table blames rule 2, not 5/6 | 07 10, 9.6, 9.8 |
| U-D5 | S | Leak notice prints every command; guard routing undefined | 07 4.4 daemon-remembered notice; 6.1 routing |
| U-D6 | S | hc readiness narrows to idle; `done` unrecorded | 07 9.6 limit; upstream ask; W2 |
| U-A1 | B | Quiet rule submits draft after pre-read | 07 10.3 no hc wake until upstream (D6) |
| U-A2 | B | `HostGuardedStart` makes "unreachable" Errs reachable | 07 4.1/4.6: in-place `Unsupported`, `archival_evidence`, audit row fixed |
| U-A3 | S | G4 protects nothing | 07 1 G4 redefined; variants per phase |
| U-A4 | S | Kind sniff after host row write | 07 4.2 probe before `attach_host`, `kind_probed_at` |
| U-A5 | S | Old daemon gives parse/"not reachable" errors | 07 6.2 `seat.host_sites_v1` |
| U-A6 | S | No-regression gate compares against deleted fn | 07 2, 4.3: `has_idle_wake` kept as oracle |

Partially adopted:

- **H-T2.** "A known core id at a new path is the same host" is not implemented. It is treated as a new host,
  because session dirs are fixed (inferred rare).
- **N-D2.** Classifying a named Claude seat by whether the mod is installed, rather than by a live channel, is
  replaced by persisting the report once ever.

## Part 5. Decisions needed from the user

1. **D1 Inherited seat name.** The daemon refuses a `HERDR_THREADS_SEAT_NAME` that the host server process env also
   carries. Inheritance through non-host parents is only made observable (second-locator notice, `seat inspect`),
   and ht scrubs the variable from what it spawns.
   - Alternatives: a scope stamp (`HERDR_THREADS_SEAT_NAME_SCOPE`), or a `herdr-threads run --name` launcher with a
     root marker. Both put the burden on users.
   - Limits of the recommended rule: a user who deliberately sets the same value per command inside a server that
     already carries it is refused too (indistinguishable from inheritance); and where the server's pid or environ
     cannot be read (E1 fails on a platform), `server_process()` is `None` and the check is skipped, leaving
     observability only (07 14 risk 11).
   - **Recommend: server-env refusal plus observability.**
2. **D2 Precedence scope.** The name replaces only the caller's target, and instance discovery is unchanged when
   Herdr env is present (SA1). **Recommend: accept.**
3. **D3 Read-only bindings.** Add `HERDR_THREADS_SEAT_READONLY=1` for headless or one-shot sessions on a role seat.
   Such a session reads and sends but never ACKs on display. The alternative is to document distinct names only.
   **Recommend: add it.** It is small, and without it use case 3 silently discharges obligations.
4. **D4 Selector reading of names.** A bare token is a name only when no pane-path reading matches, and
   auto-creation happens only on seat-acting calls. The alternative is an explicit `seat:` prefix.
   **Recommend: fallback reading.**
5. **D5 Self-retire on the wire.** Send `Operator`, with the local label and an accepted limit, and refuse while
   another live binding exists unless `--force`. The alternative is a cap-gated `RetireReason::SelfRetired` now.
   **Recommend: limit now, cap later if peers need it.**
6. **D6 hc wake.** No composer wake on hc until herdr-core ships a draft-safe send (detected through features);
   Channel wake for Claude with the mod is kept. File the upstream asks. The alternative is a screen pre-read plus
   `if:idle` with an accepted limit, which submits a person's draft at their first 3 s pause.
   **Recommend: wait for upstream.**
7. **D7 hc handoff.** Phase H3, after H1, with `no_parent`/`no_notify`, the prompt through ht and the env blanked.
   **Recommend: H3, not v1.**
8. **D8 Coexisting with hc notes.** Our own launches never produce notes, and user-made hc children are the user's
   configuration. **Recommend: accept.**
9. **D9 Cron loops.** A named seat requires a harness session, and named person seats (`me init --name`) are
   deferred (07 O5). **Recommend: accept.**
10. **D10 hc closure.**
    - `killed`, or `removed` of a resolved seat, retires the seat.
    - Exit, signal, or core stop/shutdown unresolves it, so a resume reattaches it and otherwise the operator
      retires it.
    - Only shell-rooted terminals give continuity across sessions.

    **Recommend: accept.**
11. **D11 hc labels.** Display and select hc terminals as `<HC_SESSION>:tN`, using bare `tN` only when exactly one
    core is attached. **Recommend: accept.**
12. **D12 Restore on multi-kind nodes.** A seat with no name is held unresolved for the operator unless the node's
    dialled hosts are all of one kind. **Recommend: accept.**
13. **D13 (O1).** Drop `SeatStatusEntry.name` from the spec. **Recommend: drop.**
14. **D14 (O2).** Use the fixed reserved named locator `/.herdr-threads/named-host`. **Recommend: fixed path.**
15. **D15 (O3).** `cooperative_named_seat` names the named-host observation, not the binding. **Recommend: accept.**
16. **D16 Approve spec amendments SA1–SA15** (Part 1) into `2026-10-31-named-seats-design.md`.
    **Recommend: approve.** SA7 depends on D3 and SA14 on D4.
17. **D17 Phase order.** F, then N, then H1. H2 waits for upstream; H3 follows H1.
    **Recommend: accept, and merge N before H1.**
18. **D18 Minor items.**
    - O6: keep the `peer-tab-create` alias for one release.
    - O8: `check-host-neutral` as a follow-up bead.
    - O9: hc wake e2e stays opt-in until H2.
    - O7 (`focus_known` on hc) is moot until H2.

    **Recommend: accept as listed.**

19. **D19 Live-holder guard (SA16).** Narrows your earlier "concurrent sessions are bindings of one seat" to the live holder's own process tree: a second unrelated live session with the same name is refused; a crashed or exited one frees the name at once. **Recommend: adopt, alongside D1.**
20. **D20 tmux wake for named seats (SA18).** Phase N2 after N, gated on T1-T5. **Recommend: adopt as N2.**
21. **D21 Agent status.** Host-neutral rule now: each host reports `agent_status` with its source; null/missing is Unknown, never idle; every wake still needs a composer read (both Herdr's screen status and OSC 7501 say idle with a draft). Upstream ask to Herdr TUI: parse OSC 7501 and answer `7501;?`, prefer reported status over screen detection with fallback, expose `status_source` and `turn` in pane.get/events. Our Herdr path consumes it only once Herdr exposes it (we never see pane bytes). **Recommend: file the ask; adopt the rule.**

## Part 6. Remaining spikes

| Spike | Question | Gate |
|---|---|---|
| S1 | Named allocation/enumeration race under the lifecycle fence; restart CarryForward | N |
| S2 | First empty named publication clears holds without `baseline_hold_unclaimed` | N |
| S3 | Mod `watch` inherits the session env and never stops on `no_pane` for a named site | N |
| S4 | Self-retire idempotency replay scope | N |
| S5 | Restored named seat vs a newer live seat of that name; two-kind restore (TUI + fake hc) | N |
| S6 | `doctor` on never-settled `wake_work` of named seats | N |
| E1 (new) | Read the host server's env: socket peer pid (`SO_PEERCRED`/`LOCAL_PEERPID`) plus `/proc/<pid>/environ` / `KERN_PROCARGS2`, on Linux and macOS, for Herdr and `herdr-core serve` | N (Herdr), H1 (hc) |
| D1 | A Herdr TUI socket's reply to a JSON-RPC `hello` (the probe must not hang) | H1 |
| H-C1 (new) | Codex hooks in hc with and without `--no-daemon`: does the ancestor guard see them? | H1 |
| W1 | `terminal.read` text under each `ComposerPolicy` | H2 |
| W2 | Claude/Codex status after a completed turn (`idle` vs `done`), with and without a draft | H2 |
| W3 | `working` reported during tool runs | H2 |
| W4 | `getpgid` across sessions; aisw-wrapped leader vs fingerprint pid | H2 |
| W5 | Typing in a focused app pane while a wake is due | H2 |
| L1–L4 | New-tab labels; shim `exec` keeps `terminal.pid`; `outlive_parent`+`no_notify` stop notes; Codex `--no-daemon` and the Linux socket path | H3 |
| T1-T5 | tmux: capture-pane vs Herdr detection read per ComposerPolicy; idle/working/blocked recognition incl. missed Stop; draft safety with no server-side guard; stale TMUX_PANE in nested Herdr/tmux refused; tmux discards OSC 7501 | N2 |
| Upstream | herdr-core: atomic draft-safe `send` (`if_input_empty` / `quiet:"refuse"`) and `if:[idle,done]`, advertised in `hello.features` | H2 |

## Appendix: run files

- [00 Seat mechanics today](../runs/2026-10-31-environments/00-seat-mechanics-today.md)
- [01 herdr-app research](../runs/2026-10-31-environments/01-herdr-app-research.md)
- [02 Named seats code design](../runs/2026-10-31-environments/02-named-seats-code-design.md)
- [03 ht host coupling](../runs/2026-10-31-environments/03-ht-host-coupling.md)
- [04 Mac spikes (hc 0.1.15 / herdr-app 0.1.10)](../runs/2026-10-31-environments/04-mac-spikes.md)
- [05 herdr-app vision (revised)](../runs/2026-10-31-environments/05-herdr-app-vision.md)
- [06 herdr-app code design](../runs/2026-10-31-environments/06-herdr-app-code-design.md)
- [07 Unified code design (revised)](../runs/2026-10-31-environments/07-unified-code-design.md)
- [Approved named-seats spec](2026-10-31-named-seats-design.md)

## Post-Implementation Notes

> *As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
