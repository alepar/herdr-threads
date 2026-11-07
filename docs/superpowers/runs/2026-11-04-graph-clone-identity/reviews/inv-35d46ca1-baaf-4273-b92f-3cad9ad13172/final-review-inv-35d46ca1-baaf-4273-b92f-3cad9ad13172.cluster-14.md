# Final review, cluster 14 (src-harness), epic ht-ma6i

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 1b058307490f29ece8bb4e3fe7a862f8356d992f. Both verified with `git rev-parse --verify`. The source guard bound without error. The caller checkout was clean and at the head SHA when last observed. No tests were run; this is a reading review of the head revision through `git show`.

## Cluster contents

Slices 000070 and 000071, 19 added lines and 1 removed, from commit 7a87f5209.

- `src/harness/context.rs`: a `continuity_key` field on `ContextJournal`, a `pub(crate)` builder and a getter. `open` initialises it to `None`, so every handle except the hook's is keyless.
- `src/harness/bridge.rs`: the three check-in producers present the handle's key. `prepare_event_as` moves from `record_check_in_as` to `record_check_in_lineage` with the same `None` lineage it had, `prepare_qualified_turn` adds the key beside its lineage, and `tool_boundary_check_in` sets the field on its non-durable `CheckIn`.

## Trace from the production entry point

- Only producer of a keyed handle: `session_journal` at `src/cli/hook.rs:3124`, the sole `with_continuity_key` caller in the tree. It runs on the hook's `check_in_seat_with_turn` path, and that handle reaches all three bridge producers.
- Gate: `PaneCall::continuity_key` uses `presented_continuity_key`, so a key is sent only for a top-level event with no `agent_id` and only when the daemon advertises `seat.continuity_key_v1`, matching spec 4.2 "Client side". `subagent_boundary` opens its own keyless handle (`hook.rs:3030`), so a subagent presents nothing.
- Freezing: `record_check_in_lineage` returns an existing pending intent before the factory runs, and `get_or_prepare` and `get_or_prepare_qualified` return an existing request first, so a retry replays the key it first recorded. `SemanticMutation::CooperativeCheckIn.continuity_key` carries `#[serde(default, skip_serializing_if)]`, so an intent from before the upgrade decodes as keyless.
- Wire to decision: `to_command` copies the frozen key into `CheckIn`. `decode_request` does not compare the key, as it does not compare lineage or readonly; that is consistent with the existing check, which pins claim, operation and mode. The daemon's check-in path (`src/store/seats/cooperative.rs:1048`) runs `key_decision` and `settle_key`, which binds on `Bind` and `EnrollThenBind`, records on `Conflict`, and returns `Ok` for every other verdict, so a key on any of these three commands cannot fail the hook.
- Digest: a fresh operation on the tool boundary plus the keyed digest domain (`check_in_digest`) means a keyless check-in hashes as it did before.

## Spec and policy checks

- Baseline B1 ("Continuity key: approved"): the `CheckIn` leg this cluster owns is delivered from the real hook entry point. The `ContinuityCheckIn` and `ResolveSeat` legs, the watch claim and the store decision are outside this cluster, for the seam reviewer.
- Baseline B2 and B3: no code in this cluster decides them; the seam reviewer traces them. The only way this cluster touches B2 is the gap listed in the last section of this file.
- Baseline B4: nothing here touches Herdr.
- Spec 4.2 "Variable and syntax" and "Scope" are decided elsewhere: `CallerEnv::continuity_key` parses, and `key_decision` ignores a key on a non-Herdr host. The bridge only forwards.
- Harness-neutral core: both files are neutral and the diff adds no harness name. Model-visible text: none added. Spec 4.7 holds for this cluster.
- `ContextJournal` has no `Debug` or `Clone` derive, so the key cannot leak through formatting or be lost by a copied handle. `with_agent_caller` takes `self` and keeps the field.
- Capability cost on a quiet tool hook: `tool_boundary_check_in` already calls `supports_capability`, so the eager key read in `session_journal` shares that cached read on a daemon that advertises it. On a daemon that does not, each keyed read issues a fresh `Capabilities` call, by design in `site_capability.rs`; that is a mixed-version cost only and belongs to the src-cli cluster.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- `tool_boundary_check_in` presenting the key has no pinning test: the only keyed tool event (`tests/cli/hook.rs:9318-9321`) asserts the claim count, and `sent_keys` is asserted only for start events, so setting the field to `None` at `src/harness/bridge.rs:1374` would appear to survive the suite (found by enumerating every `keyed_herdr` use; the mutation was not run).
- `prepare_qualified_turn` presenting the key has no pinning test: `key_sent_only_when_advertised` drives the lifecycle route through `prepare_event_as`, and the keyed Hermes start tests (`tests/cli/hook.rs:9376-9392`) stop at `resolve`, so dropping the key at `src/harness/bridge.rs:495` would appear to survive (by inspection; the mutation was not run).
- No test in this review was executed; passing status of the cited tests rests on the caller's own runs.

## Deferred OK
- A key missed at the lifecycle event can stay unpresented while the seat is quiet: `tool_boundary_check_in` sends its `CheckIn` only when the digest advanced and returns early on a live mod channel (`src/harness/bridge.rs:1346-1348`), so if a keyed start on a resolved keyless seat goes keyless (capability read `Unknown`) or its probe is kept during lag and later refused as owned, the reserved key binds only at the next lifecycle event or mail, and a pane close in that gap retires finally rather than reversibly; found by code reading and not reproduced, it needs a keyless seat plus an unbound key, which reserve-with-seat steers away from, and the seam reviewer should weigh it against B2.
