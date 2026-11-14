# Hooks, plugins and internal messaging

Research date: 2026-11-14. Tagged sources establish presence, not first introduction. Statements based on source traces are distinct from live qualification. No authenticated experiment was run.

A background hook completing while Codex is idle does not start a new turn. [17]
<!-- claim: 2d4a6334d2f2bb73; evidence: EP2; source: 99bc0cb8da0c7a35 -->

PreModelRequest and PostModelRequest output is diagnostic only and cannot modify context or block a model request. [17]
<!-- claim: 1163ef2f7233d880; evidence: WFE3; source: 99bc0cb8da0c7a35 -->

A Stop block creates a continuation prompt using its reason. Background hook completion while idle cannot start a new turn. [17]
<!-- claim: 6c45cb72a36e63d5; evidence: WFE2; source: 99bc0cb8da0c7a35 -->

The notify configuration executes an external program for outbound agent-turn-complete notifications. [18]
<!-- claim: 6cd6f41455d78245; evidence: WFE4; source: 3d9c8f85945796db -->

The send_message handler selects QueueOnly; the followup_task handler selects TriggerTurn. Their shared implementation resolves the target in the calling session and calls its agent-control service. [20]
<!-- claim: 00267bc09c7dd9cf; evidence: WFE6; source: d86b461fa19da88c -->

The tagged registry marks multi_agent_v2 Stable but disabled by default; wake_on_agent_messages, defer_mailbox_preemption, agent_message_board and agent_message_board_mcp are under development and disabled by default. [19]
<!-- claim: 5103640117c37025; evidence: WFE5; source: 572d149a48152eb4 -->

Side chats have separate transcripts. The tagged goals feature is Stable and enabled by default. Neither observation proves an external mailbox ingress. [14]
<!-- claim: 1480e0ee29b4769f; evidence: WFE9; source: 8994ef1678838607 -->

The inspected extension-footer PR establishes status rendering. The board is tree-scoped, and remote board configuration is for a research-host-provisioned URL and bearer credentials; these observations do not establish a public external local-TUI wake API. [24]
<!-- claim: 20ef7739e8423f11; evidence: WFE11; source: 82447ff6412101bb -->

The MCP Events documentation describes Work web, desktop Cloud and dots webhook delivery; it does not establish local Codex CLI event-triggered delivery. [28]
<!-- claim: bea4e144b747ddf9; evidence: EP5; source: d29e87e06f5c8298 -->

## Packaging and unsupported assumptions

Plugins package skills, MCP servers and lifecycle hooks, with trust review for the current hook definition. Packaging alone does not establish an incoming channel. The inspected footer extension change is UI rendering; it is not evidence of a JavaScript session-append API comparable to Claude’s mod.

MCP logging, resource-change notifications and elicitation must not be treated as model-context delivery merely because they are server-to-client traffic. The attempted local notification/extension source paths were unavailable, so this report leaves that implementation unqualified rather than asserting no such feature exists. See the failed URLs and recovery advice in [retrieval health](retrieval-health.md).

Goals maintain ongoing objectives; using one to keep a mailbox loop running would spend turns instead of leaving an idle agent idle. Side chats have independent session IDs, so delivery must never infer a target from whichever chat is currently displayed.

## Bibliography

[14] OpenAI (2026). [Codex developer commands](https://learn.chatgpt.com/docs/developer-commands).

[17] OpenAI (2026). [Hooks](https://learn.chatgpt.com/docs/hooks).

[18] OpenAI (2026). [Advanced configuration](https://learn.chatgpt.com/docs/config-file/config-advanced).

[19] OpenAI (2026). [Feature registry](https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/features/src/lib.rs).

[20] OpenAI (2026). [V2 send_message](https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/core/src/tools/handlers/multi_agents_v2/send_message.rs).

[21] OpenAI (2026). [V2 followup_task](https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/core/src/tools/handlers/multi_agents_v2/followup_task.rs).

[22] OpenAI (2026). [V2 messaging dispatch](https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/core/src/tools/handlers/multi_agents_v2/message_tool.rs).

[24] OpenAI (2026). [Extension footer rows](https://github.com/openai/codex/pull/60032).

[26] OpenAI (2026). [Remote board configuration](https://raw.githubusercontent.com/openai/codex/rust-v0.169.0/codex-rs/features/src/feature_configs.rs).

[28] OpenAI (2026). [MCP Events](https://developers.openai.com/plugins/build/mcp-events).
