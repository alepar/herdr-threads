---
super-roast verdict: Should-fix (4 confirmed) [converged]
mode: pr        iteration: 2 of 3
profile (assumed): Local developer tool (CLI plus daemon) that coordinates coding agents on one user's machine. The trust model is cooperative and same-user, the user base is small, and there is no network-exposed surface. The bar is production quality for correctness and data safety of the message store and the user's config, and an internal-tool bar elsewhere.
inputs: ht-fr7c@a230d1328 vs main@f23ab37ee
delta vs prior: 3 new confirmed (0 Blocking) · 1 carried (0 Blocking) · 4 resolved · 0 regressed (0 Blocking) · 0 punch-listed (open)
coverage: scouts 15/15 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, regression, api-contract, observability, testing, test-causality-lifecycle, test-isolation-environment, test-suite-cost, hygiene-docs, deploy-safety) · raw 5 → deduped 3 → panel 3 · spot 0 · promoted 0 · judge completion 100% · remainder-capped: 0 · review readiness: coverage complete, provenance available — basis: "git diff f23ab37ee..a230d1328 in the repository at /var/home/alepar/AleCode/herdr-threads; both commits resolved locally, full sources readable by git show"
independence: same-family (Claude) — seat-differentiated panel · rung: Workflow
seat-agreement: panels 3 · rr 0.67 · rg 1.00 · fg 0.67 · unanimous 0.67 · ground-loo 1.00 (n=2) · reproduce 3/0/0 · refute 2/1/0 · ground 3/0/0
lane-yield (found/confirmed/unique/refuted): correctness 0/0/0/0 · security 0/0/0/0 · premortem 0/0/0/0 · simplicity-design 0/0/0/0 · hot-path-perf 0/0/0/0 · concurrency-async 0/0/0/0 · regression 0/0/0/0 · api-contract 1/1/0/0 · observability 1/1/0/0 · testing 1/1/0/0 · test-causality-lifecycle 1/1/0/0 · test-isolation-environment 0/0/0/0 · test-suite-cost 0/0/0/0 · hygiene-docs 1/1/1/0 · deploy-safety 0/0/0/0

Note on prior-report tracking. No packet this round re-surfaced any iteration 1 confirmed finding, and the punch list is empty. I checked each prior confirmed finding against the head tree instead of reading absence as a fix.
Resolved (4), all in commit 2067de802:
`integrations/hermes/__init__.py:130` (stale pane in `Sessions.observe`): head lines 451-454 pop the session's pane when `session_pane()` returns False.
`src/harness/hermes.rs:1410` (replace-all `manual_argv`): `passthrough_manual_argv` is gone and a test at hermes.rs:2765 asserts `manual_argv().is_none()`.
`integrations/hermes/__init__.py:658` (NO_PANE_NOTICE remedy): the text now reads "in a Herdr pane, ask your user to run ...".
`src/harness/hermes/assets.rs:810` (rollback Conflict): docs/install.md:239 now carries the Hermes downgrade note.
Still open (1): `src/store/topology_handoff/automatic.rs:418`. `git diff f4674faf1..a230d1328` on that file and on src/cli/topology_handoff.rs is empty. It is listed below as carried.
Prior escalations: the "not set" substring match (runtime.rs:281) is closed by code; runtime.rs:314-320 now requires the last stderr line to equal `Config key not set: terminal.client_env_passthrough`, with negative cases at :814-821. The silent `register_session_env` fail-open is addressed; the bridge logs one warning (`__init__.py:924-926`) and TRUST-POLICY.md's accepted limit now covers a Hermes that refuses the registration. The gateway hook-thread question (`Sessions.resolve`) has unchanged code and no recorded human check, so it stays under Escalations.
The iteration 1 unverified nits are not re-listed here. They remain in docs/superpowers/reviews/2026-11-14-ht-fr7c-hermes-first-class-roast-pr-1.md.
No severity was demoted because of the profile.

## Confirmed findings            ← consumed by super-design, one task per finding
- [Should-fix] src/cli/launch.rs:1254 — A hint-requiring launch (Hermes) to a host that does not advertise `agent_start_process_hint_v1` and whose start is not confirmed ends `outcome_unknown` with a possibly running, unbound agent, and the report and launch record omit the `process_hint: host_unsupported` marker that the README, herdr-host.md and TRUST-POLICY.md promise. The marker is set only on `ObservedStartup`. [lanes: api-contract, observability] (new in round 2, commit c24fc4edb)
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: src/cli/launch.rs:1254-1257 and :1292-1293 set `record["process_hint"]` and `report["process_hint"]` only under `matches!(&outcome, NativeLaunchOutcome::ObservedStartup { correlation, .. } if correlation.process_hint && !correlation.process_hint_sent)`. `NativeLaunchOutcome::OutcomeUnknown` is a unit variant (src/ports.rs:2417). In src/host/native.rs `process_hint_sent` is a local of the start loop (:384, assigned :470) and reaches the caller only through `CorrelatedStartup` (:712-713). The unconfirmed exits return bare `OutcomeUnknown` (for example :540-546 and the deadline exit near :588). TRUST-POLICY.md:2832 says the launch report carries `process_hint: host_unsupported`, and :2833-2835 says a host that cannot recognize the agent without the hint ends `outcome_unknown`. integrations/hermes/README.md:49-51 and docs/compatibility/herdr-host.md:15 make the same unconditional promise. Before c24fc4edb the same launch was refused before the start frame, with the remedy named.
  severity basis: all three seats rated Should-fix. Kept, although the profile down-weights observability, because the normative policy text states the marker, the change replaced a definite pre-start refusal with an unexplained unconfirmed start that can leave a live unbound agent, and herdr-host.md says stock Herdr does not advertise the extension. No safety invariant is broken; the `outcome_unknown` ending itself is accepted by the policy. No seat ran a launch, so whether stock Herdr recognizes Hermes without the hint was not checked.
  fix-shape hint: carry `process_hint_sent` (or a hint-skipped flag) on `OutcomeUnknown`, or derive the marker in cli/launch.rs from the requested mode and the pong; alternatively narrow the three docs to "only when the start is confirmed".
- [Nit] tests/multinode/hermes_composer.rs:155 (also cited as :649, which does not exist; the file has 172 lines) — `hermes_exec_as_pane_root_is_woken_and_a_bare_shell_root_is_not` asserts that a bare shell root is never typed into after a fixed 2 s sleep, with no wait for evidence that the wake decision for P2 ran. A regression that types into a shell root can pass vacuously under load. [lanes: testing, test-causality-lifecycle] (new in round 2, commit bf7b9d081)
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: head lines 155-158 run `support::send(... b.id ... "--nudge")`, then `std::thread::sleep(Duration::from_secs(2))`, then `assert_eq!(host.prompts_for(P2), 0, ...)`. The sibling test waits first at line 64 (`wait_until("composer reads of P2, P3 and P4", ...)`). This refusal path makes no `agent.read` on P2, and FakeHerdr's `pane.process_info` handler (src/test_support/fake_herdr.rs ~89-111) keeps no per-pane counter, so the test has no edge to wait on. All three seats rated Nit: `wait_wakes(P1, 1)` runs first and proves the dispatcher is live, and unit tests cover `root_composer_choice` and `root_only_foreground_pid`. The failure mode is a silent false pass, not a red flake. No seat reproduced a missed regression.
  fix-shape hint: count `pane.process_info` requests per pane in FakeHerdr, `wait_until` P2's count reaches its post-`wait_seats` baseline + 2, then keep a short negative window before the assert.
- [Nit] TRUST-POLICY.md:2654 — The Hermes section still says, with no condition, that `launch --kind hermes` adds the process hint that makes a bare macOS Hermes recognized. The process-hint paragraph edited in the same commit says the hint is sent only when the host advertises `agent_start_process_hint_v1`, and that a host that cannot recognize the agent without it ends `outcome_unknown`. [lanes: hygiene-docs] (new in round 2, commit c24fc4edb)
  verdict: confirmed (reproduce ✓ / refute REJECT, dissent / ground ✓)
  evidence: head TRUST-POLICY.md:2653-2655: "a bare macOS Hermes is not process-detected by Herdr (`launch --kind hermes` adds the process hint that makes it recognized)". Head TRUST-POLICY.md:2829-2835: a host whose pong does not advertise the capability "gets the same fenced start without the field" and "a host that cannot recognize the agent without the hint ends `outcome_unknown`". docs/compatibility/README.md:43 still says launch "adds the guarded process hint that makes the seat wakeable". The refute seat rejected because the governing paragraph (2823-2835) already states the condition and the Hermes sentence is a platform note. The confirming seats address that same text and still find one stale unconditional sentence in the normative file, so the default stands. The premise that Herdr 0.9.5 lacks the capability comes from the commit message only. The docs/compatibility/herdr-093-audit.md:39 part of the claim was not opened by the ground seat and is a historical audit document.
  fix-shape hint: add "when Herdr advertises it" to the Hermes-section sentence and to docs/compatibility/README.md:43, with a pointer to the process-hint paragraph.
- [Nit] src/store/topology_handoff/automatic.rs:418 — Automatic V2 handoff completion checks the reported argv against the daemon's live registry composition (`compose_bootstrap_v2_argv` over `builtins()`), not a composition frozen in the plan. A daemon and CLI that compose differently reject a handoff whose child is already running. [lanes: correctness, observability] (still-open, see iteration 1)
  verdict: confirmed in iteration 1 (reproduce REJECT, dissent / refute ✗-survived / ground ✓); not re-judged this round
  evidence: head automatic.rs:419 still calls `crate::harness::launch::compose_bootstrap_v2_argv(p.launch.harness, v)`. `git diff f4674faf1..a230d1328 -- src/store/topology_handoff/automatic.rs src/cli/topology_handoff.rs` is empty, and the round 1 fix commit 2067de802 does not list it. The key is not in the caller's punch list, so it cannot be marked punch-listed. Iteration 1 seats noted that Hermes is the only composing adapter, the result is a retryable mismatch, and this is a follow-up, not a pre-merge fix.
  fix-shape hint: freeze the composed argv on the automatic plan and compare against that; or add the key to the punch list if it is deliberately deferred.

## Not verified (beyond panel cap)   ← severe candidates the panel cap left unverified — listed, never dropped
- none

## Not verified (dedupe failed or judge lost)   ← findings a pipeline failure left unjudged — listed, never dropped
- none

## Beyond remainder cap (count only)   ← low-severity candidates the dedupe remainder cap dropped; the count survives, the claims do not
- none

## Rejected (with reason)        ← so re-roasts don't re-litigate
- src/harness/hermes/runtime.rs:1127, :537, :503, :281 — `ensure_passthrough` relies on unverified Hermes CLI contracts (`config get --json`, `config set` list parsing). (carried from iteration 1, rejected 3-0; not re-surfaced)
  reason: seats read Hermes source on main; `config get --json`, list parsing through `yaml.safe_load` and the "Config key not set:" exit 1 all match. Caveat: Hermes main, not a pinned release; no seat ran Hermes.
- integrations/hermes/__init__.py:203, :233, :916, :913 — the bridge depends on a guessed `register_session_env(names, resolver)` contract with no fallback. (carried from iteration 1, rejected 3-0; not re-surfaced)
  reason: Hermes source (hermes_cli/plugins_context.py, plugins_session_env.py) matches the bridge's call; resolver exceptions are caught and logged.
- src/harness/hermes.rs:1517 — the passthrough check runs on a classic Hermes and warns about names it does not need. (carried from iteration 1, rejected 3-0; not re-surfaced)
  reason: the unconditional write is the epic's recorded decision, AGENTS.md bans version gates, and the warning clears after `setup hermes`.
- src/harness/hermes/runtime.rs:538 — a `config set` killed by the setup deadline could leave a truncated config.yaml. (carried from iteration 1, rejected 3-0; not re-surfaced)
  reason: Hermes writes config through a temp file, fsync and `os.replace`. Caveat: the link from `config set` to the atomic write is partly inferred.
- src/harness/hermes.rs:1258, :1254 — `setup hermes` writes native Hermes config with no opt-out and `unsetup` does not revert it. (carried from iteration 1, rejected 3-0; not re-surfaced)
  reason: the outcome reports the write, it is the epic's task .3, TRUST-POLICY.md states "merge only", and the README documents that `unsetup` leaves the names.
- src/harness/hermes/launch.rs:347 — the launch report's `hermes.api` gains the value `unverified` while scripts/native-hermes-probe.py:434 requires `presence_only`. (carried from iteration 1, rejected 2-1; not re-surfaced)
  reason: the probe is a manual acceptance script and failing on an unrecognized plugin API is the conservative outcome; no product code reads `hermes.api`.
- integrations/hermes/runtime_helper.py:70 — the helper's plugin-API probe drops the module-origin checks and imports `hermes_cli.plugins` in every inspection mode. (carried from iteration 1, rejected 3-0; not re-surfaced)
  reason: the removal is the stated change of ht-fr7c.1; the API facts feed only a diagnostic field and gate nothing.
- tests/golden_surfaces.rs:1411, :1407, :1404 — the golden Topology command drops every PATH directory that contains a `hermes` entry. (carried from iteration 1, rejected 2-1; not re-surfaced)
  reason: source risk only; no golden depends on a tool removed this way, and the filter is deliberate and commented.

## Unverified nits (spot-checked)
- none

## Escalations (need human)      ← UNVERIFIED externals, incomplete panels, unsettled panels, material dissent
- integrations/hermes/__init__.py:154 (head `Sessions.resolve`, lines ~466-480) — on a gateway, `Sessions.resolve` unsets the pane names for any session the bridge never recorded. This is a regression only if plugin hook callbacks cannot see the bound client env while tool subprocesses can. (carried from iteration 1; not re-surfaced by this round's scouts)
  reason: UNVERIFIED external, still open. The code is unchanged at head (`elif os.environ.get("_HERMES_GATEWAY") == "1": values.update(dict.fromkeys(PANE_ENV))`), and no human check is recorded in the branch. In iteration 1 the reproduce seat found the repo lists `bound_client_env()` on the hook thread as an open question (docs/research/2026-10-23-integration-alternatives/README.md:153, comparison/hermes.md:337); the refute and ground seats rejected because base `pane_env()` already depends on the same call.
  human check: on a gateway with `client_env_passthrough` set, log `session_pane()` in a pre_llm_call hook and run `env | grep HERDR_PANE_ID` from a tool call, with and without the bridge registered.
---