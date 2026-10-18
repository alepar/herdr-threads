# Literal Name Paths Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Address slash-containing labels using one explicit human-usable name-path syntax without silently redirecting existing selectors.

**Architecture:** Add opt-in `path:` arguments to the shared seat selector: `/` separates components, `\/` represents a literal slash and `\\` a literal backslash. Ordinary inputs remain byte-for-byte compatible. Check original raw selector candidates as well and refuse collisions with different panes, regardless of proximity. All seat slots consume the existing shared resolution stage; separate --space/--tab/--pane flags remain literal.

**Compatibility ruling:** Ordinary paths have at most three components; a nickname may have a three-component prefix plus an existing seat ID. Existing raw nickname scopes retain precedence and their mismatch refusal even under narrowing. A raw nickname keeps its historical priority over raw full-name matches when no distinct decoded target exists; distinct valid decoded name/nickname targets conflict.

**Tech Stack:** Rust, clap, existing topology resolver, isolated Herdr integration fixture.

**Spec:** none — ad-hoc ht-40iq handoff; AGENTS.md and TRUST-POLICY.md remain normative.

## Global Constraints

- Preserve canonical seat-ID precedence, pane-ID behavior, proximity, narrowing, ambiguity refusal and frozen retries.
- First reproduce on actual slash-containing workspace, tab and pane labels in an isolated named Herdr session; inspect actual child labels and never guess them.
- No shared server restart, user configuration writes, git push, git stash, or full test suite.
- Run focused tests, fmt, clippy all features, check-harness-neutral, check-default-features; stop owned processes.
- Commit trailers Outside-adapter and Co-Authored-By; commit explains interface alternatives and sizes of new model-visible errors/help.

### Task 1: Explicit escaped name paths, complete vertical slice

**Files:**
- Modify `src/cli/seat_selector.rs`: strict parser, readings, nickname prefix, collision detection and invalid-input errors.
- Modify `src/cli/seat_resolution.rs`: propagate validation and preserve ID precedence, canonical journaling and nickname verification.
- Modify `tests/cli/seat_selector.rs`: literal tables and resolver tests.
- Extend existing `tests/integration/session_bindings_e2e.rs` with the two `literal_name_paths` tests, reusing its Smoke import. No new suite or registration: AGENTS.md requires new stateless suites in combined, so the final review corrected the original standalone integration-module placement.
- Modify `src/cli/commands.rs` and relevant help golden fixtures; document compactly in `integrations/skill/SKILL.md` and `docs/CHANGELOG.md`.

**Interfaces:** Existing resolve/resolve_with and nickname verification remain the common consumers for every seat slot. A parser may return owned decoded components or ApiError; avoid decoding again or adding command-specific branches. `Outcome` may gain invalid syntax if needed. Existing exact --space/--tab/--pane values never decode.

- [x] **Step 1: Reproduce and write failing tests.** Reuse `session_bindings_smoke::Smoke`, which uses `scripts/lib/isolated-herdr.sh`. Create a workspace labeled `prj/herdr-threads`, inspect its real root pane/tab IDs, rename the actual tab and pane to slash-containing labels, verify them using Herdr get/list, allocate its seat. Show that the raw path fails and explicit flags/exact IDs work. Assert `seat inspect 'path:prj\/herdr-threads/tab\/main/pane\/main'` resolves the same seat. Add pure table tests for all three components, live agent names, omitted levels, Unicode, literal backslash, escaped escape followed by separator, invalid/trailing escapes, empty/excess components, nickname prefixes and collisions. Raw backslashes and raw `path:` labels must keep their original meaning or refuse a collision; never silently retarget.

- [x] **Step 2: Run focused tests and verify expected failure.** `nice cargo test --locked --all-features --lib cli::seat_selector` and `nice cargo test --locked --all-features --test integration literal_name_paths -- --nocapture`. Capture actual reproduction evidence before implementing.

- [x] **Step 3: Implement and document.** Decode only the explicit `path:` form; accept exactly `\/` and `\\` and reject all other/trailing escapes with a compact invalid-request error. One to three nonempty decoded components use the existing level mappings and aliases. Existing-seat nicknames may additionally have a three-component prefix plus their final seat ID (four total), matching the legacy nickname form. Raw literal matches must not be silently displaced by the decoded form: compare candidates before proximity can conceal a collision, including nickname handling. Keep exact ID precedence. If an existing literal `path:` name itself matches, preserve it; any distinct decoded target causes conflict. Malformed opt-in syntax may still name an existing raw literal selector, otherwise return a clear syntax error. Document shell single quoting and the two escapes; flags stay literal. Compare JSON array (strict but heavy quoting), new flags (cross-command burden), alternate delimiter (same escaping problem), and global decoding (breaks compatibility) in the commit.

- [x] **Step 4: Verify green.** Re-run the focused parser and CLI tests; run existing shared seat-resolution tests as appropriate. `cargo fmt`; `nice cargo clippy --locked --all-targets --all-features -- -D warnings`; `scripts/check-harness-neutral`; `nice scripts/check-default-features`. Fix failures, ensure fixture teardown, inspect diff.

- [x] **Step 5: Commit.** Commit only this feature on feat/ht-40iq, with the required trailers, interface rationale and model-visible text sizes. Return SHA, tests/results, reproduction evidence and remaining risks for independent review.
