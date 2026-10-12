status: clean [degraded: low coverage, plan roast skipped]
metrics: [upstream-feedback-analysis.json](upstream-feedback-analysis.json) — historical once-only analysis; nothing filed.

## Implemented

- Send-time ACK deadlines, bounded overdue discovery and durable originator warnings preserve historical deadlines and provenance. Lazy invitations support explicit timed confirmation; handoff freezes complete UTF-8 instructions up to 65,536 bytes; stable event hooks retain exact ownership. Sources: closed ht-0lo.1–.4 descendants; [run.md](run.md).
- Automatic handoff, AGY delivery, capability negotiation and bounded presentation/continuation repairs are integrated. Main migrations 1–36 remain immutable; additions are prospective 37–46. First-roast fixes .12–.15 and diagnosed sweep corrections .16–.19 landed. Sources: closed ht-0lo.5–.19; [run.md](run.md).
- Main successor .20 retains one in-flight invitation context and at most one latest pending replacement, with append batches bounded to 32 items / 64KiB framed UTF-8; .21 corrects the direct Lazy wake oracle while preserving guarded person producers; .22 corrects direct-reader guidance and changelog. Three independent CLEAN reviews, no minors or review fix passes. Ranges: 7ea79d3d..a7d9fbf2, ..8efa319f and ..045b1b65. Source: [successor ledger](main-review-successor-evidence/progress.md).
- Fresh acceptance at fb571aa8 / tree 6fa18abe: 827/827 PASS, comprising 821 focused cases in 63.124s plus six mandatory recovery cases; three scoped leak checks found zero, UUID 6a40efc8. All 37 beads closed. Source: [run.md](run.md), successor-final-acceptance; [evidence](main-review-successor-final-acceptance/README.md).
- Fresh strict consumer validation and all 133 module tests passed; all 47 private/fake driver tests passed with matching source bookends and zero scoped leaks, UUID d084132d. Controlled build 8.972s and strict lint 22.829s passed 60s budgets with exact restoration. Source: [run.md](run.md), successor-final-acceptance.

## Remaining

- Five historical nits remain unverified: invitation-wake wording, ownership markers, launch-preflight help, body documentation and unused renderer. Main's three Important findings and changelog Minor are resolved; the prior queue scope waiver no longer applies. Source: [successor dispositions](main-review-successor-punch-list.md).
- Main integration, future merged-tree verification, live uptake and native/release qualification remain separate. Main reported Codex 28/32 PASS, four FAIL and cleanup 0; another owner handles wake repair. Main/origin remain 7ac8c4bf. Sources: [run.md](run.md), release-boundary; successor dispositions.
- Historical design low coverage and authorized plan-roast skip remain qualifiers. Three successor helpers were removed normally, retaining branches; the active candidate, review artifacts and warm target remain for Main's decision. Source: [run.md](run.md), parked/resume/cleanup records.

## Gotchas & surprises

- Design review added fair-backlog bounds and ordinary-send capability negotiation. Accepted F1 overflow retains canonical body/pending ACK without claiming delivery; Q4 direct acceptance preserves independent binding cursors and invitation-event floors. Source: [run.md](run.md), design/F1/Q4 records.
- Main composition required 20 conflict resolutions and migration renumbering. Earlier permit-mutant runs never established causal RED; controlled transport evidence proved a fixture hazard without proving the original failure's cause. Source: [run.md](run.md), composition/diagnostic records.
- Bounding append exposed an ACK fixture coupled to the former append size; its exact 100/50 oracle now exercises ACK batching through context completion. Two foreign-compiler guard refusals preceded speed measurement; neither made a measured edit, and the second retained its warm-build result. Sources: successor ledger; [run.md](run.md), successor-final-acceptance.

## Entrypoints

1. Canonical obligations: src/store/effective.rs, src/store/receipts.rs and src/store/schema.rs; public negotiation: src/protocol/capabilities.rs. Sources: receipt dependency order; branch diff against main.
2. Presentation: src/store/invitation_content.rs, src/protocol/delivery.rs, then src/service/delivery_presentation.rs. Sources: invitation/composition tasks; branch diff.
3. Handoff and consumers: src/protocol/handoff/automatic.rs, src/cli/handoff.rs, src/harness/setup.rs and integrations/claude/mod/hooks/register.js; qualification driver: scripts/wake-e2e.py. Sources: handoff/hook/successor tasks; branch diff.

## Smells

- Confidence retains historical low coverage and same-family reviewers. The additional scoped audit used 11 fresh scouts, zero raw findings and no applicable panels, retaining five historical unverified nits; original round 2 convergence remains unchanged. Source: [successor audit](main-review-successor-roast/2026-10-12-handoff-main-review-successor-roast-pr-post-cap-audit.md).
- Earlier composition/helper fixes merged after one review/fix pass without rereview; sweep corrections .16–.19 were not re-roasted. Fresh 827 PASS resolves runtime acceptance while preserving historical failures. Source: [run.md](run.md), review/sweep/acceptance records.
