# Human message attribution and intent proposal

## Goal

Make thread transcripts and summaries distinguish human queries, one-off requests and rules, while preserving honest attribution. An agent recovering context should see unanswered questions, unfinished requests and applicable rules without treating every human message as a permanent instruction.

Status: approved design; implementation contracts finalized autonomously. The user approved the names `query`, `request` and `rule`, explicit recorded classification chosen by the forwarding agent, guidance in `ht skill`, summary-time resolution, preserving attention priority for all human intents, and legacy behavior for unclassified messages. Cross-chunk visibility is an explicit requirement described below. Implementation has not started.

## Current behavior

`send --relays-user` records the sender's cooperative claim that it is forwarding its own user's instruction. The sender remains the author. The daemon does not infer attribution from text, quotes or imperative wording, and cannot verify the claim.

`author_role` independently records the sender's binding role at send time: human, agent or service. Older roles may be backfilled and marked accordingly. Relay and role are independent; a human sender can also set the relay flag. Message JSON omits false `relays_user` values, which readers interpret as false.

Every ordinary message with a human role or a relay claim currently receives summary priority and is automatically prefilled as a `user_instruction`. This includes human questions and requests because there is no intent field. Priority also allows a message to cite an instruction supersession and bypass the catch-up attention hold. System events are excluded from instruction prefill.

The summary ledger already tracks agent-derived open items of kind ask, question, commitment or blocker. Summary workers propose their resolution with a later message citation. The daemon validates the item, sequence and allowed transition; semantic resolution is the worker's judgment. Routine inbox checking does not create or resolve these items. ACK records receipt separately.

Sources: [trust policy](../../../TRUST-POLICY.md), [existing summary design](../../design/herdr-threads/thread-summaries/2026-10-02-thread-summaries-compaction-survival-design.md), [agent guidance](../../../integrations/skill/SKILL.md), and the current ledger, fold and submission validator under `src/summary/`.

## Record attribution and intent separately

Keep `--relays-user` as the attribution claim. Add an optional `--user-intent query|request|rule`, stored as `user_intent` and carried through message results, summary bundles and ledger records. These spellings are the chosen interface.

| Intent | Meaning | Example |
|---|---|---|
| `query` | A question asking for information | “What's our progress?” |
| `request` | An action with a completion point | “Let's cut a build.” |
| `rule` | An ongoing constraint that survives individual applications | “Always run tests before cutting a release.” |

The sending agent chooses and records the intent when forwarding human input. A declared human sender can choose it directly. Intent does not turn an agent into a human author or confer permission. Validation: an ordinary agent send needs `--relays-user` to attach a human intent; direct human sends do not. Services and system events cannot acquire human attribution through this option.

Missing intent means unclassified, including historical messages. Never infer historical intent during migration. Classification is a cooperative claim, not daemon verification of the text.

Split input containing independently actionable questions, requests and rules into separate messages. Attribute the human source and keep the agent's interpretation distinct. Ambiguous duration should be clarified with the user rather than silently classified as a rule.

## Transcript and JSON presentation

Display source and intent separately. Proposed human examples:

```text
<builder> [user input relayed; sender claim] [query] What's our progress?
<builder> [user input relayed; sender claim] [request] Let's cut a build.
<builder> [user input relayed; sender claim] [rule] Always run tests before cutting a release.
<human-seat> [human] [rule] Always run tests before cutting a release.
```

Ordinary agent messages and incidental quotations receive no human-input markers. A quoted rule discussed by an agent is not automatically an adopted rule. Unclassified relays show attribution without an invented intent. System events retain their event presentation.

Retain existing compact attribution tokens and add a separate intent marker when present. JSON exposes the recorded fields rather than display labels. Update CLI help and document every presentation, including the actual summary ledger role/relay tokens. Changes to summary bundle rendering must version the renderer so stored chunks are not mixed across formats.

## Summary lifecycle

“Keep” means carry an open or active entry forward in the summary ledger. Long content may be represented by a reference to its source message. It does not imply that every inbox check repeats the item or that the system creates a reminder.

| Intent | Initial ledger state | Proposed closure |
|---|---|---|
| Query | Open question | Resolve on a later cited answer or explicit human withdrawal/replacement of the question |
| Request | Open ask | Resolve when a later cited message reports completion or explicit cancellation |
| Rule | Active rule | Supersede when later human input explicitly withdraws or replaces it |

Reuse existing open-item resolution for queries and requests. Generate their ledger records deterministically from the explicit classification so they survive even when a worker falls back to a ledger-only summary. Give each record a stable source identity and prevent the worker from duplicating the same source as another open item.

A rule needs a distinguishable ledger representation and allowed transitions: performing one compliant release must not mark the rule done. Rules stay active within their thread; this proposal does not install global instructions or edit user configuration.

### Visibility across chunks

Every level-0 worker must receive the deterministically derived query, request and rule entries from all earlier messages through its chunk's end, including messages whose summary blocks have not yet been stored. This extends the existing priority-message prefill mechanism; merely asking the first worker to discover an open item would lose that protection.

For example, a query at sequence 12 in chunk A must already appear under a stable ID in chunk B's input. When B sees an answer at sequence 27, it can submit a resolution of that ID citing 27, even if A is still running. The final ledger applies records and transitions in thread sequence order, regardless of submission order. Worker guidance must explicitly require checking the supplied open ledger against messages in the current chunk.

Tests must fetch B before A submits, then submit B before A, and verify that the final ledger resolves the query. Cover requests as well, plus unanswered queries and partial answers that remain open. This guarantees that the worker can see and cite the item; it does not guarantee that the model correctly recognizes every answer.

Ordinary agent asks discovered by summary workers still have the existing parallelism limit: B cannot close an item invented by A if it was absent when B fetched. Fixing all model-discovered open work would require another design, such as ordered jobs or a later reconciliation pass, and is outside this human-intent proposal.

Closure remains a summary worker's judgment submitted through `summary submit`, with a later message citation. Explicit human withdrawal or replacement of an unanswered query is a valid resolution; the replacement source is separate work. Partial answers, promises to act, silence and ACKs are insufficient evidence of completion. If uncertain, leave the item open. The daemon checks structural validity and allowed transitions; it cannot prove that an answer satisfies the user or that a claimed build actually succeeded.

Recently closed entries appear as short status lines; older closed entries leave the displayed ledger. Resolution does not itself delete source messages or change receipt state. Existing history retention remains a separate policy.

Summary jobs update this state when they run. Compaction can prompt an agent to request a summary; it does not itself resolve items. No routine inbox extraction, automatic resolution on send, reply correlation protocol or dedicated `ht resolve` command is proposed.

## Compatibility and priority

Preserve the existing attention priority of human and relayed ordinary messages for all three intents. A short human query can need immediate attention even though it is not an enduring rule. Separate that priority from ledger kind and lifetime.

Unclassified historical messages and existing unclassified sends retain their legacy ledger behavior. Show them as unclassified human input rather than claiming they are rules. Existing summary blocks remain readable; a new summary generation incorporates explicit intent without rewriting old message claims or receipts.

Update TRUST-POLICY.md to explain recorded intent and the separation between attention priority and summary lifetime. Preserve canonical daemon decisions, cooperative attribution and receipt provenance. The concrete compatibility contract below is finalized before implementation; incompatible clients fail explicitly rather than silently discard intent.

## Final implementation contracts

### Storage, submission and generations

Main and this branch start at schema 21 (main `5aa02d96`). This feature owns exactly one forward migration, `0022_user_message_intent.sql`, advancing to schema 22. Add nullable `messages.user_intent` with only `query`, `request`, `rule` accepted when non-null. Existing rows stay NULL. Historical migrations, source messages, role backfill and receipts are untouched. Add nullable `summary_transitions.rule_change` accepting `withdrawn` or `replaced` when present, preserving absent legacy evidence, and nullable `summary_jobs.fetched_bundle_json` to freeze the worker input for its current fetch/lease. Extend startup schema audit and new-database installation with this migration.

Wire protocol advances 4→5; reject mismatched envelopes using the existing explicit protocol mismatch behavior. Carry optional intent through SendMessage, journal/retry identity and digest, message rows/projections, history/search/inbox/body and send results, and BundleMessage. Missing JSON fields decode as None and None omits the field. Service sends/events never accept or manufacture intent. The daemon decides eligibility from the canonical send-time role: ordinary Human may declare intent with or without relay, ordinary Agent requires relay; Service and non-ordinary events refuse intent. Unknown or unbound roles do not gain eligibility from intent. Retries with changed intent conflict rather than reuse a prior result.

Summary submission schema advances 1→2 and schema1 submissions are explicitly Rejected by the normal validator/fallback path. Renderer advances 1→2, changing chunking_version; worker guidance uses `thread-summary-v2` and daemon fallback uses `daemon-fallback-v2`. Already stored blocks/items and historical JSON remain readable and retained: missing intent means None. New summary planning, Ready covers, rollups, folds and deterministic source hashes use only the current renderer generation. Old leases/jobs cannot submit into the new generation (explicit stale-generation Conflict); no old block is silently relabeled or mixed into a new cover. Old-generation closures remain in the old generation, as in the existing renderer-version contract; a rebuilt generation can reintroduce unresolved historical legacy instructions, without rewriting old claims.

### Deterministic record and transition contract

Retain `ItemBody::UserInstruction` and SQL `summary_items.kind=user_instruction` as compatibility persistence tags, adding optional `user_intent` to that body. This tag does not imply an enduring instruction. Its existing author_seat, author_role, relays_user, message_id, text/text_ref fields preserve attribution and source access. Every deterministic ordinary priority message retains the stable thread-local ID `i.<sequence>`, independent of chunk boundaries, submission order and renderer generations. One source has exactly one deterministic record. Query is presented as question work, request as ask work, and rule as an active rule; no synthetic model OpenItem is needed. Reusing open-item resolution means using existing `Resolved` status and transition validation, not changing this persisted body to OpenItem.

The allowed statuses are precise: None starts Open and accepts Done or Superseded with existing legacy guards; Query and Request start Open and accept only Resolved; Rule starts Active and accepts only Superseded. Explicit cancellation of a request is a resolution. An explicitly withdrawn or replaced query also resolves, citing the later ordinary human/relayed message that cancels the need for its answer; semantic recognition remains the worker's judgment, with no extra query transition field. An ordinary unrelayed agent cannot unilaterally withdraw a human query: worker withdrawal/replacement judgment must cite the ordinary human/relayed priority source. Agent answers remain valid answer evidence. A replacement question is a separately classified source with its own stable ID. A later answer/completion can be ordinary agent input. Rule supersession requires a later ordinary human/relayed source plus proposed `rule_change: withdrawn|replaced` and an exact nonempty quote from that citing message, representing explicit withdrawal or replacement; the daemon enforces kind/priority/citation/quote and status validity, while the worker judges the quote's meaning. Compliance, silence, ACK and incidental agent quotations never close a rule. This is cooperative derived-summary judgment, not textual inference by the daemon.

Reject new_open_items anchored at a classified deterministic source sequence, whatever proposed kind, because the supplied ledger already owns that human source. Ordinary model-discovered work remains supported; unclassified sources retain the existing behavior. All transition cites must be in the worker's current chunk, strictly later than the source. Submit validation and final folding share the same kind and citation guards. Persist optional rule_change in each Transition and in summary_transitions; non-rule transitions must omit it. Rule quotes remain submission-only after validated storage; the final fold rechecks rule_change and source kind/priority against immutable canonical messages.

Every level-0 bundle derives all ordinary priority sources through its last sequence before applying stored transitions. Seed deterministic sources before any stored block transitions, so a previously stored chunk B cannot lose its closure when chunk A has not stored yet; appending a synthetic broad-range block after B would be wrong. Persist only the local chunk's deterministic records in that chunk/fallback. The Ready/final fold orders introductions and transitions by immutable message sequence, not block arrival or a synthetic range's sort position, and deduplicates by i.<sequence>. Cumulative bundle input and final record ownership are different views of the same source IDs. The first fetch atomically persists the complete encoded JobBundle in `summary_jobs.fetched_bundle_json` in the deciding transaction. Repeated fetches and submit validation use that same serialized bundle snapshot, after current caller/lease/generation checks; changed settings never regenerate the input of an otherwise valid lease. Clear/reset it on lease acquisition/replacement with the existing fetched_at lifecycle, and never reuse a previous lease's snapshot. Timestamps remain provenance, never a membership fence: blocks committed after fetch do not enter the worker's validation input, even at an equal timestamp or after clock rollback. The current final Ready fold still sees all stored records in sequence order; only worker input is frozen. Add equal-timestamp and clock-rollback intervening-commit regressions. Existing old-generation rows remain nullable and stale; no legacy frozen bundle is invented.

Closed classified entries use the existing recently-closed one-line/older-omitted rendering. Open query/request and active rule entries keep the same source-text spilling, rollup pinning, bundle/fold size accounting and over-budget behavior as legacy priority entries. If bounded text spills, the ID, intent and source reference remain. No active rule may disappear because pinning assumed Open status alone.

### Task validation scope

Focused tests exercise direct human, relaying agent, unclassified legacy and refused service/system/unrelayed-agent configurations through send/storage/JSON and summary entry points. The terminal integration sweep implements uncovered seam tests and runs only relevant focused tests plus required fmt/clippy/default-feature checks. Full-suite and release validation are coordinator-owned and explicitly excluded from this branch's tasks.

## Agent guidance

Update `integrations/skill/SKILL.md`, the embedded text printed by `ht skill`, plus CLI help and `docs/agent-usage.md`. Include the three examples, explicit forwarding classification, mixed-input handling, source attribution and uncertainty handling.

Revise summary-worker guidance to distinguish rules from open work, cite actual answers or completion messages, and explain that closure is agent-derived. Remove wording that treats every human/relay entry as a standing instruction or as permission to act.

## Validation and delivery

Targeted tests should cover classification recording and retry fidelity; human and relay eligibility; absent historical intent; transcript/JSON distinctions; deterministic query/request/rule records; fallback preservation; duplicate prevention; closure citations; partial answers and ACK independence; rule survival after compliance; explicit withdrawal/replacement; unchanged attention priority; and version boundaries.

Run the required per-change clippy check, formatting and focused tests. Before merge, run the default-feature check. Do not run a per-tab full suite. Implementation requires independent review and a frozen SHA/base/checks handoff to threads-main, which owns the main merge. Confirm DONE+MERGED after coordinator squash and report owned process completion and cleanup details.

## Decisions for review

The user accepted summary-time resolution, attention priority for all human intents and legacy ledger behavior for unclassified messages. The proposal now makes visibility across chunks explicit for classified human input. The concrete flag and display wording remain proposed for review. There is no proposal to eliminate the separate parallelism limit for model-discovered ordinary agent asks.

## Post-Implementation Notes

**2026-10-04 — Changes vs. original design:** Implemented and independently reviewed on the relay feature branch. The precise existing changed-payload retry error remains OperationPayloadMismatch. Canonical InboxBatch/body claim projection repairs were required for actual marker visibility; aggregate JSON inbox remains a thread view. Both Codex permission texts are byte-preserved. The code roast confirmed indefinite cumulative snapshot retention as a storage-quality punch-list item; the independent scope filter left post-replay cleanup outside this run, still open. Worker input remains frozen during valid-lease replay. Main merge, channel combined wire6 and full-suite/release gates remain coordinator-owned. See the run report and code-final-review for evidence and accepted limits.

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
