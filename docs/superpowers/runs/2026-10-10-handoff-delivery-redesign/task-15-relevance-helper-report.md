# Task15 relevance helper source checkpoint

Checkpoint `25ac0d69fa214c903bf0ef51c8b66950748fae60`; exact frozen D snapshot baseline `308564b887e1d0f4af866c69a5ed40275c860b69`. Branch `helper-ht-0lo.8.4-relevance`, workspace `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.1.1`. Full baseline diff `task-15-relevance-helper.diff`, SHA256 `799c4f456820dbf8d188e35e7ffb775f37e8caeaeae74141f4bd52d4f1cf270a`, 112344 bytes. Only three admitted paths changed; all migrations1..39 remain untouched.

**Status: source-only, NOT compiled/tested/schema-approved/application-ready.** `rustfmt --edition 2024` parsed/formatted the two owned Rust files and `git diff --check BASE HEAD` is clean at this final checkpoint; initial unstaged-only check did not include then-untracked migration and missed one trailing blank line, found and removed in whitespace-only25ac0d69 before final package; neither verifies types, linked SQLite SQL/plans, producer behavior, budgets or correctness. No Cargo, Node, tests, model/network/Herdr/tracker or D runtime writes were performed. Independent full-helper/schema/interface review and quiescent application precede the D owner’s actual focused execution.

## Interface and ownership

`pub(super) advance(db:&Connection, claim:&CallerClaim, consumer:PresentationConsumer, frontier:i64, budget:&CallBudget)->Result<RelevanceProof,ApiError>` and same-shaped `revalidate`. `RelevanceProof { relevance: Open|Absent|Unknown, more:bool, work:RelevanceWork }`; work has `sources,fallback,point_probes,highwater_seeks,witness_probes: u16`. Unknown alone has `more=true`; Open/Absent are exact current judgments. advance can run on fresh/replay/control/no-output pulls. revalidate performs no source/fallback/highwater sweep. Caller MUST own the deciding transaction and complete A2 admission; the helper refuses autocommit and checks exact current seat/binding/claim itself. SQLite owner progress/deadline handling remains caller-owned; cancellation is checked between bounded passes. No independent commit/callback/port or legacy attention/effective helper is invoked.

D owns module/schema registration, observed episode counters, control debt, outbox/descriptor validation, rendering, native eligibility and all external producer wiring. This module creates no observed epoch, changes no seen/offer/output version, ACK/accept or native wake. Consumer row may be absent during the first proof call; proof FK is directly to seat. Deleting the consumer row reclaims its private proof; actual seat purge cascades its proof.

## Fixed state and bytes

`presentation_warning_proofs`: one `(seat_id,consumer)` row for each of two fixed consumers per retained seat. JSON State has exact CallerClaim; captured fence/frontier/native_version i64; `[Slot;13]`; separate source/fallback rotations; absent bool; optional Witness. Slot has high i64, after Key(ordinal,row), exhausted bool, optional candidate Key, optional typed Judge. Judge is Representations(after,published bool), or Membership(representation ordinal optional, after, high). Witness has slot index, Key, optional exact representation ordinal and exact interval ordinal. No warning/body/event JSON, arbitrary map/list/history or retained peer strings in cursors/witnesses.

Analytic upper bound: each slot’s names/enums/punctuation and <=14 decimal i64/bool fields is <700 encoded bytes; 13 slots <9100. Witness <400. At most six claim identifiers are conservatively charged at128 ASCII-graphic raw bytes/256 JSON-escaped bytes each:1536. Outer names/scalars/enums <1024. Total <12060; selected private power-of-two cap32768. Load and save both enforce serialized32768, exact serde array cardinality and deny_unknown_fields, bounded ASCII claim identities, nonnegative/range/rotation/key/judge/witness invariants, and complete-only absent state. Numeric fields use checked SQLite i64/u64 decoding, never JS/floating-point round trips. Canonical epoch u64 text belongs to D, not this proof. Schema repeats32768+json_valid.

## Source/range inventory

| Slot | Physical indexed range | Judgment identity |
|---|---|---|
| 1 | warning_conditions affected / presentation_conditions_affected partial open | condition rowid/open_warning_id |
| 2 | warning_conditions instance+inviter / warning_conditions_invitation_origin partial | condition rowid/open_warning_id |
| 3 | receipt_warning_origins instance+seat / receipt_warning_origins_seat | origin rowid/warning_id |
| 4/5 | digest_open_warning_recipients seat + fixed job/prepared / digest_open_warning_recipients_seat | physical rowid + source_ordinal; prepared uses exact ordinal |
| 6/7 | digest_open_warnings affected + fixed job/prepared / digest_open_warnings_affected | physical rowid + source_ordinal; prepared uses exact ordinal |
| 8 | digest_programmatic_warnings seat above exact offered frontier / digest_programmatic_warnings_seat | physical ordinal; sole frontier-limited source |
| 9/10 | warning_jobs fixed pending/failed / warning_jobs_ready | job ordinal/warning_id |
| 11/12 | work_jobs fixed pending/failed+send_attention / work_jobs_ready | job ordinal; service-notify publication+recipient exact keys only |
| 13 | prepared_unavailable_warnings / prepared_unavailable_warnings_ordinal | physical prepared ordinal; publication/missing-job/active-work judgment AFTER range LIMIT |

Each capture uses one indexed MAX per slot. Each source batch takes physical `(ordinal,rowid)>(after,row)` and `ordinal<=captured high` with SQL LIMIT before source_id reload, joins or judgment. Composite rowid tie preserves duplicate projection source ordinals rather than silently skipping them. Source identity reload uses exact physical rowid plus expected ordinal and current family predicate. Deleted tails exhaust through an empty indexed seek; older inserted ordinals invalidate the fence and are reconsidered only after finite captured completion.

Legacy generic warning sources4/6/9/10 have resumable prepared-representation lookup `(warning_id,ordinal)` before direct fallback; known prepared sources5/7/13 use exact PK/preparation/manifest. Every visited representation is charged, including unpublished/nonactionable ones. Published representation membership failures resume remaining representations, not an unbounded rescan. Direct fallback is permitted only after a coherent complete no-published-representation walk, and its positive witness is Unknown after a changed fence until bounded proof recaptures. This conservative generic-direct witness restriction is intentional: it never assumes an old representation absence survives unrelated canonical writes.

Membership fallback uses `(thread_id,seat_id,ordinal)`, frozen canonical interval_high_water, current exact warning recipient cutoff, charged physical LIMIT-before-joined/left filtering, and exact seat retirement cutoff. It retains one suspended cursor per source and, on positive, remembers the exact retained interval ordinal. It does not assume nonoverlap or use only the latest interval. An invalid current witness returns Unknown, not Absent. No fallback body/history/JSON is retained.

## Call/work accounting

Source/fallback visits share100 per successful advance. Fixed-source and suspended-fallback rotations are separately persisted, reserve50 each, then lend unused share. ALL source/fallback/highwater seeks charge and stop at26; first-call13-highwater capture can consume room and yield before fallback. Replay/control calls progress the same proof even with an outstanding outbox (caller wiring remains D). No fresh allocation or outbox emptying is required. Stable exact absent proof does zero source/fallback/range work. A currently exact bounded witness does no history walk.

Every typed keyed operation uses Points::row/yes, with declared joined PK/UNIQUE hop cost and an enforced64 point-result cap per judgment. Absent-result attempts are conservatively charged too. Work exposes actual source/fallback total, all range/highwater seeks, source/fallback/preflight keyed costs and separate positive witness keyed costs. The module enforces total point_probes<=6400 and witness<=64 before save. No descriptor probes occur here; D must add its separately capped<=32*64 descriptor costs for total<=8512.

Conservative analytic keyed allowance: <=32 per examined source/fallback row (100*32), plus at most26 suspended keyed preflights and26 terminal empty-fallback evaluations at<=32 each:4864<6400, separately <=64 witness. These preflights/empty seeks are explicitly counted even when no physical row is returned; they are not passed off as visited rows. Actual linked-SQLite counters/VM/plans must verify this inventory; no empirical bound is claimed. Global fixed calls additionally use: admission seat+binding(2 keys), current confirming existence(1), fence singleton(1), current proof PK(1); snapshot reads native wake metadata(1), and proof save(1 key). metadata wake_work is NOT fence authority.

Typed judge maximum inventory: source_id1; source job identity1 or source13 warning-job1+work linkage2; direct message+job2 OR prepared+manifest2; native rejection message/rejection/invitation3; open transition1 and conditional receipt-origin1 OR clear transition1; service publication1 OR legacy warning-job1 plus invitation/canonical seat/cancel/reject4 OR state-only receipt logical4, fallback physical3, human current binding1, waiver1; recipient service publication/recipient2, warning recipient1, warning job cutoff1, seat retirement1; exact witness interval1. Receipt judgment does not read availability/deadline because only effective pending state affects relevance; it preserves logical-before-physical, terminal, retirement, not-required and human-waiver precedence. No unbounded old effective_receipt/effective_warning_by_id/is_warning_recipient/warning_condition_actionable/judged_warning call.

## Mutation fence, bootstrap, writers

One singleton checked i64 fence; each INSERT/UPDATE/DELETE on every following source/judge input performs only keyed missing/negative/exhaustion guard and singleton increment in that writer transaction (overflow ABORT). No recipient SELECT loop, warning scan, fanout or new driver class. It is private absence invalidation only.

`messages`, `warning_conditions`, `receipt_warning_origins`, `warning_jobs`, `warning_recipients`, `prepared_unavailable_warnings`, `work_jobs`, `send_preparations`, `send_manifests`, `prepared_recipients`, `receipt_state`, `receipts`, `receipt_send_time_cutovers`, `invitations`, `invitation_cancellations`, `invitation_rejections`, `requirement_episodes`, `seats`, `membership_intervals`, `memberships`, `occupant_bindings`, `seat_availability`, `human_receipt_waivers`, `service_notification_publications`, `service_notification_recipients`, `digest_open_warnings`, `digest_open_warning_recipients`, `digest_programmatic_warnings`.

Excluded: derived confirming projection; proof rows and fence itself; all consumer presentation/item/epoch/outbox/seen/ledger state; notice frontier (captured independently); wake_work bookkeeping. No module judge reads host_instances, threads, recovery_holds or any transitive callee. Complete A2/host admission remains caller-owned. Fence includes conservative receipt cutover/availability/send-preparation inputs even though state-only receipt predicate does not need them. All actual module source/judge reads are covered.

Projection literal invariant is digest_pending invitation AND same canonical invitation AND (confirmation NULL/require_ack OR unique requirement episode pending/accepted). Both bootstrap and all recomputation use that exact predicate. Digest INSERT/DELETE/UPDATE removes OLD then recomputes NEW exact invitation. Requirement INSERT/DELETE/UPDATE OF invitation_id,state removes/recomputes OLD/NEW IDs, at most2 exact canonical invitations/digest rows and unique episode probes, no new alternate requirement index. Ordinary invitation policy is immutable in32. Existing canonical control/service requirement creation, acceptance, release/retirement; invitation acceptance/cancel/reject writers already mutate their canonical inputs and digest, with DB hooks covering direct canonical writers. Repository search found no DELETE FROM invitations or requirement_episodes runtime retention writer. Arbitrary corrupt orphan digest/invitation repair is not a new runtime scan.

One-time migration bootstrap SELECT over retained digest pending P plus keyed invitation/effective requirement joins and three historical source index builds is explicitly startup cost; this helper does no runtime repair/bootstrap. The parent-selected TRUST accepted startup/index cost, constant fence bookkeeping in Q, and absence/clear delay under continuous unrelated mutations must be written by D in the implementation commit. During an unstable sweep, cursor work finishes captured finite ranges first; on completion mismatched fence/frontier causes one recapture (or bounded yield awaiting13 seek room), never immediate restart every quantum. Only coherent exhaustion establishes Absent. Private proof writes cannot self-fence.

## Unrun fixtures and required subsequent evidence

Three new tests in existing tests/store/attention.rs: required ordinary-Lazy overlay insert/digest delete/reinsert/accepted/released; quiet exact absence private-write isolation and fenceMAX writer rollback;257 unpublished source jobs plus301 retained intervals, finite proof convergence, exact bounded witness revalidation, loss=>Unknown then quiescent Absent. These fixtures are UNRUN and may still have compile/setup issues. They are not claimed RED/GREEN. Earlier actual missing Attention RED belongs to D’s real handler, recorded externally; this helper does not relabel that evidence.

Mandatory after reviewed quiescent application: compile/module/schema registration; required-Lazy and all invitation writer variants/bootstrap; all13 source families/recipient/transition/rejection/service cases; source/fallback fair progress, restart/rollback/frontier changes, corruption/overflow/self-invalidation; retained intervals and representations including unrelated global mutation while captured sweep finishes; linked-SQLite populated39→40→41 atomic upgrade/reopen/catalog/FK/rollback, source and fallback EXPLAIN/VM scaling, actual counters, plus real handler missingAttention GREEN and observed-episode/replay/confirm composition. Capability remains unadvertised until complete D path is ready.

## Complete additive41 schema object inventory

- table `presentation_confirming_invitations`
- index `presentation_confirming_invitations_seat`
- trigger `presentation_confirming_digest_insert`
- trigger `presentation_confirming_digest_delete`
- trigger `presentation_confirming_digest_update`
- trigger `presentation_confirming_requirement_insert`
- trigger `presentation_confirming_requirement_delete`
- trigger `presentation_confirming_requirement_update`
- index `presentation_conditions_affected`
- index `presentation_membership_history`
- index `presentation_prepared_warning`
- table `presentation_warning_proofs`
- trigger `presentation_warning_proof_reclaim`
- table `presentation_warning_fence`
- trigger `presentation_fence_messages_insert`
- trigger `presentation_fence_messages_update`
- trigger `presentation_fence_messages_delete`
- trigger `presentation_fence_warning_conditions_insert`
- trigger `presentation_fence_warning_conditions_update`
- trigger `presentation_fence_warning_conditions_delete`
- trigger `presentation_fence_receipt_warning_origins_insert`
- trigger `presentation_fence_receipt_warning_origins_update`
- trigger `presentation_fence_receipt_warning_origins_delete`
- trigger `presentation_fence_warning_jobs_insert`
- trigger `presentation_fence_warning_jobs_update`
- trigger `presentation_fence_warning_jobs_delete`
- trigger `presentation_fence_warning_recipients_insert`
- trigger `presentation_fence_warning_recipients_update`
- trigger `presentation_fence_warning_recipients_delete`
- trigger `presentation_fence_prepared_unavailable_warnings_insert`
- trigger `presentation_fence_prepared_unavailable_warnings_update`
- trigger `presentation_fence_prepared_unavailable_warnings_delete`
- trigger `presentation_fence_work_jobs_insert`
- trigger `presentation_fence_work_jobs_update`
- trigger `presentation_fence_work_jobs_delete`
- trigger `presentation_fence_send_preparations_insert`
- trigger `presentation_fence_send_preparations_update`
- trigger `presentation_fence_send_preparations_delete`
- trigger `presentation_fence_send_manifests_insert`
- trigger `presentation_fence_send_manifests_update`
- trigger `presentation_fence_send_manifests_delete`
- trigger `presentation_fence_prepared_recipients_insert`
- trigger `presentation_fence_prepared_recipients_update`
- trigger `presentation_fence_prepared_recipients_delete`
- trigger `presentation_fence_receipt_state_insert`
- trigger `presentation_fence_receipt_state_update`
- trigger `presentation_fence_receipt_state_delete`
- trigger `presentation_fence_receipts_insert`
- trigger `presentation_fence_receipts_update`
- trigger `presentation_fence_receipts_delete`
- trigger `presentation_fence_receipt_send_time_cutovers_insert`
- trigger `presentation_fence_receipt_send_time_cutovers_update`
- trigger `presentation_fence_receipt_send_time_cutovers_delete`
- trigger `presentation_fence_invitations_insert`
- trigger `presentation_fence_invitations_update`
- trigger `presentation_fence_invitations_delete`
- trigger `presentation_fence_invitation_cancellations_insert`
- trigger `presentation_fence_invitation_cancellations_update`
- trigger `presentation_fence_invitation_cancellations_delete`
- trigger `presentation_fence_invitation_rejections_insert`
- trigger `presentation_fence_invitation_rejections_update`
- trigger `presentation_fence_invitation_rejections_delete`
- trigger `presentation_fence_requirement_episodes_insert`
- trigger `presentation_fence_requirement_episodes_update`
- trigger `presentation_fence_requirement_episodes_delete`
- trigger `presentation_fence_seats_insert`
- trigger `presentation_fence_seats_update`
- trigger `presentation_fence_seats_delete`
- trigger `presentation_fence_membership_intervals_insert`
- trigger `presentation_fence_membership_intervals_update`
- trigger `presentation_fence_membership_intervals_delete`
- trigger `presentation_fence_memberships_insert`
- trigger `presentation_fence_memberships_update`
- trigger `presentation_fence_memberships_delete`
- trigger `presentation_fence_occupant_bindings_insert`
- trigger `presentation_fence_occupant_bindings_update`
- trigger `presentation_fence_occupant_bindings_delete`
- trigger `presentation_fence_seat_availability_insert`
- trigger `presentation_fence_seat_availability_update`
- trigger `presentation_fence_seat_availability_delete`
- trigger `presentation_fence_human_receipt_waivers_insert`
- trigger `presentation_fence_human_receipt_waivers_update`
- trigger `presentation_fence_human_receipt_waivers_delete`
- trigger `presentation_fence_service_notification_publications_insert`
- trigger `presentation_fence_service_notification_publications_update`
- trigger `presentation_fence_service_notification_publications_delete`
- trigger `presentation_fence_service_notification_recipients_insert`
- trigger `presentation_fence_service_notification_recipients_update`
- trigger `presentation_fence_service_notification_recipients_delete`
- trigger `presentation_fence_digest_open_warnings_insert`
- trigger `presentation_fence_digest_open_warnings_update`
- trigger `presentation_fence_digest_open_warnings_delete`
- trigger `presentation_fence_digest_open_warning_recipients_insert`
- trigger `presentation_fence_digest_open_warning_recipients_update`
- trigger `presentation_fence_digest_open_warning_recipients_delete`
- trigger `presentation_fence_digest_programmatic_warnings_insert`
- trigger `presentation_fence_digest_programmatic_warnings_update`
- trigger `presentation_fence_digest_programmatic_warnings_delete`
