"""Offline tests for agyenv.py and fetch-agy (no agy, Herdr, network or key)."""
import contextlib
import importlib.machinery
import io
import importlib.util
import json
import os
import shutil
import sys
import tempfile
import unittest
from unittest import mock
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import agyenv  # noqa: E402


def load_fetch():
    loader = importlib.machinery.SourceFileLoader("fetch_agy", str(HERE / "fetch-agy"))
    spec = importlib.util.spec_from_loader("fetch_agy", loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module


fetch = load_fetch()

GOLDEN_PROFILE = '''(version 1) (allow default) (deny network*)
(allow network-outbound (remote ip "localhost:*")) (allow network-inbound (local ip "localhost:*"))
(allow network* (remote unix-socket))
(deny file-read* (subpath "/Users/real/.gemini") (subpath "/Users/real/.claude") (subpath "/Users/real/.codex")
                 (subpath "/Users/real/.aisw") (subpath "/Users/real/.hermes"))
(deny file-write*)
(allow file-write* (subpath "/private/tmp/ih.Ab12Cd") (literal "/dev/null") (literal "/dev/tty") (literal "/dev/ptmx")
                   (regex #"^/dev/ttys[0-9]+$") (regex #"^/dev/fd/"))
(deny mach-lookup (global-name "com.apple.securityd"))
'''

GOLDEN_WRAPPER = '''#!/bin/sh
exec /usr/bin/env -i PATH="/private/tmp/ih.Ab12Cd/bin:/usr/bin:/bin:/usr/sbin:/sbin" HOME="/private/tmp/ih.Ab12Cd/home" TMPDIR="/private/tmp/ih.Ab12Cd/tmp" \\
  XDG_CONFIG_HOME="/private/tmp/ih.Ab12Cd/cfg" XDG_CACHE_HOME="/private/tmp/ih.Ab12Cd/cache" XDG_DATA_HOME="/private/tmp/ih.Ab12Cd/data" XDG_STATE_HOME="/private/tmp/ih.Ab12Cd/st" \\
  HERDR_PLUGIN_STATE_DIR="/private/tmp/ih.Ab12Cd/ht" \\
  GEMINI_API_KEY="tok-123" GOOGLE_GEMINI_BASE_URL="http://127.0.0.1:4242" \\
  TERM="${TERM:-xterm-256color}" LANG=en_US.UTF-8 NO_COLOR=1 \\
  HERDR_ENV="$HERDR_ENV" HERDR_PANE_ID="$HERDR_PANE_ID" HERDR_SOCKET_PATH="$HERDR_SOCKET_PATH" \\
  HT_TEST_OWNER="777" HT_LEAK_RUN_ID="run-1" ${HERDR_THREADS_AGY_OPTS+HERDR_THREADS_AGY_OPTS="$HERDR_THREADS_AGY_OPTS"} \\
  /usr/bin/sandbox-exec -f "/private/tmp/ih.Ab12Cd/loopback.sb" "/private/tmp/ih.Ab12Cd/agybin/2/agy" "$@"
'''


class RenderTests(unittest.TestCase):
    def test_profile_matches_golden_with_both_substitutions(self):
        text = agyenv.render_profile("/private/tmp/ih.Ab12Cd", "/Users/real")
        self.assertEqual(text, GOLDEN_PROFILE)
        self.assertNotIn("<ROOT>", text)
        self.assertNotIn("<REAL_HOME>", text)

    def test_wrapper_matches_golden(self):
        text = agyenv.render_wrapper("/private/tmp/ih.Ab12Cd", "/private/tmp/ih.Ab12Cd/agybin/2", 4242,
                                     "tok-123", 777, "run-1")
        self.assertEqual(text, GOLDEN_WRAPPER)

    def test_wrapper_carries_the_required_pieces(self):
        text = agyenv.render_wrapper("/r", "/r/agybin/1", 1, "TOKEN-X", 5, "RUN-Y")
        for needle in ("env -i", "TOKEN-X", 'HT_TEST_OWNER="5"', 'HT_LEAK_RUN_ID="RUN-Y"',
                       '${HERDR_THREADS_AGY_OPTS+HERDR_THREADS_AGY_OPTS="$HERDR_THREADS_AGY_OPTS"}'):
            self.assertIn(needle, text)

    def test_seed_settings_has_exactly_three_keys(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = agyenv.seed_settings(tmp)
            self.assertEqual(path, Path(tmp) / "home/.gemini/antigravity-cli/settings.json")
            self.assertEqual(json.loads(path.read_text()), {
                "altScreenMode": "never", "modelProvider": "gemini",
                "trustedWorkspaces": [tmp + "/ws"]})
            self.assertEqual(path.read_text(),
                             '{"altScreenMode":"never","modelProvider":"gemini","trustedWorkspaces":["%s/ws"]}' % tmp)

    def test_real_home_comes_from_passwd_not_home_env(self):
        old = os.environ.get("HOME")
        os.environ["HOME"] = "/nonexistent/fake"
        try:
            self.assertNotEqual(agyenv.real_home(), "/nonexistent/fake")
        finally:
            os.environ["HOME"] = old


class IsolationTests(unittest.TestCase):
    def world(self, root):
        w = agyenv.World("run", os.getpid(), "/bin/true", object())
        w.root = Path(root)
        return w

    def test_ht_refuses_home_outside_root(self):
        with tempfile.TemporaryDirectory() as tmp:
            w = self.world(tmp)
            with self.assertRaises(agyenv.SafetyError):
                w.ht(["--version"], env={"HOME": os.path.expanduser("~")})

    def test_ht_refuses_empty_home(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(agyenv.SafetyError):
                self.world(tmp).ht(["--version"], env={"HOME": ""})

    def test_ht_runs_with_home_inside_root_and_explicit_state(self):
        with tempfile.TemporaryDirectory() as tmp:
            script = Path(tmp) / "fake-ht"
            script.write_text('#!/bin/sh\necho "$HOME|$*|${HERDR_PANE_ID:-}"\n')
            script.chmod(0o755)
            w = self.world(tmp)
            w.herdr_threads_bin = str(script)
            out = w.ht(["inbox"], pane="p_1").stdout.strip()
            home, argv, pane = out.split("|")
            self.assertEqual(home, tmp + "/home")
            self.assertEqual(argv, "--state-dir %s/ht --host-endpoint %s/h.sock inbox" % (tmp, tmp))
            self.assertEqual(pane, "p_1")

    def _fake_ht_world(self, tmp):
        script = Path(tmp) / "fake-ht"
        script.write_text('#!/bin/sh\necho "$*"\n')
        script.chmod(0o755)
        w = self.world(tmp)
        w.herdr_threads_bin = str(script)
        return w

    def test_setup_agy_runs_in_the_human_namespace(self):
        with tempfile.TemporaryDirectory() as tmp:
            w = self._fake_ht_world(tmp)
            base = "human --state-dir %s/ht --host-endpoint %s/h.sock setup agy" % (tmp, tmp)
            self.assertEqual(w.setup_agy().stdout.strip(), base + " --with-permissions")
            self.assertEqual(w.setup_agy(permissions=False).stdout.strip(), base)

    def test_unsetup_agy_runs_in_the_human_namespace(self):
        with tempfile.TemporaryDirectory() as tmp:
            w = self._fake_ht_world(tmp)
            self.assertEqual(w.unsetup_agy().stdout.strip(),
                             "human --state-dir %s/ht --host-endpoint %s/h.sock unsetup agy" % (tmp, tmp))

    def test_herdr_env_drops_inherited_herdr_variables(self):
        with tempfile.TemporaryDirectory() as tmp:
            os.environ["HERDR_PANE_ID"] = "leaked"
            try:
                env = self.world(tmp).herdr_env()
            finally:
                del os.environ["HERDR_PANE_ID"]
            self.assertNotIn("HERDR_PANE_ID", env)
            self.assertEqual(env["HERDR_SOCKET_PATH"], tmp + "/h.sock")
            self.assertEqual(env["HOME"], tmp + "/home")


class PaneTests(unittest.TestCase):
    class Result(object):
        def __init__(self, rc, out):
            self.returncode, self.stdout, self.stderr = rc, out, ""

    class StubWorld(object):
        timeout_scale = 1.0

        def __init__(self, result):
            self.result = result

        def herdr(self, *argv, **kwargs):
            return self.result

    def pane(self, rc, out):
        return agyenv.AgyPane(self.StubWorld(self.Result(rc, out)), "w1:p2")

    def test_agent_name_and_status_read_result_agent(self):
        pane = self.pane(0, '{"result":{"agent":{"agent":"agy","agent_status":"idle"}}}')
        self.assertEqual(pane.agent_name(), "agy")
        self.assertEqual(pane.agent_status(), "idle")

    def test_agent_name_is_none_when_not_detected_or_malformed(self):
        self.assertIsNone(self.pane(1, '{"error":{"code":"agent_not_found"}}').agent_name())
        self.assertIsNone(self.pane(0, "not json").agent_name())
        self.assertIsNone(self.pane(0, '{"result":{"agent":null}}').agent_name())

    class ScreenWorld(object):
        """Returns successive screens for ``pane read`` and records ``agent send-keys``."""
        timeout_scale = 1.0

        def __init__(self, screens):
            self.screens = list(screens)
            self.sent = []

        def herdr(self, *argv, **kwargs):
            if argv[:2] == ("pane", "read"):
                text = self.screens.pop(0) if len(self.screens) > 1 else self.screens[0]
                return PaneTests.Result(0, text)
            if argv[:2] == ("agent", "send-keys"):
                self.sent.append(tuple(argv[3:]))
            return PaneTests.Result(0, "")

    def walk_pane(self, screens):
        world = self.ScreenWorld(screens)
        return world, agyenv.AgyPane(world, "w1:p2")

    COLOR = "Choose your color scheme\n> dark"
    TERMS = "Terms of Service\n[ ] share data\n[Done]"
    FOOTER = "? for shortcuts"

    def test_onboarding_walk_dismisses_color_then_terms_and_stops_at_the_footer(self):
        world, pane = self.walk_pane([self.COLOR, self.TERMS, self.FOOTER])
        with mock.patch.object(agyenv.time, "sleep"):
            pages = pane.dismiss_onboarding()
        self.assertEqual(pages, ["color", "terms"])
        self.assertEqual(world.sent, [("Enter",), ("Down",), ("Right",), ("Enter",)])

    def test_onboarding_walk_is_a_no_op_at_the_composer(self):
        world, pane = self.walk_pane([self.FOOTER])
        with mock.patch.object(agyenv.time, "sleep"):
            self.assertEqual(pane.dismiss_onboarding(), [])
        self.assertEqual(world.sent, [])

    def test_onboarding_walk_leaves_a_trust_prompt_alone(self):
        world, pane = self.walk_pane(["Do you trust the files in this folder?\n1. Yes\n2. No"])
        with mock.patch.object(agyenv.time, "sleep"):
            self.assertEqual(pane.dismiss_onboarding(), [])
        self.assertEqual(world.sent, [])

    def test_survey_dismissal_sends_zero_only_when_the_survey_shows(self):
        world, pane = self.walk_pane([agyenv.SURVEY + "\n[0] Skip"])
        with mock.patch.object(agyenv.time, "sleep"):
            self.assertTrue(pane.dismiss_survey())
        self.assertEqual(world.sent, [("0",)])
        world, pane = self.walk_pane([self.FOOTER])
        with mock.patch.object(agyenv.time, "sleep"):
            self.assertFalse(pane.dismiss_survey())
        self.assertEqual(world.sent, [])

    def test_scrub_agent_env_drops_identity_and_credentials(self):
        env = {"CLAUDECODE": "1", "OPENROUTER_API_KEY": "k", "GEMINI_API_KEY": "g", "PATH": "/bin", "HOME": "/h"}
        self.assertEqual(agyenv.scrub_agent_env(env), {"PATH": "/bin", "HOME": "/h"})


class SpawnTests(unittest.TestCase):
    def test_spawn_owned_records_the_process_group(self):
        with tempfile.TemporaryDirectory() as tmp:
            proc = agyenv.spawn_owned(["/bin/sleep", "30"], dict(os.environ), 424242,
                                      ensure_reaper=False, reaper_root=tmp)
            try:
                pgid = os.getpgid(proc.pid)
                self.assertEqual(pgid, proc.pid)  # its own session leader
                self.assertTrue((Path(tmp) / "ih-reaper.424242" / ("g.%d" % pgid)).exists())
            finally:
                agyenv.stop_group(proc)
            self.assertIsNotNone(proc.poll())

    def test_wait_until_returns_last_value(self):
        self.assertFalse(agyenv.wait_until(lambda: 0, 0.2, 0.05))
        self.assertEqual(agyenv.wait_until(lambda: "v", 1), "v")


class FetchTests(unittest.TestCase):
    ARCHIVE = HERE / "testdata" / "agy-9.9.9.tar.gz"
    MANIFEST = HERE / "testdata" / "manifest-9.9.9.json"

    def setUp(self):
        self._ci = {k: os.environ.pop(k, None) for k in ("CI", "GITHUB_ACTIONS")}
        self.tmp = tempfile.mkdtemp()

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)
        for key, value in self._ci.items():
            if value is not None:
                os.environ[key] = value

    def run_fetch(self, *extra):
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            return fetch.main(["--dest", self.tmp + "/out", "--archive", str(self.ARCHIVE)] + list(extra))

    def test_platform_keys(self):
        self.assertEqual(fetch.platform_key("Darwin", "arm64"), "darwin_arm64")
        self.assertEqual(fetch.platform_key("Darwin", "x86_64"), "darwin_amd64")
        with self.assertRaises(fetch.FetchError):
            fetch.platform_key("Linux", "x86_64")

    def test_manifest_url_shape(self):
        self.assertEqual(
            fetch.manifest_url("darwin_arm64"),
            "https://antigravity-cli-auto-updater-974169037036.us-central1.run.app/manifests/darwin_arm64.json")

    def test_url_prefix_refuses_other_hosts(self):
        good = fetch.URL_PREFIX + "1.0/agy.tar.gz"
        self.assertEqual(fetch.check_url(good), good)
        for bad in ("https://evil.example/antigravity-public/antigravity-cli/x.tar.gz",
                    "http://storage.googleapis.com/antigravity-public/antigravity-cli/x",
                    "https://storage.googleapis.com/antigravity-public/other/x", None):
            with self.assertRaises(fetch.FetchError):
                fetch.check_url(bad)

    def test_pass_verifies_extracts_and_saves_manifest(self):
        self.assertEqual(self.run_fetch("--manifest", str(self.MANIFEST), "--version", "9.9.9"), 0)
        agy = Path(self.tmp) / "out" / "9.9.9" / "agy"
        self.assertTrue(agy.is_file())
        self.assertEqual(agy.stat().st_mode & 0o777, 0o755)
        self.assertIn(b"agy-test 9.9.9", agy.read_bytes())
        self.assertEqual(json.loads((agy.parent / "manifest.json").read_text())["version"], "9.9.9")

    def test_sha512_mismatch_refused_and_nothing_installed(self):
        bad = json.loads(self.MANIFEST.read_text())
        bad["sha512"] = "0" * 128
        path = Path(self.tmp) / "bad.json"
        path.write_text(json.dumps(bad))
        self.assertEqual(self.run_fetch("--manifest", str(path)), 1)
        self.assertFalse((Path(self.tmp) / "out" / "9.9.9" / "agy").exists())

    def test_tampered_archive_refused(self):
        tampered = Path(self.tmp) / "t.tar.gz"
        tampered.write_bytes(self.ARCHIVE.read_bytes() + b"x")
        with contextlib.redirect_stderr(io.StringIO()):
            rc = fetch.main(["--dest", self.tmp + "/out", "--archive", str(tampered),
                             "--manifest", str(self.MANIFEST)])
        self.assertEqual(rc, 1)

    def test_pinned_version_mismatch_refused(self):
        pins = Path(self.tmp) / "pins.json"
        pins.write_text(json.dumps({"versions": {"9.9.9": {"sha512": "f" * 128}}}))
        self.assertEqual(self.run_fetch("--manifest", str(self.MANIFEST), "--versions-file", str(pins)), 1)
        self.assertFalse((Path(self.tmp) / "out" / "9.9.9" / "agy").exists())

    def test_pinned_version_match_accepted(self):
        digest = json.loads(self.MANIFEST.read_text())["sha512"]
        pins = Path(self.tmp) / "pins.json"
        pins.write_text(json.dumps({"versions": {"9.9.9": {"sha512": digest}}}))
        self.assertEqual(self.run_fetch("--manifest", str(self.MANIFEST), "--versions-file", str(pins)), 0)

    def test_version_flag_mismatch_refused(self):
        self.assertEqual(self.run_fetch("--manifest", str(self.MANIFEST), "--version", "1.3.3"), 1)

    def test_manifest_url_outside_prefix_refused(self):
        bad = json.loads(self.MANIFEST.read_text())
        bad["url"] = "https://example.com/agy.tar.gz"
        path = Path(self.tmp) / "bad.json"
        path.write_text(json.dumps(bad))
        self.assertEqual(self.run_fetch("--manifest", str(path)), 1)

    def test_refuses_under_ci(self):
        os.environ["CI"] = "1"
        try:
            self.assertEqual(self.run_fetch("--manifest", str(self.MANIFEST)), 1)
        finally:
            del os.environ["CI"]
        self.assertFalse((Path(self.tmp) / "out").exists())

    def test_shipped_pins_file_is_valid_json_with_versions_object(self):
        data = json.loads((HERE / "agy-versions.json").read_text())
        self.assertIsInstance(data["versions"], dict)
        for version, pin in data["versions"].items():
            self.assertEqual(len(pin["sha512"]), 128, version)


if __name__ == "__main__":
    unittest.main()
