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

