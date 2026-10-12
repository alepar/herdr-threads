# Independent scoped notice controls review — 2026-10-11

Spec: CLEAN

Quality: CLEAN

These final scoped verdicts apply to corrected helper `7e315042b7b22e2c20d442f88c172926bc65e255`, source SHA256 `814848704dc199626718e27627fbfaa2efbc3901c097c8d971720db73c5814a2`. One concrete lookup finding on the initial `064f6b0c` candidate is resolved in this source successor, as detailed below. Initial source-only CLEAN missed that defect and is superseded. Compilation, runtime, lint, and scoped leak clearance remain parent-owned and are not inferred from source.

## Scope and exact source

Independent inline scoped review under the super-roast evidence discipline, not a full multi-seat PR roast. Read both `resume-stale-attention-control-triage.md` and `resume-notice-hook-control-triage.md`, author's final `resume-watch-fix-report.md` section, actual historical run log, exact delta, real Watch reader helpers, and current D canonical settlement/renderer/CLI references. Reviewer ran no Cargo/tests/plugin/Herdr/subagents, started no test children, and made no source changes.

- Helper HEAD `2ef70d50fd4cfcfdce3189aaccde9b0be490a61a`, tree `8d687e9256f8e98a1b59b0dbca560f403d4683c5`, observed clean.
- Helper base `77054fdfc41ff4ab9ed83032a1db9057d0b6243c`; baseline integration SHA256 `a7626b7fdf09c2590a1d4e34b4983e89f7f8d87b7777e63b2639fd7fec1ac4e8`.
- Actual D HEAD `7bdb3678f8b5ba10a5b51a7bdb6aed2fa163f7d3`, tree `899d0e5fb42e8f645879c9c5caa14265064c88ed`.
- Reviewed integration SHA256 `064f6b0cf6fe4772ad200db851db7836c61eeb5bbf8b3983f554f5389cf89a46`.
- Exact delta: 150 insertions, 44 deletions, limited to `stale_attention_is_retracted_after_accept_and_after_a_notice_offer`, `notice_published_while_live_reaches_the_next_pre_tool_use_once`, and their adjacent comments.

D initially had the mapped baseline file clean. During review the parent transferred the reviewed bytes: D then had only this integration file modified, with the exact reviewed SHA and delta. Helper production baseline is older; only this file is approved for transfer. Reconnect, production, activation recovery oracles, and relay gates are untouched.

## Actual failure boundary

Direct inspection of `/private/tmp/ht-resume-activation-082385c7/nextest-run.log` confirms 294 executed, 291 PASS, 3 FAIL. Stale-attention failed waiting for Attention after a bare modern Lazy invite; its actual stdout contains the passive invitation with `wakes:false`. Live-notice failed polling Hook for `offered notices: 1`; that historical test did not capture its particular Notice stdout, so the review does not claim that delivery occurred in that failed run. Reconnect is a separately owned failure and is not weakened by this patch. Those results belong to the historical source, not the corrected fixtures.

Current D `src/store/invitation_content.rs:2396` settles carried informational notices using `claim.execution` before the real Pull response. The real renderer emits Notice warning/thread/body fields and `notice:<warning>` ID. Therefore a later Hook must not repeat a notice already carried by Watch; a quiet Hook alone would be insufficient positive proof. Both updated tests now establish actual Watch output and canonical settlement first.

## Spec review

Stale-attention preserves the actual first invitation/seat acceptance sequence, changing only the invitation's policy to explicit `--require-ack`, a genuine confirming relevance source. It records real Attention and requires the subsequent clear's presentation epoch to equal that Attention. No synthesized control or production relevance expansion is introduced.

Its notice half retains the real reserved-service Ensure/Invite/accept/Notify producer. The new read-only projection waiter observes actual attribution to B before starting replacement Watch; it does not seed a notice/frontier or invoke a repair hook. After real Attention, the Notice lookup checks retained `seen` first because Notice may precede Attention in one page; fallback waits for actual matching Notice. Both paths retain all consumed lines. Nonempty emitted warning ID, exact schema/frame ID/body command, absent ACK field, canonical recipient/thread, matching current binding generation/execution, distinct execution/native-session, and positive ordinal covered by frontier are required before any Hook. Its clear is also scoped to that real Attention. The later successful Hook must not duplicate a notice summary, and channel/child liveness plus zero mod ACK events remain checked.

Live-notice keeps the actual connected Watch, service publication, and setup acceptance. It records the host mark immediately before Notify, then waits for actual matching Watch Notice before invoking either Hook. It applies the same exact warning/body/current-occupant frontier proof before the Hook can change anything. Both real PreToolUse calls must exit zero without digest, pending-mail command block, or repeated notice-offer summary. This establishes the intended causal race winner instead of accepting either unseen consumer outcome.

The live duplicate predicate was checked against `Watch::assert_no_line`: that helper visits retained `seen`/pending lines once and subsequently consumes new lines. The stateful count permits exactly the already observed first Notice and rejects any second for the exact warning; the final count must be one. No stdout is discarded or filtered out of storage. Existing QUIET bounds the observation; `assert_no_prompt(mark, Duration::ZERO)` then checks all host calls since publication through that completed window without adding another sleep. Zero mod ACK events and live/running assertions are retained.

## Quality and remaining gates

The canonical SQL queries require actual `digest_notice_offer` and actual projected warning rows for the exact emitted warning/recipient, joined to the current open binding; missing rows fail `query_row().unwrap()`. Explicit equality assertions reject a frontier keyed to the wrong generation or execution, and `ordinal>0 && frontier>=ordinal` avoids empty/negative-frontier success. These checks precede Hook invocation, so the hook cannot repair a failing positive witness. Existing setup/explicit acceptance actions are preserved; no extra receipt ACK, synthetic clear, CheckIn bypass, timeout constant increase, new process helper, or production relaxation is added. Source-only observations do not prove live model consumption.

Historical selectors remain stable for the inventory. Commit has truthful `Outside-adapter: tests/integration/mod_delivery.rs`. Added literals/comments are test diagnostics or expectations, not production model-visible framing. Reviewer observed helper `git diff --check HEAD^ HEAD` success; author records corrected edition-2024 formatter success after an initial edition-2021 rejection. Neither is runtime evidence.

Parent must commit/freeze exact D transfer, enumerate and execute the two retained selectors in its warm lane, run scoped leak checks after failure or success, and complete applicable per-change and broader affected gates. Any failure of physical Notice/Attention/clear or canonical frontier must be diagnosed without weakening these predicates. This scoped verdict covers only the exact reviewed file bytes above.

## Actual two-case result and resolved scoped-clear finding

Directly read `/private/tmp/ht-notice-controls-green.log`, SHA256 `c41d467489073ca8ab789b29378c37711cdc4bc295651a2ab04d954b8e25732e`: exact two executed, live Notice PASS 3.481 s, stale control FAIL 1.998 s, total 3.484 s. Stale failed at its epoch equality with actual clear `1` and expected Warn Attention `2`. Parent/author records scoped zero leaks for UUID `bbd91d44-15ec-4b60-bdcf-c6f1e055da73`; reviewer ran no leak scan. This is a fresh runtime finding against `064f6b0c`, not a production-frontier failure.

Confirmed finding on the initial candidate: `stale_attention_is_retracted_after_accept_and_after_a_notice_offer` selected the first retained `attention_cleared` by kind alone, and its future waiter also accepted any clear. Replacement Watch can legitimately replay the earlier invitation's epoch-1 clear before the notice's epoch-2 Attention, so all prior stdout must remain retained while lookup is scoped to the newly observed Attention epoch. The initial source review incorrectly missed this retained replay; its no-finding statement does not survive the actual evidence.

Fresh rereview of exact helper `2ef70d50` → `7e315042` delta shows 8 insertions/6 deletions only in this lookup. New shared predicate requires both `kind == attention_cleared` and equality to the actual emitted Notice Attention's `presentation_epoch`, and is used for retained `seen` lookup and future `wait_line`. The final epoch assertion remains; older clear lines are neither discarded nor treated as erroneous. The actual warning/canonical frontier proof still precedes Hook, the first invitation scoped clear is unchanged, and passing live Notice test bytes are unchanged. No timeout growth, source authority mutation, fake output, or production change occurs.

Observed helper `7e315042b7b22e2c20d442f88c172926bc65e255`, tree `8e79ee445b16a43a711cdf499d962501e75d41fc`, clean, source SHA256 `814848704dc199626718e27627fbfaa2efbc3901c097c8d971720db73c5814a2`. Read the appended author report containing actual failure, exact mapping, and QUIESCENT declaration. This resolves the concrete finding by source inspection; it does not claim a fresh passing runtime. Parent must transfer only this file and rerun the two exact cases with scoped cleanup before reporting runtime clearance.
