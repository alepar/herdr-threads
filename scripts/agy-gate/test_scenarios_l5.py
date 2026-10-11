"""Offline tests for the L5 gate flows (enroll, subagent, clear, resume): no agy, Herdr or network."""
import json
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent / "agy-fake-gemini"))
sys.path.insert(0, str(HERE.parent / "agy-sandbox"))
import gate  # noqa: E402
import scenario  # noqa: E402

REQUIRES = {
    "enroll": ("ht-poh.4.2", "ht-poh.4.3", "ht-poh.4.4"),
    "subagent": ("ht-poh.4.3",),
    "clear": ("ht-poh.4.3",),
    "resume": ("ht-poh.4.3",),
}


def load(name):
    return scenario.load(str(HERE / "scenarios" / (name + ".json")))


class L5ScenarioTests(unittest.TestCase):
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
        self.assertNotIn("_l5", [f.name for f in gate.discover(HERE / "flows")])

    def test_subagent_parent_steps_exclude_and_child_step_requires_the_child_prompt(self):
        steps = {s["label"]: s for s in load("subagent")["steps"]}
        for label in ("parent-call", "parent-final"):
            self.assertEqual(steps[label]["match"]["not_contains"], ["CHILD-PROMPT"])
        self.assertEqual(steps["child"]["match"]["contains"], ["CHILD-PROMPT"])
        args = steps["parent-call"]["reply"]["args"]
        self.assertEqual(steps["parent-call"]["reply"]["call"], "invoke_subagent")
        self.assertEqual(args["Subagents"][0], {"Model": "inherit", "Prompt": "CHILD-PROMPT say hi",
                                                "Role": "Gate Child", "TypeName": "self"})

    def test_subagent_routing_gives_each_request_its_own_step(self):
        cursor = scenario.Cursor(load("subagent"))
        self.assertEqual(cursor.select("PARENT-ENROLL hello")["label"], "enroll")
        parent = "PARENT-ENROLL hello\nPARENT-GO start"
        self.assertEqual(cursor.select(parent)["label"], "parent-call")
        # The parent's follow-up may arrive before the child's first request (research B interleave).
        self.assertEqual(cursor.select(parent + "\nCHILD-OK")["label"], "parent-final")
        self.assertEqual(cursor.select("CHILD-PROMPT say hi")["label"], "child")
        # Once the child reports, agy wakes the parent for one more request.
        self.assertEqual(cursor.select(parent + "\nCHILD-OK")["label"], "parent-report")
        self.assertEqual(cursor.remaining_labels(), [])

    def test_invoke_subagent_carries_the_required_properties(self):
        # agy 1.3.2 `invoke_subagent` declaration, captured by the native gate.
        args = {s["label"]: s for s in load("subagent")["steps"]}["parent-call"]["reply"]["args"]
        self.assertTrue({"Subagents", "toolSummary", "toolAction"} <= set(args), sorted(args))
        for entry in args["Subagents"]:
            self.assertTrue({"TypeName", "Role", "Prompt"} <= set(entry), sorted(entry))

    def test_prompt_markers_are_distinct_per_step(self):
        for name in ("clear", "resume"):
            steps = load(name)["steps"]
            markers = [s["match"]["contains"][0] for s in steps]
            self.assertEqual(len(set(markers)), len(markers))
            for marker in markers:  # no marker is a substring of another, so one prompt picks one step
                self.assertEqual([m for m in markers if marker in m], [marker])

    def test_resume_steps_follow_the_flow_order(self):
        self.assertEqual([s["label"] for s in load("resume")["steps"]], ["a1", "b1", "a2", "a3"])

    def test_startup_marker_matches_the_core_instruction(self):
        import importlib.util
        spec = importlib.util.spec_from_file_location("l5", str(HERE / "flows" / "_l5.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        source = (HERE.parents[1] / "src" / "harness" / "mod.rs").read_text()
        self.assertIn(module.STARTUP_MARK, source)
        self.assertIn(module.CHILD_MARK, source)
        request = {"system_messages": [module.STARTUP_MARK + " tail"]}
        self.assertTrue(module.has_startup(request))
        self.assertEqual(module.startup_count({"system_messages": [module.STARTUP_MARK] * 2}), 2)
        self.assertFalse(module.has_startup({"system_messages": ["other"]}))


class L5QueryTests(unittest.TestCase):
    def setUp(self):
        import importlib.util
        import sqlite3
        import tempfile
        spec = importlib.util.spec_from_file_location("l5q", str(HERE / "flows" / "_l5.py"))
        self.l5 = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.l5)
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        root = Path(self.tmp.name)
        (root / "ht" / "i").mkdir(parents=True)
        conn = sqlite3.connect(str(root / "ht" / "i" / "threads.sqlite3"))
        conn.execute("CREATE TABLE seats(id INTEGER)")
        conn.execute("CREATE TABLE occupant_bindings(binding_id TEXT, seat_id INTEGER, ordinal INTEGER,"
                     " generation INTEGER, harness TEXT, native_session TEXT, observation_provenance TEXT,"
                     " role TEXT, liveness TEXT, lineage_from TEXT, ended_at INTEGER)")
        conn.execute("INSERT INTO seats VALUES (1)")
        rows = [("b1", 1, 1, 1, "agy", "old", "cooperative_top_level", "top_level", "disconnected", None, None),
                ("b2", 1, 2, 1, "agy", "new", "cooperative_top_level", "top_level", "connected", "b1", None),
                ("b3", 1, 3, 1, "agy", "kid", "cooperative_subagent", "subagent", "connected", "b2", None),
                ("b4", 1, 4, 1, "agy", "gone", "cooperative_top_level", "top_level", "connected", None, 5),
                ("b5", 2, 1, 1, "human", "h", "human", "top_level", "connected", None, None)]
        conn.executemany("INSERT INTO occupant_bindings VALUES (?,?,?,?,?,?,?,?,?,?,?)", rows)
        conn.commit()
        conn.close()
        presence = root / "home" / ".gemini" / "antigravity-cli" / "presence"
        presence.mkdir(parents=True)
        (presence / "new.lock").write_text("")

        class World(object):
            state = root / "ht"
            agy_data = root / "home" / ".gemini" / "antigravity-cli"
        self.world = World()

    def test_live_binding_is_the_connected_open_top_level_agy_row(self):
        """Session-keyed (ht-54r.88): the old session's disconnected row, a subagent row, an ended row and a
        human row are not the live binding, and ``generation`` is not what picks it."""
        binding = self.l5.live_binding(self.world)
        self.assertEqual((binding["binding_id"], binding["native_session"]), ("b2", "new"))
        self.assertEqual([b["binding_id"] for b in self.l5.live_bindings(self.world)], ["b2"])

    def test_binding_for_finds_a_session_whatever_its_state(self):
        self.assertEqual(self.l5.binding_for(self.world, "old")["liveness"], "disconnected")
        self.assertEqual(self.l5.binding_for(self.world, "gone")["ended_at"], 5)
        self.assertIsNone(self.l5.binding_for(self.world, "kid"), "a subagent row is not a top-level binding")
        self.assertIsNone(self.l5.binding_for(self.world, "nope"))
        self.assertEqual([b["binding_id"] for b in self.l5.all_bindings(self.world)], ["b1", "b2", "b3", "b4"])

    def test_seat_count_and_presence_ids(self):
        self.assertEqual(self.l5.seat_count(self.world), 1)
        self.assertEqual(self.l5.presence_ids(self.world), ["new"])

    def test_queries_are_empty_before_the_database_exists(self):
        class Bare(object):
            state = Path(self.tmp.name) / "nowhere"
        self.assertIsNone(self.l5.live_binding(Bare()))
        self.assertIsNone(self.l5.seat_count(Bare()))


if __name__ == "__main__":
    unittest.main()
