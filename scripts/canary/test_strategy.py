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


if __name__ == "__main__":
    unittest.main()
