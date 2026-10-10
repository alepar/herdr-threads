"""``subagent``: the parent invokes a subagent (test-infra spec D7, research B shape).

The child's conversation is listed under the parent's ``subagents/`` and has no presence lock;
the child's request carries the subagent restriction ``systemMessage``; the binding generation and
native session are unchanged and no new seat exists. Child and parent steps are routed by match.
"""
from __future__ import annotations

import sys
from pathlib import Path

GATE_DIR = Path(__file__).resolve().parent.parent
for _dir in (GATE_DIR, GATE_DIR / "flows", GATE_DIR.parent / "agy-sandbox"):
    if str(_dir) not in sys.path:
        sys.path.insert(0, str(_dir))

import _l5  # noqa: E402
import observe  # noqa: E402
from gate import Flow  # noqa: E402

SCENARIO = GATE_DIR / "scenarios" / "subagent.json"


def identity(binding):
    return None if binding is None else (binding["seat_id"], binding["generation"], binding["native_session"])


def run(world):
    scenario = _l5.load_scenario(SCENARIO)
    world.backend.load(scenario)
    setup = world.setup_agy()
    world.check("setup agy succeeds", setup.returncode == 0, (setup.stderr or setup.stdout).strip()[:200])
    pane = world.start_agy()
    before = _l5.await_binding(world, lambda b: True)
    world.check("the parent is enrolled before the prompt", before is not None,
                "bindings=%s" % _l5.live_bindings(world))
    seats_before = _l5.seat_count(world)
    pane.prompt("PARENT-GO start")
    child_request = _l5.await_step(world, "child", 60)
    parent_final = _l5.await_step(world, "parent-final", 60)
    settled = pane.wait("idle", 60)
    world.check("the parent invoked the subagent", _l5.request_for(world, "parent-call") is not None)
    world.check("the child request was served", child_request is not None)
    world.check("the parent resumed after the child", parent_final is not None)
    world.check("agy returned to idle", settled)
    parent = before["native_session"] if before else None
    children = observe.wait_until(lambda: _l5.subagent_listings(world, parent), 15) if parent else []
    world.check("a child is listed under the parent's subagents/", bool(children),
                "children=%s" % children)
    locks = _l5.presence_ids(world)
    world.check("the parent alone holds a presence lock", locks == [parent], "locks=%s" % locks)
    world.check("no child has a presence lock", not (set(children) & set(locks)),
                "children=%s locks=%s" % (children, locks))
    world.check("the child's request carries the subagent restriction",
                child_request is not None and _l5.has_child_restriction(child_request),
                "system_messages=%s" % (child_request or {}).get("system_messages"))
    after = _l5.live_binding(world)
    world.check("binding generation and native session are unchanged", identity(after) == identity(before),
                "before=%s after=%s" % (identity(before), identity(after)))
    world.check("no new seat exists", _l5.seat_count(world) == seats_before == 1,
                "before=%s after=%s" % (seats_before, _l5.seat_count(world)))
    pane.exit()
    _l5.finish_checks(world, scenario)


FLOW = Flow(name="subagent", requires=("ht-poh.4.3",), run=run)
