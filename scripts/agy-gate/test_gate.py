"""Offline tests for gate.py, observe.py and the boot flow's scenario (no agy, Herdr or network)."""
import contextlib
import io
import json
import os
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent / "agy-fake-gemini"))
sys.path.insert(0, str(HERE.parent / "agy-sandbox"))
import gate  # noqa: E402
import observe  # noqa: E402
import scenario  # noqa: E402

FLOW_MODULE = textwrap.dedent('''
    from gate import Flow
    def run(world):
        world.check("ran", True)
    FLOW = Flow(name=%r, requires=%r, run=run)
''')


def write_flows(directory, **flows):
    for name, requires in flows.items():
        (Path(directory) / (name + ".py")).write_text(FLOW_MODULE % (name, requires))


class FakeWorld(object):
    """Stands in for agyenv.World: records checks, builds nothing."""
    built = []

    def __init__(self, run_id, owner, binary, backend, scale, artifact_dir=None, keep_on_fail=False):
        self.checks = []
        self.root = Path("/private/tmp/ih.fake")
        self.agy_info = {"sha512": "ab", "size": 1}
        FakeWorld.built.append(self)

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False

    def check(self, name, ok, detail=""):
        self.checks.append({"name": name, "ok": ok, "detail": detail})

    def agy_version(self):
        return "agy 9.9.9"


def run_main(argv, env, flows_dir, **kwargs):
    out = io.StringIO()
    kwargs.setdefault("platform_name", "darwin")
    kwargs.setdefault("herdr_version_fn", lambda: "herdr 0.9.1")
    rc = gate.main(argv, env=env, flows_dir=flows_dir, out=out, **kwargs)
    return rc, out.getvalue()


class GateTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.flows = Path(self.tmp.name) / "flows"
        self.flows.mkdir()
        self.agy = Path(self.tmp.name) / "agy"
        self.agy.write_text("#!/bin/sh\n")
        self.agy.chmod(0o755)
        self.env = {"HT_AGY_BIN": str(self.agy)}
        self.report = str(Path(self.tmp.name) / "report.json")
        FakeWorld.built = []
        for key in ("HT_LEAK_RUN_ID", "HT_TEST_OWNER"):
            self.addCleanup(os.environ.pop, key, None)

    # -- discovery and --list
    def test_discovery_finds_flows_sorted_from_a_temp_dir_with_two_modules(self):
        write_flows(self.flows, zeta=("ht-1",), boot=("ht-2", "ht-3"))
        found = gate.discover(self.flows)
        self.assertEqual([f.name for f in found], ["boot", "zeta"])
        self.assertEqual(found[0].requires, ("ht-2", "ht-3"))

    def test_discovery_rejects_a_module_without_flow(self):
        (self.flows / "bad.py").write_text("x = 1\n")
        with self.assertRaises(gate.Refusal):
            gate.discover(self.flows)

    def test_shipped_boot_flow_is_discovered(self):
        names = [f.name for f in gate.discover(HERE / "flows")]
        self.assertIn("boot", names)

    def test_list_prints_requires(self):
        write_flows(self.flows, alpha=("ht-poh.4.4", "ht-poh.5"), beta=())
        rc, out = run_main(["--list"], {}, self.flows)
        self.assertEqual(rc, 0)
        self.assertIn("alpha\trequires: ht-poh.4.4, ht-poh.5", out)
        self.assertIn("beta\trequires: -", out)

    # -- refusals: exit 2 with the stated text
    def test_refuses_without_ht_agy_bin(self):
        write_flows(self.flows, boot=())
        rc, out = run_main(["--flow", "boot"], {}, self.flows)
        self.assertEqual(rc, 2)
        self.assertIn("set HT_AGY_BIN to a genuine agy binary (scripts/agy-sandbox/fetch-agy can download one)", out)

    def test_refuses_under_ci_and_github_actions(self):
        write_flows(self.flows, boot=())
        for var, value in (("CI", "1"), ("GITHUB_ACTIONS", "true")):
            env = dict(self.env, **{var: value})
            rc, out = run_main(["--flow", "boot"], env, self.flows)
            self.assertEqual(rc, 2, var)
            self.assertIn("never runs in CI", out)

    def test_refuses_non_darwin(self):
        write_flows(self.flows, boot=())
        rc, out = run_main([], self.env, self.flows, platform_name="linux")
        self.assertEqual(rc, 2)
        self.assertIn("needs macOS", out)

    def test_refuses_herdr_older_than_0_9_1(self):
        write_flows(self.flows, boot=())
        rc, out = run_main([], self.env, self.flows, herdr_version_fn=lambda: "herdr 0.9.0")
        self.assertEqual(rc, 2)
        self.assertIn("older than 0.9.1", out)

    def test_refuses_missing_herdr(self):
        write_flows(self.flows, boot=())
        rc, out = run_main([], self.env, self.flows, herdr_version_fn=lambda: "")
        self.assertEqual(rc, 2)
        self.assertIn("herdr not found", out)

    def test_version_parser(self):
        self.assertEqual(gate.parse_herdr_version("herdr 0.9.1"), (0, 9, 1))
        self.assertEqual(gate.parse_herdr_version("herdr 0.10.0"), (0, 10, 0))
        self.assertLess(gate.parse_herdr_version("0.9.0"), gate.MIN_HERDR)
        self.assertGreaterEqual(gate.parse_herdr_version("0.10.0"), gate.MIN_HERDR)
        self.assertIsNone(gate.parse_herdr_version("garbage"))

    def test_unknown_flow_is_a_refusal(self):
        write_flows(self.flows, boot=())
        rc, out = run_main(["--flow", "nope"], self.env, self.flows)
        self.assertEqual(rc, 2)
        self.assertIn("unknown flow", out)

    def test_build_failure_exits_2(self):
        write_flows(self.flows, boot=())

        def broken():
            raise gate.Refusal("build failed:\nboom")
        rc, out = run_main(["--report", self.report], self.env, self.flows, build_fn=broken,
                           world_factory=FakeWorld, leak_check=lambda rid: (True, ""))
        self.assertEqual(rc, 2)
        self.assertIn("build failed", out)

    # -- run, report shape, exit codes
    def run_gate(self, argv=(), leak=(True, ""), flows=None):
        return run_main(["--herdr-threads-bin", "/bin/true", "--report", self.report] + list(argv),
                        self.env, self.flows, world_factory=FakeWorld,
                        backend_factory=lambda: object(), leak_check=lambda rid: leak)

    def test_pass_writes_report_with_expected_shape_and_exits_0(self):
        write_flows(self.flows, boot=("ht-poh.6.1",), other=())
        rc, out = self.run_gate()
        self.assertEqual(rc, 0, out)
        report = json.loads(Path(self.report).read_text())
        self.assertEqual({"run_id", "owner_pid", "herdr", "herdr_threads", "agy", "flows", "leak_check",
                          "interrupted"}, set(report))
        self.assertEqual([f["name"] for f in report["flows"]], ["boot", "other"])
        boot = report["flows"][0]
        self.assertEqual(boot["status"], "pass")
        self.assertEqual(boot["requires"], ["ht-poh.6.1"])
        self.assertEqual(boot["checks"], [{"name": "ran", "ok": True, "detail": ""}])
        self.assertIn("duration_s", boot)
        self.assertEqual(boot["agy"]["version"], "agy 9.9.9")
        self.assertEqual(report["leak_check"]["ok"], True)
        self.assertEqual(report["agy"]["bin"], str(self.agy))
        self.assertEqual(len(report["agy"]["sha512"]), 128)

    def test_flow_selection_runs_only_named_flows(self):
        write_flows(self.flows, boot=(), other=())
        rc, _ = self.run_gate(["--flow", "other"])
        self.assertEqual(rc, 0)
        report = json.loads(Path(self.report).read_text())
        self.assertEqual([f["name"] for f in report["flows"]], ["other"])

    def test_failed_check_exits_1_and_other_flows_still_run(self):
        write_flows(self.flows, aaa=(), bbb=())
        (self.flows / "aaa.py").write_text(textwrap.dedent('''
            from gate import Flow
            def run(world):
                world.check("bad", False, "why")
            FLOW = Flow(name="aaa", requires=(), run=run)
        '''))
        rc, out = self.run_gate()
        self.assertEqual(rc, 1)
        report = json.loads(Path(self.report).read_text())
        self.assertEqual([f["status"] for f in report["flows"]], ["fail", "pass"])  # no fail-fast
        self.assertIn("bad", out)

    def test_raising_flow_fails_with_error_recorded(self):
        (self.flows / "boom.py").write_text(textwrap.dedent('''
            from gate import Flow
            def run(world):
                raise RuntimeError("kaput")
            FLOW = Flow(name="boom", requires=(), run=run)
        '''))
        rc, _ = self.run_gate()
        self.assertEqual(rc, 1)
        rec = json.loads(Path(self.report).read_text())["flows"][0]
        self.assertEqual(rec["status"], "fail")
        self.assertIn("kaput", rec["error"])

    def test_leak_check_failure_exits_3_even_when_every_flow_passed(self):
        write_flows(self.flows, boot=())
        rc, out = self.run_gate(leak=(False, "LEAK rule=run-id pid=999 ppid=1 cmd=agy"))
        self.assertEqual(rc, 3)
        report = json.loads(Path(self.report).read_text())
        self.assertEqual(report["flows"][0]["status"], "pass")
        self.assertFalse(report["leak_check"]["ok"])
        self.assertIn("pid=999", report["leak_check"]["output"])
        self.assertIn("pid=999", out)

    def test_run_tags_environment_with_owner_and_run_id(self):
        write_flows(self.flows, boot=())
        self.run_gate()
        report = json.loads(Path(self.report).read_text())
        self.assertEqual(os.environ["HT_LEAK_RUN_ID"], report["run_id"])
        self.assertEqual(os.environ["HT_TEST_OWNER"], str(os.getpid()))


class ObserveTests(unittest.TestCase):
    def test_wait_until_scales_its_timeout(self):
        observe.set_timeout_scale(0.0)
        try:
            self.assertFalse(observe.wait_until(lambda: False, 5, 0.01))  # scale 0 => returns at once
        finally:
            observe.set_timeout_scale(1.0)
        self.assertEqual(observe.wait_until(lambda: 7, 1), 7)

    def test_db_is_read_only(self):
        import sqlite3

        class W(object):
            pass
        with tempfile.TemporaryDirectory() as tmp:
            world = W()
            world.state = Path(tmp)
            self.assertIsNone(observe.db(world))
            path = Path(tmp) / "instances" / "x" / "threads.sqlite3"
            path.parent.mkdir(parents=True)
            conn = sqlite3.connect(str(path))
            conn.execute("create table t(a)")
            conn.execute("insert into t values (1)")
            conn.commit()
            conn.close()
            ro = observe.db(world)
            self.assertEqual(ro.execute("select a from t").fetchone()["a"], 1)
            with self.assertRaises(sqlite3.OperationalError):
                ro.execute("insert into t values (2)")
            ro.close()

    def test_backend_clean_flags_unmatched(self):
        class Backend(object):
            def __init__(self, reqs):
                self.reqs = reqs

            def requests(self):
                return self.reqs

        class W(object):
            pass
        world = W()
        world.backend = Backend([{"seq": 1, "kind": "main"}, {"seq": 2, "kind": "main", "unmatched": True}])
        ok, detail = observe.backend_clean(world)
        self.assertFalse(ok)
        self.assertIn("2", detail)
        world.backend = Backend([{"seq": 1, "kind": "main", "unmatched": False}])
        self.assertTrue(observe.backend_clean(world)[0])


class BootScenarioTests(unittest.TestCase):
    def test_boot_scenario_validates_and_replies_boot_ok(self):
        doc = json.loads((HERE / "scenarios" / "boot.json").read_text())
        scenario.validate(doc)
        self.assertEqual(doc["steps"][0]["reply"], {"text": "BOOT-OK"})
        self.assertEqual(len(doc["steps"]), 1)


if __name__ == "__main__":
    unittest.main()
