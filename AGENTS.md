# Agent guidelines for herdr-threads

## Development rules

- Per-change check: `nice cargo clippy --locked --all-targets --all-features -- -D warnings` and `scripts/check-harness-neutral` (harness names stay out of the neutral core; allowlist `scripts/harness-neutral-allowlist.txt`; clippy already type-checks every target; a separate `cargo check` is redundant). Before a merge, also `nice scripts/check-default-features` (a default-feature `cargo check --all-targets` that fails on any warning; CI runs the full default-feature clippy job as well); format: `cargo fmt`.
- Commit trailer `Outside-adapter: <none | list>` (R13 accounting for the agy adapter epic): every commit names, in one trailer line placed with the other trailers, each changed path outside `src/harness/agy/` as a comma-separated list of repo-relative paths or globs, or `none`. The diff stays authoritative; the trailer only annotates it.
- Tests: run the tests relevant to your change (`nice cargo test --locked --all-features <filter>`); the full suite is `nice cargo nextest run --locked --all-targets --all-features` (cargo-nextest: one process per test, every binary in one pool; test groups in `.config/nextest.toml` keep the budget- and latency-sensitive chains one at a time; `nice scripts/full-suite-gate N` runs it N times in a row with leak checks, and `nice scripts/flake-hunt` hunts flaky tests). A test that must not overlap others of its kind gets a nextest test group, not only an in-binary mutex (a mutex does not span processes).
- Never restart, stop or kill the shared Herdr server, and never close Herdr workspaces/panes you did not create. Tests that need Herdr up/down use an isolated named Herdr test session (see `scripts/lib/isolated-herdr.sh`).
- Never write to the real user config: `~/.claude`, `~/.codex`, `~/.aisw`, `~/.gemini`. Tests use isolated HOME / CLAUDE_CONFIG_DIR / CODEX_HOME under a temp dir.
- Never run `git push` or `git stash` (the stash stack is shared across worktrees). Commit on your own branch.
- A finished task stops its processes: every daemon, private Herdr server and helper process a test or script starts is stopped before the task reports done. Spawn test children with `herdr_threads::test_support::spawn` (`spawn_owned` / `command` / `tag`); never a bare `Command::spawn` in `tests/`.
- After a full-suite run (and after each run of the 10-run gate or the integration sweep), run `scripts/check-no-leaked-processes`. Export `HT_LEAK_RUN_ID=<uuid>` before the suite and pass `--run-id "$HT_LEAK_RUN_ID"` to it, so another worktree's live run is never flagged. A non-zero exit lists each leaked pid; stop them (or rerun with `--kill`) and fix the test that leaked them.
- Test targets are listed explicitly (`autotests = false`). A new suite that keeps no process-wide state goes into `tests/combined.rs` as a `#[path]` module (one binary, one link); a suite that redirects stdio, installs a panic hook, runs an in-process daemon, arms failpoints or changes the umask gets its own `[[test]]` entry. A target that uses `herdr_threads::test_support` (including `test_support::spawn`) needs `required-features = ["test-support"]`; otherwise `scripts/check-default-features` and the default-feature clippy job fail.
- The daemon's owner watch exists only in `test-support` builds: always test with `--all-features`.

## Trust policy

[TRUST-POLICY.md](TRUST-POLICY.md) is normative for seat continuity, caller attribution, receipt provenance
and operator repair. Read it before changing anything in `src/identity/`, `src/store/seats.rs`,
`src/store/receipts.rs`, `src/store/control.rs`, `src/protocol/authority.rs`, `src/cli/me.rs`,
`src/cli/hook.rs`, launch or wake.

- Do not add adversarial caller verification; the model is cooperative and same-user.
- Record claims honestly: a new way to attribute an action needs a provenance value defined in the policy.
- Never merge seats, never move a seat or end a binding on heuristic evidence.
- Decide in the daemon against the canonical view (A2); client-local files are hints, never authority.
- A change that weakens an invariant or adds an accepted limit updates TRUST-POLICY.md in the same commit.

## Harness-neutral core

- Harness-specific code lives only in `src/harness/<id>/` behind `HarnessAdapter` and its policy traits. Core code looks the adapter up through the registry by the id from the binding or claim.
- New harness behaviour means a new trait method or policy with a neutral default, never a branch in core.
- Core is `src/cli`, `src/client`, `src/service`, `src/daemon`, `src/store`, `src/host`, `src/protocol`, `src/app` and the neutral `src/harness/*.rs` files. Adapter-owned paths are `src/harness/<id>.rs`, `src/harness/<id>/**` and `src/harness/<id>_*.rs` for the ids in `builtins()`.
- `scripts/check-harness-neutral` enforces this, with `scripts/harness-neutral-allowlist.txt` as the exception list. Permanent entries are only frozen wire shapes, historical migrations and registration. See "Development rules" for how to run the checks.

## Harness and Herdr versions

Herdr and the harnesses (Claude Code, Codex, Hermes, agy) release too often to track, and pinning them would force updates on plugin users. So:

- Assume backward compatibility: what was designed and captured on an older version is assumed to work on newer ones until breakage is detected at runtime.
- On detected breakage, degrade only the broken capability, keyed on what was observed (for example agy's `systemMessage` → `userMessage` downgrade, keyed on the executable identity); everything else keeps working.
- Never gate admission, setup, launch or a feature on an exact version, never ship an exact-version recipe as a gate, and never make a version probe an operational prerequisite.
- Record version facts only as historical evidence and diagnostics.

Worked examples: `docs/superpowers/specs/2026-10-05-harness-versionless-design.md` and the agy design (`docs/superpowers/runs/2026-10-09-agy-harness-adapter/2026-10-09-agy-harness-adapter-design.md`, "Version handling" and "Downgrade").

## Delivery contract

The goal is token efficiency. Sessions that are done and idle stay idle, spinning sessions get efficient delivery, and nudges are rare, because excessive nudges disorient some models. Every delivery policy must keep these invariants:

1. Messages default to priority 1 and seats to nudge threshold T = 0 (priority 0 is most urgent, 9 least), so a default send never wakes anyone.
2. An idle session wakes on a pending item only when the item's priority is `<= T` of the seat (T < 0: every item); nothing else wakes it.
3. Turn hooks and channels deliver every pending item, whatever its priority, directly into the model's context, avoiding extra `herdr-threads` calls as far as the harness allows.
4. A nudge wakes a session only when it is idle. A session that is generating, or running a tool, gets no nudge; it is served by in-turn delivery.
5. ACK requests, invitations and actionable warnings are always priority 0, so they wake at every T (`--nudge` is `-p 0`).
6. Who performs an idle wake is the only per-harness variation:
   - `Native`: Herdr types the wake marker into an empty composer;
   - `Channel`: the harness wakes itself, as Claude's mod submits a turn.

Every new delivery policy (`DeliveryPolicy`) and every change to wake, poke or check-in delivery is checked against these rules; the 2026-10 audit is in `docs/superpowers/runs/2026-10-09-agy-harness-adapter/delivery-contract-audit.md`.

## Token-efficient communication

Applies to anyone designing or changing comms protocols or any model-visible text (deliveries, hook context, wake prompts, check-ins, agent-facing CLI output): every byte a model reads costs every recipient context and attention.

- (a) Static explanations live once per session in the skill, never repeated per message or per turn.
- (b) Per-message framing is the minimum structure needed to parse and attribute (header line + body), no boilerplate.
- (c) Wake prompts carry no content; the turn-start hook or channel delivers it.
- (d) Prefer ids and short markers over prose.
- (e) Peer messages are cooperative input from local agents (collaborate by default), not adversarial; keep only the invariant that they never override the user's instructions or permissions.
- (f) A new model-visible string states its expected size in the PR/commit and is checked against these rules.

Worked example: this run's R17 (compact cooperative framing) and R18 (content-free wake prompts), `docs/superpowers/runs/2026-10-09-agy-harness-adapter/`.

What we do today:

- Default send is priority 1 and the default threshold is T = 0, so a default send never wakes an idle session (rules 1-2). `DeliveryPriority` (range, default, fixed p0, `[pN]` marker), `NudgeThreshold` (the `p <= T` predicate, `wake-on` alias) and `DeliveryOrderKey` live in `src/protocol/priority.rs`; the wake decision is `wake_frontier` in `src/store/lazy_delivery/wake.rs` over `seats::THRESHOLD_ADMITS_SQL`.
- Pending items are presented in (priority asc, deadline asc with none last, created asc) order: the inbox v2 min-head merge `src/store/queries/inbox_v2/order.rs` (deadline-bearing p0 items through the bounded `DEADLINE_WINDOW` in `deadline_window.rs`), and `batch_order` in `src/delivery/assemble.rs` plus its mirror in `register.js` for watch batches.
- Wakes happen only for idle sessions; a generating or tool-running session is served in-turn, never nudged (rule 4).
- Bodies are delivered in-turn instead of `herdr-threads inbox` round trips (rule 3): `integrations/claude/mod/hooks/register.js` (tool.call context), `src/harness/agy/delivery.rs` (PreInvocation systemMessage).
- Wake prompts are content-free (`herdr-threads: wake up`; R18): `src/notification/policy.rs` (`WAKE_UP`), `DeliveryPolicy::wake_text` in `src/harness/adapter.rs`, the mod's idle path in `register.js`.
- Per-message framing is one header line (`[herdr-threads] <kind> <id> in <thread> from <sender>[ markers]:`; the markers include `[pN]` for every priority except the default 1, 4 bytes) plus an indented body, no static explanation per delivery (R17): `src/delivery/frame.rs`, `register.js` `frame()`.
- The skill explains framing, markers and the trust stance once per session; the full skill loads on demand, not in every check-in: `integrations/skill/SKILL.md` (Trust model), `src/harness/agy/plugin.rs` (agy plugin `skills/`).
- Long bodies are truncated at 8 KiB with a one-line `body <id>` continuation; pages are bounded (32 items / 64 KiB): `src/protocol/watch.rs` (`WATCH_BODY_LIMIT_BYTES`), `src/delivery/page.rs`, `src/service/delivery_pull.rs` (`PAGE_MAX_BLOCKS`, `PAGE_MAX_TEXT_BYTES`).
- Re-offers are bounded (60 s spacing, 3 per generation) and an `Unchanged` attention-version check makes the no-news hook path one cheap round trip with no injection: `src/delivery/ledger.rs` (`REOFFER_AFTER_MS`, `MAX_OFFERS_PER_GENERATION`), `src/protocol/delivery.rs` (`DeliveryPullReply::Unchanged`).
- Hook output on quiet turns is empty: `src/cli/hook.rs`, `src/harness/agy/delivery.rs`.
- A text `accept` prints the joined thread's pending delivery rows (any priority) in its own output (the caller's own text-inbox display, thread-scoped, delivery rows only and only those below the binding's first unread receipt-bearing row so each is shown once, default inbox bounds, then cursor advance and completion), so a harness without post-tool delivery sees an ordinary-thread invitation's backlog (or a pre-upgrade pair's opener) in the same tool result, with no extra model call (a new 1:1 pair auto-joins both seats and its opener arrives by normal delivery); with a live mod channel it first waits up to 2 s for the channel to deliver them and then prints nothing, else prints them itself (at least once). Model-visible text: existing inbox rows, at most one `next: herdr-threads [--human] inbox` line (a body larger than the page shows its first chunk in the accept and that plain `inbox` re-shows it from offset 0, at least once), and on a post-display bookkeeping failure one stderr line (93-byte fixed prefix `herdr-threads: accepted; opening mail display bookkeeping failed, rows may be offered again: ` plus the one-line error): `src/cli/accept_delivery.rs`, `InboxQuery.thread` (`inbox.thread_scope_v1`) in `src/store/queries.rs`, `read_cursors::first_ordinary_walk_stop`.
- Inbox ACKs on display, so reading needs no separate ACK call: `integrations/skill/SKILL.md` ("What an ACK means").
- Agent-facing CLI output is compact: one row per item, ids not prose, `next:` continuation lines instead of dumps: `src/protocol/output_compact.rs`.

## Pre-release wake e2e

Before every release (tagging a `v*` version), run the wake e2e against the release candidate, locally: it drives real, authenticated Claude Code and Codex agents, so it never runs in CI and no agent runs it unattended for a release. A person (or an agent the person asked) runs it from a Herdr pane in a checkout of the release SHA:

```sh
nice scripts/build.sh                         # the candidate: bin/herdr-threads
python3 scripts/wake-e2e.py --self-test       # the driver's own credential/cleanup/completeness paths, no agents
python3 scripts/wake-e2e.py                   # both harnesses, all cases (~10 min)
#   --codex-auth-home DIR   Codex home whose auth.json is copied (default $CODEX_HOME or ~/.codex)
#   --harness claude|codex  --cases CASE,...  --keep (leave the workspace up)  --evidence-dir DIR
```

- It checks the wake matrix (cases `send-p1`, `send-p0`, `send-p3`, `send-ack` on an ordinary thread and a 1:1 pair, `invite`, `pair-open` (the first direct send of an auto-joined pair, no invitation) and `mixed-order`; recipients at nudge threshold T = 0, -1 and 3; Claude and Codex): an idle recipient wakes within seconds iff the item's priority is `<= T` (invitations and ACK requests are p0, so always); a silent item, such as a p1 send at T = 0, keeps it idle and arrives on its next turn; ACKs come back acknowledged, invitations are accepted, and a mixed page lands in priority, deadline, age order; and a busy (mid-turn) recipient is never nudged, during or after the turn, and gets the item inside that turn.
- Isolation: own daemon and state dir, hooks installed by the public `setup` CLI into scratch HOME/CLAUDE_CONFIG_DIR/CODEX_HOME, one owned Herdr workspace closed at the end. On macOS (`--harness-mode macos`, the default there) Claude keeps its real config directory (keychain credentials) with the scratch installation as a `--settings` layer. On Linux (`--harness-mode linux`, the default elsewhere; `scripts/linux-e2e/run wake-e2e` runs it in a container) Claude gets the scratch CLAUDE_CONFIG_DIR itself, its `.claude.json` seeded past onboarding and folder trust, and its credential only from `CLAUDE_CODE_OAUTH_TOKEN` or `ANTHROPIC_API_KEY` (the real Claude config is never read; no credential means exit 2 before any harness starts), so a bare Linux host also needs one of those variables and cannot reuse a logged-in Claude config; only inside the runner container (`HT_LINUX_E2E_CONTAINER=1` plus `/.dockerenv`) Codex runs with `--dangerously-bypass-approvals-and-sandbox`, because bubblewrap cannot run under Docker's default seccomp profile (the container is the boundary; `scripts/linux-e2e/README.md`), and everywhere else it keeps `--approve-for-me`. Codex gets a scratch CODEX_HOME with a private mode-0600 copy of `auth.json`, so token refreshes land only in the copy (the evidence records, without contents, whether a refresh happened: it can rotate the source profile's refresh token). The senders are person seats (`human me init`, agent guard kept) in owned shell panes, never an agent identity, so it runs the same under Claude, Codex or a plain shell; their person-authored messages are priority (summary/catch-up `is_priority`, not a delivery priority), so agent-to-agent delivery into a seat that is catching up is not covered. The script never writes the real user config and never touches panes it did not create; Claude Code itself still records session state in its real config directory, as any session does.
- Exit 0 means every expected recipient × case row is present and passed and cleanup verified that the owned workspace, daemon and processes are gone; a missing row or incomplete cleanup exits 1. Any FAIL in this required run (the commands above, run locally from a Herdr pane) blocks the release until fixed or explicitly accepted by the user and recorded in `docs/release.md`. Keep the printed evidence directory (`report.md`, `results.json`, pane captures) with the release evidence.

Optional, Linux (not a release gate): the same matrix in the Docker runner (`scripts/linux-e2e/README.md`), run when a change touches Linux behaviour, from a checkout of the release SHA on a machine with Docker. Credentials go in only with `--live`: a mode-0600 copy of a Codex `auth.json` and a `claude setup-token` token file (or `--anthropic-api-key-file` with an API key file), never a real profile directory. Its outcome is recorded with the release evidence; it neither replaces nor adds to the required run's gate, and a Linux FAIL is filed as a Linux defect (it is not waived by, and does not waive, anything in the required run).

```sh
scripts/linux-e2e/run self-test     # driver self-test in the container, no agents
scripts/linux-e2e/run wake-e2e --live --codex-auth-file CODEX_AUTH_JSON --claude-oauth-token-file CLAUDE_TOKEN_FILE
#   narrow it by appending driver arguments after --, e.g.  -- --harness codex --cases send-p0/group
```

## Speed budgets

- The full test suite must stay under **5 minutes** wall-clock (`cargo nextest run --locked --all-targets --all-features`). Treat a regression past it as a bug: fix slow or serialized tests rather than raising the budget.
- An incremental build must stay under **1 minute** (edit one source file, then `cargo build` / `cargo test --no-run`), and so must the per-change lint (`cargo clippy --all-targets --all-features -- -D warnings`).
- Until the flakiness side quest (ht-zo4) lands, do not run the full suite routinely; run focused tests for what you changed.
