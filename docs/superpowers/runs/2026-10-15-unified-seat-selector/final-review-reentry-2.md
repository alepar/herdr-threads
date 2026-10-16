**Final review: epic ht-u3ym, branch `super-auto/unified-seat-selector`**

The fork point from `main` is `cee04a6d6`. The branch has 41 commits and changes 50 files, about 7.9k lines added. The review package is `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto-unified-seat-selector/.superpowers/sdd/ht-u3ym-plan/review-cee04a6d6..a73a58234.diff`. I checked it against the root spec (`docs/superpowers/runs/2026-10-15-unified-seat-selector/2026-10-15-unified-seat-selector-design.md`) and the runtime-stage sub-spec.

## STEP 1: my own view, before reading the ledger

**How the tasks fit together (all verified in the code):**
- **Stage order.** In `run_parsed_in_pane` the order is: thread and pane stages, then the agent-without-session refusal (`derive_refusing_origin`, mod.rs:733), then pass 1 (`resolve_cli_seats`), then the early returns. None of the early returns carries a seat slot. Pass 2 (`allocate_deferred_seats`) runs in exactly two places:
  - the `--operator` branch, after `require_operator_actor`;
  - `run_caller_scoped_deferred`, after `derive_caller`.
  
  `resolve_recipient_seats` comes after pass 2 in both, so seats named by pane (`--require-ack-pane`) are de-duplicated against the frozen IDs.
- **D2 order.** `resolve_one` checks, in order: an exact ID via `SeatInspect` (retired seats included), then the nickname form, then a bare pane ID without narrowing, then a name. An exact ID with no `--space`/`--tab` never reads Herdr. `live_target` treats a retired, mid-retirement or unresolved seat as unverifiable.
- **No raw string reaches dispatch.** Placeholders are checked at the end of pass 1 (`check_unfrozen`) and pass 2, and `run_cooperative` and `dispatch` both refuse leftover `seat_selectors` or `seat_narrowing`. Retry carries no slots. Every production caller of `parse_argv` goes through `run_parsed_in_pane`; `harness/cache.rs` and `picker.rs` call it only in tests. `next:` continuations are built by the daemon from the frozen ID, so they never re-resolve a name.
- **`RequireAck(index)`.** Every index is written before `post_freeze_checks` removes duplicates, and nothing in between changes the vector, so the indices stay aligned.
- **Resolver (D3, D6).** The readings table, matching per level, aliases, narrowing, tiering, the 32-candidate cap with an "N omitted" count, and the renderer contract all match the spec. The round-trip fixture includes a label equal to a seat ID, as the spec requires.
- **`panes::resolve_parents` extraction.** It is move-only, and tab defaulting stays only in the `--pane` path.
- **Inventory and docs.** The D8b clap tree-walk test exists (`every_seat_valued_clap_argument_is_a_recorded_slot`). D9 is done: TRUST-POLICY C1, `docs/agent-usage.md` (including the "label equal to a seat ID" accepted limit) and `integrations/skill/SKILL.md` are updated.

**Checks I ran on `a73a58234`:**
- `cargo clippy --locked --all-targets --all-features -- -D warnings`: clean (51 s).
- `scripts/check-default-features`: exit 0.
- `cargo fmt --check`: clean.
- `scripts/check-harness-neutral`: clean.
- Every non-merge commit carries an `Outside-adapter:` trailer.

**Findings:**
1. **Minor, behaviour change outside the spec: the session refusal moved earlier for every command.** The M1 fix (ht-u3ym.12) calls `derive_refusing_origin` before pass 1 for every command, not only commands with seat arguments. Two paths that returned before `run_caller_scoped` are now refused too:
   - the human-presentation `read` path (`--human read THREAD` → `follow::render_history`);
   - the `--operator` branch.
   
   An agent without a session id now gets `session_claim_required` (exit 4) on those paths. Before, the `read` path rendered, and the operator path gave the `require human` `invalid_request`. This matches TRUST-POLICY ("refused before any daemon call, reads included"), but it was never reviewed as a change. The full-suite sweep will show whether any test depended on the old behaviour. The fix also runs `caller::derive` twice per invocation (mod.rs:733 and 1211); the process-table walk is cheap.
2. **Minor, stale comments.** After the fix, two comments still promise that a refused command allocates nothing:
   - mod.rs:725-730 ("so a refused command allocates nothing");
   - the doc comment on `allocate_deferred_seats` (seat_resolution.rs:345-348: "so a refused command journals no `ResolveSeat`").
   
   Both contradict the corrected TRUST-POLICY C1 sentence: refusals after pass 2 (`check_selection`, `validate_claim`, the caller-binding check) can leave an idempotent guarded seat.
3. **Minor (already a roast Nit).** seat_resolution.rs:337 calls `check_direct_ack(SeatId::new("target"), ["other"]).expect_err(...)` in production code. If `check_direct_ack` is ever relaxed, `send --seat X --require-ack Y` panics instead of refusing.
4. **Minor.** For a lookup name, the `resolved:` line prints during pass 1, so a refusal that comes later can follow an already-printed line. This is cosmetic.
5. **Plan-mandated, needs the human to confirm.** `seat retire`/`rebind`/`--replace` by name use the same proximity tiering. If one match is in the caller's tab and another is elsewhere, the nearer one is silently chosen for an irreversible retire. The only safeguard is the `resolved:` line plus `--operator`.

I found no spec requirement left uncovered, and no seam between tasks that composes wrongly.

## STEP 2: ledger triage

- **Parked items, parked findings, `BLOCKED-AUTH` lines:** none. The fix pass was never entered (0/0/0).
- **Recurring minor (×3 across ht-u3ym.3.3, .4 and .6), class "weak-assertion":** integration tests on the fake `Runtime` fixture assert only that the stage did not fail (negative error-code checks, or a `let _ =` that throws the result away) instead of checking the end state. That is one smell in the fixture: it cannot easily drive a direct send or handoff to completion. There is also one pipeline gap: the sweep bead (.6) did not re-check earlier minors, which its own deferred line admits. The end-to-end behaviour is still pinned elsewhere (`d4_direct_send_empty_pane_allocates_then_sends`, plus assertions on journal and daemon calls). It hides no known defect, so it is fine to defer, with a follow-up to tighten the fixture.
- **Misgraded minors:** none. No minor names a skipped or ignored test, a known product defect or a data-loss path.
- **Individual minors:**
  - index-divergence: I verified it is safe (see STEP 1).
  - help-text-drift: fixed in .6.
  - leftover `sleep 20`: stopped; the process sweep shows 0 survivors.
  - unexplained capability edit in a test: not a defect.
  - test-only shim, dummy-ID reuse (finding 3), the generic `E`, unrolled tests, missing RED-run evidence, the missing no-topology-read check for `handoff --existing` with an exact ID, the help test checking at command level, and reuse of the nickname paste-back test (`nickname_round_trip_as_the_transcript_renders_it` exists): all fine to defer.
- **Metrics:** consistent. 10 merges and 10 completions, no `METRICS INVALID` line, and `landed=completed`.

## Verdict
**Ready, conditional on the full-suite sweep.** No code fix is required, but no full-suite measurement of this branch exists yet: the `cargo nextest` sweep was deferred to the caller. The branch is not tested as a whole until that sweep and the leak check pass.

## Must fix before landing
- Run the deferred full-suite sweep (`nice cargo nextest run --locked --all-targets --all-features`, with `HT_LEAK_RUN_ID` set), then `scripts/check-no-leaked-processes --run-id`. It must pass and stay within the 5-minute budget. Pay attention to tests of a session-less agent doing a human-presentation `read` and to `--operator` refusal precedence (finding 1).
- Get the human's confirmation on finding 5 (operator repair picks by proximity). It is plan-mandated, so it needs a decision, not a code fix.

## Untested scope
- No full-suite run exists for this tree.
- The 5-minute suite budget is unmeasured. The ledger shows one test call hit the 10-minute tool limit (`--test combined lazy_send`), and `--lib cli::seat_selector` took up to 10 minutes per run.
- All coverage uses the fake host and fake daemon. Names have not been resolved against a real Herdr topology or a real daemon.
- The `resolved:` line is checked only through a test-only thread-local; real stderr is never captured.
- No test covers the daemon's own duplicate-recipient refusal now that the CLI de-duplicates first.

## Deferred OK
- Findings 1 (keep it; confirm it in the sweep), 2, 3 and 4. Findings 2 and 3 are cheap to fix in passing.
- The weak-assertion cluster, plus a follow-up to let the `Runtime` fixture assert end-to-end completion.
- All other ledger minors listed above.
- The punch-listed roast Nit: a caller-pane read that fails with any error other than host-unavailable is treated as "no caller". That only widens the result to tier `any` (more conflicts), never a wrong pick.