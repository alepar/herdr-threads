# Pi harness research

Recommendation: implement upstream pi as a terminal-only, channel-woken HarnessAdapter. Reuse opencode’s channel-only core after it lands. Do not equate sendMessage success with delivery: the adapter needs explicit context evidence and a state machine that cannot turn passive mail into an idle wake. These are design recommendations, not upstream guarantees.

## Decision evidence

<!-- facet: F1 -->
Extensions are discovered globally under the pi agent extensions directory and in trusted project-local .pi/extensions directories. [1] <!-- claim: 7926e6b1bc897929; evidence: 75f1329a58b7abc7; source: 458882967aaf8636 -->

<!-- facet: F1 -->
Session replacement shuts down the old extension instance and starts a new one; start reasons distinguish startup, reload, new, resume and fork. [1] <!-- claim: 015f91ae16a225d8; evidence: a2e50e9bae70efff; source: 458882967aaf8636 --> [3] <!-- claim: 015f91ae16a225d8; evidence: 42bb598f3c044834; source: c155a99b7cd580d6 -->

<!-- facet: F3 -->
The extension sendMessage API returns void, with steer, followUp and nextTurn delivery options. [3] <!-- claim: e4f138f4584f0a9e; evidence: 86ad88fb37b6d3d4; source: c155a99b7cd580d6 -->

<!-- facet: F3 -->
The implementation handles nextTurn first. Otherwise, when streaming and triggerTurn is not false, it selects followUp if requested or steer otherwise. With no streaming run, triggerTurn true can start one. [2] <!-- claim: 4cf44d3e83484434; evidence: 316b625ab33adf12; source: acc3443733afabb8 -->

<!-- facet: F3 -->
agent_end is not final idle; agent_settled is the documented final-idle signal. [1] <!-- claim: 3ce0caf0acca4e0a; evidence: 2227af8cd1e2361b; source: 458882967aaf8636 -->

<!-- facet: F3 -->
The implementation distinguishes active-run state from idle: idle also excludes compaction. [2] <!-- claim: b4ed0f948f865efb; evidence: 1ac99f294e6317a4; source: acc3443733afabb8 -->

<!-- facet: F3 -->
Extension message callbacks precede custom-message entry insertion; the agent context transformation precedes conversion to provider messages. [2] <!-- claim: b06eb77aa8392e7d; evidence: 823d604fbe3e18ed; source: acc3443733afabb8 --> [8] <!-- claim: b06eb77aa8392e7d; evidence: 0a7d35769748f403; source: c8186231332ca614 -->

<!-- facet: F3 -->
Session entry persistence can be deferred until conversation content exists. [7] <!-- claim: 4dd19dbae81cffb6; evidence: e855ebcc5f1bfba1; source: 81c8aeb92fa5effb -->

<!-- facet: F2 -->
The extension context distinguishes tui, rpc, json and print; hasUI is true for both TUI and RPC. [3] <!-- claim: 9fac5b9cbe217fc5; evidence: 483f4ee86d6a818f; source: c155a99b7cd580d6 -->

<!-- facet: F2 -->
PI_SESSION_ID is assigned the result of ctx.sessionManager.getSessionId(). [14] <!-- claim: 7ed45546dcd7309a; evidence: 378b072aab745c82; source: 820a295bc7723a91 --> [9] <!-- claim: 7ed45546dcd7309a; evidence: a4902f203ac73292; source: bdb4424801a5f0d7 -->

<!-- facet: F2 -->
The cited session-manager code restores the session ID from a header, creates a new session ID, and assigns the active leaf pointer. [7] <!-- claim: b1b076785eeacbaa; evidence: 90aa1cb2c97301d1; source: 81c8aeb92fa5effb --> [7] <!-- claim: b1b076785eeacbaa; evidence: 324bf29fbf63c3df; source: 81c8aeb92fa5effb --> [7] <!-- claim: b1b076785eeacbaa; evidence: 382566ea17eb67c1; source: 81c8aeb92fa5effb -->

<!-- facet: F2 -->
The subagent-example excerpt defines an argument array containing --mode json, -p and --no-session; the cited issue discussion recommends passing a custom CLI flag to subagent invocations. [17] <!-- claim: 18d8d07fe9c0862d; evidence: 5aeaace5d53f855f; source: 6c98051b4bb1b75a --> [20] <!-- claim: 18d8d07fe9c0862d; evidence: a6780503d9de1b76; source: c77c685c1ef77189 -->

<!-- facet: F4 -->
PI_CODING_AGENT_DIR overrides the default ~/.pi/agent configuration directory; auth.json is resolved beneath the selected agent directory. [9] <!-- claim: 7ed2fc68612c8b93; evidence: 37ddb01af918f554; source: bdb4424801a5f0d7 --> [19] <!-- claim: 7ed2fc68612c8b93; evidence: fee0bb0b9a737ce6; source: 80a0e277e5fa77b9 -->

<!-- facet: F4 -->
OpenRouter supports OPENROUTER_API_KEY; the separate DeepSeek provider uses DEEPSEEK_API_KEY. [13] <!-- claim: ce1d9f99bf9132eb; evidence: d081c4191a4f608a; source: 216620f68d93ea0f --> [18] <!-- claim: ce1d9f99bf9132eb; evidence: b5efaf4107f9ea3e; source: b2bb11719683b105 -->

<!-- facet: F4 -->
The model selector accepts provider/id with an optional thinking suffix; --provider restricts model lookup and requires --model. [10] <!-- claim: 326e648a812abcd0; evidence: 255f84f09a6d7dc7; source: 02931a1a3c55a4f0 --> [10] <!-- claim: 326e648a812abcd0; evidence: 6e4e7fe832380134; source: 02931a1a3c55a4f0 -->

<!-- facet: F4 -->
SDK runtime replacement requires subscriptions to be rebound to the new AgentSession. [12] <!-- claim: 7e39ed34b51d25d8; evidence: 9ab0bd3b2d6efaac; source: 6278041c35c0c602 -->

## Proposed design and verification

The companion design (`../design.md`) uses a session-fenced watch child, exact frame tracking, context-based ACK, a bounded local pending buffer and sorted native boundary drafts, and a content-free channel wake only when the daemon admits it. Completed-run boundaries can request one more model step; ambiguous, failed or aborted phases retain pending mail. No composer fallback is proposed.

The [extension dossier](extensions.md) covers delivery and lifecycle. The [identity dossier](identity.md) covers modes, session claims and provider isolation. The [real-SDK probe](candidates/probe/results.md) confirms the important queue/entry/context distinctions with a scripted provider; it also demonstrates the stale-busy wake hazard. Native RPC was tested only for credential-free boot and state reporting. These probes are supporting experiments, not a qualification of the TUI or live provider.

## Limitations and retrieval stop

This is Deep-mode primary-source research, with one upstream implementation as the authority. Docs, source and local experiments are complementary but not independent vendors. No exact pi version is proposed as an admission gate. Open gaps are real TUI draft/modal behavior; complete tool/retry/compaction/reload race coverage; process-table classification across npm/native/wrapper distributions; third-party interactive child contracts; and credentialed OpenRouter/DeepSeek wake qualification.

Retrieval stopped as **budget-exhausted**, after the primary-source pass, counterevidence checks and a review-driven boundary-API follow-up. This is not a coverage-saturation or exhaustive-recall claim. All four source facets have supporting evidence; the remaining gaps are explicitly retained for design review and implementation qualification.

## Methodology

Search discovery used the search CLI and host web fallback. Primary passages were subsequently retrieved through approved CLI-only search extraction. Two sandboxed searches and extract attempts failed; repeated original-document accesses are logged independently. Counts and failures below come from the persisted ledgers.

| Measure | Recorded value |
|---|---|
| Search attempts | 8; two failed in the sandbox |
| Result entries considered | 39 assigned to individual queries, plus 12 in one combined two-query batch; individual allocation in that batch is unknown |
| Original-document fetch attempts | 57 |
| Distinct original documents read | 24; mirrors normalized |
| Failed fetch attempts | 3 |
| Claim support | 38/38 supported by fresh judges configured as gpt-6.1-terra; no lexical fallback |
| Bibliography links | 14/14 accessible under the network-enabled citation check |

All supported factual claims link to registered raw-extract evidence. Source paths/locators and extraction hashes preserve auditability; upstream main/latest URLs are moving references, not version requirements. The unavailable guessed singular `/session` route is not relied on: session behavior is supported by retrieved source and other official documentation. Recover failed discovery queries through approved search when broader retrieval is needed; use the actual sessions documentation route rather than the failed guessed URL.

### Retrieval health

**Status: DEGRADED.** 2 search attempt(s) failed; 7 provider failure type(s) reduced search coverage; 1 document(s) still inaccessible.

- Search preflight: ok (search) - CLI capabilities inspected; first deep scoped query succeeded with all providers; Q1 later failed all providers with network errors, switched to host web tool
- Search attempts: 8 (2 failed, 6 success)
- Document fetches: 57 attempts, 24 distinct documents read, 3 failed attempts, 1 recovered, 1 still inaccessible
- Partial text only: 3 document(s)
- High-priority quotes: 50 (50 verbatim from search -m extract, 0 non-verbatim fallback, 0 provenance unknown)
- Claim support check: model; 38 claim(s) judged by model (gpt-6.1-terra), 0 unverified by model

Provider failures (search still returned results from other providers):

| Provider | Category | Queries affected |
|---|---|---|
| brave | network | 2 |
| brave_llm_context | network | 2 |
| exa | network | 2 |
| kagi | network | 2 |
| perplexity | network | 2 |
| serper | network | 2 |
| tavily | network | 2 |

Failed searches:

| Query ID | Query | Category | Error |
|---|---|---|---|
| Q1 | site:github.com/earendil-works/pi sendMessage triggerTurn before_agent_start system prompt | network | All seven providers failed network requests |
| IMQ1 | site:pi.dev/docs/latest session PI_CODING_AGENT_DIR OpenRouter RPC | network | All seven providers network-failed under sandbox |

Documents still inaccessible:

| URL | Tools tried | Category | Error |
|---|---|---|---|
| https://pi.dev/docs/latest/session | search -m extract, web.run | not_found | Redirect target inaccessible |

Recovered after a failed attempt:

| URL | Failed with | Recovered by |
|---|---|---|
| https://pi.dev/docs/latest/extensions | search -m extract (network) | search -m extract |

Recommended actions:

- **network** (https://pi.dev/docs/latest/session, provider brave, provider brave_llm_context, provider exa, provider kagi and 5 more): Transient network failure: rerun the affected queries or fetches.
- **not_found** (https://pi.dev/docs/latest/session): The URL is dead or moved: find a current or archived copy.

## Boundary-based revision

<!-- facet: F5 -->
The documented turn_end boundary runs after assistant and tool-result persistence. [1] <!-- claim: 3af85b504852d4e7; evidence: ff9eee237a7fa5a2; source: 458882967aaf8636 -->

The documented boundary continue option ensures one next provider request. [1] <!-- claim: 9f2a5e77002fb1fc; evidence: a5b045a2cd64e867; source: 458882967aaf8636 -->

The first design review found queue survival across reload, cross-batch FIFO ordering, and capacity gaps. Revised recommendation: keep bodies in a bounded extension buffer and return a sorted custom-message draft at before_agent_start, turn_end or agent_before_settle; use sendMessage only for the content-free idle wake. This is a design inference, not a universal Pi guarantee.

The extended [real-SDK probe](candidates/probe/results.md) asserts nine boundary cases. Completed-run draft continuation reaches the next provider request within one outer run. The tested specimen unexpectedly permits a pre-settle proposal to continue after synthetic error/aborted responses, despite the documentation’s hard-exit wording. Guard explicitly on outcome equal to completed. Actual cancellation can still commit a draft without continuation; no ACK follows without later exact context evidence. TUI, actual provider and lifecycle race qualification remain open.

## Bibliography

[1] [extensions.md — upstream pi](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/extensions.md)
[3] [types.ts — upstream pi](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/src/core/extensions/types.ts)
[2] [agent-session.ts — upstream pi](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/src/core/agent-session.ts)
[8] [agent-loop.ts — upstream pi](https://github.com/earendil-works/pi/blob/main/packages/agent/src/agent-loop.ts)
[7] [session-manager.ts — upstream pi](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/src/core/session-manager.ts)
[14] [resolveSpawnContext — upstream pi](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/src/core/tools/bash.ts)
[9] [Shell Tool Session Environment — upstream pi](https://pi.dev/docs/latest/environment-variables)
[17] [runSingleAgent — upstream pi](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/examples/extensions/subagent/index.ts)
[20] [maintainer comment — upstream pi](https://github.com/earendil-works/pi/issues/4469)
[19] [getAgentDir — upstream pi](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/src/config.ts)
[13] [OpenRouter — upstream pi](https://pi.dev/docs/latest/providers)
[18] [getApiKeyEnvVars — upstream pi](https://github.com/earendil-works/pi/blob/main/packages/ai/src/env-api-keys.ts)
[10] [Invocation and output — upstream pi](https://pi.dev/docs/latest/cli)
[12] [Session lifecycle — upstream pi](https://pi.dev/docs/latest/sdk)
