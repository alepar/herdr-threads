# Implementation seams for dependent briefs

## Task 5: historical receipt upgrade

Task1 owns prerequisite transactional `receipt_send_time_cutovers(instance_id,historical_decision_seq)` once-only marker. Capture existing instances once, missing future instance cutoff0. `receipt_state.deadline_at` wins. Historical sparse running deadlines use original canonical firstavailability aftersend/beforeretirement only when availability seq<=cutoff; latepostcutover availability cannot restart dormant deadline. Task5 owns bounded physical dormant upgrade/indexing, preserves marker/restart semantics and existing running/terminal states. Read Task1 final report/source before implementation.

## Task 6: invitation lifecycle

Task2 producer seam: `protocol::commands::InvitationConfirmation::{Lazy,RequireAck}`, `Invite.confirmation: Option<InvitationConfirmation>` default/skipNone, existing `deadline_millis`; None=historicaltimed. Same optional field in saved SemanticMutation::Invite; frozen recursion preserves. Schema provisionalv29 nullable confirmation/inviter_seat_id FK, immutablepolicy/origin trigger, invitations_confirmation_due index excludingLazy. Original legacy attribution must use canonical episode evidence, never laterduplicate/currentmember inference. `INVITATION_CONFIRMATION_V1="invitation.confirmation_v1"` defined but NOT advertised until lifecyclehandler active. Temporary canonical control guard rejects confirmation Some Unsupported; remove only together with active lifecycle+advertisement.

Task2 leaves resultmetadata absent. Task6 canonical projection producer owns additive optional confirmation/deadline metadata on InboxBatchItem/InboxBatchV2Item Invitation and other resulttypes as needed by Task8; historicalNone omitted fields/digests preserved. Expand declared protocol/results.rs/mechanical constructors only as needed; taskreview checks compatibility.

Task6 owns invitation-class scanner/cursor API and isolated fairness/lifecycle tests, not shareddriver. Task7 depends onTask6 and owns combined driver fair scheduling/crossclass250+ backlog tests. No futureTask7 dependency for Task6 isolated acceptance.

## Task 10: handoff policy

Task4 source parser seam: HandoffArgs.body Option<String>, file Option<String>, stdin bool in requiredexclusive group; input::read_body immediately before effects, retain only actualbody String in unchanged HandoffRequest/HandoffPayload fields, no sourceref/path. Policy fields separately owned Task10.

Task1 exposes SEND_TIME_RECEIPTS_V1 and cli::require_send_time_receipts_capability(client,clock). Capability required before newACKhandoff durable/nativeeffects. Pending/commitunknownretry onunsupported daemon conservatively refuses (ordinarypublicOperationStatus lacks verifiedscope); completedretainedcompound/localservicepresentation staysnoeffect. Read finalTask1 report.

## Schema merge

Task2 andTask4 each provisionally implement migration29 onbase28; serialintegration renumbers second to30, preserves0028 bytes, and runs freshschema seam review+realpopulatedoldschema upgradefixtures. Task1 cutpoint is additive transactional initialization marker, not conflictingnumberedmigration.
