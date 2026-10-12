"""Read-only observation primitives for the native gate (test-infra spec D6).

Flow-specific queries live in the flow module; this file holds only what every flow shares.
"""
from __future__ import annotations

import json
import sqlite3
import time

TIMEOUT_SCALE = 1.0


def set_timeout_scale(scale):
    global TIMEOUT_SCALE
    TIMEOUT_SCALE = float(scale)


def wait_until(pred, timeout, step=0.1):
    """Poll ``pred`` until it is truthy or ``timeout`` (scaled by ``--timeout-scale``) passes.

    Returns the last value of ``pred``, so a caller can both test it and report it."""
    deadline = time.monotonic() + timeout * TIMEOUT_SCALE
    while True:
        value = pred()
        if value or time.monotonic() >= deadline:
            return value
        time.sleep(step)


def db_path(world):
    found = sorted(world.state.glob("**/threads.sqlite3"))
    return found[0] if found else None


def db(world):
    """A read-only SQLite connection to the daemon database (``mode=ro``), or None before it exists."""
    path = db_path(world)
    if path is None:
        return None
    conn = sqlite3.connect("file:%s?mode=ro" % path, uri=True, timeout=5)
    conn.row_factory = sqlite3.Row
    return conn


def cli_json(world, argv, pane=None):
    """Run ``herdr-threads --json <argv>`` and return the decoded ``result.data`` (or the whole object)."""
    result = world.ht(["--json"] + list(argv), pane=pane, check=False)
    try:
        doc = json.loads(result.stdout)
    except ValueError:
        return {"_rc": result.returncode, "_stdout": result.stdout, "_stderr": result.stderr}
    if isinstance(doc, dict) and isinstance(doc.get("result"), dict) and "data" in doc["result"]:
        return doc["result"]["data"]
    return doc


def main_requests(world):
    return [r for r in world.backend.requests() if r.get("kind") == "main"]


def backend_clean(world):
    """``(ok, detail)``: no main request was unmatched or hit a template error."""
    bad = [r for r in world.backend.requests() if r.get("unmatched") or r.get("template_error")]
    return (not bad, "unmatched/template_error requests: %s" % [r.get("seq") for r in bad] if bad else "")
