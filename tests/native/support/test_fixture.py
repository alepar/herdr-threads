import json
import pathlib
import subprocess
import sys
import tempfile
import unittest

from fixture import NativeFixture, codex_argv, verify_context
from evidence import reconcile, validate_manifest


class FakeHost:
    def __init__(self):
        self.closed = []

    def close_exact_owned_id(self, kind, resource_id):
        self.closed.append((kind, resource_id))


class InterruptedHost(FakeHost):
    def close_exact_owned_id(self, kind, resource_id):
        super().close_exact_owned_id(kind, resource_id)
        raise RuntimeError("response lost")


class FixtureTests(unittest.TestCase):
    def test_cleanup_closes_only_recorded_ids_and_preserves_evidence(self):
        with tempfile.TemporaryDirectory() as root:
            fixture = NativeFixture.create(pathlib.Path(root) / "run")
            fixture.record_owned("pane", "w9:p2")
            fixture.record_owned("agent", "native-subject")
            fixture.write_evidence("interrupted.json", {"artifact_type": "fixture_event", "run_id": "run-1", "status": "FAIL", "reason": "interrupted"})
            host = FakeHost()
            fixture.cleanup(host)
            self.assertEqual(host.closed, [("agent", "native-subject"), ("pane", "w9:p2")])
            self.assertEqual(json.loads((fixture.evidence_dir / "interrupted.json").read_text())["status"], "FAIL")
            self.assertEqual((fixture.root / "ledger.jsonl").read_text().count("\n"), 6)
            self.assertEqual(fixture.root.stat().st_mode & 0o777, 0o700)

    def test_reopened_ledger_recovers_durable_prefix_after_torn_append(self):
        with tempfile.TemporaryDirectory() as root:
            path = pathlib.Path(root) / "run"
            fixture = NativeFixture.create(path)
            fixture.record_owned("pane", "w9:p2")
            with fixture.ledger_path.open("ab") as stream:
                stream.write(b'{"event":"owned","kind":"pane","id":"w9:p3"')
            host = FakeHost()
            NativeFixture.open(path).cleanup(host)
            self.assertEqual(host.closed, [("pane", "w9:p2")])
            NativeFixture.open(path).cleanup(host)
            self.assertEqual(host.closed, [("pane", "w9:p2")])
            self.assertEqual(len(list(path.glob("ledger.torn-*.bin"))), 1)
            self.assertIn(b'w9:p3', next(path.glob("ledger.torn-*.bin")).read_bytes())

    def test_middle_ledger_corruption_is_rejected(self):
        with tempfile.TemporaryDirectory() as root:
            path = pathlib.Path(root) / "run"
            fixture = NativeFixture.create(path)
            fixture.record_owned("pane", "w9:p2")
            with fixture.ledger_path.open("ab") as stream:
                stream.write(b'garbage\n{"event":"owned","kind":"pane","id":"w9:p3"')
            with self.assertRaises(ValueError):
                NativeFixture.open(path)
            self.assertFalse(list(path.glob("ledger.torn-*.bin")))

    def test_invalid_complete_transition_does_not_repair_tail(self):
        with tempfile.TemporaryDirectory() as root:
            path = pathlib.Path(root) / "run"
            fixture = NativeFixture.create(path)
            fixture.record_owned("pane", "w9:p2")
            with fixture.ledger_path.open("ab") as stream:
                stream.write(b'{"event":"closed","kind":"pane","id":"w9:p2"}\npartial')
            with self.assertRaises(ValueError):
                NativeFixture.open(path)
            self.assertFalse(list(path.glob("ledger.torn-*.bin")))

    def test_complete_non_object_ledger_row_is_corruption(self):
        with tempfile.TemporaryDirectory() as root:
            path = pathlib.Path(root) / "run"
            fixture = NativeFixture.create(path)
            fixture.record_owned("pane", "w9:p2")
            with fixture.ledger_path.open("ab") as stream:
                stream.write(b'["kind","id","event"]\npartial')
            with self.assertRaises(ValueError):
                NativeFixture.open(path)
            self.assertFalse(list(path.glob("ledger.torn-*.bin")))

    def test_ambiguous_close_is_not_replayed(self):
        with tempfile.TemporaryDirectory() as root:
            path = pathlib.Path(root) / "run"
            fixture = NativeFixture.create(path)
            fixture.record_owned("pane", "w9:p2")
            with self.assertRaises(RuntimeError):
                fixture.cleanup(InterruptedHost())
            host = FakeHost()
            NativeFixture.open(path).cleanup(host)
            self.assertEqual(host.closed, [])
            self.assertIn('"event": "close_intent"', fixture.ledger_path.read_text())

    def test_receipt_requires_model_call_and_database_record(self):
        base = {"schema_version": 1, "run_id": "run-1", "status": "FAIL", "reason": "missing_receipt", "scenario": "synthetic",
                "source": "synthetic", "accepted_pairs": [{"message_id": "m1", "recipient_id": "s1"}],
                "model_calls": [], "db_receipts": [], "transport_hints": [{"message_id": "m1", "recipient_id": "s1"}],
                "retired_pairs": [], "artifacts": []}
        validate_manifest(base)
        self.assertEqual(reconcile(base)["pending"], [{"message_id": "m1", "recipient_id": "s1"}])
        base["status"] = "PASS"
        base["reason"] = ""
        with self.assertRaises(ValueError):
            validate_manifest(base)
        base["source"] = "native"
        base["model_calls"] = [{"call_id": "call-1", "message_id": "m1", "recipient_id": "s1", "actor": "native-root", "artifact": "calls.jsonl"}]
        base["db_receipts"] = [{"call_id": "call-1", "message_id": "m1", "recipient_id": "s1", "actor": "native-root", "receipt_id": "r1", "artifact": "sqlite.json"}]
        base["artifacts"] = ["calls.jsonl", "sqlite.json"]
        validate_manifest(base)
        self.assertEqual(reconcile(base)["acked"], [{"message_id": "m1", "recipient_id": "s1"}])
        base["model_calls"][0]["artifact"] = "missing.jsonl"
        with self.assertRaises(ValueError):
            validate_manifest(base)

    def test_cooperative_source_may_pass_but_synthetic_and_unknown_may_not(self):
        # Kills: dropping "cooperative" from SOURCES/LIVE_SOURCES (a live cooperative run could never PASS), and
        # widening PASS to any source (a synthetic dry-run or an unlabelled source could claim PASS).
        manifest = {"schema_version": 1, "run_id": "run-1", "status": "PASS", "reason": "", "scenario": "prelaunch",
                    "source": "cooperative", "accepted_pairs": [{"message_id": "m1", "recipient_id": "s1"}],
                    "model_calls": [{"call_id": "c1", "message_id": "m1", "recipient_id": "s1", "actor": "root", "artifact": "calls.json"}],
                    "db_receipts": [{"call_id": "c1", "message_id": "m1", "recipient_id": "s1", "actor": "root", "receipt_id": "r1", "artifact": "db.json"}],
                    "transport_hints": [], "retired_pairs": [], "artifacts": ["calls.json", "db.json"]}
        validate_manifest(manifest)
        for source in ("synthetic", "native-ish"):
            with self.assertRaises(ValueError):
                validate_manifest({**manifest, "source": source})

    def test_launch_and_context_are_scoped(self):
        with tempfile.TemporaryDirectory() as root:
            fixture = NativeFixture.create(pathlib.Path(root) / "run")
            argv = codex_argv("subject", "w9:p2", ["--model", "test"])
            self.assertEqual(argv[:9], ["herdr", "agent", "start", "subject", "--kind", "codex", "--pane", "w9:p2", "--"])
            self.assertEqual(argv[9:], ["--no-daemon", "--model", "test"])
            self.assertEqual(fixture.session_env()["HCOM_DIR"], str(fixture.root / "hcom"))
            self.assertEqual(fixture.session_env()["HCOM_AUTO_APPROVE"], "0")
            self.assertTrue(verify_context({"HERDR_ENV": "1", "HERDR_WORKSPACE_ID": "w9", "HERDR_TAB_ID": "w9:t1", "HERDR_PANE_ID": "w9:p2"}, "w9:p2"))
            self.assertFalse(verify_context({"HERDR_ENV": "1", "HERDR_WORKSPACE_ID": "w9", "HERDR_TAB_ID": "w9:t1", "HERDR_PANE_ID": "w4:p1"}, "w9:p2"))

    def test_cli_context_exits_nonzero_on_wrong_inherited_identity(self):
        script = pathlib.Path(__file__).resolve().parents[3] / "scripts" / "native-fixture.py"
        process = subprocess.run([sys.executable, script, "verify-context", "--pane", "w9:p2"],
                                 capture_output=True, text=True, env={"HERDR_ENV": "1", "HERDR_WORKSPACE_ID": "w4", "HERDR_TAB_ID": "w4:t1", "HERDR_PANE_ID": "w4:p1"})
        self.assertEqual(process.returncode, 2)
        self.assertEqual(json.loads(process.stdout)["status"], "FAIL")

    def test_empty_native_pass_cannot_satisfy_receipt_gate(self):
        manifest = {"schema_version": 1, "run_id": "run-1", "status": "PASS", "reason": "", "scenario": "prelaunch",
                    "source": "native", "accepted_pairs": [], "model_calls": [], "db_receipts": [],
                    "transport_hints": [], "retired_pairs": [], "artifacts": []}
        with self.assertRaises(ValueError):
            validate_manifest(manifest)

    def test_rejects_duplicate_sqlite_receipt_for_one_pair(self):
        manifest = {"schema_version": 1, "run_id": "run-1", "status": "PASS", "reason": "", "scenario": "prelaunch",
                    "source": "native", "accepted_pairs": [{"message_id": "m1", "recipient_id": "s1"}],
                    "model_calls": [{"call_id": "c1", "message_id": "m1", "recipient_id": "s1", "actor": "root", "artifact": "calls.jsonl"}],
                    "db_receipts": [{"call_id": "c1", "message_id": "m1", "recipient_id": "s1", "actor": "root", "receipt_id": "r1", "artifact": "db.json"},
                                    {"call_id": "c1", "message_id": "m1", "recipient_id": "s1", "actor": "root", "receipt_id": "r2", "artifact": "db.json"}],
                    "transport_hints": [], "retired_pairs": [], "artifacts": ["calls.jsonl", "db.json"]}
        with self.assertRaises(ValueError):
            validate_manifest(manifest)

    def test_retirement_only_cannot_pass_receipt_gate(self):
        manifest = {"schema_version": 1, "run_id": "run-1", "status": "PASS", "reason": "", "scenario": "retired",
                    "source": "native", "accepted_pairs": [{"message_id": "m1", "recipient_id": "s1"}],
                    "model_calls": [], "db_receipts": [], "transport_hints": [],
                    "retired_pairs": [{"message_id": "m1", "recipient_id": "s1"}], "artifacts": []}
        with self.assertRaises(ValueError):
            validate_manifest(manifest)

    def test_evidence_writer_rejects_secret_extra_field(self):
        with tempfile.TemporaryDirectory() as root:
            fixture = NativeFixture.create(pathlib.Path(root) / "run")
            with self.assertRaises(ValueError):
                fixture.write_evidence("bad.json", {"artifact_type": "fixture_event", "run_id": "run-1",
                                                    "status": "FAIL", "reason": "interrupted", "secret": "credential"})
            self.assertFalse((fixture.evidence_dir / "bad.json").exists())
            manifest = {"schema_version": 1, "run_id": "run-1", "status": "FAIL", "reason": "missing_receipt", "scenario": "prelaunch",
                        "source": "synthetic", "accepted_pairs": [], "model_calls": [], "db_receipts": [],
                        "transport_hints": [], "retired_pairs": [], "artifacts": [], "secret": "credential"}
            with self.assertRaises(ValueError):
                fixture.write_evidence("bad-manifest.json", manifest)
            self.assertFalse((fixture.evidence_dir / "bad-manifest.json").exists())
            with self.assertRaises(ValueError):
                fixture.write_evidence("bad-type.json", {"artifact_type": "fixture_event", "run_id": "run-1",
                                                         "status": "FAIL", "reason": ["interrupted"]})


if __name__ == "__main__":
    unittest.main()
