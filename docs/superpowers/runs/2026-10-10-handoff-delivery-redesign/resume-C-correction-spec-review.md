# C composition-failure corrections Spec review

Verdict: **scoped source Spec PASS** for the three correction/diagnostic commits through `95e7e3bf`. No spec revision request found. Capability/schema changes address independently explained stale expectations. Hook/capacity changes add bounded failure evidence; they do **not** resolve or establish the cause of the original runtime failures. Parent successor verification remains pending; no runtime-success or combined-readiness claim is made.

## Frozen scope and provenance

Reviewed immutable composition `95e7e3bf725c5bc771cbb55be7513e8e16fae563`, tree `682a9afda7568f3a127850367cba0befab76fd26`, with `git show` and immutable diffs. Observed HEAD is that commit and checkout diff is empty. Read full `resume-C-capability-schema-triage.md`, `resume-C-hook-startup-triage.md`, and `resume-C-capacity-triage.md`, plus the actual original C385 execution manifest.

Exact reviewed commits are `f970d6faabd0861521456e4422055bed35275ab6` (capability/schema), `9ff4913a4bb5a10302c57c97fa6a526ee9f87ddd` (Hook startup diagnostic), and `95e7e3bf725c5bc771cbb55be7513e8e16fae563` (capacity diagnostic). Each commit's `Outside-adapter:` trailer exactly names its changed test paths. Production model-visible text added by these three commits is 0 bytes. Independent object comparisons found no `src`, migrations, package/lock or nextest-config changes from `b046c123`; production control/capability and migration40/41 bytes are unchanged.

Reviewed test-file SHA256 values:

| Path | SHA256 |
| --- | --- |
| tests/protocol/capabilities.rs | `65f905780543624d9c8c9e09dca7d1f0a68f97c7a7a53c523f15eb2c3eeffcd8` |
| tests/store/effective.rs | `0a1fac9225eb70954e96563120a85bc11027ebbd919d83eccf8c788fc6edf2ca` |
| tests/hook_entrypoint/agy.rs | `ed404b59c821f22bb65665456821d6da65062d36d3ae33e3843c3113e06dedcd` |
| tests/integration/mod_delivery.rs | `bd1b6afb7bf8668bc73c4901f1a4911e36cc776a0bb90d1984cc94a900ffcbab` |

An intervening TRUST-POLICY A7 prose reconciliation also exists in the larger `b046c123..95e7e3bf` ancestry. Read its exact delta: it removes the duplicate blanket affected-seat-only sentence and qualifies passive observers around the already-defined canonical origin wakes. It is outside these three test commits; these corrections introduce no new authority, provenance or policy weakening.

## Capability expectation correction

The entire capability delta is one literal `direct_threads.ensure_creation_policy_v1` in each exact ordered expected list (`tests/protocol/capabilities.rs:285,576`). No assertion, negative control, handler probe or capability gate was removed. Bare-daemon no-recorder checks retain unsupported harness evidence/health, nonexistent capability and inspected-recovery assertions (`597–600`), as well as the old-daemon controls immediately following.

This advertised capability was already defined/advertised in unchanged production (`src/protocol/capabilities.rs:31,86`), has an existing explicit Some(Lazy) Ensure handler probe (`tests/protocol/capabilities.rs:354–365`), and has the genuine guarded canonical modern producer/result verified in the earlier C public Ensure review. The correction therefore aligns exact expectations with implemented behavior; it neither adds premature advertisement nor hides a refused capability.

## Schema expectation correction preserves historical witnesses

- **Genuine precursor lineage remains:** `tests/store/effective.rs:1046–1085` still obtains an actual populated v28 physical file, executes immutable migrations29..34 in transactions with their actual stamps, inserts the historical timed invitation, physically reopens, verifies version34/FK integrity and exercises both FK modes. It does not initialize latest then rewind a version.
- **Existing catalog filter/39 allowance remains exact:** old catalog captures type/name/table/rootpage/SQL (`1108–1112`). Independently byte-compared the entire `normalize_capacity` closure (`1133–1161`) against `b046c123`: unchanged. Its historical four-table39 rebuilt-table/implicit-index page allowance is not broadened; old objects outside that prior allowance keep exact page witnesses. Existing fixed additive35..38 exclusions are unchanged. No whole-catalog rootpage normalization, wildcard new-object exclusion or selected-column replacement was introduced.
- **New objects come from independent immutable declarations:** the expected scratch connection starts from captured historical table DDL, with the already-independent37 warning adjustment and separately required38 origin table; it never reads current initialized catalog SQL as an expected oracle (`1162–1208`). It executes exact immutable40 and41 `include_str!` SQL, derives only newly declared names, asserts exactly118 objects split6 tables/16 indexes/96 triggers, and compares their complete type/name/table/SQL definitions (`1209–1263`). New rootpages alone are normalized by that exact derived name set (`1234–1248`), so unknown actual additions/removals or definition changes fail the full catalog equality. The original invitations SQL is changed only to its independently executed40 declaration.
- **Rows retain every historical value:** full `SELECT rowid,*` snapshots remain across all eight historical tables (`1094–1107`). The expected suffix extension adds exactly INTEGER0 to invitation rows (`1264–1270`), retaining rowids, timing, policy, attribution and all old values. No historical column is omitted; all other table shapes remain equal.
- **Rollback/reopen assertions strengthen:** the existing migration35 temp-view obstruction, failure, autocommit, version34, FK-state and main-marker absence checks remain (`1272–1297`). New full-row rollback comparisons (`1298–1303`) strengthen the old catalog equality. Physical reopen then performs real initialize to current version and exact normalized catalog/full-row comparisons (`1305–1334`). Marker high-water/nonrecapture and retained cutover assertions remain. Second physical reopen and initialize now additionally compare every retained row and the **complete unfiltered** catalog including all actual newly allocated rootpages (`1340–1363`), followed by original expected-catalog/FK assertions. The first comparison permits fresh allocation of only declared new objects; the second proves their actual pages remain stable across reopen.

This is a precise composed fixture expectation extension, not a migration repair or an old-DDL/history waiver. Runtime must still demonstrate the asserted118 definitions/counts and both FK paths in actual linked SQLite.

## Hook startup diagnostic preserves failure semantics

`tests/hook_entrypoint/agy.rs:820–868` adds caller location and retains the same installed SessionStart invocation, zero-exit/nonempty-output assertions, one observer snapshot at the same point, actual CheckedIn selection and startup Fault::None. Borrowing that vector and cloning its real returned context preserves selection; absent CheckedIn still panics. It does not synthesize a claim, retry startup, synchronize differently, add a sleep or change budgets/fault placement. Later DropConfirm/LoseConfirmReply recovery bodies remain unchanged. `iteration=unrecorded` is honest rather than inventing which loop iteration failed.

`startup_observation` (`1095–1215`) runs only on missing startup CheckedIn. It redacts the private fixture root before escaping stdout/stderr, caps each encoded snippet at1536 bytes with explicit omitted-redacted-byte count, records at most12 first/last indexed command/result tags and error codes, and never Debug-dumps claims, commands, pages, tokens, payloads, environment, full argv or transcript/database content. Stderr length is honestly labeled captured UTF-8 length rather than purported original bytes. The whole diagnostic reserves128 bytes for the assertion prefix inside6KiB and truncates at a valid UTF-8 boundary while retaining final stage/exchange metadata. The captured output is only isolated SessionStart before peer/message fixtures exist; no new delivery-body logging path is added. No causal verdict is encoded in these observations.

## Capacity transport diagnostic preserves the original oracle/order

The relay records actual downstream write success or ErrorKind **after** the same `activation_write` call and retains the same `written?` error propagation (`tests/integration/mod_delivery.rs:1769–1782`). At most65 per-admitted-Pull entries are stored (`1415–1418,1772`); a release permit is never presented as proof of physical receipt.

The existing terminal response is still held through the existing paced-work barrier; only its ordinal/index/Instant are captured. Barrier timing is diagnostic only. Initial64-observation bound, exactly256 offered IDs/no duplicate or ID257, canonical job completion and retained257th body/carry assertions are unchanged. Terminal release still precedes the same `watch.wait_item(sent[255])` (`2834–2868`); no fixture ordering correction or timeout increase was slipped into a diagnostic patch.

New failure record contains terminal shape/target membership, hold/barrier elapsed time, child try_wait, bounded flow counters/write result, compact last statuses/IDs and a256-byte stderr prefix (`2821–2877`). It does not dump message bodies, environments, argv, full requests or databases. IDs/token/epochs are canonical synthetic fixture metadata, not independent authorization. The separately retained panic record is capped4096 bytes at a UTF-8 boundary with a truncation marker (`2878–2887`), then the original panic payload is resumed (`2888`). Any passing behavior still reaches all original receipt-pending/no-mod-ACK checks, explicit Attention gating, strict resumed more=false/empty-output checks and byte-equal normal carry tuple (`2890–2945`). The diagnostic distinguishes observed failure stages; it does not claim transport-hold causality before actual reproduction.

## Original actual C385 failure history remains authoritative

Independently parsed `/private/tmp/ht-C-composed-b046c123/execution.json`. It records immutable `b046c123`/`ecc9977a...`, source-before equal to source-final, exact selected/executed385 focused cases, **377 PASS / 8 FAIL**, with no missing/extra/duplicate names; overall run_failed/exit1. Six separate mandatory cases are **6/6 PASS**, zero ignored. Focused, mandatory and final scoped leak exits are all0 under UUID `28b15c51-3941-483f-83b0-2ef768028f03`.

All eight actual failures remain in history:

1. `cli::topology_handoff::tests::live::automatic_live::modern_automatic_actual_admission_boundary_completes_and_next_metadata_size_has_zero_effects`
2. `cli::topology_handoff::tests::live::automatic_live::modern_automatic_maximum_escaped_body_completes_and_recovers_exactly`
3. `cli::topology_handoff::tests::live::automatic_live::historical_automatic_recovery_over_limit_replay_refuses`
4. `agy::activation::activation_v2_registered_agy_lost_completion_preserves_exact_recovery`
5. `protocol::capabilities::tests::bare_daemon_advertises_legacy_capabilities_without_v2_recorder`
6. `protocol::capabilities::tests::capability_constants_are_stable`
7. `mod_delivery::activation_v2_watch_full_pending_capacity_parks_until_real_channel_input`
8. `store::effective::tests::dormant_upgrade_actual_v31_backfill32_reopens_and_rolls_back_without_recapture`

The two capability/schema expectation causes have narrow source corrections; the two diagnostic leaves remain causally unresolved. The three topology failures are outside this correction review. Historical earlier leaf/D passes cannot replace actual failing composed evidence. Parent owns fresh exact-successor enumeration/execution, diagnostics interpretation, source gates, leaks and matrix disposition. This reviewer performed no Cargo/runtime/Herdr command, source/Git mutation or delegation. Sole write is this designated report. **COMPLETE, QUIESCENT**.
