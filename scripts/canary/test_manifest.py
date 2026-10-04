#!/usr/bin/env python3
"""Offline tests for scripts/canary/manifest.py (the canary's harness-manifest writer). Stdlib only; the
herdr-threads binary is scripts/canary/testdata/manifest/stub-herdr-threads, which logs every call."""
import sys
# scripts/canary/bisect.py shadows the stdlib `bisect` (needed by random/tempfile) when this directory is first on
# sys.path (`unittest discover -s scripts/canary`); import tempfile with it off the path.
_saved = list(sys.path)
sys.path[:] = [p for p in sys.path if p not in ("", __import__("os").path.dirname(__import__("os").path.abspath(__file__)))]
sys.modules.pop("bisect", None)
import copy, importlib.util, json, os, pathlib, subprocess, tempfile, unittest
sys.path[:] = _saved

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent.parent
DATA = HERE / "testdata" / "manifest"
STUB = str(DATA / "stub-herdr-threads")
SCRIPT = HERE / "manifest.py"
spec = importlib.util.spec_from_file_location("manifest_under_test", SCRIPT)
manifest = importlib.util.module_from_spec(spec)
spec.loader.exec_module(manifest)

MAIN = {"claude": "c1a0c1a0c1a0c1a0", "codex": "c0dec0dec0dec0de"}  # the stub's ids
OTHER = "dddddddddddddddd"  # a release contract that differs from main's
THIRD = "eeeeeeeeeeeeeeee"  # a contract that is neither live
NOW = "2026-10-02T06:00:00Z"

SESSION_OK = {"event": "SessionStart", "kind": "ok", "field": None}
TOOL_OK = {"event": "PreToolUse", "kind": "ok", "field": None}
VIOLATION = {"event": "SessionStart", "kind": "violation", "field": "session_id"}
MALFORMED = {"event": None, "kind": "malformed", "field": None}


def row(harness, version, status="verified", cid=MAIN["claude"], **kw):
    r = {"harness": harness, "version": version, "status": status,
         "evidence": "live" if status == "verified" else "schema", "contract_id": cid, "source": "canary",
         "supported_since": None, "broken_event": None, "broken_field": None, "last_working": None,
         "issue_url": None, "recipe": None, "known_broken": []}
    if status == "known_broken":
        r.update(broken_event="SessionStart", broken_field="session_id")
    r.update(kw)
    return r


def doc(rows, **kw):
    d = {"schema_version": 2, "generated_from": "test", "generated_at": NOW, "latest_release": "0.4.0",
         "contracts": dict(MAIN), "rows": rows}
    d.update(kw)
    return d


def probe(version, payloads=(SESSION_OK, TOOL_OK), result="pass", flaky=False, checks=None, role="newest"):
    return {"version": version, "role": role, "result": result, "flaky": flaky,
            "attempts": [{"result": result, "tier1": False, "duration_ms": 1,
                          "checks": checks or [{"id": "t0.payload-parse", "status": "pass"}],
                          "contract": {"contract_id": MAIN["claude"], "payloads": list(payloads), "release": None}}]}


def report(*blocks, harness="claude"):
    out = []
    for b in blocks:
        if isinstance(b, list):
            b = {"probes": b}
        b = {"harness": harness, "status": "all_pass", "first_bad": None, **b}
        out.append(b)
    return {"schema_version": 1, "canary_commit": "1" * 40, "harnesses": out}


def release(probes, cid=None, tag="v0.4.0", supported=True, error=None):
    if error is not None:
        return {"tag": tag, "error": error}
    if not supported:
        return {"tag": tag, "supported": False}
    return {"tag": tag, "supported": True, "contract_id": cid or dict(MAIN),
            "probes": [{"harness": h, "version": v, "payloads": list(p)} for h, v, p in probes]}


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(dir=os.environ.get("TMPDIR"))
        self.addCleanup(self.tmp.cleanup)
        self.dir = pathlib.Path(self.tmp.name)
        self.log = self.dir / "stub.log"
        self.env = dict(os.environ, STUB_LOG=str(self.log))
        os.environ["STUB_LOG"] = str(self.log)
        self.addCleanup(os.environ.pop, "STUB_LOG", None)

    def put(self, name, data):
        p = self.dir / name
        p.write_text(data if isinstance(data, str) else json.dumps(data, indent=2) + "\n")
        return p

    def calls(self):
        return [json.loads(l) for l in self.log.read_text().splitlines()] if self.log.exists() else []

    def cli(self, *args, check=None):
        cp = subprocess.run([sys.executable, str(SCRIPT), *map(str, args)], capture_output=True, text=True,
                            env=self.env, cwd=ROOT, timeout=120)
        if check is not None:
            self.assertEqual(cp.returncode, check, cp.stdout + cp.stderr)
        return cp

    def write(self, baseline, rep, rel=None, urls=None, tag="v0.4.0"):
        args = ["write", "--baseline", self.put("baseline.json", baseline), "--report", self.put("report.json", rep),
                "--binary", STUB, "--generated-at", NOW, "--out", self.dir / "out.json"]
        if rel is not None:
            args += ["--release-results", self.put("release.json", rel), "--latest-release", tag]
        if urls is not None:
            args += ["--issue-urls", self.put("urls.json", urls)]
        cp = self.cli(*args)
        out = json.loads((self.dir / "out.json").read_text()) if cp.returncode == 0 else None
        return cp, out

    @staticmethod
    def by_key(out):
        return {(r["harness"], r["version"], r["contract_id"]): r for r in out["rows"]}


class Writer(Base):
    def test_writer_uses_binary_contract_id(self):
        cp, out = self.write(doc([]), report([probe("2.1.288")]))
        self.assertEqual(cp.returncode, 0, cp.stderr)
        self.assertEqual(out["contracts"], MAIN)
        r = self.by_key(out)[("claude", "2.1.288", MAIN["claude"])]
        self.assertEqual((r["status"], r["evidence"], r["source"]), ("verified", "live", "canary"))
        self.assertIn(["contract-id", "--json"], self.calls())
        self.assertEqual(out["generated_from"], "harness-canary " + "1" * 40)
        self.assertEqual(list(out), ["schema_version", "generated_from", "generated_at", "latest_release", "contracts", "rows"])
        self.assertEqual(list(out["rows"][0]), list(manifest.ROW_KEYS))

    def test_writer_normalizes_versions_through_binary(self):
        cp, out = self.write(doc([]), report([probe("2.1.288"), probe("2.1.289 (Claude Code)"), probe("nightly")]))
        self.assertEqual(cp.returncode, 0, cp.stderr)
        normalize = [c for c in self.calls() if c[:2] == ["harness-version", "normalize"]]
        self.assertEqual(sorted(c[3] for c in normalize), ["2.1.288", "2.1.289 (Claude Code)", "nightly"])
        self.assertEqual({r["version"] for r in out["rows"]}, {"2.1.288", "2.1.289"})  # the key is the binary's answer
        self.assertIn("cannot normalize claude version 'nightly'", cp.stderr)

    def test_evidence_is_no_model_without_a_tool_payload(self):
        _, out = self.write(doc([]), report([probe("2.1.288", [SESSION_OK])]))
        self.assertEqual(self.by_key(out)[("claude", "2.1.288", MAIN["claude"])]["evidence"], "no_model")

    def test_payload_violation_writes_known_broken_with_last_working_and_issue(self):
        base = doc([row("claude", "2.1.286")])
        rep = report({"status": "break", "first_bad": "2.1.289",
                      "probes": [probe("2.1.287"), probe("2.1.289", [VIOLATION], result="fail")]})
        _, out = self.write(base, rep, urls={"claude 2.1.289": "https://github.com/o/r/issues/9"})
        r = self.by_key(out)[("claude", "2.1.289", MAIN["claude"])]
        self.assertEqual((r["status"], r["broken_event"], r["broken_field"]), ("known_broken", "SessionStart", "session_id"))
        self.assertEqual(r["last_working"], "2.1.287")  # newest verified below, from this run's output
        self.assertEqual(r["issue_url"], "https://github.com/o/r/issues/9")

    def test_last_working_comes_from_a_same_contract_baseline_row_below(self):
        base = doc([row("claude", "2.1.286")])
        _, out = self.write(base, report({"status": "break", "first_bad": "2.1.287",
                                          "probes": [probe("2.1.287", [VIOLATION], result="fail")]}))
        self.assertEqual(self.by_key(out)[("claude", "2.1.287", MAIN["claude"])]["last_working"], "2.1.286")

    def last_working(self, baseline_rows, broken):
        rep = report({"status": "break", "first_bad": broken, "probes": [probe(broken, [VIOLATION], result="fail")]})
        cp, out = self.write(doc(baseline_rows), rep)
        self.assertEqual(cp.returncode, 0, cp.stderr)
        self.assertEqual(manifest.validate_doc(out), [])
        return self.by_key(out)[("claude", broken, MAIN["claude"])]["last_working"]

    def test_last_working_never_names_the_broken_version(self):
        self.assertIsNone(self.last_working([row("claude", "2.1.287", cid=OTHER)], "2.1.287"))

    def test_last_working_below_the_baselines_greatest_verified_version(self):
        base = [row("claude", "2.1.285"), row("claude", "2.1.290", cid=OTHER)]
        self.assertEqual(self.last_working(base, "2.1.288"), "2.1.285")

    def test_last_working_never_names_a_newer_version(self):
        base = [row("claude", "2.1.290", cid=OTHER), row("claude", "2.1.289")]
        self.assertIsNone(self.last_working(base, "2.1.287"))

    def test_last_working_equal_version_under_the_same_contract_is_skipped(self):
        base = [row("claude", "2.1.286"), row("claude", "2.1.287")]
        self.assertEqual(self.last_working(base, "2.1.287"), "2.1.286")

    def test_last_working_counts_a_null_contract_row(self):
        base = [row("claude", "2.1.284", cid=None, source="manual")]
        self.assertEqual(self.last_working(base, "2.1.287"), "2.1.284")

    def test_issue_only_outcomes_write_no_row(self):
        base = doc([row("claude", "2.1.286")])
        failing = [{"id": "t0.setup", "status": "fail", "detail": "x"}]
        cases = {
            "flaky": [probe("2.1.287", flaky=True)],
            "infra": [probe("2.1.287", [], result="infra")],
            "tier0": [probe("2.1.287", result="fail", checks=failing)],
            "malformed-only": [probe("2.1.287", [MALFORMED])],
            "no-payloads": [probe("2.1.287", [])],
        }
        for name, probes in cases.items():
            with self.subTest(name):
                cp, out = self.write(base, report(probes))
                self.assertEqual(cp.returncode, 0, cp.stderr)
                self.assertEqual([r["version"] for r in out["rows"]], ["2.1.286"])
        for status in ("inconclusive", "infra_error"):
            with self.subTest(status):
                _, out = self.write(base, report({"status": status, "probes": [probe("2.1.287")]}))
                self.assertEqual([r["version"] for r in out["rows"]], ["2.1.286"])

    def test_known_broken_row_persists_until_the_version_verifies(self):
        broken = row("claude", "2.1.287", "known_broken", last_working="2.1.286")
        _, out = self.write(doc([broken]), report([probe("2.1.288")]))
        self.assertEqual(self.by_key(out)[("claude", "2.1.287", MAIN["claude"])]["status"], "known_broken")
        _, out = self.write(doc([broken]), report([probe("2.1.287")]))
        r = self.by_key(out)[("claude", "2.1.287", MAIN["claude"])]
        self.assertEqual((r["status"], r["broken_event"], r["last_working"]), ("verified", None, None))

    def test_manual_row_survives_a_canary_run(self):
        manual = row("claude", "2.1.287", "known_broken", source="manual", last_working="2.1.280")
        cp, out = self.write(doc([manual]), report([probe("2.1.287")]))  # the canary says verified
        self.assertEqual(self.by_key(out)[("claude", "2.1.287", MAIN["claude"])], manual)
        self.assertIn("kept manual row", cp.stderr)
        # a manual row under a contract that is not live is also kept, canary rows of it are not
        _, out = self.write(doc([row("claude", "2.1.200", cid=THIRD, source="manual"), row("claude", "2.1.201", cid=THIRD)]),
                            report([]))
        self.assertEqual([r["version"] for r in out["rows"]], ["2.1.200", "2.1.201"])  # missing evidence preserves history

    def test_rows_for_both_live_contracts(self):
        rel = release([("claude", "2.1.288", [SESSION_OK, TOOL_OK])], cid={"claude": OTHER, "codex": MAIN["codex"]})
        base = doc([row("claude", "2.1.100", cid=THIRD)])
        _, out = self.write(base, report([probe("2.1.288")]), rel)
        keys = set(self.by_key(out))
        self.assertEqual(keys, {("claude", "2.1.288", MAIN["claude"]), ("claude", "2.1.288", OTHER)})
        main = self.by_key(out)[("claude", "2.1.288", MAIN["claude"])]
        old = self.by_key(out)[("claude", "2.1.288", OTHER)]
        self.assertEqual((main["supported_since"], old["supported_since"]), ("0.4.0", "0.4.0"))

    def test_release_with_same_contract_folds_into_one_row(self):
        rel = release([("claude", "2.1.288", [SESSION_OK, TOOL_OK])])
        _, out = self.write(doc([]), report([probe("2.1.288")]), rel)
        self.assertEqual(len(out["rows"]), 1)
        self.assertEqual(out["rows"][0]["supported_since"], "0.4.0")

    def test_client_contract_mismatch_fixture(self):
        baseline = DATA / "client-mismatch-baseline.json"  # known_broken 2.1.286 under contract A (not main's B)
        rep = DATA / "client-mismatch-report.json"          # this run verifies 2.1.286 under B (the stub's id)
        out_path = self.dir / "out.json"
        args = ["write", "--baseline", baseline, "--report", rep, "--binary", STUB, "--generated-at", NOW, "--out", out_path]
        self.cli(*args, check=0)
        keys = set(self.by_key(json.loads(out_path.read_text())))
        a = "aaaaaaaaaaaaaaaa"
        self.assertIn(("claude", "2.1.286", MAIN["claude"]), keys)
        self.assertNotIn(("claude", "2.1.286", a), keys)  # A is not live: neither main's nor the release's
        # with A as the release contract its row is kept next to B's verified row
        rel = self.put("release.json", release([], cid={"claude": a, "codex": MAIN["codex"]}))
        self.cli(*args, "--release-results", rel, "--latest-release", "v0.4.0", check=0)
        out = json.loads(out_path.read_text())
        keys = self.by_key(out)
        self.assertEqual(keys[("claude", "2.1.286", a)]["status"], "known_broken")
        self.assertEqual(keys[("claude", "2.1.286", MAIN["claude"])]["status"], "verified")

    def test_supported_since_filled_from_the_passing_release(self):
        base = doc([row("claude", "2.1.286", supported_since="0.3.0")])
        rep = report([probe("2.1.286"), probe("2.1.288")])
        rel = release([("claude", "2.1.286", [SESSION_OK]), ("claude", "2.1.288", [SESSION_OK])], cid={"claude": OTHER, "codex": MAIN["codex"]})
        _, out = self.write(base, rep, rel)
        k = self.by_key(out)
        self.assertEqual(k[("claude", "2.1.286", MAIN["claude"])]["supported_since"], "0.3.0")  # existing value wins
        self.assertEqual(k[("claude", "2.1.288", MAIN["claude"])]["supported_since"], "0.4.0")

    def test_reprobed_other_contract_known_broken_gains_main_row_with_supported_since(self):
        base = doc([row("claude", "2.1.290", "known_broken", cid=OTHER)])
        rep = report([probe("2.1.290", role="reprobe")])
        rel = release([("claude", "2.1.290", [SESSION_OK, TOOL_OK])], tag="v0.5.0")  # C2 has shipped
        _, out = self.write(base, rep, rel, tag="v0.5.0")
        k = self.by_key(out)
        main_row = k[("claude", "2.1.290", MAIN["claude"])]
        self.assertEqual((main_row["status"], main_row["supported_since"]), ("verified", "0.5.0"))
        self.assertNotIn(("claude", "2.1.290", OTHER), k)  # no longer a live contract
        # the release still carries the other contract: a main row, but no supported_since, and the old row stays
        rel = release([("claude", "2.1.290", [VIOLATION])], cid={"claude": OTHER, "codex": MAIN["codex"]}, tag="v0.5.0")
        _, out = self.write(base, rep, rel, tag="v0.5.0")
        k = self.by_key(out)
        self.assertEqual(k[("claude", "2.1.290", MAIN["claude"])]["status"], "verified")
        self.assertIsNone(k[("claude", "2.1.290", MAIN["claude"])]["supported_since"])
        self.assertEqual(k[("claude", "2.1.290", OTHER)]["status"], "known_broken")
        # so the next run re-probes again
        vspec = importlib.util.spec_from_file_location("versions_under_test", HERE / "versions.py")
        versions = importlib.util.module_from_spec(vspec)
        vspec.loader.exec_module(versions)
        self.assertEqual(versions.reprobe(["2.1.290"], out, "claude", MAIN["claude"]), ["2.1.290"])

    def test_supported_since_not_filled_when_the_release_contract_violates(self):
        rel = release([("claude", "2.1.288", [VIOLATION])], cid={"claude": OTHER, "codex": MAIN["codex"]})
        _, out = self.write(doc([]), report([probe("2.1.288")]), rel)
        k = self.by_key(out)
        self.assertIsNone(k[("claude", "2.1.288", MAIN["claude"])]["supported_since"])
        self.assertEqual(k[("claude", "2.1.288", OTHER)]["status"], "known_broken")
        # a release that predates contract-id (unsupported) fills nothing
        _, out = self.write(doc([]), report([probe("2.1.288")]), release([], supported=False))
        self.assertIsNone(out["rows"][0]["supported_since"])

    def test_retention_and_size_cap(self):
        rows = [row("claude", f"1.0.{i}") for i in range(60)]
        rows += [row("claude", "0.9.0", "known_broken"), row("claude", "0.9.1", recipe="r"), row("claude", "0.9.2", source="manual")]
        _, out = self.write(doc(rows), report([probe("2.0.0")]))
        versions = [r["version"] for r in out["rows"] if r["harness"] == "claude"]
        self.assertEqual(len([v for v in versions if v.startswith("1.0.")]), 49)  # 50 newest = 2.0.0 + 49
        self.assertIn("1.0.59", versions)
        self.assertNotIn("1.0.10", versions)
        self.assertTrue({"0.9.0", "0.9.1", "0.9.2", "2.0.0"} <= set(versions))
        pad = "x" * 2000
        big = doc([row("claude", f"1.{i}.0", issue_url=pad, source="manual") for i in range(120)])
        (self.dir / "out.json").unlink()
        cp, _ = self.write(big, report([]))
        self.assertEqual(cp.returncode, 1)
        self.assertRegex(cp.stdout, r"manifest too large: \d+ bytes > 209715")
        self.assertFalse((self.dir / "out.json").exists())

    def test_schema_1_baseline_is_upgraded(self):
        v1 = {"schema_version": 1, "rows": [{"harness": "claude", "version": "2.1.283", "recipe": "claude-hooks-2.1.283",
                                             "evidence": "live", "known_broken": []}]}
        _, out = self.write(v1, report([]))
        self.assertEqual(out["schema_version"], 2)
        r = out["rows"][0]
        self.assertEqual((r["version"], r["status"], r["source"], r["recipe"], r["contract_id"]),
                         ("2.1.283", "verified", "manual", "claude-hooks-2.1.283", None))

    def test_both_harnesses_in_one_run(self):
        rep = report([probe("2.1.288")])
        rep["harnesses"].append({"harness": "codex", "status": "all_pass", "first_bad": None, "probes": [probe("0.160.0")]})
        _, out = self.write(doc([]), rep)
        self.assertEqual({(r["harness"], r["contract_id"]) for r in out["rows"]},
                         {("claude", MAIN["claude"]), ("codex", MAIN["codex"])})


class WriterInfra(Base):
    REL = OTHER

    def _baseline(self):
        return doc([row("claude", "2.1.286"), row("claude", "2.1.285", cid=self.REL)])

    def test_release_infra_error_keeps_baseline_release_rows(self):
        cp, out = self.write(self._baseline(), report([]), release([], error="build failed"))
        self.assertEqual(cp.returncode, 0, cp.stderr)
        self.assertEqual(set(self.by_key(out)), {("claude", "2.1.286", MAIN["claude"]), ("claude", "2.1.285", self.REL)})
        self.assertIn("build failed", cp.stderr)

    def test_release_unsupported_still_drops_release_rows(self):
        cp, out = self.write(self._baseline(), report([]), release([], supported=False))
        self.assertEqual(cp.returncode, 0, cp.stderr)
        self.assertEqual(set(self.by_key(out)), {("claude", "2.1.286", MAIN["claude"])})

    def _tier0(self, failing):
        checks = [{"id": i, "status": "fail", "detail": "x"} for i in failing]
        _, out = self.write(doc([]), report([probe("2.1.287", [SESSION_OK, VIOLATION], result="fail", checks=checks)]))
        return self.by_key(out)

    def test_violation_with_unrelated_tier0_failure_writes_no_row(self):
        rows = self._tier0(["t0.setup", "t0.payload-parse"])
        self.assertNotIn(("claude", "2.1.287", MAIN["claude"]), rows)

    def test_violation_with_only_payload_parse_failing_is_known_broken(self):
        r = self._tier0(["t0.payload-parse"])[("claude", "2.1.287", MAIN["claude"])]
        self.assertEqual((r["status"], r["broken_event"], r["broken_field"]), ("known_broken", "SessionStart", "session_id"))

    def test_release_violation_with_unrelated_source_tier0_failure_writes_no_row(self):
        checks = [{"id": "t0.setup", "status": "fail"}, {"id": "t0.payload-parse", "status": "fail"}]
        rep = report([probe("2.1.287", [VIOLATION], result="fail", checks=checks)])
        rel = release([("claude", "2.1.287", [VIOLATION])], cid={"claude": OTHER, "codex": MAIN["codex"]})
        cp, out = self.write(doc([]), rep, rel)
        self.assertEqual(cp.returncode, 0, cp.stderr)
        self.assertEqual(out["rows"], [])

    def test_release_violation_with_only_source_payload_parse_failing_breaks_both_contracts(self):
        checks = [{"id": "t0.payload-parse", "status": "fail"}]
        rep = report([probe("2.1.287 (Claude Code)", [VIOLATION], result="fail", checks=checks)])
        rel = release([("claude", "2.1.287", [VIOLATION])], cid={"claude": OTHER, "codex": MAIN["codex"]})
        cp, out = self.write(doc([]), rep, rel)
        self.assertEqual(cp.returncode, 0, cp.stderr)
        self.assertEqual({(r["contract_id"], r["status"], r["broken_event"], r["broken_field"]) for r in out["rows"]},
                         {(MAIN["claude"], "known_broken", "SessionStart", "session_id"),
                          (OTHER, "known_broken", "SessionStart", "session_id")})

    def test_excluded_source_keeps_existing_broken_rows_for_both_live_contracts(self):
        baseline = [row("claude", "2.1.287", "known_broken", cid=cid, last_working="2.1.280",
                        broken_event="PreToolUse", broken_field="tool_input", issue_url="https://github.com/o/r/issues/9")
                    for cid in (MAIN["claude"], OTHER)]
        rep = report([probe("2.1.287", [VIOLATION], result="fail", checks=[{"id": "t0.setup", "status": "fail"}])])
        rel = release([("claude", "2.1.287", [VIOLATION])], cid={"claude": OTHER, "codex": MAIN["codex"]})
        cp, out = self.write(doc(baseline), rep, rel)
        self.assertEqual(cp.returncode, 0, cp.stderr)
        self.assertEqual(self.by_key(out), {(r["harness"], r["version"], r["contract_id"]): r for r in baseline})

    def test_release_violation_without_source_attempt_writes_no_row(self):
        rel = release([("claude", "2.1.287", [VIOLATION])], cid={"claude": OTHER, "codex": MAIN["codex"]})
        no_attempts = probe("2.1.287", [])
        no_attempts["attempts"] = []
        for probes in ([], [no_attempts], [probe("2.1.288", [])]):
            with self.subTest(probes=probes):
                cp, out = self.write(doc([]), report(probes), rel)
                self.assertEqual(cp.returncode, 0, cp.stderr)
                self.assertEqual(out["rows"], [])

    def test_release_violation_with_ambiguous_source_writes_no_row(self):
        eligible = probe("2.1.287", [], result="fail", checks=[{"id": "t0.payload-parse", "status": "fail"}])
        excluded = probe("2.1.287 (Claude Code)", [], result="fail", checks=[{"id": "t0.setup", "status": "fail"}])
        rel = release([("claude", "2.1.287", [VIOLATION])], cid={"claude": OTHER, "codex": MAIN["codex"]})
        for probes in ([eligible, excluded], [excluded, eligible], [eligible, copy.deepcopy(eligible)]):
            with self.subTest(probes=probes):
                cp, out = self.write(doc([]), report(probes), rel)
                self.assertEqual(cp.returncode, 0, cp.stderr)
                self.assertEqual(out["rows"], [])

    def test_release_source_eligibility_uses_only_the_final_attempt(self):
        first = probe("2.1.287", [VIOLATION], result="fail", checks=[{"id": "t0.setup", "status": "fail"}])
        last = probe("2.1.287", [VIOLATION], result="fail", checks=[{"id": "t0.payload-parse", "status": "fail"}])
        last["attempts"].insert(0, first["attempts"][0])
        rel = release([("claude", "2.1.287", [VIOLATION])], cid={"claude": OTHER, "codex": MAIN["codex"]})
        cp, out = self.write(doc([]), report([last]), rel)
        self.assertEqual(cp.returncode, 0, cp.stderr)
        self.assertEqual({r["contract_id"] for r in out["rows"]}, {MAIN["claude"], OTHER})


class Contract(Base):
    def test_contract_prints_the_stub_ids(self):
        got = json.loads(self.cli("contract", "--binary", STUB, check=0).stdout)
        self.assertEqual({h: got[h] for h in MAIN}, MAIN)

    @unittest.skipUnless(os.access(ROOT / "target/debug/herdr-threads", os.X_OK), "herdr-threads is not built")
    def test_contract_with_the_real_binary(self):
        binary = str(ROOT / "target/debug/herdr-threads")
        got = json.loads(self.cli("contract", "--binary", binary, check=0).stdout)
        own = json.loads(subprocess.run([binary, "contract-id", "--json"], capture_output=True, text=True, check=True).stdout)
        for h in MAIN:
            self.assertRegex(got[h], r"^[0-9a-f]{16}$")
            self.assertEqual(got[h], own[h])

    def test_contract_fails_loudly_on_a_failing_binary(self):
        cp = self.cli("contract", "--binary", "/usr/bin/false")
        self.assertNotEqual(cp.returncode, 0)
        self.assertIn("contract-id", cp.stderr)


class Validate(Base):
    def violations(self, d):
        p = self.put("m.json", d)
        cp = self.cli("validate", p)
        return cp.returncode, cp.stdout

    def test_validate_accepts_generator_and_writer_output(self):
        # the in-repo generator's output (null contract ids, recipe rows) and the Rust reader's writer-shaped fixture
        for path in (ROOT / "docs/compatibility/harness-versions.json", ROOT / "tests/harness/testdata/manifest/writer-output.json"):
            with self.subTest(path=path.name):
                self.cli("validate", path, check=0)
        _, out = self.write(doc([]), report([probe("2.1.288")]))
        self.cli("validate", self.dir / "out.json", check=0)

    def test_validate_rejects_schema_violations(self):
        good = row("claude", "2.1.288")

        def with_row(**kw):
            return doc([{**good, **kw}])

        cases = {
            "schema_version": doc([good], schema_version=1),
            "rows not a list": {**doc([good]), "rows": {}},
            "harness": with_row(harness="gemini"),
            "version not X.Y.Z": with_row(version="2.1"),
            "version prerelease": with_row(version="2.1.288-beta"),
            "status": with_row(status="maybe"),
            "evidence": with_row(evidence="vibes"),
            "source": with_row(source="robot"),
            "contract_id uppercase": with_row(contract_id="C1A0C1A0C1A0C1A0"),
            "contract_id short": with_row(contract_id="abc"),
            "known_broken without event": with_row(status="known_broken", broken_event=None, broken_field="f"),
            "known_broken without field": with_row(status="known_broken", broken_event="e", broken_field=None),
            "duplicate key": doc([good, dict(good)]),
            "too large": doc([row("claude", f"1.{i}.0", issue_url="x" * 2000) for i in range(120)]),
        }
        for name, d in cases.items():
            with self.subTest(name):
                code, out = self.violations(d)
                self.assertEqual(code, 1, out)
                self.assertTrue(out.strip(), "a violation is described")
        # a null evidence, null contract_id and the B6 `none` evidence are fine
        for ok in (with_row(evidence=None), with_row(contract_id=None), with_row(evidence="none")):
            self.assertEqual(self.violations(ok)[0], 0)
        # the same version under two contracts is two keys, not a duplicate
        self.assertEqual(self.violations(doc([good, {**good, "contract_id": OTHER}]))[0], 0)
        self.assertEqual(self.cli("validate", self.dir / "missing.json").returncode, 1)
        self.put("bad.json", "not json")
        self.assertEqual(self.cli("validate", self.dir / "bad.json").returncode, 1)


class SetCommand(Base):
    def test_set_writes_manual_row_and_validates(self):
        path = self.dir / "m.json"
        self.cli("set", "--file", path, "--harness", "claude", "--version", "2.1.290 (Claude Code)", "--status", "known_broken",
                 "--broken-event", "PreToolUse", "--broken-field", "tool_input.command", "--last-working", "2.1.288",
                 "--binary", STUB, check=0)
        d = json.loads(path.read_text())
        r = d["rows"][0]
        self.assertEqual((r["version"], r["source"], r["contract_id"], r["status"]), ("2.1.290", "manual", MAIN["claude"], "known_broken"))
        self.assertEqual(list(r), list(manifest.ROW_KEYS))
        self.cli("validate", path, check=0)
        self.assertIn(["harness-version", "normalize", "claude", "2.1.290 (Claude Code)"], self.calls())
        # a second set upserts the same key rather than duplicating it
        self.cli("set", "--file", path, "--harness", "claude", "--version", "2.1.290", "--status", "verified",
                 "--evidence", "live", "--binary", STUB, check=0)
        rows = json.loads(path.read_text())["rows"]
        self.assertEqual([(r["status"], r["source"]) for r in rows], [("verified", "manual")])
        # an explicit contract id gives a second key
        self.cli("set", "--file", path, "--harness", "claude", "--version", "2.1.290", "--status", "verified",
                 "--contract-id", OTHER, "--binary", STUB, check=0)
        self.assertEqual(len(json.loads(path.read_text())["rows"]), 2)

    def test_set_refuses_what_validate_would(self):
        path = self.dir / "m.json"
        cp = self.cli("set", "--file", path, "--harness", "claude", "--version", "2.1.290", "--status", "known_broken",
                      "--binary", STUB)
        self.assertEqual(cp.returncode, 1)
        self.assertIn("broken_event", cp.stdout)
        self.assertFalse(path.exists())
        cp = self.cli("set", "--file", path, "--harness", "claude", "--version", "nightly", "--status", "verified", "--binary", STUB)
        self.assertEqual(cp.returncode, 1)
        self.assertFalse(path.exists())

    def test_contract_prints_the_binarys_document(self):
        cp = self.cli("contract", "--binary", STUB, check=0)
        got = json.loads(cp.stdout)
        self.assertEqual({h: got[h] for h in MAIN}, MAIN)


class CheckRuleset(Base):
    def rc(self, name):
        return self.cli("check-ruleset", "--rulesets-json", DATA / f"{name}.json", "--default-branch", "main")

    def test_check_ruleset_complete_passes(self):
        self.assertEqual(self.rc("rulesets-complete").returncode, 0)

    def test_check_ruleset_incomplete_fails_and_says_what_is_missing(self):
        for name, needle in (("rulesets-no-tag", "refs/tags/v*"), ("rulesets-inactive", "default branch"),
                             ("rulesets-missing-rule", "non_fast_forward"), ("rulesets-empty", "default branch")):
            with self.subTest(name):
                cp = self.rc(name)
                self.assertEqual(cp.returncode, 1)
                self.assertIn(needle, cp.stdout)

    def test_check_ruleset_unreadable_input_fails(self):
        self.put("junk.json", "not json")
        cp = self.cli("check-ruleset", "--rulesets-json", self.dir / "junk.json", "--default-branch", "main")
        self.assertEqual(cp.returncode, 1)

    def test_check_ruleset_excluded_ref_does_not_count(self):
        d = json.loads((DATA / "rulesets-complete.json").read_text())
        d[1]["conditions"]["ref_name"]["exclude"] = ["refs/tags/v*"]
        self.put("ex.json", d)
        self.assertEqual(self.cli("check-ruleset", "--rulesets-json", self.dir / "ex.json").returncode, 1)


class Embed(Base):
    def test_embed_copies_valid_branch_file(self):
        out = self.put("in-repo.json", "in-repo\n")
        branch = self.put("branch.json", doc([row("claude", "2.1.288")]))
        cp = self.cli("embed", "--branch-file", branch, "--out", out, check=0)
        self.assertIn("embedded harness-manifest branch file", cp.stdout)
        self.assertEqual(out.read_bytes(), branch.read_bytes())

    def test_embed_keeps_in_repo_file_on_invalid_or_oversized(self):
        bad = {
            "missing": self.dir / "nope.json",
            "not json": self.put("a.json", "{"),
            "schema 1": self.put("b.json", {"schema_version": 1, "rows": []}),
            "bad row": self.put("c.json", doc([row("claude", "2.1.288", status="maybe")])),
            "oversized": self.put("d.json", doc([], pad="x" * (manifest.MAX_MANIFEST_BYTES + 1))),
        }
        for name, path in bad.items():
            with self.subTest(name):
                out = self.put("in-repo.json", "in-repo\n")
                cp = self.cli("embed", "--branch-file", path, "--out", out, check=0)
                self.assertIn("::notice::", cp.stdout)
                self.assertEqual(out.read_text(), "in-repo\n")


class WriterFixture(Base):
    """tests/harness/testdata/manifest/writer-output.json is the writer's own output (the Rust reader parses it)."""

    FIXTURE = ROOT / "tests/harness/testdata/manifest/writer-output.json"

    def regenerate(self):
        """The command behind the fixture: the selftest `all-pass` case, then `payload-break` on top of its output."""
        cases = ROOT / "scripts/harness-canary-selftest/manifest-cases"
        first = self.dir / "first.json"
        out = self.dir / "writer-output.json"
        stages = [("all-pass", cases.joinpath("all-pass.json"), None, first), ("payload-break", cases.joinpath("payload-break.json"), first, out)]
        for _, case_path, prev, dest in stages:
            case = json.loads(case_path.read_text())
            baseline = prev or self.put("base.json", case["baseline"])
            self.cli("write", "--baseline", baseline, "--report", self.put("rep.json", case["report"]), "--binary", STUB,
                     "--generated-at", "2026-10-02T06:00:00Z", "--out", dest, check=0)
        return out.read_bytes()

    def test_regenerating_reproduces_the_committed_fixture(self):
        got = self.regenerate()
        if os.environ.get("HT_BLESS") == "1":
            self.FIXTURE.write_bytes(got)
        self.assertEqual(got, self.FIXTURE.read_bytes(),
                         "writer-output.json drifted from the writer (HT_BLESS=1 python3 -m unittest scripts/canary/test_manifest.py)")



class RuntimeFixtures(Base):
    # Catches dropping rich baseline collections and semver/stage promotion.
    def setUp(self):
        super().setUp()
        import test_strategy
        self.adapter = test_strategy.adapter()
        self.adapter['canary_strategy'] = None
        self.discovery = {'schema_version': 1, 'adapters': [self.adapter]}
        self.binary = self.dir / 'binary'
        self.binary.write_text('#!' + sys.executable + '\nimport json,os,sys\n'
                               'if sys.argv[1:] == ["adapters", "--json"]:\n'
                               ' print(' + repr(json.dumps(self.discovery)) + ')\n'
                               'else: os.execv(' + repr(STUB) + ', [' + repr(STUB) + '] + sys.argv[1:])\n')
        self.binary.chmod(0o755)
        self.result = test_strategy.result()
        descriptor = {'source': 'source', 'release_version': None, 'base_version': '1.2.3',
                      'derived_version': '1.2.3-dev', 'commit': 'a' * 40, 'dirty': False, 'distance': 1}
        import hashlib
        key = 'build:' + hashlib.sha256(json.dumps(dict(descriptor, schema_version=1),
                                    sort_keys=True, separators=(',', ':')).encode()).hexdigest()
        self.result['identity'] = dict(key=key, **descriptor)
        self.work = self.dir / 'work/third-try1'
        self.work.mkdir(parents=True)
        self.index = {'schema_version': 1, 'attempts': [{'harness': 'third', 'attempt': 'try1',
                      'identity_key': key, 'evidence_stage': 'no_model',
                      'result_path': 'work/third-try1/result.json', 'capture_paths': []}]}

    def write_runtime(self, baseline=None, release_doc=None):
        (self.work / 'result.json').write_text(json.dumps(self.result))
        args = ['write', '--binary', self.binary, '--baseline', self.put('baseline.json', baseline or doc([])),
                '--report', self.put('report.json', report([])), '--artifact-index', self.put('artifact-index.json', self.index),
                '--generated-at', NOW, '--out', self.dir / 'out.json']
        if release_doc is not None:
            args += ['--release-results', self.put('release.json', release_doc)]
        cp = self.cli(*args)
        return cp, json.loads((self.dir / 'out.json').read_text()) if cp.returncode == 0 else None



class RuntimeWriter(RuntimeFixtures):
    def test_external_runtime_manifest_preserves_baselines_and_exact_release_replay_stage(self):
        cp, out = self.write_runtime()
        self.assertEqual(cp.returncode, 0, cp.stderr)
        rich = out['runtime_rows'][0]
        self.assertEqual(rich['identity'], self.result['identity'])
        self.assertEqual(rich['evidence_stage'], 'no_model')
        self.assertEqual(rich['last_seen_at'], 1790920800000)
        self.assertEqual(out['rows'], [])
        rich['source'] = 'manual'
        baseline = dict(out, rows=[row('claude', '2.1.286', cid=THIRD)],
                        contracts={'claude': THIRD, 'codex': MAIN['codex']})
        self.result['outcome'] = 'infra_failure'
        cp, kept = self.write_runtime(baseline, {'tag': 'v1.0.0', 'error': 'build failed'})
        self.assertEqual(cp.returncode, 0, cp.stderr)
        for collection in ('rows', 'contracts', 'runtime_rows', 'runtime_contracts'):
            self.assertEqual(kept[collection], baseline[collection])
        cp, kept = self.write_runtime(baseline)
        self.assertEqual(cp.returncode, 0, cp.stderr)
        for collection in ('rows', 'contracts', 'runtime_rows', 'runtime_contracts'):
            self.assertEqual(kept[collection], baseline[collection])

    def test_incomplete_capture_unsupported_and_bad_identity_cannot_verify(self):
        for change in ({'evidence_stage': 'source_captured'}, {'outcome': 'unsupported'},
                       {'outcome': 'inconclusive'}):
            with self.subTest(change=change):
                self.result.update(change)
                self.index['attempts'][0]['evidence_stage'] = self.result['evidence_stage']
                cp, out = self.write_runtime()
                self.assertEqual(cp.returncode, 0, cp.stderr)
                self.assertEqual(out['runtime_rows'], [])
                self.result.update(outcome='complete', evidence_stage='no_model')
        self.index['attempts'][0]['evidence_stage'] = 'no_model'
        self.result['domains'][0]['successful_milestones'] = ['session_start']
        cp, out = self.write_runtime()
        self.assertEqual(cp.returncode, 0, cp.stderr)
        self.assertEqual(out['runtime_rows'], [])
        self.index['attempts'][0]['identity_key'] = 'build:' + 'b' * 64
        cp, _ = self.write_runtime()
        self.assertNotEqual(cp.returncode, 0)

    def test_runtime_validation_rejects_timestamp_and_origin_mismatch(self):
        cp, out = self.write_runtime()
        self.assertEqual(cp.returncode, 0, cp.stderr)
        for value in (-1, True, '2026-10-02T06:00:00Z', 2**64):
            bad = copy.deepcopy(out)
            bad['runtime_rows'][0]['last_seen_at'] = value
            self.assertTrue(manifest.validate_doc(bad), value)
        bad = copy.deepcopy(out)
        bad['runtime_rows'][0]['origin'] = 'bridge_envelope'
        self.assertTrue(manifest.validate_doc(bad))

class ReleaseReplay(RuntimeFixtures):
    # Catches filename-derived joins, stage promotion and symlink escape.
    def prepare(self):
        self.put('artifact-index.json', self.index)
        (self.work / 'result.json').write_text(json.dumps(self.result))
        (self.work / 'request.json').write_text(json.dumps({'adapter': self.adapter}))
        return manifest.prepare_replay(self.dir, None, MAIN)

    def test_old_release_replays_only_representable_indexed_legacy(self):
        plan = self.prepare()
        self.assertEqual(plan['probes'], [])  # exact builds are not legacy versions
        self.assertEqual(plan['unsupported'][0]['domain'], 'native_shape')
        self.result['identity'] = {'key': 'release:2.1.288', 'release_version': '2.1.288',
                'source': 'npm', 'base_version': None, 'derived_version': None,
                'commit': None, 'dirty': False, 'distance': None}
        self.result['harness'] = 'claude'
        self.adapter['id'] = 'claude'
        self.adapter['contracts'][0].update(domain='native_payload', origin='native_payload')
        self.result['domains'][0].update(domain='native_payload', origin='native_payload')
        self.index['attempts'][0].update(harness='claude', identity_key='release:2.1.288',
                    result_path='work/claude-try1/result.json')
        self.work.rename(self.dir / 'work/claude-try1')
        self.work = self.dir / 'work/claude-try1'
        (self.work / 'capture.stdin').write_text('{}')
        (self.work / 'capture.argv').write_text('hook\nclaude\n')
        captures = []
        for suffix in ('stdin', 'argv'):
            name = 'meta-' + suffix + '.json'
            (self.work / name).write_text(json.dumps({'domain': 'native_payload', 'origin': 'native_payload',
                 'evidence_stage': 'no_model', 'path': 'capture.' + suffix}))
            captures.append('work/claude-try1/' + name)
        self.index['attempts'][0]['capture_paths'] = captures
        plan = self.prepare()
        self.assertEqual(len(plan['probes']), 1)
        self.assertEqual(plan['probes'][0]['evidence_stage'], 'no_model')
        self.assertEqual(plan['probes'][0]['attempt'], 'try1')
        (self.dir / plan['probes'][0]['work_path'] / 'canary-rust.json').write_text(json.dumps({'payloads': [{'contract': TOOL_OK}]}))
        replay = manifest.collect_replay(self.dir, plan, 'v1.0.0')
        self.assertEqual(replay['probes'][0]['evidence_stage'], 'no_model')
        self.assertEqual(manifest.release_observations(replay, report([probe('2.1.288')]), lambda h,v:v)[0][2], ('verified', 'no_model'))

    def test_release_index_rejects_traversal_symlink_and_ambiguous_source(self):
        self.prepare()
        self.index['attempts'][0]['result_path'] = '../result.json'
        with self.assertRaises(ValueError): self.prepare()
        self.index['attempts'][0]['result_path'] = 'work/third-try1/result.json'
        (self.work / 'result.json').unlink()
        (self.work / 'result.json').symlink_to(self.put('outside.json', self.result))
        self.put('artifact-index.json', self.index)
        with self.assertRaises(ValueError): manifest.prepare_replay(self.dir, self.discovery, MAIN)

class RuntimeHistory(RuntimeFixtures):
    # Catches pruning protected history and misclassifying declared violations.
    def test_runtime_broken_manual_and_bounded_retention(self):
        cp, out = self.write_runtime()
        self.assertEqual(cp.returncode, 0, cp.stderr)
        seed = out['runtime_rows'][0]
        history = []
        for n in range(55):
            r = copy.deepcopy(seed)
            r['identity'] = {'key': f'release:1.0.{n}', 'release_version': f'1.0.{n}', 'source': 'npm',
                             'base_version': None, 'derived_version': None, 'commit': None, 'dirty': False, 'distance': None}
            r['last_seen_at'] = n
            r['harness'] = 'claude'
            history.append(r)
        history[0]['source'] = 'manual'
        history[1].update(status='known_broken', broken_event='SessionStart', broken_field='session_id')
        out['runtime_rows'] = history
        out['runtime_contracts']['claude'] = copy.deepcopy(out['runtime_contracts']['third'])
        out['rows'] = [row('claude', '1.0.2', recipe='current')]
        cp, kept = self.write_runtime(out)
        self.assertEqual(cp.returncode, 0, cp.stderr)
        self.assertEqual(len(kept['runtime_rows']), 54)
        self.assertIn('release:1.0.0', [r['identity']['key'] for r in kept['runtime_rows']])
        self.result['domains'][0].update(outcome='contract_violation', violations=[{'event':'SessionStart', 'field':'session_id'}])
        cp, broken = self.write_runtime()
        self.assertEqual(cp.returncode, 0, cp.stderr)
        self.assertEqual(broken['runtime_rows'][0]['status'], 'known_broken')
        self.assertEqual(broken['runtime_rows'][0]['broken_field'], 'session_id')
        # Each valid protected row stays, so too much protected history fails atomically.
        out['runtime_rows'] = history * 10
        for n, r in enumerate(out['runtime_rows']):
            out['runtime_rows'][n] = copy.deepcopy(r)
            out['runtime_rows'][n]['identity'].update(key=f'release:2.0.{n}', release_version=f'2.0.{n}')
            out['runtime_rows'][n]['source'] = 'manual'
        before = (self.dir / 'out.json').read_bytes()
        cp, _ = self.write_runtime(out)
        self.assertNotEqual(cp.returncode, 0)
        self.assertIn('manifest too large', cp.stdout)
        self.assertEqual((self.dir / 'out.json').read_bytes(), before)

class ReplayAttribution(RuntimeFixtures):
    def test_release_runtime_requires_exact_source_attempt_and_preserves_stage(self):
        a = copy.deepcopy(self.adapter)
        a['contracts'][0].update(domain='native_payload', origin='native_payload', id=OTHER)
        self.result['domains'][0].update(domain='native_payload', origin='native_payload')
        source = [(self.index['attempts'][0], self.result)]
        replay = {'discovery': {'schema_version':1, 'adapters':[a]}, 'tag':'v1.0.0',
                  'probes':[{'harness':'third', 'attempt':'try1', 'identity': self.result['identity'],
                             'evidence_stage':'no_model', 'payloads':[SESSION_OK, {'event':'TurnEnd','kind':'ok','field':None}]}]}
        registry, results = manifest.replay_results(replay, source)
        out = manifest.add_runtime(doc([]), doc([]), registry, results, NOW)
        self.assertEqual(out['runtime_rows'][0]['evidence_stage'], 'no_model')
        self.assertEqual(out['runtime_rows'][0]['contract_id'], OTHER)
        replay['probes'][0]['attempt'] = 'other'
        with self.assertRaises(ValueError): manifest.replay_results(replay, source)
        replay['discovery'] = None
        with self.assertRaises(ValueError): manifest.replay_results(replay, source)

class RuntimeSetup(RuntimeFixtures):
    def test_legacy_setup_failure_never_creates_rich_broken_or_verified_rows(self):
        (self.work / 'legacy-probe.json').write_text(json.dumps({'checks': [{'id':'t0.help','status':'fail'}], 'result':'fail'}))
        cp, out = self.write_runtime()
        self.assertEqual(cp.returncode, 0, cp.stderr)
        self.assertEqual(out['runtime_rows'], [])


class ReleaseShell(RuntimeFixtures):
    prepare = ReleaseReplay.prepare
    def test_shell_uses_built_release_legacy_fallback_and_writes_infra_error(self):
        self.prepare()
        fakebin = self.dir / 'fakebin'
        fakebin.mkdir()
        git = fakebin / 'git'
        git.write_text('#!' + sys.executable + '\nimport pathlib,shutil,sys\n'
                       'a=sys.argv[1:]\n'
                       'if "add" in a: pathlib.Path(a[-2]).mkdir(parents=True)\n'
                       'elif "remove" in a: shutil.rmtree(a[-1])\n')
        git.chmod(0o755)
        cargo = fakebin / 'cargo'
        cargo.write_text('#!' + sys.executable + '\nimport os,pathlib,shutil\n'
                         'p=pathlib.Path(os.environ["CARGO_TARGET_DIR"])/"debug/herdr-threads"\n'
                         'p.parent.mkdir(parents=True,exist_ok=True)\n'
                         'shutil.copyfile(' + repr(STUB) + ',p);p.chmod(0o755)\n')
        cargo.chmod(0o755)
        home = self.dir / 'home'
        home.mkdir()
        env = dict(self.env, PATH=str(fakebin) + os.pathsep + os.environ['PATH'], HOME=str(home))
        def run_shell():
            return subprocess.run(['bash', str(HERE / 'release_contract.sh'), 'v1.0.0', str(self.dir)],
                                  env=env, capture_output=True, text=True, timeout=30)
        cp = run_shell()
        self.assertEqual(cp.returncode, 0, cp.stderr)
        result = json.loads((self.dir / 'release-contract.json').read_text())
        self.assertTrue(result['supported'])
        self.assertIsNone(result['discovery'])
        self.assertEqual(result['probes'], [])
        self.assertEqual(result['unsupported_domains'][0]['domain'], 'native_shape')
        rich = self.dir / 'rich-release'
        rich.write_text('#!' + sys.executable + '\nimport sys\n'
                        'if sys.argv[1:] != ["adapters", "--json"]: sys.exit(2)\n'
                        'print(' + repr(json.dumps(self.discovery)) + ')\n')
        rich.chmod(0o755)
        cargo.write_text(cargo.read_text().replace(repr(STUB), repr(str(rich))))
        cp = run_shell()
        self.assertEqual(cp.returncode, 0, cp.stderr)
        result = json.loads((self.dir / 'release-contract.json').read_text())
        self.assertTrue(result['supported'])
        self.assertEqual(result['discovery'], self.discovery)
        self.index['attempts'][0]['result_path'] = '../escape.json'
        self.put('artifact-index.json', self.index)
        cp = run_shell()
        self.assertEqual(cp.returncode, 2, cp.stderr)
        result = json.loads((self.dir / 'release-contract.json').read_text())
        self.assertIn('invalid indexed source artifacts', result['error'])
        self.assertFalse((self.dir / 'release-src').exists())


class ReleaseSupport(RuntimeFixtures):
    def test_unsupported_release_does_not_support_main_runtime(self):
        cp, out = self.write_runtime(release_doc={'tag':'v1.0.0', 'supported':False})
        self.assertEqual(cp.returncode, 0, cp.stderr)
        self.assertIsNone(out['runtime_rows'][0]['supported_since'])


class MissingEvidence(Base):
    def test_infra_probe_preserves_legacy_rows_and_contracts(self):
        baseline = doc([row('claude', '2.1.286', cid=THIRD)], contracts={'claude':THIRD})
        cp, out = self.write(baseline, report([probe('2.1.287', result='infra')]))
        self.assertEqual(cp.returncode, 0, cp.stderr)
        self.assertEqual(out['rows'], baseline['rows'])
        self.assertEqual(out['contracts'], baseline['contracts'])
        cp, out = self.write(baseline, report([probe('2.1.287', result='fail', checks=[{'id':'t0.help','status':'fail'}])]))
        self.assertEqual(cp.returncode, 0, cp.stderr)
        self.assertEqual(out['rows'], baseline['rows'])
        self.assertEqual(out['contracts'], baseline['contracts'])


if __name__ == "__main__":
    unittest.main()
