# Task7 scoped rebased seam review

Spec CLEAN. Quality CLEAN.

Reviewed frozen integration BASE `07305ed8634ffac0e7cb34bd354ac700284089b8` through rebased HEAD `ef048d163d6e2b014b8390c0384f5eaca963a202`, against approved original BASE `a95623dc6a6f5a84d48b2d54e09d8701bc978b25` and HEAD `c7ccb29c6b23cc1420e345a13d892f65db13ad09`. Scope is composition, preservation and new seam breakage; original task review and I1 CLEAN rereview remain the leaf approval. No Critical, Important or Minor seam finding.

## Independently verified composition

Read task brief/report, original review, I1 rereview, seam context and the complete 164597-byte seam package. Independently compared Git tree entries rather than relying on the operator's blob list:

- Exactly 30 paths comprise original and rebased task deltas; both path sets match.
- Every one of those 30 preimages on original BASE is identical to its integration BASE preimage. Thus no intervening task-file edit was overwritten by preserving an old final blob.
- Every one of those 30 final blobs is byte-identical between approved original and rebased HEAD. This covers all runtime hunks, port changes, schema wiring, migration35, TRUST-POLICY and all test additions, rather than only preserved migration text.
- Every non-task tree entry at rebased HEAD is identical to integration BASE, including all inherited Task8/Task9 CLI/setup work. No unrelated path was introduced, removed or changed by rebasing.
- All migrations1..34 are identical to integration BASE. Migration35 is the same approved additive DDL; its three tables, four explicit indexes and ten triggers match the fixture catalog increments. No old migration, DDL cutpoint or rootpage comparison was replaced.
- Inherited `integrations/claude/mod/hooks/register.js`, `tests/handoff_topology_cli.rs` and `tests/integration/topology_activation.rs` are identical to integration BASE. The unprefixed mod log and stronger bounded/cumulative handoff staging and upstream topology continuation fixtures survive exactly.

These checks cover all 17 conservative all-parent overlap paths listed in the context, including TRUST-POLICY, attention/receipts/schema/wake runtime and every listed store/integration fixture. The empty first-parent overlap claim does not substitute for these checks.

## Focused producer and consumer checks

- `src/store/receipts.rs:551` retains ordinal-leading physical SQL, SQL-enforced high water and LIMIT; `:571` retains the shared four-producer repair allowance; `:656` retains the combined completion reset inside the decision transaction. The complete scan body was consulted because the package hunk ends before its final `more`, cursor assignment and commit behavior. Cursor publication still happens only after successful `execute_decision`; no rebased runtime hunk is missing.
- `src/store/materialization.rs:40` remains the exact inherited Task5 producer API, including local one-shot completion. The new combined repair consumes that API without moving its local reset semantics. `src/store/invitation_due.rs:194` remains the inherited invitation scanner signature and result producer consumed by `src/store/mod.rs:2334`.
- `src/ports.rs:386`, `:410` and `:471` preserve the approved internal sparse ordinal/completion/maintenance interfaces; `src/store/mod.rs:432`, `:471`, `:2291` and `:2428` preserve import/export and paired-cycle full-turn scheduling. `src/scheduler/deadlines.rs:208` still submits the100 cap and `:226` retains immediate successful continuation for an unfinished skipped peer. No wire/persisted cursor format is added by this composition.
- `migrations/0035_receipt_warning_origins.sql:49`, `:75`, `:103` and `src/store/receipts.rs:553` retain exact native author/instance and receipt-to-open-condition predicates; sparse origins retain exact published manifest/prepared-recipient guards. `:127` retains passive final-clear addressing and origin deletion. `src/store/schema.rs:3792` retains the existing exact required/pending/unwaived physical-plus-sparse shared-condition closure check. No caller, service label or current membership is substituted for provenance.
- Named early-gate/mod compatibility risk: inspected unchanged actual seat discovery and mod consumers because a downstream predicate alone could miss an upstream exclusion. `src/store/mod.rs:1081`, `:1096`, `:1127` retain same-instance live non-human finite seat walking and use the actual `seat_has_pending_rows` probe before attention/loading. `src/store/attention.rs:1078` includes receipt origins alongside inherited invitation origins; `:1194`, `:1220`, `:1253` and `:1269` retain independent bounded canonical origin selection/frontiers. `src/store/wake.rs:196` consumes the combined origins beyond informational offer settlement. Unchanged `src/store/seats.rs:4568`/`:4641` consumes the same narrowed warnings for real mod fingerprinting. Existing binding/reservation/native guards are unchanged blobs; no admission or ACK provenance shortcut is introduced.
- `src/store/schema.rs:669` retains transactional35 installation and strict object verification. `src/service/kicks.rs:108`/`:120` retains explicit wake/deadline classification for origin/projection/repair work, with no unknown-table error suppression.

## Mandatory test-change audit

Read the mandatory20-file stat (1824 additions/79 deletions), capped first400 test-diff lines and ALL1958 remaining test-diff lines in sequential passes. No omitted tail, removed test, ignored case, relaxed timing/process budget or weakened assertion was found.

The catalog/latest guards advance to35 while true prior-version construction/cutpoints stay historical. Named additive-object exclusions preserve exact old definitions/rootpages. The retirement adaptation asserts exact20 receipt sources plus one invitation instead of only the old blind count. The ordinal dormant-arrival adaptation preserves full100 progress, actual four-stream accounting, captured arrival discovery during repair and every old association, rather than preserving the invalid newest-arrival-first assumption.

The I1 coverage fix survives byte-for-byte: `tests/store/receipts.rs:3844` completes initial real reservations; `:3928` checks exact actor/generation/whole observation and complete receipt/message history; `:3959` reruns actual discovery/load/mod after A1, A2 and B1; `:3999`/`:4008` prove no outstanding reservation and exactly one passive clear. `tests/scheduler/poke_flow.rs:1073` uses real canonical cooperative ACK; `:1121` checks all actual final paths; `:1178` removes only scheduling dedup as a negative control and `:1203` proves the actual scheduler submits no new native prompt. The result cannot be explained by a stale reservation, dedup frontier or unrelated waking obligation.

Four-stream/H-D-I fairness, genuine populated34 rollback/reopen, reusable raw rowid versus monotonic sparse episodes, atomic combined reset failure, terminal-history charging and real producer VM-work checks remain in the full diff. Assertions and conditional bounds are unchanged from the approved original; native guard/outage tests still distinguish durable warning detection from key submission.

No Cargo, tests, hooks, Herdr actions, source edits, branch/index mutations or subagents were used. Git commands were limited to the specifically requested independent preservation/tree checks. The requested report is the only file written. Original runtime evidence is not claimed as verification of the rebased tip: the coordinator still owns the exact-tip integration gate, later task seams/sweep and parent final readiness/default-feature checks.
