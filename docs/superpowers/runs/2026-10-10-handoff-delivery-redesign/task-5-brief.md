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


### Settled receipt producer and validation seams

Read fixture-preflight.md and execution-seams.md before writing fixtures. Task1 final source is receipt migration0030, following immutable invitation0029. Read task-1-report.md, task-1-seamfix-report.md and task-1-seam-review.md. receipt_send_time_cutovers(instance_id,historical_decision_seq) is already captured transactionally once by receipt constraint upgrade, before capability advertisement; future instances imply cutoff0. Do not recapture/replace marker or rewrite any previous migration. Stored deadlines win; old sparse running reconstruction uses first original canonical availability after send and before retirement bounded by cutoff, never current availability or later registration. Old terminal ACK with absent times stays absent after late registration/restart, with exact actor/generation/proof/history.

Task5 owns bounded physical dormant backfill/index consistency only, preserves Task1 logical projection and immutable journal/request/result bytes. Task7 owns shared due-driver continuation/fairness; Task6 supplies invitation cursor. Do not invent a reverse dependency: test your own multiple100-row finite backlog through the owned backfill and existing discovery interfaces, and export producer seam toTask7. Any schema addition is a new migration after latest integration version, not modification of receipt0030. Hot-file cap3 allows schema.rs Task4/Task6/Task5; keep declared surfaces exact, request scope expansion before material extras.

Tests must use actual old migrations/version cutpoint and populated physical/manifest rows, terminal/waived/running cases, restart/replay and indexed discovery. Follow fixture-preflight.md for canonical claims/FK graph, version/count guards, finite drain helpers that fail stored work errors rather than looping. Keep source frozen during builds. One heavy Cargo lane requires explicit grant; CARGO_TARGET_DIR must be the exact existing absolute taskworktree/target supplied by coordinator, never a newly invented /tmp cache. Parent authorized CARGO_BUILD_JOBS=4, nice, one Cargo invocation at a time; no profile or budget changes. No full suite/default-feature/final readiness (parent owns later). Stop owned/tagged children and report exact timing/diagnostics/zero-process state.
