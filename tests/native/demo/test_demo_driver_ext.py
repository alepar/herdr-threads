"""Offline checks for ht-4is.11.12: interactive Codex (TUI, /new), done-as-idle, and the lostprompt, blockedui and
children scenarios. Every verdict path is exercised against a real-schema SQLite fixture and stubbed Herdr/daemon
calls; no daemon, Herdr or model is involved.

Run: python3 -m unittest tests/native/demo/test_demo_driver_ext.py
"""

import contextlib
import importlib.util
import io
import json
from pathlib import Path
import shlex
import sys
import tempfile
import unittest
from unittest import mock

HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("demo_verify_base", HERE / "test_demo_verify.py")
base = importlib.util.module_from_spec(_spec)
sys.modules["demo_verify_base"] = base
_spec.loader.exec_module(base)
demo = base.demo
SEAT, COORD, THREAD, MSG, SESSION = base.SEAT, base.COORD, base.THREAD, base.MSG, base.SESSION


class Clock:
    """Deterministic time for bounded polls: sleep advances monotonic."""

    def __init__(self):
        self.now = 0.0
        self.slept = 0.0

    def sleep(self, seconds):
        self.now += seconds
        self.slept += seconds

    def monotonic(self):
        return self.now


@contextlib.contextmanager
def fake_time():
    clock = Clock()
    with mock.patch.object(demo.time, "sleep", clock.sleep), mock.patch.object(demo.time, "monotonic", clock.monotonic):
        yield clock


def health(state):
    return json.dumps({"data": {"health": {"host": {"safe_prompt": state}}}})


class ExtArgsTests(base.ArgsTests):
    def test_codex_tui_is_accepted_with_spend_opt_in_only(self):
        self.parse("--harness", "codex", "--mode", "tui", "--allow-uncapped-spend")
        self.parse("--harness", "codex", "--mode", "tui", "--dry-run")
        self.rejected("--harness", "codex", "--mode", "tui")

    def test_codex_tui_hook_trust_bypass_only_under_private_tmp(self):
        self.rejected("--harness", "codex", "--mode", "tui", "--allow-uncapped-spend", "--run-root", "/var/tmp/x")
        self.parse("--harness", "codex", "--mode", "tui", "--allow-uncapped-spend", "--run-root", "/private/tmp/x")

    def test_lostprompt_and_blockedui_need_an_interactive_agent(self):
        for name in ("lostprompt", "blockedui"):
            self.rejected("--harness", "claude", "--scenario", name)
            self.rejected("--harness", "codex", "--allow-uncapped-spend", "--scenario", name)
            self.parse("--harness", "codex", "--mode", "tui", "--allow-uncapped-spend", "--scenario", name)
            self.parse("--harness", "claude", "--mode", "tui", "--tui-accept-trust", "--allow-uncapped-spend", "--scenario", name)

    def test_prompt_scenarios_do_not_mix(self):
        self.rejected("--harness", "codex", "--mode", "tui", "--allow-uncapped-spend", "--scenario", "lostprompt,children")
        self.rejected("--harness", "claude", "--scenario", "child,children")
        self.parse("--harness", "claude", "--scenario", "children")

    def test_default_codex_tui_run_root_is_private_tmp(self):
        with mock.patch.object(demo.NativeFixture, "create") as create, mock.patch.object(Path, "mkdir"), \
                mock.patch("builtins.open", mock.mock_open()):
            create.return_value = mock.Mock(root=Path("/private/tmp/x/r"), evidence_dir=Path("/private/tmp/x/r/evidence"))
            try:
                demo.Driver(base.args("/unused", harness="codex", mode="tui", run_root=None))
            except Exception:  # noqa: BLE001 - only the chosen base directory matters here
                pass
            self.assertTrue(str(create.call_args[0][0]).startswith("/private/tmp/ht-native-demo/"))


class CodexTuiCommandTests(unittest.TestCase):
    def setUp(self):
        self.host = base.FakeHostDriver(self, harness="codex", mode="tui")
        self.d = self.host.driver
        self.d.facts["codex_hook_args"] = ["-c", "hooks.SessionStart=[x]", "-c", "network_proxy.unix_sockets=[s]"]

    def argv(self, phase):
        shell, rcfile, transcript = self.d.launch_command(phase)
        self.assertIsNone(transcript)
        self.assertIn(f"echo $? >{shlex.quote(str(rcfile))}", shell)
        return self.d.facts["native_argv"][phase], shell

    def test_interactive_argv_keeps_hooks_and_allowance_without_exec_or_bypass(self):
        argv, shell = self.argv("initial")
        self.assertEqual(argv[0], "--no-daemon")
        self.assertNotIn("exec", argv)
        for flag in ("--json", "--ignore-user-config", "--skip-git-repo-check", "--dangerously-bypass-approvals-and-sandbox", "--yolo"):
            self.assertNotIn(flag, argv)
        self.assertEqual(argv[argv.index("-a") + 1], "on-request")  # explicit approval policy and sandbox (folder trust is answered separately, scratch only)
        self.assertEqual(argv[argv.index("-s") + 1], "workspace-write")
        self.assertIn("--dangerously-bypass-hook-trust", argv)
        self.assertIn("hooks.SessionStart=[x]", argv)
        self.assertIn("network_proxy.unix_sockets=[s]", argv)
        self.assertNotIn(demo.PROMPT, argv)  # the prompt is submitted through Herdr, never on the command line
        self.assertIn("aisw workspace check --tool codex", shell)

    def test_resume_uses_interactive_resume_with_the_known_thread(self):
        self.d.facts["codex_session"] = "thread-9"
        argv, _ = self.argv("resume")
        self.assertEqual(argv[1], "resume")
        self.assertEqual(argv[-1], "thread-9")
        self.assertEqual(self.d.facts["sessions"]["resume"], "thread-9")

    def test_clear_and_restart_discover_their_thread(self):
        self.d.facts["sessions"] = {"initial": "thread-1"}
        self.argv("clear")
        self.assertIsNone(self.d.facts["sessions"]["clear"])


class CodexTuiLaunchTests(unittest.TestCase):
    """The Codex pane flow: nothing is typed into any trust/approval screen; /new clears."""

    def launch(self, state, screens, phase="initial", scenario="base"):
        host = base.FakeHostDriver(self, harness="codex", mode="tui", scenario=scenario)
        host.wait_state, host.screens = state, list(screens)
        host.driver.facts.update({"agent_pane": "w9:p1", "phase_started_utc": {phase: "2026-09-30T00:00:00+00:00"}})
        with contextlib.redirect_stdout(io.StringIO()), fake_time():
            result = host.driver.launch_tui(phase, "true", host.driver.ev / "x.rc")
        return host, result

    @staticmethod
    def typed(host):
        return [c for c in host.calls if c[1].startswith(("tui:prompt", "tui:accept-trust", "tui:trust-select")) or "send-keys" in c[2]]

    def test_codex_trust_screen_fails_with_nothing_typed(self):
        for screen in ("Do you trust the files in this folder?\n> 1. Yes, continue\n  2. No, quit",
                       "These hooks are not trusted yet. Review hooks"):
            host, (status, detail, _) = self.launch("blocked", [screen])
            self.assertEqual(status, demo.FAIL)
            self.assertIn("trust", detail)
            self.assertEqual(self.typed(host), [])

    def launch_trust(self, accept, screens, tmp_dir="/private/tmp", project=None):
        host = base.FakeHostDriver(self, harness="codex", mode="tui", tui_accept_trust=accept, tmp_dir=tmp_dir)
        if project is not None:
            host.driver.project = project
        host.wait_state, host.screens = "blocked", list(screens)
        host.driver.facts.update({"agent_pane": "w9:p1", "phase_started_utc": {"initial": "2026-09-30T00:00:00+00:00"}})
        with contextlib.redirect_stdout(io.StringIO()), fake_time():
            return host, host.driver.launch_tui("initial", "true", host.driver.ev / "x.rc")

    FOLDER = "Trust this folder? Codex can read, edit, and run files here.\n› 1. Trust and continue\n  2. Quit\n enter continue"

    def test_approved_codex_folder_trust_is_accepted_then_prompt_sent(self):
        # User decision 2026-09-30: accept Codex folder trust for /private/tmp scratch only (--tui-accept-trust).
        host, (status, detail, _) = self.launch_trust(True, [self.FOLDER, "OpenAI Codex", "? for shortcuts", "done"])
        self.assertEqual(status, demo.PASS, detail)
        tags = [c[1] for c in host.calls]
        self.assertLess(tags.index("tui:accept-codex-trust:initial"), tags.index("tui:prompt:initial"))

    def test_codex_folder_trust_outside_scratch_root_is_refused(self):
        # tmp_dir=None puts the run root under the platform temp dir (/var/folders on macOS), outside /private/tmp.
        if str(Path(tempfile.gettempdir()).resolve()).startswith("/private/tmp/"):
            self.skipTest("gettempdir is under /private/tmp here")
        host, (status, detail, _) = self.launch_trust(True, [self.FOLDER], tmp_dir=None)
        self.assertEqual(status, demo.FAIL)
        self.assertIn("outside", detail)
        self.assertIn("nothing typed", detail)
        self.assertNotIn("tui:accept-codex-trust:initial", [c[1] for c in host.calls])
        self.assertEqual(self.typed(host), [])

    def test_codex_folder_trust_project_moved_outside_root_is_refused(self):
        # The check uses the live project path, not --run-root.
        host, (status, detail, _) = self.launch_trust(True, [self.FOLDER], project=Path("/private/tmp"))
        self.assertEqual(status, demo.FAIL)
        self.assertIn("outside the run root", detail)
        self.assertEqual(self.typed(host), [])

    def test_codex_folder_trust_acceptance_is_recorded(self):
        host = base.FakeHostDriver(self, harness="codex", mode="tui", tui_accept_trust=True, tmp_dir="/private/tmp")
        d = host.driver
        host.wait_state, host.screens = "blocked", [self.FOLDER, "OpenAI Codex", "? for shortcuts", "done"]
        d.facts.update({"agent_pane": "w9:p1", "phase_started_utc": {"initial": "2026-09-30T00:00:00+00:00"}})
        d.codex_tui_command("initial", "", "", d.ev / "x.rc")
        with contextlib.redirect_stdout(io.StringIO()), fake_time():
            status, detail, _ = d.launch_tui("initial", "true", d.ev / "x.rc")
        self.assertEqual(status, demo.PASS, detail)
        d.codex_tui_command("restart", "", "", d.ev / "y.rc")  # rebuilt per phase: the list must survive
        record = d.facts["codex_tui"]["folder_trust_acceptances"]
        self.assertEqual(len(record), 1)
        self.assertEqual(record[0]["phase"], "initial")
        self.assertEqual(record[0]["project"], str(d.project.resolve()))
        self.assertEqual(record[0]["run_root"], str(d.root.resolve()))
        self.assertTrue(record[0]["codex_config"].endswith("config.toml"))
        self.assertIsInstance(record[0]["utc"], str)
        self.assertNotIn("folder_trust_avoided_by", d.facts["codex_tui"])
        self.assertIn("accepted on screen", d.facts["codex_tui"]["folder_trust"])

    def test_codex_tui_record_without_flag_says_not_approved(self):
        host = base.FakeHostDriver(self, harness="codex", mode="tui")
        host.driver.codex_tui_command("initial", "", "", host.driver.ev / "x.rc")
        self.assertIn("not approved", host.driver.facts["codex_tui"]["folder_trust"])

    def test_codex_folder_trust_without_flag_still_fails(self):
        host, (status, _, _) = self.launch_trust(False, [self.FOLDER])
        self.assertEqual(status, demo.FAIL)
        self.assertEqual(self.typed(host), [])

    def test_codex_folder_trust_with_quit_selected_types_nothing(self):
        host, (status, _, _) = self.launch_trust(True, ["Trust this folder?\n  1. Trust and continue\n› 2. Quit"])
        self.assertEqual(status, demo.FAIL)
        self.assertNotIn("tui:accept-codex-trust:initial", [c[1] for c in host.calls])

    def test_trust_screen_fails_even_when_herdr_says_idle(self):
        host, (status, _, _) = self.launch("idle", ["Do you trust the files in this folder?"])
        self.assertEqual(status, demo.FAIL)
        self.assertEqual(self.typed(host), [])

    def test_other_blocking_dialog_fails_with_nothing_typed(self):
        host, (status, detail, _) = self.launch("blocked", ["Would you like to run the following command?"])
        self.assertEqual(status, demo.FAIL)
        self.assertIn("nothing typed", detail)
        self.assertEqual(self.typed(host), [])

    def test_ready_codex_gets_the_prompt_and_waits_accept_done(self):
        host, (status, detail, _) = self.launch("done", ["OpenAI Codex", "? for shortcuts   100% context left", "done"])
        self.assertEqual(status, demo.PASS, detail)
        wait = next(c for c in host.calls if c[1].startswith("tui:wait-ready"))
        self.assertIn("done", wait[2])
        prompts = [c for c in host.calls if c[1] == "tui:prompt:initial"]
        self.assertEqual(len(prompts), 1)
        self.assertIn(demo.PROMPT, prompts[0][2])
        self.assertIn("launch_outcome", host.driver.facts)

    def test_clear_phase_types_new(self):
        host, (status, detail, _) = self.launch("idle", ["? for shortcuts", "done"], phase="clear")
        clear = next(c for c in host.calls if c[1] == "tui:clear")
        self.assertEqual(clear[2][3], "/new")
        self.assertEqual(status, demo.PASS, detail)

    def test_usage_limit_on_screen_is_environment(self):
        host = base.FakeHostDriver(self, harness="codex", mode="tui")
        host.wait_state, host.screens = "idle", ["ready", "? for shortcuts", "■ You've hit your usage limit. Try again later."]
        host.driver.facts["agent_pane"] = "w9:p1"
        real = host.herdr

        def herdr(*argv, tag, timeout=30):
            if tag.startswith("tui:prompt"):
                return 1, None, "", "agent_prompt_timeout"
            return real(*argv, tag=tag, timeout=timeout)
        host.driver.herdr = herdr
        with contextlib.redirect_stdout(io.StringIO()), fake_time():
            status, detail, _ = host.driver.launch_tui("initial", "true", host.driver.ev / "x.rc")
        self.assertEqual(status, demo.ENVIRONMENT, detail)

    def test_lostprompt_types_no_prompt(self):
        host = base.FakeHostDriver(self, harness="codex", mode="tui", scenario="lostprompt")
        host.wait_state, host.screens = "idle", ["ready", "? for shortcuts"]
        d = host.driver
        d.facts.update({"agent_pane": "w9:p1", "agent_seat": SEAT, "messages": {"initial": MSG}})
        d.receipts_for = lambda seat: []
        d.wake_row = lambda: {}
        d.ht = lambda *argv, tag, **kw: (0, health("unsupported"), "")
        with contextlib.redirect_stdout(io.StringIO()), fake_time() as clock:
            status, detail, _ = d.launch_tui("initial", "true", d.ev / "x.rc")
        self.assertEqual(status, demo.PASS, detail)
        self.assertFalse([c for c in host.calls if c[1].startswith("tui:prompt")])
        self.assertLessEqual(clock.slept, demo.LOSTPROMPT_UNSUPPORTED_WAIT_S + 40)
        self.assertEqual(d.facts["lostprompt"]["safe_prompt"], "unsupported")
        self.assertEqual(d.facts["lostprompt"]["bound_s"], min(d.args.timeout, demo.LOSTPROMPT_UNSUPPORTED_WAIT_S))


class CodexTuiTranscriptTests(unittest.TestCase):
    def setUp(self):
        self.host = base.FakeHostDriver(self, harness="codex", mode="tui")
        self.d = self.host.driver
        self.d.project.mkdir(exist_ok=True)
        self.home = Path(self.host.tmp.name) / "codex-home"
        self.d.facts.update({"codex_home": str(self.home), "phase_started_utc": {"initial": "2000-01-01T00:00:00+00:00",
                                                                                "clear": "2000-01-01T00:00:00+00:00"}})

    def rollout(self, thread, parent=None):
        path = self.home / "sessions" / "2026" / "09" / "30" / f"rollout-2026-09-30T00-00-00-{thread}.jsonl"
        path.parent.mkdir(parents=True, exist_ok=True)
        meta = {"id": thread, "cwd": str(self.d.project)}
        if parent:
            meta["parent_thread_id"] = parent
        path.write_text(json.dumps({"type": "session_meta", "payload": meta}) + "\n")
        return path

    def test_initial_then_new_conversation_each_find_their_own_rollout(self):
        first = self.rollout("thread-a")
        self.rollout("child-x", parent="thread-a")
        self.assertEqual(self.d.tui_transcript("initial"), first)
        self.assertEqual(self.d.facts["sessions"]["initial"], "thread-a")
        second = self.rollout("thread-b")
        self.assertEqual(self.d.tui_transcript("clear"), second)
        self.assertEqual(self.d.facts["sessions"]["clear"], "thread-b")
        self.assertEqual(self.d.facts["codex_session"], "thread-b")

    def test_ambiguous_rollouts_are_not_guessed(self):
        self.rollout("thread-a")
        self.rollout("thread-b")
        self.assertIsNone(self.d.tui_transcript("initial"))


class DoneIsIdleTests(base.ScenarioBase):
    SCENARIO = "warning"

    def setUp(self):
        super().setUp()
        self.driver.args.mode = "tui"
        self.waits = []
        self.driver.facts.update({"coordinator_seat": COORD, "coordinator_pane": "w1:p2", "agent_pane": "w1:p3"})
        self.driver.ht = lambda *argv, **kw: (0, json.dumps({"message_id": "m1"}) if "send" in argv else health("supported"), "")
        self.driver.capture_pane = lambda phase: ""

    def test_done_state_counts_as_idle_for_sw0(self):
        # Kills: native-claude-tui-2 SW0 NOT_EXERCISED because Herdr reported the finished turn as `done`.
        def herdr(*argv, tag, timeout=30):
            self.waits.append(argv)
            return 0, {"status": "done"}, "", ""
        self.driver.herdr = herdr
        with fake_time():
            status, detail, _ = self.driver.s_warning_idle_send()
        self.assertEqual(status, demo.PASS, detail)
        self.assertIn("done", self.waits[0])
        self.assertEqual(self.driver.facts["warning_safe_prompt"], "supported")

    def test_working_state_is_still_not_exercised(self):
        self.driver.herdr = lambda *argv, tag, timeout=30: (0, {"status": "working"}, "", "")
        self.assertEqual(self.driver.s_warning_idle_send()[0], demo.NOT_EXERCISED)


class WarningUnsupportedTests(base.WarningScenarioTests):
    def test_no_wake_with_unsupported_safe_prompt_is_not_exercised(self):
        self.receipt(acked_at=9000, warning="w-1")
        self.warn("w-1", 10)
        self.driver.s_warning_event()
        self.driver.facts["warning_safe_prompt"] = "unsupported"
        status, detail, _ = self.driver.s_warning_wake()
        self.assertEqual(status, demo.NOT_EXERCISED, detail)
        self.assertIn("safe_prompt unsupported", detail)
        self.assertTrue(self.driver.facts["safe_wake_unsupported"])

    def test_observed_wake_still_passes_when_health_says_unsupported(self):
        # Evidence over claims: a covering reservation is judged on its rows, whatever health reports.
        self.receipt(acked_at=9000, warning="w-1")
        self.warn("w-1", 10)
        self.driver.s_warning_event()
        self.wake(10)
        self.driver.facts["warning_safe_prompt"] = "unsupported"
        self.assertEqual(self.driver.s_warning_wake()[0], demo.PASS)


class SafePromptHealthTests(unittest.TestCase):
    def test_parse(self):
        self.assertEqual(demo.Driver.parse_safe_prompt(health("unsupported")), "unsupported")
        self.assertEqual(demo.Driver.parse_safe_prompt(health("supported")), "supported")
        self.assertEqual(demo.Driver.parse_safe_prompt("not json"), "unknown")
        self.assertEqual(demo.Driver.parse_safe_prompt(json.dumps({"host": {}})), "unknown")

    def test_health_step_is_info_and_recorded(self):
        host = base.FakeHostDriver(self)
        host.driver.ht = lambda *argv, tag, **kw: (0, health("unsupported"), "")
        status, detail, _ = host.driver.s_safe_prompt_health()
        self.assertEqual(status, demo.INFO)
        self.assertIn("unsupported", detail)
        self.assertEqual(host.driver.facts["safe_prompt_observations"][-1]["safe_prompt"], "unsupported")


class WakeFixture(base.ScenarioBase):
    """ScenarioBase plus wake_work rows and pane captures."""

    def wake(self, reservation="r1", outcome=None, receipt_seq=5, invitation_seq=None):
        self.db.execute("DELETE FROM wake_work WHERE seat_id=?", (SEAT,))
        self.db.execute("INSERT INTO wake_work(seat_id,reason_bits,last_reservation_id,last_reservation_boot,last_reserved_at_utc,"
                        "last_receipt_seq,last_receipt_offset,last_invitation_seq,last_invitation_offset,last_outcome) "
                        "VALUES (?,?,?,?,?,?,?,?,?,?)",
                        (SEAT, 2, reservation, "b1" if reservation else None, 7000 if reservation else None, receipt_seq,
                         0 if receipt_seq else None, invitation_seq, 0 if invitation_seq else None, outcome))
        self.db.commit()

    def pane(self, name, text):
        (self.driver.ev / f"pane-{name}.txt").write_text(text)


class LostPromptTests(WakeFixture):
    SCENARIO = "lostprompt"

    def observe(self, safe_prompt="supported", before=None, after=None):
        self.driver.facts["lostprompt"] = {"safe_prompt": safe_prompt, "wake_before": before or {},
                                           "wake_after": after if after is not None else self.driver.wake_row(), "waited_s": 60}

    def test_wake_covering_the_receipt_passes_sl1(self):
        self.wake(outcome="submitted")
        self.observe()
        status, detail, _ = self.driver.s_lostprompt_wake()
        self.assertEqual(status, demo.PASS, detail)
        self.assertIn("receipt", detail)

    def test_reservation_older_than_the_launch_is_not_a_wake(self):
        self.wake(outcome="submitted")
        row = self.driver.wake_row()
        self.observe(before=row, after=row)
        self.assertEqual(self.driver.s_lostprompt_wake()[0], demo.FAIL)

    def test_reservation_covering_nothing_fails(self):
        self.wake(receipt_seq=None)
        self.observe()
        self.assertEqual(self.driver.s_lostprompt_wake()[0], demo.FAIL)

    def test_no_wake_with_unsupported_safe_prompt_is_not_exercised(self):
        self.observe(safe_prompt="unsupported", after={})
        status, detail, _ = self.driver.s_lostprompt_wake()
        self.assertEqual(status, demo.NOT_EXERCISED, detail)
        self.assertTrue(self.driver.facts["safe_wake_unsupported"])

    def test_no_wake_with_claimed_capability_fails(self):
        for claim in ("supported", "unknown"):
            self.observe(safe_prompt=claim, after={})
            self.assertEqual(self.driver.s_lostprompt_wake()[0], demo.FAIL)

    def test_marker_once_passes_sl2(self):
        self.observe(after={"last_outcome": "submitted"})
        self.pane("lostprompt", f"› {demo.WAKE_MARKER}; run herdr-threads inbox\n")
        self.assertEqual(self.driver.s_lostprompt_delivery()[0], demo.PASS)

    def test_repeated_marker_fails_sl2(self):
        self.observe(after={"last_outcome": "submitted"})
        self.pane("lostprompt", f"{demo.WAKE_MARKER}\n{demo.WAKE_MARKER}\n")
        self.assertEqual(self.driver.s_lostprompt_delivery()[0], demo.FAIL)

    def test_submitted_without_marker_is_unverified(self):
        self.observe(after={"last_outcome": "submitted"})
        self.pane("lostprompt", "nothing here")
        self.assertEqual(self.driver.s_lostprompt_delivery()[0], demo.UNVERIFIED)

    def test_undelivered_wake_under_unsupported_is_not_exercised(self):
        self.observe(safe_prompt="unsupported", after={"last_outcome": "unavailable"})
        self.assertEqual(self.driver.s_lostprompt_delivery()[0], demo.NOT_EXERCISED)

    def test_undelivered_wake_with_claimed_capability_fails(self):
        self.observe(after={"last_outcome": "unsafe"})
        self.assertEqual(self.driver.s_lostprompt_delivery()[0], demo.FAIL)

    def test_missing_ack_is_not_exercised_only_when_nothing_could_prompt(self):
        self.observe(safe_prompt="unsupported", after={})
        status, detail, _ = self.driver.s_lostprompt_verify(None)
        self.assertEqual(status, demo.NOT_EXERCISED, detail)
        self.observe(safe_prompt="supported", after={})
        self.assertEqual(self.driver.s_lostprompt_verify(None)[0], demo.FAIL)
        self.observe(safe_prompt="unsupported", after={"last_outcome": "submitted"})  # a wake was delivered anyway
        self.assertEqual(self.driver.s_lostprompt_verify(None)[0], demo.FAIL)

    def test_woken_model_ack_passes_s18(self):
        self.wake(outcome="submitted")
        self.observe()
        self.settle()
        self.root_ack()
        status, detail, _ = self.driver.s_lostprompt_verify(self.transcript)
        self.assertEqual(status, demo.PASS, detail)

    def test_manifest_is_unsupported_when_the_wake_is_unavailable(self):
        self.driver.facts["sessions"] = {"initial": SESSION}
        self.observe(safe_prompt="unsupported", after={})
        self.driver.record("S17", "launch", demo.PASS, "ok")
        self.driver.step("S18", "verify", lambda: self.driver.s_lostprompt_verify(None), needs=("S17",))
        self.driver.step("S18C", "child", lambda: self.driver.s_child_check("initial"), needs=("S18",))
        self.driver.step("SL1", "wake", self.driver.s_lostprompt_wake, needs=("S17",))
        self.driver.step("SL2", "delivery", self.driver.s_lostprompt_delivery, needs=("SL1",))
        self.assertEqual([s["status"] for s in self.driver.steps[1:]], [demo.NOT_EXERCISED] * 4)
        self.assertEqual(self.driver.manifest(), "UNSUPPORTED")
        self.assertEqual(json.loads((self.driver.ev / "manifest.json").read_text())["reason"], "safe_wake_unsupported")


class BlockedUiTests(WakeFixture):
    SCENARIO = "blockedui"
    DIALOG = "Bash command\n  mkdir /ht-blockedui-abc\nDo you want to proceed?\n❯ 1. Yes\n  2. No"

    def setUp(self):
        super().setUp()
        self.driver.args.mode = "tui"
        self.calls = []
        self.state = {"idle": "idle", "blocked": "blocked"}
        self.screens = [self.DIALOG]
        self.driver.facts.update({"coordinator_seat": COORD, "coordinator_pane": "w1:p2", "agent_pane": "w1:p3"})
        self.safe = "supported"
        self.driver.ht = self.ht
        self.driver.herdr = self.herdr

    def ht(self, *argv, tag, **kw):
        self.calls.append(("ht", tag, argv))
        if "send" in argv:
            return 0, json.dumps({"message_id": "msg-blocked"}), ""
        return 0, health(self.safe), ""

    def herdr(self, *argv, tag, timeout=30):
        self.calls.append(("herdr", tag, argv))
        if tag.startswith("herdr:pane-read"):
            return 0, None, self.screens.pop(0) if len(self.screens) > 1 else self.screens[0], ""
        if tag.startswith("tui:wait-idle"):
            return 0, {"status": self.state["idle"]}, "", ""
        if tag.startswith("tui:wait-blocked"):
            state = self.state["blocked"]
            return (0 if state == "blocked" else 1), ({"status": state} if state == "blocked" else None), "", "timeout"
        return 0, None, "", ""

    def enter(self):
        with fake_time():
            return self.driver.s_blockedui_enter()

    def answered(self):
        return [c for c in self.calls if c[0] == "herdr" and ("send-keys" in c[2] or c[1].startswith("tui:accept"))]

    def test_recognized_approval_ui_passes_su0_and_nothing_is_answered(self):
        status, detail, _ = self.enter()
        self.assertEqual(status, demo.PASS, detail)
        prompt = next(c for c in self.calls if c[1] == "tui:prompt:blockedui")
        self.assertNotIn("--wait", prompt[2])
        self.assertTrue(self.driver.facts["blockedui"]["target"].startswith("/ht-blockedui-"))
        self.assertEqual(self.answered(), [])

    def test_codex_approval_ui_is_recognized(self):
        self.screens = ["Would you like to run the following command?\n  $ mkdir /ht-blockedui-x\n› 1. Yes, proceed"]
        self.assertEqual(self.enter()[0], demo.PASS)

    def test_never_blocked_is_not_exercised(self):
        self.state["blocked"] = "idle"
        self.assertEqual(self.enter()[0], demo.NOT_EXERCISED)

    def test_blocked_without_recognized_ui_is_not_exercised(self):
        self.screens = ["Choose a theme"]
        self.assertEqual(self.enter()[0], demo.NOT_EXERCISED)

    def test_agent_not_idle_is_not_exercised_and_types_nothing(self):
        self.state["idle"] = "working"
        self.assertEqual(self.enter()[0], demo.NOT_EXERCISED)
        self.assertFalse([c for c in self.calls if c[1] == "tui:prompt:blockedui"])

    def test_dry_run_is_blocked(self):
        self.driver.dry = True
        self.assertRaises(demo.Blocked, self.driver.s_blockedui_enter)

    def queue(self, after_screen=None, after_wake=None):
        self.enter()
        if after_wake:
            after_wake()
        self.screens = [after_screen if after_screen is not None else self.DIALOG]
        with fake_time() as clock:
            status, detail, _ = self.driver.s_blockedui_queue()
        self.assertEqual(status, demo.PASS, detail)
        self.assertLessEqual(clock.slept, self.driver.args.warning_deadline + 60 + demo.WARNING_POLL_S)
        return self.driver.s_blockedui_no_injection()

    def test_queue_uses_the_short_deadline_and_stays_out_of_the_handoffs(self):
        self.queue()
        send = next(c for c in self.calls if c[0] == "ht" and "send" in c[2])
        self.assertEqual(send[2][send[2].index("--deadline") + 1], str(self.driver.args.warning_deadline))
        self.assertNotIn("blockedui", self.driver.facts["messages"])
        self.assertEqual(self.driver.facts["blockedui"]["message"], "msg-blocked")

    def test_nothing_injected_and_no_wake_passes(self):
        status, detail, _ = self.queue()
        self.assertEqual(status, demo.PASS, detail)
        self.assertIn("wake stays pending", detail)
        self.assertEqual(self.answered(), [])

    def test_refused_wake_attempt_passes(self):
        original = self.driver.wake_row
        rows = iter([{}, None])

        def wake_row():
            row = next(rows, None)
            if row is None:
                self.wake(reservation="r9", outcome="unsafe")
                return original()
            return row
        self.driver.wake_row = wake_row
        status, detail, _ = self.queue()
        self.assertEqual(status, demo.PASS, detail)
        self.assertIn("'unsafe' (not submitted)", detail)

    def test_submitted_wake_while_blocked_fails(self):
        rows = iter([{}, {"last_reservation_id": "r9", "last_outcome": "submitted"}])
        self.driver.wake_row = lambda: next(rows)
        status, detail, _ = self.queue()
        self.assertEqual(status, demo.FAIL)
        self.assertIn("submitted a prompt", detail)

    def test_marker_on_blocked_screen_fails(self):
        status, detail, _ = self.queue(after_screen=self.DIALOG + f"\n{demo.WAKE_MARKER}")
        self.assertEqual(status, demo.FAIL)
        self.assertIn("marker", detail)

    def test_ack_of_the_queued_message_fails(self):
        original = self.driver.receipts_for
        self.driver.receipts_for = lambda seat: [{"message_id": "msg-blocked", "state": "acked"}] + original(seat)
        status, detail, _ = self.queue()
        self.assertEqual(status, demo.FAIL)
        self.assertIn("ACKed", detail)

    def test_agent_leaving_the_ui_is_unverified(self):
        status, _, _ = self.queue(after_screen="Welcome back")
        self.assertEqual(status, demo.UNVERIFIED)

    def test_unsupported_safe_prompt_without_attempt_is_not_exercised(self):
        self.safe = "unsupported"
        status, detail, _ = self.queue()
        self.assertEqual(status, demo.NOT_EXERCISED, detail)
        self.assertTrue(self.driver.facts["safe_wake_unsupported"])


class ChildrenClaudeTests(base.ScenarioBase):
    SCENARIO = "children"

    def spawn_pair(self, concurrent=True):
        self.spawn("toolu_t1")
        if not concurrent:
            self.result("toolu_t1", "child one done")
        self.spawn("toolu_t2")
        if concurrent:
            self.result("toolu_t1", "child one done")
        self.result("toolu_t2", "child two done")

    def child_read(self, parent, call):
        self.call(call, "herdr-threads pending-receipts", parent=parent)
        self.result(call, f"{MSG} pending", parent=parent)

    def test_children_prompt_is_used(self):
        self.assertEqual(self.driver.prompt(), demo.CHILDREN_PROMPT)
        self.assertTrue(self.driver.delegates())

    def test_two_concurrent_reading_children_with_root_ack_pass(self):
        self.settle()
        self.spawn_pair()
        self.child_read("toolu_t1", "toolu_c1")
        self.child_read("toolu_t2", "toolu_c2")
        self.root_ack()
        status, detail, _ = self.verify()
        self.assertEqual(status, demo.PASS, detail)
        status, detail, _ = self.driver.s_children_concurrent()
        self.assertEqual(status, demo.PASS, detail)
        self.assertEqual(self.driver.s_child_no_ack("initial")[0], demo.PASS)
        self.assertEqual(self.driver.s_top_level_ack("initial")[0], demo.PASS)

    def test_sequential_children_are_not_exercised(self):
        self.settle()
        self.spawn_pair(concurrent=False)
        self.child_read("toolu_t1", "toolu_c1")
        self.child_read("toolu_t2", "toolu_c2")
        self.root_ack()
        self.verify()
        status, detail, _ = self.driver.s_children_concurrent()
        self.assertEqual(status, demo.NOT_EXERCISED, detail)
        self.assertIn("one after the other", detail)

    def test_one_child_is_not_exercised(self):
        self.settle()
        self.spawn("toolu_t1")
        self.child_read("toolu_t1", "toolu_c1")
        self.root_ack()
        self.verify()
        self.assertEqual(self.driver.s_children_concurrent()[0], demo.NOT_EXERCISED)

    def test_only_one_reading_child_is_not_exercised(self):
        self.settle()
        self.spawn_pair()
        self.child_read("toolu_t1", "toolu_c1")
        self.call("toolu_c2", "echo hi", parent="toolu_t2")
        self.root_ack()
        self.verify()
        status, detail, _ = self.driver.s_children_concurrent()
        self.assertEqual(status, demo.NOT_EXERCISED, detail)

    def test_successful_child_ack_fails_sk2(self):
        self.settle()
        self.spawn_pair()
        self.child_read("toolu_t1", "toolu_c1")
        self.call("toolu_c9", f"herdr-threads ack {MSG}", parent="toolu_t2")
        self.result("toolu_c9", "acked", parent="toolu_t2")
        self.root_ack()
        self.assertEqual(self.verify()[0], demo.FAIL)
        self.assertEqual(self.driver.s_child_no_ack("initial")[0], demo.FAIL)

    def test_missing_root_ack_fails_sk3(self):
        self.spawn_pair()
        self.child_read("toolu_t1", "toolu_c1")
        self.child_read("toolu_t2", "toolu_c2")
        self.verify()
        self.assertEqual(self.driver.s_top_level_ack("initial")[0], demo.FAIL)

    def test_sk_unverified_maps_to_child_ack_unverified(self):
        self.assertEqual(demo.Driver.unverified_reason("SK1"), "child_ack_unverified")


class ChildrenCodexTests(base.ScenarioBase):
    HARNESS = "codex"
    SCENARIO = "children"

    def setUp(self):
        super().setUp()
        self.home = Path(self.tmp.name) / "codex-home"
        self.driver.facts.update({"codex_home": str(self.home), "phase_started_utc": {"initial": "2000-01-01T00:00:00+00:00"}})

    def child(self, thread, start, end, command="herdr-threads pending-receipts"):
        path = self.home / "sessions" / "2026" / "09" / "30" / f"rollout-2026-09-30T00-00-00-{thread}.jsonl"
        path.parent.mkdir(parents=True, exist_ok=True)
        lines = [{"timestamp": start, "type": "session_meta", "payload": {"id": thread, "parent_thread_id": SESSION}},
                 {"timestamp": start, "type": "response_item", "payload": {"type": "function_call", "name": "shell", "call_id": f"c-{thread}",
                                                                           "arguments": json.dumps({"command": ["bash", "-lc", command]})}},
                 {"timestamp": end, "type": "response_item", "payload": {"type": "function_call_output", "call_id": f"c-{thread}",
                                                                         "output": json.dumps({"output": "ok", "metadata": {"exit_code": 0}})}}]
        path.write_text("".join(json.dumps(line) + "\n" for line in lines))

    def root(self):
        # Interactive Codex: the root transcript is a rollout; its spawn calls are function calls.
        for call_id in ("s1", "s2"):
            self.emit({"type": "response_item", "payload": {"type": "function_call", "name": "spawn_agent", "call_id": call_id,
                                                            "arguments": "{}"}})
        for index, command in enumerate((f"herdr-threads accept {THREAD}", f"herdr-threads ack {MSG}")):
            self.emit({"type": "response_item", "payload": {"type": "function_call", "name": "shell", "call_id": f"r{index}",
                                                            "arguments": json.dumps({"command": ["bash", "-lc", command]})}})

    def test_overlapping_child_rollouts_pass(self):
        self.settle()
        self.root()
        self.child("child-1", "2026-09-30T00:00:01Z", "2026-09-30T00:00:10Z")
        self.child("child-2", "2026-09-30T00:00:02Z", "2026-09-30T00:00:09Z")
        status, detail, _ = self.verify()
        self.assertEqual(status, demo.PASS, detail)
        # W10-E6: children are counted by stable identity (the two child rollouts), not by the two spawn call ids.
        self.assertEqual(self.driver.phase_results[-1]["sidechain"]["spawns"], ["child-1", "child-2"])
        status, detail, _ = self.driver.s_children_concurrent()
        self.assertEqual(status, demo.PASS, detail)
        self.assertIn("timestamp", detail)
        self.assertEqual(self.driver.s_child_no_ack("initial")[0], demo.PASS)

    def test_disjoint_child_rollouts_are_not_exercised(self):
        self.settle()
        self.root()
        self.child("child-1", "2026-09-30T00:00:01Z", "2026-09-30T00:00:05Z")
        self.child("child-2", "2026-09-30T00:00:06Z", "2026-09-30T00:00:09Z")
        self.verify()
        self.assertEqual(self.driver.s_children_concurrent()[0], demo.NOT_EXERCISED)

    def test_rollout_spawn_counts_as_subagent_activity(self):
        self.root()
        items = self.driver.subagent_activity(self.transcript)
        self.assertEqual([i[0] for i in items], ["s1", "s2"])


class ClearBeforeHandoffTests(unittest.TestCase):
    """The clear phase starts the new conversation before its handoff is sent (the idle wake would otherwise ACK it
    in the pre-clear session, native Codex w12)."""

    def test_s_tui_clear_then_launch_does_not_clear_again(self):
        host = base.FakeHostDriver(self, harness="codex", mode="tui")
        host.wait_state, host.screens = "idle", ["? for shortcuts", "done"]
        host.driver.facts.update({"agent_pane": "w9:p1"})
        with contextlib.redirect_stdout(io.StringIO()), fake_time():
            status, detail, _ = host.driver.s_tui_clear()
            self.assertEqual(status, demo.PASS, detail)
            started = host.driver.facts["phase_started_utc"]["clear"]
            host.driver.launch_tui("clear", "true", host.driver.ev / "x.rc")
        clears = [c for c in host.calls if c[1] == "tui:clear"]
        self.assertEqual(len(clears), 1)
        self.assertEqual(clears[0][2][3], "/new")
        self.assertEqual(host.driver.facts["phase_started_utc"]["clear"], started)


if __name__ == "__main__":
    unittest.main()
