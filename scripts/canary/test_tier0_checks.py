#!/usr/bin/env python3
"""Offline shell tests for the tier-0 checks of harness-canary.sh (ht-p03.14.6, nested spec §D3, coverage r2).

The script is sourced with HT_CANARY_SOURCE_ONLY=1 (it stops after its function definitions) and single checks
run against a fake probe directory with a fake herdr-threads and a fake cargo. No network, npm install, real
cargo or harness. Covered: t0.payload-parse fails on an empty capture/tier0 while t0.hook-fires ran and parses
only captures whose argv ends `hook <h>`; after a witness swap and restore t0.unsetup succeeds and nothing it
(or doctor) does is captured into capture/tier0."""
import os, pathlib, shutil, subprocess, sys, json, hashlib

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
  doctor) echo '{"doctor":{"hooks":{"claude":{"installed":{"admission":"contract_declared","version":null}}}}}'; exit 3 ;;
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

    # Setup's core declaration cannot replace this explicit diagnostic check.
    def test_final_fix_version_diagnostic_keeps_exact_output_and_exit_checks(self):
        for harness, version, line in (("claude", "2.1.287", "2.1.287 (Claude Code)"),
                                       ("codex", "0.160.0", "codex-cli 0.160.0")):
            binary = self.p / "bin" / harness
            binary.parent.mkdir(exist_ok=True)
            for output, code, want in ((line, 0, "pass"), ("different version", 0, "fail"),
                                       (line, 93, "fail")):
                with self.subTest(harness=harness, output=output, code=code):
                    self._exe(binary, f"#!/bin/sh\n[ \"$1\" = --version ] || exit 94\nprintf '%s\\n' '{output}'\nexit {code}\n")
                    self.bash(f"H={harness}; V={version}; run_check t0.version")
                    self.assertEqual(self.checks()["t0.version"][0], want, self.checks())

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

    def test_unsetup_assert_detects_stable_owned_command_residue(self):
        conf = self.p / "home/.codex"
        conf.mkdir(parents=True)
        command = "'/x/herdr-threads' 'hook-event' 'codex' 'SessionStart'"
        (conf / "hooks.json").write_text(json.dumps({"hooks": {"SessionStart": [{"hooks": [{"command": command}]}]}}))
        setup = self.p / "logs/setup.out"
        setup.write_text(json.dumps({"setup": {"commands": {"SessionStart": command}}}))
        report = self.p / "logs/unsetup.out"
        report.write_text('{"setup":{"action":"removed"}}')
        result = self.bash(f't0_py unsetup-assert codex "{report}" "{conf}" "{self.p}/absent-before.toml"')
        self.assertTrue(result.stdout.startswith("fail\t"), result.stdout)

    def test_stable_setup_assert_requires_exact_manifest_owned_groups(self):
        conf = self.p / "home/.claude"
        conf.mkdir(parents=True)
        settings = conf / "settings.json"
        manifest = self.p / "ownership.json"
        report = self.p / "setup.json"
        report.write_text(json.dumps({"setup": {"action": "installed", "manifest": str(manifest),
            "harness_version": {"admission": "contract_declared", "version": None}}}))

        def install(pre_matcher):
            groups = []
            for event, matcher in [("SessionStart", None), ("PreToolUse", pre_matcher)]:
                group = {"hooks": [{"type": "command", "command": f"'/x/herdr-threads' 'hook-event' 'claude' '{event}'"}]}
                if matcher:
                    group["matcher"] = matcher
                digest = hashlib.sha256(json.dumps(group, separators=(",", ":"), sort_keys=True).encode()).hexdigest()
                groups.append({"event": event, "group": group, "fingerprint": "sha256:" + digest})
            settings.write_text(json.dumps({"hooks": {e["event"]: [e["group"]] for e in groups}}))
            manifest.write_text(json.dumps({"version": 3, "harness": "claude", "scope": "user", "path": str(settings),
                "installation_id": "0b5e8f3c-1111-4222-8333-444455556666", "phase": "installed", "owned": groups}))
            return groups

        # A pre-every-tool install owns only a Bash PreToolUse group.
        install("Bash")
        result = self.bash(f't0_py setup-assert claude ignored "{report}" "{conf}"')
        self.assertTrue(result.stdout.startswith("fail\t"), result.stdout)
        self.assertIn("every-tool PreToolUse", result.stdout)
        groups = install("*")
        result = self.bash(f't0_py setup-assert claude ignored "{report}" "{conf}"')
        self.assertTrue(result.stdout.startswith("pass\t"), result.stdout)
        value = json.loads(settings.read_text())
        duplicate = json.loads(json.dumps(groups[0]["group"]))
        duplicate["hooks"][0]["timeout"] = 99
        value["hooks"]["SessionStart"].append(duplicate)
        settings.write_text(json.dumps(value))
        result = self.bash(f't0_py setup-assert claude ignored "{report}" "{conf}"')
        self.assertTrue(result.stdout.startswith("fail\t"), result.stdout)
        manifest.unlink()
        result = self.bash(f't0_py setup-assert claude ignored "{report}" "{conf}"')
        self.assertTrue(result.stdout.startswith("fail\t"), result.stdout)

    def test_stable_native_event_capture_is_parsed(self):
        self.capture("1-1", ["--state-dir", "/s", "--host-endpoint", "/h", "hook-event", "claude", "SessionStart"])
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

    def test_task3_r1_live_core_admission_ignores_historical_ladder_and_schema(self):
        for expected in ("listed", "refused", "optimistic", "schema-matched-or-optimistic", "unasserted"):
            for schema in ("match", "drift", ""):
                with self.subTest(expected=expected, schema=schema):
                    self.bash(f"EXPECTED_ADMISSION='{expected}'; SCHEMA_RESULT='{schema}'; run_check t0.admission")
                    status, detail = self.checks()["t0.admission"]
                    self.assertEqual(status, "pass", detail)
                    self.assertIn("no exact-runtime/native proof", detail)

    def test_task3_r1_live_core_admission_retains_errors_and_infra(self):
        for value, want, infra in (("unknown", "fail", "0"), ("listed", "fail", "0"),
                                   (None, "fail", "0"), ("not_found", "fail", "1")):
            with self.subTest(value=value):
                import json
                doc = json.dumps({"doctor": {"hooks": {"claude": {"installed": {"admission": value}}}}})
                self._exe(self.p / "ht/herdr-threads", f"#!/bin/sh\nprintf '%s\\n' '{doc}'\nexit 3\n")
                proc = self.bash("EXPECTED_ADMISSION=listed; run_check t0.admission; printf 'infra=%s\\n' \"$INFRA\"")
                self.assertEqual(self.checks()["t0.admission"][0], want)
                self.assertIn(f"infra={infra}", proc.stdout)
        for bad in ("{}", "not json"):
            with self.subTest(bad=bad):
                self._exe(self.p / "ht/herdr-threads", f"#!/bin/sh\nprintf '%s\\n' '{bad}'\nexit 3\n")
                self.bash("EXPECTED_ADMISSION=listed; run_check t0.admission")
                self.assertEqual(self.checks()["t0.admission"][0], "fail")

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
