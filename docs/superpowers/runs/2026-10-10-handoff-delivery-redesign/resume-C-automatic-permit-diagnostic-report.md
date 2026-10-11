# Automatic permit diagnostic checkpoint

Status: QUIESCENT, diagnostic authored and committed, NOT compiled or executed. Parent owns Cargo and the next causal run. No production fix or acceptance claim.

- Helper: `.worktrees/handoff-delivery-redesign/.worktrees/finish-composed-permit-diagnostic`.
- Branch: `helper/handoff-composed-permit-diagnostic`.
- Base: `b046c123be452957d3f7f10d8fbdc60b18a4533e`.
- Commit: `e762aecf5115aa5c7904bbba506a8d0c3838b08f`.
- Tree: `92b014dc2ef00a65602647164814aa0e14aaac74`.
- Only committed path: `tests/cli/topology_handoff.rs`; truthful `Outside-adapter` trailer names it. Working tree clean after commit.

Parent explicitly approved the fixture-only plan before editing. The existing three C failures are the actual RED evidence. Read systematic-debugging, TDD plus its test-writing reference, verification-before-completion, and current trust-policy A2/automatic envelope requirements. No new test was added: this checkpoint observes the already failing actual chains.

`AutoPeer` now retains an injected `AutomaticPermitClock` which delegates every UTC/monotonic result to a live `SystemClock`. It neither freezes nor advances time. A callback installed through the existing `StoreContext::set_lane_fault` records the first store access while armed and always returns None. All production sources, test bodies, geometry, budgets, TTL, ALLSLOTS configuration, original grants and error/results remain unchanged.

Immediately after real store issuance, the fixture captures at most 2048 UTF-8 bytes of the existing fresh permit Debug. At this point `handoff_requirement` is None, so it contains metadata and no body-bearing request. The private `observed_at: MonoInstant(...)` is parsed from those actual bytes; missing/truncated/unrecognized Debug yields None rather than fabricated timing. Debug capture occurs before mutation and its real elapsed cost remains included in observed permit age; issue_end→mutate_start exposes that gap.

Observation is reset/armed only after issuance and directly before mutation. Under the currently inspected synchronous parent/child/Send accountable paths, `StoreContext::execute_budgeted_decision_with_constraints` performs the first UTC read followed immediately by the exact monotonic sample stored in DecisionInstant. Prefix and budget reads are monotonic-only. No other clock user/background task exists in AutoPeer. The observer records the first pair and total pair count, preserving later historical-presentation pairs without overwriting the first. This is explicitly a **source-dependent clock-pair observation**, not a production tracing API. Namespace/validation regions remain combined: the callback observes public writer entry, not individual internal validation functions. If the actual trace contradicts these assumptions, stop interpreting age until the seam is corrected.

On issuance/mutation error only, a UTF-8-safe bounded formatter emits one record no larger than 4095 bytes plus newline. Mutation records contain role, actual operation, call start, build start, issue start/end, mutation start, first store entry, first decision pair, pair count, outcome time, observed_at, signed decision-minus-observation age, error code, bounded detail and issued metadata. No body is printed, no SQL is added, and the original Result is returned unchanged. Injected lost replies retain their original position after the actual mutation result.

Parent should execute the three failed leaves plus controls under the unchanged nextest policy and retain the exact source/tree/output. For `permit expired`, expected records have a concrete role, one first decision pair, observed_at and signed age >250. Modern role is currently unknown. Historical failure should identify `auto_attach`. Negative age or absent observed timestamp is diagnostic failure, not support for the original hypothesis. A GREEN rerun alone does not resolve the timing defect. No deterministic-clock experiment or production optimization is included in this checkpoint.

Checks actually run: direct `rustfmt --edition 2024 tests/cli/topology_handoff.rs`, subsequent exact-file `rustfmt --check`, and `git diff HEAD^ HEAD --check`, all exit 0. These establish formatting/syntax parsing and whitespace only; no type-check, clippy, neutral/default gate or tests ran. No owned daemon, helper background process, Herdr action or subordinate agent was started. Normal Git staging/commit needed the approved CLI sandbox escalation because worktree Git metadata is read-only in the workspace sandbox; both escalations succeeded.
