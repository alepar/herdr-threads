"""Pure SQLite fixtures for the queue evidence reader; no agents or credentials."""
import copy
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest
from unittest import mock

from wake_e2e_queue import codex_turn_delivery, eligible_scope, queue_activity_intervals, queue_proof, queue_snapshot

SESSION = "00000000-0000-0000-0000-000000000001"
OWNER = "00000000-0000-0000-0000-000000000002"
SCOPE = {"binding": "binding", "native_session": SESSION, "execution": "execution", "generation": 1,
         "process": {"pid": 20, "start": 99}, "kernel_boot": "kernel"}
HELPER = {"id": "00000000-0000-0000-0000-000000000003", "purpose": "add", "subject": None,
          "owner_boot": OWNER, "kernel_boot": "kernel", "witness": {
              "process": {"pid": 30, "start": 100}, "kernel_boot": "kernel", "pgid": 30}}


class QueueEvidence(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="ht-queue-evidence-selftest.")
        self.addCleanup(self.tmp.cleanup)
        self.state = Path(self.tmp.name)
        (self.state / "node").mkdir()
        self.path = self.state / "node" / "herdr-threads.db"
        self.db = sqlite3.connect(self.path)
        self.addCleanup(self.db.close)
        self.db.executescript("""
            CREATE TABLE seats(id TEXT,state TEXT,generation INTEGER,target_id TEXT,target_generation INTEGER,instance_id TEXT);
            CREATE TABLE node_kernel_boot(id INTEGER,boot TEXT);
            CREATE TABLE occupant_bindings(ordinal INTEGER PRIMARY KEY,binding_id TEXT,seat_id TEXT,harness TEXT,native_session TEXT,execution_id TEXT,generation INTEGER,liveness_pid INTEGER,liveness_start INTEGER,liveness_boot TEXT,liveness TEXT,role TEXT,agent_id TEXT,registered_at INTEGER,observation_provenance TEXT,target_id TEXT,target_generation INTEGER,ended_at INTEGER);
            CREATE TABLE native_turn_state(binding_id TEXT,instance_id TEXT,scope_json BLOB,turn_id TEXT,phase TEXT,observation_ordinal INTEGER);
            CREATE TABLE wake_work(seat_id TEXT,reservation_id TEXT,reservation_boot TEXT,last_reservation_id TEXT,last_reservation_boot TEXT,last_outcome TEXT,reserved_at_utc INTEGER,last_reserved_at_utc INTEGER,completed_at_utc INTEGER,reason_bits INTEGER,retry_step INTEGER);
            CREATE TABLE wake_intents(ordinal INTEGER PRIMARY KEY,instance_id TEXT,attempt TEXT,adapter TEXT,origin_seat TEXT,target_json BLOB,phase TEXT,revision INTEGER,owner_boot TEXT,arm_nonce TEXT,arm_native_id TEXT,arm_native_ordinal INTEGER,logical_status TEXT,fulfillment_turn TEXT,fulfillment_ordinal INTEGER,cleanup_quarantined INTEGER,cleanup_list_attempted INTEGER,native_no_add_exception INTEGER,helper_json BLOB,ids_complete INTEGER,created_at INTEGER,updated_at INTEGER,descriptor BLOB,diagnostic TEXT);
            CREATE TABLE wake_intent_ids(intent_ordinal INTEGER,queue_id TEXT,cleanup_attempted INTEGER);
            INSERT INTO seats VALUES('seat','resolved',1,'pane',1,'instance');
            INSERT INTO node_kernel_boot VALUES(1,'kernel');
            INSERT INTO occupant_bindings VALUES(1,'binding','seat','codex','00000000-0000-0000-0000-000000000001','execution',1,20,99,'kernel','connected','top_level','',1,'cooperative_top_level','pane',1,NULL);
        """)
        self.db.execute("INSERT INTO native_turn_state VALUES('binding','instance',?,'baseline','ended',2)",
                        (json.dumps(SCOPE).encode(),))
        self.db.commit()

    def snapshot(self, after=0):
        return queue_snapshot(self.state, "seat", after)

    def add_intent(self, phase="accepted", witnessed=True, ordinal=1, helper=None):
        helper = copy.deepcopy(HELPER if helper is None else helper)
        if not witnessed:
            helper["witness"] = None
        self.db.execute("INSERT INTO wake_intents VALUES(?, 'instance', ?, 'codex', 'seat', ?, ?, 4, ?, 'nonce', 'baseline', 2, 'active', NULL, NULL, 0, 0, 0, ?, 0, 100, 110, ?, ?)",
                        (ordinal, f"attempt-{ordinal}", json.dumps(SCOPE).encode(), phase, OWNER,
                         json.dumps(helper).encode(), b"SECRET-DESCRIPTOR /real/config/auth.json", "SECRET-DIAGNOSTIC"))
        if phase == "accepted":
            self.db.execute("INSERT INTO wake_intent_ids VALUES(?, 'owned-queue-id', 0)", (ordinal,))
        self.db.commit()

    def test_queue_activity_uses_conservative_observation_bounds(self):
        def sample(start, end, phase, logical):
            return {"observation_start_ms": start, "observation_end_ms": end,
                    "intents": [{"attempt": "a", "phase": phase, "logical_status": logical,
                                 "updated_at": 12345}]}
        prefix = [sample(2000, 2010, "prepared", "active"), sample(8000, 8010, "accepted", "active")]
        passed = queue_activity_intervals(prefix + [sample(14000, 14010, "accepted", "fulfilled")], {"a"})
        self.assertEqual(passed[0]["accepted_to_activity_upper_ms"], 12010)
        self.assertEqual(passed[0]["status"], "passed")
        ambiguous = queue_activity_intervals(prefix + [sample(19000, 19010, "accepted", "fulfilled")], {"a"})
        self.assertEqual(ambiguous[0]["status"], "ambiguous")
        failed = queue_activity_intervals(prefix + [sample(30000, 30010, "accepted", "active"),
                                                  sample(31000, 31010, "accepted", "fulfilled")], {"a"})
        self.assertEqual(failed[0]["status"], "failed")
        self.assertEqual(queue_activity_intervals(prefix, {"a"})[0]["status"], "ambiguous")

    def test_projection_is_read_only_and_does_not_export_paths_or_descriptors(self):
        self.add_intent()
        before_changes = self.db.total_changes
        before_version = self.db.execute("PRAGMA data_version").fetchone()[0]
        snapshot = self.snapshot()
        self.assertTrue(snapshot["ok"])
        self.assertEqual(self.db.total_changes, before_changes)
        self.assertEqual(self.db.execute("PRAGMA data_version").fetchone()[0], before_version)
        text = json.dumps(snapshot)
        for forbidden in ("SECRET", "auth.json", "descriptor", "diagnostic"):
            self.assertNotIn(forbidden, text)
        self.assertEqual(snapshot["watermark"], 1)
        self.assertEqual(snapshot["intents"][0]["ids"], [{"id": "owned-queue-id", "cleanup_attempted": False}])

    def test_missing_database_or_schema_fails_closed_without_creation(self):
        missing = self.state / "missing"
        self.assertFalse(queue_snapshot(missing, "seat")["ok"])
        self.assertFalse(missing.exists())
        self.db.execute("DROP TABLE native_turn_state")
        self.db.commit()
        self.assertFalse(self.snapshot()["ok"])

    def test_matched_bootstrap_and_exact_scope_are_required(self):
        before = self.snapshot()
        qualified = eligible_scope(before)
        self.assertIsNotNone(qualified)
        self.assertEqual(qualified["scope"], SCOPE)
        for phase in ("end_first", "unknown", "started"):
            self.db.execute("UPDATE native_turn_state SET phase=?", (phase,))
            self.db.commit()
            self.assertIsNone(eligible_scope(self.snapshot()))
            if phase == "started":
                self.assertEqual(eligible_scope(self.snapshot(), "started")["scope"], SCOPE)
                stale = self.snapshot()
                stale["native"][0]["scope"]["execution"] = "old-execution"
                self.assertIsNone(eligible_scope(stale, "started"))
                disconnected = self.snapshot()
                disconnected["bindings"][0]["connected"] = False
                self.assertIsNone(eligible_scope(disconnected, "started"))
        self.db.execute("UPDATE native_turn_state SET phase='ended',scope_json=?",
                        (json.dumps(dict(SCOPE, execution="sibling")).encode(),))
        self.db.commit()
        self.assertIsNone(eligible_scope(self.snapshot()))

    def test_silent_or_known_busy_interval_has_no_possible_effect(self):
        for phase in ("ended", "started"):
            self.db.execute("UPDATE native_turn_state SET phase=?", (phase,))
            self.db.commit()
            before = self.snapshot()
            self.assertTrue(queue_proof(before, [self.snapshot()], "none")["passed"])
        self.add_intent("unknown")
        self.assertFalse(queue_proof(before, [self.snapshot()], "none")["passed"])

    def test_accepted_requires_nonce_baseline_owned_id_and_durable_add_witness(self):
        before = self.snapshot()
        self.add_intent(witnessed=False)
        self.assertFalse(queue_proof(before, [self.snapshot()], "accepted")["passed"])
        self.db.execute("UPDATE wake_intents SET helper_json=?", (json.dumps(HELPER).encode(),))
        self.db.commit()
        accepted = self.snapshot()
        self.assertTrue(queue_proof(before, [accepted], "accepted")["passed"])
        for field, value in (("arm_nonce", None), ("arm_native_id", "other-turn"), ("arm_native_ordinal", 1)):
            altered = copy.deepcopy(accepted)
            if field == "arm_nonce":
                altered["intents"][0][field] = value
            else:
                altered["intents"][0]["baseline"]["turn_id" if field == "arm_native_id" else "ordinal"] = value
            self.assertFalse(queue_proof(before, [altered], "accepted")["passed"])
        altered = copy.deepcopy(accepted)
        altered["intents"][0]["ids"] = []
        self.assertFalse(queue_proof(before, [altered], "accepted")["passed"])

    def test_unknown_never_passes_positive_or_native(self):
        before = self.snapshot()
        self.add_intent("unknown")
        for expectation in ("accepted", "none", "native"):
            self.assertFalse(queue_proof(before, [self.snapshot()], expectation)["passed"])

    def test_earlier_add_witness_survives_later_cleanup_and_native_start(self):
        before = self.snapshot()
        self.add_intent("armed")
        add = self.snapshot()
        cleanup = copy.deepcopy(HELPER)
        cleanup.update(id="00000000-0000-0000-0000-000000000004", purpose="list")
        self.db.execute("UPDATE wake_intents SET phase='accepted',helper_json=?,logical_status='fulfilled',fulfillment_turn='next-turn',fulfillment_ordinal=3", (json.dumps(cleanup).encode(),))
        self.db.execute("INSERT INTO wake_intent_ids VALUES(1,'owned-queue-id',1)")
        self.db.execute("UPDATE native_turn_state SET turn_id='next-turn',phase='started',observation_ordinal=3")
        self.db.commit()
        later = self.snapshot()
        self.assertFalse(queue_proof(before, [later], "accepted")["passed"])
        proof = queue_proof(before, [add, later], "accepted")
        self.assertTrue(proof["passed"])
        self.assertIn("compositional", proof["transport_inference"])

    def test_plain_or_multiple_actual_bindings_support_native_inference(self):
        qualified = self.snapshot()
        self.assertFalse(queue_proof(qualified, [qualified], "native")["passed"])
        self.db.execute("UPDATE occupant_bindings SET registered_at=NULL,observation_provenance='plain_cli'")
        self.db.commit()
        plain = self.snapshot()
        self.assertTrue(queue_proof(plain, [plain], "native")["passed"])
        self.db.execute("UPDATE occupant_bindings SET registered_at=1,observation_provenance='cooperative_top_level'")
        self.db.execute("INSERT INTO occupant_bindings SELECT 2,'sibling',seat_id,harness,native_session,'sibling-execution',generation,liveness_pid,liveness_start,liveness_boot,liveness,role,agent_id,registered_at,observation_provenance,target_id,target_generation,ended_at FROM occupant_bindings WHERE ordinal=1")
        self.db.commit()
        multiple = self.snapshot()
        self.assertIsNone(eligible_scope(multiple))
        self.assertTrue(queue_proof(multiple, [multiple], "native")["passed"])

    def test_incomplete_census_and_intent_windows_are_explicit(self):
        for n in range(2, 66):
            self.db.execute("INSERT INTO occupant_bindings SELECT ?,?,seat_id,harness,native_session,execution_id,generation,liveness_pid,liveness_start,liveness_boot,liveness,role,agent_id,registered_at,observation_provenance,target_id,target_generation,ended_at FROM occupant_bindings WHERE ordinal=1", (n, f"binding-{n}"))
        self.db.commit()
        census = self.snapshot()
        self.assertTrue(census["truncated"])
        self.assertFalse(census["census_complete"])
        self.assertFalse(queue_proof(census, [census], "native")["passed"])
        self.db.execute("DELETE FROM occupant_bindings WHERE ordinal>1")
        self.db.commit()
        for n in range(1, 34):
            self.add_intent(ordinal=n)
        intents = self.snapshot()
        self.assertEqual(len(intents["intents"]), 32)
        self.assertTrue(intents["truncated"])
        self.assertFalse(queue_proof(intents, [intents], "none")["passed"])

    def test_old_intents_or_wrong_witness_owner_cannot_prove_this_case(self):
        self.add_intent()
        before = self.snapshot()
        self.assertFalse(queue_proof(before, [self.snapshot()], "accepted")["passed"])
        before["watermark"] = 0
        wrong = copy.deepcopy(HELPER)
        wrong["owner_boot"] = "other-owner"
        self.db.execute("UPDATE wake_intents SET helper_json=?", (json.dumps(wrong).encode(),))
        self.db.commit()
        self.assertFalse(queue_proof(before, [self.snapshot()], "accepted")["passed"])

    def test_incidental_unknown_does_not_hide_behind_another_accepted_attempt(self):
        before = self.snapshot()
        self.add_intent()
        self.add_intent("unknown", ordinal=2)
        self.assertFalse(queue_proof(before, [self.snapshot()], "accepted")["passed"])

    def test_cross_seat_or_skipped_interval_samples_fail_closed(self):
        before = self.snapshot()
        for key, value in (("seat", "another-seat"), ("after", 100)):
            altered = self.snapshot()
            altered[key] = value
            self.assertFalse(queue_proof(before, [altered], "none")["passed"])
        self.assertFalse(queue_proof(before, [self.snapshot()] * 129, "none")["passed"])

    def test_owned_id_and_total_byte_limits_are_explicit(self):
        self.add_intent("armed")
        self.db.executemany("INSERT INTO wake_intent_ids VALUES(1,?,0)", [(f"owned-{n}",) for n in range(129)])
        self.db.commit()
        bounded = self.snapshot()
        self.assertTrue(bounded["truncated"])
        self.assertEqual(len(bounded["intents"][0]["ids"]), 128)
        self.db.execute("DELETE FROM wake_intent_ids")
        for n in range(2, 6):
            self.add_intent("armed", ordinal=n)
        for n in range(1, 6):
            self.db.executemany("INSERT INTO wake_intent_ids VALUES(?,?,0)", [(n, f"{n}-{k}-" + "x" * 120) for k in range(128)])
        self.db.commit()
        capped = self.snapshot()
        self.assertFalse(capped["ok"])
        self.assertTrue(capped["truncated"])
        self.assertLessEqual(len(json.dumps(capped).encode()), 65536)

    def test_malformed_metadata_is_not_exported_as_diagnostic_text(self):
        self.add_intent()
        self.db.execute("UPDATE wake_intents SET helper_json=?", (b"SECRET invalid JSON",))
        self.db.commit()
        bad = self.snapshot()
        self.assertFalse(bad["ok"])
        self.assertNotIn("SECRET", json.dumps(bad))

    def test_snapshot_stays_consistent_across_a_concurrent_wal_commit(self):
        self.db.execute("PRAGMA journal_mode=WAL")
        connect = sqlite3.connect
        fixture = self

        class CommitBetweenReads(sqlite3.Connection):
            def execute(self, sql, parameters=()):
                result = super().execute(sql, parameters)
                if sql.startswith("SELECT binding_id,"):
                    fixture.add_intent()
                    fixture.db.execute("UPDATE native_turn_state SET phase='started'")
                    fixture.db.commit()
                return result

        def reader(*args, **kwargs):
            return connect(*args, **kwargs, factory=CommitBetweenReads)

        with mock.patch("wake_e2e_queue.sqlite3.connect", side_effect=reader):
            snapshot = self.snapshot()
        self.assertTrue(snapshot["ok"])
        self.assertEqual(snapshot["watermark"], 0)
        self.assertEqual(snapshot["intents"], [])
        self.assertEqual(snapshot["native"][0]["phase"], "ended")
        self.assertEqual(self.db.execute("SELECT phase FROM native_turn_state").fetchone()[0], "started")
        self.assertEqual(self.db.execute("SELECT count(*) FROM wake_intents").fetchone()[0], 1)

    def test_wake_work_projects_only_attempts_outcome_and_numbers(self):
        self.db.execute("INSERT INTO wake_work VALUES('seat',NULL,NULL,'attempt-1',?,'submitted',NULL,100,110,1,1)", (OWNER,))
        self.db.commit()
        work = self.snapshot()["wake_work"]
        self.assertEqual(work, {"active_attempt": None, "active_boot": None, "last_attempt": "attempt-1",
                               "last_boot": OWNER, "last_outcome": "submitted", "reserved_at_utc": None,
                               "last_reserved_at_utc": 100, "completed_at_utc": 110, "reason_bits": 1, "retry_step": 1})

    def test_absent_process_or_native_metadata_is_not_bootstrap(self):
        self.db.execute("DELETE FROM native_turn_state")
        self.db.commit()
        missing_native = self.snapshot()
        self.assertTrue(missing_native["ok"])
        self.assertIsNone(eligible_scope(missing_native))
        self.db.execute("UPDATE occupant_bindings SET liveness_pid=NULL,liveness_start=NULL,liveness_boot=NULL")
        self.db.commit()
        missing_process = self.snapshot()
        self.assertTrue(missing_process["ok"])
        self.assertIsNone(missing_process["bindings"][0]["scope"])
        self.assertIsNone(eligible_scope(missing_process))


if __name__ == "__main__":
    unittest.main()


class TargetTurnDelivery(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="ht-target-turn-selftest.")
        self.addCleanup(self.tmp.cleanup)
        self.home = Path(self.tmp.name)
        self.path = self.home / "sessions" / "rollout-session.jsonl"
        self.path.parent.mkdir()
        self.bodies = {"mONE": "body one", "mTWO": "body two"}

    def write(self, turn="turn", output=None, developer=True, complete=True, developer_text=None):
        text = "RECEIVED mONE: body one\nRECEIVED mTWO: body two" if output is None else output
        def row(kind, payload):
            return {"timestamp": "2026-11-15T00:00:00Z", "type": kind, "payload": payload}
        rows = [row("event_msg", {"type": "task_started", "turn_id": turn})]
        if developer:
            rows.append(row("response_item", {"type": "message", "role": "developer", "content": [
                {"text": "[herdr-threads] message mONE in t1 from s1:\n  body one\n[herdr-threads] message mTWO in t1 from s1:\n  body two" if developer_text is None else developer_text}]}))
        rows.append(row("response_item", {"type": "message", "role": "assistant", "content": [{"text": text}]}))
        if complete:
            rows.append(row("event_msg", {"type": "task_complete", "turn_id": turn, "last_agent_message": text}))
        self.path.write_text("".join(json.dumps(r) + "\n" for r in rows))

    def read(self):
        return codex_turn_delivery(self.home, "session", "turn", self.bodies)

    def test_matching_target_turn_requires_context_and_completed_both_bodies(self):
        self.write()
        out = self.read()
        self.assertTrue(out["passed"], out)
        self.assertEqual(out["input_ids"], ["mONE", "mTWO"])
        self.assertEqual(out["output_ids"], ["mONE", "mTWO"])
        self.assertNotIn("body one", json.dumps(out))
        self.assertNotIn(str(self.home), json.dumps(out))
        compact = ("message mONE t1#1 from s1 bytes 0..8/8: topic\n  body one\n"
                   "message mTWO t1#2 from s1 bytes 0..8/8: topic\n  body two")
        self.write(developer_text="Hook context\npeer_data: " + json.dumps(compact))
        self.assertTrue(self.read()["passed"])

    def test_wrong_session_or_turn_is_not_delivery(self):
        self.write(turn="other")
        self.assertFalse(self.read()["passed"])
        self.write()
        self.path.rename(self.path.with_name("rollout-other.jsonl"))
        self.assertFalse(self.read()["passed"])
        self.path.with_name("rollout-other.jsonl").rename(self.path.with_name("rollout-sessionX.jsonl"))
        self.assertFalse(self.read()["passed"])

    def test_missing_context_body_or_completion_fails(self):
        for options in ({"developer": False}, {"complete": False},
                        {"output": "RECEIVED mONE: body one"},
                        {"output": "RECEIVED mONEX: body one\nRECEIVED mTWOX: body two"},
                        {"output": "RECEIVED mONE: body two\nRECEIVED mTWO: body one"}):
            with self.subTest(options=options):
                self.write(**options)
                self.assertFalse(self.read()["passed"])

    def test_swapped_developer_associations_do_not_cross_message_frames(self):
        for text in ("mONE: body two; mTWO: body one",
                     "[herdr-threads] message mONE in t1 from s1:\n  body two\n"
                     "[herdr-threads] message mTWO in t1 from s1:\n  body one"):
            with self.subTest(text=text):
                self.write(developer_text=text)
                self.assertFalse(self.read()["passed"])

    def test_received_body_cannot_cross_another_message_id_on_same_line(self):
        self.write(output="RECEIVED mONE mTWO: body two; body one")
        self.assertFalse(self.read()["passed"])

    def test_source_derived_formatter_positive_preserves_multiline_bodies(self):
        # These are production formatter goldens, not a recreated live capture.
        import wake_e2e_queue as Q
        root = Path(__file__).resolve().parents[1] / "tests" / "fixtures" / "delivery"
        text = (root / "frame-multiline.txt").read_text()
        items = json.loads((root / "frame-multiline.json").read_text())
        self.assertTrue(items)
        import re
        expected = {item["id"]: "\n".join(re.split(r"\r\n|[\n\r\v\f\x85\u2028\u2029]", item["body"])).rstrip("\n")
                    for item in items}
        self.assertEqual(Q.message_bodies(text), expected)
        wrapped = "Hook context\npeer_data: " + json.dumps(text) + "\nMore inbox context."
        self.assertEqual(Q.message_bodies(wrapped), expected)

    def test_file_and_byte_bounds_fail_closed(self):
        self.write()
        (self.path.parent / "other.jsonl").write_text("{}\n")
        with mock.patch("wake_e2e_queue.ROLLOUT_FILE_LIMIT", 1):
            self.assertFalse(self.read()["passed"])
        with mock.patch("wake_e2e_queue.ROLLOUT_BYTE_LIMIT", 100):
            out = self.read()
            self.assertFalse(out["passed"])
            self.assertLessEqual(out["bytes_read"], 100)

    def test_malformed_timestamp_never_exports_private_data(self):
        self.write()
        self.path.write_text(self.path.read_text().replace("2026-11-15T00:00:00Z", "/private/profile/secret"))
        out = self.read()
        self.assertFalse(out["passed"])
        self.assertNotIn("/private/profile", json.dumps(out))
