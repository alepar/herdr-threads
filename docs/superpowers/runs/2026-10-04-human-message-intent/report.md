status: clean [degraded: Per-tab full suite prohibited by user brief; main coordinator owns combined-tree full-suite and release validation., sweep: SWEEP DEFERRED (caller-owned)]
metrics: upstream-feedback-draft.md (local proposal; nothing filed)

## Implemented

All nine leaves and epic ht-nmp are closed; nine actual task merges landed. The independently reviewed runtime tree is 6fb781d0f026bf6bfbd18bcab5844f2570285a85; later commits contain run artifacts only. Sources: run.md, code-final-review.md, .superpowers/sdd/ht-nmp-plan/progress.md and closed beads.

- ht-nmp.1: optional recorded intent/evidence, historical compatibility, ONE additive0022/schema22/wire5 (08faf84..f790b0c). Source: task-1-implementation.md in the ignored plan workspace.
- ht-nmp.2: --user-intent query|request|rule, canonical eligibility, frozen journal/retry claims and independent priority/receipts (e274366..ad8e3fe, rebased7b873b6). Source: task-2-implementation.md.
- ht-nmp.3: deterministic source IDs, Query/Request resolution, Active Rule evidence and source/citation-ordered folding (425cbc9..c69dd4c). Source: task-3-implementation.md.
- ht-nmp.4: cumulative source visibility, local persistence, pinning/budgets and generation isolation; submission2/renderer2/thread-summary-v2/daemon-fallback-v2 (67027cf..48417df). Source: task-4-implementation.md and code-final-review.md.
- ht-nmp.5: canonical source/intent markers, actual inbox/body projections, worker/help/trust guidance and preserved literal Codex permissions (863be7a..24d9384). Source: task-5-implementation.md and task-5-review.md.
- ht-nmp.6: actual CLI configuration matrix and lifetime/receipt smoke cases (947eb87..47da473). Source: task-6-implementation.md.
- ht-nmp.8: query withdrawal/replacement and uncertain/agent-cancellation regressions (6b32580..17d8ff3). Source: task-7-implementation.md.
- ht-nmp.9: atomic persisted first-fetch bundles and exact replay after current caller/lease/generation fences; equal-time/rollback/reset/corruption regressions (cbcce82..3521c8a). Source: task-8-implementation.md.
- ht-nmp.7: complete send→B-before-A/fallback→Ready→rule-withdrawal chain (32c3f0b..78a80c7). Source: task-9-implementation.md.

Whole-epic review is ready for coordinator integration. Both design and code roasts converged; code round2 had12/12 fresh scouts, zero new findings and one previously confirmed open punch-list item. Focused final evidence includes51 intent library and6 real CLI integration cases, affected summary/version/service filters, fmt, clippy, default features and scoped leak scans. Filters overlap; no unique-test or full-suite total is claimed. Sources: code-final-review.md, task-9-implementation.md, roast-pr-2.md.

## Remaining

- **OPEN [Should-fix] src/store/summary.rs:1388 — out of scope (filtered):** completed jobs retain cumulative snapshots indefinitely, allowing quadratic durable storage growth as live ledger entries accumulate. The independent scope filter classified post-replay cleanup as a storage quality improvement outside the named attribution/lifetime/visibility goal. This is neither resolved nor waived by convergence. Sources: roast-pr-1.md, roast-pr-1-scope-filter.json and run.md.
- **SWEEP DEFERRED (caller-owned):** main coordinator owns the exact combined relay/channel tree full suite, scoped leak scan, five-minute budget, main merge and release. No release readiness is claimed here. Sources: run.md and code-final-review.md.
- Channel ONE0023/final combined wire6 await actual peer code reconciliation after relay absorption. Release HOLD remains. Source: code-final-review.md and pause-checkpoint.md.

## Gotchas & surprises

- Design roast corrected unanswered query withdrawal and equal-time/rollback fetch membership through explicit closure and persisted snapshot contracts. Sources: design roast rounds1/2 and design step-back.
- Existing retry semantics remain OperationPayloadMismatch; the plan's generic Conflict meant semantic conflict. Sources: task-1-implementation.md and friction.md.
- Copyable inbox/body required canonical claim producer/consumer repairs; public JSON inbox remains an aggregate thread view. Sources: task-5/6-implementation.md.
- Semantic closure is cooperative worker judgment; scripted cases establish structural visibility/persistence, not live-model recognition. Ordinary model-discovered asks retain the parallel visibility limit, fallback can lose semantic closure evidence, and generation rebuilds can reintroduce legacy unresolved input. Claims and receipt state remain independent. Sources: code-final-review.md and design roast2.
- Workflow was unavailable; serial ordinary-subagent chains and manual roast fan-out were used. Same-family agreement is not cross-family verification. Sources: run.md, friction.md and roast reports.

## Entrypoints

1. migrations/0022_user_message_intent.sql, src/store/schema.rs and src/protocol/summary.rs: persisted shape and public contracts. Source: ht-nmp.1 and base-to-feature diff.
2. src/store/messages.rs, src/cli/commands.rs and src/cli/journal.rs: canonical sends and retry identity. Source: ht-nmp.2 and diff.
3. src/summary/ledger.rs, fold.rs and validate.rs: deterministic ownership/lifetimes/evidence. Source: ht-nmp.3 and diff.
4. src/store/summary.rs then src/cli/summary.rs: cumulative input, frozen bundles, generation fences and public operations. Source: ht-nmp.4/.9 and diff.
5. src/protocol/output_compact.rs, integrations/skill/SKILL.md and tests/integration/summary_flow.rs: surfaces, guidance and composed flow. Source: ht-nmp.5/.6/.7/.8 and diff.

## Smells

- Task4's single bounded fix pass declined Task5-owned prose and merged with its Important finding parked; Task5 and final independent review verified resolution with the unchanged guide assertion11/11. Preserve original accounting. Sources: progress.md, task-4/5 reviews and code-final-review.md.
- The retained-snapshot storage concern remains open despite round2 convergence. Sources: code roast1/2 and scope filter.
- Combined-tree full-suite and release confidence remain unestablished by this branch. Sources: run.md and code-final-review.md.

Source filenames roast-pr-N.md/design roastN abbreviate the corresponding 2026-10-04-human-message-intent-roast-pr-N.md/-roast-design-N.md in this directory. Task reports/reviews and progress.md remain in .superpowers/sdd/ht-nmp-plan until coordinator cleanup. No task worktrees or owned helper processes were kept.
