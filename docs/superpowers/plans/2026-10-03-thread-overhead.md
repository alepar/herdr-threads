# Daily Thread Overhead Implementation Plan

> **For agentic workers:** Execute each task with test-first focused cycles and independent review. Coordinator controls the full-suite slot and main merge.

**Goal:** One text inbox command displays bounded actionable content and ACKs only fully displayed pending agent messages; ordinary wakes batch for 30 seconds by default; condition warnings notify once on open and clear.

**Architecture:** The daemon returns a bounded inbox batch under its canonical read snapshot. The CLI writes and flushes that page before committing exact receipt IDs using its durable intent journal. The scheduler keeps a separate persisted first-wake deadline, while warning transitions use canonical condition rows and existing bounded fanout jobs.

**Tech Stack:** Rust, SQLite, clap, typed Unix protocol, private named Herdr sessions.

**Spec:** `docs/superpowers/specs/2026-10-03-thread-overhead-design.md`.

## Global Constraints

- `--json` and `--machine` inbox paths and existing reads are read-only; text default inbox ACKs only after full-page write and flush.
- Exactly the displayed canonical pending agent receipt IDs may be ACKed; a partial body is not ACKed.
- `empty` is the entire compact text result only for a complete, genuinely empty inbox; failures are visible.
- Bounded high-water cursors preserve continuation and retry under arrivals and ACKs.
- `wake_batch_delay_ms` defaults to 30000, allows 0, and does not change retry spacing or host safety guards.
- Only a first open and a recorded clear of one canonical condition notify; reopening notifies again. Migration never invents historic clears.
- Use full stored typed IDs. No real home settings, shared Herdr server changes, push, stash, or independent merge.
- Focused tests and per-change clippy are permitted only outside coordinator holds. Full suite needs an exclusive coordinator slot. Stop owned processes and run scoped leak checks after any suite.

---

### Task 1: Text inbox page and exact displayed ACK

**Files:** `src/protocol/commands.rs`, `src/protocol/results.rs`, `src/protocol/pagination.rs`, `src/store/queries.rs`, `src/store/mod.rs`, `src/cli/commands.rs`, `src/cli/mod.rs`, `src/cli/inbox.rs`, `src/protocol/output_compact.rs`, `src/cli/retry.rs`, `src/cli/journal.rs`, `TRUST-POLICY.md`, `tests/store/queries.rs`, `tests/cli/cooperative.rs`, `tests/cli/commands.rs`.

**Interfaces:** `InboxBatchQuery` is read-only and returns `Page<InboxBatchItem>` with exact ACK candidate IDs only for complete ordinary bodies. An additive `AckDisplayed(Ack)` command carries a distinct frozen operation kind and records `cooperative_inbox_display` action provenance while preserving the actor's binding provenance. The existing `Ack`, `InboxQuery`, and machine/JSON output remain compatible and read-only where applicable.

- [ ] Write focused tests for no ACK on partial write, failed flush, cancellation before submission, JSON/machine read, foreign seat, partial body, and required-service invitation; successful text page ACKs exactly complete pending agent message IDs. The writer test double returns an error after a chosen byte count and a separate flush error, so the observable daemon call log must contain no ACK in either case. A cancellation after submission retains an uncertain intent for replay.
- [ ] Run `nice cargo test --locked --all-features inbox_display` and confirm each new test fails for missing behavior.
- [ ] Add a bounded daemon page with cursor binding seat, scope, global publication fence, physical/manifest receipt high waters, invitation/warning high waters, and intra-thread/body positions. Fit selected output before returning at most 100 ACK IDs. Test 101 small pending messages, a body above page budget, arrivals into visited and unvisited old threads, and projection after page one.
- [ ] Add a text CLI path that renders the page, writes and flushes it, then journals one frozen ACK for `ack_candidates`. Return any ACK failure after the displayed page as an explicit error with recovery reference. Do not send an ACK for zero candidates.
- [ ] Run the focused tests, `cargo fmt`, and `nice cargo clippy --locked --all-targets --all-features -- -D warnings` when the coordinator allows load.

### Task 2: Durable first-wake batch window

**Files:** `src/daemon/settings.rs`, `src/service/config.rs`, `src/notification/policy.rs`, `src/notification/dispatch.rs`, `src/scheduler/mod.rs`, `src/store/wake.rs`, `src/store/mod.rs`, `src/ports.rs`, `src/service/workers.rs`, migration chosen after the latest main schema, `tests/scheduler/dispatch.rs`, `tests/store/wake.rs`, `tests/service/composition.rs`.

**Interfaces:** `wake_batch_delay_ms` is independent of `minimum_wake_delay_ms`; the scheduler asks the store for a canonical ordinary-attention batch deadline and enforces it before reservation. A zero delay returns immediately eligible while retained retry spacing still applies.

- [ ] Write failing focused tests for first attention at 29,999/30,000 ms, coalesced second arrival, zero delay, restart, backward clock, drained attention, host outage, and due-warning bypass.
- [ ] Run focused tests to observe expected failure.
- [ ] Persist the first-window deadline from canonical publication time and expose it through the wake port. Reconstruct a monotonic wait after restart without resetting the already elapsed window. Keep the existing reservation and host guard paths intact.
- [ ] Run focused tests, `cargo fmt`, and per-change clippy outside a hold.

### Task 3: Warning open, quiet persistence, clear, reopen and active listing

**Files:** migration chosen after latest main, `src/store/schema.rs`, `src/store/receipts.rs`, `src/store/seats.rs`, `src/store/materialization.rs`, `src/store/effective.rs`, `src/store/queries.rs`, `src/protocol/commands.rs`, `src/protocol/results.rs`, `src/cli/commands.rs`, `src/protocol/output_compact.rs`, `TRUST-POLICY.md`, `tests/store/queries.rs`, `tests/store/wake.rs`, `tests/cli/commands.rs`.

**Interfaces:** Condition identity and open/clear sequence are persisted. A thread/recipient overdue backlog has one open condition while any warned pending receipt remains; invitation and unavailable conditions retain their canonical identity. `warnings active THREAD` is a paginated read.

- [ ] Write failing store tests for two overdue receipts in one thread causing one notification, first ACK with another pending causing no clear, last ACK causing one clear, retry causing no second clear, new overdue receipt reopening, restart preserving state, and a pre-migration unresolved warning remaining in `warnings active` without a synthetic clear. Include delayed materialization of both open and clear before any offer, with another participant receiving both in event order.
- [ ] Run the focused warning tests and confirm failures are behavioral.
- [ ] Add the condition table and transition helpers inside the existing deciding transactions. Reuse bounded recipient attribution work for open and clear messages; suppress extra warning fanout while keeping source markers and canonical actionability correct.
- [ ] Add the active-warning protocol/read/CLI with a high-water cursor and compact full-ID output. Keep `warnings --seat` as historical read-only behavior.
- [ ] Run focused tests, `cargo fmt`, and per-change clippy outside a hold.

### Task 4: Native measurements and review

**Files:** `docs/evidence/thread-overhead/2026-10-03-findings.md`, `docs/superpowers/specs/2026-10-03-thread-overhead-design.md`, CLI/help/skill documentation touched by the final behavior.

- [ ] Use `scripts/lib/isolated-herdr.sh` with a private named session and isolated HOME/state. Record baseline and new paths for one, five and twenty messages across threads: tool calls, command count, output bytes, approximate tokens, wake count and p50/p95 latency. Include first/persistent/clear/reopen warning behavior and restart.
- [ ] Stop every owned server/daemon/helper and run scoped leak checks. Preserve exact commands, time samples and output sizes in the evidence file.
- [ ] Run the required format, focused tests, all-feature clippy and default-feature guard. Request an exclusive coordinator full-suite slot; after GO run it with `HT_LEAK_RUN_ID` and scoped leak check.
- [ ] Obtain independent code review, resolve findings, freeze SHA and send concise coordinator checkpoint without `--wait`. Coordinator merges to main and reports landing verification.
