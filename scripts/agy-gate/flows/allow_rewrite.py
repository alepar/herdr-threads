"""``allow-rewrite``: does agy drop the allow member when the user trusts a workspace (test-infra spec D7)?

Starts agy in an untrusted workspace ``ws2``, accepts the trust prompt, and records whether agy
rewrote ``settings.json`` without the owned member. Either outcome is a finding, written to the
``allow-rewrite-finding.json`` artifact and the check detail. When agy did drop it, ``setup-status``
must say ``missing`` and a plain ``setup agy`` (the consent-free Ask path, keeping the recorded grant)
must add it back as ``owned``.
"""
from __future__ import annotations

import sys
from pathlib import Path

GATE_DIR = Path(__file__).resolve().parent.parent
for _dir in (GATE_DIR, GATE_DIR / "flows", GATE_DIR.parent / "agy-sandbox"):
    if str(_dir) not in sys.path:
        sys.path.insert(0, str(_dir))

import _l4  # noqa: E402
from gate import Flow  # noqa: E402

SCENARIO = GATE_DIR / "scenarios" / "allow-rewrite.json"
TRUST_WORDS = ("trust", "do you want")


def run(world):
    scenario = _l4.load_scenario(SCENARIO)
    world.backend.load(scenario)
    world.check("setup agy succeeds", world.setup_agy().returncode == 0)
    world.check("allow entry is owned before agy runs",
                _l4.allow_state(_l4.setup_status(world)) == "owned")
    ws2 = world.root / "ws2"
    ws2.mkdir(exist_ok=True)
    trusted_before = (_l4.read_settings(world) or {}).get("trustedWorkspaces")
    world.check("ws2 is not trusted yet", str(ws2) not in (trusted_before or []), "trusted=%s" % trusted_before)

    shell = _l4.shell_pane(world, "agy-ws2", cwd=ws2)
    pane = _l4.adopt_agy_pane(world, shell)
    world.start_agy(pane=shell)
    screen = pane.read_pane()
    prompted = any(word in screen.lower() for word in TRUST_WORDS)
    world.artifact("trust-prompt-screen.txt", screen)
    if prompted:
        pane.keys("Enter")
    pane.wait("idle", 30)
    pane.exit()

    settings = _l4.read_settings(world) or {}
    trusted_after = settings.get("trustedWorkspaces") or []
    dropped = _l4.ALLOW_RULE not in _l4.allow_members(world)
    status = _l4.setup_status(world)
    finding = {"trust_prompt_seen": prompted, "ws2_trusted_after": str(ws2) in trusted_after,
               "agy_dropped_allow_member": dropped, "allow_state": _l4.allow_state(status),
               "settings_keys": sorted(settings), "allow": _l4.allow_members(world)}
    world.artifact("allow-rewrite-finding.json", finding)
    world.check("finding recorded: agy %s the allow member on trust accept" %
                ("dropped" if dropped else "kept"), True, finding)
    world.check("ws2 is trusted after accepting the prompt", str(ws2) in trusted_after or not prompted,
                "trusted=%s prompted=%s" % (trusted_after, prompted))
    if dropped:
        world.check("setup-status reports the allow entry missing", _l4.allow_state(status) == "missing",
                    status.get("allow_entry"))
        rc, _ = _l4.setup_json(world, permissions=False)
        world.check("plain setup agy keeps the recorded grant and re-adds the member", rc == 0 and _l4.ALLOW_RULE in _l4.allow_members(world),
                    "allow=%s" % _l4.allow_members(world))
        world.check("the re-added member is owned", _l4.allow_state(_l4.setup_status(world)) == "owned")
    else:
        world.check("setup-status still reports the allow entry owned", _l4.allow_state(status) == "owned",
                    status.get("allow_entry"))
    _l4.finish_checks(world, scenario)


FLOW = Flow(name="allow-rewrite", requires=("ht-poh.4.4",), run=run)
