"""``enroll``: setup, start agy, prompt (test-infra spec D7).

The seat's binding is harness ``agy``, native session equal to the conversation id (the only
``presence/*.lock``), provenance ``cooperative_top_level``. The first main request's
``system_messages`` holds the startup check-in and its system instruction lists the
``herdr-threads`` skill path.
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

SCENARIO = GATE_DIR / "scenarios" / "enroll.json"


def run(world):
    scenario = _l5.load_scenario(SCENARIO)
    world.backend.load(scenario)
    setup = world.setup_agy()
    world.check("setup agy succeeds", setup.returncode == 0, (setup.stderr or setup.stdout).strip()[:200])
    pane = world.start_agy()
    request, settled = _l5.run_turn(world, pane, "ENROLL-PROMPT hello", "enroll")
    world.check("the scripted enroll step was served", request is not None)
    world.check("agy returned to idle", settled)
    binding = _l5.await_binding(world, lambda b: True)
    world.check("exactly one live agy binding exists", binding is not None,
                "bindings=%s" % _l5.live_bindings(world))
    locks = _l5.presence_ids(world)
    if binding is not None:
        world.check("binding harness is agy", binding["harness"] == "agy", binding["harness"])
        world.check("binding native session is the only presence lock",
                    locks == [binding["native_session"]],
                    "locks=%s native_session=%s" % (locks, binding["native_session"]))
        world.check("binding provenance is cooperative_top_level",
                    binding["observation_provenance"] == "cooperative_top_level",
                    binding["observation_provenance"])
    world.check("one seat exists", _l5.seat_count(world) == 1, "seats=%s" % _l5.seat_count(world))
    mains = observe.main_requests(world)
    first = mains[0] if mains else None
    world.check("the first main request carries the startup check-in",
                first is not None and _l5.has_startup(first),
                "system_messages=%s" % (first or {}).get("system_messages"))
    instruction = _l5.system_instruction_text(world, first) if first else ""
    world.check("the system instruction lists the herdr-threads skill path",
                _l5.SKILL_PATH in instruction, "instruction bytes=%d" % len(instruction))
    pane.exit()
    _l5.finish_checks(world, scenario)


FLOW = Flow(name="enroll", requires=("ht-poh.4.2", "ht-poh.4.3", "ht-poh.4.4"), run=run)
