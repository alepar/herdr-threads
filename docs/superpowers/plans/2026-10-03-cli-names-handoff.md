# CLI names and handoff Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the user-approved combined CLI names, channel picker and recoverable handoff design.

**Architecture:** Resolve human selectors into canonical IDs at the CLI boundary and retain daemon decisions for seat/caller authority. Store indexed nonunique thread names and indexed recent activity in SQLite. The picker consumes paged read-only results; handoff persists a compound private plan and uses existing keyed mutations and guarded launch.

**Tech Stack:** Rust, clap, rusqlite, serde, libc, existing native Herdr adapter.

**Spec:** docs/superpowers/specs/2026-10-03-cli-names-handoff-design.md

## Global Constraints

- Work only in branch cli-human-targets in its isolated worktree; never merge main, push, tag or release.
- Targeted tests only; no full suite. Coordinator owns final sweep, cleanup and v0.2.2 publication.
- Never stop/restart shared Herdr or change real ~/.claude, ~/.codex, ~/.aisw. All probes use private named sessions and isolated configuration.
- Preserve TRUST-POLICY.md: labels only select requests, canonical daemon decisions own seat/binding authority and honest provenance.
- Exact IDs stay canonical; no fuzzy execution selectors or arbitrary first match. Conflicts report escaped candidates and exact IDs.
- Read never ACKs; foreign-pane inbox is read-only. Native approvals stay native: no network grant or implicit full access.
- Per change: cargo fmt, nice cargo clippy --locked --all-targets --all-features -- -D warnings. Before handoff: nice scripts/check-default-features, cargo fmt --check, git diff --check, scoped leak check.
- Tests use existing combined target for process-free suites; process-owning fixtures use herdr_threads::test_support::spawn and required-features.

### Task 1: Scoped human pane targets and recipient ergonomics

**Files:** modify src/cli/commands.rs, src/cli/panes.rs, src/cli/mod.rs, src/host/observation.rs, src/host/native.rs; focused tests in tests/cli/panes_hint.rs and tests/cli/cooperative.rs; update docs/agent-usage.md and integrations/skill/SKILL.md target guidance.

**Interfaces:** consume RuntimeContext and caller_pane in run_in_pane. Produce reusable PaneSelector { space: Option<String>, tab: Option<String>, pane: Option<String> } plus a validated host topology snapshot and a pure resolve_selector returning HostTargetId. Store parsed target-selection metadata separately from canonical wire types, and expose resolution for handoff later. Keep parse_argv pure and testable; defer host reads until execution.

- [ ] Write parser and resolver tests before implementation: invite THREAD --tab tryout --pane alice parses; labels with identical names in other tabs are ignored; label/agent collisions are Conflict; parent mismatch with pane ID fails; omitted selector uses live caller; different explicit parent selects sole child or Conflict; unavailable host never passes names through; explicit IDs remain unchanged; foreign-pane inbox cannot display ACK; read selector never allocates; invite can guarded-resolve an empty target. Include caller-moved/stale-context and escaped candidates.
- [ ] Run `nice cargo test --locked --all-features --lib cli::panes` and relevant parser tests to record expected RED. Compile errors for genuinely new interfaces count as missing-feature RED; do not confuse fixture errors with product gaps.
- [ ] Implement selector parsing and topology extraction from a single validated session snapshot; resolve caller by live `pane current --current` on the selected endpoint when needed. Resolve exact IDs first; validate explicit parents, union pane labels and live agent names, deduplicate and enforce unique scoped match. Existing single-pane-tab alias stays within scope. Add alternatives for invite and seat-filtered reads and repeatable --require-ack-pane; freeze resolved recipient seat IDs before journaling. Use daemon read mappings for reads, guarded allocating resolve for invite/send targets. --cooperative-target participates without altering the claimed seat/harness/role. Require explicit pane for launch/handoff/rebind/fresh-seat, default ordinary resolve to caller.
- [ ] Run focused resolver/parser/cooperative tests and required per-change lint/format. Self-review all pane/seat selectors for caller-versus-recipient confusion; document concrete defaults and errors. Commit only this task's files and report signatures for later tasks.

### Task 2: Optional thread names everywhere

**Files:** src/protocol/commands.rs, src/protocol/results.rs, src/protocol/output*.rs, src/store/schema.rs, src/store/control.rs, src/store/queries.rs, src/daemon dispatch, src/cli/commands.rs, src/cli/mod.rs, new focused src/cli/threads.rs if useful, src/cli/journal.rs, human/compact renderers, relevant client request encoders, tests/cli and tests/store, docs/agent-usage.md, integrations/skill/SKILL.md.

**Interfaces:** nullable name in thread storage and optional serde-compatible name in summaries; indexed read-only thread-selector resolution returns an exact ThreadId or NotFound/Conflict. Journaled create carries optional name; journaled name set/clear carries canonical ThreadId and existing caller claim. Thread resolution helper walks every CLI thread argument including filters/follow/summary and freezes IDs before caller dispatch and mutation intent. Task 1's target metadata must remain intact.

- [ ] Write focused real-store/parser tests: create named/unnamed; nonunique names; exact existing ID wins name collision; unique match over all instance memberships and archives; cross-instance isolation; no match; duplicate candidates; 128-byte boundary and controls; name set/clear/rename authorization; migration preserves existing IDs and unnamed rows; durable replay and rename after frozen selection. Include every thread selector in a table-driven CLI parse/resolve test with literal expected IDs.
- [ ] Run focused tests for expected RED, then add schema migration and nonunique (instance_id,name) index. Implement bounded indexed resolution that detects second match and emits bounded escaped candidates. Add public `thread create --name`, `thread name THREAD [--set NAME|--clear]`, `thread rename THREAD NAME`; names are separate from topics/goals. Existing-thread writes retain joined-caller authorization. Update command dispatch, protocol serde defaults, journal semantic digest/replay and presentation consistently.
- [ ] Integrate resolution across show/topic/participants and alias, invite, accept/accept-required, leave, send, archive/reopen, read/follow, pending-receipts --thread, diagnostics --thread, warnings --active, search --thread, summary THREAD, and name commands. Never reinterpret recovery refs, invitation IDs, message IDs, summary job IDs or lease tokens as thread names. Continuations/hook commands retain exact IDs.
- [ ] Run focused name/migration/CLI/render tests and required lint/format; commit with documentation and report the interfaces for recent picker/handoff.

### Task 3: Recent directory and builtin human picker

**Files:** src/store/schema.rs, src/store/schema timeline publication helpers, src/store/queries.rs, src/protocol/commands.rs/results.rs and pagination/output, src/cli/commands.rs/mod.rs, new src/cli/picker.rs, src/cli/follow.rs only if required, focused store/picker tests, docs/agent-usage.md.

**Interfaces:** directory recent-order flag defaults false for existing requests; rows expose optional name plus canonical ID and last activity. Indexed activity-order cursor freezes traversal (invalidate/restart when relevant movement would skip rows); existing ordinal directory order remains unchanged. Bare read parses to a distinct local picker action carrying existing ReadArgs history options. Picker selects an exact ThreadId then executes ordinary history/follow.

- [ ] Write failing store tests for activity updates and migration backfill, ordering/tie stability, >one page, relevant concurrent activity/rename invalidation, all memberships/archives, and existing directory order. Write pure matcher/navigation tests plus terminal boundary tests for agent/nonTTY/machine/JSON/dumb TERM refusal, cancellation/empty filtered list, escaped hostile rows, page loading after zero matches, terminal restore, and selected read not ACKing.
- [ ] Run focused RED. Add indexed materialized activity updated at committed timeline publication, migrated once; do not rely on bounded HotThreads or fetch the entire directory to sort it. Add `thread list --recent --all`, with exact continuation commands/cursor binding. Keep bounded store work and output budgets.
- [ ] Implement a small libc-backed terminal picker (no external fzf). Fuzzy subsequence matching on names/topics only; arrows and Ctrl-N/P navigation, Enter selects canonical ID, Esc/Ctrl-C cancels exit 0. Require stdin/stdout/stderr TTY, nondumb TERM, human presentation and no agent marker. Show loading/more while bounded pages progressively load; no silent prefix inventory, and no implicit selection when no match. Restore terminal state by RAII including failures. UI uses stderr, transcript stdout; bare --cursor is InvalidRequest, explicit read unchanged, default picker history recent 20.
- [ ] Run focused store/picker/history/follow tests, required lint/format, and one owned terminal/private-instance exercise with cleanup. Commit and report callable interfaces for final docs.

### Task 4: Recoverable durable handoff then guarded launch

**Files:** new src/cli/handoff.rs, src/cli/commands.rs/mod.rs/journal.rs/retry.rs, src/cli/launch.rs, src/harness/launch.rs only for shared preflight if necessary, focused tests/cli/handoff.rs (registered through existing lib/combined seams), docs/agent-usage.md, integrations/skill/SKILL.md, TRUST-POLICY.md only if an accepted invariant limit actually changes.

**Interfaces:** HandoffRequest combines exact caller selection, frozen recipient pane/seat, existing ThreadId or new topic/goal/name, harness/name/native arguments and one durable body. Private compound plan persists sub-operation keys, original payload, completed results and possible-start boundary with sync/rename discipline. Pending-ops/retry recognize compound recovery refs and preserve scope. Reuse Task 1 selectors, Task 2 named thread resolution and existing launch composition.

- [ ] Write failing parser tests for exactly one --new-thread/--thread, one body after --, new-thread-only options, explicit target requirement, native --agent-arg and agent --name versus --thread-name. Real journal/controlled LocalClient tests cover create/invite/send/launch order, sender membership, already-joined invite, freeze/retry on rename, broken output, crash at every durable boundary, duplicate prevention, definitive rejection, target unsafe and uncertain native start.
- [ ] Run focused RED. Implement required preflight before durable thread work: body/caller/thread membership, resolved target, harness/hook status and available shell as production evidence permits. Persist compound plan with canonical IDs and exact keyed steps. New topic defaults `Handoff to DISPLAY` bounded appropriately, goal defaults topic. Create joins sender; invite does not join recipient; send explicitly addresses recipient; launch retains guarded fences and native refusal policy.
- [ ] Store body once; pass a fixed bootstrap with canonical thread/inbox commands as native initial prompt. `handoff -- ...` is a single quoted body, while `launch -- ...` stays unchanged; handoff --agent-arg passes native options. Persist possible-start marker before native call. Retry definite pre-start failures safely; unknown launch/crash across start never auto-repeats launch. Preserve committed create/invite/send and print completed IDs, failed phase and exact recovery/inspect/manual-launch argv. Successful launch is no accept/ACK/check-in.
- [ ] Run focused parser/journal/recovery/launch tests, required lint/format and owned private native scenario(s), with isolated homes and scoped leak cleanup. Update help, embedded skill and docs. Commit and report remaining checks.

### Task 5: Combined documentation, evidence and readiness

**Files:** README.md, docs/agent-usage.md, integrations/skill/SKILL.md, docs/superpowers/specs/2026-10-03-cli-names-handoff-design.md/INDEX.md, docs/evidence/cli-names-handoff/report.md; any narrowly required contract tests or fixes.

**Interfaces:** consumes final public help, schema and runtime behavior. Produces sanitized evidence report with exact commands/results/limitations and frozen branch SHA/base readiness package for coordinator.

- [ ] Reconcile public help and all docs with implemented selectors/name management/picker/handoff. Add before/after examples, outside-Herdr guidance, conflicts, cancellation, noninteractive behavior, scope, partial failure/retry, attribution and receipts. Keep paths sanitized in public evidence; fix touched outdated human-waiver/display-ACK prose without unrelated rewriting.
- [ ] Validate CLI help and focused end-to-end paths in owned private instances only. Every process is tagged and stopped; run scoped `scripts/check-no-leaked-processes --run-id ID`.
- [ ] Run `nice cargo clippy --locked --all-targets --all-features -- -D warnings`, `nice scripts/check-default-features`, `cargo fmt --check`, `git diff --check`; targeted relevant tests only. Independent broad final review must pass before frozen readiness. Do not rerun passing checks without changed code or new concerns.
- [ ] Update spec Post-Implementation Notes and evidence with actual implementation/verification, commit, ensure branch clean. Send threads-main one frozen milestone with SHA/base/checks/review/evidence/clean/no-process state; coordinator independently verifies and performs squash integration, final sweep, cleanup and publication. Wait for coordinator DONE+MERGED verification; never merge/release or remove the worktree yourself.

## User-approved Task 3 amendment (2026-10-03): relative transcript nicknames
Human explicit or picker-selected `read`, including follow, displays author and event recipient pane nicknames relative to the live caller: `alice` in the same tab, `tryout/alice` in another tab of the same workspace, `project/tryout/alice` in another workspace. Compare canonical workspace/tab IDs, never equal label strings, and retain the pane component with each displayed component bounded independently. Missing labels use IDs; hostile labels are escaped. Host failure must not break durable history. Extend the existing bounded `follow::NickCache` / `irc::Lookup` seams and share `irc::relative_pane_nick` with Task 4 handoff. No additional SQLite migration, identity decision, canonical machine/recovery ID, or authority change. This amendment is included in Task 3 implementation and independent review.
