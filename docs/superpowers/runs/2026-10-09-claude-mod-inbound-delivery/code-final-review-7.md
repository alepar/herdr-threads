**Final review of ht-j16 (super-auto/claude-mod-inbound-delivery @ 52ea888e, fork point 90066db1)**

Inputs: the review package `.superpowers/sdd/ht-j16-plan/review-90066db1..52ea888e.diff` (88 commits, 207 files), the spec `docs/superpowers/runs/2026-10-09-claude-mod-inbound-delivery/2026-10-09-claude-mod-inbound-delivery-design.md`, and the ledger `.superpowers/sdd/ht-j16-plan/progress.md`.

The ledger is complete: merges 23 = 21 clean + 2 after a fix pass. It has no `parked`, `Recurring` or `BLOCKED-AUTH` lines.

**Step 1: my own findings, before reading the ledger**

1. **Important: the mod can submit while a main turn is open.**
   - Where: `integrations/claude/mod/hooks/register.js:382-409`, in `pump()`.
   - ht-j16.29 added `persistTurns(); await S.stateChain` between the `busy()`/hold checks and `io.submit`. After that wait the code only checks `S.disposed || gen !== S.gen`. It does not check `busy()` or `abortHoldSince` again.
   - So a `turn.start` (the user presses Enter) or an aborted `turn.complete` during the `$.state.set` write still leads to `$.prompt.submit` inside an open turn. That is the Esc-takeover race that D5 rule 1 and the "never called while any main turn is open" rule exist to prevent.
   - `stress.test.ts` cannot catch it, because its `state.set` stub finishes immediately (a microtask), so no event can land in that gap.
2. **Important (data loss): an id can be marked delivered and acked without ever being submitted.**
   - Where: `register.js:409` together with `:614`. The task reviewer of ht-j16.29 raised this as a minor (`unnamed-inference-window`), which under-grades it.
   - When the core is disposed during the pre-submit state write, the bail at 409 leaves `turns.submitting` in `$.state` even though no submit was issued.
   - The next core takes that record over (as `S.pred`). When the turn that was open at its load completes, rule (b) (`!S.pred.sawStart`) marks those ids delivered via submit and acks them.
   - The receipts then settle as `cooperative_mod_delivery`, and nothing re-streams them or wakes the seat natively. The agent never sees the message.
   - It shares a fix site with finding 1: on any bail after the wait, clear `submitting` and the in-flight flags.
3. **Spec gap (D4): thread and sender names are never sent.**
   - Where: `src/cli/watch.rs:699-701` always sets `thread_name: None` and `sender_name: None`, and the drain drops the `InboxBatchV2` `topic_data`.
   - D4 asks for "thread id and name, sender". The live evidence headers read `[herdr-threads] message mkgnGXIyF in txjwiIAcl from seHBtKi7k`.
   - The agent cannot tell which thread or which peer a message came from without an extra tool call, which works against the epic's goal.
4. **Known product defect: a stale attention prompt at every session start.**
   - Source: `docs/evidence/claude-mod-delivery/README.md`, under "Observations".
   - At each session start the mod submits `attention pending; run herdr-threads inbox`. The model runs `inbox` and finds it empty, which costs one unprompted model turn per start.
   - Likely cause: `watch` drains at connect, the SessionStart check-in then settles the notice it offered, and the queued attention item is never re-checked before it is submitted.
5. **Minor: the attention marker repeats.** `watch` emits `attention:<v>` on every new attention version while any invitation or notice stays pending, so each incoming batch gets another "run inbox" block. This matches the spec's wording but adds noise.
6. **Minor: other loose ends.**
   - The mod's `$.store` `delivered` set is never pruned, and `saveRec` deep-copies all of it on every change.
   - Items already queued by the mod but settled through another path are still delivered later.
   - `ModChannelRegistry::pass` never closes a channel because a recovery hold appears (low risk: holds only cover unowned targets).
   - The attention marker text is a bare `herdr-threads inbox` with no state-dir/endpoint selectors (the same as the native wake).

The rest of the seams look right:
- per-seat liveness across `/clear` (ht-ows is closed, and its test workaround was removed);
- a stalled channel does not suppress native wake;
- daemon-side ack decision and the resume rewrite against the current binding;
- `Replaced` exits 3, so two watchers cannot keep replacing each other;
- the mod-notify table observer;
- the transport's separate watch-connection slots;
- setup's lift/restore of the settings edit;
- the TRUST-POLICY A3/A4/A5/A8 rows and the accepted limits.

**Step 2: ledger triage**

- **Clusters.** The ledger has no `Recurring` lines, but two classes repeat:
  - No RED (failing-test-first) evidence (tasks 1, 2, 4, 10, 13, 15, 17). This is a process/report defect, not a code defect. Its real risk shows in task 12's `coincidental-red-tests`, where the new tests passed even before the change.
  - Stress floors tuned to one seed (tasks 19, 22, 23; the task-23 fix pass was needed because drops fell to 47 against a floor of 50). The stress gate is fragile.
- **Must-fix from the ledger:** Task 22 `unnamed-inference-window`, which is finding 2 above.
- **Already resolved at HEAD:**
  - Task 7 `unreachable-status-field` (fixed by ht-j16.22).
  - Task 7 `best-effort-guess` (verified against 2.1.295).
  - Task 8 `workaround-masks-bug` (the workaround was removed after ht-j16.17).
  - Task 9 live-path minors (superseded by the real live run).
  - Task 5 `unverified-integration`: notices do arrive as `Warning` items, so `InboxBatchV2` covers them.
- **Pre-existing, needs a follow-up bead:** Task 2 `unexplained-test-anomaly`. `cargo test --lib` stops after about 400 tests, which matches the `std::process::exit(0)` in `src/test_support/owner_watch.rs` (test-support only, not introduced by this branch). It could let tests pass without running their assertions.

---

**Verdict:** not ready

**Must fix before landing**
1. `register.js` `pump()`: after `await S.stateChain`, check `busy()` and `abortHoldSince` again. On any bail (disposed, session changed, now busy), clear `turns.submitting`, `submitInflight` and the items' in-flight flags. Add a stress or unit case where `state.set` is slow (a `turn.start` lands during the write).
2. The same site: a submit that was never issued must never be settled by rule (b) at `:614`. This is the loss path where a message is acked but the agent never sees it.
3. D4: fill `thread_name` and `sender_name` in `watch` lines (thread topic and sender display name), or have the human explicitly drop that requirement.
4. Stale attention prompt at session start: fix it (check attention against current state before submitting), or have the human accept it and record it as an accepted limit in TRUST-POLICY and the spec notes.

**Untested scope**
- There is no full-suite run of this branch yet; the caller's sweep comes after this review. Treat the branch as untested until then. That sweep must:
  - check the 5-minute budget against the fixed sleeps of up to 3.2 s and the 64-watch test in `tests/integration/mod_delivery.rs`;
  - run `scripts/check-no-leaked-processes --run-id`, which task 18 never ran.
- The mod's TypeScript tests (`delivery.test.ts`, `stress.test.ts`) are not in nextest or CI. They run only through `scripts/test-claude-mod`, and only when `claude` 2.1.287 or newer is installed.
- The live stress ran at `c6caf381`, before the D1–D3 fixes (ht-j16.28, .29, .30). None of those fixes was re-run live, and the D3 `REJECTED` fixture shape is unverified against 2.1.295.
  - Re-run `clear_rebind`, `reload_mid_turn` and `denied_tool_with_pending_context` live.
- Never exercised live: truncated bodies, the 120 s hold expiry, the draft hold, stall and cooldown, daemon restart, `/resume <id>`, `/branch`, subagent tool calls, and real Herdr.

**Deferred OK**
- Task 1: the plugin validate run that was skipped and the test-coverage counts (both are covered by later runs).
- Task 3: the read before the permit check, and the `too_many_arguments` lint allow.
- Task 6: context-path ack timing (the outer-mod accepted limit), the weak API probe (`goInert` handles a missing API on first use), the fixed seed, the untested `stream.return()` (the getppid poll is a backstop), and the unchanged `types/index.d.ts`.
- Task 5: the attribute-order style nit, and transport coverage only against a fake server (covered by ht-j16.10).
- Task 7: doctor `--fix` and the installer now also install the mod (owned and reversible; flag it to the human as a scope change).
- Task 4: the swallowed test summary.
- Task 2:
  - the unmeasured digest-per-channel cost of `pass()` (file a measurement bead);
  - the mod worker not showing in Health;
  - the trait placement;
  - the non-atomic register (fixed by the next worker pass).
- Task 8: the test that accepts exit 0 or 1.
- Task 12: an older daemon is retried rather than treated as unsupported.
- Task 10: default-impl drift that only affects test fakes.
- Task 14: the hook.rs scope deviation and the new `live` field.
- Task 16: the long doc line and stale fixture text.
- Task 17: the import placement.
- Task 20: the type-annotation edit.
- Tasks 19, 22, 23: the thin or seed-sensitive floors (fragility; noted under clusters).
- Task 21: the interleave and stale `endReason`.
- Task 22: the misleading test title and the dead `why` parameter.
- Task 23: the broad rejection heuristic (documented; it delays a submit by up to 120 s).
- Step-1 minors 5 and 6 above.
- The TDD-RED evidence cluster.
