#!/usr/bin/env python3
"""Native end-to-end herdr-threads demonstration driver (ht-4is.11.3 / ht-4is.11.4 / ht-910).

One run: isolated daemon + state, owned hooks in a scratch project, a thread with a topic,
a prelaunch INVITE of a seat mapped to a scratch Herdr pane, a require-ACK handoff message,
then the real native agent (Claude or Codex) launched in that pane with a prompt that only
says to check herdr-threads mail. The MODEL must accept and ACK; the driver never does.
Verification reads SQLite read-only and joins DB provenance with the agent transcript.

--dry-run exercises every step except the model launch (it substitutes captured native hook
payloads for the SessionStart/PreToolUse hook calls) and reports exactly which step fails.

Cooperative limit (B1): under the landed cooperative caller model an ACK's DB actor, provenance and
`execution` come from the calling CLI's claim (the seat's context journal, which subagent hook events never
replace), so a child agent's `herdr-threads ack` in the same pane is indistinguishable in SQLite from the
root's. So child ACK absence is decided from transcripts, never from SQLite:
- no subagent/collab activity in the root transcript: PASS (nothing could have delegated);
- `--scenario child` with a COMPLETE sidechain record (every spawn's child calls found: Claude subagent stream events
  or transcripts, Codex child rollouts) and no successful child write: PASS at the transcript level only;
- any other subagent activity: UNVERIFIED and the manifest UNSUPPORTED (`child_ack_unverified`), never PASS.
DB-level child separation stays UNVERIFIED until the product records an identity that separates a child caller from
the root. The check "ACK DB execution = root binding execution" is reported for consistency only: under B1 a child's
ACK carries that same execution, so it does NOT discriminate a child ACK from a root ACK.

Hooks: `--setup auto` (default) uses the public user-level `herdr-threads setup claude|codex` CLI when the binary
has it (setup_mode=cli); otherwise the driver installs equivalent owned hooks itself (setup_mode=driver_fallback,
labelled as not setup-CLI evidence). Either way the "user" files are scratch copies inside the run root, never the
real ones: setup runs with HOME, CLAUDE_CONFIG_DIR and CODEX_HOME pointed at the run root.
- Claude: setup writes `<run>/claude-config/settings.json`. The launch keeps the real Claude config directory (Claude
  on macOS keys its keychain credentials by the config directory, so a scratch CLAUDE_CONFIG_DIR would break
  authentication) and adds that scratch file with `--settings` (a flag layer, loaded with `--setting-sources
  project,local`, which keeps the real user settings, and any real herdr-threads installation in them, out).
- Codex: setup writes `<run>/codex-home/{hooks.json,config.toml}` (hooks plus the sandbox socket allowance). The
  launch runs with CODEX_HOME at that scratch home, seeded only with a symlink to the source profile's `auth.json`
  (`--codex-profile`, else $CODEX_HOME or ~/.codex; the driver never reads it). Codex may refresh tokens through
  it. `--ignore-user-config` is no longer passed, since the scratch config is the run's own; hook trust is still
  bypassed (`--dangerously-bypass-hook-trust`, scratch only).

Hook-context delivery (S<n>H) is read READ-ONLY from the Claude session transcript or the Codex session rollout
(`$CODEX_HOME/sessions/**/rollout-*<thread_id>.jsonl`, developer messages); a missing rollout is UNVERIFIED and the
manifest UNSUPPORTED (`hook_context_unverified`), never PASS. S17 always surfaces what the harness reported (Codex
`turn.failed` / `error` text, token usage from `turn.completed.usage`; Claude result usage, cost and
`permission_denials`); a usage-limit or auth failure is ENVIRONMENT and the manifest UNSUPPORTED
(`environment_<kind>`). S16R (Claude) fails before launch unless the project allow rules permit every ready command
(`Bash(herdr-threads *)` from setup). `--codex-profile NAME|auto` selects an aisw Codex profile as CODEX_HOME for
the Codex launch only; `auto` moves to the next profile only on a Codex usage-limit failure (bounded, recorded).

Transport (D2/D3, ht-910): Codex's default workspace-write sandbox refuses connect() to the daemon socket.
`--codex-transport setup` (default) launches with the sandbox allowance `setup codex` writes for the driver's own
daemon (its stable socket path, checked against the published endpoint); `none` launches with
`-c sandbox_workspace_write.network_access=false`, which keeps Codex's proxy (and so the allowance) off. A run whose
model was refused the socket (the product's `transport_denied` error, or the pre-P2 `host_unavailable` text while the
driver's daemon is healthy outside the sandbox) is UNSUPPORTED `transport_denied`, never a product FAIL.

Opt-in scenarios (`--scenario child,midturn,warning,burst,required`, ht-4is.11.3 / 11.4 / 32.3 / ht-910) each add
their own verdict steps (SC*, SM*, SW*, SB*, SR*), dry-run-only product probes (SM0D, SW0D, SB0D, SR0D) and a manifest
scenario suffix. A scenario whose model-side trigger never happened is NOT_EXERCISED: the manifest is then UNSUPPORTED
`scenario_not_exercised`, never PASS and never a product FAIL. `--launch managed` starts the agent through
`herdr-threads launch` when the build offers it (else UNSUPPORTED `managed_launch_unavailable`). `--mode tui
--tui-accept-trust` is approved for /private/tmp scratch projects only and records ~/.claude.json read-only before and
after (hash, top-level key hashes, this project's entry; never the whole file).

Discipline: never writes ~/.claude, ~/.codex or aisw profiles (the harnesses write their own
session transcripts; the Codex launch home only symlinks the profile's auth.json); never restarts the shared Herdr server; creates only its own tab/panes,
records them in an owned-resource ledger and closes exactly those; stops only a daemon it
started. Every outcome is recorded as observed; nothing unrun is reported as PASS.
"""

import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import shlex
import shutil
import signal
import sqlite3
import subprocess
import sys
import time
import uuid

REPO = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPO / "tests" / "native" / "support"))
from fixture import NativeFixture  # noqa: E402

PROMPT = "Check your herdr-threads mail and follow its instructions."
# --scenario child (ht-910 delegation): the launch prompt itself asks for one reading subagent; everything else
# (accept, ACK) stays with the top-level agent. Never names a command or an ID.
CHILD_PROMPT = ("Check your herdr-threads mail by delegating the reading to exactly one subagent: it may only read "
                "with herdr-threads commands and report back, and must not accept, ACK, or check in. Then you, the "
                "top-level agent, decide and follow the mail's instructions yourself.")
# --scenario children (ht-4is.11.12 d): two concurrent reading subagents; accept and ACK stay with the top-level agent.
CHILDREN_PROMPT = ("Check your herdr-threads mail by starting exactly two subagents at the same time (concurrently, not one "
                   "after the other): each may only read with herdr-threads commands and report back, and must not accept, "
                   "ACK, send, or check in. Then you, the top-level agent, decide and follow the mail's instructions yourself.")
# --scenario blockedui (ht-4is.11.12 c): a command the harness must ask approval for; the driver NEVER answers it (the
# owned tab is closed at cleanup). {path} is inside the driver's own run root, outside the project workspace.
BLOCKEDUI_PROMPT = ("Run exactly this shell command now, and if it needs approval or escalated permissions, request "
                    "them and wait for the answer: `mkdir {path}`")
# Opt-in scenarios (ht-4is.11.3 / 11.4 / 32.3 / ht-910 / 11.12), each with its own verdict steps and manifest suffix.
SCENARIOS = ("child", "midturn", "warning", "burst", "required", "lostprompt", "blockedui", "children")
# Herdr 0.9.1 reports a TUI that finished its turn as `done`; for an idle wake target `done` is idle (ht-4is.5.6).
IDLE_STATES = ("idle", "done")
# The approval / question UIs the blockedui scenario recognizes on screen (Claude permission dialog, Codex approval).
APPROVAL_UI = re.compile(r"Do you want to (proceed|make this edit|create)|Would you like to run|Allow command|"
                         r"requires approval|Yes, proceed|approve this|Yes, and don't ask again", re.I)
# Codex interactive: its own folder-trust / hook-trust screens persist trust into CODEX_HOME (config.toml), which is
# NOT approved (the aisw profile is used read-only). The driver passes -s/-a so Codex skips the folder-trust screen and
# --dangerously-bypass-hook-trust (scratch only) for hook trust; any such screen on display is a FAIL, never an Enter.
CODEX_TRUST_UI = re.compile(r"trust (the files in )?this (folder|directory)|Do you trust|trust (these|the) hooks?|"
                            r"hooks? (are|is) not trusted|Review hooks", re.I)
CODEX_FOLDER_TRUST = re.compile(r"Trust this folder\?")
CODEX_TRUST_CONTINUE_SELECTED = re.compile(r"›\s*1\.\s*Trust and continue")
CODEX_INPUT_READY = re.compile(r"for shortcuts|context left|send\b.*newline", re.I)
CODEX_APPROVAL_POLICY = "on-request"
# lostprompt / blockedui: how long to keep observing when health already reports the safe wake unsupported.
LOSTPROMPT_UNSUPPORTED_WAIT_S = 60
# The fixed coalesced wake marker (scheduler design: one bounded marker for all threads/reasons).
WAKE_MARKER = "herdr-threads: attention pending"
# tui D1 warning wake: poll interval, and how long a woken agent gets to show the marker before the pane capture.
WARNING_POLL_S = 2
WAKE_SETTLE_S = 15
# Default inbox page size (src/protocol/pagination.rs DEFAULT_PAGE_LIMIT): a burst needs more threads than this.
INBOX_PAGE_LIMIT = 20
MAX_BURST_THREADS = 40
# User approval (run.md, 2026-09-30): the Claude TUI trust dialog may be accepted for /private/tmp scratch projects
# only; ~/.claude.json is recorded (hash + this project's entry, never copied whole) before and after.
TUI_TRUST_ROOTS = ("/private/tmp/",)
CLAUDE_JSON = Path.home() / ".claude.json"
# Verbs that read without mutating (child reads are allowed; a child ACK/accept/check-in is not).
READ_VERBS = re.compile(r"\b(pending-receipts|inbox|read|body|search|thread\s+list|warnings|overdue)\b")
# Every herdr-threads subcommand that writes (src/cli/commands.rs `Top` and its groups): a child/subagent must issue
# none of them, since the hook text says every write acts as the top-level seat. Matched on the parsed subcommand
# (`herdr_threads_verbs`), never on free text, so a `search send` or a body containing "accept" is not a write.
MUTATING_VERBS = frozenset({"ack", "accept", "accept-required", "check-in", "leave", "send", "invite", "archive",
                            "reopen", "retry", "setup", "unsetup", "launch", "thread create", "thread topic",
                            "seat resolve", "seat rebind", "daemon ensure", "daemon stop",
                            # W6-D1: disconnects the daemon's host service (commands.rs ServiceSub::Disconnect).
                            # `cached-check-in` is not here: it only reads a completed cached check-in page.
                            "service disconnect"})
# Global options that take a value (src/cli/commands.rs `Cli`), skipped when locating the subcommand.
HT_VALUE_OPTIONS = ("--state-dir", "--host-endpoint", "--cooperative-seat", "--cooperative-target",
                    "--cooperative-harness", "--cooperative-role")
HT_GROUPS = ("thread", "seat", "delivery", "daemon", "service")
# The first sentence of every herdr-threads hook additionalContext (src/harness/mod.rs TOP_LEVEL_INSTRUCTION). D7
# (Codex matrix 1): Codex also records its own `<skills_instructions>` developer message, which is not a delivery.
HOOK_PREAMBLE = "The top-level agent reads pending mail"
# The CLI clips a message preview to 256 escaped bytes (src/protocol/output.rs MAX_ESCAPED_BYTES). D1 (Claude matrix
# 1) / D5 (Codex matrix 1): a model reading only the preview must still see the scenario steps, so the handoff puts
# them first and keeps them inside this budget.
PREVIEW_BYTES = 256
# Distinctive phrases of each scenario instruction; finding one in the model's transcript (or a `body` read of the
# handoff) is the evidence that the model saw the instruction at all.
SCENARIO_MARKERS = {"midturn": "watch for new herdr-threads mail", "burst": "page through your whole inbox",
                    "required": "required invite, thread"}
# Default argv template for `--launch managed` (harness design: `launch --pane ADDRESS --kind codex|claude`).
MANAGED_LAUNCH_ARGV = "launch --pane {pane} --kind {harness} --"
# How many recent unwrapped lines a pane capture reads.
PANE_CAPTURE_LINES = 400
# A managed launch reuses one agent pane for every phase, so the owned shell prints this marker (plus run, phase and a
# nonce) right before `herdr-threads launch`; the phase's outcome is parsed only from the capture lines after it.
PHASE_MARKER = "ht-demo-phase-start"
# DB provenance classes a top-level model ACK/accept may carry. Only `verified_current_target` is native
# proof; `cooperative_top_level` is the hook-established cooperative claim (store::control::AccountableActor
# "never represents native proof"). The transcript join, not the DB class, vouches for model issuance.
MODEL_PROVENANCE = {"cooperative_top_level", "verified_current_target"}
NATIVE_PROOF = "verified_current_target"
# Steps that only make sense without a model: on a live run they are SKIPPED, never BLOCKED (D1).
DRY_ONLY = ("S15", "S16", "S16C", "S16X")
# Codex exec-stream item types that spawn or drive a child agent (collab_tool_call: spawn/wait/...).
CODEX_CHILD_ITEMS = re.compile(r"collab|subagent|spawn", re.I)
# Claude tool names that start a subagent (Task on 2.1.x, Agent on later builds).
CLAUDE_CHILD_TOOLS = {"Task", "Agent"}
# The Claude folder-trust dialog text; Enter is sent only when this is on screen (S1).
TRUST_DIALOG = re.compile(r"trust (the files in )?this folder|Do you trust", re.I)
TRUST_OPTIONS = re.compile(r"Yes, I trust this folder")
TUI_INPUT_READY = re.compile(r"for shortcuts")
TRUST_NO_SELECTED = re.compile(r"❯\s*(\d\.\s*)?No, exit")
TRUST_YES_SELECTED = re.compile(r"❯\s*(\d\.\s*)?Yes, I trust this folder")
MAX_LAUNCH_TIMEOUT_S = 600
MAX_TOTAL_LAUNCH_S = 1800
CLI_HINT = ("DIAGNOSTIC HINT: `herdr-threads` is a command-line tool on PATH; run it with the shell tool "
            "(`herdr-threads --help` lists its commands).\n")
CAPTURES = REPO / "tests" / "native" / "demo" / "payloads"
# The owned allow rule `herdr-threads setup claude` installs (run.md user decision 2026-09-30, superseding the
# export-prefix rule, which the production hook never needed): print mode denies any Bash command no allow rule
# permits, so the hook's ready commands (`herdr-threads ...`) need it for an unattended run.
HERDR_THREADS_ALLOW = "Bash(herdr-threads *)"
# The dead rule the previous setup installed; its presence alone permits no ready command.
LEGACY_EXPORT_ALLOW = "Bash(export HERDR_THREADS_CALLER_CONTEXT=*)"
READY_HEADER = "Ready commands"
# aisw Codex profiles, in the order `--codex-profile auto` tries them. Used read-only: only CODEX_HOME is set.
AISW_CODEX_PROFILES = ("codex-1", "codex-2", "codex-3", "default")
AISW_CODEX_ROOT = Path.home() / ".aisw" / "profiles" / "codex"
PROFILE_NAME = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{0,63}\Z")
# Account/environment failures reported by the harness itself: the model was never reached, so the run says
# nothing about the product (manifest UNSUPPORTED environment_<kind>, never FAIL scenario_failed).
ENV_FAILURES = (
    ("usage_limit", re.compile(r"usage limit|rate limit|quota|insufficient_quota|purchase more credits|credit balance", re.I)),
    ("auth", re.compile(r"\b401\b|unauthori[sz]ed|not logged in|log ?in again|please (re-?)?login|authenticat|"
                        r"refresh token|token (has )?expired|invalid api key|api key", re.I)),
)


def classify_environment(text):
    """`usage_limit` / `auth` when a harness error message is an account or credential failure, else None."""
    for kind, pattern in ENV_FAILURES:
        if text and pattern.search(text):
            return kind
    return None


def ready_commands(context):
    """Every `herdr-threads ...` command in the `Ready commands` block of a hook additionalContext
    (src/harness/mod.rs NextActions: `- <label>: <cmd>` lines, the continuation joining two with `; <label>: `)."""
    lines = (context or "").splitlines()
    try:
        start = next(i for i, line in enumerate(lines) if line.startswith(READY_HEADER))
    except StopIteration:
        return []
    commands = []
    for line in lines[start + 1:]:
        if not line.startswith("- "):
            break
        commands += [c.strip() for c in re.findall(r"(?:^- [^:]*: |; [^:;]*: )(herdr-threads\b[^;\n]*)", line)]
    return commands


def invokes_herdr_threads(command):
    """D7 (Claude demo 3): True when some simple command of `command` runs `herdr-threads` as its command word
    (after any `NAME=value` assignments; a path ending in `/herdr-threads` counts). A word elsewhere, such as
    `type herdr-threads` or `alias herdr-threads`, is not a herdr-threads command."""
    for segment in re.split(r"[;&|()\n]+", str(command or "")):
        try:
            words = shlex.split(segment, comments=True)
        except ValueError:
            words = segment.split()
        while words and re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*=.*", words[0], re.S):
            words.pop(0)
        if words and (words[0] == "herdr-threads" or words[0].endswith("/herdr-threads")):
            return True
    return False


def herdr_threads_verbs(command):
    """The subcommand of every `herdr-threads` invocation in a shell command (`ack`, `thread create`, ...), skipping
    global options; nested `sh -c '...'` quoting is tolerated because each occurrence is scanned on its own."""
    verbs = []
    text = str(command or "")
    # W6-D2: the name may end a command substitution or a quoted path (`"$(which herdr-threads)" ack X`); the
    # subcommand then follows those closing characters.
    for match in re.finditer(r"(?:^|(?<=[\s;&|(`'\"/]))herdr-threads(?=[\s)\"'`])", text):
        rest = re.split(r"[;&|)\n]", text[match.end():].lstrip(")\"'`"), maxsplit=1)[0]
        words = [w.strip("'\"`{}(),;\\") for w in rest.split()]
        found, skip = [], False
        for word in words:
            if skip:
                skip = False
                continue
            if word.startswith("-"):
                skip = word in HT_VALUE_OPTIONS
                continue
            if not word:
                continue
            found.append(word)
            if found[0] not in HT_GROUPS or len(found) == 2:
                break
        if found:
            verbs.append(" ".join(found))
    return verbs


def mutating_call(command):
    """True when some herdr-threads invocation in `command` runs a writing subcommand (MUTATING_VERBS)."""
    return any(verb in MUTATING_VERBS for verb in herdr_threads_verbs(command))


def agent_shim(binary, state, endpoint):
    """The agent-pane `herdr-threads` PATH shim. D8 (Claude demo 3): it pins the run's --state-dir and
    --host-endpoint only when the argv does not already carry that flag, so a rendered ready command (which names
    --state-dir itself) runs as written instead of failing `cannot be used multiple times`."""
    q = shlex.quote
    return ("#!/bin/sh\n"
            "pin_state=1 pin_host=1\n"
            'for word in "$@"; do\n'
            '  case "$word" in\n'
            "    --state-dir|--state-dir=*) pin_state=0 ;;\n"
            "    --host-endpoint|--host-endpoint=*) pin_host=0 ;;\n"
            "  esac\n"
            "done\n"
            f'if [ "$pin_host" = 1 ]; then set -- --host-endpoint {q(endpoint)} "$@"; fi\n'
            f'if [ "$pin_state" = 1 ]; then set -- --state-dir {q(str(state))} "$@"; fi\n'
            f'exec {q(binary)} "$@"\n')


def _rule_pattern(rule):
    """The inner pattern of a `Bash(...)` rule; `""` for bare `Bash` (every command); None for other tools."""
    rule = rule.strip()
    if rule == "Bash":
        return ""
    m = re.fullmatch(r"Bash\((.*)\)", rule, re.S)
    return m.group(1) if m else None


def rule_matches(rule, command):
    """Claude Code Bash permission-rule matching: `prefix:*` is a prefix match; otherwise `*` is a wildcard
    over the whole command, and a trailing ` *` also matches the bare command (word boundary)."""
    pattern = _rule_pattern(rule)
    if pattern is None:
        return False
    if pattern in ("", "*"):
        return True
    if pattern.endswith(":*"):
        return command.startswith(pattern[:-2])
    if pattern.endswith(" *") and command == pattern[:-2]:
        return True
    return re.fullmatch(".*".join(re.escape(part) for part in pattern.split("*")), command, re.S) is not None


def command_permitted(command, allow, deny=()):
    """Every simple command of `command` (split on unquoted-looking && || ; |) is allowed and none denied."""
    segments = [s for s in re.split(r"\s*(?:&&|\|\||;|\|)\s*", command.strip()) if s]
    return bool(segments) and all(any(rule_matches(r, s) for r in allow) and not any(rule_matches(r, s) for r in deny)
                                  for s in segments)
RECIPE_LINE = re.compile(r"^(?P<id>\S+)\s+(?:\[(?P<lo>\d+\.\d+\.\d+),\s*(?P<hi>\d+\.\d+\.\d+)\]|\{(?P<set>[^}]*)\})$")


def vtuple(text):
    return tuple(int(x) for x in text.split("."))


def recipe_admits(line, installed):
    """(admitted, parsed) for a doctor `hooks.<h>.recipes` line: `id [lo, hi]` closed interval or
    `id {a, b}` exact set, several joined by `; `. Any other recipe form (e.g. a schema-fingerprint
    admission) is reported unparsed: the hook itself is then the only authority."""
    admitted, parsed = False, True
    for part in [p.strip() for p in (line or "").split(";") if p.strip()]:
        m = RECIPE_LINE.match(part)
        if not m:
            parsed = False
            continue
        if installed and m.group("lo"):
            admitted |= vtuple(m.group("lo")) <= vtuple(installed) <= vtuple(m.group("hi"))
        elif installed:
            admitted |= installed in [v.strip() for v in m.group("set").split(",")]
    return admitted, parsed and bool(line)
PASS, FAIL, SKIP, BLOCKED, INFO = "PASS", "FAIL", "SKIPPED", "BLOCKED", "INFO"
# B1 (cooperative limit): a step whose claim the available evidence cannot decide either way. Never PASS;
# the manifest is then UNSUPPORTED (`child_ack_unverified`), never PASS.
UNVERIFIED = "UNVERIFIED"
# The harness failed for an account/credential reason (usage limit, auth) before reaching the model.
ENVIRONMENT = "ENVIRONMENT"
# D2 (Codex demo 2): the harness sandbox refused the CLI's connect() to the daemon socket, so the model could not
# reach the product at all. The run says nothing about the product: manifest UNSUPPORTED `transport_denied`, never FAIL.
TRANSPORT = "TRANSPORT_DENIED"
# Product error text for a sandbox-refused daemon socket (src/client/mod.rs `connect_error`).
TRANSPORT_DENIED_OUTPUT = re.compile(r"\(transport_denied\)|daemon socket not reachable from this sandbox")
# What builds before ht-910 P2 printed for the same refusal; ambiguous alone (a stopped daemon prints it too), so it
# is classified transport_denied only when the launch had no socket allowance and the driver's own daemon, checked
# from outside the sandbox after the launch, is still healthy.
LEGACY_UNAVAILABLE_OUTPUT = re.compile(r"daemon connection unavailable \(host_unavailable\)")
# An opt-in scenario whose triggering model behaviour did not happen (no delegation, the run ended before the
# mid-turn send, an ACK beat the short deadline, no leave attempt): the claim was not tested. Never PASS; the
# manifest is UNSUPPORTED `scenario_not_exercised`, never FAIL (the product was not shown wrong).
NOT_EXERCISED = "NOT_EXERCISED"
# A configuration the build under test does not offer (e.g. `--launch managed` without a `launch` command). The
# manifest is UNSUPPORTED with the step's recorded reason.
UNSUPPORTED_STEP = "UNSUPPORTED"
# Manifest reasons for UNVERIFIED evidence, most serious first (the manifest carries the first that applies).
UNVERIFIED_REASONS = ("child_ack_unverified", "warning_wake_unverified", "hook_context_unverified", "evidence_unverified")


def parse_scenarios(text):
    """`--scenario a,b` -> ordered unique names; `base`/empty -> none. Unknown names raise ValueError."""
    names = [n.strip() for n in str(text or "").split(",") if n.strip() and n.strip() != "base"]
    unknown = [n for n in names if n not in SCENARIOS]
    if unknown:
        raise ValueError(f"unknown scenario(s) {unknown}; choose from {', '.join(SCENARIOS)} (or base)")
    return list(dict.fromkeys(names))


def encode_frame(value):
    """The daemon's local IPC framing (src/daemon/transport.rs): a 4-byte big-endian length, then JSON."""
    body = json.dumps(value, separators=(",", ":")).encode()
    return len(body).to_bytes(4, "big") + body


class ServiceClient:
    """Minimal Python speaker of the D2 programmatic service wire (src/protocol/service.rs, the same frames
    `client::service::PersistentServiceClient` sends): one registered, connection-scoped session on the
    driver's own daemon socket. Used only for the `required` scenario's service-side setup (ensure a managed
    thread, a required invitation, one info event); it never accepts, ACKs or leaves for anyone."""

    def __init__(self, endpoint, instance, timeout=10.0):
        import socket
        self.instance = str(instance)
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.settimeout(timeout)
        self.sock.connect(str(endpoint))
        self.sequence = 0
        self.transcript = []

    def close(self):
        try:
            self.sock.close()
        except OSError:
            pass

    def _read_exact(self, size):
        data = b""
        while len(data) < size:
            chunk = self.sock.recv(size - len(data))
            if not chunk:
                raise ConnectionError("service connection closed by the daemon")
            data += chunk
        return data

    def call(self, service):
        self.sequence += 1
        request = {"version": 1, "request_id": f"ht-demo-{self.sequence}-{secrets.token_hex(3)}",
                   "expected_instance": self.instance, "service": service}
        self.sock.sendall(encode_frame(request))
        size = int.from_bytes(self._read_exact(4), "big")
        response = json.loads(self._read_exact(size))
        self.transcript.append({"request": request, "response": response})
        result = response.get("result") or {}
        if "Err" in result:
            raise RuntimeError(f"service {service.get('kind')}/{(service.get('args') or {}).get('kind')} refused: {result['Err']}")
        return result.get("Ok") or {}

    def register(self):
        return self.call({"kind": "register", "args": {"capability": "service_session_v1"}})

    def operation(self, kind, args):
        return self.call({"kind": "operation", "args": {"kind": kind, "args": args}})


def default_signals():
    """N1: children of the driver get default SIGINT/SIGTERM/SIGHUP even while cleanup ignores them."""
    for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
        signal.signal(sig, signal.SIG_DFL)


def utc():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def sha256(path):
    try:
        with open(path, "rb") as stream:
            return hashlib.sha256(stream.read()).hexdigest()
    except OSError as error:
        return f"unavailable:{type(error).__name__}"


def find_key(value, keys):
    """First string under any of `keys`, depth-first; tolerant of result envelopes."""
    if isinstance(value, dict):
        for key in keys:
            if isinstance(value.get(key), str) and value[key]:
                return value[key]
        for child in value.values():
            found = find_key(child, keys)
            if found:
                return found
    elif isinstance(value, list):
        for child in value:
            found = find_key(child, keys)
            if found:
                return found
    return None


def find_value(value, key):
    """First value under `key` anywhere in a JSON document (booleans and lists included)."""
    if isinstance(value, dict):
        if key in value:
            return value[key]
        for child in value.values():
            found = find_value(child, key)
            if found is not None:
                return found
    elif isinstance(value, list):
        for child in value:
            found = find_value(child, key)
            if found is not None:
                return found
    return None


class Blocked(Exception):
    pass


class Interrupted(BaseException):
    """SIGTERM/SIGHUP during a run. A BaseException so `step()` cannot turn it into a FAIL and carry on."""


class Driver:
    def __init__(self, args):
        self.args = args
        self.harness = args.harness
        self.dry = args.dry_run
        self.run_id = f"ht-demo-{self.harness}-{datetime.datetime.now():%Y%m%dT%H%M%S}-{secrets.token_hex(3)}"
        self.scenarios = parse_scenarios(getattr(args, "scenario", ""))
        self.launch_mode = getattr(args, "launch", "manual") or "manual"
        if args.run_root:
            base = Path(args.run_root)
        elif getattr(args, "tui_accept_trust", False) or (args.harness == "codex" and args.mode == "tui"):
            # the trust approval (Claude) and the hook-trust bypass (interactive Codex) cover /private/tmp scratch only
            base = Path("/private/tmp") / "ht-native-demo"
        else:
            base = Path(os.environ.get("TMPDIR", "/private/tmp")) / "ht-native-demo"
        base.mkdir(mode=0o700, parents=True, exist_ok=True)
        self.fixture = NativeFixture.create(base / self.run_id)
        self.root = self.fixture.root
        self.ev = self.fixture.evidence_dir
        self.state = Path(args.state_dir) if args.state_dir else self.root / "state"
        self.project = self.root / "project"
        self.bindir = self.root / "bin"
        self.bin = str(Path(args.bin).resolve())
        self.endpoint = args.host_endpoint or os.environ.get("HERDR_SOCKET_PATH", "")
        self.steps = []
        self.facts = {"run_id": self.run_id, "harness": self.harness, "dry_run": self.dry,
                      "mode": args.mode, "started_utc": utc(), "root": str(self.root),
                      "scenarios": list(self.scenarios), "launch": self.launch_mode}
        self.unsupported_reasons = {}
        self.cmdlog = (self.ev / "commands.jsonl").open("a", encoding="utf-8")
        self.daemon_owned = False
        self.phase_results = []
        self.cleaned = False
        self.interrupted = None

    # ---------- process helpers ----------
    def run(self, argv, *, env=None, stdin=None, timeout=60, tag=None, cwd=None):
        started = time.monotonic()
        full_env = dict(os.environ if env is None else env)
        try:
            process = subprocess.run(argv, input=b"" if stdin is None else stdin, capture_output=True, env=full_env,
                                     timeout=timeout, check=False, cwd=cwd, preexec_fn=default_signals)
            rc, out, err = process.returncode, process.stdout.decode("utf-8", "replace"), process.stderr.decode("utf-8", "replace")
        except subprocess.TimeoutExpired as error:
            rc, out, err = "timeout", (error.stdout or b"").decode("utf-8", "replace"), (error.stderr or b"").decode("utf-8", "replace")
        except OSError as error:
            rc, out, err = f"oserror:{error.errno}", "", str(error)
        record = {"utc": utc(), "tag": tag, "argv": argv, "rc": rc, "elapsed_s": round(time.monotonic() - started, 3),
                  "stdout": out[:8192], "stderr": err[:8192],
                  "env_overrides": {k: v for k, v in (env or {}).items() if k.startswith(("HERDR_PANE", "HERDR_ENV", "HERDR_SOCKET"))}}
        self.cmdlog.write(json.dumps(record, sort_keys=True) + "\n")
        self.cmdlog.flush()
        return rc, out, err

    def ht(self, *argv, tag, pane=None, coop=False, json_out=True, stdin=None, timeout=60, extra_env=None):
        env = dict(os.environ)
        env.update(extra_env or {})
        env.pop("HERDR_PANE_ID", None)  # never derive a seat from the controller's own pane
        if pane:
            env["HERDR_PANE_ID"] = pane
        prefix = [self.bin, "--state-dir", str(self.state), "--host-endpoint", self.endpoint]
        if json_out:
            prefix.append("--json")
        if coop:
            prefix += ["--cooperative-seat", self.facts["coordinator_seat"], "--cooperative-target", self.facts["coordinator_pane"],
                       "--cooperative-harness", "claude", "--cooperative-role", "top-level"]
        return self.run(prefix + list(argv), env=env, stdin=stdin, timeout=timeout, tag=tag)

    def herdr(self, *argv, tag, timeout=30):
        rc, out, err = self.run(["herdr", *argv], tag=tag, timeout=timeout)
        try:
            data = json.loads(out) if out.strip() else None
        except ValueError:
            data = None
        return rc, data, out, err

    # ---------- step runner ----------
    def step(self, sid, title, fn, needs=(), dry_only=False):
        if dry_only and not self.dry:  # D1: not applicable to a live run, so never BLOCKED by its own skipped inputs
            self.record(sid, title, SKIP, "dry-run-only step; not applicable to a live run")
            return None
        missing = [n for n in needs if self.status_of(n) != PASS]
        if missing and all(self.status_of(n) == NOT_EXERCISED for n in missing):
            self.record(sid, title, NOT_EXERCISED, f"prerequisite not exercised: {', '.join(missing)}")
            return None
        if missing:
            self.record(sid, title, BLOCKED, f"prerequisite not PASS: {', '.join(missing)}")
            return None
        try:
            status, detail, value = fn()
        except Blocked as blocked:
            status, detail, value = SKIP, str(blocked), None
        except Exception as error:  # every unexpected driver error is a FAIL, never a pass
            status, detail, value = FAIL, f"driver error {type(error).__name__}: {error}", None
        self.record(sid, title, status, detail)
        return value

    def record(self, sid, title, status, detail):
        row = {"step": sid, "title": title, "status": status, "detail": detail, "utc": utc()}
        self.steps.append(row)
        print(f"[{status:7}] {sid} {title}: {detail}", flush=True)

    def status_of(self, sid):
        for row in reversed(self.steps):
            if row["step"] == sid:
                return row["status"]
        return None

    # ---------- steps ----------
    def s_preflight(self):
        problems = []
        if os.environ.get("HERDR_ENV") != "1":
            problems.append("HERDR_ENV!=1 (driver must run inside a Herdr pane)")
        if not self.endpoint:
            problems.append("no host endpoint (HERDR_SOCKET_PATH unset)")
        if not os.access(self.bin, os.X_OK):
            problems.append(f"herdr-threads binary not executable: {self.bin}")
        for tool in ("herdr", "git", self.harness):
            if not shutil.which(tool):
                problems.append(f"{tool} not on PATH")
        versions = {}
        for label, argv in (("herdr", ["herdr", "--version"]), (self.harness, [self.harness, "--version"]),
                            ("herdr_threads", [self.bin, "--version"])):
            rc, out, err = self.run(argv, tag=f"version:{label}", timeout=15)
            versions[label] = {"rc": rc, "out": (out or err).strip()[:200]}
        versions["herdr_threads_sha256"] = sha256(self.bin)
        harness_path = shutil.which(self.harness)
        versions[f"{self.harness}_path"] = str(Path(harness_path).resolve()) if harness_path else None
        versions[f"{self.harness}_sha256"] = sha256(Path(harness_path).resolve()) if harness_path else None
        rc, out, _ = self.run(["git", "-C", str(REPO), "rev-parse", "HEAD"], tag="version:driver_repo")
        versions["driver_repo_head"] = out.strip()
        self.facts["versions"] = versions
        self.facts["host_endpoint"] = self.endpoint
        self.facts["controller_pane"] = os.environ.get("HERDR_PANE_ID")
        self.facts["workspace"] = os.environ.get("HERDR_WORKSPACE_ID")
        if versions["herdr_threads"]["rc"] != 0:  # identity still pinned by SHA-256; reported, not blocking
            self.record("S01V", "herdr-threads --version", FAIL, f"rc={versions['herdr_threads']['rc']}: {versions['herdr_threads']['out'][:160]}")
        return (FAIL if problems else PASS), ("; ".join(problems) or "environment usable"), None

    def s_version_pin(self):
        """Compare the installed harness version with the recipe registry of the build under test,
        as `herdr-threads doctor` reports it (never a hard-coded or source-scraped pin)."""
        installed = re.search(r"\d+\.\d+\.\d+", self.facts["versions"][self.harness]["out"] or "")
        installed = installed.group(0) if installed else None
        rc, out, err = self.run([self.bin, "--state-dir", str(self.state), "--host-endpoint", self.endpoint, "--json", "doctor"],
                                tag="doctor:recipes", timeout=30, cwd=str(self.project))
        (self.ev / "doctor.json").write_text(out or err)
        try:
            line = json.loads(out)["doctor"]["hooks"][self.harness]["recipes"]
        except (ValueError, KeyError, TypeError):
            line = None
        try:
            observed = json.loads(out)["doctor"]["hooks"][self.harness].get("installed")
        except (ValueError, KeyError, TypeError, AttributeError):
            observed = None
        observed = observed if isinstance(observed, dict) else {}
        admission = observed.get("admission")
        self.facts["harness_version"] = {"installed": installed, "doctor_recipes": line, "doctor_rc": rc,
                                         "doctor_admission": admission, "doctor_installed": observed or None}
        if not line:
            return FAIL, f"installed {installed}; doctor rc={rc} reported no hooks.{self.harness}.recipes line: {(err or out).strip()[:200]}", None
        # Codex schema-fingerprint admission (c277a65): an unlisted version whose embedded hook schemas match a
        # recipe is admitted as `schema-matched, live-unverified`; doctor reports that for the installed binary.
        if admission and str(admission).startswith("schema-matched") and observed.get("version") == installed:
            return PASS, (f"installed {self.harness} {installed} admitted by doctor as {admission!r} (recipe "
                          f"{observed.get('recipe')}; not listed in {line!r}); live delivery decided by this run"), None
        admitted, parsed = recipe_admits(line, installed)
        if admitted:
            return PASS, f"installed {self.harness} {installed} admitted by doctor recipes: {line}", None
        if not parsed:
            return INFO, (f"installed {self.harness} {installed}; doctor recipes {line!r} include a form the driver does not "
                          f"parse (e.g. fingerprint admission); S15/S16 hook output decides"), None
        return FAIL, (f"installed {self.harness} {installed} is outside doctor recipes {line!r}; the hook fails closed "
                      f"(exit 0, no output, no check-in reaches the model)"), None

    def s_hook_version_gate(self):
        """Side-effect-free probe of the landed hook's own version gate: an empty payload is refused
        after the installed-version observation. `installed <h> version: ...` on stderr means the
        hook would fail closed for every real event; `unsupported hook payload` means the version
        was admitted and only the (empty) payload was refused."""
        env = dict(os.environ)
        # A pane of the driver's Herdr instance (a placeholder id: the empty payload is refused before any seat
        # lookup), so the hook passes its instance gate and reaches the version observation.
        env.update({"HERDR_ENV": "1", "HERDR_PANE_ID": "w0:p0-version-probe", "HERDR_SOCKET_PATH": self.endpoint})
        rc, out, err = self.run(self.hook_argv(), env=env, stdin=b"", timeout=15, tag="hook:version-gate")
        diag = err.strip()
        self.facts["hook_version_gate"] = {"rc": rc, "stdout": out[:200], "stderr": diag[:400]}
        if rc != 0:
            return FAIL, f"hook exited rc={rc} (must always exit 0): {diag[:200]}", None
        if "unsupported hook payload" in diag and "version" not in diag.split("unsupported hook payload")[0]:
            return PASS, f"hook admits installed {self.harness} version (empty payload refused only: {diag[:120]})", None
        return FAIL, f"hook fails closed before parsing: {diag[:300]}", None

    def s_hook_quiet(self):
        """The user-level hook runs in every harness session: outside a Herdr pane, and in a pane of another Herdr
        server, it must exit 0 at once with no stdout and no stderr (no version probe, no daemon start)."""
        results = {}
        for label, extra in (("no_herdr", {}),
                             ("other_instance", {"HERDR_ENV": "1", "HERDR_PANE_ID": "w0:p0-quiet-probe",
                                                 "HERDR_SOCKET_PATH": str(self.root / "other-herdr.sock")})):
            env = {k: v for k, v in os.environ.items() if not k.startswith("HERDR_")}
            env.update(extra)
            started = time.monotonic()
            rc, out, err = self.run(self.hook_argv(), env=env, stdin=b"{}", timeout=15, tag=f"hook:quiet:{label}")
            results[label] = {"rc": rc, "stdout": out[:200], "stderr": err[:200],
                              "elapsed_s": round(time.monotonic() - started, 3)}
        self.facts["hook_quiet"] = results
        noisy = {k: v for k, v in results.items() if v["rc"] != 0 or v["stdout"] or v["stderr"]}
        if noisy:
            return FAIL, f"hook not silent outside its Herdr instance: {noisy}", None
        return PASS, "hook exits 0 silently outside Herdr and in another Herdr instance's pane: " + \
            ", ".join(f"{k} {v['elapsed_s']}s" for k, v in results.items()), None

    def s_cli_surface(self):
        surface = {}
        for name, argv in (("hook", ["hook", "--help"]), ("setup", ["setup", "--help"]),
                           ("operator_resolve", ["seat", "resolve", "--help"]), ("check_in", ["check-in", "--help"]),
                           ("launch", ["launch", "--help"])):
            rc, out, err = self.ht(*argv, tag=f"surface:{name}", json_out=False, timeout=15)
            surface[name] = {"rc": rc, "present": rc == 0, "first_line": (err or out).strip().splitlines()[:1]}
        self.facts["cli_surface"] = surface
        return INFO, "; ".join(f"{k}={'present' if v['present'] else 'absent(rc=' + str(v['rc']) + ')'}" for k, v in surface.items()), None

    def s_hook_entrypoint(self):
        s = self.facts["cli_surface"]["hook"]
        if s["present"] and "unrecognized subcommand" not in " ".join(s["first_line"]):
            return PASS, f"`hook` subcommand present: {s['first_line']}", None
        return FAIL, (f"`herdr-threads hook {self.harness}` is absent (rc={s['rc']}: {s['first_line']}); an installed hook "
                      f"command would exit nonzero on every event" + ("; exit 2 from a Claude PreToolUse hook blocks the tool call" if self.harness == "claude" else "")), None

    def s_scratch(self):
        self.state.mkdir(mode=0o700, parents=True, exist_ok=True)
        self.project.mkdir(mode=0o700)
        self.bindir.mkdir(mode=0o700)
        (self.project / "README.md").write_text(f"herdr-threads native demo scratch project {self.run_id}\n")
        for argv in (["git", "init", "-q", str(self.project)],
                     ["git", "-C", str(self.project), "-c", "user.name=ht-demo", "-c", "user.email=ht-demo@invalid", "add", "README.md"],
                     ["git", "-C", str(self.project), "-c", "user.name=ht-demo", "-c", "user.email=ht-demo@invalid", "commit", "-qm", "scratch"]):
            rc, _, err = self.run(argv, tag="scratch:git")
            if rc != 0:
                return FAIL, f"{argv[1:4]} rc={rc} {err.strip()}", None
        wrapper = self.bindir / "herdr-threads"
        wrapper.write_text(agent_shim(self.bin, self.state, self.endpoint))
        wrapper.chmod(0o700)
        self.facts["paths"] = {"state_dir": str(self.state), "project": str(self.project), "agent_cli_wrapper": str(wrapper)}
        return PASS, f"run root {self.root} (0700), state {self.state}, project {self.project}", None

    def s_daemon(self):
        rc0, _, _ = self.ht("daemon", "health", tag="daemon:health-before", timeout=20)
        rc, out, err = self.ht("daemon", "ensure", tag="daemon:ensure", timeout=60)
        if rc0 != 0:
            self.daemon_owned = True
            self.fixture.record_owned("daemon", str(self.state))
        hrc, hout, herr = self.ht("daemon", "health", tag="daemon:health", timeout=20)
        (self.ev / "daemon-health.json").write_text(hout or herr)
        self.facts["safe_prompt"] = self.parse_safe_prompt(hout)
        self.facts["daemon"] = {"ensure_rc": rc, "ensure_stderr": err.strip()[:400], "health_rc": hrc, "owned": self.daemon_owned}
        if hrc != 0:
            return FAIL, f"daemon not reachable: ensure rc={rc} ({err.strip()[:200]}), health rc={hrc} ({herr.strip()[:200]})", None
        note = "" if rc == 0 else f"; NOTE ensure exited {rc} although health is reachable ({err.strip()[:160]})"
        return PASS, f"daemon reachable (health rc=0){note}", None

    def s_panes(self):
        rc, data, out, err = self.herdr("tab", "create", "--workspace", self.facts["workspace"], "--cwd", str(self.project),
                                        "--label", self.run_id[-18:], "--no-focus", tag="herdr:tab-create")
        tab = find_key(data, ["tab_id"]) if data else None
        pane = find_key((data or {}).get("result", {}).get("root_pane", {}), ["pane_id"]) if data else None
        if rc != 0 or not tab or not pane:
            return FAIL, f"tab create rc={rc} {err.strip()[:200]}", None
        self.fixture.record_owned("pane", f"tab:{tab}")
        self.facts["tab"] = tab
        self.facts["agent_pane"] = pane
        rc, data, out, err = self.herdr("pane", "split", pane, "--direction", "right", "--cwd", str(self.project), "--no-focus",
                                        tag="herdr:pane-split")
        coord = find_key((data or {}).get("result", {}).get("pane", {}), ["pane_id"]) if data else None
        if rc != 0 or not coord:
            return FAIL, f"coordinator pane split rc={rc} {err.strip()[:200]}", None
        self.facts["coordinator_pane"] = coord
        return PASS, f"owned tab {tab}: agent pane {pane}, coordinator pane {coord}", None

    def resolve(self, pane, label):
        rc, out, err = self.ht("seat", "resolve", "--pane", pane, "--new-seat", "--operator", tag=f"seat:resolve:{label}", timeout=30)
        try:
            seat = find_key(json.loads(out), ["seat_id", "seat", "id", "data"]) if rc == 0 else None
        except ValueError:
            seat = None
        if rc != 0 or not seat:
            return FAIL, f"`seat resolve --pane {pane} --new-seat --operator` rc={rc}: {(err or out).strip()[:300]}", None
        self.facts[f"{label}_seat"] = seat
        return PASS, f"{label} seat {seat} for pane {pane}", seat

    def s_coord_checkin(self):
        event = f"demo-coordinator-{uuid.uuid4()}"
        rc, out, err = self.ht("check-in", "--lifecycle-event", event, "--native-session", f"demo-coordinator-{self.run_id}",
                               tag="coordinator:check-in", pane=self.facts["coordinator_pane"], coop=True)
        if rc != 0:
            return FAIL, f"coordinator lifecycle check-in rc={rc}: {(err or out).strip()[:300]}", None
        return PASS, f"coordinator checked in (cooperative claim, driver-operated seat; not the agent under test)", None

    def s_thread(self):
        topic = f"ht native demo {self.run_id}"
        rc, out, err = self.ht("thread", "create", "--topic", topic, "--goal", "Prove model-issued accept and ACK",
                               tag="coordinator:thread-create", pane=self.facts["coordinator_pane"], coop=True)
        thread = None
        if rc == 0:
            try:
                thread = find_key(json.loads(out), ["thread_id", "thread", "id", "data"])
            except ValueError:
                pass
        if not thread:
            return FAIL, f"thread create rc={rc}: {(err or out).strip()[:300]}", None
        self.facts["thread"], self.facts["topic"] = thread, topic
        return PASS, f"thread {thread} topic {topic!r}", None

    def s_invite(self):
        rc, out, err = self.ht("invite", self.facts["thread"], "--seat", self.facts["agent_seat"], "--deadline", str(self.args.deadline),
                               tag="coordinator:invite", pane=self.facts["coordinator_pane"], coop=True)
        if rc != 0:
            return FAIL, f"invite rc={rc}: {(err or out).strip()[:300]}", None
        try:
            self.facts["invitation"] = find_key(json.loads(out), ["invitation_id", "invitation", "id", "data"])
        except ValueError:
            pass
        return PASS, f"agent seat {self.facts['agent_seat']} invited before launch (invitation {self.facts.get('invitation')})", None

    def prompt(self):
        """The launch prompt: the fixed unhinted PROMPT, or CHILD_PROMPT under --scenario child."""
        if "children" in self.scenarios:
            return CHILDREN_PROMPT
        return CHILD_PROMPT if "child" in self.scenarios else PROMPT

    def delegates(self):
        """True under a scenario whose prompt asks for subagents (child, children): child sidechains are then read."""
        return bool({"child", "children"} & set(self.scenarios))

    def interactive(self):
        """An interactive TUI agent (Claude or Codex) that stays alive between prompts in its pane."""
        return self.args.mode == "tui"

    def warning_after_idle(self):
        """tui D1: only an interactive agent that goes idle and stays alive can be an idle wake target. There the warning
        scenario sends a second short-deadline message after the initial ACK; print/exec agents keep it on the handoff."""
        return self.interactive()

    def scenario_instructions(self, phase):
        """The opt-in scenario steps for the initial handoff (never an ID to ACK: the model still has to read the
        message ID itself). D1 (Claude matrix 1) / D5 (Codex matrix 1): they lead the body, so a model that reads only
        the clipped preview still sees them; each alone fits the preview (see `preview_prefix`)."""
        if phase != "initial":
            return ""
        extra = []
        if "midturn" in self.scenarios:
            extra.append("Watch for new herdr-threads mail while you work: ACK any new message by its exact ID too, before DONE.")
        if "burst" in self.scenarios:
            # D8 (Codex matrix 2): the handoff thread's own accept is named inside this step, so a model that reads only
            # the preview never sees "accept no other invitation" without the one invitation it must accept.
            extra.append("Page through your whole inbox (run each continuation) and report its thread count; accept this "
                         "thread's invitation (step 1) but no burst invitation.")
        required = self.facts.get("required") or {}
        if "required" in self.scenarios and required.get("thread"):
            extra.append(f"Required invite, thread {required['thread']}: accept-required it, then run "
                         f"`herdr-threads leave {required['thread']}` once; report result.")
        if not extra:
            return ""
        return "".join(f" {chr(97 + i)}) {text}" for i, text in enumerate(extra))

    @staticmethod
    def preview_prefix(body):
        """The part of `body` the CLI shows as `preview_data` (src/protocol/output.rs `escaped_snippet`: 256 escaped
        bytes, a quote/backslash/tab/newline counting 2, other control characters 6)."""
        out, used = [], 0
        for ch in body:
            width = 2 if ch in '"\\\n\r\t\b\f' else 6 if ord(ch) < 0x20 else len(ch.encode())
            if used + width > PREVIEW_BYTES:
                break
            used += width
            out.append(ch)
        return "".join(out)

    def instruction_seen(self, scenario, phase="initial"):
        """(seen, how): whether the model can have seen `scenario`'s handoff instruction: a root `body` read of the
        phase's handoff, or the instruction's marker phrase anywhere in the phase transcript (a tool result showing the
        preview, a hook context, or the model's own text). Unseen -> the scenario judge is NOT_EXERCISED, never FAIL."""
        transcript = self.phase_transcript(phase) or self.live_transcript(phase)
        message = self.facts.get("messages", {}).get(phase) or "\0"
        for _, command in self.root_tool_calls(transcript):
            if "body" in herdr_threads_verbs(command) and message in command:
                return True, f"root `body {message}` call"
        marker = SCENARIO_MARKERS.get(scenario)
        try:
            text = Path(transcript).read_text(errors="replace") if transcript else ""
        except OSError:
            text = ""
        if marker and marker.lower() in text.lower():
            return True, f"instruction marker {marker!r} in the transcript"
        return False, f"no `body {message}` call and no {marker!r} in the transcript"

    def gate_on_instruction(self, scenario, result):
        """A FAIL or NOT_EXERCISED scenario verdict whose instruction the model never saw is NOT_EXERCISED (the claim was
        not tested); a PASS stands (the model did it anyway)."""
        status, detail, value = result
        if status not in (FAIL, NOT_EXERCISED):
            return result
        seen, how = self.instruction_seen(scenario)
        if seen:
            return result
        return NOT_EXERCISED, f"the model never saw the {scenario} instruction ({how}); not exercised: {detail}", value

    def send(self, phase, deadline=None):
        if phase == "midturn":
            body = (f"Mid-turn handoff ({self.run_id}). You are the top-level agent for this seat. Read this message and ACK it "
                    f"yourself by its exact message ID with `herdr-threads ack MESSAGE_ID`.")
        elif phase in ("warning", "blockedui"):
            body = (f"{phase.capitalize()}-scenario handoff ({self.run_id}). You are the top-level agent for this seat. Read this message "
                    f"and ACK it yourself by its exact message ID with `herdr-threads ack MESSAGE_ID`.")
        else:
            instructions = self.scenario_instructions(phase)
            lead = f"Required handoff (phase {phase})."
            if instructions:
                lead += f" Full text: `herdr-threads body MESSAGE_ID`.{instructions}"
            body = (f"{lead} You are the top-level agent for this seat. "
                    f"1) If you have not joined thread {self.facts['thread']}, accept its invitation yourself: "
                    f"`herdr-threads accept {self.facts['thread']}`. "
                    f"2) List your pending receipts with `herdr-threads pending-receipts`, read this message, then ACK it by its exact "
                    f"message ID with `herdr-threads ack MESSAGE_ID`. Do these yourself; do not delegate acceptance or ACK to a subagent. "
                    f"3) Reply with the word DONE. (Run {self.run_id}.)")
            if instructions:
                self.facts.setdefault("handoff_preview", {})[phase] = {
                    "preview": self.preview_prefix(body),
                    "instructions_in_preview": instructions.strip() in self.preview_prefix(body)}
        if deadline is None:
            deadline = self.args.deadline
            # --scenario warning: a deadline the model will miss. Under a TUI the short deadline goes on a second message
            # sent once the agent is idle instead (tui D1), so the warning can owe an idle wake.
            if (phase == "initial" and "warning" in self.scenarios and not self.warning_after_idle()) or phase in ("warning", "blockedui"):
                deadline = self.args.warning_deadline
        self.facts.setdefault("message_deadlines", {})[phase] = deadline
        rc, out, err = self.ht("send", self.facts["thread"], "--body", body, "--require-ack", self.facts["agent_seat"],
                               "--deadline", str(deadline), tag=f"coordinator:send:{phase}",
                               pane=self.facts["coordinator_pane"], coop=True)
        message = None
        if rc == 0:
            try:
                message = find_key(json.loads(out), ["message_id", "message", "id", "data"])
            except ValueError:
                pass
        if not message:
            return FAIL, f"send rc={rc}: {(err or out).strip()[:300]}", None
        self.facts.setdefault("messages", {})[phase] = message
        return PASS, f"require-ACK message {message} to {self.facts['agent_seat']}", message

    # ----- hooks -----
    def hook_argv(self):
        """Exactly `cli::hook::installed_argv`: `<bin> --state-dir S --host-endpoint H hook <harness>`. The host
        endpoint names the Herdr instance; the hook is silent in panes of any other."""
        return [self.bin, "--state-dir", str(self.state), "--host-endpoint", self.endpoint, "hook", self.harness]

    # ----- scratch "user" homes -----
    def claude_config_dir(self):
        return self.root / "claude-config"

    def codex_setup_home(self):
        return self.root / "codex-home"

    def setup_env(self):
        """HOME, CLAUDE_CONFIG_DIR and CODEX_HOME for `herdr-threads setup|launch`: scratch copies in the run root."""
        home = self.root / "home"
        home.mkdir(mode=0o700, exist_ok=True)
        return {"HOME": str(home), "CLAUDE_CONFIG_DIR": str(self.claude_config_dir()),
                "CODEX_HOME": str(self.codex_setup_home())}

    def codex_source_home(self, profile_home=None):
        """Where the launch's Codex credentials live: the --codex-profile directory, else $CODEX_HOME or ~/.codex."""
        return Path(profile_home or self.facts.get("codex_profile_home") or os.environ.get("CODEX_HOME")
                    or Path.home() / ".codex").expanduser()

    def codex_launch_home(self, name=None, profile_home=None):
        """The scratch CODEX_HOME one Codex launch uses. The profile setup ran under uses the setup home itself (so a
        managed launch's inspection finds the owned installation); another (an `auto` retry) gets a copy of its owned
        hooks.json/config.toml. Each is seeded with a symlink to the source auth.json only; nothing is read from it."""
        name = name or self.facts.get("codex_profile") or "default"
        home = self.codex_setup_home()
        if name != self.facts.get("codex_setup_profile", name):
            home = self.root / f"codex-home-{name}"
            home.mkdir(mode=0o700, exist_ok=True)
            for owned in ("hooks.json", "config.toml"):
                if (self.codex_setup_home() / owned).exists() and not (home / owned).exists():
                    shutil.copy(self.codex_setup_home() / owned, home / owned)
        home.mkdir(mode=0o700, exist_ok=True)
        auth = self.codex_source_home(profile_home) / "auth.json"
        link = home / "auth.json"
        if not self.dry and not link.exists() and not link.is_symlink() and auth.exists():
            link.symlink_to(auth)
        self.facts.setdefault("codex_launch_homes", {})[name] = {"home": str(home), "auth_source": str(auth)}
        return home

    def s_hooks(self):
        mode = self.args.setup
        fallback = mode == "auto" and not self.facts["cli_surface"]["setup"]["present"]
        if mode == "auto":
            mode = "driver" if fallback else "cli"
        self.facts["setup_mode"] = "driver_fallback" if fallback else mode
        env = self.setup_env()
        self.facts["setup_env"] = env
        if self.harness == "codex":
            self.facts["codex_setup_profile"] = self.facts.get("codex_profile") or "default"
        if mode == "cli":
            # The public user-level `herdr-threads setup claude|codex`, run through the driver's --state-dir and
            # --host-endpoint with every config home in the run root, so it writes only scratch copies.
            if self.args.setup_argv:
                argv = shlex.split(self.args.setup_argv.format(harness=self.harness, project=str(self.project)))
            else:
                argv = ["setup", self.harness]
            rc, out, err = self.ht(*argv, tag="setup:cli", timeout=30, extra_env=env)
            (self.ev / "setup-cli-output.txt").write_text(out + err)
            if rc != 0:
                return FAIL, f"setup CLI {argv} rc={rc}: {(err or out).strip()[:300]}", None
            try:
                report = json.loads(out).get("setup", {})
            except (ValueError, AttributeError):
                report = {}
            self.facts["setup_report"] = report
            if self.harness == "claude":
                target = self.claude_config_dir() / "settings.json"
                try:
                    allow = json.loads(target.read_text()).get("permissions", {}).get("allow", [])
                except (OSError, ValueError):
                    allow = []
                if target.exists():
                    shutil.copy(target, self.ev / "claude-settings.json")
                    self.facts["claude_settings"] = str(target)
                if HERDR_THREADS_ALLOW not in allow:
                    legacy = " (it installed only the dead export-prefix rule)" if LEGACY_EXPORT_ALLOW in allow else ""
                    return FAIL, (f"setup CLI did not install {HERDR_THREADS_ALLOW} in {target}{legacy}; Claude print mode "
                                  f"would deny every `herdr-threads` ready command (demo-2 P6)"), None
            transport = ""
            if self.harness == "codex":
                hooks = self.codex_setup_home() / "hooks.json"
                try:
                    owned = "herdr-threads-owner" in hooks.read_text()
                except OSError:
                    owned = False
                if not owned:
                    return FAIL, f"setup CLI {argv} rc=0 but wrote no owned hooks into scratch {hooks}: {out.strip()[:300]}", None
                for name in ("hooks.json", "config.toml"):
                    if (self.codex_setup_home() / name).exists():
                        shutil.copy(self.codex_setup_home() / name, self.ev / f"codex-{name}")
                problem, transport = self.codex_transport_args(report)
                if problem:
                    return FAIL, problem, None
            return PASS, (f"setup_mode=cli: owned hooks installed by public setup CLI {argv} into scratch homes "
                          f"{env}; hook argv {report.get('hook_argv')}{transport}"), None
        command = shlex.join(self.hook_argv())
        if self.harness == "claude":
            settings = {
                "permissions": {"allow": [HERDR_THREADS_ALLOW]},
                "hooks": {
                    "SessionStart": [{"hooks": [{"type": "command", "command": command, "timeout": 10}]}],
                    "PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": command, "timeout": 10}]}],
                },
            }
            target = self.claude_config_dir() / "settings.json"
            target.parent.mkdir(mode=0o700, exist_ok=True)
            target.write_text(json.dumps(settings, indent=2) + "\n")
            shutil.copy(target, self.ev / "claude-settings.json")
            self.facts["claude_settings"] = str(target)
            detail = (f"setup_mode={self.facts['setup_mode']}: driver-installed owned hooks + allow {HERDR_THREADS_ALLOW} "
                      f"in scratch {target} (not setup-CLI evidence)")
        else:
            def group(matcher=None):
                body = {"hooks": [{"type": "command", "command": command, "timeout": 10}]}
                return {**({"matcher": matcher} if matcher else {}), **body}
            target = self.codex_setup_home() / "hooks.json"
            target.parent.mkdir(mode=0o700, exist_ok=True)
            target.write_text(json.dumps({"hooks": {"SessionStart": [group()], "SubagentStart": [group()],
                                                    "PreToolUse": [group("^Bash$")]}}, indent=2) + "\n")
            shutil.copy(target, self.ev / "codex-hooks.json")
            self.facts["codex_hook_args"] = []
            detail = (f"setup_mode={self.facts['setup_mode']}: driver-written owned hooks in scratch {target}, no sandbox "
                      f"allowance (not setup-CLI evidence)")
        return PASS, detail + f"; hook argv {self.hook_argv()}", None

    def daemon_endpoint(self):
        """The socket path the driver's own daemon published (its endpoint descriptor, read-only), or None."""
        for descriptor in sorted((self.state / "instances").glob("*/endpoint.json")):
            try:
                return json.loads(descriptor.read_text()).get("endpoint")
            except (OSError, ValueError):
                continue
        return None

    def codex_transport_args(self, report):
        """D3: `--codex-transport setup` (default) launches with the sandbox allowance `setup codex` wrote into the
        scratch config.toml, which must name the socket the driver's own daemon published; `none` turns the proxy
        off for the launch (`-c sandbox_workspace_write.network_access=false`), so the default sandbox refuses the
        socket. No hook arguments: the hooks are on disk in the scratch CODEX_HOME. Returns (problem, detail)."""
        mode = getattr(self.args, "codex_transport", "setup")
        sandbox = report.get("sandbox") if isinstance(report.get("sandbox"), dict) else {}
        self.facts["codex_transport"] = {"mode": mode, "socket_path": sandbox.get("socket_path"),
                                         "present": sandbox.get("present"), "omitted": sandbox.get("omitted"),
                                         "argv": []}
        if mode == "none":
            off = ["-c", "sandbox_workspace_write.network_access=false"]
            self.facts["codex_hook_args"] = off
            self.facts["codex_transport"]["argv"] = []
            return None, "; codex transport none: proxy off for the launch (no socket allowance)"
        self.facts["codex_hook_args"] = []
        if not sandbox.get("socket_path") or not sandbox.get("present"):
            why = sandbox.get("omitted") or "no `sandbox` allowance present in the setup report"
            return (f"--codex-transport setup: `setup codex` wrote no sandbox socket allowance ({why}); "
                    f"pass --codex-transport none to launch without one"), ""
        endpoint = self.daemon_endpoint()
        if endpoint and endpoint != sandbox["socket_path"]:
            return (f"setup codex allows socket {sandbox['socket_path']} but the driver's daemon published {endpoint}; "
                    f"the allowance would not reach it"), ""
        self.facts["codex_transport"]["daemon_endpoint"] = endpoint
        self.facts["codex_transport"]["argv"] = ["config.toml"] + list(sandbox.get("keys") or [])
        return None, f"; codex transport setup: scratch config.toml allows only {sandbox['socket_path']}"

    def s_cli_hint(self):
        """--cli-hint (DIAGNOSTIC ONLY): a one-line project instruction naming the CLI. It changes what the
        model is told, so a hinted run is never acceptance evidence: the manifest is UNSUPPORTED."""
        name = "CLAUDE.md" if self.harness == "claude" else "AGENTS.md"
        target = self.project / name
        target.write_text(CLI_HINT)
        shutil.copy(target, self.ev / f"diagnostic-cli-hint-{name}")
        self.facts["diagnostic_cli_hint"] = {"file": str(target), "text": CLI_HINT.strip()}
        return INFO, f"DIAGNOSTIC (not acceptance): wrote CLI hint to scratch {target}; manifest cannot PASS", None

    def s_codex_profile(self):
        """--codex-profile NAME|auto: resolve the aisw Codex profile directory whose path becomes CODEX_HOME for the
        Codex launch only. Read-only: only the directory's existence is checked (config/auth are never opened or
        written; the scratch CODEX_HOME symlinks its auth.json); the launch line still runs `aisw workspace check
        --tool codex` first."""
        if self.harness != "codex":
            return FAIL, "--codex-profile applies to --harness codex only", None
        root = Path(self.args.aisw_codex_root).expanduser()
        wanted = AISW_CODEX_PROFILES if self.args.codex_profile == "auto" else (self.args.codex_profile,)
        candidates = [(name, str(root / name)) for name in wanted if (root / name).is_dir()]
        missing = [name for name in wanted if not (root / name).is_dir()]
        self.facts["codex_profile_request"] = {"requested": self.args.codex_profile, "root": str(root), "missing": missing}
        if not candidates:
            return FAIL, f"no aisw Codex profile directory for {list(wanted)} under {root}", None
        if self.args.codex_profile == "auto":
            self.facts["codex_profile_candidates"] = candidates
            self.facts["codex_profile"], self.facts["codex_profile_home"] = candidates[0]
            return PASS, (f"auto: candidates {[n for n, _ in candidates]} (missing {missing}); first launch on {candidates[0][0]}, "
                          f"next candidate only on a Codex usage-limit failure"), None
        self.facts["codex_profile"], self.facts["codex_profile_home"] = candidates[0]
        self.facts["codex_profile_pinned"] = candidates[0][0]
        return PASS, (f"credentials of {candidates[0][1]} for the Codex launch only (its auth.json symlinked into the "
                      f"scratch CODEX_HOME; the profile is never written)"), None

    def claude_permission_rules(self):
        """(allow, deny) from the settings the launch loads: the scratch project's (`--setting-sources project,local`)
        and the scratch user settings setup wrote, which the launch adds with `--settings`."""
        allow, deny = [], []
        files = [self.project / ".claude" / name for name in ("settings.json", "settings.local.json")]
        if self.facts.get("claude_settings"):
            files.append(Path(self.facts["claude_settings"]))
        for path in files:
            try:
                permissions = json.loads(path.read_text()).get("permissions", {})
            except (OSError, ValueError, AttributeError):
                continue
            allow += [r for r in permissions.get("allow", []) if isinstance(r, str)]
            deny += [r for r in permissions.get("deny", []) if isinstance(r, str)]
        return allow, deny

    def representative_ready_commands(self):
        """Live runs have no probe context (a synthetic hook would register a fake execution), so check the exact
        command forms the hook renders (src/harness/mod.rs next_actions: `herdr-threads --state-dir S <verb> ...`)
        with this run's IDs."""
        prefix = f"herdr-threads --state-dir {shlex.quote(str(self.state))}"
        thread = self.facts.get("thread", "thread-x")
        message = self.facts.get("messages", {}).get("initial", "msg-x")
        seat = self.facts.get("agent_seat", "seat-x")
        verbs = [f"accept {thread}", f"accept-required {thread} --invitation invitation-x --requirement requirement-x --revision 1",
                 f"read {thread} --recent 20", f"ack {message}", "inbox", "pending-receipts", f"thread list --seat {seat}"]
        return [f"{prefix} {verb}" for verb in verbs]

    def s_ready_permitted(self):
        """D6 (demo 2 P6): Claude print mode denies any Bash command no project allow rule permits, and the model then
        stops. Every ready command the SessionStart context names must be permitted before a model is launched.
        Dry run: the commands in the SessionStart probe's actual context. Live: the rendered command forms."""
        if self.harness != "claude":
            if self.args.mode == "tui":
                return INFO, (f"interactive Codex has no Claude-style allow rules; ready commands run inside the "
                              f"{self.args.codex_sandbox} sandbox (socket allowance from setup), approval policy "
                              f"{CODEX_APPROVAL_POLICY}; not applicable"), None
            return INFO, "codex exec has no Claude-style allow rules (approval policy never under exec); not applicable", None
        context = self.facts.get("probe_context", {}).get("SessionStart")
        commands = ready_commands(context) if context else []
        source = "SessionStart probe context" if commands else "rendered ready-command forms"
        if not commands:
            commands = self.representative_ready_commands()
        allow, deny = self.claude_permission_rules()
        denied = [c for c in commands if not command_permitted(c, allow, deny)]
        self.facts["ready_commands_check"] = {"source": source, "commands": commands, "allow": allow, "deny": deny, "unpermitted": denied}
        if denied:
            return FAIL, (f"{len(denied)}/{len(commands)} ready commands ({source}) are not permitted by the project allow rules "
                          f"{allow}; print mode would deny them and the model would stop (demo-2 P6). First: {denied[0][:200]}"), None
        return PASS, f"all {len(commands)} ready commands ({source}) permitted by {allow}", None

    def synth_payload(self, event, session):
        """Captured native payload shapes (claude 2.1.286 / codex 0.158.0) rebound to this run."""
        fixtures = REPO / "tests" / "fixtures"
        path = {("claude", "SessionStart"): fixtures / "claude-2.1.286" / "01-sessionstart-startup.json",
                ("claude", "PreToolUse"): fixtures / "claude-2.1.286" / "02-pretooluse-bash-root.json",
                ("codex", "SessionStart"): CAPTURES / "codex-sessionstart-startup.json",
                ("codex", "PreToolUse"): CAPTURES / "codex-pretooluse-bash-root.json"}[(self.harness, event)]
        data = json.loads(path.read_text())
        payload = dict(data.get("payload", data))
        payload.update({"session_id": session, "cwd": str(self.project)})
        if "transcript_path" in payload and payload["transcript_path"]:
            payload["transcript_path"] = str(self.root / "synthetic-transcript.jsonl")
        if event == "PreToolUse":
            payload["tool_input"] = {**payload.get("tool_input", {}), "command": "herdr-threads inbox"}
        return payload

    def s_hook_probe(self, event):
        if not self.dry:
            raise Blocked("live run: a synthetic hook would register a fake execution before the real agent; only the real harness fires hooks")
        session = str(uuid.uuid4()) if event == "SessionStart" else self.facts.get("synthetic_session", str(uuid.uuid4()))
        self.facts["synthetic_session"] = session
        payload = self.synth_payload(event, session)
        env = dict(os.environ)
        env.update({"HERDR_ENV": "1", "HERDR_PANE_ID": self.facts["agent_pane"], "HERDR_SOCKET_PATH": self.endpoint})
        rc, out, err = self.run(["/bin/sh", "-c", shlex.join(self.hook_argv())], env=env, stdin=json.dumps(payload).encode(),
                                timeout=15, tag=f"hook-probe:{event}")
        (self.ev / f"hook-probe-{event}.json").write_text(json.dumps({"payload": payload, "rc": rc, "stdout": out, "stderr": err}, indent=2))
        if rc != 0:
            blocks = " (exit 2 from Claude PreToolUse BLOCKS the tool call)" if self.harness == "claude" and event == "PreToolUse" and rc == 2 else ""
            return FAIL, f"hook rc={rc}{blocks}: {err.strip()[:300]}", None
        if not out.strip() and err.strip():  # the landed hook fails open: exit 0, empty stdout, stderr diagnostic
            return FAIL, f"hook rc=0 but quiet (failed open): {err.strip()[:300]}", None
        try:
            context = json.loads(out)["hookSpecificOutput"]["additionalContext"] if out.strip() else ""
        except (ValueError, KeyError, TypeError):
            return FAIL, f"hook rc=0 but stdout is not hookSpecificOutput JSON: {out[:200]!r}", None
        self.facts.setdefault("probe_context", {})[event] = context
        if event == "PreToolUse" and "updatedInput" in json.loads(out or "{}").get("hookSpecificOutput", {}):
            self.facts["pretooluse_rewrite"] = json.loads(out)["hookSpecificOutput"]["updatedInput"].get("command", "")[:200]
        if event == "SessionStart" and self.facts.get("thread") and self.facts["thread"] not in context and self.facts.get("topic", "") not in context:
            return FAIL, f"SessionStart hook rc=0 but additionalContext does not expose thread {self.facts['thread']}: {context[:200]!r}", None
        return PASS, f"hook rc=0, additionalContext {len(context)} bytes", None

    # ----- launch -----
    def launch_command(self, phase):
        tag = f"{phase}-{secrets.token_hex(2)}"
        rcfile = self.ev / f"agent-{tag}.rc"
        env_prefix = f"PATH={shlex.quote(str(self.bindir))}:\"$PATH\" HERDR_AGENT={self.harness}"
        unset = "unset CLAUDECODE CLAUDE_CODE_ENTRYPOINT CODEX_HOME_OVERRIDE; "
        if self.harness == "claude":
            session = self.facts.get("claude_session")
            argv = ["command", "claude", "--model", self.args.claude_model, "--setting-sources", "project,local",
                    "--permission-mode", self.args.claude_permission_mode]
            if self.facts.get("claude_settings"):  # the scratch user-level installation, as a flag settings layer
                argv += ["--settings", self.facts["claude_settings"]]
            if phase in ("initial", "restart"):
                session = str(uuid.uuid4())
                argv += ["--session-id", session]
            elif phase == "resume":
                argv += ["--resume", session]
            self.facts["claude_session"] = session
            # /clear starts a new conversation with a session id the driver does not choose; it is discovered from
            # the transcript directory after the phase (claude_transcript_since), never assumed.
            self.facts.setdefault("sessions", {})[phase] = None if phase == "clear" else session
            if self.args.mode == "print":
                out = self.ev / f"claude-{tag}.stream.jsonl"
                argv += ["-p", "--max-budget-usd", str(self.args.claude_budget_usd), "--output-format", "stream-json", "--verbose", self.prompt()]
                self.facts.setdefault("native_argv", {})[phase] = argv[2:]
                shell = (f"{unset}command aisw workspace check --tool claude && {env_prefix} {shlex.join(argv)} "
                         f"</dev/null >{shlex.quote(str(out))} 2>{shlex.quote(str(out.with_suffix('.stderr')))}; "
                         f"echo $? >{shlex.quote(str(rcfile))}")
                return shell, rcfile, out
            self.facts.setdefault("native_argv", {})[phase] = argv[2:]
            shell = f"{unset}command aisw workspace check --tool claude && {env_prefix} {shlex.join(argv)}; echo $? >{shlex.quote(str(rcfile))}"
            return shell, rcfile, None
        out = self.ev / f"codex-{tag}.events.jsonl"
        # The scratch CODEX_HOME holding setup's hooks.json/config.toml and only a symlink to the source auth.json.
        self.facts["codex_home"] = str(self.codex_launch_home())
        env_prefix += f" CODEX_HOME={shlex.quote(str(self.facts['codex_home']))}"
        if self.args.mode == "tui":
            return self.codex_tui_command(phase, unset, env_prefix, rcfile)
        common = ["--dangerously-bypass-hook-trust", "--json", "--skip-git-repo-check",
                  "-m", self.args.codex_model, "-c", f'model_reasoning_effort="{self.args.codex_effort}"',
                  *self.facts.get("codex_hook_args", []), *sum((["-c", c] for c in self.args.codex_config), [])]
        argv = ["command", "codex", "--no-daemon", "exec", "-s", self.args.codex_sandbox, "-C", str(self.project)]
        if phase == "resume":
            argv += ["resume", *common, self.facts.get("codex_session", "<thread_id-from-initial-run>"), self.prompt()]
        else:
            argv += [*common, self.prompt()]
        self.facts.setdefault("native_argv", {})[phase] = argv[2:]
        shell = (f"{unset}command aisw workspace check --tool codex && {env_prefix} {shlex.join(argv)} "
                 f"</dev/null >{shlex.quote(str(out))} 2>{shlex.quote(str(out.with_suffix('.stderr')))}; echo $? >{shlex.quote(str(rcfile))}")
        return shell, rcfile, out

    def codex_tui_command(self, phase, unset, env_prefix, rcfile):
        """Interactive Codex (ht-4is.11.12 a): the same model and scratch CODEX_HOME (setup's hooks and socket allowance),
        with no prompt argument (the driver submits prompts through Herdr, as for the Claude TUI). Explicit -s and
        -a make Codex skip its folder-trust screen (which would persist trust into CODEX_HOME); the hook-trust bypass
        is limited to the /private/tmp scratch project (validate_args); no approval or sandbox bypass. The update check
        is off so no "Update available" screen (Enter = update the aisw-managed install) is ever shown. Interactive
        Codex has no --ignore-user-config / --skip-git-repo-check / --json (exec-only flags)."""
        common = ["-s", self.args.codex_sandbox, "-a", CODEX_APPROVAL_POLICY, "-C", str(self.project),
                  "--dangerously-bypass-hook-trust", "-c", "check_for_update_on_startup=false", "-m", self.args.codex_model,
                  "-c", f'model_reasoning_effort="{self.args.codex_effort}"',
                  *self.facts.get("codex_hook_args", []), *sum((["-c", c] for c in self.args.codex_config), [])]
        argv = ["command", "codex", "--no-daemon"]
        if phase == "resume":
            argv += ["resume", *common, self.facts.get("codex_session", "<thread_id-from-initial-run>")]
        else:
            argv += common
        self.facts.setdefault("native_argv", {})[phase] = argv[2:]
        # /new starts a new conversation whose thread id the driver does not choose: discovered from the rollouts.
        if phase in ("clear", "restart"):
            self.facts.setdefault("sessions", {})[phase] = None
        elif phase == "resume":  # the resumed thread keeps its id and rollout
            self.facts.setdefault("sessions", {})[phase] = self.facts.get("codex_session")
        shell = f"{unset}command aisw workspace check --tool codex && {env_prefix} {shlex.join(argv)}; echo $? >{shlex.quote(str(rcfile))}"
        return shell, rcfile, None

    def s_tui_clear(self):
        """Start the new conversation (Claude /clear, Codex /new) before the clear-phase handoff is sent."""
        pane = self.facts["agent_pane"]
        self.facts.setdefault("phase_started_utc", {})["clear"] = utc()
        rc, _, _, err = self.herdr("agent", "prompt", pane, "/new" if self.harness == "codex" else "/clear", "--wait",
                                   "--timeout", "30000", tag="tui:clear", timeout=40)
        if rc != 0 and "agent_prompt_stalled" not in (err or ""):  # a slash command may not start a turn
            return FAIL, f"clear command rc={rc}: {(err or '').strip()[:200]}", None
        self.facts["tui_cleared"] = True
        return PASS, "new conversation started before the handoff", None

    def s_launch(self, phase):
        shell, rcfile, transcript = self.launch_command(phase)
        (self.ev / f"launch-{phase}.sh").write_text(shell + "\n")
        managed = self.launch_mode == "managed"
        if managed:
            argv = self.managed_launch_argv(phase)
            (self.ev / f"launch-managed-{phase}.json").write_text(json.dumps({"argv": argv, "path_export": self.managed_path_export()}, indent=2))
        if self.dry:
            wanted = ("S02H", "S02Q", "S04", "S13", "S14", "S15", "S16", "S16C", "S16P") + (("S16R",) if self.harness == "claude" else ()) + (
                ("S16A",) if self.args.codex_profile else ()) + (("S17M",) if managed else ())
            unmet = [s for s in wanted if self.status_of(s) != PASS]
            would = f"; a live run would be BLOCKED by {unmet}" if unmet else "; all live prerequisites PASS"
            raise Blocked(f"dry-run: model launch not performed; composed command in evidence/launch-{phase}.sh{would}")
        self.facts.setdefault("phase_started_utc", {}).setdefault(phase, utc())  # clear: set by s_tui_clear
        self.facts.setdefault("bindings_before", {})[phase] = self.query(
            "SELECT COALESCE(MAX(ordinal), 0) AS n FROM occupant_bindings WHERE seat_id=?", (self.facts["agent_seat"],))[0]["n"]
        if phase == "initial" and "lostprompt" in self.scenarios:
            # SL1 baseline before the agent starts: its SessionStart check-in binds the seat and the product may reserve
            # the idle recovery wake while the driver is still waiting for the input box.
            self.facts["lostprompt_wake_before"] = self.wake_row()
        try:
            if managed:
                return self.launch_managed(phase)
            if self.args.mode == "tui":
                return self.launch_tui(phase, shell, rcfile)
            if self.harness == "codex" and self.facts.get("codex_profile_candidates") and not self.facts.get("codex_profile_pinned"):
                return self.launch_codex_auto(phase)
            return self.judge_launch(phase, *self.launch_print(phase, shell, rcfile, transcript))
        finally:
            self.facts.setdefault("phase_ended_utc", {})[phase] = utc()

    def launch_codex_auto(self, phase):
        """`--codex-profile auto`: launch on the first candidate profile; when Codex itself reports a usage-limit
        failure (the model never ran), relaunch on the next candidate. Bounded by the candidate list (at most
        len(AISW_CODEX_PROFILES) launches, each bounded by --timeout); every attempt is recorded. The first profile
        that is not usage-limited is pinned for every later phase (a resume must use the same CODEX_HOME)."""
        candidates = list(self.facts["codex_profile_candidates"])
        attempts = self.facts.setdefault("codex_profile_attempts", [])
        result = (FAIL, "no aisw Codex profile candidate", None)
        for index, (name, home) in enumerate(candidates):
            self.facts["codex_profile"], self.facts["codex_profile_home"] = name, home
            shell, rcfile, transcript = self.launch_command(phase)
            (self.ev / f"launch-{phase}-{name}.sh").write_text(shell + "\n")
            result = self.judge_launch(phase, *self.launch_print(phase, shell, rcfile, transcript))
            outcome = self.facts.get("launch_outcome", {}).get(phase, {})
            attempts.append({"phase": phase, "profile": name, "status": result[0], "environment": outcome.get("environment"),
                             "failures": outcome.get("failures", [])[:3], "utc": utc()})
            # Retry only a launch that actually failed on a usage limit (judge_launch turns such a FAIL into
            # ENVIRONMENT); a transient usage-looking `error` event inside a successful run never relaunches.
            if not (result[0] == ENVIRONMENT and outcome.get("environment") == "usage_limit") or index == len(candidates) - 1:
                break
            print(f"[INFO   ] S17 codex profile {name} is usage-limited; retrying on {candidates[index + 1][0]}", flush=True)
        self.facts["codex_profile_pinned"] = self.facts["codex_profile"]
        status, detail, transcript = result
        tried = ", ".join(f"{a['profile']}={a['environment'] or a['status']}" for a in attempts if a["phase"] == phase)
        return status, f"{detail}; aisw profile attempts [{tried}]", transcript

    def transcript_outcome(self, transcript, lenient=False):
        """D4/D5/D6: what the harness itself reported. Codex exec --json: thread id, `turn.failed.error.message` and
        top-level `error` events, token usage summed over `turn.completed.usage`. Claude stream-json: the `result`
        event's usage, cost, `is_error` text and `permission_denials`."""
        outcome = {"failures": [], "usage": {}, "permission_denials": [], "thread_id": None, "cost_usd": None, "environment": None,
                   "transport_denied": None, "transport_suspect": None}
        if not transcript or not Path(transcript).exists():
            return outcome
        for line in Path(transcript).read_text(errors="replace").splitlines():
            try:
                if lenient:  # a pane capture: a JSON event may sit between a prompt and other text on its line
                    event = json.JSONDecoder().raw_decode(line[line.index('{"'):])[0] if '{"' in line else None
                else:
                    event = json.loads(line)
            except ValueError:
                continue
            if not isinstance(event, dict):
                continue
            kind = event.get("type")
            if self.harness == "codex":
                if kind == "thread.started" and event.get("thread_id"):
                    outcome["thread_id"] = event["thread_id"]
                elif kind == "turn.failed":
                    error = event.get("error")
                    message = error.get("message") if isinstance(error, dict) else error
                    outcome["failures"].append(f"turn.failed: {message}")
                elif kind == "error" and event.get("message"):
                    outcome["failures"].append(f"error: {event['message']}")
                elif kind == "item.completed" and isinstance(event.get("item"), dict) and event["item"].get("type") == "command_execution":
                    text = str(event["item"].get("aggregated_output") or "")
                    line = next((l.strip() for l in text.splitlines() if TRANSPORT_DENIED_OUTPUT.search(l)), None)
                    if line and not outcome["transport_denied"]:
                        outcome["transport_denied"] = {"kind": "transport_denied_error", "evidence": line[:300],
                                                       "command": str(event["item"].get("command"))[:200]}
                    line = next((l.strip() for l in text.splitlines() if LEGACY_UNAVAILABLE_OUTPUT.search(l)), None)
                    if line and not outcome["transport_suspect"]:
                        outcome["transport_suspect"] = {"evidence": line[:300], "command": str(event["item"].get("command"))[:200]}
                elif kind == "turn.completed" and isinstance(event.get("usage"), dict):
                    for key, value in event["usage"].items():
                        if isinstance(value, (int, float)):
                            outcome["usage"][key] = outcome["usage"].get(key, 0) + value
            elif kind == "result":
                if isinstance(event.get("usage"), dict):
                    outcome["usage"] = {k: v for k, v in event["usage"].items() if isinstance(v, (int, float))}
                outcome["cost_usd"] = event.get("total_cost_usd")
                if event.get("is_error"):
                    outcome["failures"].append(f"result error ({event.get('subtype')}): {event.get('result')}")
                for denial in event.get("permission_denials") or []:
                    if isinstance(denial, dict):
                        command = (denial.get("tool_input") or {}).get("command") if isinstance(denial.get("tool_input"), dict) else None
                        outcome["permission_denials"].append({"tool": denial.get("tool_name"), "tool_use_id": denial.get("tool_use_id"),
                                                              "command": command})
        outcome["environment"] = next((k for k in map(classify_environment, outcome["failures"]) if k), None)
        return outcome

    def judge_launch(self, phase, status, detail, transcript, outcome=None):
        """Fold the harness-reported outcome into S17: failure text, token usage and permission denials are always
        surfaced; an account/credential failure is ENVIRONMENT, not a product FAIL. A managed launch passes the
        outcome it recovered from the pane capture (`managed_outcome`)."""
        outcome = self.transcript_outcome(transcript) if outcome is None else outcome
        if outcome["thread_id"]:
            self.facts["codex_session"] = outcome["thread_id"]
            self.facts.setdefault("sessions", {})[phase] = outcome["thread_id"]
        self.facts.setdefault("launch_outcome", {})[phase] = outcome
        if outcome["usage"] or outcome["cost_usd"] is not None:
            self.facts.setdefault("usage", {})[phase] = {"tokens": outcome["usage"], "cost_usd": outcome["cost_usd"],
                                                         "codex_profile": self.facts.get("codex_profile")}
        extra = []
        if outcome["failures"]:
            extra.append("harness reported: " + " | ".join(f[:300] for f in outcome["failures"][:3]))
        if outcome["permission_denials"]:
            extra.append(f"{len(outcome['permission_denials'])} permission denial(s): "
                         + "; ".join(str(d.get("command") or d.get("tool"))[:160] for d in outcome["permission_denials"][:3]))
        extra.append(f"tokens {outcome['usage'] or 'none reported'}" + (f", ${outcome['cost_usd']}" if outcome["cost_usd"] is not None else ""))
        if status == FAIL and outcome["environment"]:
            status = ENVIRONMENT
            extra.insert(0, f"ENVIRONMENT ({outcome['environment']}): the harness failed before reaching the model; not a product result")
        denied = self.classify_transport(outcome)
        if denied:
            extra.insert(0, f"transport_denied ({denied['kind']}): the sandbox refused the daemon socket: {denied['evidence'][:200]}")
            if status == FAIL:
                status = TRANSPORT
        return status, detail + "; " + "; ".join(extra), transcript

    def transport_allowance(self):
        """True when the launch carried a sandbox socket allowance (setup's scratch config.toml, or a manual
        --codex-config)."""
        transport = self.facts.get("codex_transport") or {}
        manual = any("network_proxy.unix_sockets" in c for c in getattr(self.args, "codex_config", []) or [])
        return bool(transport.get("mode") == "setup" and transport.get("argv")) or manual

    def classify_transport(self, outcome):
        """D2: a sandbox transport denial. The product's own `transport_denied` error is definitive. The legacy
        `host_unavailable` text is ambiguous, so it counts only when the launch ran in a Codex sandbox without a
        socket allowance and the driver's daemon, probed from outside the sandbox now, is healthy."""
        if self.harness != "codex":
            return None
        if not outcome.get("transport_denied") and outcome.get("transport_suspect"):
            sandboxed = getattr(self.args, "codex_sandbox", "workspace-write") != "danger-full-access"
            if sandboxed and not self.transport_allowance():
                rc, _, _ = self.ht("daemon", "health", tag="daemon:health-transport-check", timeout=20)
                if rc == 0:
                    outcome["transport_denied"] = {"kind": "inferred_host_unavailable_while_daemon_healthy",
                                                   **outcome["transport_suspect"]}
        return outcome.get("transport_denied")

    def launch_print(self, phase, shell, rcfile, transcript):
        rc, _, _, err = self.herdr("pane", "run", self.facts["agent_pane"], shell, tag=f"herdr:pane-run:{phase}")
        if rc != 0:
            return FAIL, f"pane run rc={rc}: {err.strip()[:200]}", None
        deadline = time.monotonic() + self.args.timeout
        while time.monotonic() < deadline and not rcfile.exists():
            self.launch_tick(phase, transcript)
            time.sleep(1)
        self.capture_pane(phase)
        if not rcfile.exists():
            self.herdr("pane", "send-keys", self.facts["agent_pane"], "ctrl+c", tag=f"herdr:interrupt:{phase}")
            return FAIL, f"agent did not exit within {self.args.timeout}s (interrupted with ctrl+c)", transcript
        code = rcfile.read_text().strip()
        return (PASS if code == "0" else FAIL), f"agent process exited {code}; transcript {transcript}", transcript

    def launch_tui(self, phase, shell, rcfile):
        """Interactive agent in the owned pane (Claude TUI, or Codex TUI: ht-4is.11.12 a). Nothing is ever typed into
        a dialog except the user-approved Claude folder-trust dialog of a /private/tmp scratch project; any other
        blocking screen (Codex trust or hook trust, approval, theme, ...) is a FAIL with nothing typed."""
        pane = self.facts["agent_pane"]
        codex = self.harness == "codex"
        if phase in ("initial", "restart", "resume"):
            if phase != "initial":
                self.herdr("agent", "prompt", pane, "/quit" if codex else "/exit", "--wait", "--timeout", "30000",
                           tag=f"tui:exit:{phase}", timeout=40)
                t_end = time.monotonic() + 30
                while time.monotonic() < t_end and not any(self.ev.glob("agent-*.rc")):
                    time.sleep(1)
            rc, _, _, err = self.herdr("pane", "run", pane, shell, tag=f"tui:run:{phase}")
            if rc != 0:
                return FAIL, f"pane run rc={rc}: {err.strip()[:200]}", None
            # Herdr only knows the pane as an agent once it has detected the started TUI; `agent wait` fails at once
            # with agent_not_found before that, so retry on that error only, within the same 60 s budget. Herdr 0.9.1
            # reports a finished turn as `done`, which is idle for this purpose.
            t_ready = time.monotonic() + 60
            while True:
                rc, data, out, err = self.herdr("agent", "wait", pane, "--until", "idle", "--until", "done", "--until", "blocked",
                                                "--timeout", "60000", tag=f"tui:wait-ready:{phase}", timeout=70)
                if rc == 0 or "agent_not_found" not in (err or "") or time.monotonic() >= t_ready:
                    break
                time.sleep(1)
            state = find_key(data, ["status", "state"]) if data else None
            blocked = state == "blocked" or "blocked" in (out + err)
            # S1: whatever Herdr reports (blocked or idle), look at the screen before typing anything.
            screen = self.capture_pane(phase + "-ready")
            if codex and CODEX_FOLDER_TRUST.search(screen) and self.args.tui_accept_trust:
                # User-approved (2026-09-30): Codex's folder-trust screen may be accepted for /private/tmp scratch
                # projects only (parse() refuses --tui-accept-trust elsewhere). It saves a trust_level entry into the
                # profile's config.toml, as exec runs already do. Hook-trust screens stay a FAIL.
                if not CODEX_TRUST_CONTINUE_SELECTED.search(screen):
                    return FAIL, f"Codex folder-trust screen without 'Trust and continue' selected; nothing typed (pane-{phase}-ready.txt)", None
                self.herdr("agent", "send-keys", pane, "enter", tag=f"tui:accept-codex-trust:{phase}")
                time.sleep(1)
                screen = self.capture_pane(phase + "-codex-trusted")
                if CODEX_FOLDER_TRUST.search(screen):
                    return FAIL, "Codex folder-trust screen still up after Enter; nothing else typed", None
                blocked = False
            if codex:
                if CODEX_TRUST_UI.search(screen) or TRUST_DIALOG.search(screen):
                    return FAIL, ("Codex TUI shows a trust screen; answering it would persist trust into CODEX_HOME "
                                  f"(not approved; the driver passes -s/-a and --dangerously-bypass-hook-trust to avoid it). "
                                  f"Nothing typed (see pane-{phase}-ready.txt)"), None
                if blocked:
                    return FAIL, f"Codex TUI blocked at a dialog (see pane-{phase}-ready.txt); nothing typed", None
            elif TRUST_DIALOG.search(screen):
                failed = self.accept_claude_trust(pane, phase, screen)
                if failed:
                    return FAIL, failed, None
            elif blocked:
                return FAIL, f"Claude TUI blocked at a dialog that is not the folder-trust dialog (see pane-{phase}-ready.txt); nothing typed", None
        elif not self.facts.get("tui_cleared"):  # clear: same process, new conversation (Claude /clear, Codex /new)
            self.herdr("agent", "prompt", pane, "/new" if codex else "/clear", "--wait", "--timeout", "30000", tag="tui:clear", timeout=40)
        if phase in ("initial", "restart", "resume", "clear"):  # clear: the new conversation redraws too
            # Herdr reports idle as soon as a dialog closes, before the input box is drawn; text typed then is lost
            # (agent_prompt_stalled). Wait for the input box footer, then let it settle.
            # An unrecognised startup screen (Codex "Update available! ... Press enter to continue", where Enter
            # means "Update now"; a config migration prompt; ...) may be reported idle, not blocked: without the
            # input box footer on screen nothing is typed (neither the prompt nor, for lostprompt, anything else).
            ready = CODEX_INPUT_READY if codex else TUI_INPUT_READY
            t_box = time.monotonic() + 30
            drawn = False
            while True:
                if ready.search(self.capture_pane(phase + "-input")):
                    drawn = True
                    break
                if time.monotonic() >= t_box:
                    break
                time.sleep(1)
            if not drawn:
                return FAIL, (f"{'Codex' if codex else 'Claude'} TUI input box never appeared within 30s (an unrecognised "
                              f"startup screen may be waiting for an answer; see pane-{phase}-input.txt); nothing typed"), None
            time.sleep(1)
        if phase == "initial" and "lostprompt" in self.scenarios:
            return self.lostprompt_wait(phase)
        for attempt in (1, 2):
            rc, data, out, err = self.herdr("agent", "prompt", pane, self.prompt(), "--wait", "--timeout", str(self.args.timeout * 1000),
                                            tag=f"tui:prompt:{phase}" + ("" if attempt == 1 else ":retry"), timeout=self.args.timeout + 15)
            # One retry only for a prompt that never started a turn (nothing reached the model, so nothing is doubled).
            if rc == 0 or "agent_prompt_stalled" not in (err or ""):
                break
            time.sleep(2)
        screen = self.capture_pane(phase)
        transcript = self.tui_transcript(phase)
        where = f"; session transcript {transcript}" if transcript else "; session transcript not found (model issuance UNVERIFIED)"
        result = (PASS if rc == 0 else FAIL), f"agent prompt rc={rc} {(err or '').strip()[:200]}{where}", transcript
        return self.judge_codex_tui(phase, result, screen) if codex else result

    def accept_claude_trust(self, pane, phase, screen):
        """The user-approved Claude folder-trust answer (/private/tmp scratch only). None on success, else why not."""
        if not self.args.tui_accept_trust:
            return ("Claude TUI shows the folder-trust dialog (acceptance writes the user-global ~/.claude.json); "
                    "rerun with --tui-accept-trust or use --mode print")
        # Claude 2.1.286's dialog opens on "❯ No, exit": a bare Enter exits Claude. Move to the Yes option and
        # press Enter only once the screen shows the cursor on it.
        if TRUST_NO_SELECTED.search(screen):
            self.herdr("agent", "send-keys", pane, "down", tag=f"tui:trust-select-yes:{phase}")
            time.sleep(0.5)
            screen = self.capture_pane(phase + "-trust-select")
        if TRUST_OPTIONS.search(screen) and not TRUST_YES_SELECTED.search(screen):
            return "folder-trust dialog: could not select the Yes option; nothing confirmed"
        self.herdr("agent", "send-keys", pane, "enter", tag=f"tui:accept-trust:{phase}")
        self.herdr("agent", "wait", pane, "--until", "idle", "--until", "done", "--timeout", "60000",
                   tag=f"tui:wait-trusted:{phase}", timeout=70)
        if TRUST_DIALOG.search(self.capture_pane(phase + "-trusted")):
            return "folder-trust dialog still on screen after Enter; prompt not sent"
        return None

    def judge_codex_tui(self, phase, result, screen):
        """Codex TUI: fold the session rollout (thread id, per-phase token usage) and any account failure the pane
        shows into S17 through judge_launch, as for exec (an account failure is ENVIRONMENT, never a product FAIL)."""
        status, detail, transcript = result
        outcome = self.transcript_outcome(None)
        outcome["thread_id"] = self.facts.get("sessions", {}).get(phase)
        outcome["usage"], outcome["usage_basis"] = self.phase_usage(transcript) if transcript else ({}, None)
        outcome["usage"] = outcome["usage"] or {}
        for line in (screen or "").splitlines():
            if re.search(r"error|limit|log ?in|unauthori", line, re.I) and classify_environment(line):
                outcome["failures"].append(f"pane: {line.strip()[:300]}")
        outcome["environment"] = next((k for k in map(classify_environment, outcome["failures"]) if k), None)
        return self.judge_launch(phase, status, detail, transcript, outcome=outcome)

    def lostprompt_wait(self, phase):
        """(b) --scenario lostprompt: the agent starts with NO initial prompt (the startup check-in is its only contact).
        Wait, bounded by --timeout (60 s when health already reports safe_prompt unsupported), for the product's idle
        recovery wake to prompt it and the model to ACK the handoff; nothing is typed by the driver. SL1/SL2/S18
        judge the observed DB wake rows and pane captures, never the health claim alone."""
        seat, message = self.facts["agent_seat"], self.facts["messages"][phase]
        health = self.safe_prompt_state("lostprompt")
        # The baseline was read in s_launch before the agent started (a reservation made while the driver waited for
        # the input box must count as new); a direct call without it reads it now.
        before = self.facts["lostprompt_wake_before"] if "lostprompt_wake_before" in self.facts else self.wake_row()
        record = {"started_utc": utc(), "safe_prompt": health, "wake_before": before, "prompt_sent_by_driver": False}
        self.facts["lostprompt"] = record
        bound = self.args.timeout if health != "unsupported" else min(self.args.timeout, LOSTPROMPT_UNSUPPORTED_WAIT_S)
        started = time.monotonic()
        acked_at = None
        while time.monotonic() - started < bound:
            time.sleep(WARNING_POLL_S)
            receipt = {r["message_id"]: r for r in self.receipts_for(seat)}.get(message) or {}
            if receipt.get("state") == "acked":
                acked_at = acked_at or time.monotonic()
                if time.monotonic() - acked_at >= WAKE_SETTLE_S:  # let the model finish its reply
                    break
        record.update({"wake_after": self.wake_row(), "waited_s": round(time.monotonic() - started, 1), "bound_s": bound,
                       "acked": acked_at is not None, "ended_utc": utc()})
        self.capture_pane("lostprompt")
        self.capture_pane(phase)
        transcript = self.tui_transcript(phase)
        (self.ev / "lostprompt.json").write_text(json.dumps(record, indent=2, default=str))
        where = f"; session transcript {transcript}" if transcript else "; session transcript not found"
        return PASS, (f"agent started with no initial prompt (startup check-in only); waited {record['waited_s']}s of {bound}s "
                      f"for the idle recovery wake (health safe_prompt {health}); handoff ACK "
                      f"{'observed' if acked_at else 'not observed'}{where}"), transcript

    def tui_transcript(self, phase):
        """The Claude session transcript of a TUI phase, read-only: the chosen --session-id for initial/restart/resume,
        and for /clear the one new session transcript written in this project since the phase started. Codex: the
        phase's root session rollout (codex_tui_transcript)."""
        if self.harness == "codex":
            return self.codex_tui_transcript(phase)
        session = self.facts.get("sessions", {}).get(phase)
        if session:
            return self.claude_session_transcript(session)
        known = {s for s in self.facts.get("sessions", {}).values() if s}
        found = self.claude_transcript_since(self.facts.get("phase_started_utc", {}).get(phase), known)
        if found:
            self.facts.setdefault("sessions", {})[phase] = found.stem
        return found

    def claude_project_transcripts(self):
        base = Path(self.args.claude_projects_dir).expanduser()
        found = []
        for cwd in dict.fromkeys((str(self.project), str(self.project.resolve()))):
            directory = base / re.sub(r"[^A-Za-z0-9]", "-", cwd)
            if directory.is_dir():
                found += sorted(directory.glob("*.jsonl"))
        return found

    def claude_transcript_since(self, since, exclude=()):
        """The single top-level session transcript of this scratch project modified since `since` whose session is not
        in `exclude` (the /clear conversation); None when there is none or more than one."""
        floor = datetime.datetime.fromisoformat(since).timestamp() if since else 0
        candidates = [p for p in self.claude_project_transcripts() if p.stem not in exclude and p.stat().st_mtime >= floor]
        return candidates[0] if len(candidates) == 1 else None

    def capture_pane(self, phase):
        rc, _, out, err = self.herdr("pane", "read", self.facts["agent_pane"], "--source", "recent-unwrapped",
                                     "--lines", str(PANE_CAPTURE_LINES),
                                     tag=f"herdr:pane-read:{phase}")
        (self.ev / f"pane-{phase}.txt").write_text(out if rc == 0 else f"pane read rc={rc}: {err}")
        return out if rc == 0 else ""

    # ----- verification -----
    def db_path(self):
        found = sorted(self.state.glob("**/threads.sqlite3"))
        return found[0] if found else None

    def query(self, sql, params=()):
        path = self.db_path()
        if not path:
            raise RuntimeError("no threads.sqlite3 under state dir")
        connection = sqlite3.connect(f"file:{path}?mode=ro", uri=True, timeout=5)
        connection.row_factory = sqlite3.Row
        try:
            return [dict(row) for row in connection.execute(sql, params)]
        finally:
            connection.close()

    def receipts_for(self, seat):
        """Effective receipts, mirroring store::effective::effective_receipt: logical sends are
        send_manifests x prepared_recipients with an optional receipt_state overlay (no overlay =
        pending, retired seat = recipient_retired); legacy rows come from `receipts`."""
        rows = self.query(
            "SELECT sm.message_id, pr.seat_id, CASE WHEN rs.state IS NOT NULL THEN rs.state "
            "WHEN s.retired_at IS NOT NULL THEN 'recipient_retired' ELSE 'pending' END AS state, "
            "rs.ack_actor_seat_id, rs.ack_generation, rs.ack_observation, rs.acked_at, 'manifest' AS src "
            "FROM send_manifests sm JOIN prepared_recipients pr ON pr.preparation_id=sm.preparation_id "
            "JOIN seats s ON s.id=pr.seat_id LEFT JOIN receipt_state rs ON rs.message_id=sm.message_id AND rs.seat_id=pr.seat_id "
            "WHERE pr.seat_id=?", (seat,))
        seen = {r["message_id"] for r in rows}
        rows += [r for r in self.query("SELECT message_id, seat_id, state, ack_actor_seat_id, ack_generation, ack_observation, acked_at, 'receipts' AS src "
                                       "FROM receipts WHERE seat_id=?", (seat,)) if r["message_id"] not in seen]
        return rows

    def transcript_calls(self, transcript):
        """(call_id, command, is_child) for every herdr-threads shell call in a native transcript."""
        calls = []
        if not transcript or not Path(transcript).exists():
            return calls
        for line in Path(transcript).read_text(errors="replace").splitlines():
            try:
                event = json.loads(line)
            except ValueError:
                continue
            if not isinstance(event, dict):
                continue
            if self.harness == "claude":
                # D4 (demo 2): `system/permission_denied` carries `message` as a string; only an assistant message
                # object with a content list can hold tool_use blocks.
                message = event.get("message")
                content = message.get("content") if isinstance(message, dict) else None
                for block in content if isinstance(content, list) else []:
                    if not isinstance(block, dict) or not isinstance(block.get("input"), dict):
                        continue
                    if block.get("type") == "tool_use" and "herdr-threads" in json.dumps(block["input"]):
                        child = bool(event.get("parent_tool_use_id") or event.get("isSidechain"))
                        calls.append((block.get("id"), str(block["input"].get("command", "")), child))
            else:
                item = event.get("item") if isinstance(event.get("item"), dict) else {}
                command = item.get("command") or ""
                if event.get("type") == "item.completed" and "herdr-threads" in command:
                    calls.append((item.get("id"), command, bool(item.get("agent_id") or event.get("agent_id"))))
                found = self.rollout_call(event)  # a Codex session rollout (managed launch): root thread's own calls
                if found and "herdr-threads" in found[1]:
                    calls.append((found[0], found[1], False))
        return calls

    def claude_session_transcript(self, session):
        """The Claude session transcript `<projects>/<encoded cwd>/<session>.jsonl` (every non-alphanumeric
        cwd character becomes `-`). Located, never written. Falls back to a unique `*/<session>.jsonl`."""
        base = Path(self.args.claude_projects_dir).expanduser()
        for cwd in dict.fromkeys((str(self.project), str(self.project.resolve()))):
            candidate = base / re.sub(r"[^A-Za-z0-9]", "-", cwd) / f"{session}.jsonl"
            if candidate.is_file():
                return candidate
        matches = sorted(base.glob(f"*/{session}.jsonl")) if base.is_dir() else []
        return matches[0] if len(matches) == 1 else None

    def codex_home(self):
        return Path(self.facts.get("codex_home") or os.environ.get("CODEX_HOME") or Path.home() / ".codex").expanduser()

    def codex_rollout(self, thread_id):
        """The persisted Codex session rollout `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<stamp>-<thread_id>.jsonl`.
        Located by glob and only ever opened for reading; None unless exactly one file matches."""
        sessions = self.codex_home() / "sessions"
        matches = sorted(sessions.glob(f"**/rollout-*{thread_id}.jsonl")) if thread_id and sessions.is_dir() else []
        return matches[0] if len(matches) == 1 else None

    @staticmethod
    def rollout_message(entry):
        """(role, text) of a rollout model message, from the `{timestamp, type: response_item, payload: {type:
        message, role, content: [{text}]}}` line form or a flat `{type: message, role, text}` excerpt; else None."""
        payload = entry.get("payload") if isinstance(entry.get("payload"), dict) else entry
        if payload.get("type") != "message":
            return None
        content = payload.get("content")
        if isinstance(content, list):
            text = "\n".join(str(c.get("text", "")) for c in content if isinstance(c, dict))
        else:
            text = str(payload.get("text") or content or "")
        return str(payload.get("role") or ""), text

    def codex_hook_context(self, phase):
        """D4 (Codex demo 1): Codex delivers hook additionalContext as `developer` messages in the session rollout
        (codex-158-live-hook-capture: SessionStart context before the phase's user prompt, PreToolUse context after
        the tool call). Read READ-ONLY; a missing rollout (e.g. --ephemeral, other CODEX_HOME) is UNVERIFIED, never
        a pass and never a failure of the product."""
        session = self.facts.get("sessions", {}).get(phase)
        since = self.facts.get("phase_started_utc", {}).get(phase)
        until = self.facts.get("phase_ended_utc", {}).get(phase)
        path = self.codex_rollout(session)
        record = {"phase": phase, "session": session, "since_utc": since, "until_utc": until, "codex_home": str(self.codex_home()),
                  "rollout": str(path) if path else None, "read_only": True, "deliveries": []}
        if not path:
            (self.ev / f"hook-context-{phase}.json").write_text(json.dumps(record, indent=2))
            return UNVERIFIED, (f"no single Codex rollout for thread {session} under {self.codex_home() / 'sessions'}; "
                                f"additionalContext delivery UNVERIFIED (the exec stream does not record it)"), None
        floor = datetime.datetime.fromisoformat(since) if since else None
        ceiling = datetime.datetime.fromisoformat(until) if until else None
        seen_user = False
        with open(path, "r", encoding="utf-8", errors="replace") as stream:  # read-only
            lines = stream.read().splitlines()
        for number, line in enumerate(lines, start=1):
            try:
                entry = json.loads(line)
            except ValueError:
                continue
            if not isinstance(entry, dict):
                continue
            if floor or ceiling:
                try:
                    stamp = datetime.datetime.fromisoformat(str(entry.get("timestamp", "")).replace("Z", "+00:00"))
                except ValueError:
                    continue
                if (floor and stamp < floor) or (ceiling and stamp > ceiling):
                    continue
            message = self.rollout_message(entry)
            if not message:
                continue
            role, text = message
            if role == "user" and self.prompt() in text:
                seen_user = True
            if role != "developer":
                continue
            mentions_message = self.facts["messages"].get(phase, "\0") in text
            mentions_thread = self.facts.get("thread", "\0") in text
            # D7 (Codex matrix 1): only herdr-threads hook output counts; Codex's own developer messages (e.g. the
            # `<skills_instructions>` block, which may name herdr-threads) are not deliveries.
            if HOOK_PREAMBLE not in text:
                record["other_developer_messages"] = record.get("other_developer_messages", 0) + 1
                continue
            record["deliveries"].append({
                "line": number, "timestamp": entry.get("timestamp"), "hook_event": "PreToolUse" if seen_user else "SessionStart",
                "bytes": len(text.encode()), "sha256": hashlib.sha256(text.encode()).hexdigest(),
                "mentions_message": mentions_message, "mentions_thread": mentions_thread, "excerpt": text[:600]})
        (self.ev / f"hook-context-{phase}.json").write_text(json.dumps(record, indent=2))
        starts = [d for d in record["deliveries"] if d["hook_event"] == "SessionStart"]
        pre = [d for d in record["deliveries"] if d["hook_event"] == "PreToolUse"]
        self.facts.setdefault("hook_context", {})[phase] = {"session_start": len(starts), "pre_tool_use": len(pre), "rollout": str(path)}
        tail = f"PreToolUse developer context {len(pre)}x ({sum(d['mentions_message'] for d in pre)} naming {self.facts['messages'].get(phase)})"
        if not any(d["mentions_message"] or d["mentions_thread"] for d in starts):
            return FAIL, f"rollout {path.name} has no developer message naming the thread/message before the prompt since {since}; {tail}", None
        return PASS, (f"SessionStart context delivered {len(starts)}x as rollout developer message naming the handoff; {tail}; "
                      f"evidence hook-context-{phase}.json"), None

    def s_hook_context(self, phase):
        """D2: hook additionalContext delivery as the harness recorded it. Claude print mode shows PreToolUse
        context only as `hook_additional_context` attachments in the session transcript under
        ~/.claude/projects, read here READ-ONLY and limited to entries written since this phase started."""
        if self.harness != "claude":
            return self.codex_hook_context(phase)
        session = self.facts.get("sessions", {}).get(phase)
        since = self.facts.get("phase_started_utc", {}).get(phase)
        until = self.facts.get("phase_ended_utc", {}).get(phase)
        path = self.claude_session_transcript(session) if session else None
        record = {"phase": phase, "session": session, "since_utc": since, "until_utc": until, "transcript": str(path) if path else None,
                  "read_only": True, "deliveries": []}
        if not path:
            (self.ev / f"hook-context-{phase}.json").write_text(json.dumps(record, indent=2))
            return FAIL, f"Claude session transcript for {session} not found under {self.args.claude_projects_dir}; delivery UNVERIFIED", None
        floor = datetime.datetime.fromisoformat(since) if since else None
        ceiling = datetime.datetime.fromisoformat(until) if until else None
        with open(path, "r", encoding="utf-8", errors="replace") as stream:  # read-only
            lines = stream.read().splitlines()
        for line in lines:
            try:
                entry = json.loads(line)
            except ValueError:
                continue
            attachment = entry.get("attachment")
            if entry.get("type") != "attachment" or not isinstance(attachment, dict) or attachment.get("type") != "hook_additional_context":
                continue
            try:
                stamp = datetime.datetime.fromisoformat(str(entry.get("timestamp", "")).replace("Z", "+00:00"))
            except ValueError:
                stamp = None
            if (floor or ceiling) and stamp is None:
                continue
            if (floor and stamp < floor) or (ceiling and stamp > ceiling):
                continue
            content = attachment.get("content")
            text = "\n".join(content) if isinstance(content, list) else str(content or "")
            record["deliveries"].append({
                "timestamp": entry.get("timestamp"), "hook_event": attachment.get("hookEvent"), "hook_name": attachment.get("hookName"),
                "tool_use_id": attachment.get("toolUseID"), "sidechain": bool(entry.get("isSidechain")),
                "bytes": len(text.encode()), "sha256": hashlib.sha256(text.encode()).hexdigest(),
                "mentions_message": self.facts["messages"].get(phase, "\0") in text,
                "mentions_thread": self.facts.get("thread", "\0") in text, "excerpt": text[:600]})
        (self.ev / f"hook-context-{phase}.json").write_text(json.dumps(record, indent=2))
        starts = [d for d in record["deliveries"] if d["hook_event"] == "SessionStart" and not d["sidechain"]]
        pre = [d for d in record["deliveries"] if d["hook_event"] == "PreToolUse"]
        self.facts.setdefault("hook_context", {})[phase] = {"session_start": len(starts), "pre_tool_use": len(pre),
                                                             "transcript": str(path)}
        tail = (f"PreToolUse additionalContext delivered {len(pre)}x "
                f"({sum(d['mentions_message'] for d in pre)} naming {self.facts['messages'].get(phase)})")
        if not any(d["mentions_message"] or d["mentions_thread"] for d in starts):
            return FAIL, f"no SessionStart additionalContext naming the thread/message in {path.name} since {since}; {tail}", None
        return PASS, f"SessionStart additionalContext delivered {len(starts)}x naming the handoff; {tail}; evidence hook-context-{phase}.json", None

    def subagent_activity(self, transcript):
        """B1: every subagent/collab item the root transcript shows: [(item_id, kind, tool)].

        Codex: `codex exec --json` never carries a child's own command_execution items, but it does carry
        every collab/subagent tool call the root made (spawn, wait, ...). Claude: a Task/Agent tool_use, or
        any event attributed to a subagent (parent_tool_use_id / isSidechain)."""
        items = []
        if not transcript or not Path(transcript).exists():
            return items
        for line in Path(transcript).read_text(errors="replace").splitlines():
            try:
                event = json.loads(line)
            except ValueError:
                continue
            if not isinstance(event, dict):
                continue
            found = []
            if self.harness == "codex":
                item = event.get("item") or {}
                kind = str(item.get("type") or "")
                if event.get("type") in ("item.started", "item.completed") and CODEX_CHILD_ITEMS.search(kind):
                    found.append((item.get("id"), kind, item.get("tool")))
                name = self.rollout_child_call(event)  # interactive Codex: the root rollout's own collab calls
                if name:
                    found.append((name[0], "rollout_function_call", name[1]))
            else:
                message = event.get("message") or {}
                content = message.get("content") if isinstance(message, dict) else None
                for block in content if isinstance(content, list) else []:
                    if isinstance(block, dict) and block.get("type") == "tool_use" and block.get("name") in CLAUDE_CHILD_TOOLS:
                        found.append((block.get("id"), "tool_use", block.get("name")))
                if event.get("parent_tool_use_id") or event.get("isSidechain"):
                    found.append((event.get("parent_tool_use_id"), "subagent_event", event.get("type")))
            for entry in found:
                if entry not in items:
                    items.append(entry)
        return items

    def root_execution(self, phase, bindings):
        """Diagnostic only: the first occupant binding registered during this phase (ordinal above the
        pre-launch high-water mark) whose native session is the root session the harness reported.

        It is NOT evidence that the root (rather than a child) issued an ACK: under the cooperative model the
        ACK's `execution` is the calling CLI's claim from the seat's context journal, which subagent hook
        events never replace, so a child's `herdr-threads ack` in the same pane records this same execution."""
        session = self.facts.get("sessions", {}).get(phase)
        before = self.facts.get("bindings_before", {}).get(phase)
        if not session or before is None:
            return None
        for binding in bindings:
            if binding["ordinal"] > before and binding["native_session"] == session:
                return binding["execution_id"]
        return None

    def s_wait_and_verify(self, phase, transcript):
        seat, message = self.facts["agent_seat"], self.facts["messages"][phase]
        deadline = time.monotonic() + (0 if self.dry else self.args.verify_wait)
        while True:
            receipts = {r["message_id"]: r for r in self.receipts_for(seat)}
            if receipts.get(message, {}).get("state") == "acked" or time.monotonic() >= deadline:
                break
            time.sleep(2)
        invitations = self.query("SELECT id, state, accepted_actor_seat_id, accepted_generation, accepted_observation, accepted_at "
                                 "FROM invitations WHERE thread_id=? AND seat_id=?", (self.facts["thread"], seat))
        bindings = self.query("SELECT ordinal, generation, harness, native_session, execution_id, observation_provenance, registered_at, ended_at "
                              "FROM occupant_bindings WHERE seat_id=? ORDER BY ordinal", (seat,))
        snapshot = {"phase": phase, "seat": seat, "message": message, "receipts": list(receipts.values()),
                    "invitations": invitations, "occupant_bindings": bindings}
        (self.ev / f"sqlite-{phase}.json").write_text(json.dumps(snapshot, indent=2, sort_keys=True))
        problems = []
        provenance = {"ack": None, "accept": None, "ack_execution": None, "accept_execution": None}
        receipt = receipts.get(message)
        if not receipt:
            problems.append(f"no receipt row for message {message} / seat {seat}")
        elif receipt["state"] != "acked":
            problems.append(f"receipt {message} state={receipt['state']} (not ACKed within bound)")
        else:
            obs = json.loads(receipt["ack_observation"] or "{}")
            provenance["ack"], provenance["ack_execution"] = obs.get("provenance"), obs.get("execution")
            if receipt["ack_actor_seat_id"] != seat:
                problems.append(f"ACK actor seat {receipt['ack_actor_seat_id']} != agent seat {seat}")
            if obs.get("provenance") not in MODEL_PROVENANCE:
                problems.append(f"ACK provenance {obs.get('provenance')!r} is not a top-level model provenance class")
            if obs.get("harness") not in (self.harness, self.harness.capitalize()):
                problems.append(f"ACK harness {obs.get('harness')!r} != {self.harness}")
            expected_session = self.facts.get("sessions", {}).get(phase)
            if expected_session and obs.get("session") not in (None, expected_session):
                problems.append(f"ACK session {obs.get('session')!r} != native session {expected_session}")
        for earlier in self.phase_results:  # earlier ACKs keep their original provenance
            before = earlier.get("receipt") or {}
            now = receipts.get(earlier["message"]) or {}
            if before.get("state") == "acked" and (now.get("state"), now.get("ack_observation")) != ("acked", before.get("ack_observation")):
                problems.append(f"earlier receipt {earlier['message']} changed after phase {phase}")
        accepted = [i for i in invitations if i["state"] == "accepted"]
        if phase == "initial":
            if not accepted:
                problems.append(f"invitation not accepted (states {[i['state'] for i in invitations]})")
            else:
                obs = json.loads(accepted[0]["accepted_observation"] or "{}")
                provenance["accept"], provenance["accept_execution"] = obs.get("provenance"), obs.get("execution")
                if accepted[0]["accepted_actor_seat_id"] != seat or obs.get("provenance") not in MODEL_PROVENANCE:
                    problems.append(f"acceptance actor/provenance {accepted[0]['accepted_actor_seat_id']}/{obs.get('provenance')!r} not a top-level model class")
        # Model issuance: the transcript must contain the root agent's own ack call for this ID; the driver ledger none.
        calls = self.transcript_calls(transcript)
        (self.ev / f"transcript-calls-{phase}.json").write_text(json.dumps(calls, indent=2))
        root_acks = [c for c in calls if re.search(r"\back\b", c[1]) and message in c[1] and not c[2]]
        child_mut = [c for c in calls if c[2] and mutating_call(c[1])]
        if self.delegates():  # a child attempt the product refused is the designed outcome, not a failure
            outcomes = self.transcript_results(transcript)
            child_mut = [c for c in child_mut if (outcomes.get(c[0]) or {}).get("error") is not True]
        if transcript is not None and not root_acks:
            problems.append(f"transcript has no root `herdr-threads ack {message}` call")
        if transcript is None and self.args.mode == "tui":
            problems.append("TUI mode: transcript join not automated; see pane capture (model issuance UNVERIFIED)")
        if child_mut:
            problems.append(f"child/subagent issued mutating calls: {child_mut}")
        all_denials = self.facts.get("launch_outcome", {}).get(phase, {}).get("permission_denials", [])
        # D7 (demo 3): only a denial whose simple-command word is herdr-threads blocked the product path; any other
        # (e.g. `alias herdr-threads; type herdr-threads`) is reported as info, never a FAIL.
        denials = [d for d in all_denials if invokes_herdr_threads(d.get("command"))]
        other_denials = [d for d in all_denials if not invokes_herdr_threads(d.get("command"))]
        if denials:  # D4 (demo 2): the real reason the model stopped, from the result event's permission_denials
            problems.append(f"harness denied {len(denials)} herdr-threads command(s) (permission_denials): "
                            + "; ".join(str(d["command"])[:200] for d in denials[:3]))
        # B1 (cooperative limit): the DB cannot tell a child's ACK from the root's (both carry the pane's
        # cooperative claim, same execution). So child non-ACK is decided only by the transcript: no subagent
        # activity at all -> nothing could have delegated; any subagent activity -> UNVERIFIED (see S<n>C).
        child_items = self.subagent_activity(transcript)
        root_exec = self.root_execution(phase, bindings)
        sidechain = self.child_sidechain(phase, transcript) if self.delegates() else None
        if transcript is None:
            child_check = "unverified_no_transcript"
        elif child_items and sidechain and self.sidechain_clears(sidechain, provenance.get("ack_execution"), root_exec):
            # --scenario child: the complete sidechain shows every child call and none a successful write: a
            # transcript-level PASS (ht-910). The execution equality is consistency only; it cannot discriminate (B1).
            child_check = "sidechain_verified_no_child_mutation"
        elif child_items:
            child_check = "unverified_subagent_activity"
        else:
            child_check = "no_subagent_activity"
        driver_mutations = [json.loads(l)["argv"] for l in (self.ev / "commands.jsonl").read_text().splitlines()
                            if json.loads(l).get("tag", "") and re.match(r"(coordinator|seat|hook-probe)", json.loads(l)["tag"])
                            and any(a in ("ack", "accept", "accept-required") for a in json.loads(l)["argv"])]
        if driver_mutations:
            problems.append(f"driver issued ack/accept itself: {driver_mutations}")
        klass = self.provenance_label([v for k, v in provenance.items() if k in ("ack", "accept") and v])
        self.phase_results.append({"phase": phase, "message": message, "root_ack_calls": root_acks, "receipt": receipt,
                                   "permission_denials": denials, "other_permission_denials": other_denials,
                                   "provenance": provenance, "provenance_label": klass, "root_binding_execution_diagnostic": root_exec,
                                   "child_agent_calls": child_items, "child_check": child_check, "sidechain": sidechain,
                                   "problems": problems})
        info = (f"; info: {len(other_denials)} other permission denial(s), not herdr-threads commands: "
                + "; ".join(str(d.get("command"))[:200] for d in other_denials[:3])) if other_denials else ""
        denied = self.facts.get("launch_outcome", {}).get(phase, {}).get("transport_denied")
        if problems and denied:
            # D2: the model never reached the daemon; the missing ACK is a transport result, not a product FAIL.
            return TRANSPORT, (f"UNSUPPORTED transport_denied ({denied['kind']}): the harness sandbox refused the daemon socket "
                               f"({denied['evidence'][:160]}); " + "; ".join(problems)), None
        if problems:
            return FAIL, "; ".join(problems) + info, None
        return PASS, (f"model ACK of {message} (transcript root call) matches SQLite; DB provenance "
                      f"ack={provenance['ack']} accept={provenance['accept'] or 'n/a'} -> {klass}"
                      + ("" if klass == "native" else " (cooperative claim, NOT native proof)") + info), None

    def s_child_check(self, phase):
        """B1: child/subagent ACK absence for `phase`. PASS when the root transcript shows no subagent or collab
        activity, or (--scenario child) when the complete sidechain record shows no successful child write: a
        transcript-level PASS only. Otherwise UNVERIFIED (never PASS): under the cooperative model a child's ACK in
        the same pane is stored with the root's seat, provenance and execution, so SQLite cannot exclude it."""
        result = next((r for r in reversed(self.phase_results) if r["phase"] == phase), None)
        if result is None:
            return FAIL, f"no verification result for phase {phase}", None
        check = result["child_check"]
        if check == "no_subagent_activity":
            return PASS, "root transcript shows no subagent/collab activity; no child could have issued the accept/ACK", None
        if check == "sidechain_verified_no_child_mutation":
            sidechain = result.get("sidechain") or {}
            return PASS, (f"subagent activity {result['child_agent_calls'][:3]}, but the complete sidechain "
                          f"({len(sidechain.get('calls', []))} child herdr-threads call(s)) has no successful child "
                          f"write (transcript-level evidence; DB-level child separation stays UNVERIFIED under B1)"), None
        if check == "unverified_no_transcript":
            return UNVERIFIED, "no machine-readable transcript (TUI); subagent activity unknown, child ACK absence UNVERIFIED", None
        return UNVERIFIED, (f"subagent/collab activity {result['child_agent_calls'][:5]} in root transcript; a child ACK would be "
                            f"stored with the same seat/provenance/execution (cooperative limit), so child ACK absence is "
                            f"UNVERIFIED; manifest cannot PASS"), None

    @staticmethod
    def sidechain_clears(sidechain, ack_execution, root_execution):
        """True when a child sidechain record excludes a child ACK/accept: complete and every child mutating call has a
        recorded refusal. The execution comparison is a consistency check only (a mismatch is a failure); equality does
        NOT discriminate, since under B1 a child's ACK carries the root binding's execution too."""
        if not sidechain.get("complete"):
            return False
        for call in sidechain.get("calls", []):
            if mutating_call(call["command"]) and call.get("result_error") is not True:
                return False
        return not (ack_execution and root_execution and ack_execution != root_execution)

    @staticmethod
    def provenance_label(classes):
        """`native` only when every observed accept/ACK class is native proof; `none` when no accept/ACK
        provenance was stored at all (D5, demo 2); otherwise `cooperative` (S5)."""
        if not classes:
            return "none"
        return "native" if all(c == NATIVE_PROOF for c in classes) else "cooperative"

    def s_agent_read(self):
        """Dry-run only: the agent pane's own CLI (no --cooperative-* flags, only HERDR_PANE_ID) can read
        its pending receipts through the context the SessionStart hook established. A read, never an ACK."""
        if not self.dry:
            raise Blocked("live run: only the model uses the agent pane's CLI")
        env = dict(os.environ)
        env.update({"HERDR_ENV": "1", "HERDR_PANE_ID": self.facts["agent_pane"], "HERDR_SOCKET_PATH": self.endpoint})
        rc, out, err = self.run([str(self.bindir / "herdr-threads"), "--json", "pending-receipts"], env=env,
                                timeout=30, tag="agent-read:pending-receipts", cwd=str(self.project))
        (self.ev / "agent-read-pending-receipts.json").write_text(out or err)
        message = self.facts["messages"]["initial"]
        if rc != 0:
            return FAIL, f"agent-pane `pending-receipts` rc={rc}: {(err or out).strip()[:300]}", None
        if message not in out:
            return FAIL, f"agent-pane `pending-receipts` rc=0 but does not list {message}: {out[:200]!r}", None
        return PASS, f"agent pane CLI lists pending {message} via hook-established context", None

    def s_ready_executes(self):
        """D8 (demo 3), dry-run only: run one read-only ready command from the SessionStart probe context exactly as
        rendered, through the agent pane's PATH shim, and require exit 0 (a rendered command that fails as written
        costs the model its first turns). A read, never an accept or ACK."""
        if not self.dry:
            raise Blocked("live run: only the model uses the agent pane's CLI")
        commands = ready_commands(self.facts.get("probe_context", {}).get("SessionStart"))
        chosen = next((c for verb in ("pending-receipts", "inbox") for c in commands if re.search(rf"\s{verb}$", c)), None)
        if chosen is None:
            return FAIL, f"no read-only ready command (pending-receipts/inbox) in the SessionStart probe context: {commands}", None
        env = dict(os.environ)
        env.update({"HERDR_ENV": "1", "HERDR_PANE_ID": self.facts["agent_pane"], "HERDR_SOCKET_PATH": self.endpoint,
                    "PATH": f"{self.bindir}{os.pathsep}{os.environ.get('PATH', '')}"})
        rc, out, err = self.run(["/bin/sh", "-c", chosen], env=env, timeout=30, tag="agent-run:ready-command", cwd=str(self.project))
        self.facts["ready_command_executed"] = {"command": chosen, "rc": rc}
        if rc != 0:
            return FAIL, f"rendered ready command `{chosen[:200]}` exits {rc} through the agent shim: {(err or out).strip()[:300]}", None
        return PASS, f"rendered ready command `{chosen[:200]}` exits 0 through the agent shim", None

    def s_prelaunch_state(self):
        """Before any agent runs: the invitation and receipt exist and are pending; nobody ACKed."""
        seat, message = self.facts["agent_seat"], self.facts["messages"]["initial"]
        receipts = {r["message_id"]: r for r in self.receipts_for(seat)}
        invitations = self.query("SELECT id, state FROM invitations WHERE thread_id=? AND seat_id=?", (self.facts["thread"], seat))
        memberships = self.query("SELECT state FROM memberships WHERE thread_id=? AND seat_id=?", (self.facts["thread"], seat))
        bindings = self.query("SELECT COUNT(*) AS n FROM occupant_bindings WHERE seat_id=?", (seat,))[0]["n"]
        snapshot = {"receipts": list(receipts.values()), "invitations": invitations, "memberships": memberships, "occupant_bindings": bindings}
        (self.ev / "sqlite-prelaunch.json").write_text(json.dumps(snapshot, indent=2, sort_keys=True))
        problems = []
        if receipts.get(message, {}).get("state") != "pending":
            problems.append(f"receipt for {message} is {receipts.get(message, {}).get('state')!r}, expected 'pending'")
        if [i["state"] for i in invitations] != ["pending"]:
            problems.append(f"invitation states {[i['state'] for i in invitations]}, expected ['pending']")
        if any(r["state"] == "acked" for r in receipts.values()):
            problems.append("an ACK exists before any agent launch")
        return (FAIL if problems else PASS), ("; ".join(problems) or
                f"invitation pending, receipt {message} pending, membership {[m['state'] for m in memberships]}, {bindings} occupant bindings"), None

    # ----- opt-in scenarios: shared transcript readers -----
    def transcript_events(self, transcript):
        if not transcript or not Path(transcript).exists():
            return []
        events = []
        for line in Path(transcript).read_text(errors="replace").splitlines():
            try:
                event = json.loads(line)
            except ValueError:
                continue
            if isinstance(event, dict):
                events.append(event)
        return events

    @staticmethod
    def rollout_call(event):
        """(call_id, command) of a Codex rollout shell call (`response_item` payload `function_call` /
        `local_shell_call`, or a code-mode `custom_tool_call` whose script runs `tools.exec_command({cmd: "..."})`:
        its commands, joined by newlines), else None."""
        payload = event.get("payload") if isinstance(event.get("payload"), dict) else None
        if not payload or payload.get("type") not in ("function_call", "local_shell_call", "custom_tool_call"):
            return None
        if payload.get("type") == "custom_tool_call" and isinstance(payload.get("input"), str):
            commands = Driver.exec_script_commands(payload["input"])
            if commands:
                return payload.get("call_id") or payload.get("id"), "\n".join(commands)
        raw = payload.get("arguments") or payload.get("input") or payload.get("action") or ""
        try:
            args = json.loads(raw) if isinstance(raw, str) else raw
        except ValueError:
            args = raw
        command = args.get("command") or args.get("cmd") if isinstance(args, dict) else args
        if isinstance(command, list):
            command = shlex.join(str(c) for c in command)
        return payload.get("call_id") or payload.get("id"), str(command or "")

    @staticmethod
    def exec_script_commands(script):
        """Every `cmd` string literal of a Codex code-mode script (`tools.exec_command({cmd: "...", ...})`, double- or
        single-quoted, JSON escapes decoded); [] when the script names none. W6-D1: when some `cmd:` is not a quoted
        literal (a template or a variable), the raw script is appended, so its command is still scanned."""
        commands = []
        literal_re = re.compile(r"""("(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*')""")
        opaque = False
        for match in re.finditer(r"\bcmd\s*:\s*", script):
            quoted = literal_re.match(script, match.end())
            if not quoted:
                opaque = True
                continue
            literal = quoted.group(1)
            if literal.startswith("'"):
                literal = '"' + literal[1:-1].replace('\\\'', "'").replace('"', '\\"') + '"'
            try:
                commands.append(str(json.loads(literal)))
            except ValueError:
                commands.append(literal[1:-1])
        if opaque:
            commands.append(script)
        return commands

    def root_tool_calls(self, transcript):
        """[(call_id, command)] of every top-level tool call, in transcript order (any tool, not only herdr-threads)."""
        calls = []
        for event in self.transcript_events(transcript):
            if self.harness == "claude":
                if event.get("parent_tool_use_id") or event.get("isSidechain"):
                    continue
                message = event.get("message")
                content = message.get("content") if isinstance(message, dict) else None
                for block in content if isinstance(content, list) else []:
                    if isinstance(block, dict) and block.get("type") == "tool_use":
                        command = (block.get("input") or {}).get("command") if isinstance(block.get("input"), dict) else None
                        calls.append((block.get("id"), str(command or block.get("name") or "")))
            else:
                item = event.get("item") if isinstance(event.get("item"), dict) else {}
                if event.get("type") in ("item.started", "item.completed") and item.get("type") == "command_execution":
                    if not (item.get("agent_id") or event.get("agent_id")) and item.get("id") not in [c[0] for c in calls]:
                        calls.append((item.get("id"), str(item.get("command") or "")))
                found = self.rollout_call(event)
                if found and found[0] not in [c[0] for c in calls]:
                    calls.append(found)
        return calls

    def first_tool_call(self, transcript):
        calls = self.root_tool_calls(transcript)
        return calls[0][0] if calls else None

    def transcript_results(self, transcript):
        """{call_id: {"text", "error"}} for every tool result the transcript records (Claude tool_result blocks,
        Codex exec command_execution items, Codex rollout function_call_output)."""
        results = {}
        for event in self.transcript_events(transcript):
            message = event.get("message")
            content = message.get("content") if isinstance(message, dict) else None
            for block in content if isinstance(content, list) else []:
                if isinstance(block, dict) and block.get("type") == "tool_result":
                    body = block.get("content")
                    if isinstance(body, list):
                        body = "\n".join(str(b.get("text", "")) for b in body if isinstance(b, dict))
                    results[block.get("tool_use_id")] = {"text": str(body or ""), "error": bool(block.get("is_error"))}
            item = event.get("item") if isinstance(event.get("item"), dict) else {}
            if event.get("type") == "item.completed" and item.get("type") == "command_execution":
                code = item.get("exit_code")
                results[item.get("id")] = {"text": str(item.get("aggregated_output") or ""),
                                           "error": (code not in (0, None)) or item.get("status") == "failed"}
            payload = event.get("payload") if isinstance(event.get("payload"), dict) else {}
            if payload.get("type") in ("function_call_output", "custom_tool_call_output"):
                output = payload.get("output")
                text, code = output, None
                if isinstance(output, str):
                    try:
                        parsed = json.loads(output)
                        if isinstance(parsed, dict):
                            text, code = parsed.get("output", output), (parsed.get("metadata") or {}).get("exit_code")
                    except ValueError:
                        pass
                elif isinstance(output, dict):
                    text, code = output.get("output") or output.get("content"), (output.get("metadata") or {}).get("exit_code")
                if isinstance(text, list):  # code-mode output: [{type: input_text, text}, ...]
                    text = "\n".join(str(t.get("text", "")) if isinstance(t, dict) else str(t) for t in text)
                match = re.search(r"(?:Process exited with code|Exit code:?)\s*(\d+)", str(text or ""))
                if code is None and match:
                    code = int(match.group(1))
                results[payload.get("call_id")] = {"text": str(text or ""), "error": code not in (0, None)}
        return results

    def phase_transcript(self, phase):
        return self.facts.get("phase_transcripts", {}).get(phase)

    def phase_result(self, phase):
        return next((r for r in reversed(self.phase_results) if r["phase"] == phase), None)

    # ----- child (ht-910 delegation) -----
    def claude_subagent_files(self, session):
        """Claude subagent (sidechain) transcripts for `session`: `<project dir>/<session>/subagents/*.jsonl`, plus
        any `agent-*.jsonl` beside the session whose entries name the session. Read-only."""
        files = []
        base = Path(self.args.claude_projects_dir).expanduser()
        for cwd in dict.fromkeys((str(self.project), str(self.project.resolve()))):
            directory = base / re.sub(r"[^A-Za-z0-9]", "-", cwd)
            files += sorted((directory / str(session) / "subagents").glob("*.jsonl")) if session else []
            for candidate in sorted(directory.glob("agent-*.jsonl")) if directory.is_dir() else []:
                if session and f'"sessionId":"{session}"' in candidate.read_text(errors="replace").replace(" ", ""):
                    files.append(candidate)
        return list(dict.fromkeys(files))

    def child_sidechain(self, phase, transcript):
        """Every child/subagent herdr-threads call the evidence records, and whether that record is complete.

        Claude: each root Task/Agent tool_use is a spawn; its calls come from stream events carrying that
        parent_tool_use_id and from the session's subagent transcripts. Complete when every spawn has sidechain
        evidence. Codex: collab spawn items name child threads; their rollouts (read-only) hold the child calls.
        Complete only when every spawn names a thread whose rollout is found. Incomplete -> child ACK absence stays
        UNVERIFIED (B1)."""
        record = {"spawns": [], "calls": [], "files": [], "complete": False, "missing": []}
        events = self.transcript_events(transcript)
        if self.harness == "claude":
            by_parent = {}
            for event in events:
                parent = event.get("parent_tool_use_id")
                message = event.get("message")
                content = message.get("content") if isinstance(message, dict) else None
                for block in content if isinstance(content, list) else []:
                    if not isinstance(block, dict) or block.get("type") != "tool_use":
                        continue
                    if not parent and not event.get("isSidechain") and block.get("name") in CLAUDE_CHILD_TOOLS:
                        record["spawns"].append(block.get("id"))
                    elif parent or event.get("isSidechain"):
                        command = str((block.get("input") or {}).get("command", "")) if isinstance(block.get("input"), dict) else ""
                        by_parent.setdefault(parent, 0)
                        by_parent[parent] += 1
                        if "herdr-threads" in command:
                            record["calls"].append({"id": block.get("id"), "command": command, "source": "stream", "parent": parent})
            files = self.claude_subagent_files(self.facts.get("sessions", {}).get(phase))
            for path in files:
                record["files"].append(str(path))
                for event in self.transcript_events(path):
                    message = event.get("message")
                    content = message.get("content") if isinstance(message, dict) else None
                    for block in content if isinstance(content, list) else []:
                        if isinstance(block, dict) and block.get("type") == "tool_use" and isinstance(block.get("input"), dict):
                            command = str(block["input"].get("command", ""))
                            if "herdr-threads" in command and block.get("id") not in [c["id"] for c in record["calls"]]:
                                record["calls"].append({"id": block.get("id"), "command": command, "source": str(path.name), "parent": None})
            results = self.transcript_results(transcript)
            for path in files:
                results.update(self.transcript_results(path))
            record["missing"] = [s for s in record["spawns"] if s not in by_parent] if len(files) < len(record["spawns"]) else []
            record["complete"] = bool(record["spawns"]) and not record["missing"]
        else:
            threads = []
            for event in events:
                item = event.get("item") if isinstance(event.get("item"), dict) else {}
                if event.get("type") in ("item.started", "item.completed") and CODEX_CHILD_ITEMS.search(str(item.get("type") or "")):
                    if item.get("id") not in record["spawns"]:
                        record["spawns"].append(item.get("id"))
                    for key, value in item.items():
                        values = value if isinstance(value, list) else [value]
                        if ("thread" in key and "sender" not in key) or key in ("agent_id", "receiver_id"):
                            threads += [v for v in values if isinstance(v, str) and v and v != self.facts.get("sessions", {}).get(phase)]
            for event in events:  # interactive Codex (rollout transcript): collab tool calls of the root thread
                found = self.rollout_child_call(event)
                if found and found[0] not in record["spawns"]:
                    record["spawns"].append(found[0])
            threads = list(dict.fromkeys(threads))
            # D1 (Codex matrix 1): 0.159 shows a spawn only as a `wait` collab call with no receiver thread ID, but the
            # child's rollout names the root in its session_meta (parent_thread_id / forked_from_id).
            children = self.codex_child_rollouts(self.facts.get("sessions", {}).get(phase),
                                                 self.facts.get("phase_started_utc", {}).get(phase))
            record["discovered"] = [{"thread": t, "rollout": str(p)} for t, p in children]
            paths = {}
            for thread in threads:
                path = self.codex_rollout(thread) or dict(children).get(thread)
                if not path:
                    record["missing"].append(thread)
                    continue
                paths[str(path)] = (thread, path)
            for thread, path in children:
                paths.setdefault(str(path), (thread, path))
            threads = list(dict.fromkeys(threads + [t for t, _ in children]))
            record["children"] = threads
            results = {}
            for thread, path in paths.values():
                record["files"].append(str(path))
                results.update(self.transcript_results(path))
                for event in self.transcript_events(path):
                    found = self.rollout_call(event)
                    if found and "herdr-threads" in found[1]:
                        record["calls"].append({"id": found[0], "command": found[1], "source": path.name, "parent": thread})
            record["complete"] = bool(record["spawns"]) and bool(threads) and not record["missing"]
        for call in record["calls"]:
            outcome = results.get(call["id"])
            call["result_error"] = None if outcome is None else outcome["error"]
            call["result_excerpt"] = None if outcome is None else outcome["text"][:300]
        (self.ev / f"child-sidechain-{phase}.json").write_text(json.dumps(record, indent=2))
        return record

    @staticmethod
    def rollout_child_call(event):
        """(call_id, name) of a Codex rollout collab/subagent tool call (`function_call` / `custom_tool_call` whose name
        says spawn/collab/subagent), else None."""
        payload = event.get("payload") if isinstance(event.get("payload"), dict) else {}
        if payload.get("type") in ("function_call", "custom_tool_call") and CODEX_CHILD_ITEMS.search(str(payload.get("name") or "")):
            return payload.get("call_id") or payload.get("id"), str(payload.get("name"))
        return None

    def codex_session_meta(self, path):
        """The `session_meta` payload of a Codex rollout (its first such line), read-only; {} when absent."""
        try:
            with open(path, "r", encoding="utf-8", errors="replace") as stream:
                for _, line in zip(range(20), stream):
                    try:
                        entry = json.loads(line)
                    except ValueError:
                        continue
                    if isinstance(entry, dict) and entry.get("type") == "session_meta":
                        payload = entry.get("payload")
                        return payload if isinstance(payload, dict) else {}
        except OSError:
            pass
        return {}

    def codex_child_rollouts(self, root, since=None):
        """[(thread_id, path)] of every Codex rollout under CODEX_HOME written since `since` whose session_meta names
        `root` as parent_thread_id or forked_from_id (at any depth of the payload). Read-only."""
        sessions = self.codex_home() / "sessions"
        if not root or not sessions.is_dir():
            return []
        floor = datetime.datetime.fromisoformat(since).timestamp() if since else 0
        found = []
        for path in sorted(sessions.glob("**/rollout-*.jsonl")):
            try:
                if path.stat().st_mtime < floor:
                    continue
            except OSError:
                continue
            meta = self.codex_session_meta(path)
            if root in (find_value(meta, "parent_thread_id"), find_value(meta, "forked_from_id")) and meta.get("id") != root:
                found.append((str(meta.get("id") or path.stem), path))
        return found

    def s_child_read(self, phase):
        result = self.phase_result(phase)
        sidechain = (result or {}).get("sidechain")
        if sidechain is None:
            return FAIL, f"no verification result for phase {phase}", None
        if not sidechain["spawns"]:
            return NOT_EXERCISED, "the model did not delegate to any subagent; child read was not exercised", None
        reads = [c for c in sidechain["calls"] if READ_VERBS.search(c["command"]) and not mutating_call(c["command"])]
        if not reads:
            if sidechain["complete"]:
                return NOT_EXERCISED, f"{len(sidechain['spawns'])} subagent(s) ran but made no herdr-threads read call", None
            return UNVERIFIED, f"subagent(s) {sidechain['spawns'][:3]} ran but their sidechain was not recorded ({sidechain['missing'][:3]})", None
        refused = [c for c in reads if c["result_error"]]
        if len(refused) == len(reads):
            return FAIL, f"every child read was refused: {[c['command'][:120] for c in refused[:3]]}", None
        return PASS, (f"child read allowed: {len(reads) - len(refused)} child herdr-threads read call(s) succeeded "
                      f"(first `{reads[0]['command'][:120]}`, source {reads[0]['source']})"), None

    def s_child_no_ack(self, phase):
        """Child ACK/accept absence (cooperative, transcript level): the complete sidechain shows no successful child
        write. The DB ACK execution = root binding execution comparison is reported but does not discriminate (B1)."""
        result = self.phase_result(phase)
        sidechain = (result or {}).get("sidechain")
        if sidechain is None:
            return FAIL, f"no verification result for phase {phase}", None
        if not sidechain["spawns"]:
            return NOT_EXERCISED, "the model did not delegate to any subagent; child ACK absence not exercised", None
        mutations = [c for c in sidechain["calls"] if mutating_call(c["command"])]
        succeeded = [c for c in mutations if c["result_error"] is False]
        if succeeded:
            return FAIL, f"child issued herdr-threads write(s) that succeeded: {[c['command'][:160] for c in succeeded[:3]]}", None
        ack_exec = (result.get("provenance") or {}).get("ack_execution")
        root_exec = result.get("root_binding_execution_diagnostic")
        if ack_exec and root_exec and ack_exec != root_exec:
            return FAIL, f"DB ACK execution {ack_exec} is not the root binding execution {root_exec}", None
        if not sidechain["complete"]:
            return UNVERIFIED, (f"sidechain incomplete (missing {sidechain['missing'][:3]}); a child ACK would be stored with the "
                                f"root's seat/provenance/execution (cooperative limit), so absence stays UNVERIFIED"), None
        unknown = [c for c in mutations if c["result_error"] is None]
        if unknown:
            return UNVERIFIED, f"child mutating call(s) without a recorded result: {[c['command'][:120] for c in unknown[:3]]}", None
        refused = f"; {len(mutations)} child attempt(s) refused" if mutations else ""
        return PASS, (f"sidechain complete ({len(sidechain['spawns'])} spawn(s), {len(sidechain['calls'])} child herdr-threads "
                      f"call(s)): no child write succeeded{refused} (transcript level); DB ACK execution {ack_exec or 'n/a'} "
                      f"= root binding execution {root_exec or 'n/a'} (consistency only; does not discriminate under B1)"), None

    def s_top_level_ack(self, phase):
        result = self.phase_result(phase)
        if not result:
            return FAIL, f"no verification result for phase {phase}", None
        receipt = result.get("receipt") or {}
        if receipt.get("state") != "acked" or not result.get("root_ack_calls"):
            return FAIL, f"top-level ACK missing: receipt {receipt.get('state')!r}, root ack calls {len(result.get('root_ack_calls') or [])}", None
        if receipt.get("ack_actor_seat_id") not in (None, self.facts.get("agent_seat")):
            return FAIL, f"ACK actor {receipt.get('ack_actor_seat_id')} is not the agent seat", None
        return PASS, f"top-level ACK of {result['message']} by root call {result['root_ack_calls'][0][0]} (provenance {result['provenance'].get('ack')})", None

    # ----- midturn (tool-boundary arrival) -----
    def launch_tick(self, phase, transcript):
        """Called while a launch runs. --scenario midturn: once the model's first tool call appears, send one new
        require-ACK message (exactly once per run)."""
        if "midturn" not in self.scenarios or phase != "initial" or self.facts.get("midturn"):
            return
        call = self.first_tool_call(transcript or self.live_transcript(phase))
        if not call:
            return
        self.facts["midturn"] = {"after_call": call, "sent_utc": utc()}
        status, detail, message = self.send("midturn")
        self.facts["midturn"].update({"send_status": status, "detail": detail, "message": message})
        print(f"[INFO   ] SM1 mid-turn send after tool call {call}: {status} {detail}", flush=True)

    def live_transcript(self, phase):
        """The transcript a launch without its own output file writes (managed launch / TUI)."""
        if self.harness == "claude":
            session = self.facts.get("sessions", {}).get(phase)
            return self.claude_session_transcript(session) if session else None
        return self.codex_rollout_since(self.facts.get("phase_started_utc", {}).get(phase))

    def codex_tui_transcript(self, phase):
        """Interactive Codex: the root rollout of this phase, read-only. A resumed phase reuses the known thread; a new
        conversation (initial, restart, /new clear) is the one root rollout of this project written since the phase
        started whose thread is not an earlier phase's. The thread id found becomes the phase's session."""
        sessions = self.facts.setdefault("sessions", {})
        if sessions.get(phase):
            return self.codex_rollout(sessions[phase])
        known = {s for p, s in sessions.items() if s and p != phase}
        found = [p for p in self.codex_root_rollouts(self.facts.get("phase_started_utc", {}).get(phase))
                 if self.codex_session_meta(p).get("id") not in known]
        if len(found) != 1:
            return None
        thread = self.codex_session_meta(found[0]).get("id")
        if thread:
            sessions[phase] = thread
            self.facts["codex_session"] = thread
        return found[0]

    def codex_root_rollouts(self, since):
        """Every ROOT rollout (session_meta cwd is this scratch project; no parent_thread_id / forked_from_id) written
        since `since`. Read-only."""
        sessions = self.codex_home() / "sessions"
        floor = datetime.datetime.fromisoformat(since).timestamp() if since else 0
        projects = {str(self.project), str(self.project.resolve())}
        found = []
        for path in sorted(sessions.glob("**/rollout-*.jsonl")) if sessions.is_dir() else []:
            if path.stat().st_mtime < floor:
                continue
            meta = self.codex_session_meta(path)
            if find_value(meta, "parent_thread_id") or find_value(meta, "forked_from_id"):
                continue
            if find_value(meta, "cwd") in projects:
                found.append(path)
        return found

    def codex_rollout_since(self, since):
        """D6 (Codex matrix 1): the ROOT rollout of a launch whose exec stream the driver does not hold (managed launch):
        the one rollout written since `since` whose session_meta cwd is this scratch project and that is not a child
        (no parent_thread_id / forked_from_id). Read-only; None unless exactly one matches."""
        found = self.codex_root_rollouts(since)
        return found[0] if len(found) == 1 else None

    def s_midturn_sent(self):
        midturn = self.facts.get("midturn")
        if not midturn:
            return NOT_EXERCISED, "no top-level tool call was observed during the launch; the mid-turn message was never sent", None
        if midturn.get("send_status") != PASS:
            return FAIL, f"mid-turn send failed: {midturn.get('detail')}", None
        return PASS, f"require-ACK {midturn['message']} sent at {midturn['sent_utc']} after the model's first tool call {midturn['after_call']}", None

    def midturn_deliveries(self, since):
        """PreToolUse additionalContext entries recorded at or after `since` (Claude transcript attachments,
        Codex rollout developer messages after the prompt)."""
        path = self.claude_session_transcript(self.facts.get("sessions", {}).get("initial") or "") if self.harness == "claude" \
            else self.codex_rollout(self.facts.get("sessions", {}).get("initial"))
        floor = datetime.datetime.fromisoformat(since)
        deliveries = []
        seen_user = False
        for entry in self.transcript_events(path):
            try:
                stamp = datetime.datetime.fromisoformat(str(entry.get("timestamp", "")).replace("Z", "+00:00"))
            except ValueError:
                stamp = None
            if self.harness == "claude":
                attachment = entry.get("attachment")
                if entry.get("type") != "attachment" or not isinstance(attachment, dict) or attachment.get("type") != "hook_additional_context":
                    continue
                if attachment.get("hookEvent") != "PreToolUse" or stamp is None or stamp < floor:
                    continue
                content = attachment.get("content")
                deliveries.append("\n".join(content) if isinstance(content, list) else str(content or ""))
            else:
                message = self.rollout_message(entry)
                if not message:
                    continue
                role, text = message
                seen_user |= role == "user" and self.prompt() in text
                if role == "developer" and seen_user and stamp is not None and stamp >= floor and HOOK_PREAMBLE in text:
                    deliveries.append(text)
        return path, deliveries

    def s_midturn_delivery(self):
        midturn = self.facts.get("midturn") or {}
        message = midturn.get("message")
        if not message:
            return NOT_EXERCISED, "mid-turn message not sent", None
        transcript = self.phase_transcript("initial")
        calls = self.root_tool_calls(transcript) or self.root_tool_calls(self.live_transcript("initial"))
        ids = [c[0] for c in calls]
        later = ids[ids.index(midturn["after_call"]) + 1:] if midturn["after_call"] in ids else []
        path, deliveries = self.midturn_deliveries(midturn["sent_utc"])
        naming = [d for d in deliveries if message in d]
        record = {"message": message, "sent_utc": midturn["sent_utc"], "transcript": str(path) if path else None,
                  "tool_calls_after_send": later, "deliveries": [d[:600] for d in deliveries]}
        (self.ev / "midturn-delivery.json").write_text(json.dumps(record, indent=2))
        self.facts["midturn"]["delivery"] = "additional_context" if deliveries else None
        if naming:
            return PASS, f"PreToolUse additionalContext named {message} {len(naming)}x after the send (evidence midturn-delivery.json)", None
        if deliveries:
            return PASS, (f"PreToolUse additionalContext delivered {len(deliveries)}x after the send (digest form; "
                          f"{message} not named verbatim); evidence midturn-delivery.json"), None
        acked = [c for c in self.transcript_calls(transcript or self.live_transcript("initial"))
                 if not c[2] and re.search(r"\back\b", c[1]) and message in c[1]]
        if acked:
            self.facts["midturn"]["delivery"] = "behaviour_inferred"
            return PASS, (f"no PreToolUse additionalContext recorded after the send (Claude 2.1.285 may not record PreToolUse), "
                          f"but the model ACKed {message} in the same turn: delivery inferred from model behaviour"), None
        if not later:
            return NOT_EXERCISED, "the model made no tool call after the mid-turn send, so no PreToolUse hook could deliver it", None
        return UNVERIFIED, (f"{len(later)} tool call(s) after the send, but no PreToolUse additionalContext is recorded and the "
                            f"model did not ACK {message}; delivery UNVERIFIED"), None

    def s_midturn_ack(self):
        midturn = self.facts.get("midturn") or {}
        message = midturn.get("message")
        if not message:
            return NOT_EXERCISED, "mid-turn message not sent", None
        seat = self.facts["agent_seat"]
        transcript = self.phase_transcript("initial") or self.live_transcript("initial")
        deadline = time.monotonic() + (0 if self.dry else self.args.verify_wait)
        while True:
            receipt = {r["message_id"]: r for r in self.receipts_for(seat)}.get(message)
            if (receipt or {}).get("state") == "acked" or time.monotonic() >= deadline:
                break
            time.sleep(2)
        root_acks = [c for c in self.transcript_calls(transcript) if not c[2] and re.search(r"\back\b", c[1]) and message in c[1]]
        obs = json.loads((receipt or {}).get("ack_observation") or "{}")
        self.phase_results.append({"phase": "midturn", "message": message, "root_ack_calls": root_acks, "receipt": receipt,
                                   "provenance": {"ack": obs.get("provenance"), "accept": None, "ack_execution": obs.get("execution")},
                                   "child_check": "no_subagent_activity" if not self.subagent_activity(transcript) else "unverified_subagent_activity",
                                   "problems": []})
        if not receipt or receipt["state"] != "acked":
            # D1 (Claude matrix 1): an unACKed mid-turn message whose "watch for new mail" instruction the model never
            # saw is NOT_EXERCISED; an ACK that exists is always judged in full below.
            return self.gate_on_instruction("midturn", (
                FAIL, f"mid-turn message {message} not ACKed (state {(receipt or {}).get('state')!r})", None))
        if receipt["ack_actor_seat_id"] != seat or obs.get("provenance") not in MODEL_PROVENANCE:
            return FAIL, f"mid-turn ACK actor/provenance {receipt['ack_actor_seat_id']}/{obs.get('provenance')!r} not a top-level model class", None
        if not root_acks:
            return FAIL, f"mid-turn {message} ACKed in SQLite but the transcript has no root `herdr-threads ack {message}` call", None
        return PASS, f"model ACKed mid-turn {message} by root call {root_acks[0][0]} (provenance {obs.get('provenance')})", None

    # ----- warning (missed deadline) -----
    def warning_message(self, phase):
        """The message the warning scenario judges: the handoff of `phase`, or (phase `warning`, TUI) the separate
        short-deadline message, which is kept out of `messages` because it is not a handoff the manifest pairs."""
        return self.facts.get("warning_message") if phase == "warning" else self.facts["messages"][phase]

    def send_warning_message(self):
        """Send the separate short-deadline message (tui D1) and record it apart from the handoffs."""
        status, detail, message = self.send("warning")
        self.facts.get("messages", {}).pop("warning", None)
        if status == PASS:
            self.facts["warning_message"] = message
            self.facts["warning_sent_utc"] = utc()
        return status, detail, message

    def s_warning_idle_send(self):
        """tui D1: once the TUI agent is idle after the initial ACK, send one short-deadline require-ACK message, then
        poll (bounded: deadline + the first 30 s retry step + 30 s) for the durable warning and a wake reservation
        covering it, capturing the pane (the wake marker) at the end. SW1/SW2 judge what this observed."""
        if self.dry:
            raise Blocked("live run only: the idle TUI agent is the wake target")
        pane = self.facts["agent_pane"]
        # Herdr reports a Claude TUI whose turn finished (output not yet seen) as `done`, not `idle`: both mean the
        # agent awaits input, and both are what the production wake targets (native-claude-tui-2 waited for `idle`
        # alone and timed out on `done`).
        rc, data, out, err = self.herdr("agent", "wait", pane, "--until", "idle", "--until", "done", "--timeout", "60000",
                                        tag="tui:wait-idle:warning", timeout=70)
        state = find_key(data, ["agent_status", "status", "state"]) if data else None
        if rc != 0 or (state and state not in IDLE_STATES):
            return NOT_EXERCISED, f"the TUI agent never reported idle (rc={rc}, state {state!r}); no idle wake could be owed", None
        status, detail, message = self.send_warning_message()
        if status != PASS:
            return FAIL, f"warning message send failed: {detail}", None
        self.facts["warning_safe_prompt"] = self.safe_prompt_state("warning")
        limit = time.monotonic() + self.args.warning_deadline + 60
        covered = False
        while not covered:
            time.sleep(WARNING_POLL_S)
            warns = self.query("SELECT decision_seq FROM messages WHERE kind='warn' AND source_message_id=?", (message,))
            rows = self.query("SELECT last_warning_seq FROM wake_work WHERE seat_id=?", (self.facts["agent_seat"],))
            covered = bool(warns and rows and rows[0]["last_warning_seq"] is not None
                           and rows[0]["last_warning_seq"] >= min(w["decision_seq"] for w in warns))
            if time.monotonic() >= limit:
                break
        time.sleep(WAKE_SETTLE_S if covered else 0)  # let the woken agent show the marker before the capture
        self.capture_pane("warning")
        return PASS, (f"idle agent; sent {message} with a {self.args.warning_deadline}s deadline; wake reservation "
                      f"{'observed' if covered else 'not observed'} within the bound (judged by SW1/SW2)"), None

    def s_warning_event(self, phase="initial"):
        message, seat = self.warning_message(phase), self.facts["agent_seat"]
        state = self.query("SELECT state, available_at, deadline_at, acked_at, warning_message_id FROM receipt_state "
                           "WHERE message_id=? AND seat_id=?", (message, seat))
        state = state[0] if state else {}
        warns = self.query("SELECT id, decision_seq, decision_at, source_message_id FROM messages WHERE kind='warn' AND "
                           "(source_message_id=? OR id=?)", (message, state.get("warning_message_id") or "\0"))
        self.facts["warning"] = {"receipt": state, "warnings": warns}
        (self.ev / f"warning-{phase}.json").write_text(json.dumps(self.facts["warning"], indent=2, default=str))
        deadline_at, acked_at = state.get("deadline_at"), state.get("acked_at")
        if deadline_at is None:
            return FAIL, f"receipt {message} never started its deadline timer (receipt_state {state or 'absent'})", None
        if acked_at is not None and acked_at <= deadline_at and not warns:
            return NOT_EXERCISED, f"the model ACKed {message} before its {self.facts['message_deadlines'].get(phase)}s deadline; no warning due", None
        if len(warns) == 1:
            linked = "" if state.get("warning_message_id") == warns[0]["id"] else f" (receipt warning link {state.get('warning_message_id')!r})"
            late = f"; late ACK at {acked_at} kept" if acked_at else "; receipt still pending"
            return PASS, f"exactly one durable warning {warns[0]['id']} (decision seq {warns[0]['decision_seq']}) for the missed deadline{linked}{late}", None
        if not warns:
            return FAIL, f"deadline {deadline_at} passed (acked_at {acked_at}) but no durable warning exists for {message}", None
        return FAIL, f"{len(warns)} warnings for one missed deadline (duplicates): {[w['id'] for w in warns]}", None

    def s_warning_wake(self):
        warning = (self.facts.get("warning") or {}).get("warnings") or []
        if len(warning) != 1:
            return NOT_EXERCISED, "no single durable warning to wake for", None
        seq = warning[0]["decision_seq"]
        rows = self.query("SELECT reason_bits, last_reserved_at_utc, last_receipt_seq, last_warning_seq, last_outcome, retry_step "
                          "FROM wake_work WHERE seat_id=?", (self.facts["agent_seat"],))
        offer = self.query("SELECT offered_through_seq FROM warning_offer WHERE seat_id=?", (self.facts["agent_seat"],))
        markers = {p.name: p.read_text(errors="replace").count(WAKE_MARKER) for p in sorted(self.ev.glob("pane-*.txt"))}
        record = {"warning_seq": seq, "wake_work": rows, "warning_offer": offer, "marker_counts": markers}
        (self.ev / "warning-wake.json").write_text(json.dumps(record, indent=2, default=str))
        storm = {name: n for name, n in markers.items() if n > 1}
        if storm:
            return FAIL, f"wake marker repeated in one pane capture (not coalesced): {storm}", None
        row = rows[0] if rows else {}
        if row.get("last_warning_seq") is not None and row["last_warning_seq"] >= seq:
            both = " together with the pending receipt" if row.get("last_receipt_seq") is not None else ""
            seen = f"; marker seen {sum(markers.values())}x" if any(markers.values()) else ""
            return PASS, (f"one coalesced wake reservation (per-seat wake_work row, outcome {row.get('last_outcome')!r}) covered "
                          f"warning seq {seq}{both}{seen}"), None
        offered = offer[0]["offered_through_seq"] if offer else None
        if offered is not None and offered >= seq:  # codex D2: nothing was owed, so the wake claim was not tested
            return NOT_EXERCISED, (f"warning seq {seq} was already offered at a verified check-in (offered_through_seq "
                                   f"{offered}), so no wake was owed; the coalesced wake was not exercised"), None
        state = self.facts.get("warning_safe_prompt") or self.facts.get("safe_prompt")
        if self.wake_unsupported({"safe_prompt": state}):
            return NOT_EXERCISED, (f"no wake reservation covered warning seq {seq} and health reports safe_prompt unsupported: "
                                   f"the idle wake is unavailable in this build (ht-4is.5.6); coalesced wake not exercised"), None
        return UNVERIFIED, (f"coalesced wake UNVERIFIED: no wake reservation covered warning seq {seq} (wake_work "
                            f"{row or 'absent'}); a working or exited print-mode agent is not an idle wake target"), None

    # ----- safe wake evidence: health, lostprompt, blockedui (ht-4is.11.12) -----
    @staticmethod
    def parse_safe_prompt(text):
        """`host.safe_prompt` of a `daemon health --json` output: supported / unsupported / unknown (absent)."""
        try:
            value = find_value(json.loads(text), "safe_prompt") if text and text.strip() else None
        except ValueError:
            value = None
        return value.lower() if isinstance(value, str) and value else "unknown"

    def safe_prompt_state(self, label):
        """The daemon's current health claim for the host safe prompt (the wake capability), recorded per use. A claim
        only: verdicts are decided by the wake rows and pane captures observed, this only names why nothing happened."""
        rc, out, _ = self.ht("daemon", "health", tag=f"daemon:health-{label}", timeout=20)
        state = self.parse_safe_prompt(out) if rc == 0 else "unknown"
        self.facts.setdefault("safe_prompt_observations", []).append({"label": label, "rc": rc, "safe_prompt": state, "utc": utc()})
        self.facts["safe_prompt"] = state
        return state

    def s_safe_prompt_health(self):
        state = self.safe_prompt_state("scenario")
        note = {"supported": "the product claims it can prompt an idle agent",
                "unsupported": "wake-dependent verdicts become NOT_EXERCISED unless a delivered wake is observed anyway",
                }.get(state, "capability not reported")
        return INFO, f"daemon health host.safe_prompt={state}: {note}", None

    def wake_row(self):
        """The agent seat's wake_work row (read-only), {} when absent."""
        rows = self.query("SELECT * FROM wake_work WHERE seat_id=?", (self.facts["agent_seat"],))
        return rows[0] if rows else {}

    @staticmethod
    def new_wake(before, after):
        """True when `after` shows a wake reservation made since `before` was read (reserve sets last_reservation_id)."""
        return bool((after or {}).get("last_reservation_id")) and (after or {}).get("last_reservation_id") != (before or {}).get("last_reservation_id")

    def pane_markers(self, name):
        try:
            return (self.ev / f"pane-{name}.txt").read_text(errors="replace").count(WAKE_MARKER)
        except OSError:
            return 0

    def lostprompt_delivered(self):
        """Observed delivery of the idle recovery wake: the marker on the agent's screen, or a `submitted` outcome."""
        after = (self.facts.get("lostprompt") or {}).get("wake_after") or {}
        return self.pane_markers("lostprompt") > 0 or after.get("last_outcome") == "submitted"

    def wake_unsupported(self, observation):
        """True (and recorded) when health reported the safe prompt unsupported for an observation that saw no
        delivered wake: the wake claim was not tested (NOT_EXERCISED, manifest safe_wake_unsupported)."""
        if (observation or {}).get("safe_prompt") == "unsupported":
            self.facts["safe_wake_unsupported"] = True
            return True
        return False

    def s_lostprompt_wake(self):
        """SL1: the product reserved an idle recovery wake for the un-prompted agent, covering its pending handoff."""
        lp = self.facts.get("lostprompt")
        if not lp:
            return FAIL, "the lostprompt launch recorded no observation", None
        before, after = lp.get("wake_before") or {}, lp.get("wake_after") or {}
        (self.ev / "lostprompt-wake.json").write_text(json.dumps({"before": before, "after": after, "safe_prompt": lp.get("safe_prompt")},
                                                                 indent=2, default=str))
        if self.new_wake(before, after):
            covered = [k[5:-4] for k in ("last_receipt_seq", "last_invitation_seq") if after.get(k) is not None]
            if covered:
                return PASS, (f"idle recovery wake reservation {after['last_reservation_id']} for the un-prompted agent covered its "
                              f"pending {' + '.join(covered)} (outcome {after.get('last_outcome')!r}, reason bits {after.get('reason_bits')})"), None
            return FAIL, f"a wake reservation {after['last_reservation_id']} covered no pending receipt or invitation: {after}", None
        if self.wake_unsupported(lp):
            return NOT_EXERCISED, (f"health reports host safe_prompt unsupported and no wake reservation was observed in "
                                   f"{lp.get('waited_s')}s: the idle recovery wake is unavailable in this build (ht-4is.5.6); "
                                   f"lost-prompt recovery not exercised"), None
        return FAIL, (f"no idle recovery wake reservation for the un-prompted agent within {lp.get('waited_s')}s although health "
                      f"reports safe_prompt {lp.get('safe_prompt')!r} (wake_work {after or 'absent'})"), None

    def s_lostprompt_delivery(self):
        """SL2: the reserved wake reached the agent: the fixed marker on its screen exactly once."""
        lp = self.facts.get("lostprompt") or {}
        after = lp.get("wake_after") or {}
        markers, outcome = self.pane_markers("lostprompt"), after.get("last_outcome")
        if markers > 1:
            return FAIL, f"wake marker {markers}x in one pane capture (not coalesced)", None
        if markers == 1:
            return PASS, f"wake marker on the agent's screen once (reservation outcome {outcome!r})", None
        if outcome == "submitted":
            return UNVERIFIED, "reservation outcome `submitted` but the marker is not in pane-lostprompt.txt", None
        if self.wake_unsupported(lp):
            return NOT_EXERCISED, (f"wake reserved (outcome {outcome!r}) but health reports safe_prompt unsupported and no "
                                   f"marker reached the screen: the prompt capability is unavailable (ht-4is.5.6)"), None
        return FAIL, f"the wake never reached the agent: outcome {outcome!r}, no marker in pane-lostprompt.txt", None

    def s_lostprompt_verify(self, transcript):
        """S18 under lostprompt: an ACK missing because nothing could prompt the agent (safe prompt unsupported, no
        delivered wake observed) is NOT_EXERCISED; with a delivered wake, or a claimed capability, it stays a FAIL."""
        status, detail, value = self.s_wait_and_verify("initial", transcript)
        if status == FAIL and not self.lostprompt_delivered() and self.wake_unsupported(self.facts.get("lostprompt")):
            return NOT_EXERCISED, (f"nothing prompted the un-prompted agent (health safe_prompt unsupported, no delivered wake); "
                                   f"the ACK after recovery was not exercised: {detail}"), value
        return status, detail, value

    def blocked_state(self, tag, timeout_ms):
        """(blocked?, state): whether Herdr reports the agent pane blocked within `timeout_ms`."""
        rc, data, out, err = self.herdr("agent", "wait", self.facts["agent_pane"], "--until", "blocked", "--timeout", str(timeout_ms),
                                        tag=tag, timeout=timeout_ms // 1000 + 10)
        state = find_key(data, ["status", "state"]) if data else None
        return rc == 0 and (state == "blocked" or (state is None and "blocked" in (out or ""))), state or (f"rc={rc}" if rc else None)

    def s_blockedui_enter(self):
        """SU0 (c): put the idle agent into a recognized approval UI: ask it to run a command its harness must ask
        approval for (Claude default permission mode; Codex workspace-write sandbox, approval on-request). The target
        is on the read-only system volume, so even an approval could not create it; the driver NEVER answers."""
        if self.dry:
            raise Blocked("live run only: a real harness approval UI is the blocked state")
        pane = self.facts["agent_pane"]
        rc, data, _, _ = self.herdr("agent", "wait", pane, "--until", "idle", "--until", "done", "--timeout", "60000",
                                    tag="tui:wait-idle:blockedui", timeout=70)
        state = find_key(data, ["status", "state"]) if data else None
        if rc != 0 or (state and state not in IDLE_STATES):
            return NOT_EXERCISED, f"the agent never reported idle before the blocked-UI prompt (rc={rc}, state {state!r})", None
        target = Path("/") / f"ht-blockedui-{secrets.token_hex(3)}"
        record = {"target": str(target), "started_utc": utc()}
        self.facts["blockedui"] = record
        rc, _, _, err = self.herdr("agent", "prompt", pane, BLOCKEDUI_PROMPT.format(path=target), tag="tui:prompt:blockedui", timeout=40)
        if rc != 0:  # no --wait: the turn stays open on the dialog
            return FAIL, f"blocked-UI prompt not submitted: rc={rc} {(err or '').strip()[:200]}", None
        blocked, state = self.blocked_state("tui:wait-blocked:blockedui", min(self.args.timeout, 180) * 1000)
        screen = self.capture_pane("blockedui-entered")
        ui = APPROVAL_UI.search(screen or "")
        record.update({"herdr_blocked": blocked, "state": state, "approval_ui": ui.group(0) if ui else None})
        if target.exists():
            return FAIL, f"{target} exists: the command ran without an approval UI", None
        if blocked and ui:
            return PASS, f"Herdr reports blocked and the screen shows an approval UI ({ui.group(0)!r}); nothing answered", None
        if blocked:
            return NOT_EXERCISED, "Herdr reports blocked but no recognized approval UI is on screen (pane-blockedui-entered.txt)", None
        return NOT_EXERCISED, f"the agent never entered an approval UI within the bound (state {state!r}); blocked-UI wake not exercised", None

    def s_blockedui_queue(self):
        """SU1: queue a short-deadline require-ACK message for the blocked agent, then observe for deadline + 60 s."""
        status, detail, message = self.send("blockedui")
        self.facts.get("messages", {}).pop("blockedui", None)  # not a handoff the manifest pairs
        if status != PASS:
            return FAIL, f"blocked-UI message send failed: {detail}", None
        record = self.facts["blockedui"]
        record.update({"message": message, "sent_utc": utc(), "wake_before": self.wake_row(),
                       "safe_prompt": self.safe_prompt_state("blockedui")})
        limit = time.monotonic() + self.args.warning_deadline + 60
        while time.monotonic() < limit:
            time.sleep(WARNING_POLL_S)
        record["warnings"] = self.query("SELECT id, decision_seq FROM messages WHERE kind='warn' AND source_message_id=?", (message,))
        record["wake_after"] = self.wake_row()
        record["still_blocked"], record["state_after"] = self.blocked_state("tui:wait-blocked:blockedui-after", 3000)
        self.capture_pane("blockedui-after")
        return PASS, (f"queued {message} ({self.args.warning_deadline}s deadline) while blocked; observed "
                      f"{self.args.warning_deadline + 60}s; {len(record['warnings'])} warning(s)"), None

    def s_blockedui_no_injection(self):
        """SU2: nothing was injected into the blocked agent and the wake stayed pending. FAIL on a marker on screen, a
        `submitted` wake outcome, or an ACK of the queued message; NOT_EXERCISED when no wake was attempted because the
        host reports safe_prompt unsupported (the claim was not tested)."""
        record = self.facts.get("blockedui") or {}
        before, after = record.get("wake_before") or {}, record.get("wake_after") or {}
        receipt = {r["message_id"]: r for r in self.receipts_for(self.facts["agent_seat"])}.get(record.get("message")) or {}
        markers = self.pane_markers("blockedui-after")
        try:
            screen = (self.ev / "pane-blockedui-after.txt").read_text(errors="replace")
        except OSError:
            screen = ""
        attempted = self.new_wake(before, after)
        record.update({"receipt_state": receipt.get("state"), "markers": markers, "wake_attempted": attempted,
                       "approval_ui_after": bool(APPROVAL_UI.search(screen))})
        (self.ev / "blockedui.json").write_text(json.dumps(record, indent=2, default=str))
        problems = []
        if markers:
            problems.append(f"wake marker injected into the blocked pane ({markers}x)")
        if attempted and after.get("last_outcome") == "submitted":
            problems.append(f"wake reservation {after.get('last_reservation_id')} submitted a prompt while the agent was blocked")
        if receipt.get("state") == "acked":
            problems.append(f"{record.get('message')} was ACKed while the agent sat in the approval UI")
        if problems:
            return FAIL, "; ".join(problems), None
        if not record.get("still_blocked") or not record["approval_ui_after"]:
            return UNVERIFIED, (f"the agent left the approval UI during the window (state {record.get('state_after')!r}, UI on "
                                f"screen {record['approval_ui_after']}); no-injection cannot be judged"), None
        if not attempted and self.wake_unsupported(record):
            return NOT_EXERCISED, ("health reports safe_prompt unsupported and no wake was attempted: no-injection while blocked "
                                   "holds trivially and was not exercised"), None
        wake = (f"wake reservation {after.get('last_reservation_id')} ended {after.get('last_outcome')!r} (not submitted)" if attempted
                else f"no wake reservation while blocked (wake stays pending, reason bits {after.get('reason_bits')})")
        return PASS, f"{wake}; no marker on screen; approval UI still up; {record.get('message')} still {receipt.get('state')!r}", None

    # ----- children (two concurrent subagents) -----
    def children_overlap(self, phase, sidechain):
        """(overlap, how): whether at least two subagents ran at the same time. Claude: a spawn issued before another
        spawn's result came back (root transcript order). Codex: child rollouts whose timestamp spans intersect."""
        if self.harness == "claude":
            events = self.transcript_events(self.phase_transcript(phase))
            start, end = {}, {}
            for index, event in enumerate(events):
                message = event.get("message")
                content = message.get("content") if isinstance(message, dict) else None
                for block in content if isinstance(content, list) else []:
                    if not isinstance(block, dict):
                        continue
                    if block.get("type") == "tool_use" and block.get("id") in sidechain["spawns"]:
                        start.setdefault(block["id"], index)
                    elif block.get("type") == "tool_result" and block.get("tool_use_id") in sidechain["spawns"]:
                        end.setdefault(block["tool_use_id"], index)
            spans = [(start[s], end.get(s, len(events))) for s in sidechain["spawns"] if s in start]
            how = "root transcript spawn/result order"
        else:
            spans = []
            for path in sidechain.get("files", []):
                stamps = []
                for event in self.transcript_events(path):
                    try:
                        stamps.append(datetime.datetime.fromisoformat(str(event.get("timestamp", "")).replace("Z", "+00:00")))
                    except ValueError:
                        continue
                if stamps:
                    spans.append((min(stamps), max(stamps)))
            how = "child rollout timestamp spans"
        if len(spans) < 2:
            return None, f"{how}: fewer than two child spans recorded"
        spans.sort()
        reach = spans[0][1]
        for begin, finish in spans[1:]:
            if begin < reach:
                return True, how
            reach = max(reach, finish)
        return False, how

    def s_children_concurrent(self, phase="initial"):
        """SK1 (d): at least two subagents ran concurrently and at least two of them read mail."""
        result = self.phase_result(phase)
        sidechain = (result or {}).get("sidechain")
        if sidechain is None:
            return FAIL, f"no verification result for phase {phase}", None
        children = sidechain.get("children") or sidechain["spawns"]
        if len(children) < 2:
            return NOT_EXERCISED, f"the model started {len(children)} subagent(s); two concurrent children not exercised", None
        overlap, how = self.children_overlap(phase, sidechain)
        if overlap is False:
            return NOT_EXERCISED, f"{len(children)} subagents ran one after the other ({how}); concurrency not exercised", None
        if overlap is None:
            return UNVERIFIED, f"concurrency of {len(children)} subagents could not be established ({how})", None
        reads = [c for c in sidechain["calls"] if READ_VERBS.search(c["command"]) and not mutating_call(c["command"])]
        readers = {c.get("parent") or c.get("source") for c in reads if c.get("result_error") is False}
        if len(readers) >= 2:
            return PASS, f"{len(children)} concurrent subagents ({how}); {len(readers)} of them read mail successfully", None
        if not sidechain["complete"]:
            return UNVERIFIED, f"concurrent subagents ran but their sidechain was not fully recorded ({sidechain['missing'][:3]})", None
        return NOT_EXERCISED, f"{len(children)} concurrent subagents ran but only {len(readers)} read mail successfully", None

    # ----- burst (more than one inbox page) -----
    def s_burst_setup(self):
        created = []
        for index in range(self.args.burst_threads):
            rc, out, err = self.ht("thread", "create", "--topic", f"burst {index + 1} {self.run_id[-10:]}", "--goal", "Inbox paging burst",
                                   tag=f"coordinator:burst-thread:{index}", pane=self.facts["coordinator_pane"], coop=True)
            thread = find_key(json.loads(out), ["thread_id", "thread", "id", "data"]) if rc == 0 and out.strip() else None
            if not thread:
                return FAIL, f"burst thread {index + 1} create rc={rc}: {(err or out).strip()[:200]}", None
            rc, out, err = self.ht("invite", thread, "--seat", self.facts["agent_seat"], "--deadline", str(self.args.deadline),
                                   tag=f"coordinator:burst-invite:{index}", pane=self.facts["coordinator_pane"], coop=True)
            if rc != 0:
                return FAIL, f"burst invite {index + 1} rc={rc}: {(err or out).strip()[:200]}", None
            created.append(thread)
        self.facts["burst_threads"] = created
        rows = self.query("SELECT COUNT(DISTINCT thread_id) AS n FROM invitations WHERE seat_id=? AND state='pending'",
                          (self.facts["agent_seat"],))
        listed = rows[0]["n"]
        if listed <= INBOX_PAGE_LIMIT:
            return FAIL, f"only {listed} inbox threads for the agent seat; a burst needs more than one {INBOX_PAGE_LIMIT}-thread page", None
        return PASS, f"{len(created)} extra threads invite the agent seat: {listed} inbox threads > one {INBOX_PAGE_LIMIT}-thread page", None

    def agent_pane_cli(self, argv, tag):
        env = dict(os.environ)
        env.update({"HERDR_ENV": "1", "HERDR_PANE_ID": self.facts["agent_pane"], "HERDR_SOCKET_PATH": self.endpoint,
                    "PATH": f"{self.bindir}{os.pathsep}{os.environ.get('PATH', '')}"})
        return self.run(argv, env=env, timeout=30, tag=tag, cwd=str(self.project))

    def s_burst_probe(self):
        """Dry-run only: the agent pane's own read of page 1 reports has_more with a continuation, and that
        continuation (run as rendered, through the shim) returns the rest. Reads only."""
        if not self.dry:
            raise Blocked("live run: only the model uses the agent pane's CLI")
        rc, out, err = self.agent_pane_cli([str(self.bindir / "herdr-threads"), "--json", "inbox"], "agent-read:burst-inbox-1")
        (self.ev / "burst-inbox-page1.json").write_text(out or err)
        try:
            page = json.loads(out)
        except ValueError:
            return FAIL, f"agent-pane inbox rc={rc}: {(err or out).strip()[:200]}", None
        has_more, next_argv = find_value(page, "has_more"), find_value(page, "next_argv")
        if has_more is not True or not isinstance(next_argv, list):
            return FAIL, f"page 1 has_more={has_more!r} next_argv={next_argv!r} with {len(self.facts.get('burst_threads', []))} burst threads", None
        # The continuation is run exactly as rendered (it already names --json/--state-dir/--host-endpoint; the shim
        # pins only what is missing), with --json added only when the rendered argv lacks it.
        rendered = [str(a) for a in next_argv[1:]]
        argv = [str(self.bindir / "herdr-threads")] + ([] if "--json" in rendered else ["--json"]) + rendered
        rc, out, err = self.agent_pane_cli(argv, "agent-read:burst-inbox-2")
        (self.ev / "burst-inbox-page2.json").write_text(out or err)
        if rc != 0:
            return FAIL, f"continuation {next_argv[:4]}... exits {rc}: {(err or out).strip()[:200]}", None
        return PASS, f"page 1 has_more=true with next_argv; continuation exits 0 (has_more={find_value(json.loads(out or '{}'), 'has_more')!r})", None

    def root_inbox_calls(self, transcript):
        results = self.transcript_results(transcript)
        calls = [c for c in self.transcript_calls(transcript) if not c[2] and re.search(r"\binbox\b", c[1])]
        return [(c[0], c[1], results.get(c[0])) for c in calls]

    def s_burst_has_more(self):
        calls = self.root_inbox_calls(self.phase_transcript("initial"))
        if not calls:
            return FAIL, "the model never ran `herdr-threads inbox`", None
        shown = [c for c in calls if c[2] and re.search(r'"has_more"\s*:\s*true|has_more[=:]\s*true|^next: herdr-threads\b|--cursor|more (threads|pages?)', c[2]["text"], re.I | re.M)]
        if shown:
            return PASS, f"root inbox result shows has_more / a continuation (call {shown[0][0]})", None
        if all(c[2] is None for c in calls):
            return UNVERIFIED, f"{len(calls)} root inbox call(s) but no tool result recorded", None
        return FAIL, f"{len(calls)} root inbox call(s); no result showed has_more or a continuation", None

    def s_burst_continuation(self):
        calls = self.root_inbox_calls(self.phase_transcript("initial"))
        continued = [c for c in calls if "--cursor" in c[1]]
        if continued:
            refused = [c for c in continued if c[2] and c[2]["error"]]
            if len(refused) == len(continued):
                return FAIL, f"every continuation call failed: {[(c[1][:160], (c[2] or {}).get('text', '')[:120]) for c in refused[:2]]}", None
            return PASS, f"model ran {len(continued)} continuation inbox call(s) with --cursor (first `{continued[0][1][:160]}`)", None
        return FAIL, f"model ran {len(calls)} inbox call(s) but never the continuation (--cursor)", None

    # ----- required (service membership, ht-4is.32.3) -----
    def daemon_descriptor(self):
        for descriptor in sorted((self.state / "instances").glob("*/endpoint.json")):
            try:
                data = json.loads(descriptor.read_text())
            except (OSError, ValueError):
                continue
            if data.get("endpoint") and data.get("instance_uuid"):
                return data
        return None

    def s_required_setup(self):
        descriptor = self.daemon_descriptor()
        if not descriptor:
            return FAIL, f"no daemon endpoint descriptor under {self.state / 'instances'}", None
        thread = f"svc-{self.run_id[-20:]}"
        client = ServiceClient(descriptor["endpoint"], descriptor["instance_uuid"])
        try:
            registration = client.register()
            client.operation("ensure_thread", {"thread": thread, "topic": f"required demo {self.run_id}",
                                               "goal": "Required service membership", "operation": f"{thread}-ensure"})
            invitation = client.operation("invite", {"thread": thread, "seat": self.facts["agent_seat"], "constraint": "required",
                                                     "deadline_millis": self.args.deadline * 1000, "operation": f"{thread}-invite"})
            client.operation("notify", {"thread": thread, "severity": "info", "operation": f"{thread}-notify",
                                        "event_json": {"kind": "ht_demo_required", "run": self.run_id}})
        finally:
            client.close()
            (self.ev / "service-client.json").write_text(json.dumps(client.transcript, indent=2))
        data = invitation.get("data") or {}
        requirement = data.get("requirement") or {}
        self.facts["required"] = {"thread": thread, "author": (registration.get("data") or {}).get("author"),
                                  "invitation": data.get("invitation"), "requirement": requirement.get("requirement"),
                                  "revision": requirement.get("revision")}
        if not requirement:
            return FAIL, f"service invite returned no requirement: {invitation}", None
        return PASS, (f"service author {self.facts['required']['author']} ensured managed thread {thread} and required-invited "
                      f"the agent seat (requirement {requirement.get('requirement')} rev {requirement.get('revision')}) over the "
                      f"D2 service wire; one info event"), None

    def s_required_probe(self):
        if not self.dry:
            raise Blocked("live run: only the real harness fires hooks")
        context = self.facts.get("probe_context", {}).get("SessionStart") or ""
        thread = self.facts["required"]["thread"]
        if thread in context or "accept-required" in context:
            return PASS, f"SessionStart context shows the required invitation ({'accept-required' if 'accept-required' in context else thread})", None
        return FAIL, f"SessionStart context does not show required thread {thread}: {context[:200]!r}", None

    def s_required_accept(self):
        required, seat = self.facts["required"], self.facts["agent_seat"]
        rows = self.query("SELECT id, state, revision, accepted_by_seat_id, accepted_observation FROM requirement_episodes "
                          "WHERE thread_id=? AND seat_id=?", (required["thread"], seat))
        calls = [c for c in self.transcript_calls(self.phase_transcript("initial"))
                 if not c[2] and "accept-required" in c[1] and required["thread"] in c[1]]
        (self.ev / "required-accept.json").write_text(json.dumps({"requirements": rows, "root_calls": calls}, indent=2))
        accepted = [r for r in rows if r["state"] == "accepted"]
        if not accepted:
            return FAIL, f"required invitation not accepted (states {[r['state'] for r in rows]})", None
        obs = json.loads(accepted[0]["accepted_observation"] or "{}")
        if accepted[0]["accepted_by_seat_id"] != seat or obs.get("provenance") not in MODEL_PROVENANCE:
            return FAIL, f"required acceptance actor/provenance {accepted[0]['accepted_by_seat_id']}/{obs.get('provenance')!r}", None
        if not calls:
            return FAIL, "requirement accepted in SQLite but the transcript has no root `accept-required` call", None
        return PASS, f"model accepted required {accepted[0]['id']} by root call {calls[0][0]}; stored actor {seat} provenance {obs.get('provenance')}", None

    def s_required_leave(self):
        required, seat = self.facts["required"], self.facts["agent_seat"]
        transcript = self.phase_transcript("initial")
        results = self.transcript_results(transcript)
        calls = [c for c in self.transcript_calls(transcript) if not c[2] and re.search(r"\bleave\b", c[1]) and required["thread"] in c[1]]
        membership = self.query("SELECT * FROM memberships WHERE thread_id=? AND seat_id=?", (required["thread"], seat))
        membership = membership[0] if membership else {}
        (self.ev / "required-leave.json").write_text(json.dumps({"calls": calls, "results": {c[0]: results.get(c[0]) for c in calls},
                                                                 "membership": membership}, indent=2))
        if not calls:
            return NOT_EXERCISED, "the model never ran `herdr-threads leave` on the required thread", None
        if membership.get("state") != "joined":
            return FAIL, f"membership after leave is {membership.get('state')!r}: the required membership was not held", None
        outcome = results.get(calls[-1][0])
        if outcome is None:
            return UNVERIFIED, "leave call recorded without a tool result; membership still joined", None
        if not outcome["error"] and not re.search(r"required|refus|cannot leave|not allowed", outcome["text"], re.I):
            return FAIL, f"leave returned success text while membership stayed joined: {outcome['text'][:200]!r}", None
        return PASS, f"leave refused ({outcome['text'][:160]!r}); membership still joined", None

    def s_required_no_receipts(self):
        thread = self.facts["required"]["thread"]
        events = self.query("SELECT id, kind FROM messages WHERE thread_id=? AND kind IN ('info','warn')", (thread,))
        ids = [e["id"] for e in events]
        rows = []
        for message in ids:
            rows += self.query("SELECT message_id, seat_id, state FROM receipt_state WHERE message_id=?", (message,))
            rows += self.query("SELECT message_id, seat_id, state FROM receipts WHERE message_id=?", (message,))
            rows += self.query("SELECT sm.message_id, pr.seat_id, 'prepared' AS state FROM send_manifests sm JOIN prepared_recipients pr "
                               "ON pr.preparation_id=sm.preparation_id WHERE sm.message_id=?", (message,))
        if not events:
            return FAIL, f"no service/system event in managed thread {thread}", None
        if rows:
            return FAIL, f"service/system events carry receipt rows: {rows[:3]}", None
        return PASS, f"{len(events)} service/system event(s) in {thread}, none with a receipt or ACK row", None

    # ----- managed launch -----
    def s_managed_available(self):
        if "cli_surface" not in self.facts:
            return FAIL, "CLI surface probe (S03) did not run; managed launch availability unknown", None
        surface = self.facts["cli_surface"].get("launch") or {}
        if surface.get("present") and "unrecognized subcommand" not in " ".join(surface.get("first_line") or []):
            return PASS, f"`herdr-threads launch` present: {surface.get('first_line')}", None
        self.unsupported_reasons["S17M"] = "managed_launch_unavailable"
        return UNSUPPORTED_STEP, (f"`herdr-threads launch` is absent in this build (rc={surface.get('rc')}); --launch managed "
                                  f"cannot run (lane/managed-launch not landed?)"), None

    def managed_step(self):
        # S03 is INFO by design (a surface report), so S17M depends on preflight only and reads S03's facts itself.
        self.step("S17M", "managed launch: `herdr-threads launch` present in this build", self.s_managed_available, needs=("S01",))

    def managed_launch_argv(self, phase):
        prefix = [self.bin, "--state-dir", str(self.state), "--host-endpoint", self.endpoint, "--json"]
        template = shlex.split(self.args.launch_argv.format(pane=self.facts.get("agent_pane", "PANE"), harness=self.harness))
        native = list(self.facts.get("native_argv", {}).get(phase, []))
        if self.harness == "codex":
            # D3 (Codex matrix 1): `launch` refuses caller `hooks.*` overrides (the owned hooks are on disk in the
            # scratch CODEX_HOME), so drop any such pair. The caller's leading `--no-daemon` stays: launch then adds
            # none after the prompt (product P4).
            kept, index = [], 0
            while index < len(native):
                if native[index] == "-c" and index + 1 < len(native) and native[index + 1].startswith("hooks."):
                    index += 2
                    continue
                kept.append(native[index])
                index += 1
            native = kept
        return prefix + template + native

    def managed_path_export(self, marker=None):
        """The owned pane shell's environment for `herdr-threads launch`, which starts the agent BY NAME in that shell.
        D4 (Codex matrix 1): drop any user shell function or alias named codex/claude (e.g. one adding unapproved
        approval-routing flags) so the name resolves to the binary; each unset is separate so a missing one is harmless.
        With `marker`, the shell then prints it on a line of its own (the typed command line never equals it), so the
        phase's pane capture can be cut at the launch (`pane_span`)."""
        line = f"export PATH={shlex.quote(str(self.bindir))}:\"$PATH\" HERDR_AGENT={self.harness}"
        if self.harness == "codex":
            self.facts["codex_home"] = str(self.codex_launch_home())
            line += f" CODEX_HOME={shlex.quote(str(self.facts['codex_home']))}"
        shadows = "; ".join(f"unset -f {name} 2>/dev/null; unalias {name} 2>/dev/null" for name in ("codex", "claude"))
        printed = f"; printf '%s\\n' {shlex.quote(marker)}" if marker else ""
        return line + f"; unset CLAUDECODE CLAUDE_CODE_ENTRYPOINT CODEX_HOME_OVERRIDE; {shadows}{printed}; true"

    def phase_marker(self, phase):
        """A fresh, unique pane marker for this managed phase, recorded in facts."""
        marker = f"{PHASE_MARKER} {self.run_id} {phase} {secrets.token_hex(4)}"
        self.facts.setdefault("pane_markers", {})[phase] = marker
        return marker

    def pane_span(self, phase):
        """(text, how): the part of the phase's pane capture that belongs to this launch. The agent pane is reused by
        every managed phase and a capture reads the last PANE_CAPTURE_LINES lines, so earlier phases' exec --json /
        stream-json output is usually still on screen; only lines after this phase's own marker count. When the marker
        is absent from a full-size capture it has scrolled out, so every captured line postdates it. A short capture
        without the marker cannot be attributed (except for the first phase, whose pane had no agent output before)."""
        try:
            text = (self.ev / f"pane-{phase}.txt").read_text(errors="replace")
        except OSError:
            return "", "no pane capture"
        marker = self.facts.get("pane_markers", {}).get(phase)
        lines = text.splitlines()
        if not marker:
            return text, "whole capture (no phase marker was printed)"
        index = next((i for i in range(len(lines) - 1, -1, -1) if lines[i].strip() == marker), None)
        if index is not None:
            return "\n".join(lines[index + 1:]) + "\n", "capture lines after this phase's marker"
        if len(lines) >= PANE_CAPTURE_LINES:
            return text, "whole capture (phase marker scrolled out, so every captured line postdates it)"
        earlier = [p for p in self.facts.get("pane_markers", {}) if p != phase]
        if not earlier:
            return text, "whole capture (phase marker not found; no earlier managed phase used this pane)"
        return "", "none (phase marker not found in a short capture; earlier phases' output cannot be excluded)"

    def managed_outcome(self, phase, transcript):
        """D6 (Codex) / D3 (Claude) matrix 1: a managed launch's native stream goes to the pane, so recover what the
        harness reported from THIS phase's span of the pane capture (Codex exec --json `thread.started` /
        `turn.completed`, Claude stream-json `result`), then fill any usage still missing from the session transcript
        or rollout (read-only), counted per phase (`phase_usage`)."""
        text, how = self.pane_span(phase)
        span = self.ev / f"pane-{phase}-span.txt"
        span.write_text(text)
        pane = self.transcript_outcome(span, lenient=True)
        pane["pane_span"] = how
        fallback, basis = self.phase_usage(transcript)
        if not pane["usage"] and fallback:
            pane["usage"] = fallback
            pane["usage_source"] = "session transcript" if self.harness == "claude" else "session rollout"
            pane["usage_basis"] = basis
        elif pane["usage"]:
            pane["usage_source"] = "pane capture"
        if self.harness == "codex" and not pane["thread_id"] and transcript:
            pane["thread_id"] = self.codex_session_meta(transcript).get("id")
        return pane

    def usage_parts(self, transcript):
        """(codex_total, claude_by_message): the last Codex rollout `token_count` total (cumulative for the session) and
        the Claude session transcript's root assistant usage keyed by message id."""
        total, by_message = {}, {}
        for event in self.transcript_events(transcript):
            payload = event.get("payload") if isinstance(event.get("payload"), dict) else {}
            if payload.get("type") == "token_count":
                found = find_value(payload, "total_token_usage")
                if isinstance(found, dict):
                    total = {k: v for k, v in found.items() if isinstance(v, (int, float))}
            message = event.get("message") if isinstance(event.get("message"), dict) else {}
            if event.get("type") == "assistant" and isinstance(message.get("usage"), dict) and not event.get("isSidechain"):
                by_message[message.get("id") or f"#{len(by_message)}"] = message["usage"]
        return total, by_message

    @staticmethod
    def sum_usage(by_message):
        usage = {}
        for counts in by_message.values():
            for key, value in counts.items():
                if isinstance(value, (int, float)):
                    usage[key] = usage.get(key, 0) + value
        return usage

    def phase_usage(self, transcript):
        """(usage, basis): this phase's share of a session record's usage. A Codex rollout `token_count` is cumulative
        per session, so a resumed session reports the delta from the total recorded after the previous phase on the
        same session; Claude assistant messages already counted by an earlier phase (a resumed transcript repeats or
        appends to them) are skipped by message id."""
        if not transcript:
            return {}, None
        total, by_message = self.usage_parts(transcript)
        if by_message:
            counted = self.facts.setdefault("usage_counted_messages", [])
            fresh = {k: v for k, v in by_message.items() if k.startswith("#") or k not in counted}
            counted.extend(k for k in fresh if not k.startswith("#"))
            skipped = len(by_message) - len(fresh)
            return self.sum_usage(fresh), (f"assistant messages new in this phase ({skipped} counted by an earlier phase skipped)"
                                           if skipped else "assistant messages of this session transcript")
        if not total:
            return {}, None
        session = self.codex_session_meta(transcript).get("id") or str(transcript)
        baselines = self.facts.setdefault("usage_baselines", {})
        previous = baselines.get(session)
        baselines[session] = dict(total)
        if not previous:
            return total, "session token_count total (first phase on this session)"
        if all(total.get(k, 0) >= v for k, v in previous.items()):
            return ({k: v - previous.get(k, 0) for k, v in total.items()},
                    "per-phase delta of the session's cumulative token_count")
        return total, "session token_count total (counter lower than after the previous phase; not a delta)"

    def launch_managed(self, phase):
        """`--launch managed`: prepare the owned empty shell (PATH shim only), then `herdr-threads launch` starts the
        native agent itself. Launch success is host-observed startup, never receipt; completion is judged from the
        ACK in SQLite and the harness transcript."""
        pane = self.facts["agent_pane"]
        rc, _, _, err = self.herdr("pane", "run", pane, self.managed_path_export(self.phase_marker(phase)),
                                   tag=f"managed:path:{phase}")
        if rc != 0:
            return FAIL, f"pane run (PATH shim) rc={rc}: {err.strip()[:200]}", None
        # launch inspects the owned installation in the scratch homes setup wrote (the agent shares them: Codex through
        # the pane's CODEX_HOME, Claude through `--settings` in its arguments).
        env = dict(os.environ)
        env.update(self.facts.get("setup_env") or self.setup_env())
        if self.harness == "codex":
            env["CODEX_HOME"] = self.facts.get("codex_home") or str(self.codex_launch_home())
        rc, out, err = self.run(self.managed_launch_argv(phase), env=env, tag=f"managed:launch:{phase}", timeout=60,
                                cwd=str(self.project))
        (self.ev / f"managed-launch-{phase}.out").write_text(out + err)
        if rc != 0:
            return FAIL, f"`herdr-threads launch` rc={rc}: {(err or out).strip()[:300]}", None
        message = self.facts["messages"].get(phase)
        deadline = time.monotonic() + self.args.timeout
        acked_at = None
        while time.monotonic() < deadline:
            transcript = self.live_transcript(phase)
            self.launch_tick(phase, transcript)
            receipt = {r["message_id"]: r for r in self.receipts_for(self.facts["agent_seat"])}.get(message) or {}
            if receipt.get("state") == "acked":
                acked_at = acked_at or time.monotonic()
                if time.monotonic() - acked_at > 15:  # let the model finish its reply
                    break
            time.sleep(2)
        self.capture_pane(phase)
        transcript = self.live_transcript(phase)
        outcome = self.managed_outcome(phase, transcript)
        return self.judge_launch(phase, PASS if transcript else FAIL,
                                 f"managed launch rc=0 ({out.strip()[:160]}); transcript {transcript or 'not found'}", transcript,
                                 outcome=outcome)

    # ----- Claude TUI trust (~/.claude.json record) -----
    def claude_json_snapshot(self, label):
        """Read-only record of ~/.claude.json: hash, size, per-top-level-key hashes and this project's entry. The file
        is never copied whole (it holds account data) and never written by the driver."""
        path = Path(self.args.claude_json).expanduser()
        record = {"label": label, "utc": utc(), "path": str(path), "sha256": sha256(path)}
        try:
            data = json.loads(path.read_text())
        except (OSError, ValueError) as error:
            record["error"] = type(error).__name__
            data = {}
        record["size"] = path.stat().st_size if path.exists() else None
        record["keys"] = {k: hashlib.sha256(json.dumps(v, sort_keys=True).encode()).hexdigest()[:16] for k, v in data.items()}
        projects = data.get("projects") if isinstance(data.get("projects"), dict) else {}
        # tui D2: project paths are the user's own directory names, so the evidence keys them by a path hash too.
        record["project_hashes"] = {self.path_key(k): hashlib.sha256(json.dumps(v, sort_keys=True).encode()).hexdigest()[:16]
                                    for k, v in projects.items()}
        record["project_count"] = len(projects)
        record["project_entry"] = {k: projects[k] for k in (str(self.project), str(self.project.resolve())) if k in projects}
        (self.ev / f"claude-json-{label}.json").write_text(json.dumps({k: v for k, v in record.items() if k != "keys"} |
                                                                      {"top_level_keys": sorted(record["keys"])}, indent=2))
        return record

    @staticmethod
    def path_key(path):
        """A stable, non-reversible evidence key for a ~/.claude.json project path."""
        return "path-sha256:" + hashlib.sha256(str(path).encode()).hexdigest()[:16]

    def s_tui_trust_before(self):
        project = str(self.project.resolve())
        if not any(project.startswith(root) for root in TUI_TRUST_ROOTS):
            return FAIL, f"scratch project {project} is outside {TUI_TRUST_ROOTS}: the trust-dialog approval covers only those", None
        self.facts["claude_json_before"] = self.claude_json_snapshot("before")
        before = self.facts["claude_json_before"]
        # PASS, not INFO: S17 needs this gate in TUI mode (scratch project inside the approved trust root, before-record taken)
        return PASS, f"recorded {before['path']} sha256 {before['sha256'][:16]} (project entry present: {bool(before['project_entry'])})", None

    def s_tui_trust_after(self):
        before = self.facts.get("claude_json_before")
        if not before:
            return INFO, "no before-record (TUI trust step did not run)", None
        after = self.claude_json_snapshot("after")
        changed_keys = sorted(k for k in set(before["keys"]) | set(after["keys"]) if before["keys"].get(k) != after["keys"].get(k))
        ours = {self.path_key(self.project), self.path_key(self.project.resolve())}
        other_projects = sorted(k for k in set(before["project_hashes"]) | set(after["project_hashes"])
                                if k not in ours and before["project_hashes"].get(k) != after["project_hashes"].get(k))
        diff = {"changed_top_level_keys": changed_keys, "our_project_entry_before": before["project_entry"],
                "our_project_entry_after": after["project_entry"], "other_project_entries_changed": other_projects}
        (self.ev / "claude-json-diff.json").write_text(json.dumps(diff, indent=2))
        self.facts["claude_json_diff"] = {k: v for k, v in diff.items() if not k.startswith("our_")}
        note = f"; {len(other_projects)} other project entr(ies) changed (concurrent Claude sessions?)" if other_projects else ""
        return INFO, (f"~/.claude.json sha256 {before['sha256'][:12]} -> {after['sha256'][:12]}; changed top-level keys "
                      f"{changed_keys}; scratch project entry {'added' if after['project_entry'] and not before['project_entry'] else 'unchanged or absent'}"
                      f"{note}; evidence claude-json-diff.json"), None

    # ----- dry-run probes for the scenarios -----
    def s_midturn_probe(self):
        """Dry-run only: after the SessionStart probe (the agent is registered), a new require-ACK message followed by
        a captured PreToolUse payload: the hook's additionalContext must carry it (the product half of midturn)."""
        if not self.dry:
            raise Blocked("live run: only the real harness fires hooks")
        status, detail, message = self.send("midturn")
        if status != PASS:
            return FAIL, detail, None
        self.facts["midturn"] = {"after_call": "dry-run-probe", "sent_utc": utc(), "send_status": status, "detail": detail, "message": message}
        status, detail, _ = self.s_hook_probe("PreToolUse")
        context = self.facts.get("probe_context", {}).get("PreToolUse") or ""
        if status != PASS:
            return status, detail, None
        if message in context:
            return PASS, f"PreToolUse additionalContext after the mid-turn send names {message}", None
        if context.strip():
            return PASS, f"PreToolUse additionalContext after the mid-turn send is non-empty ({len(context)} bytes; digest form, id not named)", None
        return FAIL, f"PreToolUse hook stayed quiet after a new require-ACK message {message}", None

    def s_warning_probe(self):
        """Dry-run only: after the SessionStart probe started the receipt timer, wait out the short deadline; the
        daemon must write exactly one durable warning with nobody ACKing."""
        if not self.dry:
            raise Blocked("live run: the model's late ACK decides this (SW1)")
        phase = "initial"
        if self.warning_after_idle():  # tui D1: the short deadline is on its own message, not the handoff
            status, detail, _ = self.send_warning_message()
            if status != PASS:
                return FAIL, detail, None
            phase = "warning"
        limit = time.monotonic() + self.args.warning_deadline + 30
        while time.monotonic() < limit:
            status, detail, _ = self.s_warning_event(phase)
            if status == PASS:
                return status, detail, None
            time.sleep(2)
        return self.s_warning_event(phase)

    # ----- cleanup / report -----
    def cleanup(self):
        if self.cleaned:
            return
        self.cleaned = True
        path = self.db_path()
        if path:
            try:
                source = sqlite3.connect(f"file:{path}?mode=ro", uri=True, timeout=5)
                target = sqlite3.connect(self.ev / "threads.snapshot.sqlite3")
                source.backup(target)
                source.close(), target.close()
            except sqlite3.Error as error:
                (self.ev / "sqlite-snapshot-error.txt").write_text(str(error))
        closes = {}
        if self.args.keep_panes:
            self.record("S99K", "cleanup", INFO, "--keep-panes: owned tab left open; daemon stopped only if owned")
        closing = [r for r in self.fixture.owned_resources() if not (self.args.keep_panes and r["kind"] == "pane")]
        for resource in reversed(closing):  # the ledger records the attempt; S99 below checks the outcome
            self.fixture._append({**resource, "event": "close_intent"})
            if resource["kind"] == "pane" and resource["id"].startswith("tab:"):
                closes[resource["id"]] = self.herdr("tab", "close", resource["id"][4:], tag="cleanup:tab-close")[0]
            elif resource["kind"] == "daemon":
                closes["daemon"] = self.ht("daemon", "stop", tag="cleanup:daemon-stop", timeout=30)[0]
            self.fixture._append({**resource, "event": "closed"})
        problems = []
        # S4: an owned daemon must be observably stopped; a failed `tab list` is not "no tab".
        hrc, _, _ = self.ht("daemon", "health", tag="cleanup:health-after", timeout=15)
        if self.daemon_owned and hrc == 0:
            problems.append(f"owned daemon still healthy after stop (stop rc={closes.get('daemon')}, health-after rc=0)")
        if self.facts.get("tab") and not self.args.keep_panes:
            trc, _, tout, terr = self.herdr("tab", "list", "--workspace", self.facts.get("workspace") or "", tag="cleanup:tab-list")
            if trc != 0:
                problems.append(f"`herdr tab list` rc={trc} ({terr.strip()[:120]}); owned tab {self.facts['tab']} closure UNVERIFIED")
            elif f'"{self.facts["tab"]}"' in tout:
                problems.append(f"owned tab {self.facts['tab']} still listed after close (close rc={closes.get('tab:' + self.facts['tab'])})")
        self.facts["cleanup"] = {"close_rc": closes, "daemon_health_after_rc": hrc, "daemon_owned": self.daemon_owned}
        self.record("S99", "cleanup", FAIL if problems else PASS,
                    "; ".join(problems) or f"closed {len(closing)} owned resources {closes}; daemon health after stop rc={hrc}"
                    + (" (owned daemon stopped)" if self.daemon_owned else " (daemon not owned; left running)"))
        if self.facts.get("claude_json_before"):
            self.step("S99T", "~/.claude.json after (read-only diff)", self.s_tui_trust_after)

    def manifest(self):
        seat = self.facts.get("agent_seat")
        messages = self.facts.get("messages", {})
        pairs = [{"message_id": m, "recipient_id": seat} for m in messages.values()] if seat else []
        calls, receipts, artifacts = [], [], []
        for result in self.phase_results:
            for call_id, _, _ in result["root_ack_calls"][:1]:
                artifact = f"evidence/transcript-calls-{result['phase']}.json"
                artifacts.append(artifact)
                calls.append({"call_id": call_id, "message_id": result["message"], "recipient_id": seat, "actor": "root", "artifact": artifact})
                if result["receipt"] and result["receipt"]["state"] == "acked":
                    sq = f"evidence/sqlite-{result['phase']}.json"
                    artifacts.append(sq)
                    receipts.append({"call_id": call_id, "message_id": result["message"], "recipient_id": seat, "actor": "root",
                                     "artifact": sq, "receipt_id": f"{result['message']}.{seat}"[:128]})
        failed = [s for s in self.steps if s["status"] in (FAIL, BLOCKED)]
        hard_failed = [s for s in self.steps if s["status"] == FAIL]
        environment = [s for s in self.steps if s["status"] == ENVIRONMENT]
        transport = [s for s in self.steps if s["status"] == TRANSPORT]
        unverified_steps = [s["step"] for s in self.steps if s["status"] == UNVERIFIED]
        unverified = unverified_steps + [r["phase"] for r in self.phase_results if str(r.get("child_check", "")).startswith("unverified")]
        self.facts["unverified"] = unverified
        not_exercised = [s["step"] for s in self.steps if s["status"] == NOT_EXERCISED]
        self.facts["not_exercised"] = not_exercised
        unsupported = [s["step"] for s in self.steps if s["status"] == UNSUPPORTED_STEP]
        if self.dry:
            status, reason = "UNSUPPORTED", "dry_run_no_model"
        elif self.args.cli_hint:  # a hinted prompt is a diagnostic, never acceptance evidence
            status, reason = "UNSUPPORTED", "diagnostic_cli_hint"
        elif environment and not hard_failed:
            # D5 (Codex demo 1): the harness failed for an account reason before reaching the model; the steps it
            # blocked say nothing about the product. Any genuine FAIL elsewhere still makes the run FAIL.
            kind = next((o.get("environment") for o in self.facts.get("launch_outcome", {}).values() if o.get("environment")), "failure")
            status, reason = "UNSUPPORTED", f"environment_{kind}"
        elif transport and not hard_failed:
            # D2 (Codex demo 2): the sandbox refused the daemon socket; the model never reached the product.
            status, reason = "UNSUPPORTED", "transport_denied"
        elif unsupported and not hard_failed:
            # A configuration this build does not offer (e.g. --launch managed without `launch`): nothing ran.
            status, reason = "UNSUPPORTED", self.unsupported_reasons.get(unsupported[0], "configuration_unsupported")
        elif not failed and not_exercised and len(receipts) < len(pairs):
            # lostprompt: the handoff ACK was not exercised (nothing could prompt the agent); never PASS, never FAIL.
            status, reason = "UNSUPPORTED", "safe_wake_unsupported" if self.facts.get("safe_wake_unsupported") else "scenario_not_exercised"
        elif not failed and receipts and len(receipts) == len(pairs):
            # B1: PASS only when no subagent activity occurred; otherwise everything else held but child ACK
            # absence is UNVERIFIED, which the schema (PASS carries no reason) can only express as UNSUPPORTED.
            # D4 (Codex demo 1): an unread hook-context delivery (S<n>H UNVERIFIED) is likewise never PASS.
            if not unverified and not not_exercised:
                status, reason = "PASS", ""
            elif not_exercised:  # an opt-in scenario's trigger never happened: its claim is untested, not wrong
                status, reason = "UNSUPPORTED", ("safe_wake_unsupported" if self.facts.get("safe_wake_unsupported")
                                                 else "scenario_not_exercised")
            else:  # codex D2: the reason names what is unverified, most serious first
                reasons = {u: self.unverified_reason(u) for u in unverified}
                self.facts["unverified_reasons"] = reasons
                status, reason = "UNSUPPORTED", min(reasons.values(), key=UNVERIFIED_REASONS.index)
        else:
            status, reason = "FAIL", "scenario_failed"
        # S5: `native` only when every observed accept/ACK carries native proof; cooperative_top_level is labelled cooperative.
        classes = [c for r in self.phase_results for k, c in (r.get("provenance") or {}).items() if k in ("ack", "accept") and c]
        source = "synthetic" if self.dry else self.provenance_label(classes)
        self.facts["provenance_classes"] = sorted(set(classes))
        self.facts["manifest_source"] = source
        record = {"schema_version": 1, "run_id": self.run_id, "status": status, "reason": reason,
                  "scenario": self.scenario_name(), "source": source,
                  "accepted_pairs": pairs, "model_calls": calls, "db_receipts": receipts, "transport_hints": [],
                  "retired_pairs": [], "artifacts": sorted(set(artifacts))}
        try:
            self.fixture.write_evidence("manifest.json", record)
        except ValueError as error:
            (self.ev / "manifest.invalid.json").write_text(json.dumps({"error": str(error), "record": record}, indent=2))
        return status

    @staticmethod
    def unverified_reason(item):
        """The manifest reason for one UNVERIFIED step (or a phase whose child check is unverified)."""
        item = str(item)
        if re.fullmatch(r"S\d+H", item):
            return "hook_context_unverified"
        if item == "SW2":
            return "warning_wake_unverified"
        if re.fullmatch(r"S\d+C|SC\d+|SK\d+", item) or not re.fullmatch(r"S[A-Z0-9]+", item):
            return "child_ack_unverified"
        return "evidence_unverified"

    def scenario_name(self):
        name = f"{self.harness}-{self.args.mode}-prelaunch-handoff"
        if self.launch_mode == "managed":
            name += ".managed"
        return ".".join([name, *self.scenarios])

    def main(self):
        """S3: every exit path (normal, driver error, ctrl+c, SIGTERM/SIGHUP) runs cleanup exactly once,
        so the owned tab (and the agent in it) and an owned daemon never outlive the driver."""
        def on_signal(signum, _frame):
            raise Interrupted(signal.Signals(signum).name)
        previous = {sig: signal.signal(sig, on_signal) for sig in (signal.SIGTERM, signal.SIGHUP)}
        try:
            self.run_steps()
        except (KeyboardInterrupt, Interrupted) as error:
            self.interrupted = str(error) or type(error).__name__
            self.record("S98", "run interrupted", FAIL, f"{self.interrupted}; remaining steps not run, cleanup follows")
        finally:
            for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):  # cleanup itself must not be interrupted
                signal.signal(sig, signal.SIG_IGN)
            try:
                self.cleanup()
            except Exception as error:  # noqa: BLE001 - recorded, never a pass
                self.record("S99", "cleanup", FAIL, f"cleanup error {type(error).__name__}: {error}")
            finally:
                signal.signal(signal.SIGINT, signal.default_int_handler)
                for sig, handler in previous.items():
                    signal.signal(sig, handler)
        return self.finish()

    def run_steps(self):
        a = self.args
        self.step("S01", "preflight environment and versions", self.s_preflight)
        self.step("S03", "CLI surface probe", self.s_cli_surface, needs=("S01",))
        self.step("S04", "hook entrypoint present", self.s_hook_entrypoint, needs=("S01",))
        self.step("S05", "scratch run/state/project dirs", self.s_scratch, needs=("S01",))
        claude_tui = a.mode == "tui" and self.harness == "claude"
        if claude_tui:  # USER-APPROVED trust accept for /private/tmp scratch projects only; ~/.claude.json recorded
            self.step("S01T", "~/.claude.json before (read-only) + scratch project under /private/tmp", self.s_tui_trust_before, needs=("S05",))
        self.step("S02", "installed harness version vs doctor recipe registry", self.s_version_pin, needs=("S01", "S05"))
        self.step("S02H", "hook version gate (empty-stdin probe, no service call)", self.s_hook_version_gate, needs=("S04",))
        self.step("S02Q", "user-level hook silent outside its Herdr instance", self.s_hook_quiet, needs=("S04", "S05"))
        self.step("S06", "isolated daemon ensure + health", self.s_daemon, needs=("S05",))
        self.step("S07", "owned scratch tab: agent + coordinator panes", self.s_panes, needs=("S05",))
        self.step("S08", "operator seat for agent pane (prelaunch)", lambda: self.resolve(self.facts["agent_pane"], "agent"), needs=("S06", "S07"))
        self.step("S09", "operator seat for coordinator pane", lambda: self.resolve(self.facts["coordinator_pane"], "coordinator"), needs=("S06", "S07"))
        self.step("S10", "coordinator lifecycle check-in", self.s_coord_checkin, needs=("S09",))
        self.step("S11", "thread create with topic", self.s_thread, needs=("S10",))
        self.step("S12", "INVITE agent seat before agent start", self.s_invite, needs=("S08", "S11"))
        setup_needs = ()
        if "burst" in self.scenarios:
            self.step("SB0", f"burst: {a.burst_threads} more invited threads (inbox > one page)", self.s_burst_setup, needs=("S12",))
            setup_needs += ("SB0",)
        if "required" in self.scenarios:
            self.step("SR0", "required: service registers, ensures a managed thread, required-invites the agent seat (D2 wire)",
                      self.s_required_setup, needs=("S06", "S08"))
            setup_needs += ("SR0",)
        self.step("S13", "send require-ACK handoff (initial)", lambda: self.send("initial"), needs=("S12",) + setup_needs)
        if a.codex_profile:  # D5: before S14, so `setup codex` runs under the launch profile's CODEX_HOME
            self.step("S16A", "aisw Codex profile (read-only; credentials for the scratch launch CODEX_HOME)", self.s_codex_profile, needs=("S01",))
        self.step("S14", "install owned user-level hooks into scratch config homes", self.s_hooks, needs=("S05",))
        if a.cli_hint:
            self.step("S14D", "DIAGNOSTIC --cli-hint project instruction (never acceptance)", self.s_cli_hint, needs=("S05",))
        self.step("S15", "SessionStart hook with captured payload (dry-run only)", lambda: self.s_hook_probe("SessionStart"), needs=("S14", "S07"), dry_only=True)
        self.step("S16", "PreToolUse(Bash) hook with captured payload (dry-run only)", lambda: self.s_hook_probe("PreToolUse"), needs=("S14", "S07"), dry_only=True)
        self.step("S16C", "agent-pane CLI read of pending receipts (dry-run only)", self.s_agent_read, needs=("S15", "S13"), dry_only=True)
        self.step("S16X", "one rendered ready command executes via the agent shim (dry-run only)", self.s_ready_executes, needs=("S15", "S13"), dry_only=True)
        if "midturn" in self.scenarios:
            self.step("SM0D", "midturn: PreToolUse context after a new require-ACK send (dry-run only)", self.s_midturn_probe, needs=("S16", "S13"), dry_only=True)
        if "warning" in self.scenarios:
            self.step("SW0D", f"warning: one durable warning after the {a.warning_deadline}s deadline (dry-run only)", self.s_warning_probe, needs=("S15", "S13"), dry_only=True)
        if "burst" in self.scenarios:
            self.step("SB0D", "burst: agent-pane inbox page 1 has_more + continuation (dry-run only)", self.s_burst_probe, needs=("S15", "SB0"), dry_only=True)
        if "required" in self.scenarios:
            self.step("SR0D", "required: SessionStart context shows the required invitation (dry-run only)", self.s_required_probe, needs=("S15", "SR0"), dry_only=True)
        self.step("S16P", "prelaunch SQLite state (read-only): pending invite + receipt, no ACK", self.s_prelaunch_state, needs=("S13",))
        self.step("S16R", "every ready command is permitted by the project allow rules", self.s_ready_permitted, needs=("S14", "S13"))
        if self.launch_mode == "managed":
            self.managed_step()
        live_needs = ("S02H", "S02Q", "S04", "S13", "S14", "S16P") + (("S16R",) if self.harness == "claude" else ()) + (
            ("S16A",) if a.codex_profile else ()) + (("S17M",) if self.launch_mode == "managed" else ()) + (
            ("S01T",) if claude_tui else ())
        transcript = self.step("S17", "launch native agent (initial)", lambda: self.s_launch("initial"), needs=() if self.dry else live_needs)
        self.facts.setdefault("phase_transcripts", {})["initial"] = str(transcript) if transcript else None
        self.step("S17H", f"hook additionalContext delivery ({self.hook_context_source()}, read-only) (initial)", lambda: self.s_hook_context("initial"), needs=("S17",))
        verify = (lambda: self.s_lostprompt_verify(transcript)) if "lostprompt" in self.scenarios else (
            lambda: self.s_wait_and_verify("initial", transcript))
        self.step("S18", "verify model accept + ACK in SQLite and transcript (initial)", verify, needs=("S17",))
        self.step("S18C", "child/subagent ACK absence (initial)", lambda: self.s_child_check("initial"), needs=("S18",))
        self.scenario_verdicts()
        phases = [p for p in a.phases.split(",") if p and p != "initial"]
        for index, phase in enumerate(phases):
            n = 19 + 3 * index
            if phase == "clear" and a.mode != "tui":
                self.record(f"S{n}", f"phase {phase}", SKIP, "clear needs an interactive TUI session (--mode tui: Claude /clear, Codex /new); not available for this launch mode")
                self.facts.setdefault("skipped_phases", []).append(phase)
                continue
            send_needs = ("S13",)
            if phase == "clear" and not self.dry:
                # Clear first, then send: with the production idle wake, a handoff sent to the idle pre-clear session is
                # woken and ACKed there before /clear or /new runs (native Codex w12), so it never tests the new session.
                self.step(f"S{n}X", "clear the TUI conversation before the handoff", self.s_tui_clear, needs=("S18",))
                send_needs = ("S13", f"S{n}X")
            self.step(f"S{n}", f"send require-ACK handoff ({phase})", lambda p=phase: self.send(p), needs=send_needs)
            t = self.step(f"S{n+1}", f"{phase} native agent", lambda p=phase: self.s_launch(p), needs=() if self.dry else (f"S{n}", "S18"))
            self.facts.setdefault("phase_transcripts", {})[phase] = str(t) if t else None
            self.step(f"S{n+1}H", f"hook additionalContext delivery ({phase})", lambda p=phase: self.s_hook_context(p), needs=(f"S{n+1}",))
            self.step(f"S{n+2}", f"verify model ACK ({phase}); earlier receipts unchanged", lambda p=phase, t=t: self.s_wait_and_verify(p, t), needs=(f"S{n+1}",))
            self.step(f"S{n+2}C", f"child/subagent ACK absence ({phase})", lambda p=phase: self.s_child_check(p), needs=(f"S{n+2}",))
        if "blockedui" in self.scenarios:  # last: the agent is left in the approval UI, never answered; cleanup closes the tab
            self.step("SU0", "blockedui: agent in a recognized approval UI (never answered)", self.s_blockedui_enter, needs=("S17",))
            self.step("SU1", "blockedui: short-deadline require-ACK queued while blocked", self.s_blockedui_queue, needs=("SU0",))
            self.step("SU2", "blockedui: no prompt injected while blocked; the wake stays pending", self.s_blockedui_no_injection,
                      needs=("SU1",))

    def scenario_verdicts(self):
        """Each opt-in scenario's own verdict items, judged on the initial launch (blockedui runs after every phase)."""
        if {"warning", "lostprompt", "blockedui"} & set(self.scenarios):
            self.step("SH0", "host safe_prompt capability (daemon health claim)", self.s_safe_prompt_health, needs=("S06",))
        if "lostprompt" in self.scenarios:
            self.step("SL1", "lostprompt: idle recovery wake reserved for the un-prompted agent", self.s_lostprompt_wake, needs=("S17",))
            self.step("SL2", "lostprompt: the wake reached the agent (marker on screen once)", self.s_lostprompt_delivery, needs=("SL1",))
        if "children" in self.scenarios:
            self.step("SK1", "children: two concurrent subagents read mail", lambda: self.s_children_concurrent("initial"), needs=("S17",))
            self.step("SK2", "children: no successful child write (complete sidechain)", lambda: self.s_child_no_ack("initial"), needs=("S17",))
            self.step("SK3", "children: top-level ACK present", lambda: self.s_top_level_ack("initial"), needs=("S17",))
        if "child" in self.scenarios:
            self.step("SC1", "child: subagent read allowed", lambda: self.s_child_read("initial"), needs=("S17",))
            self.step("SC2", "child: no successful child write (complete sidechain; DB execution is consistency only)", lambda: self.s_child_no_ack("initial"), needs=("S17",))
            self.step("SC3", "child: top-level ACK present", lambda: self.s_top_level_ack("initial"), needs=("S17",))
        if "midturn" in self.scenarios:
            self.step("SM1", "midturn: new require-ACK sent after the first tool call", self.s_midturn_sent, needs=("S17",))
            self.step("SM2", "midturn: PreToolUse additionalContext delivered it", self.s_midturn_delivery, needs=("SM1",))
            self.step("SM3", "midturn: model ACKed it (root call + SQLite)", self.s_midturn_ack, needs=("SM1",))
        if "warning" in self.scenarios:
            if self.warning_after_idle():
                self.step("SW0", "warning (TUI): short-deadline require-ACK sent once the agent is idle; wait for the wake",
                          self.s_warning_idle_send, needs=("S18",))
                self.step("SW1", "warning: exactly one durable missed-deadline warning", lambda: self.s_warning_event("warning"),
                          needs=("SW0",))
            else:
                self.step("SW1", "warning: exactly one durable missed-deadline warning", self.s_warning_event, needs=("S17",))
            self.step("SW2", "warning: one coalesced wake", self.s_warning_wake, needs=("SW1",))
        if "burst" in self.scenarios:
            self.step("SB1", "burst: model saw has_more on inbox page 1",
                      lambda: self.gate_on_instruction("burst", self.s_burst_has_more()), needs=("S17", "SB0"))
            self.step("SB2", "burst: model used the continuation (--cursor)",
                      lambda: self.gate_on_instruction("burst", self.s_burst_continuation()), needs=("S17", "SB0"))
        if "required" in self.scenarios:
            self.step("SR1", "required: model accepted the required invitation (root call + stored actor)",
                      lambda: self.gate_on_instruction("required", self.s_required_accept()), needs=("S17", "SR0"))
            self.step("SR2", "required: model's leave refused, membership held",
                      lambda: self.gate_on_instruction("required", self.s_required_leave()), needs=("S17", "SR0"))
            self.step("SR3", "required: service events carry no receipt/ACK rows", self.s_required_no_receipts, needs=("SR0",))

    def hook_context_source(self):
        """D4: where S<n>H reads delivery from, per harness."""
        return "Claude session transcript" if self.harness == "claude" else "Codex session rollout"

    def finish(self):
        status = self.manifest()
        self.facts["finished_utc"] = utc()
        provenance = [{"phase": r["phase"], "message": r["message"], **(r.get("provenance") or {}),
                       "label": r.get("provenance_label"), "child_check": r.get("child_check")} for r in self.phase_results]
        summary = {"facts": self.facts, "steps": self.steps, "phase_results": self.phase_results, "manifest_status": status,
                   "manifest_source": self.facts.get("manifest_source"), "provenance": provenance,
                   "interrupted": self.interrupted, "diagnostic_cli_hint": bool(self.args.cli_hint),
                   "skipped_phases": self.facts.get("skipped_phases", []), "unverified": self.facts.get("unverified", []),
                   "scenarios": list(self.scenarios), "launch": self.launch_mode, "not_exercised": self.facts.get("not_exercised", []),
                   "usage": self.facts.get("usage", {}), "codex_profile_attempts": self.facts.get("codex_profile_attempts", [])}
        (self.ev / "summary.json").write_text(json.dumps(summary, indent=2, sort_keys=True, default=str))
        first_fail = next((s for s in self.steps if s["status"] == FAIL), None)
        counts = {k: sum(1 for s in self.steps if s["status"] == k)
                  for k in (PASS, FAIL, BLOCKED, SKIP, INFO, UNVERIFIED, ENVIRONMENT, TRANSPORT, NOT_EXERCISED, UNSUPPORTED_STEP)}
        print(json.dumps({"run_id": self.run_id, "evidence": str(self.ev), "manifest_status": status, "usage": self.facts.get("usage", {}),
                          "manifest_source": self.facts.get("manifest_source"), "counts": counts,
                          "first_failure": first_fail}, indent=2))
        self.cmdlog.close()
        if self.interrupted:
            return 130
        if self.dry:
            return 0 if not first_fail else 1
        return 0 if status == "PASS" else 1


def parse(argv=None):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--harness", choices=["claude", "codex"], required=True)
    p.add_argument("--mode", choices=["print", "tui"], default="print", help="print (default: claude -p / codex exec) or tui (interactive Claude or Codex in the pane; enables the clear "
                        "phase: Claude /clear, Codex /new)")
    p.add_argument("--dry-run", action="store_true", help="everything except the model launch")
    p.add_argument("--bin", default=str(REPO / "bin" / "herdr-threads"), help="herdr-threads executable under test")
    p.add_argument("--state-dir", help="use this (e.g. installed plugin) state dir instead of an isolated one")
    p.add_argument("--host-endpoint", help="default $HERDR_SOCKET_PATH")
    p.add_argument("--run-root", help="parent of the private run directory (default $TMPDIR/ht-native-demo)")
    p.add_argument("--phases", default="initial,restart,resume", help="comma list of initial,restart,resume,clear")
    p.add_argument("--timeout", type=int, default=240, help="seconds per agent launch")
    p.add_argument("--verify-wait", type=int, default=30, help="seconds to poll SQLite for the ACK after the agent exits")
    p.add_argument("--deadline", type=int, default=900, help="invitation/receipt deadline seconds")
    p.add_argument("--setup", choices=["auto", "cli", "driver"], default="auto",
                   help="auto: public `setup` CLI when the binary has it (setup_mode=cli), else driver install (driver_fallback)")
    p.add_argument("--setup-argv", default=None,
                   help="override the setup CLI argv (default `setup claude` / `setup codex`; run with scratch HOME, "
                        "CLAUDE_CONFIG_DIR and CODEX_HOME)")
    p.add_argument("--claude-model", default="claude-haiku-4-5-20251001")
    p.add_argument("--claude-budget-usd", type=float, default=0.20, help="per launch; print mode only")
    p.add_argument("--claude-permission-mode", default="default")
    p.add_argument("--tui-accept-trust", action="store_true",
                   help="required for --mode tui: answer the Claude folder-trust dialog, only when it is on screen (writes ~/.claude.json)")
    p.add_argument("--allow-uncapped-spend", action="store_true",
                   help="required for live Codex and Claude --mode tui runs, which have no dollar cap (bounded only by --timeout x phases)")
    p.add_argument("--claude-projects-dir", default=str(Path.home() / ".claude" / "projects"),
                   help="where Claude writes session transcripts; read-only, for hook-context delivery evidence")
    p.add_argument("--cli-hint", action="store_true",
                   help="DIAGNOSTIC ONLY: write a one-line project instruction naming the CLI; the manifest is then UNSUPPORTED, never PASS")
    p.add_argument("--codex-model", default="gpt-6-luna")
    p.add_argument("--codex-effort", default="low")
    p.add_argument("--codex-sandbox", default="workspace-write")
    p.add_argument("--codex-config", action="append", default=[], help="extra codex -c key=value (e.g. scoped unix-socket proxy policy)")
    p.add_argument("--codex-transport", choices=["setup", "none"], default="setup",
                   help="setup (default): launch with the sandbox socket allowance `setup codex` writes into the scratch "
                        "config.toml for the driver's own daemon (its stable socket; must match the published endpoint); none: "
                        "proxy off for the launch, so the default sandbox refuses the socket and the run is UNSUPPORTED "
                        "transport_denied")
    p.add_argument("--codex-profile", default=None,
                   help="aisw Codex profile NAME: its auth.json (symlinked) for the scratch CODEX_HOME of the Codex launch (never written; "
                        "`aisw workspace check` still runs). `auto`: " + ", ".join(AISW_CODEX_PROFILES) + " in order, moving to "
                        "the next profile only when Codex reports a usage-limit failure (bounded, recorded)")
    p.add_argument("--aisw-codex-root", default=str(AISW_CODEX_ROOT), help=argparse.SUPPRESS)
    p.add_argument("--keep-panes", action="store_true")
    p.add_argument("--scenario", default="base",
                   help="comma list of opt-in scenarios, each with its own verdict steps: " + ", ".join(SCENARIOS) +
                        " (default base: the prelaunch handoff only). child: the prompt delegates reading to one subagent; "
                        "midturn: a new require-ACK message after the model's first tool call; warning: a short ACK deadline; "
                        "burst: more than one inbox page of threads; required: service required invitation + leave refusal; "
                        "lostprompt (tui): no launch prompt, the product's idle recovery wake must reach the agent; blockedui "
                        "(tui): agent left in an approval UI (never answered), a queued warning must inject nothing; children: "
                        "two concurrent reading subagents, no child write, root ACK")
    p.add_argument("--burst-threads", type=int, default=INBOX_PAGE_LIMIT + 2,
                   help=f"burst: extra invited threads (>{INBOX_PAGE_LIMIT} fills more than one default inbox page; max {MAX_BURST_THREADS})")
    p.add_argument("--warning-deadline", type=int, default=5, help="warning: seconds of ACK deadline on the initial handoff (1..120)")
    p.add_argument("--launch", choices=["manual", "managed"], default="manual",
                   help="manual (default): the driver starts the agent in the pane; managed: `herdr-threads launch` starts it "
                        "(detected; UNSUPPORTED managed_launch_unavailable when the build has no `launch` command)")
    p.add_argument("--launch-argv", default=MANAGED_LAUNCH_ARGV, help="managed launch argv template ({pane}, {harness}); native argv follows")
    p.add_argument("--claude-json", default=str(CLAUDE_JSON), help=argparse.SUPPRESS)
    args = p.parse_args(argv)
    validate_args(p, args)
    return args


def validate_args(p, args):
    scenarios_early = [n.strip() for n in str(getattr(args, "scenario", "") or "").split(",")]
    if args.harness == "codex" and args.mode == "tui" and args.run_root and not str(Path(args.run_root).resolve()).startswith(TUI_TRUST_ROOTS):
        p.error(f"codex --mode tui bypasses hook trust, approved for scratch projects under {TUI_TRUST_ROOTS} only")
    for name in ("lostprompt", "blockedui"):
        if name in scenarios_early and args.mode != "tui":
            p.error(f"--scenario {name} needs an interactive agent that stays alive in its pane: use --mode tui")
    if "lostprompt" in scenarios_early and ({"child", "children"} & set(scenarios_early)):
        p.error("--scenario lostprompt sends no launch prompt, so it cannot combine with child/children (prompt-driven)")
    if "child" in scenarios_early and "children" in scenarios_early:
        p.error("--scenario child and children ask for different delegation prompts; choose one")
    if args.harness == "claude" and args.mode == "tui" and not args.tui_accept_trust:  # S1
        p.error("--mode tui needs --tui-accept-trust: the new scratch project always shows the folder-trust dialog")
    launches = 1 + len([x for x in args.phases.split(",") if x and x != "initial"])  # initial always launches
    profile = getattr(args, "codex_profile", None)
    if profile is not None:
        if args.harness != "codex":
            p.error("--codex-profile applies to --harness codex only")
        if profile != "auto" and not PROFILE_NAME.match(profile):
            p.error("--codex-profile must be `auto` or a plain aisw profile name")
        if profile == "auto":  # each usage-limit retry is one more bounded launch
            launches += len(AISW_CODEX_PROFILES) - 1
    if args.timeout <= 0 or args.timeout > MAX_LAUNCH_TIMEOUT_S:  # S2
        p.error(f"--timeout must be 1..{MAX_LAUNCH_TIMEOUT_S} seconds per launch")
    if args.timeout * launches > MAX_TOTAL_LAUNCH_S:
        p.error(f"--timeout x phases = {args.timeout * launches}s exceeds {MAX_TOTAL_LAUNCH_S}s")
    try:
        scenarios = parse_scenarios(getattr(args, "scenario", ""))
    except ValueError as error:
        p.error(str(error))
    if not INBOX_PAGE_LIMIT < getattr(args, "burst_threads", INBOX_PAGE_LIMIT + 2) <= MAX_BURST_THREADS:
        p.error(f"--burst-threads must be {INBOX_PAGE_LIMIT + 1}..{MAX_BURST_THREADS} (more than one default inbox page, bounded)")
    if not 1 <= getattr(args, "warning_deadline", 5) <= 120:
        p.error("--warning-deadline must be 1..120 seconds")
    managed = getattr(args, "launch", "manual") == "managed"
    if "midturn" in scenarios and args.mode == "tui" and not managed:
        p.error("--scenario midturn watches the transcript while the launch runs: use --mode print (Claude), codex exec, or --launch managed")
    if managed and args.mode == "tui":
        p.error("--launch managed composes the print/exec native argv; use --mode print")
    if args.tui_accept_trust and args.run_root and not any(
            str(Path(args.run_root).resolve()).rstrip("/") + "/" == root or str(Path(args.run_root).resolve()).startswith(root)
            for root in TUI_TRUST_ROOTS):
        p.error(f"--tui-accept-trust is approved only for scratch projects under {TUI_TRUST_ROOTS}; --run-root {args.run_root} is outside")
    uncapped = args.harness == "codex" or args.mode == "tui"
    if uncapped and not args.dry_run and not args.allow_uncapped_spend:
        p.error("live Codex / Claude TUI runs have no dollar cap; pass --allow-uncapped-spend to accept spend bounded "
                "only by --timeout x phases")


if __name__ == "__main__":
    sys.exit(Driver(parse()).main())
