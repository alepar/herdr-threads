"""Offline checks for R13's shell phase (W9-5). No Herdr server, daemon or model is started.

Run: python3 -m unittest discover -s tests/native/recovery -p 'test_r13.py'
"""

from pathlib import Path
import sys
import tempfile
import types
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import run_recovery


class R13ShellPhase(unittest.TestCase):
    def scenario(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        suite = types.SimpleNamespace(evidence_dir=Path(tmp.name), run_dir=Path(tmp.name))
        return run_recovery.Scenario(suite, "R13", "cooperative idle wake", "shell never prompted"), suite

    def run_check(self, rows):
        """Run the Suite's real shell-phase check over `rows` (successive wake_work reads) and return the scenario."""
        s, _ = self.scenario()
        suite = run_recovery.Suite.__new__(run_recovery.Suite)
        reads = iter(rows)
        suite.wake_rows = lambda seat: next(reads, rows[-1])
        run_recovery.Suite.check_shell_phase_wake(suite, s, "seat-w", wait_s=0)
        return s

    def test_r13_requires_unsafe_row(self):
        for name, rows in (("no wake_work row at all", [None]),
                           ("row without an outcome (no attempt recorded)", [{"last_outcome": None, "last_warning_seq": None,
                                                                              "reservation_id": None}]),
                           ("a submitted outcome (the shell was prompted)", [{"last_outcome": "submitted"}]),
                           ("an unavailable outcome", [{"last_outcome": "unavailable"}])):
            with self.subTest(name):
                s = self.run_check(rows)
                self.assertEqual(s.finish()["status"], "FAIL")
        s = self.run_check([{"last_outcome": "unsafe", "last_warning_seq": None, "reservation_id": None}])
        self.assertEqual(s.finish()["status"], "PASS")

    def test_pure_check_names_the_expected_outcome(self):
        ok, detail = run_recovery.shell_phase_wake_check({"last_outcome": "submitted"})
        self.assertFalse(ok)
        self.assertIn("'unsafe'", detail)
        self.assertEqual(run_recovery.shell_phase_wake_check({"last_outcome": "unsafe"})[0], True)

    def test_row_appearing_after_the_first_read_is_awaited(self):
        s, _ = self.scenario()
        suite = run_recovery.Suite.__new__(run_recovery.Suite)
        reads = iter([None, {"last_outcome": "unsafe"}])
        suite.wake_rows = lambda seat: next(reads)
        run_recovery.Suite.check_shell_phase_wake(suite, s, "seat-w", wait_s=5)
        self.assertEqual(s.finish()["status"], "PASS")


if __name__ == "__main__":
    unittest.main()
