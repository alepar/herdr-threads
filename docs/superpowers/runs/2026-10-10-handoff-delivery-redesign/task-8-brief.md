## Task 8: Invitation CLI and presentation contract

filesTouched: ["src/cli/commands.rs", "src/cli/mod.rs", "src/cli/threads.rs", "src/cli/output.rs", "src/cli/retry.rs", "integrations/skill/SKILL.md", "README.md", "tests/cli/commands.rs", "tests/cli/output.rs", "tests/cli/skill.rs", "tests/cli/guidance.rs", "src/cli/watch.rs", "src/cli/hook_inbox.rs", "tests/cli/watch.rs"]

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


### Settled Task6 metadata, actual client consumers and fixture preflight

Read fixture-preflight.md, execution-seams.md, root/invitations normative specs, task-2 report, task-6-report.md and the coordinator's fresh Task6 fix rereview/seam evidence before implementation. Task6 original round2 rereview and fresh integration seam review are CLEAN; focused43 checks and exact-tip lint/fmt passed, and changes integrated as54796f2d before parent-authorized main compositionbfe1044e. Dispatch begins only after its approved changes integrate serially. Use that actual integration tip and actual schema catalog; provisional migration numbers in task reports are not an instruction to rewrite or duplicate migrations. Parent owns final PR roast, default-feature/final expensive verification, report and finish.

Task6 actual exported producer contract: `protocol::commands::InvitationConfirmation::{Lazy, RequireAck}`; `Invite.confirmation: Option<InvitationConfirmation>` with absent historical field preserved, existing deadline_millis, and the same optional field in saved SemanticMutation::Invite. Capability constant `INVITATION_CONFIRMATION_V1` is `invitation.confirmation_v1`; Task6 advertises it only together with active lifecycle. Both `InboxBatchItem::Invitation` and `InboxBatchV2Item::Invitation` now expose `confirmation: Option<commands::InvitationConfirmation>` and `deadline_at: Option<UtcMillis>`, serde default and omit None. Historical policy-absent rows omit BOTH fields exactly and retain legacy timed interpretation. Modern Lazy rows have Some(Lazy), no effective deadline; modern RequireAck rows carry Some(RequireAck) and the original frozen deadline. Do not infer Lazy merely from absent deadline, as historical omission must stay legacy timed. The bounded query producers read immutable invitation columns by invitation ID; Task6's V2→V1 compact conversion already preserves metadata. Producer support does not prove downstream client defaults/actionability are correct.

Evidence to consume, not rerun ceremonially: Task6 report records the real combined `invitation_confirmation_inbox_metadata_distinguishes_lazy_timed_and_legacy` regression passing for BOTH producer versions, historical omitted fields/modern Lazy deadline absence/RequireAck frozen deadline; fix-round metadata rerun passed at its schema31 source. It records existing protocol output compatibility passing. Its subsequent fix proves actual native/mod original-inviter open warning persists after committed informational offer and removes independently on acceptance, with passive clear. Fresh rereview/integration status belongs to coordinator; this brief makes no Task6-wide readiness claim. Earlier build budget overruns and final corrected30.20s clippy are documented, not erased.

Concrete deferred consumer: current `src/cli/watch.rs` near550 matches every InboxBatchV2 invitation and unconditionally sets `saw_attention=true`. Use confirmation metadata so modern Lazy remains readable/discoverable but does not create timed/mod attention, while Some(RequireAck) and historical None preserve ordinary attention. Parent explicitly admits `tests/cli/watch.rs` for an actual watch-path regression beyond the mapping's existing watch/hook_inbox source paths. Verify the real continuation/drain/output and fingerprint/attention path, not only a duplicate helper.

Correction to earlier Task6 source-location note: `src/cli/hook_inbox.rs` near105 is `whole(item)`, a DISPLAY COMPLETENESS predicate, not an actionability predicate. Invitation rows (including modern Lazy) are fully displayable; keep this true to preserve hook/inbox discoverability. Trace the actual standard-hook trigger/attention source before proposing any separate change. Passive Lazy inclusion at an ordinary hook opportunity is allowed; it is not native wake, registration, acceptance, receipt or task adoption. Do not suppress Lazy rows or withhold an acceptance-ready command merely to remove attention. Existing ACK-required message display and lazy-message presentation/completion rules remain separate and unchanged. If another actual write path is required, report it for coordinator scope admission; protocol/output_compact.rs or protocol output tests are NOT pre-admitted speculatively.

Modern ordinary invite CLI explicitly emits Lazy by default (and for --lazy). --require-ack freezes default invitation duration; --deadline implies --require-ack; --lazy conflicts with either confirmation request. Validate duration/conflicts and negotiate capability BEFORE new durable journal/daemon effects. Historical saved omitted fields retain exact legacy request bytes, operation keys, digest and interpretation; retry never silently upgrades or downgrades pending invitation policy. Required service and explicit human/operator commands retain their established semantics. Duplicate ordinary invite retains original invitation/policy/deadline/origin; already joined remains AlreadyJoined. Registration alone never settles acceptance; explicit accept settles automatically, no separate invitation ACK command. User-facing compact/machine/JSON/inbox/help/skill output must describe the policy accurately without inventing confirmation evidence or claiming task adoption. Existing metadata/carry-through may suffice for rendering; inspect real output paths before expanding scope.

Fixture preflight before the first expensive build: trace actual parser→semantic freeze→journal→capability→canonical mutation path and seed complete seat/claim/binding/membership relationships. Test actual isolated CLI for bare lazy, explicit lazy, require-ack default duration, deadline implication, conflicts/invalid duration, capability absent/refused BEFORE durable effects, duplicate frozen origin/policy and historical exact-byte replay. Prove Lazy display remains recoverable in inbox/hook and watch does not treat it as attention; RequireAck and legacy None still do. A setup refusal before the intended cutpoint is not evidence. Reply-loss fixtures must hit a real retained failure boundary rather than a path that self-recovers in the same call. Preserve complete IDs, continuations, source/actor attribution and all independent message-receipt obligations.

Keep declared surfaces exact; tests/cli/watch.rs is the explicit additional admitted regression surface. Request actual additional paths before writes, not hypothetical blanket scope. No store/lifecycle/scanner redesign is needed in this consumer leaf. Use isolated HOME/CLAUDE_CONFIG_DIR/CODEX_HOME and tagged test-support children/private sessions; stop all owned processes. Source stays frozen during assigned Cargo lane; target directory/jobs/one-build scheduling come from coordinator, with nice and no profile/cache/budget changes. Run relevant focused tests, cargo fmt and all-target/all-feature clippy; report exact evidence, timing regressions and any true blocker. No routine full suite, final default-feature check, worktree/status/dependency mutation or implementation dispatch is authorized by this brief-preparation step.

### Parent status correction — Task6 actual native discovery cleared, serial integration pending

Task6 round1 fresh rereview returned NOT ADDRESSED I1: the actual `discover_wake_candidates` upstream `seat_has_pending_rows` indexed pending-row probe omitted the independent open-origin source, even though direct attention/reserve and mod helper checks had passed. The earlier implementer report's ADDRESSED claim is therefore NOT full native discovery clearance. Round2 actual discovery RED preceded the bounded seventh indexed-probe fix;16 focused checks passed and fresh task-6-rereview-2.md is Spec CLEAN / Quality CLEAN / I1 ADDRESSED. The rebased3f9167bd candidate also has fresh task-6-seam-review.md Spec CLEAN / Quality CLEAN; focused43 checks and exact-tip lint/fmt passed, and serial integration completed as54796f2d. Do not treat prior source/helper GREEN as permission to dispatch or as proof all native paths work. Task8 starts only after Task6's new fresh review approves the actual discovery fix AND its changes integrate serially. Task6 retains invitation-origin discovery responsibility; Task8 only consumes its landed metadata/lifecycle and owns CLI/presentation/watch behavior. No downstream source responsibility is transferred by this status correction.
