# AGY final-C v0.9.1 regression diagnosis

Source-only diagnosis; no test, Cargo, runtime, Herdr, tracker, Git mutation, source edit or delegation. The only writes are this scratch report and the unapplied patch `final-C-v091-regression-agy.patch`. Runtime results below are read from existing parent evidence, not newly executed results. Candidate supplied by parent: `3d23527675170cacaec470aa13b3baf53688d4ba`.

## Confirmed cause

The previously green AGY producer/consumer test still reads the old direct-send JSON report `{thread,message,selection}`. Main v0.9.1 deliberately changed the public send contract to `{version,result:{kind:"message_sent",data:message,thread}}`. Composition correctly preserves feature selection as `result.selection`. The test retained the old paths and fails on its first JSON assertion; this is a test adaptation omitted during phase-6 upstream composition, not evidence of failed canonical Ensure, body publication or Hook delivery.

Evidence:

- `/private/tmp/ht-final-C-d8427582/focused-results.json:1494–1496` records exact binary `herdr-threads::hook_entrypoint`, case `agy::activation::modern_direct_ensure_lazy_pair_reaches_actual_turn_hook_without_settlement`, `PASS`.
- `/private/tmp/ht-final-C-v091-3d235276/nextest-run.log:316` records the same case FAIL. Lines332–335 locate `tests/hook_entrypoint/agy.rs:1642:9`, actual `Null`, expected canonical thread `tTUnFyrBO`. Thus execution already passed the actual Ensure capture, Lazy policy and invitation assertions through line1641, but has not reached the subsequent Send and Hook assertions.
- Immutable old source `f4b74f311b28c39eec0c1a4a8d726c1fbd8e5c10:src/cli/direct_send.rs:453–458` emits top-level `thread`, `message`, and feature `selection`.
- Immutable Main `20e7ca73e41a9449376169ab6875c0675da02edc:src/cli/direct_send.rs:392–404` emits a serialized `CommandResult::MessageSent` inside the versioned envelope, with `thread` added inside `result`.
- `git diff f4b74f311b28c39eec0c1a4a8d726c1fbd8e5c10 495703bd262843558cedd6b974b0d0b9216c40e6 -- src/cli/direct_send.rs` shows exactly the JSON-envelope adaptation as the entire file difference. `git diff Main20e7 495703bd -- src/cli/direct_send.rs` shows retained feature policy/selection and `result["selection"]` added to Main's unchanged JSON envelope.
- Immutable producer blobs in `495703bd262843558cedd6b974b0d0b9216c40e6` and candidate `3d23527675170cacaec470aa13b3baf53688d4ba` are identical: `afcea9b09d6b1429d7c4359a41fc479cc7219358`. Current source locations: `src/cli/direct_send.rs:453–472`.
- Immutable `tests/hook_entrypoint/agy.rs` blobs in old f4b74 and candidate3d are identical: `b79d02dcbde90118d1c727e9bde699346bcc9c51`. Old top-level assertions at1642–1644 and1670 therefore survived unchanged.
- Independent contract: `src/protocol/output.rs:253–269` encodes version+result; `src/protocol/results.rs:13,95` defines `kind:"message_sent"` / message-ID `data`; `src/protocol/wire.rs:37` defines protocol version7. `tests/cli/direct_send.rs:1890–1924` compares direct output to ordinary send, checks canonical message ID and feature created/Lazy selection, and decodes typed `MessageSent`. That exact contract test is already PASS at current `nextest-run.log:800`.

## Minimal patch and preserved oracles

The patch changes only `tests/hook_entrypoint/agy.rs`, within the failing test. It explicitly asserts the version constant, requires an object-valued `result`, pins `kind` to `message_sent`, compares `result.thread` and `result.selection.invitation` to the captured actual Ensure result, retains the literal `created` outcome assertion, compares `result.data` to the captured actual Send message ID, and independently decodes the result as `CommandResult::MessageSent`. No fallback to old fields is introduced. Selection is checked explicitly because typed MessageSent decoding alone does not prove it.

The patch preserves the exact one-Ensure/one-Send counts, origin/recipient/policy assertions, actual body and Lazy send assertions, absence of separate Invite, canonical pair/topic/goal/invitation/membership/lazy/binding/receipt/wake queries, binding cursor diagnostic, actual registered PreInvocation Hook V2 Pull, physical native output bytes, invitation context and parsed acceptance command, exact message body/ID/sender/length, no ACK requirement, bounded page/output checks, explicit presentation confirmation and canonical token/completion checks, unchanged invitation/membership/lazy/binding state, and absence of Accept/ACK/settlement calls. Everything after the producer assertions is byte-for-byte untouched.

Same-file caller audit: only the failing case uses direct `send --seat` (lines1595–1597). The sibling at514 uses ordinary `send <thread> --lazy` and the existing `data()` helper, already matching the ordinary versioned contract; it is not the direct-send producer and needs no change. No other test is patched.

This diagnoses the known early assertion failure only. It makes no claim that the downstream Hook/state oracles pass on candidate3d, nor that the candidate's other failure is repaired. Parent owns application, formatting/lint, selected verification and any authorized final-C rerun. No production contract, visibility, fallback, SQL, ancestry guard, selection inventory, test scope or waiver changes are proposed.
