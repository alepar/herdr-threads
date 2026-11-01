"""Unit tests of the leak checker's pure classifier (ht-p03.131)."""
import importlib.machinery
import importlib.util
import os
import signal
import subprocess
import sys
import time
import unittest
import uuid

SCRIPT = os.path.join(os.path.dirname(__file__), "..", "check-no-leaked-processes")
loader = importlib.machinery.SourceFileLoader("leakcheck", SCRIPT)
spec = importlib.util.spec_from_loader("leakcheck", loader)
leakcheck = importlib.util.module_from_spec(spec)
sys.modules["leakcheck"] = leakcheck
loader.exec_module(leakcheck)
Proc = leakcheck.Proc
PREFIXES = leakcheck.TEMP_PREFIXES


def proc(pid=100, ppid=1, argv0="/bin/sleep", command=None, env=None, cwd=None):
    return Proc(pid, ppid, argv0, command or argv0, env or {}, cwd)


def run(procs, root="/w", run_id=None, alive=None, exclude=()):
    alive = alive or {}
    return leakcheck.classify(
        procs,
        root=root,
        run_id=run_id,
        alive=lambda pid: alive.get(pid, False),
        exclude=set(exclude),
        temp_prefixes=PREFIXES,
    )


def rules(result):
    return [rule for _, rule in result]


def leak_candidates(run_tag):
    tag = {} if run_tag is None else {"HT_LEAK_RUN_ID": run_tag}
    return [
        proc(pid=101, env={**tag, "HT_TEST_OWNER": "4242"}),
        proc(pid=102, env={**tag, "HERDR_THREADS_TEST_OWNER_PID": "4242"}),
        proc(pid=103, argv0="/w/target/debug/herdr-threads", env=tag),
        proc(pid=104, cwd="/w/target/tmp/x", env=tag),
        proc(pid=105, command="/usr/local/bin/herdr server",
             env={**tag, "HOME": "/tmp/ih.ab12/home"}),
        proc(pid=106, command="/x/herdr-threads daemon run --state-dir /tmp/ht-x/state",
             env=tag),
    ]


class ClassifyTests(unittest.TestCase):
    def test_run_id_tag_is_a_leak(self):
        p = proc(env={"HT_LEAK_RUN_ID": "r1"})
        self.assertEqual(rules(run([p], run_id="r1")), ["run-id"])
        self.assertEqual(run([p], run_id="r2"), [])

    def test_run_id_scope_ignores_foreign_and_untagged_leaks(self):
        for tag in (None, "foreign-run", "r10"):
            for p in leak_candidates(tag):
                with self.subTest(tag=tag, pid=p.pid):
                    self.assertEqual(run([p], run_id="r1"), [])

    def test_run_id_scope_reports_matching_leaks(self):
        self.assertEqual(
            rules(run(leak_candidates("r1"), run_id="r1")),
            ["run-id", "run-id", "run-id", "run-id", "run-id", "run-id"],
        )

    def test_no_run_id_preserves_global_leak_rules(self):
        for tag in (None, "foreign-run", "r1"):
            with self.subTest(tag=tag):
                self.assertEqual(
                    rules(run(leak_candidates(tag))),
                    ["dead-owner", "dead-owner", "artifact", "artifact",
                     "orphan-temp", "orphan-temp"],
                )

    def test_dead_owner_tag_is_a_leak(self):
        for key in ("HT_TEST_OWNER", "HERDR_THREADS_TEST_OWNER_PID"):
            p = proc(env={key: "4242"})
            self.assertEqual(rules(run([p], alive={})), ["dead-owner"], key)
            self.assertEqual(run([p], alive={4242: True}), [], key)

    def test_build_artifact_is_a_leak(self):
        self.assertEqual(
            rules(run([proc(argv0="/w/target/debug/herdr-threads")], root="/w")),
            ["artifact"],
        )
        self.assertEqual(
            run([proc(argv0="/other/target/debug/herdr-threads")], root="/w"), []
        )
        self.assertEqual(
            rules(run([proc(cwd="/w/target/tmp/x")], root="/w")), ["artifact"]
        )

    def test_orphan_under_test_temp_is_a_leak(self):
        server = proc(
            argv0="/usr/local/bin/herdr",
            command="/usr/local/bin/herdr server",
            env={"HOME": "/tmp/ih.ab12/home"},
        )
        self.assertEqual(rules(run([server])), ["orphan-temp"])
        owned = proc(
            argv0="/usr/local/bin/herdr",
            command="/usr/local/bin/herdr server",
            env={"HOME": "/tmp/ih.ab12/home", "HT_TEST_OWNER": "77"},
        )
        self.assertEqual(run([owned], alive={77: True}), [])
        daemon = proc(
            argv0="/x/herdr-threads",
            command="/x/herdr-threads daemon run --state-dir /private/tmp/htlat-x/state",
        )
        self.assertEqual(rules(run([daemon])), ["orphan-temp"])

    def test_real_server_and_daemon_never_match(self):
        server = proc(
            argv0="/usr/local/bin/herdr",
            command="/usr/local/bin/herdr server",
            env={"HOME": "/Users/u"},
        )
        daemon = proc(
            argv0="/Users/u/.local/share/herdr-threads/bin/herdr-threads",
            command="/Users/u/.local/share/herdr-threads/bin/herdr-threads daemon run "
            "--state-dir /Users/u/.config/herdr/plugins/herdr-threads/state",
            env={"HOME": "/Users/u"},
        )
        self.assertEqual(run([server, daemon], root="/w", run_id="r1"), [])

    def test_self_and_ancestors_never_match(self):
        p = proc(pid=55, env={"HT_LEAK_RUN_ID": "r1"})
        self.assertEqual(run([p], run_id="r1", exclude=[55]), [])
        self.assertEqual(rules(run([p], run_id="r1")), ["run-id"])

    def test_scoped_scan_excludes_self_ancestors_and_explicit_pids(self):
        procs = [
            proc(pid=10, ppid=20, env={"HT_LEAK_RUN_ID": "r1"}),
            proc(pid=20, ppid=30, env={"HT_LEAK_RUN_ID": "r1"}),
            proc(pid=30, env={"HT_LEAK_RUN_ID": "r1"}),
            proc(pid=40, env={"HT_LEAK_RUN_ID": "r1"}),
            proc(pid=50, env={"HT_LEAK_RUN_ID": "r1"}),
        ]
        exclude = leakcheck.ancestors(procs, 10) | {40}
        self.assertEqual(
            [(p.pid, rule) for p, rule in run(procs, run_id="r1", exclude=exclude)],
            [(50, "run-id")],
        )


def core_proc(command, session=None, xdg=None, home="/home/u", pid=300, ppid=1):
    env = {"HOME": home}
    if session:
        env["HC_SESSION"] = session
    if xdg:
        env["XDG_STATE_HOME"] = xdg
        env["XDG_RUNTIME_DIR"] = xdg
    return proc(pid=pid, ppid=ppid, argv0=command.split(" ")[0], command=command, env=env)


class CoreTests(unittest.TestCase):
    def test_planted_herdr_core_serve_is_a_leak(self):
        p = core_proc("herdr-core serve", f"ht-test-{uuid.uuid4()}", "/tmp/ic.ab12/st")
        self.assertEqual(rules(run([p])), ["core-test"])

    def test_planted_herdr_core_hold_is_a_leak(self):
        p = core_proc("herdr-core hold", f"ht-test-{uuid.uuid4()}", "/tmp/ic.ab12/st")
        self.assertEqual(rules(run([p])), ["core-test"])

    def test_smoke_session_or_temp_xdg_alone_is_a_leak(self):
        smoke = core_proc("herdr-core serve", "ht-smoke-1")
        xdg = core_proc("herdr-core hold", None, "/tmp/ic.ab12/rt")
        self.assertEqual(rules(run([smoke, xdg])), ["core-test", "core-test"])

    def test_live_owner_is_not_a_leak(self):
        live = core_proc("herdr-core serve", "ht-test-1", ppid=77)
        live.env["HT_TEST_OWNER"] = "4242"
        self.assertEqual(run([live], alive={4242: True}), [])
        self.assertEqual(rules(run([live], alive={4242: False})), ["dead-owner"])

    def test_real_user_core_never_matches(self):
        for session in (None, "default", "mine-ht-test-x"):
            p = core_proc("/home/u/.local/bin/herdr-core serve", session, "/run/user/1000")
            self.assertEqual(run([p]), [])
            self.assertEqual(run([p], run_id="r1"), [])
        hold = core_proc("herdr-core hold", None, "/home/u/.local/state")
        self.assertEqual(run([hold]), [])

    def test_run_id_scope_ignores_foreign_core(self):
        p = core_proc("herdr-core serve", "ht-test-1", "/tmp/ic.ab12/st")
        self.assertEqual(run([p], run_id="r1"), [])


class CoreEndToEndTests(unittest.TestCase):
    def setUp(self):
        self.child = None

    def tearDown(self):
        if self.child and self.child.poll() is None:
            self.child.send_signal(signal.SIGKILL)
            self.child.wait()

    def test_renamed_stand_in_is_listed_by_collect_and_classify(self):
        env = {"PATH": os.environ.get("PATH", ""), "HC_SESSION": f"ht-test-{uuid.uuid4()}"}
        self.child = subprocess.Popen(
            ["bash", "-c", "exec -a 'herdr-core serve' sleep 60"], env=env,
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        pid = self.child.pid
        found = []
        for _ in range(50):
            procs = leakcheck.collect()
            found = [(p, r) for p, r in leakcheck.classify(
                procs, root=None, run_id=None, alive=leakcheck.pid_alive,
                exclude=leakcheck.ancestors(procs, os.getpid()),
                temp_prefixes=PREFIXES) if p.pid == pid]
            if found:
                break
            time.sleep(0.1)
        self.assertEqual([(p.pid, r) for p, r in found], [(pid, "core-test")])


if __name__ == "__main__":
    unittest.main()
