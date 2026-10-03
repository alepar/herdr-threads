#!/usr/bin/env python3
"""Offline shell tests for the tier-0 checks of harness-canary.sh (ht-p03.14.6, nested spec §D3, coverage r2).

The script is sourced with HT_CANARY_SOURCE_ONLY=1 (it stops after its function definitions) and single checks
run against a fake probe directory with a fake herdr-threads and a fake cargo. No network, npm install, real
cargo or harness. Covered: t0.payload-parse fails on an empty capture/tier0 while t0.hook-fires ran and parses
only captures whose argv ends `hook <h>`; after a witness swap and restore t0.unsetup succeeds and nothing it
(or doctor) does is captured into capture/tier0."""
import os, pathlib, shutil, subprocess, sys

HERE = pathlib.Path(__file__).resolve().parent
# scripts/canary/bisect.py shadows the stdlib `bisect` (needed by `random`, hence `tempfile`) when
# `discover -s scripts/canary` puts this directory first on sys.path: import the stdlib ones without it.
_saved = list(sys.path)
sys.path[:] = [p for p in sys.path if os.path.realpath(p or ".") != str(HERE)]
sys.modules.pop("bisect", None)
import tempfile, unittest  # noqa: E402
sys.path[:] = _saved

SCRIPT = HERE.parent / "harness-canary.sh"
FAKE_HT = """#!/bin/sh
case "$1" in
  unsetup) echo '{"setup":{"action":"removed"}}' ;;
  doctor) echo '{"doctor":{"hooks":{"claude":{"installed":{"admission":"optimistic"}}}}}'; exit 3 ;;
  hook) cat >/dev/null ;;
esac
"""
FAKE_CARGO = """#!/bin/sh
d=$HT_CANARY_CAPTURE_DIR
if [ -e "$d/fake-cargo-payloads" ]; then
  echo '{"observation":"ok","payloads":[{"file":"capture/tier0/1.stdin","ok":true}]}' > "$d/canary-rust.json"
else
  echo '{"observation":"ok","payloads":[]}' > "$d/canary-rust.json"
fi
echo 'test result: ok. 1 passed; 0 failed'
"""
SESSION_START = '{"hook_event_name":"SessionStart","source":"startup","session_id":"s"}'


@unittest.skipUnless(shutil.which("node") and shutil.which("npm"), "harness-canary.sh needs node and npm")
class Tier0Checks(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = pathlib.Path(self.tmp.name)
        self.p = self.root / "probe"
        self.out = self.root / "out"
        bindir = self.root / "fakebin"
        for d in (self.p / "ht", self.p / "capture/tier0", self.p / "logs", bindir):
            d.mkdir(parents=True)
        self._exe(self.p / "ht/herdr-threads", FAKE_HT)
        self._exe(bindir / "cargo", FAKE_CARGO)
        self.env = dict(os.environ, PATH=f"{bindir}{os.pathsep}{os.environ['PATH']}",
                        CARGO_HOME=str(self.root / "cargohome"), HOME=str(self.root))
        self.env.pop("CARGO_TARGET_DIR", None)

    @staticmethod
    def _exe(path, text):
        path.write_text(text)
        path.chmod(0o755)

    def capture(self, name, argv, stdin=SESSION_START):
        cap = self.p / "capture/tier0"
        (cap / f"{name}.stdin").write_text(stdin)
        (cap / f"{name}.argv").write_text("".join(f"{a}\n" for a in argv))

    def bash(self, body):
        script = f"""set -euo pipefail
export HT_CANARY_SOURCE_ONLY=1
source "{SCRIPT}" --out "{self.out}" --model-tier off
P="{self.p}"; LOGS="$P/logs"; H=claude; V=2.1.287; INFRA=0
: > "$P/checks.tsv"
build_env "$P"
{body}
"""
        # $0 names the sourced script: harness-canary.sh locates its siblings from it
        proc = subprocess.run(["bash", "-c", script, str(SCRIPT)], env=self.env, stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=120)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        return proc

    def checks(self):
        rows = {}
        for line in (self.p / "checks.tsv").read_text().splitlines():
            cid, status, detail = (line.split("\t", 2) + ["", ""])[:3]
            rows[cid] = (status, detail)
        return rows

    def tier0_files(self):
        return sorted(f.name for f in (self.p / "capture/tier0").iterdir() if not f.name.startswith("."))

    # ---- t0.payload-parse

    def test_empty_tier0_fails_when_hook_fires_ran(self):
        self.bash("HOOK_FIRES_RAN=1; run_check t0.payload-parse")
        status, detail = self.checks()["t0.payload-parse"]
        self.assertEqual(status, "fail")
        self.assertIn("no capture whose argv ends 'hook claude'", detail)

    def test_empty_tier0_is_not_a_failure_when_hook_fires_never_ran(self):
        self.bash("HOOK_FIRES_RAN=0; run_check t0.payload-parse")
        status, detail = self.checks()["t0.payload-parse"]
        self.assertEqual((status, detail.startswith("canary_payloads parsed 0 tier-0")), ("pass", True), detail)

    def test_captures_of_other_invocations_do_not_count(self):
        self.capture("1-1", ["--state-dir", "/s", "unsetup", "claude"])
        self.capture("2-1", ["--state-dir", "/s", "doctor"])
        self.capture("3-1", ["hook", "codex"])  # another harness's hook
        self.bash("HOOK_FIRES_RAN=1; run_check t0.payload-parse")
        status, detail = self.checks()["t0.payload-parse"]
        self.assertEqual(status, "fail", detail)
        self.assertIn("no capture whose argv ends 'hook claude'", detail)

    def test_a_hook_capture_is_parsed(self):
        self.capture("1-1", ["--state-dir", "/s", "--host-endpoint", "/h", "hook", "claude"])
        (self.p / "fake-cargo-payloads").write_text("")
        self.bash("HOOK_FIRES_RAN=1; run_check t0.payload-parse")
        status, detail = self.checks()["t0.payload-parse"]
        self.assertEqual(status, "pass", detail)
        self.assertIn("parsed 1 tier-0 payload(s) (1 hook capture(s))", detail)

    def test_an_evented_hook_capture_is_parsed(self):
        # setup registers `hook <harness> --event <NAME>`; a bare word after it is not a hook capture
        self.capture("1-1", ["--state-dir", "/s", "hook", "claude", "--event", "SessionStart"])
        (self.p / "fake-cargo-payloads").write_text("")
        self.bash("HOOK_FIRES_RAN=1; run_check t0.payload-parse")
        status, detail = self.checks()["t0.payload-parse"]
        self.assertEqual(status, "pass", detail)
        self.assertIn("(1 hook capture(s))", detail)

    def test_malformed_event_registrations_are_not_hook_captures(self):
        self.capture("1-1", ["hook", "claude", "--event"])
        self.capture("2-1", ["hook", "claude", "--event", "a b"])
        self.capture("3-1", ["hook", "claude", "--event", "SessionStart", "extra"])
        self.capture("4-1", ["hook", "codex", "--event", "SessionStart"])  # another harness
        self.bash("HOOK_FIRES_RAN=1; run_check t0.payload-parse")
        status, detail = self.checks()["t0.payload-parse"]
        self.assertEqual(status, "fail", detail)
        self.assertIn("no capture whose argv ends 'hook claude'", detail)

    def test_captures_the_rust_test_did_not_parse_fail(self):
        self.capture("1-1", ["hook", "claude"])
        self.bash("HOOK_FIRES_RAN=1; run_check t0.payload-parse")  # fake cargo parsed no payload
        status, detail = self.checks()["t0.payload-parse"]
        self.assertEqual(status, "fail", detail)
        self.assertIn("parsed no tier-0 payload", detail)

    # ---- witness swap and restore

    def test_witness_records_while_swapped_and_unsetup_after_restore_is_not_captured(self):
        self.bash(f"""
witness_swap
printf '%s' '{SESSION_START}' | "$P/ht/herdr-threads" --state-dir "$P/state" hook claude
# a witnessed unsetup WOULD be captured: this is what restoring first prevents
"$P/ht/herdr-threads" --state-dir "$P/state" unsetup claude >/dev/null
witness_restore
[ ! -e "$P/ht/herdr-threads.real" ]
mkdir -p "$P/capture/after"; cp "$P"/capture/tier0/* "$P/capture/after/"
rm -f "$P"/capture/tier0/*
run_check t0.unsetup
ht_xrun doctor-after 30 doctor
""")
        self.assertEqual(self.checks()["t0.unsetup"][0], "pass", self.checks()["t0.unsetup"])
        self.assertEqual(self.tier0_files(), [], "unsetup/doctor after the restore were captured")
        swapped = sorted(f.name.split(".", 1)[1] for f in (self.p / "capture/after").iterdir())
        self.assertEqual(swapped.count("argv"), 2)  # the hook and the deliberately witnessed unsetup
        argvs = [f.read_text().splitlines()[-2:] for f in (self.p / "capture/after").glob("*.argv")]
        self.assertIn(["hook", "claude"], argvs)
        self.assertIn(["unsetup", "claude"], argvs)
        stdins = [f.read_text() for f in (self.p / "capture/after").glob("*.stdin")]
        self.assertIn(SESSION_START, stdins)  # raw hook stdin recorded verbatim

    def test_unsetup_restores_a_left_over_witness_itself(self):
        self.bash("witness_swap; run_check t0.unsetup")
        self.assertEqual(self.checks()["t0.unsetup"][0], "pass", self.checks()["t0.unsetup"])
        self.assertEqual(self.tier0_files(), [])
        self.assertFalse((self.p / "ht/herdr-threads.real").exists())

    def test_doctor_checks_restore_a_left_over_witness_too(self):
        self.bash("witness_swap; EXPECTED_ADMISSION=optimistic; run_check t0.admission")
        status, detail = self.checks()["t0.admission"]
        self.assertEqual(status, "pass", detail)
        self.assertEqual(self.tier0_files(), [])

    def test_unsetup_that_leaves_settings_behind_fails(self):
        conf = self.p / "home/.claude"
        conf.mkdir(parents=True)
        (conf / "settings.json").write_text("{}")
        self.bash("run_check t0.unsetup")
        status, detail = self.checks()["t0.unsetup"]
        self.assertEqual(status, "fail")
        self.assertIn("settings.json still exists", detail)


if __name__ == "__main__":
    unittest.main()
