"""Unit tests of the leak checker's pure classifier (ht-p03.131)."""
import importlib.machinery
import importlib.util
import os
import sys
import unittest

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


class ClassifyTests(unittest.TestCase):
    def test_run_id_tag_is_a_leak(self):
        p = proc(env={"HT_LEAK_RUN_ID": "r1"})
        self.assertEqual(rules(run([p], run_id="r1")), ["run-id"])
        self.assertEqual(run([p], run_id="r2"), [])

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


if __name__ == "__main__":
    unittest.main()
