# ht-akx implementation plan

| Ordinal | Bead | Task | filesTouched |
|---|---|---|---|
| 1 | ht-akx.1 | Implement bounded name-or-topic directory matching and rename-safe cursors | `src/store/queries.rs`, `src/store/control.rs`, `tests/store/queries.rs`, `tests/store/control.rs`, `tests/cli/read_cost_names.rs` |
| 2 | ht-akx.2 | Document literal thread-name discovery in CLI help and contract | `src/cli/commands.rs`, `src/protocol/commands.rs`, `README.md`, `docs/design/herdr-threads/shared-contract-amendment-adopted.md`, `tests/cli/commands.rs`, `tests/protocol/capabilities.rs` |
| 3 | ht-akx.3 | Integration sweep: bounded CLI thread-name discovery | `tests/store/queries.rs`, `tests/cli/read_cost_names.rs`, `docs/superpowers/runs/2026-10-10-thread-search-discovery/2026-10-10-thread-search-discovery-design.md`, `src/store/queries.rs`, `src/store/control.rs`, `src/cli/commands.rs`, `src/protocol/commands.rs` |

Spec: `docs/superpowers/runs/2026-10-10-thread-search-discovery/2026-10-10-thread-search-discovery-design.md` (binding authority).

Epic requirements (verbatim):

Fix CLI discovery mismatch where --search psa-global misses named threads because only topics are searched. Deliver bounded literal name-or-topic discovery, cursor correctness, compatibility and documented CLI contract. Spec will be committed in docs/superpowers/runs/2026-10-10-thread-search-discovery.

The epic has no separately labeled Global Constraints section; applicable project constraints and spec boundaries are reproduced below. Tasks 1 and 2 are independently ready. Task 3 depends on both. Protocol comments belong to Task 2. Task 3 source declarations cover only possible small integration repairs. No new test targets are planned.

## Task 1: Implement bounded name-or-topic directory matching and rename-safe cursors

filesTouched: ["src/store/queries.rs", "src/store/control.rs", "tests/store/queries.rs", "tests/store/control.rs", "tests/cli/read_cost_names.rs"]

Bead: `ht-akx.1`. Spec: `docs/superpowers/runs/2026-10-10-thread-search-discovery/2026-10-10-thread-search-discovery-design.md`. Dependencies: none within this epic.

### Verbatim bead requirements and acceptance

Implement case-sensitive literal substring OR matching across canonical optional thread name and topic in existing DirectoryQuery.topic_contains. Preserve bounded per-candidate traversal, output/page/high-water behavior and existing membership/archive/recent semantics; no canonical-ID matching and no picker changes. Add dedicated name/all filter revision on actual SetThreadName changes (set, rename, clear), no-op changes do not invalidate; bind name revision alongside existing topic/all only for filtered directory cursors, with legacy cursor/filter safety. Own focused meaningful store and CLI regression tests covering name-only/topic-only hits, None/empty needles, unnamed threads, both fields deduped, case-sensitive misses, metacharacters literal, matching/nonmatching pagination, membership/recent/archive scopes, rename entering/leaving match set after first page, rename no-op, topic changes, and unrelated directory writes preserving filtered continuation. Preserve all protocol field/shape compatibility and message-search behavior. Files-touched: src/store/queries.rs, src/store/control.rs, tests/store/queries.rs, tests/store/control.rs (name mutation suite), tests/cli/read_cost_names.rs (reuse isolated harness), protocol comments if needed. owns: canonical directory search predicate and name revision/cursor binding; consumes: existing DirectoryQuery.topic_contains and SetThreadName canonical transaction. Acceptance: focused regressions pass with unchanged limits and exact CLI --search name discovery; cargo fmt and per-change clippy pass. Read TRUST-POLICY before control.rs changes; update policy only if changing invariant, which is outside intended scope.
Inventory every canonical and managed/service thread-name SQL write site. Every actual existing-thread name change must publish name/all in its deciding transaction, or demonstrate the path cannot rename an existing thread. New-thread initial names remain high-water controlled.
Mandatory verification: run nice scripts/check-default-features before integration. Required regression detail: place a name-only hit beyond at least one work-limited zero-match page; follow cursors without duplicates/skips. Obtain mutation-driven stale results through production store dispatch, including set, rename and clear. Verify exact-operation replay stability as well as unchanged-name no-op stability.
Also verify selected-instance isolation, clearing a matching name, and unfiltered continuation behavior remains unchanged.

### Global Constraints

- Never restart, stop or kill the shared Herdr server, and never close Herdr workspaces/panes you did not create. Tests that need Herdr up/down use an isolated named Herdr test session (see `scripts/lib/isolated-herdr.sh`).
- Never write to the real user config: `~/.claude`, `~/.codex`, `~/.aisw`. Tests use isolated HOME / CLAUDE_CONFIG_DIR / CODEX_HOME under a temp dir.
- Never run `git push` or `git stash` (the stash stack is shared across worktrees). Commit on your own branch.
- A finished task stops its processes: every daemon, private Herdr server and helper process a test or script starts is stopped before the task reports done. Spawn test children with `herdr_threads::test_support::spawn` (`spawn_owned` / `command` / `tag`); never a bare `Command::spawn` in `tests/`.
- Until the flakiness side quest (ht-zo4) lands, do not run the full suite routinely; run focused tests for what you changed.

One store-focused leaf can own candidate projection, the predicate, dedicated name revision publication/binding, and focused store regressions because those changes share a correctness invariant. A documentation/CLI-contract leaf may follow that seam for help, protocol field comments, normative amendment and parser/contract verification. Shared `src/store/queries.rs` edits must remain serialized. Do not refactor the picker, add fuzzy CLI search, search message bodies/goals/IDs, alter name resolution, change membership/archival policy, restart the shared daemon, or deploy/install the branch as part of this run.

Required verification is `cargo fmt`, `nice cargo clippy --locked --all-targets --all-features -- -D warnings`, and relevant `nice cargo test --locked --all-features <filter>` runs. Before integration run `nice scripts/check-default-features`. Do not run the full suite routinely while ht-zo4 remains open. Tests use isolated storage, user configuration and private servers only; any owned processes must stop before reporting completion, and a full-suite run if separately required must use the prescribed leak-run ID/check.

### Implementation and verification steps

1. Read TRUST-POLICY.md before editing control.rs. Inventory all canonical and managed/service name SQL writes. Inspect service_controls.rs and create_thread to establish whether they can rename existing threads; report the sites and evidence. New-thread initial names need no revision bump. Report any additional write-file scope required before editing it.
2. Add focused failing directory tests in tests/store/queries.rs for all bead matching cases, including UTF-8, literal whitespace/punctuation, name-only psa-global with unrelated announcement topic, and deduplication. Exercise selected-instance isolation, ordinal/recent order and current membership/archive behavior. Put a name-only match after a work-limited zero-match page and follow every cursor, asserting exact IDs once without skips. Run the new test filters and confirm expected failures.
3. Extend ordinal and both recent SELECT projections and row decoding with nullable name. Evaluate topic.contains(needle) OR optional name.contains(needle) once within the existing bounded candidate loop before summary admission. Preserve None and empty-string semantics, high waters, ordering, counters, byte/row/work bounds, fit handling and result shape. No added scan or per-candidate name lookup. Rerun matching tests.
4. Add production-store-dispatch mutation tests in tests/store/queries.rs and tests/store/control.rs: get a filtered continuation, then set, rename into/out of the match set, or clear a matching name; cover recent and ordinal order and already-examined versus later candidates. Assert CursorStale and restart argv preservation. Verify exact replay and unchanged-name no-op stability, topic staleness, selected-instance isolation, unchanged unfiltered cursor semantics and unrelated membership activity preserving all-membership non-recent filtered continuation. Include rejection of legacy filtered keys. Run new filters to demonstrate intended failures before fixing revision behavior.
5. Publish name/all in set_thread_name's actual-change transaction beside directory/all, preserving replay/no-op behavior. Read name/all only for Some(topic_contains), defaulting absent rows to zero. Bind the separate name revision in filtered opaque last_examined_key for both orderings while keeping topic/all in filter_revision and all lifecycle/member/activity components. Unfiltered keys remain byte-for-byte unchanged. Cover any additional genuine rename path found in step 1 transactionally. Rerun mutation regressions.
6. Reuse tests/cli/read_cost_names.rs isolated harness for actual thread list --search psa-global with unrelated topic, asserting the desired thread appears. Stop all owned processes. Task 2 owns protocol comments; do not edit them here.
7. Run cargo fmt; relevant nice cargo test --locked --all-features <filter> selections for new and affected bounds/cursor/name cases; nice cargo clippy --locked --all-targets --all-features -- -D warnings; and nice scripts/check-default-features before integration. Report exact commands/results and name-write inventory. Deliver bounded name-or-topic discovery and rename-safe filtered continuation with no wire/trust change.

## Task 2: Document literal thread-name discovery in CLI help and contract

filesTouched: ["src/cli/commands.rs", "src/protocol/commands.rs", "README.md", "docs/design/herdr-threads/shared-contract-amendment-adopted.md", "tests/cli/commands.rs", "tests/protocol/capabilities.rs"]

Bead: `ht-akx.2`. Spec: `docs/superpowers/runs/2026-10-10-thread-search-discovery/2026-10-10-thread-search-discovery-design.md`. Dependencies: none within this epic.

### Verbatim bead requirements and acceptance

Update CLI --search help and README examples to state case-sensitive literal substring matching against thread name OR topic, rather than topic-only behavior. Amend docs/design/herdr-threads/shared-contract-amendment-adopted.md section 4 explicitly while preserving bounds and generic message topic/body search. Document picker fuzzy search as distinct only where needed for clarity; no broad redesign. Keep wire DirectoryQuery.topic_contains field and APIs unchanged. Files-touched: src/cli/commands.rs (--search declaration around line 1056), README.md, docs/design/herdr-threads/shared-contract-amendment-adopted.md, focused existing CLI argument tests if needed. Acceptance: --help clearly names both fields and literal case sensitivity, docs have no current contradictory discovery assertion, docs identify actual existing scopes without promising automatic all-page CLI traversal. This task consumes the already-decided spec search semantics; it can proceed independently of behavior code.
Mandatory protocol field comment: document DirectoryQuery.topic_contains in src/protocol/commands.rs (or its actual defining file) as the retained legacy wire name for literal name-or-topic matching, preserving serde spelling. owns: public CLI and protocol discovery documentation; consumes: decided literal name-or-topic specification.
Mandatory focused parser/contract verification: serialized DirectoryQuery keeps topic_contains and regenerated search continuation argv round-trip unchanged, preserving search text, scope, ordering and output context. Reuse existing tests where they already prove these assertions; add missing meaningful checks.

### Global Constraints

- Never restart, stop or kill the shared Herdr server, and never close Herdr workspaces/panes you did not create. Tests that need Herdr up/down use an isolated named Herdr test session (see `scripts/lib/isolated-herdr.sh`).
- Never write to the real user config: `~/.claude`, `~/.codex`, `~/.aisw`. Tests use isolated HOME / CLAUDE_CONFIG_DIR / CODEX_HOME under a temp dir.
- Never run `git push` or `git stash` (the stash stack is shared across worktrees). Commit on your own branch.
- A finished task stops its processes: every daemon, private Herdr server and helper process a test or script starts is stopped before the task reports done. Spawn test children with `herdr_threads::test_support::spawn` (`spawn_owned` / `command` / `tag`); never a bare `Command::spawn` in `tests/`.
- Until the flakiness side quest (ht-zo4) lands, do not run the full suite routinely; run focused tests for what you changed.

One store-focused leaf can own candidate projection, the predicate, dedicated name revision publication/binding, and focused store regressions because those changes share a correctness invariant. A documentation/CLI-contract leaf may follow that seam for help, protocol field comments, normative amendment and parser/contract verification. Shared `src/store/queries.rs` edits must remain serialized. Do not refactor the picker, add fuzzy CLI search, search message bodies/goals/IDs, alter name resolution, change membership/archival policy, restart the shared daemon, or deploy/install the branch as part of this run.

Required verification is `cargo fmt`, `nice cargo clippy --locked --all-targets --all-features -- -D warnings`, and relevant `nice cargo test --locked --all-features <filter>` runs. Before integration run `nice scripts/check-default-features`. Do not run the full suite routinely while ht-zo4 remains open. Tests use isolated storage, user configuration and private servers only; any owned processes must stop before reporting completion, and a full-suite run if separately required must use the prescribed leak-run ID/check.

### Implementation and verification steps

1. Read the spec, --search help, README discovery prose, adopted contract section 4 and DirectoryQuery declaration. Inspect existing tests/cli/commands.rs::continuation_argv_round_trips_context_filter_format_and_bounds and tests/protocol/capabilities.rs::picker_directory_wire_keeps_old_directory_shape_and_cursor_only_contract to identify existing coverage.
2. Add only missing meaningful compatibility assertions to those existing modules: serialized nonempty DirectoryQuery search retains topic_contains; regenerated continuation argv parses back preserving literal text, scope, recent ordering, output context, cursor and bounds. Reuse existing checks for already-proven assertions. If help checks exist, extend for both fields and case-sensitive literal semantics. Run focused tests; preserved compatibility may already pass, while new help assertions should fail before the text edit.
3. Update --search help to state case-sensitive literal substring matching of thread name or topic. Update README discovery examples with --search psa-global and actual scope/paging semantics, without promising automatic page traversal. Explicitly supersede section 4's topic-only directory rule; preserve generic message topic/body search and bounds. Explain picker fuzzy behavior only where necessary.
4. Add mandatory DirectoryQuery.topic_contains comment: retained legacy wire name now means literal name-or-topic matching. Preserve serde spelling, validation, request/result shapes and version. Search maintained discovery docs for contradictions, reporting any necessary extra file declaration before editing.
5. Run focused parser/contract checks, cargo fmt, prescribed all-feature clippy and nice scripts/check-default-features before integration. Report exact evidence. Deliver independently testable docs/compatibility without relying on Task 1 runtime behavior.

## Task 3: Integration sweep: bounded CLI thread-name discovery

filesTouched: ["tests/store/queries.rs", "tests/cli/read_cost_names.rs", "docs/superpowers/runs/2026-10-10-thread-search-discovery/2026-10-10-thread-search-discovery-design.md", "src/store/queries.rs", "src/store/control.rs", "src/cli/commands.rs", "src/protocol/commands.rs"]

Bead: `ht-akx.3`. Spec: `docs/superpowers/runs/2026-10-10-thread-search-discovery/2026-10-10-thread-search-discovery-design.md`. Dependencies: ht-akx.1 and ht-akx.2 integrated.

### Verbatim bead requirements and acceptance

Verify the combined CLI name-or-topic search main flow and docs against the settled spec. Use isolated existing harnesses, never shared Herdr service/config writes. Exercise actual CLI thread list --search psa-global on an isolated named thread with unrelated announcement topic, and production rename/set/clear cursor staleness; inspect missing integration coverage not already owned by the leaf regressions, add meaningful tests only for gaps, fix small integration errors inline and file blockers for larger gaps. Sweep for unwired name revisions, candidate projections or help/legacy field semantics. Review exact combined tree, run focused relevant tests, cargo fmt, prescribed clippy and scripts/check-default-features as needed; final expensive once-per-branch checks belong to super-auto after code roast, do not run full suite routinely. Files-touched: focused existing tests/store/queries.rs and tests/cli/read_cost_names.rs if uncovered seams need tests, implementation files only for small integration gaps, spec Post-Implementation Notes and verification evidence. owns: integrated goal verification and uncovered integration tests; consumes: implemented bounded discovery behavior and public CLI/protocol contract. Acceptance: exact CLI discovery and bounded rename-safe continuation satisfy spec using behavior (needs: ht-akx.1) and public help/contracts (needs: ht-akx.2). blocked-by ht-akx.1: consumes all leaves (integration sweep). blocked-by ht-akx.2: consumes all leaves (integration sweep).

### Global Constraints

- Never restart, stop or kill the shared Herdr server, and never close Herdr workspaces/panes you did not create. Tests that need Herdr up/down use an isolated named Herdr test session (see `scripts/lib/isolated-herdr.sh`).
- Never write to the real user config: `~/.claude`, `~/.codex`, `~/.aisw`. Tests use isolated HOME / CLAUDE_CONFIG_DIR / CODEX_HOME under a temp dir.
- Never run `git push` or `git stash` (the stash stack is shared across worktrees). Commit on your own branch.
- A finished task stops its processes: every daemon, private Herdr server and helper process a test or script starts is stopped before the task reports done. Spawn test children with `herdr_threads::test_support::spawn` (`spawn_owned` / `command` / `tag`); never a bare `Command::spawn` in `tests/`.
- Until the flakiness side quest (ht-zo4) lands, do not run the full suite routinely; run focused tests for what you changed.

One store-focused leaf can own candidate projection, the predicate, dedicated name revision publication/binding, and focused store regressions because those changes share a correctness invariant. A documentation/CLI-contract leaf may follow that seam for help, protocol field comments, normative amendment and parser/contract verification. Shared `src/store/queries.rs` edits must remain serialized. Do not refactor the picker, add fuzzy CLI search, search message bodies/goals/IDs, alter name resolution, change membership/archival policy, restart the shared daemon, or deploy/install the branch as part of this run.

Required verification is `cargo fmt`, `nice cargo clippy --locked --all-targets --all-features -- -D warnings`, and relevant `nice cargo test --locked --all-features <filter>` runs. Before integration run `nice scripts/check-default-features`. Do not run the full suite routinely while ht-zo4 remains open. Tests use isolated storage, user configuration and private servers only; any owned processes must stop before reporting completion, and a full-suite run if separately required must use the prescribed leak-run ID/check.

### Implementation and verification steps

1. Wait for both leaf integrations. Read their reports and the exact integrated diff against the binding spec. Build evidence for matching semantics, candidate limits, all name-write sites, cursor revisions, exact CLI behavior and public/wire contracts. Reuse leaf regressions rather than duplicate them.
2. Verify actual CLI thread list --search psa-global with unrelated announcement topic through the isolated harness. Inspect both traversal projections and production-dispatch set/rename/clear stale tests, replay/no-op stability, selected-instance isolation, legacy filtered cursor rejection and unchanged unfiltered semantics. Check help and legacy field/argv round-trip evidence together.
3. For genuine uncovered seams, first add failing tests in tests/store/queries.rs or tests/cli/read_cost_names.rs, then make small integration fixes within declared source files and rerun. Source files declared here form a conditional repair envelope, not required edits. Report new file scope before expansion; ask the coordinator to handle larger blocker gaps.
4. Run relevant nice cargo test --locked --all-features <filter> selections on the combined tree, cargo fmt, nice cargo clippy --locked --all-targets --all-features -- -D warnings, and nice scripts/check-default-features. Stop owned processes. No routine full suite: upstream owns final expensive once-per-branch checks after code roast.
5. Append dated Post-Implementation Notes in the design recording discovered facts, deviations or confirmed assumptions. Record exact verification commands/results and test references in the SDD task report. Deliver integrated goal evidence and any small seam corrections.
