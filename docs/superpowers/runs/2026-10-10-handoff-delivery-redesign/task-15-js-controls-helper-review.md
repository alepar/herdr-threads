# Task15 JS Notice and epoch-controls helper review

Spec CLEAN.
Quality CLEAN.

Reviewed exact helper final `9cb6f2c93b5d4f43f3e07a652f59e0b8999e3dcd` against immutable baseline `dd5bfe864b9f95457a2922490f611a3f5c2361be`. Read the complete 347-line two-file diff, including every test change, the helper brief/report, final selected Notice/epoch/binding-reset clauses and design rereview 2. Verified the governing proposal SHA256 `06ebb1ea10ea72d31076e671c873b45a0dc6261d285063d4103a13cb60905072` and helper diff SHA256 `6a5ba6f134e1bac083f4547331ab8498f4a8424056c3a94561e8151aa6f91a1a` by read-only hashing.

## Checks

- `integrations/claude/mod/hooks/register.js:256-313,531`: Notice is admitted as its own informational kind, uses existing frame/delivered-ID bookkeeping, forces wakes/truncated false and cannot become ACKable. It enters the passive lane unconditionally. No new receipt, acceptance, attention-version or invitation-version authority is introduced.
- `register.js:24-25,240-251`: present epochs require string-valued canonical decimal u64 syntax and range, using string comparison rather than Number conversion. Malformed present values return before any clear; absence alone retains historical global clearing. Scoped clear retracts only matching non-inflight Attention entries and deletes only `attention:<matching epoch>` from the run set. Newer/unrelated queue entries, delivered history and in-flight entries survive.
- Named cross-interface risk, passive retry versus idle wake: read complete existing `complete`, `pumpLazy`, active `pump` and `onToolCall` functions because the diff cuts those consumers. Notice uses the passive append/retry lane, is excluded from active submit/context batches, and completion only records delivered state for it. Empty ACK sets perform no Notice ACK. The two lanes retain their established scheduling; ordered producer emission does not imply global physical append order across the lanes.
- Named lifecycle risk, stale callbacks and in-flight output: read existing `dropQueued`, `startChild`, `onChildExit` and run gating. Observed close removes non-inflight entries, marks the run not live, and later runs reject old callbacks. Already-inflight successful output may settle after observed closure, matching the selected asynchronous A7 limit rather than claiming retraction or model uptake.
- `integrations/claude/mod/tests/delivery.test.ts:1824-2008`: new behavioral tests use actual harness/createCore stream callbacks and pumps. They exercise visible Notice/dedupe, unexpected ACK/wake/truncation flags, passive batch order, denied retry, adjacent epochs above 2^53, u64MAX, matching-only clears, newer delivered dedupe, same/newer in-flight preservation, malformed-present rejection, V1 omission, closure cleanup and stale old-run callbacks. Existing tests were not removed or weakened in the final baseline diff.
- Reviewed the first implementation log's two remaining failures against final fixture corrections. Explicitly denying a Notice append before expecting queued-close removal distinguishes queued from genuinely in-flight output. Testing passive Notice retry separately from active tool-context order preserves the governing historical gate distinction. Neither correction conceals a required scheduler change or downgrades the queued-close/visibility assertions.
- Read retained final log: strict plugin validation passes; 115 tests across three files pass, zero fail, runtime 0.59 s. Recorded initial 108/5 and expanded 108/7 RED counts and first 113/2 implementation outcome remain separate history. All scoped leak checks are reported passing; this reviewer did not rerun tests or leak checks. Validator marketplace-install advice is also present in the baseline log and is unrelated to this two-file helper; it is not new test failure/noise introduced here.
- No new static explanation or framing boilerplate. The report accounts for the representative Notice block at 79 UTF-8 bytes plus existing page header/separators; machine epochs are bounded at 20 ASCII bytes. V1 strings/framing and omitted-field handling retain their existing path.

## Findings and limits

No Critical, Important or new Minor source findings within this helper scope.

This is source-only approval of the two-file helper at the exact SHA above. D must still prove canonical ordered Watch projection, actual field/ID emission, byte bounds, durable token/flush/relevance semantics and its whole composed Rust/JS path. The accepted asynchronous closure window and already-inflight/appended output remain A7 limits. This review neither authorizes applying the helper without the coordinator's new immediate quiescent ACK/hash check nor certifies whole-D completion or merge readiness.

No Cargo, source mutation, git mutation, tracker/Herdr actions, children or test reruns were performed. Only this review artifact was written.
