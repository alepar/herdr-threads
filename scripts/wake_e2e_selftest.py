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


class IdleWakeOracle(TempDir):
    """Run the real case/producer path with private fake CLI and recipient observations."""

    def run_case(self, case, wake_on, woke, delivered=True):
        calls, rows, prompts, windows = [], [], [], []
        r = object.__new__(W.Run)
        r.state, r.endpoint, r.bin = self.tmp / "state", "/fake/sock", "/fake/herdr-threads"
        r.run_id = "private-oracle"
        r.coords = {"main": {"pane": "w1:p1", "seat": "sAAAAAAAA"}}
        r.record = rows.append

        def run(argv, env=None, timeout=60, tag=None, cleanup=False):
            calls.append((argv, env))
            data = "mAAAAAAAA" if "send" in argv else "tAAAAAAAA"
            return 0, json.dumps({"result": {"data": data}}), ""
        r.run = run
        recipient = W.Recipient(r, "co-" + wake_on, "codex", wake_on, "w1:p2", "sBBBBBBBB")
        recipient.group = "tAAAAAAAA"
        recipient.wait_idle = lambda: None
        recipient.prompt = prompts.append
        recipient.joined = lambda thread: (True, "joined")
        recipient.pair_thread = lambda: "tBBBBBBBB"
        recipient.on_screen = lambda mid: delivered
        recipient.receipt = lambda mid: "acknowledged"

        def watch(window):
            windows.append(window)
            return 0.5 if woke else None
        recipient.watch_for_wake = watch
        quiet = []
        recipient.watch_quiet = lambda seconds: quiet.append(seconds)  # the trailing window: stays quiet
        recipient.case(case)
        self.assertEqual(len(rows), 1)
        self.assertEqual(len(quiet), 1)
        self.assertIsNone(rows[0]["checks"]["trailing_wake_s"])
        return rows[0], calls, prompts, windows

    def test_lazy_direct_invitation_is_passive_under_nudge(self):
        row_, calls, prompts, windows = self.run_case("invite/1:1", "nudge", woke=False)
        argv, env = calls[0]
        self.assertEqual(argv[1], "human")
        self.assertEqual(argv[argv.index("send"):],
                         ["send", "--seat", "sBBBBBBBB", "--body", "pair-open private-oracle", "--lazy"])
        self.assertEqual(env["HERDR_PANE_ID"], "w1:p1")
        self.assertFalse(any(a.startswith("--cooperative") or a == "--operator" for a in argv))
        self.assertEqual(row_["expect_wake"], False)
        self.assertEqual(row_["result"], "PASS")
        self.assertEqual(prompts, ["hello"])
        self.assertEqual(windows, [W.WAKE_WINDOW_S])
        self.assertEqual(row_["checks"]["opening_message_delivered"], True)

    def test_idle_matrix_preserves_group_invite_and_send_modes(self):
        for wake_on in ("nudge", "all"):
            for action in ("invite", "send-lazy", "send-nudge", "send-ack"):
                for channel in ("group", "1:1"):
                    case = action + "/" + channel
                    passive = wake_on == "nudge" and (action == "send-lazy" or case == "invite/1:1")
                    with self.subTest(case=case, wake_on=wake_on):
                        row_, calls, prompts, windows = self.run_case(case, wake_on, woke=not passive)
                        self.assertEqual(row_["expect_wake"], not passive)
                        self.assertEqual(row_["result"], "PASS")
                        self.assertEqual(prompts, ["hello"] if passive else [])
                        self.assertEqual(windows, [W.WAKE_WINDOW_S])
                        self.assertTrue(all(argv[1] == "human" for argv, _ in calls))
                        if case == "invite/group":
                            argv = calls[-1][0]
                            self.assertEqual(argv[argv.index("invite"):],
                                             ["invite", "tAAAAAAAA", "--seat", "sBBBBBBBB"])

    def test_unexpected_direct_wake_fails(self):
        row_, _, _, _ = self.run_case("invite/1:1", "nudge", woke=True)
        self.assertEqual(row_["result"], "FAIL")

    def test_missing_all_direct_wake_fails(self):
        row_, _, _, _ = self.run_case("invite/1:1", "all", woke=False)
        self.assertEqual(row_["result"], "FAIL")

    def test_passive_direct_opening_still_requires_delivery(self):
        row_, _, _, _ = self.run_case("invite/1:1", "nudge", woke=False, delivered=False)
        self.assertEqual(row_["checks"]["opening_message_delivered"], False)
        self.assertEqual(row_["result"], "FAIL")
# Pane text captured by the v0.9.1 Codex run 121116-d949 (busy-invite/1:1, co-all; anchors trimmed): the opener
# was not shown in the busy turn and a wake followed it once the turn had ended.
CODEX_BUSY_INVITE_PAIR = """\
\u203a Busy-turn test busy-invite/1:1 121116-d949: run the shell command `sleep 12`, then run the shell command
  `sleep 1`, then handle any herdr-threads items exactly as your standing instructions say, and finally print
  BUSY-DONE.

\u2022 I'll run both sleep commands, then handle any delivered items.

\u2022 Ran herdr-threads --state-dir /private/tmp/ht-wake-e2e.jt1uzg3f/state accept tSkTc0bQd
  \u2514 accepted
    value: "iFbz3zbB8"

\u2022 BUSY-DONE

  Worked for 23s \u2022 12:20


\u203a herdr-threads: attention pending; run herdr-threads inbox


\u2022 RECEIVED mngVD1bj4: pair-open-busy 121116-d949

  Worked for 2s \u2022 12:20
"""


class BusyTurnBoundary(unittest.TestCase):
    ANCHOR = "Busy-turn test busy-invite/1:1 121116-d949"

    def test_captured_post_turn_wake_is_after_the_turn_and_the_opener_is_not_in_it(self):
        in_turn, after, seen = W.busy_turn_split(CODEX_BUSY_INVITE_PAIR, self.ANCHOR)
        self.assertTrue(seen)
        self.assertEqual(W.wake_count(in_turn), 0)
        self.assertEqual(W.wake_count(after), 1)
        self.assertNotIn("mngVD1bj4", in_turn)
        self.assertIn("accept tSkTc0bQd", in_turn)

    def test_in_turn_delivery_and_claude_output_shape(self):
        text = (f"> {self.ANCHOR}: run ... and finally print BUSY-DONE.\n\u23fa Bash(sleep 12)\n"
                "  \u23bf delivered 1 message(s)\n\u23fa RECEIVED mOPEN: pair-open-busy\n\u23fa BUSY-DONE\n")
        in_turn, after, seen = W.busy_turn_split(text, self.ANCHOR)
        self.assertTrue(seen)
        self.assertIn("mOPEN", in_turn)
        self.assertEqual(after, "\n")

    def test_the_prompt_text_is_never_the_turn_end(self):
        for wrapped in ("and finally print\n  BUSY-DONE.\n", "and finally print BUSY-DONE.\n"):
            text = f"\u203a {self.ANCHOR}: run the shell commands, {wrapped}\n\u2022 Ran sleep 12\n"
            self.assertEqual(W.busy_turn_split(text, self.ANCHOR)[2], False, wrapped)
        self.assertIsNone(W.busy_turn_split("no prompt here", self.ANCHOR))

    def test_agent_end_line_shapes(self):
        prompt = f"\u203a {self.ANCHOR}: ... and finally print\n  BUSY-DONE.\n\n"
        for line in ("\u2022 BUSY-DONE", "  BUSY-DONE", "\u23fa BUSY-DONE.", "**BUSY-DONE**", "`BUSY-DONE`"):
            self.assertTrue(W.busy_turn_split(prompt + line + "\n", self.ANCHOR)[2], line)
        for line in ("\u2022 I'll run both sleeps, then print BUSY-DONE", "Done. BUSY-DONE"):
            self.assertFalse(W.busy_turn_split(prompt + line + "\n", self.ANCHOR)[2], line)
        echoed = (prompt + "\u2022 RECEIVED mEARLY\n"
                  + f"\u2022 Done with Busy-turn test {self.ANCHOR[15:]}; printing BUSY-DONE.\n\u2022 BUSY-DONE\n")
        in_turn, _after, seen = W.busy_turn_split(echoed, self.ANCHOR)
        self.assertTrue(seen)
        self.assertIn("mEARLY", in_turn)  # the turn starts at the prompt, not at the agent's echo of its tag


class BusyQuietWindow(unittest.TestCase):
    """A wake typed into the idle composer right after the busy turn ends (v0.9.1 Codex busy-invite/1:1, co-all) is
    seen by the quiet window and counted after the turn, and the opener it delivers is not in-turn delivery. The old
    driver's confirmed-idle wait swallowed that 2 s turn (self_started_turn_after_s null) and accepted the opener."""

    def setUp(self):
        saved = {k: getattr(W, k) for k in ("BUSY_AFTER_S", "SPACING_S")}
        self.addCleanup(lambda: [setattr(W, k, v) for k, v in saved.items()])
        W.BUSY_AFTER_S, W.SPACING_S = 3.0, 0.0
        self.rows = []

    def test_post_turn_wake_is_seen_and_the_opener_is_not_in_turn(self):
        t0 = []
        mid = "mOPENER01"
        tag = "busy-invite/1:1 000000-test"
        script = [  # (seconds after the busy prompt, screen text appended)
            (0.0, f"\u203a Busy-turn test {tag}: run the shell command `sleep 12`, ... and finally print BUSY-DONE.\n"),
            (0.5, "\u2022 Ran herdr-threads accept tPAIR\n  \u2514 accepted\n"),
            (4.0, "\u2022 BUSY-DONE\n\n  Worked for 4s\n"),
            (4.6, "\u203a herdr-threads: attention pending; run herdr-threads inbox\n"),
            (4.8, f"\u2022 RECEIVED {mid}: pair-open-busy\n"),
        ]
        turns = [(0.0, 4.0), (4.6, 6.6)]

        def since():
            return time.time() - t0[0] if t0 else -1.0

        r = object.__new__(W.Recipient)

        def herdr(*argv, **kw):
            if argv[:2] == ("agent", "prompt"):
                t0.append(time.time())
            return {}
        run = SimpleNamespace(run_id="000000-test", record=self.rows.append, stopping=threading.Event(), herdr=herdr)
        r.run, r.name, r.harness, r.wake_on, r.pane, r.seat = run, "co-all", "codex", "all", "w9:p1", "sREC"
        r.last_wake, r.group, r.pair = 0.0, "tGROUP", None
        r.status = lambda: "working" if any(a <= since() < b for a, b in turns) else "idle"
        r.screen = lambda lines=400: "".join(text for at, text in script if since() >= at)
        r.send = lambda mode, body, direct, role="main": mid
        r.pair_thread = lambda role="main": "tPAIR"
        r.joined = lambda thread, role="main": (True, "joined")
        r.busy_case("busy-invite/1:1")
        checks = self.rows[0]["checks"]
        self.assertIsNotNone(checks["self_started_turn_after_s"], checks)
        self.assertIs(checks.get("opening_message_delivered_in_turn"), False, checks)
        self.assertEqual((checks["wake_prompts_in_busy_turn"], checks["wake_prompts_after_busy_turn"]), (0, 1))
        self.assertEqual(self.rows[0]["result"], "FAIL")


class FakeAgent:
    """A recipient whose turns follow a fake daemon: a wake is reserved no sooner than `guard` after the previous
    reservation, and a pair opener that the accept turn does not show is owed a follow-up wake (the v0.9.1 Codex gap)."""

    def __init__(self, guard, opener_in_accept_turn):
        self.guard, self.in_accept = guard, opener_in_accept_turn
        self.turns, self.shown, self.reserved = [], {}, -1e9
        self.lock = threading.Lock()

    def wake(self, after=0.0, show=None, length=0.15):
        with self.lock:
            at = max(time.time() + after, self.reserved + self.guard)
            self.reserved = at
            self.turns.append((at, at + length))
            if show:
                self.shown[show] = at + 0.05

    def status(self):
        now = time.time()
        return "working" if any(a <= now < b for a, b in self.turns) else "idle"

    def visible(self, mid):
        return mid in self.shown and time.time() >= self.shown[mid]


class CaseSpacing(unittest.TestCase):
    """A follow-up wake owed by one case fails that case, and the next case is measured after it, not against the
    guard of a wake it did not cause (v0.9.1 Codex run: invite/1:1 then send-lazy/group, late_wake 11.58 s)."""

    def setUp(self):
        saved = {k: getattr(W, k) for k in ("SPACING_S", "WAKE_WINDOW_S")}
        self.addCleanup(lambda: [setattr(W, k, v) for k, v in saved.items()])
        # The 35 s spacing / 6 s window for a 30 s guard, scaled to a 5 s guard: still long against the driver's fixed
        # ~1 s idle confirmations, as the real guard is, so the owed wake lands after the case has settled.
        W.SPACING_S, W.WAKE_WINDOW_S = 6.0, 0.4
        self.rows = []

    def recipient(self, agent):
        r = object.__new__(W.Recipient)
        run = SimpleNamespace(run_id="000000-test", record=self.rows.append, stopping=threading.Event())
        r.run, r.name, r.harness, r.wake_on, r.pane, r.seat = run, "co-all", "codex", "all", "w9:p1", "sREC"
        r.last_wake, r.group, r.pair = 0.0, "tGROUP", None
        r.status = agent.status
        r.on_screen = lambda mid, tries=15: agent.visible(mid)
        r.joined = lambda thread, role="main": (True, "joined")
        r.pair_thread = lambda role="main": "tPAIR"
        r.prompt = lambda text, expect=None: self.fail(f"unexpected prompt {text!r}")

        def send(mode, body, direct, role="main"):
            mid = f"m{len(self.rows)}{mode}"
            if body.startswith("pair-open"):  # invitation wakes; the lazy opener rides the accept turn or a later wake
                agent.wake(show=mid if agent.in_accept else None)
                if not agent.in_accept:
                    agent.wake(show=mid)
            else:
                agent.wake(show=mid)
            return mid
        r.send = send
        return r

    def run_pair_then_lazy(self, opener_in_accept_turn):
        agent = FakeAgent(guard=5.0, opener_in_accept_turn=opener_in_accept_turn)
        r = self.recipient(agent)
        r.case("invite/1:1")
        r.case("send-lazy/group")
        return {row["case"]: row for row in self.rows}

    def test_the_owed_follow_up_wake_fails_its_own_case_and_not_the_next(self):
        rows = self.run_pair_then_lazy(opener_in_accept_turn=False)
        self.assertEqual(rows["send-lazy/group"]["result"], "PASS", rows["send-lazy/group"])
        self.assertEqual(rows["invite/1:1"]["result"], "FAIL")
        self.assertIsNotNone(rows["invite/1:1"]["checks"].get("trailing_wake_s"))
        self.assertFalse(rows["invite/1:1"]["checks"]["opening_message_delivered"])

    def test_opener_in_the_accept_turn_passes_both(self):
        rows = self.run_pair_then_lazy(opener_in_accept_turn=True)
        self.assertEqual([rows[c]["result"] for c in ("invite/1:1", "send-lazy/group")], ["PASS", "PASS"], rows)
        self.assertIsNone(rows["invite/1:1"]["checks"].get("trailing_wake_s"))


class RunRoot(TempDir):
    """The run root lives under a short, existing temp dir on every platform (Linux has no /private/tmp)."""

    def test_platform_roots(self):
        self.assertEqual(W.short_tmp_root("darwin"), Path("/private/tmp"))
        self.assertEqual(W.short_tmp_root("linux"), Path("/tmp"))

    def test_this_hosts_root_exists(self):
        self.assertTrue(W.short_tmp_root().is_dir(), W.short_tmp_root())

    def test_a_run_creates_its_private_root_there(self):
        # Record exactly the root this Run creates (never another run's) and remove it even if Run raises after
        # mkdtemp: the cleanup is registered before the Run exists.
        made, real = [], W.tempfile.mkdtemp

        def mkdtemp(*args, **kwargs):
            made.append(real(*args, **kwargs))
            return made[-1]
        W.tempfile.mkdtemp = mkdtemp
        self.addCleanup(setattr, W.tempfile, "mkdtemp", real)
        self.addCleanup(lambda: [shutil.rmtree(p, True) for p in made])
        r = W.Run(SimpleNamespace(evidence_dir=None, bin="/bin/sh"))
        self.assertEqual([str(r.root)], made)
        self.addCleanup(r.cmdlog.close)
        self.assertEqual(r.root.parent, W.short_tmp_root())
        self.assertTrue(r.root.name.startswith("ht-wake-e2e."))
        self.assertEqual(stat.S_IMODE(r.root.stat().st_mode), 0o700)


FIXTURES = Path(__file__).resolve().parent / "wake_e2e_fixtures"
FAKE_API_KEY = "sk-ant-api03-FAKEFAKEFAKEFAKE-selftest-only-0123456789"
# launch_line() of the macOS mode as the driver produced it before the linux mode existed (dc575a28), same inputs.
MACOS_CLAUDE_LINE = ('unset CLAUDECODE CLAUDE_CODE_ENTRYPOINT; PATH=/private/tmp/ht-wake-e2e.X/bin:"$PATH" '
                     'CLAUDE_CONFIG_DIR=/Users/me/.claude-real CLAUDE_CODE_PLUGIN_DIRS=/private/tmp/ht-wake-e2e.X/state/'
                     'claude-mod/herdr-threads HERDR_AGENT=claude command claude --setting-sources project,local '
                     '--settings /private/tmp/ht-wake-e2e.X/claude-config/settings.json --permission-mode auto')
MACOS_CODEX_LINE = ('PATH=/private/tmp/ht-wake-e2e.X/bin:"$PATH" HERDR_AGENT=codex CODEX_HOME=/private/tmp/ht-wake-e2e.X/'
                    'codex-home command codex --no-daemon -C /repo --dangerously-bypass-hook-trust -c '
                    'check_for_update_on_startup=false --approve-for-me')


class Env:
    """Temporarily set (value) or remove (None) environment variables."""

    def __init__(self, **values):
        self.values, self.saved = values, {}

    def __enter__(self):
        for k, v in self.values.items():
            self.saved[k] = os.environ.get(k)
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v

    def __exit__(self, *exc):
        for k, v in self.saved.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v


def parsed_args(argv):
    """The argparse result main() builds (and resolves) for argv, without starting a run."""
    captured = {}
    saved = {k: getattr(W.Run, k) for k in ("__init__", "preflight", "cleanup")}
    saved_evidence = W.write_evidence

    def init(run, args):
        captured["args"] = args
        run.args, run.ev, run.cleanup_report = args, None, None

    def preflight(run):
        raise W.Refused("self-test: stop after parsing")
    W.Run.__init__, W.Run.preflight, W.Run.cleanup = init, preflight, lambda run: []
    W.write_evidence = lambda run, rc: None
    try:
        import contextlib
        import io
        with contextlib.redirect_stderr(io.StringIO()), contextlib.redirect_stdout(io.StringIO()):
            W.main(argv)
    finally:
        for k, v in saved.items():
            setattr(W.Run, k, v)
        W.write_evidence = saved_evidence
    return captured["args"]


def launch_run(args):
    r = object.__new__(W.Run)
    r.args = args
    root = Path("/private/tmp/ht-wake-e2e.X")
    r.bindir, r.claude_cfg, r.state, r.codex_home = root / "bin", root / "claude-config", root / "state", root / "codex-home"
    r.cwd = "/repo"
    return r


class HarnessMode(TempDir):
    """--harness-mode: macos keeps the pre-linux behaviour exactly; linux gets a scratch Claude config and env
    credentials, never the real Claude config."""

    def test_default_follows_the_platform(self):
        self.assertEqual(W.default_harness_mode("darwin"), "macos")
        self.assertEqual(W.default_harness_mode("linux"), "linux")

    def test_macos_launch_lines_are_unchanged(self):
        with Env(CLAUDE_CONFIG_DIR="/Users/me/.claude-real"):
            args = parsed_args(["--harness-mode", "macos"])
        self.assertEqual(args.claude_config_dir, "/Users/me/.claude-real")
        self.assertEqual(args.codex_args, "--approve-for-me")
        r = launch_run(args)
        self.assertEqual(r.launch_line("claude"), MACOS_CLAUDE_LINE)
        self.assertEqual(r.launch_line("codex"), MACOS_CODEX_LINE)

    def test_linux_launch_lines_use_the_scratch_config_and_bypass_the_codex_sandbox(self):
        with Env(CLAUDE_CONFIG_DIR="/home/me/.claude-real"):  # a real dir in the environment is ignored
            args = parsed_args(["--harness-mode", "linux"])
        self.assertIsNone(args.claude_config_dir)
        r = launch_run(args)
        claude = r.launch_line("claude")
        self.assertIn(" CLAUDE_CONFIG_DIR=/private/tmp/ht-wake-e2e.X/claude-config ", claude)
        self.assertNotIn("claude-real", claude)
        self.assertEqual(claude, MACOS_CLAUDE_LINE.replace("/Users/me/.claude-real",
                                                           "/private/tmp/ht-wake-e2e.X/claude-config"))
        self.assertEqual(r.launch_line("codex"), MACOS_CODEX_LINE.replace(
            "--approve-for-me", "--dangerously-bypass-approvals-and-sandbox"))

    def test_linux_refuses_a_real_claude_config_dir(self):
        with self.assertRaises(SystemExit):
            import contextlib
            import io
            with contextlib.redirect_stderr(io.StringIO()):
                W.main(["--harness-mode", "linux", "--claude-config-dir", "/home/me/.claude"])

    def test_explicit_codex_args_win_in_both_modes(self):
        for mode in W.HARNESS_MODES:
            self.assertEqual(parsed_args(["--harness-mode", mode, "--codex-args", "-s read-only"]).codex_args,
                             "-s read-only")

    def scratch_run(self, mode, harness=("claude", "codex")):
        calls = []
        r = object.__new__(W.Run)
        r.args = SimpleNamespace(harness=list(harness), harness_mode=mode, codex_auth_home=str(self.tmp / "src"))
        r.root = self.tmp / "root"
        r.state, r.home, r.claude_cfg = r.root / "state", r.root / "home", r.root / "claude-config"
        r.codex_home, r.bindir, r.bin, r.endpoint, r.cwd = r.root / "codex-home", r.root / "bin", "/fake/ht", "/s", "/repo"
        r.daemon_attempted = False
        (self.tmp / "src").mkdir()
        (self.tmp / "src" / "auth.json").write_bytes(FAKE_AUTH)
        r.ht = lambda *argv, env=None, **kw: calls.append((argv, dict(env or {}))) or ""
        r.scratch()
        return r, calls

    def test_macos_scratch_is_unchanged(self):
        with Env(ANTHROPIC_API_KEY=FAKE_API_KEY):
            r, calls = self.scratch_run("macos")
        setup_env = {"HOME": str(r.home), "CLAUDE_CONFIG_DIR": str(r.claude_cfg), "CODEX_HOME": str(r.codex_home),
                     "CLAUDE_CODE_PLUGIN_DIRS": ""}
        self.assertEqual(calls, [(("daemon", "ensure"), {}), (("daemon", "health"), {}),
                                 (("setup", "claude", "--with-permissions"), setup_env),
                                 (("setup", "codex", "--with-permissions"), setup_env)])
        self.assertEqual(sorted(p.name for p in r.claude_cfg.iterdir()), [])  # nothing seeded: the real dir is used
        self.assertEqual((r.codex_home / "config.toml").read_text(),
                         '\n\n[projects."/repo"]\ntrust_level = "trusted"\n\n[notice]\nhide_rate_limit_model_nudge = true\n')

    def test_linux_scratch_seeds_exactly_the_first_run_keys(self):
        with Env(ANTHROPIC_API_KEY=None, CLAUDE_CODE_OAUTH_TOKEN="fake-oauth-token"):
            r, calls = self.scratch_run("linux")
        self.assertEqual([c[0][:2] for c in calls], [("daemon", "ensure"), ("daemon", "health"),
                                                     ("setup", "claude"), ("setup", "codex")])
        self.assertEqual(calls[2][1]["CLAUDE_CONFIG_DIR"], str(r.claude_cfg))  # hooks go into the scratch config
        path = r.claude_cfg / ".claude.json"
        self.assertEqual(json.loads(path.read_text()),
                         {"hasCompletedOnboarding": True, "projects": {"/repo": {"hasTrustDialogAccepted": True}}})
        self.assertEqual(stat.S_IMODE(os.lstat(path).st_mode), 0o600)
        self.assertNotIn("fake-oauth-token", path.read_text())  # an OAuth token needs no approval: never written

    def test_linux_api_key_is_approved_by_its_last_20_characters(self):
        state = W.claude_scratch_state("/repo", {"ANTHROPIC_API_KEY": FAKE_API_KEY})
        self.assertEqual(state, {"hasCompletedOnboarding": True,
                                 "projects": {"/repo": {"hasTrustDialogAccepted": True}},
                                 "customApiKeyResponses": {"approved": [FAKE_API_KEY[-20:]], "rejected": []}})

    def test_linux_seed_merges_and_never_follows_a_link(self):
        cfg = self.tmp / "cfg"
        cfg.mkdir()
        outside = self.tmp / "outside.json"
        outside.write_text("{}")
        (cfg / ".claude.json").symlink_to(outside)
        W.seed_claude_config(cfg, "/repo", {})
        self.assertEqual(outside.read_text(), "{}")
        self.assertFalse((cfg / ".claude.json").is_symlink())
        (cfg / ".claude.json").write_text(json.dumps({"userID": "u", "projects": {"/repo": {"x": 1}, "/o": {}}}))
        W.seed_claude_config(cfg, "/repo", {})
        self.assertEqual(json.loads((cfg / ".claude.json").read_text()),
                         {"userID": "u", "hasCompletedOnboarding": True,
                          "projects": {"/repo": {"x": 1, "hasTrustDialogAccepted": True}, "/o": {}}})


class LinuxPreflight(TempDir):
    """The linux mode refuses before any process starts when no Claude credential is in the environment."""

    def preflight(self, mode, harness=("claude",), **env):
        r = object.__new__(W.Run)
        r.args = SimpleNamespace(harness=list(harness), harness_mode=mode, codex_auth_home=None)
        r.endpoint, r.bin, r.run_id, r.root = "/fake/sock", "/bin/sh", "000000-test", self.tmp
        started = []
        r.run = lambda argv, **kw: started.append(argv) or (0, "x 1.0", "")
        values = {"HERDR_ENV": "1", "CLAUDE_CODE_OAUTH_TOKEN": None, "ANTHROPIC_API_KEY": None}
        values.update(env)
        path = self.tmp / "path"
        path.mkdir(exist_ok=True)
        for tool in ("herdr", "claude", "codex"):
            (path / tool).write_text("#!/bin/sh\nexit 0\n")
            (path / tool).chmod(0o700)
        values["PATH"] = str(path)
        import contextlib
        import io
        with Env(**values), contextlib.redirect_stdout(io.StringIO()):
            try:
                r.preflight()
                return None, started
            except W.Refused as e:
                return str(e), started

    def test_linux_without_credential_env_refuses_before_any_process(self):
        why, started = self.preflight("linux")
        self.assertIn("CLAUDE_CODE_OAUTH_TOKEN or ANTHROPIC_API_KEY", why)
        self.assertEqual(started, [])

    def test_empty_credential_env_is_no_credential(self):
        why, _ = self.preflight("linux", CLAUDE_CODE_OAUTH_TOKEN="")
        self.assertIn("no Claude credential", why)

    def test_linux_with_either_credential_passes(self):
        for var in W.CLAUDE_CREDENTIAL_ENV:
            why, started = self.preflight("linux", **{var: "fake"})
            self.assertIsNone(why, var)
            self.assertTrue(started)

    def test_codex_only_needs_no_claude_credential(self):
        why, _ = self.preflight("linux", harness=())
        self.assertIsNone(why)

    def test_macos_does_not_ask_for_credential_env(self):
        why, _ = self.preflight("macos")
        self.assertIsNone(why)


class StartupScreens(unittest.TestCase):
    """Captured screens (scripts/wake_e2e_fixtures/, Claude Code 2.1.296 and Codex 0.162.1 in the runner image):
    a first-run screen is refused and never typed into; a composer is not."""

    def screen(self, name):
        return (FIXTURES / name).read_text()

    def test_claude_first_run_screens_are_refused(self):
        # the theme picker and the login screen report `idle`: only the screen text can stop the driver
        for name, status, why in (("claude-theme-picker.txt", "idle", "startup dialog"),
                                  ("claude-trust-folder.txt", "blocked", "startup dialog"),
                                  ("claude-custom-api-key.txt", "blocked", "startup dialog"),
                                  ("claude-login-method.txt", "idle", "is not logged in"),
                                  ("claude-composer-not-logged-in.txt", "idle", "is not logged in")):
            got = W.startup_refusal(self.screen(name), status, "linux")
            self.assertTrue(got and got.startswith(why), (name, got))

    def test_screens_are_refused_from_text_alone(self):
        for name in ("claude-theme-picker.txt", "claude-trust-folder.txt", "claude-custom-api-key.txt"):
            self.assertTrue(W.DIALOG.search(self.screen(name)), name)

    def test_theme_picker_prompt_is_outside_the_last_400_characters(self):
        lines = [ln.strip() for ln in self.screen("claude-theme-picker.txt").splitlines() if ln.strip()]
        self.assertIsNone(W.DIALOG.search(" | ".join(lines)[-400:]))  # why the whole screen is checked

    def test_composers_are_not_refused(self):
        for name in ("claude-composer-api-key.txt", "claude-composer-oauth-token.txt", "codex-composer-bypass.txt"):
            self.assertIsNone(W.startup_refusal(self.screen(name), "idle", "linux"), name)

    def test_login_hint_names_the_mode(self):
        screen = self.screen("claude-login-method.txt")
        self.assertIn("CLAUDE_CODE_OAUTH_TOKEN", W.startup_refusal(screen, "idle", "linux"))
        self.assertIn("--claude-config-dir", W.startup_refusal(screen, "idle", "macos"))

    def test_start_types_nothing_into_a_first_run_screen(self):
        typed = []
        screen = self.screen("claude-theme-picker.txt")

        def herdr(*argv, **kw):
            words = " ".join(argv)
            if words.startswith("pane read"):
                return {"raw": "$ "}
            if words.startswith("agent get"):
                return {"result": {"agent": {"agent": "claude", "agent_status": "idle"}}}
            if words.startswith("agent read"):
                return {"raw": screen}
            typed.append(argv)  # pane run (the launch line) and any prompt
            return {}
        run = SimpleNamespace(herdr=herdr, launch_line=lambda h: "launch", args=SimpleNamespace(harness_mode="linux"),
                              coord_seats=lambda: "s")
        rec = W.Recipient(run, "cl-nudge", "claude", "nudge", "w1:p2", "sAAAAAAAA")
        with self.assertRaisesRegex(RuntimeError, "startup dialog"):
            rec.start()
        self.assertEqual([a[:2] for a in typed], [("pane", "run")])


class CodexStartOrder(unittest.TestCase):
    """Two Codex recipients share one scratch CODEX_HOME; in the linux mode their first starts never overlap (a
    concurrent state DB migration made one Codex exit in the runner). macOS keeps its concurrent starts."""

    def starts(self, mode):
        events, lock = [], threading.Lock()
        polls = {}
        screen = (FIXTURES / "claude-trust-folder.txt").read_text()  # stops each start() right after detection

        def herdr(*argv, **kw):
            words = " ".join(argv)
            pane = argv[2] if len(argv) > 2 else None
            if words.startswith("pane read"):
                return {"raw": "$ "}
            if words.startswith("pane run"):
                with lock:
                    events.append(("run", pane))
                return {}
            if words.startswith("agent get"):
                with lock:
                    polls[pane] = polls.get(pane, 0) + 1
                    up = polls[pane] > 2
                    if up and ("up", pane) not in events:
                        events.append(("up", pane))
                return {"result": {"agent": {"agent": "codex" if up else None, "agent_status": "idle"}}}
            if words.startswith("agent read"):
                return {"raw": screen}
            return {}
        run = SimpleNamespace(herdr=herdr, launch_line=lambda h: "launch", args=SimpleNamespace(harness_mode=mode),
                              coord_seats=lambda: "s", codex_start=threading.Lock())
        recipients = [W.Recipient(run, n, "codex", n[3:], pane, "sAAAAAAAA")
                      for n, pane in (("co-nudge", "w1:p2"), ("co-all", "w1:p3"))]
        errors = []

        def start(r):
            try:
                r.start()
            except RuntimeError as e:
                errors.append(str(e))
        threads = [threading.Thread(target=start, args=(r,)) for r in recipients]
        for t in threads:
            t.start()
        for t in threads:
            t.join(30)
        self.assertEqual(len(errors), 2, errors)
        return [e[0] for e in events]

    def test_linux_codex_starts_one_at_a_time(self):
        self.assertEqual(self.starts("linux"), ["run", "up", "run", "up"])

    def test_macos_codex_starts_stay_concurrent(self):
        self.assertEqual(self.starts("macos")[:2], ["run", "run"])


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
