"""Helpers shared by the L4 flows (setup, permission, allow-rewrite, unsetup, launch, handoff); not a flow.

Flow-local CLI and file queries over the isolated world (test-infra spec D6). The leading underscore
keeps ``gate.discover`` from loading this module as a flow. Builds on ``_l5`` (database and request-log
accessors) rather than repeating them.
"""
from __future__ import annotations

import json
import shlex
import subprocess
import sys
from pathlib import Path

GATE_DIR = Path(__file__).resolve().parent.parent
for _dir in (GATE_DIR, GATE_DIR / "flows", GATE_DIR.parent / "agy-sandbox"):
    if str(_dir) not in sys.path:
        sys.path.insert(0, str(_dir))

import _l5  # noqa: E402
import observe  # noqa: E402
from agyenv import AgyPane  # noqa: E402

PLUGIN_NAME = "herdr-threads"
ALLOW_RULE = "command(herdr-threads)"
PLUGIN_FILES = ("plugin.json", "hooks.json", "skills/herdr-threads/SKILL.md")
LAUNCH_REFUSAL = "agy launch refuses"

load_scenario = _l5.load_scenario


def find(data, key):
    """First value stored under ``key`` in nested JSON, depth first (None when absent)."""
    if isinstance(data, dict):
        if key in data:
            return data[key]
        for value in data.values():
            hit = find(value, key)
            if hit is not None:
                return hit
    elif isinstance(data, list):
        for value in data:
            hit = find(value, key)
            if hit is not None:
                return hit
    return None


def plugin_dir(world):
    return world.home / ".gemini" / "config" / "plugins" / PLUGIN_NAME


def settings_path(world):
    return world.agy_data / "settings.json"


def read_settings(world):
    try:
        return json.loads(settings_path(world).read_text())
    except (OSError, ValueError):
        return None


def write_settings(world, doc):
    settings_path(world).write_text(json.dumps(doc, separators=(",", ":")))


def allow_members(world):
    doc = read_settings(world)
    allow = ((doc or {}).get("permissions") or {}).get("allow")
    return list(allow) if isinstance(allow, list) else []


def plugin_files(world):
    """``{relative path: bytes}`` of the owned plugin files that exist."""
    base = plugin_dir(world)
    return {rel: (base / rel).read_bytes() for rel in PLUGIN_FILES if (base / rel).is_file()}


def setup_status(world):
    """``setup-status agy`` as a dict ``{plugin, allow_entry, ...}`` (``{}`` when unreadable)."""
    data = observe.cli_json(world, ["setup-status", "agy"])
    holder = find(data, "plugin")
    if not isinstance(holder, dict):
        return {}
    allow = find(data, "allow_entry")
    return {"plugin": holder, "allow_entry": allow if isinstance(allow, dict) else {}, "raw": data}


def allow_state(status):
    return (status.get("allow_entry") or {}).get("state")


def setup_json(world, permissions=True):
    """``setup agy`` with ``--json``: ``(returncode, data)``; ``permissions`` adds ``--with-permissions``."""
    argv = ["--json", "setup", "agy"] + (["--with-permissions"] if permissions else [])
    result = world.ht(argv, check=False, human=True)  # the operator route: an agent call must start with `setup`
    try:
        doc = json.loads(result.stdout)
    except ValueError:
        return result.returncode, {}
    data = doc.get("result", {}).get("data", doc) if isinstance(doc, dict) else {}
    return result.returncode, data


def load_into(world, path, **extra_vars):
    """Load a scenario into the world's fake server with ``cwd`` bound to the world's ``ws``
    (agy's ``run_command`` requires ``Cwd``) plus ``extra_vars``; returns the scenario."""
    scenario = load_scenario(path)
    scenario["vars"] = dict(scenario.get("vars", {}), cwd=str(world.ws), **extra_vars)
    world.backend.load(scenario)
    return scenario


def agy_cli(world, *args, timeout=30):
    """Run the genuine agy (through the sandbox wrapper) with a subcommand, in the isolated HOME."""
    return subprocess.run([str(world.root / "bin" / "agy")] + [str(a) for a in args], env=world.herdr_env(),
                          stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                          universal_newlines=True, timeout=timeout * world.timeout_scale)


def wrapper(world):
    return str(world.root / "bin" / "agy")


def shell_pane(world, label, cwd=None):
    """A new tab whose root pane is a plain shell (optionally ``cd``-ed), without focusing it."""
    pane = world._new_tab(label)
    if cwd is not None:
        world.herdr("pane", "run", pane, "cd %s" % shlex.quote(str(cwd)))
    return pane


def adopt_agy_pane(world, pane):
    """Register a pane agy was started in by something else, so teardown exits it."""
    agy_pane = AgyPane(world, pane)
    world.panes.append(agy_pane)
    return agy_pane


def body_text(world, request):
    """The saved request body as text ('' when it was not saved)."""
    rel = (request or {}).get("body")
    if not rel:
        return ""
    try:
        return (Path(world.ctl) / rel).read_text()
    except OSError:
        return ""


def _strings(node):
    if isinstance(node, str):
        yield node
    elif isinstance(node, dict):
        for value in node.values():
            yield from _strings(value)
    elif isinstance(node, list):
        for value in node:
            yield from _strings(value)


def _responses(node):
    if isinstance(node, dict):
        if isinstance(node.get("functionResponse"), dict):
            yield node["functionResponse"]
        for value in node.values():
            yield from _responses(value)
    elif isinstance(node, list):
        for value in node:
            yield from _responses(value)


def last_functionresponse(world, request):
    """The newest functionResponse of a saved request body, its string values joined by newlines.

    Each follow-up request repeats the earlier responses; the last one is this step's command output.
    """
    try:
        found = list(_responses(json.loads(body_text(world, request))))
    except ValueError:
        return ""
    return "\n".join(_strings(found[-1].get("response"))) if found else ""


def tool_declaration(world, request, name):
    """The ``functionDeclarations`` entry called ``name`` in a saved request body (None when absent)."""
    try:
        body = json.loads(body_text(world, request))
    except ValueError:
        return None
    found = None

    def walk(node):
        nonlocal found
        if isinstance(node, dict):
            if node.get("name") == name and ("parameters" in node or "parametersJsonSchema" in node):
                found = found or node
            for value in node.values():
                walk(value)
        elif isinstance(node, list):
            for value in node:
                walk(value)

    walk(body)
    return found


def version_token(world):
    """The bare version number printed by ``herdr-threads --version``."""
    out = world.ht(["--version"]).stdout.strip()
    return out.split()[-1] if out else ""


def finish_checks(world, scenario, optional=()):
    """Like ``_l5.finish_checks`` but a step named in ``optional`` need not be consumed."""
    clean, detail = observe.backend_clean(world)
    world.check("no unmatched or template-error request", clean, detail)
    used = {r.get("step") for r in observe.main_requests(world)}
    missing = [s["label"] for s in scenario["steps"] if s["label"] not in used and s["label"] not in optional]
    world.check("every scenario step was consumed", not missing, "unconsumed steps: %s" % missing)
    world.artifact("requests.json", world.backend.requests())
