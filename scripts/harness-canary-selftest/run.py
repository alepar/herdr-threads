#!/usr/bin/env python3
"""Offline self-test for the harness canary (nested spec §D11): drives scripts/canary/bisect.py with
stub_probe.py over the planted cases and asserts each report. No network, npm, cargo or harness."""
import importlib.util, json, math, os, pathlib, shlex, sys, tempfile, unittest

HERE = pathlib.Path(__file__).resolve().parent
CANARY = HERE.parent / "canary"


def _load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


# versions.py is shared by bisect.py through the canary_versions name; load it first.
versions = _load("canary_versions", CANARY / "versions.py")
canary_bisect = _load("canary_bisect", CANARY / "bisect.py")
report_mod = _load("canary_report", CANARY / "report.py")
contract = _load("probe_contract", CANARY / "test_probe_contract.py")  # reuse its schema validator

FIX = HERE / "fixtures"
STUB = HERE / "stub_probe.py"
CASES = sorted((HERE / "cases").glob("*.json"))
EXPECTED_CASES = {"all-pass", "break-mid", "break-first", "flip", "baseline-broken", "flaky", "infra",
                  "tier1-retry", "known-broken-then-new-break", "break-persists-above-range",
                  "open-ended-known-broken"}


def run_case(path):
    case = json.loads(path.read_text())
    ranges = []
    baseline = case["baseline"]
    if "harness" in case:
        doc = versions.load_versions_json(HERE / case["versions_json"])
        ranges = versions.known_broken(doc, case["harness"])
        baseline = versions.verified_max(doc, case["harness"])
    with tempfile.TemporaryDirectory() as state:
        os.environ["HT_SELFTEST_STATE"] = state
        cmd = " ".join(shlex.quote(a) for a in (sys.executable, str(STUB), str(path))) + " {version}"
        res = canary_bisect.run(case["candidates"], baseline, cmd, bisect=case["bisect"],
                                tier1=case["tier1"], known_broken=ranges)
        calls = [json.loads(l) for l in (pathlib.Path(state) / "calls.jsonl").read_text().splitlines()]
    return case, baseline, res, calls


class Cases(unittest.TestCase):
    def test_case_files_are_the_planted_set(self):
        self.assertEqual({p.stem for p in CASES}, EXPECTED_CASES)

    def test_cases(self):
        for path in CASES:
            with self.subTest(case=path.stem):
                self.check_case(path)

    def check_case(self, path):
        case, baseline, res, calls = run_case(path)
        exp = case["expect"]
        self.assertEqual(res["status"], exp["status"])
        self.assertEqual(res["first_bad"], exp["first_bad"])
        self.assertEqual(res["last_good"], exp["last_good"])

        block = report_mod.harness_block(case.get("harness", "claude"), res, verified_max=baseline)
        rep = report_mod.assemble([block], {"harness": "claude", "versions": "since-verified", "bisect": True,
                                            "model_tier": "auto"},
                                  {"os": "linux", "arch": "x86_64"}, "0" * 40, "0.0.0",
                                  generated_at="2026-10-01T00:00:00Z")
        self.assertEqual(rep["exit_code"], exp["exit_code"])
        self.assertEqual(report_mod.exit_code(rep), exp["exit_code"])

        # probe-count bound: 1 + ceil(log2 n) + 2, plus 1 per crossed known_broken range
        n = len(case["candidates"])
        bound = 1 + math.ceil(math.log2(n)) + 2 + exp.get("crossed", 0)
        self.assertLessEqual(len(res["probes"]), bound)
        if "probes" in exp:
            self.assertEqual(len(res["probes"]), exp["probes"])
        if "listed_versions" in exp:  # inconclusive: every probe listed, in order
            self.assertEqual([p["version"] for p in res["probes"]], exp["listed_versions"])
        if "excluded" in exp:
            self.assertEqual(res["excluded"], exp["excluded"])
            self.assertEqual(res["crossed_ranges"], exp["crossed"])
        for v, k in exp.get("attempts", {}).items():
            self.assertEqual([len(p["attempts"]) for p in res["probes"] if p["version"] == v][0], k)
        self.assertEqual(sorted(p["version"] for p in res["probes"] if p["flaky"]), exp.get("flaky", []))
        if exp["status"] == "infra_error":
            self.assertIsNone(rep["harnesses"][0]["first_bad"])

        # every stub invocation was one recorded attempt, and every stub output is a valid probeResult
        self.assertEqual(len(calls), sum(len(p["attempts"]) for p in res["probes"]))
        for c in calls:
            contract.check(json.loads(c["stdout"]), "probeResult")

        summary = report_mod.render_summary(rep)
        self.assertIn(f"verdict: {exp['status']}", summary)
        self.assertIn(f"exit code: {exp['exit_code']}", summary)
        if exp["status"] == "break":
            self.assertIn(f"first bad: {exp['first_bad']}", summary)
            sa = rep["harnesses"][0]["suggested_action"]
            self.assertEqual(sa["range"], f">= {exp['first_bad']}")
            a, b, c_ = exp["first_bad"].split(".")
            self.assertIn(f"min: Some(Version::new({a}, {b}, {c_}))", sa["known_broken_snippet"])
            self.assertIn("```", summary)
        else:
            self.assertIsNone(rep["harnesses"][0]["suggested_action"])

    def test_break_mid_is_exactly_c5(self):
        _, _, res, _ = run_case(HERE / "cases" / "break-mid.json")
        self.assertEqual(res["first_bad"], "1.0.6")  # C[5] of 1.0.1..1.0.9
        self.assertEqual([p["role"] for p in res["probes"]][:2], ["newest", "search"])

    def test_infra_has_no_first_bad_and_stops(self):
        _, _, res, _ = run_case(HERE / "cases" / "infra.json")
        self.assertEqual(res["probes"][-1]["result"], "infra")
        self.assertEqual(res["probes"][-1]["version"], "1.0.4")

    def test_without_bisect_every_candidate_is_probed_descending(self):
        case = json.loads((HERE / "cases" / "break-mid.json").read_text())
        with tempfile.TemporaryDirectory() as state:
            os.environ["HT_SELFTEST_STATE"] = state
            cmd = " ".join(shlex.quote(a) for a in (sys.executable, str(STUB), str(HERE / "cases" / "break-mid.json"))) + " {version}"
            res = canary_bisect.run(case["candidates"], "1.0.0", cmd, bisect=False)
        self.assertEqual([p["version"] for p in res["probes"] if p["attempts"]], list(reversed(case["candidates"])))
        self.assertIsNone(res["first_bad"])
        self.assertEqual(res["status"], "break")


class Helpers(unittest.TestCase):
    def test_codex_candidate_filtering(self):
        npm = json.loads((FIX / "npm-codex-versions.json").read_text())
        doc = versions.load_versions_json(FIX / "harness-versions.json")
        self.assertEqual(versions.verified_max(doc, "codex"), "0.157.1")
        self.assertEqual(versions.candidates(npm, "since-verified", doc, "codex"),
                         ["0.158.0", "0.159.0", "0.159.3", "0.159.9", "0.159.10"])
        self.assertEqual(versions.candidates(npm, "latest", doc, "codex"), ["0.159.10"])
        st = versions.stable(npm)
        self.assertNotIn("0.1.2504301751", st)  # patch wider than the product's 6-digit rule
        self.assertTrue(all(versions.STABLE.match(v) for v in st))
        self.assertEqual(versions.candidates(npm, "list", doc, "codex", explicit=["0.159.3", "0.158.0"]),
                         ["0.158.0", "0.159.3"])
        for bad in (["0.158.0-alpha.1"], ["9.9.9"]):
            with self.assertRaises(ValueError):
                versions.candidates(npm, "list", doc, "codex", explicit=bad)

    def test_claude_candidates_sort_numerically(self):
        npm = json.loads((FIX / "npm-claude-versions.json").read_text())
        doc = versions.load_versions_json(FIX / "harness-versions.json")
        self.assertEqual(versions.candidates(npm, "since-verified", doc, "claude"), ["2.1.287", "2.2.0", "2.10.0"])

    def test_known_broken_helpers(self):
        doc = versions.load_versions_json(FIX / "harness-versions.json")
        self.assertEqual(versions.known_broken(doc, "stubkb"), [("1.0.2", "1.0.4")])
        self.assertEqual(versions.known_broken(doc, "stubopen"), [("1.0.7", None)])
        self.assertEqual(versions.known_broken(doc, "codex"), [])
        c = [f"1.0.{i}" for i in range(1, 10)]
        self.assertEqual(versions.excluded_by_known_broken(c, [("1.0.2", "1.0.4")])[1], ["1.0.2", "1.0.3", "1.0.4"])
        kept, ex = versions.excluded_by_known_broken(c, [("1.0.7", None)])
        self.assertEqual((kept[-1], ex), ("1.0.9", ["1.0.7", "1.0.8"]))  # open range never excludes the newest
        kept, ex = versions.excluded_by_known_broken(c, [("1.0.8", "1.0.9")])
        self.assertEqual(kept[-1], "1.0.7")  # a closed range does

    def test_stub_outputs_validate_for_every_planted_outcome(self):
        import subprocess
        case = HERE / "cases" / "all-pass.json"
        for outcome, code in (("pass", 0), ("fail", 1), ("fail:t1", 1), ("infra", 2)):
            with self.subTest(outcome=outcome), tempfile.TemporaryDirectory() as d:
                tmp = pathlib.Path(d) / "case.json"
                tmp.write_text(json.dumps({"results": {"default": outcome}}))
                cp = subprocess.run([sys.executable, str(STUB), str(tmp), "1.0.1"], capture_output=True, text=True,
                                    env={**os.environ, "HT_SELFTEST_STATE": d})
                self.assertEqual(cp.returncode, code)
                doc = json.loads(cp.stdout)
                contract.check(doc, "probeResult")
                self.assertEqual(canary_bisect.parse_probe_output(cp.stdout), doc)

    def test_bisect_cli_exit_codes(self):
        import subprocess
        case = HERE / "cases" / "break-mid.json"
        c = json.loads(case.read_text())
        with tempfile.TemporaryDirectory() as d:
            cmd = " ".join(shlex.quote(a) for a in (sys.executable, str(STUB), str(case))) + " {version}"
            cp = subprocess.run([sys.executable, str(CANARY / "bisect.py"), "--probe-cmd", cmd, "--bisect",
                                 "--candidates", ",".join(c["candidates"]), "--baseline", "1.0.0"],
                                capture_output=True, text=True, env={**os.environ, "HT_SELFTEST_STATE": d})
        self.assertEqual(cp.returncode, 1, cp.stderr)
        self.assertEqual(json.loads(cp.stdout)["first_bad"], "1.0.6")

    def test_report_writer(self):
        _, baseline, res, _ = run_case(HERE / "cases" / "break-mid.json")
        block = report_mod.harness_block("codex", res, verified_max=baseline)
        rep = report_mod.assemble([block], {"harness": "codex"}, {"os": "linux", "arch": "x86_64"}, "abc", "1.2.3")
        with tempfile.TemporaryDirectory() as d:
            report_mod.write(rep, d)
            back = json.loads((pathlib.Path(d) / "canary-report.json").read_text())
            self.assertEqual(back["schema_version"], 1)
            self.assertEqual(back["harnesses"][0]["package"], "@openai/codex")
            self.assertEqual(back["exit_code"], 1)
            self.assertIn("verdict: break", (pathlib.Path(d) / "summary.md").read_text())


if __name__ == "__main__":
    unittest.main(verbosity=1)
