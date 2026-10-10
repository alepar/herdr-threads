"""``launch``: ``herdr-threads launch --kind agy`` and its refusals (test-infra spec D7).

A launch from a sender-style shell pane into a fresh pane starts agy, and the first prompt enrolls it
(provenance ``cooperative_top_level``). Each refused argument vector (``--sandbox``, ``-p x``,
``--dangerously-skip-permissions``) exits non-zero with the policy text and starts no agy process.
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

SCENARIO = GATE_DIR / "scenarios" / "launch.json"
REFUSED = (["--sandbox"], ["-p", "x"], ["--dangerously-skip-permissions"])


def launch_argv(pane, wrapper, native=()):
    argv = ["launch", "--kind", "agy", "--pane", pane, "--harness-binary", wrapper]
    return argv + (["--"] + list(native) if native else [])


def run(world):
    scenario = _l4.load_scenario(SCENARIO)
    world.backend.load(scenario)
    world.check("setup agy succeeds", world.setup_agy().returncode == 0)
    sender = world.sender()
    wrapper = _l4.wrapper(world)

    for native in REFUSED:
        pane = _l4.shell_pane(world, "refused-%s" % native[0].strip("-"))
        result = sender.ht(launch_argv(pane, wrapper, native), check=False)
        text = (result.stderr or "") + (result.stdout or "")
        label = " ".join(native)
        world.check("launch with %s exits non-zero" % label, result.returncode != 0, "rc=%s" % result.returncode)
        world.check("launch with %s prints the policy text" % label, _l4.LAUNCH_REFUSAL in text, text.strip()[:200])
        world.check("launch with %s starts no agy process" % label, not world.agy_running())

    target = _l4.shell_pane(world, "launched")
    result = sender.ht(launch_argv(target, wrapper), check=False)
    world.check("launch --kind agy succeeds", result.returncode == 0, (result.stderr or result.stdout).strip()[:300])
    pane = _l4.adopt_agy_pane(world, target)
    world.check("the launched pane reaches idle", pane.wait("idle", 60), "status=%s" % pane.agent_status())
    world.check("herdr detects the agent as agy", pane.agent_name() == "agy", "agent=%s" % pane.agent_name())
    request, settled = _l5.run_turn(world, pane, "LAUNCH-PROMPT hello", "launch")
    world.check("the scripted launch step was served", request is not None)
    world.check("agy returned to idle", settled)
    binding = _l5.await_binding(world, lambda b: b["observation_provenance"] == "cooperative_top_level")
    world.check("the first prompt enrolls the launched seat", binding is not None,
                "bindings=%s" % _l5.live_bindings(world))
    if binding is not None:
        locks = _l5.presence_ids(world)
        world.check("binding native session is the presence lock", locks == [binding["native_session"]],
                    "locks=%s native=%s" % (locks, binding["native_session"]))
    world.check("the first request carries the startup check-in", request is not None and _l5.has_startup(request),
                "system_messages=%s" % (request or {}).get("system_messages"))
    pane.exit()
    _l4.finish_checks(world, scenario)


FLOW = Flow(name="launch", requires=("ht-poh.4.6",), run=run)
