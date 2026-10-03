# Fix-loop round 1: super-code final review

The `review` field of the super-code return for the fix-loop re-entry (workflow wf_a0cdda7f-a67), verbatim. run.md's `codeBuckets.review` summarizes it.

**Verdict: not ready.** Nothing found is a hard defect, but two things block landing: one fix on the hook's critical path, and no full-suite run of this branch exists yet.

## Step 1: my own review against the spec

The branch covers what the spec asks for:
- contract declarations, `contract_id` (pinned by a test), `contract-id` and `harness-version normalize`
- `--event` registration, with legacy registrations handled and flagged by `doctor`
- the evidence table (migration 0012), transcript attribution, the per-session gate and the heartbeat
- holding an unattributed SessionStart until the session's first attributed event
- manifest schema 2: reader, embedded copy, cache, fetch policy, opt-out, size cap
- the state function, implemented in the spec's order
- Health and `doctor` rendering, the canary manifest writer, retention, the ruleset check, publishing, and the release embed
- the end-to-end stand-in scenarios

The canary Python suite passes here (110 tests). Findings, most serious first:

1. **Important: the evidence step sits on the hook's critical path.** In `src/cli/hook.rs` (around lines 2096–2108), `hook_evidence::report` runs synchronously before `observe_harness_in`.
   - It runs under the 1.5 s tool budget and the watchdog, and can use up to `CALL_CAP` = 500 ms.
   - SessionStart always sends, and its budget is only raised to 5 s after `parse_event`.
   - So a slow daemon (store writer contention, the capabilities round trip, the IMMEDIATE transaction) cuts the `--version` probe's budget from about 1350 ms to about 850 ms.
   - If evidence plus the probe exceed the watchdog, the hook exits 0 with "budget expired". That is a missed check-in that was not possible before.
   - The spec says the hook "never blocks on it".

2. **Important but latent: "working" in `doctor` while the hook refuses to run.** A version inside a recipe `known_broken` range that was verified here derives "working" (`src/harness/state.rs`, step 3).
   - The hook still fails closed on B6's ladder, and Health now renders `VersionRefused` as nothing.
   - Result: Health is silent, `doctor` says working, and herdr-threads does nothing for that harness.
   - It is latent because both recipe `known_broken` lists are empty today.

3. **Risk (untested): Codex resume misattribution.** Codex attribution depends on Codex sending SessionStart with `source: resume`, under the same `session_id`, to the hook. That was never captured (see Untested scope). If it doesn't happen, a resumed session after an upgrade files the new CLI's violations under the creator's version. Because a local violation outranks local verification, an old, working version would then show as broken.

4. **Minor: the gate's single per-session `ok` can be used up by the wrong event.** In `src/cli/hook_evidence.rs` (`should_send` / `after_send`), any `ok` other than SessionStart sets `ok_sent_at_ms`. That includes Codex `SubagentStart`, which is in the Other class. The session's PreToolUse `ok` is then held back until the hourly heartbeat. This only delays verification, and "new" is silent. Fix: let only Tool-class events use the slot.

5. **Minor: the manifest can go stale or miss fetches.**
   - `ManifestService::current()` always prefers a valid cache over a newer embedded copy, even right after a herdr-threads upgrade (no `generated_at` comparison). This is what the spec literally says.
   - Single-flight drops a fetch request for the other harness without recording an attempt.

6. **Minor: downgrade and contract-choice edge cases.**
   - Once `setup` has written `--event`, an older binary's argv parser rejects the hook command. CHANGELOG doesn't mention this downgrade break.
   - `newest_contract` picks by newest `first_seen`, so after a downgrade the newer contract's rows keep deciding Health.

7. **Minor: smaller gaps.**
   - The evidence table has no retention, and Health reads every row of each harness on every call.
   - `doctor` computes the manifest policy from its own environment, not from the daemon's.
   - The spec's Post-Implementation Notes section is empty although things diverged: the parse-failure note moved from Health to `doctor`, and `VersionRefused` was added. CHANGELOG does document these.

## Step 2: ledger triage

The ledger has no `parked`, `Recurring minor:` or `Recurring blocker:` lines. Two repeated patterns still show up, so I triaged them as classes:

- **Class A: pre-existing failures claimed but not checked (tasks 1, 2, 6, 9, 10).** Reviewers accepted these claims without checking main.
  - One claim was wrong: the `service::loop_inventory` failure came from this epic's `wait_idle` sleep. ht-xoc.22 fixed it.
  - I checked `docs_test.sh` "follow-on 6" on the fork point a7255713: it fails there too, so that one really is pre-existing.
  - The rest are still unproven: the deep-TMPDIR socket-path failures, `codex_layer_paths`, and the composition failures. The sweep has to settle them.
- **Class B: no red-first TDD evidence (tasks 1, 2, 5, 6, 7, 11, 12, 13).** This is a process issue in the pipeline, not a code defect. Its cost is that nobody showed the tests fail without the fix. The table test and the "Kills:" e2e scenarios partly make up for it.

Individual items worth naming:
- **ht-xoc.7 scenario 9** compares hook wall time against the production `TOOL_BUDGET`, so it can flake under a loaded parallel suite. Watch it in the sweep.
- **ht-xoc.5 Health budget:** in the worst case the "unresolved seats" summary line gets folded once two version lines are present.
- **ht-xoc.5 `claude_observed_text`** still says "the daemon did not admit claude" for `VersionRefused` (`src/cli/doctor.rs:248`).
- **ht-xoc.6 `release_contract.sh`:** when it dies (for example it can't check out the tag), the canary job goes red.
- The rest are cosmetic or disclosed deviations.

## Must fix before landing

1. **Take the evidence step off the hook's critical path** (finding 1). Options:
   - send the note after `observe_harness_in` and after the lifecycle budget is raised, keeping an early send only on the refusal path;
   - or cut the pane-path cap well below 500 ms and show the observe budget is unchanged;
   - or measure SessionStart latency against a busy daemon and record that the margin holds.
2. **Run the deferred full-suite sweep** (`nice scripts/full-suite-gate 1` with `HT_LEAK_RUN_ID`) and then `scripts/check-no-leaked-processes --run-id`.
   - It must come back green, inside the 5-minute budget.
   - Each Class A "pre-existing" failure must be shown to also fail on a7255713, or be fixed.
   - Watch scenario 9 of `tests/integration/harness_version_evidence.rs` for timing flakes.

## Untested scope

- **No full-suite measurement of this branch exists yet.** The caller runs the sweep after this review, so the branch has only had per-merge clippy and focused tests.
- **BLOCKED-AUTH, Task 3 (ht-xoc.8):** the live Codex capture never ran. The permission layer refused `codex exec --dangerously-bypass-hook-trust` and the `codex app-server hooks/list` probe. Not established:
  - whether Codex sends SessionStart with `source: resume` under the original `session_id` (finding 3 depends on this);
  - when Codex hook events fire relative to the rollout file being created;
  - how the shared app-server and `--no-daemon` modes differ.

  Codex attribution currently rests only on reading existing rollout files. Running the capture needs the human to approve the bypass flag.
- **Never exercised for real:** `release_contract.sh`, the `publish-manifest` job (ruleset check and the push to `harness-manifest`) and the release-workflow embed step. They are covered only by offline fixtures.
- **Schema-matched Codex admission** has no test through the hook path.

## Deferred OK

- Findings 2, 4, 5, 6 and 7 can be follow-up beads. Finding 2 must be fixed before anyone adds a recipe `known_broken` range: either let the B6 ladder's refusal show in the verdict, or let local verification override it in the hook.
- Class B (no red-first evidence). File it as a pipeline improvement, not a fix on this branch.
- All minor (deferred) lines in the ledger, including the ht-xoc.5 folding loss and the stale `doctor` wording. The folding loss deserves a follow-up so version lines rank below the unresolved-seats summary.
