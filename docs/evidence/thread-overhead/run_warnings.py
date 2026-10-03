#!/usr/bin/env python3
"""Exercise warning open, quiet persistence, clear, restart, and reopen on a private Herdr."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import time
import uuid

REPO = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO / "tests/native/recovery"))
from private_host import CommandLog, PrivateHerdr, TestDaemon, foreign_processes, wait_for
from capture_privacy import copy_jsonl, write_json


def rows(daemon, statement, args=()):
    database = daemon.instance_dir() / "threads.sqlite3"
    with sqlite3.connect(f"file:{database}?mode=ro", uri=True) as connection:
        connection.row_factory = sqlite3.Row
        return [dict(row) for row in connection.execute(statement, args)]


def received(path):
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text().splitlines()]


def deliveries(path):
    return [line for line in received(path) if line["text"]]


def snapshot(daemon, thread):
    conditions = rows(daemon, "SELECT ordinal,condition_kind,thread_id,condition_id,open_warning_id,clear_warning_id,opened_seq,cleared_seq FROM warning_conditions WHERE thread_id=? ORDER BY ordinal", (thread,))
    warning_jobs = rows(daemon, "SELECT warning_id,event_seq,status,interval_high_water FROM warning_jobs WHERE thread_id=? ORDER BY ordinal", (thread,))
    active = daemon.ok("warnings", "--active", thread, "--limit", "100")
    return {"conditions": conditions, "warning_jobs": warning_jobs,
            "active_ids": [item["warning"] for item in active["items"]]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=REPO / "target/debug/herdr-threads")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    root = Path(tempfile.mkdtemp(prefix="ih.warning-overhead-", dir="/private/tmp"))
    run_id = str(uuid.uuid4())
    os.environ.update(HT_LEAK_RUN_ID=run_id, HT_TEST_OWNER=str(os.getpid()),
                      HERDR_THREADS_TEST_OWNER_PID=str(os.getpid()),
                      CLAUDE_CONFIG_DIR=str(root / "claude-config"), CODEX_HOME=str(root / "codex-config"))
    for directory in (root / "claude-config", root / "codex-config", root / "s", root / "bin"):
        directory.mkdir(mode=0o700)
    (root / "owner").write_text(str(os.getpid()))
    capture = root / "capture.py"
    capture.write_text("import sys,json,time\nwith open(sys.argv[1],'a') as f: f.write(json.dumps({'mono_ms':time.monotonic()*1000,'utc_ms':int(time.time()*1000),'text':sys.argv[2]})+'\\n')\n")
    receive_path = root / "received.jsonl"
    stub = root / "bin/claude"
    stub.write_text(f"#!/bin/sh\nif [ \"${{1:-}}\" = --version ]; then echo '2.1.286 (Claude Code)'; exit 0; fi\necho HT-STANDIN-START\nwhile IFS= read -r line; do python3 '{capture}' '{receive_path}' \"$line\"; echo \"HT-WAKE-RECEIVED: $line\"; done\n")
    stub.chmod(0o755)
    log = CommandLog(root / "commands.jsonl")
    host = PrivateHerdr(root, log, "warning")
    daemon = TestDaemon(binary, root / "s", host, log)
    before = foreign_processes(root)
    result = {"run_id": run_id, "binary": str(binary),
              "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
              "herdr_version": subprocess.check_output(["herdr", "--version"], text=True).strip(),
              "status": "started", "stages": {}}
    try:
        host.start()
        workspace = host.api("workspace.create", {"label": "warning-lifecycle", "cwd": str(root), "focus": False,
                             "env": {"PATH": f"{root / 'bin'}:{os.environ.get('PATH', '/usr/bin:/bin')}",
                                     "CLAUDE_CONFIG_DIR": str(root / "claude-config"),
                                     "CODEX_HOME": str(root / "codex-config")}})
        sender = workspace["root_pane"]["pane_id"]
        recipient = host.api("pane.split", {"target_pane_id": sender, "direction": "right", "focus": False})["pane"]["pane_id"]
        result["schema"] = daemon.ensure()["schema"]
        sender_seat = daemon.ok("seat", "resolve", "--pane", sender)
        recipient_seat = daemon.ok("seat", "resolve", "--pane", recipient)
        for seat, pane in ((sender_seat, sender), (recipient_seat, recipient)):
            daemon.ok("check-in", "--lifecycle-event", f"warning-{pane}", "--native-session", f"standin-{pane}",
                      pane=pane, coop=(seat, pane))
        thread = daemon.ok("thread", "create", "--topic", "private-warning-lifecycle",
                           pane=sender, coop=(sender_seat, sender))
        daemon.ok("invite", thread, "--seat", recipient_seat, "--deadline", "600",
                  pane=sender, coop=(sender_seat, sender))
        daemon.ok("accept", thread, pane=recipient, coop=(recipient_seat, recipient))
        assert daemon.stop() == 0
        settings = daemon.instance_dir() / "settings.json"
        settings.write_text(json.dumps({"harness_manifest": "off", "wake_batch_delay_ms": 0,
                                        "minimum_wake_delay_ms": 30_000}))
        settings.chmod(0o600)
        daemon.ensure()
        host.run_in_pane(recipient, str(stub))
        wait_for(lambda: host.api("agent.get", {"target": recipient}, expect_ok=False).get("agent", {}).get("agent_status") in ("idle", "done"),
                 timeout=20, interval=0.2, description="private stand-in idle")
        result["setup"] = {"thread": thread, "sender_seat": sender_seat, "recipient_seat": recipient_seat}

        started = time.monotonic()
        first = daemon.ok("send", thread, "--body", "first overdue", "--require-ack", recipient_seat,
                          "--deadline", "1", pane=sender, coop=(sender_seat, sender))
        wait_for(lambda: len(rows(daemon, "SELECT ordinal FROM warning_conditions WHERE condition_kind='receipt' AND thread_id=?", (thread,))) == 1,
                 timeout=25, interval=0.2, description="first overdue warning")
        result["stages"]["first_open"] = {"after_ms": (time.monotonic()-started)*1000,
                                             "message": first, "wake_count": len(deliveries(receive_path)),
                                             **snapshot(daemon, thread)}

        second = daemon.ok("send", thread, "--body", "second overdue", "--require-ack", recipient_seat,
                           "--deadline", "1", pane=sender, coop=(sender_seat, sender))
        wait_for(lambda: bool(rows(daemon, "SELECT warning_message_id FROM receipts WHERE message_id=? AND warning_message_id IS NOT NULL UNION SELECT warning_message_id FROM receipt_state WHERE message_id=? AND warning_message_id IS NOT NULL", (second, second))),
                 timeout=25, interval=0.2, description="second overdue marker")
        quiet = snapshot(daemon, thread)
        assert len(quiet["conditions"]) == 1 and len(quiet["warning_jobs"]) == 1, quiet
        result["stages"]["quiet_persistence"] = {"after_ms": (time.monotonic()-started)*1000,
                                                      "message": second, "wake_count": len(deliveries(receive_path)),
                                                      **quiet}

        # A send can consume the immediate wake. The 30-second minimum delay
        # then gates the overdue warning's next notification attempt.
        wait_for(lambda: len(deliveries(receive_path)) >= 2,
                 timeout=40, interval=0.2, description="overdue warning wake")
        result["stages"]["persistent_wake"] = {"after_ms": (time.monotonic()-started)*1000,
                                                   "wake_count": len(deliveries(receive_path)),
                                                   **snapshot(daemon, thread)}

        daemon.ok("ack", first, pane=recipient, coop=(recipient_seat, recipient))
        one_pending = snapshot(daemon, thread)
        assert len(one_pending["active_ids"]) == 1, one_pending
        result["stages"]["first_ack"] = {"after_ms": (time.monotonic()-started)*1000, **one_pending}
        daemon.ok("ack", second, pane=recipient, coop=(recipient_seat, recipient))
        wait_for(lambda: snapshot(daemon, thread)["conditions"][0]["clear_warning_id"] is not None,
                 timeout=10, interval=0.2, description="clear warning")
        clear = snapshot(daemon, thread)
        assert len(clear["warning_jobs"]) == 2 and not clear["active_ids"], clear
        result["stages"]["clear"] = {"after_ms": (time.monotonic()-started)*1000,
                                        "wake_count": len(deliveries(receive_path)), **clear}

        before_boot = daemon.endpoint()["boot_id"]
        assert daemon.stop() == 0
        daemon.ensure()
        after_boot = daemon.endpoint()["boot_id"]
        assert before_boot != after_boot
        restarted = snapshot(daemon, thread)
        assert restarted["conditions"] == clear["conditions"] and not restarted["active_ids"], restarted
        result["stages"]["restart"] = {"after_ms": (time.monotonic()-started)*1000,
                                           "before_boot": before_boot, "after_boot": after_boot, **restarted}

        third = daemon.ok("send", thread, "--body", "third overdue", "--require-ack", recipient_seat,
                          "--deadline", "1", pane=sender, coop=(sender_seat, sender))
        wait_for(lambda: len(snapshot(daemon, thread)["conditions"]) == 2,
                 timeout=25, interval=0.2, description="reopened warning")
        reopened = snapshot(daemon, thread)
        assert len(reopened["warning_jobs"]) == 3 and len(reopened["active_ids"]) == 1, reopened
        result["stages"]["reopen"] = {"after_ms": (time.monotonic()-started)*1000,
                                          "message": third, "wake_count": len(deliveries(receive_path)), **reopened}
        # Observe any notification allowed by the same 30-second throttle.
        wait_for(lambda: len(deliveries(receive_path)) >= 3,
                 timeout=40, interval=0.2, description="reopened warning wake")
        result["stages"]["reopen_wake"] = {"after_ms": (time.monotonic()-started)*1000,
                                               "wake_count": len(deliveries(receive_path)),
                                               **snapshot(daemon, thread)}
        result["transport_lines"] = received(receive_path)
        result["received"] = deliveries(receive_path)
        result["wake_count"] = len(result["received"])
        result["first_wake_after_ms"] = (result["received"][0]["mono_ms"] - started*1000) if result["received"] else None
        result["status"] = "passed"
        print(json.dumps({"status": "passed", "warning_jobs": len(reopened["warning_jobs"]),
                          "wake_count": result["wake_count"], "first_wake_after_ms": result["first_wake_after_ms"]}), flush=True)
    except BaseException as error:
        result.update(status="failed", error=f"{type(error).__name__}: {error}")
        print(json.dumps({"status": "failed", "error": result["error"]}), flush=True)
        raise
    finally:
        try:
            if list((root / "s").glob("instances/*/endpoint.json")):
                daemon.stop()
        finally:
            host.stop()
            after = foreign_processes(root)
            result["existing_foreign_servers_unchanged"] = all(after.get(pid) == value for pid, value in before.items()
                                                              if "herdr server" in value["command"])
            for filename in ("commands.jsonl", "received.jsonl"):
                source = root / filename
                if source.exists():
                    copy_jsonl(source, output / filename)
            log.close()
            subprocess.run(["/bin/bash", "-c", '. "$1"; ih_teardown "$2"', "bash",
                            str(REPO / "scripts/lib/isolated-herdr.sh"), str(root)], check=True)
            leak = subprocess.run([str(REPO / "scripts/check-no-leaked-processes"), "--run-id", run_id,
                                   "--root", str(root)], capture_output=True, text=True)
            result["leak_check"] = {"exit_code": leak.returncode, "stdout": leak.stdout, "stderr": leak.stderr}
            write_json(output / "result.json", result)
            assert leak.returncode == 0, result["leak_check"]


if __name__ == "__main__":
    main()
