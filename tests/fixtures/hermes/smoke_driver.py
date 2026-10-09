"""Configuration-smoke driver: loads the INSTALLED Hermes bridge with a stub plugin
context and replays a scripted callback sequence (ht-4ne.3).

usage: python3 -I smoke_driver.py <installed __init__.py> <native source root>
stdin : {"steps": [{"callback": str, "kwargs": {...}, "repeat_if_slow": bool}, ...]}
stdout: first a {"bridge": {"child_restriction": ..., "hooks": [...]}} line (the
        installed module's own constant), then one JSON line per step
        {"callback", "result", "elapsed_ms", "child": [<the Rust hook's raw JSON answers
        the bridge received during this step, e.g. {"context", "lifecycle_ack"}>]}.
        The bridge itself returns only {"context"}; the spy on Bridge.child exposes the
        lifecycle_ack the bridge validated, without changing what the bridge does.

Stub native modules follow integrations/hermes/test_bridge.py (BridgeTests.setUp).
The home and profile are derived from the installed path
<home>/plugins/herdr-threads/__init__.py, so the bridge's own profile/home checks hold.
"""
import importlib.util
import json
import os
import sys
import time
import types
from pathlib import Path

VERSION = dict(base_version="0.21.5", derived_version="0.21.5+1.g1234567",
               commit="1234567890abcdef1234567890abcdef12345678",
               dirty=False, source="git", distance=1)
SLOT = "_herdr_threads_hermes_reader_schema1"


def main():
    installed = Path(sys.argv[1]).resolve()
    native = Path(sys.argv[2]).resolve()
    home = installed.parent.parent.parent
    profile = home.name if home.parent.name == "profiles" else "default"
    script = json.load(sys.stdin)

    modules = {}
    for name in ("hermes_bootstrap", "hermes_constants", "hermes_cli", "hermes_cli.config",
                 "hermes_cli.profiles", "hermes_cli.version_info", "hermes_cli.plugins"):
        relative = Path(*name.split(".")).with_suffix(".py")
        if name == "hermes_cli":
            relative = Path("hermes_cli/__init__.py")
        path = native / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        if not path.exists():
            path.write_text("# synthetic already initialized native module\n")
        module = types.ModuleType(name)
        module.__file__ = str(path)
        module.__spec__ = importlib.util.spec_from_file_location(name, str(path))
        modules[name] = module

    class FailedConfigRead(dict):
        pass

    modules["hermes_cli.config"].FailedConfigRead = FailedConfigRead
    modules["hermes_cli.config"].load_config_readonly = lambda: {
        "plugins": {"hook_callback_timeout": 30, "enabled": ["herdr-threads"]}}
    modules["hermes_cli.version_info"].get_version_info = lambda: types.SimpleNamespace(**VERSION)
    modules["hermes_constants"].get_hermes_home = lambda: home
    modules["hermes_cli.profiles"].current_profile_name = lambda default=None: profile
    modules["hermes_cli.profiles"].resolve_profile_env = lambda name: str(home)

    class PluginContext:
        __module__ = "hermes_cli.plugins"

        def __init__(self):
            self.hooks = {}
            self.unloads = []

        def register_hook(self, name, callback):
            self.hooks[name] = callback

        def on_unload(self, callback):
            self.unloads.append(callback)

    modules["hermes_cli.plugins"].PluginContext = PluginContext
    sys.modules.update(modules)

    spec = importlib.util.spec_from_file_location("herdr_threads_installed_bridge", str(installed))
    bridge = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(bridge)
    seen = []
    original_child = bridge.Bridge.child

    def spying_child(self, *args, **kwargs):
        output = original_child(self, *args, **kwargs)
        try:
            seen.append(json.loads(output))
        except ValueError:
            seen.append(None)
        return output

    bridge.Bridge.child = spying_child
    ctx = PluginContext()
    bridge.register(ctx)
    try:
        slot = sys.modules.get(SLOT)
        reader = getattr(slot, "reader", None)
        limit = time.monotonic() + 5
        while reader is not None and reader.snapshot() is None and time.monotonic() < limit:
            time.sleep(0.005)
        print(json.dumps({"bridge": {"child_restriction": bridge.CHILD_RESTRICTION,
                                     "hooks": sorted(ctx.hooks)}}), flush=True)
        for step in script["steps"]:
            hook = ctx.hooks[step["callback"]]
            attempts = 4 if step.get("repeat_if_slow") else 1
            del seen[:]
            for _ in range(attempts):
                started = time.monotonic()
                result = hook(**step["kwargs"])
                elapsed = time.monotonic() - started
                if (result and result.get("context")) or elapsed < 1.0:
                    break
            print(json.dumps({"callback": step["callback"], "result": result,
                              "elapsed_ms": int(elapsed * 1000),
                              "child": list(seen)}), flush=True)
    finally:
        for unload in ctx.unloads:
            unload()


main()
