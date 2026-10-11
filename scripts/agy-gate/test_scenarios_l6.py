"""Offline tests for the L6 gate flows (delivery, downgrade): no agy, Herdr or network."""
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
    "delivery": ("ht-poh.5", "ht-poh.3.4"),
    "downgrade": ("ht-poh.5", "ht-poh.4.7"),
}
SCENARIOS = ("delivery", "delivery-stop", "delivery-cancel", "delivery-error", "downgrade", "downgrade-ignore")
TOOL_PROPERTIES = {"CommandLine", "Cwd", "WaitMsBeforeAsync", "toolAction", "toolSummary"}

HEADER = "[herdr-threads] new messages:\n\n"


def load(name):
    return scenario.load(str(HERE / "scenarios" / (name + ".json")))


def load_helper():
    import importlib.util
    spec = importlib.util.spec_from_file_location("l6h", str(HERE / "flows" / "_l6.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def frame(kind, mid, body, thread='tT1 "gate"', sender="sS1"):
    return "[herdr-threads] %s %s in %s from %s [human]:\n  %s" % (kind, mid, thread, sender, body)


class L6ScenarioTests(unittest.TestCase):
    def test_scenarios_validate(self):
        for name in SCENARIOS:
            self.assertEqual(load(name)["name"], name)

    def test_flows_export_requires(self):
        flows = {f.name: f for f in gate.discover(HERE / "flows")}
        for name, requires in REQUIRES.items():
            self.assertIn(name, flows)
            self.assertEqual(flows[name].requires, requires)
            self.assertTrue(callable(flows[name].run))

    def test_helper_module_is_not_a_flow(self):
        self.assertNotIn("_l6", [f.name for f in gate.discover(HERE / "flows")])

    def test_step_labels_are_unique_within_a_flow(self):
        # A flow loads several scenarios into one request log, and consumption is judged by label.
        for family in ("delivery", "downgrade"):
            seen = {}
            for name in [n for n in SCENARIOS if n.startswith(family)]:
                for step in load(name)["steps"]:
                    self.assertNotIn(step["label"], seen,
                                     "%s reuses %s's label %s" % (name, seen.get(step["label"]), step["label"]))
                    seen[step["label"]] = name

    def test_every_run_command_carries_the_properties_agy_requires(self):
        for name in SCENARIOS:
            for step in load(name)["steps"]:
                reply = step["reply"]
                for call in ([reply] if "call" in reply else reply.get("calls", [])):
                    self.assertEqual(call["call"], "run_command")
                    self.assertTrue(TOOL_PROPERTIES <= set(call["args"]), (name, step["label"], call["args"]))

    def test_main_phase_acks_a1_bound_from_the_page_despite_stale_frames(self):
        sc = load("delivery")
        stale = HEADER + frame("message", "mOLD", "STOP-NOTE earlier phase")
        page = HEADER + "\n\n".join([
            frame("lazy", "mL1", "MAIN-L1 lazy note"), frame("message", "mO1", "MAIN-O1 ordinary note"),
            frame("message", "mA1", "MAIN-A1 ack me"), frame("message", "mT1", "MAIN-T1 TTTT\n  …truncated; run herdr-threads body mT1")])
        cursor = scenario.Cursor(sc)
        self.assertEqual(cursor.select("DELIV-BUSY start\n" + stale)["label"], "busy")
        step = cursor.select("DELIV-BUSY start\n" + stale + "\nMAIN-GO now\n" + page)
        self.assertEqual(step["label"], "inv0")
        bound = scenario.bind_frame(scenario.frames(stale + "\n" + page), step["bind"])
        self.assertEqual(bound["id"], "mA1")
        reply, error = scenario.render(step["reply"], {"cwd": "/w"}, bound)
        self.assertIsNone(error)
        self.assertEqual(reply["args"]["CommandLine"], "herdr-threads ack mA1")
        self.assertEqual(reply["args"]["Cwd"], "/w")
        self.assertEqual(cursor.select("MAIN-O1 again")["label"], "inv1")

    def test_phase_markers_keep_earlier_phases_from_matching_later_requests(self):
        cursor = scenario.Cursor(load("delivery"))
        history = "STOP-BUSY STOP-NOTE CANCEL-NOTE ERROR-NOTE"
        self.assertEqual(cursor.select(history + " DELIV-BUSY start")["label"], "busy")
        self.assertEqual(cursor.select(history + " DELIV-BUSY MAIN-O1")["label"], "inv0")

    def test_error_scenario_returns_a_non_retried_client_error(self):
        steps = {s["label"]: s for s in load("delivery-error")["steps"]}
        error = steps["error-inv0"]["reply"]["error"]
        self.assertEqual(error["http"], 400)

    def test_cancel_scenario_holds_longer_than_the_flow_waits(self):
        steps = {s["label"]: s for s in load("delivery-cancel")["steps"]}
        self.assertGreaterEqual(steps["cancel-inv0"]["delay_ms"], 15000)

    def test_downgrade_turns_route_by_their_own_markers(self):
        cursor = scenario.Cursor(load("downgrade"))
        for text, label in (("DOWN-WARM hello", "warm"), ("DOWN-WARM DOWN-BUSY1", "busy1"),
                            ("DOWN-BUSY1 DOWN-BUSY2", "busy2"), ("DOWN-BUSY2 DOWN-M2 frame", "m2-inv0"),
                            ("DOWN-M2 DOWN-BUSY3", "busy3"), ("DOWN-BUSY3 DOWN-M3 frame", "m3-inv0")):
            self.assertEqual(cursor.select(text)["label"], label)

    def test_ignore_scenario_has_two_distinct_turns(self):
        labels = [s["label"] for s in load("downgrade-ignore")["steps"]]
        self.assertEqual(labels[:4], ["ign-busy1", "ign-go1", "ign-busy2", "ign-go2"])


    def test_wake_join_routes_on_the_content_free_wake_text(self):
        for name, warm in (("delivery-stop", "DELIV-WARM hello"), ("downgrade", "DOWN-WARM hello")):
            cursor = scenario.Cursor(load(name))
            self.assertEqual(cursor.select(warm)["label"], "warm")
            self.assertEqual(cursor.select(warm + "\nherdr-threads: wake up")["label"], "wake-join")

    def test_the_join_wake_runs_accept_through_run_command_as_the_agy_session(self):
        """ht-54r.88: the agy seat accepts from inside agy (env claim + agy ancestor), not from the gate."""
        for name, warm, extra in (("delivery-stop", "DELIV-WARM hello", ""), ("downgrade", "DOWN-WARM hello", ""),
                                  ("wake", "WAKE-WARM hello", "")):
            data = load(name)
            labels = [s["label"] for s in data["steps"]]
            self.assertEqual(labels[labels.index("wake-join") + 1], "wake-join-done", name)
            call = next(s for s in data["steps"] if s["label"] == "wake-join")["reply"]
            self.assertEqual(call["call"], "run_command", name)
            self.assertEqual(call["args"]["CommandLine"], "herdr-threads accept {var.thread}", name)
            self.assertTrue(TOOL_PROPERTIES <= set(call["args"]), name)
            cursor = scenario.Cursor(data)
            self.assertEqual(cursor.select(warm)["label"], "warm")
            wake = warm + "\nherdr-threads: wake up"
            self.assertEqual(cursor.select(wake)["label"], "wake-join")
            self.assertEqual(cursor.select(wake + "\nexited with code 0")["label"], "wake-join-done")

    def test_accept_verdict_names_the_attribution_refusal(self):
        l6 = load_helper()
        ok, _ = l6.accept_verdict("herdr-threads: joined\nCommand exited with code 0", "joined")
        self.assertTrue(ok)
        refused = "herdr-threads: seat s1 has no lifecycle check-in context yet (invalid_request)\nexited with code 1"
        ok, detail = l6.accept_verdict(refused, "invited")
        self.assertFalse(ok)
        self.assertIn("invalid_request", detail)
        ok, detail = l6.accept_verdict("exited with code 0", "invited")
        self.assertFalse(ok, "exit 0 is not enough: the membership must be joined")
        ok, _ = l6.accept_verdict("", "joined")
        self.assertFalse(ok, "no accept output means the accept never ran")


    def test_flows_never_act_as_the_agy_seat_from_the_gate_process(self):
        """Agent seats have no pane-context journal: accept and wake-on run only inside agy (run_command)."""
        for flow in ("delivery", "downgrade", "wake"):
            text = (HERE / "flows" / (flow + ".py")).read_text()
            for verb in ('["accept"', '["wake-on"'):
                self.assertNotIn(verb, text, "%s runs %s from the gate" % (flow, verb))
        self.assertNotIn('["accept"', (HERE / "flows" / "_l6.py").read_text())

    def test_no_scenario_routes_on_the_inbox_marker(self):
        for path in sorted((HERE / "scenarios").glob("*.json")):
            for step in scenario.load(str(path))["steps"]:
                for text in (step.get("match") or {}).get("contains", []):
                    self.assertNotIn("attention pending", text, "%s step %s" % (path.name, step["label"]))

    def test_wake_text_is_the_core_constant(self):
        source = (HERE.parent.parent / "src/notification/policy.rs").read_text()
        self.assertIn('pub const WAKE_UP: &str = "herdr-threads: wake up";', source)

    def test_the_join_wake_is_not_optional(self):
        for flow in ("delivery", "downgrade"):
            text = (HERE / "flows" / (flow + ".py")).read_text()
            self.assertIn('await_step(world, "wake-join", 45)', text, flow)
            self.assertNotIn('("wake-join"', text, flow)


if __name__ == "__main__":
    unittest.main()
