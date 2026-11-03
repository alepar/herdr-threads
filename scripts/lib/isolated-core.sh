# shellcheck shell=sh
# Isolated herdr-core test sessions (ht-kqrt.76). Source this file. Every core it
# starts lives under one private root /tmp/ic.XXXXXX with `env -i`, its own
# HC_SESSION=ht-test-<uuid> and scratch XDG dirs, is started only through
# `hc run --bg` and is stopped with `server.stop` over the private socket. Never
# `hc server status`: it autostarts a core. IC_SESSION_PREFIX (default ht-test) names
# the session prefix; scripts/hc-smoke sets ht-smoke. Policy: no `hc` on PATH makes
# ic_start a no-op that prints "skipped: no hc: <case>" and returns 2.
IC_HC=$(command -v hc 2>/dev/null || true)

ic_require_hc() { # CASE
    [ -n "$IC_HC" ] && return 0
    echo "skipped: no hc: ${1:-unknown}"; return 2
}

ic_sockets() { # ROOT -> socket files under the private runtime dir
    find "$1/rt" -type s 2>/dev/null
}

ic_pids() { # SESSION -> pids of `herdr-core serve`/`hold` carrying HC_SESSION=SESSION
    ps axeww -o pid= -o command= 2>/dev/null | awk -v tag="HC_SESSION=$1" '
        $2 ~ /herdr-core$/ && ($3 == "serve" || $3 == "hold") {
            for (i = 4; i <= NF; i++) if ($i == tag) { print $1; break } }'
}

ic_start() { # CASE -> prints the root on success; 2 (skip) when hc is absent
    ic_require_hc "$1"; rc=$?
    [ "$rc" = 0 ] || return "$rc"
    root=$(mktemp -d /tmp/ic.XXXXXX) || return 1
    session="${IC_SESSION_PREFIX:-ht-test}-$(cat /proc/sys/kernel/random/uuid 2>/dev/null || date +%s)"
    mkdir -p "$root/st" "$root/rt" "$root/cfg" && chmod 700 "$root/st" "$root/rt" "$root/cfg"
    printf '%s\n' "$session" > "$root/session"
    if ! env -i HOME="${HOME:-/}" USER="${USER:-}" LOGNAME="${LOGNAME:-}" SHELL="${SHELL:-/bin/sh}" \
        TMPDIR="${TMPDIR:-/tmp}" PATH="$PATH" HT_LEAK_RUN_ID="${HT_LEAK_RUN_ID:-}" \
        HT_TEST_OWNER="${HT_TEST_OWNER:-}" HC_SESSION="$session" XDG_STATE_HOME="$root/st" \
        XDG_RUNTIME_DIR="$root/rt" XDG_CONFIG_HOME="$root/cfg" \
        "$IC_HC" run --bg </dev/null >"$root/hc-run.out" 2>&1; then
        echo "isolated-core: hc run --bg failed (see $root/hc-run.out)" >&2; return 1
    fi
    i=0
    while [ $i -lt 100 ]; do
        [ -n "$(ic_sockets "$root")" ] && { echo "$root"; return 0; }
        sleep 0.1; i=$((i + 1))
    done
    echo "isolated-core: no core socket in 10 s (see $root/hc-run.out)" >&2; return 1
}

ic_server_stop() { # SOCKET -> one JSON-RPC server.stop line
    printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"server.stop","params":{}}' |
        socat -t2 - "UNIX-CONNECT:$1" >/dev/null 2>&1 && return 0
    python3 - "$1" <<'PY_STOP' >/dev/null 2>&1
import socket
import sys
s = socket.socket(socket.AF_UNIX)
s.settimeout(2)
s.connect(sys.argv[1])
s.sendall(b'{"jsonrpc":"2.0","id":1,"method":"server.stop","params":{}}\n')
try:
    s.recv(4096)
except OSError:
    pass
PY_STOP
}

ic_stop() { # ROOT -> server.stop, wait for the socket to go, verify no process of the session remains, remove ROOT
    case "$1" in /tmp/ic.*|/private/tmp/ic.*) ;; *) echo "isolated-core: refusing to remove $1" >&2; return 1 ;; esac
    session=$(cat "$1/session" 2>/dev/null || true)
    for sock in $(ic_sockets "$1"); do ic_server_stop "$sock"; done
    i=0
    while [ $i -lt 100 ] && { [ -n "$(ic_sockets "$1")" ] || [ -n "$(ic_pids "$session")" ]; }; do
        sleep 0.1; i=$((i + 1))
    done
    left=$(ic_pids "$session")
    if [ -n "$session" ] && [ -n "$left" ]; then
        echo "isolated-core: herdr-core still running for $session: $left" >&2; return 1
    fi
    rm -rf "$1"
}
