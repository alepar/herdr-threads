## Task 10: Handoff confirmation policy wiring and recovery documentation

filesTouched: ["src/cli/commands.rs", "src/cli/handoff.rs", "src/cli/handoff_delivery.rs", "src/cli/topology_handoff.rs", "src/cli/mod.rs", "src/cli/launch.rs", "src/harness/launch.rs", "src/protocol/handoff.rs", "src/store/handoff.rs", "src/store/topology_handoff.rs", "src/store/topology_handoff/attachment.rs", "src/store/topology_handoff/runtime.rs", "tests/cli/handoff.rs", "tests/cli/handoff_delivery.rs", "tests/cli/topology_handoff.rs", "tests/cli/launch.rs", "tests/harness/launch.rs", "tests/handoff_topology_cli.rs", "tests/handoff_topology_authority.rs", "integrations/skill/SKILL.md", "README.md"]

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


### Settled producer contracts and fixture preflight

Brief preparation is not dispatch authorization. Read fixture-preflight.md, execution-seams.md, root/handoff/receipts/invitations normative specifications, TRUST-POLICY.md, Task1/4/6 reports and fresh producer integration reviews. Use the actual merged branch and schema catalog. Task7 shared driver/original-sender warning work is not landed at preparation: consume its actual merged implementation/report later; this brief assumes no new Task7 API signature and claims no unregistered-recipient warning GREEN. Task9 setup work is likewise consumed from its actual integrated tip; do not copy unfinished candidate source or infer approval from this brief.

Task4's acquired-body seam is already active: HandoffArgs has body: Option<String>, file: Option<String>, stdin: bool, with one exclusive required instruction_source; input::read_body acquires once before constructing body: String. HandoffRequest, topology_handoff::Request and protocol::handoff::HandoffPayload contain acquired body: String, not a file/stdin handle. Relevant existing entry signatures:

- cli::handoff::start<C: LocalClient + ?Sized, W: Write>(journal: &Journal, request: HandoffRequest, claim: CallerClaim, display: &str, client: &C, launcher: &mut dyn HandoffLauncher, clock: &dyn Clock, output: &OutputSpec, writer: &mut W) -> Result<(), RunError>.
- cli::topology_handoff::prepare<C: LocalClient + ?Sized>(request: &Request, inputs: Preparation<'_, C>) -> Result<protocol::handoff::BootstrapIdentity, RunError>.
- cli::handoff_delivery::prepare<C: LocalClient + ?Sized>(request: &Request, inputs: Preparation<'_, C>) -> Result<DeliveryPlan, RunError>.
- cli::require_send_time_receipts_capability<C: LocalClient + ?Sized>(client: &C, clock: &dyn Clock) -> Result<(), RunError>.
- harness::launch::prepare_managed_with_registry(registry: &Registry, host: &dyn HostPort, seats: &dyn LaunchSeatResolver, hooks: &dyn LaunchHookInspector, clock: &dyn Clock, request: ManagedLaunchRequest, caller_budget: &CallBudget, native_argv: Option<Vec<String>>) -> Result<PreparedLaunch, ApiError>.

Preserve Task4 actual serializer ceilings and all pre-effect checks, including its later archival-loader and legacy-origin overflow fixes. The report inventories bootstrap identity/progress512KiB, recovery1MiB, derived bootstrap origin/terminal formulas, delivery origin512KiB/terminal1MiB, unchanged completed/linked2MiB, metadata-only128KiB and globalwire1MiB. Maximum six-byte-escaped65,536-byte bodies and nested original JSON remain supported without clipping; do not revert to provisional design assumptions about envelope size. Body source validation preserves NUL, whitespace and newlines. No source reread or native option/environment reread on retry.

Task1 accepted compatibility limit is normative: ordinary pending/commit-unknown retry without locally retained completion conservatively refuses on an unsupported daemon, even if the original may have committed. Public ordinary OperationStatus cannot supply verified operation scope; do not invent a successful public status fixture or new inspection protocol. Completed retained compound/service outcomes remain inspectable without new sends. Canonical audience cannot be established wholly human locally, so potentially ACK-required ordinary sends conservatively require messages.send_time_receipts_v1; human sender is not recipient waiver. Require that capability and invitation.confirmation_v1 before NEW policy-bearing durable/native effects, using actual client discovery. Preserve historical omitted policy bytes/keys/digests and child requests; completed recovery is not a new mutation.

Task6's actual invitation producer is Invite.confirmation: Option<InvitationConfirmation> with Lazy/RequireAck and existing deadline_millis; None means historical timed semantics. Modern handoff freezes Lazy explicitly unless --require-invite-ack or --invite-deadline requests timed acceptance. Its body is ALWAYS an ACK-required addressed ordinary message, with independently frozen --deadline/default receipt duration. Never let Lazy invitation imply Lazy body, or acceptance imply message ACK. Task6 intentionally installed the linked invitation confirmation-is-None guard in store/topology_handoff/attachment.rs. Replace that temporary guard with exact comparison against the frozen canonical handoff policy; do not merely allow Some. Test both modern modes through actual public linked-phase validation: matching policy succeeds, altered confirmation/duration/child key fails before total_changes/native effects, and absent historical policy replays unchanged.

Task9 keeps HookInspection.configured_hook: Option<ConfiguredHook> and the existing full-configuration fingerprint semantics unchanged. HookInspection additionally reports legacy_event_registration for diagnostics; registration_conflict is a doctor/report diagnostic, not a HookInspection field or launch-admission signal. Preserve exact owned configuration fingerprint recheck WHEN that configuration is actually used. Missing/untrusted/stale/wrong-profile hook/setup observations are diagnostic-only; absence cannot veto or fabricate a fingerprint. This change does not waive fresh empty-shell/occupant/seat/caller/continuity/native argv/host-kind/environment-override grammar guards or any actually used configuration TOCTOU check. Native harness trust refusal remains truthful, with staged durable work retained; never auto-answer approval, accept an invitation, ACK a message or claim adoption/registration from launch.

Before the first expensive build, trace each fixture through actual canonical caller/membership/seat/binding/preparation prerequisites and exact failure cutpoint. Earliest meaningful policy fixtures should extend existing actual source/retry paths: maximum_instruction_file_freezes_through_explicit_launch_retry, maximum_instruction_file_freezes_through_existing_delivery_and_retry, maximum_instruction_file_freezes_through_bootstrap_completion_and_retry, handoff_exact_keys_replay_lost_responses_each_durable_phase and handoff_possible_start_crash_and_unknown_never_start_twice. Assert actual frozen child requests for the three path variants with independent message/invitation durations, lazy invitation plus ACK-required body, and historical omission. Delete/mutate files and change native options after staging; retry must retain exact original bytes/policy/keys and never restage completed children or relaunch possible-start/unknown-create.

Use isolated actual CLI/canonical fixtures for missing/stale/wrong-profile hooks and native trust refusal, retaining structural positive/negative controls. Prove otherwise-safe launch/staging proceeds with diagnostic hook absence; prove occupied/human/held/stale target, unsupported environment overrides/host grammar, and changed actually-used owned config still refuse. After Task7 lands, use its actual merged due path/fake clock to observe durable sender warning for an unregistered launch recipient without registration/acceptance; late accountable receipt clears normally. Preserve guarded native wake separately from durable detection. Native argv includes only fixed inbox instructions, exact instance UUID/canonical state directory/canonical host endpoint and pinned fallback; mismatched/null/ambiguous hook routing cannot supersede those three expected fields. Body/path never enter native argv.

### Verification ownership and completion boundary

Task10 runs relevant focused handoff/topology/delivery/launch/parser/retry tests, cargo fmt and nice cargo clippy --locked --all-targets --all-features -- -D warnings, with source frozen during builds and actual selected/passed evidence. Reuse declared test surfaces; report any actual extra write path before scope expansion. Use test-support-owned children/private Herdr sessions and isolated HOME/config; stop all owned processes. No routine full suite while ht-zo4 remains outstanding. Parent alone owns shared integration/Task11 sweep, adversarial PR roast, final nice scripts/check-default-features and any required expensive branch checks AFTER that roast at the final cleared SHA, scoped leak checks after full-suite/sweep runs, final readiness report and main-tab notification. Leaf completion and this prepared brief do not authorize dispatch, merge or external notification.

Embedded guide preflight: integrations/skill/SKILL.md has a tested strict <200-line limit (currently198). Keep new handoff/readiness/policy guidance concise within existing paragraphs; do not raise the bound or drop required content. Task8 caught and corrected a200-line regression. Check the actual embedded skill contract and line count before expensive builds.
