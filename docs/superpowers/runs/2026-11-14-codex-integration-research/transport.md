# Transport: persistent queue and app-server

Research date: 2026-11-14. Tagged sources establish presence, not first introduction. Statements based on source traces are distinct from live qualification. No authenticated experiment was run.

The queue CLI and the ten-second external-message watcher are present in both inspected release tags, 0.167.0 and 0.169.0. [1]
<!-- claim: a8de8cc5c13b3883; evidence: ET1; source: ccf1c40aed4d58eb -->

In 0.169.0, the embedded TUI initializes a state database; its in-process host passes that database to MessageProcessor, which constructs a queue service for a local persistent store and installs its external watcher. In-memory stores lack this backend. [6]
<!-- claim: 9508dd84e57422f0; evidence: ET6; source: 52dc09fca4afafb1 -->

Recommendation: test whether a content-free queued marker can wake a loaded embedded TUI sharing the sender’s persistent queue storage; the source architecture makes this worth qualification despite the absent remote connection. [4]
<!-- claim: 3e0f55c5f67f73b2; evidence: ET4; source: 434284b476b8086b -->

Queue dispatch uses an idle-only core operation, suppresses interrupted threads, and can defer input until later idle. A successful enqueue therefore does not prove a wake was executed. [3]
<!-- claim: 3ffc1b8e914f4187; evidence: ET3; source: b6b094dc064f3ee6 -->

The queue sender submits user text with fresh client-message UUIDs; its --resume path can warn while enqueue still succeeds. Its refusal of --no-daemon does not establish the target embedded runtime lacks a queue watcher. [3]
<!-- claim: a1b1270029be3123; evidence: ET3; source: b6b094dc064f3ee6 -->

A remote TUI and another client can use the same app-server endpoint. An embedded TUI has no reconnectable remote connection. [15]
<!-- claim: 8e1941340511ea29; evidence: EP3; source: b86fe73675b0f8f5 -->

App-server turn/start.toolOutput preserves supplied data as functionCallOutput and queues it into an active regular turn; thread/inject_items changes subsequent prompt history without starting a user turn. [15]
<!-- claim: bd08d5f643aa1835; evidence: EP1; source: b86fe73675b0f8f5 -->

The documented SDK ExternalMessage keeps external content at tool-output priority and can start a turn or join an active regular turn. This alone is not evidence of attaching to an existing embedded TUI. [27]
<!-- claim: 1d9d8b5ef8d5a5a8; evidence: EP4; source: 533f3b185a693d99 -->

## Targeting, authentication and process boundary

Use the native session UUID captured for the binding, the correct executable, profile and actual storage configuration. A label, current foreground pane or matching home-directory variable does not prove the storage matches. The queue sender may use an app-server internally; the receiving TUI can still be embedded. This is a cross-process storage route, not a way to attach RPC to the embedded runtime.

For explicit RPC, identify both endpoint and native thread ID. Unix-socket access is local account access; WebSocket deployments use the configured bearer/capability-token authentication. Keep transport access separate from seat continuity. A shared daemon does not become pane-owned merely because the TUI remains visible. [15]

## Operational limits

A marker can wait through a race with user activity and become a later, unnecessary wake. Automatic dispatch after an interruption is suppressed; do not override a user interruption to make a test pass. Fresh CLI submission IDs make blind retries unsafe after unknown acceptance. Queue acceptance is neither model delivery nor an ACK. The approximately ten-second cross-process scan is source behavior, not a measured wake bound. [3, 4]

The missing evidence is an isolated loaded foreground TUI experiment, including queue storage overrides, lost replies, multiple side sessions, queued cancellation and interrupted resume. The source-backed route is worth that experiment; this dossier does not qualify it for production.

## Bibliography

[1] OpenAI (2026). [Queue CLI release implementation](https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/cli/src/queue_cmd.rs).

[2] OpenAI (2026). [Queue CLI installed-tag implementation](https://raw.githubusercontent.com/openai/codex/rust-v0.167.0/codex-rs/cli/src/queue_cmd.rs).

[3] OpenAI (2026). [Session queue command](https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/tui/src/session_queue_commands.rs).

[4] OpenAI (2026). [Durable queue service current release](https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/ext/queue/src/service.rs).

[5] OpenAI (2026). [Durable queue service installed tag](https://raw.githubusercontent.com/openai/codex/rust-v0.167.0/codex-rs/ext/queue/src/service.rs).

[6] OpenAI (2026). [Queue extension registration](https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/ext/queue/src/lib.rs).

[7] OpenAI (2026). [App-server extension installation](https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/app-server/src/extensions.rs).

[8] OpenAI (2026). [Queue backend construction](https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/app-server/src/message_processor.rs).

[9] OpenAI (2026). [Embedded app-server host](https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/app-server/src/in_process.rs).

[10] OpenAI (2026). [TUI embedded state and runtime](https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/tui/src/lib.rs).

[11] OpenAI (2026). [Connection targets](https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/tui/src/app_server_connection.rs).

[15] OpenAI (2026). [Codex app-server](https://learn.chatgpt.com/docs/app-server).

[27] OpenAI (2026). [Codex SDK](https://learn.chatgpt.com/docs/codex-sdk).
