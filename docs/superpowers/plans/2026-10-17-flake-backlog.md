# Flake backlog implementation plan

**Goal:** Resolve ht-7ykh, ht-2si, ht-86a, ht-81q and the ten ht-fmww candidate families with controlled evidence.

**Architecture:** Fix missing fixture prerequisites at their causal boundaries. Change production behavior only after demonstrating a product defect. Preserve existing assertions and real latency contracts.

**Tech stack:** Rust, cargo-nextest, Python unittest, isolated test daemons.

**Spec:** Handoff mQ0VTXsa78oj; `docs/research/2026-10-16-flaky-tests/report.md`.

## Constraints

- Work on `fix/flake-backlog` from main 92edebf67 in `.worktrees/flake-backlog`, seeded from `.worktrees/herdr-095/target`.
- Leave ht-e01y to its owner. Coordinate wake driver changes with ht-5zxj.
- No assertion deletion, blind retries, or timeout increases substituting for synchronization.
- No shared Herdr server changes, real harness config writes, push or stash. Stop all owned processes.
- Include `Outside-adapter:` trailers in commits.

## Independent investigations

- [x] ht-2si: reproduce `lazy_display_json_machine_explicit_selector_readonly`; repair its invalid caller fixture or explicitly verify early refusal, preserving read-only assertions. Files: `tests/cli/lazy_display.rs`.
- [x] ht-86a: inventory staging calls, force budget exhaustion across target chunks, repair fixture setup using progress, retain intentional partial/negative staging tests. Files: `src/test_support/`, snapshot fixtures in `tests/`.
- [x] ht-7ykh: force a watch Pull across binding replacement; compare current terminal close behavior with the historical report. File: `tests/integration/mod_delivery.rs`.
- [x] ht-81q: preserve panic diagnostics through setup and in CI; attempt controlled all-lanes reproductions without changing the 100ms oracle. Files: `tests/integration/sweep_remaining.rs`, `.github/workflows/`.
- [x] ht-fmww Rust: force wake-before-mark, cancel-before-measurement, no-bytes-after-accept, and startup-before-own-history schedules. Files: `tests/integration/agy_sweep.rs`, `tests/host_adapter.rs`, `tests/store/sync_contacts.rs`, `tests/sync/incarnations_worker.rs`.
- [x] ht-fmww Hermes: force parent delays beyond helper lifetimes and real-clock cadence; replace confirmed timing assumptions with explicit handshakes. Files: `src/harness/hermes/{assets,runtime}.rs`, `integrations/hermes/test_bridge.py`.
- [x] ht-fmww live gates: force fake response completion before publication and validate busy-tool lifetime across sends with offline self-tests. Files: `scripts/agy-gate/`, `scripts/agy-fake-gemini/`, `scripts/wake-e2e.py`, `scripts/wake_e2e_selftest.py`.

## Integration and evidence

- [x] Review controlled before/after evidence for every bead and each static family; run focused repeats under owned load where appropriate.
- [x] Run fmt, both clippy variants, check-default-features and check-harness-neutral.
- [x] Run the full suite once with `HT_LEAK_RUN_ID`; run scoped process/temp leak checks and clean any owned leaks.
- [x] Append exact mechanisms, file references, commands and results to the research report; close only beads supported by evidence.
- [x] Commit changes with path trailers and report per-bead results, branch and commit to sQ0VVjdFD4f9.

## Final qualification decision

The full run passed 7617/7618 in 310.244s; its sole failure was a stale allowlist exemption for the removed Hermes 10s timer. The exemption is deleted and all three inventory tests pass; both clippy variants and remaining required checks pass on final source. Scoped process/temp cleanup is clean. The coordinator directed no second full run here; green full-suite and 300s-budget qualification belongs to the merged 2.0.2 candidate. Detailed evidence and limits are in the research report.
