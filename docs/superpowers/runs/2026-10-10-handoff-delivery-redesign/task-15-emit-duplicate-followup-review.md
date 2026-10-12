# Task 15 duplicate captured-control scoped re-review

**Duplicate captured control stamps accepted at typed completion boundary — ADDRESSED**, src/harness/agy/delivery.rs:724. Shared validate_control_stamps now runs inside valid_completion, before complete_step can dispatch confirmation or save hints/probation. The actual command validator rejects >32 stamps, malformed canonical decimal epochs and duplicate exact kind/epoch/version tuples. No authority or receipt meaning changes.

Spec CLEAN

Quality CLEAN

Scoped verdict for the frozen two-path c18-to-D followup, with one Minor coverage observation below. Prior c18 source CLEAN applies only to unchanged context; this verdict is not whole Task15 approval or readiness.

## Exact reviewed evidence

Read Task15 brief (including its latest authority), selected04 confirmation refinement, scoped SDD re-review instructions, complete followup report and every line of the fix diff. Baseline is immutable c18f3cea60af3824b059459e6233127c91cbc71c; final scope is the two frozen D file contents, not a claimed final D commit. Independently reconstructed the patch from immutable baseline blobs and final file bytes: exact SHA256 b425a8d6bc145e8ca5c717603653d23993f06272b67c03e945a3be41a4c45106.

Both baseline/final hashes match report:

- src/harness/agy/delivery.rs baseline66653edd96dbbe6bd16f941cc2883a27ecfd17cf51888639af6143df0a6f7215; final6d9ece730b07273d57577b201a41f40e614d910da060de61066d3021198c8089.
- tests/harness/agy_delivery.rs baselinec39cdf97aa74467d6d2823d31f6511e3cadacef4d6a27c06deff6ee8bed2164e; final4e20027e2e8f3f918ac9d2dfabd431ed6027c288972d9f722da8630ad85a0350.

Inspected and hash-verified all four raw logs. The first from_value(Vec<u8>) compilation failure is correctly excluded from behavioral RED. Corrected from_slice test actually failed the duplicate admission assertion after own-source build46.81s, selected1/runtime.01s. After the shared guard, own-source build18.47s and selected1/runtime.01s passed. The distinct maximal-size witness also selected1/passed1/runtime.11s. No tests were rerun by reviewer.

## Spec Compliance

The shared typed boundary refuses duplicates before canonical RPC and before local success effects, as required by04. It preserves the already reviewed status/unique exact acknowledged-subset checks, explicit-only confirmation, original pinned claim/route/boot, successful write+flush boundary, expiry/digest guards, V1 wire/hint bytes and independent expired-probation maintenance. The followup adds no Pending, piggyback, cache, file or authority.

Maximal completion geometry uses32 distinct epochs u64::MAX-n for n0..31. Every value remains a canonical20-digit u64, with unchanged cardinality and serialized width. This fixes the old invalid duplicate witness without narrowing the two4096-byte objects, claim escaping, token/digests, scalar maxima or actual16384-byte serializer bound. The executed size test supports that precise geometry claim.

## Code Quality and all test changes

New duplicate fixture prepares an actual helper page, decodes its actual typed completion, duplicates a captured stamp and checks admission refusal. It kills the observed behavioral defect; it is not a missing-API compile test. Although the focused test asserts valid_completion rather than issuing a second fake RPC, complete_step checks that same function before dispatch, so source establishes before-RPC/effect placement. Existing invalid-result/no-write/no-probation tests remain unchanged.

Added canonical native codec fixture assertions serialize actual allocated DeliveryPulledV2 pages and a WireResponse with a128-byte all-quote valid request ID plus canonical UUID identities, then validate decoding, result size and wrapper overhead. They supplement existing actual System/User final-byte and no-ACK/no-acceptance assertions. No existing assertion was removed or weakened. Their execution is not established by the duplicate/size logs supplied in this report.

**Minor — tests/harness/agy_delivery.rs:1783–1784:** new wrapper measurements use raw serde JSON response length, while encode_wire_response includes a four-byte prefix within MAX_WIRE_FRAME_BYTES. They use current PROTOCOL_VERSION, which is correct for a supported correlated response (WireResponse::correlates_to requires that version); u16::MAX is merely deserialize-valid and belongs to the separately selected conservative skeleton geometry. These assertions are useful for the current canonical page fixture but cannot alone close the governing maximum framed-envelope/512-reserve proof. The full wire gate must use actual encode_wire_response (or explicitly add4), assert full framed<=1MiB and wrapper including4<=512, and include confirmation result fields. This is an additive test-coverage limit, not a new runtime defect or weakened existing proof. Record for whole-D review.

## New breakage and remaining gates

No new Critical/Important breakage in the fix diff. The duplicate finding is addressed. Out-of-scope observations: none beyond the explicitly bounded wire-evidence limitation above.

No Cargo, source edits, application, Git mutations, tracker, Herdr or child-agent operations were performed; only this review artifact was written. Exact final two-path bytes are reviewed, and the supplied focused RED/GREEN evidence is verified, but no blanket composed PASS follows. Whole-D source/schema/interface review, all actual Hook/Watch confirmation/replay/failure and wire-envelope checks, formatting/clippy/harness-neutral/default-feature and parent/C integration gates remain mandatory before promotion or readiness.
