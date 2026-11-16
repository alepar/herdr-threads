super-roast verdict: Blocking (1 confirmed)
mode: pr        iteration: 2 of 3
profile (assumed): Production local same-user cooperative messaging tool. Wake mistakes cost user attention and quota; durable migration or message/receipt corruption is material. Same-user cooperative trust is explicit; adversarial caller hardening is outside scope.
inputs: feat/codex-queue-control@a8abe0c43 vs main@4dabf06ef
delta vs prior: 1 new confirmed (1 Blocking) · 0 carried (0 Blocking) · 4 resolved · 0 regressed (0 Blocking) · 0 punch-listed (open)
coverage: scouts 16/16 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, regression, data-migrations, deploy-safety, api-contract, observability, testing, test-causality-lifecycle, test-isolation-environment, test-suite-cost, hygiene-docs) · raw 1 → deduped 1 → panel 1 · spot 0 · promoted 0 · judge completion 100% · remainder-capped: 0 · review readiness: coverage complete, provenance available — basis: "Immutable captured head a8abe0c43 and base4dabf06ef, full source and diff in dedicated detached worktree, prior formal report and current correction qualification evidence plus historical full/live manifests available."
independence: same-family (OpenAI Codex) — seat-differentiated panel · rung: manual fan-out
seat-agreement: panels 1 · rr 1.00 · rg 1.00 · fg 1.00 · unanimous 1.00 · ground-loo 1.00 (n=1) · reproduce 1/0/0 · refute 1/0/0 · ground 1/0/0
lane-yield (found/confirmed/unique/refuted): correctness 0/0/0/0 · security 0/0/0/0 · premortem 1/1/1/0 · simplicity-design 0/0/0/0 · hot-path-perf 0/0/0/0 · concurrency-async 0/0/0/0 · regression 0/0/0/0 · data-migrations 0/0/0/0 · deploy-safety 0/0/0/0 · api-contract 0/0/0/0 · observability 0/0/0/0 · testing 0/0/0/0 · test-causality-lifecycle 0/0/0/0 · test-isolation-environment 0/0/0/0 · test-suite-cost 0/0/0/0 · hygiene-docs 0/0/0/0

The panel used static control-flow review of the captured head and base. It ran no live helper, build or daemon reproduction and established no failure frequency. The regression lane returned no findings. Source readiness is complete; all dispatched seats returned and no findings were capped or lost.

Prior confirmed findings are resolved under this round's absence-based tracking: none resurfaced, and none was punch-listed. This status does not claim a new live reproduction of their fixes.
- resolved: src/notification/queue_recovery.rs:280; src/notification/queue_recovery.rs:278 — completed-history pagination preventing idle exit.
- resolved: scripts/wake-e2e.py:1944 — sampled Add-helper witness lost during cleanup.
- resolved: src/harness/codex/wake/control.rs:502 — missing negative Add-response ownership coverage.
- resolved: src/notification/queue.rs:228 — discarded result-transaction diagnostic.

## Confirmed findings

- [Blocking] src/harness/codex/wake/control.rs:221 — An unreaped helper abandoned at the cleanup deadline can permanently fence both wake transports for its scope while the daemon lives. [lanes: premortem]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: All three seats trace finish setting cleanup_done before cleanup, followed by try_wait returning None and descheduling until after the 250 ms deadline. The deadline return drops Child without reaping it or transferring ownership; Drop cannot retry because cleanup_done is already true.
  evidence: The seats found no production SIGCHLD, waitpid or equivalent reaper. [Rust Child documentation](https://doc.rust-lang.org/std/process/struct.Child.html) says, “The standard library does not automatically wait on child processes (not even if the Child is dropped)”. [Linux wait documentation](https://man7.org/linux/man-pages/man2/wait.2.html) documents that an unwaited terminated child remains a zombie under the normal disposition.
  evidence: helper.rs:62-88 checks birth identity and UID but ignores zombie state, which [Linux proc documentation](https://docs.kernel.org/filesystems/proc.html) documents separately. The unreaped matching leader therefore stays Same and cannot classify as Quiescent, regardless of the group probe.
  evidence: queue_recovery.rs:366-405 reconciles only Quiescent. store/wake_intents.rs retains cleanup_quarantined independently of logical fulfillment; selection.rs:31-39 fences both transports. A witnessed Accepted helper does not qualify for the unwitnessed/no-add native exception. queue_recovery.rs:293-303 counts this quarantine as pending work, so ordinary idle exit does not reliably clear it.
  evidence: These control, helper, recovery and selection paths are introduced relative to base 4dabf06ef. The finding concerns the exact owned direct child, not an uncertain descendant or a generally unkillable helper.
  contract distinction: The design's 250 ms cleanup bound, conservative Unknown quarantine and prohibition on general supervision remain valid. The seats distinguish those accepted limits from discarding direct-child reap ownership and thereby preventing positive recovery even after actual termination. The ownership and recovery obligations appear in design lines 19-33 and 278.
  severity basis: All seats voted Should-fix. The required core-purpose floor raises this to Blocking: the introduced ownership failure can permanently disable both wake transports for an affected seat/cohort during the daemon's lifetime, defeating its wake-delivery purpose after the helper has ended. This is an avoidable permanent recovery obstruction, not the accepted temporary refusal while quiescence remains genuinely unknown. No profile demotion applies.
  fix-shape hint: Retain a bounded reap path for the exact owned child while preserving the cleanup deadline and conservative transport fencing.

## Not verified (beyond panel cap)

- none

## Not verified (dedupe failed or judge lost)

- none

## Beyond remainder cap (count only)

- none

## Rejected (with reason)

- none

## Unverified nits (spot-checked)

- none

## Escalations (need human)

- none
