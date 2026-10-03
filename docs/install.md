# Installation and local setup

> Pre-release. Nothing here has been published. The package lifecycle (install, startup, actions, update, link) passed an isolated gate against a private Herdr 0.9.1 server ([validation/package.md](validation/package.md)), but these steps have not been run as a tutorial against a shared, everyday Herdr session. Native support is summarized from the [validation report](validation/report.md) (verdict **PASS_WITH_GAPS**); see the [README support table](../README.md#supported-configurations).

## Prerequisites

- macOS on arm64. The manifest declares `platforms = ["macos", "linux"]`, which names no architecture; only macOS arm64 has been exercised, Linux is unverified (see [below](#installing-a-prebuilt-release)) and no other platform or architecture is claimed.
- Herdr exactly 0.9.1. The manifest's `min_herdr_version = "0.9.1"` is a floor only; the host adapter requires version `0.9.1` and protocol 22 and refuses any other server as `unsupported` ("host API version or protocol mismatch").
- A Rust toolchain with Cargo. CI pins Rust 1.94.0 (edition 2024), the version used for local checks.
- `Cargo.lock` is committed and every build uses `--locked`.
- A harness version an adapter recipe admits (setup, the hook and `launch` refuse any other; `doctor` prints both registries):
  - Claude Code 2.1.283 to 2.1.287 inclusive (recipe `claude-hooks-2.1.283`).
  - Codex exactly 0.157.1 or 0.158.0 (recipe `codex-hooks-v1`), or an unlisted version whose embedded hook schemas match that recipe's fingerprint. Codex 0.159.2 is admitted that way and reported as **schema-matched, live-unverified**. The sandbox socket allowance is printed only for 0.159.2 (see [below](#codex-sandbox-socket-allowance)).
- `herdr-threads` on the agent's `PATH`: the hook's ready commands name it bare.

## Installing a prebuilt release

Once a release is published (none is yet), the installer fetches a prebuilt archive instead of building from source:

```sh
curl -fsSL https://raw.githubusercontent.com/alepar/herdr-threads/main/scripts/install.sh | bash
# options go after `bash -s --`, e.g. a pinned version and hook setup:
curl -fsSL https://raw.githubusercontent.com/alepar/herdr-threads/main/scripts/install.sh | bash -s -- --version v0.1.0 --setup
```

It detects the OS and architecture, downloads `herdr-threads-OS-ARCH.tar.gz` and `SHA256SUMS` from the GitHub release (latest, or `--version`), refuses a checksum mismatch, installs the package into `~/.local/share/herdr-threads` (`--prefix`) and links `~/.local/bin/herdr-threads` (`--bin-dir`) to its executable, warning when that directory is not on `PATH`. It then registers the package with `herdr plugin link`. Herdr does not build a linked plugin, and the archive carries a `PREBUILT` marker that makes the manifest's build command (`scripts/build.sh`) keep the shipped executable instead of running cargo. When the Herdr server is running it runs the `ensure` action; otherwise the daemon starts with the next server start. On an upgrade it first runs the `stop` action **before** replacing the package, so the still-installed old executable stops the old daemon (a newer executable never takes over an older daemon, and across a wire-protocol change it cannot even stop it). See [Updating](#updating) for what happens when that stop fails.

Harness hooks: when a harness is found on `PATH` (`claude`, `codex`) it runs bare `herdr-threads setup` (every detected harness, one summary line each) with `--setup`, asks once on a terminal by default, and only prints the command when there is no terminal or with `--no-setup`. It runs setup only when the installed build's setup is user level; a build whose setup is per project (`--project`, default the current directory) is never run from the installer.

Re-running converges: an identical archive leaves the installation, link and registration alone; a different one replaces the package directory crash-safe at the same path (the old tree is renamed aside, the new one renamed in, and the old one restored if that fails), so the Herdr registration stays valid. It refuses to replace a directory it did not create (no `.herdr-threads-install` marker) or a regular file at the symlink path (`--force` replaces the file). An existing `herdr-threads` registration from another root (for example `herdr plugin install`) is left alone with a warning.

`--uninstall` stops the daemon, runs bare `unsetup`, which removes every recorded harness installation (user-level builds only; on a terminal it asks first, without one it needs `--yes`, otherwise the hooks are kept and the installer says so; `--no-setup` skips it), unlinks the plugin, and removes the package directory and the symlink. Unlinking needs a running Herdr server; without one the uninstall stops with an error, and `--force` removes the files anyway and tells you to unlink later. Daemon state in Herdr's plugin state directory is kept.

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

`herdr-plugin.toml` (id `herdr-threads`, name `Threads`, version 0.1.0) declares:

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
- `doctor` then exits 0 with `result: degraded` (the host is unavailable), and lists `hooks.claude.recipes: claude-hooks-2.1.283 [2.1.283, 2.1.287]` and `hooks.codex.recipes: codex-hooks-v1 {0.157.1, 0.158.0, 0.159.3}`.
- `daemon stop` exits 0 with `stop_accepted`, and `daemon health` afterwards exits 3 with `host_unavailable`.
- With `instances/` changed to 0755, `doctor` exits 2 (`result: unsafe_state_dir`) and `daemon ensure` exits 2 (`unsafe private directory`).

## Settings: default deadlines and wake spacing

Per-operation deadlines use `--deadline SECONDS` on `invite` and `send`. The value must be positive. The default for both is 300 seconds.

Instance-wide defaults come from an optional `settings.json` in the instance directory (`STATE_DIR/instances/<digest>/settings.json`):

```json
{"invitation_default_ms": 300000, "receipt_default_ms": 300000, "minimum_wake_delay_ms": 30000}
```

- All keys are optional; unknown keys are rejected.
- The file must be a regular file you own, mode 0600, at most 4096 bytes.
- Durations must be positive. `minimum_wake_delay_ms` must be at least 30000.
- The daemon reads it at start. Edits apply after `daemon stop` then `daemon ensure`. `daemon health` prints the effective `settings`.
- The chosen duration is frozen on each invitation and message when it is created.

Observed on the earlier docs draft (base `a54008e`), not re-run at this commit: a 0600 file with `receipt_default_ms: 600000` and `invitation_default_ms: 120000` was reflected in health after ensure. With the file changed to 0644, `daemon ensure` failed with exit 3 ("daemon did not become ready within five seconds").

## Harness hooks

**Entrypoint and setup CLI: in this build.**

The hook entrypoint is `herdr-threads [--state-dir PATH] hook claude` or `hook codex`. It is hidden from `--help`. Agent panes do not inherit the plugin state directory, so the installed command passes `--state-dir`; the host endpoint comes from `HERDR_SOCKET_PATH` and the pane from `HERDR_PANE_ID`. The hook:

- runs the first absolute `claude` (or `codex`) on `PATH` with `--version` and parses the payload for every admitted version: listed, schema-matched (Codex only, see below) or optimistic (newer than the verified range or inside the supported span, parsed under an assumed recipe). Under optimistic admission a payload it cannot parse is counted (Health shows `N hook payloads not understood`) and logged, rate limited. It stays quiet, with one stderr line naming the version, only for a refused version.
- always exits 0 (observed for a malformed payload, a bad argv and an unsupported version), within 1.5 s at a tool boundary or 5 s at session start. It never decides permissions and never accepts or ACKs.
- at `SessionStart` (`startup`, `clear`, `resume`) makes a durable lifecycle check-in and prints pending attention plus the attention digest line.
- at a Bash `PreToolUse` boundary (and Codex `compact`) reads the seat's attention digest and prints only when something new is pending. See [operations](operations.md#attention-digest-and-notices).

[agent-usage.md](agent-usage.md#native-hooks) is the authoritative description of hook output.

Recipes in this build (the registry is `harness::recipe`; `doctor` prints both):

| Harness | Recipe and versions | Evidence scope |
| --- | --- | --- |
| Claude Code | `claude-hooks-2.1.283`: closed interval [2.1.283, 2.1.287], exactly those five versions | 2.1.283: Bash PreToolUse input and `updatedInput` rewrite observed; SessionStart startup/clear/resume input observed. 2.1.284: SessionStart startup/resume and root/subagent Bash PreToolUse input captured and parsed unchanged; hook-output application unverified. 2.1.285: same inputs parsed unchanged; in print mode root Bash `updatedInput` executed and SessionStart/PreToolUse `additionalContext` delivered. 2.1.286: the same, re-captured. 2.1.287: the native matrix manual and managed core-flow cells (ht-p03.20; [validation report](validation/report.md)). 2.1.285 and 2.1.286 permission-check a rewritten command; the production hook does not rewrite. An unattended (print-mode) session needs the allow rule `Bash(herdr-threads *)` that `setup claude` installs, or the hook's ready commands are denied. Recipe profile: model receipt unsupported for all five (see below). |
| Codex | `codex-hooks-v1`: exactly {0.157.1, 0.158.0, 0.159.3} | 0.157.1: pinned source contract and native root/child PreToolUse probe. 0.158.0: embedded hook schemas byte-identical to 0.157.1; live SessionStart startup/resume, SubagentStart and root/child Bash PreToolUse input captured; context-only `additionalContext` delivery observed for SessionStart and PreToolUse (not SubagentStart). 0.159.3: the native matrix manual and managed core-flow cells (ht-p03.20; [validation report](validation/report.md)). SessionStart `fork` is refused. Invocation transport and model receipt unsupported. |

The recipe profiles are what this build declares, and they still mark model receipt (and Codex invocation transport) unsupported, so daemon health reports both harnesses `unsupported`. Separately, native validation recorded cooperative model-issued accepts and ACKs joined to SQLite receipts on Claude Code 2.1.285 and 2.1.286 and on Codex 0.159.2 (schema-matched, live-unverified), with every required scenario passing and open gaps; none ran on Claude 2.1.283/2.1.284 or Codex 0.157.1/0.158.0. See the [validation report](validation/report.md).

An unlisted Codex version is admitted only when the hook JSON schemas embedded in its binary hash-match a recipe's captured schemas (`codex-hooks-v1`: `sha256:86858f2456c999030224a92d8dfb535183fe0edf8601690d8941978fadbb066d`, 23 schemas). Such an admission is reported as **"schema-matched, live-unverified"**: no live hook capture exists for that version. Codex 0.159.2 is admitted this way; it is the version the native Codex demos ran on, but its admission rests on the schema match, not on a recipe capture. Any other version, or schemas that do not match or cannot be extracted within the observation deadline, is refused with a message naming the supported recipes. Evidence files are listed in [compatibility/harnesses.md](compatibility/harnesses.md).

The Codex hook keeps two private files under `STATE_DIR/harness/` (a 0700 directory it creates under an existing owned state root):

- `codex-schema-cache.json`, a bounded 0600 fingerprint cache keyed by the binary's identity (canonical path, device, inode, size, mtime, ctime), so only the first observation after an install or upgrade scans the binary (about 0.9 s for 0.159.2; later hooks skip the scan);
- `codex-hook-admission.json`, the admission evidence of the latest hook observation (listed, schema-matched live-unverified, or refused).

`doctor` reads both without writing them (`hooks.codex.installed`, `hooks.codex.last_hook`). `launch --kind codex`, which is not bound by the hook's time budget, also writes the fingerprint cache, so on a slow machine the first managed launch can warm it for the hook. That warm-up is best effort: when the state root is missing or not owned, launch keeps the scan in memory and starts anyway, and a failed warm-up only costs the first hook a fingerprint scan.

Setup CLI (`herdr-threads setup|unsetup|setup-status [claude|codex]`, see `setup --help` for exit statuses). Setup is **user level**, the way Herdr installs its own agent hooks (`~/.claude/settings.json` SessionStart, `~/.codex/hooks.json`). Run it once, from anywhere:

```
herdr-threads setup          # every detected harness
herdr-threads setup claude   # or one harness
herdr-threads setup codex
```

With no harness named, `setup` sets up every detected harness (each of `claude` and `codex` found on `PATH`) and prints one line per harness: `installed`, `already installed`, `skipped` (not on `PATH`) or `refused` (the version is covered by no recipe, with the reason). Skipped and refused are not failures: the exit status is nonzero only when a detected harness failed (it is that harness's status, and the other harness is still set up). Warnings follow the summary, and the Codex hook-trust reminder is printed once at the end. Bare `unsetup` removes both recorded installations, whether or not the harness is still on `PATH`, and bare `setup-status` reports both. `--json` returns `{"setup": {"action": "install_all"|"remove_all"|"status_all", "harnesses": [{"harness", "detected", "outcome", "report"|"reason"|"error"}], "trust_reminder", "exit_status"}}`; each `report` is the single-harness report. `--harness-binary` needs a named harness. Example:

```
$ herdr-threads setup
claude: installed: claude 2.1.284 (recipe claude-hooks-2.1.283); /Users/me/.claude/settings.json
codex: skipped: no executable `codex` on PATH
installed is not observed: run `herdr-threads doctor` for native evidence
```

- **The Herdr instance is detected**, by setup and by every other command (`doctor`, `inbox`, `me init`, `thread ...`, `launch`, ...) alike. `--state-dir` / `--host-endpoint` win, then `HERDR_PLUGIN_STATE_DIR` (plugin actions only) / `HERDR_SOCKET_PATH` (Herdr exports it into every pane). Then Herdr's default locations, with filesystem checks only and no subprocess: the state directory `$XDG_STATE_HOME/herdr/plugins/herdr-threads`, else `~/.local/state/herdr/plugins/herdr-threads`, when it exists, and the socket `$XDG_CONFIG_HOME/herdr/herdr.sock`, else `~/.config/herdr/herdr.sock`, when a socket exists there. With `XDG_STATE_HOME` set, the `~/.local/state` directory is a legacy location: it is chosen only when it holds a store (an `instances/<id>/` directory with `threads.sqlite3`) and the XDG directory does not, and `doctor` then shows `state_dir.source: legacy ~/.local/state (holds a store; XDG_STATE_HOME has none)`; a `~/.local/state` directory without a store is a stale leftover and never takes precedence over XDG. Both holding a store is refused as ambiguous. A named Herdr session (`HERDR_SESSION` or `HERDR_CONFIG_PATH` set) gets neither default, because its server, not the default one, owns the plugin: both the state directory and the socket then come from the flags, Herdr's environment or the `herdr` queries below. A state directory taken from the default whose plugin Herdr's registry file (`$XDG_CONFIG_HOME/herdr/plugins.json`, else `~/.config/herdr/plugins.json`, read as a file, no subprocess) does not list, or lists as disabled, is a leftover of an uninstalled plugin: `setup` refuses it (`plugin not installed (leftover state dir <path>)`, exit 2, nothing written), `doctor` reports it as a limitation, and every other command (`unsetup`, `setup-status`, `inbox`, ...) keeps working on it. A missing or unreadable registry file claims nothing. Nothing is cached. Only when a default is missing does the command ask the `herdr` CLI (read-only, 5 s bound each): `herdr plugin list --json` must list the `herdr-threads` plugin as installed and enabled, and its state directory is Herdr's plugin state root joined with the plugin id: `$XDG_STATE_HOME/herdr/plugins/herdr-threads`, else `~/.local/state/herdr/plugins/herdr-threads`. The host endpoint is the running server's socket from `herdr status server --json`. A missing or disabled plugin, two state roots that both hold a store, or a stopped server are refused (exit 2) with the reason; pass the flags then. Named Herdr sessions (`herdr --session NAME`) have their own socket and are never answered from the defaults: run inside the session's pane (Herdr exports `HERDR_SOCKET_PATH`) or pass `--host-endpoint`. Reports show what was used under `instance`.
- **The installed hook command names the instance:** `<herdr-threads> --state-dir <state> --host-endpoint <socket> hook claude|codex --event <EVENT>` (each hook group names its own event; hooks installed before `--event` existed keep working, and `doctor` suggests re-running `setup`). Because a user-level hook runs in every Claude or Codex session, the hook first checks the environment: outside a Herdr pane (`HERDR_ENV`/`HERDR_PANE_ID` unset), or in a pane of another Herdr server (`HERDR_SOCKET_PATH` is not the recorded socket), it exits 0 at once with no output, no harness version probe, no state access and no daemon start.
- `setup claude` writes `$CLAUDE_CONFIG_DIR/settings.json` (default `~/.claude/settings.json`; created as `{}` with its directory when absent) plus a private ownership manifest under `<state>/setup/`. It adds the SessionStart and Bash PreToolUse hook groups beside the hooks already there (for example Herdr's `herdr-agent-state.sh`) and the owned allow rule `Bash(herdr-threads *)`. That rule lets Claude run any single `herdr-threads ...` command (every subcommand of this CLI, including the ready commands the hook suggests) without asking; a command chained after it is still checked separately. `herdr-threads` must be on the agent's `PATH`, because the ready commands name it bare. See [the Claude integration notes](../integrations/claude/README.md#permission-allow-rule) for exactly what the pattern covers. Re-running `setup claude` on an installation made by an earlier version replaces its owned `Bash(export HERDR_THREADS_CALLER_CONTEXT=*)` rule, which allowed nothing in use. Setup preserves unrelated keys, hooks and permission rules. A Claude session started with another `CLAUDE_CONFIG_DIR`, or with `--setting-sources` that excludes `user`, does not load these hooks.
- `setup codex` writes the three hook groups (SessionStart, SubagentStart, Bash PreToolUse) into `$CODEX_HOME/hooks.json` (default `~/.codex/hooks.json`), appended after the groups already there, and, on a Codex version whose sandbox default-deny was measured, the sandbox socket allowance described below into `$CODEX_HOME/config.toml`. Codex also loads hooks from `config.toml`, `/etc/codex` and trusted project `.codex/` layers; setup reads those to report them and refuses (exit 1) when one already runs the identical herdr-threads hook command, which would otherwise run twice. A Codex session with another `CODEX_HOME`, or `codex exec --ignore-user-config`, does not load them.
- **Ownership and safety.** Every owned hook command carries a `# herdr-threads-owner:<id>` marker and is recorded, with the exact baseline bytes, in a private manifest under `<state>/setup/` (keyed by the file path). Setup refuses (exit 1) an unowned identical hook, an owned hook edited by hand, a recorded allow rule removed by hand, a config.toml key the allowance needs that holds another value, or a file that changed during setup; every file is replaced crash-safe (temp file, fsync, rename) after an unchanged-baseline check, and an invalid file, or a symlinked `settings.json` / `hooks.json` / `config.toml`, is refused (exit 2): setup does not follow the link, so neither the link nor its target is changed (to manage a dotfile that is a symlink, point `CLAUDE_CONFIG_DIR` / `CODEX_HOME` at the directory that holds the real file). Re-running setup is idempotent (`action: already_installed`). `setup codex` installs the hooks first and the sandbox allowance second; when the allowance cannot be recorded or written, the hook installation is rolled back (hooks.json is restored, or removed when setup created it) and the error says nothing was installed. Re-running setup from a binary at another path than the recorded hook command's is refused (exit 1) with a message naming the recorded executable and the current one; run `herdr-threads unsetup <harness>` (from either binary: it removes the recorded groups whatever executable they name), then `setup` from the binary you keep. `unsetup claude|codex` removes only what setup added: byte for byte when nothing else changed, structurally otherwise, and a file or directory setup created is deleted again when it is back to its created state.
- `setup-status claude|codex` reports `installed` (from the manifest and the file) separately from `observed` (only native evidence proves delivery; see `doctor`), the recorded command and whether it matches this executable and instance, the allow rule (Claude), the allowance and the hook trust keys (Codex), and whether the installed harness version is admitted.
- Setup observes `claude --version` / `codex --version` and classifies the version: **listed** (a recipe covers it); **schema-matched, live-unverified** (Codex only: an unlisted version, not older than every recipe's minimum, whose embedded hook JSON schemas hash-match a verified recipe); **optimistic** (newer than the verified range, or inside the supported span but unlisted: admitted on an assumed recipe and labelled, for example `optimistic — newer than verified 2.1.286, assumed compatible with recipe claude-hooks-2.1.283; major version change`, with a note in Health and doctor); or **refused** (unparsable, inside a known-broken range, which names the range and the newest working version, or older than every recipe). A refused version makes `setup <harness>` exit 4 (with no harness named it is reported as `refused`, not a failure); the other three install. `doctor` reports `hooks.claude.setup_installed` and `hooks.codex.setup_installed` (JSON: `hooks.<harness>.setup.installed`) separately from `hooks.claude.observed`: installed does not mean observed. In `doctor --json`, `hooks.claude.installed` is the binary object `{binary, version, admission, recipe}`, mirroring `hooks.codex.installed`; `admission` is one of `listed`, `schema-matched, live-unverified` (Codex only), `optimistic`, `refused` or `not_found` (no such harness on PATH, with `binary`, `version` and `recipe` null). Doctor resolves the first absolute `claude` / `codex` on PATH, runs `--version` and reports its admission there; `not_found` means no such binary is on PATH.

**Codex hook trust.** Codex runs a hook from `hooks.json` only once it is trusted: `codex app-server` `hooks/list` reports a freshly installed user hook as `trustStatus: untrusted` (checked on 0.159.2 with a scratch `CODEX_HOME`). The next interactive `codex` start shows that hooks need review (or open `/hooks`); trusting them makes Codex record each hook's hash in `config.toml` as `[hooks.state."<hooks.json path>:<event>:<group>:<hook>"] trusted_hash = "sha256:..."`. That is also how Herdr's own `hooks.json` entry is trusted: Herdr writes the hook (and `features.hooks = true`), and Codex's review records the trust; Herdr writes no trust hash. `setup codex` likewise never writes trust; `setup-status codex` lists the owned groups' trust keys and whether a `trusted_hash` is recorded for each (the hash itself is Codex's and is not verified). Setup appends its groups, so the keys (positions) of hooks you already trusted do not change; `unsetup codex` warns when groups after a removed one move up and will need review again. `codex exec` cannot review: trust the hooks once interactively, or, in scratch only, pass `--dangerously-bypass-hook-trust` (the recorded Codex demos and evidence runs did, in scratch homes). `launch` never adds it.

### Codex sandbox socket allowance

Codex's default `-s workspace-write` sandbox refuses `connect()` to the herdr-threads daemon socket with `EPERM`, so the agent's `herdr-threads` commands cannot reach the daemon. The CLI reports that as `transport_denied` (exit 4), not `host_unavailable`. Codex demo 2 (Codex 0.159.2) showed both halves. Without an allowance the model stopped before accepting or ACKing anything. With the three overrides below it accepted and ACKed on its own ([demo 2 report](evidence/native-codex-demo-2/report.md)).

`setup codex` writes these three keys into `$CODEX_HOME/config.toml` for this instance's daemon socket (equivalent to the `-c` overrides the earlier session-scoped setup printed):

```
sandbox_workspace_write.network_access = true
features.network_proxy.enabled = true
features.network_proxy.unix_sockets = { "<daemon socket>" = "allow" }
```

The edit is structural (`toml_edit`): comments, layout and every other key stay as they were, an existing `[features]` table gains a sub-table instead of being redefined, and a key you already have with the same value is recorded as yours (never removed). A key with another value (say `network_access = false`) refuses setup with exit 1 and nothing written. Other keys already under `features.network_proxy` (for example `domains`, `allow_local_binding` or `dangerously_allow_all_unix_sockets = true`) are never edited, but they do nothing while `network_access` is off and the allowance turns it on: setup records them as pre-existing, and `setup`, `setup-status` and `doctor` warn `features.network_proxy.<key> was already set in <config.toml>; enabling network access for the sandbox makes it effective` for each one (`doctor`: a `limitation:` line and `hooks.codex.sandbox_proxy_warnings`). The report carries the exact path as `sandbox.socket_path`, the keys, and whether they are `present`. What each one does:

- `sandbox_workspace_write.network_access=true` makes Codex start its network proxy. The two proxy settings do nothing while network access is off.
- `features.network_proxy.enabled=true` turns on the proxy's enforcement.
- `features.network_proxy.unix_sockets` gains one entry, this instance's daemon socket. Setup adds no other socket, but entries you already had in that table stay, and they take effect too once `network_access` is on.

No domain is allowed, so on Codex 0.159.2 and 0.159.3 other network access from sandboxed commands stays denied. Demo 2 verified this default-deny on 0.159.2 before use, and a no-model `codex sandbox` probe repeated it on 0.159.3 for both the `-c` form and a `config.toml` holding only the three keys ([0.159.3 probe](evidence/codex-1593-sandbox-probe/README.md)). Under the allowance, the allowlisted daemon socket connected and `daemon health` returned 0. Other Unix sockets, the Herdr server socket, loopback TCP and external TCP were still refused (`EPERM`), and proxied HTTPS got 403. Filesystem and approval policy are unchanged.

**The default-deny was measured on Codex 0.159.2 and 0.159.3 only.** `network_access=true` is the dangerous half: on a Codex build that ignored or did not enforce `features.network_proxy`, it would leave workspace-write with unrestricted networking. So setup writes the allowance only for a measured version (0.159.2 or 0.159.3), matched by exact version string. An allowance recorded on a measured version stays quiet after an update to another measured one (0.159.2 to 0.159.3); after an update to an unmeasured one, `setup`, `setup-status` and `doctor` warn until `unsetup codex`. For any other admitted version, including the listed recipe versions 0.157.1 and 0.158.0 and any schema-matched, live-unverified version, setup installs the hooks only, reports why in `sandbox.omitted`, and adds a warning. To check your own version before adding the three keys by hand, run the negative control, which must fail (`403` or `EPERM`):

```
codex sandbox -c 'sandbox_mode="workspace-write"' -c sandbox_workspace_write.network_access=true \
  -c features.network_proxy.enabled=true -c 'features.network_proxy.unix_sockets={"<socket>"="allow"}' \
  -- curl https://example.com
```

**Writable roots for the client journals (ht-4is.8.20).** Reaching the socket is not enough. Every mutation (`send`, `ack`, `accept`, `accept-required`, `leave`, `invite`) and every `check-in` first writes this instance's client-side journals: a pending-operation intent in `<state>/instances/<hash>/intents/` (for recovery and `retry`), and the caller's context in `<state>/instances/<hash>/contexts/`. Workspace-write allows writes only under the workspace and tmp, so with the state directory in `~/.local/state` those writes fail and the command prints `herdr-threads: Operation not permitted (os error 1)`. Setup therefore also adds exactly those two directories, as canonical paths, to

```
sandbox_workspace_write.writable_roots = ["<instance>/intents", "<instance>/contexts"]
```

Each root is an owned array member: setup appends it to an array you already have, keeps a member you already listed as yours, creates the array when it is absent, and `unsetup` removes only its own members, one occurrence each (a duplicate of a root that you added by hand after setup stays), and the array, if setup created it and nothing else is left in it. A `writable_roots` that is not an array refuses setup. No root covers the instance directory, the SQLite database, the endpoint descriptor, the owner lock, the daemon log or the setup manifests. A no-model `codex sandbox` probe on 0.159.3 measured both halves ([write probe](evidence/codex-sandbox-writes-probe/README.md)). With only the socket keys, `send`, `check-in` and `accept` failed with `EPERM`. With setup's keys, check-in, send, ACK, accept, leave and invite succeeded, and writes to the database, its WAL, the endpoint descriptor, the owner lock, the state directory and a symlink escape out of a root were still denied. The report's `sandbox.writable_roots` and `sandbox.writable_roots_present` show the roots. An allowance written before the roots existed is upgraded in place by the next `setup codex`. Until then, `setup-status codex` and `doctor` warn that sandboxed mutations fail, and `launch` refuses as if the allowance were missing.

The socket path is stable across daemon restarts: it is `<state>/instances/<hash>/daemon.sock`, or `/private/tmp/herdr-threads-<uid>/<hash>.sock` when that would be too long for a Unix socket. It belongs to one state directory and one Herdr instance (host endpoint). A user-level installation serves one instance: to move to another state directory or Herdr server, run `unsetup codex` and `setup codex` with that context (setup refuses an installation recorded for another socket or hook command).

`herdr-threads check-in` remains the manual lifecycle entry for a launch driver: `--lifecycle-event ID` plus the `--cooperative-*` flags (see [agent-usage](agent-usage.md)). A child agent must never run it.

## Managed launch

`herdr-threads launch --pane PANE --kind claude|codex [--name NAME] [-- AGENT_ARG...]` starts Claude or Codex in one explicit existing Herdr pane that is at its interactive shell prompt. It never creates, splits or picks a pane, and never types into an occupied one.

Before anything starts, launch:

1. observes `<kind> --version` and places it on the admission ladder: an unparsable version, one inside a known-broken range, or one older than every recipe is refused (exit 4); a listed, schema-matched or newer optimistic version proceeds with its label;
2. reads the pane afresh through the Herdr API and refuses a known-occupied pane;
3. resolves the pane's seat through the daemon's ordinary `seat resolve` path, so a recovery-held target refuses until `seat rebind ... --operator` or a fresh seat;
4. requires the owned user-level installation in the Claude settings / Codex `hooks.json` of the directory the agent will use: Herdr's `agent start` carries no environment, so the agent inherits the pane shell's, and launch asks that shell (`$SHELL -ic`, the launcher's own `CODEX_HOME` / `CLAUDE_CONFIG_DIR` removed first) for an absolute `CODEX_HOME` / `CLAUDE_CONFIG_DIR` it exports itself, else uses the launcher's resolved value (or `HOME`); the report's `config_dir` names the directory and its `source` (`pane_shell` or `launcher`), and a pane that only inherits a different value from the Herdr server's own environment cannot be seen from here (otherwise it refuses, naming the inspected directory and the setup command); for Codex under a sandbox that refuses the daemon socket (the default `workspace-write`), also the recorded `config.toml` allowance for this instance's socket (otherwise it refuses unless the arguments after `--` choose `-s danger-full-access`);
5. reads the pane once more, then asks Herdr's guarded `agent start` to start the agent. Herdr itself refuses a pane that is not an available shell, or a name another live agent already holds.

The Herdr agent gets a readable name: `--name NAME` when given, else the pane's Herdr label (or its tab's label when the tab holds only that pane), else `seat-<short seat id>` (for example `seat-k3fq9a2b`). The name is fitted to Herdr's rule `[a-z][a-z0-9_-]{0,31}`: lowercased, other characters become `-`, anything before the first letter is dropped, and it is cut to 32 bytes (a `--name` with no letter is refused; a `--name` that had to change is reported in `warnings`). Launch correlates Herdr's start and readiness answers with that exact name. When Herdr refuses it as `agent_name_taken`, launch retries once with `-<short seat id>` appended, never more. The report and `launches.jsonl` record `agent_name` and `agent_name_source` (`name`, `pane_label` or `seat`); after exit 5 they list both `agent_name_candidates`.

The agent's arguments are the arguments after `--`, unchanged and in order: the owned configuration is on disk, so launch adds no hook or sandbox arguments. Codex gets `--no-daemon` exactly once, before any subcommand (interactive `codex`, `exec`, `exec resume` and `resume` are accepted). Herdr starts `codex` by name in the pane's interactive shell, so launch first asks that shell how it resolves `codex` (`$SHELL -ic`, falling back to `/bin/zsh`; `whence -f codex` for zsh, `type codex` otherwise; bounded to 3 s): when a user function or alias already passes `--no-daemon`, launch adds none (and drops a caller's own top-level one) and reports `codex_wrapper`; if the probe fails or times out, launch adds the flag as usual. The launch line a Codex setup plan composes (`CodexSetupPlan::launch_argv_for`) follows the same rule, so a line built for a pane whose `codex` function already passes `--no-daemon` carries none (Codex refuses the flag twice). Any other Codex subcommand (including `exec fork` and `exec review`), an explicit `--daemon`, a `--no-daemon` after the subcommand, and a caller `-c hooks.*` or whole-table `-c hooks=...` override that would replace the owned hook are refused. A separated `-i FILE` / `--image FILE` is refused too, because the option takes several values and would swallow a following subcommand or prompt: write `--image=FILE`, or put it after `--`. No auto-approve flag is added.

Launch watches the start for up to its observation window (30 s). Herdr keeps an agent that exits straight away (for example Codex refusing its arguments with a usage error) `launch_pending` with no detected agent, so once a second after the first one launch also reads the pane's last lines: when the harness's command line was echoed and the pane is back at a shell prompt, launch fails at once with `invalid_request` quoting those lines (nothing is running) instead of waiting out the window and exiting 5.

For Codex the report carries a `codex` block: `codex_home` (the effective `CODEX_HOME`), `config_path` (its `config.toml`, with `config_present`), and the selected `profile` with `profile_source`. That answers "why was my Codex profile not applied?": Codex applies `-p/--profile NAME` from the command line first (the last one wins), else the top-level `profile = "NAME"` in the `config.toml` of that `CODEX_HOME`, else only the plain top-level settings (reported as `profile: default`, `profile_source: none`). A profile that is not applied usually means launch, and so the agent, used a different `CODEX_HOME` than expected: compare `codex.codex_home` and `config_dir.source` with the directory you edited. The same block is appended to `launches.jsonl`.

Exit 0 means Herdr detected the agent ready in that pane. Exit 5 means the start may have happened but readiness was not confirmed (for example a trust dialog blocked startup): inspect the pane with `herdr agent get` / `herdr agent read` before launching again. Every submitted start is appended to `<state>/instances/<hash>/launches.jsonl`.

Launch is not receipt: it never checks in, accepts or ACKs. Invitations and messages sent before launch stay pending. The agent's SessionStart hook shows them, and `herdr-threads inbox --seat SEAT` lists them even if the agent's initial prompt is lost. A manually started agent with the same hooks behaves the same way.

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
2. Remove the owned harness entries: `herdr-threads unsetup` (both harnesses; or `unsetup claude` / `unsetup codex`) (with the same `CLAUDE_CONFIG_DIR` / `CODEX_HOME` and Herdr instance as setup). Do this before step 3: once the plugin is uninstalled or disabled, `herdr plugin list` no longer names its state directory, and `unsetup` fails with `state directory unknown` unless the default state directory still exists or you pass `--state-dir` (the manifests that record what to remove live under `<state>/setup/`). Unchanged files are restored byte for byte; Codex's own `[hooks.state]` trust entries are Codex state and stay.
3. Unlink or uninstall the plugin in Herdr.

State is preserved. There is no data-deletion command.
