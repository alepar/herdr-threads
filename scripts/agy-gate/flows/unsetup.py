"""``unsetup``: a clean world and a pre-seeded member (test-infra spec D7).

Owned files and the owned allow member are removed, other settings keys and other allow members
stay, a member the user already had stays, and no seat is enrolled afterwards.
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

SCENARIO = GATE_DIR / "scenarios" / "unsetup.json"
SEED_KEYS = ("altScreenMode", "modelProvider", "trustedWorkspaces")


def run(world):
    scenario = _l4.load_scenario(SCENARIO)
    world.backend.load(scenario)

    # Clean world: the member is ours, so unsetup removes it.
    seed = _l4.read_settings(world) or {}
    seed["permissions"] = {"allow": ["command(ls)"]}
    _l4.write_settings(world, seed)
    world.check("setup agy succeeds (clean)", world.setup_agy().returncode == 0)
    status = _l4.setup_status(world)
    world.check("allow entry is owned after setup", _l4.allow_state(status) == "owned", status.get("allow_entry"))
    result = world.unsetup_agy()
    world.check("unsetup agy succeeds", result.returncode == 0, (result.stderr or result.stdout).strip()[:200])
    world.check("owned plugin files are gone", not _l4.plugin_files(world),
                "left=%s" % sorted(_l4.plugin_files(world)))
    world.check("the owned allow member is removed", _l4.ALLOW_RULE not in _l4.allow_members(world),
                "allow=%s" % _l4.allow_members(world))
    world.check("another allow member is kept", "command(ls)" in _l4.allow_members(world),
                "allow=%s" % _l4.allow_members(world))
    doc = _l4.read_settings(world) or {}
    world.check("the other settings keys are kept", all(k in doc for k in SEED_KEYS), "keys=%s" % sorted(doc))

    # Pre-seeded world: the user already had the member, so unsetup leaves it.
    seed = dict(doc)
    seed["permissions"] = {"allow": [_l4.ALLOW_RULE, "command(ls)"]}
    _l4.write_settings(world, seed)
    world.check("setup agy succeeds (pre-seeded)", world.setup_agy().returncode == 0)
    status = _l4.setup_status(world)
    world.check("allow entry is pre_existing", _l4.allow_state(status) == "pre_existing", status.get("allow_entry"))
    result = world.unsetup_agy()
    world.check("unsetup agy succeeds (pre-seeded)", result.returncode == 0,
                (result.stderr or result.stdout).strip()[:200])
    world.check("the pre-existing allow member is kept", _l4.ALLOW_RULE in _l4.allow_members(world),
                "allow=%s" % _l4.allow_members(world))
    world.check("the other allow member is kept too", "command(ls)" in _l4.allow_members(world))
    world.check("owned plugin files are gone (pre-seeded)", not _l4.plugin_files(world))
    doc = _l4.read_settings(world) or {}
    world.check("the other settings keys are kept (pre-seeded)", all(k in doc for k in SEED_KEYS),
                "keys=%s" % sorted(doc))
    seats = _l5.seat_count(world)
    world.check("no seat is enrolled", not seats, "seats=%s" % seats)
    _l4.finish_checks(world, scenario)


FLOW = Flow(name="unsetup", requires=("ht-poh.4.4",), run=run)
