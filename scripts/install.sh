#!/usr/bin/env bash
# Herdr Threads installer: installs a prebuilt release from GitHub.
#
#   curl -fsSL https://raw.githubusercontent.com/alepar/herdr-threads/main/scripts/install.sh | bash
#   curl -fsSL .../install.sh | bash -s -- --version v0.1.0 --setup
#   curl -fsSL .../install.sh | bash -s -- --uninstall
#
# What it does (each step is idempotent; re-running converges):
#   1. detects the OS (macOS, Linux) and architecture (aarch64, x86_64);
#   2. downloads herdr-threads-OS-ARCH.tar.gz and SHA256SUMS from the release
#      (latest, or --version) and verifies the checksum;
#   3. on an upgrade, stops the running daemon with the still-installed old
#      executable before replacing anything (Herdr's stop action, else
#      `herdr-threads daemon stop`, retried per live instance with its state
#      directory and host endpoint named) and, if it cannot, warns with its
#      pid and ends with exit 3;
#      installs the package into ~/.local/share/herdr-threads (replaced
#      crash-safe: the old tree is renamed aside, the new one renamed in, and
#      the old one restored if that fails; an identical install is left
#      alone) and links ~/.local/bin/herdr-threads to its executable;
#   4. registers the package with Herdr (`herdr plugin link`; Herdr does not
#      build a linked plugin, and the package's build command keeps the
#      prebuilt binary), and ensures the daemon if Herdr runs;
#   5. optionally runs `herdr-threads setup` (every detected harness)
#      (claude, codex): asks on a terminal, or --setup / --no-setup;
#   6. prints the next steps and ONE final status line.
#
# The final status line and the exit status (docs/install.md has the table):
#   0  installed and linked / upgraded / uninstalled
#   3  done, but not everything: installed, not linked; setup incomplete;
#      uninstalled, not unregistered; the old daemon could not be stopped and
#      is still running (the line names the reason or the pids, and the
#      exact command to finish)
#   1  bad arguments, download or verification failure, any refusal
#
# --uninstall stops the daemon, removes the harness setup this tool owns
# (asks first on a terminal; without one it needs --yes), unlinks the plugin
# from Herdr and removes the files above. The daemon's state (threads,
# receipts) in Herdr's plugin state directory is kept.
#
# Environment overrides (flags win): HERDR_THREADS_VERSION,
# HERDR_THREADS_RELEASE_URL (a mirror or file:// tree laid out like GitHub
# releases: latest/download/ASSET and download/TAG/ASSET),
# HERDR_THREADS_HOME (install dir), HERDR_THREADS_BIN_DIR, HERDR_BIN (herdr).
# The whole script is one brace group: bash parses all of it before running
# anything, so under `curl | bash` no command can read the rest of the script
# from stdin, and a truncated download runs nothing.
{
set -euo pipefail

REPO='alepar/herdr-threads'
PLUGIN_ID='herdr-threads'
MARKER=.herdr-threads-install

version=${HERDR_THREADS_VERSION:-latest}
release_url=${HERDR_THREADS_RELEASE_URL:-https://github.com/$REPO/releases}
install_dir=${HERDR_THREADS_HOME:-$HOME/.local/share/herdr-threads}
bin_dir=${HERDR_THREADS_BIN_DIR:-$HOME/.local/bin}
herdr=${HERDR_BIN:-}
mode=install
setup=ask
register=1
force=0
yes=0

say() { printf '%s\n' "herdr-threads: $*"; }
warn() { printf '%s\n' "herdr-threads: warning: $*" >&2; }
die() { printf '%s\n' "herdr-threads: error: $*" >&2; exit 1; }

usage() {
    cat <<'EOF'
usage: install.sh [options]

  --version TAG      install this release (vX.Y.Z or X.Y.Z; default: latest)
  --setup            run `herdr-threads setup` (sets up every detected harness)
  --no-setup         skip harness setup (default when not on a terminal)
  --no-herdr         do not register the plugin with Herdr
  --prefix DIR       install directory (default ~/.local/share/herdr-threads)
  --bin-dir DIR      symlink directory (default ~/.local/bin)
  --release-url URL  release base URL (default https://github.com/alepar/herdr-threads/releases)
  --yes              uninstall: remove the harness hooks without asking
                     (needed when there is no terminal)
  --force            replace an unmanaged file at the symlink path; uninstall
                     even when Herdr cannot unlink the plugin
  --uninstall        remove the installation (keeps daemon state)
  -h, --help         show this help
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --version) [ $# -ge 2 ] || die "--version needs a value"; version=$2; shift 2 ;;
        --version=*) version=${1#*=}; shift ;;
        --setup) setup=yes; shift ;;
        --no-setup) setup=no; shift ;;
        --no-herdr) register=0; shift ;;
        --prefix) [ $# -ge 2 ] || die "--prefix needs a value"; install_dir=$2; shift 2 ;;
        --bin-dir) [ $# -ge 2 ] || die "--bin-dir needs a value"; bin_dir=$2; shift 2 ;;
        --release-url) [ $# -ge 2 ] || die "--release-url needs a value"; release_url=$2; shift 2 ;;
        --yes|-y) yes=1; shift ;;
        --force) force=1; shift ;;
        --uninstall) mode=uninstall; shift ;;
        -h|--help) usage; exit 0 ;;
        *) usage >&2; die "unknown argument: $1" ;;
    esac
done

case "$install_dir" in /*) ;; *) die "--prefix must be an absolute path: $install_dir" ;; esac
case "$bin_dir" in /*) ;; *) die "--bin-dir must be an absolute path: $bin_dir" ;; esac
install_dir=${install_dir%/}
bin_dir=${bin_dir%/}
link_path="$bin_dir/herdr-threads"
installed_binary="$install_dir/bin/herdr-threads"

if [ -z "$herdr" ]; then
    herdr=$(command -v herdr 2>/dev/null || true)
fi

# --- Herdr helpers ----------------------------------------------------------

# The plugin root Herdr has registered for herdr-threads ("" when none).
# `herdr plugin list` prints "- herdr-threads (Threads) enabled [local:/path]"
# (with "; N warning(s)" inside the brackets when the root's manifest is gone).
registered_root() {
    [ -n "$herdr" ] || return 0
    "$herdr" plugin list 2>/dev/null |
        sed -n "s/^- $PLUGIN_ID ([^)]*) [a-z]* \[\(.*\)\]\$/\1/p" | sed 's/; [0-9]* warning(s)$//' | head -n 1
}

# Value at a dotted path of the JSON on stdin (exit 1 when absent), read by
# the installed executable's hidden helper: JSON is never split with shell tools.
json_field() { "$installed_binary" internal json-field "$1"; }

# Invoke a plugin action and wait for it to finish. Needs a running Herdr
# server. Success is Herdr's exit status plus the documented field (the
# started action's log id), then that log entry's own status and exit code,
# all read with json_field. Returns the action's exit status (or 1 when the
# action could not be started or did not finish in time).
herdr_action() {
    local action=$1 invoked log_id deadline logs i found status code
    [ -x "$installed_binary" ] || return 1
    invoked=$("$herdr" plugin action invoke "$action" --plugin "$PLUGIN_ID" 2>&1) || return 1
    case $invoked in *'"log_id"'*) ;; *) return 1 ;; esac   # fixed-string match, no pipe (pipefail + early exit)
    log_id=$(printf '%s' "$invoked" | json_field result.log.log_id) || return 1
    [ -n "$log_id" ] || return 1
    deadline=$((SECONDS + 30))
    while [ "$SECONDS" -lt "$deadline" ]; do
        logs=$("$herdr" plugin log list --plugin "$PLUGIN_ID" --limit 50 2>/dev/null) || logs=''
        case $logs in
            *"\"log_id\":\"$log_id\""*)
            i=0
            while found=$(printf '%s' "$logs" | json_field "result.logs.$i.log_id"); do
                if [ "$found" = "$log_id" ]; then
                    status=$(printf '%s' "$logs" | json_field "result.logs.$i.status") || status=''
                    if [ -n "$status" ] && [ "$status" != running ]; then
                        code=$(printf '%s' "$logs" | json_field "result.logs.$i.exit_code") || code=1
                        [ "$code" = 0 ]
                        return
                    fi
                    break
                fi
                i=$((i + 1))
            done
            ;;
        esac
        sleep 0.3
    done
    return 1
}

server_running() {
    [ -n "$herdr" ] || return 1
    # Any request that needs the server; server_not_running exits nonzero.
    "$herdr" plugin log list --plugin "$PLUGIN_ID" --limit 1 >/dev/null 2>&1
}

detected_harnesses() {
    local h
    for h in claude codex; do
        if command -v "$h" >/dev/null 2>&1; then printf '%s\n' "$h"; fi
    done
}

# Asks a yes/no question on the controlling terminal (works under `curl | bash`).
confirm() {
    local answer
    [ -r /dev/tty ] && [ -w /dev/tty ] || return 1
    { printf '%s [y/N] ' "$1" > /dev/tty; } 2>/dev/null || return 1
    IFS= read -r answer < /dev/tty || return 1
    case "$answer" in y|Y|yes|YES|Yes) return 0 ;; *) return 1 ;; esac
}

# Setup is safe to run from an installer only when it is user-level. Builds
# whose setup is project-scoped (`--project`, default: the current directory)
# would write into whatever directory the installer happens to run from.
user_level_setup() {
    "$installed_binary" setup --help 2>/dev/null | grep -i 'user level' >/dev/null
}

# Builds whose bare `setup` / `unsetup` cover every detected harness (and
# print a per-harness summary) say so in `setup --help`; older ones need the
# harness named, one call each.
bare_setup() {
    "$installed_binary" setup --help 2>/dev/null | grep -i 'sets up every detected harness' >/dev/null
}

# The pid in an instance directory's endpoint.json, when that process is a
# live herdr-threads one.
instance_pid() {
    local pid
    pid=$(sed -n 's/.*"pid":\([0-9][0-9]*\).*/\1/p' "$1/endpoint.json" 2>/dev/null | head -n 1)
    [ -n "$pid" ] || return 1
    kill -0 "$pid" 2>/dev/null || return 1
    ps -p "$pid" -o command= 2>/dev/null | grep -qF herdr-threads || return 1
    printf '%s\n' "$pid"
}

# Instance directories of live herdr-threads daemons under Herdr's default
# plugin state roots, one per line ("$state/instances/<digest>"). Best effort:
# a daemon run with another --state-dir is not found.
live_instances() {
    local state instance
    for state in "${XDG_STATE_HOME:-$HOME/.local/state}/herdr/plugins/$PLUGIN_ID" \
        "$HOME/.local/state/herdr/plugins/$PLUGIN_ID"; do
        for instance in "$state"/instances/*; do
            [ -f "$instance/endpoint.json" ] || continue
            instance_pid "$instance" > /dev/null || continue
            printf '%s\n' "$instance"
        done
    done | sort -u
}

# PIDs of live herdr-threads daemons published under Herdr's default plugin
# state directory (the `pid` in each instance's endpoint.json), one per line.
daemon_pids() {
    local instance
    live_instances | while IFS= read -r instance; do
        instance_pid "$instance" || true
    done | sort -u
}

# Stop the running daemon with the still-installed executable: first with its
# own context resolution, then once per live instance with that instance's
# state dir and host endpoint (its `locator`) named explicitly, so a
# Herdr-less, HERDR_CONFIG_PATH environment still reaches it. Returns 0 only
# when a stop succeeded and no daemon found by the scan is left running; the
# last stop error line is left in $stop_err.
stop_with_installed() {
    local ok=0 err instance locator _
    stop_err=''
    if err=$("$installed_binary" daemon stop 2>&1 > /dev/null); then
        ok=1
    else
        stop_err=$(printf '%s\n' "$err" | tail -n 1)
    fi
    if [ "$ok" = 0 ] || [ -n "$(live_instances)" ]; then
        while IFS= read -r instance; do
            [ -n "$instance" ] || continue
            locator=$(cat "$instance/locator" 2>/dev/null) || continue
            [ -n "$locator" ] || continue
            if err=$("$installed_binary" --state-dir "${instance%/instances/*}" \
                --host-endpoint "$locator" daemon stop 2>&1 > /dev/null); then
                ok=1
            else
                stop_err=$(printf '%s\n' "$err" | tail -n 1)
            fi
        done < <(live_instances)
    fi
    # A stop may return just before the process is gone.
    for _ in 1 2 3 4 5; do
        [ -n "$(daemon_pids)" ] || break
        sleep 0.2
    done
    [ "$ok" = 1 ] && [ -z "$(daemon_pids)" ]
}

# Of the recorded old-daemon pids, the ones still alive (space-separated): a
# later Herdr stop or ensure may have stopped them.
old_daemon_alive() {
    local pid alive=''
    for pid in $OUT_OLD_DAEMON; do
        kill -0 "$pid" 2>/dev/null || continue
        ps -p "$pid" -o command= 2>/dev/null | grep -qF herdr-threads || continue
        alive="${alive:+$alive }$pid"
    done
    printf '%s' "$alive"
}

can_prompt() {
    # Under `curl | bash` stdin is the script, so ask on /dev/tty, but only
    # when output goes to a terminal too (a person is watching).
    [ -t 1 ] && { : < /dev/tty; } 2>/dev/null
}

# --- final status -----------------------------------------------------------
# Every outcome is recorded where it happens; nothing prints hints mid-run.
# next_steps() and finish() read the record, so the hints and the one final
# status line always agree with what actually happened.
OUT_LINKED=0          # Herdr has the plugin linked from $install_dir
OUT_NOT_LINKED=''     # why not (install), or why not unregistered (uninstall)
OUT_REGISTER_CMD=''   # the command that finishes registering / unregistering
OUT_UPGRADED=0
OUT_DAEMON_UP=0
OUT_SETUP=none        # none_found|skipped|per_project|not_registered|suggested|declined|complete|failed
OUT_SETUP_FAILED=''   # harnesses whose setup failed
OUT_CODEX_SET_UP=0
OUT_HOOKS_KEPT=0      # uninstall: hooks left in place (no --yes, no terminal)
OUT_OLD_DAEMON=''     # pids of a daemon the stop could not stop (re-checked at the end)

# --- uninstall --------------------------------------------------------------

# A stop that left a daemon running: warn with its pid and record it.
uninstall_stop_failed() {
    local pids
    pids=$(daemon_pids | tr '\n' ' ')
    pids=${pids% }
    [ -n "$pids" ] || return 0
    OUT_OLD_DAEMON=$pids
    warn "could not stop the running daemon (pid $pids, from its endpoint.json)${stop_err:+: $stop_err}"
    warn "stop it by hand: kill $pids"
}

uninstall() {
    local root herdr_ok=0 h still
    if [ -e "$install_dir" ] && [ ! -f "$install_dir/$MARKER" ]; then
        die "$install_dir was not created by this installer (no $MARKER); not removing it"
    fi
    # --no-herdr leaves Herdr alone, as it does for an install.
    if [ "$register" = 1 ] && [ -n "$herdr" ]; then herdr_ok=1; fi
    root=''
    if [ "$herdr_ok" = 1 ]; then root=$(registered_root); fi
    if [ -x "$installed_binary" ]; then
        if [ "$herdr_ok" = 1 ] && [ "$root" = "local:$install_dir" ] && server_running; then
            if herdr_action stop || stop_with_installed; then
                say "stopped the daemon"
            else
                uninstall_stop_failed
                [ -n "$OUT_OLD_DAEMON" ] || warn "could not stop the daemon (it may not be running)"
            fi
        elif stop_with_installed; then
            say "stopped the daemon"
        else
            uninstall_stop_failed
        fi
        if [ "$setup" = no ]; then
            :
        elif ! user_level_setup; then
            :
        elif [ "$yes" = 1 ] || { [ "$setup" != no ] && can_prompt &&
            confirm "Remove the herdr-threads hooks from the detected harnesses (herdr-threads unsetup)?"; }; then
            if bare_setup; then
                # Bare unsetup removes every recorded installation, whether or
                # not the harness is still on PATH.
                if "$installed_binary" unsetup; then
                    say "removed the harness hooks this tool owns"
                else
                    warn "\`herdr-threads unsetup\` failed; run it again by hand if a harness was set up"
                fi
            else
                for h in $(detected_harnesses); do
                    if "$installed_binary" unsetup "$h"; then
                        say "removed the $h hooks this tool owns"
                    else
                        warn "\`herdr-threads unsetup $h\` failed; run it again by hand if $h was set up"
                    fi
                done
            fi
        else
            OUT_HOOKS_KEPT=1
            say "kept the harness hooks (no terminal to ask on and no --yes)"
        fi
    fi
    if [ "$root" = "local:$install_dir" ]; then
        if "$herdr" plugin unlink "$PLUGIN_ID" >/dev/null 2>&1; then
            say "unlinked the plugin from Herdr"
        elif [ "$force" = 1 ]; then
            warn "Herdr could not unlink the plugin (is the server running?); removing files anyway (--force)"
            OUT_NOT_LINKED="Herdr could not unlink the plugin"
            OUT_REGISTER_CMD="herdr plugin unlink $PLUGIN_ID"
        else
            die "Herdr could not unlink the plugin (is the Herdr server running?). Start Herdr and re-run, or pass --force"
        fi
    elif [ -n "$root" ]; then
        say "Herdr has $PLUGIN_ID registered from $root, not from this installer; leaving it"
    elif [ "$herdr_ok" = 0 ]; then
        if [ "$register" = 0 ]; then
            OUT_NOT_LINKED="--no-herdr: Herdr was not asked to unregister the plugin"
        else
            OUT_NOT_LINKED="herdr is not on PATH, so the plugin registration was not checked"
        fi
        OUT_REGISTER_CMD="herdr plugin unlink $PLUGIN_ID"
    fi
    if [ -L "$link_path" ] && [ "$(readlink "$link_path")" = "$installed_binary" ]; then
        rm -f "$link_path"
        say "removed $link_path"
    fi
    if [ -d "$install_dir" ]; then
        rm -rf "$install_dir"
        say "removed $install_dir"
    fi
    rm -rf "$install_dir.new" "$install_dir.old"
    say "Daemon state in Herdr's plugin state directory was kept."
    if [ "$OUT_HOOKS_KEPT" = 1 ]; then
        say "harness hooks were left in place: re-run with --yes to remove them (they do nothing without herdr-threads)"
    fi
    still=$(old_daemon_alive)
    if [ -n "$still" ]; then
        printf '%s\n' "uninstalled; daemon still running: pid $still"
        printf '  stop it with: kill %s\n' "$still"
        exit 3
    fi
    if [ -n "$OUT_NOT_LINKED" ]; then
        printf '%s\n' "uninstalled, not unregistered: $OUT_NOT_LINKED"
        printf '  unregister it with: %s\n' "$OUT_REGISTER_CMD"
        exit 3
    fi
    printf '%s\n' "uninstalled"
    exit 0
}

if [ "$mode" = uninstall ]; then
    uninstall
fi

# --- platform ---------------------------------------------------------------

case "$(uname -s)" in
    Darwin) os=macos ;;
    Linux) os=linux ;;
    *) die "unsupported operating system: $(uname -s) (macOS and Linux only)" ;;
esac
case "$(uname -m)" in
    arm64|aarch64) arch=aarch64 ;;
    x86_64|amd64) arch=x86_64 ;;
    *) die "unsupported architecture: $(uname -m) (aarch64 and x86_64 only)" ;;
esac
if [ "$os" = linux ]; then
    warn "Linux support is EXPERIMENTAL and unvalidated: the plugin manifest declares macOS only,"
    warn "and the host incarnation witness is macOS-only, so Linux runs degraded (Unknown incarnation)."
fi
asset="herdr-threads-$os-$arch.tar.gz"

for tool in curl tar uname mktemp; do
    command -v "$tool" >/dev/null 2>&1 || die "$tool is required"
done
if command -v shasum >/dev/null 2>&1; then
    sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
elif command -v sha256sum >/dev/null 2>&1; then
    sha256() { sha256sum "$1" | cut -d' ' -f1; }
else
    die "shasum or sha256sum is required"
fi

# --- download and verify ----------------------------------------------------

release_url=${release_url%/}
case "$version" in
    latest) download="$release_url/latest/download" ;;
    v*) download="$release_url/download/$version" ;;
    *) version="v$version"; download="$release_url/download/$version" ;;
esac

work=$(mktemp -d "${TMPDIR:-/tmp}/herdr-threads-install.XXXXXX")
trap 'rm -rf "$work"' EXIT
say "downloading $asset ($version) from $download"
curl -fsSL --retry 3 -o "$work/$asset" "$download/$asset" || die "download failed: $download/$asset"
curl -fsSL --retry 3 -o "$work/SHA256SUMS" "$download/SHA256SUMS" || die "download failed: $download/SHA256SUMS"
expected=$(awk -v a="$asset" '$2 == a || $2 == "*" a { print $1 }' "$work/SHA256SUMS" | head -n 1)
[ -n "$expected" ] || die "SHA256SUMS has no entry for $asset"
actual=$(sha256 "$work/$asset")
[ "$actual" = "$expected" ] || die "checksum mismatch for $asset: expected $expected, got $actual"
say "checksum verified ($actual)"

mkdir "$work/extract"
tar -xzf "$work/$asset" -C "$work/extract"
package="$work/extract/herdr-threads"
for required in herdr-plugin.toml bin/herdr-threads scripts/view.sh scripts/build.sh VERSION PREBUILT; do
    [ -e "$package/$required" ] || die "release archive is missing $required"
done
[ -x "$package/bin/herdr-threads" ] || die "release archive's bin/herdr-threads is not executable"
new_version=$(cat "$package/VERSION")
printf 'version=%s\nsha256=%s\nasset=%s\n' "$new_version" "$actual" "$asset" > "$package/$MARKER"

# --- install files ----------------------------------------------------------

previous_version=''
changed=1
if [ -e "$install_dir" ]; then
    [ -f "$install_dir/$MARKER" ] ||
        die "$install_dir exists but was not created by this installer; move it away or pass another --prefix"
    previous_version=$(sed -n 's/^version=//p' "$install_dir/$MARKER")
    if grep -qx "sha256=$actual" "$install_dir/$MARKER" && [ -x "$installed_binary" ]; then
        changed=0
    fi
fi

# --- stop the old daemon (upgrade) -------------------------------------------
# Before the files are replaced, stop the running daemon with the executable
# that is still installed: a newer executable never takes over an older
# daemon, and across a wire-protocol change it cannot even stop it (the old
# daemon cannot decode its request). Herdr runs the `stop` action from the
# registered plugin root, which still holds the old package here.
old_stopped=0
stop_err=''
if [ "$changed" = 1 ] && [ -n "$previous_version" ] && [ -x "$installed_binary" ]; then
    if [ "$register" = 1 ] && [ "$(registered_root)" = "local:$install_dir" ] && server_running; then
        if herdr_action stop; then
            old_stopped=1
            say "stopped the running daemon with the installed $previous_version executable"
        fi
    fi
    if [ "$old_stopped" = 0 ] && stop_with_installed; then
        # Herdr is down, absent, not being used (--no-herdr), or its stop
        # action failed (an older installed executable may lack the helper
        # herdr_action reads its result with): stop the daemon directly with
        # the old executable, which speaks the running daemon's protocol.
        old_stopped=1
        say "stopped the old daemon (herdr-threads daemon stop) before replacing it"
    fi
    if [ "$old_stopped" = 0 ]; then
        old_pids=$(daemon_pids | tr '\n' ' ')
        old_pids=${old_pids% }
        if [ -n "$old_pids" ]; then
            OUT_OLD_DAEMON=$old_pids
            warn "could not stop the running daemon (pid $old_pids, from its endpoint.json) with the installed $previous_version executable${stop_err:+: $stop_err}"
            warn "if the new version cannot take it over (\`herdr-threads doctor\` reports a version or protocol mismatch),"
            warn "stop it by hand: kill $old_pids, then herdr plugin action invoke ensure --plugin $PLUGIN_ID"
        fi
    fi
fi

if [ "$changed" = 1 ]; then
    mkdir -p "$(dirname "$install_dir")"
    rm -rf "$install_dir.new" "$install_dir.old"
    cp -R "$package" "$install_dir.new"
    if [ -e "$install_dir" ]; then
        # Same path, so Herdr's registration (plugin_root) stays valid.
        mv "$install_dir" "$install_dir.old"
        if ! mv "$install_dir.new" "$install_dir"; then
            mv "$install_dir.old" "$install_dir" || true
            die "could not move the new package into $install_dir; the previous install was restored"
        fi
        rm -rf "$install_dir.old"
        OUT_UPGRADED=1
        say "replaced $install_dir ($previous_version -> $new_version)"
    else
        mv "$install_dir.new" "$install_dir"
        say "installed $new_version into $install_dir"
    fi
else
    say "$new_version is already installed in $install_dir"
fi

mkdir -p "$bin_dir"
if [ -L "$link_path" ]; then
    if [ "$(readlink "$link_path")" != "$installed_binary" ]; then
        warn "replacing symlink $link_path (was -> $(readlink "$link_path"))"
        ln -sfn "$installed_binary" "$link_path"
    fi
elif [ -e "$link_path" ]; then
    if [ "$force" = 1 ]; then
        warn "replacing $link_path (--force)"
        rm -f "$link_path"
        ln -s "$installed_binary" "$link_path"
    else
        die "$link_path exists and is not a symlink; move it away or pass --force"
    fi
else
    ln -s "$installed_binary" "$link_path"
    say "linked $link_path -> $installed_binary"
fi
on_path=0
case ":$PATH:" in *":$bin_dir:"*) on_path=1 ;; esac
"$installed_binary" --version >/dev/null || die "the installed executable does not run on this machine"

# --- Herdr registration -----------------------------------------------------

registered=0
herdr_cmd=${herdr:-herdr}
link_cmd="$herdr_cmd plugin link $install_dir"
if [ "$register" = 0 ]; then
    OUT_NOT_LINKED="--no-herdr"
    OUT_REGISTER_CMD=$link_cmd
    say "skipping Herdr registration (--no-herdr)"
elif [ -z "$herdr" ]; then
    OUT_NOT_LINKED="herdr is not on PATH"
    OUT_REGISTER_CMD=$link_cmd
else
    root=$(registered_root)
    if [ "$root" = "local:$install_dir" ]; then
        registered=1
        say "Herdr already has the plugin linked from $install_dir"
    elif [ -n "$root" ]; then
        OUT_NOT_LINKED="Herdr already has $PLUGIN_ID registered from $root; left alone"
        OUT_REGISTER_CMD="$herdr_cmd plugin unlink $PLUGIN_ID && $link_cmd"
    elif link_output=$("$herdr" plugin link "$install_dir" 2>&1); then
        registered=1
        say "linked the plugin into Herdr (herdr plugin link $install_dir)"
    else
        OUT_NOT_LINKED="Herdr refused the link: $link_output"
        OUT_REGISTER_CMD=$link_cmd
        if [ "$os" = linux ]; then
            warn "the manifest declares Linux, but Linux is unverified for this release; see Herdr's error above"
        fi
    fi
    if [ "$registered" = 1 ] && server_running; then
        if [ "$changed" = 1 ] && [ -n "$previous_version" ] && [ "$old_stopped" = 0 ]; then
            # The old executable did not stop the daemon (warned above if
            # one is still running). Within one wire protocol the new
            # executable can stop it; across a protocol change it cannot,
            # and ensure below then reports the mismatch.
            herdr_action stop || true
        fi
        if herdr_action ensure; then
            OUT_DAEMON_UP=1
            say "daemon is running (Herdr action ensure)"
        else
            warn "the ensure action failed; run \`herdr plugin action invoke doctor --plugin $PLUGIN_ID\`"
            old_pids=$(daemon_pids | tr '\n' ' ')
            old_pids=${old_pids% }
            if [ -n "$old_pids" ] && [ -n "$previous_version" ] && [ "$changed" = 1 ]; then
                warn "a daemon is still running (pid $old_pids, from its endpoint.json); if doctor reports a version or protocol mismatch,"
                warn "stop it by hand: kill $old_pids, then herdr plugin action invoke ensure --plugin $PLUGIN_ID"
            fi
        fi
    elif [ "$registered" = 1 ]; then
        say "Herdr server is not running: no daemon was started; it starts with the next Herdr server start"
    fi
fi
OUT_LINKED=$registered

# --- harness setup ----------------------------------------------------------

harnesses=$(detected_harnesses | tr '\n' ' ')
harnesses=${harnesses% }
if [ -z "$harnesses" ]; then
    OUT_SETUP=none_found
    say "no supported harness (claude, codex) found on PATH; skipping hook setup"
elif [ "$setup" = no ]; then
    OUT_SETUP=skipped
    say "skipping hook setup (--no-setup)"
elif [ "$registered" = 0 ]; then
    OUT_SETUP=not_registered
    say "skipping hook setup: the plugin is not registered with Herdr"
elif ! user_level_setup; then
    OUT_SETUP=per_project
    say "this build's setup is per project; the installer does not run it"
elif bare_setup; then
    if [ "$setup" = ask ] && ! can_prompt; then
        OUT_SETUP=suggested
        say "detected $harnesses; hook setup not run (no terminal and no --setup)"
    elif [ "$setup" = ask ] && ! confirm "Install herdr-threads hooks for every detected harness ($harnesses) (herdr-threads setup)?"; then
        OUT_SETUP=declined
        say "skipping hook setup"
    elif "$installed_binary" setup; then
        OUT_SETUP=complete
        case " $harnesses " in *" codex "*) OUT_CODEX_SET_UP=1 ;; esac
        say "set up hooks for the detected harnesses (summary above)"
    else
        OUT_SETUP=failed
        OUT_SETUP_FAILED=$harnesses
        warn "\`herdr-threads setup\` failed for a harness; see its output above"
    fi
else
    OUT_SETUP=suggested
    for h in $harnesses; do
        if [ "$setup" = ask ]; then
            if ! can_prompt; then
                say "detected $h; its hook setup not run (no terminal and no --setup)"
                continue
            fi
            if ! confirm "Install herdr-threads hooks for $h (herdr-threads setup $h)?"; then
                OUT_SETUP=declined
                continue
            fi
        fi
        if "$installed_binary" setup "$h"; then
            [ "$OUT_SETUP" = failed ] || OUT_SETUP=complete
            [ "$h" != codex ] || OUT_CODEX_SET_UP=1
            say "set up $h hooks"
        else
            OUT_SETUP=failed
            OUT_SETUP_FAILED="${OUT_SETUP_FAILED:+$OUT_SETUP_FAILED }$h"
            warn "\`herdr-threads setup $h\` failed; see its output above"
        fi
    done
fi

# --- next steps and final status -------------------------------------------

step() { printf '  - %s\n' "$1"; }

next_steps() {
    printf '\n%s\n' "herdr-threads $new_version ($os-$arch)"
    printf '  executable: %s -> %s\n' "$link_path" "$installed_binary"
    printf '  package:    %s\n' "$install_dir"
    printf '%s\n' "Next steps:"
    if [ "$on_path" = 0 ]; then
        step "$bin_dir is not on PATH; add it, e.g.: export PATH=\"$bin_dir:\$PATH\""
    fi
    if [ "$OUT_DAEMON_UP" = 1 ]; then
        step "Check it: herdr-threads doctor"
    elif [ "$OUT_LINKED" = 1 ]; then
        step "Start Herdr (or restart its server) so the plugin's startup entry starts the daemon,"
        printf '    %s\n' "then check it: herdr-threads doctor"
    else
        step "Register the plugin with Herdr: $OUT_REGISTER_CMD, then start Herdr"
        printf '    %s\n' "and check it: herdr-threads doctor"
    fi
    case "$OUT_SETUP" in
        failed) step "Fix agent hook setup (see above): herdr-threads setup; check: herdr-threads setup-status" ;;
        none_found) step "No supported harness (claude, codex) found on PATH; after installing one, run: herdr-threads setup" ;;
        not_registered) step "After the plugin is registered, set up agent hooks: herdr-threads setup (every detected harness; check: herdr-threads setup-status)" ;;
        per_project) step "Set up agent hooks inside each project: herdr-threads setup claude|codex" ;;
        complete) ;;
        *) step "Set up agent hooks: herdr-threads setup (every detected harness; check: herdr-threads setup-status)" ;;
    esac
    if [ "$OUT_CODEX_SET_UP" = 1 ]; then
        step "Trust the Codex hooks once: start codex interactively and trust the herdr-threads hooks it lists for review (or open /hooks)"
    fi
    step "Try it: in a Herdr shell pane run herdr-threads me init, then follow https://github.com/$REPO#try-it-yourself"
    step "Upgrade: re-run this installer. Remove: re-run it with --uninstall."
    if [ "$os" = linux ]; then
        printf '%s\n' "  - Linux is EXPERIMENTAL and unvalidated; please report problems."
    fi
}

# The one final status line, and the exit status that goes with it.
finish() {
    local verb='installed' still ensure
    [ "$OUT_UPGRADED" = 0 ] || verb='upgraded'
    still=$(old_daemon_alive)
    if [ -n "$still" ]; then
        if [ "$OUT_LINKED" = 1 ]; then
            printf '%s\n' "$verb and linked; old daemon still running: pid $still"
            ensure="herdr plugin action invoke ensure --plugin $PLUGIN_ID"
        else
            printf '%s\n' "$verb, not linked; old daemon still running: pid $still"
            ensure="herdr-threads daemon ensure"
        fi
        printf '  stop it with: kill %s, then %s\n' "$still" "$ensure"
        exit 3
    fi
    if [ "$OUT_LINKED" = 0 ]; then
        printf '%s\n' "$verb, not linked: $OUT_NOT_LINKED"
        printf '  register it with: %s\n' "$OUT_REGISTER_CMD"
        exit 3
    fi
    if [ "$OUT_SETUP" = failed ]; then
        printf '%s\n' "installed and linked; setup incomplete: $OUT_SETUP_FAILED"
        printf '  finish it with: herdr-threads setup\n'
        exit 3
    fi
    if [ "$OUT_UPGRADED" = 1 ]; then
        printf '%s\n' "upgraded: herdr-threads $previous_version -> $new_version (linked)"
    else
        printf '%s\n' "installed and linked: herdr-threads $new_version"
    fi
    exit 0
}

next_steps
finish
}
