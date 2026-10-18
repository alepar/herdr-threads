# Integration sweep: peer wire compat hardening (ht-j87z.12)

Tree: integration tip `2c9b5a718` plus the unmerged branch of ht-j87z.5 (the `opaque_relay` tests). Logs are under `evidence/ht-j87z.12/`.

## Goal checks

| Check | Result |
|---|---|
| `cargo test --locked --all-features --test combined opaque_relay` | 2 passed (`relay_forwards_htjrnl99_to_third_node_byte_exactly`, `capless_third_node_is_withheld_and_stays_healthy`): `opaque-relay.log` |
| `... --test combined peer_wire` | 4 passed: `goal-cargo.log` |
| `... --test combined forward_compat` | 5 passed (2.1.1 shadow decode): `goal-cargo.log` |
| `... --test combined journal_port_contract` | 68 passed: `goal-cargo.log` |
| `scripts/check-wire-compat` | ok, 0 violations, 0 stale, 255 fields, 18 variants, 268 baseline entries: `check-wire-compat.log` |
| `python3 -m unittest discover -s scripts/tests -p test_check_wire_compat.py` | OK: `unittest.log` |
| `scripts/freeze-peer-wire 2.1.1 --at v2.1.1 --check` | byte-identical to `tests/fixtures/peer_wire/2.1.1`: `freeze-211.log` |
| `scripts/freeze-peer-wire 2.0.4 --at v2.0.4 --check` | byte-identical to `tests/fixtures/peer_wire/2.0.4`: `freeze-204.log` |

The plan's step 1 names `--test opaque_relay`; no such target exists (`autotests = false`). The suite is the `opaque_relay` module of `tests/combined.rs`, so the filter form above is the correct command.

## Wiring sweep

| Item | Answer |
|---|---|
| `OPAQUE_CAP` pushed into the hello caps in the production path | yes: `src/sync/coordinator.rs:694-695` (`advertise_opaque`, default true at `:629`); `src/app.rs:2127` calls `SyncCoordinator::start` and sets no option; test `hello_advertises_opaque_cap` (`coordinator.rs:5190`) |
| `LinkStatus.withheld` reaches `LinkRow.withheld` and `sync status` text | yes: `coordinator.rs:2341` -> `src/service/sync_admin.rs:247` -> `src/protocol/sync_admin.rs:172` -> `src/protocol/output_compact.rs:1218` |
| `LogRow.blocked_on` filled | yes: `src/store/sync_status.rs:105,142,171`, text at `output_compact.rs:1196` |
| `RepairRow.cannot_reissue` filled | yes: `src/store/incarnations.rs:1503,1673-1688` -> `src/service/sync_admin.rs:270`; the worker logs it at `src/sync/incarnations/worker.rs:1451` |
| `journal_opaque` and `journal_opaque_promoting` in the kick registry | yes: `src/service/kicks.rs:240-241` |
| `gate_batch` the only place serve paths consult `opaque_cap()` | yes: only `src/sync/coordinator/serve.rs:251` and `:484`, both as the argument of `gate_batch` |
| nothing but the Chain-row reader selects from `journal_opaque` | yes for serve, fork and branch code. In `src/store/journal.rs` the selects are `entries_from`/`entry_at_position`/`branch_entry` (reader), `insert_opaque`, `promote_opaque` and the `verify_invariants` count (`:249`). Outside it only `src/store/incarnations.rs:250,1594` (re-issue candidate arm and `cannot_reissue` count), which the design lists among the chain-defining union readers (§4, "incarnations/resync readers"). Not a gap. |
| `IngestOutcome::Oversize` handled in every match | yes: the exhaustive matches are `src/sync/resync.rs:536-544` and `src/sync/coordinator/ingest.rs:405+` (`:425`); the tree is clean under clippy `-D warnings` |
| TRUST-POLICY accepted-limit entry | yes: `TRUST-POLICY.md:1993` "Opaque relay (ht-j87z)" |
| docs: operations, release, changelog | yes: `docs/operations.md:307-316`, `docs/release.md:134` (step 8b), `CHANGELOG.md:5` |
| schema delta registered, release-renumber note | yes: `src/store/schema.rs:74`, `docs/release.md:134` |
| no production caller passes `advertise_opaque: false` | yes: `false` appears only in tests (`tests/sync/opaque_relay.rs:113`, `tests/sync/opaque_wire.rs:94`, `coordinator.rs:5206,5441`); `SyncNodeOptions` is `test_support` only |

## Gaps and actions

None found; no inline fix and no blocker bead. The one discrepancy is the plan's wrong `--test opaque_relay` command (above), a plan typo and not a product gap.

## Full suite

- `nice cargo clippy --locked --all-targets --all-features -- -D warnings`: pass. `scripts/check-harness-neutral`: ok, 0 violations. `nice scripts/check-default-features`: rc 0. `cargo fmt --check`: rc 0.
- `nice cargo nextest run --locked --all-targets --all-features` (`HT_LEAK_RUN_ID` set): 7805 tests run, 7805 passed (1 slow, 6 leaky), 48 skipped. Nextest run time 404 s; script wall time 570 s including the cold test-profile compile. Over the 5-minute budget (recorded, not a blocker): `package manifest::failpoints_are_compiled_out_of_release_builds` alone took 312 s (it builds the release profile). The 6 `LEAK` lines are nextest handle-leak flags on tests this epic did not touch (`suite-summary.log`).
- `scripts/check-no-leaked-processes --run-id`: no leaked test temp files, no leaked test processes, rc 0.
