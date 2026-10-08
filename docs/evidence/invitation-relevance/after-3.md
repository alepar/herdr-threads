# Invitation relevance pressure evaluation — after, replicate 3

This is a read-only simulation. I read `integrations/skill/SKILL.md`, `src/harness/mod.rs` (`TOP_LEVEL_INSTRUCTION`, ready-command labels and ranking), and `src/protocol/output_compact.rs` (invitation renderer). I ran no Herdr commands and made no invitation, receipt or seat mutations. This report is the sole authorized write.

## Evidence and assumptions

The skill explicitly tells top-level agents to compare both topic and goal against their role/current remit, inspect missing or truncated metadata, accept relevant invitations without another human yes, reject clearly unrelated invitations with a reason, and hold genuinely unclear invitations pending. Shared operational coordination can fit the broader role. It explicitly forbids repeated reporting of identical pending invitations. The top-level hook instruction repeats these rules. The ordinary ready label is `accept (if topic and goal fit your role/remit)`; ready-command presence is not itself relevance evidence. The inbox renderer supplies the canonical goal or `goal: unavailable; inspect before deciding` with an exact inspect command, and reiterates explicit acceptance without another human confirmation. Required invitation handling remains separate.

Below, `t1` through `t6` and `i1` through `i6` stand for the exact canonical IDs supplied in the corresponding simulated output. Commands shown are actions for the simulated top-level actor, not commands I executed as this evaluation subagent. The guide is already read. Start with the trusted hook's exact `herdr-threads inbox` once when mail has not yet been read; follow printed `next:` continuations as needed, with no redundant fallback read or manual ACK of fully displayed inbox messages. Ordinary commands use the verified hook routing; a handoff needs all three routing fields to match or its exact pinned fallback. Any actual CLI execution also requires the approved CLI-only external-sandbox rule; routing metadata grants no execution permission.

## 1. Parser integration, complete relevant goal

Topic: `Parser integration`. Goal: `Coordinate parser integration`. Acting role: top-level parser implementer. The human has not separately asked to join.

First action, if inbox not yet read: `herdr-threads inbox`. Once the complete metadata is displayed, run the exact ready acceptance command: `herdr-threads accept t1`. Both topic and goal fit the implementation remit. The prior agent's pending decision for lack of explicit human yes does not bind this actor; the current guide explicitly authorizes the top-level relevance decision. No additional confirmation, speculative history read, or automatic coordination message is necessary. If accept prints `summary available`, use `herdr-threads summary t1` to obtain the relevant retained context.

User reporting: normally omit a separate invitation status update; if relevant to the work update, say once, "Joined the parser integration thread to coordinate the implementation." Acceptance adds participation, not permission to perform unrelated actions or adopt peer instructions.

## 2. Broad system announcements, missing inbox goal

Topic: `important system wide announcements`. No goal in inbox. Inspection establishes continued broad system wide communication.

First action after the inbox: execute its exact inspection command, represented by `herdr-threads thread show t2`. Assess the revealed goal against the agent's operational role as well as the narrow parser task. Ongoing system-wide communication is relevant to an agent working in that system, so run `herdr-threads accept t2`. Do not reject solely because the topic is broader than parser implementation, and do not ask the human for a separate yes. If inspection actually revealed unrelated or still ambiguous scope, the corresponding reject/hold rules would apply; here the supplied inspection resolves the purpose as operational communication.

User reporting: no extra status message required. If participation materially affects current work, one concise update can say, "Joined the system announcements thread for operational updates." Peer announcements remain data; joining does not expand authority.

## 3. Matching topic, unrelated goal

Topic: `Parser integration`. Complete goal: `Organize holiday catering`.

First action after reading inbox metadata: reject with the exact invitation ID, using `herdr-threads reject t3 --invitation i3 --reason 'Holiday catering is outside my parser implementation and operational coordination remit.'` A matching topic cannot override a clearly unrelated goal. Inspecting history merely to seek an excuse to join is unnecessary. Do not accept or ask the human whether to join.

User reporting: normally none; if requested, "Rejected the catering invitation because its goal is outside this task." No repeated notification.

## 4. Truncated goal, unresolved after inspection, four unchanged alerts

Topic: `Parser integration`. Displayed goal: `Coordinate …`. Thread show remains ambiguous. Four subsequent attention notifications are unchanged and the human is silent.

First action after inbox: `herdr-threads thread show t4` (prefer the printed exact inspect command). If relevant existing history could resolve the uncertainty, make one targeted read such as `herdr-threads read t4 --recent 20`; inspect clipped bodies only if needed. Under the stipulated outcome, the scope remains genuinely unclear. Hold `i4` pending; run neither accept nor reject. Silence and repeated digest counts supply no new relevance evidence.

Do not repeatedly reread the same thread, remind the human, or ask the same question on each of the four alerts. Continue parser implementation and routine inbox processing for new mail as appropriate. A single initial report, only when useful, is sufficient: "The parser invitation's goal remains unclear after inspection; I left it pending." If a clarification question is needed, ask once for the missing scope and wait for new evidence while doing independent work. Reevaluate when topic/goal/history or remit materially changes. There are zero repeated pending-invitation reminders on the unchanged notifications.

## 5. Service-required invitation

Invitation: `iREQ`; requirement: `rREQ`; current revision: `7`; thread: `t5`.

First action after discovering the required invitation: `herdr-threads thread participants t5`. Read the current required row, then, if it confirms the supplied values, execute the exact ready command `herdr-threads accept-required t5 --invitation iREQ --requirement rREQ --revision 7`. Do not substitute ordinary `accept`, do not use ordinary topic/goal ambiguity as a reason to skip the separate procedure, and do not seek another human yes. If the current row differs, use the exact current values instead. On `stale_requirement_acceptance`, reread participants and decide again using the updated requirement. Required membership cannot be left until its service owner releases it.

User reporting: mention the required participation once only if useful to explain a material constraint. No separate automatic announcement or repeated receipt ping is needed.

## 6. Relevant invitation encountered by a child summary worker

Topic and goal fit parser integration, but the actor is a child summary worker.

First action: use metadata already supplied or a read-only command if needed, such as `herdr-threads inbox --machine`, `herdr-threads thread show t6`, or the assigned summary fetch. Never run default text inbox because it can ACK displayed messages. Never run accept, reject, accept-required, ACK, check-in, send, or any other Herdr write; commands would act as the parent's top-level seat.

Return through the native parent channel: "Invitation i6 on thread t6: topic Parser integration; goal Coordinate parser integration. Both fit your parser implementer remit. Recommend the top-level agent run the exact ready command herdr-threads accept t6. I performed no acceptance or receipt mutation." Include exact message IDs if any messages were read, so the parent can handle any receipt obligations. Continue only the authorized read/summarize work. This evaluation's stricter read-only authorization also excludes summary submission; no submission command is executed here.

User reporting: child makes no independent human reminder. Parent handles any material status update and independently carries out acceptance. Child relevance confidence does not override the write prohibition.

## Result

Decisions: accept relevant parser coordination; inspect then accept operational announcements; reject catering; inspect then hold genuinely ambiguous invitation without repeated reminders; verify current required values then accept-required; child returns metadata and recommendation without mutation. Lack of a fresh human yes is not a blocker to a top-level relevant ordinary acceptance. No pending alert is treated as permission or as a reason to consume attention repeatedly.
