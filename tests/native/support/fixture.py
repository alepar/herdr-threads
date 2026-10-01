"""Private native validation resources. No fixture method launches or ACKs an agent."""

import json
import os
from pathlib import Path
import subprocess
import uuid

from evidence import validate_manifest, validate_supporting_artifact


KINDS = frozenset({"pane", "agent", "server", "daemon"})


class NativeFixture:
    def __init__(self, root):
        self.root = Path(root)
        self.evidence_dir = self.root / "evidence"
        self.ledger_path = self.root / "ledger.jsonl"

    @classmethod
    def create(cls, root):
        root = Path(root)
        root.mkdir(mode=0o700, parents=True, exist_ok=False)
        for name in ("config", "state", "hcom", "evidence"):
            (root / name).mkdir(mode=0o700)
        (root / "ledger.jsonl").touch(mode=0o600)
        return cls(root)

    @classmethod
    def open(cls, root):
        fixture = cls(root)
        if not fixture.ledger_path.is_file() or not fixture.evidence_dir.is_dir():
            raise ValueError("not a native fixture run")
        fixture.owned_resources()
        return fixture

    def session_env(self):
        return {"HCOM_DIR": str(self.root / "hcom"), "HCOM_AUTO_APPROVE": "0",
                "HCOM_AUTO_TRUST_WORKSPACE": "0"}

    def record_owned(self, kind, resource_id):
        if kind not in KINDS or not isinstance(resource_id, str) or not resource_id or any(c in resource_id for c in "\n\r\0"):
            raise ValueError("invalid owned resource")
        entry = {"kind": kind, "id": resource_id}
        if any(row["kind"] == kind and row["id"] == resource_id for row in self._ledger_rows()):
            raise ValueError("duplicate owned resource")
        self._append({**entry, "event": "owned"})

    def _append(self, entry):
        with self.ledger_path.open("a", encoding="utf-8") as stream:
            stream.write(json.dumps(entry, sort_keys=True) + "\n")
            stream.flush()
            os.fsync(stream.fileno())

    def _ledger_rows(self):
        raw = self.ledger_path.read_bytes()
        prefix_end = raw.rfind(b"\n") + 1
        complete, tail = raw[:prefix_end], raw[prefix_end:]
        entries = []
        states = {}
        for line in complete.splitlines():
            entry = json.loads(line.decode("utf-8"))
            if not isinstance(entry, dict) or set(entry) != {"kind", "id", "event"} or entry["kind"] not in KINDS or not isinstance(entry["id"], str) or not entry["id"] or entry["event"] not in ("owned", "close_intent", "closed"):
                raise ValueError("malformed resource ledger")
            key = entry["kind"], entry["id"]
            expected = {None: "owned", "owned": "close_intent", "close_intent": "closed"}.get(states.get(key))
            if entry["event"] != expected:
                raise ValueError("invalid resource ledger transition")
            states[key] = entry["event"]
            entries.append(entry)
        # A final unterminated record is a torn append. Save it before truncating,
        # so later appends cannot turn its bytes into an apparently valid event.
        if tail:
            torn = self.root / f"ledger.torn-{uuid.uuid4().hex}.bin"
            descriptor = os.open(torn, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(descriptor, "wb") as stream:
                stream.write(tail)
                stream.flush()
                os.fsync(stream.fileno())
            with self.ledger_path.open("r+b") as stream:
                stream.truncate(prefix_end)
                stream.flush()
                os.fsync(stream.fileno())
        return entries

    def owned_resources(self):
        entries = []
        states = {}
        for entry in self._ledger_rows():
            key = entry["kind"], entry["id"]
            states[key] = entry["event"]
            if entry["event"] == "owned":
                entries.append({"kind": entry["kind"], "id": entry["id"]})
        entries = [entry for entry in entries if states[(entry["kind"], entry["id"])] == "owned"]
        return entries

    def cleanup(self, host):
        # Leave ledger and evidence intact, including when a close fails.
        for resource in reversed(self.owned_resources()):
            self._append({**resource, "event": "close_intent"})
            host.close_exact_owned_id(resource["kind"], resource["id"])
            self._append({**resource, "event": "closed"})

    def write_evidence(self, name, record):
        if Path(name).name != name or name in ("", ".", "..") or not name.endswith(".json"):
            raise ValueError("evidence name must be one JSON basename")
        if not isinstance(record, dict):
            raise ValueError("evidence must be an allowlisted object")
        if "schema_version" in record:
            validate_manifest(record)
        else:
            validate_supporting_artifact(record)
        path = self.evidence_dir / name
        with path.open("x", encoding="utf-8") as stream:
            json.dump(record, stream, sort_keys=True)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        return path


def codex_argv(name, pane_id, native_args=()):
    if not name or not pane_id:
        raise ValueError("name and pane required")
    if any(arg in ("--dangerously-bypass-approvals-and-sandbox", "--yolo") for arg in native_args):
        raise ValueError("blanket permission bypass is prohibited")
    args = list(native_args)
    if "--no-daemon" not in args:
        args.insert(0, "--no-daemon")
    return ["herdr", "agent", "start", name, "--kind", "codex", "--pane", pane_id, "--", *args]


def verify_context(environment, pane_id):
    workspace = pane_id.split(":", 1)[0]
    return (environment.get("HERDR_ENV") == "1" and environment.get("HERDR_PANE_ID") == pane_id
            and environment.get("HERDR_WORKSPACE_ID") == workspace
            and environment.get("HERDR_TAB_ID", "").startswith(workspace + ":t"))


def capture_versions(executable="herdr-threads"):
    """Capture exact command output with finite waits; unavailable tools are explicit."""
    commands = {"herdr": ["herdr", "--version"], "codex": ["codex", "--version"],
                "claude": ["claude", "--version"], "plugin": [executable, "--version"],
                "git_sha": ["git", "rev-parse", "HEAD"]}
    result = {}
    for label, argv in commands.items():
        try:
            process = subprocess.run(argv, capture_output=True, text=True, timeout=5, check=False,
                                     cwd=Path(__file__).resolve().parents[3] if label == "git_sha" else None)
            result[label] = process.stdout.strip() if process.returncode == 0 else {"status": "UNSUPPORTED", "exit_code": process.returncode}
        except (OSError, subprocess.TimeoutExpired) as error:
            result[label] = {"status": "UNSUPPORTED", "reason": type(error).__name__}
    return result
