---
super-roast verdict: Should-fix (2 confirmed) [converged]
mode: pr        iteration: 2 of 3
profile (assumed): herdr-threads is a local, single-user Rust daemon plus CLI that relays messages between coding agents in Herdr panes, with a bundled Claude Code delivery mod (JS). The trust model is cooperative and same-user (AGENTS.md, TRUST-POLICY.md); the store is local SQLite behind a Unix socket, with no network exposure, no external users and no money or regulated data. Rollback is a binary swap plus `setup`/`unsetup`. Blast radius is low: a mod delivery outage degrades to the native wake path and is recovered by a plugin reload or daemon restart.
inputs: super-auto/claude-mod-inbound-delivery@bd8e6661 vs main merge-base 08cea709
delta vs prior: 2 new confirmed (0 Blocking) · 0 carried (0 Blocking) · 4 resolved · 0 regressed (0 Blocking) · 4 punch-listed (open)
coverage: scouts 12/12 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, regression, api-contract, observability, testing, hygiene-docs, deploy-safety) · raw 5 → deduped 4 → panel 3 · spot 1 · promoted 0 · judge completion 100% · remainder-capped: 0
independence: same-family (Claude) — seat-differentiated panel · rung: Workflow
seat-agreement: panels 3 · rr 1.00 · rg 0.67 · fg 0.67 · unanimous 0.67 · ground-loo 0.67 (n=3) · reproduce 2/1/0 · refute 2/1/0 · ground 3/0/0
lane-yield (found/confirmed/unique/refuted): correctness 1/1/1/0 · security 0/0/0/0 · premortem 0/0/0/0 · simplicity-design 0/0/0/0 · hot-path-perf 0/0/0/0 · concurrency-async 0/0/0/0 · regression 2/1/0/1 · api-contract 1/1/0/0 · observability 0/0/0/0 · testing 1/0/0/0 · hygiene-docs 0/0/0/0 · deploy-safety 0/0/0/0

prior-report tracking (iteration 1 confirmed findings):
- resolved: [Should-fix] integrations/claude/mod/hooks/register.js:228; src/cli/watch.rs:719 (ack batches over 100 ids) — fixed by ht-j16.18 (0571772c); not re-surfaced by any current packet.
- resolved: [Should-fix] src/cli/watch.rs:340 (transient Capabilities failure reported as `unsupported`) — fixed by ht-j16.19 (42dbc875); not re-surfaced.
- resolved: [Nit] src/service/mod_channels.rs:288 (`pass()` closes a newer registration by seat only) — fixed by ht-j16.17 (d9e3a7d7, identity-checked registry closes); not re-surfaced.
- resolved: [Nit] src/cli/setup.rs:1369; src/cli/setup.rs:1401 (lifted mod entry not restored on later errors) — fixed by ht-j16.22 (fc3ce6f9); the current regression packet on that commit concerns a different, deliberate non-error path and is rejected below.
- punch-listed (open): [Nit] src/service/mod_channels.rs:240 (sweep stall close by seat only).
- punch-listed (open): [Nit] src/service/mod_channels.rs:193; src/daemon/settings.rs:20 (`set_mod_delivery` has no production caller).
- punch-listed (open): [Nit] src/cli/setup.rs:1708 (interactive `[Y/n]` inside `execute()`).
- punch-listed (open): [Nit] scripts/test-claude-mod:13; scripts/test-claude-mod:1 (JS mod tests unenforced).

## Confirmed findings
- [Should-fix] src/protocol/watch.rs:86 — The truncation marker the watch child appends to a cut body tells the agent to run bare `herdr-threads body <id>, then herdr-threads ack <id>` with no `--state-dir`/`--host-endpoint`. On an instance the pane's auto-detection does not reach, the configuration the ht-j16.20 fix exists to support, both commands go to the wrong daemon (or none), and the truncated receipt has no working settle path. [lanes: correctness] (new this iteration) [fix-regression]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: src/protocol/watch.rs:86-89 `truncation_marker` returns a fixed string with no instance selectors; src/cli/watch.rs:674 appends it to every body over WATCH_BODY_LIMIT_BYTES. The module doc (watch.rs:30-36) makes the marker the only settle route: the mod never acks a truncated item. The hook builds every other agent-facing command through `cli_prefix(&pane_selectors(...))` (src/cli/hook.rs:952-990), which adds selectors whenever pane auto-detection does not reach the instance; the marker has no equivalent. ht-j16.20 (c04aae82) launches `watch` with explicit `--state-dir`/`--host-endpoint` for exactly this reason, and its test (tests/integration/mod_delivery.rs:1099) uses a non-default state dir and endpoint. The marker test (tests/integration/mod_delivery.rs ~1255-1300) runs the parsed steps through `rig.cli`, which always prepends the selectors, so it hides the gap. A grep of src for `herdr-threads ack`/`herdr-threads body` finds no other mod-side settle instruction. All three seats confirmed at Should-fix; the effect is limited to bodies over 8 KiB on non-auto-detected instances, and a text `inbox` run with selectors would still settle the receipt, so not Blocking.
  fix-shape hint: have the watch child render the selectors it was launched with (the `cli_prefix` equivalent) into the marker, and make the marker test run the marker's steps verbatim without `rig.cli`'s implicit prefix.

- [Nit] src/protocol/watch.rs:86 — The ht-j16.23 truncation marker tells the agent to run `herdr-threads body {id}, then herdr-threads ack {id}` on truncated lazy rows too, but lazy rows have no receipt so the daemon rejects the `ack` with InvalidRequest, leaving the agent with a failing instruction and the row pending. [lanes: regression, api-contract] [fix-regression] (new this iteration)
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: src/protocol/watch.rs:84-88 `truncation_marker(id)` takes no lazy or ack_required input. src/cli/watch.rs ~666-697 `emit_chunk` appends the marker before branching on `if chunk.lazy { WatchItem::Lazy }`, so a lazy row over 8 KiB carries the `ack` instruction. Lazy audiences go only into `lazy_recipients` (src/store/lazy_delivery.rs:237); `prepared_recipients` is filled only on the ordinary path (src/store/messages.rs:587), so `effective_receipt` (src/store/effective.rs:1714) returns None and `ack_impl` (src/store/receipts.rs ~301-314) returns InvalidRequest "ACK ID is not an addressed ordinary message", rejecting the whole batch. The same commit's module doc (src/protocol/watch.rs:33-36) says "A truncated `lazy` row has no receipt; it stays pending until a text `inbox` shows it", contradicting the marker. Before 40da4f37 the marker said only `body {id}` (read-only), so the failing instruction is new to this branch. No test covers a truncated lazy row. Seats: reproduce Nit, refute Nit, ground Should-fix.
  demoted: two of three seats rate it Nit; the profile states a cooperative single-user tool where the lazy row stays pending exactly as the module doc already accepts and the mod never acks truncated items, so the cost is one failing command with no wrong state. Nit here.
  fix-shape hint: give lazy rows their own marker (body only, or naming `inbox`) by passing `chunk.lazy` into `truncation_marker`, and add a truncated-lazy-row test.

## Not verified (beyond panel cap)
- none

## Not verified (dedupe failed or judge lost)
- none

## Beyond remainder cap (count only)
- none

## Rejected (with reason)
- src/harness/claude_mod.rs:301 — The ht-j16.22 commit also adds a `ManagedPolicy::Unverifiable` verdict, so a managed-settings source that cannot be read or is not a JSON object now blocks the mod install where before it was skipped, and on a `setup claude` that lifts the mod for a hook upgrade the previously working mod entry stays removed. [lanes: regression] [fix-regression] Rejected 2-1: the mechanism is accurate but the behavior is a documented, deliberate fail-closed choice. integrations/claude/README.md "Managed policy" states "A policy file or drop-in directory setup cannot read, or a file that is not a JSON object, is treated as unsafe: the write is skipped and `managed_policy.state` is `unverifiable`"; the enum doc (claude_mod.rs ~:392) says setup "cannot rule out disableSideloadFlags, so it is treated as set". A real `DisableSideloadFlags` policy already produces the identical lift-then-skip outcome on a hook upgrade, so `Unverifiable` adds no new failure path. `SkippedManagedPolicy` is an Ok result, not an error, so the restore guard correctly does not fire; if the unreadable source did set the flag, leaving the entry removed is the safe result. Hooks stay installed and the README documents the hooks-plus-native-wake fallback; setup warns and names the source. Trigger is narrow (root-only or non-object managed file). Drop-in handling is specified in the README and tested (tests/harness/claude_mod.rs:485-525). Dissent noted: the ground seat confirmed at Nit on the same facts, calling it a deliberate choice with a low-probability trigger and noting the extra scope bundled into ht-j16.22; it cites nothing the majority's evidence does not address, so the rejection stands. Related prior finding (src/cli/setup.rs:1369; :1401, restore on error exits) is resolved by this same commit.

## Unverified nits (spot-checked)
- [Nit] integrations/claude/mod/hooks/register.js:592 — No test runs the JS side of the ht-j16.20 launch fix: nothing loads a `register.js` with a rendered, non-null `LAUNCH` line and checks the argv the mod spawns, so the `buildIo` read `argv: Array.isArray(LAUNCH?.argv) && LAUNCH.argv.length ? LAUNCH.argv : null` is the only link between what `setup claude` writes and what the mod runs, and breaking it leaves every test green. (spot: CONFIRM Nit — Rust tests inspect only the rendered text; tests/integration/mod_delivery.rs ~1098-1128 parses the JSON and runs the argv in Rust; delivery.test.ts injects argv straight into `createCore`, and its only `register()` smoke test (:763) uses the embedded `LAUNCH = null`. Glue is correct today; a `register()` smoke test with a rendered LAUNCH line asserting the spawned `[...prefix, 'watch', ...]` closes it.)

## Escalations (need human)
- none
---