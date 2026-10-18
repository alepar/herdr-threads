Exercise public handoff configuration and real child env/cwd with isolated stand-ins across the existing Herdr release matrix.

Spec: docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/2026-10-17-handoff-profile-and-skill-guidance-design.md
Files-touched: tests/integration/herdr_release_smoke.rs, tests/cli/topology_handoff.rs or new combined test module, scripts/herdr-release-smoke only if needed, docs/validation/herdr-release-matrix.md
owns: executable configuration smoke and release-matrix directory evidence.
consumes: end-to-end frozen new-tab launch wiring.
blocked-by ht-1urq.6: consumes end-to-end frozen new-tab launch wiring
owns: credential-free owned-fixture evidence, scoped process/temp cleanup and no real config or shared-server changes.

## Acceptance
- [ ] Invoke public CLI with scratch Codex/Claude profile directories and cwd against owned stand-ins; actual child records selected env/cwd. Include Hermes source-bound fixture and agy unsupported/cwd behavior.
- [ ] Extend herdr_release_launch_and_handoff for every listed release without exact-version operational gating; include defaults, spaces/quotes, bad directories and zero-effect pane/existing refusal.
- [ ] Exercise automatic and explicit-thread profile selection and changed caller-env/cwd retry at public boundary; no real user config/credentials, live model quota or shared server restart.
- [ ] All test processes use test_support ownership, scratch paths and scoped HT_LEAK_RUN_ID; stop/reap owned helpers/server and run scoped leak check after matrix/sweep. Record matrix failures honestly; no full-suite repetition from this leaf.
