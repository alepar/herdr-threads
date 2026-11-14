# opencode integration

herdr-threads supports opencode through an owned plugin, `herdr-threads.js`, that opencode loads
from its config directory. The plugin enrolls each top-level session at its first prompt, names
the session on every shell call, and runs a `watch` channel that puts message bodies into the
session and wakes it when idle (`Channel`, like the Claude mod). Nothing is typed into the
composer. `launch` and `handoff --kind opencode` are supported.

## Setup

```sh
herdr-threads setup opencode
herdr-threads setup-status opencode
herdr-threads unsetup opencode
```

`setup opencode` writes two owned files under the opencode config root, recorded with their
digests in a manifest under the herdr-threads state directory:

- `plugins/herdr-threads.js`: the plugin, with the absolute `herdr-threads` executable, state
  directory and host endpoint rendered into it. It has no npm dependency.
- `skills/herdr-threads/SKILL.md`: the shared skill plus a short opencode section.

The config root is `$OPENCODE_CONFIG_DIR`, else `$XDG_CONFIG_HOME/opencode`, else
`~/.config/opencode`. `opencode.json` is never read or written. A file at an owned path that
setup did not write, or that was edited, is a conflict and is never overwritten. `unsetup
opencode` removes only files that still match their record; edited ones are reported as residue
and left in place. Restart opencode after setup or unsetup.

## Permissions

With consent, the plugin allows `herdr-threads *` and `ht *` in opencode's bash permissions. It
adds the two rules in its `config` hook, to the merged config held in memory, last so they win
(last match wins); a user `deny` on either exact key is kept and that grant is skipped. The grant
covers every herdr-threads command, person and setup commands included.

`setup opencode --with-permissions` renders the plugin with the grant; `--without-permissions`
renders it without. A plain `setup opencode` keeps the recorded choice; when there is none it asks
on a terminal (empty line is yes), and without a terminal it reports `permissions.action: advised`
and grants nothing. Without the grant opencode asks before each command until you choose "Allow
always".

## Delivery and wake

- A thread item is appended to the bound session as a stored message with a hidden synthetic part:
  the TUI shows no bubble for it, only the model's reply. The ACK carries the stored message id.
- Busy session: the running turn's next step reads it; no wake.
- Idle session: a passive item waits for the next turn. A waking item (priority at or below the
  seat's `nudge-threshold`, ACK requests and invitations) is followed by the prompt
  `herdr-threads: wake up`, submitted in-process; it shows as a user message, and a typed draft
  stays in the composer.
- With the channel down (stalled, `mod_delivery` off, watch refused), pending items arrive at the
  next turn start through the plugin's turn hook, and there is no wake.
- Subagents (`task` child sessions) get the subagent restriction, read-only, and are never woken.

See the [delivery contract](../../AGENTS.md#delivery-contract).

## Launch and handoff

```sh
herdr-threads launch --kind opencode --pane <pane> -- -m <provider/model>
herdr-threads handoff --pane <pane> --new-thread --topic <topic> --goal <goal> --kind opencode \
  -- "<durable message>"
```

Accepted: `-m`/`--model`, `--agent`, `-c`/`--continue`, `-s`/`--session`, `--fork` (with `-c` or
`-s`) and `--prompt`, as `--name value` or `--name=value`; `-c` and `-s` conflict. Refused:
positionals (so every subcommand: `run`, `serve`, `attach`, `acp`, ...), `--pure` (no plugin),
`--auto`/`--yolo`/`--dangerously-skip-permissions`, `--port`/`--hostname`/`--mdns`, `--mini`,
unknown flags and `--`. `HERDR_THREADS_OPENCODE_OPTS` holds default flags in the same grammar.

opencode creates a session only at the first prompt, so a launch without `--prompt` starts with
the prompt `herdr-threads: wake up` and the session enrolls at once. A handoff passes its bootstrap
as `--prompt`, so a caller's own `--prompt` conflicts with it. A launch with the plugin missing
warns and starts anyway.

The launch checks the plugin in the root the pane's shell selects (`OPENCODE_CONFIG_DIR`, else
`$XDG_CONFIG_HOME/opencode`), else in setup's root. `handoff --new-tab --profile-dir DIR` starts
opencode with `OPENCODE_CONFIG_DIR=DIR`; run `setup opencode` with that variable set first.

## Doctor

`herdr-threads doctor` reports the plugin, skill and permission states and the plugin heartbeat:
the plugin writes `<state>/opencode/plugin-heartbeat.json` at the end of its load. Limitations:

- plugin not installed, unowned, missing, modified or stale; skill not current;
- `installed, never loaded`: opencode logs a load failure only in its own log
  (`failed to load plugin`);
- not loaded since setup wrote it (restart opencode);
- a second plugin copy installed in another opencode config directory (it stays inert);
- a capability this opencode lacks (for example no wake: `wake=no_wake`);
- a user `deny` rule on a grant key.

Doctor also lists same-named `herdr-threads` skills in `~/.agents/skills` and `~/.claude/skills`.

## Limits

- **Plugin not loaded** (`--pure`, a load error, setup not run): no enrollment, delivery or wake;
  CLI calls are refused `session_claim_required` naming `HERDR_THREADS_OPENCODE_SESSION`.
- **A woken session can be one the TUI is not showing.** After `/new` or a session switch the
  plugin keeps the previous session bound until the next prompt, so a wake runs it in the
  background and Herdr shows the pane idle. A permission or question in that turn blocks unseen;
  the plugin shows a toast naming the session.
- **A session resumed by hand** (`-c`, `-s`) has no channel until its first prompt: no wake, and a
  sender sees `wake=no_wake`.
- **`opencode serve` with `attach`**: plugins run in the server, so its sessions belong to the
  server's pane.
- **A plain `launch --new-terminal` onto a herdr-core terminal** gets no first prompt, so the seat
  enrolls at the first prompt you type (ht-vxdp.11).
- **Duplicate skill names.** opencode shows one `herdr-threads` skill and picks among same-named
  copies (`~/.agents/skills`, `~/.claude/skills`) unpredictably. The opencode guidance also rides
  each check-in, so it does not depend on which copy wins.

Attribution, roles and these limits are normative in the opencode passages of the
[trust policy](../../TRUST-POLICY.md).
