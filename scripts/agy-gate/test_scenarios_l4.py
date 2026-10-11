"""Offline tests for the L4 gate flows (setup, permission, allow-rewrite, unsetup, launch, handoff): no agy, Herdr or network."""
import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent / "agy-fake-gemini"))
sys.path.insert(0, str(HERE.parent / "agy-sandbox"))
import gate  # noqa: E402
import scenario  # noqa: E402

REQUIRES = {
    "setup": ("ht-poh.4.4", "ht-poh.4.7"),
    "permission": ("ht-poh.4.4",),
    "allow-rewrite": ("ht-poh.4.4",),
    "unsetup": ("ht-poh.4.4",),
    "launch": ("ht-poh.4.6",),
    "handoff": ("ht-poh.4.6", "ht-poh.8"),
}


def load(name):
    return scenario.load(str(HERE / "scenarios" / (name + ".json")))


def helper():
    spec = importlib.util.spec_from_file_location("l4h", str(HERE / "flows" / "_l4.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class L4ScenarioTests(unittest.TestCase):
    def test_scenarios_validate(self):
        for name in REQUIRES:
            self.assertEqual(load(name)["name"], name)

    def test_flows_export_requires(self):
        flows = {f.name: f for f in gate.discover(HERE / "flows")}
        for name, requires in REQUIRES.items():
            self.assertIn(name, flows)
            self.assertEqual(flows[name].requires, requires)
            self.assertTrue(callable(flows[name].run))

    def test_helper_module_is_not_a_flow(self):
        self.assertNotIn("_l4", [f.name for f in gate.discover(HERE / "flows")])

    def test_permission_steps_route_allowed_then_compound(self):
        cursor = scenario.Cursor(load("permission"))
        self.assertEqual(cursor.select("PERM-ALLOWED go")["label"], "allowed-call")
        self.assertEqual(cursor.select("PERM-ALLOWED go")["label"], "allowed-final")
        history = "PERM-ALLOWED go PERM-DENIED go"
        self.assertEqual(cursor.select(history)["label"], "denied-call")
        self.assertEqual(cursor.select(history)["label"], "denied-final")
        steps = {s["label"]: s for s in load("permission")["steps"]}
        self.assertEqual(steps["allowed-call"]["reply"]["args"]["CommandLine"], "herdr-threads --version")
        self.assertEqual(steps["denied-call"]["reply"]["args"]["CommandLine"], "herdr-threads --version; touch gate-marker")

    def test_handoff_command_uses_new_tab_kind_agy_and_a_var_for_the_binary(self):
        data = load("handoff")
        line = data["steps"][0]["reply"]["args"]["CommandLine"]
        for part in ("handoff", "--new-tab gate-child", "--kind agy", "{var.AGY}"):
            self.assertIn(part, line)
        self.assertEqual(scenario.Cursor(data).select("HANDOFF-PROMPT go")["label"], "handoff-call")

    def test_launch_scenario_serves_the_prompt_once(self):
        cursor = scenario.Cursor(load("launch"))
        self.assertEqual(cursor.select("LAUNCH-PROMPT hello")["label"], "launch")
        self.assertEqual(cursor.remaining_labels(), [])

    def test_launch_refusal_text_matches_the_core(self):
        source = (HERE.parents[1] / "src" / "harness" / "agy" / "launch.rs").read_text()
        self.assertIn(helper().LAUNCH_REFUSAL, source)
        self.assertIn('ALLOW_RULE: &str = "%s"' % helper().ALLOW_RULE,
                      (HERE.parents[1] / "src" / "harness" / "agy" / "setup.rs").read_text())


    def test_every_run_command_carries_the_properties_agy_requires(self):
        required = {"CommandLine", "Cwd", "WaitMsBeforeAsync", "toolAction", "toolSummary"}
        for name in REQUIRES:
            sc = load(name)
            for step in sc["steps"]:
                reply = step["reply"]
                for call in ([reply] if "call" in reply else reply.get("calls", [])):
                    if call["call"] != "run_command":
                        continue
                    self.assertTrue(required <= set(call["args"]), (name, step["label"], call["args"]))
                    self.assertEqual(call["args"]["Cwd"], "{var.cwd}", (name, step["label"]))
                    self.assertIn("cwd", sc.get("vars", {}), name)


class L4WorldHelperTests(unittest.TestCase):
    def test_setup_json_runs_in_the_human_namespace(self):
        calls = []

        class Result(object):
            returncode = 0
            stdout = '{"result":{"data":{"action":"installed"}}}'

        class World(object):
            def ht(self, argv, check=True, human=False, **kwargs):
                calls.append((argv, check, human))
                return Result()

        rc, data = helper().setup_json(World())
        self.assertEqual((rc, data), (0, {"action": "installed"}))
        self.assertEqual(calls, [(["--json", "setup", "agy", "--with-permissions"], False, True)])

    def test_load_into_binds_cwd_and_extra_vars(self):
        loaded = []

        class Backend(object):
            def load(self, scenario):
                loaded.append(scenario)

        class World(object):
            ws = Path("/x/ws")
            backend = Backend()

        sc = helper().load_into(World(), HERE / "scenarios" / "handoff.json", AGY="/bin/agy")
        self.assertEqual(sc["vars"]["cwd"], "/x/ws")
        self.assertEqual(sc["vars"]["AGY"], "/bin/agy")
        self.assertEqual(loaded, [sc])


class L4HelperTests(unittest.TestCase):
    def setUp(self):
        self.h = helper()
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        root = Path(self.tmp.name)

        class World(object):
            home = root / "home"
            agy_data = root / "home" / ".gemini" / "antigravity-cli"
            ctl = root / "ctl"
        World.agy_data.mkdir(parents=True)
        World.ctl.mkdir()
        self.world = World

    def test_find_walks_nested_json(self):
        doc = {"result": {"data": {"plugin": {"installed": True}, "list": [{"allow_entry": {"state": "owned"}}]}}}
        self.assertEqual(self.h.find(doc, "plugin"), {"installed": True})
        self.assertEqual(self.h.find(doc, "allow_entry"), {"state": "owned"})
        self.assertIsNone(self.h.find(doc, "absent"))

    def test_allow_members_reads_only_the_permissions_list(self):
        self.assertEqual(self.h.allow_members(self.world), [])
        self.h.write_settings(self.world, {"permissions": {"allow": ["a", "b"]}, "x": ["c"]})
        self.assertEqual(self.h.allow_members(self.world), ["a", "b"])
        self.h.write_settings(self.world, {"permissions": {"allow": "oops"}})
        self.assertEqual(self.h.allow_members(self.world), [])

    def test_plugin_files_lists_only_existing_owned_files(self):
        base = self.world.home / ".gemini" / "config" / "plugins" / "herdr-threads"
        (base / "skills" / "herdr-threads").mkdir(parents=True)
        (base / "plugin.json").write_text("{}")
        (base / "skills" / "herdr-threads" / "SKILL.md").write_text("s")
        self.assertEqual(sorted(self.h.plugin_files(self.world)), ["plugin.json", "skills/herdr-threads/SKILL.md"])

    def test_body_text_is_empty_without_a_saved_body(self):
        self.assertEqual(self.h.body_text(self.world, {}), "")
        (self.world.ctl / "b.json").write_text(json.dumps({"v": "1.2.3"}))
        self.assertIn("1.2.3", self.h.body_text(self.world, {"body": "b.json"}))

    def test_finish_checks_tolerates_only_named_optional_steps(self):
        class Backend(object):
            def requests(self):
                return [{"kind": "main", "step": "a", "seq": 1}]

        class World(object):
            backend = Backend()
            checks = []

            def check(self, name, ok, detail=""):
                self.checks.append((name, ok))

            def artifact(self, *_):
                pass
        data = {"steps": [{"label": "a"}, {"label": "b"}]}
        world = World()
        self.h.finish_checks(world, data, optional=("b",))
        self.assertTrue(all(ok for _, ok in world.checks))
        world = World()
        world.checks = []
        self.h.finish_checks(world, data)
        self.assertIn(("every scenario step was consumed", False), world.checks)


class RunCommandEnvTests(unittest.TestCase):
    def flow_module(self):
        spec = importlib.util.spec_from_file_location("rce", str(HERE / "flows" / "run_command_env.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def test_flow_is_discovered(self):
        flows = {f.name: f for f in gate.discover(HERE / "flows")}
        self.assertEqual(flows["run_command_env"].requires, ("ht-poh.4.4",))

    def test_steps_chain_env_echo_inbox_then_text(self):
        data = load("run-command-env")
        cursor = scenario.Cursor(data)
        labels = [cursor.select("ENV-PROBE go")["label"] for _ in range(4)]
        self.assertEqual(labels, ["env-call", "echo-call", "inbox-call", "env-final"])
        commands = [s["reply"]["args"]["CommandLine"] for s in data["steps"][:3]]
        self.assertEqual(commands, ["env", "echo ppid=$PPID", "herdr-threads inbox"])

    def test_parse_ppid_reads_the_echoed_pid(self):
        module = self.flow_module()
        self.assertEqual(module.parse_ppid("Output:\nppid=4242\r\n"), 4242)
        self.assertIsNone(module.parse_ppid("zsh:1: operation not permitted: ps"))

    def test_agy_family_names_match_exactly(self):
        module = self.flow_module()
        self.assertTrue(module.is_agy_family("/x/y/agy"))
        self.assertTrue(module.is_agy_family("antigravity-cli"))
        self.assertFalse(module.is_agy_family("/bin/zsh"))
        self.assertFalse(module.is_agy_family("agy-helper"))


if __name__ == "__main__":
    unittest.main()
