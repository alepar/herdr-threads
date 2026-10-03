#!/usr/bin/env python3
"""Unit tests for admission.py (nested spec §D3 t0.admission): `python3 -m unittest discover -s scripts/canary`."""
import importlib.util, json, os, pathlib, sys

HERE = pathlib.Path(__file__).resolve().parent
# scripts/canary/bisect.py shadows the stdlib `bisect` (needed by `random`, hence `tempfile`) when
# `discover -s scripts/canary` puts this directory first on sys.path: import the stdlib ones without it.
_saved = list(sys.path)
sys.path[:] = [p for p in sys.path if os.path.realpath(p or ".") != str(HERE)]
sys.modules.pop("bisect", None)
import subprocess, tempfile, unittest  # noqa: E402
sys.path[:] = _saved
_spec = importlib.util.spec_from_file_location("canary_admission", HERE / "admission.py")
admission = importlib.util.module_from_spec(_spec)
sys.modules["canary_admission"] = admission
_spec.loader.exec_module(admission)

MATCHED = "schema-matched, live-unverified"


def doctor(harness, value):
    installed = {"admission": value, "binary": "/p/bin/x", "recipe": None, "version": "1.2.3"}
    return {"doctor": {"hooks": {harness: {"installed": installed}}}}


class Evaluate(unittest.TestCase):
    def status(self, harness, value, expected, schema=None):
        return admission.evaluate(doctor(harness, value), expected, schema, harness=harness)[0]

    def test_equal_listed_optimistic_refused_pass(self):
        for value in ("listed", "optimistic", "refused"):
            with self.subTest(value=value):
                self.assertEqual(self.status("claude", value, value), "pass")

    def test_mismatch_fails(self):
        self.assertEqual(self.status("claude", "listed", "optimistic"), "fail")
        self.assertEqual(self.status("claude", "optimistic", "listed"), "fail")
        self.assertEqual(self.status("claude", "optimistic", "refused"), "fail")
        self.assertEqual(self.status("claude", "refused", "listed"), "fail")

    def test_not_found_is_infra_whatever_was_expected(self):
        for expected in ("listed", "optimistic", "refused", "schema-matched-or-optimistic", "unasserted"):
            with self.subTest(expected=expected):
                self.assertEqual(self.status("claude", "not_found", expected), "infra")

    def test_codex_schema_match_requires_schema_matched(self):
        e = "schema-matched-or-optimistic"
        self.assertEqual(self.status("codex", MATCHED, e, "match"), "pass")
        # coverage r2: match + optimistic fails
        self.assertEqual(self.status("codex", "optimistic", e, "match"), "fail")

    def test_codex_schema_drift_or_unextractable_requires_optimistic(self):
        e = "schema-matched-or-optimistic"
        for schema in ("drift", "unextractable"):
            with self.subTest(schema=schema):
                self.assertEqual(self.status("codex", "optimistic", e, schema), "pass")
                # coverage r2: drift + 'schema-matched, live-unverified' fails
                self.assertEqual(self.status("codex", MATCHED, e, schema), "fail")

    def test_codex_deferred_verdict_without_a_schema_result_fails(self):
        e = "schema-matched-or-optimistic"
        self.assertEqual(self.status("codex", MATCHED, e, None), "fail")
        self.assertEqual(self.status("codex", "optimistic", e, "bogus"), "fail")

    def test_schema_is_ignored_for_a_closed_expectation(self):
        self.assertEqual(self.status("codex", "listed", "listed", "drift"), "pass")
        self.assertEqual(self.status("codex", "refused", "refused", "match"), "pass")

    def test_unasserted_is_recorded_not_asserted(self):
        status, detail = admission.evaluate(doctor("claude", "optimistic"), "unasserted", None, harness="claude")
        self.assertEqual(status, "skip")
        self.assertIn("optimistic", detail)

    def test_detail_names_the_observed_admission(self):
        status, detail = admission.evaluate(doctor("claude", "listed"), "optimistic", None, harness="claude")
        self.assertEqual(status, "fail")
        self.assertIn("listed", detail)
        self.assertIn("optimistic", detail)

    def test_malformed_doctor_json_fails(self):
        for doc in ({}, {"doctor": {}}, {"doctor": {"hooks": {"claude": {"installed": {}}}}},
                    {"doctor": {"hooks": {"claude": {"installed": {"admission": 7}}}}}, [], "not json"):
            with self.subTest(doc=doc):
                self.assertEqual(admission.evaluate(doc, "listed", None, harness="claude")[0], "fail")

    def test_accepts_a_json_string(self):
        self.assertEqual(admission.evaluate(json.dumps(doctor("claude", "listed")), "listed", None,
                                            harness="claude")[0], "pass")

    def test_cli_prints_status_detail_admission(self):
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "doctor.json")
            with open(path, "w") as f:
                json.dump(doctor("codex", MATCHED), f)
            out = subprocess.run([sys.executable, str(HERE / "admission.py"), "evaluate", "--harness", "codex",
                                  "--expected", "schema-matched-or-optimistic", "--schema", "match", path],
                                 capture_output=True, text=True, check=True).stdout.rstrip("\n").split("\t")
        self.assertEqual(out[0], "pass")
        self.assertEqual(out[2], MATCHED)

    def test_cli_without_a_schema_argument(self):
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "doctor.json")
            with open(path, "w") as f:
                json.dump(doctor("claude", "optimistic"), f)
            out = subprocess.run([sys.executable, str(HERE / "admission.py"), "evaluate", "--harness", "claude",
                                  "--expected", "optimistic", path], capture_output=True, text=True,
                                 check=True).stdout.split("\t")
        self.assertEqual(out[0], "pass")


if __name__ == "__main__":
    unittest.main()
