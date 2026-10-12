# Q4 applied-controls baseline failures: scratch controls3 correction

2026-10-12. Scratch-only single-file test correction against the current applied `tests/store/query_work_q4.rs`, independently verified SHA256 `4a5ec6d5411b793030d7886dc68b3aaf075adfcb56c81761d911a5a3a39c5d82`. Current production is frozen while the parent owns another runtime lane; no candidate/source/test edit, Cargo, SQL execution, Git/Herdr action or nested agent occurred here. Parent owns compilation, execution and physical causal experiments.

## Actual baseline retained honestly

Read `/private/tmp/ht-C-resume1012-library9-causal-sql-baseline/execution.json`, run.log and leaks.log. Actual candidate runtime was HEAD `1d1c6112384dbc95b29fdb20da2484522751fd61`, tree `2ba0635e970df5d60bdf0178bd8e5f3f044261f2`. Q4 five: **2 PASS / 3 FAIL**. Three-event-key/floor VM control and actual continued-tail once-only control passed. Point/scalar controls failed the recipient plan oracle; raw control failed visited99 versus100. The entire nine-test selection was2PASS/7FAIL, not Q4-only9. Its other four failures remain outside this scratch change.

Build/list phase was **184.649799s**, a multifile build above the incremental budget; retain that result as184.65s, not a successful controlled incremental benchmark. Selected runtime was12.919255s, exit100; scoped leak check exit0 under UUID `39a338ac-792f-4868-a26d-676ce7412a11`. The run manifest and logs are hashed in the new source seal. Neither failed Q4 observation is represented here as a newly proved production bug.

## Recipient oracle: my omitted valid schema key

The actual plan prints `SEARCH r USING INDEX sqlite_autoindex_lazy_recipients_1 (preparation_id=? AND seat_id=?)`, reached through the exact canonical manifest lookup. Migration0026 declares both UNIQUE(preparation_id,seat_id) and UNIQUE(message_id,seat_id). My controls2 oracle accepted only the latter, incorrectly rejecting a complete unique point. Controls3 explicitly accepts either complete equality tuple. Both columns remain mandatory in either alternative; a lone preparation/message/seat prefix or any range-only SEARCH is still refused. No autoindex number is hardcoded. All other alias/count/FullscanStep/VM/branch controls stay intact; SQL and fixtures are unchanged.

## Raw fixture: source-generated continuation, not invented initial progress

Current applied raw test calls `ordinary_presentation_page(...,None,...)`; source capture initializes lazy_after_ordinal0. Therefore ordinal1-skipping is not an observed initial-cursor cause. Source inspection identifies the boundary: query_mode checks `source==Warnings && CANDIDATE_LIMIT-visited<2` before its source_done skip. After Lazy visit99, rotation through exhausted other sources reaches Warnings and can stop before returning to Lazy for the100th candidate, even with warning exhaustion already established. This static attribution is consistent with actual visited99; the successor has not run to confirm its returned cursor/second quantum.

Controls3 follows the actual helper-returned cursor **unchanged**, beginning with None, until this fixed100-candidate slice exhausts in at most two real quanta (current source predicts99+1). It does not construct a different initial cursor, delete a row, start after ordinal1, enlarge a cap, change production or claim one fresh page visited100. Each quantum retains cap<=100, exact scope=3*visits, no ordinary continuation charge and no retired body. The returned cursor must carry inspected progress from ordinal0, fixed high-water100 and no body tail. The aggregate must be100 physical visits,300 scope probes and25 exact rendered IDs msg-q4-0..24. A connection-local trace additionally requires exactly100 executions of actual PRIVATE_LAZY_ELIGIBILITY_SQL and zero continuation-scope executions, killing omitted/duplicated eligibility SQL even across the follow-up quantum.

Whole raw100 traversal VM work, including real capture, cursor validation, returned-cursor traversal and fitting, is still compared between noise128/8192. Exact-ID Due14 remains independently measured after it, with prior no-double-charge expectations and constant VM comparison unchanged. The log now labels raw100 rather than falsely labeling the aggregate as one fresh quantum, and records actual quantum count. Same literal population, high-water, IDs, episode/publication cutoffs, growing relation noise, source counters and caps are preserved.

## Artifacts and validation boundary

- `resume-v090-q4-query-controls3.patch`: `8a7dd94d6b53165e2f5387fa3b5da3b1260cac04d59e3f4e3c23de0e409c2a4b`
- `resume-v090-q4-query-controls3.rs`: `be76a431aa3ca8e04da76d30829de4afe7a9e8edaa69ed70cbade9c2f78e56d1`
- `resume-v090-q4-query-controls3-source-seal.json`: `bf0f06b69c898976f53fb86840aaff083ac177541567a4bfaf0ae5d1ada4a4df`

This is an **incremental single-file patch against applied controls2**, unlike controls2's original module/mount replacement. Only the recipient unique-key alternatives and raw traversal/counter/log block change. Source hashes and actual baseline logs are sealed; applied module still equals its preimage after scratch assembly.

Scoped rustfmt parse/format and --check exited0 (edition2024, skip_children=true). No successor compilation, fixture, plan, VM, trace or runtime PASS is claimed. Obtain scoped review before parent application; rerun all five Q4 controls, retain baseline2PASS/3FAIL and184.65s evidence, and separately perform the previously proposed real omitted-cursor-thread-key causal experiment. A setup/compiler failure is not a physical causal RED. This patch alone does not recertify full256-key caller work, public/completion bounds, latency, controlled incremental speed or merge readiness.
