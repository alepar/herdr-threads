#!/usr/bin/env bash
# Model-free capture of `herdr agent read --source detection` for genuine agy states
# (ht-poh.4.5). No model call is made: the fake Gemini server (the frozen research
# probe copy, harness-fake_server.py) answers every request from a scripted queue.
#
# Usage: HT_AGY_BIN=/path/to/agy tests/fixtures/agy/composer/capture.sh [OUT_DIR]
#
# Everything runs in one private root /tmp/ih.XXXXXX with its own Herdr session, HOME
# and fake server; agy runs under the loopback sandbox (scripts/agy-sandbox). The root
# is torn down on exit. Captures are written to OUT_DIR (default: this directory), with
# the root path rewritten to <root>.
set -u
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO=$(cd "$HERE/../../../.." && pwd)
OUT=${1:-$HERE}
: "${HT_AGY_BIN:?set HT_AGY_BIN to the genuine agy binary}"
[ -x "$HT_AGY_BIN" ] || { echo "HT_AGY_BIN is not executable: $HT_AGY_BIN" >&2; exit 1; }
mkdir -p "$OUT"
# shellcheck source=../../../../scripts/lib/isolated-herdr.sh
. "$REPO/scripts/lib/isolated-herdr.sh"
ih_trap_install
REAL_HOME=$(cd ~ && pwd)

ROOT=$(ih_root_new) || exit 1
ROOT=$(cd "$ROOT" && pwd -P) # the sandbox matches real paths (/private/tmp)
ih_start "$ROOT" composer-capture || exit 1
mkdir -p "$ROOT/tmp" "$ROOT/ctl" "$ROOT/ws"
hd() { ( IFS='
'; exec env $(ih_env_args "$ROOT") "$IH_HERDR" "$@" ); }

# Sandbox profile: the repo template, loopback only, writes confined to the root.
sed -e "s|<REAL_HOME>|$REAL_HOME|g" -e "s|<ROOT>|$ROOT|g" \
    "$REPO/scripts/agy-sandbox/loopback.sb.in" > "$ROOT/loopback.sb"

# D5 seed settings: no alt screen, gemini provider, trusted workspace. The allow entry
# lets the tool-running subagent state run `sleep`; the permission-prompt state runs
# `echo`, which is not allowed.
mkdir -p "$ROOT/home/.gemini/antigravity-cli"
cat > "$ROOT/home/.gemini/antigravity-cli/settings.json" <<JSON
{
  "altScreenMode": "never",
  "modelProvider": "gemini",
  "permissions": {"allow": ["command(sleep)"]},
  "trustedWorkspaces": ["$ROOT/ws"]
}
JSON

cp "$HERE/harness-fake_server.py" "$ROOT/fake_server.py"
python3 "$ROOT/fake_server.py" "$ROOT/ctl" >/dev/null 2>&1 &
for _ in $(seq 50); do [ -s "$ROOT/ctl/port" ] && break; sleep 0.1; done
PORT=$(cat "$ROOT/ctl/port")
echo "[]" > "$ROOT/ctl/queue.json"

cat > "$ROOT/run-agy.sh" <<SH
#!/bin/sh
printf '\033[H\033[2J'
cd "$ROOT/ws"
env -i PATH=/usr/bin:/bin:/usr/sbin:/sbin HOME="$ROOT/home" TMPDIR="$ROOT/tmp" \\
  XDG_CONFIG_HOME="$ROOT/home/.config" XDG_CACHE_HOME="$ROOT/home/.cache" \\
  XDG_DATA_HOME="$ROOT/home/.local/share" XDG_STATE_HOME="$ROOT/home/.local/state" \\
  GEMINI_API_KEY=dummy GOOGLE_GEMINI_BASE_URL="http://127.0.0.1:$PORT" \\
  TERM=xterm-256color NO_COLOR=1 LANG=en_US.UTF-8 \\
  /usr/bin/sandbox-exec -f "$ROOT/loopback.sb" "$HT_AGY_BIN" "\$@"
echo "AGY-EXITED rc=\$?"
sleep 600
SH
chmod +x "$ROOT/run-agy.sh"

# set_queue PYTHON_EXPR -> the fake server's reply queue; the expression builds a list
# from text(s, sleep=None), call(name, args), spawn(prompt) and sh(command).
set_queue() {
    python3 - "$ROOT" "$1" > "$ROOT/ctl/queue.json" <<'PY'
import json, sys
root, expr = sys.argv[1], sys.argv[2]
def text(t, sleep=None):
    d = {"text": t}
    if sleep:
        d["sleep"] = sleep
    return d
def call(name, args):
    return {"call": name, "args": args}
def spawn(prompt):
    return call("invoke_subagent", {"Subagents": [{"TypeName": "self", "Role": "Probe Child", "Prompt": prompt, "Model": "inherit"}], "toolSummary": "spawn", "toolAction": "Invoking subagent"})
def sh(cmd):
    return call("run_command", {"CommandLine": cmd, "Cwd": root + "/ws", "WaitMsBeforeAsync": 2000, "toolSummary": "Run " + cmd, "toolAction": "Running"})
# eval is safe here: expr is a literal written in this script, never external input.
print(json.dumps(eval(expr)))
PY
}

screen() { hd agent read "$PANE" --source detection 2>/dev/null; }
screen_has() { screen | grep -qF -- "$1"; }
wait_screen() { # TEXT [SECONDS] -> 0 once the detection text holds TEXT
    local i=0 limit=$((${2:-20} * 5))
    while [ $i -lt $limit ]; do screen_has "$1" && return 0; sleep 0.2; i=$((i + 1)); done
    echo "wait_screen: '$1' not seen" >&2; return 1
}
start_pane() { # LABEL -> a fresh workspace running agy at an empty composer; sets PANE
    hd workspace create --cwd "$ROOT/ws" --label "$1" > "$ROOT/ws-$1.json"
    PANE=$(sed -n 's/.*"root_pane":{[^}]*"pane_id":"\([^"]*\)".*/\1/p' "$ROOT/ws-$1.json")
    hd pane run "$PANE" "PS1='% '; clear" >/dev/null; sleep 1
    hd pane run "$PANE" "exec $ROOT/run-agy.sh" >/dev/null
    local i=0
    while [ $i -lt 150 ]; do # first run only: walk the onboarding screens
        if screen_has "Choose your color scheme"; then hd pane send-keys "$PANE" enter >/dev/null
        elif screen_has "Terms of Service"; then hd pane send-keys "$PANE" down right >/dev/null; sleep 0.5; hd pane send-keys "$PANE" enter >/dev/null
        elif screen_has "for shortcuts"; then sleep 1; return 0; fi
        sleep 0.4; i=$((i + 1))
    done
    echo "start_pane: no composer" >&2; return 1
}
stop_pane() { hd pane close "$PANE" >/dev/null 2>&1; sleep 1; }
snap() { # NAME -> detection text with the root path rewritten to <root>
    screen | sed "s|$ROOT|<root>|g" > "$OUT/$1.read-detection.txt"
    echo "captured $1 ($(wc -l < "$OUT/$1.read-detection.txt") lines)"
}
type_text() { hd pane send-text "$PANE" "$1" >/dev/null; sleep 0.8; }
key() { hd pane send-keys "$PANE" "$@" >/dev/null; sleep 0.8; }
submit() { key enter; }

# 1. States that need no model reply.
start_pane main || exit 1
snap empty
hd agent list | sed "s|$ROOT|<root>|g" > "$OUT/agent-list.txt"
type_text "?"
snap help-overlay
key esc
type_text "hello draft"
snap draft
key ctrl+u; key ctrl+u
type_text "first row"
key shift+enter
type_text "second row"
snap multiline
key ctrl+u; snap multiline-after-ctrl-u-1
key ctrl+u; snap multiline-after-ctrl-u-2
key ctrl+u; key ctrl+u; key ctrl+u

# 2. Busy: the fake server holds the reply for 40 s.
set_queue '[text("late reply", sleep=40)]'
type_text "busy please"; submit
wait_screen "Generating" 15
snap busy
stop_pane

# 3. Permission prompt: a run_command that is not in the allow list.
start_pane perm || exit 1
set_queue '[sh("echo herdr-threads-ok"), text("DONE")]'
type_text "please run the echo"; submit
wait_screen "Run this command" 30
snap permission-prompt
stop_pane

# 4. Subagent states. The parent and child requests race for the next queue entry, so
# each attempt checks the screen and a wrong ordering is retried in a fresh pane.
subagent_state() { # NAME QUEUE_EXPR PROMPT
    local attempt
    for attempt in 1 2 3 4 5 6; do
        start_pane "$1-$attempt" || exit 1
        set_queue "$2"
        type_text "$3"; submit
        if wait_screen "● Agent(" 20; then
            sleep 3
            if screen_has "● Agent("; then snap "$1"; stop_pane; return 0; fi
        fi
        echo "subagent_state $1: attempt $attempt did not reach the state" >&2
        stop_pane
    done
    echo "subagent_state $1: no attempt reached the state" >&2; exit 1
}
subagent_state subagent-generating \
    '[spawn("CHILD-PROMPT say hi"), text("parent done"), text("child late", sleep=60)]' "spawn a child"
subagent_state subagent-tool-running \
    '[spawn("CHILD-PROMPT sleep"), text("parent done"), sh("sleep 90"), text("child end")]' "spawn a sleeping child"

# 5. An Agent(<name>) row outside the live region: a model reply that quotes one.
start_pane old || exit 1
set_queue '[text("● Agent(DeepCoder)  Generating · 12m31s")]'
type_text "quote an agent row"; submit
wait_screen "Generating · 12m31s" 20
sleep 3
snap old-agent-row
stop_pane

echo "capture done"
