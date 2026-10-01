# Remaining findings after the first herdr-threads run (2026-10-01)

Read-only analysis of every finding still open at `main@cf8c83d7`, deduplicated across the run report
(`docs/history/herdr-threads-run/report.md`), both PR roast reports, the parked-findings ledger and its
disposition (full ledger: tag `archive/herdr-threads-run-2026-09-26`), and the validation sweep/report.

Excluded as fixed: the four round-1 roast fixes, the two regression-pass fixes, release hygiene
(license, notices, evidence trim, personal data), the FIX-NOW/FIXED/WAIVE rows, and items spot-checked
as fixed in code (`read --follow` retry classes, cooperative doctor state, 0.159.3 docs, already-joined
invite, release builds in Actions).

## Overview

About 165 unique open findings: 7 Should-fix (round-2 roast), 29 Nit (10 punch-list + 19 spot-confirmed
round-1), 1 escalation without a verdict (F6), 10 validation/evidence gaps, 2 remainder-capped roast
candidates whose content was never recorded, and about 116 Minor items (40 ROAST-dispositioned parked
rows, ~70 undispositioned items from waves 15–31, process/UX items).

| # | Bucket | Findings | Priority |
|---|---|---|---|
| B1 | Scaling with retained history | 8 | two SQL filters before release, rest soon after |
| B2 | Background cadence: polling, backoff, event wiring | 5 | Herdr-down hot loop before release |
| B3 | Diagnostics and error semantics | 13 | first-run supportability set before release |
| B4 | Contract-layer drift (ports vs production) | 9 | decision: remove the unreachable layer |
| B5 | Identity, seat continuity and the cooperative trust edge | 15 | F6 + `me init` guards before release |
| B6 | Harness integration, setup and version drift | 28 | version honesty + re-observation before release |
| B7 | Release, packaging and installer | 17 | whole bucket before public release |
| B8 | Native validation evidence and review debt | 40 | G0 rerun, validator false-PASS, re-review before release |
| B9 | Test-suite reliability and coverage | 15 | soon after release |
| B10 | CLI and agent output UX | 15 | backlog, except lost agent continuations |

## B1. Scaling with retained history (8)

The store's design promises that progress never scans retained settled history, but several background
and read paths walk whole tables instead of pending-only projections, and nothing deletes history:
retired seats, completed work jobs, superseded snapshot generations, settled receipts and warnings. The
shared root is that the schema has no retention model and discovery filters in Rust instead of using
indexed pending-only views. For a daemon that runs for weeks the result is slow, silent decay: idle CPU
and IO rise, the 5 s reconciliation cadence stretches (~5 s extra at 800 lifetime seats), wake latency
grows, and the database gains a snapshot row per pane every 5 s. The coherent fix is one design move:
define live versus settled per table, run every discovery query over partial indexes or pending-only
projections, and add a cleanup lane that prunes settled rows on a retention policy (the digest path
already does this; generalise it).

- `src/store/seats.rs:1217` (Should-fix): observation walk includes retired seats, 16 per ~100 ms.
- `src/store/mod.rs:838` (Should-fix): wake worker rebuilds attention from full history each cycle (folds in effective.rs:493 O(seats×warnings)).
- `src/store/seats.rs:702` (Should-fix): snapshot generations and target rows never pruned.
- `src/store/mod.rs:689` (Should-fix): work-job discovery scans completed rows, ignores `work_jobs_ready`.
- `src/store/queries.rs:990` (Nit): quadratic page sizing, re-encode per candidate (13 sites).
- W6-R1 (Minor): show/participants extra daemon connect plus full seat-page walk.
- Wave 18 (Minor): human seats with pending ACK re-examined as wake candidates every pass.
- Wave 27 (Minor): human `read` does SeatInspect + `pane_names` per author and one body fetch per clipped preview.

## B2. Background cadence: polling, backoff and event wiring (5)

The daemon runs on fixed-interval loops (10 ms cancellation polls, 100 ms worker turns, 1 s scheduler
tick) with no notify plumbing and no failure-aware backoff. The design called for event-driven ticks and
bounded retries; only the timers were wired. The idle daemon never quiesces; when Herdr is down the
observation lane hot-loops at ~10 Hz with two durable commits per turn; real work waits up to a second
because nothing kicks the worker. The fix is one shared wakeup-and-backoff primitive (Notify-backed
cancellation, per-lane kick from commit paths, capped exponential backoff on consecutive failures)
adopted by every loop.

- `src/identity/reconcile.rs:885` (Should-fix): failed captures retry every ~100 ms, no backoff.
- `src/service/workers.rs:857` (Nit): 1 job/s; `after_committed_change` never called in production.
- `src/daemon/transport.rs:67/68` (Nit): 10 ms AtomicBool sleep-poll.
- W9-1 (Minor): pre-send refusals climb the 30 s→300 s ladder; a wake can wait 5 min (R20 then MET-WITH-GAP).
- `src/daemon/transport/service_connection.rs:213` (Nit): `SessionLease::drop` takes a blocking std Mutex on the current_thread runtime.

## B3. Diagnostics and error semantics (13)

When the daemon fails the operator often learns nothing actionable: pre-sink startup errors go to
/dev/null, lane failures sit in an unread 8-entry ring, a poisoned status mutex reads as Ready, transient
I/O is reported as "database corrupt", version skew shows as a 5 s timeout, and remedy text points to the
wrong command and an undocumented log path. Nobody owned a failure-to-operator path end to end. For a
curl|bash-distributed tool this is the support surface. The fix is one diagnostics contract: a durable,
rate-limited daemon log at a path `doctor` reports, every lane/startup error routed there, Health saying
"degraded: see log", a precise error taxonomy (transient, unavailable, corrupt, version-skew) checked
before decode, and remedy strings generated from it.

- `src/main.rs:52` (Should-fix): startup failures to /dev/null.
- `src/service/workers.rs:625` (Nit): lane errors only in an unread ring.
- `src/service/workers.rs:599` (Nit): poisoned mutex reads as Ready.
- `src/store/connection.rs:550` (Nit): unlisted SQLite errors → StoreCorrupt.
- `src/daemon/lifecycle.rs:181` (Nit): skew becomes a timeout; old CLI gets a decode error (`deny_unknown_fields`).
- `src/daemon/lifecycle.rs:273` (Nit): "inspect logs" with no path.
- `src/cli/exit.rs:21` (Nit): exit-3 help says ensure where stop is needed.
- W5-3 (Minor): `transitions_refused` never read; `last_reconciliation_at` advances on refusal.
- Wave 26 (Minor): `observe_harness_admissions` spawn result discarded.
- Wave 26 (Minor): `record_tick` counts passes that carry a `work_error`.
- Wave 26 (Minor): doctor labels Unsupported as "(cooperative mode)".
- Wave 16 (Minor): Health's 16-line cap has one line of headroom and no test.
- Wave 25 (Minor): stale Herdr socket file reported as connect failure, not "server not running".

## B4. Contract-layer drift: ports versus production (9)

`src/ports.rs` encodes the earlier adversarial design: allocation, revocation and CallerVerifier paths
reachable only from tests, StorePort defaults returning Unsupported, fake-friendly DeadlinePort
defaults, ~250 lines of forwarding traits, concrete FairWriter/LiveServiceGate leaking through the port
surface. When the trust model became cooperative, production was rewired around this layer, so green
tests exercise a design production does not run. The owner decided (2026-10-01) to remove it: delete the
unreachable verification layer and its store paths, collapse ports to what production calls, and update
the design docs to the cooperative reality in the same pass.

- `src/ports.rs:1292` / sweep B1 (Should-fix): `allocate_seat`, `revoke_registration`, `CallerVerifier`, `MutationPermit::new` have no production callers.
- `src/ports.rs:2150` (Nit): StorePort Unsupported defaults.
- `src/identity/reconcile.rs:24` (Nit): forwarding traits.
- `src/scheduler/deadlines.rs:52` (Nit): fake-friendly defaults.
- `src/ports.rs:2192` (Nit): concrete FairWriter, duplicate `_admitted` entry points.
- `src/ports.rs:2618` (Nit): concrete LiveServiceGate.
- `src/protocol/results.rs:150` (Nit): no ApiError constructor, ~110 literals.
- W6-R5 (Minor): design docs not updated for rank/self/alias; specs INDEX still "draft".
- Wave 20 (Minor): misleading "local compositions" error now also covers Skill.

## B5. Identity, seat continuity and the cooperative trust edge (15)

Seat identity carries the most semantics: a pane-bound seat must survive restarts, moves, `/clear` and
repair, while who may ACK is cooperative rather than proven (user decision 2026-09-28). The residual
findings sit where these meet: the unadjudicated F6 restore-hold dissent, occupancy always Unknown, `me
init` recording agent actions as `operator_human`, a top-level resume accepted without live evidence, a
request racing a restart onto the new boot, directories shared across seats' sandboxes. Each is
defensible under "cooperative, same user", but they were never gathered into one explicit trust model.
The coherent move is a short normative continuity-and-attribution invariants section that adjudicates F6,
names the accepted limits, and adds the few cheap guards that fall out of it. Being reviewed in a
separate session.

- F6 (Escalation): restored-pane creation versus repair reservation.
- P10 (Minor): adapter never reports EmptyShell.
- W5-1 (Minor): `decision_fence` reports `known_invalidated` after reconfirmation.
- W5-2 (Minor): `stage_recipient` change not pinned by a test.
- W6-C3 (Minor): top-level `resume` accepted without live evidence.
- P35 (Minor): request racing a restart reaches the new boot.
- O1 (Observation): joined seat unavailable after daemon restart until next check-in; extra warning.
- Wave 18 (Minor): `me init` records agent actions as `operator_human`.
- Wave 18 (Minor): `me init` ignores the service binding harness and replaces an agent binding.
- Wave 30 (Minor): shared `intents/` and `contexts/` writable across seats (document as accepted).
- `src/cli/journal.rs:467` (Nit): `allocator.lock` opened without O_NOFOLLOW.
- Wave 28 (Minor): suffix-retry agent name can start a second agent for the same seat.
- W9-2 (Minor): `cooperative_wake_ready` does not check the harness.
- W6-R2 (Minor): "mark it self" wording holds only in-pane.
- Waves 29/19 (accept): 48-bit pagination tag, 2^-47 prep-ID reuse, "104 bits" comment (actually 112).

## B6. Harness integration, setup and version drift (28)

The product lives inside two fast-moving harnesses (Claude 2.1.284→2.1.286 and Codex 0.159.2→0.159.3
during the run alone). Support is asserted through recipe intervals, schema fingerprints and hard-coded
version strings spread across recipes, health strings, docs and evidence, with no single source of truth
and no runtime re-observation; evidence lags the assertions. Setup also makes environment assumptions
that fail quietly. For early adopters, whose harness is newer than anything tested on day one, this is
the likeliest first failure. Owner direction (2026-10-01): optimistically admit new versions, and run a
scheduled CI job that pulls fresh harness versions, tests them, bisects the exact breaking version on
failure and adds an adapter for that version range.

- Versions without evidence (Gap): Claude 2.1.283/2.1.284, Codex 0.157.1/0.158.0 have no receipt runs; Codex 0.159.2 schema-only; Claude managed launch last ran on 2.1.285.
- Sandbox writes (Gap): no live run with a non-tmp state dir; probe never exercised pane resolution, the hook or daemon start inside the sandbox.
- Wave 25 (Minor): daemon keeps the Codex version it saw at start.
- Wave 16 (Minor): `COOPERATIVE_RECEIPT_LINE` hard-codes versions.
- Wave 26 (Minor): latent `codex_status` would report Supported if any recipe declared native receipt.
- Wave 26 (Minor): doctor does not check whether the `claude` on PATH is admitted.
- Wave 17 (Minor): pre-existing `features.network_proxy` keys widen the allowance.
- Wave 17 (Minor): launch checks the launcher's CLAUDE_CONFIG_DIR/CODEX_HOME, not the pane's.
- Wave 17 (Minor): `codex_install` non-atomic (hooks first, then config).
- Wave 17 (Minor): unsetup after plugin uninstall and symlinked dotfiles undocumented.
- Wave 17 (Minor): quiet gate applies only after a successful parse.
- Wave 25 (Minor): stale `~/.local` state dir chosen when the XDG one is missing.
- Wave 25 (Minor): `HERDR_SESSION` disables only the socket default; fast path skips plugin-installed check.
- Wave 25 (Minor): no fresh-install 0.159.3 setup test.
- Wave 30 (Minor): v2 manifest refuses after downgrade; `plan_remove` removes hand-added duplicates.
- Wave 20 (Minor): hook hint also fires on SubagentStart; docs say SessionStart only.
- P1 (Minor): setup always prepends `--no-daemon`.
- P2 (Minor): `codex_layer_paths` walks up to `/`.
- P29 (Minor): misleading moved-binary conflict message.
- W6-C4 (Minor): no rc-5 early-exit detection for managed Codex.
- P19/P20/W6-D5 (Minor): context budget fallbacks (trim order, no fit check, long run-root fails S15).
- W6-R3 (Minor): digest None drops the D2 procedure line.
- Wave 21 (Minor): a PTY harness (Codex `tty:true`) gets human output.
- Wave 28 (UX): wake prompt left typed but unsent in a Claude composer.
- Wave 31 (UX): Herdr shows name "None" for launched agents.
- Wave 28 (UX): "codex profile didn't get applied" never answered.
- FIX-NOW lane (Minor): install.md:119 list broken; launch cache warm-up undocumented as best effort.
- FIX-NOW lane (Minor): launch.rs:333 `-c`/`-i` refusal also hits positionals.

## B7. Release, packaging and installer (17)

The release path has never run for real: no tag pushed, Linux musl and Intel macOS never compiled. The
workflow creates a release on a manual dispatch from a tag, cannot re-run after a partial failure, and
pins actions to mutable tags under `contents: write`. The installer reports success when Herdr refuses
to link on Linux (manifest is macOS-only), skips stopping the old daemon on upgrade when Herdr is down,
parses JSON by splitting on "{", and gives inconsistent next-step hints. The fix is a release rehearsal:
tag v0.1.0, build every target, install/upgrade/uninstall on a clean machine with Herdr up and down, and
make the installer's final status truthful; pin actions by SHA, gate release creation on push events,
decide the platforms list.

- Linux musl and Intel macOS never compiled (Gap).
- `herdr-plugin.toml` `platforms=["macos"]` (Minor): installer exits 0 when linking is refused.
- `release.yml:134` (Minor): dispatch on a tag creates a release.
- `release.yml` (Minor): re-run fails at `gh release create`.
- `release.yml:128/133` (Nit): actions pinned to mutable tags.
- `install.sh:377` (Nit): no upgrade stop while the server is down, then exit 3; message at :385 misleading.
- `install.sh` (Minor): uninstall without herdr on PATH leaves the registration.
- `install.sh` (Minor): uninstall runs unsetup with no prompt.
- `install.sh` (Minor): `herdr_action` splits JSON on "{".
- `install.sh` (Minor): "atomic" replacement is two renames.
- `install.sh` (Minor): inconsistent doctor/register hints, even after `--no-herdr`.
- `install.sh` (Minor): setup reminder wrong when no harness is found or one is declined.
- `tests/release/install_test.sh` (Minor): partial setup failure untested.
- `docs/release.md:12` (Nit): false claim the build is unchanged since d635aca.
- P25 (Minor): clippy runs only with `--all-features`.
- Release notes (Minor): STATUS constants removed, `inv-` prefix, manifest v2 downgrade.
- README demo and "Try it" polish (Minor).

## B8. Native validation evidence and review debt (40)

The support claims rest on live-model evidence that describes older code (G0: 60+ source files changed
since the report base). Five matrix cells were never exercised, and the validator that turns transcripts
into verdicts has known false-PASS paths. Alongside sits process debt: regression-pass fixes never
re-roasted, the last third of features lightly reviewed, unenumerated parked items from four merges, the
human goal-versus-tree read-through never done. Evidence is stamped per run, not per release, and the
validator is trusted more than it is tested. The coherent move is a release-SHA evidence pipeline: harden
the validator's false-PASS paths, rerun the full matrix on one SHA with the owed cells folded in, and do
one targeted review pass over the least-reviewed code.

- G0/B2 (Gap): stale native evidence; rerun every row on the release SHA.
- B3 (Gap): concurrent children NOT_EXERCISED on both harnesses.
- B3 (Gap): SW2 coalesced warning wake NOT_EXERCISED live on both harnesses.
- B3 (Gap): Codex TUI children write-absence NOT_EXERCISED.
- Token overhead (Gap): no benchmark.
- ht-910 (Gap): no live run of a daemon restart while the agent stays in its pane.
- P40 (Gap): crash fix3 real-host acceptance never run.
- Validator false-PASS (5, Minor): P15 nested `claude -p`/`codex exec`; P31 env/sudo/exec/backtick wrappers; W10-E1 SL1 seq coverage; W10-E6 inflated spawn count; W10-E7 one child counted as two readers.
- Validator labels/false-FAIL (6, Minor): W6-D3, W6-D4, W10-E2, W10-E3, W10-E5, W10-E8.
- W9-5 (Minor): R13 shell-phase check does not assert an Unsafe row.
- FIX-NOW lane (2, Minor): quoted-literal false positive untested; native.rs incarnation-recheck test does not move server state.
- Wave 15 report/claims precision (7, Minor).
- Wave 16 sweep status consistency (2, Minor): R20 and R23 should be MET-WITH-GAP.
- Process debt (10): P41–P44 unenumerated parked items; P45 degraded verdicts and missing goal-vs-tree read-through; regression-pass code never re-reviewed; light-review lanes (user-level setup, short IDs, token diet, human output, release/installer, `read --follow`); 17 DONE_WITH_CONCERNS reports never re-checked; 2 remainder-capped roast candidates never seen.

## B9. Test-suite reliability and coverage (15)

The suite is green only serially: `hook_entrypoint` exits 101 under the parallel harness (waived, never
root-caused), a process-global VM-instruction counter corrupts cost-flatness tests, at least five named
tests flake under load, and CI pins `--test-threads=1`. Separately, several tests assert weakly (bare
`is_err()`, sleep-then-assert-absence, wall-clock bounds) and some branches are untested. The root cause
is shared mutable test infrastructure plus time-based assertions in a concurrent system. The fix is one
isolation pass: per-test state roots and counter contexts, deterministic clocks or barriers instead of
sleeps, the serial pin dropped or confined to a named group, Python demo suites in CI, and the missing
negative tests.

- `ci.yml:101`, ht-4is.11.8, P12, P13 (Nit): serial pin; `hook_entrypoint` exits 101 in parallel.
- `tests/store/attention.rs:277` (Nit): global UNITS counter.
- Named flakes (Minor): `concurrent_publishers_receive_distinct_ordered_keys`, `async_accept_listener_keeps_owner_lock_until_server_stops`, `pending_invitation_axis_is_flat`, `digest_cost_is_flat…`, host-seats service timing.
- `tests/host_adapter.rs:487` (Nit): wall-clock slack.
- `tests/service/resolution.rs:2001` (Nit): sleep then assert absence.
- `tests/store/attention.rs:481` (Nit): recipients backfill untested.
- `src/store/messages.rs:521` (Nit): publish-time archived recheck untested.
- `tests/store/receipts.rs:1455` (Nit): asserts only `is_err()`.
- `tests/hook_entrypoint.rs:1015` (Nit): payload pasted 7 times.
- `ci.yml:105` (Nit): Python demo suites not in CI.
- P37 (Minor): ownership test depends on fixture path length.
- Wave 18 (Minor): no test for wake suppression or hook-path takeover.
- Wave 26 (Minor): sweep does not pin "healthy" in cooperative setup.
- Wave 16 (Minor): sweep's late-ACK check uses a fixed 2 s sleep; "rejected" misnamed.
- Wave 28 (Minor): latency test never asserts prompts > 0; stderr undrained.

## B10. CLI and agent output UX (15)

Two output modes exist (compact/machine for agents, human for a TTY), and several fields and
continuation commands were lost in the split: human inbox has no topic or invitation id, `checked_in`
drops the warnings continuation, compact `thread` drops the topic-detail argv, timestamps lack a date.
Escaping covers C0/C1 but not bidi or Cf characters. The follower misclassifies one slow connect as
"lost the daemon" and prints fatal errors twice. The presentation layers were added late under token-diet
pressure without a per-field contract. The fix is one presentation contract (per result type: must-keep
fields and next-step argv per mode, enforced by golden tests) plus one shared escaping function.

- Wave 21: inbox drops topic and invitation id.
- Wave 21: `checked_in` drops warnings and `offered_through`.
- Wave 21: `write_selected` return-length doc wrong.
- Wave 21: `is_unsafe` misses LRM/RLM/Cf; CJK misaligns columns.
- Wave 31: escaping decided per page, undocumented.
- Waves 31/27: bidi/zero-width print raw; follow notices and argv unescaped; irc indentation is the only forged-line guard.
- Wave 31: `--offset` fallback is dead code.
- Wave 31: already-joined invite prints the generic form.
- Wave 29: compact `thread()` drops `topic_detail_argv`.
- Wave 29: HH:MMZ timestamps have no date.
- Wave 29: `event_row` drops fields.
- Wave 20: SKILL.md overclaims `--json`; digest example misplaced.
- Wave 18: pane-name precedence silent; `pane_not_found` gives no hint.
- Waves 27/28: follow exits on Ctrl-C only between calls; NickCache takes `context.lock`; one slow connect prints "lost the daemon"; Fatal notice printed twice.
- Wave 25: autodetect can make a command wait up to HERDR_QUERY_TIMEOUT.
