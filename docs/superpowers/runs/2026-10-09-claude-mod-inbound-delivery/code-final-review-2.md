**Review: ht-j16 (Claude Code mod inbound delivery), branch `super-auto/claude-mod-inbound-delivery` at 8fd6731d, fork point 08cea709**

- Review package: `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto-claude-mod-inbound-delivery/.superpowers/sdd/ht-j16-plan/review-08cea709..8fd6731d.diff` (34 commits, 113 files)
- Spec: `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto-claude-mod-inbound-delivery/docs/superpowers/runs/2026-10-09-claude-mod-inbound-delivery/2026-10-09-claude-mod-inbound-delivery-design.md`

**The branch and its worktree are out of step.** The branch ref is still at 8fd6731d. The integration worktree is detached at cb56e49f, which is 8fd6731d plus two docs-only commits (the PR roast round 1 record and the fix-bead record) that are not on the branch. The source code is the same in both. Because of the detached state, the second launch's ledger shows the merge of ht-j16.19 as "held". The fix pass for ht-j16.21 reports FIXED (commits 956d135..8b1330a) but has not been merged. None of the fix beads ht-j16.17 to ht-j16.22 has landed on the branch.

**Bead state:**
- ht-j16.1 to .7 and ht-j16.10 are merged and closed.
- Still open: ht-j16.8 (fix-loop gate), ht-j16.9 (live race stress) and fix beads ht-j16.17 to .22.
- An earlier final review (`code-final-review-1.md`) and PR roast round 1 already exist. Their findings became beads .17 to .22.

## Step 1: my own findings from the diff and the spec

**N1 (Important, new; no earlier review raised it). The truncation marker sends the agent to a command that cannot settle the receipt.**
- `watch` cuts bodies over 8 KiB and appends `…truncated; run herdr-threads body <id>` (`src/protocol/watch.rs:81`).
- The mod never acks truncated items, and the daemon refuses them as `refused_terminal/truncated`.
- `body` is a `Message` query, so it is read-only. TRUST-POLICY says the same: "bodies … remain read-only".
- So an agent that follows the marker reads the text, and the receipt stays pending.
- While the channel is live, nothing reminds the agent afterwards:
  - native wake and poke are suppressed;
  - the PreToolUse and SessionStart digest is suppressed;
  - the stall predicate excludes truncated receipts on purpose;
  - `deadline_millis` is optional, so a send with no deadline never produces a warning.
- Result: the receipt stays pending, and the sender sees it unacked, until a deadline warning fires or the channel drops.
- The integration test `truncated_body_streamed_with_marker_and_ack_refused_terminal` settles the receipt only with an explicit `ack`, which the marker never mentions.
- The spec (D4 and the decision record) and TRUST-POLICY both claim that truncated items "settle through `body`, `inbox` or `ack`". That claim is false for `body`.

**Earlier findings I confirm independently (already beads):**
- **/clear race, ht-ows → ht-j16.17.** After a /clear check-in, the wake path asks `is_live` for the new generation while the registry entry still holds the old one. In that window the wake lane can prompt the pane being cleared. The integration test hides this with a "working" pane.
- **Registry close races → ht-j16.17.** `pass()` and `sweep()` close a channel by seat only, so they can close a newer registration.
- **Notices → ht-j16.21.** Notices are not delivered while a channel is live: the fingerprint does not track them, the drain never emits them, and the tool-boundary hook returns early.
- **Wrong binary or instance → ht-j16.20.** The mod runs bare `herdr-threads` from PATH, with no `--state-dir` or `--host-endpoint`.
- **Acks over 100 ids → ht-j16.18.** An ack batch over 100 ids is rejected whole on every retry and never settles.
- **Probe failure stops watch → ht-j16.19.** A failed capability probe is reported as exit 3, which stops `watch` for the rest of the session.
- **Setup → ht-j16.22.** The lifted mod settings entry is not restored on every error path. The managed-settings cache name and the drop-in directory need handling, and `claude_version_supported` is always null.

**Other seam observations (minor):**
- The `mod_delivery` setting is read only at daemon boot; `set_mod_delivery` has no production caller.
- `thread_name` and `sender_name` are always `None`, so the mod shows raw ids.
- `rec.attentionVersions` is recorded but never read.
- Two sets grow without bound for the life of a session: the mod's `rec.delivered` (in `$.store`) and the `watch` process's `emitted` set.
- After a daemon restart, the wake lane's first pass runs before the mod re-registers. A seat can then get a native marker as well as the mod delivery. This is at-least-once delivery, consistent with the spec, but not listed as an accepted limit.
- The spec's Post-Implementation Notes section is empty. It should record the divergences above: the table-based commit observer instead of per-call-site notify, the boot-only setting, and `replaced` exiting 3.

## Step 2: ledger triage

- The ledger has no `Recurring minor:`, `Recurring blocker:`, `parked` or `BLOCKED-AUTH` lines.
- **Unflagged class, process:** no RED-first TDD evidence in tasks 1, 2 and 4. This is a pipeline or implementer habit, not a code defect.
- **Unflagged class, coverage:** coverage stops at fakes and in-process stand-ins (T5, T6, T8). Nothing runs the real mod in real Claude against a daemon set up by `setup claude`.

Minors that were graded too low and must be fixed:
- **T8 [workaround-masks-bug]:** this names a known product defect (ht-ows), so it must be fixed. ht-j16.17 tracks the fix and the removal of the test workaround.
- **T7 [unreachable-status-field]:** the D8 requirement to report the Claude version gate is not met. ht-j16.22 tracks it.
- **T7 [best-effort-guess]:** if the managed-settings cache name is wrong, setup can write `CLAUDE_CODE_PLUGIN_DIRS` under a server policy that forbids it, and Claude Code then refuses to start. ht-j16.22 tracks this, including the drop-in directory.
- **T1 [skipped-optional-check], `claude plugin validate` skipped:** I have now run `scripts/test-claude-mod` on this head with Claude 2.1.295 and an isolated config. `validate --strict` passes and 39/39 mod tests pass, including the stress test. The run is resolved; the enforcement gap is not, because the script still skips with exit 0 and nothing in CI or nextest calls it.
- **T2 [unexplained-test-anomaly] and T4 [test-summary-swallowed]:** a lib test binary exits 0 without printing a summary, so tests may be silently not running. Before landing, confirm under nextest that the count of `--lib` and `cli::hook` tests that actually ran matches `cargo nextest list`.

## Verdict

Not ready.

## Must fix before landing

1. Merge the six open fix beads (ht-j16.17 to .22) into the branch. First re-attach the integration worktree to `super-auto/claude-mod-inbound-delivery` (fast-forward the branch to cb56e49f), because the detached state is blocking every fix merge. The ht-j16.21 fix is already FIXED but not merged.
2. **ht-j16.17:** per-seat liveness for wake candidates (fixes ht-ows), identity-checked registry closes, and removal of the "working" pane workaround in `clear_within_rebind_grace_emits_no_session_start_digest_and_no_native_kick`.
3. **ht-j16.20:** the mod runs the hooks' exact argv (absolute executable, `--state-dir`, `--host-endpoint`), with an integration case that uses no extra flags.
4. **ht-j16.21:** notices are delivered while a channel is live.
5. **ht-j16.18:** chunk acks to at most 100 ids per call, in both the mod and `watch ack`.
6. **ht-j16.19:** a transient probe failure exits 2 (retry), not 3 (stop).
7. **ht-j16.22:** restore the mod settings entry on every error path, honour `managed-settings.d/*.json`, record the verified cache name, and wire the version gate.
8. **N1 (new, needs a bead):** change the truncation marker so the agent can settle the receipt, for example `…truncated; run herdr-threads body <id>, then herdr-threads ack <id>`, or direct it to `inbox`. Correct the D4 and decision-record text and the TRUST-POLICY line that says truncated items settle through `body`. Add an integration assertion that follows the marker's instruction and ends with the receipt acked.
9. Verify that no lib tests are silently skipped (T2/T4) during the nextest sweep.

## Untested scope

- **Full suite:** there is no full-suite measurement of this branch. The sweep is deferred to the caller, so the branch has not been run against the whole suite, the 5-minute budget or `scripts/check-no-leaked-processes`. The mod-delivery integration group adds fixed sleeps of up to 3.2 s, a 64-watch admission test and many `watch` children.
- **ht-j16.9:** live race stress in real Claude TUI sessions is not started. None of the D10 live scenarios have evidence: Esc takeover, queued prompt, Stop-hook continuation, permission dialog, `/clear`, `/resume`, reload mid-turn, UserPromptSubmit drop.
- **Live engine behaviour:**
  - whether `stream.return()` actually stops the watch child;
  - the ordering of `session.end`, the SessionStart hook and the registry's Close;
  - whether `$.state` survives a real reload;
  - the daemon-restart window in which a native wake and the mod can both deliver.
- **Mod JS tests:** they pass now (my run above), but nothing in CI or nextest enforces them, and the script exits 0 when `claude` is missing.
- **Fix beads:** none of the six fixes has been tested on the branch, because none is merged.
- The ledger has no BLOCKED-AUTH lines, so no coverage was lost to permission refusals.

## Deferred OK

- **T1:** missing TDD evidence and the test-count check.
- **T2:** unmeasured digest cost per pass (measure it during the sweep), non-atomic register (heals on the next pass), the ModStoreReads trait location, and TDD order. Health visibility of the mod worker: file a follow-up bead, since a dead worker would never sweep grace or stall.
- **T3:** pre-auth read (cooperative model) and the lint allow.
- **T4:** missing RED log.
- **T5:** style nit. The drain and transport coverage is now end to end through ht-j16.10, except notices, which ht-j16.21 covers.
- **T6:** context-delivery timing is the accepted limit "outer mod could strip context". Also deferred: the weak API probe (it fails into inert mode plus grace), the constant seed, and the unchanged d.ts.
- **T7 [scope-side-effect]:** `doctor --fix` and the installer now install the mod. Acceptable once ht-j16.22 lands.
- **T8:** fixed sleeps and the tolerant exit assertion. Revisit if the sweep breaks the speed budget.
- **Seam observations:** the boot-only `mod_delivery` setting, raw ids instead of thread and sender names, the unused `attentionVersions`, and the unbounded `rec.delivered` and `emitted` sets. Record the divergences in the spec's Post-Implementation Notes.