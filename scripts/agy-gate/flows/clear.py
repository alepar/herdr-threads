"""``clear``: ``agent prompt /clear``, then a prompt (test-infra spec D7).

Same seat, binding generation +1, native session equal to the new conversation id, and the
startup check-in appears again in the post-clear conversation.
"""
from __future__ import annotations

import sys
from pathlib import Path

GATE_DIR = Path(__file__).resolve().parent.parent
for _dir in (GATE_DIR, GATE_DIR / "flows", GATE_DIR.parent / "agy-sandbox"):
    if str(_dir) not in sys.path:
        sys.path.insert(0, str(_dir))

import _l5  # noqa: E402
from gate import Flow  # noqa: E402

SCENARIO = GATE_DIR / "scenarios" / "clear.json"


def run(world):
    scenario = _l5.load_scenario(SCENARIO)
    world.backend.load(scenario)
    setup = world.setup_agy()
    world.check("setup agy succeeds", setup.returncode == 0, (setup.stderr or setup.stdout).strip()[:200])
    pane = world.start_agy()
    pre, _ = _l5.run_turn(world, pane, "PRE-CLEAR-PROMPT hello", "pre-clear")
    world.check("the pre-clear step was served", pre is not None)
    before = _l5.await_binding(world, lambda b: True)
    world.check("the seat is enrolled before /clear", before is not None, "bindings=%s" % _l5.live_bindings(world))
    pre_startup = pre is not None and _l5.has_startup(pre)
    world.check("the pre-clear request carries the startup check-in", pre_startup)
    pane.prompt("/clear")
    pane.wait("idle", 30)
    post, settled = _l5.run_turn(world, pane, "POST-CLEAR-PROMPT hello", "post-clear")
    world.check("the post-clear step was served", post is not None)
    world.check("agy returned to idle", settled)
    if before is not None:
        after = _l5.await_binding(world, lambda b: b["generation"] != before["generation"])
        world.check("same seat after /clear", after is not None and after["seat_id"] == before["seat_id"],
                    "before=%s after=%s" % (before, after))
        world.check("binding generation advanced by one",
                    after is not None and after["generation"] == before["generation"] + 1,
                    "before=%s after=%s" % (before["generation"], (after or {}).get("generation")))
        new_ids = [i for i in _l5.presence_ids(world) if i != before["native_session"]]
        world.check("native session is the new conversation id",
                    after is not None and after["native_session"] != before["native_session"]
                    and after["native_session"] in new_ids,
                    "after=%s new_locks=%s" % ((after or {}).get("native_session"), new_ids))
    world.check("no new seat exists", _l5.seat_count(world) == 1, "seats=%s" % _l5.seat_count(world))
    world.check("the startup check-in appears again after /clear", post is not None and _l5.has_startup(post),
                "system_messages=%s" % (post or {}).get("system_messages"))
    pane.exit()
    _l5.finish_checks(world, scenario)


FLOW = Flow(name="clear", requires=("ht-poh.4.3",), run=run)
