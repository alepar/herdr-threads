# Task 15 cumulative emit helper source re-review

Final reviewed SHA: `cfe7989ae2ae443d5457f2ebbd0ffd6354e7648b`, tree `ced7b098fd31176003e9a4b9c47e5f44dbcaa16a`, baseline `ee1e598c3ee96566ddc9b397185cdc97c8bf6218`.

Spec CLEAN

Quality CLEAN

Source-only verdict for the entire admitted ten-path helper implementation. No remaining findings in this bounded scope. This is neither descriptor-only review nor approval of the composed production emit path.

## Original Important finding: ADDRESSED

The f5 finding treated V2 Page.attention_version metadata as emitted Attention. Final source restricts that inherited assignment to V1 (`src/harness/agy/delivery.rs:1173`) and makes modern Unchanged return without hint mutation. V2 validates the ordered emitted_controls projection against actual items, regenerates exact Hook text from those same items, validates control epochs and native final bytes, and only then constructs completion. `complete_step` applies the exact emitted controls after successful native emission and any required original-claim canonical confirmation. Invitation-only replay, empty obsolete replay, notices-only pages and unchanged metadata cannot advance local Attention seen state.

Exact decimal u64 epoch parsing avoids Number rounding. Scoped clear requires the same epoch and version; the epoch highwater remains after clear so delayed same/older Attention cannot resurrect it. A new greater epoch can advance seen. Binding-generation replacement clears local old hints. New source regressions preserve an existing >2^53 epoch/version while invitation-only metadata advances, verify empty/Unchanged do not mutate the saved file, test old clear/old Attention/new epoch ordering, and reject mismatched controls/text/size/codec without emission or completion. The prior generic modern success fixture now actually includes an Attention item and derives its stamps/text; it no longer claims metadata alone proves emission.

## Complete immutable review scope

Read the full brief/report including all supersessions, original review, complete final framing proposal (verified SHA256 `06ebb1ea10ea72d31076e671c873b45a0dc6261d285063d4103a13cb60905072`), and its final rereview2. Read all 2349 cumulative diff lines and every changed test line. Independently verified the cumulative artifact is byte-identical to immutable baseline-to-final Git diff: 89953 bytes, SHA256 `12f7735a2d549ed9b8370125a24213d6af5756e517ea38ee8acd9abf2e5dd9a2`. Workspace HEAD is exact and clean. All original six frozen baseline hashes were reverified; registry baseline is exact blob `5726ee32b36a71ec9cec2e27e1e7a8256b1a2f6e`.

Exactly ten paths change: src/cli/hook.rs; src/harness/adapter.rs; src/harness/agy/{decode.rs,delivery.rs,mod.rs}; src/harness/registry.rs; tests/cli/{hook.rs,hook_neutral.rs}; tests/harness/{agy_decode.rs,agy_delivery.rs}. The extra four paths are explicitly admitted by the approved framing disposition. No source write or Git mutation was performed in review.

## Descriptor, codec and replay contract

PresentationCodec is registered static data with default None; no caller envelope, arbitrary callback, adapter-local file read in core fitting, or new authority claim. ErasedAdapter forwards through TypedAdapter and Registration. AGY exposes exactly selectors0/1 through InjectionStep lookup, unsupported selectors return None. Descriptor constants are the same constants used by the capped native encoder. Source fixtures compare actual historical serde render bytes against both descriptor composition and actual encoder for empty/plain/control/quotes/backslash/CRLF/Unicode, verify registry forwarding and unsupported selectors, and exercise exact65536/65537 and expanded-NUL boundaries.

Both confirmable/passive preferences freeze before V2 pull from one local downgrade decision after independent expiration maintenance. Reply-selected supported codec controls emission; local preference is not reread to reformat a replay. Ordered items/text/stamps and advertised output bytes must agree, actual encoding is capped65536, and mismatch returns no stdout/completion with the fixed stderr diagnostic. Unknown removed codecs similarly refuse. This explicitly preserves the accepted frozen-replay tradeoff: changed or newly broken local preference cannot silently repartition/reformat an outstanding canonical token; unsupported/drifted frozen representation requires binding/operator/plugin repair. It is not proof of downstream uptake.

## Completion, compatibility and bounded state

Neutral HookStep defaults retain existing bytes/no descriptor and inert completion. HookOutcome owns its registered adapter, resolved adapter directory, original endpoint/instance/boot/clock and deadline. Actual CLI write_all AND flush success gates callback, after existing attention/lazy completions, with original watchdog/diagnostic behavior retained. Existing real writer failure controls remain intact.

V1 old pull fields, native serde bytes and absent-epoch hint format are retained. Offered-page hint writes are deferred; no empty ACK, claim RPC, CheckIn or new V1 verification appears. V2 capability probing precedes progress/token calls. Original frozen claim, Hook consumer and pinned route/boot are used for confirmation; only Confirmed/AlreadyConfirmed allows hint/probation writes, with expiry checked before dispatch and after reply. This is emitted presentation cooperation, not receipt ACK, invitation acceptance or model reading. Existing transcript-based DeliveryConfirm behavior is preserved separately.

Payload is typed: state<=4096 (including optional epoch, <=32 tracked ids), probation<=4096, five graphic ID claim fields plus checked ASCII instance<=128 each (escaped total<=2048), three64hex keys/digests192, optional32hex token32, <=32 typed canonical-decimal control stamps<3072, fixed keys/scalars<1024. Conservative sum<14560 fits16384; actual total is separately enforced before emission and on decode. Both records are nested objects, not JSON strings. Existing bounded-state pruning is reused. The maximal fixture retains both exact4096-byte records,32 entries,u64 maxima,32 controls, escaped fields and component-overflow rejection. All these Rust assertions are unexecuted source evidence.

New probation opening is deferred to completion after required canonical confirmation; independent expired probation maintenance remains independent. Observed changed conversation/downgrade file digests refuse replacement before either hint write. This is explicitly not CAS, locking, canonical authority, atomic cross-file persistence or exactly-once physical output. Report/source limits are honest. Failed confirmation, stale reply, expiration, malformed payload, retry availability and changed hint controls remain meaningful.

## Full test-change assessment

Original actual eager-seen and eager-probation RED regressions remain intact, with the previously inspected actual one-test failure evidence. Legacy fixture helpers now explicitly simulate successful emission before persistence assertions. Scripted replay is honestly a fresh producer invocation with scripted daemon replies, not real process-restart/canonical replay proof.

The historical scripted V1 oversized tests preserve full exact content and exact100333/100313 geometry. Their removed65536 claims are a documented parent-approved scope correction: eight1024NUL topic/goal pages were handcrafted daemon replies outside supported released4096-byte canonical event admission. V1 is not falsely claimed fixed or bounded. Replacement System/User tests create24 lazy invitations through actual DomainService CreateThread/Invite with300NUL per topic/goal, then run actual V2 allocation through actual AGY codec, perform write+flush before actual canonical confirmation, require <=65536 and exact advertised size, preserve every full topic/goal across bounded pages, and leave24 lazy invitations pending and the original receipt pending. The >32768 aggregate assertion prevents a tiny-page-only surrogate. These are genuine canonical-allocation source fixtures, not only descriptor arithmetic; actual execution still must establish their behavior.

No unjustified assertion weakening or implementation-mirroring substitution was found. Exact fake-page controls remain useful for transport/codec failure cases and are distinguished from actual DomainService fixture coverage.

## Point-in-time D interface inspection and limits

Read actual warm D protocol definitions, ordered projection, service and Watch/ports surfaces read-only. Required helper types/field names and signatures exist: DeliveryPullV2.render, DeliveryPageV2 ordered items/render/output_bytes/emitted_controls, typed control stamps, exact epoch validation and projection helpers. D is actively writing its own files; no coherent composed snapshot or compilation is claimed. Observed in-progress service work still omitted new profile fields and retained the old eager ledger.read_attention assignment at the inspected point. These are D-owned integration obligations, not changes in this immutable helper, and they prevent treating this source verdict as composed approval.

Point-in-time SHA256 evidence (not an atomic D snapshot): protocol/delivery.rs `ac701c523e68158f619e2ba8e8c08bd9ddbe406b4e1cb25c0ae2af41e22d6585`; service/delivery_presentation.rs `7f4fa245ffbe639549e0b7e2ba16bfae2e3897b55cce017c28527dd4a8c2852b`; delivery/presentation.rs `1c987393f1e7756fb32f97a6df95002ca042c858307c0c99e7551677bb933e86`; cli/watch.rs `8961d1fcbbd7b525a92abc81d95b4707c13816295a81da4f2bd09a384717b92d`; ports.rs `56e6030212cf2a7d6e28faddc59f7ec9c2c0139d15afd614ce320280a009ca11`. Subsequent reads already showed further ports work, reinforcing the boundary.

No Cargo/tests/clippy were run by this reviewer. Application requires D's new immediate positive quiescent ACK and scoped application, then actual D compilation/nonzero focused tests and mandatory checks under the sole Cargo lease. Final composed D whole/schema/interface review, canonical relevance/episode/outbox/service correctness, Watch/JS delivery, real CLI pinned-boot/replay failure controls and capability-readiness approval remain mandatory. This source-only CLEAN neither certifies those scopes nor substitutes for their execution.
