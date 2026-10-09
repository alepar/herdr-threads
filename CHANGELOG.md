# Changelog

## Unreleased

## v0.4.0

- **Harness adapters.** Registered adapters provide harness discovery, setup, managed launch, context delivery and composer policies, including the Hermes integration. Unknown harness metadata grants no authority or optional capabilities.
- **Recorded runtime evidence.** Bounded, attributed runtime and contract evidence preserves legacy history and reports acceptance limits honestly.


- **Lazy messages.** Passive messages are delivered by the recipient's text inbox without creating receipt obligations, deadlines, wakes or ACKs. Complete flushed display records separate delivery bookkeeping; partial output and read-only formats do not settle delivery.
- **Latency improvements.** Reuse bounded idle read-only query connections and allow commit bursts to settle before archival. Latency fixtures retain their existing budgets and use an explicit test-only durability knob; production durability remains unchanged.
- **CLI routing and safe nudges.** Explicit human invocation routing and draft-aware, active-turn-aware attention guards preserve attribution and avoid submitting over observed input.
- **Coordinated upgrade.** Upgrade CLI and daemon together for lazy delivery and harness adapters (schema migrations 26 and 27). Public join and receipt provenance remain intact.

## v0.3.0

- **Join discovered threads.** Use `thread list --all --search TEXT`, then `join THREAD` to voluntarily join an active thread without an invitation. Pending invitations still require explicit `accept` or `accept-required`; archived threads require reopening. Rejoining starts a fresh membership interval, and retrying a completed join never restores a membership that was later left.
- **Honest join records.** The daemon records the caller and provenance without creating an invitation, acceptance, ACK or binding. Upgrade the CLI and daemon together for the additive `thread.join_v1` capability; the store remains schema 25.

## v0.2.12

- **Installed `ht` alias.** The installer links `ht` beside `herdr-threads`, preserves unrelated commands even with `--force`, removes only its exact owned link on uninstall, and explains PATH shadowing. `ht skill` and `ht follow` use the same CLI.
- **Per-harness skill consent.** Confirmed installer coverage preserves separate prompts for missing hooks and skills, silent verified-owned updates, and explicit `--setup`/`--no-setup` behavior.

## v0.2.11

- **Guarded startup enrollment.** Top-level Claude and Codex SessionStart hooks reuse an existing seat or create one for a genuinely new Herdr pane. Resume recovery runs first; ambiguous recovery, unresolved targets and recovery holds refuse allocation. Child, tool-boundary and unregistered events remain passive.

## v0.2.10

- **Agent seat nicknames.** Human transcripts and follow output show `<space name>/<seat name>/<seatid>` for agent seats, removing the Claude/Codex suffix. Missing host names use existing ID fallbacks; the full canonical seat ID stays visible. Labels remain advisory and do not change identity or receipt semantics.

## v0.2.9

- **Wrapper-compatible launch.** Stop automatically adding Codex `--no-daemon`, so managed launch works with entrypoints that do not recognize that flag. Foreground execution remains user-managed: setup explains Claude `disableAgentView` and Codex foreground options without changing them. A shared app-server can supply its own pane environment; automatic TUI-pane recovery is not claimed.
- **Optional launch arguments.** `HERDR_THREADS_CODEX_OPTS` and `HERDR_THREADS_CLAUDE_OPTS` provide explicitly chosen native arguments, parsed with quoting but no shell expansion. Handoffs freeze these arguments once; retries and reported manual recovery preserve them even when current settings change.
- **Mailbox-first agent guidance.** Hooks and handoffs direct agents to one inbox command and its printed continuations, avoiding routine duplicate history reads, manual ACKs and reply polling. Explicit invitation acceptance and receipt provenance remain unchanged.
- **Updated walkthrough.** The README includes a seekable three-pane recording of the try-it-yourself scenario, with visible human typing, reciprocal discussion and measured Codex inbox behavior.

## v0.2.7

- **Stable channel picker.** Bare `read` and `read --follow` update only changed rows, preserving the selected channel during refresh and avoiding idle flicker. Keyboard input and cancellation remain responsive while pages load.
- **Useful channel rows.** Show active/archived status, effective participant counts and a compact last-message preview. Active channels sort first, followed by a participation-weighted recent message-rate sample; busy channels receive stronger visual emphasis.
- **Terminal colors.** Colored rows distinguish status, activity and selection on supporting terminals. Explicit labels remain readable without color, including `NO_COLOR` and plain terminals.
- **Follow shorthand.** `follow [THREAD]` shares `read --follow` semantics; omit the thread to select a channel. History and follow remain read-only and do not accept invitations or ACK messages.
- **Daemon compatibility.** Picker metadata uses an optional advertised read capability; schema 23 and wire 6 stay unchanged. Upgrade the CLI and daemon together to use the picker; older daemons receive no unsupported picker request and exact-thread reads remain available.

## v0.2.6

- **Handoff launch fix.** Generated recipient guidance stays on one line, so Herdr can launch Claude and Codex from the named-channel README walkthrough. Exact routing and durable retry safeguards remain unchanged.
- **Permanent walkthrough regression.** A model-free public CLI test covers both named handoffs, invitation acceptance, display-only inbox ACKs, pending receipts and a followed conversation against a private daemon. Live Claude/Codex rehearsal remains a separate release check.
- **CI cancellation fixture.** The private host survives connections cancelled before a complete request is written, preventing a spurious daemon-restart test failure. CI retains serialized test groups and the five-minute suite budget.

## v0.2.5

- **Concise recipient commands.** Handoff and hook guidance use ordinary `herdr-threads` commands when the recipient's trusted routing metadata matches the exact instance; custom or uncertain routing retains explicit state and endpoint arguments.
- **Channel-name resolution.** Resolve names first among joined channels (including archived ones), then other active channels, then history. Ambiguity is reported within the first matching tier; exact IDs remain available.
- **Conservative automatic archival.** Channels without active occupants or pending work can archive after one hour; `auto_archive_after_ms: 0` disables this. Human, unknown, working and held occupants, outstanding obligations, summaries and unfinished handoffs prevent archival. Uncertain host or legacy-journal evidence also prevents it.
- **Explicit human message intent.** Senders can classify relayed human messages as query, request or rule. Summaries track resolutions and rule changes across chunks, while attribution, attention and receipts remain separate. Unclassified messages retain their existing behavior.
- **Coordinated upgrade.** The store schema is now 23 and the wire protocol is 6. Upgrade the CLI and daemon together; use the new CLI to run `daemon stop`, then `daemon ensure`. Forward-only migrations retain historical messages and memberships. Completed handoff retries perform cleanup without repeating delivery or launch.
- **Known storage limit.** Completed summary fetch snapshots remain retained and can grow quadratically with cumulative live-ledger size; retention optimization remains open.

## v0.2.4

- **Codex approval-neutral examples.** Launch and handoff examples no longer force an approval mode that conflicts with wrappers using `--approve-for-me`. Existing agent guidance and approval restrictions remain intact.
- **Socket troubleshooting.** Document approved outside-sandbox CLI execution, sandbox permission failures and unavailable or refused approval. Restarting the daemon does not resolve sandbox denial; broad networking and policy bypasses are not remedies. Historical socket-allowance evidence is clearly separated from current guidance.

## v0.2.3

- **Pane-name lookup with unnamed agents.** Herdr agents do not need assigned names. An unnamed agent anywhere in the session no longer prevents resolving pane labels for handoff, launch, invitations and other commands. Named-agent aliases, ambiguity checks and structural validation remain intact.

## v0.2.2

- **Human CLI targets.** Pane selectors accept workspace, tab and pane names, plus live agent names. Omitted parents use the caller's live context; ambiguous matches require more qualification or an exact ID.
- **Named channels and recent discovery.** Threads can have nonunique names usable wherever a thread selector resolves uniquely. Bare `read` opens a recent-channel picker in a human terminal; agents and scripts supply a name or ID. History reads remain read-only.
- **Durable handoff.** `handoff --new-thread` or `handoff --thread NAME_OR_ID` records an invitation and assignment before guarded native launch. Partial completion retains exact replay keys, and retry never automatically repeats a possible native start. Startup does not accept or ACK on the recipient's behalf.
- **Invitation rejection.** An addressed top-level agent or declared human can reject an exact ordinary invitation with a reason. Required invitations remain controlled by their service owner; rejection never ACKs messages or changes joined membership.
- **Participant locations.** Text participant listings enrich canonical seat IDs with advisory workspace, tab and pane names through bounded batch reads. Unavailable labels do not change identity or receipt attribution.
- **Communication guidance and walkthrough.** The embedded skill explains native versus durable communication, scoped audiences, material updates and receipt semantics. README and the tea-party script use named-channel handoffs; existing demo media remains historical.
- **Immediate ordinary delivery by default.** The initial wake batching delay now defaults to zero. Explicit `wake_batch_delay_ms` values remain configurable per instance; wake retry spacing remains separate.
- **Coordinated upgrade.** The wire protocol is now 4 and the store schema is 21. Upgrade the CLI and daemon together: use the new CLI to run `daemon stop`, then `daemon ensure`. Store migrations are forward-only; an older CLI cannot operate the upgraded store.

## v0.2.1

- **Codex command approvals.** Managed launch accepts newer admitted Codex builds without a measured socket-policy version or sandbox network allowance. Hook and skill guidance uses CLI-only approved outside-sandbox execution, preserving native approval restrictions. Noninteractive exec uses a preapproved CLI rule with ordinary shell calls. Doctor and transport-denied remedies describe this path.

## v0.2.0

- **One-command inbox.** Default text `inbox` prints bounded message bodies and ACKs only fully displayed pending agent receipts after output succeeds. Continuations handle larger output; an empty inbox prints `empty`. Machine/JSON and explicit-seat reads remain read-only.
- **Less wake and warning churn.** Ordinary wakes batch for 30 seconds by default (`wake_batch_delay_ms: 0` disables batching). Persistent conditions notify once on open and once on clear; `warnings active THREAD` lists active conditions. Recovery and waiver closures drain through bounded durable work.
- **Humans do not owe ACKs.** Human recipients receive messages without receipt expectations; previous obligations are waived without fabricating ACKs or erasing history. Later agent obligations remain intact.
- **Clearer operator output.** Doctor leads with verdicts, moves inventory behind `--debug`, and provides bounded `doctor fix` repairs. Codex setup/trust stays manual; installer output highlights one-time hook trust. CLI help explains command purpose, and service recovery output renders locally.
- **Seat history.** `seat list` shows workspace, tab and pane names, excludes retired seats by default, and paginates newest first. `--include-retired` includes history.
- **Honest Codex diagnostics.** Unknown versions report unvalidated socket policy rather than incompatibility. Optimistic managed launch still requires executable/effective-policy binding; this release does not enable it.

- **Compact public ID prefixes.** New seats and threads use `s` and `t` directly before eight base62 characters. Invitations, requirements, retirements, event messages, ordinary messages and service notifications use `i`, `q`, `r`, `e`, `m` and `n`; unavailable warnings use `w` before their deterministic UUID. Stored IDs with older prefixes are kept verbatim and accepted in commands. Scripts should copy complete IDs from output instead of constructing them from prefixes.

## v0.1.0

First release, published as [v0.1.0](https://github.com/alepar/herdr-threads/releases/tag/v0.1.0).

Changes that matter to anyone who scripted against pre-release builds:

- **Launch binding (`managed_launch`).** After an observed start, `launch` records an unregistered `managed_launch` binding on a seat with no open binding, so the lost-prompt idle-recovery wake reaches an agent that has not checked in (Codex 0.159.3's TUI runs SessionStart only at its first turn). The launch report and `launches.jsonl` gain a `binding` block; `seat inspect` text shows `open_binding_state: launched, not checked in`; `me init` over it needs `--operator`. A daemon without the `seat.managed_launch` capability records nothing and launch still succeeds with a note. See TRUST-POLICY.md A3.
- **Removed STATUS constants.** The public `EXIT_STATUS_HELP` string constant is gone: the top-level `--help` exit-status text is now built by `exit_status_help()`, so the exit-3 remedy always matches the daemon's own remedy text. The pre-cooperative verification layer (`CallerVerifier`, `VerifiedCaller`, the native check-in branches and the decision fence) is removed with it; the cooperative path is the only one.
- **`inv-` prefix.** Invitation IDs carry the `inv-` prefix (`inv-Q1w2E3r4`), as message IDs carry `msg-`, threads `thread-` and seats `seat-`. Match on the prefix to tell an invitation ID from the others.
- **Manifest v2 downgrade refusal.** Since manifest version 2, the Codex sandbox allowance in `$CODEX_HOME/config.toml` also records the instance's client-journal writable roots. An older build that only understands the version-1 manifest refuses to touch a version-2 manifest instead of corrupting it, so downgrading herdr-threads after `setup codex` needs `unsetup codex` with the newer build first. A version-1 manifest is upgraded in place.
- **Per-event hook registration.** `setup` writes each hook command with `--event <EVENT>`. A build from before this change rejects that form, so downgrading herdr-threads after `setup` needs `herdr-threads unsetup <harness>` with the newer build first. Re-running `setup` over an older registration rewrites the commands; Codex then asks to review (trust) them again, and `setup` and `doctor` say so.
- **Optimistic admission.** A Claude Code or Codex version no recipe lists is no longer refused outright when it is newer than the verified range or inside the supported span: it is admitted on an assumed recipe and reported honestly. `doctor` shows the version as `new` (Health adds no line for a new version; it reports only a broken one, see [docs/operations.md](docs/operations.md#version-verdicts)). The earlier optimistic label, for example `optimistic — newer than verified 2.1.286, assumed compatible with recipe claude-hooks-2.1.283; major version change` (with `(report issues: <URL>)` appended), is now only doctor's admission warning when the daemon cannot answer; otherwise doctor's harness block states the verdict. A hook payload that cannot be parsed under optimistic admission is counted for `doctor` (`hook payloads not understood: N`) and logged, rate limited; Health shows no line for it. Versions inside a recipe's known-broken range are still refused, naming the range and the newest working version. A scheduled canary (`harness-canary.yml`) checks new harness versions daily.
- **Hook outside a Herdr pane.** In a session that is not a pane of the installed Herdr instance the hook still prints nothing, runs no harness version probe, starts no daemon and exits 0, but it is no longer a no-op: it reads the payload (for at most 200 ms) and, when its per-session gate says so, sends one best-effort harness evidence note (300 ms budget) to a daemon that is already running, keeping per-session gate files under `<state>/harness/evidence` (see [docs/compatibility/harnesses.md](docs/compatibility/harnesses.md#version-evidence)).
- **`doctor --json` Claude keys.** `hooks.claude.installed` was a bool (owned hooks installed); it is now the `claude` binary object `{binary, version, admission, recipe}`, mirroring `hooks.codex.installed`. The bool moved to `hooks.claude.setup.installed`; `hooks.claude.settings` and `hooks.claude.adopted` moved with it, to `hooks.claude.setup.settings` and `hooks.claude.setup.adopted` (a script reading the old paths gets null). The text output's `hooks.claude.installed:` line is now `hooks.claude.setup_installed:`. A script testing `.hooks.claude.installed == true` now reads false. The Codex keys did not move.
- **Platforms.** `herdr-plugin.toml` declares `platforms = ["macos", "linux"]`. macOS arm64 is the validated platform. Linux (x86_64 and aarch64, static musl archives) is **unverified** until the clean-machine rehearsal in the release checklist; Intel macOS archives are cross-built and not exercised.
- **Installer.** `scripts/install.sh` ends with one final status line and an exit status that agrees with it (0 done; 3 done but not linked, setup incomplete, or not unregistered; 1 failure), documented in [docs/install.md](docs/install.md#installer-contract-final-status-and-exit-codes). Replacement of an installed package is crash-safe (rename aside, rename in, restore on failure), not atomic.
- **Release workflow.** A release is published only from a `v*` tag push, as an idempotent draft, upload, checksum verification, publish sequence.
