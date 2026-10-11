# ht-0lo implementation plan

Upgrade-aware discovery refinement: see [upgrade-discovery-refinement.md](upgrade-discovery-refinement.md). Task5/Task7 authorized refinement sections supersede historical copied formula text when H>0; original two-class formula remains applicable when H=0 under its stated snapshots. Ordinal mappings and existing historical bead snapshots remain unchanged.

Durable plan and progress ledger live in this run directory by caller override. Scratch briefs/review packages use `.superpowers/sdd/ht-0lo-plan/`, anchored by the unmodified SDD scripts. Parent owns final readiness and finish; no source implementation is part of materialization.

Coordinator ruling: ht-0lo.1.2 owns shared due driver/fair scheduling and combined fairness tests; ht-0lo.2.2 owns invitation-class cursor/store API/isolated tests. Preserve full requirements plus explicit cross-task seam and final sweep coverage. Final default-feature check is parent-owned after PR roast per explicit sequencing.

| Ordinal | Bead | Task | filesTouched |
| --- | --- | --- | --- |
| 1 | ht-0lo.1.1 | Canonical send-time receipt projection | `["src/store/effective.rs", "src/store/materialization.rs", "src/store/schema.rs", "src/store/messages.rs", "src/store/service_send.rs", "src/protocol/capabilities.rs", "src/cli/mod.rs", "src/cli/retry.rs", "src/client/service.rs", "tests/store/effective.rs", "tests/store/materialization.rs", "tests/store/receipts.rs", "tests/cli/lazy_send.rs", "tests/protocol/capabilities.rs", "tests/store/service_send.rs", "TRUST-POLICY.md"]` |
| 2 | ht-0lo.2.1 | Invitation confirmation protocol and schema contract | `["src/protocol/commands.rs", "src/protocol/results.rs", "src/protocol/capabilities.rs", "src/store/schema.rs", "src/cli/commands.rs", "src/cli/mod.rs", "src/cli/threads.rs", "src/cli/retry.rs", "src/cli/journal.rs", "src/cli/handoff.rs", "src/test_support/history.rs", "tests/protocol/capabilities.rs", "tests/store/schema.rs", "tests/store/control.rs", "tests/store/cooperative_checkin.rs", "tests/store/retention.rs", "tests/store/facade.rs", "tests/integration/channel_activation.rs", "tests/integration/topology_activation.rs", "tests/integration/lazy_attention.rs", "tests/handoff_topology_authority.rs", "tests/cli/cooperative.rs", "tests/cli/handoff_delivery.rs", "tests/service/cooperative.rs", "tests/service/crash_matrix.rs"]` |
| 3 | ht-0lo.4.1 | Stable native event dispatch and command generation | `["src/cli/hook.rs", "src/cli/commands.rs", "src/cli/mod.rs", "src/cli/setup.rs", "src/harness/setup.rs", "src/harness/claude.rs", "src/harness/codex.rs", "tests/cli/hook.rs", "tests/cli/commands.rs", "tests/hook_entrypoint.rs", "tests/harness/setup.rs"]` |
| 4 | ht-0lo.3.1 | Handoff instruction source acquisition and frozen payload | `["src/cli/commands.rs", "src/cli/handoff.rs", "src/cli/input.rs", "src/cli/handoff_delivery.rs", "src/cli/topology_handoff.rs", "src/cli/journal.rs", "src/protocol/handoff.rs", "src/store/topology_handoff.rs", "src/store/topology_handoff/persistence.rs", "tests/cli/handoff.rs", "tests/cli/handoff_delivery.rs", "tests/cli/topology_handoff.rs", "tests/cli/journal.rs", "tests/store/topology_handoff.rs", "tests/store/topology_handoff_schema.rs", "tests/handoff_topology_cli.rs", "TRUST-POLICY.md"]` |
| 5 | ht-0lo.1.3 | Upgrade dormant historical receipt obligations | `["src/store/schema.rs", "src/store/effective.rs", "src/store/materialization.rs", "tests/store/schema.rs", "tests/store/effective.rs", "tests/store/materialization.rs", "tests/store/receipts.rs"]` |
| 6 | ht-0lo.2.2 | Invitation acceptance, due and delivery lifecycle | `["src/store/control.rs", "src/store/invitation_due.rs", "src/store/wake.rs", "src/store/queries.rs", "src/store/service_controls.rs", "src/store/seats.rs", "src/store/schema.rs", "src/store/mod.rs", "src/daemon/control.rs", "tests/store/control.rs", "tests/store/invitation_due.rs", "tests/store/wake.rs", "tests/store/queries.rs", "tests/daemon/control.rs", "TRUST-POLICY.md"]` |
| 7 | ht-0lo.1.2 | Due discovery and originating-seat warning lifecycle | `["src/store/receipts.rs", "src/store/poke.rs", "src/store/wake.rs", "src/store/queries.rs", "src/store/schema.rs", "src/store/mod.rs", "src/scheduler/deadlines.rs", "src/daemon/lifecycle.rs", "tests/store/receipts.rs", "tests/store/poke.rs", "tests/store/wake.rs", "tests/store/wake_discovery_cost.rs", "tests/scheduler/deadlines.rs", "tests/scheduler/poke_flow.rs", "TRUST-POLICY.md", "src/store/attention.rs"]` |
| 8 | ht-0lo.2.3 | Invitation CLI and presentation contract | `["src/cli/commands.rs", "src/cli/mod.rs", "src/cli/threads.rs", "src/cli/output.rs", "src/cli/retry.rs", "integrations/skill/SKILL.md", "README.md", "tests/cli/commands.rs", "tests/cli/output.rs", "tests/cli/skill.rs", "tests/cli/guidance.rs"]` |
| 9 | ht-0lo.4.2 | Owned stable registration migration and tooling | `["src/harness/setup.rs", "src/harness/claude/setup.rs", "src/harness/codex/setup.rs", "src/cli/setup.rs", "src/cli/setup/foreground.rs", "src/cli/launch.rs", "src/harness/launch.rs", "src/harness/claude_mod.rs", "tests/harness/setup.rs", "tests/harness/launch.rs", "tests/harness/claude_mod.rs", "tests/setup_cli.rs", "tests/integration/setup_canary_seam.rs", "tests/canary_manifest_writer.rs", "scripts/canary/manifest.py", "scripts/canary/test_manifest.py", "integrations/skill/SKILL.md", "integrations/claude/README.md", "integrations/codex/README.md"]` |
| 10 | ht-0lo.3.2 | Handoff confirmation policy wiring and recovery documentation | `["src/cli/commands.rs", "src/cli/handoff.rs", "src/cli/handoff_delivery.rs", "src/cli/topology_handoff.rs", "src/cli/mod.rs", "src/cli/launch.rs", "src/harness/launch.rs", "src/protocol/handoff.rs", "src/store/handoff.rs", "src/store/topology_handoff.rs", "src/store/topology_handoff/attachment.rs", "src/store/topology_handoff/runtime.rs", "tests/cli/handoff.rs", "tests/cli/handoff_delivery.rs", "tests/cli/topology_handoff.rs", "tests/cli/launch.rs", "tests/harness/launch.rs", "tests/handoff_topology_cli.rs", "tests/handoff_topology_authority.rs", "integrations/skill/SKILL.md", "README.md"]` |
| 11 | ht-0lo.5 | Integration sweep: handoff delivery redesign | `["tests/cli/handoff.rs", "tests/cli/handoff_delivery.rs", "tests/cli/topology_handoff.rs", "tests/handoff_topology_cli.rs", "tests/integration/notice_wake.rs", "tests/integration/setup_canary_seam.rs", "tests/scheduler/deadlines.rs", "tests/cli/skill.rs", "integrations/skill/SKILL.md", "README.md", "TRUST-POLICY.md", "src/cli/handoff.rs", "src/cli/handoff_delivery.rs", "src/cli/topology_handoff.rs"]` |

Closed design-only ht-0lo.6 and ht-0lo.7 are excluded. Ordinals are immutable; all bd commands use bead IDs, all task-brief commands use ordinals. Dependency order below is topological; independent ready tasks may dispatch together under hot-file cap.

### Root epic full text

{
  "id": "ht-0lo",
  "title": "Handoff delivery redesign",
  "description": "Deliver thread-carried complete handoff instructions with send-time receipt deadlines, lazy optional-confirmation invitations and stable native hook entrypoints. Spec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign-design.md. User authorized autonomous implementation; integration merge remains coordinator decision.",
  "spec_id": "docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign-design.md",
  "status": "open",
  "priority": 2,
  "issue_type": "epic",
  "owner": "235553+alepar@users.noreply.github.com",
  "created_at": "2026-10-10T22:45:27Z",
  "created_by": "Alexey Parfenov",
  "updated_at": "2026-10-10T22:57:33Z",
  "labels": [
    "sp:ht-0lo"
  ],
  "dependent_count": 7,
  "dependency_count": 0,
  "comment_count": 0,
  "revision": "6615966910601246209"
}

## Task 1: Canonical send-time receipt projection

filesTouched: ["src/store/effective.rs", "src/store/materialization.rs", "src/store/schema.rs", "src/store/messages.rs", "src/store/service_send.rs", "src/protocol/capabilities.rs", "src/cli/mod.rs", "src/cli/retry.rs", "src/client/service.rs", "tests/store/effective.rs", "tests/store/materialization.rs", "tests/store/receipts.rs", "tests/cli/lazy_send.rs", "tests/protocol/capabilities.rs", "tests/store/service_send.rs", "TRUST-POLICY.md"]

bead: ht-0lo.1.1

Dependencies: none

### Full bead text (canonical snapshot)

```json
{
  "id": "ht-0lo.1.1",
  "title": "Canonical send-time receipt projection",
  "description": "owns: receipt original deadline semantics, preserving running deadlines and terminal provenance; consumes: existing immutable send timestamps/durations. Files: src/store/{effective,materialization,schema,messages}.rs and preparation modules, TRUST-POLICY deadline clauses, focused store tests. Cover sparse and physical receipts, restart, lazy/human waiver, registration not resetting and explicit A6 extension distinction.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--receipts-design.md\nReceipt semantic capability: advertise messages.send_time_receipts_v1 only when canonical send-time projection and its ordinary send handlers are active. Modern clients require this capability before every new ACK-required ordinary send or handoff durable/native effect; absent/refused discovery produces explicit unsupported-daemon error, with no silent legacy fallback. All CLI entry paths and service-capable client sends that promise this contract are covered. Old clients talking to an upgraded daemon receive canonical new-send semantics without modifying their old wire requests. Lazy/human-waived sends remain exempt; completed retained keyed historical operations remain inspectable/replayable with original bytes/digests/outcomes and are never rewritten. A pending historical retry on an unsupported daemon refuses before additional effects. Test newCLI/olddaemon refusal, oldCLI/newdaemon canonical send, current/current unbound sends and lazy/human exemptions using capability fixtures.",
  "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass. Receipt semantic capability: advertise messages.send_time_receipts_v1 only when canonical send-time projection and its ordinary send handlers are active. Modern clients require this capability before every new ACK-required ordinary send or handoff durable/native effect; absent/refused discovery produces explicit unsupported-daemon error, with no silent legacy fallback. All CLI entry paths and service-capable client sends that promise this contract are covered. Old clients talking to an upgraded daemon receive canonical new-send semantics without modifying their old wire requests. Lazy/human-waived sends remain exempt; completed retained keyed historical operations remain inspectable/replayable with original bytes/digests/outcomes and are never rewritten. A pending historical retry on an unsupported daemon refuses before additional effects. Test newCLI/olddaemon refusal, oldCLI/newdaemon canonical send, current/current unbound sends and lazy/human exemptions using capability fixtures.",
  "status": "open",
  "priority": 2,
  "issue_type": "task",
  "owner": "235553+alepar@users.noreply.github.com",
  "created_at": "2026-10-10T22:48:10Z",
  "created_by": "Alexey Parfenov",
  "updated_at": "2026-10-10T23:18:44Z",
  "labels": [
    "sp:ht-0lo"
  ],
  "dependencies": [
    {
      "id": "ht-0lo.1",
      "title": "Receipt deadlines",
      "description": "owns: canonical send-time/upgrade receipt semantics, due materialization and originating-seat warning routing; consumes: existing preparation/projection schema. Files: src/store/{effective,materialization,receipts,messages,poke,wake,schema,queries}.rs, relevant send preparation modules and tests, TRUST-POLICY.md deadline clauses. Exercise unavailable/unbound/invited send+expiry, sparse/physical/restart, human/lazy exemptions and catch-up distinction before integration.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign-design.md",
      "acceptance_criteria": "Implement the corresponding spec contract and exercise all configurations named in this description with isolated owned children before terminal integration. Run focused tests and mandated per-change lint.",
      "spec_id": "docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--receipts-design.md",
      "status": "open",
      "priority": 2,
      "issue_type": "epic",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:45:44Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T22:57:30Z",
      "metadata": {
        "sp_depth": 1,
        "sp_order": 1
      },
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "parent-child"
    }
  ],
  "parent": "ht-0lo.1",
  "dependent_count": 4,
  "dependency_count": 1,
  "comment_count": 0,
  "revision": "879765571807701544"
}
```

### Acceptance criteria (verbatim)

Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass. Receipt semantic capability: advertise messages.send_time_receipts_v1 only when canonical send-time projection and its ordinary send handlers are active. Modern clients require this capability before every new ACK-required ordinary send or handoff durable/native effect; absent/refused discovery produces explicit unsupported-daemon error, with no silent legacy fallback. All CLI entry paths and service-capable client sends that promise this contract are covered. Old clients talking to an upgraded daemon receive canonical new-send semantics without modifying their old wire requests. Lazy/human-waived sends remain exempt; completed retained keyed historical operations remain inspectable/replayable with original bytes/digests/outcomes and are never rewritten. A pending historical retry on an unsupported daemon refuses before additional effects. Test newCLI/olddaemon refusal, oldCLI/newdaemon canonical send, current/current unbound sends and lazy/human exemptions using capability fixtures.

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

#### Message deadline contract

For every new durable ACK-required ordinary send, freeze the duration using existing validation/default configuration and set its original deadline to canonical committed send time plus that duration. This applies to every recipient, including no binding, managed-launch/unregistered binding, unavailable binding and pending invitation. Availability never changes the original start/deadline, and cannot reset it on registration, accept, restart, or rebind. Lazy messages and human receipt waivers remain free of ACK obligations. Receipt still means receipt only.

The effective receipt projection, manifest preparation, sparse physical materialization, due cursors, soft-poke calculations and wake paths must agree. Keep availability separately meaningful: setting a deadline is not permission to wake, nor evidence of a live harness. Do not pretend send time is observed availability. Existing structural/native execution launch guards and wake harness/empty-composer guards remain mandatory.

Upgrade outstanding pending ACK-required messages too: preserve every previously running deadline, but give formerly dormant obligations the deadline derived from their immutable message send time and frozen duration. Already expired rows become due on the normal bounded scan. Preserve terminal states, receipt provenance and warning identities. A transactionally recorded schema/semantics marker makes old and new manifest/physical projections consistent; materialize/index due work so unavailable recipients are actually discoverable, not merely capable of returning a deadline on individual lookup. Do not rewrite immutable requests or old result digests.

Existing explicit summary catch-up extensions in TRUST-POLICY A6 remain separate and visible; this redesign removes availability-based deferral, not the policy's bounded progress-based extension. Tests distinguish original deadline from an explicitly extended effective deadline. Warning delivery to the originating seat is durable even if it later leaves the thread; joined observers may retain existing warning visibility. An overdue warning says confirmation missed its deadline, never why. The original sender's open warning is waking attention under the existing composer/host/mod admission guards, even if the sender is no longer joined; clear remains passive and observer visibility is unchanged. For non-seat service authors preserve existing service reporting rather than inventing a sender seat. Update TRUST-POLICY A7 to record this narrow new waking recipient rule. Native wake can be deferred by a draft, active turn or unavailable host; the durable warning remains pending, so bounded deadline detection does not promise bounded native key submission. Late ACK clears the existing warning through normal lifecycle without retroactively changing the deadline.

#### Confirmed roast refinements

Healthy-daemon discovery contract: retain the5,000ms safety tick and100candidate per-drive cap; continue immediately while successful due progress reports more. Each class advances a stable finite high-water cursor fairly; newly arriving obligations do not restart/starve the current snapshot. Native wake admission/completion never gates cursor progress. For a snapshot with at most N candidates per class and successful runnable driver quanta taking at most Q ms including storage/scheduling, durable pending-warning detection finishes within5,000ms +(2*ceil(N/100)+2)*Q; this is conditional on the daemon running and successful bounded storage, not a guarantee across outages, repeated errors, unbounded overload or native key submission. Restart resumes/reconstructs fair discovery and scans already overdue obligations. Early fake-clock tests use at least250candidates in EACH invitation and receipt class, blocked native wakes, mixed future/due rows, continuing arrivals and restart; verify every initial overdue row is warned within the formula and arrivals cannot starve either class. Historical upgrade also uses multiple bounded batches without losing overdue indexed discovery.

Receipt semantic capability: advertise messages.send_time_receipts_v1 only when canonical send-time projection and its ordinary send handlers are active. Modern clients require this capability before every new ACK-required ordinary send or handoff durable/native effect; absent/refused discovery produces explicit unsupported-daemon error, with no silent legacy fallback. All CLI entry paths and service-capable client sends that promise this contract are covered. Old clients talking to an upgraded daemon receive canonical new-send semantics without modifying their old wire requests. Lazy/human-waived sends remain exempt; completed retained keyed historical operations remain inspectable/replayable with original bytes/digests/outcomes and are never rewritten. A pending historical retry on an unsupported daemon refuses before additional effects. Test newCLI/olddaemon refusal, oldCLI/newdaemon canonical send, current/current unbound sends and lazy/human exemptions using capability fixtures.

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

1. Add fake-clock store regressions covering new unbound, unavailable, unregistered and invited recipients, both sparse manifest and physical projections, and registration/acceptance/restart/rebind that cannot restart original deadlines. Distinguish explicit A6 catch-up extension; retain lazy and human waivers.
2. Change canonical projection and publication/materialization to derive original pending ACK deadlines from committed send timestamp and frozen duration, preserving already running deadlines and all terminal states/provenance. Keep observed availability independent.
3. Advertise messages.send_time_receipts_v1 only when handlers implement the new semantics. Require it before every new ACK-required ordinary send across actual CLI, retained pending retry, and service-capable client entry paths; completed historical replay remains unchanged. Add new/old client-daemon matrix tests and explicit unsupported/refused discovery no-effects assertions.
4. Run focused projection/materialization/send/capability tests and per-change format/clippy. Expose the capability string and canonical projection semantics to .1.3, .1.2 and .3.2.

## Task 2: Invitation confirmation protocol and schema contract

filesTouched: ["src/protocol/commands.rs", "src/protocol/results.rs", "src/protocol/capabilities.rs", "src/store/schema.rs", "src/cli/commands.rs", "src/cli/mod.rs", "src/cli/threads.rs", "src/cli/retry.rs", "src/cli/journal.rs", "src/cli/handoff.rs", "src/test_support/history.rs", "tests/protocol/capabilities.rs", "tests/store/schema.rs", "tests/store/control.rs", "tests/store/cooperative_checkin.rs", "tests/store/retention.rs", "tests/store/facade.rs", "tests/integration/channel_activation.rs", "tests/integration/topology_activation.rs", "tests/integration/lazy_attention.rs", "tests/handoff_topology_authority.rs", "tests/cli/cooperative.rs", "tests/cli/handoff_delivery.rs", "tests/service/cooperative.rs", "tests/service/crash_matrix.rs"]

bead: ht-0lo.2.1

Dependencies: none

### Full bead text (canonical snapshot)

```json
{
  "id": "ht-0lo.2.1",
  "title": "Invitation confirmation protocol and schema contract",
  "description": "owns: additive invitation delivery/confirmation wire fields, capability refusal and schema fields/indexes/legacy defaults; consumes: existing operation digests and membership episodes. Files: src/protocol commands/results/capabilities or existing capability module, src/store/schema.rs and schema tests. Absent historical fields mean legacy timed behavior; explicit modern Lazy requests retain timing/inviter origin without altering old bytes. Freeze duration at creation, preserve required and human invitation semantics.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--invitations-design.md",
  "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass.",
  "status": "open",
  "priority": 2,
  "issue_type": "task",
  "owner": "235553+alepar@users.noreply.github.com",
  "created_at": "2026-10-10T22:48:13Z",
  "created_by": "Alexey Parfenov",
  "updated_at": "2026-10-10T22:48:13Z",
  "labels": [
    "sp:ht-0lo"
  ],
  "dependencies": [
    {
      "id": "ht-0lo.2",
      "title": "Invitation lifecycle",
      "description": "owns: invitation mode/duration/acceptance contract, protocol capability and inviter warning routing through CLI/store/query/wake; consumes: existing membership lifecycle. Files: src/protocol/{commands,results,capabilities}.rs and wire negotiation, src/cli parser invitation branches, src/store/{control,invitation_due,wake,queries,schema}.rs, relevant tests, TRUST-POLICY invitation clauses. Exercise bare lazy invite, explicit timed invite, never-register expiry, register-without-accept expiry, acceptance/rejection, duplicates, legacy retry and required memberships before integration.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign-design.md",
      "acceptance_criteria": "Implement the corresponding spec contract and exercise all configurations named in this description with isolated owned children before terminal integration. Run focused tests and mandated per-change lint.",
      "spec_id": "docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--invitations-design.md",
      "status": "open",
      "priority": 2,
      "issue_type": "epic",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:45:45Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T22:57:31Z",
      "metadata": {
        "sp_depth": 1,
        "sp_order": 2
      },
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "parent-child"
    }
  ],
  "parent": "ht-0lo.2",
  "dependent_count": 3,
  "dependency_count": 1,
  "comment_count": 0,
  "revision": "-1895875518325156364"
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

#### Invitation contract

Ordinary agent invitations default to Lazy: membership is pending and discoverable through existing inbox/hook opportunities, but the invitation itself requests no timed confirmation or native wake. Add invitation `--require-ack` and optional `--deadline SECONDS`; explicit deadline implies require-ack for ergonomic compatibility, and `--lazy` conflicts with either. An ACK-required invitation starts its acceptance deadline at canonical invitation creation. Only explicit recipient acceptance settles it automatically; no separate invitation ACK command and no registration settlement. Expiry notifies its originating seat with invitation deadline missed, regardless of failure to register or accept. The inviter's open warning is waking attention under existing safe composer/host/mod guards even after leaving; clear remains passive and existing debtor/observer visibility remains. Originator attribution is frozen from the original invitation episode, never inferred from a current member or a later duplicate caller. Rejection/cancellation/retirement retain existing truthful terminal outcomes and attributed rejection notices; lazy does not suppress explicit rejection notices.

Existing required service memberships and explicit human/operator invitation semantics remain unchanged. Existing ordinary pending invitations retain their current timed obligation; historical retry with omitted new fields retains its original interpretation. Use an additive optional delivery/confirmation field whose absence denotes legacy timed behavior; modern CLI explicitly emits Lazy for the new default. Persist requested timing and original inviter attribution independently of joined membership. Pending-invite duplicate calls return the existing invitation without silently upgrading/downgrading its deadline or waking it; changing obligation policy requires a new episode through existing disposition flow. Already joined is still AlreadyJoined, never a new obligation. Existing human mode cannot manufacture an agent ACK requirement.

Queries, inbox compact/machine/JSON, digest, mod/native wake selection and due scans all honor the requested mode. Lazy presentation is bookkeeping only. ACK-required invitation attention remains ordinary guarded delivery; accept remains an accountable recipient choice. Add schema fields/index predicate changes conservatively and negotiate protocol capability/version before any mutation requiring the new representation. Older saved intent bytes/digests are preserved and replayed unchanged; unsupported daemon refuses new semantics before writing.

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

1. Write serialization fixtures proving omitted confirmation field retains exactly historical request bytes/digests and legacy timed meaning; explicit modern Lazy and timed requests round-trip distinctly. Define additive optional mode plus frozen timing and original inviter schema/index representation.
2. Add conservative schema migration and capability advertisement/refusal for the new representation. Preserve required/operator/human membership semantics and legacy pending invitations. Update only mechanical Invite struct literal construction in existing source/tests as required to keep all targets compiling; pass legacy absence in historical callers, without prematurely selecting modern defaults.
3. Test supported and unsupported capability negotiation before new representation effects; schema restart and original inviter/timing persistence. Document exact new wire type/field/capability for .2.2, .2.3 and .3.2.
4. Run focused protocol/schema compatibility tests, cargo fmt and required clippy. Do not implement the lifecycle or modern CLI default assigned to later leaves.

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

## Task 4: Handoff instruction source acquisition and frozen payload

filesTouched: ["src/cli/commands.rs", "src/cli/handoff.rs", "src/cli/input.rs", "src/cli/handoff_delivery.rs", "src/cli/topology_handoff.rs", "src/cli/journal.rs", "src/protocol/handoff.rs", "src/store/topology_handoff.rs", "src/store/topology_handoff/persistence.rs", "tests/cli/handoff.rs", "tests/cli/handoff_delivery.rs", "tests/cli/topology_handoff.rs", "tests/cli/journal.rs", "tests/store/topology_handoff.rs", "tests/store/topology_handoff_schema.rs", "tests/handoff_topology_cli.rs", "TRUST-POLICY.md"]

bead: ht-0lo.3.1

Dependencies: none

### Full bead text (canonical snapshot)

```json
{
  "id": "ht-0lo.3.1",
  "title": "Handoff instruction source acquisition and frozen payload",
  "description": "owns: positional/file/stdin source exclusivity, bounded nonempty UTF-8 65,536-byte acquisition and exact bytes frozen in all handoff intent forms; consumes: existing compound intent/body schema. Files: src/cli/handoff.rs parser and validation, src/cli/{handoff_delivery,topology_handoff}.rs saved plans and associated parser sources/tests. Preserve topic/goal limits separately; invalid sources fail before mutations; mutate/delete source after initial staging and prove retry keeps original content. Native argv contains fixed routing/inbox only.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--handoff-design.md\nAlso owns: body-bearing envelope capacity calculation and limits: bootstrap identity/progress512KiB, recovery1MiB; preserve metadata-only creation/attachment128KiB, completed2MiB/globalwire1MiB unless actual serializer worst-case fixtures prove a narrowly scoped bound adjustment necessary. Files additionally src/store/topology_handoff.rs and persistence.rs, src/cli/journal.rs, TRUST-POLICY encoded-limit clauses. Reuse src/cli/input.rs; protocol body validator must follow normal nonempty UTF-8 message bytes (NUL/whitespace are not identifier validation). Inventory all repeated bodies/nested originals; actual serializer64KiB six-byte escape public wire/canonical/local retained/retry fixtures prove support under ordinary bounded metadata. Exact serialized preflight before effects; retain historical bytes/digests and no clipping.",
  "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass.",
  "status": "open",
  "priority": 2,
  "issue_type": "task",
  "owner": "235553+alepar@users.noreply.github.com",
  "created_at": "2026-10-10T22:48:19Z",
  "created_by": "Alexey Parfenov",
  "updated_at": "2026-10-10T22:59:16Z",
  "labels": [
    "sp:ht-0lo"
  ],
  "dependencies": [
    {
      "id": "ht-0lo.3",
      "title": "Handoff payload",
      "description": "owns: full frozen handoff payload and policy integration; consumes: send-time receipt semantics and invitation confirmation wire contract. Files: src/cli handoff paths, src/protocol/handoff.rs, src/store handoff paths and tests. Spec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--handoff-design.md",
      "acceptance_criteria": "Implement the corresponding spec contract and exercise all configurations named in this description with isolated owned children before terminal integration. Run focused tests and mandated per-change lint.",
      "spec_id": "docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--handoff-design.md",
      "status": "open",
      "priority": 2,
      "issue_type": "epic",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:45:46Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T22:57:32Z",
      "metadata": {
        "sp_depth": 1,
        "sp_order": 4
      },
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "parent-child"
    }
  ],
  "parent": "ht-0lo.3",
  "dependent_count": 2,
  "dependency_count": 1,
  "comment_count": 0,
  "revision": "1169857968755968353"
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

#### Durable handoff interface and recovery

Support exactly one instruction source: the legacy quoted positional body after `--`, `--file PATH`, or `--stdin`. Read once before any durable effect, require nonempty valid UTF-8 within 65,536 bytes, preserve exact content/newlines, bound input while reading, and freeze actual bytes into the compound intent. Retrying never rereads a file/stdin or native environment options. Keep topic/goal validation separate. Reuse src/cli/input.rs normal message source validation; body validation is nonempty valid UTF-8, not bounded_text's unrelated topic/identifier trim or NUL restrictions. The source bytes and their JSON escaping are preserved exactly. All three modes (new-tab, explicit empty-pane launch, existing-recipient delivery) use the same body/deadline contract and exact keys.

Handoff always sends an ACK-required addressed instructions message, defaulting to the existing receipt-duration setting; add `--deadline SECONDS` to override that message deadline, frozen before staging. Its invitation is lazy unless the caller explicitly requests `--require-invite-ack`; add `--invite-deadline SECONDS` to imply that option. Message and invitation deadlines are independent. Stage messages while invitation is pending; sender already joined is required. Carry policy through all public request, preparation, canonical bootstrap, legacy handoff, existing delivery and retained retry representations. Omitted new fields in historical intents preserve historical child requests/digests. No restaging of completed child mutations or automatic launch after possible-start.

Raise only body-bearing retained envelope ceilings conservatively: canonical bootstrap identity and request-bearing local bootstrap progress from128KiB to512KiB; recovery from256KiB to1MiB. Keep metadata-only creation/attachment128KiB, completed2MiB and global wire1MiB unless the actual serializer's supported worst-case fixtures demonstrate a required narrowly scoped adjustment. Six JSON bytes per input byte means a65,536-byte body consumes393,216 encoded bytes per occurrence; inventory every duplicated body and nested serialized original rather than assuming one occurrence. The handoff source task owns actual encoded-bound calculation/tests across public wire, canonical identity/recovery/completion, local progress/linked/terminal/origin snapshots and replay. Test maximum ordinary text, maximum six-byte-escaped UTF-8 body and source mutations through each public handoff path; all supported body values under ordinary bounded metadata must stage and survive retry. Exact serialized capacity checks occur before durable effects; independent oversized metadata/native response limits still fail honestly without truncation or reclassification. Update TRUST-POLICY encoded-limit documentation in the implementation commit and preserve historical bytes/keys/digests. No unconditional global frame increase or unbounded reads.

Native startup contains only fixed inbox instructions, exact canonical routing metadata and pinned fallback. It contains no task body, body path or arbitrary task argv. Compare all three routing fields before preferring hook commands. Existing launched agents with changing harness versions still recover instructions from durable inbox; setup diagnostics/trust refusals do not discard them. Hook/setup/profile readiness inspection is diagnostic and does not block staging or an otherwise safe guarded launch: observed hook installation/trust cannot establish delivery, and a wrong inherited profile must not veto launching a user-selected native harness. Retain exact empty-shell/seat/caller/native argv safety guards; fingerprint-recheck any owned launch configuration actually used, but absence of owned hook configuration is not a veto and never gets a fabricated fingerprint. Unsupported environment overrides and native host-kind/grammar capabilities remain safety refusals; native harness approval/configuration refusals remain truthful refusals, never automatically answered. ACK or overdue warning confirms the bounded handoff outcome, independent of advisory setup status. Launch success is not registration, acceptance, receipt or task adoption. On missed confirmation the sender inspects and recovers; trust/approval remains user consent, never automatic.

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

1. Add exactly-one-source positional/file/stdin parser/acquisition tests at empty/invalid UTF-8/65,536/65,537 byte boundaries; preserve whitespace, NUL and newlines as ordinary message bytes. Reuse bounded src/cli/input.rs acquisition; keep topic/goal checks separate.
2. Freeze actual acquired bytes once in every compound handoff intent. Test mutate/delete file and unavailable stdin after staging, then replay with original body and unchanged keyed identities. Native argv contains only fixed inbox/routing, no body/path.
3. Inventory body duplication and nested escaping across public wire/canonical identity/recovery/completion/local progress/linked/terminal/origin snapshots. Exercise actual serializer worst-case six-byte escape 64KiB fixtures through all three paths; preflight encoded limits before effects. Raise only approved body-bearing bounds, with narrowly justified exceptions supported by fixtures; preserve historical bytes/digests.
4. Update TRUST-POLICY encoded-limit clauses in this same change. Run focused parser/journal/topology retained-retry tests and format/clippy. Export frozen payload shapes to .3.2.

## Task 5: Upgrade dormant historical receipt obligations

filesTouched: ["src/store/schema.rs", "src/store/effective.rs", "src/store/materialization.rs", "tests/store/schema.rs", "tests/store/effective.rs", "tests/store/materialization.rs", "tests/store/receipts.rs"]

bead: ht-0lo.1.3

Dependencies: ht-0lo.1.1

### Full bead text (canonical snapshot)

```json
{
  "id": "ht-0lo.1.3",
  "title": "Upgrade dormant historical receipt obligations",
  "description": "owns: transactional upgrade/backfill of formerly dormant historical ACK obligations, sparse/physical due projection consistency and idempotent schema marker; consumes: canonical send-time receipt projection. Preserve already running deadlines, terminal states/provenance and immutable request/result digests. Files: src/store/{schema,effective,materialization}.rs and focused migration/store tests. Exercise legacy unavailable receipts upgrade/restart/replay and overdue indexed discovery, bounded warning burst.\nblocked-by ht-0lo.1.1: consumes canonical send-time receipt projection\nExercise dormant historical upgrade over multiple100candidate batches and verify no overdue initial row is lost or starved by newly arriving work; discovery contract is owned by ht-0lo.1.2.",
  "acceptance_criteria": "Legacy dormant physical/manifest ACK obligations receive send-time deadlines while existing started/terminal/waived obligations remain unchanged. Idempotent restart and focused migration tests pass. Exercise dormant historical upgrade over multiple100candidate batches and verify no overdue initial row is lost or starved by newly arriving work; discovery contract is owned by ht-0lo.1.2.",
  "status": "open",
  "priority": 2,
  "issue_type": "task",
  "owner": "235553+alepar@users.noreply.github.com",
  "created_at": "2026-10-10T22:49:48Z",
  "created_by": "Alexey Parfenov",
  "updated_at": "2026-10-10T23:18:48Z",
  "labels": [
    "sp:ht-0lo"
  ],
  "dependencies": [
    {
      "id": "ht-0lo.1.1",
      "title": "Canonical send-time receipt projection",
      "description": "owns: receipt original deadline semantics, preserving running deadlines and terminal provenance; consumes: existing immutable send timestamps/durations. Files: src/store/{effective,materialization,schema,messages}.rs and preparation modules, TRUST-POLICY deadline clauses, focused store tests. Cover sparse and physical receipts, restart, lazy/human waiver, registration not resetting and explicit A6 extension distinction.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--receipts-design.md\nReceipt semantic capability: advertise messages.send_time_receipts_v1 only when canonical send-time projection and its ordinary send handlers are active. Modern clients require this capability before every new ACK-required ordinary send or handoff durable/native effect; absent/refused discovery produces explicit unsupported-daemon error, with no silent legacy fallback. All CLI entry paths and service-capable client sends that promise this contract are covered. Old clients talking to an upgraded daemon receive canonical new-send semantics without modifying their old wire requests. Lazy/human-waived sends remain exempt; completed retained keyed historical operations remain inspectable/replayable with original bytes/digests/outcomes and are never rewritten. A pending historical retry on an unsupported daemon refuses before additional effects. Test newCLI/olddaemon refusal, oldCLI/newdaemon canonical send, current/current unbound sends and lazy/human exemptions using capability fixtures.",
      "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass. Receipt semantic capability: advertise messages.send_time_receipts_v1 only when canonical send-time projection and its ordinary send handlers are active. Modern clients require this capability before every new ACK-required ordinary send or handoff durable/native effect; absent/refused discovery produces explicit unsupported-daemon error, with no silent legacy fallback. All CLI entry paths and service-capable client sends that promise this contract are covered. Old clients talking to an upgraded daemon receive canonical new-send semantics without modifying their old wire requests. Lazy/human-waived sends remain exempt; completed retained keyed historical operations remain inspectable/replayable with original bytes/digests/outcomes and are never rewritten. A pending historical retry on an unsupported daemon refuses before additional effects. Test newCLI/olddaemon refusal, oldCLI/newdaemon canonical send, current/current unbound sends and lazy/human exemptions using capability fixtures.",
      "status": "open",
      "priority": 2,
      "issue_type": "task",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:48:10Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T23:18:44Z",
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "blocks"
    },
    {
      "id": "ht-0lo.1",
      "title": "Receipt deadlines",
      "description": "owns: canonical send-time/upgrade receipt semantics, due materialization and originating-seat warning routing; consumes: existing preparation/projection schema. Files: src/store/{effective,materialization,receipts,messages,poke,wake,schema,queries}.rs, relevant send preparation modules and tests, TRUST-POLICY.md deadline clauses. Exercise unavailable/unbound/invited send+expiry, sparse/physical/restart, human/lazy exemptions and catch-up distinction before integration.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign-design.md",
      "acceptance_criteria": "Implement the corresponding spec contract and exercise all configurations named in this description with isolated owned children before terminal integration. Run focused tests and mandated per-change lint.",
      "spec_id": "docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--receipts-design.md",
      "status": "open",
      "priority": 2,
      "issue_type": "epic",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:45:44Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T22:57:30Z",
      "metadata": {
        "sp_depth": 1,
        "sp_order": 1
      },
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "parent-child"
    }
  ],
  "parent": "ht-0lo.1",
  "dependent_count": 2,
  "dependency_count": 2,
  "comment_count": 0,
  "revision": "-3440678625004889246"
}
```

### Acceptance criteria (verbatim)

Legacy dormant physical/manifest ACK obligations receive send-time deadlines while existing started/terminal/waived obligations remain unchanged. Idempotent restart and focused migration tests pass. Exercise dormant historical upgrade over multiple100candidate batches and verify no overdue initial row is lost or starved by newly arriving work; discovery contract is owned by ht-0lo.1.2.

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

#### Message deadline contract

For every new durable ACK-required ordinary send, freeze the duration using existing validation/default configuration and set its original deadline to canonical committed send time plus that duration. This applies to every recipient, including no binding, managed-launch/unregistered binding, unavailable binding and pending invitation. Availability never changes the original start/deadline, and cannot reset it on registration, accept, restart, or rebind. Lazy messages and human receipt waivers remain free of ACK obligations. Receipt still means receipt only.

The effective receipt projection, manifest preparation, sparse physical materialization, due cursors, soft-poke calculations and wake paths must agree. Keep availability separately meaningful: setting a deadline is not permission to wake, nor evidence of a live harness. Do not pretend send time is observed availability. Existing structural/native execution launch guards and wake harness/empty-composer guards remain mandatory.

Upgrade outstanding pending ACK-required messages too: preserve every previously running deadline, but give formerly dormant obligations the deadline derived from their immutable message send time and frozen duration. Already expired rows become due on the normal bounded scan. Preserve terminal states, receipt provenance and warning identities. A transactionally recorded schema/semantics marker makes old and new manifest/physical projections consistent; materialize/index due work so unavailable recipients are actually discoverable, not merely capable of returning a deadline on individual lookup. Do not rewrite immutable requests or old result digests.

Existing explicit summary catch-up extensions in TRUST-POLICY A6 remain separate and visible; this redesign removes availability-based deferral, not the policy's bounded progress-based extension. Tests distinguish original deadline from an explicitly extended effective deadline. Warning delivery to the originating seat is durable even if it later leaves the thread; joined observers may retain existing warning visibility. An overdue warning says confirmation missed its deadline, never why. The original sender's open warning is waking attention under the existing composer/host/mod admission guards, even if the sender is no longer joined; clear remains passive and observer visibility is unchanged. For non-seat service authors preserve existing service reporting rather than inventing a sender seat. Update TRUST-POLICY A7 to record this narrow new waking recipient rule. Native wake can be deferred by a draft, active turn or unavailable host; the durable warning remains pending, so bounded deadline detection does not promise bounded native key submission. Late ACK clears the existing warning through normal lifecycle without retroactively changing the deadline.

#### Confirmed roast refinements

Healthy-daemon discovery contract: retain the5,000ms safety tick and100candidate per-drive cap; continue immediately while successful due progress reports more. Each class advances a stable finite high-water cursor fairly; newly arriving obligations do not restart/starve the current snapshot. Native wake admission/completion never gates cursor progress. For a snapshot with at most N candidates per class and successful runnable driver quanta taking at most Q ms including storage/scheduling, durable pending-warning detection finishes within5,000ms +(2*ceil(N/100)+2)*Q; this is conditional on the daemon running and successful bounded storage, not a guarantee across outages, repeated errors, unbounded overload or native key submission. Restart resumes/reconstructs fair discovery and scans already overdue obligations. Early fake-clock tests use at least250candidates in EACH invitation and receipt class, blocked native wakes, mixed future/due rows, continuing arrivals and restart; verify every initial overdue row is warned within the formula and arrivals cannot starve either class. Historical upgrade also uses multiple bounded batches without losing overdue indexed discovery.

Receipt semantic capability: advertise messages.send_time_receipts_v1 only when canonical send-time projection and its ordinary send handlers are active. Modern clients require this capability before every new ACK-required ordinary send or handoff durable/native effect; absent/refused discovery produces explicit unsupported-daemon error, with no silent legacy fallback. All CLI entry paths and service-capable client sends that promise this contract are covered. Old clients talking to an upgraded daemon receive canonical new-send semantics without modifying their old wire requests. Lazy/human-waived sends remain exempt; completed retained keyed historical operations remain inspectable/replayable with original bytes/digests/outcomes and are never rewritten. A pending historical retry on an unsupported daemon refuses before additional effects. Test newCLI/olddaemon refusal, oldCLI/newdaemon canonical send, current/current unbound sends and lazy/human exemptions using capability fixtures.

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

1. Construct historical physical and manifest pending obligations with dormant timers, existing running deadlines, terminal/waived receipts and immutable retained request/result digests.
2. Implement transactional idempotent semantics marker/backfill so dormant obligations derive deadline from immutable send timestamp/frozen duration while all running and terminal values remain unchanged. Ensure sparse/materialized due indexes expose overdue rows.
3. Test multiple 100-candidate upgrade batches, restart/replay and continuing arrivals; prove no original overdue row disappears. Discovery scheduling belongs to .1.2 and consumes this landed indexed representation.
4. Run focused migration/effective/materialization tests and format/clippy; record marker and indexed discovery contract for .1.2.


### Authorized upgrade-aware acceptance refinement (supersedes prior formula)

Healthy-daemon discovery retains the 5,000ms safety tick and 100 actually inspected candidates per successful global due-driver quantum. Every two successful runnable global quanta give each unfinished receipt/invitation class one turn with a 100-candidate allowance; successful `more` continues immediately. All queries enforce finite captured high-water bounds before LIMIT, advance stable cursors, and count inspected rows even if future, terminal, waived or otherwise ineffective. Ordinary receipt scanning lends unused physical/sparse allowance so a non-final snapshot turn can consume its full allowance; native wake admission/completion never gates cursor progress. The 100 limit is a total row-inspection allowance, not 100 repaired rows plus 100 due rows.

During unfinished historical receipt repair, reserve at most ceil(limit/2) candidates for repair and the remaining allowance for ordinary due discovery (at least 50 at limit=100); lend unused repair allowance to due discovery. Advance persistent finite physical/prepared ordinal repair cursors without restarting ordinary due traversal every batch. Inspect each captured repair candidate at most once; published projection alone is repaired, while any physically visited unpublished prepared candidate still counts. Repair completion emits a one-shot `completed` transition and resets the ordinary receipt cursor before due selection in that same committed transaction. Repair progress, that completion/reset decision and due-warning effects commit or roll back together. This fresh post-repair traversal catches repaired deadlines behind a previously advanced due cursor. Empty repair finalization uses zero row allowance and shares the ordinary turn at full capacity. Failed decisions cannot consume the completion transition. Standalone backfill callers retaining a due cursor must honor `completed` too.

Bound definitions: start at a fresh paired discovery cycle with the target obligations already overdue, or restart with reconstructed finite cursors. H is an upper bound on ALL actually inspected candidates in the captured historical physical/prepared repair workload, including future/terminal/waived/unpublished candidates; it is not the number changed. D bounds ALL inspected ordinary receipt candidates in the complete fresh post-repair snapshot, including repaired rows, duplicate physical/sparse projections and finite arrivals captured before that snapshot freezes. I bounds ALL inspected invitation candidates in its initial finite snapshot, including future or subsequently ineffective rows. Q bounds elapsed time of one successful runnable GLOBAL driver quantum, including storage, scheduling and other admitted work. Later insertions outside captured high waters cannot enter or restart those walks, even when their deadlines are earlier. A finite D bound must cover any arrivals before the post-repair snapshot freezes; unbounded arrivals during repair do not justify using only the initial receipt count.

When H=0, empty repair bookkeeping completes within the first ordinary turn without reducing its capacity. With D and I at most N, the original two-class bound remains 5,000ms + (2*ceil(N/100)+2)*Q. For H>0, set R=ceil(H/50)+2: the extra two receipt turns conservatively cover finite-stream/end probes and repair completion. After those R receipt turns, at most ceil(D/100)+1 further receipt turns finish the fresh post-repair snapshot; the +1 covers a full final page's empty completion. Invitations need at most ceil(I/100)+1 invitation turns. Fair alternation therefore gives the conservative GLOBAL bound:

`5,000ms + 2*max(R + ceil(D/100)+1, ceil(I/100)+1)*Q` for H>0.

The bound concerns durable warning detection for still-pending, unextended target obligations, not bounded native key submission. It is conditional on successful bounded storage/driver service, the stated finite workload/snapshot limits and Q; it does not cover outages, repeated errors, unbounded overload, newly expiring rows already passed by a snapshot cursor, or arbitrary arrivals outside the counted snapshots. Such rows belong to a subsequent fresh cycle. Restart resumes or reconstructs finite traversal without losing repair progress; a newly reconstructed traversal must count its inspected workload again. Tests must measure actual inspected work and global quanta, rather than changed rows or receipt-only calls.

Task5 verifies the producer locally: multiple batches, finite captured high waters, late earlier-deadline arrivals excluded from repair, mixed future/terminal/waived/unpublished candidates, all-terminal old backlog, one-shot atomic completion/reset, and rollback/restart. Task7 verifies the global conversion with at least 250 candidates in EACH receipt and invitation class, both H=0 and H>0, blocked native wakes, mixed future/due rows, continuing earlier-deadline arrivals, actual <=100 total inspections per quantum, immediate successful continuation, early ordinary-due progress before long repair exhaustion, final post-repair discovery and restart. Task7 owns normal due-query high-water enforcement/full allowance lending and shared fairness; Task5 does not claim the combined bound from receipt-only tests.

## Task 6: Invitation acceptance, due and delivery lifecycle

filesTouched: ["src/store/control.rs", "src/store/invitation_due.rs", "src/store/wake.rs", "src/store/queries.rs", "src/store/service_controls.rs", "src/store/seats.rs", "src/store/schema.rs", "src/store/mod.rs", "src/daemon/control.rs", "tests/store/control.rs", "tests/store/invitation_due.rs", "tests/store/wake.rs", "tests/store/queries.rs", "tests/daemon/control.rs", "TRUST-POLICY.md"]

bead: ht-0lo.2.2

Dependencies: ht-0lo.2.1

### Full bead text (canonical snapshot)

```json
{
  "id": "ht-0lo.2.2",
  "title": "Invitation acceptance, due and delivery lifecycle",
  "description": "owns: canonical lazy/timed invitation create/duplicate/accept/reject/cancel/retire behavior, due scanner and inviter-addressed warning routing plus read/wake/mod projections; consumes: invitation mode schema/wire contract. Files: src/store/{control,invitation_due,wake,queries,service_controls,seats}.rs as needed, daemon projections, TRUST-POLICY invitation clauses and focused tests. Run lazy invite no native wake/deadline, timed never-register and register-without-accept expiry, acceptance clears, duplicate never mutates policy, inviter leaving warning, required membership and explicit rejection notice cases.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--invitations-design.md\nblocked-by ht-0lo.2.1: consumes invitation mode schema/wire contract\nOriginal inviter attribution frozen at invitation creation; overdue inviter open warning waking under existing native/mod guards even after leaving, clear passive; update TRUST-POLICY A7. Duplicate caller never retargets originator warning.\nHealthy-daemon discovery contract: retain the5,000ms safety tick and100candidate per-drive cap; continue immediately while successful due progress reports more. Each class advances a stable finite high-water cursor fairly; newly arriving obligations do not restart/starve the current snapshot. Native wake admission/completion never gates cursor progress. For a snapshot with at most N candidates per class and successful runnable driver quanta taking at most Q ms including storage/scheduling, durable pending-warning detection finishes within5,000ms +(2*ceil(N/100)+2)*Q; this is conditional on the daemon running and successful bounded storage, not a guarantee across outages, repeated errors, unbounded overload or native key submission. Restart resumes/reconstructs fair discovery and scans already overdue obligations. Early fake-clock tests use at least250candidates in EACH invitation and receipt class, blocked native wakes, mixed future/due rows, continuing arrivals and restart; verify every initial overdue row is warned within the formula and arrivals cannot starve either class. Historical upgrade also uses multiple bounded batches without losing overdue indexed discovery.",
  "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass. Healthy-daemon discovery contract: retain the5,000ms safety tick and100candidate per-drive cap; continue immediately while successful due progress reports more. Each class advances a stable finite high-water cursor fairly; newly arriving obligations do not restart/starve the current snapshot. Native wake admission/completion never gates cursor progress. For a snapshot with at most N candidates per class and successful runnable driver quanta taking at most Q ms including storage/scheduling, durable pending-warning detection finishes within5,000ms +(2*ceil(N/100)+2)*Q; this is conditional on the daemon running and successful bounded storage, not a guarantee across outages, repeated errors, unbounded overload or native key submission. Restart resumes/reconstructs fair discovery and scans already overdue obligations. Early fake-clock tests use at least250candidates in EACH invitation and receipt class, blocked native wakes, mixed future/due rows, continuing arrivals and restart; verify every initial overdue row is warned within the formula and arrivals cannot starve either class. Historical upgrade also uses multiple bounded batches without losing overdue indexed discovery.",
  "status": "open",
  "priority": 2,
  "issue_type": "task",
  "owner": "235553+alepar@users.noreply.github.com",
  "created_at": "2026-10-10T22:48:14Z",
  "created_by": "Alexey Parfenov",
  "updated_at": "2026-10-10T23:18:51Z",
  "labels": [
    "sp:ht-0lo"
  ],
  "dependencies": [
    {
      "id": "ht-0lo.2.1",
      "title": "Invitation confirmation protocol and schema contract",
      "description": "owns: additive invitation delivery/confirmation wire fields, capability refusal and schema fields/indexes/legacy defaults; consumes: existing operation digests and membership episodes. Files: src/protocol commands/results/capabilities or existing capability module, src/store/schema.rs and schema tests. Absent historical fields mean legacy timed behavior; explicit modern Lazy requests retain timing/inviter origin without altering old bytes. Freeze duration at creation, preserve required and human invitation semantics.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--invitations-design.md",
      "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass.",
      "status": "open",
      "priority": 2,
      "issue_type": "task",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:48:13Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T22:48:13Z",
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "blocks"
    },
    {
      "id": "ht-0lo.2",
      "title": "Invitation lifecycle",
      "description": "owns: invitation mode/duration/acceptance contract, protocol capability and inviter warning routing through CLI/store/query/wake; consumes: existing membership lifecycle. Files: src/protocol/{commands,results,capabilities}.rs and wire negotiation, src/cli parser invitation branches, src/store/{control,invitation_due,wake,queries,schema}.rs, relevant tests, TRUST-POLICY invitation clauses. Exercise bare lazy invite, explicit timed invite, never-register expiry, register-without-accept expiry, acceptance/rejection, duplicates, legacy retry and required memberships before integration.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign-design.md",
      "acceptance_criteria": "Implement the corresponding spec contract and exercise all configurations named in this description with isolated owned children before terminal integration. Run focused tests and mandated per-change lint.",
      "spec_id": "docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--invitations-design.md",
      "status": "open",
      "priority": 2,
      "issue_type": "epic",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:45:45Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T22:57:31Z",
      "metadata": {
        "sp_depth": 1,
        "sp_order": 2
      },
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "parent-child"
    }
  ],
  "parent": "ht-0lo.2",
  "dependent_count": 3,
  "dependency_count": 2,
  "comment_count": 0,
  "revision": "8084961125494513044"
}
```

### Acceptance criteria (verbatim)

Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass. Healthy-daemon discovery contract: retain the5,000ms safety tick and100candidate per-drive cap; continue immediately while successful due progress reports more. Each class advances a stable finite high-water cursor fairly; newly arriving obligations do not restart/starve the current snapshot. Native wake admission/completion never gates cursor progress. For a snapshot with at most N candidates per class and successful runnable driver quanta taking at most Q ms including storage/scheduling, durable pending-warning detection finishes within5,000ms +(2*ceil(N/100)+2)*Q; this is conditional on the daemon running and successful bounded storage, not a guarantee across outages, repeated errors, unbounded overload or native key submission. Restart resumes/reconstructs fair discovery and scans already overdue obligations. Early fake-clock tests use at least250candidates in EACH invitation and receipt class, blocked native wakes, mixed future/due rows, continuing arrivals and restart; verify every initial overdue row is warned within the formula and arrivals cannot starve either class. Historical upgrade also uses multiple bounded batches without losing overdue indexed discovery.

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

#### Invitation contract

Ordinary agent invitations default to Lazy: membership is pending and discoverable through existing inbox/hook opportunities, but the invitation itself requests no timed confirmation or native wake. Add invitation `--require-ack` and optional `--deadline SECONDS`; explicit deadline implies require-ack for ergonomic compatibility, and `--lazy` conflicts with either. An ACK-required invitation starts its acceptance deadline at canonical invitation creation. Only explicit recipient acceptance settles it automatically; no separate invitation ACK command and no registration settlement. Expiry notifies its originating seat with invitation deadline missed, regardless of failure to register or accept. The inviter's open warning is waking attention under existing safe composer/host/mod guards even after leaving; clear remains passive and existing debtor/observer visibility remains. Originator attribution is frozen from the original invitation episode, never inferred from a current member or a later duplicate caller. Rejection/cancellation/retirement retain existing truthful terminal outcomes and attributed rejection notices; lazy does not suppress explicit rejection notices.

Existing required service memberships and explicit human/operator invitation semantics remain unchanged. Existing ordinary pending invitations retain their current timed obligation; historical retry with omitted new fields retains its original interpretation. Use an additive optional delivery/confirmation field whose absence denotes legacy timed behavior; modern CLI explicitly emits Lazy for the new default. Persist requested timing and original inviter attribution independently of joined membership. Pending-invite duplicate calls return the existing invitation without silently upgrading/downgrading its deadline or waking it; changing obligation policy requires a new episode through existing disposition flow. Already joined is still AlreadyJoined, never a new obligation. Existing human mode cannot manufacture an agent ACK requirement.

Queries, inbox compact/machine/JSON, digest, mod/native wake selection and due scans all honor the requested mode. Lazy presentation is bookkeeping only. ACK-required invitation attention remains ordinary guarded delivery; accept remains an accountable recipient choice. Add schema fields/index predicate changes conservatively and negotiate protocol capability/version before any mutation requiring the new representation. Older saved intent bytes/digests are preserved and replayed unchanged; unsupported daemon refuses new semantics before writing.

#### Confirmed roast refinements

Healthy-daemon discovery contract: retain the5,000ms safety tick and100candidate per-drive cap; continue immediately while successful due progress reports more. Each class advances a stable finite high-water cursor fairly; newly arriving obligations do not restart/starve the current snapshot. Native wake admission/completion never gates cursor progress. For a snapshot with at most N candidates per class and successful runnable driver quanta taking at most Q ms including storage/scheduling, durable pending-warning detection finishes within5,000ms +(2*ceil(N/100)+2)*Q; this is conditional on the daemon running and successful bounded storage, not a guarantee across outages, repeated errors, unbounded overload or native key submission. Restart resumes/reconstructs fair discovery and scans already overdue obligations. Early fake-clock tests use at least250candidates in EACH invitation and receipt class, blocked native wakes, mixed future/due rows, continuing arrivals and restart; verify every initial overdue row is warned within the formula and arrivals cannot starve either class. Historical upgrade also uses multiple bounded batches without losing overdue indexed discovery.

Receipt semantic capability: advertise messages.send_time_receipts_v1 only when canonical send-time projection and its ordinary send handlers are active. Modern clients require this capability before every new ACK-required ordinary send or handoff durable/native effect; absent/refused discovery produces explicit unsupported-daemon error, with no silent legacy fallback. All CLI entry paths and service-capable client sends that promise this contract are covered. Old clients talking to an upgraded daemon receive canonical new-send semantics without modifying their old wire requests. Lazy/human-waived sends remain exempt; completed retained keyed historical operations remain inspectable/replayable with original bytes/digests/outcomes and are never rewritten. A pending historical retry on an unsupported daemon refuses before additional effects. Test newCLI/olddaemon refusal, oldCLI/newdaemon canonical send, current/current unbound sends and lazy/human exemptions using capability fixtures.

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

1. Add fake-clock lifecycle tests for lazy no-deadline/no-wake invitation; explicitly timed never-register and register-without-accept expiry; accept/late accept/reject/cancel/retire, original inviter leaving and duplicate callers trying to change timing/originator.
2. Implement canonical mode and frozen origin/timing through invitation create/duplicate/terminal/query/inbox/digest/mod/wake projections. Acceptance alone settles timed confirmation automatically; registration does not. Retain required and human/operator behavior and attributed lazy rejection notice.
3. Own invitation-class bounded high-water cursor/store API and isolated invitation due tests. Exclude lazy rows from due/wake selection; route original inviter open warning as guarded waking attention after leave, clear passively. Publish cursor/progress API for shared-driver owner .1.2; do not independently redesign shared scheduling.
4. Update TRUST-POLICY invitation/A7 clauses and run focused store lifecycle/due/wake/query tests plus format/clippy. Shared combined fairness acceptance remains in .1.2 and terminal sweep.

## Task 7: Due discovery and originating-seat warning lifecycle

filesTouched: ["src/store/receipts.rs", "src/store/poke.rs", "src/store/wake.rs", "src/store/queries.rs", "src/store/schema.rs", "src/store/mod.rs", "src/scheduler/deadlines.rs", "src/daemon/lifecycle.rs", "tests/store/receipts.rs", "tests/store/poke.rs", "tests/store/wake.rs", "tests/store/wake_discovery_cost.rs", "tests/scheduler/deadlines.rs", "tests/scheduler/poke_flow.rs", "TRUST-POLICY.md", "src/store/attention.rs"]

bead: ht-0lo.1.2

Dependencies: ht-0lo.1.3, ht-0lo.1.1

Coordinator-added dependency: ht-0lo.2.2 supplies the invitation-class scanner consumed by shared fair due-driver tests. The canonical snapshot above predates this added edge.

### Full bead text (canonical snapshot)

```json
{
  "id": "ht-0lo.1.2",
  "title": "Due discovery and originating-seat warning lifecycle",
  "description": "owns: bounded indexed due discovery plus sender-addressed open/clear warning routing and guarded poke/wake calculations; consumes: send-anchored canonical receipt projection. Files: src/store/{receipts,poke,wake,queries,schema}.rs, scheduler tests. Exercise unbound/unregistered/unavailable/pending-invite recipient sends through expiry and late ACK; originator leaving does not lose notification.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--receipts-design.md\nblocked-by ht-0lo.1.1: consumes send-anchored canonical receipt projection\nblocked-by ht-0lo.1.3: consumes historical dormant receipt upgrade\nOriginator open warning is waking attention under existing safe composer/host/mod guards even after leaving; clear passive; update TRUST-POLICY A7. No invented seat for service authors. Test notification pending while drafts/working/host outage defer actual native wake.\nHealthy-daemon discovery contract: retain the5,000ms safety tick and100candidate per-drive cap; continue immediately while successful due progress reports more. Each class advances a stable finite high-water cursor fairly; newly arriving obligations do not restart/starve the current snapshot. Native wake admission/completion never gates cursor progress. For a snapshot with at most N candidates per class and successful runnable driver quanta taking at most Q ms including storage/scheduling, durable pending-warning detection finishes within5,000ms +(2*ceil(N/100)+2)*Q; this is conditional on the daemon running and successful bounded storage, not a guarantee across outages, repeated errors, unbounded overload or native key submission. Restart resumes/reconstructs fair discovery and scans already overdue obligations. Early fake-clock tests use at least250candidates in EACH invitation and receipt class, blocked native wakes, mixed future/due rows, continuing arrivals and restart; verify every initial overdue row is warned within the formula and arrivals cannot starve either class. Historical upgrade also uses multiple bounded batches without losing overdue indexed discovery.",
  "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass. Healthy-daemon discovery contract: retain the5,000ms safety tick and100candidate per-drive cap; continue immediately while successful due progress reports more. Each class advances a stable finite high-water cursor fairly; newly arriving obligations do not restart/starve the current snapshot. Native wake admission/completion never gates cursor progress. For a snapshot with at most N candidates per class and successful runnable driver quanta taking at most Q ms including storage/scheduling, durable pending-warning detection finishes within5,000ms +(2*ceil(N/100)+2)*Q; this is conditional on the daemon running and successful bounded storage, not a guarantee across outages, repeated errors, unbounded overload or native key submission. Restart resumes/reconstructs fair discovery and scans already overdue obligations. Early fake-clock tests use at least250candidates in EACH invitation and receipt class, blocked native wakes, mixed future/due rows, continuing arrivals and restart; verify every initial overdue row is warned within the formula and arrivals cannot starve either class. Historical upgrade also uses multiple bounded batches without losing overdue indexed discovery.",
  "status": "open",
  "priority": 2,
  "issue_type": "task",
  "owner": "235553+alepar@users.noreply.github.com",
  "created_at": "2026-10-10T22:48:11Z",
  "created_by": "Alexey Parfenov",
  "updated_at": "2026-10-10T23:18:46Z",
  "labels": [
    "sp:ht-0lo"
  ],
  "dependencies": [
    {
      "id": "ht-0lo.1.3",
      "title": "Upgrade dormant historical receipt obligations",
      "description": "owns: transactional upgrade/backfill of formerly dormant historical ACK obligations, sparse/physical due projection consistency and idempotent schema marker; consumes: canonical send-time receipt projection. Preserve already running deadlines, terminal states/provenance and immutable request/result digests. Files: src/store/{schema,effective,materialization}.rs and focused migration/store tests. Exercise legacy unavailable receipts upgrade/restart/replay and overdue indexed discovery, bounded warning burst.\nblocked-by ht-0lo.1.1: consumes canonical send-time receipt projection\nExercise dormant historical upgrade over multiple100candidate batches and verify no overdue initial row is lost or starved by newly arriving work; discovery contract is owned by ht-0lo.1.2.",
      "acceptance_criteria": "Legacy dormant physical/manifest ACK obligations receive send-time deadlines while existing started/terminal/waived obligations remain unchanged. Idempotent restart and focused migration tests pass. Exercise dormant historical upgrade over multiple100candidate batches and verify no overdue initial row is lost or starved by newly arriving work; discovery contract is owned by ht-0lo.1.2.",
      "status": "open",
      "priority": 2,
      "issue_type": "task",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:49:48Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T23:18:48Z",
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "blocks"
    },
    {
      "id": "ht-0lo.1.1",
      "title": "Canonical send-time receipt projection",
      "description": "owns: receipt original deadline semantics, preserving running deadlines and terminal provenance; consumes: existing immutable send timestamps/durations. Files: src/store/{effective,materialization,schema,messages}.rs and preparation modules, TRUST-POLICY deadline clauses, focused store tests. Cover sparse and physical receipts, restart, lazy/human waiver, registration not resetting and explicit A6 extension distinction.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--receipts-design.md\nReceipt semantic capability: advertise messages.send_time_receipts_v1 only when canonical send-time projection and its ordinary send handlers are active. Modern clients require this capability before every new ACK-required ordinary send or handoff durable/native effect; absent/refused discovery produces explicit unsupported-daemon error, with no silent legacy fallback. All CLI entry paths and service-capable client sends that promise this contract are covered. Old clients talking to an upgraded daemon receive canonical new-send semantics without modifying their old wire requests. Lazy/human-waived sends remain exempt; completed retained keyed historical operations remain inspectable/replayable with original bytes/digests/outcomes and are never rewritten. A pending historical retry on an unsupported daemon refuses before additional effects. Test newCLI/olddaemon refusal, oldCLI/newdaemon canonical send, current/current unbound sends and lazy/human exemptions using capability fixtures.",
      "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass. Receipt semantic capability: advertise messages.send_time_receipts_v1 only when canonical send-time projection and its ordinary send handlers are active. Modern clients require this capability before every new ACK-required ordinary send or handoff durable/native effect; absent/refused discovery produces explicit unsupported-daemon error, with no silent legacy fallback. All CLI entry paths and service-capable client sends that promise this contract are covered. Old clients talking to an upgraded daemon receive canonical new-send semantics without modifying their old wire requests. Lazy/human-waived sends remain exempt; completed retained keyed historical operations remain inspectable/replayable with original bytes/digests/outcomes and are never rewritten. A pending historical retry on an unsupported daemon refuses before additional effects. Test newCLI/olddaemon refusal, oldCLI/newdaemon canonical send, current/current unbound sends and lazy/human exemptions using capability fixtures.",
      "status": "open",
      "priority": 2,
      "issue_type": "task",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:48:10Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T23:18:44Z",
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "blocks"
    },
    {
      "id": "ht-0lo.1",
      "title": "Receipt deadlines",
      "description": "owns: canonical send-time/upgrade receipt semantics, due materialization and originating-seat warning routing; consumes: existing preparation/projection schema. Files: src/store/{effective,materialization,receipts,messages,poke,wake,schema,queries}.rs, relevant send preparation modules and tests, TRUST-POLICY.md deadline clauses. Exercise unavailable/unbound/invited send+expiry, sparse/physical/restart, human/lazy exemptions and catch-up distinction before integration.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign-design.md",
      "acceptance_criteria": "Implement the corresponding spec contract and exercise all configurations named in this description with isolated owned children before terminal integration. Run focused tests and mandated per-change lint.",
      "spec_id": "docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--receipts-design.md",
      "status": "open",
      "priority": 2,
      "issue_type": "epic",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:45:44Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T22:57:30Z",
      "metadata": {
        "sp_depth": 1,
        "sp_order": 1
      },
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "parent-child"
    }
  ],
  "parent": "ht-0lo.1",
  "dependent_count": 2,
  "dependency_count": 3,
  "comment_count": 0,
  "revision": "-4092183272479722019"
}
```

### Acceptance criteria (verbatim)

Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass. Healthy-daemon discovery contract: retain the5,000ms safety tick and100candidate per-drive cap; continue immediately while successful due progress reports more. Each class advances a stable finite high-water cursor fairly; newly arriving obligations do not restart/starve the current snapshot. Native wake admission/completion never gates cursor progress. For a snapshot with at most N candidates per class and successful runnable driver quanta taking at most Q ms including storage/scheduling, durable pending-warning detection finishes within5,000ms +(2*ceil(N/100)+2)*Q; this is conditional on the daemon running and successful bounded storage, not a guarantee across outages, repeated errors, unbounded overload or native key submission. Restart resumes/reconstructs fair discovery and scans already overdue obligations. Early fake-clock tests use at least250candidates in EACH invitation and receipt class, blocked native wakes, mixed future/due rows, continuing arrivals and restart; verify every initial overdue row is warned within the formula and arrivals cannot starve either class. Historical upgrade also uses multiple bounded batches without losing overdue indexed discovery.

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

#### Message deadline contract

For every new durable ACK-required ordinary send, freeze the duration using existing validation/default configuration and set its original deadline to canonical committed send time plus that duration. This applies to every recipient, including no binding, managed-launch/unregistered binding, unavailable binding and pending invitation. Availability never changes the original start/deadline, and cannot reset it on registration, accept, restart, or rebind. Lazy messages and human receipt waivers remain free of ACK obligations. Receipt still means receipt only.

The effective receipt projection, manifest preparation, sparse physical materialization, due cursors, soft-poke calculations and wake paths must agree. Keep availability separately meaningful: setting a deadline is not permission to wake, nor evidence of a live harness. Do not pretend send time is observed availability. Existing structural/native execution launch guards and wake harness/empty-composer guards remain mandatory.

Upgrade outstanding pending ACK-required messages too: preserve every previously running deadline, but give formerly dormant obligations the deadline derived from their immutable message send time and frozen duration. Already expired rows become due on the normal bounded scan. Preserve terminal states, receipt provenance and warning identities. A transactionally recorded schema/semantics marker makes old and new manifest/physical projections consistent; materialize/index due work so unavailable recipients are actually discoverable, not merely capable of returning a deadline on individual lookup. Do not rewrite immutable requests or old result digests.

Existing explicit summary catch-up extensions in TRUST-POLICY A6 remain separate and visible; this redesign removes availability-based deferral, not the policy's bounded progress-based extension. Tests distinguish original deadline from an explicitly extended effective deadline. Warning delivery to the originating seat is durable even if it later leaves the thread; joined observers may retain existing warning visibility. An overdue warning says confirmation missed its deadline, never why. The original sender's open warning is waking attention under the existing composer/host/mod admission guards, even if the sender is no longer joined; clear remains passive and observer visibility is unchanged. For non-seat service authors preserve existing service reporting rather than inventing a sender seat. Update TRUST-POLICY A7 to record this narrow new waking recipient rule. Native wake can be deferred by a draft, active turn or unavailable host; the durable warning remains pending, so bounded deadline detection does not promise bounded native key submission. Late ACK clears the existing warning through normal lifecycle without retroactively changing the deadline.

#### Confirmed roast refinements

Healthy-daemon discovery contract: retain the5,000ms safety tick and100candidate per-drive cap; continue immediately while successful due progress reports more. Each class advances a stable finite high-water cursor fairly; newly arriving obligations do not restart/starve the current snapshot. Native wake admission/completion never gates cursor progress. For a snapshot with at most N candidates per class and successful runnable driver quanta taking at most Q ms including storage/scheduling, durable pending-warning detection finishes within5,000ms +(2*ceil(N/100)+2)*Q; this is conditional on the daemon running and successful bounded storage, not a guarantee across outages, repeated errors, unbounded overload or native key submission. Restart resumes/reconstructs fair discovery and scans already overdue obligations. Early fake-clock tests use at least250candidates in EACH invitation and receipt class, blocked native wakes, mixed future/due rows, continuing arrivals and restart; verify every initial overdue row is warned within the formula and arrivals cannot starve either class. Historical upgrade also uses multiple bounded batches without losing overdue indexed discovery.

Receipt semantic capability: advertise messages.send_time_receipts_v1 only when canonical send-time projection and its ordinary send handlers are active. Modern clients require this capability before every new ACK-required ordinary send or handoff durable/native effect; absent/refused discovery produces explicit unsupported-daemon error, with no silent legacy fallback. All CLI entry paths and service-capable client sends that promise this contract are covered. Old clients talking to an upgraded daemon receive canonical new-send semantics without modifying their old wire requests. Lazy/human-waived sends remain exempt; completed retained keyed historical operations remain inspectable/replayable with original bytes/digests/outcomes and are never rewritten. A pending historical retry on an unsupported daemon refuses before additional effects. Test newCLI/olddaemon refusal, oldCLI/newdaemon canonical send, current/current unbound sends and lazy/human exemptions using capability fixtures.

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

1. Add expiry/late ACK tests for unbound, unavailable, managed-launch/unregistered and pending-invite recipients, sender leave, service authors and unavailable native wake due to drafts/working/host outage. Durable warning presence is separate from actual wake submission.
2. Implement bounded indexed receipt due discovery over upgraded sparse/physical rows and sender-addressed open/clear routing. Open warning wakes only original sender under existing guards; clear passive; do not invent a seat for service authors. Update TRUST-POLICY A7.
3. Own shared due driver/cursor scheduling and combined fake-clock fairness tests using .2.2 invitation-class store API. Keep 5,000ms safety tick and 100-candidate per-drive cap; successful continuation runs immediately and alternates classes over stable finite high-water snapshots. Test at least 250 candidates EACH, future/due mix, blocked wakes, continuing arrivals, restart and conditional detection bound 5,000ms +(2*ceil(N/100)+2)*Q. Native wake completion cannot gate scanning.
4. Run focused receipt/poke/wake/scheduler tests and format/clippy; publish tested independent sender-warning lifecycle to .3.2.


### Authorized upgrade-aware acceptance refinement (supersedes prior formula)

Healthy-daemon discovery retains the 5,000ms safety tick and 100 actually inspected candidates per successful global due-driver quantum. Every two successful runnable global quanta give each unfinished receipt/invitation class one turn with a 100-candidate allowance; successful `more` continues immediately. All queries enforce finite captured high-water bounds before LIMIT, advance stable cursors, and count inspected rows even if future, terminal, waived or otherwise ineffective. Ordinary receipt scanning lends unused physical/sparse allowance so a non-final snapshot turn can consume its full allowance; native wake admission/completion never gates cursor progress. The 100 limit is a total row-inspection allowance, not 100 repaired rows plus 100 due rows.

During unfinished historical receipt repair, reserve at most ceil(limit/2) candidates for repair and the remaining allowance for ordinary due discovery (at least 50 at limit=100); lend unused repair allowance to due discovery. Advance persistent finite physical/prepared ordinal repair cursors without restarting ordinary due traversal every batch. Inspect each captured repair candidate at most once; published projection alone is repaired, while any physically visited unpublished prepared candidate still counts. Repair completion emits a one-shot `completed` transition and resets the ordinary receipt cursor before due selection in that same committed transaction. Repair progress, that completion/reset decision and due-warning effects commit or roll back together. This fresh post-repair traversal catches repaired deadlines behind a previously advanced due cursor. Empty repair finalization uses zero row allowance and shares the ordinary turn at full capacity. Failed decisions cannot consume the completion transition. Standalone backfill callers retaining a due cursor must honor `completed` too.

Bound definitions: start at a fresh paired discovery cycle with the target obligations already overdue, or restart with reconstructed finite cursors. H is an upper bound on ALL actually inspected candidates in the captured historical deadline-repair physical/prepared AND projection-bootstrap physical/sparse workload (counting duplicate visits separately), including future/terminal/waived/unpublished candidates; it is not the number changed. D bounds ALL inspected ordinary receipt candidates in the complete fresh post-repair snapshot, including repaired rows, duplicate physical/sparse projections and finite arrivals captured before that snapshot freezes. I bounds ALL inspected invitation candidates in its initial finite snapshot, including future or subsequently ineffective rows. Q bounds elapsed time of one successful runnable GLOBAL driver quantum, including storage, scheduling and other admitted work. Later insertions outside captured high waters cannot enter or restart those walks, even when their deadlines are earlier. A finite D bound must cover any arrivals before the post-repair snapshot freezes; unbounded arrivals during repair do not justify using only the initial receipt count.

When H=0, empty repair bookkeeping completes within the first ordinary turn without reducing its capacity. With D and I at most N, the original two-class bound remains 5,000ms + (2*ceil(N/100)+2)*Q. For H>0, set R=ceil(H/50)+2: the extra two receipt turns conservatively cover finite-stream/end probes and repair completion. After those R receipt turns, at most ceil(D/100)+1 further receipt turns finish the fresh post-repair snapshot; the +1 covers a full final page's empty completion. Invitations need at most ceil(I/100)+1 invitation turns. Fair alternation therefore gives the conservative GLOBAL bound:

`5,000ms + 2*max(R + ceil(D/100)+1, ceil(I/100)+1)*Q` for H>0.

The bound concerns durable warning detection for still-pending, unextended target obligations, not bounded native key submission. It is conditional on successful bounded storage/driver service, the stated finite workload/snapshot limits and Q; it does not cover outages, repeated errors, unbounded overload, newly expiring rows already passed by a snapshot cursor, or arbitrary arrivals outside the counted snapshots. Such rows belong to a subsequent fresh cycle. Restart resumes or reconstructs finite traversal without losing repair progress; a newly reconstructed traversal must count its inspected workload again. Tests must measure actual inspected work and global quanta, rather than changed rows or receipt-only calls.

Task5 verifies the producer locally: multiple batches, finite captured high waters, late earlier-deadline arrivals excluded from repair, mixed future/terminal/waived/unpublished candidates, all-terminal old backlog, one-shot atomic completion/reset, and rollback/restart. Task7 verifies the global conversion with at least 250 candidates in EACH receipt and invitation class, both H=0 and H>0, blocked native wakes, mixed future/due rows, continuing earlier-deadline arrivals, actual <=100 total inspections per quantum, immediate successful continuation, early ordinary-due progress before long repair exhaustion, final post-repair discovery and restart. Task7 owns normal due-query high-water enforcement/full allowance lending and shared fairness; Task5 does not claim the combined bound from receipt-only tests.


### Authorized coalesced multi-originator warning acceptance refinement

Receipt-overdue warning conditions remain coalesced by their existing thread/recipient backlog identity. Preserve exactly one canonical open event for that condition, exact physical/manifest receipt-to-warning associations, and the existing final clear event. Never manufacture per-sender warning conditions/events or derive original senders from current thread membership, the warning event's first source message alone, or a later caller. Enumerate canonical original seat authors of ALL messages exactly associated with the coalesced warning via receipts/receipt_state.warning_message_id; retain existing service-author handling without inventing a seat.

For EACH canonical original sender of an exactly associated receipt, the still-open shared condition must participate in actual native waking-attention selection and mod/watch pending-set/fingerprint paths under existing admission guards, including a sender who left the thread. A successful test of notice-row insertion or inbox history alone is insufficient. A later receipt association from another original sender must become actionable even when it reuses an older open event and an informational notice-offer frontier has already passed that event's sequence. Do not conflate informational offer settlement with the canonical open-condition/associated-origin state. Preserve the existing per-occupant wake-offer deduplication and safe composer/host/harness admission: this refinement does not authorize repeated native submission for an unchanged already-offered actionable state, new receipt provenance, or a task-adoption/completion claim. Cross-path attention versions/fingerprints must represent newly actionable associations honestly rather than inventing a new canonical warning event.

Existing partial settlement semantics are verified in schema::clear_warning_conditions_for_receipts: the condition stays open while ANY exact associated receipt remains required/pending and not human-waived. ACK/waiver/terminal settlement of a subset must preserve all remaining origins' obligations and waking eligibility. Do not emit a condition clear, withdraw another origin's pending warning, or rewrite settled receipt/ACK provenance merely because the first event's source receipt settled. Historical informational delivery remains historical. Only settlement of the final required/pending/unwaived associated receipt clears the shared condition, once, through its existing passive-clear lifecycle; clears/observer notices create no waking attention. Routing preserves the backlog-wide open-condition lifetime; this refinement does not mandate quieting an individual sender after only its own receipts settle. All canonical associated original senders remain covered while the shared condition is open.

Task7 fixture/review requirements: construct at least TWO canonical original senders and THREE exact associated receipts sharing the same thread/recipient condition (for example A1,A2 from sender A and B1 from sender B), with at least one original sender leaving before warning routing. Exercise physical and published-manifest associations, mixed original authors and repeat scans. Prove exactly one open condition/event and all exact associations. Advance informational notice-offer frontiers, then coalesce a receipt from the other original sender; show actual native selection and mod/watch state/fingerprint expose the new origin under normal guards. Partially ACK A1 and prove A2/B1 origins remain actionable; settle A2 (the event's first origin now has no pending associated receipt) and prove B1 remains actionable without premature clear or extra open event. Finally settle B1 and prove one passive clear, no warning-only native/mod wake, unchanged exact ACK actor/generation/proof/history, and no synthetic receipt/adoption evidence. Per-occupant offer deduplication and blocking admission still apply independently to each origin. Existing H/D/I upgrade discovery bound refinement remains normative and unchanged.


### Required Task7 actual-discovery and shared early-gates acceptance refinement

For EVERY canonical original message sender in the coalesced-warning fixture, exercise the actual `discover_wake_candidates`/public wake-candidate discovery path before offer/load/reserve and native delivery; also exercise the real mod/watch path. Predicate-only `warning_wakes_seat`, direct attention calculation, hand-built WakeCandidate and direct reserve-only GREEN do not establish delivery/discovery. The fixture must leave original senders with the still-open condition as their sole waking source: settle informational offers, avoid unrelated pending invitation/receipt/notice rows masking discovery, include a sender who left, and coalesce messages from at least two canonical origins under one shared thread/recipient open warning. Verify actual discovery returns EACH original sender, then that downstream candidate/offer/load/reserve/native and mod fingerprints retain the open-origin work independently of informational notice frontiers. This must cover originators whose association is not the first warning event's source message. Existing per-occupant/reservation retry and native safety guards remain, but a generic informational `notice_frontier` or legacy warning-offer check must not silently discard this independent canonical open-origin source.

Fresh Task7 review MUST enumerate the full shared EARLY GATES chain, with source locations and real tests: (1) same-instance live non-human seat ordinal/high-water walk, page/continuation/budget limits; (2) `attention::seat_has_pending_rows` and actual indexed `WAKE_PROBES` inclusion of every canonical receipt-origin source, not just downstream predicates; (3) source publication/association, exact canonical author/instance attribution and remaining shared-condition lifetime; (4) bounded index-leading origin enumeration with SQL-enforced finite ordinal/high-water scope and honest inspection/continuation accounting, no full historical scan or first-event-only shortcut; (5) canonical attention pending-set narrowing, legacy/informational notice frontiers and independent open-origin selection; (6) candidate loading, current binding/occupant, actionable-work and offer-frontier gates; (7) canonical witness/freshness/reservation validation and native/mod admission. An origin source missing from ANY upstream exclusion invalidates the acceptance claim even if direct attention/reserve helpers pass. Task6's round1 rereview found exactly this gap at `seat_has_pending_rows`: direct downstream GREEN was not actual native discovery clearance.

Use isolated fake-host/test-support fixtures to verify draft/composer content, active working turn and unavailable host defer native key submission under the existing guards while the durable open warning and recoverable discovery/mod source remain present. After each blocker clears, the actual discovery→offer/load/reserve/native path must resume for the correct sender without recreating the canonical warning or losing another sender's origin. Do not interpret a blocked native submission as absent durable warning, remove native safety guards, or claim bounded key submission. Check exact canonical source associations, no synthetic ACK/adoption/completion, and existing receipt actor/generation/proof/history. Audit dynamic coalescing after informational settlement, partial ACK while another associated receipt remains, and final passive clear through the actual paths rather than only stored rows.

Preserve all settled Task7 requirements: one canonical shared condition/open event, origins derived from ALL exactly associated canonical messages (including departed senders), no individual quieting mandate after own ACK, same open condition while ANY required/pending/unwaived association remains, final single passive clear, and the full H/D/I upgrade-aware discovery bound/accounting/fairness contract. This is a tracked acceptance refinement on the existing owner, not a new leaf or responsibility transfer from Task6's invitation fix. Task7's implementation/reviewer must report every audited early gate and its actual path evidence explicitly before declaring sender notification complete.


### Concrete shared early-gate source admission

Coordinator explicitly admits `src/store/attention.rs` to Task7 writes: it owns `WAKE_PROBES`, `seat_has_pending_rows` and native/mod open-warning attention projection. The controller updates its cumulative mapping separately. No additional test paths are admitted; existing receipt/wake/wake-discovery/scheduler fixtures may suffice, and the implementer reports any actual additional need before writing. Task6’s invitation-origin index represents a single origin per invitation; a shared receipt warning condition can have multiple canonical message senders. Task7 must cover ALL those canonical senders through actual discovery and native/mod paths, without treating the first event source or a single invitation origin as the complete receipt audience. This admission imposes no architecture and creates no new implementation leaf. All preceding H/D/I, coalesced-condition partial/final settlement and full-discovery/early-gates requirements remain unchanged.


### Bounded projection bootstrap refinement (Task7)

Migration35 captures only indexed MAX physical receipt ordinal and raw sparse receipt_state rowid plus durable progress/completion; it performs no historical INSERT SELECT scan. Immediate canonical association/eligibility triggers cover new work. Bounded receipt-turn repair bootstraps historical open-origin associations and monotonic sparse due projections from genuine old rows. H includes ALL visits from Task5 physical/prepared deadline repair AND migration35 physical/sparse bootstrap; duplicate physical visits count again, as do terminal/future/waived/unpublished or otherwise ineffective rows actually visited. D and I include every actually traversed snapshot candidate, including future/lazy/non-due candidates whenever visited; neither means only warnings emitted or currently due rows.

All repair producers share the same at-most50 repair slots within the100 receipt-turn allowance. Unused quota and zero-row end probes lend within that same quantum; there is no hidden phase turn or second safety tick. The existing R=ceil(H/50)+2 conservative envelope is retained only with that measured full-allowance/end-probe proof. One combined global completion/reset occurs atomically only after ALL repair/bootstrap producers finish, before same-transaction ordinary due selection. Existing Task5 producer-local completion semantics remain intact; a local completion cannot prematurely consume the combined global reset. Failed decisions roll back progress, completion/reset and warning effects together. H=0 empty bookkeeping leaves the full100 ordinary slots in its first turn.

Fresh Task7 and integration review must prove genuine34→35 populated rollback/reopen, every preexisting eligible sparse obligation and associated open origin reconstructed without startup-wide scan, all visited work/lending/end probes, combined reset exactly once, all-terminal old backlog, continuing earlier-deadline arrivals, and deletion/reinsertion/rowid reuse during bootstrap. Triggers give newly eligible sparse candidates fresh monotonic ordinals; historical raw rowid reuse cannot lose obligations or rewrite canonical receipt/sender identity. Native wake admission remains independent. The conditional H/D/I bound and original H=0 formula remain unchanged, with expanded H honestly measured.

## Task 8: Invitation CLI and presentation contract

filesTouched: ["src/cli/commands.rs", "src/cli/mod.rs", "src/cli/threads.rs", "src/cli/output.rs", "src/cli/retry.rs", "integrations/skill/SKILL.md", "README.md", "tests/cli/commands.rs", "tests/cli/output.rs", "tests/cli/skill.rs", "tests/cli/guidance.rs"]

bead: ht-0lo.2.3

Dependencies: ht-0lo.2.2

### Full bead text (canonical snapshot)

```json
{
  "id": "ht-0lo.2.3",
  "title": "Invitation CLI and presentation contract",
  "description": "owns: ordinary invite --lazy default, --require-ack/--deadline implication and conflicts, compact/JSON/inbox/help/embedded skill presentation; consumes: invitation confirmation protocol/store contract. Files: src/cli parser invitation branches/mod/output and help/skill resources, relevant CLI tests. Invoke actual CLI in isolated daemon/config for bare lazy, require-ack default duration, explicit deadline, conflicting flags, duplicate and historical keyed retry. Required/operator commands remain unchanged.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--invitations-design.md\nblocked-by ht-0lo.2.2: consumes invitation confirmation store behavior",
  "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass.",
  "status": "open",
  "priority": 2,
  "issue_type": "task",
  "owner": "235553+alepar@users.noreply.github.com",
  "created_at": "2026-10-10T22:48:15Z",
  "created_by": "Alexey Parfenov",
  "updated_at": "2026-10-10T22:48:33Z",
  "labels": [
    "sp:ht-0lo"
  ],
  "dependencies": [
    {
      "id": "ht-0lo.2.2",
      "title": "Invitation acceptance, due and delivery lifecycle",
      "description": "owns: canonical lazy/timed invitation create/duplicate/accept/reject/cancel/retire behavior, due scanner and inviter-addressed warning routing plus read/wake/mod projections; consumes: invitation mode schema/wire contract. Files: src/store/{control,invitation_due,wake,queries,service_controls,seats}.rs as needed, daemon projections, TRUST-POLICY invitation clauses and focused tests. Run lazy invite no native wake/deadline, timed never-register and register-without-accept expiry, acceptance clears, duplicate never mutates policy, inviter leaving warning, required membership and explicit rejection notice cases.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--invitations-design.md\nblocked-by ht-0lo.2.1: consumes invitation mode schema/wire contract\nOriginal inviter attribution frozen at invitation creation; overdue inviter open warning waking under existing native/mod guards even after leaving, clear passive; update TRUST-POLICY A7. Duplicate caller never retargets originator warning.\nHealthy-daemon discovery contract: retain the5,000ms safety tick and100candidate per-drive cap; continue immediately while successful due progress reports more. Each class advances a stable finite high-water cursor fairly; newly arriving obligations do not restart/starve the current snapshot. Native wake admission/completion never gates cursor progress. For a snapshot with at most N candidates per class and successful runnable driver quanta taking at most Q ms including storage/scheduling, durable pending-warning detection finishes within5,000ms +(2*ceil(N/100)+2)*Q; this is conditional on the daemon running and successful bounded storage, not a guarantee across outages, repeated errors, unbounded overload or native key submission. Restart resumes/reconstructs fair discovery and scans already overdue obligations. Early fake-clock tests use at least250candidates in EACH invitation and receipt class, blocked native wakes, mixed future/due rows, continuing arrivals and restart; verify every initial overdue row is warned within the formula and arrivals cannot starve either class. Historical upgrade also uses multiple bounded batches without losing overdue indexed discovery.",
      "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass. Healthy-daemon discovery contract: retain the5,000ms safety tick and100candidate per-drive cap; continue immediately while successful due progress reports more. Each class advances a stable finite high-water cursor fairly; newly arriving obligations do not restart/starve the current snapshot. Native wake admission/completion never gates cursor progress. For a snapshot with at most N candidates per class and successful runnable driver quanta taking at most Q ms including storage/scheduling, durable pending-warning detection finishes within5,000ms +(2*ceil(N/100)+2)*Q; this is conditional on the daemon running and successful bounded storage, not a guarantee across outages, repeated errors, unbounded overload or native key submission. Restart resumes/reconstructs fair discovery and scans already overdue obligations. Early fake-clock tests use at least250candidates in EACH invitation and receipt class, blocked native wakes, mixed future/due rows, continuing arrivals and restart; verify every initial overdue row is warned within the formula and arrivals cannot starve either class. Historical upgrade also uses multiple bounded batches without losing overdue indexed discovery.",
      "status": "open",
      "priority": 2,
      "issue_type": "task",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:48:14Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T23:18:51Z",
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "blocks"
    },
    {
      "id": "ht-0lo.2",
      "title": "Invitation lifecycle",
      "description": "owns: invitation mode/duration/acceptance contract, protocol capability and inviter warning routing through CLI/store/query/wake; consumes: existing membership lifecycle. Files: src/protocol/{commands,results,capabilities}.rs and wire negotiation, src/cli parser invitation branches, src/store/{control,invitation_due,wake,queries,schema}.rs, relevant tests, TRUST-POLICY invitation clauses. Exercise bare lazy invite, explicit timed invite, never-register expiry, register-without-accept expiry, acceptance/rejection, duplicates, legacy retry and required memberships before integration.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign-design.md",
      "acceptance_criteria": "Implement the corresponding spec contract and exercise all configurations named in this description with isolated owned children before terminal integration. Run focused tests and mandated per-change lint.",
      "spec_id": "docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--invitations-design.md",
      "status": "open",
      "priority": 2,
      "issue_type": "epic",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:45:45Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T22:57:31Z",
      "metadata": {
        "sp_depth": 1,
        "sp_order": 2
      },
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "parent-child"
    }
  ],
  "parent": "ht-0lo.2",
  "dependent_count": 1,
  "dependency_count": 2,
  "comment_count": 0,
  "revision": "-5326939878451608756"
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

#### Invitation contract

Ordinary agent invitations default to Lazy: membership is pending and discoverable through existing inbox/hook opportunities, but the invitation itself requests no timed confirmation or native wake. Add invitation `--require-ack` and optional `--deadline SECONDS`; explicit deadline implies require-ack for ergonomic compatibility, and `--lazy` conflicts with either. An ACK-required invitation starts its acceptance deadline at canonical invitation creation. Only explicit recipient acceptance settles it automatically; no separate invitation ACK command and no registration settlement. Expiry notifies its originating seat with invitation deadline missed, regardless of failure to register or accept. The inviter's open warning is waking attention under existing safe composer/host/mod guards even after leaving; clear remains passive and existing debtor/observer visibility remains. Originator attribution is frozen from the original invitation episode, never inferred from a current member or a later duplicate caller. Rejection/cancellation/retirement retain existing truthful terminal outcomes and attributed rejection notices; lazy does not suppress explicit rejection notices.

Existing required service memberships and explicit human/operator invitation semantics remain unchanged. Existing ordinary pending invitations retain their current timed obligation; historical retry with omitted new fields retains its original interpretation. Use an additive optional delivery/confirmation field whose absence denotes legacy timed behavior; modern CLI explicitly emits Lazy for the new default. Persist requested timing and original inviter attribution independently of joined membership. Pending-invite duplicate calls return the existing invitation without silently upgrading/downgrading its deadline or waking it; changing obligation policy requires a new episode through existing disposition flow. Already joined is still AlreadyJoined, never a new obligation. Existing human mode cannot manufacture an agent ACK requirement.

Queries, inbox compact/machine/JSON, digest, mod/native wake selection and due scans all honor the requested mode. Lazy presentation is bookkeeping only. ACK-required invitation attention remains ordinary guarded delivery; accept remains an accountable recipient choice. Add schema fields/index predicate changes conservatively and negotiate protocol capability/version before any mutation requiring the new representation. Older saved intent bytes/digests are preserved and replayed unchanged; unsupported daemon refuses new semantics before writing.

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

1. Write actual isolated CLI tests for bare lazy invite, --require-ack default duration, --deadline implies confirmation, --lazy conflict with either timed option, duplicate unchanged policy, historical keyed retry and unchanged required/operator commands.
2. Select explicit modern Lazy in ordinary invitation CLI by default; freeze requested mode/duration before journal submission and require the negotiated representation capability before writes. Preserve absent historical saved fields and old child digests.
3. Update compact/machine/JSON/inbox/help/embedded skill presentation to represent lazy versus timed invitations without claiming acceptance or registration receipt. Keep pending invitations discoverable while suppressing timed wake for lazy.
4. Run focused CLI/parser/output/skill tests and format/clippy; coordinate common docs with .3.2/.4.2 through serial merge rather than broad rewrites.

## Task 9: Owned stable registration migration and tooling

filesTouched: ["src/harness/setup.rs", "src/harness/claude/setup.rs", "src/harness/codex/setup.rs", "src/cli/setup.rs", "src/cli/setup/foreground.rs", "src/cli/launch.rs", "src/harness/launch.rs", "src/harness/claude_mod.rs", "tests/harness/setup.rs", "tests/harness/launch.rs", "tests/harness/claude_mod.rs", "tests/setup_cli.rs", "tests/integration/setup_canary_seam.rs", "tests/canary_manifest_writer.rs", "scripts/canary/manifest.py", "scripts/canary/test_manifest.py", "integrations/skill/SKILL.md", "integrations/claude/README.md", "integrations/codex/README.md"]

bead: ht-0lo.4.2

Dependencies: ht-0lo.4.1

### Full bead text (canonical snapshot)

```json
{
  "id": "ht-0lo.4.2",
  "title": "Owned stable registration migration and tooling",
  "description": "owns: manifest-recorded exact ownership/install identity, repeat setup in place, recoverable UUID/shared-command migration, safe unsetup and status/trust/canary/launcher command compatibility; consumes: stable native per-event command grammar. Files: src/harness/setup.rs and claude/codex setup modules, src/cli setup/launch consumers, canary/tooling/docs/tests. Invoke isolated Claude/Codex setup-repeat/migration/interruption/unsetup with foreign identical/drifted groups and ensure no ownership theft, no trust writes, no group shifting on equivalent reinstall. Hermes/Claude mod regressions stay passing.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--hooks-design.md\nblocked-by ht-0lo.4.1: consumes stable native per-event command grammar",
  "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass.",
  "status": "open",
  "priority": 2,
  "issue_type": "task",
  "owner": "235553+alepar@users.noreply.github.com",
  "created_at": "2026-10-10T22:48:18Z",
  "created_by": "Alexey Parfenov",
  "updated_at": "2026-10-10T22:48:31Z",
  "labels": [
    "sp:ht-0lo"
  ],
  "dependencies": [
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
      "dependency_type": "blocks"
    },
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
  "dependent_count": 1,
  "dependency_count": 2,
  "comment_count": 0,
  "revision": "-5021341820112912234"
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

1. Add isolated Claude/Codex ownership fixtures for equivalent repeat setup preserving group position, legacy UUID/shared-event owned migration, interrupted transition/recovery, foreign identical and drifted groups, safe unsetup and status/trust inspection.
2. Consume .4.1 exact per-event command grammar while retaining manifest installation identity, full exact owned groups/fingerprints and conservative foreign-group refusal. Migrate only under explicit setup/doctor-fix intent; preserve unrelated keys/group order and never edit native trust records/hooks.state.
3. Update shared-command consumers/status/trust/canary/launcher expectations to compare actual per-event commands or derive canonical route as appropriate. Keep Hermes transport and Claude mod launch route behavior unchanged. Publish owned-configuration fingerprint representation to .3.2 without weakening its safety.
4. Run focused setup/launch/mod/canary tests and format/clippy. All native configuration fixtures use isolated HOME/CLAUDE_CONFIG_DIR/CODEX_HOME; no real configs.

## Task 10: Handoff confirmation policy wiring and recovery documentation

filesTouched: ["src/cli/commands.rs", "src/cli/handoff.rs", "src/cli/handoff_delivery.rs", "src/cli/topology_handoff.rs", "src/cli/mod.rs", "src/cli/launch.rs", "src/harness/launch.rs", "src/protocol/handoff.rs", "src/store/handoff.rs", "src/store/topology_handoff.rs", "src/store/topology_handoff/attachment.rs", "src/store/topology_handoff/runtime.rs", "tests/cli/handoff.rs", "tests/cli/handoff_delivery.rs", "tests/cli/topology_handoff.rs", "tests/cli/launch.rs", "tests/harness/launch.rs", "tests/handoff_topology_cli.rs", "tests/handoff_topology_authority.rs", "integrations/skill/SKILL.md", "README.md", "src/ports.rs", "src/harness/adapter.rs", "src/harness/claude.rs", "src/harness/codex.rs", "tests/cli/follow_retry.rs", "tests/integration/channel_activation.rs", "tests/integration/topology_activation.rs", "tests/lifecycle_ux.rs", "tests/store/archival_legacy.rs", "tests/store/topology_handoff.rs", "src/harness/hermes/launch.rs", "src/test_support/mod.rs", "src/host/native.rs", "tests/contracts.rs", "TRUST-POLICY.md", "src/harness/hermes/runtime.rs", "tests/cli/cooperative.rs", "tests/store/topology_attachment_guard.rs", "src/harness/registry.rs"]

bead: ht-0lo.3.2

Dependencies: ht-0lo.1.1, ht-0lo.2.2, ht-0lo.2.1, ht-0lo.1.2, ht-0lo.3.1

### Full bead text (canonical snapshot)

```json
{
  "id": "ht-0lo.3.2",
  "title": "Handoff confirmation policy wiring and recovery documentation",
  "description": "owns: handoff --deadline/--require-invite-ack/--invite-deadline parsing/freeze/pass-through, modern lazy invitation plus addressed ACK-required body staging across new-tab/empty-pane/existing paths, retained child digest compatibility and source/help/skill docs; consumes: frozen handoff payload, invitation confirmation wire contract and send-time receipt semantics. Files: src/cli/{handoff,handoff_delivery,topology_handoff,mod}.rs, src/protocol/handoff.rs, src/store/{handoff,topology_handoff}.rs and related tests/help/skill. Run isolated actual handoff per all three paths/body sources; crash/retry after stage and possible-start/unknown-create never duplicates/restages/relaunches. Older plans absent new fields replay original keyed requests, modern deadlines freeze before staging; routing exact3field fallback and no auto trust/accept/ACK.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--handoff-design.md\nblocked-by ht-0lo.3.1: consumes frozen handoff payload\nblocked-by ht-0lo.2.1: consumes invitation confirmation wire contract\nblocked-by ht-0lo.1.1: consumes send-time receipt semantics\nAlso owns: separating hook/setup/trust/profile readiness diagnostics from launch admission; src/cli/launch.rs prepare_launch and src/harness/launch.rs configuration checks. Missing/untrusted/wrong-profile hooks do not block staged work or otherwise structurally safe launch. Preserve empty-shell/seat/caller/native argv and owned launch-configuration fingerprint safety, and truthful native trust refusal; never auto-approve. Test missing/stale profile hooks through guarded handoff and observe durable ACK deadline warning without registration.\nblocked-by ht-0lo.2.2: consumes canonical invitation lifecycle\nblocked-by ht-0lo.1.2: consumes overdue receipt warning lifecycle\nHook readiness is advisory: keep owned launch configuration fingerprint recheck only when that configuration is actually used; absence cannot veto and no fingerprint is fabricated. Unsupported environment-overrides/native grammar/host-kind safety refusals remain.\nRequire messages.send_time_receipts_v1 before new handoff effects; consume ht-0lo.1.1 capability contract and verify unsupported-daemon refusal before staging/launch.",
  "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass. Observe durable overdue warning for an unregistered recipient (needs: ht-0lo.1.2). Require messages.send_time_receipts_v1 before new handoff effects; consume ht-0lo.1.1 capability contract and verify unsupported-daemon refusal before staging/launch.",
  "status": "open",
  "priority": 2,
  "issue_type": "task",
  "owner": "235553+alepar@users.noreply.github.com",
  "created_at": "2026-10-10T22:48:20Z",
  "created_by": "Alexey Parfenov",
  "updated_at": "2026-10-10T23:18:53Z",
  "labels": [
    "sp:ht-0lo"
  ],
  "dependencies": [
    {
      "id": "ht-0lo.3",
      "title": "Handoff payload",
      "description": "owns: full frozen handoff payload and policy integration; consumes: send-time receipt semantics and invitation confirmation wire contract. Files: src/cli handoff paths, src/protocol/handoff.rs, src/store handoff paths and tests. Spec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--handoff-design.md",
      "acceptance_criteria": "Implement the corresponding spec contract and exercise all configurations named in this description with isolated owned children before terminal integration. Run focused tests and mandated per-change lint.",
      "spec_id": "docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--handoff-design.md",
      "status": "open",
      "priority": 2,
      "issue_type": "epic",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:45:46Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T22:57:32Z",
      "metadata": {
        "sp_depth": 1,
        "sp_order": 4
      },
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "parent-child"
    },
    {
      "id": "ht-0lo.1.1",
      "title": "Canonical send-time receipt projection",
      "description": "owns: receipt original deadline semantics, preserving running deadlines and terminal provenance; consumes: existing immutable send timestamps/durations. Files: src/store/{effective,materialization,schema,messages}.rs and preparation modules, TRUST-POLICY deadline clauses, focused store tests. Cover sparse and physical receipts, restart, lazy/human waiver, registration not resetting and explicit A6 extension distinction.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--receipts-design.md\nReceipt semantic capability: advertise messages.send_time_receipts_v1 only when canonical send-time projection and its ordinary send handlers are active. Modern clients require this capability before every new ACK-required ordinary send or handoff durable/native effect; absent/refused discovery produces explicit unsupported-daemon error, with no silent legacy fallback. All CLI entry paths and service-capable client sends that promise this contract are covered. Old clients talking to an upgraded daemon receive canonical new-send semantics without modifying their old wire requests. Lazy/human-waived sends remain exempt; completed retained keyed historical operations remain inspectable/replayable with original bytes/digests/outcomes and are never rewritten. A pending historical retry on an unsupported daemon refuses before additional effects. Test newCLI/olddaemon refusal, oldCLI/newdaemon canonical send, current/current unbound sends and lazy/human exemptions using capability fixtures.",
      "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass. Receipt semantic capability: advertise messages.send_time_receipts_v1 only when canonical send-time projection and its ordinary send handlers are active. Modern clients require this capability before every new ACK-required ordinary send or handoff durable/native effect; absent/refused discovery produces explicit unsupported-daemon error, with no silent legacy fallback. All CLI entry paths and service-capable client sends that promise this contract are covered. Old clients talking to an upgraded daemon receive canonical new-send semantics without modifying their old wire requests. Lazy/human-waived sends remain exempt; completed retained keyed historical operations remain inspectable/replayable with original bytes/digests/outcomes and are never rewritten. A pending historical retry on an unsupported daemon refuses before additional effects. Test newCLI/olddaemon refusal, oldCLI/newdaemon canonical send, current/current unbound sends and lazy/human exemptions using capability fixtures.",
      "status": "open",
      "priority": 2,
      "issue_type": "task",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:48:10Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T23:18:44Z",
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "blocks"
    },
    {
      "id": "ht-0lo.2.2",
      "title": "Invitation acceptance, due and delivery lifecycle",
      "description": "owns: canonical lazy/timed invitation create/duplicate/accept/reject/cancel/retire behavior, due scanner and inviter-addressed warning routing plus read/wake/mod projections; consumes: invitation mode schema/wire contract. Files: src/store/{control,invitation_due,wake,queries,service_controls,seats}.rs as needed, daemon projections, TRUST-POLICY invitation clauses and focused tests. Run lazy invite no native wake/deadline, timed never-register and register-without-accept expiry, acceptance clears, duplicate never mutates policy, inviter leaving warning, required membership and explicit rejection notice cases.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--invitations-design.md\nblocked-by ht-0lo.2.1: consumes invitation mode schema/wire contract\nOriginal inviter attribution frozen at invitation creation; overdue inviter open warning waking under existing native/mod guards even after leaving, clear passive; update TRUST-POLICY A7. Duplicate caller never retargets originator warning.\nHealthy-daemon discovery contract: retain the5,000ms safety tick and100candidate per-drive cap; continue immediately while successful due progress reports more. Each class advances a stable finite high-water cursor fairly; newly arriving obligations do not restart/starve the current snapshot. Native wake admission/completion never gates cursor progress. For a snapshot with at most N candidates per class and successful runnable driver quanta taking at most Q ms including storage/scheduling, durable pending-warning detection finishes within5,000ms +(2*ceil(N/100)+2)*Q; this is conditional on the daemon running and successful bounded storage, not a guarantee across outages, repeated errors, unbounded overload or native key submission. Restart resumes/reconstructs fair discovery and scans already overdue obligations. Early fake-clock tests use at least250candidates in EACH invitation and receipt class, blocked native wakes, mixed future/due rows, continuing arrivals and restart; verify every initial overdue row is warned within the formula and arrivals cannot starve either class. Historical upgrade also uses multiple bounded batches without losing overdue indexed discovery.",
      "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass. Healthy-daemon discovery contract: retain the5,000ms safety tick and100candidate per-drive cap; continue immediately while successful due progress reports more. Each class advances a stable finite high-water cursor fairly; newly arriving obligations do not restart/starve the current snapshot. Native wake admission/completion never gates cursor progress. For a snapshot with at most N candidates per class and successful runnable driver quanta taking at most Q ms including storage/scheduling, durable pending-warning detection finishes within5,000ms +(2*ceil(N/100)+2)*Q; this is conditional on the daemon running and successful bounded storage, not a guarantee across outages, repeated errors, unbounded overload or native key submission. Restart resumes/reconstructs fair discovery and scans already overdue obligations. Early fake-clock tests use at least250candidates in EACH invitation and receipt class, blocked native wakes, mixed future/due rows, continuing arrivals and restart; verify every initial overdue row is warned within the formula and arrivals cannot starve either class. Historical upgrade also uses multiple bounded batches without losing overdue indexed discovery.",
      "status": "open",
      "priority": 2,
      "issue_type": "task",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:48:14Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T23:18:51Z",
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "blocks"
    },
    {
      "id": "ht-0lo.2.1",
      "title": "Invitation confirmation protocol and schema contract",
      "description": "owns: additive invitation delivery/confirmation wire fields, capability refusal and schema fields/indexes/legacy defaults; consumes: existing operation digests and membership episodes. Files: src/protocol commands/results/capabilities or existing capability module, src/store/schema.rs and schema tests. Absent historical fields mean legacy timed behavior; explicit modern Lazy requests retain timing/inviter origin without altering old bytes. Freeze duration at creation, preserve required and human invitation semantics.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--invitations-design.md",
      "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass.",
      "status": "open",
      "priority": 2,
      "issue_type": "task",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:48:13Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T22:48:13Z",
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "blocks"
    },
    {
      "id": "ht-0lo.1.2",
      "title": "Due discovery and originating-seat warning lifecycle",
      "description": "owns: bounded indexed due discovery plus sender-addressed open/clear warning routing and guarded poke/wake calculations; consumes: send-anchored canonical receipt projection. Files: src/store/{receipts,poke,wake,queries,schema}.rs, scheduler tests. Exercise unbound/unregistered/unavailable/pending-invite recipient sends through expiry and late ACK; originator leaving does not lose notification.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--receipts-design.md\nblocked-by ht-0lo.1.1: consumes send-anchored canonical receipt projection\nblocked-by ht-0lo.1.3: consumes historical dormant receipt upgrade\nOriginator open warning is waking attention under existing safe composer/host/mod guards even after leaving; clear passive; update TRUST-POLICY A7. No invented seat for service authors. Test notification pending while drafts/working/host outage defer actual native wake.\nHealthy-daemon discovery contract: retain the5,000ms safety tick and100candidate per-drive cap; continue immediately while successful due progress reports more. Each class advances a stable finite high-water cursor fairly; newly arriving obligations do not restart/starve the current snapshot. Native wake admission/completion never gates cursor progress. For a snapshot with at most N candidates per class and successful runnable driver quanta taking at most Q ms including storage/scheduling, durable pending-warning detection finishes within5,000ms +(2*ceil(N/100)+2)*Q; this is conditional on the daemon running and successful bounded storage, not a guarantee across outages, repeated errors, unbounded overload or native key submission. Restart resumes/reconstructs fair discovery and scans already overdue obligations. Early fake-clock tests use at least250candidates in EACH invitation and receipt class, blocked native wakes, mixed future/due rows, continuing arrivals and restart; verify every initial overdue row is warned within the formula and arrivals cannot starve either class. Historical upgrade also uses multiple bounded batches without losing overdue indexed discovery.",
      "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass. Healthy-daemon discovery contract: retain the5,000ms safety tick and100candidate per-drive cap; continue immediately while successful due progress reports more. Each class advances a stable finite high-water cursor fairly; newly arriving obligations do not restart/starve the current snapshot. Native wake admission/completion never gates cursor progress. For a snapshot with at most N candidates per class and successful runnable driver quanta taking at most Q ms including storage/scheduling, durable pending-warning detection finishes within5,000ms +(2*ceil(N/100)+2)*Q; this is conditional on the daemon running and successful bounded storage, not a guarantee across outages, repeated errors, unbounded overload or native key submission. Restart resumes/reconstructs fair discovery and scans already overdue obligations. Early fake-clock tests use at least250candidates in EACH invitation and receipt class, blocked native wakes, mixed future/due rows, continuing arrivals and restart; verify every initial overdue row is warned within the formula and arrivals cannot starve either class. Historical upgrade also uses multiple bounded batches without losing overdue indexed discovery.",
      "status": "open",
      "priority": 2,
      "issue_type": "task",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:48:11Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T23:18:46Z",
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "blocks"
    },
    {
      "id": "ht-0lo.3.1",
      "title": "Handoff instruction source acquisition and frozen payload",
      "description": "owns: positional/file/stdin source exclusivity, bounded nonempty UTF-8 65,536-byte acquisition and exact bytes frozen in all handoff intent forms; consumes: existing compound intent/body schema. Files: src/cli/handoff.rs parser and validation, src/cli/{handoff_delivery,topology_handoff}.rs saved plans and associated parser sources/tests. Preserve topic/goal limits separately; invalid sources fail before mutations; mutate/delete source after initial staging and prove retry keeps original content. Native argv contains fixed routing/inbox only.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--handoff-design.md\nAlso owns: body-bearing envelope capacity calculation and limits: bootstrap identity/progress512KiB, recovery1MiB; preserve metadata-only creation/attachment128KiB, completed2MiB/globalwire1MiB unless actual serializer worst-case fixtures prove a narrowly scoped bound adjustment necessary. Files additionally src/store/topology_handoff.rs and persistence.rs, src/cli/journal.rs, TRUST-POLICY encoded-limit clauses. Reuse src/cli/input.rs; protocol body validator must follow normal nonempty UTF-8 message bytes (NUL/whitespace are not identifier validation). Inventory all repeated bodies/nested originals; actual serializer64KiB six-byte escape public wire/canonical/local retained/retry fixtures prove support under ordinary bounded metadata. Exact serialized preflight before effects; retain historical bytes/digests and no clipping.",
      "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass.",
      "status": "open",
      "priority": 2,
      "issue_type": "task",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:48:19Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T22:59:16Z",
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "blocks"
    }
  ],
  "parent": "ht-0lo.3",
  "dependent_count": 1,
  "dependency_count": 6,
  "comment_count": 0,
  "revision": "7414178549115987505"
}
```

### Acceptance criteria (verbatim)

Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass. Observe durable overdue warning for an unregistered recipient (needs: ht-0lo.1.2). Require messages.send_time_receipts_v1 before new handoff effects; consume ht-0lo.1.1 capability contract and verify unsupported-daemon refusal before staging/launch.

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

#### Durable handoff interface and recovery

Support exactly one instruction source: the legacy quoted positional body after `--`, `--file PATH`, or `--stdin`. Read once before any durable effect, require nonempty valid UTF-8 within 65,536 bytes, preserve exact content/newlines, bound input while reading, and freeze actual bytes into the compound intent. Retrying never rereads a file/stdin or native environment options. Keep topic/goal validation separate. Reuse src/cli/input.rs normal message source validation; body validation is nonempty valid UTF-8, not bounded_text's unrelated topic/identifier trim or NUL restrictions. The source bytes and their JSON escaping are preserved exactly. All three modes (new-tab, explicit empty-pane launch, existing-recipient delivery) use the same body/deadline contract and exact keys.

Handoff always sends an ACK-required addressed instructions message, defaulting to the existing receipt-duration setting; add `--deadline SECONDS` to override that message deadline, frozen before staging. Its invitation is lazy unless the caller explicitly requests `--require-invite-ack`; add `--invite-deadline SECONDS` to imply that option. Message and invitation deadlines are independent. Stage messages while invitation is pending; sender already joined is required. Carry policy through all public request, preparation, canonical bootstrap, legacy handoff, existing delivery and retained retry representations. Omitted new fields in historical intents preserve historical child requests/digests. No restaging of completed child mutations or automatic launch after possible-start.

Raise only body-bearing retained envelope ceilings conservatively: canonical bootstrap identity and request-bearing local bootstrap progress from128KiB to512KiB; recovery from256KiB to1MiB. Keep metadata-only creation/attachment128KiB, completed2MiB and global wire1MiB unless the actual serializer's supported worst-case fixtures demonstrate a required narrowly scoped adjustment. Six JSON bytes per input byte means a65,536-byte body consumes393,216 encoded bytes per occurrence; inventory every duplicated body and nested serialized original rather than assuming one occurrence. The handoff source task owns actual encoded-bound calculation/tests across public wire, canonical identity/recovery/completion, local progress/linked/terminal/origin snapshots and replay. Test maximum ordinary text, maximum six-byte-escaped UTF-8 body and source mutations through each public handoff path; all supported body values under ordinary bounded metadata must stage and survive retry. Exact serialized capacity checks occur before durable effects; independent oversized metadata/native response limits still fail honestly without truncation or reclassification. Update TRUST-POLICY encoded-limit documentation in the implementation commit and preserve historical bytes/keys/digests. No unconditional global frame increase or unbounded reads.

Native startup contains only fixed inbox instructions, exact canonical routing metadata and pinned fallback. It contains no task body, body path or arbitrary task argv. Compare all three routing fields before preferring hook commands. Existing launched agents with changing harness versions still recover instructions from durable inbox; setup diagnostics/trust refusals do not discard them. Hook/setup/profile readiness inspection is diagnostic and does not block staging or an otherwise safe guarded launch: observed hook installation/trust cannot establish delivery, and a wrong inherited profile must not veto launching a user-selected native harness. Retain exact empty-shell/seat/caller/native argv safety guards; fingerprint-recheck any owned launch configuration actually used, but absence of owned hook configuration is not a veto and never gets a fabricated fingerprint. Unsupported environment overrides and native host-kind/grammar capabilities remain safety refusals; native harness approval/configuration refusals remain truthful refusals, never automatically answered. ACK or overdue warning confirms the bounded handoff outcome, independent of advisory setup status. Launch success is not registration, acceptance, receipt or task adoption. On missed confirmation the sender inspects and recovers; trust/approval remains user consent, never automatic.

#### Message deadline contract

For every new durable ACK-required ordinary send, freeze the duration using existing validation/default configuration and set its original deadline to canonical committed send time plus that duration. This applies to every recipient, including no binding, managed-launch/unregistered binding, unavailable binding and pending invitation. Availability never changes the original start/deadline, and cannot reset it on registration, accept, restart, or rebind. Lazy messages and human receipt waivers remain free of ACK obligations. Receipt still means receipt only.

The effective receipt projection, manifest preparation, sparse physical materialization, due cursors, soft-poke calculations and wake paths must agree. Keep availability separately meaningful: setting a deadline is not permission to wake, nor evidence of a live harness. Do not pretend send time is observed availability. Existing structural/native execution launch guards and wake harness/empty-composer guards remain mandatory.

Upgrade outstanding pending ACK-required messages too: preserve every previously running deadline, but give formerly dormant obligations the deadline derived from their immutable message send time and frozen duration. Already expired rows become due on the normal bounded scan. Preserve terminal states, receipt provenance and warning identities. A transactionally recorded schema/semantics marker makes old and new manifest/physical projections consistent; materialize/index due work so unavailable recipients are actually discoverable, not merely capable of returning a deadline on individual lookup. Do not rewrite immutable requests or old result digests.

Existing explicit summary catch-up extensions in TRUST-POLICY A6 remain separate and visible; this redesign removes availability-based deferral, not the policy's bounded progress-based extension. Tests distinguish original deadline from an explicitly extended effective deadline. Warning delivery to the originating seat is durable even if it later leaves the thread; joined observers may retain existing warning visibility. An overdue warning says confirmation missed its deadline, never why. The original sender's open warning is waking attention under the existing composer/host/mod admission guards, even if the sender is no longer joined; clear remains passive and observer visibility is unchanged. For non-seat service authors preserve existing service reporting rather than inventing a sender seat. Update TRUST-POLICY A7 to record this narrow new waking recipient rule. Native wake can be deferred by a draft, active turn or unavailable host; the durable warning remains pending, so bounded deadline detection does not promise bounded native key submission. Late ACK clears the existing warning through normal lifecycle without retroactively changing the deadline.

#### Invitation contract

Ordinary agent invitations default to Lazy: membership is pending and discoverable through existing inbox/hook opportunities, but the invitation itself requests no timed confirmation or native wake. Add invitation `--require-ack` and optional `--deadline SECONDS`; explicit deadline implies require-ack for ergonomic compatibility, and `--lazy` conflicts with either. An ACK-required invitation starts its acceptance deadline at canonical invitation creation. Only explicit recipient acceptance settles it automatically; no separate invitation ACK command and no registration settlement. Expiry notifies its originating seat with invitation deadline missed, regardless of failure to register or accept. The inviter's open warning is waking attention under existing safe composer/host/mod guards even after leaving; clear remains passive and existing debtor/observer visibility remains. Originator attribution is frozen from the original invitation episode, never inferred from a current member or a later duplicate caller. Rejection/cancellation/retirement retain existing truthful terminal outcomes and attributed rejection notices; lazy does not suppress explicit rejection notices.

Existing required service memberships and explicit human/operator invitation semantics remain unchanged. Existing ordinary pending invitations retain their current timed obligation; historical retry with omitted new fields retains its original interpretation. Use an additive optional delivery/confirmation field whose absence denotes legacy timed behavior; modern CLI explicitly emits Lazy for the new default. Persist requested timing and original inviter attribution independently of joined membership. Pending-invite duplicate calls return the existing invitation without silently upgrading/downgrading its deadline or waking it; changing obligation policy requires a new episode through existing disposition flow. Already joined is still AlreadyJoined, never a new obligation. Existing human mode cannot manufacture an agent ACK requirement.

Queries, inbox compact/machine/JSON, digest, mod/native wake selection and due scans all honor the requested mode. Lazy presentation is bookkeeping only. ACK-required invitation attention remains ordinary guarded delivery; accept remains an accountable recipient choice. Add schema fields/index predicate changes conservatively and negotiate protocol capability/version before any mutation requiring the new representation. Older saved intent bytes/digests are preserved and replayed unchanged; unsupported daemon refuses new semantics before writing.

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

1. Test actual new-tab, explicit empty-pane launch and existing-seat delivery with positional/file/stdin sources, frozen --deadline, lazy invite default and --require-invite-ack/--invite-deadline independence. Require messages.send_time_receipts_v1 and invitation representation capability before any new durable/native effects.
2. Carry frozen policy/body across public/preparation/canonical/legacy/existing/retained intent layers, preserving historical omitted fields and child digests. Exercise staged crash/retry, unknown-create and possible-start outcomes with no duplication/restaging/automatic relaunch.
3. Separate advisory hook/setup/trust/profile readiness diagnostics from launch admission. Retain caller/seat/empty-shell/native argv/host-kind/environment grammar guards and recheck actual owned launch configuration fingerprint only if used; absence is no veto and no fabricated fingerprint. Native trust refusal is truthful and never auto-approved.
4. Test missing/stale/wrong-profile hooks and unregistered recipient overdue sender warning with .1.2; verify exact three-field routing fallback and fixed startup argv, no auto acceptance/ACK/trust. Update help/skill/recovery docs and run focused handoff/launch/retry tests plus format/clippy.

## Task 11: Integration sweep: handoff delivery redesign

filesTouched: ["tests/cli/handoff.rs", "tests/cli/handoff_delivery.rs", "tests/cli/topology_handoff.rs", "tests/handoff_topology_cli.rs", "tests/integration/notice_wake.rs", "tests/integration/setup_canary_seam.rs", "tests/scheduler/deadlines.rs", "tests/cli/skill.rs", "integrations/skill/SKILL.md", "README.md", "TRUST-POLICY.md", "src/cli/handoff.rs", "src/cli/handoff_delivery.rs", "src/cli/topology_handoff.rs"]

bead: ht-0lo.5

Dependencies: ht-0lo.2.3, ht-0lo.2.2, ht-0lo.6, ht-0lo.1.3, ht-0lo.1.1, ht-0lo.7, ht-0lo.3.2, ht-0lo.2.1, ht-0lo.4.1, ht-0lo.4.2, ht-0lo.3.1, ht-0lo.1.2

### Full bead text (canonical snapshot)

```json
{
  "id": "ht-0lo.5",
  "title": "Integration sweep: handoff delivery redesign",
  "description": "owns: remaining cross-feature integration wiring, documentation consistency and end-to-end tests no owning leaf covers; consumes: all implementation leaves. Verify long durable inbox handoff, all native delivery paths, send-time unavailable-recipient expiry, lazy/timed invitation acceptance and independent sender warnings, retained keyed retry and stable per-event owned hooks. Fix small integration gaps inline; file blockers for larger discoveries. Run focused isolated tests, cargo fmt, nice cargo clippy --locked --all-targets --all-features -- -D warnings and nice scripts/check-default-features on the final candidate. Do not run the routine full suite while ht-zo4 restriction remains. Tests own/tag/stop all children/private sessions and never alter shared server or real user configs. Files: integration tests, embedded skills/help/docs, TRUST-POLICY.md and minimal seam wiring.\nblocked-by ht-0lo.1.1: consumes all leaves (integration sweep)\nblocked-by ht-0lo.1.2: consumes all leaves (integration sweep)\nblocked-by ht-0lo.1.3: consumes all leaves (integration sweep)\nblocked-by ht-0lo.2.1: consumes all leaves (integration sweep)\nblocked-by ht-0lo.2.2: consumes all leaves (integration sweep)\nblocked-by ht-0lo.2.3: consumes all leaves (integration sweep)\nblocked-by ht-0lo.3.1: consumes all leaves (integration sweep)\nblocked-by ht-0lo.3.2: consumes all leaves (integration sweep)\nblocked-by ht-0lo.4.1: consumes all leaves (integration sweep)\nblocked-by ht-0lo.4.2: consumes all leaves (integration sweep)\nblocked-by ht-0lo.6: consumes all leaves (integration sweep)\nblocked-by ht-0lo.7: consumes all leaves (integration sweep)",
  "acceptance_criteria": "Main handoff flows complete end to end with exact frozen bodies/routing and independent receipt/invitation deadlines. Overdue originators receive durable guarded waking attention; late confirmation clears passively. All owned native configurations and legacy retries remain safe. Final format, focused tests, all-feature clippy and default-feature gates pass; owned processes stopped. (needs: ht-0lo.1.1) (needs: ht-0lo.1.2) (needs: ht-0lo.1.3) (needs: ht-0lo.2.1) (needs: ht-0lo.2.2) (needs: ht-0lo.2.3) (needs: ht-0lo.3.1) (needs: ht-0lo.3.2) (needs: ht-0lo.4.1) (needs: ht-0lo.4.2)",
  "spec_id": "docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign-design.md",
  "status": "open",
  "priority": 2,
  "issue_type": "task",
  "owner": "235553+alepar@users.noreply.github.com",
  "created_at": "2026-10-10T23:09:38Z",
  "created_by": "Alexey Parfenov",
  "updated_at": "2026-10-10T23:18:42Z",
  "metadata": {
    "depth": 1,
    "order": 5
  },
  "labels": [
    "sp:ht-0lo"
  ],
  "dependencies": [
    {
      "id": "ht-0lo.2.3",
      "title": "Invitation CLI and presentation contract",
      "description": "owns: ordinary invite --lazy default, --require-ack/--deadline implication and conflicts, compact/JSON/inbox/help/embedded skill presentation; consumes: invitation confirmation protocol/store contract. Files: src/cli parser invitation branches/mod/output and help/skill resources, relevant CLI tests. Invoke actual CLI in isolated daemon/config for bare lazy, require-ack default duration, explicit deadline, conflicting flags, duplicate and historical keyed retry. Required/operator commands remain unchanged.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--invitations-design.md\nblocked-by ht-0lo.2.2: consumes invitation confirmation store behavior",
      "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass.",
      "status": "open",
      "priority": 2,
      "issue_type": "task",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:48:15Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T22:48:33Z",
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "blocks"
    },
    {
      "id": "ht-0lo.2.2",
      "title": "Invitation acceptance, due and delivery lifecycle",
      "description": "owns: canonical lazy/timed invitation create/duplicate/accept/reject/cancel/retire behavior, due scanner and inviter-addressed warning routing plus read/wake/mod projections; consumes: invitation mode schema/wire contract. Files: src/store/{control,invitation_due,wake,queries,service_controls,seats}.rs as needed, daemon projections, TRUST-POLICY invitation clauses and focused tests. Run lazy invite no native wake/deadline, timed never-register and register-without-accept expiry, acceptance clears, duplicate never mutates policy, inviter leaving warning, required membership and explicit rejection notice cases.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--invitations-design.md\nblocked-by ht-0lo.2.1: consumes invitation mode schema/wire contract\nOriginal inviter attribution frozen at invitation creation; overdue inviter open warning waking under existing native/mod guards even after leaving, clear passive; update TRUST-POLICY A7. Duplicate caller never retargets originator warning.\nHealthy-daemon discovery contract: retain the5,000ms safety tick and100candidate per-drive cap; continue immediately while successful due progress reports more. Each class advances a stable finite high-water cursor fairly; newly arriving obligations do not restart/starve the current snapshot. Native wake admission/completion never gates cursor progress. For a snapshot with at most N candidates per class and successful runnable driver quanta taking at most Q ms including storage/scheduling, durable pending-warning detection finishes within5,000ms +(2*ceil(N/100)+2)*Q; this is conditional on the daemon running and successful bounded storage, not a guarantee across outages, repeated errors, unbounded overload or native key submission. Restart resumes/reconstructs fair discovery and scans already overdue obligations. Early fake-clock tests use at least250candidates in EACH invitation and receipt class, blocked native wakes, mixed future/due rows, continuing arrivals and restart; verify every initial overdue row is warned within the formula and arrivals cannot starve either class. Historical upgrade also uses multiple bounded batches without losing overdue indexed discovery.",
      "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass. Healthy-daemon discovery contract: retain the5,000ms safety tick and100candidate per-drive cap; continue immediately while successful due progress reports more. Each class advances a stable finite high-water cursor fairly; newly arriving obligations do not restart/starve the current snapshot. Native wake admission/completion never gates cursor progress. For a snapshot with at most N candidates per class and successful runnable driver quanta taking at most Q ms including storage/scheduling, durable pending-warning detection finishes within5,000ms +(2*ceil(N/100)+2)*Q; this is conditional on the daemon running and successful bounded storage, not a guarantee across outages, repeated errors, unbounded overload or native key submission. Restart resumes/reconstructs fair discovery and scans already overdue obligations. Early fake-clock tests use at least250candidates in EACH invitation and receipt class, blocked native wakes, mixed future/due rows, continuing arrivals and restart; verify every initial overdue row is warned within the formula and arrivals cannot starve either class. Historical upgrade also uses multiple bounded batches without losing overdue indexed discovery.",
      "status": "open",
      "priority": 2,
      "issue_type": "task",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:48:14Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T23:18:51Z",
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "blocks"
    },
    {
      "id": "ht-0lo.6",
      "title": "Define fair due-scan progress and conditional detection bound",
      "description": "Design-only fix for confirmed roast R1-1. owns: normative finite-backlog scanner progress/latency contract and early multi-batch validation; consumes: existing5s safety tick,100candidate batch and frozen cursor semantics. Update root/receipt/invitation specs and task acceptances; implementation remains owned by ht-0lo.1.2,ht-0lo.1.3 and ht-0lo.2.2.",
      "acceptance_criteria": "Normative specs and implementation owners explicitly resolve the confirmed design finding; independent next roast verifies the changed contract.",
      "status": "closed",
      "priority": 2,
      "issue_type": "task",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T23:18:25Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T23:18:55Z",
      "closed_at": "2026-10-10T23:18:55Z",
      "close_reason": "Design contract and implementation task acceptance updated; independent round2 will verify.",
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "blocks"
    },
    {
      "id": "ht-0lo.1.3",
      "title": "Upgrade dormant historical receipt obligations",
      "description": "owns: transactional upgrade/backfill of formerly dormant historical ACK obligations, sparse/physical due projection consistency and idempotent schema marker; consumes: canonical send-time receipt projection. Preserve already running deadlines, terminal states/provenance and immutable request/result digests. Files: src/store/{schema,effective,materialization}.rs and focused migration/store tests. Exercise legacy unavailable receipts upgrade/restart/replay and overdue indexed discovery, bounded warning burst.\nblocked-by ht-0lo.1.1: consumes canonical send-time receipt projection\nExercise dormant historical upgrade over multiple100candidate batches and verify no overdue initial row is lost or starved by newly arriving work; discovery contract is owned by ht-0lo.1.2.",
      "acceptance_criteria": "Legacy dormant physical/manifest ACK obligations receive send-time deadlines while existing started/terminal/waived obligations remain unchanged. Idempotent restart and focused migration tests pass. Exercise dormant historical upgrade over multiple100candidate batches and verify no overdue initial row is lost or starved by newly arriving work; discovery contract is owned by ht-0lo.1.2.",
      "status": "open",
      "priority": 2,
      "issue_type": "task",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:49:48Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T23:18:48Z",
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "blocks"
    },
    {
      "id": "ht-0lo.1.1",
      "title": "Canonical send-time receipt projection",
      "description": "owns: receipt original deadline semantics, preserving running deadlines and terminal provenance; consumes: existing immutable send timestamps/durations. Files: src/store/{effective,materialization,schema,messages}.rs and preparation modules, TRUST-POLICY deadline clauses, focused store tests. Cover sparse and physical receipts, restart, lazy/human waiver, registration not resetting and explicit A6 extension distinction.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--receipts-design.md\nReceipt semantic capability: advertise messages.send_time_receipts_v1 only when canonical send-time projection and its ordinary send handlers are active. Modern clients require this capability before every new ACK-required ordinary send or handoff durable/native effect; absent/refused discovery produces explicit unsupported-daemon error, with no silent legacy fallback. All CLI entry paths and service-capable client sends that promise this contract are covered. Old clients talking to an upgraded daemon receive canonical new-send semantics without modifying their old wire requests. Lazy/human-waived sends remain exempt; completed retained keyed historical operations remain inspectable/replayable with original bytes/digests/outcomes and are never rewritten. A pending historical retry on an unsupported daemon refuses before additional effects. Test newCLI/olddaemon refusal, oldCLI/newdaemon canonical send, current/current unbound sends and lazy/human exemptions using capability fixtures.",
      "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass. Receipt semantic capability: advertise messages.send_time_receipts_v1 only when canonical send-time projection and its ordinary send handlers are active. Modern clients require this capability before every new ACK-required ordinary send or handoff durable/native effect; absent/refused discovery produces explicit unsupported-daemon error, with no silent legacy fallback. All CLI entry paths and service-capable client sends that promise this contract are covered. Old clients talking to an upgraded daemon receive canonical new-send semantics without modifying their old wire requests. Lazy/human-waived sends remain exempt; completed retained keyed historical operations remain inspectable/replayable with original bytes/digests/outcomes and are never rewritten. A pending historical retry on an unsupported daemon refuses before additional effects. Test newCLI/olddaemon refusal, oldCLI/newdaemon canonical send, current/current unbound sends and lazy/human exemptions using capability fixtures.",
      "status": "open",
      "priority": 2,
      "issue_type": "task",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:48:10Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T23:18:44Z",
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "blocks"
    },
    {
      "id": "ht-0lo.7",
      "title": "Define ordinary-send receipt semantic compatibility boundary",
      "description": "Design-only fix for confirmed roast R1-4. owns: named send-time receipt capability and pre-effect modern client refusal boundary; consumes: existing capability discovery mechanism. Update root/receipt/handoff specs and ht-0lo.1.1 acceptance. No historical request mutation or new adversarial verification.",
      "acceptance_criteria": "Normative specs and implementation owners explicitly resolve the confirmed design finding; independent next roast verifies the changed contract.",
      "status": "closed",
      "priority": 2,
      "issue_type": "task",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T23:18:35Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T23:18:59Z",
      "closed_at": "2026-10-10T23:18:59Z",
      "close_reason": "Design contract and implementation task acceptance updated; independent round2 will verify.",
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "blocks"
    },
    {
      "id": "ht-0lo.3.2",
      "title": "Handoff confirmation policy wiring and recovery documentation",
      "description": "owns: handoff --deadline/--require-invite-ack/--invite-deadline parsing/freeze/pass-through, modern lazy invitation plus addressed ACK-required body staging across new-tab/empty-pane/existing paths, retained child digest compatibility and source/help/skill docs; consumes: frozen handoff payload, invitation confirmation wire contract and send-time receipt semantics. Files: src/cli/{handoff,handoff_delivery,topology_handoff,mod}.rs, src/protocol/handoff.rs, src/store/{handoff,topology_handoff}.rs and related tests/help/skill. Run isolated actual handoff per all three paths/body sources; crash/retry after stage and possible-start/unknown-create never duplicates/restages/relaunches. Older plans absent new fields replay original keyed requests, modern deadlines freeze before staging; routing exact3field fallback and no auto trust/accept/ACK.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--handoff-design.md\nblocked-by ht-0lo.3.1: consumes frozen handoff payload\nblocked-by ht-0lo.2.1: consumes invitation confirmation wire contract\nblocked-by ht-0lo.1.1: consumes send-time receipt semantics\nAlso owns: separating hook/setup/trust/profile readiness diagnostics from launch admission; src/cli/launch.rs prepare_launch and src/harness/launch.rs configuration checks. Missing/untrusted/wrong-profile hooks do not block staged work or otherwise structurally safe launch. Preserve empty-shell/seat/caller/native argv and owned launch-configuration fingerprint safety, and truthful native trust refusal; never auto-approve. Test missing/stale profile hooks through guarded handoff and observe durable ACK deadline warning without registration.\nblocked-by ht-0lo.2.2: consumes canonical invitation lifecycle\nblocked-by ht-0lo.1.2: consumes overdue receipt warning lifecycle\nHook readiness is advisory: keep owned launch configuration fingerprint recheck only when that configuration is actually used; absence cannot veto and no fingerprint is fabricated. Unsupported environment-overrides/native grammar/host-kind safety refusals remain.\nRequire messages.send_time_receipts_v1 before new handoff effects; consume ht-0lo.1.1 capability contract and verify unsupported-daemon refusal before staging/launch.",
      "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass. Observe durable overdue warning for an unregistered recipient (needs: ht-0lo.1.2). Require messages.send_time_receipts_v1 before new handoff effects; consume ht-0lo.1.1 capability contract and verify unsupported-daemon refusal before staging/launch.",
      "status": "open",
      "priority": 2,
      "issue_type": "task",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:48:20Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T23:18:53Z",
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "blocks"
    },
    {
      "id": "ht-0lo.2.1",
      "title": "Invitation confirmation protocol and schema contract",
      "description": "owns: additive invitation delivery/confirmation wire fields, capability refusal and schema fields/indexes/legacy defaults; consumes: existing operation digests and membership episodes. Files: src/protocol commands/results/capabilities or existing capability module, src/store/schema.rs and schema tests. Absent historical fields mean legacy timed behavior; explicit modern Lazy requests retain timing/inviter origin without altering old bytes. Freeze duration at creation, preserve required and human invitation semantics.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--invitations-design.md",
      "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass.",
      "status": "open",
      "priority": 2,
      "issue_type": "task",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:48:13Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T22:48:13Z",
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "blocks"
    },
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
      "dependency_type": "blocks"
    },
    {
      "id": "ht-0lo.4.2",
      "title": "Owned stable registration migration and tooling",
      "description": "owns: manifest-recorded exact ownership/install identity, repeat setup in place, recoverable UUID/shared-command migration, safe unsetup and status/trust/canary/launcher command compatibility; consumes: stable native per-event command grammar. Files: src/harness/setup.rs and claude/codex setup modules, src/cli setup/launch consumers, canary/tooling/docs/tests. Invoke isolated Claude/Codex setup-repeat/migration/interruption/unsetup with foreign identical/drifted groups and ensure no ownership theft, no trust writes, no group shifting on equivalent reinstall. Hermes/Claude mod regressions stay passing.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--hooks-design.md\nblocked-by ht-0lo.4.1: consumes stable native per-event command grammar",
      "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass.",
      "status": "open",
      "priority": 2,
      "issue_type": "task",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:48:18Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T22:48:31Z",
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "blocks"
    },
    {
      "id": "ht-0lo.3.1",
      "title": "Handoff instruction source acquisition and frozen payload",
      "description": "owns: positional/file/stdin source exclusivity, bounded nonempty UTF-8 65,536-byte acquisition and exact bytes frozen in all handoff intent forms; consumes: existing compound intent/body schema. Files: src/cli/handoff.rs parser and validation, src/cli/{handoff_delivery,topology_handoff}.rs saved plans and associated parser sources/tests. Preserve topic/goal limits separately; invalid sources fail before mutations; mutate/delete source after initial staging and prove retry keeps original content. Native argv contains fixed routing/inbox only.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--handoff-design.md\nAlso owns: body-bearing envelope capacity calculation and limits: bootstrap identity/progress512KiB, recovery1MiB; preserve metadata-only creation/attachment128KiB, completed2MiB/globalwire1MiB unless actual serializer worst-case fixtures prove a narrowly scoped bound adjustment necessary. Files additionally src/store/topology_handoff.rs and persistence.rs, src/cli/journal.rs, TRUST-POLICY encoded-limit clauses. Reuse src/cli/input.rs; protocol body validator must follow normal nonempty UTF-8 message bytes (NUL/whitespace are not identifier validation). Inventory all repeated bodies/nested originals; actual serializer64KiB six-byte escape public wire/canonical/local retained/retry fixtures prove support under ordinary bounded metadata. Exact serialized preflight before effects; retain historical bytes/digests and no clipping.",
      "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass.",
      "status": "open",
      "priority": 2,
      "issue_type": "task",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:48:19Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T22:59:16Z",
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "blocks"
    },
    {
      "id": "ht-0lo.1.2",
      "title": "Due discovery and originating-seat warning lifecycle",
      "description": "owns: bounded indexed due discovery plus sender-addressed open/clear warning routing and guarded poke/wake calculations; consumes: send-anchored canonical receipt projection. Files: src/store/{receipts,poke,wake,queries,schema}.rs, scheduler tests. Exercise unbound/unregistered/unavailable/pending-invite recipient sends through expiry and late ACK; originator leaving does not lose notification.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--receipts-design.md\nblocked-by ht-0lo.1.1: consumes send-anchored canonical receipt projection\nblocked-by ht-0lo.1.3: consumes historical dormant receipt upgrade\nOriginator open warning is waking attention under existing safe composer/host/mod guards even after leaving; clear passive; update TRUST-POLICY A7. No invented seat for service authors. Test notification pending while drafts/working/host outage defer actual native wake.\nHealthy-daemon discovery contract: retain the5,000ms safety tick and100candidate per-drive cap; continue immediately while successful due progress reports more. Each class advances a stable finite high-water cursor fairly; newly arriving obligations do not restart/starve the current snapshot. Native wake admission/completion never gates cursor progress. For a snapshot with at most N candidates per class and successful runnable driver quanta taking at most Q ms including storage/scheduling, durable pending-warning detection finishes within5,000ms +(2*ceil(N/100)+2)*Q; this is conditional on the daemon running and successful bounded storage, not a guarantee across outages, repeated errors, unbounded overload or native key submission. Restart resumes/reconstructs fair discovery and scans already overdue obligations. Early fake-clock tests use at least250candidates in EACH invitation and receipt class, blocked native wakes, mixed future/due rows, continuing arrivals and restart; verify every initial overdue row is warned within the formula and arrivals cannot starve either class. Historical upgrade also uses multiple bounded batches without losing overdue indexed discovery.",
      "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass. Healthy-daemon discovery contract: retain the5,000ms safety tick and100candidate per-drive cap; continue immediately while successful due progress reports more. Each class advances a stable finite high-water cursor fairly; newly arriving obligations do not restart/starve the current snapshot. Native wake admission/completion never gates cursor progress. For a snapshot with at most N candidates per class and successful runnable driver quanta taking at most Q ms including storage/scheduling, durable pending-warning detection finishes within5,000ms +(2*ceil(N/100)+2)*Q; this is conditional on the daemon running and successful bounded storage, not a guarantee across outages, repeated errors, unbounded overload or native key submission. Restart resumes/reconstructs fair discovery and scans already overdue obligations. Early fake-clock tests use at least250candidates in EACH invitation and receipt class, blocked native wakes, mixed future/due rows, continuing arrivals and restart; verify every initial overdue row is warned within the formula and arrivals cannot starve either class. Historical upgrade also uses multiple bounded batches without losing overdue indexed discovery.",
      "status": "open",
      "priority": 2,
      "issue_type": "task",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:48:11Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T23:18:46Z",
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "blocks"
    },
    {
      "id": "ht-0lo",
      "title": "Handoff delivery redesign",
      "description": "Deliver thread-carried complete handoff instructions with send-time receipt deadlines, lazy optional-confirmation invitations and stable native hook entrypoints. Spec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign-design.md. User authorized autonomous implementation; integration merge remains coordinator decision.",
      "spec_id": "docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign-design.md",
      "status": "open",
      "priority": 2,
      "issue_type": "epic",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:45:27Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T22:57:33Z",
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "parent-child"
    }
  ],
  "parent": "ht-0lo",
  "dependent_count": 0,
  "dependency_count": 13,
  "comment_count": 0,
  "revision": "-6971199537433044500"
}
```

### Acceptance criteria (verbatim)

Main handoff flows complete end to end with exact frozen bodies/routing and independent receipt/invitation deadlines. Overdue originators receive durable guarded waking attention; late confirmation clears passively. All owned native configurations and legacy retries remain safe. Final format, focused tests, all-feature clippy and default-feature gates pass; owned processes stopped. (needs: ht-0lo.1.1) (needs: ht-0lo.1.2) (needs: ht-0lo.1.3) (needs: ht-0lo.2.1) (needs: ht-0lo.2.2) (needs: ht-0lo.2.3) (needs: ht-0lo.3.1) (needs: ht-0lo.3.2) (needs: ht-0lo.4.1) (needs: ht-0lo.4.2)

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

#### Acceptance and validation

- Long instructions survive restart/retry unchanged; native argv excludes the body. Invalid, oversized, conflicting or unreadable sources fail before durable writes.
- New and previously dormant ACK obligations expire without recipient activity; already running deadlines do not move. Late registration/acceptance never restarts the clock. Due scans are bounded/indexed, sender receives warnings, late receipt clears normally.
- Lazy ordinary invitation produces no overdue/native wake; explicitly timed invitation warns the originator and acceptance settles automatically. No acceptance/ACK is synthesized by launch or registration.
- Equivalent reinstall emits identical per-event commands while foreign and drifted configuration survives; explicit legacy migration is recoverable and honest about native trust needing review.
- Existing bootstrap unknown-create/possible-start guarantees, human waivers, receipt provenance, required memberships, routing and hook trust boundaries continue passing their focused regression tests.
- Run cargo fmt, nice cargo clippy --locked --all-targets --all-features -- -D warnings and focused tests per change; before merge readiness also nice scripts/check-default-features. Use test-support-owned children/private Herdr sessions, stop all owned processes, and perform scoped leak checks after any full suite.

#### Non-goals and follow-on

No auto-trust, recipient auto-accept, task-adoption ACK, delivery exactly-once claim, automatic 1:1 thread selection, server restart, seat repair heuristics, real user config writes, deployment or merge into main. The parent run notifies its established main-tab discussion when the integration branch is ready for review/merge.

#### Confirmed roast refinements

Healthy-daemon discovery contract: retain the5,000ms safety tick and100candidate per-drive cap; continue immediately while successful due progress reports more. Each class advances a stable finite high-water cursor fairly; newly arriving obligations do not restart/starve the current snapshot. Native wake admission/completion never gates cursor progress. For a snapshot with at most N candidates per class and successful runnable driver quanta taking at most Q ms including storage/scheduling, durable pending-warning detection finishes within5,000ms +(2*ceil(N/100)+2)*Q; this is conditional on the daemon running and successful bounded storage, not a guarantee across outages, repeated errors, unbounded overload or native key submission. Restart resumes/reconstructs fair discovery and scans already overdue obligations. Early fake-clock tests use at least250candidates in EACH invitation and receipt class, blocked native wakes, mixed future/due rows, continuing arrivals and restart; verify every initial overdue row is warned within the formula and arrivals cannot starve either class. Historical upgrade also uses multiple bounded batches without losing overdue indexed discovery.

Receipt semantic capability: advertise messages.send_time_receipts_v1 only when canonical send-time projection and its ordinary send handlers are active. Modern clients require this capability before every new ACK-required ordinary send or handoff durable/native effect; absent/refused discovery produces explicit unsupported-daemon error, with no silent legacy fallback. All CLI entry paths and service-capable client sends that promise this contract are covered. Old clients talking to an upgraded daemon receive canonical new-send semantics without modifying their old wire requests. Lazy/human-waived sends remain exempt; completed retained keyed historical operations remain inspectable/replayable with original bytes/digests/outcomes and are never rewritten. A pending historical retry on an unsupported daemon refuses before additional effects. Test newCLI/olddaemon refusal, oldCLI/newdaemon canonical send, current/current unbound sends and lazy/human exemptions using capability fixtures.

### Coordinator execution contract

Coordinator ruling: ht-0lo.1.2 owns shared due driver/fair scheduling and combined fairness tests; ht-0lo.2.2 owns invitation-class cursor/store API/isolated tests. Preserve full requirements plus explicit cross-task seam and final sweep coverage. Final default-feature check is parent-owned after PR roast per explicit sequencing.

Read TRUST-POLICY.md before any protected source changes. Only implement declared ownership; mechanical compile-preserving adaptations are permitted. Report any additional write path before expanding filesTouched. Do not write root run-state/report/ledger from task worktrees. Test additions reuse existing module targets unless process-wide state requires a dedicated test target.

### TDD implementation and independently testable deliverable

1. Review final combined implementation against root and four subtree specs; identify uncovered interface cases and add focused isolated end-to-end tests for long durable inbox body, all delivery paths, receipt and invitation independent deadlines, originating-seat wake after leave, exact keyed retry and stable owned event configuration.
2. Run relevant focused tests on integration tip, cargo fmt and all-feature clippy, fix small remaining seam/docs inconsistencies inline; file larger blocker discoveries rather than concealing them. Preserve explicit process ownership and stop all private sessions/helpers. Export a fresh HT_LEAK_RUN_ID and run scripts/check-no-leaked-processes --run-id "$HT_LEAK_RUN_ID" after the integration sweep.
3. Parent coordinator owns readiness after PR roast: default-feature gate and any final expensive branch checks run once against the roast-cleared exact tip, with SHA evidence. This leaf supplies focused sweep evidence and does not falsely report the deferred gate as passed. No routine full suite under ht-zo4 restriction.
4. Return final implementation-seam test evidence and remaining blockers to parent. Parent owns report, roasts, final readiness, notify-main milestone and merge decision.

Terminal sweep also verifies expanded H and combined bootstrap completion from the Task7 bounded projection bootstrap refinement; no due-only or changed-row count may establish the global bound.

Task10 production seam admission: real canonical public handoff plus actual bounded materialization creates overdue warnings but lacks original-sender receipt_warning_origins. Native publication intentionally retains author_kind NULL; migration0002 explicitly defines NULL plus immutable actor seat as native. Exact migrations/0035_receipt_warning_origins.sql and src/store/receipts.rs admitted for all six maintained triggers and bounded historical projection to accept established native interpretation, with author_service_id absent, same-instance canonical actor seat, and exact published recipient association retained. tests/store/receipts.rs admitted for real ordinary-send positive, genuine historical NULL restart/backfill and service/built-in/unknown negative controls, alongside real public handoff RED. Do not rewrite immutable author history or synthesize fixture native kinds. TRUST wording clarifies existing provenance rather than inventing authority. Fresh Task10 review and explicit production/schema seam review must inspect original Task7 omission and narrow correction.

Exact tests/harness/adapter_smoke.rs admitted for mechanical Option adaptation in genuine configured-hook fixture; preserve original complete fingerprint assertions and run adapter smoke selection. Audit all explicit target consumers before rebuild. Mapped29363 compile-only uncovered missing fixture adaptation, not behavioral failure.


## Appended upstream composition ordinal mapping

| Ordinal | Bead | Task |
| --- | --- | --- |
| 12 | ht-0lo.8.1 | Released schema preservation and isolated upstream composition |
| 13 | ht-0lo.8.2 | Automatic Ensure creation policy and durable handoff composition |
| 14 | ht-0lo.8.3 | Combined upstream composition focused integration proof |
| 15 | ht-0lo.8.4 | AGY delivery setup and launch semantic composition |

## Task 12: Released schema preservation and isolated upstream composition

filesTouched: ["CHANGELOG.md", "Cargo.lock", "Cargo.toml", "README.md", "TRUST-POLICY.md", "docs/agent-usage.md", "docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/2026-10-10-automatic-one-to-one-threads--handoff-design.md", "docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/2026-10-10-automatic-one-to-one-threads--pair-boundary-design.md", "docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/2026-10-10-automatic-one-to-one-threads-design.md", "docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/2026-10-10-automatic-one-to-one-threads-roast-design-1-coverage.json", "docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/2026-10-10-automatic-one-to-one-threads-roast-design-1-raw.json", "docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/2026-10-10-automatic-one-to-one-threads-roast-design-1.md", "docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/2026-10-10-automatic-one-to-one-threads-roast-design-2-coverage.json", "docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/2026-10-10-automatic-one-to-one-threads-roast-design-2-raw.json", "docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/2026-10-10-automatic-one-to-one-threads-roast-design-2.md", "docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/2026-10-10-automatic-one-to-one-threads-roast-pr-1.md", "docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/2026-10-10-automatic-one-to-one-threads-roast-pr-2.md", "docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/automatic-one-to-one-threads-coverage-ledger.md", "docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/coverage-requirements.md", "docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/coverage-round-1-checks-suffix.md", "docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/coverage-round-1-handoff-packet.md", "docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/coverage-round-1-pair-packet.md", "docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/coverage-round-1-report.md", "docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/coverage-round-1-root-packet.md", "docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/coverage-round-1-tree.json", "docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/coverage-round-2-handoff-checks-chunk.md", "docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/coverage-round-2-handoff-packet.md", "docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/coverage-round-2-handoff-review-state-chunk.md", "docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/coverage-round-2-handoff-specs-chunk.md", "docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/coverage-round-2-pair-packet.md", "docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/coverage-round-2-report.md", "docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/coverage-round-2-root-packet.md", "docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/coverage-round-2-tree-supplement.md", "docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/coverage-round-2-tree.json", "docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/design-fix-1-id.txt", "docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/friction.md", "docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/integration-evidence.md", "docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/promotion-review.md", "docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/report.md", "docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/run.md", "docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/upstream-feedback-draft.md", "docs/superpowers/specs/INDEX.md", "herdr-plugin.toml", "integrations/skill/SKILL.md", "migrations/0029_direct_threads.sql", "migrations/0030_automatic_handoff.sql", "src/cli/commands.rs", "src/cli/direct_send.rs", "src/cli/handoff.rs", "src/cli/handoff_delivery.rs", "src/cli/journal.rs", "src/cli/mod.rs", "src/cli/retry.rs", "src/cli/topology_handoff.rs", "src/cli/topology_handoff/automatic.rs", "src/cli/topology_recover.rs", "src/cli/topology_recover/automatic.rs", "src/cli/topology_runtime.rs", "src/daemon/control.rs", "src/ports.rs", "src/protocol/authority.rs", "src/protocol/capabilities.rs", "src/protocol/commands.rs", "src/protocol/handoff.rs", "src/protocol/handoff/automatic.rs", "src/protocol/handoff/automatic_bootstrap.rs", "src/protocol/results.rs", "src/protocol/wire.rs", "src/service/dispatch.rs", "src/service/kicks.rs", "src/store/archival.rs", "src/store/control.rs", "src/store/handoff.rs", "src/store/handoff/automatic.rs", "src/store/messages.rs", "src/store/mod.rs", "src/store/schema.rs", "src/store/seats.rs", "src/store/service_controls.rs", "src/store/topology_handoff.rs", "src/store/topology_handoff/attachment.rs", "src/store/topology_handoff/automatic.rs", "src/store/topology_handoff/runtime.rs", "src/test_support/history.rs", "tests/cli/commands.rs", "tests/cli/direct_send.rs", "tests/cli/follow_retry.rs", "tests/cli/handoff.rs", "tests/cli/handoff_delivery.rs", "tests/cli/journal.rs", "tests/cli/readme_commands.rs", "tests/cli/topology_handoff.rs", "tests/combined.rs", "tests/contracts.rs", "tests/handoff_topology_authority.rs", "tests/integration/channel_activation.rs", "tests/integration/lazy_attention.rs", "tests/integration/notice_wake.rs", "tests/integration/summary_flow.rs", "tests/integration/sweep_remaining.rs", "tests/integration/topology_activation.rs", "tests/lifecycle_ux.rs", "tests/protocol/capabilities.rs", "tests/service/cooperative.rs", "tests/service/crash_matrix.rs", "tests/service/graph_d2.rs", "tests/service/graph_send.rs", "tests/service/operator.rs", "tests/store/archival_legacy.rs", "tests/store/author_role.rs", "tests/store/commit_kicks.rs", "tests/store/control.rs", "tests/store/cooperative_checkin.rs", "tests/store/direct_threads.rs", "tests/store/facade.rs", "tests/store/handoff_fences.rs", "tests/store/lazy26_prerequisite.rs", "tests/store/lazy_inbox.rs", "tests/store/lazy_publication.rs", "tests/store/lazy_schema.rs", "tests/store/messages.rs", "tests/store/receipts.rs", "tests/store/retention.rs", "tests/store/schema.rs", "tests/store/topology_attachment_guard.rs", "tests/store/topology_handoff.rs", "tests/store/topology_handoff_schema.rs", "src/store/receipts.rs", "src/store/attention.rs", "src/store/materialization.rs", "src/store/catch_up.rs", "src/store/topology_handoff/persistence.rs", "src/archival_legacy.rs", "tests/store/materialization.rs", "tests/store/attention.rs", "tests/store/wake_discovery_cost.rs", "migrations/0029_invitation_confirmation.sql", "migrations/0030_receipt_send_time.sql", "migrations/0031_handoff_body_bounds.sql", "migrations/0032_receipt_deadline_backfill.sql", "migrations/0033_invitation_lifecycle.sql", "migrations/0034_invitation_warning_origin.sql", "migrations/0035_receipt_warning_origins.sql", "tests/store/effective.rs", "tests/store/invitation_due.rs", "migrations/0038_receipt_warning_origins.sql", "migrations/0034_handoff_body_bounds.sql", "migrations/0036_invitation_lifecycle.sql", "migrations/0032_invitation_confirmation.sql", "migrations/0035_receipt_deadline_backfill.sql", "migrations/0037_invitation_warning_origin.sql", "migrations/0033_receipt_send_time.sql", ".github/workflows/ci.yml", "AGENTS.md", "docs/compatibility/harnesses.md", "docs/dev/outside-adapter-report.md", "docs/evidence/README.md", "docs/evidence/agy-live-smoke/2026-10-10-4d2ec663dbfa/daemon-excerpt.json", "docs/evidence/agy-live-smoke/2026-10-10-4d2ec663dbfa/pane-final.txt", "docs/evidence/agy-live-smoke/2026-10-10-4d2ec663dbfa/relay.jsonl", "docs/evidence/agy-live-smoke/2026-10-10-4d2ec663dbfa/report.md", "docs/evidence/agy-live-smoke/2026-10-10-4d2ec663dbfa/requests-excerpt.json", "docs/evidence/agy-live-smoke/2026-10-10-4d2ec663dbfa/summary.json", "docs/evidence/agy-live-smoke/2026-10-11-144d9d6640eb/daemon-excerpt.json", "docs/evidence/agy-live-smoke/2026-10-11-144d9d6640eb/pane-final.txt", "docs/evidence/agy-live-smoke/2026-10-11-144d9d6640eb/relay.jsonl", "docs/evidence/agy-live-smoke/2026-10-11-144d9d6640eb/report.md", "docs/evidence/agy-live-smoke/2026-10-11-144d9d6640eb/requests-excerpt.json", "docs/evidence/agy-live-smoke/2026-10-11-144d9d6640eb/summary.json", "docs/evidence/agy-live-smoke/2026-10-11-2bf61304750f/daemon-excerpt.json", "docs/evidence/agy-live-smoke/2026-10-11-2bf61304750f/pane-final.txt", "docs/evidence/agy-live-smoke/2026-10-11-2bf61304750f/relay.jsonl", "docs/evidence/agy-live-smoke/2026-10-11-2bf61304750f/report.md", "docs/evidence/agy-live-smoke/2026-10-11-2bf61304750f/requests-excerpt.json", "docs/evidence/agy-live-smoke/2026-10-11-2bf61304750f/summary.json", "docs/evidence/agy-live-smoke/README.md", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/README.md", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/allow-rewrite/allow-rewrite-finding.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/allow-rewrite/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/allow-rewrite/requests.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/allow-rewrite/trust-prompt-screen.txt", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/boot/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/boot/pane-final.txt", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/boot/requests.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/clear/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/clear/requests.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/delivery/error-pane.txt", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/delivery/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/delivery/receipts.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/delivery/requests.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/delivery/send-CANCEL_NOTE_.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/delivery/send-ERROR_NOTE_o.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/delivery/send-MAIN_A1_ack_.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/delivery/send-MAIN_L1_lazy.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/delivery/send-MAIN_O1_ordi.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/delivery/send-MAIN_T1_TTTT.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/delivery/send-STOP_NOTE_or.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/delivery/state-files.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/delivery/thread-create-gate-delivery.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/downgrade/abort-fired.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/downgrade/abort-pane.txt", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/downgrade/doctor-delivery-abort.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/downgrade/doctor-delivery-restaged.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/downgrade/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/downgrade/requests.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/downgrade/send-DOWN_M1_abor.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/downgrade/send-DOWN_M2_user.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/downgrade/send-DOWN_M3_syst.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/downgrade/send-DOWN_M4_igno.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/downgrade/send-DOWN_M5_igno.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/downgrade/setup-status-mutated.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/downgrade/setup-status-restored.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/downgrade/state-files.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/downgrade/thread-create-gate-downgrade.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/enroll/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/enroll/requests.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/handoff/handoff-response.txt", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/handoff/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/handoff/requests.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/launch/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/launch/requests.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/permission/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/permission/permission-blocked-screen.txt", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/permission/requests.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/resume/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/resume/requests.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/setup/requests.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/subagent/invoke-subagent-declaration.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/subagent/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/subagent/requests.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/unsetup/requests.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/wake/a-wake-user-requests.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/wake/c-follow-up-body.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/wake/c-receipts.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/wake/c-transcript-n2.jsonl", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/wake/d-detection.txt", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/wake/e-wake-on.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/wake/f-after.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/wake/f-detection.txt", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/wake/g-detection.txt", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/wake/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/wake/requests.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/wake/send-WAKE_N1_ack_.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/wake/send-WAKE_N2_mid_.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/wake/send-WAKE_N3_whil.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/wake/send-WAKE_N4_duri.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/wake/send-WAKE_N5_duri.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/wake/send-WAKE_Z1_lazy.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/wake/send-WAKE_Z2_lazy.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/artifacts-36840d17-ce84-43c2-84bb-a036c94c0e89/wake/thread-create-gate-wake.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/report.json", "docs/evidence/agy-native-gate/2026-10-10-9af7ae9f3985/summary.txt", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/README.md", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/allow-rewrite/allow-rewrite-finding.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/allow-rewrite/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/allow-rewrite/requests.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/allow-rewrite/trust-prompt-screen.txt", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/boot/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/boot/pane-final.txt", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/boot/requests.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/clear/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/clear/requests.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/delivery/error-pane.txt", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/delivery/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/delivery/receipts.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/delivery/requests.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/delivery/send-CANCEL_NOTE_.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/delivery/send-ERROR_NOTE_o.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/delivery/send-MAIN_A1_ack_.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/delivery/send-MAIN_L1_lazy.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/delivery/send-MAIN_O1_ordi.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/delivery/send-MAIN_T1_TTTT.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/delivery/send-STOP_NOTE_or.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/delivery/state-files.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/delivery/thread-create-gate-delivery.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/downgrade/abort-fired.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/downgrade/abort-pane.txt", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/downgrade/doctor-delivery-abort.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/downgrade/doctor-delivery-restaged.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/downgrade/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/downgrade/requests.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/downgrade/send-DOWN_M1_abor.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/downgrade/send-DOWN_M2_user.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/downgrade/send-DOWN_M3_syst.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/downgrade/send-DOWN_M4_igno.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/downgrade/send-DOWN_M5_igno.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/downgrade/setup-status-mutated.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/downgrade/setup-status-restored.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/downgrade/state-files.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/downgrade/thread-create-gate-downgrade.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/enroll/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/enroll/requests.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/handoff/handoff-response.txt", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/handoff/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/handoff/requests.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/launch/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/launch/requests.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/permission/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/permission/permission-blocked-screen.txt", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/permission/requests.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/resume/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/resume/requests.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/setup/requests.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/subagent/invoke-subagent-declaration.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/subagent/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/subagent/requests.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/unsetup/requests.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/wake/a-wake-user-requests.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/wake/c-follow-up-body.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/wake/c-receipts.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/wake/c-transcript-n2.jsonl", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/wake/d-detection.txt", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/wake/e-wake-on.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/wake/f-after.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/wake/f-detection.txt", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/wake/g-detection.txt", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/wake/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/wake/requests.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/wake/send-WAKE_N1_ack_.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/wake/send-WAKE_N2_mid_.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/wake/send-WAKE_N3_whil.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/wake/send-WAKE_N4_duri.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/wake/send-WAKE_N5_duri.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/wake/send-WAKE_Z1_lazy.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/wake/send-WAKE_Z2_lazy.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/artifacts-1a90217a-4c41-4aa9-8471-9776f861eae9/wake/thread-create-gate-wake.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/report.json", "docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/summary.txt", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/allow-rewrite/allow-rewrite-finding.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/allow-rewrite/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/allow-rewrite/requests.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/allow-rewrite/trust-prompt-screen.txt", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/boot/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/boot/pane-final.txt", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/boot/requests.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/clear/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/clear/requests.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/delivery/error-pane.txt", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/delivery/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/delivery/receipts.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/delivery/requests.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/delivery/send-CANCEL_NOTE_.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/delivery/send-ERROR_NOTE_o.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/delivery/send-MAIN_A1_ack_.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/delivery/send-MAIN_L1_lazy.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/delivery/send-MAIN_O1_ordi.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/delivery/send-MAIN_T1_TTTT.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/delivery/send-STOP_NOTE_or.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/delivery/state-files.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/delivery/thread-create-gate-delivery.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/downgrade/abort-fired.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/downgrade/abort-pane.txt", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/downgrade/doctor-delivery-abort.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/downgrade/doctor-delivery-restaged.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/downgrade/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/downgrade/requests.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/downgrade/send-DOWN_M1_abor.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/downgrade/send-DOWN_M2_user.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/downgrade/send-DOWN_M3_syst.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/downgrade/send-DOWN_M4_igno.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/downgrade/send-DOWN_M5_igno.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/downgrade/setup-status-mutated.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/downgrade/setup-status-restored.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/downgrade/state-files.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/downgrade/thread-create-gate-downgrade.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/enroll/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/enroll/requests.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/handoff/handoff-response.txt", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/handoff/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/handoff/requests.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/launch/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/launch/requests.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/permission/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/permission/permission-blocked-screen.txt", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/permission/requests.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/resume/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/resume/requests.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/setup/requests.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/subagent/invoke-subagent-declaration.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/subagent/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/subagent/requests.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/unsetup/requests.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/wake/a-wake-user-requests.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/wake/c-follow-up-body.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/wake/c-receipts.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/wake/c-transcript-n2.jsonl", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/wake/d-detection.txt", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/wake/e-wake-on.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/wake/f-after.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/wake/f-detection.txt", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/wake/g-detection.txt", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/wake/onboarding-pages.txt", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/wake/requests.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/wake/send-WAKE_N1_ack_.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/wake/send-WAKE_N2_mid_.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/wake/send-WAKE_N3_whil.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/wake/send-WAKE_N4_duri.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/wake/send-WAKE_N5_duri.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/wake/send-WAKE_Z1_lazy.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/wake/send-WAKE_Z2_lazy.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/artifacts-130d3e0b-f6a6-4494-bb60-faeb6871fe0f/wake/thread-create-gate-wake.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/report.json", "docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/summary.txt", "docs/install.md", "docs/operations.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/2026-10-09-agy-harness-adapter--agy-adapter-design.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/2026-10-09-agy-harness-adapter--core-sweep-design.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/2026-10-09-agy-harness-adapter--delivery-policy-design.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/2026-10-09-agy-harness-adapter--test-infra-design.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/2026-10-09-agy-harness-adapter-design.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/2026-10-10-agy-harness-adapter-roast-design-1-step-back.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/2026-10-10-agy-harness-adapter-roast-design-1.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/2026-10-10-agy-harness-adapter-roast-design-2.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/2026-10-10-agy-harness-adapter-roast-pr-1-punch-list.txt", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/2026-10-10-agy-harness-adapter-roast-pr-1-scope-filter.json", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/2026-10-10-agy-harness-adapter-roast-pr-1-scope-keys.txt", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/2026-10-10-agy-harness-adapter-roast-pr-1-step-back-keys.txt", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/2026-10-10-agy-harness-adapter-roast-pr-1-step-back.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/2026-10-10-agy-harness-adapter-roast-pr-1.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/2026-10-10-agy-harness-adapter-roast-pr-2-punch-list.txt", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/2026-10-10-agy-harness-adapter-roast-pr-2.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/2026-10-10-agy-harness-adapter-scoped-audit-417c534f-9af7ae9f.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/code-final-review-10.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/code-final-review-11.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/code-final-review-12.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/code-final-review-13.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/code-final-review-6.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/code-final-review-7.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/code-final-review-8.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/code-final-review-9.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/coverage-findings-round-1.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/coverage-findings-round-2.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/coverage-inputs-round-1.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/coverage-inputs-round-2.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/coverage-ledger.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/coverage-precheck-round-1.txt", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/coverage-precheck-round-2.txt", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/coverage-round-1-requirements.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/coverage-round-1-review-1.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/coverage-round-1-review-2.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/coverage-round-2-requirements.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/coverage-round-2-review-1.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/coverage-round-2-review-2.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/delivery-contract-audit.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/design-decisions.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/friction.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/handoff-continuation.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/integration-sweep.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/outside-adapter-report.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/report.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/roast-design-1-tree.json", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/run.md", "docs/superpowers/runs/2026-10-09-agy-harness-adapter/upstream-feedback-draft.md", "docs/validation/agy-native.md", "integrations/agy/README.md", "integrations/claude/README.md", "integrations/claude/mod/README.md", "integrations/claude/mod/hooks/register.js", "integrations/claude/mod/tests/delivery.test.ts", "integrations/claude/mod/tests/fixtures/frame-goldens.ts", "integrations/claude/mod/tests/frame_golden.test.ts", "integrations/claude/mod/tests/stress.test.ts", "integrations/claude/mod/types/index.d.ts", "migrations/0031_seat_wake_on.sql", "scripts/agy-fake-gemini/scenario.py", "scripts/agy-fake-gemini/server.py", "scripts/agy-fake-gemini/test_fake_gemini.py", "scripts/agy-fake-gemini/testdata/agy-request-systemmessage.json", "scripts/agy-gate/flows/_l4.py", "scripts/agy-gate/flows/_l5.py", "scripts/agy-gate/flows/_l6.py", "scripts/agy-gate/flows/_l7.py", "scripts/agy-gate/flows/allow_rewrite.py", "scripts/agy-gate/flows/boot.py", "scripts/agy-gate/flows/clear.py", "scripts/agy-gate/flows/delivery.py", "scripts/agy-gate/flows/downgrade.py", "scripts/agy-gate/flows/enroll.py", "scripts/agy-gate/flows/handoff.py", "scripts/agy-gate/flows/launch.py", "scripts/agy-gate/flows/permission.py", "scripts/agy-gate/flows/resume.py", "scripts/agy-gate/flows/setup.py", "scripts/agy-gate/flows/subagent.py", "scripts/agy-gate/flows/unsetup.py", "scripts/agy-gate/flows/wake.py", "scripts/agy-gate/gate.py", "scripts/agy-gate/live_smoke.py", "scripts/agy-gate/observe.py", "scripts/agy-gate/relay_backend.py", "scripts/agy-gate/scenarios/allow-rewrite.json", "scripts/agy-gate/scenarios/boot.json", "scripts/agy-gate/scenarios/clear.json", "scripts/agy-gate/scenarios/delivery-cancel.json", "scripts/agy-gate/scenarios/delivery-error.json", "scripts/agy-gate/scenarios/delivery-stop.json", "scripts/agy-gate/scenarios/delivery.json", "scripts/agy-gate/scenarios/downgrade-ignore.json", "scripts/agy-gate/scenarios/downgrade.json", "scripts/agy-gate/scenarios/enroll.json", "scripts/agy-gate/scenarios/handoff.json", "scripts/agy-gate/scenarios/launch.json", "scripts/agy-gate/scenarios/permission.json", "scripts/agy-gate/scenarios/resume.json", "scripts/agy-gate/scenarios/setup.json", "scripts/agy-gate/scenarios/subagent.json", "scripts/agy-gate/scenarios/unsetup.json", "scripts/agy-gate/scenarios/wake-subagent.json", "scripts/agy-gate/scenarios/wake.json", "scripts/agy-gate/step_mutator.py", "scripts/agy-gate/test_gate.py", "scripts/agy-gate/test_live_smoke.py", "scripts/agy-gate/test_scenarios_l4.py", "scripts/agy-gate/test_scenarios_l5.py", "scripts/agy-gate/test_scenarios_l6.py", "scripts/agy-gate/test_scenarios_l7.py", "scripts/agy-gate/test_step_mutator.py", "scripts/agy-live-smoke", "scripts/agy-native-gate", "scripts/agy-relay/budget.py", "scripts/agy-relay/fake_upstream.py", "scripts/agy-relay/relay.py", "scripts/agy-relay/test_relay.py", "scripts/agy-relay/test_translate.py", "scripts/agy-relay/testdata/agy-native-requests.json", "scripts/agy-relay/testdata/agy-tool-declarations.json", "scripts/agy-relay/testdata/openrouter-stream-text.sse", "scripts/agy-relay/testdata/openrouter-stream-toolcall.sse", "scripts/agy-relay/translate.py", "scripts/agy-sandbox/agy-versions.json", "scripts/agy-sandbox/agyenv.py", "scripts/agy-sandbox/fetch-agy", "scripts/agy-sandbox/loopback.sb.in", "scripts/agy-sandbox/test_agyenv.py", "scripts/agy-sandbox/testdata/agy-9.9.9.tar.gz", "scripts/agy-sandbox/testdata/manifest-9.9.9.json", "scripts/check-harness-neutral", "scripts/gen-frame-goldens.mjs", "scripts/harness-neutral-allowlist.txt", "scripts/outside-adapter-report", "scripts/tests/test_check_harness_neutral.py", "scripts/tests/test_outside_adapter_report.py", "src/app.rs", "src/cli/doctor.rs", "src/cli/help.rs", "src/cli/hook.rs", "src/cli/hook_evidence.rs", "src/cli/launch.rs", "src/cli/me.rs", "src/cli/output.rs", "src/cli/setup.rs", "src/cli/setup/foreground.rs", "src/cli/skill.rs", "src/cli/summary.rs", "src/cli/wake_on.rs", "src/cli/watch.rs", "src/client/mod.rs", "src/daemon/harness_evidence.rs", "src/daemon/harness_states.rs", "src/daemon/health.rs", "src/delivery/assemble.rs", "src/delivery/frame.rs", "src/delivery/ledger.rs", "src/delivery/mod.rs", "src/delivery/page.rs", "src/delivery/routing.rs", "src/delivery/settle.rs", "src/harness/adapter.rs", "src/harness/agy/composer.rs", "src/harness/agy/contract.rs", "src/harness/agy/decode.rs", "src/harness/agy/delivery.rs", "src/harness/agy/doctor.rs", "src/harness/agy/launch.rs", "src/harness/agy/mod.rs", "src/harness/agy/paths.rs", "src/harness/agy/plugin.rs", "src/harness/agy/setup.rs", "src/harness/agy/witness.rs", "src/harness/attribution.rs", "src/harness/bridge.rs", "src/harness/cache.rs", "src/harness/claude.rs", "src/harness/claude/composer.rs", "src/harness/claude/delivery.rs", "src/harness/claude/doctor.rs", "src/harness/claude/permission_rules.rs", "src/harness/claude/permissions.rs", "src/harness/claude/prompt_suggestion.rs", "src/harness/claude/setup.rs", "src/harness/codex.rs", "src/harness/codex/composer.rs", "src/harness/codex/doctor.rs", "src/harness/codex/permission_rules.rs", "src/harness/codex/permissions.rs", "src/harness/codex/setup.rs", "src/harness/codex_evidence.rs", "src/harness/codex_schema.rs", "src/harness/composer.rs", "src/harness/contract.rs", "src/harness/evidence.rs", "src/harness/hermes.rs", "src/harness/hermes/launch.rs", "src/harness/launch.rs", "src/harness/manifest.rs", "src/harness/mod.rs", "src/harness/operational.rs", "src/harness/permissions.rs", "src/harness/process.rs", "src/harness/recipe.rs", "src/harness/registry.rs", "src/harness/runtime.rs", "src/harness/setup.rs", "src/host/native.rs", "src/lib.rs", "src/notification/dispatch.rs", "src/notification/policy.rs", "src/protocol/attention.rs", "src/protocol/delivery.rs", "src/protocol/mod.rs", "src/protocol/output.rs", "src/protocol/output_compact.rs", "src/protocol/watch.rs", "src/scheduler/mod.rs", "src/service/delivery_pull.rs", "src/service/mod.rs", "src/service/mod_channels.rs", "src/service/workers.rs", "src/store/lazy_delivery.rs", "src/store/queries.rs", "src/store/topology_handoff/attempts.rs", "src/store/wake.rs", "src/test_support/archival_composer_fixture.rs", "src/test_support/attention_oracle.rs", "src/test_support/mod.rs", "tests/cli/cooperative.rs", "tests/cli/doctor_json.rs", "tests/cli/doctor_labels.rs", "tests/cli/hook.rs", "tests/cli/hook_evidence.rs", "tests/cli/hook_inbox.rs", "tests/cli/hook_local_evidence.rs", "tests/cli/hook_neutral.rs", "tests/cli/hook_pull_lazy.rs", "tests/cli/installer.rs", "tests/cli/launch.rs", "tests/cli/lazy_display.rs", "tests/cli/lazy_settlement.rs", "tests/cli/output.rs", "tests/cli/skill.rs", "tests/cli/summary.rs", "tests/cli/wake_on.rs", "tests/cli/watch.rs", "tests/daemon/control.rs", "tests/daemon/harness_evidence.rs", "tests/daemon/health_budget.rs", "tests/daemon/health_harness_state.rs", "tests/daemon/health_optimistic.rs", "tests/delivery/extraction.rs", "tests/delivery/frame.rs", "tests/delivery/ledger.rs", "tests/delivery/promotion.rs", "tests/delivery/push_channel.rs", "tests/delivery/routing.rs", "tests/delivery/settle.rs", "tests/fixtures/agy/composer/README.md", "tests/fixtures/agy/composer/agent-list.txt", "tests/fixtures/agy/composer/busy.read-detection.txt", "tests/fixtures/agy/composer/capture.sh", "tests/fixtures/agy/composer/draft.read-detection.txt", "tests/fixtures/agy/composer/empty.read-detection.txt", "tests/fixtures/agy/composer/harness-fake_server.py", "tests/fixtures/agy/composer/help-overlay.read-detection.txt", "tests/fixtures/agy/composer/multiline-after-ctrl-u-1.read-detection.txt", "tests/fixtures/agy/composer/multiline-after-ctrl-u-2.read-detection.txt", "tests/fixtures/agy/composer/multiline.read-detection.txt", "tests/fixtures/agy/composer/old-agent-row.read-detection.txt", "tests/fixtures/agy/composer/permission-prompt.read-detection.txt", "tests/fixtures/agy/composer/subagent-generating.read-detection.txt", "tests/fixtures/agy/composer/subagent-tool-running.read-detection.txt", "tests/fixtures/agy/delivery/README.md", "tests/fixtures/agy/delivery/output-system-message.json", "tests/fixtures/agy/delivery/output-text.txt", "tests/fixtures/agy/delivery/output-user-message.json", "tests/fixtures/agy/delivery/stop-cancelled.json", "tests/fixtures/agy/delivery/stop-error-failed-to-inject.json", "tests/fixtures/agy/delivery/stop-error-unknown-step.json", "tests/fixtures/agy/delivery/stop-error-unrelated.json", "tests/fixtures/agy/delivery/stop-model-stop.json", "tests/fixtures/agy/delivery/stop-no-tool-call.json", "tests/fixtures/agy/delivery/tool-output-line.jsonl", "tests/fixtures/agy/delivery/transcript-forged-body.jsonl", "tests/fixtures/agy/delivery/transcript-forged-escaped-newline.jsonl", "tests/fixtures/agy/delivery/transcript-frame.jsonl", "tests/fixtures/agy/delivery/transcript-genuine-post-tool-call.jsonl", "tests/fixtures/agy/delivery/transcript-genuine-system-message.jsonl", "tests/fixtures/agy/delivery/transcript-genuine-user-message.jsonl", "tests/fixtures/agy/delivery/transcript-header-in-model-output.jsonl", "tests/fixtures/agy/delivery/transcript-header-in-tool-output.jsonl", "tests/fixtures/agy/delivery/transcript-header-in-user-input.jsonl", "tests/fixtures/agy/delivery/transcript-malformed-lines.jsonl", "tests/fixtures/agy/delivery/transcript-no-frame.jsonl", "tests/fixtures/agy/delivery/transcript-truncated.jsonl", "tests/fixtures/agy/help/agy-1.3.2.txt", "tests/fixtures/agy/layouts/README.md", "tests/fixtures/agy/layouts/child-under-bound.txt", "tests/fixtures/agy/layouts/child-under-other.txt", "tests/fixtures/agy/layouts/clear.txt", "tests/fixtures/agy/layouts/missing-lock.txt", "tests/fixtures/agy/layouts/over-cap-brain.txt", "tests/fixtures/agy/layouts/recent-listed.txt", "tests/fixtures/agy/layouts/recent-without-lock.txt", "tests/fixtures/agy/layouts/resume-stale-lock.txt", "tests/fixtures/agy/layouts/top-level.txt", "tests/fixtures/agy/payloads/README.md", "tests/fixtures/agy/payloads/pre-invocation-inv0-fresh.json", "tests/fixtures/agy/payloads/pre-invocation-inv0-resume.json", "tests/fixtures/agy/payloads/pre-invocation-inv1.json", "tests/fixtures/agy/payloads/session-start-after-clear.json", "tests/fixtures/agy/payloads/session-start-child.json", "tests/fixtures/agy/payloads/session-start-top.json", "tests/fixtures/agy/payloads/stop-inject-error.json", "tests/fixtures/agy/payloads/stop-no-tool-call.json", "tests/fixtures/agy/plugin/SKILL.md", "tests/fixtures/agy/plugin/hooks.json", "tests/fixtures/agy/plugin/plugin.json", "tests/fixtures/delivery/frame-attention.json", "tests/fixtures/delivery/frame-attention.txt", "tests/fixtures/delivery/frame-controls.json", "tests/fixtures/delivery/frame-controls.txt", "tests/fixtures/delivery/frame-empty.json", "tests/fixtures/delivery/frame-empty.txt", "tests/fixtures/delivery/frame-header.txt", "tests/fixtures/delivery/frame-markers.json", "tests/fixtures/delivery/frame-markers.txt", "tests/fixtures/delivery/frame-multiline.json", "tests/fixtures/delivery/frame-multiline.txt", "tests/fixtures/delivery/frame-name-equals-id.json", "tests/fixtures/delivery/frame-name-equals-id.txt", "tests/fixtures/golden/contract-id/json.json", "tests/fixtures/golden/contract-id/json.json.status", "tests/fixtures/golden/contract-id/text.txt", "tests/fixtures/golden/contract-id/text.txt.status", "tests/fixtures/golden/doctor/all.debug.text", "tests/fixtures/golden/doctor/all.debug.text.status", "tests/fixtures/golden/doctor/all.json", "tests/fixtures/golden/doctor/all.json.status", "tests/fixtures/golden/doctor/all.text", "tests/fixtures/golden/doctor/all.text.status", "tests/fixtures/golden/doctor/hermes.json", "tests/fixtures/golden/doctor/hermes.json.status", "tests/fixtures/golden/doctor/hermes.text", "tests/fixtures/golden/doctor/hermes.text.status", "tests/fixtures/golden/env-markers/detection.json", "tests/fixtures/golden/handoff/claude.new-tab.bootstrap-argv.json", "tests/fixtures/golden/handoff/claude.new-tab.json", "tests/fixtures/golden/handoff/claude.new-tab.json.status", "tests/fixtures/golden/handoff/claude.new-tab.text", "tests/fixtures/golden/handoff/claude.new-tab.text.status", "tests/fixtures/golden/handoff/claude.pane.bootstrap-argv.json", "tests/fixtures/golden/handoff/claude.pane.json", "tests/fixtures/golden/handoff/claude.pane.json.status", "tests/fixtures/golden/handoff/claude.pane.text", "tests/fixtures/golden/handoff/claude.pane.text.status", "tests/fixtures/golden/handoff/codex.new-tab.bootstrap-argv.json", "tests/fixtures/golden/handoff/codex.new-tab.json", "tests/fixtures/golden/handoff/codex.new-tab.json.status", "tests/fixtures/golden/handoff/codex.new-tab.text", "tests/fixtures/golden/handoff/codex.new-tab.text.status", "tests/fixtures/golden/handoff/codex.pane.bootstrap-argv.json", "tests/fixtures/golden/handoff/codex.pane.json", "tests/fixtures/golden/handoff/codex.pane.json.status", "tests/fixtures/golden/handoff/codex.pane.text", "tests/fixtures/golden/handoff/codex.pane.text.status", "tests/fixtures/golden/handoff/hermes.json", "tests/fixtures/golden/handoff/hermes.json.status", "tests/fixtures/golden/handoff/hermes.text", "tests/fixtures/golden/handoff/hermes.text.status", "tests/fixtures/golden/health/observed.json", "tests/fixtures/golden/health/unobserved.json", "tests/fixtures/golden/help/handoff.txt", "tests/fixtures/golden/help/handoff.txt.status", "tests/fixtures/golden/help/launch.txt", "tests/fixtures/golden/help/launch.txt.status", "tests/fixtures/golden/help/me-init.txt", "tests/fixtures/golden/help/me-init.txt.status", "tests/fixtures/golden/help/setup.txt", "tests/fixtures/golden/help/setup.txt.status", "tests/fixtures/golden/hook-healthy/claude.SessionStart.stdout", "tests/fixtures/golden/hook-healthy/claude.SessionStart.stdout.status", "tests/fixtures/golden/hook-healthy/codex.SessionStart.stdout", "tests/fixtures/golden/hook-healthy/codex.SessionStart.stdout.status", "tests/fixtures/golden/hook/claude.PostToolUse.stdout", "tests/fixtures/golden/hook/claude.PostToolUse.stdout.status", "tests/fixtures/golden/hook/claude.PreToolUse.stdout", "tests/fixtures/golden/hook/claude.PreToolUse.stdout.status", "tests/fixtures/golden/hook/claude.SessionStart.stdout", "tests/fixtures/golden/hook/claude.SessionStart.stdout.status", "tests/fixtures/golden/hook/claude.Stop.stdout", "tests/fixtures/golden/hook/claude.Stop.stdout.status", "tests/fixtures/golden/hook/claude.UserPromptSubmit.stdout", "tests/fixtures/golden/hook/claude.UserPromptSubmit.stdout.status", "tests/fixtures/golden/hook/codex.PostToolUse.stdout", "tests/fixtures/golden/hook/codex.PostToolUse.stdout.status", "tests/fixtures/golden/hook/codex.PreToolUse.stdout", "tests/fixtures/golden/hook/codex.PreToolUse.stdout.status", "tests/fixtures/golden/hook/codex.SessionStart.stdout", "tests/fixtures/golden/hook/codex.SessionStart.stdout.status", "tests/fixtures/golden/hook/codex.Stop.stdout", "tests/fixtures/golden/hook/codex.Stop.stdout.status", "tests/fixtures/golden/hook/codex.UserPromptSubmit.stdout", "tests/fixtures/golden/hook/codex.UserPromptSubmit.stdout.status", "tests/fixtures/golden/hook/hermes.PostToolUse.stdout", "tests/fixtures/golden/hook/hermes.PostToolUse.stdout.status", "tests/fixtures/golden/hook/hermes.PreToolUse.stdout", "tests/fixtures/golden/hook/hermes.PreToolUse.stdout.status", "tests/fixtures/golden/hook/hermes.SessionStart.stdout", "tests/fixtures/golden/hook/hermes.SessionStart.stdout.status", "tests/fixtures/golden/hook/hermes.Stop.stdout", "tests/fixtures/golden/hook/hermes.Stop.stdout.status", "tests/fixtures/golden/hook/hermes.UserPromptSubmit.stdout", "tests/fixtures/golden/hook/hermes.UserPromptSubmit.stdout.status", "tests/fixtures/golden/launch/claude.json", "tests/fixtures/golden/launch/claude.json.status", "tests/fixtures/golden/launch/claude.start-request.json", "tests/fixtures/golden/launch/claude.text", "tests/fixtures/golden/launch/claude.text.status", "tests/fixtures/golden/launch/codex.json", "tests/fixtures/golden/launch/codex.json.status", "tests/fixtures/golden/launch/codex.start-request.json", "tests/fixtures/golden/launch/codex.text", "tests/fixtures/golden/launch/codex.text.status", "tests/fixtures/golden/launch/hermes.json", "tests/fixtures/golden/launch/hermes.json.status", "tests/fixtures/golden/launch/hermes.text", "tests/fixtures/golden/launch/hermes.text.status", "tests/fixtures/golden/setup-status/all.json", "tests/fixtures/golden/setup-status/all.json.status", "tests/fixtures/golden/setup-status/all.text", "tests/fixtures/golden/setup-status/all.text.status", "tests/fixtures/golden/setup-status/claude.absent.json", "tests/fixtures/golden/setup-status/claude.absent.json.status", "tests/fixtures/golden/setup-status/claude.absent.text", "tests/fixtures/golden/setup-status/claude.absent.text.status", "tests/fixtures/golden/setup-status/claude.json", "tests/fixtures/golden/setup-status/claude.json.status", "tests/fixtures/golden/setup-status/claude.text", "tests/fixtures/golden/setup-status/claude.text.status", "tests/fixtures/golden/setup-status/codex.absent.json", "tests/fixtures/golden/setup-status/codex.absent.json.status", "tests/fixtures/golden/setup-status/codex.absent.text", "tests/fixtures/golden/setup-status/codex.absent.text.status", "tests/fixtures/golden/setup-status/codex.json", "tests/fixtures/golden/setup-status/codex.json.status", "tests/fixtures/golden/setup-status/codex.text", "tests/fixtures/golden/setup-status/codex.text.status", "tests/fixtures/golden/setup-status/hermes.absent.json", "tests/fixtures/golden/setup-status/hermes.absent.json.status", "tests/fixtures/golden/setup-status/hermes.absent.text", "tests/fixtures/golden/setup-status/hermes.absent.text.status", "tests/fixtures/golden/setup-status/hermes.json", "tests/fixtures/golden/setup-status/hermes.json.status", "tests/fixtures/golden/setup-status/hermes.text", "tests/fixtures/golden/setup-status/hermes.text.status", "tests/fixtures/golden/setup/all.json", "tests/fixtures/golden/setup/all.json.status", "tests/fixtures/golden/setup/all.text", "tests/fixtures/golden/setup/all.text.status", "tests/fixtures/golden/setup/claude.json", "tests/fixtures/golden/setup/claude.json.status", "tests/fixtures/golden/setup/claude.text", "tests/fixtures/golden/setup/claude.text.status", "tests/fixtures/golden/setup/codex.json", "tests/fixtures/golden/setup/codex.json.status", "tests/fixtures/golden/setup/codex.text", "tests/fixtures/golden/setup/codex.text.status", "tests/fixtures/golden/setup/hermes.json", "tests/fixtures/golden/setup/hermes.json.status", "tests/fixtures/golden/setup/hermes.text", "tests/fixtures/golden/setup/hermes.text.status", "tests/fixtures/golden/unsetup/claude.absent.json", "tests/fixtures/golden/unsetup/claude.absent.json.status", "tests/fixtures/golden/unsetup/claude.absent.text", "tests/fixtures/golden/unsetup/claude.absent.text.status", "tests/fixtures/golden/unsetup/claude.json", "tests/fixtures/golden/unsetup/claude.json.status", "tests/fixtures/golden/unsetup/claude.text", "tests/fixtures/golden/unsetup/claude.text.status", "tests/fixtures/golden/unsetup/codex.absent.json", "tests/fixtures/golden/unsetup/codex.absent.json.status", "tests/fixtures/golden/unsetup/codex.absent.text", "tests/fixtures/golden/unsetup/codex.absent.text.status", "tests/fixtures/golden/unsetup/codex.json", "tests/fixtures/golden/unsetup/codex.json.status", "tests/fixtures/golden/unsetup/codex.text", "tests/fixtures/golden/unsetup/codex.text.status", "tests/fixtures/golden/unsetup/hermes.json", "tests/fixtures/golden/unsetup/hermes.json.status", "tests/fixtures/golden/unsetup/hermes.text", "tests/fixtures/golden/unsetup/hermes.text.status", "tests/golden_surfaces.rs", "tests/handoff_agy.rs", "tests/handoff_topology_cli.rs", "tests/harness/adapter_seam.rs", "tests/harness/adapter_smoke.rs", "tests/harness/admission_ladder.rs", "tests/harness/agy_composer.rs", "tests/harness/agy_decode.rs", "tests/harness/agy_delivery.rs", "tests/harness/agy_doctor.rs", "tests/harness/agy_launch.rs", "tests/harness/agy_setup.rs", "tests/harness/agy_wake.rs", "tests/harness/agy_witness.rs", "tests/harness/attribution.rs", "tests/harness/canary_payloads.rs", "tests/harness/claude.rs", "tests/harness/claude_mod.rs", "tests/harness/claude_permissions.rs", "tests/harness/codex.rs", "tests/harness/codex_permissions.rs", "tests/harness/codex_schema.rs", "tests/harness/contract.rs", "tests/harness/cooperative.rs", "tests/harness/launch.rs", "tests/harness/recipe.rs", "tests/harness/role_witness.rs", "tests/harness/setup.rs", "tests/harness/versions_json_guard.rs", "tests/hook_entrypoint.rs", "tests/hook_entrypoint/agy.rs", "tests/host/wake_submission.rs", "tests/integration/agy_sweep.rs", "tests/integration/harness_adapter_wiring.rs", "tests/integration/mod_delivery.rs", "tests/integration/sweep.rs", "tests/integration/trust_policy.rs", "tests/protocol/delivery.rs", "tests/protocol/watch.rs", "tests/scheduler/dispatch.rs", "tests/scheduler/hook_pull_routing.rs", "tests/scheduler/poke_flow.rs", "tests/scheduler/policy.rs", "tests/scheduler/wake_text.rs", "tests/service/delivery_pull.rs", "tests/service/lanes.rs", "tests/service/lazy_hook_pull.rs", "tests/service/loop_inventory.rs", "tests/service/mod_channels.rs", "tests/service/resolution.rs", "tests/setup_cli.rs", "tests/store/mod_seat_view.rs", "tests/store/queries.rs", "tests/store/wake.rs", "tests/store/wake_on.rs", "tests/store/wake_on_wake.rs", "migrations/0039_automatic_handoff_body_bounds.sql"]

bead: ht-0lo.8.1

Dependencies: isolated candidate starts from reviewed current integration plus main478ec4a9; coordinator sequences Task11 integration

### Proposed full bead body (coordinator persistence required)

```json
{
  "id": "ht-0lo.8.1",
  "title": "Released schema preservation and isolated upstream composition",
  "description": "Own isolated mainab2291c5 merge import and exact mechanical composition, released29/30/31 preservation, our29..35\u219232..38 renumber, additive39 automatic body-bearing capacity migration and schema verification/catalog fixtures. All upstream imported paths are declared explicitly; import does not authorize unrelated semantic edits. Resolve mechanical conflicts without erasing either feature; report semantic seams to Task13. Preserve immutable released SQL, original requests/digests/keys/provenance and full old FK graph. Keep candidate isolated; no provisional root merge. Supply actual compilable branch/schema/interface/capacity map to Task13.",
  "acceptance_criteria": "Genuine populated published main31\u2192combined latest physically reopens/upgrades, rolls back failed migration atomically, restores FK enforcement, preserves every old row/JSON/key/digest and exact old definitions except explicitly additive/rebuilt capacity objects, and matches fresh catalog. Released29/30/31 git blobs exact; genuine older fixture cutpoints remain genuine. Additive39 enlarges only actual automatic body-bearing checks, with actual65536-byte ordinary/six-byte-escaped original/recovery serializers and independent metadata limits. Run focused schema/upgrade/capacity tests, fmt and per-change all-target/all-feature clippy; independent schema/mechanical review CLEAN. Mechanical compile checks do not claim Task13 automatic behavior. No placeholders or unresolved markers into root integration. New Ensure creation-policy capability direct_threads.ensure_creation_policy_v1 is independently required for modern policy, including Lazy directsend beforeeffects; advertise only actual canonical activation. Oldv0.7 automatic/direct peers includingLazy prove zeroeffects; Task13 owns semanticcapability, Task12 activation-ready schema ifneeded without speculativeDDL. Actual latest AGENTS constraints: scripts/check-harness-neutral and truthful Outside-adapter trailer on every NEW commit; no real ~/.gemini writes. Preserve AGY delivery/wake_on/schema addition and provide stable neutraladapter/launch/delivery interfaces to Task15; semantic readiness/pull earlygates deferred explicitly, not falselyclaimed mechanicalGREEN. Parent-settled aggregate envelope: escaped65536-byte body supported under ordinary bounded metadata, not simultaneous independent metadata maxima. Full-max combinations refuse truthfully before durable/native effects; general requests remain1MiB. Exact supported original must fit every later Complete/Recover request, reply, local/recovery serializer including near-aggregate boundary and future bounded fields, or initial preflight refuses. Never accept an origin that later becomes unrecoverable. Preserve exact retained bytes/digests; positive maxbody ordinarymetadata and fullmax negative controls required. Exceptional AutomaticBootstrap response only, final numeric limit derived from full supported-envelope fixtures.",
  "dependencies": [],
  "parent": "ht-0lo.8",
  "status": "proposed; coordinator creates bead"
}
```

### Acceptance and settled producer contract

Genuine populated published main30→combined latest physically reopens/upgrades, rolls back failed migration atomically, restores FK enforcement, preserves every old row/JSON/key/digest and exact old definitions except explicitly additive/rebuilt capacity objects, and matches fresh catalog. Released29/30 git blobs exact; genuine older fixture cutpoints remain genuine. Additive38 enlarges only actual automatic body-bearing checks, with actual65536-byte ordinary/six-byte-escaped original/recovery serializers and independent metadata limits. Run focused schema/upgrade/capacity tests, fmt and per-change all-target/all-feature clippy; independent schema/mechanical review CLEAN. Mechanical compile checks do not claim Task13 automatic behavior. No placeholders or unresolved markers into root integration.

## Authorized upstream composition refinement — supersedes automatic invitation-flag refusal

Parent fact-driven ruling: Ensure actually creates the initial invitation. Automatic handoff supports --require-invite-ack and --invite-deadline for INITIAL pair creation, independently from the always-addressed ACK-required body and frozen message deadline. Add optional frozen Ensure creation policy Lazy/RequireAck plus duration; ABSENT means historical timed behavior and preserves old bytes/digests/keys. Modern direct send explicitly requests Lazy for a NEW pair invitation. Modern automatic handoff defaults that initial invitation to Lazy and can request timed acceptance. Registration/launch is never acceptance or ACK.

Preserve canonical pair lifecycle exactly: existing joined/pending episodes are reused without policy/deadline/origin changes; rejected/left pairs refuse without reinvite. A later caller requesting different creation policy cannot override/reset an existing episode. Present the ACTUAL retained policy/outcome honestly; AlreadyJoined creates no new invitation/obligation. Do not manufacture a separate Invite child: automatic remains Ensure→Send with exact canonical attachment/publication/completion proof. Existing explicit paths retain their Invite children. Test initial Lazy/RequireAck/default-duration/explicit-duration, historical absent exact bytes, duplicate policy mismatch retaining original, AlreadyJoined, rejected/left refusal and independent message/invitation deadlines (7/11 seconds).

All automatic and explicit handoff routes acquire exactly one nonempty valid UTF-8 source <=65536 bytes once, freeze exact body/policy/durations before effects, preserve NUL/whitespace/newlines and retry without rereading. Require send-time/invitation/automatic/direct capabilities as applicable BEFORE journal/allocation/Ensure/topology/native effects. Hook/setup/trust/profile readiness is advisory; structural/caller/seat/native grammar/actually-used owned fullconfig fingerprint checks and truthful native trust refusal remain mandatory. No auto trust/accept/ACK/adoption.

Released main478ec4a9 migrations0029_direct_threads and0030_automatic_handoff remain exact. Our unlanded29..35 shift31..37; add migration38 for automatic body-bearing original/recovery capacities proven with actual serializers, never rewrite released30 SQL. Preserve historical retained rows/requests/digests, genuine main30→combined upgrade/rollback/reopen and all dependent FK relationships. Count nested body copies across child/attachment/recovery/terminal/archival and enforce exact limits before effects; no clipping/unconditional globalwire increase.

Owned composition epic ht-0lo.8 has serial leaves .8.1/.8.2/.8.3, plan ordinals12/13/14. Work remains in separate isolated candidate workspace, distinct task branches/provenance reusing one owned warm validation slot serially. No provisional or unresolved composition merges into root integration until the whole reviewed A/B/C contract passes. Root closure waits. Parent owns final PR roast and subsequent expensive/defaultfeature exact-tip readiness. Prior11 ordinals and three edge-audit budget remain unchanged.

### Ensure creation-policy capability refinement

New optional Ensure creation-policy fields require their OWN honest additive capability, provisionally direct_threads.ensure_creation_policy_v1; invitation.confirmation_v1 and old automatic/direct capability do not establish support in released v0.7.0 Ensure handlers. Task13 owns constant, canonical activation/advertisement and every modern caller preflight. Advertise only when canonical Ensure policy creation is actually active. Modern Lazy directsend remains exempt from messages.send_time_receipts_v1 but MUST require this new Ensure-policy capability BEFORE journal/pair/allocation/native effects; modern automatic policy likewise requires it before any topology/durable/native effects, in addition to applicable automatic/invitation/receipt capabilities. Explicit historical absent fields preserve historical behavior/bytes and completed inspection; unsupported modern semantics never silently fall back. Test real old-v0.7 capability peers for automatic and direct routes, INCLUDING Lazy directsend, and assert no new journal/pair/topology/native effects. Task12 supplies the actual schema/canonical activation-ready cutpoint if needed with additive38; do not invent extra DDL merely to gate a capability. This is parent-authorized autonomous bounded composition within ht-0lo.8; no new confirmation request is required.

## Parent-authorized AGY composition refinement — latest provisional migration sequence

Mainab2291c5 v0.8.0 supersedes the earlier upstream478ec4a9 composition target. Released migrations0029_direct_threads.sql,0030_automatic_handoff.sql and0031_seat_wake_on.sql remain byte-for-byte exact. Our unlanded redesign29..35 shift to32..38, and automatic capacity migration becomes0039_automatic_handoff_body_bounds.sql. Prior31..37/capacity38 proposals are planning history only. Execute genuine populated released main31→combined latest upgrades/rollback/reopen; never relabel initialized latest or rewrite historicalSQL. A imports actualab2291c5 only after its frozen build drains; an owned-branch478 mechanical checkpoint is permitted ancestry, never ungated root promotion.

Additional bounded Task15 ht-0lo.8.4 owns AGY neutral hook-event/setup/launch and delivery-ledger/pull semantic composition after Task12. Task13 remains dependent only on Task12. Task14 consumes BOTH Task13 and Task15 actual reviewed artifacts; no invented B↔D dependency. Work stays in isolated .8 candidate branches/provenance; one owned warm validation slot has one writer/Cargo command at a time. Whole A/B/C/D contract must clear before root integration/closure. Original11 ordinals and prior edge-audit budget unchanged.

Preserve parent-authorized optional frozen Ensure creationpolicy Lazy/RequireAck+duration absenthistoricaltimed, modern directsendLazy initialinvite and automatichandoff independentACKbody/invite deadlines; no superseded automaticflag refusal. Task13 also owns the approved structural response-only transport seam: request-known AutomaticBootstrap plus matching result family can use a measured narrow response allowance; default request/service/watch/unrelatedreply limits remain1MiB. Numeric ceiling requires actual serializer evidence, no clipping/dedup/history rewrite.

AGY hooks/setup/launch compose via neutral registry/adapter traits, not harness-name branches in neutral core. Stable per-event owned hook command bridges remain exact-route, equivalent reinstall unchanged, foreign/drifted groups survive and explicit migration recoverable. AGY rejects rewritten/ownednativeargv: preserve structural/native grammar/host and usertrust refusal, while hook/setup/plugin/profile readiness is diagnostic-only for an otherwise safe launch. Recheck actually-used owned fullconfig fingerprint only when used; absence never fabricates one. No auto trust/accept/ACK/adoption or version admission gates.

New in-turn delivery must expose independent still-open warnings for EACH canonical original receipt sender/inviter, including departed origins, after informational frontier settlement and offered-ledger/Unchanged/whole-read suppression. Preserve one coalesced condition/event, exact associations, partial/final lifetime, passive clear, canonical nullable attribution and service-author handling; never synthesize sender from NULL/currentmembership. Lazy invitation is discoverable but not timed/native attention, RequireAck/historicalNone remaintimed. Preserve finite due-driver highwaters/budget/fairness/expandedH combinedreset; delivery visibility or native completion cannot gate discovery. Message ACK remains receipt only.

Read ACTUAL upstream AGENTS.md. Every NEW commit includes Outside-adapter: <none | comma-separated repo-relative paths/globs> accounting for all changed paths outside src/harness/agy/; diff remains authoritative. Run scripts/check-harness-neutral in addition to fmt/clippy, no new permissive allowlist entries without actual frozen-wire/registration rationale. No real ~/.claude/~/.codex/~/.aisw/~/.gemini writes. Use isolatedHOME/config, taggedchildren/private sessions and scoped leak cleanup. Lazy messages never idlewake unless wake_on:all opted-in; ACKmessages nudgeclass, generating/tool-running servedinturn notnudged; wakeprompt contentfree. Never gate on exact harnessversion, degrade only observedbroken capability. Parent owns final roast then exact-tip expensive/defaultfeature readiness.

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



### Composition execution boundary

Serial distinct task branches reuse one owned warm validation slot in isolated .8 candidate workspace with exact branch/source/test provenance. No simultaneous writer or Cargo lane. Source frozen during builds; preflight actual schema/caller/membership/binding/permit/serializer/cutpoint before first meaningfulRED. Wake completionbool is restoration, Text versusJSON explicit, nonzero selection mandatory. New target only for actual need with explicit registration/test-support feature and scoped coordinator admission. Never alter runstate/ledger from task worktrees. Parent owns final PR roast followed by exact-tip defaultfeature/expensive readiness; this task does not certify later tasks or authorize rootintegration before whole A/B/C review.

## Task 13: Automatic Ensure creation policy and durable handoff composition

filesTouched: ["src/protocol/commands.rs", "src/protocol/capabilities.rs", "src/protocol/handoff.rs", "src/protocol/handoff/automatic.rs", "src/protocol/handoff/automatic_bootstrap.rs", "src/cli/commands.rs", "src/cli/mod.rs", "src/cli/input.rs", "src/cli/direct_send.rs", "src/cli/handoff.rs", "src/cli/handoff_delivery.rs", "src/cli/topology_handoff.rs", "src/cli/topology_handoff/automatic.rs", "src/cli/topology_runtime.rs", "src/cli/topology_recover/automatic.rs", "src/cli/journal.rs", "src/cli/retry.rs", "src/store/control.rs", "src/store/handoff.rs", "src/store/handoff/automatic.rs", "src/store/topology_handoff/automatic.rs", "src/store/topology_handoff/attachment.rs", "src/store/topology_handoff/runtime.rs", "src/archival_legacy.rs", "src/store/archival.rs", "src/service/dispatch.rs", "src/daemon/control.rs", "tests/cli/commands.rs", "tests/cli/direct_send.rs", "tests/cli/handoff.rs", "tests/cli/handoff_delivery.rs", "tests/cli/topology_handoff.rs", "tests/cli/journal.rs", "tests/handoff_topology_authority.rs", "tests/cli/launch.rs", "tests/harness/launch.rs", "tests/store/control.rs", "tests/store/archival_legacy.rs", "tests/protocol/capabilities.rs", "README.md", "docs/agent-usage.md", "integrations/skill/SKILL.md", "TRUST-POLICY.md", "src/protocol/wire.rs", "src/daemon/transport.rs", "src/client/local.rs", "tests/daemon/transport.rs"]

bead: ht-0lo.8.2

Dependencies: ht-0lo.8.1

### Proposed full bead body (coordinator persistence required)

```json
{
  "id": "ht-0lo.8.2",
  "title": "Automatic Ensure creation policy and durable handoff composition",
  "description": "Own optional frozen Ensure creation policy Lazy/RequireAck+duration absent=historical timed, modern directsend Lazy newpair and automatichandoff lazy/timed initial creation, body acquisition/freezing, independentACKbody/message deadline, capability-before-effects, canonical automatic validators/adapters/public routes/journal/retry/loader/archival/readiness and user docs. Consume Task12 actual additive schema/capacity map; do not edit released history or invent standalone Invite. Preserve existing joined/pending/rejected/left pair semantics exactly and actual outcome presentation. Normative refinement in upstream-composition-refinement.md supersedes prior flag refusal.",
  "acceptance_criteria": "Actual automatic existing-seat/existing-pane/launched-pane/new-tab and explicit routes support nonemptyUTF8<=65536 positional/file/stdin acquired once, maximum ordinary/escaped/NUL/whitespace exact bodies and source mutation/deletion retries. Initial Ensure Lazy/RequireAck/default/explicit durations correct; absent historical exact requests/digests unchanged; duplicate policy mismatch retains original episode/deadline/origin, AlreadyJoined creates no obligation, rejected/left never reinvite. Message remains addressed ordinaryACK independently,7second message/11second invite, creation/send-based clocks, registration/launch never acceptance/ACK. Capability discovery precedes journal/allocation/Ensure/topology/native effects incl ordinary directsend and pending retries; completed historical inspection unchanged with Task1 conservative unsupported limits. Exact frozen children/attachment/published proof, lostEnsure\u2192reject/leave, lostSend, childcomplete/outerincomplete, possible-start/unknowncreate preserve no duplicate/restage/relaunch. Actual all body-bearing loader/recovery/terminal/archival paths honor Task12 proven ceilings and before-effect refusal. Advisory missing/untrusted/wrongprofile hooks with actualusedfingerprint and structural/grammar/trust guards retained. Focused public/canonical tests, fmt/clippy; independent interface+publicroute review includes clean-auto-merge paths, not just conflicts. New Ensure creation-policy capability direct_threads.ensure_creation_policy_v1 is independently required for modern policy, including Lazy directsend beforeeffects; advertise only actual canonical activation. Oldv0.7 automatic/direct peers includingLazy prove zeroeffects; Task13 owns semanticcapability, Task12 activation-ready schema ifneeded without speculativeDDL. Parent-settled aggregate envelope: escaped65536-byte body supported under ordinary bounded metadata, not simultaneous independent metadata maxima. Full-max combinations refuse truthfully before durable/native effects; general requests remain1MiB. Exact supported original must fit every later Complete/Recover request, reply, local/recovery serializer including near-aggregate boundary and future bounded fields, or initial preflight refuses. Never accept an origin that later becomes unrecoverable. Preserve exact retained bytes/digests; positive maxbody ordinarymetadata and fullmax negative controls required. Exceptional AutomaticBootstrap response only, final numeric limit derived from full supported-envelope fixtures.",
  "dependencies": [
    "ht-0lo.8.1"
  ],
  "parent": "ht-0lo.8",
  "status": "proposed; coordinator creates bead"
}
```

### Acceptance and settled producer contract

Actual automatic existing-seat/existing-pane/launched-pane/new-tab and explicit routes support nonemptyUTF8<=65536 positional/file/stdin acquired once, maximum ordinary/escaped/NUL/whitespace exact bodies and source mutation/deletion retries. Initial Ensure Lazy/RequireAck/default/explicit durations correct; absent historical exact requests/digests unchanged; duplicate policy mismatch retains original episode/deadline/origin, AlreadyJoined creates no obligation, rejected/left never reinvite. Message remains addressed ordinaryACK independently,7second message/11second invite, creation/send-based clocks, registration/launch never acceptance/ACK. Capability discovery precedes journal/allocation/Ensure/topology/native effects incl ordinary directsend and pending retries; completed historical inspection unchanged with Task1 conservative unsupported limits. Exact frozen children/attachment/published proof, lostEnsure→reject/leave, lostSend, childcomplete/outerincomplete, possible-start/unknowncreate preserve no duplicate/restage/relaunch. Actual all body-bearing loader/recovery/terminal/archival paths honor Task12 proven ceilings and before-effect refusal. Advisory missing/untrusted/wrongprofile hooks with actualusedfingerprint and structural/grammar/trust guards retained. Focused public/canonical tests, fmt/clippy; independent interface+publicroute review includes clean-auto-merge paths, not just conflicts.

## Authorized upstream composition refinement — supersedes automatic invitation-flag refusal

Parent fact-driven ruling: Ensure actually creates the initial invitation. Automatic handoff supports --require-invite-ack and --invite-deadline for INITIAL pair creation, independently from the always-addressed ACK-required body and frozen message deadline. Add optional frozen Ensure creation policy Lazy/RequireAck plus duration; ABSENT means historical timed behavior and preserves old bytes/digests/keys. Modern direct send explicitly requests Lazy for a NEW pair invitation. Modern automatic handoff defaults that initial invitation to Lazy and can request timed acceptance. Registration/launch is never acceptance or ACK.

Preserve canonical pair lifecycle exactly: existing joined/pending episodes are reused without policy/deadline/origin changes; rejected/left pairs refuse without reinvite. A later caller requesting different creation policy cannot override/reset an existing episode. Present the ACTUAL retained policy/outcome honestly; AlreadyJoined creates no new invitation/obligation. Do not manufacture a separate Invite child: automatic remains Ensure→Send with exact canonical attachment/publication/completion proof. Existing explicit paths retain their Invite children. Test initial Lazy/RequireAck/default-duration/explicit-duration, historical absent exact bytes, duplicate policy mismatch retaining original, AlreadyJoined, rejected/left refusal and independent message/invitation deadlines (7/11 seconds).

All automatic and explicit handoff routes acquire exactly one nonempty valid UTF-8 source <=65536 bytes once, freeze exact body/policy/durations before effects, preserve NUL/whitespace/newlines and retry without rereading. Require send-time/invitation/automatic/direct capabilities as applicable BEFORE journal/allocation/Ensure/topology/native effects. Hook/setup/trust/profile readiness is advisory; structural/caller/seat/native grammar/actually-used owned fullconfig fingerprint checks and truthful native trust refusal remain mandatory. No auto trust/accept/ACK/adoption.

Released main478ec4a9 migrations0029_direct_threads and0030_automatic_handoff remain exact. Our unlanded29..35 shift31..37; add migration38 for automatic body-bearing original/recovery capacities proven with actual serializers, never rewrite released30 SQL. Preserve historical retained rows/requests/digests, genuine main30→combined upgrade/rollback/reopen and all dependent FK relationships. Count nested body copies across child/attachment/recovery/terminal/archival and enforce exact limits before effects; no clipping/unconditional globalwire increase.

Owned composition epic ht-0lo.8 has serial leaves .8.1/.8.2/.8.3, plan ordinals12/13/14. Work remains in separate isolated candidate workspace, distinct task branches/provenance reusing one owned warm validation slot serially. No provisional or unresolved composition merges into root integration until the whole reviewed A/B/C contract passes. Root closure waits. Parent owns final PR roast and subsequent expensive/defaultfeature exact-tip readiness. Prior11 ordinals and three edge-audit budget remain unchanged.

### Ensure creation-policy capability refinement

New optional Ensure creation-policy fields require their OWN honest additive capability, provisionally direct_threads.ensure_creation_policy_v1; invitation.confirmation_v1 and old automatic/direct capability do not establish support in released v0.7.0 Ensure handlers. Task13 owns constant, canonical activation/advertisement and every modern caller preflight. Advertise only when canonical Ensure policy creation is actually active. Modern Lazy directsend remains exempt from messages.send_time_receipts_v1 but MUST require this new Ensure-policy capability BEFORE journal/pair/allocation/native effects; modern automatic policy likewise requires it before any topology/durable/native effects, in addition to applicable automatic/invitation/receipt capabilities. Explicit historical absent fields preserve historical behavior/bytes and completed inspection; unsupported modern semantics never silently fall back. Test real old-v0.7 capability peers for automatic and direct routes, INCLUDING Lazy directsend, and assert no new journal/pair/topology/native effects. Task12 supplies the actual schema/canonical activation-ready cutpoint if needed with additive38; do not invent extra DDL merely to gate a capability. This is parent-authorized autonomous bounded composition within ht-0lo.8; no new confirmation request is required.

## Parent-authorized AGY composition refinement — latest provisional migration sequence

Mainab2291c5 v0.8.0 supersedes the earlier upstream478ec4a9 composition target. Released migrations0029_direct_threads.sql,0030_automatic_handoff.sql and0031_seat_wake_on.sql remain byte-for-byte exact. Our unlanded redesign29..35 shift to32..38, and automatic capacity migration becomes0039_automatic_handoff_body_bounds.sql. Prior31..37/capacity38 proposals are planning history only. Execute genuine populated released main31→combined latest upgrades/rollback/reopen; never relabel initialized latest or rewrite historicalSQL. A imports actualab2291c5 only after its frozen build drains; an owned-branch478 mechanical checkpoint is permitted ancestry, never ungated root promotion.

Additional bounded Task15 ht-0lo.8.4 owns AGY neutral hook-event/setup/launch and delivery-ledger/pull semantic composition after Task12. Task13 remains dependent only on Task12. Task14 consumes BOTH Task13 and Task15 actual reviewed artifacts; no invented B↔D dependency. Work stays in isolated .8 candidate branches/provenance; one owned warm validation slot has one writer/Cargo command at a time. Whole A/B/C/D contract must clear before root integration/closure. Original11 ordinals and prior edge-audit budget unchanged.

Preserve parent-authorized optional frozen Ensure creationpolicy Lazy/RequireAck+duration absenthistoricaltimed, modern directsendLazy initialinvite and automatichandoff independentACKbody/invite deadlines; no superseded automaticflag refusal. Task13 also owns the approved structural response-only transport seam: request-known AutomaticBootstrap plus matching result family can use a measured narrow response allowance; default request/service/watch/unrelatedreply limits remain1MiB. Numeric ceiling requires actual serializer evidence, no clipping/dedup/history rewrite.

AGY hooks/setup/launch compose via neutral registry/adapter traits, not harness-name branches in neutral core. Stable per-event owned hook command bridges remain exact-route, equivalent reinstall unchanged, foreign/drifted groups survive and explicit migration recoverable. AGY rejects rewritten/ownednativeargv: preserve structural/native grammar/host and usertrust refusal, while hook/setup/plugin/profile readiness is diagnostic-only for an otherwise safe launch. Recheck actually-used owned fullconfig fingerprint only when used; absence never fabricates one. No auto trust/accept/ACK/adoption or version admission gates.

New in-turn delivery must expose independent still-open warnings for EACH canonical original receipt sender/inviter, including departed origins, after informational frontier settlement and offered-ledger/Unchanged/whole-read suppression. Preserve one coalesced condition/event, exact associations, partial/final lifetime, passive clear, canonical nullable attribution and service-author handling; never synthesize sender from NULL/currentmembership. Lazy invitation is discoverable but not timed/native attention, RequireAck/historicalNone remaintimed. Preserve finite due-driver highwaters/budget/fairness/expandedH combinedreset; delivery visibility or native completion cannot gate discovery. Message ACK remains receipt only.

Read ACTUAL upstream AGENTS.md. Every NEW commit includes Outside-adapter: <none | comma-separated repo-relative paths/globs> accounting for all changed paths outside src/harness/agy/; diff remains authoritative. Run scripts/check-harness-neutral in addition to fmt/clippy, no new permissive allowlist entries without actual frozen-wire/registration rationale. No real ~/.claude/~/.codex/~/.aisw/~/.gemini writes. Use isolatedHOME/config, taggedchildren/private sessions and scoped leak cleanup. Lazy messages never idlewake unless wake_on:all opted-in; ACKmessages nudgeclass, generating/tool-running servedinturn notnudged; wakeprompt contentfree. Never gate on exact harnessversion, degrade only observedbroken capability. Parent owns final roast then exact-tip expensive/defaultfeature readiness.

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



### Composition execution boundary

Serial distinct task branches reuse one owned warm validation slot in isolated .8 candidate workspace with exact branch/source/test provenance. No simultaneous writer or Cargo lane. Source frozen during builds; preflight actual schema/caller/membership/binding/permit/serializer/cutpoint before first meaningfulRED. Wake completionbool is restoration, Text versusJSON explicit, nonzero selection mandatory. New target only for actual need with explicit registration/test-support feature and scoped coordinator admission. Never alter runstate/ledger from task worktrees. Parent owns final PR roast followed by exact-tip defaultfeature/expensive readiness; this task does not certify later tasks or authorize rootintegration before whole A/B/C review.

## Task 14: Combined upstream composition focused integration proof

filesTouched: ["tests/cli/direct_send.rs", "tests/cli/handoff.rs", "tests/cli/handoff_delivery.rs", "tests/cli/topology_handoff.rs", "tests/handoff_topology_authority.rs", "tests/handoff_topology_cli.rs", "tests/store/schema.rs", "tests/store/receipts.rs", "tests/integration/notice_wake.rs", "tests/integration/setup_canary_seam.rs", "tests/scheduler/deadlines.rs", "tests/cli/skill.rs", "README.md", "integrations/skill/SKILL.md", "TRUST-POLICY.md"]

bead: ht-0lo.8.3

Dependencies: ht-0lo.8.2, ht-0lo.8.4

### Recorded bead body (preparation refinement)

```json
{
  "id": "ht-0lo.8.3",
  "title": "Combined upstream composition focused integration proof",
  "description": "Own missing focused combined integration proof after reviewed Task12 exact020/main31 composition and BOTH actual frozen Task13/Task15 reports. Declared existing target/test/doc surfaces only; no new production scope or routinefullsuite. Verify automatic+explicit/direct routes, oldmain31 upgrade, bounded canonical summary/local sidecar/NonStarted/aggregate envelope and scoped response, passive invitation goal/content-cache/channel/scheduler/AGY seams, canonicalorigin lifecycle and expandedH/D/I. Coordinator owns final integration/PRroast/defaultfeature/readiness.",
  "acceptance_criteria": "Actual reviewed candidate combines A main02041b93/released1..31 exact, redesign32..38 and capacity39 with B+D. Focused selection proves representative public automatic+explicit/direct policy/source/retry/capability paths; ordinarymetadata maximumescaped65536 positive plus aggregateboundary success/aboveboundary zeroeffects. Distinct bounded canonical completion summary versus exact optional local diagnostic sidecar, criticalsummary-beforeartifact cutpoints, NonStarted truthful recovery and no relaunch, missingartifact completed inspection, unchanged historicalbytes. Conditional2MiB only request-known matching AutomaticBootstrap replies after allfuturetypedshape proof; genericrequests/service/watch/errors/unrelatedreplies1MiB remain, wrongfamily/length/correlation controls. D passiveLazyinvitation goal/continuation/contextbookkeeping positive without acceptance/ACK/ordinaryidlewake, bounded contentcache invalidation on removal/arrival and channelactualdiscovery; persistent canonical originalsender/inviter eligibility after wholeoffer/Unchanged/noticefrontier under actualscheduler/AGYguards, partial/finalclear passive. Genuineoldmain31 populated rollback/reopen/retainedrows and actual expandedH total100/shared50combinedreset preserved. Do notduplicate allleafchecks; select actualcombinedseams plusindependentpositive/negativecontrols using registeredtargets/nonzerocount. fmt/allfeatureclippy/neutralcheck and ownfreshreview; scopedleakcheck afterfocusedsweep; no routinefullsuite. Final parentPRroast/exacttipdefaultfeature/expensive readiness deferred.",
  "dependencies": [
    "ht-0lo.8.2",
    "ht-0lo.8.4"
  ],
  "parent": "ht-0lo.8",
  "status": "open; awaits reviewed B and D"
}
```

### Preparation status and exact producer baseline

This prepared brief does not authorize dispatch. Task12's final e196ae1140499612d63bddc43a1624b70d004c62/tree6ab6831b7972af18ce600b553647db7185c31ccc is the authoritative A evidence, with production/schema c928a1ff3e25c2a0c251f5d87f060380afe8e39b. It includes actual upstream02041b93aa974cc9c736699f7b79c9d13557d884 ancestry and exact unchanged released migrations1..31; seven unpublished redesign bodies move32..38 and additive capacity39. Earlier main30/31..37/capacity38/three-leaf proposals are SUPERSEDED HISTORY, not active fixture instructions. Task12 storage/mechanical review does NOT certify B public maximum-body/aggregate admission or D fullcontext delivery. Read actual final A report, capacity inventory and review, current task13/15 briefs, task13-wire-preflight and latest upstream-composition-refinement before dispatch. B/D are pending: obtain actual frozen reports/merged source and test identities before choosing their exact APIs/filter names. Do not infer final symbols from proposed summary/sidecar/passive metadata names. B and D independently consume A; C depends BOTH, no B-D dependency invented.

### Focused combined proof selection

Choose the smallest set demonstrating combined interfaces; consume reviewed producer evidence for isolated behavior rather than repeat every suite. Add only missing seam fixtures in existing declared surfaces; extra source/test paths require actual scoped coordinator admission. No invented new suite/target, no routinefullsuite. Independent controls must make the intended cutpoint reachable, not fail on setup/compiler errors, unknownflags, genericUnsupported or emptyfilter.

1. Actual parsed automatic existing-seat/existing-pane/launched-pane/newtab and explicit-thread path coverage: representative combined cases plus frozen producer evidence cover every route. Modern directsend Lazy newpair and automatichandoff initial Lazy/RequireAckcreation retain exact existingpair joined/pending/rejectedleft behavior; 7second body/11second invitation independent, body alwaysaddressedACK. Test actual oldv0.7 peer advertising olddirect/automatic/Lazy but lacking newEnsurecreationpolicycap: Lazy directsend still refuses beforejournal/pair/nativeeffects; ordinaryreceiptcap independentlyrequired. Historical absentV1/V2 exactbytes/digests/keys and completed read-only status need no newpolicycap/admission. LostEnsure→reject/leave, lostSend, childcomplete/outerincomplete and unknowncreate/possible-start retain no duplicate/restage/relaunch.
2. Public supported aggregate envelope: ordinarymetadata max65536 escapedbody is positive; actual nearaggregate admissible input succeeds through later Complete/Recover/local/reply, aboveboundary refuses beforepair/journal/topology/nativeeffects. Use real generated startup argv/template and canonical relation, enforced ID/creation/witness/reason/futurefield bounds, not Task12 lower-bound measurements or fullindependentmetadata maxima. Task12 coherent typed serializer inventory motivates scoped admission and response proof; read its exact final measurements rather than treating any fixture as a universal ceiling. Modern canonical completion is a distinct bounded/versioned summary with exact composedargv/outcome/identity; full diagnostic report remains exact locally with distinct digest/artifactmarkers. Drive >2MiBdiagnostics, summarysynced-beforeartifactattempt, filesystemfault/crash/lostComplete+reopenedjournal cutpoints and missing/unreadableartifact presentation; critical completion/recovery mustnotdecodeartifact or restartnative. Include truthful NonStarted branch/control from actual B report: never classify refusal or nonstart as started; preserve its actual retry/native safety contract rather than invent a field/API.
3. Wire: conditional2MiB AutomaticBootstrap RESPONSE only after actualtyped admission proves ALLsupported future responsevariants/framing fit. Client chooses boundedallocation from knownrequest; matchingresultfamily/canonicalvalidation/correlations enforced. Genericrequest/service/watch/error/unrelatedreply stay1MiB; boundary+1 rejectedbeforeallocation, wrongresultfamily/boot/requestid controls, canonicalBegin admission and publicpreflight beforeeffects. Actual requestserializers still fit1MiB. No globalcap increase/clipping/dedupidentity/history.
4. D delivery: actual V2canonicalgoal positive plus historicalomittedexactbytes and byteconstrained usableinspectioncontinuation beforeacceptance; whole(item) is displaycompleteness. Lazy invitations reach realAssembler→page→servicepull→AGYturnhook and modernwatch→channelconsumer context/bookkeeping without fabricatedmessageACK/acceptance/nativeidlewake. Passivecontentcache is separate from wakefingerprint: arrival/removal, olderitemremoval, saturatedwindowpromotion and canonical SetTopic revision invalidate actualvisiblecontent; unchanged items and unrelatedcanonicaldecisions do not. Preserve invitation IDs and historical omitted additive metadata bytes. Actualchanneldiscovery/drain—not emitter-only—observespassivechange without idle submit or wakefingerprint churn; accepted olderconsumer limitation statedhonestly. Preserve main unprefixed Claude log.
5. Persistent warning condition: actual fulloffer then identicalversion Unchanged is allowed, but real mod/native eligibility remains whileopen independently from presentationNextPull/whole_read/noticefrontier. Shared physical/publishedsparse A1/A2/B1 has all departedcanonicalorigins, oneevent/exactassociations, partialACK preservesremaininglifetime, finalACK onepassiveclear; inviter analogous, NULL/serviceauthor/observer independentcontrols. Exercise actual scheduler→native and AGYpull/mod under presence/idle/draft/working/hostoutage/backoff/coalescing, no duplicatecontextflood or forcedwake everypull. Stableeventsetup exactroute/equivalentreinstall/foreignownership and AGY ownedargvrewriterefusal remain; missing/stale/disabled/unknownplugin readiness diagnostic, actuallyusedfingerprint/structural/nativegrammar/trust negatives remain.
6. Schema/globaldiscovery: genuine populated RELEASEDmain31 executes immutableprefix physicallythenupgrade/reopen/rollback, retaineddirect/automatic original/creation/attachment/recovery/completion graph and FKvalidity preserved. No latestinitialize/relabelversion or broadDDLnormalization. Representative combined Hpositive/H0 actualdriver proof includes allrepair/bootstrap/duplicatevisits, total100globalquantum/shared50repair/lending/SQLfinitehighwaters, combinedatomicreset/restart/arrival/terminalbacklog and250eachclass; nativeadmission doesnotgate discovery. Consume unchanged reviewed Task7/A detailedarithmetic evidence where no combinedchange affects it.

### Actual registered target and filter inventory (read-only metadata; not execution evidence)

A metadata has lib+bin and five explicit testtargets (seven total): combined, hook_entrypoint, integration, package, service; each explicittesttarget requires test-support. Every command uses --locked --all-features and explicit --lib/--test. Refresh actual final producer symbols with source/module registration and nonzero runtime selections; do not cargo-list or rebuild during preparation.

| Existing target | Actual module/fixture inventory and selection purpose |
| --- | --- |
| --lib | store::connection::tests::released_main31_populated_automatic_graph_physically_upgrades_and_reopens; store::effective::tests::dormant_upgrade_actual_v31_backfill32_reopens_and_rolls_back_without_recapture; current public schema39 positive/rollback controls. |
| --lib | automatic_capacity_actual_serializer_maximum_body_records (tests/store/topology_attachment_guard.rs registered under canonical control tests); actual canonical Complete positive control. Reuse A evidence; B final publicaggregate controls must establish admission beyond this fixture. |
| --lib | receipt_discovery_ in store::receipts::tests; scheduler::deadlines::tests. Select actual combinedfairness/reset/origin cases rather than allstore suites. |
| --lib | cli::direct_send, cli::handoff::tests, cli::handoff_delivery::tests, cli::topology_handoff::tests use actual parsed public producers; choose frozen B final creationpolicy/aggregate/summary/artifact/NonStarted tests after report. Existing automaticexisting/lostreply/completed tests are controls, not invented finaltestnames. |
| --lib | service::delivery_pull::tests, service::mod_channels::tests and push_channel_tests, harness::agy::{launch,setup,delivery}::tests, cli::watch::tests, delivery extraction modules: existing hostpaths for D fullpull/cache/channel/readiness cases. Select final D exactnames only after report. |
| --test combined | topology_handoff (tests/store/topology_handoff.rs), lazy_inbox_v2 (tests/protocol/lazy_inbox_v2.rs), invitation_confirmation_inbox_metadata. Target/module checked in tests/combined.rs; use actual boundedgoal/historicalwire and linkedintegration seams. |
| --test hook_entrypoint | agy module in tests/hook_entrypoint/agy.rs: registered PreInvocation actualbuiltCLI/daemon/isolatedpresence fixture for passiveinvitationpositive. |
| --test integration | mod_delivery::agy_sweep (nested tests/integration/agy_sweep.rs), setup_canary_seam; actual daemon+nativeprompt recording/channelintegration for combinednudge/passivecontext/launch. Existing bootstrap assumes timedinvitewake: consume D's correctedsetup, never wait on Lazy wake. |
| Existing JS consumer harness | integrations/claude/mod/tests/delivery.test.ts actual createCore/watch fixture from D's recorded supported command; no guessed package.json/test command. Runs are separate from Cargo and must have actual source/provenance. |

Source registration confirms src/service/delivery_pull.rs paths tests/service/delivery_pull.rs, src/service/mod_channels.rs paths tests/service/mod_channels.rs and tests/delivery/push_channel.rs; these are LIB modules, not the --test service target merely because of filelocation. Likewise AGY adapter tests are lib; tests/integration/agy_sweep.rs nests under mod_delivery. Avoid the prior zero-selection/genericwrongtarget trap. B/D may add scoped admitted tests; freeze inventory from their finalreports before dispatch, do not promise coverage from pending source.

### Verification ownership and independent review

Run focusedcombined selections on exact composedSHA, cargo fmt, nice cargo clippy --locked --all-targets --all-features -- -D warnings and scripts/check-harness-neutral, then ownfresh independentreview of combinedpublic/schema/wire/delivery interfaces INCLUDING cleanautomerge paths. Preserve Outside-adapter trailer accounting, token-efficientcontext and versionlessneutralcore constraints; no real ~/.gemini/otheruserconfig writes. Export uniqueHT_LEAK_RUN_ID BEFORE sweep, inherit in taggedownedchildren/privatefixtures, stopallownedchildren/servers/helpers and run scripts/check-no-leaked-processes --run-id "$HT_LEAK_RUN_ID" afterward. Record selected/pass/fail/ignored and exact artifact/sourceSHA; zero selected/setup failures are noevidence. Parent retains finalPRroast then finalexacttip defaultfeature/expensive readiness and notification; code coordinator closes the root only after all code-phase work and reviews clear. Report parent readiness deferred, notpassed. No routinefullsuite or additional production scope authorized here.


## Task 15: AGY delivery setup and launch semantic composition

filesTouched: ["src/harness/adapter.rs", "src/harness/setup.rs", "src/harness/launch.rs", "src/harness/agy/launch.rs", "src/harness/agy/setup.rs", "src/harness/agy/plugin.rs", "src/harness/agy/delivery.rs", "src/harness/agy/mod.rs", "src/harness/claude/setup.rs", "src/harness/codex/setup.rs", "src/cli/hook.rs", "src/cli/hook_evidence.rs", "src/cli/setup.rs", "src/cli/setup/foreground.rs", "src/cli/launch.rs", "src/delivery/assemble.rs", "src/delivery/ledger.rs", "src/delivery/page.rs", "src/delivery/routing.rs", "src/delivery/settle.rs", "src/service/delivery_pull.rs", "src/store/attention.rs", "src/store/queries.rs", "src/store/seats.rs", "src/store/wake.rs", "tests/harness/adapter_seam.rs", "tests/harness/agy_launch.rs", "tests/harness/agy_setup.rs", "tests/harness/agy_delivery.rs", "tests/harness/agy_wake.rs", "tests/harness/setup.rs", "tests/harness/launch.rs", "tests/cli/hook_neutral.rs", "tests/cli/launch.rs", "tests/hook_entrypoint/agy.rs", "tests/service/delivery_pull.rs", "tests/delivery/extraction.rs", "tests/delivery/ledger.rs", "tests/store/attention.rs", "tests/store/mod_seat_view.rs", "tests/scheduler/hook_pull_routing.rs", "tests/integration/agy_sweep.rs", "tests/handoff_agy.rs", "README.md", "docs/agent-usage.md", "docs/dev/outside-adapter-report.md", "integrations/skill/SKILL.md", "TRUST-POLICY.md"]

bead: ht-0lo.8.4

Dependencies: ht-0lo.8.1

### Proposed full bead body (coordinator persistence required)

```json
{
  "id": "ht-0lo.8.4",
  "title": "AGY delivery setup and launch semantic composition",
  "description": "Own bounded AGY neutral hook-event/setup/launch and new offered-ledger/pull warning semantics after actual reviewed Task12 ab2291c5 composition. Consume stable A registry/adapter/delivery interfaces; do not depend on B policy work. Sole shared launch-readiness owner (src/cli/launch.rs,src/harness/launch.rs), preserving B generic automatic producer interfaces. Exact stableevent owned bridges, AGY rewrite refusal, diagnostic hook/plugin/profile readiness versus structural/currentownedfingerprint/trust guards; independent canonicalorigin warning visibility through pull/ledger and passiveclear. Preserve NULL/service attribution and finite due-driver, no new authority/evidence. Read normative AGY refinement and actual AGENTS; scope extras require concrete admission.",
  "acceptance_criteria": "Actual neutral hook-event command/dispatch/setup bridges preserve exact canonical3field routing and pinnedfallback, stable per-event commands on equivalent reinstall, exact manifest/group ownership, foreign/drifted/legacy groups and interrupted explicit migration. No harness branches in neutralcore or versiongating. Actual AGY launch tests distinguish advisory missing/stale/disabled/unknownplugin/profile/hook state from structural/argv/native rewrite refusals and actuallyusedconfigurationfingerprint recheck; unsupportedownedargv remainsrefused, no fabricatedfingerprint/trustapproval/nativeaccept/ACK. Real service DeliveryPull path exercises hook_pull_view\u2192mod_seat_view/fingerprint\u2192attention_version\u2192ledgerUnchanged/whole_read/reoffer/window\u2192pagedsource/assembler\u2192settlement/confirm for EACH canonical original sender/inviter incl departed and settlednoticefrontier; no unrelatedpending masks earlygates, no helper-onlyproof. Shared A1/A2/B1 canonicalphysical/publishedsparse warning staysoneevent, partialACK retainsstillopencondition, finalsettlement clears passively with no warningonlywake, exactassociations/provenance unchanged. Dynamic addedorigin after previouslyoffered event becomesvisible without fakeevent; NULL attribution/serviceauthors remainhonest, no inferredseat. Lazyinvitation discoverablewithoutattention; RequireAck/historicalNone timed; complete/truncated offer/confirm and ACKboundary truthful, boundedpage/ledger/reoffer retained, lazy idlewake onlywake_on:all. Due-driver finite SQL highwaters,total100/shared50repair/expandedH, combinedatomicreset/NULL semantics unchanged; delivery/native keys nevergate dueprogress. Focused existing AGY/setup/launch/delivery/integration tests, fmt/alltargetallfeatureclippy and scripts/check-harness-neutral pass; independent fresh semantic review audits complete upstreamcleanmerge/interface and actualearlygate chain. Everynewcommit Outside-adapter trailer truthful; isolatedHOME no ~/.gemini orotherrealconfigs; taggedownedchildren stopped, scopedleakcheck aftersweep. Parent finalroast/exacttipdefaultfeature later; no readiness claim frompriorrelease evidence. Lazy invitation suppresses only confirmation/nativewake, neverinturnpage/context/bookkeeping; actualAssembler+turnhook/servicepull positiveLazyvisible and ordinarynativewakeabsent tested, whole(item) remainsdisplaycompleteness. Properinformationalclassification preservesR9/R17 and explicitwake_onpolicy.",
  "dependencies": [
    "ht-0lo.8.1"
  ],
  "parent": "ht-0lo.8",
  "status": "proposed; coordinator persists bead"
}
```

### Acceptance and actual sibling interface preflight

Actual neutral hook-event command/dispatch/setup bridges preserve exact canonical3field routing and pinnedfallback, stable per-event commands on equivalent reinstall, exact manifest/group ownership, foreign/drifted/legacy groups and interrupted explicit migration. No harness branches in neutralcore or versiongating. Actual AGY launch tests distinguish advisory missing/stale/disabled/unknownplugin/profile/hook state from structural/argv/native rewrite refusals and actuallyusedconfigurationfingerprint recheck; unsupportedownedargv remainsrefused, no fabricatedfingerprint/trustapproval/nativeaccept/ACK. Real service DeliveryPull path exercises hook_pull_view→mod_seat_view/fingerprint→attention_version→ledgerUnchanged/whole_read/reoffer/window→pagedsource/assembler→settlement/confirm for EACH canonical original sender/inviter incl departed and settlednoticefrontier; no unrelatedpending masks earlygates, no helper-onlyproof. Shared A1/A2/B1 canonicalphysical/publishedsparse warning staysoneevent, partialACK retainsstillopencondition, finalsettlement clears passively with no warningonlywake, exactassociations/provenance unchanged. Dynamic addedorigin after previouslyoffered event becomesvisible without fakeevent; NULL attribution/serviceauthors remainhonest, no inferredseat. Lazyinvitation discoverablewithoutattention; RequireAck/historicalNone timed; complete/truncated offer/confirm and ACKboundary truthful, boundedpage/ledger/reoffer retained, lazy idlewake onlywake_on:all. Due-driver finite SQL highwaters,total100/shared50repair/expandedH, combinedatomicreset/NULL semantics unchanged; delivery/native keys nevergate dueprogress. Focused existing AGY/setup/launch/delivery/integration tests, fmt/alltargetallfeatureclippy and scripts/check-harness-neutral pass; independent fresh semantic review audits complete upstreamcleanmerge/interface and actualearlygate chain. Everynewcommit Outside-adapter trailer truthful; isolatedHOME no ~/.gemini orotherrealconfigs; taggedownedchildren stopped, scopedleakcheck aftersweep. Parent finalroast/exacttipdefaultfeature later; no readiness claim frompriorrelease evidence.

## Parent-authorized AGY composition refinement — latest provisional migration sequence

Mainab2291c5 v0.8.0 supersedes the earlier upstream478ec4a9 composition target. Released migrations0029_direct_threads.sql,0030_automatic_handoff.sql and0031_seat_wake_on.sql remain byte-for-byte exact. Our unlanded redesign29..35 shift to32..38, and automatic capacity migration becomes0039_automatic_handoff_body_bounds.sql. Prior31..37/capacity38 proposals are planning history only. Execute genuine populated released main31→combined latest upgrades/rollback/reopen; never relabel initialized latest or rewrite historicalSQL. A imports actualab2291c5 only after its frozen build drains; an owned-branch478 mechanical checkpoint is permitted ancestry, never ungated root promotion.

Additional bounded Task15 ht-0lo.8.4 owns AGY neutral hook-event/setup/launch and delivery-ledger/pull semantic composition after Task12. Task13 remains dependent only on Task12. Task14 consumes BOTH Task13 and Task15 actual reviewed artifacts; no invented B↔D dependency. Work stays in isolated .8 candidate branches/provenance; one owned warm validation slot has one writer/Cargo command at a time. Whole A/B/C/D contract must clear before root integration/closure. Original11 ordinals and prior edge-audit budget unchanged.

Preserve parent-authorized optional frozen Ensure creationpolicy Lazy/RequireAck+duration absenthistoricaltimed, modern directsendLazy initialinvite and automatichandoff independentACKbody/invite deadlines; no superseded automaticflag refusal. Task13 also owns the approved structural response-only transport seam: request-known AutomaticBootstrap plus matching result family can use a measured narrow response allowance; default request/service/watch/unrelatedreply limits remain1MiB. Numeric ceiling requires actual serializer evidence, no clipping/dedup/history rewrite.

AGY hooks/setup/launch compose via neutral registry/adapter traits, not harness-name branches in neutral core. Stable per-event owned hook command bridges remain exact-route, equivalent reinstall unchanged, foreign/drifted groups survive and explicit migration recoverable. AGY rejects rewritten/ownednativeargv: preserve structural/native grammar/host and usertrust refusal, while hook/setup/plugin/profile readiness is diagnostic-only for an otherwise safe launch. Recheck actually-used owned fullconfig fingerprint only when used; absence never fabricates one. No auto trust/accept/ACK/adoption or version admission gates.

New in-turn delivery must expose independent still-open warnings for EACH canonical original receipt sender/inviter, including departed origins, after informational frontier settlement and offered-ledger/Unchanged/whole-read suppression. Preserve one coalesced condition/event, exact associations, partial/final lifetime, passive clear, canonical nullable attribution and service-author handling; never synthesize sender from NULL/currentmembership. Lazy invitation is discoverable but not timed/native attention, RequireAck/historicalNone remaintimed. Preserve finite due-driver highwaters/budget/fairness/expandedH combinedreset; delivery visibility or native completion cannot gate discovery. Message ACK remains receipt only.

Read ACTUAL upstream AGENTS.md. Every NEW commit includes Outside-adapter: <none | comma-separated repo-relative paths/globs> accounting for all changed paths outside src/harness/agy/; diff remains authoritative. Run scripts/check-harness-neutral in addition to fmt/clippy, no new permissive allowlist entries without actual frozen-wire/registration rationale. No real ~/.claude/~/.codex/~/.aisw/~/.gemini writes. Use isolatedHOME/config, taggedchildren/private sessions and scoped leak cleanup. Lazy messages never idlewake unless wake_on:all opted-in; ACKmessages nudgeclass, generating/tool-running servedinturn notnudged; wakeprompt contentfree. Never gate on exact harnessversion, degrade only observedbroken capability. Parent owns final roast then exact-tip expensive/defaultfeature readiness.

### Global constraints (actual upstream ab2291c5 AGENTS.md)

# Agent guidelines for herdr-threads

## Development rules

- Per-change check: `nice cargo clippy --locked --all-targets --all-features -- -D warnings` and `scripts/check-harness-neutral` (harness names stay out of the neutral core; allowlist `scripts/harness-neutral-allowlist.txt`; clippy already type-checks every target; a separate `cargo check` is redundant). Before a merge, also `nice scripts/check-default-features` (a default-feature `cargo check --all-targets` that fails on any warning; CI runs the full default-feature clippy job as well); format: `cargo fmt`.
- Commit trailer `Outside-adapter: <none | list>` (R13 accounting for the agy adapter epic): every commit names, in one trailer line placed with the other trailers, each changed path outside `src/harness/agy/` as a comma-separated list of repo-relative paths or globs, or `none`. The diff stays authoritative; the trailer only annotates it.
- Tests: run the tests relevant to your change (`nice cargo test --locked --all-features <filter>`); the full suite is `nice cargo nextest run --locked --all-targets --all-features` (cargo-nextest: one process per test, every binary in one pool; test groups in `.config/nextest.toml` keep the budget- and latency-sensitive chains one at a time; `nice scripts/full-suite-gate N` runs it N times in a row with leak checks, and `nice scripts/flake-hunt` hunts flaky tests). A test that must not overlap others of its kind gets a nextest test group, not only an in-binary mutex (a mutex does not span processes).
- Never restart, stop or kill the shared Herdr server, and never close Herdr workspaces/panes you did not create. Tests that need Herdr up/down use an isolated named Herdr test session (see `scripts/lib/isolated-herdr.sh`).
- Never write to the real user config: `~/.claude`, `~/.codex`, `~/.aisw`, `~/.gemini`. Tests use isolated HOME / CLAUDE_CONFIG_DIR / CODEX_HOME under a temp dir.
- Never run `git push` or `git stash` (the stash stack is shared across worktrees). Commit on your own branch.
- A finished task stops its processes: every daemon, private Herdr server and helper process a test or script starts is stopped before the task reports done. Spawn test children with `herdr_threads::test_support::spawn` (`spawn_owned` / `command` / `tag`); never a bare `Command::spawn` in `tests/`.
- After a full-suite run (and after each run of the 10-run gate or the integration sweep), run `scripts/check-no-leaked-processes`. Export `HT_LEAK_RUN_ID=<uuid>` before the suite and pass `--run-id "$HT_LEAK_RUN_ID"` to it, so another worktree's live run is never flagged. A non-zero exit lists each leaked pid; stop them (or rerun with `--kill`) and fix the test that leaked them.
- Test targets are listed explicitly (`autotests = false`). A new suite that keeps no process-wide state goes into `tests/combined.rs` as a `#[path]` module (one binary, one link); a suite that redirects stdio, installs a panic hook, runs an in-process daemon, arms failpoints or changes the umask gets its own `[[test]]` entry. A target that uses `herdr_threads::test_support` (including `test_support::spawn`) needs `required-features = ["test-support"]`; otherwise `scripts/check-default-features` and the default-feature clippy job fail.
- The daemon's owner watch exists only in `test-support` builds: always test with `--all-features`.

## Trust policy

[TRUST-POLICY.md](TRUST-POLICY.md) is normative for seat continuity, caller attribution, receipt provenance
and operator repair. Read it before changing anything in `src/identity/`, `src/store/seats.rs`,
`src/store/receipts.rs`, `src/store/control.rs`, `src/protocol/authority.rs`, `src/cli/me.rs`,
`src/cli/hook.rs`, launch or wake.

- Do not add adversarial caller verification; the model is cooperative and same-user.
- Record claims honestly: a new way to attribute an action needs a provenance value defined in the policy.
- Never merge seats, never move a seat or end a binding on heuristic evidence.
- Decide in the daemon against the canonical view (A2); client-local files are hints, never authority.
- A change that weakens an invariant or adds an accepted limit updates TRUST-POLICY.md in the same commit.

## Harness-neutral core

- Harness-specific code lives only in `src/harness/<id>/` behind `HarnessAdapter` and its policy traits. Core code looks the adapter up through the registry by the id from the binding or claim.
- New harness behaviour means a new trait method or policy with a neutral default, never a branch in core.
- Core is `src/cli`, `src/client`, `src/service`, `src/daemon`, `src/store`, `src/host`, `src/protocol`, `src/app` and the neutral `src/harness/*.rs` files. Adapter-owned paths are `src/harness/<id>.rs`, `src/harness/<id>/**` and `src/harness/<id>_*.rs` for the ids in `builtins()`.
- `scripts/check-harness-neutral` enforces this, with `scripts/harness-neutral-allowlist.txt` as the exception list. Permanent entries are only frozen wire shapes, historical migrations and registration. See "Development rules" for how to run the checks.

## Harness and Herdr versions

Herdr and the harnesses (Claude Code, Codex, Hermes, agy) release too often to track, and pinning them would force updates on plugin users. So:

- Assume backward compatibility: what was designed and captured on an older version is assumed to work on newer ones until breakage is detected at runtime.
- On detected breakage, degrade only the broken capability, keyed on what was observed (for example agy's `systemMessage` → `userMessage` downgrade, keyed on the executable identity); everything else keeps working.
- Never gate admission, setup, launch or a feature on an exact version, never ship an exact-version recipe as a gate, and never make a version probe an operational prerequisite.
- Record version facts only as historical evidence and diagnostics.

Worked examples: `docs/superpowers/specs/2026-10-05-harness-versionless-design.md` and the agy design (`docs/superpowers/runs/2026-10-09-agy-harness-adapter/2026-10-09-agy-harness-adapter-design.md`, "Version handling" and "Downgrade").

## Delivery contract

The goal is token efficiency. Sessions that are done and idle stay idle, spinning sessions get efficient delivery, and nudges are rare, because excessive nudges disorient some models. Every delivery policy must keep these invariants:

1. Messages are lazy by default.
2. Lazy messages never wake an idle session (unless the seat opted into `wake_on: all`).
3. Turn hooks and channels deliver every pending item, lazy or not, directly into the model's context, avoiding extra `herdr-threads` calls as far as the harness allows.
4. A nudge wakes a session only when it is idle. A session that is generating, or running a tool, gets no nudge; it is served by in-turn delivery.
5. Messages that require an ACK are always nudge-class.
6. Who performs an idle wake is the only per-harness variation:
   - `Native`: Herdr types the wake marker into an empty composer;
   - `Channel`: the harness wakes itself, as Claude's mod submits a turn.

Every new delivery policy (`DeliveryPolicy`) and every change to wake, poke or check-in delivery is checked against these rules; the 2026-10 audit is in `docs/superpowers/runs/2026-10-09-agy-harness-adapter/delivery-contract-audit.md`.

## Token-efficient communication

Applies to anyone designing or changing comms protocols or any model-visible text (deliveries, hook context, wake prompts, check-ins, agent-facing CLI output): every byte a model reads costs every recipient context and attention.

- (a) Static explanations live once per session in the skill, never repeated per message or per turn.
- (b) Per-message framing is the minimum structure needed to parse and attribute (header line + body), no boilerplate.
- (c) Wake prompts carry no content; the turn-start hook or channel delivers it.
- (d) Prefer ids and short markers over prose.
- (e) Peer messages are cooperative input from local agents (collaborate by default), not adversarial; keep only the invariant that they never override the user's instructions or permissions.
- (f) A new model-visible string states its expected size in the PR/commit and is checked against these rules.

Worked example: this run's R17 (compact cooperative framing) and R18 (content-free wake prompts), `docs/superpowers/runs/2026-10-09-agy-harness-adapter/`.

What we do today:

- Lazy delivery by default; lazy messages never wake an idle session (Delivery contract rules 1-2).
- Nudges only for idle sessions; a generating or tool-running session is served in-turn, never nudged (rule 4).
- Bodies are delivered in-turn instead of `herdr-threads inbox` round trips (rule 3): `integrations/claude/mod/hooks/register.js` (tool.call context), `src/harness/agy/delivery.rs` (PreInvocation systemMessage).
- Wake prompts are content-free (`herdr-threads: wake up`; R18): `src/notification/policy.rs` (`WAKE_UP`), `DeliveryPolicy::wake_text` in `src/harness/adapter.rs`, the mod's idle path in `register.js`.
- Per-message framing is one header line (`[herdr-threads] <kind> <id> in <thread> from <sender>[ markers]:`) plus an indented body, no static explanation per delivery (R17): `src/delivery/frame.rs`, `register.js` `frame()`.
- The skill explains framing, markers and the trust stance once per session; the full skill loads on demand, not in every check-in: `integrations/skill/SKILL.md` (Trust model), `src/harness/agy/plugin.rs` (agy plugin `skills/`).
- Long bodies are truncated at 8 KiB with a one-line `body <id>` continuation; pages are bounded (32 items / 64 KiB): `src/protocol/watch.rs` (`WATCH_BODY_LIMIT_BYTES`), `src/delivery/page.rs`, `src/service/delivery_pull.rs` (`PAGE_MAX_BLOCKS`, `PAGE_MAX_TEXT_BYTES`).
- Re-offers are bounded (60 s spacing, 3 per generation) and an `Unchanged` attention-version check makes the no-news hook path one cheap round trip with no injection: `src/delivery/ledger.rs` (`REOFFER_AFTER_MS`, `MAX_OFFERS_PER_GENERATION`), `src/protocol/delivery.rs` (`DeliveryPullReply::Unchanged`).
- Hook output on quiet turns is empty: `src/cli/hook.rs`, `src/harness/agy/delivery.rs`.
- Inbox ACKs on display, so reading needs no separate ACK call: `integrations/skill/SKILL.md` ("What an ACK means").
- Agent-facing CLI output is compact: one row per item, ids not prose, `next:` continuation lines instead of dumps: `src/protocol/output_compact.rs`.

## Speed budgets

- The full test suite must stay under **5 minutes** wall-clock (`cargo nextest run --locked --all-targets --all-features`). Treat a regression past it as a bug: fix slow or serialized tests rather than raising the budget.
- An incremental build must stay under **1 minute** (edit one source file, then `cargo build` / `cargo test --no-run`), and so must the per-change lint (`cargo clippy --all-targets --all-features -- -D warnings`).
- Until the flakiness side quest (ht-zo4) lands, do not run the full suite routinely; run focused tests for what you changed.

### Actual fixture and review preflight

src/service/delivery_pull.rs::hook_pull_view validates top-level instance/livecontinuity/hold/cooperativebinding/HookPull/native_session before delivery_pull reads mod fingerprint, attention_version and offered-ledger Unchanged/whole_read. src/delivery/assemble.rs::Assembler::push currently classifies every invitation Attention; consume actual merged optionalconfirmation and preserve Lazy display completeness. Trace each upstream exclusion using actual service pull, not manuallyassembled pending sets. src/harness/agy/launch.rs::LaunchPolicy prepare_launch currently vetoes missing/stale/disabled/unknownplugin and requiresConfiguredHook; compose_argv rejects ownedargs. Preserve rewrite refusal while making readiness advisory via neutral traits. src/harness/agy/plugin.rs::{install,remove,status} owns adapterlocal plugin bridges; preserve exact-ownedevent migration across neutral cli::hook/setup contracts. tests/service/delivery_pull.rs, tests/harness/{agy_delivery,agy_launch,agy_setup,adapter_seam}.rs, tests/integration/agy_sweep.rs and tests/handoff_agy.rs are existing concrete fixture hosts; no newtarget needed bydefault. Confirm actual target registration/callerbinding/generation/completeACKpublication prerequisites and nonzero selectedtests before firstRED. Wakecompletebool is restoration, not genericsuccess. Copy actual Ainterfaces afterreview, no B-specifictypesassumed. B remains owner Ensurepolicy/automaticpayload/capability/wire; Downs underlying neutral launch/readiness+pullvisibility. C waitsboth. Serial warmvalidation only, never source/Cargo concurrently.


### Lazy invitation positive delivery acceptance clarification

Lazy invitation confirmation/nativewake suppression MUST NOT suppress in-turn pages, turn-hook/channel context or bookkeeping/discoverability (R9/R17). Preserve full display/completeness: whole(item) is not actionability. Give Lazy invitation proper informational classification while carrying it through actual delivery::Assembler and public turn-hook/service DeliveryPull output. Add positive actual Lazy invitation page/context/bookkeeping assertions AND nativewake/confirmationtimer absence under ordinary wake_on policy, with explicit wake_on policy behavior retained. RequireAck and historical None retain their timed attention. Preserve independent departed canonical sender/inviter warning visibility through ledgerUnchanged/whole_read/noticefrontier and final passiveclear; never remove rows just to suppress attention.

Task13 scoped compile-interface admission: tests/store/direct_threads.rs and tests/service/cooperative.rs existing EnsureDirectThread constructors gain the optional field; preserve historical None serialization/assertions. No unrelated behavioral scope.

### Parent-authorized bounded modern automatic completion summary

The supported envelope is the public producer-generated envelope, not every arbitrary externally authored 1MiB diagnostic report. Modern automatic canonical completion retains a DISTINCT explicitly versioned bounded completion summary. Keep the full generated launch report locally unchanged, with its exact existing bytes, digest and history. The canonical summary contains truthful required outcome/pane/seat/harness/EXACT composed argv, a fixed format marker, and optionally the full-report digest plus an explicit diagnostics-retained-locally/omitted marker. Omit optional unbounded diagnostics entirely; never clip identity/history or pretend summary bytes equal the full local report. Bound every summary field from actual canonical ID/argv constraints, and preflight reserves the exact worst supported summary, attachment, request/reply and local/recovery shapes including generated startup prompt/template. General requests remain1MiB.

Historical canonical reports remain untouched. Modern retries and completed presentation distinguish canonical summary from full local report honestly; preserve no-relaunch and canonical Completed read-only semantics, and recover/present the full local report where available. Missing-local completed presentation uses actual retained canonical representation without inventing diagnostics. Inspect and update TRUST-POLICY wording that assumes the full report is canonically retained. Fresh interface/public-route review and actual tests cover oversized post-launch diagnostics, exact full local retention, bounded versioned summary/digest, missing-local completed historical presentation, exact retries/recovery and unsupported old-peer zero effects. This parent-authorized autonomous composition refinement resolves otherwise unbounded provider/runtime diagnostic fields without a generic request-cap expansion.

### Local full diagnostic artifact and bounded critical progress

Modern automatic critical progress must not embed arbitrary full diagnostics under the existing2MiB local-progress limit. Sync the bounded critical started summary FIRST, preserving possible-start/no-relaunch even if diagnostic saving fails or crashes. Compute distinctly named full-report SHA256 over the exact generated serialization (streaming hash/write preferred); atomically install an exact private deterministic local diagnostic sidecar afterward. Optional artifact reference/digest fields remain omitted historically. Use fixed bounded pending/saved/missing markers, never arbitrary IO text in the canonical summary. Critical retries/completion/recovery must NEVER require opening/decoding that sidecar. Full bytes are preserved when successfully saved; presentation honestly distinguishes bounded canonical summary and optional full local artifact, including missing/unreadable artifacts. Artifact references/hashes are cooperative LOCAL hints, not canonical authority. No global storage/request-cap increase and no clipping identity/history.

Required actual regressions: diagnostics >2MiB saved byte-exact with correct digest; artifact IO failure and crashes around summary/artifact sync; no native relaunch; completion/recovery from summary with missing/unreadable artifact; unchanged historical bytes and presentation. Existing full-report references and canonical summary report_digest remain distinct. Parent authorized this narrow resolution after identifying ChildProgress full launch Value under MAX_LINKED_LOCAL_BYTES2MiB.

### Conditional 2MiB AutomaticBootstrap response ceiling

Parent approves narrowly scoped2MiB AutomaticBootstrap responses only IF typed canonical admission proves EVERY supported future reply including framing <=2MiB. Generic1MiB requests/default/service/watch/errors/unrelated results remain unchanged. LocalClient chooses exceptional allocation allowance from its KNOWN AutomaticBootstrap request, never untrusted reply tag, rejects over-limit length before allocation and validates typed result family plus normal correlations. Canonical Begin rechecks admission and public preflight runs before journal/pair/native effects. Historical completed read-only presentation requires no new capability/admission.

Synthetic future shapes conservatively cover every actual enforced bound and variant: IDs, creation/witness primitives and fields, generated prompt/template/native argv,4096-byte reasons, summary/artifact reference statuses, all recovery/completed/reserved responses and critical local envelopes. No undocumented unbounded provider field. Fresh wire/interface review and tests verify wrong-result/unrelated/default cap, actual near-boundary supported success and above-boundary zero effects, plus ordinary-metadata maximum escaped65536-byte body positive. This is a supported aggregate envelope, not an independent-maxima promise; derive via actual serializers, not hardcoded measurements.
