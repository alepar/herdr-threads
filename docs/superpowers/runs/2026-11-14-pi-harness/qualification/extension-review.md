Machine-specific command paths are redacted as `HEAVY TOOLBOX` and `<worktree>`; review verdicts are unchanged.

# Task 1 review

Spec compliance: Issues found. D5 unchanged-attention suspension and D3 shutdown reaping are incomplete. Task quality: Needs fixes.

Reviewed base `228502ec0` through head `dc581e95f`, using the supplied diff package, Task 1 brief/report, current design D1–D5, event contract and recorded runtime boundary probe. Production scope is the four `integrations/pi/**` files in the package.

## Strengths

- Exact custom type/body presence in both current branch and model-bound context gates receipt evidence; complete message/lazy IDs alone enter the ACK request, and branch/attempt identities fence late ACK results (`integrations/pi/extension/herdr-threads.js:256`, `integrations/pi/extension/herdr-threads.js:353`; causal late-result test at `integrations/pi/extension/tests/extension.test.mjs:189`).
- Native boundary proposals preserve foreign entries and continuation, explicitly reject non-completed outcomes, and keep bodies out of `sendMessage`; the sole send carries the existing content-free wake marker (`integrations/pi/extension/herdr-threads.js:169`, `integrations/pi/extension/herdr-threads.js:339`).
- Buffer trimming and page construction separate pending bodies from immutable proposals, and the tests include incoming overflow, displacement, partial ACK release and real owned-child transport (`integrations/pi/extension/herdr-threads.js:108`, `integrations/pi/extension/herdr-threads.js:222`, `integrations/pi/extension/tests/extension.test.mjs:297`).
- Qualification limitations are explicitly reported, including real TUI/integrated execution (`integrations/pi/README.md:96`).

## Critical

None found.

## Important

### I1: An unchanged reoffer of another urgent ID clears rejection suspension

`integrations/pi/extension/herdr-threads.js:198` replaces `o.intent` and clears suspension whenever the incoming key differs from the *single most recent* intent, before checking whether that key is already pending/proposed. `blockAttention` at line 154 consequently protects only the last intent. This mistakes an already retained, unchanged reoffer for genuinely new attention.

Reproduced through the real registered callbacks: start an active user run; deliver waking p0 items A and B; propose both at a completed boundary; simulate a later extension changing the accepted frame; emit context and then idle settlement. Wakes remain zero, as required. Reoffer unchanged A: wakes become one. No new attention, user run or channel replacement occurred. Repeated native watch reoffers can therefore restart rejected/error attention, contrary to D5. The existing rejection regression (`integrations/pi/extension/tests/extension.test.mjs:325`) exercises only one retained urgent ID and misses this case.

Preserve the attention-generation/rejection fence across already known keys. Only genuinely new attention or an explicitly permitted lifecycle/channel transition should release it. Add a two-ID rejected-batch regression, including alternating unchanged reoffers and non-completed outcomes; keep metadata bounded.

### I2: Shutdown loses ownership of a watch already being replaced

`replaceWatch` clears `o.child` before awaiting its stop (`integrations/pi/extension/herdr-threads.js:124`, line 126). During that await, `close` sees no child and awaits only its local `stopping` value and command tasks (`integrations/pi/extension/herdr-threads.js:280`, line 288, line 293). The in-flight replacement/stop promise is not tracked in `o.calls` or otherwise joined.

Reproduced through the real registered callbacks with controlled child IO: fill 257 passive rows, propose and observe a page, let ACK release trigger replacement, and keep the predecessor's stop promise unresolved. Emit shutdown. Its promise completes while the owned predecessor still has not been reaped. Only afterwards release the stop promise. The native transport can be waiting for TERM/KILL and process exit at this point, so shutdown/reload can report completion or admit a successor before all predecessor children are gone. This violates D3 and the task's stop/reap contract. The existing shutdown regression waits for command children, not an already detached watch stop (`integrations/pi/extension/tests/extension.test.mjs:347`).

Track all owned stop/replacement promises until settled, and join them during close while fencing replacement startup immediately. Add shutdown-during-replacement and reload-during-replacement regressions with a deliberately deferred child exit.

## Minor

None recorded.

## Checks and scope limits

- Read the supplied diff in sequential chunks once for code review; subsequently extracted only exact source-line references from its addition metadata. No changed source file was separately reread, and no git diff/log commands were run.
- One focused outside-diff check addressed a concrete protocol risk: whether watch controls carry IDs and whether legitimate watch bodies are bounded before native framing. `src/protocol/watch.rs:17` documents the status/attention IDs, and its constants define 8 KiB bodies and 32-item/64 KiB pages. No wider source crawl was performed.
- Reviewed the complete reported RED/GREEN and final 41-pass evidence. No suite/build was rerun. The SDK provider and native transport tests are useful, but the two additional interleavings above were absent from that evidence.
- Ran one inline focused probe containing the two callback sequences above, through `HEAVY TOOLBOX node --input-type=module`. Output, exit 0: `rejected two-ID batch, wakes before unchanged reoffer: 0`; `wakes after unchanged reoffer: 1`; `shutdown completed while replacement child stop remains unresolved: true`. The probe imported the real extension and used controlled IO; it started no service or child process and wrote no fixture. The first sandboxed wrapper attempt could not acquire its external locks and was stopped (exit 130); the escalated wrapper run succeeded. No approval rejection was bypassed.
- Cannot verify from this task diff: canonical Rust admission/innermost-process attribution, lifecycle ObserverOnly output, public setup/launch, daemon-backed ACK/replay and presentation election, or trust-policy amendments. These remain Task 2/integration gates, not additional Task 1 scope.
- Cannot verify from this task diff: real TUI composer/modal interference, native resume/fork behavior and authenticated provider qualification. These remain Task 3 gates; scripted SDK evidence does not resolve them.

Task quality: Needs fixes. The central proposal/receipt approach is sound in the reviewed paths, but multi-item attention and asynchronous watch ownership each violate a binding invariant in a reproducible callback sequence.
