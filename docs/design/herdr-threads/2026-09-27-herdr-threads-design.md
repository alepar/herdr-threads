> **Where things are.** The area designs, the adopted shared-contract and cooperative caller amendments and the user directions this design cites sit next to it in `docs/design/herdr-threads/`. The other run working files it names (design notes, identity verification, seat-identity references, roast reports, redesign assessments) are archived in git tag `archive/herdr-threads-run-2026-09-26` under `docs/superpowers/runs/2026-09-26-herdr-native-mailbox-thread-plugin/` (path unchanged there).

> **User-approved scope addition (2026-09-28):** [Programmatic graph identity and required system memberships](../graph-system-identity/design.md), tracked by ht-4is.29–32. This adds connection-bound service authority and explicitly accepted required invitations. It preserves the cooperative caller amendment and existing native validation gates. Design complete; implementation remains tracked separately.

# herdr-threads — root design

> **Superseding user direction (2026-09-28):** Top-level acceptance/ACK responsibility may rely on demonstrated cooperation and prompting. Service-side adversarial main-agent proof is no longer a release prerequisite. See [cooperative receipt direction](cooperative-receipt-user-direction.md). Explicit model-issued ACKs, actual both-harness delegation/restart/clear/resume and SQLite evidence remain required. Conflicting older caller-enforcement clauses below are historical pending coordinated contract update.


> **Coordinated contract adopted:** [Cooperative caller amendment](cooperative-caller-contract-adoption.md) is normative for CheckIn, claimed context and top-level acceptance/ACK fences. Independent scoped review159 is CLEAN; implementation and both-harness receipt demonstrations remain open. Earlier conflicting native main-agent proof clauses below are historical.

## Goal

Build a real Herdr plugin that gives pane-bound agent roles persistent, discoverable group threads, prelaunch invitations and handoffs, explicit per-recipient receipts, and durable deadline warnings. Agents can restart or replace conversations while their seat retains its threads; communication remains inspectable and context-efficient.

Root design produced through interactive section review. design-notes.md preserves the approvals; identity-verification.md distinguishes source evidence from remaining live checks. The user approved the reviewed top-level split in advance and directed fully autonomous continuation; the matching approval is recorded in run.md.

## Adopted shared-contract detail

The [adopted shared-contract amendment, revision 4](shared-contract-amendment-adopted.md) is normative for the exact types, schema, algorithms and ownership described below. Its adoption is a design decision, not implemented or native-tested evidence. Existing acceptance remains required. Logical publication is authoritative; bounded physical projection cannot hide committed receipt/warning obligations. F6 is resolved by the [trust policy](../../../TRUST-POLICY.md) (2026-10-01), which is normative for continuity and attribution; preserve the BOTH-harness gate `ht-910`; shared types, fake ports and store tests cannot satisfy that gate. Formal design review counters and original task review histories are unchanged by these edits.

## Product boundaries

One local Herdr instance, across its workspaces and Git worktrees. Initial harness integrations are native Codex and Claude Code. Preserve native PTYs, approvals, and the user's other hooks and sessions. No cross-machine federation, automatic work-completion inference, or plugin-run model summaries. The release target is alepar/herdr-threads with a valid herdr-plugin.toml, suitable for herdr.dev/plugins/ discovery.

A thread starts with an explicit topic/purpose/goal. Its participants update the topic as scope evolves and archive when done. Any joined seat can invite, edit the topic, archive or reopen. All control changes retain actor and timestamp; there is no special creator-admin role. When no joined seats remain, expose the thread as orphaned without changing its goal or archive state. An explicit local-user operator invitation can restore the normal invitation/acceptance path; only the target's later verified acceptance grants joined authority.

## Seat and occupant

The durable participant is the role associated with a pane. CLI processes and native conversations are replaceable occupants. A plugin-owned seat record retains Herdr mappings; friendly workspace/tab/pane labels are lookup/display metadata. Layout order is never identity. Native session reference, harness, and occupant generation record who performed each action.

An ordinary CLI restart, upgrade, resume or clear in the same pane retains membership and pending obligations. A detected occupant change creates a durable system notice for affected participants and preserves old receipts with their original provenance. The new occupant receives compact recovery metadata and pending obligations. Old acknowledged history does not gain new receipt obligations.

Actual pane closure retires the seat across threads. Commit a small durable terminal fence and frozen cutover immediately, making membership and outstanding obligations effectively retired and stopping new prompt attempts. Preserve history; bounded resumable cleanup materializes recipient-retired rows, owed warnings and per-thread audits. Expose cleanup lag rather than holding the sole writer through the backlog. Reusing a label does not resurrect it. Host unavailability, denied socket access, event loss, or an unexplained missing identity is not sufficient evidence of closure.

Herdr terminal IDs change on restore. Public pane mappings survive supported restores but change on moves, and workspace numbers can be reused across some deletion/restart histories. Keep durable seat identity distinct from these addresses. The approved exceptional recovery path preserves messages and warns when continuity cannot be established, requiring explicit operator rebind. Ordinary automatic recovery still needs validation against source and isolated live tests. Rebind is an audited mapping operation, never a receipt or resurrection of a retired seat.

Observe host targets separately from allocating plugin seats. An unambiguous explicit seat resolve or verified startup can allocate a new role; a snapshot alone does not. Across ambiguous restore, hold unclaimed targets from the recovery baseline for inspection before any startup can create competing seats. Saved addresses suggest candidates but do not prove continuity; absent reliable narrowing, hold all unclaimed baseline targets while saved roles remain unresolved. Persist that recovery decision across daemon restart. An explicit operator rebind assigns a selected unresolved role to an unowned target. `seat resolve --new-seat --operator` instead records the choice of a fresh role for that target, preserving old unresolved history. Both refuse a target owned by another live seat. A pane authoritatively created after the coherent current-incarnation baseline follows ordinary allocation. F6 is resolved by the [trust policy](../../../TRUST-POLICY.md): holds lift once nothing remains to repair, a matching resumed harness session reattaches its seat, and collisions resolve only by explicit abandonment.

Administrative mode uses kernel-supplied effective UID on the local socket, matching the daemon/state owner. The service records `operator:local-user:<uid>` and does not claim a human is at the keyboard: same-user scripts, agents and children may deliberately invoke the narrowly typed operator commands. They are rebind, explicit fresh-seat allocation, and invitation into a thread with zero joined seats. Socket access alone never grants native caller authority; operator mode cannot ACK, accept, send, advance checkpoints or impersonate a native occupant. Ordinary discovery can resolve an unclaimed empty pane before a recipient exists, with service allocation provenance.

Only a verified native top-level caller accepts invitations and ACKs for its seat. Each accountable request obtains a new current-target observation after arrival, compares the proven native root/execution evidence, and consumes a short-lived internal permit at the transaction decision. A cached binding, unexpired old context, or fresh RPC returning uncharacterized cached metadata is insufficient. Missing, stale or unavailable evidence leaves the mutation uncommitted. Subagents can discover, read and summarize, but cannot settle obligations or advance the top-level delivery checkpoint. The early two-harness attribution gate must establish the caller evidence and its actual invocation transport.

Authorization is defined at that fresh host observation, followed by a bounded decision window and checks for all known invalidations. A replacement visible before the observation, or learned before the decision, rejects the predecessor. Herdr does not offer an atomic native-session/SQLite operation: a replacement after the observation that remains unobserved can occur before durable commit. Record the proven caller, execution, observation and transaction decision; never reattribute it to a successor. Do not claim physical-commit atomicity with native replacement. Both the observation's meaning and this residual interval require explicit validation and documentation.

Receipt availability requires a successfully validated check-in for the current binding. Safe wake targeting is separate: a resolved, recognized native idle occupant may receive the compact recovery hint before registration, including after its startup check-in failed. A shell, unknown harness, unresolved seat, recognized blocked UI or known human input is ineligible for prompting. A snapshot, hint or launch cannot start receipt timers, advance the offered checkpoint, accept or ACK. The resulting ordinary hook retries registration and the agent explicitly handles its mail.

## Prelaunch flow

1. Create an empty Herdr pane and resolve its seat.
2. Create a thread or use an existing thread; persist an invitation to that seat.
3. Send an ordinary assignment message, explicitly including that invited seat among required recipients.
4. Start the agent manually or through the plugin. Both use the same startup/check-in integration.
5. The top-level occupant sees the invitation and pending message, reads it, and explicitly ACKs. It accepts separately to join future fanout.

The invitation and message exist before an agent or native transcript exists. Successful launch, prompt submission, hook output and history reads never imply receipt. An invited seat can read and ACK its addressed messages before accepting. No separate handoff-message type or claim ticket is needed.

## Membership and message transactions

Sending publishes the exact deciding-time recipient set: joined seats plus explicitly requested invited seats, excluding the author and removing duplicates. Prepare the full hidden immutable set in bounded quanta; explicit recipients must be invited or joined. Under the final short writer transaction, compare captured scalar membership/lifecycle/instance-eligibility/timeline/configuration revisions and consume fresh live-request native proof, then atomically publish message, all logical receipt obligations, unavailable-warning entries and attention through one manifest. A mismatch restarts preparation; it cannot publish a stale subset. Hidden preparation is not accepted mail, grants no authority and cannot publish after its caller disappears without a fresh authorized exact-key retry. No recipient loop runs at publication. Future joins/leaves never rewrite the published set.

Accept subscribes to future messages. Leave removes only future fanout for that thread; earlier pending obligations remain. A left seat can be reinvited. Repeated invitations while one is pending must reuse it rather than silently reset its timer. A new invitation after leaving is a new auditable membership episode.

ACK means only explicit receipt. It is not agreement, acceptance of work, or completion. Batch exact IDs. Repeated ACKs are idempotent and must not create repeated success events. Validate and commit an ACK batch atomically. An invalid member rejects the batch; an already-ACKed valid member is an idempotent success. Return an unambiguous compact result.

Archive prevents new participant discussion, keeps pending invitations/receipts and their warnings inspectable, and permits late accept/ACK and binary system events. Reopen permits new discussion. Discovery/history do not confer membership or control privileges. Leave or retirement of the last joined seat preserves the same thread as orphaned. Unavailable or unresolved joined seats still count as joined. `invite THREAD --seat SEAT --operator` checks zero joined seats inside the invitation transaction and creates/reuses an ordinary pending invitation to a nonretired seat. It does not join the seat, reopen an archive, rewrite recipients or revive a retired role. A concurrent acceptance that wins first causes `thread_not_orphaned`; otherwise normal invitation and later acceptance rules apply.

## Timing and system events

The store owns the Clock and samples UTC once inside each deciding write transaction, after queue/lock waits and input/authority validation, immediately before time-dependent transitions. This successful transaction's decision instant is the deadline linearization point. A pause before sampling counts toward lateness; a pause after it, including COMMIT/fsync delay, does not reclassify the decision. Failed or crashed commits create no accepted transition. All responses/wake effects still wait for successful commit. No client or queued request supplies trusted decision time.

Invitation acceptance deadline starts at the invitation's transaction decision and ends at explicit acceptance. Launch, binding, reads and message ACKs do not settle it. Default five minutes, configurable globally and per invitation; freeze the chosen duration at creation.

A prelaunch message obligation exists immediately, but its receipt timer starts when the eligible top-level occupant becomes available and the message is available. For an available seat, it starts at the send transaction decision; otherwise freeze the selected duration at send and start it at the first eligible-availability decision. Default five minutes, configurable globally and per message. Once started, a timer never resets due to restart, retry, leave, archive or occupant replacement.

A joined seat can become unavailable without having a pending invitation. Preserve already-running receipt deadlines. New messages still wait for an eligible occupant before their receipt timer starts, and create an actionable seat-unavailable warning for affected threads, deduplicated within that unavailability episode. No new availability timer is introduced. Repeated snapshots of the same outage do not generate warning storms.

The scheduler creates one durable warning for each missed invitation deadline or message-recipient deadline. Late acceptance/ACK remains valid and resolves the obligation without erasing the warning. No hard failure, deletion, or membership removal follows elapsed time alone. Use uniqueness constraints and transactional state transitions to resolve ACK/timeout races and restart scans without duplicate warnings. ACK, acceptance, due scans and retirement share overdue event uniqueness. ACK/accept record the unique overdue event before their terminal transition when the decision is equal to or beyond a started deadline; scans reread state under the write lock. Retirement atomically preserves the obligation to record all warnings owed at its frozen cutover in a durable cleanup job alongside the terminal seat fence. Bounded cleanup materializes each warning before that row settlement and the thread retirement audit; later cleanup time never determines lateness. Reads and authority use effective retired state immediately, and report incomplete history materialization until cleanup completes. A retirement decision before the deadline creates no warning later, and an unstarted receipt cannot be overdue. Late physical closure evidence is not used to backdate retirement. A receipt whose successful decision was timely never becomes overdue merely because commit or scanning completed later. A warning delivered after settlement includes or links to the current resolved status. Persisted UTC governs deadline classification: forward wall-clock changes can make work due, backward changes delay due eligibility, and existing warnings remain. A long post-decision commit stall can consume time before visibility; this is an explicit consequence of the decision-time contract.

Successful receipt/acceptance notices are system_notify.info: durable, discoverable at the next read/check-in, and no independent idle wake. Missed deadlines are system_notify.warn and warrant a coalesced wake when safe and their underlying condition remains actionable. Retirement-materialized warnings describe already-settled conditions, so remain discoverable history without new wake fanout. System messages require no ACK. Other lifecycle notices are durable; classify actionable changes as warnings and routine metadata as info. Coalescing affects wake hints and rendering, never stored event history.

### Logical warning visibility and bounded work

A successful store decision receives an increasing instance decision sequence in addition to its UTC decision time. Immutable membership intervals and retirement sequence freeze the warning recipient set at that decision. Ordinary overdue warning creation commits its unique event, source materialization marker and one attribution job in constant-size work. Per-recipient ledger/wake projection proceeds in crash-safe quanta of at most 16 fixed-size units and five ms admission, with exact logical recipient discovery before projection. Retirement-materialized warnings retain zero wake fanout.

Unavailable warnings are immutable prepared entries published with the send manifest. Their canonical thread/seat/unavailability-episode ID, earliest published send provenance and contiguous logical timeline positions are fixed at the same atomic publication. Counts, detail, history, search, inbox/check-in and wake reservation see them even when no materializer has run. Later projection cannot append a second event or move its history position. Checked sequence arithmetic rejects overflow without partial publication. Per-thread timeline positions and global decision sequences are distinct.

Verified check-in records a warning frontier for the current binding generation/execution only after constructing reachable bounded effective warning metadata and read-only continuation. Omitted page items remain discoverable; budget failure cannot be an empty successful offer. Accepted replacement resets the successor's frontier, preserving ordinary obligations, timer anchors and all retained wake spacing. Delayed attribution compares the event decision sequence against that current occupant's frontier.

First verified availability is an immutable per-seat decision anchor. Every published receipt immediately derives its first eligible start from the send snapshot or earliest qualifying anchor, with its frozen duration; bounded timer-index materialization never changes that start. Due phases independently retain indexed pending-unwarned cursors/errors and rotate fairly. No history/member/backlog cap or recipient subset is introduced.

## Compact discovery and history

The shared directory is a browsable index exposed through CLI/API, backed by SQLite. It is not a filesystem hierarchy. Any agent in the local instance may list/search/read without joining, including subagents. Scope/filter by membership, pending invitation and topic. Unbounded bulk history is never automatic.

Progressive disclosure:

1. A compact attention hint points to the inbox when attention is needed.
2. Check-in returns bounded thread references and counts for invitations, messages and warnings.
3. Thread metadata gives current topic/goal, age, message count, participant seats and pending receipts.
4. Read bounded recent/new message batches; paginate through older/full history on demand.
5. Request detailed receipt, occupant and delivery diagnostics only when investigating.

Every bounded collection has explicit cursor/limit/encoded-byte inputs and an exact continuation command, including directories, participants, delivery recipients and local pending intents. Use immutable ordered keys and captured high-water bounds; no OFFSET pagination. History, pending-receipt and intent traversal remain usable across appended rows and settlement/removal. Mutable directory/topic inclusion changes may return an explicit scoped `cursor_stale` with a restart command; continuous relevant churn can require restarting those filtered traversals. This is a bounded live view, not a retained cross-page snapshot.

`pending-receipts [--seat SEAT] [--thread ID]` returns bounded canonical message IDs independently of history progress, including invited, left, archived-thread and unstarted obligations. Counts link to that query. Showing a body, paging, or completing a transport write cannot satisfy a receipt. Encoded output includes framing and all continuations within its requested bound; too-small budgets fail explicitly. Body and history continuation remain separately visible when both are needed.

Literal search examines bounded indexed candidate slices with explicit no-match progress, cancellation and a separate read worker. Page size alone does not bound work. Search cannot occupy the domain writer or hook executor, and no page pins a database snapshot between requests. Topics and all peer-controlled labels/previews remain marked data in hook output; fixed plugin instructions and typed next commands never derive authority from those fields.

Send/ACK results contain compact committed IDs and status, without echoing bodies. Support stdin/file input for long messages and structured output for scripts, avoiding nested shell JSON. Summaries are optional agent work: instructions can suggest a cheap Luna/Sonnet-level subagent reading full history or the latest N messages. The plugin makes no model calls for summarization.

## Runtime components

Approved architecture: one small daemon per Herdr instance owns SQLite writes, deadlines, host reconciliation and notification scheduling. Thin CLI/API clients serve agents, hooks and the operator view. Store durable data in Herdr's plugin state directory, separate from the installed source checkout. Keep separate Herdr instances from accidentally sharing seat namespaces.

Use a Rust binary with daemon and CLI subcommands, SQLite persistence and a local IPC transport. Package with Herdr's native manifest/build/command surfaces. Select minimum versions and supported platform matrix from actual integration tests, documenting restrictions rather than claiming untested portability.

Herdr startup commands are one-shot and unsupervised. Provide idempotent start/ensure-running, health and explicit stop. Use exclusive process/database ownership, bounded client timeouts and an identifiable daemon instance; stale PID alone is insufficient to kill a process. Startup/check-in/operator commands can recover a crashed service without duplicating writers. Health reports unavailable or degraded capabilities honestly.

Events request reconciliation and conservatively invalidate permits; their payloads do not directly install identity state. One bounded observation lane serializes snapshot/current-target capture and application, rejects old boot/epoch responses, and applies a view before admitting the next. Periodic five-second complete snapshots recover silent event loss in Herdr 0.9.1. Absence can retire only from an authoritative coherent enumeration in a verified unchanged host incarnation; a partial, failed or uncharacterized view yields unresolved/degraded state. Reconnects trigger reconciliation. Persistent pending work and deadlines reconstruct after process failure.

The service serializes short database decisions. Retirement fences never enumerate the backlog; cleanup uses at most 16 fixed-size units/five ms between-unit work per transaction, with fair foreground turns and boot continuation. This bounds application work, including event and attention cost; it imposes no history or backlog cap. Host reads/prompts and bounded search run outside that writer, with finite deadlines, cancellation, bounded admission and late-result guards. Connected but hung Herdr calls release attempt reservations into retry while local health and deadline processing continue. No host or native evidence wait runs inside a write transaction.

## Delivery and errors

Commit durable state before attempting notification. Retries use a caller operation key associated with the same payload; mismatched reuse is an error. A lost response is recovered by explicit exact-key retry without duplicating messages, invitations or receipts. Once a complete IPC request may have been submitted, response loss/correlation failure is typed `unknown_outcome`; known pre-submission connection failure retains its distinct transport error. Neither triggers automatic mutation replay or pending-intent deletion. Preserve the operation key across the failure window: API callers supply a stable key; the CLI persists its generated pending operation before submission and offers recovery/retry without requiring another visible message-ID layer. An intentional repeated message uses a new operation key. A failed database commit returns failure and must not send a wake implying acceptance.

The shared startup/tool/turn check-in path handles managed and manual launches. Only verified top-level calls change delivery checkpoints. Hooks surface bounded data at supported boundaries; native Herdr prompts can wake a safely identified idle target before successful registration. Once a running daemon and supported hooks recover from a startup outage, the generic hint triggers fresh check-in and explicit model handling even when the initial prompt was lost. A permanently stopped unsupervised daemon still needs an ensure-capable invocation. Do not claim interruption during uninterrupted inference.

Coalesce per seat, persist reasons and suppress repeated wake storms. Enforce minimum wake spacing using process-local monotonic time, independently of UTC deadlines. Anchor conservatively after reservation commit and attempt completion; new attention cannot bypass the minimum or an in-flight attempt. Persist the delay/minimum and reservation history even after reasons clear. After restart, seats with prior reservations wait a full retained effective delay (new attention may shorten only to the retained/current minimum); do not infer elapsed downtime from wall time. Repeated restarts can postpone wake, while stable service for one delay restores time eligibility. Recheck target identity/state immediately before prompting; never inject into recognized approval/question UI or known human input. Host prompt success is transport submission only. Missing atomic expected-session/composer guards leaves a documented residual race; the current user-approved optimistic native-session policy does not authorize false ACK attribution.

Distinguish a committed message, attempted wake, submitted prompt, returned/read body and explicit ACK in diagnostics. Unknown host/harness state keeps obligations pending and exposes the limitation. Disk full, lock contention, corrupt state, incompatible schema, missing hook and unsupported harness errors require actionable compact results; no silent fresh empty database over damaged history.

## Reliability and operator section

Retain messages and audit history until explicit future data-management work; archive is not deletion. Avoid automatic retention expiry in the initial plugin. Supply inspectable CLI output plus a compact Herdr command/view for threads, overdue obligations and daemon health. The initial operator surface can use the same read API rather than a separate web application.

Validate deterministic state transitions with an injected clock and restart/crash fault points. Test membership/ACK/timeout races, idempotent response-loss retry, multi-recipient snapshots, every collection beyond one page, orphan recovery, operator authority, leave/archive/overdue-retire behavior and source-version compatibility failures. Independently advance UTC and monotonic clocks; pause before/after the transaction decision; delay old snapshots, hang connected host calls, and run sparse searches while receipts/check-ins proceed.

Run isolated native Codex and Claude scenarios for prelaunch handoff (including lost initial prompt), idle wake, active-turn boundary delivery, clear/resume/replacement, child reads with rejected child ACKs, and blocked UI. Verify daemon crash recovery and host-event gaps. Test restore and move identity in an isolated Herdr instance; never restart the user's shared instance. Count explicit receipts, duplicates, missing messages and notification overhead separately. Compare token overhead only under matched workloads.

The user approved this reliability/operator scope on 2026-09-27. Platform support must match actual validation. The top-level task split has the user’s recorded advance approval.

## Review and delivery workflow

After section design is settled, write and self-review the root spec, create the beads decomposition, and obtain a fresh promotion review. The user has approved the resulting top-level task split in advance. Run super-auto autonomously with design and code roasts enabled. After every nonzero roast, review the entire cumulative history and record the high-level redesign assessment before fixes. Preserve counters/caps and all unresolved qualifications.

Prepare a publishable Herdr plugin, install/release documentation and marketplace follow-on instructions. Public release/marketplace activation is follow-on to the merge-bounded implementation epic. The final integration decision remains the user's under super-auto.


## Acceptance evidence

These are checks to run, not test results. Each native test must retain the accepted message IDs, model-issued ACK calls and resulting SQLite receipt records. A terminal write or an agent saying “received” without a committed ACK is insufficient.

| Requirement | Evidence required |
|---|---|
| Real Herdr plugin | Native manifest validation and installation from a clean checkout; registered command invokes the shipped executable using Herdr runtime context and state paths. |
| Prelaunch handoff | Invite and ordinary assignment committed before agent launch; both Codex and Claude discover them at startup, explicitly ACK and accept separately. Repeat with lost initial prompt. |
| Durable seat | Native restart/new transcript/clear keeps membership; labels and tab order do not affect identity; observed pane move changes address only; actual close retires the seat. |
| Restore continuity | Isolated host restore verifies supported mapping recovery; ambiguity produces the agreed recovery behavior, never a fabricated match or receipt. |
| Top-level accountability | Parent ACK succeeds; child ACK/accept fails while child read succeeds. A predecessor is rejected when replacement is visible at the required fresh observation or known before transaction decision. Validate the observation's actual native meaning, expired permits and lost events; document the bounded unobserved post-observation race with original caller provenance. Missing/unknown evidence cannot settle obligations. |
| Group fanout | Concurrent send/join/leave has one exact deciding-time recipient snapshot through revision-validated atomic logical publication; explicit invited recipients receive the same ordinary obligation; author excluded and duplicates removed. Wide preparation yields, final publication has no recipient loop, and fresh proof is required after preparation. |
| Receipt semantics | Reads, transport writes, startup and invitation acceptance never ACK. Exact-ID batch and duplicate ACK preserve per-recipient attribution and produce one success event. |
| Five-minute deadlines | Injected-clock tests assert transaction-decision start, frozen duration, availability start, no resets, pre/post-decision waits, late ACK/accept, overdue retirement, one warning per deadline and restart/sleep reconciliation. UTC jumps and monotonic wake spacing have separate assertions. |
| Lifecycle | Leave keeps existing receipts; archive stops discussion and retains processing; retirement immediately fences effective state, preserves cutover-owed warnings through bounded crash-safe cleanup, and yields to unrelated ACK/check-in. |
| Retirement progress | Large mixed backlog and wide threads, fence/quantum crashes, per-thread warning-before-audit order, inspectable cleanup lag, no later false warning or new retirement-warning fanout, and fair eventual completion without truncation. Zero joined seats exposes orphan recovery via explicit operator invite and real target acceptance, including concurrent acceptance and archived threads. |
| Crash durability | Crash before commit, after commit/before response, before wake, after wake/before attempt record, and after ACK/before response. Each accepted obligation survives and retries resolve to its original ID. |
| Progressive disclosure | Every collection traverses beyond its hard page limit using emitted commands. Old pending IDs are directly queryable after unrelated history reads. New writes/settlements preserve stable traversal; relevant mutable filters restart explicitly. Encoded byte bounds retain both body and page continuations. Sparse/no-match search cannot block receipt work. |
| System notifications | Info events persist without independent wake; warnings cause bounded coalesced wake attempts. Restart does not duplicate warnings. No system event creates a receipt obligation. |
| Native delivery | Both harnesses demonstrate idle wake, active-turn hints, and idle recovery after startup check-in fails with no initial prompt. Registration/timers/checkpoints/ACK remain distinct. Blocked UI is untouched; hostile topics remain data. No uninterrupted-inference claim. |
| Reconciliation | Missing events/reconnect recover from ordered authoritative observations; delayed snapshots cannot retire new terminals or roll back generations. Errors/unknown views cannot retire. Restore holds prevent allocation from preempting operator repair; explicit fresh-role choice is audited. Hung host calls are finite while deadlines continue. |
| Single daemon | Concurrent ensure-running starts converge on one owner; crash recovery rebuilds pending work; stale PID cannot stop an unrelated process. |
| Operator inspection | List topic/stats/participants and overdue receipts, inspect original actor/session provenance, daemon health and unresolved identity/delivery state. |
| Packaging and finish | Publishable repository layout, documented installation/build and support matrix, marketplace prerequisites, passing checks at reviewed SHA, cumulative roast histories and final integration hand-back. |

## Implementation boundaries

Keep the persistent domain, Herdr identity adapter, scheduler, daemon lifecycle, agent interface and harness adapters separately testable. A small compiled contract establishes the request/result shapes and component ports before parallel implementation. The application composition owns the wiring between these components; the native plugin package owns installation and operator commands. Every boundary has one owner in the task tree.

Use SQLite transactions for domain operations. Enable foreign keys and choose/document WAL, synchronization and busy handling through the same connection factory used in production tests. Keep immutable message bodies, recipient snapshots, membership episodes, occupant provenance, idempotent operation records and system events recoverable together. Do not introduce an external broker.

Identity feasibility is an early prerequisite for accountable operations. The native parent/child check must establish usable caller evidence for both installed harnesses; a missing distinction is a design defect to resolve before enabling ACK/accept, not permission to ship an environment-only fallback. Source-format adapters must reject unknown shapes and carry compatibility fixtures. The shared local-user trust model does not claim isolation against a malicious process that can edit the plugin database or harness transcripts.

A wire request can carry an operation identifier for retries, but clients must never assert their own verified top-level status. The service validates the tested harness evidence against a fresh native target observation and consumes its internal permit at the transaction decision. Generation, boot/epoch, expiry and known-change fences are checked there. The store owns decision time and shared overdue transitions. Operator actions are a separate whitelist using kernel peer identity, never a client assertion or a native receipt. Shared contracts distinguish observations, registration, mutation permits, administrative intent, UTC/monotonic time and bounded page/work results.

## Design roast 1 revision

The complete roast and cumulative assessment required coherent changes across observation/recovery, transaction time and bounded public operations. Thirteen tracked design fixes cover confirmed findings. F6 remains an escalation pending fresh review of explicit restore holds and allocation; F1/F5/F16 retain their rejection reasons. The next roast uses the full revised tree and regression lens; no runtime support is inferred from these edits.

## Alternatives considered

- Conversation-bound membership and one-use claim tickets were replaced by the user's pane-bound seat model. Native conversation identity remains receipt provenance. This supports CLI restarts, cleared contexts and prelaunch invites.
- Direct multiwriter SQLite clients would require every hook and CLI to implement concurrency, identity and warning rules. The approved daemon centralizes those transitions and keeps clients small.
- Hooks alone cannot wake an indefinitely idle agent. The approved approach combines compact boundary check-ins with native idle prompts and durable recovery.
- Automatic summaries would add model cost and policy. Topic/stats and paginated history supply deterministic recovery; cheap summaries are optional agent delegation.

## Sources and evidence limits

The original assignment and required research are preserved in the development archive (tag `archive/herdr-threads-run-2026-09-26`, `HANDOFF.md`). All six research package reports and both PoC reports were read; their findings inform this design but do not establish the new plugin's behavior. seat-identity-references.md covers Wheelhouse's persistent seats and the inspected Gas Town precedents. The broader mailbox survey is not a complete architectural audit of Wheelhouse.

The installed baseline is Herdr 0.9.1/protocol 22, Codex 0.157.1, Claude Code 2.1.283. identity-verification.md links pinned Herdr source and official hook contracts. The previous experiments demonstrated bounded native idle/tool-boundary delivery; neither universal interruption nor crash durability of this plugin is proven. Their unequal workloads do not support a controlled token-savings percentage.

The [Herdr marketplace](https://herdr.dev/plugins/) discovers public repositories carrying the herdr-plugin topic and a parseable herdr-plugin.toml on refresh. Prepare those artifacts and instructions. Publishing/marketplace activation is follow-on work; this super-auto epic ends at merge readiness and the human integration decision.

## Shared-contract adoption record

2026-09-27: Reconciled this canonical spec with the adopted revision-4 shared contract. Detailed normative algorithms/types and preserved revision responses are in the [adopted shared-contract amendment, revision 4](shared-contract-amendment-adopted.md). This is specification work before the next formal design review; no source implementation or native-support completion is asserted.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
