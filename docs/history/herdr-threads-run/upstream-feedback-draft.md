<!-- upstream-feedback draft — NOT FILED: addressed upstream in v6.4.2-alepar4.3 (5c92bce), per the maintainer reply relayed by the human on 2026-10-01:
  D1 cluster loss: fixed (step-back clusters: lines; scope filter decides clusters as a unit; scripts/scope-dispositions).
  D2 zero-raw late round: fixed (emptyLateRound → [converged], no [low coverage]).
  D3 report writer: fixed (coordinator writes and commits report.md).
  D4 exit 126: not a defect at current version (stale hand-copied installs; Codex now installs the fork as a plugin; guarded by a test).
  Doc gap mid-run switch/ledger: clarified (ledger continues; post-switch lanes must go through the super-code Workflow; hand-run lanes are out of contract).
  Design Q A (detector persistence): already persisted as Detector: ledger lines; this run lacked them because post-switch lanes ran by hand.
  Design Q B (coverage input size): open with the owner.
  Also in 4.3, from round-2 friction: bounded phase-6 sweep-fix pass (sweepFix:), and lanes run the tests of callers of changed shared helpers. -->
# 2026-09-26-herdr-native-mailbox-thread-plugin: code-roast step-back "sweep the cluster" decision is dropped by the scope filter, and the pattern recurs next round

Plugin: final phases on superpowers skills @ e8244b0 (6.4.2-alepar4.0); earlier phases on plugin cache 6.3.0-alepar3.8, then local b5bf5d6 (3.9), 44f9868, 3d369ed, 46cc007 (3.10). Run: 126 beads (81 task / 19 bug / 13 feature / 13 epic), longest blocks-chain 14, 214 blocks edges, 3 design roasts + 2 PR roasts, 1,064 commits, 2026-09-26 → 2026-10-01 (~4.3 days wall, with pauses; ran on Codex/GPT until 2026-09-29, then on Claude Code/Opus).

## Defects

### 1. The step-back's `patch` decision ("sweep each cluster in one pass") never reaches the scope filter, so cluster members go to the punch list one by one and the pattern comes back as a new finding next round.
- **Evidence:**
  - `roast-pr-1-step-back.md`: "decision: patch … fix each cluster with one consistent sweep … apply the rule to every list/discovery query in one pass, not only the three cited lines, so the same issue does not reappear elsewhere next round."
  - scopeFilter-round-1: 8 of the 10 findings the step-back named across clusters (a)/(b)/(c) went to the punch list (store/mod.rs:838, mod.rs:689, seats.rs:702, workers.rs:599, workers.rs:625, reconcile.rs:885, workers.rs:857, transport.rs:67). Only hook.rs:716 and app.rs:446 were in scope. Final line: "scope-filter: 4 in-scope · 20 punch-listed".
  - Round 2 confirmed a new cluster-(a) finding, seats.rs:1217, "for consistency with iteration 1, which rated the analogous linear-growth-with-lifetime-history findings (store/mod.rs:838, store/mod.rs:689, seats.rs:702) Should-fix".
  - `scope-filter-prompt.md` receives only `{{CONFIRMED_FINDINGS}}` (findings the step-back did not dissolve). It gets no cluster or sweep directive.
- **Premise to verify:** a `patch` step-back is meant to shape fix scope. If the scope filter is the sole scope authority, the step-back should stop issuing sweep instructions.
- **Suggested fix shape:** feed the step-back's `pattern:` clusters into the scope filter and decide each cluster as a unit, or require a recorded override when the filter splits a cluster.

### 2. A late-round roast whose scouts all return nothing gets `[low coverage]`, which rules out `[converged]`, even though the late-round stance calls "no material findings" an expected outcome.
- **Evidence:**
  - roast-design-3: "clean (0 nits) [low coverage]", "9/9 scouts returned; 0 dead … 0 raw".
  - `run.md`: "qualifier required by reporter rule … no convergence claim". The result was parked as a degraded verdict.
  - `friction.md`: "these contracts can prevent convergence after an otherwise complete empty late-round review".
  - Still present in the current checkout: `reporter-prompt.md:174` "or when `coverage.rawFindings` is 0 and you judge the artifact non-trivial", and `[converged]` requires "you did not add `[low coverage]` yourself".
- **Premise to verify:** zero raw findings with full scout liveness in round ≥ 2 should count as convergence evidence, not as a blind-scout signal.
- **Suggested fix shape:** exempt rounds ≥ 2 that have a prior report and full scout liveness from the zero-raw rule, or use a non-blocking qualifier.

### 3. Phase 6 does not say who persists `report.md`; on Claude Code a delegated report writer cannot write it.
- **Evidence:**
  - The coordinator observed the report-writing subagent's Write of report.md blocked by the harness ("subagents should return findings as text"); the coordinator wrote the file.
  - The super-auto phase-6 row says only "write `report.md` per `./report-prompt.md`", while `report-prompt.md:155-156` anticipates "the agent writing `report.md` may not be the agent … that ran phase".
- **Premise to verify:** delegating report authorship is intended.
- **Suggested fix shape:** state that the report writer returns the body as text and the coordinator writes and commits it.

### 4. A nested helper invoked by path fails with exit 126 when the installed copy lost its exec bit (twice, under a Codex skill install).
- **Evidence:**
  - `friction.md`: "installed scripts/sdd-workspace lacks executable permission; direct invocation returned exit 126".
  - `friction.md`: "Default task-brief exit126: nested scripts/sdd-workspace is non-executable".
  - The file is 100755 in git @e8244b0.
- **Premise to verify:** some install paths drop file modes (not verified here).
- **Suggested fix shape:** call the helpers as `bash <script>` from other helpers and from skill text, or check the exec bit at pre-flight.

## Run metrics
### Judge panel
- design · iteration 1 of 3 · independence: same-family (GPT) — seat-differentiated panel · seat-agreement: panels 17 · rr 0.88 · rg 0.82 · fg 0.82 · unanimous 0.76 · ground-loo 0.87 (n=15) · reproduce 13/4/0 · refute 13/4/0 · ground 16/1/0
- design · iteration 2 of 3 · independence: same-family (GPT) — seat-differentiated panel · seat-agreement: panels 1 · rr 1.00 · rg 1.00 · fg 1.00 · unanimous 1.00 · ground-loo 1.00 (n=1) · reproduce 1/0/0 · refute 1/0/0 · ground 1/0/0
- design · iteration 3 of 3 · independence: same-family (GPT) — fresh scouts; no judge panel dispatched because no candidates · seat-agreement: none (no panel)
- PR · iteration 1 of 3 · independence: same-family (claude) — seat-differentiated panel · seat-agreement: panels 45 · rr 0.82 · rg 0.84 · fg 0.76 · unanimous 0.71 · ground-loo 0.86 (n=37) · reproduce 24/21/0 · refute 20/25/0 · ground 29/16/0
- PR · iteration 2 of 3 · independence: same-family (claude) — seat-differentiated panel · seat-agreement: panels 3 · rr 1.00 · rg 1.00 · fg 1.00 · unanimous 1.00 · ground-loo 1.00 (n=3) · reproduce 3/0/0 · refute 3/0/0 · ground 3/0/0
- delta lines:
  - design-2: "1 new confirmed (0 Blocking) · 0 carried · 13 resolved · 0 regressed"
  - design-3: "0 new · 0 carried · 1 resolved · 0 regressed"
  - PR-2: "3 new confirmed (0 Blocking) · 20 carried (0 Blocking) · 4 resolved · 0 regressed (0 Blocking)"
### Fix loop
none (the ledger has no `Metrics: completions` or `Metrics: fix-pass` line). Supplementary, from run.md and not a Metrics line: "fixBeads-round-1: ht-4is.33 · .34 · .35 · .36 — all merged via guarded-merge (check+clippy pass), closed"; "fixLoopExit: converged at round 2 · new round-2 findings introduced by round-1 fixes: src/cli/retry.rs:119 (020055fa), src/cli/hook.rs:734 (eed544e7) — parked".
### Merge-back
- `Metrics: merges`: none. `ledger-check`: none.
- From the 24 ledger `Merge:` lines (pre-switch format, "gate" field, last written ~2026-09-28): conflict 2 (ht-4is.3.2 "rebase conflict: 1 files", ht-4is.3.5 "rebase conflict:5 files") · seam-review fired 0 · check fail→fixed 0 · check fail 1 (ht-4is.3.2 "gate fail") · → blocker 2 (ht-4is.3.2; ht-4is.3.5 → ht-3xy).
- Later merges appear only in run.md prose (e.g. "Wave 6 landed with check pass on every merge"; one no-op merge reported "check pass" and was recovered, see Already fixed).
### Coverage
- coverage-round-1: `requirements: 25 · mapped: 25 · unmapped: 0`
- coverage-round-2: `requirements: 25 · mapped: 25 · unmapped: 0`
- fix round 1: `scope-filter: 4 in-scope · 20 punch-listed`
- fix round 2: none (exited on [converged]; "no round-2 scope filter: nothing is filed on converge")
### Bead graph
Shape: 126 nodes, all closed. 214 `blocks` edges, 1 external blocker (ht-fy0). Longest chain 14. Width per depth for non-epic nodes: 60/3/7/3/10/12/4/3/1/3/4/1/1/1. 69 nodes have no blocks edge. Sweep node ht-4is.12 has 91 blockers.

| id | type | title | what (first sentence of description) |
| --- | --- | --- | --- |
| ht-4is | epic | Build herdr-threads native mailbox plugin | Build the real Herdr-native persistent thread plugin described by the root spec. |
| ht-4is.1 | task | Define compiled component and API contracts | Land the minimal Rust crate skeleton and typed command/result, ID, clock, store, Herdr host, caller-verification, notification and local-service boundaries. |
| ht-4is.2 | epic | Demonstrate native top-level cooperative receipt workflow | Current user direction (supersedes the historical stronger caller-enforcement assumptions below): demonstrated cooperation through prompting is sufficient for the top-level-only acceptance/ACK workflow. |
| ht-4is.2.1 | task | Capture Codex root and child invocation evidence | Capture native Codex parent, child, resumed/new conversation, concurrent child and stale/missing caller metadata using the nested spec's harmless nonce probe. |
| ht-4is.2.2 | task | Capture Claude root and child invocation evidence | Capture native Claude parent, child, resumed/new conversation, concurrent child and stale/missing caller metadata using the nested spec's harmless nonce probe. |
| ht-4is.2.3 | task | Finalize verified caller-attribution recipe | Consolidate both native capture results into the versioned production caller-attribution recipe. |
| ht-4is.3 | epic | Implement durable thread and receipt transactions | Implement SQLite schema/migrations and the domain transaction layer: thread topic/history/archive, invitation episodes, join/leave, sender snapshots plus explicit invited recipients, immutable messages, exact-ID ACK batc… |
| ht-4is.3.1 | task | Create SQLite schema and connection substrate | Implement schema v1 and the production connection/transaction factory with WAL/FULL, foreign keys, bounded busy handling, integrity/version checks and typed error conversion. |
| ht-4is.3.2 | task | Implement thread membership and seat control transactions | Implement thread creation/topic/archive/reopen, invite/reinvite/accept/leave and internal seat/binding/unavailable/rebind/retire transitions. |
| ht-4is.3.3 | task | Implement message snapshots and atomic receipt settlement | Implement send, exact-ID ACK batch, availability-based receipt timer start, due evaluation, use of shared idempotent-operation primitives and immutable event/wake-work writes. |
| ht-4is.3.4 | task | Implement bounded history and pending-obligation queries | Implement topic/member filters, deterministic thread stats, pending invites/receipts, recent/forward/backward pages, body continuation and escaped literal search. |
| ht-4is.3.5 | task | Expose the complete production StorePort implementation | Compose concrete StorePort dispatch and shared transaction/operation wrappers across all store command and read methods. |
| ht-4is.3.6 | task | Implement pending invitation deadline scans | Scan still-pending invitation episodes in bounded deadline order and append their unique overdue system warning and durable wake work using the schema transaction/event primitives. |
| ht-4is.3.7 | task | Complete shared contracts and effective durable-work substrate | Implement approved revision-4 shared types, amended unreleased v1 schema, ordered provenance, immutable effective receipt/warning model, source warning markers, scalar revisions and bounded work APIs. |
| ht-4is.3.8 | task | Materialize warning attribution and receipt indexes in bounded quanta | Implement projections only over authoritative logical published state; never publish a staged send or decide a replacement logical warning. |
| ht-4is.3.9 | task | Adapt accepted invitation scan to pending-unwarned indexed work | Follow up closed Task9 without reopening it. |
| ht-4is.3.10 | task | Honor live predecision budgets in observation and retirement producers | Migrate the eight already-budget-bearing observation/reconciliation/retirement producers to the accepted StoreContext::execute_budgeted_decision primitive. |
| ht-4is.3.11 | task | Write and clear retirements.last_error in production retirement processing | Production advance_retirement never wrote/cleared retirements.last_error (fixture-only). |
| ht-4is.3.12 | bug | Inviting an already-joined seat fails with 'unexpected mutation result' and leaves a pending intent | Found by the 8.20 sandbox probe: 'herdr-threads invite THREAD --seat S' where S already joined returns 'herdr-threads: unexpected mutation result', sandboxed or not, and leaves a pending intent in the journal. |
| ht-4is.4 | epic | Implement Herdr seat and occupant reconciliation | Map durable plugin seats to current Herdr pane addresses, terminal observations and native occupants. |
| ht-4is.4.1 | task | Implement explicit-target Herdr host adapter | Implement complete snapshot, lifecycle subscription, current-target read and native prompt primitives with explicit pane addressing. |
| ht-4is.4.2 | task | Implement seat reconciliation decisions | Implement namespace/terminal matching, observed moves, same-incarnation disappearance, ambiguous restore, retirement and occupant replacement decisions, applying typed StorePort transitions. |
| ht-4is.4.3 | task | Verify current top-level invocation contexts | Implement the finite verified native context recipe for both harnesses; validate root identity, execution instance, expiry, host/seat resolution and generation. |
| ht-4is.4.4 | task | Compose identity check-in and operator repair | Compose identity facade, eligible check-in, unavailable routing and explicit repair. |
| ht-4is.4.5 | task | Thread request budget into OrdinaryIdentity::invalidate | Health fix3 sibling sweep (retirement-health-production-fix3-report.md, confirmed by review): src/identity/repair.rs:300 creates a detached 2s CallBudget with Cancellation::default() and is reachable below DomainService … |
| ht-4is.4.6 | bug | Classify witness transport test failure under load | host::transport::witness_tests::fix1_error_capture_observes_cancel_expiry_and_live_budget_before_write_and_after_response failed once in a full serial lib run at 77980f4 (tests/host/witness_transport.rs:248, load avg ~4)… |
| ht-4is.4.7 | bug | Registered seat never follows observed move; Stale outcome stalls reconciliation (CursorStale) | Found by 11.5 (R08, docs/superpowers/runs/2026-09-26-herdr-native-mailbox-thread-plugin/host-recovery-validation/README.md D2). |
| ht-4is.5 | epic | Implement deadline scheduling and coalesced native wakes | Drive persisted due/settlement operations and recoverable per-seat notification work. |
| ht-4is.5.1 | task | Drive bounded deadline scans with injected clock | Implement one-second tick/due driver, bounded 100-record batches and continuation, wakeup scheduling after commit, duration validation and frozen defaults. |
| ht-4is.5.2 | task | Implement durable attention selection and retry policy | Implement per-seat coalescing, generation exposure, current-condition filtering and persisted backoff choices for invites/ordinary/warn reasons. |
| ht-4is.5.3 | task | Compose scheduler reservations and native prompt attempts | Implement Scheduler facade, persistent reservation before prompt, immediate explicit target identity/state recheck, HostPort invocation and bounded transport observations. |
| ht-4is.5.4 | task | Adapt accepted wake policy to occupant-scoped logical warnings | Follow up closed Task16; consume effective candidate and current occupant frontier types while retaining pure-policy isolation and all retry spacing tests. |
| ht-4is.5.5 | task | Redact remaining WorkerStatus feeders into production Health | From last_error fix2 report: Work/WorkDiscovery (src/service/workers.rs:471), General drive/due errors (:562,:574), due phase text (:589), wake callbacks with seat/attempt identities (:510,:769), observation worker (:104… |
| ht-4is.5.6 | feature | Production safe wake: native idle prompt target and submission under the cooperative policy | Found by live TUI demo (native-claude-tui-2): src/host/native.rs safe_wake_target always returns None and submit_prompt returns TargetUnsafe 'current native execution is unverified', so no coalesced warning wake or idle … |
| ht-4is.6 | epic | Implement single-daemon lifecycle and local transport | Implement one service owner per scoped Herdr instance, local IPC, client bounds, exclusive ownership, start/ensure-running/health/stop, graceful shutdown and stale-owner recovery. |
| ht-4is.6.1 | task | Implement scoped runtime paths and exclusive ownership | Resolve runtime host/state/bin context, stable locator namespace, private instance paths and bounded-length socket location. |
| ht-4is.6.2 | task | Implement bounded versioned local IPC client and server | Implement Unix socket framing/client/server and versioned instance/correlation checks with 1MiB allocation bound, 32-connection/queue backpressure and bounded connect/read/response timeout. |
| ht-4is.6.3 | task | Implement daemon run and ensure coordinator | Implement daemon run/ensure, detached self-launch, five-second readiness wait and service-owner loop with pluggable readiness/drain callbacks. |
| ht-4is.6.4 | task | Implement daemon health stop and bounded diagnostics | Implement health response assembly from injected component status, matching instance/boot stop client and bounded log rotation. |
| ht-4is.6.5 | task | Seam contract: detached daemon diagnostics | Deliver compilable daemon diagnostics interface and install/drain lifecycle hooks spanning detached startup and the bounded sink. |
| ht-4is.6.6 | task | Seam integration: detached daemon diagnostics | Compose detached startup with the concrete bounded diagnostic sink and verify the child-owned output path. |
| ht-4is.6.7 | bug | Daemon requests exceed 5 s under a multi-agent party; follow prints 'lost the daemon' | Live demo 2026-10-01: during a tea party (host + 4 agents doing hook check-ins, sends, acks, wakes) 'read --follow' printed '-!- lost the daemon (request deadline elapsed); reconnecting' — a History request took > 5 s. |
| ht-4is.7 | epic | Implement compact agent and operator command interface | Implement create/list/topic/invite/accept/leave/send/ack/archive/reopen, compact inbox/check-in, recent/full paginated reads/search, and operator inspection/health/rebind command rendering over the local API. |
| ht-4is.7.1 | task | Implement typed agent and operator command dispatch | Implement documented commands and structured argument parsing into typed requests, stdin/file/body sources, exact-ID batches, state context selection and typed errors. |
| ht-4is.7.2 | task | Implement bounded progressive output and operator view | Implement compact text/JSON rendering, continuation commands and a read-only plain-text operator snapshot using the same renderer. |
| ht-4is.7.3 | task | Implement durable CLI mutation intent and recovery | Persist private atomic synced operation intent before mutation; recover by stable key with fresh proof; retain until validated result is successfully printed. |
| ht-4is.7.4 | task | Provide exact selected-output encoder checkpoint | Extract the shared pure text/JSON byte encoder checkpoint from Task25 work before query sizing. |
| ht-4is.8 | epic | Implement native Codex and Claude check-in adapters | Implement supported startup/tool/turn hooks and accountable caller-evidence submission using the proven native recipe. |
| ht-4is.8.1 | task | Implement native Codex hook adapter | Implement versioned Codex native hook parsing, allowlisted context normalization, invocation-scoped transport and bounded startup/tool/turn output using the proven recipe. |
| ht-4is.8.2 | task | Implement native Claude hook adapter | Implement versioned Claude native hook parsing, allowlisted context normalization, invocation-scoped transport and bounded startup/tool/turn output using the proven recipe. |
| ht-4is.8.3 | task | Compose owned native hook setup inspection and removal | Implement explicit session/project-scoped setup/inspect/remove, ownership manifest and safe structural composition with existing hook entries. |
| ht-4is.8.4 | task | Implement native managed launch preflight | Implement launch request helper targeting an existing empty pane, seat resolution before start, immediate availability recheck and native argument construction. |
| ht-4is.8.5 | bug | Hook ready commands: rank pending require-ACK handoff first; accepts optional; conditional required paragraph; self seat marker | Native matrix wave 5 (native-claude-matrix-1 P1; native-codex-matrix-1 P1,P2,P3,O1). |
| ht-4is.8.6 | bug | Managed Codex launch: --no-daemon placement and owned -c hooks ignored before exec | native-codex-matrix-1 P4/P5. |
| ht-4is.8.7 | bug | Burst: hook omits the handoff thread's own accept line; driver burst text contradicts step 1 | native-codex-matrix-2 P6/D8. |
| ht-4is.8.8 | feature | User-level setup: install hooks and allow rule in user settings, auto-detect the Herdr instance, drop project mode | User decision 2026-09-30, after a live trial. |
| ht-4is.8.9 | feature | Operator sender mode: let a human pane send without cooperative flags | A live trial (2026-09-30): a person in a shell pane must seat-resolve their own pane, then pass --cooperative-seat/target/harness/role on every command and run a manual check-in before thread create or send works. |
| ht-4is.8.10 | feature | Accept Herdr pane names and labels in --pane | A live trial: 'seat resolve --pane try-target' fails pane_not_found; only Herdr pane IDs (w4:pAB) work. |
| ht-4is.8.11 | feature | Short user-facing IDs: type prefix + 8 base62 chars instead of UUIDs | A live user trial (2026-09-30): IDs like seat-bc121d12-9d30-4510-b93c-e7cd26e890f1 are too long to type or read. |
| ht-4is.8.12 | feature | Bake an agent SKILL.md into the binary: 'herdr-threads skill', plus a help pointer | User request 2026-09-30, following Herdr's convention: 'herdr --skill' prints the agent skill file, and 'herdr --help' ends with an 'Are you an AI?' section that points to it. |
| ht-4is.8.13 | feature | 'herdr-threads setup' with no harness argument sets up every detected harness | User request 2026-09-30: a bare 'herdr-threads setup' (no claude\|codex argument) detects the installed harnesses (claude and codex on PATH whose versions an adapter recipe admits) and runs the user-level setup for each. |
| ht-4is.8.14 | bug | Every CLI command auto-detects the Herdr instance (doctor and others fail in a plain shell) | User trial after a real install (2026-09-30): 'herdr-threads doctor' in a Herdr pane printed 'context: invalid: state directory missing'. |
| ht-4is.8.15 | task | Measure Codex 0.159.3 sandbox default-deny and admit it for the socket allowance | The user's Codex auto-updated to 0.159.3 (2026-09-30). |
| ht-4is.8.16 | bug | Doctor/health report degraded for the designed cooperative mode | User (2026-09-30): doctor shows result: degraded on a healthy install. |
| ht-4is.8.17 | feature | read --follow: live tail of a thread for humans | User request 2026-09-30, from the tea-party demo: a realtime channel view. |
| ht-4is.8.18 | feature | Token diet for agent-facing output: short cursors and a compact machine form | User, live demo 2026-10-01: agents' tool output is huge. |
| ht-4is.8.19 | feature | Launched agents get readable Herdr names (pane label / participant name), not ht-<hash> | User, live demo 2026-10-01 (screenshot): Herdr's agent list shows agents started by 'herdr-threads launch' as 'ht-3a322c6dad083e3…' (NativeLaunchRequest::agent_name = 'ht-' + 12 bytes of sha256(seat)). |
| ht-4is.8.20 | bug | Codex sandbox cannot write the herdr-threads state dir, so mutations fail on real installs | Live tea party take 8 (2026-10-01): a Codex guest's 'herdr-threads accept' failed with a local permission error, and earlier 'ack' needed escalation. |
| ht-4is.9 | task | Wire the production application across component boundaries | Compose the real store, identity verifier, native host adapter, scheduler, service handler and harness check-in path into the executable. |
| ht-4is.10 | epic | Package the native Herdr plugin and release documentation | Provide a valid herdr-plugin.toml, build/install path, startup ensure-running, native operator-view entrypoint and inspection actions. |
| ht-4is.10.1 | task | Build native manifest and runtime entrypoint package | Create herdr-plugin.toml, locked release build script and argv-safe startup/action/operator-view wrappers. |
| ht-4is.10.2 | task | Document installation operation and release with CI checks | Write README/install/operations/release docs and supported-platform deterministic CI. |
| ht-4is.10.3 | task | Verify clean native installation and package lifecycle | Implement and run isolated clean-checkout build/link/install verification with private Herdr configuration/state. |
| ht-4is.10.4 | task | Finalize package support claims from validation evidence | Reconcile initial manifest and release-facing documentation with the completed exact-version native support matrix. |
| ht-4is.10.5 | task | Make cargo clippy --all-targets pass on integration | cargo clippy --locked --all-targets fails on integration 8f94442: error never_loop at src/daemon/transport.rs:405 (loop { tokio::select! |
| ht-4is.10.6 | feature | Human-readable CLI output by default | User request 2026-09-30, after a trial: current CLI output is a key: value form with JSON blobs (e.g. |
| ht-4is.10.7 | feature | Release packaging and curl \| bash installer from GitHub releases | User request 2026-09-30, target repo alepar/herdr-threads (not pushed or published by the agent). |
| ht-4is.10.8 | feature | README rewrite with pitch, install, try-it-yourself, and screencast | User request 2026-09-30, near the end: - README opens with a cheesy pitch line ('Supercharge your agents with their own private message board'), then a couple of neutral, serious sentences. |
| ht-4is.10.9 | bug | Installer next steps are stale after --setup | After a successful --setup install, the installer still prints 'Start Herdr ... |
| ht-4is.11 | epic | Validate crash recovery and native delivery configurations | Exercise the root acceptance matrix across actual component seams and isolated native Codex/Claude sessions. |
| ht-4is.11.1 | task | Test composed-service crash and concurrency boundaries | Implement real-store composed-service failure matrix with test-only subprocess failpoints, injected clock/host and transaction barriers. |
| ht-4is.11.2 | task | Build isolated native validation fixture and evidence format | Provide small native-test resource fixture for private run/config directories, owned pane/server IDs, version capture, allowlisted evidence and cleanup; define receipt evidence manifest consumed by all suites. |
| ht-4is.11.3 | task | Configuration smoke and delivery evidence for native Codex | Run Codex manual and managed prelaunch flows, including lost initial prompt, using packaged plugin. |
| ht-4is.11.4 | task | Configuration smoke and delivery evidence for native Claude | Run Claude manual and managed prelaunch flows, including lost initial prompt, using packaged plugin. |
| ht-4is.11.5 | task | Validate isolated Herdr identity and recovery transitions | Run real isolated host rename/reorder/move/missed-event/reconnect/close/restore and explicit ambiguous rebind scenarios. |
| ht-4is.11.6 | task | Reconcile validation receipts and publish evidence report | Aggregate independent result manifests, join accepted recipient pairs to explicit model calls and SQLite records, and emit truthful scenario/support matrix stamped with code SHA and versions. |
| ht-4is.11.7 | bug | Make service resolution fake-host writer probe load-tolerant | tests/service/resolution.rs:159-161 fake-host SQLite writer probe uses BEGIN IMMEDIATE with zero busy timeout and panics if any other writer is active; elected_health_reports_unverified_then_verified_host_evidence and si… |
| ht-4is.11.8 | bug | hook_entrypoint test target exits 101 under parallel harness | tests/hook_entrypoint.rs passes serially (--test-threads=1) but the process exits 101 with no failure output under the default parallel libtest harness (reproduced at d3ecb8d and later). |
| ht-4is.11.9 | bug | Parallel-harness lock-timeout flakes (journal allocator, bridge duplicate hook, attention VM-ratio) | Under default parallel libtest threads: cli::journal concurrent_publishers_receive_distinct_ordered_keys and harness::bridge duplicate_concurrent_hook_allocates_one_intent_and_execution fail with intent allocator lock ti… |
| ht-4is.11.10 | bug | Sends conflict after host outage: observed_targets keep old host epoch | Found by 11.5 host-recovery validation (R04/R05, docs/superpowers/runs/2026-09-26-herdr-native-mailbox-thread-plugin/host-recovery-validation/README.md D1). |
| ht-4is.11.11 | task | Demo driver fixes from native matrix wave 5 | scripts/validate-native-demo.py: (claude D1/codex D5) move scenario instructions to the front of the handoff body, within the preview; scenario judges return NOT_EXERCISED unless the model saw the instruction. |
| ht-4is.11.12 | task | Demo driver: interactive Codex mode with /new, and lost-prompt, blocked-UI and concurrent-children scenarios | Close-out audit 2: ht-910, 11.3 and 11.4 still need native evidence for: (a) Codex interactive (TUI) mode in a Herdr pane, including a /new (clear) phase (Codex's equivalent of Claude /clear), plus the warning idle-wake … |
| ht-4is.12 | task | Integration sweep: durable native Herdr threads | Verify the complete plugin goal through its installed CLI/API and native integrations after every implementation leaf. |
| ht-4is.12.1 | task | Parked review findings for final sweep (super-code phase) | Findings parked under the user's lighter super-code review policy (merge at 0 Blocking). |
| ht-4is.13 | task | Design roast 1 F2: Define continuation for every bounded public collection | Design-stage fix for confirmed Blocking finding F2. |
| ht-4is.14 | task | Design roast 1 F3: Expose pending receipt IDs independently of history | Design-stage fix for confirmed Should-fix finding F3. |
| ht-4is.15 | task | Design roast 1 F4: Define recovery for threads with no joined seats | Design-stage fix for confirmed Should-fix finding F4. |
| ht-4is.16 | task | Design roast 1 F7: Bound history search execution and isolate read work | Design-stage fix for confirmed Should-fix finding F7. |
| ht-4is.17 | task | Design roast 1 F8: Bound host calls and recover stalled wake attempts | Design-stage fix for confirmed Blocking finding F8. |
| ht-4is.18 | task | Design roast 1 F9: Define operator actor establishment and repair authority | Design-stage fix for confirmed Should-fix finding F9. |
| ht-4is.19 | task | Design roast 1 F10: Require current host evidence for accountable mutations | Design-stage fix for confirmed Blocking finding F10. |
| ht-4is.20 | task | Design roast 1 F11: Treat peer topics and metadata as hook data | Design-stage fix for confirmed Should-fix finding F11. |
| ht-4is.21 | task | Design roast 1 F12: Order host observations before identity transitions | Design-stage fix for confirmed Blocking finding F12. |
| ht-4is.22 | task | Design roast 1 F13: Sample deadline time in the deciding transaction | Design-stage fix for confirmed Blocking finding F13. |
| ht-4is.23 | task | Design roast 1 F14: Separate wake spacing from wall-clock deadlines | Design-stage fix for confirmed Should-fix finding F14. |
| ht-4is.24 | task | Design roast 1 F15: Recover idle startup registration after transient outage | Design-stage fix for confirmed Blocking finding F15. |
| ht-4is.25 | task | Design roast 1 F17: Record overdue warnings before retirement | Design-stage fix for confirmed Blocking finding F17. |
| ht-4is.26 | task | Design roast 2 R2-F1: Bound resumable seat retirement | Resolve design roast 2 R2-F1 (Should-fix) by defining bounded resumable retirement before store implementation. |
| ht-4is.27 | task | Implement immutable local CheckIn cache paging | Implement owner1 of the exact compact startup contract adopted after CLEAN176 at d70ebc9494a5cf6f833a324f3030098eaa3fdf81. |
| ht-4is.28 | task | Compose bounded startup thread overview and hook output | Implement owner2 of the exact compact startup contract adopted after CLEAN176. |
| ht-4is.29 | task | Define programmatic participant and authority gate contracts | Design complete: user-approved split, nested specifications and final coverage review; main epic owner schedules implementation. |
| ht-4is.30 | epic | Design and implement persistent system registration | Design complete: user-approved split, nested specifications and final coverage review; main epic owner schedules implementation. |
| ht-4is.30.1 | task | Implement persistent system connection and fenced authority | Design complete: implementation remains to be performed by the epic owner. |
| ht-4is.30.2 | task | Expose generation-targeted system disconnect and diagnostics | Design complete: implementation remains to be performed by the epic owner. |
| ht-4is.31 | epic | Design and implement service-managed threads and required invitations | Design complete: user-approved split, nested specifications and final coverage review; main epic owner schedules implementation. |
| ht-4is.31.1 | task | Migrate service author and required membership substrate | Design complete: implementation remains to be performed by the epic owner. |
| ht-4is.31.2 | task | Implement managed thread and required invitation transitions | Design complete: implementation remains to be performed by the epic owner. |
| ht-4is.31.3 | task | Publish attributable programmatic system notifications | Design complete: implementation remains to be performed by the epic owner. |
| ht-4is.32 | epic | Design and integrate programmatic client and service flows | Design complete: user-approved split, nested specifications and final coverage review; main epic owner schedules implementation. |
| ht-4is.32.1 | task | Provide persistent programmatic client and durable request intents | Design complete: implementation remains to be performed by the epic owner. |
| ht-4is.32.2 | task | Wire system operations and versioned native acceptance end to end | Design complete: implementation remains to be performed by the epic owner. |
| ht-4is.32.3 | task | Validate required system membership in Codex and Claude | Design complete: implementation remains to be performed by the epic owner. |
| ht-4is.33 | bug | Seat lookup by pane pages the whole seats table | Finding [Should-fix] src/cli/hook.rs:716: find_seat (hook.rs, MAX_SEAT_PAGES=8) and pane_seat (cli/mod.rs ~974-1018) page all seats (retired included) over RPC and match target==pane client-side; once ~800 seats exist, l… |
| ht-4is.34 | bug | Body paging misses the complete remainder | Finding [Should-fix] src/store/queries.rs:2126: body paging binary-searches a non-monotonic size predicate (complete page has no cursor/argv and is smaller than a partial page), so it can return a truncated page+cursor o… |
| ht-4is.35 | bug | CLI and store disagree on the message body limit | Finding [Should-fix] src/cli/input.rs:7: CLI MAX_BODY_BYTES=900_000 but store messages.rs:48 MAX_BODY_BYTES=65_536; a 64KiB-900KB body is journaled as a durable intent, rejected, and never completes. |
| ht-4is.36 | bug | Dead worker lanes keep Health reporting Ready | Finding [Should-fix] src/app.rs:446; src/app.rs:192: deadline/wake/observation worker threads are unsupervised; a panic stops the lane silently while Health reports scheduler Ready (joined only at shutdown). |

| dependent | blocker | reason (from `blocked-by` line, or `unstated`) |
| --- | --- | --- |
| ht-4is.1 | ht-4is.26 | consumes bounded retirement transition contract |
| ht-4is.2.3 | ht-4is.2.1 | consumes Capture Codex root and child invocation evidence results and fixtures |
| ht-4is.2.3 | ht-4is.2.2 | consumes Capture Claude root and child invocation evidence results and fixtures |
| ht-4is.3.1 | ht-4is.26 | consumes bounded retirement transition contract |
| ht-4is.3.1 | ht-4is.1 | consumes compiled component and ID contracts |
| ht-4is.3.2 | ht-4is.3.1 | consumes SQLite schema and transaction substrate |
| ht-4is.3.2 | ht-4is.3.7 | unstated |
| ht-4is.3.3 | ht-4is.3.1 | consumes SQLite schema and transaction substrate |
| ht-4is.3.3 | ht-4is.3.2 | unstated |
| ht-4is.3.3 | ht-4is.3.7 | unstated |
| ht-4is.3.4 | ht-4is.7.4 | unstated |
| ht-4is.3.4 | ht-4is.3.1 | consumes SQLite schema and indexed read substrate |
| ht-4is.3.4 | ht-4is.3.7 | unstated |
| ht-4is.3.5 | ht-4is.3.4 | consumes bounded query handlers |
| ht-4is.3.5 | ht-4is.3.2 | consumes thread/seat control handlers |
| ht-4is.3.5 | ht-4is.3.7 | unstated |
| ht-4is.3.5 | ht-4is.3.3 | consumes message/receipt/idempotency/due handlers |
| ht-4is.3.5 | ht-4is.3.9 | unstated |
| ht-4is.3.5 | ht-4is.3.6 | consumes pending invitation deadline scan handler |
| ht-4is.3.5 | ht-4is.3.8 | unstated |
| ht-4is.3.6 | ht-4is.3.1 | consumes SQLite schema and transaction substrate |
| ht-4is.3.7 | ht-4is.1 | unstated |
| ht-4is.3.7 | ht-4is.3.1 | unstated |
| ht-4is.3.8 | ht-4is.3.3 | unstated |
| ht-4is.3.8 | ht-4is.3.7 | unstated |
| ht-4is.3.8 | ht-4is.3.2 | unstated |
| ht-4is.3.9 | ht-4is.3.7 | unstated |
| ht-4is.3.9 | ht-4is.3.6 | unstated |
| ht-4is.3.10 | ht-fy0 | consumes accepted cooperative core serial integration and reviewed service-fixture compatibility gate. |
| ht-4is.4.1 | ht-4is.1 | consumes compiled HostPort and identity types |
| ht-4is.4.1 | ht-4is.3.7 | unstated |
| ht-4is.4.2 | ht-4is.4.1 | consumes normalized host snapshots and incarnation evidence |
| ht-4is.4.2 | ht-4is.3.7 | unstated |
| ht-4is.4.3 | ht-4is.1 | consumes compiled CallerVerifier and binding types |
| ht-4is.4.3 | ht-4is.2.3 | consumes verified two-harness attribution and invocation transport recipe |
| ht-4is.4.4 | ht-4is.3.7 | unstated |
| ht-4is.4.4 | ht-4is.4.2 | consumes seat reconciliation transitions |
| ht-4is.4.4 | ht-4is.4.3 | consumes current top-level caller verifier |
| ht-4is.5.1 | ht-4is.3.7 | unstated |
| ht-4is.5.1 | ht-4is.1 | consumes Clock and StorePort due contracts |
| ht-4is.5.2 | ht-4is.1 | consumes wake work and host state contracts |
| ht-4is.5.3 | ht-4is.5.2 | consumes attention selection and retry decisions |
| ht-4is.5.3 | ht-4is.3.7 | unstated |
| ht-4is.5.3 | ht-4is.5.4 | unstated |
| ht-4is.5.3 | ht-4is.5.1 | consumes deadline scan driver |
| ht-4is.5.4 | ht-4is.5.2 | unstated |
| ht-4is.5.4 | ht-4is.3.7 | unstated |
| ht-4is.6.1 | ht-4is.1 | consumes runtime context and instance identity contracts |
| ht-4is.6.2 | ht-4is.3.7 | unstated |
| ht-4is.6.2 | ht-4is.1 | consumes request/result and pluggable handler contracts |
| ht-4is.6.3 | ht-4is.6.1 | consumes exclusive ownership and scoped paths |
| ht-4is.6.3 | ht-4is.6.2 | consumes bounded IPC server/client |
| ht-4is.6.3 | ht-4is.6.5 | consumes boundary contract |
| ht-4is.6.4 | ht-4is.6.1 | consumes instance and boot descriptor types |
| ht-4is.6.4 | ht-4is.6.2 | consumes bounded IPC client and handler |
| ht-4is.6.4 | ht-4is.6.5 | consumes boundary contract |
| ht-4is.6.5 | ht-4is.1 | consumes compiled daemon callback and error contracts |
| ht-4is.6.5 | ht-4is.6.1 | unstated |
| ht-4is.6.6 | ht-4is.6.3 | consumes detached startup and owner-loop diagnostics routing |
| ht-4is.6.6 | ht-4is.6.4 | consumes bounded diagnostic sink and health/control helpers |
| ht-4is.7.1 | ht-4is.3.7 | unstated |
| ht-4is.7.1 | ht-4is.1 | consumes compiled request/client and command contracts |
| ht-4is.7.2 | ht-4is.7.1 | unstated |
| ht-4is.7.2 | ht-4is.7.4 | unstated |
| ht-4is.7.2 | ht-4is.1 | consumes query/result/page and client contracts |
| ht-4is.7.3 | ht-4is.3.7 | unstated |
| ht-4is.7.3 | ht-4is.7.4 | unstated |
| ht-4is.7.3 | ht-4is.1 | consumes semantic request/client and operation result contracts |
| ht-4is.7.4 | ht-4is.3.7 | unstated |
| ht-4is.8.1 | ht-4is.2.3 | consumes verified Codex attribution and invocation transport recipe |
| ht-4is.8.1 | ht-4is.1 | consumes compiled hook/check-in and caller evidence contracts |
| ht-4is.8.2 | ht-4is.2.3 | consumes verified Claude attribution and invocation transport recipe |
| ht-4is.8.2 | ht-4is.1 | consumes compiled hook/check-in and caller evidence contracts |
| ht-4is.8.3 | ht-4is.8.2 | consumes Claude hook declarations and config schema |
| ht-4is.8.3 | ht-4is.3.7 | unstated |
| ht-4is.8.3 | ht-4is.8.1 | consumes Codex hook declarations and config schema |
| ht-4is.8.4 | ht-4is.3.7 | unstated |
| ht-4is.8.4 | ht-4is.1 | consumes HostPort launch/seat lookup and native argument contracts |
| ht-4is.8.4 | ht-4is.4.1 | unstated |
| ht-4is.8.4 | ht-4is.8.3 | unstated |
| ht-4is.8.4 | ht-4is.4.4 | unstated |
| ht-4is.9 | ht-4is.8.3 | consumes both native adapters and owned setup facade |
| ht-4is.9 | ht-4is.7.2 | consumes bounded renderer and operator view |
| ht-4is.9 | ht-4is.3.8 | unstated |
| ht-4is.9 | ht-4is.3.9 | unstated |
| ht-4is.9 | ht-4is.6.6 | consumes verified daemon runtime and diagnostic composition |
| ht-4is.9 | ht-4is.7.1 | consumes typed command runner |
| ht-4is.9 | ht-4is.5.4 | unstated |
| ht-4is.9 | ht-4is.5.3 | consumes deadline and notification loops |
| ht-4is.9 | ht-4is.7.4 | unstated |
| ht-4is.9 | ht-4is.7.3 | consumes durable intent/retry coordinator |
| ht-4is.9 | ht-4is.3.5 | consumes concrete StorePort and domain transactions |
| ht-4is.9 | ht-4is.4.4 | consumes Herdr mapping and current-caller verifier |
| ht-4is.9 | ht-4is.8.4 | consumes managed native launch policy |
| ht-4is.10.1 | ht-4is.7.1 | consumes established command syntax and parser contract |
| ht-4is.10.2 | ht-4is.8.3 | consumes owned hook setup and compatibility instructions |
| ht-4is.10.2 | ht-4is.7.1 | consumes agent command usage contract |
| ht-4is.10.3 | ht-4is.10.1 | consumes native manifest/build/runtime package |
| ht-4is.10.3 | ht-4is.9 | consumes composed executable and service |
| ht-4is.10.4 | ht-4is.10.2 | consumes initial installation operation and release documentation |
| ht-4is.10.4 | ht-4is.10.1 | consumes native package manifest and build layout |
| ht-4is.10.4 | ht-4is.11.6 | consumes reconciled validation support matrix and evidence report |
| ht-4is.11.1 | ht-4is.3.9 | unstated |
| ht-4is.11.1 | ht-4is.9 | consumes production composed service and executable |
| ht-4is.11.1 | ht-4is.3.8 | unstated |
| ht-4is.11.1 | ht-4is.5.4 | unstated |
| ht-4is.11.2 | ht-4is.1 | consumes ID/result and evidence-facing types |
| ht-4is.11.3 | ht-4is.11.2 | consumes isolated native fixture and evidence schema |
| ht-4is.11.3 | ht-4is.10.3 | consumes verified installed plugin entrypoints |
| ht-4is.11.4 | ht-4is.11.2 | consumes isolated native fixture and evidence schema |
| ht-4is.11.4 | ht-4is.10.3 | consumes verified installed plugin entrypoints |
| ht-4is.11.5 | ht-4is.11.2 | consumes isolated native host fixture and evidence schema |
| ht-4is.11.5 | ht-4is.10.3 | consumes verified installed plugin entrypoints |
| ht-4is.11.6 | ht-4is.11.5 | consumes isolated host recovery evidence |
| ht-4is.11.6 | ht-4is.11.3 | consumes Codex native receipt evidence |
| ht-4is.11.6 | ht-4is.11.1 | consumes deterministic fault results |
| ht-4is.11.6 | ht-4is.11.4 | consumes Claude native receipt evidence |
| ht-4is.12 | ht-4is.10.4 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.19 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.5.1 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.30.1 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.11.5 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.26 | consumes bounded retirement transition contract |
| ht-4is.12 | ht-4is.6.5 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.30.2 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.4.1 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.2.3 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.4.2 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.24 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.7.1 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.25 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.3.1 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.5.6 | unstated |
| ht-4is.12 | ht-4is.11.1 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.11.3 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.3.6 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.21 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.8.3 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.32.3 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.3.10 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.5.2 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.7.3 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.8.2 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.11.4 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.10.3 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.32.1 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.8.1 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.6.2 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.8.7 | unstated |
| ht-4is.12 | ht-4is.1 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.4.4 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.5.3 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.3.7 | unstated |
| ht-4is.12 | ht-4is.10.2 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.7.2 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.17 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.6.3 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.5.4 | unstated |
| ht-4is.12 | ht-4is.7.4 | unstated |
| ht-4is.12 | ht-4is.3.2 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.10.1 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.31.2 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.14 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.2.1 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.3.8 | unstated |
| ht-4is.12 | ht-4is.9 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.13 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.20 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.11.6 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.31.3 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.27 | unstated |
| ht-4is.12 | ht-4is.29 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.3.5 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.31.1 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.16 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.32.2 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.18 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.2.2 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.28 | unstated |
| ht-4is.12 | ht-4is.6.1 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.22 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.23 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.8.4 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.6.6 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.3.9 | unstated |
| ht-4is.12 | ht-4is.3.4 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.3.3 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.4.3 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.6.4 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.11.2 | consumes all leaves (integration sweep) |
| ht-4is.12 | ht-4is.15 | consumes all leaves (integration sweep) |
| ht-4is.28 | ht-4is.27 | unstated |
| ht-4is.30 | ht-4is.29 | consumes compiled programmatic participant and authority gate contracts |
| ht-4is.30.1 | ht-4is.6.2 | consumes implemented component interface and its accepted tests |
| ht-4is.30.2 | ht-4is.6.4 | consumes implemented component interface and its accepted tests |
| ht-4is.30.2 | ht-4is.30.1 | consumes implemented component interface and its accepted tests |
| ht-4is.31 | ht-4is.29 | consumes compiled programmatic participant and authority gate contracts |
| ht-4is.31.1 | ht-4is.3.7 | consumes implemented component interface and its accepted tests |
| ht-4is.31.2 | ht-4is.3.2 | consumes implemented component interface and its accepted tests |
| ht-4is.31.2 | ht-4is.31.1 | consumes implemented component interface and its accepted tests |
| ht-4is.31.3 | ht-4is.3.3 | consumes implemented component interface and its accepted tests |
| ht-4is.31.3 | ht-4is.31.1 | consumes implemented component interface and its accepted tests |
| ht-4is.31.3 | ht-4is.3.8 | consumes bounded notification audience/attention materialization |
| ht-4is.32 | ht-4is.29 | consumes compiled programmatic participant and authority gate contracts |
| ht-4is.32.2 | ht-4is.31.3 | consumes implemented component interface and its accepted tests |
| ht-4is.32.2 | ht-4is.31.2 | consumes implemented component interface and its accepted tests |
| ht-4is.32.2 | ht-4is.9 | consumes implemented component interface and its accepted tests |
| ht-4is.32.2 | ht-4is.30.1 | consumes implemented component interface and its accepted tests |
| ht-4is.32.2 | ht-4is.30.2 | consumes implemented component interface and its accepted tests |
| ht-4is.32.2 | ht-4is.32.1 | consumes implemented component interface and its accepted tests |
| ht-4is.32.3 | ht-4is.8.1 | consumes implemented component interface and its accepted tests |
| ht-4is.32.3 | ht-4is.8.2 | consumes implemented component interface and its accepted tests |
| ht-4is.32.3 | ht-4is.11.2 | consumes isolated native validation fixture and evidence format |
| ht-4is.32.3 | ht-4is.32.2 | consumes implemented component interface and its accepted tests |

## Design questions

### A. Should the `parallelism:` detector lines be persisted to a file?
- **Evidence:** this run has none. They are Workflow log output that only the invoking session can capture, and the run crossed 4+ pause/resume checkpoints and a Codex→Claude Code harness move. So the speed lens had no instrument.
- **For persisting** (append each line to the ledger or friction.md): cheap, survives restarts and harness moves, and upstream-feedback can read it from a file like every other input.
- **Against:** log noise in a working file, and per-round shape may be partly reconstructible from `Merge:` timing.

If upstream decides otherwise, please state the position explicitly so downstream can reconcile against words rather than silence.

### B. How should coverage-round inputs reach reviewers when the runtime takes only literal prompt strings and caps tool output?
- **Evidence:** "thirty exact input windows total 2,627,571 bytes". "The single-read bootstrap truncated even the smallest52KB input … Two seats exhausted their one allowed redispatch". Result: "30 seats / 35 attempts / 28 valid", attribution and store 2/3, coverage marked advisory and the method substituted.
- **For** a sanctioned attachment mode (read one immutable hashed file in bounded chunks, then no tools): it keeps the no-roaming boundary at any input size.
- **Against:** bound the window size at planning time so the literal-prompt contract always fits, and keep reviewers tool-free.

If upstream decides otherwise, please state the position explicitly so downstream can reconcile against words rather than silence.

## Doc gaps
- **Phase 6 report writer** (Defect 3 above): who writes and commits `report.md` when authorship is delegated.
- **Mid-run skill switch and the ledger.** `super-auto/SKILL.md:103-105` records `skillSource`/`migrated:` but does not say whether the super-code ledger continues, or is re-initialised, under the new format.
  - Evidence: the ledger's 24 `Merge:` lines stop around 2026-09-28 in the old "gate" format, with no `Metrics:` block.
  - Every later merge is recorded only as `run.md` prose, so `## Run metrics` Fix loop and Merge-back are empty for most of the run.
  - Premise: this coordinator ran lanes by hand after the switch, not through the super-code Workflow. Upstream should state whether that is in contract.

## Already fixed — do not re-litigate
- **Per-merge build-only check** (friction 2026-09-30: "caught 3 real cross-branch compile seams (b0d24c5, 80f90cc, bf00d19)"): `config.mergeCheck`, plus a scoped merge-check fix at 44f9868.
- **No-op merge reported "check pass"** because an untracked identical file blocked `git merge` silently: fail-closed merge (clean porcelain, exit 0 + MERGE_HEAD) and the lane write fence at 3d369ed.
- **Mid-run switch to local skill definitions had no procedure:** `super-auto/SKILL.md:103-105` @e8244b0.
- **Cumulative full-history audit after every NONZERO review** (>270 reports, about 1 MB each time): superseded by the §Phase 5 step-back + scope filter (`run.md` `migrated:` line).
- **"Don't end a turn" vs the background Workflow/Agent wait model:** super-code `SKILL.md` @fe26d80 now says "Whenever you are woken during a run (a notification, …)". Upstream should confirm this is the intended carve-out.
- **Released in 4cd542b (6.4.2-alepar4.2)**, after this analysis ran (the analyst saw them uncommitted); this run switched to 4cd542b and ran the regression-only pass (`regressionPass-round-2`, beads ht-4is.37/.38):
  - **No super-roast args assembler.** The coordinator hand-maintained a `build.js` and hand-extended it for iteration 2 (stance/recall blocks, regression lane, prior report, report filename). Now `skills/super-roast/scripts/assemble-args{,.mjs}`.
  - **Converged round parks fix-introduced regressions.** Round 2 reported "0 regressed" while 2 of 3 new Should-fix were "introduced by commit 020055fa / eed544e7, the fix for iteration-1 …", and the loop exited on `[converged]` with them parked. Now the engine adds a `[fix-regression]` tag and §Phase 5 runs a `regressionPass-round-<N>` regression-only pass.
  - **Lane agent used `git stash push/pop`.** A sonnet fix-lane did this despite the harness's shared-stash warning; no skill text @e8244b0 mentions stash. Now a "Never run git stash" clause is in the lane write fence and the pre-merge clean step of `coordinator-workflow.md`.

## Not established
- By default: the coordinator's ledger-append path is fire-and-forget and can lose a line without the coordinator noticing, so every ledger-derived count in `## Run metrics` (Fix loop, Merge-back) is a lower bound. The `Metrics: ledger-check` line is the one cross-check that exists.
- **Missing sources:**
  - `parallelism:` detector lines: not captured anywhere.
  - Ledger `Metrics:` block (completions, fix-pass, merges) and the `ledger-check` line: absent. The ledger (`.superpowers/sdd/ht-4is-plan/progress.md`) was last modified 2026-09-28.
  - `Slowness:` / `Edge cut:` lines: absent. `codeBuckets.slowness` was never recorded.
- **Merge-back counts** cover only the 24 pre-switch `Merge:` lines (old "gate" field). Post-switch merges are known only from `run.md` prose and are not counted.
- **Judge-panel seat agreement** is not comparable across rounds: design roasts used same-family GPT seats, run as a manual fallback outside the Workflow engine; PR roasts used same-family Claude seats.
- **No speed claim is made.** There is no measured wall-time or concurrency number; ~4.3 days is calendar span including user pauses.
- **Defect 3** (report writer blocked) and the three now-released fixes above come from the coordinator's session observations; they are not friction.md lines. The hand-maintained `build.js` / `build.js.r1` live only in a session scratchpad, not in the run dir.
- **Defect 4** (exec bit): this report does not check whether the Codex skill installer drops file modes. Only the two exit-126 events are established.
- **Analyst model:** Opus 5.5 (claude-opus-5-5), fresh context. The analyst read the run's files directly rather than receiving pre-gathered inputs.

## Verification bar
- **Defect 1:** replay this run's roast-pr-1 (24 confirmed) and its step-back through the scope filter. Expect cluster (a) store/mod.rs:838, mod.rs:689, seats.rs:702 and hook.rs:716 to be scoped together, or a recorded override line. Then check that a round-2 roast does not re-raise the cluster pattern (the seats.rs:1217 analogue).
- **Defect 2:** run a round-2 roast with a prior report on a non-trivial artifact where all scouts return nothing. The verdict should be able to reach `clean … [converged]`, or carry a qualifier that is explicitly non-blocking.
- **Defect 3:** run phase 6 on Claude Code with a delegated report writer. `report.md` must land and be committed, with no harness Write refusal.
- **Defect 4:** install the plugin through the Codex skills path and confirm `subagent-driven-development/scripts/sdd-workspace` is executable, or that every caller invokes it via `bash`.
- **Released fixes (4cd542b):**
  - Run `assemble-args --iteration 2 --prior-report <roast-pr-1>` and diff its prompts against this run's hand-built iteration-2 args (stance/recall blocks, regression lane, prior report, filename).
  - Feed roast-pr-2's findings through the engine and confirm exactly retry.rs:119 and hook.rs:734 get `[fix-regression]`, and seats.rs:1217 does not.

---
If a premise above is wrong, stop and say so rather than improvising a larger change.
