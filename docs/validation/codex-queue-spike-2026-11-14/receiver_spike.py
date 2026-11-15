#!/usr/bin/env python3
"""Test competing embedded receivers with a controlled private queue write lock."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import signal
import sqlite3
import struct
import termios
import time
import uuid

from admission_spike import AdmissionSpike
from rpc_spike import QueueRPC


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--auth-file", type=Path, required=True)
    ap.add_argument("--output", type=Path, required=True)
    args = ap.parse_args()
    run = AdmissionSpike(args)
    error = None
    second_fd = None
    lock = None
    rpc = None
    try:
        run.start()
        primary_fd = run.master
        second_fd, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 45, 160, 0, 0))
        second = run.spawn(["codex", "--no-daemon", "--no-alt-screen", "--dangerously-bypass-hook-trust", "-s", "workspace-write", "-a", "never", "resume", run.session], stdin=slave, stdout=slave, stderr=slave)
        os.close(slave)
        fds = [primary_fd, second_fd]

        def pump_both(duration=0.2):
            until = time.monotonic() + duration
            while time.monotonic() < until:
                ready, _, _ = select.select(fds, [], [], 0.05)
                for fd in ready:
                    try:
                        chunk = os.read(fd, 65536)
                    except OSError:
                        continue
                    run.raw.extend(chunk)
                    for query, reply in [(b"\x1b[6n", b"\x1b[1;1R"), (b"\x1b[?u", b"\x1b[?0u"), (b"\x1b[c", b"\x1b[?1;2c")]:
                        if query in chunk:
                            os.write(fd, reply)
        run.pump = pump_both
        run.wait(lambda: sum(r["event"] == "SessionStart" for r in run.events()) >= 2 or second.poll() is not None, label="second same-session receiver")
        if second.poll() is not None:
            run.record("second_receiver_refused", True, characterization_only=True, returncode=second.returncode)
        else:
            run.record("two_processes_load_same_native_session", True, characterization_only=True, primary_pid=run.tui.pid, second_pid=second.pid, native_session=run.session)
            rpc = QueueRPC(run)
            (run.root / "admission-mode").write_text("block")
            # Both TUI processes are owned by this driver. Pause them before enqueue.
            os.kill(run.tui.pid, signal.SIGSTOP)
            os.kill(second.pid, signal.SIGSTOP)
            accepted = rpc.add(str(uuid.uuid4()))
            lock = sqlite3.connect(run.chome / "queue_1.sqlite")
            lock.execute("BEGIN IMMEDIATE")
            before = len(run.markers())
            os.kill(run.tui.pid, signal.SIGCONT)
            os.kill(second.pid, signal.SIGCONT)
            until = time.monotonic() + 15
            while time.monotonic() < until:
                run.pump()
                markers = run.markers()[before:]
                runtime_pids = {a["pid"] for r in markers for a in r.get("ancestry", []) if a["exe_name"] == "codex"}
                if run.tui.pid in runtime_pids and second.pid in runtime_pids:
                    break
            lock.commit()
            lock.close()
            lock = None
            run.pump(3)
            markers = run.markers()[before:]
            runtime_pids = {a["pid"] for r in markers for a in r.get("ancestry", []) if a["exe_name"] == "codex"}
            both = run.tui.pid in runtime_pids and second.pid in runtime_pids
            run.record("same_queue_entry_competing_receiver_admission", both, characterization_only=True, delayed_delete_with_private_sqlite_write_lock=True, accepted=accepted, marker_events=markers, receiving_runtime_pids=sorted(runtime_pids), same_entry_admitted_by_both=both, pending_after=rpc.items())
    except Exception as exc:
        error = exc
        run.log(type(exc).__name__ + ": " + str(exc))
    finally:
        if lock:
            lock.rollback()
            lock.close()
        for proc in run.children:
            try:
                os.kill(proc.pid, signal.SIGCONT)
            except ProcessLookupError:
                pass
        # Save bounded runtime events before the private home is removed.
        args.output.mkdir(parents=True, exist_ok=True)
        events = []
        for r in run.history():
            p = r.get("payload", {})
            if r.get("type") == "event_msg" and p.get("type") in ("task_started", "task_complete", "hook_started", "hook_completed"):
                events.append({"timestamp": r.get("timestamp"), "payload": {k: p[k] for k in ("type", "turn_id", "last_agent_message") if k in p}})
        (args.output / "runtime-events.json").write_text(json.dumps(events, indent=2) + "\n")
        run.cleanup()
        if second_fd is not None:
            os.close(second_fd)
        run.save(error)
    return int(error is not None or any(not r["passed"] for r in run.results))


if __name__ == "__main__":
    raise SystemExit(main())
