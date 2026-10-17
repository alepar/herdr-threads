"""Offline tests for the L7 wake flow (no agy, Herdr or network)."""
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE / "flows"))
sys.path.insert(0, str(HERE.parent / "agy-fake-gemini"))
sys.path.insert(0, str(HERE.parent / "agy-sandbox"))
import _l7  # noqa: E402
import gate  # noqa: E402
import scenario  # noqa: E402

REQUIRES = ("ht-poh.4.5", "ht-poh.5", "ht-poh.3.4", "ht-poh.3.6")
SCENARIOS = ("wake", "wake-subagent")
TOOL_PROPERTIES = {"CommandLine", "Cwd", "WaitMsBeforeAsync", "toolAction", "toolSummary"}


def load(name):
    return scenario.load(str(HERE / "scenarios" / (name + ".json")))


class L7ScenarioTests(unittest.TestCase):
    def test_scenarios_validate(self):
        for name in SCENARIOS:
            self.assertEqual(load(name)["name"], name)

    def test_flow_exports_requires(self):
        flows = {f.name: f for f in gate.discover(HERE / "flows")}
        self.assertEqual(flows["wake"].requires, REQUIRES)
        self.assertTrue(callable(flows["wake"].run))

    def test_helper_module_is_not_a_flow(self):
        self.assertNotIn("_l7", [f.name for f in gate.discover(HERE / "flows")])

    def test_step_labels_are_unique_across_the_flow(self):
        seen = {}
        for name in SCENARIOS:
            for step in load(name)["steps"]:
                self.assertNotIn(step["label"], seen)
                seen[step["label"]] = name

    def test_every_run_command_carries_the_properties_agy_requires(self):
        for name in SCENARIOS:
            for step in load(name)["steps"]:
                reply = step["reply"]
                if reply.get("call") == "run_command":
                    self.assertTrue(TOOL_PROPERTIES <= set(reply["args"]), (name, step["label"]))

    def test_invoke_subagent_carries_the_tool_properties_agy_requires(self):
        # Without toolAction/toolSummary agy refuses the call and no child ever runs (live gate run).
        calls = [s["reply"] for s in load("wake-subagent")["steps"] if s["reply"].get("call") == "invoke_subagent"]
        self.assertEqual(len(calls), 2)
        for reply in calls:
            self.assertTrue({"toolAction", "toolSummary", "Subagents"} <= set(reply["args"]))

    def test_busy_step_waits_for_publication_release(self):
        steps = {s["label"]: s for s in load("wake")["steps"]}
        self.assertGreater(steps["c-busy"]["hold_ms"], 0)

    def test_wake_steps_route_by_their_own_markers(self):
        cursor = scenario.Cursor(load("wake"))
        history = "WAKE-WARM hello"
        self.assertEqual(cursor.select(history)["label"], "warm")
        self.assertEqual(cursor.select(history + " herdr-threads: wake up")["label"], "wake-join")
        history += " herdr-threads: wake up WAKE-N1 ack me"
        self.assertEqual(cursor.select(history)["label"], "a-ack")
        self.assertEqual(cursor.select(history)["label"], "a-done")
        history += " WAKE-BUSY-C go"
        self.assertEqual(cursor.select(history)["label"], "c-busy")
        # The follow-up carries N2 inside a functionResponse, which the text routing cannot see.
        self.assertEqual(cursor.select(history)["label"], "c-after")
        history += " WAKE-N2 WAKE-DRAFT keep me WAKE-N3"
        self.assertEqual(cursor.select(history)["label"], "d-draft")
        history += " WAKE-SETON go"
        self.assertEqual(cursor.select(history)["label"], "e-set")
        self.assertEqual(cursor.select(history)["label"], "e-done")
        self.assertEqual(cursor.select(history + " herdr-threads: wake up WAKE-Z2 lazy")["label"], "e-wake")

    def test_later_wakes_do_not_take_the_join_step(self):
        cursor = scenario.Cursor(load("wake"))
        self.assertEqual(cursor.select("WAKE-WARM x")["label"], "warm")
        self.assertEqual(cursor.select("WAKE-WARM herdr-threads: wake up WAKE-N1")["label"], "a-ack")

    def test_subagent_parent_and_child_steps_route_apart(self):
        cursor = scenario.Cursor(load("wake-subagent"))
        parent = "WAKE-PGEN go"
        self.assertEqual(cursor.select(parent)["label"], "f-parent-call")
        self.assertEqual(cursor.select(parent + " WAKE-KIDGEN say hi")["label"], "f-child")
        self.assertEqual(cursor.select(parent)["label"], "f-parent-final")
        self.assertEqual(cursor.select(parent + " WAKE-KIDGEN-DONE")["label"], "f-parent-after")
        parent = "WAKE-PTOOL go"
        self.assertEqual(cursor.select(parent)["label"], "g-parent-call")
        self.assertEqual(cursor.select("WAKE-KIDTOOL run it")["label"], "g-child-call")
        self.assertEqual(cursor.select("WAKE-KIDTOOL run it")["label"], "g-child-final")
        self.assertEqual(cursor.select(parent)["label"], "g-parent-final")
        self.assertEqual(cursor.select(parent + " WAKE-KIDTOOL-DONE")["label"], "g-parent-after")

    def test_subagent_child_holds_and_blocks_longer_than_the_quiet_windows(self):
        steps = {s["label"]: s for s in load("wake-subagent")["steps"]}
        self.assertGreater(steps["f-child"]["delay_ms"], 2000 + 12000)
        self.assertGreater(steps["g-child-call"]["reply"]["args"]["WaitMsBeforeAsync"], 2000 + 12000)


class UntypedTests(unittest.TestCase):
    def test_a_wake_is_a_user_request_the_gate_did_not_type(self):
        typed = ["WAKE-WARM hello", "WAKE-BUSY-C go"]
        request = {"user_requests": ["WAKE-WARM hello", "herdr-threads: wake up", "WAKE-BUSY-C go"]}
        self.assertEqual(_l7.untyped_entries(request, typed), ["herdr-threads: wake up"])
        self.assertEqual(_l7.untyped_entries({"user_requests": ["WAKE-WARM hello"]}, typed), [])
        self.assertEqual(_l7.untyped_entries({}, typed), [])

    def test_an_untyped_prompt_is_not_hidden_by_an_empty_typed_text(self):
        self.assertEqual(_l7.untyped_entries({"user_requests": ["x"]}, [""]), ["x"])

    def test_first_wake_is_the_first_request_over_the_baseline(self):
        typed = _l7.Typed()
        typed.texts.append("T")

        class Obs(object):
            reqs = [{"seq": 1, "user_requests": ["T"]},
                    {"seq": 2, "user_requests": ["T", "wake up"]},
                    {"seq": 3, "user_requests": ["T", "wake up", "wake up"]}]
        import observe
        original = observe.main_requests
        observe.main_requests = lambda world: Obs.reqs
        try:
            self.assertEqual(_l7.first_wake(None, typed, 0)["seq"], 2)
            self.assertEqual(_l7.first_wake(None, typed, 1)["seq"], 3)
            self.assertIsNone(_l7.first_wake(None, typed, 2))
            self.assertEqual(_l7.untyped_count(None, typed), 2)
        finally:
            observe.main_requests = original

    def test_draft_counts_as_typed_only_once_submitted(self):
        class Pane(object):
            def __init__(self):
                self.sent = []

            def keys(self, *k):
                self.sent.append(k)

            def draft(self, text):
                self.sent.append(("draft", text))
        typed = _l7.Typed()
        pane = Pane()
        pane.draft("D")
        self.assertEqual(typed.texts, [])
        typed.submit_draft(pane, "D")
        self.assertEqual((typed.texts, pane.sent[-1]), (["D"], ("Enter",)))

    def test_body_frames_reads_a_frame_injected_into_a_function_response(self):
        # Shape captured from genuine agy 1.3.2: the hook's systemMessage rides in the tool output.
        output = ("The command exited with code 0.\n\n<SYSTEM_MESSAGE>\n[herdr-threads] Messages follow.\n\n"
                  "[herdr-threads] message mN2 in tT1 \"gate\" from sS1 [human]:\n  WAKE-N2 mid turn\n</SYSTEM_MESSAGE>")
        body = {"contents": [{"role": "model", "parts": [{"functionResponse": {"name": "run_command",
                                                                              "response": {"output": output}}}]}]}
        self.assertEqual(_l7.frame_ids_in_body(body), ["mN2"])
        self.assertEqual(_l7.frame_ids_in_body({"contents": []}), [])
        self.assertEqual(_l7.frame_ids_in_body(None), [])

    def test_frames_of_reads_ids_from_system_messages(self):
        text = '[herdr-threads] message mN1 in tT1 "gate" from sS1 [human]:\n  body'
        self.assertEqual(_l7.frames_of({"system_messages": [text]}), ["mN1"])
        self.assertEqual(_l7.frames_of(None), [])


if __name__ == "__main__":
    unittest.main()
