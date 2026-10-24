"""Offline tests for turn_end_probe.py (no agy, Herdr or relay)."""
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import turn_end_probe as probe  # noqa: E402


class Helpers(unittest.TestCase):
    def test_markers_lists_each_requests_probe_markers(self):
        bodies = [{"m": "x"}, {"m": "a PROBE-STOP-REASON-1 b"}, {"m": ["PROBE-STOP-REASON-1", "PROBE-STOP-REASON-10."]}]
        self.assertEqual(probe.markers(bodies, "PROBE-"),
                         [[], ["PROBE-STOP-REASON-1"], ["PROBE-STOP-REASON-1", "PROBE-STOP-REASON-10"]])

    def test_hooks_config_registers_one_command_per_event(self):
        cfg = probe.hooks_config(Path("/r/ctl"))["turn-end-probe"]
        self.assertEqual(sorted(cfg), sorted(probe.EVENTS))
        self.assertEqual(cfg["Stop"][0]["command"], "/usr/bin/python3 /r/ctl/probe_hook.py Stop /r/ctl")


class Hook(unittest.TestCase):
    def run_hook(self, ctl, event, mode):
        (ctl / "mode").write_text(mode)
        out = subprocess.run([sys.executable, str(ctl / "probe_hook.py"), event, str(ctl)], input="{}",
                             capture_output=True, text=True, check=True).stdout
        return json.loads(out)

    def test_outputs_per_mode(self):
        with tempfile.TemporaryDirectory() as tmp:
            ctl = Path(tmp)
            (ctl / "probe_hook.py").write_text(probe.HOOK)
            self.assertEqual(self.run_hook(ctl, "Stop", "stop-continue"),
                             {"decision": "continue", "reason": "PROBE-STOP-REASON-1"})
            self.assertEqual(self.run_hook(ctl, "Stop", "stop-continue")["reason"], "PROBE-STOP-REASON-2")
            self.assertEqual(self.run_hook(ctl, "PostInvocation", "post-inject"),
                             {"injectSteps": [{"systemMessage": {"systemMessage": "PROBE-POST-INJECT-1"}}]})
            self.assertEqual(self.run_hook(ctl, "PostInvocation", "post-inject"), {})  # first call only
            self.assertEqual(self.run_hook(ctl, "PostInvocation", "post-force")["terminationBehavior"],
                             "force_continue")
            self.assertEqual(self.run_hook(ctl, "Stop", "post-force"), {})


if __name__ == "__main__":
    unittest.main()
