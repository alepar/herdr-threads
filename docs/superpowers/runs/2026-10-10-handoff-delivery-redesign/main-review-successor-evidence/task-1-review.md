# ht-0lo.20 — first light task review

Status: CLEAN
Base: 7ea79d3d8e598b01729b61be9c0faa31a6e046f4
Author HEAD: a7d9fbf24131aac1c31f925eac7cd5ce9dc02e7d
Parent evidence HEAD: 56c3cf4325870d9ba5cb5bd056a638a962bea2f1

## Spec compliance

✅ The current bead, brief and frozen Design ruling agree. Current `bd --sandbox --readonly show ht-0lo.20 --json` and `bd --sandbox --readonly comments ht-0lo.20` were read; there are no comments or conflicting amendments. Parent overrides restrict this to one light source/evidence review, without running tests, Cargo, Herdr or delegation.

✅ The nonempty review package contains two commits and both requested source/test paths. The supplied base is an ancestor of HEAD. `git diff 56c3cf43 a7d9fbf24131aac1c31f925eac7cd5ce9dc02e7d -- integrations/claude/mod/hooks/register.js integrations/claude/mod/tests/delivery.test.ts` returns no difference, independently tying the actual parent engine evidence to these exact task files.

✅ Invitation arrivals retain inflight objects and coalesce other same-ID pending contexts. Same-version whole context is protected from a partial downgrade. Refused completion releases only the batch's objects and retains the newest remaining pending invitation.

✅ Both passive and idle append selectors use the same 32-item / 65,536-byte cap on the actual `frame()` output encoded as UTF-8, including headers, indentation and separators. Whole overflow items remain queued in order, and existing append completion/tick paths drain them. No body truncation, new model-visible strings, invitation acceptance or invitation ACK was introduced.

✅ The old-run inflight invitation is excluded from replacement-run deduplication, so replay cannot rewrite its run fence. Successful whole completion removes its exact object and only identical noninflight context replays; newer context, partial-to-whole upgrades and other inflight objects survive. Denials continue through exact-object invitation handling in `discardStale`.

Focused checks outside the diff, each for a concrete risk:

- **Unreachable helper or uncapped sibling append:** traced `register` session startup → `createCore(buildIo(...))` → watch callback → `onLine` → `pumpLazy` / `pump` → `$.session.append`. The two `io.append(frame(batch))` callers both use the capped selector. The busy `onToolCall` context path remains distinct, preserving in-turn delivery and the ACK fixture's original batching boundary.
- **Late result mutates a replacement session/run:** read `dropQueued`, `discardStale`, `onChildExit`, `onSessionEnd` and both append completion callbacks. Existing generation guards precede completion, run changes preserve inflight objects, and denial retirement uses invitation object identity rather than shared invitation ID.
- **A single canonical item blocks the ordered batch forever:** read the existing 8-KiB body envelope and assembler truncation, bounded topic labels, and invitation rendering. Canonical supported rows fit individually; the new selector preserves their complete emitted bodies. The report honestly records that an independently oversized malformed item would remain pending.
- **Passive human direct invitations acquire wake/ACK authority:** checked canonical `direct_send::start` freezes Lazy creation policy, `InvitationContext::wakes` remains false for that policy without a required obligation, and unchanged `passive` routes such invitations to passive append. `complete` still records invitation whole display without adding mail ACKs. This agrees with the original delivery/token-efficiency contract.

No whole-branch audit or authenticated runtime qualification was performed. Required compile-only merge gates and serial integration remain parent-owned follow-up gates, not claims made by this review.

## Issues

Critical: none.

Important: none.

Minor: none.

## Test evidence

The report records the focused real-production-module Node command:

```sh
TMPDIR=/tmp/sp-bd3546df/ht-0lo.20 node --disable-warning=ExperimentalWarning integrations/claude/mod/node_modules/task-evidence/unit.mjs
```

Its retained RED evidence targets unbounded stalled/refused invitation retention, separate item/UTF-8 envelope failures, old-run replay identity, and redundant successful whole replay. The final report appendix records 12 selected tests passing, including the preserved ACK boundary control. This is plausible targeted evidence for the changed paths, with failures and environment limitations retained rather than glossed over.

Actual parent evidence was read from `/private/tmp/ht-successor-queue20-completed-real-plugin/execution.json` and `mod.log`: `nice claude plugin validate --strict <parent>/integrations/claude/mod` exits 0; `nice claude plugin test <parent>/integrations/claude/mod` exits 0, 133 pass / 0 fail, 0.778 seconds wall time. The exact changed-file match above makes this evidence applicable to the reviewed source. The run-scoped `scripts/check-no-leaked-processes --run-id 730d3008-939e-4085-9c22-ee69654a746b` exits 0, and parent HEAD/tree remain unchanged and clean afterward.

The initial actual 131-pass / 1-fail result is preserved in the report. Its ACK fixture correction changes only the delivery setup to one busy-turn context completion; the original `[100, 50]`, ordered 150-ID and empty-unacked assertions remain exact, along with the stored `[100, 100, 30]` backlog controls. It does not loosen the ACK cap or the new append cap.

Test-change commands, run in the assigned task worktree:

```sh
git diff --stat 7ea79d3d8e598b01729b61be9c0faa31a6e046f4..HEAD -- 'tests/**' 'test/**' 'spec/**' '**/tests/**' '**/test/**' '**/spec/**' '*_test.*' '*.test.*' 'test_*.*' '*_spec.*' '*.spec.*' '**/*_test.*' '**/*.test.*' '**/test_*.*' '**/*_spec.*' '**/*.spec.*'
git diff 7ea79d3d8e598b01729b61be9c0faa31a6e046f4..HEAD -- 'tests/**' 'test/**' 'spec/**' '**/tests/**' '**/test/**' '**/spec/**' '*_test.*' '*.test.*' 'test_*.*' '*_spec.*' '*.spec.*' '**/*_test.*' '**/*.test.*' '**/test_*.*' '**/*_spec.*' '**/*.spec.*'
```

Both succeed; one test file changes (150 additions, 2 deletions). The added controls cover 2,000 revisions, denial recovery, whole/partial preference, replacement-run late denial and same-context success versus changed/partial context, and independently separate passive/idle count/byte batches with distinct-ID uniqueness and delivered-ledger checks. No test is removed, skipped, or weakened.

## Strengths

The framed byte oracle uses multibyte, multiline bodies rather than raw body length. Replay tests distinguish identical whole settlement from both changed context and partial-to-whole delivery, making the causal completion rule reviewable without an ID-only shortcut.
