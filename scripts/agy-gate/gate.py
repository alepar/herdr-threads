#!/usr/bin/env python3
"""agy native gate runner (test-infra spec D6, D8, D12).

  scripts/agy-native-gate [--flow NAME]... [--list] [--herdr-threads-bin PATH] [--report PATH]
                          [--keep-on-fail] [--timeout-scale F]

Exit codes: 0 every selected flow passed; 1 a flow failed; 2 environment refusal; 3 the leak
check failed. Never runs in CI. Python 3.9 compatible, standard library only.
"""
from __future__ import annotations

import argparse
import dataclasses
import importlib.util
import json
import os
import re
import signal
import subprocess
import sys
import time
import traceback
import uuid
from pathlib import Path
from typing import Callable, Tuple

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
SANDBOX_DIR = REPO / "scripts" / "agy-sandbox"
for _dir in (HERE, SANDBOX_DIR):
    if str(_dir) not in sys.path:
        sys.path.insert(0, str(_dir))
# Flow modules do ``from gate import Flow``; make that resolve to this module when run as a script.
sys.modules.setdefault("gate", sys.modules[__name__])

MIN_HERDR = (0, 9, 1)
EXIT_OK, EXIT_FAIL, EXIT_REFUSED, EXIT_LEAK = 0, 1, 2, 3


@dataclasses.dataclass(frozen=True)
class Flow:
    name: str
    requires: Tuple[str, ...]
    run: Callable


class Refusal(Exception):
    pass


def discover(flows_dir):
    """Every ``flows/*.py`` module's ``FLOW``, sorted by flow name (adding a flow edits no shared file)."""
    flows = []
    for path in sorted(Path(flows_dir).glob("*.py")):
        if path.name.startswith("_"):
            continue
        spec = importlib.util.spec_from_file_location("agy_gate_flow_" + path.stem, str(path))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        flow = getattr(module, "FLOW", None)
        if flow is None or not all(hasattr(flow, a) for a in ("name", "requires", "run")):
            raise Refusal("flow module %s exports no FLOW" % path.name)
        flows.append(flow)
    names = [f.name for f in flows]
    if len(set(names)) != len(names):
        raise Refusal("duplicate flow names: %s" % names)
    return sorted(flows, key=lambda f: f.name)


def parse_herdr_version(text):
    match = re.search(r"(\d+)\.(\d+)\.(\d+)", text or "")
    return tuple(int(g) for g in match.groups()) if match else None


def refusals(env, platform_name, herdr_version_text):
    """The first environment refusal (a message), or None. Pure: every input is a parameter."""
    if env.get("CI") or env.get("GITHUB_ACTIONS"):
        return "refusing to run under CI/GITHUB_ACTIONS: the native gate never runs in CI"
    if not env.get("HT_AGY_BIN"):
        return "set HT_AGY_BIN to a genuine agy binary (scripts/agy-sandbox/fetch-agy can download one)"
    import agyenv
    why = agyenv.sandbox_refusal(platform_name)
    if why:
        return "the native gate %s" % why
    if not os.path.isfile(env["HT_AGY_BIN"]) or not os.access(env["HT_AGY_BIN"], os.X_OK):
        return "HT_AGY_BIN is not an executable file: %s" % env["HT_AGY_BIN"]
    version = parse_herdr_version(herdr_version_text)
    if version is None:
        return "herdr not found or its version is unreadable (need herdr %s or newer)" % ".".join(map(str, MIN_HERDR))
    if version < MIN_HERDR:
        return "herdr %s is older than %s, which first detects agy" % (
            ".".join(map(str, version)), ".".join(map(str, MIN_HERDR)))
    return None


def herdr_version_text():
    try:
        result = subprocess.run(["herdr", "--version"], stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT, universal_newlines=True, timeout=15)
    except (OSError, subprocess.TimeoutExpired):
        return ""
    return result.stdout.strip()


def git_info():
    def git(*args):
        return subprocess.run(["git", "-C", str(REPO)] + list(args), stdout=subprocess.PIPE,
                              stderr=subprocess.DEVNULL, universal_newlines=True).stdout.strip()
    return {"sha": git("rev-parse", "HEAD"), "dirty": bool(git("status", "--porcelain"))}


def build_herdr_threads():
    """``cargo build --locked --all-features --bin herdr-threads``; the built path, or a Refusal."""
    result = subprocess.run(["cargo", "build", "--locked", "--all-features", "--bin", "herdr-threads"],
                            cwd=str(REPO), stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, universal_newlines=True)
    if result.returncode != 0:
        raise Refusal("build failed:\n" + result.stdout[-2000:])
    target = os.environ.get("CARGO_TARGET_DIR") or str(REPO / "target")
    return str(Path(target) / "debug" / "herdr-threads")


class Interrupted(SystemExit):
    pass


def _raise_interrupted(signum, _frame):
    raise Interrupted(128 + signum)


def run_flow(flow, args, run_id, owner, binary, report_dir, world_factory, backend_factory):
    started = time.monotonic()
    record = {"name": flow.name, "requires": list(flow.requires), "status": "fail", "checks": [],
              "artifacts": str(report_dir / flow.name), "error": None}
    world = None
    try:
        world = world_factory(run_id, owner, binary, backend_factory(), args.timeout_scale,
                              artifact_dir=report_dir / flow.name, keep_on_fail=args.keep_on_fail)
        with world:
            try:
                record["agy"] = dict(world.agy_info)
                try:
                    record["agy"]["version"] = world.agy_version()
                except Exception as error:  # a version probe never fails the flow
                    record["agy"]["version"] = "unknown (%s)" % error
                record["root"] = str(world.root)
                flow.run(world)
            finally:
                record["checks"] = list(world.checks)
    except Interrupted:
        raise
    except Exception as error:
        record["error"] = "%s: %s" % (type(error).__name__, error)
        record["traceback"] = traceback.format_exc()[-3000:]
        if world is not None:
            record["checks"] = list(world.checks)
    else:
        if all(c["ok"] for c in record["checks"]):
            record["status"] = "pass"
    record["duration_s"] = round(time.monotonic() - started, 2)
    return record


def main(argv=None, env=None, platform_name=None, herdr_version_fn=None, leak_check=None,
         world_factory=None, backend_factory=None, flows_dir=None, build_fn=None, out=None):
    env = os.environ if env is None else env
    out = out or sys.stdout
    ap = argparse.ArgumentParser(prog="agy-native-gate", description=__doc__.splitlines()[0])
    ap.add_argument("--flow", action="append", default=[])
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--herdr-threads-bin")
    ap.add_argument("--report")
    ap.add_argument("--keep-on-fail", action="store_true")
    ap.add_argument("--timeout-scale", type=float, default=1.0)
    args = ap.parse_args(argv)

    try:
        flows = discover(flows_dir or HERE / "flows")
    except Refusal as error:
        out.write("agy-native-gate: %s\n" % error)
        return EXIT_REFUSED
    if args.list:
        for flow in flows:
            out.write("%s\trequires: %s\n" % (flow.name, ", ".join(flow.requires) or "-"))
        return EXIT_OK
    known = {f.name for f in flows}
    unknown = [n for n in args.flow if n not in known]
    if unknown:
        out.write("agy-native-gate: unknown flow(s): %s (known: %s)\n" % (", ".join(unknown), ", ".join(sorted(known))))
        return EXIT_REFUSED
    selected = [f for f in flows if not args.flow or f.name in args.flow]

    version_text = (herdr_version_fn or herdr_version_text)()
    refusal = refusals(env, platform_name or sys.platform, version_text)
    if refusal:
        out.write("agy-native-gate: %s\n" % refusal)
        return EXIT_REFUSED

    import agyenv
    import observe
    observe.set_timeout_scale(args.timeout_scale)
    world_factory = world_factory or agyenv.World
    backend_factory = backend_factory or agyenv.FakeGeminiBackend
    owner = os.getpid()
    run_id = str(uuid.uuid4())
    os.environ["HT_TEST_OWNER"] = str(owner)
    os.environ["HT_LEAK_RUN_ID"] = run_id
    env_agy = env["HT_AGY_BIN"]
    try:
        binary = args.herdr_threads_bin or (build_fn or build_herdr_threads)()
    except Refusal as error:
        out.write("agy-native-gate: %s\n" % error)
        return EXIT_REFUSED
    report_path = Path(args.report) if args.report else REPO / "target" / "agy-gate" / ("report-%s.json" % run_id)
    report_dir = report_path.parent / ("artifacts-" + run_id)

    previous = {}
    for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
        previous[sig] = signal.signal(sig, _raise_interrupted)
    flow_records = []
    interrupted = None
    try:
        for flow in selected:
            flow_records.append(run_flow(flow, args, run_id, owner, binary, report_dir, world_factory,
                                         backend_factory))
    except Interrupted as stop:
        interrupted = stop
    finally:
        for sig, handler in previous.items():
            signal.signal(sig, handler)

    leak_ok, leak_output = (leak_check or agyenv.check_leaks)(run_id)
    try:
        agy_sha = agyenv.sha512_file(env_agy)
        agy_size = os.path.getsize(env_agy)
    except OSError:
        agy_sha, agy_size = None, None
    report = {
        "run_id": run_id, "owner_pid": owner, "herdr": version_text,
        "herdr_threads": dict(git_info(), bin=str(binary)),
        "agy": {"bin": env_agy, "sha512": agy_sha, "size": agy_size},
        "flows": flow_records, "leak_check": {"ok": leak_ok, "output": leak_output},
        "interrupted": bool(interrupted),
    }
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(json.dumps(report, indent=2, sort_keys=True))
    for rec in flow_records:
        failed = [c["name"] for c in rec["checks"] if not c["ok"]]
        out.write("%-12s %s (%ss)%s\n" % (rec["name"], rec["status"].upper(), rec["duration_s"],
                                           "  failed checks: " + "; ".join(failed) if failed else ""))
        if rec["error"]:
            out.write("  error: %s\n" % rec["error"])
    out.write("leak check: %s\nreport: %s\n" % ("clean" if leak_ok else "LEAKS\n" + leak_output, report_path))
    if interrupted is not None:
        return interrupted.code
    if not all(r["status"] == "pass" for r in flow_records):
        return EXIT_FAIL
    return EXIT_OK if leak_ok else EXIT_LEAK


if __name__ == "__main__":
    sys.exit(main())
