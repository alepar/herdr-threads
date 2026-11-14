# Credential-free Pi runtime probe

Historical specimen: official npm `@earendil-works/pi-coding-agent` 1.5.2, observed 2026-11-14, Node 22.23.1. This is evidence, never an admission/version gate. `package-lock.json` records the resolved package integrity. Production code was not changed.

`probe.mjs` uses the real SDK, extension runtime, agent loop and file-backed session manager. Only provider auth preflight and the provider stream are replaced, with a manually released fake stream. No live key, provider request, Herdr service or real profile is used. `trace.jsonl` records event order and active-branch custom-message types; each type/body is unique in this probe. `native.mjs` separately launches the actual packaged CLI in RPC mode with a minimal environment and sends only `get_state`.

| Operation | Observed storage/context behavior |
|---|---|
| Idle `sendMessage`, `triggerTurn:false` | Returns `undefined`; stored synchronously before return. SDK message events occur after storage. Extension `message_end` does not fire. No run starts. |
| Busy `triggerTurn:false` | Returns `undefined` with no stored entry. Message appends after extension `turn_end`, before `agent_end`; SDK custom message events follow storage. No additional provider request occurs in that run. Thus an accepted session entry is not evidence of same-turn model consumption. |
| `deliverAs:nextTurn`, false | No stored entry at first settlement. A subsequent user prompt delivers it; both extension and SDK custom `message_end` see it absent from storage. The following `context` sees it present. |
| Busy `steer`, true | Queue acceptance returns before storage. Both extension and SDK `message_end` see it absent; following `context` sees it stored, and another provider call consumes it in the existing run. |
| Busy observation retained across settlement, then `steer`, true | The delayed call starts a new run. `before_agent_start` carries the custom message. A stale busy observation is unsafe for a passive item. |
| End versus settled | `agent_end` callbacks see `isIdle=false`, `isStreaming=true`. `agent_settled` sees `isIdle=true`, `isStreaming=false`. |

The event path and idle append path have different persistence order and different extension event visibility. Never use the void API return or extension `message_end` as a delivery receipt. Match a finalized active-branch `custom_message` entry by session, unique nonce and complete body. For a stronger cooperative delivery claim, also require a later context containing the exact finalized frame; this still does not prove provider receipt or model consumption. A later extension can transform the message before persistence; this probe observes pre-persistence callbacks but does not install a transformer.

A synchronous `ctx.isIdle()` check and immediate `sendMessage` in one JavaScript continuation can avoid the tested asynchronous stale-snapshot race. Do not `await` between classification and injection. Cross-process poll results cannot themselves supply that guarantee. `triggerTurn:false` is safe against unintended wake across busy/idle changes, but while busy may miss the current run's final model request. This is a concrete gap for the project's strict in-turn delivery contract.

Isolation: SDK `getAgentDir()` honored `PI_CODING_AGENT_DIR`; HOME and all session/config/cwd paths were private probe subdirectories. The SDK generated a session ID and JSONL session path; no session-ID environment variable was needed or injected. Native RPC boot succeeded without credentials, reported `isStreaming:false` and a session ID, and exited after owned SIGTERM (exit 143). Its environment contained only HOME, PI_CODING_AGENT_DIR, PATH and TERM. Observed runtime is a Node CLI, not a shell wrapper. No TUI composer, busy tool, compaction, reload/resume/fork or process-table classifier was exercised; RPC environment does not prove all native launch environment behavior.

Reproduce: install the package locally from the lockfile, run `node probe.mjs` and `node native.mjs` through the project's required heavy/toolbox wrappers. The harness writes private scratch under this directory. After capture, remove scratch and node_modules. No owned child or daemon remains. The npm cache location was not changed by the initial toolbox install; no real Pi profile was used.

Official sources checked: [extension documentation](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/extensions.md), [SDK documentation](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/sdk.md), [agent-session source](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/src/core/agent-session.ts), [config source](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/src/config.ts). Local installed declarations/source were inspected before constructing probes; relevant functions are `sendCustomMessage`, `_appendCustomMessage`, `_flushPendingCustomMessages`, extension `sendMessage` binding, and `getAgentDir`.

## Boundary follow-up

`boundary.mjs` and `boundary-trace.jsonl` exercise the installed official boundary API with nine cases and assertions. This supersedes the earlier conclusion that a busy delivery must use the native steer queue: a local pending buffer can instead propose sorted `custom_message` drafts at a boundary and ask for continuation.

| Case | Provider requests | `before_agent_start` calls | Result |
|---|---:|---:|---|
| Completed `turn_end` drafts + continue | 2 | 1 | Exact sorted batch stored and in next context/request |
| Completed `agent_before_settle` drafts + continue | 2 | 1 | Same; continuation remains within original outer run |
| Provider `error` response; drafts at pre-settle | 2 | 1 | Unexpected continuation despite outcome error |
| Provider `aborted` response; drafts at pre-settle | 2 | 1 | Unexpected continuation despite outcome aborted |
| Abort requested inside pre-settle handler | 1 | 1 | Drafts persisted; continuation suppressed; settled aborted=true |
| Provider error; drafts at turn_end | 1 | 1 | Drafts persisted; no next request |
| Provider aborted; drafts at turn_end | 1 | 1 | Drafts persisted; no next request |
| Actual `session.abort()` during provider response | 1 | 1 | No pre-settle proposal; no batch persisted |
| Error plus explicit completed-outcome guard | 1 | 1 | Handler skips proposal; no batch persisted |

Every case also injects an exact startup message from `before_agent_start`; it is in persisted branch and first model context. In completed cases the first handler proposes a batch sorted p0,p1,p3. The next registered boundary handler sees that order in `event.entries` and its rebuilt preview context, while the real branch still contains only startup. The next ordinary context sees the exact batch bodies in the same order in both actual branch and context; assertions enforce this. No new `before_agent_start` fires during continuation, so the probe establishes continuation inside one outer run, even when a second low-level run is used at pre-settle.

Important discrepancy with current extension docs: error/aborted provider responses are hard exits for `turn_end` continuation, but a pre-settle proposal appending custom context can make `_buildBoundaryContext().canContinue` true and allow continuation. Actual cancellation via `session.abort()` is different and prevents it. Guard both boundary handlers with `event.outcome === "completed"`, and preserve pending delivery on every other outcome. Do not treat pre-settle draft commit as proof of consumption; cancellation during that handler persists the batch without another model request.

Viability: **yes, for completed active runs**, this is a stronger approach than native body queues: sort the locally buffered batch, return boundary drafts and a guarded one-shot continuation, then prove finalized branch+context identity before consumption receipt. No stale busy snapshot needs to trigger a new run. Idle arrivals still need a separate priority-aware wake decision; drafts arriving after the final boundary wait until the next run. ACK/receipt correctness must tolerate later handler changes and persistence failure (the upstream boundary append is not transactional). This follow-up tests fake text responses, not real tools, compaction/retry interactions, concurrent queue merges, reload or crash recovery.
