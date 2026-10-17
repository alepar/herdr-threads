# Topic 7: Detecting a Herdr remote attach

Research date 2026-10-13. Primary source: upstream Herdr at `6521382` (Cargo `0.9.4` plus the preview, cloned read-only) [S1], the docs shipped with it [S2-S5], the local `herdr 0.9.1` CLI [S6] and this repo [S7]. No remote was attached. No live remote connection was running locally during this research (`ps` found no bridge processes), so the process-table shapes below come from reading the source, not from watching a live connection.

## Recommendations

1. **Herdr has no attach hook, event or notification, so do not wait for one.** Plugin `[[startup]]` hooks run "after Herdr restores the session… but not when a client attaches" [S4 plugins.mdx:249-251]. The socket event set (`pane.*`, `tab.*`, `workspace.*`, `worktree.*`, `layout.updated`) has no client or attach kinds [S5]. Supporting a future upstream `client.attached`/`machine.*` event would be welcome, but no part of v1 should depend on it.
2. **Primary discovery: saved machines.** Poll `herdr machine list --json`, a documented script interface that returns `{id,label,target,session,enabled}` [S2:84, S6]. Store each enabled entry as a *candidate* peer `(ssh target, remote Herdr session)`. Do not parse the backing file `~/.local/state/herdr/client/endpoints.json` [S1 src/client/endpoint/catalog.rs:373]. It is internal, although watching its mtime is a cheap way to know when to re-poll.
3. **Secondary discovery: ad-hoc `herdr --remote`.** Use the existing `src/host/process_table.rs` approach. Match a local `herdr --remote <target> [--session <s>]` client, or its `ssh … -T <target> <cmd>` child whose command contains `remote-client-bridge` (with `--session <s>` before it when the session is named) [S1 src/remote/attach.rs:416-443]. These are hints only. Record them as peers only after a successful handshake.
4. **Explicit `peer add <dest> [--session <s>]` stays first-class.** It is the only way to add a peer from the remote side, and it covers `ssh box; herdr` (tmux-style) usage, which leaves no trace on the client.
5. **The peer key must include the remote Herdr session, not just the ssh destination.** One host can run several Herdr servers (named sessions), and each one is a herdr-threads node. The command to dial is therefore `ssh <dest> herdr-threads peer --stdio --session <s>`, or something equivalent that selects the instance.
6. **Do not assume herdr-threads exists on the remote.** Herdr "does not copy local command plugins, configuration, executables, or secrets onto SSH hosts" [S2:114]. If the remote binary is missing, the dial fails visibly, and the peer is shown as "candidate: not installed" rather than retried forever. Copy Herdr's lookup order for non-interactive PATH: `PATH`, then `~/.local/bin`, then known install roots [S3:76]. Today the binary is symlinked as `~/.local/bin/herdr-threads -> ~/.local/share/herdr-threads/bin/herdr-threads`.
7. **Do not reuse Herdr's ControlMaster.** Let herdr-threads' own `ssh` use the user's config, so the user's own `ControlMaster`/`ControlPath`, if set, is shared naturally. See the ControlMaster section below.
8. **The remote side cannot learn a route back. Confirmed.** Keep the vision's rule that only the dialer dials. The stdio link is bidirectional, so the dialed side never needs to dial out.
9. **Gate auto-dial on the user's opt-in.** Saved machines are per user, not per local Herdr session (see "Caveats"). Auto-adding every saved machine to every local instance needs a policy, for example dialing only from the `default` local session or a config flag. Otherwise *N* local sessions × *M* machines all dial.

## How a remote connection works

There are three user paths [S3:46-58, S2]:

| Path | What runs where | Trace left on the local machine |
|---|---|---|
| `ssh box` then `herdr` | Client and server both run on the remote | None (only a plain ssh process) |
| `herdr --remote <target> [--session s]` | Local TUI client. The remote runs `herdr [--session s] remote-client-bridge` under ssh | Client argv and ssh child argv |
| Saved machine (`herdr machine add <target>`) | The local TUI multiplexes Local plus N machines. One ssh bridge per enabled machine, with backoff reconnects of up to 2 min | Catalog entry plus ssh child argv |

- **Target form.** Any ssh destination: a `~/.ssh/config` alias (`workbox`), `user@host`, or `ssh://you@server:2222` [S2:28, S3:51-52]. The saved catalog stores the target string verbatim, for example `"target":"blacky.dayton"` locally [S6]. Herdr passes it straight to `ssh -T <target>`, so it is exactly what herdr-threads should pass to its own `ssh`.
- **Local command.** Herdr writes a temporary ssh config: `Include ~/.ssh/config`, then fallback keepalives, where user values win [S1 attach.rs:3091-3110; S3:78]. It runs:
  - `ssh -C -F <tmpcfg> -S /tmp/hssh-<uid>/<hash24>-%C -o ControlMaster=auto -o ControlPersist=600 [BatchMode=yes … ServerAliveInterval=15 …] -T <target> '<cmd>'` [S1 attach.rs:1379-1416, 3196-3212].
  - With `[remote].manage_ssh_config=false` it runs plain `ssh` with no control socket [S3:78].
- **Remote command.**
  - `--remote` runs `printf … 'herdr-remote-output-ready:1'; exec "$HOME/.local/bin/herdr" remote-client-bridge`.
  - Saved machines run `exec <herdr> --session <s> remote-client-bridge </dev/null`.
  - API forwarding (`herdr --machine X …`) runs `<herdr> --session <s> remote-api-bridge` [S1 attach.rs:416-443, 2878-2894, 5274-5288].
- **Remote side.** `run_remote_client_bridge` starts the server daemon if needed. It registers `SSH_AUTH_SOCK` through `server.ssh_agent.register`, then pipes stdio to the server's local client socket [S1 src/remote/host.rs:6-37]. The server sees an ordinary local client-socket connection. The only SSH-specific API call is the agent-socket registration [S5; the "SSH registration" in `docs/compatibility/herdr-093-audit.md:31`].

## What the remote side can observe

- The remote `herdr remote-client-bridge` process inherits sshd's environment: `SSH_CONNECTION` (`client_ip client_port server_ip server_port`), `SSH_CLIENT` and possibly `SSH_AUTH_SOCK`. A remote herdr-threads could see that process in its process table (`/proc/<pid>/environ` on Linux; macOS restricts reading another process's env). Herdr itself reads `SSH_CONNECTION`/`SSH_TTY` only to tell it is running under SSH [S1 src/client/terminal_setup.rs:556, src/selection.rs:348].
- The server API exposes no client identity, address or attach event [S5]. `server.ssh_agent.register` carries only a remote-host socket path [S1 src/api/schema/server.rs:36-39].
- **No route back.** `SSH_CONNECTION` gives an IP as seen from the server. That IP may be behind NAT, a Tailscale/VPN address, a jump host or the bastion's address. It carries no username, alias, port of an sshd on the client, or credentials, and the client often runs no sshd at all. The remote can therefore at most show "a client from 100.x.y.z attached", which is a diagnostic hint, never a dialable peer. This confirms the assumption in the brief and fits the vision's dialer-only reconnect rule.

## Plugin on the remote

herdr-threads is a Herdr plugin (`herdr-plugin.toml`, `[[startup]] ./scripts/view.sh ensure`) [S7]. Plugins are installed per host under that host's Herdr config (`~/.config/herdr/plugins.json`) and run on the server that advertises them [S2:114, S4]. Herdr installs or updates only Herdr itself on a remote [S2:30, S3:76].

So on a remote with the plugin installed, its startup hook starts that node's daemon when the remote server starts. A remote without the plugin has no daemon, and `peer --stdio` must fail with a clear "not installed" error. The daemon socket and state dir come from `HERDR_PLUGIN_STATE_DIR`/`HERDR_SOCKET_PATH` [S7 src/daemon/paths.rs:79-103]. Those variables are set only under a plugin invocation, so `peer --stdio` started by sshd has neither. It must resolve the instance itself from `--session` (Herdr's socket path for that session plus the plugin state dir), or ask the remote `herdr` CLI for them.

## ControlMaster reuse

- Herdr's control socket is `/tmp/hssh-<euid>/<sha256(config_path\0target)[..24]>-%C`, mode 0700 and same user [S1 src/platform/unix_common.rs:666-705]. A same-user process could compute the path and run `ssh -S <path> -o ControlMaster=no …`. The docs even note that plain `ssh` gains Herdr's approval only "if it uses the shared ControlPath" [S2:16].
- Against reuse:
  - The path scheme is undocumented and internal. It hashes Herdr's config path, so `HERDR_CONFIG_PATH` matters.
  - The master idles out after `ControlPersist 600`.
  - Herdr retires stale masters itself (`ssh -O stop`) [S1 attach.rs:931; S2:18].
  - It exists only on Unix with `manage_ssh_config=true`.
  - Coupling to it would make herdr-threads break whenever Herdr's transport changes, which runs against the AGENTS.md versionless rule.
- Recommended: herdr-threads keeps one long-lived `ssh` per peer under the user's own config. Users who want multiplexing set `ControlMaster auto` in `~/.ssh/config`, which both tools then share. MFA-gated hosts are the one real cost, because herdr-threads' `BatchMode` dial fails until the user authenticates. Surface "auth needed: run `ssh <dest>`" the same way Herdr's `! auth` badge does.

## Caveats

- The saved-machine catalog lives in client state and is shared by all local Herdr sessions of the user [S1 src/config/io.rs:37, catalog.rs:373]. It describes what the TUI *client* connects to, not what a particular local *server* serves. Mapping it to a node, meaning which local instance becomes the dialer, is a herdr-threads policy decision (Recommendation 9).
- `herdr machine list` exists in local 0.9.1, which is admitted. Older Herdr versions without it should degrade to process observation plus `peer add`, keyed on the observed failure, per AGENTS.md.
- A disabled machine should not be dialed automatically, but disabling one should not `forget` the peer either.

## Bibliography

- [S1] herdrdev/herdr source, commit 6521382dbed36f2d001417108149c338aee116ea: `src/remote/attach.rs`, `src/remote/host.rs`, `src/remote/ssh_agent.rs`, `src/platform/unix_common.rs`, `src/client/endpoint/catalog.rs`, `src/api/schema/server.rs`, `src/main.rs:565`, `CHANGELOG.md`.
- [S2] `docs/next/website/src/content/docs/connecting-machines.mdx` (same commit).
- [S3] `docs/next/website/src/content/docs/persistence-remote.mdx`.
- [S4] `docs/next/website/src/content/docs/plugins.mdx`.
- [S5] `docs/next/api/herdr-api.schema.json` (method and event names).
- [S6] Local `herdr --help`, `herdr status client` (0.9.1, protocol 22), `herdr machine list --json`.
- [S7] This repo: `herdr-plugin.toml`, `src/daemon/paths.rs`, `docs/compatibility/herdr-093-audit.md`.

**Method:** Upstream source was cloned (git) instead of web-searched; this was the primary source, and the `search` CLI was not used. Seven facets were all closed from source and docs. Gaps: no live remote attach was observed, and the remote-side `/proc` environment visibility is inferred.
