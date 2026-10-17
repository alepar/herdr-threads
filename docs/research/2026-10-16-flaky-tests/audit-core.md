# Read-only flaky-test-pattern audit: core and scripts

Checkout: `/Users/alepar/AleCode/herdr-threads/.worktrees/fix-v2-flakes`. No source edits, cargo runs, thread mutations, or dynamic reproductions. Findings below are static candidates, with fault-injection reproduction recipes rather than claimed observed failures. Read project AGENTS.md. Excluded the already identified crashed-daemon startup guard and store-only daemon election/host-attachment defects.

## Findings

### 1. Hermes child-lifetime tests observe request entry but do not hold the child through the tested race (high confidence)

Locations: `integrations/hermes/test_bridge.py:47` (`slow`: sleep .3), `:669-680` (`test_child_results_errors_caps_and_nonblocking_generation`), `:716-739` (`test_reload_cannot_spawn_successor_while_unloaded_bridge_owns_live_child`), `:1148-1161` (`test_unload_during_owned_child_blocks_late_context`).

Exact interleaving: child appends its request, then sleeps .3s; controlling test sees request append, but is descheduled for >.3s before invoking the competing request or unload. The old request successfully returns context before the mutation. The production bridge can behave correctly while the test expects old outcome `[None]`, no successor, or exactly one request and fails. Request-file publication proves entry, not continuing ownership. Child process startup or stdout publication is similarly not a lifetime barrier.

Repro: inject a .4s pause immediately after the test's `assertEqual(len(self.requests()), 1)` and before unload in either unload/reload test, or immediately before the competing call in the nonblocking-generation test. The old successful result is allowed by the fixture. Run the one Python unittest under the existing interpreter/env, not cargo.

Fix: replace `slow` in lifetime-race tests with an explicit child-entered + parent-controlled release handshake; trigger unload/competing invocation while the hold is armed, then release in finally and join. Existing `lifetime_hold` at `:48-50` and usage at `:778-811` is a useful starting point, but its automatic 3s escape must be only a diagnostic failure guard, not a successful release. Its callback deadline must also not expire during the orchestrated race. Keep actual timeout/cap tests separate.

### 2. Hermes refresh-cadence test counts background work against real elapsed time (high confidence)

Locations: `integrations/hermes/test_bridge.py:1133-1142`, `integrations/hermes/__init__.py:219-245`; `setUp` uses `self.clock = None` (`test_bridge.py:208`).

Exact interleaving: `load()` completes the first read; test observes one read and sleeps .1s. If its main thread is descheduled through the reader's real one-second refresh boundary, the background thread legitimately performs the second read. The following assertion `len(self.reads) == 1` fails. There is no product requirement that the controlling thread resume in .1s. Later `iteration = counts + 1` selection can also miss an iteration entered just before the config changes, though awaiting exact published quality already improves correctness.

Repro: replace only this test's .1 sleep with 1.1s; a correct one-second cadence produces the failure. A scheduler pause after the first count assertion does the same.

Fix: use the synthetic injected monotonic clock already supported by `load()` (`:252-255`), keep it fixed to prove no due refresh, explicitly advance it past `next_refresh`, wake the reader, and await exact read publication via existing `await_completion`. This tests the cadence predicate without relying on the speed of the test thread.

### 3. agy live gate's "busy" prerequisite expires while the coordinator performs sends (high confidence)

Locations: `scripts/agy-gate/flows/delivery.py:46-59` (`busy_turn_send`), especially the four sends `:123-128`; `scripts/agy-gate/scenarios/delivery.json:19`, `delivery-cancel.json:19`, `delivery-error.json:19`, `delivery-stop.json:68` (five-second delays); analogous `flows/wake.py:77-81` and `scenarios/wake.json:20` (nine-second delay). Fake server implementation `scripts/agy-fake-gemini/server.py:276-278` records the request before sleeping and automatically replies when the delay expires.

Exact interleaving: observing the named request starts no ownership barrier. A stalled coordinator or slow CLI sequence takes >5s, server returns the busy response, agent goes idle, and a later p0/ACK send legitimately wakes it before the intended injecting turn. The next phase then sees consumed steps, wrong request, or unexpected wake. With four separate CLI sends this can even split a supposedly single pending page across turns. The assertions would implicate delivery despite a violated fixture prerequisite.

Repro: add a >5s test-only pause after `await_step` in `busy_turn_send` or between its sends; watch the fixture reply and recipient become idle before the remaining sends. A >9s pause reproduces the wake phase counterpart. No authenticated/live gate run performed here.

Fix: add fake-server hold/release control keyed to scenario step. Await held-entry, perform all sends, verify recipient remains in that held turn, explicitly release it, then await turn completion. Treat automatic watchdog expiry as infrastructure failure. Increasing fixed delays merely moves the race.

### 4. Release wake-e2e busy case uses activity status plus a three-second sleep as a tool-running proof (high confidence for fixture race; live release gate only)

Locations: `scripts/wake-e2e.py:121-127` (`BUSY_PROMPT`: sleep 12 then sleep 1), `:1119-1134`, `:1188-1199`.

Exact interleaving: non-idle status may mean model generation before the tool, not entry into sleep 12. After status is seen, the coordinator sleeps three seconds and takes `busy_at_send`, then may perform several CLI calls before the actual send/invite. Descheduling or slow CLI makes the turn finish before publication; conversely the send may arrive during generation before either tool exists. A correct idle wake is then classified as a forbidden busy wake, or the claimed tool-running case was never exercised. The eventual `busy_at_send not in IDLE` assertion detects some failures but does not make the fixture valid or correlate activity with send publication.

Repro: insert >14s pause after `busy_at_send = self.status()` and before `send`/invite; the recorded status remains busy while actual publication happens idle. Or delay model tool entry after the first non-idle observation: the test sends before its documented tool prerequisite.

Fix: have the test's owned shell tool publish entered and wait for release; coordinator awaits entered, publishes the tested item and checks correlation, then releases. Keep an end-to-end watchdog as a hang guard. For a real tool's second in-turn delivery hook, explicitly gate/release both stages as needed. Respect the existing release requirement to run authenticated agents unattended; no release gate was run in this audit.

## Lower-priority follow-ups (not confirmed findings)

* `src/client/service.rs:1073-1203`, `queued_register_observes_its_live_budget`, has a real one-second owner request budget, 20ms queued deadlines, 100ms outer completion guards, and a cancellation task triggered by a 5ms timer. The lock-entry and release oneshots are good synchronization. These assertions test budget behavior, but the 100ms guards are not listed in nextest's all-slots timing override. Severe process contention can expire the owner or outer guard even with correct deadline logic. Decide whether 100ms is an explicit performance contract; if yes give the test the same isolated nextest treatment as other measured latency contracts, otherwise use stepped clock/cancel handshakes and an ordinary hang guard. Do not blindly relax an intended performance assertion.
* `src/sync/coordinator.rs:3479-3491` quiet helper sleeps 200ms and drains `try_recv`; it is used throughout paused-time tests, including two-link fixture helpers `:3534-3567`. This is mostly a **false-negative risk**, not evidence of a current flake: a queued off-runtime/blocking worker may not complete before a virtual quiet window ends. Prefer actor-processed fences or explicit worker completion for negative assertions. The neighboring `:3660-3668` explicitly polls real time because blocking jobs cannot be assumed complete when virtual time advances, demonstrating awareness of this boundary.
* `src/service/host_sessions.rs:1495-1505` says several sweep ticks passed after a 100ms wait, but does not observe any tick. Under starvation the absence assertion can pass without a sweep. Positive attach waits afterwards are sound. Use an iteration counter/idle event if verifying missing-socket sweep behavior itself matters.

## Coverage and positive controls

Searches covered neutral `src` (excluding `cli` and `harness` delegated elsewhere), embedded tests, `src/test_support`, all scripts and integrations, and `.config/nextest.toml`. Inspected timing/sleep/cancellation/timeout, process and static-mutex usage, temp/env/port patterns, teardown, virtual-clock boundaries and readiness patterns. This is pattern-based static coverage, not a claim that every source line or every one of the 199 path-included test modules was read; the other auditor covers tests/.

Positive controls reviewed:

* `.config/nextest.toml:1-8` correctly states process-per-test mutex limits; test groups and all-slots overrides already protect known shared scheduling/real latency contracts. No specific uncovered cross-process mutable resource collision was established in the neutral core.
* `src/test_support/server_completion.rs` distinguishes worker entry, worker exit and search permit destructor completion; this is exactly the needed observation/completion separation.
* `src/test_support/isolation.rs` allocates private UUID roots and short sockets, scrubs harness attribution, and isolates HOME/XDG for commands. `scratch.rs` registers scratch paths and SQLite sidecars for cleanup. Constant `/tmp` strings in protocol/path tests mostly represent parsed data; no actionable port collision found.
* `src/test_support/spawn.rs` tags owned children, uses process groups and reaping, and `ran_in_own_process` skips redundant subprocesses under nextest. Existing `OwnedChild::wait_with_output` remains conventional blocking pipe collection; no concrete leaking caller was established, so not elevated to a finding.
* Most coordinator/link time-based tests use `#[tokio::test(start_paused = true)]` and injected clocks. Their exact logical timer assertions should stay; they are not equivalent to arbitrary wall sleeps.
* `src/sync/discovery.rs:866-899` waits for pacer idle and explicitly advances an injected clock; missing fake-time advance is not confused with a real scheduling deadline.
* `src/app.rs` CountingAdapter observer tests carry 50ms budgets but the adapter ignores them and the observer does not enforce wall-time expiry itself. Therefore the superficial 50ms literal is **not** a valid flake finding.
* Native host cancellation latency `src/host/native.rs:3692-3712` has an existing all-slots nextest override; retain its actual performance assertion. Held-channel host fixture tests provide stronger completion controls than delayed responses.

Suggested priority: fix Hermes lifetime barriers first, then Hermes refresh test; treat authenticated live gate holds as a separate scoped fixture improvement. The lower-priority entries need purpose/contract review before changing assertions.

## Coordinator runner follow-up

Read `.github/workflows/ci.yml:98-124` and `full-suite.yml:92-120` plus Cargo target configuration. Hosted shards use nextest process-per-test and the CI profile; the unsharded workflow retains its strict 300s guard. No blanket retry masking or newly established runner resource collision found. These workflow checks supplement the source/tests/scripts scan; they are static configuration review, not a fresh hosted-CI run.
