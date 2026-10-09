# Other harnesses and adapter prior art

<!-- facet: other --> <!-- facet: prior -->

Startup is never blocked; context is appended to the prompt for this turn only. [11]. <!-- claim: 71d0ab9411524a64; evidence: 4a71c6b4dc708da3; source: e214abd2fa5d6182 -->

OpenCode plugins are JavaScript or TypeScript modules returning a hooks object. [12]. <!-- claim: 345509fac214c295; evidence: 2e3b2a2538e0e2cf; source: 4c56732288be166a -->

Pi session_start reasons include startup, reload, new, resume and fork. [13]. <!-- claim: c03040732421aa4e; evidence: f86ec0733cb92c26; source: fba6670183aa1c1b -->

OpenCode shell.env has optional sessionID and callID. [14]. <!-- claim: a8a728f13458c4a5; evidence: 43cc31ed3e83e531; source: 0ffd40e2f15d989d -->

ACP capabilities omitted during initialization are unsupported. [15]. <!-- claim: 0f1099d2ce56c565; evidence: ec91f9309cd6ca88; source: ff99c3f9a5934cf3 -->

The current Claude ACP adapter uses the official Claude Agent SDK. [16]. <!-- claim: b77c522bbc2e4fac; evidence: 7692a2ef05fba5e2; source: cc710ab394c03e9d -->

The current Codex ACP adapter starts the Codex App Server. [17]. <!-- claim: 12510b108f9af56e; evidence: 3841fb3c35f6397e; source: bf2d0b63bdf7f42e -->

hcom documents committing a delivery acknowledgment after stdout is successfully written. [18]. <!-- claim: c419f45e13aae06a; evidence: d4d0bf99bbf77d05; source: 669ae3a1f59b6450 -->

Design inferences: adapters must own language-specific bridge assets and native codecs, not just JSON command declarations. Context output must be event-specific and carry its lifetime/offer semantics. Optional capabilities must be absent by default; a branded adapter cannot promise every version supports every launch form or child path. Detached backends and optional session identifiers need explicit unsupported attribution paths. Session branching is not child-agent provenance.

ACP's capability rule is useful, but adopting ACP would host/replace runtime interaction rather than decorate the existing Herdr CLI agents. hcom is closer in purpose, yet its centralized harness constructors are a concrete example of an edit point to avoid. Keep normalized observations in core and native input/output mapping in each adapter.

Counterevidence and limits: Pi docs have changed repository identity and lifecycle event structure; OpenCode publishes multiple generations of docs. These are interface counterexamples, not promised integrations or minimum-version evidence. No other harness was installed or launched. Prior-art branches are mutable and setup/removal implementation details were not audited. A fake minimal adapter and real Hermes remain the extensibility acceptance test.


## Bibliography

[11] [Gemini CLI hooks reference](https://geminicli.com/docs/hooks/reference/)

[12] [OpenCode Plugins](https://opencode.ai/docs/plugins/)

[13] [Pi extension event types (current main)](https://raw.githubusercontent.com/earendil-works/pi/main/packages/coding-agent/src/core/extensions/types.ts)

[14] [OpenCode plugin Hooks types (development branch)](https://raw.githubusercontent.com/anomalyco/opencode/dev/packages/plugin/src/index.ts)

[15] [Agent Client Protocol v1 Initialization](https://agentclientprotocol.com/protocol/v1/initialization)

[16] [ACP adapter for the Claude Agent SDK README](https://raw.githubusercontent.com/agentclientprotocol/claude-agent-acp/main/README.md)

[17] [ACP adapter for Codex CLI README](https://raw.githubusercontent.com/agentclientprotocol/codex-acp/main/README.md)

[18] [hcom shared hook infrastructure Rust source](https://raw.githubusercontent.com/aannoo/hcom/main/src/hooks/mod.rs)
