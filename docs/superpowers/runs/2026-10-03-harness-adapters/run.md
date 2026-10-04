# super-auto run — 2026-10-03-harness-adapters

flags: planOneShot=false skipPlanRoast=false skipCodeRoast=false autonomous=true
resumeChange: User confirmed schema allocation: CLI v19 thread names, v20 recent activity; durable handoff uses private journals. Adapter/runtime-evidence migration is v21 after CLI lands. Prototype only clearly provisional until absorb main before freeze; historical migrations immutable; exact Hermes identity/model-free labels preserved.
resumeChange: User reports main 021ac1ec with wake_batch_delay_ms default 0 in InstanceSettings/StoreSettings, explicit batching overrides and retry spacing retained, legacy HealthSettings absent-field fallback preserved. Absorb main before freeze and preserve this behavior/opt-in fixtures; no migration or immediate rebase required.
resumeChange: 2026-10-03 · "Critical schema allocation update: user-added invitation rejection belongs to the next v0.2.2 release and requires an additive immutable rejection-overlay table. CLI retains v19 thread names/v20 activity; invitation rejection receives v21. Your later v0.3.0 adapter/runtime-evidence migration must therefore be v22, after absorbing those merged changes. No historical migrations edited. Preserve invitation effective-state helpers when integrating this seam. No immediate status response/rebase needed; report only a blocker." · supersedes earlier v21 adapter allocation; active specs/bead4 use v22, invitation effective-state behavior preserved.

resumeChange: 2026-10-04 · Independent coordinator read-only source audit /private/tmp/herdr-hermes-readonly-boundary-review.md confirms no complete official nonmutating route on installed37daf85, including warm callbacks. Keep Unsupported; concrete Hermes acceptance UNMET. Practical dependency is genuine official structured readonly producer/effective-config and dispatcher timeout snapshot, or separately reviewed/measured equivalent boundary. Tempclone/changedHOME cannot qualify original; path-preserving COW is conditional/unavailable. Native negative timeout defaults30; zero disables; callbacks lack captured snapshot, so no raw-config/later-read inference. Existing safe Task11 reviewed/merged d3bd7eaf; preserve both review gates, exact source identity and evidence labels. No new run, native-source edits, external messages or experiments authorized.

resumeChange: 2026-10-04 · Coordinator v0.2.3 targeted HOLD released; released main63d02b880f73ec3ba8a1cd89c1f5752bcfec10ee absorbed into isolated branch69b405d8 after fresh CLEAN staged-tree review and36 targeted/lint/default/scopedleak checks. Optional agent.name topology fix1676bf53 and parent-owned detached writer835f4706 preserved exactly; protocol4/schema21 unchanged, adapter22 retained. Coordinator integrated2927/2927 in247s and installer201/realupgrade/leaks are coordinator evidence, not this run full suite. No main writes; final integrated sweep/publication/cleanup remain coordinator-owned.

phase: code
codeMechanism: ordinary-subagents

idea: Own the harness adapter architecture and Hermes feature; ideally adding a new harness is as easy as implementing the new interface. Audit Claude/Codex seams and authoritative Hermes integration docs, preview and approve the architecture, then implement and review in an isolated worktree for coordinator-owned integration into main. Approved scope includes a built-in registry in the same binary and the Hermes Python bridge, for the later 0.3.0 release.
branch: harness-adapters
base: main
epic: ht-3bi
spec: 2026-10-03-harness-adapters-design.md

approvals:
- design-review · approved — user approved broad architecture and Python bridge before invoking super-auto; detailed design proceeds autonomously with both reviews retained.
- top-split · auto · ht-3bi.1 LEAF, ht-3bi.2 PROMOTE, ht-3bi.3 PROMOTE, ht-3bi.4 LEAF, ht-3bi.5 PROMOTE, ht-3bi.6 PROMOTE, ht-3bi.7 LEAF, ht-3bi.8 LEAF, ht-3bi.9 LEAF

- coverage-round-1 · auto · requirements: 18 · mapped: 18 · unmapped: 0 · canonical list: coverage-round-1-requirements.md · C1/R18 applied to ht-3bi and ht-3bi.7 · two valid independent reviews
- coverage-round-2 · auto · requirements: 18 · mapped: 18 · unmapped: 0 · two valid reviews · no findings · C1/R18 closed · widening: no · targeted integration sweep ht-3bi.10 added after coverage

parked:
- native-boundary-audit · escalation · "Independent source audit on installed37daf85 found no complete official nonmutating structured producer/effective-config/dispatcher-timeout snapshot route. Concrete Hermes acceptance remains UNMET until a genuine upstream API or separately reviewed/measured equivalent boundary exists. Current Unsupported is an approved safe slice, not goal completion; no native-source edits, external requests or experiments authorized. Audit /private/tmp/herdr-hermes-readonly-boundary-review.md; both review gates retained."
- run.md · degraded-verdict · "Final integrated full-suite sweep, main merge, worktree cleanup and 0.3.0 release belong to threads-main w4:p1 by explicit user instruction; this run supplies focused checks and a frozen merge request."
- design-roast-1 · escalation · "Material dissent — runtime identity snapshot after a live Hermes source-checkout change remains unresolved. Once-per-load capture does not establish unchanged source for later lazy imports; no confirmation/native PASS asserted. Full entry in design roast1 Escalations section; retained for final report."

roast-design: 2026-10-03-harness-adapters-roast-design-1.md, 2026-10-03-harness-adapters-roast-design-2.md
roastDesignRound: 2

stepBackDesign-round-1: patch — independent registration context namespace and resolved-profile transaction ordering omissions; source-mutation escalation unresolved

Design roast1 fixes applied inline: context spelling validation/reserved Human in root/.1/.7; shared resolved-profile transaction exclusion in Hermes/.6.2/.7. Changed design/data handling requires roast2 with prior report. Historical report remains unchanged.

Design roast2 exit: clean [converged], all9scouts returned0raw; prior2confirmed resolved, runtime-source mutation escalation remains parked. Owned relay exit0/session44341 reaped. Parallelism pass next.

graph-pass: depth 11→11 · width 2.5→2.5 · applied 0 · parked 1
