# herdr-threads trust policy

Status: adopted 2026-10-01; B5 guards implemented (epic `ht-rzi`). Normative for seat continuity, caller
attribution, receipt provenance and operator repair. Where an older design document requires adversarial
proof of who is calling, this policy supersedes it. C5's guard is owned by B4 (`ht-p03.2`) and is the one
guard still marked **required**. Amended 2026-10-02 for thread summaries and deadline extension (epic ht-1ip), and 2026-10-03 for the
`managed_launch` binding (ht-5n6), and 2026-10-03 for human receipt waivers, and 2026-10-10 for cooperative peer-data wording (ht-poh.41). Amended 2026-10-10 for the session-keyed binding vocabulary (ht-54r; implemented
end to end, swept by ht-54r.25: every provenance value below has a production writer), and 2026-10-11 for absorbing
the agy adapter into it (agy's conversation id is a harness session variable under `cooperative_env_session`).
Amended 2026-10-12 for the text accept's thread-scoped opening-mail display (ht-br0), bounded by the display walk's
ordinary stop and, with a live mod channel, by a bounded channel grace.
Amended 2026-10-12 for the consented Codex pane environment block (foreground configuration only; no seat,
binding, receipt or attribution rule changes).
Amended 2026-10-13 for message priority delivery (ht-yha.3): delivery rows are the bookkeeping of every message and
audience member, and receipts exist only for explicit ACK recipients (A8, A5a, A6).
Amended 2026-10-13 for the one-process composer wake (ht-327): several bindings of one process share its
composer for seat-level attention; no binding is chosen, merged, moved or ended.
Amended 2026-10-14 for service sends under D9 (ht-yha.47): a service send's receipts are only its explicit
non-human `recipients`; its joined snapshot gets priority-0 delivery rows (A8).
Amended 2026-10-14 for per-session pending views held only by bindings that can still be offered rows
(ht-yha.54): a binding disconnected other than by process exit drops its view and is re-seeded on reconnect;
no delivery or wake decision reads the seat-wide delivery-row state (A8).
Amended 2026-10-14 for connected-only pending views (ht-yha.57): every disconnect, process exit included, drops
the binding's view and its next connected transition re-seeds it from its cursors and display marks; a
disconnected binding costs publication nothing (A8).

## Abstract

herdr-threads gives coding agents in Herdr panes a durable mailbox: threads, invitations, messages that an
agent recipient may be required to explicitly ACK, deadlines, and a safe wake when something is pending. Every one of those
features rests on two questions the system must answer about each request: *which seat is this*, and *who
acted*. A seat is a long-lived role ("the reviewer in the right-hand pane") that has to outlive everything
that happens to the processes behind it: daemon restarts, Herdr restarts, pane moves, `/clear`, session
resume, and an operator's repairs. An ACK is only worth recording if it says something true about who gave it.

The first design answered both questions adversarially: the daemon would prove, service-side, that a caller
was the top-level agent of its pane and not a subagent, a script or another agent impersonating it. That proof
needed evidence the host does not provide (current-execution attestation, synchronous transcript roots) and it
defended against an opponent who does not exist in the deployment: every agent, script and daemon involved
runs as the same local user, on the same machine, on behalf of the same person. Code running as that user can
already read the database, edit the files and type into the panes. A boundary against it is theatre.

The vision is therefore **cooperative honesty with conservative continuity**:

1. **Cooperate, don't police.** Agents are told what they may do (the top-level agent accepts and ACKs;
   subagents read and summarize) and are trusted to do it. The system does not try to stop a same-user
   program from lying. It refuses to fabricate evidence it does not have.
2. **Record claims as claims.** Every receipt says *how* it was attributed: a prompted top-level claim, a
   person who declared their pane human, a local-account operator decision. Provenance is the product; a
   reader of the record must be able to tell an agent's ACK from a person's.
3. **Never lose a role silently, never invent one.** When continuity is uncertain the system holds and asks;
   it does not guess, merge, or let a newcomer take a seat's place. Losing a seat strands its threads and
   obligations; misattributing one corrupts the record. Both are worse than a short wait for the operator.
4. **Decide against one view.** Each decision is made by the daemon, inside one transaction, against one
   canonical picture of the world: the daemon boot the caller addressed, the caller's own canonical binding, and the
   effective host observation. Clients and local files are hints.
5. **Use only evidence that exists.** A transition may depend only on evidence the production host adapter
   actually produces. Heuristics may suggest; they never move a seat or end a binding.

What follows turns that vision into invariants, names who may do what, and states the accepted limits
explicitly, so that a weakness is a documented decision rather than a surprise.

## Vocabulary

- **Seat**: a durable plugin identity for one role. Threads, invitations and receipts belong to seats.
- **Target**: a Herdr pane address the seat is currently mapped to. A locator, not an identity.
- **Binding**: one occupant record of a seat, keyed by (seat, harness, native session, agent id) with the
  agent id empty for a top-level session. A seat holds many bindings, each with its own `binding_id`, role
  (`top_level` or `subagent`, a subagent pointing at its parent binding), execution, the mapping generation,
  the daemon boot and host epoch it was recorded under, and its provenance. Liveness (connected or
  disconnected, with a process fingerprint) is a separate property from identity: a disconnected binding is
  still that session's binding. `seats.generation` is the seat's *mapping* generation, bumped only by rebind,
  unresolve and retire, never by a check-in.
- **Herdr boot / incarnation**: one Herdr server process. A Herdr restart is a new incarnation; terminal ids
  and pane addresses from the old one prove nothing about the new one.
- **Daemon boot**: one herdr-threads daemon process. A daemon restart does not change the Herdr incarnation.
- **Operator**: a request made with `--operator`, accepted when the kernel peer UID matches the daemon owner.
  This is the local account, not proof of a person.

## Continuity invariants

**C1. How a seat's mapping may change.** Exactly three ways:

| Path | Trigger | Recorded as |
|---|---|---|
| Structural reconfirm | Same terminal, same Herdr boot and incarnation | automatic, no new provenance |
| Cooperative continuity | A resumed top-level session (SessionStart source `resume`) whose harness session id uniquely matches a top-level binding (any of its bindings, not only the latest) of an unresolved seat; when several unresolved seats match, only those on which that session's binding was still open when the seat became unresolved (disconnect reason `seat_unresolved`) count, if there are any | `cooperative_continuity` |
| Operator decision | `ht human seat rebind`, `ht human seat resolve --new-seat`, `ht human seat retire`, `ht human seat rebind --replace` (each with `--operator`) | `operator:local-user:<uid>` |

Seats are never merged. Pane labels, saved addresses, terminal-id hints and Herdr's agent field may *suggest*
candidates in diagnostics; they never move a seat, allocate one, or end a binding.

**C2. Restore holds.** After a new or unknown Herdr incarnation while nonretired seats exist, every saved
seat whose continuity is not structurally proven becomes unresolved, and the daemon takes a coherent baseline
of current targets. Every unowned target in that baseline is **held**: ordinary resolution (`seat resolve`,
`launch`, top-level `SessionStart` enrollment, `ht human me init`) refuses it with inspect / rebind / fresh-seat guidance. Panes authoritatively created
after the baseline are not held. Holds are durable and survive daemon restart. A hold is released by:
operator rebind or fresh seat on that target; cooperative continuity on that target (C1); or, instance-wide,
when no unresolved nonretired seats remain (implemented, nothing left to protect).

**Pane-close retirement (ht-54r.14).** A pane absent from a coherent snapshot of the *same* Herdr
incarnation retires its seat, however many session bindings the seat holds. The retirement fence disconnects
every connected binding with reason `seat_retired` through the one `store::bindings::disconnect` transition
(side-effect hooks run) and ends every open binding; a binding already disconnected keeps its own reason.
Memberships retire, receipts settle recipient-retired and invitations close, all seat-level. The seat record and
every `binding_cursors` row persist, for undo and herdr-graph. A new or unknown incarnation never retires: it
yields unresolved seats under the C2 restore holds above, and each binding ends with reason `seat_unresolved`
so the same session can reopen it after a rebind.

**Startup enrollment.** Installed Claude/Codex top-level `SessionStart` lifecycle hooks enroll
`HERDR_PANE_ID` through the same journaled ordinary canonical guarded seat resolver as `launch`.
It freshly reads the actual target and decides against A2: reuse its resolved seat, or allocate only
when ordinary resolution permits a genuinely new pane. An unresolved mapping, restore hold or collision
is never bypassed by creating, moving, rebinding or retiring a seat. A resumed session attempts C1
cooperative continuity first, even when the client's seat listing appears resolved; pending recovery,
ambiguous matches and unusable recovery evidence stop enrollment. A confirmed no-match may proceed
only through ordinary guards; a resume without a usable native session cannot allocate a seat.
Resolution and check-in retain their separate durable operation journals: concurrent/replayed startup
and response loss cannot split a pane's role; an uncertain resolution remains explicitly retryable.
A historical resolution result is revalidated against the current pane mapping before check-in.
A Hermes CLI top-level `on_session_start` (platform `cli`, the bridge's `cli_session_start`
association) enrolls through the same resolver without a check-in or context offer; a qualified
top-level CLI `pre_llm_call` enrolls the same way only when the pane has no nonretired seat
(`Unowned`), then checks in in the same callback. Both meet the same guards; a held or unresolved
pane is refused.
Enrollment by a plain call records `cooperative_env_session` provenance on the binding. Receipt authority follows
A5 (a connected top-level binding of the seat, whatever its enrollment provenance); a receipt's own provenance comes
from the claim (A3), not from the binding's enrollment.
Subagent callbacks, `PreToolUse` and Current/compact checks never allocate; the exact installed event
registration must match the native payload before service or journal access.

**C3. Collisions resolve by abandonment, never by merge.** When a rebind finds its target owned by another
live seat, the refusal offers exactly two resolutions, each as ready argv:
- abandon the old seat: `ht human seat retire OLD --operator` (implemented);
- abandon the new role: `ht human seat rebind OLD --pane P --replace NEW --operator` (implemented), which retires NEW
  and rebinds OLD in one decision so the target cannot be claimed in between. NEW's pending obligations settle
  as recipient-retired; nothing moves from NEW to OLD.

**C4. Availability ends only on evidence.** A joined seat stays available across a daemon restart when its
mapping is structurally reconfirmed; the open cooperative or operator binding carries forward to the new host epoch (implemented; a native binding re-registers instead, and a send before the first reconciliation pass assumes the carry rather than warning). A `managed_launch` binding is never availability, so it is not carried; it stays open and unregistered until a check-in replaces it. Availability ends when the mapping becomes
unresolved, the seat retires, or the binding disconnects or ends. Availability is derived per binding: a seat is available while any open, connected, registered binding of it is confirmed by the effective observation, so the last connected binding disconnecting (for example on liveness evidence) opens the seat's unavailability episode, and a check-in on a connected binding closes it (a plain-call reconnect outside a check-in leaves the episode open until the binding's next check-in; derived availability follows liveness at once).
Herdr not answering (a timed-out or refused connection, a read that finishes past its budget) and a
daemon-side failure to stage a capture are missing evidence, not evidence of change: they write no host
invalidation, so seats, bindings and the published view stay **frozen** until Herdr answers (implemented,
ht-yms). Meanwhile anything that needs a live Herdr read (current-target resolution, continuity) is
refused as transient, and wake prompts and pokes are not attempted: the wake lane is frozen, with no reservation,
refusal record or Herdr call, until the observation lane's first answered capture kicks it (implemented,
ht-72q); store decisions that need no host read (sends, ACKs) proceed. Only
evidence invalidates: an incomplete enumeration, an unknown or new incarnation (C2), an incoherent capture.

**C5. Only producible evidence drives transitions.** The production Herdr adapter reports structure
(terminal, incarnation, generation) but not occupancy or current execution. Code paths that need an observed
empty shell or a verified execution do not exist in production and are removed, not kept as dead branches
(**required**, folded into the removal of the pre-cooperative verification layer). Consequence: an agent that
exits back to its shell stays bound until a check-in replaces it or its pane disappears.

## Attribution invariants

**A1. The caller is the pane's seat, cooperatively.** A command acts as the seat mapped to `HERDR_PANE_ID`
(never the focused pane), or as the seat named by an explicit selector. Any process running as the user can
act as any seat: a child agent, a script, another seat's agent writing the shared client directories. This is
in-contract (see Accepted limits).

**Invocation spelling.** Ordinary root commands act as agents. Person and local-operator
commands require immediate argv[1] `human`: `ht human me init [--operator]`,
`ht human send THREAD --body TEXT`, and `ht human seat rebind ... --operator`.
Put `--state-dir`, `--host-endpoint` and output flags after `human`. `--human` changes
output formatting only; relay, user-intent and delivery flags do not declare a person.
Root accountable actions refuse inferred Human contexts. The namespace records no new
provenance and confers no canonical authority. Subagents read with `--machine` or `--json`
and never mutate, accept or ACK.

Retry classification reads the original validated frozen semantic and scope before
state creation, connection, completed presentation or cleanup. Retained person/operator
intents use `ht human retry REF`. A historical Agent intent remains Agent after a binding
change. Presentation may clone known command argv fields, but never changes the original
claim, operation key, digest, response bytes, cursor or rows, or rewrites peer text.

**A2. One canonical decision view.** Every accountable mutation is decided by the daemon, in its deciding
transaction, against:
- the daemon boot the client addressed: a request carrying a different expected boot is refused before
  dispatch (implemented);
- the seat being resolved, nonretired and not on a held target;
- the exact binding generation, harness, harness session and execution of the open binding;
- the effective host observation (published snapshot or newer current-target read) at the current host boot
  and epoch, never the raw per-target rows.

Client-local state (`contexts/`, `intents/`) only selects what to ask; it never authorizes. Agent claims come
from the harness environment (`cooperative_env_session`: `CLAUDE_CODE_SESSION_ID`, `CODEX_SESSION_ID`,
`HERMES_SESSION_ID`, `ANTIGRAVITY_CONVERSATION_ID` for the CLI; the payload's session id for hooks) with the harness process's liveness
fingerprint; the pane context journal remains a human-seat hint only, and an agent's own pending check-in
records there are replay machinery, never its identity.

**A3. Provenance values.** Exactly these, each with one meaning:

| Value | Where | Meaning |
|---|---|---|
| `cooperative_top_level` | bindings, receipts | The pane's top-level agent claimed the action through its hook-registered check-in. Prompted, not proven: a disobedient child is indistinguishable. |
| `cooperative_inbox_display` | receipt action observation | The selected top-level agent's text `inbox` command fully wrote and flushed a bounded page before claiming that its listed complete messages were displayed. It is a cooperative output claim, not proof that the model consumed the text. The receipt retains the binding's observation provenance separately. JSON, machine, foreign-seat and other read commands do not make this claim. |
| `cooperative_mod_channel` | mod channel registrations only (process-local) | The pane's top-level session of a harness whose adapter declares a push delivery channel (today Claude), through that harness's bundled herdr-threads mod's `watch` child, opened a delivery channel for its own binding. Channels are held per binding: every connected top-level binding of such a harness on a seat may hold one, and its deliveries, closes, grace and stall cooldown are that binding's alone. The daemon decides the registration against A2 (seat resolved and not held; the claim's key of seat, a harness that declares a push delivery channel, native session and no agent id names an open, connected, top-level binding of that harness (whatever its observation provenance); no stall cooldown for that binding; `mod_delivery` on). A subagent claim, an unknown session or a disconnected binding holds no channel, and a binding that disconnects or ends loses its channel. It grants nothing but receiving that binding's deliveries and making `cooperative_mod_delivery` claims through that binding. Never on bindings or receipts. |
| `cooperative_mod_delivery` | receipt action observation; lazy completion claim | The mod reported that a delivery path's predicate held (spec D6): for an ordinary message, that its full body entered the model's context through the mod (an answered tool result carrying the mod's context, or, while idle, a `$.session.append` of its frame that resolved without `deny` (the mod then submits the content-free wake prompt `herdr-threads: wake up`, which carries no item); the `$.prompt.submit` wording applies only to a predecessor record written by an older mod instance: a submit that resolved without `drop`; or, after a plugin reload disposed the instance whose submit was still in flight, a main turn whose prompt carries the mod's frame naming the id, or the completion of the turn that was open when the successor loaded (the engine does not report the disposed instance's submit outcome)); for a lazy row, that it was appended to the transcript (`$.session.append` resolved without `deny`). The ACK is decided against the channel's own binding (the one the claim's key names) and the receipt records that binding as its ACK binding; the seat's receipt still settles once, so a second binding's ACK of the same id is `already_settled`. A cooperative delivery claim, not proof that the model read it. The receipt keeps the binding's observation provenance separately. A truncated item never carries it. |
| `cooperative_hook_delivery` | receipt action observation; lazy completion claim | The top-level hook of a harness whose adapter declares hook-pull delivery reported that it found the id's frame header at the start of a line of a hook-injected step it decoded from the session's transcript, after a `DeliveryPull` showed the id (spec delivery-policy DP5/DP6). The daemon decides it against A2 and additionally requires that it recorded the id as offered to that binding (the connected top-level binding the claim's key names, at its binding generation) by a `DeliveryPull` (process-local offered ledger). A cooperative delivery claim, not proof that the model read it. The receipt keeps the binding's observation provenance separately. A truncated item never carries it. |
| `cooperative_hook_context` | lazy completion claim | A top-level standard native hook **of a harness whose adapter declares no hook-pull delivery** wrote and flushed output whose context carried the lazy message's complete body, and the adapter reported that the output carries context (A8). Claimed by `CompleteInboxDelivery.via = hook_context`, which is part of the accountable operation's payload digest (as `AckModDelivered` carries the mod claim); no per-row source is stored. Not proof that the harness ingested it or the model read it. A prefix never carries it. The daemon refuses this claim from a hook-pull binding. |
| `operator_human` | bindings, receipts | A person declared this pane human with `ht human me init` and acted from it. Best effort: refused where the system sees evidence of an agent (A4). |
| `cooperative_continuity` | seat rebinds only | The seat was reattached because a resumed harness session id matched (C1). Never on receipts. |
| `operator:local-user:<uid>` | audit of administrative decisions | The local account made a repair or recovery decision. Never on receipts. |
| `managed_launch` | bindings only | Herdr's guarded `agent.start` in this pane was observed starting the harness; the agent has not checked in. Never on receipts; authorizes nothing but a wake prompt (an ordinary wake or a soft-deadline poke) to the bound harness. The daemon records it only after the launcher's host-correlated `ObservedStartup` (never on an unconfirmed start), on any seat, beside whatever bindings the seat already holds, decided against the effective observation (A2). It carries the liveness fingerprint (pid and start time, from host process info) of the one process Herdr reports in the pane's foreground besides its shell; with none or several observable, nothing is recorded. Its session and execution are `launch:` placeholders no caller claim can match, it has no `registered_at`, it keeps the seat generation, and it establishes no availability. Receipt deadlines start at send time independently of this binding. |
| `cooperative_env_session` | binding claims | The CLI or a hook took the caller's session id from its harness environment or hook payload (`CLAUDE_CODE_SESSION_ID`; `CODEX_SESSION_ID` with `CODEX_THREAD_ID`; `HERMES_SESSION_ID`; and the session variable a registered adapter declares in its caller shape, today agy's `ANTIGRAVITY_CONVERSATION_ID`), choosing the innermost harness among the caller's ancestors. Cooperative and prompted, not native attestation. Never on receipts. |
| `cooperative_hook_agent` | binding claims | herdr-threads' own Claude PreToolUse hook inserted `--agent-id=<agent_id>` into a recognized `herdr-threads`/`ht` command, attributing the call to that subagent. Shapes the hook cannot recognize stay attributed to the parent session. Never on receipts. |
| `liveness_process_exit` | binding disconnections only | The binding's fingerprinted process (pid and start time) no longer exists, or the pid now has a different start time. Structural evidence; it disconnects the binding and never ends it, moves a seat or merges seats. |
| `liveness_end_event` | binding disconnections only | The harness reported an end event for this session (Codex SessionEnd/SubagentStop, Claude SubagentStop, Hermes `on_session_finalize`/`subagent_stop`; never Hermes `on_session_end`, which fires after every turn). The hook sends a `BindingEvent` for the pane's seat; the daemon disconnects the named binding (a session end also disconnects the subagents attached to it) and never ends it, moves a seat or merges seats. An unknown or already disconnected binding is a no-op. A `BindingEvent` is not an accountable seat write, so a subagent claim may report one; a subagent start enrolls only the subagent binding (the parent first, if needed). |
| `canonical_binding_composer` | archival sample association only | The daemon associated a fresh registered-parser composer read with an already registered cooperative top-level binding after exact canonical binding, structural, activity and timing validation. The harness/session/execution in the association come from that binding; the host does not attest occupancy or current execution. This value authorizes no seat, binding, membership, receipt or invitation transition. |
| `daemon_lifecycle` | automatic archive events only | The daemon applied the configured quiet-channel lifecycle policy after complete canonical validation. No actor seat, binding or receipt claim; no membership or continuity change. |
| `legacy_local_journal_hint` | handoff archival vetoes only | A bounded read-only scan found a pending compound in this instance's intents directory. It can only prevent automatic archival; it authorizes no action and claims no native execution. |
| `derived_summary` | summary blocks only | The block was written by an agent acting for the seat (the top-level agent or a child summary worker, which the CLI cannot tell apart) under the seat's claim, with the model it declared. Never on receipts, never delivery, and never authority for any state change other than storing that block. Submission validation bounds what a block can claim. |

**Recorded message claims.** `messages.author_role` (`human`, `agent`, `service`) and `messages.relays_user`
are claims recorded at send time in the deciding transaction: `author_role` is `service` for a programmatic
sender, otherwise it comes from the sender's open binding (a `human` harness binding, i.e.
`operator_human`, gives `human`; any other binding gives `agent`); a sender with no open binding and seat-less
system events record none. Messages written before the
summary migration carry a **backfilled** role (`author_role_backfilled = 1`) derived from the sender's binding
covering the message's decision time, and none (read as `agent`) where no binding covers it. `relays_user` is
set by `send --relays-user`; like everything an agent sends it is a cooperative claim (A1); service sends
record 0 and pre-migration rows are 0. Neither field authorizes anything; together they only rank a message as
*priority* for summaries and for the catch-up hold bypass.

**Recorded human intent.** Optional `messages.user_intent` (`query`, `request`, `rule`) is a
cooperative send-time claim under the existing caller provenance, independently of author role and
`relays_user`. The daemon decides eligibility against the canonical send-time binding: ordinary Human
may classify with or without relay, ordinary Agent needs relay; services and system events cannot
classify. Missing intent remains unclassified, including every historical message; there is no
historical inference, body parsing or adversarial verification. Intent changes neither attention
priority nor receipt attribution, and grants no permission or authority.

Under `derived_summary`, classified query/request sources are deterministic open work and rules are
active constraints within their thread. Workers compare supplied live sources (including earlier
unstored chunks) with current chunk messages. Query/request resolution cites a later answer/completion
or explicit human/relayed cancellation/withdrawal/replacement; an agent can report an answer or
completion, but cannot unilaterally withdraw a human question. A replacement question has its own
source. Partial answers, promises, silence and ACKs are insufficient completion evidence. Rules can
only be superseded by later ordinary human/relayed input with `rule_change: withdrawn|replaced` and an
exact nonempty quote; compliant behavior does not end a rule. The worker judges semantic closure and
the daemon validates structure, status and citation evidence, under the existing `derived_summary`
provenance. Unclassified sources retain legacy Done/Superseded behavior without being labeled rules.
No send or inbox operation automatically resolves this work, no classified source is duplicated as
model open work, and summary closure never changes receipt state or installs global instructions.

**A4. Binding-kind transitions.**
- *Binding transitions (ht-54r.3)*, decided by the daemon against the canonical view (A2):
  enrollment adds a binding for an unknown (seat, harness, session, agent id) and never ends another one,
  idempotently per key; a lifecycle check-in reconnects and re-registers the session's own binding in place;
  a plain call touches its binding and refreshes its fingerprint, and reconnects it only when that binding is
  open, on a resolved seat, disconnected for `liveness_process_exit` or `migrated`, the call carries a liveness
  fingerprint and no connected lifecycle-claimed top-level binding (a `managed_launch` placeholder does not
  count) of the same instance and harness already holds that fingerprint (spec §3). A binding disconnected `superseded`, `liveness_end_event`, `seat_retired`,
  `seat_unresolved` or `replaced` never reconnects on a plain call: the call is served as that binding
  (attribution and cursor) and its liveness is unchanged, so a stale caller never flaps it. A plain reconnect records the call's fingerprint and keeps the binding's lifecycle claim
  time, placement, provenance and cursors; it runs no enroll hook, so it never supersedes, never replaces a
  placeholder and never fast-forwards; a session unresolved with its seat and seen again after the seat is rebound reopens
  its own binding (ended only as `seat_unresolved`; cursors kept), while a binding ended any other way is
  never reopened; a subagent claim attaches a `subagent` binding to its parent (enrolling the parent first
  when it is unknown), attachment only; a session id seen on another seat gets a new binding on that seat and
  nothing moves or merges (cross-pane resume). Lineage (copy-before-supersede of a predecessor's cursors) is
  specified under *Lineage* below (ht-54r.20, ht-54r.17). A person and an agent still never hold a seat
  together and the first agent lifecycle check-in replaces `managed_launch` (both below, ended as
  `replaced`, through the enroll-hook registry and the one disconnect transition; ht-54r.4). A plain human call never enrolls a binding. Re-adoption (ht-54r.24) is the plain reconnect of a V32-migrated binding (`migrated`, disconnected for reason `migrated`); it also refreshes the placement and seeds its cursors (`Readopt`), and happens on any agent call whose claim matches its stored session exactly (the key the migration kept), unless a connected lifecycle-claimed binding already holds the call's fingerprint (then the row stays `migrated`); a claim with another session enrolls its own binding and nothing is matched by heuristic.
- *Binding liveness (ht-54r.10)*: a binding is connected or disconnected. Disconnection never ends a
  binding, moves a seat or merges seats, and every disconnect (end event, process exit, pane close,
  supersede, seat retirement or unresolve) goes through the one `store::bindings::disconnect`
  transition, which records the reason (`liveness_process_exit`, `liveness_end_event`, `seat_retired`,
  `seat_unresolved`, `superseded`, plus `replaced` and `migrated`) and runs the registered side-effect
  hooks. A binding (re)connects on enrollment, on a lifecycle check-in, or on a plain call
  carrying its fingerprint after it was disconnected because its process exited or by migration
  (re-adoption), unless a connected lifecycle-claimed binding already holds that process. A binding
  disconnected as superseded, by an end event, by seat retirement or unresolve stays disconnected on a
  plain call: the call is served as that binding (attribution and cursor), and its liveness is unchanged,
  so a stale caller never flaps. Same-process supersede: an enrollment by a lifecycle check-in, or a lifecycle check-in, of a
  top-level binding disconnects every other connected top-level binding of the same instance and harness with the same
  process fingerprint (pid and start time). This is structural evidence, not a heuristic (a reused pid
  has a different start time and never matches); the most recent claimer wins, so switching back to an
  older session in the same process disconnects the one just left. A plain call never supersedes: a
  binding enrolled by a plain call (provenance `cooperative_env_session`, recorded honestly and never
  rewritten by a later tool-boundary check-in) never supersedes; in the liveness worker's same-process
  rule it ranks below every lifecycle-claimed binding of that process (it can lose, never win; this
  includes a V32-migrated binding re-adopted by a plain call), a `managed_launch` placeholder ranks the
  same way (it is no session: it can lose, never win), and a group with no lifecycle-claimed session is
  left as it is, so a delegate child's early CLI call never disconnects its parent and a placeholder never
  disconnects the session of its process. When `subagent_start`
  registers a child session, the child's connected top-level plain-call row on that seat is disconnected
  `superseded` (the registered subagent binding replaces it), so the child is read-only from then on.
  Subagents never supersede and are never superseded, and a supersede-induced disconnect never triggers
  fast-forward. A superseded binding never reconnects on a plain call.
  A lineage successor
  supersedes its predecessor only after copying its cursors (except the plain-first late link, see
  *Lineage*). A seat is active while any open binding is
  connected, or while its open human binding sits on a resolved seat.
- *Liveness worker (ht-54r.23)*: the daemon re-checks the process fingerprint of every connected,
  open, non-human binding on a cadence (5 s) and, for the candidate's seat, before each wake decision.
  A binding whose process no longer exists, whose pid now has another start time, or that is
  connected with no fingerprint is disconnected `liveness_process_exit` (decision 9: every
  connected agent binding without a fingerprint is disconnected, migrated or not; the migrated 24 h
  grace is archival only; human bindings stay exempt). A C1 reattachment is a lifecycle check-in
  through the general resolver: it reopens the session's own binding when the unresolve ended it (its
  cursors kept), else enrolls one at the seat's read position, runs the enroll hooks, and records the
  hook's current fingerprint (a retried intent submits the current one, never the journaled one); a
  reattachment request without one is refused `liveness_fingerprint_required` (decision 9). Among connected top-level bindings
  of one instance and harness sharing one fingerprint, the latest lifecycle claim stays connected
  and the others, plain-call-only bindings and `managed_launch` placeholders included, are disconnected
  `superseded`; neither of those two kinds ever wins, and a group with no lifecycle-claimed session is
  left as it is (subagents excluded). Disconnection on liveness evidence
  is structural evidence, never a heuristic: it goes through the one `store::bindings::disconnect`
  transition, and the worker never ends a binding, retires, moves or merges a seat. It never judges a
  human binding (a person has no harness process), and it skips seats that are unresolved or retired
  and every seat of an instance under a recovery hold, so a Herdr restart never disconnects through the
  worker: the reconciler's C2 path owns those seats. A verdict is applied only if the binding is still
  connected with the fingerprint it was judged on. Availability follows liveness per binding (C4).
- *Refusals*: an agent call without a harness session id (empty, a `launch:` placeholder, and from
  a `plugin_context:` session), reads included, is refused with `session_claim_required`. An agent
  call that would enroll, reconnect or reopen a binding without a liveness fingerprint is refused with
  `liveness_fingerprint_required`. Both rules are enforced: every CLI call and hook of an agent sends its
  environment's session and fingerprint. The CLI refuses before any daemon call an agent whose session id is
  unset (`session_claim_required`), whose harness process cannot be fingerprinted
  (`liveness_fingerprint_required`), or that runs under `codex app-server --managed-daemon`
  (`managed_daemon_refused`, client-local). A process with no harness ancestor and no harness marker is a
  person and keeps the pane-context path; a person's shell cannot act through an agent seat.
- *Subagent read-only* (ht-54r.20, ht-54r.58): the daemon decides read-only from the role on the binding
  row it resolves (A2), at every accountable entry point: check-in, handoff, create thread, invite, join,
  accept, reject, send, ACK, display ACK, mod-delivery ACK, completion of an inbox delivery, leave, topic
  and name changes, archive and reopen. A claim that resolves to a `subagent` binding row (a Hermes
  registered child whose CLI claim is top-level-shaped, say) is refused `subagent_read_only` in the
  deciding transaction. A subagent-shaped claim (the declared subagent role, or any agent id) is
  additionally refused before dispatch: an early refusal with the stable code, never the authority.
  Subagents read, summarize and advance their own binding's cursor: a subagent's text `inbox` displays
  its own stream and sends only `AdvanceCursors` (never a display ACK or a delivery completion).
  Where the subagent has no stream of its own (no recognizable agent id in its environment, or a daemon
  without per-binding cursors), its text `inbox` is the plain read-only read: it carries no claim, moves
  no cursor and settles nothing.
  Client-side role derivation fails closed: a malformed subagent marker (a `CODEX_THREAD_ID` that is not
  a valid id) is refused, never read as the top-level session. This is cooperative, on the recognized
  claim: a Claude shape the hook cannot recognize stays attributed to the parent session (the accepted
  limit from ht-54r.6) and is not refused.
- *Receipt settlement by a top-level binding* (ht-54r.20): receipt obligations stay per seat. The first
  connected top-level binding of the seat to ACK or fully display a message settles the obligation, once;
  the receipt records that binding as `ack_binding_id` (beside the historical `ack_generation`, which is
  the mapping generation). A later ACK of an already settled obligation, by any binding, succeeds with no
  change and the first `ack_binding_id` stays. An ACK by a binding disconnected by
  process exit or migration reconnects it first (a plain call carrying its live fingerprint does) and then
  settles. A binding disconnected as superseded, ended by a harness end event or retired with its seat is
  not reconnected by a plain call (only a lifecycle check-in of that session reclaims it), so its ACK is
  refused as a stale-context ACK (`caller_unverified`), as is a claim without a fingerprint. Each ordinary send records the acting
  binding as `messages.sender_binding_id`; service-authored sends leave it empty.
- *Lineage* (ht-54r.20): a successor copies its predecessor's cursors, then supersedes it, only when the
  predecessor is a top-level binding of the same seat and harness with the same process fingerprint (pid
  and start time) as the successor's claim; otherwise the hint is ignored and the successor seeds fresh.
  Only a successor born with the lineage copies cursors; linking an already enrolled successor records
  `lineage_from` and moves no cursor (it keeps its own read position, so it never passes mail the
  predecessor displayed after the successor enrolled; main review I2).
  Hermes transport (ht-54r.17): the hint is the `parent_session_id` of a non-delegate top-level
  `pre_llm_call` (a compression or `/branch` successor), carried as `CheckIn.lineage_parent`. The daemon
  resolves it to the open top-level Hermes binding of that session on the claim's seat and verifies it as
  above; a delegate child (`platform` `subagent`) is answered locally by the bridge and is never lineage,
  and a successor whose first claim is a plain CLI call enrolls at the seat maximum and is linked by its
  next `pre_llm_call` without moving its cursors (accepted limit: it inherits the seat maximum, not the
  predecessor's position). Without a parent in the payload, `/new` and compression alike are
  a new binding.
- *Shared seat*: memberships, invitations and receipts belong to the seat, so any binding's join or leave
  applies to every binding of the seat. Bindings of one seat are not separate participants.
- *Human to agent* (ht-54r.4): a human-mode seat holds only its human binding. Any agent binding enrolled or
  reconnected on the seat, and any plain agent call there (a touch included), ends the human binding
  (disconnect `replaced`, then `ended_at`). Allowed. A resume after that end enrolls a fresh binding: an ended
  row is never reopened. Human bindings are exempt from the liveness sweep (no process fingerprint; the seat
  stays human-held until an agent replaces it). Receipt waivers and the wake exclusion key on "the seat holds
  an open human binding", not on the seat's latest binding.
- *Agent to agent, same seat*: a lifecycle check-in (startup, `/clear`, resume) adds the session's own binding, or reconnects it; it
  disconnects another binding only through the same-process rule or lineage (A4). Allowed. A check-in by a different
  agent harness on the same seat adds its own binding; no client-local context of another harness is retired.
  A `managed_launch` placeholder is replaced by the first top-level agent binding of the same harness that a
  lifecycle check-in enrolls or reconnects on the seat, and only by that: other bindings, other harnesses'
  placeholders and the person's binding are untouched, a Current (tool-boundary) check-in never adopts or
  registers it, a `launch:` session is never a caller, C1 continuity never matches it, and it never supersedes
  a session of the same process (it is no session). The launch keeps the seat generation, so no generation
  allowance exists (ht-54r.4).
- *Agent to human*: the daemon refuses a human lifecycle check-in while any agent binding of the seat
  (`cooperative_*` or `managed_launch`) is connected, unless the request is `--operator`; a successful
  `ht human me init` ends the seat's disconnected agent bindings (`ended_at`), and with `--operator` also the
  connected ones (disconnect `replaced` first) (implemented, ht-54r.4). `ht human me init` additionally refuses
  when agent evidence is present: one of the person-guard environment markers registered adapters declare (today `CLAUDECODE`,
  `CODEX_SANDBOX`, `CODEX_SANDBOX_NETWORK_DISABLED`, `ANTIGRAVITY_AGENT`), or Herdr reporting a registered adapter's recognized native host kind in the
  pane. Registry metadata alone never establishes native recognition, continuity or caller authority.
  Unregistered agent kinds are not evidence, and a failed Herdr read counts as no evidence. `--operator`
  overrides the refusal through a local per-execution mark set only after the daemon accepts the request
  (implemented, best effort, client-side).
- *Second agent*: `launch` refuses to start an agent for a seat whose bound agent (`cooperative_top_level`
  or `managed_launch`) Herdr reports live in another pane (implemented).
- *Wake*: a wake prompt goes only to a native host kind declared by the bound registered adapter
  (implemented). Recognized aliases do not change the binding or merge seats. A missing composer
  provider defers ordinary idle/done wake; soft poke and stash/restore also send no keys.
  *Per binding* (ht-54r.13): a seat may hold several bindings, and a wake is decided per connected
  top-level binding. A wake is reserved per seat (the fence is the seat's mapping generation, so a binding
  enrolling or reading its mail does not void it) and names as targets the open, connected, top-level,
  non-human bindings placed on the observed terminal that have something to act on (seat-level attention
  such as an invitation, receipt or warning reaches every one of them; otherwise the binding's own unread
  mail). Subagent, disconnected, ended and human bindings are never targets, and a seat that holds a human
  binding is never woken. A target whose mod channel is live and not stalled is woken through that channel
  and is never also typed to. The composer wake (typing into the pane) goes at most once per seat per wake:
  only to the one target whose fingerprint pid is among Herdr's pane foreground processes (the non-shell
  entries of `pane.process_info`; a harness launcher and its native child, and any helper the harness
  runs in its foreground process group, are all foreground), and with several targets only when Herdr
  reports the pane idle. Membership is exact pid equality: an ancestor or descendant of a foreground
  process is not a match. No match, several matches, no fingerprint or no foreground report means no composer wake, with
  no fallback to the latest claim or touch, even for a seat with one target (nothing is guessed, and the
  refusal is pre-send so the reminder ladder does not climb). One exception (ht-327): several matching targets
  with the same fingerprint pid and harness are bindings of one process (a plain-call-only group the
  same-process rule leaves connected, such as the in-process `/side` sessions of a Codex TUI), and a wake for
  seat-level attention (an invitation, receipt or warning, or a receipt's soft-deadline poke) types once into
  that process's composer: the item reaches whichever of its bindings takes the turn, so no binding is
  chosen. Like a single target it is not held to the several-targets idle condition (an ordinary wake
  reads no composer before that point); the adapter's final composer read immediately before submission
  still refuses typed input and a busy agent. For a receipt or invitation it is owed only while every binding of the process is
  unserved (one served in its turn served the process), and a binding whose mod channel woke it takes its process's other bindings off the composer
  targets. Seat-level attention includes a delivery row the seat's nudge threshold admitted (reason `lazy`,
  the lazy frontier admitted by `NudgeThreshold`): under message priority delivery (D9) a bare `--nudge`
  (p0, no `--require-ack`) makes no receipt, so this row is what wakes the seat, and it takes the one-process
  composer wake as `pending_receipt` does (orchestrator decision at the ht-327 merge; the item is shown
  to whichever binding of the process takes the turn, as before). Unlike a receipt, a delivery row is each
  binding's own stream, so its owedness is decided per binding (below). A wake whose reasons name no such item still
  needs a unique match, and a threshold-silent row (priority above T) never reserves a wake at all. Only a reservation that names no targets
  (a legacy source) keeps the single-harness rule. A soft poke follows the same targets and rule.
  *Served in-turn* (ht-705, A8): a target that was shown every pending receipt (or invitation) the wake is
  reserved for inside its own turn is owed no wake for it; the seat-level candidate drops those items only
  when every target was served, so one binding's turn never hides a sibling's owed wake. Immediately before
  the prompt, after the reservation fence, the daemon rechecks the binding the composer chose (or, with no
  composer choice, the seat-level binding a registered or recovery-hint wake reaches) against its own records; a served
  binding gets a pre-send refusal (nothing typed, the ladder does not climb) and an unserved sibling's wake
  stays owed for a later attempt. A target that already displayed the threshold-admitted frontier delivery row the
  wake is reserved for (reason `lazy`: its cursor passed the row or its displayed mark holds it, decided from that binding's own records, never the
  seat-wide delivery-row state) is owed no wake for
  it, but a one-process composer wake for that row stays owed while any binding of the process still finds it
  unread, and is refused (pre-send, nothing typed) only once every reached binding displayed it (main review I4,
  ht-yha.62; this reverses the ht-yha.45 wording, under which a `/side` sibling's display refused the idle main
  binding's wake on every retry). The accepted cost is at most one composer wake after a sibling's display: a
  submitted wake keeps the frontier reserved, so it is not retried until a newer row is admitted. A wake
  reserved for several reasons is owed while any of them is (receipts and invitations by the every-binding
  rule, the row by the any-binding rule).
  Verified 2026-10-11 (`evidence/ht-54r.89/summary.md`, derived fingerprints, no harness logged in): the
  fingerprint pid is a foreground process for Claude 2.1.296 (live: `CLAUDE_PID` among seven foreground
  processes), Codex 0.162.1 native and via npm (node launcher plus native child), Hermes v0.21.5 and
  agy 1.3.3. Unverified: any real binding row, Hermes through `herdr-threads launch`, and a harness
  running tools. A fingerprint outside the foreground group (an ancestor or descendant) is not
  matched and was not observed.
- *Poke*: a soft-deadline poke is the fixed reminder
  `herdr-threads: receipt due in <N>s on <thread-ids>; run herdr-threads inbox` (thread ids only),
  submitted through the wake dispatcher and its limits to the seat's bound native agent of the bound
  harness. It is decided from a fresh observation immediately before the prompt, only when the pane is not
  focused, the composer is empty, and the agent is idle/done. Working agents are
  always deferred; there is no turn-time poke capability (ht-poh.42). Existing exact-version recipe declarations remain
  historical captured qualification, not operational admission. Ordinary daemon observation resolves
  executables without probing runtime metadata; absent current-runtime qualification declares neither
  capability. Valid contract input, historical version evidence and manifest/release ladders cannot
  qualify the current runtime or grant compact recovery, composer stash or native
  receipts. Until a separately safe current-runtime qualifier exists, richer behavior remains unavailable.
  It is never sent in an approval or
  question state or an unknown state. A poke carries no authority and changes no receipt state other than
  `soft_poked_at`, which is set only when the host accepts the prompt; the hard-deadline warning stays the
  backstop. When an ordinary wake is due for the same seat, one prompt goes out with the poke text, and only
  when the poke itself is eligible and needs no stash.
- *Mod channel*: while a mod channel is live for a seat (from registration until its stream ends, through a
  30 s reconnect grace after a drop, and through a 30 s seat-level rebind grace after `Close{binding_changed}`),
  and only when that adapter's delivery policy says the harness is woken by the channel (`idle_wake: Channel`),
  no native wake prompt or poke goes to a binding it serves (the question is per binding: a seat's other
  bindings without a channel are still woken natively) and that harness's check-in results omit the attention digest
  and ready commands; pending attention stays pending and deadlines and hard-deadline warnings keep running,
  reaching the agent through the mod. A grace that expires, or a `Close` for `retired`, `unresolved`,
  `stalled`, `disabled` or `stopping`, removes the channel and kicks the wake lane. *Stall handover*: a
  channel is stalled when its last mod ACK (or its registration) is more than 10 minutes old and an ordinary,
  non-truncated pending receipt for the seat, published at or before the last pushed attention frame, is
  itself more than 10 minutes old; the daemon closes it (`stalled`) and refuses re-registration of that
  binding generation for 10 minutes, so the native ladder with its composer guards is the only delivery path
  during the cooldown (contract; implemented by ht-j16.2 and ht-j16.4).
  - *Idle is read, not assumed.* Herdr's `agent_status` does not show typed input, so the adapter also reads
    the composer (`agent read --source detection`). The state is idle only when `agent_status` is `idle` or
    `done` and the composer is empty; composer text is typed input (human input); a `working` status with an
    empty composer is an active turn, and with composer text it is unknown (a poke would merge into the
    draft); `blocked` is an approval or question. Claude's placeholder counts as empty only when it is exactly
    a captured placeholder (`docs/evidence/poke-spike/findings.md`); any other placeholder-shaped row is
    unknown and skipped. Claude Code draws a prompt suggestion in the composer after a turn, and the
    detection text cannot tell it from typed input; until its styling is captured, any Claude composer text
    other than a captured empty marker is not known empty and is never stashed. An unreadable composer, a failed read or any other status is
    unknown and skipped. The same reader classifies observations and final native delivery admission.
    Ordinary native wakes also require an empty composer, checked after their final
    identity/status recheck. A missing registered composer provider, nonempty composer or unreadable composer refuses delivery before input is sent;
    pending attention and receipt obligations remain intact and the prior retry ladder is restored.
    Unfocused empty panes may wake immediately. Focused panes require a minute of observed empty
    composer samples, measured with the daemon's monotonic clock. Qualification is process-local,
    bound to seat/target/terminal/harness/server identity, bounded to 1024 windows, and restarts after
    a failed final admission, a successful send, clock reversal, or a sample gap over one minute.
    Intervening composer reads that fail or observe nonempty/unreadable input reset qualification.
    Soft-deadline-only pokes retain their stricter unconditional focused-pane skip; the one-minute
    focused rule applies to ordinary wakes. Status/focus checks and screen reads are an approximation,
    not keystroke telemetry.
  - *Drafts are deferred.* Attention never stashes, clears, retypes or submits a known draft, including
    soft-deadline pokes even if an old recipe declares composer stash. Existing adapter stash helpers
    are retained but the notification dispatcher never invokes them. A parked draft or unreadable
    screen can defer attention indefinitely. Claude suggestions may also defer delivery unless
    disabled through the existing user-approved setup option; this change never edits configuration.
  - *Verification is read-only.* A held marker yields an unsubmitted diagnostic/uncertain delivery;
    verification never sends an additional Enter, which could submit newly typed user text.
  - *During a turn.* Unsolicited attention never queues into a reported active turn. The dispatcher
    refuses ActiveTurn and both native prompt entry points require idle/done. Pending obligations wait for a later eligible observation.
    Composer emptiness never establishes idle status. These host status observations are sampled
    screen/lifecycle inference, not independent proof of physical turn completion.
  - *Post-send verification.* The read-only check never changes the composer or sends keys.
  - *Declarations.* Claude 2.1.287 declares both (spike evidence). No Codex recipe declares either: the
    spike tested Codex 0.160.0 (mock provider), which no recipe covers, so Codex seats are poked only when
    idle with an empty composer. Claude's `composer_stash` declaration currently never stashes, because no
    Claude composer text is known to be a typed draft.
  - *Retry spacing.* A seat whose poke was skipped or failed is re-evaluated no sooner than the wake retry
    spacing afterwards, so an ineligible seat costs one observation per spacing rather than one per tick.
    The spacing is in memory; a restart re-evaluates once.
- *Hook-pull delivery*: a harness whose adapter declares `HookPull` has no channel and no wake change: the
  existing native wake applies, and its turn's `PreInvocation` pull delivers. Tool-boundary check-ins of such a
  seat omit the attention digest and ready commands; its lifecycle check-in keeps them. Pending attention stays
  pending, and deadlines and hard-deadline warnings keep running.

**A5. Who may do what.**

Delivery and authority gates read the binding's role, liveness and harness, never its observation provenance.

| Action | Who |
|---|---|
| ACK, accept, reject an ordinary invitation, send, leave, archive, reopen | a connected top-level binding of the seat (top-level agent or declared human); never a subagent binding |
| advance one's own binding's read cursors (`AdvanceCursors`, a display of its own stream) | any binding of the seat, subagent bindings included; it moves only the caller's own binding cursors and settles nothing |
| display ACK after text inbox output | any connected top-level agent binding of the seat, for exact canonical pending agent receipts fully displayed on the page; the daemon decides eligibility again before settlement |
| open a mod delivery channel (`watch`) | the pane's top-level session of a push-channel harness through its mod, for its own connected top-level binding (the claim's native session); several bindings of one seat each hold their own; recorded as `cooperative_mod_channel`; it grants no other row |
| mod delivery ACK (`watch ack`) | the connected top-level binding of a push-channel harness that holds a live channel (grace included), the one the claim's key names; the daemon decides each id again before settlement and refuses unknown, unaddressed and truncated ids |
| hook-pull delivery (`DeliveryPull`, `DeliveryConfirm`) | the connected top-level binding of a hook-pull harness that the claim's key (seat, harness, native session) names, from any of its top-level hooks (including `Stop`); settlement only for ids the daemon offered to that binding; the daemon decides each id against A2 again; recorded as `cooperative_hook_delivery` |
| set the nudge threshold (`SetNudgeThreshold`; legacy `wake-on [nudge\|all]` = threshold 0 \| -1) | a connected top-level binding of the seat (cooperative; changes wake eligibility only, never receipts or authority) |
| check in | the pane's top-level agent (hook) or a human via `ht human me init` |
| record a launch binding (`managed_launch`) | `launch`, after a host-correlated startup with one observable started process; it grants no row above |
| rebind, fresh seat, retire, replace, orphan-thread invite | operator |
| service-authored send, notify, and managed-thread controls (ensure, invite, topic, release, archive, reopen) | the registered service connection |
| summary, summary job, summary submit | the seat's binding or its children (summary workers), all under the seat's claim: read-mostly; submit only stores a validated block for a live lease issued to the seat |
| automatic quiet-channel archive | daemon lifecycle policy, after fresh complete eligibility and protected-work validation; never service-owned channels |
| anything else on behalf of another seat | nobody by design; possible by spoofing (Accepted limits) |

Automatic archival changes only the channel archive flag and appends a seatless lifecycle event. It
never retires or moves a seat, leaves a membership, settles a receipt or invitation, or releases a
requirement. Explicit reopen starts fresh grace. A thread archives after one quiet hour. Only an
active member blocks: a seat with a connected binding (still needing the idle sample, so
unregistered, held and uncertain occupants block) or a human seat while its pane exists. A member
whose seat has no connected binding, an unresolved or retired seat, and the pending invitations to
and obligations owed by such a seat do not block. A binding migrated from a pre-session-keyed
database counts as active for 24 hours after the upgrade, so an upgrade causes no archival wave. The
archival scan restarts only on membership, member-liveness (binding connect, disconnect, end) and
seat-lifecycle changes, not on fingerprint refreshes, cursor advances or other seat writes. Pending compound handoffs remain protected indefinitely;
completed compound identities are absorbing tombstones. Exact historical completed retry may present
and clean up its retained local intent, even after archival or binding change, but cannot resume any
create/invite/send/launch side effect or revive protection. A legacy hint is reconciled only by exact
immutable compound identity; absence or removal of a local file never completes a canonical fence.

The operator never ACKs, accepts, rejects invitations, sends or advances a checkpoint.
The service never ACKs, accepts or rejects invitations and is never a receipt recipient.

A service-required attachment may reuse a pending ordinary invitation only if no
requirement episode has ever referenced it. Once referenced, the invitation remains
linked to that one immutable requirement, including after release. Reattachment creates
a fresh requirement-owned invitation with its own timing; it preserves any independent
ordinary invitation, its timing and attribution, and the released requirement history.
Releasing the fresh pending requirement cancels only its requirement-owned invitation.
Every new requirement still needs exact-revision acceptance by the addressed seat;
reattachment never accepts membership or settles receipts. Exact operation replay returns
its historical result without reactivating a released requirement. Native ordinary
acceptance selects the newest pending, noncancelled, nonrejected invitation before any
accepted historical invitation, consistently at permit preparation and acceptance.
This lets the seat explicitly settle a preserved ordinary invitation after accepting
a newer requirement without creating a second join interval.

An ordinary invitation rejection is the addressed seat's explicit, accountable decision about one
invitation episode, with a nonblank reason of at most 4096 UTF-8 bytes. It retains the actor seat, binding
generation, timestamp and existing `cooperative_top_level` or `operator_human` observation in an immutable
ledger. New rejections publish an attributed native warning event in the thread; historical info
events remain unchanged on replay (A7). Rejection settles only that invitation's attention and warning
condition: never a message ACK or waiver, an acceptance, or a fabricated join/leave interval.
Declared subagents cannot reject. Required invitations remain service-owner controlled; a recipient must
ask that owner to release the requirement and reread its current state before attempting rejection.
Reinvitation creates a fresh episode; replay of an old rejection cannot reject that new invitation.

Every newly recorded invitation (agent, human or operator; ht-yha.6, D6) is confirming: it stores
confirmation `require_ack` with the explicit or instance-default deadline, is a wake source and has an
inviter-overdue warning (an operator invitation has no inviter seat, so it stores the absent policy, which
is the effective RequireAck; unchanged). A request's `confirmation` (Lazy, RequireAck or absent, from any client version) is
accepted and ignored for the recorded policy; the earlier refusal of an explicit human RequireAck is gone.
The Lazy and absent-policy rules in the rest of this section describe stored pre-upgrade rows only, which are
never rewritten.

Ordinary agent invitation confirmation is an immutable episode policy. Stored pre-upgrade Lazy invitations remain
pending and discoverable at ordinary inbox/hook opportunities, with no acceptance timer or native/mod
wake. Their inherited physical duration/deadline columns retain legacy storage timing only; no canonical
deadline, event, diagnostic or wake projection interprets that timing as a confirmation obligation.
Explicit RequireAck, and historical invitations whose policy is absent, retain a frozen deadline from
canonical creation. Registration and launch never accept an invitation or reset its deadline. Only the
addressed seat's explicit accountable acceptance settles participation; rejection, requirement release
and retirement retain their existing distinct terminal outcomes. Duplicate pending invitations preserve
the first policy, timing and inviter; an already joined target creates no new invitation. Required service
memberships retain their timed participation contract: imposing a requirement on a pending Lazy
voluntary invitation creates a fresh timed episode and preserves the old invitation, origin and keyed
outcome. Releasing that requirement leaves the original Lazy invitation discoverable without a timer.
Explicit human/operator invitation semantics remain separate and unchanged.

**Automatic pair reservation.** `EnsureDirectThread` resolves one immutable canonical channel for
exactly the caller and recipient seats, under the caller's ordinary `cooperative_top_level` or
`operator_human` claim and the A2 deciding transaction. Initial creation joins both reserved seats in that
one transaction and records no invitation (the user's explicit decision D7, advertised as
`direct.auto_join_v1`): a pair is a reserved two-seat channel, so the consent an invitation would collect is
given by pair creation itself. The peer's membership interval starts at its own join event, and a creation
policy is validated for shape and otherwise ignored. A pair seat with no cursor for the pair starts at the
thread head as of that join (the auto-joined pair floor), not at its first session's head, so its first
session is offered the opener and every later send; any seat that already holds a cursor keeps it. Joining
is never ACK or check-in. A pair created before the upgrade with a pending invitation keeps it: it reports
`ExistingPending` and still needs `accept`.
All ordinary, operator and service enrollment paths preserve the two-seat participant universe;
services cannot adopt or own these channels. Invited seats remain reserved even before acceptance.
Explicit leave, rejection and archival require ordinary join/accept/reinvite/reopen repair, never
automatic resurrection or replacement. Exact operation replay returns historical selection before
fresh A2/lifecycle checks; it grants no permission for a new send. Direct markers freeze the exact
other pair seat independently of ACK requests and are validated at preparation and publication,
including corrupt participant/audience refusal. Lazy messages retain A8. An absent Ensure creation policy preserves the historical
initial timed invitation and exact `ThreadCreated` result bytes. Explicit creation policy
uses a bounded `DirectThreadEnsured` result: canonical thread/recipient, the selection
outcome (created, existing pending or already joined), and the actual pending
invitation's ID, episode, confirmation, effective deadline and inviter. This is the
canonical selection at the original keyed decision; replay never refreshes the
policy or deadline, and local copies remain hints. Historical NULL confirmation is
presented as effective RequireAck while preserving its original deadline and NULL
inviter. Existing episodes and historical absent-policy creation remain unchanged.
The additive `direct_threads.ensure_creation_policy_v1`
capability admits an explicit frozen initial Lazy or RequireAck policy and optional
positive duration. Only creation of a new pair applies it; existing pending or joined
participation keeps its original policy, inviter and deadline, and left/rejected
participation still requires explicit repair. Modern direct sends and automatic
handoffs send the policy with the pair ensure (an auto-joining daemon ignores it). A requested body ACK has its own send-time deadline and
cannot accept an invitation or settle participation. Pair identity remains with its channel history;
there is no seat merge, remapping or generation rollover and no new provenance value.

**Voluntary self-join.** An explicit `join THREAD` enrolls the caller's seat in an active
thread of its instance under the ordinary A2 deciding transaction and its existing
`cooperative_top_level` or `operator_human` claim. The immutable native-authored join
info event retains actor seat, binding generation, observation provenance and decision
time; the membership interval starts at that event's canonical decision sequence.
It creates no invitation, invitation acceptance, receipt ACK or binding. A pending
ordinary invitation requires explicit acceptance or rejection first; a pending required
invitation requires exact-revision `accept-required`. Join cannot accept, release or alter
a service requirement. An already joined seat is a settled no-op. Leaving and joining
again creates a fresh interval without changing historical recipient snapshots or
obligations. Replay returns its original result even after leaving, archival or a binding
change; it never recreates membership. Archived threads require a member or service
owner to reopen them before a new join. Discovery and names confer no additional authority.

**A5a. Human seats do not owe ACKs.** The daemon classifies a recipient from its canonical open
binding when staging a send. A human binding gets the message through a priority-0 delivery row (A8) but no
receipt expectation, even when the sender requested an ACK; an unbound or agent bound explicit ACK
recipient keeps the existing expectation. A human lifecycle check-in waives every pending obligation already
owed by that seat, including obligations inherited from an earlier agent binding. It records a
durable decision cutoff and a bounded reconciliation job in the same decision. The job records
each receipt table's high water independently and sets the separate `ack_required=0` obligation
marker on retained pending rows; later agent mail stays outside those bounds. The receipt stays
physically pending as historical evidence, but its effective state is `not_required`; it leaves
pending, wake and overdue scans. No ACK, ACK actor or ACK provenance is fabricated. A later agent
binding cannot revive the waived obligation; messages addressed to that agent afterward can require
ACKs. Already acknowledged receipts retain their exact historical status and provenance. A person
may still explicitly ACK an older addressed receipt; that action is attributed `operator_human`,
but the system never demands it. Invitation acceptance, reads and message history are separate
from receipt expectations.

For an existing store, migration derives the last human check-in cutoff from recorded human
binding generations and seat availability decision sequences. An open human binding can waive
through the current decision sequence; a closed one stops at its own check-in. A pending send
published within a recorded human binding's time interval is also waived, including a send made
while that binding was temporarily unregistered and its availability provenance is absent. The
end of a closed binding is exclusive: mail published after it ends, including mail for a newly
managed launched agent before that agent's first availability decision, remains owed. Legacy
manifest rows with recorded `operator_human` recipient availability are waived as well. The
one time reconciliation marks waived physical and sparse rows and removes pending projections;
old warning events, receipt timestamps, and ACK provenance remain historical evidence. A send
at the exact recorded end time is assigned to the successor for this migration, because the
millisecond clock cannot order simultaneous decisions within that boundary.

Recovery and human waiver may close many warning conditions. Their canonical decision records
one durable close sweep with a frozen condition boundary, recipient decision cutoff, and membership
high water; budgeted workers publish one clear transition per condition. A covered condition is
logically closed for active-warning queries before its clear event is published. The captured
snapshot, rather than worker time, controls clear recipients. A new open in the same thread waits
for its prior marked clear to publish, so a blocked send can return `StoreBusy` and require a fresh
operation after bounded preparation cleanup. A sweep cannot clear conditions opened after its
boundary or claim that an unprocessed clear event was delivered.

**A6. Effective receipt deadlines.** New durable ACK-required sends freeze their duration
and original deadline at canonical committed send time plus that duration. This applies to
unbound, unregistered, unavailable and invited recipients; later registration, acceptance,
restart or rebind cannot restart the clock. Availability remains observed availability and
confers no new wake or receipt authority. Delivery rows (every priority, A8) and canonical human waivers remain
free of ACK obligations; only explicit ACK recipients owe one. The durable per-instance send-time cutover preserves every previously
running sparse deadline using the earliest canonical availability recorded before the cutover
and before retirement; stored running deadlines and terminal receipt provenance are preserved.
Previously dormant obligations derive their deadline from immutable send time and duration.

Modern clients require `messages.send_time_receipts_v1` before a new ACK-required send;
absent or refused discovery fails explicitly without a legacy fallback. Sends without explicit ACK recipients are exempt.
Accepted mixed-version limit: ordinary CLI and service clients cannot prove that the canonical
recipient audience is entirely human-waived, so an ordinary send that can create an agent
obligation conservatively requires the capability, even when its eventual audience is all human.
The sender's human harness never implies a recipient waiver. Completed retained compound
results and service journal records remain inspectable unchanged without new send effects.
Public ordinary CLI operation-status reads require a verified scope unavailable on that wire path,
so the client cannot recover an exact pending key's committed outcome through that command.
Ordinary commit-unknown CLI retries and service pending replays without locally retained completion
conservatively require the capability even if their original send may already have committed. Immutable request bytes, keys, digests and outcomes are never rewritten.

A receipt's effective deadline is the later of its frozen deadline and
the `extension_until` of the latest catch-up row for the receipt's (seat, thread). The daemon decides it (A2)
from catch-up state the seat itself entered through its accountable claim (a `summary` call that returned
work). The first entry, and any entry after a ready or superseded row, grants one p99 job duration;
re-entry after a stall grants nothing until a block is stored (the most recent row ended stalled and no
block for the thread was stored since), and a repeated call on an active row grants nothing. Otherwise it
is extended only by stored summary progress (each newly stored block, and the exit grace after a Ready),
never on heuristic evidence, so a stall and re-entry cycle cannot postpone a warning. Frozen deadlines are
never rewritten: they stay stored and displayed beside the effective one. Overdue classification, warnings, pending receipts and the soft-deadline poke use the
effective deadline.

**A7. Informational warning delivery.** Service notices, canonical built-in warning open/clear
events, and native ordinary-invitation rejection notices have separate delivery rows for their frozen
recipients. A rejection notice retains its native seat author, exact invitation source and reason;
its audience is the members at the rejection decision plus the rejecting seat, never later members.
Only the exact native warning backed by the retained rejection record is classified this way.
Historical info rejection events remain unchanged on replay. Rejection notices are passive for every
recipient: they create no wake, poke, receipt or ACK obligation. A committed check-in, or a
hook-pull harness's `DeliveryPull` page or a capable Watch presentation page, settles only the
bounded prefix of attributed events it carries to the exact checking-in or pulling binding:
the notice frontier and warning offer frontier are kept per binding, so another session's
check-in or lifecycle start neither settles nor resets them, and a binding's digest token names
that binding. The pull settles before it replies, for the connected top-level binding its claim
names; nothing is transcript-confirmed and no provenance is recorded. The pull's settle refuses
unless the carried notices are still exactly that binding's unoffered prefix. Check-in, Hook
and capable Watch share that binding's exact notice frontier: fresh allocation carries each
notice once per binding, while bounded recovery may restream the same frozen outbox token.
The pull's version includes the newest notice pending for that binding, so an unchanged reply
never hides one; notices never wake. A subagent binding never checks in and settles nothing:
its hook reads its own binding's digest (unread mail past its own cursor, notices above its
parent's frontier) and presents it with read-only wording, compared only with its own client-local
mark (a hint keyed by the binding key, never authority). A caller-bound inbox read decides its
warning cutoff and informational-notice view against the reading binding's own frontiers
(a subagent's against its parent's until it has its own), never a sibling's. Seat-level readers
(wake, receipts, an unbound inbox) follow the seat's newest open top-level binding, a connected
one first. The mod channel is held per connected top-level binding of a push-channel harness,
not per seat. A global decision watermark cannot settle uncarried events, including late attribution.
History and immutable operation results remain readable. A successor occupant gets its own offers;
response loss can lose an informational offer after commitment, and so can a hook-pull injection
that the harness accepts but the model never sees (an agy `systemMessage` that silently misses;
notices are not transcript-confirmed, ht-7jh); an explicit operation retry
can present the retained result again. Delivery is neither invitation acceptance nor receipt ACK,
agreement, adoption or task completion. Logical publication tokens stay unchanged by projection;
an advertised read-only delivery hint lets hooks drain newly attributed and remaining notice pages.
Delivery is not a wake. A built-in transition wakes the affected seat of a still-open
condition (the hard-deadline backstop for the obligation that seat owes). An invitation deadline-missed
open transition also wakes its originally recorded native inviter under the same host, composer and mod
admission guards, even after that inviter leaves the thread or its informational notice was already
carried by a committed offer. A separate bounded open-condition projection preserves this wake until
canonical closure; the informational offer remains settled and creates no repeated delivery obligation.
The open episode freezes its instance and original inviter address; historical
rows use only their exact canonical invitation-creation event's recorded inviter, never a current member
or later duplicate caller. Unknown historical/service/operator origins do not manufacture a native seat.
A still-open receipt deadline condition also wakes every canonical native seat author of its exactly
associated physical or published-manifest receipts, including authors who left the thread. The shared
thread/recipient condition retains one open event: later associations add derived origin addresses,
not new warning events or receipt provenance. Informational offers cannot settle this independent
open-origin attention. The indexed origin projection persists for all associated authors while any
required, pending, unwaived receipt remains; settling a subset cannot withdraw another author's
warning. Final settlement clears it once, passively. Canonical closure alone removes these waking
addresses. Unknown and programmatic authors create no native seat address. Historical projection
bootstrap and deadline repair run in bounded transactional turns; durable warning discovery remains
independent of native wake admission, so composer drafts, active turns and host outages may defer
key submission indefinitely.
A clear remains passive. Apart from the canonical origin wakes above, transitions about another
seat remain passive for other observers and wait for the recipient's next check-in offer or
hook-pull page. Accepted limit: an idle member who is only a passive observer learns of another
seat's overdue or cleared obligation only when its pane next checks in or, on a hook-pull harness,
at its next turn-start pull. When
more than the bounded window of pending warnings or unoffered notices is visible, narrowing cannot rule
out an older waking notice (such as a service notice), so the wake keeps its unnarrowed answer and the
offer probe stays conservative: it may still wake, never silently drop one.
The same rule decides a live Claude mod channel's attention: the daemon counts only waking warnings in
the channel's attention fingerprint (with the same saturated fallback) and marks each other pending
warning `informational` in the inbox batch, so `watch` raises no `attention` line for it.

**A8. Passive delivery-row bookkeeping.** Every message carries an immutable delivery priority 0..=9
(default 1), frozen at send and independent of sender role, relay and human intent. Delivery rows
(`lazy_recipients`, the former "lazy" bookkeeping, now for every priority) are the bookkeeping of the whole
audience: each joined non-author audience member that is not an explicit ACK recipient gets one row carrying the
message priority, human seats included. Only an explicit non-human ACK recipient (`--require-ack SEAT`, or a service send's explicit `recipients`) gets a
receipt instead (D9); a human explicit recipient gets a priority-0 row, never a receipt, and a priority above 0
cannot be combined with explicit ACK recipients (the daemon refuses it, as it refuses a deadline without explicit
ACK recipients). Receipts, deadlines, overdue warnings and ACK obligations therefore exist only for explicit ACK
recipients; the old rule that every joined agent owes an ACK for an ordinary message ended. A priority-0 row-only
send keeps the send-time unavailable-recipient warning (sender feedback, not an obligation); a priority above 0
produces none. The recorded `delivery_mode` column is audit only, written `ordinary` iff the send has explicit
ACK recipients and `lazy` otherwise, and is never read for decisions; an old client's modes map `ordinary` to
priority 0 and `lazy` to priority 1 (a request's own priority wins; its payload digest covers the priority only
when carried, so journaled old-shape intents retry unchanged). A delivery audience is frozen by canonical
preparation and publication; recipient identity is immutable, and its pending/displayed progress is monotonic.
Each open binding (session) keeps its own pending view of delivery rows (user decision 2026-10-14): a row is
pending for a binding until that binding displays it, by any display path (a completion, or a cursor-only
display: a full `body` read, an `accept` display, a hook page whose completion then failed), its cursor
passes it, or the binding ends. A binding's own reads (inbox, hook, accept, presentation) seek only its own
view, so a sibling's display never changes what a binding is offered and a sibling's undisplayed backlog
never costs it anything. A view is held only by a connected binding (any role; a subagent reads its own stream). Every disconnect (a
process exit, superseded, an end event such as a session end or a subagent stop, seat retirement or unresolve,
replaced, migration) drops the view in the disconnecting transaction, and publication writes it no rows while it
stays disconnected, so a disconnected binding costs only its cursors and display marks; when it reconnects (a
plain call after a process exit, or a lifecycle check-in such as a resume, a re-adoption or a reopening) its
view is re-seeded, at a cost proportional to its own backlog, from its own cursors and display marks, so it is offered
exactly the rows it has not displayed, whatever a sibling displayed meanwhile. No seat-level view reads a
disconnected binding's view: the wake frontier and discovery, the seat inbox filter, the digest's pending
count and the mod fingerprint, kick count and stall rule consider connected top-level bindings only (plus
legacy seat-pending rows). The seat-wide `displayed` mark (set once no top-level binding that still owes the
row, connected or disconnected by process exit, finds it unread) is kept only as the join frontier's "no
completion has displayed" fact below and the binding-less completion state; no delivery or wake decision reads
it (the served-in-turn recheck asks the woken binding's own cursor and marks). Accepted limit: a disconnected binding
whose process still calls without reconnecting (a superseded session's stale process, say; plain calls never
reconnect it) sees no delivery rows in its own reads until a lifecycle check-in reconnects it.
Already addressed rows survive leaving, retirement and archival, with no transfer to another seat.
Undisplayed rows (`state` still `pending`) past the seat's furthest cursor when it joins become seat-pending
(join frontier, below).
Unpublished rows are invisible and may be discarded in bounded cleanup; published progress is retained.
Delivery rows create no receipt, deadline, ACK evidence, wake attention, wake, poke or automatic adoption,
except that a row whose priority the seat's current nudge threshold admits (priority <= T; every priority when
T < 0; the legacy `wake_on: all` is T = -1, `nudge` is T = 0) wakes an idle seat (wake eligibility only; still no
receipt, digest attention or poke). The threshold is read at every wake decision, never frozen at send.

Only the caller's default text inbox, a standard native hook, a push-channel mod or a hook-pull harness's hook, as below, may claim completion after complete contiguous body output has
been written and flushed. The completion handler validates the current top-level or human canonical
caller (A2) and exact published addressed message IDs in its deciding transaction; declared subagents
cannot complete delivery. JSON, machine, explicit-seat reads, history, bodies and summaries remain
read-only for Lazy completion. A complete successful own text body display may separately advance only
the calling binding's read cursor, as specified below; it is not Lazy completion or a receipt ACK.
A binding's display of a delivery row that stays above its cursor (delivery order is (priority, decision), the
cursor walks message sequence, so a higher-priority row can be shown before a lower-sequence one) is remembered per
binding (`binding_row_displays`, written by the one cursor-advance function for every display path, top-level
completion and `AdvanceCursors` alike): every binding-level unread and pending predicate excludes a marked row, the
display walk passes it, the mark is dropped once the cursor passes it, and a successor binding inherits the marks
of the cursor it inherits. Marks are the binding's own display bookkeeping, never evidence about another binding,
and confer no receipt, ACK or wake authority; a bookkeeping failure still leaves the row offered again (at least once).
A top-level agent's text `accept` (and `accept-required`) output is that same default text inbox
display, thread-scoped (ht-br0): after the accept result was written and flushed (with a live mod channel,
only after the channel grace below has run out), it reads one `InboxBatchV2` page of the caller's own stream restricted to the
accepted thread's unread delivery rows of every priority (`InboxQuery.thread`, `inbox.thread_scope_v1`; no ordinary, receipt,
invitation or warning row). The thread scope only narrows the unchanged lazy eligibility (the binding's own
cursor/scope branch and the top-level legacy-pending branch); it never admits a row the unscoped text inbox
would not show that binding. It narrows it further to rows below the binding's first ordinary walk stop in
that thread: the first receivable ordinary message past the binding's cursor that the binding has not
display-ACKed (`read_cursors::first_ordinary_walk_stop`, where the display walk below stops when a display
shows no ordinary row). So every row the page shows whole is passed by the binding's own walk, and no later
hook, `inbox` or channel offers it to that binding again, when the page holds the lowest-sequence unread lazy
rows below the stop. The page is ordered by lazy ordinal (assigned at preparation) and the walk by message
sequence (assigned at publication); with concurrent senders the two can disagree, and a page cut by its bounds
may then leave a shown row above the binding's cursor, offered again (at least once, as the plain text inbox); lazy rows above the stop stay in the binding's own
stream and are shown once, with the ordinary rows, by its next hook or `inbox`. A claim that names no binding
of the seat gets no scoped row. It writes and flushes the page, then advances the binding's cursors and
completes the lazy rows it showed whole exactly as the text inbox does, under the same claim (via none; no new
provenance). The accept is already committed and its result and exit status do not depend on this display; a
failure leaves the rows to be offered again by the next hook, wake or `inbox` (one diagnostics line when the
page was already shown; a bookkeeping step that did succeed is kept, so the line asserts no pending state). While a mod channel is live for the caller's binding, the accept first gives the channel a
bounded grace (2 s from before its first peek, 100 ms peeks; every peek's call budget is capped at the
grace's end and at the caller's own budget, and no peek starts after it, so slow reads cannot extend the
wait): it re-reads the scoped page read-only (no display, no bookkeeping) until
every row of its first read has been completed by the channel (`watch ack --via append`, which runs the same
walk), and then shows nothing; a peek refused inside the grace also shows nothing (the rows stay with the
channel). When the grace runs out, including a peek cut off by its end, it shows and completes the remaining rows itself as
above; a channel append that lands later may repeat them in the transcript (at least once; the channel's ACK
then finds them `already_settled`). A JSON, `--machine` or `human` accept and a subagent show no rows and
complete nothing; an `accept` retry presents only its retained result. Local display journals are cooperative hints, never authority. Completion is idempotent
presentation bookkeeping: it records no ACK actor, receipt provenance, timeline ACK, agreement,
instruction adoption, task completion or proof of model consumption. Partial output and changed
bindings cannot manufacture complete display. Frozen completion retries preserve their original full
caller claim, harness and intent scope through submission and local cleanup. These same-user cooperative
limits are deliberate; no adversarial execution verification is added.

A push-channel mod (today the bundled Claude mod) is a second completion source: it completes lazy rows it appended to the transcript
(`watch ack --via append`), decided by the daemon against A2 under the same checks as the mod delivery ACK
(A5) under the `cooperative_mod_delivery` claim "appended to the transcript" (no per-row source is stored). Like inbox
completion it is presentation bookkeeping (no ACK actor, adoption or proof of consumption).

A seat whose nudge threshold (A5) admits a row's priority has that row promoted for push-channel delivery: the daemon marks such a
row `wakes`, computed against the seat's current threshold, in the inbox batch, and the mod delivers it as a `message` line with no ACK to run, marked `wakes`
on the watch line so the mod still wakes an idle session for it (a `message` line with no ACK and no `wakes` is
mail another binding already settled: appended, never an idle wake). The daemon still
creates no receipt, deadline or wake for it. The mod may complete a promoted row delivered via `context` or
`submit` (`watch ack --via context|submit`); the daemon completes it as a lazy row (a lazy-mode id completes
regardless of `via`), decided against A2 under the same checks as above. The mod's completion is the
cooperative claim that the row entered the model's context through the mod (`cooperative_mod_delivery`), carried
by the request; the daemon stores only the row's `displayed` progress and no per-row source. Like every lazy completion it is presentation bookkeeping: no ACK actor,
adoption or proof of consumption.

A standard native hook (top-level SessionStart, Bash PreToolUse, Codex UserPromptSubmit or qualified turn;
never a subagent event) is a third completion source. Only a digest the hook explicitly requests (`AttentionDigestQuery.lazy`)
reports pending lazy rows and a lazy token key, so a lazy arrival moves the hook's change marker; the wake
frontier, wake candidates, prompts, pokes, deadlines and warnings never read delivery rows, except that a seat
is woken for its own rows its nudge threshold admits (wake eligibility only, above). When the hook
presents (marker advanced, or a lifecycle check-in) and no mod channel is live, it reads one bounded
read-only v2 inbox page and places only complete items in the bytes its ordinary context leaves under the
context bound. After its stdout was written and flushed, and the adapter reported that the output carries
context, it submits exactly the lazy IDs shown whole with `CompleteInboxDelivery` under the registered
execution's canonical claim and the `cooperative_hook_context` claim (`via: hook_context`). The daemon
decides it under the same checks as inbox completion (A2). The claim is that the hook emitted the body as
context, not that the harness ingested it or the model read it. The hook never ACKs: ACK-required
messages it shows stay pending, with an exact `ack` command for the agent. A hook that ran inside the
occupant's turn (a tool boundary, or a turn start that checks in the same way; never a lifecycle check-in,
after which the session may sit idle) marks its cursor advance `in_turn` (ht-705, only to a daemon
advertising `inbox.in_turn_offer_v1`) and names the invitations it showed: the daemon then records in
`in_turn_offer`, for the displaying top-level binding, each displayed message whose seat receipt is still
pending and each named invitation still pending for the seat. That record is wake eligibility only, and
it is per binding (delivery contract rule 4: served in-turn, so no idle wake is owed to that binding):
the wake candidate skips the seat's pending receipts (and, separately, its pending invitations) only when
every one of them is recorded for every binding a wake could reach (each connected top-level target;
with none, the seat-level binding), and the dispatch recheck refuses the prompt to the one binding it
reaches when that binding is served (A4 *Wake*, *Served in-turn*). It settles nothing: no ACK actor or
provenance, no acceptance; the receipt or invitation stays pending, listed by `inbox` and subject to its
deadline. A newer unshown item, an unserved sibling or new occupant, or a lifecycle check-in of the binding
(which clears that binding's records only) makes the wake owed again. Warnings need no record: the check-in that carries them already advances the binding's warning
offer frontier. It writes no
local journal record. A failed or unknown completion leaves the rows pending and they are shown again (at least once).
A hook-pull harness's standard hook never reads an inbox page or makes this claim: its lazy rows complete only
through `DeliveryConfirm` (`cooperative_hook_delivery`) or the caller's own text inbox; the daemon refuses
`via: hook_context` from a hook-pull binding. So on a hook-pull harness only `cooperative_hook_delivery`
applies, never `cooperative_hook_context`.

A hook-pull harness's hook is a fourth completion source: it completes lazy rows whose frame header it found in the
session's transcript (`DeliveryConfirm`), decided by the daemon against A2, restricted to ids offered to the binding
generation, under the `cooperative_hook_delivery` claim (no per-row source is stored). Same presentation-bookkeeping limits. It is the only hook path that completes a hook-pull seat's lazy rows.

**Per-binding read position (`AdvanceCursors`, `InboxQuery.claim`).** An inbox read that carries the caller's
claim is that binding's own stream: the messages of the seat's joined threads past the binding's cursors,
the narrowly admitted current direct-invitation Lazy stream below, plus the seat's open invitations,
its still-pending ACK obligations and, for a top-level binding, the seat-wide pending pre-join lazy rows
(join frontier, below). A display moves only the displaying
binding's cursors, with the exact message IDs it showed whole. The command is decided in the daemon against
the canonical view (A2) and resolves the claim to its own binding like any plain cooperative call (it may
enroll). It is the one write a declared subagent may send, because it changes no seat state: it records no ACK
actor, creates no receipt, settles no obligation and attributes no action. Its `in_turn` form (a top-level
standard hook's in-turn display, above) adds only the binding's `in_turn_offer` rows, which affect wake
eligibility and nothing else; a subagent's or a non-in-turn advance records none, and only the `in_turn`
form may name invitations. A cursor never moves backwards,
never moves past an unshown message the seat can receive except at a join and a resumed fast-forward (both
below), and a read never enrolls. A client that fails to
send it only causes redisplay (at least once). Only a top-level binding's display ACK settles the seat's
obligation; other bindings see that message in their own stream marked settled. A full text `body <id>` read
is such a display (ht-54r F1; a `--json` or `--machine` body read, like a machine inbox read, is read-only and
moves nothing): when the written chunk reaches the end of the body and the read began at byte 0
or followed the body's own continuation, the CLI sends that one id in the calling binding's `AdvanceCursors`
(any role; a subagent moves only its own cursor). So a message delivered truncated, whose mod or hook
settlement is refused, no longer holds the binding's cursor once it is read whole. A partial read (a chunk
short of the end, or a read from an explicit offset past 0) moves nothing, and neither the read nor an
explicit `ack` settles or moves anything else: an `ack` carries no display, and a message the binding read
whole was already passed by that read. The walk also counts as displayed a message whose seat receipt this
same binding display-ACKed (`ack_binding_id` is the walking binding and the ACK observation carries
`action_provenance`, which only display settlements write: text-inbox display, mod or hook-pull delivery),
so a later message shown whole while the cursor was held behind a truncated one is passed together with it.
Another binding's ACK and an explicit (non-display) `ack` are never such evidence. Accepted limit: a
continuation chain begun at an explicit offset counts as whole (the continuation does not record where the
read began).

**Direct Lazy readers during an invitation.** An exact published Lazy recipient in an immutable
canonical pair may be read before acceptance in any active invitation episode, not only the first.
For a binding without its own cursor and an unjoined recipient, admission requires the exact pair
endpoint/instance, a nonretired seat, current voluntarily invited membership, and its exact pending
invitation by `(thread,seat,episode)`. Only messages published after that invitation's immutable creation
decision enter this cursorless stream. Retained `lazy_recipients.state` is not modern binding unread
authority. Existing own cursors retain their addressed unread stream, including earlier episodes and
A8-retained work after leave, retirement or archival; the new episode boundary never raises them.
Joined-reader and frozen legacy-pending rules remain separate and unchanged.

Canonical pair writers make this a bounded current-episode decision. Native, operator and ordinary
service invitations reuse an active voluntary invitation rather than create parallel active episodes.
Rejection clears that episode's voluntary invited marker even if raw invited/pending states remain.
Pairs cannot become service-owned, Required invitations require that ownership, and requirement release
is the only invitation-cancellation writer. Thus exact current pending invitation plus voluntary invited
membership excludes canonically rejected/cancelled pair episodes; future writers must preserve this
implication or revise the predicate and its counted work. This is not a claim about hand-edited stores.

The invitation creation decision is the publication lower bound, not a thread cursor value. When a
cursorless reader needs a baseline, resolve the invitation's actual timeline event using at most three
UNIQUE event-key points: `invite:<id>`, `operator_invite:<id>` and `service_invite:<id>:ordinary`.
Validate instance, thread, kind, source invitation and decision ordering; choose the earliest valid
thread sequence among these fixed results. A current duplicate Ordinary ServiceInvite returns the
original invitation without appending another event. Missing or conflicting event authority is an
error, never a guessed zero/head/timestamp baseline or an invitation-history scan.

An exact currently eligible full display may use this event sequence as a virtual baseline in the
common cursor walk, through both own `AdvanceCursors` and existing logical Lazy completion entry points.
Only resulting progress beyond the virtual floor is persisted; an earlier unshown current-episode
receivable message still blocks progress. Existing cursors start unchanged. A query, partial/failed
output, machine read, arbitrary message ID or physical V2 presentation confirmation cannot establish
that baseline. A delayed older-episode display cannot borrow a newer invitation floor. Existing explicit
published-ID completion authority is preserved with its existing baseline/gap rules where the new
current-episode baseline is inapplicable. Cursor scope is not evidence that older mail was displayed;
Lazy completion remains presentation bookkeeping with no ACK, adoption or participation effect.

Accepting this exact active direct invitation preserves every existing own cursor and inserts missing
open-binding cursors at the event floor, never at the thread head. Capture the floor in the deciding
transaction before changing membership; acceptance is not display. In the existing open-binding fanout,
invalidate each affected binding's reader exactly once even if its cursor did not change: joining also
makes settled addressed ordinary direct messages above the retained baseline newly readable. This
mixed ordinary/Lazy consequence is intentional because they share a cursor. The helper uses exact
current binding/seat scope and two fixed Hook/Watch consumer points, increments only matching existing
private passive-generation/content-version counters, and preserves carry, token, partial body and
parked progress. No consumer-history scan or sibling substitution is allowed. Checked overflow or any
failure rolls back invitation, membership/interval, cursor and all tentative private writes together.
Historical accept replay neither reseeds nor invalidates again. General Join and nonpair acceptance
retain their head policy; no new binding fanout occurs at publication or reading.
The accepting top-level binding's own text `accept` then shows its own unread delivery rows (any priority) of the pair
above that floor in its output (A8, ht-br0) and completes them as its text inbox would: only that
binding's cursors advance, and each sibling binding keeps its independent own stream (no seat-wide
settlement, no legacy-pending marking). Lazy and ordinary rows share that one cursor and the page carries
no ordinary row, so it shows only the Lazy rows below the binding's first ordinary walk stop (A8); the Lazy
rows above it stay in the binding's own stream and its next hook or `inbox` shows them once, together with
the ordinary rows (pinned by
`text_pair_accept_shows_each_lazy_opening_row_once_across_accept_and_hooks`).

A new binding while the invitation is pending can independently receive that episode's addressed work,
even if a sibling displayed it. A new binding after acceptance retains normal joined-thread seat-maximum
or head seeding. These rules do not rewind historical cursors already advanced by prior join or genuine
display, and do not claim to recover historically skipped bodies. Existing explicit lifecycle
fast-forward remains the separate accepted Lazy limit described below. No binding-birth heuristic,
autoaccept, receipt provenance, new schema or deadline is inferred from this reader exception.

For private ordinary presentation only, an intact captured modern Lazy continuation whose current
reader scope has become inapplicable may retire its exact private body/ordinal instead of blocking
later episodes. Exact message/instance/Lazy mode, published addressing, captured publication/range,
canonical binding, and the assembler's matching ID/offset or skip-body state must validate first;
missing/corrupt authority remains an error. This retirement requires `legacy_pending=0`: modern-scope
failure alone never discards the independent top-level legacy-pending branch. Public continuation
remains strict. Advance only that captured Lazy ordinal, clear only its matching body/prefix, and
charge one existing physical candidate visit plus three scope probes once. Preserve the frozen scan
version/passive-generation/publication/ordinal capture so that finishing the old scan allows the next
normal quantum to observe newer publications. Preserve unrelated carry, token, parked state and logical
offer stamps. Retirement creates no canonical Lazy completion, deletion, receipt, ACK, read cursor,
acceptance or display evidence; any failure rolls back private progress. Existing private work caps
remain unchanged, with no history scan, event lookup or new durable state.

**First-contact stats and fast-forward.** Two bounded, informational summaries ride on the response that
creates or resumes a binding; neither is authority and neither writes a receipt. The lifecycle check-in that
*enrolls* a binding carries per-thread history counts (at most 8 threads and one count, no summary text), once;
an inbox read of a session the daemon has not enrolled carries the same counts and enrolls nothing (the CLI's
following `AdvanceCursors`, possibly empty, enrolls it). A lifecycle check-in that *reconnects or reopens* an
existing binding whose unread backlog exceeds one ordinary inbox page jumps that binding's own cursors to the
thread heads and reports the gap per thread. Apart from the join frontier (below), this is the one place a
cursor passes unshown mail the seat can receive; it is decided in the daemon in the check-in's transaction, never on enrollment, a plain touch, a plain-call reconnect or a
supersede-induced disconnect (the superseded binding's cursors are untouched until it itself resumes), and it
moves only the resumed binding's cursors. The seat's still-pending ACK obligations are not cursor-bound: the
inbox keeps listing them until they are settled, so a fast-forward never hides or settles one, and a subagent
binding's fast-forward moves only its own cursors. Accepted limit: a binding a plain call reconnected is
already connected when its own lifecycle resume arrives, so that resume is not a reconnect and does not
fast-forward. The limit covers only a binding a plain call reconnected after a process exit.

**Join frontier and pre-join lazy mail (ht-8y9).** Except for the exact active direct-invitation
acceptance described above, every transition of a seat into Joined (`accept`,
`accept-required`, `join`, thread creation and a direct pair's Ensure for its caller) runs one
deciding-transaction step, `read_cursors::init_for_join`, in this order:

1. *Seat-wide pending population.* Every `lazy_recipients` row of the seat in that thread whose published
   message's sequence is greater than the seat's furthest cursor there (the highest cursor any binding of the
   seat holds in the thread, open or ended; 0 when none) and that no completion has displayed (`state` is
   still `pending`) is marked seat-pending (`legacy_pending`, the same flag rows carried from before V34 use).
   So a row displayed out of order while a cursor was held behind an undisplayed one (a truncated delivery)
   is never shown again after a join. Nothing else selects the population: not membership intervals and not
   when the row was addressed. These can include retained direct-send target rows reached by a generic Join, plus joined-era
   rows no binding has passed yet. Exact active direct-invitation acceptance uses its own event floor
   instead and does not mark this seat-wide population.
2. *Join-head frontier.* Every open binding of the seat, any role, gets a cursor at the thread head; a higher
   cursor it already has is kept.

A seat-pending row is unread for every top-level binding of the seat whatever its cursor (the lazy predicate's
legacy branch, not membership-gated), and it counts as seat-level unread lazy mail (digest; wake only for a row
its nudge threshold admits, A8). The first completion that displays it clears the flag for the seat. A completion from a
matched top-level binding advances only that binding's cursors: a sibling top-level binding then gets no copy
of it, even though its own cursor never moved. A completion matching no open binding (the pre-binding legacy
form, which completes for the whole seat) advances the cursors of every open top-level binding of the seat. A
subagent binding never sees a seat-pending row (the branch requires `top_level`) and, like every subagent,
cannot complete one; it sees only mail past its head cursor. A binding enrolled later sees still-pending rows
if it is top-level. On a generic `accept` or `accept-required` (not the active direct-invitation
exception), the accepting top-level binding's own text `accept` displays the joined thread's seat-pending lazy
rows in its output (A8, ht-br0), so that completion clears them for the seat as above and the accept itself is
the first opportunity, with no later hook needed. Ordinary messages are not part of this: one addressed to the seat before it joined (an
ACK request to an invitee, a direct `--nudge` target) is a pending receipt from publication, listed with the
seat's ACK obligations until settled and never re-offered through the cursor stream after the join, because
every cursor starts at the head. That pending-receipt path is an agent seat's; a human seat's receipt is waived,
and the join frontier gives a human seat's pre-join ordinary messages no separate path (unchanged from before
ht-8y9).

A rejoin follows the same rule: an older sibling's cursor rises to the head, past joined-era mail another
binding of the seat read or passed (lazy rows at or below the seat's furthest cursor are not marked; ordinary
ones keep only their seat-level receipts). The furthest cursor need not come from a display: a resumed
fast-forward, an ended binding's cursor or a subagent's `AdvanceCursors` can set it without completing any
lazy row. So only undisplayed lazy rows past the furthest cursor are guaranteed to survive a join (marked
seat-pending); an undisplayed lazy row at or below it is passed by every binding. Example: A fast-forwards past
unshown lazy rows 1..50 while sibling B is at 0; after a leave and rejoin rows 1..50 are not marked and B rises to
the head, so no binding sees them.

## Accepted limits

For the generic join frontier (excluding active direct-invitation acceptance above), pre-join lazy
mail is seat-level, not per binding (A8). A seat-pending row is settled for the
seat by the first top-level binding that displays it; a sibling top-level binding gets no copy, and a subagent
never sees it. A join raises every open binding's cursor to the head, so on a rejoin an older sibling skips
joined-era mail another binding of the seat read or passed. A lazy row at or below the seat's furthest cursor
is never marked, even when that cursor came from a fast-forward, an ended binding or a subagent's
`AdvanceCursors` rather than a display, so such a row undisplayed by any binding is passed by all of them after
the join. This keeps a mixed opening (lazy and ordinary rows
interleaved) from re-offering ordinary rows already offered or ACKed as pending receipts, which one per-binding
cursor cannot express. A human seat's pre-join ordinary messages get no pending-receipt path (unchanged).
The narrow active direct-invitation acceptance exception preserves independent own streams, as
described above; its mixed-mode read behavior differs from this generic join frontier.
The text accept's thread-scoped display (A8, ht-br0) is pinned by `tests/hook_entrypoint.rs`
`codex_pair_accept_output_carries_the_opening_message`,
`text_pair_accept_shows_each_lazy_opening_row_once_across_accept_and_hooks`,
`text_pair_accept_with_more_than_a_page_prints_one_working_inbox_next_line`,
`text_pair_accept_completes_the_opener_for_the_accepting_binding_only`,
`text_generic_accept_shows_seat_pending_lazy_mail_once` and
`human_json_and_machine_accept_leave_the_opening_message_pending`, the mod tests
`text_accept_with_a_live_mod_channel_prints_nothing_when_the_channel_completes_in_time` and
`text_accept_with_a_live_mod_channel_shows_the_opener_after_the_grace`, the store tests
`thread_scoped_page_lists_only_that_threads_lazy_rows` and
`thread_scoped_page_stops_at_the_first_unread_ordinary_row`, and the CLI unit tests in
`tests/cli/accept_delivery.rs`.

Delivery-only cleanup recovery retains a private, bounded versioned terminal record after
intent/progress removal. It preserves exact original immutable intent bytes and the genuine
completed fence result and staged-work report; it records no new provenance. This local
record never authorizes live work or proves canonical completion: every replay must classify
the original actor, compare the canonical namespace and obtain the exact canonical Completed
fence before presentation or cleanup. It permits honest replay after output, unlink or directory
sync failure, at the cost of retained local disk records. It is not live archival protection;
malformed or contradictory records conservatively refuse, and canonical terminal state dominates
stale archival scans. Public new modes require the guarded runtime capability. Delivery
carries the exact frozen Agent original and namespace on restricted phase and read
requests; the daemon compares its independently selected path bytes before preparation,
effects, replay and historical presentation. Read-only preparation checks the canonical
caller, channel and recipient before local publication. Each live effect checks current
recipient eligibility in its deciding transaction. A resolved unbound recipient is allowed;
this neither starts an agent nor accepts an invitation or ACKs a receipt. Completed
presentation checks the exact canonical fence and retained report without live mapping
requirements. Generic runtimes without the required producer/deciding adapter do not
advertise this capability and cannot execute these routes.
Linked bootstrap Invite and Send phases derive their required namespace and exact
payload from canonical child-key registration, full frozen parent and attachment.
Missing selected context refuses even cached history; unregistered legacy phases
retain their existing behavior. Completed linked history skips only live guards.

A retained successful version-1 bootstrap launch report is
validated against frozen V1 composition: the original generated prompt bytes and the
original Codex/Claude acceptance grammar with empty owned argv, never today's renderer
or registry. Deciding completion, completed preview, terminal decode and presentation
share it. Validation does not repeat the producer's filesystem routing lookup: it
accepts only the verbatim canonical-namespace routing object (valid instance UUID) or
the null-routing form, with identical fallback and suffix; any other routing refuses.
So if a symlink component appears in a not-yet-existing namespace suffix, or the host
endpoint parent changes, between preparation and launch, the launch-time canonical
object differs and that genuine report refuses, without relaunch or data loss.
Fresh Root preparation and launch still apply today's native admission and geometry,
and refuse before effects when today's composer would differ from frozen V1, so a
future composer change stops new Root launches rather than producing reports that
completion cannot validate.

Canonical automatic handoff storage retains the full normalized V2 original request,
including frozen scope, caller, digest, namespace, child keys and any outer launch,
context and startup template, within 128 KiB for historical absent-policy originals
and 512 KiB for modern explicitly frozen invitation policy originals. JSON escaping counts;
a syntactically valid plan can exceed this representation ceiling. Shared request
validation refuses it before local publication or canonical effects; no fields are
truncated or omitted to make it fit. Exact historical presentation compares this
complete immutable original and canonical attachment. This bound creates no new
provenance, receipt authority or permission from local data.

The additive V2 automatic topology parent retains the full original agent claim,
scope, namespace, topology, launch, startup template and delivery payload within
128 KiB for historical absent-policy originals or 512 KiB for modern policy
originals. Its exact typed attachment is derived from that original and canonical
creation and ordinary resolution; the corresponding original's capacity applies. V2 has no Invite
phase or wire key. A registered child cannot fall back to unlinked admission.
Possible-start checks require fresh sealed canonical creation evidence, original
claim, exact recipient owner and current pair lifecycle; Send success grants no
start authority. Exact child-completed/parent-incomplete reconciliation retains
only the original successful report and absorbing tombstones, even after binding
or membership changes. It cannot create another message or launch.

V2 recovery uses the authenticated local-account operator lane and an immutable
inspection digest. Its request and complete retained historical result each fit
256 KiB for historical absent-policy originals. Modern recovery retains at most
2 MiB of encoded JSON; escaping and nested copies count. General requests remain
limited to a framed 1 MiB, including modern recovery requests. Fresh overflow refuses
atomically. At most one creation/noncreation decision and one subsequent
cancellation are retained per attempt. Recovery never replaces the original agent
claim, allocates a seat, proves quiescence or authorizes native submission. An
unknown attempt cannot become transport-proven not-submitted; an inspected
noncreation decision starts a distinct attempt while retaining the unknown record.
Modern automatic public admission checks the aggregate future envelope before local
publication and the canonical first Begin repeats that check before inserting a
parent. It derives supported generated startup prompts, frozen template/native argv,
128-byte escaped opaque identifiers, bounded creation/witness fields, the 4096-byte
operator reason and every request/result/reservation/recovery/local envelope. Independent
metadata maxima may therefore be refused even when the body itself is within 65,536
UTF-8 bytes. No retained identity, body or history is clipped. The framed response
exception is exactly 2 MiB only for a known AutomaticBootstrap request with a frozen
modern policy (Some) and its matching result family. Historical absent-policy (None)
requests retain their preexisting 1 MiB response ceiling, as do every other request,
response, watch, service and error. The historical 256 KiB canonical recovery-result
limit is separate: transport does not add a semantic validator for handcrafted
historical results between 256 KiB and 1 MiB.
The client chooses its allocation limit from the known request before reading the
frame body and checks result family plus request/instance/boot and embedded original
identity correlation.

A modern successful automatic launch canonically retains the explicitly versioned
`automatic_launch_summary_v1`: truthful started outcome, exact pane/seat/harness and
composed argv, and a separately named `full_report_digest`. Optional generated
diagnostics are omitted from this summary. Their exact generated JSON bytes are
streamed to a private deterministic local artifact after bounded critical started
progress is synced; the full-report digest hashes those exact bytes, independently
of the canonical summary digest. The artifact's path, presence marker and hash are
cooperative local hints, never authority. Critical retry, recovery and completion
never decode the artifact. A missing, unreadable or unsuccessfully written artifact
cannot authorize another native submission or invalidate canonical Completed replay.
A non-Started report uses a distinct bounded local noncompletion observation and
never becomes a successful canonical completion. Any possible native start keeps the
existing no-relaunch guard, including IO failure or interruption before summary sync.
A valid frozen modern original and exact canonical Completed result remain readable
when child progress and the optional artifact are truly absent. If no real retained
message ID is available, presentation uses a null message and an explicit unavailable
marker. It performs no Begin, Ensure, Send, launch or new-capability admission to
reconstruct one. Existing corrupt or mismatched child records still refuse.
Historical absent-policy reports, bytes, digests and presentation remain unchanged.
The generated summary contract covers supported public producers; it is not a
promise to carry arbitrary externally authored diagnostic JSON through a request.

Live automatic parents protect archival before child registration and on the exact
selected thread. Completed or cancelled tombstones release only that parent veto;
replay cannot restore it or cancel a live downstream child fence or journal hint.

Canonical topology storage has encoded-byte ceilings for retained bootstrap envelopes:
512 KiB for the full frozen identity; 128 KiB for metadata-only creation evidence
and attachment; 1 MiB for an immutable recovery decision; and 2 MiB for the full completed result, including
identity, attachment, launcher report and legacy result. The launch report's inner
JSON retains its independent 1 MiB protocol bound. JSON escaping and repeated nested
fields count toward the storage ceiling; a protocol-shaped envelope need not fit.
Oversized writes must refuse before effects and preserve existing records; bounded
reads of oversized or corrupt data must refuse, never present clipped data or grant
submission permission. Records are never truncated to fit. These limits neither
change attribution nor authorize from a client-local journal or today's occupant.
Public preparation also checks the actual serialized request-bearing local progress
against its 512 KiB cap using the observed endpoint witness, before publication,
Begin or reservation. A canonical maximum-size identity need not fit that larger producer
envelope. Future native response metadata is not statically known; an unpersistable created
response conservatively retains possible creation and refuses automatic resubmission.
Transition writers retain the canonical guards and enforce the same limits.
Handoff instructions use ordinary message bytes: nonempty UTF-8 up to 65,536 bytes,
including whitespace and NUL. Exactly one positional, file or stdin source is acquired
once with a bounded read before publication; the actual body, never its source path,
is frozen in every compound intent. Topic and goal limits remain independent.
A maximum six-byte-escaped body occupies 393,216 JSON bytes per occurrence. Delivery
Send envelopes repeat it in the immutable plan and child request (786,432 bytes before
metadata); the global wire ceiling remains 1 MiB. Delivery origins are bounded at
512 KiB and terminal records at 1 MiB: nesting the exact original JSON as a string
adds a backslash to each body escape (458,752 bytes for that occurrence). Bootstrap
origins retain their identity-derived bound; linked progress remains 2 MiB and terminal
records retain their completion-plus-escaped-origin bound. Actual serializer fixtures
cover these repetitions and historical originals without clipping or recomputing digests.
Origin capacity is checked before allocator publication, bootstrap identity and
request-bearing progress before topology effects, and terminal capacity before completion.
The bounded archival scanner uses these same handoff origin/progress/terminal ceilings;
its legacy launch-handoff origin shares the 512 KiB body-bearing cap. Generic non-handoff
record and header caps remain independent, and oversized records still veto coverage.
Independent oversized metadata and native responses still refuse rather than truncate.
The schema upgrade rebuilds only the two body-bearing bounded tables, copies their
historical blobs and operation keys verbatim, and checks foreign keys before commit.

These are decisions, not bugs. Each is safe to rely on only as stated.

- **Bounded advisory evidence suppression.** Negotiated v2 recording remembers resumed creating-CLI
  lifecycle suppression with a fixed 8-KiB monotonic hash filter, scoped by harness, session, domain,
  origin and contract. Hold expiry/eviction cannot forget a recorded suppression during that recorder's
  lifetime. Hash collisions or saturation may withhold lifecycle milestones from unrelated new sessions;
  they never grant evidence verification, seat continuity or receipt authority. The filter is in memory
  and starts empty on daemon restart; suppression does not claim durable session history.

- **Global hooks enroll supported top-level Herdr sessions.** A user-level Claude/Codex hook or
  the agy plugin's hooks enroll every supported top-level lifecycle session in its Herdr
  instance, including sessions started outside `launch`, subject to the ordinary canonical guards
  above. Outside Herdr it remains silent. No wrapper or global configuration change is made by
  enrollment. The actual pane environment must come from the user's foreground harness
  configuration; enrollment cannot infer a different attaching TUI pane or turn a shared-server
  environment into proof.
- **agy's top-level role is read from native runtime artifacts.** The artifacts are the
  conversation's presence lock and the subagent listings under its candidate parents. A
  hook-runner rule that is neutral to the harness reads them and fails toward child: a missing or
  unreadable witness, or one past its budget, makes a child. The resulting role is a cooperative
  top-level claim (`cooperative_top_level`). It is not payload-declared and not daemon-verified.
  The daemon decides every enrollment, rotation and reattachment it leads to against A2.
  A child is answered locally with the subagent restriction and never reaches the daemon. agy
  fires no session start on resume, so the first turn-start hook of a conversation unknown to
  the pane context (a client-local hint) carries the resume or clear lifecycle. Only the pane
  context makes a conversation known; the adapter's per-conversation delivery state never does.
- **A child with a presence lock is still a child through the listing.** A child conversation
  that agy does give a presence lock would read as top-level on the lock alone, but the subagent
  listing still makes it a child. A listing outside the known layout would let such a child
  rotate the pane's binding.
- **A `brain/` directory past 65,536 entries, or a check past its budget, reads as a child.** No
  agy session of that machine enrolls until the directory is pruned. Doctor reports it.
- **A presence lock that appears after `SessionStart` is recovered one turn later.** The next
  turn-start re-check reads it. Meanwhile the session may already have seen a child notice,
  which the top-level check-in explicitly supersedes.
- **An agy session's own CLI calls name its conversation through its environment.** agy exports
  `ANTIGRAVITY_CONVERSATION_ID` to the processes it starts; a `herdr-threads` call under an `agy`
  process is attributed to that conversation's binding (`cooperative_env_session`, A3), and refused
  with `session_claim_required` when the variable is absent (decision 15), never attributed to the
  pane. A child conversation's call carries the child's conversation id; when a parent's subagent
  listing names it, the call is a read-only subagent claim of that parent (native session swapped to the
  parent's conversation, the child id as agent id, `cooperative_env_session`) and enrolls as a subagent
  binding of the parent. A conversation with neither a presence lock nor a listing, or whose witnesses
  cannot be read, is refused `unattributed_session`, never attributed to the parent's top level.
- **Hermes enrolls through its CLI hooks.** The witness is Hermes's native `platform == "cli"`
  (cooperative, A1): delegated children run with platform `subagent`: the bridge answers them locally with the subagent
  restriction, and their `subagent_start` registers the child session as a read-only subagent binding of
  the parent (`subagent_stop` disconnects it). An
  empty or missing platform never enrolls. The start callback enrolls without a check-in (Hermes ignores
  its return value); the first qualified turn checks in. An older installed bridge (no
  `enrollment_witness`) keeps launch-only behavior until `setup hermes --profile NAME` is re-run. There
  is no C1 resume continuity: a resumed session checks in as `Startup` in a seated pane, enrolls through
  the Unowned-turn fallback in an unseated one, and meets the restore hold after a Herdr restart. Wake: a
  bare macOS Hermes is not process-detected by Herdr, so its seat is not woken while idle; `launch --kind
  hermes` stays the way to get a wakeable Hermes. CLI side agents (`/background`: platform `cli`, own
  session, no parent, no native side-agent flag) are indistinguishable from the foreground session: they
  enroll no new seat, but their turns check in as their own binding of the seat, as on a launch-created seat
  (follow-on ht-e73). Operator retire: retiring the seat of a running bare Hermes leaves the pane
  `Unowned`, so its next CLI turn re-enrolls a fresh seat through the ordinary resolver, never the retired
  one.
- **Foreground harness execution is user managed.** Hooks and tools must inherit the TUI
  pane's `HERDR_PANE_ID`. A shared harness server started in another pane can instead supply
  its own pane environment; herdr-threads does not recover the attaching TUI pane from
  process names, cached sessions or focus. Setup and setup-status inspect user settings and
  explain the required foreground configuration. Setup changes it in exactly one place, and
  only with explicit consent: the Codex pane environment below. Claude's settings are never
  written for this. A configured setting or environment is advisory, not execution proof:
  wrappers, higher-precedence settings, other shells and starts outside a Herdr pane remain
  the user's responsibility. Claude's `disableAgentView: true` disables its background agent
  supervisor. For Codex, `daemon_auto_start = false` still permits attachment to an existing
  server. Codex skips the implicit shared daemon whenever `CODEX_EXEC_SERVER_URL` is set
  (presence, so an empty value counts) and treats an empty value as ordinary local execution,
  so the persistent opt-out is that variable exported empty inside Herdr panes. Native
  `--no-daemon` and optional `HERDR_THREADS_CODEX_OPTS` / `HERDR_THREADS_CLAUDE_OPTS` launch
  arguments remain alternatives, with no automatic daemon argument by default. No argument,
  settings check or environment block grants receipt authority, moves a seat or relaxes the
  canonical binding checks in A2.
- **The Codex pane environment is a consented, owned shell-startup block.** With
  `setup [codex] --with-pane-env` or a typed yes to setup's terminal question (default no),
  setup appends a `HERDR_ENV`-guarded block exporting `CODEX_EXEC_SERVER_URL=` to
  `${ZDOTDIR:-~}/.zshenv`, to `~/.bashrc` and to bash's login file (the first existing of
  `~/.bash_profile`, `~/.bash_login`, `~/.profile`, else a created `~/.bash_profile`,
  because Herdr starts pane shells as login shells and a login bash does not read
  `~/.bashrc` by itself; a created `~/.bash_profile` holds only the block, so it shadows a
  `~/.profile` added later, which the advice states), and writes a whole fish drop-in under
  `conf.d`, for `$SHELL`'s
  shell plus each shell whose startup file exists. zsh files follow setup's own `ZDOTDIR`; a
  Herdr server started without it reads `~/.zshenv` (the advice says so). Without a flag
  setup refreshes only a block byte-identical to what it recorded writing, never adds one,
  never adopts a block it did not write, and without a terminal only advises. A typed yes
  adds missing blocks and adopts an unrecorded current one (the question names both sets of
  files), but never replaces a block the user edited; only `--with-pane-env` does, after a pre-image. The installer
  (`internal installer-integrations`) never writes these files; it only reports and
  advises. Each write is recorded in a private manifest under `<state>/setup/`, preceded by
  a timestamped pre-image of the existing file, and published through the staged
  replacement; symlinked, special or over-1 MiB files are skipped with the line to add (or,
  on removal, the block to remove) by hand. Unsetup and `--without-pane-env` remove a block
  only while byte-identical to the record; they delete a file setup created once nothing
  else is left, and a recorded fish drop-in that still holds exactly the recorded content
  (with a pre-image when setup did not create it). Launch cannot set the variable (Herdr's
  agent start carries argv only) and only reports whether a block is present. A present
  block makes the Codex foreground status read `configured` (`modified_by_setup` only when
  the ownership record names that file), which stays advisory: it is never execution
  evidence, a provenance value or attribution authority, and no seat, binding or receipt
  decision reads it.
- **Native permission rules are cooperative matching limits.** Owned Claude rules allow
  the bare `herdr-threads` / `ht` command text and ask for `human`, `setup`, `unsetup`,
  `doctor fix` and `internal installer-integrations`; owned Codex rules use a literal
  executable-prefix allow with `prompt` prefixes for the same words. The CLI refuses an
  agent that does not write those words first, but quoting or rearranging words to dodge
  a text rule is outside the model. Stronger deny/ask/managed policy remains effective.
  Foreign broad allowances are preserved and can still cover Human commands. Bare names
  cannot attest later PATH, alias or function resolution. Generated matching and native
  checker evidence never prove a live classifier verdict or installed wrapper support.
  Apart from the consented Codex pane environment block (an environment export, not a shell,
  network, sandbox or approval-mode policy), setup changes no general shell, network, sandbox
  or approval-mode setting. The owned
  agy allow member `command(herdr-threads)` has no ask counterpart herdr-threads can
  write, so once granted it also covers person and setup commands; agy's consent
  question says so, and it is installed only with consent (`--with-permissions` or a
  yes) or kept from an earlier recorded grant.
- **Unattributed public reads retain ordinary daemon spelling.** Public read requests
  carry no invocation actor; a Human subject seat or peer label cannot identify the
  requesting viewer. Known current caller/action producers include Human spelling before
  page fitting. For an explicit Human invocation of a generic public read, the client
  renders a clone of known argv fields and measures its final selected encoding against
  the original byte budget. Namespace overhead that cannot fit reports the exact required
  minimum, including framing and newline, without changing cursor or rows or refitting a
  historical response. Frozen check-in claims determine their own historical ready spelling.
- **Owned configuration locks coordinate participating writers only.** A stable advisory
  lock serializes owned writers, which refresh and revalidate under the guard before each
  replacement. A noncooperating editor changing a file after the final check and before
  rename can escape detection; the guard is not atomic compare-and-swap. Historical
  ownership transfer recovery covers process termination on a functioning filesystem,
  not every power loss or storage failure.
- **Child agents can ACK.** Subagents are instructed not to; the plugin cannot tell them from their parent.
- **Same-user spoofing.** Any same-user process can select another seat, set `HERDR_PANE_ID`, or write another
  seat's files under the instance `contexts/` and `intents/` directories (writable from the Codex sandbox by
  design). Forged client files cannot satisfy A2's binding match, but they can disrupt that seat's local
  state. Sandbox-writable files are opened without following symlinks (implemented for `allocator.lock`).
- **Unseen exits.** An agent that exits to its shell stays bound until the next check-in or retirement (C5).
- **Launch is not check-in.** A successful `launch` means Herdr started the agent, not that it registered.
  Its only record is a wake-eligible placeholder binding (`managed_launch`, A3) carrying the started process's
  fingerprint: not
  a registration, availability, receipt or authority for any accountable action, and replaced by the agent's
  first lifecycle check-in. Forms outside the captured native launch grammar remain refused
  (including `codex resume`).
- **Integration readiness is advisory for launch.** Missing, stale, disabled, untrusted or
  wrong-profile owned hooks do not authorize attribution and do not alone block a structurally
  safe selected native launch. No configuration descriptor or fingerprint is invented when none
  is used. Registration identity, selected native binary/API/profile/home, argv/environment,
  caller/seat/empty-shell and host guards remain mandatory. Actually used owned configuration
  retains its exact fingerprint recheck. Hermes also rechecks validated selected native launch
  inputs: runtime/source/module/interpreter identity, profile/logical and physical home,
  environment/bootstrap and API evidence. Only the schema3 top-level `enabled`, `disabled`,
  `config_quality` and `fallback_kind` diagnostic values are excluded from that launch stamp;
  the complete observation stamp and all schema/fallback rejection rules remain unchanged.
  Native trust refusal stays a refusal; launch never auto-approves, accepts invitations or ACKs.
- **Process hints are cooperative recognition input.** A registered launch policy may require Herdr's
  narrow `process_hint` mode, advertised by each compatible start negotiation and fenced to the actual
  local server peer before submission. The request and startup correlation record the requested mode,
  not an effective child environment or proof of native execution. Same-user processes can inherit or
  imitate recognition hints; platform support, environment readability and descendant coverage can
  prevent recognition. Hints add no registration, receipt, continuity or caller authority and no new
  provenance (A1–A3, C1); arbitrary environment overrides remain refused. A possibly submitted start
  with an uncertain response remains unknown, without a safe fallback or inferred binding.
- **A launch binding can outlive its agent.** Like any binding (Unseen exits), a `managed_launch` binding
  whose agent exits before checking in stays until a check-in of its harness replaces it or the seat is unresolved or
  retired (the liveness sweep disconnects it once its fingerprinted process is gone, without ending it); a later
  `launch` into the seat records another placeholder, and a wake for it is refused when Herdr's agent kind
  differs from the recorded harness.
- **Restore costs operator time.** A genuinely new role in a restored pane waits for an explicit choice when
  cooperative continuity does not apply.
- **Any invocation of a seat can enter catch-up.** CLI calls cannot tell a seat's top-level agent from its
  child workers, so any `summary` call by the seat may open its catch-up row. Harmless: workers exist only
  after the parent's own `summary` entered the row, and an active row keeps its frontier, so repeated or
  worker calls never move it.
- **Summary blocks name the seat, not the invocation.** A block records its author seat and declared model;
  it does not record whether a child wrote it, because the CLI cannot know.
- **Optional runtime qualification may be absent.** Core contracts can operate cooperatively without
  runtime metadata. Unknown metadata inherits no optional compact, composer, turn-time poke or native
  receipt capability. Historical captured declarations (Claude 2.1.287's richer behavior; every Codex
  version declares neither rich poke capability) remain evidence for their exact captures, never a grant
  to an unidentified current runtime. Working agents and typed drafts always defer native attention,
  including with historical turn-time qualification. The hard-deadline warning remains
  durable pending attention, subject to the same native composer admission when waking.
- **Unavailable-runtime diagnostics are bounded advisory history.** Failures use only the existing
  harness/session/contract producer scope, never an invented version/build key or account authority.
  The first event/field stays sticky across later successful inputs. Storage keeps at most 256 rows per
  harness with deterministic eviction, a 30-day retention projection and at most 20 rows per read;
  Health shows a failure only in its 24-hour window. Retention expiry or bounded eviction can retire a
  failure; success cannot clear it. Empty or missing session identity records a bounded parse diagnostic
  without inventing a session. Codex rollout creator metadata never identifies the current runtime,
  including startup and known resume. Exact attributed historical evidence remains unchanged.
- **Pokes skip Claude panes that show composer text.** A Claude pane whose composer shows a prompt
  suggestion (or any typed text) is not poked: the suggestion cannot be told from a draft without captured
  styling. The skip is for that poke only and is retried after the wake retry spacing; only the
  hard-deadline warning reaches a pane that keeps showing a suggestion (ht-1ip.46). Claude shows one after
  every turn by default, so this skips most idle Claude panes unless suggestions are off: with
  `promptSuggestionEnabled: false` an idle Claude composer reads empty (a bare prompt or a captured
  placeholder) and the poke is sent. `setup claude` explains this and offers to turn suggestions off; it
  writes the setting only on an interactive yes or `--disable-prompt-suggestions`, never silently, and
  `unsetup` reverts what it set (ht-6jt). A project or managed setting, or the session's
  `CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION`, can turn them back on; the poke then skips as above.
- **Native attention uses sampled composer/focus guards.** Check-to-send focus or input races are
  accepted: a user may type or change focus after the final read. Empty samples do not establish an
  absence of keystrokes, cursor moves, or type/delete activity between samples. Focus is Herdr's pane
  selection, not every attached client's physical attention. Restart/outage gaps can delay focused
  wakes; nonempty/unreadable composers can defer indefinitely. No atomic host/client buffer claim is made.
- **Composer reads see the screen, not the buffer.** Herdr's detection read shows composer rows without
  trailing whitespace, so whitespace typed at the end of a draft row is invisible, and a draft of only
  spaces can read as empty and be nudged over. A draft typed to match a captured
  Claude placeholder exactly reads as empty the same way. A Claude placeholder that was never captured reads
  as unknown, so native attention remains deferred until the placeholder is captured and listed or
  the composer becomes recognizably empty; hard-deadline warnings remain pending.
- **Conservative archival liveness.** Joined-agent archival qualifies against the exact existing cooperative binding and fresh structure plus empty composer evidence from its registered parser. The reported host kind selects that parser and must agree with the bound harness; it is not occupancy or current-execution attestation. Unseen exits and same-user imitated recognition remain the cooperative model's accepted limits. A shell/unrecognized kind, absent provider, unreadable composer or changed canonical binding cannot qualify. Archival does not establish liveness, availability, execution or receipt authority. Repeated composer-aware idle observations use a 60-second cadence
  with a maximum 120-second gap and fresh evidence window. Boot changes, outages, clock discontinuities
  and missed observations restart qualification. Partial/malformed/inaccessible legacy journal coverage
  vetoes cleanup. Every published noncompound intent, even a valid Send or ACK, also vetoes
  coverage until recovered or absent in a fresh scan. This creates no canonical hint and the importer
  never deletes intents. Fully verified compounds alone may create veto hints, with completed canonical
  identity taking precedence transactionally. Published records follow the journal's immutable/rename
  contract; source generation tracks directory publication/removal/replacement. Bounded validation uses a global canonical mutation fence, so continuous unrelated
  activity or a channel too large to refresh within that window can keep it open indefinitely. Structural
  snapshots alone never prove idle; the existing composer visibility limits apply to archival too.
- **Probabilistic identifiers.** Pagination cursor binding tags are 48 bits (about 2^-48 acceptance per forged
  or stale cursor; scope, direction and order are still compared exactly). Send-preparation ids are drawn from
  62^8 ≈ 2^47.6; reuse of a retired id has probability about (retired ids) × 2^-47.6 and is harmless once
  nothing references it.
- **The Herdr incarnation is a same-process inference.** The daemon names a Herdr incarnation by the kernel
  peer of its API socket: PID, effective UID and process start time (macOS `proc_pidinfo`, microseconds;
  Linux `SO_PEERCRED` and procfs, boot time plus clock-tick start, typically 10 ms). A new process reusing
  the PID within the same start tick would read as the same incarnation. On Linux a wall-clock step moves
  the boot time, so the same server then reads as a new incarnation: holds (C2), never a false match.
  Other platforms have no witness: every observation is unverified and host-observed flows refuse.
- **Mod delivery claims are cooperative.** A same-user process could run `herdr-threads watch ack` and settle
  receipts as `cooperative_mod_delivery`; the daemon checks the canonical binding the claim's key names and that binding's live
  channel, not the process.
- **Hook delivery claims are cooperative.** A same-user process could send `DeliveryConfirm` for ids the daemon
  offered and settle them as `cooperative_hook_delivery`; the daemon checks the canonical binding, its
  generation and its offered ledger, not the process.
- **The offered ledger is process-local.** A daemon restart loses it: a later confirm is `not_offered`, the items
  stay pending, and the next pull offers them again (at-least-once): a pull is never answered unchanged before the
  daemon process has read the binding's whole stream (pending plus settled-but-unread), whatever version the hook last saw.
- **Re-offers are bounded.** An unconfirmed id is shown at most 3 times per binding generation, 60 s apart, to
  each binding; after that it waits for `inbox`. Each binding has its own offered ledger: at most 256 ids, never
  evicting one. While it is full, new ids are not shown to that binding by the hook (they stay pending). While a
  binding a wake would reach holds a parked id, or its ledger is full, that wake names `inbox`.
- **An unreadable transcript leaves items pending.** A hook-pull hook confirms only ids whose frame header it
  found at the start of a line in a hook-injected step of a transcript it could read (text in tool output, model
  output or user input never confirms; malformed records are skipped): a missing, unreadable or unparseable transcript, or one that grew
  more than 8 MiB past the recorded injection offset, settles nothing (the items stay pending and are offered again
  within the re-offer bounds, never settled; the hook stops tracking an id once more than 8 MiB of transcript follows its injection offset, so such an id never holds back the confirmation of later frames), and a cancelled or errored turn counts as no evidence about
  delivery. Absence is a downgrade signal only when the transcript was read in full over two distinct turns of
  one agent executable identity. The hook trusts the `transcriptPath` its harness payload names (cooperative
  model; the file is only read, never trusted for authority).
- **A resume keeps the binding.** A resume re-registers the same session's binding and keeps its offered set; a
  new session (`/clear`) is a new binding with an empty one, and a rotated binding generation discards it.
- **A native wake marker names `inbox` only for a harness whose turn start injects nothing.** A hook-pull harness (agy) is
  woken with the content-free `herdr-threads: wake up` only when its next hook pull would carry something: an id shown
  less than 60 s ago is not wake work until its re-offer is due, a seat whose next pull would show nothing is not woken,
  and a binding holding ids parked for `inbox` is woken with the marker that names `inbox`. A woken pull
  session that also runs `inbox` may display bodies twice; it never settles twice.
- **Engine-reported idleness is trusted.** The mod takes busy and idle from Claude Code's turn events; the
  post-abort hold bounds the known Esc takeover.
- **A remote mods kill switch returns seats to native wake.** A mod that never loads looks like no channel.
- **A reload during an in-flight submit is settled by inference.** It now concerns only a record an older mod instance
  wrote. An idle delivery is settled by its append; a wake submit that drops is retried, and the pending wake survives a
  reload in the mod's state, so the appended items are not left without a turn. The successor treats the turn open at its
  load as the submitted turn. If that submit produced no turn and the successor sees no turn start for 120 s with an
  empty prompt box, it delivers the ids again; a submitted turn that both started and ended unseen inside the
  reload gap can be delivered twice.
- **An unrecovered handoff is at-least-once.** An item the mod delivered whose ACK did not land before the
  channel dropped stays pending; the mod re-ACKs it after re-registering (same generation, or on resume the
  immediately previous generation of the same native session), and otherwise the agent may see it again
  through `inbox` or the native wake. Never lost. `/branch` keeps the transcript under a new session id, so
  items delivered but not ACKed before a branch may be delivered again.
- **A text accept with a slow live channel is at-least-once.** When a live mod channel has not completed the
  accepted thread's lazy rows within the accept's bounded grace (A8), the accept shows and completes them itself;
  a channel append that lands afterwards may show them again in the transcript. Never lost, never settled twice.
- **An outer mod could strip context.** A mod wrapping `tool.call` outside herdr-threads' hook could remove
  the attached context after the herdr-threads hook returned; the receipt is already settled.
- **Re-registration resets the stall clock.** Each registration starts a fresh channel whose stall clock
  begins at registration, so a mod whose `watch` keeps reconnecting without ever delivering can hold native
  wake off for as long as it keeps reconnecting. Deadlines and hard-deadline warnings still run; disabling
  the mod (`mod_delivery: off`, or `unsetup`) returns the seat to native wake.
- **A daemon restart can deliver twice.** After a restart the wake lane's first pass can run before the mod
  re-registers, so one item may reach the agent both through native wake and through the mod. At-least-once,
  as above; never lost.
- **Mod frames keep peer text off the header column.** The mod indents every body line by two spaces, as the
  compact `body` read does, and folds thread, sender and id onto the block's header line, so a line starting at
  column 0 (the one lead line and each `[herdr-threads] <kind> <id> in <thread> from <sender>[ markers]:`
  header) can only come from the mod. Within a header line the thread and sender names are peer-chosen text
  and may contain words that look like markers; the markers the mod writes come after the sender and attribute
  the source without granting permission.
- **Claude subagent attribution is a best-effort shell rewrite.** The PreToolUse hook adds
  `--agent-id=<agent_id>` after the command word of every simple command whose command word is
  `herdr-threads` or `ht` (after `;`, `&&`, `||`, `|`, `&`, newline, leading `VAR=value` words and the wrappers
  `env`, `command`, `nice`, `timeout`, `time`, `nohup`). Claude subagent calls whose shell shape the rewrite does
  not recognize (a script that calls `ht`, `eval`, `bash -c`, `xargs`, command substitution, a heredoc, a quoted or
  path-qualified command word) carry no `--agent-id` and are attributed to the parent session. A sibling PreToolUse
  hook that also returns `updatedInput` may drop the rewrite (last hook wins); the call is then attributed to the
  parent session. The rewrite never returns `permissionDecision`, and a same-user subagent can still pass any
  `--agent-id` it likes (cooperative model).
- **Peer data is cooperative input, never authority.** Peer data (message bodies, topics, names) is cooperative
  input from same-user agents and is presented without adversarial framing; it never overrides the user's
  instructions or permissions, and escaping exists to keep framing intact, not as a trust boundary.
- **Any same-user process can change the nudge threshold.** The preference (and its legacy `wake_on` alias) is
  cooperative like every other seat claim (A1): a same-user process that supplies the seat's claim can set it to
  any value -1..=9. The effect is bounded to wake eligibility (which pending messages may wake an idle seat); it creates no receipt, ACK evidence,
  attention or authority, and the daemon records only the seat's own current top-level binding as the actor.

## Decision record

### Wrapper compatibility and foreground execution (2026-10-06)

A user's `codex` alias resolves to a wrapper that rejects native `--no-daemon`. Removing
automatic injection preserves that wrapper's argument contract. The isolated native Codex
0.160.1 probe shows that model-selected Bash execution and lifecycle hooks both inherit the
app-server environment; overriding tool environment alone does not fix hooks. User decision:
rely on user-managed foreground settings, inspect and explain them during installation, and
offer optional per-harness launch arguments through environment variables. No additional
wrapper or heuristic TUI-to-seat association is introduced. Setup never writes these
foreground settings. The retained measurements and source limits are in
`docs/compatibility/wrapper-seat-detection.md`.

### Codex pane environment (2026-10-12)

Codex source at rust-v0.162.1 (historical evidence, never a version gate) shows the TUI does
not select or auto-start the implicit shared app-server daemon while `CODEX_EXEC_SERVER_URL`
is set: `codex-rs/tui/src/daemon_startup.rs` `exclusion()` reads it as presence (an empty value
counts), `startup_orchestration.rs` then skips the default daemon socket and auto-start, and
`lib.rs` `app_server_target_for_launch` reuses a local daemon only while it is unset. The exec
server treats an empty value as the local environment (`exec-server/src/environment_provider.rs`;
only `none` disables it). Exporting it empty in every Herdr pane keeps Codex in its pane
whatever wrapper starts it, including wrappers that reject `--no-daemon`. User decision:
`setup codex` may add a `HERDR_ENV`-guarded block to the shell startup files, with consent
only, owned, recorded, backed up and reversible like other setup edits; the installer only
advises. This supersedes "Setup never writes these foreground settings" above for this one
variable; Claude's foreground settings are still never written. Seat attribution, binding
checks, provenance and receipt authority are unchanged: a configured block is advisory, not
execution proof (other shells, wrappers that reset the environment and non-Herdr starts are
not covered).

### F6: restored-pane creation versus repair reservation (resolved 2026-10-01)

Roast 1 left open whether a restored pane may get a fresh seat before the operator repairs the old one. The
dissent mattered most under the adversarial model; under this policy the risk is a stranded or misattributed
seat, never a compromise. Decision: keep restore holds (C2, already implemented), make them lift when nothing
is left to repair, add cooperative continuity so a resumed session reattaches itself (C1), and resolve
collisions only by explicit abandonment (C3). Rejected: optimistic allocation with merge (merges are the
misattribution this policy exists to prevent); narrowing holds by address or terminal hints (unreliable across
an incarnation, so either F6 again or the same holds); no reservation (loses obligations on every Herdr
restart).

### Thread summaries and deadline extension (2026-10-02)

Summaries are agent-produced, daemon-validated data under `derived_summary`; priority comes from recorded
claims (`author_role`, `relays_user`), not from the text. Catch-up extends effective deadlines once on the
first entry or an entry after a ready or superseded row, and otherwise only on stored progress (A6; re-entry
after a stall grants nothing until a block is stored, ht-hqg). The A4 poke rule (soft-deadline pokes) is defined with its behaviour above.

### Managed launch binding (2026-10-03, ht-5n6)

Codex 0.159.3's TUI runs SessionStart hooks only at the first turn, so an agent launched without an initial
prompt never checked in, its seat had no open binding, and the lost-prompt idle-recovery wake (which requires
one, A4 *Wake*) never fired. Decision (user, 2026-10-03): `launch` reports its host-correlated startup and the
daemon opens an unregistered `managed_launch` binding (A3) on a seat with no open binding (ht-54r.4: now beside
any bindings the seat holds, with the started process's fingerprint); it carries the
harness the wake recheck compares and nothing a caller can claim. Rejected: waking a resolved but unbound
seat whose pane Herdr reports as an idle agent (Herdr's agent field may only suggest, C1/C5); documenting
lost-prompt recovery as unsupported on Codex 0.159.3 and later.

### Mod delivery is the receipt (2026-10-09, ht-j16)

The 2026-10 direction forbids blanket read or delivery auto-ACK. In this run the user approved an exception for
the bundled Claude mod: "we already judged delivery is the receipt". Precedent: `cooperative_inbox_display`. The
claim is "the full body entered the model's context through the mod", not proof of reading; the mod never
acks a truncated item; its receipt settles only through the agent's explicit `ack` (the truncation marker names
`body`, which is read-only, then `ack`) or a text `inbox` that displays it in full. Rejected: reusing `cooperative_inbox_display` (a different
action); an explicit agent ACK after mod delivery (keeps the tool-call overhead the goal removes); delivering
only the marker.

### Hermes global enrollment through its hooks (2026-10-09, ht-4ne)

Decision: enrollment splits from check-in. A witnessed Hermes CLI `on_session_start` enrolls the pane
(ordinary guarded resolver, daemon ensured, no check-in, empty output); the first qualified turn checks in.
The witness is the platform (`cli`) and the adapter declares an `EnrollmentPolicy`; a qualified CLI turn in
an `Unowned` pane enrolls as a fallback on the existing turn path (a dropped start callback or a cold daemon
recovers on a later turn). No new provenance value: the check-in stays `cooperative_top_level`. Rejected:
`pre_llm_call`-only enrollment; `on_session_start`-only enrollment; Herdr `pane.agent_session` as trigger or
witness; treating an empty platform as top-level; waiting for the agy generic witness; a full lifecycle
check-in at `on_session_start`; an `if harness == hermes` branch in `hook.rs`; a bridge-side foreground lock
for side agents.

### Residual trust-edge findings (bucket B5)

| Finding | Disposition | Invariant |
|---|---|---|
| F6 restore holds | guards + docs | C1, C2, C3 |
| P10 occupancy never EmptyShell | remove dead paths with the verification-layer removal | C5 |
| W5-1 decision fence on raw rows | removed with native permits | A2 |
| W5-2 `stage_recipient` unpinned | add the missing generation-change test | C4 |
| W6-C3 `codex resume` launch uncaptured | refuse until captured | Accepted limits |
| P35 request crosses daemon boot | expected boot in the request envelope | A2 |
| O1 unavailable after daemon restart | carry binding forward on structural reconfirm | C4 |
| ht-yms Herdr timeout ended every binding | unavailability freezes state, writes no invalidation | C4 |
| Wave 18 `ht human me init` as `operator_human` | env-marker and Herdr-agent refusal | A3, A4 |
| Wave 18 `ht human me init` replaces agent binding | daemon refuses agent-to-human without `--operator` | A4 |
| Wave 30 shared client directories | accepted, documented | A1, Accepted limits |
| `allocator.lock` without `O_NOFOLLOW` | open without following symlinks | Accepted limits |
| Wave 28 second agent via name retry | launch refuses while bound agent is live | A4 |
| W9-2 wake ignores harness | compare bound harness with pane agent | A4 |
| ht-5n6 Codex launched without a prompt never checks in, so no lost-prompt wake | `managed_launch` binding after a correlated launch | A3, A4, A5, Accepted limits |
| W6-R2 "mark it self" wording | "when run in this pane" | A1 |
| Waves 29/19 identifier sizes | accepted; fix the 104-bit comment (it is 112) | Accepted limits |

## Bootstrap creation attempts and administrative recovery (2026-10-08)

A bootstrap freezes the original agent claim, scope, namespace and payload. Its
canonical first attempt exists at Begin. Only the transaction first reserving a
prepared attempt issues submission authorization; status, restart and lost-reply
replay never issue another authorization. A fresh pre-submit decision checks the
original A2 claim, exact attempt and reservation administrative revision. A
transport-proven `NotSubmitted` closes that attempt and allocates a distinct next
attempt. A recorded unknown cannot later be reclassified as transport zero submission.
Unknown creation remains fenced; delayed older creation cannot replace
an inspected noncreation decision or a later attempt.

The public administrative grammar is immediate `human handoff recover REF --attempt N`
with exactly one `--created-pane EXACT_PANE`, `--not-created`, or `--cancel --reason TEXT`.
Pinned routing and output flags follow immediate `human`; a root `--operator` form
cannot substitute for this namespace. Recovery accepts bootstrap references only.
The separate local recovery intent has operator UID scope and retains the original
bootstrap reference, agent identity and exact inspected attempt as immutable data.
Its nested original caller claim is a reference, never the operator decision's caller.
The same effective UID must retry it. Original actor classification runs before
completed presentation or cleanup; root agent retry refuses an operator decision.
Public recovery execution remains inert until the production canonical guards are integrated.

Administrative recovery is recorded separately as `operator:local-user:<uid>`,
using the authenticated local-account peer UID. The decision freezes an explicit
attempt number and the complete assertion payload. Exact replay presents that
historical decision even after later attempts; stale undecided attempts and
contradictory assertions refuse. Each attempt retains at most one creation or
noncreation recovery decision and one subsequent cancellation decision. Cancelling
a confirmed creation retains its earlier decision for historical presentation.
Recovery never rewrites original actor claims,
produces receipts, proves occupancy or reports successful launch. Created-pane
assertions need fresh coherent canonical structural evidence and the ordinary
restore/hold/ownership guards; they do not move or allocate a seat. The retained
`structural_reference` is historical operator-assertion correlation, not fresh
admission or authority. A deciding daemon read has its own unchanged call ID and
must supply full workspace/tab/pane/terminal/process incarnation and genuine
endpoint witness from that same response, published through the canonical lane.
Fresh creation recovery compares the complete witness to the assertion and checks
the admission's lifecycle revision plus exactly one publication for both owned
and unowned targets. Exact saved decisions replay before any fresh host read.
Full witness equality is deliberately conservative: socket metadata changes or
an auxiliary socket for the same server may prevent fresh recovery even when
process identity remains unchanged. Metadata does not prove creation, occupancy
or caller authority, and refusal never repairs or rewrites a frozen witness. The normal
operation lock continuously excludes a known in-flight invocation from fresh
canonical inspection through immutable assertion publication and deciding dispatch.
The frozen recovery request retains a versioned digest of the actual canonical
attempt state, creation, attachment and latest recovery sampled under that lock.
Saved retry never refreshes this inspection. After exact committed historical replay,
the deciding transaction requires the retained binding to match its canonical view;
missing old bindings and changed undecided snapshots conservatively refuse.
New-client fresh recovery and every operator retry require the concrete inspected-
recovery capability before publication or deciding dispatch, even for old unbound
plans. Exact old committed requests keep their absent-field bytes, keys and results.
Older clients may fail closed on new guarded recovery results, including results
nested in bootstrap status; an inspection-aware client is required to present these
new records. Guards are never stripped to fabricate older-client compatibility. No heuristic PID or topology
snapshot proves quiescence, and recovery never kills an unowned process.

Herdr offers correlation rather than creation idempotency. The product therefore
sacrifices automatic progress after an uncertain submission. An operator's
mistaken inspected noncreation or quiescence assertion can permit duplicate
topology. That assertion is retained honestly as an administrative claim, never
as transport proof of zero submission. Created topology is never automatically
closed by product cleanup.

Bootstrap cancellation is an absorbing administrative tombstone, not successful
completion. A nonblank reason of at most 4096 UTF-8 bytes and inspected quiescence
are retained with the original identity, attempt, evidence and exact attachment.
Its deciding transaction must prove that no exact downstream legacy child fence
or live local-journal hint exists. Cancellation retains messages, invitations,
membership, receipts and topology; it releases only the bootstrap's protection.
An already completed bootstrap cannot be cancelled or reactivated. A live legacy
child remains protected indefinitely, including after pane loss or seat
retirement; bootstrap cancellation cannot release that child fence. Broader
legacy-child cancellation is outside this feature.

The additive `RecordBootstrapNotSubmitted` boundary reports only the actual
typed transport's zero-byte branch. It uses its own frozen deterministic attempt
key, distinct from creation evidence; the daemon rechecks the original A2 claim
and canonical namespace before closing that attempt. This is a cooperative
original-caller transport-outcome claim under the frozen `cooperative_top_level`
attribution, not independent daemon attestation of a host write, an operator
inspection or receipt provenance. An incorrect producer claim can permit
duplicate topology. Error classes, response loss and unknown submission never
stand in for this claim. Public dispatch remains inert until the actual typed
transport producer is integrated with all canonical guards.

Receipt warning origins retain the canonical immutable message author interpretation:
`author_kind='native'` or historical NULL kind with a canonical same-instance actor seat
and no service author identifies the native origin. NULL kind without an actor seat,
built-in/service authors, labels and unrelated sparse overlay rows establish no origin.
This applies to exact receipt marker triggers and bounded historical projection; stored
message history is never rewritten to manufacture provenance.

Launch setup status failures/unavailable observations are diagnostic and establish no
owned-hook claim. They do not veto an independently validated native launch. The
selected binary, native API/profile inputs and fingerprints of configuration actually
used remain mandatory; incomplete structural observations still refuse submission.

## Bounded invitation presentation (2026-10-11)

Invitation presentation is cooperative FULL OFFER evidence after complete stdout
write and flush, for both fixed Hook and Watch consumers. It is neither invitation
acceptance nor a message receipt ACK, transcript proof, or model uptake. A7's
accepted downstream silent-loss limit still applies. A crash after flush but before
canonical confirmation can physically replay the same logical allocation.

V2 admission resolves the claim to its own canonical binding under A2: open,
connected, top-level, matching the adapter/harness and current mapping, with no
recovery hold. Enrollment provenance is recorded honestly and is not a
`cooperative_top_level`-only gate. A canonical subagent binding is refused even
when its incoming claim looks top-level. Watch Pull/Confirm requires the exact
connected Live channel owner and never reconnects or enrolls a binding. Hook uses
the same canonical plain-call reconnect decision as ordinary hook delivery:
only the existing allowed process-exit/migration case with its fingerprint can
reconnect; superseded/end-event bindings stay disconnected. V2 selects the bounded
presentation view before and after that decision, not an ordinary digest walk.
The resolved binding_id passed between service and store is a hint that the store
rechecks, never caller authority. A per-seat permit may serialize decisions;
it cannot select a sibling binding or share/replace its durable state.

Canonical progress/outbox rows belong to a resolved `binding_id` and one of two
fixed consumers, Hook or Watch: at most two durable consumer rows per used binding,
not per seat. Item bookkeeping is bounded by retained canonical invitation/binding
pairs times two consumers, plus explicit removed-item cleanup debt. The private
relevance proof is likewise binding/consumer-owned. Binding history can multiply
these private rows; there is no seat-count bound or global durable-row cap.
Stale generation/execution state resets lazily for that binding when visited;
another binding's turn never resets it. Seat invitations and receipt obligations
remain shared, while presentation tokens, cursors, carry, controls and notices
belong to the addressed binding.
Continued passive calls drain finite cleanup debt; no cleanup progress is promised
without calls. Context changes coalesce per retained thread: eventual latest context
requires that thread and relevant render shape to become quiescent; every transient
revision is not promised. This does not alter invitation/receipt deadline guarantees.

Migration 44 (the former unlanded migration 40) constructs exact ordinal-seek
indexes over the existing canonical pending-invitation projection. Its index startup cost scales with indexed pending
rows; it seeds no new projection and rewrites no invitation history. The invitation
command epoch uses byte-preserving ADD COLUMN with DEFAULT0 and no added CHECK.
On the actual released STRICT invitation relation, linked SQLite invokes named-table
quick_check: one-time read validation covers retained invitation rows/types/existing
CHECK expressions and invitation index pages/bytes, plus catalog/schema work.
No constant-time, no-row-read or all-retained-table startup guarantee is claimed.
The earlier outer-VM-only metadata/no-row-validation inference is superseded.
Deadline processing resumes after healthy migration without deadline resets.
Canonical readers/writers reject negative epochs and checked increments reject
exhaustion atomically. New presentation tables enforce their bounded-state constraints.

Private ordinary presentation partial state preserves the inherited 8 KiB body
limit. Only the current cursor's one incomplete message may be retained; checked
readers and writers reject multiple entries, invalid message identities, or a
partial at or above 8192 raw bytes. Its JSON bound is 49415 bytes: at most six
serialized bytes per raw body byte, twice the128 ASCII-graphic identity bytes,
and seven object/key/value framing bytes. This private persistence bound does
not enlarge ordinary display, invitation outbox, final frame, or receipt ACK
semantics. Corrupt or genuinely oversized records are refused.


Private capable-delivery ordinary carry stores at most32 structured <=8KiB presentation bodies with one <=8-argument/4096-byte frozen marker prefix. Its serialized cap is155731 bytes:65536 fetched-page bytes +49146 escaped previous-partial bytes +32*512 metadata +24601 prefix +64 framing. This does not increase model output, canonical body or RPC limits. Marker reconstruction uses only the exact producer-generated suffix and frozen prefix; it is not receipt or identity authority. Checked loads refuse malformed or out-of-contract private records.


Ordinary and Lazy publication observation uses a fixed per-binding/fixed-consumer
cursor over immutable instance publication order. Each admitted pull or passive
refresh inspects at most100 manifests before recipient filtering, with at most200
keyed probes and three publication range/head seeks. Each visited manifest first
probes its exact message key, checking instance, thread, decision sequence and
ordinary kind while reading immutable delivery mode. A matching message then
uses one mode-selected recipient point probe by preparation/seat: ordinary reads
prepared_recipients and matches thread, while Lazy reads lazy_recipients and
matches message and thread. The canonical manifest supplies the ordinary
recipient's message association; that recipient relation has no message-id column.
No effective-receipt read, audience fanout or materialization occurs in this scan.
This adds100 source visits:
refresh visits at most160, fresh pull433, plus separately declared indexed probes.
No publication audience fanout, global output version or native wake is added.
Initial/rebound consumers may traverse all retained instance publications before
reaching addressed content. A stable prefix of M rows needs at most ceil(M/100)+2
successful maintenance quanta; with continuing arrivals, finite predecessor count
P gives conservative P+2 quanta, followed by existing ordinary output traversal.
There is no progress guarantee during perpetual admission/storage failure.
Migration 45 (the former unlanded migration 41) adds four checked default-zero
private scalar columns; its one-time SQLite validation may inspect each existing
consumer row per added CHECK (up to 4C row inspections, where C is at most two
consumer rows per used binding). It does not seed message history/projections or
reset deadlines. Development stores already stamped with the earlier redesign's
migration numbers are not claimed to rerun or reinterpret this refinement.
Existing 155731-byte raw carry, 8KiB ordinary body and final64KiB/RPC bounds
remain unchanged.

Direct-invitation Lazy reader eligibility adds three exact indexed relation probes per evaluation:
pair, seat and current invitation, reusing the membership point. It does not seek invitation events in
the presentation predicate. Lazy due/replay costs at most 14 probes; ordinary remains the maximum 18,
so due 1803 and carry 579 caps remain unchanged. Known-offer reconciliation reuses the canonical
message mode/thread/sequence/decision point already read in the same snapshot and removes its duplicate
message join: first point 1 plus eligibility 9 is at most 10 per key, retaining 256 visits and 2560 probes.
Exact recipient, manifest message/thread/instance/publication and binding-seat checks remain required;
client scalars or an uncounted second message lookup cannot substitute for that canonical row.
Fresh/private Lazy scope checks, including continued-body scope checks, charge three additional probes
per physically visited candidate, at most 300 across the existing 100-candidate envelope; rejected
candidates count before filtering. These costs are separate from the unchanged publication-observation
scan above. Production SQL and actual query plans must establish the exact-key accounting.

Direct acceptance additionally makes N reader-invalidation helper calls within its existing fanout over N open bindings;
at most 4N indexed relation reads (binding, seat and two consumer points) and 2N exact private
updates, besides cursor writes. This is fixed work per binding, not constant work for acceptance, and
is outside presentation caps. Each required cursorless event floor uses only three fixed key points.
The inherited public Lazy readers may filter many physical rows before LIMIT, seat unread may examine
multiple connected bindings, and the display walk may cross arbitrarily many nonreceivable messages.
The episode floor reduces old-history work without making these paths globally bounded; the private
100-candidate presentation bound is not an end-to-end public-read or completion-mutation guarantee.

A fresh explicit Watch presentation confirmation may record local channel health
only for an applicable acknowledged Attention and the exact currently Live
seat/binding_id/generation/native-session/lease owner, checked and written under the same
registry entry lock. Hook, passive/clear, replay, AlreadyConfirmed and refused
confirmations do not record or reset that evidence. This timestamp is the
existing receipt-publication cutoff, not a new stall countdown or receipt ACK.
The allocator's single-control-per-page invariant makes fresh confirmation
sufficient to distinguish that first application. Canonical confirmation and
local registry bookkeeping are not durably atomic: a process crash between them
may omit health evidence; reconnect/normal operator recovery remains necessary.

Capable presentation relevance uses the union of canonical direct associations
and prepared representations, including coexistence at the same warning id.
Positive witnesses use exact current local condition/association authority;
unrelated canonical mutations must not delay a still-applicable positive witness.
The global mutation fence is negative-proof invalidation only. Continued unrelated
writes may keep absence Unknown and delay a clear; they are never recipient output
or native-wake authority. One bounded advance serves every successful pull/replay;
explicit confirmation performs no discovery, fallback or revalidation walk.
Future condition/sweep writers and repairs must preserve the exact successor
interval chain and invalidate its canonical fence; repairing that chain is not
an invitation to infer continuity from presentation history or local hints.

Presentation has one current row per binding and fixed Hook/Watch consumer. Its
outbox contains bounded current nonordinary descriptors, not episode history or
ordinary-message ACK authority. Topic changes coalesce to eventual latest context
after updates quiesce; every transient topic revision is not promised. Removal,
binding changes and output completion use the canonical row/frontier rules.
An output offer is cooperative physical emission after successful writer flush,
not evidence that a model read it, accepted an invitation or acknowledged a
receipt. Exact captured control stamps and bounded applicable acknowledged subsets
fence local seen/probation updates; stale, foreign, unknown or refused confirmation
never supplies those effects. Partial retirement preserves original descriptor
slots until that token's work is complete. Confirmation checks the exact binding,
consumer, token and captured control masks; a sibling's token is foreign even at
the same seat and mapping generation. Physical presentation confirmation never
ACKs a message or advances a binding's read cursor. A sibling binding's ACK cannot
discard this binding's outbox or still-unread ordinary carry/offers: settled mail
remains eligible in this binding's own stream, passively, under A8. A sibling's
settlement alone is not evidence that this binding read it; exact canonical
addressing, publication and binding-reader checks still decide eligibility.

A pending Hook outbox retains its originally registered codec selection across
new local System/User preferences. A genuinely removed or drifted codec refuses
boundedly before allocation/retirement; binding/operator repair may be needed.
Client hints never select authority or override the immutable canonical token,
binding, render identity and boot admission. Frame sizing includes the actual
registered native codec or Watch JSON-line bytes; the final RPC wrapper reserve
is separate from the 64KiB presentation bound.


Capable ordinary delivery parks its existing cursor, carry, partial body and
captured scan signals byte-for-byte when the 256-offer ledger is full. Due known
IDs then use exact canonical keys instead of the normal ordinary source: at most
100 attempts and 32 hydrated candidates per call, charged separately as at most
1803 keyed relation probes and 100 indexed single-row availability seeks. The
probe allowance is three setup probes plus 100 attempts at at most 18 probes:
settled-but-unread ordinary eligibility needs five binding-reader probes beyond
the prior 13-probe pending-receipt path. Pending ordinary and Lazy branches remain
bounded by 13 and 14 probes per attempt respectively. This explicitly supersedes
the former 1303-probe allowance; attempts, hydration and seek counts do not grow.
Reconciliation separately inspects at most 256 known keys with at most 10 indexed
probes each: 2560 probes, explicitly replacing the former 1280-probe allowance.
The additional binding-reader checks keep a sibling's seat-level ACK from
reclaiming this binding's still-unread offer. Physical discovery433,
32 blocks/65536 encoded output bytes, sizing352, carry155731 and partial49415
bounds do not grow. Canonical pending/addressed/publication checks still decide
eligibility, including this binding's settled-but-unread stream; a missing key
cannot authorize settlement or an offer-history reset. Physical candidate seeks
are bounded before canonical filtering, so rejected rows still consume work.

Ordinary carry has its own exact-key revalidation allowance: at most 32 retained
items, 579 keyed probes (3 setup + 32×18), 32 availability seeks and 32 body
hydrations. These are separate carry counters, not work hidden inside the
1803-probe due/replay allowance. A carry quantum performs no normal ordinary-page
walk. Carry revalidation, logical due, owner replay and a normal ordinary-page
walk are mutually exclusive ordinary-source modes; their allowances cannot be
combined to enlarge one mode. Reconciliation and independent bounded passive or
control work retain their separately declared accounting. These finite allowances
add no time-budget extension, weaken no cancellation or exact-owner fence, and
install no progress after refusal or transaction rollback.

A private normal ordinary-page continuation may classify its one captured
Unread body against the exact binding's display cursor and canonical receipt.
This separately charges at most 15 keyed probes (five captured-recipient/read
relations plus ten conservative effective-receipt probes) in
`ordinary_continuation_key_probes`; existing setup and body hydration remain
separate. Retirement consumes one of the existing 100 normal candidate visits,
with no additional source walk, due/replay or carry work. Only positively
Acknowledged, RecipientRetired or NotRequired receipt authority plus this
binding's completed cursor may retire that exact captured body. Pending remains
eligible; missing, mismatched or invalid authority refuses. Captured bounds,
binding fence and exact partial-byte/skip bookkeeping remain checked, and a
refusal rolls the private update back. Public inbox cursor validation is
unchanged; no cursor reset or receipt ACK is implied.

Finite failed-key traversal uses two process-local negative caches (Hook and
Watch) per retained ledger binding, each at most256 ID/full-offer-stamp pairs,
scoped to binding_id, execution, ordinary signal, passive publication generation
and render key. They share the ledger's at-most4096-binding LRU lifetime, with
at most256 logical offers per binding. Payload per consumer is
bounded by256*(128 ID bytes + two i64 timestamps + u8 count + bool truncated),
plus a <=128-byte execution and fixed scope; bounded tree/allocator overhead is
additional. Successful updates must match the binding_id, ledger incarnation,
generation and exact current offer stamps. Cache hints never ACK, accept,
complete lazy display or change offer history, and rollback/refusal installs no
progress. A different binding, generation rotation, ledger recreation or changed
scope invalidates the relevant hints; resuming the same binding alone does not
replace it or reset its offered set. Ordinary continuation uses remaining capacity:
a completed normal pass at positive capacity is quiet even with missing due
hints; at zero capacity only unattempted due keys sustain ordinary continuation.
Independent passive/control/outbox work retains its own continuation.

Accepted render limit: a due ID whose inherited ordinary truncation still cannot
fit a standalone output is deferred at its unchanged full offer stamp and current
render/canonical scope, without a logical offer or delivery evidence. A declared
scope change retries it. Topic-only shortening is not guaranteed to retry until
such a scope change occurs. This adds no wake timer, delivery guarantee, larger
output limit or receipt authority.

Accepted normal render limit: at positive post-reconciliation logical capacity,
a canonically validated normal candidate whose unchanged inherited rendering
actually exceeds the standalone encoded-output bound on an empty page may be
deferred for that finite captured normal pass, including a previously unoffered
ID. An exhausted sizing allowance, nonempty-page failure, invalid authority,
malformed item or unavailable/drifted codec does not establish this size failure.
Deferral advances only existing bounded private traversal; it does not change
the candidate's canonical full body, exact addressing/publication, pending receipt,
Lazy display state, binding read cursor or any preexisting full logical Offer
stamp, and allocates no new logical offer. It supplies no ACK, acceptance,
display, output or presentation-confirmation evidence. Later renderable mail may
progress in the same capture, and a completed stable pass is quiet once its
independent work is exhausted. A later independently admitted normal capture
retries the pending candidate; automatic in-context delivery of its unrenderable
shape and a new retry wake/timer are not promised. Unknown deferred IDs may remain
unknown to the model; canonical inbox/history/body inspection remains available.
No normal per-ID negative cache, body fragment or recovery cursor is introduced;
existing source, sizing, output, carry, partial and ledger bounds remain unchanged.
Pre-commit refusal or transaction rollback installs no deferral progress. After a
successful private commit, response/transport/write/flush loss retains that
private progress without granting delivery, ACK, display or confirmation authority.
A caller-visible error with an uncertain commit outcome is not proof of rollback.
Existing due-cache, owner-replay, capacity parking and downstream-loss limits
remain unchanged.


A newly admitted live Watch owner may physically restream its captured pending
ordinary offers once per presentation lease, independently of DP5 logical retry
eligibility. Recovery preserves every full logical Offer stamp, including
nonzero recorded u8 counts above the three-offer eligibility threshold from
supported concurrent recording. Canonical exact-key eligibility still decides
addressing, publication, effective pending receipts, this binding's settled-but-unread
mail and lazy state. Physical replay is not ACK, read-cursor advancement, lazy
completion, acceptance, health evidence or native wake outlook; logical due
continuation excludes only actual logical allocations.

The process-local registry retains at most 64 live-owner worklists globally,
not per seat or binding, each at most 256 ID/full-stamp pairs plus bounded scope: 256*(128 ID bytes +16 timestamp bytes
+1 count byte +1 truncation byte), a <=128-byte execution, 16-byte incarnation
and fixed bounded seat/binding_id/generation/native-session/lease bookkeeping;
bounded tree/allocator overhead is additional. Each live request has one worklist snapshot and the existing bounded logical ledger
snapshot. The first successful capture is final for that lease even when empty;
newly arriving offers never refill it. Initialization/progress installs only on
successful store return; refusal/rollback installs neither. The exact Live lease
and canonical binding_id, generation, native session and execution fence every
pull. Grace cannot use replay authority. Replacement starts a fresh pass; ledger incarnation loss abandons remaining work without same-lease refill.
Removal, disabled/stopped registry and daemon restart discard this local state.
Successful allocation followed by response/output loss consumes that lease's
attempt; replacement can recover canonically pending content. Existing downstream
silent-loss and daemon-restart limits remain unchanged.

Recovery replaces the ordinary source slice: at most 100 exact attempts, 32
hydrated bodies, 1803 keyed probes (3 + 100×18) and 100 availability seeks.
Separately bounded reconciliation is 256 known keys/2560 probes. Recovery does
not also run the 32-item carry revalidation or a normal ordinary-page walk.
Fresh discovery433, sizing352, 32 blocks/65536 actual encoded bytes, carry155731
and partial49415 remain unchanged. No normal cursor/carry/partial
or captured scan tuple advances during replay. Missing or standalone-unrenderable
keys finish physical attempts without proving settlement or releasing capacity.
The existing nonordinary outbox must be replayed/confirmed normally before fresh
ordinary recovery; no Message descriptor enters that outbox. Full stamp and
incarnation pruning at snapshot/application cannot overwrite newer logical
history. Recovery leaves current post-store native outlook unchanged, including
empty, mixed, final and active-recovery outbox/control-only quanta.


Canonical reader operations coalesce private presentation invalidation once per
actually changed binding at their existing transaction boundary. The added
source-derived envelope per changed operation/binding is one exact BindingId
lookup statement joining two indexed canonical relations (occupant_bindings and
seats), two fixed Hook/Watch consumer point reads, and at most two guarded
consumer updates. No retained binding history or all-consumer scan is added.
AdvanceCursors coalesces all its thread changes; complete-display coalesces each
already selected target; first-contact fast-forward coalesces all gaps, including
gaps beyond its returned-description cap. A historical unbound completion that
already selects N targets adds N times this envelope; the whole operation has no
new constant bound. This source accounting is not a measured SQLite VM, elapsed
time or disk-I/O bound; the physical query controls still require execution.

The exact canonical BindingId owns native session, seat, instance, generation and
execution; presentation rows have no second native-session field. Only matching
seat/instance rows whose primary generation/execution or passive-refresh
generation/execution matches that canonical scope are invalidated. Both-stale or
foreign rows remain unchanged. The helper updates only checked private passive
generation and content version counters; it changes no captured scan tuple,
cursor, carry, partial body, token or logical offer stamp. Zero consumers and
no-change operations require no private update, including established subagent
reader use. Counter corruption, exhaustion or later operation failure rolls back
canonical reader writes and all tentative private updates together. This
maintenance adds no V2 admission, receipt provenance, ACK, invitation acceptance,
channel health or native-wake authority.

Successful ordinary-or-Lazy completion may conservatively request a passive
canonical recheck for the admitted seat's current Live capable channels. The
registry holds one ephemeral pending bit per entry under its existing global
64-channel cap; it visits only that seat's currently registered entries, never
retained presentation rows. Repeated hints coalesce. A retained successful result
may hint again and establishes no fresh receipt delta; all-refused/error outcomes
and presentation confirmation do not hint. Registry bookkeeping follows release
of the canonical writer turn. Hint consumption performs bounded canonical
delivery work and confers no retry eligibility, offer stamp, reader advancement,
receipt/acceptance evidence, extra health timestamp or idle-wake permission.
Existing successful mod ACK health accounting, including AlreadySettled, remains
separate. Replacement/restart can discard a bit; initial owner drain, ordinary
canonical eligibility and the existing safety pass retain their own contracts.

Q3 publication observation uses the message-first ordinary/Lazy point SQL above
without enlarging the 100-manifest/200-probe/three-seek source allowance. The
production constants are shared with query-plan controls so those controls can
measure the actual statements, including mixed unrelated prefixes. This source
accounting is not a measured SQLite VM, elapsed-time or disk-I/O claim; physical
work still requires execution of those controls. Addressed ordinary publication
changes only the existing private passive generation/content counters. It does
not recreate a sibling-settled receipt or change native attention. The subsequent
canonical pull decides whether content is pending or passive and whether this
binding's own cursor still needs it.

Watch presentation mode is an explicit per-channel election, advertised by
mod.watch_presentation_v2 together with the existing generic presentation and
binding-cursor capabilities. Absent/false election keeps the historical Watch
wire and aggregate view; only capable Live owners receive presentation leases.
Worker and direct-producer sends compare their captured channel id and immutable
mode under the same registry lock as final enqueue and any existing active-push
health update. A replacement cannot inherit the old owner's frame or health.
Passive sends and failed enqueue add no health. A closed receiver retains a
capable owner's recheck bit only while that exact owner remains Live, without an
immediate retry kick; existing safety passes and reconnect's initial drain retain
their recovery roles. Enqueue is not socket-flush or model-uptake evidence.
