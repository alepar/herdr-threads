"""Self-test for scripts/wake-e2e.py: its credential, cleanup and completeness paths, with fakes instead of Herdr,
the daemon and the harnesses. Run with `python3 scripts/wake-e2e.py --self-test`. Uses only fake credentials."""
from __future__ import annotations

import importlib.util
import json
import os
import shutil
import stat
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

_SPEC = importlib.util.spec_from_file_location("wake_e2e", Path(__file__).resolve().parent / "wake-e2e.py")
W = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(W)

FAKE_AUTH = b'{"tokens":{"access_token":"fake-access","refresh_token":"fake-refresh"}}\n'


class TempDir(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="wake-e2e-selftest."))
        self.addCleanup(shutil.rmtree, self.tmp, True)


class CodexAuthCopy(TempDir):
    def setUp(self):
        super().setUp()
        self.source = self.tmp / "real-profile" / "auth.json"
        self.source.parent.mkdir()
        self.source.write_bytes(FAKE_AUTH)
        self.dest = self.tmp / "scratch" / "auth.json"
        self.dest.parent.mkdir()

    def native_save(self, payload):
        # What Codex's file store does on refresh: open with create + truncate + write, following links.
        with open(self.dest, "wb") as f:
            f.write(payload)

    def test_copy_is_a_private_regular_file(self):
        W.provision_codex_auth(self.source, self.dest)
        st = os.lstat(self.dest)
        self.assertTrue(stat.S_ISREG(st.st_mode))
        self.assertEqual(stat.S_IMODE(st.st_mode), 0o600)
        self.assertEqual(self.dest.read_bytes(), FAKE_AUTH)

    def test_native_refresh_never_writes_the_source(self):
        digest = W.provision_codex_auth(self.source, self.dest)
        self.native_save(b'{"tokens":{"access_token":"refreshed"}}\n')
        self.assertEqual(self.source.read_bytes(), FAKE_AUTH)
        self.assertNotEqual(W.file_sha256(self.dest), digest)  # the refresh is detectable

    def test_replaces_a_stale_symlink_without_following_it(self):
        self.dest.symlink_to(self.source)
        W.provision_codex_auth(self.source, self.dest)
        self.assertFalse(self.dest.is_symlink())
        self.native_save(b"truncated")
        self.assertEqual(self.source.read_bytes(), FAKE_AUTH)


class FakeRun:
    """A Run with scripted process results: each argv prefix maps to a callable returning (rc, out, err)."""

    def __init__(self, tmp, keep=False, workspace="w99", daemon_attempted=True):
        self.r = object.__new__(W.Run)
        r = self.r
        r.args = SimpleNamespace(keep=keep)
        r.root = tmp / "root"
        r.workspace = workspace
        r.daemon_attempted = daemon_attempted
        r.stop_deadline_s = 0.3
        r.cleanup_report = None
        r.bin, r.state, r.endpoint = "/fake/herdr-threads", tmp / "state", "/fake/sock"
        self.calls = []
        self.script = {
            "workspace close": lambda: (0, "", ""),
            "workspace list": lambda: (0, json.dumps({"result": {"workspaces": []}}), ""),
            "daemon stop": lambda: (0, "", ""),
            "daemon health": lambda: (1, "", "down"),
            "pgrep": lambda: (1, "", ""),
        }
        r.run = self.run

    def run(self, argv, env=None, timeout=60, tag=None):
        words = " ".join(str(a) for a in argv if not str(a).startswith(("/", "--")))
        for key, fn in self.script.items():
            if key in words:
                self.calls.append(key)
                return fn()
        raise AssertionError(f"unscripted command {argv}")


class Cleanup(TempDir):
    def test_clean_run_has_no_problems(self):
        f = FakeRun(self.tmp)
        self.assertEqual(f.r.cleanup(), [])
        self.assertEqual(f.calls[:2], ["workspace close", "workspace list"])
        self.assertIn("daemon stop", f.calls)

    def test_close_failure_still_stops_the_daemon_and_fails(self):
        f = FakeRun(self.tmp)
        f.script["workspace close"] = lambda: (1, "", "boom")
        f.script["workspace list"] = lambda: (0, json.dumps({"result": {"workspaces": [{"workspace_id": "w99"}]}}), "")
        problems = f.r.cleanup()
        self.assertTrue(any("still open" in p for p in problems), problems)
        self.assertIn("daemon stop", f.calls)

    def test_close_raising_still_stops_the_daemon(self):
        f = FakeRun(self.tmp)

        def boom():
            raise OSError("herdr gone")
        f.script["workspace close"] = boom
        f.r.cleanup()
        self.assertIn("daemon stop", f.calls)

    def test_unverifiable_workspace_closure_fails(self):
        f = FakeRun(self.tmp)
        f.script["workspace list"] = lambda: (1, "", "no server")
        self.assertTrue(any("not verified" in p for p in f.r.cleanup()))

    def test_daemon_still_healthy_after_deadline_fails(self):
        f = FakeRun(self.tmp)
        f.script["daemon stop"] = lambda: ("timeout", "", "")
        f.script["daemon health"] = lambda: (0, "ok", "")
        problems = f.r.cleanup()
        self.assertTrue(any("still healthy" in p for p in problems), problems)

    def test_uncertain_daemon_start_is_still_stopped(self):
        f = FakeRun(self.tmp, workspace=None, daemon_attempted=True)  # `daemon ensure` timed out before layout
        self.assertEqual(f.r.cleanup(), [])
        self.assertEqual(f.calls[0], "daemon stop")

    def test_leftover_owned_process_fails(self):
        f = FakeRun(self.tmp)
        f.script["pgrep"] = lambda: (0, "424242\n", "")
        self.assertTrue(any("424242" in p for p in f.r.cleanup()))

    def test_keep_skips_cleanup_without_failing(self):
        f = FakeRun(self.tmp, keep=True)
        self.assertEqual(f.r.cleanup(), [])
        self.assertEqual(f.calls, [])
        self.assertEqual(f.r.cleanup_report["skipped"], "--keep")


def row(recipient, case, result="PASS"):
    return {"recipient": recipient, "case": case, "result": result, "wake_on": "nudge", "harness": "claude"}


class Completeness(TempDir):
    def run_with(self, rows, recipients=("cl-nudge",)):
        return SimpleNamespace(results=rows, recipients=[SimpleNamespace(name=n) for n in recipients])

    def test_complete_all_pass(self):
        run = self.run_with([row("cl-nudge", "invite/group"), row("cl-nudge", "send-lazy/group")])
        self.assertTrue(W.summary(run, ["invite/group", "send-lazy/group"]))

    def test_missing_row_fails(self):
        run = self.run_with([row("cl-nudge", "invite/group")])
        self.assertFalse(W.summary(run, ["invite/group", "send-lazy/group"]))
        self.assertEqual(run.completeness["missing"], [("cl-nudge", "send-lazy/group")])

    def test_missing_recipient_fails(self):
        run = self.run_with([row("cl-nudge", "invite/group")], recipients=("cl-nudge", "co-nudge"))
        self.assertFalse(W.summary(run, ["invite/group"]))

    def test_duplicate_row_fails(self):
        run = self.run_with([row("cl-nudge", "invite/group"), row("cl-nudge", "invite/group")])
        self.assertFalse(W.summary(run, ["invite/group"]))

    def test_blocked_fails(self):
        run = self.run_with([row("cl-nudge", "busy-nudge/group", "BLOCKED")])
        self.assertFalse(W.summary(run, ["busy-nudge/group"]))

    def test_no_rows_fails(self):
        self.assertFalse(W.summary(self.run_with([], recipients=()), ["invite/group"]))


class MainExitCode(TempDir):
    """All cases pass but cleanup fails: the run must exit non-zero and say so in its evidence."""

    roots = []

    def setUp(self):
        super().setUp()
        self.saved = {k: getattr(W.Run, k) for k in ("preflight", "scratch", "layout", "cleanup")}
        self.addCleanup(lambda: [setattr(W.Run, k, v) for k, v in self.saved.items()])

        def preflight(run):
            run.facts = {"run_id": run.run_id}
            MainExitCode.roots.append(run.root)

        def layout(run):
            recipient = SimpleNamespace(name="cl-nudge")
            recipient.run_cases = lambda cases: [run.record(dict(row("cl-nudge", c), checks={})) for c in cases]
            run.recipients = [recipient]
        W.Run.preflight = preflight
        W.Run.scratch = lambda run: None
        W.Run.layout = layout

    def main(self):
        ev = self.tmp / "ev"
        rc = W.main(["--harness", "claude", "--cases", "invite/group", "--evidence-dir", str(ev)])
        return rc, json.loads((ev / "results.json").read_text())

    def tearDown(self):
        while self.roots:
            shutil.rmtree(self.roots.pop(), True)  # the run roots this test's Run objects made

    def test_cleanup_problem_fails_an_all_pass_run(self):
        W.Run.cleanup = lambda run: setattr(run, "cleanup_report", {"problems": ["daemon alive"]}) or ["daemon alive"]
        rc, doc = self.main()
        self.assertEqual(rc, W.EXIT_FAIL)
        self.assertEqual(doc["facts"]["exit_code"], W.EXIT_FAIL)
        self.assertEqual(doc["cleanup"]["problems"], ["daemon alive"])

    def test_clean_all_pass_run_exits_zero(self):
        W.Run.cleanup = lambda run: setattr(run, "cleanup_report", {"problems": []}) or []
        rc, doc = self.main()
        self.assertEqual(rc, W.EXIT_OK)
        self.assertEqual(doc["completeness"]["missing"], [])

    def test_cleanup_raising_fails(self):
        def cleanup(run):
            raise RuntimeError("herdr gone")
        W.Run.cleanup = cleanup
        rc, doc = self.main()
        self.assertEqual(rc, W.EXIT_FAIL)
        self.assertIn("herdr gone", doc["cleanup"]["problems"][0])


if __name__ == "__main__":
    unittest.main()
