# Outside a Herdr pane

herdr-threads also runs where there is no Herdr pane: a named process (on a machine with or without Herdr) and a herdr-core terminal (`hc run`, or a pane of the Herdr desktop app). Each gets a durable seat and messages like a pane seat: send, inbox, ACK, invitations, turn-hook delivery. What differs is how a seat is found and whether an idle agent can be woken.

## Install

Install as usual ([Installation](install.md)). On a machine without Herdr, the installer skips the plugin registration and still runs `setup`, so the harness hooks are in place; the store lives in the default state directory, `$XDG_STATE_HOME/herdr/plugins/herdr-threads` (else `~/.local/state/...`). After upgrading from 2.8, restart the daemon (`herdr-threads daemon stop && herdr-threads daemon ensure`): an older daemon refuses named and terminal seats (`daemon predates named/terminal seats; restart it`).

## Named seats

Start the agent with a name:

```sh
HERDR_THREADS_SEAT_NAME=reviewer claude
```

- The name is the seat. The first command or hook under it creates the seat (`created seat <seat> (reviewer)`); every later session under the same name, on this node, is the same seat, with its history and obligations. Other nodes address it as `reviewer@MACHINE`.
- Names: 1-64 bytes of `A-Za-z0-9._-`, not starting with `-` or `.`. A typo is a new seat, not an error.
- One holder at a time: a second, unrelated process under a name that a live process holds is refused. Subagents and other processes of the holder's own process tree share it.
- A named seat is never retired by absence. When the agent exits, the seat shows offline and keeps its mail. End it for good with `herdr-threads seat retire --self` from the seat (`--force` when another connected session holds it), or `herdr-threads human seat retire SEAT --operator`. A retired name starts a new seat next time.
- Read-only: `HERDR_THREADS_SEAT_READONLY=1` beside the name lets a session read the seat's mail without acknowledging it: ACK requests stay pending for a writable session of the same seat.
- Inside a Herdr pane a name takes over from the pane: that session is the named seat, not the pane's seat. A name inherited from the Herdr server's own environment is refused (`... inherited from the Herdr server`); do not export it into a shell that starts Herdr.

## Waking a named seat

An idle named seat is woken only by:

- **Claude with the herdr-threads mod**: its live channel wakes it, as in a pane.
- **tmux**: run the agent inside a tmux pane. herdr-threads verifies the pane against the agent's process and types the wake into an empty composer, only for an idle agent and never over typed text, a copy-mode view or a blocked screen. A focused pane (attached and active) waits until its composer has stayed empty for a minute. Works for Claude and agy; Codex and Hermes get no tmux wake.

Without either, the seat has no idle wake: ACK requests and p0 messages arrive on its next turn, the sender sees `wake=no_wake` once (`wake=no_wake offline` when no session is running), and `nudge-threshold` has no effect.

## herdr-core terminals

An agent in a herdr-core terminal needs nothing extra: the core sets `HC_TERMINAL_ID` and `HC_TERMINAL_SOCKET`, and the hooks enroll the terminal's seat. A seat is (core, terminal): a wiped or recreated core starts new seats. It is shown as `SESSION:tN`; bare `tN` works while only one core is attached. Labels from the desktop app fill the workspace and tab columns.

- No composer wake on herdr-core yet: only a live Claude mod channel wakes an idle terminal agent. Otherwise it behaves like a named seat without a wake (above).
- `seat retire --self` is for named seats; a terminal seat ends when its terminal is closed for good.
- A terminal started from a Herdr pane can inherit `HERDR_PANE_ID`. herdr-threads then picks the terminal or the pane by process ancestry and refuses when neither matches (`... neither is an ancestor of this process; unset the stale one`).

## Launching onto a core

`launch` and `handoff` create the agent's place first, then start it:

```sh
herdr-threads launch --kind claude --new-terminal                      # a terminal parented on this one
herdr-threads launch --kind claude --new-tab review --space SPACE      # a tab in a desktop-app space
herdr-threads handoff --kind codex --new-terminal -- "Review the parser change"
herdr-threads launch --kind claude --new-terminal --space @MACHINE --cwd /abs/dir   # on another machine
```

- `--new-tab` onto a core needs `--space`. `--new-terminal` is herdr-core only.
- A launched agent starts idle. `--harness-binary` and `--name` are refused onto a core, and `handoff --thread` / `--new-thread` into a core are refused.
- The agent runs under the launching side's harness profile. A launch requested by another machine with no profile starts the harness in its default mode.

## Accepted limits

- **Credential variables in a core.** A core keeps the environment of the process that first started it. If that held `ANTHROPIC_API_KEY`, `CLAUDE_CODE_OAUTH_TOKEN` or `OPENAI_API_KEY`, every launch in that core gets it, and it can override the launch's profile. Start the core from a shell without them, or restart it.
- **No wake** for a named seat without the Claude mod or tmux, and for any herdr-core seat without the mod (above).
- **Cooperative, not enforced.** A name and a terminal id are claims of processes under your account, as panes are ([trust policy](../TRUST-POLICY.md)).
