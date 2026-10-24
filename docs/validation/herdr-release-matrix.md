# Herdr release matrix

`scripts/herdr-release-smoke` runs every test in `tests/integration/herdr_release_smoke.rs` against each official Herdr release in `tests/herdr-releases.tsv` (each binary checksum-verified, each test on its own private IsolatedHerdr server, a release's tests in parallel, `--jobs` releases at once). The macOS arm64 `herdr-releases` CI job runs it on manual dispatch; the Linux `linux-e2e` job runs it on every push, on x86_64 and aarch64.

There is no provider, model or credential. The Codex agent is a stand-in, `tests/fixtures/herdr-matrix/codex.js` (Python, started in npm Codex's `<interpreter> .../codex.js` shape so Herdr and herdr-threads both recognize it). It draws Codex's composer, footer and terminal title, and it runs the installed `hooks.json` the way Codex does: SessionStart, UserPromptSubmit and PreToolUse with the turn's `turn_id`, Stop and Interrupt, SessionEnd. Every server also has Herdr's own Codex integration installed, as a Herdr user has it. So every binding, delivery and wake in the matrix goes through the real hooks, the real daemon and the release under test. The Claude stand-in only draws Claude's idle screen.

Complete hook injection automatically ACKs requested receipts; a later inbox call need not repeat the body. Core coverage checks the body in the actual hook context and the sender's acknowledged receipt. Wake coverage keeps the small p0 request's wake and in-turn delivery assertions and checks its automatic ACK. Its separate overdue request exceeds the hook's display bound: the hook prints an inbox continuation, the receipt stays pending past its deadline, and a complete text inbox display settles it. No explicit ACK command is needed on these display paths.

The oversized request also spans text inbox pages. The test follows the exact printed `next:` commands, checks that the first partial page leaves the receipt pending, and requires the final body marker before checking acknowledgement.

The stand-in records `ready` after installing raw terminal input and drawing its composer. Launch setup waits for this event as well as SessionStart before submitting test input: SessionStart alone precedes `tty.setraw`, whose input flush could discard the busy test's first prompt.

Wall time: one release is about 130 s, dominated by the per-seat wake spacing of the move test (about 65 s for the others, which run alongside it); the five listed releases take about 4 minutes at `--jobs 3` on an M-series Mac.

Herdr behaviour the move test pins, identical on 0.9.1 through 0.9.5: renames never change ids; a move inside a workspace keeps the pane id; a move to another workspace gives a new pane id with the same terminal id, the running processes keep the old `HERDR_PANE_ID`, and `pane.get`/`pane.current` answer the old id (through any chain of moves) with the pane's current record; ids are never reused within a server. There is no tab-move command.

## Coverage

The 1.0.1 column is the single smoke test that version shipped. The 1.0.2 column names the matrix test that covers the row: core (`herdr_release_smoke`), handoff (`herdr_release_launch_and_handoff`), wake (`herdr_release_wake`), busy (`herdr_release_busy`), move (`herdr_release_move`).

| Feature | Commands | 1.0.1 | 1.0.2 |
| --- | --- | --- | --- |
| Plugin link, startup daemon, host reachability | `daemon health` (startup entry runs `daemon ensure`) | yes | yes, every test |
| Seat resolution in real panes | `seat resolve` | yes | core |
| Person identity in a shell pane | `human me init` | no | every test |
| Agent check-in through its own hooks | SessionStart / UserPromptSubmit / PreToolUse hooks | no (cooperative CLI check-in) | every agent |
| Threads | `thread create`, `thread show`, `thread list`, `participants` | create | core |
| Invitations | `invite`, `accept` (by the agent), `reject` | invite, accept | core |
| Membership lifecycle | `join`, `leave`, `archive`, `reopen` | no | core |
| Priorities | `send -p 0/3/9`, `--nudge` | no | core, busy |
| 1:1 pair, auto-join | `send --seat` | no | core, wake |
| Required ACK and receipts | `send --require-ack`, complete hook injection and text `inbox` ACK on display, `delivery recipients`, `pending-receipts` | yes (people) | core (hook display), wake (hook and text inbox display) |
| Overdue required ACK | `--deadline`, `overdue`; an oversized body remains pending until full display | no | wake |
| Reading never ACKs | `read`, `body`, `search` | no | core |
| Nudge threshold | `nudge-threshold`, `wake-on` alias | no | core, wake |
| Wake (Herdr types into the idle composer) | p0 at T = 0, p3 at T = 3, p0 under Codex's `/side` placeholder; no wake for p1 at T = 0 or p5 at T = 3 | no | wake |
| In-turn delivery | turn-start hook (UserPromptSubmit) and tool-boundary hook (PreToolUse) contexts | no | wake, busy |
| Busy agent never nudged | p0 sent while Herdr reports the agent working | no | busy |
| Launch | `launch --kind codex`, `launch --kind claude` (existing pane) | codex | handoff |
| Handoff, every target | automatic `--new-tab`, `--new-tab --new-thread`, `--new-tab --thread`, `--pane`, `--existing --seat`, each recipient receiving its work through its own hooks | automatic `--new-tab` only | handoff |
| Doctor | `doctor` | no | core |
| Setup and removal | `setup codex` / `setup claude`, `setup-status`, `unsetup` | setup | core, handoff |
| Pane and tab renames, workspace rename | `herdr tab/pane/workspace rename` with live seats: labels follow, identity, delivery and wake unchanged | no | move |
| Moves with a live seated agent and person | `herdr pane move --new-tab` (same id), `--workspace`, `--new-workspace` and a chain of moves (new id, same terminal; the processes keep their old `HERDR_PANE_ID`): seats follow, in-turn delivery, the agent's `inbox`/`send`, the person's inbox and the p0 wake keep working, and `seat resolve` of a just-moved pane never allocates a second seat | no | move |
| Herdr old-pane-id alias (the client heal's premise) | after a cross-workspace move and after a double move, `herdr pane get <old id>` returns the current record (same terminal id, new pane id, through the chain) and an unknown id returns `pane_not_found`; a failure reads `Herdr old-pane-id alias dropped: client heal in ht <version> degrades` | no | move |
| Herdr's own Codex integration alongside the owned hooks | `herdr integration install codex` | no | every test |
| Local recovery and warnings listings | `pending-ops`, `warnings` | no | core |

## Left out on purpose

| Feature | Why it is not in the matrix | Where it is covered |
| --- | --- | --- |
| `skill`, `adapters`, `contract-id`, `harness-version`, `help` | Local only: they never reach the daemon or Herdr | Golden and unit tests in the suite |
| Claude wake and Claude in-turn delivery | Claude is woken and served by its own channel (the mod), not by Herdr typing; a stand-in cannot run Claude's channel | The required real-agent wake e2e (`scripts/wake-e2e.py`, `scripts/wake-e2e-herdr`) |
| Model behaviour (an agent reading, ACKing, accepting on its own) | Needs a real model | The real-agent wake e2e |
| `summary` | Summary jobs are run by model workers; no Herdr surface | Service and summary tests in the suite |
| `service`, `accept-required` | Programmatic service connections; no Herdr surface beyond seats | Service tests in the suite |
| `follow`, `read --follow` | An unbounded stream; `read` is covered | Follow tests in the suite |
| `retry` of unknown outcomes | Needs a fault between a request and its reply | Failpoint tests in the suite |
| Seat repair (`seat` operator subcommands), operator rebind/retire | Destructive operator repair | Integration tests in the suite, on the CI-pinned Herdr only |
| `view`, `cached-check-in`, `diagnostics`, other `delivery` views | Read-only views of daemon state with no Herdr dependency | Suite |
| Native Hermes activation and agy model behavior | The directory smoke uses owned stand-ins; Hermes's source-bound helper fixture checks selected-root/named-profile scope separately | `scripts/agy-gate`, Hermes enrollment smoke and native qualification gates |
| Busy cases beyond one p0 (busy invite, busy ACK request) | One busy case already pins the Herdr-dependent part (working detection, no nudge, tool-boundary delivery) | The real-agent wake e2e busy cases |

## V2 auto ACK repair verification

Verified on 2026-10-16 on macOS arm64, from candidate base `4fc664dc8` plus this test and fixture repair. Command: `HT_LEAK_RUN_ID=8d6d7343-65e1-4b03-8434-b860bf978c61 nice scripts/herdr-release-smoke --jobs 3`. Every listed release and all five cases passed (25 checks); the command exited 0. Both the driver's leak check and the subsequent scoped process/scratch-file check were clean.

| Herdr | Time | Core | Launch and handoff | Wake | Busy | Move |
| --- | --- | --- | --- | --- | --- | --- |
| 0.9.1 | 140.6 s | PASS | PASS | PASS | PASS | PASS |
| 0.9.2 | 143.2 s | PASS | PASS | PASS | PASS | PASS |
| 0.9.3 | 140.3 s | PASS | PASS | PASS | PASS | PASS |
| 0.9.4 | 133.7 s | PASS | PASS | PASS | PASS | PASS |
| 0.9.5 | 136.0 s | PASS | PASS | PASS | PASS | PASS |

All-target/all-feature Clippy with warnings denied, the harness-neutral check, the default-feature guard, formatting and whitespace checks passed. Independent read-only review found no remaining issues. Earlier runs exposed the text inbox page bound and the stand-in startup input-flush race described above; those runs also cleaned up their private servers. Product code and model-visible delivery text are unchanged.

## Handoff directory configuration smoke

`herdr_release_launch_and_handoff` also launches Codex and Claude stand-ins through
the public CLI with distinct scratch profile directories and cwd containing spaces,
single quotes and double quotes. Both automatic and explicit-thread routes assert
the real child's selected profile environment, cwd and unchanged scratch HOME.
The ordinary no-option handoff remains a control. Completed retries from a changed
caller profile environment and cwd must retain the original result without another
tab, child, thread, seat, invitation or ordinary message. A separate executable
public-CLI test holds an owned fake host at a proven pre-start refusal, changes the
caller's profile environment, cwd and native defaults, then verifies the actual
retry child keeps its original profile, cwd and argv in both routing modes.
Lost-reply and removed-directory boundaries remain covered by the focused topology
tests.

Missing/file directory inputs, pane/existing directory options and agy's unsupported
profile selector must produce zero new durable or native effects. agy's cwd-only
new-tab handoff records the actual child's cwd. The separate
`hermes_selected_directory_source_fixture_preserves_named_scope_and_child_inputs`
test runs the source-bound Python inspection fixture, checks the helper's selected
`HERMES_HOME`, cwd and scratch HOME, and resolves the named profile below that root.

These checks measure declared child inputs and their consumption by the owned
stand-ins. Shell startup may change those inputs in a user's setup; the matrix
does not prove native harness consumption, authenticated account selection or
callback/model activation. It uses no credentials, real user configuration,
shared-server lifecycle changes or live model quota. `--case` selects this matrix
row alone; `--logs-dir` retains complete per-release output, and
`HT_HERDR_MATRIX_EVIDENCE` retains child records and private server/daemon logs
before fixture cleanup.

Verified on 2026-10-17 on macOS arm64 with `nice scripts/herdr-release-smoke
--case herdr_release_launch_and_handoff --jobs 3` (scratch cache and retained log
directories supplied), using scoped run
`fa756dca-e2c5-44f9-91c2-48e3114b33c3`. Tested source tree:
`4a0ffd863daf1d8491c270208ab1f147a9a98b10` (stack base `7da6cf2a0` plus the
configuration smoke changes, before adding these result rows). All rows and the
driver's leak check passed; a second scoped process/scratch check was clean.

| Herdr | Launch/handoff directory smoke | Time |
| --- | --- | --- |
| 0.9.1 | PASS | 80.4 s |
| 0.9.2 | PASS | 82.1 s |
| 0.9.3 | PASS | 81.3 s |
| 0.9.4 | PASS | 85.7 s |
| 0.9.5 | PASS | 82.0 s |

The initial 0.9.5 run failed before child creation: the native tab boundary still
rejected every nonempty environment projection (41.8 s, tested source tree
`6a61225ab6dee752dfb6b9e34e70f4a09e4a59d3`). The typed directory-context fix
in `ht-1urq.19` was stacked before the passing matrix. The executable public CLI
suite passed 24 tests, the Hermes fixture suite passed five, and agy's public
checks passed three. All-target/all-feature Clippy completed in 30.61 s; the
harness-neutral, formatting and whitespace checks passed. These are focused
checks; this leaf did not run the full suite or authenticated release e2e.
