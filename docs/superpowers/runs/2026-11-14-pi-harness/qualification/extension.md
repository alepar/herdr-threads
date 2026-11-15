# Native extension task evidence

Scope: integrations/pi only; Rust registration, canonical daemon integration, real TUI and live provider qualification are separate gates. No credential or real profile was used.

Implementation commits: `dc581e95f`, `6bcff9047`, `aa744e7a4`. Final extension source is the last of these. The first review found multi-ID urgent reoffer suspension and detached-watch reaping defects. Fix round 1 resolved them but introduced a recovered-owner wake regression; fix round 2 resolved that regression. [Initial review](extension-review.md), [first re-review](extension-rereview-1.md), [final scoped re-review](extension-rereview-2.md).

The final review approved the frozen code conditionally on validation and patch identity. The coordinator verified both conditions: final **48/48 tests passed**, scoped cleanup passed, and `git diff -U10 8ba848027 aa744e7a4 -- integrations/pi` has SHA-256 `760e9475b61399330a38b645072b88097946c94b9f856a7b820c9107e305fd9b`, identical to the reviewed snapshot. Task ht-jr1g.3 is accepted; this is not integrated harness or merge approval.

## Validation

All commands used the required shared heavy/toolbox wrapper. The final affected command was:

```sh
node --test integrations/pi/extension/tests/extension.test.mjs integrations/pi/extension/tests/sdk.test.mjs
```

It passed 48 tests, zero failures/skips/cancellations, in 2.417 seconds after waiting for a shared slot. Scoped run ID: `pi-task1-fix2-20261114-1634`. The final guard/cleanup result, summarized:

```text
check-harness-neutral: ok — 0 violations, 87 allowlisted, 53 entries
check-wire-compat: ok — 0 violations, 0 stale, 456 fields, 24 variants, 274 baseline entries
no leaked test temp files (no ledger for this run)
no leaked test processes
```

Node fixtures use isolated worktree scratch with finally cleanup; the no-ledger result is explicit, not a claim of Rust scratch-ledger coverage. No fixture scratch or runtime child remained. The one worktree-local SDK dependency directory is retained for subsequent integrated qualification and must be removed at whole-task cleanup.

Initial task checks also passed formatting and all-target/all-feature locked clippy with warnings denied. Clippy took 87 seconds on the rebased source tree, exceeding the one-minute target; no Rust source was changed by this task, so this is not an attributed pi Rust regression. These checks were not redundantly rerun for subsequent JavaScript-only fixes. Neutral/wire checks and scoped leak checks were rerun after the final fix. No full Rust suite was run for this isolated task.

## What the tests establish

Callback/native-child tests exercise role/mode/owner exclusion; priority/deadline/stable-tie ordering; exact finalized-branch plus model-bound-context receipt evidence; truncation and invitation exclusion; all overflow/replay-debt paths; partial ACK release; stale lifecycle/receipt callbacks; completed-only continuation; rejected and multi-ID reoffer suppression; bounded wake-history saturation; empty-editor/prompt/idle checks; and stop/replacement joins through shutdown/reload.

The real SDK with scripted provider exercises native default-extension loading, completed turn/pre-settle continuation, error/abort/cancellation, transformed drafts, rejected urgent content, content-free self-wake with selected model/provider preserved, actual session.reload(), and accepted-but-unobserved urgent recovery. The latter requires exactly one successor run, one native body frame, and no ACK before successor context evidence. SDK disposal alone does not emit native shutdown, so fixtures drive that event; actual SDK reload exercises native shutdown/start ordering separately.

RED/GREEN evidence for each review regression is retained in the controller's task report; the committed causal tests reproduce those boundaries. The full final TAP output was inspected before task acceptance.

## Limits

SDK/callback tests do not establish real TUI draft/modal behavior, canonical daemon admission/ACK/replay, public setup/launch, native process fingerprinting or live-provider behavior. Watch tie-breaking inherits stable canonical arrival because the wire has no creation key. At blocked-attention history saturation, automatic wake degrades conservatively until a permitted user/owner/channel transition; canonical mail remains pending. Native current-branch lookup scales with the exposed branch. Context/branch observation remains cooperative evidence, not provider/model consumption or disk-flush proof.
