# Session-keyed seat bindings — design

## Goal

Several agents can occupy one Herdr pane's seat at once (a foreground agent, its subagents, a
background or one-shot agent), each as its own **binding keyed by its harness session**. Each binding has its own
read cursor, so none of them misses or steals another's mail. Calls are attributed to the exact
calling session. Dead sessions are marked disconnected, closed panes retire their seats, and threads with no
active participants archive. Hermes enrolls globally through its hooks (absorbing ht-4ne), and a
one-shot `hermes -z` inside a Claude pane adds a binding instead of displacing Claude's.

Run: `docs/superpowers/runs/2026-10-09-session-keyed-seat-bindings/` (branch
`super-auto/session-keyed-seat-bindings`, cut from ht-4ne's tip with main v0.6.0 merged).

## Background

- **Today:** a seat has exactly one open occupant binding. The database enforces it with
  `occupant_bindings_current` (a unique partial index on `seat_id WHERE ended_at IS NULL`). A
  lifecycle check-in by any harness replaces it, and the binding generation is the seat's
  `generation`. Plain CLI calls build their `CallerClaim` from the pane's single client-side
  `OccupantContext` file (`contexts/<seat>/context.json`), so every process in the pane acts as
  whoever checked in last.
- **ht-4ne probe** (isolated Herdr, real daemon), showing the failure:
  - A one-shot Hermes start closed Claude's binding.
  - Claude's PreToolUse hook then printed `no registered execution for this session` and stopped
    delivering notices.
  - Claude's `inbox` displayed messages but did not ACK them.
  - Claude's sends were attributed to the Hermes session.
  - This lasted until Claude's next SessionStart.
- **Attribution research,** summarized in decision 12 below:
  `~/Documents/Agent_Session_Attribution_Research_20261009/` (`report.md`,
  `spike-claude-updatedinput/RESULTS.md`, `workers/codex-daemon-panes.md`).
- **Current-model map,** used for decomposition:
  - `store/queries.rs::open_binding` is the single A4 reader.
  - `store/seats.rs`: `cooperative_mapping` (~3949), `register_available` (~4218),
    `record_managed_launch` (~4414).
  - Archival: `store/archival.rs` `scan_members` (~316).
  - Mod channel: `service/mod_channels.rs` (registry keyed by seat).
  - Wake: `store/wake.rs` `cooperative_authority`.
  - Per-seat delivery state: `lazy_recipients`, `warning_offer`/`digest_notice_offer`,
    `catch_up`, the client-side `attention.json`.
  - Schema: `src/store/schema.rs`, `LATEST_VERSION = 28`.

## Decisions (settled with the user)

1. **One seat per Herdr pane.** There is no group or individual join machinery; herdr-graph will
   adopt this model later.
2. **Seat state is shared by all its bindings.**
   - Memberships and invitations belong to the seat, and every join or leave applies to the whole
     seat.
   - A background agent's leave therefore removes the seat for the foreground agent too. This is
     accepted, and the skill and docs say so plainly.
3. **A seat may have many bindings.**
   - A binding is keyed by (seat, harness, native session id, agent id).
   - `/clear`, `/new` and a fresh start open a new session, so they get a new binding.
   - Resuming a session continues its binding.
4. **Each binding has its own per-thread read cursor.** Every binding sees its own full stream,
   with no races and no skips.
5. **Enrollment is cheap and applies to everyone.**
   - This covers the main session, subagents, one-shots (`hermes -z`, `claude -p`), a new harness
     and `/clear`.
   - A new binding's cursor starts at the seat's maximum cursor in each thread.
   - Once, at first contact, it gets per-thread counts of the history before that cursor.
6. **Resume** continues from the binding's cursor. When the backlog exceeds one ordinary inbox
   page, it fast-forwards instead (§5).
7. **ACK obligations stay per seat.** The first top-level binding that fully displays a message
   settles it. Per-binding obligations were rejected, because they would turn a binding into a
   full participant.
8. **Wake is evaluated per connected top-level binding.** A binding that cannot take an injected
   turn ignores it.
9. **Liveness is separate from identity.**
   - A binding is `connected` or `disconnected`, and a disconnected binding is never deleted (it
     may be resumed).
   - A participant is active iff at least one of its bindings is connected.
   - **A binding with no liveness fingerprint is not admitted** (no zombie bindings).
10. **Pane close within the same live Herdr instance retires the seat.**
    - Its bindings disconnect and its memberships are retired.
    - Its record persists for undo and herdr-graph.
    - A Herdr restart still goes through C2 restore holds.
11. **Archival:** a thread archives after one hour quiet. Inactive members (no connected binding,
    unresolved or retired seat), and pending obligations owed by them, no longer block it. Members
    with a connected binding still need the idle-sample check. Human seats count as active while
    their pane exists.
12. **Callers are attributed from the harness environment** (see the table in §1). MCP is not used.
    PID ancestry is used for liveness only.
13. **Subagents** (Claude Agent-tool subagents, Codex spawned threads, Hermes delegate children)
    get their own binding and cursor. They are **read-only**: they may not ACK, send, accept,
    reject, leave, invite, check in for the seat, or do any other write.
14. **Human seats (`me init`) are exclusive, as today.**
    - A human-mode seat holds only its human binding.
    - An agent's lifecycle check-in in that pane ends the human binding.
    - `me init` is refused while any agent binding is connected, unless `--operator`.
    - Receipt waivers and the no-wake rule keep their meaning.
    - Human seats keep today's pane-context attribution path.
15. **No session id means refusal for agents.** An agent CLI or hook call without a session id is
    refused, reads included. Only human seats are exempt.

## §1 Bindings and caller identity

**The binding row.** `occupant_bindings` keeps its rows and gains these columns:
- `binding_id`: stable, unique.
- `role`: `top_level` | `subagent`.
- `parent_binding`: for a subagent.
- `agent_id`: empty for top-level bindings.
- `lineage_from`: the predecessor binding, when the session continues one.
- `liveness`: `connected` | `disconnected`, with `disconnected_at` and `disconnect_reason`.
- `liveness_pid` and `liveness_start`: the process fingerprint.

Constraints and index changes:
- The partial unique index `occupant_bindings_current` is dropped.
- It is replaced by uniqueness of (seat, harness, native_session, agent_id) among bindings of a
  non-retired seat.
- `ended_at` now means "ended with the seat" (retire, unresolve or rebind).
- Disconnection is the separate `liveness` state.

**`seats.generation`.** It becomes the seat's mapping generation, bumped only by rebind,
unresolve and retire. The joins written `b.generation = s.generation`, which mean "the current
binding", are rewritten to select bindings by identity or by liveness.

**Claims come from the environment.** The CLI and every hook derive a `CallerClaim` of
(harness, native session, agent id, role, liveness fingerprint, target pane):

| Harness | Session | Subagent identification | Liveness fingerprint |
|---|---|---|---|
| Claude Code (≥2.1.214) | `CLAUDE_CODE_SESSION_ID` (equal to the hook `session_id`; updated on `/clear`) | `--agent-id=<agent_id>`, inserted right after argv0 by herdr-threads' PreToolUse hook through `updatedInput`, with **no** permission decision. The hook rewrites only commands that start with `herdr-threads`/`ht`. Spike-verified: existing `Bash(herdr-threads:*)` allow rules keep matching, and a hook `allow` would approve whole compound commands. | `CLAUDE_PID` plus that process's start time |
| Codex (≥0.148) | `CODEX_THREAD_ID` | `CODEX_THREAD_ID` ≠ `CODEX_SESSION_ID` means a subagent whose parent is the binding of `CODEX_SESSION_ID`. Hook payloads carry `session_id` (the root) and `agent_id` (the subagent thread). | the agent ancestor process of the caller. A process under `codex app-server --managed-daemon` is **refused**, with a hint (keep Codex in its pane: the pane-env block from branch `codex-daemon-env`, or `--no-daemon`) |
| Hermes | `HERMES_SESSION_ID` (fresh in every local command) | A delegate child is known through the `subagent_start` hook, which reports the child session with its parent. A CLI call whose session is a registered child acts as that subagent binding. | the Hermes process (ancestor of the command or hook) |
| Human (`me init`) | today's pane `OccupantContext` path | n/a | today's rules |

**Refusals:**
- An agent call with no session id is refused.
- An agent call with no derivable fingerprint is refused.
- These refusals carry stable error codes and name the missing variable or harness requirement.

**The daemon decides (A2).** It matches the claim to a binding of the pane's seat:
- an existing binding is touched, and becomes `connected` if it was disconnected;
- an unknown session is enrolled (§2);
- a subagent claim attaches to its parent binding.

**The client journal.** The per-seat client `OccupantContext` journal is retired for agent seats
(no file-based claim, no "shared context" for the pane). Human seats keep it. Hook-side attention
marks become per binding (§2).

**Lineage.** A successor session that carries the conversation forward copies its predecessor's
cursors (`lineage_from`).
- **Known lineage:** Hermes compression and `/branch`, through `parent_session_id`; Claude
  `--resume <id>`, which is the same id and therefore the same binding.
- **Everything else** gets a new binding: Claude fork (its predecessor is not exposed), `/new`,
  `/clear` and a fresh start.

**New provenance values in TRUST-POLICY.md:**
- `cooperative_env_session`: a session id claimed from the harness environment.
- `cooperative_hook_agent`: an agent id inserted by the PreToolUse hook.
- `liveness_process_exit`: a disconnection recorded because the fingerprinted process exited.
- `liveness_end_event`: a disconnection recorded on a harness end event.

## §2 Enrollment, cursors and delivery

**Enrollment** happens on a binding's lifecycle hook, or on the first CLI or hook call that
carries an unknown session on an existing seat.
- **Lifecycle hooks:** Claude SessionStart; Codex SessionStart and SubagentStart; Hermes
  `on_session_start` (ht-4ne's witnessed CLI start); Claude and Hermes subagent start.
- **Uniqueness:** enrollment is idempotent per binding key, decided in one daemon transaction, so
  concurrent hooks cannot create two bindings.
- **New seats:** the ordinary guarded resolver allocates a new seat only for a genuinely new pane.
  Restore holds and unresolved seats refuse enrollment.

**Cursors.** A new table, `binding_cursors(binding_id, thread_id, seq)`.
- A new binding's cursor in each joined thread starts at the maximum cursor any binding of the
  seat holds there. If the seat has no cursor in that thread, it starts at the thread's head.
- When the seat joins a thread, every binding of the seat gets a cursor at the join point.
- A lineage successor copies its predecessor's cursors.
- Cursor rows of retired seats are kept with the seat record.

**First-contact stats.** Shown once, in the response to the binding's first check-in or call.
- One line per joined thread that has history before the binding's cursor:
  `#name (id): N earlier messages, H from people (R open requests), last Xm ago — herdr-threads summary <id>`.
- At most 8 threads, ordered by most recent activity, followed by
  `+N more threads (herdr-threads thread list --joined)`.
- **Sources:**
  - `messages.author_role`, `relays_user` and `user_intent` give the human-input counts;
  - the summary ledger's open request and question items give the open-request count;
  - the counts cover messages up to the cursor.
- No summary text is included inline.

**Inbox, per binding.**
- **It shows:**
  - the binding's unread messages, meaning everything in joined threads past its cursor, under the
    existing lazy and ordinary delivery rules;
  - the seat's open invitations;
  - the seat's still-pending ACK obligations, including ones that predate the binding.
- **Display:** fully displaying a message advances only *this* binding's cursor.
- **State that becomes per binding or is replaced by the cursor:**
  - the per-seat `lazy_recipients` display state;
  - the `warning_offer` and `digest_notice_offer` frontiers;
  - `catch_up` rows;
  - client `attention.json` marks.

**ACK.**
- A top-level binding's display ACK (or an explicit `ack`) settles the seat's obligation. The
  receipt records the acting binding's id.
- Other bindings still see the message in their own stream, marked already settled.
- A subagent binding's display advances its own cursor and never settles anything.

**Fast-forward.**
- **Trigger:** a resumed or reconnected binding whose unread backlog exceeds one ordinary inbox
  page (the existing byte budget).
- **What happens:** its cursors jump to each thread's head, and it gets the last message per
  thread, per-thread gap counts in the same shape as first-contact stats, and a `summary` pointer.
- **Never skipped:** pending ACK obligations stay listed until they are settled.

**Hook notifications.** The PreToolUse/Current hook output and the attention digest are computed
per binding. Each binding gets its own new-mail notices, and one binding's display never hides mail
from another.

**Subagent bindings are read-only.** Their CLI writes are refused cooperatively, using the
recognized subagent claim. This makes enforceable the skill rule that subagents never write.

## §3 Liveness, wake, delivery channel and cleanup

**Becoming connected:** on a lifecycle check-in, or on any call the binding makes. The call
records or refreshes its fingerprint.

**Becoming disconnected** (never deleted):
- **The process exited.** The daemon re-checks fingerprints of connected bindings on a timer and
  before wake decisions.
- **A newer session reports the same PID and harness** (Claude `/clear`, Hermes `/new`). The older
  binding disconnects.
- **A harness end event:** Codex SessionEnd/SubagentStop, Claude SubagentStop, Hermes
  `on_session_end`/`subagent_stop`.
- **The seat retires or becomes unresolved.**

There is no timeout. A binding without a fingerprint is never admitted (decision 9).

**Presence.**
- A seat is **active** iff at least one of its bindings is connected. A human seat is active while
  its pane exists.
- `thread participants` and `seat list` show active versus inactive members, IRC-style.
- Today's per-seat `availability` / unavailability episodes are recomputed from binding liveness.

**Wake.**
- The scheduler evaluates wake per connected top-level binding.
- **Mod delivery channel:** the registry in `service/mod_channels.rs` becomes keyed by binding,
  so each Claude binding has its own channel. `BindingChanged`/`Replaced` closes apply to that
  binding only, and stall cooldowns are keyed per binding.
- **Pane-level composer wake** (typing into the pane): it targets the binding Herdr reports as the
  pane's foreground agent with `agent_status` idle. It fires at most once per seat per wake.
- **Bindings with no channel** (headless one-shots, subagents) are skipped; they pick up mail
  through their hooks and `inbox`.
- Human seats are never woken.

**Managed launch.** `record_managed_launch` no longer requires "no open binding". It records a
launch placeholder binding for the launched harness. The launched agent's first lifecycle check-in
replaces that placeholder, and only that one.

**Pane-close retirement.**
- A pane absent from a same-incarnation snapshot retires its seat through the existing
  `BeginRetirement` → `begin_retirement_fence` path:
  - bindings disconnect;
  - memberships retire;
  - receipts settle as recipient-retired;
  - invitations close;
  - the seat record and its cursors persist.
- A new or unknown Herdr incarnation still yields unresolved seats and C2 restore holds.

**Archival.**
- **No longer block a thread** (in `store/archival.rs`):
  - joined members whose seat has no connected binding;
  - unresolved and retired seats;
  - pending invitations and obligations of those seats.
- **Still need today's idle-sample check:** members with a connected binding.
- **Human seats** count as active while their pane exists.
- **Quiet window:** unchanged (1 h).
- **Scan-churn fix:** the archival mutation revision is bumped today by *any* seat or binding write,
  which restarts the scan, so busy instances never finish one. Only changes that affect a scanned
  channel's membership or member liveness restart it now.
- Archived threads reopen as today.

## §4 Absorbing ht-4ne, migration, policy and tests

**What ht-4ne keeps:**
- the `enrollment_witness` envelope field;
- `EnrollmentPolicy`/`EnrollmentKind`;
- the witnessed CLI `on_session_start` enrollment;
- the fallback for an unowned turn.

**What changes from ht-4ne:**
- Enrollment now creates a binding keyed by the Hermes session.
- **The cross-harness context retire in `lifecycle_check_in` (ht-4ne spec D10) is removed.**
- A one-shot `hermes -z`, `chat -q` or `claude -p` inside another agent's pane **adds** a binding
  with its own process fingerprint, and disconnects when it exits.
- ht-4ne's pending decision (headless or nested runs replacing an agent's binding) dissolves.

**ht-4ne punch list, now in scope:**
- the `startup_enrollment_hermes_*` tests join the nextest group that runs them one at a time;
- the concurrent start/first-turn test detects a duplicate binding.

**ht-4ne's probe becomes a regression test** (isolated Herdr, real daemon): two agents in one pane
both keep receiving notices and ACKing, sends are attributed to each one's own binding, and the
one-shot disconnects when it exits.

**Migration V29** (`src/store/schema.rs`):
- Each open binding becomes a top-level binding keyed by its stored native session, with liveness
  `connected` and no fingerprint. On its next call it must present one, or it is disconnected the
  first time a liveness check finds no fingerprint.
- One cursor per (binding, joined thread), at the thread's current head, so nothing is delivered
  twice.
- Per-seat delivery state either moves to that binding or is dropped where the cursor replaces it.
- Receipts are unchanged, since they are per seat.
- Archived and retired data are untouched.

**Protocol.**
- **Wire version bump.** The existing skew guard makes clients fail fast against an old daemon.
- **Re-running `setup`:**
  - re-installs Claude's PreToolUse hook, now with the `--agent-id` rewrite;
  - adds the SubagentStart, SubagentStop and end-event hooks;
  - updates the Codex hooks with SubagentStart/SubagentStop/SessionEnd;
  - updates the Hermes bridge with `subagent_start`/`subagent_stop` and session-end callbacks.
- **Agents mid-session at upgrade** re-enroll on their next call, from their environment.

**TRUST-POLICY.md** is rewritten in the same commits as the invariant changes:
- **Identity and transitions:**
  - the binding definition;
  - A4: adding a binding, lineage, human exclusivity, subagents being read-only;
  - the refusal of agent calls without a session id or a fingerprint;
  - the new provenance values.
- **Per-binding behaviour:**
  - availability per binding;
  - the mod channel and wake per binding;
  - receipt settlement by a top-level binding;
  - disconnection on liveness evidence, which counts as structural evidence and not a heuristic.
- **Seat lifecycle:**
  - pane-close retirement;
  - the archival rule;
  - the shared-seat consequence (any binding's join or leave applies to the whole seat).

**Skill and docs.** `herdr-threads skill`, README and docs/agent-usage cover:
- subagents reading their own stream, read-only;
- first-contact stats and fast-forward;
- the shared-seat warning;
- the session-id requirement and the managed-daemon refusal.

**Configurations to exercise** (a `Configuration smoke:` bead, early, using an isolated Herdr):
1. Claude main session plus two subagents (the `--agent-id` rewrite), with separate cursors and
   subagent writes refused.
2. Codex embedded (thread and subagent ids) and Codex under a managed daemon (refused).
3. Hermes interactive, plus `hermes -z` inside a Claude pane: an added binding, no displacement,
   and the one-shot disconnects on exit.
4. Human pane exclusivity (`me init` refused while an agent is connected; an agent replaces a
   human binding).
5. Resume within one page, and resume past one page (fast-forward).
6. A pane closing within a live Herdr: retirement, followed by archival of a thread left with no
   active participants.

## Out of scope / follow-on

- **Codex pane environment** (`CODEX_EXEC_SERVER_URL=` in Herdr panes): branch `codex-daemon-env`.
- **herdr-graph** adopting one-seat-per-pane: herdr-graph's own work.
- **Linux `/proc` fingerprint details:** implemented if cheap, otherwise left to a follow-on bead.
  Linux real-Herdr integration is unverified today.
- **Any MCP-based attribution.**

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
