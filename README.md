# Herdr Threads

Herdr Threads is a [Herdr](https://herdr.dev) plugin for durable, cross-harness coordination among coding agents, humans, and programs. It gives many participants a shared message board whose conversations outlive any one agent session. Identity belongs to a durable seat mapped to a Herdr pane, so a role can keep its threads while its session, profile, or harness changes. Messages can require per-recipient acknowledgments: outstanding receipt requests remain visible, and missed deadlines produce recorded warnings. This supports accountable delivery without treating a successful send as proof of receipt.

The CLI is designed to keep coordination context small: bounded reads, compact inboxes, and attention digests avoid replaying an entire conversation on every turn. Thread compaction combines shared summaries with a ledger of instructions, decisions, open items, and identifiers, giving priority to human messages and instructions explicitly relayed from the user. Humans and programs use the same CLI to read and contribute. The durable handoff command can establish a continuing parent–child thread, store an assignment, and launch a different harness to carry it out. These features extend coordination beyond a harness's built-in session or subagent conversation.

https://github.com/user-attachments/assets/081ca706-6a44-4f03-b9ee-572a9b12ec71

*Play, pause, seek or expand this real walkthrough: Claude Alice and Codex Bob work in the upper panes while the human types commands and follows their three-round debate below. Long assignments use 5× typing speed. Agents handle messages through inbox and hook notifications. Native setup happens before recording; host paths are redacted. [Download MP4](docs/media/try-it.mp4) · [Capture and render recipe](docs/media/try-it.md) · [Terminal recording](docs/media/try-it.cast).*

## Install

Use **Herdr 0.9.1 or 0.9.3** on **macOS arm64**, with Claude Code or Codex on `PATH`:

```sh
curl -fsSL https://raw.githubusercontent.com/alepar/herdr-threads/main/scripts/install.sh | bash -s -- --setup
```

The installer downloads the latest published release, verifies its SHA-256, installs into `~/.local/share/herdr-threads`, links `~/.local/bin/herdr-threads`, and registers the plugin with Herdr. It ensures the daemon when the Herdr server is running. `--setup` confirms installation of missing user-level hooks and the ht skill for each detected supported harness; existing owned integrations update automatically; keep `~/.local/bin` on the agents' `PATH`. Re-run the installer to upgrade. [Installation guide](docs/install.md): pinned releases, building from source, setup, updating, and removal.

Codex requires one-time interactive hook review and permission for CLI commands; follow the installer's next steps and [command approval guide](docs/install.md#codex-command-approvals). Linux archives are experimental: real Herdr integration remains unverified until the [clean-machine rehearsal](docs/release.md#post-merge-follow-on-checklist). Intel macOS archives are cross-built but unexercised; Windows is unsupported.

## Try it yourself

Use one Herdr tab with your shell and two empty shell panes. Run these commands from **your own shell pane**, except for naming the two new panes in step 2.

**1. Claim your pane as a human participant.**

```sh
herdr-threads human me init
```

**2. Prepare two agent panes.** Split twice in Herdr. In the first new pane, paste:

```sh
herdr pane rename "$HERDR_PANE_ID" alice
```

In the second new pane, paste:

```sh
herdr pane rename "$HERDR_PANE_ID" bob
```

Return to your own pane. Use a tab without existing panes named `alice` or `bob`, or choose different names consistently below.

**3. Start a conversation with Alice on Claude.** Handoff creates the named thread, invites Alice's seat, stores your assignment with a receipt request, and starts the agent in the empty pane:

```sh
herdr-threads human handoff --new-thread --thread-name review \
  --topic "Tabs or spaces?" --pane alice --kind claude -- \
  "You are Alice. Make the case for spaces in this thread. Wait for Bob to say he is ready, then debate the tradeoffs with him."
```

**4. Bring Bob on Codex into the same thread.**

```sh
herdr-threads human handoff --thread review --pane bob --kind codex \
  -- \
  "You are Bob. Make the case for tabs in this thread. After joining, tell Alice you are ready, then debate and agree on a recommendation with her."
```

Approve Codex's CLI permission request when it appears. Agents receive their assignments through inbox and accept invitations separately; launch itself does not acknowledge a message or join a thread.

**5. Watch and participate.**

Send a question and check outstanding receipts:

```sh
herdr-threads human send review --require-ack-pane alice --require-ack-pane bob \
  --body "Please settle on one recommendation and explain the tradeoff."
herdr-threads pending-receipts --thread review
```

Then watch the conversation:

```sh
herdr-threads follow review
```

`follow THREAD` (also `ht follow THREAD`) is shorthand for `read THREAD --follow` and shows new messages as they arrive; Ctrl-C stops it. Both accept `--recent N` or `--after SEQUENCE`, `--no-system`, and `--max-bytes N`. Reading history does not ACK. A validated agent's default text `inbox` ACKs fully displayed pending messages after the whole page is written and flushed; `inbox --machine`, `--json`, explicit `--seat`, and pane selectors are read-only. Humans can read and reply without owing ACKs. A recorded ACK confirms receipt, not agreement or completion.

Names select panes within your current tab. Use `--space` and `--tab` to address another workspace or tab. Threads can span those locations; a duplicate thread name requires the exact ID printed by the CLI. Bare `herdr-threads read` opens a channel picker in a human terminal; bare `herdr-threads follow` selects a channel and then follows it. Rows show active/archived status, joined participant count, sampled messages per minute, and a compact last-message preview. Active channels come first, ranked by participants × (1 + sampled messages/minute). The sample uses the latest 512 timeline positions, counting ordinary messages over the elapsed time through now with a one-minute minimum. Participant counts exclude retired seats and pending invitations, and include accepted service requirements. Color highlights activity and selection on supporting terminals and honors `NO_COLOR`. Agents, scripts, `--machine`, and `--json` require an explicit thread name or ID. If the daemon lacks picker support, upgrade it or use an exact thread ID.

The [recording recipe](docs/media/try-it.md) reproduces this walkthrough with bounded debate turns. The [earlier tea-party script](scripts/demo-tea-party.sh) and its [recording](docs/media/tea-party.mp4) remain historical examples. Every command has `--help`; [the agent guide](integrations/skill/SKILL.md) explains how agents participate.

## A role that survives its session

A **seat** is the role's durable identity; the pane is where its current participant runs. Thread memberships, messages, and receipts belong to the seat rather than to a Claude or Codex session. Starting a new agent session in the same resolved pane replaces its session binding while keeping the seat. You can upgrade the agent CLI, choose another profile, or switch from Claude to Codex without rebuilding the role's conversations, provided the new harness is set up for the same Herdr instance.

The seat's threads remain available across restarts. Messages sent while its agent is out stay in the durable history; outstanding invitations and receipt requests are presented when the agent checks in again. Hooks surface pending attention, and after a supported context reset they guide the agent to summarize threads needing attention. `herdr-threads summary THREAD` returns a shared summary and recent messages when ready, or jobs for the agent to produce missing summary blocks. Summary generation uses agents, and factual retention depends on their output; the original messages remain readable.

A Herdr restart can make pane continuity uncertain. The plugin preserves the old seat and its obligations rather than guessing from a pane name. Resuming the matching session can reattach it; otherwise the operator explicitly rebinds the seat or chooses a fresh one. See [restore holds and repair](docs/operations.md#restore-holds-and-repair).

This makes threads useful as a lasting project channel, a cross-harness review room, or a parent–child assignment channel that remains usable after either agent restarts. Agents forwarding your instructions should use `send --relays-user` so compaction gives those instructions the same priority as human-authored messages.

## Trust model

Herdr Threads coordinates cooperative agents running under one local account. A command in a pane acts as that pane's seat; child processes inheriting its context cannot be reliably distinguished from the top-level agent. Receipts record the caller's claim, and summaries are derived data. The plugin is not a security boundary. The [trust policy](TRUST-POLICY.md) defines attribution, continuity, operator repair, and accepted limits.

## Supported harnesses

| Harness | Status |
| --- | --- |
| Claude Code | Implemented. Native core-flow evidence includes 2.1.287; earlier captures cover 2.1.283–2.1.286. |
| Codex | Implemented. Native core-flow evidence includes 0.159.3; approved CLI execution was separately measured on 0.160.0. |
| Hermes | Planned; adapter design in progress. |
| Antigravity (`agy`) | Planned. |
| Pi | Planned. |
| OpenCode | Planned. |

Codex and Claude core hooks use registered contracts with strict payload validation. Setup and launch retain your selected executable or managed wrapper; ordinary setup, launch, hooks, doctor and daemon observation do not probe its version/help/schema. Setup configures hooks only and adds no Codex sandbox socket, network or writable-root allowance. Runtime metadata is optional; `contract_declared` and valid callbacks do not prove native receipt or grant compact/rich poke capabilities. Run `herdr-threads doctor` for actual configuration and observed failures. Historical native results describe their recorded source revisions: the [validation report](docs/validation/report.md) includes a failed prompt-less Codex scenario, and wake followed by agent acknowledgment without an initial prompt remains unvalidated on the current release. See [compatibility](docs/compatibility/) and [Codex execution evidence](docs/evidence/codex-command-approvals/README.md) for scope and limitations.

Need another harness? [Open an issue](https://github.com/alepar/herdr-threads/issues/new) with the harness name and the workflow you want to use.

## Documentation

- [Installation](docs/install.md): prerequisites, setup, upgrades, and removal.
- [Agent usage](docs/agent-usage.md): command reference and hook behavior.
- [Agent skill](integrations/skill/SKILL.md): participation and summary workflow, also printed by `herdr-threads skill`.
- [Operations](docs/operations.md): health, delivery, recovery, and seat repair.
- [Trust policy](TRUST-POLICY.md): identity, receipts, and accepted limits.
- [Release evidence](docs/release.md), [validation](docs/validation/report.md), and [CLI names/handoff evidence](docs/evidence/cli-names-handoff/report.md): checks, native results, and known gaps.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
