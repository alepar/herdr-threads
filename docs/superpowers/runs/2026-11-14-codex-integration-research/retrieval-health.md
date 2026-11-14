### Retrieval health

**Status: DEGRADED.** 3 search attempt(s) failed; 18 document(s) still inaccessible; 3 high-priority quote(s) not verbatim (extract failed or paraphrased).

- Search preflight: ok (search CLI) - search 0.14.1 agent-info inspected; deep scoped live query succeeded
- Search attempts: 13 (3 failed, 10 success)
- Document fetches: 72 attempts, 30 distinct documents read, 29 failed attempts, 8 recovered, 18 still inaccessible
- Partial text only: 28 document(s)
- High-priority quotes: 5 (2 verbatim from search -m extract, 3 non-verbatim fallback, 0 provenance unknown)
- Claim support check: model; 18 claim(s) judged by model (GPT-6, gpt-6), 0 unverified by model

Failed searches:

| Query ID | Query | Category | Error |
|---|---|---|---|
| WFQ1 | site:learn.chatgpt.com Codex goals side mailbox send_message notification MCP | network | all provider network failure |
| PQ5 | site:learn.chatgpt.com codex queue interruption idle external messages | network | all seven providers failed network; automatic web fallback |
| PQ6 | site:learn.chatgpt.com Codex app-server turn start toolOutput inject_items idle | network | all seven providers failed network; automatic web fallback |

Documents still inaccessible:

| URL | Tools tried | Category | Error |
|---|---|---|---|
| https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/cli/src/queue.rs | web.run open | not_found | 404 or cache miss while discovering source path |
| https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/tui/src/session_queue.rs | web.run open | not_found | 404 or cache miss while discovering source path |
| https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/thread-store/src/local_queue.rs | web.run open | other | 404 or cache miss while discovering source path |
| https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/thread-store/src/queue.rs | web.run open | not_found | 404 or cache miss while discovering source path |
| https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/app-server/src/request_processors/thread_queue.rs | web.run open | not_found | 404 or cache miss while discovering source path |
| https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/app-server/src/request_processors/thread_queue/mod.rs | web.run open | other | 404 or cache miss while discovering source path |
| https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/core/src/mcp_connection_manager.rs | search -m extract, web.run open | not_found | not_found or web cache miss (see worker limitation) |
| https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/mcp-client/src/lib.rs | web.run open | not_found | not_found or web cache miss (see worker limitation) |
| https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/core/src/mcp/mod.rs | web.run open | other | not_found or web cache miss (see worker limitation) |
| https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/core/src/mcp_connection_manager/mod.rs | web.run open | other | not_found or web cache miss (see worker limitation) |
| https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/rmcp-client/src/logging_client_handler.rs | web.run open | other | not_found or web cache miss (see worker limitation) |
| https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/rmcp-client/src/event_notification_transport.rs | web.run open | other | not_found or web cache miss (see worker limitation) |
| https://learn.chatgpt.com/docs/subagents | web.run open | not_found | not_found or web cache miss (see worker limitation) |
| https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/tui/src/extensions.rs | web.run open | other | not_found or web cache miss (see worker limitation) |
| https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/agent-message-board/src/lib.rs | web.run open | other | not_found or web cache miss (see worker limitation) |
| https://github.com/openai/codex/tree/rust-v0.169.0/codex-rs/tui/src | web.run open | other | not_found or web cache miss (see worker limitation) |
| https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/agent-message-board-host/src/lib.rs | web.run open | other | not_found or web cache miss (see worker limitation) |
| https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/app-server-protocol/src/protocol/v2.rs | web.run open | not_found | 404: outdated guessed source path |

Recovered after a failed attempt:

| URL | Failed with | Recovered by |
|---|---|---|
| https://github.com/openai/codex/blob/rust-v0.169.0/codex-rs/cli/src/queue_cmd.rs | search -m extract (network) | web.run open/find |
| https://github.com/openai/codex/blob/rust-v0.169.0/codex-rs/ext/queue/src/service.rs | search -m extract (network) | web.run open/find |
| https://github.com/openai/codex/blob/rust-v0.169.0/codex-rs/app-server/src/message_processor.rs | search -m extract (network) | web.run open/find |
| https://learn.chatgpt.com/docs/developer-commands | search -m extract (network) | web.run open/find |
| https://github.com/openai/codex/blob/rust-v0.169.0/codex-rs/cli/src/main.rs | search -m extract (network) | web.run open |
| https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/features/src/lib.rs | search -m extract (network) | web.run open |
| https://github.com/openai/codex/releases/tag/rust-v0.169.0 | search -m extract (network) | web.run open |
| https://learn.chatgpt.com/docs/codex-sdk | search -m extract (network) | web.run open |

Non-verbatim high-priority quotes (search -m extract failed or paraphrased):

| Evidence | Facet | Quoted via | Document |
|---|---|---|---|
| ET4 | F1 | web.run open/find | https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/ext/queue/src/service.rs |
| ET8 | F1 | web.run open/find | https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/app-server/src/message_processor.rs |
| WFE5 | F2 | web.run open | https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/features/src/lib.rs |

Recommended actions:

- **network** (https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/core/src/mcp_connection_manager.rs, query PQ5, query PQ6, query WFQ1): Transient network failure: rerun the affected queries or fetches.
- **not_found** (https://learn.chatgpt.com/docs/subagents, https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/app-server-protocol/src/protocol/v2.rs, https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/app-server/src/request_processors/thread_queue.rs, https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/cli/src/queue.rs, https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/core/src/mcp_connection_manager.rs and 3 more): The URL is dead or moved: find a current or archived copy.
- **other** (https://github.com/openai/codex/tree/rust-v0.169.0/codex-rs/tui/src, https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/agent-message-board-host/src/lib.rs, https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/agent-message-board/src/lib.rs, https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/app-server/src/request_processors/thread_queue/mod.rs, https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/core/src/mcp/mod.rs and 6 more): Inspect the recorded error text.
- **non_verbatim_quote** (evidence ET4, evidence ET8, evidence WFE5): search -m extract failed or returned only a paraphrase, so these high-priority quotes may not be word-for-word: check them against the original document, or supply the document so they can be quoted verbatim.
