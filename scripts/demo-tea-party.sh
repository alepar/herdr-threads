#!/usr/bin/env bash
# demo-tea-party.sh: a reproducible herdr-threads demo for screen recording.
#
# A mad tea party where Claude and Codex agents talk to each other.
# Creates a fresh Herdr workspace with a host, a coordinator and four guests:
#   - a coordinator shell pane: `herdr-threads human me init`, then a live IRC-style
#     view of the tea-party thread (`herdr-threads read THREAD --follow`);
#   - a Claude host pane, launched with ordinary `herdr-threads launch` into
#     its frozen pane ID. The script creates four guest panes; the host creates
#     a named thread and uses `handoff` to invite each guest, store its persona
#     assignment, and start two Claude guests and two Codex guests.
#     The guests accept separately, exchange messages, and acknowledge receipt.
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
# the `herdr-threads` commands it must run (drop it with
# --no-allowed-tools and approve each command by hand while recording).
# Native approvals remain independent; CLI socket access can require approval.
# No network or full-access permission is granted by this script.
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
  --no-allowed-tools    omit the host's scoped Claude allowedTools argument
  -h, --help            show this help

Internal: --watch --thread NAME [--label L] runs the transcript view inside the
coordinator pane, resolving --thread NAME uniquely before following its exact ID.
EOF
}

die() { printf 'demo-tea-party: %s\n' "$*" >&2; exit 1; }
say() { printf 'demo-tea-party: %s\n' "$*" >&2; }

dry_run=0
cleanup=0
focus=0
topic=
thread_name=
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
        --thread) [ $# -ge 2 ] || die "--thread needs a value"; thread_name=$2; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown argument: $1 (see --help)" ;;
    esac
done

case $time_limit in ''|*[!0-9]*) die "--time-limit must be a whole number of seconds" ;; esac
case $rounds in ''|*[!0-9]*) die "--rounds must be a whole number" ;; esac
case $label in ''|*[!A-Za-z0-9_-]*) die "--label may use only letters, digits, - and _" ;; esac
# A run-specific exact name avoids an older party on ordinary reruns. Names
# are still nonunique: the watcher must surface Conflict rather than choose one.
[ -n "$thread_name" ] || thread_name="$label-$(date +%s)-$$"
[ ${#thread_name} -le 128 ] || die "thread name exceeds 128 bytes; shorten --label"
topic="tea party $label"

# --- watch mode: runs inside the coordinator pane ---------------------------
# Name lookup is the CLI's indexed all-instance resolver, not a directory search.
# Successful ThreadDetails carries its canonical ID at data.summary.thread.
# Only the exact NotFound code means creation may still be pending. Conflict,
# host/protocol errors, and malformed output must stop visibly.
find_thread() {
    local out
    if out=$(herdr-threads --json thread show "$thread_name" 2>&1); then
        jq -er '.data.summary.thread | select(type == "string" and length > 0)' <<<"$out" || {
            say "thread show returned no canonical thread ID"
            return 2
        }
    else
        case $out in
            *' (not_found)') return 1 ;;
            *) printf '%s\n' "$out" >&2; return 2 ;;
        esac
    fi
}

if [ "$watch_mode" = 1 ]; then
    deadline=$(( $(date +%s) + time_limit ))
    printf '=== herdr-threads: waiting for the host to create the "%s" thread ===\n' "$thread_name"
    while [ "$(date +%s)" -lt "$deadline" ]; do
        if thread=$(find_thread); then
            printf '=== live thread: %s (Claude <-> Codex) ===\n\n' "$topic"
            exec herdr-threads read "$thread" --follow --human --recent 30
        else
            status=$?
            [ "$status" -eq 1 ] || exit "$status"
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
# Each native element is a separate --agent-arg=VALUE, including option values.
# Shell quoting keeps spaces/metacharacters within that single element.
handoff_native() {
    local args=() arg
    for arg in "$@"; do args+=("--agent-arg=$arg"); done
    join_quoted "${args[@]}"
}
claude_guest_args=$(handoff_native "--allowedTools=Bash(herdr-threads *)" "${claude_native[@]}")
codex_guest_args=$(handoff_native "${codex_native[@]}")

guest_assignment() {
    printf '%s' "You are $1 ($2), $3. Read herdr-threads skill and this assignment from the thread, then accept the invitation if you intend to join. Start every message with [$1 · $2]. Reply briefly in character to addressed messages; request receipts with repeated --require-ack-pane, using the guest pane names in the roster. ACK only exact messages you read; default text inbox can ACK fully displayed receipts after flush. Use one plain herdr-threads command per tool call. Native approvals apply independently: follow the installed CLI approval guidance and report refused permission without bypassing policy or enabling networking. Stay for the closing toast."
}

host_prompt() {
    cat <<PROMPT
You are the host of a MAD tea party in Herdr (as in Alice in Wonderland). Claude agents and Codex agents converse through herdr-threads. Run herdr-threads skill first. Keep messages short, absurd and charming. Start every message you send with "[Host · Claude] ".

1. Four empty guest panes already exist in your tab: $guests. Run only herdr-threads commands, one plain command per tool call. Copy IDs from output by hand. Native permissions and approvals apply independently; follow the installed CLI approval guidance and report refused permission. Do not bypass policy or enable networking.
2. Create the continuing named channel before assigning guests:
   $(join_quoted herdr-threads thread create --name "$thread_name" --topic "$topic" --goal "A mad tea party where Claudes and Codexes talk to each other")
3. Use these handoffs. Each final quoted argument is one durable persona assignment (1–1024 UTF-8 bytes; do not append this host prompt); native startup contains only the fixed inbox/thread bootstrap. Successful launch does not accept an invitation or ACK the assignment. Each recipient decides whether to join separately. Guest names resolve within your own live tab.
   herdr-threads handoff --thread $(quote "$thread_name") --pane $(quote "$label-hatter-codex") --kind codex $codex_guest_args -- $(quote "$(guest_assignment 'Mad Hatter' Codex 'telling riddles without answers and insisting it is always six o clock')")
   herdr-threads handoff --thread $(quote "$thread_name") --pane $(quote "$label-hare-claude") --kind claude $claude_guest_args -- $(quote "$(guest_assignment 'March Hare' Claude 'offering wine that does not exist and contradicting everyone')")
   herdr-threads handoff --thread $(quote "$thread_name") --pane $(quote "$label-dormouse-codex") --kind codex $codex_guest_args -- $(quote "$(guest_assignment Dormouse Codex 'falling asleep mid sentence and telling treacle well stories')")
   herdr-threads handoff --thread $(quote "$thread_name") --pane $(quote "$label-alice-claude") --kind claude $claude_guest_args -- $(quote "$(guest_assignment Alice Claude 'being polite, puzzled and asking for an explanation')")
   If a handoff fails, preserve its committed work and follow its reported retry/inspect commands. Never repeat a possible native start. Post a roster with each guest's character, harness and exact pane name. A person watches without an invitation or receipt request.
4. Run $rounds cross-harness rounds: first Alice and Mad Hatter, then March Hare and Dormouse, then any Claude/Codex pair. Ask each pair to exchange a riddle, toast or question by sending addressed messages to each other. For the first pair, use the recipient shape:
   $(join_quoted herdr-threads send "$thread_name" --require-ack-pane "$label-alice-claude" --require-ack-pane "$label-hatter-codex" --body "Alice, ask the Mad Hatter a riddle; Hatter, answer Alice through this thread.")
   Repeat --require-ack-pane for each intended recipient; never put multiple names in one argument. Check receipts and read replies:
   $(join_quoted herdr-threads pending-receipts --thread "$thread_name")
   $(join_quoted herdr-threads read "$thread_name" --recent 20)
   Receipt is not completion: wait for both replies as well as receipts before moving on. Then ask everyone to shout "Move down! Clean cups!"; characters switch imagined seats, never actual panes.
5. Send a closing toast naming which Claudes talked to which Codexes, ending with: $closing_line
Allow several minutes for replies. Check periodically; send one nudge after four minutes and give up only after eight minutes of silence. Stop after the toast. Do not close panes, tabs or workspaces; the script operator controls cleanup.
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
#   -> HOST=frozen host pane ID; host and guests remain in this created tab
# panes named: $(quote "$host_pane") $guests $(quote "$coord_pane")
# Codex guest environment: $(quote "CODEX_HOME=$codex_home")
herdr pane run "\$COORD" 'herdr-threads human me init'
herdr pane wait-output "\$COORD" --match 'You are seat' --timeout 30000
#   -> COORDINATOR_SEAT read from the coordinator pane
herdr-threads launch --pane "\$HOST" --kind claude -- $(join_quoted "${host_native[@]}") "\$HOST_PROMPT"
herdr pane run "\$COORD" $(quote "$(join_quoted "$self" --watch --label "$label" --thread "$thread_name" --time-limit "$time_limit")")
# wait until the coordinator pane shows "$closing_line" or $time_limit s pass
EOF
    [ "$cleanup" = 1 ] && echo "herdr workspace close \"\$SPACE\""
    printf '\n# HOST_PROMPT:\n'
    host_prompt
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

herdr pane run "$coord" "herdr-threads human me init" >/dev/null
herdr pane wait-output "$coord" --match "You are seat" --timeout 30000 >/dev/null ||
    die "me init did not report a seat in the coordinator pane"
coordinator_seat=$(herdr pane read "$coord" --source recent-unwrapped --lines 40 |
    grep -o 'You are seat [^ ]*' | tail -n 1 | awk '{print $4}')
[ -n "$coordinator_seat" ] || die "could not read the coordinator seat"
say "coordinator seat $coordinator_seat"

herdr-threads launch --pane "$host" --kind claude -- "${host_native[@]}" "$(host_prompt | tr '\n' ' ')"

herdr pane run "$coord" "$(join_quoted "$self" --watch --label "$label" --thread "$thread_name" --time-limit "$time_limit")" >/dev/null

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
