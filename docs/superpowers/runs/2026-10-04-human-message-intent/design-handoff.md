Root epic: ht-nmp. Caller owns code handoff; super-code was not invoked here.

Settled leaves (all direct children, open, label sp:ht-nmp):
- ht-nmp.1 shared compilable fields/projections/fixtures, schema22/wire5; first ready.
- ht-nmp.2 CLI/canonical send eligibility; needs1.
- ht-nmp.3 pure deterministic ledger lifetimes/guards/fold; needs1.
- ht-nmp.4 cumulative summary runtime/fallback/pinning/generations; needs1,3.
- ht-nmp.5 transcript/ledger/guidance/trust; needs1.
- ht-nmp.6 executable source configuration smoke; needs2,4,5.
- ht-nmp.8 withdrawn/replaced query regression/guidance corrections; needs4,5.
- ht-nmp.9 frozen fetched bundle capture/replay/reset; needs1,4.
- ht-nmp.7 focused-only terminal integration sweep; needs every other leaf.

Exactly two coverage rounds: requirements11/mapped11, findings6→0, widening no. Promotion review retained3/4 as cohesive leaves with sp:demoted-by-session reasons; all decisions settled before code.

Design roast reports1,2;2 rounds. Final clean (0 nits) [converged]; no unresolved degradation/caps/escalations or punch list. Round1 snapshot Should-fix and query-withdrawal Blocking were patched after independent step-back and cleared in round2. Accepted semantic/fallback limits preserved; no eventual semantic closure or error-repair guarantee.

graph-pass: depth 5→5 · width 1.8→1.8 · applied 0 · parked 0

Exactly ONE forward migration:0022_user_message_intent.sql, schema21→22. Nullable messages.user_intent; nullable summary_transitions.rule_change; nullable summary_jobs.fetched_bundle_json. Historical migrations unchanged. Wire4→5; submission1→2; renderer1→2; worker thread-summary-v2; fallback daemon-fallback-v2. Old blocks retained readable and isolated; new jobs/leases enforce version fences. Frozen bundle never regenerates for compatible setting changes and resets lease acquisition/replacement.

UserIntent independent of source attribution; all human intent attention priority retained. Unclassified None legacy ledger retained. Stable i.seq IDs; seed before transitions; cumulative bundle versus local record ownership. Query/request Open→Resolved (query explicit human withdrawal/replacement allowed); rule Active→Superseded only explicit human assertion+quote; legacy None Open→Done|Superseded. Semantic interpretation is worker judgment.

No code or migration implementation, test run, live model, Herdr mutation, real config write or owned daemon/helper occurred in design phase. Only read-only source/tracker inspection and short artifact/tracker/git commands. git diff --check passes. Coordinator owns combined full suite, main merge and release; existing coordinator parked note preserved. Parent owns sequential ordinary-subagent code scheduler and checks.
