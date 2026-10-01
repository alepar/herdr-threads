"""Inert CLI boundary tests. No Herdr, Claude, IPC, login, or server is started."""

import contextlib
import io
import json
from pathlib import Path
import runpy
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "support"))
from fixture import NativeFixture

SCRIPT = Path(__file__).with_name("run.py")


class PrivateCliTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="capture grammar ")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / "private fixture"
        self.fixture = NativeFixture.create(self.root)
        self.fixture.record_owned("pane", "w7:p3")
        (self.root / "env.json").write_text(
            json.dumps(
                {
                    "HERDR_SOCKET_PATH": str(self.root / "state/herdr.sock"),
                    "HERDR_CONFIG_PATH": str(self.root / "config/herdr.toml"),
                    "HCOM_AUTO_APPROVE": "0",
                    "HCOM_AUTO_TRUST_WORKSPACE": "0",
                }
            )
        )

    def invoke(self, arguments, response=None, action="cli"):
        self.calls = []
        output = io.StringIO()

        def inert(argv, **kwargs):
            self.calls.append((argv, kwargs))
            return subprocess.CompletedProcess(
                argv, 0, response or '{"result":{"type":"ok"}}', ""
            )

        with patch.object(
            sys, "argv", [str(SCRIPT), action, str(self.root), *arguments]
        ), patch("subprocess.run", side_effect=inert), patch(
            "subprocess.Popen", side_effect=AssertionError("native startup forbidden")
        ), contextlib.redirect_stdout(
            output
        ), contextlib.redirect_stderr(
            output
        ):
            try:
                runpy.run_path(str(SCRIPT), run_name="__main__")
            except SystemExit as error:
                return error.code, output.getvalue()
            except Exception as error:
                self.fail(
                    f"invalid command dispatched before rejection: {type(error).__name__}: {error}"
                )
        return 0, output.getvalue()

    def reject(self, arguments, action="cli"):
        code, _ = self.invoke(arguments, action=action)
        self.assertEqual(code, 2, arguments)
        self.assertEqual(self.calls, [], "rejected command reached subprocess")

    def native_args(self):
        return [
            "--model",
            "sonnet",
            "--effort",
            "low",
            "--settings",
            str(self.root / "config/overlay.json"),
            "--setting-sources",
            "user",
            "--tools",
            "",
            "--strict-mcp-config",
            "--mcp-config",
            '{"mcpServers":{}}',
        ]

    def test_alternate_selectors_and_management_routes_never_execute(self):
        for arguments in (
            ["--session=other", "server"],
            ["session", "attach", "other"],
            ["session", "stop", "default"],
            ["--machine=other", "workspace", "list"],
            ["--remote=host", "workspace", "list"],
            ["--socket=/tmp/other", "pane", "get", "w7:p3"],
            ["workspace", "list", "--session=other"],
            ["--session", "other", "workspace", "list"],
            [
                "workspace",
                "create",
                "--cwd",
                "/private",
                "--label",
                "--session=other",
                "--no-focus",
            ],
            ["workspace", "list", "--remote-keybindings=server"],
            ["server"],
            ["server", "stop"],
            ["api", "call"],
            ["workspace", "close", "w7"],
        ):
            with self.subTest(arguments=arguments):
                self.reject(arguments)

    def test_unknown_flags_missing_payload_and_surplus_arguments_never_execute(self):
        for arguments in (
            ["workspace", "list", "extra"],
            ["pane", "get", "w7:p3", "extra"],
            ["pane", "run", "w7:p3", "echo missing delimiter"],
            ["pane", "run", "w7:p3", "--", "echo", "extra"],
            ["agent", "prompt", "w7:p3", "--unknown", "--", "text"],
            ["agent", "prompt", "w7:p3", "--wait", "--timeout", "60001", "--", "text"],
            ["agent", "read", "w7:p3", "--source", "detection", "--lines", "0"],
            [
                "agent",
                "start",
                "probe",
                "--kind",
                "claude",
                "--pane",
                "w7:p3",
                "--timeout",
                "20000",
                "--",
                "--unknown",
            ],
        ):
            with self.subTest(arguments=arguments):
                self.reject(arguments)

    def test_dedicated_actions_reject_surplus_before_any_process(self):
        for action in (
            "server",
            "cleanup",
            "prepare",
            "prepare-trusted",
            "verify-global",
        ):
            with self.subTest(action=action):
                self.reject(["--session=other"], action=action)

    def test_unowned_or_closed_mutating_panes_never_execute(self):
        for arguments in (
            ["pane", "run", "w7:p9", "--", "echo harmless"],
            ["agent", "prompt", "arbitrary-name", "--", "text"],
            [
                "agent",
                "start",
                "probe",
                "--kind",
                "claude",
                "--pane",
                "w7:p9",
                "--timeout",
                "20000",
                "--",
                *self.native_args(),
            ],
        ):
            with self.subTest(arguments=arguments):
                self.reject(arguments)
        self.fixture._append({"kind": "pane", "id": "w7:p3", "event": "close_intent"})
        self.fixture._append({"kind": "pane", "id": "w7:p3", "event": "closed"})
        self.reject(["pane", "run", "w7:p3", "--", "echo closed"])

    def test_option_shaped_payloads_cannot_become_herdr_selectors(self):
        for payload in (
            "--session=other",
            "--session",
            "--remote=host",
            "--machine=other",
            "--",
            "",
        ):
            with self.subTest(payload=payload):
                self.reject(["pane", "run", "w7:p3", "--", payload])
                self.reject(["agent", "prompt", "w7:p3", "--", payload])

    def test_literal_payload_and_private_paths_are_preserved_without_shell(self):
        payload = "printf '%s' 'literal $() --session=other with spaces'"
        code, _ = self.invoke(["pane", "run", "w7:p3", "--", payload])
        self.assertEqual(code, 0)
        argv, kwargs = self.calls[0]
        self.assertEqual(argv, ["herdr", "pane", "run", "w7:p3", payload])
        self.assertEqual(kwargs["timeout"], 60)
        self.assertEqual(
            kwargs["env"]["HERDR_SOCKET_PATH"], str(self.root / "state/herdr.sock")
        )
        self.assertNotIn("shell", kwargs)
        self.assertNotIn("HERDR_SESSION", kwargs["env"])

    def test_owned_prompt_wait_and_reads_use_explicit_pane(self):
        cases = (
            (
                [
                    "agent",
                    "prompt",
                    "w7:p3",
                    "--wait",
                    "--timeout",
                    "30000",
                    "--",
                    "text with spaces",
                ],
                [
                    "herdr",
                    "agent",
                    "prompt",
                    "w7:p3",
                    "text with spaces",
                    "--wait",
                    "--timeout",
                    "30000",
                ],
            ),
            (["pane", "get", "w7:p3"], ["herdr", "pane", "get", "w7:p3"]),
            (
                ["agent", "read", "w7:p3", "--source", "detection", "--lines", "35"],
                [
                    "herdr",
                    "agent",
                    "read",
                    "w7:p3",
                    "--source",
                    "detection",
                    "--lines",
                    "35",
                ],
            ),
            (["workspace", "list"], ["herdr", "workspace", "list"]),
        )
        for arguments, expected in cases:
            with self.subTest(arguments=arguments):
                code, _ = self.invoke(arguments)
                self.assertEqual(code, 0)
                self.assertEqual(self.calls[0][0], expected)

    def test_owned_claude_recipe_preserves_native_delimiter_and_resume(self):
        resume = "406c6097-796d-4585-bedb-519986dd577d"
        arguments = [
            "agent",
            "start",
            "probe",
            "--kind",
            "claude",
            "--pane",
            "w7:p3",
            "--timeout",
            "20000",
            "--",
            *self.native_args(),
            "--resume",
            resume,
        ]
        code, _ = self.invoke(arguments)
        self.assertEqual(code, 0)
        self.assertEqual(self.calls[0][0], ["herdr", *arguments])
        self.assertEqual(self.calls[0][1]["timeout"], 60)
        arguments[-2:] = ["--dangerously-skip-permissions"]
        self.reject(arguments)
        arguments[-1:] = ["--resume", "--session=other"]
        self.reject(arguments)

    def test_creation_records_only_success_response_root_pane(self):
        arguments = [
            "workspace",
            "create",
            "--cwd",
            "/trusted repo/path with spaces",
            "--label",
            "private probe",
            "--no-focus",
        ]
        response = json.dumps(
            {
                "id": "cli:workspace:create",
                "result": {
                    "type": "workspace_created",
                    "root_pane": {"pane_id": "w8:p1"},
                },
            }
        )
        code, _ = self.invoke(arguments, response)
        self.assertEqual(code, 0)
        self.assertIn({"kind": "pane", "id": "w8:p1"}, self.fixture.owned_resources())
        self.assertEqual(self.calls[0][0], ["herdr", *arguments])

    def test_malformed_creation_preserves_response_without_recording_or_retry(self):
        arguments = [
            "workspace",
            "create",
            "--cwd",
            "/trusted repo",
            "--label",
            "private probe",
            "--no-focus",
        ]
        before = self.fixture.ledger_path.read_bytes()
        for response in (
            "not-json",
            '{"result":{"type":"wrong","root_pane":{"pane_id":"w9:p1"}}}',
            '{"result":{"type":"workspace_created","root_pane":{"pane_id":"--session=other"}}}',
        ):
            with self.subTest(response=response):
                code, output = self.invoke(arguments, response)
                self.assertEqual(code, 2)
                self.assertIn(response, output)
                self.assertEqual(len(self.calls), 1, "ambiguous creation was retried")
                self.assertEqual(self.fixture.ledger_path.read_bytes(), before)


if __name__ == "__main__":
    unittest.main()
