"""``RelayBackend``: a ``ModelBackend`` that fronts agy with ``scripts/agy-relay/relay.py`` (test-infra spec D10, D11).

The relay (and, for ``--dry-run``, ``fake_upstream.py``) runs OUTSIDE the agy sandbox and is started
with ``agyenv.spawn_owned`` and ``--owner-pid``, so the leak check and the reaper own it. Its run
directory is ``<world root>/relay``; the relay derives the ``<root>`` redaction from that placement.

Python 3.9 compatible, standard library only.
"""
from __future__ import annotations

import json
import os
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
RELAY_DIR = REPO / "scripts" / "agy-relay"
SANDBOX_DIR = REPO / "scripts" / "agy-sandbox"
for _dir in (HERE, SANDBOX_DIR):
    if str(_dir) not in sys.path:
        sys.path.insert(0, str(_dir))

import agyenv  # noqa: E402

UPSTREAM_FORMAT = "agy-relay-fake-upstream/1"
FAKE, LIVE = "fake", "live"


def read_text(path):
    try:
        return Path(path).read_text().strip()
    except OSError:
        return ""


def read_jsonl(path):
    out = []
    try:
        text = Path(path).read_text()
    except OSError:
        return out
    for line in text.splitlines():
        line = line.strip()
        if line:
            try:
                out.append(json.loads(line))
            except ValueError:
                continue
    return out


class RelayBackend(agyenv.ModelBackend):
    """``start(world) -> (base_url, token)`` read from the relay's run dir; ``requests()`` is ``relay.jsonl``."""

    def __init__(self, mode=FAKE, script=None, relay_py=None, upstream_py=None, python=None,
                 max_requests=None, max_cost_usd=None, startup_timeout=20):
        if mode not in (FAKE, LIVE):
            raise ValueError("mode must be %r or %r" % (FAKE, LIVE))
        self.mode = mode
        self.script = script
        self.relay_py = Path(relay_py or (RELAY_DIR / "relay.py"))
        self.upstream_py = Path(upstream_py or (RELAY_DIR / "fake_upstream.py"))
        self.python = python or sys.executable
        self.max_requests = max_requests
        self.max_cost_usd = max_cost_usd
        self.startup_timeout = startup_timeout
        self.run_dir = None
        self.upstream_dir = None
        self.relay_proc = None
        self.upstream_proc = None

    # -- lifecycle
    def _wait_file(self, path, proc, label, scale):
        agyenv.wait_until(lambda: path.exists() or proc.poll() is not None, self.startup_timeout * scale)
        if not path.exists():
            log = read_text(path.parent / ("relay.log" if label == "relay" else "server.log"))
            raise agyenv.WorldError("%s did not become ready (rc=%s): %s" % (label, proc.poll(), log[-400:]))

    def _spawn(self, world, argv, log_path):
        log = open(str(log_path), "ab")
        try:
            return agyenv.spawn_owned([self.python, "-I", "-B"] + argv, world.base_env(), world.owner_pid,
                                      stdout=log, stderr=log)
        finally:
            log.close()

    def start(self, world):
        scale = world.timeout_scale
        self.run_dir = Path(world.root) / "relay"
        self.run_dir.mkdir(parents=True, exist_ok=True)
        for stale in ("port", "local-token"):  # a leftover file would pass for readiness
            (self.run_dir / stale).unlink(missing_ok=True)
        argv = [str(self.relay_py), "--run-dir", str(self.run_dir), "--owner-pid", str(world.owner_pid)]
        if self.mode == FAKE:
            self.upstream_dir = Path(world.root) / "upstream"
            self.upstream_dir.mkdir(parents=True, exist_ok=True)
            if self.script is not None:
                self.load(self.script)
            self.upstream_proc = self._spawn(
                world, [str(self.upstream_py), "--ctl", str(self.upstream_dir), "--owner-pid",
                        str(world.owner_pid)], self.upstream_dir / "server.log")
            self._wait_file(self.upstream_dir / "port", self.upstream_proc, "fake upstream", scale)
            argv += ["--fake-upstream", "http://127.0.0.1:%s" % read_text(self.upstream_dir / "port")]
        else:
            argv += ["--live"]
        if self.max_requests is not None:
            argv += ["--max-requests", str(self.max_requests)]
        if self.max_cost_usd is not None:
            argv += ["--max-cost-usd", str(self.max_cost_usd)]
        self.relay_proc = self._spawn(world, argv, self.run_dir / "relay.log")
        self._wait_file(self.run_dir / "port", self.relay_proc, "relay", scale)
        port = read_text(self.run_dir / "port")
        token = read_text(self.run_dir / "local-token")
        if not port or not token:
            raise agyenv.WorldError("relay run dir holds no port or local-token")
        return "http://127.0.0.1:%s" % port, token

    def stop(self):
        for attr in ("relay_proc", "upstream_proc"):
            proc = getattr(self, attr)
            if proc is not None:
                agyenv.stop_group(proc)
                setattr(self, attr, None)

    # -- observation
    def requests(self):
        return read_jsonl(self.run_dir / "relay.jsonl") if self.run_dir else []

    def upstream_requests(self):
        """Full OpenAI bodies the fake upstream received (dry-run only; never saved as evidence)."""
        return read_jsonl(self.upstream_dir / "requests.jsonl") if self.upstream_dir else []

    def budget(self):
        try:
            return json.loads((self.run_dir / "budget.json").read_text())
        except (OSError, ValueError, TypeError):
            return {}

    def alive(self):
        return self.relay_proc is not None and self.relay_proc.poll() is None

    def load(self, scenario):
        """Replace the fake upstream's ``script.json`` (dry-run only)."""
        if self.mode != FAKE:
            raise agyenv.WorldError("only the fake-upstream mode takes a script")
        if not isinstance(scenario, dict) or scenario.get("format") != UPSTREAM_FORMAT:
            raise ValueError("script format must be %r" % UPSTREAM_FORMAT)
        target = self.upstream_dir / "script.json"
        tmp = self.upstream_dir / ".script.tmp"
        tmp.write_text(json.dumps(scenario))
        os.replace(str(tmp), str(target))
