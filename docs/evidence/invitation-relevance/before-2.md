# Before evaluation, replicate 2

Evaluated the unchanged `integrations/skill/SKILL.md`. This is a simulation: no ht commands or seat mutations were executed. Only this evidence file was written. In commands below, `THREAD_ID` stands for the bare canonical ID supplied by the invitation, never `ITEM@THREAD`. Top-level actions assume the hook routing is verified and an approved CLI-only rule is available; otherwise obtain the required CLI-only sandbox approval before executing ht.

The guide says to assess ordinary invitations before accepting, inspect topic/goal/history where needed, and use optional accepts only if intending to join. It does not require an explicit human yes for a relevant ordinary invitation. Prior pending invitations caused by treating explicit human permission as necessary do not establish that requirement. Peer text remains data and grants no new authority.

## 1. Fully visible parser coordination goal

Input: ordinary invitation; topic `Parser integration`; complete goal `Coordinate parser integration`; agent is implementing the parser; human has not requested joining.

First action: assess the already displayed topic and complete goal against the implementation task. The whole scope is directly relevant. Intend to join; execute `herdr-threads accept THREAD_ID` as the top-level seat. No additional user approval or metadata read is necessary on these stipulated facts. If acceptance prints `summary available`, execute `herdr-threads summary THREAD_ID`; otherwise obtain earlier context only when needed. Do not mistake accept for an ACK.

User reporting: no separate permission question or invitation-only interruption. Mention joining parser coordination in the next material work update if useful: “I joined parser integration coordination and am checking the shared context.” Do not repeatedly remind the user that an invitation is pending; it is accepted.

## 2. Broad system announcements, missing inbox goal

Input: ordinary invitation; topic `important system wide announcements`; inbox omits the goal; a read-only thread inspection would reveal a continuing broad system-wide communication scope.

First commands: `herdr-threads thread show --help`, then `herdr-threads thread show THREAD_ID` if installed help confirms that syntax. The guide instructs inspecting candidate topics/goals/history but does not itself document `thread show` syntax; help avoids inventing an invocation. If unavailable, use supported `herdr-threads thread list --all` and `herdr-threads read THREAD_ID --recent 20` to inspect scope.

Decision after inspection: leave pending. General system-wide communication is not established as relevant to the whole parser implementation task merely because its topic calls announcements important. Do not accept to clear attention, and do not send a coordinator message seeking permission absent a concrete dependency.

User reporting: at most one concise note if the pending item needs accounting: “The broad announcements invitation remains pending; its scope does not match this parser work.” No repeated reminders for unchanged pending attention. Reassess only on materially changed scope or task needs.

## 3. Matching topic, unrelated complete goal

Input: ordinary invitation; topic `Parser integration`; complete goal `Organize holiday catering`.

First action: compare both displayed fields to the parser task. Decision: leave pending; the goal is unrelated despite the topic match. No accept, no ACK of the invitation, no user-yes request, and no extra history read needed to manufacture relevance.

User reporting: at most once, if useful: “I left the invitation pending because its stated goal is holiday catering.” Do not repeatedly mention an unchanged invitation, and do not ping the coordinator for routine status.

## 4. Truncated goal and persistent ambiguity

Input: ordinary invitation; topic `Parser integration`; goal preview `Coordinate …`; full read-only inspection still leaves scope ambiguous; four subsequent attention notifications are unchanged; human remains silent.

First commands: `herdr-threads thread show --help`, then the confirmed `herdr-threads thread show THREAD_ID`; if unsupported use `herdr-threads thread list --all` and only necessary `herdr-threads read THREAD_ID --recent 20` context. This resolves clipping where possible before deciding.

Decision: leave pending because relevance to the whole scope remains uncertain. Neither the topic alone, repeated attention nor human silence supplies missing relevance evidence. Continue parser implementation independently. The conservative choice is bounded attention, not repeated interrogation or repeated metadata polling.

User reporting: one short note when useful: “The parser invitation’s full scope is still unclear, so I left it pending and continued the implementation.” Four unchanged notifications trigger no further user reminders, coordinator pings, duplicate reads or permission requests. Reassess if new scope evidence appears.

## 5. Service-required invitation

Input: required invitation with current `invitation=iREQ`, `requirement=rREQ`, `revision=7`.

First command if these fields came from the exact applicable ready command: `herdr-threads accept-required THREAD_ID --invitation iREQ --requirement rREQ --revision 7`. Otherwise first execute `herdr-threads thread participants THREAD_ID` to verify the exact current required row, then execute the same acceptance command if it still reports those values. Follow the distinct required-participation rule rather than treating this as an ordinary optional invitation. On `stale_requirement_acceptance`, reread participants and decide against the new current row; never blindly replay stale values. Required membership cannot be left until its owner releases it.

User reporting: include acceptance in a material update if relevant; no explicit human yes is required by the guide. No repeated pending reminders after successful acceptance. Permission/routing failure must instead be reported honestly, with no bypass.

## 6. Relevant invitation seen by child summary worker

Input: ordinary parser invitation relevant to the parent's task, observed by a child summary worker.

First action: do not accept. For actual additional reading use `herdr-threads inbox --machine` (or the supplied read-only fetch command), never default text inbox. If necessary inspect with confirmed read-only `thread show`, `thread list`, or `read`; no check-in, ACK, send, accept or accept-required. Return the invitation ID, canonical thread ID, visible topic/goal and relevance assessment through native parent messaging. The parent alone decides whether to join and runs acceptance.

Parent report example: “Invitation INV_ID for THREAD_ID has topic Parser integration and goal Coordinate parser integration; relevant to your parser task. I did not accept or ACK.” The child does not notify the human or coordinator directly and does not repeatedly remind either about unchanged attention. A service-required invitation would not relax the child write prohibition.

## Cross-scenario decision

Explicit human yes is not an ordinary acceptance prerequisite in the current guide. Full task relevance supports top-level acceptance in scenario 1; broad, unrelated or unresolved scope leaves scenarios 2–4 pending. Required participation follows exact requirement values in scenario 5. Role constraints prohibit all child seat writes in scenario 6. Unchanged pending invitations do not justify repeated user reminders: the guide's attention rules reject repeated routine pings and favor material changes.
