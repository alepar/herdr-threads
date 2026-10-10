"""``resume``: A, exit, fresh B, exit, ``--conversation A``, exit, ``-c`` (test-infra spec D7).

Everything runs in one pane so the seat is the same. After ``--conversation A``: same seat, binding
native session A, no new seat. ``-c`` resumes the most recent conversation, which is A only because
A was the most recent at that point: no rotation, generation unchanged. Neither resume start fires
``SessionStart``, so no request carries a second startup check-in.
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

SCENARIO = GATE_DIR / "scenarios" / "resume.json"


def run(world):
    scenario = _l5.load_scenario(SCENARIO)
    world.backend.load(scenario)
    setup = world.setup_agy()
    world.check("setup agy succeeds", setup.returncode == 0, (setup.stderr or setup.stdout).strip()[:200])

    pane = world.start_agy()
    first, _ = _l5.run_turn(world, pane, "ALPHA-ONE hello", "a1")
    world.check("conversation A's first step was served", first is not None)
    bind_a = _l5.await_binding(world, lambda b: True)
    world.check("conversation A is enrolled", bind_a is not None, "bindings=%s" % _l5.live_bindings(world))
    conv_a = bind_a["native_session"] if bind_a else None
    pane.exit()

    pane = world.start_agy(pane=pane.pane_id)
    second, _ = _l5.run_turn(world, pane, "BRAVO-ONE hello", "b1")
    world.check("conversation B's step was served", second is not None)
    bind_b = _l5.await_binding(world, lambda b: b["native_session"] != conv_a)
    world.check("the fresh conversation B is a different native session", bind_b is not None,
                "A=%s binding=%s" % (conv_a, _l5.live_binding(world)))
    pane.exit()
    seat = bind_a["seat_id"] if bind_a else None
    seats = _l5.seat_count(world)

    pane = world.start_agy(args=["--conversation", conv_a or ""], pane=pane.pane_id)
    third, _ = _l5.run_turn(world, pane, "ALPHA-TWO hello", "a2")
    world.check("--conversation A's step was served", third is not None)
    bind_a2 = _l5.await_binding(world, lambda b: b["native_session"] == conv_a)
    world.check("--conversation A rebinds the same seat to native session A",
                bind_a2 is not None and bind_a2["seat_id"] == seat,
                "seat=%s A=%s binding=%s" % (seat, conv_a, _l5.live_binding(world)))
    world.check("--conversation A creates no new seat", _l5.seat_count(world) == seats,
                "before=%s after=%s" % (seats, _l5.seat_count(world)))
    pane.exit()

    pane = world.start_agy(args=["-c"], pane=pane.pane_id)
    fourth, _ = _l5.run_turn(world, pane, "ALPHA-THREE hello", "a3")
    world.check("-c's step was served", fourth is not None)
    bind_c = _l5.live_binding(world)
    world.check("-c resumes A without rotation: seat, generation and native session unchanged",
                bind_a2 is not None and bind_c is not None
                and (bind_c["seat_id"], bind_c["generation"], bind_c["native_session"])
                == (bind_a2["seat_id"], bind_a2["generation"], conv_a),
                "before=%s after=%s" % (bind_a2, bind_c))
    world.check("-c creates no new seat", _l5.seat_count(world) == seats,
                "before=%s after=%s" % (seats, _l5.seat_count(world)))
    pane.exit()

    mains = observe.main_requests(world)
    doubled = [r.get("seq") for r in mains if _l5.startup_count(r) > 1]
    world.check("no request carries a second startup check-in", not doubled, "requests=%s" % doubled)
    _l5.finish_checks(world, scenario)


FLOW = Flow(name="resume", requires=("ht-poh.4.3",), run=run)
