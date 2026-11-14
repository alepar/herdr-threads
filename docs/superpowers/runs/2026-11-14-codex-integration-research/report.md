# Newer Codex integration options for herdr-threads

Research date: 2026-11-14. Research only; no production changes or authenticated live experiments. Installed CLI inspected: 0.167.0. Latest upstream release inspected: 0.169.0, released November 13. These versions are historical evidence, never proposed admission gates. [23]

**Codex has shipped a credible route to remove composer typing: its persistent queue can reach a loaded foreground TUI. It is worth an isolated prototype now, but it is not yet a qualified replacement for Native wake or a complete Claude-style channel.** The useful finding is the queue watcher inside the embedded runtime, not merely the presence of a JSON-RPC server. [4, 8, 9, 10]

Recommendation: qualify queue-backed wake for storage matching, interruption, delayed dispatch and uncertain enqueue handling before relying on it. [3]
<!-- claim: 0eb15ad185af2b35; evidence: ET3; source: b6b094dc064f3ee6 -->

The smallest proposal retains existing hook delivery for message bodies and uses a content-free queued marker only to request an eligible idle turn. A full push channel is a separate, larger app-server topology change. Details: [transport dossier](transport.md), [hooks and messaging dossier](features.md).

<!-- facet: F1 -->
## What shipped and what it enables

| Mechanism | Passive context / active turn | Idle turn without typing | Release and fit |
|---|---|---|---|
| Persistent `codex queue` | Text becomes user input; not a passive body channel | Source-backed candidate for loaded persistent embedded TUI | CLI and watcher in inspected 0.167.0 and 0.169.0 tags; stable command |
| `thread/inject_items` | Changes loaded thread history for later model requests | No | Documented RPC; requires reachable owning runtime |
| `turn/start.toolOutput` / SDK `ExternalMessage` | Retains external data as tool output; joins an active regular turn | Yes, when idle | Documented; strongest full-channel primitive if same server owns the TUI thread |
| `turn/steer` | Active-turn user input; checks expected turn ID | No | Useful steering, wrong attribution for peer bodies |
| `--remote` / daemon control proxy | Connect clients to a server-backed runtime | Through turn RPC | App-server command / TCP WebSocket control experimental; cannot attach to an already embedded TUI |
| Async hooks | Context at safe boundaries | No | Documented lifecycle feature |
| Stop continuation | Finishing-turn continuation | Cannot originate later idle work | Already fits end-of-turn delivery, not a subscription |
| Plugins / MCP | Tools and trusted hooks | No public local-TUI channel established by retrieved evidence | Packaging exists; notification ingress remains unqualified |
| Internal agent mailbox / board | Runtime-scoped messaging | Some internal trigger modes | Relevant wake/board flags under development and off by default |

The table synthesizes tagged queue/runtime sources, protocol and SDK documentation, and the feature registry. The dossiers distinguish verified behavior from inference and record exact source locators. [1, 2, 4, 5, 8, 11, 15, 17, 19, 20, 21, 22, 27]

## Why queue matters for today's foreground TUI

The embedded runtime initializes SQLite, constructs a persistent queue backend for local storage and installs the watcher. Its cross-process scan notices changed queues for loaded threads and calls an idle-only start operation. The watcher is present in both examined tags; the complete registration trace was checked in 0.169.0. In-memory storage has no such backend. This supports feasibility without moving tools and hooks into the shared daemon. [4, 5, 6, 7, 8, 9, 10]

The inferred route is:

```mermaid
sequenceDiagram
    participant HT as herdr-threads
    participant Q as Codex persistent queue
    participant T as Foreground Codex TUI
    HT->>HT: Eligible pending item and idle binding
    HT->>Q: Queue existing content-free wake marker
    Q-->>T: Watcher discovers change; try start if idle
    T->>HT: Existing turn hook pulls pending items
    HT-->>T: Attributed message context
```

This is hybrid delivery: hooks carry content, a queue requests the turn. It is not a passive push channel. Queue input has user-message attribution, so sending peer bodies through it would change their meaning. Keep peer data in the existing attributed hook envelope. `--resume` is not required for the inferred loaded embedded-target path; it resumes through a daemon or selected remote server and can warn even after a successful enqueue. [3, 4]

<!-- facet: F2 -->
## What does not close the gap

Async hooks wait for the next user turn when idle. Stop creates a continuation at turn completion. Model-request hooks are diagnostic. `notify` is outbound completion signaling. None independently starts a later idle turn. Plugins bundle capabilities; they do not automatically create a mod-style channel. [17, 18, 29]

Internal `send_message` and `followup_task` use different delivery modes through the caller's agent runtime. The tagged registry includes under-development wake and board flags. Tree-scoped boards and research-host credentials do not establish an external endpoint for independent TUI roots. MCP Events is documented for Work web, desktop Cloud and dots, rather than local Codex CLI. [28] The complete local MCP notification and extension API implementation remains an explicit retrieval gap. [19, 20, 21, 22, 25, 26]

Side chats have separate transcripts; 0.169.0 adds opt-in workspace tabs and reopening. Goals continue an objective. Neither is a reason to run a permanent mailbox turn. Exact native session IDs matter more as one pane contains more chats. [14, 19, 23]

## Full channel alternative: dedicated app-server

A TUI attached to an explicit server can share that owning runtime with an integration client. The client can inject passive context and submit attributed tool output. SDK `ExternalMessage` uses that lower-priority attribution too. A new SDK process by itself does not attach to an embedded TUI. Individual RPC/field experimental gating was not pinned against generated release schemas; the table distinguishes documented behavior from command/transport maturity. RPC requires the endpoint plus native thread ID; an embedded TUI exposes no reconnectable remote connection. [11, 15, 27]

A dedicated pane-owned server could inherit the correct pane environment while its TUI stays visible. That is a design option, not a qualification result. Preserve the existing shared-daemon refusal until environment propagation, session continuity, process ownership and reconnect behavior are demonstrated. The reason for that refusal is documented in [the Codex pane environment adapter](../../../../src/harness/codex/pane_env.rs). Remote-control pairing does not repair attribution. [12, 13]

<!-- facet: F3 -->
## Proposed adapter work and acceptance criteria

Start with an opt-in queue-wake qualification, leaving Native as the default. The prototype should add a neutral optional wake-transport policy behind `HarnessAdapter`; Codex-specific command construction and observation live in `src/harness/codex/`. The core continues to decide priority eligibility, canonical binding/session targets, idle state and wake reservations. No harness-name branch belongs there.

[The current policy pairing](../../../../src/harness/registry.rs) admits HookPull with Native, while Channel is coupled to push delivery. A queue wake plus HookPull does not fit that declaration honestly. Add an explicit neutral representation for a programmatic wake independently of body transport; update registry validation and the trust policy together. Do not label queue acceptance as a push delivery to satisfy the existing enum.

The proposal needs these behaviors:

- Record each binding's native session UUID, executable identity and actual queue storage/profile configuration. Never choose a chat by label, latest activity or current display. Preserve all bindings; never merge seats or move them based on a queue lookup.
- Only enqueue for pending priority `p <= T` after canonical idle checks. p1 at T=0 stays silent; busy sessions receive bodies through their normal hooks. An upstream idle-only start protects generation overlap, but does not eliminate a deferred stale marker after a user-activity race.
- Reuse the existing wake marker; add no model-visible wording. Keep ordering, truncation, invitation acceptance and flush-based receipt ACKs on the current delivery path. A queue result is acceptance, not display or ACK.
- Distinguish absent capability, proven pre-submit refusal, accepted queue item and unknown outcome. Degrade to Native only before confirmed submission, or after proving a queued item cannot still dispatch. An uncertain result must not trigger a second transport.
- Reconcile or cancel stale markers and deduplicate retries using observed queue identity. The CLI generates new client-message IDs on each invocation; the research did not establish a reliable cancellation/reconciliation implementation. Respect interrupted sessions rather than forcibly overriding their pause.
- Probe actual behavior and missing capabilities, cached against executable/storage identity. Never gate on an exact release string. A hook-only or in-memory configuration retains Native.

Before enabling it, an isolated authenticated experiment must cover p0/p1 thresholds, busy arrival and idle races, draft and modal preservation, root versus side targeting, interrupted and dormant resume, storage overrides, duplicate/lost enqueue replies, stale-marker cleanup, delivery receipt evidence and total wake latency. Compare the observed approximately ten-second scan with the project's wake expectations; no latency guarantee was measured here. The existing release wake matrix is the appropriate eventual regression test.

## Methodology, limitations and stop

Used official OpenAI documentation, official tagged upstream source, release notes and an upstream issue; source-code behavior outranks the issue report. Search CLI 0.14.1 preflight and its first deep scoped query succeeded. Later provider network failures triggered hosted web search/open fallback. Read-only installed CLI help/version checks confirmed available commands. No real profiles, credentials, shared services or production files were changed.

The canonical ledgers retain source identities, passages, queries, access failures, claims and independent model support verdicts. All 18 recorded claims received supported verdicts after qualification of overbroad statements. The judge runs requested the skill’s Terra model; returned model labels were generic GPT-6, so the precise judge backend is not independently verified. Initial batched per-query result counters and worker screening-only access records are incomplete; reported totals describe logged attempts and must not be treated as exhaustive effort or recall. Source extraction fallback passages are explicitly non-verbatim. Reported inaccessible URLs include guessed paths that may not exist; they do not represent a count of confirmed existing documents behind an access barrier. The source and report validators cannot prove runtime behavior. The standalone citation checker detected no suspicious entries but could not resolve any of its 29 URLs because local DNS was unavailable; [its output](citation-check.txt) is retained. Canonical citation identity checks passed, and the substantive sources were read through the hosted web tool or successful extraction.

Retrieval stopped as **budget-exhausted**, not coverage-saturated: the bounded documentation/source investigation completed, while full local MCP/extension ingress and production race/recovery semantics remain unresolved. Live authenticated qualification required separate approval under the handoff and was not run.

The saved logs contain 13 search attempts (3 failures), at least 33 inspected result entries (4 per-query counters unavailable), and 72 document fetch records with 30 distinct documents read; these are lower-bound accounting, with repeated navigations sometimes grouped. There were 29 failed fetch records, 8 recovered document identities and 18 unrecovered URLs, many guessed paths. Three high-priority passages use non-verbatim fallback.

Retrieval-health output, failed URLs, recovered documents and recommended actions are preserved in [retrieval-health.md](retrieval-health.md). Revisit missing implementations via the current tagged source tree, recover exact passages when extraction works, then run the isolated prototype before changing wake policy.

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

[12] OpenAI (2026). [Session command startup](https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/tui/src/session_archive_commands.rs).

[13] OpenAI (2026). [Daemon selection](https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/tui/src/daemon_startup.rs).

[14] OpenAI (2026). [Codex developer commands](https://learn.chatgpt.com/docs/developer-commands).

[15] OpenAI (2026). [Codex app-server](https://learn.chatgpt.com/docs/app-server).

[16] OpenAI (2026). [Queue interruption report](https://github.com/openai/codex/issues/56914).

[17] OpenAI (2026). [Hooks](https://learn.chatgpt.com/docs/hooks).

[18] OpenAI (2026). [Advanced configuration](https://learn.chatgpt.com/docs/config-file/config-advanced).

[19] OpenAI (2026). [Feature registry](https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/features/src/lib.rs).

[20] OpenAI (2026). [V2 send_message](https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/core/src/tools/handlers/multi_agents_v2/send_message.rs).

[21] OpenAI (2026). [V2 followup_task](https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/core/src/tools/handlers/multi_agents_v2/followup_task.rs).

[22] OpenAI (2026). [V2 messaging dispatch](https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/core/src/tools/handlers/multi_agents_v2/message_tool.rs).

[23] OpenAI (2026). [Release 0.169.0](https://github.com/openai/codex/releases/tag/rust-v0.169.0).

[24] OpenAI (2026). [Extension footer rows](https://github.com/openai/codex/pull/60032).

[25] OpenAI (2026). [Message board fixed views](https://github.com/openai/codex/pull/59993).

[26] OpenAI (2026). [Remote board configuration](https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/features/src/feature_configs.rs).

[27] OpenAI (2026). [Codex SDK](https://learn.chatgpt.com/docs/codex-sdk).

[28] OpenAI (2026). [MCP Events](https://developers.openai.com/plugins/build/mcp-events).

[29] OpenAI (2026). [Package your plugin](https://developers.openai.com/plugins/build/plugins).
