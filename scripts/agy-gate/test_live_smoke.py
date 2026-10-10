"""Offline tests for live_smoke.py and relay_backend.py (no agy, Herdr, network, key or spend)."""
import contextlib
import io
import json
import os
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent / "agy-sandbox"))
sys.path.insert(0, str(HERE.parent / "agy-relay"))
import agyenv  # noqa: E402
import fake_upstream  # noqa: E402
import live_smoke  # noqa: E402
import relay_backend  # noqa: E402

AGY = "/bin/sh"  # any executable file satisfies the HT_AGY_BIN guard
OK_ENV = {"HT_AGY_BIN": AGY}
HERDR = "herdr 0.9.1"
CLEAN = {"sha": "0123456789abcdef0123456789abcdef01234567", "dirty": False}
DIRTY = dict(CLEAN, dirty=True)


def parse(*argv):
    return live_smoke.build_parser().parse_args(list(argv))


def run_main(argv, env=None, platform_name="darwin", git=CLEAN, **kwargs):
    out = io.StringIO()
    kwargs.setdefault("herdr_version_fn", lambda: HERDR)
    kwargs.setdefault("build_fn", lambda: "/nonexistent/herdr-threads")
    with contextlib.redirect_stdout(out):
        rc = live_smoke.main(argv, env=OK_ENV if env is None else env, platform_name=platform_name,
                             git_fn=lambda: git, **kwargs)
    return rc, out.getvalue()


class ExplodingWorld(object):
    def __init__(self, *a, **k):
        raise AssertionError("a refused run must not build a World")


class ArgumentGuardTests(unittest.TestCase):
    def test_live_and_dry_run_are_mutually_exclusive_and_one_is_required(self):
        for argv in (["--live", "--dry-run"], []):
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as caught:
                parse(*argv)
            self.assertEqual(caught.exception.code, 2, argv)
        self.assertTrue(parse("--live").live)
        self.assertTrue(parse("--dry-run").dry_run)

    def test_live_refuses_under_ci_and_github_actions(self):
        for name in ("CI", "GITHUB_ACTIONS"):
            rc, out = run_main(["--live"], env=dict(OK_ENV, **{name: "true"}), world_factory=ExplodingWorld)
            self.assertEqual(rc, live_smoke.EXIT_REFUSED, name)
            self.assertIn("CI", out)

    def test_dry_run_also_refuses_under_ci(self):
        rc, _ = run_main(["--dry-run"], env=dict(OK_ENV, CI="1"), world_factory=ExplodingWorld)
        self.assertEqual(rc, live_smoke.EXIT_REFUSED)

    def test_empty_ci_variable_does_not_refuse(self):
        self.assertIsNone(live_smoke.refusal(parse("--dry-run"), dict(OK_ENV, CI=""), "darwin", False, HERDR))

    def test_refuses_on_non_darwin(self):
        rc, out = run_main(["--dry-run"], platform_name="linux", world_factory=ExplodingWorld)
        self.assertEqual(rc, live_smoke.EXIT_REFUSED)
        self.assertIn("macOS", out)

    def test_live_refuses_a_dirty_worktree_unless_allowed(self):
        rc, out = run_main(["--live"], git=DIRTY, world_factory=ExplodingWorld)
        self.assertEqual(rc, live_smoke.EXIT_REFUSED)
        self.assertIn("clean worktree", out)
        self.assertIsNone(live_smoke.refusal(parse("--live", "--allow-dirty"), OK_ENV, "darwin", True, HERDR))

    def test_dry_run_accepts_a_dirty_worktree(self):
        self.assertIsNone(live_smoke.refusal(parse("--dry-run"), OK_ENV, "darwin", True, HERDR))

    def test_missing_agy_binary_and_old_herdr_are_refused(self):
        self.assertIn("HT_AGY_BIN", live_smoke.refusal(parse("--dry-run"), {}, "darwin", False, HERDR))
        self.assertIn("older", live_smoke.refusal(parse("--dry-run"), OK_ENV, "darwin", False, "herdr 0.9.0"))

    def test_clean_live_run_is_not_refused(self):
        self.assertIsNone(live_smoke.refusal(parse("--live"), OK_ENV, "darwin", False, HERDR))


class EvidenceDirTests(unittest.TestCase):
    def test_live_default_is_the_dated_sha12_directory_in_the_repo(self):
        path = live_smoke.default_evidence_dir(True, CLEAN["sha"], "2026-10-10", repo="/repo")
        self.assertEqual(path, Path("/repo/docs/evidence/agy-live-smoke/2026-10-10-0123456789ab"))

    def test_dry_run_default_is_under_the_scratch_root_not_the_repo(self):
        path = live_smoke.default_evidence_dir(False, CLEAN["sha"], "2026-10-10", scratch_root="/scratch",
                                               repo="/repo")
        self.assertEqual(path.parent, Path("/scratch"))
        self.assertIn("0123456789ab", path.name)
        self.assertNotIn("/repo", str(path))

    def test_utc_date_is_formatted(self):
        import datetime
        self.assertEqual(live_smoke.utc_date(datetime.datetime(2026, 1, 2, 23, 59)), "2026-01-02")


class RedactionTests(unittest.TestCase):
    def test_keys_root_and_home_are_replaced(self):
        text = "key sk-or-v1-AbC_9-x at /private/tmp/ih.1/ws and /Users/me/.gemini"
        out = live_smoke.redact_text(text, "/private/tmp/ih.1", "/Users/me")
        self.assertEqual(out, "key sk-or-REDACTED at <root>/ws and <home>/.gemini")

    def test_key_leaks_flags_a_key_but_not_the_marker(self):
        self.assertTrue(live_smoke.key_leaks("x sk-or-v1-abc"))
        self.assertTrue(live_smoke.key_leaks("sk-or-REDACTEDabc"))
        self.assertFalse(live_smoke.key_leaks("sk-or-REDACTED and sk-or-REDACTED"))
        self.assertFalse(live_smoke.key_leaks("no key here"))

    def test_writer_redacts_what_it_writes_and_finishes(self):
        with tempfile.TemporaryDirectory() as tmp:
            writer = live_smoke.EvidenceWriter(tmp, root="/private/tmp/ih.9", home="/Users/me")
            writer.write("a.txt", "sk-or-v1-secret in /private/tmp/ih.9/x")
            writer.write("b.json", {"k": "sk-or-v1-other", "p": "/Users/me/z"})
            self.assertEqual(writer.finish(), ["a.txt", "b.json"])
            self.assertEqual((Path(tmp) / "a.txt").read_text(), "sk-or-REDACTED in <root>/x")
            self.assertNotIn("sk-or-v1", (Path(tmp) / "b.json").read_text())
            self.assertIn("<home>/z", (Path(tmp) / "b.json").read_text())

    def test_writer_refuses_to_finish_when_a_written_file_holds_a_key(self):
        with tempfile.TemporaryDirectory() as tmp:
            writer = live_smoke.EvidenceWriter(tmp)
            writer.write("clean.txt", "fine")
            writer.write("planted.txt", "fine")
            (Path(tmp) / "planted.txt").write_text("leaked sk-or-v1-planted")  # bypasses the redaction
            with self.assertRaises(live_smoke.EvidenceError) as caught:
                writer.finish()
            self.assertIn("planted.txt", str(caught.exception))
            self.assertNotIn("clean.txt", str(caught.exception))


class SummarySchemaTests(unittest.TestCase):
    INFO = dict(CLEAN, agy_version="1.3.2", agy_sha512="ab" * 64, herdr_version=HERDR)
    PHASES = [{"name": "enroll", "ok": True, "seconds": 1.0}]
    GOOD = [{"name": "c", "ok": True, "detail": ""}]

    def summary(self, checks=None, aborted=None, ack_phase="wake", relay=None, mode="live"):
        return live_smoke.build_summary(self.INFO, self.GOOD if checks is None else checks, self.PHASES,
                                        relay or {"requests": 7, "cost_usd": 0.021}, ack_phase, aborted, mode)

    def test_every_required_key_is_present_and_serialisable(self):
        summary = self.summary()
        for key in live_smoke.SUMMARY_KEYS:
            self.assertIn(key, summary)
        self.assertEqual(json.loads(json.dumps(summary)), summary)
        self.assertEqual((summary["sha"], summary["dirty"], summary["requests"], summary["cost_usd"]),
                         (CLEAN["sha"], False, 7, 0.021))
        self.assertEqual(summary["model"], live_smoke.UPSTREAM_MODEL)

    def test_pass_needs_all_checks_green_and_no_abort(self):
        self.assertTrue(self.summary()["pass"])
        self.assertFalse(self.summary(checks=self.GOOD + [{"name": "x", "ok": False, "detail": "d"}])["pass"])
        self.assertFalse(self.summary(aborted="budget")["pass"])
        self.assertFalse(self.summary(checks=[])["pass"])

    def test_dry_run_summary_names_no_model_and_report_flags_prompt_phase(self):
        summary = self.summary(mode="dry-run", ack_phase="prompt")
        self.assertNotEqual(summary["model"], live_smoke.UPSTREAM_MODEL)
        self.assertIn("model behaviour", live_smoke.report_md(summary))
        self.assertIn("PASS", live_smoke.report_md(summary))


class DryRunScriptTests(unittest.TestCase):
    def test_script_is_the_fake_upstream_format_and_the_spec_sequence(self):
        script = live_smoke.dry_run_script()
        self.assertEqual(script["format"], fake_upstream.FORMAT)
        replies = script["replies"]
        self.assertEqual(replies[0]["text"], "READY")
        commands = [r["tool_calls"][0]["arguments"]["CommandLine"] for r in replies[1:3]]
        self.assertEqual(commands, ["herdr-threads ack {frame.id}",
                                    "herdr-threads send {frame.thread} --body SMOKE-OK"])
        self.assertEqual(replies[3], {"text": "done", "cost": 0.0})
        self.assertEqual(len(replies), 4)

    def test_templates_render_against_a_frame_header_the_fake_upstream_understands(self):
        body = {"messages": [{"role": "user", "content": "[herdr-threads] message m123 in t456 from sABC:\n  x"}]}
        rendered = fake_upstream.render(live_smoke.dry_run_script()["replies"][1:3], fake_upstream.last_frame(body))
        self.assertEqual(rendered[0]["tool_calls"][0]["arguments"]["CommandLine"], "herdr-threads ack m123")
        self.assertIn("send t456 --body", rendered[1]["tool_calls"][0]["arguments"]["CommandLine"])

    def test_literal_ids_replace_the_templates_and_every_tool_property_is_set(self):
        script = live_smoke.dry_run_script("mID", "tID", "/ws")
        text = json.dumps(script)
        self.assertNotIn("{frame.", text)
        args = script["replies"][1]["tool_calls"][0]["arguments"]
        self.assertEqual(args["CommandLine"], "herdr-threads ack mID")
        for key in ("CommandLine", "Cwd", "WaitMsBeforeAsync", "toolSummary", "toolAction"):
            self.assertIn(key, args)
        self.assertTrue(all(r["cost"] == 0.0 for r in script["replies"]))

    def test_commands_use_subcommands_and_flags_the_cli_defines(self):
        source = (HERE.parent.parent / "src" / "cli" / "commands.rs").read_text()
        self.assertIn("struct SendArgs", source)
        self.assertIn("body: Option<String>", source)  # `send THREAD --body TEXT`
        self.assertIn("herdr-threads ack MESSAGE", source)  # `ack MESSAGE`


class FakeWorld(object):
    def __init__(self, root):
        self.root = Path(root)
        self.owner_pid = os.getpid()
        self.timeout_scale = 1.0
        self.agy_data = self.root / "home" / ".gemini" / "antigravity-cli"

    def base_env(self):
        return dict(os.environ)


FAKE_RELAY = textwrap.dedent('''
    import argparse, os, time
    ap = argparse.ArgumentParser()
    ap.add_argument("--run-dir"); ap.add_argument("--owner-pid"); ap.add_argument("--fake-upstream")
    ap.add_argument("--live", action="store_true"); ap.add_argument("--max-requests")
    a = ap.parse_args()
    os.makedirs(a.run_dir, exist_ok=True)
    open(os.path.join(a.run_dir, "local-token"), "w").write("tok-" + str(a.fake_upstream) + "\\n")
    open(os.path.join(a.run_dir, "port"), "w").write("45678\\n")
    time.sleep(60)
''')


class RelayBackendTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.procs = []
        real = agyenv.spawn_owned

        def spawn(argv, env, owner, **kwargs):
            self.argvs.append(list(argv))
            proc = real(argv, env, owner, ensure_reaper=False, reaper_root=self.tmp.name, **kwargs)
            self.procs.append(proc)
            return proc
        self.argvs = []
        patcher = mock.patch.object(agyenv, "spawn_owned", spawn)
        patcher.start()
        self.addCleanup(patcher.stop)
        self.addCleanup(lambda: [agyenv.stop_group(p) for p in self.procs])
        self.relay_py = Path(self.tmp.name) / "relay.py"
        self.relay_py.write_text(FAKE_RELAY)
        self.world = FakeWorld(Path(self.tmp.name) / "root")
        self.world.root.mkdir()

    def test_start_returns_base_url_and_token_read_from_the_run_dir(self):
        backend = relay_backend.RelayBackend(relay_backend.LIVE, relay_py=self.relay_py, startup_timeout=15)
        url, token = backend.start(self.world)
        self.assertEqual(url, "http://127.0.0.1:45678")
        self.assertEqual(token, "tok-None")
        self.assertEqual(backend.run_dir, self.world.root / "relay")
        argv = self.argvs[0]
        self.assertIn("--live", argv)
        self.assertEqual(argv[argv.index("--owner-pid") + 1], str(os.getpid()))
        self.assertEqual(argv[argv.index("--run-dir") + 1], str(self.world.root / "relay"))
        self.assertIn("-I", argv)  # isolated interpreter
        backend.stop()
        self.assertIsNone(backend.relay_proc)

    def test_fake_mode_starts_the_upstream_first_and_points_the_relay_at_it(self):
        backend = relay_backend.RelayBackend(relay_backend.FAKE, script=live_smoke.dry_run_script(),
                                             relay_py=self.relay_py, startup_timeout=15)
        url, token = backend.start(self.world)
        try:
            upstream_port = (self.world.root / "upstream" / "port").read_text().strip()
            self.assertEqual(token, "tok-http://127.0.0.1:%s" % upstream_port)
            self.assertEqual(json.loads((self.world.root / "upstream" / "script.json").read_text())["format"],
                             fake_upstream.FORMAT)
            relay_argv = self.argvs[-1]
            self.assertEqual(relay_argv[relay_argv.index("--fake-upstream") + 1],
                             "http://127.0.0.1:%s" % upstream_port)
            self.assertEqual(len(self.argvs), 2)
        finally:
            backend.stop()

    def test_caps_are_passed_to_the_relay_and_a_dead_relay_is_an_error(self):
        backend = relay_backend.RelayBackend(relay_backend.LIVE, relay_py=self.relay_py, max_requests=24,
                                             startup_timeout=15)
        backend.start(self.world)
        self.assertEqual(self.argvs[0][self.argvs[0].index("--max-requests") + 1], "24")
        backend.stop()
        broken = Path(self.tmp.name) / "broken.py"
        broken.write_text("import sys; sys.exit(3)")
        bad = relay_backend.RelayBackend(relay_backend.LIVE, relay_py=broken, startup_timeout=15)
        with self.assertRaises(agyenv.WorldError):
            bad.start(self.world)

    def test_requests_and_budget_read_the_run_dir_files(self):
        backend = relay_backend.RelayBackend(relay_backend.LIVE, relay_py=self.relay_py)
        self.assertEqual(backend.requests(), [])
        self.assertEqual(backend.budget(), {})
        backend.run_dir = self.world.root / "relay"
        backend.run_dir.mkdir()
        (backend.run_dir / "relay.jsonl").write_text('{"seq": 1, "kind": "main"}\nnot json\n{"seq": 2}\n')
        (backend.run_dir / "budget.json").write_text('{"exhausted": true}')
        self.assertEqual([r["seq"] for r in backend.requests()], [1, 2])
        self.assertTrue(backend.budget()["exhausted"])

    def test_load_rejects_a_foreign_format_and_live_mode_takes_no_script(self):
        backend = relay_backend.RelayBackend(relay_backend.FAKE)
        backend.upstream_dir = self.world.root
        with self.assertRaises(ValueError):
            backend.load({"format": "other/1"})
        backend.load(live_smoke.dry_run_script("m", "t"))
        self.assertIn("ack m", (self.world.root / "script.json").read_text())
        with self.assertRaises(agyenv.WorldError):
            relay_backend.RelayBackend(relay_backend.LIVE).load(live_smoke.dry_run_script())
        with self.assertRaises(ValueError):
            relay_backend.RelayBackend("other")


class FlowHelperTests(unittest.TestCase):
    class StubBackend(object):
        def __init__(self, budget=None, requests=None, upstream=None):
            self._budget, self._requests, self._upstream = budget or {}, requests or [], upstream or []

        def budget(self):
            return self._budget

        def requests(self):
            return self._requests

        def upstream_requests(self):
            return self._upstream

    def test_clock_aborts_on_the_wall_clock_cap(self):
        now = [100.0]
        clock = live_smoke.Clock(self.StubBackend(), cap=600, now=lambda: now[0])
        clock.poll()
        now[0] += 601
        with self.assertRaises(live_smoke.SmokeAbort) as caught:
            clock.poll()
        self.assertEqual(caught.exception.reason, "cap")

    def test_clock_aborts_with_budget_when_the_relay_reports_exhausted(self):
        clock = live_smoke.Clock(self.StubBackend(budget={"exhausted": True}))
        with self.assertRaises(live_smoke.SmokeAbort) as caught:
            clock.wait(lambda: False, 5)
        self.assertEqual(caught.exception.reason, "budget")

    def test_clock_wait_returns_the_value_or_the_last_falsey_one(self):
        clock = live_smoke.Clock(self.StubBackend())
        self.assertEqual(clock.wait(lambda: "v", 1), "v")
        self.assertFalse(clock.wait(lambda: 0, 0.05, step=0.01))

    def test_ack_calls_match_only_the_ack_of_this_message(self):
        def backend(command):
            call = {"name": "run_command", "args_trimmed": json.dumps({"CommandLine": command})}
            return self.StubBackend(requests=[{"seq": 1, "tool_calls": [call]}])
        self.assertTrue(live_smoke.ack_calls(backend("herdr-threads ack mAAA"), "mAAA"))
        self.assertTrue(live_smoke.ack_calls(backend("herdr-threads --state-dir /x ack mAAA"), "mAAA"))
        self.assertFalse(live_smoke.ack_calls(backend("herdr-threads ack mBBB"), "mAAA"))
        self.assertFalse(live_smoke.ack_calls(backend("herdr-threads send t --body mAAA"), "mAAA"))
        self.assertFalse(live_smoke.ack_calls(backend("echo ack mAAA"), "mAAA"))

    def test_injected_messages_come_from_transcripts_and_dedupe(self):
        with tempfile.TemporaryDirectory() as tmp:
            world = FakeWorld(tmp)
            brain = world.agy_data / "brain" / "conv"
            brain.mkdir(parents=True)
            entry = {"step": 2, "type": "SYSTEM_MESSAGE",
                     "content": "<SYSTEM_MESSAGE>\n[herdr-threads] message mXYZ in tT from sS:\n  hi\n</SYSTEM_MESSAGE>"}
            other = dict(entry, content="<SYSTEM_MESSAGE>\nunrelated\n</SYSTEM_MESSAGE>")
            (brain / "t.jsonl").write_text("\n".join(json.dumps(e) for e in (entry, entry, other)) + "\n")
            found = live_smoke.injected_messages(world, self.StubBackend(), "mXYZ")
            self.assertEqual(found, [entry["content"]])
            self.assertEqual(live_smoke.injected_messages(world, self.StubBackend(), "mNONE"), [])

    def test_injected_messages_also_read_the_fake_upstream_bodies(self):
        body = {"messages": [{"role": "user", "content": "<SYSTEM_MESSAGE>\nmessage mQ1 here\n</SYSTEM_MESSAGE>"},
                             {"role": "user", "content": "mQ1 outside any system message"}]}
        with tempfile.TemporaryDirectory() as tmp:
            found = live_smoke.injected_messages(FakeWorld(tmp), self.StubBackend(upstream=[body]), "mQ1")
        self.assertEqual(len(found), 1)

    def test_data_id_accepts_a_bare_id_or_a_nested_key(self):
        self.assertEqual(live_smoke.data_id("tU1RB3iaY\n", "thread_id"), "tU1RB3iaY")
        self.assertEqual(live_smoke.data_id({"x": {"message_id": "m1"}}, "message_id", "id"), "m1")
        self.assertIsNone(live_smoke.data_id({"x": 1}, "id"))


if __name__ == "__main__":
    unittest.main()
