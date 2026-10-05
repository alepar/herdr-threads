#!/usr/bin/env python3
"""Seam integration (ht-p03.14.9): the canary probe result and the capture-directory files, end to end.

Crosses three seams with the real scripts and the real gated Rust test (only npm and the harness binaries are
fakes, so no network and no model call):
  harness-canary.sh --probe -> HT_CANARY_PLANT_TIER1_FAIL -> bisect.py's retry policy;
  --versions-json -> canary-probe.json expected_admission == versions.py -> separate historical recipe diagnostic;
  a Codex probe -> help/codex.txt -> canary-rust.json launch_tables.

Run: python3 -m unittest discover -s scripts/canary -p 'test_*.py'
"""
import json
import os
import pathlib
import shutil
import subprocess
import sys
import textwrap
import unittest

HERE = pathlib.Path(__file__).resolve().parent
if str(HERE) not in sys.path:
    sys.path.append(str(HERE))  # after the stdlib: bisect.py here would shadow it
import test_capture_hook as tch  # noqa: E402  (module reference only: its test classes are not re-collected)
import versions  # noqa: E402

ROOT = HERE.parent.parent
SCRIPT = str(ROOT / "scripts" / "harness-canary.sh")

CLAUDE_LISTED, CLAUDE_BROKEN, CLAUDE_NEWER = "2.1.286", "2.1.290", "2.1.295"
CODEX_BROKEN = "0.158.1"

FIXTURE = {
    "schema_version": 1,
    "rows": [
        {"harness": "claude", "version": "2.1.285", "recipe": "claude-fixture", "evidence": "no_model",
         "known_broken": []},
        {"harness": "claude", "version": CLAUDE_LISTED, "recipe": "claude-fixture", "evidence": "no_model",
         "known_broken": [{"min": CLAUDE_BROKEN, "max": "2.1.291"}]},
        {"harness": "codex", "version": "0.157.0", "recipe": "codex-fixture", "evidence": "no_model",
         "known_broken": [{"min": "0.158.0", "max": "0.158.9"}]},
    ],
}

FAKE_CODEX_HELP = textwrap.dedent("""\
    Codex CLI

    Commands:
      exec    Run Codex non-interactively [aliases: e]
      resume  Resume a previous session

    Options:
      -h, --help  Print help
    """)


def classify(doc, harness, version):
    """versions.py's own reading of the document, independent of the shell wrapper under test."""
    if versions.in_any_range(version, versions.known_broken(doc, harness)):
        return "refused"
    if any(r["version"] == version for r in doc["rows"] if r["harness"] == harness):
        return "listed"
    vmax = versions.verified_max(doc, harness)
    if vmax is not None and versions.version_key(version) > versions.version_key(vmax):
        return "optimistic"
    return "unasserted"


@unittest.skipUnless(shutil.which("node") and shutil.which("npm"), "the canary script needs node and npm on PATH")
class SeamIntegration(unittest.TestCase):
    # the fake-npm harness of test_capture_hook.ScriptTier1, reused rather than copied
    VERSION = tch.ScriptTier1.VERSION
    setUp = tch.ScriptTier1.setUp
    tearDown = tch.ScriptTier1.tearDown
    install_fake_claude = tch.ScriptTier1.install_fake_claude
    env = tch.ScriptTier1.env
    probe_args = tch.ScriptTier1.probe_args
    claude_calls = tch.ScriptTier1.claude_calls
    bisect = tch.ScriptTier1.bisect

    def real_cargo_env(self, **extra):
        """The probe environment with no fake cargo: the script and this test run the real gated test."""
        (self.bin / "cargo").unlink(missing_ok=True)
        # HOME is a throwaway: point rustup and cargo at the real, read-only toolchain homes
        real = pathlib.Path(os.environ.get("HOME", "~")).expanduser()
        toolchain = {"CARGO_HOME": os.environ.get("CARGO_HOME", str(real / ".cargo")),
                     "RUSTUP_HOME": os.environ.get("RUSTUP_HOME", str(real / ".rustup"))}
        return self.env(**{**toolchain, **extra})

    def write_fixture(self):
        path = self.t / "harness-versions.json"
        path.write_text(json.dumps(FIXTURE))
        return str(path)

    def keep_probe(self, harness, version, fixture, tier0, extra_env=None):
        """One --keep probe with --versions-json; returns (exit code, probe dir, stderr)."""
        out = self.t / f"out-{harness}-{version}"
        env = self.real_cargo_env()
        env["HT_CANARY_TIER0_CHECKS"] = tier0
        env.update(extra_env or {})
        cp = subprocess.run(
            ["bash", SCRIPT, "--probe", harness, version, "--out", str(out), "--herdr-threads", str(self.ht),
             "--model-tier", "off", "--versions-json", fixture, "--keep"],
            env=env, capture_output=True, text=True, cwd=ROOT, timeout=2400)
        (probe_dir,) = (out / "work").glob(f"{harness}-{version}-*")
        return cp, probe_dir

    def gated_cargo(self, capture_dir, fixture):
        env = {k: v for k, v in os.environ.items() if k not in ("HT_CANARY_PLANT_TIER1_FAIL",)}
        env.update(HT_CANARY_CAPTURE_DIR=str(capture_dir), HT_TEST_RECIPES_JSON=fixture)
        return subprocess.run(
            ["nice", "cargo", "test", "--locked", "--all-features", "--lib", "gated_canary_payloads"],
            env=env, capture_output=True, text=True, cwd=ROOT, timeout=2400)

    # -- 1. bisect.py's tier-1 retry policy against the real --probe path

    def test_tier1_planted_failure_is_retried_twice(self):
        res, invocations = self.bisect(f"claude@{self.VERSION}:2")
        self.assertEqual(invocations, [self.VERSION] * 3, "one attempt plus two retries")
        (probe,) = res["probes"]
        self.assertEqual([a["result"] for a in probe["attempts"]], ["fail", "fail", "pass"])
        self.assertTrue(all(a["tier1"] for a in probe["attempts"][:2]), "both failures are tier-1 failures")
        self.assertTrue(probe["flaky"])
        self.assertEqual(res["status"], "all_pass", "a pass after the planted count is exhausted is flaky, not a break")
        self.assertEqual(self.claude_calls(), [], "the planted path makes no model call")

    # -- 2. declared historical expectations stay separate from operational contract parsing

    def test_expected_admission_matches_versions_py_and_the_gated_test_honours_it(self):
        if not shutil.which("cargo"):
            self.skipTest("cargo not on PATH: the gated Rust test cannot run")
        fixture = self.write_fixture()
        doc = versions.load_versions_json(fixture)
        want = {CLAUDE_LISTED: "listed", CLAUDE_NEWER: "optimistic", CLAUDE_BROKEN: "refused"}
        dirs = {}
        for version, expected in want.items():
            self.assertEqual(classify(doc, "claude", version), expected, "the fixture exercises each admission")
            cp, dirs[version] = self.keep_probe("claude", version, fixture, "t0.version")
            self.assertEqual(cp.returncode, 0, cp.stderr)
            probe = json.loads((dirs[version] / "canary-probe.json").read_text())
            self.assertEqual(probe["expected_admission"], expected, version)
            self.assertEqual((probe["harness"], probe["version"]), ("claude", version))
            self.assertTrue(os.path.exists(probe["binary"]), "the binary named by canary-probe.json exists")
        # Core contract selection does not identify the runtime or inherit a
        # historical recipe refusal. The diagnostic classifies only explicit
        # probe metadata, never an operational/native witness.
        cp = self.gated_cargo(dirs[CLAUDE_BROKEN], fixture)
        self.assertEqual(cp.returncode, 0, cp.stdout[-2000:] + cp.stderr[-2000:])
        self.assertIn("test result: ok. 1 passed", cp.stdout)
        rust = json.loads((dirs[CLAUDE_BROKEN] / "canary-rust.json").read_text())
        self.assertEqual(rust["observation"], "ok")
        self.assertEqual(rust.get("observation_scope"), "operational_contract")
        self.assertIsNone(rust.get("runtime_version", "missing"))
        historical = rust["historical_diagnostic"]
        self.assertEqual(historical["classification"], "refused")
        self.assertEqual(historical["version"], CLAUDE_BROKEN)
        self.assertEqual(historical["version_source"], "canary-probe.json.version")
        self.assertEqual(historical["scope"], "recipe_history_only")
        self.assertFalse(historical["runtime_qualified"])
        self.assertFalse(historical["native_qualified"])
        # The separate historical refusal with a wrong expectation fails the diagnostic.
        wrong = self.t / "wrong-expectation"
        shutil.copytree(dirs[CLAUDE_BROKEN], wrong, symlinks=True)
        probe_file = wrong / "canary-probe.json"
        probe = json.loads(probe_file.read_text())
        probe["expected_admission"] = "listed"
        probe["binary"] = str(dirs[CLAUDE_BROKEN] / "bin" / "claude")
        probe_file.write_text(json.dumps(probe))
        cp = self.gated_cargo(wrong, fixture)
        self.assertNotEqual(cp.returncode, 0, "an unexpected historical refusal must fail its diagnostic")
        self.assertIn("historical recipe diagnostic refused", cp.stdout + cp.stderr)
        wrong_rust = json.loads((wrong / "canary-rust.json").read_text())
        self.assertEqual(wrong_rust["observation"], "ok")
        self.assertEqual(wrong_rust["historical_diagnostic"]["classification"], "refused")
        # Listed and newer history remains independently classified while
        # operational runtime metadata remains absent for both.
        for version in [CLAUDE_LISTED, CLAUDE_NEWER]:
            cp = self.gated_cargo(dirs[version], fixture)
            self.assertEqual(cp.returncode, 0, cp.stdout[-2000:] + cp.stderr[-2000:])
            rust = json.loads((dirs[version] / "canary-rust.json").read_text())
            self.assertEqual(rust["observation"], "ok")
            self.assertEqual(rust["observation_scope"], "operational_contract")
            self.assertIsNone(rust["runtime_version"])
            self.assertEqual(rust["historical_diagnostic"]["classification"], want[version])
            self.assertFalse(rust["historical_diagnostic"]["runtime_qualified"])
            self.assertFalse(rust["historical_diagnostic"]["native_qualified"])

        # An expected historical refusal must never hide a real captured
        # payload's core parser failure.
        bad = self.t / "bad-core-payload"
        shutil.copytree(dirs[CLAUDE_BROKEN], bad, symlinks=True)
        captures = bad / "capture" / "tier0"
        captures.mkdir(parents=True, exist_ok=True)
        (captures / "bad.stdin").write_text('{"hook_event_name":"Bogus"}')
        (captures / "bad.argv").write_text("hook\nclaude\n--event\nSessionStart\n")
        cp = self.gated_cargo(bad, fixture)
        self.assertNotEqual(cp.returncode, 0, "historical refusal masked core payload error")
        self.assertIn("capture/tier0/bad.stdin", cp.stdout + cp.stderr)
        rust = json.loads((bad / "canary-rust.json").read_text())
        self.assertEqual(rust["historical_diagnostic"]["classification"], "refused")
        self.assertEqual(rust["observation"], "ok")
        self.assertFalse(rust["payloads"][0]["ok"])

    # -- 3. help/*.txt written by the probe, read by launch_tables

    def test_help_texts_exist_and_launch_tables_reads_them(self):
        if not shutil.which("cargo"):
            self.skipTest("cargo not on PATH: the gated Rust test cannot run")
        write_exe = tch.write_exe
        write_exe(self.t / "fake-codex.sh", textwrap.dedent(f"""\
            #!/bin/sh
            case "$*" in
              "--version") echo "codex-cli __VERSION__" ;;
              "--help") cat <<'EOF'
            {FAKE_CODEX_HELP}EOF
                ;;
              "exec --help") printf 'Run Codex non-interactively\\n\\nUsage: codex exec [OPTIONS]\\n' ;;
              *) exit 2 ;;
            esac
            """))
        write_exe(self.bin / "npm", textwrap.dedent(f"""\
            #!/bin/sh
            prefix=; pkg=
            while [ $# -gt 0 ]; do
              case $1 in --prefix) prefix=$2; shift ;; install|--*) ;; *) pkg=$1 ;; esac
              shift
            done
            dir=$prefix/node_modules/@openai/codex-fake/vendor/fake/bin
            mkdir -p "$dir" && sed "s/__VERSION__/${{pkg##*@}}/" {str(self.t / "fake-codex.sh")} > "$dir/codex" \\
              && chmod +x "$dir/codex"
            """))
        fixture = self.write_fixture()
        cp, probe_dir = self.keep_probe("codex", CODEX_BROKEN, fixture, "t0.payload-parse t0.launch-tables")
        self.assertEqual(cp.returncode, 0, cp.stdout[-3000:] + cp.stderr[-2000:])
        checks = {c["id"]: c for c in json.loads(cp.stdout)["checks"]}
        probe = json.loads((probe_dir / "canary-probe.json").read_text())
        self.assertEqual(probe["expected_admission"], "refused")
        self.assertIn("Commands:", (probe_dir / "help" / "codex.txt").read_text())
        self.assertIn("Usage: codex exec", (probe_dir / "help" / "codex-exec.txt").read_text())
        rust = json.loads((probe_dir / "canary-rust.json").read_text())
        self.assertEqual(rust["help_present"], [True, True])
        self.assertIsInstance(rust["launch_tables"], dict, "launch_tables is read from help/*.txt, not null")
        self.assertFalse(rust["launch_tables"].get("drift"), rust["launch_tables"])
        self.assertEqual(checks["t0.launch-tables"]["status"], "pass", checks["t0.launch-tables"])


if __name__ == "__main__":
    unittest.main()
