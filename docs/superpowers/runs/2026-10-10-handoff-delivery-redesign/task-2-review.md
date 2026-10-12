### Spec Compliance

- ✅ Spec compliant. `src/protocol/commands.rs:667` and `src/cli/journal.rs:147` add optional, omitted-when-None confirmation without changing historical serialization. Explicit modes, validation, canonical refusal and conservative schema preparation match the task's approved activation boundary (`src/protocol/commands.rs:1093`, `src/store/control.rs:1527`, `migrations/0029_invitation_confirmation.sql:1`).
- ✅ Approved seams respected: capability is defined without advertisement; modern defaults, full lifecycle and additive query metadata remain Task 6/downstream work. Migration 29 remains provisional for serial composition.
- ⚠️ Final combined migration composition and lifecycle activation cannot be verified from this isolated diff; controller must check these during integration. `src/store/schema.rs:60` currently assigns this candidate version 29.

### Strengths

- `tests/protocol/capabilities.rs:1663` checks exact legacy request bytes and a fixed digest, distinct modern round trips, explicit timing and human/Lazy conflicts. `tests/cli/cooperative.rs:5169` independently covers retained semantic bytes and command construction.
- `src/store/control.rs:1527` refuses explicit policy before transaction work. `tests/store/control.rs:10212` checks both policies return Unsupported with unchanged SQLite total_changes and no invitation rows; these assertions establish actual refusal behavior rather than a structurally guaranteed property.
- `migrations/0029_invitation_confirmation.sql:1` preserves historical NULL semantics and frozen timing, requires a known modern origin, prevents origin/policy rewrites and provides a mode-filtered due index. `tests/store/schema.rs:6448` covers migration, repeat initialization, retained values, index membership and failed rewrites; the companion test checks missing-guard rejection.

### Issues

#### Critical (Must Fix)

- None found.

#### Important (Should Fix)

- None found.

#### Minor (Nice to Have)

- `tests/cli/cooperative.rs:5190`: capability coverage calls the helper directly. Add a public `run_cooperative` retry regression when activating modern producers to prove unsupported frozen intents leave journal bytes untouched and never submit Invite. Current ordering is correct by inspection; this is a coverage improvement, not a blocker.
- `task-2-report.md:67` and `task-2-report.md:94`: disclosed final compilation (4m53s) and clippy (6m26s) exceed the one-minute project budgets; initial RED output also contains the reported nice permission diagnostic. Final lint is clean, and reported compiler contention prevents attributing the timing to this source change. Controller should retain the budget observation and measure an uncontended incremental edit before asserting performance compliance.

### Assessment

**Task quality:** Approved

**Reasoning:** Additive representations preserve historical wire and intent semantics, and canonical mutation remains closed until lifecycle support exists. Tests exercise meaningful compatibility and schema behavior without weakening existing assertions.

### Checks

- Confirmed checkout HEAD with `git rev-parse HEAD`: `30c1f07c387ce0de6fcb562be75e8d9e6c5e1772`.
- Read brief, implementer report, normative trust-policy attribution/invitation/linked-phase clauses, and the nonempty 1,575-line supplied package once in two sequential passes. No changed-file rereads or Cargo/test runs; no source, index, branch or Herdr actions.
- Named risk: public CLI capability bypass. Checked public `run_cooperative` entry and sibling raw-wire/compound routing (`src/cli/mod.rs:2561`, `:2650`, `:1712`), then used package context to trace mutation freeze -> capability guard -> new journal submission (`:2749`), and pending retry -> guard -> replay (`:2839`). Frozen detection recurses (`src/cli/journal.rs:294`). Raw-wire submission is still canonically refused.
- Named risk: canonical alternate-route bypass or writes before refusal. Searched Invite/DeliveryAction/PermitMutation sibling call sites; checked service dispatch (`src/service/dispatch.rs:818`, `:390`, `:466`), facade routing (`src/store/mod.rs:1544`), delivery envelope conversion (`src/protocol/handoff.rs:76`), and linked-phase validation (`src/store/topology_handoff/attachment.rs:279`). They converge on `control::invite` and its unconditional Some guard. Checked permit issuance (`src/store/mod.rs:1751`, `src/store/seats.rs:4119`): queries and permit construction occur in a rolled-back read transaction, with no durable mutation. The existing envelope validators do not independently compare confirmation; the canonical guard prevents a current bypass, and Task 6 must audit them when opening activation.
- Verified capability advertisement list excludes the new constant (`src/protocol/capabilities.rs:65`); no unsupported modern behavior is advertised.
- Reviewed report's final focused test/lint evidence without rerunning it: six confirmation tests, protocol capability tests, store connection/schema tests and store control regressions passed; final clippy/fmt/whitespace checks reported clean. Runtime/compile limitations are disclosed above.

### Test changes

- Ran successfully: `git diff --stat 957544e8849a36c5a211820aa5bcd3c4b0c92ea5..30c1f07c387ce0de6fcb562be75e8d9e6c5e1772 -- 'tests/**' 'test/**' 'spec/**' '**/tests/**' '*_test.*' '*.test.*' 'test_*.*' '*_spec.*' '*.spec.*'`.
- Result: 14 files changed, 282 insertions, 2 deletions.
- Ran the full matching `git diff BASE..HEAD -- <same pathspecs> | sed -n '1,400p'` successfully; output deliberately capped at 400 lines. A matching diff line-count command reports 541 total lines, so 141 lines were truncated from that supplemental view; all remaining changes were already inspected in the supplied full package.
- No tests deleted, skipped or loosened. Existing constructors gain only legacy `confirmation: None`. The two expectation replacements in `tests/store/schema.rs:6259` are justified by the required migration: indexes 139 -> 140, triggers 181 -> 183. Existing fresh/migrated catalog equality remains intact. New coverage adds wire/semantic compatibility, capability negotiation, canonical refusal and schema preservation/guards.
