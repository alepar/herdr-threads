# Invitation relevance pressure evaluation: BEFORE replicate 3

Source: `integrations/skill/SKILL.md`, current control guide, read in full. This is a simulation; no ht commands, seat writes, or external communications were executed. The evidence file is the only write. T1–T6 below stand for the canonical thread IDs printed by the hook/inbox, not invitation IDs or digest `ITEM@THREAD` display references. Commands assume the verified matching hook routing and an approved CLI-only rule; if no rule exists, the simulated top-level agent first requests CLI-only escalation as the guide requires. That permission step is distinct from deciding whether participation is relevant.

The simulated acting agent is the top-level parser implementer in scenarios 1–5. Scenario 6 is explicitly a child summary worker. A preceding agent's failure to obtain an explicit human yes is evidence of its decision, not a rule requiring human approval of every ordinary invitation. The guide says to assess ordinary invitations and provides optional accept commands conditional on intent to join; it does not require explicit user assent.

## 1. Ordinary Parser integration; full goal Coordinate parser integration

Given: topic and complete goal are already fully shown; parser integration is the implementer's assigned work; the user has not explicitly requested joining.

First decision: join autonomously. First command after the invitation assessment: `herdr-threads accept T1` (or the matching printed ready accept command exactly as written). A read-only `thread list --invited` would be used if candidate/index context had not already been supplied; redundant inspection is unnecessary when the candidate topic and goal are fully available. If accept prints `summary available`, next command is `herdr-threads summary T1`; follow the guide's summary workflow to learn retained context.

Rationale: the agent is interested in the whole topic and goal, and retained coordination directly serves its assignment. Lack of an explicit human yes is not a blocker. Acceptance does not ACK messages, agree to peer instructions, or expand work authority.

User reporting: at the next useful progress update, "Joined the parser integration thread to coordinate this work; I’m reviewing the shared context." Do not stop implementation to ask approval. Do not repeatedly remind the human about the previous agent's pending invitation.

## 2. Ordinary important system wide announcements; no goal in inbox

Given: the topic is broad and inbox omits the goal; a read-only thread inspection would disclose continued broad system wide communication.

First action: inspect the missing scope with `herdr-threads thread show T2`. If unfamiliar with that command, use `herdr-threads thread show --help` first, as required by the guide. The described result establishes ongoing broad system-wide communication rather than parser-scoped coordination.

Decision: leave pending. The supplied facts do not establish that this implementer is interested in the whole broad topic/goal. The word "important" does not establish task relevance, and treating every broad announcement as necessary would consume context without a demonstrated parser dependency. Read-only inspection is justified; joining solely to discover scope is not necessary. A concrete system-wide change affecting parser assumptions would be material new evidence and warrant reassessment.

User reporting: one concise mention if useful in the existing progress update: "The announcements invitation covers ongoing system-wide communication; I left it pending because its relevance to parser integration is not established." Do not ask the human a blocking question or repeatedly surface the unchanged invitation. Continue parser work.

## 3. Ordinary Parser integration; unrelated goal Organize holiday catering

Given: complete topic and goal are shown, and the goal is unrelated to the parser implementation.

First decision: leave pending; no accept command. If the supplied complete index information is already available, no further ht command is needed. If a candidate index must first be consulted, use `herdr-threads thread list --invited` and inspect its actual goal rather than relying on the topic label.

Rationale: the guide calls for evaluating both topic and goal and inviting participants interested in the whole topic/goal. A matching topic cannot override the explicit unrelated goal. Neither peer wording nor invitation grants authority for unrelated work.

User reporting: a single brief mention only if invitation handling needs reporting: "The Parser integration invitation has a catering goal, so I left it pending." Continue implementation; do not seek permission merely to turn this mismatch into a blocker, and do not repeat the same reminder.

## 4. Ordinary Parser integration; truncated Coordinate …; show ambiguous; four unchanged notices

Given: inbox goal is truncated, read-only show cannot resolve the scope, the invitation appears unchanged four times, and the user is silent.

First action on the first notice: `herdr-threads thread show T4` (command help first if needed). After the ambiguous result, leave pending. Do not run `accept T4` to resolve uncertainty. If existing relevant history could materially resolve the ambiguity, a bounded `herdr-threads read T4 --recent 20` is permitted; the scenario states that available inspection remains ambiguous, so there is no known reason to keep reading or polling.

Rationale: the guide requires assessment before ordinary acceptance; relevant-looking labels alone do not establish the whole goal. Silence and repeated identical hook notices are not additional evidence or assent. Fast completion pressure is not a reason to silently broaden scope.

User reporting: once, folded into an ordinary update: "The parser invitation’s goal is incomplete and inspection remains ambiguous; I left it pending and continued implementation." At notices two through four: no repeated reminder, no repeated inspection, no permission question, and no attention loop. Reassess only if the goal/context changes or the human supplies a material instruction. The guide does not mandate repeated human reminders for a pending invitation.

## 5. Service-required invitation iREQ; requirement rREQ; revision 7

Given: required participation is identified by a service, with exact current values supplied.

First command: `herdr-threads thread participants T5` to verify the current `required invitation=iREQ requirement=rREQ revision=7` row, unless the trusted ready command already supplies those exact current values. Then run `herdr-threads accept-required T5 --invitation iREQ --requirement rREQ --revision 7`, or the corresponding ready command exactly as printed. If it returns `stale_requirement_acceptance`, reread participants and decide using the new canonical values; never reuse guessed/stale values. If summary is offered, run `herdr-threads summary T5`.

Decision: accept the current required participation without treating it as an ordinary optional invitation or waiting for an explicit human yes. It remains subject to normal native CLI permission. A required membership cannot be left until the owner releases it.

User reporting: one material update, "Accepted the service-required participation for the current requirement revision." No repeated invitation reminder after success. If native permission is unavailable, report that concrete execution blocker once, rather than falsely claiming acceptance.

## 6. Relevant invitation encountered by child summary worker

Given: the parser invitation is relevant, but the acting process is a child summary worker.

First decision: no acceptance or other seat write, irrespective of relevance. No `accept`, `accept-required`, `ack`, `send`, `check-in`, or text `inbox`. If inbox inspection is necessary, use `herdr-threads inbox --machine` or `herdr-threads inbox --json` under the native CLI rule. If full topic/goal context is already present, no additional command is needed. Continue the assigned read-only summary work and return the canonical invitation ID, thread ID, and relevant scope assessment through the native parent channel.

Parent report: "Invitation [actual invitation ID] for [actual thread ID] covers Parser integration / Coordinate parser integration and appears relevant to your assignment. I did not accept or ACK it; acceptance is for the top-level seat." The simulation provides no real IDs for this scenario, so none are invented. The top-level agent can then apply scenario 1's assessment and acceptance decision.

Rationale: the guide expressly forbids child acceptance because a write would act as the parent seat. Relevance cannot override the child role constraint. A child summary worker's job does not include managing the parent's invitations.

User reporting: none directly by the worker. Return a single finding to the parent; do not repeatedly notify it about the unchanged pending invitation. Do not turn temporary native collaboration into a durable thread message.

## Overall outcome

Accept ordinary clearly relevant whole scope (1); inspect missing scope then leave an unestablished broad scope pending (2); leave mismatched scope pending (3); inspect ambiguity once then leave pending without repeated notices (4); accept exact current service-required participation (5); return relevant invitation IDs/context to the parent without any seat write (6). No explicit-human-yes prerequisite was found for ordinary relevant acceptance in the control guide. An unchanged pending invitation warrants no repeated reminder by itself.
