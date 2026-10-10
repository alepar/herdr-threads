#!/usr/bin/env python3
"""agy live smoke runner (test-infra spec D11).

  scripts/agy-live-smoke (--live | --dry-run) [--evidence-dir DIR] [--allow-dirty] [--herdr-threads-bin PATH]

``--dry-run`` runs the identical flow against a scripted fake upstream (genuine agy, zero spend).
``--live`` spends real money through the OpenRouter relay and is executed only by the orchestrator,
once per final SHA. Exit codes: 0 pass; 1 the smoke failed or aborted; 2 environment refusal;
3 the leak check failed. Never runs in CI. Python 3.9 compatible, standard library only.
"""
from __future__ import annotations

import argparse
import datetime
import json
import os
import re
import signal
import subprocess
import sys
import tempfile
import time
import traceback
import uuid
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
SANDBOX_DIR = REPO / "scripts" / "agy-sandbox"
for _dir in (HERE, HERE / "flows", SANDBOX_DIR):
    if str(_dir) not in sys.path:
        sys.path.insert(0, str(_dir))

import agyenv  # noqa: E402
import gate  # noqa: E402
import observe  # noqa: E402
import relay_backend  # noqa: E402

EXIT_OK, EXIT_FAIL, EXIT_REFUSED, EXIT_LEAK = 0, 1, 2, 3
CAP_SECONDS = 600
READY_PROMPT = "This is an automated herdr-threads smoke test. Reply with the single word READY."
ACK_BODY = ("Smoke check: please acknowledge this message, then reply in this thread with the text "
            "SMOKE-OK.")
FALLBACK_PROMPT = ("Check your herdr-threads messages, acknowledge any that require it, and reply "
                   "SMOKE-OK in that thread.")
REPLY_TEXT = "SMOKE-OK"
UPSTREAM_MODEL = "google/gemini-2.5-flash"
LIVE_MAX_REQUESTS, LIVE_MAX_COST_USD = 24, 0.25  # below the relay's 40 / $0.50 ceilings; expected 6-12 / ~$0.04
WAKE_TIMEOUT, ACT_TIMEOUT, IDLE_TIMEOUT = 90, 180, 120
SUMMARY_KEYS = ("sha", "dirty", "agy_version", "agy_sha512", "herdr_version", "model", "requests",
                "cost_usd", "phases", "ack_phase", "checks", "pass")
EVIDENCE_FILES = ("summary.json", "report.md", "relay.jsonl", "daemon-excerpt.json", "pane-final.txt",
                  "requests-excerpt.json")

_KEY_RE = re.compile(r"sk-or-[A-Za-z0-9_-]+")
_KEY_LEFT_RE = re.compile(r"sk-or-(?!REDACTED(?![A-Za-z0-9_-]))")
TEXT_LIMIT = 2048


class SmokeAbort(Exception):
    """The run stopped before a verdict: ``reason`` is ``budget``, ``cap`` or ``error``."""

    def __init__(self, reason, detail=""):
        Exception.__init__(self, "%s: %s" % (reason, detail) if detail else reason)
        self.reason = reason


class EvidenceError(Exception):
    pass


# ---------------------------------------------------------------- pure helpers

def build_parser():
    ap = argparse.ArgumentParser(prog="agy-live-smoke", description=__doc__.splitlines()[0])
    mode = ap.add_mutually_exclusive_group(required=True)
    mode.add_argument("--live", action="store_true", help="spend real money through the OpenRouter relay")
    mode.add_argument("--dry-run", action="store_true", help="scripted fake upstream; zero spend")
    ap.add_argument("--evidence-dir")
    ap.add_argument("--allow-dirty", action="store_true")
    ap.add_argument("--herdr-threads-bin")
    return ap


def refusal(args, env, platform_name, dirty, herdr_version_text):
    """The first environment refusal (a message), or None. Pure: every input is a parameter."""
    if env.get("CI") or env.get("GITHUB_ACTIONS"):
        return "refusing to run under CI/GITHUB_ACTIONS: the live smoke never runs in CI"
    if platform_name != "darwin":
        return "the live smoke needs macOS (sandbox-exec and Herdr agy detection); found %s" % platform_name
    if args.live and dirty and not args.allow_dirty:
        return "--live needs a clean worktree (the record names the SHA); commit, or pass --allow-dirty"
    return gate.refusals(dict(env), platform_name, herdr_version_text)


def utc_date(now=None):
    return (now or datetime.datetime.utcnow()).strftime("%Y-%m-%d")


def default_evidence_dir(live, sha, date, scratch_root=None, repo=REPO):
    """``docs/evidence/agy-live-smoke/<UTC date>-<sha12>/`` for live, the scratch root for dry-run."""
    tag = "%s-%s" % (date, (sha or "unknown")[:12])
    if live:
        return Path(repo) / "docs" / "evidence" / "agy-live-smoke" / tag
    return Path(scratch_root or tempfile.gettempdir()) / ("agy-live-smoke-dry-run-" + tag)


def dry_run_script(message_id=None, thread_id=None, cwd="/tmp"):
    """The fake-upstream script: READY, ACK, reply (the ``send`` argv per ``src/cli/commands.rs``), done.

    Without ids the commands use the ``{frame.id}`` / ``{frame.thread}`` templates. Every reply costs
    ``0.0`` so the relay records zero spend instead of its worst-case estimate."""
    def call(command):
        # agy's run_command schema requires all five properties (tool validation rejects fewer).
        args = {"CommandLine": command, "Cwd": str(cwd), "WaitMsBeforeAsync": 2000,
                "toolSummary": "Run " + command, "toolAction": "Running"}
        return {"cost": 0.0, "tool_calls": [{"name": "run_command", "arguments": args}]}
    return {
        "format": relay_backend.UPSTREAM_FORMAT,
        "replies": [
            {"text": "READY", "cost": 0.0},
            call("herdr-threads ack %s" % (message_id or "{frame.id}")),
            call("herdr-threads send %s --body %s" % (thread_id or "{frame.thread}", REPLY_TEXT)),
            {"text": "done", "cost": 0.0},
        ],
        "default": {"text": "done", "cost": 0.0},
    }


def redact_text(text, root=None, home=None):
    """Replace ``sk-or-…`` keys, the world root (``<root>``) and the real home (``<home>``)."""
    text = _KEY_RE.sub("sk-or-REDACTED", text)
    for path, label in ((root, "<root>"), (home, "<home>")):
        if path:
            for variant in sorted({str(path), os.path.realpath(str(path))}, key=len, reverse=True):
                if variant and variant != "/":
                    text = text.replace(variant, label)
    return text


def key_leaks(text):
    """True when ``text`` holds a key-shaped ``sk-or-`` not already replaced by the redaction marker."""
    return bool(_KEY_LEFT_RE.search(text))


def trim(text, limit=TEXT_LIMIT):
    text = str(text)
    return text if len(text) <= limit else text[:limit] + "...[trimmed]"


class EvidenceWriter(object):
    """Writes redacted evidence files and refuses to finish if ``sk-or-`` appears in any of them."""

    def __init__(self, directory, root=None, home=None):
        self.dir = Path(directory)
        self.root = root
        self.home = home
        self.written = []
        self.dir.mkdir(parents=True, exist_ok=True)

    def redact(self, text):
        return redact_text(text, self.root, self.home)

    def write(self, name, data):
        if isinstance(data, (dict, list)):
            data = json.dumps(data, indent=2, sort_keys=True, default=str) + "\n"
        path = self.dir / name
        path.write_text(self.redact(str(data)))
        if name not in self.written:
            self.written.append(name)
        return path

    def finish(self):
        """Scan every file written; raise ``EvidenceError`` naming each that holds ``sk-or-``."""
        bad = [name for name in self.written if key_leaks((self.dir / name).read_text())]
        if bad:
            raise EvidenceError("refusing to finish: sk-or- found in %s" % ", ".join(bad))
        return list(self.written)


def build_summary(info, checks, phases, relay, ack_phase, aborted=None, mode="dry-run"):
    """The ``summary.json`` object (every ``SUMMARY_KEYS`` key is always present)."""
    relay = relay or {}
    ok = bool(checks) and all(c["ok"] for c in checks) and not aborted
    return {
        "sha": info.get("sha"), "dirty": bool(info.get("dirty")),
        "agy_version": info.get("agy_version"), "agy_sha512": info.get("agy_sha512"),
        "herdr_version": info.get("herdr_version"),
        "model": UPSTREAM_MODEL if mode == "live" else "fake-upstream (scripted, no model)",
        "requests": relay.get("requests", 0), "cost_usd": relay.get("cost_usd", 0.0),
        "cost_estimated": bool(relay.get("cost_estimated")), "refusals": relay.get("refusals", 0),
        "phases": list(phases), "ack_phase": ack_phase, "checks": list(checks),
        "aborted": aborted, "mode": mode, "pass": ok,
    }


def report_md(summary):
    lines = ["# agy live smoke (%s)" % summary["mode"], "",
             "- SHA: `%s`%s" % (summary["sha"], " (dirty)" if summary["dirty"] else ""),
             "- agy: %s, sha512 `%s`" % (summary["agy_version"], (summary["agy_sha512"] or "")[:16]),
             "- herdr: %s" % summary["herdr_version"],
             "- model: %s; main requests: %s; cost: $%s%s" % (
                 summary["model"], summary["requests"], summary["cost_usd"],
                 " (estimated)" if summary.get("cost_estimated") else ""),
             "- ACK phase: %s%s" % (summary["ack_phase"],
                                    " (model behaviour: the wake did not lead to an ACK)"
                                    if summary["ack_phase"] == "prompt" else ""),
             "- result: **%s**%s" % ("PASS" if summary["pass"] else "FAIL",
                                     " (aborted: %s)" % summary["aborted"] if summary["aborted"] else ""),
             "", "## Phases", ""]
    for phase in summary["phases"]:
        lines.append("- %s: %s in %ss" % (phase["name"], "ok" if phase["ok"] else "failed", phase["seconds"]))
    lines += ["", "## Checks", ""]
    for check in summary["checks"]:
        lines.append("- [%s] %s%s" % ("x" if check["ok"] else " ", check["name"],
                                      " (%s)" % check["detail"] if check["detail"] and not check["ok"] else ""))
    return "\n".join(lines) + "\n"


# ---------------------------------------------------------------- flow

class Clock(object):
    """The wall-clock cap and the budget abort, polled by every wait."""

    def __init__(self, backend, cap=CAP_SECONDS, now=time.monotonic):
        self.backend, self.now = backend, now
        self.start = now()
        self.cap = cap

    def elapsed(self):
        return self.now() - self.start

    def poll(self):
        if self.elapsed() > self.cap:
            raise SmokeAbort("cap", "%ds wall-clock cap reached" % self.cap)
        if self.backend.budget().get("exhausted"):
            raise SmokeAbort("budget", "the relay reported its budget exhausted")

    def wait(self, pred, timeout, step=0.5):
        """Poll ``pred`` for up to ``timeout`` seconds (bounded by the cap); the last value."""
        deadline = self.now() + timeout
        while True:
            self.poll()
            value = pred()
            if value or self.now() >= deadline:
                return value
            time.sleep(step)


def walk_onboarding(world, pane, clock, timeout=90):
    """Walk genuine agy's first-run screens (colour scheme, Terms of Service) with keys until the composer.

    The isolated HOME is fresh every run, so every run meets them (recipe: tests/fixtures/agy/composer)."""
    send = lambda *keys: world.herdr("pane", "send-keys", pane.pane_id, *keys, check=False)

    def step():
        screen = pane.read_detection()
        if "for shortcuts" in screen:
            return True
        if "Choose your color scheme" in screen:
            send("enter")
        elif "Terms of Service" in screen:
            send("down", "right")
            time.sleep(0.5)
            send("enter")
        return False
    return clock.wait(step, timeout, step=0.4)


def db_rows(world, sql, params=()):
    import sqlite3
    try:
        conn = observe.db(world)
        if conn is None:
            return []
        try:
            return [dict(row) for row in conn.execute(sql, params).fetchall()]
        finally:
            conn.close()
    except sqlite3.Error:
        return []


def human_json(world, argv, pane):
    """``herdr-threads human --json <argv>`` from a person's pane: the decoded ``result.data`` or a ``_rc`` stub."""
    result = world.ht(["--json"] + list(argv), pane=pane, check=False, human=True)
    try:
        doc = json.loads(result.stdout)
    except ValueError:
        return {"_rc": result.returncode, "_stdout": result.stdout, "_stderr": result.stderr}
    if isinstance(doc, dict) and isinstance(doc.get("result"), dict) and "data" in doc["result"]:
        return doc["result"]["data"]
    return doc


def agy_seat(world):
    import _l5
    binding = _l5.live_binding(world)
    return binding["seat_id"] if binding else None


def data_id(doc, *names):
    if isinstance(doc, str):  # `thread create --json` returns the bare ID
        return doc.strip() or None
    return agyenv.find_key(doc, names)


def injected_messages(world, backend, message_id):
    """SYSTEM_MESSAGE texts naming ``message_id``: agy's transcripts, and the fake upstream's bodies."""
    found = []
    for path in world.agy_data.rglob("*.jsonl") if world.agy_data.is_dir() else []:
        try:
            lines = path.read_text(errors="replace").splitlines()
        except OSError:
            continue
        for line in lines:
            if message_id not in line or "SYSTEM_MESSAGE" not in line:
                continue
            try:
                entry = json.loads(line)
                text = entry.get("content") if isinstance(entry, dict) else None
            except ValueError:
                text = None
            found.append(text if isinstance(text, str) else line)
    for body in backend.upstream_requests():
        for msg in body.get("messages") or []:
            content = msg.get("content") if isinstance(msg, dict) else None
            text = content if isinstance(content, str) else json.dumps(content)
            if message_id in text and "<SYSTEM_MESSAGE>" in text:
                found.append(text)
    return list(dict.fromkeys(found))


def receipt_rows(world, message_id):
    """Receipt rows of a message: the receipts table, else its ``receipt_state`` projection."""
    rows = db_rows(world, "SELECT * FROM receipts WHERE message_id=?", (message_id,))
    return rows or db_rows(world, "SELECT * FROM receipt_state WHERE message_id=?", (message_id,))


def thread_messages(world, thread_id):
    return db_rows(world, "SELECT id, sequence, kind, actor_seat_id, body, event_json FROM messages "
                          "WHERE thread_id=? ORDER BY sequence", (thread_id,))


def acked(world, message_id, seat):
    return any(r["seat_id"] == seat and r["state"] == "acked" for r in receipt_rows(world, message_id))


def replied(world, thread_id, seat):
    return any(m["kind"] == "ordinary" and m["actor_seat_id"] == seat and m["body"]
               and REPLY_TEXT.lower() in m["body"].lower() for m in thread_messages(world, thread_id))


def ack_calls(backend, message_id):
    """The model's ``run_command`` calls that ACK ``message_id`` (``herdr-threads ack <id>``)."""
    pattern = re.compile(r"herdr-threads\b.*\back\b.*%s" % re.escape(message_id))
    return [c for c in run_tool_calls(backend) if c.get("name") == "run_command"
            and pattern.search(c.get("args_trimmed") or "")]


def tool_results(backend):
    """What the model's tool calls returned (dry-run only: read from the fake upstream's bodies)."""
    out = []
    for body in backend.upstream_requests():
        for msg in body.get("messages") or []:
            if isinstance(msg, dict) and msg.get("role") == "tool" and isinstance(msg.get("content"), str):
                if msg["content"] not in out:
                    out.append(msg["content"])
    return out


def run_tool_calls(backend):
    out = []
    for rec in backend.requests():
        for call in rec.get("tool_calls") or []:
            out.append(dict(call, seq=rec.get("seq")))
    return out


def smoke(world, backend, clock, state):
    """Flow steps 1-7 of spec D11. Records into ``state`` (phases, checks via ``world.check``)."""
    phases = state["phases"]

    def phase(name, started, ok):
        phases.append({"name": name, "ok": bool(ok), "seconds": round(time.monotonic() - started, 1)})

    # 1. enroll
    t0 = time.monotonic()
    # World.setup_agy() prefixes --state-dir, which the agent-actor escalating-word rule refuses
    # (`setup` must be the first word); the human namespace is the person/operator route.
    setup = world.ht(["setup", "agy", "--with-permissions"], check=False, human=True)
    world.check("setup agy succeeds", setup.returncode == 0, (setup.stderr or setup.stdout).strip()[:200])
    pane = world.start_agy()
    state["pane"] = pane
    world.check("agy onboarding walked to the composer", walk_onboarding(world, pane, clock))
    pane.prompt(READY_PROMPT)
    import _l5
    binding = clock.wait(lambda: _l5.live_binding(world), 60)
    seat = binding["seat_id"] if binding else None
    ready = world.check("the agy seat is enrolled", binding is not None and binding["harness"] == "agy",
                        "binding=%s" % binding)
    phase("enroll", t0, ready)
    if not ready:
        raise SmokeAbort("error", "no enrolled agy seat")

    # 2. idle, then the ACK-required message from a sender in a new thread
    t0 = time.monotonic()
    idle = clock.wait(lambda: pane.wait("idle", 5), IDLE_TIMEOUT, step=0)
    world.check("agy reached idle after the READY prompt", idle)
    sender = world.sender()
    # The topic has no spaces on purpose; the thread is joined by the agy seat below so the message
    # is delivered to a member (an explicit ACK recipient must already be invited or joined).
    thread_doc = human_json(world, ["thread", "create", "--topic", "agy-live-smoke"], sender.pane_id)
    thread_id = data_id(thread_doc, "thread_id", "id")
    world.check("the sender created a thread", bool(thread_id), "result=%s" % (thread_doc,))
    invite = world.ht(["invite", str(thread_id), "--seat", str(seat)], pane=sender.pane_id, check=False,
                      human=True)
    world.check("the sender invited the agy seat", invite.returncode == 0,
                (invite.stderr or invite.stdout).strip()[:300])
    # Scaffolding, not the model: the runner accepts in the agy pane's own identity (the model is
    # asked only to ACK and reply).
    accept = world.ht(["accept", str(thread_id)], pane=pane.pane_id, check=False)
    world.check("the agy seat joined the thread (runner scaffolding)", accept.returncode == 0,
                (accept.stderr or accept.stdout).strip()[:300])
    before = len([r for r in backend.requests() if r.get("kind") == "main"])
    sent = world.ht(["--json", "send", str(thread_id), "--body", ACK_BODY, "--require-ack", str(seat)],
                    pane=sender.pane_id, check=False, human=True)
    world.check("the ACK-required message was sent", sent.returncode == 0,
                (sent.stderr or sent.stdout).strip()[:300])
    try:
        sent_doc = json.loads(sent.stdout)
        sent_data = sent_doc.get("result", {}).get("data", sent_doc) if isinstance(sent_doc, dict) else sent_doc
    except ValueError:
        sent_data = None
    message_id = data_id(sent_data, "message_id", "id")
    if not message_id:
        rows = db_rows(world, "SELECT id FROM messages WHERE thread_id=? AND kind='ordinary' ORDER BY ordinal "
                              "LIMIT 1", (thread_id,))
        message_id = rows[0]["id"] if rows else None
    world.check("the message is stored", bool(message_id))
    state.update(thread_id=thread_id, message_id=message_id, seat=seat, sender=sender)
    phase("send", t0, idle and sent.returncode == 0 and bool(message_id))
    if not message_id:
        raise SmokeAbort("error", "message was not stored")
    if backend.mode == relay_backend.FAKE:
        # fake_upstream's frame-header regex predates the quoted thread name in real headers
        # (`in ID "NAME" from`), so {frame.*} cannot match; the ids are known now, before the wake.
        backend.load(dry_run_script(message_id, thread_id, world.ws))

    # 3. wake: a model request nobody typed, whose SYSTEM_MESSAGE holds the frame header and the id
    t0 = time.monotonic()
    blocked = []

    def watch_blocked():
        if pane.agent_status() == "blocked":
            blocked.append(time.monotonic())

    def woken():
        watch_blocked()
        new = len([r for r in backend.requests() if r.get("kind") == "main"]) > before
        return new and injected_messages(world, backend, message_id)
    wake = clock.wait(woken, WAKE_TIMEOUT)
    injected = injected_messages(world, backend, message_id)
    header = [t for t in injected if re.search(r"^\[herdr-threads\] \S+ %s in " % re.escape(message_id), t, re.M)]
    world.check("wake: a new model request carried the injected SYSTEM_MESSAGE with the message id "
                "(the runner typed no prompt)", bool(wake), "main requests before=%d" % before)
    world.check("the injected SYSTEM_MESSAGE holds the frame header with the message id", bool(header),
                "injected=%d" % len(injected))
    typed = [t for t in injected if READY_PROMPT in t or FALLBACK_PROMPT in t]
    world.check("no USER_REQUEST of the wake turn is a runner prompt", not typed, "matches=%d" % len(typed))
    state["injected"] = injected
    phase("wake", t0, wake)

    # 4. act
    t0 = time.monotonic()

    def settled():
        watch_blocked()
        return ack_calls(backend, message_id) and replied(world, thread_id, seat)
    done = clock.wait(settled, ACT_TIMEOUT)
    if ack_calls(backend, message_id):
        state["ack_phase"] = "wake"
    phase("act", t0, done)

    # 5. fallback: one explicit prompt when the woken turn produced no ACK
    if not ack_calls(backend, message_id):
        t0 = time.monotonic()
        clock.wait(lambda: pane.wait("idle", 5), 60, step=0)
        pane.prompt(FALLBACK_PROMPT)
        done = clock.wait(settled, ACT_TIMEOUT)
        if ack_calls(backend, message_id):
            state["ack_phase"] = "prompt"
        phase("fallback", t0, done)
    clock.wait(lambda: pane.wait("idle", 5), 60, step=0)

    # 6. verdict checks
    receipts = [r for r in receipt_rows(world, message_id) if r["seat_id"] == seat]
    tools = run_tool_calls(backend)
    world.check("the agy seat ACKed through run_command", bool(ack_calls(backend, message_id)),
                "tool calls=%s" % tools)
    world.check("the receipt is acked", acked(world, message_id, seat),
                "receipts=%s" % [(r["state"], r.get("ack_observation")) for r in receipts])
    world.check("no permission prompt: the agent was never blocked", not blocked,
                "blocked samples=%d" % len(blocked))
    world.check("the agy seat replied SMOKE-OK in the thread", replied(world, thread_id, seat),
                "messages=%s" % [(m["actor_seat_id"], (m["body"] or "")[:40])
                                 for m in thread_messages(world, thread_id)])
    by_hook = any("cooperative_hook_delivery" in (r.get("ack_observation") or "") for r in receipts)
    world.check("the delivery receipt is settled (cooperative_hook_delivery, or superseded by the ACK)",
                by_hook or acked(world, message_id, seat),
                "ack_observation=%s" % [r.get("ack_observation") for r in receipts])
    state["settled_by"] = "cooperative_hook_delivery" if by_hook else "ack"
    budget = backend.budget()
    world.check("no relay refusal", not budget.get("refusals"), "refusals=%s" % budget.get("refusals"))
    state["blocked"] = len(blocked)


# ---------------------------------------------------------------- evidence

def collect_evidence(world, backend, state, summary, writer):
    seat, thread_id, message_id = state.get("seat"), state.get("thread_id"), state.get("message_id")
    pane = state.get("pane")
    daemon = {
        "seat": db_rows(world, "SELECT * FROM seats WHERE id=?", (seat,)),
        "bindings": db_rows(world, "SELECT * FROM occupant_bindings WHERE seat_id=?", (seat,)),
        "receipts": receipt_rows(world, message_id) if message_id else [],
        "invitations": db_rows(world, "SELECT * FROM invitations WHERE thread_id=?", (thread_id,)),
        "memberships": db_rows(world, "SELECT * FROM memberships WHERE thread_id=?", (thread_id,)),
        "messages": [dict(m, body=trim(m["body"] or "")) for m in thread_messages(world, thread_id)]
        if thread_id else [],
    }
    excerpt = {"injected_system_messages": [trim(t) for t in state.get("injected", [])],
               "tool_calls": run_tool_calls(backend),
               "tool_results": [trim(t, 400) for t in tool_results(backend)]}
    relay_log = Path(backend.run_dir) / "relay.jsonl" if backend.run_dir else None
    writer.write("summary.json", summary)
    writer.write("report.md", report_md(summary))
    writer.write("relay.jsonl", relay_log.read_text() if relay_log and relay_log.exists() else "")
    writer.write("daemon-excerpt.json", daemon)
    writer.write("pane-final.txt", pane.read_pane(200) if pane else "")
    writer.write("requests-excerpt.json", excerpt)


def git_info(repo=REPO):
    def git(*args):
        return subprocess.run(["git", "-C", str(repo)] + list(args), stdout=subprocess.PIPE,
                              stderr=subprocess.DEVNULL, universal_newlines=True).stdout.strip()
    return {"sha": git("rev-parse", "HEAD"), "dirty": bool(git("status", "--porcelain"))}


def execute(args, env, info, binary, evidence_dir, owner, run_id, herdr_version, world_factory=None,
            backend_factory=None, leak_check=None):
    """Run the smoke in one World; write the evidence; return the exit code."""
    mode = "live" if args.live else "dry-run"
    backend = (backend_factory or relay_backend.RelayBackend)(
        mode=relay_backend.LIVE if args.live else relay_backend.FAKE,
        script=None if args.live else dry_run_script(),
        max_requests=LIVE_MAX_REQUESTS if args.live else None,
        max_cost_usd=LIVE_MAX_COST_USD if args.live else None)
    world = (world_factory or agyenv.World)(run_id, owner, binary, backend, 1.0)
    clock = Clock(backend)
    state = {"phases": [], "ack_phase": None}
    aborted, error, summary, writer = None, None, None, None
    started = time.monotonic()
    try:
        with world:
            try:
                info = dict(info, agy_version=world.agy_version(), agy_sha512=world.agy_info.get("sha512"),
                            herdr_version=herdr_version)
                smoke(world, backend, clock, state)
            except SmokeAbort as stop:
                aborted = stop.reason
                world.check("the smoke ran to a verdict", False, str(stop))
            except Exception as exc:  # a flow bug or environment failure: recorded, never a pass
                aborted = "error"
                error = traceback.format_exc()[-3000:]
                world.check("the smoke ran to a verdict", False, "%s: %s" % (type(exc).__name__, exc))
            summary = build_summary(info, world.checks, state["phases"], backend.budget(),
                                    state["ack_phase"], aborted, mode)
            writer = EvidenceWriter(evidence_dir, root=world.root, home=agyenv.real_home())
            collect_evidence(world, backend, state, summary, writer)
    finally:
        pass
    if error:
        sys.stderr.write(error)
    leak_ok, leak_output = (leak_check or agyenv.check_leaks)(run_id)
    summary["leak_check"] = {"ok": leak_ok, "output": leak_output[-1000:]}
    summary["duration_s"] = round(time.monotonic() - started, 1)
    summary["pass"] = bool(summary["pass"] and leak_ok and state["phases"])
    writer.write("summary.json", summary)
    writer.write("report.md", report_md(summary) + ("\nLeak check: %s\n" % ("clean" if leak_ok else "LEAKS")))
    try:
        writer.finish()
    except EvidenceError as exc:
        sys.stdout.write("agy-live-smoke: %s\n" % exc)
        return EXIT_FAIL
    for check in summary["checks"]:
        sys.stdout.write("%s  %s%s\n" % ("ok  " if check["ok"] else "FAIL", check["name"],
                                         "" if check["ok"] or not check["detail"] else " -- " + check["detail"]))
    sys.stdout.write("ack_phase: %s  requests: %s  cost_usd: %s  leak check: %s\nresult: %s%s\nevidence: %s\n" % (
        summary["ack_phase"], summary["requests"], summary["cost_usd"], "clean" if leak_ok else "LEAKS",
        "PASS" if summary["pass"] else "FAIL", " (aborted: %s)" % aborted if aborted else "", evidence_dir))
    if not leak_ok:
        sys.stdout.write(leak_output + "\n")
        return EXIT_LEAK
    return EXIT_OK if summary["pass"] else EXIT_FAIL


def main(argv=None, env=None, platform_name=None, herdr_version_fn=None, git_fn=None, build_fn=None,
         scratch_root=None, now=None, **kwargs):
    env = os.environ if env is None else env
    args = build_parser().parse_args(argv)
    info = (git_fn or git_info)()
    version_text = (herdr_version_fn or gate.herdr_version_text)()
    message = refusal(args, env, platform_name or sys.platform, info["dirty"], version_text)
    if message:
        sys.stdout.write("agy-live-smoke: %s\n" % message)
        return EXIT_REFUSED
    evidence_dir = Path(args.evidence_dir) if args.evidence_dir else default_evidence_dir(
        args.live, info["sha"], utc_date(now), scratch_root)
    try:
        binary = args.herdr_threads_bin or (build_fn or gate.build_herdr_threads)()
    except gate.Refusal as error:
        sys.stdout.write("agy-live-smoke: %s\n" % error)
        return EXIT_REFUSED
    owner, run_id = os.getpid(), str(uuid.uuid4())
    os.environ["HT_TEST_OWNER"] = str(owner)
    os.environ["HT_LEAK_RUN_ID"] = run_id
    previous = {}
    for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
        previous[sig] = signal.signal(sig, gate._raise_interrupted)
    try:
        return execute(args, env, info, binary, evidence_dir, owner, run_id, version_text, **kwargs)
    finally:
        for sig, handler in previous.items():
            signal.signal(sig, handler)


if __name__ == "__main__":
    sys.exit(main())
