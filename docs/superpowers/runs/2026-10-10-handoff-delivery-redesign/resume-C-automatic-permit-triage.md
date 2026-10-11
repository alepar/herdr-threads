# C automatic-permit causal triage

Read-only diagnosis; no source/Git mutations, Cargo, test execution, processes, Herdr calls or delegation. The only write is this report. Systematic-debugging skill applied; trust-policy A2 and cooperative provenance constraints retained.

## Actual source and failures

Inspected candidate `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/upstream-composition-478ec4a9`, clean HEAD `b046c123be452957d3f7f10d8fbdc60b18a4533e`, execution tree `ecc9977a535a932b5514ab8820650d1c832691a5`. Actual evidence: `/private/tmp/ht-C-composed-b046c123/{execution.json,nextest-run.log}`; leak scope `28b15c51-3941-483f-83b0-2ef768028f03`. Parent recorded 377/385 focused PASS, eight FAIL, six mandatory PASS, scoped leaks zero. This report covers exactly three failures, all `CallerUnverified: permit expired`:

| Leaf under `cli::topology_handoff::tests::live::automatic_live::` | Actual failure | Duration |
| --- | --- | --- |
| `modern_automatic_actual_admission_boundary_completes_and_next_metadata_size_has_zero_effects` | Public completion assertion, line 7624 | 7.974 s |
| `modern_automatic_maximum_escaped_body_completes_and_recovers_exactly` | Public completion assertion, line 7035 | 4.003 s |
| `historical_automatic_recovery_over_limit_replay_refuses` | Shared fixture Attach unwrap, line 6399 | 2.329 s |

The modern leaves ran first with the unchanged automatic-envelope ALLSLOTS override (`.config/nextest.toml:158–165`). Their failure cannot be waived as those tests overlapping. The historical failure occurs BEFORE installing the oversized recovery row or invoking replay; it is not evidence that the intended replay refusal is wrong. Historical write/read/below-limit siblings passed in this run.

## Proven mechanism versus remaining uncertainty

`MAX_PERMIT_MILLIS` is 250 (`src/protocol/authority.rs:129`). `consume_cooperative` returns the exact error only when decision time precedes issuance or exceeds issuance by more than 250 ms (`:294`). Here `AutoPeer::call` mints and consumes each permit through the SAME `self.store` (`tests/cli/topology_handoff.rs:6189–6200`, `:6261–6270`), whose immutable `StoreContext` clock is one `SystemClock` (`:5821–5828`). `SystemClock::monotonic_now` measures elapsed time from its fixed `Instant` (`src/app.rs:43–70`). There is no backward clock or separately created permit clock on these paths. Thus the actual failure establishes **mint-to-DecisionInstant age >250 ms**. Whole test duration and whole mutation duration are not that age.

The fixture separately creates `base.clock` at line 1114 and the store clock at 5824. That mismatch affects absolute call-budget origins, but cannot explain this specific permit error: both permit timestamps use the store clock. Sharing a real clock would make budgets coherent but would not remove elapsed serialization time or itself cure these failures.

Canonical flow after issuance:

1. Parent runtime calls `automatic::mutate` (`src/store/topology_handoff/runtime.rs:227`). It performs request validation, namespace validation, request cloning and complete payload hashing before opening the accountable transaction (`automatic.rs:881–897`).
2. Transaction `before_replay` runs `current()` (`:907–910`), including full original decoding, lost-field check, typed validation, exact serialized identity equality, attachment validation and canonical key/state reads (`:108 onward`).
3. `StoreContext::execute_budgeted_decision_with_constraints` samples `DecisionInstant` AFTER its validation closure (`src/store/connection.rs:358–375`). `control::decide_accountable` then checks handoff requirements and calls `seats::decide_cooperative`; that function passes the saved decision sample to `consume_cooperative` (`src/store/control.rs:2408`, `src/store/seats.rs:4194`). Work after that sample does not contribute to the checked age.
4. Child runtime similarly validates namespace before child mutation; `automatic::mutate` derives/clones/validates/hashes, and `validate_phase` traverses canonical parent/child state before decision (`src/store/topology_handoff/runtime.rs:300`, `src/store/handoff/automatic.rs:319–360`). Send follows the ordinary store mutation route with automatic requirement retained; preparation happens before minting.

The production service follows this same mint-then-runtime-validation pattern (`src/service/dispatch.rs:255–284,341–348`) with fresh canonical authority and shared injected clock. This is not exclusively an AutoPeer-only performance hazard. Production fair-writer admission reduces contention but does not remove validation CPU between mint and decision. No test starts another writer/daemon in these AutoPeer calls.

**Not yet measured in C:** which parent/child action fails in either modern leaf; exact age; split among runtime prefix, writer acquisition and canonical validation. The historical action is known: Attach. Claiming a particular serialization function or new D trigger caused the threshold crossing would exceed the evidence.

## B/Main comparison and prior causal evidence

Compared immutable B `cdbf3c37e873a2cb571c098450602a53f81a6185` to C. Exact unchanged blobs:

| Path | Blob |
| --- | --- |
| `tests/cli/topology_handoff.rs` | `9ac63c44ea78a00718e4a65d10331cb2f0c10f3e` |
| `src/protocol/authority.rs` | `983ec322bfa7ea6ac32737be6fb0a83e314a7ddc` |
| `src/store/topology_handoff/automatic.rs` | `547944293334185bc156c4bfc4bba5c3e9c41e9b` |
| `src/store/topology_handoff/runtime.rs` | `990893011a2cc45b9a627c5aaf0fbee0e72f1fc8` |
| `src/store/handoff/automatic.rs` | `5e3dc7424e6f4ed08f4e6c7e1ec2707883163687` |
| `src/store/connection.rs` | `d5d531435f0e0147423d8145989958372006a811` |
| `.config/nextest.toml` | `c7a59d8c9c7802bcbc3d6dd7422f28044d2be0a4` |

`schema.rs` adds D's 40/41 schema and checks, but its accountable transaction helper is unchanged. Schema initialization runs at store construction before permits. `dispatch.rs` changes presentation state/routes, not these automatic functions. Main's permit issuance change selects pending-first invitations only for `AcceptCurrent`; these bootstrap permits do not use that obligation. C-only commits after 601761fa add tests in two other files. A clean composition is not a timing proof, but no new automatic permit semantic change was identified.

Read original `task-13-report.md` timing incident/final package, `task-13-review.md` R13-3, `task-13-rereview-1.md`, and `task-13-serial-evidence.md`. B previously failed maximum CHILD BEGIN and boundary CHILD SEND with this error. Solo child store-entry-to-decision measurements were 180.5/191.5/192.2 ms, excluding runtime prefix. Send 379/554 ms numbers were TOTAL mutation duration, not permit ages. B's grouped 32/32 PASS (17.899 s; modern leaves 8.515/5.508 s) established one successful run; it did not remove the documented headroom limit. Final historical recovery selection was 7/7 PASS; serial B promotion gates ran clippy/fmt/neutral but no tests. The prior independent review explicitly retained this as a minor measured-performance concern. Actual C now demonstrates that serialization scheduling alone is insufficient for stable acceptance evidence.

## Smallest parent experiment and controls

Parent owns the only Cargo lane. First make a temporary diagnostics-only checkpoint, preserving inputs, budgets, TTL, guards and scheduling. At `consume_cooperative`'s failure branch, record only operation ID, observed_at, decision now and age (no bodies). At AutoPeer mutation return, print parent/child role and elapsed issuance/mutation durations only on error. A narrow monotonic phase capture at parent/child runtime entry and before the decision sample separates prefix/validation time if needed; do not relabel total mutation duration as permit age. Run exactly the three failed leaves plus historical below-limit and same-geometry modern recovery controls. Modern leaves retain ALLSLOTS. A passing rerun alone does not resolve the failure.

Expected discriminator: positive age >250 for the failed role, with normal canonical context, and no rollback-visible new operation/attachment for that failed action. A negative age would contradict the traced clock identity and requires stopping this hypothesis. If time is mostly before runtime entry, investigate fixture work; if inside canonical decoding, the documented production performance concern is confirmed at that phase. Record exact source/tree, enumeration, output and leak scope.

Then, only if authorizing a fixture-focused experiment, use ONE injected controllable clock shared by AutoPeer's call budgets and store, held steady for geometry/recovery assertions. Existing working patterns are `tests/service/cooperative.rs:40–90` and `tests/store/cooperative_checkin.rs:18–78`; do not merely swap the store clock while leaving budget origins different. Hold all accepted body/metadata/recovery bytes and original assertions unchanged. Expected: both full completion/retry positives pass and the historical case reaches its intended `StoreCorrupt` replay refusal.

Required nonvacuous negative: using the same real issuance and deciding store path, advance that shared clock by 251 ms between mint and mutation for a fresh Attach/child action; assert exact `CallerUnverified/permit expired` and zero canonical effects. At +250 ms the unchanged guard is still valid, subject to otherwise valid state. Existing `protocol::authority::tests::cooperative_permit_is_payload_bound_expires_and_single_use`, `store::facade::tests::public_facade_rejects_expired_permit`, and the store writer-wait expiry control are additional guard checks, not substitutes for exercising this automatic path.

No production source fix is established by this report. A deterministic fixture clock can isolate geometry from wall scheduling, but cannot be presented as fixing production headroom. If optimizing production after measurement, the prior review's bounded direction is reuse of already validated canonical bytes within ONE transaction while retaining strict lost-field decoding, exact serialized identity/path spellings, fresh A2 mapping and all historical controls. That requires its own causal regression and review. Do not increase TTL/budgets/timeouts, retry away expiry, replace serialized equality with PathBuf-derived Eq, shrink geometry, or admit cached authority.
