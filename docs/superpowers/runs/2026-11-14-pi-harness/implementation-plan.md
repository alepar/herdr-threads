# Pi implementation plan

Status: approved by the user; execution started after opencode landed. Tracker: `ht-jr1g`.

## Ownership and ordering

| Bead | Owner role | Exclusive implementation scope | Entry condition |
|---|---|---|---|
| `ht-jr1g.1` | Design coordinator | Research, design, review dispositions and event contract | Active |
| `ht-jr1g.2` | Sol Rust implementer | `src/harness/pi/`, shared registry/module/unsupported-marker registration, Rust adapter/setup/launch tests and affected help/goldens | Design reviewed with user; rebase on landed opencode core |
| `ht-jr1g.3` | Sol extension implementer | `integrations/pi/`, its isolated native tests | Design reviewed with user; frozen event/environment contract |
| `ht-jr1g.4` | Sol qualification implementer, coordinator review | Public-command and real-binary integration tests, e2e/matrix dispatch, user documentation and trust amendments | Rust adapter and extension integrated |

Use one integration branch and one seeded build directory. The Rust and extension owners may work concurrently only after agreeing on payload fields, hook output, event names, environment ownership, watch lifecycle and receipt predicate. Shared-file changes have one owner. A task may not claim passing integration tests against a temporary stub or stale counterpart; report such a boundary explicitly.

The coordinator designs, assigns, reviews and integrates. Sol writes production code and fixes. Scope expansions or new neutral APIs return to the coordinator with a concrete need; do not add a pi branch to neutral core. Opencode's channel-only wake rules are an external integration dependency; do not duplicate them. Known herdr-core launch composition gaps stay with their current owners.

## Contract handoff

Before code starts, the task briefs carry the reviewed design revision and exact agreed fields for lifecycle, context and watch ownership. They identify the permitted branch/files, expected public behavior, tests, acceptance evidence and dependencies. The extension owns pi API behavior; Rust owns canonical policy declaration and decoding. Neither side infers receipt evidence from the other side's success code.

The event contract must explicitly distinguish admitted TUI lifecycle, ordinary turn/context opportunities, compact re-entry, teardown, excluded modes/children and failed check-in. It must specify session and process witnesses and the generation fence. Watch protocol framing, truncation, ACK semantics and daemon wake eligibility are reused, not independently reinvented.

## Required task rules

- Work only on the pi branch/worktree; no main/foreign branch commits, push or stash.
- Run builds/tests through the coordinator-provided heavy/toolbox wrapper. Keep one seeded target directory; no build trees under temporary RAM storage.
- Never link/unlink the shared plugin or restart/stop shared Herdr or the real threads daemon.
- Never set up or install against real harness configuration. Use isolated HOME, pi agent root, state and owned test hosts.
- Never read or use the existing OpenRouter credential without separate coordinator authorization. If authorized, keep it only in a private scratch file, never argv, logs, traces or commits; remove it afterwards.
- Test processes use the repository's owned-process helpers and scratch ledger. Stop/reap everything created and run scoped leak checks for integrated/full runs.
- Read TRUST-POLICY before identity, receipt, launch or wake changes. Preserve cooperative claims and conservative continuity; no adversarial verification.
- Every commit carries `Outside-adapter:` listing changed paths outside `src/harness/agy/`, per repository accounting. A pi change therefore lists its pi and shared paths.
- New model-visible strings state their size and pass the token-efficient framing rules. Prefer existing frames/markers and quiet no-news callbacks.

## Verification sequence

First verify deterministic adapter and extension behavior: malformed/stale identity, explicit mode, session transitions, ownership fences, bounded delivery/ACK state, priority wake admission, busy/idle boundaries, setup manifests and interactive argument grammar. Each regression test must exercise a real causal boundary, especially void-return ACK refusal, stale-busy passive wake prevention, and canonical replay of discarded lower-ranked overflow after old-watch re-offers are exhausted and capacity is released.

Next run public setup/status/unsetup and launch/handoff against an isolated real pi with a scripted provider. Exercise draft/modal input, tool-running delivery, retries, compaction, resume/fork/reload, channel reconnect/replacement, truncation and failure recovery. Assert that an extension-started idle wake preserves the session’s currently selected provider/model and any applicable agent selection; do not assume the wake API retains these. This qualification requirement follows the opencode integration review and must be checked against pi itself. SDK-only tests are useful but cannot qualify terminal behavior. Test both documented executable distributions when available, or report the exact unqualified shape.

Run focused Rust tests and required `cargo fmt`, all-target/all-feature clippy, harness-neutral and wire compatibility checks. Before integration handoff also run default-feature checks. Preserve build/lint/suite time budgets.

Finally run the integrated tree's unfiltered full nextest suite with a unique leak-run ID, then scoped process/temp cleanup checks. Run the live wake matrix only after credential authorization, through the real terminal extension and the public setup/launch paths; record missing coverage honestly. A minor release is the main coordinator's subsequent responsibility, including release-specific gates.

A review-ready report names the branch/commits, independent super-roast verdict and open findings, exact checks and evidence paths, live e2e result or remaining permission gap, and cleanup status. Main reviews and merges. After confirmed merge, remove owned target/worktree/branch resources and report cleanup.

## Task 1: Native extension (ht-jr1g.3)

Read design.md D1-D5 and the research boundary probe results. Implement only `integrations/pi/` (extension, focused tests, integration README). Freeze the Rust/extension interface in event-contract.md; report any necessary correction before deviating. Use a dependency-free JavaScript extension entrypoint `integrations/pi/extension/herdr-threads.js`, with supporting focused modules if needed; verify the native loader shape from the recorded source. Setup will install the owned extension directory. Export one default registration function; tests may reach explicit internal exports if pi's loader allows them.

Implement the approved owner/session/role/mode guards, lifecycle guidance, bounded priority buffer and every-overflow replay debt, native boundary proposals, exact context-and-current-branch receipt predicate, completed-only continuation, idle wake fencing/timeouts, and watch/process cleanup. Reuse the actual watch wire and frame semantics, including nonordinary presentation/control confirmation if elected; never confuse presentation confirmation with mail ACK. No native queued message bodies. No credential access in this task. Preserve selected model/provider on self-wake and report real-TUI uncertainty honestly.

Test causal behavior with the real extension callbacks and controlled IO, then the recorded real-SDK scripted provider where feasible. Include overflow with exhausted reoffers, lifecycle late callbacks, foreign parent markers, error/abort guards, rejected or altered draft evidence, passive idle silence, and failed ACK/replay. TDD is required for new state-machine behavior. Run tests through the heavy/toolbox wrapper; report RED/GREEN commands and output. Do not run the full Rust suite for this isolated integration task. Commit only owned paths, with required trailer; do not create subagents. Report to the coordinator before any shared code change.

## Task 2: Rust adapter and public integration (ht-jr1g.2)

Implement `src/harness/pi/` and the minimal neutral output-policy seam, registry/unsupported-marker registration, process identity, setup/doctor, hook decode, launch parser and relevant Rust tests/goldens. Consume Task 1's real extension assets and event-contract.md, not a stub. Amend TRUST-POLICY.md in this activating commit for D7. Preserve default behavior for every other adapter. No pi-name conditional in neutral core. Scope existing files only where registration or the neutral policy requires it; leave shared herdr-core launch gaps to their owners. TDD for behavioral changes and required per-change checks; task review follows.

## Task 3: Integrated qualification and documentation (ht-jr1g.4)

Use the integrated public setup/launch/watch paths with isolated real pi and controlled provider tests, then the authorized live OpenRouter/DeepSeek credential under the coordinator's private brief. Qualify TUI drafts/modals, busy boundaries, lifecycle, receipts, priority/threshold wake, no-channel behavior, model continuity and cleanup. Add pi dispatch to wake e2e/Herdr matrix and user docs as appropriate; preserve existing harness qualification. Run required checks and one unfiltered full suite with scoped leak checks on the integrated tree. Any unsupported runtime capability must degrade only that capability with evidence and documented limits; do not silently declare a test bypass as support.
