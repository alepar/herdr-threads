Align help, user documentation and changelog with supported new-tab directories and deliberate pane/agy limitations.

Spec: docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/2026-10-17-handoff-profile-and-skill-guidance-design.md
Files-touched: src/cli/help.rs, README.md, CHANGELOG.md, tests/fixtures/golden/help/handoff.txt, related handoff help goldens, docs/agent-usage.md (handoff reference only)
owns: public handoff option reference, help/goldens and unreleased change summary.
consumes: public handoff directory grammar.
blocked-by ht-1urq.2: consumes public handoff directory grammar
owns: proposed unreleased minor 2.1.0 with no package version bump, tag or release.

## Acceptance
- [ ] Document --new-tab --profile-dir with --cwd example, adapter mapping/unsupported agy, absolute existing paths, frozen retry, directory disappearance and plain existing/pane refusal.
- [ ] State selected profile is declared child input subject to shell initialization, not proof of authenticated account; no logs/docs contain credential contents.
- [ ] Unreleased changelog proposes 2.1.0 for the additive feature; do not bump package version, tag, release, push or merge main.
- [ ] Update CLI help goldens and checked references; coordinate agent-usage sections with guidance leaf to avoid overwrite; relevant golden/doc checks pass.
