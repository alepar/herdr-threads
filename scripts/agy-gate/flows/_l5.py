"""Helpers shared by the L5 flows (enroll, subagent, clear, resume); not a flow itself.

Flow-local DB queries over the read-only daemon database (test-infra spec D6 observation 1) and
the request-log accessors the four flows share. The leading underscore keeps ``gate.discover`` from
loading this module as a flow.
"""
from __future__ import annotations

import json
import sqlite3
import sys
from pathlib import Path

GATE_DIR = Path(__file__).resolve().parent.parent
for _dir in (GATE_DIR, GATE_DIR.parent / "agy-sandbox"):
    if str(_dir) not in sys.path:
        sys.path.insert(0, str(_dir))

import observe  # noqa: E402

# Opening words of ``harness::TOP_LEVEL_INSTRUCTION``: the startup check-in a top-level SessionStart injects.
STARTUP_MARK = "Complete hook/channel deliveries and default text inbox displays ACK"
# Opening words of ``harness::CHILD_RESTRICTION``: the restriction every role's context carries.
CHILD_MARK = "Subagents may discover, read and summarize"
SKILL_PATH = "skills/herdr-threads/SKILL.md"

# Session-keyed bindings (spec §1): a seat keeps one row per native session, so a binding is "live" when it
# is the seat's connected top-level row. ``generation`` is the seat's mapping generation at enrollment, not a
# per-binding counter, and a row left behind by an exited or cleared session stays open but ``disconnected``.
_COLUMNS = ("binding_id, seat_id, ordinal, generation, harness, native_session, observation_provenance, "
            "role, liveness, lineage_from, ended_at")
BINDING_SQL = ("SELECT " + _COLUMNS + " FROM occupant_bindings WHERE harness='agy' AND ended_at IS NULL "
               "AND role='top_level' AND liveness='connected' ORDER BY seat_id, ordinal DESC")
ALL_BINDINGS_SQL = ("SELECT " + _COLUMNS + " FROM occupant_bindings WHERE harness='agy' "
                    "ORDER BY seat_id, ordinal")


def load_scenario(path):
    return json.loads(Path(path).read_text())


def query(world, sql):
    """Rows of a read-only query as dicts; ``[]`` before the database or its tables exist."""
    try:
        conn = observe.db(world)
        if conn is None:
            return []
        try:
            return [dict(row) for row in conn.execute(sql).fetchall()]
        finally:
            conn.close()
    except sqlite3.Error:
        return []


def live_bindings(world):
    """The live agy binding of each seat: its connected, open top-level row with the highest ordinal."""
    seen, out = set(), []
    for row in query(world, BINDING_SQL):
        if row["seat_id"] not in seen:
            seen.add(row["seat_id"])
            out.append(row)
    return out


def all_bindings(world):
    """Every agy binding row, any role, liveness or end state, oldest first (diagnostics and history checks)."""
    return query(world, ALL_BINDINGS_SQL)


def binding_for(world, native_session):
    """The agy binding row keyed by ``native_session`` (None when there is none)."""
    rows = [r for r in all_bindings(world) if r["native_session"] == native_session and r["role"] == "top_level"]
    return rows[-1] if rows else None


def live_binding(world):
    rows = live_bindings(world)
    return rows[0] if len(rows) == 1 else None


def seat_count(world):
    rows = query(world, "SELECT count(*) AS n FROM seats")
    return rows[0]["n"] if rows else None


def presence_ids(world):
    base = world.agy_data / "presence"
    return sorted(p.name[:-len(".lock")] for p in base.glob("*.lock")) if base.is_dir() else []


def subagent_listings(world, parent):
    base = world.agy_data / "brain" / parent / ".system_generated" / "subagents"
    return sorted(p.stem for p in base.glob("*.json")) if base.is_dir() else []


def request_for(world, label):
    """The first main request that consumed scenario step ``label``, or None."""
    for request in observe.main_requests(world):
        if request.get("step") == label:
            return request
    return None


def await_step(world, label, timeout=30):
    return observe.wait_until(lambda: request_for(world, label), timeout)


def await_binding(world, pred, timeout=20):
    return observe.wait_until(lambda: (lambda b: b if b is not None and pred(b) else None)(live_binding(world)),
                              timeout)


def run_turn(world, pane, text, label, timeout=30):
    """Send a prompt, wait for the scripted step ``label`` to be served, then for the agent to idle."""
    pane.prompt(text)
    request = await_step(world, label, timeout)
    settled = pane.wait("idle", timeout)
    return request, settled


def startup_count(request):
    return sum(m.count(STARTUP_MARK) for m in request.get("system_messages", []))


def has_startup(request):
    return startup_count(request) > 0


def has_child_restriction(request):
    return any(CHILD_MARK in m for m in request.get("system_messages", []))


def system_instruction_text(world, request):
    """The request body's system instruction as JSON text ('' when the body was not saved)."""
    rel = request.get("body")
    if not rel:
        return ""
    try:
        body = json.loads((Path(world.ctl) / rel).read_text())
    except (OSError, ValueError):
        return ""
    instruction = body.get("systemInstruction") or body.get("system_instruction") or {}
    return json.dumps(instruction)


def finish_checks(world, scenario):
    """Every step consumed, nothing unmatched, no template error (spec D7 closing rule)."""
    clean, detail = observe.backend_clean(world)
    world.check("no unmatched or template-error request", clean, detail)
    used = {r.get("step") for r in observe.main_requests(world)}
    missing = [s["label"] for s in scenario["steps"] if s["label"] not in used]
    world.check("every scenario step was consumed", not missing, "unconsumed steps: %s" % missing)
    world.artifact("requests.json", world.backend.requests())
