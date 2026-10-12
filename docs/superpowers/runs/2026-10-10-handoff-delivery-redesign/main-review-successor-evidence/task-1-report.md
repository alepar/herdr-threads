# ht-0lo.20 implementation evidence

Status: IMPLEMENTED
Base: 7ea79d3d8e598b01729b61be9c0faa31a6e046f4
Head: 5aada258f553c88317945499cf0e654c5e32258b
Branch: task-ht-0lo.20
Worktree: /Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.20
Stacked: false. No rebase, following parent override.

## Change and independently grounded oracle

Read current `bd --sandbox --readonly show ht-0lo.20 --json` and comments (none), task brief, plan Design ruling, and `/private/tmp/handoff-delivery-main-merge-review.txt` I1. The independent bound comes from the established 32-item / 64-KiB delivery envelope in `src/protocol/watch.rs` and `src/service/delivery_pull.rs`, applied to actual framed UTF-8 output including indentation, header, separators and metadata. Each test's multibyte raw body is 8,000 bytes, individually inside the 8-KiB watch body limit; framing adds indentation and exceeds the batch envelope only when too many whole rows are combined.

Invitation revisions are contexts of one invitation rather than independent messages: one exact inflight object must survive, with at most one newest pending context. Same-version whole context supersedes partial; same-version partial never replaces whole. Refused completion releases the old object and removes it if a newer pending context exists. Replay from a replacement watch run does not mutate the old inflight invitation's run or consume the replacement object. Invitation completion remains exact-object removal, and no invitation ACK/acceptance is added.

Both passive and idle append paths select whole-item batches of at most 32 items and 65,536 framed UTF-8 bytes. Overflow remains queued for later appends. No model-visible strings were added or changed: maximum framed bytes per append is 65,536; repeated boilerplate added is zero bytes. The existing frame header remains once per actual append batch.

Files changed:
- integrations/claude/mod/hooks/register.js
- integrations/claude/mod/tests/delivery.test.ts

## TDD evidence

I used the test-driven-development skill. Permanent tests live in the existing `delivery.test.ts`; the ignored Node runner imports the real production module and executes selected permanent tests with only fake IO and Node assertions. It strips TypeScript using Node's built-in transform, substitutes the testing assertion/registration API, and never starts any daemon, harness, authenticated agent or child process. Node is `/opt/homebrew/bin/node`, v26.9.0; its imported production package resolves to this task worktree's exact `integrations/claude/mod/hooks/register.js`. There is no package install/setup for this lightweight test. Actual plugin validation and all module tests belong to the parent.

Command for each run (absolute task-worktree cwd, corresponding output file):
```sh
cd /Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.20
TMPDIR=/tmp/sp-bd3546df/ht-0lo.20 node --disable-warning=ExperimentalWarning integrations/claude/mod/node_modules/task-evidence/unit.mjs
```

RED before initial production edit (`red.log`):
```text
PASS a whole same-revision goal arriving during partial append is retained
PASS a changed command context arriving during complete append remains queued
FAIL bounded invitation retains only inflight and latest pending context while append stalls: 2001 !== 2
FAIL bounded invitation coalesces refused revisions and preserves whole same-version context: 2002 !== 1
FAIL bounded invitation late old-run denial leaves current whole context pending: two current IDs instead of one
FAIL framed append batches bound distinct IDs and UTF-8 bytes without losing messages: 80 <= 32
2 passed, 4 failed, 6 selected
```
Each failure is the missing bounded-state behavior, not runner startup or syntax failure. Assertions after the first failure in each test were unmeasured in RED. Existing inflight upgrade controls passed.

Expanded RED on the exact base production source (`git show 7ea79d3d:integrations/claude/mod/hooks/register.js`, restored the fix immediately afterward), `red-all.log`:
```text
FAIL passive item count: 80 <= 32
FAIL passive UTF-8 bytes: 241319 <= 65536
FAIL idle item count: 80 <= 32
FAIL idle UTF-8 bytes: 241379 <= 65536
2 passed, 7 failed, 9 selected
```
This separated the byte and item-count oracles, so a first item-count failure did not hide the byte measurement. The earlier retention failures also repeated.

Additional RED before adjusting replay deduplication (`red-replay.log`):
```text
FAIL bounded invitation replay keeps the old inflight run distinct from its live replacement: 1 !== 2
9 passed, 1 failed, 10 selected
```
The old duplicate branch rewrote the inflight object's run instead of retaining a distinct live replacement. Excluding old-run inflight invitation objects from that duplicate predicate fixes this fence.

Final GREEN command is the command above; full output in `integrations/claude/mod/node_modules/task-evidence/green-final.log`:
```text
PASS a whole same-revision goal arriving during partial append is retained
PASS a changed command context arriving during complete append remains queued
PASS bounded invitation retains only inflight and latest pending context while append stalls
PASS bounded invitation coalesces refused revisions and preserves whole same-version context
PASS bounded invitation late old-run denial leaves current whole context pending
PASS bounded invitation replay keeps the old inflight run distinct from its live replacement
PASS framed append batches bound passive item count without losing messages
PASS framed append batches bound passive UTF-8 bytes without losing messages
PASS framed append batches bound idle item count without losing messages
PASS framed append batches bound idle UTF-8 bytes without losing messages
10 passed, 0 failed, 10 selected
```
Node exits 1 on RED and 0 on GREEN; the shell display call follows with `cat`, so its outer exit status is the display command's status. Runs took under one second. `git diff --check` passed; postcommit status was empty.

## Assertions and failure controls

Queue bounds catch uncoalesced obsolete revisions. Latest goal/record-version assertions catch delivering or recording obsolete context; rejecting omitted goals catches whole-to-partial downgrade. Late denial/replay controls catch stale removal of the replacement or mutation of the inflight run. Batch byte/count checks catch either unbounded selector. Distinct-ID count, uniqueness and delivered-ledger count catch loss or repeated delivery during draining. Empty invitation ACK logs catch fabricated mail settlement, and empty passive submit logs catch a new wake. Existing upgrade tests catch failure to retain whole/current context during unresolved older append. These are behavior observations through the real core, not assertions about test helper structure.

## Test hygiene

No processes were spawned except foreground Node. Cleanup check from the task worktree initially refused its own ancestor cwd, rather than reporting leaks. Re-ran from `/private/tmp`:
```sh
cd /private/tmp
bash /Users/alepar/.codex/plugins/cache/superpowers-alepar/superpowers/6.4.2-alepar5.1/skills/super-code/scripts/stop-run-processes --check /Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.20 /tmp/sp-bd3546df/ht-0lo.20
```
All corrected checks returned 0 with empty output; no match/survived lines. No real config, temp HOME, daemon state, credentials or Herdr calls were used. Evidence and runner are ignored inside the assigned worktree. No background processes remain.

## Assumptions and concerns

Parent overrides control qualification ownership: no Cargo, Rust suite, full-plugin validation, private daemon or runtime qualification was run by this implementer. This report makes no final runtime clearance claim. The parent must run actual strict plugin validation/all module tests and compile gates, including checking TextEncoder in the real plugin engine. A single malformed item whose own frame exceeds 65,536 bytes remains queued rather than being truncated or falsely settled; canonical watch items fit individually under the existing topic/goal/body bounds. No new amendment appeared in current bead comments. The source-only scope is unchanged, and the task's original Goal/evidence was not rewritten.

## Initial implementation completion after parent engine qualification

Parent reported actual strict plugin validation PASS and real module suite 131 PASS / 1 FAIL (132 cases) on its frozen integration snapshot. I read `/private/tmp/ht-successor-queue20-real-plugin/mod.log`; it establishes actual TextEncoder availability and that the eight new controls pass in the real engine. Its failure was the unchanged `acks are capped at 100 ids per watch ack invocation` fixture:
```text
Expected: [100, 50]
Received: [32, 32, 32, 32, 22]
```
The fixture depended on an unlimited idle append completing 150 messages together. I changed its setup to a busy main turn and one successful `onToolCall` context completion, an existing delivery path unaffected by the append cap. This exercises the original ACK batching boundary directly. The exact [100,50], flattened ordered 150 IDs and empty unacked ledger assertions remain unchanged, as do the stored-backlog [100,100,30], all 230 IDs and empty unacked ledger controls. Production ACK batching and append limits are unchanged.

Parent also requested self-review of successful old-run whole replay. Existing completeness semantics suppress unchanged whole context in `rec.invitationVersions`; a successful physical old-run append in the same session generation must therefore settle an identical pending whole replay without another model-visible append. This does not justify ID-only suppression: a newer topic/command context must still display, and a successful partial append cannot settle its whole upgrade.

Added a permanent causal test before modifying successful completion. Actual RED against first task commit plus the new test (`red-whole-replay.log`):
```text
FAIL bounded invitation successful old-run whole append settles only an identical pending context: 2 !== 1
10 passed, 1 failed, 11 selected
```
The test exercises a watch run replacement with an old append still unresolved. After successful whole completion the old source issued another append of the same whole context. Completion now removes its exact object plus only noninflight invitation objects whose ID and exact contextVersion match a successfully whole-displayed item. Other inflight objects remain exact; newer context and old-partial/new-whole replacements remain queued. The permanent test covers all three outcomes independently: identical whole replay settles once, newer version displays, and partial-to-whole displays. Existing denial/run-fence controls remain.

Final focused command (same absolute task-worktree cwd and TMPDIR as above) includes the preserved ACK test in the ignored runner selection:
```sh
TMPDIR=/tmp/sp-bd3546df/ht-0lo.20 node --disable-warning=ExperimentalWarning integrations/claude/mod/node_modules/task-evidence/unit.mjs
```
Full output in `green-initial-completion.log`:
```text
PASS acks are capped at 100 ids per watch ack invocation
PASS a whole same-revision goal arriving during partial append is retained
PASS a changed command context arriving during complete append remains queued
PASS bounded invitation retains only inflight and latest pending context while append stalls
PASS bounded invitation coalesces refused revisions and preserves whole same-version context
PASS bounded invitation late old-run denial leaves current whole context pending
PASS bounded invitation replay keeps the old inflight run distinct from its live replacement
PASS bounded invitation successful old-run whole append settles only an identical pending context
PASS framed append batches bound passive item count without losing messages
PASS framed append batches bound passive UTF-8 bytes without losing messages
PASS framed append batches bound idle item count without losing messages
PASS framed append batches bound idle UTF-8 bytes without losing messages
12 passed, 0 failed, 12 selected
```
Leak checks from `/private/tmp` returned 0, no output; no processes were started besides foreground Node. `git diff --check` passed. Commit `a7d9fbf2` extends `5aada258` on the same task branch without rebase; recorded base remains `7ea79d3d8e598b01729b61be9c0faa31a6e046f4`. Two source/test paths changed, with truthful Outside-adapter trailer. Model-visible framing additions remain zero bytes and the maximum append remains 65,536 framed UTF-8 bytes. Parent owns rerun of actual engine qualification and the first task review; no review fix pass has been spent.
