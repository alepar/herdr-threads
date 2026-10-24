#!/usr/bin/env python3
"""Probe agy's turn-end extension hooks with genuine agy and a scripted model (ht-kyfw). Zero spend.

    HT_AGY_BIN=/path/to/agy scripts/agy-gate/turn_end_probe.py --evidence-dir DIR [--mode MODE ...]

Each mode runs in its own isolated world (agyenv.World, fake upstream that always answers ``done``), with probe
hooks in the world's own ``~/.gemini/config/hooks.json`` (never the real one) and no herdr-threads plugin:

- ``stop-continue``: Stop always answers ``{"decision":"continue","reason":...}``. Records how many times agy
  re-enters the loop before it stops anyway (the cap), and whether each ``reason`` reaches the next model request.
- ``post-inject``: PostInvocation answers ``injectSteps`` (one systemMessage) on its first call only. Records whether
  agy makes another model call for it in the same turn, or carries it to the next user turn's request.
- ``post-force``: PostInvocation always answers ``injectSteps`` plus ``terminationBehavior: force_continue``.
  Records how many extra model calls agy makes before it stops (the cap).

The relay's request ceiling (40) bounds a mode with no cap of its own; hitting it is recorded as "no cap below 40".
Writes ``<mode>.json`` (hook log, request markers, verdict) and ``summary.json`` to DIR.
"""
from __future__ import annotations

import argparse
import json
import os
import sys
import time
import uuid
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent / "agy-sandbox"))

import agyenv  # noqa: E402
import gate  # noqa: E402
import relay_backend  # noqa: E402

MODES = ("stop-continue", "post-inject", "post-force")
EVENTS = ("Stop", "PostInvocation")
TURN_TIMEOUT_S = 150

HOOK = r'''import json, os, sys
event, ctl = sys.argv[1], sys.argv[2]
payload = json.load(sys.stdin)
mode = open(os.path.join(ctl, "mode")).read().strip()
log = os.path.join(ctl, "hooks.jsonl")
n = sum(1 for line in open(log) if json.loads(line)["event"] == event) + 1 if os.path.exists(log) else 1
out = {}
if event == "Stop" and mode == "stop-continue":
    out = {"decision": "continue", "reason": "PROBE-STOP-REASON-%d" % n}
elif event == "PostInvocation" and mode == "post-inject" and n == 1:
    out = {"injectSteps": [{"systemMessage": {"systemMessage": "PROBE-POST-INJECT-%d" % n}}]}
elif event == "PostInvocation" and mode == "post-force":
    out = {"injectSteps": [{"systemMessage": {"systemMessage": "PROBE-POST-FORCE-%d" % n}}],
           "terminationBehavior": "force_continue"}
keep = ("executionNum", "terminationReason", "error", "fullyIdle", "invocationNum", "initialNumSteps")
with open(log, "a") as f:
    f.write(json.dumps({"event": event, "n": n, "payload": {k: payload.get(k) for k in keep if k in payload},
                        "out": out}) + "\n")
print(json.dumps(out))
'''


def hooks_config(ctl, python="/usr/bin/python3"):
    hook = ctl / "probe_hook.py"
    return {"turn-end-probe": {ev: [{"type": "command", "command": "%s %s %s %s" % (python, hook, ev, ctl),
                                     "timeout": 10}] for ev in EVENTS}}


def markers(bodies, prefix):
    """For each upstream request (in order), the probe markers its body carries."""
    return [sorted({m for m in _find(json.dumps(b), prefix)}) for b in bodies]


def _find(text, prefix):
    i = text.find(prefix)
    while i >= 0:
        j = i + len(prefix)
        while j < len(text) and (text[j].isalnum() or text[j] == "-"):
            j += 1
        yield text[i:j]
        i = text.find(prefix, j)


def run_mode(mode, binary, evidence, owner, run_id):
    backend = relay_backend.RelayBackend(relay_backend.FAKE, script={
        "format": relay_backend.UPSTREAM_FORMAT, "replies": [], "default": {"text": "done", "cost": 0.0}})
    world = agyenv.World(run_id, owner, binary, backend, 1.0)
    record = {"mode": mode}
    with world:
        ctl = world.root / "ctl"
        (ctl / "probe_hook.py").write_text(HOOK)
        (ctl / "mode").write_text(mode)
        cfg = world.home / ".gemini" / "config"
        cfg.mkdir(parents=True, exist_ok=True)
        (cfg / "hooks.json").write_text(json.dumps(hooks_config(ctl), indent=1))
        record["agy_version"] = world.agy_version()
        pane = world.start_agy()
        for turn, text in enumerate(("Reply with the single word ONE.", "Reply with the single word TWO.")):
            before = len(backend.upstream_requests())
            pane.prompt(text)
            time.sleep(3)
            idle = pane.wait("idle", TURN_TIMEOUT_S)
            record["turn%d" % (turn + 1)] = {"idle": bool(idle), "requests": len(backend.upstream_requests()) - before}
            time.sleep(2)
        bodies = backend.upstream_requests()
        log = ctl / "hooks.jsonl"
        record["hooks"] = [json.loads(l) for l in log.read_text().splitlines()] if log.exists() else []
        record["requests_total"] = len(bodies)
        record["request_markers"] = markers(bodies, "PROBE-")
        record["budget"] = backend.budget()
        record["pane_tail"] = pane.read_pane(40)[-2500:]
        record["relay_refusals"] = [r for r in backend.requests() if r.get("refused")]
    return record


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--evidence-dir", required=True)
    ap.add_argument("--mode", action="append", choices=MODES)
    ap.add_argument("--herdr-threads-bin")
    args = ap.parse_args(argv)
    why = gate.refusals(dict(os.environ), sys.platform, gate.herdr_version_text())
    if why:
        sys.stdout.write("turn-end-probe: %s\n" % why)
        return 2
    binary = args.herdr_threads_bin or gate.build_herdr_threads()
    owner, run_id = os.getpid(), str(uuid.uuid4())
    os.environ["HT_TEST_OWNER"], os.environ["HT_LEAK_RUN_ID"] = str(owner), run_id
    out = Path(args.evidence_dir)
    out.mkdir(parents=True, exist_ok=True)
    summary = {}
    for mode in args.mode or MODES:
        try:
            record = run_mode(mode, binary, out, owner, run_id)
        except Exception as exc:  # recorded, never hidden
            record = {"mode": mode, "error": "%s: %s" % (type(exc).__name__, exc)}
        (out / ("%s.json" % mode)).write_text(json.dumps(record, indent=1, sort_keys=True))
        summary[mode] = {k: record.get(k) for k in ("error", "turn1", "turn2", "requests_total")}
        summary[mode]["hook_fires"] = {ev: sum(1 for h in record.get("hooks", []) if h["event"] == ev) for ev in EVENTS}
        sys.stdout.write("%s: %s\n" % (mode, json.dumps(summary[mode])))
    leak_ok, leak_out = agyenv.check_leaks(run_id)
    summary["leak_check"] = {"ok": leak_ok, "output": leak_out[-500:]}
    (out / "summary.json").write_text(json.dumps(summary, indent=1, sort_keys=True))
    sys.stdout.write("leak check: %s\n" % ("clean" if leak_ok else "LEAKS"))
    return 0 if leak_ok else 3


if __name__ == "__main__":
    sys.exit(main())
