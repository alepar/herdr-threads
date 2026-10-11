# ht-54r.89: pane foreground PID vs binding fingerprint PID

Probe: `probe.sh` (this directory) against an isolated Herdr 0.9.1 session
(`scripts/lib/isolated-herdr.sh`; HOME, CLAUDE_CONFIG_DIR, CODEX_HOME, HERMES_HOME all under the
private root). Raw output in `run/<harness>/`: `process-info.{1,2,3}.json` (three samples one second
apart; `process-info.json` is the third), `ps-tree.txt`, `foreground.txt`,
`fingerprint-candidates.txt`. Run on 2026-10-11, macOS arm64.

No harness could log in (no credentials in the isolated homes), so each stopped at its login or setup
prompt and **no SessionStart hook ran: there is no binding row.** The fingerprint pid is therefore
*derived* with the adapter rule of `caller_env::walk` / `fingerprint_for`: the nearest harness-shaped
ancestor of any process the harness spawns (`CallerEnvShape.process_names` / `process_match`), or the
pid in `pid_var`. `fingerprint-candidates.txt` lists the harness-shaped processes of the pane tree;
the rule picks the one nearest the leaves. For Claude the rule is also confirmed live (last row).

"Current rule" = `started_process_pid` before this change: the single non-shell foreground pid, `None`
(no composer wake) when there are two or more. "Set rule" = the fingerprint pid is a member of the
non-shell foreground set (this change).

| Harness | Version | Non-shell foreground pids (samples 1/2/3) | Fingerprint pid | Current rule | Set rule | Why |
|---|---|---|---|---|---|---|
| Claude (isolated pane, login screen) | 2.1.296 | `[42815]` x3 | 42815 (argv0 `claude`) | match | match | One process: the harness itself. |
| Claude (live session this task ran under, pane `w4:pEN` of the shared server, read-only `pane process-info`) | 2.1.296 | `[50292 caffeinate, 52778 herdr-threads watch, 51712 node playwright-mcp, 51692 bun qmd, 51646 node qmd, 51642 mcp@latest, 51586 claude]` | 51586 = `CLAUDE_PID` = the `claude` process | **no match (`None`)** | match | Claude, its MCP servers, the `herdr-threads watch` mod process and `caffeinate` share Claude's process group, which is the pane's foreground group. |
| Codex native binary | codex-cli 0.162.1 | `[43259, 43396]`, `[43259]`, `[43259]` | 43259 (argv0 `codex`) | match in 2 of 3 samples, **`None` in sample 1** | match | A short-lived `git` helper (43396, plugin-clone in CODEX_HOME at startup) joins the foreground group at startup and whenever Codex shells out. |
| Codex via npm (`@openai/codex@0.162.1`, temp prefix) | codex-cli 0.162.1 | `[44462, 44490, 46272 (git helper)]`, `[44462, 44490]`, `[44462, 44490]` | 44490 (native `codex`; node shim 44462 is not harness-shaped) | **no match (`None`)** in every sample | match | `node` launcher (44462) and the native child (44490) are both foreground in pgid 44462; the fingerprint is the native child. |
| Hermes (real executable, first-run bootstrap finished, setup prompt) | v0.21.5 | `[48173]` x3 | 48173 (python launcher, `/.hermes/` in argv) | match | match | One process once bootstrap ends. During bootstrap (about 4 minutes on a fresh HERMES_HOME) 4 python/node workers are foreground (seen in an earlier manual run, not kept). |
| agy | 1.3.3 (fetched with `scripts/agy-sandbox/fetch-agy`) | `[56651]` x3 | 56651 (argv0 `agy`) | match | match | One process; matches the `agy -> -zsh -> herdr` ancestry recorded by ht-54r.84. |

## Verdict: mismatch observed (fix path)

Shape (a): several foreground pids, one of which is the fingerprint pid. Observed for npm Codex
(launcher plus native child), for Codex native during startup and tool runs (helpers), and for the live
Claude session (MCP servers, the watch process, `caffeinate`). With the current rule these panes get
`None` and **no composer wake fires at all**; in practice Claude, whose pane always carries its MCP
servers and the mod's `watch` process, would never be typed to.

Shape (b) (the fingerprint is an ancestor or descendant of the one foreground pid) was **not**
observed, and shape (c) (the fingerprint outside the foreground group) was not observed either. So the
fix is the set match only; no `ProcessInfo` ancestor rule was added.

## Not measured

- **`herdr-threads launch` for Hermes.** Hermes was started by its real executable in a pane, not by
  `herdr-threads launch` (needs `setup`, a daemon and a seat; `launch` ends in the same `agent.start`
  command line). The launch-time placeholder fingerprint is unchanged: it still uses the strict
  `started_process_pid`, so it is only recorded when exactly one process is foreground.
- **A real binding row** for any harness (no credentials). Fingerprint pids are derived, as above;
  only Claude's was independently confirmed (`CLAUDE_PID` = the `claude` foreground process).
- Codex native and Claude were not observed while running tools (needs a logged-in session).
- A harness started through the shell wrapper functions of this machine (`claude`/`codex` add flags);
  the real executables were run directly.
