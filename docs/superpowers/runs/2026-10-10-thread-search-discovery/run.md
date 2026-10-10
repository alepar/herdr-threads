# super-auto run — 2026-10-10-thread-search-discovery

flags: planOneShot=true skipPlanRoast=false skipCodeRoast=false autonomous=true
phase: roast-design

idea: lets debug, then once we understand the problem, $super-auto it, fully autonomous, both roasts on
branch: super-auto/thread-search-discovery
base: main
epic: ht-akx
spec: 2026-10-10-thread-search-discovery-design.md

approvals:
- top-split · ht-akx.1 LEAF, ht-akx.2 LEAF · auto chosen under user authorization “fully autonomous”; records the chosen bounded shape, not an explicit human review of these ids
- coverage-round-1 · canonical R-list: R1 “literal name OR topic predicate and edge cases”, R2 “bounded traversal/scopes/order/output unchanged”, R3 “transactional name revision for every actual mutation”, R4 “filtered cursor rename consistency and unfiltered/legacy safety”, R5 “wire and other-command compatibility”, R6 “help/README/adopted-contract/protocol-comment documentation”, R7 “focused production-path regressions and required verification” · requirements: 7 · mapped: 7 · unmapped: 0 · 3/3 valid reviews · auto C1 → extend ht-akx.1 default-feature verification; auto C2 → extend ht-akx.2 parser/contract round-trips; auto C3 → extend ht-akx.1 explicit production mutation/replay/zero-match/isolation/unfiltered cases
- coverage-round-2 · requirements: 7 · mapped: 7 · unmapped: 0 · 3/3 valid fresh-context reviews, all findings empty; C1/C2/C3 verified closed · auto clean exit · trajectory 3 → 0 findings, novel fraction 0/0 (none)

roastDesignRound: 1
