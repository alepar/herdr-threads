"""Real-shell checks: capture must preserve pacing and stop on a failed command."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "demo-try-it.py"


class CaptureTests(unittest.TestCase):
    def record(self, commands, follow=False, timeout=10, stale=False):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            plan = root / "commands.json"
            plan.write_text(json.dumps(commands))
            cast = root / "session.cast"
            extra = ["--timeout", str(timeout)]
            if follow:
                extra += ["--finish-file", str(root / "finished")]
            if stale:
                (root / "finished").touch()
            result = subprocess.run(
                ["python3", str(SCRIPT), "--commands", str(plan), "--output", str(cast),
                 "--delay", "0.02", "--pause", "0", "--cwd", str(root), *extra],
                capture_output=True, timeout=20,
            )
            events = [json.loads(line) for line in cast.read_text().splitlines()] if cast.exists() else []
            if (root / "child.pid").exists():
                child = int((root / "child.pid").read_text())
                with self.assertRaises(ProcessLookupError):
                    os.kill(child, 0)
            return result, events

    def test_types_into_real_shell_at_visible_intervals(self):
        result, events = self.record(["printf 'real-output\\n'"])
        self.assertEqual(result.returncode, 0, result.stderr.decode())
        typed = [event for event in events[1:] if event[1] == "i"]
        self.assertGreater(len(typed), 10)
        self.assertTrue(all(len(event[2]) == 1 for event in typed))
        self.assertGreater(typed[-1][0] - typed[0][0], 0.3)
        output = "".join(event[2] for event in events[1:] if event[1] == "o")
        self.assertIn("\r\nreal-output\r\n", output)

    def test_failure_stops_before_next_command(self):
        result, events = self.record(["false", "printf 'must-not-run\\n'"])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b"command failed", result.stderr)
        typed = "".join(event[2] for event in events[1:] if event[1] == "i")
        self.assertNotIn("must-not-run", typed)

    def test_long_assignment_is_typed_five_times_as_fast_as_command(self):
        command = "printf 'abcdefghijklmnopqrst'"
        result, events = self.record([{"command": command, "fast_from": 8}])
        self.assertEqual(result.returncode, 0, result.stderr.decode())
        typed = [event for event in events[1:] if event[1] == "i"]
        # Scheduler delays are not the configured typing pace. Check the real
        # shell input above, and the exact delay used by capture independently.
        spec = importlib.util.spec_from_file_location("recorder", SCRIPT)
        recorder = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(recorder)
        for char in ("x", " "):
            normal = recorder.typing_delay(0.02, char, 7, 8)
            fast = recorder.typing_delay(0.02, char, 8, 8)
            self.assertAlmostEqual(normal, 5 * fast)
            self.assertEqual(normal, recorder.typing_delay(0.02, char, 8, None))
        self.assertEqual("".join(event[2] for event in typed), command + "\n")

    def test_follow_early_failure_is_not_success(self):
        result, _ = self.record(["false"], follow=True, timeout=1)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b"follow returned", result.stderr)

    def test_validated_follow_can_handle_ctrl_c_with_exit_zero(self):
        result, _ = self.record([
            'bash -c \'trap "exit 0" INT; touch finished; while :; do sleep 1; done\''
        ], follow=True)
        self.assertEqual(result.returncode, 0, result.stderr.decode())

    def test_stale_finish_marker_is_rejected(self):
        result, events = self.record(["true"], follow=True, stale=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b"finish marker already exists", result.stderr)
        self.assertEqual(events, [])

    def test_timeout_reaps_foreground_child(self):
        result, _ = self.record(["sleep 30 & echo $! > child.pid; wait"], timeout=0.2)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b"did not return", result.stderr)

    def test_multiline_prelude_cannot_mask_a_later_slow_failure(self):
        result, events = self.record([
            "# prepare the panes\n# approvals outside camera\nprintf 'ready\\n'",
            "sleep 0.3; false", "printf 'must-not-run\\n'",
        ])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b"command failed", result.stderr)
        typed = "".join(event[2] for event in events[1:] if event[1] == "i")
        self.assertNotIn("must-not-run", typed)


if __name__ == "__main__":
    unittest.main()
