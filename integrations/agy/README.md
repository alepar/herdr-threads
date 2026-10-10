# Antigravity CLI (`agy`) integration

herdr-threads supports the Antigravity CLI (`agy`) through an owned plugin. The plugin registers
three hooks (`SessionStart`, `PreInvocation`, `Stop`), which enrol the session, deliver pending
messages into the conversation and report turn ends. Delivery is by hook pull (`HookPull`): agy
asks for pending attention at each turn start, and message bodies arrive as hook-injected system
messages. An idle wake is typed by Herdr (`Native`). `launch` and `handoff` are supported.

agy has no tool-event hooks, so nothing is delivered mid-tool. A busy session receives its
messages at the next turn start; an idle one receives only nudge-class messages (see the
[delivery contract](../../AGENTS.md#delivery-contract)).

## Setup

```sh
herdr-threads setup agy
herdr-threads setup-status agy
herdr-threads unsetup agy
```

`setup agy` writes two things, both recorded in an ownership manifest under the herdr-threads
state directory:

- the plugin directory `~/.gemini/config/plugins/herdr-threads/` with `plugin.json`, `hooks.json`
  and `skills/herdr-threads/SKILL.md` (the core skill plus a short agy section);
- one `permissions.allow` member, `command(herdr-threads)`, in
  `~/.gemini/antigravity-cli/settings.json`, only with consent.

The allow member goes through the same consent and backup model as Claude and Codex.
`setup agy --with-permissions` adds it; `--without-permissions` removes the one setup added (the
plugin stays; a member that was already in your file stays too). A plain `setup agy` keeps what
is there: it keeps a recorded grant current (re-adding a member agy rewrote away) and records your
own member as pre-existing. When nothing is granted it asks on a terminal, `Let agy agents run
herdr-threads commands without prompting? The allow entry command(herdr-threads) also covers person
and setup commands [Y/n]` (empty line is yes), and without a terminal it reports
`permissions.action: advised` with a note and changes nothing. The entry covers every
`herdr-threads` command, person and setup commands included, because herdr-threads writes no agy
ask rule. Each run keeps one pre-image of `settings.json` beside it
(`settings.json.<stamp>-<id>.herdr-threads`) before its first change; a file setup itself creates
gets none.

Setup writes a file only when it is new or still equals what it recorded. A file it does not own,
or a `herdr-threads` plugin directory without its manifest, is a conflict and is never
overwritten. It does not use `agy plugin install` (that copies files untracked and writes
`import_manifest.json`), and it never edits `~/.gemini/config/hooks.json`.

`unsetup agy` removes only files that still match their record, the directories it created, and
the allow member only if it added it. A member that was already in your file stays. Edited
plugin files are reported as residue and left in place. `import_manifest.json` and `config.json`
are never touched.

## Plugin enable

agy keeps plugin enablement in `~/.gemini/config/config.json`
(`plugins["herdr-threads"].enabled`). An absent file or key means enabled. herdr-threads only
reads that file. If the plugin is disabled, `setup-status`, `doctor` and `launch` report it with
the repair, and you run it yourself:

```sh
agy plugin enable herdr-threads
```

herdr-threads never re-enables the plugin. An unreadable `config.json` reports enablement as
unknown, and `launch` refuses until you fix it or enable the plugin.

## Allow entry and agy's rewrites

The allow member `command(herdr-threads)` lets the agent run `herdr-threads ...` through
`run_command` without a prompt per command. agy rewrites `settings.json` itself and can drop the
member. When a grant was recorded, `setup-status` and `doctor` then report the entry as `missing`,
`launch` warns (it does not refuse; agy then asks before each herdr-threads command), and re-running
`herdr-threads setup agy` adds it back. `not_granted` (no record and no member: consent was never
given) is not a defect: it produces no warning, limitation or repair.

Agents should run each command as one plain `herdr-threads ...` command. Chaining (`;`, `&&`,
`|`) and prefixes (`cd`, `env`) are outside the allow rule and need approval.

## Launch and handoff

```sh
herdr-threads launch --kind agy --pane <pane> -- --model gemini-3
herdr-threads handoff --pane <pane> --new-thread --topic <topic> --goal <goal> --kind agy \
  --agent-arg=--model --agent-arg=gemini-3 -- "<durable message>"
```

Launch requires the plugin installed, current and enabled. A missing or stale plugin is refused
with "run herdr-threads setup agy"; a disabled one with the `agy plugin enable` hint. The accepted
flags follow `agy --help` 1.3.2 with Go flag grammar (`--name value` or `--name=value`):

- with a value: `--add-dir` (repeatable), `--agent`, `--conversation`, `--effort`
  (`low|medium|high|xhigh|max`), `--mode` (`accept-edits|plan`), `--model`, `--project`,
  `--log-file`, `-i`/`--prompt-interactive`;
- boolean: `-c`/`--continue`, `--new-project`.

Everything else is refused: print mode (`-p`/`--print`/`--prompt` and the print-only flags),
`--dangerously-skip-permissions`, `--sandbox`, `--remote-control`, unknown flags, positionals,
subcommands and `--`.

`-c` is `--continue`: it resumes the most recent conversation. It is not an alias of
`--conversation <id>`, which resumes a specific one. Passing both is refused.

`HERDR_THREADS_AGY_OPTS` holds default agy flags, in the same grammar, placed before the flags on
the command line (`HERDR_THREADS_AGY_OPTS="--model gemini-3 --effort high"`). A refused flag in
it is refused after composition.

A handoff starts the new agy with `--prompt-interactive <bootstrap text>` appended, so a caller who
already passes `-i`/`--prompt-interactive` conflicts with it. Handoff to agy goes through bootstrap
plan version 2, which freezes agy's own composition of the flags. Use `handoff --pane`; `handoff --new-tab --kind agy` is
not supported yet: its preflight still appends the prompt as a bare argument, which agy's flag
grammar refuses.

## `--sandbox`

`agy --sandbox` sessions may not reach the herdr-threads daemon socket. This is untested, and
sandbox socket support is outside this adapter. `launch` and `handoff` refuse `--sandbox`; a
session you start with it yourself is unsupported.

## Herdr coexistence

Herdr installs its own `herdr` hook block in agy's global `~/.gemini/config/hooks.json`.
herdr-threads uses its own plugin (`hooks.json` inside the plugin directory) and leaves that
global file and block untouched, so Herdr's agent detection and the plugin run side by side.

## Doctor

`herdr-threads doctor` reports for agy:

- the plugin status (installed, current, enabled, version) and the allow entry state
  (`owned`, `pre-existing`, `not-granted` or `missing`) and the `permissions` state;
- the witness scan: a bounded, read-only look at up to 64 recent `brain/` conversations, counting
  top-level conversations that have no presence lock;
- the `--sandbox` limit above;
- limitations: plugin missing, stale or disabled; allow entry missing (a not-granted entry is not a limitation);
  `SessionStart without witness: N recent top-level conversations lack a presence lock`;
  `subagent check bounded: brain/ has N entries`.

Safe repair is re-running `herdr-threads setup agy`; the manual repair is
`agy plugin enable herdr-threads`.

## Roles

agy fires no session-start hook on resume, and hooks carry no role. The role of a conversation is
read from agy's own runtime files (the presence lock and the subagent listings), and any doubt
reads as a subagent: a missing or unreadable witness never enrols a session. Subagents are
answered locally with the subagent restriction and never ACK, accept or send. See the agy section
of the [trust policy](../../TRUST-POLICY.md).
