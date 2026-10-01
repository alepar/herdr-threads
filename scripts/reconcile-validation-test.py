#!/usr/bin/env python3
"""Offline tests for scripts/reconcile-validation.py (synthetic evidence only).

Run: python3 scripts/reconcile-validation-test.py
"""

import importlib.util
import json
import os
import pathlib
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("reconcile_validation", HERE / "reconcile-validation.py")
rv = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(rv)

SEAT = "seat-a"


def pair(message):
    return {"message_id": message, "recipient_id": SEAT}


def call(message, call_id="call-1", actor="root"):
    return {**pair(message), "call_id": call_id, "actor": actor, "artifact": "evidence/calls.json"}


def receipt(message, call_id="call-1", actor="root"):
    return {**pair(message), "call_id": call_id, "actor": actor,
            "receipt_id": f"{message}.{SEAT}", "artifact": "evidence/sqlite.json"}


def manifest(scenario, status="PASS", accepted=(), calls=(), receipts=(), retired=(), hints=(),
             source="cooperative", reason=""):
    return {"schema_version": 1, "run_id": "r", "scenario": scenario, "source": source,
            "status": status, "reason": reason, "accepted_pairs": list(accepted),
            "model_calls": list(calls), "db_receipts": list(receipts),
            "retired_pairs": list(retired), "transport_hints": list(hints), "artifacts": []}


def summary(harness, steps, launch="manual", mode="print", started="2026-09-30T00:00:00",
            messages=None, dry=False, sha="a" * 40):
    return {"facts": {"harness": harness, "mode": mode, "launch": launch, "dry_run": dry,
                      "started_utc": started, "messages": messages or {"initial": "m1"},
                      "versions": {"driver_repo_head": sha, "herdr": {"out": "herdr 0.9.1"}},
                      "harness_version": {"installed": "1.0"}},
            "steps": [{"step": s, "status": st, "title": s, "detail": d}
                      for s, st, d in steps]}


def write_run(root, folder, name, man, summ, sqlite=None):
    run = root / folder / name
    run.mkdir(parents=True)
    (run / "manifest.json").write_text(json.dumps(man))
    (run / "summary.json").write_text(json.dumps(summ))
    if sqlite is not None:
        (run / "sqlite-initial.json").write_text(json.dumps({"receipts": sqlite}))
    return run


BASE_STEPS = [("S16P", "PASS", ""), ("S17", "PASS", ""), ("S17H", "PASS", ""),
              ("S18", "PASS", ""), ("S18C", "PASS", "")]


class ReceiptAccounting(unittest.TestCase):
    def test_counts_are_separate_and_require_full_join(self):
        m = manifest("claude-print-prelaunch-handoff",
                     accepted=[pair("m1"), pair("m1"), pair("m2"), pair("m3"), pair("m4")],
                     calls=[call("m1"), call("m2", call_id="call-2", actor="child")],
                     # m2's receipt names the root actor: no 4-field join, so not ACKed.
                     receipts=[receipt("m1"), receipt("m1"), receipt("m2", call_id="call-2")],
                     retired=[pair("m3")],
                     hints=[pair("m4"), pair("m4"), pair("m1")])
        acc = rv.reconcile_manifest(m)
        self.assertEqual(acc["accepted"], 4)
        self.assertEqual(acc["acked"], 1)
        self.assertEqual(acc["retired"], 1)
        self.assertEqual(acc["pending"], 2)  # m2 (unjoined) and m4
        self.assertEqual(acc["duplicate_logical_pairs"], 1)
        self.assertEqual(acc["duplicate_receipt_rows"], 1)
        self.assertEqual(acc["unjoined_receipts"], 1)
        self.assertEqual(acc["transport_hints"], 3)
        self.assertEqual(acc["repeated_transport_hints"], 1)

    def test_db_acked_without_model_call_is_not_counted(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            run = write_run(root, "native-x", "r1",
                            manifest("codex-tui-prelaunch-handoff", status="FAIL",
                                     accepted=[pair("m1")]),
                            summary("codex", BASE_STEPS),
                            sqlite=[{"message_id": "m1", "seat_id": SEAT, "state": "acked"}])
            loaded = rv.load_native_run("native-x", run, root)
            self.assertEqual(loaded["accounting"]["acked"], 0)
            self.assertEqual(loaded["accounting"]["pending"], 1)
            self.assertEqual(loaded["accounting"]["db_acked_unjoined"], 1)


class Matrix(unittest.TestCase):
    def build(self, specs):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = pathlib.Path(tmp.name)
        for folder, name, man, summ in specs:
            write_run(root, folder, name, man, summ)
        runs = rv.discover_native(root, sorted({s[0] for s in specs}))
        return runs, rv.build_matrix(runs)

    def cell(self, matrix, row_id, harness):
        return next(e for e in matrix if e["row"]["id"] == row_id)["cells"][harness]

    def test_not_exercised_and_unverified_never_pass(self):
        ok = [pair("m1")], [call("m1")], [receipt("m1")]
        runs, matrix = self.build([
            ("native-a", "warn", manifest("claude-print-prelaunch-handoff.warning", "UNSUPPORTED",
                                          *ok, reason="scenario_not_exercised"),
             summary("claude", BASE_STEPS + [("SW1", "PASS", ""), ("SW2", "NOT_EXERCISED", "x")])),
            ("native-a", "kids", manifest("claude-tui-prelaunch-handoff.children", "PASS", *ok),
             summary("claude", BASE_STEPS + [("SK1", "UNVERIFIED", "y")], mode="tui")),
        ])
        self.assertEqual(self.cell(matrix, "deadline-warning", "claude")["verdict"], "PASS")
        self.assertEqual(self.cell(matrix, "coalesced-warning-wake", "claude")["verdict"], "NOT_EXERCISED")
        self.assertEqual(self.cell(matrix, "concurrent-children", "claude")["verdict"], "UNVERIFIED")
        status, failures, gaps = rv.overall(matrix, runs, {"status": "PASS"}, {"status": "PASS"})
        self.assertEqual(status, "FAIL")  # required rows have no evidence at all
        self.assertTrue(any("coalesced-warning-wake / claude: NOT_EXERCISED" in g for g in gaps))
        self.assertTrue(any("prelaunch-managed / codex: NO_EVIDENCE" in f for f in failures))

    def test_latest_live_run_decides_and_dry_or_diag_never_do(self):
        ok = [pair("m1")], [call("m1")], [receipt("m1")]
        runs, matrix = self.build([
            ("native-a", "old", manifest("codex-print-prelaunch-handoff.burst", "FAIL", *ok),
             summary("codex", BASE_STEPS + [("SB0", "PASS", ""), ("SB1", "PASS", ""), ("SB2", "FAIL", "")],
                     started="2026-09-30T01:00:00")),
            ("native-a", "new", manifest("codex-print-prelaunch-handoff.burst", "PASS", *ok),
             summary("codex", BASE_STEPS + [("SB0", "PASS", ""), ("SB1", "PASS", ""), ("SB2", "PASS", "")],
                     started="2026-09-30T02:00:00")),
            ("native-a", "dry-later", manifest("codex-print-prelaunch-handoff.burst", "UNSUPPORTED",
                                               [pair("m1")], source="synthetic"),
             summary("codex", BASE_STEPS + [("SB2", "FAIL", "")], dry=True, started="2026-09-30T03:00:00")),
            ("native-a", "diag-later", manifest("codex-print-prelaunch-handoff.burst", "FAIL", [pair("m1")]),
             summary("codex", BASE_STEPS + [("SB2", "FAIL", "")], started="2026-09-30T04:00:00")),
        ])
        cell = self.cell(matrix, "burst", "codex")
        self.assertEqual(cell["verdict"], "PASS")
        self.assertEqual([a["run"]["name"] for a in cell["attempts"]], ["old", "new"])
        self.assertEqual(cell["attempts"][0]["verdict"], "FAIL")

    def test_blocked_is_fail_and_missing_step_is_unverified(self):
        ok = [pair("m1")], [call("m1")], [receipt("m1")]
        runs, matrix = self.build([
            ("native-a", "managed", manifest("claude-print-prelaunch-handoff.managed", "FAIL", [pair("m1")]),
             summary("claude", [("S17M", "PASS", ""), ("S17", "FAIL", ""), ("S18", "BLOCKED", "")],
                     launch="managed")),
            ("native-a", "clear", manifest("claude-tui-prelaunch-handoff", "PASS", *ok),
             summary("claude", BASE_STEPS + [("S20", "PASS", ""), ("S20H", "PASS", ""), ("S21", "PASS", ""),
                                             ("S21C", "PASS", "")],
                     mode="tui", messages={"initial": "m1", "clear": "m2"})),
        ])
        self.assertEqual(self.cell(matrix, "prelaunch-managed", "claude")["verdict"], "FAIL")
        # No S19X in this driver: the clear cell cannot be PASS.
        self.assertEqual(self.cell(matrix, "clear-new", "claude")["verdict"], "UNVERIFIED")

    def test_phase_steps_follow_position(self):
        ok = [pair("m1")], [call("m1")], [receipt("m1")]
        steps = BASE_STEPS + [("S20", "PASS", ""), ("S20H", "PASS", ""), ("S21", "PASS", ""), ("S21C", "PASS", "")]
        runs, matrix = self.build([
            ("native-a", "resume", manifest("codex-print-prelaunch-handoff", "PASS", *ok),
             summary("codex", steps, messages={"initial": "m1", "resume": "m2"})),
        ])
        cell = self.cell(matrix, "restart-resume", "codex")
        self.assertEqual(cell["verdict"], "PASS")
        self.assertEqual(sorted(cell["deciding"]["steps"]), ["S20", "S20H", "S21", "S21C"])

    def test_pass_manifest_with_pending_fails_report(self):
        runs, matrix = self.build([
            ("native-a", "bad", manifest("claude-print-prelaunch-handoff", "PASS", [pair("m1")]),
             summary("claude", BASE_STEPS)),
        ])
        _, failures, _ = rv.overall(matrix, runs, {"status": "PASS"}, {"status": "PASS"})
        self.assertTrue(any("manifest PASS with pending" in f for f in failures))


class Redaction(unittest.TestCase):
    def test_home_paths_and_emails_are_redacted(self):
        text = rv.redact("see /Users/someone/x and /home/bob/y, mail a.b@example.com")
        self.assertNotIn("someone", text)
        self.assertNotIn("bob", text)
        self.assertNotIn("example.com", text)
        self.assertIn("~/x", text)


class CommittedEvidence(unittest.TestCase):
    """Smoke test against the committed folders (read-only, no git)."""

    def test_committed_evidence_reconciles(self):
        root = pathlib.Path(os.environ.get(rv.EVIDENCE_ENV) or rv.REPO / rv.RUN_REL)
        if not (root / rv.NATIVE_FOLDERS[0]).is_dir():
            self.skipTest(f"run evidence not present (archived in tag {rv.ARCHIVE_TAG}; set {rv.EVIDENCE_ENV})")
        result = rv.reconcile_all(root, use_git=False)
        self.assertFalse(any(r.get("missing") for r in result["runs"]))
        text = rv.redact(rv.render(result))
        self.assertNotRegex(text, r"/Users/[^~]")
        status = result["overall"][0]
        self.assertIn(status, ("PASS", "PASS_WITH_GAPS", "FAIL"))
        gaps = " ".join(result["overall"][2])
        self.assertIn("concurrent-children / claude: NOT_EXERCISED", gaps)
        self.assertIn("concurrent-children / codex: NOT_EXERCISED", gaps)


if __name__ == "__main__":
    unittest.main()
