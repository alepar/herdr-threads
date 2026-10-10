#!/usr/bin/env bash
# Offline end-to-end test of scripts/install.sh against locally built fake
# releases served from file:// URLs, with a scratch HOME.
#
# usage: tests/release/install_test.sh [BINARY]
#
# BINARY defaults to target/release/herdr-threads (build it first with
# `cargo build --release --locked`). Everything lives under one fresh
# /private/tmp (or $TMPDIR) directory: HOME, XDG directories, the Herdr config
# and socket. The real ~/.config/herdr, ~/.claude, ~/.codex and ~/.local are
# never read or written.
#
# The first part drives the installer against a recording Herdr stub. The
# matrix at the end (fresh install / upgrade / uninstall x Herdr up / down /
# absent from PATH / --no-herdr, a partial setup failure, an upgrade with an
# old daemon running and Herdr down) drives the real Herdr CLI against isolated
# named Herdr sessions (scripts/lib/isolated-herdr.sh); the shared Herdr
# server is never contacted. With no `herdr` on PATH the test fails naming the
# binary; with HT_SKIP_HERDR_TESTS=1 it lists every skipped matrix case by
# name and exits 0. Prints INSTALL_TEST_PASS on success.
set -euo pipefail

repo=$(CDPATH='' cd "$(dirname "$0")/../.." && pwd -P)
binary=${1:-$repo/target/release/herdr-threads}
[ -x "$binary" ] || { echo "build the release binary first: $binary" >&2; exit 2; }
binary=$(CDPATH='' cd "$(dirname "$binary")" && pwd -P)/$(basename "$binary")
# shellcheck source=../../scripts/lib/isolated-herdr.sh
# The library reports its policy through exit statuses, so source it outside errexit.
set +e
. "$repo/scripts/lib/isolated-herdr.sh"
lib_status=$?
set -e
[ "$lib_status" = 0 ] || exit 1

base=/private/tmp
[ -d "$base" ] || base=${TMPDIR:-/tmp}
root=$(mktemp -d "$base/htit.XXXXXX")
ih_roots=''
cleanup() {
    local r
    # Stop every daemon and private Herdr server this test started.
    for r in $ih_roots; do
        pkill -f "daemon run --state-dir $r/" 2>/dev/null || true
        ih_teardown "$r" >/dev/null 2>&1 || true
    done
    pkill -f "daemon run --state-dir $root/" 2>/dev/null || true
    if [ "${KEEP:-0}" = 1 ]; then echo "kept $root"; else rm -rf "$root"; fi
}
trap cleanup EXIT
# Fatal signals exit through the EXIT trap, so cleanup runs for them too.
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
# SIGKILL runs no trap: a detached watchdog stops every isolated Herdr server
# (listed in $root/ih-roots as the matrix creates them) and every daemon from
# this test's roots once this script is gone.
start_watchdog() {
    local owner=$$
    (
        trap '' HUP INT TERM
        while kill -0 "$owner" 2>/dev/null; do sleep 1; done
        if [ -f "$root/ih-roots" ]; then
            while read -r r; do
                [ -n "$r" ] || continue
                if [ -f "$r/server.pid" ]; then kill -9 "$(cat "$r/server.pid")" 2>/dev/null || true; fi
                pkill -9 -f "daemon run --state-dir $r/" 2>/dev/null || true
            done < "$root/ih-roots"
        fi
        pkill -9 -f "daemon run --state-dir $root/" 2>/dev/null || true
    ) < /dev/null > /dev/null 2>&1 &
}

checks=0
pass() { checks=$((checks + 1)); printf 'ok %d - %s\n' "$checks" "$1"; }
fail() { printf 'not ok - %s\n' "$1" >&2; [ -z "${2:-}" ] || printf '%s\n' "$2" >&2; exit 1; }
expect() { local name=$1; shift; if "$@"; then pass "$name"; else fail "$name"; fi; }

case "$(uname -s)" in Darwin) os=macos ;; Linux) os=linux ;; *) fail "unsupported OS" ;; esac
case "$(uname -m)" in arm64|aarch64) arch=aarch64 ;; x86_64|amd64) arch=x86_64 ;; *) fail "unsupported arch" ;; esac
asset="herdr-threads-$os-$arch.tar.gz"

sha256() { if command -v shasum >/dev/null; then shasum -a 256 "$1"; else sha256sum "$1"; fi | cut -d' ' -f1; }

# --- fake releases -----------------------------------------------------------
# rel/download/vX/{asset,SHA256SUMS}; rel/latest -> newest, like GitHub.
make_release() { # make_release RELEASE_DIR TAG BINARY
    local dir="$1/download/$2"
    mkdir -p "$dir"
    TMPDIR="$root" "$repo/scripts/package-release.sh" --binary "$3" --os "$os" --arch "$arch" \
        --version "${2#v}" --out "$dir" --third-party "$root/THIRD_PARTY_LICENSES.html" >/dev/null
    # Every platform's line, like the real SHA256SUMS.
    (cd "$dir" && printf '%s  %s\n' "$(sha256 "$asset")" "$asset" > SHA256SUMS &&
        printf '%s  %s\n' 0000000000000000000000000000000000000000000000000000000000000000 \
            herdr-threads-other-platform.tar.gz >> SHA256SUMS)
}
rel="$root/rel"
printf '%s\n' '<html>stub</html>' > "$root/THIRD_PARTY_LICENSES.html"
make_release "$rel" v0.1.0 "$binary"
make_release "$rel" v0.1.1 "$binary"
mkdir -p "$rel/latest"
ln -s ../download/v0.1.1 "$rel/latest/download"

# A user-level-setup stand-in binary that records its calls, for the harness
# setup path (this build's real setup may still be per project).
stub_bin="$root/stub-herdr-threads"
cat > "$stub_bin" <<'EOF'
#!/bin/sh
case "$1" in
    --version) echo "herdr-threads 0.2.0" ;;
    # This stand-in models the older user-level setup interface, without the
    # modern reconciliation capability. Keep its bare-setup fallback covered.
    internal)
        if [ "${2:-}" = installer-integrations ]; then exit 2; fi
        exec "$REAL_BIN" "$@" ;;
    setup|unsetup)
        if [ "${2:-}" = --help ]; then
            echo "No harness named: every harness. \`setup\` sets up every detected harness"
            echo "Scope (user level, like Herdr's own agent hooks):"
            # Help may arrive in multiple writes. An early-exiting grep must
            # not SIGPIPE the producer and misclassify setup under pipefail.
            sleep 0.02
            echo "Usage: herdr-threads setup [HARNESS]"
            exit 0
        fi
        printf '%s\n' "$*" >> "$STUB_LOG"
        if [ -e "$STUB_LOG.fail" ]; then exit 1; fi ;;
esac
EOF
chmod 755 "$stub_bin"
stub_rel="$root/stub-rel"
make_release "$stub_rel" v0.2.0 "$stub_bin"
mkdir -p "$stub_rel/latest"
ln -s ../download/v0.2.0 "$stub_rel/latest/download"

# --- scratch environment ----------------------------------------------------
home="$root/home"
mkdir -p "$home" "$root/cfg" "$root/st" "$root/rt" "$root/tools" "$root/cwd"
: > "$root/cfg/herdr.toml"
tools="$root/tools"
cat > "$tools/herdr" <<'EOF'
#!/bin/bash
# Recording Herdr stub: registry in $FAKE_HERDR_DIR/registry, server up when
# $FAKE_HERDR_DIR/server exists.
d=$FAKE_HERDR_DIR; reg=$d/registry; echo "$*" >> "$d/calls"
up() { [ -e "$d/server" ] || { echo '{"error":{"code":"server_not_running"}}'; exit 1; }; }
case "$1 $2" in
"plugin list")
    if [ "${3:-}" = --json ]; then
        if [ -s "$reg" ]; then
            echo '{"result":{"plugins":[{"plugin_id":"herdr-threads","enabled":true}]}}'
        else
            echo '{"result":{"plugins":[]}}'
        fi
    elif [ -s "$reg" ]; then
        echo "1 plugin installed:"; echo "- herdr-threads (Threads) enabled [local:$(cat "$reg")]"
    else
        echo "No plugins installed."
    fi ;;
"plugin link") [ -f "$3/herdr-plugin.toml" ] || exit 1; printf '%s' "$3" > "$reg"; echo '{"result":{"type":"plugin_linked"}}' ;;
"plugin unlink") up; : > "$reg"; echo '{"result":{"removed":true}}' ;;
"plugin log") up; n=$(cat "$d/n" 2>/dev/null || echo 0); code=$(cat "$d/code-$n" 2>/dev/null || echo 0); echo "{\"result\":{\"logs\":[{\"exit_code\":$code,\"log_id\":\"plugin-log-$n\",\"status\":\"succeeded\"}]}}" ;;
"plugin action") up; n=$(( $(cat "$d/n" 2>/dev/null || echo 0) + 1 )); echo $n > "$d/n"; echo "action $4" >> "$d/actions"
    # Which package version the action ran from (the registered root's VERSION).
    echo "$4 $(cat "$(cat "$reg")/VERSION" 2>/dev/null)" >> "$d/action-versions"
    # $FAKE_HERDR_DIR/stop.fail makes every stop action exit 1.
    if [ "$4" = stop ] && [ -e "$d/stop.fail" ]; then echo 1; else echo 0; fi > "$d/code-$n"
    echo "{\"result\":{\"log\":{\"log_id\":\"plugin-log-$n\",\"status\":\"running\"}}}" ;;
*) exit 2 ;;
esac
EOF
chmod 755 "$tools/herdr"
# Harness stand-ins so the installer detects claude.
printf '#!/bin/sh\necho "2.1.285 (Claude Code)"\n' > "$tools/claude"
chmod 755 "$tools/claude"

run_env=(env -i "HOME=$home" "PATH=$tools:/usr/bin:/bin:/usr/sbin:/sbin"
    "XDG_CONFIG_HOME=$root/cfg" "XDG_STATE_HOME=$root/st" "XDG_RUNTIME_DIR=$root/rt"
    "HERDR_CONFIG_PATH=$root/cfg/herdr.toml" "HERDR_SOCKET_PATH=$root/h.sock"
    "FAKE_HERDR_DIR=$root" "STUB_LOG=$root/stub.log" "REAL_BIN=$binary" "TMPDIR=$root")
prefix="$home/.local/share/herdr-threads"
link="$home/.local/bin/herdr-threads"
out="$root/out"

extra_env=()
# install [ARGS...]: runs the installer (no terminal) from a scratch cwd;
# output in $out, status in $status.
install() {
    set +e
    (cd "$root/cwd" && "${run_env[@]}" ${extra_env[@]+"${extra_env[@]}"} bash "$repo/scripts/install.sh" \
        --release-url "file://$rel" "$@" < /dev/null > "$out" 2>&1)
    status=$?
    set -e
}
has() { grep -qF -- "$1" "$out"; }
registered() { "${run_env[@]}" "$tools/herdr" plugin list 2>/dev/null | grep -F "[local:$prefix]" >/dev/null; }
not_registered() { ! registered; }
daemons() { pgrep -f "daemon run --state-dir $root/" 2>/dev/null | wc -l | tr -d ' '; }

last_line() { tail -n 1 "$out"; }
start_watchdog

# --- 1. pinned install ------------------------------------------------------
install --version v0.1.0
[ "$status" = 0 ] || fail "pinned install exits 0" "$(cat "$out")"
pass "pinned install exits 0"
expect "final status line: installed and linked" [ "$(last_line)" = "installed and linked: herdr-threads 0.1.0" ]
expect "piped installer output has no ANSI color" bash -c "! grep -q $'\\033' '$out'"
# Run a command on a pseudo-terminal, recording its output in LOG (BSD and util-linux script differ).
pty_run() {
    local log=$1
    shift
    if [ "$(uname -s)" = Darwin ]; then
        script -q "$log" "$@"
    else
        script -q -e -c "$(printf '%q ' "$@")" "$log"
    fi
}
if command -v script > /dev/null || fail "script(1) is needed for the terminal color checks"; then
    (cd "$root/cwd" && pty_run "$root/color.log" "${run_env[@]}" TERM=xterm \
        bash "$repo/scripts/install.sh" --release-url "file://$rel" --version v0.1.0 --no-setup < /dev/null > /dev/null 2>&1)
    expect "terminal status is colored" grep -q $'\033\[1;32m' "$root/color.log"
    (cd "$root/cwd" && pty_run "$root/no-color.log" "${run_env[@]}" TERM=xterm NO_COLOR=1 \
        bash "$repo/scripts/install.sh" --release-url "file://$rel" --version v0.1.0 --no-setup < /dev/null > /dev/null 2>&1)
    expect "NO_COLOR suppresses terminal ANSI color" bash -c "! grep -q $'\\033' '$root/no-color.log'"
    (cd "$root/cwd" && pty_run "$root/dumb-color.log" "${run_env[@]}" TERM=dumb \
        bash "$repo/scripts/install.sh" --release-url "file://$rel" --version v0.1.0 --no-setup < /dev/null > /dev/null 2>&1)
    expect "TERM=dumb suppresses terminal ANSI color" bash -c "! grep -q $'\\033' '$root/dumb-color.log'"
fi
expect "exactly one final status line" [ "$(grep -c '^installed and linked' "$out")" = 1 ]
expect "checksum verified" has "checksum verified"
expect "package installed with VERSION 0.1.0" [ "$(cat "$prefix/VERSION")" = 0.1.0 ]
expect "install marker records the version" grep -qx "version=0.1.0" "$prefix/.herdr-threads-install"
expect "executable symlinked" [ "$(readlink "$link")" = "$prefix/bin/herdr-threads" ]
expect "symlink runs" sh -c "'$link' --version > /dev/null"
expect "PATH note for ~/.local/bin is a next step" has "is not on PATH"
expect "plugin linked from the install dir" registered
expect "package build command keeps the prebuilt binary" \
    sh -c "'$prefix/scripts/build.sh' | grep -qF 'prebuilt release package'"
# The installer runs setup only for user-level builds, which it decides from
# `setup --help`; check whichever branch this binary takes, and that neither
# writes hooks without a terminal or --setup.
if "$binary" setup --help 2>/dev/null | grep -qi 'user level'; then
    user_level=1
    if "$binary" internal installer-integrations --help >/dev/null 2>&1; then
        modern_integrations=1
        expect "modern integrations: missing hooks skipped without terminal" has "claude hooks: skipped"
        expect "modern integrations: missing skill skipped without terminal" has "claude skill: skipped"
        expect "modern integrations: explicit consent remedy" has "rerun install.sh --setup"
    else
        modern_integrations=0
        expect "user-level setup: no terminal, so setup not run" \
            has "detected claude; hook setup not run (no terminal and no --setup)"
    fi
    expect "user-level setup: not run without a terminal" bash -c "! grep -qF 'set up hooks for the detected harnesses' '$out'"
    expect "user-level setup: nothing written to the scratch ~/.claude" [ ! -e "$home/.claude" ]
else
    user_level=0
    modern_integrations=0
    expect "per-project setup is not run from the installer" has "this build's setup is per project"
fi
expect "no project settings written in the cwd" [ ! -e "$root/cwd/.claude" ]
expect "next steps printed" has "Next steps:"
if [ "$modern_integrations" = 1 ]; then
    expect "modern integrations: no misleading legacy setup hint" \
        bash -c "! grep -qF 'Set up agent hooks:' '$out'"
else
    expect "next steps: hooks not set up, so setup suggested" has "Set up agent hooks: herdr-threads setup"
    expect "no hints mid-run: the setup command appears only in the next steps" \
        bash -c "[ \"\$(grep -c 'herdr-threads setup' '$out')\" = 1 ]"
fi
expect "next steps: no Codex trust reminder without Codex setup" bash -c "! grep -qF 'Trust the Codex hooks' '$out'"
expect "next steps: try-it points at me init" has "herdr-threads me init"
expect "next steps: no stale agent-guide URL" bash -c "! grep -qF 'docs/agent-usage.md' '$out'"
expect "stub server down: no actions invoked" [ ! -e "$root/actions" ]
expect "stub server down: the message does not claim a daemon is running" has "no daemon was started"
expect "next steps: server down, so start Herdr" has "Start Herdr (or restart its server)"
expect "next steps: doctor after starting Herdr" has "then check it: herdr-threads doctor"
touch "$prefix/.sentinel"   # survives only if the re-run leaves the dir alone

# --- 2. idempotent re-run ---------------------------------------------------
install --version 0.1.0
[ "$status" = 0 ] || fail "re-run exits 0" "$(cat "$out")"
pass "re-run exits 0"
expect "re-run reports already installed" has "0.1.0 is already installed"
expect "re-run leaves the install dir in place" [ -e "$prefix/.sentinel" ]
expect "re-run keeps the registration" has "already has the plugin linked"
expect "re-run does not relink the symlink" bash -c "! grep -qF 'linked $link' '$out'"
expect "re-run final status" [ "$(last_line)" = "installed and linked: herdr-threads 0.1.0" ]

# --- 3. upgrade to latest ---------------------------------------------------
touch "$root/server"   # stub server up from here on
install
[ "$status" = 0 ] || fail "upgrade exits 0" "$(cat "$out")"
pass "upgrade exits 0"
expect "upgrade final status" [ "$(last_line)" = "upgraded: herdr-threads 0.1.0 -> 0.1.1 (linked)" ]
expect "latest is 0.1.1" [ "$(cat "$prefix/VERSION")" = 0.1.1 ]
expect "no staging dir left" [ ! -e "$prefix.new" ]
expect "no backup dir left" [ ! -e "$prefix.old" ]
expect "registration survives the upgrade" registered
expect "upgrade stops then ensures the daemon" \
    [ "$(tr '\n' ' ' < "$root/actions")" = "action stop action ensure " ]
expect "upgrade stops with the old package, before the swap, and ensures with the new" \
    [ "$(tr '\n' ' ' < "$root/action-versions")" = "stop 0.1.0 ensure 0.1.1 " ]
expect "upgrade reports the old executable stopped the daemon" \
    has "stopped the running daemon with the installed 0.1.0 executable"
expect "next steps: daemon running, so check with doctor" has "Check it: herdr-threads doctor"
expect "next steps: daemon running, no start-Herdr step" bash -c "! grep -qF 'Start Herdr' '$out'"

# --- 3b. the old executable cannot stop the daemon ----------------------------
# A failed old-binary stop must be visible: warn with the running daemon's pid
# (from its endpoint.json) and the manual kill. Without a live daemon, say
# nothing. Stub Herdr only (the failure is injected).
{
    : > "$root/stop.fail"
    (exec -a "herdr-threads daemon run --state-dir $root/fake" sleep 300) &
    fake_pid=$!
    fake_instance="$root/st/herdr/plugins/herdr-threads/instances/fake"
    mkdir -p "$fake_instance"
    printf '{"software_version":"0.1.1","protocol_version":1,"pid":%s,"socket_inode":1}\n' \
        "$fake_pid" > "$fake_instance/endpoint.json"
    printf '%s' "$root/h.sock" > "$fake_instance/locator"
    : > "$root/actions"
    : > "$root/action-versions"
    install --version v0.1.0
    [ "$status" = 3 ] || fail "upgrade with a failing stop and a live daemon exits 3" "$(cat "$out")"
    pass "upgrade with a failing stop and a live daemon exits 3"
    expect "failed old-binary stop: final status names the pid" \
        has "upgraded and linked; old daemon still running: pid $fake_pid"
    expect "failed old-binary stop: last line is the remedy" \
        [ "$(last_line)" = "  stop it with: kill $fake_pid, then herdr plugin action invoke ensure --plugin herdr-threads" ]
    expect "failed old-binary stop warns with the daemon pid" \
        has "could not stop the running daemon (pid $fake_pid, from its endpoint.json) with the installed 0.1.1 executable"
    expect "failed old-binary stop names the manual kill" has "stop it by hand: kill $fake_pid"
    expect "failed old-binary stop: no success claimed" bash -c "! grep -qF 'stopped the running daemon' '$out'"
    expect "failed old-binary stop: retried with the new package, then ensure" \
        [ "$(tr '\n' ' ' < "$root/action-versions")" = "stop 0.1.1 stop 0.1.0 ensure 0.1.0 " ]
    kill "$fake_pid" 2>/dev/null || true
    wait "$fake_pid" 2>/dev/null || true
    install
    [ "$status" = 0 ] || fail "upgrade with a failing stop and no daemon exits 0" "$(cat "$out")"
    expect "failing stop without a live daemon: no warning" bash -c "! grep -qF 'could not stop' '$out'"
    expect "back on 0.1.1" [ "$(cat "$prefix/VERSION")" = 0.1.1 ]
    rm -rf "$root/stop.fail" "$fake_instance"
}

# --- 4. refusals --------------------------------------------------------------
cp -R "$rel" "$root/bad-rel"
printf '%s  %s\n' "$(printf '%064d' 1)" "$asset" > "$root/bad-rel/download/v0.1.0/SHA256SUMS"
install --version v0.1.0 --release-url "file://$root/bad-rel"
expect "checksum mismatch refused" [ "$status" = 1 ]
expect "checksum mismatch named" has "checksum mismatch"
expect "failed install leaves the previous version" [ "$(cat "$prefix/VERSION")" = 0.1.1 ]

install --version v9.9.9
expect "missing release refused" [ "$status" = 1 ]
expect "missing release named" has "download failed"

install --bogus-flag
expect "bad arguments exit 1" [ "$status" = 1 ]
expect "bad arguments named" has "unknown argument: --bogus-flag"

mkdir -p "$root/foreign"
install --prefix "$root/foreign" --no-herdr
expect "unmanaged prefix refused" [ "$status" = 1 ]
expect "unmanaged prefix named" has "was not created by this installer"

mkdir -p "$root/bin2" && echo keep > "$root/bin2/herdr-threads"
install --bin-dir "$root/bin2" --no-herdr
expect "regular file at the symlink path refused" [ "$status" = 1 ]
expect "regular file at the symlink path named" has "is not a symlink"
expect "regular file kept" [ "$(cat "$root/bin2/herdr-threads")" = keep ]

# --- 5. uninstall -------------------------------------------------------------
install --uninstall --yes
[ "$status" = 0 ] || fail "uninstall exits 0" "$(cat "$out")"
pass "uninstall exits 0"
expect "uninstall final status" [ "$(last_line)" = "uninstalled" ]
expect "install dir removed" [ ! -e "$prefix" ]
expect "symlink removed" [ ! -L "$link" ]
expect "plugin unlinked" not_registered
expect "state kept message" has "Daemon state in Herdr's plugin state directory was kept"
if [ "$user_level" = 1 ]; then
    expect "user-level build: uninstall runs bare unsetup" \
        bash -c "grep -qF 'removed the harness hooks this tool owns' '$out' || grep -qF 'herdr-threads unsetup\` failed' '$out'"
else
    expect "per-project build: uninstall does not run unsetup" bash -c "! grep -qF 'unsetup' '$out'"
fi
expect "uninstall stopped the daemon" [ "$(tail -n 1 "$root/actions")" = "action stop" ]
install --uninstall
expect "second uninstall is a no-op success" [ "$status" = 0 ]

# Uninstall while Herdr cannot unlink refuses unless --force; --force finishes
# the removal but says the plugin is still registered.
install --version v0.1.0
rm -f "$root/server"
install --uninstall
expect "uninstall refuses when Herdr cannot unlink" [ "$status" = 1 ]
expect "refused uninstall keeps the files" [ -d "$prefix" ]
install --uninstall --force
expect "uninstall --force exits 3 (not unregistered)" [ "$status" = 3 ]
expect "uninstall --force names the unregister command" has "unregister it with: herdr plugin unlink herdr-threads"
expect "uninstall --force removes the files" [ ! -e "$prefix" ]
touch "$root/server"
"${run_env[@]}" "$tools/herdr" plugin unlink herdr-threads > /dev/null

# --- 6. harness setup with a user-level setup build ---------------------------
install --release-url "file://$stub_rel" --setup
[ "$status" = 0 ] || fail "--setup install exits 0" "$(cat "$out")"
expect "--setup runs bare setup (every detected harness)" grep -qx "setup" "$root/stub.log"
expect "no per-harness setup call" bash -c "! grep -q 'setup c' '$root/stub.log'"
expect "--setup next steps: no setup suggestion after setup" bash -c "! grep -qF 'Set up agent hooks' '$out'"
expect "--setup next steps: no start-Herdr step with a running daemon" bash -c "! grep -qF 'Start Herdr' '$out'"
expect "--setup next steps: doctor" has "Check it: herdr-threads doctor"
expect "--setup next steps: me init" has "herdr-threads me init"
expect "--setup next steps: no Codex trust reminder (claude only)" bash -c "! grep -qF 'Trust the Codex hooks' '$out'"
install --release-url "file://$stub_rel"
expect "no terminal and no --setup: setup suggested in the next steps" has "Set up agent hooks: herdr-threads setup"
expect "no second setup call" [ "$(grep -c '^setup' "$root/stub.log")" = 1 ]
install --uninstall
[ "$status" = 0 ] || fail "uninstall without --yes exits 0" "$(cat "$out")"
expect "non-interactive uninstall without --yes keeps the hooks" has "kept the harness hooks"
expect "non-interactive uninstall without --yes does not call unsetup" bash -c "! grep -q '^unsetup' '$root/stub.log'"
install --release-url "file://$stub_rel"
install --uninstall --yes
[ "$status" = 0 ] || fail "uninstall after setup exits 0" "$(cat "$out")"
expect "uninstall --yes removes the owned hooks with bare unsetup" grep -qx "unsetup" "$root/stub.log"
expect "uninstall leaves no install dir" [ ! -e "$prefix" ]
expect "uninstall leaves no symlink" [ ! -L "$link" ]

# --- 7. Codex detected: --setup reminds about the one-time hook trust ---------
printf '#!/bin/sh\necho "codex-cli 0.159.2"\n' > "$tools/codex"
chmod 755 "$tools/codex"
install --release-url "file://$stub_rel" --setup
[ "$status" = 0 ] || fail "--setup install with codex exits 0" "$(cat "$out")"
expect "codex --setup next steps: Codex trust reminder" has "Trust the Codex hooks once"
expect "codex --setup trust reminder acknowledges unverified status" has "installer cannot verify current hook trust"
expect "codex --setup trust reminder precedes routine steps" \
    awk '/Trust the Codex hooks once/{trust=NR} /Check it:/{check=NR} END{exit !(trust && (!check || trust<check))}' "$out"
expect "codex --setup next steps: no setup suggestion" bash -c "! grep -qF 'Set up agent hooks' '$out'"
install --release-url "file://$stub_rel"
expect "codex without --setup: no Codex trust reminder" bash -c "! grep -qF 'Trust the Codex hooks' '$out'"
expect "codex without --setup: setup suggested" has "Set up agent hooks: herdr-threads setup"
: > "$root/stub.log.fail"
install --release-url "file://$stub_rel" --setup
expect "failed setup exits 3" [ "$status" = 3 ]
expect "failed setup: status line names the harnesses" has "installed and linked; setup incomplete: claude codex"
expect "failed setup: the setup command is named" has "finish it with: herdr-threads setup"
expect "failed setup: next steps say to fix setup" has "Fix agent hook setup (see above)"
expect "failed setup: no Codex trust reminder" bash -c "! grep -qF 'Trust the Codex hooks' '$out'"
rm -f "$root/stub.log.fail" "$tools/codex"
install --uninstall --yes
[ "$status" = 0 ] || fail "uninstall after codex setup exits 0" "$(cat "$out")"

# --- 8. no harness found: the setup reminder says so --------------------------
rm -f "$tools/claude"
install --release-url "file://$stub_rel"
[ "$status" = 0 ] || fail "install with no harness exits 0" "$(cat "$out")"
expect "no harness: next steps say none was found" has "No supported harness (claude, codex) found on PATH; after installing one, run: herdr-threads setup"
expect "no harness: no bare 'Set up agent hooks' reminder" bash -c "! grep -qF 'Set up agent hooks' '$out'"
install --uninstall --yes
printf '#!/bin/sh\necho "2.1.285 (Claude Code)"\n' > "$tools/claude"
chmod 755 "$tools/claude"

# --- 9. Herdr refuses the link ------------------------------------------------
printf '%s\n' '#!/bin/bash' 'if [ "$1 $2" = "plugin link" ]; then echo "{\"error\":{\"message\":\"manifest rejected\"}}"; exit 1; fi' \
    "exec '$tools/herdr' \"\$@\"" > "$root/refusing-herdr"
chmod 755 "$root/refusing-herdr"
extra_env=("HERDR_BIN=$root/refusing-herdr")
install --version v0.1.0
extra_env=()
expect "refused link exits 3" [ "$status" = 3 ]
expect "refused link: status line names the reason" has "installed, not linked: Herdr refused the link:"
expect "refused link: the register command is printed" has "register it with: $root/refusing-herdr plugin link $prefix"
expect "refused link: files are still installed" [ -x "$prefix/bin/herdr-threads" ]
expect "refused link: next steps say to register" has "Register the plugin with Herdr: $root/refusing-herdr plugin link $prefix"
expect "refused link: setup skipped because not registered" has "skipping hook setup: the plugin is not registered with Herdr"
install --uninstall --yes

# --- 10. matrix against isolated named Herdr sessions ---------------------------
# Each case gets its own private root (scripts/lib/isolated-herdr.sh: HOME, XDG
# dirs, Herdr config and API socket) with a real Herdr server it starts and
# stops itself; the shared Herdr server is never contacted. Without `herdr`
# every case here is listed as skipped (HT_SKIP_HERDR_TESTS=1) or the test has
# already failed naming the binary (sourcing the library).
skipped_cases=''
mx_root=''; mx_case=''; mx_prefix=''; mx_path=''; mx_relurl=''
mx_begin() { # CASE -> 0 to run it, 1 when skipped (no herdr)
    local rc=0
    ih_require_herdr "$1" || rc=$?
    if [ "$rc" = 2 ]; then
        echo "skipped: no herdr: $1"
        skipped_cases="$skipped_cases $1"
        return 1
    fi
    [ "$rc" = 0 ] || fail "$1: isolated Herdr unavailable"
    mx_case=$1
    mx_root=$(ih_root_new)
    mx_root=$(cd "$mx_root" && pwd -P)   # Herdr reports canonical paths
    ih_roots="$ih_roots $mx_root"
    printf '%s\n' "$mx_root" >> "$root/ih-roots"
    mx_prefix="$mx_root/home/.local/share/herdr-threads"
    mkdir -p "$mx_root/home" "$mx_root/tools" "$mx_root/notools" "$mx_root/tmp"
    ln -s "$IH_HERDR" "$mx_root/tools/herdr"
    mx_path="$mx_root/tools"
    mx_relurl="$rel"
    ih_start "$mx_root" "$mx_case" || fail "$mx_case: could not start the isolated Herdr"
}
mx_end() {
    pkill -f "daemon run --state-dir $mx_root/" 2>/dev/null || true
    ih_teardown "$mx_root"
    pass "matrix case $mx_case"
}
mx_mkenv() { # PATHDIR -> mx_run (env words for this root)
    mx_run=(env -i "HOME=$mx_root/home" "PATH=$1:/usr/bin:/bin:/usr/sbin:/sbin"
        "XDG_CONFIG_HOME=$mx_root/cfg" "XDG_STATE_HOME=$mx_root/st" "XDG_RUNTIME_DIR=$mx_root/rt"
        "HERDR_CONFIG_PATH=$mx_root/cfg/herdr.toml" "HERDR_SOCKET_PATH=$mx_root/h.sock"
        "TMPDIR=$mx_root/tmp" "REAL_BIN=$binary" "STUB_LOG=$mx_root/stub.log")
}
mx_install() { # ARGS; PATH dir is $mx_path, release tree $mx_relurl
    mx_mkenv "$mx_path"
    set +e
    (cd "$mx_root" && "${mx_run[@]}" bash "$repo/scripts/install.sh" \
        --release-url "file://$mx_relurl" "$@" < /dev/null > "$out" 2>&1)
    status=$?
    set -e
}
mx_daemons() { pgrep -f "daemon run --state-dir $mx_root/" 2>/dev/null | wc -l | tr -d ' '; }
mx_registered() { # no `grep -q` on a pipe: an early exit would SIGPIPE `plugin list` under pipefail
    local listing
    mx_mkenv "$mx_root/tools"
    listing=$("${mx_run[@]}" "$IH_HERDR" plugin list 2>/dev/null) || return 1
    printf '%s\n' "$listing" | grep -F "[local:$mx_prefix" > /dev/null
}
mx_not_registered() { ! mx_registered; }
mx_expect() { local name=$1; shift; expect "$mx_case: $name" "$@"; }
mx_last() { [ "$(last_line)" = "$1" ]; }
mx_prepare_old() { # an old install, linked, with its daemon running
    mx_path="$mx_root/tools"
    mx_install --version v0.1.0
    { [ "$status" = 0 ] && [ "$(mx_daemons)" = 1 ]; } ||
        fail "$mx_case: precondition: old install linked with one running daemon" "$(cat "$out")"
}

case_fresh_install_herdr_up() {
    mx_install --version v0.1.0
    mx_expect "exits 0" [ "$status" = 0 ]
    mx_expect "final status" mx_last "installed and linked: herdr-threads 0.1.0"
    mx_expect "linked in Herdr" mx_registered
    mx_expect "ensure started one daemon" [ "$(mx_daemons)" = 1 ]
}
case_fresh_install_herdr_down() {
    ih_stop "$mx_root"
    mx_install --version v0.1.0
    mx_expect "exits 0 (Herdr links offline)" [ "$status" = 0 ]
    mx_expect "final status" mx_last "installed and linked: herdr-threads 0.1.0"
    mx_expect "says no daemon was started" has "no daemon was started"
    mx_expect "next steps: start Herdr" has "Start Herdr (or restart its server)"
    mx_expect "no daemon" [ "$(mx_daemons)" = 0 ]
}
case_fresh_install_herdr_absent() {
    mx_path="$mx_root/notools"
    mx_install --version v0.1.0
    mx_expect "exits 3" [ "$status" = 3 ]
    mx_expect "status line names the reason" has "installed, not linked: herdr is not on PATH"
    mx_expect "register command printed" has "register it with: herdr plugin link $mx_prefix"
    mx_expect "files installed anyway" [ -x "$mx_prefix/bin/herdr-threads" ]
    mx_expect "not linked in Herdr" mx_not_registered
    mx_expect "no daemon" [ "$(mx_daemons)" = 0 ]
}
case_fresh_install_no_herdr() {
    mx_install --version v0.1.0 --no-herdr
    mx_expect "exits 3" [ "$status" = 3 ]
    mx_expect "status line names the reason" has "installed, not linked: --no-herdr"
    mx_expect "register command printed" has "register it with: $mx_root/tools/herdr plugin link $mx_prefix"
    mx_expect "Herdr untouched" mx_not_registered
    mx_expect "no daemon" [ "$(mx_daemons)" = 0 ]
}
case_upgrade_herdr_up() {
    mx_prepare_old
    before=$(pgrep -f "daemon run --state-dir $mx_root/")
    mx_install
    mx_expect "exits 0" [ "$status" = 0 ]
    mx_expect "final status" mx_last "upgraded: herdr-threads 0.1.0 -> 0.1.1 (linked)"
    mx_expect "one daemon" [ "$(mx_daemons)" = 1 ]
    mx_expect "daemon restarted (stop, then ensure)" [ "$(pgrep -f "daemon run --state-dir $mx_root/")" != "$before" ]
}
case_upgrade_with_herdr_down() {
    mx_prepare_old
    ih_stop "$mx_root"
    mx_expect "precondition: the old daemon outlives the Herdr server" [ "$(mx_daemons)" = 1 ]
    mx_install
    mx_expect "exits 0" [ "$status" = 0 ]
    mx_expect "old daemon stopped with daemon stop, before replacement" \
        has "stopped the old daemon (herdr-threads daemon stop) before replacing it"
    mx_expect "final status" mx_last "upgraded: herdr-threads 0.1.0 -> 0.1.1 (linked)"
    mx_expect "says no daemon was started" has "no daemon was started"
    mx_expect "no stray daemon afterwards" [ "$(mx_daemons)" = 0 ]
    mx_expect "package replaced" [ "$(cat "$mx_prefix/VERSION")" = 0.1.1 ]
    mx_expect "Herdr was not restarted" [ "$(ih_state "$mx_root")" = stopped ]
}
case_upgrade_herdr_absent() {
    mx_prepare_old
    mx_path="$mx_root/notools"
    mx_install
    mx_expect "exits 3" [ "$status" = 3 ]
    mx_expect "status line" has "upgraded, not linked: herdr is not on PATH"
    mx_expect "package replaced" [ "$(cat "$mx_prefix/VERSION")" = 0.1.1 ]
    mx_expect "old daemon stopped" [ "$(mx_daemons)" = 0 ]
    mx_expect "stop names the old daemon" has "stopped the old daemon"
    mx_expect "no old-daemon warning" bash -c "! grep -qF 'could not stop' '$out'"
}
case_upgrade_no_herdr() {
    mx_prepare_old
    mx_install --no-herdr
    mx_expect "exits 3" [ "$status" = 3 ]
    mx_expect "status line" has "upgraded, not linked: --no-herdr"
    mx_expect "package replaced" [ "$(cat "$mx_prefix/VERSION")" = 0.1.1 ]
    mx_expect "old daemon stopped" [ "$(mx_daemons)" = 0 ]
}
case_uninstall_herdr_up() {
    mx_prepare_old
    mx_install --uninstall --yes
    mx_expect "exits 0" [ "$status" = 0 ]
    mx_expect "final status" mx_last "uninstalled"
    mx_expect "unlinked" mx_not_registered
    mx_expect "files removed" [ ! -e "$mx_prefix" ]
    mx_expect "daemon stopped" [ "$(mx_daemons)" = 0 ]
}
case_uninstall_herdr_down() {
    mx_prepare_old
    ih_stop "$mx_root"
    mx_install --uninstall --yes
    mx_expect "refuses without --force" [ "$status" = 1 ]
    mx_expect "refusal names the cause" has "Herdr could not unlink the plugin"
    mx_expect "files kept" [ -d "$mx_prefix" ]
    mx_install --uninstall --yes --force
    mx_expect "--force exits 3" [ "$status" = 3 ]
    mx_expect "--force status line" has "uninstalled, not unregistered: Herdr could not unlink the plugin"
    mx_expect "--force unregister command" has "unregister it with: herdr plugin unlink herdr-threads"
    mx_expect "--force removed the files" [ ! -e "$mx_prefix" ]
    mx_expect "daemon stopped" [ "$(mx_daemons)" = 0 ]
}
case_uninstall_herdr_absent() {
    mx_prepare_old
    mx_path="$mx_root/notools"
    mx_install --uninstall --yes
    mx_expect "exits 3" [ "$status" = 3 ]
    mx_expect "status line" has "uninstalled, not unregistered: herdr is not on PATH"
    mx_expect "unregister command printed" has "unregister it with: herdr plugin unlink herdr-threads"
    mx_expect "files removed" [ ! -e "$mx_prefix" ]
    mx_expect "daemon stopped" [ "$(mx_daemons)" = 0 ]
    mx_expect "says it stopped the daemon" has "stopped the daemon"
}
case_uninstall_no_herdr() {
    mx_prepare_old
    mx_install --uninstall --yes --no-herdr
    mx_expect "exits 3" [ "$status" = 3 ]
    mx_expect "status line" has "uninstalled, not unregistered: --no-herdr"
    mx_expect "unregister command printed" has "unregister it with: herdr plugin unlink herdr-threads"
    mx_expect "files removed" [ ! -e "$mx_prefix" ]
    mx_expect "Herdr untouched: still registered" mx_registered
    mx_expect "daemon stopped" [ "$(mx_daemons)" = 0 ]
}
case_partial_setup_failure() {
    # A user-level setup build whose `setup` fails, with claude detected.
    printf '#!/bin/sh\necho "2.1.285 (Claude Code)"\n' > "$mx_root/tools/claude"
    chmod 755 "$mx_root/tools/claude"
    : > "$mx_root/stub.log.fail"
    mx_relurl="$stub_rel"
    mx_install --setup
    mx_expect "exits 3" [ "$status" = 3 ]
    mx_expect "status line names the harness" has "installed and linked; setup incomplete: claude"
    mx_expect "setup command printed" has "finish it with: herdr-threads setup"
    mx_expect "setup was attempted" grep -qx setup "$mx_root/stub.log"
    mx_expect "plugin is linked" mx_registered
    mx_install --uninstall --yes
    mx_expect "uninstall afterwards exits 0" [ "$status" = 0 ]
}

for name in fresh_install_herdr_up fresh_install_herdr_down fresh_install_herdr_absent fresh_install_no_herdr \
    upgrade_herdr_up upgrade_with_herdr_down upgrade_herdr_absent upgrade_no_herdr \
    uninstall_herdr_up uninstall_herdr_down uninstall_herdr_absent uninstall_no_herdr partial_setup_failure; do
    if mx_begin "$name"; then "case_$name"; mx_end; fi
done

expect "no daemon left running from the scratch root" [ "$(daemons)" = 0 ]
if [ -n "$skipped_cases" ]; then
    printf 'INSTALL_TEST_PASS %d checks (herdr matrix SKIPPED, no herdr:%s)\n' "$checks" "$skipped_cases"
else
    printf 'INSTALL_TEST_PASS %d checks (stub herdr + isolated real herdr matrix)\n' "$checks"
fi
