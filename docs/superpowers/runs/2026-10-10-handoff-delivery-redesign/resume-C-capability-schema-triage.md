# C capability/schema causal triage

Read-only diagnosis; no Cargo, source, SQL, Git, Herdr, process-lifecycle or seat mutations. Only this report was written. Applied `systematic-debugging` Phase1–3: read actual failures, traced expected values to fixtures, compared exact definitions, and performed isolated in-memory SQLite experiments.

## Source and actual evidence

Candidate: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/upstream-composition-478ec4a9`.
HEAD: `b046c123be452957d3f7f10d8fbdc60b18a4533e`; tree: `ecc9977a535a932b5514ab8820650d1c832691a5` (read with `git rev-parse`).
Actual evidence: `/private/tmp/ht-C-composed-b046c123/nextest-run.log` and `execution.json`. Run ID `28b15c51-3941-483f-83b0-2ef768028f03`. Parent reports385 focused executions,377 passes/8 failures, six mandatory passes, scoped leak check0; this report owns only the three failures below.
Candidate `TRUST-POLICY.md` read for cooperative attribution, direct-pair policy, bounded invitation presentation and upgrade40/41 accepted work. No policy change is warranted by these failures.

## Two capability expectations are stale

`tests/protocol/capabilities.rs:269` (`capability_constants_are_stable`) and `:561` (`bare_daemon_advertises_legacy_capabilities_without_v2_recorder`) both fail solely because actual advertisement includes `direct_threads.ensure_creation_policy_v1` immediately after `thread.direct_v1`, while their exact literal expected lists omit it. No removed entry, extra second entry or reordering remains.

The capability is defined at `src/protocol/capabilities.rs:31` and intentionally included in `ADVERTISED` at:86. `tests/protocol/capabilities.rs:353` already probes it via `EnsureDirectThread` with explicit Lazy creation policy. Production `src/store/control.rs:558` validates the policy, freezes it into the accountable digest, passes it to canonical `resolve_direct_pair`, and requests the explicit-policy result shape. The policy is normative in TRUST-POLICY. Therefore removing advertisement would break the intentional negotiated feature; the causal correction belongs in the two exact expectation lists.

Smallest experiment already completed: decode each actual left/right list and compare set/order after inserting this one literal at the prescribed slot. The actual diff contains exactly that insertion. Parent-owned focused Rust rerun should cover these two names plus `every_advertised_capability_has_a_handler`; no production capability edit is proposed.

## Schema failure is a stale latest-version fixture expectation

Actual failing assertion: `tests/store/effective.rs:1212`, in `dormant_upgrade_actual_v31_backfill32_reopens_and_rolls_back_without_recapture`. Despite its historical name and assertion text, fixture construction replays actual immutable migrations29–34 over a populated actual v28 file and records `user_version=34`; `schema::initialize` now advances through41. The fixture derives exact expected warning-origin DDL from37 and already explicitly accounts for39's four body-bearing table rebuilds. Its additive-object exclusion list stops at38 and its old-table transformation stops at39. It does not account for intentional40/41 additions.

The actual rollback branch reaches and passes before this failure: temp obstruction rolls migration35 back, leaves version34/FK setting/main marker absence intact, and preserves the exact pre-upgrade catalog. Failure occurs only after physical reopen and successful initialization to LATEST_VERSION.

The giant Rust debug left/right values were decoded using Python `ast.parse` plus a strict node walker accepting only Constant/List/Tuple and one-argument `Some(...)` calls; no eval or SQL/data interpretation of arbitrary log content. Tuples are `(type,name,tbl_name,rootpage,sql)`.

Exact extracted difference:

- Actual649 objects versus expected531.
- Exactly118 actual-only objects; zero expected-only objects.
- Exactly one shared-object SQL difference: `invitations` has `command_context_epoch INTEGER NOT NULL DEFAULT 0` appended after `inviter_seat_id`, as required by migration40.
- Every shared object's rootpage remains exactly equal after the existing explicit39 capacity normalization. Every other shared SQL definition is byte-equal.

Isolated causal experiment: reconstructed the expected pre40 schema from the logged expected declarations in a new Python in-memory SQLite connection (skip SQLite-owned declarations), supplied the excluded immutable38 `receipt_warning_origins` table declaration, and executed the exact candidate40 and41 SQL. No real database or file was changed.

| SQL stage | New tables | New indexes, including autoindexes | New triggers | Total |
|---|---:|---:|---:|---:|
|40|3|10|2|15|
|41|3|6|94|103|
|Combined|6|16|96|118|

The combined new-object name set is exactly the actual-only118 set. All118 final `(type,name,tbl_name,sql)` definitions exactly match the actual failure catalog. The invitations SQL after40 exactly matches actual. The final `invitation_presentation_consumers` SQL after41 exactly matches actual, including all four checked default-zero scalar columns. At the intermediate40 stage that consumer table naturally differs from final41; after41 there are zero definition mismatches. Thus these failures show no production catalog drift.

Migration39 is already covered by the existing fixture's precise capacity replacements/rootpage exception for only its four explicitly rebuilt tables; no additional39 correction is needed. Released Main composition first-parent baseline is `7c4753208f46b27e8483427ceeeb44a89760b916`. All31 migration files numbered1–31 were individually compared as bytes against `git show BASE:migrations/NAME`: zero differences. A name-only migration diff from that baseline to candidate lists only40 and41.

## Narrow fixture correction proposal; no implementation made

1. Insert only `direct_threads.ensure_creation_policy_v1` into the two exact capability literals at the actual prescribed slot. Keep their exact list/order assertions.
2. Extend the schema fixture's expected transformation with the exact migration40 invitations ADD COLUMN and exact40/41 newly introduced catalog definitions. Keep old-object membership, SQL and rootpages exact, retain the existing tightly scoped39 rebuild exception, and reject any unknown addition/removal/definition change. Do not simply exclude all `invitation_*`/`presentation_*` objects or accept actual SQL as the oracle.
3. Build the new-object definition oracle independently by applying the exact immutable40/41 SQL to a scratch schema derived from the captured old definitions (plus separately excluded38 dependencies), as proven by the experiment above. Compare all118 exact type/name/table/SQL tuples and require the exact complete new-object set; new-object rootpage allocation is a creation property rather than old-object continuity. Any handling of newly allocated pages must be explicit and confined to these exact declared new names; existing object rootpages must remain in the original equality assertion. Alternatively, clone the actual pre-upgrade physical fixture and derive a complete expected migration path when parent can preserve an independent exact39 expectation. No whole-catalog rootpage normalization.
4. The next retained-row assertion also needs a precise expectation extension: `receipt_upgrade_snapshot` uses `SELECT rowid,*`, so the old invitation row gains one final40 default-zero INTEGER. Append exactly `Value::Integer(0)` to the captured expected invitations rows; preserve every preexisting value, ordering and rowid. Do not drop the new column or compare only selected historical fields. All other captured table shapes remain unchanged by40/41. Update the assertion's stale additive-range description if desired; this is test-only wording.
5. Parent should run this one schema test with both FK settings exercised by its existing loop, verify the retained default-zero epoch, rollback preservation, physical reopen, backfill-marker nonrecapture and second initialization, then run the focused capability probes and required per-change checks. This agent has not run Cargo and makes no passing-test claim.

Production assessment: no production issue is demonstrated by these three actual failures; all observed catalog/advertisement differences are intentional implemented additions. Do not repair migration SQL, modify1–31, remove the capability, weaken old-catalog equality or broaden39's rebuild allowance. The remaining five actual failures belong to other owners.
