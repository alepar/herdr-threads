# Final review: cluster 10 (src-harness), epic ht-ma6i

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 0755e609de0328e6d73584dd65eb2854e1fd7bf7. Both resolve as commits. The caller checkout was at the head and clean when read, and it stayed on that head throughout. Inputs: `slices/000035.diff` (`src/harness/bridge.rs`), `slices/000036.diff` (`src/harness/context.rs`), `slices/000037.diff` (`src/harness/hermes.rs`), the digest and the root spec. All code was read at the head with `git show`. Nothing was built or run: this is a read-only trace, and no test result here is my own.

## What the cluster changes

The three files carry the client half of spec 4.2 ("Client side") on the path from the hook to `CheckIn`:

- `ContextJournal` gains an optional continuity key, set by `with_continuity_key`, that the handle's check-ins present. The field defaults to `None` in the only constructor, so every handle built elsewhere stays keyless.
- `prepare_event_as` and `prepare_qualified_turn` freeze the key with the claim, through `Journal::record_check_in_lineage`.
- `tool_boundary_check_in` sends the key on its non-durable `Current` check-in.
- The Hermes `setup_scope` text tells a launcher to list the variable in `terminal.client_env_passthrough`.

## Trace from production entry points

Baseline row B1 ("Continuity key: approved"), for the part inside this cluster:

- The only production call to `with_continuity_key` is `session_journal` (`src/cli/hook.rs:3088`). The only top-level route into it is `check_in_seat_with_turn` (`hook.rs:2839`), reached from the hook entry point. Subagents and unknown roles return earlier in that function (`event.role != Role::TopLevel`), and the key helper at `hook.rs:2427` gates on `event.role == Role::TopLevel && event.agent_id.is_none()` as well. The spec's "top-level, non-subagent" rule therefore holds twice.
- That handle reaches all three check-in sites. A lifecycle event goes `lifecycle_check_in` to `run_hook_event_reporting_notices` to `cached_event` to `prepare_event_as`. A Hermes turn goes `lifecycle_check_in` to `run_qualified_hook_event_reporting_notices` or the direct `prepare_qualified_turn` call at `hook.rs:3415`. A routine tool event goes to `tool_boundary_check_in` at `hook.rs:2867`.
- Capability gating is done once, in `presented_continuity_key` (`src/cli/site_capability.rs:43`), before the handle is built. `with_continuity_key` documents that the caller decided. A keyless caller asks the daemon nothing, and a capability answer is cached per client, so a keyed hook adds at most one round trip per process.
- Freezing is correct for retries. `record_check_in_lineage` returns the existing intent for a repeated (scope, event), and the semantic mutation carries the key with `#[serde(default, skip_serializing_if = "Option::is_none")]`, so a pre-upgrade intent still decodes and a keyless one keeps its bytes. `cli/journal.rs` restores the key onto both `CheckIn` and `OperatorCheckIn`, and the daemon's `check_in_digest` uses its own domain for a keyed claim, so a replay under one operation id keeps one digest.
- The tool boundary's fresh `OperationId` means no replay concerns there. On the daemon side `cooperative.rs:1048` commits rather than refuses: `settle_key` binds or records a conflict in the same transaction, and `NotReady` and `Pending` write nothing. A keyed check-in therefore never adds a hook failure, which is the spec's "the hook does not fail" rule.
- The keyless handles are deliberate. `seat_contexts` callers in `cli/mod.rs`, `me.rs`, `watch.rs`, `summary.rs` and `wake_on.rs` stay `None`. The spec feeds the plain CLI only on its seat-resolving path, and the watch's claim presents through `watch_continuity`, which is outside this cluster.

Hermes:

- `integrations/hermes/__init__.py` `PANE_ENV` now holds the variable. In a gateway session the bridge strips the gateway's own value and takes the session's from the client env, and a classic CLI inherits its own environment. The child's `CallerEnv` then supplies the key through the trace above.
- A Hermes child decodes as `EventRole::Subagent`, and an unrecognised role as `EventRole::Unknown`, so neither presents a key.
- The new `setup_scope` sentence is accurate, it matches `integrations/hermes/README.md`, and `tests/fixtures/golden/help/setup.txt` was updated with it. It is operator-facing setup help, not model-visible delivery text, so the token rules add nothing. `BRIDGE_SCHEMA` is untouched, correctly, because it hashes the callback schema rather than the environment list.

Rows B2, B3 and B4 have nothing in this cluster. Retirement, memberships and Herdr are untouched by these three files. How the client and daemon halves of B1 compose is the seam reviewer's.

## Other risks checked

- Harness neutrality: the additions to `bridge.rs` and `context.rs` name no harness, and the variable's name appears only in the adapter-owned `hermes.rs`.
- A quiet tool boundary sends no key, because `tool_boundary_check_in` returns before the check-in when the digest has not advanced or a live mod channel serves it. That matches the spec, which binds at a check-in. The lifecycle or first-turn check-in presents the key first, and the spec's required order reserves before launch.
- `decode_request` does not compare the key against anything, because `OccupantContext` carries none. The payload comes from the same-user intent journal, which the cooperative trust model accepts.
- `record_check_in_as` now has one caller, `record_check_in`, and passes `None`. It is correct and harmless.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- `tool_boundary_check_in` carrying the key has no test: every keyed hook test in `tests/cli/hook.rs` drives `SyntheticStart` (`start_input`, `subagent_sends_nothing`), so replacing `src/harness/bridge.rs:1374` with `None` would apparently pass; this comes from reading the tests, not from running a mutant.
- `prepare_qualified_turn` freezing the key has no test: no keyed Rust test drives a qualified turn and `tests/integration/hermes_enrollment_smoke.rs` has no key, so replacing `src/harness/bridge.rs:495` with `None` would apparently pass, and that is the site a Hermes turn's check-in uses.
- The Hermes chain is covered only in pieces: `integrations/hermes/test_bridge.py` pins the bridge's environment and stops at a stand-in child, and nothing runs the hook behind a gateway session's client env; a live Hermes gateway is left to the source design's section 6 qualification, as the spec states.

## Deferred OK
- A stale installed Hermes bridge has the old `PANE_ENV`, so a gateway started from a keyed pane leaks its key into other panes' hook children and those panes can record `key_live_elsewhere`; the README and setup text already say to rerun setup after upgrading and `setup-status` reports `bridge_current`, so this is a narrow upgrade edge rather than a defect in the shipped bridge.
- For the seam reviewer, unverified for reachability and outside this cluster: `LocalSocketClient::capabilities` (`src/client/local.rs:54`) caches `Capabilities::none()` for the process on any failed read, so a keyed hook whose capability read fails while a later call succeeds would go keyless, which is harmless on a check-in here but could allow a keyless enrollment on the `hook.rs:1941` path.
