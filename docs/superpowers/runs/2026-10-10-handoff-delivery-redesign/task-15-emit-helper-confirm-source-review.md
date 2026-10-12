# Task 15 emit helper confirmation-scope source review

Reviewed final SHA `c18f3cea60af3824b059459e6233127c91cbc71c`, followup base `cfe7989ae2ae443d5457f2ebbd0ffd6354e7648b`, cumulative base `ee1e598c3ee96566ddc9b397185cdc97c8bf6218`.

Spec CLEAN

Quality CLEAN

No findings in this scoped source followup. Prior cumulative cfe source CLEAN and its explicit integration/execution limits remain applicable. The original f5 emitted-version finding remains ADDRESSED; this correction further limits local seen updates to the daemon's acknowledged captured subset.

Read the complete selected confirmation refinement and final design rereview1, governing helper brief, appended final report and entire fix diff, including every changed test line. Verified refinement SHA256 `04e1055bef037fb72ca2c269ad9fc42dcbf0f1d05eae2d457ccc8d2e69691677`. Independently checked exact helper HEAD/clean status and byte equality of both artifacts to immutable Git diffs: fix13439bytes/SHA256 `9c6bc54bfe8b2647a79f27ae1587859dc0a6f59c01fc91162b2dd76e842fddc1`; cumulative97706bytes/SHA256 `689ab7f2cd4f9e85d7668c8397b1daa818ab77f14cf64601ded17a2cd6e585eb`. Only src/harness/agy/delivery.rs and tests/harness/agy_delivery.rs differ from the fully reviewed cfe tree; the other eight admitted paths are unchanged. This preserves prior whole-helper source review without pretending a descriptor-only check covers the implementation.

## Exact completion semantics

The in-memory bounded completion sends its original captured emitted_controls in explicit DeliveryPresentationConfirmV2 alongside its original claim, Hook consumer, token and absent Watch lease. It cannot acquire stamps from a subsequent pull or current metadata. The removed V2 confirmed_token initializer is gone. No Pending status, piggyback, durable completion cache, additional file, recovery RPC, ACK or acceptance path is introduced.

Before any success hint or probation write, complete_step requires the typed confirmation reply, Confirmed/AlreadyConfirmed status, <=32 acknowledged stamps, canonical exact decimal epochs, no duplicates and exact kind/epoch/version membership in the captured vector. The maximum matching work is bounded by32x32 tuple comparisons. Error, wrong result, stale/not-current status, foreign/duplicate/malformed/oversized acknowledgement or expiration refuses saving. The returned subset replaces completion.emitted_controls before the existing deadline/digest checks and apply_emitted_controls; unacknowledged and obsolete controls therefore cannot advance local seen. An empty valid acknowledgement retains existing seen state while permitting the already authorized noncontrol/injection/turn hint and probation behavior of a successful completion. No whole-token/whole-outbox completion or receipt authority is inferred.

Original route/instance/boot, capability preflight, write_all+flush callback gating, observed changed-file checks, deadline refusal, V1 bytes/old hint format, native codec validation, typed payload component/16384 limits and independent expired-probation maintenance are unchanged from cfe. Captured controls were already included in payload accounting; the followup adds no stored payload or model-visible text. Canonical response serialization and RPC reserve proof remain D-owned integration obligations.

## Full changed-test assessment

All previously scripted successful confirmations use the new bounded reply wrapper. Lost-reply/restart and scoped-control fixtures retain their behavioral assertions; the transport assertion is stronger because it now verifies exact emitted_controls. Stale replies retain empty acknowledgements and no-save behavior. Invitation-only success returns an empty acknowledgement, and control tests return the actual captured projection. Real canonical native-codec fixtures continue consuming actual DomainService replies, with no fake replacement of that production path.

The new subset fixture checks both partial acknowledgement and obsolete-empty AlreadyConfirmed. It verifies the full original captured vector was transmitted, confirms current metadata999 and unacknowledged epoch2 never become seen, and distinguishes valid unrelated turn-hint persistence from Attention evidence. Its two-Attention scripted page exercises generic client subset selection; it does not claim canonical allocation permits that episode combination.

The new invalid-result fixture covers duplicate, foreign epoch, wrong version, wrong kind, noncanonical epoch and33-stamp replies. It prepares actual probation-capable output, verifies no state file and byte-identical downgrade hint after completion, checks the exact sent captured stamp, and asserts only pull+confirm calls occurred. Existing failed write/flush, lost response, expiration, malformed payload and stale controls remain intact. No assertion weakening or implementation-mirroring substitute was found. These are source-authored assertions, not executed proof.

## Active D interface and verification boundary

Inspected actual D protocol/result/service definitions read-only: DeliveryPresentationConfirmV2 has emitted_controls; DeliveryPullV2 has no confirmed_token; CommandResult::DeliveryPresentationConfirmedV2 wraps DeliveryPresentationConfirmationReplyV2 with status and acknowledged_controls. The exact field names/types match this helper; no stale unit-result assumption remains in helper production or its fixtures. The existing protocol validator also enforces stamp shape and subset semantics. D's files remain actively edited, and its helper source has not yet been applied, so no coherent composed build/behavior claim follows.

Point-in-time SHA256: src/protocol/delivery.rs `b926e2a53afe9c6eb813122b2f9c2aa1e38b47d1986efe77303781d11c4abdf2`; src/protocol/results.rs `3fabf35ee814dfb53f8141142a5b9cb193e6873f33e8b151b1b89e030f87a51d`; src/service/delivery_presentation.rs `9d9dd64fb4395915602046384d57e62e31bf9ff68a741e7293ac240008c14abb`. These are individual observed file hashes, not an atomic D snapshot.

No Cargo/tests/clippy, source/Git mutations, application, tracker, Herdr or child-agent operations were performed. Only this owned review artifact was written. Parent must read this exact verdict before scoped application after D's new immediate quiescent acknowledgement. Actual composed D compilation/nonzero focused tests and required checks remain mandatory. Canonical original-slot masks, partial/last-token/replay/obsolete/Unknown transitions, Watch capture/flush, bounded result serialization and final D whole/schema/interface/capability approval remain separately required. Source CLEAN is not composed emit-path approval or PASS.
