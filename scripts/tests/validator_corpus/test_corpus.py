"""Negative corpus for the native demo validator (ht-p03.18, spec B8 Decision 1).

One synthetic fixture per known false-PASS / false-FAIL path of scripts/validate-native-demo.py. Each fixture is a
directory `fixtures/<id>/` holding the minimal inputs the code path reads and an `expected.json`:

    {"finding": "<id>", "entry": "<adapter below>", "verdict": "PASS|FAIL|UNSUPPORTED|NOT_EXERCISED|UNVERIFIED",
     "reason": "<exact reason string or null>"}

`case.json` (optional) describes the seat's SQLite rows, driver facts, recorded steps and pane captures; the other files
are the transcripts the entry reads (`transcript.jsonl`, `subagents/*.jsonl` for Claude children, `codex_home/` for
Codex child rollouts). The string `{tmp}` in an expected `reason` stands for the run's scratch directory.

Run from the repository root:  python3 -m unittest discover -s scripts/tests/validator_corpus

Each test builds a real-schema SQLite database from migrations/ in a temp dir; no daemon, Herdr or model is involved.
"""

import argparse
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import re
import shutil
import sqlite3
import tempfile
import unittest
from unittest import mock

REPO = Path(__file__).resolve().parents[3]
FIXTURES = Path(__file__).resolve().parent / "fixtures"
spec = importlib.util.spec_from_file_location("demo", REPO / "scripts" / "validate-native-demo.py")
demo = importlib.util.module_from_spec(spec)
spec.loader.exec_module(demo)

SEAT, COORD, THREAD, SESSION = "seat-a", "seat-c", "thread-1", "11111111-2222-3333-4444-555555555555"


def driver_args(root, **over):
    base = dict(harness="claude", dry_run=False, mode="print", run_root=str(root), state_dir=None, bin="/bin/true",
                host_endpoint="/nonexistent.sock", verify_wait=0, cli_hint=False, keep_panes=False, tui_accept_trust=False,
                claude_projects_dir=str(Path(root) / "claude-projects"), timeout=0, phases="initial",
                codex_profile=None, aisw_codex_root=str(Path(root) / "aisw-codex"), codex_model="gpt-6-luna",
                codex_effort="low", codex_sandbox="workspace-write", codex_config=[], codex_transport="setup",
                claude_model="claude-haiku-4-5-20251001", claude_permission_mode="default", claude_budget_usd=0.2,
                scenario="base", burst_threads=22, warning_deadline=5, launch="manual", launch_argv=demo.MANAGED_LAUNCH_ARGV,
                claude_json=str(Path(root) / "claude.json"), deadline=900)
    base.update(over)
    return argparse.Namespace(**base)


class Case:
    """One fixture, loaded into a driver over a real-schema database."""

    def __init__(self, test, name):
        self.dir = FIXTURES / name
        self.tmp = tempfile.TemporaryDirectory()
        test.addCleanup(self.tmp.cleanup)
        root = Path(self.tmp.name)
        self.expected = json.loads(self.read("expected.json"))
        self.case = json.loads(self.read("case.json")) if (self.dir / "case.json").exists() else {}
        harness = self.case.get("harness", "claude")
        self.driver = demo.Driver(driver_args(root, harness=harness, mode=self.case.get("mode", "print"),
                                              scenario=self.case.get("scenario", "base")))
        test.addCleanup(self.driver.cmdlog.close)
        self.driver.project.mkdir(exist_ok=True)
        self.db = None
        self.transcript = None
        self.harness = harness
        facts = {"agent_seat": SEAT, "thread": THREAD, "messages": {"initial": "msg-1"}, "sessions": {"initial": SESSION}}
        facts.update(self.case.get("facts", {}))
        self.driver.facts.update(facts)
        if harness == "codex":  # never read the real ~/.codex: a scratch CODEX_HOME, empty unless the fixture has one
            (root / "codex-home").mkdir()
            self.driver.facts["codex_home"] = str(root / "codex-home")
        self.seed_db(self.case.get("db", {}))
        test.addCleanup(self.close_db)
        self.place_files(root)
        for sid, status in self.case.get("steps", []):
            self.driver.record(sid, sid, status, "fixture")
        for name, text in self.case.get("panes", {}).items():
            (self.driver.ev / f"pane-{name}.txt").write_text(text)
        for result in self.case.get("phase_results", []):
            self.driver.phase_results.append(result)
        observation = self.driver.facts.get("lostprompt")
        if observation and observation.get("wake_after") == "@wake_row":
            observation["wake_after"] = self.driver.wake_row()

    def read(self, name):
        return (self.dir / name).read_text()

    def close_db(self):
        if self.db:
            self.db.close()

    def place_files(self, root):
        """Copy the fixture's transcripts to where the driver looks for them."""
        if (self.dir / "transcript.jsonl").exists():
            self.transcript = root / "transcript.jsonl"
            shutil.copy(self.dir / "transcript.jsonl", self.transcript)
            self.driver.facts.setdefault("phase_transcripts", {})["initial"] = str(self.transcript)
        if (self.dir / "subagents").is_dir():
            base = Path(self.driver.args.claude_projects_dir) / re.sub(r"[^A-Za-z0-9]", "-", str(self.driver.project)) / SESSION
            shutil.copytree(self.dir / "subagents", base / "subagents")
        if (self.dir / "codex_home").is_dir():
            shutil.copytree(self.dir / "codex_home", root / "codex-home", dirs_exist_ok=True)
        if self.harness == "codex":
            self.driver.facts.setdefault("phase_started_utc", {})["initial"] = "2000-01-01T00:00:00+00:00"

    def seed_db(self, spec):
        instance = self.driver.state / "instances" / "i"
        instance.mkdir(parents=True)
        self.db = sqlite3.connect(instance / "threads.sqlite3")
        for migration in sorted((REPO / "migrations").glob("*.sql")):
            self.db.executescript(migration.read_text())
        # The integrity triggers guard the production writers; the fixture seeds only the rows the read-only verifier
        # joins, so drop them rather than fabricate a whole send pipeline.
        for (name,) in self.db.execute("SELECT name FROM sqlite_master WHERE type='trigger'").fetchall():
            self.db.execute(f'DROP TRIGGER "{name}"')
        for seat in (SEAT, COORD):
            self.db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES (?,?,?,?,?,?)",
                            (seat, "i", "resolved", "operator_fresh", 1, 0))
        observation = json.dumps({"harness": self.case.get("harness", "claude"), "session": SESSION,
                                  "provenance": "cooperative_top_level", "execution": "exec-root"})
        for message in spec.get("messages", []):
            self.db.execute("INSERT INTO send_manifests VALUES (?,?, 'i', ?, ?, 0, 3, 1, 1, 0)",
                            (f"prep-{message['id']}", message["id"], THREAD, message["seq"]))
            self.db.execute("INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,"
                            "eligible_at_snapshot) VALUES (?, ?, ?, 1, 900000, 0)", (f"prep-{message['id']}", THREAD, SEAT))
            if message.get("acked"):
                self.db.execute("INSERT INTO receipt_state(message_id,seat_id,state,ack_actor_seat_id,ack_generation,ack_observation,"
                                "acked_at) VALUES (?,?,'acked',?,1,?,1)", (message["id"], SEAT, SEAT, observation))
        for invitation in spec.get("invitations", []):
            accepted = invitation["state"] == "accepted"
            self.db.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,"
                            "deadline_at,accepted_at,accepted_actor_seat_id,accepted_generation,accepted_observation) "
                            "VALUES (?,?,?,1,?,?,0,900000,1,?,?,?,?)",
                            (invitation["id"], THREAD, SEAT, invitation["state"], invitation["seq"], 1 if accepted else None,
                             SEAT if accepted else None, 1 if accepted else None, observation if accepted else None))
        if "wake_work" in spec:
            row = {"seat_id": SEAT, "reason_bits": 2, **spec["wake_work"]}
            for kind in ("invitation", "receipt", "warning"):
                if row.get(f"last_{kind}_seq") is not None:
                    row[f"last_{kind}_offset"] = 0
            self.db.execute(f"INSERT INTO wake_work({','.join(row)}) VALUES ({','.join('?' * len(row))})", list(row.values()))
        self.db.commit()

    def phase_result_from_verify(self):
        return self.driver.s_wait_and_verify("initial", self.transcript)


# ---- entry adapters: (case) -> (verdict, reason) ----------------------------------------------------------------

def entry_verify(case):
    """S18: the model-ACK verification of the initial phase."""
    status, detail, _ = case.phase_result_from_verify()
    return status, detail


def entry_child_check(case):
    """S18C: child/subagent ACK absence after the S18 verification joined SQLite with the transcript."""
    case.phase_result_from_verify()
    status, detail, _ = case.driver.s_child_check("initial")
    return status, detail


def entry_lostprompt_wake(case):
    """SL1: the reserved idle recovery wake for the un-prompted agent."""
    status, detail, _ = case.driver.s_lostprompt_wake()
    return status, detail


def entry_children_concurrent(case):
    """SK1: two concurrent subagents that each read mail, over the sidechain the driver rebuilt from the fixture."""
    driver = case.driver
    sidechain = driver.child_sidechain("initial", case.transcript)
    driver.phase_results.append({"phase": "initial", "sidechain": sidechain})
    status, detail, _ = driver.s_children_concurrent("initial")
    return status, detail


def entry_child_no_ack(case):
    """SK2/SC: no successful child write, from the complete sidechain the driver rebuilt from the fixture."""
    case.phase_result_from_verify()
    status, detail, _ = case.driver.s_child_no_ack("initial")
    return status, detail


def entry_midturn_ack(case):
    """SM3: the model ACKed the mid-turn message."""
    status, detail, _ = case.driver.s_midturn_ack()
    return status, detail


def entry_gate(case):
    """A scenario judge's FAIL passed through the instruction gate."""
    status, detail, _ = case.driver.gate_on_instruction(case.case["scenario"], (demo.FAIL, "judge failed", None))
    return status, detail


def entry_manifest(case):
    """The manifest the recorded steps and phase results produce: (status, manifest reason)."""
    status = case.driver.manifest()
    return status, json.loads((case.driver.ev / "manifest.json").read_text())["reason"]


def entry_lostprompt_launch_manifest(case):
    """W10-E3: the lostprompt launch (S17) run against the pane the fixture shows, then the manifest it produces."""
    driver = case.driver
    screen = (driver.ev / "pane-initial.txt").read_text()
    driver.safe_prompt_state = lambda label: "supported"
    driver.wake_row = lambda: {}
    driver.receipts_for = lambda seat: []
    driver.capture_pane = lambda name: screen
    driver.tui_transcript = lambda phase: None
    with contextlib.redirect_stdout(io.StringIO()), mock.patch.object(demo.time, "sleep"):
        driver.step("S17", "launch", lambda: driver.lostprompt_wait("initial"), needs=())
    return entry_manifest(case)


def entry_codex_tui_evidence(case):
    """W10-E5: what the interactive Codex launch records about the user config it loads and the hook-trust bypass."""
    driver = case.driver
    driver.codex_tui_command("initial", "", "", driver.ev / "x.rc")
    recorded = driver.facts.get("codex_tui") or {}
    if not recorded:
        return demo.FAIL, "no codex_tui evidence recorded"
    return demo.PASS, (f"user_config_path={recorded.get('user_config_path') and Path(recorded['user_config_path']).name} "
                       f"hook_trust_bypass={recorded.get('hook_trust_bypass')}")


ENTRIES = {name[len("entry_"):]: fn for name, fn in globals().items() if name.startswith("entry_")}


class CorpusTests(unittest.TestCase):
    def check(self, name):
        with contextlib.redirect_stdout(io.StringIO()):  # the driver prints every recorded step
            case = Case(self, name)
            expected = case.expected
            self.assertEqual(expected["finding"], name)
            verdict, reason = ENTRIES[expected["entry"]](case)
        expected_reason = expected["reason"].replace("{tmp}", case.tmp.name) if expected["reason"] is not None else None
        self.assertEqual(verdict, expected["verdict"], f"{name}: verdict (reason {reason!r})")
        self.assertEqual(reason, expected_reason, f"{name}: reason")


def _register(name):
    setattr(CorpusTests, f"test_{name}", lambda self, name=name: self.check(name))


for _path in sorted(FIXTURES.iterdir()) if FIXTURES.is_dir() else []:
    if (_path / "expected.json").exists():
        _register(_path.name)


class CorpusCoverage(unittest.TestCase):
    """The brief's fixture ids all exist: a deleted fixture is a failing test, not a silently smaller corpus."""
    REQUIRED = ("p15_nested_claude_p", "p15_nested_codex_exec", "p31_env_wrapper", "p31_sudo_exec_command_nohup_timeout",
                "p31_sh_c_and_backticks_and_dollar_paren", "w10_e1_sl1_missing_seq", "w10_e6_spawn_events_vs_children",
                "w10_e7_one_child_two_reads", "w6_d3_sm2_delivery_counts_as_seen", "w6_d4_tui_without_transcript_is_unverified",
                "w10_e2_offered_at_checkin_not_exercised", "w10_e3_usage_limit_screen_not_product_fail",
                "w10_e5_tui_user_config_recorded", "w10_e8_not_exercised_only_lostprompt", "fixnow_quoted_literal_documented")

    def test_every_required_fixture_is_present(self):
        for name in self.REQUIRED:
            self.assertTrue((FIXTURES / name / "expected.json").exists(), name)
            self.assertTrue(hasattr(CorpusTests, f"test_{name}"), name)


if __name__ == "__main__":
    unittest.main()
