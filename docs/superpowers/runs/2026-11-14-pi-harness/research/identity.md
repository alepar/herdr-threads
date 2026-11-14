# Identity, modes and qualification

These findings describe retrieved upstream source and documentation. Proposed integration decisions are distinguished below from upstream guarantees.

<!-- facet: F2 -->
The extension context distinguishes tui, rpc, json and print; hasUI is true for both TUI and RPC. [3] <!-- claim: f4b2f9c2ebb7f90a; evidence: 483f4ee86d6a818f; source: c155a99b7cd580d6 -->

<!-- facet: F2 -->
PI_SESSION_ID is assigned the result of ctx.sessionManager.getSessionId(). [14] <!-- claim: 653665ff2b3d0245; evidence: 378b072aab745c82; source: 820a295bc7723a91 --> [9] <!-- claim: 653665ff2b3d0245; evidence: a4902f203ac73292; source: bdb4424801a5f0d7 -->

<!-- facet: F2 -->
The cited session-manager code restores the session ID from a header, creates a new session ID, and assigns the active leaf pointer. [7] <!-- claim: abd4072683faeee7; evidence: 90aa1cb2c97301d1; source: 81c8aeb92fa5effb --> [7] <!-- claim: abd4072683faeee7; evidence: 324bf29fbf63c3df; source: 81c8aeb92fa5effb --> [7] <!-- claim: abd4072683faeee7; evidence: 382566ea17eb67c1; source: 81c8aeb92fa5effb -->

<!-- facet: F2 -->
The npm pi command maps to dist/bundle/cli.js, while CLI setup sets the process title and PI_CODING_AGENT marker. [16] <!-- claim: 5a51b347fc40d758; evidence: 8f722fc334e9862b; source: 9c7c725a75af593f --> [15] <!-- claim: 5a51b347fc40d758; evidence: 701c608ac6ab8cff; source: 3c51d17fd4c5012a --> [15] <!-- claim: 5a51b347fc40d758; evidence: ff8639cc850d58b0; source: 3c51d17fd4c5012a -->

<!-- facet: F2 -->
The subagent-example excerpt defines an argument array containing --mode json, -p and --no-session; the cited issue discussion recommends passing a custom CLI flag to subagent invocations. [17] <!-- claim: a83c46645030114e; evidence: 5aeaace5d53f855f; source: 6c98051b4bb1b75a --> [20] <!-- claim: a83c46645030114e; evidence: a6780503d9de1b76; source: c77c685c1ef77189 -->

<!-- facet: F4 -->
PI_CODING_AGENT_DIR overrides the default ~/.pi/agent configuration directory; auth.json is resolved beneath the selected agent directory. [9] <!-- claim: ac572232b65a514c; evidence: 37ddb01af918f554; source: bdb4424801a5f0d7 --> [19] <!-- claim: ac572232b65a514c; evidence: fee0bb0b9a737ce6; source: 80a0e277e5fa77b9 -->

<!-- facet: F4 -->
OpenRouter supports OPENROUTER_API_KEY; the separate DeepSeek provider uses DEEPSEEK_API_KEY. [13] <!-- claim: 973a2c54df134857; evidence: d081c4191a4f608a; source: 216620f68d93ea0f --> [18] <!-- claim: 973a2c54df134857; evidence: b5efaf4107f9ea3e; source: b2bb11719683b105 -->

<!-- facet: F4 -->
The model selector accepts provider/id with an optional thinking suffix; --provider restricts model lookup and requires --model. [10] <!-- claim: 6352b5d6580437c7; evidence: 255f84f09a6d7dc7; source: 02931a1a3c55a4f0 --> [10] <!-- claim: 6352b5d6580437c7; evidence: 6e4e7fe832380134; source: 02931a1a3c55a4f0 -->

<!-- facet: F4 -->
SDK runtime replacement requires subscriptions to be rebound to the new AgentSession. [12] <!-- claim: 1b15c3e8b882fc9f; evidence: 9ab0bd3b2d6efaac; source: 6278041c35c0c602 -->

## Design implications and counterevidence

Admit explicit terminal mode, not hasUI. Use the native session ID plus extension-owned session/PID/mode claims under the existing cooperative attribution policy. The normal shell environment includes process.env, so inherited plugin variables are feasible; arbitrary spawn hooks and remote shell tools can remove them. This is an inference requiring tests, not a universal shell contract.

The official subagent example does not establish a universal role signal. Reject excluded nonterminal modes and stale inherited ownership. Third-party interactive children and concurrent SDK embeddings require their own cooperative contract; ordinary TUI mode alone cannot prove absence of all custom children.

## Limits and qualification

A credential-free native RPC boot and SDK probe are available in the [probe report](candidates/probe/results.md). They do not qualify the interactive process-table matcher, TUI behavior or OpenRouter model output. A live run requires coordinator authorization and an isolated mode-0600 credential file; an OpenRouter key must not be relabeled as a direct DeepSeek key.

## Bibliography

[3] [types.ts — upstream pi](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/src/core/extensions/types.ts)
[14] [resolveSpawnContext — upstream pi](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/src/core/tools/bash.ts)
[9] [Shell Tool Session Environment — upstream pi](https://pi.dev/docs/latest/environment-variables)
[7] [session-manager.ts — upstream pi](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/src/core/session-manager.ts)
[16] [bin / build:binary — upstream pi](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/package.json)
[15] [setupCli — upstream pi](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/src/cli/setup.ts)
[17] [runSingleAgent — upstream pi](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/examples/extensions/subagent/index.ts)
[20] [maintainer comment — upstream pi](https://github.com/earendil-works/pi/issues/4469)
[19] [getAgentDir — upstream pi](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/src/config.ts)
[13] [OpenRouter — upstream pi](https://pi.dev/docs/latest/providers)
[18] [getApiKeyEnvVars — upstream pi](https://github.com/earendil-works/pi/blob/main/packages/ai/src/env-api-keys.ts)
[10] [Invocation and output — upstream pi](https://pi.dev/docs/latest/cli)
[12] [Session lifecycle — upstream pi](https://pi.dev/docs/latest/sdk)
