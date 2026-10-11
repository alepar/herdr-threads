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
   page, it fast-forwards instead (§2).
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
    PID ancestry picks *which* harness is calling (the innermost harness rule, §1) and supplies
    the liveness fingerprint. It never supplies the session identity itself.
13. **Subagents** (Claude Agent-tool subagents, Codex spawned threads, Hermes delegate children)
    get their own binding and cursor. They are **read-only**: they may not ACK, send, accept,
    reject, leave, invite, check in for the seat, or do any other write.
14. **Human seats (`me init`) are exclusive, as today.**
    - A human-mode seat holds only its human binding.
    - Any agent binding that is enrolled or reconnected on a human-mode seat, whether by a
      lifecycle check-in or a plain CLI or hook call, ends the human binding (`ended_at` is set).
    - `me init` is refused while any agent binding is connected, unless `--operator`. A
      successful `me init` ends the seat's disconnected agent bindings (`ended_at` is set).
    - Human bindings migrate as they are (V29) and are exempt from the liveness sweep.
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
- It is replaced by a partial unique index on (seat, harness, native_session, agent_id) over
  rows with `ended_at IS NULL`. Historical ended rows never conflict with it.
- `ended_at` is set when the seat retires, becomes unresolved or is rebound, and by the two
  human-exclusivity transitions (decision 14). A claim whose key matches an ended row on a
  restored (rebound) seat **reopens that row**: it clears `ended_at` and reconnects the binding,
  keeping its cursors, provided no open row holds the key. A resume after a human-exclusivity end
  enrolls a fresh binding.
- **V29 rebuilds `occupant_bindings`** with SQLite's 12-step table rebuild. This drops the
  table-level `UNIQUE(seat_id, generation)`, and recreates the indexes and the archival/activity
  triggers. A binding row's `generation` becomes informational: the seat's mapping generation at
  enrollment. `receipts.ack_generation` keeps its historical meaning, and new settlements use
  `ack_binding_id`.
- Disconnection is the separate `liveness` state.

**`seats.generation`.** It becomes the seat's mapping generation, bumped only by rebind,
unresolve and retire. The joins written `b.generation = s.generation`, which mean "the current
binding", are rewritten to select bindings by identity or by liveness.

**Claims come from the environment.** The CLI and every hook derive a `CallerClaim` of
(harness, native session, agent id, role, liveness fingerprint, target pane):

| Harness | Session | Subagent identification | Liveness fingerprint |
|---|---|---|---|
| Claude Code (≥2.1.214) | `CLAUDE_CODE_SESSION_ID` (equal to the hook `session_id`; updated on `/clear`) | `--agent-id=<agent_id>`, inserted right after the `herdr-threads`/`ht` command word by herdr-threads' PreToolUse hook through `updatedInput`, with **no** permission decision. The hook rewrites every simple command in the Bash line whose command word is `herdr-threads`/`ht`. It parses the line into simple commands separated by `;`, `&&`, `\|\|` and `\|`, and skips leading `VAR=value` assignments and the wrappers `env`, `command`, `nice`, `timeout`, `time` and `nohup`. Shapes it cannot recognize (a script that calls `ht`, `eval`, `$(…)`) are attributed to the parent session. This is an accepted limit, recorded in TRUST-POLICY.md. Spike-verified: existing `Bash(herdr-threads:*)` allow rules keep matching, and a hook `allow` would approve whole compound commands. | `CLAUDE_PID` plus that process's start time |
| Codex (≥0.148) | `CODEX_SESSION_ID` (the root session), with agent id `CODEX_THREAD_ID` when the two differ and empty otherwise. Hooks build the same key from `session_id` and `agent_id` | `CODEX_THREAD_ID` ≠ `CODEX_SESSION_ID` means a subagent whose parent is the top-level binding of `CODEX_SESSION_ID`. Hook payloads carry `session_id` (the root) and `agent_id` (the subagent thread). | the agent ancestor process of the caller. A process under `codex app-server --managed-daemon` is **refused**, with a hint (keep Codex in its pane: the pane-env block from branch `codex-daemon-env`, or `--no-daemon`) |
| Hermes | `HERMES_SESSION_ID` (fresh in every local command) | A delegate child is known through the `subagent_start` hook, which reports the child session with its parent. A CLI call whose session is a registered child acts as that subagent binding. | the Hermes process (ancestor of the command or hook) |
| Human (`me init`) | today's pane `OccupantContext` path | n/a | today's rules |

**Innermost harness rule.** When several harnesses' variables are present, the caller's harness
is the nearest harness process among its ancestors: `claude`, `codex`, or the Hermes process
shape. That harness's own session variable is used, and variables inherited from an outer harness
are ignored. For example, `hermes -z` run from Claude's Bash tool sees `CLAUDE_CODE_SESSION_ID`
and `HERMES_SESSION_ID`. Its nearest harness ancestor is Hermes, so it claims the Hermes session.
If the nearest harness's session variable is missing, the call is refused (decision 15).

**Refusals:**
- An agent call with no session id is refused.
- An agent call with no derivable fingerprint is refused.
- These refusals carry stable error codes and name the missing variable or harness requirement.

**The daemon decides (A2).** It matches the claim to a binding of the pane's seat:
- an existing binding is touched. A binding (re)connects on enrollment, on a lifecycle check-in,
  or on a plain call carrying its fingerprint after it was disconnected because its process
  exited or by migration (re-adoption), unless a connected lifecycle-claimed binding already
  holds that process. A binding disconnected as superseded, by an end event, by seat retirement
  or unresolve stays disconnected on a plain call: the call is served as that binding
  (attribution and cursor), and its liveness is unchanged, so a stale caller (for example a
  background process still carrying a pre-`/clear` session id) never flaps. Only an enrollment
  or a lifecycle check-in applies the same-PID supersede rule (§3);
- an unknown session is enrolled (§2);
- a subagent claim attaches to its parent binding.

**The client journal.** The per-seat client `OccupantContext` journal is retired for agent seats
(no file-based claim, no "shared context" for the pane). Human seats keep it. Hook-side attention
marks become per binding (§2).

**Lineage.** A successor session that carries the conversation forward copies its predecessor's
cursors (`lineage_from`).
- **Known lineage:**
  - Hermes compression and `/branch`: the source is the `parent_session_id` in the Hermes
    `pre_llm_call` payload. On the first turn of the successor session, the bridge reports
    (session, parent session). When the parent is a top-level binding of the same Hermes
    process, the successor inherits its cursors. A delegate child is distinguished by
    `platform="subagent"` and its `subagent_start`.
  - Claude `--resume <id>`: the same id, and therefore the same binding.
- **If the successor's first claim arrives as a plain CLI call** before its first `pre_llm_call`,
  it enrolls as a new binding at the seat maximum (decision 5). It skips nothing the *seat* has
  not displayed, but it may skip mail the predecessor had not yet displayed. This is the one
  exception to copy-before-supersede, and an accepted limit. The later `pre_llm_call` links
  `lineage_from` without moving cursors.
- **If the Hermes payload carries no compression parent** (unverified: Hermes documents
  `parent_session_id` for delegation), compression is treated as a new binding at the seat
  maximum. Lineage then covers `/branch` and delegation only.
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
- **What happens to today's per-seat delivery state:**
  - `lazy_recipients` display state is replaced by the cursor. A lazy message is unread for a binding
    until its cursor passes it. `CompleteInboxDelivery` advances the cursor.
  - The `warning_offer` and `digest_notice_offer` frontiers become per binding, keyed by
    `binding_id`.
  - `catch_up` rows become per binding: `binding_id` replaces the binding generation and execution.
  - Client `attention.json` marks become per binding.

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
recognized subagent claim. This enforces the skill rule that subagents never write, for every call
the harness identifies as a subagent's. Claude subagent calls whose shape the rewrite cannot
recognize are the accepted limit in §1.

## §3 Liveness, wake, delivery channel and cleanup

**Becoming connected:** on enrollment, on a lifecycle check-in, or on a plain call carrying its
fingerprint after it was disconnected because its process exited or by migration (re-adoption),
unless a connected lifecycle-claimed binding already holds that process. A binding disconnected
as superseded, by an end event, by seat retirement or unresolve stays disconnected on a plain
call: the call is served as that binding (attribution and cursor), and its liveness is
unchanged, so a stale caller (for example a background process still carrying a pre-`/clear`
session id) never flaps. The call records or refreshes its fingerprint.

**Becoming disconnected** (never deleted):
- **The process exited.** The daemon re-checks fingerprints of connected bindings on a timer and
  before wake decisions.
- **Same-PID supersede.** An enrollment or lifecycle check-in by a `top_level` binding disconnects
  every other connected `top_level` binding with the same harness and the same liveness PID
  (Claude `/clear`, Hermes `/new`, or resuming an older session through its lifecycle start). The
  most recent lifecycle claimer wins, and plain touches never supersede.
  - Subagent bindings never supersede, and are never superseded.
  - A lineage successor supersedes its predecessor only after copying its cursors.
- **A harness end event:** Codex SessionEnd/SubagentStop, Claude SubagentStop, Hermes
  `on_session_finalize` (session teardown) and `subagent_stop`. Hermes `on_session_end` fires
  after every turn and is **not** an end event. A supersede-induced disconnect never triggers
  fast-forward; only a lifecycle resume does.
- **The seat retires or becomes unresolved.**

There is no timeout. A binding without a fingerprint is never admitted (decision 9). Migrated
bindings start disconnected (V29), so no exception is needed. Human bindings are exempt from the
sweep (decision 14).

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
- **Pane-level composer wake** (typing into the pane): it fires only when Herdr reports the pane's
  foreground agent as idle (`agent_status`).
  - It targets the connected `top_level` binding whose fingerprint PID equals the pane's
    foreground process PID, from Herdr's pane process info.
  - That binding's unread set decides whether to nudge.
  - If no binding matches, no composer wake fires.
  - It fires at most once per seat per wake.
- **Bindings with no channel** (headless one-shots, subagents) are skipped; they pick up mail
  through their hooks and `inbox`.
- Human seats are never woken.

**Managed launch.** `record_managed_launch` no longer requires "no open binding". It records a
launch placeholder binding for the launched harness. The launched agent's first lifecycle check-in
replaces that placeholder, and only that one.

The placeholder's key is (harness, `launch:<uuid>` session). Its liveness fingerprint is the process
Herdr's guarded `agent.start` observed starting in the pane: its PID and start time, from
host process info. If no such process is observable, the launch is refused, consistent with
decision 9.

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
- **Every affected table and row class is listed with its fate:**
  - **Open agent bindings** become `top_level` bindings keyed by their stored native session,
    marked `migrated` (that session is the key re-adoption matches), with liveness
    `disconnected` (reason `migrated`). An agent's next call carrying that session reconnects the
    binding, with its fingerprint and cursors. An idle agent waiting at its prompt gets no wake
    until its next turn. This is an accepted, one-time upgrade cost, documented in the release
    notes and the docs.
  - **Archival grace:** for 24 hours after the upgrade, a `migrated` binding counts as active for
    archival, so the upgrade does not cause an archival wave.
  - **Open human bindings** migrate as they are (role `top_level`, harness human), exempt from
    the liveness sweep.
  - **Historical ended rows** get a synthetic `binding_id`, role `top_level` and liveness
    `disconnected`. They keep `ended_at`, so they stay outside the partial unique index.
  - **Receipts** gain `ack_binding_id`; **messages** gain `sender_binding_id`. Both are NULL
    before V29.
  - **Other tables that store a binding generation** (`seat_availability`, `wake_work`,
    `warning_offer`, `digest_notice_offer`, `catch_up`, `seat_archival`) are keyed by the migrated
    binding's `binding_id`, or dropped where the cursor replaces them. The owning beads (§2, §3)
    add their migrations after 0029.
- One cursor per (binding, joined thread), at the thread's current head, so nothing is delivered
  twice.
- Per-seat delivery state either moves to that binding or is dropped where the cursor replaces it.
- Receipt obligations stay per seat.
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

**Changes vs. original design (2026-10-11):**
- Migrations land as 0032–0035 (not V29–V31): main v0.8.0 owns 0029 direct_threads, 0030 automatic_handoff, 0031 seat_wake_on.
- agy (a fourth harness from main) is attributed through an adapter-owned `HarnessAdapter::caller_env()` / `CallerEnvShape` (`ANTIGRAVITY_CONVERSATION_ID`); claude/codex/hermes attribution moved behind the same adapter hook to satisfy `scripts/check-harness-neutral`.
- `/clear` wake model (ht-ozo, decided autonomously): no rebind-grace suppression; `/clear` opens a new binding at seat-max cursors; composer wake only for mail that binding has not read; `/resume` keeps the binding.
- Accepted deviations pending sign-off: a seat with a single wake target keeps the legacy composer rule (ht-54r.13); a disconnected binding is refused for explicit ACK/AckDisplayed (ht-54r.20); compact `participants` omits presence (ht-54r.16).
- Read-only is decided from the resolved binding's role, and accountable top-level means role + connected liveness rather than `cooperative_top_level` provenance (receipts/archival; mod channel and hook-pull still provenance-gated — open).
- Known open defects at hand-off are listed in docs/superpowers/runs/2026-10-09-session-keyed-seat-bindings/report.md (Remaining).
