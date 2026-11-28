#!/usr/bin/env bash
# Model-free capture of agy screens with a user-configured statusline (ht-rot9), which
# replaces agy's default footer (`? for shortcuts …` / `esc to cancel …`) below the
# composer. Same setup as capture.sh (private Herdr session and HOME, the fake Gemini
# server answering from a scripted queue); settings add a `statusLine` command that
# prints one fixed bar. agy runs under the loopback sandbox where `sandbox-exec` exists
# (macOS), else with only its isolated HOME and the loopback base URL.
#
# Usage: HT_AGY_BIN=/path/to/agy tests/fixtures/agy/composer/capture-statusline.sh [OUT_DIR]
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
ROOT=$(cd "$ROOT" && pwd -P)
ih_start "$ROOT" statusline-capture || exit 1
mkdir -p "$ROOT/tmp" "$ROOT/ctl" "$ROOT/ws"
hd() { ( IFS='
'; exec env $(ih_env_args "$ROOT") "$IH_HERDR" "$@" ); }

SANDBOX=
if [ -x /usr/bin/sandbox-exec ]; then
    sed -e "s|<REAL_HOME>|$REAL_HOME|g" -e "s|<ROOT>|$ROOT|g" \
        "$REPO/scripts/agy-sandbox/loopback.sb.in" > "$ROOT/loopback.sb"
    SANDBOX="/usr/bin/sandbox-exec -f $ROOT/loopback.sb"
fi

# A statusline shaped like a real user's (a context bar and | fields); it ignores stdin.
cat > "$ROOT/statusline.sh" <<'SH'
#!/bin/sh
cat >/dev/null
printf '%s\n' '████░░░░░░░░░░░░░░░░ 18%/1m | Fake Model (Medium) | wk:100%/100% | default | !sbx'
SH
chmod +x "$ROOT/statusline.sh"

mkdir -p "$ROOT/home/.gemini/antigravity-cli"
cat > "$ROOT/home/.gemini/antigravity-cli/settings.json" <<JSON
{
  "altScreenMode": "never",
  "modelProvider": "gemini",
  "statusLine": {"type": "command", "command": "sh $ROOT/statusline.sh"},
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
  $SANDBOX "$HT_AGY_BIN" "\$@"
echo "AGY-EXITED rc=\$?"
sleep 600
SH
chmod +x "$ROOT/run-agy.sh"

set_queue() { # JSON -> the fake server's reply queue
    printf '%s\n' "$1" > "$ROOT/ctl/queue.json"
}
screen() { hd agent read "$PANE" --source detection 2>/dev/null; }
screen_has() { screen | grep -qF -- "$1"; }
wait_screen() { # TEXT [SECONDS] -> 0 once the detection text holds TEXT
    local i=0 limit=$((${2:-20} * 5))
    while [ $i -lt $limit ]; do screen_has "$1" && return 0; sleep 0.2; i=$((i + 1)); done
    echo "wait_screen: '$1' not seen" >&2; return 1
}
snap() { # NAME -> detection text with the root path rewritten to <root>
    screen | sed "s|$ROOT|<root>|g" > "$OUT/$1.read-detection.txt"
    echo "captured $1 ($(wc -l < "$OUT/$1.read-detection.txt") lines)"
}
type_text() { hd pane send-text "$PANE" "$1" >/dev/null; sleep 0.8; }
key() { hd pane send-keys "$PANE" "$@" >/dev/null; sleep 0.8; }

hd workspace create --cwd "$ROOT/ws" --label statusline > "$ROOT/ws.json"
PANE=$(sed -n 's/.*"root_pane":{[^}]*"pane_id":"\([^"]*\)".*/\1/p' "$ROOT/ws.json")
hd pane run "$PANE" "PS1='% '; clear" >/dev/null; sleep 1
hd pane run "$PANE" "exec $ROOT/run-agy.sh" >/dev/null
i=0
while [ $i -lt 150 ]; do # first run only: walk the onboarding screens
    if screen_has "Choose your color scheme"; then hd pane send-keys "$PANE" enter >/dev/null
    elif screen_has "Terms of Service"; then hd pane send-keys "$PANE" down right >/dev/null; sleep 0.5; hd pane send-keys "$PANE" enter >/dev/null
    elif screen_has "Fake Model (Medium)"; then sleep 1; break; fi
    sleep 0.4; i=$((i + 1))
done
[ $i -lt 150 ] || { echo "no composer"; screen; exit 1; }

snap statusline-empty
type_text "?"
snap statusline-help-overlay
key esc
type_text "hello draft"
snap statusline-draft
key ctrl+u; key ctrl+u
set_queue '[{"text": "late reply", "sleep": 40}]'
type_text "busy please"; key enter
wait_screen "Generating" 15
snap statusline-busy
hd pane close "$PANE" >/dev/null 2>&1
echo "capture done"
