"""``setup``: ``setup agy``, ``setup-status``, plugin disable and enable, re-setup (test-infra spec D7).

Owned files exist; ``setup-status`` reports the plugin installed, current and enabled and the allow
entry ``owned``. ``agy plugin disable herdr-threads`` turns ``enabled`` false and doctor reports
``agy_plugin_disabled``; ``enable`` reverses it; a second ``setup agy`` changes nothing.
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

SCENARIO = GATE_DIR / "scenarios" / "setup.json"


def doctor_text(world):
    result = world.ht(["--json", "doctor"], check=False)
    return (result.stdout or "") + (result.stderr or "")


def run(world):
    scenario = _l4.load_scenario(SCENARIO)
    world.backend.load(scenario)
    rc, data = _l4.setup_json(world)
    world.check("setup agy succeeds", rc == 0, "rc=%s data=%s" % (rc, str(data)[:200]))
    files = _l4.plugin_files(world)
    world.check("the owned plugin files exist", sorted(files) == sorted(_l4.PLUGIN_FILES),
                "found=%s" % sorted(files))
    world.check("settings.json holds the allow rule", _l4.ALLOW_RULE in _l4.allow_members(world),
                "allow=%s" % _l4.allow_members(world))
    status = _l4.setup_status(world)
    plugin = status.get("plugin", {})
    world.check("setup-status: plugin installed", plugin.get("installed") is True, plugin)
    world.check("setup-status: plugin current", plugin.get("current") is True, plugin)
    world.check("setup-status: plugin enabled", plugin.get("enabled") is True, plugin)
    world.check("setup-status: allow entry owned", _l4.allow_state(status) == "owned", status.get("allow_entry"))

    disabled = _l4.agy_cli(world, "plugin", "disable", _l4.PLUGIN_NAME)
    world.check("agy plugin disable succeeds", disabled.returncode == 0, disabled.stdout.strip()[:200])
    status = _l4.setup_status(world)
    world.check("setup-status: enabled is false after disable",
                status.get("plugin", {}).get("enabled") is False, status.get("plugin"))
    world.check("doctor reports agy_plugin_disabled", "agy_plugin_disabled" in doctor_text(world))

    enabled = _l4.agy_cli(world, "plugin", "enable", _l4.PLUGIN_NAME)
    world.check("agy plugin enable succeeds", enabled.returncode == 0, enabled.stdout.strip()[:200])
    status = _l4.setup_status(world)
    world.check("setup-status: enabled is true after enable",
                status.get("plugin", {}).get("enabled") is True, status.get("plugin"))

    before = _l4.plugin_files(world)
    settings_before = _l4.read_settings(world)
    rc, again = _l4.setup_json(world)
    world.check("re-setup succeeds", rc == 0, "rc=%s" % rc)
    world.check("re-setup reports already_installed", _l4.find(again, "action") == "already_installed",
                "action=%s" % _l4.find(again, "action"))
    world.check("re-setup leaves the plugin files unchanged", _l4.plugin_files(world) == before)
    world.check("re-setup leaves settings.json unchanged", _l4.read_settings(world) == settings_before)
    _l4.finish_checks(world, scenario)


FLOW = Flow(name="setup", requires=("ht-poh.4.4", "ht-poh.4.7"), run=run)
