# Pi extension and channel research candidates

Scope: F1/F3. Research-only findings from current upstream documentation and source, retrieved 2026-11-14. These are historical observations, not version admission gates. No runtime experiment was performed. Candidate evidence identifiers and exact extracted passages are in `evidence-candidates.jsonl`; the numbered raw extraction files retain source context for reconciliation.

## Extension lifecycle and installation

The upstream [extension documentation](https://raw.githubusercontent.com/earendil-works/pi/main/packages/coding-agent/docs/extensions.md) describes TypeScript modules loaded inside the pi process. Global discovery is under `~/.pi/agent/extensions`; project discovery under `.pi/extensions` requires project trust. Explicit `-e` paths support isolated tests. Settings also accept extension paths and packages. Setup should install an owned extension under an explicitly isolated agent config; do not install to the real user config.

The current lifecycle recreates extension instances across replacement: `session_shutdown` cleans the old runtime, then `session_start` initializes the replacement. Reasons distinguish startup/reload/new/resume/fork. Older `session_switch` event recipes are stale for this source snapshot. Run-scoped events include `before_agent_start`, `agent_start`, `turn_start`, `turn_end`, `context`, tool hooks, `agent_end`, `agent_before_settle`, and `agent_settled`. A run can continue after `agent_end`; `agent_settled` is final notification, with `ctx.isIdle()` true. Recreate watch ownership on every start; stop resources and invalidate pending asynchronous callbacks on shutdown.

## Delivery and wake mechanics

The [upstream external-trigger example](https://raw.githubusercontent.com/earendil-works/pi/main/packages/coding-agent/examples/extensions/file-trigger.ts) demonstrates an asynchronous filesystem callback invoking `sendMessage` with `triggerTurn: true`. This is direct evidence that a terminal extension can initiate a model run without composer input. Its illustrative watcher is not a production cleanup pattern: it never closes its watcher.

[AgentSession source](https://raw.githubusercontent.com/earendil-works/pi/main/packages/coding-agent/src/core/agent-session.ts), `sendCustomMessage`, gives four distinct paths:

- `nextTurn` queues until a future run starts, without starting one.
- Busy plus trigger omitted/true uses `agent.steer` (or `followUp` when selected).
- Idle plus true calls `_startRun`.
- False while busy queues a custom append until current tool results are recorded; idle false appends immediately.

`_startRun` marks active synchronously, runs `_prepareRun`, and emits `before_agent_start` for custom messages. The current source therefore contains the path missing in historical triggerTurn bug reports. Do not rely on a version string to assume it works at runtime. A bridge should observe supported callbacks and degrade only the affected feature.

The [agent loop](https://raw.githubusercontent.com/earendil-works/pi/main/packages/agent/src/agent-loop.ts) checks steering after the current tool batch, adds those messages before the next provider request, and awaits message callbacks. Steering continues the existing run; it may cause another model request when the run otherwise would have finished. Follow-up is later, after the natural loop runs out. Therefore steering is the strongest API match for prompt in-turn delivery; triggerTurn false avoids continuation but can leave mail unseen by the model until another natural request. This is a design tradeoff, not an API equivalence.

## Honest acknowledgement boundary

[Public API types](https://raw.githubusercontent.com/earendil-works/pi/main/packages/coding-agent/src/core/extensions/types.ts) declare ordinary `pi.sendMessage` as returning `void`. `_bindExtensionCore` catches its asynchronous failures as runtime errors. A successful function return proves neither acceptance nor display.

The extension `message_end` callback runs **before** SessionManager appends that message; later handlers may replace it. Conversely, the idle non-trigger append emits to public session listeners directly, without invoking extension `message_end`. Neither callback-return shortcut is a universal receipt predicate.

[SDK wiring](https://raw.githubusercontent.com/earendil-works/pi/main/packages/coding-agent/src/core/sdk.ts) connects the agent's pre-request transform to extension `emitContext`. A possible stronger, cooperative predicate is: in a current-session `context` callback, locate the exact offered frame marker and complete unchanged body in the messages, and match its current-branch custom entry through the read-only session manager. Record receipt only for such observed IDs, never the whole queued batch. This is a recommendation requiring design review, not an upstream acknowledgement guarantee.

[SessionManager](https://raw.githubusercontent.com/earendil-works/pi/main/packages/coding-agent/src/core/session-manager.ts) exposes `getEntry`, `getBranch`, `getLeafEntry`, and `getEntries`. Accepted in-memory entries are not proof of disk flush: `_persist` can defer the initial file until user/assistant conversation exists, and no-session operation omits disk persistence. Bound lookup work and scope it to the current branch and exact session; avoid full-history scans every hook.

A context observation still precedes later extensions, conversion, and provider transport. Those downstream loss windows must remain explicit accepted limits. The extension cannot honestly promise final model consumption. Trigger/body separation is feasible in principle: enqueue content as `nextTurn`, and submit a content-free custom wake only when daemon-approved idle eligibility holds; `_prepareRun` combines both. However its later-handler loss and shutdown/replacement races still need tests and documented receipt semantics.

## TUI safety and capability gaps

[Interactive source](https://raw.githubusercontent.com/earendil-works/pi/main/packages/coding-agent/src/modes/interactive/interactive-mode.ts) handles custom message-start by appending chat and requesting render. That path does not modify editor text. Its agent-start handler resets pending tools and retry escape handling. This supports a channel integration preserving composer drafts, but is not an empirical proof for every modal/custom editor.

The public UI API has `getEditorText` but no global modal/blocked getter. Built-in selectors and extension dialogs track blocked state internally. `ctx.isIdle()` describes run/compaction state, not whether a model picker or permission dialog is open. Channel wakes therefore avoid typing into a modal, but do not automatically honor a product rule that all modal states block new model runs. If that rule applies to channels, a reliable public predicate or explicit accepted limit is needed; do not infer safety from `isIdle` alone.

Current `ExtensionContext` has mode `tui|rpc|json|print`; `hasUI` is true for TUI **and RPC**, so it is not a terminal predicate. No public subagent role field was found in that interface. TUI-only enrollment is a useful first-scope guard, while arbitrary custom SDK child runtimes require explicit cooperative policy.

## Remaining gaps and suggested tests

- Actual idle p0 wake, silent p1, busy steering, final-tool boundary, abort, retry, compaction, and direct receive without inbox.
- Draft, model picker, nested permission dialog, and custom editor under channel wake; verify no unwanted editor/focus changes.
- Watch reconnect/replay with identical frame IDs, session new/resume/fork/reload, stale callback delivery after shutdown, and current-branch matching.
- Void send failure, later message/context transformer removal, custom-only first session, no-session, and provider transport failure: no invented ACK authority.
- Feature observation for older pi APIs, including absent `agent_settled`; no exact-version gate.

Retrieval: one sandboxed extract failed with a network error; web fallback opened two documents, then approved CLI-only search extraction succeeded for eight upstream source/doc files. No independent searches were executed by this worker; discovery and historical counterevidence searches belong to the lead. No hostnames, credentials, or machine-specific paths were added to findings.

## Concrete bridge recommendation and busy race

Prefer ACK after exact current-session/current-branch frame observation in a context callback. Do not ACK an idle stored custom entry alone: it may never enter a model request. Retain an offered-ID map bounded by daemon page/offer limits; remove only exact observed current-frame entries after successful receipt settlement. Shutdown cancels every pending frame and asynchronous callback via a per-extension-instance token. Replay deduplication remains keyed by daemon offer identity, not text similarity or a transcript-wide scan.

For busy mail, call steering only when a run is positively active. In one synchronous JavaScript segment, recheck the current instance/session and run state immediately before `sendMessage`; do not await between the check and submission. `!ctx.isIdle()` alone is insufficient: the source says idle is false during compaction even when `isStreaming` is false, and triggerTurn true on the latter path starts a run. Track run-start/settled and compaction state from observed lifecycle callbacks; hold mail during manual compaction or ambiguous phases. If callback ordering cannot establish active-run state, stage content and wait for a natural context boundary instead of guessing. A final-response steering message can add one request inside the existing run; test this explicitly and distinguish it from an idle wake.

For eligible idle wake, stage body as `nextTurn`, then submit a content-free custom wake with true only after rechecking daemon-approved wake eligibility and current session/idle state. A single flight fence prevents duplicate wake triggers. For idle silent mail, keep it staged and do not ACK it until a later actual context offer. A failure after staging but before wake leaves pending mail, not a receipt. The `nextTurn` queue can also be consumed by a later custom run (not only typed user input in the actual source), so session/offer fences must survive any trigger source. These are design recommendations derived from EXE11, EXE14, EXE17–EXE19; they require tests before adoption.
