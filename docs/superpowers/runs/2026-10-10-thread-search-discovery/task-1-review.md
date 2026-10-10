## Spec Compliance

✅ CLEAN — spec compliant. Canonical name/topic literal OR matching occurs in the bounded candidate loop (`src/store/queries.rs:1763`), with nullable name projected in all three indexed queries. Filtered keys bind the selected instance's dedicated directory/name/all revision (`src/store/queries.rs:1630–1657`); None preserves the original ordinal/recent key strings. No protocol fields or serialized shapes change.

✅ Actual SetThreadName changes publish the revision in the deciding transaction (`src/store/control.rs:1316–1349`); unchanged values skip publication and exact replay skips apply. The approved directory/name/all scope complies with the coordinator correction.

## Strengths

- Production mutation coverage (`tests/store/control.rs:9416–9629`) exercises set, rename out/in, clear, replay, unchanged-name no-op, topic staleness, restart search arguments, unfiltered semantics, and unrelated membership activity. The revision count of four makes unintended replay/no-op bumps observable (`:9572`).
- Literal/case/UTF-8/membership/archive/instance/order cases assert exact IDs (`tests/store/queries.rs:5562`); bounded zero-match pages and complete continuation traversal assert the sole expected name-only result (`:5637`). Legacy filtered cursors are explicitly rejected (`:5707`).
- Real CLI regression asserts both canonical name and created thread ID (`tests/handoff_topology_cli.rs:2124–2150`). The additional file is controller-approved and reuses an isolated fixture.

## Named external risk checks

- Risk: another production name writer bypasses revision publication. Searched thread INSERT/UPDATE/REPLACE sites across src and migrations. Only `src/store/control.rs:1324` updates an existing name; `create_thread_impl` generates a fresh ID before INSERT (`:1450–1457`), while `src/store/service_controls.rs:251–258` returns existing managed threads unchanged and inserts unnamed absent threads. Migration name mentions add the nullable column or observe updates, not rename rows.
- Risk: publication is bypassed by production dispatch or replay performs another bump. Traced `src/store/mod.rs:1586–1587` to set_thread_name and the accountable wrapper (`src/store/schema.rs:3817–3858`); the existing replay path returns before domain apply (`:3804`).
- Risk: new opaque key is generated but not validated. Inspected directory continuation validation and cursor generation (`src/store/queries.rs:1682–1689,1796,1842`): comparison and both generation paths use the same key closure. Existing recent revision trigger still reacts to name writes (`migrations/0020_recent_activity.sql:12–15`), explaining preserved recent unfiltered staleness.
- Expanded the control and directory hunks only where the package context cut off their surrounding transaction/continuation functions; no unrelated code crawl.

## Test changes

Ran the required `git diff --stat BASE..HEAD -- tests` and `git diff BASE..HEAD -- tests | head -400` for base 871521afbe22fd60ad09e67cf0a532e04a5647e2 and head 038407d76c69b7ae28d9739e8d8af45bba78cc8c. Stat: 3 files, 458 insertions, zero deletions. The full test diff display was truncated at 400 lines; remaining additions were visible in the supplied review package. No existing assertions deleted, skipped, or loosened. New assertions are reachable and check real returned results/cursors; production mutations use StorePort::mutate rather than directly altering revision state.

Reviewed reported successful focused evidence: 41 directory, 24 name, 24 recent, and one CLI test; fmt, all-feature clippy, default-feature check and run-scoped leak check succeeded. Did not rerun tests. Initial socket-denied runs were subsequently successful outside sandbox.

## Issues

Critical: none.
Important: none.
Minor: report records `nice: setpriority: Operation not permitted` in sandbox validation (`task-1-report.md`, TDD evidence and final checks). This environment warning does not invalidate successful cargo results; run nice in an environment permitting its priority adjustment if pristine command output is required.

## Assessment

Task quality: Approved. CLEAN.

The localized predicate/projection change preserves traversal and admission behavior, and the separate transactionally published revision prevents rename-sensitive filtered continuation from silently skipping results. No blocking spec or quality findings.
