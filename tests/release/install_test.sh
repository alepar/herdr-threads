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
# By default Herdr is a recording stub. With HT_RELEASE_TEST_HERDR=/path/to/herdr
# (Herdr 0.9.1) the test also drives the real Herdr CLI against a private
# `herdr server` it starts in the scratch directory and stops by PID; the
# shared Herdr server is never contacted. Prints INSTALL_TEST_PASS on success.
set -euo pipefail

repo=$(CDPATH='' cd "$(dirname "$0")/../.." && pwd -P)
binary=${1:-$repo/target/release/herdr-threads}
[ -x "$binary" ] || { echo "build the release binary first: $binary" >&2; exit 2; }
binary=$(CDPATH='' cd "$(dirname "$binary")" && pwd -P)/$(basename "$binary")
real_herdr=${HT_RELEASE_TEST_HERDR:-}

base=/private/tmp
[ -d "$base" ] || base=${TMPDIR:-/tmp}
root=$(mktemp -d "$base/htit.XXXXXX")
server_pid=''
cleanup() {
    if [ -n "$server_pid" ]; then
        kill "$server_pid" 2>/dev/null || true
        for _ in $(seq 50); do kill -0 "$server_pid" 2>/dev/null || break; sleep 0.1; done
        kill -9 "$server_pid" 2>/dev/null || true
    fi
    # Stop any daemon started from this scratch root.
    pkill -f "daemon run --state-dir $root/" 2>/dev/null || true
    if [ "${KEEP:-0}" = 1 ]; then echo "kept $root"; else rm -rf "$root"; fi
}
trap cleanup EXIT

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
    setup|unsetup)
        if [ "${2:-}" = --help ]; then
            echo "No harness named: every harness. \`setup\` sets up every detected harness"
            echo "Scope (user level, like Herdr's own agent hooks):"
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
if [ -n "$real_herdr" ]; then
    ln -s "$real_herdr" "$tools/herdr"
else
    cat > "$tools/herdr" <<'EOF'
#!/bin/bash
# Recording Herdr stub: registry in $FAKE_HERDR_DIR/registry, server up when
# $FAKE_HERDR_DIR/server exists.
d=$FAKE_HERDR_DIR; reg=$d/registry; echo "$*" >> "$d/calls"
up() { [ -e "$d/server" ] || { echo '{"error":{"code":"server_not_running"}}'; exit 1; }; }
case "$1 $2" in
"plugin list") if [ -s "$reg" ]; then echo "1 plugin installed:"; echo "- herdr-threads (Threads) enabled [local:$(cat "$reg")]"; else echo "No plugins installed."; fi ;;
"plugin link") [ -f "$3/herdr-plugin.toml" ] || exit 1; printf '%s' "$3" > "$reg"; echo '{"result":{"type":"plugin_linked"}}' ;;
"plugin unlink") up; : > "$reg"; echo '{"result":{"removed":true}}' ;;
"plugin log") up; n=$(cat "$d/n" 2>/dev/null || echo 0); echo "{\"result\":{\"logs\":[{\"exit_code\":0,\"log_id\":\"plugin-log-$n\",\"status\":\"succeeded\"}]}}" ;;
"plugin action") up; n=$(( $(cat "$d/n" 2>/dev/null || echo 0) + 1 )); echo $n > "$d/n"; echo "action $4" >> "$d/actions"; echo "{\"result\":{\"log\":{\"log_id\":\"plugin-log-$n\",\"status\":\"running\"}}}" ;;
*) exit 2 ;;
esac
EOF
    chmod 755 "$tools/herdr"
fi
# Harness stand-ins so the installer detects claude.
printf '#!/bin/sh\necho "2.1.285 (Claude Code)"\n' > "$tools/claude"
chmod 755 "$tools/claude"

run_env=(env -i "HOME=$home" "PATH=$tools:/usr/bin:/bin:/usr/sbin:/sbin"
    "XDG_CONFIG_HOME=$root/cfg" "XDG_STATE_HOME=$root/st" "XDG_RUNTIME_DIR=$root/rt"
    "HERDR_CONFIG_PATH=$root/cfg/herdr.toml" "HERDR_SOCKET_PATH=$root/h.sock"
    "FAKE_HERDR_DIR=$root" "STUB_LOG=$root/stub.log" "TMPDIR=$root")
prefix="$home/.local/share/herdr-threads"
link="$home/.local/bin/herdr-threads"
out="$root/out"

# install [ARGS...]: runs the installer (no terminal) from a scratch cwd;
# output in $out, status in $status.
install() {
    set +e
    (cd "$root/cwd" && "${run_env[@]}" bash "$repo/scripts/install.sh" \
        --release-url "file://$rel" "$@" < /dev/null > "$out" 2>&1)
    status=$?
    set -e
}
has() { grep -qF -- "$1" "$out"; }
registered() { "${run_env[@]}" "$tools/herdr" plugin list 2>/dev/null | grep -F "[local:$prefix]" >/dev/null; }
not_registered() { ! registered; }
daemons() { pgrep -f "daemon run --state-dir $root/" 2>/dev/null | wc -l | tr -d ' '; }

if [ -n "$real_herdr" ]; then
    (cd "$root" && "${run_env[@]}" "$real_herdr" server > "$root/server.log" 2>&1 < /dev/null) &
    server_pid=$!
    for _ in $(seq 100); do [ -S "$root/h.sock" ] && grep -q "api socket" "$root/server.log" && break; sleep 0.1; done
    grep -qF "api socket: $root/h.sock" "$root/server.log" || fail "private Herdr server did not start" "$(cat "$root/server.log")"
    pass "private Herdr server owns $root/h.sock"
fi

# --- 1. pinned install ------------------------------------------------------
install --version v0.1.0
[ "$status" = 0 ] || fail "pinned install exits 0" "$(cat "$out")"
pass "pinned install exits 0"
expect "checksum verified" has "checksum verified"
expect "package installed with VERSION 0.1.0" [ "$(cat "$prefix/VERSION")" = 0.1.0 ]
expect "install marker records the version" grep -qx "version=0.1.0" "$prefix/.herdr-threads-install"
expect "executable symlinked" [ "$(readlink "$link")" = "$prefix/bin/herdr-threads" ]
expect "symlink runs" sh -c "'$link' --version > /dev/null"
expect "PATH warning for ~/.local/bin" has "is not on PATH"
expect "plugin linked from the install dir" registered
expect "package build command keeps the prebuilt binary" \
    sh -c "'$prefix/scripts/build.sh' | grep -qF 'prebuilt release package'"
# The installer runs setup only for user-level builds, which it decides from
# `setup --help`; check whichever branch this binary takes, and that neither
# writes hooks without a terminal or --setup.
if "$binary" setup --help 2>/dev/null | grep -qi 'user level'; then
    user_level=1
    expect "user-level setup: no terminal, so setup only suggested" \
        has "detected claude; set up their hooks with: herdr-threads setup (or re-run with --setup)"
    expect "user-level setup: not run without a terminal" bash -c "! grep -qF 'set up hooks for the detected harnesses' '$out'"
    expect "user-level setup: nothing written to the scratch ~/.claude" [ ! -e "$home/.claude" ]
else
    user_level=0
    expect "per-project setup is not run from the installer" has "this build's setup is per project"
fi
expect "no project settings written in the cwd" [ ! -e "$root/cwd/.claude" ]
expect "next steps printed" has "Next steps:"
expect "next steps: hooks not set up, so setup suggested" has "Set up agent hooks: herdr-threads setup"
expect "next steps: no Codex trust reminder without Codex setup" bash -c "! grep -qF 'Trust the Codex hooks' '$out'"
expect "next steps: try-it points at me init" has "herdr-threads me init"
expect "next steps: no stale agent-guide URL" bash -c "! grep -qF 'docs/agent-usage.md' '$out'"
if [ -n "$real_herdr" ]; then
    expect "ensure started one daemon from the install" [ "$(daemons)" = 1 ]
    expect "installer reports the running daemon" has "daemon is running"
    expect "next steps: daemon running, so just doctor" has "Check it: herdr-threads doctor"
else
    expect "stub server down: no actions invoked" [ ! -e "$root/actions" ]
    expect "next steps: server down, so start Herdr" has "Start Herdr (or restart its server)"
    expect "next steps: doctor after starting Herdr" has "then check it: herdr-threads doctor"
fi
touch "$prefix/.sentinel"   # survives only if the re-run leaves the dir alone

# --- 2. idempotent re-run ---------------------------------------------------
install --version 0.1.0
[ "$status" = 0 ] || fail "re-run exits 0" "$(cat "$out")"
pass "re-run exits 0"
expect "re-run reports already installed" has "0.1.0 is already installed"
expect "re-run leaves the install dir in place" [ -e "$prefix/.sentinel" ]
expect "re-run keeps the registration" has "already has the plugin linked"
expect "re-run does not relink the symlink" bash -c "! grep -qF 'linked $link' '$out'"
if [ -n "$real_herdr" ]; then expect "re-run keeps one daemon" [ "$(daemons)" = 1 ]; fi

# --- 3. upgrade to latest ---------------------------------------------------
if [ -z "$real_herdr" ]; then touch "$root/server"; fi   # stub server up from here on
daemon_before=$(pgrep -f "daemon run --state-dir $root/" || true)
install
[ "$status" = 0 ] || fail "upgrade exits 0" "$(cat "$out")"
pass "upgrade exits 0"
expect "upgrade reported" has "upgraded $prefix (0.1.0 -> 0.1.1)"
expect "latest is 0.1.1" [ "$(cat "$prefix/VERSION")" = 0.1.1 ]
expect "no staging dir left" [ ! -e "$prefix.new" ]
expect "no backup dir left" [ ! -e "$prefix.old" ]
expect "registration survives the upgrade" registered
if [ -n "$real_herdr" ]; then
    expect "upgrade leaves exactly one daemon" [ "$(daemons)" = 1 ]
    expect "upgrade restarted the daemon (stop, then ensure)" \
        [ "$(pgrep -f "daemon run --state-dir $root/")" != "$daemon_before" ]
else
    expect "upgrade stops then ensures the daemon" \
        [ "$(tr '\n' ' ' < "$root/actions")" = "action stop action ensure " ]
fi
expect "next steps: daemon running, so check with doctor" has "Check it: herdr-threads doctor"
expect "next steps: daemon running, no start-Herdr step" bash -c "! grep -qF 'Start Herdr' '$out'"

# --- 4. refusals --------------------------------------------------------------
cp -R "$rel" "$root/bad-rel"
printf '%s  %s\n' "$(printf '%064d' 1)" "$asset" > "$root/bad-rel/download/v0.1.0/SHA256SUMS"
install --version v0.1.0 --release-url "file://$root/bad-rel"
expect "checksum mismatch refused" [ "$status" != 0 ]
expect "checksum mismatch named" has "checksum mismatch"
expect "failed install leaves the previous version" [ "$(cat "$prefix/VERSION")" = 0.1.1 ]

install --version v9.9.9
expect "missing release refused" [ "$status" != 0 ]
expect "missing release named" has "download failed"

mkdir -p "$root/foreign"
install --prefix "$root/foreign" --no-herdr
expect "unmanaged prefix refused" [ "$status" != 0 ]
expect "unmanaged prefix named" has "was not created by this installer"

mkdir -p "$root/bin2" && echo keep > "$root/bin2/herdr-threads"
install --bin-dir "$root/bin2" --no-herdr
expect "regular file at the symlink path refused" [ "$status" != 0 ]
expect "regular file at the symlink path named" has "is not a symlink"
expect "regular file kept" [ "$(cat "$root/bin2/herdr-threads")" = keep ]

# --- 5. uninstall -------------------------------------------------------------
install --uninstall
[ "$status" = 0 ] || fail "uninstall exits 0" "$(cat "$out")"
pass "uninstall exits 0"
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
if [ -n "$real_herdr" ]; then
    expect "uninstall stopped the daemon" [ "$(daemons)" = 0 ]
else
    expect "uninstall stopped the daemon" [ "$(tail -n 1 "$root/actions")" = "action stop" ]
fi
install --uninstall
expect "second uninstall is a no-op success" [ "$status" = 0 ]

# Uninstall while Herdr cannot unlink refuses unless --force.
if [ -z "$real_herdr" ]; then
    install --version v0.1.0
    rm -f "$root/server"
    install --uninstall
    expect "uninstall refuses when Herdr cannot unlink" [ "$status" != 0 ]
    expect "refused uninstall keeps the files" [ -d "$prefix" ]
    install --uninstall --force
    expect "uninstall --force exits 0" [ "$status" = 0 ]
    expect "uninstall --force removes the files" [ ! -e "$prefix" ]
    touch "$root/server"
    "${run_env[@]}" "$tools/herdr" plugin unlink herdr-threads > /dev/null
fi

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
expect "no terminal and no --setup: setup only suggested" has "set up their hooks with: herdr-threads setup"
expect "no second setup call" [ "$(grep -c '^setup' "$root/stub.log")" = 1 ]
install --uninstall
[ "$status" = 0 ] || fail "uninstall after setup exits 0" "$(cat "$out")"
expect "uninstall removes the owned hooks with bare unsetup" grep -qx "unsetup" "$root/stub.log"
expect "uninstall leaves no install dir" [ ! -e "$prefix" ]
expect "uninstall leaves no symlink" [ ! -L "$link" ]

# --- 7. Codex detected: --setup reminds about the one-time hook trust ---------
printf '#!/bin/sh\necho "codex-cli 0.159.2"\n' > "$tools/codex"
chmod 755 "$tools/codex"
install --release-url "file://$stub_rel" --setup
[ "$status" = 0 ] || fail "--setup install with codex exits 0" "$(cat "$out")"
expect "codex --setup next steps: Codex trust reminder" has "Trust the Codex hooks once"
expect "codex --setup next steps: no setup suggestion" bash -c "! grep -qF 'Set up agent hooks' '$out'"
install --release-url "file://$stub_rel"
expect "codex without --setup: no Codex trust reminder" bash -c "! grep -qF 'Trust the Codex hooks' '$out'"
expect "codex without --setup: setup suggested" has "Set up agent hooks: herdr-threads setup"
: > "$root/stub.log.fail"
install --release-url "file://$stub_rel" --setup
[ "$status" = 0 ] || fail "--setup install with failing setup exits 0" "$(cat "$out")"
expect "failed setup: next steps say to fix setup" has "Fix agent hook setup (see above)"
expect "failed setup: no Codex trust reminder" bash -c "! grep -qF 'Trust the Codex hooks' '$out'"
rm -f "$root/stub.log.fail" "$tools/codex"
install --uninstall
[ "$status" = 0 ] || fail "uninstall after codex setup exits 0" "$(cat "$out")"

if [ -n "$real_herdr" ]; then expect "no daemon left running" [ "$(daemons)" = 0 ]; fi
printf 'INSTALL_TEST_PASS %d checks (%s)\n' "$checks" "$([ -n "$real_herdr" ] && echo "real herdr $real_herdr" || echo "stub herdr")"
