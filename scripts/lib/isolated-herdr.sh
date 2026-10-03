# shellcheck shell=sh
# Isolated named Herdr test sessions (ht-p03.1). Source this file; never run the
# shared Herdr server through it. Every server it starts lives under one private
# root /tmp/ih.XXXXXX with its own HOME, XDG dirs, config and API socket, and is
# signalled only after an ownership check. Policy: no `herdr` on PATH fails naming
# the binary, unless HT_SKIP_HERDR_TESTS=1 (then each start reports
# "skipped: no herdr: <case>").
IH_HERDR=$(command -v herdr 2>/dev/null || true)

# Owner tag (ht-p03.131): inherited by every server, pane and daemon started
# from here; a Rust test passes its own pid, a script defaults to itself.
# `ih_reaper_ensure` starts one detached reaper per owner: once the owner pid is
# gone (exit, panic, SIGTERM or SIGKILL alike) it tears down every root whose
# `owner` file names that pid, then stops every process group that test recorded
# (`<lock dir>/g.<pgid>`, written by `spawn_owned`) and every process tagged
# with it. A
# reaper that is itself SIGKILLed leaves /tmp/ih-reaper.<owner>, which only
# matters if that pid is reused.
: "${HT_TEST_OWNER:=$$}"; export HT_TEST_OWNER
# shellcheck disable=SC3028
IH_LIB=${IH_LIB:-${BASH_SOURCE:-}} # bash callers; the Rust helper exports IH_LIB

ih_require_herdr() { # CASE
    [ -n "$IH_HERDR" ] && return 0
    if [ "${HT_SKIP_HERDR_TESTS:-}" = 1 ]; then return 2; fi
    echo "isolated-herdr: required binary 'herdr' not found on PATH (case: ${1:-unknown}; set HT_SKIP_HERDR_TESTS=1 to skip)" >&2
    return 1
}

ih_root_new() {
    r=$(mktemp -d /tmp/ih.XXXXXX) && printf '%s\n' "$HT_TEST_OWNER" > "$r/owner" && echo "$r"
}

ih_tagged_pids() { # OWNER -> pids whose environment carries HT_TEST_OWNER=OWNER
    ps axeww -o pid= -o command= 2>/dev/null | awk -v tag="HT_TEST_OWNER=$1" '
        { for (i = 2; i <= NF; i++) if ($i == tag) { print $1; break } }'
}

ih_root_pids() { # ROOT -> pids whose argv or environment names ROOT (never this shell)
    # shellcheck disable=SC2009
    ps axeww -o pid= -o command= 2>/dev/null | grep -F -- "$1/" | awk -v self="$$" '$1 != self { print $1 }'
}

ih_kill_pids() { # PID... -> TERM, wait up to 2 s, KILL survivors
    [ $# -gt 0 ] || return 0
    # A listed pid may already be gone (ih_root_pids also lists its own ps/grep pipeline
    # while it runs), and a vanished pid must not abort a `set -e` caller.
    kill -TERM "$@" 2>/dev/null || true; i=0
    while [ $i -lt 20 ]; do
        live=''; for p in "$@"; do kill -0 "$p" 2>/dev/null && live="$live $p"; done
        [ -n "$live" ] || return 0; sleep 0.1; i=$((i + 1))
    done
    # shellcheck disable=SC2086
    kill -KILL $live 2>/dev/null; return 0
}

ih_group_alive() { # PGID -> 0 while any process is still in that group
    ps -axo pgid= 2>/dev/null | awk -v g="$1" '$1 == g { f = 1 } END { exit !f }'
}

ih_recorded_groups() { # OWNER -> process groups a Rust test recorded (g.<pgid> files) that still exist
    for f in "/tmp/ih-reaper.$1"/g.*; do
        [ -e "$f" ] || continue
        g=${f##*/g.}; ih_group_alive "$g" && echo "$g"
    done; return 0
}

ih_kill_groups() { # PGID... -> TERM each group, wait up to 2 s, KILL what is left
    [ $# -gt 0 ] || return 0
    for g in "$@"; do kill -s TERM -- "-$g" 2>/dev/null; done; i=0
    while [ $i -lt 20 ]; do
        live=''; for g in "$@"; do ih_group_alive "$g" && live="$live $g"; done
        [ -n "$live" ] || return 0; sleep 0.1; i=$((i + 1))
    done
    for g in $live; do kill -s KILL -- "-$g" 2>/dev/null; done; return 0
}

ih_teardown_owned() { # [OWNER] -> tear down every root whose owner file names OWNER
    o=${1:-$HT_TEST_OWNER}
    for r in /tmp/ih.* /private/tmp/ih.*; do
        [ -f "$r/owner" ] && [ "$(cat "$r/owner")" = "$o" ] && ih_teardown "$r"
    done; return 0
}

ih_trap_install() { # opt-in for scripts: teardown on exit and on fatal signals
    trap 'ih_teardown_owned' EXIT
    trap 'exit 129' HUP; trap 'exit 130' INT; trap 'exit 143' TERM
}

ih_reaper_loop() { # OWNER -> wait for OWNER to die, then reap what it left
    while kill -0 "$1" 2>/dev/null; do sleep 0.2; done
    # Process groups first: ps cannot read the environment of Apple platform
    # binaries (/bin/sleep, /bin/sh), so the tag below cannot find those.
    # shellcheck disable=SC2046
    ih_kill_groups $(ih_recorded_groups "$1")
    ih_teardown_owned "$1"
    # shellcheck disable=SC2046
    ih_kill_pids $(ih_tagged_pids "$1")
    rm -rf "/tmp/ih-reaper.$1"; return 0
}

ih_reaper_ensure() { # [OWNER] -> start one detached reaper per owner (mkdir lock)
    o=${1:-$HT_TEST_OWNER}
    mkdir "/tmp/ih-reaper.$o" 2>/dev/null || return 0
    [ -n "$IH_LIB" ] || { echo "isolated-herdr: no IH_LIB; reaper not started" >&2; rmdir "/tmp/ih-reaper.$o"; return 0; }
    # exec'd with the tag removed, so the reaper never matches its own scan
    ( trap '' HUP INT TERM
      # shellcheck disable=SC2016
      exec env -u HT_TEST_OWNER -u HERDR_THREADS_TEST_OWNER_PID HT_SKIP_HERDR_TESTS=1 \
        /bin/sh -c '. "$1"; ih_reaper_loop "$2"' sh "$IH_LIB" "$o" ) </dev/null >/dev/null 2>&1 &
    return 0
}

ih_env_args() { # ROOT -> words for `env`: unset inherited HERDR_*, then the private vars
    for name in $(env | sed -n 's/^\(HERDR_[A-Za-z0-9_]*\)=.*/\1/p'); do printf -- '-u\n%s\n' "$name"; done
    printf '%s\n' "HOME=$1/home" "XDG_CONFIG_HOME=$1/cfg" "XDG_STATE_HOME=$1/st" "XDG_RUNTIME_DIR=$1/rt" \
        "HERDR_CONFIG_PATH=$1/cfg/herdr.toml" "HERDR_SOCKET_PATH=$1/h.sock"
}

ih_owned() { # PID ROOT -> 0 when PID is this root's `herdr server`
    cmd=$(ps -o command= -p "$1" 2>/dev/null) || return 1
    case "$cmd" in *"herdr server") ;; *) return 1 ;; esac
    ps eww -o command= -p "$1" 2>/dev/null | grep -qF "$2"
}

ih_pid() { [ -f "$1/server.pid" ] && cat "$1/server.pid"; }

ih_start() { # ROOT CASE
    ih_require_herdr "$2"; rc=$?
    if [ "$rc" = 2 ]; then echo "skipped: no herdr: $2"; return 0; fi
    [ "$rc" = 0 ] || return 1
    pid=$(ih_pid "$1"); if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then echo "isolated-herdr: already up" >&2; return 1; fi
    ih_reaper_ensure
    mkdir -p "$1/home" "$1/cfg" "$1/st" "$1/rt" && chmod 700 "$1/home" "$1/cfg" "$1/st" "$1/rt"
    [ -f "$1/cfg/herdr.toml" ] || : > "$1/cfg/herdr.toml"
    rm -f "$1/h.sock"
    n=$(( $(cat "$1/starts" 2>/dev/null || echo 0) + 1 )); echo "$n" > "$1/starts"
    log="$1/server-$n.out"
    # shellcheck disable=SC2046
    ( IFS='
'; exec env $(ih_env_args "$1") "$IH_HERDR" server </dev/null >"$log" 2>&1 ) &
    echo $! > "$1/server.pid"
    i=0
    while [ $i -lt 150 ]; do
        if [ -S "$1/h.sock" ] && grep -q 'api socket:' "$log" 2>/dev/null; then return 0; fi
        kill -0 "$(ih_pid "$1")" 2>/dev/null || { echo "isolated-herdr: server exited early (see $log)" >&2; return 1; }
        sleep 0.1; i=$((i + 1))
    done
    echo "isolated-herdr: server did not create its socket in 15 s (see $log)" >&2; return 1
}

ih_stop() { # ROOT
    pid=$(ih_pid "$1"); [ -n "$pid" ] || return 0
    if kill -0 "$pid" 2>/dev/null; then
        ih_owned "$pid" "$1" || { echo "isolated-herdr: refusing to signal $pid (not this root's server)" >&2; return 1; }
        if [ -S "$1/h.sock" ]; then # graceful first, inside the private env, bounded to 5 s
            # shellcheck disable=SC2046
            ( IFS='
'; exec env $(ih_env_args "$1") "$IH_HERDR" server stop ) >/dev/null 2>&1 &
            stopper=$!; i=0
            while kill -0 "$pid" 2>/dev/null && [ $i -lt 50 ]; do sleep 0.1; i=$((i + 1)); done
            kill -0 "$stopper" 2>/dev/null && kill -TERM "$stopper" 2>/dev/null
            wait "$stopper" 2>/dev/null
        fi
        kill -0 "$pid" 2>/dev/null && kill -TERM "$pid"; i=0
        while kill -0 "$pid" 2>/dev/null && [ $i -lt 150 ]; do sleep 0.1; i=$((i + 1)); done
        kill -0 "$pid" 2>/dev/null && kill -KILL "$pid"
    fi
    rm -f "$1/server.pid"; i=0
    while [ -e "$1/h.sock" ] && [ $i -lt 100 ]; do sleep 0.1; i=$((i + 1)); done
    rm -f "$1/h.sock"
}

ih_kill() { # ROOT -> SIGKILL, leaving the socket file behind (stale socket)
    pid=$(ih_pid "$1"); [ -n "$pid" ] || return 0
    ih_owned "$pid" "$1" || { echo "isolated-herdr: refusing to signal $pid" >&2; return 1; }
    kill -KILL "$pid"; i=0
    while kill -0 "$pid" 2>/dev/null && [ $i -lt 100 ]; do sleep 0.1; i=$((i + 1)); done
    rm -f "$1/server.pid"
}

ih_stale_socket() { # ROOT CASE -> a socket file whose server is dead
    [ -n "$(ih_pid "$1")" ] || ih_start "$1" "$2" || return 1
    ih_kill "$1" && [ -S "$1/h.sock" ]
}

ih_restart() { ih_stop "$1" && ih_start "$1" "$2"; } # ROOT CASE

ih_state() { # ROOT -> up | stale-socket | stopped | never-started
    pid=$(ih_pid "$1")
    if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then echo up
    elif [ -S "$1/h.sock" ]; then echo stale-socket
    elif [ -f "$1/starts" ]; then echo stopped
    else echo never-started; fi
}

ih_teardown() { # ROOT -> stop (or kill) our server, remove the root
    case "$1" in /tmp/ih.*|/private/tmp/ih.*) ;; *) echo "isolated-herdr: refusing to remove $1" >&2; return 1 ;; esac
    ih_stop "$1" || ih_kill "$1"
    # Panes run in their own sessions, so sweep everything that names the root.
    # shellcheck disable=SC2046
    ih_kill_pids $(ih_root_pids "$1")
    rm -rf "$1"
}

# Sourcing enforces the policy (skipped cases still source fine).
ih_require_herdr "${IH_CASE:-source}"; [ $? = 1 ] && return 1
:
