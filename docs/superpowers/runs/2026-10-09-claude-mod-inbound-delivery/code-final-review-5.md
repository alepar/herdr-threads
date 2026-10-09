**Verdict: not ready.** One spec and TRUST-POLICY invariant is broken in the mod, and the policy's stated reason for accepting marker spoofing is factually wrong. No full-suite measurement exists for this branch yet, and the mod has never run in a real Claude Code session.

## Step 1: my own review against the spec (ht-j16 design doc)

What I read: the 68-commit review package `.superpowers/sdd/ht-j16-plan/review-25db37f3..e21f12cb.diff`, plus the full sources of the mod registry, watch transport, watch CLI, the ack and store paths, the scheduler routing, the hook changes, the mod (`register.js`), setup, and the TRUST-POLICY diff. I also checked the mod's engine calls against the Claude Code 2.1.294 type definitions (`claude-code.d.ts`). The calls match: `process.run` returns `exitCode`, spawn returns `{code, signal}` and `return()` kills the child, and `PromptBox.text`, submit `drop` and append `deny` all match.

Things that compose correctly across tasks:
- Liveness is checked per seat for wake, poke and hook digest. The ht-j16.17 fix is in, and the clear/rebind-grace integration test no longer needs its "pane working" workaround.
- The ack is decided against the canonical binding, and truncation is judged from the stored body length.
- Chunked bodies keep `ack_required` on the final chunk.
- The watch connection sits outside the ordinary connection limit.
- Setup lifts the mod entry and restores it around the hook rewrite.

**F1: the mod keeps delivering stale queued items after its channel is gone (cross-task seam).**
- In `integrations/claude/mod/hooks/register.js`, `S.queue` is cleared only on `/clear`. It survives `onChildExit`, `startChild`, `Close{stalled|retired|disabled|replaced}` and cooldown refusals.
- `onToolCall` and `pump()` never check whether a channel is connected.
- The designed stall path (a long post-abort hold with the user away) plays out like this:
  1. The daemon closes the channel as `stalled` and the native ladder takes over.
  2. The agent runs `inbox` and settles the messages in a turn that completes normally.
  3. During that same turn, `onToolCall` attaches the same held batch as context. If not, the cleared hold makes `pump()` submit it as a new turn.
- This breaks TRUST-POLICY A4 ("the native ladder … is the only delivery path during the cooldown") and D7's rejected alternative ("two paths could deliver the same item").
- The same thing happens when a reconnect grace expires, and after `disabled` or `replaced`.
- Fix: drop queued items that are not in flight when the watch exits or closes (or on each `connected`), because the next watch re-streams whatever is still pending. Alternatively, deliver only while a watch run is connected.

**F2: the marker-spoofing accepted limit is mis-stated in TRUST-POLICY.**
- The new limit says the mod's raw framing makes markers "as trustworthy as the peers, as on every other read path".
- That is false. `src/protocol/output_compact.rs` has an explicit invariant that body lines are indented two spaces, so a body can never start at column 0 and fake a row.
- The mod's `frame()` writes bodies unindented. A peer body can forge a `[herdr-threads] message … [human]:` block.
- This came from a background security-review escalation, which was parked (commit 31774f63) with no follow-up bead.

**F3: spec divergences that are documented and acceptable.**
- `mod_delivery` is read only at daemon boot (bead ht-h2u is open).
- Notices are not delivered over the channel, so an idle seat with a live channel sees new notices only at its next check-in.
- `replaced` exits 3.
- The resume re-ack branch for a previous generation is nearly dead code: `watch ack` always claims the current context's generation, so the decision really hinges on the native session. That is harmless.

**F4: minor.**
- The mod's per-session `rec.delivered` in `$.store` and the watch process's `emitted` set are never pruned, and the whole record is re-serialized on every save.
- Each new attention version re-emits the generic attention marker while an invitation or warning stays pending.

**F5: bookkeeping.** Bead ht-ows is still open although the design notes say ht-j16.17 fixed it.

## Step 2: ledger triage

- There are no `Recurring`, `parked` or `BLOCKED-AUTH` lines, and the metrics are consistent.
- **Unlabelled cluster: no RED-first TDD evidence.** It appears in tasks 1, 2, 4, 10, 13, 15 and 17, which is at or above the cluster threshold.
  - The class is a pipeline defect: implementer runs don't capture a failing run before the fix.
  - The consequence: the regression tests for fix beads ht-j16.17, .20, .22 and .24 are not shown to fail on the old code. The ht-j16.19 tests are confirmed to have passed before that change.
  - This is a process finding to report upstream, not a code blocker.
- **Already resolved by later tasks:**
  - [unreachable-status-field] and [best-effort-guess] remote-settings.json, both fixed by ht-j16.22.
  - [workaround-masks-bug] in the rebind-grace test, fixed by ht-j16.17; the workaround is gone.
  - [unverified-integration] for the InboxBatchV2-only drain, covered end to end by `tests/integration/mod_delivery.rs`.
- **[unexplained-test-anomaly] and [test-summary-swallowed]:** under `cargo test --lib` the run summary is lost to a test that redirects stdio, but the exit status still reflects pass or fail. Nextest isolates each test, so this does not mask failures. Deferrable.
- **Task 6 [context-delivery-timing]:** covered by the accepted limit "an outer mod could strip context". OK.
- **Task 9 live-path minors:** these are untested scope, listed below.

## Must fix before landing
1. **F1:** stop the mod delivering queued items after its watch run ends. Add a mod unit test: stall close, then cooldown, then a non-aborted turn must produce no context attach or submit.
2. **F2:** needs your decision.
   - Option A (cheap): indent the body lines in `frame()`, matching the `output_compact` invariant.
   - Option B: correct the TRUST-POLICY accepted-limit wording and file a follow-up bead.
   - Either way the current "as on every other read path" claim can't land.
3. **Full suite:** run the deferred full-suite sweep with leak checks; there is no measurement of this branch yet. This branch adds a roughly 2.2k-line integration suite with fixed sleeps of up to 3.2 s and a 64-channel test, on top of a base that had just been trimmed to fit the 5-minute budget.
4. **Live run:** decide whether to land with no live evidence (see below). `setup claude`, `doctor --fix` and the installer now write `CLAUDE_CODE_PLUGIN_DIRS` into users' Claude settings by default. A mod that registers but fails to deliver holds native wake off for up to 10 minutes per stall cycle.

## Untested scope
- **Real sessions:** the mod has never run inside a real Claude Code session. All 14 live scenarios were not run because the copied profile was not signed in, so `stress.py`'s live path is unexercised. Its marker-file hooks are never installed and it is not wired to an isolated Herdr session.
- **Unit level only:** the real `$.prompt.submit`, `$.session.append`, context attachment, turn and abort event order, `stream.return()` child kill and module-reload behaviour are covered only by stubs and `claude plugin test`.
- **Evidence SHA:** the recorded evidence (`docs/evidence/claude-mod-delivery/`) is at 13d795f9, not the final SHA e21f12cb.
- **Not run:** the full nextest suite, the 5-minute budget measurement, and `scripts/check-no-leaked-processes`. Task 18 also skipped the leak check.

## Deferred OK
- **Task 3:** [pre-auth-read] (cooperative model) and [lint-allow].
- **Task 6:** [api-probe-weak] (recovers on its own), [seed-not-overridable] and [spec-file-skipped].
- **Task 7:** [scope-side-effect] (`doctor --fix` installs the mod; note it in release notes).
- **Task 2:** [unmeasured-cost] of the per-pass digest (bounded at 64 seats), [health-visibility-gap], [test-api-deviation] and [non-atomic-register] (the next worker pass corrects it).
- **Task 8:** [tolerant-assertion].
- **Task 12:** [transport-close-reads-retryable].
- **Task 10:** [default-impl-drift] (test fakes only).
- **Task 14:** [scope-deviation] and [new-public-field].
- **Task 16:** [doc-line-wrap] and [stale-fixture-text].
- **Task 17:** [style-import-placement].
- **Task 5:** [style-nit].
- **Tasks 5 and 8:** [fake-only-coverage] and [fixed-sleeps]; the latter is subject to the budget measurement in item 3 above.
- **Cleanups:** F4 pruning, and closing ht-ows.

Ledger: /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto-claude-mod-inbound-delivery/.superpowers/sdd/ht-j16-plan/progress.md

Key files:
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto-claude-mod-inbound-delivery/integrations/claude/mod/hooks/register.js
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto-claude-mod-inbound-delivery/TRUST-POLICY.md
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto-claude-mod-inbound-delivery/src/protocol/output_compact.rs