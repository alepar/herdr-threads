# super-auto run — 2026-10-10-thread-search-discovery

flags: planOneShot=true skipPlanRoast=false skipCodeRoast=false autonomous=true
phase: code

idea: lets debug, then once we understand the problem, $super-auto it, fully autonomous, both roasts on
branch: super-auto/thread-search-discovery
base: main
epic: ht-akx
spec: 2026-10-10-thread-search-discovery-design.md

approvals:
- top-split · ht-akx.1 LEAF, ht-akx.2 LEAF · auto chosen under user authorization “fully autonomous”; records the chosen bounded shape, not an explicit human review of these ids
- coverage-round-1 · canonical R-list: R1 “Directory --search uses case-sensitive literal substring across optional name OR topic; deduplicate dual hits, retain None/empty behavior and literal UTF-8 punctuation semantics.”, R2 “Candidate traversal, high water, row/byte/work bounds, zero-match continuation, ordering, instance and existing membership/archive semantics remain intact.”, R3 “Every actual existing-thread name mutation publishes dedicated name/all atomically; no-op/replay and initial creation keep correct semantics.”, R4 “Filtered recent and ordinal cursors bind name and existing topic revisions, stale and restart correctly after relevant changes, preserve unfiltered/legacy safety and avoid unrelated invalidation.”, R5 “Serialized request/result/cursor shapes and legacy field names remain compatible; top-level search, picker, trust/identity semantics unchanged.”, R6 “CLI help, README, adopted contract amendment and protocol field comment explain exact new contract.”, R7 “Focused production-path regressions exercise relevant literal/scoped/pagination/rename cases and required fmt, clippy and default-feature checks validate change without routine full-suite use.” · requirements: 7 · mapped: 7 · unmapped: 0 · 3/3 valid reviews · auto C1 → extend ht-akx.1 default-feature verification; auto C2 → extend ht-akx.2 parser/contract round-trips; auto C3 → extend ht-akx.1 explicit production mutation/replay/zero-match/isolation/unfiltered cases
- coverage-round-2 · requirements: 7 · mapped: 7 · unmapped: 0 · 3/3 valid fresh-context reviews, all findings empty; C1/C2/C3 verified closed · auto clean exit · trajectory 3 → 0 findings, novel fraction 0/0 (none)

roastDesignRound: 1
roast-design: 2026-10-10-thread-search-discovery-roast-design-1.md

codeBuckets:
  completed:
  escalated:
  pendingRetry:
  parked:
  stalled: false
  review: pending

HT_LEAK_RUN_ID: c494afe8-aa16-4b6b-a2a8-b9bee8e65cb8
