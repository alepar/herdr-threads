# Pi extension / Rust adapter contract

Approved design: design.md. This local payload is cooperative adapter input, not peer wire.

The extension invokes the setup-rendered argv prefix plus `hook-event pi EVENT`, JSON on stdin, bounded stdout. Setup replaces a single marked `LAUNCH` constant with `{ "argv": [...] }`; absent configuration means `herdr-threads` from PATH. The directory is owned as one setup artifact so relative helper imports remain valid.

Each payload has `event`, `session_id` (native pi session), `role: "top_level"`, `mode: "tui"`, `pid` (positive native process integer), and `cwd`. Environment carries the same owner via `HERDR_THREADS_PI_SESSION`, `HERDR_THREADS_PI_ROLE`, `HERDR_THREADS_PI_MODE`, and decimal `HERDR_THREADS_PI_PID`. Capture inherited markers before overwriting; unknown/child/nonterminal does not enroll. Rust verifies the claimed PID against the actual innermost pi process, not merely against itself.

| Event | Fields and meaning |
|---|---|
| SessionStart | `source`: startup, reload, new, resume, fork, compact, tree. Rust maps startup/reload to Startup, new to Clear, resume to Resume, fork to Fork, compact/tree to Compact (tree refreshes guidance without inventing a successor session). |
| TurnStart | Current check-in/liveness, no mail or repeated fixed guidance; report Busy through the existing turn-state seam before the native run, after lifecycle admission. |
| TurnEnd | Observer-only, no mail/guidance; `source` is `settled` for an actual agent_settled callback or `startup` for explicit post-admission idle initialization. Requires `idle: true` from a fresh current-owner ctx.isIdle() check at emission. Report Idle through the existing seam; missing/false/unknown witness never reports Idle. |
| SessionEnd | Observer event with existing SessionEnd binding-liveness report; no retirement. Captured old owner fields survive until this best-effort child call is formed; stale cleanup must not delete successor env. |

Lifecycle and turn-edge calls are serialized per current owner. Recheck identity and native idle state when executing a queued Idle report, so an old callback cannot overtake a newer Busy edge or report for a successor. Startup Idle uses the separate Observer-only TurnEnd after successful SessionStart because the existing shared runner suppresses an Idle edge on nonempty output; do not change that shared continuation rule or omit lifecycle guidance. agent_end, turn_end, error and cancellation alone never authorize Idle.

Hook output is plain UTF-8 guidance/routing/diagnostics, maximum the existing hook context bound (4096 bytes), encoded ObserverOnly. Successful empty stdout is valid; nonzero exit, timeout or existing check-in-unavailable sentinel is failure. Shared output policy omits mail offers before composition; extension only stages successful guidance for an admitted native boundary. Mail is exclusively the watch path.

Channel-owned admission, decode and local-evidence refusals must carry the existing check-in-unavailable sentinel; successful empty stdout cannot represent refusal. Apply this before any binding mutation and on both hook entry paths. Existing adapters retain their quiet refusal behavior. The extension must leave enrollment pending and not start its watch after a refusal; the next valid lifecycle attempt can retry.

Watch uses the existing CLI: `watch --harness pi --session SESSION`; only an admitted current TUI owner starts it. Completion uses existing `watch ack --via context` and exact eligible IDs; no invented stored-row ID. Use current protocol types as authority for all frames/controls/leases and election. Branch/execution/nonces are local proposal evidence, never new daemon authority. Generation changes fence callbacks, timers, retries and children.

No API version gate. If a required native boundary/API shape is unavailable, disable that capability and expose a compact diagnostic. Changes to this contract require coordinator review before either side diverges.

Implementation clarification: WatchLine exposes priority/deadline, not creation time. Sort the whole retained union by those keys and stable first-observed canonical watch order; retain the ordinal on reoffers. Exact cross-replay age is not claimed. Ordinary CLI watch automatically negotiates presentation mode and confirms nonordinary output after flush; the extension must not disable that election or duplicate its confirmations.


Native-wake amendment: delivery remains PushChannel/context receipts, but the adapter declares IdleWake::Native and supplies a strict default-UI ComposerPolicy. The extension never submits an automatic idle run. The existing host types `herdr-threads: wake up` only through its normal guarded wake lane; it arrives as an ordinary user prompt and the boundary path supplies bodies. A missing channel falls back to explicit inbox, while a host without composer wake reports no_wake. Built-in selectors are guarded; arbitrary other-extension custom overlays have the accepted native-screen limit described in design D5 and must be documented in the normative policy.
