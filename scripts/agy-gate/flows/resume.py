"""``resume``: A, exit, fresh B, exit, ``--conversation A``, exit, ``-c`` (test-infra spec D7).

Everything runs in one pane so the seat is the same. Bindings are session-keyed (spec §1): each native
session owns one row on the seat. After ``--conversation A``: same seat, A's own row (same ``binding_id``) is
connected again and B's row is not, no new seat. ``-c`` resumes the most recent conversation, which is A only
because A was the most recent at that point: no new row, A's binding unchanged. Neither resume start fires
``SessionStart``, so no request carries a second SessionStart-injected startup check-in. The Resume
lifecycle's own notice ("this conversation is the pane's top-level session") restates the startup
text by design (PRE_INVOCATION_LIFECYCLE_PREFIX) and is not a SessionStart injection.
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
SESSION_START_PREFIX = "The following system message was injected by a SessionStart hook"


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

    def connected(session):
        row = _l5.binding_for(world, session)
        return row if row is not None and row["liveness"] == "connected" and row["ended_at"] is None else None

    bind_a2 = observe.wait_until(lambda: connected(conv_a), 20)
    world.check("--conversation A reconnects A's own binding on the same seat",
                bind_a2 is not None and bind_a2["seat_id"] == seat and bind_a is not None
                and bind_a2["binding_id"] == bind_a["binding_id"],
                "seat=%s A=%s A-row=%s bindings=%s" % (seat, conv_a, bind_a2, _l5.all_bindings(world)))
    left = observe.wait_until(lambda: (lambda r: r is not None and r["liveness"] != "connected")(
        _l5.binding_for(world, bind_b["native_session"] if bind_b else None)), 20)
    world.check("the conversation just left (B) is no longer connected", bool(left),
                "B-row=%s" % _l5.binding_for(world, bind_b["native_session"] if bind_b else None))
    world.check("--conversation A creates no new seat", _l5.seat_count(world) == seats,
                "before=%s after=%s" % (seats, _l5.seat_count(world)))
    rows = len(_l5.all_bindings(world))
    pane.exit()

    pane = world.start_agy(args=["-c"], pane=pane.pane_id)
    fourth, _ = _l5.run_turn(world, pane, "ALPHA-THREE hello", "a3")
    world.check("-c's step was served", fourth is not None)
    bind_c = observe.wait_until(lambda: connected(conv_a), 20)
    world.check("-c resumes A without a new binding: seat, binding and native session unchanged",
                bind_a2 is not None and bind_c is not None
                and (bind_c["seat_id"], bind_c["binding_id"], bind_c["native_session"])
                == (bind_a2["seat_id"], bind_a2["binding_id"], conv_a)
                and len(_l5.all_bindings(world)) == rows,
                "before=%s after=%s rows=%s/%s" % (bind_a2, bind_c, rows, len(_l5.all_bindings(world))))
    world.check("-c creates no new seat", _l5.seat_count(world) == seats,
                "before=%s after=%s" % (seats, _l5.seat_count(world)))
    world.artifact("bindings.json", _l5.all_bindings(world))
    pane.exit()

    mains = observe.main_requests(world)
    doubled = [r.get("seq") for r in mains
               if sum(1 for m in r.get("system_messages", [])
                      if _l5.STARTUP_MARK in m and m.startswith(SESSION_START_PREFIX)) > 1]
    world.check("no request carries a second startup check-in", not doubled, "requests=%s" % doubled)
    _l5.finish_checks(world, scenario)


FLOW = Flow(name="resume", requires=("ht-poh.4.3",), run=run)
