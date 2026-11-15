---
super-roast verdict: Should-fix (6 confirmed)
mode: pr        iteration: 1 of 3
profile (assumed): herdr-threads is a local, same-user CLI and daemon for coordinating coding agents, and it is released to users. Its trust model is cooperative, not adversarial, per TRUST-POLICY.md. Human attribution provenance is the product. It is treated as production-grade internal tooling.
inputs: fix/operator-guard@797c6a7e8 vs main@e60ec5eb5
coverage: scouts 12/12 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, api-contract, observability, testing, test-causality-lifecycle, test-isolation-environment, hygiene-docs) · raw 55 → deduped 33 → panel 13 · spot 20 · promoted 0 · judge completion 100% · remainder-capped: 0 · review readiness: coverage complete, provenance available — basis: "git diff e60ec5eb5..797c6a7e8 in the operator-guard worktree"
independence: same-family (Claude) — seat-differentiated panel · rung: Workflow
seat-agreement: panels 13 · rr 1.00 · rg 0.92 · fg 0.92 · unanimous 0.92 · ground-loo 0.92 (n=13) · reproduce 6/7/0 · refute 6/7/0 · ground 5/8/0
lane-yield (found/confirmed/unique/refuted): correctness 7/3/2/4 · security 5/2/1/3 · premortem 5/0/0/5 · simplicity-design 8/0/0/7 · hot-path-perf 0/0/0/0 · concurrency-async 0/0/0/0 · api-contract 7/2/0/5 · observability 4/0/0/4 · testing 8/1/0/5 · test-causality-lifecycle 3/1/1/1 · test-isolation-environment 2/0/0/2 · hygiene-docs 6/1/0/3

Three of the six confirmed findings are pre-existing on base, so they are FYI and do not drive a fix round. Each is a place where the new TRUST-POLICY A4 text, or the relayed request ("agents must not get or use the human operator override"), reads wider than the code enforces. Whether to narrow the policy text or widen the guard is a scope decision for the caller. No severity was demoted because of the profile.

## Confirmed findings            ← consumed by super-design, one task per finding
- [Should-fix] docs/agent-usage.md:91; docs/agent-usage.md:160; docs/operations.md:245 — docs/agent-usage.md, docs/operations.md and docs/install.md still describe `human me init --operator` as the working override for an env-marker refusal and list only the guard markers. This contradicts the shipped behaviour. [lanes: api-contract, hygiene-docs]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: At head, `person_guard` in src/cli/mod.rs refuses first on any `Registry::agent_session_markers` hit and never reads `operator_override` on that path.
    docs/agent-usage.md:91 still says the refusal "names the way out ... or override as the local account with `herdr-threads human me init --operator`".
    docs/agent-usage.md:160 still tells a shell that inherited `PI_CODING_AGENT` to run `human me init --operator` first. That command is now refused.
    docs/operations.md:245 still says to "override as the local account with `me init --operator`" after an env-marker refusal.
    docs/install.md:278 still calls `--operator` "the existing explicit local-account override" (ground seat).
    None of these files is among the 17 changed files. TRUST-POLICY.md, SKILL.md and the help golden were updated.
    The docs also omit the new session-variable triggers (HERMES_SESSION_ID, CLAUDE_CODE_SESSION_ID, CODEX_THREAD_ID and others).
    These docs are agent-facing, so they still advertise the path the change closes. Introduced by this change: the base docs matched base behaviour.
  fix-shape hint: Rewrite the four passages to match TRUST-POLICY A4 *Agent session*: an agent-session variable refuses with no override, the remedy is to unset the inherited variable, and `--operator` lifts only the Herdr-report and connected-binding refusals.
- [Nit] src/cli/mod.rs:254 — `person_guard` now consumes `std::env::vars()` in full even when `--operator` or a recorded operator mark applies. A non-UTF-8 environment key or value now panics `human me init --operator` and every Human-context command under an operator mark. Base never read the environment on those paths. [lanes: correctness]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: Head src/cli/mod.rs:498-501 collects every key (`env.into_iter().map(|(k, _)| k.as_ref().to_owned()).collect()`) before the override check.
    src/cli/me.rs:210 and src/cli/mod.rs:4963 pass `std::env::vars()`.
    Base e60ec5eb5 short-circuited with `!operator && agent_evidence(std::env::vars(), ...)` and `!operator_override && ...`, so the iterator was never consumed.
    std docs, quoted by the refute seat from https://doc.rust-lang.org/std/env/fn.vars.html: "While iterating, the returned iterator will panic if any key or value in the environment is not valid unicode."
    The failure is rare and loud, and it is not a guard bypass. Base already has the same hazard on non-operator paths (legacy purge, mod.rs:3609).
  fix-shape hint: Read keys from `std::env::vars_os()` with lossy conversion, as picker.rs already does. Only the keys are needed.
- [Nit] tests/integration/operator_ux.rs:817 — `me_init_operator_then_person_pane_commands_work` now passes whether or not `me init --operator` recorded the operator mark. No integration test still covers a persisted mark being honoured by a later person-pane command. [lanes: test-causality-lifecycle]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: Head line 792 starts the FakeHost with `pane("w1:p1", "term-a")`, a plain shell pane with no reported agent.
    Lines 817-826 run `human inbox` and `human thread create` with `&[]` env.
    In `person_guard`, `operator_override` is read only inside `if !operator_override && let Some(evidence) = agent_evidence(...)`. With an empty env and no reported agent, `agent_evidence` returns None, so the result is Ok either way.
    Base ran the same steps with `CODEX_SANDBOX=seatbelt`, so success depended on the mark at mod.rs:4957.
    `operator_mark_files()` is asserted only `== 0` at line 884. Other coverage is unit-level only (tests/cli/cooperative.rs `select(..., true)`, tests/harness/context.rs round trip).
    Status: missing prerequisite shown from source. No failure was reproduced. Lane: test-causality-lifecycle. Introduced by this diff.
  fix-shape hint: Run the post-override steps against `agent_pane("w1:p1", "term-a", "claude", None)` so they are refused without the mark, or assert `plugin.operator_mark_files() == 1` after `me init --operator`.
- [FYI] src/cli/mod.rs:4936; src/cli/mod.rs:4904; src/cli/mod.rs:4933 — `human retry` of a retained person intent returns the frozen Human claim from `replay_selection` before `person_guard` runs. An agent session can complete a person-attributed operation, which contradicts the new TRUST-POLICY sentence. No test covers `human retry` from an agent session. [lanes: correctness, security, api-contract, testing]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: In `derive_selection`, `if let Some(claim) = replay && ... && let Some(historical) = replay_selection(...) { return Ok(historical); }` runs before `selection_from_context`, the only `person_guard` call site for Human contexts (mod.rs:5009).
    `replay_selection` (mod.rs:2989-3008) copies `harness: claim.harness.into()` and reads no environment.
    `validate_actor_harness` is skipped for Retry.
    `preflight_original_intent` (src/cli/retry.rs:610-628) steers an Agent-actor retry to `herdr-threads human retry local:N`.
    The diff adds to TRUST-POLICY A4: "every person-pane command that selects a Human context [is] refused when the caller's environment carries an agent-session marker".
    pre-existing on base: replay_selection early returns at e60ec5eb5 src/cli/mod.rs:4866 and :4902, untouched by the diff. Severity is FYI for that reason. All three seats rated it Nit.
    Limits the seats stated: the replayed body was authored by the person, it needs a retained person intent in the same node journal, pane and instance, and no seat checked whether the daemon refuses the replay downstream.
  fix-shape hint: Either apply the agent-session marker check to Human-claim replays, or add one sentence to TRUST-POLICY A4 exempting replay of a person's own retained intent. Add a `human retry` agent-session test for whichever is chosen.
- [FYI] src/cli/mod.rs:214 — several agent-visible refusals still name `--operator` repair commands. The diff edits only the `legacy purge` and person-guard text, so an agent session is still shown the operator repair command line. [lanes: security]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✗)
  evidence: `unresolved_seat_refusal` (src/cli/mod.rs:214-221) prints "repair with `seat rebind {seat} --pane {pane} --operator`" and is returned by `selected_generation` (:238) for any seat-acting mutation, agents included.
    src/store/seats/resolution.rs:293,298 and src/store/seats/continuity.rs:889 print `human seat resolve --new-seat --operator` and `human seat rebind ... --operator`.
    The reproduce seat also lists src/daemon/health.rs:462,481,499,825, src/store/queries.rs:4589 and src/store/seats/operator.rs:486. Whether agents see those diagnostics was not checked.
    TRUST-POLICY line ~1167 limits the rule to "this one, and `legacy purge`'s agent-evidence refusal".
    pre-existing on base: the same strings exist unchanged at e60ec5eb5. Severity is FYI for that reason. The two confirming seats rated it Should-fix; the refute seat said it drops to Nit if the ruling covers only person-attribution overrides.
    Ground seat dissent: the diff's own text says operator repairs "attribute no person and are not refused in an agent session" and "are unchanged", so repair hints are in contract. Both confirming seats read that same text and still found the literal ruling unmet, so the dissent is addressed and the tally stands.
  fix-shape hint: Decide the scope once. Either state in TRUST-POLICY that operator-repair hints are exempt from the "never name `--operator`" rule, or omit the flag from these refusals when the caller carries an agent-session marker.
- [FYI] src/cli/actor_route.rs:76 — the generic actor-route refusal echoes the caller's argv with `human` inserted. An agent that runs `herdr-threads me init --operator` is told to run `herdr-threads human me init --operator`, so a refusal shown to an agent session still names `--operator`. [lanes: correctness]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: actor_route.rs:76: `format!("person/operator actions require immediate human namespace; use {}", tokens...join(" "))`.
    src/cli/commands.rs:2858-2859 returns it for `InvocationActor::Agent` whenever `requires_human` is true. That covers `MeInit`, `Invite { operator: true }`, `CheckInLifecycle { operator: true }`, Rebind, Retire and Replace.
    The path is not marker-aware. The diff does not touch actor_route.rs.
    No test in tests/integration/operator_ux.rs runs `me init --operator` without `human` from an agent session.
    pre-existing on base: identical `guidance()` and `requires_human` block at e60ec5eb5. Severity is FYI for that reason. Seat severities were Should-fix, Nit, Nit. The reproduce seat called it a gap in the fix, not a regression.
    Impact the seats stated: the agent typed `--operator` itself, and the suggested `human me init --operator` is then refused by `person_guard`. The cost is one misleading hint, not a bypass.
  fix-shape hint: Drop `--operator` from the echoed argv, or word the refusal without a suggested command, when the action is `MeInit` or an agent-session marker is present.

## Not verified (beyond panel cap)   ← severe candidates the panel cap left unverified — listed, never dropped
- none

## Not verified (dedupe failed or judge lost)   ← findings a pipeline failure left unjudged — listed, never dropped
- none

## Beyond remainder cap (count only)   ← low-severity candidates the dedupe remainder cap dropped; the count survives, the claims do not
- none

## Rejected (with reason)        ← so re-roasts don't re-litigate
- src/cli/legacy.rs:157; src/cli/mod.rs:3647; src/cli/legacy.rs:69; tests/cli/legacy_purge.rs:118 — `legacy purge --confirm --operator` is still honoured inside an agent session. Rejected 3/3.
  reason: The facts are accurate. `legacy purge` attributes no person; it records `operator:local-user:<uid>` (TRUST-POLICY line 786).
    The added A4 text says administrative recovery is "unchanged" and lists this refusal only among those that "never name `--operator`". The diff does exactly that, and the test asserts it.
    The `--operator` bypass in `purge_preconditions` is unchanged from base e60ec5eb5.
    Residual, not confirmed: the policy text does not say outright that the purge override stays usable in an agent session.
- TRUST-POLICY.md:1171 — agent sessions can still use `--operator` for operator repairs (`seat rebind`, `resolve --new-seat`, `retire`, `--replace`). Rejected 3/3.
  reason: The diff states the exemption openly: these repairs "attribute no person and are not refused in an agent session" and "are unchanged".
    They were already unrestricted on base (base TRUST-POLICY.md lines 258, 683-684).
    AGENTS.md says "Do not add adversarial caller verification; the model is cooperative and same-user".
    The finding itself concedes the exemption follows the ruling. Extending the guard to seat repairs is a separate policy decision.
- src/cli/retry.rs:627 — the agent-namespace retry refusal spells out `herdr-threads human retry local:N`. Rejected 3/3.
  reason: The message names the `human` namespace, not `--operator` or any override.
    The line is unchanged from base e60ec5eb5 (retry.rs:627). src/cli/actor_route.rs:76 uses the same routing wording.
    Retry replays an intent a person already journaled and creates no new person attribution.
    The related replay-before-guard ordering is reported separately under Confirmed findings (FYI).
- src/cli/mod.rs:3648 — `legacy purge --confirm` detects agents with `agent_env_marker` only, not `agent_session_markers`, so a Hermes or session-id-only session is not refused. Rejected 3/3.
  reason: The TRUST-POLICY sentence constrains what the refusal text says, not which detection purge uses.
    The diff changed no purge detection. Base e60ec5eb5 mod.rs:3609 has the same call.
    `purge_preconditions` accepts `--operator` from any session, so wider detection would only change which refusal fires.
    At most an optional hardening follow-up.
- src/cli/mod.rs:5008 — a person pane that recorded `me init --operator` under an inherited marker is refused for every Human-context command after upgrade. Rejected 3/3.
  reason: This is the stated intent. TRUST-POLICY A4: "A recorded `--operator` mark does not lift it."
    The Refusals accepted limit was rewritten on purpose: "`human me init --operator` is no way out there".
    The refusal names the offending variable, so the remedy (unset it) is actionable. Saying the mark is ignored would name the override, which the ruling forbids.
    At most a release-note item.
- src/harness/registry.rs:1476 — the guard now refuses on many more variables with no override (CODEX_THREAD_ID, CLAUDE_PID, CODEX_SESSION_ID, HERMES_SESSION_ID, ANTIGRAVITY_CONVERSATION_ID, opencode variables). Rejected 3/3.
  reason: The widened set is the registry's neutral marker lookup the change was asked to use.
    TRUST-POLICY A4 *Agent session* lists the same variables, "matched by presence", and states the cost: a person whose shell inherited one "runs the command with it unset".
    The cited design note (core-sweep-design.md:161) concerned the older guard list, not this refusal.
    Plain shells with no marker keep the override.
- src/cli/mod.rs:519 — with no session marker but a Herdr-reported agent in the pane, the refusal still names `human me init --operator` and the override is honoured (external claim about harness env propagation). Rejected 3/3.
  reason: The rule is scoped to callers that carry a marker. `agent_evidence_refusal` is documented as shown only to a caller with no marker.
    TRUST-POLICY A4 calls the guard "best effort, client-side" and accepts that "an agent that unsets the variables is not detected". A harness that exports no registered marker is the same accepted limit.
    The finding names no concrete harness that omits its markers. The ground seat noted the external fact is unverified and that it does not change the verdict.
    Not a regression: on base `--operator` overrode everything.

## Unverified nits (spot-checked)
- [Nit] tests/integration/operator_ux.rs:612 — the final loop of `me_init_refuses_with_claudecode_or_codex_marker` never checks that the operator mark exists before asserting it does not help an agent session. (spot: confirmed at Nit. `operator_mark_files()` exists at line 131 and is not called in this test. Repair: assert it is 1 after `me init --operator`.)
- [Nit] tests/integration/operator_ux.rs:826 — `me_init_operator_then_person_pane_commands_work` dropped the check that a plain `me init` still refuses after the mark is recorded, with no replacement. (spot: confirmed at Nit. me.rs:210 passes only the argv `operator` flag; a change that made `me init` honour `contexts.operator_mark()` would not be caught.)
- [Nit] src/cli/mod.rs:509; src/cli/mod.rs:508 — the env-marker half of `agent_evidence` is dead inside `person_guard`, because every person-guard marker is already in `agent_session_markers` and refused earlier. (spot: confirmed at Nit. `person_guard` is now the only caller of `agent_evidence`. No behavioural effect.)
- [FYI] src/cli/mod.rs:259 — the agent-session refusal names the variable but gives a person with an inherited variable no fix. (spot: refuted. The message names the exact marker, and TRUST-POLICY documents the unset remedy as an accepted limit.)
- [FYI] CHANGELOG.md:1 — user-visible behaviour changed with no CHANGELOG entry. (spot: refuted. CHANGELOG.md has no Unreleased section and is edited only in "Prepare vX" commits. Reminder for release prep.)
- [FYI] src/cli/mod.rs:236 — agent-session detection matches by presence, so an empty value such as `CODEX_THREAD_ID=` refuses, while output-form markers document a non-empty value. (spot: refuted. TRUST-POLICY A4 says "matched by presence"; the non-empty rule still governs output form only; the failure direction is fail-closed.)
- [FYI] tests/integration/operator_ux.rs:828 — nothing asserts what a text `human inbox` does from an agent session in a pane carrying the operator mark. (spot: refuted. `selection_from_context` runs `person_guard` on every Human-context selection, and tests/cli/cooperative.rs ~1510-1530 asserts "human text inbox is content read-only".)
- [FYI] src/cli/help.rs:315 — `me init` help lists `person_guard_markers()` while the guard enforces the wider `agent_session_markers()`. (spot: refuted. The help text adds "or a registered agent harness's session variable set", and the refusal names the offending variable.)
- [FYI] src/cli/mod.rs:484 — the refusal names only the first marker found, so several inherited variables are unset one refusal at a time. (spot: refuted. Inconvenience on a rare path; the guard's outcome is unchanged, and listing every marker would hand agents the list to unset.)
- [FYI] scripts/wake_e2e_selftest.py:966 — the wake e2e driver keeps a hand-maintained `CALLER_SESSION_ENV` and checks it by regex-scraping Rust literals with an opencode `witness.rs` special case. (spot: refuted. The tuple exists on base at scripts/wake-e2e.py:330; the diff adds the drift checks; the list is complete at this head; a miss would fail the release-gate run loudly.)
- [FYI] src/test_support/isolation.rs:215 — the test scrub `ambient_harness_vars` never names `session_marker_vars`, which the new guard refuses on. (spot: refuted. All three current entries are removed by `SCRUBBED_PREFIXES`; source risk only, and a future miss would fail tests loudly.)
- [FYI] tests/integration/operator_ux.rs:595 — the "plain shell keeps the override" assertion runs `human me init --operator` with no agent evidence, so it does not exercise the override. (spot: refuted. The override over a connected agent binding is exercised in `me_init_operator_then_person_pane_commands_work`, and this assertion checks `operator_human` provenance.)
- [FYI] src/cli/mod.rs:480 — a second, presence-based "is this an agent session" check now lives in cli/mod.rs beside caller derivation, and the two can disagree. (spot: refuted. The list is derived from the registry's `env_markers()` and `session_vars()`; a disagreement refuses, which is the intended direction.)
- [FYI] src/cli/mod.rs:259; src/cli/mod.rs:498 — the new refusal uses the generic `invalid_request` code. (spot: refuted. The whole `me init` / person-pane guard family already uses `invalid_request`, including on base; `session_claim_required` covers a different condition; no consumer was named.)
- [FYI] src/cli/mod.rs:248 — the check reads only environment variables, so `env -u ... herdr-threads human me init --operator` passes. (spot: refuted. TRUST-POLICY A4 records this as an accepted limit, and AGENTS.md forbids adversarial caller verification.)
- [FYI] src/cli/mod.rs:483 — `agent_session_marker` hard-codes `builtins()` with no registry-injected variant. (spot: refuted. `Registry::agent_session_markers()` is testable per registry, as tests/harness/adapter_seam.rs:376 does; the wrapper follows the file's existing pattern.)
- [FYI] tests/integration/operator_ux.rs:807 — the test's first refusal now depends on the stand-in Claude binding still being connected, which is not asserted first. (spot: refuted. The same pattern is on base at line 636 and at lines 493 and 531; a disconnected binding fails the assertion loudly.)
- [FYI] src/cli/mod.rs:492 — a refused agent attempt at person attribution leaves no durable record. (spot: refuted. The guard is specified as "best effort, client-side"; no requirement asks for an audit trail; base refusals also log nothing.)
- [FYI] src/harness/registry.rs:1476; src/harness/registry.rs:1472 — `Registry::agent_session_markers` returns a new `Vec` with a duplicated dedup loop where siblings return iterators. (spot: refuted. `foreign_session_vars` at registry.rs:1571 has the same return type and idiom; style only.)
- [FYI] src/cli/mod.rs:481; src/cli/mod.rs:492 — `person_guard` collects every env key into a `Vec<String>` and re-wraps each as `(k, ())`. (spot: refuted. One allocation per CLI invocation, not a hot path; the finding itself reports it "only for completeness".)

## Escalations (need human)      ← UNVERIFIED externals, incomplete panels, unsettled panels, material dissent
- none
---