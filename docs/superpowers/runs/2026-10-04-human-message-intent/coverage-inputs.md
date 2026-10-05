## Goals

### ht-nmp (root)
Make thread transcripts and summaries distinguish human queries, one-off requests and rules, while preserving honest attribution. An agent recovering context should see unanswered questions, unfinished requests and applicable rules without treating every human message as a permanent instruction.

Status: approved design; implementation contracts finalized autonomously. The user approved the names `query`, `request` and `rule`, explicit recorded classification chosen by the forwarding agent, guidance in `ht skill`, summary-time resolution, preserving attention priority for all human intents, and legacy behavior for unclassified messages. Cross-chunk visibility is an explicit requirement described below. Implementation has not started.

## Task tree

- ht-nmp · Recorded human message intent and summary lifetimes (epic) · Implement the approved human query/request/rule intent design independently of relay attribution, preserving legacy messages and attention priority. · deps: none
  - ht-nmp.1 · Seam contract: recorded intent and rule-change storage · Land compilable optional UserIntent and RuleChange types with message/result/journal/bundle/item/transition plumbing, one schema22 migration and wire5 boundary, · deps: none
      owns: UserIntent/RuleChange optional serialized contract, nullable schema22 fields and audits, wire5 fence, None-preserving projections and digest/retry plumbin
      consumes: approved design contract.
  - ht-nmp.2 · Classified human sends and attribution validation · Expose --user-intent query|request|rule and validate canonical ordinary-human or relaying-agent eligibility without changing human attention priority or receipt · deps: ht-nmp.1
      owns: CLI intent option to canonical send validation, exact send outcomes, rejected service/system/unrelayed-agent paths.
      consumes: optional serialized intent and schema22 send contract.
      boundary contract: ht-nmp.1
  - ht-nmp.3 · Intent-aware deterministic ledger lifetimes · Derive one stable i.seq record per priority source, preserve unclassified legacy behavior, apply query/request resolution and active-rule withdrawal/replacement · deps: ht-nmp.1
      consumes: UserIntent/RuleChange body and transition types.
      boundary contract: ht-nmp.1
      owns: classified status/evidence guards, duplicate rejection and sequence-ordered folding.
  - ht-nmp.4 · Cumulative summary jobs and compatible generations · Wire intent-aware records through cumulative level0 bundles, local-block persistence/fallback, fetched_at replay, Ready/rollup pinning and renderer2/submission2 · deps: ht-nmp.1, ht-nmp.3
      consumes: intent-aware prefill/fold/validator and optional source/transition contract.
      boundary contract: ht-nmp.1
      owns: cumulative bundles, local persistence/fallback, historical decoding, spilling/budgets and active/open pinning.
  - ht-nmp.5 · Human intent transcript and agent guidance · Render independent attribution/intent markers and actual summary ledger tokens and update ht skill, CLI help, agent usage and trust policy for explicit forwardi · deps: ht-nmp.1
      owns: every transcript/ledger presentation and guidance consumer, three examples, mixed-input splitting, uncertainty, no permission inference, unchanged Codex g
      consumes: optional intent body/types and schema2 worker contract.
      boundary contract: ht-nmp.1
  - ht-nmp.6 · Configuration smoke: direct human, relaying agent, legacy and refused sources · Exercise direct human, relaying agent, unclassified legacy, unrelayed-agent and service/system refusal configurations through send/storage/JSON and summary entr · deps: ht-nmp.2, ht-nmp.4, ht-nmp.5
      consumes: completed classified sends and cumulative summary jobs.
      owns: configuration matrix, focused regression commands and safe process cleanup.
  - ht-nmp.7 · Integration sweep: human message intent · Verify main flows end to end, add uncovered seam tests and fix small gaps, then run only focused tests and required fmt/clippy/default-feature checks. · deps: ht-nmp.1, ht-nmp.2, ht-nmp.3, ht-nmp.4, ht-nmp.5, ht-nmp.6
      owns: terminal uncovered-seam tests and merge-ready focused verification.
      consumes: all leaves (integration sweep).
