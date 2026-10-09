#!/usr/bin/env python3
"""Live race stress of Claude mod delivery in real Claude Code TUI sessions (ht-j16.9).

Drives real Claude Code sessions in a private tmux server (`tmux -L ht-mod-<uuid>`) with a COPY of a
signed-in profile as CLAUDE_CONFIG_DIR, the mod installed by the built binary's `setup claude`, an
isolated herdr-threads state dir, and sends from a second seat through `herdr-threads send`. Model
replies are never asserted on content. Delivery is asserted from the daemon database (read-only
SQLite) and the mod ledger (HERDR_THREADS_MOD_LEDGER).

Fallback: with no usable signed-in profile the driver runs scripts/test-claude-mod (the unit-level
randomized stress) and records the live gap in the summary and the evidence README.

Python 3 standard library only. Every process the run starts is stopped in `finally` and on
SIGINT/SIGTERM. Never touches the shared Herdr server or the real ~/.claude.
"""

import argparse
import json
import os
import random
import re
import shutil
import signal
import sqlite3
import subprocess
import sys
import tempfile
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
    p.add_argument("--profile", default=DEFAULT_PROFILE)
    p.add_argument("--evidence-dir", default=os.path.join(REPO, "docs", "evidence", "claude-mod-delivery"))
    p.add_argument("--seed", type=int, default=None)
    p.add_argument("--bin", default=os.path.join(REPO, "target", "debug", "herdr-threads"))
    p.add_argument("--pane-id", default=None, help="fake pane id for the claude seat (default: generated)")
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
        self.tmux_sockets = []
        self.popens = []
        self.state_dirs = []
        self.bin = None

    def run(self, argv, **kw):
        kw.setdefault("capture_output", True)
        kw.setdefault("text", True)
        kw.setdefault("timeout", 120)
        return subprocess.run(argv, **kw)

    def stop(self):
        for sock in self.tmux_sockets:
            subprocess.run(["tmux", "-L", sock, "kill-server"], capture_output=True)
        for p in self.popens:
            if p.poll() is None:
                p.terminate()
                try:
                    p.wait(5)
                except subprocess.TimeoutExpired:
                    p.kill()
        for sd in self.state_dirs:
            if self.bin:
                subprocess.run([self.bin, "daemon", "stop", "--state-dir", sd], capture_output=True, timeout=30)
        self.tmux_sockets, self.popens, self.state_dirs = [], [], []


# ---------------------------------------------------------------- live driver

class Live:
    def __init__(self, args, root, procs):
        self.args, self.root, self.procs = args, root, procs
        self.rng = random.Random(args.seed)
        self.sock = "ht-mod-" + uuid.uuid4().hex[:10]
        self.state = os.path.join(root, "state")
        self.home = os.path.join(root, "home")
        self.cfg = os.path.join(root, "profile")
        self.work = os.path.join(root, "work")
        self.pane = args.pane_id or "p_" + uuid.uuid4().hex[:8]
        self.sender_pane = "p_" + uuid.uuid4().hex[:8]
        self.sent = []           # message ids
        self.truncated = set()
        self.resets = []         # ms timestamps of /clear
        self.holds = []          # (start, end) ms of post-abort holds
        self.failures = []
        self.sessions = {}       # name -> ledger path
        for d in (self.state, self.home, self.work):
            os.makedirs(d, exist_ok=True)

    # env shared by every child: isolated HOME/XDG/state, no autoupdater
    def env(self, pane, ledger=None):
        e = {k: v for k, v in os.environ.items() if not k.startswith(("HERDR_", "CLAUDE"))}
        e.update(HOME=self.home, XDG_CONFIG_HOME=self.home + "/.config", XDG_DATA_HOME=self.home + "/.local/share",
                 XDG_STATE_HOME=self.home + "/.local/state", XDG_CACHE_HOME=self.home + "/.cache",
                 CLAUDE_CONFIG_DIR=self.cfg, DISABLE_AUTOUPDATER="1", HERDR_PANE_ID=pane,
                 HT_TEST_OWNER=str(os.getpid()))
        if ledger:
            e["HERDR_THREADS_MOD_LEDGER"] = ledger
        return e

    def ht(self, *argv, pane=None, check=True):
        r = self.procs.run([self.args.bin, *argv, "--state-dir", self.state], env=self.env(pane or self.sender_pane))
        if check and r.returncode != 0:
            raise RuntimeError("herdr-threads %s failed: %s" % (" ".join(argv), (r.stderr or r.stdout)[:400]))
        return r

    def tmux(self, *argv):
        return self.procs.run(["tmux", "-L", self.sock, *argv]).stdout

    def setup(self):
        shutil.copytree(self.args.profile, self.cfg)
        self.procs.state_dirs.append(self.state)
        self.procs.bin = self.args.bin
        self.procs.tmux_sockets.append(self.sock)
        self.ht("setup", "claude", pane=self.pane)  # installs hooks and the mod into the copied profile
        self.ht("daemon", "ensure", pane=self.pane)

    def start_claude(self, name, extra=()):
        led = os.path.join(self.root, "ledger-%s.jsonl" % name)
        self.sessions[name] = led
        cmd = "exec claude --permission-mode default " + " ".join(extra)
        env = self.env(self.pane, ledger=led)
        envs = " ".join("%s=%s" % (k, shell_quote(env[k])) for k in sorted(env)
                        if k.startswith(("HOME", "XDG_", "CLAUDE_", "DISABLE_", "HERDR_", "HT_")))
        self.tmux("new-session", "-d", "-s", name, "-x", "200", "-y", "50", "-c", self.work, "env %s %s" % (envs, cmd))
        self.wait_pane(name, r"(\?|>|❯)", 60)
        return led

    def pane_text(self, name):
        return self.tmux("capture-pane", "-p", "-t", name)

    def wait_pane(self, name, pattern, timeout):
        end = time.time() + timeout
        while time.time() < end:
            if re.search(pattern, self.pane_text(name)):
                return True
            time.sleep(0.5)
        return False

    def type_prompt(self, name, text):
        self.tmux("send-keys", "-t", name, "-l", text)
        self.tmux("send-keys", "-t", name, "Enter")

    def send(self, thread, body, lazy=False, big=False):
        argv = ["send", thread, "--body", body, "--json"]
        if lazy:
            argv.append("--lazy")
        else:
            argv += ["--require-ack-pane", self.pane]
        out = self.ht(*argv).stdout
        mid = json.loads(out).get("message_id") or json.loads(out).get("id")
        self.sent.append(mid)
        if big:
            self.truncated.add(mid)
        return mid

    def ledger(self, name):
        try:
            with open(self.sessions[name]) as f:
                return parse_ledger(f.read())
        except FileNotFoundError:
            return []

    def receipts(self):
        db = os.path.join(self.state, "herdr-threads.sqlite3")
        if not os.path.exists(db):
            cands = [os.path.join(d, f) for d, _, fs in os.walk(self.state) for f in fs if f.endswith((".sqlite3", ".db"))]
            db = cands[0] if cands else db
        con = sqlite3.connect("file:%s?mode=ro" % db, uri=True)
        try:
            return {m: s for m, s in con.execute("SELECT message_id, state FROM receipts")}
        finally:
            con.close()

    def wait_settled(self, ids, timeout=90):
        end = time.time() + timeout
        while time.time() < end:
            rc = self.receipts()
            if all(rc.get(i) == "acked" for i in ids):
                return True
            time.sleep(1)
        return False

    def capture(self, tag):
        for name in self.sessions:
            with open(os.path.join(self.root, "pane-%s-%s.txt" % (name, tag)), "w") as f:
                f.write(self.pane_text(name))


def shell_quote(s):
    return "'" + str(s).replace("'", "'\\''") + "'"


def now_ms():
    return int(time.time() * 1000)


# Scenario functions: each performs one iteration against the session `s` and thread `th`.
# They send, perturb the TUI and return; settlement is asserted by the caller from receipts+ledger.

def sc_busy_context(L, s, th, i):
    L.type_prompt(s, "Run `sleep 8` with the Bash tool, then reply done.")
    L.wait_pane(s, r"sleep 8|Bash", 30)
    L.send(th, "busy-%d" % i)


def sc_idle_submit(L, s, th, i):
    L.wait_pane(s, r"(>|❯)", 60)
    L.send(th, "idle-%d" % i)


def sc_lazy_append(L, s, th, i):
    L.send(th, "lazy-%d" % i, lazy=True)


def sc_queued_user_prompt(L, s, th, i):
    L.type_prompt(s, "Run `sleep 6` with Bash, then reply done.")
    L.wait_pane(s, r"sleep 6|Bash", 30)
    L.type_prompt(s, "queued user prompt %d" % i)
    L.send(th, "queued-%d" % i)


def sc_stop_hook_continuation(L, s, th, i):
    open(os.path.join(L.work, "stop-armed"), "w").close()
    L.type_prompt(s, "Say ALPHA.")
    L.send(th, "stop-%d" % i)


def sc_esc_interrupt_hold(L, s, th, i):
    L.type_prompt(s, "Run `sleep 20` with Bash.")
    L.wait_pane(s, r"sleep 20|Bash", 30)
    start = now_ms()
    L.tmux("send-keys", "-t", s, "Escape")
    L.send(th, "esc-%d" % i)
    time.sleep(3)
    # the hold lasts until a later non-aborted turn completes; end it with a plain user turn
    L.type_prompt(s, "Say OK.")
    L.wait_pane(s, r"OK", 60)
    L.holds.append((start, now_ms()))


def sc_permission_dialog(L, s, th, i):
    L.type_prompt(s, "Create a file named perm-%d.txt containing x using the Write tool." % i)
    L.wait_pane(s, r"(Do you want|\(y/n\)|Yes)", 60)
    L.send(th, "perm-%d" % i)
    L.tmux("send-keys", "-t", s, "Escape")


def sc_clear_rebind(L, s, th, i):
    L.send(th, "prelear-%d" % i)
    L.wait_pane(s, r"(>|❯)", 60)
    L.resets.append(now_ms())
    L.type_prompt(s, "/clear")
    time.sleep(2)
    L.send(th, "postclear-%d" % i)


def sc_resume_keeps_set(L, s, th, i):
    L.send(th, "preresume-%d" % i)
    L.wait_pane(s, r"(>|❯)", 60)
    L.tmux("send-keys", "-t", s, "C-c")
    L.tmux("send-keys", "-t", s, "C-c")
    time.sleep(2)
    L.start_claude(s + "r", ("--continue",))
    L.send(th, "postresume-%d" % i)


def sc_reload(L, s, th, i):
    L.send(th, "prereload-%d" % i)
    L.type_prompt(s, "/reload-plugins")
    time.sleep(3)
    L.send(th, "postreload-%d" % i)


def sc_reload_mid_turn(L, s, th, i):
    L.type_prompt(s, "Run `sleep 10` with Bash.")
    L.wait_pane(s, r"sleep 10|Bash", 30)
    L.send(th, "midturn-%d" % i)
    L.type_prompt(s, "/reload-plugins")


def sc_kill_watch(L, s, th, i):
    L.send(th, "prekill-%d" % i)
    out = L.procs.run(["pgrep", "-f", "watch --harness claude --session"]).stdout.split()
    for pid in out:  # only children whose environment carries this run's state dir
        cmd = L.procs.run(["ps", "eww", "-p", pid, "-o", "command="]).stdout
        if L.state in cmd:
            os.kill(int(pid), signal.SIGKILL)
    time.sleep(2)
    L.send(th, "postkill-%d" % i)


def sc_ups_block_drop(L, s, th, i):
    open(os.path.join(L.work, "block-ups"), "w").close()  # the profile's UserPromptSubmit hook blocks while present
    L.send(th, "upsblock-%d" % i)
    time.sleep(5)
    os.remove(os.path.join(L.work, "block-ups"))


def sc_denied_tool(L, s, th, i):
    L.type_prompt(s, "Run `rm -rf /nonexistent-%d` with Bash." % i)
    L.wait_pane(s, r"(Do you want|\(y/n\))", 60)
    L.send(th, "denied-%d" % i)
    L.tmux("send-keys", "-t", s, "Escape")


SCENARIOS = {
    "busy_context": sc_busy_context, "idle_submit": sc_idle_submit, "lazy_append": sc_lazy_append,
    "queued_user_prompt": sc_queued_user_prompt, "stop_hook_continuation": sc_stop_hook_continuation,
    "esc_interrupt_hold": sc_esc_interrupt_hold, "permission_dialog": sc_permission_dialog,
    "clear_rebind": sc_clear_rebind, "resume_keeps_set": sc_resume_keeps_set, "reload": sc_reload,
    "reload_mid_turn": sc_reload_mid_turn, "kill_watch_fallback_and_restream": sc_kill_watch,
    "user_prompt_submit_block_drop": sc_ups_block_drop, "denied_tool_with_pending_context": sc_denied_tool,
}
assert sorted(SCENARIOS) == sorted(SCENARIO_NAMES)


def run_live(args, root, procs):
    L = Live(args, root, procs)
    L.setup()
    summary = {"mode": "live", "iterations": args.iterations, "seed": args.seed, "scenarios": {}}
    th = L.ht("thread", "create", "--json").stdout  # thread bootstrap; id parsed below
    th = json.loads(th).get("thread_id") or json.loads(th).get("id")
    L.ht("invite", th, "--seat", "self", check=False)
    for name in args.scenarios:
        s = "s-" + name.replace("_", "-")[:20]
        L.start_claude(s)
        passes, failures = 0, []
        for i in range(args.iterations):
            before = len(L.sent)
            SCENARIOS[name](L, s, th, i)
            ids = L.sent[before:]
            L.wait_settled(ids, 90)
            L.capture("%s-%d" % (name, i))
            led = [e for n in L.sessions for e in L.ledger(n)]
            v = check_ledger(led, L.resets, L.truncated, L.holds) + check_settled(L.sent, L.receipts(), led)
            if v:
                failures.append({"iteration": i, "violations": v})
            else:
                passes += 1
        summary["scenarios"][name] = {"pass": passes, "fail": len(failures), "failures": failures}
        L.tmux("kill-session", "-t", s)
    summary["sent"] = len(L.sent)
    for n, path in L.sessions.items():
        shutil.copy(path, os.path.join(args.evidence_dir, "ledger-%s.jsonl" % n)) if os.path.exists(path) else None
    with open(os.path.join(args.evidence_dir, "receipts.json"), "w") as f:
        json.dump(L.receipts(), f, indent=1, sort_keys=True)
    return summary


def preflight(args, root, procs):
    """Returns (usable, reason, claude_version)."""
    if shutil.which("claude") is None:
        return False, "claude CLI not found on PATH", None
    out = procs.run(["claude", "--version"]).stdout
    m = re.match(r"([0-9][0-9.]*)", out.strip())
    version = m.group(1) if m else None
    if not version or not version_at_least(version, MIN_CLAUDE):
        return False, "claude %s older than %s" % (version, MIN_CLAUDE), version
    if shutil.which("tmux") is None:
        return False, "tmux not found", version
    if not os.path.isdir(args.profile):
        return False, "profile %s missing" % args.profile, version
    cfg = os.path.join(root, "preflight-profile")
    try:
        shutil.copytree(args.profile, cfg)
    except OSError as e:
        return False, "profile copy failed: %s" % e, version
    env = {k: v for k, v in os.environ.items() if not k.startswith(("HERDR_", "CLAUDE"))}
    env.update(HOME=os.path.join(root, "home"), CLAUDE_CONFIG_DIR=cfg, DISABLE_AUTOUPDATER="1")
    os.makedirs(env["HOME"], exist_ok=True)
    try:
        r = procs.run(["claude", "-p", "Reply with the single word ok."], env=env, timeout=120)
    except subprocess.TimeoutExpired:
        return False, "signed-in probe timed out", version
    text = (r.stdout or "") + (r.stderr or "")
    if r.returncode != 0 or not auth_output_usable(r.stdout or ""):
        return False, "copied profile is not signed in (claude -p: %s)" % text.strip()[:120], version
    return True, "ok", version


def run_fallback(reason):
    r = subprocess.run(["sh", os.path.join(REPO, "scripts", "test-claude-mod")], capture_output=True, text=True, timeout=900)
    tail = [l for l in r.stdout.splitlines() if re.search(r"\d+ (pass|fail)|Ran \d+|skipped|Validation", l)]
    return {"mode": "fallback", "live_gap": reason, "unit_exit": r.returncode, "unit_summary": tail,
            "unit_output": r.stdout}


def main(argv=None):
    args = parse_args(sys.argv[1:] if argv is None else argv)
    procs = Procs()
    root = tempfile.mkdtemp(prefix="ht-modstress.")
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
            usable, reason, version = preflight(args, root, procs)
        summary = run_live(args, root, procs) if usable else run_fallback(reason)
        summary.update(sha=sha, claude_version=version, finished=time.strftime("%Y-%m-%dT%H:%M:%S%z"))
        with open(os.path.join(args.evidence_dir, "summary.json"), "w") as f:
            json.dump(summary, f, indent=1, sort_keys=True)
        print(json.dumps({k: v for k, v in summary.items() if k != "unit_output"}, indent=1, sort_keys=True))
        failed = (summary["mode"] == "live" and any(s["fail"] for s in summary["scenarios"].values())) or \
                 (summary["mode"] == "fallback" and summary["unit_exit"] != 0)
        return 1 if failed else 0
    finally:
        procs.stop()
        shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
