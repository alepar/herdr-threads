# Harness adapters: research and architecture preview

Recommendation: a small registry of Rust adapters, with an admitted recipe object and typed optional operations. Each adapter owns native configuration, bridge assets, input decoding, output encoding and launch argument policy. The daemon keeps all canonical seat/receipt decisions. This is a design proposal awaiting user approval; no product implementation has started.

<!-- facet: existing -->
The current harness identity is a closed Claude/Codex/Human enum. [2]. <!-- claim: 17c9d3cdee99988a; evidence: 138e1c6040271eaf; source: 88dbd502ca1eaead -->

The audit found repeated selection in CLI, hooks, setup, observation, Health, evidence, host wake and canary code. A wrapper around the two parser functions would leave those extension points scattered. [Existing seam audit](existing-seams.md) records the migration surface and old manifest/serialization obligations.

<!-- facet: hermes -->
parent_session_id is excluded from extra by _TOP_LEVEL_PAYLOAD_KEYS. [6]. <!-- claim: 14042a7158338b54; evidence: c3334ac950b21e2e; source: 94c13c730f6c770f -->

A small Python callback bridge is the recommended Hermes backend: it can forward bounded identity facts and receive a context response, without transferring history or replacing the harness. [Hermes dossier](hermes.md) records native consent, profile, runtime-version and lifecycle limits.

<!-- facet: other -->
OpenCode plugins use in-process JavaScript or TypeScript callbacks. [12]. <!-- claim: 4967935f5e986384; evidence: 2e3b2a2538e0e2cf; source: 4c56732288be166a -->

This is a concrete counterexample to requiring every adapter to install a JSON shell hook. Gemini and Pi add differing context/lifecycle semantics. [Other harnesses and prior art](other-harnesses-and-prior-art.md) explains the implications and unsupported assumptions.

<!-- facet: prior -->
Codex ACP starts the Codex App Server. [17]. <!-- claim: 9f1a1e5a102bf24a; evidence: 3841fb3c35f6397e; source: bf2d0b63bdf7f42e -->

ACP capability negotiation is useful precedent; converting the product to ACP is a larger runtime change. Keep native hooks and adapter-local bridges.

## Approaches and tradeoffs

| Approach | Advantage | Cost |
|---|---|---|
| Registry + trait + admitted object (recommended) | Cohesive harness module; native variation stays local; core discovers support | One deliberate cross-cutting migration now |
| Retain closed enums and add a facade | Small initial diff; compiler exhaustiveness | Each future harness needs enum, CLI, Health and evidence edits; misses the user criterion |
| External plugins or ACP service | Independent distribution; standardized external protocol | Process/runtime/permission ownership and protocol complexity exceed this task |

Proposed interface sketch (names illustrative): `HarnessAdapter` supplies its ID, metadata/contracts, bounded observation/admission and setup backend. `AdmittedHarness` supplies version/recipe-bound decoding and encoding plus optional launch/composer capabilities. No public downcast to Codex or Claude in core. Unsupported operations return a named reason, not default success. Classification is available even when admission refuses, preserving violation evidence.

The public author interface remains one `HarnessAdapter`; optional typed providers and admitted decoder objects are implementation details composed within its module. Registration is explicit and static; no inventory/linker discovery or dynamically loaded Rust libraries.

Data flow: registry lookup → bounded admission → adapter decode into check-in intent/role/observation → existing canonical daemon decision → bounded neutral offer → adapter event-specific encode → successful output flush → existing offer bookkeeping. Observer-only hooks cannot consume an offer. Render fixed guidance and escaped peer data centrally; native wrappers and context timing stay adapter-local.

Use one registry declaration for selectors, host-kind recognition, enumeration, contract/version discovery and built-in registration. Replace the two duplicated identities with a registry-backed agent ID and retain Human as a separate occupant, with compatibility serializers for old wire/context spellings. Health and doctor iterate registered IDs rather than fixed fields; output/protocol compatibility is a design decision requiring explicit approval.

Setup/unsetup/status share baseline/fingerprint/ownership and atomic file helpers. Each adapter supplies its concrete backend, optional assets, native enable guidance and extra settings policies. Reuse Claude JSON and Codex JSON/TOML transaction code. Hermes plugin-file staging can avoid rewriting YAML/consent entirely: installation and native enabling are distinct states, and launch refuses a disabled installation. Optional manual instructions must be actionable and native-tested.

Launch keeps Herdr's correlated guarded start and managed_launch semantics in core. Codex's --no-daemon, wrapper probe, subcommand insertion, approval flags and uncaptured-resume refusal become adapter-local. Claude preserves existing argv and hook behavior. Hermes permits only captured interactive forms; preserve caller bytes/order and profile selection, and refuse uncaptured overrides. The SQL migration removes per-brand checks from active schema and inbox-display eligibility, while validating agent IDs and role/provenance in canonical decisions. Existing legacy schema verifiers stay version-aware so upgrades preserve all old seats and receipts.

Ordinary wake requires matching registered host kind and idle/done recheck. Hermes soft pokes remain unsupported until composer/turn evidence exists.

## New-harness author contract

Implement one adapter module containing metadata, contracts/recipes, observation, setup backend/assets, event decode/encode, runtime attribution and supported launch policy. Register it once. Generic CLI/Health/daemon/host discovery must then work without core branches. Add fixtures and native evidence beside it. A genuinely new native primitive can require a shared capability addition; host recognition needs independent Herdr support; automatic release-canary installation may need an adapter-owned installer companion. These are explicit exceptions, not routine per-harness core switches.

<!-- facet: proof -->
The existing canary accepts only claude, codex or both. [4]. <!-- claim: 8d38e9595ce54cf7; evidence: 26207cee00c45bf3; source: ba9c2e5befc15cf7 -->

The npm-specific canary needs generic registry discovery and adapter-owned probe strategy; a Python/source installation is not interchangeable with npm. Release rows and contract IDs must remain generated from the adapters, and unperformed model validation remains skipped/inconclusive.

Evidence acceptance after approval: migrate Claude/Codex unchanged under targeted regression tests; register a deliberately minimal fake adapter and exercise selector/setup-status/contract/doctor/host lookup without core edits; install Hermes bridge in isolated HOME/HERMES_HOME and a private named Herdr session; observe native discovery, enable/refusal, launch, context delivery, child suppression and cooperative accept/read/ACK. Capture reset/resume/compaction separately and state every unverified limitation. A configured file or replayed payload is never native PASS. Run targeted tests, clippy, default-feature check, format/diff checks and scoped process leak checks. Independent architecture/implementation review precedes coordinator-owned serialized merge and final sweep.

## Material decisions for the user

Hermes lifecycle proposal: first role-qualified pre_llm_call performs Startup/attach; later turns Current; explicit reset is deferred until a qualified Clear callback. Role-incomplete observers never register or consume offers. Existing history does not imply Resume continuity. Pre-tool callbacks can record contract evidence, but do not promise mid-turn context delivery. Callback timeout may discard context, so successful bridge output remains an offered claim, never receipt. Validate timeout/replay/lost-output paths.

Recommended scope: Hermes interactive CLI with explicitly selected/default profile, small Python bridge, turn-boundary delivery, installed/enabled/observed status and measured cooperative interaction. Defer Gateway/Desktop/TUI, exact compression callback parity, unproven automatic restored-seat resume continuity and composer pokes. Broader parity is possible only with additional captured APIs, not inferred events.

Recommended release intent: 0.3.0 for this pre-1.0 architectural/API change, unless the user intends the public stability commitment of 1.0.0. Coordinator chooses and publishes the approved version later; this branch never bumps/publishes on its own. The concurrent CLI naming/v0.2.2 task stays separate.

## Method and limits

Primary-source browsing and installed/source code audit; independent bounded retrieval for other harnesses and prior art; lead checked selected original sources. Upstream Hermes is pinned; other harness/prior-art docs remain dated mutable counterexamples. Native probes and exact version floors await design approval. Targeted baseline: 13 cooperative and 18 contract tests passed. No full suite or real user configuration writes.

Retrieval stopped **budget-exhausted** at the preliminary decision budget, not claimed coverage saturation. High-priority unresolved facets: Hermes complete native/API parity and extensibility/native acceptance proof. Remaining gaps are listed in the dossiers. Further product scope and native probing depend on design approval; source evidence supports the architectural recommendation now.


## Bibliography

[2] [Existing harness identity and dispatch](https://github.com/alepar/herdr-threads/blob/4b026b382b063e0795bc27a4befca94c988eeb80/src/protocol/authority.rs)

[4] [Existing harness canary](https://github.com/alepar/herdr-threads/blob/4b026b382b063e0795bc27a4befca94c988eeb80/scripts/harness-canary.sh)

[6] [Hermes shell hook serializer](https://github.com/NousResearch/hermes-agent/blob/ea81748579ee1732d214ccb75f91d22208ed623d/agent/shell_hooks.py)

[12] [OpenCode Plugins](https://opencode.ai/docs/plugins/)

[17] [ACP adapter for Codex CLI README](https://raw.githubusercontent.com/agentclientprotocol/codex-acp/main/README.md)
