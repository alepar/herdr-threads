## Task 7: Due discovery and originating-seat warning lifecycle

filesTouched: ["src/store/receipts.rs", "src/store/poke.rs", "src/store/wake.rs", "src/store/queries.rs", "src/store/schema.rs", "src/store/mod.rs", "src/scheduler/deadlines.rs", "src/daemon/lifecycle.rs", "tests/store/receipts.rs", "tests/store/poke.rs", "tests/store/wake.rs", "tests/store/wake_discovery_cost.rs", "tests/scheduler/deadlines.rs", "tests/scheduler/poke_flow.rs", "TRUST-POLICY.md"]

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

Bound definitions: start at a fresh paired discovery cycle with the target obligations already overdue, or restart with reconstructed finite cursors. H is an upper bound on ALL actually inspected candidates in the captured historical physical/prepared repair workload, including future/terminal/waived/unpublished candidates; it is not the number changed. D bounds ALL inspected ordinary receipt candidates in the complete fresh post-repair snapshot, including repaired rows, duplicate physical/sparse projections and finite arrivals captured before that snapshot freezes. I bounds ALL inspected invitation candidates in its initial finite snapshot, including future or subsequently ineffective rows. Q bounds elapsed time of one successful runnable GLOBAL driver quantum, including storage, scheduling and other admitted work. Later insertions outside captured high waters cannot enter or restart those walks, even when their deadlines are earlier. A finite D bound must cover any arrivals before the post-repair snapshot freezes; unbounded arrivals during repair do not justify using only the initial receipt count.

When H=0, empty repair bookkeeping completes within the first ordinary turn without reducing its capacity. With D and I at most N, the original two-class bound remains 5,000ms + (2*ceil(N/100)+2)*Q. For H>0, set R=ceil(H/50)+2: the extra two receipt turns conservatively cover finite-stream/end probes and repair completion. After those R receipt turns, at most ceil(D/100)+1 further receipt turns finish the fresh post-repair snapshot; the +1 covers a full final page's empty completion. Invitations need at most ceil(I/100)+1 invitation turns. Fair alternation therefore gives the conservative GLOBAL bound:

`5,000ms + 2*max(R + ceil(D/100)+1, ceil(I/100)+1)*Q` for H>0.

The bound concerns durable warning detection for still-pending, unextended target obligations, not bounded native key submission. It is conditional on successful bounded storage/driver service, the stated finite workload/snapshot limits and Q; it does not cover outages, repeated errors, unbounded overload, newly expiring rows already passed by a snapshot cursor, or arbitrary arrivals outside the counted snapshots. Such rows belong to a subsequent fresh cycle. Restart resumes or reconstructs finite traversal without losing repair progress; a newly reconstructed traversal must count its inspected workload again. Tests must measure actual inspected work and global quanta, rather than changed rows or receipt-only calls.

Task5 verifies the producer locally: multiple batches, finite captured high waters, late earlier-deadline arrivals excluded from repair, mixed future/terminal/waived/unpublished candidates, all-terminal old backlog, one-shot atomic completion/reset, and rollback/restart. Task7 verifies the global conversion with at least 250 candidates in EACH receipt and invitation class, both H=0 and H>0, blocked native wakes, mixed future/due rows, continuing earlier-deadline arrivals, actual <=100 total inspections per quantum, immediate successful continuation, early ordinary-due progress before long repair exhaustion, final post-repair discovery and restart. Task7 owns normal due-query high-water enforcement/full allowance lending and shared fairness; Task5 does not claim the combined bound from receipt-only tests.


### Authorized coalesced multi-originator warning acceptance refinement

Receipt-overdue warning conditions remain coalesced by their existing thread/recipient backlog identity. Preserve exactly one canonical open event for that condition, exact physical/manifest receipt-to-warning associations, and the existing final clear event. Never manufacture per-sender warning conditions/events or derive original senders from current thread membership, the warning event's first source message alone, or a later caller. Enumerate canonical original seat authors of ALL messages exactly associated with the coalesced warning via receipts/receipt_state.warning_message_id; retain existing service-author handling without inventing a seat.

For EACH canonical original sender of an exactly associated receipt, the still-open shared condition must participate in actual native waking-attention selection and mod/watch pending-set/fingerprint paths under existing admission guards, including a sender who left the thread. A successful test of notice-row insertion or inbox history alone is insufficient. A later receipt association from another original sender must become actionable even when it reuses an older open event and an informational notice-offer frontier has already passed that event's sequence. Do not conflate informational offer settlement with the canonical open-condition/associated-origin state. Preserve the existing per-occupant wake-offer deduplication and safe composer/host/harness admission: this refinement does not authorize repeated native submission for an unchanged already-offered actionable state, new receipt provenance, or a task-adoption/completion claim. Cross-path attention versions/fingerprints must represent newly actionable associations honestly rather than inventing a new canonical warning event.

Existing partial settlement semantics are verified in schema::clear_warning_conditions_for_receipts: the condition stays open while ANY exact associated receipt remains required/pending and not human-waived. ACK/waiver/terminal settlement of a subset must preserve all remaining origins' obligations and waking eligibility. Do not emit a condition clear, withdraw another origin's pending warning, or rewrite settled receipt/ACK provenance merely because the first event's source receipt settled. Historical informational delivery remains historical. Only settlement of the final required/pending/unwaived associated receipt clears the shared condition, once, through its existing passive-clear lifecycle; clears/observer notices create no waking attention. Routing preserves the backlog-wide open-condition lifetime; this refinement does not mandate quieting an individual sender after only its own receipts settle. All canonical associated original senders remain covered while the shared condition is open.

Task7 fixture/review requirements: construct at least TWO canonical original senders and THREE exact associated receipts sharing the same thread/recipient condition (for example A1,A2 from sender A and B1 from sender B), with at least one original sender leaving before warning routing. Exercise physical and published-manifest associations, mixed original authors and repeat scans. Prove exactly one open condition/event and all exact associations. Advance informational notice-offer frontiers, then coalesce a receipt from the other original sender; show actual native selection and mod/watch state/fingerprint expose the new origin under normal guards. Partially ACK A1 and prove A2/B1 origins remain actionable; settle A2 (the event's first origin now has no pending associated receipt) and prove B1 remains actionable without premature clear or extra open event. Finally settle B1 and prove one passive clear, no warning-only native/mod wake, unchanged exact ACK actor/generation/proof/history, and no synthetic receipt/adoption evidence. Per-occupant offer deduplication and blocking admission still apply independently to each origin. Existing H/D/I upgrade discovery bound refinement remains normative and unchanged.
