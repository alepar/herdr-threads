"""``permission``: the owned allow rule lets ``herdr-threads`` run unprompted, nothing wider (test-infra spec D7).

The scenario calls ``run_command "herdr-threads --version"``: the agent never reaches ``blocked`` and
the next request carries a ``functionResponse`` holding the version. Then it calls
``"herdr-threads --version; true"``: the compound command is outside the rule, the agent goes
``blocked``, and the gate denies it with keys (the screen is kept as an artifact).
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
from gate import Flow  # noqa: E402

SCENARIO = GATE_DIR / "scenarios" / "permission.json"
DENY_KEYS = ("Escape",)


def run(world):
    scenario = _l4.load_scenario(SCENARIO)
    world.backend.load(scenario)
    world.check("setup agy succeeds", world.setup_agy().returncode == 0)
    pane = world.start_agy()

    pane.prompt("PERM-ALLOWED run the version command")
    call = _l5.await_step(world, "allowed-call", 30)
    world.check("the allowed command call was served", call is not None)
    blocked = pane.wait("blocked", 5)
    world.check("an allowed command never blocks the agent", not blocked, "screen=%s" % pane.read_pane(40)[-300:])
    final = _l5.await_step(world, "allowed-final", 30)
    world.check("the request after the allowed command was served", final is not None)
    responses = (final or {}).get("function_responses") or []
    world.check("that request carries a functionResponse for run_command",
                any(r.get("name") == "run_command" for r in responses), "responses=%s" % responses)
    version = _l4.version_token(world)
    world.check("the functionResponse holds the herdr-threads version",
                bool(version) and version in _l4.body_text(world, final), "version=%r" % version)
    world.check("agy returned to idle", pane.wait("idle", 30))

    pane.prompt("PERM-DENIED run the compound command")
    call = _l5.await_step(world, "denied-call", 30)
    world.check("the compound command call was served", call is not None)
    world.check("the compound command blocks the agent", pane.wait("blocked", 30),
                "status=%s" % pane.agent_status())
    world.artifact("permission-blocked-screen.txt", pane.read_pane())
    pane.keys(*DENY_KEYS)
    world.check("the agent leaves blocked after the gate denies", pane.wait("idle", 30),
                "status=%s" % pane.agent_status())
    pane.exit()
    _l4.finish_checks(world, scenario, optional=("denied-final",))


FLOW = Flow(name="permission", requires=("ht-poh.4.4",), run=run)
