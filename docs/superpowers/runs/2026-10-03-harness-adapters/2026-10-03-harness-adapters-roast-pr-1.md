---
super-roast verdict: Should-fix (12 confirmed)
mode: pr        iteration: 1 of 3
profile (assumed): Production local same-user cooperative CLI/SQLite daemon. Seats and receipts are canonical and TRUST-POLICY.md is normative; diagnostic and cache state is advisory. Local operational visibility has limited blast radius, while interface behavior, compatibility and evidence correctness remain material.
inputs: harness-adapters@b77d839600358c0feef67a5846b0f06d2c72ff2e vs main@aba0fc91dc3b3979d380483e30e2fc4c1916bd46; existing ht-3bi original round1
coverage: scouts 13/13 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, data-migrations, deploy-safety, api-contract, observability, testing, dependency, hygiene-docs) · raw 16 → deduped 16 → panel 13 · spot 3 · promoted 0 · judge completion 100% · remainder-capped: 0
independence: same-family (Codex) — seat-differentiated panel · rung: manual fan-out
seat-agreement: panels 13 · rr 1.00 · rg 0.92 · fg 0.92 · unanimous 0.92 · ground-loo 0.92 (n=13) · reproduce 12/1/0 · refute 12/1/0 · ground 13/0/0

## Confirmed findings
- [Should-fix] integrations/hermes/__init__.py:493 — A callback paused before child-lock acquisition can launch its Rust child after unload and after a successor bridge lifetime has been reserved, allowing predecessor and successor children to overlap.
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: Reproduce, refute and ground identify the same permitted pause after the valid snapshot and before child_lock. integrations/hermes/__init__.py:419-438 and :491-494 admit a later child launch without a lifetime recheck; close at :349-352 and successor reservation at :282-297 can finish first. The closed check at :497 only discards output after Rust work. Existing :457/:640 bridge tests cover a child already holding the lock. This defeats predecessor-child exclusion, without establishing seat corruption.
  fix-shape hint: Fence child launch against close and reservation transfer, and cover the callback paused before lock acquisition.

- [Should-fix] src/daemon/harness_evidence.rs:631 — Concurrent resumed-session suppression can finish before an extracted held lifecycle is persisted, yet the v2 recorder still credits that lifecycle using its earlier unsuppressed snapshot.
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: All three seats trace a valid held Codex lifecycle extracted with suppressed=false at src/daemon/harness_evidence.rs:626-631. A resumed request can mark suppression at :574-578 and finish its record before the extracted hold reaches store_one at :633. src/store/harness_evidence.rs:487-489 then credits the stale eligible lifecycle. The shared recorder and independent handler workers at daemon/control.rs:295-306 and daemon/transport.rs:446-466 permit this order. Recording-design.md requires sticky suppression with no later lifecycle credit; store serialization does not serialize the eligibility decision.
  fix-shape hint: Order suppression and lifecycle publication together so a completed suppression cannot be followed by stale lifecycle credit.

- [Should-fix] src/store/schema.rs:658 — The v25 schema verifier accepts changes to case-sensitive SQL literals and character classes, allowing incompatible evidence-table constraints to pass the reopen audit.
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: All three seats show src/store/schema.rs:658-664 folding quoted SQL literals and GLOB classes while comparing installed v25 DDL. An empty table changed from object to OBJECT or [a-z] to [A-Z] passes this comparison but rejects ordinary later evidence INSERTs at src/store/harness_evidence.rs:450-481. The ground seat fetched [SQLite JSON type documentation](https://www.sqlite.org/json1.html#jtype), [comparison documentation](https://www.sqlite.org/datatype3.html#collation) and [GLOB documentation](https://www.sqlite.org/lang_expr.html#like) establishing the semantic difference. This is a conditional reopen-audit defect; shipped migration SQL is correct and no real-data loss or failed normal upgrade is established.
  fix-shape hint: Preserve quoted predicate bytes in DDL comparison and cover case-changing CHECK literals and classes.

- [Should-fix] src/daemon/harness_states.rs:146 — Seventeen retained scoped diagnostics make the advertised harness.health_v2 endpoint reject its own otherwise valid report with InvalidRequest instead of returning bounded health data.
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: All three seats trace the legal 20-row diagnostic read cap at src/store/harness_evidence.rs:635,699-707 into uncapped limitations at src/daemon/harness_states.rs:142-146,325-340. src/protocol/results.rs:1750,1785 permits only sixteen limitations. Seventeen diagnostics, or sixteen plus a status limitation, make the valid no-argument endpoint return InvalidRequest through daemon/control.rs:257-264. health_lines at harness_states.rs:398-404 also loses its independently computed legacy broken rollup on that error.
  fix-shape hint: Bound the outgoing limitations after accounting for status lines, preserving the independent newest-broken rollup.

- [Should-fix] src/harness/adapter.rs:19 — The public adapter API represents callback admission twice, allowing an adapter to declare QualifiedCallback while core still performs installed observation and omits its callback input.
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: All three seats show independently overridable callback_admission and hook_admission_policy at src/harness/adapter.rs:19-29, with public QualifiedCallback at :241-245 and no registration coherence guard. cli/hook.rs:3086-3130 and :2472-2497 consult the boolean for observation and input routing. An enum-only QualifiedCallback adapter therefore receives installed observation and input=None, or refusal before admission. Current Hermes overrides the boolean coherently, so the demonstrated defect affects the public extension seam rather than current Hermes delivery.
  fix-shape hint: Make one admission policy authoritative and derive callback routing predicates from it.

- [Should-fix] src/harness/adapter.rs:675 — The shared launch-provider interface exposes Codex's shell-wrapper and --no-daemon details to every harness instead of keeping those details in the Codex policy.
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: All three seats distinguish the old compatibility request field from the new public LaunchPolicy exposure at src/harness/adapter.rs:650-698. Every provider accepts CodexShellProbe, whose mandatory operation is resolve_codex at harness/codex.rs:1854-1857, and a Codex-specific composition flag. Claude uses neutral environment access; Hermes ignores both. tests/harness/adapter_smoke.rs:460-465 supplies a dummy resolve_codex that panics. The native-policies design places shell function/alias inspection inside Codex policy behind a shared facility. No bad native argv or authority mutation is demonstrated.
  fix-shape hint: Separate neutral shell/environment facilities from Codex wrapper probing and keep its composition fact in its concrete policy or compatibility wrapper.

- [Nit] integrations/hermes/__init__.py:575 — The Hermes bridge discards the Rust hook's diagnostic output and suppresses child-launch, timeout and malformed-result failures, leaving repeated loss of turn context silent in the native runtime.
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: All three seats trace Rust Quiet/Unavailable diagnostics at src/cli/hook.rs:2864-2897,2980-2982 into stderr bytes drained and discarded at integrations/hermes/__init__.py:541,575-578. Empty/malformed output, missing executable, timeout and nonzero exit collapse to None through :409-413, without a bounded reason. Earlier daemon evidence cannot observe child-launch failures or replace the lost delivery signal. Fail-open behavior and the prohibition on raw stderr/exception bodies remain accepted.
  demoted: profile states a local same-user cooperative CLI/daemon; this failure loses bounded operational visibility without demonstrated loss or mutation of canonical seats/receipts, so it is a Nit here.
  fix-shape hint: Expose bounded allowlisted delivery-failure tokens separately from callback context and preserve privacy and fail-open behavior.

- [Nit] integrations/hermes/__init__.py:621 — The Hermes plugin can permanently disable all context delivery after registration or startup-identity failure without exposing any diagnostic that distinguishes that failure from a plugin that has not received a callback.
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: All three seats show registration exceptions swallowed at integrations/hermes/__init__.py:609-624 and startup capture failures retained only as private identity_unavailable at :200-207. snapshot at :241-248 and _callback at :442-444 then prevent any Rust invocation, so daemon evidence cannot distinguish failed activation from idle callbacks. The refute seat checks that the separate setup diagnostic does not report this loaded reader state. Hermes-design.md:99 explicitly requires honest reporting of a surviving hung read, which close at :256-269 does not expose.
  demoted: profile states a local same-user cooperative CLI/daemon; this failure loses bounded operational visibility without demonstrated loss or mutation of canonical seats/receipts, so it is a Nit here.
  fix-shape hint: Publish bounded activation/reader reason tokens, including surviving owned work, without changing identity qualification or accepted recovery limits.

- [Nit] src/cli/hook_evidence.rs:865 — The new rich evidence gate directory can retain expired files indefinitely because every pruning invocation rescans only the same initial directory entries without a progress cursor.
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: All three seats confirm conditional starvation in src/cli/hook_evidence.rs:411-430: each prune starts a new read_dir and scans only its first 256 entries without retained progress. run_v2 at :864-865 adds the independent persistent evidence-v2 directory and exact-build/domain/session fanout. Hourly updates can keep that prefix young while an expired suffix is never visited. The seats fetched [Rust read_dir documentation](https://doc.rust-lang.org/std/fs/fn.read_dir.html), which supplies no traversal-fairness guarantee. The helper already exists on base, but the new directory materially extends its retention surface; impact is small advisory files, not authority or a measured disk incident.
  fix-shape hint: Give bounded pruning eventual traversal progress across invocations or explicitly bound directory cardinality.

- [Should-fix] src/daemon/harness_states.rs:146 — The new health-v2 limitations expose expired 24-hour contract failures as current limitations for the full 30-day diagnostic retention period.
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: All three seats trace retained two-day-old diagnostics from the 30-day store read at src/store/harness_evidence.rs:637,699-707 into untimestamped current limitations at src/daemon/harness_states.rs:142-146. Separate :267-280 and :384-386 predicates correctly enforce the 24-hour broken/legacy Health window but do not filter this vector. Doctor prints the new limitations at cli/doctor.rs:1472-1497. TRUST-POLICY.md:381-388 explicitly distinguishes retention from the 24-hour Health failure window.
  fix-shape hint: Filter current limitations by the Health window or expose retained history with explicit timestamps and historical labeling.

- [Should-fix] integrations/hermes/test_bridge.py:338 — The asynchronous bridge rejection and refresh tests use fixed sleeps instead of confirming reader completion, so scheduler timing can both fail correct code and let rejection regressions pass while the reader is merely pending.
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: All three seats demonstrate source-level false-pass and false-failure schedules. integrations/hermes/test_bridge.py:242-245 only wakes refresh; :335-340 sleeps thirty milliseconds without awaiting the configured result. A previous usable snapshot can remain valid before read entry, while pending can mask erroneous completed acceptance. Startup tests at :512-520 and :674-680 can pass on initial pending/identity=None; entered at :165 precedes read_error publication checked at :397. The existing :522-539 helper waits for completion. No measured flake rate or production rejection bug is claimed.
  fix-shape hint: Await a bounded acknowledgment of the specific read and its completed quality before asserting refusal.

- [Should-fix] integrations/hermes/__init__.py:235 — There is no behavior regression ensuring that a completed slow official timeout read remains stale based on read-entry time rather than becoming fresh at completion.
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: All three seats confirm present production correctly records read-entry time at integrations/hermes/__init__.py:224-239 and rejects age greater than five seconds at :245-248. test_bridge.py:343-345 overwrites observed_at after a fast read and cannot distinguish entry from completion publication. Blocked-read tests either unload before completion or complete without advancing age. A successful still-valid read from simulated time100 to106 would be rejected with entry time but accepted if :235 changed to finished. Hermes-design.md:99 explicitly requires entry-based age; this is regression coverage, not a present stale-delivery defect.
  fix-shape hint: Use controlled clock/events to complete a still-valid read after more than five simulated seconds and assert no child delivery.

## Not verified (beyond panel cap)

- none

## Not verified (dedupe failed or judge lost)

- none

## Beyond remainder cap (count only)

- none

## Rejected (with reason)

- [FYI] src/store/schema.rs:501 — Starting this build permanently upgrades an existing schema-24 instance to schema 25, so rolling the executable back to the base release leaves that instance unusable without restoring an earlier database, but the changed operator documentation supplies no database rollback or pre-upgrade backup procedure.
  verdict: rejected (reproduce REJECT / refute REJECT / ground CONFIRM).
  reason: Reproduce and refute establish the real downgrade refusal but identify no requirement for reverse migration or a pre-upgrade rollback procedure. Root Storage migration and its accepted main-reconciliation amendment expressly require forward v25 migration and old-schema reopening with the new implementation. The ground seat confirms the same operational boundary and concedes the forward-only documentation convention is inherited; its concrete evidence does not leave an unanswered contract dispute. The default rejection stands.
  pre-existing on base: src/store/schema.rs:481-496 and docs/install.md:265-275 already automatically migrate older stores forward and document upgrade/recovery without a database downgrade procedure. This does not establish real-data loss or an unintended irreversible-migration risk in the shipped upgrade. No fix round is driven by this FYI.

## Unverified nits (spot-checked)

- [Nit] src/cli/hook_evidence.rs:710 — The new v2 evidence gate has no saturation regression for its distinct count-eviction and serialized-byte-trimming paths.
  evidence: Spot refute confirms no v2 count-eviction or write-time byte-trimming/reload/resend regression at src/cli/hook_evidence.rs:710-740. tests/cli/hook_evidence.rs:790-803 covers legacy GateState; :1323-1327 checks small output and :1500-1510 rejects an already oversized file. No production saturation failure is established.
  verification limit: one refute-seat spot check; not panel-strength confirmation.

- [Nit] docs/research/harness-adapters-2026-10-03/run_manifest.json:15 — The committed research manifest exposes the operator's personal account name and absolute workstation checkout path in public research evidence.
  evidence: Spot refute confirms the literal account/checkout path at run_manifest.json:15. Relative artifact links already exist, and base docs/research/doctor-redesign-2026-10-03/run_manifest.json:14 uses a relative report directory. No secret or runtime failure is claimed.
  verification limit: one refute-seat spot check; not panel-strength confirmation.

- [Nit] src/cli/hook.rs:185 — The public hook-registration documentation excludes underscores from --event names even though this PR adds underscore-bearing native events and accepts them in the actual hook entrypoint.
  evidence: Spot refute confirms src/cli/hook.rs:183-188 and harness/evidence.rs:45-48 admit underscores, while docs/compatibility/harnesses.md:118 still describes letters and digits only. Hermes event declarations at harness/hermes.rs:42-62 make the mismatch concrete. Bundled bridge invocation does not depend on --event, limiting impact.
  verification limit: one refute-seat spot check; not panel-strength confirmation.

## Escalations (need human)

- none

## Review limitations

- All sixteen engine packets are accounted for: twelve confirmed, one rejected and three unverified nits. No route is overruled. Coverage and agreement lines are copied verbatim; original iteration1 is not converged.
- Distinct triage, thirteen scouts, dedupe and panel seats through panel7 inherited surrounding parent conversation, including earlier review/severity context. Finding JSON was mechanically blinded, but those actors were not strictly context-blind. Panels8 through12, spots13 through15 and this reporter used fork_turns=none. context-isolation-observation.json and dispatch-index.json preserve that provenance. No cross-model independence is claimed; numerical completion does not remove this limitation.
- The complete 180024-byte reporter packet was read in bounded contiguous chunks. Initially truncated supporting-file outputs were reread in bounded ranges or projected from complete parsed JSON; no required read was denied. This reporter reasons over the admitted seat evidence and normative contracts, without deriving new source findings or executing tests, builds, native code, imports of native modules, probes or repairs. Seats likewise reported source-only review, not native reproduction.
- Frozen branch HEAD b77d839600358c0feef67a5846b0f06d2c72ff2e and tree632a585d93f0344ae1369f7bdd310422550d5f11 were verified with clean git status. The report uses the supplied aba0fc91 base, not a claim of latest-main absorption; the live main ref observed during reporting is f922988a61db2edf00893b91deaab01f899557e8.
- No confirmed packet establishes exploitable privilege escalation, secrets granting otherwise inaccessible data, real-data loss, or a failed irreversible upgrade. Current built-ins keep coherent admission choices and working launch composition; their public interface defects show extension risk. Health, advisory evidence, ownership and regression-coverage failures remain Should-fix, without claiming the overall artifact core goal has been satisfied or its full native delivery purpose independently disproved by these packets.
- The first whole-epic review separately remains NOT_READY with WE-F1 (Hermes structural classification), WE-F2 (absent-Hermes installation/Health reporting), WE-F3 (public setup/removal scope) and inherited ht-92q Must-fix. Those are separate gates from this roast's findings. ht-92q retains its separately mapped Phase5 repair route; ht-1c5 remains UNKNOWN. No gate, old counter or round is reset or waived.
- Actual native/model acceptance remains UNMET. Source, synthetic, Unsupported and diagnostic outputs do not replace original-launcher/plugin/API/context/top-level/child/reset/receipt acceptance. Startup-captured bridge-lifetime identity, periodically observed official timeout, uncancellable singleton reader and normal native-inspection effects remain the accepted limits; continuous freshness and dispatcher-snapshot proof are not newly demanded.
- Working Hermes support merged to actual main remains the goal, with full18 requirements and the cohesive interface plus one static registration criterion. Latest-main absorption, both overall reviews, actual main merge and coordinator-owned final full-suite/timing/leak gates remain outstanding. Historical main migrations1..24 plus forward25/six archival triggers, wire6/summary2/renderer2/promptv2/defaultbatch0 remain conserved requirements. Official091/093 protocol22 and exact-true process_hint capability are distinct from foreign modified091 identities; versionless metadata supplies no fabricated runtime, null hash or native receipt.
---
