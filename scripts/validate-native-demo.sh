#!/bin/sh
# Native herdr-threads demonstration driver (ht-4is.11.3 Codex / ht-4is.11.4 Claude / ht-910).
# Thin wrapper; all logic and option help: scripts/validate-native-demo.py --help
#
#   scripts/validate-native-demo.sh --harness claude --dry-run --bin target/debug/herdr-threads
#   scripts/validate-native-demo.sh --harness claude --bin bin/herdr-threads            (print mode, $ cap per launch)
#   scripts/validate-native-demo.sh --harness claude --mode tui --tui-accept-trust --allow-uncapped-spend --bin ...
#   scripts/validate-native-demo.sh --harness codex --allow-uncapped-spend --bin bin/herdr-threads
#   scripts/validate-native-demo.sh --harness codex --allow-uncapped-spend --codex-profile auto --bin ...
#   Setup is user level, so it runs with HOME, CLAUDE_CONFIG_DIR and CODEX_HOME in the run root (scratch copies).
#   Claude launches keep the real config dir (auth) plus `--settings <run>/claude-config/settings.json`; Codex
#   launches use CODEX_HOME=<run>/codex-home (setup's hooks.json/config.toml, auth.json symlinked from the profile).
#   --codex-profile NAME uses ~/.aisw/profiles/codex/NAME's credentials for the Codex launch (never written);
#   `auto` tries codex-1, codex-2, codex-3, default, moving on only when Codex reports a usage limit.
#   A usage-limit/auth failure is ENVIRONMENT: manifest UNSUPPORTED environment_<kind>, never a product FAIL.
#   Claude: S16R fails before launch unless every ready command is permitted (setup must install Bash(herdr-threads *)).
#   --cli-hint adds a one-line CLI hint to the scratch project: DIAGNOSTIC ONLY, manifest never PASS.
#   Any subagent/collab activity in the root transcript makes child-ACK absence UNVERIFIED and the manifest
#   UNSUPPORTED (child_ack_unverified): cooperative ACKs cannot tell a child caller from the root. Only under
#   --scenario child does a complete sidechain with no successful child write give a transcript-level PASS.
#   The manifest reason names the most serious UNVERIFIED item: child_ack_unverified, warning_wake_unverified
#   (SW2), hook_context_unverified (S<n>H), else evidence_unverified.
#   Hooks come from the public `herdr-threads setup` CLI when present (setup_mode=cli), else driver_fallback.
#
# Opt-in scenarios (--scenario a,b,...; each adds its own verdict steps, dry-run probes and manifest suffix):
#   child     prompt delegates reading to one subagent; SC1 child read allowed, SC2 no successful child herdr-threads
#             write (complete sidechain: Claude subagent events, Codex child rollouts found by session_meta parent;
#             the DB ACK execution = root binding check is consistency only), SC3 top-level ACK present
#   midturn   a new require-ACK after the model's first tool call; SM1 sent, SM2 PreToolUse context delivered it
#             (transcript attachment, else model behaviour, else UNVERIFIED), SM3 model ACKed it
#   warning   short initial ACK deadline (--warning-deadline); SW1 exactly one durable warning, SW2 one coalesced wake
#             (NOT_EXERCISED when a check-in already offered the warning). Claude --mode tui: the handoff keeps
#             --deadline; SW0 sends a second short-deadline message once the agent is idle and polls for the wake.
#   Scenario steps lead the handoff body (inside the 256-byte preview); a midturn/burst/required judge whose
#   instruction the model never saw (no `body` read, no instruction text in the transcript) is NOT_EXERCISED.
#   burst     --burst-threads (>20) extra invited threads; SB1 model saw has_more, SB2 model used --cursor
#   required  D2 service wire: managed thread + required invitation; SR1 model accept-required, SR2 leave refused
#             with membership held, SR3 service events carry no receipt rows (ht-4is.32.3)
#   servicesend  a v2 service session sends an ACK-required request to the agent seat (SS0 setup); SS1 the model ACKed it by
#             a root `ack` call with stored provenance cooperative_top_level, SS2 the request is a programmatic message with
#             no author receipt row (ht-5nb.4)
#   lostprompt (--mode tui) the agent starts with NO prompt (startup check-in only); SL1 the product reserved an idle
#             recovery wake covering the pending handoff, SL2 the wake marker reached the screen once, S18 the model ACKed
#   blockedui (--mode tui, runs last) the agent is asked to run `mkdir /ht-blockedui-*` and left in its approval UI
#             (never answered; cleanup closes the tab); SU1 a short-deadline require-ACK is queued, SU2 no marker, no
#             `submitted` wake outcome and no ACK while blocked (the wake stays pending)
#   children  the prompt asks for two concurrent reading subagents; SK1 concurrency (Claude spawn/result order, Codex
#             child rollout spans) and two child reads, SK2 no successful child write, SK3 root ACK
#   Wake verdicts are judged from wake_work rows (read-only SQLite) and pane captures. When `daemon health` reports
#   host safe_prompt unsupported and no delivered wake was observed they are NOT_EXERCISED (manifest UNSUPPORTED
#   safe_wake_unsupported), never PASS and never a product FAIL. Herdr's `done` state counts as idle.
#   A scenario whose trigger never happened is NOT_EXERCISED -> manifest UNSUPPORTED scenario_not_exercised.
#   scripts/validate-native-demo.sh --harness claude --dry-run --scenario child,midturn,warning,burst,required --bin ...
#   --launch managed uses `herdr-threads launch` when the build has it, else UNSUPPORTED managed_launch_unavailable.
#   --mode tui --tui-accept-trust (approved for /private/tmp scratch projects only) records ~/.claude.json before and
#   after (hash + this project's entry, read-only) and supports --phases initial,clear.
#   --harness codex --mode tui: interactive Codex in the pane with the hook overrides and socket allowance `setup codex`
#   printed, explicit -s/-a on-request and --dangerously-bypass-hook-trust (scratch under /private/tmp only; no
#   approval/sandbox bypass). Codex 0.159.3+ shows its folder-trust screen anyway: with --tui-accept-trust the driver
#   answers it, only when the project is inside the run root under /private/tmp, and records the acceptance in
#   summary.json (codex_tui). Any other trust/approval screen at startup is a FAIL with nothing typed. --phases initial,clear uses /new; restart /quit + relaunch; resume `codex resume <thread>`.
#   scripts/validate-native-demo.sh --harness codex --mode tui --allow-uncapped-spend --phases initial,clear --scenario warning --bin ...
#
# Cell entry points (ht-p03.38; ht-p03.20 reuses them): `validate-native-demo.sh --cell list` prints the names;
# `--cell NAME [driver args]` runs that native-matrix cell once inside its own isolated named Herdr session (private
# server under /tmp/ih.XXXXXX via scripts/lib/isolated-herdr.sh, the real HOME kept so the harness can read its
# credentials; the shared Herdr server is never touched) and prints one `CELL ...` record line. Claude cells run the
# Claude Code $HT_CLAUDE_VERSION (default 2.1.287) resolved once from ~/.local/share/claude/versions/<v> and exposed as
# `claude` on a run-root PATH entry ahead of ~/.local/bin (DISABLE_AUTOUPDATER=1 is exported; it does not stop another
# Claude Code process repointing the shared launcher, hence the fixed versioned path). Codex cells run the fixed Codex
# 0.159.3 install ($HT_CODEX_BIN overrides; the `codex` on PATH auto-updates) via --codex-bin, and the Codex TUI cells
# accept the folder-trust screen for their /private/tmp scratch run root only.
# `--matrix [CELL...]` (ht-p03.20) runs the cells (default: all) one after another, each up to $HT_CELL_ATTEMPTS (3)
# times on the same SHA, and prints one `MATRIX ...` row per cell: sha, harness version before/after and the version
# the agent itself reported in its transcript/screen (`harness_ran`; any mismatch, a missing report, or a version other
# than the pinned one invalidates the attempt), attempts, outcome (PASS, PASS (flaky), FAIL,
# NOT_EXERCISED), manifest evidence dir; $HT_MATRIX_LOG, when set, gets a copy of every ATTEMPT and MATRIX line
# (`scripts/reconcile-validation.py --collect-native-rerun` reads it). Exit 0 always; the rows are the result.
#
# Must run inside a Herdr pane (HERDR_ENV=1), except under --cell, which makes its own. Creates one owned scratch tab and closes it;
# stops only a daemon it started; never edits user-global Claude/Codex/aisw configuration.
set -eu
script_dir=$(CDPATH='' cd "$(dirname "$0")" && pwd -P)
umask 077

CELLS="codex-manual codex-managed claude-manual claude-managed codex-no-initial-prompt claude-no-initial-prompt
children-claude children-codex sw2-claude sw2-codex codex-tui-children-write-absence ht910-claude ht910-codex
p40-crash-fix3 codex-sandbox-xdg-state wave28-claude-wake-submission"

# NAME -> driver flags (one word per line is not needed: no flag value contains whitespace).
cell_flags() {
    case "$1" in
    codex-manual) echo "--harness codex --allow-uncapped-spend --phases initial" ;;
    codex-managed) echo "--harness codex --allow-uncapped-spend --launch managed --phases initial" ;;
    claude-manual) echo "--harness claude --phases initial" ;;
    claude-managed) echo "--harness claude --launch managed --phases initial" ;;
    codex-no-initial-prompt) echo "--harness codex --mode tui --tui-accept-trust --allow-uncapped-spend --scenario lostprompt --phases initial" ;;
    claude-no-initial-prompt) echo "--harness claude --mode tui --tui-accept-trust --allow-uncapped-spend --scenario lostprompt --phases initial" ;;
    children-claude) echo "--harness claude --scenario children --phases initial" ;;
    children-codex) echo "--harness codex --allow-uncapped-spend --scenario children --phases initial" ;;
    sw2-claude) echo "--harness claude --mode tui --tui-accept-trust --allow-uncapped-spend --scenario warning --phases initial" ;;
    sw2-codex) echo "--harness codex --mode tui --tui-accept-trust --allow-uncapped-spend --scenario warning --phases initial" ;;
    codex-tui-children-write-absence) echo "--harness codex --mode tui --tui-accept-trust --allow-uncapped-spend --scenario children --phases initial" ;;
    ht910-claude) echo "--harness claude --mode tui --tui-accept-trust --allow-uncapped-spend --phases initial,daemon-restart --verify-wait 150" ;;
    ht910-codex) echo "--harness codex --mode tui --tui-accept-trust --allow-uncapped-spend --phases initial,daemon-restart --verify-wait 150" ;;
    p40-crash-fix3) echo "--harness codex --mode tui --tui-accept-trust --allow-uncapped-spend --phases initial,daemon-crash --verify-wait 150" ;;
    # Codex sandbox with the state dir at the XDG default layout under a run HOME, not the driver's default <run>/state.
    codex-sandbox-xdg-state) echo "--harness codex --allow-uncapped-spend --phases initial --state-dir @XDG_STATE@" ;;
    wave28-claude-wake-submission) echo "--harness claude --mode tui --tui-accept-trust --allow-uncapped-spend --scenario lostprompt --phases initial" ;;
    *) return 1 ;;
    esac
}

cell_claude_version() { echo "${HT_CLAUDE_VERSION:-2.1.287}"; }

cell_claude_prefix() { # pins the versioned Claude binary on a run-root PATH entry; prints the entry
    v=$(cell_claude_version)
    bin=${HT_CLAUDE_BIN:-$HOME/.local/share/claude/versions/$v}
    [ -x "$bin" ] || { echo "cell: claude $v binary not found at $bin (set HT_CLAUDE_BIN)" >&2; return 1; }
    shim=${HT_CLAUDE_SHIM:-/private/tmp/ht-cell-claude-$v}/shim
    mkdir -p "$shim" && ln -sf "$bin" "$shim/claude"
    echo "$shim"
}

cell_codex_bin() { # the fixed Codex 0.159.3 install (the `codex` on PATH auto-updates); $HT_CODEX_BIN overrides
    echo "${HT_CODEX_BIN:-$HOME/.codex/packages/standalone/releases/0.159.3-aarch64-apple-darwin/bin/codex}"
}

cell_versions() { # HARNESS-BINARY -> its version line (a bare name resolves on the current PATH)
    "$1" --version 2>&1 | head -n 1
}

cell_ran_versions() { # DRIVER-OUT -> the harness version(s) the agent itself reported in the run's evidence
    ev=$(sed -n 's/.*"evidence": "\([^"]*\)".*/\1/p' "$1" | tail -n 1)
    [ -n "$ev" ] && [ -d "$ev" ] || return 0
    run=$(dirname "$ev")
    {
        # Under set -e a failed step would end the group early; every source may be absent (ht-4p6).
        { cat "$ev"/*.jsonl 2>/dev/null || true; } | sed -n 's/.*"claude_code_version":"\([0-9.]*\)".*/\1/p'
        { cat "$ev"/pane-*.txt 2>/dev/null || true; } | sed -n 's/.*Claude Code v\([0-9][0-9.]*\).*/\1/p; s/.*OpenAI Codex (v\([0-9][0-9.]*\)).*/\1/p'
        find "$run/codex-home/sessions" -name '*.jsonl' -exec sed -n 's/.*"cli_version":"\([0-9.]*\)".*/\1/p' {} + 2>/dev/null || true
        # The session transcripts the driver recorded (a managed launch streams into the pane, not into the evidence
        # dir): Claude writes its own "version" on every entry. Read-only.
        python3 -c 'import json, sys
for path in (json.load(open(sys.argv[1])).get("facts", {}).get("phase_transcripts") or {}).values():
    print(path)' "$ev/summary.json" 2>/dev/null | while IFS= read -r t; do
            [ ! -f "$t" ] || sed -n 's/.*"version":"\([0-9][0-9.]*\)".*/\1/p; s/.*"cli_version":"\([0-9.]*\)".*/\1/p' "$t"
        done
    } | sort -u | tr '\n' ' ' | sed 's/ $//'
}

run_cell() {
    name=$1; shift
    if [ "$name" = list ]; then for c in $CELLS; do echo "$c"; done; return 0; fi
    repo=$(CDPATH='' cd "$script_dir/.." && pwd -P)
    sha=$(git -C "$repo" rev-parse HEAD)
    flags=$(cell_flags "$name") || { echo "cell: unknown cell '$name' (try --cell list)" >&2; return 2; }
    out=${HT_CELL_OUT:-/private/tmp/ht-cell}/$name-$(date +%Y%m%d-%H%M%S)
    mkdir -p "$out"
    harness=${flags#--harness }; harness=${harness%% *}
    harness_bin=$harness
    case "$flags" in
    --harness\ claude*)
        shim=$(cell_claude_prefix) || return 1
        PATH="$shim:$PATH"; export DISABLE_AUTOUPDATER=1
        harness_bin=$shim/claude
        flags="$flags --claude-bin $harness_bin" ;;
    --harness\ codex*)
        harness_bin=$(cell_codex_bin)
        if [ ! -x "$harness_bin" ]; then
            echo "CELL $name sha=$sha outcome=NOT_EXERCISED reason=fixed Codex 0.159.3 binary not found at $harness_bin (set HT_CODEX_BIN)"
            return 1
        fi
        flags="$flags --codex-bin $harness_bin" ;;
    esac
    . "$script_dir/lib/isolated-herdr.sh"
    # The pane shells inherit the private server's environment: keep the real HOME (harness credentials are read from
    # it) and isolate everything else (config, state, runtime dir, socket) under the private root.
    ih_env_args() {
        for n in $(env | sed -n 's/^\(HERDR_[A-Za-z0-9_]*\)=.*/\1/p'); do printf -- '-u\n%s\n' "$n"; done
        # A cell launched from inside a Claude Code session must not leak its markers into the pane (a leaked
        # CLAUDE_CODE_CHILD_SESSION turns the pane's Claude transcript saving off).
        for n in $(env | cut -d= -f1 | grep -E '^(CLAUDE_CODE_[A-Za-z0-9_]*|CLAUDECODE|CLAUDE_PID|CLAUDE_EFFORT)$' || true); do printf -- '-u\n%s\n' "$n"; done
        printf '%s\n' "XDG_CONFIG_HOME=$1/cfg" "XDG_STATE_HOME=$1/st" "XDG_RUNTIME_DIR=$1/rt" \
            "HERDR_CONFIG_PATH=$1/cfg/herdr.toml" "HERDR_SOCKET_PATH=$1/h.sock"
    }
    root=$(ih_root_new)
    trap 'ih_teardown "$root" >/dev/null 2>&1 || true' EXIT INT TERM
    ih_start "$root" "cell-$name" || { echo "CELL $name sha=$sha outcome=NOT_EXERCISED reason=isolated Herdr session did not start"; return 1; }
    ws=$(env HERDR_SOCKET_PATH="$root/h.sock" HERDR_CONFIG_PATH="$root/cfg/herdr.toml" XDG_CONFIG_HOME="$root/cfg" \
        XDG_STATE_HOME="$root/st" XDG_RUNTIME_DIR="$root/rt" herdr workspace create --cwd "$out" --label "cell-$name")
    wid=$(printf '%s' "$ws" | sed -n 's/.*"workspace_id":"\([^"]*\)".*/\1/p' | head -n 1)
    pid=$(printf '%s' "$ws" | sed -n 's/.*"pane_id":"\([^"]*\)".*/\1/p' | head -n 1)
    [ -n "$wid" ] || { echo "CELL $name sha=$sha outcome=NOT_EXERCISED reason=private Herdr session could not host a workspace: $ws"; return 1; }
    xdg=$out/home/.local/state/herdr-threads
    flags=$(printf '%s' "$flags" | sed "s#@XDG_STATE@#$xdg#")
    before=$(cell_versions "$harness_bin")
    rc=0
    # shellcheck disable=SC2086
    env HERDR_ENV=1 HERDR_WORKSPACE_ID="$wid" HERDR_PANE_ID="$pid" HERDR_SOCKET_PATH="$root/h.sock" \
        HERDR_CONFIG_PATH="$root/cfg/herdr.toml" XDG_CONFIG_HOME="$root/cfg" XDG_STATE_HOME="$root/st" XDG_RUNTIME_DIR="$root/rt" \
        python3 "$script_dir/validate-native-demo.py" $flags --bin "${HT_BIN:-$repo/target/debug/herdr-threads}" "$@" \
        >"$out/driver.out" 2>"$out/driver.err" || rc=$?
    after=$(cell_versions "$harness_bin")
    ran=$(cell_ran_versions "$out/driver.out")
    status=$(sed -n 's/.*"manifest_status": "\([^"]*\)".*/\1/p' "$out/driver.out" | tail -n 1)
    evidence=$(sed -n 's/.*"evidence": "\([^"]*\)".*/\1/p' "$out/driver.out" | tail -n 1)
    echo "CELL $name sha=$sha rc=$rc manifest_status=${status:-none} harness_bin=$harness_bin harness_before=[$before] harness_after=[$after] harness_ran=[$ran] evidence=${evidence:-none} out=$out"
    return 0
}

# --matrix: every cell up to N attempts on this SHA; an attempt whose harness version moved is invalid.
run_matrix() {
    cells=${*:-$CELLS}
    attempts=${HT_CELL_ATTEMPTS:-3}
    for c in $cells; do
        n=0; outcome=FAIL; line=
        while [ "$n" -lt "$attempts" ]; do
            n=$((n + 1))
            line=$(run_cell "$c" | grep '^CELL ' | tail -n 1 || true)
            echo "ATTEMPT $n $line"
            [ -z "${HT_MATRIX_LOG:-}" ] || echo "ATTEMPT $n $line" >>"$HT_MATRIX_LOG"
            before=$(printf '%s' "$line" | sed -n 's/.*harness_before=\[\([^]]*\)\].*/\1/p')
            after=$(printf '%s' "$line" | sed -n 's/.*harness_after=\[\([^]]*\)\].*/\1/p')
            st=$(printf '%s' "$line" | sed -n 's/.*manifest_status=\([^ ]*\).*/\1/p')
            case "$c" in
            claude-*|children-claude|sw2-claude|ht910-claude|wave28-*) want="$(cell_claude_version) (Claude Code)"; want_ran=$(cell_claude_version) ;;
            *) want="codex-cli 0.159.3"; want_ran=0.159.3 ;;
            esac
            if [ -z "$st" ]; then # the cell never reached the driver (setup failure): not a product verdict
                outcome="NOT_EXERCISED (cell setup)"; continue
            fi
            ran=$(printf '%s' "$line" | sed -n 's/.*harness_ran=\[\([^]]*\)\].*/\1/p')
            if [ "$before" != "$after" ] || [ "$before" != "$want" ] || [ "$ran" != "$want_ran" ]; then
                outcome="INVALID: version before [$before] after [$after] ran [$ran] want [$want]"; continue
            fi
            case "$st" in
            PASS) [ "$n" -eq 1 ] && outcome=PASS || outcome="PASS (flaky)"; break ;;
            UNSUPPORTED|NOT_EXERCISED) outcome="NOT_EXERCISED ($st)"; break ;;
            *) outcome="FAIL ($st)" ;;
            esac
        done
        echo "MATRIX cell=$c attempts=$n outcome=$outcome :: $line"
        [ -z "${HT_MATRIX_LOG:-}" ] || echo "MATRIX cell=$c attempts=$n outcome=$outcome :: $line" >>"$HT_MATRIX_LOG"
    done
}

if [ "${1:-}" = --matrix ]; then
    shift
    run_matrix "$@"
    exit 0
fi
if [ "${1:-}" = --ran-versions ]; then # DRIVER-OUT: the harness_ran detector alone (tests)
    [ $# -eq 2 ] || { echo "usage: $0 --ran-versions DRIVER-OUT" >&2; exit 2; }
    cell_ran_versions "$2"
    exit 0
fi
if [ "${1:-}" = --cell ]; then
    [ $# -ge 2 ] || { echo "usage: $0 --cell list|NAME [driver args]" >&2; exit 2; }
    shift
    run_cell "$@"
    exit $?
fi
exec python3 "$script_dir/validate-native-demo.py" "$@"
