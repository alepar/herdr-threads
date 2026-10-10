"""Offline tests for the tier-1 pieces (ht-p03.14.3): capture_hook.py, --model-tier gating, the planted tier-1
failure and the tier-1 run of tier1.sh against a fake claude. No network, no real npm/cargo/harness."""
import json, os, pathlib, sys

HERE = pathlib.Path(__file__).resolve().parent
# scripts/canary/bisect.py shadows the stdlib `bisect` (needed by `random`, hence `tempfile`) when
# `discover -s scripts/canary` puts this directory first on sys.path: import the stdlib ones without it.
_saved = list(sys.path)
sys.path[:] = [p for p in sys.path if os.path.realpath(p or ".") != str(HERE)]
sys.modules.pop("bisect", None)
import shlex, shutil, stat, subprocess, tempfile, textwrap, unittest  # noqa: E402
sys.path[:] = _saved
ROOT = HERE.parent.parent
SCRIPT = str(ROOT / "scripts" / "harness-canary.sh")
HOOK = str(HERE / "capture_hook.py")
BISECT = str(HERE / "bisect.py")

SESSION_START = json.dumps({"hook_event_name": "SessionStart", "source": "startup", "session_id": "s1"})
PRE_TOOL_USE = json.dumps({"hook_event_name": "PreToolUse", "tool_name": "Bash", "session_id": "s1",
                           "tool_input": {"command": "echo canary-ok"}})


def run_hook(directory, nonce_file, stdin):
    return subprocess.run([sys.executable, HOOK, str(directory), str(nonce_file)], input=stdin,
                          capture_output=True, text=True, check=True)


class CaptureHook(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = pathlib.Path(self.tmp.name) / "capture" / "tier1"
        self.nonces = pathlib.Path(self.tmp.name) / "nonces.json"
        self.nonces.write_text(json.dumps({"SessionStart": "n1", "PreToolUse": "n2"}))

    def tearDown(self):
        self.tmp.cleanup()

    def records(self):
        return [json.loads(p.read_text()) for p in sorted(self.dir.glob("*.json"))]

    def test_session_start_and_pre_tool_use_are_captured_with_the_nonce_envelope(self):
        a = run_hook(self.dir, self.nonces, SESSION_START)
        b = run_hook(self.dir, self.nonces, PRE_TOOL_USE)
        self.assertEqual(json.loads(a.stdout), {"hookSpecificOutput": {
            "hookEventName": "SessionStart", "additionalContext": "canary-nonce-SessionStart: n1"}})
        self.assertEqual(json.loads(b.stdout), {"hookSpecificOutput": {
            "hookEventName": "PreToolUse", "additionalContext": "canary-nonce-PreToolUse: n2"}})
        recs = self.records()
        self.assertEqual([r["event"] for r in recs], ["SessionStart", "PreToolUse"])  # capture order
        self.assertEqual(recs[0]["stdin"], SESSION_START)  # the raw string, byte for byte
        self.assertEqual(recs[1]["stdin"], PRE_TOOL_USE)
        self.assertEqual(recs[0]["stdout"], a.stdout.strip())
        self.assertEqual(set(recs[0]), {"event", "stdin", "stdout"})

    def test_event_without_a_nonce_and_garbage_stdin_are_captured_silently(self):
        for raw in ('{"hook_event_name":"Stop"}', "not json"):
            self.assertEqual(run_hook(self.dir, self.nonces, raw).stdout, "")
        recs = self.records()
        self.assertEqual([r["event"] for r in recs], ["Stop", "unknown"])
        self.assertEqual(recs[1]["stdin"], "not json")
        self.assertEqual(recs[1]["stdout"], "")

    def test_two_captures_in_the_same_instant_do_not_overwrite_each_other(self):
        for _ in range(20):
            run_hook(self.dir, self.nonces, SESSION_START)
        self.assertEqual(len(self.records()), 20)

    def test_usage_error(self):
        cp = subprocess.run([sys.executable, HOOK], capture_output=True, text=True)
        self.assertEqual(cp.returncode, 64)


def write_exe(path, text):
    path.write_text(text)
    path.chmod(path.stat().st_mode | stat.S_IXUSR)


@unittest.skipUnless(shutil.which("node") and shutil.which("npm"), "the canary script needs node and npm on PATH")
class ScriptTier1(unittest.TestCase):
    """harness-canary.sh --probe against a fake npm that installs a fake claude."""
    VERSION = "2.1.286"

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        # Canonical: macOS's default TMPDIR is under the /var -> /private/var link.
        t = pathlib.Path(self.tmp.name).resolve()
        self.t = t
        self.home = t / "home"
        self.home.mkdir()
        self.log = t / "claude-calls.jsonl"
        self.cargo_log = t / "cargo-calls.jsonl"
        self.bin = t / "fakebin"
        self.bin.mkdir()
        self.ht = t / "herdr-threads"
        write_exe(self.ht, "#!/bin/sh\nexit 0\n")
        # The generic entrypoint builds the binary used for strict adapter discovery.
        # Payload-parser invocations still record their supplied capture directories below.
        discovery = json.loads((ROOT / "scripts/harness-canary-selftest/fixtures/adapter-discovery.json").read_text())
        for descriptor, key in zip(discovery["adapters"], ("ANTHROPIC_API_KEY", "OPENAI_API_KEY")):
            descriptor["canary_strategy"]["model_key_env"] = key
        self.discovery_log = t / "discovery-calls.jsonl"
        self.fixture_target = t / "cargo-target"
        self.discovery = t / "discovery"
        write_exe(self.discovery, f"#!{sys.executable}\nimport json, sys\n"
                  f"with open({str(self.discovery_log)!r}, 'a') as f: f.write(json.dumps(sys.argv[1:]) + '\\n')\n"
                  "assert sys.argv[1:] == ['adapters', '--json']\n"
                  f"print({json.dumps(discovery)!r})\n")
        write_exe(self.bin / "cargo", textwrap.dedent(f"""\
            #!{sys.executable}
            import json, os, pathlib, shutil, sys
            if sys.argv[1:2] == ["build"]:
                target = pathlib.Path(os.environ["CARGO_TARGET_DIR"])
                assert target == pathlib.Path({str(self.fixture_target)!r}), "fake Cargo requires its own target"
                binary = target / "debug" / "herdr-threads"
                binary.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2({str(self.discovery)!r}, binary)
                sys.exit(0)
            cap = os.environ.get("HT_CANARY_CAPTURE_DIR", "")
            with open({str(self.cargo_log)!r}, "a") as f:
                f.write(json.dumps({{"cap": cap, "sub": sorted(os.listdir(os.path.join(cap, "capture"))),
                                    "files": sorted(os.listdir(os.path.join(cap, "capture", "tier1")))
                                    if os.path.isdir(os.path.join(cap, "capture", "tier1")) else []}}) + "\\n")
            """))
        self.install_fake_claude(echo_nonces=True)
        write_exe(self.bin / "npm", textwrap.dedent(f"""\
            #!/bin/sh
            # fake npm: `npm install --prefix DIR ... PKG@VERSION` drops the fake claude binary
            prefix=; pkg=
            while [ $# -gt 0 ]; do
              case $1 in --prefix) prefix=$2; shift ;; install|--*) ;; *) pkg=$1 ;; esac
              shift
            done
            dir=$prefix/node_modules/@anthropic-ai/claude-code/bin
            mkdir -p "$dir" && sed "s/__VERSION__/${{pkg##*@}}/" {str(t / "fake-claude.py")} > "$dir/claude.exe" \\
              && chmod +x "$dir/claude.exe"
            """))

    def tearDown(self):
        self.tmp.cleanup()

    def install_fake_claude(self, echo_nonces):
        write_exe(self.t / "fake-claude.py", textwrap.dedent(f"""\
            #!{sys.executable}
            import json, os, subprocess, sys
            if sys.argv[1:] == ["--version"]:
                print("__VERSION__ (Claude Code)")
                sys.exit(0)
            with open({str(self.log)!r}, "a") as f:
                f.write(json.dumps({{"argv": sys.argv[1:], "cwd": os.getcwd(),
                                    "key": os.environ.get("ANTHROPIC_API_KEY"),
                                    "home": os.environ.get("HOME"),
                                    "config_dir": os.environ.get("CLAUDE_CONFIG_DIR")}}) + "\\n")
            settings = json.load(open(".claude/settings.local.json"))
            contexts = []
            for event, groups in settings["hooks"].items():
                payload = {{"hook_event_name": event, "session_id": "fake"}}
                if event == "PreToolUse":
                    payload.update(tool_name="Bash", tool_input={{"command": "echo canary-ok"}})
                cmd = groups[0]["hooks"][0]["command"]
                out = subprocess.run(cmd, shell=True, input=json.dumps(payload), capture_output=True, text=True).stdout
                if out.strip() and {echo_nonces!r}:
                    contexts.append(json.loads(out)["hookSpecificOutput"]["additionalContext"])
            print(json.dumps({{"type": "result", "result": "\\n".join(contexts)}}))
            """))

    def env(self, **extra):
        env = {k: v for k, v in os.environ.items() if k not in ("ANTHROPIC_API_KEY", "OPENAI_API_KEY",
                                                                  "HT_CANARY_PLANT_TIER1_FAIL", "CARGO_HOME")}
        # the fake claude implements --version and the tier-1 run only: keep tier 0 to the checks it can answer
        env.update(HOME=str(self.home), PATH=f"{self.bin}{os.pathsep}{env['PATH']}",
                   HT_CANARY_TIER0_CHECKS="t0.version t0.config-load t0.payload-parse", **extra)
        if (self.bin / "cargo").exists():
            env["CARGO_TARGET_DIR"] = str(self.fixture_target)
        return env

    def probe_args(self, out, *extra):
        return ["bash", SCRIPT, "--probe", "claude", self.VERSION, "--out", str(out),
                "--herdr-threads", str(self.ht), *extra]

    def probe(self, *extra, **env):
        out = self.t / "out"
        cp = subprocess.run(self.probe_args(out, *extra), env=self.env(**env), capture_output=True, text=True,
                            cwd=ROOT, timeout=300)
        try:
            doc = json.loads(cp.stdout)
        except ValueError:
            self.fail(f"no probeResult on stdout (exit {cp.returncode}): {cp.stdout!r} {cp.stderr!r}")
        return cp.returncode, doc, {c["id"]: c for c in doc["checks"]}

    def claude_calls(self):
        return [json.loads(l) for l in self.log.read_text().splitlines()] if self.log.exists() else []

    # -- gating

    def test_required_without_a_key_is_exit_2_naming_the_variable(self):
        for harness, var in (("claude", "ANTHROPIC_API_KEY"), ("codex", "OPENAI_API_KEY")):
            with self.subTest(harness=harness):
                cp = subprocess.run(["bash", SCRIPT, "--harness", harness, "--model-tier", "required",
                                     "--out", str(self.t / "out-required")], env=self.env(), capture_output=True,
                                    text=True, cwd=ROOT, timeout=60)
                self.assertEqual(cp.returncode, 2, cp.stderr)
                self.assertIn(var, cp.stderr)
                self.assertFalse((self.t / "out-required").exists(), "refused before any work")
        cp = subprocess.run(["bash", SCRIPT, "--harness", "both", "--model-tier", "required",
                             "--out", str(self.t / "out-required")], env=self.env(ANTHROPIC_API_KEY="k"),
                            capture_output=True, text=True, cwd=ROOT, timeout=60)
        self.assertEqual((cp.returncode, "OPENAI_API_KEY" in cp.stderr), (2, True), cp.stderr)  # only codex's is missing
        cp = subprocess.run(["bash", SCRIPT, "--harness", "claude", "--model-tier", "required",
                             "--out", str(self.t / "out-required")], env=self.env(ANTHROPIC_API_KEY=""),
                            capture_output=True, text=True, cwd=ROOT, timeout=60)
        self.assertEqual(cp.returncode, 2, "an empty key counts as missing")

    def test_auto_and_off_without_keys_skip_every_tier1_check_and_leave_the_exit_code(self):
        results = {}
        for tier in ("auto", "off"):
            rc, doc, checks = self.probe("--model-tier", tier)
            results[tier] = rc
            t1 = {i: c for i, c in checks.items() if i.startswith("t1.")}
            self.assertEqual(set(t1), {"t1.session-start", "t1.pre-tool-use", "t1.context-delivery",
                                       "t1.payload-parse"})
            self.assertEqual({c["status"] for c in t1.values()}, {"skip"})
            self.assertEqual(doc["failed_tier"], None, doc)
            shutil.rmtree(self.t / "out")
        self.assertEqual(results, {"auto": 0, "off": 0})
        self.assertEqual(self.claude_calls(), [], "no model call without a key")

    # -- the tier-1 run itself (fake claude stands in for the model)

    def test_tier1_run_checks_and_key_handling(self):
        rc, doc, checks = self.probe("--model-tier", "auto", "--keep", ANTHROPIC_API_KEY="sk-canary-test")
        statuses = {i: c["status"] for i, c in checks.items() if i.startswith("t1.")}
        self.assertEqual(statuses, {"t1.session-start": "pass", "t1.pre-tool-use": "pass",
                                    "t1.context-delivery": "pass", "t1.payload-parse": "pass"}, checks)
        self.assertEqual((rc, doc["result"], doc["failed_tier"]), (0, "pass", None))
        (call,) = self.claude_calls()
        self.assertEqual(call["key"], "sk-canary-test")
        self.assertNotIn("sk-canary-test", json.dumps(call["argv"]), "the key never travels in argv")
        flags = call["argv"]
        self.assertEqual(flags[0], "-p")
        self.assertIn("echo canary-ok", flags[1])
        for want in (["--model", "claude-haiku-4-5-20251001"], ["--max-budget-usd", "0.10"],
                     ["--setting-sources", "project,local"], ["--output-format", "json"]):
            i = flags.index(want[0])
            self.assertEqual(flags[i:i + 2], want)
        # fresh home: a tier-1 directory of its own, not the tier-0 one, and no key left on disk
        probe_dir = next((self.t / "out" / "work").glob("claude-*"))
        self.assertEqual(os.path.realpath(call["home"]), os.path.realpath(probe_dir / "t1" / "home"))
        self.assertFalse((probe_dir / "t1" / "key").exists())
        settings = json.loads((probe_dir / "proj" / ".claude" / "settings.local.json").read_text())
        self.assertEqual(settings["permissions"]["allow"], ["Bash(echo canary-ok)"])
        self.assertEqual(settings["hooks"]["PreToolUse"][0]["matcher"], "Bash")
        self.assertIn("SessionStart", settings["hooks"])
        # the captures are the contract files, and the Rust test saw only the tier-1 directory
        caps = [json.loads(p.read_text()) for p in sorted((probe_dir / "capture" / "tier1").glob("*.json"))]
        self.assertEqual(sorted(c["event"] for c in caps), ["PreToolUse", "SessionStart"])
        calls = [json.loads(l) for l in self.cargo_log.read_text().splitlines()]
        tier1_calls = [c for c in calls if c["cap"].endswith("/t1/cap")]
        self.assertEqual(len(tier1_calls), 1, calls)
        self.assertEqual(tier1_calls[0]["sub"], ["tier1"])
        self.assertEqual(len(tier1_calls[0]["files"]), 2)

    def test_missing_nonce_in_the_final_output_fails_context_delivery_as_tier1(self):
        self.install_fake_claude(echo_nonces=False)
        rc, doc, checks = self.probe("--model-tier", "auto", ANTHROPIC_API_KEY="sk-canary-test")
        self.assertEqual(checks["t1.context-delivery"]["status"], "fail")
        self.assertIn("SessionStart", checks["t1.context-delivery"]["detail"])
        self.assertEqual(checks["t1.session-start"]["status"], "pass")  # the hook did fire
        self.assertEqual((rc, doc["result"], doc["failed_tier"]), (1, "fail", 1))

    # -- the planted tier-1 failure

    def test_planted_tier1_failure_makes_no_model_call(self):
        rc, doc, checks = self.probe("--model-tier", "auto", HT_CANARY_PLANT_TIER1_FAIL=f"claude@{self.VERSION}:1",
                                     ANTHROPIC_API_KEY="sk-canary-test")
        self.assertEqual(checks["t1.planted"]["status"], "fail")
        self.assertEqual((rc, doc["result"], doc["failed_tier"]), (1, "fail", 1))
        self.assertEqual(self.claude_calls(), [], "the planted path returns before tier1.sh runs")
        self.assertNotIn("t1.session-start", checks)

    def test_planted_failure_for_another_version_is_inert(self):
        rc, doc, checks = self.probe("--model-tier", "auto", HT_CANARY_PLANT_TIER1_FAIL="claude@2.1.999")
        self.assertNotIn("t1.planted", checks)
        self.assertEqual((rc, doc["failed_tier"]), (0, None))

    def test_malformed_planted_spec_is_a_usage_error(self):
        out = self.t / "out"
        cp = subprocess.run(self.probe_args(out), env=self.env(HT_CANARY_PLANT_TIER1_FAIL="claude"),
                            capture_output=True, text=True, cwd=ROOT, timeout=300)
        self.assertEqual(cp.returncode, 2)
        self.assertIn("HT_CANARY_PLANT_TIER1_FAIL", cp.stderr)

    def bisect(self, planted):
        """Run bisect.py over the one candidate with a probe wrapper that counts invocations."""
        calls = self.t / "probe-invocations.txt"
        wrapper = self.t / "probe-wrapper.sh"
        out = self.t / "out-bisect"
        write_exe(wrapper, f"#!/bin/sh\necho \"$1\" >> {shlex.quote(str(calls))}\n"
                           f"exec bash {shlex.quote(SCRIPT)} --probe claude \"$1\" --out {shlex.quote(str(out))} "
                           f"--herdr-threads {shlex.quote(str(self.ht))} --model-tier auto\n")
        cp = subprocess.run([sys.executable, BISECT, "--probe-cmd", f"{wrapper} {{version}}", "--candidates",
                             self.VERSION, "--baseline", "2.1.285", "--tier1"],
                            env=self.env(HT_CANARY_PLANT_TIER1_FAIL=planted), capture_output=True, text=True,
                            timeout=600)
        res = json.loads(cp.stdout)
        return res, calls.read_text().split()

    def test_bisect_retries_a_planted_tier1_failure_twice(self):
        res, invocations = self.bisect(f"claude@{self.VERSION}")
        self.assertEqual(invocations, [self.VERSION] * 3, "one attempt plus two retries")
        self.assertEqual(res["status"], "break")
        (probe,) = res["probes"]
        self.assertEqual([a["result"] for a in probe["attempts"]], ["fail"] * 3)
        self.assertTrue(all(a["tier1"] for a in probe["attempts"]))
        self.assertEqual(res["failing_checks"][0]["id"], "t1.planted")

    def test_planted_count_bounds_the_failing_attempts(self):
        res, invocations = self.bisect(f"claude@{self.VERSION}:2")
        self.assertEqual(invocations, [self.VERSION] * 3)  # fail, fail, then the planted failure is exhausted
        (probe,) = res["probes"]
        self.assertEqual([a["result"] for a in probe["attempts"]], ["fail", "fail", "pass"])
        self.assertTrue(probe["flaky"])
        self.assertEqual(res["status"], "all_pass")


if __name__ == "__main__":
    unittest.main()
