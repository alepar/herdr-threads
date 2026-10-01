#!/usr/bin/env python3
"""Set up or inspect private native runs; never starts an agent or closes a host resource."""

import argparse
import json
import os
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tests" / "native" / "support"))
from fixture import NativeFixture, capture_versions, verify_context


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    create = sub.add_parser("create")
    create.add_argument("run_dir", type=Path)
    record = sub.add_parser("record-owned")
    record.add_argument("run_dir", type=Path)
    record.add_argument("kind", choices=["pane", "agent", "server", "daemon"])
    record.add_argument("id")
    inspect = sub.add_parser("dry-run-cleanup")
    inspect.add_argument("run_dir", type=Path)
    context = sub.add_parser("verify-context")
    context.add_argument("--pane", required=True)
    sub.add_parser("versions")
    args = parser.parse_args()
    if args.command == "create":
        fixture = NativeFixture.create(args.run_dir)
        output = {"run_dir": str(fixture.root), "session_env": fixture.session_env()}
    elif args.command == "record-owned":
        fixture = NativeFixture.open(args.run_dir)
        fixture.record_owned(args.kind, args.id)
        output = {"recorded": {"kind": args.kind, "id": args.id}}
    elif args.command == "dry-run-cleanup":
        fixture = NativeFixture.open(args.run_dir)
        output = {"would_close": list(reversed(fixture.owned_resources()))}
    elif args.command == "verify-context":
        fields = {key: os.environ.get(key) for key in ("HERDR_ENV", "HERDR_WORKSPACE_ID", "HERDR_TAB_ID", "HERDR_PANE_ID")}
        valid = verify_context(fields, args.pane)
        output = {"status": "PASS" if valid else "FAIL", "context": fields}
        print(json.dumps(output, sort_keys=True))
        return 0 if valid else 2
    else:
        output = capture_versions()
    print(json.dumps(output, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
