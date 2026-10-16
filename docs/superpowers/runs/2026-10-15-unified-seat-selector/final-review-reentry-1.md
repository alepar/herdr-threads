**Verdict: not ready.** One small fix is needed (M1 below), and the branch has not had a full-suite run yet.

Read-only review of `super-auto/unified-seat-selector` against `main`. The fork point is `cee04a6d6`. There are 37 commits and 45 files changed. I checked the diff against the root spec `docs/superpowers/runs/2026-10-15-unified-seat-selector/2026-10-15-unified-seat-selector-design.md` and the runtime-stage sub-spec. The review package is at `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto-unified-seat-selector/.superpowers/sdd/ht-u3ym-plan/review-cee04a6d6..599b27a42.diff`.

## STEP 1: my own view, before reading the ledger

The tasks compose correctly. Specifically:

- **Order of resolution (D2).** `resolve_one` in `src/cli/seat_resolution.rs` runs the steps in spec order: exact ID via `SeatInspect`, then the nickname, then a bare pane ID, then a name. An exact ID with no `--space`/`--tab` makes no Herdr read. The daemon's `seat_inspect` returns `NotFound` for an unknown seat, so a name correctly falls through to the next step.
- **Two passes.** No path skips pass 2. After pass 1, the only routes that can carry an allocating slot are the `--operator` branch (pass 2 runs after `require_operator_actor`) and `run_caller_scoped_deferred` (pass 2 runs after `derive_caller`). The other early returns (me init, launch, picker, follow, history) carry no seat slots.
- **`--require-ack` indexing.** `RequireAck(index)` stays aligned: each slot writes its position before `post_freeze_checks` removes duplicates.
- **Direct-ACK check.** The direct-ACK comparison runs before any allocation. When one side is an exact seat and the other a pane, it only looks up the pane's existing seat and never allocates.
- **Old CLI checks.** The old parse-time duplicate check and direct-ACK check now run only after IDs are frozen. The leftover-selector guard is in place both in `run_cooperative` and at the end of pass 2.
- **`--space`/`--tab` with `--seat`.** These go to `SeatNarrowing` and never become a pane selector. `seat rebind` is correctly left out, because its `--space`/`--tab` locate the target pane. For `handoff --existing`, the selector is cleared when `--seat` is given (D8a).
- **Capability refactor.** `direct_send::require_capabilities` now always passes `modern=true`. This changes nothing, because `start` always sets `creation_policy` to `Some`.
- **Path renderer (D6).** It matches the contract: sibling-uniqueness and ID-collision checks at each level, and same-tab agent-name and pane-ID collisions for panes.
- **Retry.** Retry carries no slots, so it never re-resolves. The retry-after-relabel test pins this.
- **Coverage of seat arguments.** A test walks the whole clap command tree (D8b), so a new seat argument cannot skip resolution.
- **Docs and policy.** `TRUST-POLICY.md` C1, `docs/agent-usage.md` and `integrations/skill/SKILL.md` were all updated (D9).
- **Checks I ran.** `scripts/check-harness-neutral` passes, `cargo fmt --check` is clean, and every non-merge commit carries an `Outside-adapter:` trailer.

What I found:

1. **Important: an agent with no session now gets the wrong refusal.** In `src/cli/mod.rs`, the seat stage's first pass (`resolve_cli_seats`, around line 730) runs before the agent-without-session refusal in `run_caller_scoped_deferred` (around line 1187).
   - Pass 1 makes daemon calls (`SeatInspect`, `Seats`/`pane_seat`, up to 32 candidate lookups) and Herdr reads, and can print `resolved:` lines, all before that refusal.
   - So an agent with no session that passes a seat name now gets `host_unavailable`, `not_found`/`conflict` (exit 1) or `no_seat_for_pane` (exit 2), instead of the documented `session_claim_required` (exit 4).
   - That contradicts `TRUST-POLICY.md` ("The CLI refuses before any daemon call an agent whose session id is unset") and `docs/agent-usage.md` ("Session id requirement").
   - Thread-name resolution (`resolve_cli_threads`) already had this ordering before this branch. The seat stage extends it and adds a dependency on Herdr.
   - Nothing is written in the wrong order: the existing test `refused_commands_journal_no_resolve_seat` (a) confirms no `ResolveSeat` happens. Only the error code and its precedence are wrong.
   - AGENTS.md requires `TRUST-POLICY.md` to be updated in the same commit when an invariant is weakened, and it wasn't.
2. **Minor: dead check.** In `post_freeze_checks`, `check_unique_recipients` runs right after a `retain` that already removed duplicates, so it can never fail. Repeated `--require-ack` values are now silently de-duplicated instead of refused. The spec intends this, and the docs say "duplicates deduplicate".
3. **Minor: empty name.** `--seat ""` (or a whole-string reading) can match a pane, tab or space whose label is the empty string. The renderer never prints empty labels, but matching does accept them.
4. **Design point to confirm: operator repair by proximity.** `seat retire`/`rebind` by name follow the same tier ranking as everything else. If one match is in the caller's tab and another is elsewhere, the command quietly picks the nearer seat and retires it, which cannot be undone. The `resolved:` line shows which seat, but there is no confirmation step. The spec and TRUST-POLICY chose this on purpose ("uniquely resolves to", with tiering), so it is plan-mandated, but it is worth the human confirming.

## STEP 2: ledger triage

- **Parked items, parked findings, `BLOCKED-AUTH` lines:** none. The fix pass was never entered.
- **Recurring minor (×3, across ht-u3ym.3.3, .4 and .6), class "weak-assertion":** some tests assert that the command did not fail in certain ways, or throw away its result with `let _ =`, instead of asserting the end state. It is really one finding raised again, not a systemic defect. The sweep task (ht-u3ym.6) didn't re-check earlier deferred minors, and its own minor admits this; that is the pipeline gap. Most matrix cases still check the right outcome through issued daemon calls or the journal. The weakest is `direct_send_seat_name_with_space_narrows`, which never checks that `EnsureDirectThread` was reached; the end-to-end send is covered by `d4_direct_send_empty_pane_allocates_then_sends`. Fine to defer.
- **Misgraded minors:** none. No minor names a skipped or ignored test, a known product defect or a data-loss path.
- **Individual minors:**
  - index-divergence: I verified it is safe.
  - help-text-drift: fixed in ht-u3ym.6.
  - leftover `sleep 20`: stopped during cleanup, and the process sweep shows 0 survivors.
  - The rest (unrecorded RED runs, the test-only shim, dummy-ID reuse in `check_direct_ack_before_allocation`, the generic `E`, unrolled tests, the missing no-topology-read assertion for handoff with an exact ID) are all fine to defer.

## Verdict
Not ready. Fix M1, then run the full-suite sweep. There are no merge or check failures and every task review came back clean.

## Must fix before landing
- **M1.** Restore the documented refusal order. Either:
  - move the `caller::derive` / `caller_needs_identity` session refusal above `seat_resolution::resolve_cli_seats` in `run_parsed_in_pane` (`src/cli/mod.rs`). It depends only on the action kind, not on seats, so it can move safely. Add a test where an agent with no session and a seat name gets `session_claim_required` with no `SeatInspect` call; or
  - record this as an accepted limit in `TRUST-POLICY.md` (Refusals) and `docs/agent-usage.md` ("Session id requirement").

## Untested scope
- **No full-suite run of this branch exists yet.** The `cargo nextest` sweep was deferred to the caller, so treat the branch as not yet tested as a whole.
- No evidence in the ledger that clippy (`-D warnings`, all features) or `scripts/check-default-features` ran on the merged tip. The merge check was only `cargo check`, which does not fail on warnings.
- All coverage uses the fake host and fake daemon. Name resolution has not been run against a real Herdr topology or a real daemon. The `resolved:` line is checked through a test-only thread-local, not by capturing the real stderr.
- No test runs the daemon's own duplicate-recipient refusal from the CLI now that the CLI removes duplicates.

## Deferred OK
- Finding 2 (the dead `check_unique_recipients` call).
- Finding 3 (empty-string labels can match).
- Finding 4 (operator repair by tier ranking): plan-mandated; the human should confirm it.
- The weak-assertion cluster and all other ledger minors listed above.