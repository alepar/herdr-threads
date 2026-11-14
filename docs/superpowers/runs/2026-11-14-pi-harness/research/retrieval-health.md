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
