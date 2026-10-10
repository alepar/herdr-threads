"""Offline checks that the demo driver's verifier never turns missing or non-model evidence into PASS.

Run: python3 -m unittest tests/native/demo/test_demo_verify.py
Builds a real schema from migrations/ in a temp dir; no daemon, Herdr or model is involved.
"""

import argparse
import contextlib
import importlib.util
import io
import json
import os
import re
from pathlib import Path
import signal
import sqlite3
import tempfile
import time
import unittest
from unittest import mock

REPO = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("demo", REPO / "scripts" / "validate-native-demo.py")
demo = importlib.util.module_from_spec(spec)
spec.loader.exec_module(demo)

# The platform scratch root the driver approves folder trust under: /private/tmp on macOS, /tmp on Linux.
SCRATCH_ROOT = demo.SCRATCH_ROOT


def non_scratch_dir():
    """A writable existing directory outside SCRATCH_ROOT, for tests that need a refusal to happen: the platform temp
    dir when it is not scratch (/var/folders on macOS), else /dev/shm, /var/tmp or the home directory. None if none."""
    for candidate in (tempfile.gettempdir(), "/dev/shm", "/var/tmp", str(Path.home())):
        path = Path(candidate).resolve()
        if (path.is_dir() and os.access(path, os.W_OK | os.X_OK)
                and str(path) != SCRATCH_ROOT and not str(path).startswith(SCRATCH_ROOT + "/")):
            return str(path)
    return None


SEAT, COORD, THREAD, MSG, SESSION = "seat-a", "seat-c", "thread-1", "msg-1", "11111111-2222-3333-4444-555555555555"


def args(root, **over):
    base = dict(harness="claude", dry_run=False, mode="print", run_root=str(root), state_dir=None, bin="/bin/true",
                host_endpoint="/nonexistent.sock", verify_wait=0, cli_hint=False, keep_panes=False, tui_accept_trust=False,
                claude_projects_dir=str(Path(root) / "claude-projects"), timeout=5, phases="initial",
                codex_profile=None, aisw_codex_root=str(Path(root) / "aisw-codex"), codex_model="gpt-6-luna",
                codex_effort="low", codex_sandbox="workspace-write", codex_config=[], codex_transport="setup",
                claude_model="claude-haiku-4-5-20251001", claude_permission_mode="default", claude_budget_usd=0.2,
                scenario="base", burst_threads=22, warning_deadline=5, launch="manual", launch_argv=demo.MANAGED_LAUNCH_ARGV,
                claude_json=str(Path(root) / "claude.json"), deadline=900, codex_bin=None)
    base.update(over)
    return argparse.Namespace(**base)


class DbFixture(unittest.TestCase):
    """A real-schema SQLite fixture plus a Claude stream transcript; no test methods of its own."""
    HARNESS = "claude"

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.driver = demo.Driver(args(self.tmp.name, harness=self.HARNESS))
        instance = self.driver.state / "instances" / "i"
        instance.mkdir(parents=True)
        self.db = sqlite3.connect(instance / "threads.sqlite3")
        for migration in sorted((REPO / "migrations").glob("*.sql")):
            self.db.executescript(migration.read_text())
        # Integrity triggers guard the production writers; this fixture seeds only the rows the
        # read-only verifier joins, so drop them rather than fabricate a whole send pipeline.
        for (name,) in self.db.execute("SELECT name FROM sqlite_master WHERE type='trigger'").fetchall():
            self.db.execute(f'DROP TRIGGER "{name}"')
        for seat in (SEAT, COORD):
            self.db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES (?,?,?,?,?,?)",
                            (seat, "i", "resolved", "operator_fresh", 1, 0))
        self.db.execute("INSERT INTO send_manifests VALUES ('prep-1',?, 'i', ?, 5, 0, 3, 1, 1, 0)", (MSG, THREAD))
        self.db.execute("INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) "
                        "VALUES ('prep-1', ?, ?, 1, 900000, 0)", (THREAD, SEAT))
        self.db.commit()
        self.driver.facts.update({"agent_seat": SEAT, "thread": THREAD, "messages": {"initial": MSG},
                                  "sessions": {"initial": SESSION}})
        self.transcript = Path(self.tmp.name) / "stream.jsonl"

    def tearDown(self):
        self.db.close()
        self.driver.cmdlog.close()
        self.tmp.cleanup()

    def settle(self, provenance="cooperative_top_level", actor=SEAT, accept=True, execution="exec-root"):
        observation = json.dumps({"harness": self.HARNESS, "session": SESSION, "provenance": provenance, "execution": execution})
        self.db.execute("INSERT INTO receipt_state(message_id,seat_id,state,ack_actor_seat_id,ack_generation,ack_observation,acked_at) "
                        "VALUES (?,?,'acked',?,1,?,1)", (MSG, SEAT, actor, observation))
        if accept:
            self.db.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at,"
                            "accepted_at,accepted_actor_seat_id,accepted_generation,accepted_observation) VALUES "
                            "('inv-1',?,?,1,'accepted',1,0,900000,1,1,?,1,?)", (THREAD, SEAT, actor, observation))
        self.db.commit()

    def say(self, command, child=False):
        event = {"type": "assistant", "parent_tool_use_id": "toolu_parent" if child else None,
                 "message": {"content": [{"type": "tool_use", "id": f"toolu_{len(command)}", "name": "Bash", "input": {"command": command}}]}}
        with self.transcript.open("a") as stream:
            stream.write(json.dumps(event) + "\n")

    def verify(self):
        return self.driver.s_wait_and_verify("initial", self.transcript)


class VerifyTests(DbFixture):
    def test_model_ack_with_matching_provenance_passes(self):
        self.settle()
        self.say(f"herdr-threads accept {THREAD}")
        self.say(f"herdr-threads ack {MSG}")
        status, detail, _ = self.verify()
        self.assertEqual(status, demo.PASS, detail)

    def test_pending_receipt_fails(self):
        self.say(f"herdr-threads ack {MSG}")
        status, detail, _ = self.verify()
        self.assertEqual(status, demo.FAIL)
        self.assertIn("state=pending", detail)

    def test_operator_or_service_provenance_fails(self):
        self.settle(provenance="operator")
        self.say(f"herdr-threads ack {MSG}")
        status, detail, _ = self.verify()
        self.assertEqual(status, demo.FAIL)
        self.assertIn("provenance", detail)

    def test_db_ack_without_model_call_fails(self):
        self.settle()
        status, detail, _ = self.verify()
        self.assertEqual(status, demo.FAIL)
        self.assertIn("no root", detail)

    def test_child_ack_fails(self):
        self.settle()
        self.say(f"herdr-threads ack {MSG}", child=True)
        status, detail, _ = self.verify()
        self.assertEqual(status, demo.FAIL)
        self.assertIn("child/subagent", detail)

    def test_missing_acceptance_fails(self):
        self.settle(accept=False)
        self.say(f"herdr-threads ack {MSG}")
        status, detail, _ = self.verify()
        self.assertEqual(status, demo.FAIL)
        self.assertIn("invitation not accepted", detail)


    def test_pass_detail_and_phase_result_label_cooperative_not_native(self):
        # Kills (S5): describing cooperative_top_level as native provenance, or dropping the recorded class.
        self.settle()
        self.say(f"herdr-threads accept {THREAD}")
        self.say(f"herdr-threads ack {MSG}")
        status, detail, _ = self.verify()
        self.assertEqual(status, demo.PASS, detail)
        self.assertIn("NOT native proof", detail)
        result = self.driver.phase_results[-1]
        self.assertEqual(result["provenance"]["ack"], "cooperative_top_level")
        self.assertEqual(result["provenance"]["accept"], "cooperative_top_level")
        self.assertEqual(result["provenance_label"], "cooperative")


class RecipeLineTests(unittest.TestCase):
    """The driver reads the doctor recipe line; it never hard-codes pins."""

    def test_interval_and_set(self):
        self.assertEqual(demo.recipe_admits("claude-hooks-2.1.283 [2.1.283, 2.1.286]", "2.1.286"), (True, True))
        self.assertEqual(demo.recipe_admits("claude-hooks-2.1.283 [2.1.283, 2.1.286]", "2.1.287"), (False, True))
        self.assertEqual(demo.recipe_admits("codex-hooks-v1 {0.157.1, 0.158.0}", "0.158.0"), (True, True))
        self.assertEqual(demo.recipe_admits("codex-hooks-v1 {0.157.1, 0.158.0}", "0.159.2"), (False, True))

    def test_unknown_form_is_unparsed_not_admitted(self):
        line = "codex-hooks-v1 {0.157.1, 0.158.0}; codex-fp-1 schema-fingerprint sha256:abcd"
        self.assertEqual(demo.recipe_admits(line, "0.159.2"), (False, False))
        self.assertEqual(demo.recipe_admits(None, "0.159.2"), (False, False))


class ManifestTests(unittest.TestCase):
    """D1/S5/--cli-hint: the manifest verdict and its provenance label."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.driver = demo.Driver(args(self.tmp.name))
        self.driver.facts.update({"agent_seat": SEAT, "messages": {"initial": MSG}})

    def tearDown(self):
        self.driver.cmdlog.close()
        self.tmp.cleanup()

    def passing_live_run(self, provenance="cooperative_top_level"):
        d = self.driver
        for sid in ("S01", "S02H", "S04", "S13", "S14"):
            d.record(sid, sid, demo.PASS, "ok")
        # Exactly how main() declares the dry-run-only steps; on a live run S15 is skipped by its own body.
        d.step("S15", "dry-only", lambda: (_ for _ in ()).throw(demo.Blocked("live")), needs=("S14",), dry_only=True)
        d.step("S16", "dry-only", lambda: (_ for _ in ()).throw(demo.Blocked("live")), needs=("S14",), dry_only=True)
        d.step("S16C", "dry-only", lambda: (demo.PASS, "unreachable", None), needs=("S15", "S13"), dry_only=True)
        for sid in ("S16P", "S17", "S17H", "S18", "S99"):
            d.record(sid, sid, demo.PASS, "ok")
        d.phase_results.append({"phase": "initial", "message": MSG, "root_ack_calls": [("toolu_1", f"herdr-threads ack {MSG}", False)],
                                "receipt": {"state": "acked"}, "provenance": {"ack": provenance, "accept": provenance}})

    def test_dry_only_steps_are_skipped_not_blocked_on_live_run(self):
        # Kills (D1): the S16C needs-check running before the dry-only short-circuit (S16C BLOCKED on S15).
        self.passing_live_run()
        self.assertEqual(self.driver.status_of("S16C"), demo.SKIP)
        self.assertNotIn(demo.BLOCKED, [s["status"] for s in self.driver.steps])

    def test_live_manifest_passes_when_every_live_step_passes_and_is_labelled_cooperative(self):
        # Kills (D1): a live manifest that can never PASS; (S5): `source: native` for cooperative_top_level.
        self.passing_live_run()
        self.assertEqual(self.driver.manifest(), "PASS")
        record = json.loads((self.driver.ev / "manifest.json").read_text())
        self.assertEqual(record["source"], "cooperative")
        self.assertEqual(self.driver.facts["provenance_classes"], ["cooperative_top_level"])

    def test_none_source_is_schema_valid_but_never_pass(self):
        # fix3 D5: `none` is a legal manifest source, and the evidence schema refuses it on a PASS.
        import evidence
        self.passing_live_run()
        self.assertEqual(self.driver.manifest(), "PASS")
        record = json.loads((self.driver.ev / "manifest.json").read_text())
        evidence.validate_manifest({**record, "status": "FAIL", "reason": "scenario_failed", "source": "none"})
        with self.assertRaises(ValueError):
            evidence.validate_manifest({**record, "source": "none"})

    def test_native_label_only_when_every_class_is_native_proof(self):
        # Kills (S5): labelling mixed/cooperative evidence native, or never labelling verified evidence native.
        self.passing_live_run(provenance="verified_current_target")
        self.assertEqual(self.driver.manifest(), "PASS")
        self.assertEqual(json.loads((self.driver.ev / "manifest.json").read_text())["source"], "native")
        self.assertEqual(demo.Driver.provenance_label(["verified_current_target", "cooperative_top_level"]), "cooperative")
        self.assertEqual(demo.Driver.provenance_label([]), "none")

    def test_cli_hint_run_is_never_acceptance(self):
        # Kills (--cli-hint): a hinted diagnostic run reporting manifest PASS.
        self.driver.args.cli_hint = True
        self.passing_live_run()
        self.assertEqual(self.driver.manifest(), "UNSUPPORTED")
        self.assertEqual(json.loads((self.driver.ev / "manifest.json").read_text())["reason"], "diagnostic_cli_hint")

    def test_unverified_child_check_makes_manifest_unsupported_not_pass(self):
        # Kills (B1, fix2): a run with subagent activity reporting manifest PASS.
        self.passing_live_run()
        self.driver.record("S18C", "child", demo.UNVERIFIED, "collab activity")
        self.assertEqual(self.driver.manifest(), "UNSUPPORTED")
        record = json.loads((self.driver.ev / "manifest.json").read_text())
        self.assertEqual(record["reason"], "child_ack_unverified")
        self.assertIn("S18C", self.driver.facts["unverified"])
        self.assertFalse((self.driver.ev / "manifest.invalid.json").exists())

    def test_unverified_phase_result_alone_blocks_pass(self):
        # Kills: relying on the step row only (e.g. S18C BLOCKED/missing) when the phase result says unverified.
        self.passing_live_run()
        self.driver.phase_results[-1]["child_check"] = "unverified_subagent_activity"
        self.assertEqual(self.driver.manifest(), "UNSUPPORTED")

    def test_failed_step_with_unverified_child_is_fail(self):
        self.passing_live_run()
        self.driver.record("S18C", "child", demo.UNVERIFIED, "collab activity")
        self.driver.record("S17H", "hook context", demo.FAIL, "not delivered")
        self.assertEqual(self.driver.manifest(), "FAIL")

    def test_no_subagent_activity_run_passes(self):
        self.passing_live_run()
        self.driver.phase_results[-1]["child_check"] = "no_subagent_activity"
        self.driver.record("S18C", "child", demo.PASS, "none")
        self.assertEqual(self.driver.manifest(), "PASS")
        self.assertEqual(self.driver.facts["unverified"], [])

    def test_any_failed_step_fails_manifest(self):
        self.passing_live_run()
        self.driver.record("S17H", "hook context", demo.FAIL, "not delivered")
        self.assertEqual(self.driver.manifest(), "FAIL")


class HookContextTests(unittest.TestCase):
    """D2: PreToolUse/SessionStart additionalContext read READ-ONLY from the Claude session transcript."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.driver = demo.Driver(args(self.tmp.name))
        self.driver.project.mkdir()
        self.driver.facts.update({"thread": THREAD, "messages": {"initial": MSG}, "sessions": {"initial": SESSION},
                                  "phase_started_utc": {"initial": "2026-09-30T06:44:00+00:00"}})
        encoded = "".join(c if c.isalnum() else "-" for c in str(self.driver.project))
        self.path = Path(self.driver.args.claude_projects_dir) / encoded / f"{SESSION}.jsonl"
        self.path.parent.mkdir(parents=True)

    def tearDown(self):
        self.driver.cmdlog.close()
        self.tmp.cleanup()

    def attach(self, event, text, stamp="2026-09-30T06:44:16.538Z", tool="toolu_1"):
        entry = {"type": "attachment", "timestamp": stamp, "isSidechain": False, "sessionId": SESSION,
                 "attachment": {"type": "hook_additional_context", "content": [text], "hookName": event,
                                "hookEvent": event.split(":")[0], "toolUseID": tool}}
        with self.path.open("a") as stream:
            stream.write(json.dumps(entry) + "\n")

    def test_session_start_and_pretooluse_delivery_recorded(self):
        # Kills (D2): not reading the transcript / not counting PreToolUse attachments / writing to it.
        self.attach("SessionStart", f"pending {MSG} in {THREAD}")
        self.attach("PreToolUse:Bash", f"still pending {MSG}", stamp="2026-09-30T06:44:23.383Z")
        before = (self.path.stat().st_mtime_ns, self.path.read_bytes())
        status, detail, _ = self.driver.s_hook_context("initial")
        self.assertEqual(status, demo.PASS, detail)
        self.assertIn("PreToolUse additionalContext delivered 1x", detail)
        self.assertEqual(self.driver.facts["hook_context"]["initial"]["pre_tool_use"], 1)
        self.assertEqual((self.path.stat().st_mtime_ns, self.path.read_bytes()), before)
        evidence = json.loads((self.driver.ev / "hook-context-initial.json").read_text())
        self.assertTrue(evidence["read_only"])
        self.assertEqual([d["hook_event"] for d in evidence["deliveries"]], ["SessionStart", "PreToolUse"])

    def test_entries_before_phase_start_do_not_count(self):
        # Kills (D2): a resumed session's earlier SessionStart being credited to the resume phase.
        self.attach("SessionStart", f"pending {MSG}", stamp="2026-09-30T06:40:00.000Z")
        status, detail, _ = self.driver.s_hook_context("initial")
        self.assertEqual(status, demo.FAIL, detail)

    def test_entries_after_phase_end_do_not_count(self):
        # Kills (D2): a later phase's SessionStart (same resumed session file) being credited to this phase.
        self.driver.facts["phase_ended_utc"] = {"initial": "2026-09-30T06:44:30+00:00"}
        self.attach("SessionStart", f"pending {MSG}", stamp="2026-09-30T06:44:42.996Z")
        status, detail, _ = self.driver.s_hook_context("initial")
        self.assertEqual(status, demo.FAIL, detail)

    def test_missing_transcript_is_unverified_not_pass(self):
        # Kills (D2): treating an absent transcript as delivered.
        self.path.parent.rmdir()
        status, detail, _ = self.driver.s_hook_context("initial")
        self.assertEqual(status, demo.FAIL)
        self.assertIn("UNVERIFIED", detail)


class CodexChildTests(VerifyTests):
    """B1: a Codex child's calls never reach the parent exec stream."""
    HARNESS = "codex"

    def setUp(self):
        super().setUp()
        self.driver.facts["bindings_before"] = {"initial": 0}
        for ordinal, (session, execution) in enumerate([(SESSION, "exec-root"), ("child-thread", "exec-child")], start=1):
            self.db.execute("INSERT INTO occupant_bindings(ordinal,seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,"
                            "execution_id,observation_provenance,observed_at,ended_at) VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
                            (ordinal, SEAT, ordinal, "w1:p1", "boot", 1, "codex", session, execution, "cooperative_top_level", 1,
                             1 if ordinal == 1 else None))
        self.db.commit()

    def event(self, item, kind="item.completed"):
        with self.transcript.open("a") as stream:
            stream.write(json.dumps({"type": kind, "item": item}) + "\n")

    def say(self, command, child=False):
        self.event({"id": f"item_{len(command)}", "type": "command_execution", "command": command, "exit_code": 0,
                    "status": "completed", "aggregated_output": ""})

    def spawn(self):
        self.event({"id": "item_9", "type": "collab_tool_call", "tool": "spawn_agent", "sender_thread_id": SESSION,
                    "receiver_thread_ids": ["child-thread"], "prompt": "do it", "agents_states": {}, "status": "completed"})

    def test_child_ack_fails(self):  # Codex child calls are invisible in the parent stream; see B1 tests below
        pass

    def verdict(self):
        status, detail, _ = self.verify()
        child = self.driver.s_child_check("initial")
        return status, detail, child

    def test_collab_call_with_same_execution_ack_is_never_pass(self):
        # Kills (B1, fix2): the vacuous proof. Under the cooperative model a child's ACK carries the caller's claim,
        # i.e. the root execution; an ACK with that execution plus a collab call must not PASS the child check.
        self.settle(execution="exec-root")
        self.spawn()
        self.say(f"herdr-threads accept {THREAD}")
        self.say(f"herdr-threads ack {MSG}")
        status, detail, (child_status, child_detail, _) = self.verdict()
        self.assertEqual(status, demo.PASS, detail)  # everything the DB and root transcript can show holds
        self.assertEqual(child_status, demo.UNVERIFIED, child_detail)
        self.assertIn("cooperative limit", child_detail)
        result = self.driver.phase_results[-1]
        self.assertEqual(result["child_check"], "unverified_subagent_activity")
        self.assertNotIn("root_execution_proven", json.dumps(result))

    def test_collab_call_with_other_execution_is_unverified(self):
        self.settle(execution="exec-child")
        self.spawn()
        self.say(f"herdr-threads accept {THREAD}")
        self.say(f"herdr-threads ack {MSG}")
        _, _, (child_status, _, _) = self.verdict()
        self.assertEqual(child_status, demo.UNVERIFIED)

    def test_collab_item_started_only_is_activity(self):
        # Kills: counting only completed collab items (a child still running at exit).
        self.settle()
        self.event({"id": "item_9", "type": "collab_tool_call", "tool": "spawn_agent", "status": "in_progress"}, kind="item.started")
        self.say(f"herdr-threads accept {THREAD}")
        self.say(f"herdr-threads ack {MSG}")
        _, _, (child_status, _, _) = self.verdict()
        self.assertEqual(child_status, demo.UNVERIFIED)

    def test_no_collab_call_passes_child_check(self):
        self.settle()
        self.say(f"herdr-threads accept {THREAD}")
        self.say(f"herdr-threads ack {MSG}")
        status, detail, (child_status, child_detail, _) = self.verdict()
        self.assertEqual(status, demo.PASS, detail)
        self.assertEqual(child_status, demo.PASS, child_detail)
        self.assertEqual(self.driver.phase_results[-1]["child_check"], "no_subagent_activity")

    def test_root_binding_execution_is_diagnostic_only(self):
        # The binding lookup is kept as a diagnostic; a binding from before this launch is not this phase's root.
        self.driver.facts["bindings_before"] = {"initial": 1}
        self.settle()
        self.say(f"herdr-threads accept {THREAD}")
        self.say(f"herdr-threads ack {MSG}")
        self.verify()
        self.assertIsNone(self.driver.phase_results[-1]["root_binding_execution_diagnostic"])


class ClaudeSubagentTests(VerifyTests):
    """B1 for Claude: any Task/Agent tool use or sidechain event makes child ACK absence UNVERIFIED."""

    def tool(self, name, child=False):
        event = {"type": "assistant", "parent_tool_use_id": "toolu_parent" if child else None,
                 "message": {"content": [{"type": "tool_use", "id": "toolu_task", "name": name, "input": {"prompt": "x"}}]}}
        with self.transcript.open("a") as stream:
            stream.write(json.dumps(event) + "\n")

    def run_case(self):
        self.settle()
        self.say(f"herdr-threads accept {THREAD}")
        self.say(f"herdr-threads ack {MSG}")
        status, detail, _ = self.verify()
        return status, detail, self.driver.s_child_check("initial")[0]

    def test_task_tool_use_is_unverified(self):
        self.tool("Task")
        status, detail, child = self.run_case()
        self.assertEqual(status, demo.PASS, detail)
        self.assertEqual(child, demo.UNVERIFIED)

    def test_agent_tool_use_is_unverified(self):
        self.tool("Agent")
        self.assertEqual(self.run_case()[2], demo.UNVERIFIED)

    def test_sidechain_event_without_mutation_is_unverified(self):
        self.tool("Read", child=True)
        self.assertEqual(self.run_case()[2], demo.UNVERIFIED)

    def test_plain_root_run_passes_child_check(self):
        self.tool("Read")
        self.assertEqual(self.run_case()[2], demo.PASS)

    def test_tui_without_transcript_is_unverified(self):
        self.driver.phase_results.append({"phase": "initial", "child_check": "unverified_no_transcript", "child_agent_calls": []})
        self.assertEqual(self.driver.s_child_check("initial")[0], demo.UNVERIFIED)


class ArgsTests(unittest.TestCase):
    """S1/S2: argument gates, checked before any resource is created."""

    def parse(self, *argv):
        with contextlib.redirect_stderr(io.StringIO()):
            return demo.parse(["--bin", "/bin/true", *argv])

    def rejected(self, *argv):
        with self.assertRaises(SystemExit):
            self.parse(*argv)

    def test_tui_requires_trust_flag(self):
        # Kills (S1): a TUI run that can type into the trust dialog without explicit opt-in.
        self.rejected("--harness", "claude", "--mode", "tui", "--allow-uncapped-spend")
        self.parse("--harness", "claude", "--mode", "tui", "--tui-accept-trust", "--allow-uncapped-spend")

    def test_uncapped_live_runs_require_explicit_flag(self):
        # Kills (S2): Codex / Claude TUI live spend without opt-in; the dry-run and capped print mode stay free.
        self.rejected("--harness", "codex")
        self.rejected("--harness", "claude", "--mode", "tui", "--tui-accept-trust")
        self.parse("--harness", "codex", "--allow-uncapped-spend")
        self.parse("--harness", "codex", "--dry-run")
        self.parse("--harness", "claude")

    def test_timeout_times_phases_is_bounded(self):
        # Kills (S2): an unbounded --timeout or --timeout x phases.
        self.rejected("--harness", "claude", "--timeout", str(demo.MAX_LAUNCH_TIMEOUT_S + 1), "--phases", "initial")
        self.rejected("--harness", "claude", "--timeout", "600", "--phases", "initial,restart,resume,clear")
        self.rejected("--harness", "claude", "--timeout", "0")
        self.parse("--harness", "claude", "--timeout", "600", "--phases", "initial,restart,resume")


class FakeHostDriver:
    """Stubs the process helpers so cleanup/TUI logic runs without Herdr or a daemon."""

    def __init__(self, test, **over):
        self.tmp = tempfile.TemporaryDirectory(dir=over.pop("tmp_dir", None))
        test.addCleanup(self.tmp.cleanup)
        self.driver = demo.Driver(args(self.tmp.name, **over))
        test.addCleanup(self.driver.cmdlog.close)
        self.calls = []
        self.health_rc, self.tab_list_rc, self.tab_list_out, self.screens = 3, 0, "[]", []
        self.driver.herdr = self.herdr
        self.driver.ht = self.ht

    def herdr(self, *argv, tag, timeout=30):
        self.calls.append(("herdr", tag, argv))
        if tag == "cleanup:tab-list":
            return self.tab_list_rc, None, self.tab_list_out, "boom" if self.tab_list_rc else ""
        if tag.startswith("herdr:pane-read"):
            return 0, None, self.screens.pop(0) if self.screens else "", ""
        if tag.startswith("tui:wait-ready"):
            if getattr(self, "not_found_waits", 0) > 0:
                self.not_found_waits -= 1
                return 1, None, "", '{"error":{"code":"agent_not_found","message":"agent target w9:p1 not found"}}'
            return 0, {"status": self.wait_state}, "", ""
        return 0, None, "", ""

    def ht(self, *argv, tag, **kw):
        self.calls.append(("ht", tag, argv))
        return (self.health_rc if tag == "cleanup:health-after" else 0), "", ""


class CleanupTests(unittest.TestCase):
    """S4: S99 must observe the owned daemon stopped and the owned tab gone."""

    def owned(self, **over):
        host = FakeHostDriver(self, **over)
        d = host.driver
        d.daemon_owned = True
        d.fixture.record_owned("daemon", str(d.state))
        d.fixture.record_owned("pane", "tab:w9:t7")
        d.facts.update({"tab": "w9:t7", "workspace": "w9"})
        return host

    def test_clean_shutdown_passes(self):
        host = self.owned()
        host.driver.cleanup()
        self.assertEqual(host.driver.status_of("S99"), demo.PASS)

    def test_owned_daemon_still_healthy_fails(self):
        # Kills (S4): ignoring the health-after rc of an owned daemon.
        host = self.owned()
        host.health_rc = 0
        host.driver.cleanup()
        self.assertEqual(host.driver.status_of("S99"), demo.FAIL)
        self.assertIn("still healthy", host.driver.steps[-1]["detail"])

    def test_tab_list_failure_fails(self):
        # Kills (S4): an empty stdout from a failed `tab list` passing the tab check.
        host = self.owned()
        host.tab_list_rc = 1
        host.driver.cleanup()
        self.assertEqual(host.driver.status_of("S99"), demo.FAIL)
        self.assertIn("UNVERIFIED", host.driver.steps[-1]["detail"])

    def test_tab_still_listed_fails(self):
        host = self.owned()
        host.tab_list_out = '[{"tab_id": "w9:t7"}]'
        host.driver.cleanup()
        self.assertEqual(host.driver.status_of("S99"), demo.FAIL)


class InterruptTests(unittest.TestCase):
    """S3: every exit path runs cleanup once and still writes the summary."""

    def run_with(self, error):
        host = FakeHostDriver(self)
        d = host.driver
        d.fixture.record_owned("pane", "tab:w9:t7")
        d.facts.update({"tab": "w9:t7", "workspace": "w9"})

        def boom():
            d.record("S01", "preflight", demo.PASS, "ok")
            raise error
        d.run_steps = boom
        with contextlib.redirect_stdout(io.StringIO()):
            rc = d.main()
        return host, rc

    def test_keyboard_interrupt_runs_cleanup(self):
        # Kills (S3): cleanup outside try/finally (ctrl+c during the launch wait leaves the tab and agent open).
        host, rc = self.run_with(KeyboardInterrupt())
        self.assertEqual(rc, 130)
        self.assertIn(("herdr", "cleanup:tab-close", ("tab", "close", "w9:t7")), host.calls)
        self.assertEqual(host.driver.status_of("S98"), demo.FAIL)
        self.assertTrue((host.driver.ev / "summary.json").exists())

    def test_sigterm_runs_cleanup(self):
        # Kills (S3): SIGTERM not handled (default action kills the driver before cleanup).
        host = FakeHostDriver(self)
        d = host.driver
        d.fixture.record_owned("pane", "tab:w9:t7")
        d.facts.update({"tab": "w9:t7", "workspace": "w9"})
        d.run_steps = lambda: os.kill(os.getpid(), signal.SIGTERM) or time.sleep(5)
        with contextlib.redirect_stdout(io.StringIO()):
            rc = d.main()
        self.assertEqual(rc, 130)
        self.assertIn("SIGTERM", host.driver.interrupted)
        self.assertEqual(sum(1 for c in host.calls if c[1] == "cleanup:tab-close"), 1)
        self.assertIs(signal.getsignal(signal.SIGTERM), signal.SIG_DFL)


class TuiTrustTests(unittest.TestCase):
    """S1: Enter is sent only when the trust dialog is actually on screen."""

    def launch(self, state, screens):
        host = FakeHostDriver(self, mode="tui", tui_accept_trust=True)
        host.wait_state, host.screens = state, list(screens)
        host.driver.facts["agent_pane"] = "w9:p1"
        with contextlib.redirect_stdout(io.StringIO()):
            result = host.driver.launch_tui("initial", "true", host.driver.ev / "x.rc")
        return host, result

    def enters(self, host):
        return [c for c in host.calls if c[1].startswith("tui:accept-trust")]

    def test_idle_state_with_trust_dialog_on_screen_answers_it(self):
        # Kills (S1): trusting Herdr's `blocked` state instead of the screen (idle -> prompt typed into the dialog).
        host, (status, detail, _) = self.launch("idle", ["Do you trust the files in this folder?", "Welcome", "? for shortcuts"])
        self.assertEqual(len(self.enters(host)), 1)
        prompt_index = next(i for i, c in enumerate(host.calls) if c[1].startswith("tui:prompt"))
        enter_index = next(i for i, c in enumerate(host.calls) if c[1].startswith("tui:accept-trust"))
        self.assertLess(enter_index, prompt_index)

    def test_wait_retries_until_herdr_detects_the_started_agent(self):
        # Kills: one-shot `agent wait` right after `pane run` (agent_not_found before Herdr detects the TUI) failing S17.
        host = FakeHostDriver(self, mode="tui", tui_accept_trust=True)
        host.wait_state, host.screens, host.not_found_waits = "idle", ["Welcome", "? for shortcuts"], 2
        host.driver.facts["agent_pane"] = "w9:p1"
        with contextlib.redirect_stdout(io.StringIO()), mock.patch.object(demo.time, "sleep"):
            status, detail, _ = host.driver.launch_tui("initial", "true", host.driver.ev / "x.rc")
        self.assertEqual(status, demo.PASS, detail)
        self.assertEqual(len([c for c in host.calls if c[1].startswith("tui:wait-ready")]), 3)

    def test_trust_dialog_defaulting_to_no_moves_to_yes_before_enter(self):
        # Kills: a bare Enter on Claude 2.1.286's dialog, whose cursor starts on "No, exit" (Claude exits).
        no = "Quick safety check\n ❯ No, exit\n   Yes, I trust this folder\n Enter to confirm"
        yes = "Quick safety check\n   No, exit\n ❯ Yes, I trust this folder\n Enter to confirm"
        host = FakeHostDriver(self, mode="tui", tui_accept_trust=True)
        host.wait_state, host.screens = "blocked", [no.replace("Quick safety check", "Do you trust this folder?"), yes, "Welcome", "? for shortcuts"]
        host.driver.facts["agent_pane"] = "w9:p1"
        with contextlib.redirect_stdout(io.StringIO()), mock.patch.object(demo.time, "sleep"):
            status, detail, _ = host.driver.launch_tui("initial", "true", host.driver.ev / "x.rc")
        tags = [c[1] for c in host.calls]
        down = tags.index("tui:trust-select-yes:initial")
        enter = tags.index("tui:accept-trust:initial")
        self.assertLess(down, enter)
        self.assertEqual(status, demo.PASS, detail)

    def test_trust_dialog_that_cannot_reach_yes_confirms_nothing(self):
        no = "Do you trust this folder?\n ❯ No, exit\n   Yes, I trust this folder"
        host = FakeHostDriver(self, mode="tui", tui_accept_trust=True)
        host.wait_state, host.screens = "blocked", [no, no]
        host.driver.facts["agent_pane"] = "w9:p1"
        with contextlib.redirect_stdout(io.StringIO()), mock.patch.object(demo.time, "sleep"):
            status, detail, _ = host.driver.launch_tui("initial", "true", host.driver.ev / "x.rc")
        self.assertEqual(status, demo.FAIL)
        self.assertNotIn("tui:accept-trust:initial", [c[1] for c in host.calls])

    def test_stalled_prompt_is_retried_once_after_the_input_box_is_drawn(self):
        # Kills: typing the prompt while Claude is still drawing its input box (agent_prompt_stalled, prompt lost).
        host = FakeHostDriver(self, mode="tui", tui_accept_trust=True)
        host.wait_state, host.screens = "idle", ["Welcome", "loading", "? for shortcuts", "done"]
        host.driver.facts["agent_pane"] = "w9:p1"
        prompts = []
        real = host.herdr
        def herdr(*argv, tag, timeout=30):
            if tag.startswith("tui:prompt"):
                prompts.append(tag)
                host.calls.append(("herdr", tag, argv))
                if len(prompts) == 1:
                    return 1, None, "", '{"error":{"code":"agent_prompt_stalled"}}'
                return 0, None, "", ""
            return real(*argv, tag=tag, timeout=timeout)
        host.driver.herdr = herdr
        with contextlib.redirect_stdout(io.StringIO()), mock.patch.object(demo.time, "sleep"):
            status, detail, _ = host.driver.launch_tui("initial", "true", host.driver.ev / "x.rc")
        self.assertEqual(prompts, ["tui:prompt:initial", "tui:prompt:initial:retry"])
        self.assertEqual(status, demo.PASS, detail)
        reads = [c[1] for c in host.calls if c[1].startswith("herdr:pane-read:initial-input")]
        self.assertEqual(len(reads), 2)  # polled until the input box footer appeared

    def test_blocked_by_other_dialog_sends_nothing(self):
        # Kills (S1): a bare Enter to whatever dialog is blocking.
        host, (status, detail, _) = self.launch("blocked", ["Choose a theme"])
        self.assertEqual(status, demo.FAIL)
        self.assertEqual(self.enters(host), [])
        self.assertFalse([c for c in host.calls if c[1].startswith("tui:prompt")])


class TuiUnrecognisedStartupScreenTests(unittest.TestCase):
    """Nothing is typed into a TUI whose input box never appeared (an unrecognised dialog Herdr reports idle)."""

    UPDATE = ("✨ Update available! 0.159.2 -> 0.160.0\n\n› 1. Update now (runs `brew upgrade codex`)\n  2. Skip\n"
              "  3. Skip until next version\n\n  Press enter to continue")
    MIGRATION = "Import your external agent configuration?\n› 1. Yes, migrate\n  2. No\n  Press enter to confirm"

    def launch(self, harness, screen, scenario="base"):
        host = FakeHostDriver(self, harness=harness, mode="tui", tui_accept_trust=True, scenario=scenario)
        host.wait_state, host.screens = "idle", [screen] * 40
        host.driver.facts["agent_pane"] = "w9:p1"
        clock = iter(range(0, 10_000, 5))
        with contextlib.redirect_stdout(io.StringIO()), mock.patch.object(demo.time, "sleep"), \
                mock.patch.object(demo.time, "monotonic", side_effect=lambda: next(clock)):
            status, detail, _ = host.driver.launch_tui("initial", "true", host.driver.ev / "x.rc")
        typed = [c[1] for c in host.calls if c[1].startswith(("tui:prompt", "tui:accept-trust", "tui:trust-select"))
                 or (c[0] == "herdr" and ("send-keys" in c[2] or "prompt" in c[2]))]
        return status, detail, typed

    def test_codex_update_screen_reported_idle_fails_with_nothing_typed(self):
        # Kills: typing the prompt + Enter into Codex's "Update available" screen after the 30 s input-box wait
        # timed out (Enter = "Update now": answers a dialog and modifies the aisw-managed install).
        status, detail, typed = self.launch("codex", self.UPDATE)
        self.assertEqual(status, demo.FAIL, detail)
        self.assertIn("nothing typed", detail)
        self.assertEqual(typed, [])

    def test_codex_migration_prompt_fails_with_nothing_typed(self):
        status, detail, typed = self.launch("codex", self.MIGRATION)
        self.assertEqual(status, demo.FAIL, detail)
        self.assertEqual(typed, [])

    def test_claude_without_input_box_fails_with_nothing_typed(self):
        status, detail, typed = self.launch("claude", "Choose the text style that looks best with your terminal")
        self.assertEqual(status, demo.FAIL, detail)
        self.assertEqual(typed, [])

    def test_lostprompt_without_input_box_fails_before_observing(self):
        # Kills: the lostprompt path proceeding to its wake observation although a dialog holds the screen.
        host = FakeHostDriver(self, harness="codex", mode="tui", scenario="lostprompt")
        host.wait_state, host.screens = "idle", [self.UPDATE] * 40
        host.driver.facts["agent_pane"] = "w9:p1"
        host.driver.lostprompt_wait = mock.Mock(side_effect=AssertionError("lostprompt_wait reached"))
        clock = iter(range(0, 10_000, 5))
        with contextlib.redirect_stdout(io.StringIO()), mock.patch.object(demo.time, "sleep"), \
                mock.patch.object(demo.time, "monotonic", side_effect=lambda: next(clock)):
            status, detail, _ = host.driver.launch_tui("initial", "true", host.driver.ev / "x.rc")
        self.assertEqual(status, demo.FAIL, detail)
        host.driver.lostprompt_wait.assert_not_called()

    def test_codex_input_box_present_still_prompts(self):
        status, detail, typed = self.launch("codex", "› Ask Codex to do anything\n  ? for shortcuts   100% context left")
        self.assertIn("tui:prompt:initial", typed)

    def test_codex_tui_disables_the_startup_update_check(self):
        host = FakeHostDriver(self, harness="codex", mode="tui")
        shell, _, _ = host.driver.codex_tui_command("initial", "", "", host.driver.ev / "x.rc")
        argv = host.driver.facts["native_argv"]["initial"]
        self.assertIn("check_for_update_on_startup=false", argv)
        self.assertEqual(argv[argv.index("check_for_update_on_startup=false") - 1], "-c")


class LostpromptBaselineTests(unittest.TestCase):
    """SL1's wake baseline is read before the agent starts, not after the startup waits."""

    def test_reservation_made_during_startup_waits_counts_as_new(self):
        # Kills: reading wake_before after `pane run` + ready/input waits, when the product already reserved the
        # idle recovery wake (SL1 then FAILs "no idle recovery wake reservation" although the wake happened).
        host = FakeHostDriver(self, harness="claude", mode="tui", tui_accept_trust=True, scenario="lostprompt")
        d = host.driver
        d.facts.update({"agent_seat": SEAT, "agent_pane": "w9:p1", "messages": {"initial": MSG}})
        reserved = {"seat_id": SEAT, "last_reservation_id": "r-1", "last_receipt_seq": 3, "last_outcome": "submitted",
                    "reason_bits": 1}
        rows = {"now": {}}
        d.wake_row = lambda: dict(rows["now"])
        d.query = lambda sql, params=(): [{"n": 0}]
        d.launch_command = lambda phase: ("true", d.ev / "x.rc", None)
        d.safe_prompt_state = lambda label: "supported"
        d.receipts_for = lambda seat: [{"message_id": MSG, "state": "acked"}]
        d.capture_pane = lambda name: ""
        d.tui_transcript = lambda phase: None
        def launch_tui(phase, shell, rcfile):
            rows["now"] = reserved  # the SessionStart check-in bound the seat; the wake was reserved at once
            return d.lostprompt_wait(phase)
        d.launch_tui = launch_tui
        clock = iter(range(0, 10_000, 5))
        with contextlib.redirect_stdout(io.StringIO()), mock.patch.object(demo.time, "sleep"), \
                mock.patch.object(demo.time, "monotonic", side_effect=lambda: next(clock)):
            d.s_launch("initial")
        self.assertEqual(d.facts["lostprompt"]["wake_before"], {})
        status, detail, _ = d.s_lostprompt_wake()
        self.assertEqual(status, demo.PASS, detail)


class SetupModeTests(unittest.TestCase):
    """The driver prefers the public `herdr-threads setup` CLI and labels its own install as a fallback. Setup always
    runs with HOME, CLAUDE_CONFIG_DIR and CODEX_HOME in the run root, so only scratch copies are written."""

    def driver(self, harness, present, output="", rc=0, allow=None):
        host = FakeHostDriver(self, harness=harness, setup="auto", setup_argv=None)
        d = host.driver
        d.project.mkdir()
        d.facts["cli_surface"] = {"setup": {"present": present}}
        def ht(*argv, tag, **kw):
            host.calls.append(("ht", tag, argv, kw))
            env = kw.get("extra_env") or {}
            if argv[:1] == ("setup",) and rc == 0:
                if harness == "claude":
                    target = Path(env["CLAUDE_CONFIG_DIR"]) / "settings.json"
                    target.parent.mkdir(exist_ok=True)
                    target.write_text(json.dumps({"permissions": {"allow": allow or [demo.HERDR_THREADS_ALLOW]}}))
                else:
                    target = Path(env["CODEX_HOME"]) / "hooks.json"
                    target.parent.mkdir(exist_ok=True)
                    target.write_text(json.dumps({"hooks": {"SessionStart": [{"hooks": [
                        {"type": "command", "command": "'x' 'hook' 'codex' # herdr-threads-owner:1"}]}]}}))
            return rc, output, ""
        d.ht = ht
        return host, d

    def test_claude_cli_setup_uses_public_argv_and_scratch_homes(self):
        host, d = self.driver("claude", True, json.dumps({"setup": {"action": "installed", "hook_argv": ["x"]}}))
        status, detail, _ = d.s_hooks()
        self.assertEqual(status, demo.PASS, detail)
        self.assertEqual(d.facts["setup_mode"], "cli")
        call = [c for c in host.calls if c[1] == "setup:cli"][0]
        self.assertEqual(list(call[2]), ["setup", "claude"])
        env = call[3]["extra_env"]
        for key in ("HOME", "CLAUDE_CONFIG_DIR", "CODEX_HOME"):
            self.assertTrue(env[key].startswith(str(d.root)), (key, env))
        settings = d.claude_config_dir() / "settings.json"
        self.assertEqual(d.facts["claude_settings"], str(settings))
        # The launch keeps the real Claude config dir (auth) and adds the scratch file as a flag layer.
        shell, _, _ = d.launch_command("initial")
        argv = d.facts["native_argv"]["initial"]
        self.assertEqual(argv[argv.index("--settings") + 1], str(settings))
        self.assertEqual(argv[argv.index("--setting-sources") + 1], "project,local")
        self.assertNotIn("CLAUDE_CONFIG_DIR", shell)

    def test_codex_cli_setup_writes_scratch_hooks_and_no_hook_arguments(self):
        report = {"setup": {"scope": "user", "sandbox": {"socket_path": None, "omitted": "unmeasured"}}}
        host, d = self.driver("codex", True, json.dumps(report))
        d.args.codex_transport = "none"
        status, detail, _ = d.s_hooks()
        self.assertEqual(status, demo.PASS, detail)
        argv = [c for c in host.calls if c[1] == "setup:cli"][0][2]
        self.assertEqual(list(argv), ["setup", "codex"])
        self.assertEqual(d.facts["codex_hook_args"], ["-c", "sandbox_workspace_write.network_access=false"])
        shell, _, _ = d.launch_command("initial")
        self.assertIn(f"CODEX_HOME={d.codex_setup_home()}", shell)
        self.assertNotIn("--ignore-user-config", shell)
        self.assertNotIn("hooks.", shell)

    def test_codex_cli_setup_without_owned_hooks_fails(self):
        _, d = self.driver("codex", True, json.dumps({"setup": {}}), rc=0)
        d.ht = lambda *argv, tag, **kw: (0, json.dumps({"setup": {}}), "")
        self.assertEqual(d.s_hooks()[0], demo.FAIL)

    def test_cli_refusal_fails_not_falls_back(self):
        _, d = self.driver("claude", True, "", rc=4)
        self.assertEqual(d.s_hooks()[0], demo.FAIL)

    def test_absent_cli_falls_back_with_label(self):
        for harness in ("claude", "codex"):
            _, d = self.driver(harness, False)
            status, detail, _ = d.s_hooks()
            self.assertEqual(status, demo.PASS, detail)
            self.assertEqual(d.facts["setup_mode"], "driver_fallback")
            self.assertIn("setup_mode=driver_fallback", detail)
            self.assertIn(str(d.root), detail)


SOCK = "/private/tmp/herdr-threads-501/0123456789abcdef.sock"
ALLOWANCE = ["-c", "sandbox_workspace_write.network_access=true", "-c", "features.network_proxy.enabled=true",
             "-c", 'features.network_proxy.unix_sockets={"%s"="allow"}' % SOCK]
KEYS = ["sandbox_workspace_write.network_access", "features.network_proxy.enabled",
        f"features.network_proxy.unix_sockets.{SOCK}"]


class CodexTransportSetupTests(unittest.TestCase):
    """D3/D5 (Codex demo 2): the launch runs under setup's scratch config.toml allowance for the driver's own daemon."""

    driver = SetupModeTests.driver

    def report(self, sandbox=True):
        report = {"scope": "user"}
        report["sandbox"] = ({"socket_path": SOCK, "keys": KEYS, "present": True, "note": "n"} if sandbox
                             else {"socket_path": None, "omitted": "unmeasured on Codex 0.158.0"})
        return json.dumps({"setup": report})

    def publish(self, d, endpoint=SOCK):
        instance = d.state / "instances" / "abc"
        instance.mkdir(parents=True)
        (instance / "endpoint.json").write_text(json.dumps({"endpoint": endpoint, "boot_id": "b"}))

    def test_default_setup_transport_launches_with_the_allowance(self):
        host, d = self.driver("codex", True, self.report())
        self.publish(d)
        status, detail, _ = d.s_hooks()
        self.assertEqual(status, demo.PASS, detail)
        self.assertEqual(d.facts["codex_hook_args"], [])
        self.assertEqual(d.facts["codex_transport"]["socket_path"], SOCK)
        self.assertTrue(d.transport_allowance())
        self.assertIn(f"allows only {SOCK}", detail)
        d.facts["agent_pane"] = "w1:p1"
        shell, _, _ = d.launch_command("initial")
        self.assertIn(f"CODEX_HOME={d.codex_setup_home()}", shell)

    def test_none_transport_turns_the_proxy_off(self):
        host, d = self.driver("codex", True, self.report())
        d.args.codex_transport = "none"
        status, detail, _ = d.s_hooks()
        self.assertEqual(status, demo.PASS, detail)
        self.assertEqual(d.facts["codex_hook_args"], ["-c", "sandbox_workspace_write.network_access=false"])
        self.assertFalse(d.transport_allowance())

    def test_allowance_for_another_socket_fails(self):
        host, d = self.driver("codex", True, self.report())
        self.publish(d, endpoint="/private/tmp/herdr-threads-501/ffffffffffffffff.sock")
        status, detail, _ = d.s_hooks()
        self.assertEqual(status, demo.FAIL)
        self.assertIn("would not reach it", detail)

    def test_missing_allowance_fails_under_setup_transport(self):
        _, d = self.driver("codex", True, self.report(sandbox=False))
        status, detail, _ = d.s_hooks()
        self.assertEqual(status, demo.FAIL)
        self.assertIn("unmeasured", detail)

    def test_setup_and_launch_never_use_the_profile_as_codex_home(self):
        # The profile supplies only credentials (a symlinked auth.json); setup writes the scratch home.
        calls = []
        host, d = self.driver("codex", True, self.report())
        profile = Path(d.args.aisw_codex_root) / "codex-1"
        profile.mkdir(parents=True)
        (profile / "auth.json").write_text("{}")
        d.facts["codex_profile"], d.facts["codex_profile_home"] = "codex-1", str(profile)
        inner = d.ht
        def ht(*argv, tag, **kw):
            calls.append(kw.get("extra_env"))
            return inner(*argv, tag=tag, **kw)
        d.ht = ht
        self.publish(d)
        self.assertEqual(d.s_hooks()[0], demo.PASS)
        self.assertEqual(calls[0]["CODEX_HOME"], str(d.codex_setup_home()))
        shell, _, _ = d.launch_command("initial")
        self.assertIn(f"CODEX_HOME={d.codex_setup_home()}", shell)
        self.assertNotIn(f"CODEX_HOME={profile}", shell)
        link = d.codex_setup_home() / "auth.json"
        self.assertTrue(link.is_symlink())
        self.assertEqual(os.readlink(link), str(profile / "auth.json"))
        self.assertEqual(sorted(p.name for p in profile.iterdir()), ["auth.json"])

    def test_profile_step_precedes_setup_and_titles_name_the_codex_rollout(self):
        # D5 order and D4 title, as run_steps declares them (every step body stubbed).
        host = FakeHostDriver(self, harness="codex", codex_profile="codex-1", dry_run=True)
        d = host.driver
        order = []
        d.step = lambda sid, title, fn, needs=(), dry_only=False: order.append((sid, title))
        d.run_steps()
        ids = [sid for sid, _ in order]
        self.assertLess(ids.index("S16A"), ids.index("S14"))
        self.assertLess(ids.index("S02Q"), ids.index("S14"))
        title = dict(order)["S17H"]
        self.assertIn("Codex session rollout", title)
        self.assertNotIn("Claude", title)


CODEX2_RUN1 = Path(__file__).resolve().parent / "evidence" / "codex-demo2-run1-initial.events.jsonl"


class CodexTransportDeniedTests(unittest.TestCase):
    """D2 (Codex demo 2 run 1): a sandbox-refused daemon socket is UNSUPPORTED transport_denied, never product FAIL."""

    HARNESS = "codex"
    tearDown = VerifyTests.tearDown

    def setUp(self):
        VerifyTests.setUp(self)
        self.health = []
        self.health_rc = 0
        def ht(*argv, tag, **kw):
            self.health.append(tag)
            return self.health_rc, "", ""
        self.driver.ht = ht
        self.driver.facts["codex_transport"] = {"mode": "none", "argv": []}

    def launch_and_verify(self, events):
        status, detail, _ = self.driver.judge_launch("initial", demo.PASS, "agent process exited 0", events)
        self.driver.record("S17", "launch", status, detail)
        verdict = self.driver.s_wait_and_verify("initial", events)
        self.driver.record("S18", "verify", verdict[0], verdict[1])
        return verdict

    def test_demo2_run1_is_transport_denied_not_fail(self):
        outcome = self.driver.transcript_outcome(CODEX2_RUN1)
        self.assertIn("host_unavailable", outcome["transport_suspect"]["evidence"])
        self.assertIsNone(outcome["transport_denied"])
        status, detail, _ = self.launch_and_verify(CODEX2_RUN1)
        self.assertEqual(status, demo.TRANSPORT, detail)
        self.assertIn("UNSUPPORTED transport_denied (inferred_host_unavailable_while_daemon_healthy)", detail)
        self.assertEqual(self.health, ["daemon:health-transport-check"])
        for sid in ("S01", "S13", "S14", "S16P", "S99"):
            self.driver.record(sid, sid, demo.PASS, "ok")
        self.driver.step("S18C", "child", lambda: (demo.PASS, "", None), needs=("S18",))
        self.assertEqual(self.driver.manifest(), "UNSUPPORTED")
        self.assertEqual(json.loads((self.driver.ev / "manifest.json").read_text())["reason"], "transport_denied")

    def test_legacy_text_with_unhealthy_daemon_stays_product_fail(self):
        # A stopped daemon prints the same pre-P2 text: that is not a transport denial.
        self.health_rc = 3
        status, _, _ = self.launch_and_verify(CODEX2_RUN1)
        self.assertEqual(status, demo.FAIL)

    def test_legacy_text_with_socket_allowance_is_not_inferred(self):
        self.driver.facts["codex_transport"] = {"mode": "setup", "argv": ALLOWANCE}
        status, _, _ = self.launch_and_verify(CODEX2_RUN1)
        self.assertEqual(status, demo.FAIL)
        self.assertEqual(self.health, [])

    def test_product_transport_denied_error_is_definitive(self):
        events = Path(self.tmp.name) / "denied.jsonl"
        output = ("herdr-threads: daemon socket not reachable from this sandbox (permission denied): /s.sock; launch the "
                  "harness with the sandbox socket allowance printed by `herdr-threads setup codex` (see docs/install.md) "
                  "(transport_denied)\n")
        events.write_text(json.dumps({"type": "item.completed", "item": {"id": "item_1", "type": "command_execution",
                                      "command": "herdr-threads inbox", "aggregated_output": output, "exit_code": 4}}) + "\n")
        self.driver.facts["codex_transport"] = {"mode": "setup", "argv": ALLOWANCE}
        status, detail, _ = self.launch_and_verify(events)
        self.assertEqual(status, demo.TRANSPORT, detail)
        self.assertIn("transport_denied_error", detail)
        self.assertEqual(self.health, [])

    def test_genuine_failure_elsewhere_still_fails(self):
        self.launch_and_verify(CODEX2_RUN1)
        self.driver.record("S99", "cleanup", demo.FAIL, "tab still listed")
        self.assertEqual(self.driver.manifest(), "FAIL")


class VersionPinTests(unittest.TestCase):
    """S02 reads admission from the build's doctor, including Codex schema-fingerprint admission."""

    def run_pin(self, installed, doctor):
        host = FakeHostDriver(self, harness="codex")
        d = host.driver
        d.facts["versions"] = {"codex": {"out": f"codex-cli {installed}"}}
        d.run = lambda argv, **kw: (0, json.dumps({"doctor": {"hooks": {"codex": doctor}}}), "")
        return d.s_version_pin()

    def test_listed_version_passes(self):
        self.assertEqual(self.run_pin("0.158.0", {"recipes": "codex-hooks-v1 {0.157.1, 0.158.0}"})[0], demo.PASS)

    def test_schema_matched_unlisted_version_passes_with_label(self):
        status, detail, _ = self.run_pin("0.159.2", {"recipes": "codex-hooks-v1 {0.157.1, 0.158.0}", "installed": {
            "admission": "schema-matched, live-unverified", "recipe": "codex-hooks-v1", "version": "0.159.2"}})
        self.assertEqual(status, demo.PASS, detail)
        self.assertIn("schema-matched", detail)

    def test_refused_unlisted_version_fails(self):
        status, _, _ = self.run_pin("0.159.2", {"recipes": "codex-hooks-v1 {0.157.1, 0.158.0}", "installed": {
            "admission": "refused", "version": "0.159.2"}})
        self.assertEqual(status, demo.FAIL)


class ScratchTrustGuardTests(unittest.TestCase):
    """ht-p03.140: a folder-trust answer is approved only for a project inside the run's own scratch root
    (/private/tmp on macOS, /tmp on Linux)."""

    def test_project_inside_private_tmp_run_root_is_approved(self):
        with tempfile.TemporaryDirectory(dir=SCRATCH_ROOT) as tmp:
            root = Path(tmp) / "run"
            (root / "project").mkdir(parents=True)
            self.assertIsNone(demo.scratch_trust_refusal(root / "project", root))

    def test_tmp_spelling_of_private_tmp_root_is_approved(self):
        with tempfile.TemporaryDirectory(dir=SCRATCH_ROOT) as tmp:
            root = Path(tmp) / "run"
            (root / "project").mkdir(parents=True)
            alias = Path("/tmp") / Path(tmp).name / "run"
            if not alias.exists():
                self.skipTest(f"/tmp is not an alias of {SCRATCH_ROOT} here")
            self.assertIsNone(demo.scratch_trust_refusal(alias / "project", alias))

    def test_root_outside_private_tmp_is_refused_naming_the_root(self):
        outside = non_scratch_dir()
        if outside is None:
            self.skipTest(f"no writable directory outside {SCRATCH_ROOT} here")
        with tempfile.TemporaryDirectory(dir=outside) as tmp:
            root = Path(tmp).resolve() / "run"
            (root / "project").mkdir(parents=True)
            reason = demo.scratch_trust_refusal(root / "project", root)
            self.assertIn(str(root), reason)
            self.assertIn("outside", reason)

    def test_project_outside_the_run_root_is_refused_naming_the_project(self):
        with tempfile.TemporaryDirectory(dir=SCRATCH_ROOT) as tmp:
            root = Path(tmp) / "run"
            other = Path(tmp) / "other"
            root.mkdir()
            other.mkdir()
            reason = demo.scratch_trust_refusal(other, root)
            self.assertIn(str(other.resolve()), reason)
            self.assertIn("outside the run root", reason)

    def test_symlink_inside_root_pointing_outside_is_refused(self):
        with tempfile.TemporaryDirectory(dir=SCRATCH_ROOT) as tmp:
            root = Path(tmp) / "run"
            other = Path(tmp) / "other"
            root.mkdir()
            other.mkdir()
            (root / "project").symlink_to(other)
            self.assertIn("outside the run root", demo.scratch_trust_refusal(root / "project", root))

    def test_dotdot_cannot_step_outside(self):
        with tempfile.TemporaryDirectory(dir=SCRATCH_ROOT) as tmp:
            root = Path(tmp) / "run"
            (Path(tmp) / "other").mkdir()
            root.mkdir()
            self.assertIsNotNone(demo.scratch_trust_refusal(root / ".." / "other", root))

    def test_private_tmp_itself_as_project_is_refused(self):
        with tempfile.TemporaryDirectory(dir=SCRATCH_ROOT) as tmp:
            self.assertIsNotNone(demo.scratch_trust_refusal(Path(SCRATCH_ROOT), Path(tmp)))


class CodexBinTests(unittest.TestCase):
    """ht-p03.140: --codex-bin pins the Codex binary for preflight, children, the pane launch and the hook."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.stub = Path(self.tmp.name).resolve() / "pinned" / "codex"
        self.stub.parent.mkdir()
        self.stub.write_text("#!/bin/sh\necho 'codex-cli 0.159.3'\n")
        self.stub.chmod(0o755)
        saved = os.environ["PATH"]
        self.addCleanup(os.environ.__setitem__, "PATH", saved)

    def parse(self, *argv):
        with contextlib.redirect_stderr(io.StringIO()):
            return demo.parse(["--bin", "/bin/true", *argv])

    def rejected(self, *argv):
        with self.assertRaises(SystemExit):
            self.parse(*argv)

    def driver(self, **over):
        host = FakeHostDriver(self, harness="codex", codex_bin=str(self.stub), **over)
        return host.driver

    def test_rejected_with_claude_relative_or_non_executable(self):
        self.rejected("--harness", "claude", "--dry-run", "--codex-bin", str(self.stub))
        self.rejected("--harness", "codex", "--dry-run", "--codex-bin", "codex")
        plain = self.stub.parent / "plain"
        plain.write_text("x")
        self.rejected("--harness", "codex", "--dry-run", "--codex-bin", str(plain))

    def test_absolute_executable_is_accepted(self):
        self.assertEqual(self.parse("--harness", "codex", "--dry-run", "--codex-bin", "/bin/sh").codex_bin, "/bin/sh")

    def test_env_default_is_read(self):
        with mock.patch.dict(os.environ, {"HT_CODEX_BIN": str(self.stub)}):
            self.assertEqual(self.parse("--harness", "codex", "--dry-run").codex_bin, str(self.stub))

    def test_pinned_binary_is_named_in_print_and_tui_launch_strings(self):
        d = self.driver()
        shell, _, _ = d.launch_command("initial")
        self.assertIn(f"command {self.stub} --no-daemon exec", shell)
        self.assertNotIn("command codex ", shell)
        d = self.driver(mode="tui")
        shell, _, _ = d.launch_command("initial")
        self.assertIn(f"command {self.stub} --no-daemon", shell)
        self.assertNotIn("command codex ", shell)

    def test_unpinned_launch_still_names_codex(self):
        host = FakeHostDriver(self, harness="codex")
        shell, _, _ = host.driver.launch_command("initial")
        self.assertIn("command codex --no-daemon exec", shell)

    def test_driver_path_resolves_pinned_binary_first(self):
        self.driver()
        self.assertEqual(os.environ["PATH"].split(os.pathsep)[0], str(self.stub.parent))

    def test_scratch_symlinks_bindir_codex_to_the_pinned_binary(self):
        d = self.driver()
        d.run = lambda argv, **kw: (0, "", "")
        d.s_scratch()
        link = d.bindir / "codex"
        self.assertTrue(link.is_symlink())
        self.assertEqual(os.readlink(link), str(self.stub))

    def test_preflight_records_pinned_binary_version_path_and_pin(self):
        d = self.driver()
        seen = []

        def run(argv, **kw):
            seen.append(argv)
            return 0, "codex-cli 0.159.3\n" if argv[-1] == "--version" and argv[0] == str(self.stub) else "x\n", ""
        d.run = run
        with mock.patch.object(demo.shutil, "which", lambda tool: f"/usr/bin/{tool}"):
            d.s_preflight()
        v = d.facts["versions"]
        self.assertIn([str(self.stub), "--version"], seen)
        self.assertEqual(v["codex"]["out"], "codex-cli 0.159.3")
        self.assertEqual(v["codex_path"], str(self.stub.resolve()))
        self.assertEqual(v["codex_bin_pinned"], str(self.stub))
        self.assertEqual(v["codex_sha256"], demo.sha256(self.stub))

    def test_preflight_unpinned_records_no_pin(self):
        host = FakeHostDriver(self, harness="codex")
        d = host.driver
        d.run = lambda argv, **kw: (0, "codex-cli 0.160.0\n", "")
        with mock.patch.object(demo.shutil, "which", lambda tool: f"/usr/bin/{tool}"):
            d.s_preflight()
        self.assertIsNone(d.facts["versions"]["codex_bin_pinned"])


EVIDENCE = Path(__file__).resolve().parent / "evidence"
DEMO2_STREAM = EVIDENCE / "claude-demo2-initial.stream.jsonl"
DEMO2_READ = ("herdr-threads --state-dir $SCRATCH/ht-demo-claude-20260930T014825-3e2d33/state read "
              "thread-c5194919-0e4f-4651-a5fc-7fc2a8fa10bc --recent 20")
CODEX1_EVENTS = EVIDENCE / "codex-demo1-initial.events.jsonl"


class ClaudeDemo2Tests(VerifyTests):
    """fix3 D4/D5 (Claude demo 2): the real print-mode stream with a `permission_denied` event."""

    def test_transcript_calls_skips_string_message_events(self):
        # Kills (D4): AttributeError on `system/permission_denied`, whose `message` is a string.
        calls = self.driver.transcript_calls(DEMO2_STREAM)
        self.assertEqual(calls, [("toolu_01GWw3ZvU5fCKsa4Bg5A17nR", DEMO2_READ, False)])

    def test_launch_surfaces_permission_denials_cost_and_tokens(self):
        status, detail, _ = self.driver.judge_launch("initial", demo.PASS, "agent process exited 0", DEMO2_STREAM)
        self.assertEqual(status, demo.PASS, detail)
        self.assertIn("1 permission denial(s)", detail)
        self.assertIn("read thread-c5194919", detail)
        self.assertIn("$0.0258818", detail)
        self.assertEqual(self.driver.facts["usage"]["initial"]["tokens"]["output_tokens"], 573)

    def test_verify_reports_denial_not_driver_error(self):
        # Kills (D4): S18 hiding the real reason (the denied ready command) behind a driver error.
        self.driver.judge_launch("initial", demo.PASS, "exited 0", DEMO2_STREAM)
        status, detail, _ = self.driver.step("S18", "verify", lambda: self.driver.s_wait_and_verify("initial", DEMO2_STREAM)) or (None, None, None)
        row = self.driver.steps[-1]
        self.assertEqual(row["status"], demo.FAIL)
        self.assertNotIn("driver error", row["detail"])
        self.assertIn("harness denied 1 herdr-threads command(s)", row["detail"])
        self.assertIn(DEMO2_READ, row["detail"])

    def test_manifest_source_is_none_without_stored_provenance(self):
        # Kills (D5): `source: cooperative` when no accept/ACK provenance was stored.
        self.driver.s_wait_and_verify("initial", DEMO2_STREAM)
        self.driver.record("S18", "verify", demo.FAIL, "denied")
        self.assertEqual(self.driver.manifest(), "FAIL")
        self.assertEqual(json.loads((self.driver.ev / "manifest.json").read_text())["source"], "none")
        self.assertEqual(self.driver.facts["manifest_source"], "none")
        self.assertEqual(self.driver.phase_results[-1]["provenance_label"], "none")


class ReadyCommandPermissionTests(unittest.TestCase):
    """fix3 D6: setup must install `Bash(herdr-threads *)`, and every ready command must be permitted before launch."""

    def setUp(self):
        self.host = FakeHostDriver(self)
        self.d = self.host.driver
        (self.d.project / ".claude").mkdir(parents=True)
        self.context = json.loads(json.loads((EVIDENCE / "claude-demo2-hook-probe-SessionStart.json").read_text())["stdout"])[
            "hookSpecificOutput"]["additionalContext"]

    def settings(self, allow, deny=()):
        (self.d.project / ".claude" / "settings.local.json").write_text(json.dumps({"permissions": {"allow": allow, "deny": list(deny)}}))

    def test_ready_commands_parsed_from_real_context(self):
        commands = demo.ready_commands(self.context)
        self.assertEqual(len(commands), 5)
        self.assertTrue(all(c.startswith("herdr-threads --state-dir ") for c in commands))
        self.assertEqual([c.split()[3] for c in commands], ["accept", "read", "ack", "inbox", "pending-receipts"])
        self.assertEqual(demo.ready_commands("no block here"), [])

    def test_demo2_setup_rules_fail_early_on_probe_context(self):
        # Kills (D6): launching a model whose every ready command print mode will deny (demo 2 P6).
        (self.d.project / ".claude" / "settings.local.json").write_text((EVIDENCE / "claude-demo2-settings.local.json").read_text())
        self.d.facts["probe_context"] = {"SessionStart": self.context}
        status, detail, _ = self.d.s_ready_permitted()
        self.assertEqual(status, demo.FAIL, detail)
        self.assertIn("5/5 ready commands (SessionStart probe context)", detail)

    def test_new_rule_permits_probe_context(self):
        self.settings([demo.HERDR_THREADS_ALLOW])
        self.d.facts["probe_context"] = {"SessionStart": self.context}
        status, detail, _ = self.d.s_ready_permitted()
        self.assertEqual(status, demo.PASS, detail)

    def test_live_run_checks_rendered_forms(self):
        self.d.facts.update({"thread": THREAD, "messages": {"initial": MSG}, "agent_seat": SEAT})
        self.settings([demo.LEGACY_EXPORT_ALLOW])
        self.assertEqual(self.d.s_ready_permitted()[0], demo.FAIL)
        self.settings([demo.HERDR_THREADS_ALLOW])
        status, detail, _ = self.d.s_ready_permitted()
        self.assertEqual(status, demo.PASS, detail)
        self.assertIn("rendered ready-command forms", detail)
        self.assertEqual(self.d.facts["ready_commands_check"]["unpermitted"], [])

    def test_deny_rule_wins(self):
        self.settings([demo.HERDR_THREADS_ALLOW], deny=["Bash(herdr-threads *ack*)"])
        self.d.facts["probe_context"] = {"SessionStart": self.context}
        status, detail, _ = self.d.s_ready_permitted()
        self.assertEqual(status, demo.FAIL)
        self.assertIn("1/5", detail)

    def test_rule_matching(self):
        m = demo.rule_matches
        self.assertTrue(m("Bash(herdr-threads *)", "herdr-threads --state-dir /s inbox"))
        self.assertTrue(m("Bash(herdr-threads *)", "herdr-threads"))
        self.assertFalse(m("Bash(herdr-threads *)", "herdr-threadsX inbox"))
        self.assertTrue(m("Bash(herdr-threads:*)", "herdr-threads inbox"))
        self.assertFalse(m(demo.LEGACY_EXPORT_ALLOW, "herdr-threads inbox"))
        self.assertFalse(m("Read(*)", "herdr-threads inbox"))
        self.assertFalse(demo.command_permitted("herdr-threads inbox && rm -rf /tmp/x", [demo.HERDR_THREADS_ALLOW]))
        self.assertTrue(demo.command_permitted("herdr-threads inbox", ["Bash"]))

    def test_codex_is_not_applicable(self):
        host = FakeHostDriver(self, harness="codex")
        self.assertEqual(host.driver.s_ready_permitted()[0], demo.INFO)

    def test_setup_requires_new_rule_not_export_rule(self):
        # Kills (D6): accepting the dead export rule as a working setup.
        d = self.d
        d.facts["cli_surface"] = {"setup": {"present": True}}
        d.args.setup, d.args.setup_argv = "auto", None
        written = {}
        def ht(*argv, tag, **kw):
            target = Path(kw["extra_env"]["CLAUDE_CONFIG_DIR"]) / "settings.json"
            target.parent.mkdir(exist_ok=True)
            target.write_text(json.dumps({"permissions": {"allow": written["allow"]}}))
            return 0, json.dumps({"setup": {}}), ""
        d.ht = ht
        written["allow"] = [demo.LEGACY_EXPORT_ALLOW]
        status, detail = d.s_hooks()[:2]
        self.assertEqual(status, demo.FAIL)
        self.assertIn("dead export-prefix rule", detail)
        written["allow"] = [demo.HERDR_THREADS_ALLOW]
        self.assertEqual(d.s_hooks()[0], demo.PASS)


class CodexDemo1Tests(unittest.TestCase):
    """fix3 D5/D6 (Codex demo 1): a usage-limit turn failure is ENVIRONMENT, surfaced, and never a product FAIL."""

    def setUp(self):
        self.host = FakeHostDriver(self, harness="codex")
        self.d = self.host.driver
        self.d.facts.update({"agent_seat": SEAT, "messages": {"initial": MSG}})

    def test_outcome_surfaces_turn_failed_and_classifies_usage_limit(self):
        outcome = self.d.transcript_outcome(CODEX1_EVENTS)
        self.assertEqual(outcome["environment"], "usage_limit")
        self.assertEqual(outcome["thread_id"], "01a0f180-fa65-7573-9556-834025e37e76")
        self.assertTrue(any(f.startswith("turn.failed: You") for f in outcome["failures"]))
        status, detail, _ = self.d.judge_launch("initial", demo.FAIL, "agent process exited 1", CODEX1_EVENTS)
        self.assertEqual(status, demo.ENVIRONMENT)
        self.assertIn("hit your usage limit", detail)
        self.assertIn("tokens none reported", detail)
        self.assertEqual(self.d.facts["sessions"]["initial"], "01a0f180-fa65-7573-9556-834025e37e76")

    def test_auth_failure_is_environment(self):
        self.assertEqual(demo.classify_environment("turn.failed: 401 Unauthorized: please login again"), "auth")
        self.assertIsNone(demo.classify_environment("turn.failed: sandbox denied socket connect"))

    def test_non_environment_failure_stays_fail(self):
        path = Path(self.host.tmp.name) / "e.jsonl"
        path.write_text(json.dumps({"type": "turn.failed", "error": {"message": "stream disconnected"}}) + "\n")
        status, detail, _ = self.d.judge_launch("initial", demo.FAIL, "exited 1", path)
        self.assertEqual(status, demo.FAIL)
        self.assertIn("stream disconnected", detail)

    def test_token_usage_summed_from_turn_completed(self):
        path = Path(self.host.tmp.name) / "ok.jsonl"
        usage = {"input_tokens": 14752, "cached_input_tokens": 11008, "output_tokens": 40, "reasoning_output_tokens": 0}
        path.write_text("".join(json.dumps({"type": "turn.completed", "usage": usage}) + "\n" for _ in range(2)))
        status, detail, _ = self.d.judge_launch("initial", demo.PASS, "exited 0", path)
        self.assertEqual(status, demo.PASS)
        self.assertEqual(self.d.facts["usage"]["initial"]["tokens"]["input_tokens"], 29504)
        self.assertIn("'output_tokens': 80", detail)

    def blocked_run(self):
        d = self.d
        for sid in ("S01", "S13", "S14", "S16P"):
            d.record(sid, sid, demo.PASS, "ok")
        d.judge_launch("initial", demo.FAIL, "exited 1", CODEX1_EVENTS)
        d.record("S17", "launch", demo.ENVIRONMENT, "usage limit")
        d.step("S17H", "hook", lambda: (demo.PASS, "", None), needs=("S17",))
        d.step("S18", "verify", lambda: (demo.PASS, "", None), needs=("S17",))
        d.record("S99", "cleanup", demo.PASS, "ok")

    def test_manifest_is_environment_unsupported_not_scenario_failed(self):
        self.blocked_run()
        self.assertEqual(self.d.manifest(), "UNSUPPORTED")
        record = json.loads((self.d.ev / "manifest.json").read_text())
        self.assertEqual(record["reason"], "environment_usage_limit")
        self.assertEqual(record["source"], "none")

    def test_genuine_failure_elsewhere_still_fails(self):
        self.blocked_run()
        self.d.record("S99", "cleanup", demo.FAIL, "tab still listed")
        self.assertEqual(self.d.manifest(), "FAIL")


class CodexRolloutTests(unittest.TestCase):
    """fix3 D4 (Codex demo 1): hook-context delivery read READ-ONLY from the Codex rollout, else UNVERIFIED."""

    THREAD_ID = "01a0f180-fa65-7573-9556-834025e37e76"

    def setUp(self):
        self.host = FakeHostDriver(self, harness="codex")
        self.d = self.host.driver
        self.home = Path(self.host.tmp.name) / "codex-home"
        self.d.facts.update({"codex_home": str(self.home), "thread": THREAD, "messages": {"initial": MSG},
                             "sessions": {"initial": self.THREAD_ID},
                             "phase_started_utc": {"initial": "2026-09-30T08:49:00+00:00"},
                             "phase_ended_utc": {"initial": "2026-09-30T08:50:00+00:00"}})
        self.rollout = self.home / "sessions" / "2026" / "09" / "30" / f"rollout-2026-09-30T08-49-01-{self.THREAD_ID}.jsonl"

    def line(self, role, text, stamp="2026-09-30T08:49:05.000Z"):
        self.rollout.parent.mkdir(parents=True, exist_ok=True)
        entry = {"timestamp": stamp, "type": "response_item",
                 "payload": {"type": "message", "role": role, "content": [{"type": "input_text", "text": text}]}}
        with self.rollout.open("a") as stream:
            stream.write(json.dumps(entry) + "\n")

    def test_missing_rollout_is_unverified_not_blocked(self):
        status, detail, _ = self.d.s_hook_context("initial")
        self.assertEqual(status, demo.UNVERIFIED, detail)

    def test_session_start_developer_message_is_delivery(self):
        self.line("user", "<environment_context>...</environment_context>")
        self.line("developer", f"{demo.HOOK_PREAMBLE} ... Ready commands ... herdr-threads ack {MSG} in {THREAD}")
        self.line("user", demo.PROMPT)
        self.line("developer", f"{demo.HOOK_PREAMBLE} ... still pending {MSG}", stamp="2026-09-30T08:49:09.000Z")
        before = (self.rollout.stat().st_mtime_ns, self.rollout.read_bytes())
        status, detail, _ = self.d.s_hook_context("initial")
        self.assertEqual(status, demo.PASS, detail)
        self.assertEqual(self.d.facts["hook_context"]["initial"], {"session_start": 1, "pre_tool_use": 1, "rollout": str(self.rollout)})
        self.assertEqual((self.rollout.stat().st_mtime_ns, self.rollout.read_bytes()), before)

    def test_rollout_without_context_fails(self):
        self.line("user", demo.PROMPT)
        self.assertEqual(self.d.s_hook_context("initial")[0], demo.FAIL)

    def test_entries_outside_phase_window_do_not_count(self):
        self.line("developer", f"{demo.HOOK_PREAMBLE} pending {MSG} in {THREAD}", stamp="2026-09-30T08:40:00.000Z")
        self.assertEqual(self.d.s_hook_context("initial")[0], demo.FAIL)

    def test_flat_excerpt_form_is_parsed(self):
        items = json.loads((EVIDENCE / "codex-158-rollout-model-items.json").read_text())
        developer = [demo.Driver.rollout_message(i) for i in items if (demo.Driver.rollout_message(i) or ("",))[0] == "developer"]
        self.assertIn(("developer", "herdr-threads-session-marker"), developer)
        self.assertIn(("developer", "herdr-threads-capture-marker"), developer)

    def live_codex_run(self, hook_status):
        d = self.d
        d.facts.update({"agent_seat": SEAT})
        for sid in ("S01", "S02H", "S04", "S13", "S14", "S16P", "S17", "S18", "S18C", "S99"):
            d.record(sid, sid, demo.PASS, "ok")
        d.record("S17H", "hook", hook_status, "rollout")
        d.phase_results.append({"phase": "initial", "message": MSG, "root_ack_calls": [("item_1", f"herdr-threads ack {MSG}", False)],
                                "receipt": {"state": "acked"}, "provenance": {"ack": "cooperative_top_level", "accept": "cooperative_top_level"},
                                "child_check": "no_subagent_activity"})

    def test_live_codex_manifest_can_pass(self):
        # Kills (D4): a live Codex manifest that can never PASS.
        self.live_codex_run(demo.PASS)
        self.assertEqual(self.d.manifest(), "PASS")

    def test_unverified_hook_context_is_unsupported_not_fail(self):
        self.live_codex_run(demo.UNVERIFIED)
        self.assertEqual(self.d.manifest(), "UNSUPPORTED")
        self.assertEqual(json.loads((self.d.ev / "manifest.json").read_text())["reason"], "hook_context_unverified")


class CodexProfileTests(unittest.TestCase):
    """--codex-profile NAME|auto: CODEX_HOME for the launch only, read-only, usage-limit retry bounded and recorded."""

    def setUp(self):
        self.host = FakeHostDriver(self, harness="codex")
        self.d = self.host.driver
        self.root = Path(self.d.args.aisw_codex_root)
        for name in ("codex-1", "codex-2", "default"):
            (self.root / name).mkdir(parents=True)
            (self.root / name / "config.toml").write_text("sentinel\n")
        self.listing = self.snapshot()

    def snapshot(self):
        return sorted((str(p), p.stat().st_mtime_ns, p.read_bytes() if p.is_file() else b"") for p in self.root.rglob("*"))

    def test_named_profile_sets_codex_home_for_launch_only(self):
        self.d.args.codex_profile = "codex-2"
        status, detail, _ = self.d.s_codex_profile()
        self.assertEqual(status, demo.PASS, detail)
        shell, _, _ = self.d.launch_command("initial")
        self.assertIn(f"CODEX_HOME={self.d.codex_setup_home()}", shell)
        self.assertEqual(self.d.facts["codex_launch_homes"]["codex-2"]["auth_source"],
                         str(self.root / "codex-2" / "auth.json"))
        self.assertLess(shell.index("aisw workspace check --tool codex"), shell.index("CODEX_HOME="))
        self.assertLess(shell.index("CODEX_HOME="), shell.index("command codex"))
        self.assertEqual(self.snapshot(), self.listing)

    def test_missing_profile_fails(self):
        self.d.args.codex_profile = "codex-3"
        self.assertEqual(self.d.s_codex_profile()[0], demo.FAIL)

    def fake_launches(self, limited):
        def launch_print(phase, shell, rcfile, transcript):
            name = self.d.facts["codex_profile"]
            self.assertIn(f"CODEX_HOME={self.d.facts['codex_launch_homes'][name]['home']}", shell)
            if name in limited:
                transcript.write_text(CODEX1_EVENTS.read_text())
                return demo.FAIL, "agent process exited 1", transcript
            transcript.write_text(json.dumps({"type": "thread.started", "thread_id": f"t-{name}"}) + "\n"
                                  + json.dumps({"type": "turn.completed", "usage": {"input_tokens": 10, "output_tokens": 2}}) + "\n")
            return demo.PASS, "agent process exited 0", transcript
        self.d.launch_print = launch_print
        self.d.args.codex_profile = "auto"
        self.assertEqual(self.d.s_codex_profile()[0], demo.PASS)

    def test_auto_retries_next_profile_on_usage_limit(self):
        self.fake_launches({"codex-1"})
        status, detail, _ = self.d.launch_codex_auto("initial")
        self.assertEqual(status, demo.PASS, detail)
        self.assertEqual([a["profile"] for a in self.d.facts["codex_profile_attempts"]], ["codex-1", "codex-2"])
        self.assertEqual(self.d.facts["codex_profile_attempts"][0]["environment"], "usage_limit")
        self.assertEqual(self.d.facts["codex_profile_pinned"], "codex-2")
        self.assertEqual(self.d.facts["sessions"]["initial"], "t-codex-2")
        self.assertIn("codex-1=usage_limit", detail)
        self.assertEqual(self.snapshot(), self.listing)
        # The pinned profile is reused for a later phase (resume must see the same CODEX_HOME).
        resume = self.d.launch_command("resume")[0]
        self.assertIn(f"CODEX_HOME={self.d.facts['codex_launch_homes']['codex-2']['home']}", resume)
        self.assertEqual(self.d.facts["codex_launch_homes"]["codex-2"]["auth_source"],
                         str(self.root / "codex-2" / "auth.json"))

    def test_auto_is_bounded_and_ends_environment(self):
        self.fake_launches({"codex-1", "codex-2", "default"})
        status, detail, _ = self.d.launch_codex_auto("initial")
        self.assertEqual(status, demo.ENVIRONMENT, detail)
        self.assertEqual([a["profile"] for a in self.d.facts["codex_profile_attempts"]], ["codex-1", "codex-2", "default"])

    def test_auto_does_not_retry_non_environment_failure(self):
        self.fake_launches(set())
        self.d.launch_print = lambda phase, shell, rcfile, transcript: (demo.FAIL, "exited 2", None)
        status, _, _ = self.d.launch_codex_auto("initial")
        self.assertEqual(status, demo.FAIL)
        self.assertEqual(len(self.d.facts["codex_profile_attempts"]), 1)

    def test_auto_does_not_relaunch_a_successful_run_with_a_usage_looking_error_event(self):
        # Kills the fix3-review B1 mutation (retry keyed on outcome["environment"] alone): a run that exited 0
        # but whose stream carries a transient usage-limit-looking `error` event must keep its PASS and stop.
        self.fake_launches(set())

        def launch_print(phase, shell, rcfile, transcript):
            transcript.write_text(CODEX1_EVENTS.read_text()
                                  + json.dumps({"type": "turn.completed", "usage": {"input_tokens": 10, "output_tokens": 2}}) + "\n")
            return demo.PASS, "agent process exited 0", transcript
        self.d.launch_print = launch_print
        status, detail, _ = self.d.launch_codex_auto("initial")
        self.assertNotEqual(status, demo.ENVIRONMENT, detail)
        self.assertEqual(len(self.d.facts["codex_profile_attempts"]), 1)

    def test_args(self):
        with contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit):
                demo.parse(["--bin", "/bin/true", "--harness", "claude", "--codex-profile", "codex-1"])
            with self.assertRaises(SystemExit):
                demo.parse(["--bin", "/bin/true", "--harness", "codex", "--dry-run", "--codex-profile", "../x"])
            with self.assertRaises(SystemExit):  # auto adds up to 3 retry launches to the timeout budget
                demo.parse(["--bin", "/bin/true", "--harness", "codex", "--dry-run", "--codex-profile", "auto", "--timeout", "400"])
            demo.parse(["--bin", "/bin/true", "--harness", "codex", "--dry-run", "--codex-profile", "auto"])


class ChildSignalTests(unittest.TestCase):
    def test_children_get_default_signals_while_driver_ignores_them(self):
        # Kills (N1): SIG_IGN inherited by cleanup children across exec.
        host = FakeHostDriver(self)
        previous = signal.signal(signal.SIGTERM, signal.SIG_IGN)
        try:
            rc, out, _ = host.driver.run(["/bin/sh", "-c", "trap; grep SigIgn /proc/self/status 2>/dev/null; kill -TERM $$; echo alive"], tag="t")
        finally:
            signal.signal(signal.SIGTERM, previous)
        self.assertNotIn("alive", out)
        self.assertEqual(rc, -signal.SIGTERM)


DEMO3_STREAM = EVIDENCE / "claude-demo3-initial.stream.jsonl"
DEMO3_SQLITE = json.loads((EVIDENCE / "claude-demo3-sqlite-initial.json").read_text())
DEMO3_DENIED = 'alias herdr-threads 2>&1; declare -F herdr-threads 2>&1; echo "---"; type herdr-threads'


class ClaudeDemo3VerifyTests(unittest.TestCase):
    """D7 (Claude demo 3): S18 on the real demo-3 stream and SQLite extract. The model accepted and ACKed unaided;
    its one permission denial was an `alias/declare/type herdr-threads` probe, not a herdr-threads command."""

    HARNESS = "claude"
    tearDown = VerifyTests.tearDown

    def setUp(self):
        VerifyTests.setUp(self)  # the real migrated schema; the demo-3 rows are added below
        seat, message = DEMO3_SQLITE["seat"], DEMO3_SQLITE["message"]
        receipt, invitation = DEMO3_SQLITE["receipts"][0], DEMO3_SQLITE["invitations"][0]
        self.thread = "thread-d8ad9d0a-a8c0-4b1f-818f-f354b76e7414"
        self.db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES (?,?,?,?,?,?)",
                        (seat, "i", "resolved", "operator_fresh", 2, 0))
        self.db.execute("INSERT INTO send_manifests VALUES ('prep-3',?, 'i', ?, 6, 0, 3, 1, 1, 0)", (message, self.thread))
        self.db.execute("INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) "
                        "VALUES ('prep-3', ?, ?, 1, 900000, 0)", (self.thread, seat))
        self.db.execute("INSERT INTO receipt_state(message_id,seat_id,state,ack_actor_seat_id,ack_generation,ack_observation,acked_at) "
                        "VALUES (?,?,'acked',?,?,?,?)", (message, seat, receipt["ack_actor_seat_id"], receipt["ack_generation"],
                                                        receipt["ack_observation"], receipt["acked_at"]))
        self.db.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at,"
                        "accepted_at,accepted_actor_seat_id,accepted_generation,accepted_observation) VALUES (?,?,?,1,'accepted',1,0,900000,1,?,?,?,?)",
                        (invitation["id"], self.thread, seat, invitation["accepted_at"], invitation["accepted_actor_seat_id"],
                         invitation["accepted_generation"], invitation["accepted_observation"]))
        self.db.commit()
        self.driver.facts.update({"agent_seat": seat, "thread": self.thread, "messages": {"initial": message},
                                  "sessions": {"initial": "893e0b3b-f6cf-41aa-ab85-4a707c29bd42"}})

    def test_s18_passes_on_demo3_evidence_and_reports_other_denial_as_info(self):
        # Kills (D7): counting `alias herdr-threads; ...; type herdr-threads` as a denied herdr-threads command.
        status, detail, _ = self.driver.judge_launch("initial", demo.PASS, "exited 0", DEMO3_STREAM)
        self.assertIn("1 permission denial(s)", detail)
        self.driver.step("S18", "verify", lambda: self.driver.s_wait_and_verify("initial", DEMO3_STREAM))
        row = self.driver.steps[-1]
        self.assertEqual(row["status"], demo.PASS, row["detail"])
        self.assertIn("info: 1 other permission denial(s), not herdr-threads commands", row["detail"])
        self.assertIn("type herdr-threads", row["detail"])
        result = self.driver.phase_results[-1]
        self.assertEqual(result["permission_denials"], [])
        self.assertEqual([d["command"] for d in result["other_permission_denials"]], [DEMO3_DENIED])
        self.assertEqual(result["provenance_label"], "cooperative")

    def test_herdr_threads_denial_still_fails(self):
        # The D4 guard stays: a denied command whose command word is herdr-threads is still a FAIL.
        self.driver.judge_launch("initial", demo.PASS, "exited 0", DEMO3_STREAM)
        outcome = self.driver.facts["launch_outcome"]["initial"]
        outcome["permission_denials"] = outcome["permission_denials"] + [{"command": "cd /p && herdr-threads ack msg-x"}]
        status, detail, _ = self.driver.s_wait_and_verify("initial", DEMO3_STREAM)
        self.assertEqual(status, demo.FAIL, detail)
        self.assertIn("harness denied 1 herdr-threads command(s)", detail)
        self.assertIn("info: 1 other permission denial(s)", detail)


class HerdrThreadsCommandWordTests(unittest.TestCase):
    """D7: a simple command's command word, not a substring, decides a herdr-threads denial."""

    def test_command_word_matching(self):
        yes = ["herdr-threads inbox", "herdr-threads", "cd /p && herdr-threads ack msg-1", "FOO=1 herdr-threads inbox",
               "/opt/bin/herdr-threads --state-dir /s inbox", "echo x; herdr-threads read t | head", "(herdr-threads inbox)",
               "x=$(herdr-threads inbox)"]
        no = [DEMO3_DENIED, "which herdr-threads", "echo herdr-threads", "herdr-threadsX inbox", "ls ~/herdr-threads-data",
              "grep herdr-threads log", "", None]
        for command in yes:
            self.assertTrue(demo.invokes_herdr_threads(command), command)
        for command in no:
            self.assertFalse(demo.invokes_herdr_threads(command), command)


FAKE_BIN = "#!/bin/sh\n# stands in for the old clap CLI: a repeated --state-dir is exit 2\n" \
           'n=0; for w in "$@"; do [ "$w" = --state-dir ] && n=$((n+1)); done\n' \
           '[ "$n" -gt 1 ] && { echo "the argument --state-dir cannot be used multiple times" >&2; exit 2; }\n' \
           'printf "%s\\n" "$@"\n'


class AgentShimTests(unittest.TestCase):
    """D8 (Claude demo 3): the agent PATH shim never doubles --state-dir/--host-endpoint, and the dry run executes
    one rendered ready command through it."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.bin = Path(self.tmp.name) / "fake-herdr-threads"
        self.bin.write_text(FAKE_BIN)
        self.bin.chmod(0o700)

    def shim(self, text=None):
        path = Path(self.tmp.name) / "herdr-threads"
        path.write_text(text or demo.agent_shim(str(self.bin), Path("/st ate"), "/h.sock"))
        path.chmod(0o700)
        return path

    def run_shim(self, *argv):
        import subprocess
        p = subprocess.run([str(self.shim()), *argv], capture_output=True, text=True)
        return p.returncode, p.stdout.splitlines(), p.stderr

    def test_pins_only_missing_flags(self):
        self.assertEqual(self.run_shim("inbox")[:2], (0, ["--state-dir", "/st ate", "--host-endpoint", "/h.sock", "inbox"]))
        self.assertEqual(self.run_shim("--state-dir", "/s", "inbox")[:2], (0, ["--host-endpoint", "/h.sock", "--state-dir", "/s", "inbox"]))
        self.assertEqual(self.run_shim("--state-dir=/s", "--host-endpoint", "/x", "inbox")[:2], (0, ["--state-dir=/s", "--host-endpoint", "/x", "inbox"]))
        # Kills the `case " $* "` substring shortcut: a body that merely mentions the flag is not the flag.
        self.assertEqual(self.run_shim("send", "t", "--body", "see --state-dir docs")[1][:2], ["--state-dir", "/st ate"])

    def dry_driver(self):
        d = demo.Driver(args(self.tmp.name, dry_run=True, bin=str(self.bin), host_endpoint="/h.sock"))
        self.addCleanup(d.cmdlog.close)
        self.assertEqual(d.s_scratch()[0], demo.PASS)
        probe = json.loads((EVIDENCE / "claude-demo3-hook-probe-SessionStart.json").read_text())
        context = json.loads(probe["stdout"])["hookSpecificOutput"]["additionalContext"]
        context = context.replace("$SCRATCH/ht-demo-claude-20260930T023754-66b6fa/state", shlex_quote(str(d.state)))
        d.facts.update({"agent_pane": "w1:p1", "probe_context": {"SessionStart": context}})
        return d

    def test_dry_run_executes_a_rendered_ready_command(self):
        d = self.dry_driver()
        status, detail, _ = d.s_ready_executes()
        self.assertEqual(status, demo.PASS, detail)
        self.assertTrue(d.facts["ready_command_executed"]["command"].endswith(" pending-receipts"))
        self.assertIn(f"--state-dir {d.state}", d.facts["ready_command_executed"]["command"])

    def test_dry_run_step_catches_the_demo3_shim(self):
        # Kills (D8): the demo-3 shim (always pins --state-dir) must fail the dry run before any model launch.
        d = self.dry_driver()
        (d.bindir / "herdr-threads").write_text("#!/bin/sh\nexec " + " ".join([str(self.bin), "--state-dir", str(d.state),
                                                                                  "--host-endpoint", "/h.sock"]) + ' "$@"\n')
        status, detail, _ = d.s_ready_executes()
        self.assertEqual(status, demo.FAIL)
        self.assertIn("exits 2", detail)
        self.assertIn("cannot be used multiple times", detail)

    def test_live_run_skips(self):
        d = demo.Driver(args(self.tmp.name))
        self.addCleanup(d.cmdlog.close)
        with self.assertRaises(demo.Blocked):
            d.s_ready_executes()
        self.assertIn("S16X", demo.DRY_ONLY)


# ---------------------------------------------------------------------------------------------------------------
# Opt-in scenario matrix (ht-4is.11.3 / 11.4 / 32.3 / ht-910): every new verdict path, offline.
# ---------------------------------------------------------------------------------------------------------------

class ScenarioBase(DbFixture):
    """A VerifyTests fixture with scenarios enabled and Claude stream helpers for spawns, results and sidechains."""
    SCENARIO = "base"

    def setUp(self):
        super().setUp()
        self.driver.args.scenario = self.SCENARIO
        self.driver.scenarios = demo.parse_scenarios(self.SCENARIO)
        self.driver.project.mkdir(exist_ok=True)
        self.driver.facts["phase_transcripts"] = {"initial": str(self.transcript)}

    def emit(self, event, path=None):
        with (path or self.transcript).open("a") as stream:
            stream.write(json.dumps(event) + "\n")

    def call(self, call_id, command, parent=None, name="Bash", sidechain=False):
        self.emit({"type": "assistant", "parent_tool_use_id": parent, "isSidechain": sidechain,
                   "message": {"content": [{"type": "tool_use", "id": call_id, "name": name, "input": {"command": command}}]}})

    def result(self, call_id, text, error=False, parent=None):
        self.emit({"type": "user", "parent_tool_use_id": parent,
                   "message": {"content": [{"type": "tool_result", "tool_use_id": call_id, "content": text, "is_error": error}]}})

    def spawn(self, call_id="toolu_task"):
        self.emit({"type": "assistant", "parent_tool_use_id": None,
                   "message": {"content": [{"type": "tool_use", "id": call_id, "name": "Task", "input": {"prompt": "read mail"}}]}})

    def root_ack(self):
        self.call("toolu_accept", f"herdr-threads accept {THREAD}")
        self.call("toolu_ack", f"herdr-threads ack {MSG}")


class ChildScenarioTests(ScenarioBase):
    """(a) --scenario child: child reads allowed, no child ACK/accept, top-level ACK present."""
    SCENARIO = "child"

    def test_delegated_read_with_root_ack_passes_every_item(self):
        # Kills: treating any subagent activity as unverifiable even when the complete sidechain proves no child ACK.
        self.settle()
        self.spawn()
        self.call("toolu_c1", "herdr-threads pending-receipts", parent="toolu_task")
        self.result("toolu_c1", f"{MSG} pending", parent="toolu_task")
        self.root_ack()
        status, detail, _ = self.verify()
        self.assertEqual(status, demo.PASS, detail)
        self.assertEqual(self.driver.phase_results[-1]["child_check"], "sidechain_verified_no_child_mutation")
        self.assertEqual(self.driver.s_child_check("initial")[0], demo.PASS)
        self.assertEqual(self.driver.s_child_read("initial")[0], demo.PASS)
        self.assertEqual(self.driver.s_child_no_ack("initial")[0], demo.PASS)
        self.assertEqual(self.driver.s_top_level_ack("initial")[0], demo.PASS)
        self.assertTrue((self.driver.ev / "child-sidechain-initial.json").exists())

    def test_refused_child_ack_is_the_designed_outcome(self):
        self.settle()
        self.spawn()
        self.call("toolu_c1", "herdr-threads read thread-1 --recent 5", parent="toolu_task")
        self.result("toolu_c1", "ok", parent="toolu_task")
        self.call("toolu_c2", f"herdr-threads ack {MSG}", parent="toolu_task")
        self.result("toolu_c2", "Exit code 1\nherdr-threads: error: subagent cannot ACK (unauthorized)", error=True, parent="toolu_task")
        self.root_ack()
        status, detail, _ = self.verify()
        self.assertEqual(status, demo.PASS, detail)
        status, detail, _ = self.driver.s_child_no_ack("initial")
        self.assertEqual(status, demo.PASS, detail)
        self.assertIn("1 child attempt(s) refused", detail)

    def test_successful_child_ack_fails(self):
        # Kills: a child ACK that the product accepted passing as delegated evidence.
        self.settle()
        self.spawn()
        self.call("toolu_c2", f"herdr-threads ack {MSG}", parent="toolu_task")
        self.result("toolu_c2", "acked", parent="toolu_task")
        self.root_ack()
        self.assertEqual(self.verify()[0], demo.FAIL)
        self.assertEqual(self.driver.s_child_no_ack("initial")[0], demo.FAIL)

    def test_no_delegation_is_not_exercised(self):
        # Kills: reporting child read/absence PASS when the model never delegated.
        self.settle()
        self.root_ack()
        self.verify()
        self.assertEqual(self.driver.s_child_read("initial")[0], demo.NOT_EXERCISED)
        self.assertEqual(self.driver.s_child_no_ack("initial")[0], demo.NOT_EXERCISED)

    def test_spawn_without_sidechain_record_is_unverified(self):
        # Kills: PASS for child ACK absence when the child's calls are not in the evidence at all.
        self.settle()
        self.spawn()
        self.root_ack()
        self.verify()
        self.assertEqual(self.driver.phase_results[-1]["child_check"], "unverified_subagent_activity")
        self.assertEqual(self.driver.s_child_check("initial")[0], demo.UNVERIFIED)
        self.assertEqual(self.driver.s_child_no_ack("initial")[0], demo.UNVERIFIED)
        self.assertEqual(self.driver.s_child_read("initial")[0], demo.UNVERIFIED)

    def test_subagent_transcript_file_completes_the_sidechain(self):
        # Claude writes subagent turns to <project>/<session>/subagents/*.jsonl; those count as the sidechain record.
        self.settle()
        self.spawn()
        self.root_ack()
        encoded = "".join(c if c.isalnum() else "-" for c in str(self.driver.project))
        side = Path(self.driver.args.claude_projects_dir) / encoded / SESSION / "subagents" / "agent-a1.jsonl"
        side.parent.mkdir(parents=True)
        self.emit({"type": "assistant", "isSidechain": True, "sessionId": SESSION,
                   "message": {"content": [{"type": "tool_use", "id": "toolu_s1", "name": "Bash", "input": {"command": "herdr-threads inbox"}}]}}, side)
        self.emit({"type": "user", "isSidechain": True,
                   "message": {"content": [{"type": "tool_result", "tool_use_id": "toolu_s1", "content": "1 thread"}]}}, side)
        self.verify()
        status, detail, _ = self.driver.s_child_read("initial")
        self.assertEqual(status, demo.PASS, detail)
        self.assertIn("agent-a1.jsonl", detail)
        self.assertEqual(self.driver.s_child_no_ack("initial")[0], demo.PASS)

    def test_ack_execution_other_than_root_binding_fails(self):
        # Kills: ignoring the DB provenance execution join of the cooperative child check.
        sidechain = {"spawns": ["t"], "calls": [], "complete": True, "missing": []}
        self.assertFalse(demo.Driver.sidechain_clears(sidechain, "exec-child", "exec-root"))
        self.assertTrue(demo.Driver.sidechain_clears(sidechain, "exec-root", "exec-root"))
        self.driver.phase_results.append({"phase": "initial", "message": MSG, "sidechain": sidechain, "root_ack_calls": [],
                                          "provenance": {"ack_execution": "exec-child"}, "root_binding_execution_diagnostic": "exec-root"})
        status, detail, _ = self.driver.s_child_no_ack("initial")
        self.assertEqual(status, demo.FAIL)
        self.assertIn("not the root binding execution", detail)

    def test_child_prompt_is_used_for_the_launch(self):
        shell, _, _ = self.driver.launch_command("initial")
        self.assertIn(shlex_quote(demo.CHILD_PROMPT), shell)
        self.assertNotIn(shlex_quote(demo.PROMPT), shell)


class CodexChildScenarioTests(ScenarioBase):
    HARNESS = "codex"
    SCENARIO = "child"

    def setUp(self):
        super().setUp()
        self.driver.facts["codex_home"] = str(Path(self.tmp.name) / "codex-home")

    def item(self, item, kind="item.completed"):
        self.emit({"type": kind, "item": item})

    def root_ack(self):
        for command in (f"herdr-threads accept {THREAD}", f"herdr-threads ack {MSG}"):
            self.item({"id": f"item_{len(command)}", "type": "command_execution", "command": command, "exit_code": 0, "aggregated_output": ""})

    def rollout(self, thread, command, exit_code=0):
        path = Path(self.driver.facts["codex_home"]) / "sessions" / "2026" / "09" / "30" / f"rollout-2026-09-30T00-00-00-{thread}.jsonl"
        path.parent.mkdir(parents=True, exist_ok=True)
        self.emit({"type": "response_item", "payload": {"type": "function_call", "name": "shell", "call_id": "call_1",
                                                        "arguments": json.dumps({"command": ["bash", "-lc", command]})}}, path)
        self.emit({"type": "response_item", "payload": {"type": "function_call_output", "call_id": "call_1",
                                                        "output": json.dumps({"output": "ok", "metadata": {"exit_code": exit_code}})}}, path)

    def test_child_rollout_read_completes_the_sidechain(self):
        self.settle()
        self.item({"id": "item_9", "type": "collab_tool_call", "tool": "spawn_agent", "sender_thread_id": SESSION,
                   "receiver_thread_ids": ["child-1"], "status": "completed"})
        self.rollout("child-1", "herdr-threads pending-receipts")
        self.root_ack()
        status, detail, _ = self.verify()
        self.assertEqual(status, demo.PASS, detail)
        self.assertEqual(self.driver.s_child_read("initial")[0], demo.PASS)
        self.assertEqual(self.driver.s_child_no_ack("initial")[0], demo.PASS)
        self.assertEqual(self.driver.s_child_check("initial")[0], demo.PASS)

    def test_missing_child_rollout_stays_unverified(self):
        # Kills: Codex child calls are invisible in the parent stream; without the child rollout absence is unproven.
        self.settle()
        self.item({"id": "item_9", "type": "collab_tool_call", "tool": "spawn_agent", "receiver_thread_ids": ["child-2"]})
        self.root_ack()
        self.verify()
        self.assertEqual(self.driver.s_child_no_ack("initial")[0], demo.UNVERIFIED)
        self.assertEqual(self.driver.s_child_check("initial")[0], demo.UNVERIFIED)


class MidturnScenarioTests(ScenarioBase):
    """(b) --scenario midturn."""
    SCENARIO = "midturn"
    MID = "msg-mid"

    def setUp(self):
        super().setUp()
        self.sent = []

        def fake_send(phase, deadline=None):
            self.sent.append(phase)
            self.driver.facts["messages"][phase] = self.MID
            return demo.PASS, "sent", self.MID
        self.driver.send = fake_send
        encoded = "".join(c if c.isalnum() else "-" for c in str(self.driver.project))
        self.session_path = Path(self.driver.args.claude_projects_dir) / encoded / f"{SESSION}.jsonl"
        self.session_path.parent.mkdir(parents=True)

    def pre_tool_use(self, text, stamp):
        self.emit({"type": "attachment", "timestamp": stamp, "attachment": {"type": "hook_additional_context", "content": [text],
                   "hookEvent": "PreToolUse", "hookName": "PreToolUse:Bash"}}, self.session_path)

    def trigger(self):
        self.call("toolu_first", "herdr-threads pending-receipts")
        self.driver.launch_tick("initial", self.transcript)
        self.driver.facts["midturn"]["sent_utc"] = "2026-09-30T06:44:00+00:00"

    def test_tick_sends_exactly_once_after_first_tool_call(self):
        self.driver.launch_tick("initial", self.transcript)
        self.assertEqual(self.sent, [])
        self.call("toolu_first", "herdr-threads pending-receipts")
        self.driver.launch_tick("initial", self.transcript)
        self.driver.launch_tick("initial", self.transcript)
        self.assertEqual(self.sent, ["midturn"])
        self.assertEqual(self.driver.facts["midturn"]["after_call"], "toolu_first")
        self.assertEqual(self.driver.s_midturn_sent()[0], demo.PASS)

    def test_no_tool_call_is_not_exercised(self):
        self.assertEqual(self.driver.s_midturn_sent()[0], demo.NOT_EXERCISED)
        self.driver.step("SM1", "sent", self.driver.s_midturn_sent)
        self.driver.step("SM2", "delivery", self.driver.s_midturn_delivery, needs=("SM1",))
        self.assertEqual(self.driver.status_of("SM2"), demo.NOT_EXERCISED)  # propagated, never BLOCKED

    def test_failed_send_fails(self):
        self.driver.facts["midturn"] = {"after_call": "x", "sent_utc": "2026-09-30T06:44:00+00:00", "send_status": demo.FAIL, "detail": "rc=1"}
        self.assertEqual(self.driver.s_midturn_sent()[0], demo.FAIL)

    def test_pretooluse_attachment_naming_message_is_delivery(self):
        self.trigger()
        self.call("toolu_ack", f"herdr-threads ack {MSG}")
        self.pre_tool_use("stale context", "2026-09-30T06:43:00.000Z")  # before the send: never counted
        self.pre_tool_use(f"new message {self.MID}", "2026-09-30T06:44:05.000Z")
        status, detail, _ = self.driver.s_midturn_delivery()
        self.assertEqual(status, demo.PASS, detail)
        self.assertIn("named msg-mid", detail)

    def test_no_attachment_but_model_ack_is_behaviour_inferred(self):
        self.trigger()
        self.call("toolu_ack2", f"herdr-threads ack {self.MID}")
        status, detail, _ = self.driver.s_midturn_delivery()
        self.assertEqual(status, demo.PASS, detail)
        self.assertEqual(self.driver.facts["midturn"]["delivery"], "behaviour_inferred")

    def test_later_calls_without_delivery_or_ack_are_unverified(self):
        self.trigger()
        self.call("toolu_other", "ls")
        self.pre_tool_use("early", "2026-09-30T06:40:00.000Z")
        self.assertEqual(self.driver.s_midturn_delivery()[0], demo.UNVERIFIED)

    def test_no_later_tool_call_is_not_exercised(self):
        self.trigger()
        self.assertEqual(self.driver.s_midturn_delivery()[0], demo.NOT_EXERCISED)

    def insert_mid(self, acked):
        self.db.execute("INSERT INTO send_manifests VALUES ('prep-2',?, 'i', ?, 6, 0, 4, 1, 1, 0)", (self.MID, THREAD))
        self.db.execute("INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) "
                        "VALUES ('prep-2', ?, ?, 1, 900000, 0)", (THREAD, SEAT))
        if acked:
            observation = json.dumps({"harness": "claude", "session": SESSION, "provenance": "cooperative_top_level", "execution": "e"})
            self.db.execute("INSERT INTO receipt_state(message_id,seat_id,state,ack_actor_seat_id,ack_generation,ack_observation,acked_at) "
                            "VALUES (?,?,'acked',?,1,?,1)", (self.MID, SEAT, SEAT, observation))
        self.db.commit()

    def test_midturn_ack_passes_and_joins_the_manifest(self):
        self.trigger()
        self.insert_mid(acked=True)
        self.call("toolu_ack2", f"herdr-threads ack {self.MID}")
        status, detail, _ = self.driver.s_midturn_ack()
        self.assertEqual(status, demo.PASS, detail)
        self.assertEqual(self.driver.phase_results[-1]["phase"], "midturn")

    def test_midturn_pending_fails(self):
        # The model saw the "watch for new mail" instruction (a root `body` read of the handoff), so an unACKed
        # mid-turn message is a FAIL; unseen, it is NOT_EXERCISED (InstructionGateTests).
        self.call("toolu_body", f"herdr-threads body {MSG}")
        self.trigger()
        self.insert_mid(acked=False)
        self.call("toolu_ack2", f"herdr-threads ack {self.MID}")
        self.assertEqual(self.driver.s_midturn_ack()[0], demo.FAIL)

    def test_midturn_db_ack_without_root_call_fails(self):
        self.trigger()
        self.insert_mid(acked=True)
        status, detail, _ = self.driver.s_midturn_ack()
        self.assertEqual(status, demo.FAIL)
        self.assertIn("no root", detail)

    def test_initial_handoff_asks_for_new_mail(self):
        self.assertIn("new herdr-threads mail", self.driver.scenario_instructions("initial"))
        self.assertEqual(self.driver.scenario_instructions("restart"), "")


class WarningScenarioTests(ScenarioBase):
    """(c) --scenario warning: one durable warning, one coalesced wake."""
    SCENARIO = "warning"

    def setUp(self):
        super().setUp()
        self.driver.facts["message_deadlines"] = {"initial": 5}

    def receipt(self, deadline_at=5000, acked_at=None, warning=None):
        self.db.execute("INSERT INTO receipt_state(message_id,seat_id,state,available_at,deadline_at,acked_at,warning_message_id) "
                        "VALUES (?,?,?,?,?,?,?)", (MSG, SEAT, "acked" if acked_at else "pending", 0 if deadline_at is not None else None,
                                                   deadline_at, acked_at, warning))
        self.db.commit()

    def warn(self, wid, seq):
        self.db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_seq,decision_at,source_message_id) "
                        "VALUES (?,?,?,?,'warn','{}',?,6000,?)", (wid, "i", THREAD, seq, seq, MSG))
        self.db.commit()

    def test_one_warning_after_late_ack_passes(self):
        self.receipt(acked_at=9000, warning="w-1")
        self.warn("w-1", 10)
        status, detail, _ = self.driver.s_warning_event()
        self.assertEqual(status, demo.PASS, detail)
        self.assertIn("late ACK", detail)

    def test_missing_warning_fails(self):
        self.receipt(acked_at=9000)
        self.assertEqual(self.driver.s_warning_event()[0], demo.FAIL)

    def test_duplicate_warning_fails(self):
        self.receipt(acked_at=9000, warning="w-1")
        self.warn("w-1", 10)
        self.warn("w-2", 11)
        status, detail, _ = self.driver.s_warning_event()
        self.assertEqual(status, demo.FAIL)
        self.assertIn("duplicates", detail)

    def test_ack_before_deadline_is_not_exercised(self):
        self.receipt(acked_at=4000)
        self.assertEqual(self.driver.s_warning_event()[0], demo.NOT_EXERCISED)

    def test_unstarted_timer_fails(self):
        self.receipt(deadline_at=None)
        self.assertEqual(self.driver.s_warning_event()[0], demo.FAIL)

    def wake(self, warning_seq, receipt_seq=None):
        self.db.execute("INSERT INTO wake_work(seat_id,reason_bits,last_reservation_id,last_reservation_boot,last_reserved_at_utc,"
                        "last_warning_seq,last_warning_offset,last_receipt_seq,last_receipt_offset,last_outcome) VALUES (?,?,?,?,?,?,?,?,?,?)",
                        (SEAT, 1, "r1", "b1", 7000, warning_seq, 0 if warning_seq else None, receipt_seq, 0 if receipt_seq else None, "submitted"))
        self.db.commit()

    def test_wake_reservation_covering_the_warning_passes(self):
        self.receipt(acked_at=9000, warning="w-1")
        self.warn("w-1", 10)
        self.driver.s_warning_event()
        self.wake(10, receipt_seq=5)
        (self.driver.ev / "pane-initial.txt").write_text(f"{demo.WAKE_MARKER}; run herdr-threads inbox\n")
        status, detail, _ = self.driver.s_warning_wake()
        self.assertEqual(status, demo.PASS, detail)
        self.assertIn("together with the pending receipt", detail)

    def test_no_wake_is_unverified_not_pass(self):
        self.receipt(acked_at=9000, warning="w-1")
        self.warn("w-1", 10)
        self.driver.s_warning_event()
        self.assertEqual(self.driver.s_warning_wake()[0], demo.UNVERIFIED)

    def test_repeated_marker_fails(self):
        self.receipt(acked_at=9000, warning="w-1")
        self.warn("w-1", 10)
        self.driver.s_warning_event()
        self.wake(10)
        (self.driver.ev / "pane-initial.txt").write_text(f"{demo.WAKE_MARKER}\n{demo.WAKE_MARKER}\n")
        self.assertEqual(self.driver.s_warning_wake()[0], demo.FAIL)

    def test_earlier_wake_in_the_warning_scrollback_is_not_a_storm(self):
        # ht-p03.20: the warning capture still shows the initial handoff's wake; one new marker is one coalesced wake.
        self.receipt(acked_at=9000, warning="w-1")
        self.warn("w-1", 10)
        self.driver.s_warning_event()
        self.wake(10, receipt_seq=5)
        self.driver.facts["warning_marker_baseline"] = 1
        (self.driver.ev / "pane-warning-baseline.txt").write_text(f"{demo.WAKE_MARKER}\n")
        (self.driver.ev / "pane-warning.txt").write_text(f"{demo.WAKE_MARKER}\nmore\n{demo.WAKE_MARKER}\n")
        status, detail, _ = self.driver.s_warning_wake()
        self.assertEqual(status, demo.PASS, detail)

    def test_two_new_markers_after_the_baseline_fail(self):
        self.receipt(acked_at=9000, warning="w-1")
        self.warn("w-1", 10)
        self.driver.s_warning_event()
        self.wake(10)
        self.driver.facts["warning_marker_baseline"] = 1
        (self.driver.ev / "pane-warning.txt").write_text(f"{demo.WAKE_MARKER}\n" * 3)
        self.assertEqual(self.driver.s_warning_wake()[0], demo.FAIL)

    def test_short_deadline_goes_on_the_initial_handoff_only(self):
        calls = []
        self.driver.facts.update({"coordinator_seat": COORD, "coordinator_pane": "w1:p2"})
        self.driver.ht = lambda *argv, **kw: (calls.append(argv), (0, json.dumps({"message_id": "m"}), ""))[1]
        self.driver.send("initial")
        self.driver.send("restart")
        self.assertEqual(calls[0][calls[0].index("--deadline") + 1], "5")
        self.assertEqual(calls[1][calls[1].index("--deadline") + 1], "900")


class BurstScenarioTests(ScenarioBase):
    """(d) --scenario burst."""
    SCENARIO = "burst"

    def test_has_more_and_continuation_pass(self):
        self.call("toolu_i1", "herdr-threads inbox")
        self.result("toolu_i1", json.dumps({"inbox": {"items": [], "has_more": True, "next_argv": ["herdr-threads", "inbox", "--cursor", "c"]}}))
        self.call("toolu_i2", "herdr-threads inbox --seat seat-a --limit 20 --max-bytes 16384 --cursor c")
        self.result("toolu_i2", "{}")
        self.assertEqual(self.driver.s_burst_has_more()[0], demo.PASS)
        self.assertEqual(self.driver.s_burst_continuation()[0], demo.PASS)

    def test_compact_text_next_line_passes(self):
        """ht-4is.8.18: the compact machine text names its continuation on one `next:` line."""
        self.call("toolu_i1", "herdr-threads inbox")
        self.result("toolu_i1", "inbox\nthread-a invitations=1\nnext: herdr-threads inbox --seat seat-a --cursor c3:BAIBFAKQffeisGs\n")
        self.call("toolu_i2", "herdr-threads inbox --seat seat-a --cursor c3:BAIBFAKQffeisGs")
        self.result("toolu_i2", "inbox\nthread-b invitations=1\n")
        self.assertEqual(self.driver.s_burst_has_more()[0], demo.PASS)
        self.assertEqual(self.driver.s_burst_continuation()[0], demo.PASS)

    def test_compact_text_without_next_line_fails(self):
        self.call("toolu_i1", "herdr-threads inbox")
        self.result("toolu_i1", "inbox\nthread-a invitations=1\n")
        self.assertEqual(self.driver.s_burst_has_more()[0], demo.FAIL)

    def test_no_continuation_fails(self):
        self.call("toolu_i1", "herdr-threads inbox")
        self.result("toolu_i1", '{"has_more": true}')
        self.assertEqual(self.driver.s_burst_has_more()[0], demo.PASS)
        status, detail, _ = self.driver.s_burst_continuation()
        self.assertEqual(status, demo.FAIL)
        self.assertIn("never the continuation", detail)

    def test_no_inbox_call_fails(self):
        self.call("toolu_p", "herdr-threads pending-receipts")
        self.assertEqual(self.driver.s_burst_has_more()[0], demo.FAIL)

    def test_result_without_has_more_fails(self):
        self.call("toolu_i1", "herdr-threads inbox")
        self.result("toolu_i1", '{"has_more": false}')
        self.assertEqual(self.driver.s_burst_has_more()[0], demo.FAIL)

    def test_failed_continuation_fails(self):
        self.call("toolu_i2", "herdr-threads inbox --cursor c")
        self.result("toolu_i2", "cursor stale", error=True)
        self.assertEqual(self.driver.s_burst_continuation()[0], demo.FAIL)

    def test_dry_probe_runs_the_rendered_continuation_verbatim(self):
        # Kills (dry run 1): prepending --json to a continuation that already carries it (exit 2, duplicate flag).
        self.driver.dry = True
        self.driver.facts["burst_threads"] = ["t"] * 22
        seen = []
        next_argv = ["herdr-threads", "--state-dir", "S", "--json", "inbox", "--seat", SEAT, "--cursor", "c"]

        def fake_cli(argv, tag):
            seen.append(argv)
            page = {"inbox": {"has_more": len(seen) == 1, "next_argv": next_argv if len(seen) == 1 else None}}
            return 0, json.dumps(page), ""
        self.driver.agent_pane_cli = fake_cli
        status, detail, _ = self.driver.s_burst_probe()
        self.assertEqual(status, demo.PASS, detail)
        self.assertEqual(seen[1].count("--json"), 1)
        self.assertEqual(seen[1][1:], next_argv[1:])

    def test_setup_needs_more_than_one_page(self):
        created = []

        def fake_ht(*argv, tag, **kw):
            if tag.startswith("coordinator:burst-thread"):
                created.append(f"thread-b{len(created)}")
                self.db.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at) "
                                "VALUES (?,?,?,1,'pending',1,0,900000,1)", (f"inv-{created[-1]}", created[-1], SEAT))
                self.db.commit()
                return 0, json.dumps({"thread_id": created[-1]}), ""
            return 0, "{}", ""
        self.driver.ht = fake_ht
        self.driver.facts.update({"coordinator_pane": "w1:p2", "coordinator_seat": COORD})
        self.driver.args.burst_threads = 22
        status, detail, _ = self.driver.s_burst_setup()
        self.assertEqual(status, demo.PASS, detail)
        self.assertEqual(len(self.driver.facts["burst_threads"]), 22)
        self.assertIn("whole inbox", self.driver.scenario_instructions("initial"))


class RequiredScenarioTests(ScenarioBase):
    """(e) --scenario required (ht-4is.32.3)."""
    SCENARIO = "required"
    SVC = "svc-thread"

    def setUp(self):
        super().setUp()
        self.driver.facts["required"] = {"thread": self.SVC, "invitation": "inv-r", "requirement": "req-1", "revision": 1}
        self.db.execute("INSERT INTO service_authors(id,instance_id,created_at) VALUES ('author-1','i',0)")
        self.db.commit()

    def requirement(self, state="accepted", provenance="cooperative_top_level"):
        observation = json.dumps({"provenance": provenance, "harness": "claude"})
        self.db.execute("INSERT INTO requirement_episodes(id,thread_id,seat_id,issuer_author_id,invitation_id,revision,state,created_decision_seq,"
                        "created_at,accepted_by_seat_id,accepted_generation,accepted_observation,accepted_at) VALUES "
                        "('req-1',?,?,'author-1','inv-r',1,?,1,0,?,?,?,?)",
                        (self.SVC, SEAT, state, *((SEAT, 1, observation, 1) if state == "accepted" else (None, None, None, None))))
        self.db.commit()

    def membership(self, state):
        self.db.execute("INSERT INTO memberships(thread_id,seat_id,state) VALUES (?,?,?)", (self.SVC, SEAT, state))
        self.db.commit()

    def test_accept_required_with_root_call_passes(self):
        self.requirement()
        self.call("toolu_r", f"herdr-threads accept-required {self.SVC} --invitation inv-r --requirement req-1 --revision 1")
        status, detail, _ = self.driver.s_required_accept()
        self.assertEqual(status, demo.PASS, detail)

    def test_db_acceptance_without_root_call_fails(self):
        self.requirement()
        self.assertEqual(self.driver.s_required_accept()[0], demo.FAIL)

    def test_pending_requirement_fails(self):
        self.requirement(state="pending")
        self.call("toolu_r", f"herdr-threads accept-required {self.SVC}")
        self.assertEqual(self.driver.s_required_accept()[0], demo.FAIL)

    def test_refused_leave_with_membership_held_passes(self):
        self.membership("joined")
        self.call("toolu_l", f"herdr-threads leave {self.SVC}")
        self.result("toolu_l", "herdr-threads: error: required membership cannot be left (conflict)", error=True)
        status, detail, _ = self.driver.s_required_leave()
        self.assertEqual(status, demo.PASS, detail)

    def test_leave_that_left_fails(self):
        self.membership("left")
        self.call("toolu_l", f"herdr-threads leave {self.SVC}")
        self.result("toolu_l", "left")
        self.assertEqual(self.driver.s_required_leave()[0], demo.FAIL)

    def test_no_leave_attempt_is_not_exercised(self):
        self.membership("joined")
        self.assertEqual(self.driver.s_required_leave()[0], demo.NOT_EXERCISED)

    def event(self, mid):
        self.db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_seq,decision_at) VALUES (?,?,?,?,'info','{}',?,1)",
                        (mid, "i", self.SVC, 1, 40))
        self.db.commit()

    def test_service_events_without_receipts_pass(self):
        self.event("evt-1")
        self.assertEqual(self.driver.s_required_no_receipts()[0], demo.PASS)

    def test_service_event_with_receipt_row_fails(self):
        self.event("evt-1")
        self.db.execute("INSERT INTO receipt_state(message_id,seat_id,state) VALUES ('evt-1',?,'pending')", (SEAT,))
        self.db.commit()
        self.assertEqual(self.driver.s_required_no_receipts()[0], demo.FAIL)

    def test_handoff_names_the_required_thread_and_leave(self):
        text = self.driver.scenario_instructions("initial")
        self.assertIn(self.SVC, text)
        self.assertIn(f"herdr-threads leave {self.SVC}", text)


class ServiceSendVerdicts(ScenarioBase):
    """--scenario servicesend (ht-5nb.4): the model ACKs a service-authored request by its exact ID."""
    SCENARIO = "servicesend"
    SVC = "svc-send-thread"
    SVC_MSG = "msg-svc"
    AUTHOR = "author-1"

    def setUp(self):
        super().setUp()
        self.driver.facts["servicesend"] = {"thread": self.SVC, "author": self.AUTHOR, "message": self.SVC_MSG,
                                            "invitation": "inv-r", "requirement": "req-1", "revision": 1}
        self.db.execute("INSERT INTO service_authors(id,instance_id,created_at) VALUES (?,'i',0)", (self.AUTHOR,))
        self.db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES (?,'i','t','g',0,0)", (self.SVC,))
        self.db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,author_kind,author_service_id,actor_seat_id,"
                        "decision_seq,decision_at) VALUES (?,?,?,1,'ordinary','do it','programmatic',?,NULL,40,1)",
                        (self.SVC_MSG, "i", self.SVC, self.AUTHOR))
        self.db.execute("INSERT INTO send_manifests VALUES ('prep-svc',?, 'i', ?, 6, 0, 3, 1, 1, 0)", (self.SVC_MSG, self.SVC))
        self.db.execute("INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) "
                        "VALUES ('prep-svc', ?, ?, 1, 900000, 0)", (self.SVC, SEAT))
        self.db.commit()

    def ack(self, provenance="cooperative_top_level", actor=SEAT):
        observation = json.dumps({"harness": "claude", "session": SESSION, "provenance": provenance, "execution": "exec-root"})
        self.db.execute("INSERT INTO receipt_state(message_id,seat_id,state,ack_actor_seat_id,ack_generation,ack_observation,acked_at) "
                        "VALUES (?,?,'acked',?,1,?,1)", (self.SVC_MSG, SEAT, actor, observation))
        self.db.commit()

    def test_root_ack_with_cooperative_provenance_passes(self):
        self.ack()
        self.call("toolu_s", f"herdr-threads ack {self.SVC_MSG}")
        status, detail, _ = self.driver.s_servicesend_ack()
        self.assertEqual(status, demo.PASS, detail)
        self.assertIn("cooperative_top_level", detail)
        self.assertIn("toolu_s", detail)
        record = json.loads((self.driver.ev / "servicesend-ack.json").read_text())
        self.assertEqual((record["provenance"], len(record["root_ack_calls"])), ("cooperative_top_level", 1))

    def test_acked_without_a_root_call_fails(self):
        self.ack()
        self.call("toolu_e", f'echo "herdr-threads ack {self.SVC_MSG}"')
        self.assertEqual(self.driver.s_servicesend_ack()[0], demo.FAIL)

    def test_acked_by_a_child_call_is_not_a_root_ack(self):
        self.ack()
        self.call("toolu_c", f"herdr-threads ack {self.SVC_MSG}", parent="toolu_task", sidechain=True)
        self.assertEqual(self.driver.s_servicesend_ack()[0], demo.FAIL)

    def test_ack_by_another_actor_fails(self):
        self.ack(actor=COORD)
        self.call("toolu_s", f"herdr-threads ack {self.SVC_MSG}")
        status, detail, _ = self.driver.s_servicesend_ack()
        self.assertEqual(status, demo.FAIL)
        self.assertIn(COORD, detail)

    def test_ack_with_non_model_provenance_fails(self):
        self.ack(provenance="operator_assertion")
        self.call("toolu_s", f"herdr-threads ack {self.SVC_MSG}")
        self.assertEqual(self.driver.s_servicesend_ack()[0], demo.FAIL)

    def test_unacked_request_with_unseen_instruction_is_not_exercised(self):
        self.call("toolu_x", "herdr-threads inbox")
        gated = self.driver.gate_on_instruction("servicesend", self.driver.s_servicesend_ack())
        self.assertEqual(gated[0], demo.NOT_EXERCISED, gated[1])
        self.assertIn("never saw", gated[1])

    def test_unacked_request_with_seen_instruction_fails(self):
        self.call("toolu_x", "herdr-threads inbox")
        self.result("toolu_x", f"a) {demo.SCENARIO_MARKERS['servicesend']} {self.SVC}: accept-required it")
        gated = self.driver.gate_on_instruction("servicesend", self.driver.s_servicesend_ack())
        self.assertEqual(gated[0], demo.FAIL, gated[1])

    def test_programmatic_message_without_author_receipt_passes(self):
        status, detail, _ = self.driver.s_servicesend_author()
        self.assertEqual(status, demo.PASS, detail)

    def test_seat_authored_message_fails_the_author_check(self):
        self.db.execute("UPDATE messages SET author_kind=NULL, author_service_id=NULL, actor_seat_id=? WHERE id=?", (COORD, self.SVC_MSG))
        self.db.commit()
        self.assertEqual(self.driver.s_servicesend_author()[0], demo.FAIL)

    def test_receipt_row_for_the_author_fails_the_author_check(self):
        self.db.execute("INSERT INTO receipt_state(message_id,seat_id,state) VALUES (?,?,'pending')", (self.SVC_MSG, self.AUTHOR))
        self.db.commit()
        self.assertEqual(self.driver.s_servicesend_author()[0], demo.FAIL)

    def test_steps_are_registered_and_gated(self):
        driver = self.driver
        for sid in ("S17", "SS0"):
            driver.record(sid, sid, demo.PASS, "ok")
        self.call("toolu_x", "herdr-threads inbox")
        driver.scenario_verdicts()
        status = {row["step"]: row["status"] for row in driver.steps}
        self.assertEqual(status["SS1"], demo.NOT_EXERCISED)
        self.assertEqual(status["SS2"], demo.PASS)

    def test_scenario_is_selectable(self):
        self.assertEqual(demo.parse_scenarios("servicesend"), ["servicesend"])
        self.assertEqual(demo.parse_scenarios("required,servicesend"), ["required", "servicesend"])

    def test_handoff_names_the_service_thread(self):
        text = self.driver.scenario_instructions("initial")
        self.assertIn("Accept this thread's invitation (step 1)", text)
        self.assertIn(f"service request, thread {self.SVC}: accept-required it", text)
        self.assertIn("then ACK its service message by its exact ID", text)


class ServiceWireTests(unittest.TestCase):
    """The Python D2 service client speaks the daemon framing and the ServiceWireRequest shape."""

    def setUp(self):
        import socket
        import threading
        self.tmp = tempfile.TemporaryDirectory(dir="/tmp")
        self.sock_path = str(Path(self.tmp.name) / "d.sock")
        self.server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.server.bind(self.sock_path)
        self.server.listen(1)
        self.requests = []
        self.refuse = None

        def serve():
            conn, _ = self.server.accept()
            with conn:
                while True:
                    head = conn.recv(4)
                    if not head:
                        return
                    size = int.from_bytes(head, "big")
                    body = b""
                    while len(body) < size:
                        body += conn.recv(size - len(body))
                    request = json.loads(body)
                    self.requests.append(request)
                    service = request["service"]
                    kind = service["args"].get("kind") if service["kind"] == "operation" else "register"
                    if kind == self.refuse:
                        result = {"Err": {"code": "conflict", "detail": "no", "restart_argv": None}}
                    elif kind == "register":
                        result = {"Ok": {"kind": "registered", "data": {"author": "author-1", "daemon_boot": "b", "connection_generation": 1}}}
                    elif kind == "invite":
                        result = {"Ok": {"kind": "invitation", "data": {"invitation": "inv-r", "requirement": {
                            "requirement": "req-1", "revision": 1, "invitation": "inv-r", "thread": service["args"]["args"]["thread"],
                            "seat": SEAT, "issuer": "author-1", "state": "pending", "accepted_by": None, "accepted_at": None}}}}
                    elif kind == "send":
                        result = {"Ok": {"kind": "message_sent", "data": {"summary": {"message": "msg-svc", "thread": "t", "sequence": 3},
                                                                        "author": "author-1", "recipient_count": 1,
                                                                        "receipt_duration_millis": 900000}}}
                    else:
                        result = {"Ok": {"kind": "thread_ensured", "data": {}}}
                    reply = json.dumps({"version": demo.WIRE_VERSION, "request_id": request["request_id"], "instance": request["expected_instance"],
                                        "daemon_boot": "b", "result": result}).encode()
                    conn.sendall(len(reply).to_bytes(4, "big") + reply)
        self.thread = threading.Thread(target=serve, daemon=True)
        self.thread.start()
        self.driver = demo.Driver(args(self.tmp.name, scenario="required"))
        descriptor = self.driver.state / "instances" / "i" / "endpoint.json"
        descriptor.parent.mkdir(parents=True)
        descriptor.write_text(json.dumps({"endpoint": self.sock_path, "instance_uuid": "00000000-0000-4000-8000-000000000001"}))
        self.driver.facts.update({"agent_seat": SEAT})

    def tearDown(self):
        self.server.close()
        self.driver.cmdlog.close()
        self.tmp.cleanup()

    def test_required_setup_over_the_wire(self):
        status, detail, _ = self.driver.s_required_setup()
        self.assertEqual(status, demo.PASS, detail)
        kinds = [r["service"]["kind"] if r["service"]["kind"] == "register" else r["service"]["args"]["kind"] for r in self.requests]
        self.assertEqual(kinds, ["register", "ensure_thread", "invite", "notify"])
        invite = self.requests[2]["service"]["args"]["args"]
        self.assertEqual((invite["constraint"], invite["seat"], invite["deadline_millis"]), ("required", SEAT, 900000))
        self.assertEqual({r["expected_instance"] for r in self.requests}, {"00000000-0000-4000-8000-000000000001"})
        self.assertEqual(self.requests[0]["service"]["args"], {"capability": "service_session_v1"})
        self.assertEqual(self.driver.facts["required"]["requirement"], "req-1")
        self.assertTrue((self.driver.ev / "service-client.json").exists())

    def test_servicesend_setup_registers_v2_and_sends_an_ack_required_request(self):
        self.driver.scenarios = demo.parse_scenarios("servicesend")
        status, detail, _ = self.driver.s_servicesend_setup()
        self.assertEqual(status, demo.PASS, detail)
        kinds = [r["service"]["kind"] if r["service"]["kind"] == "register" else r["service"]["args"]["kind"] for r in self.requests]
        self.assertEqual(kinds, ["register", "ensure_thread", "invite", "send"])
        self.assertEqual(self.requests[0]["service"]["args"], {"capability": "service_session_v2"})
        send = self.requests[3]["service"]["args"]["args"]
        self.assertEqual((send["recipients"], send["deadline_millis"], send["operation"]),
                         ([SEAT], 900000, f"{send['thread']}-send"))
        self.assertIn("ACK this message by its exact ID", send["body"])
        facts = self.driver.facts["servicesend"]
        self.assertEqual((facts["message"], facts["author"], facts["requirement"]), ("msg-svc", "author-1", "req-1"))
        self.assertIn(facts["thread"], self.driver.scenario_instructions("initial"))
        self.assertTrue((self.driver.ev / "service-client.json").exists())

    def test_service_refusal_is_a_driver_failure_not_a_pass(self):
        self.refuse = "invite"
        self.driver.step("SR0", "required setup", self.driver.s_required_setup)
        self.assertEqual(self.driver.status_of("SR0"), demo.FAIL)


class ManagedLaunchTests(unittest.TestCase):
    """(f2) --launch managed: detected, never faked."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.driver = demo.Driver(args(self.tmp.name, launch="managed"))
        self.driver.facts.update({"agent_pane": "w1:p9"})

    def tearDown(self):
        self.driver.cmdlog.close()
        self.tmp.cleanup()

    def test_absent_launch_command_is_unsupported_not_fail(self):
        self.driver.facts["cli_surface"] = {"launch": {"present": False, "rc": 2, "first_line": ["error: unrecognized subcommand 'launch'"]}}
        self.driver.step("S17M", "managed", self.driver.s_managed_available)
        self.assertEqual(self.driver.status_of("S17M"), demo.UNSUPPORTED_STEP)
        self.driver.step("S17", "launch", lambda: (demo.PASS, "x", None), needs=("S17M",))
        self.assertEqual(self.driver.manifest(), "UNSUPPORTED")
        record = json.loads((self.driver.ev / "manifest.json").read_text())
        self.assertEqual(record["reason"], "managed_launch_unavailable")
        self.assertTrue(record["scenario"].endswith(".managed"))

    def test_info_surface_probe_does_not_block_the_detection(self):
        # Kills (dry run 1): S17M needing S03, which is INFO by design, so S17M was BLOCKED and never reported.
        self.driver.record("S01", "preflight", demo.PASS, "ok")
        self.driver.record("S03", "surface", demo.INFO, "launch=absent")
        self.driver.facts["cli_surface"] = {"launch": {"present": False, "rc": 2, "first_line": []}}
        self.driver.managed_step()
        self.assertEqual(self.driver.status_of("S17M"), demo.UNSUPPORTED_STEP)

    def test_present_launch_command_passes(self):
        self.driver.facts["cli_surface"] = {"launch": {"present": True, "rc": 0, "first_line": ["Usage: herdr-threads launch"]}}
        self.assertEqual(self.driver.s_managed_available()[0], demo.PASS)

    def test_managed_argv_is_launch_plus_native_args(self):
        self.driver.launch_command("initial")
        argv = self.driver.managed_launch_argv("initial")
        index = argv.index("launch")
        self.assertEqual(argv[index:index + 6], ["launch", "--pane", "w1:p9", "--kind", "claude", "--"])
        self.assertEqual(argv[index + 6], "--model")  # native args, program name dropped
        self.assertIn(demo.PROMPT, argv)
        self.assertIn("--session-id", argv)


class TuiTrustRecordTests(unittest.TestCase):
    """(f) --mode tui --tui-accept-trust: scratch root (/private/tmp on macOS) only; ~/.claude.json recorded, never written."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(dir=SCRATCH_ROOT)
        self.driver = demo.Driver(args(self.tmp.name, mode="tui", tui_accept_trust=True))
        self.driver.project.mkdir()
        self.path = Path(self.driver.args.claude_json)
        self.path.write_text(json.dumps({"numStartups": 1, "oauthAccount": {"email": "x"}, "projects": {"/other": {"a": 1}}}))

    def tearDown(self):
        self.driver.cmdlog.close()
        self.tmp.cleanup()

    def test_before_after_records_trust_entry_without_copying_the_file(self):
        status, detail, _ = self.driver.s_tui_trust_before()
        self.assertEqual(status, demo.PASS, detail)  # S17's TUI prerequisite: INFO would BLOCK every live TUI launch
        data = json.loads(self.path.read_text())
        data["projects"][str(self.driver.project)] = {"hasTrustDialogAccepted": True}
        data["numStartups"] = 2
        self.path.write_text(json.dumps(data))
        written = self.path.read_bytes()
        status, detail, _ = self.driver.s_tui_trust_after()
        self.assertEqual(status, demo.INFO, detail)
        self.assertIn("added", detail)
        diff = json.loads((self.driver.ev / "claude-json-diff.json").read_text())
        self.assertEqual(diff["changed_top_level_keys"], ["numStartups", "projects"])
        self.assertEqual(diff["other_project_entries_changed"], [])
        self.assertEqual(self.path.read_bytes(), written)  # read-only
        for name in ("claude-json-before.json", "claude-json-after.json", "claude-json-diff.json"):
            self.assertNotIn("oauthAccount\": {", (self.driver.ev / name).read_text())
            self.assertNotIn("email", (self.driver.ev / name).read_text())

    def test_project_outside_private_tmp_fails(self):
        outside = non_scratch_dir()
        if outside is None:
            self.skipTest(f"no writable directory outside {SCRATCH_ROOT} here")
        self.driver.project = Path(outside) / "not-private-tmp-project"
        self.assertEqual(self.driver.s_tui_trust_before()[0], demo.FAIL)

    def test_default_run_root_is_private_tmp_under_trust(self):
        driver = demo.Driver(args(self.tmp.name, mode="tui", tui_accept_trust=True, run_root=None))
        self.addCleanup(driver.cmdlog.close)
        self.assertTrue(str(driver.root).startswith(SCRATCH_ROOT + "/ht-native-demo/"))
        import shutil
        self.addCleanup(shutil.rmtree, driver.root, True)

    def test_clear_phase_transcript_is_the_new_session_file(self):
        encoded = "".join(c if c.isalnum() else "-" for c in str(self.driver.project))
        directory = Path(self.driver.args.claude_projects_dir) / encoded
        directory.mkdir(parents=True)
        (directory / f"{SESSION}.jsonl").write_text("{}\n")
        self.driver.facts["sessions"] = {"initial": SESSION, "clear": None}
        self.driver.facts["phase_started_utc"] = {"clear": demo.utc()}
        time.sleep(0.01)
        (directory / "22222222-0000-0000-0000-000000000000.jsonl").write_text("{}\n")
        found = self.driver.tui_transcript("clear")
        self.assertEqual(found.name, "22222222-0000-0000-0000-000000000000.jsonl")
        self.assertEqual(self.driver.facts["sessions"]["clear"], "22222222-0000-0000-0000-000000000000")

    def test_clear_phase_never_reuses_the_previous_session(self):
        self.driver.facts["claude_session"] = SESSION
        self.driver.launch_command("clear")
        self.assertIsNone(self.driver.facts["sessions"]["clear"])


class ScenarioArgsTests(ArgsTests):
    def test_scenarios_are_selectable_and_validated(self):
        parsed = self.parse("--harness", "claude", "--scenario", "child,warning,burst")
        self.assertEqual(demo.parse_scenarios(parsed.scenario), ["child", "warning", "burst"])
        self.rejected("--harness", "claude", "--scenario", "child,bogus")
        self.assertEqual(demo.parse_scenarios("base"), [])

    def test_bounds(self):
        self.rejected("--harness", "claude", "--scenario", "burst", "--burst-threads", "20")
        self.rejected("--harness", "claude", "--scenario", "burst", "--burst-threads", str(demo.MAX_BURST_THREADS + 1))
        self.rejected("--harness", "claude", "--warning-deadline", "0")
        self.parse("--harness", "claude", "--scenario", "burst", "--burst-threads", "21")

    def test_midturn_and_managed_need_a_watchable_launch(self):
        self.rejected("--harness", "claude", "--mode", "tui", "--tui-accept-trust", "--allow-uncapped-spend", "--scenario", "midturn")
        self.rejected("--harness", "claude", "--mode", "tui", "--tui-accept-trust", "--allow-uncapped-spend", "--launch", "managed")
        self.parse("--harness", "claude", "--scenario", "midturn")
        self.parse("--harness", "codex", "--dry-run", "--scenario", "midturn", "--launch", "managed")

    def test_trust_accept_only_under_private_tmp(self):
        self.rejected("--harness", "claude", "--mode", "tui", "--tui-accept-trust", "--allow-uncapped-spend", "--run-root", "/var/tmp/x")
        self.parse("--harness", "claude", "--mode", "tui", "--tui-accept-trust", "--allow-uncapped-spend", "--run-root", SCRATCH_ROOT + "/x")


class ScenarioManifestTests(ManifestTests):
    def test_not_exercised_scenario_is_unsupported_not_pass_or_fail(self):
        self.passing_live_run()
        self.driver.record("SC1", "child read", demo.NOT_EXERCISED, "no delegation")
        self.assertEqual(self.driver.manifest(), "UNSUPPORTED")
        self.assertEqual(json.loads((self.driver.ev / "manifest.json").read_text())["reason"], "scenario_not_exercised")

    def test_scenario_failure_fails(self):
        self.passing_live_run()
        self.driver.record("SB2", "continuation", demo.FAIL, "never paged")
        self.assertEqual(self.driver.manifest(), "FAIL")

    def test_scenario_name_carries_the_selection(self):
        self.driver.scenarios = ["child", "required"]
        self.passing_live_run()
        self.driver.manifest()
        self.assertEqual(json.loads((self.driver.ev / "manifest.json").read_text())["scenario"],
                         "claude-print-prelaunch-handoff.child.required")

    def test_midturn_pair_needs_its_receipt(self):
        self.passing_live_run()
        self.driver.facts["messages"]["midturn"] = "msg-mid"
        self.assertEqual(self.driver.manifest(), "FAIL")
        (self.driver.ev / "manifest.json").unlink()
        self.driver.phase_results.append({"phase": "midturn", "message": "msg-mid", "root_ack_calls": [("t2", "herdr-threads ack msg-mid", False)],
                                          "receipt": {"state": "acked"}, "provenance": {"ack": "cooperative_top_level"}})
        self.assertEqual(self.driver.manifest(), "PASS")


class ScenarioDryProbeTests(unittest.TestCase):
    """Dry-run-only scenario probes skip on a live run and never act for the model."""

    def test_live_run_skips_every_dry_probe(self):
        with tempfile.TemporaryDirectory() as tmp:
            driver = demo.Driver(args(tmp, scenario="midturn,warning,burst,required"))
            try:
                for sid, fn in (("SM0D", driver.s_midturn_probe), ("SW0D", driver.s_warning_probe),
                                ("SB0D", driver.s_burst_probe), ("SR0D", driver.s_required_probe)):
                    driver.step(sid, sid, fn, dry_only=True)
                    self.assertEqual(driver.status_of(sid), demo.SKIP)
                    self.assertRaises(demo.Blocked, fn)
            finally:
                driver.cmdlog.close()


UUID_THREAD = "0192f3a4-5b6c-7d8e-9f01-23456789abcd"


class HandoffPreviewTests(unittest.TestCase):
    """claude D1 / codex D5 (matrix 1): scenario steps lead the handoff, inside the 256-byte CLI preview."""

    def driver_for(self, scenario):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        driver = demo.Driver(args(tmp.name, scenario=scenario))
        self.addCleanup(driver.cmdlog.close)
        driver.scenarios = demo.parse_scenarios(scenario)
        driver.facts.update({"thread": UUID_THREAD, "agent_seat": SEAT, "coordinator_seat": COORD, "coordinator_pane": "w1:p2",
                             "messages": {}, "required": {"thread": UUID_THREAD},
                             "servicesend": {"thread": UUID_THREAD}})
        self.calls = []
        driver.ht = lambda *argv, **kw: (self.calls.append(argv), (0, json.dumps({"message_id": "m"}), ""))[1]
        return driver

    def body(self):
        return self.calls[-1][self.calls[-1].index("--body") + 1]

    def test_each_scenario_instruction_alone_fits_the_preview(self):
        for scenario in ("midturn", "burst", "required", "servicesend"):
            with self.subTest(scenario=scenario):
                driver = self.driver_for(scenario)
                driver.send("initial")
                record = driver.facts["handoff_preview"]["initial"]
                self.assertTrue(record["instructions_in_preview"], record["preview"])
                self.assertLessEqual(len(record["preview"].encode()), demo.PREVIEW_BYTES)
                body = self.body()
                self.assertLess(body.index(" a) "), body.index(" 1) "))  # scenario steps before the standard steps
                self.assertIn(demo.SCENARIO_MARKERS[scenario].lower(), record["preview"].lower())

    def test_burst_step_names_the_handoff_accept_inside_the_preview(self):
        # D8 (Codex matrix 2): "accept no other invitation" preceded step 1's accept, which fell past the preview cut.
        driver = self.driver_for("burst")
        driver.send("initial")
        preview = driver.facts["handoff_preview"]["initial"]["preview"]
        accept = "accept this thread's invitation (step 1)"
        self.assertIn(accept, preview)
        self.assertIn("but no burst invitation", preview)
        self.assertNotIn("accept no other invitation", self.body())
        self.assertLess(preview.index(accept), preview.index("but no burst invitation"))
        self.assertIn(f"herdr-threads accept {UUID_THREAD}", self.body())

    def test_body_pointer_is_in_the_preview_when_several_scenarios_overflow(self):
        driver = self.driver_for("midturn,burst,required")
        driver.send("initial")
        self.assertIn("herdr-threads body MESSAGE_ID", driver.facts["handoff_preview"]["initial"]["preview"])

    def test_preview_prefix_counts_escaped_bytes(self):
        self.assertEqual(len(demo.Driver.preview_prefix('"' * 200)), 128)
        self.assertEqual(len(demo.Driver.preview_prefix("a" * 400)), 256)

    def test_base_handoff_has_no_preview_record(self):
        driver = self.driver_for("base")
        driver.send("initial")
        self.assertNotIn("handoff_preview", driver.facts)
        self.assertNotIn(" a) ", self.body())


class InstructionGateTests(ScenarioBase):
    """claude D1: a scenario judge whose instruction the model never saw is NOT_EXERCISED, not FAIL."""
    SCENARIO = "burst"

    def test_unseen_instruction_turns_fail_into_not_exercised(self):
        self.call("toolu_i1", "herdr-threads read --recent 20")
        status, detail, _ = self.driver.gate_on_instruction("burst", self.driver.s_burst_continuation())
        self.assertEqual(status, demo.NOT_EXERCISED, detail)
        self.assertIn("never saw", detail)

    def test_body_call_means_seen_so_fail_stands(self):
        self.call("toolu_b", f"herdr-threads body {MSG}")
        self.call("toolu_i1", "herdr-threads inbox")
        self.result("toolu_i1", '{"has_more": true}')
        self.assertEqual(self.driver.gate_on_instruction("burst", self.driver.s_burst_continuation())[0], demo.FAIL)

    def test_marker_in_transcript_means_seen(self):
        self.call("toolu_r", "herdr-threads read --recent 20")
        self.result("toolu_r", f"preview: a) {demo.SCENARIO_MARKERS['burst']} (run each continuation it prints)")
        self.assertEqual(self.driver.gate_on_instruction("burst", self.driver.s_burst_continuation())[0], demo.FAIL)

    def test_pass_is_never_gated(self):
        result = (demo.PASS, "ok", None)
        self.assertEqual(self.driver.gate_on_instruction("burst", result), result)

    def test_unacked_midturn_without_instruction_is_not_exercised(self):
        self.driver.facts["midturn"] = {"message": "msg-mid"}
        self.driver.scenarios = demo.parse_scenarios("midturn")
        # W6-D4: with no transcript at all, visibility is unknown (UNVERIFIED); NOT_EXERCISED needs a transcript that lacks it.
        self.assertEqual(self.driver.s_midturn_ack()[0], demo.UNVERIFIED)
        self.call("toolu_x", "herdr-threads inbox")
        self.assertEqual(self.driver.s_midturn_ack()[0], demo.NOT_EXERCISED)
        self.call("toolu_b", f"herdr-threads body {MSG}")
        self.assertEqual(self.driver.s_midturn_ack()[0], demo.FAIL)

    def test_required_and_burst_steps_are_registered_gated(self):
        driver = self.driver
        driver.args.scenario = "burst,required"
        driver.scenarios = demo.parse_scenarios("burst,required")
        driver.facts["required"] = {"thread": "svc"}
        for sid in ("S17", "SB0", "SR0"):
            driver.record(sid, sid, demo.PASS, "ok")
        for name in ("s_burst_has_more", "s_burst_continuation", "s_required_accept", "s_required_leave"):
            setattr(driver, name, lambda: (demo.FAIL, "judge failed", None))
        driver.s_required_no_receipts = lambda: (demo.PASS, "ok", None)
        self.call("toolu_i1", "herdr-threads inbox")  # a transcript that lacks the instruction (W6-D4: none would be UNVERIFIED)
        driver.scenario_verdicts()
        status = {row["step"]: row["status"] for row in driver.steps}
        for sid in ("SB1", "SB2", "SR1", "SR2"):
            self.assertEqual(status[sid], demo.NOT_EXERCISED, sid)


class MutatingVerbTests(unittest.TestCase):
    """claude D4: every herdr-threads write counts as a child mutation, matched on the parsed subcommand."""

    def test_writes_are_mutating(self):
        for command in ("herdr-threads send t --body x", "herdr-threads invite t s", "herdr-threads check-in",
                        "herdr-threads --state-dir /s --json ack m", "herdr-threads thread create --topic x",
                        "cd /p && herdr-threads leave t", "sh -c 'herdr-threads archive t'"):
            with self.subTest(command=command):
                self.assertTrue(demo.mutating_call(command), demo.herdr_threads_verbs(command))

    def test_reads_and_free_text_are_not(self):
        for command in ("herdr-threads search send", "herdr-threads body m", "herdr-threads inbox --cursor c",
                        "herdr-threads thread list", "echo accept ack send", "herdr-threads --state-dir send read m"):
            with self.subTest(command=command):
                self.assertFalse(demo.mutating_call(command), demo.herdr_threads_verbs(command))

    def test_service_disconnect_is_mutating_and_cached_check_in_is_not(self):
        # W6-D1: `service disconnect` writes (commands.rs ServiceSub::Disconnect); cached-check-in only reads.
        self.assertTrue(demo.mutating_call("herdr-threads service disconnect --expected-boot b --expected-generation 1"))
        self.assertFalse(demo.mutating_call("herdr-threads service inspect"))
        self.assertFalse(demo.mutating_call("herdr-threads cached-check-in --reference r"))

    def test_substituted_binary_path_is_matched(self):
        # W6-D2: the name ends a command substitution or a quoted path.
        for command in ('"$(which herdr-threads)" ack X', "$(command -v herdr-threads) accept t",
                        "`which herdr-threads` send t --body x", "'/opt/bin/herdr-threads' ack m"):
            with self.subTest(command=command):
                self.assertTrue(demo.mutating_call(command), demo.herdr_threads_verbs(command))
        self.assertFalse(demo.mutating_call('"$(which herdr-threads)" inbox'))

    def test_verbs_of_several_invocations(self):
        self.assertEqual(demo.herdr_threads_verbs("herdr-threads inbox; herdr-threads seat rebind x"), ["inbox", "seat rebind"])


class ChildSendScenarioTests(ScenarioBase):
    SCENARIO = "child"

    def test_successful_child_send_fails_child_no_ack(self):
        self.settle()
        self.spawn()
        self.call("toolu_c1", f"herdr-threads send {THREAD} --body hi", parent="toolu_task", sidechain=True)
        self.result("toolu_c1", "sent", parent="toolu_task")
        self.root_ack()
        self.verify()
        status, detail, _ = self.driver.s_child_no_ack("initial")
        self.assertEqual(status, demo.FAIL, detail)
        self.assertIn("write", detail)

    def test_pass_detail_labels_execution_equality_as_non_discriminating(self):
        self.settle()
        self.spawn()
        self.call("toolu_c1", "herdr-threads pending-receipts", parent="toolu_task", sidechain=True)
        self.result("toolu_c1", "[]", parent="toolu_task")
        self.root_ack()
        self.verify()
        status, detail, _ = self.driver.s_child_no_ack("initial")
        self.assertEqual(status, demo.PASS, detail)
        self.assertIn("does not discriminate", detail)
        self.assertIn("does NOT discriminate", demo.__doc__)  # claude D2: the docstring no longer claims corroboration


class CodexChildMetaTests(CodexChildScenarioTests):
    """codex D1: a `wait`-only collab call; the child is found by session_meta.parent_thread_id, its calls in
    code-mode custom_tool_call exec scripts."""

    def child_rollout(self, child, script, output, parent_key="parent_thread_id", parent=SESSION):
        path = Path(self.driver.facts["codex_home"]) / "sessions" / "2026" / "09" / "30" / f"rollout-2026-09-30T00-00-01-{child}.jsonl"
        path.parent.mkdir(parents=True, exist_ok=True)
        self.emit({"type": "session_meta", "payload": {"id": child, "cwd": str(self.driver.project), parent_key: parent}}, path)
        self.emit({"type": "response_item", "payload": {"type": "custom_tool_call", "name": "exec", "call_id": "cc_1", "input": script}}, path)
        self.emit({"type": "response_item", "payload": {"type": "custom_tool_call_output", "call_id": "cc_1",
                                                        "output": [{"type": "input_text", "text": output}]}}, path)
        return path

    def wait_only(self):
        self.item({"id": "item_w", "type": "collab_tool_call", "tool": "wait", "sender_thread_id": SESSION,
                   "receiver_thread_ids": [], "status": "completed"})

    def test_child_found_by_parent_thread_id_completes_the_sidechain(self):
        self.settle()
        self.wait_only()
        self.child_rollout("child-9", 'const r = await tools.exec_command({cmd: "herdr-threads pending-receipts", yield_time_ms: 1000});', "[]")
        self.root_ack()
        status, detail, _ = self.verify()
        self.assertEqual(status, demo.PASS, detail)
        self.assertEqual(self.driver.s_child_read("initial")[0], demo.PASS)
        self.assertEqual(self.driver.s_child_no_ack("initial")[0], demo.PASS)
        record = json.loads((self.driver.ev / "child-sidechain-initial.json").read_text())
        self.assertEqual(record["discovered"][0]["thread"], "child-9")

    def test_forked_from_id_also_names_the_parent(self):
        self.settle()
        self.wait_only()
        self.child_rollout("child-8", "tools.exec_command({cmd: 'herdr-threads ack msg-1'})", "Process exited with code 0",
                           parent_key="forked_from_id")
        self.root_ack()
        self.verify()
        self.assertEqual(self.driver.s_child_no_ack("initial")[0], demo.FAIL)

    def test_other_roots_children_are_ignored(self):
        self.settle()
        self.wait_only()
        self.child_rollout("child-7", 'tools.exec_command({cmd: "herdr-threads inbox"})', "{}", parent="someone-else")
        self.root_ack()
        self.verify()
        self.assertEqual(self.driver.s_child_no_ack("initial")[0], demo.UNVERIFIED)

    def test_exec_script_commands_decodes_both_quotes(self):
        script = 'a = tools.exec_command({cmd: "herdr-threads body \\"m\\""}); b = tools.exec_command({ cmd : \'echo it\\\'s\' })'
        self.assertEqual(demo.Driver.exec_script_commands(script), ['herdr-threads body "m"', "echo it's"])


class ExecScriptOpaqueCmdTests(unittest.TestCase):
    """W6-D1: a code-mode script mixing a literal `cmd:` with a template or variable `cmd:` keeps the latter visible."""

    def test_template_cmd_beside_literal_keeps_raw_script(self):
        script = 'tools.exec_command({cmd: "herdr-threads inbox"}); tools.exec_command({cmd: `herdr-threads ack ${m}`})'
        commands = demo.Driver.exec_script_commands(script)
        self.assertEqual(commands[0], "herdr-threads inbox")
        self.assertIn(script, commands)
        self.assertTrue(demo.mutating_call("\n".join(commands)))

    def test_variable_cmd_keeps_raw_script(self):
        script = 'const c = "herdr-threads accept t"; tools.exec_command({cmd: "echo hi"}); tools.exec_command({cmd: c})'
        self.assertTrue(demo.mutating_call("\n".join(demo.Driver.exec_script_commands(script))))

    def test_all_literal_script_is_unchanged(self):
        script = 'tools.exec_command({cmd: "herdr-threads body m"})'
        self.assertEqual(demo.Driver.exec_script_commands(script), ["herdr-threads body m"])


class UnverifiedReasonTests(ManifestTests):
    """codex D2: the manifest reason names what is unverified; SW2 has its own reason."""

    def test_sw2_only_is_warning_wake_unverified(self):
        self.passing_live_run()
        self.driver.record("SW2", "wake", demo.UNVERIFIED, "no wake")
        self.assertEqual(self.driver.manifest(), "UNSUPPORTED")
        self.assertEqual(json.loads((self.driver.ev / "manifest.json").read_text())["reason"], "warning_wake_unverified")

    def test_child_reason_outranks_the_others(self):
        self.passing_live_run()
        for sid in ("SW2", "S17H", "SC2"):
            self.driver.record(sid, sid, demo.UNVERIFIED, "x")
        self.driver.manifest()
        self.assertEqual(json.loads((self.driver.ev / "manifest.json").read_text())["reason"], "child_ack_unverified")

    def test_reason_mapping(self):
        self.assertEqual(demo.Driver.unverified_reason("S17H"), "hook_context_unverified")
        self.assertEqual(demo.Driver.unverified_reason("S18C"), "child_ack_unverified")
        self.assertEqual(demo.Driver.unverified_reason("initial"), "child_ack_unverified")
        self.assertEqual(demo.Driver.unverified_reason("S17"), "evidence_unverified")


class WarningOfferedTests(WarningScenarioTests):
    def test_warning_offered_at_check_in_is_not_exercised(self):
        # codex D2: offered_through_seq >= warning seq means no wake was owed; the claim was not tested.
        self.receipt(acked_at=9000, warning="w-1")
        self.warn("w-1", 10)
        self.driver.s_warning_event()
        self.db.execute("INSERT INTO warning_offer(seat_id,binding_generation,execution_id,offered_through_seq) VALUES (?,1,'e',10)", (SEAT,))
        self.db.commit()
        status, detail, _ = self.driver.s_warning_wake()
        self.assertEqual(status, demo.NOT_EXERCISED, detail)


class TuiWarningTests(ScenarioBase):
    """tui D1: under a Claude TUI the short deadline goes on a second message sent once the agent is idle."""
    SCENARIO = "warning"

    def setUp(self):
        super().setUp()
        self.driver.args.mode = "tui"
        self.sent = []
        self.driver.facts.update({"coordinator_seat": COORD, "coordinator_pane": "w1:p2", "agent_pane": "w1:p3"})
        self.driver.ht = lambda *argv, **kw: (self.sent.append(argv), (0, json.dumps({"message_id": f"m{len(self.sent)}"}), ""))[1]

    def deadline(self, index):
        return self.sent[index][self.sent[index].index("--deadline") + 1]

    def test_initial_handoff_keeps_the_long_deadline_under_tui(self):
        self.assertTrue(self.driver.warning_after_idle())
        self.driver.send("initial")
        self.assertEqual(self.deadline(0), "900")

    def test_print_mode_keeps_the_short_deadline_on_the_handoff(self):
        self.driver.args.mode = "print"
        self.assertFalse(self.driver.warning_after_idle())
        self.driver.send("initial")
        self.assertEqual(self.deadline(0), "5")

    def test_idle_send_uses_the_short_deadline_and_stays_out_of_the_handoffs(self):
        self.driver.herdr = lambda *argv, tag, timeout=30: (0, {"status": "idle"}, "", "")
        self.driver.capture_pane = lambda phase: None
        self.driver.args.warning_deadline = 0
        waits = []
        clock = [0.0]

        def sleep(seconds):
            waits.append(seconds)
            clock[0] += seconds
        orig = demo.time.sleep, demo.time.monotonic
        demo.time.sleep, demo.time.monotonic = sleep, lambda: clock[0]
        try:
            status, detail, _ = self.driver.s_warning_idle_send()
        finally:
            demo.time.sleep, demo.time.monotonic = orig
        self.assertEqual(sum(waits), 60)  # bounded poll: deadline 0 + 60 s, then no settle wait (nothing covered)
        self.assertEqual(status, demo.PASS, detail)
        self.assertEqual(self.deadline(0), "0")
        self.assertEqual(self.driver.facts["warning_message"], "m1")
        self.assertNotIn("warning", self.driver.facts["messages"])
        self.assertEqual(self.driver.warning_message("warning"), "m1")
        self.assertIn("not observed", detail)

    def test_done_agent_awaits_input_and_receives_the_warning(self):
        # Kills (tui-2 SW0): waiting for `idle` alone; Herdr reports a finished Claude TUI turn as `done`.
        calls = []

        def herdr(*argv, tag, timeout=30):
            calls.append(argv)
            return 0, {"result": {"agent": {"agent": "claude", "agent_status": "done"}}}, "", ""
        self.driver.herdr = herdr
        self.driver.capture_pane = lambda phase: None
        self.driver.args.warning_deadline = 0
        orig = demo.time.sleep, demo.time.monotonic
        clock = [0.0]
        demo.time.sleep = lambda seconds: clock.__setitem__(0, clock[0] + seconds)
        demo.time.monotonic = lambda: clock[0]
        try:
            status, detail, _ = self.driver.s_warning_idle_send()
        finally:
            demo.time.sleep, demo.time.monotonic = orig
        self.assertEqual(status, demo.PASS, detail)
        wait = calls[0]
        self.assertEqual([wait[i + 1] for i, arg in enumerate(wait) if arg == "--until"], ["idle", "done"])
        self.assertEqual(self.deadline(0), "0")

    def test_agent_never_idle_is_not_exercised(self):
        self.driver.herdr = lambda *argv, tag, timeout=30: (0, {"status": "working"}, "", "")
        self.assertEqual(self.driver.s_warning_idle_send()[0], demo.NOT_EXERCISED)
        self.assertEqual(self.sent, [])

    def test_dry_run_is_blocked(self):
        self.driver.dry = True
        self.assertRaises(demo.Blocked, self.driver.s_warning_idle_send)


class ManagedMatrixTests(unittest.TestCase):
    """codex D3/D4/D6, claude D3: managed argv, the owned pane shell and the recovered launch outcome."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)

    def driver_for(self, harness):
        driver = demo.Driver(args(self.tmp.name, harness=harness, launch="managed"))
        self.addCleanup(driver.cmdlog.close)
        driver.facts["agent_pane"] = "w1:p9"
        return driver

    def test_codex_managed_argv_drops_hook_pairs_only(self):
        # The owned hooks are on disk in the scratch CODEX_HOME; launch refuses caller `hooks.*` overrides.
        driver = self.driver_for("codex")
        other = ["-c", "sandbox_workspace_write.writable_roots=[\"/s\"]"]
        driver.facts["native_argv"] = {"initial": ["--no-daemon", "exec", "--json", "-c", "hooks.SessionStart=[x]",
                                                   *other, "-c", "hooks.Extra=1", "-c", "model_reasoning_effort=low",
                                                   "PROMPT"]}
        argv = driver.managed_launch_argv("initial")
        tail = argv[argv.index("--") + 1:]
        self.assertEqual(tail, ["--no-daemon", "exec", "--json", *other, "-c", "model_reasoning_effort=low", "PROMPT"])

    def test_claude_managed_argv_is_untouched(self):
        driver = self.driver_for("claude")
        driver.facts["native_argv"] = {"initial": ["-c", "hooks.x=1", "-p", "PROMPT"]}
        argv = driver.managed_launch_argv("initial")
        self.assertEqual(argv[argv.index("--") + 1:], ["-c", "hooks.x=1", "-p", "PROMPT"])

    def test_path_export_drops_shell_functions_and_aliases(self):
        line = self.driver_for("codex").managed_path_export()
        for name in ("codex", "claude"):
            self.assertIn(f"unset -f {name} 2>/dev/null", line)
            self.assertIn(f"unalias {name} 2>/dev/null", line)
        self.assertTrue(line.endswith("; true"))
        import subprocess
        rc = subprocess.run(["zsh", "-c", "codex() { echo shadow; }; alias claude=echo; " + line + "; whence -w codex claude; exit 0"],
                            capture_output=True, text=True) if Path("/bin/zsh").exists() else None
        if rc is not None:
            self.assertNotIn("function", rc.stdout)
            self.assertNotIn("alias", rc.stdout)

    def test_claude_outcome_from_pane_capture(self):
        driver = self.driver_for("claude")
        result = {"type": "result", "is_error": False, "total_cost_usd": 0.012, "usage": {"input_tokens": 10, "output_tokens": 5}}
        (driver.ev / "pane-initial.txt").write_text("% claude -p ...\n" + "user@host % " + json.dumps(result) + "\n")
        outcome = driver.managed_outcome("initial", None)
        self.assertEqual(outcome["usage_source"], "pane capture")
        self.assertEqual(outcome["usage"].get("input_tokens"), 10)

    def test_claude_usage_falls_back_to_the_session_transcript(self):
        driver = self.driver_for("claude")
        (driver.ev / "pane-initial.txt").write_text("garbled tui text\n")
        transcript = Path(self.tmp.name) / "session.jsonl"
        events = [{"type": "assistant", "message": {"id": "a", "usage": {"input_tokens": 3, "output_tokens": 1}}},
                  {"type": "assistant", "message": {"id": "a", "usage": {"input_tokens": 3, "output_tokens": 1}}},
                  {"type": "assistant", "message": {"id": "b", "usage": {"input_tokens": 4, "output_tokens": 2}}},
                  {"type": "assistant", "isSidechain": True, "message": {"id": "c", "usage": {"input_tokens": 99}}}]
        transcript.write_text("".join(json.dumps(e) + "\n" for e in events))
        outcome = driver.managed_outcome("initial", transcript)
        self.assertEqual(outcome["usage"], {"input_tokens": 7, "output_tokens": 3})
        self.assertEqual(outcome["usage_source"], "session transcript")

    def test_codex_outcome_from_rollout_and_root_found_by_cwd(self):
        driver = self.driver_for("codex")
        home = Path(self.tmp.name) / "codex-home"
        driver.facts["codex_home"] = str(home)
        day = home / "sessions" / "2026" / "09" / "30"
        day.mkdir(parents=True)
        root = day / "rollout-2026-09-30T00-00-00-root.jsonl"
        root.write_text(json.dumps({"type": "session_meta", "payload": {"id": "root-1", "cwd": str(driver.project)}}) + "\n"
                        + json.dumps({"type": "event_msg", "payload": {"type": "token_count", "info": {"total_token_usage":
                                     {"input_tokens": 20, "output_tokens": 4}}}}) + "\n")
        (day / "rollout-2026-09-30T00-00-01-child.jsonl").write_text(json.dumps(
            {"type": "session_meta", "payload": {"id": "child-1", "cwd": str(driver.project), "parent_thread_id": "root-1"}}) + "\n")
        (day / "rollout-2026-09-30T00-00-02-other.jsonl").write_text(json.dumps(
            {"type": "session_meta", "payload": {"id": "other", "cwd": "/elsewhere"}}) + "\n")
        found = driver.codex_rollout_since(None)
        self.assertEqual(found, root)
        (driver.ev / "pane-initial.txt").write_text("codex output without json\n")
        outcome = driver.managed_outcome("initial", found)
        self.assertEqual(outcome["thread_id"], "root-1")
        self.assertEqual(outcome["usage"], {"input_tokens": 20, "output_tokens": 4})
        self.assertEqual(outcome["usage_source"], "session rollout")

    @staticmethod
    def codex_turn(tokens, failure=None):
        events = [{"type": "thread.started", "thread_id": f"t-{tokens}"}]
        if failure:
            events.append({"type": "turn.failed", "error": {"message": failure}})
        events.append({"type": "turn.completed", "usage": {"input_tokens": tokens, "output_tokens": 1}})
        return "".join("user@host % " + json.dumps(e) + "\n" for e in events)

    def managed_capture(self, driver, phases):
        """Simulate a reused agent pane: each phase's typed shell line, its printed marker, then its own output; the
        capture of phase N holds every earlier phase too."""
        screen = ""
        for phase, output in phases:
            marker = driver.phase_marker(phase)
            screen += "user@host % " + driver.managed_path_export(marker) + "\n" + marker + "\n" + output
            (driver.ev / f"pane-{phase}.txt").write_text(screen)
        return screen

    def test_marker_is_printed_by_the_pane_shell_and_not_equal_to_the_typed_line(self):
        driver = self.driver_for("codex")
        marker = driver.phase_marker("restart")
        line = driver.managed_path_export(marker)
        self.assertTrue(line.endswith("; true"))
        self.assertNotEqual(line.strip(), marker)
        import subprocess
        out = subprocess.run(["sh", "-c", line], capture_output=True, text=True).stdout
        self.assertEqual(out.splitlines(), [marker])

    def test_codex_later_phase_counts_only_its_own_turns(self):
        # claude D3 / codex D6 (wave 6): the pane is reused, so a restart/resume capture still holds earlier phases.
        driver = self.driver_for("codex")
        self.managed_capture(driver, [("initial", self.codex_turn(100, failure="boom earlier")),
                                      ("restart", self.codex_turn(20)), ("resume", self.codex_turn(3))])
        initial = driver.managed_outcome("initial", None)
        self.assertEqual(initial["usage"], {"input_tokens": 100, "output_tokens": 1})
        restart = driver.managed_outcome("restart", None)
        self.assertEqual(restart["usage"], {"input_tokens": 20, "output_tokens": 1})
        self.assertEqual(restart["failures"], [])
        self.assertEqual(restart["thread_id"], "t-20")
        self.assertEqual(restart["pane_span"], "capture lines after this phase's marker")
        resume = driver.managed_outcome("resume", None)
        self.assertEqual(resume["usage"], {"input_tokens": 3, "output_tokens": 1})

    def test_claude_phase_without_its_own_result_does_not_inherit_the_previous_one(self):
        driver = self.driver_for("claude")
        result = {"type": "result", "is_error": False, "total_cost_usd": 0.5, "usage": {"input_tokens": 50}}
        self.managed_capture(driver, [("initial", "user@host % " + json.dumps(result) + "\n"),
                                      ("restart", "claude crashed before printing a result\n")])
        restart = driver.managed_outcome("restart", None)
        self.assertEqual(restart["usage"], {})
        self.assertIsNone(restart["cost_usd"])

    def test_short_capture_without_the_phase_marker_is_not_attributed(self):
        driver = self.driver_for("codex")
        self.managed_capture(driver, [("initial", self.codex_turn(100))])
        driver.phase_marker("restart")
        (driver.ev / "pane-restart.txt").write_text((driver.ev / "pane-initial.txt").read_text())
        restart = driver.managed_outcome("restart", None)
        self.assertEqual(restart["usage"], {})
        self.assertTrue(restart["pane_span"].startswith("none"))

    def test_full_capture_without_the_marker_postdates_it(self):
        driver = self.driver_for("codex")
        self.managed_capture(driver, [("initial", self.codex_turn(100))])
        driver.phase_marker("restart")
        filler = "".join(f"noise {i}\n" for i in range(demo.PANE_CAPTURE_LINES))
        (driver.ev / "pane-restart.txt").write_text(filler + self.codex_turn(7))
        restart = driver.managed_outcome("restart", None)
        self.assertEqual(restart["usage"], {"input_tokens": 7, "output_tokens": 1})

    def test_codex_resume_rollout_usage_is_a_per_phase_delta(self):
        driver = self.driver_for("codex")
        rollout = Path(self.tmp.name) / "rollout-resume.jsonl"

        def write(*totals):
            rollout.write_text(json.dumps({"type": "session_meta", "payload": {"id": "sess-1"}}) + "\n" + "".join(
                json.dumps({"type": "event_msg", "payload": {"type": "token_count", "info": {"total_token_usage":
                           {"input_tokens": t, "output_tokens": t // 10}}}}) + "\n" for t in totals))

        for phase in ("initial", "resume"):
            (driver.ev / f"pane-{phase}.txt").write_text("no json here\n")
        write(100)
        first = driver.managed_outcome("initial", rollout)
        self.assertEqual(first["usage"], {"input_tokens": 100, "output_tokens": 10})
        write(100, 250)
        second = driver.managed_outcome("resume", rollout)
        self.assertEqual(second["usage"], {"input_tokens": 150, "output_tokens": 15})
        self.assertEqual(second["usage_basis"], "per-phase delta of the session's cumulative token_count")

    def test_claude_resumed_transcript_skips_messages_counted_earlier(self):
        driver = self.driver_for("claude")
        transcript = Path(self.tmp.name) / "session-resume.jsonl"
        for phase in ("initial", "resume"):
            (driver.ev / f"pane-{phase}.txt").write_text("no json here\n")
        first = [{"type": "assistant", "message": {"id": "a", "usage": {"input_tokens": 3}}}]
        transcript.write_text("".join(json.dumps(e) + "\n" for e in first))
        self.assertEqual(driver.managed_outcome("initial", transcript)["usage"], {"input_tokens": 3})
        later = first + [{"type": "assistant", "message": {"id": "b", "usage": {"input_tokens": 5}}}]
        transcript.write_text("".join(json.dumps(e) + "\n" for e in later))
        self.assertEqual(driver.managed_outcome("resume", transcript)["usage"], {"input_tokens": 5})


class CodexSkillsMessageTests(CodexRolloutTests):
    def test_skills_instructions_developer_message_is_not_a_delivery(self):
        # codex D7: Codex's own `<skills_instructions>` developer message names herdr-threads but is not a hook delivery.
        skills = f"<skills_instructions> herdr-threads skill: ack {MSG} in {THREAD} </skills_instructions>"
        self.line("developer", skills)
        self.line("user", demo.PROMPT)
        self.assertEqual(self.d.s_hook_context("initial")[0], demo.FAIL)
        self.rollout.unlink()
        self.line("developer", skills)
        self.line("developer", f"{demo.HOOK_PREAMBLE} ... pending {MSG} in {THREAD}")
        self.line("user", demo.PROMPT)
        status, detail, _ = self.d.s_hook_context("initial")
        self.assertEqual(status, demo.PASS, detail)
        self.assertEqual(self.d.facts["hook_context"]["initial"]["session_start"], 1)
        record = json.loads((self.d.ev / "hook-context-initial.json").read_text())
        self.assertEqual(record["other_developer_messages"], 1)


class TuiTrustHashTests(TuiTrustRecordTests):
    def test_project_paths_are_hashed_in_the_evidence(self):
        # tui D2: ~/.claude.json project keys are the user's directory names; the evidence never lists them.
        self.driver.s_tui_trust_before()
        self.driver.s_tui_trust_after()
        for name in ("claude-json-before.json", "claude-json-after.json", "claude-json-diff.json"):
            self.assertNotIn('"/other"', (self.driver.ev / name).read_text())
        before = json.loads((self.driver.ev / "claude-json-before.json").read_text())
        self.assertEqual(before["project_count"], 1)
        self.assertTrue(all(k.startswith("path-sha256:") for k in before["project_hashes"]))


def shlex_quote(text):
    import shlex
    return shlex.quote(text)


if __name__ == "__main__":
    unittest.main()
