## Goal

Top-level agents explicitly reject ordinary invitations they have decided are a bad fit, using an exact invitation ID and a meaningful nonblank reason. Thread members receive the reason as an honestly attributed warning system message, once across durable retries.

## Existing behavior and scope

The shipped skill already directs rejection when clearly unrelated. Strengthen it to cover every decided bad fit, explicitly prohibit leaving that decision pending, require a meaningful nonblank reason, and explain automatic thread warning delivery. Inspect missing/truncated topic or goal first. Genuinely unresolved fit may remain pending; explicit preapproval and service-required memberships retain their rules. Subagents never accept/reject.

The store already validates reasons and exact invitation ownership, retains rejection provenance, settles only the invitation, guards required/stale episodes, and deduplicates using reject:<invitation>. It currently emits an info audit. Overdue/clear warnings are separate and do not carry the rejection reason. Keep the original event and payload and change new rejection events to warn; no parallel event, schema migration, historical rewrite or new provenance.

## Delivery design

Reuse bounded warning_jobs/work_jobs attribution, freezing the thread membership-interval high water and publication decision sequence in the rejection transaction. The rejecting seat is the affected recipient; thread members at that frontier receive it. Late joiners are excluded. Native event attribution remains the rejecting seat. Add an exact canonical helper recognizing only native warn events with the original reject event key, source invitation and matching retained rejection actor; never infer this class from arbitrary event JSON.

Before attribution, existing warning backlog scans judge the new warning through this canonical helper. During bounded recipient attribution, project these warnings into the existing occupant-scoped digest_programmatic_warnings informational delivery table. This table name is historical and already covers built-in transitions. Mark them informational in actionability/offer handling; after projection the backlog cannot revive a carried notice. They do not wake recipients: delivery waits for their next hook/check-in or explicit inbox. Check-in settles only its carried bounded prefix for the current binding. Update TRUST-POLICY A7 to document rejection notices with the same delivery limits. No receipt ACK or membership acceptance is inferred.

## Compatibility and boundaries

Exact historical operation replay remains historical: an older info event is not rewritten or redelivered. Same-operation replay and fresh-operation same-invitation/same-reason replay return the retained rejection without new warning/job/delivery rows. Changed reasons, required membership, cancelled/accepted/retired episodes, foreign recipient and stale binding retain refusal behavior. No caller authority changes or new accepted continuity limits. CLI help describes the reason and automatic warning; no extra send is necessary.

## Verification

A focused store regression must fail against the old info event. Exercise timely rejection, canonical native author/reason/source, actual thread-member inbox and system-body warning output, pre/post bounded attribution visibility, no wake, check-in offer settlement, same-key and new-key replay, no duplicate event or redelivery. Add independent controls for whitespace reason, wrong thread/invitation and frozen recipients. Run existing rejection/race/required/receipt/journal/parser tests and relevant warning/materialization/skill tests. Run cargo fmt, required all-target/all-feature clippy and default-feature check. Use isolated fixture databases and no real config writes or shared-server restart. No full suite routinely while ht-zo4 remains open.

## Task split

1. Shipped skill and CLI guidance: one coherent guidance change, independently reviewable.
2. Native rejection warning delivery: store transaction, canonical warning classification and bounded attribution, with focused regressions.
3. Integration sweep: review the combined behavior, focused verification and checks; depends on both leaves.

No operational rollout or real invitation rejection is in scope. Main merge remains a separate human/coordinator decision.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*

2026-10-10 — Changes vs. original design: none to the delivery architecture. Canonical classification additionally compares retained payload/timestamp. Both roasts and whole-epic review cleared the implementation. Passive notices retain A7's existing conservative saturated-window wake exception; the PR panel unanimously rejected treating that accepted limit as a new regression. Combined verification passed 131 focused tests plus fmt/clippy/default-feature gates. Main merge remains separate.
