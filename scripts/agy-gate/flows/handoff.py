"""``handoff``: ``handoff --new-tab --kind agy`` from an enrolled agy seat (test-infra spec D7).

The parent agy runs the handoff itself through ``run_command`` (the guarded bootstrap V2 path needs an
original top-level agent). The handoff produces a plan and a new tab whose pane runs agy, and a second
seat appears. The child agy's own requests are not scripted, so unmatched requests from it are
expected and not a failure; the parent's requests must all have matched.
"""
from __future__ import annotations

import sys
from pathlib import Path

GATE_DIR = Path(__file__).resolve().parent.parent
for _dir in (GATE_DIR, GATE_DIR / "flows", GATE_DIR.parent / "agy-sandbox"):
    if str(_dir) not in sys.path:
        sys.path.insert(0, str(_dir))

import _l4  # noqa: E402
import _l5  # noqa: E402
import observe  # noqa: E402
from gate import Flow  # noqa: E402

SCENARIO = GATE_DIR / "scenarios" / "handoff.json"
CHILD_TAB = "gate-child"


def child_tab_present(world):
    data = world.herdr_json("tab", "list", "--workspace", world.workspace, check=False)
    return CHILD_TAB in str(data)


def run(world):
    scenario = _l4.load_into(world, SCENARIO, AGY=_l4.wrapper(world))
    world.check("setup agy succeeds", world.setup_agy().returncode == 0)
    pane = world.start_agy()
    # The first turn enrolls the parent; the handoff turn then runs as an enrolled top-level seat.
    pane.prompt("HANDOFF-PROMPT start the handoff")
    call = _l5.await_step(world, "handoff-call", 30)
    world.check("the handoff call was served", call is not None)
    final = _l5.await_step(world, "handoff-final", 90)
    world.check("the request after the handoff command was served", final is not None)
    responses = (final or {}).get("function_responses") or []
    world.check("that request carries a functionResponse for run_command",
                any(r.get("name") == "run_command" for r in responses), "responses=%s" % responses)
    world.artifact("handoff-response.txt", _l4.body_text(world, final))
    world.check("a tab labeled %s was created" % CHILD_TAB,
                bool(observe.wait_until(lambda: child_tab_present(world), 30)))
    both = observe.wait_until(lambda: len(_l5.live_bindings(world)) >= 2, 60)
    world.check("a second live agy seat exists", bool(both), "bindings=%s" % _l5.live_bindings(world))
    world.check("the parent run still has a live binding with cooperative_top_level provenance",
                any(b["observation_provenance"] == "cooperative_top_level" for b in _l5.live_bindings(world)),
                "bindings=%s" % _l5.live_bindings(world))
    world.check("an agy process runs for the child", world.agy_running())
    pane.wait("idle", 30)
    pane.exit()
    # The child's unscripted requests may be unmatched; only the parent's steps must all be consumed.
    used = {r.get("step") for r in observe.main_requests(world)}
    missing = [s["label"] for s in scenario["steps"] if s["label"] not in used]
    world.check("every scenario step was consumed", not missing, "unconsumed steps: %s" % missing)
    world.artifact("requests.json", world.backend.requests())


FLOW = Flow(name="handoff", requires=("ht-poh.4.6", "ht-poh.8"), run=run)
