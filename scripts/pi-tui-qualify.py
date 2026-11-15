#!/usr/bin/env python3
"""Opt-in Pi native-TUI qualification with public setup/launch and owned private hosts.

The default uses the retained official runtime and a scripted provider. --live
uses only OPENROUTER_API_KEY from the explicitly selected env file; no real
profile is copied or modified. Run through the repository's heavy/toolbox wrapper.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import signal
import subprocess
import time
import unittest
import urllib.request
import uuid

REPO = Path(__file__).resolve().parent.parent
FIXTURE = REPO / "scripts/pi_tui_fixtures/provider.mjs"
MODULES = REPO / "docs/superpowers/runs/2026-11-14-pi-harness/research/candidates/probe/node_modules"
WAKE = "herdr-threads: wake up"
CASES = ("setup", "safety", "threshold", "busy", "receipts", "ordering", "invitations", "negative", "failures", "lifecycle", "compaction", "handoff", "channel", "named")


def redact(text, replacements, secrets=()):
    for secret in secrets:
        if secret:
            text = text.replace(secret, "<redacted>")
    for old, new in sorted(replacements.items(), key=lambda pair: len(pair[0]), reverse=True):
        text = text.replace(str(old), new)
    return re.sub(r"herdr-server:pid=\d+:start=[\d.]+:uid=\d+", "<private-host-boot>", text)


def require_complete(results, expected):
    actual = [row["case"] for row in results]
    if len(actual) != len(set(actual)) or set(actual) != set(expected):
        raise RuntimeError("qualification case set incomplete")
    if any(row["outcome"] != "pass" for row in results):
        raise RuntimeError("qualification contains a failed case")


class Run:
    def __init__(self, args):
        self.args = args
        self.bin = Path(args.bin).resolve()
        self.modules = Path(args.modules).resolve()
        self.pi = Path(args.pi).resolve() if args.pi else self.modules / "@earendil-works/pi-coding-agent/dist/bundle/cli.js"
        self.ev = Path(args.evidence_dir).resolve()
        self.ev.mkdir(mode=0o700, parents=True, exist_ok=True)
        self.run_id = str(uuid.uuid4())
        self.base = {"PATH": os.environ["PATH"], "TERM": "xterm-256color", "LANG": "C.UTF-8", "SHELL": "/bin/bash",
                     "USER": os.environ.get("USER", ""), "LOGNAME": os.environ.get("LOGNAME", ""),
                     "HT_TEST_OWNER": str(os.getpid()), "HT_LEAK_RUN_ID": self.run_id,
                     "CARGO_BUILD_JOBS": "4", "NEXTEST_TEST_THREADS": "4", "RUST_TEST_THREADS": "4"}
        self.env = dict(self.base)
        self.host = self.root = self.workspace = self.recipient = self.sender = self.seat = None
        self.tmux_socket = None
        self.secrets = []
        self.commands, self.results, self.captures = [], [], {}
        self.facts = {"run_id": self.run_id, "candidate_sha256": hashlib.sha256(self.bin.read_bytes()).hexdigest(),
                      "live": args.live, "host": "isolated-herdr", "cleanup": {}}

    def clean(self, text):
        replacements = {str(REPO): "$CHECKOUT", str(self.root): "$SCRATCH"} if self.root else {str(REPO): "$CHECKOUT"}
        return redact(text, replacements, self.secrets)

    def run(self, argv, env=None, check=True, timeout=45):
        argv = list(map(str, argv))
        p = subprocess.Popen(argv, env=env or self.env, cwd=REPO, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                             text=True, start_new_session=True)
        try:
            out, err = p.communicate(timeout=timeout)
        except BaseException:
            os.killpg(p.pid, signal.SIGTERM)
            try:
                p.communicate(timeout=2)
            except subprocess.TimeoutExpired:
                os.killpg(p.pid, signal.SIGKILL)
                p.communicate()
            raise
        self.commands.append({"at": time.time(), "argv": argv, "rc": p.returncode, "stdout": out, "stderr": err})
        if check and p.returncode:
            raise RuntimeError(self.clean(f"{argv[:4]}: rc={p.returncode} {(err or out)[:500]}"))
        return p.returncode, out, err

    def herdr(self, *args, check=True):
        _, out, _ = self.run(["herdr", *args], check=check)
        try:
            return json.loads(out) if out.strip() else {}
        except ValueError:
            return {"raw": out}

    def ht(self, *args, pane=None, human=False, setup=False, check=True, extra_env=None):
        env = {**self.env, **(extra_env or {})}
        if pane:
            env["HERDR_PANE_ID"] = pane
        prefix = ["--state-dir", str(self.root / "state"), "--host-endpoint", str(self.root / "h.sock"), "--json"]
        words = [*args, *prefix] if setup else [*(["human"] if human else []), *prefix, *args]
        _, out, _ = self.run([self.bin, *words], env, check=check)
        return json.loads(out) if out.strip() else {}

    def rows(self):
        path = self.root / "native.jsonl"
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

    def count(self, event):
        return sum(row["event"] == event for row in self.rows())

    def wait(self, label, predicate, seconds=30):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            value = predicate()
            if value:
                return value
            time.sleep(.1)
        raise RuntimeError("timed out: " + label)

    def keys(self, *keys):
        self.herdr("pane", "send-keys", self.recipient, *keys)

    def prompt(self, text):
        self.herdr("pane", "send-text", self.recipient, text)
        self.keys("enter")

    def turn(self, text):
        settled = self.count("agent_settled")
        self.prompt(text)
        self.wait("native settlement", lambda: self.count("agent_settled") > settled, 90 if self.args.live else 30)
        self.wait("host idle", lambda: self.herdr("agent", "get", self.recipient)["result"]["agent"]["agent_status"] in ("idle", "done"))

    def capture(self, label):
        value = {"at": time.time(), "visible": self.herdr("pane", "read", self.recipient, "--source", "visible"),
                 "styled": self.herdr("pane", "read", self.recipient, "--source", "visible", "--format", "ansi"),
                 "detection": self.herdr("agent", "read", self.recipient, "--source", "detection", check=False),
                 "agent": self.herdr("agent", "get", self.recipient, check=False)}
        self.captures[label] = value
        return value

    def send(self, body, priority=0, ack=False):
        flags = ["--require-ack", self.seat, "--deadline", "120"] if ack else ["-p", str(priority)]
        return self.ht("send", "--seat", self.seat, "--body", body, *flags, pane=self.sender, human=True)["result"]["data"]

    def receipt(self, mid):
        rows = self.ht("delivery", "recipients", mid, pane=self.sender, human=True)["result"]["data"]["items"]
        return next(row for row in rows if row["seat"] == self.seat)

    def model_saw(self, body, since=0):
        return any(row["event"] in ("provider_request", "context") and body in json.dumps(row) for row in self.rows()[since:])

    def quiet(self, seconds=1):
        starts = self.count("before_agent_start")
        time.sleep(seconds)
        if self.count("before_agent_start") != starts:
            raise AssertionError("unexpected native run during quiet observation")

    def wake_lane(self):
        # A refusal test must not pass merely because the preceding successful
        # wake still owns the host's ordinary 30-second spacing window.
        while True:
            wakes = [r["at"] / 1000 for r in self.rows() if r["event"] == "before_agent_start" and r["data"].get("prompt") == WAKE]
            if not wakes:
                return
            remaining = wakes[-1] + 32 - time.time()
            if remaining <= 0:
                return
            time.sleep(min(remaining, 1))

    def start(self):
        if not self.pi.exists():
            raise RuntimeError("official pi runtime missing; provide --pi/--modules")
        self.facts["source_head"] = self.run(["git", "rev-parse", "HEAD"], self.base)[1].strip()
        self.facts["fixture_sha256"] = hashlib.sha256(FIXTURE.read_bytes()).hexdigest()
        self.facts["hc"] = {"binary_present": bool(shutil.which("hc", path=self.base["PATH"])),
                            "native_wake": False, "runtime_qualification": "not exercised by this Herdr/tmux driver"}
        helper = '''set -u
source "$1"
probe_root=$(ih_root_new)
trap 'ih_teardown "$probe_root"' EXIT
printf '%s\\n' "$probe_root"
mkdir -p "$probe_root/cfg" "$probe_root/project" "$probe_root/bin" "$probe_root/agent"
export PATH="$probe_root/bin:$PATH"
printf "default_shell = '/bin/bash'\\n" > "$probe_root/cfg/herdr.toml"
ih_start "$probe_root" pi-native-qualification || exit 1
printf 'READY\\n'
read -r _ || true
'''
        lib = REPO / "scripts/lib/isolated-herdr.sh"
        self.host = subprocess.Popen(["bash", "-c", helper, "bash", str(lib)], env={**self.base, "IH_LIB": str(lib)},
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.root = Path(self.host.stdout.readline().strip())
        if self.host.stdout.readline().strip() != "READY":
            raise RuntimeError("isolated Herdr failed to start")
        self.env = {**self.base, "HOME": str(self.root / "home"), "XDG_CONFIG_HOME": str(self.root / "cfg"),
                    "XDG_STATE_HOME": str(self.root / "st"), "XDG_RUNTIME_DIR": str(self.root / "rt"),
                    "XDG_CACHE_HOME": str(self.root / "cache"), "XDG_DATA_HOME": str(self.root / "data"),
                    "TMPDIR": str(self.root / "tmp"),
                    "HERDR_CONFIG_PATH": str(self.root / "cfg/herdr.toml"), "HERDR_SOCKET_PATH": str(self.root / "h.sock"),
                    "PI_CODING_AGENT_DIR": str(self.root / "agent"), "PI_OFFLINE": "1", "HT_PI_QUALIFY_ROOT": str(self.root),
                    "HT_PI_QUALIFY_MODULES": str(self.modules), "PATH": str(self.root / "bin") + ":" + self.base["PATH"]}
        self.env["HT_PI_QUALIFY_CLI"] = json.dumps([str(self.bin), "--state-dir", str(self.root / "state"), "--host-endpoint", str(self.root / "h.sock"), "--json"])
        for directory in ("cache", "data", "tmp", "state/node"):
            (self.root / directory).mkdir(mode=0o700, parents=True, exist_ok=True)
        (self.root / "state/node/settings.json").write_text('{"sync":{"discover":false},"harness_manifest":"off","update_check":"off"}\n')
        (self.root / "agent/settings.json").write_text('{"quietStartup":true,"packages":[],"retry":{"enabled":false},"compaction":{"enabled":false,"keepRecentTokens":64,"reserveTokens":128}}\n')
        self.provider, self.model = "ht-fixture", "scripted"
        secret_command = ""
        if self.args.live:
            self.prepare_live()
            secret_command = ". " + shlex.quote(str(self.root / "provider.env")) + "\n"
            self.env["HT_PI_QUALIFY_LIVE"] = "1"
        shim = self.root / "bin/pi"
        executable = "exec " + ("node " if self.pi.suffix == ".js" else "") + shlex.quote(str(self.pi)) + ' "$@"\n'
        # A public handoff creates its pane from the private host environment.
        # Keep qualification-only provider/observer config in this owned shim,
        # so the new process receives it without inheriting any owner claim.
        fixture_path = self.root / "provider.mjs"
        production_extension = self.root / "agent/extensions/herdr-threads/index.js"
        extension_args = ["--extension", str(production_extension), "--extension", str(fixture_path)]
        fixture_env = {"HT_PI_QUALIFY_ROOT": str(self.root), "HT_PI_QUALIFY_MODULES": str(self.modules),
                       "HT_PI_QUALIFY_CLI": self.env["HT_PI_QUALIFY_CLI"], "PI_OFFLINE": "1",
                       "HT_PI_QUALIFY_OPTS": shlex.join(["--provider", self.provider, "--model", self.model, *extension_args])}
        if self.args.live:
            fixture_env["HT_PI_QUALIFY_LIVE"] = "1"
        shim.write_text("#!/bin/sh\n" + secret_command + "".join("export " + k + "=" + shlex.quote(v) + "\n" for k, v in fixture_env.items()) + executable)
        shim.chmod(0o700)
        self.shim = shim
        cli_shim = self.root / "bin/herdr-threads"
        cli_shim.write_text("#!/bin/sh\n"
                            "human=0\n[ \"${1:-}\" = human ] && { human=1; shift; }\n"
                            "state=1 host=1\nfor word in \"$@\"; do\ncase \"$word\" in\n"
                            "--state-dir|--state-dir=*) state=0 ;;\n--host-endpoint|--host-endpoint=*) host=0 ;;\nesac\ndone\n"
                            + "[ \"$host\" = 0 ] || set -- --host-endpoint " + shlex.quote(str(self.root / "h.sock")) + ' "$@"\n'
                            + "[ \"$state\" = 0 ] || set -- --state-dir " + shlex.quote(str(self.root / "state")) + ' "$@"\n'
                            + '[ "$human" = 0 ] || set -- human "$@"\nexec ' + shlex.quote(str(self.bin)) + ' "$@"\n')
        cli_shim.chmod(0o700)
        self.fixture = self.root / "provider.mjs"
        shutil.copyfile(FIXTURE, self.fixture)
        self.env["HT_PI_QUALIFY_OPTS"] = fixture_env["HT_PI_QUALIFY_OPTS"]
        self.extension_args = extension_args
        self.facts["extension_order"] = [str(production_extension), str(self.fixture)]
        flags = sum((["--env", key + "=" + value] for key, value in self.env.items()), [])
        layout = self.herdr("workspace", "create", "--cwd", str(self.root / "project"), "--label", "pi-qualification", "--no-focus", *flags)
        self.workspace = layout["result"]["workspace"]["workspace_id"]
        self.sender = layout["result"]["root_pane"]["pane_id"]
        tab = self.herdr("tab", "create", "--workspace", self.workspace, "--cwd", str(self.root / "project"), "--label", "pi-recipient", "--no-focus", *flags)
        self.recipient = tab["result"]["root_pane"]["pane_id"]
        self.ht("daemon", "ensure")
        self.ht("setup", "pi", "--harness-binary", str(shim), setup=True)
        self.facts["setup_status"] = self.ht("setup-status", "pi")
        self.ht("me", "init", pane=self.sender, human=True)
        self.facts["launch"] = self.ht("launch", "--kind", "pi", "--pane", self.recipient, "--harness-binary", str(shim), "--",
                                           "--provider", self.provider, "--model", self.model, *self.extension_args)
        if self.facts["launch"]["launch"]["outcome"] != "started":
            raise RuntimeError("public launch did not confirm startup")
        self.seat = self.facts["launch"]["launch"]["seat"]
        self.wait("native session_start", lambda: self.count("session_start"))
        self.turn("Qualification baseline. Reply only READY. Do not call tools.")
        self.capture("startup")

    def prepare_live(self):
        # Read only the selected value; do not source or copy a real profile.
        value = None
        with Path(self.args.openrouter_env_file).expanduser().open() as source:
            for line in source:
                match = re.match(r"\s*(?:export\s+)?OPENROUTER_API_KEY\s*=\s*(.*?)\s*$", line)
                if match:
                    value = match.group(1).strip().strip("\"'")
                    break
        if not value:
            raise RuntimeError("authorized OpenRouter value unavailable")
        self.secrets.append(value)
        path = self.root / "provider.env"
        path.write_text("export OPENROUTER_API_KEY=" + shlex.quote(value) + "\n")
        path.chmod(0o600)
        with urllib.request.urlopen("https://openrouter.ai/api/v1/models", timeout=20) as response:
            available = {model["id"] for model in json.load(response)["data"]}
        chosen = self.args.model or "deepseek/deepseek-v4-pro"
        if chosen not in available or not chosen.startswith("deepseek/"):
            raise RuntimeError("deliberately selected DeepSeek model is absent from current provider catalog")
        self.provider, self.model = "openrouter", chosen
        self.facts["model_catalog_check"] = {"provider": self.provider, "model": self.model, "available": True}

    def case_setup(self):
        before = (self.root / "agent/extensions/herdr-threads/index.js").read_bytes()
        self.ht("setup", "pi", "--harness-binary", str(self.shim), setup=True)
        assert (self.root / "agent/extensions/herdr-threads/index.js").read_bytes() == before
        foreign = self.root / "agent/extensions/foreign.txt"
        foreign.write_text("foreign fixture")
        # Exercise preservation in a second scratch scope, never alter the live runtime's extension.
        other = self.root / "setup-agent"
        other.mkdir()
        extra = {"PI_CODING_AGENT_DIR": str(other)}
        self.ht("setup", "pi", "--harness-binary", str(self.shim), setup=True, extra_env=extra)
        owned = other / "extensions/herdr-threads/index.js"
        edited = owned.read_text() + "\n// edited qualification artifact\n"
        owned.write_text(edited)
        self.ht("unsetup", "pi", extra_env=extra, check=False)
        assert owned.read_text() == edited and foreign.read_text() == "foreign fixture"
        self.facts["setup_preserves_edits"] = True

    def case_safety(self):
        self.capture("empty")
        for label, text in (("literal", "UNSENT DRAFT"), ("whitespace", "    "), ("wrapped", "W" * 260)):
            self.wake_lane()
            self.herdr("pane", "send-text", self.recipient, text)
            time.sleep(.2)
            self.capture(label)
            before = self.count("before_agent_start")
            mid = self.send("SAFETY " + label)
            self.quiet(2)
            assert self.count("before_agent_start") == before
            self.keys("ctrl+c")
            # New attention restarts the ordinary refusal ladder immediately after draft removal.
            settled = self.count("agent_settled")
            self.send("SAFETY RELEASE " + label)
            self.wait("scheduler wake after draft clear", lambda: self.count("agent_settled") > settled, 45)
            assert self.model_saw("SAFETY " + label)
        baseline = self.capture("whitespace-equivalence-empty")
        self.herdr("pane", "send-text", self.recipient, "    "); self.keys("ctrl+a"); time.sleep(.2)
        equivalent = self.capture("whitespace-home-limit")
        assert baseline["styled"]["raw"] == equivalent["styled"]["raw"], "record actual equivalence, never assert preservation"
        self.keys("ctrl+c")
        self.herdr("pane", "send-text", self.recipient, "first")
        self.keys("shift+enter")
        self.herdr("pane", "send-text", self.recipient, "second")
        self.keys("ctrl+a")
        self.capture("multiline-home")
        self.send("MULTILINE DRAFT"); self.quiet(2); self.keys("ctrl+c")
        for command, label in ((None, "model-picker"), ("/settings", "settings-picker"), ("/probe-select", "inline-select")):
            self.wake_lane()
            if command:
                self.prompt(command)
            else:
                self.keys("ctrl+l")
            time.sleep(.3); self.capture(label)
            self.send("MODAL " + label); self.quiet(2)
            self.keys("esc")
            settled = self.count("agent_settled")
            self.send("MODAL RELEASE " + label)
            self.wait("scheduler wake after picker close", lambda: self.count("agent_settled") > settled, 45)
        self.facts["accepted_limits"] = ["external custom overlays", "whitespace-only draft with cursor returned to column zero"]

    def case_threshold(self):
        # Set the threshold as the current native process, not the controller.
        self.prompt("/probe-threshold 0"); time.sleep(.2)
        since = len(self.rows())
        self.send("PASSIVE-P1", priority=1); self.quiet(2)
        self.turn("Qualification manual turn. Reply READY without tools.")
        assert self.model_saw("PASSIVE-P1", since)
        for threshold, priority in ((0, 0), (3, 3), (-1, 9)):
            self.wake_lane()
            self.prompt("/probe-threshold " + str(threshold)); time.sleep(.2)
            settled = self.count("agent_settled")
            body = f"THRESHOLD {threshold} PRIORITY {priority}"
            self.send(body, priority=priority)
            self.wait("threshold scheduler wake", lambda: self.count("agent_settled") > settled, 45)
            assert self.model_saw(body)
        self.prompt("/probe-threshold 0"); time.sleep(.2)

    def case_busy(self):
        for mode, signal_file, event in (("hold-text", "release-text", "stream_held"), ("tool", "release-tool", "tool_held")):
            path = self.root / signal_file
            path.unlink(missing_ok=True)
            (self.root / "mode.txt").write_text(mode)
            held = self.count(event); settled = self.count("agent_settled")
            self.prompt("qualification " + mode)
            self.wait("held native run", lambda: self.count(event) > held)
            time.sleep(.7); self.capture(mode)
            assert self.herdr("agent", "get", self.recipient)["result"]["agent"]["agent_status"] == "working"
            starts = self.count("before_agent_start")
            self.send("BUSY-EXACT-" + mode); self.quiet(.5)
            path.touch()
            self.wait("busy run settled", lambda: self.count("agent_settled") > settled)
            assert self.model_saw("BUSY-EXACT-" + mode)
            # Exceeds the first native wake retry; delivered attention must not trail the turn.
            self.quiet(31)
            assert self.count("before_agent_start") == starts

    def case_receipts(self):
        self.wake_lane()
        settled = self.count("agent_settled")
        mid = self.send("RECEIPT-EXACT-FRAME", ack=True)
        self.wait("ACK wake", lambda: self.count("agent_settled") > settled, 45)
        row = self.wait("exact context receipt", lambda: (lambda r: r if r["effective_status"] == "acknowledged" else None)(self.receipt(mid)))
        observation = json.loads(row["ack_provenance"]["native_observation"])
        assert observation["via"] == "context" and observation["action_provenance"] == "cooperative_mod_delivery"
        assert self.model_saw("RECEIPT-EXACT-FRAME")
        self.facts["receipt"] = row
        contexts = [r for r in self.rows() if r["event"] == "context"]
        assert any(any("RECEIPT-EXACT-FRAME" in str(m.get("content", "")) and any(e.get("customType") == m.get("customType") and e.get("content") == m.get("content") for e in r["branch"]) for m in r["data"]["messages"]) for r in contexts)

    def case_lifecycle(self):
        sid = next(r["state"]["sid"] for r in self.rows() if r["event"] == "session_start")
        starts = self.count("session_start")
        self.prompt("/reload")
        self.wait("native reload", lambda: self.count("session_start") > starts)
        self.wait("reload UI restored", lambda: "Reloaded keybindings" in self.herdr("pane", "read", self.recipient, "--source", "visible").get("raw", ""))
        self.turn("After reload, reply READY without tools.")
        current = [r for r in self.rows() if r["event"] == "session_start"][-1]
        assert current["state"]["sid"] == sid
        self.prompt("/probe-tree")
        self.wait("tree navigation", lambda: self.count("tree_done"))
        self.turn("After tree, reply READY without tools.")
        starts = self.count("session_start"); self.prompt("/probe-fork")
        self.wait("native fork", lambda: self.count("session_start") > starts)
        self.wait("fork UI restored", lambda: self.count("fork_done"))
        assert [r for r in self.rows() if r["event"] == "session_start"][-1]["state"]["sid"] != sid
        self.keys("ctrl+c")
        self.turn("After fork, reply READY without tools.")
        starts = self.count("session_start"); self.prompt("/probe-new")
        self.wait("new native session", lambda: self.count("session_start") > starts)
        self.wait("new UI restored", lambda: self.count("new_done"))
        self.turn("After new session, reply READY without tools.")
        starts = self.count("session_start"); self.prompt("/probe-resume")
        self.wait("native resume", lambda: self.count("session_start") > starts and self.count("resume_done"))
        assert [r for r in self.rows() if r["event"] == "session_start"][-1]["state"]["sid"] == sid
        self.turn("After resume, reply READY without tools.")

    def case_ordering(self):
        # Hold the composer, so the actual watch batch contains all priorities
        # before one native prompt can project a sorted page into model context.
        self.wake_lane()
        self.herdr("pane", "send-text", self.recipient, "OWNED ORDERING HOLD")
        for priority in (5, 1, 0):
            self.send("ORDER-" + str(priority), priority=priority)
        time.sleep(.5)
        settled = self.count("agent_settled")
        self.keys("ctrl+c")
        self.send("ORDER-TRIGGER", priority=0)
        self.wait("ordered native page", lambda: self.count("agent_settled") > settled, 45)
        payloads = [json.dumps(r["data"]["messages"]) for r in self.rows() if r["event"] == "context"]
        page = next(p for p in payloads if all("ORDER-" + str(n) in p for n in (0, 1, 5)))
        assert page.index("ORDER-0") < page.index("ORDER-1") < page.index("ORDER-5")
        self.facts["native_page_order"] = [0, 1, 5]

    def case_invitations(self):
        self.wake_lane()
        thread = self.ht("thread", "create", "--name", "pi-qualification-invite", "--topic", "synthetic native invitation", "--goal", "qualification", pane=self.sender, human=True)["result"]["data"]
        settled = self.count("agent_settled")
        self.ht("invite", thread, "--seat", self.seat, pane=self.sender, human=True)
        self.wait("fixed-p0 invitation wake", lambda: self.count("agent_settled") > settled, 45)
        assert self.model_saw(thread)
        commands = self.count("command")
        self.prompt("/probe-accept " + thread)
        self.wait("owned native invitation acceptance", lambda: self.count("command") > commands)
        assert [r for r in self.rows() if r["event"] == "command"][-1]["code"] == 0
        self.facts["invitation"] = {"native_wake": True, "accepted": True}

    def case_failures(self):
        for mode in ("error", "abort", "cancel-settle"):
            (self.root / "release-text").unlink(missing_ok=True)
            (self.root / "mode.txt").write_text("hold-error" if mode == "error" else "hold-text")
            held = self.count("stream_held"); settled = self.count("agent_settled")
            self.prompt("qualification failed boundary " + mode)
            self.wait("held failure boundary", lambda: self.count("stream_held") > held)
            mid = self.send("FAILED-BOUNDARY-" + mode, priority=1)
            since = len(self.rows())
            starts = self.count("before_agent_start")
            if mode == "cancel-settle":
                (self.root / "cancel-settle").touch()
            if mode == "abort":
                self.keys("esc")
            else:
                (self.root / "release-text").touch()
            self.wait("failed native run settles", lambda: self.count("agent_settled") > settled)
            self.quiet(.5)
            assert self.count("before_agent_start") == starts
            # Non-receipt p1 rows remain unread until the next actual user turn.
            assert not any(r["event"] == "context" and "FAILED-BOUNDARY-" + mode in json.dumps(r) for r in self.rows()[since:])
            (self.root / "cancel-settle").unlink(missing_ok=True)
            self.turn("User recovery after " + mode + ". Reply READY without tools.")
            assert self.model_saw("FAILED-BOUNDARY-" + mode)
        self.facts["failed_boundaries"] = ["provider error", "native abort", "cancellation at settle"]

    def case_compaction(self):
        self.turn("Synthetic compaction history " + "qualification data " * 150)
        before = self.count("session_compact")
        self.prompt("/probe-compact")
        self.wait("native compaction", lambda: self.count("session_compact") > before, 45)
        self.wait("compaction UI completion", lambda: self.count("compact_done"))
        self.turn("After native compaction, reply READY without tools.")
        self.facts["native_compaction"] = True

    def case_negative(self):
        # A later native boundary transformer rejects the exact full-frame
        # proposal before persistence. A returned draft must not settle a receipt.
        (self.root / "mode.txt").write_text("hold-text")
        (self.root / "release-text").unlink(missing_ok=True)
        held = self.count("stream_held"); settled = self.count("agent_settled")
        self.prompt("qualification transformed boundary")
        self.wait("held negative run", lambda: self.count("stream_held") > held)
        mid = self.send("NEGATIVE-EXACT-FRAME", ack=True)
        (self.root / "transform").touch()
        (self.root / "release-text").touch()
        self.wait("transformed run settles", lambda: self.count("agent_settled") > settled)
        assert self.receipt(mid)["effective_status"] == "pending"
        (self.root / "transform").unlink()
        self.turn("User retry after rejected draft. Reply READY without tools.")
        self.wait("retry exact receipt", lambda: self.receipt(mid)["effective_status"] == "acknowledged")
        # A truncated native frame never supplies a complete receipt.
        long_mid = self.send("TRUNCATED-BEGIN " + "synthetic " * 1100 + " TRUNCATED-END", ack=True)
        self.wait("truncated frame proposed", lambda: self.model_saw("TRUNCATED-BEGIN"), 45)
        assert self.receipt(long_mid)["effective_status"] == "pending"
        self.facts["truncation_pending"] = True
        self.prompt("/probe-read-ack " + long_mid)
        self.wait("explicit full-body ACK cleanup", lambda: self.receipt(long_mid)["effective_status"] == "acknowledged")

    def case_named(self):
        if not shutil.which("tmux", path=self.env["PATH"]):
            raise RuntimeError("named/tmux qualification requires tmux; absence is not a pass")
        self.tmux_socket = self.root / "tmux.sock"
        tmux_config = self.root / "tmux.conf"
        tmux_config.write_text("set-option -g default-shell /bin/bash\nset-window-option -g remain-on-exit on\n")
        named = {k: v for k, v in self.env.items() if not k.startswith("HERDR_")}
        named["HERDR_THREADS_SEAT_NAME"] = "pi-qualification-named"
        command = "env -i " + shlex.join([k + "=" + v for k, v in named.items()])
        command += ' TMUX="$TMUX" TMUX_PANE="$TMUX_PANE" '
        command += shlex.join([str(self.shim), "--provider", self.provider, "--model", self.model, *self.extension_args])
        starts = self.count("session_start")
        self.run(["tmux", "-S", self.tmux_socket, "-f", tmux_config, "new-session", "-d", "-x", "120", "-y", "40", "-c", self.root / "project", "-s", "pi", command])
        self.wait("named native startup", lambda: self.count("session_start") > starts)
        sid = [r for r in self.rows() if r["event"] == "session_start"][-1]["state"]["sid"]
        def identity():
            rows = self.ht("seat", "list", pane=self.sender, human=True)["result"]["data"]["items"]
            # Allocation precedes CheckIn. An allocated, still-unbound named
            # seat is not ready to receive an ACK-required request yet.
            return next((row["seat"] for row in rows if row.get("target") == "pi-qualification-named"
                         and row.get("active") is True and any(b.get("harness") == "pi" and b.get("connected") is True for b in row.get("bindings", []))), None)
        seat = self.wait("named own identity", identity)
        self.facts["named_startup_binding"] = self.ht("seat", "inspect", seat, pane=self.sender, human=True)
        old_seat = self.seat
        try:
            self.seat = seat
            before = self.count("agent_settled")
            mid = self.send("NAMED-EXACT-WAKE", ack=True)
            self.wait("verified tmux native wake", lambda: any(r["event"] == "agent_settled" and r["state"]["sid"] == sid for r in self.rows()), 45)
            self.wait("named context receipt", lambda: self.receipt(mid)["effective_status"] == "acknowledged")
            assert self.count("agent_settled") > before
            self.send("NAMED-PASSIVE-P1", priority=1)
            self.quiet(1)
            before = self.count("agent_settled")
            self.run(["tmux", "-S", self.tmux_socket, "send-keys", "-t", "pi", "Named user turn after passive p1.", "Enter"])
            self.wait("named user turn", lambda: self.count("agent_settled") > before)
            assert self.model_saw("NAMED-PASSIVE-P1")
            (self.root / "mode.txt").write_text("tool")
            (self.root / "release-tool").unlink(missing_ok=True)
            held = self.count("tool_held"); before = self.count("agent_settled")
            self.run(["tmux", "-S", self.tmux_socket, "send-keys", "-t", "pi", "Named held-tool turn.", "Enter"])
            self.wait("named busy tool", lambda: self.count("tool_held") > held)
            starts = self.count("before_agent_start")
            busy_mid = self.send("NAMED-BUSY-P0", ack=True)
            self.quiet(.5)
            (self.root / "release-tool").touch()
            self.wait("named busy turn settles", lambda: self.count("agent_settled") > before)
            self.wait("named busy context receipt", lambda: self.receipt(busy_mid)["effective_status"] == "acknowledged")
            self.quiet(31)
            assert self.count("before_agent_start") == starts
            self.facts["named"] = {"receipt": self.receipt(mid), "native_session": sid,
                                   "passive_p1_next_turn": True, "busy_p0_receipt": self.receipt(busy_mid), "no_trailing_wake": True,
                                   "screen": self.run(["tmux", "-S", self.tmux_socket, "capture-pane", "-p", "-e", "-t", "pi"])[1]}
        finally:
            self.facts["named_final_binding"] = self.ht("seat", "inspect", seat, pane=self.sender, human=True)
            self.seat = old_seat

    def case_handoff(self):
        self.prompt("/probe-handoff")
        result = self.wait("top-level native handoff result", lambda: next((r for r in self.rows() if r["event"] == "handoff_result"), None))
        assert result["code"] == 0, result["stderr"]
        self.facts["handoff"] = json.loads(result["stdout"])
        self.wait("handoff body in native context", lambda: self.model_saw("HANDOFF EXACT SYNTHETIC BODY"), 60)

    def case_channel(self):
        self.wake_lane()
        settings = self.root / "state/node/settings.json"
        self.ht("daemon", "stop")
        settings.write_text('{"sync":{"discover":false},"mod_delivery":"off"}\n')
        self.wait("owned daemon stopped", lambda: not self.ht("daemon", "health", check=False))
        self.ht("daemon", "ensure")
        self.turn("Manual channel-disabled turn; reply READY without tools.")
        since = len(self.rows()); settled = self.count("agent_settled")
        mid = self.send("NO-CHANNEL-EXACT", ack=True)
        self.wait("native wake without channel", lambda: self.count("agent_settled") > settled, 45)
        assert not self.model_saw("NO-CHANNEL-EXACT", since)
        assert self.receipt(mid)["effective_status"] == "pending"
        count = self.count("explicit_inbox"); self.prompt("/probe-inbox")
        self.wait("explicit inbox fallback", lambda: self.count("explicit_inbox") > count)
        shown = [r for r in self.rows() if r["event"] == "explicit_inbox"][-1]
        assert shown["code"] == 0 and "NO-CHANNEL-EXACT" in shown["stdout"]
        self.wait("explicit inbox receipt", lambda: self.receipt(mid)["effective_status"] == "acknowledged")
        self.ht("daemon", "stop")
        settings.write_text('{"sync":{"discover":false},"mod_delivery":"on"}\n')
        self.wait("owned daemon stopped again", lambda: not self.ht("daemon", "health", check=False))
        self.ht("daemon", "ensure")
        starts = self.count("session_start"); self.prompt("/reload")
        self.wait("channel-enabled reload", lambda: self.count("session_start") > starts)
        self.wait("reload restored", lambda: "Reloaded keybindings" in self.herdr("pane", "read", self.recipient, "--source", "visible").get("raw", ""))
        self.facts["channel_fallback"] = {"native_wake": True, "no_false_context_receipt": True, "explicit_inbox": True}

    def case_live(self):
        since = len(self.rows())
        self.case_receipts()
        tools = [r["data"] for r in self.rows()[since:] if r["event"] == "tool_execution_start"]
        inbox_calls = sum(bool(re.search(r"\bherdr-threads\b[^\n]*\binbox\b", json.dumps(tool))) for tool in tools)
        self.facts["live_wake_tools"] = tools
        self.facts["live_wake_inbox_calls"] = inbox_calls
        assert inbox_calls == 0, "redundant inbox call after exact native context delivery"
        self.send("LIVE PASSIVE P1. Reply only RECEIVED when delivered.", priority=1)
        self.quiet(2)
        self.turn("Reply READY without tools.")
        models = {(r["state"]["provider"], r["state"]["model"]) for r in self.rows() if r.get("state") and r["event"] in ("before_agent_start", "context")}
        assert models == {(self.provider, self.model)}
        assert any(r["event"] == "after_provider_response" and r["data"]["status"] == 200 for r in self.rows())
        self.facts["live_model_continuity"] = sorted(models)

    def cleanup(self):
        cleanup = self.facts["cleanup"]
        trace = ""
        try:
            if self.recipient:
                self.capture("final")
                self.keys("esc")
                self.prompt("/quit")
                time.sleep(.5)
            if self.root and (self.root / "native.jsonl").exists():
                trace = (self.root / "native.jsonl").read_text()
            if self.tmux_socket:
                self.facts["named_final_screen"] = self.run(["tmux", "-S", self.tmux_socket, "capture-pane", "-p", "-e", "-t", "pi"], check=False)[1]
                self.run(["tmux", "-S", self.tmux_socket, "kill-server"], check=False)
            if self.workspace:
                cleanup["workspace_close"] = self.herdr("workspace", "close", self.workspace, check=False)
            if self.root:
                cleanup["daemon_stop"] = self.ht("daemon", "stop", check=False)
        except BaseException as exc:
            cleanup["error"] = self.clean(str(exc))
        finally:
            if self.root:
                (self.root / "provider.env").unlink(missing_ok=True)
            if self.host:
                try:
                    self.host.communicate("stop\n", timeout=30)
                    cleanup["helper_rc"] = self.host.returncode
                except BaseException as exc:
                    cleanup["helper_error"] = self.clean(str(exc))
            cleanup["root_removed"] = self.root is not None and not self.root.exists()
            rc, out, err = self.run([REPO / "scripts/check-no-leaked-processes", "--run-id", self.run_id], self.base, check=False)
            cleanup.update(leak_rc=rc, leak_stdout=out, leak_stderr=err)
            for name, value in (("manifest.json", self.facts), ("results.json", self.results), ("commands.json", self.commands), ("captures.json", self.captures)):
                (self.ev / name).write_text(self.clean(json.dumps(value, indent=2)) + "\n")
            (self.ev / "native.jsonl").write_text(self.clean(trace))
            for secret in self.secrets:
                if any(secret in path.read_text() for path in self.ev.iterdir() if path.is_file()):
                    shutil.rmtree(self.ev)
                    raise RuntimeError("credential detected in retained evidence; evidence removed")
        return cleanup.get("helper_rc") == 0 and cleanup["root_removed"] and cleanup["leak_rc"] == 0 and "error" not in cleanup


class DriverTests(unittest.TestCase):
    def test_missing_duplicate_and_failed_case_cannot_pass(self):
        for rows in ([], [{"case": "a", "outcome": "fail"}], [{"case": "a", "outcome": "pass"}] * 2):
            with self.assertRaises(RuntimeError):
                require_complete(rows, ["a"])
        require_complete([{"case": "a", "outcome": "pass"}], ["a"])

    def test_secret_and_private_host_details_are_removed(self):
        value = redact("/scratch/key secret herdr-server:pid=12:start=30.0:uid=1", {"/scratch": "$SCRATCH"}, ["secret"])
        self.assertNotIn("secret", value)
        self.assertNotIn("pid=12", value)
        self.assertIn("$SCRATCH/key", value)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin", default=str(REPO / "target/debug/herdr-threads"))
    parser.add_argument("--pi")
    parser.add_argument("--modules", default=str(MODULES))
    parser.add_argument("--evidence-dir", default=str(REPO / ".superpowers/sdd/implementation-plan/task-3-tui"))
    parser.add_argument("--cases", default=",".join(CASES))
    parser.add_argument("--live", action="store_true")
    parser.add_argument("--openrouter-env-file")
    parser.add_argument("--model")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        return 0 if unittest.TextTestRunner().run(unittest.defaultTestLoader.loadTestsFromTestCase(DriverTests)).wasSuccessful() else 1
    if args.live and not args.openrouter_env_file:
        parser.error("--live requires the explicitly authorized --openrouter-env-file")
    cases = ["live"] if args.live else args.cases.split(",")
    if not cases or any(case not in (*CASES, "live") for case in cases):
        parser.error("unknown qualification case")
    os.umask(0o077)
    run = Run(args)
    ok = True
    try:
        run.start()
        for case in cases:
            started = time.monotonic()
            try:
                getattr(run, "case_" + case)()
                run.results.append({"case": case, "outcome": "pass", "elapsed_s": round(time.monotonic() - started, 3)})
                print(case + ": pass", flush=True)
            except BaseException as exc:
                run.results.append({"case": case, "outcome": "fail", "error": run.clean(type(exc).__name__ + ": " + str(exc))})
                raise
        require_complete(run.results, cases)
    except BaseException as exc:
        run.facts["error"] = run.clean(str(exc))
        print("pi qualification: " + run.clean(str(exc)), flush=True)
        ok = False
    finally:
        ok = run.cleanup() and ok
    print("cleanup: " + ("clean" if run.facts["cleanup"].get("leak_rc") == 0 and run.facts["cleanup"].get("root_removed") else "failed"), flush=True)
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
