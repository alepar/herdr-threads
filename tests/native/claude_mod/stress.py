#!/usr/bin/env python3
"""Live race stress of Claude mod delivery in real Claude Code TUI sessions (ht-j16.9).

Drives real Claude Code sessions in a private tmux server (`tmux -L ht-mod-<uuid>`) against a
signed-in profile used IN PLACE as CLAUDE_CONFIG_DIR (its login is bound to that path in the macOS
Keychain, so a copy or an isolated HOME is not signed in; the real HOME is kept). Nothing is written
to the profile's settings: the binary built from this checkout runs `setup claude` into a scratch
config dir, and the driver passes the generated hooks (plus its own Stop and UserPromptSubmit
scenario hooks) with `claude --settings <file>` and the mod with CLAUDE_CODE_PLUGIN_DIRS.

herdr-threads runs isolated: a private state dir and daemon, and a stand-in Herdr endpoint (a Unix
socket served by this driver, the protocol of tests/integration/sweep.rs `host_reply`) that knows two
fake panes, the sender's plain pane and the Claude pane. It records every call, so a native wake
(`agent.prompt`) is observed but never typed. Model replies are never asserted on content. Delivery is
asserted from the daemon database (read-only SQLite), the mod ledger (HERDR_THREADS_MOD_LEDGER, polled
and merged because a reload rewrites it) and the scenario hook log (UserPromptSubmit and Stop events).

Fallback: with no usable signed-in profile the driver runs scripts/test-claude-mod (the unit-level
randomized stress) and records the live gap in the summary and the evidence README.

Python 3 standard library only. Every process the run starts is stopped in `finally` and on
SIGINT/SIGTERM. Never touches the shared Herdr server, the user's daemon or the real ~/.claude.
"""

import argparse
import json
import os
import random
import re
import shutil
import signal
import socket
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import uuid

MIN_CLAUDE = "2.1.287"
SCENARIO_NAMES = [
    "busy_context", "idle_submit", "lazy_append", "queued_user_prompt", "stop_hook_continuation",
    "esc_interrupt_hold", "permission_dialog", "clear_rebind", "resume_keeps_set", "reload",
    "reload_mid_turn", "kill_watch_fallback_and_restream", "user_prompt_submit_block_drop",
    "denied_tool_with_pending_context",
]
DEFAULT_PROFILE = (
    "/private/tmp/claude-501/-Users-alepar-AleCode-herdr-threads/6ac583b8-5b3a-4ce5-9809-728b586f23fc/"
    "scratchpad/modspike/profile"
)
REPO = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", ".."))
SETTLED_OK = {"settled"}
PANE_A = "w1:p1"  # the sender: a plain pane the native ladder never prompts
PANE_B = "w1:p2"  # the Claude seat
BUSY_MARK = re.compile(r"esc to interrupt", re.I)


# ---------------------------------------------------------------- pure helpers (unit-tested)

def parse_ledger(text):
    """Ledger lines to dicts. Blank lines and a torn final line are skipped; a complete
    non-object line is an error."""
    rows = []
    lines = text.split("\n")
    for i, line in enumerate(lines):
        if not line.strip():
            continue
        try:
            obj = json.loads(line)
        except ValueError:
            if i == len(lines) - 1:
                continue
            raise
        if not isinstance(obj, dict):
            raise ValueError("ledger line is not an object: %r" % line)
        rows.append(obj)
    return rows


def check_ledger(entries, resets=(), truncated=(), holds=()):
    """Invariants over one session's mod ledger. Returns violation strings (empty is clean).

    resets: times at which the delivered set is legitimately discarded (/clear).
    truncated: ids whose body was cut; the mod must never ack them.
    holds: (start, end) intervals of a post-abort hold, end exclusive; no submit inside.
    """
    out = []
    resets = sorted(resets)
    truncated = set(truncated)
    delivered = {}
    settled = {}
    for e in entries:
        at, kind = e.get("at", 0), e.get("kind")
        seg = sum(1 for r in resets if r <= at)
        ids = e.get("ids") or []
        if kind == "delivered":
            for i in ids:
                delivered[(seg, i)] = delivered.get((seg, i), 0) + 1
                if delivered[(seg, i)] == 2:
                    out.append("duplicate delivered %s at %s" % (i, at))
        elif kind == "acked":
            for i in ids:
                if i in truncated:
                    out.append("ack for truncated %s at %s" % (i, at))
                if e.get("reason") in SETTLED_OK:
                    settled[(seg, i)] = settled.get((seg, i), 0) + 1
                    if settled[(seg, i)] == 2:
                        out.append("duplicate settling ack %s at %s" % (i, at))
        elif kind == "refused" and e.get("reason") == "ack:stale_generation":
            for i in ids:  # the delivery is forgotten, so one re-stream delivery is legitimate
                delivered.pop((seg, i), None)
                settled.pop((seg, i), None)
        elif kind == "submit":
            if e.get("turn") is not None:
                out.append("submit while turn %s open at %s" % (e.get("turn"), at))
            for start, end in holds:
                if start <= at < end:
                    out.append("submit inside hold [%s,%s) at %s" % (start, end, at))
    return out


def check_settled(sent, receipts, entries):
    """Every sent id ends acked or pending in the daemon (never missing), and a settling ack in
    the ledger agrees with the daemon."""
    out = []
    for i in sent:
        state = receipts.get(i)
        if state is None:
            out.append("lost message %s: no receipt" % i)
        elif state not in ("acked", "pending"):
            out.append("message %s ended in state %s" % (i, state))
    led_settled = {i for e in entries if e.get("kind") == "acked" and e.get("reason") in SETTLED_OK
                   for i in e.get("ids") or []}
    for i in sent:
        if i in led_settled and receipts.get(i) == "pending":
            out.append("ledger settled %s but daemon pending" % i)
    return out


def check_mod_settled(ids, receipts, entries):
    """Every id the daemon holds acked was settled by the mod (a ledger `acked`), not by another
    path such as the model running `herdr-threads inbox` while the mod's channel was down."""
    by_mod = {i for e in entries if e.get("kind") == "acked" and e.get("reason") in ("settled", "already_settled")
              for i in e.get("ids") or []}
    return ["%s acked outside the mod (no ledger ack)" % i for i in ids
            if receipts.get(i) == "acked" and i not in by_mod]


def turn_intervals(hook_events, aborts=()):
    """Main-turn intervals seen from outside, from the scenario hook log.

    A turn opens at a UserPromptSubmit the hook did not block and closes at the next Stop the hook
    did not block (a blocked Stop is a continuation inside the same turn), at the next abort (an
    Esc or a session exit the driver caused; neither fires Stop) or at the next SessionStart (a new,
    cleared or resumed session has no open turn). A prompt submitted while a turn is open (a queued
    prompt folded into it) does not open a second one. An interval still open at the end is closed
    at +infinity."""
    evs = sorted([(e["at"], e["event"], bool(e.get("blocked"))) for e in hook_events] +
                 [(a, "Abort", False) for a in aborts])
    out, start = [], None
    for at, ev, blocked in evs:
        if ev == "UserPromptSubmit" and not blocked and start is None:
            start = at
        elif ((ev == "Stop" and not blocked) or ev in ("Abort", "SessionStart")) and start is not None:
            out.append((start, at))
            start = None
    if start is not None:
        out.append((start, float("inf")))
    return out


def check_turn_overlap(entries, hook_events, aborts=()):
    """No mod `submit` while a main turn is open, judged by the hook log rather than the mod's own
    turn bookkeeping. A submit's own turn opens after it, so it never counts against itself."""
    out = []
    spans = turn_intervals(hook_events, aborts)
    for e in entries:
        if e.get("kind") != "submit":
            continue
        at = e.get("at", 0)
        for start, end in spans:
            if start < at < end:
                out.append("submit %s at %s inside a turn open [%s,%s) per the hook log"
                           % (",".join(e.get("ids") or []), at, start, end))
    return out


def merge_lines(seen, seen_set, text):
    """Adds the complete JSON lines of a ledger snapshot not seen before (the mod rewrites the
    whole file, and a reload starts a new file from an empty tail). Returns the number added."""
    added = 0
    lines = text.split("\n")
    for i, line in enumerate(lines):
        line = line.strip()
        if not line or line in seen_set:
            continue
        try:
            obj = json.loads(line)
        except ValueError:
            if i == len(lines) - 1:
                continue
            raise
        if isinstance(obj, dict):
            seen_set.add(line)
            seen.append(obj)
            added += 1
    return added


def version_at_least(have, want):
    h = [int(x) for x in re.findall(r"\d+", have)[:4]]
    w = [int(x) for x in re.findall(r"\d+", want)[:4]]
    h += [0] * (4 - len(h))
    w += [0] * (4 - len(w))
    return h >= w


def auth_output_usable(text):
    t = text.strip()
    return bool(t) and "not logged in" not in t.lower() and "/login" not in t


def parse_args(argv):
    p = argparse.ArgumentParser(prog="stress.py", description=__doc__.split("\n")[0])
    p.add_argument("--iterations", type=int, default=20)
    p.add_argument("--scenarios", default=",".join(SCENARIO_NAMES))
    p.add_argument("--profile", default=DEFAULT_PROFILE,
                   help="signed-in CLAUDE_CONFIG_DIR, used in place (never copied, settings never written)")
    p.add_argument("--evidence-dir", default=os.path.join(REPO, "docs", "evidence", "claude-mod-delivery"))
    p.add_argument("--seed", type=int, default=None)
    p.add_argument("--bin", default=os.path.join(REPO, "target", "debug", "herdr-threads"))
    p.add_argument("--claude", default=None, help="claude executable (default: first on PATH)")
    p.add_argument("--model", default=None, help="passed to claude --model")
    p.add_argument("--settle", type=float, default=90, help="seconds to wait for the iteration's acks")
    p.add_argument("--debug-file", action="store_true", help="claude --debug-file per session")
    p.add_argument("--keep-root", action="store_true", help="keep the private run dir for inspection")
    p.add_argument("--fallback-only", action="store_true", help="skip the live run, run the unit-level stress")
    a = p.parse_args(argv)
    if a.iterations < 1:
        p.error("--iterations must be >= 1")
    names = [s for s in a.scenarios.split(",") if s]
    bad = [s for s in names if s not in SCENARIO_NAMES]
    if bad:
        p.error("unknown scenario(s): %s" % ",".join(bad))
    a.scenarios = names
    return a


# ---------------------------------------------------------------- process bookkeeping

class Procs:
    """Everything the run starts; stop() is idempotent and runs on every exit path."""

    def __init__(self):
        self.tmux = []           # (socket, env)
        self.state_dirs = []     # (state, host socket)
        self.servers = []
        self.bin = None
        self.cli_env = None

    def run(self, argv, **kw):
        kw.setdefault("capture_output", True)
        kw.setdefault("text", True)
        kw.setdefault("timeout", 120)
        return subprocess.run(argv, **kw)

    def stop(self):
        for sock, env in self.tmux:
            subprocess.run(["tmux", "-L", sock, "kill-server"], capture_output=True, env=env)
        for sd, host in self.state_dirs:
            if self.bin:
                subprocess.run([self.bin, "daemon", "stop", "--state-dir", sd, "--host-endpoint", host],
                               capture_output=True, timeout=30, env=self.cli_env)
            # watch children poll getppid each second; anything left naming the state dir goes
            for pid in pids_naming(sd):
                try:
                    os.kill(pid, signal.SIGTERM)
                except OSError:
                    pass
        for s in self.servers:
            s.stop()
        self.tmux, self.state_dirs, self.servers = [], [], []


def pids_naming(path):
    out = subprocess.run(["ps", "-axo", "pid=,command="], capture_output=True, text=True).stdout
    me = os.getpid()
    return [int(l.split(None, 1)[0]) for l in out.splitlines()
            if path in l and int(l.split(None, 1)[0]) != me and "ps -axo" not in l]


# ---------------------------------------------------------------- stand-in Herdr

def plain_pane(pid, terminal):
    return {"pane_id": pid, "terminal_id": terminal, "workspace_id": "w1", "tab_id": "w1:t1",
            "focused": False, "agent_status": "idle", "revision": 3}


class StandInHerdr:
    """The private Herdr endpoint: one JSON request line per connection, one reply line. Same
    answers as tests/integration/sweep.rs `host_reply`; `agent.prompt` is recorded and answered
    as delivered but never typed into the pane."""

    def __init__(self, path, read_text):
        self.path = path
        self.read_text = read_text
        claude = plain_pane(PANE_B, "term-b")
        claude["agent"] = "claude"
        self.panes = [plain_pane(PANE_A, "term-a"), claude]
        self.calls = []
        self.lock = threading.Lock()
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.bind(path)
        self.sock.listen(64)
        self.sock.settimeout(0.2)
        self.stopped = False
        self.thread = threading.Thread(target=self.loop, daemon=True)
        self.thread.start()

    def loop(self):
        while not self.stopped:
            try:
                conn, _ = self.sock.accept()
            except socket.timeout:
                continue
            except OSError:
                return
            threading.Thread(target=self.serve, args=(conn,), daemon=True).start()

    def serve(self, conn):
        try:
            conn.settimeout(10)
            buf = b""
            while b"\n" not in buf:
                chunk = conn.recv(65536)
                if not chunk:
                    break
                buf += chunk
            req = json.loads(buf.split(b"\n", 1)[0].decode())
            reply = self.reply(req)
            conn.sendall((json.dumps(reply) + "\n").encode())
        except (OSError, ValueError):
            pass
        finally:
            conn.close()

    def find(self, pid):
        return next((p for p in self.panes if p["pane_id"] == pid), None)

    def reply(self, req):
        rid, method, params = req.get("id"), req.get("method", ""), req.get("params") or {}
        with self.lock:
            self.calls.append({"at": now_ms(), "method": method, "params": params})
        err = {"id": rid, "error": {"code": "agent_not_found", "message": "no agent in pane"}}
        if method == "ping":
            return {"id": rid, "result": {"type": "pong", "version": "0.9.1", "protocol": 22}}
        if method == "session.snapshot":
            return {"id": rid, "result": {"type": "session_snapshot", "snapshot": {
                "version": "0.9.1", "protocol": 22, "panes": self.panes, "agents": [],
                "tabs": [{"tab_id": "w1:t1", "workspace_id": "w1"}], "workspaces": [{"workspace_id": "w1"}],
                "layouts": []}}}
        if method in ("pane.current", "pane.get"):
            pane = self.find(params.get("caller_pane_id") if method == "pane.current" else params.get("pane_id"))
            if not pane:
                return {"id": rid, "error": {"code": "pane_not_found", "message": "pane not found"}}
            return {"id": rid, "result": {"type": "pane_current" if method == "pane.current" else "pane_info",
                                          "pane": pane}}
        pane = self.find(params.get("target"))
        if not pane or "agent" not in pane:
            return err
        if method == "agent.get":
            return {"id": rid, "result": {"type": "agent_info", "agent": {
                "agent": pane["agent"], "agent_status": pane["agent_status"], "focused": pane["focused"],
                "pane_id": pane["pane_id"], "terminal_id": pane["terminal_id"]}}}
        if method == "agent.read":
            return {"id": rid, "result": {"type": "pane_read", "read": {
                "pane_id": pane["pane_id"], "source": "detection", "text": self.read_text}}}
        if method == "agent.prompt":
            return {"id": rid, "result": {"type": "agent_prompted", "agent": {
                "agent": pane["agent"], "agent_status": "working", "pane_id": pane["pane_id"],
                "terminal_id": pane["terminal_id"]}}}
        return err

    def prompts(self, since=0):
        with self.lock:
            return [c for c in self.calls if c["method"] == "agent.prompt" and c["at"] >= since]

    def stop(self):
        self.stopped = True
        try:
            self.sock.close()
        except OSError:
            pass
        self.thread.join(2)
        try:
            os.unlink(self.path)
        except OSError:
            pass


# ---------------------------------------------------------------- scenario hooks

HOOK_SCRIPT = r'''import json, os, sys, time
event = sys.argv[1]
work, log = sys.argv[2], sys.argv[3]
try:
    payload = json.load(sys.stdin)
except ValueError:
    payload = {}
row = {"at": int(time.time() * 1000), "event": event, "session": payload.get("session_id")}
if event == "SessionStart":
    row["source"] = payload.get("source")
out = None
if event == "Stop":
    row["stop_hook_active"] = payload.get("stop_hook_active")
    m = os.path.join(work, "stop-armed")
    if os.path.exists(m):
        os.remove(m)
        out = {"decision": "block", "reason": "Stop-hook continuation for the stress driver: now reply with the single word DELTA."}
elif event == "UserPromptSubmit":
    prompt = payload.get("prompt") or ""
    row["prompt"] = prompt[:120]
    if os.path.exists(os.path.join(work, "block-ups")):
        out = {"decision": "block", "reason": "blocked by the stress driver"}
row["blocked"] = out is not None
with open(log, "a") as f:
    f.write(json.dumps(row) + "\n")
if out:
    print(json.dumps(out))
'''


# ---------------------------------------------------------------- live driver

class Live:
    def __init__(self, args, root, procs):
        self.args, self.root, self.procs = args, root, procs
        self.rng = random.Random(args.seed)
        self.sock = "ht-mod-" + uuid.uuid4().hex[:10]
        self.state = os.path.join(root, "state")
        self.host_sock = os.path.join(root, "h.sock")
        self.home = os.path.join(root, "home")
        self.scratch_cfg = os.path.join(root, "cfg")
        self.work = os.path.join(root, "work")
        self.bindir = os.path.join(root, "bin")
        self.hooklog = os.path.join(root, "hooks.jsonl")
        self.out = os.path.join(root, "out")
        self.cfg = args.profile
        self.claude = args.claude or shutil.which("claude")
        self.sent = []           # every require-ack message id
        self.lazy = []           # every lazy message id
        self.truncated = set()
        self.resets = []         # ms timestamps of /clear
        self.holds = []          # (start, end) ms of post-abort holds
        self.aborts = []         # ms timestamps of Esc presses that abort a turn
        self.sessions = {}       # tmux session -> ledger path
        self.ledgers = {}        # tmux session -> (rows, line set)
        self.scenario_of = {}    # tmux session -> scenario
        self.a = self.b = self.thread = None
        self.mod_dir = None
        self.poll_stop = threading.Event()
        self.watch_seen = {}     # pid -> {pid, session, first_seen}
        self.notes = []          # per-iteration observations that are not invariants
        for d in (self.state, self.home, self.scratch_cfg, self.work, self.bindir, self.out):
            os.makedirs(d, exist_ok=True)

    # ---- environments

    def cli_env(self):
        """herdr-threads CLI calls from the driver: isolated HOME and scratch Claude config."""
        e = {k: v for k, v in os.environ.items() if not k.startswith(("HERDR_", "CLAUDE"))}
        e.update(HOME=self.home, CLAUDE_CONFIG_DIR=self.scratch_cfg, CODEX_HOME=os.path.join(self.home, ".codex"),
                 HERDR_THREADS_OFFLINE="1", NO_COLOR="1", HT_TEST_OWNER=str(os.getpid()))
        return e

    def claude_env(self):
        """The tmux server's environment, inherited by every Claude session: the REAL HOME (the
        Keychain login needs it), the profile in place, the isolated herdr-threads instance named
        so that any `herdr-threads` the model runs reaches it (built binary first on PATH)."""
        e = {k: v for k, v in os.environ.items() if not k.startswith(("HERDR_", "CLAUDE", "TMUX"))}
        e.update(CLAUDE_CONFIG_DIR=self.cfg, DISABLE_AUTOUPDATER="1", HERDR_ENV="1", HERDR_PANE_ID=PANE_B,
                 HERDR_SOCKET_PATH=self.host_sock, HERDR_PLUGIN_STATE_DIR=self.state,
                 CLAUDE_CODE_PLUGIN_DIRS=self.mod_dir or "", HERDR_THREADS_OFFLINE="1",
                 PATH=self.bindir + ":" + os.environ.get("PATH", "/usr/bin:/bin"),
                 HT_TEST_OWNER=str(os.getpid()))
        return e

    def ht(self, *argv, caller=None, check=True):
        full = [self.args.bin, "--json", "--state-dir", self.state, "--host-endpoint", self.host_sock]
        if caller:
            seat, pane = caller
            full += ["--cooperative-seat", seat, "--cooperative-target", pane,
                     "--cooperative-harness", "claude", "--cooperative-role", "top-level"]
        r = self.procs.run(full + list(argv), env=self.cli_env())
        if check and r.returncode != 0:
            raise RuntimeError("herdr-threads %s failed (%s): %s" % (" ".join(argv), r.returncode,
                                                                    (r.stdout + r.stderr)[:600]))
        try:
            r.value = json.loads(r.stdout)
        except ValueError:
            r.value = None
        return r

    def data(self, r):
        return (r.value or {}).get("result", {}).get("data")

    def tmux(self, *argv):
        return self.procs.run(["tmux", "-L", self.sock, *argv], env=self.tenv).stdout

    # ---- setup

    def setup(self):
        self.procs.bin = self.args.bin
        self.procs.cli_env = self.cli_env()
        os.symlink(self.args.bin, os.path.join(self.bindir, "herdr-threads"))
        with open(os.path.join(REPO, "docs", "evidence", "poke-spike", "captures",
                               "claude-q1-empty.read-detection.txt")) as f:
            read_text = f.read()
        self.host = StandInHerdr(self.host_sock, read_text)
        self.procs.servers.append(self.host)
        self.procs.state_dirs.append((self.state, self.host_sock))
        # setup into the scratch config dir; the profile's settings.json is never written
        r = self.ht("setup", "claude", "--keep-prompt-suggestions", "--harness-binary", self.claude)
        setup = (r.value or {}).get("setup") or {}
        self.mod_dir = (setup.get("mod") or {}).get("dir")
        if not self.mod_dir or not os.path.isfile(os.path.join(self.mod_dir, "hooks", "register.js")):
            raise RuntimeError("setup claude installed no mod: %s" % r.stdout[:600])
        with open(os.path.join(self.scratch_cfg, "settings.json")) as f:
            settings = json.load(f)
        hook_py = os.path.join(self.root, "stress_hook.py")
        with open(hook_py, "w") as f:
            f.write(HOOK_SCRIPT)
        for ev in ("Stop", "UserPromptSubmit", "SessionStart"):
            cmd = "%s %s %s %s %s" % (shell_quote(sys.executable), shell_quote(hook_py), ev,
                                      shell_quote(self.work), shell_quote(self.hooklog))
            settings.setdefault("hooks", {}).setdefault(ev, []).append(
                {"hooks": [{"type": "command", "command": cmd, "timeout": 10}]})
        self.settings = os.path.join(self.root, "settings.json")
        with open(self.settings, "w") as f:
            json.dump(settings, f, indent=1)
        self.ht("daemon", "ensure")
        self.a = self.data(self.ht("seat", "resolve", "--pane", PANE_A))
        self.b = self.data(self.ht("seat", "resolve", "--pane", PANE_B))
        self.ht("check-in", "--lifecycle-event", "stress-a-start", caller=(self.a, PANE_A))
        self.thread = self.data(self.ht("thread", "create", "--topic", "mod stress", caller=(self.a, PANE_A)))
        self.ht("invite", self.thread, "--seat", self.b, caller=(self.a, PANE_A))
        self.accepted = False
        self.tenv = self.claude_env()
        self.procs.tmux.append((self.sock, self.tenv))
        threading.Thread(target=self.poll_ledgers, daemon=True).start()

    # ---- claude sessions

    def start_claude(self, name, scenario, extra=()):
        led = os.path.join(self.root, "ledger-%s.jsonl" % name)
        self.sessions[name] = led
        self.ledgers.setdefault(name, ([], set()))
        self.scenario_of[name] = scenario
        argv = [self.claude, "--settings", self.settings, "--permission-mode", "default"]
        if self.args.model:
            argv += ["--model", self.args.model]
        if self.args.debug_file:
            argv += ["--debug-file", os.path.join(self.root, "debug-%s.log" % name)]
        argv += list(extra)
        self.tmux("new-session", "-d", "-s", name, "-x", "160", "-y", "50", "-c", self.work,
                  "-e", "HERDR_THREADS_MOD_LEDGER=" + led, " ".join(shell_quote(a) for a in argv))
        end = time.time() + 60
        while time.time() < end:
            t = self.pane_text(name)
            if re.search(r"Yes, I trust this folder", t):  # the default option is "No, exit"
                if re.search(r"❯ Yes, I trust this folder", t):
                    self.tmux("send-keys", "-t", name, "Enter")
                    time.sleep(2)
                else:
                    time.sleep(1.5)  # keys sent before the dialog takes input are lost
                    self.tmux("send-keys", "-t", name, "Down")
                    time.sleep(0.7)
                continue
            if "❯" in t and not BUSY_MARK.search(t):
                break
            time.sleep(0.5)
        else:
            raise RuntimeError("claude did not reach the prompt in %s:\n%s" % (name, self.pane_text(name)[-1500:]))
        # the mod connects once SessionStart's check-in has committed
        self.wait_ledger(name, lambda e: e.get("kind") in ("received", "restart", "refused", "held"), 1, 20)
        if not self.accepted:
            # B's lifecycle check-in is the first session's SessionStart hook
            end = time.time() + 30
            while True:
                r = self.ht("accept", self.thread, caller=(self.b, PANE_B), check=time.time() > end)
                if r.returncode == 0:
                    break
                time.sleep(1)
            self.accepted = True
        # a pending attention item is submitted once per watch run: let that turn finish first
        time.sleep(3)
        self.wait_idle(name)
        return led

    def stop_claude(self, name):
        self.wait_idle(name, 60)
        self.aborts.append(now_ms())  # an exit fires no Stop: close any turn the hook log has open
        self.tmux("send-keys", "-t", name, "C-c")
        time.sleep(0.4)
        self.tmux("send-keys", "-t", name, "C-c")
        end = time.time() + 15
        while time.time() < end and name in self.tmux("list-sessions", "-F", "#{session_name}").split():
            time.sleep(0.3)
        self.tmux("kill-session", "-t", name)

    def pane_text(self, name):
        return self.tmux("capture-pane", "-p", "-J", "-t", name)

    def wait_pane(self, name, pattern, timeout):
        end = time.time() + timeout
        while time.time() < end:
            if re.search(pattern, self.pane_text(name)):
                return True
            time.sleep(0.3)
        return False

    def busy(self, name):
        return bool(BUSY_MARK.search(self.pane_text(name)))

    def wait_busy(self, name, timeout=30):
        end = time.time() + timeout
        while time.time() < end:
            if self.busy(name):
                return True
            time.sleep(0.2)
        return False

    def wait_idle(self, name, timeout=120, quiet=1.5):
        end, since = time.time() + timeout, None
        while time.time() < end:
            if self.busy(name):
                since = None
            elif since is None:
                since = time.time()
            elif time.time() - since >= quiet:
                return True
            time.sleep(0.3)
        return False

    def type_prompt(self, name, text):
        self.tmux("send-keys", "-t", name, "-l", text)
        time.sleep(0.3)
        self.tmux("send-keys", "-t", name, "Enter")

    def escape(self, name):
        self.aborts.append(now_ms())
        self.tmux("send-keys", "-t", name, "Escape")
        time.sleep(1)
        # an interrupt before the first response puts the prompt back in the box; clear it
        self.tmux("send-keys", "-t", name, "C-e")
        self.tmux("send-keys", "-t", name, "C-u")

    # ---- hook log and ledgers

    def hook_events(self):
        try:
            with open(self.hooklog) as f:
                return [json.loads(l) for l in f if l.strip()]
        except FileNotFoundError:
            return []

    def stops(self):
        return sum(1 for e in self.hook_events() if e["event"] == "Stop" and not e.get("blocked"))

    def wait_stops(self, before, n=1, timeout=120):
        end = time.time() + timeout
        while time.time() < end:
            if self.stops() >= before + n:
                return True
            time.sleep(0.3)
        return False

    def poll_ledgers(self):
        last_ps = 0
        while not self.poll_stop.is_set():
            if time.time() - last_ps >= 0.5:  # which session id each watch child was started with
                last_ps = time.time()
                for pid, sid in self.watch_children():
                    if pid not in self.watch_seen:
                        self.watch_seen[pid] = {"pid": pid, "session": sid, "first_seen": now_ms()}
            for name, path in list(self.sessions.items()):
                try:
                    with open(path) as f:
                        text = f.read()
                except FileNotFoundError:
                    continue
                rows, seen = self.ledgers[name]
                try:
                    merge_lines(rows, seen, text)
                except ValueError:
                    pass
            time.sleep(0.1)

    def ledger(self, name):
        return sorted(self.ledgers.get(name, ([], set()))[0], key=lambda e: e.get("at", 0))

    def wait_ledger(self, name, pred, n=1, timeout=30):
        end = time.time() + timeout
        while time.time() < end:
            if sum(1 for e in self.ledger(name) if pred(e)) >= n:
                return True
            time.sleep(0.2)
        return False

    # ---- daemon side

    def send(self, tag, lazy=False):
        body = ("stress %s: an automated delivery test message. No action is needed; do not run any "
                "commands, just reply with the single word ok." % tag)
        argv = ["send", self.thread, "--body", body]
        if not lazy:
            argv += ["--require-ack", self.b]
        mid = self.data(self.ht(*argv, caller=(self.a, PANE_A)))
        (self.lazy if lazy else self.sent).append(mid)
        return mid

    def db(self):
        path = os.path.join(self.state, "herdr-threads.sqlite3")
        if not os.path.exists(path):
            cands = [os.path.join(d, f) for d, _, fs in os.walk(self.state) for f in fs
                     if f.endswith((".sqlite3", ".sqlite", ".db"))]
            path = cands[0] if cands else path
        con = sqlite3.connect("file:%s?mode=ro" % path, uri=True, timeout=5)
        return con

    def receipts(self, ids):
        """id -> settled receipt state, `pending` while no settled row, None when the daemon has no
        message with that id at all."""
        con = self.db()
        try:
            out = {}
            for mid in ids:
                state = None
                for table in ("receipt_state", "receipts"):
                    row = con.execute("SELECT state FROM %s WHERE message_id=? AND seat_id=? AND state<>'pending'"
                                      % table, (mid, self.b)).fetchone()
                    if row:
                        state = row[0]
                        break
                if state is None:
                    known = con.execute("SELECT 1 FROM messages WHERE id=?", (mid,)).fetchone()
                    state = "pending" if known else None
                out[mid] = state
            return out
        finally:
            con.close()

    def lazy_states(self, ids):
        con = self.db()
        try:
            return {m: (con.execute("SELECT state FROM lazy_recipients WHERE message_id=? AND seat_id=?",
                                    (m, self.b)).fetchone() or [None])[0] for m in ids}
        finally:
            con.close()

    def wait_settled(self, ids, lazy_ids=(), timeout=90):
        end = time.time() + timeout
        while time.time() < end:
            rc = self.receipts(ids)
            lz = self.lazy_states(lazy_ids)
            if all(rc.get(i) == "acked" for i in ids) and all(s == "displayed" for s in lz.values()):
                return True
            time.sleep(0.5)
        return False

    def watch_children(self):
        out = self.procs.run(["ps", "-axo", "pid=,command="]).stdout
        found = []
        for l in out.splitlines():
            m = re.search(r"watch --harness claude --session (\S+)", l)
            if m and self.state in l:
                found.append((int(l.split(None, 1)[0]), m.group(1)))
        return found

    def watch_pids(self):
        return [pid for pid, _ in self.watch_children()]

    def channels(self):
        r = self.ht("doctor", check=False)
        try:
            return r.value["doctor"]["harness_states"]["mod_channels"]
        except (TypeError, KeyError):
            return {"unreadable": (r.stdout or r.stderr)[:400]}

    def capture(self, name, tag):
        path = os.path.join(self.out, "pane-%s-%s.txt" % (name, tag))
        with open(path, "w") as f:
            f.write(self.pane_text(name))
        return path


def shell_quote(s):
    return "'" + str(s).replace("'", "'\\''") + "'"


def now_ms():
    return int(time.time() * 1000)


# Scenario functions: each performs one iteration against the Claude session `s` and returns the
# tmux session that is current afterwards (resume replaces it). They send, perturb the TUI and
# return; settlement and invariants are asserted by the caller from receipts, ledger and hook log.

def sc_busy_context(L, s, i):
    n = L.stops()
    L.type_prompt(s, "Run `sleep 8` with the Bash tool, then reply with the single word done.")
    L.wait_pane(s, r"\$ sleep 8", 40)  # the running tool, not the typed prompt
    time.sleep(1)
    L.send("busy-%d" % i)  # lands while the sleep runs: rides the tool result as context
    L.wait_stops(n, 1, 120)
    return s


def sc_idle_submit(L, s, i):
    L.wait_idle(s)
    n = L.stops()
    L.send("idle-%d" % i)  # idle: one framed $.prompt.submit starts a turn
    L.wait_stops(n, 1, 120)
    return s


def sc_lazy_append(L, s, i):
    L.wait_idle(s)
    L.send("lazy-%d" % i, lazy=True)  # $.session.append, starts no turn
    return s


def sc_queued_user_prompt(L, s, i):
    n = L.stops()
    L.type_prompt(s, "Run `sleep 6` with the Bash tool, then reply with the single word done.")
    L.wait_pane(s, r"\$ sleep 6", 40)  # the running tool, not the typed prompt
    L.type_prompt(s, "Also reply with the word BRAVO (queued user prompt %d)." % i)
    L.send("queued-%d" % i)
    L.wait_stops(n, 1, 120)
    L.wait_idle(s)
    return s


def sc_stop_hook_continuation(L, s, i):
    open(os.path.join(L.work, "stop-armed"), "w").close()
    n = L.stops()
    L.type_prompt(s, "Reply with the single word ALPHA.")
    L.wait_busy(s, 15)
    L.send("stop-%d" % i)  # sent inside the turn; the Stop hook keeps the turn going once
    L.wait_stops(n, 1, 120)
    L.wait_idle(s)
    return s


def end_hold(L, s, start):
    """Ends a post-abort hold with an ordinary user turn. The hold cannot legally end before that
    turn is typed, so the recorded interval ends there (conservative)."""
    time.sleep(3)
    L.holds.append((start, now_ms()))
    n = L.stops()
    L.type_prompt(s, "Reply with the single word OK.")
    L.wait_stops(n, 1, 120)


def sc_esc_interrupt_hold(L, s, i):
    L.type_prompt(s, "Run `sleep 20` with the Bash tool.")
    L.wait_pane(s, r"\$ sleep 20", 40)  # the running tool, not the typed prompt
    time.sleep(1)
    start = now_ms()
    L.escape(s)
    L.send("esc-%d" % i)
    end_hold(L, s, start)
    return s


def sc_permission_dialog(L, s, i):
    n = L.stops()
    L.type_prompt(s, "Create a file named perm-%d.txt containing x using the Write tool, then reply done." % i)
    seen = L.wait_pane(s, r"Do you want to (create|make|write)", 60)
    L.send("perm-%d" % i)  # arrives while the dialog is open
    time.sleep(2)
    if seen:
        L.tmux("send-keys", "-t", s, "Enter")  # approve: the answered result carries the context
    L.wait_stops(n, 1, 120)
    return s


def sc_clear_rebind(L, s, i):
    L.wait_idle(s)
    n = L.stops()
    L.send("preclear-%d" % i)
    L.wait_stops(n, 1, 120)
    L.wait_idle(s)
    L.resets.append(now_ms())
    L.type_prompt(s, "/clear")
    time.sleep(3)
    n = L.stops()
    L.send("postclear-%d" % i)  # after the check-in rotated the generation
    L.wait_stops(n, 1, 120)
    return s


def sc_resume_keeps_set(L, s, i):
    L.wait_idle(s)
    n = L.stops()
    L.send("preresume-%d" % i)
    L.wait_stops(n, 1, 120)
    L.wait_idle(s)
    L.stop_claude(s)
    r = "%s-r%d" % (s, i)
    L.start_claude(r, L.scenario_of[s], ("--continue",))
    n = L.stops()
    L.send("postresume-%d" % i)
    L.wait_stops(n, 1, 120)
    return r


def sc_reload(L, s, i):
    L.wait_idle(s)
    L.send("prereload-%d" % i)
    time.sleep(0.5)
    L.type_prompt(s, "/reload-plugins")
    time.sleep(4)
    L.send("postreload-%d" % i)
    L.wait_idle(s)
    return s


def sc_reload_mid_turn(L, s, i):
    # /reload-plugins typed during a turn is queued by the TUI until the turn ends, so the reload
    # is caused by a save of the installed module instead (the plugin-dir watch hot-reloads it).
    # Claude Code 2.1.295 applies that reload at the turn boundary too (the note records where).
    n = L.stops()
    L.type_prompt(s, "Run `sleep 25` with the Bash tool, then reply with the single word done.")
    L.wait_pane(s, r"\$ sleep 25", 40)  # the running tool, not the typed prompt
    mid = L.send("midturn-%d" % i)
    L.wait_ledger(s, lambda e: e.get("kind") == "received" and mid in (e.get("ids") or []), 1, 10)
    before = set(L.watch_seen)
    t = now_ms()
    with open(os.path.join(L.mod_dir, "hooks", "register.js"), "a") as f:
        f.write("// stress reload %s\n" % uuid.uuid4().hex[:8])
    end = time.time() + 40
    while time.time() < end and not (set(L.watch_seen) - before):
        time.sleep(0.3)
    new = [L.watch_seen[p]["first_seen"] for p in set(L.watch_seen) - before]
    L.wait_stops(n, 1, 120)
    stop_at = max([e["at"] for e in L.hook_events() if e["event"] == "Stop"] or [0])
    L.notes.append("reload_mid_turn %d: module saved at %d, new watch child at %s, turn ended at %d (%s)"
                   % (i, t, min(new) if new else None, stop_at,
                      "applied mid-turn" if new and min(new) < stop_at else "applied at the turn boundary"))
    L.wait_idle(s)
    return s


def sc_kill_watch(L, s, i):
    # a message the mod has received (queued for a context ride) when its watch child dies
    n = L.stops()
    L.type_prompt(s, "Run `sleep 8` with the Bash tool, then reply with the single word done.")
    L.wait_pane(s, r"\$ sleep 8", 40)  # the running tool, not the typed prompt
    mid = L.send("prekill-%d" % i)
    L.wait_ledger(s, lambda e: e.get("kind") == "received" and mid in (e.get("ids") or []), 1, 10)
    for pid in L.watch_pids():
        try:
            os.kill(pid, signal.SIGKILL)
        except OSError:
            pass
    time.sleep(1)
    L.send("postkill-%d" % i)  # while the child restarts: the daemon re-streams what is unacked
    L.wait_stops(n, 1, 120)
    L.wait_idle(s)
    return s


def sc_ups_block_drop(L, s, i):
    L.wait_idle(s)
    marker = os.path.join(L.work, "block-ups")
    open(marker, "w").close()  # the scenario UserPromptSubmit hook blocks while it exists
    mid = L.send("upsblock-%d" % i)
    L.wait_ledger(s, lambda e: e.get("kind") == "refused" and mid in (e.get("ids") or []), 1, 20)
    os.remove(marker)
    n = L.stops()
    L.wait_stops(n, 1, 90)  # the 30 s drop backoff, then a submit that goes through
    return s


def sc_denied_tool(L, s, i):
    L.type_prompt(s, "Run `rm -rf /tmp/ht-stress-nonexistent-%d` with the Bash tool." % i)
    seen = L.wait_pane(s, r"Do you want to proceed", 60)
    L.send("denied-%d" % i)  # waits for a tool result; the denied one must not carry it
    time.sleep(2)
    start = now_ms()
    if seen:
        L.escape(s)
    end_hold(L, s, start)
    return s


SCENARIOS = {
    "busy_context": sc_busy_context, "idle_submit": sc_idle_submit, "lazy_append": sc_lazy_append,
    "queued_user_prompt": sc_queued_user_prompt, "stop_hook_continuation": sc_stop_hook_continuation,
    "esc_interrupt_hold": sc_esc_interrupt_hold, "permission_dialog": sc_permission_dialog,
    "clear_rebind": sc_clear_rebind, "resume_keeps_set": sc_resume_keeps_set, "reload": sc_reload,
    "reload_mid_turn": sc_reload_mid_turn, "kill_watch_fallback_and_restream": sc_kill_watch,
    "user_prompt_submit_block_drop": sc_ups_block_drop, "denied_tool_with_pending_context": sc_denied_tool,
}
assert sorted(SCENARIOS) == sorted(SCENARIO_NAMES)


def paths_of(entries, ids):
    """via of each delivered id, as the ledger recorded it."""
    out = {}
    for e in entries:
        if e.get("kind") == "delivered":
            for i in e.get("ids") or []:
                if i in ids:
                    out.setdefault(i, []).append(e.get("via"))
    return out


def run_live(args, root, procs):
    L = Live(args, root, procs)
    L.setup()
    summary = {"mode": "live", "iterations": args.iterations, "seed": args.seed, "scenarios": {},
               "profile": "signed-in profile in place (CLAUDE_CONFIG_DIR), real HOME",
               "herdr": "stand-in endpoint (fake panes %s sender, %s claude)" % (PANE_A, PANE_B)}
    try:
        for name in args.scenarios:
            s = "s-" + name.replace("_", "-")[:24]
            L.start_claude(s, name)
            sessions = [s]
            passes, failures, iters = 0, [], []
            reported = set()
            for i in range(args.iterations):
                before, before_lazy, t0 = len(L.sent), len(L.lazy), now_ms()
                err = None
                try:
                    s = SCENARIOS[name](L, s, i)
                except Exception as e:  # a driver step failed: recorded, the run goes on
                    err = "%s: %s" % (type(e).__name__, e)
                if s not in sessions:
                    sessions.append(s)
                ids, lazy_ids = L.sent[before:], L.lazy[before_lazy:]
                timeout = args.settle + (60 if name == "user_prompt_submit_block_drop" else 0)
                ok = L.wait_settled(ids, lazy_ids, timeout)
                time.sleep(1)
                led = sorted([e for n in sessions for e in L.ledger(n)], key=lambda e: e.get("at", 0))
                v = check_ledger(led, L.resets, L.truncated, L.holds)
                v += check_turn_overlap(led, L.hook_events(), L.aborts)
                rc = L.receipts(ids)
                v += check_settled(ids, rc, led)
                v += check_mod_settled(ids, rc, led)
                v = [x for x in v if x not in reported]
                reported.update(v)
                if not ok:
                    v.append("not settled within %ss: receipts %s lazy %s" % (timeout, rc, L.lazy_states(lazy_ids)))
                    sids = sorted({e.get("session") for e in L.hook_events() if e.get("at", 0) >= t0} - {None})
                    v.append("diagnostic: hook-log session ids since the iteration began %s; watch children %s; "
                             "mod channels %s" % (sids, [w for w in L.watch_seen.values() if w["first_seen"] >= t0][-6:],
                                                  json.dumps(L.channels())[:1200]))
                if err:
                    v.append("driver step: " + err)
                pane = L.capture(s, "%s-%d" % (name, i))
                notes, L.notes = L.notes, []
                it = {"notes": notes, "iteration": i, "ids": ids, "lazy": lazy_ids, "paths": paths_of(led, set(ids + lazy_ids)),
                      "receipts": rc, "lazy_states": L.lazy_states(lazy_ids), "settled": ok,
                      "native_prompts": len(L.host.prompts(t0)), "seconds": round((now_ms() - t0) / 1000, 1)}
                if v:
                    it["violations"] = v
                    it["pane_tail"] = L.pane_text(s).rstrip().split("\n")[-25:]
                    it["pane_file"] = os.path.basename(pane)
                    failures.append(it)
                else:
                    passes += 1
                iters.append(it)
                print("[%s %d] %s %s" % (name, i, "PASS" if not v else "FAIL", v if v else it["paths"]), flush=True)
            summary["scenarios"][name] = {"pass": passes, "fail": len(failures), "iterations": iters}
            for n in sessions:
                L.stop_claude(n)
    finally:
        L.poll_stop.set()
        summary["sent"] = len(L.sent)
        summary["sent_lazy"] = len(L.lazy)
        summary["native_prompts_total"] = len(L.host.prompts())
        live_dir = os.path.join(args.evidence_dir, "live")
        os.makedirs(live_dir, exist_ok=True)
        for n in L.sessions:
            with open(os.path.join(live_dir, "ledger-%s.jsonl" % n), "w") as f:
                for e in L.ledger(n):
                    f.write(json.dumps(e, sort_keys=True) + "\n")
        if os.path.exists(L.hooklog):
            shutil.copy(L.hooklog, os.path.join(live_dir, "hooks.jsonl"))
        for fn in os.listdir(L.out):
            shutil.copy(os.path.join(L.out, fn), os.path.join(live_dir, fn))
        with open(os.path.join(live_dir, "watch-children.json"), "w") as f:
            json.dump(sorted(L.watch_seen.values(), key=lambda w: w["first_seen"]), f, indent=1)
        with open(os.path.join(live_dir, "herdr-calls.json"), "w") as f:
            json.dump([c for c in L.host.calls if c["method"] in ("agent.prompt",)], f, indent=1)
        try:
            with open(os.path.join(live_dir, "receipts.json"), "w") as f:
                json.dump({"acked_require": L.receipts(L.sent), "lazy": L.lazy_states(L.lazy)}, f, indent=1,
                          sort_keys=True)
        except sqlite3.Error as e:
            summary["receipts_error"] = str(e)
    return summary


def preflight(args, procs):
    """Returns (usable, reason, claude_version). The profile is used in place, with the real HOME."""
    claude = args.claude or shutil.which("claude")
    if claude is None:
        return False, "claude CLI not found on PATH", None
    out = procs.run([claude, "--version"]).stdout
    m = re.match(r"([0-9][0-9.]*)", out.strip())
    version = m.group(1) if m else None
    if not version or not version_at_least(version, MIN_CLAUDE):
        return False, "claude %s older than %s" % (version, MIN_CLAUDE), version
    if shutil.which("tmux") is None:
        return False, "tmux not found", version
    if not os.path.isdir(args.profile):
        return False, "profile %s missing" % args.profile, version
    if not os.path.isfile(args.bin):
        return False, "herdr-threads binary %s missing (nice cargo build --locked --all-features)" % args.bin, version
    env = {k: v for k, v in os.environ.items() if not k.startswith(("HERDR_", "CLAUDE"))}
    env.update(CLAUDE_CONFIG_DIR=args.profile, DISABLE_AUTOUPDATER="1")
    try:
        r = procs.run([claude, "-p", "Reply with the single word ok."], env=env, timeout=120)
    except subprocess.TimeoutExpired:
        return False, "signed-in probe timed out", version
    text = (r.stdout or "") + (r.stderr or "")
    if r.returncode != 0 or not auth_output_usable(r.stdout or ""):
        return False, "profile is not signed in (claude -p: %s)" % text.strip()[:120], version
    return True, "ok", version


def run_fallback(reason):
    r = subprocess.run(["sh", os.path.join(REPO, "scripts", "test-claude-mod")], capture_output=True, text=True, timeout=900)
    tail = [l for l in r.stdout.splitlines() if re.search(r"\d+ (pass|fail)|Ran \d+|skipped|Validation", l)]
    return {"mode": "fallback", "live_gap": reason, "unit_exit": r.returncode, "unit_summary": tail,
            "unit_output": r.stdout}


def main(argv=None):
    args = parse_args(sys.argv[1:] if argv is None else argv)
    procs = Procs()
    # short: Unix socket paths (the stand-in Herdr, the daemon's) must stay under 104 bytes
    root = tempfile.mkdtemp(prefix="ht-ms-", dir="/private/tmp")
    os.makedirs(args.evidence_dir, exist_ok=True)

    def on_signal(signum, _frame):
        procs.stop()
        sys.exit(128 + signum)

    signal.signal(signal.SIGINT, on_signal)
    signal.signal(signal.SIGTERM, on_signal)
    try:
        sha = subprocess.run(["git", "-C", REPO, "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
        if args.fallback_only:
            usable, reason, version = False, "--fallback-only", None
        else:
            usable, reason, version = preflight(args, procs)
        started = time.time()
        summary = run_live(args, root, procs) if usable else run_fallback(reason)
        summary.update(status="LIVE" if summary["mode"] == "live" else "FALLBACK", sha=sha, claude_version=version, finished=time.strftime("%Y-%m-%dT%H:%M:%S%z"),
                       minutes=round((time.time() - started) / 60, 1))
        with open(os.path.join(args.evidence_dir, "summary.json"), "w") as f:
            json.dump(summary, f, indent=1, sort_keys=True)
        brief = {k: v for k, v in summary.items() if k not in ("unit_output", "scenarios")}
        if summary["mode"] == "live":
            brief["scenarios"] = {n: "%d/%d" % (s["pass"], s["pass"] + s["fail"]) for n, s in summary["scenarios"].items()}
        print(json.dumps(brief, indent=1, sort_keys=True))
        failed = (summary["mode"] == "live" and any(s["fail"] for s in summary["scenarios"].values())) or \
                 (summary["mode"] == "fallback" and summary["unit_exit"] != 0)
        return 1 if failed else 0
    finally:
        procs.stop()
        if args.keep_root:
            print("kept run dir:", root)
        else:
            shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
