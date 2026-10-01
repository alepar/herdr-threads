"""Harmless external executable for the native Codex invocation-context probe."""

import argparse
import json
import os
import re
import socket


NONCE = re.compile(r"HT_NONCE_[A-Za-z0-9_-]{4,64}\Z")


def request(nonce, environment):
    if not NONCE.fullmatch(nonce):
        raise ValueError("expected a synthetic HT_NONCE_* value")
    return {
        "nonce": nonce,
        "context": environment.get("HT_PROBE_CONTEXT", ""),
        "herdr": environment.get("HERDR_ENV", ""),
        "pane": environment.get("HERDR_PANE_ID", ""),
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("nonce")
    parser.add_argument("--socket", help="Private Unix socket for request-arrival observation")
    args = parser.parse_args(argv)
    payload = request(args.nonce, os.environ)
    if args.socket:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
            connection.settimeout(8)
            connection.connect(args.socket)
            connection.sendall(json.dumps(payload).encode())
            result = connection.recv(8192)
        print(json.dumps(json.loads(result), sort_keys=True))
    else:
        print(json.dumps(payload, sort_keys=True))


if __name__ == "__main__":
    main()
