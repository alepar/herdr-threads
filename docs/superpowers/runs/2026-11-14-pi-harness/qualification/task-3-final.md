# Task 3 qualification record

Qualification is bounded. The scripted native cases below have passing evidence,
and the required static/SDK/driver checks passed before the final test-only
expectation corrections. The one observed unfiltered suite failed ten tests and
exceeded the five-minute budget. Live qualification was rejected before launch
by automatic approval review; no credential was read or provider request sent.
Neither a complete functional suite pass nor live qualification is claimed.

[Selected evidence, exact hashes and actual styled safety captures](task-3-final-evidence.json)
map every required native case to its retained attempt. Full command traces,
manifests, native event logs and failed attempts remain in private qualification
artifacts. Every native attempt listed below completed owned cleanup: helper
reaped, scratch root removed and exact-run process/temp scan status 0.

## Native case mapping

`r2` used source `ae6626ff8`, driver `8d7754f8…`, fixture `f102cc5e…` and
candidate `ac61f595…`. The independently reviewed supplement used the same
candidate with frozen qualification-only timing/cancellation patch `485406dc…`
(driver `2a16e19e…`, fixture `1a818c4d…`, later committed as `3ad10e775`).
The remaining-case attempt used that committed source. The footer focus used
reviewed production patch `a9c8db29…` (committed as `a6957f160`) and rebuilt
candidate `198e81ff9307f3e88c29cd47ba8ae425d0ed4f6283e5bb80a676636058895cce`.
Full hashes are in the selected evidence.

| Required case | Credited evidence | Result |
| --- | --- | --- |
| setup | r2 | Public idempotent setup/status and foreign/edited-file preservation |
| safety | supplement | Seven observable draft/selector holds preserve exact styled screen, fresh editor/session/model/provider and pre-send run count |
| threshold | supplement | Cooldown-eligible passive p1 silence/next turn; T0/p0, T3/p3 and T−1/p9 wake |
| busy | r2 | Busy text/tool exact context delivery; no new outer run during or over 31 s afterward |
| receipts | r2 | Exact finalized native branch plus model-context receipt with cooperative delivery provenance |
| ordering | r2 | Retained page priorities 0, 1, 5 |
| invitations | r2 | Fixed-p0 wake and separate actual public acceptance |
| negative | r2 | Transformed/truncated drafts cannot earn exact-body receipt; explicit recovery remains possible |
| failures | supplement | Provider error/native abort plus specifically cancelled settle proposal; pending receipt/draft preserved and direct user recovery |
| lifecycle | remaining | Actual reload, tree, fork, new and resume |
| compaction | footer focus | Public compaction and default unknown-usage composer |
| handoff | remaining | Current native top-level public handoff; durable body in context |
| channel | footer focus | Channel-disabled native wake, no false context receipt, explicit inbox fallback and channel reload |
| named | supplement | Actual verified private tmux startup binding, p0 receipt, eligible passive p1, busy-tool p0 receipt and no trailing wake |

The final production delta after the broad candidate is the narrow default
post-compaction `?/capacity` footer recognition. The new candidate passed the
actual compaction→channel reproducer and all 15 Pi library tests, including draft
and built-in/inline selector refusals with numeric and unknown-usage footers.
Earlier broad cases are compatibility evidence on their recorded candidate;
this is not represented as a contiguous new-binary 14-case run.

## Preserved failures and corrections

The first integrated attempt compared a stale periodic editor sample against a
newer multiline screen. Its failed record is retained. The reviewed fresh
session/nonce observer fixed qualification only; equal pane bytes alone did not
dismiss a possible mutation. See [observer evidence](task-3-observer-fix.md).

The second attempt's multiline/passive negatives could be masked by a previous
wake's cooldown and receive no negative-control credit. The supplement records
actual session-specific eligibility before the sends. The second attempt also
incorrectly asserted whole-run absence when a completed turn had already legally
delivered mail before a later cancellation. The corrected barrier isolates a new
receipt-bearing target at settle; a held draft prevents legitimate Native wake,
and direct draft submission recovers it. See [timing and cancellation evidence](task-3-timing-settle-fix.md).

The remaining-case channel timeout exposed the production footer omission:
actual default `?/100k` was rejected by a parser requiring `%/`. Captured RED,
reviewed narrow fix, 15 passing fixture/adapter tests and new-binary native PASS
are preserved in [footer evidence](task-3-postcompact-footer.md).

## Unfiltered suite and bounded follow-up

Exactly one unfiltered attempt has been observed in this record, on clean
`4dbdcd5f1f7b7d602ec9dbf101be962fae91c6c6` (test formatting only after
`a6957f160`). Command: `cargo nextest run --locked --all-targets --all-features`,
cap 4. It compiled in 1m 39s, then ran 9,616 tests in **878.113 s**: **9,606
passed, 10 failed, 65 skipped**, exit **100**. Same-run process/temp scan returned
0. The five-minute runtime budget was not met and is tracked as **ht-n4xh**.

The failures were eight stale Pi registry/wake/help/setup/enrollment expectations
and two inherited-name fixtures that refused the new required canonical current
observation. Those fixture failures are distinct from production admission
behavior; their fixture-only correction is checked separately, with no production
currentness relaxation authorized.
The eight expectation corrections update explicit expected rows/strings and the
Lifecycle policy table. Golden files change only the native wake roster; no blind
blessing was used. The eight-test selection passes (1.033 s, scoped leaks 0). The first focused
attempt reached a later stale handoff-help assertion in the launch test; that
explicit Pi options expectation was corrected too. Results are recorded in the
selected evidence and do not turn the original unfiltered failure into a pass.

A comparable existing main observation records 9,590 passed/64 skipped in 951 s
at `-j4` on `09d3cf7f0`, with clean leaks:
[baseline evidence](../../../../validation/claude-wake-busy-2026-11-14.md).
The current run is faster with 26 additional tests, so it does not establish an
introduced regression. Both exceed the absolute budget. A single resource
snapshot showed four active owned test children and load 6.42/8.86/7.68 on 32
CPUs; this does not establish host/training starvation. No budget or cap was
raised. Slowest completed cases are retained in the selected evidence, including
the existing release failpoint test at 180.481 s.

## Remaining limits

External custom overlays may preserve an apparently empty default composer and
intercept wake input. Spaces-only drafts with the cursor returned to column zero
can be byte-identical to empty, even with styling. These accepted native-screen
limits remain explicit in TRUST-POLICY and the Pi guide; they are not passing
preservation claims. Unknown layouts otherwise fail closed, built-in selectors
remain guarded and the existing focused-empty-minute requirement is preserved.

Named/tmux evidence uses actual turn edges and canonical startup readiness; it is
separate from Herdr host evidence. This driver did not exercise hc runtime, and
hc has no native composer wake by contract. Scripted invitation acceptance is
covered; live-model invitation acceptance is not.

The live command was rejected before process launch because automatic approval
review could not verify authorization for this specific Pi/OpenRouter credential
use. No alternate execution or indirect credential use occurred. Live exact
receipt, provider/model continuity and real-model redundant-inbox behavior remain
unqualified. Task3 and the epic remain open pending these gate dispositions and
whole-branch review.
