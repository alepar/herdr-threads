**Verdict: not ready.** I found one real defect in the reload-during-submit fix. The live stress evidence also predates the fixes for the three scenarios that failed live. No full-suite measurement of this branch exists yet: the full-suite sweep was deferred to the caller and runs after this review, so the branch should not be treated as tested.

## STEP 1: my own review of the branch against the spec

**1. Important (confirmed by a repro). The reload-during-submit fix (ht-j16.29/.31) fails whenever the in-flight batch contains an `attention` item. The message is then submitted twice.**
- **Where:** `integrations/claude/mod/hooks/register.js`.
  - `pump()` puts attention items in the batch, so `S.turns.submitting.ids` holds e.g. `["attention:5","m1"]`.
  - `framedIds()` (line ~531) only matches `^\[herdr-threads\] (?:message|lazy) (\S+) in `.
  - `onTurnStart` therefore checks `S.pred.ids.every(id => framed.includes(id))`, which is never true. Rule (a) never fires and `pred.turnId` stays null.
  - Once `sawStart` is true, rule (b) in `onTurnComplete` (`!S.pred.sawStart || S.pred.turnId === e.turnId`) never fires either.
- **Effect:** `pred` keeps `busy()` true, so no idle submits happen until 120 s of idle with an empty box. `releasePred()` then submits `m1` again, although the first submit already put it in the transcript. The attention marker is repeated too.
- **What it breaks:** the D10 invariant "every streamed id delivered at most once". It also brings back live defect D2 for this batch shape, and it is not one of the windows TRUST-POLICY accepts.
- **Repro:** `/private/tmp/claude-501/-Users-alepar-AleCode-herdr-threads/6ac583b8-5b3a-4ce5-9809-728b586f23fc/scratchpad/modrepro/repro.mjs` (node, imports `createCore` directly). Output: `pred` survives the submitted turn's `turn.start` and `turn.complete`, then `submits total 2`, and the second submit holds `message m1` again.
- **Test blind spot:** `stress.test.ts` counts duplicates from the successor's `delivered` ledger entries only. A disposed predecessor's delivery is never ledgered, so this class cannot be detected. This is the same gap the live README describes for D2 ("Why the ledger alone misses it"). The modeled "turn seen" reload arm also fires only about 3 times in 600 schedules.
- **Fix:** include attention headers in `framedIds`, or compare only message/lazy ids. Add a delivery test with an attention item in the in-flight batch, and make the stress model count predecessor submits.

**2. Untested at the final SHA: the live stress ran before the fixes for its own failures.** `docs/evidence/claude-mod-delivery/README.md` records the run at `c6caf381`, where `clear_rebind` passed 0/3, `reload_mid_turn` 1/3 and `denied_tool_with_pending_context` 0/3 (defects D1–D3).
- The fixes (ht-j16.28, .29/.31 and .30, plus .32 and .33) are verified only against stubbed events.
- The D3 fix is a heuristic built on a `tool.call` result shape the authors could not confirm from the 2.1.295 binary. If Claude Code reports the permission-dialog Esc differently, D3 is not fixed.

**3. Minor**
- `rec.attentionVersions` is written but never read. The "once per attention version" dedupe actually comes from `runAttention` plus the watch's emitted set; this is dead state.
- At a fresh session start, before the mod registers, the SessionStart digest and a native wake are both presented, and then the mod delivers the same items. Live evidence shows 7 native prompts at session starts. TRUST-POLICY names the daemon-restart window but not this one; add it as an accepted limit.
- `watch` `drain()` re-pages every pending body from the cursor's start on every Attention frame. That includes large truncated bodies already emitted and still pending, so each frame costs work proportional to all pending bytes. This is an efficiency issue only.
- During rebind grace, `ack_mod_delivered` accepts an ack for the new generation before any new registration exists, because `is_live` returns true for `RebindGrace` at any generation. This is harmless but looser than D6 says.

## STEP 2: ledger triage

- **No `parked`, `Recurring` or `BLOCKED-AUTH` lines.** Metrics are consistent (completions 26 = merges 26).
- **There is an unflagged recurring class: "RED-phase TDD evidence not captured".** It appears in 9 tasks under different slugs: ht-j16.1, .2, .4, .10, .13, .15, .17, .24 and .25 (tdd-red-not-observed, tdd-not-followed, missing-red-*, red-step-skipped, and others).
  - It was not clustered because the ledger groups by exact signature slug and these slugs all differ. That is a pipeline defect.
  - The practical effect is that several regression tests are not shown to fail without their fix. Two instances are confirmed: Task 12 [coincidental-red-tests] and Task 26 [missing-isolating-test], where the `digest_notice_offer` notify entry is unproven.
  - It does not block landing by itself.
- **Minors re-checked against the final code, now resolved:**
  - Task 8 [workaround-masks-bug] (ht-ows): closed by ht-j16.17, and the workaround is gone from `clear_within_rebind_grace_*`.
  - Task 7 [unreachable-status-field]: `claude_version_gate` now fills `claude_version_supported`.
  - Task 7 [best-effort-guess]: `remote-settings.json` was verified (ht-j16.22).
  - Task 22 [unnamed-inference-window]: closed by the `issued` flag (ht-j16.31).
- **Minors that need a decision:**
  - Task 23 [broad-heuristic]/[unverified-engine-signal]: any denied or errored last tool call holds submits up to 120 s. This is documented, but it is the open D3 risk from item 2.
  - Task 8 [fixed-sleeps] and the slow 64-watch cap test put the 5-minute suite budget at risk. Check this in the sweep.

## Must fix before landing
1. The reload-during-submit duplicate when the batch holds an attention item (`register.js` `framedIds` / rule (a) / rule (b)), plus a delivery test and a stress model that counts predecessor submits.
2. Re-run the live stress (`tests/native/claude_mod/stress.py`) at the final SHA, at least `clear_rebind`, `reload_mid_turn` and `denied_tool_with_pending_context`. D1–D3 are claimed fixed but have no live evidence, and the D3 fix rests on an unverified engine signal.

## Untested scope
- **Full suite:** no `cargo nextest run --locked --all-targets --all-features` and no `scripts/check-no-leaked-processes --run-id` on this branch (Task 18 [leak-check-gap] too). The 5-minute budget is unmeasured after the integration tests added in ht-j16.10.
- **Lint checks:** clippy `-D warnings` and `scripts/check-default-features` were not run by me.
- **Mod JS tests:** `delivery.test.ts` and `stress.test.ts` are not part of nextest. They run only through `scripts/test-claude-mod`, which reports `skipped` and exits 0 when `claude` is missing. The caller's sweep must run it explicitly; Task 1 also skipped `plugin validate`.
- **Not exercised live (per the evidence README):** truncated bodies, the 120 s hold and draft timers, the 10-minute stall and cooldown, daemon restart, in-session `/resume <id>` and `/branch`, subagent tool calls, other permission modes, and a real Herdr (the run used a stand-in).
- **Not tested at all:** the non-atomic register path (Task 2 [non-atomic-register]), and liveness when the mod-channel worker itself fails (Task 2 [health-visibility-gap]: the worker does not report to Health).

## Deferred OK
- Task 3 [pre-auth-read] and [lint-allow].
- Task 5 [style-nit], [unverified-integration] (covered end to end by the ht-j16.10 integration tests) and [fake-only-coverage].
- Task 6 [context-delivery-timing] (the "outer mod could strip context" accepted limit), [api-probe-weak] (a missing API makes the mod go inert on first use), [seed-not-overridable], [untested-live-behavior] and [spec-file-skipped].
- Task 7 [scope-side-effect].
- Task 2 [unmeasured-cost] and [test-api-deviation].
- Task 8 [tolerant-assertion].
- Task 9 [unexercised-live-path], [missing-test-fixture] and [isolation-not-wired] (all superseded by the later live run), and [evidence-not-final-sha] (folded into Must fix 2).
- Task 10 [default-impl-drift] (affects test fakes only).
- Task 12 [transport-close-reads-retryable] (against an older daemon the mod retries every 30 s instead of stopping).
- Task 14 [scope-deviation] and [new-public-field].
- Task 16 [doc-line-wrap] and [stale-fixture-text].
- Task 17 [style-import-placement].
- Task 19 [thin-test-margin] and Task 22 [rng-sensitive-floor] (fixed seed; worth hardening alongside Must fix 1).
- Task 20 [unrequested-test-edit].
- Task 21 [async-interleave] and [stale-state] (`endReason` is not consumed when the session id is unchanged).
- Task 22 [test-title-mismatch] and [dead-code].
- Task 24 [brief-deviation] and [accepted-residual-window].
- Task 25 [spec-wording-drift]: update spec D4's wording.
- The RED-evidence cluster, apart from the two isolating-test instances named above.
