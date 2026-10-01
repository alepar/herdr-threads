#!/usr/bin/env bash
# demo-tea-party.sh: a reproducible herdr-threads demo for screen recording.
#
# A mad tea party where Claude and Codex agents talk to each other.
# Creates a fresh Herdr workspace with two panes:
#   - a coordinator shell pane: `herdr-threads me init`, then a live IRC-style
#     view of the tea-party thread (`herdr-threads read THREAD --follow`);
#   - a host pane, where a Claude is launched with `herdr-threads launch` and
#     told to host a tea party: it creates panes with `herdr pane split`,
#     launches two Claude guests and two Codex guests into them with
#     `herdr-threads launch`, creates a "tea party" thread, invites everyone
#     (and the coordinator, as an observer) and runs a few rounds of chat in
#     which each guest replies in character and ACKs, then wraps up.
#
# It waits until the host posts the closing line or the time limit passes,
# then (with --cleanup) closes the workspace, which also ends every agent in it.
# Without --cleanup the agents keep running after the time limit.
#
# Requirements (nothing here installs or configures them):
#   - run inside a Herdr pane (HERDR_ENV=1) with `herdr`, `herdr-threads` and
#     `jq` on PATH, and the herdr-threads daemon running;
#   - the user-level setup done: `herdr-threads setup` for claude and codex,
#     and the Codex hooks trusted once interactively;
#   - --cwd (default: the current directory) already trusted by Claude Code,
#     so no folder-trust dialog blocks the launches.
#
# Sandboxes and approvals are never bypassed. The only permission the host
# Claude gets beyond the user's settings is a scoped --allowedTools list for
# the `herdr pane` / `herdr-threads` commands it must run (drop it with
# --no-allowed-tools and approve each command by hand while recording).
# Codex guests run with their normal sandbox and approval policy.
#
# Usage:
#   scripts/demo-tea-party.sh [--dry-run] [--cleanup] [options]
#   scripts/demo-tea-party.sh --help

set -euo pipefail

usage() {
    cat <<'EOF'
Usage: demo-tea-party.sh [options]

  --dry-run             print the planned commands and the host prompt; run nothing
  --cleanup             close the demo workspace when the party ends or times out
  --focus               focus the new demo workspace (default: create it without taking focus)
  --codex-home DIR      CODEX_HOME for the Codex guest panes (for example an aisw profile directory);
                        default: whatever the new panes' shells set
  --time-limit SECONDS  stop waiting after this long (default 900)
  --rounds N            chat rounds the host runs (default 3)
  --claude-model NAME   model for every Claude, host and guests (default sonnet)
  --claude-arg ARG      extra native argument for every Claude (repeatable)
  --codex-model NAME    model for the Codex guests (default: Codex's default)
  --codex-effort LEVEL  model_reasoning_effort for the Codex guests (default low)
  --codex-profile NAME  Codex config profile (-p) for the Codex guests
  --cwd DIR             working directory of the new tab (default: current)
  --label TEXT          tab label and pane-name prefix (default tea-HHMMSS)
  --no-allowed-tools    do not pre-allow the host's herdr commands
  -h, --help            show this help

Internal: --watch [--label L] [--seat SEAT] runs the transcript view inside the
coordinator pane, following the thread whose topic is "tea party L".
EOF
}

die() { printf 'demo-tea-party: %s\n' "$*" >&2; exit 1; }
say() { printf 'demo-tea-party: %s\n' "$*" >&2; }

dry_run=0
cleanup=0
focus=0
topic=
codex_home=
time_limit=900
rounds=3
claude_model=sonnet
claude_args=()
codex_model=
codex_effort=low
codex_profile=
cwd=$PWD
label="tea-$(date +%H%M%S)"
allowed_tools=1
watch_mode=0
watch_seat=
closing_line="THE TEA PARTY IS OVER"

while [ $# -gt 0 ]; do
    case $1 in
        --dry-run) dry_run=1; shift ;;
        --cleanup) cleanup=1; shift ;;
        --focus) focus=1; shift ;;
        --codex-home) [ $# -ge 2 ] || die "--codex-home needs a value"; codex_home=$2; shift 2 ;;
        --time-limit) [ $# -ge 2 ] || die "--time-limit needs a value"; time_limit=$2; shift 2 ;;
        --rounds) [ $# -ge 2 ] || die "--rounds needs a value"; rounds=$2; shift 2 ;;
        --claude-model) [ $# -ge 2 ] || die "--claude-model needs a value"; claude_model=$2; shift 2 ;;
        --claude-arg) [ $# -ge 2 ] || die "--claude-arg needs a value"; claude_args+=("$2"); shift 2 ;;
        --codex-model) [ $# -ge 2 ] || die "--codex-model needs a value"; codex_model=$2; shift 2 ;;
        --codex-effort) [ $# -ge 2 ] || die "--codex-effort needs a value"; codex_effort=$2; shift 2 ;;
        --codex-profile) [ $# -ge 2 ] || die "--codex-profile needs a value"; codex_profile=$2; shift 2 ;;
        --cwd) [ $# -ge 2 ] || die "--cwd needs a value"; cwd=$2; shift 2 ;;
        --label) [ $# -ge 2 ] || die "--label needs a value"; label=$2; shift 2 ;;
        --no-allowed-tools) allowed_tools=0; shift ;;
        --watch) watch_mode=1; shift ;;
        --topic) [ $# -ge 2 ] || die "--topic needs a value"; topic=$2; shift 2 ;;
        --seat) [ $# -ge 2 ] || die "--seat needs a value"; watch_seat=$2; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown argument: $1 (see --help)" ;;
    esac
done

case $time_limit in ''|*[!0-9]*) die "--time-limit must be a whole number of seconds" ;; esac
case $rounds in ''|*[!0-9]*) die "--rounds must be a whole number" ;; esac
case $label in ''|*[!A-Za-z0-9_-]*) die "--label may use only letters, digits, - and _" ;; esac
# The topic is unique per run (label plus start time), so a re-run with the
# same --label never follows an earlier party's thread. --watch is handed the
# exact topic by the launcher.
[ -n "$topic" ] || topic="tea party $label $(date +%H%M%S)"

# --- watch mode: runs inside the coordinator pane ---------------------------
# Waits for the host to create the thread, then hands the pane to
# `herdr-threads read --follow`: an IRC-style live view that prints the recent
# messages and then each new one once, never clearing the screen. The
# coordinator seat only reads; the follower never ACKs or accepts. Ctrl-C
# stops it.
# find_thread prints the ID of the thread whose topic is exactly $topic,
# following every directory page. With --seat it lists only threads that seat
# joined or was invited to.
find_thread() {
    local selectors=(--search "$topic") cursor='' out id _
    if [ -n "$watch_seat" ]; then selectors+=(--seat "$watch_seat"); else selectors+=(--all); fi
    for _ in $(seq 1 200); do
        if [ -n "$cursor" ]; then
            out=$(herdr-threads --json thread list "${selectors[@]}" --cursor "$cursor" 2>/dev/null) || return 1
        else
            out=$(herdr-threads --json thread list "${selectors[@]}" 2>/dev/null) || return 1
        fi
        id=$(jq -r --arg t "$topic" 'first(.. | objects | select(.topic_data? == $t) | .thread) // empty' <<<"$out") || return 1
        [ -n "$id" ] && { printf '%s\n' "$id"; return 0; }
        cursor=$(jq -r 'first(.. | objects | select(has("items") and has("next_cursor")) | .next_cursor) // empty' <<<"$out") || return 1
        [ -n "$cursor" ] || return 1
    done
    return 1
}

if [ "$watch_mode" = 1 ]; then
    deadline=$(( $(date +%s) + time_limit ))
    printf '=== herdr-threads: waiting for the host to create the "%s" thread ===\n' "$topic"
    while [ "$(date +%s)" -lt "$deadline" ]; do
        if thread=$(find_thread) && [ -n "$thread" ]; then
            printf '=== live thread: %s (Claude <-> Codex) ===\n\n' "$topic"
            exec herdr-threads read "$thread" --follow --human --recent 30
        fi
        sleep 2
    done
    printf '\n(time limit reached before the thread was created)\n'
    exit 0
fi

# --- plan ---------------------------------------------------------------------
self=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")
host_pane="$label-host"
coord_pane="$label-you"
guests="$label-hatter-codex $label-hare-claude $label-dormouse-codex $label-alice-claude"

quote() { printf '%q' "$1"; }
join_quoted() { local out='' a; for a in "$@"; do out+=" $(quote "$a")"; done; printf '%s' "${out# }"; }

claude_native=(--model "$claude_model")
[ ${#claude_args[@]} -gt 0 ] && claude_native+=("${claude_args[@]}")
codex_native=(-c "model_reasoning_effort=\"$codex_effort\"")
[ -n "$codex_model" ] && codex_native+=(-m "$codex_model")
[ -n "$codex_profile" ] && codex_native+=(-p "$codex_profile")
claude_guest_args=$(join_quoted "--allowedTools=Bash(herdr-threads *)" "${claude_native[@]}")
codex_guest_args=$(join_quoted "${codex_native[@]}")

host_prompt() {
    local coordinator_seat=$1
    cat <<PROMPT
You are the host of a MAD tea party in Herdr (as in Alice in Wonderland), and the point of the show is that Claude agents and Codex agents talk to each other through herdr-threads. Use herdr-threads for all conversation (run \`herdr-threads skill\` first if you have not read it). Keep every message short, absurd and charming. Start every message you send with "[Host · Claude] ".

1. The four guest panes already exist and are named: $guests. Run ONLY herdr-threads commands (no other shell commands, no herdr pane commands), exactly one plain herdr-threads command per tool call: never use pipes, &&, ;, variables, or \$(...) command substitution, and never prefix commands with cd or test. Copy IDs from earlier output by hand. Plain herdr-threads commands are pre-approved, so nothing will ask for permission.
2. Launch the guests (two Codex, two Claude), each with a first prompt that gives its persona and these rules: answer herdr-threads messages in character; start EVERY message with its signature "[NAME · HARNESS] " (for example "[Mad Hatter · Codex] "); when asked to talk to another guest, send that guest a message with \`--require-ack\` naming the other guest's seat; ACK every message addressed to it; never leave early; run ONLY plain herdr-threads commands, one per call, with no pipes, &&, variables or \$(...) (they are pre-approved; never run other shell commands). Codex guests: herdr-threads commands work inside your normal sandbox, so never request escalated permissions for them.
   - (Each PERSONA PROMPT must be a single line: no line breaks.)
   - \`herdr-threads launch --pane $label-hatter-codex --kind codex -- $codex_guest_args "PERSONA PROMPT"\`: the Mad Hatter (Codex), riddles without answers, insists it is always six o'clock;
   - \`herdr-threads launch --pane $label-hare-claude --kind claude -- $claude_guest_args "PERSONA PROMPT"\`: the March Hare (Claude), offers wine that does not exist, contradicts everyone;
   - \`herdr-threads launch --pane $label-dormouse-codex --kind codex -- $codex_guest_args "PERSONA PROMPT"\`: the Dormouse (Codex), falls asleep mid-sentence, tells treacle-well stories;
   - \`herdr-threads launch --pane $label-alice-claude --kind claude -- $claude_guest_args "PERSONA PROMPT"\`: Alice (Claude), polite, puzzled, keeps asking for an explanation.
   Note the seat each launch prints.
3. Create the thread: \`herdr-threads thread create --topic "$topic" --goal "A mad tea party where Claudes and Codexes talk to each other"\`. Invite the four guest seats (a person is watching the thread live; do not invite or address them). Then post a roster message listing every guest as NAME · HARNESS · SEAT, so guests can address each other.
4. Run $rounds rounds. Every round is a cross-harness exchange: pair a Claude guest with a Codex guest (round 1: Alice (Claude) and the Mad Hatter (Codex); round 2: the March Hare (Claude) and the Dormouse (Codex); then any Claude/Codex pairing). Send one prompt to the thread with \`--require-ack\` naming both paired seats, telling the Claude guest to ask the Codex guest something (a riddle, a toast, a question about the time) by sending it a message, and the Codex guest to answer the Claude guest the same way. Wait until both have ACKed (\`herdr-threads pending-receipts --thread THREAD\`) and both have replied to each other, reading \`herdr-threads read THREAD --recent 20\`. Then have everyone shout "Move down! Clean cups!" and switch seats.
5. Wrap up: send a short closing toast that names which Claudes talked to which Codexes, ending with the exact line: $closing_line
Codex guests are slower than Claude guests (several minutes per reply is normal): before moving on or closing, wait until every paired guest has replied, checking every 30 seconds; send one short nudge after 4 minutes of silence and only give up on a guest after 8 minutes. Never close the party while a reply you asked for is still outstanding. Never ask the person watching any question and never wait for their input. Then stop. Do not close panes or tabs; the person running the demo does that.
PROMPT
}

# Claude's --allowedTools is variadic (`--allowedTools <tools...>`): every
# following bare word is read as another tool, so a prompt placed right after
# the list would be swallowed. Pass the list as one `=`-joined value and put it
# first, so --model (and any --claude-arg) sits between it and the prompt.
host_native=()
if [ "$allowed_tools" = 1 ]; then
    host_native+=("--allowedTools=Bash(herdr-threads *),Bash(herdr-threads:*)")
fi
host_native+=("${claude_native[@]}")

focus_flag() { if [ "$focus" = 1 ]; then echo --focus; else echo --no-focus; fi; }
if [ "$dry_run" = 1 ]; then
    cat <<EOF
# demo-tea-party.sh --dry-run: planned commands (nothing is run)
herdr workspace create --label $(quote "$label") --cwd $(quote "$cwd") $(focus_flag)
#   -> SPACE=.result.workspace.workspace_id  COORD=.result.root_pane.pane_id
# split the root into: host (left, large), a 2x2 guest grid (right) and a short coordinator strip (bottom)
# panes named: $(quote "$host_pane") $guests $(quote "$coord_pane")
herdr pane run "\$COORD" 'herdr-threads me init'
herdr pane wait-output "\$COORD" --match 'You are seat' --timeout 30000
#   -> COORDINATOR_SEAT read from the coordinator pane
herdr-threads launch --pane $(quote "$host_pane") --kind claude -- $(join_quoted "${host_native[@]}") "\$HOST_PROMPT"
herdr pane run "\$COORD" $(quote "$(quote "$self") --watch --label $label --seat COORDINATOR_SEAT --time-limit $time_limit")
# wait until the coordinator pane shows "$closing_line" or $time_limit s pass
EOF
    [ "$cleanup" = 1 ] && echo "herdr workspace close \"\$SPACE\""
    printf '\n# HOST_PROMPT:\n'
    host_prompt "COORDINATOR_SEAT"
    exit 0
fi

# --- preflight ----------------------------------------------------------------
[ "${HERDR_ENV:-}" = 1 ] || die "run this inside a Herdr pane (HERDR_ENV=1)"
for tool in herdr herdr-threads jq; do
    command -v "$tool" >/dev/null 2>&1 || die "$tool not found on PATH"
done
[ -d "$cwd" ] || die "--cwd is not a directory: $cwd"
herdr-threads setup-status >/dev/null 2>&1 ||
    say "warning: herdr-threads setup-status reports a problem; run 'herdr-threads setup-status' and 'herdr-threads setup' first"

focus_flag() { if [ "$focus" = 1 ]; then echo --focus; else echo --no-focus; fi; }
json_field() { jq -er "$1" <<<"$2" || die "unexpected Herdr response (no $1): $2"; }

# --- run ----------------------------------------------------------------------
# A separate workspace keeps the party out of the user's own tabs.
created=$(herdr workspace create --label "$label" --cwd "$cwd" "$(focus_flag)")
tab=$(json_field '.result.workspace.workspace_id' "$created")
coord=$(json_field '.result.root_pane.pane_id' "$created")
say "workspace $tab"

close_tab() {
    if [ "$cleanup" = 1 ]; then
        say "closing workspace $tab"
        herdr workspace close "$tab" >/dev/null 2>&1 || say "warning: could not close workspace $tab"
    else
        say "leaving workspace $tab open (pass --cleanup to close it)"
    fi
}
trap close_tab EXIT

# Layout (the script makes every pane, so the host needs no herdr permissions):
#   +--------------+--------+--------+
#   |              | hatter | hare   |
#   |    host      +--------+--------+
#   |              | dormse | alice  |
#   +--------------+--------+--------+
#   |  you: live transcript (short)  |
#   +--------------------------------+
split_pane() {
    local env_args=()
    [ -n "${4:-}" ] && env_args=(--env "$4")
    json_field '.result.pane.pane_id' "$(herdr pane split "$1" --direction "$2" --ratio "$3" --cwd "$cwd" --no-focus ${env_args[@]+"${env_args[@]}"})"
}
codex_env=
[ -n "$codex_home" ] && codex_env="CODEX_HOME=$codex_home"
host=$coord
coord=$(split_pane "$host" down 0.72)
# Guests: hatter (Codex) top-left, hare (Claude) top-right, dormouse (Codex)
# bottom-left, alice (Claude) bottom-right; Codex panes get --codex-home.
grid=$(split_pane "$host" right 0.42 "$codex_env")
grid_low=$(split_pane "$grid" down 0.5 "$codex_env")
read -r -a guest_names <<<"$guests"
herdr pane rename "$grid" "${guest_names[0]}" >/dev/null
herdr pane rename "$(split_pane "$grid" right 0.5)" "${guest_names[1]}" >/dev/null
herdr pane rename "$grid_low" "${guest_names[2]}" >/dev/null
herdr pane rename "$(split_pane "$grid_low" right 0.5)" "${guest_names[3]}" >/dev/null
herdr pane rename "$host" "$host_pane" >/dev/null
herdr pane rename "$coord" "$coord_pane" >/dev/null
say "host pane $host, coordinator pane $coord"

herdr pane run "$coord" "herdr-threads me init" >/dev/null
herdr pane wait-output "$coord" --match "You are seat" --timeout 30000 >/dev/null ||
    die "me init did not report a seat in the coordinator pane"
coordinator_seat=$(herdr pane read "$coord" --source recent-unwrapped --lines 40 |
    grep -o 'You are seat [^ ]*' | tail -n 1 | awk '{print $4}')
[ -n "$coordinator_seat" ] || die "could not read the coordinator seat"
say "coordinator seat $coordinator_seat"

herdr-threads launch --pane "$host_pane" --kind claude -- "${host_native[@]}" "$(host_prompt "$coordinator_seat" | tr '\n' ' ')"

herdr pane run "$coord" "$(quote "$self") --watch --label $label --topic $(quote "$topic") --time-limit $time_limit" >/dev/null

deadline=$(( $(date +%s) + time_limit ))
while [ "$(date +%s)" -lt "$deadline" ]; do
    # The live view wraps long messages, so join lines before matching.
    if herdr pane read "$coord" --source recent-unwrapped --lines 200 2>/dev/null |
        tr '\n' ' ' | tr -s ' ' | grep -q "$closing_line"; then
        say "the host closed the party"
        exit 0
    fi
    sleep 5
done
say "time limit of ${time_limit}s reached"
