super-roast verdict: Should-fix (3 confirmed) [converged]
mode: PR        iteration: 2 of 3
profile (assumed): herdr-threads is a pre-release (Cargo 0.1.0, README: "No GitHub release is published yet") Rust plugin for the Herdr terminal multiplexer that runs a single-user local daemon on macOS. The README states identity is cooperative, not a security boundary, and the daemon serves only same-UID callers. There are no external users, no real money or multi-tenant data, and rollback is a local reinstall. Resilience, observability and cost findings are down-weighted; correctness breaks of the documented upgrade and trust-policy paths are not.
inputs: trust-model-invariants@d2bb9641 vs main@55512edb
delta vs prior: 3 new confirmed (0 Blocking) · 0 carried (0 Blocking) · 13 resolved · 0 regressed (0 Blocking)
coverage: scouts 13/13 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, regression, data-migrations, deploy-safety, api-contract, observability, testing, hygiene-docs) · raw 9 → deduped 4 → panel 3 · spot 1 · promoted 0 · judge completion 100% · remainder-capped: 0
independence: same-family (Claude) — seat-differentiated panel
seat-agreement: panels 3 · rr 0.67 · rg 1.00 · fg 0.67 · unanimous 0.67 · ground-loo 1.00 (n=2) · reproduce 3/0/0 · refute 2/1/0 · ground 3/0/0

## Confirmed findings
- [Should-fix] src/cli/hook.rs:1029; src/cli/hook.rs:1037 — After a C1 reattachment whose reply was lost, the pending continuity intent is never completed on the ordinary path; a later seatless resume of the same session in the pane reuses that intent's operation key, the daemon replays the historical ContinuityReattached result without reattaching, and the hook installs a stale context, completes the intent and reports success while the seat stays unresolved. (new this iteration; introduced by the ht-rzi.18 fix commit 45d61f69)
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: submit_continuity returns Kept at the hook deadline after transport/UnknownOutcome results, leaving the intent on disk even when the daemon committed; tests/hook_entrypoint.rs:3823-3850 asserts `fx.intents() == 1` after the lost reply. check_in (hook.rs ~928-933) calls reattach_by_continuity only when find_seat is not Resolved, and the doc at hook.rs:1087 says tool events never read the intent, so nothing completes it. continuity_intent (hook.rs:1029-1050) reuses the pending intent's reference and execution whenever harness and native_session match, with no check that the operation was already decided. OrdinaryIdentity::continuity (src/identity/repair.rs:225-236) calls store.replay_continuity first; replay_continuity (src/store/seats.rs:3773-3806) matches on operation key plus a digest of (instance, target, harness, native_session, execution) that contains no boot or epoch, and `grep 'DELETE FROM operations' src` finds nothing, so the old row is returned with no state change. reattach_by_continuity (hook.rs ~1097-1130) then calls install_reattached with the old generation, runs journal.complete, swallows the presentation check-in failure with unwrap_or_else, and returns Some(done). The only reuse test (tests/cli/hook.rs:2156) uses a scripted daemon and never models a decision committed under an earlier incarnation. All three seats rated Should-fix; the trigger is narrow (lost reply, second unresolve, same-session resume in the same pane) and self-heals on the following resume, but it is a silent false success on the C1 recovery path the fix pass redesigned.
  fix-shape hint: complete the intent once the seat resolves on the ordinary path, or have continuity_intent/reattach_by_continuity discard a reused intent whose replayed binding generation is not the seat's current one and record a fresh intent instead.
- [Nit] scripts/install.sh:380; src/cli/mod.rs:125 — After the PROTOCOL_VERSION bump to 2, the new executable refuses `daemon stop` against a protocol-1 daemon, but scripts/install.sh deletes the old executable before stopping, so an upgrade leaves the old daemon running with no documented way to stop it short of a manual kill. (new this iteration; follow-on to the prior Should-fix at src/protocol/wire.rs:20, which is resolved) [fix-regression]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: Fix commit 0f2784ff adds to connect() (src/cli/mod.rs:125-131) `if descriptor.protocol_version != PROTOCOL_VERSION { return Err(protocol_mismatch_error(..)) }`, and DaemonAction::Stop (mod.rs:451-452) goes through connect. scripts/install.sh (untouched by the diff) runs `mv "$install_dir" "$install_dir.old"; mv "$install_dir.new" "$install_dir"; rm -rf "$install_dir.old"` at lines 308-317 and only afterwards `herdr_action stop || true` at lines 377-380 with the new binary; the refusal is swallowed and `herdr_action ensure` then fails with the hint "run daemon stop with the matching older executable" (lifecycle.rs:119), which no longer exists on disk. docs/install.md 'Updating' and docs/operations.md:80 say "Stop the old daemon with the old executable first"; neither mentions install.sh does the opposite, and grep of docs/install.md, docs/operations.md and README.md finds no kill/pid guidance; docs/install.md:243 says Herdr has no shutdown hook. The refute seat REJECTed as an accepted tradeoff (the docs state the cross-protocol refusal on purpose) and immaterial pre-release; the two confirming seats acknowledge both points and still rate the installer-contradicts-docs gap real. The refute seat's evidence is addressed by the majority, so the tally stands.
  demoted: profile states a pre-release single-user local tool with no published release, so no install.sh 1->2 upgrade exists in the wild and the manual kill is cheap; Nit.
  fix-shape hint: run `herdr_action stop` with the old executable before the install-dir swap in scripts/install.sh, or document the manual kill (descriptor pid) in docs/install.md 'Updating' and the mismatch hint.
- [Nit] src/cli/hook.rs:1110 — When the daemon has committed a C1 reattachment but the hook then fails locally (seat_contexts or install_reattached returns Err), reattach_by_continuity returns None and the hook emits the pre-reattachment diagnostic "pane seat mapping is Unresolved" as Failure::Unavailable, although the daemon now holds the seat resolved; nothing reports the committed reattachment or the local install failure. (new this iteration)
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: reattach_by_continuity (hook.rs ~1088-1150) matches ContinuityOutcome::Reattached (daemon committed), then runs `seat_contexts(..).ok()?` and `contexts.install_reattached(context).ok()?`, turning any ContextError into a bare None. check_in (~930) maps None to `Err(absent.refusal(pane))`, where `absent` is the PaneSeat::HeldOrUnresolved("pane seat mapping is {:?}") built by find_seat at hook.rs:785 before the commit, and refusal() (hook.rs:723) yields Failure::Unavailable. The in-code comment says the daemon replays the same result on the next resume, but the next resume's find_seat returns Resolved and never reaches reattach_by_continuity, so only `herdr-threads retry` would complete the leftover intent. docs/operations.md:192 ("the pane stays held") is inaccurate for this post-commit state. `git show main:src/cli/hook.rs | grep -c reattach_by_continuity` gives 0, so not pre-existing. Distinct from the prior-rejected pre-commit silent paths (hook.rs:1076, :1081), where the fallback diagnostic is still true. All three seats rated Nit: requires a rare local lock timeout or I/O error after a daemon commit; daemon state is correct; no data loss.
  demoted: profile down-weights observability for a single-user local tool with easy recovery; Nit.
  fix-shape hint: log or return a distinct Failure for the committed-but-not-installed case instead of `.ok()?`, and correct the comment and docs/operations.md paragraph.

## Not verified (beyond panel cap)
- none

## Not verified (dedupe failed or judge lost)
- none

## Beyond remainder cap (count only)
- none

## Rejected (with reason)
- none this iteration. No scout re-surfaced any finding the prior report rejected; those rejections stand.

## Prior confirmed findings (iteration 1) — status
- src/protocol/wire.rs:20 (no PROTOCOL_VERSION bump) — resolved by 0f2784ff (ht-rzi.23): PROTOCOL_VERSION is 2, connect() reports the mismatch, skew docs added. The installer follow-on is the new Nit above.
- src/identity/repair.rs:237 (diagnostic read inside the 250 ms guard window) — resolved by df3b96a6 (ht-rzi.20); packet evidence confirms the 750 ms read now runs before the observation.
- src/notification/dispatch.rs:137 (wake harness guard fails open, FYI pre-existing) — resolved by 905c017b (ht-rzi.21); not re-surfaced by any scout.
- src/store/seats.rs:1799 (CarryForward ignores provenance) — resolved by 6ada1509 (ht-rzi.19); not re-surfaced.
- src/cli/mod.rs:1194 (A4 refusal advertises an override selection_from_context ignores) — resolved by d0742034 (ht-rzi.22); not re-surfaced.
- src/cli/mod.rs:229 (any CODEX_* counts as agent evidence) — resolved by d0742034 (ht-rzi.22); not re-surfaced.
- src/cli/hook.rs:924 (finish_pending_continuity on every hook event) — resolved by 45d61f69 (ht-rzi.18); `grep finish_pending_continuity src` finds nothing. Its replacement design is the source of the new Should-fix above.
- src/cli/journal.rs:769 (unguarded intent header scan, FYI pre-existing) — resolved as to this change: the per-tool-event trigger is removed; the base-branch scan itself remains pre-existing on main.
- src/cli/launch.rs:483 (paged SeatInspect launch guard) — resolved by 905c017b (ht-rzi.21); not re-surfaced.
- src/cli/journal.rs:766 (pending_continuity fails the whole scan on one bad file) — resolved: journal.rs:780-807 now skips unreadable headers (`let Some(header) = read_intent_header(&path) else { continue }`).
- src/cli/mod.rs:601 (operator override retires the local context before the daemon accepts) — resolved: mod.rs:641-646 comment and code now retire nothing on the operator direction; the daemon decides and dispatch replaces the context only on success.
- src/host/native.rs:907 (diagnostic read bumps the connection epoch) — resolved by df3b96a6 (ht-rzi.20, "outside the guard window and epoch fence"); not re-surfaced.
- src/service/workers.rs:1323 (no loop-level refused-pass test) — resolved: tests/service/worker_health.rs:1400-1440 runs spawn_observation_loop with refuse_transitions=true and asserts pass_records stays 0.

## Unverified nits (spot-checked)
- [spot: CONFIRM Nit] src/cli/hook.rs:1143 — The C1 reattachment path sends a Compact-kind check-in and prepends render_context(TopLevel) to a result that already starts with the same instruction when the seat has pending attention or the digest read fails, so the standing instruction is emitted twice and the second copy is JSON-escaped inside `untrusted_peer_data`; no test seeds pending mail on a reattach. Spot: bridge.rs:999-1010 encode_hook writes the instruction first; encode_native (hook.rs:393-396) strips one prefix only; the only C1 presentation test (tests/hook_entrypoint.rs:3329) checks `starts_with` alone. Introduced by 45d61f69; benign fixed text, low impact.

## Escalations (need human)
- none
