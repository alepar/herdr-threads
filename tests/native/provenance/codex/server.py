"""One-pane private socket observer for the synthetic Codex probe."""

import argparse
import datetime
import json
import os
import re
import socket
import subprocess
from pathlib import Path


PANE = re.compile(r"w[0-9]+:p[0-9]+\Z")
TOKEN = re.compile(r"[A-Za-z0-9_-]{0,128}\Z")
NONCE = re.compile(r"HT_NONCE_[A-Za-z0-9_-]{4,64}\Z")


def now_utc():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def observe_host(pane):
    agent = json.loads(subprocess.run(
        ["herdr", "agent", "get", pane], capture_output=True, text=True,
        timeout=3, check=True,
    ).stdout)["result"]["agent"]
    process = json.loads(subprocess.run(
        ["herdr", "pane", "process-info", "--pane", pane],
        capture_output=True, text=True, timeout=3, check=True,
    ).stdout)["result"]["process_info"]
    pids = [item["pid"] for item in process["foreground_processes"]
            if item.get("name") == "codex" and type(item.get("pid")) is int]
    return {
        "host_session": agent.get("agent_session", {}).get("value"),
        "host_pid": pids[0] if len(pids) == 1 else None,
        "observed_utc": now_utc(),
    }


def handle_request(payload, expected_pane, observer=observe_host):
    received_utc = now_utc()
    if not isinstance(payload, dict) or payload.get("pane") != expected_pane:
        return {"error": "pane", "received_utc": received_utc}
    nonce = payload.get("nonce")
    context = payload.get("context")
    if not isinstance(nonce, str) or not NONCE.fullmatch(nonce):
        return {"error": "nonce", "received_utc": received_utc}
    if not isinstance(context, str) or not TOKEN.fullmatch(context):
        return {"error": "context", "received_utc": received_utc}
    result = {"nonce": nonce, "context": context, "received_utc": received_utc}
    try:
        result.update(observer(expected_pane))
    except (OSError, subprocess.SubprocessError, ValueError, KeyError, TypeError):
        result["error"] = "host_unavailable"
    return result


def append_private(path, result):
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
    with os.fdopen(descriptor, "a", encoding="utf-8") as output:
        output.write(json.dumps(result, sort_keys=True) + "\n")


def serve(socket_path, evidence_path, pane):
    if not PANE.fullmatch(pane):
        raise ValueError("invalid owned pane ID")
    if os.path.lexists(socket_path):
        raise FileExistsError(socket_path)
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
        listener.bind(socket_path)
        os.chmod(socket_path, 0o600)
        listener.listen(4)
        print("READY", flush=True)
        try:
            while True:
                connection, _ = listener.accept()
                with connection:
                    connection.settimeout(8)
                    try:
                        payload = json.loads(connection.recv(8192))
                        result = handle_request(payload, pane)
                    except (OSError, ValueError, TypeError):
                        result = {"error": "invalid_request", "received_utc": now_utc()}
                    append_private(evidence_path, result)
                    connection.sendall(json.dumps(result, sort_keys=True).encode())
        finally:
            Path(socket_path).unlink(missing_ok=True)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--socket", required=True, help="New socket path in a private directory")
    parser.add_argument("--evidence", required=True, help="Private JSONL output path")
    parser.add_argument("--pane", required=True, help="Exactly one owned Herdr pane ID")
    args = parser.parse_args(argv)
    serve(args.socket, args.evidence, args.pane)


if __name__ == "__main__":
    main()
