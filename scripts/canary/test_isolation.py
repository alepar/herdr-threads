import importlib.util, os, pathlib, sys

HERE = pathlib.Path(__file__).resolve().parent

# scripts/canary/bisect.py shadows the stdlib `bisect` (needed by `random`, hence `tempfile`) when
# `discover -s scripts/canary` puts this directory first on sys.path: import those with it off the path.
_saved = list(sys.path)
sys.path[:] = [p for p in sys.path if os.path.realpath(p or ".") != str(HERE)]
sys.modules.pop("bisect", None)
import contextlib, io, subprocess, tempfile, time, unittest  # noqa: E402
sys.path[:] = _saved


def _load(name):
    spec = importlib.util.spec_from_file_location(f"canary_{name}", HERE / f"{name}.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


iso = _load("isolation")
runner = _load("run")


class Tripwire(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.path = os.path.join(self.tmp.name, "settings.json")

    def cycle(self, before_action=None, during=None):
        if before_action:
            before_action()
        before = iso.snapshot([self.path])
        if during:
            during()
        return iso.compare(before, iso.snapshot([self.path]))

    def test_absent_to_absent_passes(self):
        ok, _ = self.cycle()
        self.assertTrue(ok)

    def test_absent_to_present_is_infra(self):
        ok, detail = self.cycle(during=lambda: open(self.path, "w").close())
        self.assertFalse(ok)
        self.assertIn("created", detail)

    def test_present_unchanged_passes(self):
        ok, _ = self.cycle(before_action=lambda: open(self.path, "w").close())
        self.assertTrue(ok)

    def test_present_changed_mtime_is_infra(self):
        def touch():
            st = os.stat(self.path)
            os.utime(self.path, ns=(st.st_atime_ns, st.st_mtime_ns + 5_000_000_000))
        ok, detail = self.cycle(before_action=lambda: open(self.path, "w").close(), during=touch)
        self.assertFalse(ok)
        self.assertIn("mtime changed", detail)

    def test_present_to_removed_is_infra(self):
        ok, detail = self.cycle(before_action=lambda: open(self.path, "w").close(),
                                during=lambda: os.unlink(self.path))
        self.assertFalse(ok)
        self.assertIn("removed", detail)

    def test_cli_snapshot_check_roundtrip(self):
        home = self.tmp.name
        snap = os.path.join(home, "snap.json")
        env = dict(os.environ, HOME=home)
        py = str(HERE / "isolation.py")
        subprocess.run([sys.executable, py, "snapshot", snap], check=True, env=env)
        self.assertEqual(subprocess.run([sys.executable, py, "check", snap], env=env,
                                        capture_output=True).returncode, 0)
        os.makedirs(os.path.join(home, ".codex"))
        open(os.path.join(home, ".codex", "hooks.json"), "w").close()
        r = subprocess.run([sys.executable, py, "check", snap], env=env, capture_output=True, text=True)
        self.assertEqual(r.returncode, 2)
        self.assertIn("hooks.json was created", r.stdout)


class RunPy(unittest.TestCase):
    def test_timeout_kills_the_whole_group(self):
        with tempfile.TemporaryDirectory() as d:
            pidfile = os.path.join(d, "child.pid")
            script = f"sleep 300 & echo $! > {pidfile}; wait"
            start = time.monotonic()
            rc = runner.run(["sh", "-c", script], timeout=1.0)
            self.assertEqual(rc, 124)
            self.assertLess(time.monotonic() - start, 10)
            pid = int(pathlib.Path(pidfile).read_text())
            time.sleep(0.3)
            with self.assertRaises(ProcessLookupError):
                os.kill(pid, 0)

    def test_until_file_stops_run_with_success(self):
        with tempfile.TemporaryDirectory() as d:
            marker = os.path.join(d, "seen")
            rc = runner.run(["sh", "-c", f"sleep 0.3; touch {marker}; sleep 300"], timeout=30,
                            until_file=marker)
            self.assertEqual(rc, 0)

    def test_exit_code_propagates_and_missing_binary_is_127(self):
        self.assertEqual(runner.run(["sh", "-c", "exit 3"], timeout=5), 3)
        with contextlib.redirect_stderr(io.StringIO()) as err:
            self.assertEqual(runner.run(["/nonexistent/bin"], timeout=5), 127)
        self.assertIn("cannot start", err.getvalue())


class VersionsJsonOverride(unittest.TestCase):
    """--versions-json feeds canary-probe.json: a fixture with a known_broken range refuses a version in it."""
    SCRIPT = str(HERE.parent / "harness-canary.sh")
    FIXTURE = str(HERE.parent / "harness-canary-selftest" / "fixtures" / "harness-versions.json")

    def expected(self, harness, version, *extra):
        with tempfile.TemporaryDirectory() as d:
            subprocess.run(["bash", self.SCRIPT, "--write-probe-files", d, harness, version, "/bin/sh", *extra],
                           check=True)
            import json
            return json.loads((pathlib.Path(d) / "canary-probe.json").read_text())["expected_admission"]

    def test_known_broken_range_is_refused_only_with_the_override(self):
        fx = ("--versions-json", self.FIXTURE)
        self.assertEqual(self.expected("stubkb", "1.0.3", *fx), "refused")
        self.assertEqual(self.expected("stubkb", "1.0.1", *fx), "schema-matched-or-optimistic")  # above verified_max
        self.assertEqual(self.expected("stubkb", "1.0.0", *fx), "listed")
        self.assertEqual(self.expected("stubkb", "1.0.3"), "unasserted")  # the real doc has no such harness

    def test_ladder_above_and_below_verified_max(self):
        fx = ("--versions-json", self.FIXTURE)
        self.assertEqual(self.expected("claude", "2.1.286", *fx), "listed")
        self.assertEqual(self.expected("claude", "2.1.287", *fx), "optimistic")
        self.assertEqual(self.expected("claude", "2.1.280", *fx), "unasserted")
        self.assertEqual(self.expected("codex", "0.160.0", *fx), "schema-matched-or-optimistic")

    def test_open_ended_range_refuses_everything_from_min(self):
        self.assertEqual(self.expected("stubopen", "1.0.9", "--versions-json", self.FIXTURE), "refused")


if __name__ == "__main__":
    unittest.main()
