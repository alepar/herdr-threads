# Pi extension / Rust adapter contract

Approved design: design.md. This local payload is cooperative adapter input, not peer wire.

The extension invokes the setup-rendered argv prefix plus `hook-event pi EVENT`, JSON on stdin, bounded stdout. Setup replaces a single marked `LAUNCH` constant with `{ "argv": [...] }`; absent configuration means `herdr-threads` from PATH. The directory is owned as one setup artifact so relative helper imports remain valid.

Each payload has `event`, `session_id` (native pi session), `role: "top_level"`, `mode: "tui"`, `pid` (positive native process integer), and `cwd`. Environment carries the same owner via `HERDR_THREADS_PI_SESSION`, `HERDR_THREADS_PI_ROLE`, `HERDR_THREADS_PI_MODE`, and decimal `HERDR_THREADS_PI_PID`. Capture inherited markers before overwriting; unknown/child/nonterminal does not enroll. Rust verifies the claimed PID against the actual innermost pi process, not merely against itself.

| Event | Fields and meaning |
|---|---|
| SessionStart | `source`: startup, reload, new, resume, fork, compact, tree. Rust maps startup/reload to Startup, new to Clear, resume to Resume, fork to Fork, compact/tree to Compact (tree refreshes guidance without inventing a successor session). |
| TurnStart | Current check-in/liveness only, no mail or repeated fixed guidance. Used before a native run after any necessary lifecycle admission. |
| SessionEnd | Observer event with existing SessionEnd binding-liveness report; no retirement. Captured old owner fields survive until this best-effort child call is formed; stale cleanup must not delete successor env. |

Hook output is plain UTF-8 guidance/routing/diagnostics, maximum the existing hook context bound (4096 bytes), encoded ObserverOnly. Successful empty stdout is valid; nonzero exit, timeout or existing check-in-unavailable sentinel is failure. Shared output policy omits mail offers before composition; extension only stages successful guidance for an admitted native boundary. Mail is exclusively the watch path.

Channel-owned admission, decode and local-evidence refusals must carry the existing check-in-unavailable sentinel; successful empty stdout cannot represent refusal. Apply this before any binding mutation and on both hook entry paths. Existing adapters retain their quiet refusal behavior. The extension must leave enrollment pending and not start its watch after a refusal; the next valid lifecycle attempt can retry.

Watch uses the existing CLI: `watch --harness pi --session SESSION`; only an admitted current TUI owner starts it. Completion uses existing `watch ack --via context` and exact eligible IDs; no invented stored-row ID. Use current protocol types as authority for all frames/controls/leases and election. Branch/execution/nonces are local proposal evidence, never new daemon authority. Generation changes fence callbacks, timers, retries and children.

No API version gate. If a required native boundary/API shape is unavailable, disable that capability and expose a compact diagnostic. Changes to this contract require coordinator review before either side diverges.

Implementation clarification: WatchLine exposes priority/deadline, not creation time. Sort the whole retained union by those keys and stable first-observed canonical watch order; retain the ordinal on reoffers. Exact cross-replay age is not claimed. Ordinary CLI watch automatically negotiates presentation mode and confirms nonordinary output after flush; the extension must not disable that election or duplicate its confirmations.
