## Task 3: Stable native event dispatch and command generation

filesTouched: ["src/cli/hook.rs", "src/cli/commands.rs", "src/cli/mod.rs", "src/cli/setup.rs", "src/harness/setup.rs", "src/harness/claude.rs", "src/harness/codex.rs", "tests/cli/hook.rs", "tests/cli/commands.rs", "tests/hook_entrypoint.rs", "tests/harness/setup.rs"]

bead: ht-0lo.4.1

Dependencies: none

### Full bead text (canonical snapshot)

```json
{
  "id": "ht-0lo.4.1",
  "title": "Stable native event dispatch and command generation",
  "description": "owns: hook-event HARNESS EVENT parser/registered-codec dispatch and stable per-event shell command grammar; consumes: existing harness event registry and canonical routing. Files: src/cli/{hook,mod,setup}.rs, src/harness/{setup,claude,codex}.rs, registry declarations/tests. Keep legacy hook syntax; validate invalid pairs before effects; command stability includes executable/route/harness/event only. Exercise declared Claude/Codex events with fixtures through dedicated entrypoints; retain Hermes callback and bundled mod route compatibility.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--hooks-design.md",
  "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass.",
  "status": "open",
  "priority": 2,
  "issue_type": "task",
  "owner": "235553+alepar@users.noreply.github.com",
  "created_at": "2026-10-10T22:48:17Z",
  "created_by": "Alexey Parfenov",
  "updated_at": "2026-10-10T22:48:17Z",
  "labels": [
    "sp:ht-0lo"
  ],
  "dependencies": [
    {
      "id": "ht-0lo.4",
      "title": "Stable hooks",
      "description": "owns: native event command surface, exact command generation, owned migration/unsetup/status/canary/launcher consistency; consumes: existing registered harness codec declaration. Files: src/cli/{hook,setup,mod}.rs, src/harness/{setup,claude,codex}.rs and setup submodules, installed manifest/canary/tooling tests and docs. Exercise Claude/Codex per-event dispatch/setup repeat/migration/unsetup in isolated config; verify Hermes and mod route remain compatible before integration.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign-design.md",
      "acceptance_criteria": "Implement the corresponding spec contract and exercise all configurations named in this description with isolated owned children before terminal integration. Run focused tests and mandated per-change lint.",
      "spec_id": "docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--hooks-design.md",
      "status": "open",
      "priority": 2,
      "issue_type": "epic",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:45:47Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T22:57:33Z",
      "metadata": {
        "sp_depth": 1,
        "sp_order": 3
      },
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "parent-child"
    }
  ],
  "parent": "ht-0lo.4",
  "dependent_count": 2,
  "dependency_count": 1,
  "comment_count": 0,
  "revision": "735408300042819471"
}
```

### Acceptance criteria (verbatim)

Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass.

### Global constraints (verbatim project instructions)

##### Agent guidelines for herdr-threads

###### Development rules

- Per-change check: `nice cargo clippy --locked --all-targets --all-features -- -D warnings` (clippy already type-checks every target; a separate `cargo check` is redundant). Before a merge, also `nice scripts/check-default-features` (a default-feature `cargo check --all-targets` that fails on any warning; CI runs the full default-feature clippy job as well); format: `cargo fmt`.
- Tests: run the tests relevant to your change (`nice cargo test --locked --all-features <filter>`); the full suite is `nice cargo nextest run --locked --all-targets --all-features` (cargo-nextest: one process per test, every binary in one pool; test groups in `.config/nextest.toml` keep the budget- and latency-sensitive chains one at a time; `nice scripts/full-suite-gate N` runs it N times in a row with leak checks, and `nice scripts/flake-hunt` hunts flaky tests). A test that must not overlap others of its kind gets a nextest test group, not only an in-binary mutex (a mutex does not span processes).
- Never restart, stop or kill the shared Herdr server, and never close Herdr workspaces/panes you did not create. Tests that need Herdr up/down use an isolated named Herdr test session (see `scripts/lib/isolated-herdr.sh`).
- Never write to the real user config: `~/.claude`, `~/.codex`, `~/.aisw`. Tests use isolated HOME / CLAUDE_CONFIG_DIR / CODEX_HOME under a temp dir.
- Never run `git push` or `git stash` (the stash stack is shared across worktrees). Commit on your own branch.
- A finished task stops its processes: every daemon, private Herdr server and helper process a test or script starts is stopped before the task reports done. Spawn test children with `herdr_threads::test_support::spawn` (`spawn_owned` / `command` / `tag`); never a bare `Command::spawn` in `tests/`.
- After a full-suite run (and after each run of the 10-run gate or the integration sweep), run `scripts/check-no-leaked-processes`. Export `HT_LEAK_RUN_ID=<uuid>` before the suite and pass `--run-id "$HT_LEAK_RUN_ID"` to it, so another worktree's live run is never flagged. A non-zero exit lists each leaked pid; stop them (or rerun with `--kill`) and fix the test that leaked them.
- Test targets are listed explicitly (`autotests = false`). A new suite that keeps no process-wide state goes into `tests/combined.rs` as a `#[path]` module (one binary, one link); a suite that redirects stdio, installs a panic hook, runs an in-process daemon, arms failpoints or changes the umask gets its own `[[test]]` entry. A target that uses `herdr_threads::test_support` (including `test_support::spawn`) needs `required-features = ["test-support"]`; otherwise `scripts/check-default-features` and the default-feature clippy job fail.
- The daemon's owner watch exists only in `test-support` builds: always test with `--all-features`.

###### Trust policy

[TRUST-POLICY.md](TRUST-POLICY.md) is normative for seat continuity, caller attribution, receipt provenance
and operator repair. Read it before changing anything in `src/identity/`, `src/store/seats.rs`,
`src/store/receipts.rs`, `src/store/control.rs`, `src/protocol/authority.rs`, `src/cli/me.rs`,
`src/cli/hook.rs`, launch or wake.

- Do not add adversarial caller verification; the model is cooperative and same-user.
- Record claims honestly: a new way to attribute an action needs a provenance value defined in the policy.
- Never merge seats, never move a seat or end a binding on heuristic evidence.
- Decide in the daemon against the canonical view (A2); client-local files are hints, never authority.
- A change that weakens an invariant or adds an accepted limit updates TRUST-POLICY.md in the same commit.

###### Speed budgets

- The full test suite must stay under **5 minutes** wall-clock (`cargo nextest run --locked --all-targets --all-features`). Treat a regression past it as a bug: fix slow or serialized tests rather than raising the budget.
- An incremental build must stay under **1 minute** (edit one source file, then `cargo build` / `cargo test --no-run`), and so must the per-change lint (`cargo clippy --all-targets --all-features -- -D warnings`).
- Until the flakiness side quest (ht-zo4) lands, do not run the full suite routinely; run focused tests for what you changed.


### Normative parent contracts (verbatim)

#### Stable hook entrypoints and ownership

Expose `herdr-threads hook-event HARNESS EVENT` as the dedicated stable CLI surface; map each declared native event exactly to its registered codec/handler. Keep existing `hook HARNESS --event EVENT` syntax working for old registrations. Reject invalid harness/event pairs before daemon effects. A given executable spelling plus canonical state-dir/host-endpoint plus harness/event yields byte-identical shell command across reinstalls. Routing selectors remain pinned where required; different installations/routes must not silently collapse.

Remove the per-install UUID shell comment from newly generated command strings, not the manifest installation identity. Persist exact installed groups/fingerprints and installation ID in ownership records. Existing unmanaged identical groups are foreign: do not adopt, overwrite, remove or treat them as owned without the existing explicit adoption path. Conflicting same-event registrations fail safely with actionable diagnostics. Reinstall reuses its existing exact owned group, retaining position to avoid unnecessary native trust-key shifts. Migrate legacy UUID-marked or shared-event owned groups only under explicit setup/doctor-fix intent, preserving unrelated groups/keys and interrupted-update recovery. Unsetup removes only exactly owned records; drift fails safely. No automatic edit of native trust records, hooks.state, or permission grants. Update status/trust reporting to compare each event's actual stable command rather than infer one shared string. Never silently rewrite real user configurations during development.

The Claude and Codex registrations get their declared per-event entrypoints. Hermes callback transport remains its existing fixed Python bridge; do not mechanically apply native shell grammar to it. Any bundled launch/mod prefix that currently consumes hook argv continues to obtain the canonical route rather than an event command. Registry manifests/canary expectations and embedded skill/help must agree with stable generated commands. TRUST-POLICY records invitation confirmation meaning and repaired receipt timer semantics; no new receipt provenance is needed for invitation acceptance.

#### Acceptance and validation

- Long instructions survive restart/retry unchanged; native argv excludes the body. Invalid, oversized, conflicting or unreadable sources fail before durable writes.
- New and previously dormant ACK obligations expire without recipient activity; already running deadlines do not move. Late registration/acceptance never restarts the clock. Due scans are bounded/indexed, sender receives warnings, late receipt clears normally.
- Lazy ordinary invitation produces no overdue/native wake; explicitly timed invitation warns the originator and acceptance settles automatically. No acceptance/ACK is synthesized by launch or registration.
- Equivalent reinstall emits identical per-event commands while foreign and drifted configuration survives; explicit legacy migration is recoverable and honest about native trust needing review.
- Existing bootstrap unknown-create/possible-start guarantees, human waivers, receipt provenance, required memberships, routing and hook trust boundaries continue passing their focused regression tests.
- Run cargo fmt, nice cargo clippy --locked --all-targets --all-features -- -D warnings and focused tests per change; before merge readiness also nice scripts/check-default-features. Use test-support-owned children/private Herdr sessions, stop all owned processes, and perform scoped leak checks after any full suite.

#### Non-goals and follow-on

No auto-trust, recipient auto-accept, task-adoption ACK, delivery exactly-once claim, automatic 1:1 thread selection, server restart, seat repair heuristics, real user config writes, deployment or merge into main. The parent run notifies its established main-tab discussion when the integration branch is ready for review/merge.

### Coordinator execution contract

Coordinator ruling: ht-0lo.1.2 owns shared due driver/fair scheduling and combined fairness tests; ht-0lo.2.2 owns invitation-class cursor/store API/isolated tests. Preserve full requirements plus explicit cross-task seam and final sweep coverage. Final default-feature check is parent-owned after PR roast per explicit sequencing.

Read TRUST-POLICY.md before any protected source changes. Only implement declared ownership; mechanical compile-preserving adaptations are permitted. Report any additional write path before expanding filesTouched. Do not write root run-state/report/ledger from task worktrees. Test additions reuse existing module targets unless process-wide state requires a dedicated test target.

### TDD implementation and independently testable deliverable

1. Add parser and registered event fixture regressions for dedicated hook-event HARNESS EVENT, legacy hook HARNESS --event EVENT compatibility, and invalid harness/event refusal before daemon effects.
2. Implement dedicated CLI dispatch through existing registry/codecs and stable per-event shell command generation using executable spelling plus exact canonical route/harness/event. Remove random UUID shell suffix only from new command grammar; retain manifest install identity.
3. Keep Hermes Python transport and bundled Claude mod canonical route extraction compatible. Publish exact command-generation and route-extraction APIs for .4.2; preserve existing registrations until explicit migration owns their transition.
4. Run focused hook/entrypoint/setup command grammar tests and format/clippy, including repeat identical spelling and different route separation.

