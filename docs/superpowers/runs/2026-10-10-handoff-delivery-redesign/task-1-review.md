### Spec Compliance

- ✅ Spec compliant for Task 1's assigned deliverable at HEAD `282d5461efc8c49f98bb000353a8a8bba92f1a2d`: `src/store/effective.rs:1796` keeps canonical availability independent, preserves stored deadlines, bounds historical running-clock reconstruction by the durable cutover, and derives new/dormant pending deadlines from committed send time. `src/cli/mod.rs:2761`, `src/cli/mod.rs:2862`, and `src/client/service.rs:675` guard ordinary new sends and pending retries before submission/journaling.
- ✅ The authorized migration amendment is implemented without changing historical 0001: `migrations/0029_receipt_send_time.sql:4` backs up rows before rebuilding the original table names; `:153` restores receipt sequence high-water; `:161` creates/captures the marker. `src/store/schema.rs:589` wraps constraint replacement, FK validation, version change and marker capture in one immediate transaction and restores the original FK setting on success/error.
- ✅ The accepted conservative compatibility limits are recorded in `TRUST-POLICY.md:387`; human sender identity is not used as a recipient waiver. Lazy sends remain exempt at `src/cli/mod.rs:2758`, while terminal attribution fields remain unchanged in `src/store/effective.rs:1852`.
- ⚠️ Cannot verify from this task diff: bounded historical dormant physical backfill/indexed discovery (Task 5), shared fair due-driver behavior, sender warning routing, and later handoff consumers of the new preflight. Those remain explicitly separate integration obligations. The provisional migration29 must receive the requested fresh schema seam review after serial renumbering; this verdict does not approve that future composition.
- ⚠️ Direct lifecycle coverage for acceptance/rebind and the complete old/new public client-daemon matrix is not newly demonstrated by this diff. The projection uses frozen timestamps and stored deadline precedence, and existing focused suites were reported passing; the final cross-task sweep should retain the named lifecycle scenarios.

### Strengths

- `tests/store/effective.rs:982` exercises a genuinely populated v28 graph, compares old canonical indexes/triggers and retained row data, preserves sparse rowids/physical ordinals/autoincrement high-water, and checks inbound FK dependents. `tests/store/effective.rs:1067` verifies late failure rolls back constraints, version and marker before retry.
- `tests/store/effective.rs:921`, `:953`, `:1112`, and `:1123` distinguish old running clocks, old dormant clocks, pre-cutover retirement, and acknowledged absent deadlines, including post-cutover registration and restart.
- `tests/store/materialization.rs:846` checks an unavailable recipient becomes physically due while availability stays NULL, and bounds the worker loop while asserting stored errors are absent.
- `tests/cli/lazy_send.rs:445` and `:475` cover absent/refused discovery before new intent or retry effects; `src/client/service.rs:1839` and `:1869` exercise real socket discovery for service sends/replays rather than only mocking a semantic helper.

### Issues

#### Critical (Must Fix)

- None found.

#### Important (Should Fix)

- None found in the assigned task implementation.

#### Minor (Nice to Have)

- `task-1-report.md:37`, `:43`, and `:47`: measured same-path rebuilds (6m08s/4m15s) and clippy (70s) exceed the repository's one-minute budgets. These are real overruns, explicitly reported; the available evidence does not establish a feature-caused regression versus host contention. Preserve this concern for parent performance validation; do not describe the timing budgets as satisfied.
- `tests/cli/lazy_send.rs:505` and `src/client/service.rs:1905`: refusal tests compare parsed semantic/request values and pending state, rather than byte-for-byte files. The production guards return before journal mutation, so this is a coverage improvement, not an observed rewrite defect. Snapshot raw retained files if strengthening exact-byte regression coverage.

### Checks

- Required test-change review: ran `git diff --stat BASE..HEAD -- tests` and `git diff BASE..HEAD -- tests | head -n 400` in the task cwd. The latter was intentionally truncated at 400 lines; full test hunks were reviewed in the supplied diff package. No skipped tests, deleted test cases or unauthorized assertion weakening found. Deadline expectation changes reflect the requested semantics; schema fixtures add canonical FK prerequisites and exclude only the newly additive marker from old-root snapshots (`tests/store/schema.rs:536`, `:5699`, `:6259`).
- Diff read in portions. An initial combined tool result truncated the early diff output; recovered those portions from the supplied package. No separate full changed-file reread. Focused reads of `src/store/effective.rs:1825`, `src/client/service.rs:715`, and `src/cli/mod.rs:2640` completed mid-function hunks to judge physical fallback, completion/replay ordering and public wire routing.
- Named risk — projection/materialization divergence: checked unchanged `src/store/materialization.rs:583` and `:609`. Both send and later availability work use `effective_receipt`; UPSERT preserves stored deadline and only fills missing availability. New unavailable receipts therefore enter the existing due index without fabricated availability.
- Named risk — historical physical/terminal settlement boundary: checked unchanged physical fallback at `src/store/effective.rs:1876` and ACK persistence at `src/store/receipts.rs:453`. Historical physical rows remain retained for Task 5; new ACKs store the projected original deadline and attribution.
- Named risk — falsely successful ordinary status recovery: checked unchanged public dispatcher at `src/service/dispatch.rs:807`; actual `OperationStatus` returns Unauthorized for missing verified scope, matching the conservative retry limit and new public-dispatch regression.
- Named risk — bypassing service durable preflight: checked unchanged `src/client/service.rs:588`; query rejects keyed mutations. Typed send funnels through submit (`:715`), and completed replay checks precede the new capability guard (`:771`).
- Named risk — service/native publication mismatch and pre-advertisement migration: checked unchanged `src/store/service_send.rs:461` (shared message publication) and `src/store/connection.rs:167` (writer initialization completes schema initialization). New semantics are implemented in canonical projection rather than depending on modern wire fields.
- Named risk — A6 extension accidentally replaced: checked unchanged `tests/store/receipts.rs:3032`, `:3052`, and `:3069`; explicit extension and frozen-later precedence remain separate from the original-deadline change.
- No Cargo/test commands run by this reviewer. Reviewed the implementer's reported 317 focused passes, clean format and warning-free clippy evidence (`task-1-report.md:47`, `:76`); these are attributed evidence, not independently rerun results.

### Assessment

**Task quality:** Approved

**Reasoning:** The implementation centralizes send-time semantics in the canonical projection, preserves historical running and terminal evidence, and guards modern ordinary effects before durable submission. Migration preservation/rollback and compatibility refusal have focused regressions; future migration composition, remaining cross-task behavior and measured timing overruns remain explicit integration concerns.

