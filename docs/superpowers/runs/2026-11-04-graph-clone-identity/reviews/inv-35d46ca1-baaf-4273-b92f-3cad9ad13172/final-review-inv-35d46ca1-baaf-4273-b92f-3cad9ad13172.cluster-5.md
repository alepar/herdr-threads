# Final review, cluster 5: archived review bundle inv-4ebdbb23

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 1b058307490f29ece8bb4e3fe7a862f8356d992f, both verified with `git rev-parse --verify`. The source binding for inv-35d46ca1-baaf-4273-b92f-3cad9ad13172 was accepted before this file was written. The caller checkout read clean at the head at the start and before this write; no commit or status change was observed.

Scope: the 31 report files under `docs/superpowers/runs/2026-11-04-graph-clone-identity/reviews/inv-4ebdbb23-9aba-4f61-ae91-b4a977ad8e63/` (30 cluster reviews and the seam review of head 0755e609d), added by 2fda2cca6. They hold no product behaviour. The cluster's risk is that the record is wrong or incomplete, or that an item it requires was closed without being resolved.

Executed here: git and `bd` reads, and read-only reads of herdr-graph at 0dbe477. No build, test or probe was run; each disposition below rests on reading source at the head.

## Integrity of the record

- All 31 files are byte-identical to the workspace copies under `.superpowers/sdd/ht-ma6i-plan/`.
- `run.md:70` lists 206 files for the bundle, with `complete: true` and no gaps. Every listed sha256 matches its blob at the head.
- 0755e609d, the head the reports name, is an ancestor of the reviewed head. The line numbers in the reports are point-in-time, and the seam report's header says which head they belong to.
- The reports contain no credential pattern and no absolute home path. Committing review evidence under `docs/superpowers/runs` matches existing runs.
- `run.md` holds 31 `finding-source` rows for this invocation and 17 `finding-route` rows, one for each landing item in the seam report.
- Every landing item in a cluster report reappears in the seam report: c1 as 15; c2, c7, c12, c14, c15, c18 and c25 as 2 to 4; c3 as 5; c4 as 9 and 2 to 3; c5 as 12 to 14; c8 as 6 to 8; c16 as 10; c28 as 16; c29 as 17; c30 as 11.
- Every child of ht-ma6i is closed, and the stale ht-j7a1 and ht-j40h are closed. The epic is the only open bead tied to this work.

## Disposition of the 17 archived landing items at the head

1. v3 client: resolved. `src/client/service.rs:547` registers `service_session_v3`, and `result_matches` has the v3 arms (`:952-964`). `register_negotiates_service_session_v3` covers it.
2. G5: resolved in code. `src/identity/reconcile.rs:271-274` plans the three actions, and `target_absence_evidence` is at `src/store/seats/observation.rs:1445`. Whether the behaviour is correct belongs to the src clusters.
3. G2: resolved in code. `src/identity/shell_continuity.rs` exists, `src/identity/repair.rs:726` decides, and `src/service/workers.rs:2555` wires the lane.
4. Sweep: `tests/clone_identity_sweep.rs` exists, is registered in `Cargo.toml:28-30` with `required-features`, and has a nextest group (`.config/nextest.toml:239`). ht-ma6i.11 is closed.
5. Hermes: not resolved the way the archived item offered. See the section below.
6. Capability read: resolved. `PresentedKey::Unknown` (`src/cli/site_capability.rs:42-85`) re-reads definitively, and an unread capability ends the enrollment with a retry (`src/cli/hook.rs:1939`, `:1984`), not a keyless fallback.
7. Own pane: resolved. `is_own_pane` takes `on_own_server` (`src/cli/caller_location.rs:344`), and both callers compute it (`src/cli/mod.rs:1321`, `:4045`).
8. Subagent gate: resolved. `resolve_arm_key` derives the origin from `parsed` (`src/cli/site_capability.rs`), and `caller::is_top_level` serves a branch outside the guard.
9. Inspect panic: resolved. `evidence_or_none` is at `src/store/queries.rs:1190` and is used at `:1449`; the `inspect` helper in `tests/store/reversible_facts.rs` drives the public read.
10. Re-issued facts: resolved. `episode_fact_applied` (`src/store/seats/reversible.rs:322`) guards both facts (`src/store/seats/publication.rs:504`, `:538`), pinned at `tests/sync/incarnations_scenarios.rs:1561`.
11. Parked replay: resolved. `TRUST-POLICY.md` under *A node that does not know the reversible kinds* bounds the replayed shift by `unretired_at` and states the clock-skew exception.
12. Inherited key limit: resolved. The limit now reads "Unsetting the variable does not lift the hold" and lists the real exits.
13. "Shell continuity" term: resolved. It has a C1 row (`TRUST-POLICY.md:214`), a section (`:461`) and a provenance value (`:717`).
14. C1 count: resolved. Line 206 reads "Only these ways", with no number.
15. Refusal table: resolved. `docs/programmatic-service-client.md:51-52` list the retryable `ServiceBusy`.
16. B3 test: resolved. `tests/store/restored_pane.rs:721` now seeds and asserts a requirement and drives `bindings::disconnect_with_hooks`.
17. Freeze generator: resolved as filed. `UNCHAINED_KINDS` moved to `tests/sync/peer_wire_fixtures.rs:33`. The script's remaining nonzero exit has another cause, covered below.

## The Hermes item and baseline row B1

The archived item offered two exits: a variable name that passes the filter, or a human scope change row. The head took neither. It keeps the name, records an accepted limit (`TRUST-POLICY.md:2254-2258`) and corrects the documents. `run.md:19` records that as a degraded verdict, "no human scope row", and the spec's human scope changes still read "none".

I do not rate this undelivered. B1's source, design §4 D1, promises the key for the clones the graph launches: "The graph launches every clone agent". herdr-graph's `Harness` enum at 0dbe477 is `Claude | Codex` (`src/model/harness.rs:8-11`), and its source and the H2 design contain no mention of Hermes. So the request's own conditions do not reach a Hermes gateway, and the archived item read B1 more widely than its source. Delivery on the Claude and Codex paths is the seam reviewer's to trace.

The limit is honest and consistent across `integrations/hermes/README.md:67`, `integrations/hermes/__init__.py:37` and TRUST-POLICY, and the setup notes no longer promise it. It is still an agent's decision the owner has not seen, so it goes forward as a recorded item for the user at finish.

## The freeze check's remaining exit

`scripts/freeze-peer-wire 3.0.0 --at v3.0.0 --check` still exits 1, per the ledger's run-failure rows. The cause predates this epic: the committed `tests/fixtures/peer_wire/3.0.0/MANIFEST.md` records `at: 5a4e28abe`, the tag resolves to 95e9ebac8, and no fixture under that directory changed in the range. The ledger still reads "gap — open" with no owner. This closes the attribution, not the defect: `RELEASE.md:56` prescribes that check after every tag.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- This review ran no build, test or probe; the 17 dispositions above are by reading source at the head, and test evidence for them belongs to the task reports and the other clusters.
- Seam reviewer, B1: no real Claude or Codex run has carried `HERDR_THREADS_CONTINUITY_KEY` from launch env to hook; `run.md:23` says the source design's §6 qualification owns this, but that list (cold restart, live handoff, downtime close, close then undo) does not name it, so the check has no named owner, and the sweep uses stand-in harnesses.
- `agent_exit_with_pane_open_keeps_the_seat` has new assertions that were never run against a mutated exit path (ledger, ht-ma6i.23), so its power to fail is unproven.
- `scripts/test-claude-mod` was not re-run here; `integrations/claude/mod/tests/frame_golden.test.ts:22` now lists `sender-label`, which matches the archived cause.
- Archived untested items that fall in other clusters, such as the keyed client paths and the multi-host keyed flows, were not rechecked here.

## Deferred OK
- B1 and Hermes gateway: the accepted limit at `TRUST-POLICY.md:2254` was an agent decision with no human scope row (`run.md:19`), and it is outside B1's source because herdr-graph launches only Claude and Codex; state it to the user at finish, since the archived review asked for a name change or a human row.
- `RELEASE.md:56`: `scripts/freeze-peer-wire 3.0.0 --at v3.0.0 --check` exits 1 on `at:` and `commit:` lines, because the 3.0.0 fixtures were frozen at 5a4e28abe before the tag at 95e9ebac8; this predates the epic, and a follow-up should have `--check` ignore those lines as it ignores `generator:`, and give the ledger's open run-failure an owner.
- Archived deferred items still standing at the head have no bead (`bd` lists only the epic as open for this work): `src/store/seats/continuity.rs:588` reads lag on the pane's host, `clear_alert_only_if_owner_changed` has one caller, `shift_pending_deadlines` leaves `available_at` unmoved, `tests/cli_env_read_scan.rs` omits the key variable, and `KEY_CONFLICT_CAUSE` still classifies by string prefix; file follow-ups at finish, or they survive only in this archive.
- `integrations/hermes/test_bridge.py:480` still stubs a client-env key that Hermes 0.22.1 and later cannot produce; the test covers only an older or hypothetical path and should say so.
- `TRUST-POLICY.md`'s amendment log has no entry for ht-ma6i.6, ht-ma6i.7 or ht-ma6i.20 to ht-ma6i.23, although the policy text itself was updated, including the new Hermes limit; add entries so the log stays a true index.
- The ledger's `scripts/test-claude-mod` run-failure still reads "gap — open" although the archived cause is fixed at the head and predates the epic; close the record at finish.
