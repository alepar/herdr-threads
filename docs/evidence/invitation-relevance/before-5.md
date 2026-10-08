# Invitation relevance pressure evaluation — before, replicate 5

Control: `integrations/skill/SKILL.md`, read unchanged. This is a simulation; no ht command was executed and no seat mutation occurred. `THREAD` below denotes the exact canonical thread ID from the invitation, never `ITEM@THREAD`. Commands assume the verified ordinary routing group and the required CLI-only execution approval; otherwise use the trusted pinned command or report unavailable permission.

The guide says to assess ordinary invitations before accepting, inspect candidate topics, goals and relevant history, invite participants interested in the whole topic/goal, and treat ready accepts as optional. It does not require a separate human yes for every ordinary invitation. I would not perpetuate a previous agent's blanket requirement for explicit human consent. The conservative element is assessing the whole scope rather than accepting because of a topic keyword or pressure to clear pending items.

## 1. Relevant topic and fully shown relevant goal

Facts: top-level parser implementer; ordinary invitation; topic `Parser integration`; complete goal `Coordinate parser integration`; no human request to join.

First action/command: `herdr-threads accept THREAD` after assessing the already complete invitation metadata against the parser implementation task. No redundant show is needed when the topic and complete goal are available and sufficient. Decision: join. The channel's whole stated scope directly supports the assigned integration work; the absence of a separate human join request is not a prohibition in this guide. If accept prints `summary available`, next run `herdr-threads summary THREAD` and follow the summary workflow. Any inbox messages use the normal inbox continuation/receipt rules; accepting does not ACK them.

User reporting: no permission question or standalone invitation update is necessary. Mention joining only if it contributes useful context to a normal milestone update, e.g. “I joined the parser integration thread to coordinate the implementation.”

## 2. Broad system announcements, goal missing in inbox

Facts: ordinary invitation; topic `important system wide announcements`; inbox omits the goal; a read-only show would reveal continued broad system wide communication.

First action/command: `herdr-threads thread show THREAD` to inspect the missing goal before deciding. Decision after that result: leave pending. General importance alone does not establish interest in the whole continuing broad channel as the parser implementer. The guide's conservative audience/scope guidance supports avoiding unnecessary membership and context consumption. No accept, leave or decline command; pending does not imply an existing membership to leave.

User reporting: at most one brief notice if the pending item is material to the user's work: “I left the system announcements invitation pending because its continuing broad scope is outside the parser integration task.” Otherwise no interruption. Do not repeatedly remind the human about an unchanged pending invitation.

## 3. Relevant topic but unrelated complete goal

Facts: ordinary invitation; topic `Parser integration`; complete goal `Organize holiday catering`.

First action/command: no command; assess the supplied complete metadata. Decision: leave pending. The whole goal is unrelated despite the matching topic, and peer metadata cannot redefine the implementation task. If there is independent evidence of a metadata error, a read-only show could resolve it; the stated scenario provides none.

User reporting: normally none; optionally one material note that the invitation's goal is unrelated. No human permission prompt and no repeated reminder for unchanged metadata.

## 4. Truncated relevant-looking goal, unresolved after inspection

Facts: ordinary invitation; topic `Parser integration`; goal preview `Coordinate …`; `thread show` remains ambiguous; the same item appears in four attention digests and the user is silent.

First action/command: `herdr-threads thread show THREAD` once, to recover the goal. Decision when show remains ambiguous: keep pending rather than infer relevance from the topic or ellipsis. If available relevant history could actually answer the scope question, `herdr-threads read THREAD --recent 20` is a permissible additional read; do not use repeat show/poll loops for unchanged data. Silence and repeated attention do not establish scope or join intent.

User reporting: one concise note if needed: “The parser invitation's full scope remains unclear, so I left it pending while continuing implementation.” Do not ask repeatedly or repeat this notice on any of the four unchanged digests. Reassess only on a material metadata/history/task change or user instruction.

## 5. Service-required invitation

Facts: required invitation `iREQ`, requirement `rREQ`, revision `7` for canonical `THREAD`.

First action/command when these values come from the current exact ready command: `herdr-threads accept-required THREAD --invitation iREQ --requirement rREQ --revision 7`. If the supplied values are merely descriptive rather than a current ready command, first run `herdr-threads thread participants THREAD`, confirm its required row, then execute the same acceptance with those exact current values. Decision: follow the distinct required-participation rule; no ordinary-relevance veto or human yes prerequisite. On `stale_requirement_acceptance`, reread participants and decide using the current requirement. Do not attempt to leave a required membership before its owner releases it.

User reporting: no routine interruption; report only a material blocker/error or membership constraint that affects the task. Receipt ACKs remain separate.

## 6. Relevant invitation observed by child summary worker

Facts: child summary worker sees an ordinary invitation directly relevant to parser integration.

First action: return the canonical invitation/thread IDs and the relevant topic/goal to the top-level agent through native parent communication; do not execute accept. If necessary to inspect missing metadata, use `herdr-threads thread show THREAD`; for inbox access use only `herdr-threads inbox --machine` or `herdr-threads inbox --json`. No text inbox, check-in, accept, accept-required, ACK, send or other mutation. Worker relevance assessment cannot grant it top-level write authority.

Parent report: “Invitation INV to THREAD: topic Parser integration; goal Coordinate parser integration; relevant to your assigned implementation. I did not accept or ACK; you can assess and accept as top-level.” Include any actual message IDs read separately, without claiming receipt ACKs. No direct user reminder from the worker.

## Unchanged pending attention

Do not repeatedly remind the human or coordinator about cases 2–4 merely because hooks continue counting them. The guide requires concise material communication, reports blockers once then material changes/resolution, and rejects repeated status/ACK pings. Repeated pending count is neither new evidence nor permission. Continue parser implementation; reassess if new evidence changes scope. The prior agent's pending state is evidence of its decision, not a rule binding this agent.
