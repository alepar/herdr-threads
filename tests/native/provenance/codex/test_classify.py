import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from classify import classify, sanitize
from capture import transcript_metadata, process_identity
from receiver import request
from server import handle_request
import tempfile


class ClassifierTests(unittest.TestCase):
    def setUp(self):
        self.root = {
            "harness": "codex", "version": "0.157.1", "event": "PreToolUse",
            "session_id": "root-session", "turn_id": "root-turn",
            "execution_id": "root-execution", "tool_use_id": "tool-1",
            "nonce": "probe-1", "transport": "rewritten-command",
            "transport_observed": True, "thread_source": "cli",
            "transcript_turn_id": "root-turn", "root_turn_id": "root-turn",
            "host_session_id": "root-session", "hook_codex_pid": 123,
            "host_codex_pid": 123,
        }
        self.host = {
            "session_id": "root-session", "root_execution_id": "root-execution",
            "codex_pid": 123,
            "generation": 2, "observed_generation": 2,
            "observation_after_request": True, "metadata_current": True,
            "context_issued_ms": 1000, "context_expires_ms": 1250,
            "decision_ms": 1100,
        }
        self.root["evidence_available_ms"] = 1001
        self.root["hook_return_ms"] = 1002

    def test_missing_unknown_child_and_stale_deny(self):
        self.assertEqual(classify(self.root, self.host), "SUPPORTED_ROOT")
        for change in (
            {"execution_id": None}, {"execution_id": "child-execution"},
            {"event": "NewHook"}, {"transport_observed": False},
            {"turn_id": None}, {"version": "future"},
            {"agent_id": "child"}, {"thread_source": "subagent"},
            {"transcript_turn_id": "other"}, {"hook_codex_pid": None},
            {"host_codex_pid": 456},
        ):
            with self.subTest(change=change):
                self.assertNotEqual(classify({**self.root, **change}, self.host), "SUPPORTED_ROOT")
        for change in (
            {"generation": 1}, {"metadata_current": False},
            {"observation_after_request": False},
            {"root_execution_id": None},
            {"codex_pid": 456},
        ):
            with self.subTest(host=change):
                self.assertNotEqual(classify(self.root, {**self.host, **change}), "SUPPORTED_ROOT")

    def test_sanitize_only_allowlist(self):
        envelope = {**self.root, "tool_input": {"command": "secret"},
                    "transcript_path": "/private/path", "api_key": "secret"}
        safe = sanitize(envelope)
        self.assertNotIn("tool_input", safe)
        self.assertNotIn("transcript_path", safe)
        self.assertNotIn("api_key", safe)
        self.assertEqual(safe["turn_id"], "root-turn")

    def test_clear_source_is_allowlisted_for_native_session_event(self):
        self.assertEqual(sanitize({"source": "clear", "prompt": "discard"}),
                         {"source": "clear"})

    def test_fixture_manifest_cannot_promote_capture_to_support(self):
        base = Path(__file__).parents[3] / "fixtures/callers/codex"
        manifest = json.loads((base / "manifest.json").read_text())
        cases = json.loads((base / "cases.json").read_text())
        self.assertEqual(manifest["capture_status"], "PASS")
        self.assertIn(manifest["authorization_status"], ("SUPPORTED", "UNSUPPORTED"))
        self.assertEqual({c["scenario"] for c in cases}, set(manifest["scenarios"]))
        for case in cases:
            self.assertEqual(classify(case["call"], case["host"]), case["classification"])
        if manifest["authorization_status"] == "UNSUPPORTED":
            self.assertTrue(all(c["classification"] != "SUPPORTED_ROOT"
                                for c in cases if c["transport_profile"] == "default"))

    def test_hook_time_transcript_metadata_is_allowlisted(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "rollout.jsonl"
            path.write_text('\n'.join([
                json.dumps({"type": "session_meta", "payload": {"id": "child", "source": {"subagent": {"thread_spawn": {"parent_thread_id": "root", "depth": 1}}}}}),
                json.dumps({"type": "turn_context", "payload": {"turn_id": "child-turn", "root_turn_id": "root-turn", "secret": "discard"}}),
            ]) + '\n')
            self.assertEqual(transcript_metadata(str(path)), {
                "thread_source": "subagent", "execution_id": "child",
                "transcript_turn_id": "child-turn", "root_turn_id": "root-turn",
            })

    def test_process_identity_requires_one_codex_foreground_process(self):
        result = {"result": {"process_info": {"foreground_processes": [
            {"name": "codex", "pid": 123}, {"name": "zsh", "pid": 456}]}}}
        self.assertEqual(process_identity(result), 123)
        result["result"]["process_info"]["foreground_processes"].append(
            {"name": "codex", "pid": 789})
        self.assertIsNone(process_identity(result))

    def test_cached_predecessor_matches_session_and_pid_but_is_unknown(self):
        self.assertEqual(classify(self.root, {
            **self.host, "metadata_current": False,
        }), "UNSUPPORTED")

    def test_expired_context_and_post_hook_evidence_deny(self):
        self.assertEqual(classify(self.root, {
            **self.host, "decision_ms": 1251,
        }), "UNSUPPORTED")
        self.assertEqual(classify({
            **self.root, "evidence_available_ms": 1003,
        }, self.host), "UNSUPPORTED")
        self.assertEqual(classify({
            key: value for key, value in self.root.items()
            if key != "hook_return_ms"
        }, self.host), "UNSUPPORTED")

    def test_early_root_child_root_rows_preserve_unknown_native_fields(self):
        base = Path(__file__).parents[3] / "fixtures/callers/codex"
        cases = {case["scenario"]: case for case in json.loads((base / "cases.json").read_text())}
        for scenario, nonce in (
            ("manual_root_before_child", "HT_NONCE_BEFORE"),
            ("native_child", "HT_NONCE_CHILD1"),
            ("manual_root_after_child", "HT_NONCE_AFTER1"),
        ):
            call = cases[scenario]["call"]
            self.assertEqual(cases[scenario]["native_evidence_nonce"], nonce)
            self.assertEqual(call["version"], "unknown")
            self.assertNotIn("execution_id", call)
            self.assertNotIn("hook_codex_pid", call)
            self.assertNotIn("transcript_turn_id", call)
            self.assertEqual(cases[scenario]["classification"], "UNSUPPORTED")

    def test_parameterized_receiver_and_observer_keep_request_context(self):
        payload = request("HT_NONCE_SYNTHETIC", {
            "HT_PROBE_CONTEXT": "exec-token", "HERDR_ENV": "1",
            "HERDR_PANE_ID": "w9:p1",
        })
        observed = []

        def observer(pane):
            observed.append(pane)
            return {"host_session": "session", "host_pid": 321,
                    "observed_utc": "later"}

        result = handle_request(payload, "w9:p1", observer)
        self.assertEqual(observed, ["w9:p1"])
        self.assertEqual(result["context"], "exec-token")
        self.assertEqual(result["host_pid"], 321)
        self.assertIn("received_utc", result)
        self.assertEqual(handle_request(payload, "w9:p2", observer)["error"], "pane")
        self.assertEqual(observed, ["w9:p1"])


if __name__ == "__main__":
    unittest.main()
