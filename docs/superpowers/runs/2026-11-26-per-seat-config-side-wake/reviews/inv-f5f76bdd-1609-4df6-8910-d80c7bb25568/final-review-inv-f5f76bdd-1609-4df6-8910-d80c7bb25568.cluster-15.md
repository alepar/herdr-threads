# Final review: cluster 15 (src-harness), epic ht-z41e

Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168
(both verified with `git rev-parse --verify`; the caller checkout was at the head SHA with a clean
status when read, so working-tree reads equal `git show <head>:<path>`). Inputs: slices
000080-000093 from clusters.json (src-harness), digest.md, the spec
`docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-26-per-seat-config-side-wake-design.md`
(it has a `## Requirement baseline`, rows B1-B10 and scope change S1).

## What the cluster changes

- `HarnessAdapter::wake_process_root` (neutral default `None`) and `RootCandidate`
  (`native_session`, `observation_provenance`, `concurrent_session`, `auxiliary_session`), plumbed
  through `ErasedAdapter`/`TypedAdapter`/`Registration` in `src/harness/registry.rs`.
- Codex `wake_process_root`: exactly one candidate with neither bit, every other candidate
  auxiliary; otherwise `None`. Matches spec §Selection "Root marker" bullet 3 case by case (the
  registry unit test covers ordering, fork-side with both bits, non-side fork, two plain, auxiliary
  root, lone root).
- Codex `auxiliary_session(name, value)`: null `transcript_path` on top-level SessionStart,
  UserPromptSubmit, PreToolUse or Stop, none for an absent key, a path, or an event naming an agent.
  PostToolUse is listed in the spec but Codex registers no such hook, so its absence is correct.
- Neutral `LifecycleEvent.auxiliary_session` / `EventMetadata.auxiliary_session`, carried through
  `DecodedEvent::from_native` and `context_event` (round trip tested in tests/harness/codex.rs).
- `src/harness/bridge.rs`: `prepare_event_as` and `prepare_qualified_turn` copy the event's evidence
  onto the journaled claim; `decode_request` masks it before the frozen-claim comparison;
  `cached_event` drops it before dispatch when the daemon lacks `seat.auxiliary_session_v1`.
- Section 3 (B9): `CODEX_PLACEHOLDERS` drops `Ask a follow-up question`; the captured side composer
  now reads `Text(...)`, so a native wake owed while a side chat is displayed is a pre-send refusal.
  The neutral module doc in `src/harness/composer.rs` is updated to match.
- Permission-rule unit tests: Claude asks and Codex prompts on `human config seat set|unset`
  (with and without `--seat`) and allow the agent `config seat set|get|list` verbs (B6 gating).
- Mechanical `auxiliary_session: false` additions in agy, opencode, pi, hermes, claude decoders.

## Tracing the requirement rows that touch this cluster

- B9 (stop typing into a side chat): delivered. The placeholder list is the only input
  `read_composer` uses for Codex emptiness; the side capture reads as text; the native gate
  refusal and retry live in `src/host/native.rs` (another cluster; Task 1's
  `cooperative_wake_defers_on_a_side_composer_and_prompts_once_it_reads_empty`).
- B6 (human-tier writes need the person): the cluster's part is the permission rules the installer
  writes; both harnesses' rules gate the `human` spelling and leave agent verbs allowed. Tier
  enforcement in the daemon/CLI is another cluster's.
- B8 (wakes still go to the root while `/side` is open): the adapter policy is correct, but the
  evidence it depends on reaches the daemon on only part of the production hook paths. The spec
  amendment (code cycle 1) says the marker is read "on every hook event" precisely so that a side
  first enrolled by a later event is not read as a second root; the only accepted limit is a side
  that has emitted no hook event yet.
  Production path traced at the head: Codex PreToolUse, UserPromptSubmit and Stop decode as
  `EventIntent::Current` (`EventKind::Tool`). `src/cli/hook.rs` `check_in_seat_with_turn` sends every
  non-lifecycle top-level event to `bridge::tool_boundary_check_in` (hook.rs:2960), not to
  `cached_event`. `tool_boundary_check_in` (bridge.rs:1271) builds its claim with
  `contexts.claim(&saved)` -> `caller_claim(...)`, which hard-codes `auxiliary_session: false`, and
  never copies `event.auxiliary_session`. It then sends a `Command::CheckIn { mode: Current, claim }`
  whenever the digest advanced (always, with no stored mark, if anything is pending). The store opens
  a binding for an unbound native session on a Current check-in (asserted by
  tests/store/cooperative_checkin.rs `side_first_seen_by_any_hook_event_enrolls_auxiliary_and_never_concurrent`),
  so a side whose first daemon contact is a tool-boundary event gets a `(concurrent 0, auxiliary 0)`
  binding. `mark_auxiliary` (src/store/bindings.rs:1477) is the only writer of the bit and runs
  only on a claim that carries it, so the bit stays 0 for that binding's life. Root (0,0) plus side
  (0,0) makes Codex `wake_process_root` return `None`, the wake goes native, and the native gate
  defers until the side chat closes: the case the redesign was meant to remove.
  The test that claims to cover this (`side_first_seen_by_any_hook_event_...`) goes through the
  test-only helper `check_in_payload`, which copies `event.auxiliary_session` onto the claim by hand
  (cooperative_checkin.rs:3640) and calls the store directly. It never exercises the hook's
  tool-boundary path, so the production route is untested and does not carry the evidence.
  The common path works: a `/side` emits SessionStart `source: fork` with a null `transcript_path`
  (spike S1), which is a lifecycle check-in through `prepare_event_as` and sets both bits. The gap is
  the side whose SessionStart hook did not reach the daemon (hook timeout, daemon down or restarted,
  plugin installed mid-session), which the spike names as the reason for the every-event marker.
  It fails closed (native, never a wrong target), so this is a delivery gap, not a safety defect.

## Other checks

- `scripts/check-harness-neutral` at the head: ok (0 violations). The neutral comments in
  `src/harness/mod.rs` and `adapter.rs` mention Codex and `/side` in prose only, which the checker
  does not count.
- `decode_request` masking is sound: the journaled payload keeps the evidence for replay, and only
  the context-rebuild comparison ignores it.
- `drop_unadvertised_auxiliary` costs no capability round trip for a claim without the evidence, so a
  default caller's bytes and calls are unchanged.
- I did not run tests: this review is read-only and the per-task evidence logs record the unit
  runs. The conclusions above come from reading the code.

## Must fix before landing

- B8 / spec §Selection "Root marker" (amended 2026-11-27): `bridge::tool_boundary_check_in` (src/harness/bridge.rs:1271, called from src/cli/hook.rs:2960 for every Codex PreToolUse/UserPromptSubmit/Stop) builds its Current `CheckIn` claim from `contexts.claim(&saved)` (auxiliary_session hard-coded false) and never copies `event.auxiliary_session`, so a `/side` first enrolled by a tool-boundary event gets a (0,0) binding forever and Codex `wake_process_root` names no root (native fallback while the side is open). Copy the event's evidence onto that claim and apply `drop_unadvertised_auxiliary` before its `require_readonly`/dispatch, then add a test that drives a real side PreToolUse payload through the hook's tool-boundary path (not the hand-copying `check_in_payload` helper) and asserts the binding's auxiliary bit.

## Untested scope

- The production hook tool-boundary path for auxiliary evidence: the only "side first seen by a later event" test (tests/store/cooperative_checkin.rs:3681) injects the flag through a test helper and bypasses `tool_boundary_check_in`.
- No end-to-end hook test shows a side's SessionStart fork check-in via `cached_event` setting both bits in the store; the bridge copy is covered only by reading and the decode round-trip test.
- `drop_unadvertised_auxiliary` inside `cached_event` against a daemon without `seat.auxiliary_session_v1` is unit-tested in site_capability.rs only, not through the bridge dispatch.

## Deferred OK

- Neutral-core prose in src/harness/mod.rs and src/harness/adapter.rs names Codex and `/side` in doc comments; the harness-neutral check passes, but the wording could be made generic.
- The spec lists PostToolUse among the classified events; Codex registers no PostToolUse hook, so the implementation correctly omits it. The spec wording could be corrected.
- `RootCandidate.observation_provenance` is passed but unused by the Codex policy; harmless, kept for adapters that may need it.

## Verdict
not ready (B8: Codex tool-boundary check-ins drop the /side auxiliary evidence, so a side first enrolled by PreToolUse/UserPromptSubmit/Stop reads as a second root)

## Must fix before landing
- B8 / spec §Selection "Root marker": src/harness/bridge.rs `tool_boundary_check_in` (line 1271, reached from src/cli/hook.rs:2960) sends a Current CheckIn whose claim never carries `event.auxiliary_session`, so a /side first enrolled by a tool-boundary event gets auxiliary_session=0 permanently and queue routing falls back to native; copy the evidence, gate it with `drop_unadvertised_auxiliary`, and test it through the real hook path.

## Untested scope
- Hook tool-boundary path for auxiliary evidence is untested; tests/store/cooperative_checkin.rs:3681 hand-copies the flag via `check_in_payload` and calls the store directly.
- No end-to-end hook test of a side SessionStart fork check-in through `cached_event` setting both binding bits.
- `drop_unadvertised_auxiliary` in `cached_event` against a daemon lacking `seat.auxiliary_session_v1` is unit-tested only in src/cli/site_capability.rs, not through bridge dispatch.

## Deferred OK
- Doc comments in neutral src/harness/mod.rs and src/harness/adapter.rs name Codex and /side; the harness-neutral check passes, but the wording could be made generic.
- Spec lists PostToolUse among classified events, but Codex registers no such hook; the omission is correct and only the spec wording drifts.
- `RootCandidate.observation_provenance` is unused by the Codex policy; harmless.
