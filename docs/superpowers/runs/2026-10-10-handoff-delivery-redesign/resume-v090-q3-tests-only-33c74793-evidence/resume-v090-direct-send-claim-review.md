# Scoped DirectSend caller-claim review

SPEC: **PASS**. QUALITY: **PASS (source review; runtime verification pending)**.

Reviewed the proposed two-statement replacement in `resume-v090-direct-send-liveness-proposed.patch` against the candidate at `.worktrees/handoff-delivery-redesign/.worktrees/upstream-composition-v090` (parent-supplied HEAD `16ede4ed2eaf3c24b62077f8f38cb0faf24b23cc`). Read the triage, actual failure log, applicable TRUST-POLICY continuity/attribution/direct-pair rules, and the claim construction, dispatch, freeze and retry paths. No production, index or Git changes; no Cargo, runtime fixture, Herdr or subagent activity. This report is the only write.

## Evidence and intended origin

The actual direct test failure at `/private/tmp/ht-C-v090-repaired-hook6-first/run.log:94` compares identical instance, seat, generation, role, harness, native session, execution, target and agent ID. Only the Ensure claim's `liveness: None` differs from the originating registered sender's `Some(pid: 55010, start: 1791763540598154)`. The fixture explicitly supplies that origin fingerprint through `EnvClaim` (`tests/hook_entrypoint/agy.rs:1490`), so this failure is separate from the changing-parent-PID failures in the other hook tests.

`caller_journal` (`src/cli/mod.rs:248`) builds the agent handle from the active `CallerOrigin::Agent(env)`, the selected instance/seat/target/generation, and `env.fingerprint`. `caller::agent_context` (`src/cli/caller.rs:134`) supplies the environment's harness/native session and its stable session execution. `ContextJournal::effective` prefers that handle's context over saved pane context. The proposed `current()?` therefore obtains exactly the same selected context as the old expression; it does not resolve another caller, recipient, binding or execution.

## All claim branches and effects

`ContextJournal::claim` (`src/harness/context.rs:526`) is concrete, not an adapter dispatch. Its complete behavior is:

1. Call the same `bridge::caller_claim(context)` used before. Invalid format, nil instance/execution, non-top-level context, invalid session or invalid parsed IDs still return the same error.
2. If the handle has an agent and both its session and harness equal the supplied context's, assign `Some(agent.liveness)`.
3. If there is no agent, or either equality fails, return the original claim with `None`.

It performs no filesystem, process-table, environment, host or daemon I/O; no lock, journal write, enrollment, refresh or native operation. The fingerprint is copied from the already derived agent handle. `current()` itself is not pure: it takes/possibly creates `context.lock`, loads and validates `context.json`, then clones the effective context (`context.rs:872`, lock/load at 540/567). That existing I/O occurs with the same count and ordering under this patch, including the earlier actor validation read.

For DirectSend's own agent handle, `current()` returns `agent.current`, so both equality conditions necessarily hold. Human and legacy handles without an agent retain their previous fingerprint-free behavior. The general helper does not compare seat/target/execution before attaching liveness, but this caller supplies the handle's own current context; it introduces no foreign-context attachment opportunity. All remaining CallerClaim fields are constructed by the unchanged bridge conversion (`src/harness/bridge.rs:249`).

## Authority and effect ordering

Actor/harness validation in `run_cooperative` and the top-level role gate (`src/cli/mod.rs:3098`, `3215`) still precede DirectSend dispatch. Bridge validation remains an additional rejection of a subagent context. The patch neither synthesizes top-level authority nor bypasses canonical daemon binding-role checks.

`direct_send::start` still validates the plan/claim, checks modern direct-thread/lazy capabilities, and inspects caller and recipient before recording the frozen intent (`src/cli/direct_send.rs:245`). These existing capability and inspection RPCs precede recording; the claim helper adds no RPC or effect ahead of them. Ensure and Send remain after those preflights. Retry actor/namespace preflight still uses the original read-only intent before state creation/connection in the outer process path (`src/cli/mod.rs:543`), and remains repeated at the cooperative seam. No launch/native behavior changes.

TRUST-POLICY A2 explicitly requires an agent environment/session claim with the harness fingerprint. A4 assigns any resulting fingerprint refresh or permitted reconnect to the daemon's canonical plain-call decision; the helper itself performs neither. Existing A3 provenance and A5 automatic-pair attribution remain intact. A fingerprint may now enable an ordinary canonical touch/reconnect that the dropped fingerprint prevented; that is the intended correction, not new client authority. There is no seat movement, merge, inferred identity, new provenance or weakened invariant requiring a policy amendment.

## Frozen content, digest and replay

`SemanticMutation::freeze` stores the supplied whole claim unchanged (`src/cli/journal.rs:305`). `Journal::record` serializes the semantic; `record_locked` hashes those bytes; loading validates the reserialized semantic digest (`journal.rs:1274`, `1420`, `1653`). New agent DirectSend intents will intentionally contain the fingerprint and therefore have a different semantic encoding/digest than an otherwise identical old fingerprint-free intent. This does **not** mean rewriting any existing digest or retained intent.

`direct_send::retry` loads `original.semantic`, extracts its saved claim, checks namespace/progress and clones that same claim into Ensure and `Plan::send` (`direct_send.rs:369`, `393`, `90`). Progress validates full equality with the frozen Send. Body, pair recipient, invitation creation policy, relay/user-intent fields, deadlines, child operation keys, retained namespace and output handling are untouched. Retry does not call the changed fresh DirectSend arm or replace the claim with a current fingerprint. Old retained `None` claims remain `None`, and new retained `Some` claims remain their original `Some` even after the environment changes.

## Verification boundary

No blocking source defect found. Runtime success is not claimed. After applying the patch outside the frozen run, retain the exact full Ensure and Send claim comparisons in the actual registered-hook control and run focused direct-send failure/retry coverage. In particular, verify a newly frozen `Some` claim survives recovery after current caller data changes, with exact body/child keys and digest preserved; existing `tests/cli/direct_send.rs` already checks exact child-command replay and body retention. Do not weaken the hook oracle or retrofit historical claims to obtain a green result. The separate persistent-harness and Notice-replay proposals are outside this verdict.
