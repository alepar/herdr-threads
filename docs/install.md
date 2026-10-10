# Installation and local setup

> [v0.1.0 is published](https://github.com/alepar/herdr-threads/releases/tag/v0.1.0); [v0.2.0 is published](https://github.com/alepar/herdr-threads/releases/tag/v0.2.0); v0.2.1 adds approved outside-sandbox Codex CLI execution. Package lifecycle and a real GitHub install/reinstall/uninstall matrix passed against private Herdr 0.9.1 sessions. These rehearsals did not alter a shared, everyday Herdr session. Native support is summarized from the [validation report](validation/report.md) (verdict **PASS_WITH_GAPS**); see the [README support table](../README.md#supported-harnesses).

## Prerequisites

- macOS on arm64 (supported). Linux x86_64 and arm64 are experimental: prebuilt archives exist, but the Herdr link is unverified and the daemon runs degraded there (see [below](#installing-a-prebuilt-release)). The manifest declares `platforms = ["macos", "linux"]`, which names no architecture; only macOS arm64 has been exercised, and no other platform or architecture is claimed.
- Herdr 0.9.1 or newer (tested with 0.9.1–0.9.3). The adapter enforces the manifest's `min_herdr_version = "0.9.1"` floor and nothing else about the release: a newer release connects, and `daemon health` adds the limitation `untested Herdr X.Y.Z; tested 0.9.1-0.9.3` without degrading. An operation that release no longer serves fails as `unsupported` on its own. Optional operations still require their own capabilities. See [Herdr host compatibility](compatibility/herdr-host.md) and the [Herdr audit](compatibility/herdr-093-audit.md).
- Claude Code or Codex on `PATH`, including a managed wrapper. Core setup, launch and hooks use registered contracts with strict payload validation and do not run harness version/help/schema probes. Runtime metadata may be unavailable; installation and valid input do not prove native support or enable richer capabilities.
- Building from source requires Cargo and the pinned Rust toolchain (CI uses Rust 1.94.0, edition 2024). Prebuilt installation does not require Cargo. Source builds use the committed lockfile with `--locked`.
- `herdr-threads` on the agent's `PATH`: the hook's ready commands name it bare.

## Installing a prebuilt release

The installer fetches a published prebuilt archive instead of building from source:

```sh
curl -fsSL https://raw.githubusercontent.com/alepar/herdr-threads/main/scripts/install.sh | bash
# options go after `bash -s --`, e.g. a pinned version and hook setup:
curl -fsSL https://raw.githubusercontent.com/alepar/herdr-threads/main/scripts/install.sh | bash -s -- --version v0.2.1 --setup
```

It detects the OS and architecture, downloads `herdr-threads-OS-ARCH.tar.gz` and `SHA256SUMS` from the GitHub release (latest, or `--version`), refuses a checksum mismatch, installs the package into `~/.local/share/herdr-threads` (`--prefix`) and links `~/.local/bin/herdr-threads` and `~/.local/bin/ht` (`--bin-dir`) to its executable, warning when that directory is not on `PATH`. It then registers the package with `herdr plugin link`. Herdr does not build a linked plugin, and the archive carries a `PREBUILT` marker that makes the manifest's build command (`scripts/build.sh`) keep the shipped executable instead of running cargo. When the Herdr server is running it runs the `ensure` action; otherwise the daemon starts with the next server start. On an upgrade it first runs the `stop` action **before** replacing the package, so the still-installed old executable stops the old daemon (a newer executable never takes over an older daemon, and across a wire-protocol change it cannot even stop it). See [Updating](#updating) for what happens when that stop fails.

`ht` is installed by default and runs the same CLI: `ht skill` prints the embedded guide and `ht follow` follows a thread. It does not rename the agent skill: invoke that skill as `$herdr-threads`. Only an exact symlink to this installation's executable is considered owned. Any other entry at the chosen `ht` path (including a broken symlink or directory) is preserved, even with `--force`; use the absolute installed `herdr-threads` executable instead. Upgrades preserve an owned alias, and uninstall removes only that exact link, leaving any foreign replacement untouched.

Creating the link does not guarantee that bare `ht` selects it. The installer checks its own `PATH` and reports an earlier executable (for example `/Library/TeX/texbin/ht`) or an absent bin directory, with the absolute `ht skill` / `ht follow` invocation. Put the chosen bin directory first on `PATH` to select this alias. Caller shell aliases/functions may also override it; the installer does not inspect or edit shell startup files.

Harness integrations: for each supported harness found on `PATH` (`claude`, `codex`), the installer reconciles three components separately: hooks, the agent skill and permissions. Exact owned components update automatically without prompting. Each missing component gets its own confirmation on the controlling terminal (including `curl | bash`); declining leaves it absent. Permissions are asked separately, and only where the hooks are installed. With redirected output or no terminal, missing components are skipped. `--setup` explicitly confirms installation of every missing component, including permissions; `--without-permissions` never grants, so agents keep prompting; `--no-setup` skips all integration checks and updates. Partial, edited or foreign integrations are reported and preserved; other components still proceed. See [Agent permissions](#agent-permissions) for the rules.

Hook writes use canonical `herdr-threads setup` ownership and conflict handling. The embedded skill matching the installed executable goes to `$CLAUDE_CONFIG_DIR/skills/herdr-threads/SKILL.md` or `$CODEX_HOME/skills/herdr-threads/SKILL.md` (default roots `~/.claude` and `~/.codex`). Its private ownership manifest records the exact path and content fingerprint; a skill name or frontmatter alone never grants permission to overwrite it. A foreign skill directory, an edited owned file, or an interrupted publication requires manual inspection. Other settings, hook groups and skills are preserved. Hook installation does not prove native delivery or launch readiness; Codex still requires the user to trust its hooks. Installer output names each component's result; supporting terminals get color unless `NO_COLOR` is set.

Older releases without the installer integration command retain their legacy hook-only setup behavior. A build with project-scoped setup is never run by the installer. Normal upgrades refresh the owned skill in the effective harness config roots, including an active `CODEX_HOME` profile; a new profile without an owned skill still needs consent. Use normal installation for skill freshness: `--no-setup` explicitly skips those updates. Uninstall retains skill files; hook removal remains the existing owned `unsetup` flow.

Re-running converges: an identical archive leaves the installation, link and registration alone; a different one replaces the package directory crash-safe at the same path (the old tree is renamed aside, the new one renamed in, and the old one restored if that fails), so the Herdr registration stays valid. It refuses to replace a directory it did not create (no `.herdr-threads-install` marker) or a regular file at the symlink path (`--force` replaces the file). An existing `herdr-threads` registration from another root (for example `herdr plugin install`) is left alone with a warning.

`--uninstall` stops the daemon, runs bare `unsetup`, which removes every recorded harness installation (user-level builds only; on a terminal it asks first, without one it needs `--yes`, otherwise the hooks are kept and the installer says so; `--no-setup` skips it), unlinks the plugin, and removes the package directory and the exact owned `herdr-threads` and `ht` symlinks. Unlinking needs a running Herdr server; without one the uninstall stops with an error, and `--force` removes the files anyway and tells you to unlink later. Daemon state in Herdr's plugin state directory is kept.

### Installer contract: final status and exit codes

The installer records each outcome as it happens, prints one `Next steps:` block built from those records (nothing prints hints mid-run), and ends with exactly one final status line. The exit status follows the line:

| Outcome | Final status line | Exit |
| --- | --- | --- |
| installed and linked | `installed and linked: herdr-threads <v>` | 0 |
| upgraded and linked | `upgraded: herdr-threads <old> -> <new> (linked)` | 0 |
| Herdr refused the link, `--no-herdr`, `herdr` not on `PATH`, or another registration left alone | `installed, not linked: <reason>` (`upgraded, not linked: <reason>` on an upgrade), then `register it with: herdr plugin link <dir>` | 3 |
| setup failed for a harness | `installed and linked; setup incomplete: <harnesses>`, then `finish it with: herdr-threads setup` | 3 |
| the old daemon could not be stopped and is still running | `<verb> and linked; old daemon still running: pid <pids>` (`<verb>, not linked; old daemon still running: pid <pids>` when not linked; `uninstalled; daemon still running: pid <pids>` on an uninstall), then `stop it with: kill <pids>, then ...` (just `kill <pids>` on an uninstall) | 3 |
| uninstall complete | `uninstalled` | 0 |
| uninstall without `herdr` on `PATH` (or with `--no-herdr`), or `--force` when Herdr could not unlink | `uninstalled, not unregistered: <reason>`, then `unregister it with: herdr plugin unlink herdr-threads` | 3 |
| bad arguments, download or checksum failure, any refusal | `herdr-threads: error: ...` on stderr | 1 |

On an upgrade the installer stops the old daemon with the still-installed old executable before replacing anything: with Herdr up through the plugin's `stop` action, and when Herdr is down (or that action fails) with `herdr-threads daemon stop`. With Herdr up it then runs `ensure` with the new executable. Herdr's plugin actions are read without splitting JSON in the shell: success is Herdr's exit status plus the log id field, and every value comes from the just-installed executable's hidden `herdr-threads internal json-field <dotted.path>` helper (JSON on stdin; exit 1 when the path is absent, 2 when stdin is not JSON).

Archives: macOS arm64 (the supported platform), macOS x86_64 (cross-built, not exercised), and Linux x86_64 and aarch64 (static musl). **Linux is experimental and its Herdr link is unverified until the follow-on rehearsal** ([release checklist](release.md#post-merge-follow-on-checklist), items 4 and 5): the manifest declares `platforms = ["macos", "linux"]`, but the host incarnation witness is macOS-only, so on Linux the daemon runs degraded with an Unknown incarnation, and Herdr may still refuse the link (the installer then exits 3 with `installed, not linked`). The installer says so. Evidence: `tests/release/install_test.sh` drives the installer against fake releases served from `file://` in a scratch HOME, and runs the Herdr-up and Herdr-down cases against an isolated named Herdr session (`scripts/lib/isolated-herdr.sh`; with no `herdr` it fails naming the binary, and `HT_SKIP_HERDR_TESTS=1` lists every skipped case by name and exits 0). It never contacts the shared Herdr server. No real GitHub release has been downloaded yet.

## Build

From a checkout:

```sh
./scripts/build.sh
```

The script runs `cargo build --release --locked` with `CARGO_TARGET_DIR` pinned to the checkout's own `target/` (an inherited `CARGO_TARGET_DIR` is ignored, so a stale executable is never installed). It then installs the result by rename at `bin/herdr-threads` (mode 0755). A failed build leaves the previous `bin/herdr-threads` in place and exits nonzero. It needs no Herdr runtime variables. This is also the manifest's `[[build]]` command.

## What the package declares

`herdr-plugin.toml` (id `herdr-threads`, name `Threads`, version 0.2.8) declares:

| Entry | Command | Runs |
| --- | --- | --- |
| build `binary` | `./scripts/build.sh` | the locked release build above |
| startup `daemon` | `./scripts/view.sh ensure` | `daemon ensure`, once, then exits |
| action `health` | `./scripts/view.sh health` | `daemon health` |
| action `doctor` | `./scripts/view.sh doctor` | `doctor` |
| action `ensure` | `./scripts/view.sh ensure` | `daemon ensure` |
| action `stop` | `./scripts/view.sh stop` | `daemon stop` |
| action `view` | `./scripts/view.sh open` | opens the `operator` pane through `$HERDR_BIN_PATH plugin pane open` |
| pane `operator` (overlay) | `./scripts/view.sh view` | `view --once`, then Enter refreshes and `q` quits |

`scripts/view.sh` requires `HERDR_PLUGIN_STATE_DIR` and `HERDR_SOCKET_PATH` (and `HERDR_BIN_PATH` for `open`), and passes them to the binary as quoted `--state-dir` and `--host-endpoint` arguments.

## Activating in Herdr

What the isolated package gate showed on a private Herdr 0.9.1 server (details and limits in [validation/package.md](validation/package.md)):

- `herdr plugin install OWNER/REPO --yes` (optionally `--ref REF`) checks out the source, runs the manifest build and registers the plugin. A failing build exits nonzero, registers nothing and leaves no checkout behind; over a working install it leaves the registration and executable unchanged.
- The `[[startup]]` entry runs when the Herdr server starts, including the server that a `herdr server live-handoff` spawns. It starts exactly one daemon with Herdr's plugin state directory and socket, and a rerun against a serving daemon keeps that daemon.
- The five actions are listed and work, and the `view` action opens the overlay pane.
- `herdr plugin link PATH` registers a local checkout **without building it**. Until you run `./scripts/build.sh` there, `health` and `ensure` fail loudly, naming the missing `bin/herdr-threads`. After the build, the linked daemon keeps the same instance and data.

Not exercised: the real GitHub fetch (the gate used a private `file://` rewrite), marketplace indexing, a server started implicitly by an attaching client, recovery after an unclean server crash, and `herdr update --handoff` with a real updated Herdr. Enabling a plugin does not by itself start the daemon until a server start runs the startup entry; run the `ensure` action and then `doctor` after activation. Registration is not proof that the daemon or any hooks work.

Native validation ran in isolated, owned Herdr panes, and the native rows describe the code at each run's SHA, not necessarily this build ([validation report](validation/report.md#stamp)). Until a release SHA is re-validated, prefer a private, isolated Herdr configuration and server for trials.

## Running the CLI outside a plugin action

Pass the same context the Herdr instance gives the plugin:

```text
/absolute/path/to/bin/herdr-threads --state-dir STATE_DIR --host-endpoint HOST_SOCKET daemon ensure
/absolute/path/to/bin/herdr-threads --state-dir STATE_DIR --host-endpoint HOST_SOCKET doctor
```

`STATE_DIR` is the plugin state directory (`HERDR_PLUGIN_STATE_DIR`) and `HOST_SOCKET` is the Herdr socket (`HERDR_SOCKET_PATH`). They are placeholders, not shell variables. Every command against one daemon must use the same pair: the daemon's instance directory is `STATE_DIR/instances/<sha256 of HOST_SOCKET>/`. The host endpoint is a filesystem path of up to 1024 bytes and may contain spaces.

The state directory must be owned by you and not writable by group or others (Herdr's 0755 directory is fine). Everything the plugin creates below it is 0700. An unsafe state directory, or a private directory below it (`instances/`, the instance directory, the runtime socket directory) whose mode or owner has been changed, is an invalid local context: `daemon ensure` and `doctor` exit 2, change nothing and do not repair the mode.

No global shell configuration is edited. Put the absolute executable path in your own agent instructions or PATH as you see fit.

Observed on this build (merge of integration `38a31f5`) with a missing host socket and a scratch state directory:

- `doctor` before ensure exits 3 (`daemon: not_running`, `result: unavailable`).
- `daemon ensure` exits 0 and prints health with `state: "degraded"`, `unresolved_seats: 0`, and a limitation for the unavailable host; admitted harnesses are `cooperative` and their facts are `notes`.
- `doctor` then exits 0 with `result: degraded` (the host is unavailable), and lists `hooks.claude.recipes: claude-hooks-2.1.283 [2.1.283, 2.1.286]; claude-hooks-2.1.287 {2.1.287}`, `hooks.claude.compaction_recovery: claude-hooks-2.1.283: unsupported (resume/clear and herdr-threads summary); claude-hooks-2.1.287: supported` and `hooks.codex.recipes: codex-hooks-v1 {0.157.1, 0.158.0, 0.159.3}`.
- `daemon stop` exits 0 with `stop_accepted`, and `daemon health` afterwards exits 3 with `host_unavailable`.
- With `instances/` changed to 0755, `doctor` exits 2 (`result: unsafe_state_dir`) and `daemon ensure` exits 2 (`unsafe private directory`).

## Settings: default deadlines and wake spacing

Per-operation deadlines use `--deadline SECONDS` on `invite` and `send`. The value must be positive. The default for both is 300 seconds.

Instance-wide defaults come from an optional `settings.json` in the instance directory (`STATE_DIR/instances/<digest>/settings.json`):

```json
{"invitation_default_ms": 300000, "receipt_default_ms": 300000, "minimum_wake_delay_ms": 30000, "harness_manifest": "auto"}
```

- All keys are optional; unknown keys are rejected. One schema covers every key, so the same file may mix them freely.
- The file must be a regular file you own, mode 0600, at most 4096 bytes. These rules apply to all keys.
- `harness_manifest`: `auto` (default) or `off`; `off` stops the daemon fetching the harness version manifest ([docs/compatibility/harnesses.md](compatibility/harnesses.md), "Manifest").
- Invitation and receipt durations must be positive. `minimum_wake_delay_ms` must be at least 30000.
- `auto_archive_after_ms` defaults to `3600000`; `0` disables automatic channel archival. See [channel lifecycle](channel-archival.md) for protected work, composer evidence and conservative legacy-journal coverage. This feature activates with the combined schema23/wire6 integration.
- The daemon reads it at start. Edits apply after `daemon stop` then `daemon ensure`. `daemon health` prints the effective `settings`.
- The chosen duration is frozen on each invitation and message when it is created.

An optional nested `"summary"` object tunes thread summaries, catch-up and soft-deadline pokes. Every key is optional:

```json
{"summary": {"chunk_bytes": 24576, "display_bytes": 40960, "narrative_bytes": 3072, "bundle_bytes": 49152, "fold_display_bytes": 24576, "max_new_leases": 8, "tracker_prefixes": ["ht-"], "fan_in": 8, "hot_window_ms": 86400000, "exit_grace_ms": 60000, "p99_cold_ms": 90000, "soft_fraction": 0.6}}
```

- Unknown keys inside `"summary"` are rejected, like the top level.
- Every number must be positive. `soft_fraction` must be strictly between 0 and 1. `fan_in` is fixed at 8 in this version.
- `tracker_prefixes` holds 1 to 8 entries of 1 to 16 ASCII letters, digits, `-` or `_` (for example `ht-`, `bd-`).
- Changing `chunk_bytes` or `tracker_prefixes` starts a new summary block generation: blocks made under the old values stay stored but are no longer used.

Observed on the earlier docs draft (base `a54008e`), not re-run at this commit: a 0600 file with `receipt_default_ms: 600000` and `invitation_default_ms: 120000` was reflected in health after ensure. With the file changed to 0644, `daemon ensure` failed with exit 3 ("daemon did not become ready within five seconds").

## Harness hooks

**Entrypoint and setup CLI: in this build.**

The hook entrypoint is `herdr-threads [--state-dir PATH] hook claude` or `hook codex`. It is hidden from `--help`. Agent panes do not inherit the plugin state directory, so the installed command passes `--state-dir`; the host endpoint comes from `HERDR_SOCKET_PATH` and the pane from `HERDR_PANE_ID`. The hook:

- selects the registered Claude or Codex input contract without resolving or probing a harness executable;
- validates the declared event, required fields, values and cooperative role before check-in or journal access; unsupported input remains quiet and contributes bounded advisory diagnostics;
- always exits 0 for invalid payload or arguments, within the tool-boundary or lifecycle deadline; it never decides permissions and never accepts or ACKs;
- makes a durable lifecycle check-in for supported SessionStart events and prints pending attention; at supported tool boundaries it prints new attention only.

[agent-usage.md](agent-usage.md#native-hooks) describes hook output. [Harness compatibility](compatibility/harnesses.md) separates operational contracts from historical recipes and captured native evidence. Ordinary hooks do not read or warm the old binary schema/admission caches. Runtime metadata is optional; `contract_declared` is not native qualification. Compact recovery, composer stash and turn-time poke stay unavailable without separate safe current-runtime qualification. Historical validation remains scoped to its recorded executable and source revision.

Setup CLI (`herdr-threads setup|unsetup|setup-status [claude|codex]`, see `setup --help` for exit statuses). Setup is **user level**, the way Herdr installs its own agent hooks (`~/.claude/settings.json` SessionStart, `~/.codex/hooks.json`). Run it once, from anywhere:

```
herdr-threads setup          # every detected harness
herdr-threads setup claude   # or one harness
herdr-threads setup codex
```

With no harness named, `setup` sets up every detected harness (each of `claude` and `codex` found on `PATH`) and prints one line per harness: `installed`, `already installed`, `skipped` (not on `PATH`) or `refused` (the selected executable or owned configuration is unavailable or invalid, with the reason). Skipped and refused are not failures: the exit status is nonzero only when a detected harness failed (it is that harness's status, and the other harness is still set up). Warnings follow the summary, and the Codex hook-trust reminder is printed once at the end. Bare `unsetup` removes both recorded installations, whether or not the harness is still on `PATH`, and bare `setup-status` reports both. `--json` returns `{"setup": {"action": "install_all"|"remove_all"|"status_all", "harnesses": [{"harness", "detected", "outcome", "report"|"reason"|"error"}], "trust_reminder", "exit_status"}}`; each `report` is the single-harness report. `--harness-binary` needs a named harness. Example:

```
$ herdr-threads setup
claude: installed (contract_declared; runtime metadata unavailable)
codex: skipped: no executable `codex` on PATH
installed is not observed: run `herdr-threads doctor` for native evidence
```

- **The Herdr instance is detected**, by setup and by every other command (`doctor`, `inbox`, `human me init`, `thread ...`, `launch`, ...) alike. `--state-dir` / `--host-endpoint` win, then `HERDR_PLUGIN_STATE_DIR` (plugin actions only) / `HERDR_SOCKET_PATH` (Herdr exports it into every pane). Then Herdr's default locations, with filesystem checks only and no subprocess: the state directory `$XDG_STATE_HOME/herdr/plugins/herdr-threads`, else `~/.local/state/herdr/plugins/herdr-threads`, when it exists, and the socket `$XDG_CONFIG_HOME/herdr/herdr.sock`, else `~/.config/herdr/herdr.sock`, when a socket exists there. With `XDG_STATE_HOME` set, the `~/.local/state` directory is a legacy location: it is chosen only when it holds a store (an `instances/<id>/` directory with `threads.sqlite3`) and the XDG directory does not, and `doctor` then shows `state_dir.source: legacy ~/.local/state (holds a store; XDG_STATE_HOME has none)`; a `~/.local/state` directory without a store is a stale leftover and never takes precedence over XDG. Both holding a store is refused as ambiguous. A named Herdr session (`HERDR_SESSION` or `HERDR_CONFIG_PATH` set) gets neither default, because its server, not the default one, owns the plugin: both the state directory and the socket then come from the flags, Herdr's environment or the `herdr` queries below. A state directory taken from the default whose plugin Herdr's registry file (`$XDG_CONFIG_HOME/herdr/plugins.json`, else `~/.config/herdr/plugins.json`, read as a file, no subprocess) does not list, or lists as disabled, is a leftover of an uninstalled plugin: `setup` refuses it (`plugin not installed (leftover state dir <path>)`, exit 2, nothing written), `doctor` reports it as a limitation, and every other command (`unsetup`, `setup-status`, `inbox`, ...) keeps working on it. A missing or unreadable registry file claims nothing. Nothing is cached. Only when a default is missing does the command ask the `herdr` CLI (read-only, 5 s bound each): `herdr plugin list --json` must list the `herdr-threads` plugin as installed and enabled, and its state directory is Herdr's plugin state root joined with the plugin id: `$XDG_STATE_HOME/herdr/plugins/herdr-threads`, else `~/.local/state/herdr/plugins/herdr-threads`. The host endpoint is the running server's socket from `herdr status server --json`. A missing or disabled plugin, two state roots that both hold a store, or a stopped server are refused (exit 2) with the reason; pass the flags then. Named Herdr sessions (`herdr --session NAME`) have their own socket and are never answered from the defaults: run inside the session's pane (Herdr exports `HERDR_SOCKET_PATH`) or pass `--host-endpoint`. Reports show what was used under `instance`.
- **The installed hook command names the instance:** `<herdr-threads> --state-dir <state> --host-endpoint <socket> hook claude|codex --event <EVENT>` (each hook group names its own event; hooks installed before `--event` existed keep working, and `doctor` suggests re-running `setup`). A herdr-threads build from before per-event registration rejects the `--event` form: to downgrade herdr-threads, run `herdr-threads unsetup <harness>` with the newer build first. Because a user-level hook runs in every Claude or Codex session, the hook first checks the environment: outside a Herdr pane (`HERDR_ENV`/`HERDR_PANE_ID` unset), or in a pane of another Herdr server (`HERDR_SOCKET_PATH` is not the recorded socket), it prints nothing, runs no harness version probe, starts no daemon and exits 0. It still reads the payload (for at most 200 ms) and, when its per-session gate says so, sends one best-effort harness evidence note (300 ms budget) to a daemon that is already running; its gate files live under `<state>/harness/evidence` (see [Version evidence](compatibility/harnesses.md#version-evidence)).
- `setup claude` writes the SessionStart and Bash PreToolUse hook groups beside existing groups in `$CLAUDE_CONFIG_DIR/settings.json` (default `~/.claude/settings.json`), with independent hook ownership under `<state>/setup/`. Hooks grant no command permission; the separate permission component does (see [Agent permissions](#agent-permissions)). A Claude session using another `CLAUDE_CONFIG_DIR`, or excluding user settings with `--setting-sources`, does not load these settings.
- **Claude prompt suggestions.** After installing the hooks, `setup claude` checks Claude's `promptSuggestionEnabled` in the same `settings.json`. Claude shows a dim prompt suggestion in its input box after every turn by default, and herdr-threads cannot tell it from typed text, so it never pokes a Claude pane that shows one (only the hard-deadline warning reaches it). Unless the key is already `false`, setup explains this and, on an interactive terminal, asks `Disable prompt suggestions? [y/N]`; only a yes writes `promptSuggestionEnabled: false`. Without a terminal it changes nothing and prints the advice as a warning. `--disable-prompt-suggestions` / `--keep-prompt-suggestions` decide without asking (for scripted installs; `setup` and `setup claude` only). The write is recorded like the hooks, `unsetup claude` reverts it only if setup set it and it still reads `false`, and `setup-status claude` and `doctor` (`hooks.claude.prompt_suggestions`) report the current state. See [the Claude integration notes](../integrations/claude/README.md#prompt-suggestions).
- `setup codex` writes the three hook groups (SessionStart, SubagentStart, Bash PreToolUse) into `$CODEX_HOME/hooks.json` (default `~/.codex/hooks.json`), appended after the groups already there, and adds no sandbox allowance to `$CODEX_HOME/config.toml`; its permission component owns `$CODEX_HOME/rules/herdr-threads.rules` ([Agent permissions](#agent-permissions)). Codex also loads hooks from `config.toml`, `/etc/codex` and trusted project `.codex/` layers; setup reads those to report them and refuses (exit 1) when one already runs the identical herdr-threads hook command, which would otherwise run twice. A Codex session with another `CODEX_HOME`, or `codex exec --ignore-user-config`, does not load them.
- **Ownership and safety.** Every owned hook command carries a `# herdr-threads-owner:<id>` marker and is recorded, with the exact baseline bytes, in a private manifest under `<state>/setup/` (keyed by the file path). Setup refuses (exit 1) an unowned identical hook, an owned hook edited by hand, a damaged or edited recorded legacy allowance, or a file that changed during setup; every file is replaced crash-safe (temp file, fsync, rename) after an unchanged-baseline check, and an invalid file, or a symlinked `settings.json` / `hooks.json` / `config.toml`, is refused (exit 2): setup does not follow the link, so neither the link nor its target is changed (to manage a dotfile that is a symlink, point `CLAUDE_CONFIG_DIR` / `CODEX_HOME` at the directory that holds the real file). Re-running setup is idempotent (`action: already_installed`). `setup codex` validates recorded legacy ownership before installing hooks and never writes a new sandbox allowance. Re-running setup from a binary at another path than the recorded hook command's is refused (exit 1) with a message naming the recorded executable and the current one; run `herdr-threads unsetup <harness>` (from either binary: it removes the recorded groups whatever executable they name), then `setup` from the binary you keep. `unsetup claude|codex` removes only what setup added, permission rules included: byte for byte when nothing else changed, structurally otherwise, and a file setup created is deleted again when it is back to its created state.
- **Backups.** Before replacing or deleting user config (Claude `settings.json`, Codex `hooks.json`, `config.toml`, the rules file), setup keeps the previous bytes beside it as `<name>.<UTC timestamp>-<uuid>.herdr-threads`, mode 0600. One command keeps one backup per file (its state before that run), however many components write it. Backups are never pruned; an empty or `{}` file is not backed up. Because backups stay, a config directory setup created can remain after `unsetup`, holding only backups.
- `setup-status claude|codex` reports `installed` (from the manifest and the file) separately from `observed` (only native evidence proves delivery; see `doctor`), the recorded command and whether it matches this executable and instance, the `permissions` object (see below), the allowance and the hook trust keys (Codex), and the declared contract with optional runtime metadata; installation is not native observation.
- Setup resolves the selected executable without running it. Reports use `admission: contract_declared` and `version: null` when metadata is unavailable. Doctor likewise separates hook configuration from runtime/native evidence; no ordinary version/help/schema probe is required. Missing executables or invalid ownership remain actionable refusals, and an explicit harness executable must still be absolute and executable. Historical recipe/version qualification remains available in explicit diagnostic tooling; it does not grant optional current-runtime capabilities.

**Codex hook trust.** Codex runs a hook from `hooks.json` only once it is trusted: `codex app-server` `hooks/list` reports a freshly installed user hook as `trustStatus: untrusted` (checked on 0.159.2 with a scratch `CODEX_HOME`). The next interactive `codex` start shows that hooks need review (or open `/hooks`); trusting them makes Codex record each hook's hash in `config.toml` as `[hooks.state."<hooks.json path>:<event>:<group>:<hook>"] trusted_hash = "sha256:..."`. That is also how Herdr's own `hooks.json` entry is trusted: Herdr writes the hook (and `features.hooks = true`), and Codex's review records the trust; Herdr writes no trust hash. Re-running `setup codex` over hooks installed before per-event registration rewrites their commands, so Codex asks to review them again. `setup codex` likewise never writes trust; `setup-status codex` lists the owned groups' trust keys and whether a `trusted_hash` is recorded for each (the hash itself is Codex's and is not verified). `doctor` reports a missing recorded key as `review_required`, a present record as `recorded_unverified`, and an unreadable or malformed config as `unknown`; it cannot prove current Codex trust or native hook execution. `doctor fix` names the manual review step without writing trust. The installer puts the one-time interactive review guidance first after Codex hook setup and uses color for status and important steps only on a supporting terminal (`NO_COLOR` disables it; redirected output is plain). Setup appends its groups, so the keys (positions) of hooks you already trusted do not change; `unsetup codex` warns when groups after a removed one move up and will need review again. `codex exec` cannot review: trust the hooks once interactively, or, in scratch only, pass `--dangerously-bypass-hook-trust` (the recorded Codex demos and evidence runs did, in scratch homes). `launch` never adds it.

### Agent permissions

Hooks and permissions are separate components. Hook setup grants nothing; the permission component owns these rules and records them in its own manifest under the state directory:

- **Claude** (`settings.json`): allow `Bash(herdr-threads)`, `Bash(herdr-threads *)`, `Bash(ht)` and `Bash(ht *)`; ask, bare and with ` *`, for each spelling followed by `human`, `setup`, `unsetup`, `doctor fix` or `internal installer-integrations`. Claude applies ask before allow. A rule the user already had is recorded as pre-existing and never removed.
- **Codex**: setup owns the whole file `$CODEX_HOME/rules/herdr-threads.rules`: one execpolicy `prefix_rule` allowing `["herdr-threads","ht"]`, plus `prompt` rules for `human` and the same self-granting commands. A file at that path that setup did not write, or one someone edited, is reported (`state: foreign` / `edited`) and never overwritten or deleted.
- **Hermes** needs no grant: the shipped plugin's `pre_tool_call` hook returns `approve` (Hermes asks the person) before a terminal command runs `herdr-threads`/`ht` with `human`, `setup`, `unsetup`, `doctor fix` or `internal installer-integrations` ([Hermes notes](../integrations/hermes/README.md)).

Those commands ask because the person namespace acts for the human and the others could let an agent grant itself permissions. Outside the `human` namespace the CLI requires those words first, from anyone: `herdr-threads setup claude --json` works, but `herdr-threads --json setup claude` is refused, and `human` must be the first word after the executable. This is cooperative ([trust policy](../TRUST-POLICY.md)): quoting words to dodge a text rule is outside the model. Stronger deny/ask/managed rules remain authoritative, and bare names cannot attest later PATH, alias or function resolution. Setup changes no general shell allowance, network permission, sandbox setting or approval mode.

Consent:

- `setup <harness> --with-permissions` grants; `--without-permissions` removes the rules setup owns and keeps the hooks. Bare `setup` (every harness) takes the same flags.
- Plain `setup` keeps what is there. When nothing is granted it asks on a terminal, `Let claude agents run herdr-threads commands without prompting? Person and setup commands still ask [Y/n]` (naming each harness) (default yes); without a terminal it reports `permissions.action: advised` with a note.
- `unsetup` removes the rules together with the hooks. A permission problem never stops `unsetup` from removing the hooks.

**Upgrade from older setups.** Plain `setup claude` takes over an earlier setup's owned `Bash(herdr-threads *)` in place, without consent, and adds the ask rules; a retired `Bash(export HERDR_THREADS_CALLER_CONTEXT=*)` is replaced. A broad rule the user added is left and reported as `foreign_broad_rules`. A rule the user deleted is not re-added, and a hand-removed rule no longer makes `setup claude` fail. `ht` rules still need consent.

**Reporting.** `setup`, `setup-status` and `doctor` report a `permissions` object: `state` is `installed`, `not_installed`, `historical` or `interrupted` (plus `edited` / `foreign` for Codex), and `action` is `granted`, `removed`, `kept`, `advised` or `declined`. Doctor's text output has `hooks.claude.permissions:` and `hooks.codex.permissions:` lines. `allow_rule` remains only to report a historical hook-recorded rule and is null otherwise.

**Recovery.** Every permission update records its intent first; the next `setup` or `unsetup` finishes or rolls back an interrupted run. If the file was edited in between, setup refuses with guidance to review the rules and remove the permission record.

### Human commands

Ordinary root commands act as agents. Human and operator commands begin `ht human`;
put `--state-dir`, `--host-endpoint` and output flags after `human`. `--human` changes
output only. Declare a person with `ht human me init` in their shell pane, or
`ht human me init --operator` for the existing explicit local-account override.
Person communication uses `ht human send`, `ht human invite`, `ht human accept` and
`ht human ack`. Recover a retained Human/operator intent with `ht human retry REF`.
Subagents read using `--machine` or `--json` and never mutate.

For example, routing remains pinned after the namespace:

```sh
ht human --state-dir /path/to/state --host-endpoint /path/to/herdr.sock --json me init
ht human --state-dir /path/to/state --host-endpoint /path/to/herdr.sock retry local:12
```

Public reads carry no invocation actor, so daemon producers cannot infer a Human viewer
from a Human subject seat. Explicit Human CLI rendering changes only known command argv
on a clone and checks the final selected encoding against the original byte budget. If
namespace overhead cannot fit, it returns the precise required minimum including framing
and newline, keeping cursor, rows and retained response bytes unchanged. Retry routing
comes from the original frozen semantic/scope; an old Agent intent remains Agent after
binding changes. `pending-ops` retains reference-only records rather than inventing an
actor field or rewriting retained payloads.

### Codex command approvals

Managed Codex launch requires owned hooks, not a socket-policy version allowlist.
The hook and embedded skill instruct the agent to run `herdr-threads` (`ht`)
commands outside the sandbox through Codex's ordinary approval mechanism. Other
commands remain sandboxed. Launch changes neither the approval policy nor network
permissions, and never supplies a full-access or bypass flag.

Use your configured interactive Codex approval mode, for example:

```sh
herdr-threads launch --pane bob --kind codex -- "You are Bob."
```

Examples omit approval flags so they do not override your configuration or shell
wrapper. A wrapper that adds `--approve-for-me` conflicts with `-a` /
`--ask-for-approval`; omit those arguments, including corresponding handoff
`--agent-arg` options. Automatic review may still refuse a request.

The owned rules file ([Agent permissions](#agent-permissions)) allows ordinary
commands; person and setup commands still prompt. Do not broaden this into a shell grant.
Permission rules and managed restrictions remain authoritative; an agent reports a
refusal rather than bypassing it. Guidance itself grants no permissions.
For socket denials or unavailable approval, see
[Codex troubleshooting](../integrations/codex/README.md#troubleshooting).

Codex 0.160.0's noninteractive `exec` forces approval policy `never`: explicit
escalation requests are rejected there. For `exec`, grant the owned rules
(`setup codex --with-permissions`) before starting and use ordinary shell calls without escalation-only arguments.
The native probe verified that this routes CLI calls outside the sandbox while
forbidden rules still reject them. Without a rule, the ordinary call stays sandboxed
and reports `transport_denied`. Interactive TUI sessions can request approval.
Lifecycle hooks run through Codex itself; isolated native testing confirmed a
SessionStart hook and an approved shell call both reached a private daemon with
no socket/network allowance. Forbidden command rules and approval `never` refused
shell execution as expected. See [native evidence](evidence/codex-command-approvals/README.md).

[Codex command rules](https://learn.chatgpt.com/docs/agent-configuration/rules)
control command-specific outside-sandbox execution.

### Codex sandbox socket allowance

This section records a legacy transport, not a current installation instruction. Current `setup codex` installs hooks only: it adds no socket, network or writable-root allowance. Use approved CLI-only outside-sandbox execution described [above](#codex-command-approvals); restarting the daemon does not fix a sandbox policy denial.

Earlier setup versions wrote instance-specific network-proxy socket and journal-root settings. Their default-deny behavior was measured only on Codex 0.159.2/0.159.3; it must not be inferred for an unknown wrapper or build. Historical measurements remain in the [socket probe](evidence/codex-1593-sandbox-probe/README.md) and [write probe](evidence/codex-sandbox-writes-probe/README.md). Do not enable broad networking or copy those settings as a workaround.

Existing valid owned legacy records are inspected before hook updates and preserved. Partial, edited, damaged or symlinked ownership records refuse before writes. `unsetup codex` retains its recorded ownership-based removal behavior; it never removes unrelated settings. Doctor reports policy validation as not run and withholds global automatic repair. Unknown runtime metadata grants no socket-policy validation or native capability.

`herdr-threads check-in` remains the manual lifecycle entry for a launch driver: `--lifecycle-event ID` plus the `--cooperative-*` flags (see [agent-usage](agent-usage.md)). A child agent must never run it.

## Managed launch

Seat attribution requires hooks and tool commands to inherit the pane running the TUI.
A shared harness server started in another pane can supply that server's pane instead.
`setup` and `setup-status` inspect the user configuration and explain the foreground
requirement; they do not change these settings. The user configuration is an advisory
check, because wrappers and higher-precedence settings can change the effective mode.

For Claude, set `"disableAgentView": true` in
`$CLAUDE_CONFIG_DIR/settings.json` (default `~/.claude/settings.json`). The equivalent
user-settings entry is `"env": {"CLAUDE_CODE_DISABLE_AGENT_VIEW": "1"}`. This disables
background agents and their on-demand supervisor. `CLAUDE_CODE_DISABLE_BACKGROUND_TASKS`
controls tool backgrounding and is not the supervisor opt-out.

Codex 0.160.1 has no verified persistent setting equivalent to native `--no-daemon`.
`[features] daemon_auto_start = false` prevents automatic startup but still permits
attachment to an existing server. Configure your existing native launcher or wrapper
to keep execution in the TUI process. For a Codex entrypoint that accepts the native flag,
herdr-threads can add it explicitly:

```sh
export HERDR_THREADS_CODEX_OPTS='--no-daemon'
```

Leave this unset when your wrapper rejects the flag; configure that wrapper's supported
foreground mode instead. The measured tool and hook behavior and setting sources are
documented in [wrapper compatibility](compatibility/wrapper-seat-detection.md).

`herdr-threads launch --pane PANE --kind claude|codex [--name NAME] [-- AGENT_ARG...]` starts Claude or Codex in one explicit existing Herdr pane that is at its interactive shell prompt. It never creates, splits or picks a pane, and never types into an occupied one.

Before anything starts, launch:

1. resolves the selected executable or wrapper and validates the owned declared hook contract without running version/help/schema probes; missing or nonexecutable selections refuse, and unknown runtime metadata stays unknown;
2. reads the pane afresh through the Herdr API and refuses a known-occupied pane;
3. resolves the pane's seat through the daemon's ordinary `seat resolve` path, so a recovery-held target refuses until `ht human seat rebind ... --operator` or a fresh seat;
4. requires the owned user-level installation in the Claude settings / Codex `hooks.json` of the directory the agent will use: Herdr's `agent start` carries no environment, so the agent inherits the pane shell's. Launch probes a separate interactive `$SHELL -ic` for an absolute `CODEX_HOME` / `CLAUDE_CONFIG_DIR`, else uses the launcher's resolved value (or `HOME`); the report's `config_dir` names the inspected directory and its source. That separate shell does not prove the target pane's effective environment or executable. If the installation is absent in the inspected directory, launch refuses and names the setup command. Codex commands use approved outside-sandbox execution; launch does not require a socket allowance or a measured sandbox-policy version;
5. reads the pane once more, then asks Herdr's guarded `agent start` to start the agent. Herdr itself refuses a pane that is not an available shell, or a name another live agent already holds.

The Herdr agent gets a readable name: `--name NAME` when given, else the pane's Herdr label (or its tab's label when the tab holds only that pane), else `seat-<short seat id>` (for example `seat-k3fq9a2b`). The name is fitted to Herdr's rule `[a-z][a-z0-9_-]{0,31}`: lowercased, other characters become `-`, anything before the first letter is dropped, and it is cut to 32 bytes (a `--name` with no letter is refused; a `--name` that had to change is reported in `warnings`). Launch correlates Herdr's start and readiness answers with that exact name. When Herdr refuses it as `agent_name_taken`, launch retries once with `-<short seat id>` appended, never more. The report and `launches.jsonl` record `agent_name` and `agent_name_source` (`name`, `pane_label` or `seat`); after exit 5 they list both `agent_name_candidates`.

The caller's arguments after `--` remain unchanged and in order: the owned configuration is on disk, so launch adds no hook or sandbox arguments. By default launch adds no daemon-mode argument and does not inspect shell functions or aliases to deduplicate one. This lets the pane's `codex` wrapper select its own supported mode: a wrapper script may reject `--no-daemon` even when native Codex accepts it. Explicit caller arguments remain unchanged, including a top-level `--no-daemon` for a native CLI that supports it. A Codex setup plan adds only its owned session `-c` overrides, inserted at the level that loads hooks (interactive, `exec`, or `exec resume`). Other subcommands (including top-level `resume`, `exec fork` and `exec review`), an explicit `--daemon`, duplicate or misplaced caller `--no-daemon`, and a caller `-c hooks.*` or whole-table `-c hooks=...` override that would replace the owned hook are refused. A separated `-i FILE` / `--image FILE` is refused too, because the option takes several values and would swallow a following subcommand or prompt: write `--image=FILE`, or put it after `--`. No auto-approve flag is added.

Optional `HERDR_THREADS_CODEX_OPTS` and `HERDR_THREADS_CLAUDE_OPTS` add arguments for the
selected harness only. They are read from the environment of the herdr-threads command,
split using shell-style quotes, and placed before the caller's arguments so top-level
flags such as `--no-daemon` precede a subcommand or prompt. Unset or empty variables add
nothing. Values undergo no variable, wildcard or command expansion: `$HOME` and
`$(command)` remain literal arguments. Invalid quoting, non-UTF-8 values and NUL bytes
are refused before any launch or handoff effect. Existing launch validation also applies
to configured arguments; conflicting hook overrides, duplicate `--no-daemon` and
unsupported forms remain refused. A wrapper that supplies its own flag needs no duplicate
here.

For example:

```sh
export HERDR_THREADS_CODEX_OPTS='--no-daemon --model "my model"'
export HERDR_THREADS_CLAUDE_OPTS='--model sonnet'
```

`handoff` freezes these arguments with the durable handoff before preflight and effects;
retry uses the frozen arguments even if the environment later changes.

Launch watches the start for up to its observation window (30 s). Herdr keeps an agent that exits straight away (for example Codex refusing its arguments with a usage error) `launch_pending` with no detected agent, so once a second after the first one launch also reads the pane's last lines: when the harness's command line was echoed and the pane is back at a shell prompt, launch fails at once with `invalid_request` quoting those lines (nothing is running) instead of waiting out the window and exiting 5.

For Codex the report carries a `codex` block: `codex_home` (the effective `CODEX_HOME`), `config_path` (its `config.toml`, with `config_present`), and the selected `profile` with `profile_source`. That answers "why was my Codex profile not applied?": Codex applies `-p/--profile NAME` from the command line first (the last one wins), else the top-level `profile = "NAME"` in the `config.toml` of that `CODEX_HOME`, else only the plain top-level settings (reported as `profile: default`, `profile_source: none`). A profile that is not applied usually means launch, and so the agent, used a different `CODEX_HOME` than expected: compare `codex.codex_home` and `config_dir.source` with the directory you edited. The same block is appended to `launches.jsonl`.

Exit 0 means Herdr detected the agent ready in that pane. Exit 5 means the start may have happened but readiness was not confirmed (for example a trust dialog blocked startup): inspect the pane with `herdr agent get` / `herdr agent read` before launching again. Every submitted start is appended to `<state>/instances/<hash>/launches.jsonl`.

Launch is not receipt: it never checks in, accepts or ACKs. Invitations and messages sent before launch stay pending. The agent's SessionStart hook shows them, and `herdr-threads inbox --seat SEAT` lists them even if the agent's initial prompt is lost. A manually started agent with the same hooks behaves the same way, with one difference: after an observed start, `launch` asks the daemon to record a `managed_launch` binding on a seat that has none (report `binding`; `seat inspect` shows `open_binding_state: launched, not checked in`). It is not a registration and authorizes nothing but a wake prompt to the launched harness, so an idle agent that has not checked in yet is still woken for pending work. Codex 0.159.3's TUI runs its SessionStart hook only at the first turn, so a Codex agent launched without a prompt depends on it. The agent's first lifecycle check-in replaces it ([trust policy](../TRUST-POLICY.md) A3).

Evidence: deterministic fake-host and real-daemon tests and a real-host smoke on Herdr 0.9.1 with a harmless stand-in executable; native, the managed-launch prelaunch handoff **passed** on Codex 0.159.2 (`native-codex-matrix-2/live-managed`, including restart and resume) and Claude Code 2.1.285 (`native-claude-matrix-1/run-managed`); managed launch was not re-run on Claude 2.1.286 (run folders archived in git tag `archive/herdr-threads-run-2026-09-26` under `docs/superpowers/runs/2026-09-26-herdr-native-mailbox-thread-plugin/`). These rows describe the code at their run SHAs; see the [validation report](validation/report.md).

## Updating

1. Run `daemon stop` with the executable that is currently installed, before rebuilding. A rebuild replaces `bin/herdr-threads` in place.
2. Rebuild (`./scripts/build.sh`) or reinstall the plugin.
3. Run `daemon ensure`, then `doctor`.

The prebuilt-release installer (`scripts/install.sh`) does step 1 itself: on an upgrade with the plugin linked from its install directory and the Herdr server running, it invokes Herdr's `stop` action while the old package is still in place, so the old executable stops the old daemon; then it swaps the package and runs `ensure` with the new one. If that stop fails while a daemon is still running, it does not hide it: it warns with the daemon's pid (read from the instance's `endpoint.json` under Herdr's default plugin state directory) and the manual recovery, then retries `stop` with the new executable (skew-tolerant, see below) and runs `ensure`. When `ensure` still fails it repeats the pid and the hint. Without a running Herdr server the installer runs the old executable's `daemon stop` directly, and warns the same way if a live daemon remains. When the plain `daemon stop` fails (for example with `herdr` off `PATH` and `HERDR_CONFIG_PATH` set, where the state directory cannot be resolved), the installer retries once per live instance with `--state-dir` and `--host-endpoint` taken from that instance's directory and its `locator` file; this works without `herdr` on `PATH`. A daemon still alive at the end makes the run (upgrade or uninstall) exit 3 with a final status line naming the pids and a `kill` remedy.

**Manual recovery when the old executable is gone.** `daemon stop` from the new executable works across a protocol change: on a mismatch it sends the old daemon no request, confirms the owner lock and the endpoint descriptor's pid and boot, and signals that pid (SIGTERM), then waits for the lock to be released. If even that fails, stop the daemon by its pid: the `pid` field of `STATE_DIR/instances/<sha256 of HOST_SOCKET>/endpoint.json` (by default `~/.local/state/herdr/plugins/herdr-threads/instances/*/endpoint.json`): `kill PID`, and check that it exited before going on. Its owner lock is released with the process, so `ensure` treats the leftover descriptor as stale. Then run `daemon ensure` (or `herdr plugin action invoke ensure --plugin herdr-threads`) with the new executable and `doctor`. The state directory and its data are kept; the new daemon takes over the same instance.

A newer executable does not take over a running older daemon. `daemon ensure` and the startup entry exit 3 with `daemon_version_mismatch` and the hint to stop then ensure, start no second daemon, and `doctor` exits 3 with `result: daemon_version_mismatch`. Within one protocol version, the `stop` action from the **newer** install reaches and stops the older daemon, after which `ensure` starts the new version with the same instance and the threads and seats still present. Across a protocol change (protocol 2 added the expected daemon boot; the real release skew pair is protocol 1 against 2) every other command from the newer executable stops with `unknown_wire_version` and the remedy `daemon stop` then `daemon ensure`, because the older daemon cannot decode its requests; the newer `daemon stop` is skew-tolerant (it signals the published pid instead of sending a wire `Stop`), so that remedy works from the newer executable too. Step 1 remains the clean path. Reinstalling keeps the state directory and its data.

## Removing

Herdr has no shutdown hook, so disabling or uninstalling does not stop the daemon.

1. `daemon stop`.
2. Remove the owned harness entries: `herdr-threads unsetup` (both harnesses; or `unsetup claude` / `unsetup codex`) (with the same `CLAUDE_CONFIG_DIR` / `CODEX_HOME` and Herdr instance as setup). Do this before step 3: once the plugin is uninstalled or disabled, `herdr plugin list` no longer names its state directory, and `unsetup` fails with `state directory unknown` unless the default state directory still exists or you pass `--state-dir` (the manifests that record what to remove live under `<state>/setup/`). This removes the owned permission rules too. Unchanged files are restored byte for byte; Codex's own `[hooks.state]` trust entries are Codex state and stay, as do setup's `.herdr-threads` backups.
3. Unlink or uninstall the plugin in Herdr.

State is preserved. There is no data-deletion command.
