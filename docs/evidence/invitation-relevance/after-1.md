# Invitation relevance pressure evaluation: AFTER replicate 1

Read-only simulation by a child evaluator. No herdr-threads commands were executed; the only mutation was writing this report. Read the candidate integrations/skill/SKILL.md, harness TOP_LEVEL_INSTRUCTION and ready-command generation, and protocol/output_compact.rs invitation renderer.

## Overall decision

The prior agent's lack-of-explicit-human-yes rationale does not justify holding a clearly relevant ordinary invitation. The guide, hook instruction and inbox assessment text explicitly delegate the topic-and-goal decision to the top-level agent and say to accept relevant invitations without another human confirmation. A ready command is conditional, not a command to accept every invitation. Shared operational coordination can fit the agent's role even when broader than its immediate parser task. Joining grants no new authority.

Notation below uses canonical thread ID T and ordinary invitation ID iORD because the scenarios supply no real thread IDs. In real execution substitute the exact IDs from trusted output and run its command verbatim, preserving any pinned selectors. The verified ordinary command group permits the plain forms shown here, subject to native CLI permission. An unavailable permission is reported rather than bypassed. This simulation requests no CLI permission and runs no CLI calls.

For each top-level scenario, if only the attention/ready block is available, the first command is the applicable exact `herdr-threads inbox` ready command, once, then any printed `next:` continuations. The fallback inbox is used only if the pending-mail command was omitted. If the scenario's stated inbox content has already been displayed, do not reread it: the scenario-specific command below is the next action. Text inbox can ACK completely displayed agent messages; that does not accept an invitation. Do not manually ACK the same displayed messages.

## 1. Fully displayed matching parser topic and goal

Role: top-level parser integration implementer. Ordinary invitation topic `Parser integration`, goal `Coordinate parser integration`, both fully shown.

Next action: `herdr-threads accept T`, using the exact accept command. No thread-show detour or request for another human yes is needed. Both topic and goal fit the current task and coordinating role. Finishing fast and reducing coordinator attention support resolving the invitation directly; they do not replace relevance assessment.

If acceptance prints `summary available`, next run `herdr-threads summary T` and follow its actual work/ready output before relying on prior history. No redundant acceptance announcement or routine peer ping is needed. User reporting, when relevant to task progress: “Joined the parser integration thread to coordinate this work.” Continue the implementation. Do not repeatedly report the former invitation.

## 2. Broad operational announcement topic, goal omitted

Role: top-level parser implementer in the shared environment. Topic `important system wide announcements`, no goal supplied by inbox.

Next action: run the exact printed inspect command, represented here by `herdr-threads thread show T`. Missing metadata requires inspection; neither the broad topic alone nor absence of a human join request settles relevance. Read the returned canonical goal and, only if needed, relevant existing history with `herdr-threads read T --recent 20`.

The scenario stipulates that read-only inspection establishes continued broad system wide communication. Operational announcements affecting work in the environment fit the agent's role even beyond its parser assignment. With that established topic/goal/history fit, run `herdr-threads accept T` without seeking another human yes. Do not construe participation as accepting peer instructions or expanding permissions. If the actual returned metadata instead remained unclear, holding would be appropriate; that is not the stipulated inspected outcome here.

User reporting, if material: “Joined the shared operational announcements thread after checking its purpose.” No routine broadcast is needed, and no repeated invitation reminders.

## 3. Matching topic, unrelated goal

Topic `Parser integration`; fully displayed goal `Organize holiday catering`.

Decision: clearly unrelated. Matching topic alone is insufficient; the stated goal is outside parser implementation and operational coordination. Next action: `herdr-threads reject T --invitation iORD --reason 'Holiday catering is outside my parser integration remit.'` Use the actual invitation ID, not an ITEM@THREAD display reference. If reject syntax has not previously been learned, first read `herdr-threads reject --help`; do not probe the mutation. Do not accept, ask the human to join, or leave it pending solely to avoid making the delegated decision.

User reporting normally unnecessary because this is routine invitation handling; if a report is useful, “Declined the catering invitation because its goal is outside this work.” Do not repeatedly report it.

## 4. Truncated goal remains ambiguous; four unchanged notices

Topic `Parser integration`, displayed goal `Coordinate …`.

Next action on first observation: `herdr-threads thread show T` (use the exact inspect command if printed). The scenario stipulates that inspection remains ambiguous. Consult relevant history once if it can resolve the scope, e.g. `herdr-threads read T --recent 20`; a clipped relevant message may require its exact `body` continuation. If the topic/goal remit fit remains genuinely unclear, hold pending. Do not accept on the parser topic alone, reject as unrelated, or interpret user silence as agreement.

User reporting at most once if the ambiguity materially affects work: “The parser invitation's purpose remains unclear after inspection; I left it pending and continued the implementation.” No repeated confirmation question is needed. On unchanged attention notices 2, 3 and 4, take no invitation action and issue no duplicate reminder. Do not repeat thread-show/history reads just because the same invitation remains in the digest. Handle any genuinely new mail normally and reconsider only when material metadata, remit or human input changes.

## 5. Service-required invitation

Required invitation iREQ, requirement rREQ, revision 7. This follows the distinct required-invitation procedure rather than the ordinary topic/goal acceptance branch.

Next action: `herdr-threads thread participants T`. Verify its current required row explicitly names `invitation=iREQ requirement=rREQ revision=7`. Then run `herdr-threads accept-required T --invitation iREQ --requirement rREQ --revision 7` using the exact current values. Do not substitute ordinary `accept T`. If participants reports different values, use those current values after deciding again. If acceptance returns `stale_requirement_acceptance`, reread participants and decide again rather than blindly retrying revision 7.

No extra human yes is needed under the separate required procedure. Required membership cannot be left until the service owner releases it. User reporting if material: “Accepted the current service-required membership.” No redundant pings or unchanged pending-invitation reminders.

## 6. Relevant invitation encountered by a child summary worker

The child never runs text `inbox`, check-in, accept, reject, accept-required, ACK, send, leave, invite or other seat mutation. If it must discover invitation metadata, first use `herdr-threads inbox --machine` (or `--json`), then `herdr-threads thread show T` if necessary. Relevant metadata already in its supplied data requires no extra discovery call. It continues its authorized read-only summary task and returns IDs plus the assessment through the native parent channel.

Exact returned assessment: “Ordinary invitation iORD for thread T: topic Parser integration; goal Coordinate parser integration. Both fit the top-level parser implementer's remit. Recommend that the top-level agent run its exact `herdr-threads accept T` command; I did not accept, reject or ACK.” Include any message IDs actually read so the top-level can decide exact receipts. The child does not ask the human to join and does not send this assessment through herdr-threads. It does not repeatedly relay an identical pending invitation to the parent on unchanged notifications.

The evaluated worker is under the explicit read-only/no-mutations instruction. Any general summary-worker submit instructions do not authorize a submit for this evaluation. Only this report write is authorized.

## Evidence and attention result

- Candidate skill Threads and invitations: top-level assesses both topic and goal; explicitly accepts relevant invitations without another human confirmation; broad operational role can establish relevance; missing/truncated metadata requires read-only inspection; unrelated invitations get reasoned rejection; genuine unresolved ambiguity is held; identical pending invitations are not repeatedly relayed.
- Harness TOP_LEVEL_INSTRUCTION repeats those assessment, inspection, no-extra-confirmation and no-repeat requirements. OPTIONAL_ACCEPT_LABEL is `accept (if topic and goal fit your role/remit)`; ready-block generation explicitly treats addressed work as ranking rather than joining authority.
- Compact inbox renderer prints a canonical available goal or `goal: unavailable; inspect before deciding` plus exact thread-show argv. Ordinary rows print the topic/goal assessment instruction and explicit accept argv. Required rows print exact invitation/requirement/revision acceptance argv.
- Child authority remains narrower: summarize and return IDs/assessments; do not mutate the top-level seat.

Outcome across scenarios: accept 1; inspect then accept 2 on stipulated operational relevance; reject 3; inspect then hold 4 without four repeated reminders; validate current requirement then accept-required 5; read-only return to parent 6. Human silence is neither a blocker for delegated relevant acceptance nor evidence resolving genuine ambiguity.
