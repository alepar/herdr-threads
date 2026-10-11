# Task15 neutral emit helper — phase one checkpoint

Commit `e25eecffe6a9b6ad045b364501e996bfaeb9db94` on `helper-ht-0lo.8.4-neutral-emit`, baseline `ee1e598c3ee96566ddc9b397185cdc97c8bf6218`.

Only `tests/harness/agy_delivery.rs` changed: one 54-line regression. No production edits or inert scaffold were necessary. Patch SHA256 `ef0127a11410965037cda492ba8edcc28a2c017b1d2fad598030b11b5ab55cb7`.

`harness::agy::delivery::tests::hook_step_does_not_commit_offered_page_before_stdout_emission` invokes existing `AgyHookStep::run` with a decoded real PreInvocation payload, real temporary transcript and state directory, a complete scripted delivery page, and canonical CallerClaim fixture. It asserts the exact native systemMessage text first, then checks the real persisted ConversationState before the returned bytes have been emitted: no cached attention version, no injected ids, no successful-injection turn. The daemon fake supplies only the external reply; serialization, admission parsing, adapter producer and filesystem state are real. It runs no harness executable/model/network/config operation.

Expected meaningful RED: the current producer writes `seen_attention_version=Some(11)` before returning stdout. The later injected-id and turn assertions preserve the full required behavior when the first failure is fixed. No missing API or compile/setup failure qualifies as RED.

D owner entrypoint: `nice cargo test --locked --all-features --lib hook_step_does_not_commit_offered_page_before_stdout_emission -- --exact harness::agy::delivery::tests::hook_step_does_not_commit_offered_page_before_stdout_emission` (or use the filter alone if the exact full module path differs). Require nonzero selection and actual source compilation evidence.

Verification here: rustfmt on the sole changed test file and `git diff --check` passed. NO Cargo run and NO RED/GREEN/PASS claim. Failed-write/failed-flush/completion callback fixtures await the new reachable neutral completion interface after the actual producer RED; fabricating such APIs now would test missing scaffolding rather than current behavior. No new model-visible strings. No child processes or daemons started. Helper stops at phase one until coordinator clearance.
