#!/usr/bin/env python3
"""Offline self-test for the harness canary (nested spec §D11): drives scripts/canary/bisect.py with
stub_probe.py over the planted cases and asserts each report. No network, npm, cargo or harness."""
import importlib.util, json, math, os, pathlib, shlex, subprocess, sys, tempfile, unittest

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


MANIFEST_CASES = sorted((HERE / "manifest-cases").glob("*.json"))
EXPECTED_MANIFEST_CASES = {"all-pass", "payload-break", "known-broken-persists", "flaky", "tier0-failure-issue-only",
                           "retention", "size-cap-failure", "schema1-baseline-upgrade"}
MANIFEST_PY = CANARY / "manifest.py"
STUB_HT = CANARY / "testdata" / "manifest" / "stub-herdr-threads"
STUB_IDS = {"claude": "c1a0c1a0c1a0c1a0", "codex": "c0dec0dec0dec0de"}


def expand_baseline(case):
    """The case's baseline document, with `baseline_rows_gen` entries expanded into schema-2 canary rows:
    {harness, count, prefix} -> `<prefix>0 .. <prefix>count-1`; {harness, version} -> one row; optional status,
    recipe, source, issue_url_pad."""
    base = json.loads(json.dumps(case["baseline"]))
    for g in case.get("baseline_rows_gen", []):
        versions_ = [g["version"]] if "version" in g else [f"{g['prefix']}{i}" for i in range(g["count"])]
        for v in versions_:
            status = g.get("status", "verified")
            row = {"harness": g["harness"], "version": v, "status": status,
                   "evidence": "live" if status == "verified" else "schema", "contract_id": STUB_IDS[g["harness"]],
                   "source": g.get("source", "canary"), "supported_since": None,
                   "broken_event": "SessionStart" if status == "known_broken" else None,
                   "broken_field": "session_id" if status == "known_broken" else None,
                   "last_working": None, "issue_url": "x" * g["issue_url_pad"] if g.get("issue_url_pad") else None,
                   "recipe": g.get("recipe"), "known_broken": []}
            base["rows"].append(row)
    return base


def run_manifest_case(path, state):
    """Run manifest.py write over one case's inputs; (case, completed process, output document or None)."""
    case = json.loads(path.read_text())
    state = pathlib.Path(state)
    (state / "baseline.json").write_text(json.dumps(expand_baseline(case)))
    (state / "report.json").write_text(json.dumps(case["report"]))
    out = state / "out.json"
    cmd = [sys.executable, str(MANIFEST_PY), "write", "--baseline", str(state / "baseline.json"),
           "--report", str(state / "report.json"), "--binary", str(STUB_HT), "--generated-at", "2026-10-02T06:00:00Z",
           "--out", str(out)]
    if "release_results" in case:
        (state / "release.json").write_text(json.dumps(case["release_results"]))
        cmd += ["--release-results", str(state / "release.json"), "--latest-release", case.get("latest_release", "v0.4.0")]
    cp = subprocess.run(cmd, capture_output=True, text=True, timeout=120)
    return case, cp, (json.loads(out.read_text()) if out.exists() else None)


class ManifestCases(unittest.TestCase):
    def test_case_files_are_the_planted_set(self):
        self.assertEqual({p.stem for p in MANIFEST_CASES}, EXPECTED_MANIFEST_CASES)

    def test_cases(self):
        for path in MANIFEST_CASES:
            with self.subTest(case=path.stem), tempfile.TemporaryDirectory() as state:
                self.check_case(path, state)

    def check_case(self, path, state):
        case, cp, out = run_manifest_case(path, state)
        exp = case["expect"]
        self.assertEqual(cp.returncode, exp["exit"], cp.stdout + cp.stderr)
        if "stdout" in exp:
            self.assertIn(exp["stdout"], cp.stdout)
        if exp.get("no_output_file"):
            self.assertIsNone(out)
            return
        self.assertEqual(out["schema_version"], exp.get("schema_version", 2))
        for want in exp.get("rows", []):
            matches = [r for r in out["rows"] if r["harness"] == want["harness"] and r["version"] == want["version"]]
            self.assertEqual(len(matches), 1, f"{want['harness']} {want['version']}: {len(matches)} rows")
            for k, v in want.items():
                self.assertEqual(matches[0][k], v, f"{want['harness']} {want['version']}.{k}")
        for h, v in exp.get("absent", []):
            self.assertFalse([r for r in out["rows"] if r["harness"] == h and r["version"] == v], f"{h} {v} must be absent")
        if "row_count" in exp:
            self.assertEqual(len(out["rows"]), exp["row_count"])
        if "claude_versions_count" in exp:
            self.assertEqual(len([r for r in out["rows"] if r["harness"] == "claude"]), exp["claude_versions_count"])
        keys = {(r["harness"], r["version"], r["contract_id"]) for r in out["rows"]}
        self.assertEqual(len(keys), len(out["rows"]), "no duplicate (harness, version, contract_id) keys")
        vp = subprocess.run([sys.executable, str(MANIFEST_PY), "validate", str(pathlib.Path(state) / "out.json")],
                            capture_output=True, text=True)
        self.assertEqual(vp.returncode, 0, vp.stdout)


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

    def test_reprobe_runs_after_the_search_and_keeps_the_verdict(self):
        src = HERE / "cases" / "all-pass.json"
        case = json.loads(src.read_text())

        def run_with(results, candidates, reprobe, name="c.json"):
            with tempfile.TemporaryDirectory() as state:
                cp = pathlib.Path(state) / name
                cp.write_text(json.dumps(dict(case, results=results)))
                os.environ["HT_SELFTEST_STATE"] = state
                cmd = " ".join(shlex.quote(a) for a in (sys.executable, str(STUB), str(cp))) + " {version}"
                return canary_bisect.run(candidates, "1.0.0", cmd, bisect=True, reprobe=reprobe)

        res = run_with({"default": "pass"}, case["candidates"], ["0.9.5"])
        self.assertEqual(res["status"], "all_pass")
        self.assertEqual({k: res["probes"][-1][k] for k in ("version", "role")}, {"version": "0.9.5", "role": "reprobe"})
        self.assertEqual(res["reprobed"], ["0.9.5"])
        res = run_with({"default": "pass"}, [], ["0.9.5"])
        self.assertEqual(res["status"], "no_candidates")
        self.assertEqual([p["version"] for p in res["probes"]], ["0.9.5"])
        # a re-probe version equal to the newest candidate is not probed twice
        res = run_with({"default": "pass"}, case["candidates"], ["1.0.9"])
        self.assertEqual([p["version"] for p in res["probes"]].count("1.0.9"), 1)
        self.assertEqual(res["reprobed"], [])
        # a failing re-probe changes neither the verdict nor the exit code
        def exit_code(res):
            block = report_mod.harness_block("claude", res, verified_max="1.0.0")
            return report_mod.assemble([block], {"harness": "claude", "versions": "since-verified", "bisect": True,
                                                 "model_tier": "auto"}, {"os": "linux", "arch": "x86_64"},
                                       "0" * 40, "0.0.0", generated_at="2026-10-01T00:00:00Z")["exit_code"]
        plain = run_with({"default": "pass"}, case["candidates"], [])
        failing = run_with({"default": "pass", "0.9.5": "fail"}, case["candidates"], ["0.9.5"])
        self.assertEqual(failing["probes"][-1]["result"], "fail")
        for k in ("status", "first_bad", "last_good", "failing_checks", "signals"):
            self.assertEqual(failing[k], plain[k], k)
        self.assertEqual(exit_code(failing), exit_code(plain))


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

    def test_selection_is_scoped_to_main_contract(self):
        c1, c2 = "1111111111111111", "2222222222222222"

        def row(version, status="verified", cid=None, **kw):
            return dict({"harness": "claude", "version": version, "status": status, "contract_id": cid,
                         "known_broken": []}, **kw)
        doc = {"schema_version": 2, "rows": [
            row("2.1.285"), row("2.1.300", cid=c1), row("2.1.290", "known_broken", c1), row("2.1.288", cid=c2),
            row("2.1.280", known_broken=[{"min": "2.1.281", "max": "2.1.282"}])]}
        npm = [f"2.1.{i}" for i in range(280, 303)] + ["2.1.303-beta.1"]
        cands = lambda cid: versions.candidates(npm, "since-verified", doc, "claude", contract_id=cid)

        self.assertEqual(versions.verified_max(doc, "claude", c2), "2.1.288")
        self.assertEqual(versions.known_broken(doc, "claude", c2), [("2.1.281", "2.1.282")])
        self.assertEqual(cands(c2)[0], "2.1.289")
        self.assertIn("2.1.290", cands(c2))
        self.assertEqual(versions.reprobe(npm, doc, "claude", c2), ["2.1.290"])

        self.assertEqual(versions.verified_max(doc, "claude", c1), "2.1.300")
        self.assertIn(("2.1.290", "2.1.290"), versions.known_broken(doc, "claude", c1))
        self.assertEqual(cands(c1), ["2.1.301", "2.1.302"])
        self.assertEqual(versions.reprobe(npm, doc, "claude", c1), [])

        self.assertEqual(versions.verified_max(doc, "claude", None), "2.1.300")
        self.assertCountEqual(versions.known_broken(doc, "claude", None), [("2.1.281", "2.1.282"), ("2.1.290", "2.1.290")])
        self.assertEqual(versions.reprobe(npm, doc, "claude", None), [])
        self.assertEqual(cands(None), cands(c1))

        both = {"schema_version": 2, "rows": doc["rows"] + [row("2.1.290", "known_broken", c2)]}
        self.assertEqual(versions.reprobe(npm, both, "claude", c2), [])  # broken under main too: stays excluded
        self.assertEqual(versions.reprobe([v for v in npm if v != "2.1.290"], doc, "claude", c2), [])  # unpublished

    def test_schema_1_and_2_documents_give_the_same_answers(self):
        committed_path = HERE.parents[1] / "docs" / "compatibility" / "harness-versions.json"
        committed = versions.load_versions_json(committed_path)
        self.assertEqual(committed["schema_version"], 2)
        # The same rows written as a schema-1 document (no schema-2 fields) upgrade to the same answers.
        v1 = {"schema_version": 1,
              "rows": [{k: r[k] for k in ("harness", "version", "recipe", "evidence", "known_broken")}
                       for r in committed["rows"]]}
        with tempfile.TemporaryDirectory() as tmp:
            v1_path = pathlib.Path(tmp) / "v1.json"
            v1_path.write_text(json.dumps(v1))
            upgraded = versions.load_versions_json(v1_path)
        self.assertEqual(upgraded["schema_version"], 2)
        for harness in ("codex", "claude"):
            self.assertIsNotNone(versions.verified_max(committed, harness))
            self.assertEqual(versions.verified_max(upgraded, harness), versions.verified_max(committed, harness))
            self.assertEqual(versions.known_broken(upgraded, harness), versions.known_broken(committed, harness))
        self.assertEqual(versions.verified_max(committed, "codex"), "0.159.3")
        self.assertTrue(all(r["status"] == "verified" and r["contract_id"] is None for r in upgraded["rows"]))
        # The checked-in schema-1 fixture still loads.
        fixture = versions.load_versions_json(FIX / "harness-versions.json")
        self.assertEqual(versions.verified_max(fixture, "codex"), "0.157.1")
        self.assertEqual(versions.known_broken(fixture, "codex"), [])
        with tempfile.TemporaryDirectory() as tmp:
            bad = pathlib.Path(tmp) / "v3.json"
            bad.write_text(json.dumps({"schema_version": 3, "rows": []}))
            with self.assertRaises(ValueError):
                versions.load_versions_json(bad)

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
