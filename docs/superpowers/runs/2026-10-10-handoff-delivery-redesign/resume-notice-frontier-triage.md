# Notice frontier triage — CONFIRM

Source-only independent triage at D `69c21b95`, compared with D baseline `ee1e598c` and actual Main `026ad3ab`. Read the supplied/current AGENTS, D TRUST-POLICY, ROOT accepted bounded dispositions, minimized warning/channel scope verification, selected zero-expansion disposition in ROOT progress.md:470, and `/private/tmp/ht-resume-watch-3d06d6b8.log`. No Cargo, tests, Herdr operations, source edits, or subagents. This report is the only write.

## Finding and exact path

**CONFIRM: introduced production identity-domain bug, shared by capable Hook and Watch.** The bad argument is in `src/store/invitation_content.rs:2398–2403`, not directly in `src/service/delivery_presentation.rs`. The allocator calls `attention::settle_carried_notices(..., state.generation, claim.native_session.as_str(), ...)`; that argument is an execution ID.

- `src/protocol/authority.rs:8–16` declares `CallerClaim.native_session: NativeSessionId` and `CallerClaim.execution: ExecutionId`, distinct typed identities. Erasing both to `&str` makes the incorrect call compile.
- `src/service/delivery_presentation.rs:75–86,189–204` admits Hook or Watch and forwards the intact `DeliveryPullV2` claim through `PresentationInput`.
- `src/store/mod.rs:1287–1297` executes `invitation_content::pull` in the deciding transaction.
- `src/store/invitation_content.rs:262–269,1924–1935` checks the claim against the open canonical binding, including generation, `b.execution_id=claim.execution` and `b.native_session=claim.native_session`. Therefore `claim.execution` is already canonically validated in the transaction; no new lookup or admission mechanism is required.
- `src/store/invitation_content.rs:2199–2217,2320–2335` selects a bounded notice page and retains exactly the notices allocated to output; `2398–2403` then supplies the wrong identity to settlement.
- `src/store/attention.rs:906–928` explicitly accepts `(generation, execution)` and validates the carried oldest-first prefix. It forwards the string to `settle_offered_notices`; `883–896` writes it to `digest_notice_offer.execution_id`.
- `src/store/attention.rs:699–712` reads the frontier only when both binding generation and execution ID match the current canonical occupant. A stored native session differing from that execution makes the frontier read **zero**, not the just-carried ordinal. `informational_notice_pending` at `718–739` therefore remains true, and `notice_offer_page` at `841–848` selects the notice again.

The defect already exists at its introducing bounded-presentation commit `4c4988d9`, `src/store/invitation_content.rs:2100–2106`; it was not introduced by the recent causal Watch fixture correction.

## Policy and baseline comparison

D TRUST A7, `TRUST-POLICY.md:448–467`, explicitly permits a committed check-in **or pull** to settle the bounded carried prefix before reply, under the canonical binding's execution. The old check-in-only wording in helper comments is historical shorthand; neither policy nor production implementation makes a subsequent CheckIn necessary for this frontier.

Both `ee1e598c` and Main `026ad3ab` distinguish the identities correctly. Their V1 service passes native session to the differently specified `settle_pulled_notices` wrapper; that wrapper resolves canonical `b.execution_id` and then calls `settle_offered_notices` with the resolved execution (baseline `src/store/attention.rs:724–758`; Main `696–730`). Current D preserves that correct V1 wrapper at `src/store/attention.rs:770–805`. CheckIn also passes `claim.execution` at `src/store/seats.rs:4391–4396`. The similar V1 service argument is therefore not evidence that the new direct call is valid.

ROOT `task-15-bounded-design-dispositions.md:100,104` preserves existing ordinary/notice ledger semantics and explicitly accounts for bounded notice settlement validations. ROOT `task-15-warning-channel-minimal-scope-verification.md:13` specifically requires exact generation/execution notice frontier settlement, distinct from ACK, with frozen recovery descriptors for emission retry. Its `:35` and ROOT `progress.md:470` accept passive Clear only at a delivery opportunity, and decline additional observer/kick/schema/Refresh machinery. They do **not** accept indefinite failure to settle notices carried by admitted V2 pulls. Fixing this argument changes none of those bounds or scope decisions.

## Observed failure and blast radius

The supplied run has two recovery cases passing and the positive elected Watch case failing only at the scoped-clear wait. Its output includes the same programmatic Notice three times. The positive test's exact matched-token/control-mask proof is at `tests/integration/mod_delivery.rs:2499–2511`; later explicit ordinary ACK and passive invitation acceptance occur at `2533–2544`. This is consistent with the identity mismatch: token retirement does not repair the independent canonical notice frontier.

`src/store/invitation_content.rs:1965–1978` supplies the current frontier to canonical relevance. `src/store/presentation_relevance.rs:1063–1080,1217–1241` uses that frontier to reject already-carried notice witnesses; an ineffective frontier retains a still-applicable notice witness and prevents absence/Clear. The observed repeated notices establish further delivery opportunities in this run; the failure is not merely an unsupported demand for an idle retraction wake.

All V2 Hook/Watch notice allocations for bindings whose native session and execution differ are affected. Consequences include repeated informational offers, persistent programmatic-Warn pending relevance/Attention, starvation of later notices behind a repeatedly selected prefix, and replacement of a previously valid shared occupant frontier with an ineffective native-session frontier when a later notice is carried. CheckIn or correct V1 settlement can mask/repair the row, but the next affected V2 allocation can break it again. Coincidentally equal fixture IDs hide the error. Ordinary receipt ACK, invitation acceptance and independent open-condition/origin warning actionability remain distinct; this finding does not justify auto-ACK, auto-accept, or clearing genuinely open conditions.

## Smallest required fix and validation

Replace only `claim.native_session.as_str()` with `claim.execution.as_str()` in the direct `settle_carried_notices` call at `src/store/invitation_content.rs:2402`. The existing transaction and `scope_matches` already establish canonical execution. No schema, wire, budget, wake, confirmation-discovery, or public interface change is needed.

Add a focused V2 regression using deliberately different native session and execution IDs, for both fixed consumers: allocate a real attributed informational notice; assert the stored `digest_notice_offer.execution_id` equals the current binding execution, effective frontier covers exactly the carried ordinal, and the notice is no longer canonically pending. After exact page confirmation, another admitted pull must not create a fresh allocation of that notice. Preserve frozen outbox replay before confirmation and prove a successor execution cannot inherit its predecessor's frontier. Prefer extending existing service/store presentation tests rather than a new target.

Then rerun the already meaningful positive elected Watch case (and the two recovery controls) unchanged: actual ordinary ACK plus passive invitation acceptance followed by a real admitted Watch opportunity should yield the scoped Clear. Do not add a check-in/hook call to the fixture; it would exercise the separate correct writer and hide this defect. Parent owns all runtime validation. The one-line fix is source-supported; successful runtime closure is not claimed here.
