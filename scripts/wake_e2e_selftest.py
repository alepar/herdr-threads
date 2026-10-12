"""Self-test for scripts/wake-e2e.py: its credential, cleanup and completeness paths, with fakes instead of Herdr,
the daemon and the harnesses. Run with `python3 scripts/wake-e2e.py --self-test`. Uses only fake credentials."""
from __future__ import annotations

import importlib.util
import json
import os
import shutil
import stat
import signal
import subprocess
import tempfile
import threading
import time
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
        r.process_grace_s = 0.6
        r.layout_attempted = workspace is not None
        r.run_id = "000000-test"
        r.auth_digest = None
        r.auth_refreshed = None
        r.codex_home = tmp / "codex-home"
        r.cleanup_report = None
        r.lock = threading.Lock()
        r.stopping = threading.Event()
        r.children, r.workers, r.start_attempted = set(), [], set()
        r.drain_timeout_s, r.child_term_grace_s = 1.0, 0.5
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

    def run(self, argv, env=None, timeout=60, tag=None, cleanup=False):
        assert cleanup, f"cleanup ran a non-cleanup helper: {argv}"
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

    def test_timed_out_health_probe_is_not_stopped(self):
        f = FakeRun(self.tmp)
        f.script["daemon health"] = lambda: ("timeout", "", "")
        self.assertTrue(any("still healthy" in p for p in f.r.cleanup()))

    def test_exiting_process_inside_the_grace_period_passes(self):
        f = FakeRun(self.tmp)
        answers = iter([(0, "424242\n", ""), (0, "424242\n", "")])
        f.script["pgrep"] = lambda: next(answers, (1, "", ""))
        self.assertEqual(f.r.cleanup(), [])

    def test_uncertain_workspace_create_is_found_by_label_and_closed(self):
        f = FakeRun(self.tmp, workspace=None)
        f.r.layout_attempted = True
        listed = {"result": {"workspaces": [{"workspace_id": "w77", "label": "wake-e2e-000000-test"}]}}
        lists = iter([(0, json.dumps(listed), ""), (0, json.dumps({"result": {"workspaces": []}}), "")])
        f.script["workspace list"] = lambda: next(lists)
        self.assertEqual(f.r.cleanup(), [])
        self.assertEqual(f.r.workspace, "w77")
        self.assertIn("workspace close", f.calls)

    def test_unchanged_auth_copy_is_deleted(self):
        f = FakeRun(self.tmp)
        f.r.codex_home.mkdir()
        (self.tmp / "src.json").write_bytes(FAKE_AUTH)
        f.r.auth_digest = W.provision_codex_auth(self.tmp / "src.json", f.r.codex_home / "auth.json")
        self.assertEqual(f.r.cleanup(), [])
        self.assertFalse((f.r.codex_home / "auth.json").exists())
        self.assertFalse(f.r.auth_refreshed)

    def test_refreshed_auth_copy_is_kept_and_reported(self):
        f = FakeRun(self.tmp)
        f.r.codex_home.mkdir()
        (self.tmp / "src.json").write_bytes(FAKE_AUTH)
        f.r.auth_digest = W.provision_codex_auth(self.tmp / "src.json", f.r.codex_home / "auth.json")
        (f.r.codex_home / "auth.json").write_bytes(b"refreshed")
        f.r.cleanup()
        self.assertTrue((f.r.codex_home / "auth.json").exists())
        self.assertTrue(f.r.auth_refreshed)
        self.assertEqual((self.tmp / "src.json").read_bytes(), FAKE_AUTH)

    def test_failed_process_scan_is_unknown_not_clean(self):
        f = FakeRun(self.tmp)
        f.script["pgrep"] = lambda: (2, "", "pgrep: bad option")
        self.assertTrue(any("scan failed" in p for p in f.r.cleanup()))

    def test_timed_out_process_scan_is_unknown_not_clean(self):
        f = FakeRun(self.tmp)
        f.script["pgrep"] = lambda: ("timeout", "", "")
        self.assertTrue(any("scan failed" in p for p in f.r.cleanup()))

    def test_keep_skips_cleanup_without_failing(self):
        f = FakeRun(self.tmp, keep=True)
        self.assertEqual(f.r.cleanup(), [])
        self.assertEqual(f.calls, [])
        self.assertEqual(f.r.cleanup_report["skipped"], "--keep")


def real_run(tmp):
    """A Run whose helpers are real processes (harmless `sleep` / `true`) but owns no workspace or daemon."""
    r = object.__new__(W.Run)
    r.args = SimpleNamespace(keep=False)
    r.root = tmp / "root"
    r.root.mkdir()
    r.lock = threading.Lock()
    r.cmdlog = open(tmp / "commands.jsonl", "a")
    r.stopping = threading.Event()
    r.children, r.workers, r.start_attempted = set(), [], set()
    r.drain_timeout_s, r.child_term_grace_s = 5.0, 1.0
    r.workspace, r.layout_attempted, r.daemon_attempted = None, False, False
    r.process_grace_s, r.stop_deadline_s = 0.5, 0.5
    r.auth_digest = r.auth_refreshed = r.cleanup_report = None
    return r


class Interruption(TempDir):
    """Cleanup begins while a recipient worker is blocked in a helper (like `herdr agent prompt --wait`)."""

    def setUp(self):
        super().setUp()
        self.r = real_run(self.tmp)
        self.addCleanup(self.r.cmdlog.close)
        self.started, self.after = [], []

    def worker(self):
        try:
            self.r.run(["sleep", "30"], tag="blocked-helper")
        finally:
            try:
                self.r.run(["true"], tag="after-stop")
                self.after.append("launched")
            except W.Stopped:
                self.after.append("refused")

    def start_worker(self, target):
        def quiet():  # a cut-off helper raises Stopped out of the worker, as in run_cases
            try:
                target()
            except W.Stopped:
                pass
        t = threading.Thread(target=quiet, daemon=True, name="cl-test")
        self.r.workers.append(t)
        self.r.start_attempted.add(t)
        t.start()
        end = time.time() + 5
        while not self.r.children and time.time() < end:
            time.sleep(0.02)
        with self.r.lock:
            self.started = list(self.r.children)
        self.assertEqual(len(self.started), 1)

    def test_cleanup_terminates_the_helper_drains_the_worker_and_refuses_new_helpers(self):
        self.start_worker(self.worker)
        problems = self.r.cleanup()
        self.assertEqual(problems, [])
        self.assertIsNotNone(self.started[0].poll())  # the blocked helper was terminated
        self.assertFalse(self.r.workers[0].is_alive())
        self.assertEqual(self.after, ["refused"])  # no child launched after cleanup began
        self.assertEqual(self.r.children, set())

    def test_a_worker_that_does_not_stop_fails_cleanup(self):
        def stubborn():
            try:
                self.r.run(["sleep", "30"])
            finally:
                time.sleep(3)  # ignores the stop for longer than the drain allows
        self.r.drain_timeout_s = 0.5
        self.start_worker(stubborn)
        self.assertTrue(any("did not stop" in p for p in self.r.cleanup()))

    def test_interrupt_in_the_calling_thread_kills_its_helper(self):
        """Ctrl-C/SIGTERM while the main thread waits on a helper (e.g. `workspace create`): no orphan."""
        marker = f"wake-e2e-selftest-{os.getpid()}-{time.time_ns()}"

        def interrupt(_signum, _frame):
            raise KeyboardInterrupt
        previous = signal.signal(signal.SIGALRM, interrupt)
        try:
            signal.setitimer(signal.ITIMER_REAL, 0.5)
            with self.assertRaises(KeyboardInterrupt):
                self.r.run(["/bin/sh", "-c", f"sleep 30; : {marker}"])
        finally:
            signal.setitimer(signal.ITIMER_REAL, 0)
            signal.signal(signal.SIGALRM, previous)
        self.assertEqual(self.r.children, set())
        self.assertEqual(subprocess.run(["pgrep", "-f", marker], capture_output=True).returncode, 1)

    def test_helper_cut_off_by_the_drain_reports_stopped_not_a_result(self):
        outcome = []

        def worker():
            try:
                outcome.append(self.r.run(["sleep", "30"]))
            except W.Stopped:
                outcome.append("stopped")
        self.start_worker(worker)
        self.assertEqual(self.r.cleanup(), [])
        self.assertEqual(outcome, ["stopped"])

    def test_a_helper_ignoring_sigterm_is_killed(self):
        def ignorer():
            self.r.run(["/bin/sh", "-c", "trap '' TERM; sleep 30"])
        self.start_worker(ignorer)
        self.assertEqual(self.r.cleanup(), [])
        self.assertIsNotNone(self.started[0].poll())


class PartialStartAndDrainErrors(TempDir):
    """Cleanup must reach every owned resource even when worker startup was cut short or the drain itself fails.
    Herdr and the daemon are faked at the helper boundary; the blocked worker helper is a real `sleep`."""

    def setUp(self):
        super().setUp()
        self.saved = {k: getattr(W.Run, k) for k in ("preflight", "scratch", "layout", "run", "drain")}
        self.saved_threading = W.threading
        self.addCleanup(self.restore)
        self.resource_calls, self.after_stop = [], []
        self.roots = []
        test = self
        real_run = W.Run.run

        def run(self_, argv, env=None, timeout=60, tag=None, cleanup=False):
            words = " ".join(str(a) for a in argv)
            if argv[0] == "herdr" or argv[0] == "/fake/herdr-threads":
                test.resource_calls.append(words.replace("/fake/herdr-threads", "ht"))
                if "workspace list" in words:
                    return 0, json.dumps({"result": {"workspaces": []}}), ""
                if "daemon health" in words:
                    return 1, "", "down"
                return 0, "", ""
            return real_run(self_, argv, env=env, timeout=timeout, tag=tag, cleanup=cleanup)

        def preflight(run_):
            test.roots.append(run_.root)
            run_.facts = {"run_id": run_.run_id}
            run_.bin, run_.endpoint = "/fake/herdr-threads", "/fake/sock"
            run_.drain_timeout_s, run_.child_term_grace_s, run_.process_grace_s = 5.0, 1.0, 0.5

        def scratch(run_):
            run_.daemon_attempted = True
            run_.codex_home.mkdir(parents=True)
            (test.tmp / "src-auth.json").write_bytes(FAKE_AUTH)
            run_.auth_digest = W.provision_codex_auth(test.tmp / "src-auth.json", run_.codex_home / "auth.json")
            test.auth_copy = run_.codex_home / "auth.json"

        def layout(run_):
            run_.layout_attempted, run_.workspace = True, "wfake"
            test.run_obj = run_

            def blocked(cases):
                try:
                    run_.run(["sleep", "30"], tag="blocked-helper")
                    run_.run(["true"], tag="after-stop")
                    test.after_stop.append("launched")
                except W.Stopped:
                    pass
            run_.recipients = [SimpleNamespace(name="cl-nudge", run_cases=blocked),
                               SimpleNamespace(name="cl-all", run_cases=lambda cases: None)]
        W.Run.run, W.Run.preflight, W.Run.scratch, W.Run.layout = run, preflight, scratch, layout

    def restore(self):
        for k, v in self.saved.items():
            setattr(W.Run, k, v)
        W.threading = self.saved_threading
        for root in self.roots:
            shutil.rmtree(root, True)

    def fail_second_start(self, exc):
        """Thread.start succeeds for the first worker, which then blocks in its helper, and raises for the second."""
        test = self
        real = threading.Thread

        class Thread(real):
            def start(self):
                if self.name == "cl-all":
                    end = time.time() + 5  # the first worker is inside its helper before the failure
                    while not test.run_obj.children and time.time() < end:
                        time.sleep(0.02)
                    raise exc
                super().start()
        W.threading = SimpleNamespace(**{**vars(threading), "Thread": Thread})

    def main(self):
        ev = self.tmp / "ev"
        rc = W.main(["--harness", "claude", "--cases", "invite/group", "--evidence-dir", str(ev)])
        return rc, json.loads((ev / "results.json").read_text())

    def assert_all_resources_attempted(self, doc):
        self.assertIn("herdr workspace close wfake", self.resource_calls)
        self.assertTrue(any(c.endswith("daemon stop") for c in self.resource_calls), self.resource_calls)
        self.assertFalse(self.auth_copy.exists())  # credential copy retired
        self.assertEqual(self.after_stop, [])      # no helper admitted after the stop
        self.assertEqual(self.run_obj.children, set())
        self.assertFalse(any(w.is_alive() for w in self.run_obj.workers))
        self.assertEqual(doc["facts"]["exit_code"], W.EXIT_FAIL)

    def test_failed_second_thread_start(self):
        self.fail_second_start(RuntimeError("can't start new thread"))
        rc, doc = self.main()
        self.assertEqual(rc, W.EXIT_FAIL)
        self.assert_all_resources_attempted(doc)
        self.assertEqual(doc["cleanup"]["problems"], [])

    def test_interrupt_between_thread_starts(self):
        self.fail_second_start(KeyboardInterrupt("signal 15"))
        rc, doc = self.main()
        self.assertEqual(rc, W.EXIT_FAIL)
        self.assert_all_resources_attempted(doc)
        self.assertEqual(doc["cleanup"]["problems"], [])

    def test_a_worker_interrupted_mid_start_is_still_joined(self):
        """Thread.start launched the thread, then the interrupt arrived before start() returned."""
        real = threading.Thread

        class Thread(real):
            def start(self):
                super().start()
                if self.name == "cl-nudge":
                    raise KeyboardInterrupt("signal 15 inside start()")
        W.threading = SimpleNamespace(**{**vars(threading), "Thread": Thread})
        rc, doc = self.main()
        self.assertEqual(rc, W.EXIT_FAIL)
        self.assertIn(self.run_obj.workers[0], self.run_obj.start_attempted)
        self.assert_all_resources_attempted(doc)
        self.assertEqual(doc["cleanup"]["problems"], [])

    def test_drain_joins_a_thread_that_finishes_starting_late(self):
        """The join retries while start() has not completed (the mid-start window), then joins the thread."""
        r = real_run(self.tmp)
        self.addCleanup(r.cmdlog.close)

        class LateStart:  # join() raises like a not-yet-started Thread for 0.2 s, then joins
            name = "cl-late"

            def __init__(self):
                self.ready, self.joined = time.time() + 0.2, False

            def join(self, timeout=None):
                if time.time() < self.ready:
                    raise RuntimeError("cannot join thread before it is started")
                self.joined = True

            def is_alive(self):
                return False
        late = LateStart()
        r.workers, r.start_attempted = [late], {late}
        self.assertEqual(r.drain(), [])
        self.assertTrue(late.joined)

    def test_a_drain_error_does_not_skip_owned_resources(self):
        def drain(run_):
            raise RuntimeError("drain exploded")
        W.Run.drain = drain
        # the real drain never runs in this test: stop the still-blocked helper even if an assertion fails
        self.addCleanup(lambda: self.saved["drain"](self.run_obj))
        self.fail_second_start(KeyboardInterrupt("signal 2"))
        rc, doc = self.main()
        self.assertEqual(rc, W.EXIT_FAIL)
        self.assertIn("herdr workspace close wfake", self.resource_calls)
        self.assertTrue(any(c.endswith("daemon stop") for c in self.resource_calls))
        self.assertFalse(self.auth_copy.exists())
        self.assertTrue(self.run_obj.stopping.is_set())  # admission closed before the drain ran
        self.assertTrue(any("drain raised" in p for p in doc["cleanup"]["problems"]))


class CoordinatorCalls(TempDir):
    """Coordinator traffic is a person seat of the local account: the `human` namespace, the coordinator's own
    pane, no cooperative claim and none of the calling agent's session variables or markers."""

    def test_coordinator_call_is_a_human_call_without_agent_identity(self):
        captured = []
        r = object.__new__(W.Run)
        r.state, r.endpoint, r.bin = self.tmp / "state", "/fake/sock", "/fake/herdr-threads"
        r.coords = {"main": {"pane": "w1:p1", "seat": "sAAAAAAAA"}}
        r.run = lambda argv, env=None, timeout=60, tag=None, cleanup=False: (
            captured.append((argv, env)) or (0, '{"result":{"data":"mAAAAAAAA"}}', ""))
        recipient = W.Recipient(r, "co-nudge", "codex", "nudge", "w1:p2", "sBBBBBBBB")
        markers = {var: "agent-value" for var in W.CALLER_SESSION_ENV}
        saved = {k: os.environ.get(k) for k in markers}
        os.environ.update(markers)
        try:
            recipient.coord("send", "tAAAAAAAA", "--body", "x")
        finally:
            for k, v in saved.items():
                if v is None:
                    os.environ.pop(k, None)
                else:
                    os.environ[k] = v
        argv, env = captured[0]
        self.assertEqual(argv[1], "human")
        self.assertFalse(any(a.startswith("--cooperative") for a in argv))
        self.assertEqual(env["HERDR_PANE_ID"], "w1:p1")
        self.assertFalse(set(W.CALLER_SESSION_ENV) & set(env), "agent identity leaked into a coordinator call")

    def test_every_registered_agent_env_marker_is_stripped(self):
        """Read from the adapters' source: a new harness marker must join CALLER_SESSION_ENV."""
        import re
        src = Path(W.REPO) / "src" / "harness"
        declared, literals = set(), 0
        for path in src.rglob("*.rs"):
            text = path.read_text(encoding="utf-8", errors="replace")
            names = re.findall(r'AgentEnvMarker\s*\{\s*name:\s*"([A-Z0-9_]+)"', text)
            declared |= set(names)
            # every struct literal must have been parsed: a declaration in another shape fails here, not silently
            literals += len(re.findall(r'(?<!struct )AgentEnvMarker\s*\{', text))
            self.assertEqual(len(names), len(re.findall(r'(?<!struct )AgentEnvMarker\s*\{', text)), path)
        self.assertTrue(literals >= 5 and {"CLAUDECODE", "CODEX_SANDBOX", "ANTIGRAVITY_AGENT"} <= declared, declared)
        self.assertEqual(sorted(declared - set(W.CALLER_SESSION_ENV)), [])

    def test_layout_records_person_seats_with_the_guard_kept(self):
        """layout(): `human me init` without --operator in each coordinator pane, no check-in or cooperative claim."""
        calls = []
        r = object.__new__(W.Run)
        r.state, r.endpoint, r.bin, r.cwd, r.run_id = self.tmp / "state", "/fake/sock", "/fake/ht", "/repo", "x"
        r.args = SimpleNamespace(harness=[])
        r.coords, r.layout_attempted, r.workspace = {}, False, None
        panes = iter(["w1:p2", "w1:p3"])
        seats = iter(["sCCCCCCCC", "sDDDDDDDD"])

        def run(argv, env=None, timeout=60, tag=None, cleanup=False):
            calls.append((argv, env))
            words = " ".join(argv)
            if "workspace create" in words:
                return 0, json.dumps({"result": {"workspace": {"workspace_id": "w1"}, "tab": {"tab_id": "w1:t1"},
                                                 "root_pane": {"pane_id": "w1:p1"}}}), ""
            if "tab create" in words:
                return 0, json.dumps({"result": {"root_pane": {"pane_id": next(panes)}}}), ""
            if "me init" in words:
                return 0, json.dumps({"result": {"seat": next(seats)}}), ""
            return 0, "{}", ""
        r.run = run
        r.layout()
        ht_calls = [(a, e) for a, e in calls if a[0] == "/fake/ht"]
        self.assertEqual([a[a.index("me"):] for a, _ in ht_calls], [["me", "init"], ["me", "init"]])
        self.assertTrue(all(a[1] == "human" for a, _ in ht_calls))
        self.assertEqual([e["HERDR_PANE_ID"] for _, e in ht_calls], ["w1:p1", "w1:p2"])
        self.assertFalse(any("--operator" in a or "check-in" in a or any(x.startswith("--cooperative") for x in a)
                             for a, _ in ht_calls))
        self.assertEqual({k: v["seat"] for k, v in r.coords.items()}, {"main": "sCCCCCCCC", "fresh": "sDDDDDDDD"})


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

    def test_interrupt_still_cleans_up_and_fails(self):
        cleaned = []
        W.Run.cleanup = lambda run: cleaned.append(True) or setattr(run, "cleanup_report", {"problems": []}) or []

        def layout(run):
            raise KeyboardInterrupt("signal 15")
        W.Run.layout = layout
        rc, doc = self.main()
        self.assertEqual((rc, cleaned), (W.EXIT_FAIL, [True]))
        self.assertEqual(doc["facts"]["exit_code"], W.EXIT_FAIL)

    def test_sigterm_while_joining_a_blocked_worker_drains_it(self):
        W.Run.cleanup = self.saved["cleanup"]  # the real cleanup: this run owns no workspace or daemon
        helpers = []

        def layout(run):
            def run_cases(cases):
                try:
                    run.run(["sleep", "30"], tag="blocked-helper")  # cut off: raises Stopped
                    helpers.append("returned normally after the drain")
                    run.run(["true"], tag="after-stop")
                    helpers.append("launched after stop")
                except W.Stopped:
                    pass
            recipient = SimpleNamespace(name="cl-nudge", run_cases=run_cases)
            run.recipients = [recipient]
            threading.Timer(0.5, os.kill, (os.getpid(), signal.SIGTERM)).start()
        W.Run.layout = layout
        before = signal.getsignal(signal.SIGTERM)
        rc, doc = self.main()
        self.assertEqual(rc, W.EXIT_FAIL)
        self.assertEqual(helpers, [])
        self.assertEqual(doc["cleanup"]["problems"], [])
        self.assertEqual(doc["completeness"], None)  # interrupted: no summary claims completeness
        self.assertIs(signal.getsignal(signal.SIGTERM), before)  # handlers restored

    def test_cleanup_raising_fails(self):
        def cleanup(run):
            raise RuntimeError("herdr gone")
        W.Run.cleanup = cleanup
        rc, doc = self.main()
        self.assertEqual(rc, W.EXIT_FAIL)
        self.assertIn("herdr gone", doc["cleanup"]["problems"][0])


if __name__ == "__main__":
    unittest.main()
