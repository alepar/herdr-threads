# Task 15 neutral emit helper independent source review

Final reviewed helper SHA: `f5cf90bfdeff613f4cc0dba4f33179ea63d68986`, baseline `ee1e598c3ee96566ddc9b397185cdc97c8bf6218`.

Spec Issues

Quality NeedsFixes

## Important finding: V2 page metadata is treated as emitted attention

`src/harness/agy/delivery.rs:1044` unconditionally assigns `state.seen_attention_version = Some(page.attention_version)` in every Page branch. The V2 conversion at line 994 preserves only that metadata and the text/message fields; no field proves that an Attention block carrying that version was actually emitted. The deferred completion later saves this state after successful output and any token confirmation. That verifies emission of the returned page, not emission of a control block absent from its content.

This is a concrete composed seam, not merely hypothetical framing truncation: the inspected D source `src/store/invitation_content.rs:362` replays frozen invitation-only outbox text with its version metadata, while line 359 can return an empty page carrying the current input attention version after stale descriptors are removed. Advancing local seen from those pages conflates invitation presentation/version metadata with delivered attention. A changed canonical warning must remain eligible until its actual control content is carried. The current modern fake-page success assertion also expects seen=11 from the generic TEXT fixture and does not establish a carried control block.

Hold source CLEAN until the separately settled framing/control contract supplies explicit emitted/frozen control-version evidence and the helper uses it for V2 seen updates. Add a regression that replays an invitation outbox while a newer canonical attention version exists, and another for no emitted control; neither may mark the new control seen. Preserve explicitly authorized historical V1 semantics. This review does not prescribe an unauthorized wire/API field or implement the pending amendment.

## Complete scope and immutable evidence

Read the complete brief, all report checkpoints and dispositions, and all 1420 lines of the cumulative production/test diff, including every changed test line. Independently verified all six baseline file SHA256 values against immutable ee1e blobs, all six final file values against the probation provenance inventory, and exact cumulative patch equality to Git diff. Patch: 51521 bytes, SHA256 `dac914b3fa1b36ce5fd2653adcd2ec09f6be57add2f710dc2f3c2e20befd82eb`. Helper HEAD is exact f5cf90bf. Paths are the adapter trait, neutral hook CLI, AGY delivery, existing neutral-hook and AGY unit hosts, and one additive HookOutcome constructor update in tests/cli/hook.rs.

Read the actual initial producer RED log: unique one-test own-crate build9.97s/runtime.02s, persisted Some(11) versus None before emission. Read actual probation RED: unique one-test own-crate build32.41s/runtime.05s, probation true/86401000 versus prior false/None. These are behavioral regressions, not compile/API failures. The helper's later fixtures are unrun source evidence, not GREEN. Read the two actual native renderer failures (100333/100313 bytes); their authorized production framing amendment remains held separately and is not another requested helper fix in this review.

## Reviewed behavior without further findings

The additive HookStep defaults preserve existing run bytes, produce no completion, and keep default completion inert. The neutral HookOutcome descriptor owns the original registered harness, adapter directory, endpoint, instance, boot, clock and deadline without a closure/borrowed client or harness-specific core branch. The CLI invokes adapter completion after actual write_all AND flush success, following the existing attention and lazy completions. Broken-write/broken-flush/success source controls exercise these actual writer helpers. The old watchdog and diagnostic behavior remain.

V1 retains old pull bytes and local-hint serialization, deferring offered-page seen/injected state persistence until successful output. It adds no ACK, acceptance, claim RPC, CheckIn or new V1 authority verification. V2 probes capability before progress/token calls and confirms with the frozen original claim and Hook consumer through the pinned connection; only Confirmed or AlreadyConfirmed permits hint persistence. Lost/malformed/stale/not-current replies and expired completion refuse local saving. The deadline is rechecked after canonical confirmation. Confirmation is presentation bookkeeping, not message receipt ACK, invitation acceptance or model consumption.

The payload uses typed ConversationState and optional typed DowngradeRecord, each separately checked <=4096, with <=32 tracked entries. Existing serialize_bounded pruning is reused; no arbitrary Value snapshot or new pruning policy is introduced. Six claim strings are explicitly bounded ASCII <=128; role/generation, 64hex conversation key and prior-state/downgrade digests, and optional32hex token are checked. Two nested typed records contribute <=8192, escaped claim content <=4608, three digests192, token32 and fixed fields/scalars <1024, giving conservative <=14048 under the private16384 cap. Actual encoded length is checked before output and on decode. The separate65536 opaque transport cap does not bound native output or claim route/path sizes. The maximal geometry fixture checks both actual4096-byte serialized objects,32 entries,u64 maxima, escaped claim fields, typed nested objects and rejection beyond component caps. It remains unexecuted.

New probation opening moves into postflush completion after any required token confirmation. Independent expired-probation maintenance remains in preparation as explicitly authorized, with a dedicated source control. Changed conversation/downgrade digests conservatively reject observed changed hints before writes. Both report and source honestly retain non-CAS cooperative concurrency limits and independent best-effort cross-file writes; they claim no locking, atomic pair, canonical authority or physical exactly-once guarantee.

All inherited legacy fixture adaptations explicitly simulate successful emission before expecting persistence. The original actual un-emitted regression remains intact. New controls cover confirmation loss with a fresh producer invocation and scripted canonical replay, exact frozen claims/token/consumer and no ACK/accept calls, expiration, stale replies, malformed/oversized payloads, observed hint changes and probation lifecycle. The replay fixture is not real process restart or real canonical-outbox integration evidence. No existing assertion was weakened. The inert-default test covers default complete; source inspection confirms default run_with_completion wraps old bytes with no descriptor.

## Verification boundary

No Cargo or tests were run by this reviewer; no source/Git/tracker/Herdr mutations or child agents. Only this review artifact was written. All current helper success/failure/geometry fixtures still require actual D execution after coherent integration under its sole Cargo lease. Native framing amendment, composed D whole-task/schema/interface review, and real CLI/canonical replay controls remain mandatory. This source review does not certify those deferred behaviors or clear the held renderer failures.
