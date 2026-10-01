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
#   3. installs the package into ~/.local/share/herdr-threads (replaced
#      atomically; an identical install is left alone) and links
#      ~/.local/bin/herdr-threads to its executable;
#   4. registers the package with Herdr (`herdr plugin link`; Herdr does not
#      build a linked plugin, and the package's build command keeps the
#      prebuilt binary), and on an upgrade restarts the daemon if Herdr runs;
#   5. optionally runs `herdr-threads setup` (every detected harness)
#      (claude, codex): asks on a terminal, or --setup / --no-setup;
#   6. prints next steps.
#
# --uninstall stops the daemon, removes the harness setup this tool owns,
# unlinks the plugin from Herdr and removes the files above. The daemon's
# state (threads, receipts) in Herdr's plugin state directory is kept.
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
# `herdr plugin list` prints "- herdr-threads (Threads) enabled [local:/path]".
registered_root() {
    [ -n "$herdr" ] || return 0
    "$herdr" plugin list 2>/dev/null |
        sed -n "s/^- $PLUGIN_ID ([^)]*) [a-z]* \[\(.*\)\]\$/\1/p" | head -n 1
}

# Invoke a plugin action and wait for it to finish. Needs a running Herdr
# server. Prints the action's stdout/stderr summary; returns its exit status
# (or 1 when the action could not be started or did not finish in time).
herdr_action() {
    local action=$1 invoked log_id entry deadline
    invoked=$("$herdr" plugin action invoke "$action" --plugin "$PLUGIN_ID" 2>&1) || return 1
    log_id=$(printf '%s' "$invoked" | sed -n 's/.*"log_id":"\([^"]*\)".*/\1/p' | head -n 1)
    [ -n "$log_id" ] || return 1
    deadline=$((SECONDS + 30))
    while [ "$SECONDS" -lt "$deadline" ]; do
        # Log entries contain no nested objects, so splitting on "{" puts each
        # entry on one line.
        entry=$("$herdr" plugin log list --plugin "$PLUGIN_ID" --limit 50 2>/dev/null |
            tr '{' '\n' | grep -F "\"log_id\":\"$log_id\"" | head -n 1 || true)
        if [ -n "$entry" ] && ! printf '%s' "$entry" | grep -qF '"status":"running"'; then
            local code
            code=$(printf '%s' "$entry" | sed -n 's/.*"exit_code":\([0-9-]*\).*/\1/p')
            [ "${code:-1}" = 0 ]
            return
        fi
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
    "$installed_binary" setup --help 2>/dev/null | grep -qi 'user level'
}

# Builds whose bare `setup` / `unsetup` cover every detected harness (and
# print a per-harness summary) say so in `setup --help`; older ones need the
# harness named, one call each.
bare_setup() {
    "$installed_binary" setup --help 2>/dev/null | grep -qi 'sets up every detected harness'
}

can_prompt() {
    # Under `curl | bash` stdin is the script, so ask on /dev/tty, but only
    # when output goes to a terminal too (a person is watching).
    [ -t 1 ] && { : < /dev/tty; } 2>/dev/null
}

# --- uninstall --------------------------------------------------------------

uninstall() {
    local root
    if [ -e "$install_dir" ] && [ ! -f "$install_dir/$MARKER" ]; then
        die "$install_dir was not created by this installer (no $MARKER); not removing it"
    fi
    root=$(registered_root)
    if [ -x "$installed_binary" ]; then
        if [ "$root" = "local:$install_dir" ] && server_running; then
            if herdr_action stop; then say "stopped the daemon"; else warn "could not stop the daemon (it may not be running)"; fi
        fi
        if [ "$setup" != no ] && user_level_setup && bare_setup; then
            # Bare unsetup removes every recorded installation, whether or
            # not the harness is still on PATH.
            if "$installed_binary" unsetup; then
                say "removed the harness hooks this tool owns"
            else
                warn "\`herdr-threads unsetup\` failed; run it again by hand if a harness was set up"
            fi
        elif [ "$setup" != no ] && user_level_setup; then
            local h
            for h in $(detected_harnesses); do
                if "$installed_binary" unsetup "$h"; then
                    say "removed the $h hooks this tool owns"
                else
                    warn "\`herdr-threads unsetup $h\` failed; run it again by hand if $h was set up"
                fi
            done
        fi
    fi
    if [ "$root" = "local:$install_dir" ]; then
        if "$herdr" plugin unlink "$PLUGIN_ID" >/dev/null 2>&1; then
            say "unlinked the plugin from Herdr"
        elif [ "$force" = 1 ]; then
            warn "Herdr could not unlink the plugin (is the server running?); removing files anyway (--force)"
            warn "run \`herdr plugin unlink $PLUGIN_ID\` once Herdr is running"
        else
            die "Herdr could not unlink the plugin (is the Herdr server running?). Start Herdr and re-run, or pass --force"
        fi
    elif [ -n "$root" ]; then
        say "Herdr has $PLUGIN_ID registered from $root, not from this installer; leaving it"
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
    say "uninstalled. Daemon state in Herdr's plugin state directory was kept."
}

if [ "$mode" = uninstall ]; then
    uninstall
    exit 0
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

if [ "$changed" = 1 ]; then
    mkdir -p "$(dirname "$install_dir")"
    rm -rf "$install_dir.new" "$install_dir.old"
    cp -R "$package" "$install_dir.new"
    if [ -e "$install_dir" ]; then
        # Same path, so Herdr's registration (plugin_root) stays valid.
        mv "$install_dir" "$install_dir.old"
        mv "$install_dir.new" "$install_dir"
        rm -rf "$install_dir.old"
        say "upgraded $install_dir ($previous_version -> $new_version)"
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
if [ "$on_path" = 0 ]; then
    warn "$bin_dir is not on PATH; add it, e.g.: export PATH=\"$bin_dir:\$PATH\""
fi
"$installed_binary" --version >/dev/null || die "the installed executable does not run on this machine"

# --- Herdr registration -----------------------------------------------------

registered=0
daemon_up=0
if [ "$register" = 0 ]; then
    say "skipping Herdr registration (--no-herdr)"
elif [ -z "$herdr" ]; then
    warn "herdr is not on PATH; register later with: herdr plugin link $install_dir"
else
    root=$(registered_root)
    if [ "$root" = "local:$install_dir" ]; then
        registered=1
        say "Herdr already has the plugin linked from $install_dir"
    elif [ -n "$root" ]; then
        warn "Herdr already has $PLUGIN_ID registered from $root; leaving that registration alone"
        warn "to use this install instead: herdr plugin uninstall/unlink $PLUGIN_ID, then herdr plugin link $install_dir"
    elif link_output=$("$herdr" plugin link "$install_dir" 2>&1); then
        registered=1
        say "linked the plugin into Herdr (herdr plugin link $install_dir)"
    else
        warn "herdr plugin link failed: $link_output"
        if [ "$os" = linux ]; then
            warn "the manifest declares platforms = [\"macos\"]; Herdr may refuse it on Linux"
        fi
    fi
    if [ "$registered" = 1 ] && server_running; then
        if [ "$changed" = 1 ] && [ -n "$previous_version" ]; then
            # A newer executable never takes over an older running daemon.
            herdr_action stop || true
        fi
        if herdr_action ensure; then
            daemon_up=1
            say "daemon is running (Herdr action ensure)"
        else
            warn "the ensure action failed; run \`herdr plugin action invoke doctor --plugin $PLUGIN_ID\`"
        fi
    elif [ "$registered" = 1 ]; then
        say "Herdr server is not running; the daemon starts with the next Herdr server start"
    fi
fi

# --- harness setup ----------------------------------------------------------

harnesses=$(detected_harnesses | tr '\n' ' ')
harnesses=${harnesses% }
# What setup did, for the next steps: setup_done when every harness setup
# that ran succeeded (and at least one ran); codex_set_up when Codex's hooks
# were installed and so still need their one-time trust review.
setup_done=0
setup_failed=0
codex_set_up=0
if [ -z "$harnesses" ]; then
    say "no supported harness (claude, codex) found on PATH; skipping hook setup"
elif [ "$setup" = no ]; then
    say "skipping hook setup (--no-setup)"
elif [ "$registered" = 0 ]; then
    say "skipping hook setup: the plugin is not registered with Herdr"
elif ! user_level_setup; then
    say "this build's setup is per project; set up hooks inside each project: herdr-threads setup claude|codex"
elif bare_setup; then
    if [ "$setup" = ask ] && ! can_prompt; then
        say "detected $harnesses; set up their hooks with: herdr-threads setup (or re-run with --setup)"
    elif [ "$setup" = ask ] && ! confirm "Install herdr-threads hooks for every detected harness ($harnesses) (herdr-threads setup)?"; then
        say "skipping hook setup; run it later with: herdr-threads setup"
    elif "$installed_binary" setup; then
        setup_done=1
        case " $harnesses " in *" codex "*) codex_set_up=1 ;; esac
        say "set up hooks for the detected harnesses (summary above)"
    else
        setup_failed=1
        warn "\`herdr-threads setup\` failed for a harness; see its output above"
    fi
else
    for h in $harnesses; do
        if [ "$setup" = ask ]; then
            if ! can_prompt; then
                say "detected $h; set up its hooks with: herdr-threads setup $h (or re-run with --setup)"
                continue
            fi
            confirm "Install herdr-threads hooks for $h (herdr-threads setup $h)?" || continue
        fi
        if "$installed_binary" setup "$h"; then
            setup_done=1
            [ "$h" != codex ] || codex_set_up=1
            say "set up $h hooks"
        else
            setup_failed=1
            warn "\`herdr-threads setup $h\` failed; see its output above"
        fi
    done
fi

# --- next steps -------------------------------------------------------------

cat <<EOF

herdr-threads $new_version ($os-$arch) is installed.
  executable: $link_path -> $installed_binary
  package:    $install_dir
Next steps:
EOF
step() { printf '  - %s\n' "$1"; }
if [ "$daemon_up" = 1 ]; then
    step "Check it: herdr-threads doctor"
elif [ "$registered" = 1 ]; then
    step "Start Herdr (or restart its server) so the plugin's startup entry starts the daemon,"
    printf '    %s\n' "then check it: herdr-threads doctor"
else
    step "Register the plugin with Herdr: herdr plugin link $install_dir, then start Herdr"
    printf '    %s\n' "and check it: herdr-threads doctor"
fi
if [ "$setup_failed" = 1 ]; then
    step "Fix agent hook setup (see above): herdr-threads setup; check: herdr-threads setup-status"
elif [ "$setup_done" = 0 ]; then
    step "Set up agent hooks: herdr-threads setup (every detected harness; check: herdr-threads setup-status)"
fi
if [ "$codex_set_up" = 1 ]; then
    step "Trust the Codex hooks once: start codex interactively and trust the herdr-threads hooks it lists for review (or open /hooks)"
fi
step "Try it: in a Herdr shell pane run herdr-threads me init, then follow https://github.com/$REPO#try-it-yourself"
step "Upgrade: re-run this installer. Remove: re-run it with --uninstall."
if [ "$os" = linux ]; then
    printf '%s\n' "  - Linux is EXPERIMENTAL and unvalidated; please report problems."
fi

exit 0
}
