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
#   printed, explicit -s/-a on-request (Codex then skips its folder-trust screen) and --dangerously-bypass-hook-trust
#   (scratch under /private/tmp only; no approval/sandbox bypass). Any trust/approval screen at startup is a FAIL with
#   nothing typed. --phases initial,clear uses /new; restart /quit + relaunch; resume `codex resume <thread>`.
#   scripts/validate-native-demo.sh --harness codex --mode tui --allow-uncapped-spend --phases initial,clear --scenario warning --bin ...
#
# Must run inside a Herdr pane (HERDR_ENV=1). Creates one owned scratch tab and closes it;
# stops only a daemon it started; never edits user-global Claude/Codex/aisw configuration.
set -eu
script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd -P)
umask 077
exec python3 "$script_dir/validate-native-demo.py" "$@"
