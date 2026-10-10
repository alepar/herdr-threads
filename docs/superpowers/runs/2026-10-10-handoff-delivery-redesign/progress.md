# SDD ledger — plan: /Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/ht-0lo-plan.md

Launch: epic ht-0lo · branch super-auto/handoff-delivery-redesign · concurrency 3 · hotFileCap 2 · gate nice cargo clippy --locked --all-targets --all-features -- -D warnings + cargo fmt --check · sweep focused integration leaf only; final exact-tip readiness owned by parent after PR roast
Ruling: ht-0lo.1.2 owns shared due driver/cursor fair scheduling and combined fairness tests; ht-0lo.2.2 owns invitation-class cursor/store API and isolated due tests — overlapping design descriptions require one shared implementation — risk if wrong: integration seam rework, mitigated by combined sweep.
Ruling: parent performs final default-feature/exact-tip readiness after PR roast — explicit super-auto ordering overrides .5 leaf ordering, not its required evidence — risk if wrong: readiness remains pending until parent phase6.
Preflight: all11 leaf internals and exhaustive shared-file/interface pair table checked in preflight-table.md; no unresolved design conflicts.
Ruling: add ht-0lo.1.2 blocked-by ht-0lo.2.2 — combined fair due driver consumes invitation-class lifecycle/scanner; no reverse dependency — risk if wrong: additional critical-path time, preserves satisfiable acceptance.
Detector: round 1 — 4 ready · cap3 · peak in-flight3 · hot-file deferral ht-0lo.4.1 on src/cli/mod.rs (tasks1/2); admitted task4 handoff body instead
Task 1 (ht-0lo.1.1): implementing at957544e8 on task-ht-0lo.1.1
Task 2 (ht-0lo.2.1): implementing at957544e8 on task-ht-0lo.2.1
Task 4 (ht-0lo.3.1): implementing at957544e8 on task-ht-0lo.3.1
