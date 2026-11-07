# Final review, cluster 8 (src-cli), epic ht-ma6i

## Scope and evidence

Reviewed the ten `src/cli` slices (caller, caller_location, commands, hook, journal, launch, me, mod, retry, site_capability) at head 0755e609de0328e6d73584dd65eb2854e1fd7bf7 against base cf13ed76c1c2871b321ad290d03229cb785c400e and the spec `docs/superpowers/runs/2026-11-04-graph-clone-identity/2026-11-04-graph-clone-identity-design.md`. Both SHAs verified with `git rev-parse --verify`. The caller checkout stayed clean at the reviewed head throughout. Other sources opened only to check a named risk: `src/client/local.rs`, `src/store/seats/{continuity,resolution,key_verdict}.rs`, `src/cli/{watch,seat_resolution,topology_runtime}.rs`, `TRUST-POLICY.md`.

This was a reading review: no build or test ran, and each trace below is from code at the head SHA. No repro was run for the three defects; each is a direct reading of the named lines.

## Requirement baseline trace

- B1 (continuity key): client side traced from the real entry. `main` marks the process entry, `seat_filter` then admits the variable, `check_in` calls `reattach_by_continuity`, `continuity_intent` freezes the key, and `submit_continuity` sends `ContinuityCheckIn` with it. A fresh non-resume session reaches this through `SAME_SESSION_CONTINUITY_V1`. Enrollment (`resolve_seat`), check-in (`with_continuity_key` then `bridge.rs`) and the watch claim (`watch.rs:1291`, `CallerEnv::for_site`) all carry it. The daemon's decision on the key is for the store and seam reviewers. Delivered in the normal path; the first defect below is an exception under a transient read failure.
- B2 (pane close retires, graph undo reverses): no CLI producer is involved. Close and the service operations leave this cluster, so that path is the seam reviewer's to trace.
- B3 (agent exit keeps seat and memberships): nothing here changes it.
- B4 (Herdr unchanged): this cluster adds no Herdr call or requirement.

## Client path analysis

Sound on the paths checked:

- A key refusal stays quiet. `api_failure` matches `ContinuityKeyPending` and the `KEY_CONFLICT_CAUSE` prefix, and the hook's stdout stays empty. Alert-only conflicts answer `TargetAlreadyOwned`, so the Resolved branch proceeds to the owner's check-in.
- A keyed `ServiceBusy` stays quiet on both the claim (`pending_failure`) and enrollment (`keyed` in `check_in`), with no keyless retry in that invocation.
- `enroll_pane` lacks the `keyed` branch, but `HookOutcome` there has empty stdout and only a diagnostic, so nothing reaches the model.
- `ServiceAllocation` intents discard every correlated rejection other than `UnknownOutcome`, so `ContinuityKeyPending` and conflict refusals leave no intent. `ContinuityKeyPending` is added to `is_continuity_refusal`, and `TargetUnresolved` was already there, so a claim refusal completes its intent.
- The journal fields are `serde(default, skip_serializing_if)`, so older intents still load. A changed key supersedes a retained continuity intent.
- A keyless caller makes no extra round trip (`may_present_key`). The capability read is cached per client.
- Named and terminal sites: the daemon ignores the key on a non-Herdr host (`key_decision`), and `is_own_pane` is false for them.
- `launch` and `handoff` pass `None`, and `CONTINUITY_KEY_ENV` is on `child_env_scrub_list`.

## Verdict
not ready (a failed capability read mints a keyless seat; own-pane check ignores the host; subagent gate is dead on seat resolve)

## Must fix before landing
- B1, `src/cli/site_capability.rs:50` with `src/client/local.rs:54-60` and `src/cli/hook.rs:2132`: a failed capability read is treated as "not advertised", so a keyed hook falls back to keyless enrollment and mints a second seat. `capabilities()` caches `Capabilities::none()` for any transport error for the client's lifetime, and its own comment says a caller that must tell a failed call from an answer uses `call_definitive(Command::Capabilities)`. The connect sub-budget is 2 s, so that one read can fail while later calls succeed. Failure trace: a clone relaunched in a new pane after un-retire, first Capabilities read times out; `reattach_and_install` returns `Declined` on the missing `SAME_SESSION_CONTINUITY_V1`; the pane is Unowned, so `check_in` runs `resolve_seat` with `key = None`; the daemon mints seat T while key K stays bound to unresolved S. Every later event then gets `pane_owned_by_other_seat`, and the clone has two identities until an operator acts. This contradicts spec 4.2 "never a keyless fallback enrollment" and TRUST-POLICY C1. Fix: when the environment carries a valid key, read the capability definitively and on a failed read end the hook quietly with no enrollment, keeping a no-capability answer as the only keyless path.
- `src/cli/caller_location.rs:343`: `is_own_pane` compares only the pane id, not the Herdr server, so the key is presented for another server's pane with the same id, against spec 4.2 "`ResolveSeat` presents the key only for the caller's own pane". Failure trace: a keyed clone in `w1:p1` runs `--host-endpoint /other.sock seat resolve w1:p1`, or a recipient allocation for that pane (`src/cli/mod.rs:1315`, `src/cli/mod.rs:3924`). `recipient_host` picks the other server, so `key_decision` judges K at (other host, `w1:p1`) and returns `key_live_elsewhere`. The command is refused and a hold-bearing conflict row lands on an unrelated pane, where ordinary resolution stays refused and quiet in a hook until the row clears. With a reserved unbound key the same call binds K to the other server's seat. Pane ids like `w1:p1` collide naturally across servers. Fix: also require the resolve's host to be the caller's own server (`CallerEnv::pane_host` already spells it canonically), with a test.
- `src/cli/mod.rs:1315` with `src/cli/caller.rs:108` and `src/cli/mod.rs:1678`: the subagent gate is dead on `seat resolve`, so a subagent presents the parent's key, against TRUST-POLICY C1 "only for a top-level, non-subagent caller". The `ActiveGuard` lives inside `run_caller_scoped_deferred` and drops when that function returns `Ok(Some(parsed))`; `MutationSpec::Resolve` has `CallerNeed::None` and is handled afterwards at line 1306. `active()` is therefore `None` and `active_is_top_level()` always answers true there. Failure trace: a subagent runs `seat resolve <own pane>` and can bind a reserved key or write a conflict row and alert as if the top-level session had presented it. The direct harm is small, since spec 4.2 calls a same-pane child harmless, but the comment at `src/cli/site_capability.rs:55` is false on this path and the helper test passes the flag as a literal bool, so nothing catches it. Fix: compute the origin for this branch (`derive_refusing_origin` is already called at line 1066 and its result discarded) and pin it with a test that goes through the branch.

## Untested scope
- No test drives `seat resolve` (`src/cli/mod.rs:1315`), `allocate_recipient_seat` (`src/cli/mod.rs:3924`) or `me init` (`src/cli/me.rs:60`) with a key: coverage stops at the helper with literal bools and at `resolve_result` with a scripted reply, which is how the dead subagent gate went unseen.
- No test drives `enroll_pane` (Hermes SessionStart) with a key, neither its `ContinuityKeyPending` and conflict refusals nor `ServiceBusy`; the keyed hook tests take the lifecycle path through `check_in`.
- No test covers a Capabilities read that fails while later calls succeed, or a key presented with `--host-endpoint` naming another server.
- The plain CLI against a real daemon and a keyed hook through the real binary are outside this cluster's tests; `key_bound_elsewhere_mints_no_seat_on_plain_cli` uses a scripted reply. Whether the integration clusters cover the end-to-end path is the seam reviewer's to confirm.

## Deferred OK
- `api_failure` recognises a conflict by the `KEY_CONFLICT_CAUSE` prefix of `error.detail`. The constant is shared, so this works today; a typed code would be sturdier if another refusal ever needs the same treatment.
- A keyless agent starting on a pane held by a conflict row also takes the quiet path and stays seatless with no model-visible reason. This fits "no new model-visible text", and the service sees the row through the alert feed.
- `tests/cli_env_read_scan.rs` does not list `HERDR_THREADS_CONTINUITY_KEY`, so the one-helper rule is not policed there; adding it is one line.
