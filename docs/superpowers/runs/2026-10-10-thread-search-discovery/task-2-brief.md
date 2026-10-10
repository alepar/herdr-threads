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
