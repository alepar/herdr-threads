# Task15 corrected bounded relevance helper checkpoint

Frozen corrected source `367da744905c1df11dbdf72d811c133e6bdf9e21` on `helper-ht-0lo.8.4-relevance`; complete baseline `308564b887e1d0f4af866c69a5ed40275c860b69` to HEAD diff `task-15-relevance-helper.diff`: **154432 bytes**, SHA256 `2f76269cef0ca8b9945f9e82c4efc00a149bed65346f65f90b5d92a4f113c6ea`. Exactly three owned paths, all historical SQL1..39 untouched. Checkpoint JSON records final file hashes. The original25ac report/diff are preserved as `task-15-relevance-helper-report-25ac.md` / `task-15-relevance-helper-25ac.diff`; the original25ac and test-only09d65 commits remain ancestors. Historical report statements about global positive-fence gating, absent sweep coverage, batch scheduling and zero-range condition witnesses are superseded by this report/final70 proposal.

**Source-only correction, uncompiled/unrun and not approved for application.** rustfmt parsed/formatted both owned Rust files; full baseline-to-HEAD git diff --check succeeded; branch clean. No Cargo/Node/runner/network/model/Herdr/tracker/real-config action or D-owned source mutation. Full independent cumulative helper/schema review, new quiescent application ACK and D's actual composed execution remain mandatory. No corrected GREEN/SQL-plan/counter/speed claim is made.

## Authority and actual RED history

Parent authorized exact final proposal `70ce105d8c621d254b4880a39c33c85731e01471d7435653979dac514fb145c1` after scoped rereviews2/3 CLEAN, and explicitly released this correction after actual intended failures at D sourcepoint `0505124f338d25b6445b1bc899f97bdf37bba82738317f62a749f421c7d50289`. Raw `/private/tmp/ht-task15-relevance-diagnostic-red.log` is preserved: five selected helper tests,2 PASS, direct legacy301 membership failed to Open after256 real unrelated Lazy publications, canonical receipt sweep still returned Open while its physical clear was NULL, and requiredLazy fixture failed setup with requirement relation mismatch. The setup failure is excluded from semantic RED. D's logged build was1m31 for multi-module diagnostic registration; it is not an incremental controlled speed result.

Original review finding global unrelated churn blocks positive direct warning: corrected by exact direct/prepared positive source union and independent direct membership/witness. Logical-close omission: corrected by current canonical successor coverage on every actual condition association, plus three mutation fence hooks. R1 seek fairness and cumulative invocation concerns: full-cost protected microsteps/shared accounting in helper, exactly-one advance and no Confirm walks explicitly delegated to D caller wiring. No new positive-discovery delay limit is accepted.

## Unchanged interface and caller seam

`pub(super) advance(db:&Connection, claim:&CallerClaim, consumer:PresentationConsumer, frontier:i64, budget:&CallBudget)->Result<RelevanceProof,ApiError>` and same-shaped diagnostic `revalidate` remain unchanged. `RelevanceProof { relevance: Open|Absent|Unknown, more:bool, work:RelevanceWork }`; work exposes u16 sources,fallback,point_probes,highwater_seeks,witness_probes. Unknown alone has more=true. Caller owns the deciding transaction and full A2 admission; helper refuses autocommit and checks current exact seat/binding/claim. No commit/callback/generic port/old attention helper. Caller owns SQLite deadline/progress handling; cancellation checked before bounded actions.

Frontier is the canonical offered programmatic notice ordinal for the binding/consumer contract, NOT host decision sequence. Only zero-based7 uses it. D must call advance exactly once on EVERY successful V2Pull fresh/replay/control/no-output path and reuse that coherent result/work for all same-transaction decisions; no second advance/revalidate/new meter. ExplicitConfirm uses exact persisted observed episode/frozen stamp/token/binding/consumer/lease and bounded keyed noncontrol identity validation; it NEVER calls advance/revalidate, successor coverage or another nonunique relevance query. revalidate is diagnostic positive/cache inspection only (one optional coverage successor, no discovery walk). D owns module/schema registration, output/observed episodes/debt/counters/outbox/descriptor validation/native eligibility and TRUST wording. No helper path allocates those controls or changes seen/offer/ACK/accept/wake policy.

## Fixed state and field bounds

Exactly one proof row per `(seat_id,consumer)`, two fixed consumers. Direct seat FK permits first proof before consumer-row allocation; consumer DELETE reclaims proof, seat purge cascades. State retains exact CallerClaim, fence/frontier/native_version i64, fixed `[Slot;13]`, two rotations, absent bool and one optional Witness. Slot: high i64, after Key(ordinal,row), exhausted bool, optional candidate Key, optional Judge. Judge: Representations(after) or Membership(Authority,after,high). Authority: DirectAssociation or PreparedRepresentation(one positive i64 ordinal). Witness: slot index, Key, Authority, optional exact interval ordinal. No retained warning ID, event/body/peer JSON, list/map/dependency set or history.

All numeric fields are SQLite i64 (20 decimal characters worst case, nonnegative/positive guarded), fixed enum spellings, fixed field names; at most14 scalar numeric/bool/enum fields per slot fits conservative700 bytes and witness at most8 fits400. CallerClaim has five <=128-byte ASCII graphic IDs plus harness spelling also checked with same128 bound; JSON escaping costs at most256 bytes each, u64 generation <=20 digits and bounded role. Outer names/scalars/arrays <1024. Conservative full bound13*700+400+6*256+1024=12060 <32768. serde fixed array+deny_unknown_fields and typed enums, ID bounds including harness, nonnegative scope values, rotations<13, candidate/judge consistency, source-specific Authority validity, representation<=captured source12 high and known-prepared ordinal equality, membership cursor/high, absent complete exhaustion/no witness and witness key/interval guards enforce this structural cardinality. Producer/save validates before serialization and enforces32768; load rejects oversized raw text and validates deserialized fields; schema CHECK bytes<=32768 and json_valid. The unlanded diagnostic JSON shape is not a deployed migration compatibility promise.

## Definitive zero-based physical sources, indexes and coverage

Every source is fetched with physical `(ordinal,rowid)` keyset, captured high and LIMIT1 before joins/judgment. source_id reload is exact rowid+expected ordinal+current family predicate. Ties in projection source_ordinal preserve physical rows. Each13 capture is indexed MAX and consumes one shared seek. No host_instances/threads/recovery_holds read occurs anywhere in helper/callees; instance matching uses immutable messages.instance_id or manifest.instance_id without a thread lookup.

| Slot | Physical/index scope | Current positive authority / coverage |
| --- | --- | --- |
| 0 | warning_conditions affected / presentation_conditions_affected | exact direct actual condition; receipt/unavailable coverage, invitation skips |
| 1 | warning_conditions invitation origin instance/seat / warning_conditions_invitation_origin | exact invitation association; invitation cannot be swept |
| 2 | receipt_warning_origins instance/seat / receipt_warning_origins_seat | exact associated condition+origin; receipt coverage |
| 3 | digest_open_warning_recipients seat+job / digest_open_warning_recipients_seat | exact job then independent direct/prepared alternatives; any actual receipt/unavailable condition covered |
| 4 | same seat+prepared | exact prepared ordinal/preparation/manifest; any actual unavailable condition covered |
| 5 | digest_open_warnings affected+job / digest_open_warnings_affected | exact job then independent direct/prepared alternatives; associated receipt/unavailable coverage |
| 6 | same affected+prepared | exact prepared ordinal; unavailable coverage when associated |
| 7 | digest_programmatic_warnings seat / digest_programmatic_warnings_seat | notice frontier alone here; programmatic publication+recipient; normally no native condition; any actual association still uses overlay |
| 8 | warning_jobs pending / warning_jobs_ready | exact job then direct/prepared alternatives; associated receipt/unavailable coverage |
| 9 | warning_jobs failed / warning_jobs_ready | same independent authority and coverage |
| 10 | work_jobs pending+send_attention / work_jobs_ready | exact service-notify publication+recipient only, normally no native condition; overlay if actually associated |
| 11 | work_jobs failed+send_attention / work_jobs_ready | same exact service authority |
| 12 | prepared_unavailable_warnings / prepared_unavailable_warnings_ordinal | publication/missing job/active work AFTER LIMIT; exact prepared association and unavailable coverage |

All associated conditions use physical clear plus current logical coverage regardless of source/authority/membership/origin route. Legacy no-condition association gets no invented condition. Closed/native rejection/clear transitions cannot bypass this shared eligibility rule. Conditions not addressed to affected/origin receipt-author seat are informational, not Open. A genuine current invitation-origin warning is already positive and can end scanning; no canonical covered-invitation fixture is invented to claim full13 negative coverage.

## Positive source union and complete bounded callees

Generic3/5/8/9 now examine direct association FIRST. Exact immutable warn message/current instance plus unique warning job matching thread/event sequence and canonical invitation/receipt source (or an actual associated transition condition) can independently prove positive; programmatic requires exact publication/recipient. The direct invitation association checks invitation thread/affected seat and, if retained on message, matching source_invitation_id; receipt requires immutable source_message_id and exact effective pending receipt. No authority from opaque ID prefix, arbitrary JSON or current membership. Wrong direct job sequence/thread cannot remain a direct witness. A conflicting prepared representation cannot erase this boolean positive source; this explicit selected coexistence rule does not claim old arbitrary prepared-first rendering equivalence. Direct negative/missing or exhausted historical membership proceeds to prepared alternatives; no direct absence or failed witness proves whole-union absence.

Known prepared4/6/12 uses ordinal directly. Generic fallback walks `(warning_id,ordinal)` / presentation_prepared_warning, charges each unpublished/nonactionable representation, resumes after negative prepared membership and finishes only the captured finite range. No representation-absence prerequisite or published bool remains. Direct Membership persists its Authority and numeric after/high, ignores unrelated negative-fence changes, walks physical membership `(thread_id,seat_id,ordinal)` / presentation_membership_history with LIMIT1 before cutoff filtering, uses canonical warning cutoff/high and retirement. No latest-interval/nonoverlap assumption. A same-transaction successful physical interval row is sufficient after its current source/condition preflight, so no duplicate coverage check is needed. Witness retains exact interval ordinal; subsequent revalidation reloads all current exact source/condition/recipient keys and that exact interval. Global fence is never positive authority.

Complete helper callee spine: advance/revalidate→check/admitted/confirming/fence/load/validate; snapshot→13 MAX/native metadata; advance→witness or schedule→next_slot/microstep→source_step→first_judgment or fallback_step→evaluate. evaluate→source_id, direct/prepared, eligible→native_rejection+condition+coverage+receipt-origin/clear lookup; then condition-free direct association and actionable→receipt_pending or exact invitation/seat/rejection/cancel/service, recipient→service/recipient/cutoff/retirement, optional exact interval. finish_negative/accept_evaluation/Authority functions only fixed memory. result fixed memory; save→validate+serde+exact proof upsert. No transitive legacy effective_receipt/effective_warning_by_id/is_warning_recipient/warning_condition_actionable/attention/recovery query. Tests' real producer functions may read host/config/thread state; those are not hidden runtime helper callees.

Declared keyed hops: source_id1; projection job identity1; slot12 job1+prepared/work2; direct message/job2 OR prepared/manifest2; native rejection message/rejection/invitation3; condition open1, addressed receipt-origin1 OR clear1; condition-free direct association job/message/invitation3; programmatic publication1; legacy job1 plus exact invitation/seat/cancel/reject4 OR receipt logical(manifest/prepared/seat/sparse)4, physical fallback(receipt/seat/message)3, human current binding1, waiver1; recipient service publication/recipient2, warning recipient1, job cutoff1, retirement seat1; exact witness interval1. All attempts (including absent) counted by Points; single typed Points context per source/fallback/witness microstep enforces64. Fallback preflight source_id and transient warning reload are in the SAME context, not an uncharged second judge. Conservative individual evaluation<=32, fallback<=35 and enforced64. Fixed admission(2 keys), confirming existence(1), singleton fence(1), proof load/save(1 each), capture native wake metadata(1) separately declared fixed work; native_version is diagnostic only and wake_work excluded from fence.

## Logical-close authority and no hidden sweep

Current unique open_warning_id lookup adds ordinal/kind/episode scalars. Receipt scope `(receipt,affected,NULL)` and unavailable `(unavailable,affected,exact episode)` execute one indexed successor on schema18 warning_close_sweeps_cover(kind,affected,episode,through,after): through>=condition.ordinal ORDER BY through,after LIMIT1, THEN after<ordinal<=through. Invitation kind skips. Nonunique successor charged as one shared range seek even empty and0/1 returned physical fallback row; never mislabeled as PK probe or filtered EXISTS. Exact current witness checks use this same optional current range, no global-fence predicate.

Sole canonical INSERT is enqueue_warning_close_sweep: in serialized deciding transaction high=MAX currently open scope condition ordinal, after=MAX retained same-scope through(default0), insert only high>after, create same-transaction work job. It yields retained adjacent disjoint scope chains(0,t1],(t1,t2]... despite arbitrary missing condition/global ordinal gaps. No production UPDATE/DELETE/cleanup of sweep authority exists; warning_close_unit drains exact-ID work and clears condition, retention only prunes selected work_jobs (close jobs themselves not in prune whitelist). Old active-warning through-only coverage is therefore equivalent on canonical scope chains to earliest-successor exact after check. This is producer/lifecycle proof, not permissive DDL nonoverlap. Future/repair writers must preserve/prove it or select a new bounded coverage algorithm before landing. D must record that TRUST clarification with the selected direct union and existing accepted startup/absence limits. No new retained-history projection/revision/bootstrap or manifest→warning fanout.

## Shared budgets and finite fairness

One invocation shares100 physical rows and26 combined source/fallback/coverage/highwater seeks. sources reports physical source rows; fallback reports physical representation/interval/sweep rows even for coverage during SOURCE/witness. Empty queries charge seek/points, not fake rows. SOURCE LIMIT1 full reservation2 seeks/2 rows; FALLBACK full reservation3/3 (actual current same-transaction reuse needs at most2/2); witness<=1/1. No cap increases. A source may create a suspended candidate and still constitutes complete progress. A fallback may resolve/change candidate without a physical row; its keyed work is charged. Reservations are released for unused actual work, not reported as visits.

In-memory next_slot checks fixed13 state only. Before any doomed SQL preflight, schedule jointly protects next eligible SOURCE and next suspended FALLBACK, with room5/5; reserves possible newly created fallback too. Execute source then existing/new fallback, ending immediately on positive. Initial13 MAX plus worst witness1 leaves12 seeks, so both opportunities fit. Independent persistent rotations advance ONLY after complete performed microstep. Alternating reserved passes give each family50 physical-row share counting all coverage within its microsteps; neither borrows before both protected opportunities and reserved passes stop. Then alternating lending still requires complete reservation in shared remaining budget. Exhausted seeks yields resumable Unknown and unchanged unfinished cursor, not error/restart/absence. Every continuously eligible slot receives a progress-capable family step within13 scheduling invocations; invocations ending with a current positive witness are intentionally excluded from discovery fairness. Captured membership/source ranges finish under unrelated churn; only full exhausted mismatch recaptures13 when room exists. Quiet unchanged fence/frontier absent cache has no discovery/sweep work. Only coherent complete current scope proves Absent; lost witness alone is Unknown.

Analytic total keyed bound: at most26 microsteps with a source/fallback range; each additional no-range fallback clears/transitions an initial candidate (<=13) or a candidate introduced by one of those <=26 ranged steps. Thus at most65 typed microsteps * enforced64=4160<6400, separately<=64 witness. This remains conservative even with empty ranges, scope changes and preflights; actual evaluate/fallback hop inventory is smaller. D descriptors at most32*64=2048 independently keyed, total allowance<=8512; descriptor allowance cannot hide nonunique coverage. All row/range work must remain shared100/26. Guard after computation checks all aggregate caps. `room` is checked before full microsteps; internal unreserved coverage is an invariant failure, not the normal budget-exhaustion path. revalidate is not used to reset a caller's meter.

## Migration41 writers, bootstrap and object inventory

Literal confirming invariant unchanged: pending digest row AND same canonical invitation AND(confirmation NULL/require_ack OR unique requirement pending/accepted). One-time SELECT bootstrap over retained pending digest plus keyed invitation/requirement, and three source index builds are accepted startup costs. No runtime repair/bootstrap loop. Digest INSERT/DELETE/UPDATE exact OLD/NEW row, requirement INSERT/DELETE/UPDATE OF invitation_id,state at most2 exact OLD/NEW canonical invitation+digest/effective episode recomputations. Existing immutable invitation confirmation and canonical acceptance/cancel/reject/service release/retirement writers feed those DB hooks; no alternate source of confirming truth.

29 input tables each have INSERT/UPDATE/DELETE singleton fence hooks (87 total), every writer uses missing/negative/MAX guard +one singleton increment, overflow ABORT rollback. No child scan/fanout/loop. New table is warning_close_sweeps only in fence scope (not a newly created authority table). Complete scope: messages, warning_conditions, receipt_warning_origins, warning_jobs, warning_recipients, prepared_unavailable_warnings, work_jobs, send_preparations, send_manifests, prepared_recipients, receipt_state, receipts, receipt_send_time_cutovers, invitations, invitation_cancellations, invitation_rejections, requirement_episodes, seats, membership_intervals, memberships, occupant_bindings, seat_availability, human_receipt_waivers, service_notification_publications, service_notification_recipients, digest_open_warnings, digest_open_warning_recipients, digest_programmatic_warnings, warning_close_sweeps. Conservatively fenced preparation/availability/cutover inputs remain; all actual judge reads covered. Excludes derived confirming projection, proof/fence itself, all consumer/item/epoch/outbox/seen/ledger/private progress, independently captured frontier and wake_work. No private progress self-fences.

Producer/lifecycle inventory: direct events append_event_once_at_seq/record_overdue_inner (immutable retained messages); unavailable stage_recipient and insert_publication→record_published_unavailable_conditions plus send_unit exact job tuple; requirement service_controls/canonical accept/release/retire; receipt physical/sparse/mutual ACK/waiver reconciliation and terminal state precedence; seat binding/recovery/retirement and exact membership intervals; service notification publication/recipient materialization; retained source/job/digest projections and maintenance cleanup; canonical recovery/waiver/late-publication sweep enqueue as above. Existing prepared-child cleanup removes one exact child only while unpublished/discarded; manifest publication is immutable, runtime deletion not found. Parent manifest has unbounded warning cardinality, explicitly NOT used by a new per-warning hook. These DB mutation triggers cover direct canonical writers even beyond current service entry points; future repairs cannot bypass the documented canonical sweep-chain constraint merely because hooks fire.

## Executable fixtures and honest limitations

13 private helper tests now live in existing tests/store/attention.rs; no registration/new target/public accessor. All corrected/new versions UNRUN, possible compile/setup/assertion concerns must be resolved by D without treating setup as RED:

1. required_lazy_overlay: fixed managed thread owner relation and mandatory revision++ on accepted/released transitions; retained original overlay/digest/accepted-effective assertions, no policy weakened.
2. quiet_proof: private proof no self-fence and MAX writer rollback.
3. retained_membership:257 unpublished jobs+301 retained intervals, finite Open, bounded exact witness, loss Unknown then full Absent.
4. direct_legacy_survives_canonical_lazy_churn: original actual diagnostic RED,301 physical retained intervals and real Lazy staging/publication every call; helper must Open and preserve exact witness under unrelated writes. Shared factory refactor only, original behavior retained.
5. canonical_receipt_sweep_closes_before_clear_publication: original actual semantic RED, real overdue condition and canonical waiver/sweep; undrained physical clear/receipt. Adds positive early-exit accounting13 capture+1 source+1 empty coverage=15 seeks, witness0 physical rows+1 honest empty coverage seek; covered condition never Open and converges Absent.
6. direct_association_survives_prepared_same_id: explicit retained schema coexistence (not claim post-ledger publisher admits collision), prepared association nonactionable/direct invitation actionable; exact direct job sequence mismatch removes that positive and converges Absent.
7. coverage_reserves_fallback_progress_under_source_load:64 real covered receipts/scopes using real overdue/sweep producers, concurrent301 legacy membership and real unrelated Lazy publications. Fixed per-call100/26/6400/64 assertions, monotonic saved direct cursor, multiple source/fallback quanta and finite Open.
8. all_thirteen_physical_sources_progress_through_stale_candidates: all13 physical scopes populated, deliberate stale ordinary-message/derived candidates and unpublished prepared/service entries confer no authority; source traversal+empty suspended fallback completes, every captured source high/after>0 and exhausted. This is NOT a fabricated canonical swept invitation or programmatic/native association. Genuine invitation-origin positives instead end early, as required.
9. successor_plan_and_vm:32/512 canonical retained receipt-sweep chains via real producer, explicit adjacent expected boundaries, equality-prefix covering index/no temporary sort, actual VM units measured by existing private counter, endpoint/no successor/wrong seat/empty wrong kind+episode; full bounded helper absence over coverage-heavy history. Query tests include plans AND production helper behavior, not only copied SQL text. Runtime linked-SQLite result outstanding.
10. sweep_mutations_fence:87 installed hooks plus actual INSERT/UPDATE/DELETE keyed singleton deltas in rolled-back unit scope row (no new canonical delete writer); proof no self-fence, MAX/missing/forced-negative singleton rejects sweep insertion and rolls back row. Forced negative uses temporary CHECK bypass only to exercise corruption guard, restores PRAGMA before insertion and transaction rollback after each case.
11. unavailable_exact_episode_and_late_publication: real ordinary recipient staging creates two sealed snapshots same episode; publish first, wrong-episode close leaves exact witness Open, exact close then held old snapshot publication creates canonical adjacent(0,1],(1,2] sweep extension, both clears undrained and helper converges Absent. Internal stage/publication producers run with fixture-created joined sender/recipient/preparation snapshots; full A2 service/handler admission and revision-current publication checks remain separate D integration evidence.

D still owns actual corrected compilation/focused tests, exact SQL VM/plan/counter observations, invitation producer/bootstrap/catalog upgrade39→40→41/FK/reopen/rollback, all source families' real canonical handlers, offered frontier/restart/claim/binding/cancellation/waiver/current association controls, fair last-seek/startup/lending/coverage meter stress, exactly-one-advance across every V2 path, no relevance calls in Confirm, real missingAttention/churn/logical-close GREEN, episode/outbox/replay/raw carry/completion/native composition and required clippy/harness/default checks. Added unit stress distinguishes applicable sweep families from fixed/keyed positives; it is not a passing universal fixture claim. No source approval, application, merge readiness or final token-visible readiness asserted.

## Token accounting

No new routine model-visible framing, prompts, payloads or events. Six new exceptional corruption diagnostics are27–39 UTF-8 bytes: missing relevance candidate(27), invalid relevance close sweep(29), unreserved relevance coverage(29), invalid relevance condition scope(33), invalid relevance judgment authority(36), invalid relevance representation family(39). Existing generic error wrapping remains D-owned. Commit has truthful Outside-adapter for the three files.

## Exact hashes

- `src/store/presentation_relevance.rs` SHA256 `0a3826265cdae73f956200badaecbea11f78444ae6c2b93e127508b7e1ddc2b2`
- `migrations/0041_presentation_relevance.sql` SHA256 `ca529c1cce99529f92b64c4bdeb98d860905d368fe6ac610641d5873d7a2bccf`
- `tests/store/attention.rs` SHA256 `3053bbe770b57588cc6857f8c019524e8943145cad40ac9d0ba83ec102d719e4`

## Complete additive41 object names

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
- trigger `presentation_fence_warning_close_sweeps_insert`
- trigger `presentation_fence_warning_close_sweeps_update`
- trigger `presentation_fence_warning_close_sweeps_delete`

## Sole I1 finding disposition and narrow follow-up

Formal full corrected0129 review `task-15-relevance-helper-corrected-review.md` was read in full with receiving-code-review. Its sole Spec/Quality IMPORTANT I1 is ADDRESSED in this source checkpoint, pending fresh full cumulative review and actual D execution. Parent expressly authorized only this narrow correction: Membership previously discarded currently evaluated high-water and used its captured high-water, allowing a retained/operator-repair decrease to produce a first Open that immediate exact-witness validation would reject. The actual normal warning_jobs producer inventory shows inserts capturing this high and status/progress-only updates; no normal-producer decrease or newly observed runtime failure is claimed.

The membership microstep now obtains `(current cutoff,current high)` from the existing same-transaction evaluator and scans only through `min(captured high,current high)`. A cursor already greater than this effective bound finishes negative through the existing authority-union transition, charging already performed keyed points and making no membership range visit. Equality uses the existing single empty LIMIT1 range probe. Below-bound progress still uses exactly one indexed LIMIT1, charges its actual physical row to the same100 and its seek to the same26, and stores the original captured high on continuation. Current cutoff remains unchanged; no frontier expansion, new authority, verification model, projection, schema, fence input, hook, public API, counter allowance, reservation or new model-visible string. Protected full-cost scheduling can over-reserve this cheaper early-negative branch without consuming a seek; ordinary rotation/lending and source/fallback fairness remain unchanged.

New source-only tests (all prior11 fixtures/assertions preserved):

12. `presentation_relevance_suspended_membership_respects_repaired_current_high`: use the existing301-interval direct legacy fixture, observe persisted slot8 suspended membership cursor before its only qualifying interval, then explicitly repair warning_jobs.interval_high_water to cursor-1, cursor, cursor+1 in independent databases. Every subsequent bounded pull must avoid Open and reach finite Absent. This covers beyond/equal/below effective bound; the qualifying ordinal and cursor are read from actual source/proof rows. The fixture does not pretend a canonical producer decreases high-water.
13. `presentation_relevance_suspended_membership_unchanged_high_still_opens`: suspend the identical source without repair, require finite Open, assert persisted witness is the exact qualifying interval, then require subsequent witness recheck Open. Both new tests use the existing relevance_pull assertions for cumulative100/26/keyed limits. They are UNRUN; no fabricated new runtime RED or GREEN.

Before updating current package, exact0129 report/diff/checkpoint/review were byte-preserved as `task-15-relevance-helper-report-0129.md`, `task-15-relevance-helper-0129.diff`, `task-15-relevance-helper-checkpoint-0129.json`, `task-15-relevance-helper-corrected-review-0129.md`. Original corrected-review and25ac/09d evidence remain untouched. Exact0129→newHEAD fix diff is `task-15-relevance-helper-I1-fix.diff`; the checkpoint JSON records its hash/size plus cumulative BASE308→newHEAD and all three final file hashes. This commit changes only helper and existing attention host; migration41 is byte-identical to0129 and remains additive. The full cumulative writer/callee/SQL/index/field/source/87-hook inventory above remains authoritative except the explicitly corrected membership bound. Fresh cumulative source/schema rereview, new quiescent ACK, application and D-owned actual execution are still required before promotion.
