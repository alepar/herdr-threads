# Bare-Hermes detection in Herdr 0.9.1: evidence (2026-10-09)

Question from the ht-4ne handoff: does a bare `hermes` pane (started without
`herdr-threads launch --kind hermes`) show up in Herdr as agent `hermes`?

All probes ran in isolated named Herdr test sessions (`scripts/lib/isolated-herdr.sh`,
private HOME/XDG/socket under `/tmp/ih.*`, torn down on exit). The shared Herdr server
and the real `~/.hermes` were not written; a post-run `find ~/.hermes -newer <probe>`
showed only the user's running gateway's own cron/state files. Scripts:
`scratchpad/probe-herdr-side{,2,3}.sh` (session-local; the commands are reproduced below).

Versions: `herdr 0.9.1`; `Hermes Agent v0.21.5+3962.g37daf85`; Herdr's integration
`herdr-agent-state` v5 (`herdr integration install hermes`, which writes the plugin into
`$HOME/.hermes/plugins/` and enables it in `$HOME/.hermes/config.yaml`).

## Two separate Herdr signals

Herdr exposes two different things on a pane, and they come from different places:

| Field | Source | Set for a bare Hermes pane on this host? |
|---|---|---|
| `pane.agent` (`"hermes"`) | Herdr's process detection of the pane's foreground process | **No** (see A) |
| `pane.agent_session` (`{agent: "hermes", kind: "id", source: "herdr:hermes", value: <session id>}`) | `herdr pane report-agent-session`, called by Herdr's Hermes plugin | **Yes**, when the plugin is installed and enabled in the selected profile and `HERDR_ENV=1` (see B) |

herdr-threads' host layer reads `pane.agent` / `agent get .agent` (`src/host/native.rs`
`result.get("agent")`, `current.get("agent")`), not `agent_session`.

## A. Process detection keys on argv0 `hermes`

- `(exec -a hermes sleep 20)` → `pane.agent = "hermes"`; server log
  `agent changed ... agent=Some(Hermes) process=hermes`.
- A python process with `hermes_cli.main` only as an argument → no `agent`.
- The real launcher shape (`~/.local/bin/hermes` → `.hermes/bin/hermes`, a `sh` script that
  `exec`s `python3 -I -c '<script>' "$@"`), reproduced with the body replaced by a sleep →
  argv0 `python3`, process name `python3.14`, **no `agent`**.
- Hermes renames itself only via the optional `setproctitle` dependency
  (`hermes_cli/main.py` `_set_process_title`); on macOS without it, it uses
  `pthread_setname_np`, which changes the thread name only. `setproctitle` is not
  installed anywhere under `~/.hermes` on this host. So a real bare Hermes pane here is
  **not** process-detected; on Linux (`prctl PR_SET_NAME`, 15-char comm) or with
  `setproctitle` installed it likely would be. Unverified on Linux.

## B. The plugin's report sets `agent_session`

Running the plugin's exact argv from inside the pane
(`herdr pane report-agent-session $HERDR_PANE_ID --source herdr:hermes --agent hermes
--seq <n> --agent-session-id sess-1 --session-start-source startup`) → rc 0 and
`pane.agent_session = {agent: hermes, kind: id, source: herdr:hermes, value: sess-1}`;
`pane.agent` stays absent and `agent_status` stays `unknown`.

Follow-up reports with a new session id (`--session-start-source new`, seq 200;
`resume`, seq 300) returned rc 0 but **did not replace** `value: sess-1` in this probe.
The pane's foreground process was the shell, not a detected agent, so this may be
"first report wins while no agent process is attached" rather than general behavior;
treat `agent_session.value` as not tracking `/new` resets until verified with a live
Hermes.

The plugin reports only for interactive platforms (`cli`, `tui`, `desktop`, `acp`), only
with `HERDR_ENV=1` and a `HERDR_PANE_ID`, and only on `on_session_start` (startup),
`on_session_reset` (new) and `pre_llm_call` (resume, cli only). It carries no
top-level/delegated-child distinction.

## Not verified

A live end-to-end bare Hermes run (real `hermes` binary firing `on_session_start` in an
isolated HERMES_HOME): a fresh HERMES_HOME makes the launcher run a source-update
dependency sync (`hermes_cli/venv_sync.py`, `pm/runtime.py`) keyed on the install root,
which risks writes under the real `~/.hermes/hermes-agent`. Aborted for that reason.

## Consequence for ht-4ne

"Bare-Hermes detection is solved upstream" holds only partially:

1. Herdr's process detection (`pane.agent`), which herdr-threads' host layer consults,
   does **not** see a bare Hermes on macOS without `setproctitle`.
2. Herdr's plugin gives a reliable `agent_session.agent = hermes` marker at session start,
   but only where the user installed Herdr's integration, and herdr-threads does not read
   that field today.
3. Neither signal says whether a session is top-level or a delegated child.

So global enrollment must come from herdr-threads' own Hermes hooks (the owned plugin),
admitted at callback time, with Herdr's signals as optional corroborating hints, never as
the qualification itself.
