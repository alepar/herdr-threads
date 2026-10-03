#!/usr/bin/env python3
"""Real private Herdr + deterministic cooperative stand-in wake measurements."""
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

MARKER = "herdr-threads: attention pending; run herdr-threads inbox"


def received(path):
    return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []


def row(daemon, table, seat):
    with sqlite3.connect(f"file:{daemon.instance_dir() / 'threads.sqlite3'}?mode=ro", uri=True) as db:
        db.row_factory = sqlite3.Row
        value = db.execute(f"SELECT * FROM {table} WHERE seat_id=?", (seat,)).fetchone()
        return dict(value) if value else None


def case(binary, output, name, delay, restart):
    root = Path(tempfile.mkdtemp(prefix="ih.wake-overhead-", dir="/private/tmp"))
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
    host = PrivateHerdr(root, log, "wake")
    daemon = TestDaemon(binary, root / "s", host, log)
    before = foreign_processes(root)
    destination = output / name
    destination.mkdir(parents=True, exist_ok=True)
    result = {"case": name, "wake_batch_delay_ms": delay, "restart_during_window": restart,
              "run_id": run_id, "private_root": str(root), "status": "started"}
    try:
        host.start()
        workspace = host.api("workspace.create", {"label": f"wake-{name}", "cwd": str(root), "focus": False,
                       "env": {"PATH": f"{root / 'bin'}:{os.environ.get('PATH', '/usr/bin:/bin')}",
                               "CLAUDE_CONFIG_DIR": str(root / 'claude-config'), "CODEX_HOME": str(root / 'codex-config')}})
        sender = workspace["root_pane"]["pane_id"]
        recipient = host.api("pane.split", {"target_pane_id": sender, "direction": "right", "focus": False})["pane"]["pane_id"]
        initial = daemon.ensure()
        result["schema"] = initial["schema"]
        sender_seat = daemon.ok("seat", "resolve", "--pane", sender)
        recipient_seat = daemon.ok("seat", "resolve", "--pane", recipient)
        for seat, pane in ((sender_seat, sender), (recipient_seat, recipient)):
            daemon.ok("check-in", "--lifecycle-event", f"wake-{name}-{pane}", "--native-session", f"standin-{name}-{pane}",
                      pane=pane, coop=(seat, pane))
        thread = daemon.ok("thread", "create", "--topic", f"wake-{name}", pane=sender, coop=(sender_seat, sender))
        daemon.ok("invite", thread, "--seat", recipient_seat, "--deadline", "600", pane=sender, coop=(sender_seat, sender))
        daemon.ok("accept", thread, pane=recipient, coop=(recipient_seat, recipient))
        # Join with batching enabled first so invitation setup cannot consume the
        # zero-delay case's retry allowance. No receipt has been sent yet.
        assert daemon.stop() == 0
        settings = daemon.instance_dir() / "settings.json"
        settings.write_text(json.dumps({"harness_manifest": "off", "wake_batch_delay_ms": delay,
                                        "minimum_wake_delay_ms": 30_000}))
        settings.chmod(0o600)
        effective = daemon.ensure()
        assert effective["settings"]["wake_batch_delay_ms"] == delay, effective
        host.run_in_pane(recipient, str(stub))
        def idle():
            answer = host.api("agent.get", {"target": recipient}, expect_ok=False)
            agent = answer.get("agent", {})
            return agent.get("agent") == "claude" and agent.get("agent_status") in ("idle", "done")
        wait_for(idle, timeout=20, interval=0.2, description="private stand-in idle")
        state = row(daemon, "wake_work", recipient_seat)
        assert state is None or state["last_reservation_id"] is None, state
        assert not received(receive_path)
        start = time.monotonic()
        start_utc = int(time.time() * 1000)
        sends = []
        for n in range(6):
            remaining = start + n - time.monotonic()
            if remaining > 0:
                time.sleep(remaining)
            message = daemon.ok("send", thread, "--body", f"ordinary burst item {n}", "--require-ack", recipient_seat,
                                "--deadline", "600", pane=sender, coop=(sender_seat, sender))
            sends.append({"message": message, "sent_after_ms": (time.monotonic() - start) * 1000})
        if restart:
            time.sleep(max(0, start + 10 - time.monotonic()))
            before_window = row(daemon, "wake_batches", recipient_seat)
            before_boot = daemon.endpoint()["boot_id"]
            assert before_window is not None and not received(receive_path)
            assert daemon.stop() == 0
            daemon.ensure()
            after_window = row(daemon, "wake_batches", recipient_seat)
            result["restart"] = {"at_ms": (time.monotonic() - start) * 1000,
                                 "before_boot": before_boot, "after_boot": daemon.endpoint()["boot_id"],
                                 "before_deadline": before_window["deadline_at"], "after_deadline": after_window["deadline_at"]}
            assert before_boot != result["restart"]["after_boot"]
            assert before_window["deadline_at"] == after_window["deadline_at"]
        while time.monotonic() < start + 35:
            time.sleep(min(0.2, max(0, start + 35 - time.monotonic())))
        transport_lines = received(receive_path)
        deliveries = [line for line in transport_lines if line["text"]]
        result.update({"start_utc_ms": start_utc, "horizon_ms": (time.monotonic() - start) * 1000,
                       "sends": sends, "received": deliveries, "transport_lines": transport_lines, "wake_count": len(deliveries),
                       "first_wake_after_ms": deliveries[0]["mono_ms"] - start * 1000 if deliveries else None,
                       "wake_work": row(daemon, "wake_work", recipient_seat),
                       "wake_batch": row(daemon, "wake_batches", recipient_seat),
                       "pending_receipts": len(daemon.pending(recipient_seat))})
        assert deliveries and all(item["text"] == MARKER for item in deliveries), deliveries
        assert result["pending_receipts"] == 6, result
        if delay:
            assert 29_500 <= result["first_wake_after_ms"] <= 34_000, result
            assert result["wake_count"] == 1, result
        else:
            assert result["first_wake_after_ms"] < 2_000, result
            assert result["wake_count"] == 2, result
        result["status"] = "passed"
        print(json.dumps({key: result[key] for key in ("case", "status", "wake_count", "first_wake_after_ms", "pending_receipts")}), flush=True)
    except BaseException as error:
        result.update(status="failed", error=f"{type(error).__name__}: {error}")
        print(json.dumps({"case": name, "status": "failed", "error": result["error"]}), flush=True)
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
                    copy_jsonl(source, destination / filename)
            log.close()
            subprocess.run(["/bin/bash", "-c", '. "$1"; ih_teardown "$2"', "bash",
                            str(REPO / "scripts/lib/isolated-herdr.sh"), str(root)], check=True)
            leak = subprocess.run([str(REPO / "scripts/check-no-leaked-processes"), "--run-id", run_id,
                                   "--root", str(root)], capture_output=True, text=True)
            result["leak_check"] = {"exit_code": leak.returncode, "stdout": leak.stdout, "stderr": leak.stderr}
            write_json(destination / "result.json", result)
            assert leak.returncode == 0, result["leak_check"]
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=REPO / "target/debug/herdr-threads")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--cases", default="30s,0s,restart")
    args = parser.parse_args()
    binary = args.binary.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    definitions = {"30s": (30_000, False), "0s": (0, False), "restart": (30_000, True)}
    metadata = {"binary": str(binary), "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                "git_head": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip(),
                "herdr_version": subprocess.check_output(["herdr", "--version"], text=True).strip()}
    write_json(args.output / "metadata.json", metadata)
    results = [case(binary, args.output, name, *definitions[name]) for name in args.cases.split(",")]
    write_json(args.output / "results.json", results)


if __name__ == "__main__":
    main()
