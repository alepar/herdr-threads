#!/usr/bin/env python3
"""Bounded queue admission/veto experiments, using only isolated runtime state."""
import argparse
import json
import os
from pathlib import Path
import shlex
import sqlite3
import sys
import time
import uuid

from spike import Spike
from rpc_spike import QueueRPC


def pending(root):
    db = root / "codex/queue_1.sqlite"
    if not db.exists():
        return []
    con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    rows = [{"id": r[0], "thread": r[1]} for r in con.execute("select id,thread_id from queued_items")]
    con.close()
    return rows


def write_event(root, row):
    with (root / "hooks.jsonl").open("a") as f:
        f.write(json.dumps(row) + "\n")


def hook():
    root = Path(os.environ["SPIKE_ROOT"])
    event = json.load(sys.stdin)
    row = {k: event[k] for k in ("session_id", "turn_id", "prompt", "tool_name", "stop_hook_active") if k in event}
    row.update(event=event["hook_event_name"], at=time.time(), hook_input_keys=sorted(event), hook_pid=os.getpid(), parent_pid=os.getppid())
    row["caller_env"] = {k: os.environ.get(k) for k in ("CODEX_SESSION_ID", "CODEX_THREAD_ID", "CODEX_HOME", "CODEX_EXEC_SERVER_URL")}
    ancestry = []
    pid = os.getppid()
    for _ in range(5):
        proc = Path("/proc") / str(pid)
        try:
            fields = (proc / "stat").read_text().rsplit(")", 1)[1].split()
            ancestry.append({"pid": pid, "ppid": int(fields[1]), "start_ticks": fields[19], "exe_name": (proc / "comm").read_text().strip()})
            pid = int(fields[1])
        except OSError:
            break
    row["ancestry"] = ancestry
    marker = row["event"] == "UserPromptSubmit" and event.get("prompt") == "herdr-threads: wake up"
    if marker:
        row["queue_at_hook_entry"] = pending(root)
        row["full_hook_input"] = event
        write_event(root, row)
        if (root / "hold-admission").exists():
            (root / "hook-entered").write_text(str(os.getpid()))
            start = time.monotonic()
            while not (root / "hook-release").exists() and time.monotonic() - start < 25:
                time.sleep(0.05)
        mode = (root / "admission-mode").read_text().strip()
        write_event(root, {"event": "AdmissionDecision", "session_id": event["session_id"], "turn_id": event["turn_id"], "at": time.time(), "mode": mode, "queue_at_decision": pending(root)})
        if mode == "block":
            print(json.dumps({"decision": "block", "reason": "SPIKE_OBSOLETE_WAKE"}))
            return 0
        if mode == "error":
            print("SPIKE_ADMISSION_HANDLER_ERROR", file=sys.stderr)
            return 1
    else:
        write_event(root, row)
    print("{}")
    return 0


class AdmissionSpike(Spike):
    def __init__(self, args):
        super().__init__(args)
        (self.root / "admission-mode").write_text("allow")
        command = shlex.join([sys.executable, str(Path(__file__).resolve()), "--hook"])
        names = ("SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse", "Stop", "Interrupt", "PreModelRequest", "PostModelRequest")
        hooks = {n: [{"matcher": "*", "hooks": [{"type": "command", "command": command, "timeout": 35}]}] for n in names}
        (self.chome / "hooks.json").write_text(json.dumps({"hooks": hooks}))
        self.args = args

    def model_requests(self):
        return [r for r in self.events() if r["event"] == "PreModelRequest"]

    def markers(self):
        return [r for r in self.events() if r["event"] == "UserPromptSubmit" and r.get("prompt") == "herdr-threads: wake up"]

    def decision(self, turn):
        return next((r for r in self.events() if r["event"] == "AdmissionDecision" and r.get("turn_id") == turn), None)

    def recover(self):
        (self.root / "admission-mode").write_text("allow")
        before = self.counts()
        self.submit("Reply exactly SPIKE_RECOVERY_OK without tools.")
        self.wait(lambda: self.counts()["Stop"] > before["Stop"], label="ordinary prompt recovery")

    def block_case(self, rpc, held=False):
        before = len(self.markers())
        model_before = len(self.model_requests())
        (self.root / "admission-mode").write_text("allow" if held else "block")
        for name in ("hook-entered", "hook-release", "hold-admission"):
            (self.root / name).unlink(missing_ok=True)
        if held:
            (self.root / "hold-admission").write_text("hold")
        response = rpc.add(str(uuid.uuid4()))
        item = response["queuedSubmission"]
        self.wait(lambda: len(self.markers()) > before, label="queued marker reaches admission")
        marker = self.markers()[-1]
        deleted = None
        if held:
            self.wait(lambda: (self.root / "hook-entered").exists(), label="admission barrier")
            deleted = rpc.delete(item["id"])
            (self.root / "admission-mode").write_text("block")
            (self.root / "hook-release").write_text("release")
        self.wait(lambda: self.decision(marker["turn_id"]) is not None, label="veto decision emitted")
        self.pump(12)
        relevant = [r for r in self.model_requests()[model_before:] if r.get("turn_id") == marker["turn_id"]]
        self.record("veto_after_dequeue" if held else "veto_obsolete_marker_before_inference", not relevant, accepted=response, marker=marker, decision=self.decision(marker["turn_id"]), delete_while_hook_held=deleted, model_requests_for_turn=relevant, queue_after=rpc.items(), model_request_witness_total_before=model_before)
        (self.root / "hold-admission").unlink(missing_ok=True)
        self.recover()

    def fail_open_case(self, rpc):
        before = len(self.markers())
        stops = self.counts()["Stop"]
        (self.root / "admission-mode").write_text("error")
        response = rpc.add(str(uuid.uuid4()))
        self.wait(lambda: len(self.markers()) > before, label="error-mode marker admission")
        marker = self.markers()[-1]
        self.wait(lambda: self.counts()["Stop"] > stops, label="hook failure still completes model turn")
        requests = [r for r in self.model_requests() if r.get("turn_id") == marker["turn_id"]]
        self.record("admission_hook_error_is_fail_open", bool(requests), characterization_only=True, accepted=response, marker=marker, model_requests_for_turn=requests)
        (self.root / "admission-mode").write_text("allow")

    def unknown_add_case(self, rpc):
        for n in ("entered", "release"):
            (self.cwd / n).unlink(missing_ok=True)
        (self.cwd / "barrier.py").write_text("import pathlib,time\np=pathlib.Path('.')\n(p/'entered').write_text('ready')\ns=time.monotonic()\nwhile not (p/'release').exists() and time.monotonic()-s<60: time.sleep(.1)\nprint('RELEASED')\n")
        before = self.counts()
        self.submit("BUSY: execute exactly `python3 barrier.py` once with exec_command, yield_time_ms=1000, wait for completion with write_stdin, then reply BUSY_DONE.")
        self.wait(lambda: (self.cwd / "entered").exists(), label="unknown-outcome busy barrier")
        client = str(uuid.uuid4())
        rpc.seq += 1
        rpc.send({"method": "thread/queue/add", "id": rpc.seq, "params": {"threadId": self.session, "clientUserMessageId": client, "input": [{"type": "text", "text": "herdr-threads: wake up"}]}})
        # Deliberately do not consume the add reply; list recovers its external effect.
        found = []
        for _ in range(20):
            found = [r for r in rpc.items()["data"] if r.get("clientUserMessageId") == client]
            if found:
                break
            self.pump(0.1)
        deleted = [rpc.delete(r["id"]) for r in found]
        (self.cwd / "release").write_text("go")
        self.wait(lambda: self.counts()["Stop"] > before["Stop"], label="busy release after recovered cancellation")
        self.pump(12)
        self.record("lost_add_reply_recovered_while_pending", len(found) == 1 and all(r.get("deleted") for r in deleted) and self.counts()["UserPromptSubmit"] == before["UserPromptSubmit"] + 1, recovered=found, deletes=deleted, counts_before=before, counts_after=self.counts())


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--auth-file", type=Path)
    ap.add_argument("--output", type=Path)
    ap.add_argument("--hook", action="store_true")
    args = ap.parse_args()
    if args.hook:
        return hook()
    if not args.auth_file or not args.output:
        ap.error("--auth-file and --output required")
    run = AdmissionSpike(args)
    rpc = None
    error = None
    try:
        run.start()
        run.record("model_request_hooks_are_live_witnesses", bool(run.model_requests()), observed_initial_requests=len(run.model_requests()))
        rpc = QueueRPC(run)
        run.block_case(rpc)
        run.block_case(rpc, held=True)
        run.fail_open_case(rpc)
        run.unknown_add_case(rpc)
    except Exception as exc:
        error = exc
        run.log(type(exc).__name__ + ": " + str(exc))
    finally:
        if rpc:
            args.output.mkdir(parents=True, exist_ok=True)
            (args.output / "rpc-calls.json").write_text(run.redact(json.dumps({"calls": rpc.calls, "notifications": rpc.notifications}, indent=2)) + "\n")
        run.cleanup()
        run.save(error)
    return int(error is not None or any(not r["passed"] for r in run.results))


if __name__ == "__main__":
    raise SystemExit(main())
