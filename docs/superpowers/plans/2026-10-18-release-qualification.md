# Release qualification implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task.

**Goal:** Reduce release lead time while preserving qualification and archive verification.

**Architecture:** A local Python driver sequences compiler checks and the timing-sensitive suite, then runs isolated matrix, wake and installer branches concurrently. CI keeps stable required check names with cheap docs-only skips; release artifacts are reused only if the existing build output and provenance support safe reuse.

**Tech Stack:** Python 3, GitHub Actions, Bash, Rust tooling.

**Spec:** none — assigned bead ht-z70k and handoff mQ0V4dfh9g1d.

## Global Constraints

- CI/scripts/docs only, no product code. No tag, push, stash, real user config writes, or shared Herdr server changes.
- Keep the 4-shard split and required-check names. Docs-only skips use the coordinator's narrowed explicit allowlist: root RELEASE.md, AGENTS.md, CLAUDE.md, and docs/superpowers/**. All other paths remain heavy, including docs/release.md (consumed by release document tests).
- Preserve SHA256SUMS/VERSION/TARGET verification and the existing archive layout.
- Commits carry Outside-adapter: <paths|none> and Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>.
- No live authenticated wake agents during this task; trial uses --self-test. Keep evidence directories.

### Task 1: Integrated release qualification and workflow update

**Files:** Create scripts/release-qualify and focused scripts/tests test; modify .github/workflows/ci.yml, tests/release/workflows_test.sh, docs/release.md, RELEASE.md. Modify release.yml only if reliable reuse is justified; record analysis otherwise.

**Interfaces:** driver invokes repository commands with child-specific temp roots/run IDs and retained per-step logs, then prints a single status summary and returns nonzero on any failed step. Existing release and CI workflow names remain stable.

- [x] Inspect existing workflow assertions, CI commands, smoke/wake CLI and archive build outputs before choosing implementation details.
- [x] Add meaningful driver orchestration tests (order, isolated environments, failure aggregation, newest listed release selection and self-test avoiding agents) using fake subprocesses; run them to establish missing behavior.
- [x] Implement one driver: fmt, harness-neutral, both clippy variants and default-features; prebuild test/release proof fixtures; full nextest suite alone with unique HT_LEAK_RUN_ID and leak check even on failure; candidate build; then matrix with upstream-list check and wake self-test then newest-version wake-e2e-herdr concurrently. Include the local package lifecycle gate and the three exact CI installer commands (installer_alias_test.py, installer_integrations_test.py, installer_terminal_test.py bin/herdr-threads). Give each concurrent branch unique short scratch root, state and run ID; retain evidence. A --self-test mode substitutes the authenticated wake run only. Avoid duplicate full suites and redundant cargo checks.
- [x] Add per-job docs-only filtering without skipping required checks: an always-running changed-path detector plus stable-name jobs whose heavy steps can be skipped, or equivalent verified behavior. Cover push, pull_request and manual dispatch semantics, and ensure release tags remain runnable.
- [x] Inspect CI/release build outputs. Reuse only proven same-SHA compatible outputs; if CI has no archives or incomplete target coverage, document that and leave release.yml intact.
- [x] Update current release procedure text (not historical release evidence) to start ordinary CI alongside local qualification, drop required manual ci.yml dispatch, and use the new driver. Remove the obsolete open-improvements entry as appropriate.
- [x] Run driver/workflow focused tests, exact Python installer suites where feasible, actionlint via existing assertions, and required per-change clippy/harness-neutral/fmt checks. Run a real credential-free trial if feasible, report all failures without weakening checks.
- [x] Commit and report SHA, checks, evidence paths and any limitations. Do not claim live wake qualification.

## Delivery and evidence

Implementation commits: `d623397b` (driver/filter/docs), `f12d9a66` (single CI proof), `c46e1e10` (bounded dependency caches), `3569007f` (review fixes), `0412b490` (final documentation nits). Focused driver and detector tests, workflow assertions and actionlint pass. The credential-free real trial and retained review reports are under `/tmp/ht-z70k-qualification-approved`; its final results and limitations are recorded in [CI lead-time evidence](../../validation/ci-release-lead-time.md). No live authenticated wake qualification was performed. After merging main at `de640b91e`, `7634df5f` narrows the documentation skip policy as explicitly requested; its deny-by-default detector cases and workflow checks pass.

CI compiles all four targets but uploads no release executables/archives, and release embeds a different moving manifest input. Release artifact reuse was therefore rejected on evidence; release.yml remains unchanged. Hosted warm-cache timings require the coordinator's reviewed push and a subsequent run; local warm builds do not supply them.
