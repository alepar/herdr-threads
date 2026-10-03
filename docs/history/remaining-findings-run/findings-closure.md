# Findings closure ledger (ht-p03.37)

Every finding of buckets B1-B4 and B6-B10 in `docs/history/remaining-findings-2026-10-01.md` (150 rows; B5 is the
other 15 findings of the doc's 165 and is covered by the non-interference section below) mapped to the bead and
commit that closed it, to an explicit deferral, or to `pending ht-p03.20`. Written on integration tip `70ef9208`
(task branch `task-ht-p03.37`). `python3 findings-closure-check.py` counts the ids in the findings doc against
the rows below and checks every cell (bead exists, commit exists and names its bead, deferral reason resolves).

## How to read the table

- **id**: the findings doc's id rule (`findings-closure-check.py`, docstring): leading backticked location, else the
  token before ` (`/`:`; a repeated id inside one bucket is numbered `#1`, `#2` in document order; a bullet that
  aggregates N items (`Process debt (10)`, `Validator false-PASS (5, Minor)`, ...) expands to `[1]`..`[N]`.
- **closing bead / commit**: the bead(s) whose work closed the finding and, for each, the merge commit that put it on
  the integration branch (`Merge branch 'task-<bead>'`; ht-p03.26 is `19f76cc1`, its partial merge). A row closed by a
  seam integration bead (ht-p03.12.11/.12.13/.14.9/.40/.42/.44/.46/.48) lists that bead and its commit beside the
  bucket bead's.
- **status**: `closed`; `pending ht-p03.20` (code is merged, but the closure depends on native cells that only the
  final-SHA rerun can pass; the **backing cells** column names them, root spec section B8 Decision 3);
  `deferred: <reason>` (a bd id or a phrase from the root spec's Explicit deferrals table); `open` (a pending row
  whose backing cell did not PASS, written by ht-p03.20).
- A pending row is closed by ht-p03.20 only if every backing cell PASSes on the evidence SHA in `docs/validation/report.md`.
  A NOT_EXERCISED, FAIL or stale backing cell leaves the row `open` with that cell named. It is never closed or
  deferred silently. The early cell run (`native-smoke.md`, ht-p03.38) is not closure evidence; its outcomes are
  quoted in the finding column only to show what ht-p03.20 should expect.

## Ledger

| id | bucket | severity | closing bead | commit | status | backing cells | finding |
|---|---|---|---|---|---|---|---|
| src/store/seats.rs:1217 | B1 | Should-fix | ht-p03.12.4 | 7e1af227 | closed | - | observation walk includes retired seats, 16 per ~100 ms. => observation walk reads `seats_live_ordinal`, retired seats excluded |
| src/store/mod.rs:838 | B1 | Should-fix | ht-p03.12.5 | 7633c1ab | closed | - | wake worker rebuilds attention from full history each cycle (folds in effecti... => wake attention from the v8 pending projections with O(1) per-seat probes; the effective.rs:493 fold left the production path |
| src/store/seats.rs:702 | B1 | Should-fix | ht-p03.12.6 | 3c56528b | closed | - | snapshot generations and target rows never pruned. => retention lane prunes superseded snapshot generations and target rows outside the keep set |
| src/store/mod.rs:689 | B1 | Should-fix | ht-p03.12.4, ht-p03.12.11 | 7e1af227, d7afbdb0 | closed | - | work-job discovery scans completed rows, ignores `work_jobs_ready`. => work discovery reads `work_jobs_live`; seam .12.11 round-trips the wake/work/recovery page cursors against the PageFit oracle |
| src/store/queries.rs:990 | B1 | Nit | ht-p03.12.2 | a5807aa0 | closed | - | quadratic page sizing, re-encode per candidate (13 sites). => all paged-result sites fit pages through one linear helper |
| W6-R1 | B1 | Minor | ht-p03.12.3, ht-p03.12.13, ht-p03.126 | 36c06915, 65c78570, 75c07060 | closed | - | W6-R1 (Minor): show/participants extra daemon connect plus full seat-page walk. => one connection and a limit-2 seat page; seam .12.13 counts client cost; ht-p03.126 fixed the later pane_seat paging regression |
| Wave 18 | B1 | Minor | ht-p03.12.5 | 7633c1ab | closed | - | Wave 18 (Minor): human seats with pending ACK re-examined as wake candidates ... => human-bound seats excluded from wake discovery (`NOT EXISTS` on `occupant_bindings_current`) |
| Wave 27 | B1 | Minor | ht-p03.12.7, ht-p03.12.8, ht-p03.44, ht-p03.12.13 | b7f2691b, 17093e5c, 08bbddac, 65c78570 | closed | - | Wave 27 (Minor): human `read` does SeatInspect + `pane_names` per author and ... => author names resolved once per page; clipped previews through history `full_bodies`; seam .44 pins the `full_bodies` wire compatibility and seam .12.13 drives one human read session against every bound |
| src/identity/reconcile.rs:885 | B2 | Should-fix | ht-p03.9.5, ht-p03.104 | c03538e0, 6902f91f | closed | - | failed captures retry every ~100 ms, no backoff. => observation lane on the Pacer: backoff, repeat-invalidation skip, at most one commit per backoff step; ht-p03.104 lets a latched kick trigger observation during backoff |
| src/service/workers.rs:857 | B2 | Nit | ht-p03.9.1, ht-p03.9.4, ht-p03.40 | d2e4ec11, 45fecd03, 3955cb64 | closed | - | 1 job/s; `after_committed_change` never called in production. => commit-change kicks wired into the writer (.9.1) and the deadline/wake lanes (.9.4); seam .40 asserts registration, commit origin and idle cost |
| src/daemon/transport.rs:67/68 | B2 | Nit | ht-p03.9.2 | 5e237e37 | closed | - | 10 ms AtomicBool sleep-poll. => Notify-backed cancellation replaces the 10 ms sleep-poll |
| W9-1 | B2 | Minor | ht-p03.9.3, ht-p03.42, ht-p03.20 | 699a7a16, 80c01b54 | closed | sw2-claude, sw2-codex, wave28-claude-wake-submission | W9-1 (Minor): pre-send refusals climb the 30 s→300 s ladder; a wake can wait ... => code closed by .9.3 (pre-send refusals retry on backoff, ladder untouched; seam .42 pins refused/verified/unsubmitted outcomes). R20 MET holds only if the cells PASS; early run (native-smoke.md): sw2-claude NOT_EXERCISED, sw2-codex FAIL at the Codex trust screen (S17), wave28 PASS. Final rerun (ht-p03.20, evidence SHA `62949fe5`, docs/validation/report.md): sw2-claude, sw2-codex and wave28-claude-wake-submission PASS; R20 MET |
| src/daemon/transport/service_connection.rs:213 | B2 | Nit | ht-p03.9.2 | 5e237e37 | closed | - | `SessionLease::drop` takes a blocking std Mutex on the current_thread runtime. => `SessionLease::drop` is non-blocking (try_lock, else cancel and `spawn_blocking` the revoke) |
| src/main.rs:52 | B3 | Should-fix | ht-p03.11 | 213c3a48 | closed | - | startup failures to /dev/null. => per-attempt startup log; `ensure` prints its own attempt's tail |
| src/service/workers.rs:625 | B3 | Nit | ht-p03.11, ht-p03.40 | 213c3a48, 3955cb64 | closed | - | lane errors only in an unread ring. => every lane error reaches `daemon.log` rate-limited; seam .40 asserts failure/degraded/clear per lane |
| src/service/workers.rs:599 | B3 | Nit | ht-p03.27, ht-p03.40 | 87e88645, 3955cb64 | closed | - | poisoned mutex reads as Ready. => poisoned status mutex reports Degraded, never Ready |
| src/store/connection.rs:550 | B3 | Nit | ht-p03.10 | d4759fe4 | closed | - | unlisted SQLite errors → StoreCorrupt. => four-way taxonomy; unlisted SQLite errors map to Transient-with-detail, never Corrupt |
| src/daemon/lifecycle.rs:181 | B3 | Nit | ht-p03.10 | d4759fe4 | closed | - | skew becomes a timeout; old CLI gets a decode error (`deny_unknown_fields`). => skew detected from the handshake before decode; skew-tolerant `daemon stop` makes the remedy work |
| src/daemon/lifecycle.rs:273 | B3 | Nit | ht-p03.10, ht-p03.46, ht-p03.127 | d4759fe4, 6c2da337, 17c9a12b | closed | - | "inspect logs" with no path. => remedy() names the log path; seam .46 pins remedy()/log-path goldens; ht-p03.127 routed the remaining doctor and Health pointers through remedy() |
| src/cli/exit.rs:21 | B3 | Nit | ht-p03.107, ht-p03.46 | 5ffa60bf, 6c2da337 | closed | - | exit-3 help says ensure where stop is needed. => exit-3 text depends on error class (unavailable: ensure; skew: stop then ensure); seam .46 runs a real exit-3 command |
| W5-3 | B3 | Minor | ht-p03.27 | 87e88645 | closed | - | W5-3 (Minor): `transitions_refused` never read; `last_reconciliation_at` adva... => `transitions_refused` surfaced; `last_reconciliation_at` advances only on success |
| Wave 26#1 | B3 | Minor | ht-p03.27 | 87e88645 | closed | - | Wave 26 (Minor): `observe_harness_admissions` spawn result discarded. => `observe_harness_admissions` spawn result checked, logged and surfaced |
| Wave 26#2 | B3 | Minor | ht-p03.27 | 87e88645 | closed | - | Wave 26 (Minor): `record_tick` counts passes that carry a `work_error`. => `record_tick` counts only passes without a `work_error` |
| Wave 26#3 | B3 | Minor | ht-p03.27 | 87e88645 | closed | - | Wave 26 (Minor): doctor labels Unsupported as "(cooperative mode)". => doctor labels only Cooperative as cooperative; Unsupported/Refused print their reason |
| Wave 16 | B3 | Minor | ht-p03.27 | 87e88645 | closed | - | Wave 16 (Minor): Health's 16-line cap has one line of headroom and no test. => Health line budget test (worst case 12 of 16 lines) with per-lane folding |
| Wave 25 | B3 | Minor | ht-p03.10 | d4759fe4 | closed | - | Wave 25 (Minor): stale Herdr socket file reported as connect failure, not "se... => stale socket classified Unavailable ('server not running'); config-smoke.md records stale-socket versus never-started wording |
| src/ports.rs:1292 | B4 | Should-fix | ht-p03.2 | 5929a12d | closed | - | / sweep B1 (Should-fix): `allocate_seat`, `revoke_registration`, `CallerVerif... => verification layer deleted (allocate_seat, revoke_registration, CallerVerifier, MutationPermit::new, plus P10/W5-1 per the B5 owner); acceptance rg returns nothing in src/ and tests/ |
| src/ports.rs:2150 | B4 | Nit | ht-p03.3 | 9b779872 | closed | - | StorePort Unsupported defaults. => StorePort has no Unsupported default bodies |
| src/identity/reconcile.rs:24 | B4 | Nit | ht-p03.3 | 9b779872 | closed | - | forwarding traits. => forwarding traits became direct calls |
| src/scheduler/deadlines.rs:52 | B4 | Nit | ht-p03.3 | 9b779872 | closed | - | fake-friendly defaults. => DeadlinePort fake-friendly defaults removed |
| src/ports.rs:2192 | B4 | Nit | ht-p03.3 | 9b779872 | closed | - | concrete FairWriter, duplicate `_admitted` entry points. => FairWriter moved out of ports; duplicate `_admitted` entry points folded |
| src/ports.rs:2618 | B4 | Nit | ht-p03.3 | 9b779872 | closed | - | concrete LiveServiceGate. => LiveServiceGate moved to `service/` (`service::live_gate`) |
| src/protocol/results.rs:150 | B4 | Nit | ht-p03.4, ht-p03.22 | 842562ea, b58e7101 | closed | - | no ApiError constructor, ~110 literals. => ErrorClass enum and per-code ApiError constructors (.4); literal sites converted (.22) |
| W6-R5 | B4 | Minor | ht-p03.5 | f7485dd7 | closed | - | W6-R5 (Minor): design docs not updated for rank/self/alias; specs INDEX still... => design docs describe rank/self/alias and the cooperative reality; INDEX statuses updated |
| Wave 20 | B4 | Minor | ht-p03.5 | f7485dd7 | closed | - | Wave 20 (Minor): misleading "local compositions" error now also covers Skill. => the 'local compositions' error now reads 'setup, launch and skill are local compositions without a daemon backend' (`src/cli/commands.rs:153`) |
| Versions without evidence | B6 | Gap | ht-p03.13, ht-p03.14.5, ht-p03.20 | 0554d78a, eb5c59da | closed | claude-manual, claude-managed, codex-manual, codex-managed | Versions without evidence (Gap): Claude 2.1.283/2.1.284, Codex 0.157.1/0.158.... => listed versions get an honest evidence level and canary tier-0 evidence (.13, .14.5); installed-version live evidence and the Listed rows bind to the four core-flow cells. Early run: all four PASS on Claude 2.1.286 / Codex 0.159.3, claude 2.1.287 not exercised. Final rerun (ht-p03.20, evidence SHA `62949fe5`, docs/validation/report.md): the four core-flow cells PASS on Claude 2.1.287 and Codex 0.159.3; both Listed rows kept (claude-managed judged PASS by the coordinator-approved offline re-derivation of its agent-reported version, see report) |
| Sandbox writes | B6 | Gap | ht-p03.38, ht-p03.20 | 55325471 | closed | codex-sandbox-xdg-state | Sandbox writes (Gap): no live run with a non-tmp state dir; probe never exerc... => Codex sandbox run with a non-tmp XDG state dir; early run PASS (native-smoke.md), final evidence owed by the rerun. Final rerun (ht-p03.20, evidence SHA `62949fe5`, docs/validation/report.md): codex-sandbox-xdg-state PASS |
| Wave 25#1 | B6 | Minor | ht-p03.23 | 4e9b7349 | closed | - | Wave 25 (Minor): daemon keeps the Codex version it saw at start. => daemon re-runs admission when the resolved harness binary's path, inode, size or mtime changes |
| Wave 16 | B6 | Minor | ht-p03.23 | 4e9b7349 | closed | - | Wave 16 (Minor): `COOPERATIVE_RECEIPT_LINE` hard-codes versions. => `COOPERATIVE_RECEIPT_LINE` and the setup sandbox constants derive from the recipe tables |
| Wave 26#1 | B6 | Minor | ht-p03.23 | 4e9b7349 | closed | - | Wave 26 (Minor): latent `codex_status` would report Supported if any recipe d... => Supported requires native receipt and Listed admission |
| Wave 26#2 | B6 | Minor | ht-p03.49, ht-p03.48 | 08a805d9, 4b35bd85 | closed | - | Wave 26 (Minor): doctor does not check whether the `claude` on PATH is admitted. => doctor checks the `claude` on PATH; seam .48 drives `doctor --json` Claude admission fields through the canary's t0.admission |
| Wave 17#1 | B6 | Minor | ht-p03.15 | 9394565b | closed | - | Wave 17 (Minor): pre-existing `features.network_proxy` keys widen the allowance. => setup never widens an existing `features.network_proxy` allowance |
| Wave 17#2 | B6 | Minor | ht-p03.28 | a18a6815 | closed | - | Wave 17 (Minor): launch checks the launcher's CLAUDE_CONFIG_DIR/CODEX_HOME, n... => launch checks the CLAUDE_CONFIG_DIR/CODEX_HOME it hands the pane |
| Wave 17#3 | B6 | Minor | ht-p03.15 | 9394565b | closed | - | Wave 17 (Minor): `codex_install` non-atomic (hooks first, then config). => `codex_install` writes temp files and renames config last, rolling hooks back on failure |
| Wave 17#4 | B6 | Minor | ht-p03.15, ht-p03.36 | 9394565b, c1e79a1f | closed | - | Wave 17 (Minor): unsetup after plugin uninstall and symlinked dotfiles undocu... => unsetup-after-uninstall and symlinked dotfiles documented (`docs/install.md`, `docs/operations.md`) |
| Wave 17#5 | B6 | Minor | ht-p03.15 | 9394565b | closed | - | Wave 17 (Minor): quiet gate applies only after a successful parse. => quiet gate evaluated before parse |
| Wave 25#2 | B6 | Minor | ht-p03.15 | 9394565b | closed | - | Wave 25 (Minor): stale `~/.local` state dir chosen when the XDG one is missing. => legacy `~/.local` state dir chosen only when it holds a store and the XDG dir does not; doctor says which |
| Wave 25#3 | B6 | Minor | ht-p03.15 | 9394565b | closed | - | Wave 25 (Minor): `HERDR_SESSION` disables only the socket default; fast path ... => consistent `HERDR_SESSION` handling; fast path checks the plugin is installed |
| Wave 25#4 | B6 | Minor | ht-p03.15, ht-p03.14.2 | 9394565b, e5163332 | closed | - | Wave 25 (Minor): no fresh-install 0.159.3 setup test. => fresh-install setup test on an isolated home (.15); canary tier-0 setup/unsetup per tested version (.14.2) |
| Wave 30 | B6 | Minor | ht-p03.15, ht-p03.32 | 9394565b, 8f772c0d | closed | - | Wave 30 (Minor): v2 manifest refuses after downgrade; `plan_remove` removes h... => `plan_remove` removes only manifest-recorded entries; the v2-manifest downgrade refusal is noted in the v0.1.0 release notes (.32) |
| Wave 20 | B6 | Minor | ht-p03.29 | 3f985475 | closed | - | Wave 20 (Minor): hook hint also fires on SubagentStart; docs say SessionStart... => hook hint fires on SessionStart only |
| P1 | B6 | Minor | ht-p03.15 | 9394565b | closed | - | P1 (Minor): setup always prepends `--no-daemon`. => `--no-daemon` added once and only where absent |
| P2 | B6 | Minor | ht-p03.15 | 9394565b | closed | - | P2 (Minor): `codex_layer_paths` walks up to `/`. => `codex_layer_paths` stops at the project root |
| P29 | B6 | Minor | ht-p03.15 | 9394565b | closed | - | P29 (Minor): misleading moved-binary conflict message. => moved-binary conflict message names both paths and the fix |
| W6-C4 | B6 | Minor | ht-p03.28 | a18a6815 | closed | - | W6-C4 (Minor): no rc-5 early-exit detection for managed Codex. => managed Codex launch detects an rc-5 early exit |
| P19/P20/W6-D5 | B6 | Minor | ht-p03.29 | 3f985475 | closed | - | P19/P20/W6-D5 (Minor): context budget fallbacks (trim order, no fit check, lo... => one context-budget function with a documented trim order and a final fit check |
| W6-R3 | B6 | Minor | ht-p03.29 | 3f985475 | closed | - | W6-R3 (Minor): digest None drops the D2 procedure line. => a `None` digest still emits the D2 procedure line |
| Wave 21 | B6 | Minor | ht-p03.29 | 3f985475 | closed | - | Wave 21 (Minor): a PTY harness (Codex `tty:true`) gets human output. => output mode prefers harness markers over isatty |
| Wave 28#1 | B6 | UX | ht-p03.30, ht-p03.42, ht-p03.20 | 83f69f42, 80c01b54 | closed | wave28-claude-wake-submission | Wave 28 (UX): wake prompt left typed but unsent in a Claude composer. => wake delivery verifies submission and re-sends the submit key once (.30; seam .42). Closed only if the Claude wake-submission cell PASSes on the final SHA; early run PASS (`submitted`, marker once, ACK observed). Final rerun (ht-p03.20, evidence SHA `62949fe5`, docs/validation/report.md): wave28-claude-wake-submission PASS (wake outcome submitted, ACK observed) |
| Wave 31 | B6 | UX | ht-p03.28 | a18a6815 | closed | - | Wave 31 (UX): Herdr shows name "None" for launched agents. => launch passes the agent name to Herdr |
| Wave 28#2 | B6 | UX | ht-p03.28 | a18a6815 | closed | - | Wave 28 (UX): "codex profile didn't get applied" never answered. => launch prints the effective profile/config path; docs answer the question |
| FIX-NOW lane#1 | B6 | Minor | ht-p03.28 | a18a6815 | closed | - | FIX-NOW lane (Minor): install.md:119 list broken; launch cache warm-up undocu... => `install.md` list fixed; launch cache warm-up documented as best effort |
| FIX-NOW lane#2 | B6 | Minor | ht-p03.28 | a18a6815 | closed | - | FIX-NOW lane (Minor): launch.rs:333 `-c`/`-i` refusal also hits positionals. => `-c`/`-i` refusal applies only before `--`/the first positional |
| Linux musl and Intel macOS never compiled | B7 | Gap | ht-p03.16, ht-p03.21 | 9deda571, 55df1b88 | deferred: [[Linux musl compile and Linux Herdr link proof, if no local Linux container]] | - | Linux musl and Intel macOS never compiled (Gap). => CI `build-targets` job compiles all four targets (.16); Intel macOS built locally (.21: PASS, 1m31s); both musl targets NOT_EXERCISED locally (Docker daemon down), so the first CI run after merge is the proof |
| herdr-plugin.toml | B7 | Minor | ht-p03.16, ht-p03.31 | 9deda571, bc1aaae2 | closed | - | `platforms=["macos"]` (Minor): installer exits 0 when linking is refused. => platforms declared `macos`+`linux` (.16); a refused link ends `installed, not linked` with exit 3 (.31) |
| release.yml:134 | B7 | Minor | ht-p03.16 | 9deda571 | closed | - | dispatch on a tag creates a release. => tag push is the only publishing trigger; dispatch uploads workflow artifacts only |
| release.yml | B7 | Minor | ht-p03.16 | 9deda571 | closed | - | re-run fails at `gh release create`. => idempotent publish (draft if absent, --clobber, checksum verify, publish last) |
| release.yml:128/133 | B7 | Nit | ht-p03.16 | 9deda571 | closed | - | actions pinned to mutable tags. => every `uses:` pinned to a full SHA with the tag in a comment |
| install.sh:377 | B7 | Nit | ht-p03.31, ht-p03.125 | bc1aaae2, a8fa611a | closed | - | no upgrade stop while the server is down, then exit 3; message at :385 mislea... => upgrade stops the old daemon with the binary's own `daemon stop` (.31); a failed stop is reported (.125) |
| install.sh#1 | B7 | Minor | ht-p03.31 | bc1aaae2 | closed | - | uninstall without herdr on PATH leaves the registration. => uninstall without herdr on PATH prints the unregister command and exits 3 |
| install.sh#2 | B7 | Minor | ht-p03.31 | bc1aaae2 | closed | - | uninstall runs unsetup with no prompt. => uninstall prompts before unsetup; non-interactive needs `--yes` |
| install.sh#3 | B7 | Minor | ht-p03.31 | bc1aaae2 | closed | - | `herdr_action` splits JSON on "{". => `herdr_action` no longer splits JSON on `{` |
| install.sh#4 | B7 | Minor | ht-p03.31 | bc1aaae2 | closed | - | "atomic" replacement is two renames. => replacement documented as crash-safe, not atomic |
| install.sh#5 | B7 | Minor | ht-p03.31 | bc1aaae2 | closed | - | inconsistent doctor/register hints, even after `--no-herdr`. => one final status and `next_steps()` computed from outcomes |
| install.sh#6 | B7 | Minor | ht-p03.31 | bc1aaae2 | closed | - | setup reminder wrong when no harness is found or one is declined. => setup reminder correct when no harness is found or one is declined |
| tests/release/install_test.sh | B7 | Minor | ht-p03.31 | bc1aaae2 | closed | - | partial setup failure untested. => partial-setup-failure and Herdr-down upgrade cases added |
| docs/release.md:12 | B7 | Nit | ht-p03.32 | 8f772c0d | closed | - | false claim the build is unchanged since d635aca. => false 'unchanged since d635aca' claim dropped; release checklist added |
| P25 | B7 | Minor | ht-p03.16 | 9deda571 | closed | - | P25 (Minor): clippy runs only with `--all-features`. => clippy runs with default features and with `--all-features` |
| Release notes | B7 | Minor | ht-p03.32 | 8f772c0d | closed | - | Release notes (Minor): STATUS constants removed, `inv-` prefix, manifest v2 d... => CHANGELOG v0.1.0 records removed STATUS constants, `inv-` prefix, manifest v2 downgrade refusal |
| README demo and "Try it" polish | B7 | Minor | ht-p03.32 | 8f772c0d | closed | - | README demo and "Try it" polish (Minor). => README refreshed against current output |
| G0/B2 | B8 | Gap | ht-p03.18, ht-p03.38, ht-p03.20 | bcdeaf89, 55325471 | open | codex-manual, codex-managed, claude-manual, claude-managed, codex-no-initial-prompt, claude-no-initial-prompt, children-claude, children-codex, sw2-claude, sw2-codex, codex-tui-children-write-absence, ht910-claude, ht910-codex, p40-crash-fix3, codex-sandbox-xdg-state, wave28-claude-wake-submission | G0/B2 (Gap): stale native evidence; rerun every row on the release SHA. => stale native evidence: the rerun on the final SHA replaces it; hardened validator is .18, early cell run is .38 (not closure evidence). Closes only if every cell PASSes on the final SHA; early run: 10 PASS, 5 Codex TUI cells FAIL at the trust screen (S17), 2 NOT_EXERCISED. Final rerun (ht-p03.20, evidence SHA `62949fe5`, docs/validation/report.md): 15 of 16 cells PASS; codex-no-initial-prompt FAILs at the three-run cap (Codex 0.159.3 TUI runs no SessionStart check-in before the first turn; bd ht-5n6). The former claude-installed-2.1.287 cell is subsumed: every Claude cell ran the pinned 2.1.287 binary |
| B3#1 | B8 | Gap | ht-p03.38, ht-p03.20 | 55325471 | closed | children-claude, children-codex | B3 (Gap): concurrent children NOT_EXERCISED on both harnesses. => concurrent children on both harnesses; early run PASS for both (SK1-SK3). Final rerun (ht-p03.20, evidence SHA `62949fe5`, docs/validation/report.md): children-claude and children-codex PASS |
| B3#2 | B8 | Gap | ht-p03.38, ht-p03.20 | 55325471 | closed | sw2-claude, sw2-codex | B3 (Gap): SW2 coalesced warning wake NOT_EXERCISED live on both harnesses. => SW2 coalesced warning wake; early run sw2-claude NOT_EXERCISED (warning already offered), sw2-codex FAIL (S17). Final rerun (ht-p03.20, evidence SHA `62949fe5`, docs/validation/report.md): sw2-claude and sw2-codex PASS |
| B3#3 | B8 | Gap | ht-p03.38, ht-p03.20 | 55325471 | closed | codex-tui-children-write-absence | B3 (Gap): Codex TUI children write-absence NOT_EXERCISED. => Codex TUI children write-absence; early run FAIL at the Codex trust screen (S17). Final rerun (ht-p03.20, evidence SHA `62949fe5`, docs/validation/report.md): codex-tui-children-write-absence PASS (flaky: attempt 1 NOT_EXERCISED, the model spawned no subagent; attempt 2 PASS) |
| Token overhead | B8 | Gap | ht-p03.20 | - | deferred: [[Token-overhead benchmark harness]] | - | Token overhead (Gap): no benchmark. => the rerun records one measured per-turn injected-context number per harness; no benchmark harness |
| ht-910 | B8 | Gap | ht-p03.38, ht-p03.20 | 55325471 | closed | ht910-claude, ht910-codex | ht-910 (Gap): no live run of a daemon restart while the agent stays in its pane. => daemon restart with the agent in its pane; early run ht910-claude PASS, ht910-codex FAIL (S17). Final rerun (ht-p03.20, evidence SHA `62949fe5`, docs/validation/report.md): ht910-claude and ht910-codex PASS |
| P40 | B8 | Gap | ht-p03.38, ht-p03.20 | 55325471 | closed | p40-crash-fix3 | P40 (Gap): crash fix3 real-host acceptance never run. => crash fix3 real-host acceptance; early run FAIL at the Codex trust screen (S17). Final rerun (ht-p03.20, evidence SHA `62949fe5`, docs/validation/report.md): p40-crash-fix3 PASS |
| Validator false-PASS[1] | B8 | Minor | ht-p03.18 | bcdeaf89 | closed | - | P15 nested `claude -p`/`codex exec` counted as model-issued: negative-corpus fixture with an exact verdict, and the structural fix |
| Validator false-PASS[2] | B8 | Minor | ht-p03.18 | bcdeaf89 | closed | - | P31 env/sudo/exec/backtick wrappers not peeled: negative-corpus fixture with an exact verdict, and the structural fix |
| Validator false-PASS[3] | B8 | Minor | ht-p03.18 | bcdeaf89 | closed | - | W10-E1 SL1 sequence coverage: negative-corpus fixture with an exact verdict, and the structural fix |
| Validator false-PASS[4] | B8 | Minor | ht-p03.18 | bcdeaf89 | closed | - | W10-E6 inflated spawn count: negative-corpus fixture with an exact verdict, and the structural fix |
| Validator false-PASS[5] | B8 | Minor | ht-p03.18 | bcdeaf89 | closed | - | W10-E7 one child counted as two readers: negative-corpus fixture with an exact verdict, and the structural fix |
| Validator labels/false-FAIL[1] | B8 | Minor | ht-p03.18 | bcdeaf89 | closed | - | W6-D3: label/false-FAIL fixture in the negative corpus |
| Validator labels/false-FAIL[2] | B8 | Minor | ht-p03.18 | bcdeaf89 | closed | - | W6-D4: label/false-FAIL fixture in the negative corpus |
| Validator labels/false-FAIL[3] | B8 | Minor | ht-p03.18 | bcdeaf89 | closed | - | W10-E2: label/false-FAIL fixture in the negative corpus |
| Validator labels/false-FAIL[4] | B8 | Minor | ht-p03.18 | bcdeaf89 | closed | - | W10-E3: label/false-FAIL fixture in the negative corpus |
| Validator labels/false-FAIL[5] | B8 | Minor | ht-p03.18 | bcdeaf89 | closed | - | W10-E5: label/false-FAIL fixture in the negative corpus |
| Validator labels/false-FAIL[6] | B8 | Minor | ht-p03.18 | bcdeaf89 | closed | - | W10-E8: label/false-FAIL fixture in the negative corpus |
| W9-5 | B8 | Minor | ht-p03.18 | bcdeaf89 | closed | - | W9-5 (Minor): R13 shell-phase check does not assert an Unsafe row. => R13 shell-phase fixture asserts the Unsafe row |
| FIX-NOW lane[1] | B8 | Minor | ht-p03.18 | bcdeaf89 | closed | - | quoted-literal false positive has a fixture |
| FIX-NOW lane[2] | B8 | Minor | ht-p03.18 | bcdeaf89 | closed | - | native.rs incarnation-recheck test moves server state in the fake host |
| Wave 15 report/claims precision[1] | B8 | Minor | ht-p03.20 | - | open | codex-no-initial-prompt (every cell, G0/B2 rule) | report claims regenerated from the rerun's per-cell results (items are not itemised in the findings doc); open until the report is regenerated on the final SHA. Final rerun (ht-p03.20, evidence SHA `62949fe5`, docs/validation/report.md): report regenerated on the evidence SHA, but the G0/B2 backing rule leaves it open: codex-no-initial-prompt FAILs (bd ht-5n6) |
| Wave 15 report/claims precision[2] | B8 | Minor | ht-p03.20 | - | open | codex-no-initial-prompt (every cell, G0/B2 rule) | report claims regenerated from the rerun's per-cell results (items are not itemised in the findings doc); open until the report is regenerated on the final SHA. Final rerun (ht-p03.20, evidence SHA `62949fe5`, docs/validation/report.md): report regenerated on the evidence SHA, but the G0/B2 backing rule leaves it open: codex-no-initial-prompt FAILs (bd ht-5n6) |
| Wave 15 report/claims precision[3] | B8 | Minor | ht-p03.20 | - | open | codex-no-initial-prompt (every cell, G0/B2 rule) | report claims regenerated from the rerun's per-cell results (items are not itemised in the findings doc); open until the report is regenerated on the final SHA. Final rerun (ht-p03.20, evidence SHA `62949fe5`, docs/validation/report.md): report regenerated on the evidence SHA, but the G0/B2 backing rule leaves it open: codex-no-initial-prompt FAILs (bd ht-5n6) |
| Wave 15 report/claims precision[4] | B8 | Minor | ht-p03.20 | - | open | codex-no-initial-prompt (every cell, G0/B2 rule) | report claims regenerated from the rerun's per-cell results (items are not itemised in the findings doc); open until the report is regenerated on the final SHA. Final rerun (ht-p03.20, evidence SHA `62949fe5`, docs/validation/report.md): report regenerated on the evidence SHA, but the G0/B2 backing rule leaves it open: codex-no-initial-prompt FAILs (bd ht-5n6) |
| Wave 15 report/claims precision[5] | B8 | Minor | ht-p03.20 | - | open | codex-no-initial-prompt (every cell, G0/B2 rule) | report claims regenerated from the rerun's per-cell results (items are not itemised in the findings doc); open until the report is regenerated on the final SHA. Final rerun (ht-p03.20, evidence SHA `62949fe5`, docs/validation/report.md): report regenerated on the evidence SHA, but the G0/B2 backing rule leaves it open: codex-no-initial-prompt FAILs (bd ht-5n6) |
| Wave 15 report/claims precision[6] | B8 | Minor | ht-p03.20 | - | open | codex-no-initial-prompt (every cell, G0/B2 rule) | report claims regenerated from the rerun's per-cell results (items are not itemised in the findings doc); open until the report is regenerated on the final SHA. Final rerun (ht-p03.20, evidence SHA `62949fe5`, docs/validation/report.md): report regenerated on the evidence SHA, but the G0/B2 backing rule leaves it open: codex-no-initial-prompt FAILs (bd ht-5n6) |
| Wave 15 report/claims precision[7] | B8 | Minor | ht-p03.20 | - | open | codex-no-initial-prompt (every cell, G0/B2 rule) | report claims regenerated from the rerun's per-cell results (items are not itemised in the findings doc); open until the report is regenerated on the final SHA. Final rerun (ht-p03.20, evidence SHA `62949fe5`, docs/validation/report.md): report regenerated on the evidence SHA, but the G0/B2 backing rule leaves it open: codex-no-initial-prompt FAILs (bd ht-5n6) |
| Wave 16 sweep status consistency[1] | B8 | Minor | ht-p03.20 | - | closed | sw2-claude, sw2-codex, wave28-claude-wake-submission | R20 sweep status regenerated from the SW2 and Wave 28 cells (expected MET after B2). Final rerun (ht-p03.20, evidence SHA `62949fe5`, docs/validation/report.md): backing cells PASS; R20 regenerated MET in integration-sweep.md |
| Wave 16 sweep status consistency[2] | B8 | Minor | ht-p03.20 | - | closed | p40-crash-fix3, ht910-claude, ht910-codex | R23 (crash/concurrency tests cover production seams) sweep status regenerated from the crash and restart cells (cell choice is this ledger's reading; the spec names no backing cells for R23). Final rerun (ht-p03.20, evidence SHA `62949fe5`, docs/validation/report.md): backing cells PASS; R23 regenerated MET in integration-sweep.md |
| Process debt[1] | B8 | - | ht-p03.19 | d06a25ea | closed | - | P41 (package lifecycle fix3, unenumerated): reviewed and explicitly accepted in review-debt.md section 5; owner: ht-p03.19 (RD-3 filed as ht-p03.123) |
| Process debt[2] | B8 | - | ht-p03.19 | d06a25ea | closed | - | P42 (flaky-probe, unenumerated): files rewritten by B1/B3 beads, explicitly accepted in review-debt.md section 5; owner: ht-p03.19 |
| Process debt[3] | B8 | - | ht-p03.19 | d06a25ea | closed | - | P43 (hook+digest merge, unenumerated): explicitly accepted in review-debt.md section 5; owner: ht-p03.19 |
| Process debt[4] | B8 | - | ht-p03.19 | d06a25ea | closed | - | P44 (Claude 2.1.285 recipe merge, unenumerated): explicitly accepted in review-debt.md section 5; owner: ht-p03.19 |
| Process debt[5] | B8 | - | ht-p03.19, ht-p03.20 | d06a25ea | closed | codex-manual, codex-managed, claude-manual, claude-managed, children-claude, children-codex | P45 degraded verdicts: cooperative-claim PASS labels are re-measured by the hardened-validator rerun; owner: ht-p03.20 (ht-p03.19 section 6 notes the gap). Final rerun (ht-p03.20, evidence SHA `62949fe5`, docs/validation/report.md): all six backing cells PASS under the hardened validator |
| Process debt[6] | B8 | - | ht-p03.19 | d06a25ea | closed | - | P45 goal-vs-tree read-through: agent-written goal-vs-tree table in review-debt.md section 6, flagged for the owner's read at finish (not a human read-through); owner: ht-p03.19, then the repository owner |
| Process debt[7] | B8 | - | ht-p03.19 | d06a25ea | closed | - | regression-pass code never re-reviewed: ht-4is.37 read in full (RD-1, filed ht-p03.121, closed), ht-4is.38 covered by this run's own review; owner: ht-p03.19 |
| Process debt[8] | B8 | - | ht-p03.19 | d06a25ea | closed | - | light-review lanes (user-level setup, short IDs, token diet, human output, release/installer, `read --follow`): non-rewritten remainder read, rewritten code covered by per-task reviews; owner: ht-p03.19 |
| Process debt[9] | B8 | - | ht-p03.19 | d06a25ea | closed | - | 17 DONE_WITH_CONCERNS reports: texts lost (git-ignored), accepted with reason in review-debt.md section 4; native-evidence part re-measured by ht-p03.20; owner: ht-p03.19 |
| Process debt[10] | B8 | - | ht-p03.19 | d06a25ea | deferred: [[The two remainder-capped roast candidates]] | - | 2 remainder-capped roast candidates: content never recorded, closed as unrecoverable; owner: ht-p03.19 (section 7) |
| ci.yml:101 | B9 | Nit | ht-zo4, ht-p03.8, ht-p03.26 | 29a9eabf, 19f76cc1 | deferred: ht-zo4 (non-gating flakiness side quest; user decision 2026-10-02) | - | , ht-4is.11.8, P12, P13 (Nit): serial pin; `hook_entrypoint` exits 101 in par... => hook_entrypoint exit 101 root-caused and fixed (.8: process-wide fd redirect); the CI serial pin stays because the 10-consecutive-unpinned-runs gate was moved to the non-gating side quest ht-zo4 (ht-p03.26 closed partial at 19f76cc1) |
| tests/store/attention.rs:277 | B9 | Nit | ht-p03.6, ht-p03.24 | 627b66fd, e7a61c69 | closed | - | global UNITS counter. => per-test cost counter replaces the global UNITS counter (.6); suite migrated (.24) |
| Named flakes | B9 | Minor | ht-zo4, ht-p03.24 | e7a61c69 | deferred: ht-zo4 (residual parallel-suite flakiness; user decision 2026-10-02) | - | Named flakes (Minor): `concurrent_publishers_receive_distinct_ordered_keys`, ... => the five named tests were rewritten deterministically (.24, 16-thread 20-run evidence); residual parallel-suite flakiness found by .26 continues in ht-zo4 |
| tests/host_adapter.rs:487 | B9 | Nit | ht-p03.24 | e7a61c69 | closed | - | wall-clock slack. => the wall-clock slack assertion was replaced by a skewed clock (`SkewClock`) |
| tests/service/resolution.rs:2001 | B9 | Nit | ht-p03.24 | e7a61c69 | closed | - | sleep then assert absence. => sleep-then-assert-absence rewritten with deterministic synchronization |
| tests/store/attention.rs:481 | B9 | Nit | ht-p03.25 | 659d7e24 | closed | - | recipients backfill untested. => recipients backfill test added |
| src/store/messages.rs:521 | B9 | Nit | ht-p03.25 | 659d7e24 | closed | - | publish-time archived recheck untested. => publish-time archived-recheck test added |
| tests/store/receipts.rs:1455 | B9 | Nit | ht-p03.25 | 659d7e24 | closed | - | asserts only `is_err()`. => asserts the error code, not bare `is_err()` |
| tests/hook_entrypoint.rs:1015 | B9 | Nit | ht-p03.8 | 29a9eabf | closed | - | payload pasted 7 times. => payload pasted 7 times replaced by one `Payload` builder (Post-Implementation Notes) |
| ci.yml:105 | B9 | Nit | ht-p03.26 | 19f76cc1 | closed | - | Python demo suites not in CI. => Python demo/validator suites and `install_test.sh` run in CI |
| P37 | B9 | Minor | ht-p03.6 | 627b66fd | closed | - | P37 (Minor): ownership test depends on fixture path length. => short temp base keeps socket paths under the Unix limit |
| Wave 18 | B9 | Minor | ht-p03.25 | 659d7e24 | closed | - | Wave 18 (Minor): no test for wake suppression or hook-path takeover. => wake-suppression and hook-path takeover tests added |
| Wave 26 | B9 | Minor | ht-p03.25 | 659d7e24 | closed | - | Wave 26 (Minor): sweep does not pin "healthy" in cooperative setup. => `cooperative_setup_sweep_reports_healthy` pins healthy |
| Wave 16 | B9 | Minor | ht-p03.24 | e7a61c69 | closed | - | Wave 16 (Minor): sweep's late-ACK check uses a fixed 2 s sleep; "rejected" mi... => the sweep's fixed 2 s late-ACK sleep removed (`tests/integration/sweep.rs`) |
| Wave 28 | B9 | Minor | ht-p03.25 | 659d7e24 | closed | - | Wave 28 (Minor): latency test never asserts prompts > 0; stderr undrained. => latency test asserts `prompts > 0` |
| Wave 21#1 | B10 | - | ht-p03.17 | 54a43787 | closed | - | Wave 21: inbox drops topic and invitation id. => human inbox keeps topic and invitation id |
| Wave 21#2 | B10 | - | ht-p03.17 | 54a43787 | closed | - | Wave 21: `checked_in` drops warnings and `offered_through`. => `checked_in` keeps the warnings continuation and `offered_through` |
| Wave 21#3 | B10 | - | ht-p03.34 | a88258f4 | closed | - | Wave 21: `write_selected` return-length doc wrong. => `write_selected` return-length doc corrected (`src/cli/output.rs`) |
| Wave 21#4 | B10 | - | ht-p03.33, ht-p03.108 | 3b833b23, 601f4284 | closed | - | Wave 21: `is_unsafe` misses LRM/RLM/Cf; CJK misaligns columns. => shared `escape_for_terminal` covers LRM/RLM/Cf with display-width alignment (.33); setup `scalar()` routed through it (.108) |
| Wave 31#1 | B10 | - | ht-p03.33 | 3b833b23 | closed | - | Wave 31: escaping decided per page, undocumented. => one escaping function replaces per-page escaping |
| Waves 31/27 | B10 | - | ht-p03.33, ht-p03.34 | 3b833b23, a88258f4 | closed | - | Waves 31/27: bidi/zero-width print raw; follow notices and argv unescaped; ir... => bidi/zero-width escaped everywhere (.33); follow notices and argv escaped, irc indentation no longer the only guard (.34) |
| Wave 31#2 | B10 | - | ht-p03.34 | a88258f4 | closed | - | Wave 31: `--offset` fallback is dead code. => dead `--offset` fallback removed |
| Wave 31#3 | B10 | - | ht-p03.17 | 54a43787 | closed | - | Wave 31: already-joined invite prints the generic form. => already-joined invite prints its specific form |
| Wave 29#1 | B10 | - | ht-p03.17 | 54a43787 | closed | - | Wave 29: compact `thread()` drops `topic_detail_argv`. => compact `thread()` keeps `topic_detail_argv` |
| Wave 29#2 | B10 | - | ht-p03.17 | 54a43787 | closed | - | Wave 29: HH:MMZ timestamps have no date. => timestamps older than today carry a date |
| Wave 29#3 | B10 | - | ht-p03.17 | 54a43787 | closed | - | Wave 29: `event_row` drops fields. => `event_row` keeps the dropped fields |
| Wave 20 | B10 | - | ht-p03.34 | a88258f4 | closed | - | Wave 20: SKILL.md overclaims `--json`; digest example misplaced. => SKILL.md no longer overclaims `--json`; digest example moved |
| Wave 18 | B10 | - | ht-p03.34 | a88258f4 | closed | - | Wave 18: pane-name precedence silent; `pane_not_found` gives no hint. => pane-name precedence documented; `pane_not_found` names it |
| Waves 27/28 | B10 | - | ht-p03.34 | a88258f4 | closed | - | Waves 27/28: follow exits on Ctrl-C only between calls; NickCache takes `cont... => one slow connect is Transient; fatal notice printed once; Ctrl-C honoured during a call; NickCache off `context.lock` |
| Wave 25 | B10 | - | ht-p03.34, ht-p03.108 | a88258f4, 601f4284 | closed | - | Wave 25: autodetect can make a command wait up to HERDR_QUERY_TIMEOUT. => autodetect uses a short bounded probe (.34); doc states the 2 s bound (.108) |

## Deferrals

Every deferral cites a bd issue or a row of the root spec's Explicit deferrals table:

| rows | deferred to | reason |
|---|---|---|
| B9 `ci.yml:101`, B9 Named flakes | bd `ht-zo4` | User decision 2026-10-02: flakiness is nice to have and gates neither the run nor the final merge. The `hook_entrypoint` root cause is fixed (ht-p03.8) and the five named tests are rewritten (ht-p03.24), but the 10-consecutive-unpinned-runs gate moved to the non-gating side quest `ht-zo4` (branch `side/flakiness`), so `ci.yml` keeps `-- --test-threads=1` and ht-p03.26 closed partial (`19f76cc1`). |
| B7 Linux musl and Intel macOS never compiled | root spec: "Linux musl compile and Linux Herdr link proof, if no local Linux container" | Intel macOS and both Apple targets built locally (ht-p03.21); both musl targets are NOT_EXERCISED because the Docker daemon was down. The `build-targets` CI job (ht-p03.16) is the proof on the first post-merge run. |
| B8 Token overhead | root spec: "Token-overhead benchmark harness" | The rerun records one measured number per harness; no benchmark tooling. |
| B8 Process debt[10] | root spec: "The two remainder-capped roast candidates" | Content never recorded in the archive tag (`review-debt.md` section 7). |

Review-leaf findings below Should-fix were filed as bd issues instead of deferrals: ht-p03.121 (RD-1), ht-p03.122
(RD-2), ht-p03.123 (RD-3); all three are closed and merged (`ea4692f9`, `82d47c2c`, `2af6cfdd`).

## Process debt owners (B8 "Process debt (10)")

Owner of every item; none is ownerless. Each row's finding column carries the same owner.

| row | item | owner | disposition |
|---|---|---|---|
| Process debt[1]-[4] | P41-P44 unenumerated parked items | ht-p03.19 (`review-debt.md` section 5) | triaged by the code touched; explicit accept, RD-3 filed as ht-p03.123 |
| Process debt[5] | P45 degraded verdicts | ht-p03.20 | cooperative-claim verdicts re-measured with the hardened validator on `62949fe5`; closed |
| Process debt[6] | P45 missing goal-vs-tree read-through | ht-p03.19, then the repository owner | agent-written table (`review-debt.md` section 6), flagged for the owner's read at finish; not a human read-through |
| Process debt[7] | regression-pass code never re-reviewed | ht-p03.19 (section 3) | ht-4is.37 read in full (RD-1 filed as ht-p03.121); ht-4is.38 covered by the run's own review |
| Process debt[8] | light-review lanes | ht-p03.19 (section 1) | non-rewritten remainder read; rewritten code covered by per-task reviews |
| Process debt[9] | 17 DONE_WITH_CONCERNS reports | ht-p03.19 (section 4) | texts lost; accepted with reason; native-evidence part re-measured by ht-p03.20 |
| Process debt[10] | 2 remainder-capped roast candidates | ht-p03.19 (section 7) | unrecoverable; deferred row above |

## Rows ht-p03.20 must confirm

Rows with status `pending ht-p03.20` (every row whose closure depends on a backing cell): B2 W9-1; B6 Versions
without evidence, Sandbox writes, Wave 28#1; B8 G0/B2, B3#1-#3, ht-910, P40, Wave 15[1]-[7], Wave 16 sweep
status consistency[1]-[2], Process debt[5]. Early-run outcomes (`native-smoke.md`, tree `7590dfc5`) that predict
trouble: five Codex TUI cells fail at the folder-trust screen (S17: Codex 0.159.3 prints it despite `-s/-a`, and the
driver refuses to answer it because answering persists trust into `CODEX_HOME`), sw2-claude was NOT_EXERCISED
(the warning was already offered at a verified check-in) and `claude-installed-2.1.287` waits on Optimistic
admission. Unless ht-p03.20 clears those, the rows that name those cells stay `open`; the leaf's report must say so.
R23's backing cells in `Wave 16 sweep status consistency[2]` are this ledger's reading (the spec names none).

**Outcome (ht-p03.20, final code SHA `62949fe5`, three full-matrix runs, the cap).** Every reported cell ran on
`62949fe5` with Claude Code 2.1.287 and Codex 0.159.3 (before/after `--version` and the version the agent reported
agree). 15 of 16 cells PASS (`codex-tui-children-write-absence` PASS (flaky); `claude-managed` PASS by the
coordinator-approved offline re-derivation of its agent-reported version, runner defect bd ht-4p6). Closed: W9-1,
Versions without evidence, Sandbox writes, Wave 28#1, B3#1-#3, ht-910, P40, Wave 16[1]-[2], Process debt[5]. Open:
G0/B2 and Wave 15[1]-[7], each held by `codex-no-initial-prompt` FAIL at the cap (Codex 0.159.3 TUI runs no
SessionStart check-in before the first turn; bd ht-5n6). Both Listed recipe rows (Claude 2.1.287, Codex 0.159.3) stay:
their core-flow cells PASS. Results: `native-rerun.json`, `native-rerun-matrix.log`.

## B5 non-interference

B5 (identity, seat continuity, the cooperative trust edge) is owned by the trust-model session and is frozen for this
run (root spec Non-goals, with the P10/W5-1 carve-out the B5 owner decided and folded into B4). `TRUST-POLICY.md` is
byte-identical to `main` (`git diff main -- TRUST-POLICY.md` is empty; no commit of this run touches it).

### B5 tests on the tip

Tip `70ef9208`, run in the task worktree (`herdr-threads` at the same tree) after the run branch absorbed `main`
(`eccfc030`, which brought B5 epic ht-rzi and its follow-ups). Command form:
`TMPDIR=<short dir> nice cargo test --locked --all-features --lib -- <filter> --test-threads=1 < /dev/null`, plus
the one integration target. `tests/identity/reconcile.rs` and `tests/store/cooperative_checkin.rs` are compiled into the
lib (`#[path]` in `src/identity/reconcile.rs` and `src/store/mod.rs`), so the lib filters below run them.

| scope | command filter | result |
|---|---|---|
| continuity: `tests/identity/reconcile.rs` | `--lib -- reconcile` | 46 passed, 0 failed |
| continuity and attribution: `tests/store/cooperative_checkin.rs` | `--lib -- cooperative_checkin` | 36 passed, 0 failed |
| continuity | `--lib -- continuity` | 29 passed, 0 failed |
| trust invariants | `--lib -- trust` | 6 passed, 0 failed |
| attribution | `--lib -- attribution` | 5 passed, 0 failed |
| operator repair (F6 repair path) | `--lib -- repair` | 8 passed, 0 failed |
| operator attribution | `--lib -- operator` | 50 passed, 0 failed |
| cooperative harness/wake/launch guards (A4) | `--lib -- cooperative` | 111 passed, 0 failed |
| trust-policy walking skeleton, F6, expected boot, me init over a reattached seat, launch onto a live agent | `--test integration -- trust_policy` | 12 passed, 0 failed |

`--lib -- me_init` and `-- cli::me` match no tests (0 run): the me-init guards are covered by the integration
tests `trust_policy::me_init_over_reattached_seat_is_refused` and
`trust_policy::pane_agent_observation_reads_the_same_in_me_init_launch_and_diagnostics`, which ran and passed.

Environment note: the first `cooperative` run used `TMPDIR` inside this worktree and 11 tests failed with
`path must be shorter than SUN_LEN` (the worktree path is about 170 bytes and the fake host binds a unix socket under
`TMPDIR`). That is the socket-path-length property B9 P37 describes, not a B5 regression; the rerun with
`TMPDIR=/tmp/ht37` passed 111 of 111 and is the row above. The other filters do not bind a long socket path and
passed under both.

### Incidental touches of B5-owned code in this run

B5-owned paths: `src/identity/`, `src/store/{seats,receipts,control,operator}.rs`, `src/protocol/authority.rs`,
`src/cli/{me,hook,journal}.rs`, launch and wake (`AGENTS.md` trust-policy list). The run commits (pre-merge, `55512edb`
to `ea4692f9`) that touch them, and why semantics are unchanged:

| touch | bead (merge) | files | nature | evidence semantics are unchanged |
|---|---|---|---|---|
| P10 and W5-1 deletions | ht-p03.2 (`5929a12d`, work `141ba02b`) | `identity/reconcile.rs`, `store/{seats,control,receipts,mod}.rs`, `protocol/authority.rs`, `host/native.rs`, `harness/launch.rs` | deletes the EmptyShell/verified-execution branches (`MarkOccupantUnavailable`, `proven_empty_shell_bridge`, Reconfirm-with-execution, Replace, `NativeLaunchCapability::ProvenEmptyShell`) and `decision_fence`, plus the unreachable verification layer | The B5 owner decided these as removals: `TRUST-POLICY.md` C5/A2 and the P10 row ("remove dead paths with the verification-layer removal"); `combine-main-record.md` "B4 vs B5" records that main only threaded arguments through these paths and the deletions stand. `rg 'allocate_seat\|revoke_registration\|CallerVerifier\|MutationPermit::new\|decision_fence\|ProvenEmptyShell\|proven_empty_shell_bridge\|MarkOccupantUnavailable' src tests` returns nothing at the tip (rerun for this ledger). Live continuity is `ReconfirmStructure` only, which is what `reconcile` (46), `cooperative_checkin` (36), `continuity` (29) and `trust_policy` (12) exercise and pass. |
| ports collapse | ht-p03.3 (`9b779872`) | `identity/{reconcile,repair}.rs`, `scheduler/mod.rs`, `ports.rs` | forwarding traits became direct calls; no transition logic edited | same suites green; the collapse changed call shape, not decisions |
| ApiError constructors | ht-p03.4 (`842562ea`), ht-p03.22 (`b58e7101`) | `cli/{me,journal,launch}.rs`, `harness/bridge.rs`, `host/{native,observation}.rs`, `identity/{reconcile,repair}.rs` | mechanical literal-to-constructor conversion (e.g. `cli/me.rs` check-in conflict detail rewrapped with the same text and code) | `operator` (50), `repair` (8), `cooperative` (111), `trust_policy` (12) green; constructors carry the same `ErrorCode` |
| observation lane on the Pacer | ht-p03.9.5 (`c03538e0`), ht-p03.104 (`6902f91f`) | `identity/{reconcile,repair}.rs` | cadence and backoff of the observation lane, one commit per backoff step; no change to what a reconciliation decides | `reconcile` (46), `continuity` (29), `trust_policy::restore_with_every_seat_structurally_reconfirmed_leaves_no_hold` and the resume tests green |
| live-row discovery | ht-p03.12.4 (`7e1af227`), .12.5 (`7633c1ab`), .12.9 (`eb8c00a7`) | `store/seats.rs`, `store/mod.rs` | read-only queries over partial indexes and pending projections (`NOT EXISTS` on `occupant_bindings_current`); no write path | `trust_policy`, `cooperative_checkin` green; B1 cost-flatness and oracle tests (ht-p03.12.x) compare against the old scan |
| caller seat lookup | ht-p03.12.3 (`36c06915`) | `cli/me.rs` | `pane_seat` reuses the invocation's connection (a one-line call-shape change) | `trust_policy::me_init_over_reattached_seat_is_refused` green |
| wake refusal and submission | ht-p03.9.3 (`699a7a16`), ht-p03.30 (`83f69f42`), ht-p03.41 (`82ffaaa0`) | `store/wake.rs`, `scheduler/mod.rs`, `host/native.rs`, `ports.rs` | refused wakes retry on backoff without climbing the ladder; submission verified with one submit-key retry | the A4 wake guards (harness match, refusal before any prompt, incarnation/epoch recheck) are unchanged: `cooperative_wake_refuses_*` tests in `host::native::tests` pass (111 of 111) |
| launch correctness | ht-p03.28 (`a18a6815`) | `cli/launch.rs`, `host/native.rs`, `harness/launch.rs` | config-dir check, rc-5 early exit, agent name, `-c`/`-i` scope | `trust_policy::launch_onto_reattached_seat_with_live_agent_is_refused` green (the A4 launch guard) |
| hook context and quiet gate | ht-p03.29 (`3f985475`), ht-p03.23 (`4e9b7349`) | `cli/hook.rs` | hint on SessionStart only, budget, parse-failure report to the daemon; no attribution rule edited | `cooperative` (111), `attribution` (5) green |
| me-init presentation | ht-p03.17 (`54a43787`) | `cli/{human,irc,mod}.rs`, `protocol/{output,output_compact}.rs`, goldens | presentation contract commits `6004d71f`, `749b3d83`; they do not touch `cli/me.rs` or any me-init rendering (`git log -S'MeInit'` over the run range finds nothing, the golden set has no me-init fixture) | `trust_policy::me_init_over_reattached_seat_is_refused` and the A4 me-init rule tests green |
| validator-driven test edit | ht-p03.18 (`bcdeaf89`) | `host/native.rs` | doc comment of a test helper only | n/a (comment) |
| follow nick harness | ht-p03.34 (`a88258f4`) | `harness/context.rs` | adds a read-only `current_snapshot()` for display paths | no decision path reads it ("Not for decisions" in its doc) |
| operator replay comment | ht-p03.122 (`82d47c2c`) | `store/operator.rs` | comment spacing | n/a (comment) |

Main's B5 work meets the run in the merge `eccfc030` (47 conflict files); the resolution rule was "main's B5 guards are
authoritative where they meet B4/B5-adjacent code" and is recorded hunk by hunk in `combine-main-record.md`.

Open point for the B5 owner (not changed here): `TRUST-POLICY.md` lines 5-6 still call the C5 guard "required, owned
by B4 (`ht-p03.2`)". ht-p03.2 has landed it (acceptance `rg` above), so that sentence can say "implemented".
