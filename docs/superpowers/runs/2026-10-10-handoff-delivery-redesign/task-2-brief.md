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

