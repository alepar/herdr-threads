#!/usr/bin/env python3
"""Measure legacy chained reads and compact inbox in one private Herdr session."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
import time
import uuid

REPO = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO / "tests/native/recovery"))
from private_host import CommandLog, PrivateHerdr, TestDaemon
from capture_privacy import copy_jsonl, write_json


def utf8_size(value):
    return len(value.encode("utf-8"))


def run(binary, output):
    root = Path(tempfile.mkdtemp(prefix="ih.inbox-overhead-", dir="/private/tmp"))
    run_id = str(uuid.uuid4())
    os.environ.update(HT_LEAK_RUN_ID=run_id, HT_TEST_OWNER=str(os.getpid()),
                      HERDR_THREADS_TEST_OWNER_PID=str(os.getpid()),
                      HOME=str(root / "home"), CLAUDE_CONFIG_DIR=str(root / "claude-config"),
                      CODEX_HOME=str(root / "codex-config"))
    for dirname in ("home", "claude-config", "codex-config", "s"):
        (root / dirname).mkdir(mode=0o700)
    (root / "owner").write_text(str(os.getpid()))
    log = CommandLog(root / "commands.jsonl")
    host = PrivateHerdr(root, log, "inbox")
    daemon = TestDaemon(binary, root / "s", host, log)
    result = {"run_id": run_id, "binary": str(binary),
              "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
              "git_head": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip()}
    try:
        host.start()
        workspace = host.api("workspace.create", {"label": "inbox-overhead", "cwd": str(root), "focus": False})
        sender = workspace["root_pane"]["pane_id"]
        recipient = host.api("pane.split", {"target_pane_id": sender, "direction": "right", "focus": False})["pane"]["pane_id"]
        daemon.ensure()
        sender_seat = daemon.ok("seat", "resolve", "--pane", sender)
        recipient_seat = daemon.ok("seat", "resolve", "--pane", recipient)
        for seat, pane in ((sender_seat, sender), (recipient_seat, recipient)):
            daemon.ok("check-in", "--lifecycle-event", f"inbox-{pane}", "--native-session", f"standin-{pane}",
                      pane=pane, coop=(seat, pane))
        thread = daemon.ok("thread", "create", "--topic", "compact inbox overhead", pane=sender,
                           coop=(sender_seat, sender))
        daemon.ok("invite", thread, "--seat", recipient_seat, "--deadline", "600", pane=sender,
                  coop=(sender_seat, sender))
        daemon.ok("accept", thread, pane=recipient, coop=(recipient_seat, recipient))
        messages = [daemon.ok("send", thread, "--body", f"Message {i}: concise peer update with exact ID.",
                              "--require-ack", recipient_seat, "--deadline", "600", pane=sender,
                              coop=(sender_seat, sender)) for i in range(5)]
        result.update({"thread": thread, "sender_seat": sender_seat, "recipient_seat": recipient_seat,
                       "messages": messages, "pending_before": len(daemon.pending(recipient_seat))})
        assert result["pending_before"] == 5

        common = [str(binary), "--state-dir", str(daemon.state_dir), "--host-endpoint", str(host.socket),
                  "--cooperative-seat", recipient_seat, "--cooperative-target", recipient,
                  "--cooperative-harness", "claude", "--cooperative-role", "top-level"]
        legacy_commands = [common + ["inbox", "--machine"],
                           common + ["pending-receipts"],
                           common + ["read", thread, "--recent", "5"]]
        chain = "; ".join(shlex.join(command) for command in legacy_commands)
        env = host.environment()
        env.update(HERDR_PANE_ID=recipient, HERDR_ENV="1")
        started = time.monotonic()
        baseline = subprocess.run(["/bin/sh", "-c", chain], env=env, capture_output=True, text=True, timeout=45)
        result["baseline"] = {"shell_tool_calls": 1, "cli_commands": 3, "exit_code": baseline.returncode,
                              "stdout_bytes": utf8_size(baseline.stdout), "stderr_bytes": utf8_size(baseline.stderr),
                              "elapsed_ms": (time.monotonic() - started) * 1000,
                              "stdout": baseline.stdout, "stderr": baseline.stderr}
        assert baseline.returncode == 0, baseline.stderr
        assert len(daemon.pending(recipient_seat)) == 5, "legacy reads must be read-only"

        started = time.monotonic()
        compact = subprocess.run(common + ["inbox"], env=env, capture_output=True, text=True, timeout=45)
        result["compact"] = {"shell_tool_calls": 1, "cli_commands": 1, "daemon_requests": 3,
                             "exit_code": compact.returncode, "stdout_bytes": utf8_size(compact.stdout),
                             "stderr_bytes": utf8_size(compact.stderr),
                             "elapsed_ms": (time.monotonic() - started) * 1000,
                             "stdout": compact.stdout, "stderr": compact.stderr}
        assert compact.returncode == 0, compact.stderr
        assert all(message in compact.stdout for message in messages), compact.stdout
        result["pending_after"] = len(daemon.pending(recipient_seat))
        assert result["pending_after"] == 0, result
        result["status"] = "passed"
        print(json.dumps({"status": "passed", "baseline_bytes": result["baseline"]["stdout_bytes"],
                          "compact_bytes": result["compact"]["stdout_bytes"],
                          "baseline_ms": result["baseline"]["elapsed_ms"],
                          "compact_ms": result["compact"]["elapsed_ms"]}), flush=True)
    except BaseException as error:
        result.update(status="failed", error=f"{type(error).__name__}: {error}")
        raise
    finally:
        try:
            if list((root / "s").glob("instances/*/endpoint.json")):
                daemon.stop()
        finally:
            host.stop()
            log.close()
            output.mkdir(parents=True, exist_ok=True)
            if (root / "commands.jsonl").exists():
                copy_jsonl(root / "commands.jsonl", output / "commands.jsonl")
            subprocess.run(["/bin/bash", "-c", '. "$1"; ih_teardown "$2"', "bash",
                            str(REPO / "scripts/lib/isolated-herdr.sh"), str(root)], check=True)
            leak = subprocess.run([str(REPO / "scripts/check-no-leaked-processes"), "--run-id", run_id,
                                   "--root", str(root)], capture_output=True, text=True)
            result["leak_check"] = {"exit_code": leak.returncode, "stdout": leak.stdout, "stderr": leak.stderr}
            write_json(output / "result.json", result)
            assert leak.returncode == 0, result["leak_check"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=REPO / "target/debug/herdr-threads")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    run(args.binary.resolve(), args.output.resolve())


if __name__ == "__main__":
    main()
