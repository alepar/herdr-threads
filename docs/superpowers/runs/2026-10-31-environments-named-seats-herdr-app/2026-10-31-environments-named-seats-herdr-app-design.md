# Environments: named seats and herdr-core/herdr-app behind one host seam

- Bead: ht-kqrt. Branch: `super-auto/environments-named-seats-herdr-app` (base `main`). Status: draft, root spec of
  an autonomous super-auto run (2026-10-31), Mode B (one-shot).
- Sources: the roasted proposal
  [`docs/superpowers/specs/2026-10-31-environments-proposal.md`](../../specs/2026-10-31-environments-proposal.md)
  (decisions D1-D21, amendments SA1-SA18, phases, spikes) and the unified code design
  [`07-unified-code-design.md`](../2026-10-31-environments/07-unified-code-design.md). Supporting: the approved
  [named-seats spec](../../specs/2026-10-31-named-seats-design.md),
  [02](../2026-10-31-environments/02-named-seats-code-design.md),
  [03](../2026-10-31-environments/03-ht-host-coupling.md),
  [04](../2026-10-31-environments/04-mac-spikes.md),
  [05](../2026-10-31-environments/05-herdr-app-vision.md),
  [06](../2026-10-31-environments/06-herdr-app-code-design.md).
- Notation: "07 §4.4" is a section of 07, "06 §7" of 06, "02 §2.3" of 02. "P:" cites the proposal by part.
  Paths are repo-relative. New files are marked (new).

**Run constraints (fixed for this run, not open questions).**

1. D1-D21 are taken at the proposal's stated recommendations (P: Part 5). SA1-SA18 are applied. SA16 (live-holder
   guard) narrows the approved spec's Identity row, and D19 is adopted. Where the approved named-seats spec and an
   amendment disagree, the amendment wins. The spec file itself is not edited by this run; the amendments are
   recorded here.
2. The tree stops at merge-ready code on the run branch. The coordinator merges to `main`. This run neither merges
   to `main` nor pushes.
3. Phases in scope: **F**, **N**, **N2**, **H1**, **H3**. H2 (hc composer wake) is out of scope. herdr-core ships no
   draft-safe send, so H2 is listed as follow-on only.
4. herdr-core is not installed on this Linux host, and the macOS laptop is reachable only by the top-level
   human-facing session, never by implementers. H1 and H3 are built and tested against an in-process fake core
   (`fake_hc`). The fake speaks the JSON-RPC shape recorded in 04 and in herdr-core's
   `schema/hc.openrpc.json` (local checkout `/var/home/alepar/AleCode/herdr-core` at `dc276b7`, version 0.1.15).
   An opt-in real-core smoke is a separate leaf with a read-only fallback. Spikes that need macOS are kept off the
   critical chain or answered by a fallback. Their live parts go to the run report as remaining evidence.
5. tmux 3.7b is installed locally. N2's spikes T1-T5 run in isolated tmux servers (`tmux -L <private socket>`),
   never the user's default tmux server.
6. ht-tod8's `lifecycle::scrub_env` and `host::PANE_SCOPED_ENV_VARS` (commit 5e19e9bce) are on `main` (a617faf90;
   src/daemon/lifecycle.rs:358 and src/host/mod.rs:11 there). The run branch's base f4ddaeffa predates them, so
   the first F leaf brings `main` into the run branch and reuses the helper (local `main` is cb70a2339, which
   contains both commits; the leaf checks `git merge-base --is-ancestor 5e19e9bce main` once and stops if it
   fails). On `main`, `scrub_env` is a private `fn`. F makes it `pub(crate)` and adds one list source,
   `host::child_env_scrub_list()`, the union described under "Env scrub". `spawn_detached`, the H3 gate shim and
   every handoff spawn take their names from that one function, so the list is defined once.
7. Every phase keeps the repo rules in AGENTS.md: harness-neutral core, wire compatibility, TRUST-POLICY.md amended
   in the same commit as the behaviour, the delivery contract, stated sizes for new model-visible strings, test
   isolation, and the full suite under 5 minutes.

## Goal

An agent in a Herdr TUI pane, an agent in a plain or tmux process started with `HERDR_THREADS_SEAT_NAME=<name>`,
and an agent in a herdr-core terminal (`hc run` or a Herdr desktop-app pane) each get a durable seat with full
messaging: join, send, inbox, ACK, turn-hook and channel delivery, and cross-environment and cross-node addressing.
All three go through one environment-agnostic core, where routing, delivery, receipts and binding logic never learn
which environment a seat lives in. Idle wake works as far as each host allows.

## Problem description

herdr-threads finds its caller only through Herdr TUI env (`HERDR_PANE_ID`, `HERDR_ENV`). It dials one kind of host
(`NativeCli`, the only production `HostPort`, src/ports.rs:3907), and it assumes a Herdr server in its CLI label
reads, discovery, install and daemon start. An agent outside Herdr has no seat at all: in a herdr-core terminal,
herdr-threads 2.8.0 refuses `inbox` for want of `HERDR_PANE_ID` (04 step 7). Worse, an hc core autostarted from a
Herdr pane leaks that pane's id into every terminal, so the agent would bind the foreign pane's seat (04 B). Two
latent defects also exist before any new environment:

- **K4.** The CLI joins labels by target only (src/cli/human.rs:52-61), so two hosts that both own `t1` cross
  labels.
- **K5.** `spawn_detached` (src/daemon/lifecycle.rs:356-390) scrubs too little. A daemon autostarted inside an hc
  terminal keeps `HC_SCOPE`, and hc sweeps it when the terminal ends.

## Main challenges

The challenge is to add two environments without letting environment knowledge leak into policy, and without
weakening TRUST-POLICY's continuity and attribution invariants. That breaks down into five parts:

- **Identity.** A seat name inherited through a host server's env must not fold unrelated agents into one seat.
- **Closure.** A named host whose enumeration is the store reading itself must never retire seats by absence, and
  hc closure must follow the terminal's finish reason, not agent exit.
- **Wake.** Hosts that cannot type into a composer (named, hc until upstream) degrade delivery rules 5 and 6, and
  that must be stated honestly and warned once, not hidden. tmux wake must be draft-safe without a server-side
  guard.
- **Scale of change.** Every node-wide `attached()` consumer, 40 `HostPort` impls, `hook.rs` attribution paths, the
  store migration and the local protocol must stay skew-safe, behind defaulted methods and capability answers.
- **Evidence.** It has to be gathered without a real herdr-core on this host and without macOS access.

## Key decisions made

- **Base design.** 07's minimal-change design ("A"), with B's audit discipline grafted on (07 §1). Every
  environment is a host behind the existing `HostPort`, which gains only defaulted capability answers:
  `wake_capability`, `closure_authority`, `archival_evidence`, `target_root_process`, `server_process`, `kind`.
- **Three hosts.**
  - The built-in `NamedHost` is never dialled. It sits outside `attached()` and is `NonClosing`.
  - The dialled `HcHost` is keyed on the core id, closes by finish reason, and has wake `None`. Its
    `server_process` is the core socket's peer pid, so the inherited-name refusal covers hc terminals too.
  - `NativeCli` is unchanged.
- **One CLI env reader.** `src/cli/caller_location.rs` (new) is the only reader of caller-location env. It applies
  this precedence:
  1. a valid name replaces the target, and discovery is unchanged;
  2. one family is accepted unverified;
  3. two families go to the daemon's nearest-ancestor `LocateCaller`;
  4. otherwise the call is refused.
- **Inherited and concurrent names.** An inherited name is refused when the host server's env carries it (D1). A
  live unrelated holder of a name is refused (D19/SA16).
- **Store and wire.** There is one store migration (NodeDelta), owned by F, so N and H1 add no schema. The peer
  wire is unchanged in F/N/N2/H1. H3 adds one cap-gated launch variant.
- **Wake.**
  - Named and hc seats have no composer wake. Channel wake for Claude with a live mod is kept.
  - No-wake reports are persisted once per (sender, seat) per no-wake episode. Rules 5 and 6 are documented as degraded in AGENTS.md,
    TRUST-POLICY and the skill.
  - N2 adds tmux composer wake for named seats with a verified tmux hint, gated on hook turn state plus a screen
    recognizer, and a missing status never reads as idle (D21).
- **hc launch (H3).** It goes through `CreateTargetPort` and a launch-gate shim that blanks leaked env, with
  `no_parent`/`no_notify` on the hc side.

## Decision points, by section

### Environment model and host seam

**HostPort capability answers.** Recommended: add six defaulted methods to `HostPort` in src/ports.rs (07 §4.1):

- `kind() -> HostKind`;
- `wake_capability() -> WakeCapability {None, Composer(ComposerTraits{submit_budget})}`;
- `target_root_process(&HostTargetId, &HostCallContext) -> Result<Option<u32>, ApiError>`;
- `server_process() -> Option<u32>`;
- `archival_evidence() -> ArchivalEvidence {Supported, NoEvidence}`;
- `closure_authority() -> ClosureAuthority {Closing, NonClosing}`.

Each default reproduces today's Herdr behaviour and returns a value, never `ErrorCode::Unsupported`, so the
self-test at src/ports.rs:4533-4572 and the 40 existing impls compile unchanged. Core delivery, closure, wake,
receipt and binding policy reads these answers, never `kind()`. The permitted `kind()` readers are exactly these,
and a code-review checklist item in each leaf names any new one:

- the peer-discovery gate `any_reachable` (`kind()==Herdr`, G1 audit);
- the G4 implemented-kinds check on reattach, restore and rebind, and the probe's stored-kind comparison;
- the adapter launch argv `host_launch_argv(HostKind)` (H3), which is adapter-owned, not core policy;
- `doctor` and diagnostics output.

`native_launch_capability()` (src/ports.rs:3909) keeps meaning in-place launch: named and hc return
`Unsupported` in every phase, and hc creation is a separate answer on `CreateTargetPort` (H3). The methods a
built-in host cannot serve (`observe_current_target_for_archival`, `safe_wake_target`, `submit_prompt`,
`launch_native`) answer with a non-Unsupported `Err`. Each is unreachable because a capability answer refuses
first (07 §4.1). Considered: B's split of `HostPort` into a core plus five capability traits. It was rejected
because every impl churns for no behaviour, and a forgotten accessor returning `None` lets wake tests pass
vacuously (07 §1). Also considered: 02's `has_composer() -> bool`. It was rejected because the dispatcher needs
the submit budget (K7) and N2 needs a per-target answer.

**HostKind and implemented kinds (G4).** Recommended: `HostKind` gains one variant per phase. F has
`{Herdr, Unknown}`, N adds `Named`, H1 adds `Hc`. `Unknown` means "this build has no constructor for the persisted
kind". A static implemented-kinds set grows with the variants. Reattach, restore and rebind skip a row whose kind is
not in the set, and `doctor` names it. A field, variant or method lands in the phase that reads it. This also keeps
`scripts/check-default-features` free of dead-code warnings. Considered: defining all three variants in F. It was
rejected because an F binary reading an `hc` row would treat the kind as known and dial the core as Herdr (roast
U-A3).

**Built-in vs dialled hosts.** Recommended: `HostSessions` (src/service/host_sessions.rs) keeps its dialled map and
gains a `builtin: OnceLock<Arc<HostSession>>` slot. `attach_builtin(port, observer, locator)` skips `factory.exists`,
the on-disk canonical check and `MAX_ATTACHED_HOSTS` (16, :42). `by_locator`/`resolve` answer from the built-in slot
first. `all_for_reads()` returns dialled sessions plus the built-in one. The wake lane starts only when
`port.wake_capability() != WakeCapability::None`, so `HostSession.wake` becomes `Option<LaneHandle>`. Its consumers
(`first_host_pacer`/`Lane::Wakes` src/app.rs:1028-1043, `reachability.attach_wake_pacer` host_sessions.rs:710,
Health) take `Option`, so a missed consumer is a compile error (07 §4.2, §14 risk 3). F lands the slot, the API and
the Option plumbing with Herdr-only callers. N is the first production caller of `attach_builtin`. Considered:
putting the named host inside `sessions` (B). It was rejected because every node-wide consumer would become
load-bearing at once.

**`attached()` / `all_for_reads` audit (G1).** Recommended: apply the verified consumer table in 07 §5.3 as written:

- idle exit counts only dialled hosts that hold a resolved local seat;
- the cap and the reattach sweep stay dialled-only;
- `startup_primary` falls back to a dialled host filtered to `HostGuardedStart`;
- archival filters on `archival_evidence() == Supported`;
- `operator_host` tries dialled hosts, then the built-in host, with the text "no host attached" (G5);
- `wake_status`, `RemoteResolver::answer`, seat publisher `read_labels` and `HostTopologies` use `all_for_reads`;
- peer `place()` uses only dialled hosts that have a creation capability;
- the peer-discovery gate `any_reachable` uses only `kind()==Herdr` sessions.

Two regression tests are required: "`attached()` never contains the built-in host" and "idle exit with only the
named host exits". Each dialled-only consumer also gets a two-host (TUI + fake hc) test in H1, because hc can hold
the lowest host id: `startup_primary`, `first_host_pacer`, `place()`, the discovery gate, archival
(`archival_evidence() == Supported`) and the production `BootstrapRuntime` gate (07 §4.6). Considered: a partial list (A). It was rejected because unlisted consumers were where the roast
found defects (U-A2).

### Caller location

**`CallerEnv` / `CallerLocator` / `CallerSite`.** Recommended: `src/cli/caller_location.rs` (new) is the only reader
of caller-location env. It exposes the following (07 §4.4):

- `CallerEnv::{from_process, from_lookup}`;
- `locate(EnvMode::{Cli,Hook}) -> Result<Option<Located>, LocatorError>`;
- `Located::{One, Ambiguous}`;
- `CallerLocator::{Pane, Named, Terminal{socket,id}}`, one variant per phase. F lands `Pane`, plus a
  recognition-only reading of `HC_TERMINAL_ID`/`HC_TERMINAL_SOCKET` for the phase guard. N adds `Named`, H1 adds
  `Terminal`.

`CallerSite` answers `moved_target`, `relative_pane`, `sent_scope`, `required_capability`,
`locate_candidate`, `terminal_address`, `claim_host_locator` and `env_assignments` as one closed match. Variable names
come from each host module's `ENV_VARS` const (G3): `host::native::ENV_VARS`, `host::named::ENV_VARS`,
`host::hc::ENV_VARS`. Empty values are filtered: src/cli/mod.rs:311 does not filter them today, unlike
hook.rs:258-260. `CallerClaim` (src/protocol/authority.rs:8-29) is unchanged.

"Only reader" is mechanical. Today the caller-location reads are src/cli/mod.rs:311 (`HERDR_PANE_ID`),
src/cli/hook.rs:257-260 (`HERDR_ENV`, `HERDR_PANE_ID`; the gates at :1463, :1790, :3485, :4019 and :4393 read the
resulting `herdr_env` field) and the handoff recovery hints. F moves all of them into `caller_location.rs`, and
instance discovery (src/cli/instance.rs, step 4 of SA1's order) takes `HERDR_ENV` from `CallerEnv` rather than
reading it. A test in `tests/combined.rs` scans `src/cli/**/*.rs` and fails if a string literal `HERDR_PANE_ID`,
`HERDR_ENV`, `HC_TERMINAL_ID`, `HC_TERMINAL_SOCKET`, `HERDR_THREADS_SEAT_NAME`, `HERDR_THREADS_SEAT_READONLY`,
`TMUX` or `TMUX_PANE` appears outside `src/cli/caller_location.rs` and the host modules' `ENV_VARS` consts
(src/host/**). `HERDR_SESSION`/`HERDR_CONFIG_PATH` (src/cli/instance.rs:80-81) are discovery env, not caller
location, and stay where they are. src/cli/watch.rs reads no caller-location variable: its env closure (:1093)
feeds `caller_env::harness_fingerprint`, which is harness env and stays. watch.rs consumes `CallerSite` only for
the target it watches (`moved_target`, and the Named site in N, spike S3). Considered: `dyn CallerSite` /
`LocatorChain`. It was rejected because three built-in variants matched in about six small methods do not justify a
trait object (07 §1).

**Precedence.** Recommended, per 07 §4.4 and SA1:

1. A non-empty `HERDR_THREADS_SEAT_NAME` that parses as `SeatName` replaces only the caller's target. Instance
   discovery is unchanged. An invalid name is a hard error naming the variable (string M16).
2. One family (`One(Pane)` or `One(Terminal)`) is accepted unverified, as A1 accepts today. In hook mode a pane
   still needs `HERDR_ENV=1`. `Terminal` without `HC_TERMINAL_SOCKET` is refused (M17).
3. Both families produce `Ambiguous`, which goes to the daemon's `LocateCaller`.
4. If neither candidate verifies, the call is refused, with no fallback: in F and N by the phase guard (M1, M2),
   from H1 on by the two-family refusal (M4). With no family and no name, today's error text is unchanged.

Explicit cooperative flags combined with a name are refused with one line (02 3.1, M14). The M-numbers refer to
the table under "Model-visible strings". Considered: the approved spec's
"name wins over everything, including discovery". It was rejected because it ignores the pane's command group and
splits stores (roast N-T7, N-D6, N-A7). Also considered: 02 §9's fixed rank of `HC_TERMINAL_ID` over the pane. It
was rejected because a stale `HERDR_PANE_ID` inside an hc terminal is the common nested case (04 B).

**`LocateCaller`.** Recommended: `Command::LocateCaller{candidates, subject}` →
`LocateReply::{Verified{index, root}, Unverified{per_candidate}}`, in src/service/locate.rs (new). It is sent only
for `Ambiguous`.

- The walk starts at the subject (the claim's liveness pid, else the connection's peer pid) and goes up through
  `ProcessInfo` parents. The first pid that equals a candidate's `target_root_process` wins, nearest ancestor
  first.
- src/daemon/transport.rs:430 exposes the peer pid (tokio `UCred::pid()`) next to the uid.
- The cache is keyed `(candidates, subject pid, subject start stamp, boot/epoch)`. The start stamp is the
  `ProcessRecord.start` the walk reads for the subject; a subject whose record is unreadable (no start stamp) is
  never cached, and its walk runs every time (it is cheap). A reused pid has a different start stamp, so it never
  hits another process's verdict. Entries are dropped when the boot/epoch changes.
- In F only the Herdr candidate can verify, because `NativeCli::target_root_process` = `pane.process_info.shell_pid`.
  H1 adds the hc candidate.
- **Terminal-root wall.** The walk treats a process whose `argv[0]` basename is in
  `host::foreign_terminal_process_names()` (F: a scrub-style const list in src/host/mod.rs holding `herdr-core`
  and `hc`; it moves into `host::hc` in H1) as a wall: a candidate root found above a wall does not verify. On
  herdr-core's non-systemd spawn path the core and its holder stay descendants of the `hc` client, and so of the
  pane shell that ran it, while that client lives (herdr-core platform/unix.rs `spawn_detached` only `setsid`s).
  Without the wall the Herdr candidate would verify for an agent inside an hc terminal. In H1 the hc candidate's
  root sits below the wall and is found first, so the wall changes no H1 outcome. A process whose argv is
  unreadable is not a wall (accepted limit, F/N only, recorded in TRUST-POLICY F row).

Considered: tty comparison. It was rejected because hc exposes no tty and hooks have no controlling terminal (06 V3).
Also considered: verifying every caller. It was rejected as adversarial, which TRUST-POLICY forbids (roast H-T4).

**TUI leak guard (F).** Recommended: when both families are present before `HcHost` exists, F verifies only the Herdr
candidate. If it verifies, with no terminal-root wall crossed on the way (see "`LocateCaller`"), the pane wins.
Otherwise the CLI prints M1 and nothing binds. N appends M2, so F never
advertises a variable its build ignores. Hooks exit 0 with M1 (+M2 from N) as their one stderr diagnostic, and
inject the hook guard line M3 (+M2 from N) into context on these paths only:

- lifecycle check-ins and compact re-entry, for every harness;
- never in the section that `OutputPolicy::guidance_every_turn` repeats on every delivering hook. A harness whose
  policy says `guidance_every_turn` and that has a lifecycle event (today Claude: `SessionStart`) gets M3 on that
  check-in only. A harness with `guidance_every_turn` and no lifecycle event (today Hermes) gets M3 on its first
  delivering hook per binding. The choice is keyed on the adapter's `OutputPolicy` and lifecycle-event
  registration, never on a harness name.
- The "first delivering hook" memory is daemon memory, keyed by binding id. It re-arms on daemon restart, so such a
  harness can see M3 once more after a restart. That is an accepted limit (no column in `0012`).

H1 removes M1, M2 and M3 with their code: once `HcHost` exists a two-family caller goes to `LocateCaller`, and
"neither verifies" prints M4 from the CLI and from hooks (hooks still exit 0 and inject nothing). An H1 test asserts
that M1 and M3 no longer appear from a Terminal site. Considered: a per-process leak notice on every CLI call. It was
rejected because a one-shot CLI would print it every time.

**Inherited-name refusal (SA2, D1).** Recommended: when a host candidate sits beside the name, the resolve command
carries that candidate as a `locator_hint`. The daemon then reads the environ of the host's `server_process()`:
`/proc/<pid>/environ` on Linux, `KERN_PROCARGS2` on macOS, same user only. If the environ carries the same name,
the daemon refuses with M6 and never falls back to the host candidate. If the server env is unreadable,
`server_process()` is `None`, the check is skipped, and only observability remains (D1 limit, 07 §14 risk 11).
The check compares the name only, so it cannot tell a per-command name from an inherited one when both are the
same string: a caller that sets the server's own name per command is refused too. That is intended (the name is
the server's role) and is an accepted limit; M6's remedy says "use another name", and only a different name or a
server restarted without the variable binds.

- NativeCli's `server_process` is the socket peer pid that the incarnation witness already records
  (src/host/native.rs:104-130). `<host>` in M6 is `Herdr`.
- `HcHost::server_process` (H1) is the peer pid of the core socket (the `herdr-core serve` process), taken from
  the `LocalEndpointWitness` (src/host/continuity.rs) the client already records. A `<core>:<tN>` locator hint is
  checked the same way, and `<host>` in M6 is `herdr-core`.
- **Observability.** Each named binding records its `locator_hint`. When a binding records a hint that no earlier
  binding row of that seat (ended rows included) carries, while another distinct hint exists, the daemon returns M9.
  The decision reads the persisted `occupant_bindings.locator_hint` rows, so it fires once per (seat, locator)
  across restarts with no further column. `seat inspect` lists bindings by locator.

Considered: a scope stamp (`HERDR_THREADS_SEAT_NAME_SCOPE`), or a `herdr-threads run --name` launcher with a root
marker. Both were rejected because they put the burden on users (D1).

**Live-holder guard (SA16, D19).** Recommended: when a named seat would gain or reconnect a top-level binding, the
daemon's enroll transaction checks the seat's other connected top-level bindings with
`LivenessFingerprint::is_alive` (pid + start time; struct at src/protocol/authority.rs:57, `is_alive` at :83,
which already takes `&dyn crate::ports::ProcessInfo`). It refuses with `seat_name_in_use` (M7) when all of these
hold:

- a holder is alive;
- the holder differs from the caller;
- neither is in the other's parent chain.

The following are allowed: the same process, its ancestors and descendants, subagents (`agent_id`, as in a pane),
D3 read-only bindings, and Codex forks and Hermes under their existing rules. A dead or reused pid frees the name at
once. The liveness check itself exists. The only new plumbing is getting a `&dyn ProcessInfo` (src/ports.rs:4177)
into the enroll transaction in src/store/seats.rs. It complements SA2: SA2 catches sequential folds through a host, and SA16 catches
concurrent unrelated holders. It makes SA6's self-retire check and SA7's concurrency test mostly moot, but both are
kept. Considered: the approved spec's "concurrent sessions are bindings of one seat". It was narrowed by D19,
because two unrelated live sessions sharing one role's ACK obligations is how mail gets absorbed silently.

**Env scrub (K5, SA2 scrub).** Recommended: `spawn_detached` (src/daemon/lifecycle.rs:356-390) removes the union of
the following, through `lifecycle::scrub_env(command, names)` from ht-tod8, and reparents the daemon:

- `host::PANE_SCOPED_ENV_VARS`;
- `host::named::ENV_VARS` (`HERDR_THREADS_SEAT_NAME`, plus `HERDR_THREADS_SEAT_READONLY`);
- `host::hc::ENV_VARS` (`HC_TERMINAL_ID`, `HC_TERMINAL_SOCKET`, `HC_SCOPE`) and `HC_SESSION`.

`HC_SESSION` goes into `host::hc::ENV_VARS` too, so the lifecycle module carries no host literal. In F the named and
hc consts exist as scrub-only lists in `src/host/mod.rs`, and they move into their modules in N and H1. The union
is returned by `host::child_env_scrub_list()` (F); `scrub_env` becomes `pub(crate)` (run constraint 6). The H3 gate
shim, which removes the names from its own environment before `exec` rather than through `ProcessCommand`, and every
handoff spawn take the same list, so `HERDR_THREADS_SEAT_NAME` is dropped everywhere from one definition. The skill and `setup` text say to set the name
per command.

**Reparenting.** herdr-core's sweep (crates/herdr-core/src/sweep.rs `members`) kills the processes whose environ
holds the terminal's `HC_SCOPE` plus every live descendant of them by `ppid`. A scrubbed env alone does not take
the daemon out of that set while the process that autostarted it lives (the mod's long-lived `watch`, src/cli/watch.rs
`ensure_daemon`), because `setsid` changes the session, not the parent. So `spawn_detached` double-forks: it spawns
an intermediate `herdr-threads internal daemon-detach` with the scrubbed env, which `setsid`s, spawns the daemon
with the same args and stdio, prints the daemon pid on its stdout pipe and exits at once. The launcher reaps the
intermediate and keeps the reported pid wherever it keeps the child pid today (`Launched`). The daemon is then a
child of init (or the nearest subreaper) before it serves its first request. Wherever the design relies on the parent chain it treats a `setsid`-only spawn as still a
descendant: this rule and the terminal-root wall in `LocateCaller` are its two instances.

Considered: hygiene-only scrubbing of `HERDR_BIN_PATH`, as today. It was rejected because hc sweeps every process
carrying the terminal's `HC_SCOPE` and their descendants (06 K5), so scrub plus reparent is a correctness fix.

### Store and schema

**One NodeDelta, owned by F.** Recommended: a single new delta, `migrations/node/0012_host_environments.sql`,
registered in `NODE_DELTAS` (src/store/schema.rs:82). 0012 is the next free slot today (`main` ends at
`0011_peer_hygiene.sql`). The F leaf takes the next free `NODE_DELTAS` slot after it brings `main` in, and the
run's final pre-handoff check re-checks it against `main`. If `main` has gained a delta by then, the coordinator
renumbers this one on merge (file name and `NODE_DELTAS` position together); run.md records that rule. It carries
every column of 07 §6.4:

- `herdr_hosts.kind TEXT NOT NULL DEFAULT 'herdr'`, with no CHECK (validated in code, so no table rebuild later);
- `herdr_hosts.kind_probed_at INTEGER` (NULL means the kind came from the default, so re-probe on reattach);
- the partial unique index `herdr_hosts_one_named`;
- `seats.name` with a superset CHECK (1-64 bytes, charset `[A-Za-z0-9._-]`, as 07 §6.4's SQL). The leading-`-`/`.`
  and not-seat-id-shaped rules are validated in code by `SeatName::parse` (N), as `kind` is. N adds a corpus test
  that every name `SeatName::parse` accepts passes the CHECK, and that every name the CHECK rejects `SeatName`
  rejects. Plus the unique index `seats_live_name` on `(host_id, name)` over non-retired local rows, unresolved
  rows included;
- `occupant_bindings.locator_hint TEXT`;
- `occupant_bindings.readonly INTEGER NOT NULL DEFAULT 0` (D3 accepted);
- the table `no_wake_reports(observer TEXT, seat_id TEXT, PRIMARY KEY(observer, seat_id))`.

F writes `kind` and `kind_probed_at` for Herdr hosts. The other columns stay unwritten until N or H1. Considered: one
delta per phase. It was rejected because N and H1 run in parallel, and two deltas would race for one version number.
The run constraint is also one migration.

**`seats.name` and the literal `'named'`.** Recommended: `seats_live_name` also covers `unresolved` rows, which
`seats_live_target` does not (migrations/node/0001_node_baseline.sql:1670). So a named seat that went unresolved
cannot be split by the next resolve of its name. The store knows the literal `'named'` in two places only: the
unique index, and the restore/rebind rules (07 §7). Remote rows store `name` from `seat.created.display_name` after
grammar validation. An invalid value is stored as NULL and never rejected.

**`locator_hint`, `readonly`, `no_wake_reports`.** Recommended:

- `locator_hint` is, on the wire, every host candidate the caller has, newline-joined: the pane id and/or
  `<core socket>:<tN>`, socket included, so the daemon selects the core by socket. A binding stores only the first
  candidate, a terminal as `<core>:<tN>` (the socket's file stem). It is a hint, never authority; only the second-locator notice,
  `seat inspect` and the pane-form refusal read it.
- `readonly=1` is set when the binding's env has `HERDR_THREADS_SEAT_READONLY=1` (D3). That binding's display never
  ACKs receipt-bearing rows, and its turn hooks skip them, but it reads and sends normally.
- `no_wake_reports` rows are written by `wake_status` for seats whose effective wake capability is `None`
  (`target_wake_capability` from N2 on). Two things delete a row: retire of either seat, and a `wake_status` read
  that finds the seat's effective capability `Composer`, which matches today's forget-on-recovery
  (src/service/dispatch.rs:290-300). A row is written only while the capability is `None`. So the report fires
  once per (sender, seat) per no-wake episode, across daemon restarts and channel flaps (SA10). A seat that goes
  None → Composer → None re-reports once. The table needs no size bound like `MAX_WAKE_REPORTS` (4096, :127): it
  is bounded by local observers × seats, and retire removes rows.
- Wake rows gain `host_id` by joining `seats.host_id` in the `wake_status` query, not as a column. The protocol
  `WakeStatus` row gains an optional host id field marked `#[serde(default)]`, so an old reply decodes.

Considered: the in-memory `first` set (src/service/dispatch.rs:118-127). It was rejected for no-wake hosts because
it re-arms on every daemon restart and mod flap (roast N-D2, U-D1). The set stays for Composer hosts.

### Named host

**Observation and enumeration.** Recommended: `NamedHost` in src/host/named.rs (new) follows 02 §2.3, with 07's
changes:

- Boot and incarnation are fixed per node, derived from the node uuid (`named-host:node=<uuid>`).
- `observe_current_target(name)` returns present, generation 1, terminal id `named:<name>`, for every valid
  `SeatName`.
- `enumerate_targets` is a store read through `NamedSeatSource::live_named` of every non-retired local seat with
  `host_id = named`. The read is keyed on the host id, not the name, so no row can be missing.
- `closure_authority() = NonClosing`. Reconcile skips the absence-closure step for non-closing hosts
  (src/identity/reconcile.rs:325-335), so a named enumeration never yields `BeginRetirement` or `MarkUnresolved`.
- Snapshot rows carry `cooperative_named_seat` and are never `CoherentVerified`.
- A store guard refuses `RetireReason::Closed` for a seat on a `named` host (SA3).

The named host runs the observation lane, not the wake lane (02 C2). Considered: an enumeration derived from names.
It was rejected because a missed row would retire a live seat (roast N-T2, N-A2).

**Write load.** The named lane's only input is this node's store, so it publishes on change. A pass commits an admission, a stage, a seal, a publication and a marker, about 58 KB of WAL under `synchronous=FULL` (about 1 GB a day on every running daemon, zero named seats included), so a pass per tick would be a permanent write load whose only effect is to copy rows back into the store that holds them. Each tick reads the live set instead (`HostPort::store_enumeration`) and skips the pass when it equals the set whose publication and reconciliation pass both finished and no lane failure is outstanding. A capture forgets the set when it starts, so a failed capture, a failed page, a refused pass or a lost marker is retried by a real pass at the lane's own retry time, whatever the live set is by then: only a real pass reports recovery to Health, the Pacer, host evidence and reachability. A refused pass keeps publishing at the cadence. A daemon start publishes once, which is the S2 first-empty publication, and a seat its own hook is waiting on marks the lane dirty and kicks it. An explicit capture is never skipped. The lane's cadence is the configured one (`snapshot_due_gated` takes it), and shared test daemons set `HT_TEST_LANE_TICK_MS=300`.

**Locator (SA15, D14).** Recommended: the fixed reserved path `/.herdr-threads/named-host`. `HostLocator::parse`
never stats (src/protocol/ids.rs:298-339), so the path needs no endpoint rewrite when a store opens under another
state dir. Considered: `<state_dir>/node/named-host`, from the approved spec. It was rejected because it forces a
kind-match plus an endpoint rewrite.

**Continuity: the C1 row (SA4).** Recommended: a fourth C1 mapping path, "named continuity". An ordinary resolve on
the named host whose name equals an unresolved named seat's name, with no live seat of that name, reattaches that
seat, recorded `cooperative_named_seat`. Retire ends continuity for good, and a retired name used again mints a new
seat. The cross-kind rebind refusal applies in src/store/seats.rs `rebind_unresolved_seat` (starts at :4167): a named seat
never rebinds onto a pane or terminal, and the converse holds. Considered: a fresh seat on every unresolved name. It
was rejected because it splits a role's threads and obligations.

**Restore (SA4, D12).** Recommended, changing the local-origin restore in `materialize_seat_created`
(starts at src/store/seats.rs:242):

- A row with `display_name` goes onto the named host, unresolved, at target = name.
- If a non-retired local seat with that name already exists on the named host (resolved or unresolved, the rows
  `seats_live_name` covers), the restored row is inserted unresolved on the named host with `name` NULL, so it is
  outside the index and the insert cannot violate it. A named-host seat with `name` NULL is the held marker (every
  other named-host seat has a name), and its held name is the `display_name` of its own `seat.created` fact, so
  nothing new is stored and no `allocation_decisions` kind is added (its `kind` CHECK stays). There is no merge and
  no automatic pick: resolves of the name keep reaching the existing seat, and the held row is never reattached by
  named continuity (it has no name). `seat inspect` and `doctor` show the held name.
- Operator repair: `human seat retire SEAT --operator` of either seat. When the existing seat of that name retires,
  the same retire transaction gives the name back to the oldest held row whose `seat.created.display_name` equals
  it (it sets `seats.name`, which the index now allows); named continuity then reattaches that seat on the next
  resolve of the name. Retiring the held row just retires it. No name moves while two candidates are live.
  A replayed own `seat.retired` of a restored row takes the same transition as the live retire: it ends the row's open
  bindings (`seat_retired`) with their catch-ups, offers and wake work, since a restored `unresolved` row can open a
  binding before its fact lands.
- A row without a name is placed automatically only when every dialled host attached or remembered on the node has
  one kind. Otherwise it is held for the operator.
- The always-attached named host never satisfies `wait:host`.

Considered: today's `ORDER BY last_attached_at DESC`. It was rejected because the named host is stamped at every
start and would win (02 §3.4), and on mixed nodes it sends hc seats to the TUI host (roast U-T2).

**Self-retire (SA6, D5).** Recommended: `seat retire --self` → `Command::SelfRetire{claim, operation}`, listed in
`Command::accountable_claim`. It is routed through `cooperative_mutation`, so subagents are refused. The retire body
is factored out of `operator_retire` (src/store/seats.rs:4759-4852) as `retire_seat(tx, seat, wire, label)`.

- On the wire it is `RetireReason::Operator`. Locally it is
  `allocation_decisions(kind='operator_retire', operator_label='cooperative_self_retire:<binding>')`.
- It is refused for pane and terminal seats (M20), and while another connected top-level binding exists unless
  `--force` (M18, which names the other bindings).
- The idempotency scope `self-retire:<seat>:<operation>` is checked before the live-seat precheck (spike S4).
- Output: one compact row per abandoned item (about 30 B each), plus `retired <seat>`.

Considered: a cap-gated `RetireReason::SelfRetired`. It is deferred, because the wire enum is closed with no
`serde(other)` (src/sync/facts/seat.rs:72-79). The accepted limit is that peers see an operator retire.

**Presence (SA17).** Recommended: named presence follows the pane precedent. A named seat goes offline only when its
bindings end (`liveness_end_event`, ProcessExit, Superseded). It then shows exactly what a pane seat shows:

- "inactive, unavailable since T" in seat list, participants and inspect;
- `offline` to peers;
- one `recipient_unavailable` per episode for ACK and p0 sends;
- overdue lines per SA10.

Three fixes make that hold:

- (b) the named host never arms `baseline_hold_unclaimed` or recovery holds (today one unresolved row silences
  liveness for its whole host, src/store/seats.rs:1037-1047, :2195-2208);
- (c) the named reconcile stamps its fixed structural boot/incarnation, so the C4 carry keeps live named bindings
  available across a daemon restart;
- (d) the synthetic workspace and tab labels render `-`, and the name appears once.

The optional `presence: offline` overdue suffix (`wake=no_wake offline`, 8 B) is adopted for named and hc seats
only, to keep pane bytes unchanged. Considered: idle-time expiry. It was rejected because the approved spec forbids
retirement on heuristics.

**Display and selectors (SA14, D4).** Recommended:

- `NamedHost::seat_labels` returns pane label = name, so displays need no name branch.
- F adds `#[serde(default)] pub name: Option<String>` to `SeatDirectory` (struct at src/protocol/results.rs:1112,
  which has no `name` field today). It carries the name, and an old daemon's reply decodes with `None`.
- A bare selector token reads as a name only when no pane-path reading matches. Exact seat ids always work.
- Auto-creation happens only on seat-acting calls (hooks, send, join, ack), never on reads. Creation prints
  M8. A read (`inbox`, `seat list`, `thread show`) with an unknown name creates nothing.
- Pane-form allocation of a pane whose current named binding hinted that pane is refused with
  M10 (SA9). M10 compares host plus target: a Herdr pane's hint is the bare pane id and a herdr-core
  terminal's is `<socket stem>:<tN>`, built from the allocation's host endpoint. Accepted limit: hints store
  the socket's stem, so two cores with one stem share the form, and the outcome is M10 naming a live seat,
  never a wrong bind.
- Handoff into a named seat is refused by the port answer with M11. Handoff out of a named seat into a Herdr pane
  works when a Herdr host is reachable: the Herdr endpoint resolves lazily for non-Pane sites (SA1). The same shared
  point serves every other path that addresses a Herdr server from a non-Pane site (launch, `launch --new-tab`,
  invite and `seat resolve --pane`, `me init`'s pane read): the claim host (the reserved locator, or a core socket)
  is never dialled or sent as a Herdr server, and with no Herdr host reachable the pane form is refused with M21.
- `thread show`, delivery `from` headers, `seat list`, participants and `seat inspect` render the name through the
  pane label; workspace and tab render `-` (SA17 fix (d)).

Considered: an explicit `seat:` prefix. It was rejected because it burdens every caller (D4).

**Role semantics, use case 3 (SA7, SA8, D3, D9).** Recommended: ACK obligations belong to the role, and the first
display discharges them. Parallel workers use distinct names (`ci-bot-$N`). A `claude -p` or `codex exec` session is
the supported headless shape, and it uses `HERDR_THREADS_SEAT_READONLY=1` when it must not discharge obligations.
Named person seats (`me init` under a name) are refused in v1 with M15 (07 O5). Considered: documenting distinct names only. It was
rejected because use case 3 would silently discharge ACKs (D3).

### Wake and delivery contract

**`WakeCapability` and `idle_wake_on`.** Recommended: `HarnessRegistry::idle_wake_on(harness, WakeCapability,
channel_live) -> Option<IdleWakeOn{Composer, Channel, None}>` in src/harness/registry.rs (07 §4.3). `channel_live` =
`ModChannels::mod_suppressed(seat, now)` (src/scheduler/mod.rs:671-678), which is grace- and stall-aware. A table
test keeps the no-regression rule
`idle_wake_on(h, Composer(HERDR_TUI), c).map(|w| w != IdleWakeOn::None) == has_idle_wake(h)` for every
registration `h` and both `c` (`has_idle_wake` at :1447 stays as the `pub(crate)` oracle), and pins each value:

| `h` | Composer(HERDR_TUI), c=false | Composer(HERDR_TUI), c=true | None, c=false | None, c=true |
|---|---|---|---|---|
| claude | Some(Composer) | Some(Channel) | Some(None) | Some(Channel) |
| codex, hermes, agy | Some(Composer) | Some(Composer) | Some(None) | Some(None) |
| `synthetic_fourth` (test-support) | Some(None) | Some(None) | Some(None) | Some(None) |
| unregistered (`human`, `pi`) | None | None | None | None |

Claude with `c=false` on Herdr is hooks-only Claude, which wakes by the native prompt today, hence Composer.
On a tmux seat the per-target answer is `Composer(TMUX)` only for a harness with a blocked-screen recognizer (claude, agy)
and `None` otherwise (codex, hermes), whose sender reads `wake=no_wake` (`gate_never_types`).

`wake_status` (src/service/dispatch.rs:252) changes three
ways:

- it takes the row's own host, because wake rows gain `host_id` (a join on `seats.host_id`, see Store);
- it looks that host up through `all_for_reads`;
- it maps `None` to `WakeBlock::NoWake`, persisted per pair.

Hooks-only Claude on a named or hc seat classifies as `None`. Considered: a new `ModChannels::has_live`. It was
rejected because it ignores stalls (roast U-D2).

**Rules 5/6 degradation (SA11).** Recommended: named seats, and hc seats until H2, degrade rules 5 and 6. p0 ACK
requests, invitations and warnings arrive on the next turn. Nobody wakes the seat without a live, unstalled mod
channel, and pokes and recipient-side deadline reminders are absent. Rules 1-4 hold: a seat with no wake is never
nudged. The degradation is stated in four places:

- AGENTS.md delivery-contract rules 5 and 6, which gain an exception for hosts with no wake;
- TRUST-POLICY A4;
- one SKILL.md Warnings sentence for `wake=no_wake` (K2), with the `none` meaning change (K3), once per session;
- overdue text that reads `wake=no_wake`.

`seat threshold` on such a seat still records the value and prints M12. Pokes and recipient-side deadline
reminders are not sent to such a seat; a deadline produces only the sender's overdue line. A tripwire test makes it
mechanical. It is keyed on the per-target answer, not the host-level one, because after N2 `NamedHost` answers
`Composer` at host level and `None` only per target. The test enumerates every kind in the implemented set, builds
its host through a test-only constructor, asks `target_wake_capability` for a target with no verified wake path (a
named seat with no tmux hint; an hc terminal), and if any answer is `None`, requires SKILL.md to explain
`wake=no_wake`. N rewrites the existing harness tripwire `idle_wake_classification_of_the_builtins`
(src/harness/registry.rs:1600-1613) in the same leaf: its assertion that every shipped harness wakes stays, and its
doc comment and failure text, which say SKILL.md does not explain `wake=no_wake`, are replaced by a pointer to the
host-side tripwire. Considered: claiming rule 2 is what breaks (the approved spec). That was
wrong (roast U-D4).

**tmux wake for named seats (N2, SA18, D20).** Recommended: tmux is not a fourth host. It is a per-binding wake path
of `NamedHost`.

- **Hint.** `caller_location.rs` reads `TMUX` (socket, server pid) and `TMUX_PANE`. A seat-acting call carries the
  hint `tmux:<socket>:<server_pid>:<pane>` in the resolve command, not in `CallerClaim`. The daemon keeps it in
  memory per binding. That avoids a second migration, and the hint is refreshed by every named resolve and by every
  hook that reports a turn edge (`TurnStateReport.tmux_hint`, verified for the claim's harness fingerprint). The hint and
  the binding's turn state must outlive the routine events of an unattended seat, so a daemon that holds a
  wake-armed binding does not idle-exit (see "Lane registration"); only a crash, an upgrade or an explicit stop
  loses them.
- **Verification.** The daemon runs `tmux -S <sock> display -p -t <pane> '#{pane_pid} #{pid}'`. It requires
  `#{pid}` == server pid, and a nearest-ancestor walk from the binding's fingerprint pid to `pane_pid`. This runs at
  bind and at every wake, under a fresh `ProcessCommand` with a 2 s deadline. A failed check only disables wake for
  that binding. It never moves a seat.
- **Capability.** `NamedHost::wake_capability()` becomes `Composer(ComposerTraits::TMUX{submit_budget: 2 s})`, so
  the lane starts. A new defaulted `HostPort::target_wake_capability(&HostTargetId) -> WakeCapability` (default
  `self.wake_capability()`) returns `Composer` only when the seat's current top-level binding has a verified hint
  and its harness has a blocked-screen recognizer, else `None` (reported `wake=no_wake`). `wake_status` and `no_wake_reports` read the per-target answer.
- **Composer read.** `capture-pane -p -t <pane>` at `#{pane_width}`, fed to the harness's existing `ComposerPolicy`
  (src/harness/adapter.rs:1379). Focus = `pane_active && window_active && session_attached`, under the existing
  focused-composer hold.
- **Typing.** `send-keys -t <pane> -l <wake text>` then `send-keys Enter`, with the existing read-back and
  `clear_own_wake_text`.
- **Busy or idle (D21).** The daemon records hook turn state per binding through a new neutral adapter method
  `turn_state_edge(event) -> Option<TurnEdge>` (default `None` means no tmux wake for that harness). Every
  turn-ending path is an idle edge, because an unattended seat's next hook may never come:
  - `UserPromptSubmit` (or the harness's turn-start event) → busy;
  - `Stop` → idle, and for Claude also `StopFailure` (API errors fire it instead of `Stop`) → idle. Codex and agy
    already register `Stop` (src/harness/codex.rs:114). Claude gains report-only `Stop` and `StopFailure`
    registrations in its adapter;
  - a user interrupt fires no hook in Claude, so a stale busy is cleared by the screen: a new defaulted
    `ComposerPolicy::screen_turn_running(&str) -> Option<bool>` (default `None`) recognizes the harness's running-turn
    status line. When the recorded state is busy for at least 60 s and two composer reads at least 5 s apart both
    give `screen_turn_running == Some(false)`, the daemon records idle (`TurnEdge::ScreenIdle`, diagnostics only).
    A harness whose recognizer answers `None` keeps a stale busy until its next hook (accepted limit, stated per
    harness in TRUST-POLICY N2).
  A daemon restart (crash, upgrade) starts every binding at `Unknown`, which never reads as idle; the next hook of
  that binding sets it.
- **Blocked.** A new defaulted `ComposerPolicy::screen_blocked(&str) -> Option<bool>` (default `None` means no wake)
  recognizes approval and question screens.
- **Wake condition.** A wake needs turn state idle, `screen_blocked == Some(false)`, an empty composer read, and a
  pane that delivers keys to the program: the verification `display -p` format also reads
  `#{pane_in_mode} #{pane_input_off} #{synchronize-panes}`, and any non-zero value (copy-mode or another mode,
  input disabled, synchronized panes) gives no typing for that attempt: it is treated as busy, retried on the
  ladder, and never reported as `no_wake`. Missing evidence never reads as idle.
- **Lane registration.** With `wake_capability() == Composer(TMUX)` the built-in session gets its own wake lane at
  `attach_builtin`, registered with its own pacer (`Lane::Wakes`), with reachability (`attach_wake_pacer`) and with
  Health, exactly as a dialled session's lane is. It is not `first_host_pacer`'s lane.
- **Idle exit.** `ActivityProbe::active` (src/daemon/idle_exit.rs) gains one term: the count of wake-armed named
  bindings, meaning connected top-level named bindings that hold a tmux hint which verified at bind, whose
  `LivenessFingerprint::is_alive` holds. The built-in session answers the count from the same daemon memory that
  holds the hints, and prunes a binding whose fingerprint is dead. So the daemon stays up exactly while a tmux seat
  it could wake is alive, and idle-exits as before once none is (or with only unhinted named bindings). This keeps
  the hint in memory (no migration) and makes its loss a crash or upgrade event, not the routine 10 min idle exit.
- **Claude `Stop`/`StopFailure` (report-only).** The Claude adapter registers `Stop` and `StopFailure` only to feed
  `turn_state_edge`. Their event contract marks them report-only (no delivery, no continuation): stdout is empty
  and the exit code is 0 on every path, so they can never force a continuation. `setup` output and `tests/golden_surfaces.rs` change only by the new
  registration, in the same leaf.
- **Hint flips.** The per-target answer can flip between `Composer` and `None` as a hint verifies or fails. A
  no-wake report row is written only while the answer is `None` and is deleted when a `wake_status` read finds
  `Composer` (see Store), so a sender hears `wake=no_wake` once per no-wake episode.

Delivery-contract check (AGENTS.md rules 1-6) for a tmux-verified named seat:

| Rule | Holds by |
|---|---|
| 1 | Unchanged: default p1, T = 0 |
| 2 | The existing `wake_frontier` over `THRESHOLD_ADMITS_SQL`; the tmux path adds no other wake trigger |
| 3 | Unchanged: turn hooks and the mod channel deliver every pending item |
| 4 | Hook turn state: busy from `UserPromptSubmit` to `Stop`/`StopFailure` or a screen-idle clear, and no typing while busy, blocked, unknown, in a tmux mode or with a draft |
| 5 | Holds while the hint verifies (p0 wakes at every T); degraded to next-turn delivery when it does not (`wake=no_wake`) |
| 6 | `Native`, with tmux as the typist; `Channel` unchanged for Claude with a live mod |

Considered: tmux as a fourth host. It was rejected because pane ids are reused, tmux has no incarnation witness,
and it would compete with named identity (SA18). Also considered: relying on OSC 7501. It was rejected because tmux
drops it (T5, inferred).

**Agent status source rule (D21).** Recommended, as a host-neutral invariant tested per host:

- each host reports agent status with its source: Herdr screen detection, the hc self-reported OSC 7501, or the
  tmux hook turn state plus the screen recognizer;
- null or missing status is `HostUiState::Unknown`, never Idle;
- every wake still needs a composer read, because both Herdr's screen status and OSC 7501 report idle with a draft
  (04 C).

The source appears only in `diagnostics`/`doctor` output (no model-visible bytes). Concretely, HcHost maps
`status: null` to Unknown, which changes 06 §2.2's "idle/done → Idle" only for null. The Herdr path consumes a
reported status only once Herdr exposes `status_source` (upstream ask, follow-on). Considered: treating hc `done`
as idle. It is deferred to H2 (W2).

### hc host (H1)

**JSON-RPC client and wire types.** Recommended: src/host/hc/{mod,rpc,wire}.rs (new), per 06 §1 and §2.2.

- `rpc.rs` is a JSON-RPC 2.0 line client with one connection per call. Its methods are `hello`, `terminal.get`,
  `events.subscribe` (one-shot snapshot) and `doc.get`, with `terminal.run` in H3. `terminal.read` has no reader
  before the H2 composer read (W1), so it is not in this run. The peer witness reuses `LocalEndpointWitness`
  (src/host/continuity.rs), and its peer pid is `HcHost::server_process`.
- `wire.rs` holds tolerant serde types (`HelloReply`, `HcTerminal`, `HcStatus`, `AppDoc`), with no
  `deny_unknown_fields`.
- Calls never pass `caller` (that mutates hc's parent bookkeeping). The client never uses `terminal.list` (it
  prunes, fans out to machines and is unsequenced) and never `doc.set`.
- `enumerate_targets` = `hello` + `events.subscribe{}` snapshot on one connection. It keeps every local address,
  live and finished, and drops only `machine/tN` mirrors. A finished record carries its `reason` and the terminal's
  `keep` flag into the capture, so the closure decision (D10) reads them; only `state=="live"` records are
  reported as present targets.
- `observation_sequence` is the adapter's own counter (K2).

Considered: the long-lived events lane. It was deferred, because 5 s polling is enough (06 K6).

**Probe before attach.** Recommended: `HostFactory` (host_sessions.rs:61-77, closures in src/main.rs:67-80) becomes
`{exists, probe, build}`. `exists(path)` stays the socket precheck (host_sessions.rs:584, :876, :938).
`probe(locator, budget) -> Result<HostKind, ApiError>` is new. `build(locator)` becomes `build(kind, locator)`.

- The probe sends one `{"jsonrpc":"2.0","id":1,"method":"hello"}` line with a short deadline. A reply with
  `core.id` means `Hc`. Anything else falls back to the Herdr `ping`.
- The order is `exists`, then `probe`, then `store.attach_host`. Today `exists` (:584) is followed directly by
  `store.attach_host` (:600-608). The probe holds `attach_lock`, and the attach writes `kind` plus
  `kind_probed_at`. `attach_builtin` calls none of the three.
- A stored kind that disagrees with the probe refuses the attach, and `doctor` reports it.

Spike D1 confirms that a Herdr TUI socket answers or closes, and does not hang. The probe's deadline makes "no
hang" hold whatever the reply. Considered: sniffing after the row
write. It was rejected because a crash leaves a wrongly kinded row (roast U-A4).

**Core id boot.** Recommended: boot = incarnation = `hc-core:<core.id>`, read from `hello` (`HcBoot::from_hello`,
ASCII, at most 128 bytes). The socket is only a dial address. A known locator that answers with a different core id
is a new boot: its seats get C2 holds and are never matched by `tN`. A core restart, stop+start or `hc update` is the
same boot with a newer epoch, so `CarryForward`. Considered: `HC_SCOPE` or the core pid as boot. Both were rejected:
`HC_SCOPE` changes on stop+start, and either choice would unresolve every seat on restart (06 V2, risk 3). Also
considered: "a known core id at a new path is the same host". Not implemented; it is treated as a new host
(P: Part 4, partially adopted H-T2).

**Finish-reason closure (D10).** Recommended: closure follows herdr-core's own finality rule
(crates/herdr-core/src/terminals.rs `Registry::finish` at `dc276b7`) over a coherent capture of the same core id.
Events (`finished`, `removed`) are dirty hints that trigger a capture, never closure on their own. herdr-core's
rule: a finish leaves the terminal `state=="live"` (pending restart) only when `keep` is true and the reason is
one of `holder_lost`, `stopped_by_restart`, `server_stop`, `host_shutdown`, `signaled`; every other finish is
final, the record turns `finished`, and the core never revives it. Ids are monotonic (`Registry::create`), so a
finished `tN` never names another terminal, and finished records are pruned past `MAX_FINISHED` (500), which emits
`removed`.

| Capture shows for the seat's `tN` | `keep` | Reason | Effect on the seat |
|---|---|---|---|
| `state=="finished"` | false (every H3 launch) | any | retired through the pane-close fence (`BeginRetirement{Closed}`) |
| `state=="finished"` | true | `exited`, `killed` | retired through the pane-close fence |
| `state=="live"`, no holder (pending restart) | true | `holder_lost`, `stopped_by_restart`, `server_stop`, `host_shutdown`, `signaled` | unresolved; bindings end `seat_unresolved`; C1 resume reattaches it when the core restarts the terminal |
| absent from the capture (pruned, `removed`) | either | any | retired through the pane-close fence, whatever the seat's state (resolved or unresolved) |
| `state=="live"` with a holder | either | none | nothing (present) |

- Retiring on a final finish or on absence is evidence-based, not heuristic: herdr-core's own record says the
  terminal can never run again, and the id can never be reused. TRUST-POLICY's "never end a binding on heuristic
  evidence" holds.
- An agent that exits inside a live terminal (a shell-rooted `hc run`, the shell still running) is not a terminal
  finish. It ends only that binding (`ProcessExit`); the seat stays resolved on the live terminal.
- An agent that is the terminal's root (the H3 gate shim `exec`s the harness) finishes the terminal when it exits.
  With `keep: false` that finish is final and retires the seat. This is the terminal's closure, not the agent's.
- A failed `hello`, a reply with no usable core id or a record state this build cannot read is an `Err` (the
  observation freezes), and a core id that differs from the seat's boot holds under C2; none of them closes anything.
  Accepted limit: an unreadable record state freezes observation of that whole core until an upgrade.
- The reason vocabulary is herdr-core's: `exited`, `killed`, `signaled`, `server_stop`, `host_shutdown`,
  `holder_lost`, `stopped_by_restart`. An unknown reason on a `finished` record is final (the core already decided
  it); on a `live` record it changes nothing. This follows the version rule: no reason list gates anything, the
  record's `state` decides.

Considered: 06 V1's "finished = closure" for every terminal regardless of how the agent left it. It was rejected
because it retires on agent exit inside a live shell terminal (roast H-T1); the rule above retires only when the
core's own record is final. Also considered: a reason table keyed on invented names over a live-only capture
(this spec's first draft). It was rejected because the reason never reached the decision, the names were not
herdr-core's, and agent-rooted keep:false seats were left unresolved forever (roast design r1).

**Labels `<session>:tN` (D11).** Recommended:

- `HcHost::seat_labels`/`live_topology` take workspace from `herdr.space`, tab from `herdr.tab` and target `tN`.
- Names come from doc `herdr-app`, parsed tolerantly and cached by doc version. An unnamed tab shows its 1-based
  position. The pane label is hc `name`, else none, and never `title`.
- Displays and selectors use `<HC_SESSION>:tN`, or bare `tN` only when exactly one core is attached. An ambiguous
  bare `t1` is refused with the list of candidates.
- Labels are keyed `(host_id, target)`, which F already guarantees.

Considered: splitting `where` on " / ". It was rejected as ambiguous (04 E).

**Discovery.** Recommended: `HcDiscovery` in src/host/hc/mod.rs reads core state and never runs `hc`:

1. List `$XDG_STATE_HOME|~/.local/state/herdr-core/*/core.json`.
2. Derive each socket by the core's rules: `HC_SOCKET` or config, else `$XDG_RUNTIME_DIR/herdr-core/<session>.sock`,
   else `<state>/<session>.sock` (herdr-core docs/security.md:70-75, crates/herdr-core/src/paths.rs).
3. Send `hello`, and require `core.id` to match the core.json id.
4. Dedupe by id.

Discovery is used only when no caller site names a core (plain-shell handoff, peer `place()`). Diagnostic scans
never attach. A core with no resolved seat does not keep the daemon up, and the cap stays 16 (07 §5.3 overrides 05
D14). Considered: `hc server status`. It was rejected because it autostarts a core (04 step 1).

**Standalone install and daemon start.** Recommended, per 06 §6.3 and 07 "Install":

- One state dir for every environment: `$XDG_STATE_HOME/herdr/plugins/herdr-threads`, discovered in SA1's order
  (07 §4.4.1).
- `default_state_dir_to_create` (src/cli/instance.rs:192, beside `default_state_dir` at :172) is extended, not
  replaced. For Named and Terminal sites it returns the default dir, created 0700, only when no Herdr is configured
  (`HERDR_SESSION` and `HERDR_CONFIG_PATH` unset) and no `herdr` is on PATH. A test runs it with a PATH that holds
  no `herdr` and no `HERDR_CONFIG_PATH` (dir created), and with a stub `herdr` on PATH (dir not created).
- `scripts/install.sh` runs `setup` whenever it can (today it skips unless the plugin is linked,
  scripts/install.sh:909-911). It keeps `herdr plugin link` only when `herdr` is present.
- The installer and `setup` write `<state>/node/install.json {"mode":"standalone"}`, so the leftover check
  (src/cli/instance.rs:271-290) passes.
- `setup` bakes `--state-dir`, and no TUI `--host-endpoint` on hc-only machines (src/harness/setup.rs).
- The daemon starts by the existing lazy CLI/hook path with the K5 scrub. There is no daemon in an `hc run --keep`
  terminal.
- The skill gets one environment-neutral sentence plus the hc/ht split (K4, once per session).

Phase split. Use case 1 (a machine without Herdr) needs the Herdr-less install path, and N merges first, so N owns
it: install.sh running `setup` without a linked plugin, the `install.json` marker, `setup` baking `--state-dir`
with no `herdr` on PATH, the leftover-check fix, and the Named-site discovery and state-dir work. With `herdr`
present, install.sh and `setup` behave byte-identically to today. H1 adds only the hc-specific parts: no TUI
`--host-endpoint` on hc-only machines (a core state dir present, no `herdr` on PATH) and the Terminal-site
discovery. Considered: a
herdr-app mod hosting the daemon. It was rejected because of the mod's 30 s exec cap (04 step 8).

**`fake_hc`.** Recommended: src/test_support/fake_hc.rs (new) is an in-process thread serving JSON-RPC lines on a
scratch socket from `test_support::scratch::temp_path`. It serves `hello` (`core{id,name}`, `methods`, `features`),
`server.status`, `terminal.get`, `events.subscribe` (snapshot, seq, `missed`, heartbeat) and `doc.get` from H1, and
`terminal.run` from H3. It records every call, so a test can assert the client never sends `terminal.list`,
`doc.set` or `caller`. Knobs:

- per-terminal state, status (including `null`), turn, pid, foreground and labels;
- per-terminal `keep`, and finishes that apply herdr-core's `Registry::finish` rule (a kept terminal with a
  pending reason stays `live` with no holder; any other finish turns `finished`) with herdr-core's reason strings
  (`exited`, `killed`, `signaled`, `server_stop`, `host_shutdown`, `holder_lost`, `stopped_by_restart`), record
  pruning that emits the `removed` event, and an unknown reason string;
- core id rotation (a wipe) and seq reset (a restart);
- witness change;
- unknown fields and unknown event types.

The screen-text knob, `terminal.read` and the `blocked`/`status_mismatch` send refusals model the H2 composer read
and send, and are not built in this run (see Out of scope).

The 0.1.15 OpenRPC document is vendored as historical evidence at `tests/fixtures/hc/hc-0.1.15.openrpc.json`, copied
from herdr-core `schema/hc.openrpc.json` at `dc276b7`. A contract test asserts that every method and param HcHost
and fake_hc send exists in it. It is not a version gate: a newer schema with extra fields passes. Because the fake
runs in-thread, suites without an in-process daemon go into `tests/combined.rs`. One test needs a
separate process: the hc half of the inherited-name refusal (E1) needs a core server whose environ carries the
name, and a multithreaded test binary cannot `set_var` safely. For it, fake_hc also builds as a `fake-hc` binary
(`[[bin]]` with `required-features = ["test-support"]`) that serves the same code on a given socket. The test
starts it through `test_support::spawn` with `HERDR_THREADS_SEAT_NAME` in its env, so `server_process` is that
child. Every other suite uses the in-thread mode.

### hc launch (H3)

**`CreateTargetPort` and journal records.** Recommended: `CreateTargetPort` beside `CreateTabPort`
(src/ports.rs:5536), per 06 §2.6.

- Types: `CreateTargetRequest`, `Placement{NewTab{space}, NewTerminal{parent}}`,
  `HostWitness{HerdrSocket, HcCore{core_id, peer}}`, and `CreatedTarget` with `validate()` dispatching on the
  witness.
- `NativeCli` implements it by delegating to `create_tab`.
- `BootstrapPlan`/`BootstrapProgress`/`BootstrapAttachment` gain V3 records carrying `CreatedTarget` in
  src/protocol/handoff.rs. V1 and V2 bytes stay frozen (06 K3).
- `src/cli/host_client.rs` (new) holds the CLI-side launch handle.
- The topology_handoff*, topology_runtime, handoff, peer_launch and peer_launcher code becomes generic over the
  port.

Considered: widening `CreatedTab`. It was rejected because the journaled records use `deny_unknown_fields` with
frozen bytes.

**Gate shim and env blanking.** Recommended: `terminal.run` starts
`herdr-threads internal launch-gate --bootstrap <compound>:<attempt> -- <argv>` (src/cli/launch_gate.rs, new). The
shim:

1. reads `HC_TERMINAL_ID`/`HC_TERMINAL_SOCKET`;
2. removes `PANE_SCOPED_ENV_VARS`, the remaining `HERDR_*` vars, `HERDR_THREADS_SEAT_NAME`, each adapter's
   `CallerEnvShape` session/pid vars, and the new neutral `child_scrub_vars()` (adapter method, default empty);
3. sets the caller's config roots, so a leaked `CLAUDE_CONFIG_DIR` cannot win;
4. waits, bounded, until the daemon has resolved its `(core, tN)` as the frozen recipient
   (`ResolveBootstrapSeat`, src/cli/topology_handoff.rs);
5. `exec`s the harness.

`exec` keeps the pid (L2), which keeps TRUST-POLICY's two-phase launch invariant. Per-harness argv comes from the
new adapter method `host_launch_argv(HostKind) -> Vec<String>` (default empty). Codex adds `--no-daemon` for `Hc` in
src/harness/codex.rs (04 step 6). Considered: `terminal.run` with the harness argv directly. It was rejected because
`terminal.run` cannot unset leaked env (06 V6).

**`no_parent` / `no_notify` (D7, D8).** Recommended: `terminal.run` passes `no_notify: true`,
`outlive_parent: true` and `no_parent: true` (all three, and `keep`, are params of `terminal.run` in the vendored
0.1.15 schema), omits `caller`, sets `keep: false`, and
uses no hc `name`. Because `keep` is false, any finish of a launched terminal is final, so its seat retires
through the D10 rule when the worker exits (the harness is the terminal's root after the shim's `exec`); no
launched seat is left unresolved. Labels are `{herdr.space, herdr.tab, ht.bootstrap, ht.label}`. The prompt goes through
herdr-threads, never `first_message`. The `ht.bootstrap` label is a recovery hint only and never reclassifies a
recorded unknown (TRUST-POLICY "Bootstrap creation attempts"). Outcomes:

- a JSON-RPC error before the record exists is `NotSubmitted`;
- `spawn_failed`, a timeout or a disconnect is `OutcomeUnknown`.

User-made hc children and their notes are the user's configuration (D8). Considered: hc defaults. They were rejected
because the worker would die with its parent and `<hc>` notes would be typed into the coordinator (roast H-D2).

**CLI and placement.** Recommended:

- `handoff`/`launch --space <app space>` (`herdr.space=<id>` + fresh `herdr.tab`), `--new-tab`, and `--new-terminal`
  (no app labels, parent = caller).
- In-place `--pane tN` is refused through `native_launch_capability() == Unsupported`.
- `internal peer-target-create`, with `peer-tab-create` kept as an alias for one release (O6).

**Cap-gated `LaunchTarget`.** Recommended: peer `LaunchTarget::NewTab{space,label}` maps onto hc app spaces with no
wire change. A new variant `LaunchTarget::NewTerminal{label}` in src/sync/frame.rs carries
`// cap: PEER_LAUNCH_TERMINAL_CAP` with a `pub const PEER_LAUNCH_TERMINAL_CAP` there. It is sent only to peers
advertising the capability, and `scripts/check-wire-compat` grows its baseline only by the new type. Considered: no
remote terminal launch. It was rejected because D7 includes a cap-gated remote `LaunchTarget`.

### Peer wire

**`display_name` only.** Recommended: `seat.created.display_name` (already baselined, src/sync/facts/seat.rs:20-28)
is set from `seats.name` in `ensure_published` (starts at src/store/seats.rs:194). On receive it is validated, and an invalid
value is stored as NULL. `SeatStatusEntry.name` is dropped (SA13, D13). `remote_seat_status` accepts entries only
for already materialized seats (src/store/remote_seat_status.rs:88-92), names never change, and remote `seat_labels`
publish pane = name in the existing `SeatStatusEntry.pane` (src/sync/frame.rs:412-430). The display is `name@node`.
`cooperative_named_seat` names the named-host observation, while binding provenance stays the caller's (SA13, D15,
02 C4). Considered: the approved spec's `SeatStatusEntry.name`. It was rejected as redundant, and it would have
needed a baseline exception.

**Local protocol skew.** Recommended:

- `SeatsQuery.host: Option<HostLocator>` (a `deny_unknown_fields` struct, src/protocol/commands.rs:412-414) and
  the caller host on `ResolveThreadQuery` (also `deny_unknown_fields`, :1199-1201) are sent only when
  `sent_scope()` answers, so pane bytes are unchanged and an old daemon never sees them from a pane. Thread
  resolution from a named or terminal site is host-scoped by that field (07 §6.2).
- `Command::HostTopologies` is gated on a new local capability `topology.host_topologies_v1` (in
  src/protocol/capabilities.rs, advertised from F). Without it the CLI renders local labels as `-` and proceeds, so
  a new CLI against a pre-F daemon never fails a pane read (`seat list`, participants, delivery headers). F adds a
  skew test for that.
- `SelfRetire` and `LocateCaller` are new commands that only new sites send (a named seat, an `Ambiguous` caller).
  An old daemon gives the existing upgrade error.
- `SeatDirectory.name` and the `WakeStatus` row's host id are `#[serde(default)]`, so they are skew-safe.
- Local capabilities: `seat.host_sites_v1` (N) gates Named sites and `seat.terminal_sites_v1` (H1) gates Terminal
  sites. Two caps keep a new CLI from sending a Terminal site to an N-only daemon. Without the cap the CLI prints
  `herdr-threads: daemon predates named/terminal seats; restart it` (M13).

07 §6.2 had one cap that H1 extends. That shape is split here because N and H1 land separately.

### Install, discovery and daemon start

**Instance discovery with a name (SA1, D2).** Recommended: the order in 07 §4.4.1:

1. the `--state-dir` flag;
2. `HERDR_PLUGIN_STATE_DIR`;
3. the hook-baked `--state-dir`;
4. today's discovery when `HERDR_ENV`/`HERDR_SESSION`/`HERDR_CONFIG_PATH` is present, including the
   `herdr plugin list` query (src/cli/instance.rs:168-198);
5. the existing default dir.

The default dir is created only when no Herdr is configured or on PATH. Otherwise discovery makes one
`herdr plugin list` query, or refuses and asks for `--state-dir`. `doctor` flags two live stores for one user. The
Herdr host endpoint resolves lazily for non-Pane sites. Considered: an unconditional default dir for named callers.
It was rejected because it splits stores beside Herdr. ht-mf0w (per-XDG fast defaults for named Herdr sessions)
stays follow-on.

### TRUST-POLICY and AGENTS.md amendments

**TRUST-POLICY.md, in the same commit as the behaviour each covers (07 §9).**

| Phase | Amendment |
|---|---|
| F | Vocabulary (TRUST-POLICY.md:112-148, Herdr boot at :121-123): "host boot / incarnation" defined per host. "Target" becomes "a host target: pane, name or terminal". A1 addendum, F part: two-family nearest-ancestor guard, Herdr candidate only; refusal when none verifies; a herdr-core process on the walk is a wall the Herdr candidate cannot verify across. Accepted limit (F/N): a process with unreadable argv is not a wall. K5: the autostarted daemon is reparented, so no terminal sweep reaches it. |
| N | A3: `cooperative_named_seat` (named-host observation) and `cooperative_self_retire` (local `operator_label`). Named rule: a named observation is not host evidence, never closes by absence, and has the cross-kind refusal. New C1 row "named continuity". Restore rule (SA4): a restored name that collides is held nameless (its `seat.created.display_name` is the held name) and gets the name only when the existing seat retires; no merge, no pick. Role ACK semantics and read-only bindings (SA7). Inherited-name refusal as cooperative consistency (D1). Live-holder guard (SA16). Named presence (SA17). Accepted limits: a name inherited through a non-host parent (only observable); same-user use of the variable; typo seats; a per-command name equal to the host server's own name is refused like an inherited one; self-retire replicates as Operator; server env unreadable means the check is skipped. A4: named seats have no composer wake, rules 5/6 are degraded, the threshold is inert. |
| N2 | A4: tmux composer wake for a named seat only with a verified hint (server pid + nearest ancestor), re-verified at every wake; turn state from hooks; a missing status is never idle. Every turn-ending path clears busy: `Stop`, `StopFailure`, or a stale busy (60 s) cleared by two screen reads that show no running turn. A daemon holding a wake-armed tmux binding does not idle-exit. A pane in a tmux mode, with input off or synchronized is never typed into. Accepted limits: no server-side draft guard (read-back/clear only); a harness with no running-turn recognizer stays unwoken after a user interrupt until its next hook; the hint and turn state live in daemon memory, so after a crash, upgrade or explicit stop there is no wake until that binding's next turn edge. |
| H1 | Vocabulary hc row (core id boot, new id = new boot = C2 holds). Closure (D10): herdr-core's `(keep, reason)` finality over a coherent capture; a final finish (any reason with `keep: false`; `exited`/`killed` with `keep: true`) or record removal retires through the pane-close fence as evidence, never heuristics; a kept terminal pending restart unresolves, and the C1 structural row reattaches it when the core restarts the same `tN`; agent exit inside a live terminal ends only the binding. A1 addendum in full, plus limits: double-forked processes and Codex daemon mode are unverifiable with two families; single-family stale env is accepted. A4: hc has no composer wake until upstream. Accepted limit: a wiped or cloned core means C2 holds. |
| H3 | `managed_launch` extended to host-guarded launch (gate shim, root-pid fingerprint). C4 staging covers the hc launch. The `ht.bootstrap` label is hint-only. |

No adversarial verification is added. Every decision is made in the daemon against the canonical view (A2).

**AGENTS.md.**

- N: delivery-contract rules 5 and 6 gain the no-wake-host exception. "What we do today" gains lines for persisted
  no-wake reports, the inert threshold, named seats and `HERDR_THREADS_SEAT_READONLY`.
- N2: rule 6 `Native` reads "the host (Herdr, or tmux for a verified named seat) types the wake marker into an
  empty composer". "What we do today" gains a tmux line: named seats with a verified tmux hint wake by tmux
  `send-keys` into an empty composer, gated on hook turn state and `screen_blocked`, with the code paths in
  src/host/named.rs and `turn_state_edge`.
- H1: a "What we do today" line for hc seats, and a test-isolation line for the isolated-core helper (never
  `hc server status`).
- Leak checks: `scripts/check-no-leaked-processes` recognizes `herdr-core serve`/`hold`, and a tmux server started
  with `-L ht-test-*` (H1, N2).

### Model-visible strings

Every new model-visible string of this run, per token rule (f). Bytes are the fixed part; `<...>` fields add their
own length. "CLI" is one stderr line; "hook" is hook context; "check-in" is a lifecycle check-in or compact
re-entry. Acceptance criteria cite these rows by id, and a golden test per row pins the exact bytes.

| Id | Exact text | Bytes | Surface | Phase |
|---|---|---|---|---|
| M1 | `herdr-threads: runs in herdr-core terminal <tN>; not supported yet` | 62 + tN | CLI, hook stderr | F (removed in H1) |
| M2 | suffix `; set HERDR_THREADS_SEAT_NAME` on M1 and M3 | 29 | as M1, M3 | N (removed in H1) |
| M3 | `[herdr-threads] guard: runs in herdr-core terminal <tN>, not Herdr pane <pane>; no seat bound here` | 88 + fields | check-in (per "TUI leak guard") | F (removed in H1) |
| M4 | `herdr-threads: env names pane <pane> and terminal <session>:<tN>; neither is an ancestor of this process; unset the stale one` | 106 + fields | CLI, hook stderr | H1 |
| M6 | `herdr-threads: HERDR_THREADS_SEAT_NAME=<name> inherited from the <host> server; use another name per command or restart the server without it` | 129 + fields | CLI error | N (`Herdr`), H1 (`herdr-core`) |
| M7 | `herdr-threads: seat <name> is held by live pid <pid>; use another name` (code `seat_name_in_use`) | 59 + fields | CLI error, hook stderr | N |
| M8 | `herdr-threads: created seat <seat> (<name>)` | 31 + fields | CLI | N |
| M9 | `herdr-threads: seat <name> now bound from <loc1> and <loc2>` | 41 + fields | CLI | N |
| M10 | `herdr-threads: pane <pane> runs named seat <name>; address it by name` | 57 + fields | CLI error | N |
| M11 | `herdr-threads: named seat <name> has no pane to launch into` | 53 + name | CLI error | N |
| M12 | `herdr-threads: <seat> has no idle wake; threshold has no effect` | 57 + seat | CLI | N |
| M13 | `herdr-threads: daemon predates named/terminal seats; restart it` | 63 | CLI error | N |
| M14 | `herdr-threads: HERDR_THREADS_SEAT_NAME cannot be combined with <flag>` | 63 + flag | CLI error | N |
| M15 | `herdr-threads: named person seats are not supported; unset HERDR_THREADS_SEAT_NAME` | 82 | CLI error | N |
| M16 | `herdr-threads: HERDR_THREADS_SEAT_NAME is not a valid seat name (1-64 bytes of A-Za-z0-9._-)` | 92 | CLI error | N |
| M17 | `herdr-threads: HC_TERMINAL_ID is set without HC_TERMINAL_SOCKET; cannot locate the terminal` | 91 | CLI error | H1 |
| M18 | `herdr-threads: seat <seat> has another connected binding: <bindings>; use --force` | 65 + fields | CLI error | N |
| M19 | overdue suffix ` offline` after `wake=no_wake` (named and hc seats only) | 8 | overdue warning | N |
| M20 | `herdr-threads: seat retire --self is for named seats only` | 57 | CLI error | N |
| M21 | `herdr-threads: pane <pane> has no reachable Herdr host; pass --host-endpoint` (`a new tab` for `launch --new-tab`) | 69 + pane | CLI error | N |
| — | `seat retire --self` rows: one compact row per abandoned item plus `retired <seat>` | about 30 each | CLI | N |
| K1 | SKILL.md: ``Outside Herdr, set `HERDR_THREADS_SEAT_NAME=<name>` per command, never in a shell profile or a server env.`` | 106 | skill, once per session | N |
| K2 | SKILL.md Warnings, after the `none` entry: `` `no_wake`: the seat's host cannot wake it; it sees the item on its next turn.`` | 77 (+1 space) | skill | N |
| K3 | SKILL.md Warnings: `` `none`: no native wake (a mod-channel seat wakes without one).`` becomes `` `none`: no native wake was needed (a mod-channel seat wakes itself).`` | +6 | skill | N |
| K4 | SKILL.md: ``herdr-threads works alike in a Herdr pane, a named process and a herdr-core terminal: `hc` runs terminals, `herdr-threads` carries messages.`` | 140 | skill, once per session | H1 |

No string is added to per-message framing, wake prompts or quiet-turn hook output. M3 is the only new hook-context
string, and it rides check-ins only.

### Testing and evidence

**Per-phase gates.** Recommended: each phase's exit gate is the following.

- **Per-change checks:** `nice cargo clippy --locked --all-targets --all-features -- -D warnings`,
  `scripts/check-harness-neutral` (no new allowlist entry), `scripts/check-wire-compat` (no baseline line in
  F/N/N2/H1; one new type in H3).
- **Before the phase's merge into the run branch:** `nice scripts/check-default-features`, plus an unfiltered
  `nice cargo nextest run --locked --all-targets --all-features` under 5 minutes with `HT_LEAK_RUN_ID` exported,
  then `scripts/check-no-leaked-processes --run-id "$HT_LEAK_RUN_ID"`.
- **Test placement:**
  - suites without process-wide state go in `tests/combined.rs`;
  - in-process daemon suites get a `[[test]]` with `required-features = ["test-support"]`;
  - tests that must not overlap get a nextest group in `.config/nextest.toml`;
  - children are spawned only through `test_support::spawn`, and temp paths come only from
    `test_support::scratch::temp_path`;
  - tests that need Herdr use an isolated named Herdr test session (`scripts/lib/isolated-herdr.sh`), with scratch
    HOME, CLAUDE_CONFIG_DIR and CODEX_HOME.

**F's behavioural gate (artifact-based, quota-free).** Before F merges into the run branch, F's integrator runs
`nice scripts/herdr-release-smoke` (hook-running stand-in Codex in an isolated Herdr server; core features, launch
and every handoff target, wake and threshold, busy agent) against the newest release in `tests/herdr-releases.tsv`,
plus `python3 scripts/wake-e2e.py --self-test`. Their output goes into the run report as F's evidence. Together with
the golden surfaces and the precedence matrix this shows the hook.rs refactor changed nothing observable, before N
and H1 build on it.

**Wake e2e (run-final gate).** Recommended: the real-agent Herdr TUI wake e2e is a separate run-final gate, not
F's. The run's top-level session runs it from a Herdr pane on the final tree, not per leaf, and its evidence
directory (`report.md`, `results.json`, pane captures) is the artifact. It uses
`--codex-auth-home ~/.aisw/profiles/codex/codex-2` (read-only copy of `auth.json`). On Linux, Claude needs
`CLAUDE_CODE_OAUTH_TOKEN` or `ANTHROPIC_API_KEY`. If neither is available, the Codex half runs and the Claude half
goes to remaining evidence. Implementer leaves run the `--self-test` only. The hc wake e2e stays opt-in until H2 (D18
O9).

**Spikes and how each is answered in this run.**

| Spike | Question | Answered by (this run) | Gate |
|---|---|---|---|
| S1 | Named allocation/enumeration race under the lifecycle fence; restart `CarryForward` | Store/reconcile test (`FakeHostWorld`, src/test_support/fake_hosts.rs) | N |
| S2 | First empty named publication clears holds without `baseline_hold_unclaimed` | Store test | N |
| S3 | Mod `watch` inherits the session env, never stops on `no_pane` for a named site | `watch` integration test with a Named site; node test of `integrations/claude/mod/hooks/register.js` spawn env | N |
| S4 | Self-retire idempotency replay scope | Daemon test | N |
| S5 | Restored named seat vs a newer live seat of that name; two-kind restore (TUI + fake hc) | Store test; the two-kind half runs in H1 once `fake_hc` exists | N (named half), H1 (two-kind half) |
| S6 | `doctor` on never-settled `wake_work` of named seats | Daemon test | N |
| E1 | Read the host server env: socket peer pid + `/proc/<pid>/environ` / `KERN_PROCARGS2` | Linux, gate: the `/proc` environ reader is tested against a child spawned through `test_support::spawn` with a known env (N), and the hc path against the `fake-hc` child process with the name in its env (H1). Linux, live: the same check against an isolated Herdr test session is run and recorded when the helper starts; otherwise it goes to remaining evidence. macOS: `KERN_PROCARGS2` parser unit-tested on a synthetic buffer, behind `cfg(target_os="macos")`, returning `None` on any failure. The live macOS run goes to remaining evidence | N (Herdr reader), H1 (hc) |
| D1 | Herdr TUI socket reply to a JSON-RPC `hello` (must not hang) | Gate: a probe test against `test_support::fake_herdr`, plus a bounded-deadline test where the peer never replies, which proves "no hang" whatever the reply. Live: the isolated Herdr capture becomes a `tests/fixtures/` case when the helper starts; otherwise it goes to remaining evidence | H1 |
| H-C1 | Codex hooks in hc with and without `--no-daemon`: does the ancestor guard see them? | Fallback: `LocateCaller` tests over fake process trees (daemon-mode Codex modelled as a reparented process: two families refuse, one family accepted unverified). The live real-core run goes to remaining evidence. Off the critical chain | H1 (fallback only) |
| T1 | `capture-pane` text vs Herdr detection text under each `ComposerPolicy` | Live: replay captured screens from `tests/fixtures/` into a pane of a `tmux -L ht-test-<uuid>` server and compare `ComposerRead`s | N2 |
| T2 | idle/working/blocked recognition, including `StopFailure` and an interrupt with no `Stop` | Hook-driven turn-state tests plus `screen_blocked` and `screen_turn_running` fixture tests; `StopFailure` clears busy; an interrupt clears busy only through the screen rule, and with a `None` recognizer asserts no wake | N2 |
| T3 | Draft safety with no server-side guard | Live tmux: a draft present means no typing; read-back mismatch means clear and no submit | N2 |
| T4 | Stale `TMUX_PANE` in nested Herdr/tmux is refused | Live: nested isolated Herdr session + private tmux; verification fails, so wake is disabled and the seat is not moved | N2 |
| T5 | tmux discards OSC 7501 | Live: `printf` OSC 7501 in a private tmux pane; record the passthrough result | N2 |
| L1 | Unknown `herdr.tab` id makes a new app tab | herdr-app is macOS only. H3 sets labels per 04 E's doc shape and tests against fake_hc. The live check goes to remaining evidence | H3 (fallback) |
| L2 | The shim's `exec` keeps `terminal.pid` | Real-core smoke if available; else fake_hc plus a read-only reading of herdr-core `crates/herdr-core/src/api/run.rs` | H3 |
| L3 | `outlive_parent` + `no_notify` stop every `<hc>` note | Real-core smoke if available; else read-only reading of `crates/herdr-core/src/notes.rs` and `api/run.rs` | H3 |
| L4 | Codex `--no-daemon`; the Linux socket path | Socket path from herdr-core docs/security.md:70-75 and `paths.rs`. Codex `--no-daemon` from the hc skill (04 step 6). The live Codex run goes to remaining evidence | H3 |
| W1-W5 | hc wake | Not in scope (H2) | none |

**Real-core smoke (two leaves).** Recommended: (a) the schema-conformance test below always ships. (b)
`scripts/hc-smoke` (new) plus `tests/herdr-core-releases.tsv` ship only if the implementer can fetch and verify the
herdr-core 0.1.15 manifest (the version of the vendored schema). Otherwise leaf (b) closes as follow-on with no TSV
row; a placeholder or unverified checksum is never committed. `scripts/hc-smoke` works as follows.

- **Download.** It downloads the herdr-core 0.1.15 Linux release from
  `https://releases.herdr.dev/core/<ver>/manifest.json`. The sha256 is pinned in a new `tests/herdr-core-releases.tsv` (version, asset, sha256), and the download is
  checksum-verified. It is cached outside the repo, in `~/.cache/herdr-threads/herdr-core-releases/<ver>/`, like
  `scripts/herdr-release-smoke`, so git never sees it.
- **Isolation.** It runs under `env -i` with a private `HC_SESSION=ht-smoke-<uuid>` and scratch `XDG_STATE_HOME`,
  `XDG_RUNTIME_DIR` and `XDG_CONFIG_HOME`. It passes `HOME`, `USER`, `LOGNAME`, `SHELL`, `TMPDIR` and `PATH`.
- **Scope.** It never runs `hc server status`. It starts the core only through `hc run --bg` under the private
  session.
- **Checks.** It drives HcHost against it: attach, probe, observation, labels, the D10 rule (a `keep: false`
  `exited` retires; a `keep: true` `stopped_by_restart` across `server.stop` + restart unresolves and reattaches),
  the K5 scrub including the reparented daemon, and L2/L3.
- **Cleanup.** It stops the core with `server.stop` over the private socket, and verifies that no
  `herdr-core serve`/`hold` process of that session remains.

It is never in the default suite. **Leaf (a), always shipped, and the fallback when the download is refused or
offline:** a read-only schema-conformance test, `tests/host/hc_schema_conformance.rs` in `tests/combined.rs`. It validates every fake_hc
reply against the result schema of its method in the vendored `hc-0.1.15.openrpc.json` (required fields present,
types match, extra fields tolerated). It also validates the event notification shapes against herdr-core
`schema/README.md`. The run report records which branch ran.

## Phases and acceptance

Dependency order: **F** lands first. **N** and **H1** proceed in parallel once F's seam leaves are merged into the
run branch, and N integrates first (D17), so H1 rebases onto N before its own integration. **N2** follows N. **H3**
follows H1. "Lands" or "merged" means merged into the run branch; the coordinator merges the finished branch to
`main`.

F's leaves, in order. The seam, which N and H1 block on, is F1-F5 and F7-F9 merged. F6 may land in parallel with
N and H1; their display leaves (N.5, H1c labels) block on it.

| Leaf | Content | Seam |
|---|---|---|
| F1 | Bring `main` in; `scrub_env` `pub(crate)`, `child_env_scrub_list()`, the K5 scrub | yes |
| F2 | `HostKind{Herdr,Unknown}`, the six `HostPort` answers, the G4 implemented-kinds set | yes |
| F3 | NodeDelta `0012_host_environments.sql` | yes |
| F4 | `HostSessions` builtin slot, `attach_builtin`, `all_for_reads`, `Option` wake lane, the 07 §5.3 audit filters | yes |
| F5 | `idle_wake_on` and its table test; `wake_status` per-row host | yes |
| F6 | `HostTopologies` (+ capability), `SeatDirectory.name`, K4 labels, removal of the CLI label `NativeCli` sites | no |
| F7 | `caller_location.rs` (Pane), the env-read check, `env_assignments` recovery hints, hook gates through `locate(Hook)` | yes |
| F8 | `LocateCaller` (Herdr candidate), transport peer pid, M1 and M3 | yes |
| F9 | `SeatsQuery.host` and `ResolveThreadQuery` caller host (unsent), the CLI site-capability check mechanism | yes |

TRUST-POLICY F rows land in the commit of the behaviour they cover (F7, F8).

### Phase F: shared foundation (Herdr-only behaviour)

Scope:

- Bring `main` into the run branch. Reuse `lifecycle::scrub_env` and `host::PANE_SCOPED_ENV_VARS`, make `scrub_env`
  `pub(crate)` and add `host::child_env_scrub_list()` (run constraint 6).
- src/ports.rs: `HostKind{Herdr, Unknown}`, `WakeCapability`, `ComposerTraits::HERDR_TUI`, `ArchivalEvidence`,
  `ClosureAuthority`, and the six defaulted methods. The dispatcher budget reads `submit_budget`
  (src/notification/dispatch.rs:39).
- src/service/host_sessions.rs: builtin slot, `attach_builtin`, `all_for_reads`, capability-gated wake lane,
  `Option` wake consumers, `startup_primary` filter. src/app.rs: `first_host_pacer` on `Option`, discovery gate by
  kind.
- The 07 §5.3 audit filters in src/service/{dispatch,archival,peer_launcher,seat_publisher,remote_resolver}.rs and
  src/daemon/idle_exit.rs. `operator_host` uses the "no host attached" text.
- src/harness/registry.rs: `idle_wake_on` plus the table test (see "WakeCapability and idle_wake_on").
- `wake_status` takes the per-row host (a join on `seats.host_id`; the `WakeStatus` row's host id field is
  `#[serde(default)]`), through `all_for_reads`.
- Store: NodeDelta `0012_host_environments.sql` with every column and table above (next free slot, see Store).
  `kind`/`kind_probed_at` are written for Herdr hosts. The G4 implemented-kinds set is `{herdr}`, and the
  fail-closed unknown kind is surfaced in `doctor`.
- Labels and topology from the daemon (fixes K4): `Command::HostTopologies` gated on `topology.host_topologies_v1`,
  `SeatDirectory.name` (new field, unset in F), `SeatSummary.directory`/`ParticipantLocation.remote` filled for
  local seats from `read_labels` keyed `(host_id, target)`. CLI selectors pool `HostTopologies`. Updated callers:
  src/cli/human.rs, src/cli/peer_locations.rs, src/cli/seat_resolution.rs.
- The 13 `NativeCli::new` sites in src/cli, classified:

  | Site | Use | F |
  |---|---|---|
  | src/cli/mod.rs:986, :995, src/cli/follow.rs:230 | label reads | removed; labels come from `HostTopologies` |
  | src/cli/mod.rs:706 (`run_parsed_in_pane`, amended in ht-kqrt.9: the lazy claim-host adapter now serves only pane-selector topology and the caller's `current_pane`, no label read), :398 (`pane_agent`), :3784 (`located_pane_seat`), src/cli/hook.rs:1631 (`locate_seat_for`) | Pane-site observation of the caller's own Herdr host (moved pane, agent probe) | kept, Herdr-only, reached only from `CallerSite::Pane` |
  | src/cli/handoff.rs:2264, src/cli/peer_launch.rs:48, src/cli/mod.rs:3990 (`run_launch`), src/cli/topology_runtime.rs:388, :954, :1047 | launch and handoff | kept in F; made generic over `CreateTargetPort` in H3 |

- src/cli/caller_location.rs: `CallerEnv`, `Located`, `CallerSite` (Pane variant, hc recognition for the guard),
  empty-value filter. src/cli/mod.rs, hook.rs and watch.rs consume `CallerSite` (watch.rs only for its target; see
  "Caller location"). The `HERDR_ENV` read (hook.rs:257-260) moves into `caller_location.rs`, and the gates that
  read its result (hook.rs:1463, :1790, :3485, :4019, :4393) go through `locate(Hook)`.
  `CallerSite::env_assignments` builds the three handoff recovery hints (src/cli/handoff.rs:1930-1933,
  src/cli/handoff_delivery.rs:1397-1400, :1816-1822).
- src/service/locate.rs `LocateCaller` (Herdr candidate) and the transport peer pid (src/daemon/transport.rs:430);
  the TUI leak guard M1 and the hook guard line M3.
- `SeatsQuery.host` and `ResolveThreadQuery` caller-host fields (unsent in F). The CLI site-capability check
  mechanism (no caps advertised yet).
- K5 scrub: `spawn_detached` removes `child_env_scrub_list()`: the `ENV_VARS` lists, `HC_SESSION` and
  `HERDR_THREADS_SEAT_NAME`, and reparents the daemon through `internal daemon-detach` (see "Env scrub").
- The `LocateCaller` terminal-root wall (`host::foreign_terminal_process_names()`) and the start-stamped cache key.
- TRUST-POLICY F rows.

Acceptance:

- [ ] The `idle_wake_on` table test passes: `idle_wake_on(h, Composer(HERDR_TUI), c).map(|w| w !=
  IdleWakeOn::None) == has_idle_wake(h)` for every registration `h` and both `c`, and every cell of the table in
  "WakeCapability and idle_wake_on" holds.
- [ ] A test asserts `attached()` never contains a session attached through `attach_builtin` (test fake).
- [ ] Two fake hosts that both own target `t1` (`FakeHostWorld`) show distinct labels in `seat list`,
  `thread participants` and delivery headers.
- [ ] A test (or a `tests/combined.rs` source scan) allows `NativeCli::new` in src/cli only at the kept sites in
  the table above, and fails on any other.
- [ ] The env-read source scan (see "Caller location") passes: no caller-location variable literal in src/cli
  outside `caller_location.rs`.
- [ ] A new CLI against a daemon without `topology.host_topologies_v1` (skew test) prints `seat list`,
  participants and delivery headers with `-` labels and exits 0.
- [ ] A precedence matrix test (pane × `HERDR_ENV` × `HC_TERMINAL_ID` × empty values × missing socket) passes. With
  both `HERDR_PANE_ID` and `HC_TERMINAL_ID` set, the seat binds only when the Herdr candidate verifies, else M1 is
  printed and nothing binds.
- [ ] Nested tree (fake `ProcessInfo`): pane shell → living `hc` client → `herdr-core serve` → `herdr-core hold` →
  agent, with both families set. The walk crosses the wall, the Herdr candidate does not verify, M1 is printed and
  nothing binds. The same tree with the `hc` client gone (core reparented to init) also refuses. A plain pane tree
  (pane shell → agent) still binds the pane.
- [ ] `LocateCaller` cache: a subject pid reused with a different start stamp and the same candidates gets a fresh
  walk, not the cached verdict; a subject with no readable start stamp is never cached.
- [ ] The hook guard line M3 appears on a lifecycle check-in and on compact re-entry, and on no per-turn hook. A
  harness stand-in whose `OutputPolicy` is `guidance_every_turn` with no lifecycle event (the Hermes shape) gets M3
  exactly once per binding across several delivering hooks, and once more after a daemon restart (the accepted
  limit).
- [ ] The three handoff recovery hints for a Pane site are byte-identical to the pre-F output (golden test).
- [ ] A daemon spawned with `HC_SCOPE`, `HC_TERMINAL_ID`, `HC_TERMINAL_SOCKET`, `HC_SESSION`,
  `HERDR_THREADS_SEAT_NAME` and every `PANE_SCOPED_ENV_VARS` name in its parent env has none of them in
  `/proc/<pid>/environ`.
- [ ] Reparenting: a daemon autostarted by a still-living parent whose env carries `HC_SCOPE=<token>` (a
  `test_support::spawn` child that stays alive) has a `ppid` that is neither that parent nor any of its
  descendants, and a test copy of herdr-core's `members()` rule (env carriers plus their `ppid` descendants, over
  `/proc`) run for `<token>` while the parent lives excludes the daemon pid. The intermediate has exited and was
  reaped.
- [ ] The migration applies on a populated fixture store. Existing Herdr rows read `kind='herdr'` and are re-probed
  (`kind_probed_at IS NULL`) on reattach. A row with an unknown kind is never attached and is named by `doctor`.
- [ ] A session whose port returns `WakeCapability::None` starts no wake lane (test fake), and every `wake()`
  consumer compiles against `Option`.
- [ ] Hook outputs for every builtin harness are byte-identical to the pre-F golden surfaces
  (`tests/golden_surfaces.rs`), except where the guard applies.
- [ ] `scripts/check-wire-compat` passes with no new baseline line. `scripts/check-harness-neutral` passes with no
  new allowlist entry.

Exit gate: the per-phase gates, the golden-surface and precedence-matrix tests above, and F's behavioural gate
(`scripts/herdr-release-smoke` against the newest listed Herdr release plus the wake-e2e `--self-test`, see
"Testing and evidence"), recorded in the run report. The real-agent wake e2e is the run-final gate, not F's.

### Phase N: named seats

Consumes from F: `CallerEnv`/`CallerSite`, the G4 set, `attach_builtin`/`all_for_reads`, the capability-gated lane,
`idle_wake_on`, `wake_status` per-row host, the NodeDelta columns (`seats.name`, `seats_live_name`, `locator_hint`,
`readonly`, `no_wake_reports`), `HostTopologies`/`SeatDirectory.name`, `SeatsQuery.host`/`ResolveThreadQuery`
host, `LocateCaller` plumbing and the scrub list.

Leaves named here so H1 can cite them: **N.1** `SeatName`, `NamedHost`, Named site, discovery; **N.2** standalone
install (Herdr-less); **N.3** store (allocation, continuity, restore, cross-kind, live-holder, readonly, presence,
self-retire); **N.4** inherited-name refusal (the environ reader, `server_process` check, `locator_hint`, M9);
**N.5** display and selectors; **N.6** no-wake reports, inert threshold, `seat.host_sites_v1` and its CLI gate;
**N.7** sync `display_name`; **N.8** skill, tripwire, TRUST-POLICY and AGENTS.md text; **N.9** Hermes bridge.

Scope:

- `SeatName` (src/protocol/ids.rs): the grammar is 1-64 bytes of `[A-Za-z0-9._-]`, no leading `-` or `.`, and not
  seat-id-shaped. The corpus test against the `0012` superset CHECK.
- `HostKind::Named`, with `named` added to the implemented set. src/host/named.rs: `NamedHost`, `NamedSeatSource`
  and `ENV_VARS`. `attach_builtin` runs at daemon start, at the reserved locator.
- `CallerLocator::Named`. Discovery per SA1. `default_state_dir_to_create` extended and the lazy Herdr host in
  src/cli/instance.rs. Hooks, `check_in` and `report_binding_event` take the Named site without `HERDR_ENV`.
- Standalone install (Herdr-less, moved here from H1): scripts/install.sh runs `setup` without a linked plugin and
  keeps `herdr plugin link` only when `herdr` is present; the installer and `setup` write
  `<state>/node/install.json {"mode":"standalone"}`; src/harness/setup.rs bakes `--state-dir` with no `herdr` on
  PATH; the leftover check (src/cli/instance.rs:271-290) honours the marker.
- Store (src/store/seats.rs): name on allocation; named continuity; restore rules; cross-kind refusal; the `Closed`
  guard on named seats; `retire_seat`, `Command::SelfRetire` and `seat retire --self`; the live-holder guard (SA16)
  with `&dyn ProcessInfo` in the enroll transaction; `readonly` bindings (D3); presence fixes (b)(c)(d) (SA17).
- Daemon: the inherited-name refusal (`server_process` environ read in src/service/locate.rs, Linux `/proc`, macOS
  `KERN_PROCARGS2` behind cfg); `locator_hint`; the second-locator notice M9; the pane-form refusal M10; persisted
  no-wake reports; the inert-threshold line M12; the `seat.host_sites_v1` capability and its CLI gate M13; M2
  appended to M1 and M3.
- Sync: `display_name` publish and remote materialize (src/sync, `ensure_published`).
- Hermes bridge: `PANE_ENV` in integrations/hermes/__init__.py:37 also carries `HERDR_THREADS_SEAT_NAME` and
  `HERDR_THREADS_SEAT_READONLY`. The gateway branch (:345-350) accepts a client env that holds a name instead of
  `HERDR_PANE_ID`.
- Skill: K1, K2 and K3 (the `wake=none` meaning change, +6 B). The host-side tripwire, and the rewrite of
  `idle_wake_classification_of_the_builtins` (src/harness/registry.rs:1600-1613).
- TRUST-POLICY N rows. AGENTS.md rules 5/6 exception and "What we do today" lines.

Acceptance:

- [ ] With no `herdr` on PATH and an isolated state dir, a first `inbox` under `HERDR_THREADS_SEAT_NAME=reviewer`
  prints M8 once. A second process with the same name, which is a descendant of the first or starts after the first
  exits, binds the same seat.
- [ ] With no `herdr` on PATH, `scripts/install.sh` into a scratch HOME/CLAUDE_CONFIG_DIR/CODEX_HOME runs `setup`,
  installs hooks with a baked `--state-dir`, writes `install.json`, and `instance`/`doctor` report no leftover.
  With a stub `herdr` on PATH, install.sh output and the installed hooks are byte-identical to today's.
- [ ] Beside Herdr (an isolated, non-default Herdr session): a named caller with Herdr env uses the Herdr command
  group's store; the default dir is not created when Herdr is configured or on PATH; `doctor` flags two live stores
  for one user.
- [ ] A second, unrelated, live process with the same name is refused with M7. After the holder exits (or its pid
  is reused), the name binds at once. A subagent and a `HERDR_THREADS_SEAT_READONLY=1` binding are admitted beside
  a live holder.
- [ ] Two concurrent bindings of one named seat, one a headless `claude -p`-style stand-in with
  `HERDR_THREADS_SEAT_READONLY=1`: the readonly binding's display never ACKs receipt-bearing rows and its turn hooks
  skip them, while it reads and sends normally; the other binding's display ACKs.
- [ ] Send, inbox, ACK and hook delivery work between two named seats, and between a named seat and a pane seat in
  an isolated Herdr session, all through one daemon.
- [ ] Handoff from a named seat into a pane of an isolated Herdr session works, with the Herdr endpoint resolved
  lazily.
- [ ] A named seat missing from an enumeration stays resolved. `RetireReason::Closed` on a named seat is refused by
  the store.
- [ ] A name carried by the (isolated) Herdr server's environ is refused with M6 (`<host>` = `Herdr`) and never
  bound. A different name set per command inside that server's pane is honoured. The same name set per command is
  refused with M6 too (the accepted limit), and binds once the server runs without the variable.
- [ ] The second-locator notice M9 fires on the first binding from a second locator and not on a repeat from either
  locator, also after a daemon restart. `seat inspect` lists the seat's bindings by locator.
- [ ] Cross-kind rebind: a store test shows a named seat never rebinds onto a pane or terminal target, and an
  unresolved pane seat never rebinds onto the named host.
- [ ] The named host never satisfies `wait:host` (store test).
- [ ] Named continuity: an unresolved named seat is reattached by the next resolve of its name, recorded
  `cooperative_named_seat`.
- [ ] One unresolved named row does not silence liveness of the live named bindings on the same host (no
  `baseline_hold_unclaimed`, no recovery hold; store test).
- [ ] One `wake=no_wake` per (sender, seat) across a daemon restart and a mod channel flap. A hooks-only Claude named
  seat classifies `None`. A live-mod Claude named seat reports no block.
- [ ] Golden overdue-line test: a named seat's overdue line ends `wake=no_wake offline` (M19) once its bindings
  ended; a pane seat's overdue bytes are unchanged.
- [ ] One `recipient_unavailable` per offline episode for ACK and p0 sends to a named seat.
- [ ] A deadline on an item to a no-wake named seat produces only the sender's overdue line: no poke and no
  recipient-side reminder.
- [ ] `seat threshold` on a named seat records the value and prints M12.
- [ ] The host-side tripwire passes with K2 in SKILL.md and fails (test of the test, on a copy) without it. The
  rewritten harness tripwire passes.
- [ ] `seat retire --self` retires, lists abandoned items, replicates as Operator, shows
  `cooperative_self_retire` in `seat inspect`, is refused for pane seats (M20) and subagents, and is refused while
  another connected top-level binding exists unless `--force` (M18). A replay returns the stored result. Reusing the
  name mints a new seat.
- [ ] `human seat retire SEAT --operator` retires a named seat, and the next use of the name mints a new seat.
- [ ] Restore: a seat with `display_name` comes back unresolved on the named host. With a non-retired seat of that
  name (resolved, and in a second case unresolved) it is inserted with `name` NULL, the insert never hits
  `seats_live_name`, the name keeps resolving to the existing seat, and `seat inspect`/`doctor` show the held name.
  After `human seat retire --operator` of the existing seat, the held row carries the name and the next resolve
  reattaches it; retiring the held row instead leaves the existing seat untouched.
- [ ] `reviewer@B` addresses and displays over `test_support::multinode`. An old-node fixture decodes
  `display_name`.
- [ ] `thread show`, delivery `from` headers and `seat inspect` render the name. A golden test pins the label
  rendering: workspace and tab `-`, the name shown once.
- [ ] A read (`inbox`, `seat list`, `thread show`) under an unknown name creates no seat. A bare selector token
  that matches a pane path reads as the pane, not as a seat of that name.
- [ ] The handoff recovery hint built for a named caller through `env_assignments` contains
  `HERDR_THREADS_SEAT_NAME=<n>`, and running it resolves back to the same seat (07 §4.4.1 U-T3).
- [ ] `me init` under a name is refused with M15. An explicit cooperative flag with a name is refused with M14. An
  invalid name is refused with M16.
- [ ] Handoff into a named seat is refused with M11. Pane-form allocation of a named agent's pane is refused with
  M10.
- [ ] Idle exit with only the named host exits. A named seat whose bindings ended shows "inactive, unavailable since
  T", and after a daemon restart its live bindings stay available.
- [ ] The `SeatName` / CHECK corpus test passes.
- [ ] Spikes S1-S4 and S6, and the named half of S5, are answered by tests. E1's Linux reader is tested against a
  spawned child with a known env; the live isolated-Herdr run is recorded if the helper starts.

Exit gate: the per-phase gates, plus S1-S6 (named half) and E1 (Linux reader).

### Phase N2: tmux wake for named seats

Consumes from N: `NamedHost`, the Named site, binding `locator_hint` plumbing in the resolve command, persisted
no-wake reports. From F: the capability-gated wake lane and `ComposerTraits`.

Scope:

- `caller_location.rs` reads `TMUX`/`TMUX_PANE` into the hint `tmux:<socket>:<server_pid>:<pane>`, carried in the
  resolve command and held in daemon memory per binding. Wake-armed named bindings count in idle-exit `active()`.
- `HostPort::target_wake_capability` (defaulted). `NamedHost::wake_capability = Composer(TMUX)`. The per-target
  answer depends on verification.
- The built-in session's wake lane, registered with its own pacer, reachability and Health (see "tmux wake for named
  seats").
- src/host/named.rs tmux client (`display -p`, `capture-pane -p`, `send-keys -l`, Enter), through
  `ProcessCommand` with a 2 s deadline, verifying the server pid plus the nearest ancestor at bind and at every wake.
- Turn state: the neutral `turn_state_edge` adapter method. Claude registers report-only `Stop` and `StopFailure`.
  Codex and agy map their existing events. `ComposerPolicy::screen_turn_running` (defaulted `None`) clears a stale
  busy. The wake condition reads `#{pane_in_mode} #{pane_input_off} #{synchronize-panes}`. `ComposerPolicy::screen_blocked`, defaulted `None`, implemented by harnesses whose fixtures
  support it.
- No-wake report flips (row deleted on `Composer`).
- TRUST-POLICY N2 row. AGENTS.md rule 6 wording, and a "What we do today" line for tmux wake.

Acceptance:

- [ ] In a private `tmux -L ht-test-<uuid>` server with a stand-in harness, an idle named seat with an empty
  composer and turn state idle wakes on a p0 item within the existing ladder. A p1 item at T = 0 does not wake it.
- [ ] A draft in the composer, a busy turn (after `UserPromptSubmit`, before `Stop`), a blocked screen, an unknown
  status, or a pane in copy-mode, with input off or with synchronized panes gives no typing, and the copy-mode
  pane's scroll position is unchanged. The sender gets the existing `wake=` overdue rendering.
- [ ] Turn-ending paths: after `StopFailure` (API error) the seat wakes on the next p0. After a user interrupt (no
  `Stop`), a harness with a `screen_turn_running` recognizer wakes once the screen shows no running turn on two
  reads (after 60 s of busy); a harness without one stays unwoken until its next hook (the accepted limit).
- [ ] A stale `TMUX_PANE` (wrong server pid, or a pane that is not an ancestor, nested in an isolated Herdr
  session) disables wake and never moves or rebinds the seat.
- [ ] A named seat outside tmux still reports `wake=no_wake` once. A seat that goes None → Composer → None
  (hint verified, then failed) reports `wake=no_wake` once in each None episode.
- [ ] The built-in wake lane is registered with a pacer, reachability and Health (test). With no dialled host and
  no peer, the daemon does not idle-exit while a wake-armed (verified, live) tmux named binding exists, and a p0
  send to that idle seat more than 10 min after its last hook still wakes it. Once that binding's process exits,
  the daemon idle-exits as before; with only unhinted named bindings it idle-exits as before.
- [ ] After a daemon restart (stop + start), a tmux seat's binding reads `Unknown` (no typing) until its next turn edge,
  then wakes normally.
- [ ] Claude's `Stop` and `StopFailure` hooks print nothing on stdout and exit 0 for every input (golden test over its event
  contract, which marks it report-only). `setup` output and `tests/golden_surfaces.rs` change only by the
  registration.
- [ ] Herdr TUI wake is unchanged: the `idle_wake_on` table test, and a Herdr-pane Claude stand-in wakes as before
  the change.
- [ ] The delivery-contract table (see "tmux wake for named seats") is in the commit message and AGENTS.md's "What
  we do today" gains the tmux line.
- [ ] The tmux servers a test starts are stopped (`kill-server` on the private socket), and the leak check is clean.
- [ ] T1-T5 are answered and recorded in the run report.

Exit gate: the per-phase gates, plus T1-T5.

### Phase H1: hc observation and messaging (no wake)

Consumes from F: the factory split point and attach ordering, `LocateCaller` plus the transport peer pid,
`herdr_hosts.kind`/`kind_probed_at`, `(host_id, target)` label keying, the capability defaults
(`archival_evidence`, `closure_authority`, `target_root_process`, `server_process`), the `startup_primary`/`place()`
filters and the scrub list.

Leaves, with their dependency on N:

| Leaf | Content | Depends on N |
|---|---|---|
| H1a | src/host/hc/{rpc,wire}.rs, fake_hc (in-thread and `fake-hc` bin), the vendored schema and contract test | independent |
| H1b | `HostFactory {exists, probe, build}`, probe before `attach_host` | independent |
| H1c | `HcHost` observation, `HcBoot`, finish-reason closure, labels and doc names (labels block on F6) | independent |
| H1d | `HcDiscovery` | independent |
| H1e | `CallerLocator::Terminal`, full `LocateCaller`, M4 and M17; removal of M1, M2 and M3 | blocked on N.1 merged (M2 code) |
| H1f | `HcHost::server_process` and the hc inherited-name check | blocked on N.4 merged |
| H1g | `seat.terminal_sites_v1`, messaging end to end with `wake=no_wake`, K4 | blocked on N.6 and N.8 merged |
| H1h | hc-only setup (no TUI `--host-endpoint`), Hermes `PANE_ENV` terminal vars | blocked on N.2 and N.9 merged |
| H1i | the two smoke leaves (conformance always; `scripts/hc-smoke` only with a verified pin) | independent |

Scope:

- `HostKind::Hc`, with `hc` added to the implemented set. src/host/hc/{mod,rpc,wire}.rs: `HcHost` (wake `None`,
  `Closing` by finish reason, archival `NoEvidence`), `HcBoot`, `HcDiscovery`, `ENV_VARS`.
- `HcHost::server_process`: the peer pid of the core socket via `LocalEndpointWitness`, and the environ check for a
  `<core>:<tN>` locator hint.
- `HostFactory` becomes `{exists, probe, build}`, with the probe after `exists` and before `attach_host`.
  src/main.rs factory closures.
- `CallerLocator::Terminal`. Full `LocateCaller` (hc candidate via `terminal.get.pid`). `seat.terminal_sites_v1`.
  M4 replaces M1/M2/M3, whose code is removed.
- Labels `<session>:tN` and doc `herdr-app` names. Selector refusal of an ambiguous bare `tN`.
- Finish-reason closure keyed on herdr-core's `(keep, reason)` finality (D10). The C2 hold on a core id change. `status: null` maps
  to `HostUiState::Unknown`, and the status source shows in diagnostics/`doctor` only.
- hc-only install: `setup` bakes no TUI `--host-endpoint` on an hc-only machine. The rest of standalone install is
  in N.
- Hermes `PANE_ENV` gains `HC_TERMINAL_ID`/`HC_TERMINAL_SOCKET`.
- src/test_support/fake_hc.rs (served methods and knobs as listed under "`fake_hc`"); the `fake-hc` bin; the
  vendored schema fixture plus the contract test; the isolated-core helper; leak recognition of
  `herdr-core serve`/`hold`.
- Skill: K4.
- TRUST-POLICY H1 rows. AGENTS.md lines.
- Separate leaves: the schema-conformance test (always) and `scripts/hc-smoke` (opt-in, only with a verified pin).

Acceptance:

- [ ] Against fake_hc, an agent stand-in in terminal `t2` (`HC_TERMINAL_ID=t2`, `HC_TERMINAL_SOCKET=<fake>`) gets a
  seat, sends and receives, ACKs, and gets hook delivery (H1g). The sender gets `wake=no_wake` once.
- [ ] A `HERDR_THREADS_SEAT_NAME` carried by the `fake-hc` child's environ is refused with M6 (`<host>` =
  `herdr-core`) and never bound; a different name set per command in the terminal is honoured.
- [ ] The probe classifies fake_hc as `Hc` and a `fake_herdr` socket as `Herdr`, before any `herdr_hosts` row is
  written, and a peer that never replies fails within the probe deadline. A stored-kind mismatch refuses the attach.
- [ ] A core id rotation puts the seats under C2 holds. A restart (seq reset, same id) is `CarryForward` with zero
  transitions.
- [ ] Finish reasons, one row per D10 table row, against fake_hc applying `Registry::finish`: with `keep: false`,
  every reason (`exited`, `killed`, `signaled`, `server_stop`, `host_shutdown`, `holder_lost`,
  `stopped_by_restart`) retires the seat; with `keep: true`, `exited` and `killed` retire, and `holder_lost`,
  `stopped_by_restart`, `server_stop`, `host_shutdown` and `signaled` unresolve the seat (bindings end
  `seat_unresolved`) and the core's restart of the same `tN` reattaches it through C1; a pruned record (`removed`)
  retires a resolved and an unresolved seat alike; an unknown reason on a `finished` record retires. The outcome
  does not depend on poll timing (the same final state reached through one capture or several gives one outcome).
  A failed `hello` closes nothing.
- [ ] An agent stand-in that exits inside a live shell-rooted terminal ends only its binding; the seat stays
  resolved.
- [ ] A snapshot that carries `machine/t1` mirrors and finished terminals retires nothing live, and the finished
  records reach the closure decision with their `reason` and `keep`.
- [ ] fake_hc `status: null` reads `HostUiState::Unknown`, never Idle, and `doctor` shows the status source; no
  model-visible output changes.
- [ ] Two fake cores, both with `t1`, show `<session>:t1` labels that never cross. A bare `t1` is refused with both
  candidates.
- [ ] With both `HERDR_PANE_ID` (fake or isolated Herdr) and `HC_TERMINAL_ID` (fake_hc) set, `LocateCaller` picks
  the nearest-ancestor root. Neither verifying prints M4 from the CLI and from hooks, nothing binds, and M1 and M3
  no longer appear from a Terminal site.
- [ ] `HC_TERMINAL_ID` without `HC_TERMINAL_SOCKET` is refused with M17.
- [ ] `startup_primary`, `first_host_pacer`, `place()`, the discovery gate, archival and the production
  `BootstrapRuntime` gate each pass a two-host test (TUI fake + fake hc) where hc holds the lowest host id.
- [ ] S5 two-kind half and D12: on a node with a TUI fake host and fake_hc, an unnamed restored row is held
  unresolved and never placed on either host.
- [ ] Idle exit with only a seatless fake core attached exits.
- [ ] A live-mod Claude stand-in on an hc seat reports no `wake=no_wake`; a hooks-only one reports it once.
- [ ] `seat retire --self` from a Terminal site is refused with M20.
- [ ] `HcDiscovery` finds a fake core from a scratch `core.json`, ignores a stale dir whose socket is gone, and never
  spawns `hc` (asserted through a PATH shim that fails the test if called).
- [ ] On an hc-only machine (scratch core state dir, no `herdr` on PATH), `setup` output has `--state-dir` and no
  `--host-endpoint`.
- [ ] The contract test passes against `tests/fixtures/hc/hc-0.1.15.openrpc.json`. The conformance test passes.
  The run report records whether the `scripts/hc-smoke` leaf shipped and ran live or closed as follow-on.

Exit gate: the per-phase gates, plus D1 (fake_herdr probe and bounded-deadline test; live capture if available),
E1 (hc path via the `fake-hc` child; Linux), S5 (two-kind half), and H-C1 answered by the fallback, with the live
part listed as remaining evidence.

### Phase H3: hc launch and handoff

Consumes from H1: `HcHost`, `HcDiscovery`, `fake_hc`, the core id witness. From F: the `native_launch_capability`
audit and the `place()` filter.

Scope:

- `CreateTargetPort`, `Placement`, `HostWitness`, `CreatedTarget` (src/ports.rs). The NativeCli delegate. V3 journal
  records (src/protocol/handoff.rs).
- src/cli/launch_gate.rs (`internal launch-gate`), reading `HC_TERMINAL_*` through `CallerEnv` and scrubbing
  `child_env_scrub_list()`. The `child_scrub_vars()` and `host_launch_argv(HostKind)` adapter defaults. Codex
  `--no-daemon` for `Hc`.
- src/cli/host_client.rs. `handoff`/`launch --space/--new-tab/--new-terminal`. In-place refusal.
  `internal peer-target-create` plus the `peer-tab-create` alias. The launch and handoff `NativeCli::new` sites of
  F's table go through `CreateTargetPort`.
- `terminal.run` (fake_hc serves it from H3) with `no_notify: true`, `outlive_parent: true`, `no_parent: true`, no
  `caller`, the label set, and the prompt through ht.
- Peer `NewTab` mapping. `LaunchTarget::NewTerminal` with `// cap: PEER_LAUNCH_TERMINAL_CAP`.
- TRUST-POLICY H3 rows.

Acceptance:

- [ ] A handoff into fake_hc creates a terminal running the gate shim. The shim's env has no `HERDR_*`, no
  `HERDR_THREADS_SEAT_NAME` and no adapter session vars. It waits for the daemon's `(core, tN)` authorization before
  `exec`, and the bootstrap Send is staged before the harness runs.
- [ ] The `terminal.run` request carries `no_notify: true`, `outlive_parent: true`, `no_parent: true` and no
  `caller`, with no `first_message`. The contract test asserts each param exists in the vendored schema.
- [ ] `--space <id>` sends the label set `{herdr.space=<id>, herdr.tab=<fresh>, ht.bootstrap, ht.label}` to
  fake_hc; `--new-terminal` sends no app labels and parent = caller.
- [ ] `host_launch_argv(HostKind::Hc)` for Codex contains `--no-daemon`, and `host_launch_argv(HostKind::Herdr)` is
  empty for every harness.
- [ ] The existing Herdr handoff and launch tests and journal fixtures pass unchanged, and a Herdr new-tab handoff
  runs through the NativeCli `CreateTargetPort` delegate (asserted by a counting fake).
- [ ] `internal peer-tab-create` behaves as `internal peer-target-create` (same output and effect).
- [ ] V1 and V2 journal fixtures decode byte-identically. V3 round-trips. A recorded unknown outcome is never
  reclassified by the `ht.bootstrap` label.
- [ ] In-place launch into `tN` is refused through the port answer.
- [ ] A launched worker (gate shim `exec`s the stand-in, `keep: false`) that exits retires its seat through the
  D10 rule; no launched seat is left unresolved.
- [ ] A peer without `PEER_LAUNCH_TERMINAL_CAP` is never sent `NewTerminal`. `scripts/check-wire-compat` passes with
  only the new type added.
- [ ] L1-L4 are answered per the spike table. The live parts are listed as remaining evidence.

Exit gate: the per-phase gates, plus L1-L4 as answered.

## Unchanged behavior

Behaviour the design-roast r1 amendment (D10 rewrite, tmux wake evidence, process ancestry, restore collision,
M6 wording, `LocateCaller` cache key) must not change:

- Herdr TUI (`NativeCli`) observation, closure, wake, launch and handoff, and every pane-seat output byte
  (golden surfaces, overdue lines, recovery hints) stay as they are.
- A named host never closes by absence and never retires on `RetireReason::Closed`; named continuity, the
  cross-kind rebind refusal and the live-holder guard (SA16) keep their rules.
- Without tmux, named seats keep no composer wake (`wake=no_wake` once per episode), and the daemon idle-exits as
  today when nothing else keeps it up (Phase N's "Idle exit with only the named host exits" stands).
- The tmux hint stays out of `CallerClaim` and `occupant_bindings.locator_hint`, never triggers M9, never moves or
  rebinds a seat, and needs no migration; there is still exactly one NodeDelta (`0012`).
- The delivery contract's rules 1-4 hold for every host: a busy, blocked, unknown or drafted session is never typed
  into, and no new wake trigger exists beyond `wake_frontier`.
- hc boot stays `hc-core:<core.id>`; a core id change still gives C2 holds; a failed `hello` still closes
  nothing; `status: null` still reads `Unknown`; the hc client still never sends `terminal.list`, `doc.set` or
  `caller`, and machine mirrors are still dropped.
- An agent exiting inside a live terminal still ends only its binding (the H-T1 rejection stands).
- The inherited-name refusal still compares the name only and never falls back to the host candidate; an
  unreadable server env still skips the check.
- `LocateCaller` is still sent only for `Ambiguous` callers, verifies nearest ancestor first, and adds no
  adversarial verification; H1's M4 outcomes are unchanged.
- The K5 scrub list stays one definition (`host::child_env_scrub_list()`), shared by `spawn_detached`, the gate
  shim and every handoff spawn.
- The peer wire is unchanged in F/N/N2/H1 and gains only `LaunchTarget::NewTerminal` in H3.

## Out of scope / follow-on

- **H2, hc composer wake.** It includes `terminal.read` (the W1 composer read), the `terminal.send` refusals
  (`blocked`, `status_mismatch`) and fake_hc's screen-text and refusal knobs, none of which this run builds. Blocked
  on herdr-core shipping an atomic draft-safe `send`
  (`if_input_empty`/`quiet:"refuse"`) and `if:[idle,done]`, detected through `hello.features`. It also needs
  W1-W5, `foreground_contains`, `process_group`, `WakeBlock::{NoStatus, StatusDone}`, `ComposerTraits.focus_known`
  and `stash`, a budget of at least 10 s, and the 05 4b outcome table.
- **Upstream asks.** These are filed by the top-level session or the user, not as code tasks:
  - to herdr-core: the draft-safe send, `if:[idle,done]`, `if_turn`, a sanitized core base env, a public `scope`
    (D6, 06 §12);
  - to Herdr TUI: parse OSC 7501 and answer `7501;?`, prefer reported status over screen detection with fallback,
    and expose `status_source` and `turn` in `pane.get`/events (D21).
- A herdr-app mod panel or a labelled hc operator terminal (05 D9). App tab rename via `herdr-app do`.
- Remote cores (`machine/tN` mirrors, core-to-core links). Remote hc is reached through ht peers.
- `scripts/check-host-neutral` (G6, D18 O8). A cap-gated `RetireReason::SelfRetired`. Named person seats
  (`me init --name`). The hc wake e2e joining the release gate (O9).
- Herdr TUI defect follow-ups: ht-hsyg (keep seats resolved across a Herdr live handoff), ht-wm97 (upstream
  `agent_start_process_hint_v1`), ht-mf0w (instance fast defaults for named Herdr sessions). The rest of R8 (01):
  cross-workspace `pane.move`, plugin-action pane id, Hermes managed-launch refusal.
- Remaining evidence to record in the run report: E1 on macOS, H-C1 live, L1 live, Codex `--no-daemon` live, and the
  Claude half of the wake e2e if no Linux credential is present.

## Key abstractions

**HostPort capability answers** (`HostKind`, `WakeCapability`/`ComposerTraits`, `ClosureAuthority`, `ArchivalEvidence`): new closed enums in src/ports.rs. Each `HostPort` reports through them what it can do.
- Owns: one defaulted answer per policy question: kind, wake (host-level and, from N2, per target via `target_wake_capability`), closure authority, archival evidence, target root pid and server pid. Every default reproduces today's Herdr behaviour.
- Hides: which environment a host is. Delivery, closure, wake, archival, placement and binding policy branch on the answer, never on the kind.
- Must not: be read by `kind()` outside the four listed readers (`any_reachable`, the G4 implemented-kinds check and probe comparison, adapter `host_launch_argv`, doctor/diagnostics). A default must never return `ErrorCode::Unsupported`; methods a built-in host cannot serve return a non-Unsupported `Err` that a capability answer refuses before it is reached.
- Relationships: these are defaulted methods on the existing trait, so the 40 impls compile unchanged. Policy code (host_sessions, dispatch, archival, peer_launcher, idle_exit, reconcile) depends on the answers. Hosts depend on nothing new.
- Errors: only `target_root_process` is fallible (`ApiError`, i.e. host unreachable). The others are plain values, so a forgotten case cannot pass a test vacuously.

**HostKind implemented set (G4)**: a new static set beside `HostKind`. It grows one variant per phase.
- Owns: the answer to whether this build can construct a persisted kind. `Unknown` is fail-closed.
- Hides: phase skew between binaries and stored rows.
- Must not: be bypassed by reattach, restore or rebind. A row whose kind is outside the set is skipped and named by `doctor`, never dialled as Herdr.

**NamedHost** (src/host/named.rs): a new `HostPort` impl. The built-in host is never dialled; its enumeration is a store read.
- Owns: the fixed per-node boot and incarnation, `observe_current_target(name)` (always present, generation 1), enumeration through `NamedSeatSource::live_named` keyed on host id, `NonClosing`, `NoEvidence`, `seat_labels` (pane label = name), `ENV_VARS`. In N2 it also owns the tmux wake path: per-binding hint verification (server pid plus nearest ancestor), `capture-pane` read, `send-keys` typing and the per-target wake answer.
- Hides: that "presence" is cooperative and comes from the store. tmux is a wake path of this host, not a host of its own (SA18).
- Must not: close by absence (reconcile skips that step for `NonClosing`), arm `baseline_hold_unclaimed` or recovery holds, satisfy `wait:host`, move or rebind a seat when a tmux check fails (that only disables wake for the binding), or report wake `Composer` for a target without a verified hint.
- Relationships: composes `NamedSeatSource`, a narrow store read the daemon injects. Dependency points host → store-read trait, never store → host. The tmux client runs through the existing `ProcessCommand`, and the composer read goes through the harness's existing `ComposerPolicy`.
- Lifecycle: the daemon creates it at start and installs it in `HostSessions`' built-in slot through `attach_builtin`. It lives for the whole process. The tmux hints and turn state live in daemon memory per binding; wake-armed bindings keep the daemon from idle-exiting, so only a crash, upgrade or stop loses them (accepted limit).
- Errors: tmux command failure or deadline (2 s) → wake `None` for that binding only. A seat name that fails to parse → it is never observed.

**HcHost / HcBoot / HcDiscovery** (src/host/hc/{mod,rpc,wire}.rs): a new `HostPort` impl, an Adapter over herdr-core's JSON-RPC.
- Owns: the one-connection-per-call line client, tolerant wire types, boot = `hc-core:<core.id>`, mapping herdr-core's `(keep, reason)` finality to closure (D10), with finished records kept in the capture, `status:null` → `Unknown`, labels taken from `herdr.space`/`herdr.tab` plus the `herdr-app` doc, `server_process` taken from the `LocalEndpointWitness` peer pid, and discovery from `core.json` plus `hello`.
- Hides: the hc protocol, the socket path rules, and which of hc's methods it must never call (`terminal.list`, `doc.set`, `caller`).
- Must not: spawn `hc`, treat agent exit as closure, keep `machine/tN` mirrors, or attach on a diagnostic scan.
- Lifecycle: built by `HostFactory.build(Hc, locator)` on a dialled attach. It lives as long as its `HostSession`. It holds no long-lived connection. The observation counter and the doc-name cache are per-instance mutable state.
- Errors: a failed `hello`, a reply with no usable core id or an unreadable record state is `Err` (the observation freezes and closes nothing). A core id mismatch is a new boot (C2 holds). An RPC error before a launch record exists is `NotSubmitted`; `spawn_failed`, a timeout or a disconnect is `OutcomeUnknown` (H3).

**CallerEnv → Located → CallerSite** (src/cli/caller_location.rs): a new module with closed enums. It is the CLI's only reader of caller-location env.
- Owns: reading and empty-filtering the env (names come from each host module's `ENV_VARS`), the precedence (valid name, then one family, then `Ambiguous`, then refusal), M14/M16/M17, the tmux hint (N2), and the per-site answers (`moved_target`, `relative_pane`, `sent_scope`, `required_capability`, `locate_candidate`, `terminal_address`, `claim_host_locator`, `env_assignments`).
- Hides: which env family located the caller, from mod.rs, hook.rs, watch.rs, handoff hints and instance discovery.
- Must not: verify anything (that is `LocateCaller` in the daemon, per A2), or rank one family above another. No other src/cli file reads these variables (enforced by a source scan).
- Relationships: depends on the host modules' `ENV_VARS` consts only, not on the hosts. Consumers match on `CallerSite`'s methods, not on `CallerLocator`.
- Lifecycle: built once per CLI or hook invocation; immutable value.
- Errors: `LocatorError` keeps "invalid name", "terminal without socket" and "both families, unverified" (M1, or M4 from H1 on) distinct. No family and no name keeps today's error text.

**LocateCaller** (src/service/locate.rs): a new daemon service. It verifies an `Ambiguous` caller by nearest ancestor and checks for an inherited name.
- Owns: the walk from the subject (liveness pid, else transport peer pid) up `ProcessInfo` parents against each candidate's `target_root_process`, the herdr-core terminal-root wall, its cache keyed on (candidates, subject pid, subject start stamp, boot/epoch) with unstamped subjects uncached, and the server-environ read for the inherited-name refusal (M6). The environ reader is a plain function with a Linux `/proc` body and a macOS `KERN_PROCARGS2` body chosen by cfg, not a trait.
- Hides: how a pid maps to a host target, and how a server's environ is read on each OS.
- Must not: run for `One` callers (that would be adversarial verification, which TRUST-POLICY forbids), fall back to a candidate when none verifies, or fail closed when the environ is unreadable (`server_process() == None` skips the check).
- Relationships: delegates to `HostPort::target_root_process`/`server_process` through `HostSessions::all_for_reads`. It knows no host kind.
- Lifecycle: one per daemon. The cache is its only mutable state, shared behind the dispatch lock or a mutex.
- Errors: `Verified{index, root}` vs `Unverified{per_candidate}`; an unreachable host counts as unverified. M6 is its own refusal code.

**HostSessions** (extended): adds a `builtin` `OnceLock` slot and `attach_builtin`, which skips `exists`, the canonical check and the cap. Adds `all_for_reads` (dialled plus built-in). `attached()` stays dialled-only. `HostSession.wake` becomes `Option<LaneHandle>` and starts only when the wake capability is not `None`; in N2 the built-in session registers its own lane with its own pacer, reachability and Health. Applies the 07 §5.3 filters at each consumer, by capability answer. Each consumer's choice of `attached()` or `all_for_reads` is decided per call site, not by a flag.

**HostFactory** (extended): becomes `{exists, probe, build(kind, locator)}`. It stays closures and stays the one place a dialled `HostPort` is constructed for a kind. The probe runs under `attach_lock` before `store.attach_host` writes `kind`/`kind_probed_at`. A stored kind that disagrees with the probe refuses the attach.

**HarnessRegistry** (extended): adds `idle_wake_on(harness, WakeCapability, channel_live)`, the single classifier that combines harness, host capability and channel. `has_idle_wake` stays as the test oracle. `wake_status` uses it per row with the row's own host.

**HarnessAdapter / ComposerPolicy** (extended): new neutral defaulted methods: `turn_state_edge` (N2; default `None` = no tmux wake), `ComposerPolicy::screen_blocked` (N2; default `None` = no wake), and `child_scrub_vars` and `host_launch_argv(HostKind)` (H3; default empty). Per-harness behaviour stays in adapters.

**Store seats** (extended, src/store/seats.rs): `retire_seat` factored out of `operator_retire`, used by `SelfRetire`. A live-holder guard in the enroll transaction (`&dyn ProcessInfo` passed in). Named continuity as a fourth C1 path. The cross-kind rebind refusal. Restore placement by name or by a single dialled kind. The `Closed` guard on named seats. The literal `'named'` appears only in the unique index and in the restore/rebind rules.

**CreateTargetPort** (H3): new, beside `CreateTabPort`. Its second implementation exists in the spec (NativeCli by delegation to `create_tab`, HcHost via `terminal.run`). It carries `Placement`, plus `HostWitness`/`CreatedTarget`, whose `validate()` dispatches on the witness. Launch and handoff code becomes generic over it. V1/V2 journal bytes stay frozen and V3 carries `CreatedTarget`.

**Env scrub list** (extended): `host::child_env_scrub_list()` is the single union, built from each host module's `ENV_VARS`. `spawn_detached`, the gate shim and every handoff spawn take it. `lifecycle` holds no host literal.

Variation:
- New host environments → a `HostPort` impl per kind. The dialled kind is chosen in one place, `HostFactory.build(kind, …)`, fed by the probe; the built-in kind is installed only by `attach_builtin`. Policy reads capability answers, so a new kind adds answers, not branches.
- The CLI-side site kinds are fixed per phase, so `CallerLocator`/`CallerSite` is a closed enum with `match` (spec §Caller location rejects `dyn CallerSite`).
- Wake mechanism per target (Herdr composer, tmux, none, later hc) → `WakeCapability`/`target_wake_capability` per host, combined with the harness in `idle_wake_on` only.
- Per-harness turn edges and blocked screens → adapter defaults.
- OS-specific environ reading → a cfg-selected function.

Walk-through:
- Main use case: an agent runs `inbox` with `HERDR_THREADS_SEAT_NAME=reviewer` and no Herdr.
  1. `CallerEnv::from_process` → `locate(Cli)` → `One(Named)` → `CallerSite`. `sent_scope` makes the CLI send `SeatsQuery.host`. Instance discovery takes the default dir.
  2. The daemon resolves via `HostSessions::by_locator`, which answers from the built-in slot, to `NamedHost`.
  3. A seat-acting call enrolls: seats.rs allocates by name (M8) and runs the live-holder guard against `ProcessInfo`. If a host candidate is present, `LocateCaller` reads that host's `server_process` environ for M6.
  4. Delivery runs unchanged. A sender's `wake_status` looks up the row's host through `all_for_reads`. `target_wake_capability = None` → `idle_wake_on` → `NoWake`, persisted once in `no_wake_reports`.
  5. Reconcile sees `NonClosing` and never retires by absence.
- Likely future change, H2 hc composer wake: `HcHost` answers `wake_capability`/`target_wake_capability = Composer(..)` and gains its composer read and send. `HostSessions` then starts the lane by the capability gate, and `idle_wake_on`, `wake_status` and the no-wake rows flip with no caller edits. It touches one type (`HcHost`) and no registration.
- A fourth dialled environment touches one new `HostPort` type, one `HostKind` variant with its implemented-set entry, the factory/probe arm (the registration), one `CallerLocator` variant and its `ENV_VARS`. The `CallerLocator` edit is accepted by spec decision: closed match over a trait object.

Deliberate exceptions the spec forces:
- Per spec §"HostPort capability answers": `kind()` is read by exactly four callers (discovery gate, G4, `host_launch_argv`, doctor). These are intended per-kind branches, each checked in review.
- Per spec §"seats.name and the literal 'named'": the store branches on `'named'` in the unique index and the restore/rebind rules. Intended.
- Per spec §"Built-in vs dialled hosts": each node-wide consumer chooses `attached()` or `all_for_reads()` explicitly by call site (07 §5.3 table). This is a deliberate per-caller choice, not an abstraction.
- Per spec §"TUI leak guard": the M3 "first delivering hook per binding" memory is keyed on `OutputPolicy` plus lifecycle registration and lives only from F until H1. It is temporary daemon memory, not a new abstraction.
- Per spec §"HostPort capability answers": the B design's split of `HostPort` into capability traits is rejected in favour of defaulted methods on one trait.

Digest:
- HostPort capability answers: own each host's answer to every policy question (kind, wake, closure, archival, root/server pid). New per-environment behaviour is a different answer in that host's impl. Policy callers never branch on `kind()` outside the four listed readers.
- HostKind implemented set: owns "can this build construct this persisted kind". A new kind is added beside its `HostKind` variant in the phase that builds it. Reattach, restore and rebind never dial a kind outside it.
- NamedHost: owns the built-in, never-dialled named host: store-read enumeration, `NonClosing`, fixed boot, and the N2 tmux wake path per verified binding. New named-seat host behaviour goes in src/host/named.rs. Callers never close, hold or rebind named seats by absence or by a failed tmux check.
- HcHost / HcBoot / HcDiscovery: own the herdr-core adapter (JSON-RPC, core-id boot, finish-reason closure, labels, discovery without spawning `hc`). New hc protocol use goes in src/host/hc/. Callers never call hc directly or read hc state files.
- CallerEnv / Located / CallerSite: own reading caller-location env and its precedence in the CLI. A new site kind is a `CallerLocator` variant plus arms in its closed match. No other src/cli file reads those variables.
- LocateCaller: owns daemon-side nearest-ancestor verification of two-family callers and the inherited-name environ check. A new host candidate verifies through its `target_root_process`/`server_process` answers. Callers never verify `One` sites and never fall back when nothing verifies.
- CreateTargetPort: owns creating a host target for launch and handoff (new tab, new terminal) and validating its witness. A new host's creation is one impl of the port. Launch and handoff code never branches on the host type.

## Post-Implementation Notes

> *As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*

### 2026-11-01: H3, `launch --new-tab` and `launch --new-terminal` (ht-kqrt.186)

`launch --new-tab LABEL [--space SPACE]` and `launch --new-terminal` shipped. `--space` alone keeps selecting a
pane's workspace, as on `handoff`, so the spec's `launch --space` is `launch --new-tab LABEL --space S`. On a Herdr
host the tab is created through `CreateTargetPort` and the unchanged `launch` starts the harness in its root pane. A
plain launch onto a core uses a gate-only record (`local_launch_gates`, behind the same launch-gate view): the seat
is allocated and bound `managed_launch` before the gate opens, and the agent starts idle with no first message, so
"the bootstrap Send is staged before the harness runs" holds for a handoff only. `--harness-binary` and `--name` are
refused on a core, and there is no hook preflight there. This supersedes the narrowing recorded in run.md:220, which
stays as history.

### 2026-11-01: guarded capabilities are decided for the host they act on (ht-kqrt.200)

This amends line 168 ("`startup_primary` falls back to a dialled host filtered to `HostGuardedStart`"): every branch
of `startup_primary` is filtered, not only the fallback. A hinted or remembered built-in or core session no longer
leads startup, so a named or core first caller cannot take the guarded answers from Herdr panes. The node-wide
`handoff.bootstrap_guarded_resolution_v1`, `handoff.automatic_v1` and launch-directories capabilities are no longer frozen
once at composition from the startup primary: `Capabilities` reads them per call (`GuardedAnswers`) from the hosts
attached then, so a Herdr host attached after startup is served, and `new_guarded` takes no host. A V3 handoff plan
(a selected target) is gated on the target core's own session (`target_bootstrap()`, decided in the daemon) and on
`handoff.automatic_target_v1`, not on the node-wide automatic flag, so `handoff` onto a core works on a core-only
daemon. Health's status, incarnation witness and safe-prompt answers follow the same primary per call. The lane
slots, archival start and the startup log line stay as they were. Delivery into an existing seat
(`handoff_delivery::require_automatic_capability`) still gates on the node-wide flag; it creates no target. Every path
over a journaled V3 bootstrap (fresh, crash retry, operator recovery and its retry, with or without a profile
directory) gates on the plan's route through `require_bootstrap_route_capability`, never on the node-wide flag.

### 2026-11-01: tmux wake gate parity (ht-kqrt.214)

This amends line 583 (the composer read is `capture-pane -p` alone) and line 1317 (the tmux client's `capture-pane -p`),
which stay as written: the composer is read from the plain capture and, when the harness's `ComposerPolicy::wants_styled`
asks, from one more `capture-pane -p -e`, through the same `read_composer_with` that `native` uses over Herdr's
`agent read`. A capture that fails, carries no SGR or is past its 1 MiB limit keeps the detection read. A default hooks-only
Claude seat showing a dim prompt suggestion therefore wakes (the plain text cannot tell it from a draft); the captured
suggestion, draft and plain screens of `docs/evidence/ansi-composer/` are the tmux tests' fixtures. This adds no accepted limit.

The walk of `native`'s `submit_prompt_mode`, one step at a time:

| # | Native | tmux |
|---|---|---|
| 1 | take the empty-window sample | adopted |
| 2 | basis is `CooperativeAgent` | adopted |
| 3 | `safe_prompt_capability` | does not apply: it says the platform can witness a server incarnation; tmux's witness is `#{pid}` against the hint (step 7) |
| 4 | boot and epoch of the context | adopted |
| 5 | cancellation | adopted |
| 6 | budget floor `left <= 2 * MIN_PROMPT_MILLIS` | adopted |
| 7 | `agent get` with incarnation witness | `pane_facts` with `server_pid`, adopted |
| 8 | `cooperative_wake_ready` (status idle) | does not apply: Herdr's status has no tmux source; turn state, `screen_turn_running` and `screen_blocked` stand in |
| 9 | focus read | `facts.focused()`, adopted |
| 10 | composer read, styled refinement | adopted, fail-safe to the detection read |
| 11 | a non-`Empty` read resets the empty window, on every read path | adopted on `read_pane_composer` (the read-back and the clear loop) |
| 12 | `check_epoch`, `check_context` after the read | adopted with cancellation and budget, after the capture and after the composer read |
| 13 | `focused_empty_ready` | adopted |
| 14 | cancellation again | adopted through step 12 |
| 15 | `still_owed` | adopted |
| 16 | `left < MIN_PROMPT_MILLIS` | adopted in `type_wake` |
| 17 | a refusal notes its wake block | adopted (`refuse_with`) |

Step 8 keeps one consequence: a detection read of `Unreadable` makes Claude's `screen_blocked` answer `None`, so an
uncaptured placeholder is refused as `screen not recognised` before the styled read could vouch for it, where Herdr's
status answers for the pane. The refusal retries on the ladder as `wake=unreadable`. This is today's behaviour, not a new
limit; letting a styled `Empty` override `None` would also pass a recognizer-less harness through that arm.

### 2026-11-01: the caller site is decided after the routing flags (ht-kqrt.212)

A verified two-family terminal took its core socket for the Herdr server: the endpoint reads
(`herdr_endpoint_io`, `pane_agent`, the watch marker prefix, `run_legacy_purge`) rebuilt their inputs from the raw
environment, where the pane id still reads `Ambiguous`, so pane forms, `launch` and `handoff --pane` or `--existing`
failed on a reachable Herdr. `run` now parses first, then asks the guard with the routing flags' daemon, then decides
the site once. `InstanceInputs::from_process` is gone: `InstanceInputs::for_site` is the one process constructor and
every caller names the decided site (`None` only for the guard and `peer --stdio`); `CallerEnv::for_site` reads a
verified terminal as its lone site. A direct send's `recipient_host` drops the claim-host pin on a non-Pane site so a
recipient pane's server is never the core. `--help` and usage errors no longer pay the guard's daemon call or print M4.

### 2026-11-02: a launch runs under the launching side's profile (ht-kqrt.223)

Gate step 3 ("sets the caller's config roots, so a leaked `CLAUDE_CONFIG_DIR` cannot win") now holds for every launch
onto a core. Each adapter names its config-root variable in `child_scrub_vars` (Claude `CLAUDE_CONFIG_DIR`, Codex
`CODEX_HOME`, Hermes `HERMES_HOME`; agy's root is `HOME`, shared by core and caller, so it scrubs nothing), so the
shim removes an autostarted core's root. `launch --new-terminal` passes the caller's own root as `--config-root`; a V3
handoff freezes it in the plan's `directory_context` when no `--profile-dir` is given, so a retry from another
environment keeps it. A peer launch plan carries no root and the wire is unchanged: the harness there runs on the peer
user's default root, and the requesting side's profile cannot cross without a wire field.

### 2026-11-02: read-only bindings on the V2 delivery path (ht-kqrt.222)

D3's filter was only in the V1 pull. The V2 presentation reader (`pull` in `src/store/invitation_content.rs`: the
normal walk, the carry re-read and the due re-offer) now withholds a receipt-bearing row from a read-only binding, so it
stays pending for a writable one. The historical `watch_loop` fallback drops it in `drain`, keyed on the watch's own
`HERDR_THREADS_SEAT_READONLY`. A read-only mod channel is never judged stalled and does not suppress the seat's wake
(`ModChannelRegistration.readonly`, frozen at registration): otherwise the 10-minute `Stalled` close, which used to free
a writable sibling's wake, would have become permanent. A seat whose only channels are read-only reads as having no
channel wake. Keyed on the binding, never on a harness.

### 2026-11-02: the remote new-terminal form (ht-kqrt.225)

`LaunchTarget::NewTerminal` had no production caller: only a test-support switch sent it to a peer. The form is
`--new-terminal --space @MACHINE --cwd DIR`, on `handoff` and on `launch` (which gains `--cwd`, accepted only with
`--new-terminal`). The location before the `@` is empty because a terminal lives in no space; `--new-terminal --space
SPACE@MACHINE` is refused rather than ignoring `SPACE`, and `split_machine` keeps refusing an empty location for every
other selector (`terminal_machine` reads this one form). The label is the harness id, as on a local core launch. `--cwd`
must be absolute and is checked only on the peer (TRUST-POLICY N15). `--harness-binary`, `--name`, `--profile-dir`
and `--summary` stay refused, as for the other remote forms. The wire variant and `peer.launch.terminal.v1` are unchanged,
and a peer without the cap still refuses the request. `HT_TEST_REMOTE_TERMINAL` is gone.

### 2026-11-02: `handoff recover --created-pane` of a V3 plan keeps the core terminal (ht-kqrt.227)

On a core-only daemon the CLI built a `NativeCli` on the plan's core endpoint and sent Herdr `ping` to the core, and
the protocol refused a created-pane disposition on a V3 plan, so the one recovery that keeps a really created terminal
had no path and `--not-created` left two live terminals. `CreatedPane.evidence` is now an `AutomaticCreation` (V2 tab
bytes unchanged), checked against the plan by `creation_scope`. The CLI routes a V3 plan to `HcHost::recovered_creation`
(one `terminal.get` pinned to the core's peer, the creation text `create_target` reports); a Herdr plan keeps the
`NativeCli` read. A recovery later than the gate shim's 10 s authorization wait finds no terminal and takes
`--not-created`. TRUST-POLICY.md's V3 limit is rewritten accordingly.

### 2026-11-02: the `LocateCaller` verdict cache is deleted (ht-kqrt.232)

`LocateCallerService` keeps no verdict cache and `LocateCallerService::locate` takes no epoch. The only production
`LocateCaller` is the CLI guard's, which sends `subject: None`, so the key was the one-shot process's own pid and start
stamp and never repeated; the walk is cheap. Every call now walks afresh. This supersedes the body's cache claims (about
lines 237, 1139, 1163, 1500 and 1585-1589), which stay as written: the cache key, the reused-pid-misses and
unstamped-never-cached behaviours, and the cached-`NotAncestor` concern under terminal identity no longer exist. The
checklist item on a reused pid is met trivially (there is no cached verdict to return). TRUST-POLICY drops the matching
sentence.

### 2026-11-02: M10 needs a live binding and compares the Herdr server (ht-kqrt.234)

This amends the bullet that says a Herdr pane's hint is the bare pane id (M10, SA9). M10 now refuses only for a
connected top-level binding of a live named seat: a named process that exits is disconnected by the liveness
worker but its binding never ends, so the old query held the pane until the seat was retired. M10 also compares the
Herdr server: the caller sends its canonical `HERDR_SOCKET_PATH` as a `pane-host=<socket>` part after the pane in
`ResolveSeat.locator_hint` (a part, not a field, so `ResolveSeat` and the wire are unchanged), the binding stores it in
`occupant_bindings.locator_host` (migration 0014), and the pane hint refuses only an allocation on that host. A pane
hint with no recorded server still matches that pane id on every Herdr host (accepted limit; the outcome is M10
naming a live seat, never a wrong bind). `locator_hint`, M9 and `seat inspect` read the pane part exactly as before.
