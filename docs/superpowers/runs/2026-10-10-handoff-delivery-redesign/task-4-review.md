### Spec Compliance

- **Needs fixes.** Source acquisition, frozen payloads, bootstrap/delivery capacities and the populated-schema upgrade satisfy the reviewed requirements, but explicit-pane legacy handoff publication lacks the required exact encoded-capacity preflight (Important I1 below).
- Reviewed BASE `957544e8849a36c5a211820aa5bcd3c4b0c92ea5` through HEAD `1fe52281c2c9ba53269fb0c1772741cc97352995`. Read the full 2,727-line review package once in four sequential sections, including every test hunk. Read the brief, implementation report and normative TRUST-POLICY.
- Integration remains separate: provisional migration 29 must become the coordinator-assigned version and receive the fresh overlapping-schema/FK seam review. This is not a missing Task 4 requirement.

### Strengths

- `src/cli/commands.rs:1291`, `src/cli/commands.rs:2569`, and unchanged `src/cli/input.rs:35` enforce an exclusive source and bounded 65,537-byte probe before execution. `src/cli/mod.rs:346` parses before namespace resolution or journal creation. `src/cli/handoff.rs:114` and `src/protocol/handoff.rs:581` preserve ordinary message bytes independently of topic/goal checks.
- `tests/cli/handoff.rs:3790`, `tests/cli/handoff.rs:3907`, `tests/cli/handoff_delivery.rs:2636`, and `tests/cli/topology_handoff.rs:2298` exercise maximum body sources, source deletion/mutation, exact retry payloads and native argv exclusion across the three routes. The new stdin test retains the acquired body after its reader disappears.
- `src/cli/journal.rs:1337` preflights delivery/bootstrap origins before reservation; `src/cli/handoff_delivery.rs:604` preflights the actual nested terminal before completion. `src/cli/topology_handoff.rs:1032` preserves the derived terminal formula, and its unchanged completion callback at `src/cli/topology_handoff.rs:1607` checks the actual completed and nested-original envelopes before canonical completion.
- `migrations/0029_handoff_body_bounds.sql:4` copies historical blobs and restores original table names; `src/store/schema.rs:3929` disables FK enforcement outside the transaction, validates the rebuilt schema/whole FK graph, rolls back on failure and restores the original setting. `tests/store/topology_handoff_schema.rs:411` exercises a physically reopened populated v28 database and exact preservation of six tables and canonical history. Original migration 0028 is not changed in the package.
- `src/archival_legacy.rs:420`, `src/archival_legacy.rs:756`, and `src/archival_legacy.rs:1149` reuse owned progress/terminal/origin limits. Generic non-handoff 64 KiB records, 4 KiB headers, metadata-only 128 KiB envelopes and global 1 MiB framing stay unchanged. `tests/store/archival_legacy.rs:548` exercises the actual scanner with escaped maximum bodies and delivery cleanup.

### Issues

#### Critical

- None found.

#### Important

- **I1 — `src/cli/journal.rs:1339`: the new publication-cap match omits `IntentKind::Handoff`, although `src/archival_legacy.rs:798` now applies the 512 KiB body-bearing origin limit to that kind.** The public explicit-pane route reaches `handoff::start` and publishes its frozen `HandoffPlan` at `src/cli/handoff.rs:701` without an encoded-origin check. This is reachable with native inputs admitted by the production rules: a 65,536-byte U+0001 body contributes 393,216 encoded bytes, and two `--agent-arg=--model=<value>` arguments each containing 12,000 U+0001 bytes contribute another 144,000 encoded value bytes. That is 537,216 bytes before metadata, already above 524,288. Each argument remains below 16 KiB and their raw total below 32 KiB; U+0001 is permitted by `src/ports.rs:1862`, and the Codex startup grammar accepts nonempty `--model=` values at `src/harness/codex.rs:2123` without rejecting these controls. Ordinary routing/hooks leave sufficient native headroom. Publication can therefore succeed, and subsequent scanner coverage refuses the producer's own retained envelope. This misses the assigned exact-preflight/independent-oversized-metadata contract; it does not require changing historical generic `load`/`decode_reader` acceptance. Add the typed legacy Handoff origin capacity check before reservation/publication, and a regression proving overflow leaves the ordinal, published intents and downstream effects unchanged. If choosing a different narrowly bounded legacy capacity instead, align the writer/scanner/policy and justify it with actual serializer fixtures.

#### Minor

- No additional code findings. `task-4-report.md:65` and `task-4-report.md:89` honestly record production build budget overruns; the later warm fixture rebuild and 43.72-second lint do not establish that every production-source incremental build meets one minute. Keep that limitation visible at integration.

### Checks and Limits

- Named cross-cutting checks: public parse-to-execution ordering (`src/cli/mod.rs:346`); bounded shared input (`src/cli/input.rs:35`); explicit-pane publication/native-option reachability (`src/cli/handoff.rs:647`, `src/ports.rs:1853`, `src/harness/codex.rs:2053`); bootstrap completion and retained formulas (`src/cli/topology_handoff.rs:1032`, `src/cli/topology_handoff.rs:1607`). These focused unchanged call-site reads address the source/effect, escaping and owned-loader risks; no broad codebase review was performed.
- Ran the required test-only `git diff --stat BASE..HEAD -- tests` and `git diff BASE..HEAD -- tests | head -400`. The latter was deliberately truncated at 400 lines; the complete test changes were reviewed in the package. No unauthorized skip, deletion, assertion weakening or timeout increase was found. The whitespace-invalid fixture becomes empty-invalid, the recovery boundary becomes the approved exact 1 MiB/max+1, and staged-prefix setup retains the original budget and progress assertions.
- No Cargo or test reruns, source/index/branch mutations, Herdr operations or real-config writes. Recorded final evidence: topology 73, CLI 20, library handoff 237, maximum fixtures 11, journal 54, archival 33; clippy 43.72 seconds without warnings; formatting and scoped leak checks clean. These are the implementer's recorded results, not independent reruns.

### Assessment

**Spec compliance: Needs fixes. Quality: Needs fixes.**

The implementation handles exact source bytes and the difficult bootstrap/delivery persistence limits carefully, with substantial real serializer and migration coverage. The missing legacy publication preflight leaves one owned producer/consumer capacity contract inconsistent and should be fixed before Task 4 approval.
