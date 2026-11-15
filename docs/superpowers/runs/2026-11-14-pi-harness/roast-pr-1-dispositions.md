# PR review 1 disposition

The [round-one report](roast-pr-1/report.md) reviews `62bc385d0` against `91ce0cdeb`. It confirmed four findings through 16 scouts and 12 judges from the same model family. Its bounded-readiness verdict remains historical; this disposition does not declare the branch ready to merge.

| Finding | Disposition | Evidence |
| --- | --- | --- |
| F1: notice loss before model admission | Open, tracked in `ht-jr1g.5`. The neutral seam in `5b2713226` and Pi adoption in `6dc221c3d` / `b5f19ac3f` passed scoped source review and required checks. The final adapter run passed 80 callback/SDK tests. Targeted native qualification and whole-branch review remain pending. | [Design review](roast-pr-1/notice-design-review.md), [source review](roast-pr-1/notice-source-review.md), [adapter re-review](roast-pr-1/f1-extension-rereview.md), [ordering delta](roast-pr-1/f1-extension-order-rereview.md), [legacy delta](roast-pr-1/f1-legacy-hint-review.md) |
| F2: negative probe may miss its target | Addressed in `5f8fbcd34`. The fixture must observe the exact target before replacement and the replacement in actual context before asserting the receipt remains pending. | [Scoped review](roast-pr-1/f2-review.md) |
| F3: failed watch test leaks its child | Addressed in `d108f38dc`. Bounded first-row waits and guaranteed awaited teardown cover assertion, no-row and early-exit failures. | [Scoped review](roast-pr-1/f3-f4-review.md) |
| F4: missing mode fails silently | Addressed in `d108f38dc`. Only missing mode warns through native UI; explicit non-TUI modes remain inert. | [Scoped review](roast-pr-1/f3-f4-review.md) |

The [cross-harness audit](roast-pr-1/other-watch-notice-loss-audit.md) found the same pre-append reload/reconnect gap in Claude and opencode, tracked separately as `ht-0w99`. It did not establish their having Pi's overflow eviction. Those integrations are outside this repair.

The user explicitly authorized the isolated Pi/OpenRouter DeepSeek live test and one corrected-candidate full-suite run. Both remain held for the F1 repair and subsequent review. No live credential read or provider attempt has occurred. The original full-suite result remains 9606 passed, 10 failed, 65 skipped in 878 seconds with clean scoped leak checks; the ten test corrections passed focused review and checks. The absolute suite budget remains open as `ht-n4xh`, with a comparable main baseline of 951 seconds at the same four-thread cap.

The adapter source review first found a re-admission path that bypassed the outstanding-confirmation join. Its fix was independently re-reviewed, followed by two narrow deltas: preserving retained tie order after reload and discarding incompatible legacy hints before applying the new bounds. The coordinator verified the committed legacy files against the reviewed manifest and final passing checks. These observations establish scoped acceptance, not native/runtime qualification. The shared Rust packet fixtures also received [independent review](roast-pr-1/fixture-source-review.md).
