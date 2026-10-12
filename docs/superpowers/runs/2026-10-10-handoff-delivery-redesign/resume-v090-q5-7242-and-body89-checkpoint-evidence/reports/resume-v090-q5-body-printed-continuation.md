# Q5 body control: follow printed continuation

2026-10-11, scratch-only narrow test correction against candidate 7242. Parent reports actual focused run 35 PASS / 1 FAIL, with the remaining failure replaying step-7 bound 274. This author ran no Cargo/runtime and made no candidate/source/index/Git changes.

## Exact source proof

At tests/service/q4_body_controls.rs:330–336, BodyClient calls DomainService::handle_with_output, clones the raw MessageDetails into client.last at line334, and returns the response. Its raw continuation correctly describes the reduced effective backend request.

The CLI subsequently normalizes its local typed result at src/cli/mod.rs:1749–1755 when effective bound differs from original M. That operation does not mutate the already captured BodyClient.last clone.

The old control takes this raw clone at q4_body_controls.rs:423 and assigns its raw body_next_argv directly to the next invocation at line450. It bypasses the model-visible command after Q5 has restored M. Consequently the post-fix step-7 raw-argv274 failure does not establish that printed CLI continuation still shrinks.

The actual Body renderer emits a column-zero "more: COMMAND" at src/protocol/output_compact.rs:957–960. It does not use the History "next:" or Search "matches.next:" label. The test must follow that exact existing printed label.

## Proposed repair and preserved evidence

Authoritative scratch patch: resume-v090-q5-body-printed-continuation.patch, SHA256 426a4cfc4b454dbfb29c692e6dae0e1db12522a0783aa7d44358c37c432f0b11. Complete repaired file: resume-v090-q5-body-printed-continuation.rs, SHA256 29401c48f920c6296971166ef9a25c4722907e1e5ea48ae651c4d8812524313f.

Only the first positive test's continuation loop changes. Parse the exact stdout more command with shlex::split and feed that emitted vector verbatim into the next real public CLI invocation. Parse the public route before each invocation and for every emitted next command; require Message for the same ID with M=2048. Each actual final stdout must fit 2048 bytes. No bound increase, shortened body, repaired cursor, fixed retry or client-local synthesized next command is permitted.

Keep client.last as the independent backend evidence. Require typed total65536, offset equal to prior printed length, UTF8 boundaries, actual indented printed chunk equal to raw selected body_data and the exact original-body slice, and positive byte progress. Reconstruct the full body from actual printed chunks, retaining the final exact 65536-byte equality assertion.

For incomplete output, require exactly one printed continuation, the same raw typed opaque next cursor, cursor present/offset absent, actor/output selection unchanged, and exact raw argv token/order equality after changing only its single budget VALUE to the expected original2048. The expected vector is an assertion oracle only; it is never used as next input. Completed output must have no printed or typed continuation.

All existing flush-before-Advance evidence, partial-no-own-cursor checks, exact complete own cursor, sibling cursor independence, complete consumer-row comparisons, expected own passive/mod invalidation counters, exact one Advance with original claim/message, allowed command classes and pending invitation/noACK/noWake checks remain. Other tests and production code are untouched.

The original pre-Q5 baseline remains useful evidence: printed and backend continuations both shrank before normalization. The test's follow-path correction removes a post-fix observer error and strengthens actual stdout proof.

## Verification and review

Scoped scratch rustfmt --check passed. Independent static review requested from triage_v090_registered_agy; parent owns approval/application and the actual follow-up focused run. No compile or runtime GREEN is claimed.

Observed candidate file hash remains 4c4a0768f271b8f38681e92fd957d691a2e7809b6f9bd5acf656ddad9c51e80e. Scratch writing is quiescent.
