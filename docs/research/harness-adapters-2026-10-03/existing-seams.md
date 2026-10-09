# Existing seams and preservation constraints

<!-- facet: existing --> <!-- facet: proof -->

The migration must remove repeated harness selection throughout the application. This is an architectural recommendation based on the code audit, not an implemented change.

Existing harness identity is a closed enum containing Codex, Claude and Human. [2]. <!-- claim: 1ab804703aed2ab2; evidence: 138e1c6040271eaf; source: 88dbd502ca1eaead -->

The duplicate enum in `harness/context.rs` serializes its variants differently from protocol identity. CLI selectors are fixed Clap arrays; some conversions default every non-Codex input to Claude. `InstalledHarness` couples Claude strings and Codex's typed admission witness. Hook decoding, native output envelopes, command guidance and lifecycle handling branch in `cli/hook.rs`. All are one-time migration seams; existing persisted spellings need explicit compatibility tests.

Durable SQL is another migration seam: occupant binding and version-evidence/unattributed tables have closed harness checks (migrations 0009 and 0012); legacy schema verification also closes the harness set, while inbox display eligibility has a two-agent SQL whitelist. Replace live agent predicates with a registered-agent/kind decision without letting arbitrary database strings confer authority; migrate the closed schema once and preserve tests reopening every supported old schema. Do not edit historical migrations in place.

Registry consumers also include `contract_for`, version normalization, transcript attribution, ladder selection, `daemon/harness_states`, PATH observation in `app.rs`, Health's fixed two-field harness struct, doctor rendering, setup loops, managed launch argv, host wake recognition and notification poke selection. The evidence gate and daemon classify lifecycle/tool events by fixed native names. Move class, lifecycle buffering and required verification milestones into adapter metadata; preserve lifecycle-plus-tool verification for Claude/Codex. Hermes native observer payloads and bridge attachment/turn envelopes need distinct declared contracts/evidence descriptions, so a generated bridge envelope is never called an observed native payload. Evidence validation currently permits only alphanumeric event names: Hermes underscore names require a general event-name contract. `SessionStart` is hardcoded in evidence holding/recording and must become adapter metadata without weakening Codex resumed-rollout behavior.

Setup spans `cli/setup.rs`, JSON ownership transactions in `harness/setup.rs`, Codex TOML sandbox ownership and Claude prompt-suggestion ownership. Keep those concrete implementations; move their invocation behind the adapter, rather than replacing proven transactions with a configuration framework. Preserve old manifests, interrupted-install recovery, adoption rules, foreign hook conflict checks, exact removal, permission refusal and changed-baseline detection.

Record claims as claims; decide against one canonical view; use only evidence that exists. [1]. <!-- claim: e582eee719aaf8ee; evidence: dfb0a87826c3293c; source: 2da7696b21931f12 -->

The daemon keeps seat allocation, holds, continuity, receipts and binding decisions. Human remains a distinct occupant, with operator_human provenance and no agent hooks. Adapters may normalize observations and claim roles; they cannot move seats, grant receipt authority or infer continuity from labels or transcript ancestry. Managed launch remains a wake-only placeholder until a cooperative check-in.

The running harness records its own version in the session transcript. [3]. <!-- claim: 5197e801306efa73; evidence: 28a193378fbdb33e; source: d72a9eefa47e1d7c -->

Claude tail-version attribution and Codex creator-version/resume suppression must survive migration. PATH observations remain installation/admission evidence, not runtime-version attribution. Recipe IDs, contract hashes and evidence tiers retain their meanings.

The canary currently selects only claude, codex or both. [4]. <!-- claim: a17d4b5d565df43d; evidence: 26207cee00c45bf3; source: ba9c2e5befc15cf7 -->

Its npm-based install/discovery cannot simply be applied to Hermes. Move discovery and runtime probe descriptors to registry output; allow adapter-owned probe/install companions, preserving existing Claude/Codex canary cases. Do not turn a skipped Hermes installer or missing model credentials into PASS.

Baseline checks: the isolated worktree passed 13 cooperative harness tests and 18 contract tests. No product changes, full suite, native Hermes interaction or owned servers were started. The `nice` calls ran the tests but the sandbox denied priority adjustment.

Open gaps: approved protocol/output compatibility strategy, exact Hermes release floor and a native acceptance matrix. This is source/test evidence, not a migrated implementation.


## Bibliography

[1] [herdr-threads trust policy](https://github.com/alepar/herdr-threads/blob/4b026b382b063e0795bc27a4befca94c988eeb80/TRUST-POLICY.md)

[2] [Existing harness identity and dispatch](https://github.com/alepar/herdr-threads/blob/4b026b382b063e0795bc27a4befca94c988eeb80/src/protocol/authority.rs)

[3] [Harness version evidence design](https://github.com/alepar/herdr-threads/blob/4b026b382b063e0795bc27a4befca94c988eeb80/docs/design/herdr-threads/2026-10-02-harness-version-evidence-design.md)

[4] [Existing harness canary](https://github.com/alepar/herdr-threads/blob/4b026b382b063e0795bc27a4befca94c988eeb80/scripts/harness-canary.sh)
