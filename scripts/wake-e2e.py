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
         "busy-send-p1/1:1", "busy-send-p0/1:1", "busy-send-ack/1:1", "busy-fail-send-ack/group")
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
# Opt-in Claude case, never in the default matrix: `cl-hooks`, a Claude installed by `setup claude --hooks-only` (no
# delivery mod), gets an ACK request while idle at T=0. Herdr must type the content-free `herdr-threads: wake up`, the
# UserPromptSubmit hook must put the message into the woken turn, and its ACK must come from that hook's display
# (`via: hook_context`), not from an `inbox` round trip. cl-hooks runs nothing else.
HOOKS_CASES = ("hooks-wake/group",)
# Opt-in Codex case (ht-end-of-turn), never in the default matrix: the p0 is sent while the turn's only tool is held, so
# it arrives after the last tool boundary. The turn's Stop hook must continue it with the page as the continuation
# prompt (a `<hook_prompt>` in the rollout naming the message), with no wake typed during or after the turn.
EOT_CASES = ("eot-send-p0/group",)
EOT_PROMPT = ("End-of-turn test {tag}: run the shell command `{hold}` and no other command, then print EOT-DONE and end "
              "your turn. If herdr-threads items reach you after that, handle them exactly as your standing "
              "instructions say without running any command, and print EOT-DONE again.")
EOT_DONE_LINE = re.compile(r"^[^\w\n]*EOT-DONE[^\w\n]*$", re.M)
SIDE_FORKS = 2
# Opt-in, Codex at T=0 only (ht-wake-outcome): with Codex's `/model` picker open, p0 sends neither wake the recipient
# nor type into the picker (its highlighted `› ` row reads as a draft, a hold that ends by itself), and the sender gets
# no `wake=` warning (only lasting blocks warn); once the picker is closed the recipient wakes and gets every item.
# (`wake=unreadable` cannot be staged this way on Codex: no captured Codex screen lacks a `› ` row.)
WAKE_BLOCK_CASES = ("modal-hold/group",)
WAKE_BLOCK_HOLD_S = 30
PICKER_ROW = re.compile(r"^\u203a 1\. ")
WAKE_BLOCK_WAIT_S = 90
SIDE_DEADLINE_S = 45
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
BUSY_AFTER_S = 40.0
# The busy turn's native end: a line of the agent's output that is BUSY-DONE apart from bullets, markup or a final
# period. It is searched only after the prompt's own "BUSY-DONE." so a wrapped prompt line never matches.
BUSY_DONE_LINE = re.compile(r"^[^\w\n]*BUSY-DONE[^\w\n]*$", re.M)
BUSY_PROMPT_END = "BUSY-DONE."
BUSY_END_GRACE_S = 10.0  # idle this long with no BUSY-DONE line: the turn is over without one
# Channel literal pinned to policy::CHANNEL_WAKE_UP by tests/harness/claude_mod.rs.
CHANNEL_WAKE_UP = "wake up"
# Keep the native and pre-upgrade channel forms ahead of their short suffix.
WAKE_MARKERS = ("herdr-threads: wake up", "herdr-threads: attention pending", CHANNEL_WAKE_UP)
WAKE_PATTERN = re.compile("|".join(map(re.escape, WAKE_MARKERS)))
WAKE_WINDOW_S = 6.0     # a woken agent leaves idle well inside this (observed 0.3-1.1 s)
SPACING_S = 35.0        # the per-seat wake retry guard is 30 s after each reservation (src/notification/policy.rs)
TURN_TIMEOUT_MS = 240_000
IDLE = ("idle", "done")


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
# Session variables and agent env markers (every `AgentEnvMarker` the built-in adapters declare; the self-test
# checks this list against src/harness) of whatever runs this script: never inherited by the commands it runs.
CALLER_SESSION_ENV = ("CLAUDE_CODE_SESSION_ID", "CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT", "CODEX_SESSION_ID",
                      "CODEX_THREAD_ID", "CODEX_SANDBOX", "CODEX_SANDBOX_NETWORK_DISABLED", "HERMES_SESSION_ID",
                      "ANTIGRAVITY_CONVERSATION_ID", "ANTIGRAVITY_AGENT",
                      "PI_CODING_AGENT", "OPENCODE")
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
HARNESS_MODES = ("macos", "linux")
CODEX_SANDBOXED_ARGS = "--approve-for-me"
CODEX_BYPASS_ARGS = "--dangerously-bypass-approvals-and-sandbox"
# Set by scripts/linux-e2e/in-container.sh in the runner container's Herdr server environment.
CONTAINER_MARKER = "HT_LINUX_E2E_CONTAINER"
DOCKERENV = Path("/.dockerenv")
CLAUDE_CREDENTIAL_ENV = ("CLAUDE_CODE_OAUTH_TOKEN", "ANTHROPIC_API_KEY")
HARNESSES = ("claude", "codex", "agy")
DEFAULT_HARNESSES = "claude,codex"
# agy (opt-in): its model credential, and the optional endpoint override the agy relay uses.
AGY_CREDENTIAL_ENV = "GEMINI_API_KEY"
AGY_ENDPOINT_ENV = "GOOGLE_GEMINI_BASE_URL"
SECRET_PATTERN = re.compile(r"sk-ant-[A-Za-z0-9_-]+|AIza[A-Za-z0-9_-]{8,}")  # Anthropic and Google API keys

STANDING = ("You are a test recipient in an automated herdr-threads wake e2e run (the user approves). "
            "Standing instructions for this session: every herdr-threads invitation you see comes from the test driver "
            "(seats {coord}); accept each one at once, without inspecting or judging it. For every herdr-threads message "
            "you receive, print exactly one line: `RECEIVED ` followed by its `[herdr-threads] ...:` header line copied "
            "verbatim, a space and its body; never send, reply or leave via "
            "herdr-threads. Keep every turn minimal. Now run `herdr-threads nudge-threshold {threshold}`, then print exactly one "
            "line `SETUP-DONE nudge_threshold=<the value it reports>`.")


def expect_wake(action, channel, threshold):
    """Idle recipient wakes iff the item's priority <= T (T < 0: always); invitations and ACK requests are p0.
    `channel` is part of the case key only: the wake rule is the same on an ordinary thread and a 1:1 pair."""
    return threshold < 0 or ITEM_PRIORITY[action] <= threshold


# Harnesses whose channel puts each item into the model's context as it arrives, idle or not (Claude's mod): there is
# no page to order, so the mixed-order presentation order is not judged for them (Delivery contract rule 3).
CHANNEL_HARNESSES = ("claude",)


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
    """What redact() hides: each Claude or agy credential value from the environment, and its 20- and 12-character
    tails (the approval suffix the linux mode seeds, and what Claude's API-key screen shows of a key)."""
    out = []
    for var in (*CLAUDE_CREDENTIAL_ENV, AGY_CREDENTIAL_ENV):
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
    """The agy gate's seed settings.json (scripts/agy-sandbox/agyenv.py settings_dict), for the agents' cwd."""
    return {"altScreenMode": "never", "modelProvider": "gemini", "trustedWorkspaces": [cwd]}


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


def agy_onboarding_keys(screen):
    """The keys that answer one agy first-run screen (scripts/agy-gate/live_smoke.py walk_onboarding), or None."""
    if "Choose your color scheme" in screen:
        return [("enter",)]
    if "Terms of Service" in screen:
        return [("down", "right"), ("enter",)]
    return None


def agy_composer_up(screen):
    """agy's composer is drawn (its footer hint), so no first-run screen is left."""
    return "for shortcuts" in screen


def linux_startup_refusal(screen, status):
    """The linux mode: why a freshly started agent must not be prompted, or None. Checked on the whole visible
    screen: the theme picker reports `idle` and its prompt sits far above the screen's last 400 characters."""
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
        self.claude_env = self.root / "claude.env"  # linux: the Claude credential, written only when one is set
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
        e = dict(os.environ)
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
        for tool in ("herdr", *self.args.harness):
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
        if problems:
            raise Refused("; ".join(problems))
        versions = {}
        for name, argv in (("herdr", ["herdr", "--version"]), ("herdr-threads", [self.bin, "--version"]),
                           *((h, [h, "--version"]) for h in self.args.harness)):
            versions[name] = clip(self.run(argv, tag="version")[1].strip(), 120, getattr(self, 'secrets', None))
        sha = self.run(["git", "-C", str(REPO), "rev-parse", "HEAD"], tag="git")[1].strip()
        dirty = bool(self.run(["git", "-C", str(REPO), "status", "--porcelain"], tag="git")[1].strip())
        self.facts = {"run_id": self.run_id, "sha": sha, "dirty": dirty, "versions": versions,
                      "harness_mode": self.args.harness_mode, "codex_args": self.args.codex_args,
                      "bin": self.bin, "bin_sha256": file_sha256(self.bin), "root": str(self.root),
                      "started_utc": utc()}
        print(f"wake-e2e {self.run_id}: sha {sha[:12]}{' (dirty)' if dirty else ''}, root {self.root}", flush=True)
        for k, v in versions.items():
            print(f"  {k}: {v}", flush=True)

    @property
    def calls_log(self):
        return self.bindir / "calls.log"

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
                        "rec=$(printf '%s\\037' \"${HERDR_PANE_ID:-}\" \"$@\")\n"
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
        (self.state / "node/settings.json").write_text('{"sync":{"discover":false}}\n')
        (self.state / "node/settings.json").chmod(0o600)
        self.daemon_attempted = True
        self.ht("daemon", "ensure", tag="daemon:ensure", timeout=60)
        self.ht("daemon", "health", tag="daemon:health", timeout=20)
        setup_env = {"HOME": str(self.home), "CLAUDE_CONFIG_DIR": str(self.claude_cfg), "CODEX_HOME": str(self.codex_home),
                     "CLAUDE_CODE_PLUGIN_DIRS": ""}  # never copy a real installed mod into the scratch layer
        # cl-hooks first: `--hooks-only` removes the mod files of the shared state dir, which the full Claude setup
        # below then reinstalls for the other Claude recipients.
        if getattr(self, "hooks_cases", False):
            self.claude_hooks_cfg.mkdir(mode=0o700, parents=True, exist_ok=True)
            self.ht("setup", "claude", "--hooks-only", "--with-permissions", tag="setup:claude:hooks-only",
                    env=dict(setup_env, CLAUDE_CONFIG_DIR=str(self.claude_hooks_cfg)), timeout=120)
            if self.linux:
                seed_claude_config(self.claude_hooks_cfg, self.cwd, os.environ)
        if "agy" in self.args.harness:
            seed_agy_home(self.home, self.cwd)
            write_private(self.agy_env, "".join(f"export {k}={shlex.quote(v)}\n" for k, v in agy_env(os.environ)))
        for h in self.args.harness:
            self.ht("setup", h, "--with-permissions", env=setup_env, tag=f"setup:{h}", timeout=120)
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
        print(f"  isolated daemon up; hooks installed by setup for {', '.join(self.args.harness)}", flush=True)

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
        self.herdr("tab", "rename", data["result"]["tab"]["tab_id"], "coordinator")
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
                name = f"{h[:2]}-t{threshold}"
                pane = self.tab(name)
                self.recipients.append(Recipient(self, name, h, threshold, pane, self.seat(pane)))
        if "codex" in self.args.harness and getattr(self, "side_cases", False):
            pane = self.tab("co-plain")
            self.recipients.append(Recipient(self, "co-plain", "codex", 0, pane, self.seat(pane), plain=True))
        if "claude" in self.args.harness and getattr(self, "hooks_cases", False):
            pane = self.tab("cl-hooks")
            self.recipients.append(Recipient(self, "cl-hooks", "claude", 0, pane, self.seat(pane), hooks_only=True))
        print(f"  workspace {self.workspace}: coordinators {self.coord_seats()}; " +
              ", ".join(f"{r.name}={r.seat}" for r in self.recipients), flush=True)

    def launch_line(self, harness, plain=False, hooks_only=False):
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
            else:
                cfg += f"CLAUDE_CODE_PLUGIN_DIRS={q(str(self.state / 'claude-mod' / 'herdr-threads'))} "
            line = f"unset {unset}; {path} {cfg}HERDR_AGENT=claude {shlex.join(argv)}"
            env_file = getattr(self, "claude_env", None)
            if env_file is not None and env_file.exists():
                return f"( . {q(str(env_file))} && {line} )"
            return line
        if harness == "agy":
            # The credential stays in the private env file; the pane's command line and screen never carry it.
            return (f"( . {q(str(self.agy_env))} && {path} HOME={q(str(self.home))} HERDR_AGENT=agy "
                    f"exec agy )")
        # shell_snapshot off: the snapshot replays the interactive ~/.zshrc PATH, which can put an installed
        # herdr-threads ahead of the candidate's bin dir (ht-l16); a non-interactive login shell keeps it first.
        argv = ["command", "codex", "--no-daemon", "-C", self.cwd, "--dangerously-bypass-hook-trust",
                "-c", "check_for_update_on_startup=false", "-c", "features.shell_snapshot=false",
                *shlex.split(self.args.codex_args)]
        if self.args.codex_model:
            argv += ["-m", self.args.codex_model]
        if plain:
            argv += ["-c", "features.codex_hooks=false"]
        return f"{path} HERDR_AGENT=codex CODEX_HOME={q(str(self.codex_home))} {shlex.join(argv)}"

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

    def wait_entered(self, timeout=60):
        end = time.monotonic() + timeout
        while not (self.path / "entered").exists():
            if time.monotonic() >= end:
                raise RuntimeError("busy tool did not enter")
            time.sleep(.02)
        self.assert_held()

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


def busy_turn_split(text, anchor, done=BUSY_DONE_LINE, prompt_end_text=BUSY_PROMPT_END, last=False):
    """Split the screen at the busy turn's native boundaries: (in_turn, after_turn, end_seen). `in_turn` runs from
    the busy prompt `anchor` to the agent's done line (the last one with `last`: a continued turn prints it twice),
    `after_turn` is everything later; None if the prompt is not on screen."""
    at = text.find(anchor)  # the prompt itself: an agent echoing the tag later must not move it
    if at < 0:
        return None
    rest = text[at:]
    prompt_end = rest.find(prompt_end_text)
    ends = list(done.finditer(rest, prompt_end + len(prompt_end_text) if prompt_end >= 0 else 0))
    if not ends:
        return rest, "", False
    end = ends[-1] if last else ends[0]
    return rest[:end.end()], rest[end.end():], True


def is_wake_line(text):
    text = text.strip().lstrip("›❯> ")  # Codex, Claude, agy prompt glyphs
    return text in ("herdr-threads: wake up", CHANNEL_WAKE_UP) or text.startswith("herdr-threads: attention pending")


def wake_count(text):
    # Match whole prompt/body lines; an explanation mentioning "wake up" is not a wake.
    return sum(is_wake_line(line) for line in text.splitlines())


def admission_observation(observations):
    """The ACK observation to judge: one that carries a `row` if any does, else the first, else None."""
    return next((o for o in observations if o.get("row")), observations[0] if observations else None)


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
        return self.run.herdr("agent", "get", self.pane)["result"]["agent"]["agent_status"]

    def screen(self, lines=400):
        data = self.run.herdr("agent", "read", self.pane, "--source", "recent-unwrapped", "--lines", str(lines),
                              check=False)
        if "raw" in data:
            return data["raw"]
        return self.run.herdr("agent", "read", self.pane, "--source", "visible", check=False).get("raw", "")

    def dismiss_notice(self):
        """Cancel Claude's between-turn notice (HARNESS_NOTICE) when it covers the composer; True when one was."""
        if self.harness != "claude":
            return False
        visible = self.run.herdr("agent", "read", self.pane, "--source", "visible", check=False).get("raw", "")
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
        text = self.run.herdr("agent", "read", self.pane, "--source", "visible", check=False).get("raw", "")
        return " | ".join(l.strip() for l in text.splitlines() if l.strip())[-400:]

    def tail(self):
        """The same screen as diagnostic text for errors and evidence: redacted whole, then the last 400 characters."""
        text = self.run.herdr("agent", "read", self.pane, "--source", "visible", check=False).get("raw", "")
        return clip(" | ".join(l.strip() for l in text.splitlines() if l.strip()), 400,
                    getattr(self.run, "secrets", None), tail=True)

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
        visible = self.run.herdr("agent", "read", self.pane, "--source", "visible", check=False).get("raw", "")
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
            else:
                line = self.run.launch_line(self.harness)
            self.run.herdr("pane", "run", self.pane, line)
            end = time.time() + 180
            while time.time() < end:
                if self.harness == "agy":  # a fresh scratch HOME meets agy's first-run screens every run
                    keys = agy_onboarding_keys(self.run.herdr("pane", "read", self.pane, "--source", "visible",
                                                              check=False).get("raw", ""))
                    for i, k in enumerate(keys or ()):
                        if i:
                            time.sleep(0.5)
                        self.run.herdr("pane", "send-keys", self.pane, *k, check=False)
                    if keys or not agy_composer_up(self.run.herdr("pane", "read", self.pane, "--source", "visible",
                                                                  check=False).get("raw", "")):
                        time.sleep(1)  # Herdr may report idle behind a first-run screen
                        continue
                data = self.run.herdr("agent", "get", self.pane, check=False)
                agent = (data.get("result") or {}).get("agent") or {}
                if agent.get("agent") == self.harness and agent.get("agent_status") in IDLE + ("blocked",):
                    break
                time.sleep(1)
            else:
                raise RuntimeError(f"{self.name}: {self.harness} not detected in {self.pane}: {self.tail()}")
        if getattr(self.run.args, "harness_mode", "macos") == "linux":
            screen = self.run.herdr("agent", "read", self.pane, "--source", "visible", check=False).get("raw", "")
            why = linux_startup_refusal(screen, self.status())
            if why and why.startswith("is not logged in"):
                raise RuntimeError(f"{self.name}: {self.harness} {why}")
            if why:
                raise RuntimeError(f"{self.name}: {why}: {self.tail()}")
        else:
            if re.search(r"Not logged in|Please run /login", self.observed_tail()):
                raise RuntimeError(f"{self.name}: {self.harness} is not logged in (check --claude-config-dir)")
            if self.status() == "blocked" or DIALOG.search(self.observed_tail()):
                raise RuntimeError(f"{self.name}: startup dialog (nothing typed): {self.tail()}")
        for _attempt in range(2):  # one retry: a transient model/capacity error ends the first turn early
            reported = re.compile(r"SETUP-DONE nudge_threshold=%s\b" % re.escape(str(self.threshold)))
            self.prompt(STANDING.format(coord=self.run.coord_seats(), threshold=self.threshold)
                        + (PLAIN_HINT.format(shim=self.run.bindir / "herdr-threads") if self.plain else ""),
                        expect=reported)
            if self.shows(reported):
                break
        else:
            raise RuntimeError(f"{self.name}: agent did not report nudge-threshold {self.threshold}: {self.tail()}")

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

    # ---------- busy cases ----------
    def busy_case(self, case):
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
        with BusyBarrier(self.run.root) as hold:
            prompt = BUSY_FAIL_PROMPT if fail else BUSY_PROMPT
            self.run.herdr("agent", "prompt", self.pane, prompt.format(tag=tag, hold=hold.command()))
            hold.wait_entered()
            busy_at_send = self.status()
            checks = {"busy_at_send": busy_at_send}
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
        while time.time() < end and not (busy_turn_split(self.screen(1000), anchor) or ("", "", False))[2]:
            idle_since = (idle_since or time.time()) if self.status() in IDLE else None
            if idle_since and time.time() - idle_since > BUSY_END_GRACE_S:
                break  # recorded below as busy_turn_end_seen False, without waiting out the turn timeout
            time.sleep(0.5)
        while time.time() < end and self.status() not in IDLE:
            time.sleep(0.25)
        # A wake owed after the turn (even one held by the 30 s per-seat guard) would start a turn in here.
        checks["self_started_turn_after_s"] = self.watch_quiet(BUSY_AFTER_S)
        self.wait_idle()
        split = busy_turn_split(self.screen(1000), anchor)
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
            if kind == "ack":
                checks["ack"] = self.receipt(ref["message"])
        self.settled()
        passed = (busy_at_send not in IDLE and checks["self_started_turn_after_s"] is None
                  and checks["busy_turn_end_seen"] is True
                  and checks["wake_prompts_in_busy_turn"] == 0 and checks["wake_prompts_after_busy_turn"] == 0
                  and all(v is True or v == "acknowledged" for k, v in checks.items()
                          if k in ("joined", "opening_message_delivered_in_turn", "delivered_in_turn", "ack")))
        self.run.record({"case": case, "harness": self.harness, "threshold": self.threshold, "recipient": self.name,
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
            self.run.herdr("agent", "prompt", self.pane, EOT_PROMPT.format(tag=tag, hold=hold.command()))
            hold.wait_entered()
            busy_at_send = self.status()
            checks = {"busy_at_send": busy_at_send}
            mid = self.send("p0", f"{case} {self.run.run_id}", direct=False)
            hold.assert_held()
        anchor = f"End-of-turn test {tag}"
        split_at = lambda: busy_turn_split(self.screen(1000), anchor, EOT_DONE_LINE, "EOT-DONE and end", last=True)
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
        keys = ("session", "provenance", "liveness", "pid", "disconnect")
        return [dict(zip(keys, row)) for row in self.store_rows(
            "SELECT native_session,observation_provenance,liveness,liveness_pid,disconnect_reason "
            "FROM occupant_bindings WHERE seat_id=? AND role='top_level' AND ended_at IS NULL ORDER BY ordinal",
            (self.seat,))]

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
        self.run.herdr("agent", "send-keys", self.pane, "ctrl+c", check=False)
        time.sleep(2)
        self.wait_idle()
        # A wake attempted while the side turn was busy is refused and retried after the per-seat guard.
        checks["sibling_wake_s"] = self.watch_for_wake(SPACING_S + WAKE_WINDOW_S)
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

    def wake_state(self):
        rows = self.store_rows("SELECT last_outcome,completed_at_utc,retry_step FROM wake_work WHERE seat_id=?",
                               (self.seat,))
        return dict(zip(("last_outcome", "completed_at_utc", "retry_step"), rows[0])) if rows else None

    def open_sides(self, tag):
        """SIDE_FORKS `/side` conversations, each making one herdr-threads `inbox` call under its own session,
        then Ctrl+C back to the main thread. Returns what the store holds for the seat afterwards."""
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
            self.run.herdr("agent", "send-keys", self.pane, "ctrl+c", check=False)
            time.sleep(2)
            self.wait_idle()
        self.settled()
        return self.bindings()

    def follow_up(self):
        """The neutral next turn; a hookless recipient has no turn-start delivery, so it reads its inbox."""
        if self.plain:
            return (f"Run `{self.run.bindir / 'herdr-threads'} inbox` once and handle what it shows as your standing "
                    "instructions say.")
        return "hello"

    def side_case(self, case):
        action = case.split("/")[0]
        tag = f"{case} {self.run.run_id}"
        sides = self.open_sides(tag)
        connected = [b for b in sides if b["liveness"] == "connected"]
        checks = {"bindings_after_side": sides, "connected_same_pid":
                  len(connected) > 1 and len({b["pid"] for b in connected}) == 1}
        # The setup must have happened (a later case adds its own sides): the hookless recipient keeps every
        # side binding connected on its one pid; a hooked one has them (superseded by its lifecycle-claimed session).
        setup_ok = (checks["connected_same_pid"] and len(connected) >= 1 + SIDE_FORKS if self.plain
                    else len(sides) >= 1 + SIDE_FORKS)
        checks["side_setup_ok"] = setup_ok
        gap = SPACING_S - (time.time() - self.last_wake)
        if gap > 0:
            self.run.stopping.wait(gap)
        self.wait_idle()
        ref = {}
        if action == "side-nudge":
            expect_wake = True
            ref["message"] = self.send("p0", tag, direct=False)
            t_wake = self.watch_for_wake(WAKE_WINDOW_S)
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
            passed = (setup_ok and not woke and checks["draft_intact"] and checks["delivered"]
                      and checks["warning_wake"] not in (None, "submitted"))
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
        """modal-hold: an open picker holds the p0 wake without a sender warning; closing it wakes."""
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
        self.run.herdr("pane", "send-text", self.pane, "/model")
        time.sleep(1)
        self.run.herdr("agent", "send-keys", self.pane, "enter", check=False)
        opened = False
        for _ in range(20):  # the picker is open once its first numbered row is drawn
            if any(PICKER_ROW.match(l) for l in detection().splitlines()):
                opened = True
                break
            time.sleep(0.5)
        checks["picker_open"] = opened
        pane = self.run.coords["main"]["pane"]
        sends, mids = [], []
        try:
            if opened:
                for n in range(2):
                    out, err = self.run.ht_err("send", self.group, "--body", f"{tag} #{n}", "--nudge", pane=pane,
                                               human=True)
                    mids.append((json.loads(out).get("result") or {}).get("data"))
                    sends.append([l for l in err.splitlines() if "wake=" in l])
                self.run.stopping.wait(WAKE_BLOCK_HOLD_S)
                checks["wakes_while_open"] = wake_count(self.screen(1000)) - wakes_before
                checks["doctor_wake_open"] = self.doctor_wake()
        finally:  # never leave the picker open for a later case
            self.run.herdr("agent", "send-keys", self.pane, "esc", check=False)
        checks["send_warnings"] = sends
        t0, t_wake = time.time(), None
        while opened and time.time() - t0 < WAKE_BLOCK_WAIT_S:  # the wake is a typed wake line
            if wake_count(self.screen(1000)) > wakes_before:
                t_wake = round(time.time() - t0, 2)
                break
            time.sleep(0.5)
        woke = t_wake is not None
        if woke:
            self.last_wake = time.time()
            self.wait_idle()
        checks["delivered"] = bool(mids) and all(self.on_screen(m) for m in mids if m)
        checks["footer_after"] = footer()
        passed = (opened and checks["wakes_while_open"] == 0 and sends == [[], []] and woke
                  and checks["delivered"] and checks["footer_after"] == checks["footer_before"])
        self.settled()
        self.run.record({"case": case, "harness": self.harness, "threshold": self.threshold, "recipient": self.name,
                         "expect_wake": True, "woke": woke, "t_wake_s": t_wake, "checks": checks,
                         "ref": {"messages": mids}, "result": "PASS" if passed else "FAIL", "utc": utc()})

    # ---------- one case ----------
    def case(self, case):
        if case in WAKE_BLOCK_CASES:
            return self.wake_block_case(case)
        if case.startswith("side-"):
            return self.side_case(case)
        if case.startswith("hooks-"):
            return self.hooks_case(case)
        if case.startswith("eot-"):
            return self.eot_case(case)
        if case.startswith("busy-"):
            return self.busy_case(case)
        action, channel = case.split("/")
        if action == "mixed-order":
            return self.mixed_order_case(case)
        expect = expect_wake(action, channel, self.threshold)
        gap = SPACING_S - (time.time() - self.last_wake)
        if gap > 0:
            self.run.stopping.wait(gap)  # spacing ends early when cleanup begins
        self.wait_idle()
        body = f"{case} {self.run.run_id}"
        checks = {}
        calls_before = self.inbox_calls()
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
            ref = {"message": self.send(action.split("-")[1], body, direct=channel == "1:1")}
        t_wake = self.watch_for_wake(WAKE_WINDOW_S)
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
        # The case owns the rest of the spacing window: a turn the recipient starts on its own in here (a follow-up
        # wake for an item this case left pending) fails this case, and spacing restarts after it, so the next case
        # is never measured against the 30 s guard of a wake it did not cause.
        checks["trailing_wake_s"] = self.watch_quiet(max(0.0, SPACING_S - (time.time() - self.last_wake)))
        if checks["trailing_wake_s"] is not None:
            self.wait_idle()
            self.settled()
        verified = all(v is True or v == "acknowledged" for k, v in checks.items()
                       if k not in ("late_wake_s", "trailing_wake_s", "inbox_calls"))
        passed = woke == expect and verified and checks["trailing_wake_s"] is None
        row = {"case": case, "harness": self.harness, "threshold": self.threshold, "recipient": self.name,
               "expect_wake": expect, "woke": woke, "t_wake_s": t_wake, "hello_sent": hello,
               "checks": checks, "ref": ref, "result": "PASS" if passed else "FAIL", "utc": utc()}
        self.run.record(row)

    def hooks_case(self, case):
        """HOOKS_CASES: an idle hooks-only Claude at T=0 gets an ACK request (p0). It must wake within the window on
        the content-free native prompt, and the woken turn must show the message with its ACK settled by the
        UserPromptSubmit hook's display (`via: hook_context`): the turn made no `inbox` round trip."""
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
        passed = (woke and checks["delivered"] and checks["content_free_wake"]
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
        t_wake = self.watch_for_wake(WAKE_WINDOW_S)
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
        verified = all(v is True or v == "acknowledged" or (k == "order" and v == "unjudged")
                       for k, v in checks.items() if k in ("delivered", "order", "markers", "ack"))
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
        try:
            self.start()
            for case in cases:
                if self.run.stopping.is_set():
                    break
                try:
                    if case.split("/")[0] not in ("invite", "pair-open"):
                        self.ensure_channel(case.split("/")[1])
                    self.case(case)
                except Stopped:
                    break
                except Exception as e:  # a driver or agent error fails the case, never passes it
                    self.run.record({"case": case, "harness": self.harness, "threshold": self.threshold,
                                     "recipient": self.name, "result": "FAIL", "checks": {"error": clip(e, 400, getattr(self.run, "secrets", None))}})
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
            except Stopped:
                pass  # interrupted: no pane capture, the summary marks unfinished cases missing


def record(self, row):
    row = json.loads(redact(json.dumps(row), getattr(self, "secrets", ())))  # error tails may quote a screen
    with self.lock:
        self.results.append(row)
        print(f"[{row['result']:7}] {row['recipient']:9} {row['case']:17} T={row['threshold']!s:2} "
              f"expect={row.get('expect_wake')!s:5} woke={row.get('woke')!s:5} t={row.get('t_wake_s')} "
              f"{json.dumps(row.get('checks'))}", flush=True)


Run.record = record


def applies(recipient, case):
    """Side cases run on Codex recipients only; the plain-hooks Codex runs nothing else. Hooks cases run only on the
    hooks-only Claude, which runs nothing else. End-of-turn cases run on hooked Codex recipients; Claude cases on the
    mod Claude recipients."""
    if case in SIDE_CASES:
        return recipient.harness == "codex"
    if getattr(recipient, "hooks_only", False) or case in HOOKS_CASES:
        return getattr(recipient, "hooks_only", False) and case in HOOKS_CASES
    if case in EOT_CASES:
        return recipient.harness == "codex" and not getattr(recipient, "plain", False)
    if case in WAKE_BLOCK_CASES:
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
    if missing:
        lines.append(f"missing rows: {missing}")
    if extra:
        lines.append(f"unexpected or duplicated rows: {extra}")
    run.table = "\n".join(lines)
    run.completeness = {"expected": len(expected), "collected": len(rows), "missing": missing, "extra": extra}
    print("\n" + run.table, flush=True)
    return bool(expected) and not missing and not extra and passed == len(rows)


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
    counts["pass"] = counts["guidance_contexts"] > 0 and counts["over_allowance"] == 0
    return counts


def write_evidence(run, rc):
    """report.md / results.json, written after cleanup so they carry its outcome and the final exit code."""
    facts = dict(getattr(run, "facts", {}), finished_utc=utc(), exit_code=rc)
    if run.auth_digest:
        copy = run.codex_home / "auth.json"
        refreshed = run.auth_refreshed
        if refreshed is None and copy.exists():  # --keep: the copy was not retired
            refreshed = file_sha256(copy) != run.auth_digest
        facts["codex_auth_refreshed_in_run"] = refreshed
    with run.lock:  # workers left running after an interrupt may still append
        results = list(run.results)
    doc = {"facts": facts, "completeness": getattr(run, "completeness", None), "cleanup": run.cleanup_report,
           "codex_hook_guidance": getattr(run, "hook_guidance", None), "results": results}
    # One redaction pass over each whole document: facts (versions, flags) and cleanup diagnostics carry helper text.
    secrets = getattr(run, "secrets", None)
    secrets = secret_values(os.environ) if secrets is None else secrets
    (run.ev / "results.json").write_text(redact(json.dumps(doc, indent=2), secrets))
    (run.ev / "report.md").write_text(redact(
        f"# wake e2e {run.run_id}\n\n```json\n{json.dumps(facts, indent=2)}\n```\n\n{getattr(run, 'table', '')}\n\n"
        f"codex hook guidance: `{json.dumps(getattr(run, 'hook_guidance', None))}`\n\n"
        f"cleanup: `{json.dumps(run.cleanup_report)}`\n\nexit code: {rc}\n", secrets))


def main(argv=None):
    ap = argparse.ArgumentParser(prog="wake-e2e", description=__doc__.splitlines()[0])
    ap.add_argument("--bin", default=str(REPO / "bin" / "herdr-threads"), help="herdr-threads binary under test")
    ap.add_argument("--harness", default=DEFAULT_HARNESSES,
                    help="comma-separated: claude,codex; opt-in: agy (credential from %s)" % AGY_CREDENTIAL_ENV)
    ap.add_argument("--cases", default=",".join(CASES), help="comma-separated subset of: " + ", ".join(CASES)
                    + "; opt-in, Codex only: " + ", ".join(SIDE_CASES + EOT_CASES + WAKE_BLOCK_CASES)
                    + "; opt-in, hooks-only Claude: " + ", ".join(HOOKS_CASES))
    ap.add_argument("--evidence-dir", help="default: sibling ht-wake-evidence.<run id>")
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
    ap.add_argument("--self-test", action="store_true",
                    help="check the driver's credential, cleanup and completeness paths (no harnesses) and exit")
    args = ap.parse_args(argv)
    if args.self_test:
        return self_test()
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
    cases = [c for c in args.cases.split(",") if c]
    unknown = ([c for c in cases if c not in CASES + SIDE_CASES + HOOKS_CASES + EOT_CASES + WAKE_BLOCK_CASES]
               + [h for h in args.harness if h not in HARNESSES])
    if unknown:
        ap.error(f"unknown case/harness: {unknown}")
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
    run.hooks_cases = any(c in HOOKS_CASES for c in cases)
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
    return EXIT_OK if unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful() else EXIT_FAIL


if __name__ == "__main__":
    sys.exit(main())
