import importlib.util
import hashlib
import pathlib
import subprocess
import tempfile
import unittest


HERE = pathlib.Path(__file__).resolve().parent


def load(name):
    spec = importlib.util.spec_from_file_location(name, HERE / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ClassificationTests(unittest.TestCase):
    def test_unknown_missing_child_and_stale_fail_closed(self):
        classify = load("classify").classify
        reference = {"session_id": "root-a", "generation": 3, "observed_after_request": True,
                     "host_pid": 123, "host_observed_in_hook": True,
                     "context_expired": False, "known_replacement": False}
        base = {"harness": "claude", "version": "2.1.283", "event": "PreToolUse",
                "tool_name": "Bash", "session_id": "root-a", "tool_use_id": "tool-a",
                "invocation_context": "ctx-a", "context_received": "ctx-a",
                "generation": 3, "hook_before_tool": True,
                "hook_parent_pid": 123, "host_session_id": "root-a",
                "observer_status": "ok"}
        self.assertEqual(classify(base, reference), "ROOT_CANDIDATE")
        self.assertEqual(classify({**base, "agent_id": "child-a", "agent_type": "general-purpose"},
                                  reference), "CHILD")
        self.assertEqual(classify({**base, "agent_id": "child-a"}, reference), "UNSUPPORTED")
        self.assertEqual(classify({**base, "agent_id": "child-a", "agent_type": ""}, reference),
                         "UNSUPPORTED")
        self.assertEqual(classify({**base, "agent_type": "Explore"}, reference), "UNSUPPORTED")
        self.assertEqual(classify({k: v for k, v in base.items() if k != "tool_use_id"}, reference), "UNSUPPORTED")
        self.assertEqual(classify({**base, "session_id": "other"}, reference), "UNSUPPORTED")
        self.assertEqual(classify({**base, "generation": 2}, reference), "UNSUPPORTED")
        self.assertEqual(classify({**base, "context_received": None}, reference), "UNSUPPORTED")
        self.assertEqual(classify({**base, "hook_before_tool": False}, reference), "UNSUPPORTED")
        self.assertEqual(classify({**base, "future_field": "unknown"}, reference), "UNSUPPORTED")
        self.assertEqual(classify(base, {**reference, "observed_after_request": False}), "UNSUPPORTED")
        self.assertEqual(classify(base, {**reference, "observed_after_request": "yes"}),
                         "UNSUPPORTED")
        self.assertEqual(classify(base, {**reference, "host_pid": 999}), "UNSUPPORTED")
        self.assertEqual(classify(base, {**reference, "host_observed_in_hook": False}), "UNSUPPORTED")
        self.assertEqual(classify(base, {**reference, "context_expired": True}), "UNSUPPORTED")
        self.assertEqual(classify(base, {**reference, "known_replacement": True}), "UNSUPPORTED")
        self.assertEqual(classify({**base, "host_session_id": "cached-old"}, reference), "UNSUPPORTED")

    def test_manifest_does_not_equate_capture_with_authorization(self):
        manifest = pathlib.Path(__file__).parents[3] / "fixtures/callers/claude/manifest.json"
        import json
        data = json.loads(manifest.read_text())
        self.assertIn("capture_status", data)
        self.assertIn("authorization_status", data)
        self.assertNotEqual(data["capture_status"], data["authorization_status"])

    def test_every_native_and_synthetic_fixture_replays(self):
        from datetime import datetime
        import json
        cases_path = pathlib.Path(__file__).parents[3] / "fixtures/callers/claude/cases.json"
        data = json.loads(cases_path.read_text())
        cases = data["cases"]
        names = {case["name"] for case in cases}
        self.assertTrue({"root", "child", "root_after_child", "concurrent_child_a",
                         "concurrent_child_b", "resumed_same_conversation",
                         "new_after_clear", "replacement_before_observation",
                         "cached_predecessor_pid", "missing_metadata",
                         "unknown_shape", "expired_context", "stale_generation",
                         "observation_before_request", "cached_predecessor_session",
                         "missing_invocation_context", "evidence_after_hook_return",
                         "environment_root", "environment_child"} <= names)
        for name in ("environment_root", "environment_child"):
            environment = next(case["evidence"] for case in cases if case["name"] == name)
            self.assertEqual(environment["claude_env_session_label"], "conversation-3")
            self.assertIsNone(environment["claude_env_agent_label"])
        classify = load("classify").classify
        for case in cases:
            with self.subTest(case=case["name"]):
                self.assertEqual(classify(case["envelope"], case["reference"]),
                                 case["expected"])
                evidence = case["evidence"]
                self.assertEqual(evidence["source_nonce"], "ht4is-claude-pyzgDs")
                if evidence["kind"] == "native":
                    self.assertIn("tool_result_status", evidence)
                    hook = datetime.fromisoformat(evidence["hook_utc"])
                    result = datetime.fromisoformat(evidence["tool_result_utc"].replace("Z", "+00:00"))
                    self.assertLess(hook, result)
                    if evidence["host_observed_utc"]:
                        observed = datetime.fromisoformat(evidence["host_observed_utc"])
                        self.assertLessEqual(hook, observed)
                        self.assertLess(observed, result)
                else:
                    self.assertEqual(evidence["kind"], "synthetic_mutation")
                    self.assertEqual(evidence["derived_from"], "root_observed")

    def test_recovered_native_transitions_bind_preturn_rows_to_later_hooks(self):
        from datetime import datetime
        import json
        path = pathlib.Path(__file__).parents[3] / "fixtures/callers/claude/cases.json"
        data = json.loads(path.read_text())
        source = {row["source_line"]: row for row in data["transition_sources"]}
        transitions = {row["name"]: row for row in data["transitions"]}
        self.assertEqual(len(source), 14)
        self.assertEqual(set(transitions), {"resume_same_conversation_new_pid",
                                            "fresh_replacement_preturn",
                                            "clear_before_first_turn"})
        resume_old, resume_new = source[379], source[423]
        self.assertEqual(resume_old["session_sha256"], resume_new["session_sha256"])
        self.assertNotEqual(resume_old["pid_sha256"], resume_new["pid_sha256"])
        replacement_old, replacement_new = source[423], source[495]
        self.assertNotEqual(replacement_old["session_sha256"],
                            replacement_new["session_sha256"])
        self.assertNotEqual(replacement_old["pid_sha256"], replacement_new["pid_sha256"])
        clear_old, clear_new = source[495], source[512]
        self.assertNotEqual(clear_old["session_sha256"], clear_new["session_sha256"])
        self.assertEqual(clear_old["pid_sha256"], clear_new["pid_sha256"])
        self.assertEqual(clear_old["host_revision"], clear_new["host_revision"])
        native = {case["evidence"]["source_line"]: case for case in data["cases"]
                  if case["evidence"]["kind"] == "native"}
        for name in ("resume_same_conversation_new_pid", "clear_before_first_turn"):
            transition = transitions[name]
            old = source[transition["before_observation"]]
            new = source[transition["after_observation"]]
            native_case = native[transition["first_later_hook_line"]]
            hook = native_case["evidence"]
            self.assertEqual(new["session_sha256"], hook["source_session_sha256"])
            self.assertEqual(new["pid_sha256"], hook["source_host_pid_sha256"])
            self.assertEqual(native_case["envelope"]["session_id"],
                             transition["after_session_label"])
            self.assertLess(datetime.fromisoformat(old["event_utc"].replace("Z", "+00:00")),
                            datetime.fromisoformat(new["event_utc"].replace("Z", "+00:00")))
            self.assertLess(datetime.fromisoformat(new["event_utc"].replace("Z", "+00:00")),
                            datetime.fromisoformat(hook["hook_utc"]))
        for transition in transitions.values():
            timeline = [source[line] for line in transition["source_lines"]]
            timestamps = [datetime.fromisoformat(row["event_utc"].replace("Z", "+00:00"))
                          for row in timeline]
            self.assertEqual(timestamps, sorted(timestamps))
        self.assertEqual([source[n]["kind"] for n in
                          transitions["clear_before_first_turn"]["source_lines"]],
                         ["observe", "clear_text", "enter", "observe"])
        self.assertEqual([source[n]["kind"] for n in
                          transitions["fresh_replacement_preturn"]["source_lines"]],
                         ["observe", "exit_text", "enter", "pane_get",
                          "agent_start", "observe"])
        self.assertIsNone(transitions["fresh_replacement_preturn"]["first_later_hook_line"])

    def test_hook_recorder_rewrites_only_probe_and_allowlists_log(self):
        import json
        import os
        with tempfile.TemporaryDirectory() as tmp:
            command = "printf 'HT4IS_CONTEXT=%s\\n' \"$CLAUDE_PROBE_CONTEXT\""
            hook = {"hook_event_name": "PreToolUse", "session_id": "root-a",
                    "transcript_path": "/private/sensitive/root-a.jsonl", "tool_name": "Bash",
                    "tool_use_id": "tool-a", "tool_input": {"command": command},
                    "secret": "DO_NOT_LOG"}
            env = {**os.environ, "CLAUDE_PROBE_LOG": str(pathlib.Path(tmp) / "events.jsonl"),
                   "CLAUDE_PROBE_NONCE": "nonce-a", "CLAUDE_PROBE_GENERATION": "3",
                   "CLAUDE_CODE_SESSION_ID": "native-session-a",
                   "CLAUDE_CODE_AGENT_ID": "native-agent-a"}
            result = subprocess.run(["python3", str(HERE / "capture.py")], input=json.dumps(hook),
                                    text=True, capture_output=True, env=env)
            self.assertEqual(result.returncode, 0, result.stderr)
            output = json.loads(result.stdout)
            rewritten = output["hookSpecificOutput"]["updatedInput"]["command"]
            self.assertIn("CLAUDE_PROBE_CONTEXT=", rewritten)
            self.assertTrue(rewritten.endswith(command))
            log = json.loads(pathlib.Path(env["CLAUDE_PROBE_LOG"]).read_text().strip())
            self.assertNotIn("secret", log)
            self.assertNotIn("tool_input", log)
            self.assertNotIn("transcript_path", log)
            self.assertEqual(log["tool_use_id"], "tool-a")
            self.assertEqual(log["claude_code_session_id"], "native-session-a")
            self.assertEqual(log["claude_code_agent_id"], "native-agent-a")
            self.assertEqual(log["invocation_context"], rewritten.split("=", 1)[1].split(";", 1)[0])
            invoked = subprocess.run(["sh", "-c", rewritten], text=True, capture_output=True)
            self.assertEqual(invoked.returncode, 0, invoked.stderr)
            self.assertEqual(invoked.stdout.strip(), "HT4IS_CONTEXT=" + log["invocation_context"])
            hook["tool_input"]["command"] = "echo unrelated"
            second = subprocess.run(["python3", str(HERE / "capture.py")], input=json.dumps(hook),
                                    text=True, capture_output=True, env=env)
            self.assertEqual(second.stdout, "")

    def test_host_observer_strips_command_lines_and_requires_one_claude_pid(self):
        capture = load("capture")
        missing = capture.observe_host(None)
        self.assertEqual(missing["observer_status"], "missing_pane")
        self.assertIn("observed_utc", missing)
        self.assertIn("observed_monotonic_ns", missing)
        pane = {"result": {"pane": {"pane_id": "w4:p3", "revision": 9,
                "agent_session": {"value": "root-a"}, "agent_status": "working"}}}
        process = {"result": {"process_info": {"foreground_processes": [
            {"argv0": "claude", "name": "2.1.283", "pid": 123,
             "cmdline": "SECRET", "argv": ["SECRET"]},
            {"argv0": "node", "pid": 456, "cmdline": "SECRET"}]}}}
        row = capture.sanitize_host(pane, process, "w4:p3")
        self.assertEqual(row["host_session_id"], "root-a")
        self.assertEqual(row["host_pid"], 123)
        self.assertEqual(row["host_revision"], 9)
        self.assertNotIn("SECRET", str(row))
        process["result"]["process_info"]["foreground_processes"].append(
            {"argv0": "claude", "pid": 789})
        self.assertEqual(capture.sanitize_host(pane, process, "w4:p3"),
                         {"observer_status": "ambiguous_process"})

    def test_receiver_checks_native_tool_result_in_child_transcript(self):
        import json
        capture = load("capture")
        with tempfile.TemporaryDirectory() as tmp:
            base = pathlib.Path(tmp)
            child = base / "session-a/subagents/agent-child-a.jsonl"
            child.parent.mkdir(parents=True)
            child.write_text("\n".join(json.dumps(row) for row in [
                {"timestamp": "2026-09-27T00:00:01Z", "message": {"content": [
                    {"type": "tool_use", "name": "Bash", "id": "tool-a", "input": {
                        "command": capture.PROBE_COMMAND}}]}},
                {"timestamp": "2026-09-27T00:00:02Z", "message": {"content": [
                    {"type": "tool_result", "tool_use_id": "tool-a",
                     "content": "HT4IS_CONTEXT=ctx-a\n"}]}},
            ]) + "\n")
            event = {"session_id": "session-a", "agent_id": "child-a",
                     "tool_use_id": "tool-a", "invocation_context": "ctx-a"}
            receipt = capture.verify_receipt(event, base)
            self.assertEqual(receipt["status"], "matched")
            self.assertEqual(receipt["transcript_kind"], "child")
            self.assertEqual(receipt["result_utc"], "2026-09-27T00:00:02Z")
            self.assertNotIn("content", receipt)
            self.assertEqual(capture.verify_receipt({**event, "invocation_context": "wrong"},
                                                    base)["status"], "mismatch")

    def test_recover_preturn_observer_and_control_rows_from_original_rollout(self):
        import json
        capture = load("capture")
        pane = "w4:p3"
        before = {"observer_status": "ok", "host_pid": 101, "host_session_id": "old",
                  "host_revision": 8, "host_agent_status": "idle",
                  "observed_utc": "2026-09-27T14:53:56Z", "observed_monotonic_ns": 20}
        after = {**before, "host_session_id": "new", "host_agent_status": "done",
                 "observed_utc": "2026-09-27T14:54:46Z", "observed_monotonic_ns": 30}
        def entry(command, stdout, timestamp):
            return {"type": "event_msg", "timestamp": timestamp, "payload": {
                "type": "item_completed", "item": {"type": "CommandExecution",
                "command": ["/bin/zsh", "-lc", command], "stdout": stdout,
                "exit_code": 0}}}
        rows = [
            entry("python3 tests/native/provenance/claude/capture.py --observe w4:p3",
                  json.dumps(before) + "\n", "2026-09-27T14:53:57Z"),
            entry("herdr pane send-text w4:p3 /clear", "SECRET", "2026-09-27T14:54:16Z"),
            entry("herdr pane send-keys w4:p3 enter", "", "2026-09-27T14:54:30Z"),
            entry("python3 tests/native/provenance/claude/capture.py --observe w4:p3",
                  json.dumps(after) + "\n", "2026-09-27T14:54:47Z"),
            entry("cat private-secret", "SECRET", "2026-09-27T14:54:50Z"),
        ]
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp) / "rollout.jsonl"
            lines = [json.dumps(row) for row in rows]
            path.write_text("\n".join(lines) + "\n")
            recovered = capture.recover_timeline(path, pane)
            expected = [{
                "kind": row["kind"], "source_line": row["source_line"],
                "source_line_sha256": row["source_line_sha256"],
                "event_utc": row["event_utc"],
            } for row in recovered]
            self.assertEqual(capture.verify_transition_sources(path, pane, expected), 4)
            expected[0]["source_line_sha256"] = "wrong"
            with self.assertRaises(ValueError):
                capture.verify_transition_sources(path, pane, expected)
            del expected[0]["source_line_sha256"]
            with self.assertRaises(ValueError):
                capture.verify_transition_sources(path, pane, expected)
        self.assertEqual([row["kind"] for row in recovered],
                         ["observe", "clear_text", "enter", "observe"])
        self.assertEqual(recovered[0]["source_line"], 1)
        self.assertEqual(recovered[0]["source_line_sha256"],
                         hashlib.sha256(lines[0].encode()).hexdigest())
        self.assertEqual(recovered[3]["observation"], after)
        self.assertNotIn("SECRET", str(recovered))


if __name__ == "__main__":
    unittest.main()
