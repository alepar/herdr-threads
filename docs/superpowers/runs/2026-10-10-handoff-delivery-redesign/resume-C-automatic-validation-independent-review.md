# Scoped automatic validation review

Status: QUIESCENT. No actionable defect found in the reviewed production delta or its added expiry control. This is an independent source/evidence review by a fresh reviewer, not a test execution, whole-C clearance, merge approval, or Main/v0.90 adaptation review. No subordinate reviewers were used.

## Exact scope

- Inspected clean helper: `ROOT/.worktrees/finish-composed-automatic-validation`, where ROOT is `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign`.
- Production commit: `342be90d9788dce419f3e670f9b1a0a9dc7dede4`, tree `4562700c48c056481ce0e942d5b632c363a7e4e8`.
- Test-only parent: `71bb7ea92b820321b03b9c4206cd2e4a2fb23011`, tree `5fe2b03c0be7414ea2fcd0751920213e1175a27a`.
- Prior measured source: `f330bdab3abee8535213228e5465b085b65be750`, tree `5576d25210259cada7b4b00c484c33693369f228`.
- Production delta is exactly 11 additions / 1 deletion in `src/store/handoff/automatic.rs`; parent adds 95 test lines in `tests/cli/topology_handoff.rs`. Reviewed the actual Git diffs, source, and normative TRUST-POLICY.md cooperative model, A2, and automatic original/linkage/retention requirements.

## Specification and quality assessment

`current` retains `request.validate()` then `guard_child(..., true)` before its child reader. `validate_phase` retains its initial `guard_child(..., replay)`, then performs the same request validation previously reached through `current`, and invokes the private child reader. The removed work is solely the second `guard_child(..., true)` in that phase path.

The retained initial guard dominates the omitted guard: with replay true it is identical; with replay false it performs the common parent/linkage validation and additionally requires live parent state. The guard's replay escape only skips live checks for completed children or Ensure/Send. No SQL writes or time-dependent admission checks occur between the retained and formerly repeated guard. Inspected canonical mutation paths run within the immediate deciding transaction (`connection.rs:350`, `schema.rs:4391`), so a second read does not establish a newer canonical view. The change does not cache approval across transactions.

I independently compared the extracted child-reader body with the parent's body and found it byte-identical. It still bounds retained bytes, validates typed originals, rejects lost JSON fields, compares full canonical original bytes, verifies companion columns and all five child key registrations, and requires the legacy fence. The new helper is private and has only the two guarded callers. Parent strict decoding, full retained-state hydration, attachment equality, key lookup without unlinked fallback, namespace comparison, and live/replay handling remain present in `topology_handoff/automatic.rs:424` and its callees.

`validate_phase` retains child thread equality, completed/live decisions, sender live mapping, and recipient eligibility. `control::decide_accountable` still invokes the handoff requirement with replay false inside the deciding transaction before canonical cooperative authorization. No caller evidence, provenance, permit TTL, publication timing, original grant, operation identity, admission geometry, or accepted trust limit changes. The validator's meaningful error order is preserved: initial parent error, request error, then child reader/phase error. Eliminating a repeated read can naturally eliminate a second transient database interruption opportunity; that is not a validation bypass.

The new test obtains a real attached modern child, issues one actual permit, waits until the live SystemClock is at least 251 ms beyond a post-issuance sample, and submits that same grant. It asserts `CallerUnverified` / `permit expired`, unchanged counts in seven effect-bearing tables and exact parent status, then succeeds with a newly issued grant for the identical operation and checks one exact operation and child companion. This is a meaningful negative control, rather than freezing time or enlarging TTL. Counts plus parent status are targeted atomic-refusal evidence, not an exhaustive byte snapshot of every database table. The unchanged production expiry check remains `age > MAX_PERMIT_MILLIS`, with MAX_PERMIT_MILLIS = 250 (`authority.rs:129,295`).

## Execution evidence actually inspected

These are parent-owned recorded executions, not reviewer reruns. Each directory's `execution.json` records the exact source/tree, binary hash, selected leaves, command, exit and leak result; `run.log` corroborates test outcomes. Recorded binary hashes identify the binaries at execution time; I did not rebuild or independently reproduce their hashes from source.

| Artifact directory | Recorded outcome |
| --- | --- |
| `/private/tmp/ht-C-corrected-and-diagnostic-lib-first` | At f330bdab: 6 actual leaves, 4 passed / 2 failed, run exit 100. Both modern full-envelope leaves fail at auto_child_begin with `permit expired`; diagnostics report live decision ages 311 ms and 265 ms. Historical over-limit replay and the other three controls pass. |
| `/private/tmp/ht-C-expired-child-baseline` | At 71bb7ea: actual expiry/refusal/fresh-grant test passes, run exit 0. |
| `/private/tmp/ht-C-automatic-single-guard-green` | At 342be90d: actual admission-boundary, maximum escaped-body, and expiry-control leaves all pass, run exit 0. |
| `/private/tmp/ht-C-automatic-double-guard-causal-control` | At cbf0acb7: the same three actual leaves also pass, run exit 0, 14.980 seconds recorded command wall time; leak exit 0, run ID `dec59576-9256-4fb9-bec2-3bba2db06f3b`. |

All four execution records report leak exit 0; the inspected successor, expiry-control and mutant leak logs explicitly say `no leaked test processes`. These establish the reported executions and preservation of the tested expiry behavior. They do not establish timing headroom or isolate the removed guard as the cause across variable machine load.

The one-variable mutant `cbf0acb79756ab6b5c19b65dadb910f6d30d0483`, tree `0659c147f4eef8f7ee293ba89fb53432531fdd76`, was source-verified: its only difference from 342be90d is restoring `guard_child(db, request, true)?` immediately before the private child reader. Its completed execution metadata, run log, and leak log were subsequently inspected: all three leaves PASS. Therefore there is no successful causal RED control. The duplicate parent hydration is real redundant work and the optimization preserves reviewed semantics, but these timing-dependent results do not prove it caused or fixes the historical permit-expiry failures. The earlier RED/GREEN pair cannot support an “expiry failure fixed” claim. No retry-until-red exercise is justified by this review.

## Findings and limits

No confirmed correctness, trust-policy, replay, error-order, or maintainability finding within this narrow change. Specifically rejected after source inspection: weakening live parent authorization, bypassing exact original/registration checks, moving authority to local hints, extending TTL, and changing the request/parent error precedence.

The remaining runtime question is evidence scope, not a demonstrated source defect: both optimization and restored-guard mutant pass, so causal performance resolution remains unproven, and one successful optimized run does not guarantee the 250 ms budget under every load. The final Main-adapted candidate needs its own actual validation. Required project lint/neutral/default checks, broader acceptance, and integration remain parent-owned and are not certified here. I ran only read-only source/Git/evidence inspection, a byte-equality comparison, and `git diff --check 71bb7ea 342be90d` (exit 0). I ran no Cargo/build/test, wrote no source or Git state, used no Herdr action, and started no background process. This report is the sole authored file.
