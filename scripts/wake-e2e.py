#!/usr/bin/env python3
"""Pre-release wake e2e: does `invite` / `send -p N` wake a real idle Claude Code and Codex agent exactly when it should?

  scripts/wake-e2e.py [--bin PATH] [--harness claude,codex] [--cases CASE,...] [--evidence-dir DIR] [--keep]
                      [--harness-mode macos|linux]

The matrix is every combination of
  action      invite | send (p1, the default) | send --nudge (p0) | send -p 3 | send --require-ack (p0 + ACK)
              | the first direct send of a 1:1 pair (pair-open: p1, the pair auto-joins, no invitation)
  channel     ordinary thread | automatic 1:1 pair
  threshold   the recipient's nudge threshold T: 0 (default) | -1 (every item) | 3
  harness     claude | codex (agy: opt-in, see below)
Expected (expect_wake): an idle recipient wakes iff the item's priority <= T (T < 0: always); invitations and ACK
requests are p0, so they wake at every T. A silent item (for example a p1 send at T=0) leaves the recipient idle and
arrives on its next turn (a neutral `hello` prompt). ACK requests must come back acknowledged, invitations must be
accepted, and the first message of a 1:1 pair must be delivered with no `accept` (the pair auto-joins).
mixed-order/group sends p5, p1, p0 and an ACK request with a deadline to an idle recipient and checks they appear in
presentation order: the ACK request (deadline) first, then p0, p1, p5. The order is judged only at T=0, where the
p5 and p1 sent first cannot wake the recipient; at T=-1 and T=3 the row checks delivery, markers and the ACK and
records the order as unjudged.
The busy-* cases repeat invite/p1/p0/ack while the recipient is mid-turn: a recipient is nudged only when idle,
so no wake prompt may be typed during or after that turn, and the item must arrive inside the turn instead.
A woken recipient with hooks must also make no `inbox` call for a single-item send (no_extra_inbox): the hook or
channel already delivered it, so the skill's bare-wake rule must not fire (Delivery contract rule 3). This check
depends on model behaviour; the row records the count (inbox_calls) and calls-<recipient>.txt the logged calls.
busy-fail-send-ack/group (Claude only) makes the busy turn's last tool fail: the ACK request must still arrive and be
acknowledged inside that turn, with no wake after it.
busy-tool-p1/group (Codex only) is busy-send-p1 whose turn calls no shell command after the send, only `apply_patch`:
the item must arrive at that tool's boundary (every tool is one). Claude's mod delivers at every tool already.

--queue-proof sets programmatic_wake on each owned Codex seat (ht human config seat set --seat, owned instance, before the first send; never node settings) and requires proof using bounded read-only snapshots of the run's own
canonical store. Ordinary positive cases require exact Accepted intent, arm/native baseline and observed Add-helper
witness; Unknown is not acceptance. No native typing after a possible queue effect is a compositional inference from
the reviewed dispatcher invariant, not an independent host-call trace. Native fallback is also labeled inference from
ineligible canonical scope, a successful wake and no possible-effect intent. Draft-hold/group and native-fallback/group
are opt-in controls. Nothing reads queue state as body delivery or receipt proof.

Isolation: an isolated daemon and state dir (driven by --bin, default the repo's bin/herdr-threads), owned hooks
installed by the public `setup` CLI into scratch HOME / CLAUDE_CONFIG_DIR / CODEX_HOME copies, one owned Herdr
workspace on the current Herdr server (closed at the end unless --keep). The script itself never writes the real
user config. The harness mode (--harness-mode, default `macos` on macOS and `linux` elsewhere) decides how the
harnesses get their credentials:
- macos: Claude keeps its real config directory (on macOS its keychain credentials are keyed by it) and loads the scratch
  installation as a flag layer (`--setting-sources project,local --settings <scratch>/settings.json`), which also
  carries the delivery mod (`env.CLAUDE_CODE_PLUGIN_DIRS`). Limit: Claude Code itself still records its session
  state (transcripts, project history) in that real directory, as any Claude session does.
- Codex runs with CODEX_HOME at a scratch home holding setup's hooks.json/config.toml and a private mode-0600 copy
  of the source profile's auth.json (--codex-auth-home, else $CODEX_HOME or ~/.codex). Codex writes refreshed
  tokens only into that copy; the source file is never written, and credentials never enter the evidence. A refresh
  inside the run is recorded (no contents) because it can rotate the source profile's refresh token. The agent
  working directory is pre-trusted in that scratch config.toml only. Codex runs with --approve-for-me.
- linux (no keychain): Claude gets a fully scratch CLAUDE_CONFIG_DIR, the one setup installed into, with
  `.claude.json` pre-seeded so no first-run screen appears (see claude_scratch_state), and its credential only from
  CLAUDE_CODE_OAUTH_TOKEN or ANTHROPIC_API_KEY: a logged-in Claude config cannot be reused, it is never read, and
  --claude-config-dir is refused. Preflight checks the driver's own environment; the agents get the Herdr server's
  (panes inherit it), so both must carry the credential (inside scripts/linux-e2e both are the server's). Codex is
  set up as on macOS and keeps --approve-for-me, except inside the scripts/linux-e2e runner container
  (HT_LINUX_E2E_CONTAINER=1 from the runner and /.dockerenv), where it runs with
  --dangerously-bypass-approvals-and-sandbox: its bubblewrap sandbox cannot create a user namespace under Docker's
  default seccomp profile, so the container is the sandbox. Evidence files are redacted of `sk-ant-` tokens and of
  the Claude credential values and their tails; with --keep the scratch `.claude.json` (mode 0600) keeps an
  ANTHROPIC_API_KEY's 20-character approval suffix.
- agy (opt-in only, `--harness agy`, never in the default matrix; any harness mode): agy runs with HOME at the
  scratch home setup installed its plugin into, its settings.json seeded the way the agy gate seeds it (Gemini
  provider, the agents' working directory trusted, no alternate screen), and its model credential only from
  GEMINI_API_KEY (plus GOOGLE_GEMINI_BASE_URL when set, e.g. scripts/agy-relay) in the driver's environment, passed
  to the pane through a private mode-0600 env file in the scratch root, never on a command line. No Google sign-in
  is read or used. The first-run screens (colour scheme, Terms of Service) are walked with keys. Like Codex, agy
  wakes natively (`herdr-threads: wake up` typed into an empty composer) and its hook delivers a page, so its
  mixed-order presentation order is judged.
- opencode (opt-in only, `--harness opencode`, never in the default matrix; any harness mode): real opencode TUIs
  (`oc-t0`, `oc-t-1`, `oc-t3`) with HOME, the four XDG_*_HOME dirs, TMPDIR and OPENCODE_CONFIG_DIR in the scratch
  root. `setup opencode --with-permissions` (the consent an unattended run gives) installs the plugin and skill into
  that OPENCODE_CONFIG_DIR; a scratch global opencode.json selects the OpenRouter model (--opencode-model, default
  openrouter/deepseek/deepseek-v4-pro) and allows every shell command (no permission dialog can hold a turn). The
  agents work in a scratch project dir. The model credential comes only from OPENROUTER_API_KEY in the driver's
  environment: it is never passed to a helper process, reaches the panes only through a private mode-0600 env file in
  the scratch root (removed at cleanup even with --keep), and every evidence file is scanned for it at the end (a
  hit is scrubbed and fails the run). opencode is a channel harness like Claude's mod: its plugin appends each item
  to the session and wakes it itself, so mixed-order is unjudged; every opencode row also checks, from the plugin's
  own ledger, that items arrived by append and that a wake was the plugin's submit (none when no wake is expected),
  and that the composer is left empty. The recipient's transcript is read from its own sessions in the scratch
  opencode.db (synthetic appended parts left out, as the TUI hides them), so a turn that scrolled off the
  alternate screen is still judged.
- hermes (opt-in only, `--harness hermes --hermes-home DIR`, never in the default matrix): DIR is a scratch HOME that
  Hermes' own installer (`HOME=DIR bash setup-hermes.sh`) installed into, with its model credential and the telemetry
  answer already in DIR/.hermes (docs/evidence/hermes-composer/README.md). The driver runs `setup hermes` and
  `hermes plugins enable herdr-threads` with HOME=DIR (writing only there), adds HERDR_PANE_ID and HERDR_ENV to the
  default profile's terminal.client_env_passthrough (merged with the names listed there), and starts the usual
  recipients, `he-t0`, `he-t-1` and `he-t3`: every Hermes pane of a profile shares one gateway, and a gateway
  session reaches its own pane only through that passthrough (ht-hermes-identity; without it no session is seated).
  No Hermes from DIR may be running at start;
  cleanup stops the gateway the recipients' sessions run in. Herdr reports a Hermes pane `idle` throughout a turn, so the
  driver reads a Hermes recipient's state from its screen (hermes_idle): idle only while the composer's top rule
  carries no live status. Hermes 0.22 hosts its sessions in the gateway, so the native wake matches the
  binding's own pane rather than its process (ht-f0xn; TRUST-POLICY, Hermes composer reader).
- tmux-named (opt-in only, `--cases tmux-named/send-p0,tmux-named/send-p1`, never in the default matrix and gated on no
  version): a named seat (`HERDR_THREADS_SEAT_NAME`, no Herdr pane) runs real Claude (hooks-only installation),
  in its own private `tmux -L ht-test-<uuid>` server, and a person seat sends one direct item. At T=0 the p0 must
  wake the idle seat by a content-free wake typed into the tmux composer; the p1 control must stay silent and arrive
  on the next turn. Cleanup kills every private tmux server and verifies it gone, as part of the completeness check.
Agents work in the repository checkout (already trusted by Claude, so no folder-trust dialog is answered).
The senders are two owned coordinator shell panes recorded as person seats (`human me init`, which keeps its agent
guard: the driver strips every registered agent marker from their commands and the panes run only a shell), not
models and never an agent identity, so the run works under any agent or from a plain shell; the second one is a
fresh first contact for every recipient (busy-invite/1:1). Their messages are person-authored: they carry the
`[human]` frame marker and are priority, so they are never held by a recipient's catch-up hold; agent-to-agent
delivery into a seat that is catching up is not covered by this gate.

Exit codes: 0 every expected case passed and every owned resource was cleaned up; 1 a case failed or was missing, or
cleanup left an owned workspace or daemon alive; 2 environment refusal. `--self-test` checks the driver's own
credential, cleanup and completeness paths without harnesses. Runs locally only (needs authenticated harnesses);
never in CI. Python 3.9+, standard library only.
"""
from __future__ import annotations

import argparse
import contextlib
import datetime
import hashlib
import json
import os
import re
import shlex
import shutil
import signal
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import uuid
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
EXIT_OK, EXIT_FAIL, EXIT_REFUSED = 0, 1, 2

CASES = ("invite/group", "pair-open/1:1", "send-p1/group", "send-p0/group", "send-p3/group", "send-ack/group",
         "send-p1/1:1", "send-p0/1:1", "send-p3/1:1", "send-ack/1:1", "mixed-order/group",
         "busy-invite/group", "busy-invite/1:1", "busy-send-p1/group", "busy-send-p0/group", "busy-send-ack/group",
         "busy-send-p1/1:1", "busy-send-p0/1:1", "busy-send-ack/1:1", "busy-fail-send-ack/group",
         "busy-tool-p1/group")
# Claude only: the busy turn's last tool fails, and the item must still arrive (and be ACKed) inside the turn. The
# mod's context rides an errored tool result and is settled by the next model request (ht-mod-busy).
CLAUDE_CASES = ("busy-fail-send-ack/group",)
# Recipient nudge thresholds: 0 (the default), -1 (negative: every item wakes), 3.
THRESHOLDS = (0, -1, 3)
# Priority of each action's item. Invitations and ACK requests are always p0; pair-open is the first direct send of a
# 1:1 pair at the default p1 (the pair auto-joins both seats, so there is no invitation).
ITEM_PRIORITY = {"send-p1": 1, "send-p0": 0, "send-p3": 3, "send-ack": 0, "invite": 0, "pair-open": 1}
MIXED_DEADLINE_S = 90
# Opt-in Codex cases (ht-327), never in the default matrix: before the send the recipient's one Codex process holds
# extra session bindings from two `/side` conversations (ephemeral in-process forks) that each made one
# herdr-threads `inbox` call. side-nudge: a p0 send (`--nudge`, no ACK) still wakes an idle recipient at T=0; then, in
# its sibling-displayed phase (ht-yha.62), a second p0 sent while a third `/side` turn is busy and displayed by that
# side's own `inbox` still wakes the idle main session afterwards (a sibling's display never blocks the process's
# wake; inconclusive, not failed, when the side binding never displayed it). side-draft: a typed composer draft is never typed
# into (busy-input guard) and the sender's overdue warning names the refused wake. They also run on `co-plain`, a
# Codex with its hooks feature off, so every session binds by a plain CLI call only (as Main's seat did on
# 2026-10-13): no lifecycle claim ranks the group, the same-process rule leaves all three connected.
SIDE_CASES = ("side-nudge/group", "side-draft/group")
# Opt-in Codex cases with a `/side` chat left DISPLAYED when a p0 is sent (ht-z41e.8), never in the default matrix,
# each in exactly one queue mode. side-queue (--queue-proof required): the wake is an Accepted queue intent on the
# process's root session (never the side), the item arrives in the root's turn, and the side chat's pane shows no
# change (nothing typed, no new turn). side-native-defer (queue off, refused under --queue-proof): a displayed side
# composer reads non-Empty (ht-z41e.1), so native wake types nothing into it and the root is woken once the side
# chat closes. side-nudge/side-draft keep their pass rules in both modes, with the transport expectation per mode
# (see QueueCaseEvidence.finish): under --queue-proof a hooked recipient with side sessions is queue-woken on its root
# (a busy side binding never refuses it, spike S2 `drop`); side-draft's typed root draft refuses the queue lane (the amended
# admission), so it judges like native: no wake, draft intact, queue effect none (ht-z41e.54);
# without it everything is native, and co-plain (plain CLI bindings, never registered) is native in both modes.
# side-queue-focused (ht-z41e.43, --queue-proof, private Herdr only, run alone or beside root-draft-queue): the recipient
# pane is FOCUSED and a side turn is BUSY when the p0 is sent. The queue lane drops the Herdr `working` refusal beside a
# sibling binding and has no focus dwell, so the root is armed on the first eligible attempt (well inside
# FOCUSED_DWELL_S, the native focused dwell) and nothing is typed into the side composer. Focusing moves the user's
# panes, so it is refused unless scripts/wake-e2e-herdr set HT_WAKE_E2E_PRIVATE_HERDR=1 on its private server.
# root-draft-queue (--queue-proof): no /side; a draft typed in the root's own composer refuses the queue lane (the
# amended admission: any typed text refuses) and must stay intact, submitted by nothing; once cleared the wake is an
# Accepted queue intent and the item arrives. A lost or submitted draft, or a queue wake into a root mid-turn, is a
# release-blocking defect named in report.md.
SIDE_QUEUE_CASES = ("side-queue/group", "side-queue-focused/group")
ROOT_DRAFT_CASES = ("root-draft-queue/group",)
FOCUSED_QUEUE_CASE = "side-queue-focused/group"
PRIVATE_HERDR_ENV = "HT_WAKE_E2E_PRIVATE_HERDR"
FOCUSED_DWELL_S = 60  # the native focused-composer dwell (src/host/native.rs): a dwell-delayed arm lands past it
ROOT_DRAFT = "root-draft-kept-by-wake-e2e"
SIDE_FOCUS_SLEEP_S = 100  # spans the pre-send reads, FOCUSED_DWELL_S + 10 s of watching and the settlement reads
SIDE_FOCUS_PROMPT = ("Side focus test of wake-e2e {tag}: run the shell command `sleep {sleep}`, then print "
                     "SIDE-FOCUS-BUSY-DONE. Do not run any herdr-threads command.")
SIDE_FOCUS_BUSY_DONE = re.compile(r"^[^\w\n]*SIDE-FOCUS-BUSY-DONE", re.M)
SIDE_DEFER_CASES = ("side-native-defer/group",)
# Opt-in Codex cases for the per-seat config surface (ht-z41e.30), never in the default matrix, hooked recipient at T=0,
# each in exactly one queue mode. config-prose (queue off, refused under --queue-proof): the agent is told in prose only
# to turn on programmatic wake for its own seat and must find the setting with `config seat list`, then write it through
# the human tier (`human config seat set programmatic_wake on`); the store must read the key on. The driver unsets it
# afterwards so later cases stay native. flag-flip (--queue-proof required): up to FLAG_FLIP_TRIALS trials in which the
# driver unsets programmatic_wake the moment a new queue intent is first seen `prepared`; the stale reservation must be
# refused before arming and the same attempt must settle as a native wake (`stale_authority_native`), with the item
# delivered. A trial whose first new intent is already armed/accepted (or none) missed the window and is retried.
CONFIG_PROSE_CASES = ("config-prose/group",)
FLAG_FLIP_CASES = ("flag-flip/group",)
FLAG_FLIP_TRIALS = 3
CONFIG_PROSE_PROMPT = ("Config test {tag}: Turn on programmatic wake for your own herdr-threads seat. Find the right "
                       "setting with the herdr-threads CLI, change it, then print CONFIG-PROSE-DONE.")
CONFIG_PROSE_DONE = re.compile(r"^[^\w\n]*CONFIG-PROSE-DONE", re.M)
HOOK_TRACE_FILE = "hook-trace.jsonl"
# Opt-in Claude case, never in the default matrix: `cl-hooks`, a Claude installed by `setup claude --hooks-only` (no
# delivery mod), gets an ACK request while idle at T=0. Herdr must type the content-free `herdr-threads: wake up`, the
# UserPromptSubmit hook must put the message into the woken turn, and its ACK must come from that hook's display
# (`via: hook_context`), not from an `inbox` round trip. hooks-busy-tool/group is busy-tool-p1 on cl-hooks, whose turn
# reads a file with Read after the send: with no mod, only that Read's PreToolUse can deliver it in the turn. cl-hooks
# runs nothing else.
# `hooks-suggestion` (ht-ansi-composer): the same p0 ACK request, sent only once cl-hooks shows a dim prompt suggestion
# after two finished turns (cl-hooks runs with CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION=1, as the default-on setting would
# be). The styled composer read must take the suggestion for empty, so the idle Claude is still woken.
HOOKS_CASES = ("hooks-wake/group", "hooks-suggestion/group", "hooks-busy-tool/group")
# A composer row whose text is drawn dim after an undimmed prompt glyph, in a `--format ansi` read.
SUGGESTION_ROW = re.compile(r"^❯[\xa0 ](?:\x1b\[0m)?\x1b\[2m[^\x1b\s]")
# Opt-in Codex case (ht-end-of-turn), never in the default matrix: the p0 is sent while the turn's only tool is held, so
# it arrives after the last tool boundary. The turn's Stop hook must continue it with the page as the continuation
# prompt (a `<hook_prompt>` in the rollout naming the message), with no wake typed during or after the turn.
EOT_CASES = ("eot-send-p0/group",)
EOT_PROMPT = ("End-of-turn test {tag}: run the shell command `{hold}` and no other command, then print EOT-DONE and end "
              "your turn. If herdr-threads items reach you after that, handle them exactly as your standing "
              "instructions say without running any command, and print EOT-DONE again.")
# Matched as BUSY_DONE_LINE is, after the prompt's last mention of the marker.
EOT_DONE_LINE = re.compile(r"^[^\w\n]*EOT-DONE.*$", re.M)
EOT_PROMPT_END = "EOT-DONE again."
# Opt-in Codex case (ht-wake-loop), T=0 recipient only: our hooks stop running, as after an upgrade that breaks them.
# Right after an ACK request the candidate binary is made non-executable (the coordinator keeps a private copy), so the
# woken turn's hooks and any herdr-threads call the model makes fail and the session never reaches the daemon. When
# the first ladder retry comes due (30 s) the daemon must record the wake `unreceived` and type nothing more while the
# session stays silent; with the binary restored, the next prompt delivers the item.
UNRECEIVED_CASES = ("wake-unreceived/group",)
UNRECEIVED_VERDICT_S = 60  # the verdict lands at the first retry, 30 s after the reservation
SIDE_FORKS = 2
SIDE_BINDING_WAIT_S = 20.0  # a /side fork's binding registers within its first inbox call; bounded wait
# Opt-in, Codex at T=0 only (ht-wake-outcome): with Codex's `/model` picker open, p0 sends neither wake the recipient
# nor type into the picker (its highlighted `› ` row reads as a draft, a hold that ends by itself), and the sender gets
# no `wake=` warning (only lasting blocks warn); once the picker is closed the recipient wakes and gets every item.
# (`wake=unreadable` cannot be staged this way on Codex: no captured Codex screen lacks a `› ` row.)
WAKE_BLOCK_CASES = ("modal-hold/group", "draft-hold/group")
QUEUE_NATIVE_CASES = ("native-fallback/group",)
# Required queue proof also passes these on a stale-authority native fallback: concurrent
# decisions anywhere on the node can stale the reservation between selection and arm
# (TRUST-POLICY, queue wake stale authority).
QUEUE_STALE_CASES = ("mixed-order/group",)
QUEUE_SAMPLE_LIMIT = 128
QUEUE_EVIDENCE_BYTES = 512 * 1024
WAKE_BLOCK_HOLD_S = 30
PICKER_ROW = re.compile(r"^\u203a 1\. ")
WAKE_BLOCK_WAIT_S = 90
# Opt-in, mod Claude at T=0 only (docs/validation/claude-wake-busy-2026-11-14.md): the mod must not submit its wake
# while the engine already holds another prompt. enter-race: an ACK request is held by a typed composer draft; the
# driver presses Enter, and a 2 s UserPromptSubmit hook keeps the prompt between Enter and its turn.start; the mod
# appends the item in that window and the prompt's turn, which calls no tool, must name it. queued-prompt: an ACK
# request and a follow-up prompt both arrive while the turn writes its final answer, after its last tool call; the mod
# appends the item at the turn's end and the follow-up's turn reads it. Both are ACKed `via: append`, and neither may
# put a `wake up` into the engine's queue (the transcript's `queue-operation` rows) or on screen. A run that did not
# stage its window fails as such (`window_staged`). The slow hook sits in the scratch settings every Claude recipient
# of the run shares, so these cases run alone: a `--cases` list mixing them with others is refused.
PROMPT_RACE_CASES = ("enter-race/group", "queued-prompt/group")
PROMPT_RACE_HOOK_S = 2
PROMPT_RACE_DRAFT_S = 4.0
ENTER_RACE_PROMPT = ("Enter-race test {tag}: without running any command or using any tool, name the id of every "
                     "herdr-threads item you have received so far, and finally print RACE-DONE.")
QUEUED_PROMPT = ("Queued-prompt test {tag}: run the shell command `{hold}` and no other command, then write a 500-word "
                 "essay about ocean tides without using any tool, and finally print ESSAY-DONE.")
QUEUED_FOLLOWUP = ("Queued follow-up {tag}: handle any herdr-threads items exactly as your standing instructions say, "
                   "and finally print RACE-DONE.")
RACE_DONE_LINE = re.compile(r"^[^\w\n]*RACE-DONE.*$", re.M)
RACE_PROMPT_END = "RACE-DONE."
# Opt-in, Codex at T=0 only (ht-unmk): Codex is killed and resumed in the same pane. A resumed Codex runs no hook until
# its first turn, so its seat stays unavailable while it sits idle; a p1 still leaves it idle, and a p0 wakes it.
# resume-quit leaves Codex through `/quit` instead, so its SessionEnd hook disconnects the binding as an end event
# (`liveness_end_event`) before the same session is resumed: it must wake the same way.
RESUME_CASES = ("resume-idle/group", "resume-quit/group")
QUIT_EXIT_S = 30
RESUME_QUIET_S = 15
SIDE_DEADLINE_S = 45
# Opt-in cases (ht-kqrt.61, N2-e2e), never in the default matrix and gated on no version: a named seat
# (`HERDR_THREADS_SEAT_NAME`, no Herdr pane) runs real Claude (hooks-only installation: the mod's channel would wake it
# instead) inside its own private `tmux -L ht-test-<uuid>` server at T=0. A person seat sends one direct item;
# the daemon must type the content-free wake into the tmux composer iff the priority is <= T (p0 wakes, the p1 control
# stays silent and arrives on the next turn). The tmux server is the run's: cleanup kills it and verifies it is gone.
TMUX_CASES = ("tmux-named/send-p0", "tmux-named/send-p1")
# A running turn's status line (Claude "esc to interrupt", Codex "Esc to interrupt"); the tmux recipient has no Herdr
# agent status, so its idle/busy comes from the screen.
# Claude only: the tmux wake gate needs the harness's `screen_blocked` recognizer, and Codex has none (no captured
# fixture), so a tmux Codex seat is never typed and its sender reads `wake=no_wake`; a wake case cannot pass for it.
TMUX_HARNESSES = ("claude",)
TMUX_RUNNING = re.compile(r"esc to interrupt", re.I)
# The composer's prompt glyph at a line start (Claude `❯`, Codex `›`), possibly inside a box border.
COMPOSER_UP = re.compile(r"^[\s│]*[❯›]", re.M)
SIDE_DRAFT = "draft-kept-by-wake-e2e"
# The sibling-displayed side turn: long enough that the p0 is sent while it is busy (a wake then is refused), then the
# side's own `inbox` displays it to the side binding.
SIBLING_SIDE_PROMPT = ("Side sibling of wake-e2e {tag}: run the shell command `sleep 10`, then run the shell command "
                       "`{shim} inbox` once, then print SIDE-SIB-DONE. Do not act on anything it shows.")
SIBLING_SIDE_DONE = re.compile(r"^[^\w\n]*SIDE-SIB-DONE", re.M)
# Codex runs with its shell snapshot off (launch_line, ht-l16), so the candidate's bin dir stays first on PATH; a plain
# recipient is still given the shim's exact path, since it has no hook context naming the run's --state-dir.
PLAIN_HINT = (" Your herdr-threads hooks are off: always run herdr-threads by the exact path `{shim}` (never a bare "
              "`herdr-threads`), and whenever a turn starts with `herdr-threads: wake up` or `herdr-threads: attention pending`, run `{shim} inbox` once, "
              "then handle what it shows as above.")
# Busy cases send while the recipient is mid-turn (between two tool calls): no wake may be typed or submitted, during
# the turn or after it, and the item must arrive in that turn at the next tool boundary (Delivery contract rules 3-4).
BUSY_PROMPT = ("Busy-turn test {tag}: run the shell command `{hold}`, then run the shell command `sleep 1`, then "
               "handle any herdr-threads items exactly as your standing instructions say, and finally print BUSY-DONE.")
# The `sleep 2` keeps the failing tool running until the item has reached the mod (as `sleep 1` does in BUSY_PROMPT).
BUSY_FAIL_PROMPT = ("Busy-turn test {tag}: run the shell command `{hold}; sleep 2; exit 3` (it fails on purpose), then run no "
                    "other tool: name the id of every herdr-threads item you received, and finally print BUSY-DONE.")
# busy-tool-*: after the hold, the turn's only tool is a non-shell one, so the item can arrive in that turn only at
# that tool's boundary.
BUSY_TOOL_PROMPT = ("Busy-turn test {tag}: run the shell command `{hold}`, then {tool}; run no other shell command in "
                    "this turn. Then handle any herdr-threads items exactly as your standing instructions say, and "
                    "finally print BUSY-DONE.")
BUSY_TOOL = {"codex": "use your apply_patch tool (not the shell) to add the file `{dir}/patched.txt` containing `x`",
             "claude": "read the file `README.md` with your Read tool (not the shell)"}
BUSY_AFTER_S = 40.0
# The busy turn's native end: a line of the agent's output that starts with BUSY-DONE after bullets or markup,
# whatever follows on that line (2.7.0: a model glued text to it, "BUSY-DONECorrection: ..."); the match runs to the
# line's end. It is searched only after the prompt's own "BUSY-DONE." so a wrapped prompt line never matches.
BUSY_DONE_LINE = re.compile(r"^[^\w\n]*BUSY-DONE.*$", re.M)
BUSY_PROMPT_END = "BUSY-DONE."
BUSY_END_GRACE_S = 10.0  # idle this long with no BUSY-DONE line: the turn is over without one
HERMES_BLANK_READ_S = 5.0  # a blank Hermes detection read is retried this long (a TUI not yet drawn)
# Channel literal pinned to policy::CHANNEL_WAKE_UP by tests/harness/claude_mod.rs.
CHANNEL_WAKE_UP = "wake up"
# Keep the native and pre-upgrade channel forms ahead of their short suffix.
WAKE_MARKERS = ("herdr-threads: wake up", "herdr-threads: attention pending", CHANNEL_WAKE_UP)
WAKE_PATTERN = re.compile("|".join(map(re.escape, WAKE_MARKERS)))
QUEUE_WAKE_WINDOW_S = 15.0  # accepted external receiver polling, opt-in eligible positives only
WAKE_WINDOW_S = 6.0     # a woken agent leaves idle well inside this (observed 0.3-1.1 s)
SPACING_S = 35.0        # the per-seat wake retry guard is 30 s after each reservation (src/notification/policy.rs)
BUSY_ENTRY_S = 60  # a held tool past this with no throttle to explain it is shown on the report
TURN_TIMEOUT_MS = 240_000
IDLE = ("idle", "done")


def side_setup_ok(sides, connected_same_pid, plain):
    """The `/side` setup happened: the hookless recipient keeps every side binding connected on its one pid; a hooked
    one has root plus one binding per SIDE_FORKS side."""
    connected = [b for b in sides if b["liveness"] == "connected"]
    return bool(connected_same_pid and len(connected) >= 1 + SIDE_FORKS) if plain else len(sides) >= 1 + SIDE_FORKS


def side_nudge_passed(setup_ok, woke, checks):
    """side-nudge's verdict: the first phase woke the idle recipient and its message arrived; and when the sibling
    phase's side binding displayed its p0 (`sibling_displayed`), the idle main session was still woken for it and
    the message arrived (ht-yha.62). A sibling phase whose side binding never displayed the p0 is inconclusive and
    does not decide the verdict."""
    first = bool(setup_ok and woke and checks.get("delivered"))
    if not checks.get("sibling_displayed"):
        return first
    sibling_woke = checks.get("sibling_wake_s") is not None or bool(checks.get("sibling_woke_before_watch"))
    return first and sibling_woke and bool(checks.get("sibling_delivered"))


def resume_root_session(bindings):
    """The native session resume-idle / resume-quit resume: the newest open top-level binding that is not a `/side`
    fork (neither auxiliary nor concurrent; `None`, an unknown column, does not exclude). The newest binding overall is
    the last `/side` fork after the side cases, and a resumed fork is not the root the case is about."""
    roots = [b for b in bindings if not b.get("auxiliary") and not b.get("concurrent")]
    if not roots:
        raise RuntimeError(f"no root top-level binding to resume among {bindings}")
    return roots[-1]["session"]


def side_queue_passed(setup_ok, proof, delivered_in_root, side_typed, side_view_changed, *, side_bits, root_bits,
                      settled_by_root):
    """side-queue's verdict: the side chat was set up and left displayed, the queue proof passed (Accepted intent on the
    root session naming the side sessions), the item was delivered in the root's turn, the side chat's pane shows
    nothing typed and no other change, every side binding is auxiliary, the root binding is neither auxiliary nor
    concurrent (`None`, an unknown column, never counts as 0), and the root session's own contact settled the wake."""
    side_ok = bool(side_bits) and all((bits or {}).get("auxiliary") == 1 for bits in side_bits.values())
    root_ok = (root_bits or {}).get("auxiliary") == 0 and (root_bits or {}).get("concurrent") == 0
    return bool(setup_ok and proof and proof.get("passed") and delivered_in_root
                and not side_typed and not side_view_changed and side_ok and root_ok and settled_by_root)


def _bits_ok(side_bits, root_bits):
    side_ok = bool(side_bits) and all((bits or {}).get("auxiliary") == 1 for bits in side_bits.values())
    root_ok = (root_bits or {}).get("auxiliary") == 0 and (root_bits or {}).get("concurrent") == 0
    return side_ok and root_ok


def side_queue_focused_passed(setup, proof, armed_s, first_attempt, delivered_in_root, side_typed, root_busy_at_arm,
                              *, side_bits, root_bits, settled_by_root):
    """side-queue-focused's verdict (ht-z41e.43). `setup`: `focused` (`agent get` read focused before the send),
    `side_busy` (the side binding's turn was not `ended` at the send), `agent_status` (Herdr's status at the send, which
    must be `working`: the busy side is what the queue lane must ignore) and `side_setup_ok`. Then the queue proof
    (Accepted intent on the root session naming the side sessions) must have armed in under FOCUSED_DWELL_S, so no
    dwell delayed it, as the first intent the send produced; the item arrived in the root's turn, nothing was typed
    into the side composer, the root was not mid-turn at arm, the side/root bits are as in side-queue and the root's own
    contact settled the wake."""
    setup = setup or {}
    return bool(setup.get("focused") and setup.get("side_busy") and setup.get("agent_status") == "working"
                and setup.get("side_setup_ok") and proof and proof.get("passed")
                and armed_s is not None and armed_s < FOCUSED_DWELL_S and first_attempt and delivered_in_root
                and not side_typed and not root_busy_at_arm and _bits_ok(side_bits, root_bits) and settled_by_root)


def focused_queue_private_herdr(environ):
    """True inside scripts/wake-e2e-herdr's private server: its marker is set and the Herdr socket is named."""
    return environ.get(PRIVATE_HERDR_ENV) == "1" and bool(environ.get("HERDR_SOCKET_PATH"))


def composer_holds(screen, text):
    """True when a prompt row of `screen` (`› ...`) holds exactly `text`: the composer's whole content is the draft. A
    row's text ends at the border of a split view (`│`), whatever the other panel shows on that line."""
    return any(line.startswith("\u203a ") and line[2:].split("\u2502")[0].strip() == text
               for line in screen.splitlines())


def screen_tail(screen, lines=40):
    """The last `lines` non-blank lines of a pane read, for the evidence."""
    return [l.rstrip() for l in screen.splitlines() if l.strip()][-lines:]


def root_draft_intact(before, after):
    """The root draft is still exactly the composer's content and no wake marker was added to the pane since `before`
    (earlier wakes stay in the transcript, so markers are counted, not searched for)."""
    return (composer_holds(after, ROOT_DRAFT)
            and len(WAKE_PATTERN.findall(after)) <= len(WAKE_PATTERN.findall(before)))


def root_draft_queue_passed(checks):
    """root-draft-queue's verdict (ht-z41e.43): the draft was typed and read back in the root composer; no wake took
    effect (no armed/accepted/fulfilled intent, no new root turn) while it stood; it was still exactly the draft when
    cleared and nothing submitted it; after the clear the wake is an Accepted queue intent and the item arrived."""
    checks = checks or {}
    return bool(checks.get("draft_typed") and checks.get("no_wake_while_draft") and checks.get("draft_intact")
                and not checks.get("draft_submitted") and checks.get("queue_after_clear") and checks.get("delivered"))


def release_blocking(checks):
    """The release-blocking defects a row's checks show (ht-z41e.43): a root draft lost, a root draft submitted, or a
    queue wake into a root that was mid-turn. Empty when none."""
    checks = checks or {}
    found = []
    if checks.get("draft_intact") is False:
        found.append("root_draft_lost")
    if checks.get("draft_submitted") is True:
        found.append("root_draft_submitted")
    if checks.get("root_busy_at_arm") is True:
        found.append("queue_into_busy_root")
    return found


def queue_arm_s(samples):
    """Seconds from the publication (the send) to the first sample showing a measured-interval intent armed or later
    (armed, accepted or fulfilled); None when no sample does. Samples are appended only on change, so this is the
    first observation of the arm, never earlier than the arm itself."""
    for sample in samples:
        if any(i["phase"] in ("armed", "accepted") or i["logical_status"] == "fulfilled" for i in sample.get("intents", ())):
            end = sample.get("observation_end_ms")
            return round(end / 1000, 2) if isinstance(end, (int, float)) else None
    return None


def first_attempt_accepted(samples, proof, send_ms=None):
    """True when the first intent the send produced (lowest ordinal) is an attempt the queue proof accepted, no
    NoEffect intent has a lower ordinal, and no sample before the arm shows a refused wake_work outcome completed after
    `send_ms` (when given)."""
    intents = {i["ordinal"]: i for sample in samples for i in sample.get("intents", ())}
    accepted = {a["attempt"] for a in (proof or {}).get("attempts", ())}
    if not intents or not accepted:
        return False
    first = intents[min(intents)]
    if first["attempt"] not in accepted:
        return False
    if any(i["phase"] == "no_effect" and i["ordinal"] <= max(
            (n for n, j in intents.items() if j["attempt"] in accepted), default=0) for i in intents.values()):
        return False
    for sample in samples:
        if any(i["phase"] in ("armed", "accepted") for i in sample.get("intents", ())):
            break
        work = sample.get("wake_work") or {}
        if work.get("last_outcome") in REFUSED_OUTCOMES and (send_ms is None or (work.get("completed_at_utc") or 0) >= send_ms):
            return False
    return True


def root_busy_before_arm(samples, root):
    """True when the root session's native turn read not `ended` in any sample from the send up to the arm: the sample
    that first shows an armed/accepted intent counts unless it also shows a fulfillment (the queue wake's own turn)."""
    for sample in samples:
        intents = sample.get("intents", ())
        if any(i["logical_status"] == "fulfilled" or i.get("fulfillment") for i in intents):
            return False
        for native in sample.get("native", ()):
            if native["scope"]["native_session"] == root and native["phase"] != "ended":
                return True
        if any(i["phase"] in ("armed", "accepted") for i in intents):
            return False
    return False


def intents_took_effect(samples):
    """True when any measured-interval intent was armed, accepted, unknown or fulfilled (a wake that may have acted)."""
    return any(i["phase"] in ("armed", "accepted", "unknown") or i["logical_status"] == "fulfilled"
               for sample in samples for i in sample.get("intents", ()))


def hook_trace_record(payload, pane, now_ms):
    """One silent Codex hook-trace record: exactly {utc_ms, pane, event, session, source, transcript_path}. The path is
    only classified (`null`, `present`, `absent`), never stored; cwd, prompt and tool input are never read. The hook
    trace script embeds this function's source, so it must stay self-contained."""
    def clip(value, limit):
        if not isinstance(value, str):
            return None
        return value.encode("utf-8")[:limit].decode("utf-8", "ignore")
    payload = payload if isinstance(payload, dict) else {}
    event = clip(payload.get("hook_event_name"), 64)
    if "transcript_path" not in payload:
        path = "absent"
    else:
        path = "null" if payload["transcript_path"] is None else "present"
    return {"utc_ms": now_ms, "pane": pane, "event": event, "session": clip(payload.get("session_id"), 128),
            "source": clip(payload.get("source"), 32) if event == "SessionStart" else None, "transcript_path": path}


HOOK_TRACE_SCRIPT = """#!/usr/bin/env -S python3 -I
import json, os, sys, time
{record}
try:
    try:
        payload = json.loads(sys.stdin.buffer.read())
        row = hook_trace_record(payload, os.environ.get("HERDR_PANE_ID"), int(time.time() * 1000))
    except Exception:
        row = {{"utc_ms": int(time.time() * 1000), "pane": os.environ.get("HERDR_PANE_ID"), "event": "unparsable",
               "session": None, "source": None, "transcript_path": None}}
    fd = os.open({out!r}, os.O_WRONLY | os.O_APPEND | os.O_CREAT, 0o600)
    try:
        os.write(fd, (json.dumps(row) + "\\n").encode())
    finally:
        os.close(fd)
except BaseException:
    pass
sys.exit(0)
"""


def write_hook_trace_script(path, out):
    """Write the silent trace hook (mode 0755): one record per call appended to `out`, nothing printed, exit 0."""
    import inspect
    path = Path(path)
    path.write_text(HOOK_TRACE_SCRIPT.format(record=inspect.getsource(hook_trace_record), out=str(out)))
    path.chmod(0o755)


def add_hook_trace(doc, command):
    """A copy of a Codex hooks.json document in which every group holding a herdr-threads hook has exactly one silent
    trace entry (`command`, timeout 5) right after its last herdr-threads entry. Pure and idempotent; other groups and
    the herdr-threads entries are unchanged."""
    import copy
    entry = {"type": "command", "command": command, "timeout": 5}
    out = copy.deepcopy(doc)
    for groups in (out.get("hooks") or {}).values():
        for group in groups if isinstance(groups, list) else ():
            hooks = [h for h in group.get("hooks", ()) if h.get("command") != command]
            mine = [i for i, h in enumerate(hooks) if "herdr-threads" in str(h.get("command", ""))]
            if mine:
                hooks.insert(mine[-1] + 1, dict(entry))
            if mine or len(hooks) != len(group.get("hooks", ())):
                group["hooks"] = hooks
    return out


def read_hook_trace(path):
    """The trace's records ordered by time; an unreadable file or line is skipped, never an error."""
    rows = []
    try:
        for line in Path(path).read_text(errors="replace").splitlines():
            try:
                row = json.loads(line)
            except ValueError:
                continue
            if isinstance(row, dict) and isinstance(row.get("utc_ms"), int):
                rows.append(row)
    except OSError:
        pass
    return sorted(rows, key=lambda row: row["utc_ms"])


def root_sessions(records, pane, bootstrap_root):
    """A recipient's root sessions: its queue bootstrap root when known, else the sessions of its pane's SessionStart
    records with source startup or resume."""
    if bootstrap_root:
        return {bootstrap_root}
    return {r["session"] for r in records if r.get("pane") == pane and r.get("event") == "SessionStart"
            and r.get("source") in ("startup", "resume") and r.get("session")}


def root_transcript_verdict(records, root_sessions):
    """The (event, source) pairs the root sessions' hooks reported, in first-seen order, and those whose
    transcript_path was null. `defect` is true when any root event carried a null path (reported, never a row verdict)."""
    events, nulls = [], []
    for record in records:
        if record.get("session") not in root_sessions:
            continue
        pair = (record.get("event"), record.get("source"))
        if pair not in events:
            events.append(pair)
        if record.get("transcript_path") == "null" and pair not in nulls:
            nulls.append(pair)
    return {"events": events, "null_events": nulls, "defect": bool(nulls)}


def root_transcript_line(name, verdict):
    """The report.md line of one recipient's run-level root transcript fact."""
    pair = lambda event, source: f"{event}/{source}" if source else str(event)
    seen = ", ".join(pair(*p) for p in verdict["events"]) or "none"
    nulls = ", ".join(pair(*p) for p in verdict["null_events"]) or "none"
    return f"root transcript {name}: events {seen}; root null transcript_path: {nulls}"


def settled_by_root(wake_state, trace, root, sides, sent_ms, *, busy_side=False):
    """True when the wake was settled by the root session's own contact: the final wake state (read at least
    UNRECEIVED_VERDICT_S after the send) is not `unreceived`, the trace shows a root-session event after `sent_ms`,
    and no side-session event lies between `sent_ms` and that root event (side-only contact must not settle it).
    `busy_side` (side-queue-focused): the side runs its own turn through the send, so its tool and turn events in that
    window are its own work, not contact that could settle the wake; only the root's event after the send counts."""
    if (wake_state or {}).get("last_outcome") == "unreceived":
        return False
    events = sorted((r for r in trace if isinstance(r.get("utc_ms"), int)), key=lambda r: r["utc_ms"])
    first_root = next((r["utc_ms"] for r in events if r.get("session") == root and r["utc_ms"] >= sent_ms), None)
    if first_root is None:
        return False
    return busy_side or not any(r.get("session") in sides and sent_ms <= r["utc_ms"] <= first_root for r in events)


def approval_bypass(codex_args):
    """The Codex approval flag in use: `--approve-for-me`, the container's bypass, else the args as given."""
    for flag in ("--dangerously-bypass-approvals-and-sandbox", "--approve-for-me"):
        if flag in codex_args.split():
            return flag
    return codex_args


def _config_calls(calls):
    """(index, human, key, value) of each `config seat set` call, `human` when the human namespace word precedes it."""
    found = []
    for i, argv in enumerate(calls):
        for j in range(len(argv) - 2):
            if argv[j:j + 3] == ["config", "seat", "set"]:
                rest, positional, skip = argv[j + 3:], [], False
                for word in rest:
                    if skip:
                        skip = False
                    elif word in ("--seat", "--state-dir", "--host-endpoint"):
                        skip = True
                    elif not word.startswith("-"):
                        positional.append(word)
                found.append((i, "human" in argv[:j], positional[0] if positional else None,
                              positional[1] if len(positional) > 1 else None))
                break
    return found


def config_prose_verdict(calls, config_value):
    """config-prose's verdict from the pane's logged shim calls (argv lists) and the store's value: a `config seat list`
    before a `human config seat set` naming programmatic_wake and `on`, and the store reading it on."""
    listed = next((i for i, argv in enumerate(calls)
                   if any(argv[j:j + 3] == ["config", "seat", "list"] for j in range(len(argv)))), None)
    sets = [i for i, human, key, value in _config_calls(calls) if human and key == "programmatic_wake" and value == "on"]
    if listed is None:
        reason = "no_list"
    elif not sets:
        reason = "no_human_set"
    elif not any(i > listed for i in sets):
        reason = "no_list"
    elif config_value != "on":
        reason = "not_on"
    else:
        reason = None
    return {"passed": reason is None, "reason": reason}


def flag_flip_action(first_phase):
    """What the driver does on first sight of a new queue intent: flip the flag only while it is `prepared`."""
    return "flip" if first_phase == "prepared" else "miss"


def flag_flip_verdict(trials):
    """flag-flip's verdict over its trials: one trial that flipped the flag, delivered the item and either passed
    `stale_authority_native` or never armed any intent (`never_armed`: the flip landed before the arm re-read and no
    queue wake was possible, so the delivery was native) passes; all window misses fail `flip_window_missed`; else the
    first flipped trial's failure. An intent that armed or accepted after the flip is never `never_armed`."""
    if not trials:
        return {"passed": False, "reason": "no_trials"}
    flipped = [t for t in trials if t.get("flipped")]
    for trial in flipped:
        if ((trial.get("stale") or {}).get("passed") or trial.get("never_armed")) and trial.get("delivered"):
            return {"passed": True, "reason": None}
    if not flipped:
        return {"passed": False, "reason": "flip_window_missed"}
    first = flipped[0]
    if not ((first.get("stale") or {}).get("passed") or first.get("never_armed")):
        return {"passed": False, "reason": "stale_authority_native_failed",
                "detail": list((first.get("stale") or {}).get("reasons", ()))}
    return {"passed": False, "reason": "not_delivered"}


# `wake_work.last_outcome` values a pre-send composer refusal records (src/store/wake.rs, the completion
# disposition: Unsafe, Unavailable and TimedOut refusals) and restores `retry_step` to its pre-attempt value.
REFUSED_OUTCOMES = ("unsafe", "unavailable", "timed_out")


def side_defer_refusal(before, state, sent_ms):
    """True when `state` (the daemon's `wake_work` row) shows a native attempt refused after `sent_ms` with
    `retry_step` unchanged from its pre-send value in `before` (None: the seat had no row, step 0). A refusal
    restores the step; an advanced step is some other path (a submitted attempt), not a deferral."""
    if not state or state.get("last_outcome") not in REFUSED_OUTCOMES:
        return False
    if (state.get("completed_at_utc") or 0) < sent_ms:
        return False
    return (state.get("retry_step") or 0) == ((before or {}).get("retry_step") or 0)


def side_defer_passed(setup_ok, refused_while_side, typed_while_side, woke_after_close, delivered):
    """side-native-defer's verdict: a refused native attempt was observed while the side chat was displayed, nothing
    was typed into it, and the root woke after it closed and got the item."""
    return bool(setup_ok and refused_while_side and not typed_while_side and woke_after_close and delivered)


def archive_dialog_open(screen):
    """True when `screen` shows Codex's `Archive ...?` confirmation (its answer to Ctrl+C on an idle /side chat): the
    side chat is still displayed behind it, so the root is not a wake target yet."""
    return "Archive" in screen and "Cancel" in screen


SIDE_CLOSE_ATTEMPTS = 8  # a hookless side chat ignored three Ctrl+C in 9 s, then answered one a minute later
SIDE_CLOSE_RETRY_S = 5.0
SIDE_COMPOSER_PLACEHOLDER = "Ask a follow-up question"


def side_chat_displayed(screen):
    """True when `screen` shows a /side chat's own empty composer (its placeholder): the side chat is still the
    displayed view, so a close has not finished."""
    return SIDE_COMPOSER_PLACEHOLDER in screen


SIDE_ANSWER = re.compile(r"^[^\w\n]*SIDE-[A-Z]+-DONE", re.M)


def side_view_state(screen):
    """What a displayed side chat's pane shows that a wake could change: its prompt/composer rows (`› ...`) and its
    printed `SIDE-*-DONE` answers, plus its count of wake markers. Status and footer rows are not compared.
    In Codex's side-by-side layout (a `│` divider with the side composer's `›` right after it) the rows are read per
    pane: the root pane's prompt rows and the side pane's composer row, never the side pane's history, which the TUI
    repaints late (a `SIDE-*-DONE` answer already printed before the send can appear in a later capture)."""
    lines = screen.splitlines()
    if not any("\u2502\u203a" in line for line in lines):
        return {"prompts": [l.rstrip() for l in lines if l.startswith("\u203a ")],
                "answers": SIDE_ANSWER.findall(screen), "wake_markers": len(WAKE_PATTERN.findall(screen))}
    prompts, answers = [], []
    for line in lines:
        left, _, right = line.partition("\u2502")
        if left.startswith("\u203a ") or left.rstrip() == "\u203a":
            prompts.append(left.rstrip())
        if right.startswith("\u203a"):
            prompts.append(right.rstrip())
        answers += SIDE_ANSWER.findall(left)
    return {"prompts": prompts, "answers": answers, "wake_markers": len(WAKE_PATTERN.findall(screen))}


def side_pane_verdict(before, after):
    """(side_typed, side_view_changed) from two captures of the displayed side chat: a wake marker that was not there
    before, and any other change to its prompt rows or answers."""
    a, b = side_view_state(before), side_view_state(after)
    typed = b["wake_markers"] > a["wake_markers"]
    return typed, typed or a["prompts"] != b["prompts"] or a["answers"] != b["answers"]


def side_draft_passed(setup_ok, woke, checks):
    """side-draft's verdict, the same in both queue modes: the draft is never typed into or submitted (no wake) and
    the sender's overdue warning names the refused wake. Under --queue-proof the typed root draft refuses the queue
    lane too (the amended admission: any typed text refuses, as root-draft-queue judges), so no wake is owed
    and the queue evidence expects no effect (ht-z41e.54; the earlier `Accepted despite a draft` reading predated it)."""
    base = bool(setup_ok and checks.get("draft_intact") and checks.get("delivered"))
    return base and not woke and checks.get("warning_wake") not in (None, "submitted")


# A Claude tool-call line (`⏺ Bash(...)`, `⏺ Read(...)`).
TOOL_CALL = re.compile(r"^\s*⏺\s*[A-Z][A-Za-z]*\(")


def wake_turn(screen, mid):
    """The lines of the turn whose last `RECEIVED` line names `mid`, from its prompt (the wake text, prompt glyph
    stripped) through that line, when the prompt was a wake; else []."""
    lines = screen.splitlines()
    end = max((i for i, l in enumerate(lines) if "RECEIVED" in l and mid in l), default=None)
    if end is None:
        return []
    for i in range(end, -1, -1):
        text = lines[i].lstrip()
        if text.startswith(("›", "❯")):
            prompt = text.lstrip("›❯ ").strip()
            return [prompt] + [l.strip() for l in lines[i + 1:end + 1]] if is_wake_line(prompt) else []
    return []


def received_in_wake_turn(screen, mid):
    """Whether the last `RECEIVED` line for `mid` belongs to a turn whose prompt was a wake marker: the nearest
    composer prompt line (`›` or `❯`) above it is a native wake, or Claude's plugin label is followed
    by `wake up` (the pre-upgrade prefixed form also counts). A `/side` turn's
    own display of the message is not a wake of the main session, and its prompt is the side instruction."""
    prompt = None
    hit = False
    for line in screen.splitlines():
        text = line.lstrip()
        if text.startswith(("›", "❯")):
            prompt = text
        elif prompt is not None and "The herdr-threads plugin sent a message:" in prompt:
            if text in (CHANNEL_WAKE_UP, "herdr-threads: wake up"):
                prompt = text
            elif "RECEIVED" in text and mid in text:
                hit = False
        elif "RECEIVED" in text and mid in text:
            hit = prompt is not None and is_wake_line(prompt)
    return hit
# Session variables and agent env markers (every `AgentEnvMarker` the built-in adapters declare and every
# caller shape's session variables, which refuse `human me init`; the self-test checks this list against
# src/harness) of whatever runs this script: never inherited by the commands it runs.
CALLER_SESSION_ENV = ("CLAUDE_CODE_SESSION_ID", "CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT", "CLAUDE_PID",
                      "CODEX_SESSION_ID", "CODEX_THREAD_ID", "CODEX_SANDBOX", "CODEX_SANDBOX_NETWORK_DISABLED",
                      "HERMES_SESSION_ID", "ANTIGRAVITY_CONVERSATION_ID", "ANTIGRAVITY_AGENT",
                      "PI_CODING_AGENT", "OPENCODE", "OPENCODE_PID", "HERDR_THREADS_OPENCODE_SESSION",
                      "HERDR_THREADS_OPENCODE_PARENT", "HERDR_THREADS_OPENCODE_PID")
DIALOG = re.compile(r"Trust this folder\?|trust (the files in )?this folder|Do you trust|Allow .* to run|"
                    r"Do you want to proceed\?|Would you like to run", re.I)
# The linux mode's startup screens, matched on the whole visible screen (macOS keeps DIALOG on the last 400
# characters). "Allow" stays capitalized: the auto-mode notice a scratch Claude shows says "... mistakes that allow
# harmful commands to run". The rest are Claude Code's first-run screens (captured on 2.1.296 in a scratch config:
# scripts/wake_e2e_fixtures/): theme picker, folder trust, custom API key approval, login method.
LINUX_DIALOG = re.compile(r"Trust this folder\?|trust (the files in )?this folder|Do you trust|(?-i:Allow) .* to run|"
                          r"Do you want to proceed\?|Would you like to run|"
                          r"Choose the text style that looks best|Quick safety check: Is this a project|"
                          r"Do you want to use this API key\?|Detected a custom API key", re.I)
# A notice Claude Code (2.1.301) raises over the composer between turns, not at startup: while it is up, every
# turn the plugin submits (a wake) or the driver prompts is swallowed. The driver cancels it (Esc) whenever it
# waits for a settled recipient; it is Claude's onboarding UI, not a herdr-threads outcome.
HARNESS_NOTICE = re.compile(r"Teach auto mode about your environment\?")
LINUX_LOGIN = re.compile(r"Not logged in|Please run /login|Select login method")
# A harness that refuses its credential at startup and exits (Codex 0.163.0 on 2026-10-28, ht-8f0g: "account/read
# failed: workspace routing discovery unauthorized (401)"): no agent is ever detected, so the start fails at once.
AUTH_REJECTED = re.compile(r"account/read failed|unauthorized \(401\)")
HARNESS_MODES = ("macos", "linux")
CODEX_SANDBOXED_ARGS = "--approve-for-me"
CODEX_BYPASS_ARGS = "--dangerously-bypass-approvals-and-sandbox"
# Set by scripts/linux-e2e/in-container.sh in the runner container's Herdr server environment.
CONTAINER_MARKER = "HT_LINUX_E2E_CONTAINER"
DOCKERENV = Path("/.dockerenv")
CLAUDE_CREDENTIAL_ENV = ("CLAUDE_CODE_OAUTH_TOKEN", "ANTHROPIC_API_KEY")
HARNESSES = ("claude", "codex", "agy", "hermes", "opencode")
# What a Hermes 0.22.1+ pane's client must send its gateway session for the session to take that pane's seat.
HERMES_PASSTHROUGH = ("HERDR_PANE_ID", "HERDR_ENV")
# An inherited Hermes root or profile (any Hermes session exports HERMES_HOME) would point `setup hermes` and every
# `hermes --profile default` command at the operator's real Hermes, not the scratch home.
HERMES_ROOT_ENV = ("HERMES_HOME", "HERMES_PROFILE")
DEFAULT_HARNESSES = "claude,codex"
SHORT_NAMES = {"opencode": "oc"}  # recipient name prefix; the others use the harness's first two letters
# agy (opt-in): its model credential, and the optional endpoint override the agy relay uses.
AGY_CREDENTIAL_ENV = "GEMINI_API_KEY"
AGY_ENDPOINT_ENV = "GOOGLE_GEMINI_BASE_URL"
# opencode (opt-in): its OpenRouter credential (never handed to a helper process, see Run.run) and model.
OPENCODE_CREDENTIAL_ENV = "OPENROUTER_API_KEY"
OPENCODE_MODEL = "openrouter/deepseek/deepseek-v4-pro"
OPENCODE_SMALL_MODEL = "openrouter/deepseek/deepseek-v4-flash"
# opencode's TUI draws a user turn (a typed prompt or the plugin's wake) and a tool call as `┃`-bordered blocks; its
# composer is the last such block, closed by a `╹▀▀▀` rule, whose last row is the agent/model line.
OPENCODE_BORDER = "┃"
OPENCODE_COMPOSER_END = "╹▀"
OPENCODE_PLACEHOLDER = "Ask anything"  # the home-screen composer's placeholder
OPENCODE_SETTLE_S = 5.0


def opencode_config(model):
    """The scratch global opencode.json: the OpenRouter model, no update or share traffic, and every shell command
    and outside directory allowed, so no permission dialog can hold a recipient's turn (setup's own consented
    `herdr-threads *` / `ht *` grant is added by the plugin's config hook on top)."""
    return {"$schema": "https://opencode.ai/config.json", "model": model, "small_model": OPENCODE_SMALL_MODEL,
            "autoupdate": False, "share": "disabled",
            "permission": {"bash": {"*": "allow"}, "external_directory": "allow"}}


def opencode_composer(screen):
    """The text in opencode's composer on a visible screen ('' when empty), or None when no composer is drawn: the
    `┃` rows directly above the last `╹▀` rule, without the last one (the agent/model line)."""
    lines = screen.splitlines()
    end = max((i for i, l in enumerate(lines) if l.lstrip().startswith(OPENCODE_COMPOSER_END)), default=None)
    if end is None:
        return None
    block = []
    for line in reversed(lines[:end]):
        if not line.lstrip().startswith(OPENCODE_BORDER):
            break
        block.append(line.strip()[len(OPENCODE_BORDER):].strip())
    if not block:
        return None
    return " ".join(t for t in reversed(block[1:]) if t)


def opencode_transcript(db, sessions, output_lines=20):
    """What opencode's TUI shows of `sessions` (ids), rebuilt from its opencode.db read-only, in message order: a
    user text part as a `┃`-bordered block, a tool call as `┃  $ <command>` plus its first output lines, assistant
    text indented. Synthetic parts (the plugin's appended pages, the hook's check-in context) are left out, as the
    TUI hides them, so an id seen here was printed by the model or a tool. '' without a readable db."""
    if not sessions or not Path(db).is_file():
        return ""
    marks = ",".join("?" * len(sessions))
    try:
        with contextlib.closing(sqlite3.connect(Path(db).resolve().as_uri() + "?mode=ro", uri=True, timeout=5)) as c:
            rows = c.execute(
                f"SELECT m.data, p.data FROM part p JOIN message m ON m.id = p.message_id WHERE p.session_id IN "
                f"({marks}) ORDER BY m.time_created, m.id, p.time_created, p.id", list(sessions)).fetchall()
    except sqlite3.Error:
        return ""
    out = []
    for mdata, pdata in rows:
        try:
            role, part = json.loads(mdata).get("role"), json.loads(pdata)
        except (ValueError, AttributeError):
            continue
        if not isinstance(part, dict) or part.get("synthetic"):
            continue
        kind = part.get("type")
        if kind == "text" and role == "user":
            out += ["", f"  {OPENCODE_BORDER}"] + [f"  {OPENCODE_BORDER}  {l}" for l in str(part.get("text", "")).splitlines()]
            out.append(f"  {OPENCODE_BORDER}")
        elif kind == "text":
            out += [""] + [f"     {l}" for l in str(part.get("text", "")).splitlines()]
        elif kind == "tool":
            state = part.get("state") if isinstance(part.get("state"), dict) else {}
            inp = state.get("input") if isinstance(state.get("input"), dict) else {}
            what = inp.get("command") or f"{part.get('tool')} {json.dumps(inp)[:200]}"
            body = str(state.get("output") or "").splitlines()[:output_lines]
            out += ["", f"  {OPENCODE_BORDER}  $ {what}"] + [f"  {OPENCODE_BORDER}  {l}" for l in body]
    return "\n".join(out) + ("\n" if out else "")


def opencode_ledger(path):
    """The plugin's ledger entries (HERDR_THREADS_MOD_LEDGER, one JSON object per line); [] before any."""
    try:
        text = Path(path).read_text(errors="replace")
    except OSError:
        return []
    out = []
    for line in text.splitlines():
        try:
            entry = json.loads(line)
        except ValueError:
            continue
        if isinstance(entry, dict):
            out.append(entry)
    return out


def evidence_leaks(ev, values):
    """Files under `ev` that hold any of `values` (a credential and its tails) or an OpenRouter-shaped token."""
    leaks = []
    for path in sorted(p for p in Path(ev).rglob("*") if p.is_file()):
        try:
            text = path.read_text(errors="replace")
        except OSError:
            continue
        if any(v and v in text for v in values) or OPENROUTER_TOKEN.search(text):
            leaks.append(path)
    return leaks


def scrub_evidence(ev, values):
    """Redact every evidence file evidence_leaks names; returns their names (never their contents)."""
    leaks = evidence_leaks(ev, values)
    for path in leaks:
        path.write_text(redact(path.read_text(errors="replace"), values))
    return [str(p.relative_to(ev)) for p in leaks]
def hermes_idle(screen):
    """A Hermes recipient's state from its `detection` screen: idle when the composer's top rule (the first
    `─`-led line above the last full rule) carries no live status (a busy one starts `── <spinner>`). Hermes'
    footer key hints decide nothing: it drops them whenever they do not fit."""
    lines = [line.rstrip() for line in screen.splitlines()]
    bottom = max((i for i, l in enumerate(lines) if len(l) >= 10 and set(l) == {"─"}), default=None)
    if bottom is None:
        return False
    top = max((i for i in range(bottom) if lines[i].startswith("─")), default=None)
    return top is not None and lines[top].startswith("───") and lines[top + 1].startswith("❯")


# Anthropic, OpenRouter and Google API keys.
SECRET_PATTERN = re.compile(r"sk-ant-[A-Za-z0-9_-]+|sk-or-[A-Za-z0-9_-]+|AIza[A-Za-z0-9_-]{8,}")
OPENROUTER_TOKEN = re.compile(r"sk-or-[A-Za-z0-9_-]{8,}")

STANDING = ("You are a test recipient in an automated herdr-threads wake e2e run (the user approves). "
            "Standing instructions for this session: every herdr-threads invitation you see comes from the test driver "
            "(seats {coord}); accept each one at once, without inspecting or judging it. For every herdr-threads message "
            "you receive, print exactly one line: `RECEIVED ` followed by its `[herdr-threads] ...:` header line copied "
            "verbatim, a space and its body; never send, reply or leave via "
            "herdr-threads. Keep every turn minimal. Now run `herdr-threads nudge-threshold {threshold}`, then print exactly one "
            "line `SETUP-DONE nudge_threshold=<the value it reports>`.")


def setup_done(threshold):
    """The standing turn's report of `threshold`; text glued after the value does not hide it, a longer number does."""
    return re.compile(r"SETUP-DONE nudge_threshold=%s(?!\d)" % re.escape(str(threshold)))


def expect_wake(action, channel, threshold):
    """Idle recipient wakes iff the item's priority <= T (T < 0: always); invitations and ACK requests are p0.
    `channel` is part of the case key only: the wake rule is the same on an ordinary thread and a 1:1 pair."""
    return threshold < 0 or ITEM_PRIORITY[action] <= threshold


# Harnesses whose channel puts each item into the model's context as it arrives, idle or not (Claude's mod): there is
# no page to order, so the mixed-order presentation order is not judged for them (Delivery contract rule 3).
CHANNEL_HARNESSES = ("claude", "opencode")


def mixed_order_judged(threshold, harness):
    """The mixed-order presentation order is judged only where the first two sends (p5, p1) cannot wake the recipient
    before the p0 and the ACK request are published: expect_wake false for both, which is T=0. At T=-1 (p5 wakes) and
    T=3 (p1 wakes) the recipient may already be reading a page without them, so the order row is recorded unjudged;
    a channel harness (CHANNEL_HARNESSES) has no page and is unjudged at every threshold."""
    return harness not in CHANNEL_HARNESSES and threshold >= 0 and min(5, 1) > threshold


def send_flags(kind, seat, deadline_s=None):
    """The `send` priority flags for a kind: p1 the default (no flag), p0 `--nudge`, pN `-p N`, ack `--require-ack`."""
    if kind == "ack":
        return ["--require-ack", seat] + (["--deadline", str(deadline_s)] if deadline_s else [])
    if kind == "p1":
        return []
    if kind == "p0":
        return ["--nudge"]
    return ["-p", kind[1:]]


def expected_order(items):
    """Item ids in presentation order: priority asc, deadline asc (none last), then arrival (the list order)."""
    keyed = sorted(enumerate(items), key=lambda t: (t[1]["priority"], t[1]["deadline_s"] is None,
                                                     t[1]["deadline_s"] or 0, t[0]))
    return [item["id"] for _, item in keyed]


def screen_order(screen, ids):
    """The ids that appear on the screen, in order of first appearance."""
    seen = sorted((screen.find(i), i) for i in ids if i in screen)
    return [i for _, i in seen]


# A frame header anywhere on a line (recipients echo it after `RECEIVED `): kind, id, then `in <thread label> from
# <sender label>[ markers]:`; labels may hold spaces, so the markers are read after ` from ` up to the first `: `.
HEADER = re.compile(r"\[herdr-threads\] \S+ (\S+) in .*? from ([^\n]*)")


def header_priorities(screen):
    """{message id: priority} from the frame headers on the screen: a `[pN]` marker, else the silent default p1."""
    out = {}
    for m in HEADER.finditer(screen):
        rest = m.group(2)
        if ":" not in rest:
            # Claude's TUI breaks a long header with a real newline (soft wraps are already undone),
            # so the markers and the closing colon can start the next line.
            following = screen[m.end():].lstrip("\n").split("\n", 1)[0].strip()
            if following.startswith("["):
                rest += " " + following
        sender_and_markers = rest.split(": ", 1)[0].rstrip(":")
        marker = re.search(r"\[p(\d)\]", sender_and_markers)
        out.setdefault(m.group(1), int(marker.group(1)) if marker else 1)
    return out


def paged_order(screen, ids):
    """The ids in order of first appearance, one list per wake (split at each WAKE_MARKERS line): each wake pulls
    its own page, so presentation order is judged within a page, never across two wakes."""
    pages, seen = [], set()
    for chunk in WAKE_PATTERN.split(screen):
        page = [i for i in screen_order(chunk, ids) if i not in seen]
        seen.update(page)
        if page:
            pages.append(page)
    return pages


def paged_order_ok(pages, want):
    """Every page lists its items in the expected relative order."""
    return all(page == [i for i in want if i in page] for page in pages)


def markers_ok(shown, expected):
    """At least one of the case's messages shows its frame header on screen, and every shown header carries its
    message's priority marker (an expected id with no header is not judged; no header at all fails, so a PASS
    means a marker was actually seen)."""
    judged = [mid for mid in shown if mid in expected]
    return bool(judged) and all(shown[mid] == expected[mid] for mid in judged)


def markers_reason(case, shown, expected):
    """Why markers_ok failed for `case`, or None when it passed."""
    judged = [mid for mid in shown if mid in expected]
    if not judged:
        return f"{case}: no expected [herdr-threads] header on screen"
    bad = [f"{mid} shown p{shown[mid]} expected p{expected[mid]}" for mid in judged if shown[mid] != expected[mid]]
    return f"{case}: " + "; ".join(bad) if bad else None


def utc():
    return datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def short_tmp_root(platform=None):
    """The run root's parent: a short system temp dir that exists on this platform. macOS keeps /private/tmp
    (what /tmp resolves to there, so the paths the driver records and scans for processes are canonical) rather
    than the long per-user $TMPDIR under /var/folders; elsewhere /tmp, as Linux has no /private/tmp (and creating
    one would move the daemon's runtime socket dir, ht-3yi). $TMPDIR is deliberately not consulted."""
    return Path("/private/tmp") if (platform or sys.platform) == "darwin" else Path("/tmp")


def default_harness_mode(platform=None):
    """macos where the keychain holds Claude's credentials, linux (scratch config, env credentials) elsewhere."""
    return "macos" if (platform or sys.platform) == "darwin" else "linux"


def in_runner_container(env, dockerenv=None):
    """Both the runner's marker and Docker's /.dockerenv: a stray environment variable alone is not enough."""
    return env.get(CONTAINER_MARKER) == "1" and (dockerenv or DOCKERENV).exists()


def marker_without_dockerenv(harness_mode, env, dockerenv=None):
    """The runner's marker is set but /.dockerenv is missing (e.g. podman, which writes /run/.containerenv): the
    bypass stays off, so say why instead of letting the busy cases fail later on a bubblewrap error."""
    return harness_mode == "linux" and env.get(CONTAINER_MARKER) == "1" and not (dockerenv or DOCKERENV).exists()


def default_codex_args(harness_mode, env, dockerenv=None):
    """Codex's sandbox is bypassed only in the linux mode inside the runner container, where bubblewrap cannot run
    under Docker's default seccomp profile and the container is the boundary. Everywhere else (macOS, a bare Linux
    workstation) Codex keeps its workspace-write sandbox with automatic review."""
    if harness_mode == "linux" and in_runner_container(env, dockerenv):
        return CODEX_BYPASS_ARGS
    return CODEX_SANDBOXED_ARGS


def secret_values(env):
    """What redact() hides: each Claude, agy or opencode credential value from the environment, and its 20- and
    12-character tails (the approval suffix the linux mode seeds, and what Claude's API-key screen shows of a key)."""
    out = []
    for var in (*CLAUDE_CREDENTIAL_ENV, AGY_CREDENTIAL_ENV, OPENCODE_CREDENTIAL_ENV):
        value = (env.get(var) or "").strip()
        if value:
            out += [value] + [value[-n:] for n in (20, 12) if len(value) > n]
    return tuple(sorted(set(out), key=len, reverse=True))


def redact(text, secrets=()):
    """Evidence text without credentials: every `sk-ant-` token and every value of secret_values()."""
    for value in secrets:
        text = text.replace(value, "<redacted>")
    return SECRET_PATTERN.sub("<redacted>", text)


LONG_RUN = re.compile(r"[A-Za-z0-9_-]{8,}")


def capture_evidence(text, lines, secrets=()):
    """A pane capture for evidence. Herdr bounds a capture by whole lines, so when it is full its first line may be
    the cut-off end of a wrapped credential that redact() can no longer recognize: long token-like runs on that line
    only are masked."""
    text = redact(text, secrets)
    rows = text.split("\n")
    if len(rows) >= lines:
        rows[0] = LONG_RUN.sub("<masked>", rows[0])
    return "\n".join(rows)


def clip(text, n, secrets=None, tail=False):
    """Bounded diagnostic text: redacted whole first (a credential cut by the crop would leave an unrecognizable
    fragment), then its first (or, with tail, last) n characters. secrets None: this process's environment."""
    text = redact(str(text), secret_values(os.environ) if secrets is None else secrets)
    return text[-n:] if tail else text[:n]


def claude_scratch_state(cwd, env):
    """The `.claude.json` keys a scratch CLAUDE_CONFIG_DIR needs so Claude Code opens straight at its composer
    (measured on 2.1.296, scripts/wake_e2e_fixtures/): onboarding done (it holds the theme picker and the login
    screen), the agents' working directory trusted (folder-trust screen), and, only with ANTHROPIC_API_KEY, that key
    approved the way Claude records an approval: its last 20 characters (custom-API-key screen). An OAuth token from
    CLAUDE_CODE_OAUTH_TOKEN needs no approval."""
    state = {"hasCompletedOnboarding": True, "projects": {cwd: {"hasTrustDialogAccepted": True}}}
    key = (env.get("ANTHROPIC_API_KEY") or "").strip()
    if key:
        state["customApiKeyResponses"] = {"approved": [key[-20:]], "rejected": []}
    return state


def seed_claude_config(cfg_dir, cwd, env):
    """Merge claude_scratch_state into <cfg_dir>/.claude.json (a private 0600 regular file, never through a link)."""
    path = Path(cfg_dir) / ".claude.json"
    doc = {}
    if path.is_file() and not path.is_symlink():
        try:
            doc = json.loads(path.read_text())
        except ValueError as e:
            raise RuntimeError(f"{path} is not valid JSON ({e}); remove or fix it") from None
        if not isinstance(doc, dict):
            raise RuntimeError(f"{path} does not hold a JSON object; remove or fix it")
    state = claude_scratch_state(cwd, env)
    projects = dict(doc.get("projects") or {})
    projects[cwd] = dict(projects.get(cwd) or {}, **state.pop("projects")[cwd])
    doc.update(state, projects=projects)
    if path.is_symlink() or path.exists():
        path.unlink()
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "w") as f:
        json.dump(doc, f, indent=2)
    return path


def agy_settings(cwd):
    """The agy gate's seed settings.json (scripts/agy-sandbox/agyenv.py settings_dict), for the agents' cwd, plus
    `command(echo)`: a Gemini recipient prints its RECEIVED/SETUP-DONE lines through run_command echo, and agy
    would hold that turn at a permission prompt (seen on 1.4.6). setup adds its own `command(herdr-threads)`."""
    return {"altScreenMode": "never", "modelProvider": "gemini", "trustedWorkspaces": [cwd],
            "permissions": {"allow": ["command(echo)"]}}


def seed_agy_home(home, cwd):
    """Write agy's settings.json under the scratch HOME before setup merges its owned permission member into it."""
    path = Path(home) / ".gemini" / "antigravity-cli" / "settings.json"
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    write_private(path, json.dumps(agy_settings(cwd)) + "\n")
    return path


def agy_env(env):
    """The (name, value) pairs the agy pane gets from the driver's environment: the credential and, when set, the
    endpoint override."""
    return [(k, env[k].strip()) for k in (AGY_CREDENTIAL_ENV, AGY_ENDPOINT_ENV) if (env.get(k) or "").strip()]


def claude_env(env):
    """The (name, value) pairs of the Claude credential variables set in the driver's environment."""
    return [(k, env[k].strip()) for k in CLAUDE_CREDENTIAL_ENV if (env.get(k) or "").strip()]


def write_private(path, text):
    """A new private 0600 regular file, never written through a link."""
    path = Path(path)
    if path.is_symlink() or path.exists():
        path.unlink()
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "w") as f:
        f.write(text)


AGY_FOOTER = "? for shortcuts"
AGY_COLOR_PAGE = "Choose your color scheme"
AGY_TERMS_PAGE = "Terms of Service"
AGY_CONSENT = "] Yes, I agree"


AGY_ONBOARDING_KEYS = {"color": [("enter",)], "consent-ticked": [("enter",)],
                       "terms": [("down", "right"), ("enter",)]}


def agy_onboarding_page(screen):
    """agy's current first-run page: `color`, `consent-ticked`, `terms` (consent unticked, ready for Done), or None
    (composer drawn, or nothing recognised, including a consent box in an unknown state: Done is never pressed then).
    With altScreenMode never, earlier pages stay on screen above the current one, so the page that counts is the
    one drawn lowest. The pre-ticked consent to Google collecting interaction data is unticked (Enter toggles it)
    before Done: the e2e runs agy without the agy gate's no-network sandbox."""
    footer, color, terms = (screen.rfind(m) for m in (AGY_FOOTER, AGY_COLOR_PAGE, AGY_TERMS_PAGE))
    if footer > max(color, terms):
        return None
    if terms > color:
        box = screen.rfind(AGY_CONSENT)
        state = screen[box - 1] if box > color else None
        return {"x": "consent-ticked", " ": "terms"}.get(state)
    return "color" if color >= 0 else None


def agy_onboarding_keys(screen):
    """The keys that answer agy's current first-run page (agy_onboarding_page), or None."""
    return AGY_ONBOARDING_KEYS.get(agy_onboarding_page(screen))


def agy_composer_up(screen):
    """agy's composer is drawn after any first-run page (its footer hint is the lowest of them)."""
    return screen.rfind(AGY_FOOTER) > max(screen.rfind(AGY_COLOR_PAGE), screen.rfind(AGY_TERMS_PAGE))


def linux_startup_refusal(screen, status):
    """The linux mode: why a freshly started agent must not be prompted, or None. Checked on the whole visible
    screen: the theme picker reports `idle` and its prompt sits far above the screen's last 400 characters."""
    if AUTH_REJECTED.search(screen):
        return "auth rejected"
    if LINUX_LOGIN.search(screen):
        return "is not logged in (set CLAUDE_CODE_OAUTH_TOKEN or ANTHROPIC_API_KEY)"
    if status == "blocked" or LINUX_DIALOG.search(screen):
        return "startup dialog (nothing typed)"
    return None


class Refused(Exception):
    pass


class Stopped(Exception):
    """Cleanup has begun: no new helper process may start (except cleanup's own)."""


class Run:
    @property
    def observe_queue(self):
        return sys.platform.startswith("linux") or bool(getattr(self.args, "queue_proof", False))

    def __init__(self, args):
        self.args = args
        self.run_id = time.strftime("%H%M%S") + "-" + uuid.uuid4().hex[:4]
        self.root = Path(tempfile.mkdtemp(prefix="ht-wake-e2e.", dir=short_tmp_root()))
        os.chmod(self.root, 0o700)
        self.ev = (Path(args.evidence_dir) if args.evidence_dir else
                   self.root.parent / f"ht-wake-evidence.{self.run_id}")
        self.ev = self.ev.resolve()
        if self.ev.is_relative_to(self.root.resolve()) or self.root.resolve().is_relative_to(self.ev):
            shutil.rmtree(self.root)
            raise Refused("evidence directory must be separate from the scratch root and its parents")
        self.ev.mkdir(mode=0o700, parents=True, exist_ok=True)
        self.state = self.root / "state"
        self.home = self.root / "home"
        self.claude_cfg = self.root / "claude-config"
        self.claude_hooks_cfg = self.root / "claude-hooks-config"  # cl-hooks: `setup claude --hooks-only`
        self.codex_home = self.root / "codex-home"
        self.agy_env = self.root / "agy.env"
        self.opencode_root = self.root / "opencode"  # HOME's XDG dirs, TMPDIR, OPENCODE_CONFIG_DIR, the project dir
        self.opencode_env = self.root / "opencode.env"  # the OpenRouter credential, private, removed even with --keep
        self.claude_env = self.root / "claude.env"  # linux: the Claude credential, written only when one is set
        self.hermes_home = Path(args.hermes_home).resolve() if getattr(args, "hermes_home", None) else None
        self.bindir = self.root / "bin"
        self.bin = str(Path(args.bin).resolve())
        self.endpoint = os.environ.get("HERDR_SOCKET_PATH", "")
        self.cwd = str(REPO)
        self.lock = threading.Lock()
        self.cmdlog = (self.ev / "commands.jsonl").open("a", encoding="utf-8")
        self.results = []
        self.workspace = None
        self.daemon_attempted = False  # set before `daemon ensure`: an uncertain start is still ours to stop
        self.layout_attempted = False
        self.process_grace_s = 10.0
        self.stop_deadline_s = 20.0
        self.coords = {}  # "main" / "fresh" -> {"pane", "seat"}
        self.auth_digest = None
        self.auth_refreshed = None
        self.cleanup_report = None
        self.stopping = threading.Event()  # set (under self.lock) when cleanup begins
        self.children = set()              # in-flight helper processes, registered under self.lock
        self.workers = []
        self.start_attempted = set()  # recorded just before Thread.start: a mid-start interrupt is still joined
        self.drain_timeout_s = 30.0
        self.child_term_grace_s = 3.0
        self.codex_start = threading.Lock()  # one Codex start at a time (Recipient.start)
        self.agy_start = threading.Lock()    # one agy start at a time: the agy recipients share one scratch HOME
        self.secrets = secret_values(os.environ)  # redacted from every evidence file

    # ---------- process helpers ----------
    def run(self, argv, env=None, timeout=60, tag=None, cleanup=False):
        """Run one helper process. Every helper is registered while it runs, so cleanup can terminate it; once
        cleanup has begun only cleanup's own helpers may start (others raise Stopped)."""
        started = time.monotonic()
        # The opencode credential reaches no helper (herdr, the daemon, a version probe): only the panes, through
        # the private env file.
        env = {k: v for k, v in (os.environ if env is None else env).items() if k != OPENCODE_CREDENTIAL_ENV}
        with self.lock:  # admission and registration are atomic with cleanup's stop
            if self.stopping.is_set() and not cleanup:
                raise Stopped(f"cleanup has begun: not starting {argv[:3]}")
            p = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env,
                                 start_new_session=True)
            self.children.add(p)
        try:
            out, err = p.communicate(timeout=timeout)
            rc = p.returncode
        except subprocess.TimeoutExpired:
            kill_group(p, signal.SIGKILL)
            out, err = reap(p)
            rc = "timeout"
        except BaseException:  # Ctrl-C/SIGTERM in this thread: the helper (own session) must not outlive us
            kill_group(p, signal.SIGKILL)
            reap(p)
            raise
        finally:
            with self.lock:
                self.children.discard(p)
        out, err = out or "", err or ""
        with self.lock:
            secrets = getattr(self, "secrets", ())
            self.cmdlog.write(redact(json.dumps({"utc": utc(), "tag": tag, "argv": argv, "rc": rc,
                                                 "elapsed_s": round(time.monotonic() - started, 3),
                                                 "stdout": clip(out, 4096, secrets),
                                                 "stderr": clip(err, 4096, secrets)}), secrets) + "\n")
            self.cmdlog.flush()
        if self.stopping.is_set() and not cleanup:  # cut off by the drain: the case is interrupted, not failed
            raise Stopped(f"cleanup interrupted {argv[:3]}")
        return rc, out, err

    def ht(self, *argv, pane=None, human=False, env=None, tag=None, timeout=60, check=True):
        e = {k: v for k, v in os.environ.items() if k not in HERMES_ROOT_ENV}
        e.update(env or {})
        e.pop("HERDR_PANE_ID", None)  # never derive a seat from the controller's own pane
        if e.get("CLAUDE_CODE_PLUGIN_DIRS") == "":
            e.pop("CLAUDE_CODE_PLUGIN_DIRS")
        for var in CALLER_SESSION_ENV:  # nor a session from the agent that runs this script
            e.pop(var, None)
        if pane:
            e["HERDR_PANE_ID"] = pane
        prefix = ["--state-dir", str(self.state), "--host-endpoint", self.endpoint, "--json"]
        argv = list(argv)
        # A self-granting command takes its words first, every option after.
        # Person/operator actions take the immediate `human` namespace before the global options.
        full = [self.bin] + (argv + prefix if argv[:1] == ["setup"] else (["human"] if human else []) + prefix + argv)
        rc, out, err = self.run(full, env=e, timeout=timeout, tag=tag or "ht:" + " ".join(argv[:2]))
        if check and rc != 0:
            raise RuntimeError(f"herdr-threads {' '.join(argv[:3])}: rc={rc} {clip((err or out).strip(), 300, getattr(self, 'secrets', None))}")
        return out

    def ht_err(self, *argv, pane=None, human=False):
        """`ht` returning (stdout, stderr): stderr carries the sender-side wake warning."""
        prefix = ["--state-dir", str(self.state), "--host-endpoint", self.endpoint, "--json"]
        e = dict(os.environ)
        for var in ("HERDR_PANE_ID", *CALLER_SESSION_ENV):
            e.pop(var, None)
        if pane:
            e["HERDR_PANE_ID"] = pane
        full = [self.bin] + (["human"] if human else []) + prefix + list(argv)
        rc, out, err = self.run(full, env=e, tag="ht:" + " ".join(argv[:2]))
        if rc != 0:
            raise RuntimeError(f"herdr-threads {' '.join(argv[:3])}: rc={rc} {clip((err or out).strip(), 300, getattr(self, 'secrets', None))}")
        return out, err

    def herdr(self, *argv, tag=None, timeout=30, check=True):
        rc, out, err = self.run(["herdr", *argv], tag=tag or "herdr:" + " ".join(argv[:2]), timeout=timeout)
        if check and rc != 0:
            raise RuntimeError(f"herdr {' '.join(argv[:3])}: rc={rc} {clip((err or out).strip(), 300, getattr(self, 'secrets', None))}")
        try:
            return json.loads(out) if out.strip() else {}
        except ValueError:
            return {"raw": out}

    # ---------- setup ----------
    def preflight(self):
        problems = []
        if os.environ.get("HERDR_ENV") != "1":
            problems.append("must run inside a Herdr pane (HERDR_ENV!=1)")
        if not self.endpoint:
            problems.append("HERDR_SOCKET_PATH is unset")
        if not os.access(self.bin, os.X_OK):
            problems.append(f"herdr-threads binary not executable: {self.bin} (run scripts/build.sh)")
        if "hermes" in self.args.harness and not (self.hermes_home and os.access(self.hermes_bin(), os.X_OK)):
            problems.append("--harness hermes needs --hermes-home DIR with Hermes installed (DIR/.local/bin/hermes)")
        elif "hermes" in self.args.harness and self.hermes_gateways():
            problems.append(f"a Hermes gateway from {self.hermes_home} is already running (pids "
                            f"{self.hermes_gateways()}): stop it first (`hermes gateway stop` with HOME={self.hermes_home})")
        if getattr(self, "tmux_cases", False) and not shutil.which("tmux"):
            problems.append("the tmux-named cases need tmux on PATH")
        for tool in ("herdr", *(h for h in self.args.harness if h != "hermes")):
            if not shutil.which(tool):
                problems.append(f"{tool} not on PATH")
        auth = self.codex_auth_home() / "auth.json"
        if "codex" in self.args.harness and not auth.exists():
            problems.append(f"no Codex credentials at {auth} (--codex-auth-home)")
        if self.linux and "claude" in self.args.harness and not any((os.environ.get(v) or "").strip()
                                                                    for v in CLAUDE_CREDENTIAL_ENV):
            problems.append("no Claude credential in the environment: the linux harness mode needs "
                            "CLAUDE_CODE_OAUTH_TOKEN or ANTHROPIC_API_KEY (no keychain; the real Claude config is "
                            "never read)")
        if "agy" in self.args.harness and not (os.environ.get(AGY_CREDENTIAL_ENV) or "").strip():
            problems.append(f"no agy credential in the environment: --harness agy needs {AGY_CREDENTIAL_ENV} "
                            "(no Google sign-in is used)")
        if "opencode" in self.args.harness and not (os.environ.get(OPENCODE_CREDENTIAL_ENV) or "").strip():
            problems.append(f"no opencode credential in the environment: --harness opencode needs "
                            f"{OPENCODE_CREDENTIAL_ENV} (OpenRouter)")
        if problems:
            raise Refused("; ".join(problems))
        versions = {}
        for name, argv in (("herdr", ["herdr", "--version"]), ("herdr-threads", [self.bin, "--version"]),
                           # opencode's version is read in setup_opencode, under its scratch HOME and XDG dirs
                           *((h, [h, "--version"]) for h in self.args.harness if h not in ("hermes", "opencode"))):
            versions[name] = clip(self.run(argv, tag="version")[1].strip(), 120, getattr(self, 'secrets', None))
        if "hermes" in self.args.harness:
            out = self.run([str(self.hermes_bin()), "--version"], env=self.hermes_env(), tag="version")[1]
            versions["hermes"] = clip(out.strip().splitlines()[0] if out.strip() else "", 120, self.secrets)
        sha = self.run(["git", "-C", str(REPO), "rev-parse", "HEAD"], tag="git")[1].strip()
        dirty = bool(self.run(["git", "-C", str(REPO), "status", "--porcelain"], tag="git")[1].strip())
        self.facts = {"run_id": self.run_id, "sha": sha, "dirty": dirty, "versions": versions,
                      "harness_mode": self.args.harness_mode, "codex_args": self.args.codex_args,
                      "bin": self.bin, "bin_sha256": file_sha256(self.bin), "root": str(self.root),
                      "started_utc": utc(), "queue_proof": bool(getattr(self.args, "queue_proof", False)),
                      "queue_observation": self.observe_queue}
        print(f"wake-e2e {self.run_id}: sha {sha[:12]}{' (dirty)' if dirty else ''}, root {self.root}", flush=True)
        for k, v in versions.items():
            print(f"  {k}: {v}", flush=True)

    @property
    def calls_log(self):
        return self.bindir / "calls.log"

    def hermes_bin(self):
        return self.hermes_home / ".local" / "bin" / "hermes"

    def hermes_gateways(self):
        """Pids of running Hermes gateways (`... gateway run ...`) whose HOME is the scratch Hermes home: the
        recipients' hooks run there, and the gateway keeps the HERDR_PANE_ID of the pane that spawned it."""
        found = []
        for proc in Path("/proc").iterdir() if Path("/proc").is_dir() else ():
            if not proc.name.isdigit():
                continue
            try:
                argv = (proc / "cmdline").read_bytes().split(b"\0")
                env = (proc / "environ").read_bytes().split(b"\0")
            except OSError:
                continue
            if b"gateway" in argv and b"run" in argv and f"HOME={self.hermes_home}".encode() in env:
                found.append(int(proc.name))
        return found

    def hermes_env(self):
        """The environment of a Hermes command run by the driver: HOME at the scratch Hermes home, so its
        ~/.hermes and ~/.codex are the scratch ones (HERMES_ROOT_ENV dropped)."""
        env = {k: v for k, v in os.environ.items()
               if k not in CALLER_SESSION_ENV and k not in ("CODEX_HOME", "HERDR_PANE_ID", *HERMES_ROOT_ENV)}
        env.update(HOME=str(self.hermes_home), PATH=f"{self.hermes_bin().parent}:{env.get('PATH', '')}")
        return env

    def hermes_passthrough(self):
        """Add HERDR_PANE_ID and HERDR_ENV to the scratch default profile's terminal.client_env_passthrough, kept
        after the names already listed there: from Hermes 0.22.1 every pane's session runs in one gateway, and a
        session carries its own pane only through this list (integrations/hermes/README.md)."""
        key = "terminal.client_env_passthrough"
        hermes = [str(self.hermes_bin()), "--profile", "default", "config"]
        rc, out, err = self.run([*hermes, "get", key, "--json"], env=self.hermes_env(), tag="hermes:passthrough-get",
                                timeout=60)
        if rc == 0:
            try:
                names = json.loads(out)
            except ValueError:
                names = None
            if not isinstance(names, list):
                raise RuntimeError(f"hermes config get {key}: not a list: {clip(out.strip(), 200, self.secrets)}")
        elif "not set" in err:
            names = []
        else:
            raise RuntimeError(f"hermes config get {key}: rc={rc} {clip((err or out).strip(), 300, self.secrets)}")
        merged = names + [n for n in HERMES_PASSTHROUGH if n not in names]
        if merged == names:
            return
        rc, out, err = self.run([*hermes, "set", key, json.dumps(merged)], env=self.hermes_env(),
                                tag="hermes:passthrough-set", timeout=60)
        if rc != 0:
            raise RuntimeError(f"hermes config set {key}: rc={rc} {clip((err or out).strip(), 300, self.secrets)}")

    def codex_auth_home(self):
        return Path(self.args.codex_auth_home or os.environ.get("CODEX_HOME") or Path.home() / ".codex").expanduser()

    def scratch(self):
        for d in (self.state, self.home, self.claude_cfg, self.codex_home, self.bindir):
            d.mkdir(mode=0o700, parents=True, exist_ok=True)
        q = shlex.quote
        shim = self.bindir / "herdr-threads"
        # Pins the run's --state-dir/--host-endpoint unless a rendered ready command already names them.
        # Each call is logged (pane, argv, 0x1f-separated) for the extra-inbox check (inbox_calls), as one append per
        # record so concurrent recipients never interleave a line.
        shim.write_text("#!/bin/sh\n"
                        "rec=$(printf '%s\\037' \"${HERDR_PANE_ID:-${HERDR_THREADS_SEAT_NAME:-}}\" \"$@\")\n"
                        f"printf '%s\\n' \"$rec\" >> {q(str(self.calls_log))}\n"
                        "pin_state=1 pin_host=1\nfor word in \"$@\"; do\n  case \"$word\" in\n"
                        "    --state-dir|--state-dir=*) pin_state=0 ;;\n    --host-endpoint|--host-endpoint=*) pin_host=0 ;;\n"
                        "  esac\ndone\n"
                        f"if [ \"$pin_host\" = 1 ]; then set -- --host-endpoint {q(self.endpoint)} \"$@\"; fi\n"
                        f"if [ \"$pin_state\" = 1 ]; then set -- --state-dir {q(str(self.state))} \"$@\"; fi\n"
                        f"exec {q(self.bin)} \"$@\"\n")
        shim.chmod(0o700)
        # Disable both saved-machine and process discovery before the release binary starts.
        (self.state / "node").mkdir(mode=0o700)
        (self.state / "node/settings.json").write_text(json.dumps({
            "sync": {"discover": False},
        }) + "\n")
        (self.state / "node/settings.json").chmod(0o600)
        self.daemon_attempted = True
        self.ht("daemon", "ensure", tag="daemon:ensure", timeout=60)
        self.ht("daemon", "health", tag="daemon:health", timeout=20)
        setup_env = {"HOME": str(self.home), "CLAUDE_CONFIG_DIR": str(self.claude_cfg), "CODEX_HOME": str(self.codex_home),
                     "CLAUDE_CODE_PLUGIN_DIRS": ""}  # never copy a real installed mod into the scratch layer
        # cl-hooks first: `--hooks-only` removes the mod files of the shared state dir, which the full Claude setup
        # below then reinstalls for the other Claude recipients.
        if getattr(self, "hooks_cases", False) or (getattr(self, "tmux_cases", False) and "claude" in self.args.harness):
            self.claude_hooks_cfg.mkdir(mode=0o700, parents=True, exist_ok=True)
            self.ht("setup", "claude", "--hooks-only", "--with-permissions", tag="setup:claude:hooks-only",
                    env=dict(setup_env, CLAUDE_CONFIG_DIR=str(self.claude_hooks_cfg)), timeout=120)
            if self.linux:
                seed_claude_config(self.claude_hooks_cfg, self.cwd, os.environ)
        if "agy" in self.args.harness:
            seed_agy_home(self.home, self.cwd)
            write_private(self.agy_env, "".join(f"export {k}={shlex.quote(v)}\n" for k, v in agy_env(os.environ)))
        if "opencode" in self.args.harness:
            self.setup_opencode()
        for h in self.args.harness:
            if h == "opencode":
                continue
            if h == "hermes":
                env = {"HOME": str(self.hermes_home), "PATH": self.hermes_env()["PATH"]}
                self.ht("setup", h, "--profile", "default", env=env, tag="setup:hermes", timeout=120)
                rc, out, err = self.run([str(self.hermes_bin()), "--profile", "default", "plugins", "enable",
                                         "herdr-threads"], env=self.hermes_env(), tag="hermes:plugins-enable",
                                        timeout=120)
                if rc != 0:
                    raise RuntimeError(f"hermes plugins enable: rc={rc} {clip((err or out).strip(), 300, self.secrets)}")
                self.hermes_passthrough()
                continue
            self.ht("setup", h, "--with-permissions", env=setup_env, tag=f"setup:{h}", timeout=120)
        if "claude" in self.args.harness and getattr(self, "prompt_race_cases", False):
            add_slow_prompt_hook(self.claude_cfg / "settings.json", PROMPT_RACE_HOOK_S)
        if self.linux and "claude" in self.args.harness:
            seed_claude_config(self.claude_cfg, self.cwd, os.environ)
            # The panes' shells are the Herdr server's, not this process's: they get the credential only from
            # this private file, so neither the typed command line nor the pane screen ever carries it.
            write_private(self.claude_env, "".join(f"export {k}={shlex.quote(v)}\n" for k, v in claude_env(os.environ)))
        if "codex" in self.args.harness:
            self.auth_digest = provision_codex_auth(self.codex_auth_home() / "auth.json", self.codex_home / "auth.json")
            cfg = self.codex_home / "config.toml"
            text = cfg.read_text() if cfg.exists() else ""
            key = f'[projects."{self.cwd}"]'
            if key not in text:
                text = text.rstrip("\n") + f"\n\n{key}\ntrust_level = \"trusted\"\n"
            # A low-quota account otherwise raises a "switch model?" modal over the composer: every wake is then
            # (correctly) refused, and the driver's next Enter would pick a menu option.
            if "[notice]" not in text:
                text = text.rstrip("\n") + "\n\n[notice]\nhide_rate_limit_model_nudge = true\n"
            cfg.write_text(text)
            self.install_hook_trace()
        print(f"  isolated daemon up; hooks installed by setup for {', '.join(self.args.harness)}", flush=True)

    def install_hook_trace(self):
        """A silent trace hook beside every herdr-threads hook group of the scratch Codex hooks.json: it records only
        {utc_ms, pane, event, session, source, transcript_path null|present|absent} into the evidence directory. Codex
        runs with --dangerously-bypass-hook-trust, so no trust hash is touched."""
        script = self.bindir / "hook-trace"
        write_hook_trace_script(script, self.ev / HOOK_TRACE_FILE)
        hooks = self.codex_home / "hooks.json"
        hooks.write_text(json.dumps(add_hook_trace(json.loads(hooks.read_text()), shlex.quote(str(script))), indent=2) + "\n")

    def opencode_dirs(self):
        """name -> scratch path of every directory an opencode recipient's environment names."""
        o = self.opencode_root
        return {"XDG_CONFIG_HOME": o / "config", "XDG_DATA_HOME": o / "data", "XDG_STATE_HOME": o / "state",
                "XDG_CACHE_HOME": o / "cache", "TMPDIR": o / "tmp", "OPENCODE_CONFIG_DIR": o / "config-dir",
                "project": o / "project", "ledger": o / "ledger"}

    @property
    def opencode_db(self):
        return self.opencode_dirs()["XDG_DATA_HOME"] / "opencode" / "opencode.db"

    def setup_opencode(self):
        """The scratch opencode: its dirs, the global opencode.json, the credential file, then the public
        `setup opencode --with-permissions` into the scratch OPENCODE_CONFIG_DIR (plugin and skill)."""
        dirs = self.opencode_dirs()
        for d in dirs.values():
            d.mkdir(mode=0o700, parents=True, exist_ok=True)
        cfg = dirs["XDG_CONFIG_HOME"] / "opencode"
        cfg.mkdir(mode=0o700, exist_ok=True)
        write_private(cfg / "opencode.json", json.dumps(opencode_config(self.args.opencode_model), indent=2) + "\n")
        key = (os.environ.get(OPENCODE_CREDENTIAL_ENV) or "").strip()
        write_private(self.opencode_env, f"export {OPENCODE_CREDENTIAL_ENV}={shlex.quote(key)}\n")
        env = {"HOME": str(self.home), "XDG_CONFIG_HOME": str(dirs["XDG_CONFIG_HOME"]),
               "OPENCODE_CONFIG_DIR": str(dirs["OPENCODE_CONFIG_DIR"])}
        self.ht("setup", "opencode", "--with-permissions", env=env, tag="setup:opencode", timeout=120)
        out = self.run([shutil.which("opencode") or "opencode", "--version"], env=self.opencode_env_vars(),
                       tag="version", timeout=60)[1]
        self.facts["versions"]["opencode"] = clip(out.strip(), 120, self.secrets)
        print(f"  opencode: {self.facts['versions']['opencode']}", flush=True)

    def opencode_env_vars(self):
        """The environment of an opencode the driver itself runs (no model, so no credential): the scratch HOME,
        XDG dirs and TMPDIR, never the real ones."""
        dirs = self.opencode_dirs()
        env = {k: v for k, v in os.environ.items()
               if k not in CALLER_SESSION_ENV and not k.startswith(("XDG_", "OPENCODE")) and k != "HERDR_PANE_ID"}
        env.update({k: str(dirs[k]) for k in ("XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME",
                                              "TMPDIR", "OPENCODE_CONFIG_DIR")},
                   HOME=str(self.home), OPENCODE_DISABLE_AUTOUPDATE="1")
        return env

    def tab(self, label):
        data = self.herdr("tab", "create", "--workspace", self.workspace, "--cwd", self.cwd, "--label", label,
                          "--no-focus")
        return data["result"]["root_pane"]["pane_id"]

    def seat(self, pane):
        out = self.ht("seat", "resolve", "--pane", pane, "--new-seat", "--operator", human=True,
                      tag="seat:resolve", timeout=30)
        seat = first_id(out, "s")
        if not seat:
            raise RuntimeError(f"seat resolve {pane}: no seat id in {clip(out, 200, getattr(self, 'secrets', None))}")
        return seat

    def layout(self):
        self.layout_attempted = True  # an uncertain create is found by its label at cleanup
        data = self.herdr("workspace", "create", "--label", self.workspace_label(), "--cwd", self.cwd, "--no-focus")
        self.workspace = data["result"]["workspace"]["workspace_id"]
        self.coordinator_tab = data["result"]["tab"]["tab_id"]
        self.herdr("tab", "rename", self.coordinator_tab, "coordinator")
        # The senders are person seats (`human me init`, agent guard kept) in owned shell panes: test traffic is
        # never attributed to an agent, whatever process runs this script.
        # "fresh" never contacts a recipient before busy-invite/1:1, so that case is a real first contact.
        for role, pane in (("main", data["result"]["root_pane"]["pane_id"]), ("fresh", None)):
            pane = pane or self.tab("coordinator-fresh")
            out = self.ht("me", "init", pane=pane, human=True, tag=f"coordinator:{role}:me-init")
            seat = first_id(out, "s")
            if not seat:
                raise RuntimeError(f"me init {pane}: no seat id in {clip(out, 200, getattr(self, 'secrets', None))}")
            self.coords[role] = {"pane": pane, "seat": seat}
        self.recipients = []
        for h in self.args.harness:
            for threshold in THRESHOLDS:
                name = f"{SHORT_NAMES.get(h, h[:2])}-t{threshold}"
                pane = self.tab(name)
                self.recipients.append(Recipient(self, name, h, threshold, pane, self.seat(pane)))
        if "codex" in self.args.harness and (getattr(self, "side_cases", False) or getattr(self, "native_queue_case", False)):
            pane = self.tab("co-plain")
            self.recipients.append(Recipient(self, "co-plain", "codex", 0, pane, self.seat(pane), plain=True))
        if "claude" in self.args.harness and getattr(self, "hooks_cases", False):
            pane = self.tab("cl-hooks")
            self.recipients.append(Recipient(self, "cl-hooks", "claude", 0, pane, self.seat(pane), hooks_only=True))
        if getattr(self, "tmux_cases", False):
            for h in self.args.harness:
                if h in TMUX_HARNESSES:  # no Herdr tab: the recipient lives in its own private tmux server
                    self.recipients.append(TmuxRecipient(self, f"{h[:2]}-tmux", h, f"tmux-{h}-{self.run_id}"))
        print(f"  workspace {self.workspace}: coordinators {self.coord_seats()}; " +
              ", ".join(f"{r.name}={r.seat or r.pane}" for r in self.recipients), flush=True)

    def launch_line(self, harness, plain=False, hooks_only=False, name=None):
        q = shlex.quote
        path = f"PATH={q(str(self.bindir))}:\"$PATH\""
        if harness == "claude":
            # hooks_only: the `setup claude --hooks-only` installation, and no mod directory at all.
            scratch = self.claude_hooks_cfg if hooks_only else self.claude_cfg
            argv = ["command", "claude", "--setting-sources", "project,local", "--settings",
                    str(scratch / "settings.json"), "--permission-mode", self.args.claude_permission_mode]
            if self.args.claude_model:
                argv += ["--model", self.args.claude_model]
            if self.linux:  # the scratch config setup installed into; the credential comes from the environment
                cfg = f"CLAUDE_CONFIG_DIR={q(str(scratch))} "
            else:
                cfg = f"CLAUDE_CONFIG_DIR={q(self.args.claude_config_dir)} " if self.args.claude_config_dir else ""
            unset = "CLAUDECODE CLAUDE_CODE_ENTRYPOINT CLAUDE_PID"
            if hooks_only:
                unset += " CLAUDE_CODE_PLUGIN_DIRS"
                cfg += "CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION=1 "
            else:
                cfg += f"CLAUDE_CODE_PLUGIN_DIRS={q(str(self.state / 'claude-mod' / 'herdr-threads'))} "
            line = f"unset {unset}; {path} {cfg}HERDR_AGENT=claude {shlex.join(argv)}"
            env_file = getattr(self, "claude_env", None)
            if env_file is not None and env_file.exists():
                return f"( . {q(str(env_file))} && {line} )"
            return line
        if harness == "hermes":
            hh = self.hermes_home
            return (f"unset CODEX_HOME {' '.join(HERMES_ROOT_ENV)}; PATH={q(str(self.bindir))}:{q(str(self.hermes_bin().parent))}:\"$PATH\" "
                    f"HOME={q(str(hh))} HERDR_AGENT=hermes exec hermes")
        if harness == "opencode":
            # The credential stays in the private env file; the pane's command line and screen never carry it.
            dirs = self.opencode_dirs()
            xdg = " ".join(f"{k}={q(str(dirs[k]))}" for k in ("XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME",
                                                               "XDG_CACHE_HOME", "TMPDIR", "OPENCODE_CONFIG_DIR"))
            ledger = f"HERDR_THREADS_MOD_LEDGER={q(str(dirs['ledger'] / (name or 'opencode')))}.jsonl "
            opencode = shutil.which("opencode") or "opencode"
            return (f"( . {q(str(self.opencode_env))} && cd {q(str(dirs['project']))} && {path} "
                    f"HOME={q(str(self.home))} {xdg} OPENCODE_DISABLE_AUTOUPDATE=1 {ledger}HERDR_AGENT=opencode "
                    f"exec {q(opencode)} )")
        if harness == "agy":
            # The credential stays in the private env file; the pane's command line and screen never carry it.
            # By absolute path: the pane's shell has the Herdr server's PATH, not this driver's.
            agy = shutil.which("agy") or "agy"
            return (f"( . {q(str(self.agy_env))} && {path} HOME={q(str(self.home))} HERDR_AGENT=agy "
                    f"exec {q(agy)} )")
        # shell_snapshot off: the snapshot replays the interactive ~/.zshrc PATH, which can put an installed
        # herdr-threads ahead of the candidate's bin dir (ht-l16); a non-interactive login shell keeps it first.
        argv = ["command", "codex", "--no-daemon", "-C", self.cwd, "--dangerously-bypass-hook-trust",
                "-c", "check_for_update_on_startup=false", "-c", "features.shell_snapshot=false",
                *shlex.split(self.args.codex_args)]
        if self.args.codex_model:
            argv += ["-m", self.args.codex_model]
        if plain:
            argv += ["-c", "features.codex_hooks=false"]
        return (f"{path} HERDR_AGENT=codex CODEX_HOME={q(str(self.codex_home))} "
                f"CODEX_SQLITE_HOME={q(str(self.codex_home))} {shlex.join(argv)}")

    @property
    def linux(self):
        return getattr(self.args, "harness_mode", "macos") == "linux"

    def coord_seats(self):
        return " and ".join(c["seat"] for c in self.coords.values()) or "none"

    def cleanup(self):
        """Attempt every owned resource independently, then verify it is gone. Returns the problems found;
        any problem makes the run fail. `--keep` is the explicit diagnostic opt-out."""
        with self.lock:  # stop admission first, whatever the drain then does
            self.stopping.set()
        try:
            problems = self.drain()
        except BaseException as e:  # a drain error never skips the owned resources below
            problems = [f"drain raised: {e!r}"]
        steps = []
        self.remove_agy_env(problems)  # a running agy already holds the key; the file never outlives the run
        self.remove_opencode_env(problems)  # likewise for a running opencode
        self.remove_claude_env(problems)
        if self.args.keep:
            self.cleanup_report = {"skipped": "--keep", "workspace": self.workspace,
                                   "daemon_attempted": self.daemon_attempted, "problems": problems}
            return problems
        if not self.workspace and self.layout_attempted:
            try:
                self.workspace = self.workspace_by_label()
                steps.append({"workspace_found_by_label": self.workspace})
            except Exception as e:
                problems.append(f"workspace {self.workspace_label()}: create outcome not verified: {clip(e, 200, getattr(self, 'secrets', None))}")
        if self.workspace:
            try:
                rc = self.run(["herdr", "workspace", "close", self.workspace], tag="cleanup:workspace", timeout=30,
                              cleanup=True)[0]
                steps.append({"workspace_close_rc": rc})
            except Exception as e:  # keep going: the daemon is ours too
                steps.append({"workspace_close_error": clip(e, 200, getattr(self, 'secrets', None))})
            try:
                if self.workspace_alive():
                    problems.append(f"workspace {self.workspace} still open after close")
            except Exception as e:
                problems.append(f"workspace {self.workspace}: closure not verified: {clip(e, 200, getattr(self, 'secrets', None))}")
        self.cleanup_tmux(problems, steps)
        if getattr(self, "hermes_home", None):
            # A recipient's pane (whichever started first) spawned Hermes' gateway (`gateway run --idle-exit`), which outlives the pane.
            try:
                rc = self.run([str(self.hermes_bin()), "--profile", "default", "gateway", "stop"],
                              env=self.hermes_env(), tag="cleanup:hermes-gateway", timeout=60, cleanup=True)[0]
                steps.append({"hermes_gateway_stop_rc": rc})
                end = time.time() + self.process_grace_s
                while self.hermes_gateways() and time.time() < end:
                    time.sleep(0.5)
                left = self.hermes_gateways()
                if left:
                    problems.append(f"hermes gateway still running after stop (rc={rc}): pids {left}")
            except Exception as e:
                problems.append(f"hermes gateway stop: {clip(e, 200, getattr(self, 'secrets', None))}")
            # The bridge names this run's state dir: the next run's setup would refuse it as a conflict.
            try:
                rc = self.run([self.bin, "unsetup", "hermes", "--profile", "default", "--state-dir", str(self.state)],
                              env={"HOME": str(self.hermes_home), "PATH": self.hermes_env()["PATH"]},
                              tag="cleanup:unsetup-hermes", timeout=60, cleanup=True)[0]
                steps.append({"hermes_unsetup_rc": rc})
                if rc != 0:
                    problems.append(f"unsetup hermes: rc={rc}")
            except Exception as e:
                problems.append(f"unsetup hermes: {clip(e, 200, getattr(self, 'secrets', None))}")
        if self.daemon_attempted:
            try:
                rc = self.run_rc("daemon", "stop", timeout=30)
                steps.append({"daemon_stop_rc": rc})
            except Exception as e:
                steps.append({"daemon_stop_error": clip(e, 200, getattr(self, 'secrets', None))})
            end = time.time() + self.stop_deadline_s
            healthy = True
            while True:
                try:
                    rc = self.run_rc("daemon", "health")
                    healthy = rc == 0 or not isinstance(rc, int)  # a timed-out probe is unknown, not stopped
                except Exception:
                    healthy = True  # unknown is not stopped
                if not healthy or time.time() >= end:
                    break
                time.sleep(0.5)
            if healthy:
                problems.append(f"isolated daemon still healthy {self.stop_deadline_s:.0f}s after stop")
        # Exiting agents and their session-end hooks may still name the root for a moment: allow a grace period.
        # Codex agents carry the root only in their environment; the workspace check above covers them.
        end = time.time() + self.process_grace_s
        while True:
            try:
                leftover = self.owned_processes()
            except Exception as e:  # a failed scan is unknown, never "nothing left"
                leftover = None
                scan_error = clip(e, 200, getattr(self, 'secrets', None))
            if leftover == [] or time.time() >= end:
                break
            time.sleep(0.5)
        if leftover is None:
            problems.append(f"owned-process scan failed: {scan_error}")
        elif leftover:
            problems.append(f"owned processes still running {self.process_grace_s:.0f}s after cleanup: {leftover}")
        if self.auth_digest is not None:
            steps.append(self.retire_auth_copy())
        if not problems:
            try:
                if self.root.exists():
                    shutil.rmtree(self.root)
                if self.root.exists():
                    raise OSError("root still exists after removal")
                steps.append({"root_removed": str(self.root)})
            except Exception as e:
                problems.append(f"scratch root removal failed: {clip(e, 200, getattr(self, 'secrets', None))}")
        self.cleanup_report = {"steps": steps, "problems": problems}
        return problems

    def tmux_env(self):
        """The environment of a private tmux server (and so of the panes in it): no Herdr pane, no enclosing tmux
        and no agent session of whatever runs this script, so the recipient is a pane-less named caller."""
        return {k: v for k, v in os.environ.items()
                if not k.startswith("HERDR_") and k not in ("TMUX", "TMUX_PANE", *CALLER_SESSION_ENV)}

    def tmux_start(self):
        """Start one private `tmux -L ht-test-<uuid>` server with a shell window in the checkout; returns its socket
        name. The name is recorded before the start, so an uncertain start is still killed at cleanup."""
        name = "ht-test-" + uuid.uuid4().hex
        with self.lock:
            self.tmux_servers = [*getattr(self, "tmux_servers", []), name]
        rc, out, err = self.run(["tmux", "-L", name, "-f", "/dev/null", "new-session", "-d", "-x", "200", "-y", "50",
                                 "-c", self.cwd, "sh"], env=self.tmux_env(), tag="tmux:start")
        if rc != 0:
            detail = clip((err or out).strip(), 300, getattr(self, "secrets", None))
            raise RuntimeError(f"tmux new-session: rc={rc} {detail}")
        return name

    def tmux(self, server, *argv, timeout=30):
        """`tmux -L server argv`: (rc, stdout)."""
        rc, out, _err = self.run(["tmux", "-L", server, *argv], env=self.tmux_env(), tag="tmux:" + argv[0],
                                 timeout=timeout)
        return rc, out

    def tmux_server_gone(self, name):
        """Whether the private server `name` no longer answers and no process still carries its socket name."""
        rc = self.run(["tmux", "-L", name, "display", "-p", "x"], env=self.tmux_env(), tag="cleanup:tmux-verify",
                      timeout=10, cleanup=True)[0]
        left = self.run(["pgrep", "-f", "--", f"-L {name}"], tag="cleanup:tmux-processes", timeout=10, cleanup=True)[0]
        return rc != 0 and rc != "timeout" and left == 1

    def cleanup_tmux(self, problems, steps):
        """Kill every private tmux server this run started (the agents in it end with it) and verify each is gone."""
        for name in getattr(self, "tmux_servers", []):
            try:
                rc = self.run(["tmux", "-L", name, "kill-server"], env=self.tmux_env(), tag="cleanup:tmux-kill",
                              timeout=15, cleanup=True)[0]
                steps.append({"tmux_kill_rc": rc, "server": name})
            except Exception as e:  # keep going: the verification below decides
                steps.append({"tmux_kill_error": clip(e, 200, getattr(self, 'secrets', None)), "server": name})
            end = time.time() + self.process_grace_s
            try:
                gone = self.tmux_server_gone(name)
                while not gone and time.time() < end:
                    time.sleep(0.2)
                    gone = self.tmux_server_gone(name)
            except Exception as e:  # a failed probe is unknown, never "gone"
                problems.append(f"tmux server {name}: removal not verified: {clip(e, 200, getattr(self, 'secrets', None))}")
                continue
            if not gone:
                problems.append(f"tmux server {name} still running after kill-server")
                continue
            # kill-server leaves the socket file behind.
            env = self.tmux_env()
            socket = os.path.join(env.get("TMUX_TMPDIR") or "/tmp", f"tmux-{os.getuid()}", name)
            try:
                os.unlink(socket)
                steps.append({"tmux_socket_removed": socket})
            except FileNotFoundError:
                pass
            except OSError as e:
                problems.append(f"tmux server {name}: socket {socket} not removed: {clip(e, 200, getattr(self, 'secrets', None))}")

    def drain(self):
        """Stop admitting helpers, terminate the in-flight ones (each in its own process group), then join the
        recipient workers, all with bounded waits. Returns the problems found."""
        problems = []
        with self.lock:
            self.stopping.set()
            inflight = list(self.children)
        for p in inflight:
            kill_group(p, signal.SIGTERM)
        end = time.time() + self.child_term_grace_s
        for p in inflight:
            try:
                p.wait(timeout=max(0.0, end - time.time()))
            except subprocess.TimeoutExpired:
                kill_group(p, signal.SIGKILL)
                try:
                    p.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    problems.append(f"helper pid {p.pid} did not exit: {p.args[:3]}")
        end = time.time() + self.drain_timeout_s
        for w in self.workers:
            if w not in self.start_attempted:  # never asked to start: nothing to drain
                continue
            settle = time.time() + 1.0  # a thread still inside start() cannot be joined yet: retry briefly
            while True:
                try:
                    w.join(timeout=max(0.0, end - time.time()))
                    break
                except RuntimeError:  # "cannot join thread before it is started"
                    if time.time() >= min(settle, end):
                        break  # its start failed before the thread existed
                    time.sleep(0.05)
            if w.is_alive():
                problems.append(f"worker {w.name} did not stop within {self.drain_timeout_s:.0f}s")
        with self.lock:
            late = [p.pid for p in self.children if p.poll() is None]
        if late:
            problems.append(f"helpers still running after drain: {late}")
        return problems

    def workspace_label(self):
        return f"wake-e2e-{self.run_id}"

    def workspace_by_label(self):
        rc, out, _err = self.run(["herdr", "workspace", "list"], tag="cleanup:find-workspace", timeout=30, cleanup=True)
        if rc != 0:
            raise RuntimeError(f"workspace list rc={rc}")
        for row in walk(json.loads(out or "{}")):
            if row.get("label") == self.workspace_label() and row.get("workspace_id"):
                return row["workspace_id"]
        return None

    def remove_agy_env(self, problems):
        path = getattr(self, "agy_env", None)
        if path is None:
            return
        try:
            path.unlink(missing_ok=True)
        except OSError as e:
            problems.append(f"agy credential file not removed: {e!r}")

    def remove_opencode_env(self, problems):
        path = getattr(self, "opencode_env", None)
        if path is None:
            return
        try:
            path.unlink(missing_ok=True)
        except OSError as e:
            problems.append(f"opencode credential file not removed: {e!r}")

    def remove_claude_env(self, problems):
        path = getattr(self, "claude_env", None)
        if path is None:
            return
        try:
            path.unlink(missing_ok=True)
        except OSError as e:
            problems.append(f"claude credential file not removed: {e!r}")

    def retire_auth_copy(self):
        """Delete the credential copy unless Codex refreshed it in the run: then it may hold the only current refresh
        token (rotation can retire the source's), so it is kept, private, and the operator is told where."""
        copy = self.codex_home / "auth.json"
        if not copy.exists():
            return {"codex_auth_copy": "absent"}
        self.auth_refreshed = file_sha256(copy) != self.auth_digest
        if not self.auth_refreshed:
            copy.unlink()
            return {"codex_auth_copy": "deleted (unchanged)"}
        # Preserve the potentially sole valid refresh token outside the scratch profile.
        saved_dir = self.root.parent / f"ht-wake-auth.{self.run_id}"
        saved_dir.mkdir(mode=0o700)
        saved = saved_dir / "auth.json"
        shutil.move(copy, saved)
        os.chmod(saved, 0o600)
        print(f"wake-e2e: Codex refreshed its tokens during the run; the refreshed credentials are kept at {saved} "
              f"(mode 0600). The source profile's refresh token may have been rotated: copy this file back or log in "
              f"again, then delete it.", file=sys.stderr)
        return {"codex_auth_copy": f"kept (refreshed): {saved}"}

    def workspace_alive(self):
        rc, out, _err = self.run(["herdr", "workspace", "list"], tag="cleanup:verify-workspace", timeout=30,
                                 cleanup=True)
        if rc != 0:
            raise RuntimeError(f"workspace list rc={rc}")
        return any(row.get("workspace_id") == self.workspace for row in walk(json.loads(out or "{}")))

    def owned_processes(self):
        """pids whose command line names this run's private root (the daemon's --state-dir, the shim)."""
        rc, out, err = self.run(["pgrep", "-f", str(self.root)], tag="cleanup:verify-processes", timeout=10,
                                cleanup=True)
        if rc == 1:  # pgrep: no process matched
            return []
        if rc != 0:
            raise RuntimeError(f"pgrep rc={rc} {clip(err.strip(), 100, getattr(self, 'secrets', None))}")
        return [int(p) for p in out.split() if p.isdigit() and int(p) != os.getpid()]

    def run_rc(self, *argv, timeout=10):
        e = {k: v for k, v in os.environ.items() if k not in CALLER_SESSION_ENV and k != "HERDR_PANE_ID"}
        return self.run([self.bin, "--state-dir", str(self.state), "--host-endpoint", self.endpoint, *argv],
                        env=e, tag="cleanup:" + "-".join(argv), timeout=timeout, cleanup=True)[0]


def kill_group(p, sig):
    """Signal a helper's whole process group (it was started in its own session); a helper already reaped is
    skipped, so a reused pid is never signalled."""
    if p.poll() is not None:
        return
    try:
        os.killpg(p.pid, sig)
    except (ProcessLookupError, PermissionError):
        pass


def reap(p, timeout=5):
    """Collect a killed helper's output with a bound (a descendant outside its group could hold the pipes)."""
    try:
        return p.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        return "", ""


def file_sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def provision_codex_auth(source, dest):
    """A private regular-file copy (mode 0600) of the source credentials: Codex's file store truncates and rewrites
    auth.json on token refresh, which would follow a symlink into the real profile. Returns the copy's digest, so the
    run can report (without contents) whether Codex refreshed tokens."""
    dest = Path(dest)
    if dest.is_symlink() or dest.exists():
        dest.unlink()
    fd = os.open(dest, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "wb") as out, open(source, "rb") as src:
        shutil.copyfileobj(src, out)
    return file_sha256(dest)


def first_id(text, prefix):
    m = re.search(r'"(%s[A-Za-z0-9]{11})"' % prefix, text)
    return m.group(1) if m else None


def walk(x):
    if isinstance(x, dict):
        yield x
        for v in x.values():
            yield from walk(v)
    elif isinstance(x, list):
        for v in x:
            yield from walk(v)


class BusyBarrier:
    """A tool-entry witness and explicit release spanning the coordinator's publication. The held tool proves it is
    alive by keeping a heartbeat file fresh, not by its pid: Codex's sandbox runs it in its own PID namespace."""
    def __init__(self, root, watchdog=TURN_TIMEOUT_MS / 1000, stale=2.0):
        self.path = Path(tempfile.mkdtemp(prefix="busy-hold.", dir=root))
        self.watchdog = watchdog
        self.stale = stale

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        # Keep witnesses until the run's normal scratch cleanup: the tool may still be polling.
        (self.path / "release").touch()

    def command(self):
        helper = self.path / "hold.py"
        helper.write_text("import os, time\nfrom pathlib import Path\n"
                          "p = Path(__file__).parent\n"
                          "(p / 'alive').touch()\n"
                          "(p / 'entered.tmp').write_text(str(os.getpid()))\n"
                          "(p / 'entered.tmp').replace(p / 'entered')\n"
                          f"end = time.monotonic() + {self.watchdog!r}\n"
                          "while not (p / 'release').exists():\n"
                          "    if time.monotonic() >= end:\n"
                          "        (p / 'expired').touch()\n"
                          "        raise SystemExit(1)\n"
                          "    (p / 'alive').touch()\n"
                          "    time.sleep(.02)\n")
        return shlex.join([sys.executable, str(helper)])

    def wait_entered(self, timeout=BUSY_ENTRY_S, throttled=None, throttled_timeout=180, idle=None, resubmit=None):
        """Wait for the held tool to start. A provider rate limit backs the agent off before its first tool call (Codex
        waited 58-99 s after a 429 in the 2.5.0 qualification): when `throttled()` names one at `timeout`, the wait
        runs on to `throttled_timeout`, and a tool that still never entered fails as provider throttled. Returns the
        throttle a late entry waited out (row evidence), else None. The tool's watchdog counts from its own start, so a
        longer entry wait never shortens the hold.

        Entering the tool is this case's setup, not what it asserts. A model can pass `timeout` before its first tool
        call with no throttle to name (Codex at load 35 in the 2.8.3 qualification): the wait then runs on to
        `throttled_timeout` too, on what `idle()` observes. A recipient back at idle has no turn that will ever run the
        tool, so `resubmit()` sends the prompt once, whenever in that wait it is seen idle."""
        end, cause, late, resent, probe = time.monotonic() + timeout, None, False, False, 0
        while not (self.path / "entered").exists():
            if time.monotonic() >= end:
                if cause is None and throttled is not None and (cause := throttled()):
                    end += throttled_timeout - timeout
                    continue
                if idle is not None and cause is None and not late:
                    late = True
                    end += throttled_timeout - timeout
                    continue
                raise RuntimeError(f"provider throttled ({cause}); busy tool did not enter" if cause
                                   else "busy tool did not enter")
            if late and not resent and time.monotonic() >= probe:
                probe = time.monotonic() + 2
                if idle():
                    resent = True
                    resubmit()
            time.sleep(.02)
        self.assert_held()
        return cause

    def assert_held(self):
        if (self.path / "expired").exists():
            raise RuntimeError("busy tool barrier expired before publication completed")
        if (self.path / "release").exists():
            raise RuntimeError("busy tool barrier released before publication completed")
        try:
            age = time.time() - (self.path / "alive").stat().st_mtime
        except OSError as exc:
            raise RuntimeError("busy tool exited before publication completed") from exc
        if age > self.stale:
            raise RuntimeError("busy tool exited before publication completed")


def codex_throttle(codex_home, since):
    """The latest 429 retry Codex logged (`codex_core::responses_retry` in CODEX_HOME's logs_2.sqlite) at or after
    `since` (epoch seconds), as `429 retry N/M in Ss`; None without one or without a readable log. The Codex
    recipients share one scratch CODEX_HOME and one account, so a 429 on any of them counts."""
    db = Path(codex_home) / "logs_2.sqlite"
    if not db.is_file():
        return None
    try:
        with contextlib.closing(sqlite3.connect(db.resolve().as_uri() + "?mode=ro", uri=True, timeout=1)) as c:
            row = c.execute("SELECT feedback_log_body FROM logs WHERE target = 'codex_core::responses_retry' AND ts >= ?"
                            " AND feedback_log_body LIKE '%status: 429%' ORDER BY ts DESC, ts_nanos DESC, id DESC LIMIT 1",
                            (int(since),)).fetchone()
    except sqlite3.Error:
        return None
    if not row:
        return None
    m = re.search(r"\((\d+/\d+) in ([\d.]+)(m?s)\)", row[0] or "")
    if not m:
        return "429 retry"
    delay = float(m.group(2)) / (1000 if m.group(3) == "ms" else 1)
    return f"429 retry {m.group(1)} in {delay:.0f}s"


def late_busy_entries(rows):
    """Rows whose setup needed more than an on-time prompt with no throttle to explain it: visible, never fatal. A first
    prompt the recipient swallowed passes on the second, so a run's reader must see that it happened."""
    return [f"late busy entry: {r['recipient']} {r['case']}: "
            + json.dumps({k: r["checks"][k] for k in ("entered_after_s", "busy_prompt_resubmitted") if k in r["checks"]})
            for r in rows
            if ("busy_prompt_resubmitted" in r.get("checks", {})
                or ("entered_after_s" in r.get("checks", {}) and "provider_throttled" not in r["checks"]))]


def busy_turn_split(text, anchor, done=BUSY_DONE_LINE, prompt_end_text=BUSY_PROMPT_END, last=False, occurrence=0):
    """Split the screen at the busy turn's native boundaries: (in_turn, after_turn, end_seen). `in_turn` runs from
    the busy prompt `anchor` to the agent's done line (the last one with `last`: a continued turn prints it twice),
    `after_turn` is everything later; None if the prompt is not on screen. `occurrence` picks the anchor's copy, for a
    held turn that was not the first prompt's."""
    # The prompt itself: an agent echoing the tag later must not move it.
    at = -1
    for _ in range(occurrence + 1):
        found = text.find(anchor, at + 1)
        if found < 0:
            break
        at = found
    if at < 0:
        return None
    rest = text[at:]
    # The prompt's last marker mention, found across the terminal's line wrapping (at spaces or after a hyphen).
    words = (re.escape(w).replace(r"\-", r"-\s*") for w in prompt_end_text.split())
    prompt_end = re.compile(r"\s+".join(words)).search(rest)
    ends = list(done.finditer(rest, prompt_end.end() if prompt_end else 0))
    if not ends:
        return rest, "", False
    end = ends[-1] if last else ends[0]
    return rest[:end.end()], rest[end.end():], True


def is_wake_line(text):
    text = text.strip().lstrip("›❯> ┃")  # Codex, Claude, agy prompt glyphs; opencode's user-turn border
    return text in ("herdr-threads: wake up", CHANNEL_WAKE_UP) or text.startswith("herdr-threads: attention pending")


def _pid_alive(pid):
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def wake_count(text):
    # Match whole prompt/body lines; an explanation mentioning "wake up" is not a wake.
    return sum(is_wake_line(line) for line in text.splitlines())


def wake_delta(before, after):
    """Wake lines a case added: the capture is cumulative, so a case judges the delta, not the whole screen."""
    return wake_count(after) - wake_count(before)


def admission_observation(observations):
    """The ACK observation to judge: one that carries a `row` if any does, else the first, else None."""
    return next((o for o in observations if o.get("row")), observations[0] if observations else None)


def claude_wake_enqueues(cfg, sessions):
    """How many times the mod's wake prompt entered the engine's queue in `sessions`' transcripts
    (`<cfg>/projects/*/<session>.jsonl`, `queue-operation` `enqueue` rows whose content is the wake prompt)."""
    count = 0
    for session in sessions:
        for path in (Path(cfg) / "projects").glob(f"*/{session}.jsonl"):
            try:
                with open(path, encoding="utf-8", errors="replace") as lines:
                    for line in lines:
                        if '"queue-operation"' not in line or len(line) > 4096:
                            continue
                        try:
                            row = json.loads(line)
                        except ValueError:
                            continue
                        count += (row.get("type") == "queue-operation" and row.get("operation") == "enqueue"
                                  and row.get("content") == CHANNEL_WAKE_UP)
            except OSError:
                continue
    return count


def add_slow_prompt_hook(settings_path, seconds):
    """Append a UserPromptSubmit command hook that sleeps `seconds` to a scratch Claude settings.json."""
    path = Path(settings_path)
    doc = json.loads(path.read_text()) if path.is_file() else {}
    doc.setdefault("hooks", {}).setdefault("UserPromptSubmit", []).append(
        {"hooks": [{"type": "command", "command": f"sleep {seconds}"}]})
    path.write_text(json.dumps(doc, indent=2) + "\n")


def transcript_has_row(cfg, sessions, row):
    """True when one of `sessions`' own transcripts (`<cfg>/projects/*/<session>.jsonl`) holds an entry whose
    `uuid` is `row`; else what was missing. Reads only those files, one JSON line at a time."""
    paths = [path for session in sessions for path in (Path(cfg) / "projects").glob(f"*/{session}.jsonl")]
    if not paths:
        return "no transcript for the recipient's session"
    for path in paths:
        try:
            with open(path, encoding="utf-8", errors="replace") as lines:
                for line in lines:
                    if row not in line:
                        continue
                    try:
                        if json.loads(line).get("uuid") == row:
                            return True
                    except (ValueError, AttributeError):
                        continue
        except OSError:
            continue
    return f"row {row} not in the recipient's transcript"


# Global options that take a value; the first other non-option word is the subcommand.
VALUE_OPTIONS = ("--state-dir", "--host-endpoint", "--seat", "--pane")


def is_inbox_call(argv):
    """True when argv (a herdr-threads call without the program name) runs the `inbox` subcommand."""
    words = iter(argv)
    for word in words:
        if word in VALUE_OPTIONS:
            next(words, None)
        elif not word.startswith("-"):
            return word == "inbox"
    return False


def pane_calls(log_text, pane):
    """argv of each call the shim logged from `pane` (lines of 0x1f-terminated fields: pane, then argv)."""
    rows = (line.split("\x1f")[:-1] for line in log_text.splitlines())
    return [row[1:] for row in rows if row and row[0] == pane]


def inbox_calls(log_text, pane):
    """`inbox` calls the shim logged from `pane`."""
    return sum(1 for argv in pane_calls(log_text, pane) if is_inbox_call(argv))


class QueueCaseEvidence:
    """Bounded canonical observations from this run's private store, joined before case exit."""
    def __init__(self, recipient, case):
        from wake_e2e_queue import queue_snapshot
        self.recipient, self.case = recipient, case
        self.read = lambda after: queue_snapshot(recipient.run.state, recipient.seat, after)
        self.before = self.read(0)
        self.after = self.before.get("watermark", 0)
        self.samples, self.encoded, self.bytes = [], None, 0
        self.overflow = False
        self.error = None
        self.published = False
        self.held_samples = None
        self.stop = threading.Event()
        self.lock = threading.Lock()
        self.worker = None
        self.result = None
        self.publication_monotonic = None
        self.publication_unix_ms = None
        self.host_observations = []
        self.latest_publication_scope = None
        self.selected_windows = []
        self.release_interval_ms = None
        self.root_session = None  # set for a case whose process holds `/side` sessions: the queue target is the root

    def scope_of(self, snapshot):
        """The queue-qualified scope of a snapshot: the process root's when a root session is named, else the seat's
        one eligible binding."""
        from wake_e2e_queue import eligible_scope, process_root_scope
        return process_root_scope(snapshot, self.root_session) if self.root_session else eligible_scope(snapshot)

    def offset(self, at):
        return round((at - self.publication_monotonic) * 1000) if self.publication_monotonic is not None else None

    def observe_status(self, start, end, status):
        with self.lock:
            observation = {"start_ms": self.offset(start), "end_ms": self.offset(end),
                           "status": status if status in IDLE + ("working", "blocked", "unknown") else "other"}
            size = len(json.dumps(observation).encode())
            if len(self.host_observations) >= 512 or self.bytes + size > QUEUE_EVIDENCE_BYTES:
                self.overflow = True
            else:
                self.host_observations.append(observation)
                self.bytes += size

    def _capture(self):
        start = time.monotonic()
        snapshot = self.read(self.after)
        end = time.monotonic()
        encoded = json.dumps(snapshot, sort_keys=True)
        if encoded != self.encoded:
            snapshot = dict(snapshot, observation_start_ms=self.offset(start), observation_end_ms=self.offset(end))
            size = len(json.dumps(snapshot).encode())
            if len(self.samples) >= QUEUE_SAMPLE_LIMIT or self.bytes + size > QUEUE_EVIDENCE_BYTES:
                self.overflow = True
            else:
                self.samples.append(snapshot)
                self.bytes += size
            self.encoded = encoded
        return snapshot

    def __enter__(self):
        if not self.before.get("ok"):
            raise RuntimeError("queue qualification: canonical bootstrap snapshot unavailable")
        def observe():
            while not self.stop.wait(0.1):
                try:
                    with self.lock:
                        self._capture()
                except Exception:
                    self.error = "canonical sampler failed"
                    return
        self.worker = threading.Thread(target=observe, name="queue-proof-" + self.recipient.name)
        self.worker.start()
        return self

    def publication(self):
        with self.lock:
            if not self.published:
                self.publication_monotonic = time.monotonic()
                self.publication_unix_ms = round(time.time() * 1000)
                current = self.read(0)
                self.after = current.get("watermark", self.after)
                self.before = self.read(self.after)
                self.samples, self.encoded, self.bytes = [], None, 0
                self.published = True
                current_scope = self.before
            else:
                current_scope = self.read(self.after)
            self.latest_publication_scope = self.scope_of(current_scope)
            self._capture()

    def current_proof(self):
        """The positive queue proof over the samples so far: for a `/side` case, the root-targeted one."""
        from wake_e2e_queue import queue_proof, side_queue_proof
        with self.lock:
            samples = list(self.samples)
        if self.root_session and self.case in SIDE_QUEUE_CASES:
            return side_queue_proof(self.before, samples, self.root_session), samples
        if self.root_session:
            from wake_e2e_queue import process_root_scope
            return queue_proof(self.before, samples, "accepted",
                               baseline=process_root_scope(self.before, self.root_session)), samples
        return queue_proof(self.before, samples, "accepted"), samples

    def qualified_activity(self):
        proof, samples = self.current_proof()
        if not proof["passed"]:
            return None
        for sample in samples:
            for intent in sample.get("intents", ()):
                matched = next((a for a in proof["attempts"] if a["attempt"] == intent["attempt"]), None)
                fulfillment = intent.get("fulfillment")
                if (matched and intent.get("logical_status") == "fulfilled" and fulfillment
                        and intent.get("owner_boot") == matched["owner_boot"]
                        and fulfillment["turn_id"] != matched["baseline"]["turn_id"]
                        and fulfillment["ordinal"] > matched["baseline"]["ordinal"]):
                    return {"attempt": intent["attempt"], "native_session": intent["scope"]["native_session"],
                            "turn_id": intent["fulfillment"]["turn_id"],
                            "observed_at_ms": sample["observation_end_ms"]}
        return None

    def hold_ended(self):
        with self.lock:
            self._capture()
            self.held_samples = list(self.samples)

    def __exit__(self, *_):
        self.stop.set()
        if self.worker is not None:
            self.worker.join(timeout=2)
            if self.worker.is_alive():
                self.error = "canonical sampler did not stop"
                self.worker.join()  # bounded read-only SQLite calls; never leave a case worker behind

    def finish(self, row):
        from wake_e2e_queue import eligible_scope, queue_proof, stale_authority_native
        if self.result is not None:
            return self.result
        self.__exit__()
        with self.lock:
            self._capture()
        if self.case in FLAG_FLIP_CASES:
            # Judged per trial by flag_flip_verdict (the flag is flipped mid-measurement on purpose); the trial log is
            # the evidence, so the whole-case proof does not apply.
            self.result = {"passed": True, "expectation": "trial_log", "reasons": [], "attempts": [],
                           "observed": {"unknown": 0}, "published": self.published, "samples": []}
            return self.result
        if self.case in RESUME_CASES + QUEUE_NATIVE_CASES + SIDE_DEFER_CASES + CONFIG_PROSE_CASES:
            expectation = "native"
        elif self.root_session and self.case in SIDE_CASES and row.get("expect_wake"):
            # Queue mode with side sessions (ht-z41e.7, spike S2 `drop`): the root is queue-woken for the p0 even
            # while a side turn is busy. side-draft (no wake expected) falls through to "none": the typed root draft
            # refuses the queue lane (ht-z41e.54).
            expectation = "accepted"
        elif self.case in SIDE_CASES and row.get("expect_wake"):
            expectation = "accepted" if eligible_scope(self.before) else "native"
        else:
            expectation = "accepted" if row.get("expect_wake") else "none"
        if self.root_session and expectation == "accepted":
            result, _ = self.current_proof()
        else:
            result = queue_proof(self.before, self.samples, expectation)
        required = bool(getattr(getattr(self.recipient.run, "args", None), "queue_proof", False))
        if expectation == "accepted" and not required and not result["passed"]:
            # Native fallback may follow a refusal before any effect even when the
            # publication was eligible. No possible/unknown effect can take this path.
            native = queue_proof(self.before, self.samples, "none")
            if native["passed"]:
                result = dict(native, expectation="native",
                              transport_inference="compositional native after no queue effect; external wake check required")
        elif expectation == "accepted" and required and not result["passed"] and self.case in QUEUE_STALE_CASES:
            stale = stale_authority_native(self.before, self.samples)
            if stale["passed"]:
                result = stale
        result["accepted_wake_window_s"] = QUEUE_WAKE_WINDOW_S if result.get("attempts") else WAKE_WINDOW_S
        if (result.get("expectation") == "native" and row.get("expect_wake")
                and self.case not in WAKE_BLOCK_CASES + RESUME_CASES + SIDE_CASES + SIDE_DEFER_CASES
                + CONFIG_PROSE_CASES):
            delay = row.get("t_wake_s")
            if not isinstance(delay, (int, float)) or delay > WAKE_WINDOW_S:
                result["passed"] = False
                result["reasons"].append("native wake exceeds unchanged six-second window")
        result["before"] = self.before
        result["published"] = self.published
        result["sample_bytes"] = self.bytes
        result["publication_unix_ms"] = self.publication_unix_ms
        result["host_observations"] = self.host_observations
        if not self.published or self.overflow or self.error:
            result["passed"] = False
            result["reasons"].append(self.error or ("canonical evidence bound exceeded" if self.overflow
                                                   else "no publication checkpoint"))
        if self.case.startswith("busy-") and (required or eligible_scope(self.before, "started") is not None):
            result["known_busy_started"] = eligible_scope(self.before, "started") is not None
            if not result["known_busy_started"]:
                result["passed"] = False
                result["reasons"].append("busy publication lacks canonical Started observation")
        if self.held_samples is not None:
            held = queue_proof(self.before, self.held_samples, "none")
            baseline = eligible_scope(self.before)
            held_activity = bool(baseline) and any(
                n["scope"] == baseline["scope"] and n["turn_id"] != baseline["turn_id"]
                and n["ordinal"] > baseline["ordinal"]
                for sample in self.held_samples for n in sample.get("native", ()))
            held["native_activity_observed"] = held_activity
            if held_activity:
                held["passed"] = False
                held["reasons"].append("native activity while UI held")
            result["held_interval"] = held
            result["passed"] = result["passed"] and held["passed"]
        proved = {item["attempt"] for item in result.get("attempts", ())}
        intents = [i for sample in self.samples for i in sample.get("intents", ())
                   if i.get("attempt") in proved]
        result["fulfillment_while_armed_observed"] = any(
            i.get("phase") == "armed" and i.get("logical_status") == "fulfilled" for i in intents)
        result["logical_fulfillment_observed"] = any(i.get("logical_status") == "fulfilled" for i in intents)
        result["cleanup_list_attempted_observed"] = any(i.get("cleanup_list_attempted") for i in intents)
        result["lifecycle_attempts"] = sorted(
            attempt for attempt in proved
            if any(i.get("attempt") == attempt and i.get("logical_status") == "fulfilled" for i in intents)
            and any(i.get("attempt") == attempt and i.get("cleanup_list_attempted") for i in intents))
        if self.case in WAKE_BLOCK_CASES and (required or result.get("attempts")):
            from wake_e2e_queue import queue_activity_intervals
            result["release_interval_ms"] = self.release_interval_ms
            result["gate_retry_window_s"] = WAKE_BLOCK_WAIT_S
            result["queue_activity_intervals"] = queue_activity_intervals(self.samples, proved)
            timely = bool(proved) and all(i["status"] == "passed" for i in result["queue_activity_intervals"])
            if not timely:
                result["passed"] = False
                result["reasons"].append("UI Accepted-to-activity latency failed or ambiguous")
        elif self.case in RESUME_CASES:
            result["gate_retry_window_s"] = WAKE_BLOCK_WAIT_S
            result["queue_latency_proof"] = False
        result["samples"] = self.samples
        self.result = result
        return result


class Recipient:
    def __init__(self, run, name, harness, threshold, pane, seat, plain=False, hooks_only=False):
        self.run, self.name, self.harness, self.threshold, self.pane, self.seat = run, name, harness, threshold, pane, seat
        self.plain = plain  # Codex hooks feature off: every binding is a plain CLI call
        self.hooks_only = hooks_only  # Claude without the delivery mod: hooks and the native wake only
        self.last_wake = 0.0
        self.group = None
        self.pair = None

    # ---------- observation ----------
    def calls(self):
        """argv of each herdr-threads call this pane made through the shim ([] before any, or in self-test fakes)."""
        try:
            log = self.run.calls_log
        except AttributeError:  # self-test fakes have no shim
            return []
        try:
            return pane_calls(log.read_text(errors="replace"), self.pane)
        except FileNotFoundError:
            return []

    def inbox_calls(self):
        return sum(1 for argv in self.calls() if is_inbox_call(argv))

    def save_calls(self):
        """This pane's logged calls, kept with the evidence so an inbox_calls count is traceable."""
        mine = [" ".join(argv) for argv in self.calls()]
        if mine:
            (self.run.ev / f"calls-{self.name}.txt").write_text(
                redact("\n".join(mine) + "\n", getattr(self.run, "secrets", ())))

    def status(self):
        if self.harness == "hermes":  # Herdr reports Hermes idle mid-turn
            # Herdr detects Hermes by its process, before its TUI has drawn: a read right after startup (or
            # during a full redraw) can come back blank once (live 2026-11-14, he-t0), so retry a few seconds.
            end = time.time() + HERMES_BLANK_READ_S
            while True:
                screen = self.run.herdr("agent", "read", self.pane, "--source", "detection", check=False).get("raw", "")
                if screen.strip() or time.time() >= end:
                    break
                time.sleep(0.5)
            if not screen.strip():
                raise RuntimeError(f"{self.name}: unreadable Hermes screen")
            return "idle" if hermes_idle(screen) else "working"
        start = time.monotonic()
        status = self.run.herdr("agent", "get", self.pane)["result"]["agent"]["agent_status"]
        proof = getattr(self, "queue_evidence", None)
        if proof is not None and proof.published:
            proof.observe_status(start, time.monotonic(), status)
        return status

    def read(self, source, *extra):
        """The pane's `source` snapshot: `agent read`, else `pane read` (once the agent has exited, e.g. at startup,
        `agent read` answers agent_not_found and only the pane still shows why), else None."""
        for kind in ("agent", "pane"):
            data = self.run.herdr(kind, "read", self.pane, "--source", source, *extra, check=False)
            if "raw" in data:
                return data["raw"]
        return None

    def visible(self):
        return self.read("visible") or ""

    def screen(self, lines=400):
        if self.harness == "opencode":  # the alternate screen keeps no history: read the session transcript
            text = self.transcript()
            if text:
                return text
        data = self.read("recent-unwrapped", "--lines", str(lines))
        return data if data is not None else self.visible()

    # ---------- opencode (channel harness) ----------
    def opencode_sessions(self):
        """Every native session the seat's top-level bindings named, ended ones included (a session rebinds)."""
        return sorted({s for (s,) in self.store_rows(
            "SELECT DISTINCT native_session FROM occupant_bindings WHERE seat_id=? AND role='top_level' "
            "AND native_session IS NOT NULL", (self.seat,)) if s})

    def transcript(self):
        return opencode_transcript(self.run.opencode_db, self.opencode_sessions())

    def ledger_path(self):
        return self.run.opencode_dirs()["ledger"] / f"{self.name}.jsonl"

    def ledger(self):
        return opencode_ledger(self.ledger_path())

    def channel_checks(self, checks, since, expect_wake, mids=()):
        """opencode's channel evidence for one case, from the plugin's ledger entries after index `since`: each of
        `mids` arrived by append (`appended`), a wake was the plugin's own submit exactly when one is expected
        (`plugin_wake_ok`; the count is `plugin_wakes`), and the composer is left empty (`composer_empty`: nothing
        was typed into it)."""
        entries = self.ledger()[since:]
        # Ledger ids carry the watch item's kind (`invitation:iX`); a message id is matched without it.
        appended = {str(i).rsplit(":", 1)[-1] for e in entries if e.get("kind") == "delivered"
                    and e.get("via") == "append" for i in e.get("ids") or []}
        mids = [m for m in mids if m]
        if mids:
            checks["appended"] = all(m in appended for m in mids)
        checks["plugin_wakes"] = sum(1 for e in entries if e.get("kind") == "submit")
        checks["plugin_wake_ok"] = (checks["plugin_wakes"] >= 1) if expect_wake else checks["plugin_wakes"] == 0
        composer = opencode_composer(self.visible())
        checks["composer_empty"] = composer == "" or (composer if composer else "composer not found")

    def opencode_admission_row(self, mid):
        """The ACK observation names, as `row`, the stored message the plugin's append returned, and that message is
        in one of this recipient's own sessions with only synthetic parts. True, or what was missing."""
        observations = [json.loads(text) for table in ("receipt_state", "receipts") for (text,) in self.store_rows(
            f"SELECT ack_observation FROM {table} WHERE message_id=? AND seat_id=? AND ack_observation IS NOT NULL",
            (mid, self.seat))]
        obs = admission_observation(observations)
        if obs is None:
            return "no ack observation"
        if obs.get("via") != "append":
            return f"via {obs.get('via')}"
        row = obs.get("row")
        if not row:
            return "no row"
        sessions = self.opencode_sessions()
        db = self.run.opencode_db
        try:
            with contextlib.closing(sqlite3.connect(db.resolve().as_uri() + "?mode=ro", uri=True, timeout=5)) as c:
                found = c.execute("SELECT session_id FROM message WHERE id=?", (row,)).fetchone()
                parts = [json.loads(d) for (d,) in c.execute("SELECT data FROM part WHERE message_id=?", (row,))]
        except (sqlite3.Error, ValueError) as e:
            return f"opencode.db unreadable: {e!r}"
        if not found:
            return f"row {row} not in opencode.db"
        if found[0] not in sessions:
            return f"row {row} in another session"
        if not parts or not all(isinstance(p, dict) and p.get("synthetic") for p in parts):
            return f"row {row} is not a synthetic append"
        return True

    def dismiss_notice(self):
        """Cancel Claude's between-turn notice (HARNESS_NOTICE) when it covers the composer; True when one was."""
        if self.harness != "claude":
            return False
        visible = self.visible()
        if not HARNESS_NOTICE.search(visible):
            return False
        self.run.herdr("agent", "send-keys", self.pane, "esc", check=False)
        time.sleep(1.0)
        return True

    def wait_idle(self, timeout=240):
        end = time.time() + timeout
        while time.time() < end:
            s = self.status()
            if s == "blocked":
                raise RuntimeError(f"{self.name} is blocked at a dialog: {self.tail()}")
            if s in IDLE:
                if self.dismiss_notice():
                    continue
                time.sleep(1.0)
                if self.status() in IDLE:
                    return
            time.sleep(0.5)
        raise RuntimeError(f"{self.name} did not become idle within {timeout}s: {self.tail()}")

    def observed_tail(self):
        """The macOS startup observation: the raw last 400 characters of the visible screen, exactly as before the
        linux mode (matching only, never written to evidence). Redacting first would shrink a credential on screen
        and pull older text, e.g. a stale "Not logged in" banner, into the window."""
        text = self.visible()
        return " | ".join(l.strip() for l in text.splitlines() if l.strip())[-400:]

    def tail(self):
        """The same screen as diagnostic text for errors and evidence: redacted whole, then the last 400 characters."""
        text = self.visible()
        return clip(" | ".join(l.strip() for l in text.splitlines() if l.strip()), 400,
                    getattr(self.run, "secrets", None), tail=True)

    def wake_window(self, expect=True):
        proof = getattr(self, "queue_evidence", None)
        qualified = bool(proof is not None and proof.published and proof.latest_publication_scope)
        window = QUEUE_WAKE_WINDOW_S if expect and qualified else WAKE_WINDOW_S
        if proof is not None:
            if len(proof.selected_windows) >= 32:
                proof.overflow = True
            else:
                proof.selected_windows.append({"seconds": window, "queue_eligible": qualified,
                                               "expect_wake": bool(expect)})
        return window

    def watch_for_wake(self, window):
        t0 = time.time()
        while time.time() - t0 < window:
            s = self.status()
            if s not in IDLE:
                return round(time.time() - t0, 2)
            time.sleep(0.25)
        return None

    def shows(self, needle):
        """Whether the pane shows `needle` (a string, or a compiled pattern) now: the live viewport (`visible`, a
        plain snapshot that never scrolls) first, then the deep transcript (`screen()`, recent-unwrapped). Either
        read is evidence of the line on the recipient's screen; a line in neither is not seen.
        Claude Code draws on the alternate screen, so Herdr assembles its recent-unwrapped history by scrolling it
        while idle, and that capture can come back stale: in the v0.11.0 run (cl-all send-ack/1:1) 15 reads in a
        row ended two turns early, the last 14 with Claude's own `Jump to bottom` hint, while the turn had printed
        the RECEIVED line and later visible reads held it. The viewport is read first, before this check's deep
        read can leave the view scrolled."""
        hit = (lambda text: needle in text) if isinstance(needle, str) else (lambda text: bool(needle.search(text)))
        visible = self.visible()
        return hit(visible) or hit(self.screen())

    def on_screen(self, needle, tries=15):
        for _ in range(tries):
            if self.shows(needle):
                return True
            time.sleep(1)
        return False

    # ---------- lifecycle ----------
    def start(self):
        end = time.time() + 30
        while time.time() < end and not re.search(r"[%$#>] ?$", self.run.herdr(
                "pane", "read", self.pane, "--source", "visible", check=False).get("raw", "").rstrip("\n ") + " "):
            time.sleep(0.5)
        # The Codex recipients share one scratch CODEX_HOME: two first starts at once race on its state DB migration
        # (seen in the linux runner: "migration 3: duplicate column name: thread_id", one Codex exits). In the linux
        # mode the second start waits until the first Codex is up: the lock is released once this Codex is detected
        # (its TUI is drawn, so its state DB is migrated) or the detection failed (the error releases it too).
        # macOS hit it too with three Codex recipients (the opt-in side cases add co-plain): every Codex start is
        # serial on both platforms. The agy recipients share one scratch HOME (~/.gemini) and each walks its first-run
        # screens, so their starts are serial too, under their own lock.
        lock = {"codex": "codex_start", "agy": "agy_start"}.get(self.harness)
        with (getattr(self.run, lock) if lock else contextlib.nullcontext()):
            if self.plain:
                line = self.run.launch_line(self.harness, plain=True)
            elif self.hooks_only:
                line = self.run.launch_line(self.harness, hooks_only=True)
            elif self.harness == "opencode":  # its own plugin ledger file
                line = self.run.launch_line(self.harness, name=self.name)
            else:
                line = self.run.launch_line(self.harness)
            self.run.herdr("pane", "run", self.pane, line)
            end = time.time() + 180
            while time.time() < end:
                if self.harness == "agy":  # a fresh scratch HOME meets agy's first-run screens every run
                    screen = self.run.herdr("pane", "read", self.pane, "--source", "visible",
                                            check=False).get("raw", "")
                    page = agy_onboarding_page(screen)
                    keys = AGY_ONBOARDING_KEYS.get(page)
                    if page:  # results.json records which first-run pages were answered, the consent untick too
                        self.run.facts.setdefault("agy_onboarding", {}).setdefault(self.name, []).append(page)
                    for i, k in enumerate(keys or ()):
                        if i:
                            time.sleep(0.5)
                        self.run.herdr("pane", "send-keys", self.pane, *k, check=False)
                    if keys or not agy_composer_up(screen):
                        time.sleep(1.5)  # Herdr may report idle behind a first-run screen
                        continue
                data = self.run.herdr("agent", "get", self.pane, check=False)
                agent = (data.get("result") or {}).get("agent") or {}
                if agent.get("agent") == self.harness and agent.get("agent_status") in IDLE + ("blocked",):
                    break
                if AUTH_REJECTED.search(self.run.herdr("pane", "read", self.pane, "--source", "visible",
                                                       check=False).get("raw", "")):
                    raise RuntimeError(f"{self.name}: {self.harness} auth rejected: {self.tail()}")
                time.sleep(1)
            else:
                raise RuntimeError(f"{self.name}: {self.harness} not detected in {self.pane}: {self.tail()}")
        if getattr(self.run.args, "harness_mode", "macos") == "linux":
            screen = self.visible()
            why = linux_startup_refusal(screen, self.status())
            if why and why.startswith("is not logged in"):
                raise RuntimeError(f"{self.name}: {self.harness} {why}")
            if why == "auth rejected":
                raise RuntimeError(f"{self.name}: {self.harness} {why}: {self.tail()}")
            if why:
                raise RuntimeError(f"{self.name}: {why}: {self.tail()}")
        else:
            if re.search(r"Not logged in|Please run /login", self.observed_tail()):
                raise RuntimeError(f"{self.name}: {self.harness} is not logged in (check --claude-config-dir)")
            if self.status() == "blocked" or DIALOG.search(self.observed_tail()):
                raise RuntimeError(f"{self.name}: startup dialog (nothing typed): {self.tail()}")
        if self.harness == "opencode":
            self.opencode_ready()
        for _attempt in range(2):  # one retry: a transient model/capacity error ends the first turn early
            reported = setup_done(self.threshold)
            self.prompt(STANDING.format(coord=self.run.coord_seats(), threshold=self.threshold)
                        + (PLAIN_HINT.format(shim=self.run.bindir / "herdr-threads") if self.plain else ""),
                        expect=reported)
            if self.shows(reported):
                break
        else:
            raise RuntimeError(f"{self.name}: agent did not report nudge-threshold {self.threshold}: {self.tail()}")
        self.enable_queue_wake()
        if self.harness == "codex" and not self.plain and getattr(getattr(self.run, "args", None), "queue_proof", False):
            from wake_e2e_queue import eligible_scope, queue_snapshot
            end, qualified = time.monotonic() + 5, None
            while time.monotonic() < end:
                bootstrap = queue_snapshot(self.run.state, self.seat)
                qualified = eligible_scope(bootstrap)
                if qualified is not None:
                    break
                self.run.stopping.wait(0.1)
            if qualified is None:
                self.run.facts.setdefault("queue_bootstrap_failures", {})[self.name] = bootstrap
                raise RuntimeError(f"{self.name}: queue qualification bootstrap lacks exact matched native Start/End")
            self.run.facts.setdefault("queue_bootstrap", {})[self.name] = qualified

    def enable_queue_wake(self):
        """--queue-proof: turn programmatic_wake on for this Codex seat only, through the human CLI of the owned
        instance (never node settings, never operator config), before the queue bootstrap and any send."""
        if self.harness != "codex" or not getattr(getattr(self.run, "args", None), "queue_proof", False):
            return
        out = self.run.ht("config", "seat", "set", "--seat", self.seat, "programmatic_wake", "on",
                          human=True, tag=f"config:{self.name}")
        self.run.facts.setdefault("queue_seats", {})[self.name] = clip(str(out).strip(), 200, getattr(self.run, "secrets", None))

    def opencode_ready(self, timeout=60):
        """opencode's home screen draws (and Herdr reports it idle) before its composer takes input: a prompt typed
        then is dropped (seen on 1.18.1: the composer still showed its placeholder). Wait for the placeholder, then
        a short settle."""
        end = time.time() + timeout
        while time.time() < end and OPENCODE_PLACEHOLDER not in self.visible():
            time.sleep(0.5)
        time.sleep(OPENCODE_SETTLE_S)

    def prompt(self, text, expect=None):
        """Submit a turn and return once it is over: `expect` (text the turn prints) seen, then settled idle.
        A prompt that returns early must never let the next case start while the recipient is still mid-turn."""
        self.run.herdr("agent", "prompt", self.pane, text, "--wait", "--timeout", str(TURN_TIMEOUT_MS),
                       timeout=TURN_TIMEOUT_MS / 1000 + 30, check=False)
        if expect:
            end = time.time() + 120
            while not self.shows(expect) and time.time() < end:
                time.sleep(1)
        self.wait_idle()
        self.settled()

    def settled(self):
        """The daemon's per-seat wake guard is 30 s from its last reservation, which may follow any turn, so the
        next case waits SPACING_S from the moment the recipient last went idle."""
        self.last_wake = time.time()

    # ---------- actions ----------
    def coord(self, *argv, role="main"):
        proof = getattr(self, "queue_evidence", None)
        if proof is not None and argv[:1] in (("send",), ("invite",)):
            proof.publication()
        return json.loads(self.run.ht(*argv, pane=self.run.coords[role]["pane"], human=True))

    def send(self, kind, body, direct, role="main", deadline_s=None):
        """Send one message of `kind` (p1 | p0 | p3 | p5 | ack, see send_flags); returns its message id."""
        argv = ["send"] + (["--seat", self.seat] if direct else [self.group]) + ["--body", body]
        argv += send_flags(kind, self.seat, deadline_s)
        data = self.coord(*argv, role=role)
        mid = (data.get("result") or {}).get("data")
        if not isinstance(mid, str) or not mid:
            raise RuntimeError(f"send returned no message id: {data}")
        return mid

    def pair_thread(self, role="main"):
        """The coordinator's 1:1 pair with this recipient (the direct send made it), or None."""
        for row in walk(self.coord("thread", "list", "--joined", role=role)):
            if str(row.get("topic_data", "")).startswith("1:1") and self.seat in str(row.get("topic_data")):
                return row.get("thread")
        return None

    def joined(self, thread, role="main"):
        data = self.coord("thread", "participants", thread, role=role)
        for row in walk(data):
            if row.get("seat") == self.seat:
                return row.get("joined") is True, row.get("effective_state")
        return False, "absent"

    def receipt(self, mid):
        return self.receipt_row(mid).get("effective_status")

    def receipt_row(self, mid):
        data = self.coord("delivery", "recipients", mid)
        for row in walk(data):
            if row.get("seat") == self.seat and "effective_status" in row:
                return row
        return {}

    def watch_quiet(self, seconds):
        """Seconds until the idle recipient started a turn on its own, or None if it stayed idle throughout."""
        t0 = time.time()
        while time.time() - t0 < seconds:
            if self.status() not in IDLE:
                return round(time.time() - t0, 2)
            time.sleep(0.5)
        return None

    def throttle_probe(self, since):
        """What `BusyBarrier.wait_entered` asks when the tool is late: a Codex 429 since `since`; None elsewhere."""
        if self.harness != "codex":
            return None
        return lambda: codex_throttle(self.run.codex_home, since)

    def idle_settled(self):
        """Idle on two samples a second apart, as `wait_idle` takes it: a turn starting between tool calls can read idle."""
        if self.status() not in IDLE:
            return False
        time.sleep(1.0)
        return self.status() in IDLE

    def enter_busy(self, hold, prompt, anchor):
        """Submit the held turn's `prompt` and wait for its tool to start (`wait_entered` says what a late tool means).
        Returns (checks, occurrence): the row's evidence of the setup, and which copy of `anchor` is the held turn: the
        last one on screen when the tool entered, the first unless the prompt went twice."""
        since, resent = time.time(), []

        def resubmit():
            resent.append(True)
            self.run.herdr("agent", "prompt", self.pane, prompt)

        self.run.herdr("agent", "prompt", self.pane, prompt)
        throttled = hold.wait_entered(throttled=self.throttle_probe(since), idle=self.idle_settled, resubmit=resubmit)
        waited, checks, occurrence = time.time() - since, {}, 0
        if throttled:
            checks["provider_throttled"] = throttled
        if waited >= BUSY_ENTRY_S:
            checks["entered_after_s"] = round(waited, 1)
        if resent:
            checks["busy_prompt_resubmitted"] = True
            occurrence = max(self.screen(1000).count(anchor) - 1, 0)
        return checks, occurrence

    # ---------- busy cases ----------
    def busy_case(self, case, record=None):
        action, channel = case.split("/")
        fail = action.startswith("busy-fail-")
        if fail:
            action = "busy-" + action[len("busy-fail-"):]
        kind = "invite" if action == "busy-invite" else action.split("-", 2)[2]  # invite | p1 | p0 | ack
        gap = SPACING_S - (time.time() - self.last_wake)
        if gap > 0:
            self.run.stopping.wait(gap)  # spacing ends early when cleanup begins
        self.wait_idle()
        tag = f"{case} {self.run.run_id}"
        ledger_before = len(self.ledger()) if self.harness == "opencode" else 0
        with BusyBarrier(self.run.root) as hold:
            if action.startswith("busy-tool-"):
                prompt = BUSY_TOOL_PROMPT.format(tag=tag, hold=hold.command(), tool=BUSY_TOOL[self.harness].format(dir=hold.path))
            else:
                prompt = (BUSY_FAIL_PROMPT if fail else BUSY_PROMPT).format(tag=tag, hold=hold.command())
            entry, occurrence = self.enter_busy(hold, prompt, f"Busy-turn test {tag}")
            busy_at_send = self.status()
            checks = {"busy_at_send": busy_at_send, **entry}
            role = "main"
            if kind == "invite" and channel == "1:1":
                # First contact from the fresh coordinator, mid-turn: the pair auto-joins (no invitation) and the opener
                # is a p1 message that must arrive inside the turn.
                role = "fresh"
                ref = {"message": self.send("p1", f"pair-open-busy {self.run.run_id}", direct=True, role=role)}
                thread = self.pair_thread(role=role)
                ref["thread"] = thread
            elif kind == "invite":
                name = f"wake-e2e-busy-{self.name}-{self.run.run_id}"
                thread = self.coord("thread", "create", "--name", name, "--topic", f"wake-e2e busy test thread for {self.name}",
                                    "--goal", "wake e2e test traffic")["result"]["data"]
                self.coord("invite", thread, "--seat", self.seat)
                ref = {"thread": thread}
            else:
                ref = {"message": self.send(kind, f"{case} {self.run.run_id}", direct=channel == "1:1")}
            hold.assert_held()
        anchor = f"Busy-turn test {tag}"
        # The quiet window starts at the busy turn's native end (its BUSY-DONE line, then the first idle status),
        # not after a confirmed-idle wait: that wait would absorb a short wake turn typed right after the turn.
        end, idle_since = time.time() + TURN_TIMEOUT_MS / 1000, None
        while time.time() < end and not (busy_turn_split(self.screen(1000), anchor, occurrence=occurrence) or ("", "", False))[2]:
            idle_since = (idle_since or time.time()) if self.status() in IDLE else None
            if idle_since and time.time() - idle_since > BUSY_END_GRACE_S:
                break  # recorded below as busy_turn_end_seen False, without waiting out the turn timeout
            time.sleep(0.5)
        while time.time() < end and self.status() not in IDLE:
            time.sleep(0.25)
        # A wake owed after the turn (even one held by the 30 s per-seat guard) would start a turn in here.
        checks["self_started_turn_after_s"] = self.watch_quiet(BUSY_AFTER_S)
        self.wait_idle()
        split = busy_turn_split(self.screen(1000), anchor, occurrence=occurrence)
        if split is None:
            in_turn, after_turn, checks["busy_turn_end_seen"] = "", "", "busy prompt not on screen"
        else:
            in_turn, after_turn, checks["busy_turn_end_seen"] = split
        # Rule 4: no wake is typed into the busy turn; a wake after it means an item was not served in-turn.
        checks["wake_prompts_in_busy_turn"] = wake_count(in_turn)
        checks["wake_prompts_after_busy_turn"] = wake_count(after_turn)
        if kind == "invite":
            ok = False
            for _ in range(10):
                ok, _state = self.joined(thread, role=role) if thread else (False, "no pair thread")
                if ok:
                    break
                time.sleep(1)
            checks["joined"] = ok
            if channel == "1:1":
                checks["opening_message_delivered_in_turn"] = ref["message"] in in_turn
        else:
            checks["delivered_in_turn"] = ref["message"] in in_turn
            if action.startswith("busy-tool-"):
                checks["tool_ran"] = ((hold.path / "patched.txt").exists() if self.harness == "codex"
                                      else re.search(r"\bRead(\(| \d+ files?\b)", in_turn) is not None)
            if kind == "ack":
                checks["ack"] = self.receipt(ref["message"])
        if self.harness == "opencode":  # served in the turn: appended, never a wake
            self.channel_checks(checks, ledger_before, False, [ref.get("message")])
        self.settled()
        passed = (busy_at_send not in IDLE and checks["self_started_turn_after_s"] is None
                  and checks["busy_turn_end_seen"] is True
                  and checks["wake_prompts_in_busy_turn"] == 0 and checks["wake_prompts_after_busy_turn"] == 0
                  and all(v is True or v == "acknowledged" for k, v in checks.items()
                          if k in ("joined", "opening_message_delivered_in_turn", "delivered_in_turn", "ack",
                                   "tool_ran", "appended", "plugin_wake_ok", "composer_empty")))
        self.run.record({"case": record or case, "harness": self.harness, "threshold": self.threshold, "recipient": self.name,
                         "expect_wake": False,
                         "woke": checks["self_started_turn_after_s"] is not None
                         or checks["wake_prompts_in_busy_turn"] != 0 or checks["wake_prompts_after_busy_turn"] != 0,
                         "t_wake_s": None,
                         "hello_sent": False, "checks": checks, "ref": ref,
                         "result": "PASS" if passed else "FAIL", "utc": utc()})

    # ---------- end-of-turn case (ht-end-of-turn) ----------
    def rollout_hook_prompts(self):
        """Every `<hook_prompt>` text in the run's scratch Codex rollouts (the Stop continuation's record)."""
        found = []
        for path in sorted((self.run.codex_home / "sessions").rglob("*.jsonl")):
            for line in path.read_text(errors="replace").splitlines():
                if "<hook_prompt" in line:
                    found.append(line)
        return found

    def eot_case(self, case):
        gap = SPACING_S - (time.time() - self.last_wake)
        if gap > 0:
            self.run.stopping.wait(gap)  # spacing ends early when cleanup begins
        self.wait_idle()
        tag = f"{case} {self.run.run_id}"
        with BusyBarrier(self.run.root) as hold:
            entry, occurrence = self.enter_busy(hold, EOT_PROMPT.format(tag=tag, hold=hold.command()),
                                                f"End-of-turn test {tag}")
            busy_at_send = self.status()
            checks = {"busy_at_send": busy_at_send, **entry}
            mid = self.send("p0", f"{case} {self.run.run_id}", direct=False)
            hold.assert_held()
        anchor = f"End-of-turn test {tag}"
        split_at = lambda: busy_turn_split(self.screen(1000), anchor, EOT_DONE_LINE, EOT_PROMPT_END, occurrence=occurrence, last=True)
        end, idle_since = time.time() + TURN_TIMEOUT_MS / 1000, None
        while time.time() < end:
            idle_since = (idle_since or time.time()) if self.status() in IDLE else None
            if idle_since and time.time() - idle_since > BUSY_END_GRACE_S:
                break
            time.sleep(0.5)
        checks["self_started_turn_after_s"] = self.watch_quiet(BUSY_AFTER_S)
        self.wait_idle()
        split = split_at()
        if split is None:
            in_turn, after_turn, checks["turn_end_seen"] = "", "", "prompt not on screen"
        else:
            in_turn, after_turn, checks["turn_end_seen"] = split
        checks["wake_prompts_in_turn"] = wake_count(in_turn)
        checks["wake_prompts_after_turn"] = wake_count(after_turn)
        prompts = self.rollout_hook_prompts()
        checks["continuation_names_message"] = any(mid in p for p in prompts)
        checks["hook_prompts"] = len(prompts)
        checks["displayed"] = bool(self.displayed_by(self.binding_ids(), mid))
        self.settled()
        passed = (busy_at_send not in IDLE and checks["self_started_turn_after_s"] is None
                  and checks["turn_end_seen"] is True
                  and checks["wake_prompts_in_turn"] == 0 and checks["wake_prompts_after_turn"] == 0
                  and checks["continuation_names_message"] is True and checks["displayed"] is True)
        self.run.record({"case": case, "harness": self.harness, "threshold": self.threshold, "recipient": self.name,
                         "expect_wake": False,
                         "woke": checks["self_started_turn_after_s"] is not None
                         or checks["wake_prompts_in_turn"] != 0 or checks["wake_prompts_after_turn"] != 0,
                         "t_wake_s": None, "hello_sent": False, "checks": checks, "ref": {"message": mid},
                         "result": "PASS" if passed else "FAIL", "utc": utc()})

    # ---------- side cases (ht-327) ----------
    def store_rows(self, sql, args=()):
        """Read-only rows from the run's own daemon store (never a shared one)."""
        rows = []
        for db in sorted(self.run.state.glob("node/herdr-threads.db")):
            con = sqlite3.connect(f"file:{db}?mode=ro", uri=True, timeout=5)
            try:
                rows += con.execute(sql, args).fetchall()
            finally:
                con.close()
        return rows

    def admission_row(self, mid, timeout=5.0):
        """Claude mod idle ACK (ht-admission-ack): the receipt's ACK observation names, as `row`, the transcript row
        the engine stored for the append that carried `mid`, and that row is in this recipient's own session
        transcript. Polls briefly: nothing orders the engine's transcript write before the ACK. True, or what was
        missing."""
        observations = [json.loads(text) for table in ("receipt_state", "receipts") for (text,) in self.store_rows(
            f"SELECT ack_observation FROM {table} WHERE message_id=? AND seat_id=? AND ack_observation IS NOT NULL",
            (mid, self.seat))]
        obs = admission_observation(observations)
        if obs is None:
            return "no ack observation"
        if obs.get("via") != "append":
            return f"via {obs.get('via')}"
        if not obs.get("row"):
            return "no row"
        cfg = self.run.claude_cfg if self.run.linux else Path(
            self.run.args.claude_config_dir or Path.home() / ".claude")
        sessions = {b["session"] for b in self.bindings() if b["session"]}
        deadline = time.time() + timeout
        while True:
            found = transcript_has_row(cfg, sessions, obs["row"])
            if found is True or time.time() >= deadline:
                return found
            time.sleep(0.25)

    def bindings(self):
        """The seat's open top-level bindings. `concurrent` and `auxiliary` read the `/side` bits; a store without the
        column projects None (unknown, never "not auxiliary")."""
        present = {row[1] for row in self.store_rows("PRAGMA table_info(occupant_bindings)")}
        optional = [(key, column if column in present else "NULL")
                    for key, column in (("concurrent", "concurrent_session"), ("auxiliary", "auxiliary_session"))]
        keys = ("session", "provenance", "liveness", "pid", "disconnect") + tuple(key for key, _ in optional)
        return [dict(zip(keys, row)) for row in self.store_rows(
            "SELECT native_session,observation_provenance,liveness,liveness_pid,disconnect_reason,"
            + ",".join(column for _, column in optional)
            + " FROM occupant_bindings WHERE seat_id=? AND role='top_level' AND ended_at IS NULL ORDER BY ordinal",
            (self.seat,))]

    def hook_trace(self):
        """This pane's hook-trace records (empty when the run has no trace, as in the self-test fakes)."""
        ev = getattr(self.run, "ev", None)
        return [r for r in read_hook_trace(ev / HOOK_TRACE_FILE) if r.get("pane") == self.pane] if ev else []

    def binding_ids(self):
        return {row[0] for row in self.store_rows(
            "SELECT binding_id FROM occupant_bindings WHERE seat_id=? AND role='top_level'", (self.seat,))}

    def displayed_by(self, bindings, mid):
        """Which of `bindings` displayed message `mid`: its cursor on the thread passed the message's sequence, or
        a displayed mark holds it (the store's own per-binding view, `binding_lazy_unread_sql!`)."""
        found = set()
        for binding in bindings:
            found |= {row[0] for row in self.store_rows(
                "SELECT b.binding_id FROM occupant_bindings b JOIN messages m ON m.id=? WHERE b.binding_id=? AND "
                "(EXISTS(SELECT 1 FROM binding_cursors c WHERE c.binding_id=b.binding_id AND c.thread_id=m.thread_id "
                "AND c.seq>=m.sequence) OR EXISTS(SELECT 1 FROM binding_row_displays d WHERE "
                "d.binding_id=b.binding_id AND d.message_id=m.id))", (mid, binding))}
        return sorted(found)

    def sibling_phase(self, checks, ref, tag):
        """side-nudge, sibling-displayed (ht-yha.62): a third `/side` turn is busy when a second p0 is sent, then
        its own `inbox` displays it; back on the idle main thread the process must still be woken for it."""
        before = self.binding_ids()
        self.wait_idle()
        self.run.herdr("agent", "prompt", self.pane, "/side", check=False)
        time.sleep(2)
        shim = self.run.bindir / "herdr-threads"
        self.run.herdr("agent", "prompt", self.pane, SIBLING_SIDE_PROMPT.format(tag=tag, shim=shim), check=False)
        end = time.time() + 15
        while self.status() in IDLE and time.time() < end:
            time.sleep(0.25)
        checks["sibling_side_busy"] = self.status() not in IDLE
        sent_ms = int(time.time() * 1000)
        ref["sibling_message"] = mid = self.send("p0", tag + " sibling", direct=False)
        end = time.time() + 120
        while not self.shows(SIBLING_SIDE_DONE) and time.time() < end:
            time.sleep(1)
        self.wait_idle()
        side = sorted(self.binding_ids() - before)
        checks["sibling_side_bindings"] = side
        checks["sibling_displayed_by"] = self.displayed_by(side, mid)
        checks["sibling_displayed"] = bool(checks["sibling_displayed_by"])
        checks["sibling_close"] = self.close_side_checked()
        self.wait_idle()
        # A wake attempted while the side turn was busy is refused and retried after the per-seat guard.
        checks["sibling_wake_s"] = self.watch_for_wake(SPACING_S + self.wake_window())
        checks["sibling_wake_state"] = state = self.wake_state()
        # The retried wake can land while the side turn is being interrupted, before the watch starts: count it when
        # the daemon submitted a wake after the send and the main session printed the message in that wake's turn.
        checks["sibling_woke_before_watch"] = (
            checks["sibling_wake_s"] is None and bool(state) and state.get("last_outcome") == "submitted"
            and (state.get("completed_at_utc") or 0) >= sent_ms and received_in_wake_turn(self.screen(1000), mid))
        if checks["sibling_wake_s"] is None and not checks["sibling_woke_before_watch"]:
            checks["sibling_hello_sent"] = True
            self.prompt(self.follow_up())
        self.wait_idle()
        checks["sibling_delivered"] = self.on_screen(mid)

    def unreceived_case(self, case):
        """wake-unreceived: a wake whose session never reaches the daemon is recorded unreceived, not retyped."""
        tag = f"{case} {self.run.run_id}"
        gap = SPACING_S - (time.time() - self.last_wake)
        if gap > 0:
            self.run.stopping.wait(gap)
        self.wait_idle()
        checks, ref, hello = {}, {}, False
        reservation = lambda: (self.store_rows("SELECT last_reservation_id FROM wake_work WHERE seat_id=?",
                                               (self.seat,)) or [[None]])[0][0]
        before = reservation()
        original = Path(self.run.bin)
        private = self.run.bindir / "herdr-threads-coordinator"
        shutil.copy2(original, private)
        mode = original.stat().st_mode
        self.run.bin = str(private)
        try:
            # An ACK request: its receipt keeps the wake owed, so the ladder would retype it (a bare p0 row is
            # reserved once and never retried).
            ref["message"] = self.send("ack", tag, direct=False)
            original.chmod(0o600)  # before the wake is typed: the dispatcher's reads take longer than this
            end = time.time() + UNRECEIVED_VERDICT_S
            state = None
            while time.time() < end and not self.run.stopping.is_set():
                state = self.wake_state()
                if (state or {}).get("last_outcome") == "unreceived":
                    break
                time.sleep(1)
            checks["verdict_state"] = state
            woken = reservation()
            checks["wake_typed"] = woken is not None and woken != before
            # Past the second ladder step (60 s): a retype would have reserved again.
            self.run.stopping.wait(SPACING_S + 30)
            checks["after_state"] = self.wake_state()
            checks["retyped"] = reservation() != woken
        finally:
            original.chmod(mode)
            self.run.bin = str(original)
            private.unlink(missing_ok=True)
        self.wait_idle()
        hello = True
        self.prompt(self.follow_up())
        woke, t_wake = checks["wake_typed"], None
        self.wait_idle()
        checks["delivered"] = self.on_screen(ref["message"])
        self.settled()
        # The verdict lands when the lane next examines the seat after the retry is due (a safety tick later).
        unreceived = any((checks[k] or {}).get("last_outcome") == "unreceived" for k in ("verdict_state", "after_state"))
        passed = checks["wake_typed"] and unreceived and not checks["retyped"] and checks["delivered"]
        self.run.record({"case": case, "harness": self.harness, "threshold": self.threshold, "recipient": self.name,
                         "expect_wake": True, "woke": woke, "t_wake_s": t_wake, "hello_sent": hello,
                         "checks": checks, "ref": ref, "result": "PASS" if passed else "FAIL", "utc": utc()})

    def wake_state(self):
        rows = self.store_rows("SELECT last_outcome,completed_at_utc,retry_step FROM wake_work WHERE seat_id=?",
                               (self.seat,))
        return dict(zip(("last_outcome", "completed_at_utc", "retry_step"), rows[0])) if rows else None

    def bindings_idle(self, timeout=60):
        """Every open top-level binding of the recipient reads idle: each binding with a `native_turn_state` row
        (hooked Codex records them) is in phase `ended`; a binding with no turn row has no signal of its own, so
        the pane's `wait_idle()` and the printed done marker (`settled()`) stand in for it."""
        self.wait_idle()
        end = time.time() + timeout
        while True:
            phases = [row[0] for row in self.store_rows(
                "SELECT n.phase FROM native_turn_state n JOIN occupant_bindings b ON b.binding_id=n.binding_id "
                "WHERE b.seat_id=? AND b.role='top_level' AND b.ended_at IS NULL", (self.seat,))]
            if all(phase == "ended" for phase in phases):
                return True
            if time.time() >= end or self.run.stopping.is_set():
                return False
            time.sleep(0.5)

    def wait_new_side_binding(self, known, timeout=SIDE_BINDING_WAIT_S):
        """Poll the seat's bindings until one whose session is not in `known` appears (the `/side` fork's own
        binding, registered by its first `inbox` call); returns the bindings, or None when none appeared in time."""
        end = time.time() + timeout
        while True:
            sides = self.bindings()
            if any(b["session"] and b["session"] not in known for b in sides):
                return sides
            if time.time() >= end or self.run.stopping.is_set():
                return None
            time.sleep(0.5)

    def open_sides(self, tag):
        """SIDE_FORKS `/side` conversations, each making one herdr-threads `inbox` call under its own session and
        each provably bound as a NEW session before it is closed (`close_side_checked`: Codex answers Ctrl+C on an
        idle side chat with an `Archive ...?` dialog, and a bare Ctrl+C left the first chat displayed, so every
        later `/side` and prompt ran inside it: ht-z41e.54). Returns what the store holds for the seat afterwards;
        `self.side_setup` records, per side, whether a new binding appeared and what the close saw."""
        self.side_setup = []
        known = {b["session"] for b in self.bindings() if b["session"]}
        for i in range(SIDE_FORKS):
            self.wait_idle()
            self.run.herdr("agent", "prompt", self.pane, "/side", check=False)
            time.sleep(2)
            shim = self.run.bindir / "herdr-threads"
            # `inbox` (nothing is pending here) is a call that enrolls the side session's own binding; a read-only
            # `thread list` does not.
            self.prompt(f"Side {i + 1} of wake-e2e {tag}: run the shell command `{shim} inbox` once, then print "
                        f"SIDE-{i + 1}-DONE. Do not act on anything it shows.",
                        expect=re.compile(r"^[^\w\n]*SIDE-%d-DONE" % (i + 1), re.M))
            bound = self.wait_new_side_binding(known)
            if bound is not None:
                known |= {b["session"] for b in bound if b["session"]}
            self.side_setup.append({"side": i + 1, "new_binding": bound is not None, "close": self.close_side_checked()})
            self.wait_idle()
        self.settled()
        return self.bindings()

    def follow_up(self):
        """The neutral next turn; a hookless recipient has no turn-start delivery, so it reads its inbox."""
        if self.plain:
            return (f"Run `{self.run.bindir / 'herdr-threads'} inbox` once and handle what it shows as your standing "
                    "instructions say.")
        return "hello"

    def queue_root_session(self):
        """Under --queue-proof, the hooked Codex's root session as its queue bootstrap qualified it (before any side
        opened), else None: a queue wake targets exactly this session, never a `/side` one."""
        if self.plain or not getattr(getattr(self.run, "args", None), "queue_proof", False):
            return None
        bootstrap = (getattr(self.run, "facts", None) or {}).get("queue_bootstrap", {}).get(self.name)
        return bootstrap["scope"]["native_session"] if bootstrap else None

    def open_displayed_side(self, tag, done):
        """One `/side` chat that runs one `inbox` turn (so its binding registers) and is LEFT displayed (no Ctrl+C),
        its composer showing. Returns what the store holds for the seat afterwards."""
        self.wait_idle()
        self.run.herdr("agent", "prompt", self.pane, "/side", check=False)
        time.sleep(2)
        shim = self.run.bindir / "herdr-threads"
        self.prompt(f"Side of wake-e2e {tag}: run the shell command `{shim} inbox` once, then print {done}. Do not act "
                    "on anything it shows.", expect=re.compile(r"^[^\w\n]*" + re.escape(done), re.M))
        self.settled()
        return self.bindings()

    def detection(self):
        return self.run.herdr("pane", "read", self.pane, "--source", "detection", check=False).get("raw", "")

    def close_side(self):
        self.run.herdr("agent", "send-keys", self.pane, "ctrl+c", check=False)
        time.sleep(2)

    def side_queue_case(self, case):
        """side-queue (--queue-proof): a p0 sent while a /side chat is displayed is an Accepted queue wake on the root
        session; the item is delivered in the root's turn and the side chat's pane is untouched."""
        tag = f"{case} {self.run.run_id}"
        proof, root = getattr(self, "queue_evidence", None), self.queue_root_session()
        if proof is None or not root:
            raise RuntimeError(f"{self.name}: side-queue needs the queue evidence sampler and the bootstrap root session")
        proof.root_session = root
        sides = self.open_displayed_side(tag, "SIDE-QUEUE-DONE")
        checks = {"bindings_after_side": sides, "root_session": root}
        setup_ok = len(sides) >= 2 and any(b["session"] == root for b in sides)
        checks["side_setup_ok"] = setup_ok
        gap = SPACING_S - (time.time() - self.last_wake)
        if gap > 0:
            self.run.stopping.wait(gap)
        self.wait_idle()
        before_screen = self.detection()
        sent_ms = int(time.time() * 1000)
        ref = {"message": self.send("p0", tag, direct=False)}
        t0, activity = time.monotonic(), None
        while time.monotonic() - t0 < QUEUE_WAKE_WINDOW_S + 10 and not self.run.stopping.is_set():
            activity = proof.qualified_activity()
            if activity is not None:
                break
            time.sleep(0.5)
        t_wake = round(time.monotonic() - t0, 2) if activity is not None else None
        after_screen = self.detection()
        checks["side_typed"], checks["side_view_changed"] = side_typed, changed = side_pane_verdict(before_screen, after_screen)
        checks["side_view_before"], checks["side_view_after"] = side_view_state(before_screen), side_view_state(after_screen)
        checks["activity_evidence"] = activity
        delivered = False
        if activity is not None and activity["native_session"] == root:
            from wake_e2e_queue import codex_turn_delivery
            end, delivery = time.monotonic() + TURN_TIMEOUT_MS / 1000, {"passed": False}
            while time.monotonic() < end and not self.run.stopping.is_set():
                delivery = codex_turn_delivery(self.run.codex_home, root, activity["turn_id"], {ref["message"]: tag})
                if delivery.get("complete") or delivery.get("error"):
                    break
                self.run.stopping.wait(0.5)
            checks["delivery_evidence"] = delivery
            delivered = delivery["passed"]
        checks["delivered_in_root"] = delivered
        # The unreceived verdict lands at the first retry; read the settlement only once it is due, with the side chat
        # still displayed so its close cannot add a side-session event to the window.
        due = sent_ms / 1000 + UNRECEIVED_VERDICT_S - time.time()
        if due > 0:
            self.run.stopping.wait(due)
        state, trace = self.wake_state(), self.hook_trace()
        after = self.bindings()
        side_sessions = {b["session"] for b in after if b["session"] and b["session"] != root}
        bits = lambda b: {"auxiliary": b["auxiliary"], "concurrent": b["concurrent"]}
        side_bits = {b["session"]: bits(b) for b in after if b["session"] in side_sessions}
        root_bits = next((bits(b) for b in after if b["session"] == root), None)
        first_event = {}
        for session in sorted(side_sessions):
            first = next((r for r in trace if r.get("session") == session), None)
            first_event[session] = ({k: first.get(k) for k in ("event", "source", "transcript_path")}
                                    if first else None)
        settled = settled_by_root(state, trace, root, side_sessions, sent_ms)
        checks.update(side_bits=side_bits, root_bits=root_bits, side_first_event=first_event,
                      settled_by_root=settled, wake_state_final=state)
        self.close_side()  # later cases start on the main thread
        self.wait_idle()
        self.settled()
        proof_now, _ = proof.current_proof()
        passed = side_queue_passed(setup_ok, proof_now, delivered, side_typed, changed,
                                   side_bits=side_bits, root_bits=root_bits, settled_by_root=settled)
        self.run.record({"case": case, "harness": self.harness, "threshold": self.threshold, "recipient": self.name,
                         "expect_wake": True, "woke": activity is not None, "t_wake_s": t_wake, "hello_sent": False,
                         "checks": checks, "ref": ref, "result": "PASS" if passed else "FAIL", "utc": utc()})

    def agent_get(self):
        return self.run.herdr("agent", "get", self.pane, check=False).get("result", {}).get("agent", {}) or {}

    def focus_recipient(self, timeout=10):
        """Focus this recipient's pane (a private Herdr only) and return whether `agent get` reads it focused."""
        self.run.herdr("agent", "focus", self.pane, check=False)
        end = time.time() + timeout
        while time.time() < end and not self.run.stopping.is_set():
            if self.agent_get().get("focused") is True:
                return True
            time.sleep(0.5)
        return False

    def restore_coordinator_focus(self):
        tab = getattr(self.run, "coordinator_tab", None)
        if tab:
            self.run.herdr("tab", "focus", tab, check=False)

    def diagnose(self):
        """What the daemon says about this seat's wake and recent refusals (bounded, for a failing row's evidence)."""
        out = {}
        try:
            out["doctor_wake"] = self.doctor_wake()
        except (RuntimeError, ValueError) as e:
            out["doctor_wake"] = "unreadable: " + clip(e, 200, getattr(self.run, "secrets", None))
        try:
            text = self.run.ht("diagnostics", "--seat", self.seat, human=False, check=False)
            out["diagnostics"] = clip(text.strip(), 4000, getattr(self.run, "secrets", None), tail=True)
        except (RuntimeError, ValueError) as e:
            out["diagnostics"] = "unreadable: " + clip(e, 200, getattr(self.run, "secrets", None))
        return out

    def close_side_checked(self):
        """Ctrl+C the displayed side chat; Codex answers an idle side with an `Archive ...?` confirmation (the right
        panel), which is confirmed so the root view is back for the next case. A Ctrl+C whose screen shows neither the
        dialog nor a main view (the side chat's own composer placeholder still displayed: the key landed before the
        dialog could draw, or a hookless side chat that still ignores it, ht-z41e.54) is repeated every
        SIDE_CLOSE_RETRY_S, at most SIDE_CLOSE_ATTEMPTS times. Returns what
        was seen."""
        seen = {"archive_dialog": False, "attempts": 0}
        for _ in range(SIDE_CLOSE_ATTEMPTS):
            self.close_side()
            seen["attempts"] += 1
            screen = self.detection()
            if not archive_dialog_open(screen) and side_chat_displayed(screen):
                time.sleep(1)  # the dialog may still be drawing
                screen = self.detection()
            if archive_dialog_open(screen):
                seen["archive_dialog"] = True
                self.run.herdr("agent", "send-keys", self.pane, "right", check=False)
                self.run.herdr("agent", "send-keys", self.pane, "enter", check=False)
                time.sleep(2)
                after = self.detection()
                seen["archive_dialog_after"] = archive_dialog_open(after)
                if not seen["archive_dialog_after"] and not side_chat_displayed(after):
                    break
            elif not side_chat_displayed(screen):
                break
            else:
                time.sleep(SIDE_CLOSE_RETRY_S)
        return seen

    def root_native_phase(self, root):
        """The root session's native turn phase now (None without a row)."""
        for row in self.store_rows(
                "SELECT n.phase FROM native_turn_state n JOIN occupant_bindings b ON b.binding_id=n.binding_id "
                "WHERE b.seat_id=? AND b.role='top_level' AND b.ended_at IS NULL AND b.native_session=?",
                (self.seat, root)):
            return row[0]
        return None

    def side_native_phases(self, root):
        return [row[0] for row in self.store_rows(
            "SELECT n.phase FROM native_turn_state n JOIN occupant_bindings b ON b.binding_id=n.binding_id "
            "WHERE b.seat_id=? AND b.role='top_level' AND b.ended_at IS NULL AND b.native_session<>?",
            (self.seat, root))]

    def watch_queue_activity(self, proof, seconds):
        """Poll the queue proof until a qualified fulfilled activity or `seconds`; (activity or None, seconds taken)."""
        t0, activity = time.monotonic(), None
        while time.monotonic() - t0 < seconds and not self.run.stopping.is_set():
            activity = proof.qualified_activity()
            if activity is not None:
                break
            time.sleep(0.5)
        return activity, round(time.monotonic() - t0, 2)

    def root_delivery(self, root, activity, mid, tag):
        """codex_turn_delivery of the fulfilling turn in the root session; (delivered, evidence)."""
        if activity is None or activity["native_session"] != root:
            return False, None
        from wake_e2e_queue import codex_turn_delivery
        end, delivery = time.monotonic() + TURN_TIMEOUT_MS / 1000, {"passed": False}
        while time.monotonic() < end and not self.run.stopping.is_set():
            delivery = codex_turn_delivery(self.run.codex_home, root, activity["turn_id"], {mid: tag})
            if delivery.get("complete") or delivery.get("error"):
                break
            self.run.stopping.wait(0.5)
        return bool(delivery["passed"]), delivery

    def side_queue_focused_case(self, case):
        """side-queue-focused (--queue-proof, private Herdr): with the recipient pane focused and a side turn busy, a p0
        arms an Accepted queue intent on the root at once (no focus dwell) and types nothing into the side."""
        tag = f"{case} {self.run.run_id}"
        proof, root = getattr(self, "queue_evidence", None), self.queue_root_session()
        if proof is None or not root:
            raise RuntimeError(f"{self.name}: {case} needs the queue evidence sampler and the bootstrap root session")
        proof.root_session = root
        sides = self.open_displayed_side(tag, "SIDE-FOCUS-DONE")
        setup = {"side_setup_ok": len(sides) >= 2 and any(b["session"] == root for b in sides)}
        checks = {"bindings_after_side": sides, "root_session": root, "setup": setup}
        ref, activity, t_wake, passed = {}, None, None, False
        screens = checks["screens"] = {}
        try:
            # The per-seat spacing comes BEFORE the busy side turn: the turn must still be running when the p0 is sent.
            gap = SPACING_S - (time.time() - self.last_wake)
            if gap > 0:
                self.run.stopping.wait(gap)
            self.wait_idle()
            setup["focused"] = self.focus_recipient()
            self.run.herdr("agent", "prompt", self.pane, SIDE_FOCUS_PROMPT.format(tag=tag, sleep=SIDE_FOCUS_SLEEP_S),
                           check=False)
            end = time.time() + 30
            while time.time() < end and not self.run.stopping.is_set():
                if any(phase != "ended" for phase in self.side_native_phases(root)) and self.status() not in IDLE:
                    break
                time.sleep(0.5)
            screens["busy_side"] = screen_tail(self.detection())
            # Re-read at the send itself: the facts the verdict judges are the ones true when the p0 left.
            setup["side_busy"] = any(phase != "ended" for phase in self.side_native_phases(root))
            agent = self.agent_get()
            setup["agent_status"] = agent.get("agent_status")
            setup["focused"] = bool(setup["focused"] and agent.get("focused") is True)
            before_screen = self.detection()
            sent_ms = int(time.time() * 1000)
            ref["message"] = mid = self.send("p0", tag, direct=False)
            activity, took = self.watch_queue_activity(proof, FOCUSED_DWELL_S + 10)
            t_wake = took if activity is not None else None
            after_screen = self.detection()
            screens["after_send"] = screen_tail(after_screen)
            checks["daemon_view"] = self.diagnose()
            side_typed, _changed = side_pane_verdict(before_screen, after_screen)
            proof_now, samples = proof.current_proof()
            armed_s = queue_arm_s(samples)
            first = first_attempt_accepted(samples, proof_now, sent_ms)
            root_busy = root_busy_before_arm(samples, root)
            delivered, delivery = self.root_delivery(root, activity, mid, tag)
            checks.update(side_typed=side_typed, armed_s=armed_s, first_attempt=first, root_busy_at_arm=root_busy,
                          activity_evidence=activity, delivery_evidence=delivery, delivered_in_root=delivered,
                          side_view_before=side_view_state(before_screen), side_view_after=side_view_state(after_screen),
                          agent_status_at_send=setup["agent_status"], side_phase_at_send=self.side_native_phases(root))
            due = sent_ms / 1000 + UNRECEIVED_VERDICT_S - time.time()
            if due > 0:
                self.run.stopping.wait(due)
            state, trace = self.wake_state(), self.hook_trace()
            after = self.bindings()
            side_sessions = {b["session"] for b in after if b["session"] and b["session"] != root}
            bits = lambda b: {"auxiliary": b["auxiliary"], "concurrent": b["concurrent"]}
            side_bits = {b["session"]: bits(b) for b in after if b["session"] in side_sessions}
            root_bits = next((bits(b) for b in after if b["session"] == root), None)
            settled = settled_by_root(state, trace, root, side_sessions, sent_ms, busy_side=True)
            checks.update(side_bits=side_bits, root_bits=root_bits, settled_by_root=settled, wake_state_final=state)
            end = time.time() + 120
            while not self.shows(SIDE_FOCUS_BUSY_DONE) and time.time() < end and not self.run.stopping.is_set():
                time.sleep(1)
            screens["side_done"] = screen_tail(self.detection())
            proof_final, _ = proof.current_proof()
            passed = side_queue_focused_passed(setup, proof_final, armed_s, first, delivered, side_typed, root_busy,
                                               side_bits=side_bits, root_bits=root_bits, settled_by_root=settled)
        finally:
            self.restore_coordinator_focus()
            checks["close_side"] = self.close_side_checked()
            self.wait_idle()
            self.settled()
        checks["release_blocking"] = release_blocking(checks)
        passed = passed and not checks["release_blocking"]
        self.run.record({"case": case, "harness": self.harness, "threshold": self.threshold, "recipient": self.name,
                         "expect_wake": True, "woke": activity is not None, "t_wake_s": t_wake, "hello_sent": False,
                         "checks": checks, "ref": ref, "result": "PASS" if passed else "FAIL", "utc": utc()})

    def root_draft_queue_case(self, case):
        """root-draft-queue (--queue-proof): a draft in the root's own composer (no /side) refuses the queue lane and is
        left intact; once it is cleared the wake is an Accepted queue intent and the item arrives."""
        tag = f"{case} {self.run.run_id}"
        proof, root = getattr(self, "queue_evidence", None), self.queue_root_session()
        if proof is None or not root:
            raise RuntimeError(f"{self.name}: {case} needs the queue evidence sampler and the bootstrap root session")
        proof.root_session = root
        checks, ref, draft_set = {"root_session": root}, {}, False
        self.wait_idle()
        gap = SPACING_S - (time.time() - self.last_wake)
        if gap > 0:
            self.run.stopping.wait(gap)
        self.wait_idle()
        try:
            self.run.herdr("pane", "send-text", self.pane, ROOT_DRAFT)
            draft_set = True
            time.sleep(1)
            typed_screen = self.detection()
            checks["draft_typed"] = composer_holds(typed_screen, ROOT_DRAFT)
            sent_ms = int(time.time() * 1000)
            ref["message"] = mid = self.send("p0", tag, direct=False)
            end = time.monotonic() + QUEUE_WAKE_WINDOW_S + SPACING_S
            while time.monotonic() < end and not self.run.stopping.is_set():
                time.sleep(0.5)
            _proof, samples = proof.current_proof()
            screen = self.detection()
            trace = [r for r in self.hook_trace() if r.get("session") == root and r["utc_ms"] >= sent_ms
                     and r.get("event") == "UserPromptSubmit"]
            checks["intents_while_draft"] = [{k: i.get(k) for k in ("ordinal", "attempt", "phase", "logical_status")}
                                             for sample in samples for i in sample.get("intents", ())]
            checks["no_wake_while_draft"] = not intents_took_effect(samples) and not trace
            checks["daemon_view"] = self.diagnose()
            checks["wake_state_while_draft"] = self.wake_state()
            checks["draft_intact"] = root_draft_intact(typed_screen, screen)
            checks["draft_submitted"] = bool(trace) or self.root_native_phase(root) not in (None, "ended")
        finally:
            if draft_set:
                self.run.herdr("agent", "send-keys", self.pane, "ctrl+u", check=False)
                time.sleep(1)
        checks["draft_cleared"] = not composer_holds(self.detection(), ROOT_DRAFT)
        # The ladder's next retry after the draft window can be a step later (>= 30 s, growing): wait two spacings.
        activity, took = self.watch_queue_activity(proof, QUEUE_WAKE_WINDOW_S + 2 * SPACING_S)
        t_wake = took if activity is not None else None
        proof_now, samples = proof.current_proof()
        checks["queue_after_clear"] = bool(proof_now and proof_now.get("passed") and activity is not None)
        checks["root_busy_at_arm"] = root_busy_before_arm(samples, root) if activity is not None else False
        delivered, delivery = self.root_delivery(root, activity, ref.get("message", ""), tag)
        checks.update(delivered=delivered, delivery_evidence=delivery, activity_evidence=activity)
        self.wait_idle()
        self.settled()
        checks["release_blocking"] = release_blocking(checks)
        passed = root_draft_queue_passed(checks) and not checks["release_blocking"]
        self.run.record({"case": case, "harness": self.harness, "threshold": self.threshold, "recipient": self.name,
                         "expect_wake": True, "woke": activity is not None, "t_wake_s": t_wake, "hello_sent": False,
                         "checks": checks, "ref": ref, "result": "PASS" if passed else "FAIL", "utc": utc()})

    def side_defer_case(self, case):
        """side-native-defer (queue off): with a /side chat displayed a p0 types nothing into it; the root wakes once
        the side chat closes and gets the item."""
        tag = f"{case} {self.run.run_id}"
        sides = self.open_displayed_side(tag, "SIDE-DEFER-DONE")
        checks = {"bindings_after_side": sides}
        setup_ok = len(sides) >= 2
        checks["side_setup_ok"] = setup_ok
        gap = SPACING_S - (time.time() - self.last_wake)
        if gap > 0:
            self.run.stopping.wait(gap)
        self.wait_idle()
        checks["bindings_idle"] = idle = self.bindings_idle()
        setup_ok = setup_ok and idle
        before_screen = self.detection()
        before = self.wake_state()
        checks["wake_state_before_send"] = before
        sent_ms = int(time.time() * 1000)
        ref = {"message": self.send("p0", tag, direct=False)}
        mid = ref["message"]
        # The side chat stays displayed until a refused native attempt is observed (or the window ends).
        end, state = time.time() + WAKE_WINDOW_S + 10, None
        while not self.run.stopping.is_set():
            state = self.wake_state()
            if side_defer_refusal(before, state, sent_ms) or time.time() >= end:
                break
            time.sleep(0.5)
        refused_while_side = side_defer_refusal(before, state, sent_ms)
        setup_ok = setup_ok and refused_while_side
        checks["refused_while_side"] = state if refused_while_side else None
        side_typed, _ = side_pane_verdict(before_screen, self.detection())
        checks["wake_state_while_side"] = state
        # a submitted wake after the send, while the side chat is still displayed, is a wake typed into it
        submitted = bool(state) and state.get("last_outcome") == "submitted" and (state.get("completed_at_utc") or 0) >= sent_ms
        typed_while_side = side_typed or submitted
        checks["side_typed"], checks["submitted_while_side"] = side_typed, submitted
        closed_ms = int(time.time() * 1000)
        # Codex answers Ctrl+C on an idle side chat with an `Archive ...?` dialog (ht-z41e.51): confirm it, or the
        # side chat stays displayed and the root is correctly refused for the whole window.
        checks["close_side"] = self.close_side_checked()
        t_wake = self.watch_for_wake(SPACING_S + self.wake_window())
        state = self.wake_state()
        checks["wake_state_after_close"] = state
        # a retry typed during the close itself, finished before the watch started
        woke_before_watch = (t_wake is None and bool(state) and state.get("last_outcome") == "submitted"
                             and (state.get("completed_at_utc") or 0) >= closed_ms
                             and received_in_wake_turn(self.screen(1000), mid))
        woke = t_wake is not None or woke_before_watch
        checks["woke_before_watch"] = woke_before_watch
        hello = False
        if not woke:
            hello = True
            self.prompt(self.follow_up())
        self.wait_idle()
        delivered = self.on_screen(mid)
        checks["delivered"] = delivered
        self.settled()
        passed = side_defer_passed(setup_ok, refused_while_side, typed_while_side, woke, delivered)
        self.run.record({"case": case, "harness": self.harness, "threshold": self.threshold, "recipient": self.name,
                         "expect_wake": True, "woke": woke, "t_wake_s": t_wake, "hello_sent": hello,
                         "checks": checks, "ref": ref, "result": "PASS" if passed else "FAIL", "utc": utc()})

    def config_prose_case(self, case):
        """config-prose (queue off): told in prose only, the agent finds programmatic_wake with `config seat list` and
        turns it on through the human tier; the store reads it on. The seat is restored afterwards."""
        tag = f"{case} {self.run.run_id}"
        shim = self.run.bindir / "herdr-threads"
        alias = self.run.bindir / "ht"
        if not alias.exists():
            alias.symlink_to(shim.name)  # the shim logs every call whatever the spelling
        gap = SPACING_S - (time.time() - self.last_wake)
        if gap > 0:
            self.run.stopping.wait(gap)
        self.wait_idle()
        seen = len(self.calls())
        self.prompt(CONFIG_PROSE_PROMPT.format(tag=tag), expect=CONFIG_PROSE_DONE)
        done = self.shows(CONFIG_PROSE_DONE)
        calls = self.calls()[seen:]
        rows = self.store_rows("SELECT value FROM seat_config WHERE seat_id=? AND key='programmatic_wake'", (self.seat,))
        value = rows[0][0] if rows else None
        verdict = config_prose_verdict(calls, value)
        try:  # later cases keep native mode
            self.run.ht("config", "seat", "unset", "--seat", self.seat, "programmatic_wake", human=True,
                        tag=f"config:{self.name}:restore")
        except RuntimeError as e:
            verdict = {"passed": False, "reason": "restore_failed: " + clip(e, 200, getattr(self.run, "secrets", None))}
        self.settled()
        checks = {"done_marker": done, "calls": [" ".join(argv) for argv in calls], "config_value": value,
                  "verdict": verdict, "approval_bypass": approval_bypass(self.run.args.codex_args),
                  "ask_bypassed": True}
        passed = done and verdict["passed"]
        if not done:
            checks["verdict"] = {"passed": False, "reason": "no_done_marker"}
        self.run.record({"case": case, "harness": self.harness, "threshold": self.threshold, "recipient": self.name,
                         "expect_wake": False, "woke": False, "t_wake_s": None, "hello_sent": False,
                         "checks": checks, "ref": {}, "result": "PASS" if passed else "FAIL", "utc": utc()})

    def flag_flip_case(self, case):
        """flag-flip (--queue-proof): unset programmatic_wake the moment a new queue intent is first seen `prepared`;
        the wake must then settle as a native one (`stale_authority_native`) and deliver the item. Up to
        FLAG_FLIP_TRIALS trials; a trial that missed the window is retried with the flag back on."""
        from wake_e2e_queue import queue_proof, queue_snapshot, stale_authority_native
        tag = f"{case} {self.run.run_id}"
        flag = ("--seat", self.seat, "programmatic_wake")
        trials, refs = [], []
        try:
            for n in range(1, FLAG_FLIP_TRIALS + 1):
                gap = SPACING_S - (time.time() - self.last_wake)
                if gap > 0:
                    self.run.stopping.wait(gap)
                self.wait_idle()
                before = queue_snapshot(self.run.state, self.seat)
                if not before.get("ok"):
                    raise RuntimeError(f"{self.name}: flag-flip: canonical snapshot unavailable")
                mid = self.send("p0", f"{tag} trial {n}", direct=False)
                refs.append(mid)
                samples, encoded, first, flipped = [], None, None, False
                end = time.monotonic() + QUEUE_WAKE_WINDOW_S + 10
                while time.monotonic() < end and not self.run.stopping.is_set():
                    snap = queue_snapshot(self.run.state, self.seat, before["watermark"])
                    text = json.dumps(snap, sort_keys=True)
                    if text != encoded and len(samples) < QUEUE_SAMPLE_LIMIT:
                        samples.append(dict(snap, observed_utc_ms=int(time.time() * 1000)))
                    encoded = text
                    new = [i for i in snap.get("intents", ()) if i["ordinal"] > before["watermark"]]
                    if new and first is None:
                        first = new[0]["phase"]
                        if flag_flip_action(first) == "flip":
                            self.run.ht("config", "seat", "unset", *flag, human=True, tag=f"config:{self.name}:flip")
                            flipped = True
                            end = min(end, time.monotonic() + WAKE_WINDOW_S + 15)
                        else:
                            break
                    if flipped and self.shows(mid):
                        break
                    time.sleep(0.05)
                delivered = self.on_screen(mid)
                if not delivered:
                    self.prompt(self.follow_up())  # leave the recipient clean; the trial still fails
                self.wait_idle()
                self.settled()
                if flipped:
                    trials.append({"trial": n, "message": mid, "first_phase": first, "flipped": True,
                                   "delivered": delivered, "stale": stale_authority_native(before, samples),
                                   "never_armed": queue_proof(before, samples, "none")["passed"],
                                   "samples": samples})
                else:
                    trials.append({"trial": n, "message": mid, "window_missed": first or "no_intent",
                                   "delivered": delivered, "samples": samples})
                self.run.ht("config", "seat", "set", *flag, "on", human=True, tag=f"config:{self.name}:restore")
                if flag_flip_verdict(trials)["passed"]:
                    break
        finally:
            self.run.ht("config", "seat", "set", *flag, "on", human=True, tag=f"config:{self.name}:restore", check=False)
        verdict = flag_flip_verdict(trials)
        self.run.record({"case": case, "harness": self.harness, "threshold": self.threshold, "recipient": self.name,
                         "expect_wake": True, "woke": verdict["passed"], "t_wake_s": None, "hello_sent": False,
                         "checks": {"trials": trials, "verdict": verdict}, "ref": {"messages": refs},
                         "result": "PASS" if verdict["passed"] else "FAIL", "utc": utc()})

    def side_case(self, case):
        action = case.split("/")[0]
        tag = f"{case} {self.run.run_id}"
        proof, root = getattr(self, "queue_evidence", None), self.queue_root_session()
        queued = proof is not None and bool(root)
        if queued:
            proof.root_session = root
        sides = self.open_sides(tag)
        connected = [b for b in sides if b["liveness"] == "connected"]
        checks = {"bindings_after_side": sides, "connected_same_pid":
                  len(connected) > 1 and len({b["pid"] for b in connected}) == 1}
        # The setup must have happened (a later case adds its own sides): the hookless recipient keeps every
        # side binding connected on its one pid; a hooked one has them (superseded by its lifecycle-claimed session).
        setup_ok = side_setup_ok(sides, checks["connected_same_pid"], self.plain)
        checks["side_setup_ok"] = setup_ok
        checks["side_setup"] = getattr(self, "side_setup", None)
        gap = SPACING_S - (time.time() - self.last_wake)
        if gap > 0:
            self.run.stopping.wait(gap)
        self.wait_idle()
        ref = {}
        if action == "side-nudge":
            expect_wake = True
            ref["message"] = self.send("p0", tag, direct=False)
            t_wake = self.watch_for_wake(self.wake_window())
            checks["late_wake_s"] = None if t_wake is not None else self.watch_for_wake(20)
            woke = t_wake is not None
            checks["wake_state"] = self.wake_state()
            hello = False
            if not woke and checks["late_wake_s"] is None:
                hello = True
                self.prompt(self.follow_up())
            self.wait_idle()
            checks["delivered"] = self.on_screen(ref["message"])
            self.settled()
            gap = SPACING_S - (time.time() - self.last_wake)
            if gap > 0:
                self.run.stopping.wait(gap)
            self.sibling_phase(checks, ref, tag)
            passed = side_nudge_passed(setup_ok, woke, checks)
        else:
            # Busy-input guard: a typed draft is never typed into or submitted; the sender sees the refused wake.
            expect_wake = False
            self.run.herdr("pane", "send-text", self.pane, SIDE_DRAFT)
            try:
                woke, t_wake, mid = self.draft_phase(checks, ref, tag)
            finally:  # never leave the draft behind for a later case
                self.run.herdr("agent", "send-keys", self.pane, "ctrl+u", check=False)
                time.sleep(1)
            hello = True
            self.prompt(self.follow_up())
            checks["delivered"] = self.on_screen(mid)
            passed = side_draft_passed(setup_ok, woke, checks)
        self.settled()
        self.run.record({"case": case, "harness": self.harness, "threshold": self.threshold, "recipient": self.name,
                         "expect_wake": expect_wake, "woke": woke, "t_wake_s": t_wake, "hello_sent": hello,
                         "checks": checks, "ref": ref, "result": "PASS" if passed else "FAIL", "utc": utc()})

    def draft_phase(self, checks, ref, tag):
        """side-draft after the draft is typed: send, watch past the deadline, read composer and warning."""
        time.sleep(1)
        # An ACK request (p0, so it wakes): since D9 only an explicit --require-ack recipient gets a deadline and
        # an overdue warning, and `--nudge --deadline` is refused.
        argv = ["send", self.group, "--body", tag, "--require-ack", self.seat, "--deadline", str(SIDE_DEADLINE_S)]
        sent_seq = max([row.get("sequence") or 0 for row in walk(self.coord("read", self.group, "--recent", "1"))]
                       or [0])
        mid = (self.coord(*argv).get("result") or {}).get("data")
        ref["message"] = mid
        t_wake = self.watch_for_wake(SIDE_DEADLINE_S + 20)
        woke = t_wake is not None
        screen = self.run.herdr("pane", "read", self.pane, "--source", "detection", check=False).get("raw", "")
        composer = [l for l in screen.splitlines() if l.startswith("\u203a ")]
        checks["composer_at_end"] = composer[-1] if composer else None
        checks["draft_intact"] = bool(composer) and composer[-1] == "\u203a " + SIDE_DRAFT
        checks["wake_state"] = self.wake_state()
        # The sender's view: the overdue warning for this seat after the send, its full event read by `body`.
        warn = None
        for _ in range(30):
            for row in walk(self.coord("read", self.group, "--recent", "20")):
                # The group thread is this recipient's own; the seat is confirmed from the full event.
                if row.get("kind") == "warn" and (row.get("sequence") or 0) > sent_seq:
                    warn = row
            if warn:
                break
            time.sleep(2)
        event = None
        if warn:
            for row in walk(self.coord("body", warn["message"])):
                if "obligation" in row and "seat" in row:
                    event = row
                for key in ("body_data", "event_json", "preview_data"):
                    value = row.get(key)
                    if isinstance(value, dict) and "obligation" in value:
                        event = value
                    elif isinstance(value, str) and '"obligation"' in value:
                        with contextlib.suppress(ValueError):
                            event = json.loads(value)
        if event and event.get("seat") != self.seat:
            event = None
        checks["overdue_warning"] = {"message": warn and warn["message"], "event": event}
        checks["warning_wake"] = (event or {}).get("wake")
        checks["warning_detail"] = (event or {}).get("detail")
        return woke, t_wake, mid

    def doctor_wake(self):
        """This seat's wake block as `doctor` reports it, or None."""
        report = json.loads(self.run.ht("doctor", check=False) or "{}")
        for row in (report.get("doctor") or {}).get("wake", {}).get("seats") or []:
            if row.get("seat") == self.seat:
                return row.get("wake")
        return None

    def wake_block_case(self, case):
        """Hold p0 behind an open model picker or a single-session draft, then release it."""
        draft = case == "draft-hold/group"
        tag = f"{case} {self.run.run_id}"
        checks = {}
        gap = SPACING_S - (time.time() - self.last_wake)
        if gap > 0:
            self.run.stopping.wait(gap)
        self.wait_idle()
        def detection():
            return self.run.herdr("pane", "read", self.pane, "--source", "detection", check=False).get("raw", "")
        def footer():
            return [l.strip() for l in detection().splitlines() if " \u00b7 ~/" in l][-1:]
        checks["footer_before"] = footer()
        wakes_before = wake_count(self.screen(1000))
        if draft:
            self.run.herdr("pane", "send-text", self.pane, SIDE_DRAFT)
            time.sleep(1)
            opened = any(l == "› " + SIDE_DRAFT for l in detection().splitlines())
            checks["draft_open"] = opened
        else:
            self.run.herdr("pane", "send-text", self.pane, "/model")
            time.sleep(1)
            self.run.herdr("agent", "send-keys", self.pane, "enter", check=False)
            opened = False
            for _ in range(20):
                if any(PICKER_ROW.match(l) for l in detection().splitlines()):
                    opened = True
                    break
                time.sleep(0.5)
            checks["picker_open"] = opened
        pane = self.run.coords["main"]["pane"]
        sends, mids = [], []
        try:
            if opened:
                proof = getattr(self, "queue_evidence", None)
                if proof is not None:
                    proof.publication()
                for n in range(2):
                    out, err = self.run.ht_err("send", self.group, "--body", f"{tag} #{n}", "--nudge", pane=pane,
                                               human=True)
                    mids.append((json.loads(out).get("result") or {}).get("data"))
                    sends.append([l for l in err.splitlines() if "wake=" in l])
                self.run.stopping.wait(WAKE_BLOCK_HOLD_S)
                checks["wakes_while_open"] = wake_count(self.screen(1000)) - wakes_before
                checks["doctor_wake_open"] = self.doctor_wake()
                if draft:
                    checks["draft_intact"] = any(l == "› " + SIDE_DRAFT for l in detection().splitlines())
                if proof is not None:
                    proof.hold_ended()
        finally:  # only clear the UI this case opened
            release_start = time.monotonic()
            self.run.herdr("agent", "send-keys", self.pane, "ctrl+u" if draft else "esc", check=False)
            proof = getattr(self, "queue_evidence", None)
            if proof is not None:
                proof.release_interval_ms = [proof.offset(release_start), proof.offset(time.monotonic())]
                self.wake_window()
        checks["send_warnings"] = sends
        t0, t_wake, activity = time.time(), None, None
        while opened and time.time() - t0 < WAKE_BLOCK_WAIT_S:
            if self.run.stopping.is_set():
                raise Stopped("cleanup interrupted UI observation")
            if proof is not None:
                activity = proof.qualified_activity()
                if activity is not None:
                    elapsed = activity["observed_at_ms"] - proof.release_interval_ms[0]
                    if 0 <= elapsed <= WAKE_BLOCK_WAIT_S * 1000:
                        t_wake = round(elapsed / 1000, 3)
                        break
            elif wake_count(self.screen(1000)) > wakes_before:
                t_wake = round(time.time() - t0, 2)
                break
            time.sleep(0.5)
        woke = t_wake is not None
        if activity is not None and woke:
            from wake_e2e_queue import codex_turn_delivery
            checks["activity_evidence"] = activity
            expected = {mid: f"{tag} #{n}" for n, mid in enumerate(mids) if mid}
            end = time.monotonic() + TURN_TIMEOUT_MS / 1000
            delivery = {"passed": False}
            while time.monotonic() < end and len(expected) == 2 and not self.run.stopping.is_set():
                delivery = codex_turn_delivery(self.run.codex_home, activity["native_session"],
                                               activity["turn_id"], expected)
                if delivery.get("complete") or delivery.get("error"):
                    break
                self.run.stopping.wait(0.5)
            if self.run.stopping.is_set():
                raise Stopped("cleanup interrupted UI observation")
            checks["delivery_evidence"] = delivery
            checks["delivered"] = delivery["passed"]
        else:
            checks["delivered"] = bool(mids) and all(self.on_screen(m) for m in mids if m)
        if woke:
            self.last_wake = time.time()
            self.wait_idle()
        checks["footer_after"] = footer()
        passed = (opened and checks["wakes_while_open"] == 0 and sends == [[], []] and woke
                  and checks["delivered"] and checks["footer_after"] == checks["footer_before"]
                  and (not draft or checks.get("draft_intact") is True))
        self.settled()
        self.run.record({"case": case, "harness": self.harness, "threshold": self.threshold, "recipient": self.name,
                         "expect_wake": True, "woke": woke, "t_wake_s": t_wake, "checks": checks,
                         "ref": {"messages": mids}, "result": "PASS" if passed else "FAIL", "utc": utc()})

    def seat_inspect(self):
        out = self.run.ht("seat", "inspect", self.seat, pane=self.run.coords["main"]["pane"], human=True)
        return (json.loads(out).get("result") or {}).get("data") or {}

    def availability(self):
        return (self.seat_inspect().get("summary") or {}).get("availability", "")

    def resume_case(self, case):
        """resume-idle / resume-quit: a Codex killed (or quit with `/quit`) and resumed, idle and unknown to the
        daemon, wakes on p0 and only on p0."""
        tag = f"{case} {self.run.run_id}"
        checks = {}
        gap = SPACING_S - (time.time() - self.last_wake)
        if gap > 0:
            self.run.stopping.wait(gap)
        self.wait_idle()
        session = resume_root_session(self.bindings())
        info = self.run.herdr("pane", "process-info", "--pane", self.pane)["result"]["process_info"]
        pids = [p["pid"] for p in info["foreground_processes"] if any("codex" in a for a in (p.get("argv") or [])[:2])]
        if not pids:
            raise RuntimeError(f"{self.name}: no foreground codex process: {info}")
        quit_ = case.startswith("resume-quit")
        if quit_:  # SessionEnd runs: an end event, then the process exits
            self.run.herdr("pane", "send-text", self.pane, "/quit")
            time.sleep(1)
            self.run.herdr("agent", "send-keys", self.pane, "enter", check=False)
            end = time.time() + QUIT_EXIT_S
            while time.time() < end and any(_pid_alive(pid) for pid in pids):
                time.sleep(0.5)
            checks["quit_exited"] = not any(_pid_alive(pid) for pid in pids)
            for pid in pids:  # never leave the pane to later cases with Codex still in it
                with contextlib.suppress(ProcessLookupError):
                    os.kill(pid, signal.SIGKILL)
        else:
            for pid in pids:  # no SessionEnd hook: a process exit, not an end event
                with contextlib.suppress(ProcessLookupError):
                    os.kill(pid, signal.SIGKILL)
        end = time.time() + 30
        while time.time() < end and not self.availability().startswith("unavailable"):
            time.sleep(0.5)
        checks["unavailable_after_exit"] = self.availability().startswith("unavailable")
        reasons = self.store_rows("SELECT disconnect_reason FROM occupant_bindings WHERE seat_id=? AND native_session=? "
                                  "AND role='top_level' AND ended_at IS NULL", (self.seat, session))
        checks["disconnect_reason"] = reasons[0][0] if reasons else None
        self.run.resumes = getattr(self.run, "resumes", 0) + 1
        line = self.run.launch_line(self.harness) + " resume " + shlex.quote(session)
        self.run.herdr("pane", "run", self.pane, line)
        end = time.time() + 120
        while time.time() < end:
            agent = (self.run.herdr("agent", "get", self.pane, check=False).get("result") or {}).get("agent") or {}
            if agent.get("agent") == self.harness and agent.get("agent_status") in IDLE:
                break
            time.sleep(0.5)
        self.wait_idle()
        checks["unavailable_while_idle"] = self.availability().startswith("unavailable")
        wakes_before = wake_count(self.screen(1000))
        pane = self.run.coords["main"]["pane"]
        def send(*flags):
            proof = getattr(self, "queue_evidence", None)
            if proof is not None:
                proof.publication()
            out, _ = self.run.ht_err("send", self.group, "--body", f"{tag} {flags}", *flags, pane=pane, human=True)
            return (json.loads(out).get("result") or {}).get("data")
        silent = send()
        self.run.stopping.wait(RESUME_QUIET_S)
        checks["wakes_on_p1"] = wake_count(self.screen(1000)) - wakes_before
        urgent = send("--nudge")
        t0, t_wake = time.time(), None
        while time.time() - t0 < WAKE_BLOCK_WAIT_S:
            if wake_count(self.screen(1000)) > wakes_before:
                t_wake = round(time.time() - t0, 2)
                break
            time.sleep(0.5)
        woke = t_wake is not None
        if woke:
            self.last_wake = time.time()
            self.wait_idle()
        checks["delivered"] = all(self.on_screen(m) for m in (silent, urgent) if m)
        checks["available_after"] = self.availability() == "available"
        expected_reason = "liveness_end_event" if quit_ else "liveness_process_exit"
        passed = (checks["unavailable_after_exit"] and checks["unavailable_while_idle"]
                  and checks.get("quit_exited", True) and checks["disconnect_reason"] == expected_reason
                  and checks["wakes_on_p1"] == 0 and woke
                  and checks["delivered"] and checks["available_after"])
        self.settled()
        self.run.record({"case": case, "harness": self.harness, "threshold": self.threshold, "recipient": self.name,
                         "expect_wake": True, "woke": woke, "t_wake_s": t_wake, "checks": checks,
                         "ref": {"messages": [silent, urgent]}, "result": "PASS" if passed else "FAIL", "utc": utc()})

    # ---------- prompt-race cases (mod Claude) ----------
    def claude_cfg_dir(self):
        return self.run.claude_cfg if self.run.linux else Path(self.run.args.claude_config_dir or Path.home() / ".claude")

    def wake_enqueues(self):
        return claude_wake_enqueues(self.claude_cfg_dir(), {b["session"] for b in self.bindings() if b["session"]})

    def ack_via(self, mid, timeout=20):
        """How the recipient's ACK of `mid` was observed (`context`, `append`, ...), or None."""
        end = time.time() + timeout
        while True:
            obs = admission_observation([json.loads(text) for table in ("receipt_state", "receipts")
                                         for (text,) in self.store_rows(
                f"SELECT ack_observation FROM {table} WHERE message_id=? AND seat_id=? AND ack_observation IS NOT NULL",
                (mid, self.seat))])
            if obs is not None or time.time() >= end:
                return obs.get("via") if obs else None
            time.sleep(1)

    def prompt_race_case(self, case):
        """PROMPT_RACE_CASES: no wake is submitted while the engine holds another prompt (Enter to turn.start, or a
        prompt queued behind the turn), and the item reaches the model in that prompt's turn."""
        tag = f"{case} {self.run.run_id}"
        enter = case.startswith("enter-race")
        gap = SPACING_S - (time.time() - self.last_wake)
        if gap > 0:
            self.run.stopping.wait(gap)
        self.wait_idle()
        before, enqueued_before, checks = self.screen(1000), self.wake_enqueues(), {}
        if enter:
            anchor = f"Enter-race test {tag}"
            self.run.herdr("pane", "send-text", self.pane, ENTER_RACE_PROMPT.format(tag=tag))  # a draft, no Enter
            time.sleep(1)
            mid = self.send("ack", tag, direct=False)
            self.run.stopping.wait(PROMPT_RACE_DRAFT_S)
            checks["held_by_draft"] = (self.status() in IDLE and wake_delta(before, self.screen(1000)) == 0
                                       and self.wake_enqueues() == enqueued_before)
            self.run.herdr("agent", "send-keys", self.pane, "enter", check=False)
        else:
            anchor = f"Queued follow-up {tag}"
            with BusyBarrier(self.run.root) as hold:
                entry, _ = self.enter_busy(hold, QUEUED_PROMPT.format(tag=tag, hold=hold.command()),
                                           f"Queued-prompt test {tag}")
                checks.update(entry)
            # The hold is released: the turn is past its last tool call and writing the essay.
            time.sleep(1.5)
            mid = self.send("ack", tag, direct=False)
            checks["busy_at_send"] = self.status()
            self.run.herdr("pane", "send-text", self.pane, QUEUED_FOLLOWUP.format(tag=tag))
            time.sleep(0.5)
            self.run.herdr("agent", "send-keys", self.pane, "enter", check=False)
        end, idle_since = time.time() + TURN_TIMEOUT_MS / 1000, None
        while time.time() < end and not (busy_turn_split(self.screen(1000), anchor, done=RACE_DONE_LINE,
                                                         prompt_end_text=RACE_PROMPT_END) or ("", "", False))[2]:
            idle_since = (idle_since or time.time()) if self.status() in IDLE else None
            if idle_since and time.time() - idle_since > BUSY_END_GRACE_S + PROMPT_RACE_HOOK_S:
                break
            time.sleep(0.5)
        while time.time() < end and self.status() not in IDLE:
            time.sleep(0.25)
        checks["self_started_turn_after_s"] = self.watch_quiet(BUSY_AFTER_S)
        self.wait_idle()
        screen = self.screen(1000)
        split = busy_turn_split(screen, anchor, done=RACE_DONE_LINE, prompt_end_text=RACE_PROMPT_END)
        checks["turn_end_seen"] = split[2] if split else "prompt not on screen"
        checks["wake_prompts_on_screen"] = wake_delta(before, screen)
        checks["wake_enqueues"] = self.wake_enqueues() - enqueued_before
        checks["delivered"] = self.on_screen(mid)
        checks["ack"] = self.receipt(mid)
        checks["ack_via"] = self.ack_via(mid)
        checks["window_staged"] = checks["ack_via"] == "append"
        self.settled()
        passed = (checks["wake_enqueues"] == 0 and checks["wake_prompts_on_screen"] == 0
                  and checks["self_started_turn_after_s"] is None and checks["turn_end_seen"] is True
                  and checks["delivered"] and checks["ack"] == "acknowledged" and checks["window_staged"]
                  and checks.get("held_by_draft", True) and checks.get("busy_at_send", "working") not in IDLE)
        self.run.record({"case": case, "harness": self.harness, "threshold": self.threshold, "recipient": self.name,
                         "expect_wake": False,
                         "woke": checks["wake_enqueues"] != 0 or checks["wake_prompts_on_screen"] != 0
                         or checks["self_started_turn_after_s"] is not None,
                         "t_wake_s": None, "hello_sent": False, "checks": checks, "ref": {"message": mid},
                         "result": "PASS" if passed else "FAIL", "utc": utc()})

    # ---------- one case ----------
    def case(self, case):
        if case in PROMPT_RACE_CASES:
            return self.prompt_race_case(case)
        if case in WAKE_BLOCK_CASES:
            return self.wake_block_case(case)
        if case in RESUME_CASES:
            return self.resume_case(case)
        if case == FOCUSED_QUEUE_CASE:
            return self.side_queue_focused_case(case)
        if case in ROOT_DRAFT_CASES:
            return self.root_draft_queue_case(case)
        if case in SIDE_QUEUE_CASES:
            return self.side_queue_case(case)
        if case in SIDE_DEFER_CASES:
            return self.side_defer_case(case)
        if case in CONFIG_PROSE_CASES:
            return self.config_prose_case(case)
        if case in FLAG_FLIP_CASES:
            return self.flag_flip_case(case)
        if case.startswith("side-"):
            return self.side_case(case)
        if case.startswith("hooks-busy-"):
            # busy-tool-p1 on the hooks-only Claude, recorded under its own name.
            return self.busy_case("busy-tool-p1/" + case.split("/")[1], record=case)
        if case.startswith("hooks-"):
            return self.hooks_case(case)
        if case.startswith("eot-"):
            return self.eot_case(case)
        if case in UNRECEIVED_CASES:
            return self.unreceived_case(case)
        if case.startswith("busy-"):
            return self.busy_case(case)
        action, channel = case.split("/")
        if action == "mixed-order":
            return self.mixed_order_case(case)
        expect = True if action == "native-fallback" else expect_wake(action, channel, self.threshold)
        gap = SPACING_S - (time.time() - self.last_wake)
        if gap > 0:
            self.run.stopping.wait(gap)  # spacing ends early when cleanup begins
        self.wait_idle()
        body = f"{case} {self.run.run_id}"
        checks = {}
        calls_before = self.inbox_calls()
        ledger_before = len(self.ledger()) if self.harness == "opencode" else 0
        if action == "invite" and channel == "group":
            name = f"wake-e2e-{self.name}-{self.run.run_id}"
            data = self.coord("thread", "create", "--name", name, "--topic", f"wake-e2e test thread for {self.name}",
                              "--goal", "wake e2e test traffic")
            self.group = data["result"]["data"]
            self.coord("invite", self.group, "--seat", self.seat)
            ref = {"thread": self.group}
        elif action == "pair-open":
            # First contact at the default p1: the pair auto-joins both seats, there is no invitation to accept.
            ref = {"message": self.send("p1", f"pair-open {self.run.run_id}", direct=True)}
            self.pair = "pending"
        else:
            ref = {"message": self.send("p0" if action == "native-fallback" else action.split("-")[1], body, direct=channel == "1:1")}
        t_wake = self.watch_for_wake(self.wake_window(expect))
        woke = t_wake is not None
        hello = False
        if action == "pair-open":
            # Joined with no accept and before any hello prompt: a silent opener leaves the recipient idle until then.
            thread = self.pair = self.pair_thread()
            ok = False
            for _ in range(10):
                ok = bool(thread) and self.joined(thread)[0]
                if ok:
                    break
                time.sleep(1)
                thread = self.pair = self.pair_thread() or thread
            checks["joined"] = ok
        if woke:
            self.last_wake = time.time()
        else:
            checks["late_wake_s"] = self.watch_for_wake(20) if expect else None
            if checks["late_wake_s"] is None:
                hello = True
                self.prompt("hello")
                self.last_wake = time.time()
        self.wait_idle()
        self.settled()
        if action == "invite":
            ok = False
            for _ in range(20):
                ok, state = self.joined(self.group)
                if ok:
                    break
                time.sleep(1)
            checks["joined"] = ok
        elif action == "pair-open":
            checks["opening_message_delivered"] = self.on_screen(ref["message"])
        else:
            checks["delivered"] = self.on_screen(ref["message"])
            if woke and not self.plain:
                # A delivered wake needs no `inbox` round trip (the skill's bare-wake rule; Delivery contract rule 3).
                checks["inbox_calls"] = self.inbox_calls() - calls_before
                checks["no_extra_inbox"] = checks["inbox_calls"] == 0
            if action == "send-ack":
                status = None
                for _ in range(20):
                    status = self.receipt(ref["message"])
                    if status == "acknowledged":
                        break
                    time.sleep(1)
                checks["ack"] = status
                if self.harness == "claude" and status == "acknowledged":
                    checks["admission_row"] = self.admission_row(ref["message"])
                if self.harness == "opencode" and status == "acknowledged":
                    checks["admission_row"] = self.opencode_admission_row(ref["message"])
        # The case owns the rest of the spacing window: a turn the recipient starts on its own in here (a follow-up
        # wake for an item this case left pending) fails this case, and spacing restarts after it, so the next case
        # is never measured against the 30 s guard of a wake it did not cause.
        checks["trailing_wake_s"] = self.watch_quiet(max(0.0, SPACING_S - (time.time() - self.last_wake)))
        if checks["trailing_wake_s"] is not None:
            self.wait_idle()
            self.settled()
        if self.harness == "opencode":
            # pair-open's opener wakes nobody at T >= 1 by priority; the expectation is the case's own.
            self.channel_checks(checks, ledger_before, expect, [ref.get("message")])
        verified = all(v is True or v == "acknowledged" for k, v in checks.items()
                       if k not in ("late_wake_s", "trailing_wake_s", "inbox_calls", "plugin_wakes"))
        passed = woke == expect and verified and checks["trailing_wake_s"] is None
        row = {"case": case, "harness": self.harness, "threshold": self.threshold, "recipient": self.name,
               "expect_wake": expect, "woke": woke, "t_wake_s": t_wake, "hello_sent": hello,
               "checks": checks, "ref": ref, "result": "PASS" if passed else "FAIL", "utc": utc()}
        self.run.record(row)

    def suggestion_shown(self):
        """True when the composer shows dim text after the prompt glyph (a prompt suggestion or placeholder)."""
        text = self.run.herdr("agent", "read", self.pane, "--source", "recent", "--format", "ansi", "--lines", "60",
                              check=False).get("raw", "")
        return any(SUGGESTION_ROW.match(line) for line in text.splitlines())

    def hooks_case(self, case):
        """HOOKS_CASES: an idle hooks-only Claude at T=0 gets an ACK request (p0). It must wake within the window on
        the content-free native prompt, and the woken turn must show the message with its ACK settled by the
        UserPromptSubmit hook's display (`via: hook_context`): the turn made no `inbox` round trip."""
        suggestion = None
        if case.startswith("hooks-suggestion"):
            # Claude suggests nothing before its second finished turn.
            for _ in range(2):
                self.prompt("Reply with just: ok")
            end = time.time() + 60
            while not self.suggestion_shown() and time.time() < end:
                time.sleep(1)
            suggestion = self.suggestion_shown()
        gap = SPACING_S - (time.time() - self.last_wake)
        if gap > 0:
            self.run.stopping.wait(gap)
        self.wait_idle()
        mid = self.send("ack", f"{case} {self.run.run_id}", direct=False)
        t_wake = self.watch_for_wake(WAKE_WINDOW_S)
        woke = t_wake is not None
        if woke:
            self.last_wake = time.time()
        self.wait_idle()
        self.settled()
        checks = {"delivered": self.on_screen(mid)}
        if suggestion is not None:
            checks["suggestion_shown"] = suggestion
        turn = wake_turn(self.screen(1000), mid)
        checks["content_free_wake"] = bool(turn) and turn[0] == "herdr-threads: wake up"
        checks["received_in_wake_turn"] = bool(turn)
        # The UserPromptSubmit hook must have delivered it: a Bash call (the skill's bare-wake `inbox`, whose
        # PreToolUse page would also settle the ACK via hook_context) before RECEIVED fails the case.
        checks["tool_call_before_received"] = any(TOOL_CALL.search(line) for line in turn[1:])
        row = {}
        for _ in range(20):
            row = self.receipt_row(mid)
            if row.get("effective_status") == "acknowledged":
                break
            time.sleep(1)
        checks["ack"] = row.get("effective_status")
        try:
            via = json.loads((row.get("ack_provenance") or {}).get("native_observation") or "{}").get("via")
        except ValueError:
            via = None
        checks["ack_via"] = via
        checks["trailing_wake_s"] = self.watch_quiet(max(0.0, SPACING_S - (time.time() - self.last_wake)))
        if checks["trailing_wake_s"] is not None:
            self.wait_idle()
            self.settled()
        passed = (woke and suggestion is not False and checks["delivered"] and checks["content_free_wake"]
                  and not checks["tool_call_before_received"] and checks["received_in_wake_turn"] and checks["ack"] == "acknowledged"
                  and via == "hook_context" and checks["trailing_wake_s"] is None)
        self.run.record({"case": case, "harness": self.harness, "threshold": self.threshold, "recipient": self.name,
                         "expect_wake": True, "woke": woke, "t_wake_s": t_wake, "checks": checks,
                         "ref": {"message": mid}, "result": "PASS" if passed else "FAIL", "utc": utc()})

    def mixed_order_case(self, case):
        """p5, p1, p0 and an ACK request with a deadline go to an idle recipient; each page it is woken for must list
        its items in presentation order (expected_order: the ACK request first, then p0, p1, p5). The last two are
        sent at once, but the p0's wake can still pull its page before the ACK request is published, which then
        wakes a second page (paged_order); a channel harness gets no page (mixed_order_judged)."""
        gap = SPACING_S - (time.time() - self.last_wake)
        if gap > 0:
            self.run.stopping.wait(gap)
        self.wait_idle()
        rid = self.run.run_id
        ids = {}
        ledger_before = len(self.ledger()) if self.harness == "opencode" else 0
        ids["p5"] = self.send("p5", f"mixed-p5 {rid}", direct=False)
        ids["p1"] = self.send("p1", f"mixed-p1 {rid}", direct=False)
        errors = []

        def last(kind, body, **kw):
            try:
                ids[kind] = self.send(kind, body, direct=False, **kw)
            except Exception as e:  # reported below: a thread's error must fail the case, not vanish
                errors.append(clip(e, 200, getattr(self.run, "secrets", None)))
        threads = [threading.Thread(target=last, args=("p0", f"mixed-p0 {rid}")),
                   threading.Thread(target=last, args=("ack", f"mixed-ack {rid}"),
                                    kwargs={"deadline_s": MIXED_DEADLINE_S})]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        if errors:
            raise RuntimeError("mixed-order send failed: " + "; ".join(errors))
        items = [{"id": ids["p5"], "priority": 5, "deadline_s": None}, {"id": ids["p1"], "priority": 1, "deadline_s": None},
                 {"id": ids["p0"], "priority": 0, "deadline_s": None},
                 {"id": ids["ack"], "priority": 0, "deadline_s": MIXED_DEADLINE_S}]
        # The p0 and the ACK request wake the recipient at every threshold (expect_wake).
        t_wake = self.watch_for_wake(self.wake_window())
        woke = t_wake is not None
        hello = False
        checks = {}
        if woke:
            self.last_wake = time.time()
        else:
            checks["late_wake_s"] = self.watch_for_wake(20)
            if checks["late_wake_s"] is None:
                hello = True
                self.prompt("hello")
                self.last_wake = time.time()
        self.wait_idle()
        self.settled()
        every = [i["id"] for i in items]
        checks["delivered"] = all(self.on_screen(mid) for mid in every)
        screen = self.screen(1000)
        want = expected_order(items)
        shown = screen_order(screen, every)
        pages = paged_order(screen, every)
        judged = mixed_order_judged(self.threshold, self.harness)
        checks["order"] = paged_order_ok(pages, want) if judged else "unjudged"
        checks["order_judged"] = judged
        checks["order_shown"] = shown
        checks["order_pages"] = pages
        checks["order_expected"] = want
        shown_headers, want_markers = header_priorities(screen), {i["id"]: i["priority"] for i in items}
        checks["markers"] = markers_ok(shown_headers, want_markers)
        if not checks["markers"]:
            checks["markers_reason"] = markers_reason(case, shown_headers, want_markers)
        status = None
        for _ in range(20):
            status = self.receipt(ids["ack"])
            if status == "acknowledged":
                break
            time.sleep(1)
        checks["ack"] = status
        checks["trailing_wake_s"] = self.watch_quiet(max(0.0, SPACING_S - (time.time() - self.last_wake)))
        if checks["trailing_wake_s"] is not None:
            self.wait_idle()
            self.settled()
        expect = True
        if self.harness == "opencode":
            self.channel_checks(checks, ledger_before, True, every)
        verified = all(v is True or v == "acknowledged" or (k == "order" and v == "unjudged")
                       for k, v in checks.items()
                       if k in ("delivered", "order", "markers", "ack", "appended", "plugin_wake_ok", "composer_empty"))
        passed = woke == expect and verified and checks["trailing_wake_s"] is None
        self.run.record({"case": case, "harness": self.harness, "threshold": self.threshold, "recipient": self.name,
                         "expect_wake": expect, "woke": woke, "t_wake_s": t_wake, "hello_sent": hello,
                         "checks": checks, "ref": ids, "result": "PASS" if passed else "FAIL", "utc": utc()})

    def ensure_channel(self, channel):
        """Unmeasured setup when the matching invite case did not run first: the recipient joins the ordinary
        thread / accepts the 1:1 pair, so the send cases test delivery and not first contact."""
        if channel == "group" and not self.group:
            name = f"wake-e2e-{self.name}-{self.run.run_id}"
            self.group = self.coord("thread", "create", "--name", name, "--topic", f"wake-e2e test thread for {self.name}",
                                    "--goal", "wake e2e test traffic")["result"]["data"]
            self.coord("invite", self.group, "--seat", self.seat)
            thread = self.group
        elif channel == "1:1" and not self.pair:
            self.send("p0", f"pair-setup {self.run.run_id}", direct=True)
            thread = self.pair = self.pair_thread()
        else:
            return
        end = time.time() + 120
        while time.time() < end and not self.joined(thread)[0]:
            if self.status() in IDLE and time.time() > end - 90:
                self.prompt("Handle any pending herdr-threads items as your standing instructions say.")
            time.sleep(2)
        if not self.joined(thread)[0]:
            raise RuntimeError(f"setup: {self.name} did not join the {channel} channel")
        self.wait_idle()
        self.settled()

    def run_cases(self, cases):
        cases = [c for c in cases if applies(self, c)]
        if not cases:  # nothing to run: never start a harness (wake-unreceived disables the binary meanwhile)
            return
        try:
            self.start()
            for case in cases:
                if self.run.stopping.is_set():
                    break
                try:
                    if case.split("/")[0] not in ("invite", "pair-open"):
                        self.ensure_channel(case.split("/")[1])
                    if (getattr(self.run, "observe_queue", getattr(getattr(self.run, "args", None), "queue_proof", False))
                            and self.harness == "codex"):
                        proof = QueueCaseEvidence(self, case)
                        self.queue_evidence = proof
                        with proof:
                            self.case(case)
                    else:
                        self.case(case)
                except Stopped:
                    break
                except Exception as e:  # a driver or agent error fails the case, never passes it
                    self.run.record({"case": case, "harness": self.harness, "threshold": self.threshold,
                                     "recipient": self.name, "result": "FAIL", "checks": {"error": clip(e, 400, getattr(self.run, "secrets", None))}})
                finally:
                    self.queue_evidence = None
        except Stopped:
            pass
        except Exception as e:
            for case in cases:
                self.run.record({"case": case, "harness": self.harness, "threshold": self.threshold,
                                 "recipient": self.name, "result": "FAIL", "checks": {"error": "startup: " + clip(e, 400, getattr(self.run, "secrets", None))}})
        finally:
            try:
                (self.run.ev / f"pane-{self.name}.txt").write_text(
                    capture_evidence(self.screen(1000), 1000, getattr(self.run, "secrets", ())))
                self.save_calls()
                if self.harness == "opencode":
                    secrets = getattr(self.run, "secrets", ())
                    (self.run.ev / f"screen-{self.name}.txt").write_text(
                        capture_evidence(self.visible(), 1000, secrets))
                    if self.ledger_path().exists():
                        (self.run.ev / f"ledger-{self.name}.jsonl").write_text(
                            redact(self.ledger_path().read_text(errors="replace"), secrets))
            except Stopped:
                pass  # interrupted: no pane capture, the summary marks unfinished cases missing


class TmuxRecipient(Recipient):
    """A named seat (`HERDR_THREADS_SEAT_NAME`, no Herdr pane) running a real agent in its own private tmux server
    (TMUX_CASES). Herdr knows nothing of it, so every pane primitive of Recipient is replaced by its tmux form and
    its busy/idle state is read from the screen (TMUX_RUNNING). The shim logs the seat name where a pane id would
    be, so `calls` attributes its herdr-threads calls; `seat` is the seat id, known once the standing turn has
    enrolled the name. Claude runs the hooks-only installation: the delivery mod's channel would otherwise wake it
    instead of the tmux composer."""
    tmux = True

    def __init__(self, run, name, harness, seat_name):
        super().__init__(run, name, harness, 0, seat_name, None, hooks_only=harness == "claude")
        self.seat_name = seat_name
        self.server = None  # the private tmux server's -L name, once started
        self.target = None  # its pane

    # ---------- observation ----------
    def capture(self, *extra):
        if self.server is None:
            return ""
        rc, out = self.run.tmux(self.server, "capture-pane", "-p", *extra, "-t", self.target)
        return out if rc == 0 else ""

    def visible(self):
        return self.capture()

    def screen(self, lines=400):
        return self.capture("-J", "-S", f"-{lines}")

    def status(self):
        text = self.visible()
        # LINUX_DIALOG: its case-sensitive "Allow" does not match the auto-mode banner's "allow ... to run".
        if LINUX_DIALOG.search(text):
            return "blocked"
        return "working" if TMUX_RUNNING.search(text) else "idle"

    def keys(self, *keys):
        self.run.tmux(self.server, "send-keys", "-t", self.target, *keys)

    def dismiss_notice(self):
        if self.harness != "claude" or not HARNESS_NOTICE.search(self.visible()):
            return False
        self.keys("Escape")
        time.sleep(1.0)
        return True

    # ---------- lifecycle ----------
    def start(self):
        self.server = self.run.tmux_start()
        self.target = self.run.tmux(self.server, "display", "-p", "#{pane_id}")[1].strip()
        if not self.target.startswith("%"):
            raise RuntimeError(f"{self.name}: no tmux pane in {self.server}")
        lock = {"codex": "codex_start"}.get(self.harness)
        with (getattr(self.run, lock) if lock else contextlib.nullcontext()):
            # The name rides the pane's environment alone; the launch line adds nothing Herdr-shaped.
            line = (f"export HERDR_THREADS_SEAT_NAME={shlex.quote(self.seat_name)}; "
                    + self.run.launch_line(self.harness, hooks_only=self.hooks_only))
            self.keys("-l", line)
            self.keys("Enter")
            end = time.time() + 180
            while time.time() < end:
                text = self.visible()
                if AUTH_REJECTED.search(text) or LINUX_LOGIN.search(text):
                    raise RuntimeError(f"{self.name}: {self.harness} is not logged in or its auth was rejected: {self.tail()}")
                if LINUX_DIALOG.search(text):
                    raise RuntimeError(f"{self.name}: startup dialog (nothing typed): {self.tail()}")
                if COMPOSER_UP.search(text) and self.status() == "idle":
                    break
                time.sleep(1)
            else:
                raise RuntimeError(f"{self.name}: {self.harness} composer not up in {self.server}: {self.tail()}")
        reported = setup_done(self.threshold)
        for _attempt in range(2):  # one retry: a transient model/capacity error ends the first turn early
            self.prompt(STANDING.format(coord=self.run.coord_seats(), threshold=self.threshold), expect=reported)
            if self.shows(reported):
                break
        else:
            raise RuntimeError(f"{self.name}: agent did not report nudge-threshold {self.threshold}: {self.tail()}")
        # The turn's first herdr-threads call enrolled the name: its seat id is the sender's target.
        end = time.time() + 30
        while self.seat is None and time.time() < end:
            rows = self.store_rows("SELECT id FROM seats WHERE name=?", (self.seat_name,))
            self.seat = rows[0][0] if rows else None
            time.sleep(1)
        if self.seat is None:
            raise RuntimeError(f"{self.name}: no seat named {self.seat_name} after the standing turn")

    def prompt(self, text, expect=None):
        self.keys("-l", text)
        time.sleep(0.5)
        self.keys("Enter")
        end = time.time() + 15  # the turn starts within seconds (or is already over, its report on screen)
        while time.time() < end and self.status() in IDLE and not (expect and self.shows(expect)):
            time.sleep(0.5)
        if expect:
            end = time.time() + 120
            while not self.shows(expect) and time.time() < end:
                time.sleep(1)
        self.wait_idle()
        self.settled()

    def ensure_channel(self, channel):
        """The cases send direct: the first send auto-joins the pair, there is nothing to set up."""

    # ---------- cases ----------
    def case(self, case):
        """tmux-named/send-p0 | send-p1: one direct item to the idle named seat at T=0. An idle seat wakes (the
        content-free wake typed into its tmux composer, no `inbox` round trip) iff the priority is <= T; the silent
        control stays idle, never shows a wake line, and gets the item on its next turn."""
        action = case.split("/")[1]
        expect = expect_wake(action, "1:1", self.threshold)
        gap = SPACING_S - (time.time() - self.last_wake)
        if gap > 0:
            self.run.stopping.wait(gap)
        self.wait_idle()
        screen_before = self.screen(1000)
        calls_before = self.inbox_calls()
        mid = self.send(action.split("-")[1], f"{case} {self.run.run_id}", direct=True)
        t_wake = self.watch_for_wake(WAKE_WINDOW_S)
        woke = t_wake is not None
        checks = {}
        hello = False
        if woke:
            self.last_wake = time.time()
        else:
            checks["late_wake_s"] = self.watch_for_wake(20) if expect else None
            if checks["late_wake_s"] is None:
                hello = True
                self.prompt("hello")
                self.last_wake = time.time()
        self.wait_idle()
        self.settled()
        checks["delivered"] = self.on_screen(mid)
        wake_lines = wake_delta(screen_before, self.screen(1000))
        checks["wake_lines"] = wake_lines
        if woke:
            checks["inbox_calls"] = self.inbox_calls() - calls_before
            checks["no_extra_inbox"] = checks["inbox_calls"] == 0
        checks["trailing_wake_s"] = self.watch_quiet(max(0.0, SPACING_S - (time.time() - self.last_wake)))
        if checks["trailing_wake_s"] is not None:
            self.wait_idle()
            self.settled()
        verified = all(v is True for k, v in checks.items() if k in ("delivered", "no_extra_inbox"))
        # The wake line is on screen iff the daemon typed one: exactly one for the p0, none for the control.
        typed = wake_lines == (1 if expect else 0)
        passed = woke == expect and verified and typed and checks["trailing_wake_s"] is None
        self.run.record({"case": case, "harness": self.harness, "threshold": self.threshold, "recipient": self.name,
                         "expect_wake": expect, "woke": woke, "t_wake_s": t_wake, "hello_sent": hello,
                         "checks": checks, "ref": {"message": mid}, "result": "PASS" if passed else "FAIL",
                         "utc": utc()})


def record(self, row):
    if (row.get("checks") or {}).get("release_blocking"):
        row["result"] = "FAIL"  # a release-blocking defect fails its row whatever the verdict said
    for recipient in getattr(self, "recipients", ()):
        proof = getattr(recipient, "queue_evidence", None)
        if recipient.name == row.get("recipient") and proof is not None:
            evidence = proof.finish(row)
            row["queue_evidence"] = evidence
            row["wake_windows"] = proof.selected_windows
            row["actual_transport"] = ("unknown" if evidence.get("observed", {}).get("unknown") else
                                       "queue" if evidence.get("attempts") else
                                       "native_inference" if evidence.get("expectation") == "native" else "none")
            row.setdefault("checks", {})["queue_transport_ok"] = evidence["passed"]
            if not evidence["passed"]:
                row["result"] = "FAIL"
            break
    row = json.loads(redact(json.dumps(row), getattr(self, "secrets", ())))  # error tails may quote a screen
    with self.lock:
        self.results.append(row)
        print(f"[{row['result']:7}] {row['recipient']:9} {row['case']:17} T={row['threshold']!s:2} "
              f"expect={row.get('expect_wake')!s:5} woke={row.get('woke')!s:5} t={row.get('t_wake_s')} "
              f"{json.dumps(row.get('checks'))}", flush=True)


Run.record = record


def applies(recipient, case):
    """Side cases run on Codex recipients only; the plain-hooks Codex runs nothing else. Hooks cases run only on the
    hooks-only Claude, which runs nothing else. End-of-turn and busy-tool cases run on hooked Codex recipients
    (Claude's mod delivers at every tool already); Claude cases on the mod Claude recipients."""
    if case in QUEUE_NATIVE_CASES:
        return recipient.harness == "codex" and getattr(recipient, "plain", False)
    if getattr(recipient, "tmux", False) or case in TMUX_CASES:
        return getattr(recipient, "tmux", False) and case in TMUX_CASES
    if case in SIDE_CASES:
        return recipient.harness == "codex"
    if case in SIDE_QUEUE_CASES + ROOT_DRAFT_CASES + SIDE_DEFER_CASES + CONFIG_PROSE_CASES + FLAG_FLIP_CASES:
        return recipient.harness == "codex" and recipient.threshold == 0 and not getattr(recipient, "plain", False)
    if case in PROMPT_RACE_CASES:
        return (recipient.harness == "claude" and recipient.threshold == 0
                and not getattr(recipient, "hooks_only", False))
    if getattr(recipient, "hooks_only", False) or case in HOOKS_CASES:
        return getattr(recipient, "hooks_only", False) and case in HOOKS_CASES
    if case in EOT_CASES or case.startswith("busy-tool-"):
        return recipient.harness == "codex" and not getattr(recipient, "plain", False)
    if case in WAKE_BLOCK_CASES or case in UNRECEIVED_CASES or case in RESUME_CASES:
        return recipient.harness == "codex" and recipient.threshold == 0 and not getattr(recipient, "plain", False)
    if case in CLAUDE_CASES:
        return recipient.harness == "claude"
    return not getattr(recipient, "plain", False)


def expected_rows(run, cases):
    return {(r.name, c) for r in getattr(run, "recipients", []) for c in cases if applies(r, c)}


def summary(run, cases):
    """The table of every collected row. Passes only when each expected recipient x case has exactly one row and it
    is PASS: a missing, duplicated or unexpected row fails the run."""
    order = {c: i for i, c in enumerate(CASES)}
    rows = sorted(run.results, key=lambda r: (order.get(r["case"], 99), THRESHOLDS.index(r["threshold"]) if r["threshold"] in THRESHOLDS else 99,
                                              r["harness"]))
    lines = ["| case | T | harness | expect wake | woke (s) | checks | result |", "|---|---|---|---|---|---|---|"]
    for r in rows:
        woke = f"{r.get('woke')} ({r.get('t_wake_s')})" if r.get("woke") else str(r.get("woke"))
        lines.append(f"| {r['case']} | {r['threshold']} | {r['harness']} | {r.get('expect_wake')} | {woke} | "
                     f"{json.dumps(r.get('checks'))} | {r['result']} |")
    expected = expected_rows(run, cases)
    seen = [(r["recipient"], r["case"]) for r in rows]
    missing = sorted(expected - set(seen))
    extra = sorted({k for k in seen if k not in expected or seen.count(k) > 1})
    passed = sum(r["result"] == "PASS" for r in rows)
    lines.append(f"\n{passed}/{len(expected)} expected rows passed ({len(rows)} collected)")
    lines.extend(late_busy_entries(rows))
    if missing:
        lines.append(f"missing rows: {missing}")
    if extra:
        lines.append(f"unexpected or duplicated rows: {extra}")
    queue_rows = [r.get("queue_evidence") for r in rows if r.get("queue_evidence")]
    positive_queue = [q for q in queue_rows if q.get("expectation") == "accepted"]
    queue_lifecycle_ok = not positive_queue or any(
        q.get("passed") and q.get("lifecycle_attempts")
        for q in positive_queue)
    if positive_queue:
        lines.append(f"queue logical fulfillment + bounded cancellation observed: {queue_lifecycle_ok}")
    run.table = "\n".join(lines)
    run.completeness = {"expected": len(expected), "collected": len(rows), "missing": missing, "extra": extra}
    print("\n" + run.table, flush=True)
    return bool(expected) and not missing and not extra and passed == len(rows) and queue_lifecycle_ok


def codex_hook_guidance(run):
    """ht-static-once: Codex records each hook context as a developer message in its rollout (one file per session,
    without the hook event name). The fixed instruction is session-scoped: a session carries it at most once per
    session start plus once per compaction, never on its delivering tool or turn hooks. Counting per rollout keeps the
    judgement independent of which optional lines a near-budget start trimmed. None without a Codex recipient or
    rollout; else the counts and whether they pass."""
    sessions = getattr(run, "codex_home", Path("/nonexistent")) / "sessions"
    if not any(getattr(r, "harness", None) == "codex" for r in getattr(run, "recipients", [])) or not sessions.is_dir():
        return None
    marker, rows = "Before using threads, run herdr-threads skill", "herdr-threads inbox rows"
    counts = {"rollouts": 0, "guidance_contexts": 0, "delivering_hooks": 0, "over_allowance": 0}
    for path in sorted(sessions.rglob("*.jsonl")):
        guidance, compactions = 0, 0
        for line in path.read_text(errors="replace").splitlines():
            try:
                item = json.loads(line)
            except ValueError:
                continue
            if not isinstance(item, dict):
                continue
            payload = item.get("payload") if isinstance(item.get("payload"), dict) else {}
            if "compact" in f"{item.get('type')} {payload.get('type')}".lower():
                compactions += 1
            if item.get("type") != "response_item" or payload.get("role") != "developer":
                continue
            text = "".join(c.get("text", "") for c in payload.get("content") or [] if isinstance(c, dict))
            guidance += marker in text
            counts["delivering_hooks"] += rows in text
        counts["rollouts"] += 1
        counts["guidance_contexts"] += guidance
        counts["over_allowance"] += max(0, guidance - 1 - compactions)
    # A resume is a session start in the same rollout.
    counts["over_allowance"] = max(0, counts["over_allowance"] - getattr(run, "resumes", 0))
    counts["pass"] = counts["guidance_contexts"] > 0 and counts["over_allowance"] == 0
    return counts


def root_transcript_facts(run, facts):
    """facts["root_transcript"][<recipient>] for every hooked Codex recipient, from the hook trace; returns the report
    lines. A root event with a null transcript_path is reported, never a row verdict or the exit code (ht-z41e.30)."""
    ev, lines = getattr(run, "ev", None), []
    if ev is None:
        return lines
    trace = read_hook_trace(ev / HOOK_TRACE_FILE)
    bootstrap = (getattr(run, "facts", None) or {}).get("queue_bootstrap", {})
    for recipient in getattr(run, "recipients", ()):
        if getattr(recipient, "harness", None) != "codex" or getattr(recipient, "plain", False):
            continue
        mine = [r for r in trace if r.get("pane") == recipient.pane]
        root = (bootstrap.get(recipient.name) or {}).get("scope", {}).get("native_session")
        verdict = root_transcript_verdict(mine, root_sessions(mine, recipient.pane, root))
        facts.setdefault("root_transcript", {})[recipient.name] = verdict
        lines.append(root_transcript_line(recipient.name, verdict))
    return lines


def release_blocking_line(results):
    """`release-blocking defects: <recipient> <case>: <names>; ...` for every row whose checks name one, else ''."""
    found = [f"{r.get('recipient')} {r.get('case')}: {', '.join(names)}" for r in results
             for names in [(r.get("checks") or {}).get("release_blocking") or []] if names]
    return "release-blocking defects: " + "; ".join(found) if found else ""


def write_evidence(run, rc):
    """report.md / results.json, written after cleanup so they carry its outcome and the final exit code."""
    facts = dict(getattr(run, "facts", {}), finished_utc=utc(), exit_code=rc)
    if run.auth_digest:
        copy = run.codex_home / "auth.json"
        refreshed = run.auth_refreshed
        if refreshed is None and copy.exists():  # --keep: the copy was not retired
            refreshed = file_sha256(copy) != run.auth_digest
        facts["codex_auth_refreshed_in_run"] = refreshed
    root_lines = root_transcript_facts(run, facts)
    with run.lock:  # workers left running after an interrupt may still append
        results = list(run.results)
    blocking = release_blocking_line(results)
    if blocking:
        facts["release_blocking_defects"] = blocking
        root_lines = root_lines + [blocking]
    doc = {"facts": facts, "completeness": getattr(run, "completeness", None), "cleanup": run.cleanup_report,
           "codex_hook_guidance": getattr(run, "hook_guidance", None), "results": results}
    # One redaction pass over each whole document: facts (versions, flags) and cleanup diagnostics carry helper text.
    secrets = getattr(run, "secrets", None)
    secrets = secret_values(os.environ) if secrets is None else secrets
    (run.ev / "results.json").write_text(redact(json.dumps(doc, indent=2), secrets))
    (run.ev / "report.md").write_text(redact(
        f"# wake e2e {run.run_id}\n\n```json\n{json.dumps(facts, indent=2)}\n```\n\n{getattr(run, 'table', '')}\n\n"
        f"codex hook guidance: `{json.dumps(getattr(run, 'hook_guidance', None))}`\n\n"
        + "".join(line + "\n\n" for line in root_lines) +
        f"cleanup: `{json.dumps(run.cleanup_report)}`\n\nexit code: {rc}\n", secrets))


def main(argv=None):
    ap = argparse.ArgumentParser(prog="wake-e2e", description=__doc__.splitlines()[0])
    ap.add_argument("--bin", default=str(REPO / "bin" / "herdr-threads"), help="herdr-threads binary under test")
    ap.add_argument("--harness", default=DEFAULT_HARNESSES,
                    help="comma-separated: claude,codex; opt-in: agy (credential from %s), hermes "
                         "(--hermes-home), opencode (credential from %s)" % (AGY_CREDENTIAL_ENV,
                                                                           OPENCODE_CREDENTIAL_ENV))
    ap.add_argument("--cases", default=",".join(CASES), help="comma-separated subset of: " + ", ".join(CASES)
                    + "; opt-in, Codex only: " + ", ".join(SIDE_CASES + EOT_CASES + WAKE_BLOCK_CASES + UNRECEIVED_CASES
                                                      + RESUME_CASES + QUEUE_NATIVE_CASES + SIDE_QUEUE_CASES + ROOT_DRAFT_CASES
                                                      + SIDE_DEFER_CASES + CONFIG_PROSE_CASES + FLAG_FLIP_CASES)
                    + " (" + ", ".join(SIDE_QUEUE_CASES + ROOT_DRAFT_CASES + FLAG_FLIP_CASES) + " need --queue-proof; "
                    + FOCUSED_QUEUE_CASE + " also needs a private Herdr (scripts/wake-e2e-herdr) and runs alone or "
                    + "beside " + ", ".join(ROOT_DRAFT_CASES) + "; "
                    + ", ".join(SIDE_DEFER_CASES + CONFIG_PROSE_CASES) + " are refused under it)"
                    + "; opt-in, hooks-only Claude: " + ", ".join(HOOKS_CASES)
                    + "; opt-in, mod Claude at T=0: " + ", ".join(PROMPT_RACE_CASES)
                    + "; opt-in, a named seat in a private tmux server (claude): " + ", ".join(TMUX_CASES))
    ap.add_argument("--evidence-dir", help="default: sibling ht-wake-evidence.<run id>")
    ap.add_argument("--queue-proof", action="store_true",
                    help="Codex qualification: require canonical Accepted queue evidence for eligible idle wakes")
    ap.add_argument("--keep", action="store_true", help="leave the workspace and daemon running for inspection")
    ap.add_argument("--claude-model", help="passed as --model (default: the account default)")
    ap.add_argument("--claude-permission-mode", default="auto")
    ap.add_argument("--harness-mode", choices=HARNESS_MODES, default=default_harness_mode(),
                    help="macos: Claude keeps its real (keychain) config dir; linux: scratch Claude config, credential "
                         "from CLAUDE_CODE_OAUTH_TOKEN/ANTHROPIC_API_KEY, Codex sandbox bypassed (default: %(default)s)")
    ap.add_argument("--claude-config-dir",
                    help="macos mode: the real, logged-in Claude config dir the agents use (default $CLAUDE_CONFIG_DIR; "
                         "new Herdr panes do not inherit it); refused in linux mode")
    ap.add_argument("--codex-model", help="passed as -m (default: the profile default)")
    ap.add_argument("--codex-args",
                    help="extra Codex TUI flags (sandbox/approval), default %r; %r only in the linux mode "
                         "inside the scripts/linux-e2e runner container" % (CODEX_SANDBOXED_ARGS, CODEX_BYPASS_ARGS))
    ap.add_argument("--codex-auth-home", help="Codex home whose auth.json is copied (private, mode 0600) into "
                                              "the scratch CODEX_HOME "
                                              "(default $CODEX_HOME or ~/.codex)")
    ap.add_argument("--opencode-model", default=OPENCODE_MODEL,
                    help="--harness opencode: the provider/model its scratch opencode.json selects (default %(default)s)")
    ap.add_argument("--pi", help="--harness pi: native runtime for the dedicated isolated driver")
    ap.add_argument("--pi-cases", help="--harness pi: comma-separated cases from the dedicated native driver")
    ap.add_argument("--pi-live", action="store_true", help="--harness pi: opt-in authorized live OpenRouter qualification")
    ap.add_argument("--pi-openrouter-env-file", help="--pi-live: explicitly authorized env file; only OPENROUTER_API_KEY is read")
    ap.add_argument("--pi-model", help="--pi-live: deliberately selected DeepSeek model id")
    ap.add_argument("--hermes-home", help="--harness hermes: a scratch HOME Hermes is installed and logged in to "
                                          "(setup and the plugin enable write there)")
    ap.add_argument("--self-test", action="store_true",
                    help="check the driver's credential, cleanup and completeness paths (no harnesses) and exit")
    args = ap.parse_args(argv)
    if args.self_test:
        return self_test()
    if args.harness == "pi":
        if args.keep or args.cases != ",".join(CASES):
            ap.error("pi uses its own driver: select --pi-cases; --keep is unsupported")
        command = [sys.executable, str(REPO / "scripts/pi-tui-qualify.py"), "--bin", args.bin]
        for flag, value in (("--pi", args.pi), ("--cases", args.pi_cases), ("--evidence-dir", args.evidence_dir),
                            ("--openrouter-env-file", args.pi_openrouter_env_file), ("--model", args.pi_model)):
            if value:
                command.extend([flag, value])
        if args.pi_live:
            command.append("--live")
        os.execv(sys.executable, command)
    if args.harness_mode == "linux" and args.claude_config_dir:
        ap.error("--claude-config-dir: the linux harness mode never uses a real Claude config")
    if args.harness_mode == "macos" and args.claude_config_dir is None:
        args.claude_config_dir = os.environ.get("CLAUDE_CONFIG_DIR")
    if args.codex_args is None:
        args.codex_args = default_codex_args(args.harness_mode, os.environ)
        if marker_without_dockerenv(args.harness_mode, os.environ):
            print(f"wake-e2e: {CONTAINER_MARKER}=1 but {DOCKERENV} is missing (not Docker?): Codex keeps its sandbox "
                  "(--approve-for-me); pass --codex-args to choose explicitly", file=sys.stderr)
    args.harness = [h for h in args.harness.split(",") if h]
    if args.queue_proof and "codex" not in args.harness:
        ap.error("--queue-proof requires Codex recipients")
    cases = [c for c in args.cases.split(",") if c]
    unknown = ([c for c in cases
                if c not in CASES + SIDE_CASES + HOOKS_CASES + EOT_CASES + WAKE_BLOCK_CASES + UNRECEIVED_CASES
                                 + RESUME_CASES + TMUX_CASES + PROMPT_RACE_CASES + QUEUE_NATIVE_CASES
                                 + SIDE_QUEUE_CASES + ROOT_DRAFT_CASES + SIDE_DEFER_CASES + CONFIG_PROSE_CASES
                                 + FLAG_FLIP_CASES]
               + [h for h in args.harness if h not in HARNESSES])
    if unknown:
        ap.error(f"unknown case/harness: {unknown}")
    for case in cases:
        if case in SIDE_QUEUE_CASES + ROOT_DRAFT_CASES + FLAG_FLIP_CASES and not args.queue_proof:
            ap.error(f"{case} requires --queue-proof")
        if case in SIDE_DEFER_CASES + CONFIG_PROSE_CASES and args.queue_proof:
            ap.error(f"{case} runs without --queue-proof")
    if FOCUSED_QUEUE_CASE in cases:
        # Focusing a pane moves the user's focus: only inside the private server scripts/wake-e2e-herdr starts.
        if not focused_queue_private_herdr(os.environ):
            ap.error(f"{FOCUSED_QUEUE_CASE} focuses a pane: run it through scripts/wake-e2e-herdr "
                     f"({PRIVATE_HERDR_ENV}=1 and HERDR_SOCKET_PATH set)")
        if any(c != FOCUSED_QUEUE_CASE and c not in ROOT_DRAFT_CASES for c in cases):
            ap.error(f"{FOCUSED_QUEUE_CASE} runs alone or beside " + ", ".join(ROOT_DRAFT_CASES)
                     + ": its focus must not skew other cases")
    if any(c in PROMPT_RACE_CASES for c in cases) and not all(c in PROMPT_RACE_CASES for c in cases):
        ap.error("the prompt-race cases (" + ", ".join(PROMPT_RACE_CASES) + ") run alone: their slow "
                 "UserPromptSubmit hook would skew other cases")
    def terminate(signum, _frame):
        raise KeyboardInterrupt(f"signal {signum}")
    try:
        run = Run(args)
    except Refused as e:
        print(f"wake-e2e: refused: {e}", file=sys.stderr)
        return EXIT_REFUSED
    saved = {sig: signal.getsignal(sig) for sig in (signal.SIGINT, signal.SIGTERM)}
    signal.signal(signal.SIGTERM, terminate)  # SIGTERM takes the same cleanup path as Ctrl-C
    run.side_cases = any(c in SIDE_CASES for c in cases)
    run.native_queue_case = any(c in QUEUE_NATIVE_CASES for c in cases)
    run.hooks_cases = any(c in HOOKS_CASES for c in cases)
    run.prompt_race_cases = any(c in PROMPT_RACE_CASES for c in cases)
    run.tmux_cases = any(c in TMUX_CASES for c in cases)
    rc = EXIT_FAIL
    try:
        run.preflight()
        run.scratch()
        run.layout()
        workers = [threading.Thread(target=r.run_cases, args=(cases,), name=r.name, daemon=True)
                   for r in run.recipients]
        run.workers = workers  # registered before starting; drain skips any that never started
        for w in workers:
            run.start_attempted.add(w)
            w.start()
        while any(w.is_alive() for w in workers):  # short joins keep Ctrl-C / SIGTERM responsive
            for w in workers:
                if w.is_alive():
                    w.join(timeout=0.5)
        rc = EXIT_OK if summary(run, cases) else EXIT_FAIL
        run.hook_guidance = codex_hook_guidance(run)
        if run.hook_guidance is not None:
            print(f"codex hook guidance (static once per session): {json.dumps(run.hook_guidance)}", flush=True)
            if not run.hook_guidance["pass"]:
                rc = EXIT_FAIL
    except Refused as e:
        print(redact(f"wake-e2e: refused: {e}", secret_values(os.environ)), file=sys.stderr)
        rc = EXIT_REFUSED
    except BaseException as e:  # including KeyboardInterrupt: cleanup still runs and the run fails
        print(redact(f"wake-e2e: aborted: {e!r}", secret_values(os.environ)), file=sys.stderr)
        rc = EXIT_FAIL
    finally:
        # Cleanup runs to the end: a second Ctrl-C or SIGTERM must not cut off the remaining steps.
        signal.signal(signal.SIGINT, signal.SIG_IGN)
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        try:
            problems = run.cleanup()
        except BaseException as e:
            problems = [f"cleanup raised: {e!r}"]
            run.cleanup_report = {"problems": problems}
        if problems:
            print(redact("wake-e2e: cleanup incomplete: " + "; ".join(problems), secret_values(os.environ)),
                  file=sys.stderr)
            rc = EXIT_FAIL
        write_evidence(run, rc)
        # The last gate: no evidence file may hold a credential. A hit is scrubbed and fails the run (only the file
        # names are printed).
        secrets = getattr(run, "secrets", None) or secret_values(os.environ)
        leaked = scrub_evidence(run.ev, secrets) if run.ev is not None else []
        if leaked:
            print(f"wake-e2e: credential found in evidence (scrubbed): {', '.join(leaked)}", file=sys.stderr)
            rc = EXIT_FAIL
            run.facts = dict(getattr(run, "facts", {}), evidence_scrubbed=leaked)
            write_evidence(run, rc)
            if scrub_evidence(run.ev, secrets):
                print("wake-e2e: evidence still holds a credential after scrubbing", file=sys.stderr)
        print(f"evidence: {run.ev} (exit {rc})", flush=True)
        for sig, handler in saved.items():
            signal.signal(sig, handler)
    return rc


SELF_TEST = Path(__file__).resolve().parent / "wake_e2e_selftest.py"


def self_test():
    """The driver's own failure paths, with fakes for Herdr and the daemon (scripts/wake_e2e_selftest.py)."""
    import unittest
    sys.path.insert(0, str(SELF_TEST.parent))
    suite = unittest.defaultTestLoader.loadTestsFromName(SELF_TEST.stem)
    suite.addTests(unittest.defaultTestLoader.loadTestsFromName("wake_e2e_queue_selftest"))
    return EXIT_OK if unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful() else EXIT_FAIL


if __name__ == "__main__":
    sys.exit(main())
