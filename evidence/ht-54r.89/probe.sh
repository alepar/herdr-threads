#!/bin/bash
# ht-54r.89 probe: for each harness, start the real binary in a pane of ONE isolated Herdr session
# (scripts/lib/isolated-herdr.sh, isolated HOME / CLAUDE_CONFIG_DIR / CODEX_HOME / HERMES_HOME),
# record `herdr pane process-info`, the pane's process tree and the fingerprint pid the adapter's
# CallerEnvShape rule selects. No credentials are used: every harness stops at its login / setup prompt.
#
# usage: TMPDIR=<short temp dir> probe.sh OUT_DIR
# env:   CLAUDE_BIN CODEX_BIN CODEX_NPM_BIN HERMES_BIN AGY_BIN (a missing one is recorded as skipped)
set -u
OUT=${1:?usage: probe.sh OUT_DIR}; mkdir -p "$OUT"
HERE=$(cd "$(dirname "$0")" && pwd); REPO=$(cd "$HERE/../.." && pwd)
# shellcheck source=../../scripts/lib/isolated-herdr.sh
. "$REPO/scripts/lib/isolated-herdr.sh"
ih_trap_install
R=$(ih_root_new); ih_start "$R" ht-54r.89-probe || exit 1
hi() { ( IFS='
'; exec env $(ih_env_args "$R") herdr "$@" ); }

# fingerprint rule = caller_env::walk + fingerprint_for: the nearest harness-shaped ancestor of a
# process the harness spawned; for Claude the pid it exports in CLAUDE_PID (its own pid).
# Candidate shapes (src/harness/{claude,codex,hermes,agy}): argv0 basename match, plus
# codex `codex-*` / `@openai/codex` argv, hermes `python*` argv containing hermes_cli or /.hermes/.
shape_match() { # HARNESS COMM-ARGS-LINE -> 0 when the process is harness-shaped
    case "$1" in
    claude) printf '%s' "$2" | awk '{b=$1; sub(".*/","",b); exit !(b=="claude")}' ;;
    codex)  printf '%s' "$2" | awk '{b=$1; sub(".*/","",b); exit !(b=="codex" || b ~ /^codex-/ || $0 ~ /@openai\/codex/ || $0 ~ /\/codex\.js/)}' ;;
    hermes) printf '%s' "$2" | awk '{b=$1; sub(".*/","",b); exit !((b=="hermes") || (b ~ /^python/ && ($0 ~ /hermes_cli/ || $0 ~ /\/\.hermes\//)))}' ;;
    agy)    printf '%s' "$2" | awk '{b=$1; sub(".*/","",b); exit !(b=="agy"||b=="antigravity"||b=="antigravity-cli")}' ;;
    esac
}

descendants() { # SHELL_PID -> pids of every descendant (any depth)
    ps -axo pid=,ppid= | awk -v root="$1" '{p[$1]=$2; ids[n++]=$1}
        END{for(i=0;i<n;i++){x=ids[i]; while(x in p && x>1){ if(p[x]==root){print ids[i]; break} x=p[x]}}}'
}

capture() { # NAME HARNESS PANE
    name=$1; harness=$2; pane=$3; d="$OUT/$name"; mkdir -p "$d"
    for s in 1 2 3; do hi pane process-info --pane "$pane" > "$d/process-info.$s.json"; sleep 1; done
    cp "$d/process-info.3.json" "$d/process-info.json"
    shell=$(python3 -c 'import sys,json;print(json.load(sys.stdin)["result"]["process_info"]["shell_pid"])' < "$d/process-info.json")
    {   echo "# pid ppid pgid tpgid stat comm args  (shell $shell and its descendants)"
        ps -axo pid=,ppid=,pgid=,tpgid=,stat=,comm=,args= | awk -v s="$shell" '$1==s{print}'
        for p in $(descendants "$shell"); do ps -o pid=,ppid=,pgid=,tpgid=,stat=,comm=,args= -p "$p"; done
    } | cut -c1-220 > "$d/ps-tree.txt"
    python3 - "$d" "$harness" <<'PY' > "$d/foreground.txt"
import json,sys
d,h=sys.argv[1:]
for s in "1","2","3":
    pi=json.load(open(f"{d}/process-info.{s}.json"))["result"]["process_info"]
    fg=[p["pid"] for p in pi["foreground_processes"] if p["pid"]!=pi["shell_pid"]]
    print(f"sample {s}: shell_pid={pi['shell_pid']} non_shell_foreground={sorted(fg)} started_process_pid(current)={fg[0] if len(fg)==1 else None}")
PY
    # fingerprint candidates: harness-shaped descendants of the shell; the DEEPEST one is the nearest
    # ancestor of anything the harness spawns (the rule's pick)
    : > "$d/fingerprint-candidates.txt"
    for p in $(descendants "$shell"); do
        line=$(ps -o comm=,args= -p "$p" 2>/dev/null | cut -c1-300) || continue
        argv0=$(ps -o args= -p "$p" | awk '{print $1}')
        shape_match "$harness" "$(ps -o args= -p "$p")" && echo "$p $argv0" >> "$d/fingerprint-candidates.txt"
    done
    echo "candidates (harness-shaped descendants): $(tr '\n' ';' < "$d/fingerprint-candidates.txt")" >> "$d/foreground.txt"
}

skip() { mkdir -p "$OUT/$1"; echo "skipped: $2" > "$OUT/$1/SKIPPED.txt"; echo "$1: skipped: $2"; }
# One workspace (one root pane, one shell) per harness; the pane id comes from the create answer.
newpane() {
    hi workspace create --label "$1" --no-focus | python3 -c 'import sys,json;print(json.load(sys.stdin)["result"]["root_pane"]["pane_id"])'
}
waitfor() { # PANE REGEX SECONDS
    i=0; while [ $i -lt "$3" ]; do hi pane read "$1" 2>/dev/null | grep -Eq "$2" && return 0; sleep 1; i=$((i + 1)); done; return 1
}
HOMEDIR="$R/home"; mkdir -p "$HOMEDIR/claude-cfg" "$HOMEDIR/codex-home" "$HOMEDIR/codex-npm-home" "$HOMEDIR/hermes-home"

if [ -x "${CLAUDE_BIN:-}" ]; then
    pane=$(newpane claude); "$CLAUDE_BIN" --version > /dev/null 2>&1; "$CLAUDE_BIN" --version > "$OUT/claude.version" 2>&1
    hi pane run "$pane" "CLAUDE_CONFIG_DIR=$HOMEDIR/claude-cfg $CLAUDE_BIN"; waitfor "$pane" 'Dark mode|Choose|theme' 40; sleep 2
    capture claude claude "$pane"
else skip claude "CLAUDE_BIN not executable"; fi

if [ -x "${CODEX_BIN:-}" ]; then
    pane=$(newpane codex); "$CODEX_BIN" --version > "$OUT/codex.version" 2>&1
    hi pane run "$pane" "CODEX_HOME=$HOMEDIR/codex-home $CODEX_BIN --no-daemon"; waitfor "$pane" 'Press enter|Sign in' 40; sleep 3
    capture codex-native codex "$pane"
else skip codex-native "CODEX_BIN not executable"; fi

if [ -x "${CODEX_NPM_BIN:-}" ]; then
    pane=$(newpane codex-npm); "$CODEX_NPM_BIN" --version > "$OUT/codex-npm.version" 2>&1
    hi pane run "$pane" "PATH=$(dirname "$CODEX_NPM_BIN"):\$PATH CODEX_HOME=$HOMEDIR/codex-npm-home codex --no-daemon"; waitfor "$pane" 'Press enter|Sign in' 40; sleep 3
    capture codex-npm codex "$pane"
else skip codex-npm "CODEX_NPM_BIN not executable (npm i -g --prefix <tmp> @openai/codex@<ver> not run)"; fi

if [ -x "${HERMES_BIN:-}" ]; then
    pane=$(newpane hermes); "$HERMES_BIN" --version > "$OUT/hermes.version" 2>&1
    hi pane run "$pane" "HERMES_HOME=$HOMEDIR/hermes-home $HERMES_BIN"; waitfor "$pane" 'Run setup now' 400; sleep 3
    capture hermes hermes "$pane"
else skip hermes "HERMES_BIN not executable"; fi

if [ -x "${AGY_BIN:-}" ]; then
    pane=$(newpane agy); "$AGY_BIN" --version > "$OUT/agy.version" 2>&1
    hi pane run "$pane" "$AGY_BIN"; waitfor "$pane" 'login method' 40; sleep 2
    capture agy agy "$pane"
else skip agy "AGY_BIN not executable"; fi

# Read-only: the live Claude session this task runs under, against the SHARED server's socket (a
# single `pane process-info --current`, never a mutation). CLAUDE_PID is exported by that Claude.
if [ -n "${HERDR_PANE_ID:-}" ] && [ -n "${CLAUDE_PID:-}" ]; then
    d="$OUT/claude-live"; mkdir -p "$d"
    herdr pane process-info --pane "$HERDR_PANE_ID" > "$d/process-info.json" 2>&1
    echo "CLAUDE_PID=$CLAUDE_PID" > "$d/claude-pid.txt"
    { echo "# ancestry of the probe shell (pid ppid pgid tpgid stat comm args)"; p=$$
      while [ "$p" -gt 1 ]; do ps -o pid=,ppid=,pgid=,tpgid=,stat=,comm=,args= -p "$p" | cut -c1-200; p=$(ps -o ppid= -p "$p" | tr -d ' '); done; } > "$d/ancestry.txt"
fi
echo "probe done: $OUT"
