status: clean
metrics: upstream-feedback-draft.md

## Implemented

- ht-794l.1: Negotiated native-hook receipt completion, canonical source validation, truthful provenance and idempotent settlement, including obligations retained after leave/reinvitation. Source: `run.md`, coordinator `progress.md` and `task-1-report.md`.
- ht-794l.2: Complete/empty delivery needs no inbox or ACK call; incomplete/unavailable delivery gets routed recovery. Completion follows successful output, and attention suppression follows receipt/lazy/cursor success. Source: `task-3-report.md`, coordinator `progress.md`.
- ht-794l.3: Channel batches are bounded to 32 items/64 KiB rendered UTF-8; tails and oversized items retain recovery without premature ACK. Source: `task-2-report.md`, coordinator `progress.md`.
- ht-794l.4: Added actual EPIPE/write/flush recovery regressions, shared guidance and verification matrix; retained existing wire `how=policy`. Source: `task-4-report.md`, coordinator `progress.md`.
- ht-794l.8: Added actual native-hook coverage for once-only settlement, sibling unread isolation and independent threshold/busy/idle wake predicates. Source: `task-5-report.md`, coordinator `progress.md`.
- ht-794l.9/.10: Repaired superseded guide/current-golden/capability tests and added a configured handler probe for nonempty receipt candidates, source/session attribution and replay. Production behavior unchanged. Source: `task-6-report.md`, `task-7-report.md`, coordinator `progress.md`.
- Feature source `4274f130`: **1,572 affected Rust tests passed**; lint, neutral/default-feature/format checks and scoped process/temp cleanup passed. Source: coordinator `progress.md` → `final-verification.md`.
- Combined candidate `22924c79`: **76 Rust and 154 mod tests passed**, including actual signed-fact convergence and real two-daemon text-inbox ACK versus passive JSON. Static gates and cleanup passed; final source/adapter/evidence sent to p2p. Source: coordinator `progress.md` → `combined-node-verification.md`.
- Design round 2, code round 2 and scoped post-cap audit converged cleanly. The audit completed 10/10 scouts with no candidates/panels. Reviews used same-family GPT substitutions. Source: `run.md` and its three final roast reports.

## Remaining

- Out of scope (filtered): historical `TRUST-POLICY.md:1702` retains mod-only/body-plus-ACK wording; operative policy is updated. Full-panel Nit remains open. Source: `run.md` `scopeFilter-round-1`, PR round 1 report.
- Prior spot-check Nit: public `CompleteInboxDelivery` operation comments still describe presentation-only authority. Deliberately punch-listed; one refute-seat confirmation, no full panel. Source: PR round 1 and scoped post-cap reports.
- Main integration is the human's choice. The peer owns final integration of its p2p epic; the canonical adapter is tested in the recorded owned proof branch. All seven feature leaves are closed, with no escalations, retries, retained task worktrees or process survivors. Source: `run.md`, coordinator `progress.md`, `combined-node-verification.md`.

## Gotchas & surprises

- Design review caught quiet-hook suppression after failed completion; attention marks now follow successful completion/cursor bookkeeping. Source: `run.md` `stepBackDesign-round-1`, design round 1 report.
- Existing addressed receipts survive leave/reinvitation: requiring current joined membership was wrong. The fix preserves receipt ownership without accepting an invitation. Source: coordinator `progress.md`, design Post-Implementation Notes.
- The initial affected sweep found 17 superseded test contracts missed by leaf checks and roasts. One authorized test-only fix pass preceded the identical passing rerun. Source: `run.md` `sweepFix`, `sweep-initial-failures.md`, `final-verification.md`.
- Cold task-path builds/contention exceeded budgets; actual merged-tree lint also took 1m32s/2m04s. Final sweep-fix merge lint took about 24s, final cached feature lint 0.14s. No source-only regression was established. Source: `codeBuckets.slowness`, coordinator `Slowness:` lines, `final-verification.md`.
- Sandbox socket/process restrictions required scoped host reruns. Apparent broad-libtest success without a final summary was excluded; authoritative cleanup replaced unreliable sandbox process checks. Source: `friction.md`, coordinator `progress.md`.
- Initial/fix profiles put ht-794l.3 at a 51m critical-path bottleneck; largest rough what-if estimated 39m saved by cutting `.2 ← .1`. Model/actual ratios were 143%/108%, not measured improvements. Source: coordinator `Profile:` blocks and retained profiles.
- Sweep-fix profile's 2h23m `.10` bottleneck includes 2h05m before that task existed. It is not idle time; its largest rough what-if estimated 22m saved by halving `.2`. Latest cap 2 differs from historical cap 4. Source: coordinator profile caveat, `friction.md`.

## Entrypoints

- `src/store/receipts.rs` → `src/store/control.rs` → `src/protocol/commands.rs`/`capabilities.rs`: canonical source authority and additive public request. Source: task dependency order, `git diff main...HEAD`.
- `src/cli/hook_inbox.rs` → `src/cli/hook.rs` → `tests/hook_entrypoint.rs`: coverage and output → completion → cursor → attention ordering. Source: `task-3-report.md`, branch diff.
- `src/protocol/watch.rs` → `integrations/claude/mod/hooks/register.js` → `integrations/skill/SKILL.md`: bounded recovery and shared agent guidance. Source: `task-2-report.md`, `task-3-report.md`, branch diff.

## Smells

- The retained-membership fix merged after one task fix pass without a second task review; later whole-branch roasts and final verification passed. Source: coordinator `progress.md`, `run.md`.
- Seventeen obsolete expectations escaped regular code roasts. Fixes received fresh task review, identical affected rerun and clean scoped audit rather than regular round 3. Source: `run.md` `sweepFix`, scoped post-cap report.
- Two documentation Nits remain, with different verification strengths recorded above. Source: scoped post-cap report.
- Combined proof covers the recorded peer snapshot and reviewed seams, not paid live multi-harness network e2e, full channel-service admission e2e, the routine full suite or release readiness. Focused verification follows current AGENTS guidance. Source: `combined-node-verification.md`, scoped post-cap report.

Coordinator sources above live under `.superpowers/sdd/ht-794l-plan/`; durable verification, profiles and roast artifacts live beside this report.
