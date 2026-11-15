#!/usr/bin/env python3
"""Isolated, authenticated embedded-TUI queue experiment; no production integration.

Uses only a private credential copy, private HOME/CODEX_HOME and owned process
groups. Saves sanitized observations, never auth files or complete runtime logs.
"""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import pty
import re
import select
import shlex
import shutil
import signal
import sqlite3
import struct
import subprocess
import sys
import tempfile
import termios
import time
import uuid


def hook():
    root = Path(os.environ["SPIKE_ROOT"])
    event = json.load(sys.stdin)
    name = event["hook_event_name"]
    row = {k: event[k] for k in ("session_id", "turn_id", "prompt", "tool_name", "stop_hook_active") if k in event}
    row.update(event=name, at=time.time())
    pending = root / "pending.json"
    tokens = json.loads(pending.read_text()) if pending.exists() else []
    if tokens and name in ("UserPromptSubmit", "PreToolUse", "PostToolUse"):
        row["injected"] = tokens
        pending.unlink(missing_ok=True)
    with (root / "hooks.jsonl").open("a") as f:
        f.write(json.dumps(row) + "\n")
    if row.get("injected"):
        print(json.dumps({"hookSpecificOutput": {"hookEventName": name, "additionalContext": "Controlled spike mail tokens: " + ", ".join(tokens) + ". Include each token verbatim in your final reply."}}))
    else:
        print("{}")


class Spike:
    def __init__(self, args):
        self.args = args
        self.root = Path(tempfile.mkdtemp(prefix="ht-codex-queue-spike-"))
        self.root.chmod(0o700)
        self.home = self.root / "home"
        self.chome = self.root / "codex"
        self.cwd = self.root / "work"
        for d in (self.home, self.chome, self.cwd):
            d.mkdir(mode=0o700)
        self.env = {k: v for k, v in os.environ.items() if not k.startswith(("HERDR_", "HC_", "CODEX_", "CLAUDE_", "TMUX", "HT_"))}
        self.env.update(HOME=str(self.home), CODEX_HOME=str(self.chome), CODEX_EXEC_SERVER_URL="", SPIKE_ROOT=str(self.root), TERM="xterm-256color", COLORTERM="truecolor", HT_TEST_OWNER=str(os.getpid()), HT_LEAK_RUN_ID=str(uuid.uuid4()), XDG_CONFIG_HOME=str(self.root / "cfg"), XDG_STATE_HOME=str(self.root / "state"), XDG_CACHE_HOME=str(self.root / "cache"), XDG_RUNTIME_DIR=str(self.root / "runtime"))
        Path(self.env["XDG_RUNTIME_DIR"]).mkdir(mode=0o700)
        self.results = []
        self.children = []
        self.raw = bytearray()
        self.session = None
        self.master = None
        self.source_digest = hashlib.sha256(args.auth_file.read_bytes()).hexdigest()
        fd = os.open(self.chome / "auth.json", os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "wb") as f:
            f.write(args.auth_file.read_bytes())
        self.copy_digest = hashlib.sha256((self.chome / "auth.json").read_bytes()).hexdigest()
        self.instructions = "You are an isolated transport test recipient. Do not access any account files or network, create subagents, or run herdr-threads. Reply briefly. For a queued 'herdr-threads: wake up', include any Controlled spike mail tokens in your reply. When no tokens exist, reply QUEUE_EMPTY. If instructed BUSY, execute exactly the supplied Python barrier command once, wait for it to finish, then reply BUSY_DONE plus any mail tokens."
        (self.cwd / "AGENTS.md").write_text(self.instructions + "\n")
        config = 'model = "gpt-6.1-sol"\nmodel_reasoning_effort = "low"\ndisable_update_check = true\n'
        config += '[projects.' + json.dumps(str(self.cwd)) + ']\ntrust_level = "trusted"\n'
        (self.chome / "config.toml").write_text(config)
        command = shlex.join([sys.executable, str(Path(__file__).resolve()), "--hook"])
        hooks = {name: [{"matcher": "*", "hooks": [{"type": "command", "command": command, "timeout": 10}]}] for name in ("SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse", "Stop", "Interrupt")}
        (self.chome / "hooks.json").write_text(json.dumps({"hooks": hooks}))

    def log(self, message):
        print(message, flush=True)

    def spawn(self, argv, **kwargs):
        proc = subprocess.Popen(argv, env=self.env, cwd=self.cwd, start_new_session=True, **kwargs)
        self.children.append(proc)
        return proc

    def pump(self, duration=0.2):
        until = time.monotonic() + duration
        while time.monotonic() < until:
            if self.master is None:
                time.sleep(0.05)
                continue
            ready, _, _ = select.select([self.master], [], [], min(0.1, max(0, until - time.monotonic())))
            if ready:
                try:
                    chunk = os.read(self.master, 65536)
                except OSError:
                    return
                self.raw.extend(chunk)
                if b"\x1b[6n" in chunk:
                    os.write(self.master, b"\x1b[1;1R")
                if b"\x1b[?u" in chunk:
                    os.write(self.master, b"\x1b[?0u")
                if b"\x1b[c" in chunk:
                    os.write(self.master, b"\x1b[?1;2c")

    def events(self):
        f = self.root / "hooks.jsonl"
        if not f.exists():
            return []
        rows = []
        for line in f.read_text().splitlines():
            try:
                rows.append(json.loads(line))
            except json.JSONDecodeError:
                pass
        return rows

    def history(self):
        rows = []
        for f in (self.chome / "sessions").rglob("*.jsonl") if (self.chome / "sessions").exists() else []:
            for line in f.read_text(errors="replace").splitlines():
                try:
                    rows.append(json.loads(line))
                except json.JSONDecodeError:
                    pass
        return rows

    def wait(self, predicate, timeout=90, label="condition"):
        start = time.monotonic()
        next_update = start + 25
        while time.monotonic() - start < timeout:
            self.pump()
            if predicate():
                return time.monotonic() - start
            if time.monotonic() >= next_update:
                self.log("waiting: " + label)
                next_update += 25
            if self.tui.poll() is not None:
                raise RuntimeError("owned TUI exited while waiting for " + label)
        raise TimeoutError(label)

    def start(self):
        self.master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 45, 160, 0, 0))
        argv = ["codex", "--no-daemon", "--no-alt-screen", "--dangerously-bypass-hook-trust", "-s", "workspace-write", "-a", "never", "Reply exactly SPIKE_READY without using tools."]
        self.tui = self.spawn(argv, stdin=slave, stdout=slave, stderr=slave)
        os.close(slave)
        self.wait(lambda: any(r.get("event") == "SessionStart" for r in self.events()), label="SessionStart hook")
        self.session = next(r["session_id"] for r in self.events() if r.get("event") == "SessionStart")
        self.wait(lambda: any(r.get("event") == "Stop" for r in self.events()), label="initial response Stop")
        self.log("foreground TUI ready; native session captured")

    def queue(self, text, alternate_home=False):
        argv = ["codex", "queue", "--thread", self.session, "--message", text]
        env = self.env.copy()
        if alternate_home:
            home = self.root / "other-codex"
            home.mkdir(exist_ok=True)
            env["CODEX_HOME"] = str(home)
        proc = subprocess.Popen(argv, env=env, cwd=self.cwd, start_new_session=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        self.children.append(proc)
        start = time.monotonic()
        while proc.poll() is None and time.monotonic() - start < 35:
            self.pump()
        if proc.poll() is None:
            os.killpg(proc.pid, signal.SIGTERM)
        out, err = proc.communicate(timeout=5)
        return {"returncode": proc.returncode, "stdout": self.redact(out.decode(errors="replace")), "stderr": self.redact(err.decode(errors="replace")), "seconds": round(time.monotonic() - start, 3), "pending_rows_after_return": self.queue_rows()}

    def queue_rows(self):
        f = self.chome / "queue_1.sqlite"
        if not f.exists():
            return []
        try:
            db = sqlite3.connect(f"file:{f}?mode=ro", uri=True)
            rows = [{"id": r[0], "thread": r[1]} for r in db.execute("select id,thread_id from queued_items")]
            db.close()
            return rows
        except sqlite3.Error:
            return []

    def submit(self, text):
        os.write(self.master, b"\x1b[200~" + text.encode() + b"\x1b[201~\r")

    def model_said(self, token):
        return any(token in json.dumps(r) and r.get("type") == "response_item" and r.get("payload", {}).get("role") == "assistant" for r in self.history())

    def redact(self, text):
        return text.replace(str(self.root), "$SPIKE_ROOT").replace(str(self.args.auth_file), "$SOURCE_AUTH")

    def counts(self):
        events = self.events()
        return {n: sum(r.get("event") == n for r in events) for n in ("UserPromptSubmit", "Stop", "Interrupt")}

    def record(self, name, passed, **evidence):
        self.results.append(dict(case=name, passed=passed, **evidence))
        self.log(("PASS " if passed else "FAIL ") + name)

    def idle_case(self, name, draft=False):
        token = "MAIL_" + uuid.uuid4().hex[:12]
        (self.root / "pending.json").write_text(json.dumps([token]))
        before = self.counts()
        if draft:
            os.write(self.master, b"UNSUBMITTED_DRAFT_6942")
            self.pump(1)
        start = time.monotonic()
        result = self.queue("herdr-threads: wake up")
        if result["returncode"]:
            self.record(name, False, queue=result, counts_before=before, counts_after=self.counts())
            return False
        try:
            self.wait(lambda: self.counts()["Stop"] > before["Stop"], label=name + " Stop")
        except TimeoutError:
            self.record(name, False, queue=result, counts_before=before, counts_after=self.counts())
            return False
        hooks = self.events()
        injection = [r for r in hooks if token in r.get("injected", [])]
        model_saw_token = self.model_said(token)
        # Paginated stores may not expose rollout items: report hook observation separately.
        if draft:
            self.submit("")
            self.wait(lambda: self.counts()["Stop"] > before["Stop"] + 1, label="preserved draft submission")
        draft_seen = any(r.get("prompt") == "UNSUBMITTED_DRAFT_6942" for r in self.events()) if draft else None
        self.record(name, bool(injection) and model_saw_token and (not draft or draft_seen), queue=result, seconds_to_stop=round(time.monotonic() - start, 3), counts_before=before, counts_after=self.counts(), injection_events=injection, model_token_in_legacy_rollout=model_saw_token, draft_survived_and_submitted=draft_seen, pending_rows_after_delivery=self.queue_rows())
        if draft:
            # Clear only our owned test draft after observing the run.
            os.write(self.master, b"\x15")
        return bool(injection) and model_saw_token

    def duplicate_case(self):
        before = self.counts()
        a = self.queue("herdr-threads: wake up")
        b = self.queue("herdr-threads: wake up")
        try:
            self.wait(lambda: self.counts()["Stop"] >= before["Stop"] + 2, label="two duplicate marker turns")
            two = True
        except TimeoutError:
            two = False
        self.record("duplicate_submissions_are_not_deduplicated", two, queue_a=a, queue_b=b, counts_before=before, counts_after=self.counts())

    def busy_case(self, interrupt=False):
        for name in ("entered", "release"):
            (self.cwd / name).unlink(missing_ok=True)
        (self.cwd / "barrier.py").write_text("import pathlib,time\np=pathlib.Path('.')\n(p/'entered').write_text('ready')\nstart=time.monotonic()\nwhile not (p/'release').exists() and time.monotonic()-start<70: time.sleep(.1)\nprint('BARRIER_RELEASED')\n")
        before = self.counts()
        self.submit("BUSY: execute exactly `python3 barrier.py` once with exec_command, yield_time_ms=1000, then wait for completion with write_stdin as needed. Do not run any other command. Reply BUSY_DONE after completion.")
        self.wait(lambda: (self.cwd / "entered").exists(), label="busy tool barrier")
        token = "BUSY_MAIL_" + uuid.uuid4().hex[:10]
        (self.root / "pending.json").write_text(json.dumps([token]))
        queued = self.queue("herdr-threads: wake up")
        self.pump(12)
        no_early_turn = self.counts()["UserPromptSubmit"] == before["UserPromptSubmit"] + 1
        if interrupt:
            os.write(self.master, b"\x1b")
            self.wait(lambda: self.counts()["Interrupt"] > before["Interrupt"], label="interrupt hook")
        (self.cwd / "release").write_text("go")
        if interrupt:
            at_interrupt = self.counts()
            self.pump(22)
            held = self.counts()["UserPromptSubmit"] == at_interrupt["UserPromptSubmit"] and bool(self.queue_rows())
            self.record("interrupted_queue_is_held", no_early_turn and held, queue=queued, counts_before=before, counts_after=self.counts(), pending_rows=self.queue_rows())
            # Human-style explicit continuation of our own isolated test session.
            self.submit("Reply exactly RESUMED_BY_DRIVER without tools.")
            self.wait(lambda: not self.queue_rows() and self.counts()["Stop"] >= at_interrupt["Stop"] + 2, label="explicit continuation drains held queue")
            self.record("explicit_turn_releases_interrupted_queue", not self.queue_rows(), counts_after=self.counts())
        else:
            self.wait(lambda: self.counts()["Stop"] >= before["Stop"] + 2, label="busy turn plus deferred wake")
            rows = self.events()
            injected = next((r for r in rows if token in r.get("injected", [])), None)
            new_prompts = [r for r in rows if r.get("event") == "UserPromptSubmit"][before["UserPromptSubmit"]:]
            busy_turn = new_prompts[0].get("turn_id") if new_prompts else None
            during_busy = bool(injected and injected.get("turn_id") == busy_turn)
            self.record("busy_deferral_and_stale_marker", no_early_turn and during_busy and self.model_said(token), queue=queued, no_second_turn_while_tool_running=no_early_turn, delivered_in_busy_turn=during_busy, injection=injected, counts_before=before, counts_after=self.counts(), deferred_marker_started_extra_turn=len(new_prompts) >= 2)

    def mismatch_case(self):
        before = self.counts()
        result = self.queue("herdr-threads: wake up", alternate_home=True)
        self.pump(12)
        self.record("wrong_storage_does_not_reach_target", result["returncode"] != 0 and self.counts() == before, queue=result, counts_before=before, counts_after=self.counts())

    def modal_case(self):
        before = self.counts()
        self.submit("/model")
        self.pump(2)
        queued = self.queue("herdr-threads: wake up")
        self.pump(20)
        # Observe, then close only the test-owned picker. No model selection.
        changed = self.counts()["UserPromptSubmit"] > before["UserPromptSubmit"]
        after_hold = self.counts()
        os.write(self.master, b"\x1b")
        if not changed:
            try:
                self.wait(lambda: self.counts()["Stop"] > before["Stop"], timeout=35, label="queue after closing model picker")
            except TimeoutError:
                pass
        self.record("modal_picker_queue_behavior", True, observational=True, turn_started_while_picker_open=changed, queue=queued, counts_before=before, counts_while_open=after_hold, counts_after_close=self.counts())

    def snapshot_db(self):
        result = []
        for f in self.chome.glob("*.sqlite"):
            try:
                db = sqlite3.connect(f"file:{f}?mode=ro", uri=True)
                tables = [r[0] for r in db.execute("select name from sqlite_master where type='table'")]
                interesting = [t for t in tables if any(s in t.lower() for s in ("queue", "message", "thread"))]
                for t in interesting:
                    columns = [r[1] for r in db.execute('pragma table_info("' + t + '")')]
                    result.append({"database": f.name, "table": t, "columns": columns, "rows": db.execute('select count(*) from "' + t + '"').fetchone()[0]})
                db.close()
            except sqlite3.Error as exc:
                result.append({"database": f.name, "error": str(exc)})
        return result

    def cleanup(self):
        # Stop only process groups created by this driver; then tagged detached descendants.
        for proc in reversed(self.children):
            try:
                os.killpg(proc.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
        for proc in self.children:
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(proc.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                proc.wait(timeout=5)
        tag = ("HT_LEAK_RUN_ID=" + self.env["HT_LEAK_RUN_ID"]).encode()
        owned = []
        for f in Path("/proc").glob("[0-9]*/environ"):
            try:
                if tag in f.read_bytes().split(b"\0"):
                    owned.append(int(f.parent.name))
            except (OSError, PermissionError):
                pass
        for pid in owned:
            try:
                os.kill(pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
        if self.master is not None:
            os.close(self.master)
        self.cleanup_pids = owned

    def save(self, error=None):
        self.args.output.mkdir(parents=True, exist_ok=True)
        history = [{"type": r["type"], "timestamp": r.get("timestamp"), "payload": r["payload"]} for r in self.history() if r.get("type") == "response_item" and r.get("payload", {}).get("role") in ("user", "assistant")]
        data = {"version": subprocess.check_output(["codex", "--version"], env=self.env, stderr=subprocess.DEVNULL, text=True).strip(), "cases": self.results, "error": self.redact(str(error)) if error else None, "session": self.session, "hooks": self.events(), "message_history": history, "database_schema": self.snapshot_db(), "source_auth_unchanged": hashlib.sha256(self.args.auth_file.read_bytes()).hexdigest() == self.source_digest, "private_auth_refreshed": hashlib.sha256((self.chome / "auth.json").read_bytes()).hexdigest() != self.copy_digest, "cleanup_tagged_descendants": self.cleanup_pids}
        (self.args.output / "results.json").write_text(self.redact(json.dumps(data, indent=2)) + "\n")
        # Terminal output may contain diagnostics but never read credential contents.
        text = self.redact(self.raw.decode(errors="replace"))
        text = re.sub(r"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)", "", text)
        text = re.sub(r"\x1b\[[0-?]*[ -/]*[@-~]", "", text)
        text = re.sub(r"\x1b[@-_]", "", text)
        text = "\n".join(line.rstrip() for line in text.splitlines()).strip() + "\n"
        (self.args.output / "terminal.txt").write_text(text)
        shutil.rmtree(self.root)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--auth-file", type=Path)
    ap.add_argument("--output", type=Path)
    ap.add_argument("--hook", action="store_true")
    ap.add_argument("--matrix", action="store_true")
    args = ap.parse_args()
    if args.hook:
        hook()
        return 0
    if not args.auth_file or not args.output:
        ap.error("--auth-file and --output are required")
    run = Spike(args)
    error = None
    try:
        run.start()
        if run.idle_case("idle_marker_hook_delivery") and args.matrix:
            run.idle_case("draft_is_preserved", draft=True)
            run.duplicate_case()
            run.busy_case()
            run.busy_case(interrupt=True)
            run.mismatch_case()
            run.modal_case()
    except Exception as exc:
        error = exc
        run.log(type(exc).__name__ + ": " + str(exc))
    finally:
        run.cleanup()
        run.save(error)
    return int(error is not None or any(not r["passed"] for r in run.results))


if __name__ == "__main__":
    sys.exit(main())
