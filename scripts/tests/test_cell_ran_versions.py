"""The native cell runner's harness_ran detector (ht-4p6).

`scripts/validate-native-demo.sh --ran-versions DRIVER-OUT` prints the harness
versions the agent reported in the run's evidence. A Claude cell has no
`codex-home/sessions` dir and no stream jsonl; the detector must still reach
the session transcripts the driver recorded instead of stopping under set -e.
"""
import json
import os
import subprocess
import tempfile
import unittest

SCRIPT = os.path.join(os.path.dirname(__file__), "..", "validate-native-demo.sh")


def ran_versions(driver_out):
    result = subprocess.run(
        ["sh", SCRIPT, "--ran-versions", driver_out],
        stdin=subprocess.DEVNULL,
        capture_output=True,
        text=True,
        timeout=30,
    )
    return result.returncode, result.stdout.strip()


class CellRanVersions(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.run_dir = os.path.join(self.tmp.name, "run")
        self.evidence = os.path.join(self.run_dir, "evidence")
        os.makedirs(self.evidence)
        self.driver_out = os.path.join(self.tmp.name, "driver.out")
        with open(self.driver_out, "w") as f:
            f.write('{"evidence": "%s"}\n' % self.evidence)

    def write_summary(self, transcripts):
        with open(os.path.join(self.evidence, "summary.json"), "w") as f:
            json.dump({"facts": {"phase_transcripts": transcripts}}, f)

    def test_claude_cell_without_codex_home_reads_transcripts(self):
        transcript = os.path.join(self.tmp.name, "session.jsonl")
        with open(transcript, "w") as f:
            f.write('{"type":"user","version":"2.1.3"}\n')
        self.write_summary({"launch": transcript})
        self.assertEqual(ran_versions(self.driver_out), (0, "2.1.3"))

    def test_codex_sessions_still_read(self):
        sessions = os.path.join(self.run_dir, "codex-home", "sessions", "2026")
        os.makedirs(sessions)
        with open(os.path.join(sessions, "rollout.jsonl"), "w") as f:
            f.write('{"cli_version":"0.159.3"}\n')
        self.write_summary({})
        self.assertEqual(ran_versions(self.driver_out), (0, "0.159.3"))

    def test_missing_evidence_is_empty(self):
        os.rmdir(self.evidence)
        self.assertEqual(ran_versions(self.driver_out), (0, ""))


if __name__ == "__main__":
    unittest.main()
