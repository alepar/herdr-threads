#!/usr/bin/env python3
"""Probe queue RPC against shared scratch storage, without attaching to the TUI."""
import argparse
import json
import os
from pathlib import Path
import select
import subprocess
import time
import uuid

from spike import Spike


class QueueRPC:
    def __init__(self, run):
        self.run = run
        self.proc = run.spawn(["codex", "app-server", "--listen", "stdio://"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        self.buffer = b""
        self.seq = 0
        self.notifications = []
        self.calls = []
        self.call("initialize", {"clientInfo": {"name": "herdr_threads_queue_spike", "version": "0.1.0"}, "capabilities": {"experimentalApi": True}})
        self.send({"method": "initialized"})

    def send(self, data):
        self.proc.stdin.write((json.dumps(data) + "\n").encode())
        self.proc.stdin.flush()

    def call(self, method, params):
        self.seq += 1
        request = {"method": method, "id": self.seq, "params": params}
        self.send(request)
        start = time.monotonic()
        while time.monotonic() - start < 20:
            while b"\n" in self.buffer:
                line, self.buffer = self.buffer.split(b"\n", 1)
                row = json.loads(line)
                if row.get("id") == self.seq:
                    self.calls.append({"method": method, "response": row})
                    if "error" in row:
                        raise RuntimeError(json.dumps(row["error"]))
                    return row["result"]
                if "method" in row:
                    self.notifications.append(row)
            ready, _, _ = select.select([self.proc.stdout], [], [], 0.1)
            if ready:
                chunk = os.read(self.proc.stdout.fileno(), 65536)
                if not chunk:
                    raise RuntimeError("owned queue RPC server exited")
                self.buffer += chunk
            self.run.pump(0.05)
        raise TimeoutError(method)

    def add(self, client_id):
        return self.call("thread/queue/add", {"threadId": self.run.session, "clientUserMessageId": client_id, "input": [{"type": "text", "text": "herdr-threads: wake up"}]})

    def items(self):
        return self.call("thread/queue/list", {"threadId": self.run.session})

    def delete(self, item_id):
        return self.call("thread/queue/delete", {"threadId": self.run.session, "queuedSubmissionId": item_id})


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--auth-file", type=Path, required=True)
    ap.add_argument("--output", type=Path, required=True)
    ap.add_argument("--reconcile-duplicates", action="store_true", help="Observe distinct rows for repeated client IDs, then delete every exact queued submission")
    args = ap.parse_args()
    run = Spike(args)
    rpc = None
    error = None
    try:
        run.start()
        rpc = QueueRPC(run)
        # Busy barrier keeps submissions pending while retry and cancellation are observed.
        (run.cwd / "barrier.py").write_text("import pathlib,time\np=pathlib.Path('.')\n(p/'entered').write_text('ready')\ns=time.monotonic()\nwhile not (p/'release').exists() and time.monotonic()-s<90: time.sleep(.1)\nprint('RELEASED')\n")
        before = run.counts()
        run.submit("BUSY: execute exactly `python3 barrier.py` once with exec_command, yield_time_ms=1000, then wait for its completion with write_stdin. Reply BUSY_DONE only after it completes.")
        run.wait(lambda: (run.cwd / "entered").exists(), label="RPC busy barrier")
        client_id = str(uuid.uuid4())
        a = rpc.add(client_id)
        b = rpc.add(client_id)
        items = rpc.items()
        if args.reconcile_duplicates:
            ids = [r["id"] for r in items["data"]]
            run.record("rpc_same_client_id_creates_distinct_rows", len(ids) == 2 and len(set(ids)) == 2, first=a, retry=b, queue=items)
        else:
            run.record("rpc_same_id_retry_pending", len(items["data"]) == 1 and a == b, first=a, retry=b, queue=items)
        queued_id = items["data"][0]["id"]
        deleted = rpc.delete(queued_id)
        deleted_again = rpc.delete(queued_id)
        if args.reconcile_duplicates:
            for row in items["data"][1:]:
                rpc.delete(row["id"])
        after_delete = rpc.items()
        run.record("rpc_delete_pending_marker", deleted.get("deleted") is True and deleted_again.get("deleted") is False and not after_delete["data"], first=deleted, repeat=deleted_again, queue=after_delete)
        (run.cwd / "release").write_text("go")
        run.wait(lambda: run.counts()["Stop"] > before["Stop"], label="release tool after cancellation")
        run.pump(20)
        run.record("cancelled_marker_does_not_wake_after_busy", run.counts()["UserPromptSubmit"] == before["UserPromptSubmit"] + 1, before=before, after=run.counts())
        # A fresh RPC message reaches the separate, still-embedded TUI through its watcher.
        token = "RPC_MAIL_" + uuid.uuid4().hex[:12]
        (run.root / "pending.json").write_text(json.dumps([token]))
        before = run.counts()
        client_id = str(uuid.uuid4())
        added = rpc.add(client_id)
        run.wait(lambda: run.counts()["Stop"] > before["Stop"], label="RPC-created wake reaches embedded TUI")
        listed = rpc.items()
        run.record("rpc_queue_wakes_separate_embedded_tui", not listed["data"] and run.model_said(token), accepted=added, queue_after=listed, hook_and_model_observed=run.model_said(token))
        # Pending-ID dedupe may not be durable once the queue row has been consumed.
        before = run.counts()
        replayed = rpc.add(client_id)
        run.pump(22)
        run.record("rpc_replay_after_consumption", True, observational=True, response=replayed, extra_turn=run.counts()["UserPromptSubmit"] > before["UserPromptSubmit"], before=before, after=run.counts(), queue_after=rpc.items())
        loaded = rpc.call("thread/loaded/list", {})
        run.record("sender_server_did_not_load_target", run.session not in loaded.get("data", []), loaded=loaded)
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
