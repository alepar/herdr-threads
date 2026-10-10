"""Offline behavioral checks: metadata is not native or model qualification."""
import importlib.util
import json
import os
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
_saved = list(sys.path)
sys.path[:] = [p for p in sys.path if os.path.realpath(p or ".") != str(HERE)]
sys.modules.pop("bisect", None)
import tempfile
import subprocess
import unittest
from unittest.mock import patch
sys.path[:] = _saved

spec = importlib.util.spec_from_file_location("strategy_runner", HERE / "run.py")
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


def adapter(name="third", kind="exact_runtime"):
    return {"id": name, "display_name": name, "host_kinds": [], "setup_scopes": [],
            "legacy_contract_id": None,
            "contracts": [{"domain": "native_shape", "origin": "native_shape_observation",
                           "id": "0123456789abcdef", "events": [{"event": "SessionStart", "milestone": "session_start", "always_send": True},
                                      {"event": "TurnEnd", "milestone": "turn_end", "always_send": True}],
                           "required_milestones": ["session_start", "turn_end"]}],
            "canary_strategy": {"kind": kind,
                               "candidate_kind": "exact_build" if kind == "exact_runtime" else "stable_release",
                               "npm_package": None if kind == "exact_runtime" else "@example/third",
                               "model_key_env": None, "companion": "scripts/canary/adapters/third.py",
                               "artifact_schema_version": 1}}


def result():
    return {"schema_version": 1, "harness": "third", "attempt": "try1", "identity": None,
            "evidence_stage": "no_model", "outcome": "complete", "reason": None,
            "domains": [{"domain": "native_shape", "origin": "native_shape_observation",
                         "contract_id": "0123456789abcdef",
                         "successful_milestones": ["session_start", "turn_end"],
                         "violations": [], "outcome": "compatible"}]}


class Strategy(unittest.TestCase):
    # Empty optional declarations are lawful observations, never verification by vacuity.
    def test_empty_required_domains_never_verify(self):
        a = adapter()
        a["contracts"][0].update(required_milestones=[], events=[])
        self.assertEqual(runner.select_adapters({"schema_version": 1, "adapters": [a]}, "all"), [a])
        r = result()
        r["identity"] = {"key": "build:c7dac5c7b327a0ef51fc1e59d0a1e00d41988d64678433420766248b554c304e",
                         "source": "git", "release_version": None, "base_version": None,
                         "derived_version": None, "commit": "b" * 40, "dirty": False, "distance": 1}
        r["domains"][0]["successful_milestones"] = []
        retained = runner.validate_result(json.dumps(r).encode(), a, "try1", "no_model")
        self.assertEqual(retained, r)
        self.assertEqual(runner.verified_domains(retained, a), [])

    def test_empty_required_generic_release_is_not_all_pass(self):
        fixture = RequiredPreflight()
        fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        a = adapter(kind="npm_release")
        a["contracts"][0].update(required_milestones=[], events=[])
        fixture.optional_fixture(a)
        for route in ("shell", "python"):
            with self.subTest(route=route):
                out = fixture.base / ("generic-" + route)
                cp = fixture.call(route, "third", "off", out=out, extra=("--versions", "latest"))
                report, index, retained = fixture.optional_observation("generic-" + route, cp, out, a)
                self.assertGreater(len(index["attempts"]), 0)
                self.assertTrue(all(r["identity"]["key"] == "release:1.2.3" for r in retained))
                self.assertTrue(all(r["domains"][0]["successful_milestones"] == [] for r in retained))
                self.assertEqual(report["harnesses"][0]["status"], "inconclusive", report)
                self.assertEqual(cp.returncode, 1, cp.stderr)

    # Catches fixed two-brand selection and treating unsupported metadata as PASS.
    def test_strategy_runner_dynamic_all_exact_runtime_and_missing_companion_are_inconclusive(self):
        doc = json.loads((HERE.parent / "harness-canary-selftest/fixtures/adapter-discovery.json").read_text())
        with tempfile.TemporaryDirectory() as d:
            d = pathlib.Path(d)
            binary = d / "discovery"
            binary.write_text("#!/usr/bin/env python3\nimport json\nprint(" + repr(json.dumps(doc)) + ")\n")
            binary.chmod(0o755)
            cp = subprocess.run(["bash", str(HERE.parent / "harness-canary.sh"), "--harness", "all",
                                 "--herdr-threads", str(binary), "--out", str(d / "out"),
                                 "--model-tier", "off"], capture_output=True, text=True, timeout=10)
            self.assertEqual(cp.returncode, 1, cp.stderr)
            observed = json.loads((d / "out" / "canary-report.json").read_text())
            self.assertEqual([b["harness"] for b in observed["harnesses"]],
                             ["claude", "codex", "third", "none"])
            self.assertEqual([b["status"] for b in observed["harnesses"]], ["inconclusive"] * 4)
        self.assertEqual([a["id"] for a in runner.select_adapters(doc, "all")],
                         ["claude", "codex", "third", "none"])
        self.assertEqual([a["id"] for a in runner.select_adapters(doc, "both")], ["claude", "codex"])
        with tempfile.TemporaryDirectory() as d:
            for a in doc["adapters"]:
                block = runner.run_strategy(a, pathlib.Path(d), "/bin/false", HERE.parents[1],
                                            model_tier="off")
                self.assertEqual(block["status"], "inconclusive")
                self.assertIsNone(block["verified_max"])
                self.assertEqual(block["candidates"], [])

    # Catches crediting another attempt, stage, domain or undeclared evidence.
    def test_strict_attempt_domain_stage_and_identity(self):
        a = adapter()
        r = result()
        self.assertEqual(runner.validate_result(json.dumps(r).encode(), a, "try1", "no_model"), r)
        for field, value in [("attempt", "other"), ("harness", "claude"), ("evidence_stage", "live"),
                             ("extra", 1), ("schema_version", True)]:
            bad = dict(r, **{field: value})
            with self.subTest(field=field), self.assertRaises(ValueError):
                runner.validate_result(json.dumps(bad).encode(), a, "try1", "no_model")
        for patch in [{"origin": "bridge_envelope"}, {"domain": "other"},
                      {"contract_id": "1111111111111111"},
                      {"successful_milestones": ["session_start", "session_start"]},
                      {"successful_milestones": ["invented"]},
                      {"outcome": "contract_violation"}]:
            bad = dict(r, domains=[dict(r["domains"][0], **patch)])
            with self.subTest(patch=patch), self.assertRaises(ValueError):
                runner.validate_result(json.dumps(bad).encode(), a, "try1", "no_model")
        bad = dict(r, domains=r["domains"] * 2)
        with self.assertRaises(ValueError):
            runner.validate_result(json.dumps(bad).encode(), a, "try1", "no_model")
        bad = dict(r, identity={"key": "build:" + "a" * 64, "source": "source",
                    "release_version": None, "base_version": None, "derived_version": None,
                    "commit": "b" * 40, "dirty": False, "distance": 1})
        with self.assertRaises(ValueError):
            runner.validate_result(json.dumps(bad).encode(), a, "try1", "no_model")
        for raw in [json.dumps(r).encode() + b"{}", b" " * 65537,
                    json.dumps(r).replace('"attempt": "try1"', '"attempt": "try1", "attempt": "try1"').encode()]:
            with self.assertRaises(ValueError):
                runner.validate_result(raw, a, "try1", "no_model")

    # Catches complete/compatible alone being mistaken for exact-domain verification.
    def test_complete_requires_identity_and_each_domain_milestone(self):
        r = result()
        self.assertEqual(runner.verified_domains(r, adapter()), [])
        r["identity"] = {"key": "release:1.2.3", "source": "npm", "release_version": "1.2.3",
                         "base_version": None, "derived_version": None, "commit": None,
                         "dirty": None, "distance": None}
        self.assertEqual(len(runner.verified_domains(r, adapter())), 1)
        r["evidence_stage"] = "source_captured"
        self.assertEqual(runner.verified_domains(r, adapter()), [])
        r["evidence_stage"] = "no_model"
        r["domains"][0]["successful_milestones"] = ["session_start"]
        self.assertEqual(runner.verified_domains(r, adapter()), [])

    # Catches traversal/symlink and mismatched indexed evidence joins.
    def test_index_identity_paths_and_bounds(self):
        with tempfile.TemporaryDirectory() as d:
            d = pathlib.Path(d)
            r = result()
            (d / "work" / "third-try1").mkdir(parents=True)
            (d / "work" / "third-try1" / "result.json").write_text(json.dumps(r))
            entry = {"harness": "third", "attempt": "try1", "identity_key": None,
                     "evidence_stage": "no_model", "result_path": "work/third-try1/result.json", "capture_paths": []}
            index = {"schema_version": 1, "attempts": [entry]}
            self.assertEqual(runner.validate_index(json.dumps(index).encode(), d, [adapter()]), index)
            for patch in [{"identity_key": "release:1.2.3"}, {"result_path": "../result.json"},
                          {"capture_paths": ["../capture"]}, {"attempt": "other"},
                          {"evidence_stage": "live"}]:
                bad = dict(index, attempts=[dict(entry, **patch)])
                with self.subTest(patch=patch), self.assertRaises(ValueError):
                    runner.validate_index(json.dumps(bad).encode(), d, [adapter()])
            (d / "escape").symlink_to("/etc/passwd")
            with self.assertRaises(ValueError):
                runner.validate_index(json.dumps(dict(index, attempts=[dict(entry, result_path="escape")])).encode(),
                                      d, [adapter()])
            with self.assertRaises(ValueError):
                runner.validate_index(b" " * 262145, d, [adapter()])

    # Catches unbounded pipes and missing process-group cancellation.
    def test_bounded_capture_timeout_and_output(self):
        for script, timeout, detail in [
            ("import time; time.sleep(30)", .1, "deadline"),
            ("import sys; sys.stdout.write('x'*70000)", 2, "stdout"),
            ("import sys; sys.stderr.write('x'*10000)", 2, "stderr")]:
            with self.subTest(detail=detail):
                with self.assertRaisesRegex(ValueError, detail):
                    runner.bounded_capture([sys.executable, "-c", script], timeout=timeout)
        rc, out, err = runner.bounded_capture([sys.executable, "-c", "print('ok')"], timeout=2)
        self.assertEqual((rc, out, err), (0, b"ok\n", b""))

    def test_legacy_normalization_keeps_release_outcome_and_measured_stage(self):
        a = adapter("claude", "npm_release")
        a["contracts"][0]["origin"] = "native_payload"
        probe = {"result": "pass", "checks": [{"id": "t0.version", "status": "pass", "detail": "ok"}],
                 "failed_tier": None,
                 "contract": {"contract_id": "legacy", "release": None,
                              "payloads": [{"kind": "ok", "event": "SessionStart", "field": None},
                                           {"kind": "ok", "event": "TurnEnd", "field": None}]}}
        self.assertTrue(callable(getattr(runner, "normalize_legacy", None)),
                        "missing normalization for owned legacy companions")
        normalized = runner.normalize_legacy(probe, a, "try1", "no_model", "1.2.3")
        self.assertEqual(normalized["identity"]["key"], "release:1.2.3")
        self.assertEqual(normalized["evidence_stage"], "no_model")
        self.assertEqual(len(runner.verified_domains(normalized, a)), 1)
        probe["contract"]["payloads"] = [{"kind": "violation", "event": "SessionStart", "field": "session_id"}]
        normalized = runner.normalize_legacy(probe, a, "try1", "no_model", "1.2.3")
        self.assertEqual(normalized["domains"][0]["outcome"], "contract_violation")
        self.assertEqual(normalized["domains"][0]["violations"], [{"event": "SessionStart", "field": "session_id"}])

    # Actual discovery descriptors: tier0 observes lifecycle, never the tool milestone.
    def test_legacy_model_off_lifecycle_pass_stays_legacy_pass(self):
        descriptors = [
            ("claude", "Claude", "3f860645de4c3363", "c4c4b249584b3578",
             "@anthropic-ai/claude-code", "ANTHROPIC_API_KEY"),
            ("codex", "Codex", "d3b98d74f26f7e2c", "7c86f2da08117f13",
             "@openai/codex", "OPENAI_API_KEY")]
        for name, display, legacy_id, rich_id, package, key in descriptors:
            with self.subTest(harness=name), tempfile.TemporaryDirectory() as d:
                root = pathlib.Path(d)
                a = {"id": name, "display_name": display, "host_kinds": [name],
                     "setup_scopes": ["config_root"], "legacy_contract_id": legacy_id,
                     "contracts": [{"domain": "native_payload", "origin": "native_payload", "id": rich_id,
                         "events": [{"event": "SessionStart", "milestone": "lifecycle", "always_send": True},
                                    {"event": "PreToolUse", "milestone": "tool", "always_send": False}],
                         "required_milestones": ["lifecycle", "tool"]}],
                     "canary_strategy": {"kind": "npm_release", "candidate_kind": "stable_release",
                         "npm_package": package, "model_key_env": key,
                         "companion": "scripts/canary/adapters/" + name + ".py", "artifact_schema_version": 1}}
                if name == "codex":
                    a["contracts"][0]["events"].append(
                        {"event": "SubagentStart", "milestone": None, "always_send": False})
                probe = {"result": "pass", "checks": [{"id": "t0.version", "status": "pass"},
                          {"id": "t0.payload-parse", "status": "pass"}, {"id": "t1.model", "status": "skip"}],
                         "failed_tier": None, "contract": {"contract_id": legacy_id, "release": None,
                         "payloads": [{"kind": "ok", "event": "SessionStart", "field": None}]}}
                companions = root / "scripts/canary/adapters"
                companions.mkdir(parents=True)
                (companions / (name + ".py")).write_text(
                    "import importlib.util,json,sys,pathlib\n"
                    "s=importlib.util.spec_from_file_location('runner'," + repr(str(HERE / "run.py")) + ")\n"
                    "runner=importlib.util.module_from_spec(s);s.loader.exec_module(runner)\n"
                    "args=dict(zip(sys.argv[1::2],sys.argv[2::2]))\n"
                    "work=pathlib.Path(args['--work-dir'])\n"
                    "q=json.loads((work/'request.json').read_text())\n"
                    "probe=" + repr(probe) + "\n"
                    "(work/'legacy-probe.json').write_text(json.dumps(probe))\n"
                    "print(json.dumps(runner.normalize_legacy(probe,q['adapter'],args['--attempt'],args['--stage'],q['version'])))\n")
                npm = root / "npm"
                npm.write_text("#!" + sys.executable + "\nprint('[\"1.2.3\"]')\n")
                npm.chmod(0o755)
                with patch.dict(os.environ, PATH=str(root) + os.pathsep + os.environ["PATH"]):
                    index = {"schema_version": 1, "attempts": []}
                    block = runner.run_strategy(a, root / "out", "/bin/false", root,
                                                model_tier="off", versions_mode="latest", index=index)
                self.assertEqual(block["status"], "all_pass", block)
                retained = json.loads((root / "out" / index["attempts"][0]["result_path"]).read_text())
                self.assertEqual(retained["domains"][0]["successful_milestones"], ["lifecycle"])
                self.assertEqual(runner.verified_domains(retained, a), [])

    # HOME isolation must not hide caller-default rustup/cargo from the owned parser.
    def test_unset_toolchain_homes_reach_owned_payload_parser(self):
        with tempfile.TemporaryDirectory() as d:
            root = pathlib.Path(d)
            caller = root / "caller"
            cargo = caller / ".cargo/bin/cargo"
            cargo.parent.mkdir(parents=True)
            (caller / ".rustup").mkdir()
            cargo.write_text("#!/bin/sh\n"
                '[ "$CARGO_HOME" = "' + str(caller / ".cargo") + '" ] || exit 21\n'
                '[ "$RUSTUP_HOME" = "' + str(caller / ".rustup") + '" ] || exit 22\n'
                '[ "$HOME" != "' + str(caller) + '" ] || exit 23\n'
                "printf '%s' '{\"observation\":\"ok\",\"payloads\":[]}' > \"$HT_CANARY_CAPTURE_DIR/canary-rust.json\"\n")
            cargo.chmod(0o755)
            with patch.dict(os.environ, HOME=str(caller), PATH=str(cargo.parent) + os.pathsep + os.environ["PATH"]):
                os.environ.pop("CARGO_HOME", None)
                os.environ.pop("RUSTUP_HOME", None)
                env = runner.isolated_env(root / "attempt", adapter()["canary_strategy"], "no_model")
            probe = root / "probe"
            for rel in ("logs", "capture/tier0", "tmp"):
                (probe / rel).mkdir(parents=True, exist_ok=True)
            body = ("set -euo pipefail\nexport HT_CANARY_SOURCE_ONLY=1\n"
                    "source \"$1\" --out \"$2/out\" --model-tier off\n"
                    "P=\"$2/probe\"; LOGS=\"$P/logs\"; H=claude; V=1.2.3; HOOK_FIRES_RAN=0\n"
                    "build_env \"$P\"\nchk_t0_payload_parse\nprintf '%s' \"$CK_STATUS\"\n")
            cp = subprocess.run(["bash", "-c", body, str(HERE.parent / "harness-canary.sh"),
                                 str(HERE.parent / "harness-canary.sh"), str(root)],
                                env=env, capture_output=True, text=True, timeout=10)
            self.assertEqual(cp.returncode, 0, cp.stderr)
            self.assertEqual(cp.stdout, "pass", (cp.stdout, cp.stderr,
                             (probe / "logs/payload-parse.err").read_text()))
            self.assertEqual(env["CARGO_HOME"], str(caller / ".cargo"))
            self.assertEqual(env["RUSTUP_HOME"], str(caller / ".rustup"))
            with patch.dict(os.environ, CARGO_HOME=str(root / "explicit-cargo"),
                            RUSTUP_HOME=str(root / "explicit-rustup")):
                explicit = runner.isolated_env(root / "explicit-attempt", adapter()["canary_strategy"], "no_model")
            self.assertEqual(explicit["CARGO_HOME"], str(root / "explicit-cargo"))
            self.assertEqual(explicit["RUSTUP_HOME"], str(root / "explicit-rustup"))

    # Attributed violations cannot turn incomplete/failed operations into known-broken releases.
    def test_noncomplete_violations_never_classify_break(self):
        for kind in ("exact_runtime", "npm_release"):
            for outcome, expected in [("infra_failure", "infra_error" if kind == "npm_release" else "inconclusive"),
                                      ("unsupported", "inconclusive"), ("inconclusive", "inconclusive"),
                                      ("complete", "break")]:
                with self.subTest(kind=kind, outcome=outcome), tempfile.TemporaryDirectory() as d:
                    root = pathlib.Path(d)
                    companions = root / "scripts/canary/adapters"
                    companions.mkdir(parents=True)
                    a = adapter(kind=kind)
                    r = result()
                    r.update(outcome=outcome, reason="fixture outcome")
                    r["domains"][0].update(outcome="contract_violation", successful_milestones=[],
                                            violations=[{"event": "SessionStart", "field": "session_id"}])
                    r["identity"] = {"key": "release:1.2.3", "source": "npm", "release_version": "1.2.3",
                                     "base_version": None, "derived_version": None, "commit": None,
                                     "dirty": None, "distance": None}
                    if kind == "exact_runtime":
                        r["identity"] = {"key": "build:c7dac5c7b327a0ef51fc1e59d0a1e00d41988d64678433420766248b554c304e",
                                         "source": "git", "release_version": None, "base_version": None,
                                         "derived_version": None, "commit": "b" * 40, "dirty": False, "distance": 1}
                    (companions / "third.py").write_text(
                        "import json,sys,pathlib\nargs=dict(zip(sys.argv[1::2],sys.argv[2::2]))\n"
                        "r=" + repr(r) + "\nr['attempt']=args['--attempt']\n"
                        "q=json.loads(pathlib.Path(args['--work-dir'],'request.json').read_text())\n"
                        "if q.get('version') == '1.2.2':\n"
                        " r['identity'].update(key='release:1.2.2',release_version='1.2.2')\n"
                        " r['outcome']='complete'\n"
                        " r['domains'][0].update(outcome='compatible',successful_milestones=['session_start','turn_end'],violations=[])\n"
                        "print(json.dumps(r))\n")
                    npm = root / "npm"
                    npm.write_text("#!" + sys.executable + "\nprint('[\"1.2.3\"]')\n")
                    npm.chmod(0o755)
                    runtime = root / "runtime.json"
                    runtime.write_text('["fixture"]')
                    with patch.dict(os.environ, PATH=str(root) + os.pathsep + os.environ["PATH"]):
                        block = runner.run_strategy(a, root / "out", "/bin/false", root,
                                      model_tier="off", versions_mode="latest", runtime_command=runtime, baseline="1.2.2", bisect=True)
                    self.assertEqual(block["status"], expected, block)
                    if outcome != "complete":
                        self.assertIsNone(block["suggested_action"])
                        self.assertIsNone(block["first_bad"])
                    elif kind == "npm_release":
                        self.assertEqual(block["first_bad"], "1.2.3")

    def test_discovery_rejects_malformed_strategy_and_domains(self):
        a = adapter()
        self.assertEqual(runner.select_adapters({"schema_version": 1, "adapters": [a]}, "all"), [a])
        for bad in [dict(a, extra=1), dict(a, contracts=a["contracts"] * 2),
                    dict(a, canary_strategy=dict(a["canary_strategy"], companion="../bad.py")),
                    dict(a, canary_strategy=dict(a["canary_strategy"], candidate_kind="stable_release")),
                    dict(a, canary_strategy=dict(a["canary_strategy"], model_key_env="bad-name")),
                    dict(a, contracts=[dict(a["contracts"][0], required_milestones=["invented"])])]:
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                runner.select_adapters({"schema_version": 1, "adapters": [bad]}, "all")

    def test_model_key_and_companion_path_are_explicit(self):
        with tempfile.TemporaryDirectory() as d:
            a = adapter()
            work = pathlib.Path(d)
            env = runner.isolated_env(work, a["canary_strategy"], "no_model")
            self.assertTrue(pathlib.Path(env["HOME"]).is_relative_to(work))
            self.assertNotIn("OPENAI_API_KEY", env)
            for path in ["../else.py", "scripts/canary/adapters/../else.py", "/tmp/run.py"]:
                with self.subTest(path=path), self.assertRaises(ValueError):
                    runner.companion_path(work, dict(a["canary_strategy"], companion=path))

    def test_exact_runtime_companion_argv_isolated_env_and_index(self):
        with tempfile.TemporaryDirectory() as d:
            d = str(pathlib.Path(d).resolve())  # canonical: macOS TMPDIR is under a /var link
            root = pathlib.Path(d) / "repo"
            companions = root / "scripts/canary/adapters"
            companions.mkdir(parents=True)
            a = adapter()
            r = result()
            r["identity"] = {"key": "build:c7dac5c7b327a0ef51fc1e59d0a1e00d41988d64678433420766248b554c304e",
                             "source": "git", "release_version": None, "base_version": None,
                             "derived_version": None, "commit": "b" * 40, "dirty": False, "distance": 1}
            source = ("import json,sys,os,pathlib\n"
                      "a=dict(zip(sys.argv[1::2],sys.argv[2::2]))\n"
                      "r=" + repr(r) + "\n"
                      "r['attempt']=a['--attempt']\n"
                      "r['evidence_stage']=a['--stage']\n"
                      "pathlib.Path(a['--work-dir'],'observed.json').write_text(json.dumps("
                      "{'argv':a,'home':os.environ['HOME'],'unexpected_key':'OPENAI_API_KEY' in os.environ}))\n"
                      "print(json.dumps(r))\n")
            (companions / "third.py").write_text(source)
            command = pathlib.Path(d) / "runtime.json"
            command.write_text('["explicit-readonly-fixture-input"]')
            out = pathlib.Path(d) / "out"
            index = {"schema_version": 1, "attempts": []}
            block = runner.run_strategy(a, out, "/bin/false", root, model_tier="off",
                                        runtime_command=command, index=index)
            self.assertEqual(block["status"], "all_pass", block)
            self.assertIsNone(block["verified_max"])
            self.assertEqual(block["candidates"], [])
            self.assertEqual(block["identity"]["key"], r["identity"]["key"])
            self.assertEqual(len(index["attempts"]), 1)
            observed = json.loads((out / "work/third-attempt-1/observed.json").read_text())
            self.assertEqual(observed["argv"]["--runtime-command-file"], str(command))
            self.assertFalse(observed["unexpected_key"])
            self.assertTrue(pathlib.Path(observed["home"]).is_relative_to(out))
            self.assertEqual(runner.validate_index(json.dumps(index).encode(), out, [a]), index)
            # Redaction must not expand a bounded diagnostic into an oversized artifact.
            (companions / "third.py").write_text(source + "sys.stderr.write('k'*8192)\n")
            a["canary_strategy"]["model_key_env"] = "CANARY_FIXTURE_KEY"
            prior = os.environ.get("CANARY_FIXTURE_KEY")
            os.environ["CANARY_FIXTURE_KEY"] = "k"
            try:
                diagnostic = pathlib.Path(d) / "diagnostic"
                measured = runner.run_strategy(a, diagnostic, "/bin/false", root, model_tier="off",
                                               runtime_command=command)
            finally:
                if prior is None:
                    os.environ.pop("CANARY_FIXTURE_KEY")
                else:
                    os.environ["CANARY_FIXTURE_KEY"] = prior
            self.assertEqual(measured["status"], "all_pass", measured)
            self.assertLessEqual(len((diagnostic / "work/third-attempt-1/stderr.txt").read_bytes()), 8192)

    def test_npm_strategy_keeps_bisect_and_descriptor_package(self):
        with tempfile.TemporaryDirectory() as d:
            root = pathlib.Path(d) / "repo"
            companions = root / "scripts/canary/adapters"
            companions.mkdir(parents=True)
            a = adapter(kind="npm_release")
            r = result()
            source = ("import json,sys,pathlib\n"
                      "a=dict(zip(sys.argv[1::2],sys.argv[2::2]))\n"
                      "q=json.loads(pathlib.Path(a['--work-dir'],'request.json').read_text())\n"
                      "v=q['version']\n"
                      "r=" + repr(r) + "\n"
                      "r['attempt']=a['--attempt']\n"
                      "r['identity']={'key':'release:'+v,'source':'npm','release_version':v,"
                      "'base_version':None,'derived_version':None,'commit':None,'dirty':None,'distance':None}\n"
                      "if v in ('1.0.2','1.0.3'):\n"
                      " r['domains'][0].update(outcome='contract_violation',successful_milestones=[],"
                      "violations=[{'event':'SessionStart','field':'session_id'}])\n"
                      "print(json.dumps(r))\n")
            (companions / "third.py").write_text(source)
            npm = pathlib.Path(d) / "npm"
            npm.write_text("#!" + sys.executable + "\nimport json,sys,pathlib\n"
                           "pathlib.Path(" + repr(str(pathlib.Path(d) / "npm-argv.json")) +
                           ").write_text(json.dumps(sys.argv[1:]))\n"
                           "print('[\"1.0.0\",\"1.0.1\",\"1.0.2\",\"1.0.3\",\"2.0.0-beta\"]')\n")
            npm.chmod(0o755)
            old_path = os.environ["PATH"]
            os.environ["PATH"] = str(pathlib.Path(d)) + os.pathsep + old_path
            try:
                index = {"schema_version": 1, "attempts": []}
                block = runner.run_strategy(a, pathlib.Path(d) / "out", "/bin/false", root,
                       model_tier="off", bisect=True, index=index,
                       baseline_doc={"rows": [{"harness": "third", "version": "1.0.0", "status": "verified"}]})
            finally:
                os.environ["PATH"] = old_path
            self.assertEqual(block["status"], "break", block)
            self.assertEqual(block["candidates"], ["1.0.1", "1.0.2", "1.0.3"])
            self.assertEqual((block["first_bad"], block["last_good"]), ("1.0.2", "1.0.1"))
            self.assertEqual(block["evidence_stage"], "no_model")
            self.assertEqual(json.loads((pathlib.Path(d) / "npm-argv.json").read_text()),
                             ["view", "@example/third", "versions", "--json"])
            self.assertGreater(len(index["attempts"]), 1)
            self.assertEqual(runner.validate_index(json.dumps(index).encode(), pathlib.Path(d) / "out", [a]), index)
            (companions / "third.py").write_text(source.replace("print(json.dumps(r))", "r['identity']=None\nprint(json.dumps(r))"))
            os.environ["PATH"] = str(pathlib.Path(d)) + os.pathsep + old_path
            try:
                incomplete = runner.run_strategy(a, pathlib.Path(d) / "incomplete", "/bin/false", root,
                                                  model_tier="off", versions_mode="latest")
            finally:
                os.environ["PATH"] = old_path
            self.assertEqual(incomplete["status"], "inconclusive", incomplete)

    def test_legacy_index_keeps_tier0_stdin_and_argv(self):
        self.assertTrue(callable(getattr(runner, "index_legacy_captures", None)),
                        "owned legacy tier0 captures are not indexed")
        with tempfile.TemporaryDirectory() as d:
            work = pathlib.Path(d)
            capture = work / "legacy/work/claude-1.2.3-1/capture/tier0"
            capture.mkdir(parents=True)
            (capture / "one.stdin").write_text('{"hook_event_name":"SessionStart"}')
            (capture / "one.argv").write_text("hook\nclaude\nSessionStart\n")
            r = result()
            r["domains"][0]["origin"] = "native_payload"
            names = runner.index_legacy_captures(work, r, "claude", "1.2.3", "no_model")
            metadata = [json.loads((work / p).read_text()) for p in names]
            self.assertEqual(sorted(pathlib.Path(m["path"]).suffix for m in metadata), [".argv", ".stdin"])
            self.assertEqual({(m["domain"], m["origin"], m["evidence_stage"]) for m in metadata},
                             {("native_shape", "native_payload", "no_model")})
            (capture / "one.stdin").write_bytes(b"x" * 65537)
            with self.assertRaises(ValueError):
                runner.index_legacy_captures(work, r, "claude", "1.2.3", "no_model")

    def test_owned_final_output_parsing(self):
        for name, raw, expected in [
                ("claude", '{"type":"result","result":"nonce"}', "nonce"),
                ("codex", '{"item":{"type":"agent_message","text":"old"}}\n'
                          '{"item":{"type":"agent_message","text":"nonce"}}', "nonce")]:
            spec = importlib.util.spec_from_file_location("owned_" + name, HERE / "adapters" / (name + ".py"))
            owned = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(owned)
            self.assertTrue(callable(getattr(owned, "final_text", None)), "owned output parser absent")
            self.assertEqual(owned.final_text(raw), expected)
            self.assertEqual(owned.final_text("bad JSON"), "")

    def test_build_report_never_suggests_semver(self):
        report = runner._sibling("report")
        self.assertIsNone(report.suggested_action({"status": "break", "first_bad": "build:" + "a"*64,
                                                   "candidate_kind": "exact_build"}))

    def test_summary_labels_missing_companion_and_exact_stage(self):
        report = runner._sibling("report")
        block = runner.inconclusive(adapter(), "missing companion")
        block["identity"] = {"key": "build:" + "a" * 64, "release_version": None}
        rendered = report.render_summary(report.assemble([block], {}, {"os": "fixture", "arch": "fixture"},
                                                        "fixture", "fixture"))
        self.assertIn("missing companion", rendered)
        self.assertIn("no_model", rendered)
        self.assertIn("build:" + "a" * 64, rendered)
        self.assertNotIn("contradict monotonicity", rendered)

    def test_stable_candidates_keep_existing_selection(self):
        v = runner._sibling("versions")
        doc = {"rows": [{"harness": "third", "version": "1.0.0", "status": "verified"}]}
        self.assertEqual(v.candidates(["1.0.0", "1.0.1", "1.0.2-beta"], "since-verified", doc, "third"),
                         ["1.0.1"])


class LegacyCompanionLoader(unittest.TestCase):
    def load(self, companion, harness="claude", before=""):
        script = HERE.parent / "harness-canary.sh"
        body = ('selected=$1; selected_harness=$2; selected_out=$3; set --; '
                'HT_CANARY_SOURCE_ONLY=1 . "$0" --out "$selected_out"; ' + before +
                'HT_CANARY_COMPANION=$selected; PROBE_HARNESS=$selected_harness; load_probe_companion; '
                'load_probe_companion; '
                'for fn in probe_one tier1_keyvar tier1_key adapter_model_run; do declare -F "$fn"; done')
        with tempfile.TemporaryDirectory() as out:
            return runner.bounded_capture(["bash", "-c", body, str(script), str(companion), harness, out], timeout=10)

    def test_both_owned_emitters_load_repeatedly_at_source_only_boundary(self):
        for harness, key in (("claude", "ANTHROPIC_API_KEY"), ("codex", "OPENAI_API_KEY")):
            with self.subTest(harness=harness):
                companion = HERE / "adapters" / f"{harness}.py"
                emitted = runner.emit_shell_functions(companion)
                self.assertIn(f"tier1_keyvar() {{ echo {key}; }}", emitted)
                code, out, err = self.load(companion, harness)
                self.assertEqual(code, 0, err)
                self.assertEqual(out.decode().splitlines(),
                                 ["probe_one", "tier1_keyvar", "tier1_key", "adapter_model_run"])

    def test_emitter_failure_and_missing_functions_are_infrastructure(self):
        for harness in ("claude", "codex"):
            with tempfile.TemporaryDirectory() as d, self.subTest(harness=harness):
                broken = pathlib.Path(d) / "companion.py"
                # Even valid partial shell output must not hide the emitter's failure.
                broken.write_text("print('probe_one() { :; }')\nraise SystemExit(23)\n")
                code, _, err = self.load(broken, harness)
                self.assertEqual(code, 2)
                self.assertIn(b"emitter exited 23", err)
                # A previous good load cannot supply missing definitions to the next load.
                owned = HERE / "adapters" / f"{harness}.py"
                before = f'HT_CANARY_COMPANION="{owned}"; PROBE_HARNESS=$selected_harness; load_probe_companion; '
                broken.write_text("print('probe_one() { :; }')\n")
                code, _, err = self.load(broken, harness, before)
                self.assertEqual(code, 2)
                self.assertIn(b"missing function: tier1_keyvar", err)

    def test_emission_is_bounded_and_rejects_invalid_shell_bytes(self):
        with tempfile.TemporaryDirectory() as d:
            companion = pathlib.Path(d) / "companion.py"
            for script, message in (("print('x' * 65537)", "stdout limit"),
                                    ("import time; time.sleep(5)", "deadline"),
                                    ("import sys; sys.stdout.buffer.write(b'\\xff')", "invalid probe"),
                                    ("print('\\x00')", "invalid probe"),
                                    ("print('')", "invalid probe")):
                with self.subTest(script=script):
                    companion.write_text(script)
                    with self.assertRaisesRegex(ValueError, message):
                        runner.emit_shell_functions(companion, timeout=.2)

    def test_selected_wrong_harness_fails_before_installation(self):
        with tempfile.TemporaryDirectory() as d:
            # Keep even a regressed guard offline: the installer remains an inert fixture.
            fakebin = pathlib.Path(d) / "bin"
            fakebin.mkdir()
            npm = fakebin / "npm"
            npm.write_text("#!/bin/sh\nexit 23\n")
            npm.chmod(0o755)
            # Call the selected real probe function: its guard precedes any npm/model child.
            script = HERE.parent / "harness-canary.sh"
            for harness, wrong in (("claude", "codex"), ("codex", "claude")):
                with self.subTest(harness=harness):
                    env = dict(os.environ, HT_CANARY_COMPANION=str(HERE / "adapters" / f"{wrong}.py"),
                               PATH=str(fakebin) + os.pathsep + os.environ["PATH"])
                    code, out, err = runner.bounded_capture(
                        ["bash", str(script), "--probe", harness, "1.2.3", "--model-tier", "off",
                         "--out", str(pathlib.Path(d) / harness)], timeout=10, env=env)
                    self.assertEqual(code, 2)
                    self.assertIn(b"wrong companion harness", err)
                    self.assertEqual(out, b"")
                    self.assertEqual(list((pathlib.Path(d) / harness / "work").iterdir()), [])


class RequiredPreflight(unittest.TestCase):
    """Actual shell and Python entrypoints with bounded offline discovery and dispatch."""

    def setUp(self):
        import shutil
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        # Canonical: macOS's default TMPDIR is under the /var -> /private/var link.
        self.base = pathlib.Path(self.tmp.name).resolve()
        self.root = self.base / "repo"
        self.canary = self.root / "scripts/canary"
        self.canary.mkdir(parents=True)
        # A copied ROOT needs its own Git boundary even when TMPDIR is under an ignored checkout path.
        rc, _, err = runner.bounded_capture(["git", "init", "-q", str(self.root)], timeout=5)
        self.assertEqual(rc, 0, err)
        # Copy the real consumer sources, without adding a production companion or shortcut.
        for name in ("run.py", "report.py", "versions.py", "isolation.py",
                     "companion_schema.json", "artifact_index_schema.json"):
            shutil.copy2(HERE / name, self.canary / name)
        shutil.copy2(HERE.parent / "harness-canary.sh", self.root / "scripts/harness-canary.sh")
        (self.root / "Cargo.toml").write_text('[package]\nversion = "1.2.3"\n')
        baseline = self.root / "docs/compatibility/harness-versions.json"
        baseline.parent.mkdir(parents=True)
        baseline.write_text('{"schema_version":1,"rows":[]}')
        self.bin = self.base / "bin"
        self.bin.mkdir()
        self.discovery = self.bin / "discovery"
        self.discovery_log = self.base / "discovery.jsonl"
        self.calls = self.base / "calls.jsonl"
        self.home = self.base / "home"
        self.home.mkdir()
        self.env = {"PATH": str(self.bin) + os.pathsep + os.environ["PATH"],
                    "HOME": str(self.home), "TMPDIR": str(self.base),
                    "LANG": "C.UTF-8", "PYTHONDONTWRITEBYTECODE": "1"}
        if "HT_LEAK_RUN_ID" in os.environ:
            self.env["HT_LEAK_RUN_ID"] = os.environ["HT_LEAK_RUN_ID"]
        sentinel = (f"#!{sys.executable}\nimport pathlib,sys\n"
                    f"with pathlib.Path({str(self.calls)!r}).open('a') as f: f.write('forbidden npm/model\\n')\n"
                    "sys.exit(91)\n")
        for name in ("npm", "claude", "codex", "model"):
            (self.bin / name).write_text(sentinel)
            (self.bin / name).chmod(0o755)
        self.runtime = self.base / "runtime.json"
        self.runtime.write_text('["synthetic-input"]')
        companions = self.canary / "adapters"
        companions.mkdir()
        self.companion = companions / "third.py"
        r = result()
        r["identity"] = {"key": "build:c7dac5c7b327a0ef51fc1e59d0a1e00d41988d64678433420766248b554c304e",
                         "source": "git", "release_version": None, "base_version": None,
                         "derived_version": None, "commit": "b" * 40, "dirty": False, "distance": 1}
        self.companion.write_text("import json,sys,os,pathlib\n"
            "args=dict(zip(sys.argv[1::2],sys.argv[2::2]))\n"
            f"with pathlib.Path({str(self.calls)!r}).open('a') as f: f.write(args['--harness']+' '+args['--stage']+'\\n')\n"
            f"r={r!r}\nr.update(harness=args['--harness'],attempt=args['--attempt'],evidence_stage=args['--stage'])\n"
            "print(json.dumps(r))\n")

    def optional_fixture(self, a, *, outcome="complete", missing=False, identity=True, violation=False):
        """Inert producer uses the actual request's exact descriptor, never patches consumers."""
        self.registry([a])
        self.companion.write_text("import json,sys,os,pathlib\n"
            "args=dict(zip(sys.argv[1::2],sys.argv[2::2]))\n"
            "work=pathlib.Path(args['--work-dir'])\n"
            "q=json.loads((work/'request.json').read_text())\n"
            "a=q['adapter']\n"
            f"with pathlib.Path({str(self.calls)!r}).open('a') as f: f.write(args['--harness']+' '+args['--stage']+'\\n')\n"
            "ident={'key':'build:c7dac5c7b327a0ef51fc1e59d0a1e00d41988d64678433420766248b554c304e',"
            "'source':'git','release_version':None,'base_version':None,'derived_version':None,"
            "'commit':'" + "b" * 40 + "','dirty':False,'distance':1}\n"
            "if 'version' in q:\n"
            " ident.update(key='release:'+q['version'],source='npm',release_version=q['version'],commit=None,dirty=None,distance=None)\n"
            "domains=[]\n"
            "for c in a['contracts']:\n"
            " successes=list(c['required_milestones'])\n"
            f" if {missing!r} and successes: successes.pop()\n"
            " domains.append({'domain':c['domain'],'origin':c['origin'],'contract_id':c['id'],"
            "'successful_milestones':successes,'violations':[],'outcome':'compatible'})\n"
            f"if {violation!r}:\n"
            " domains[0].update(outcome='contract_violation',successful_milestones=[],"
            "violations=[{'event':a['contracts'][0]['events'][0]['event'],'field':'session_id'}])\n"
            f"r={{'schema_version':1,'harness':args['--harness'],'attempt':args['--attempt'],"
            f"'identity':ident if {identity!r} else None,'evidence_stage':args['--stage'],"
            f"'outcome':{outcome!r},'reason':None,'domains':domains}}\n"
            "(work/'observed.json').write_text(json.dumps({'pid':os.getpid(),'argv':args,'cwd':os.getcwd(),"
            "'request':q,'home':os.environ['HOME'],'credential_present':any(k.endswith('_KEY') for k in os.environ)}))\n"
            "print(json.dumps(r))\n")
        if a["canary_strategy"] and a["canary_strategy"]["kind"] == "npm_release":
            import shutil
            shutil.copy2(HERE / "bisect.py", self.canary / "bisect.py")
            npm = self.bin / "npm"
            npm.write_text(f"#!{sys.executable}\nimport json,pathlib,sys\n"
                "assert sys.argv[1:] == ['view','@example/third','versions','--json']\n"
                f"with pathlib.Path({str(self.base / 'npm.jsonl')!r}).open('a') as f: f.write(json.dumps(sys.argv[1:])+'\\n')\n"
                "print('[\"1.2.3\"]')\n")
            npm.chmod(0o755)

    def optional_observation(self, label, cp, out, a):
        report = json.loads((out / "canary-report.json").read_text())
        index = json.loads((out / "artifact-index.json").read_text())
        retained = [json.loads((out / e["result_path"]).read_text()) for e in index["attempts"]]
        # Save raw product outputs before assertions and before TemporaryDirectory cleanup, including RED.
        evidence = os.environ.get("HT_TASK44_EVIDENCE")
        if evidence:
            import shutil
            dest = pathlib.Path(evidence) / label
            dest.mkdir(parents=True)
            shutil.copytree(out, dest / "out")
            for name in ("discovery.jsonl", "calls.jsonl", "npm.jsonl"):
                if (self.base / name).exists():
                    shutil.copy2(self.base / name, dest / name)
            (dest / "invocation.json").write_text(json.dumps({"argv":cp.args,"exit":cp.returncode,
                "stdout":cp.stdout,"stderr":cp.stderr,"fixture_root":str(self.base),"env":self.env,
                "consumer_sha256":{n:runner.hashlib.sha256((self.canary/n).read_bytes()).hexdigest()
                                   for n in ("run.py", "report.py", "versions.py", "isolation.py")}}, indent=2))
        self.assertEqual(cp.returncode, report["exit_code"], cp.stderr)
        self.assertEqual(runner._sibling("report").exit_code(report), cp.returncode)
        self.assertEqual(runner.validate_index(json.dumps(index).encode(), out, [a]), index)
        for entry, r in zip(index["attempts"], retained):
            self.assertEqual(runner.validate_result(json.dumps(r).encode(), a, entry["attempt"], r["evidence_stage"]), r)
            observed = json.loads((out / pathlib.Path(entry["result_path"]).parent / "observed.json").read_text())
            self.assertEqual(observed["request"]["adapter"], a)
            self.assertFalse(observed["credential_present"])
            self.assertTrue(pathlib.Path(observed["home"]).is_relative_to(out))
            with self.assertRaises(ProcessLookupError):
                os.kill(observed["pid"], 0)
        if self.calls.exists():
            self.assertNotIn("forbidden", self.calls.read_text())
        self.assertEqual(json.loads(self.discovery_log.read_text().splitlines()[-1]), ["adapters", "--json"])
        return report, index, retained

    def test_empty_required_exact_runtime_is_inconclusive_through_report(self):
        a = adapter()
        a["contracts"][0].update(required_milestones=[], events=[])
        self.optional_fixture(a)
        for route in ("shell", "python"):
            with self.subTest(route=route):
                out = self.base / ("exact-" + route)
                cp = self.call(route, "third", "off", out=out)
                report, index, retained = self.optional_observation("exact-" + route, cp, out, a)
                self.assertEqual(len(index["attempts"]), 1)
                self.assertEqual(retained[0]["domains"][0]["successful_milestones"], [])
                block = report["harnesses"][0]
                self.assertEqual(block["identity"], retained[0]["identity"])
                self.assertEqual(block["domains"], retained[0]["domains"])
                self.assertEqual(block["status"], "inconclusive", report)
                self.assertEqual(cp.returncode, 1, cp.stderr)

    def test_optional_evidence_boundaries_preserve_positive_and_negative_results(self):
        # Literal expected verdicts cover shared verification and distinct infrastructure semantics.
        cases = [("empty", "inconclusive"), ("observer_events", "inconclusive"),
                 ("mixed", "inconclusive"), ("zero", "inconclusive"), ("positive", "all_pass"),
                 ("missing_milestone", "inconclusive"), ("no_identity", "inconclusive"),
                 ("source", "inconclusive"), ("violation", "break"),
                 ("infra_failure", "infra_error"), ("unsupported", "inconclusive"),
                 ("inconclusive", "inconclusive"), ("no_provider", "inconclusive"),
                 ("no_runtime", "inconclusive"), ("no_companion", "inconclusive")]
        for kind in ("exact_runtime", "npm_release"):
            for label, expected in cases:
                if kind == "npm_release" and label == "no_runtime":
                    continue  # Stable-release invocation has no explicit command prerequisite.
                a = adapter(kind=kind)
                outcome = label if label in ("infra_failure", "unsupported", "inconclusive") else "complete"
                if label in ("empty", "observer_events"):
                    a["contracts"][0].update(required_milestones=[], events=[] if label == "empty" else [
                        {"event": "Observe", "milestone": None, "always_send": False}])
                elif label == "mixed":
                    a["contracts"].append({"domain":"observer", "origin":"bridge_envelope",
                        "id":"1111111111111111", "events":[], "required_milestones":[]})
                elif label == "zero":
                    a["contracts"] = []
                elif label == "no_provider":
                    a["canary_strategy"] = None
                self.optional_fixture(a, outcome=outcome, missing=label == "missing_milestone",
                                      identity=label != "no_identity", violation=label == "violation")
                if label == "no_runtime":
                    self.runtime.unlink()
                elif label == "no_companion":
                    self.companion.unlink()
                if kind == "exact_runtime" and label == "infra_failure":
                    expected = "inconclusive"
                for route in ("shell", "python"):
                    with self.subTest(kind=kind, case=label, route=route):
                        out = self.base / (kind + "-" + label + "-" + route)
                        self.calls.unlink(missing_ok=True)
                        extra = ("--versions", "latest", "--evidence-stage",
                                 "source_captured" if label == "source" else "no_model")
                        cp = self.call(route, "third", "off", out=out, extra=extra)
                        report, index, retained = self.optional_observation(out.name, cp, out, a)
                        block = report["harnesses"][0]
                        self.assertEqual(block["status"], expected, report)
                        self.assertEqual(cp.returncode, {"all_pass":0,"inconclusive":1,"break":1,"infra_error":2}[expected])
                        if expected != "break":
                            self.assertIsNone(block["first_bad"])
                        if label in ("no_provider", "no_runtime", "no_companion"):
                            self.assertEqual(index["attempts"], [])
                            self.assertFalse(self.calls.exists())
                        else:
                            self.assertGreater(len(index["attempts"]), 0)
                            for r in retained:
                                self.assertEqual(r["outcome"], outcome)
                                self.assertEqual([(d["domain"],d["origin"],d["contract_id"]) for d in r["domains"]],
                                                 [(c["domain"],c["origin"],c["id"]) for c in a["contracts"]])
                                self.assertEqual(r["evidence_stage"], "source_captured" if label == "source" else "no_model")
                                if kind == "exact_runtime":
                                    self.assertEqual(block["identity"], r["identity"])
                                    self.assertEqual(block["domains"], r["domains"])
                                verified = runner.verified_domains(r, a)
                                self.assertEqual(len(verified), 1 if label in ("positive", "mixed") else 0)
                if label == "no_runtime":
                    self.runtime.write_text('["synthetic-input"]')
        # Malformed producers stay rejected separately; sparse legality never excuses a mismatched join.
        a = adapter()
        for change in ({"origin":"bridge_envelope"}, {"domain":"undeclared"}, {"contract_id":"1111111111111111"}):
            r = result()
            r["domains"][0].update(change)
            with self.subTest(malformed=change), self.assertRaises(ValueError):
                runner.validate_result(json.dumps(r).encode(), a, "try1", "no_model")
        r = result()
        r["domains"][0]["outcome"] = "inconclusive"
        with self.assertRaisesRegex(ValueError, "inconsistent complete"):
            runner.validate_result(json.dumps(r).encode(), a, "try1", "no_model")
        for change in ({"id":"bad"}, {"origin":"undeclared"}, {"required_milestones":["undeclared"]}):
            bad = adapter()
            bad["contracts"][0].update(change)
            with self.subTest(malformed_descriptor=change), self.assertRaises(ValueError):
                runner.select_adapters({"schema_version":1,"adapters":[bad]}, "all")

    def descriptors(self):
        values = [adapter(name) for name in ("claude", "codex", "third", "fourth")]
        for a, key in zip(values, ("CANARY_FIRST_KEY", "CANARY_SECOND_KEY", "CANARY_THIRD_KEY", "CANARY_FOURTH_KEY")):
            a["canary_strategy"]["model_key_env"] = key
        return values

    def registry(self, values, *, raw=None, code=0):
        raw = json.dumps({"schema_version": 1, "adapters": values}) if raw is None else raw
        self.discovery.write_text(f"#!{sys.executable}\nimport json,sys,pathlib\n"
            f"with pathlib.Path({str(self.discovery_log)!r}).open('a') as f: f.write(json.dumps(sys.argv[1:])+'\\n')\n"
            "assert sys.argv[1:] == ['adapters', '--json']\n"
            f"print({raw!r})\nsys.exit({code})\n")
        self.discovery.chmod(0o755)

    def call(self, route, selector="all", tier="required", *, keys=None, out=True, extra=()):
        dest = pathlib.Path(out) if isinstance(out, pathlib.Path) else self.base / "out"
        if route == "shell":
            argv = ["bash", str(self.root / "scripts/harness-canary.sh"), "--herdr-threads", str(self.discovery)]
        else:
            argv = [sys.executable, str(self.canary / "run.py"), "strategy", "--binary", str(self.discovery)]
        argv += ["--harness", selector, "--model-tier", tier, "--runtime-command-file", str(self.runtime), *extra]
        if out:
            argv += ["--out", str(dest)]
        env = dict(self.env, **(keys or {}))
        rc, stdout, stderr = runner.bounded_capture(argv, timeout=15, env=env)
        return subprocess.CompletedProcess(argv, rc, stdout.decode(), stderr.decode())

    def refused(self, cp, names=(), diagnostic=None):
        self.assertEqual(cp.returncode, 2, cp.stderr)
        for name in names:
            self.assertIn(name, cp.stderr)
        if diagnostic:
            self.assertIn(diagnostic, cp.stderr)
        self.assertNotIn("inert-present-secret", cp.stderr + cp.stdout)
        self.assertFalse((self.base / "out").exists(), "refused before output/index/work")
        self.assertFalse(self.calls.exists(), "no partial attempt, companion, npm or model")

    def build_fixture(self, behavior="success"):
        self.registry(self.descriptors())
        self.discovery.write_text(self.discovery.read_text().replace(
            "assert sys.argv", f"pathlib.Path({str(self.base / 'executed-binary')!r}).write_text(str(pathlib.Path(__file__).resolve()))\nassert sys.argv"))
        with self.companion.open("a") as f:
            f.write(f"pathlib.Path({str(self.base / 'dispatch-binary')!r}).write_text(args['--binary'])\n")
        log = self.base / "build.json"
        cargo = self.bin / "cargo"
        cargo.write_text(f"#!{sys.executable}\nimport json,os,pathlib,shutil,signal,sys,time\n"
            f"log=pathlib.Path({str(log)!r})\n"
            "target=pathlib.Path(os.environ.get('CARGO_TARGET_DIR') or 'target').resolve()\n"
            "log.write_text(json.dumps({'argv':sys.argv[1:],'cwd':os.getcwd(),'target':str(target),'pid':os.getpid()}))\n"
            f"behavior={behavior!r}\n"
            "if behavior == 'hang':\n"
            " child=os.fork()\n"
            " if child == 0:\n"
            "  def stop(signum, frame):\n"
            "   log.with_suffix('.child-stopped').write_text(str(os.getpid()));sys.exit(0)\n"
            "  signal.signal(signal.SIGTERM,stop)\n"
            "  log.with_suffix('.child-pid').write_text(str(os.getpid()))\n"
            "  while True: time.sleep(.02)\n"
            " def stop(signum,frame):\n"
            "  os.kill(child,signal.SIGTERM);os.waitpid(child,0)\n"
            "  log.with_suffix('.reaped').write_text(str(child));sys.exit(0)\n"
            " signal.signal(signal.SIGTERM,stop)\n"
            " while not log.with_suffix('.child-pid').exists(): time.sleep(.005)\n"
            " print('offline cargo lock wait',file=sys.stderr,flush=True)\n"
            " while True: time.sleep(.02)\n"
            "if behavior == 'failure':\n"
            " print('offline build failure',file=sys.stderr);sys.exit(17)\n"
            "binary=target/'debug/herdr-threads'\n"
            "binary.parent.mkdir(parents=True,exist_ok=True)\n"
            f"shutil.copy2({str(self.discovery)!r},binary)\n")
        cargo.chmod(0o755)
        return log

    def build_call(self, env, timeout=10):
        argv = ["bash", str(self.root / "scripts/harness-canary.sh"), "--harness", "fourth",
                "--model-tier", "required", "--runtime-command-file", str(self.runtime),
                "--out", str(self.base / "out")]
        return runner.bounded_capture(argv, timeout=timeout, env=env, cwd=self.base)

    def test_default_build_timeout_cancels_and_reaps_before_output(self):
        import time
        log = self.build_fixture("hang")
        copied = self.canary / "run.py"
        source = copied.read_text()
        # Only the private copy is calibrated; the real product has an immutable 60s bound.
        calibrated = source.replace("BUILD_TIMEOUT = 60", "BUILD_TIMEOUT = 0.3")
        copied.write_text(calibrated)
        import difflib
        (self.base / "deadline-calibration.diff").write_text("".join(difflib.unified_diff(
            source.splitlines(keepends=True), calibrated.splitlines(keepends=True),
            fromfile="production-run.py", tofile="private-calibrated-run.py")))
        (self.base / "deadline-calibration.json").write_text(json.dumps({
            "before": runner.hashlib.sha256(source.encode()).hexdigest(),
            "after": runner.hashlib.sha256(calibrated.encode()).hexdigest(),
            "replacement": "BUILD_TIMEOUT = 60 -> BUILD_TIMEOUT = 0.3",
            "replacements": source.count("BUILD_TIMEOUT = 60")}))
        observed = None
        try:
            started = time.monotonic()
            try:
                observed = self.build_call(self.env, timeout=3)
            except ValueError as exc:
                observed = str(exc)  # BASE outer safety timeout is failure, never product refusal.
            elapsed = time.monotonic() - started
        finally:
            # Record BASE's safety cancellation; GREEN must prove product termination and child reaping.
            if log.exists():
                pids = [json.loads(log.read_text())["pid"]]
                child_file = log.with_suffix('.child-pid')
                if child_file.exists():
                    pids.append(int(child_file.read_text()))
                (self.base / "timeout-observation.json").write_text(json.dumps({
                    "product_result": [observed[0], observed[1].decode(), observed[2].decode()]
                                      if isinstance(observed, tuple) else observed,
                    "elapsed": elapsed, "owned_pids": pids}))
                if isinstance(observed, tuple):
                    for pid in pids:
                        with self.assertRaises(ProcessLookupError):
                            os.kill(pid, 0)
                    self.assertEqual(log.with_suffix('.reaped').read_text(), child_file.read_text())
                    self.assertEqual(log.with_suffix('.child-stopped').read_text(), child_file.read_text())
        self.assertIsInstance(observed, tuple, "product build must refuse before the outer safety timeout")
        rc, out, err = observed
        self.assertEqual(rc, 2, err)
        self.assertIn(b"cannot build discovery binary", err)
        self.assertIn(b"deadline exceeded", err)
        self.assertLess(elapsed, 2)
        self.assertEqual(out, b"")
        self.assertFalse((self.base / "out").exists())
        self.assertFalse(self.discovery_log.exists())
        self.assertFalse(self.calls.exists())

    def test_default_build_failure_is_infrastructure_before_discovery(self):
        log = self.build_fixture("failure")
        rc, out, err = self.build_call(self.env)
        self.assertEqual(rc, 2, err)
        self.assertIn(b"cannot build discovery binary", err)
        self.assertIn(b"exited 17", err)
        self.assertIn(b"offline build failure", err)
        self.assertEqual(out, b"")
        self.assertFalse((self.base / "out").exists())
        self.assertFalse(self.discovery_log.exists())
        self.assertFalse(self.calls.exists())
        self.assertEqual(json.loads(log.read_text())["cwd"], str(self.root))

    def test_default_build_binds_default_absolute_and_root_relative_targets(self):
        import shutil
        log = self.build_fixture()
        stale = self.base / "private-target/debug/herdr-threads"
        stale.parent.mkdir(parents=True)
        stale.write_text(f"#!{sys.executable}\nimport pathlib,sys\n"
                         f"pathlib.Path({str(self.base / 'stale-called')!r}).touch()\nsys.exit(93)\n")
        stale.chmod(0o755)
        config = self.root / ".cargo/config.toml"
        config.parent.mkdir()
        config.write_text('[build]\ntarget-dir = "configured-competing-target"\n')
        for selection, target in (("private-target", self.root / "private-target"),
                                  (str(self.base / "absolute-target"), self.base / "absolute-target"),
                                  (None, self.root / "target")):
            with self.subTest(selection=selection):
                (self.base / "stale-called").unlink(missing_ok=True)
                env = dict(self.env)
                if selection is not None:
                    env["CARGO_TARGET_DIR"] = selection
                rc, out, err = self.build_call(env)
                self.assertEqual(rc, 2, err)
                self.assertIn(b"CANARY_FOURTH_KEY", err)
                self.assertFalse((self.base / "stale-called").exists())
                self.assertFalse((self.base / "out").exists())
                self.assertFalse(self.calls.exists())
                built = json.loads(log.read_text())
                self.assertEqual(built['cwd'], str(self.root))
                self.assertEqual(built['target'], str(target))
                self.assertEqual(built['argv'], ['build', '--locked', '--target-dir', str(target)])
                binary = target / "debug/herdr-threads"
                self.assertEqual(binary.read_bytes(), self.discovery.read_bytes())
                self.assertEqual((self.base / 'executed-binary').read_text(), str(binary))
                self.assertEqual(json.loads(self.discovery_log.read_text().splitlines()[-1]), ['adapters', '--json'])
                env['CANARY_FOURTH_KEY'] = 'inert-present-secret'
                rc, out, err = self.build_call(env)
                self.assertEqual(rc, 0, err)
                self.assertEqual(self.calls.read_text().splitlines(), ['fourth live'])
                self.assertEqual((self.base / 'dispatch-binary').read_text(), str(binary))
                self.assertNotIn(b'inert-present-secret', out + err)
                self.assertTrue(all(b'inert-present-secret' not in path.read_bytes()
                                    for path in (self.base / 'out').rglob('*') if path.is_file()))
                request = json.loads(next((self.base / 'out').rglob('request.json')).read_text())
                self.assertEqual(request['adapter']['id'], 'fourth')
                shutil.rmtree(self.base / 'out')
                self.calls.unlink()
        self.assertFalse((self.root / 'configured-competing-target').exists())

    def test_required_preflight_checks_all_selected_keys_before_output_or_attempts(self):
        values = self.descriptors()
        for route in ("shell", "python"):
            for reverse in (False, True):
                self.registry(values[::-1] if reverse else values)
                for selector in ("all", "both", "fourth"):
                    for keys in ({}, {"CANARY_FIRST_KEY": "inert-present-secret"},
                                 {"CANARY_FIRST_KEY": "inert-present-secret", "CANARY_SECOND_KEY": "",
                                  "CANARY_FOURTH_KEY": ""}):
                        with self.subTest(route=route, reverse=reverse, selector=selector, keys=list(keys)):
                            names = [a["canary_strategy"]["model_key_env"] for a in values
                                     if (selector == "all" or selector == "both" and a["id"] in ("claude", "codex")
                                         or a["id"] == selector) and not keys.get(a["canary_strategy"]["model_key_env"])]
                            self.refused(self.call(route, selector, keys=keys), names)
                # Refusal must also leave an existing output tree byte-identical.
                dest = self.base / "out"
                dest.mkdir()
                (dest / "sentinel").write_bytes(b"unchanged\x00")
                cp = self.call(route, "all", keys={"CANARY_FIRST_KEY": "inert-present-secret"})
                self.assertEqual(cp.returncode, 2, cp.stderr)
                self.assertEqual({p.relative_to(dest).as_posix(): p.read_bytes() for p in dest.rglob('*') if p.is_file()},
                                 {"sentinel": b"unchanged\x00"})
                self.assertEqual(list(dest.iterdir()), [dest / "sentinel"])
                (dest / "sentinel").unlink()
                dest.rmdir()
                self.assertFalse(self.calls.exists())

    def test_required_preflight_realistic_discovery_reaches_shell_and_python(self):
        import shutil
        self.registry(self.descriptors())
        for route in ("shell", "python"):
            with self.subTest(route=route):
                try:
                    self.refused(self.call(route, "fourth"), ["CANARY_FOURTH_KEY"])
                    self.assertEqual(json.loads(self.discovery_log.read_text().splitlines()[-1]), ["adapters", "--json"])
                    before = set(self.base.iterdir())
                    self.refused(self.call(route, "fourth", out=False), ["CANARY_FOURTH_KEY"])
                    self.assertEqual(set(self.base.iterdir()), before, "implicit output deferred too")
                finally:
                    shutil.rmtree(self.base / "out", ignore_errors=True)
                    self.calls.unlink(missing_ok=True)
        # Exercise the corrected default-build fixture through the real leaf entrypoint too.
        import test_capture_hook as tch
        fixture = tch.ScriptTier1()
        fixture.setUp()
        try:
            tch.write_exe(fixture.bin / "npm", f"#!{sys.executable}\nimport pathlib,sys\n"
                          f"pathlib.Path({str(fixture.t / 'forbidden-npm')!r}).touch()\nsys.exit(91)\n")
            argv = ["bash", tch.SCRIPT, "--harness", "both", "--model-tier", "required",
                    "--out", str(fixture.t / "out")]
            rc, _, err = runner.bounded_capture(argv, timeout=15, env=fixture.env(ANTHROPIC_API_KEY="inert-present-secret"))
            self.assertEqual(rc, 2, err.decode())
            self.assertIn(b"OPENAI_API_KEY", err)
            self.assertNotIn(b"inert-present-secret", err)
            self.assertFalse((fixture.t / "out").exists())
            self.assertFalse((fixture.t / "forbidden-npm").exists())
            self.assertTrue((fixture.fixture_target / "debug/herdr-threads").is_file())
            self.assertEqual([json.loads(line) for line in fixture.discovery_log.read_text().splitlines()],
                             [["adapters", "--json"]])
        finally:
            fixture.tearDown()

    def test_required_preflight_invalid_or_unavailable_discovery_never_creates_output(self):
        values = self.descriptors()
        bads = [("garbage", "invalid"), ('{"schema_version":1,"schema_version":1,"adapters":[]}', "duplicate"),
                (json.dumps({"schema_version": 2, "adapters": values}), "schema")]
        for mutate in (lambda a: a["canary_strategy"].update(model_key_env="bad-key"),
                       lambda a: a["canary_strategy"].update(companion="../bad.py"),
                       lambda a: a["canary_strategy"].update(kind="invented"),
                       lambda a: a["contracts"].append(dict(a["contracts"][0]))):
            a = self.descriptors()[0]
            mutate(a)
            bads.append((json.dumps({"schema_version": 1, "adapters": [a]}), "invalid"))
        duplicate = self.descriptors()
        duplicate[1]["id"] = duplicate[0]["id"]
        bads.append((json.dumps({"schema_version": 1, "adapters": duplicate}), "duplicate"))
        for route in ("shell", "python"):
            for raw, diagnostic in bads:
                with self.subTest(route=route, raw=raw[:80]):
                    self.registry([], raw=raw)
                    self.refused(self.call(route), diagnostic=diagnostic)
            self.registry(values, code=7)
            self.refused(self.call(route), diagnostic="discovery failed")
            self.discovery.unlink()
            self.refused(self.call(route))
            self.registry(values)
            self.refused(self.call(route, "unknown"), diagnostic="unknown harness")

    def test_required_preflight_no_declared_key_and_unsupported_remain_honest(self):
        for route in ("shell", "python"):
            for mode in ("no-key", "unsupported", "empty"):
                values = self.descriptors()[:2]
                if mode == "no-key":
                    values[1]["canary_strategy"]["model_key_env"] = None
                elif mode == "unsupported":
                    values[1]["canary_strategy"] = None
                else:
                    values = []
                self.registry(values)
                for selector in (("all",) if mode == "empty" else ("all", "both", "codex")):
                    self.refused(self.call(route, selector, keys={"CANARY_FIRST_KEY": "inert-present-secret"}),
                                 diagnostic={"no-key": "no declared model key", "unsupported": "unsupported strategy",
                                             "empty": "empty selection"}[mode])
        # Unselected key declarations cannot block a permitted fourth-adapter dispatch.
        self.registry(self.descriptors())
        cp = self.call("python", "fourth", keys={"CANARY_FOURTH_KEY": "inert-present-secret"})
        self.assertEqual(cp.returncode, 0, cp.stderr)
        self.assertEqual(self.calls.read_text().splitlines(), ["fourth live"])
        self.assertNotIn("inert-present-secret", cp.stdout + cp.stderr)

    def test_required_preflight_output_fences_survive_deferred_allocation(self):
        self.registry(self.descriptors())
        keys = {"CANARY_FOURTH_KEY": "inert-present-secret"}
        for route in ("shell", "python"):
            for forbidden in (self.home / ".claude", self.home / ".codex", self.home / ".aisw",
                              self.root / "unignored"):
                with self.subTest(route=route, forbidden=forbidden.name):
                    try:
                        cp = self.call(route, "fourth", keys=keys, out=forbidden / "output")
                        self.refused(cp, diagnostic="inside")
                        self.assertFalse(forbidden.exists())
                    finally:
                        import shutil
                        shutil.rmtree(forbidden, ignore_errors=True)
                        self.calls.unlink(missing_ok=True)
            # TMPDIR must not bypass the same output fences when --out is omitted.
            forbidden = self.home / ".claude"
            forbidden.mkdir()
            previous = self.env["TMPDIR"]
            self.env["TMPDIR"] = str(forbidden)
            try:
                with self.subTest(route=route, implicit=True):
                    self.refused(self.call(route, "fourth", keys=keys, out=False), diagnostic="inside")
                    self.assertEqual(list(forbidden.iterdir()), [])
            finally:
                self.env["TMPDIR"] = previous
                for path in forbidden.glob("hc-*"):
                    import shutil
                    shutil.rmtree(path)
                forbidden.rmdir()
                self.calls.unlink(missing_ok=True)

    def test_required_preflight_preserves_auto_off_and_live_safeguards(self):
        import shutil
        self.registry(self.descriptors())
        for route in ("shell", "python"):
            for tier, keys, stage in (("auto", {}, "no_model"),
                                      ("off", {"CANARY_FOURTH_KEY": "inert-present-secret"}, "no_model"),
                                      ("required", {a["canary_strategy"]["model_key_env"]: "inert-present-secret"
                                                    for a in self.descriptors()}, "live")):
                selector = "all" if tier == "required" else "fourth"
                cp = self.call(route, selector, tier, keys=keys)
                self.assertEqual(cp.returncode, 0, cp.stderr)
                expected = [a["id"] + " live" for a in self.descriptors()] if selector == "all" else ["fourth " + stage]
                self.assertEqual(self.calls.read_text().splitlines(), expected)
                self.assertNotIn("inert-present-secret", cp.stdout + cp.stderr)
                self.assertTrue(all(b"inert-present-secret" not in path.read_bytes()
                                    for path in (self.base / "out").rglob('*') if path.is_file()))
                report = json.loads((self.base / "out/canary-report.json").read_text())
                self.assertTrue(all(b["evidence_stage"] == stage for b in report["harnesses"]))
                shutil.rmtree(self.base / "out")
                self.calls.unlink()
            cp = self.call(route, "fourth", "auto", extra=("--evidence-stage", "live"))
            self.assertEqual(cp.returncode, 1, cp.stderr)
            self.assertFalse(self.calls.exists(), "live without credentials never invokes a companion")
            report = json.loads((self.base / "out/canary-report.json").read_text())
            self.assertIn("declared credential", report["harnesses"][0]["reason"])
            shutil.rmtree(self.base / "out")
            for stage in ("no_model", "source_captured"):
                cp = self.call(route, "fourth", "off", extra=("--evidence-stage", stage))
                self.assertEqual(cp.returncode, 0 if stage == "no_model" else 1, cp.stderr)
                self.assertEqual(self.calls.read_text().splitlines(), ["fourth " + stage])
                index = json.loads((self.base / "out/artifact-index.json").read_text())
                self.assertEqual(index["attempts"][0]["evidence_stage"], stage)
                shutil.rmtree(self.base / "out")
                self.calls.unlink()

        for route in ("shell", "python"):
            for keep in (False, True):
                cp = self.call(route, "fourth", "auto", out=False, extra=("--keep",) if keep else ())
                self.assertEqual(cp.returncode, 0, cp.stderr)
                directories = list(self.base.glob("hc-*"))
                self.assertEqual(len(directories), int(keep))
                if keep:
                    self.assertTrue((directories[0] / "artifact-index.json").is_file())
                    shutil.rmtree(directories[0])
                self.calls.unlink()


if __name__ == "__main__":
    unittest.main()
