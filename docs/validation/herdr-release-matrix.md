# Herdr release matrix

`scripts/herdr-release-smoke` runs every test in `tests/integration/herdr_release_smoke.rs` against each official Herdr release in `tests/herdr-releases.tsv` (each binary checksum-verified, each test on its own private IsolatedHerdr server, a release's tests in parallel, `--jobs` releases at once). The macOS arm64 `herdr-releases` CI job runs it on manual dispatch; the Linux `linux-e2e` job runs it on every push, on x86_64 and aarch64.

There is no provider, model or credential. The Codex agent is a stand-in, `tests/fixtures/herdr-matrix/codex.js` (Python, started in npm Codex's `<interpreter> .../codex.js` shape so Herdr and herdr-threads both recognize it). It draws Codex's composer, footer and terminal title, and it runs the installed `hooks.json` the way Codex does: SessionStart, UserPromptSubmit and PreToolUse with the turn's `turn_id`, Stop and Interrupt, SessionEnd. Every server also has Herdr's own Codex integration installed, as a Herdr user has it. So every binding, delivery and wake in the matrix goes through the real hooks, the real daemon and the release under test. The Claude stand-in only draws Claude's idle screen.

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
| Required ACK and receipts | `send --require-ack`, text `inbox` ACK on display (by the agent), `ack`, `delivery recipients`, `pending-receipts` | yes (people) | core (agent ACKs by display), wake (agent `ack`) |
| Overdue required ACK | `--deadline`, `overdue` | no | wake |
| Reading never ACKs | `read`, `body`, `search` | no | core |
| Nudge threshold | `nudge-threshold`, `wake-on` alias | no | core, wake |
| Wake (Herdr types into the idle composer) | p0 at T = 0, p3 at T = 3; no wake for p1 at T = 0 or p5 at T = 3 | no | wake |
| In-turn delivery | turn-start hook (UserPromptSubmit) and tool-boundary hook (PreToolUse) contexts | no | wake, busy |
| Busy agent never nudged | p0 sent while Herdr reports the agent working | no | busy |
| Launch | `launch --kind codex`, `launch --kind claude` (existing pane) | codex | handoff |
| Handoff, every target | automatic `--new-tab`, `--new-tab --new-thread`, `--new-tab --thread`, `--pane`, `--existing --seat`, each recipient receiving its work through its own hooks | automatic `--new-tab` only | handoff |
| Doctor | `doctor` | no | core |
| Setup and removal | `setup codex` / `setup claude`, `setup-status`, `unsetup` | setup | core, handoff |
| Pane and tab renames, workspace rename | `herdr tab/pane/workspace rename` with live seats: labels follow, identity, delivery and wake unchanged | no | move |
| Moves with a live seated agent and person | `herdr pane move --new-tab` (same id), `--workspace`, `--new-workspace` and a chain of moves (new id, same terminal; the processes keep their old `HERDR_PANE_ID`): seats follow, in-turn delivery, the agent's `inbox`/`send`, the person's inbox and the p0 wake keep working, and `seat resolve` of a just-moved pane never allocates a second seat | no | move |
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
| Hermes and agy launch kinds | Their own gates (`scripts/agy-gate`, the Hermes enrollment smoke) | Those gates |
| Busy cases beyond one p0 (busy invite, busy ACK request) | One busy case already pins the Herdr-dependent part (working detection, no nudge, tool-boundary delivery) | The real-agent wake e2e busy cases |
