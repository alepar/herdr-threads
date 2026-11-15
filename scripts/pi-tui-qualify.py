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
import sqlite3
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock
import urllib.request
import uuid

REPO = Path(__file__).resolve().parent.parent
FIXTURE = REPO / "scripts/pi_tui_fixtures/provider.mjs"
MODULES = REPO / "docs/superpowers/runs/2026-11-14-pi-harness/research/candidates/probe/node_modules"
WAKE = "herdr-threads: wake up"
CASES = ("setup", "safety", "threshold", "busy", "receipts", "ordering", "invitations", "negative", "failures", "lifecycle", "compaction", "handoff", "channel", "named", "notice_reload", "notice_page", "notice_wake")
HELPER_GRACE_TIMEOUT = 30
HELPER_TERM_TIMEOUT = 8
HELPER_KILL_TIMEOUT = 5
ROOT_CLEANUP_TIMEOUT = 35
SINK_DRAFT = "OWNED UNREAD NOTICE SINK DRAFT"
NOTICE_DRAFT = "OWNED NOTICE QUALIFICATION DRAFT"


def require_agent_sink(inspection, seat, sid):
    summary = inspection["summary"]
    assert summary["seat"] == seat and summary["active"] and summary["availability"] == "available", "notice sink is not available"
    bindings = [b for b in summary["bindings"] if b.get("native_session") == sid
                and b.get("binding_id") and b.get("harness") == "pi" and b.get("role") == "top_level"
                and b.get("connected") is True and not b.get("agent_id")
                and b.get("provenance") == "cooperative_top_level"]
    assert len(bindings) == 1, "notice sink lacks one exact connected admitted Pi session"
    return bindings[0]


def require_pending_cause(receipt, seat, now_ms):
    assert receipt.get("seat") == seat and receipt.get("effective_status") == "pending", "notice cause requires a pending agent receipt"
    deadline = receipt.get("effective_deadline")
    if deadline is None:
        deadline = receipt.get("deadline")
    assert isinstance(deadline, int) and not isinstance(deadline, bool) and deadline > now_ms, "notice cause lacks a future deadline"
    return deadline


def contains_frame(text, kind, identity, body):
    if not isinstance(text, str) or not isinstance(body, str) or not body:
        return False
    headers = list(re.finditer(r"^\[herdr-threads\] (message|notice|invitation|attention)\b[^\n]*\n", text, re.M))
    expected = "\n".join("  " + line if line else "" for line in re.split(r"\r\n|[\n\r\v\f\x85\u2028\u2029]", body))
    for index, header in enumerate(headers):
        if header.group(1) != kind or not re.search(r"(?<![A-Za-z0-9])" + re.escape(identity) + r"(?![A-Za-z0-9])", header.group()):
            continue
        end = headers[index + 1].start() if index + 1 < len(headers) else len(text)
        content = text[header.end():end]
        if index + 1 < len(headers) and content.endswith("\n\n"):
            content = content[:-2]
        if content == expected:
            return True
    return False


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
        self.main_sid = None
        self.sink = None
        self.notice_causes = []
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
                try:
                    p.communicate(timeout=HELPER_KILL_TIMEOUT)
                except subprocess.TimeoutExpired:
                    for stream in (p.stdin, p.stdout, p.stderr):
                        if stream:
                            stream.close()
                    p.wait(timeout=HELPER_KILL_TIMEOUT)
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

    def keys(self, *keys, pane=None):
        self.herdr("pane", "send-keys", pane or self.recipient, *keys)

    def prompt(self, text):
        self.herdr("pane", "send-text", self.recipient, text)
        self.keys("enter")

    def turn(self, text):
        settled = self.count("agent_settled")
        self.prompt(text)
        self.wait("native settlement", lambda: self.count("agent_settled") > settled, 90 if self.args.live else 30)
        self.wait("host idle", lambda: self.herdr("agent", "get", self.recipient)["result"]["agent"]["agent_status"] in ("idle", "done"))

    def capture(self, label, *, pane=None):
        pane = pane or self.recipient
        value = {"at": time.time(), "visible": self.herdr("pane", "read", pane, "--source", "visible"),
                 "styled": self.herdr("pane", "read", pane, "--source", "visible", "--format", "ansi"),
                 "detection": self.herdr("agent", "read", pane, "--source", "detection", check=False),
                 "agent": self.herdr("agent", "get", pane, check=False)}
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

    def notice_context(self, warning, body, since=0, kind="notice"):
        for row in self.rows()[since:]:
            if row["event"] != "context" or row["state"]["sid"] != self.main_sid:
                continue
            for message in row["data"]["messages"]:
                text, custom_type = message.get("content"), message.get("customType")
                if (isinstance(custom_type, str) and custom_type.startswith("herdr-threads:")
                        and contains_frame(text, kind, warning, body)
                        and any(e.get("type") == "custom_message" and e.get("customType") == custom_type
                                and e.get("content") == text for e in row["branch"])):
                    return {"at": row["at"], "sid": self.main_sid, "customType": custom_type, "content": text}
        return None

    def canonical_read(self, sql, args=()):
        # Test observation only. Never use this connection to create/settle a notice.
        path = self.root / "state/node/herdr-threads.db"
        db = sqlite3.connect(path.resolve().as_uri() + "?mode=ro", uri=True, timeout=1)
        try:
            db.row_factory = sqlite3.Row
            db.execute("PRAGMA query_only=ON")
            return [dict(row) for row in db.execute(sql, args)]
        finally:
            db.close()

    def notice_snapshot(self, cause):
        rows = self.canonical_read("""WITH warning(id) AS (
            SELECT warning_message_id FROM receipt_state WHERE message_id=?1 AND seat_id=?2
            UNION SELECT warning_message_id FROM receipts WHERE message_id=?1 AND seat_id=?2)
            SELECT b.binding_id,b.generation,b.native_session,b.execution_id,d.warning_id,d.thread_id,d.ordinal,
                   d.event_seq,m.sequence,COALESCE(f.offered_ordinal,0) frontier,
                   c.outbox_token,c.outbox_descriptors,c.outbox_retired_mask,
                   c.generation consumer_generation,c.execution consumer_execution,c.instance_id consumer_instance,
                   c.seat_id consumer_seat,s.instance_id
            FROM warning w JOIN digest_programmatic_warnings d ON d.warning_id=w.id AND d.seat_id=?3
            JOIN messages m ON m.id=d.warning_id
            JOIN occupant_bindings b ON b.seat_id=d.seat_id
            JOIN seats s ON s.id=b.seat_id AND s.generation=b.generation AND s.state='resolved'
            LEFT JOIN digest_notice_offer f ON f.binding_id=b.binding_id
            LEFT JOIN invitation_presentation_consumers c ON c.binding_id=b.binding_id AND c.consumer='watch'
            WHERE w.id IS NOT NULL AND b.harness='pi' AND b.role='top_level' AND b.agent_id=''
              AND b.native_session=?4 AND b.target_id=?5 AND b.ended_at IS NULL
              AND b.liveness='connected' AND b.registered_at IS NOT NULL
              AND b.observation_provenance='cooperative_top_level'
              AND b.target_id=s.target_id AND b.target_generation=s.target_generation""",
            (cause["message"], self.sink["seat"], self.seat, self.main_sid, self.recipient))
        assert len(rows) <= 1, "notice observer found ambiguous current binding"
        if not rows:
            return None
        state = rows[0]
        assert (state["consumer_generation"], state["consumer_execution"], state["consumer_instance"], state["consumer_seat"]) == (
            state["generation"], state["execution_id"], state["instance_id"], self.seat), "notice consumer is absent or stale for current binding"
        encoded = state.pop("outbox_descriptors") or "[]"
        assert len(encoded.encode()) <= 262144, "canonical descriptor observation exceeds bound"
        state["descriptors"] = json.loads(encoded)
        state["pending"] = state["ordinal"] > state["frontier"]
        state["notice_descriptor"] = next((descriptor for index, descriptor in enumerate(state["descriptors"])
            if descriptor["item"].get("kind") == "notice" and descriptor["item"].get("warning") == state["warning_id"]
            and descriptor["item"].get("thread") == state["thread_id"]
            and isinstance(descriptor["item"].get("body"), str) and descriptor["item"]["body"]
            and ("body" not in cause or descriptor["item"]["body"] == cause["body"])
            and descriptor.get("context_key") == f'dn1:{state["sequence"]}:{state["event_seq"]}:{state["ordinal"]}'
            and not (state["outbox_retired_mask"] or 0) & (1 << index)), None)
        return state

    def recipient_receipt(self, mid, seat):
        rows = self.ht("delivery", "recipients", mid, pane=self.sender, human=True)["result"]["data"]["items"]
        return next(row for row in rows if row["seat"] == seat)

    def probe_rows(self, probe):
        path = probe / "native.jsonl"
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

    def probe_command(self, command, expected_editor="", *, pane=None, probe=None, sid=None):
        # A public UI fixture replaces only an exact owned draft, atomically.
        # The controller still submits the registered command through the TUI.
        probe, pane, sid = probe or self.root, pane or self.recipient, sid or self.main_sid
        nonce = str(uuid.uuid4())
        request = probe / "prepare-command.json"
        request.write_text(json.dumps({"sid": sid, "nonce": nonce, "expected": expected_editor, "command": command}))
        try:
            def prepared():
                row = next((r for r in self.probe_rows(probe) if r["event"] == "command_prepared"
                            and r.get("nonce") == nonce and r["state"]["sid"] == sid), None)
                if row:
                    assert row["accepted"], "fixture refused unexpected editor contents"
                return row
            self.wait("owned command editor preparation", prepared)
            self.observe_state(command, probe=probe, sid=sid)
        finally:
            request.unlink(missing_ok=True)
        self.keys("enter", pane=pane)

    def start_notice_sink(self):
        if self.sink:
            return
        assert not self.args.live and self.provider == "ht-fixture", "notice sink requires credential-free scripted provider"
        probe = self.root / "notice-sink"
        profile = probe / "agent"
        (probe / "bin").mkdir(mode=0o700, parents=True)
        profile.mkdir(mode=0o700)
        (probe / "home").mkdir(mode=0o700)
        shutil.copyfile(self.root / "agent/settings.json", profile / "settings.json")
        fixture = probe / "provider.mjs"
        shutil.copyfile(FIXTURE, fixture)
        extension = profile / "extensions/herdr-threads/index.js"
        options = ["--provider", "ht-fixture", "--model", "scripted", "--extension", str(extension), "--extension", str(fixture)]
        extra = {"HT_PI_QUALIFY_ROOT": str(probe), "PI_CODING_AGENT_DIR": str(profile),
                 "HOME": str(probe / "home"), "HT_PI_QUALIFY_OPTS": shlex.join(options),
                 "PATH": str(probe / "bin") + ":" + self.env["PATH"]}
        env = {**self.env, **extra}
        shim = probe / "bin/pi"
        exports = {k: env[k] for k in ("HT_PI_QUALIFY_ROOT", "HT_PI_QUALIFY_MODULES", "HT_PI_QUALIFY_CLI", "PI_CODING_AGENT_DIR", "PI_OFFLINE", "HT_PI_QUALIFY_OPTS", "HOME", "PATH")}
        shim.write_text("#!/bin/sh\n" + "".join("export " + k + "=" + shlex.quote(v) + "\n" for k, v in exports.items())
                        + "exec " + ("node " if self.pi.suffix == ".js" else "") + shlex.quote(str(self.pi)) + ' "$@"\n')
        shim.chmod(0o700)
        self.ht("setup", "pi", "--harness-binary", str(shim), setup=True, extra_env=extra)
        self.facts.setdefault("installed_extensions", {})["sink"] = {
            "path": str(extension), "sha256": hashlib.sha256(extension.read_bytes()).hexdigest()}
        flags = sum((["--env", k + "=" + v] for k, v in env.items()), [])
        tab = self.herdr("tab", "create", "--workspace", self.workspace, "--cwd", str(self.root / "project"),
                         "--label", "pi-notice-sink", "--no-focus", *flags)
        self.sink = {"probe": probe, "profile": profile, "pane": tab["result"]["root_pane"]["pane_id"]}
        launch = self.ht("launch", "--kind", "pi", "--pane", self.sink["pane"], "--harness-binary", str(shim),
                         "--", *options, extra_env=extra)
        assert launch["launch"]["outcome"] == "started", "notice sink public launch did not start"
        self.sink["seat"] = launch["launch"]["seat"]
        row = self.wait("notice sink native startup", lambda: next((r for r in self.probe_rows(probe) if r["event"] == "session_start"), None))
        self.sink["sid"] = row["state"]["sid"]
        self.herdr("pane", "send-text", self.sink["pane"], "Owned notice sink baseline; reply READY without tools.")
        self.keys("enter", pane=self.sink["pane"])
        self.wait("notice sink baseline settled", lambda: any(r["event"] == "agent_settled" and r["state"]["sid"] == self.sink["sid"] for r in self.probe_rows(probe)))
        self.herdr("pane", "send-text", self.sink["pane"], SINK_DRAFT)
        self.sink["starts"] = sum(r["event"] == "before_agent_start" for r in self.probe_rows(probe))
        self.verify_notice_sink()
        self.facts["notice_sink"] = {"seat": self.sink["seat"], "sid": self.sink["sid"], "launch": launch,
                                     "binding": self.sink["binding"], "draft": SINK_DRAFT}

    def verify_notice_sink(self):
        sink = self.sink
        inspection = self.ht("seat", "inspect", "--binding-details", sink["seat"], pane=self.sender, human=True)["result"]["data"]
        sink["binding"] = require_agent_sink(inspection, sink["seat"], sink["sid"])
        state = self.observe_state(SINK_DRAFT, probe=sink["probe"], sid=sink["sid"])
        capture = self.capture("notice-sink-" + str(len(self.captures)), pane=sink["pane"])
        assert SINK_DRAFT in capture["styled"]["raw"], "notice sink literal draft is not visible"
        assert sum(r["event"] == "before_agent_start" for r in self.probe_rows(sink["probe"])) == sink["starts"], "notice sink unexpectedly ran"
        for cause in self.notice_causes:
            if not cause.get("recovered"):
                assert not any(r["event"] in ("context", "provider_request") and cause["cause_body"] in json.dumps(r)
                               for r in self.probe_rows(sink["probe"])), "unread sink received the cause in native model context"
        return state

    def new_notice_cause(self, label, hold=True):
        self.start_notice_sink()
        self.verify_notice_sink()  # Must precede the actual public send.
        body = "NOTICE CAUSE " + label + " " + str(uuid.uuid4())
        data = {"sid": self.main_sid, "target": self.sink["seat"], "body": body, "deadline": 8,
                "hold": NOTICE_DRAFT if hold else ""}
        self.observe_state(data["hold"], sid=self.main_sid)
        before = self.capture("notice-cause-" + label + "-before")
        since, starts = len(self.rows()), self.count("before_agent_start")
        self.probe_command("/probe-notice-cause " + json.dumps(data), NOTICE_DRAFT if hold else "")
        row = self.wait("actual agent-authored notice cause", lambda: next((r for r in self.rows()[since:]
                        if r["event"] == "notice_cause" and r["state"]["sid"] == self.main_sid
                        and r["body"] == body and r["target"] == self.sink["seat"]), None))
        mid = row["result"]["result"]["data"]
        receipt = self.recipient_receipt(mid, self.sink["seat"])
        deadline = require_pending_cause(receipt, self.sink["seat"], int(time.time() * 1000))
        self.verify_notice_sink()
        if hold:
            assert row["state"]["editor"] == NOTICE_DRAFT, "source command did not retain its owned draft"
            self.observe_state(NOTICE_DRAFT, sid=self.main_sid)
            self.quiet(starts, 0)
        cause = {"message": mid, "cause_body": body, "initial_receipt": receipt, "deadline": deadline, "event": row,
                 "run_starts_before_send": starts, "before_send": before}
        self.notice_causes.append(cause)
        return cause

    def hold_notice_editor(self):
        state = self.observe_state(sid=self.main_sid)["state"]
        assert state["editor"] in ("", NOTICE_DRAFT), "refusing to overwrite an unrelated primary draft"
        if not state["editor"]:
            self.herdr("pane", "send-text", self.recipient, NOTICE_DRAFT)
        self.observe_state(NOTICE_DRAFT, sid=self.main_sid)

    def pending_notice(self, cause):
        def pending():
            value = self.notice_snapshot(cause)
            return value if value and value["pending"] and value["notice_descriptor"] else None
        state = self.wait("real tagged pending canonical notice", pending, 35)
        cause.update(warning=state["warning_id"], ordinal=state["ordinal"], body=state["notice_descriptor"]["item"]["body"])
        assert cause["warning"] not in {c["warning"] for c in self.notice_causes if c is not cause and "warning" in c}, "old coalesced warning cannot prove a new case"
        return state

    def reload_notice_draft(self):
        self.observe_state(NOTICE_DRAFT, sid=self.main_sid)
        nonce = str(uuid.uuid4())
        path = self.root / "reload-hold.json"
        path.write_text(json.dumps({"sid": self.main_sid, "nonce": nonce, "editor": NOTICE_DRAFT}))
        try:
            self.probe_command("/probe-notice-reload", NOTICE_DRAFT)
            self.wait("public reload restored owned draft", lambda: next((r for r in self.rows()
                      if r["event"] == "reload_draft_held" and r.get("nonce") == nonce and r["state"]["sid"] == self.main_sid), None))
            self.wait("public reload screen restored", lambda: (lambda text: "Reloaded keybindings" in text
                      and "Reloading keybindings, extensions" not in text)(self.herdr("pane", "read", self.recipient, "--source", "visible").get("raw", "")))
            self.observe_state(NOTICE_DRAFT, sid=self.main_sid)
            self.capture("notice-reload-restored-" + nonce)
        finally:
            path.unlink(missing_ok=True)

    def assert_notice_settled(self, cause, since):
        proof = self.wait("exact canonical Notice frame in actual native branch and context",
                          lambda: self.notice_context(cause["warning"], cause["body"], since))
        state = self.wait("canonical notice frontier advanced", lambda: (lambda s: s if s and s["frontier"] >= cause["ordinal"] else None)(self.notice_snapshot(cause)))
        assert self.recipient_receipt(cause["message"], self.sink["seat"])["effective_status"] == "pending", "notice presentation unexpectedly settled the sink's distinct ACK"
        self.verify_notice_sink()
        return {"native_context": proof, "canonical_settlement": state, "sink_receipt_still_pending": True}

    def finish_notice_cause(self, cause):
        # Only after all notice assertions: let the real sink process the body,
        # so the next case cannot reuse this coalesced warning condition.
        sink = self.sink
        self.verify_notice_sink()
        before = sum(r["event"] == "agent_settled" for r in self.probe_rows(sink["probe"]))
        self.keys("enter", pane=sink["pane"])
        self.wait("intentional sink recovery settles", lambda: sum(r["event"] == "agent_settled" for r in self.probe_rows(sink["probe"])) > before)
        self.wait("actual sink context ACK", lambda: self.recipient_receipt(cause["message"], sink["seat"])["effective_status"] == "acknowledged")
        assert any(r["event"] == "context" and r["state"]["sid"] == sink["sid"] and cause["cause_body"] in json.dumps(r["data"])
                   for r in self.probe_rows(sink["probe"])), "sink recovery lacked actual native body delivery"
        cause["recovered"] = True
        sink["starts"] = sum(r["event"] == "before_agent_start" for r in self.probe_rows(sink["probe"]))
        self.herdr("pane", "send-text", sink["pane"], SINK_DRAFT)
        self.verify_notice_sink()
        self.turn("Owned notice cleanup boundary. Reply READY without tools.")

    def case_notice_reload(self):
        self.hold_notice_editor()
        cause = self.new_notice_cause("reload")
        before = self.pending_notice(cause)
        starts, since = cause["run_starts_before_send"], len(self.rows())
        self.quiet(starts, 0)
        assert not self.notice_context(cause["warning"], cause["body"])
        self.reload_notice_draft()
        replay = self.pending_notice(cause)
        self.quiet(starts, 0)
        assert replay["frontier"] < cause["ordinal"] and replay["notice_descriptor"] == before["notice_descriptor"]
        self.observe_state(NOTICE_DRAFT, sid=self.main_sid)
        self.keys("enter")
        proof = self.assert_notice_settled(cause, since)
        self.wait("notice reload recovery settled", lambda: any(r["event"] == "agent_settled" and r["state"]["sid"] == self.main_sid for r in self.rows()[since:]))
        self.facts["notice_reload"] = {"cause": cause, "before": before, "replay": replay, **proof}
        self.finish_notice_cause(cause)

    def set_private_channel(self, enabled):
        self.ht("daemon", "stop")
        settings = self.root / "state/node/settings.json"
        settings.write_text(json.dumps({"sync": {"discover": False}, "mod_delivery": "on" if enabled else "off"}) + "\n")
        self.ht("daemon", "ensure")

    def stage_notice_invitation(self):
        self.start_notice_sink()
        self.verify_notice_sink()
        sink = self.sink
        before = len(self.probe_rows(sink["probe"]))
        command = "/probe-notice-invite " + json.dumps({"sid": sink["sid"], "target": self.seat, "hold": SINK_DRAFT})
        self.probe_command(command, SINK_DRAFT, pane=sink["pane"], probe=sink["probe"], sid=sink["sid"])
        row = self.wait("actual agent-authored invitation", lambda: next((r for r in self.probe_rows(sink["probe"])[before:]
                        if r["event"] == "notice_invitation" and r["state"]["sid"] == sink["sid"] and r["target"] == self.seat), None))
        self.verify_notice_sink()
        return row

    def case_notice_page(self):
        self.hold_notice_editor()
        self.set_private_channel(False)
        invitation = self.stage_notice_invitation()
        cause = self.new_notice_cause("whole-page-positive")
        self.wait("canonical overdue warning projection", lambda: self.canonical_read(
            "SELECT warning_message_id FROM receipt_state WHERE message_id=? AND seat_id=? AND warning_message_id IS NOT NULL",
            (cause["message"], self.sink["seat"])), 30)
        self.set_private_channel(True)
        # Disabled watches stop. Restore through public reload while idle and
        # guarded, rather than assuming re-enable restarts a parked child.
        self.reload_notice_draft()
        page = self.pending_notice(cause)
        invitations = [d for index, d in enumerate(page["descriptors"])
                       if d["item"].get("kind") == "invitation" and d["item"]["content"]["thread"] == invitation["thread"]
                       and not (page["outbox_retired_mask"] or 0) & (1 << index)]
        assert len(invitations) == 1, "real notice and invitation were not retained in one canonical page"
        item = invitations[0]["item"]
        assert item["content"].get("truncated") is False, "whole-page invitation body is truncated"
        since = len(self.rows())
        self.quiet(cause["run_starts_before_send"], 0)
        assert not self.notice_context(cause["warning"], cause["body"])
        self.keys("enter")
        proof = self.assert_notice_settled(cause, since)
        invitation_proof = self.wait("exact invitation frame in actual native branch/context", lambda:
            self.notice_context(item["id"], item["content"]["body"], since, kind="invitation"))
        assert invitation_proof["customType"] == proof["native_context"]["customType"] and invitation_proof["content"] == proof["native_context"]["content"], "notice and invitation lacked one actual complete native proposal"
        participants = self.ht("thread", "participants", invitation["thread"], pane=self.sender, human=True)["result"]["data"]["items"]
        membership = next(r for r in participants if r["seat"] == self.seat)
        assert not membership["joined"], "presentation unexpectedly accepted the invitation"
        self.wait("notice-page recovery idle", lambda: self.herdr("agent", "get", self.recipient)["result"]["agent"]["agent_status"] in ("idle", "done"))
        self.probe_command("/probe-accept " + invitation["thread"])
        self.wait("actual invitation acceptance", lambda: any(r["event"] == "command" and r["args"] == ["accept", invitation["thread"]] and r["code"] == 0 for r in self.rows()[since:]))
        participants = self.ht("thread", "participants", invitation["thread"], pane=self.sender, human=True)["result"]["data"]["items"]
        assert next(r for r in participants if r["seat"] == self.seat)["joined"]
        self.facts["notice_page"] = {"cause": cause, "page": page, "invitation": invitation,
            "invitation_context": invitation_proof, "separate_public_accept": True, **proof,
            "scope": "native positive only; partial/transformed negatives belong to reviewed SDK/canonical layers; hidden controls not inferred from retained descriptors"}
        self.finish_notice_cause(cause)

    def case_notice_wake(self):
        self.start_notice_sink()
        self.wake_lane()
        self.hold_notice_editor()
        since, starts = len(self.rows()), self.count("before_agent_start")
        assert any(r["event"] == "agent_settled" and r["state"]["sid"] == self.main_sid for r in self.rows()), "notice-only wake lacks prior actual settlement"
        cause = self.new_notice_cause("notice-only-wake", hold=True)
        page = self.pending_notice(cause)
        self.quiet(starts, 0)
        held = self.observe_state(NOTICE_DRAFT, sid=self.main_sid)
        assert held["state"]["idle"] is True, "notice-only clear requires actual native idle state"
        before_clear = self.capture("notice-only-pending-before-clear")
        clear = {"input": "ctrl+c", "at": time.time(), "native_state": held, "capture": before_clear}
        self.keys("ctrl+c")
        # The host may already have submitted and completed its legitimate wake
        # before the controller samples again. This is a post-clear observation,
        # never an ephemeral empty-idle precondition or a pre-wake capture.
        after_clear = self.observe_state(sid=self.main_sid)
        self.capture("notice-only-after-clear")
        wake = self.wait("notice-only actual Native wake", lambda: next((r for r in self.rows()[since:]
                         if r["event"] == "before_agent_start" and r["state"]["sid"] == self.main_sid and r["data"].get("prompt") == WAKE), None), 90)
        assert wake["at"] >= clear["at"] * 1000, "notice wake preceded the owned clear"
        proof = self.assert_notice_settled(cause, since)
        self.wait("notice-only native wake settled", lambda: any(r["event"] == "agent_settled" and r["state"]["sid"] == self.main_sid for r in self.rows()[since:]))
        assert self.count("before_agent_start") == starts + 1, "notice-only scenario created extra runs"
        self.facts["notice_wake"] = {"cause": cause, "page": page, "owned_clear": clear,
                                     "post_clear_observation": after_clear, "actual_wake": wake, **proof}
        self.finish_notice_cause(cause)

    def quiet(self, starts, seconds=1):
        time.sleep(seconds)
        if self.count("before_agent_start") != starts:
            raise AssertionError("unexpected native run during quiet observation")

    def send_quiet(self, body, *, seconds=1, **kwargs):
        starts = self.count("before_agent_start")
        result = self.send(body, **kwargs)
        self.quiet(starts, seconds)
        return result

    def observe_state(self, editor=None, *, probe=None, sid=None):
        root = probe or self.root
        def rows():
            if probe is None:
                return self.rows()
            path = root / "native.jsonl"
            return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []
        state = next((r["state"] for r in reversed(rows()) if "state" in r), {})
        sid = sid or state.get("sid")
        if not sid:
            raise RuntimeError("native observation session unavailable")
        nonce = str(uuid.uuid4())
        (root / "observe-state.json").write_text(json.dumps({"sid": sid, "nonce": nonce}))
        def observed():
            row = next((r for r in reversed(rows()) if r.get("event") == "state"
                        and r.get("observation") == nonce and r["state"]["sid"] == sid), None)
            return row if row and (editor is None or row["state"]["editor"] == editor) else None
        return self.wait("fresh native editor observation", observed)

    def blocked_send(self, label, body, editor=None):
        observed = self.observe_state(editor)
        before = self.capture(label + "-before")
        starts = self.count("before_agent_start")
        before.update(native_observation=observed, run_starts=starts)
        self.send(body)
        self.quiet(starts, 2)
        after = self.capture(label + "-after")
        current = self.observe_state()
        after.update(native_observation=current, run_starts=self.count("before_agent_start"))
        self.quiet(starts, 0)
        assert before["styled"]["raw"] == after["styled"]["raw"], "blocked UI/editor changed"
        for field in ("sid", "editor", "provider", "model"):
            assert field in observed["state"] and observed["state"][field] == current["state"].get(field), "blocked native state changed: " + field

    def wake_lane(self, sid=None):
        # A refusal test must not pass merely because the preceding successful
        # wake still owns the host's ordinary 30-second spacing window.
        sid = sid or self.main_sid
        if not sid or not any(r["event"] == "session_start" and r["state"]["sid"] == sid for r in self.rows()):
            raise RuntimeError("wake lane lacks a native session witness")
        while True:
            wakes = [r["at"] / 1000 for r in self.rows() if r["event"] == "before_agent_start"
                     and r["state"]["sid"] == sid and r["data"].get("prompt") == WAKE]
            last = max(wakes) if wakes else None
            ready = last + 32 if last is not None else None
            remaining = ready - time.time() if ready is not None else 0
            if remaining <= 0:
                self.facts.setdefault("wake_lanes", []).append({"sid": sid, "last_native_start": last,
                                                               "eligible_at": ready, "observed_at": time.time()})
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
trap 'exit 143' TERM HUP INT
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
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                                     start_new_session=True)
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
        self.facts.setdefault("installed_extensions", {})["primary"] = {
            "path": str(production_extension), "sha256": hashlib.sha256(production_extension.read_bytes()).hexdigest()}
        self.facts["setup_status"] = self.ht("setup-status", "pi")
        self.ht("me", "init", pane=self.sender, human=True)
        self.facts["launch"] = self.ht("launch", "--kind", "pi", "--pane", self.recipient, "--harness-binary", str(shim), "--",
                                           "--provider", self.provider, "--model", self.model, *self.extension_args)
        if self.facts["launch"]["launch"]["outcome"] != "started":
            raise RuntimeError("public launch did not confirm startup")
        self.seat = self.facts["launch"]["launch"]["seat"]
        self.wait("native session_start", lambda: self.count("session_start"))
        self.main_sid = next(r["state"]["sid"] for r in self.rows() if r["event"] == "session_start")
        self.facts["main_session"] = self.main_sid
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
            self.blocked_send(label, "SAFETY " + label, editor=text)
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
        self.wake_lane()
        self.herdr("pane", "send-text", self.recipient, "first")
        self.keys("shift+enter")
        self.herdr("pane", "send-text", self.recipient, "second")
        self.keys("ctrl+a")
        self.blocked_send("multiline-home", "MULTILINE DRAFT", editor="first\nsecond")
        self.keys("ctrl+c")
        for command, label in ((None, "model-picker"), ("/settings", "settings-picker"), ("/probe-select", "inline-select")):
            self.wake_lane()
            if command:
                self.prompt(command)
            else:
                self.keys("ctrl+l")
            time.sleep(.3)
            self.blocked_send(label, "MODAL " + label, editor="")
            self.keys("esc")
            settled = self.count("agent_settled")
            self.send("MODAL RELEASE " + label)
            self.wait("scheduler wake after picker close", lambda: self.count("agent_settled") > settled, 45)
        self.facts["accepted_limits"] = ["external custom overlays", "whitespace-only draft with cursor returned to column zero"]

    def case_threshold(self):
        # Set the threshold as the current native process, not the controller.
        self.prompt("/probe-threshold 0"); time.sleep(.2)
        self.wake_lane()
        since = len(self.rows())
        self.send_quiet("PASSIVE-P1", priority=1, seconds=2)
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
            self.send("BUSY-EXACT-" + mode); self.quiet(starts, .5)
            path.touch()
            self.wait("busy run settled", lambda: self.count("agent_settled") > settled)
            assert self.model_saw("BUSY-EXACT-" + mode)
            # Exceeds the first native wake retry; delivered attention must not trail the turn.
            self.quiet(starts, 31)
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
            self.send_quiet("ORDER-" + str(priority), priority=priority, seconds=.5)
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
        for mode in ("error", "abort"):
            (self.root / "release-text").unlink(missing_ok=True)
            (self.root / "mode.txt").write_text("hold-error" if mode == "error" else "hold-text")
            held = self.count("stream_held"); settled = self.count("agent_settled")
            self.prompt("qualification failed boundary " + mode)
            self.wait("held failure boundary", lambda: self.count("stream_held") > held)
            starts = self.count("before_agent_start")
            since = len(self.rows())
            self.send_quiet("FAILED-BOUNDARY-" + mode, priority=1, seconds=.5)
            if mode == "abort":
                self.keys("esc")
            else:
                (self.root / "release-text").touch()
            self.wait("failed native run settles", lambda: self.count("agent_settled") > settled)
            self.quiet(starts, .5)
            assert self.count("before_agent_start") == starts
            # Non-receipt p1 rows remain unread until the next actual user turn.
            assert not any(r["event"] == "context" and "FAILED-BOUNDARY-" + mode in json.dumps(r) for r in self.rows()[since:]), "failed " + mode + " boundary injected pending mail"
            self.turn("User recovery after " + mode + ". Reply READY without tools.")
            assert self.model_saw("FAILED-BOUNDARY-" + mode)
        self.cancel_settle_probe()
        self.facts["failed_boundaries"] = ["provider error", "native abort", "cancellation at settle"]

    def cancel_settle_probe(self):
        # A requested receipt is fixed p0. Keep an owned draft in the composer
        # after abort so a legitimate Native wake cannot mask the receipt proof.
        for name in ("settle-barrier", "cancel-settle"):
            (self.root / name).touch()
        (self.root / "release-settle").unlink(missing_ok=True)
        (self.root / "mode.txt").write_text("normal")
        barriers, settled = self.count("settle_barrier"), self.count("agent_settled")
        self.prompt("qualification cancellation at the actual settle boundary")
        self.wait("completed-turn cancellation barrier", lambda: self.count("settle_barrier") > barriers)
        barrier = [r for r in self.rows() if r["event"] == "settle_barrier"][-1]
        assert barrier["entries"] == [] and barrier["continue"] is False, "production turn boundary was not empty before the target send"
        starts, since = self.count("before_agent_start"), len(self.rows())
        body = "CANCELLED-SETTLE-EXACT " + str(uuid.uuid4())
        mid = self.send(body, ack=True)
        draft = "OWNED CANCEL RECOVERY DRAFT"
        self.herdr("pane", "send-text", self.recipient, draft)
        before_state = self.observe_state(draft)
        before = self.capture("cancel-settle-before")
        before.update(native_observation=before_state, run_starts=starts)
        assert self.receipt(mid)["effective_status"] == "pending", "target receipt settled while the earlier turn boundary was blocked"
        (self.root / "release-settle").touch()
        self.wait("cancelled settle boundary", lambda: self.count("agent_settled") > settled)
        self.quiet(starts, .5)
        after_state = self.observe_state()
        after = self.capture("cancel-settle-after")
        after.update(native_observation=after_state, run_starts=self.count("before_agent_start"))
        self.quiet(starts, 0)
        for field in ("sid", "editor", "provider", "model"):
            assert before_state["state"][field] == after_state["state"][field], "cancellation changed held " + field
        def draft_line(capture):
            lines = [line for line in capture["styled"]["raw"].splitlines() if draft in line]
            assert len(lines) == 1, "held draft is not uniquely visible in the composer"
            return lines[0]
        assert draft_line(before) == draft_line(after), "cancellation altered the styled composer draft"
        observed = self.rows()[since:]
        abort = next(r for r in observed if r["event"] == "settle_abort")
        assert mid in json.dumps(abort["entries"]) and body in json.dumps(abort["entries"]), "target was not proposed at the cancelled settle boundary"
        assert [r for r in observed if r["event"] == "agent_settled"][-1]["data"]["aborted"] is True
        assert not any(r["event"] in ("context", "provider_request") and mid in json.dumps(r) for r in observed), "cancelled settle draft reached model context"
        pending = self.receipt(mid)
        assert pending["effective_status"] == "pending", "cancelled unobserved draft supplied a receipt"
        for name in ("settle-barrier", "cancel-settle"):
            (self.root / name).unlink()
        settled = self.count("agent_settled")
        self.keys("enter")  # Submit the existing draft directly: no empty idle gap.
        self.wait("actual user recovery after cancellation", lambda: self.count("agent_settled") > settled)
        self.wait("recovery exact context receipt", lambda: self.receipt(mid)["effective_status"] == "acknowledged")
        assert self.model_saw(body, since), "recovery did not observe the exact cancellation target"
        self.facts["settle_cancellation"] = {"message": mid, "pending": pending, "recovered": self.receipt(mid),
                                             "barrier": barrier, "abort": abort, "draft_preserved": True}

    def case_compaction(self):
        self.turn("Synthetic compaction history " + "qualification data " * 150)
        before = self.count("session_compact")
        self.prompt("/probe-compact")
        self.wait("native compaction", lambda: self.count("session_compact") > before, 45)
        self.wait("compaction UI completion", lambda: self.count("compact_done"))
        self.turn("After native compaction, reply READY without tools.")
        self.facts["native_compaction"] = True

    def transformed_target(self, mid, body, since):
        for row in self.rows()[since:]:
            if (row["event"] != "transform_applied" or row["state"]["sid"] != self.main_sid
                    or row["target"] != {"mid": mid, "body": body, "sid": self.main_sid}):
                continue
            for index, before in enumerate(row["before"]):
                content = before.get("content", "")
                if (before.get("type") != "custom_message" or not before.get("customType", "").startswith("herdr-threads:")
                        or not isinstance(content, str) or f"[herdr-threads] message {mid} in " not in content
                        or "  " + body not in content.splitlines()):
                    continue
                after = row["after"][index]
                if (after.get("type") == "custom_message" and after.get("customType") == before["customType"]
                        and after.get("content") == "qualification replaced draft"):
                    return row, before["customType"]
        raise AssertionError("exact target frame and actual replacement were not observed at the native transformer")

    def case_negative(self):
        # A later native boundary transformer rejects the exact full-frame
        # proposal before persistence. A returned draft must not settle a receipt.
        (self.root / "mode.txt").write_text("hold-text")
        (self.root / "release-text").unlink(missing_ok=True)
        held = self.count("stream_held"); settled = self.count("agent_settled")
        self.prompt("qualification transformed boundary")
        self.wait("held negative run", lambda: self.count("stream_held") > held)
        since = len(self.rows())
        body = "NEGATIVE-EXACT-FRAME " + str(uuid.uuid4())
        mid = self.send(body, ack=True)
        (self.root / "transform").write_text(json.dumps({"mid": mid, "body": body, "sid": self.main_sid}))
        # Yield through an actual public-context observation. Only the exact
        # transform witness below proves that asynchronous watch mail arrived.
        self.observe_state("")
        (self.root / "release-text").touch()
        self.wait("transformed run settles", lambda: self.count("agent_settled") > settled)
        witness, custom_type = self.transformed_target(mid, body, since)
        assert any(row["event"] == "context" and row["state"]["sid"] == self.main_sid
                   and any(message.get("customType") == custom_type and message.get("content") == "qualification replaced draft"
                           for message in row["data"]["messages"]) for row in self.rows()[since:]), "returned replacement did not reach actual native context"
        assert not self.model_saw(body, since), "transformed target body reached native model context"
        pending = self.receipt(mid)
        assert pending["effective_status"] == "pending"
        (self.root / "transform").unlink()
        recovery_since = len(self.rows())
        self.turn("User retry after rejected draft. Reply READY without tools.")
        self.wait("retry exact receipt", lambda: self.receipt(mid)["effective_status"] == "acknowledged")
        assert self.model_saw(body, recovery_since), "retry did not observe the original full target body"
        self.facts["transformed_negative"] = {"message": mid, "body": body, "witness": witness,
                                               "pending": pending, "recovered": self.receipt(mid), "native_replacement_observed": True}
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
            self.wake_lane(sid)
            self.send_quiet("NAMED-PASSIVE-P1", priority=1)
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
            self.quiet(starts, .5)
            (self.root / "release-tool").touch()
            self.wait("named busy turn settles", lambda: self.count("agent_settled") > before)
            self.wait("named busy context receipt", lambda: self.receipt(busy_mid)["effective_status"] == "acknowledged")
            self.quiet(starts, 31)
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
        self.wake_lane()
        self.send_quiet("LIVE PASSIVE P1. Reply only RECEIVED when delivered.", priority=1, seconds=2)
        self.turn("Reply READY without tools.")
        models = {(r["state"]["provider"], r["state"]["model"]) for r in self.rows() if r.get("state") and r["event"] in ("before_agent_start", "context")}
        assert models == {(self.provider, self.model)}
        assert any(r["event"] == "after_provider_response" and r["data"]["status"] == 200 for r in self.rows())
        self.facts["live_model_continuity"] = sorted(models)

    def stop_helper(self, errors):
        helper = self.host
        try:
            helper.communicate("stop\n", timeout=HELPER_GRACE_TIMEOUT)
        except BaseException as exc:
            errors["helper_graceful"] = type(exc).__name__ + ": " + str(exc)
        for sig, seconds in ((signal.SIGTERM, HELPER_TERM_TIMEOUT), (signal.SIGKILL, HELPER_KILL_TIMEOUT)):
            if helper.poll() is not None:
                break
            try:
                # Only our own session may be signalled as a group. A helper
                # from a partially completed startup still has a known PID.
                if os.getpgid(helper.pid) == helper.pid:
                    os.killpg(helper.pid, sig)
                else:
                    helper.send_signal(sig)
                helper.communicate(timeout=seconds)
            except BaseException as exc:
                errors["helper_" + sig.name] = type(exc).__name__ + ": " + str(exc)
        for stream in (helper.stdin, helper.stdout, helper.stderr):
            if stream:
                try:
                    stream.close()
                except OSError:
                    pass
        helper.wait(timeout=HELPER_KILL_TIMEOUT)
        self.facts["cleanup"]["helper_rc"] = helper.returncode
        self.facts["cleanup"]["helper_reaped"] = True

    def owned_root(self):
        return (self.root is not None and not self.root.is_symlink()
                and self.root.parent in (Path("/tmp"), Path("/private/tmp"))
                and self.root.name.startswith("ih.")
                and (self.root / "owner").read_text().strip() == self.base["HT_TEST_OWNER"])

    def cleanup(self):
        cleanup = self.facts["cleanup"]
        errors = cleanup.setdefault("errors", {})
        trace = ""
        def attempt(name, action):
            try:
                result = action()
                cleanup[name] = result
                return result
            except BaseException as exc:
                errors[name] = type(exc).__name__ + ": " + str(exc)
                return None
        sink_trace = ""
        if self.sink:
            pane = self.sink["pane"]
            attempt("sink_capture", lambda: self.capture("notice-sink-final", pane=pane))
            attempt("sink_clear", lambda: self.herdr("pane", "send-keys", pane, "ctrl+c"))
            attempt("sink_quit_text", lambda: self.herdr("pane", "send-text", pane, "/quit"))
            attempt("sink_quit_submit", lambda: self.herdr("pane", "send-keys", pane, "enter"))
            path = self.sink["probe"] / "native.jsonl"
            if path.exists():
                sink_trace = attempt("sink_trace_read", path.read_text) or ""
                cleanup.pop("sink_trace_read", None)
        if self.recipient:
            attempt("final_capture", lambda: self.capture("final"))
            attempt("recipient_quit", lambda: (self.keys("esc"), self.keys("ctrl+c"), self.prompt("/quit")))
        if self.root and (self.root / "native.jsonl").exists():
            trace = attempt("trace_read", lambda: (self.root / "native.jsonl").read_text()) or ""
            cleanup.pop("trace_read", None)
        if self.tmux_socket:
            self.facts["named_final_screen"] = attempt("named_capture", lambda: self.run(["tmux", "-S", self.tmux_socket, "capture-pane", "-p", "-e", "-t", "pi"], check=False)[1])
            attempt("tmux_stop", lambda: self.run(["tmux", "-S", self.tmux_socket, "kill-server"], check=False))
        if self.workspace:
            attempt("workspace_close", lambda: self.herdr("workspace", "close", self.workspace, check=False))
        if self.root:
            attempt("daemon_stop", lambda: self.ht("daemon", "stop", check=False))
            attempt("credential_remove", lambda: (self.root / "provider.env").unlink(missing_ok=True))
        if self.host:
            attempt("helper_stop", lambda: self.stop_helper(errors))
        if self.root and self.root.exists():
            def root_teardown():
                if not self.owned_root():
                    raise RuntimeError("refusing cleanup of an unowned root")
                lib = REPO / "scripts/lib/isolated-herdr.sh"
                return self.run(["bash", "-c", 'source "$1"; ih_teardown "$2"', "bash", lib, self.root],
                                {**self.base, "IH_LIB": str(lib), "HT_SKIP_HERDR_TESTS": "1"}, timeout=ROOT_CLEANUP_TIMEOUT)
            attempt("root_fallback", root_teardown)
        scan = [REPO / "scripts/check-no-leaked-processes", "--run-id", self.run_id]
        checked = attempt("leak_scan", lambda: self.run(scan, self.base, check=False))
        if checked and checked[0]:
            errors["leaks_detected"] = "owned run left resources; forced cleanup required"
            attempt("leak_kill", lambda: self.run([*scan, "--grace", "0", "--kill"], self.base, check=False))
            checked = attempt("leak_rescan", lambda: self.run(scan, self.base, check=False))
        cleanup.update(leak_rc=checked[0] if checked else -1,
                       leak_stdout=checked[1] if checked else "", leak_stderr=checked[2] if checked else "")
        if self.root and self.root.exists() and cleanup["leak_rc"] == 0:
            def remove_root():
                if not self.owned_root():
                    raise RuntimeError("refusing removal of an unowned root")
                shutil.rmtree(self.root)
            attempt("root_remove", remove_root)
        cleanup["root_removed"] = self.root is not None and not self.root.exists()
        cleanup["ok"] = cleanup.get("helper_rc") == 0 and cleanup["root_removed"] and cleanup["leak_rc"] == 0 and not errors
        for name, value in (("manifest.json", self.facts), ("results.json", self.results), ("commands.json", self.commands), ("captures.json", self.captures)):
            (self.ev / name).write_text(self.clean(json.dumps(value, indent=2)) + "\n")
        (self.ev / "native.jsonl").write_text(self.clean(trace))
        if self.sink:
            (self.ev / "native-sink.jsonl").write_text(self.clean(sink_trace))
        for secret in self.secrets:
            if any(secret in path.read_text() for path in self.ev.iterdir() if path.is_file()):
                shutil.rmtree(self.ev)
                raise RuntimeError("credential detected in retained evidence; evidence removed")
        return cleanup["ok"]


class DriverTests(unittest.TestCase):
    def test_sink_launch_uses_actual_public_grammar_and_isolated_profile(self):
        class ParserChecked(Exception):
            pass
        with tempfile.TemporaryDirectory(prefix="ht-pi-sink-grammar-") as root:
            run = Run.__new__(Run); run.root = Path(root); run.sink = None; run.provider = "ht-fixture"; run.args = argparse.Namespace(live=False)
            run.modules = MODULES; run.bin = REPO / "target/debug/herdr-threads"
            run.pi = MODULES / "@earendil-works/pi-coding-agent/dist/bundle/cli.js"
            run.workspace = "owned-workspace"; run.facts = {}
            primary_bin = run.root / "bin"; primary_bin.mkdir()
            primary_pi = primary_bin / "pi"; primary_pi.write_text('#!/bin/sh\nexit 93\n'); primary_pi.chmod(0o700)
            run.env = {"PATH": str(primary_bin) + ":" + os.environ["PATH"], "HT_PI_QUALIFY_MODULES": str(MODULES),
                       "HT_PI_QUALIFY_CLI": '["owned-cli","--json"]', "PI_OFFLINE": "1"}
            (run.root / "agent").mkdir(); (run.root / "agent/settings.json").write_text('{}')
            setup_env, pane_env = [], {}
            def ht(*args, **kwargs):
                if args[0] == "setup":
                    setup_env.append(kwargs["extra_env"])
                    installed = run.root / "notice-sink/agent/extensions/herdr-threads/index.js"
                    installed.parent.mkdir(parents=True); installed.write_text('synthetic installed artifact for selftest only')
                    return {}
                self.assertEqual(args[0], "launch")
                requested = args[args.index("--harness-binary") + 1]
                for label, env in (("launch caller", {**run.env, **kwargs["extra_env"]}), ("native pane", pane_env)):
                    selected = subprocess.run(['/bin/sh', '-c', 'command -v pi'],
                        env={**env, "HT_TEST_OWNER": str(os.getpid()), "HT_LEAK_RUN_ID": os.environ.get("HT_LEAK_RUN_ID", str(uuid.uuid4()))},
                        capture_output=True, text=True, timeout=5)
                    self.assertEqual(selected.returncode, 0, label + " cannot select native pi")
                    self.assertEqual(Path(selected.stdout.strip()).resolve(), Path(requested).resolve(), label + " PATH selects a different native executable")
                # --help invokes the real public parser without allocating a
                # host, launch plan, seat or process. Unknown flags still fail.
                result = subprocess.run([str(run.bin), *args[:args.index("--")], "--help"],
                    env={"PATH": os.environ["PATH"], "HOME": root, "HT_TEST_OWNER": str(os.getpid()),
                         "HT_LEAK_RUN_ID": os.environ.get("HT_LEAK_RUN_ID", str(uuid.uuid4()))},
                    cwd=REPO, capture_output=True, text=True, timeout=5)
                self.assertEqual(result.returncode, 0, result.stderr)
                raise ParserChecked()
            run.ht = ht
            def tab(*args):
                for index, arg in enumerate(args):
                    if arg == "--env":
                        key, value = args[index + 1].split("=", 1); pane_env[key] = value
                return {"result": {"root_pane": {"pane_id": "owned-sink"}}}
            run.herdr = tab
            with self.assertRaises(ParserChecked): run.start_notice_sink()
            profile = run.root / "notice-sink/agent"
            self.assertEqual(setup_env[0]["PI_CODING_AGENT_DIR"], str(profile))
            self.assertEqual(setup_env[0]["HOME"], str(run.root / "notice-sink/home"))
            shim = (run.root / "notice-sink/bin/pi").read_text()
            self.assertIn("export PI_CODING_AGENT_DIR=" + shlex.quote(str(profile)), shim)
            self.assertIn("export HOME=" + shlex.quote(str(run.root / "notice-sink/home")), shim)
            self.assertEqual(setup_env[0]["PATH"].split(":"), [str(run.root / "notice-sink/bin"), *run.env["PATH"].split(":")])
            self.assertIn("export PATH=" + shlex.quote(setup_env[0]["PATH"]), shim)
            installed = profile / "extensions/herdr-threads/index.js"
            original = 'synthetic installed artifact for selftest only'
            self.assertEqual(installed.read_text(), original, "identity capture changed the installation")
            self.assertEqual(run.facts["installed_extensions"]["sink"], {
                "path": str(installed), "sha256": hashlib.sha256(original.encode()).hexdigest()})

    def test_notice_wake_captures_pending_before_clearing_exact_owned_draft(self):
        run = Run.__new__(Run); run.main_sid = "main"; run.facts = {}
        events = [{"event": "session_start", "state": {"sid": "main"}}, {"event": "agent_settled", "state": {"sid": "main"}}]
        order = []
        run.rows = lambda: events
        run.count = lambda event: sum(r["event"] == event for r in events)
        run.start_notice_sink = lambda: None
        run.wake_lane = lambda: order.append("eligible")
        run.hold_notice_editor = lambda: order.append("owned-draft")
        def observe(editor=None, **_kwargs):
            if editor == "": self.assertIn("clear", order, "empty composer observed before staging pending proof")
            elif editor == NOTICE_DRAFT: self.assertNotIn("clear", order)
            return {"state": {"editor": editor, "idle": True}}
        run.observe_state = observe
        run.capture = lambda *_args: {}
        def cause(_label, hold):
            self.assertTrue(hold, "pending notice snapshot must be held against fast Native confirmation")
            self.assertIn("owned-draft", order)
            order.append("cause")
            return {"message": "cause"}
        run.new_notice_cause = cause
        def pending(cause):
            self.assertNotIn("clear", order)
            cause.update(warning="warning", body="canonical marker", ordinal=1)
            order.append("pending")
            return {"pending": True}
        run.pending_notice = pending
        def keys(key):
            self.assertEqual(key, "ctrl+c")
            self.assertIn("pending", order, "draft cleared before canonical body/key were captured")
            order.append("clear")
            events.extend([{"event": "before_agent_start", "at": time.time() * 1000, "state": {"sid": "main"}, "data": {"prompt": WAKE}},
                           {"event": "agent_settled", "state": {"sid": "main"}}])
        run.keys = keys
        run.quiet = lambda starts, _seconds: self.assertEqual(run.count("before_agent_start"), starts)
        run.wait = lambda _label, predicate, *_args: predicate()
        run.assert_notice_settled = lambda *_args: {"native_context": "actual-witness-placeholder-in-controller-unit-test"}
        run.finish_notice_cause = lambda _cause: order.append("cleanup")
        run.case_notice_wake()
        self.assertEqual(order, ["eligible", "owned-draft", "cause", "pending", "clear", "cleanup"])

    def test_real_fixture_notice_commands_and_atomic_editor_guard(self):
        # Fixture mechanics only: mocked exec is not canonical notice evidence.
        with tempfile.TemporaryDirectory(prefix="ht-pi-notice-fixture-") as root:
            code = r'''
import assert from 'node:assert/strict'
import fs from 'node:fs'
import { pathToFileURL } from 'node:url'
const root=process.env.HT_PI_QUALIFY_ROOT, callbacks=new Map(), commands=new Map(), calls=[]
let tick,editor='OWNED',reloads=0
globalThis.setInterval=fn=>{tick=fn;return {unref(){}}};globalThis.clearInterval=()=>{}
const ctx={isIdle:()=>true,model:{id:'scripted',provider:'ht-fixture'},
 sessionManager:{getSessionId:()=> 'main',getSessionFile:()=>null,getBranch:()=>[]},
 ui:{getEditorText:()=>editor,setEditorText:text=>{editor=text},onTerminalInput:()=>{}},reload:async()=>{reloads++}}
const pi={on:(event,fn)=>callbacks.set(event,[...(callbacks.get(event)||[]),fn]),
 registerCommand:(name,value)=>commands.set(name,value),registerTool:()=>{},
 exec:async(program,args)=>{calls.push({program,args,editor});return {code:0,stdout:JSON.stringify({result:{data:args[1]==='thread'?'thread':'message'}}),stderr:''}}}
const fixture=(await import(pathToFileURL(process.env.HT_PI_FIXTURE))).default;fixture(pi)
for(const fn of callbacks.get('session_start')) await fn({reason:'startup'},ctx)
const prepare=data=>{fs.writeFileSync(root+'/prepare-command.json',JSON.stringify(data));tick()}
prepare({sid:'foreign',nonce:'foreign',expected:'OWNED',command:'/probe-notice-reload'})
assert.equal(editor,'OWNED')
prepare({sid:'main',nonce:'wrong-editor',expected:'OTHER',command:'/probe-notice-reload'})
assert.equal(editor,'OWNED')
prepare({sid:'main',nonce:'correct',expected:'OWNED',command:'/probe-notice-reload'})
assert.equal(editor,'/probe-notice-reload');editor='NEW OWNED';tick();assert.equal(editor,'NEW OWNED')
const cause={sid:'main',target:'sink',body:'UNIQUE CAUSE',deadline:8,hold:'GUARD'}
await commands.get('probe-notice-cause').handler(JSON.stringify(cause),ctx)
assert.equal(calls[0].editor,'GUARD')
assert.deepEqual(calls[0].args,['--json','send','--seat','sink','--body','UNIQUE CAUSE','--require-ack','sink','--deadline','8'])
await assert.rejects(commands.get('probe-notice-cause').handler(JSON.stringify({...cause,sid:'foreign'}),ctx));assert.equal(calls.length,1)
await commands.get('probe-notice-invite').handler(JSON.stringify({sid:'main',target:'primary',hold:'SINK GUARD'}),ctx)
assert.deepEqual(calls[2].args,['--json','invite','thread','--seat','primary']);assert.equal(calls[2].editor,'SINK GUARD')
fs.writeFileSync(root+'/reload-hold.json',JSON.stringify({sid:'main',nonce:'reload',editor:'RELOAD GUARD'}))
for(const fn of callbacks.get('session_start')) await fn({reason:'reload'},ctx)
assert.equal(editor,'RELOAD GUARD')
await commands.get('probe-notice-reload').handler('',ctx);assert.equal(reloads,1)
const rows=fs.readFileSync(root+'/native.jsonl','utf8').trim().split('\n').map(JSON.parse)
assert.deepEqual(rows.filter(r=>r.event==='command_prepared').map(r=>[r.nonce,r.accepted]),[['wrong-editor',false],['correct',true]])
assert.equal(rows.find(r=>r.event==='notice_cause').result.result.data,'message')
assert.equal(rows.find(r=>r.event==='reload_draft_held').nonce,'reload')
assert.ok(!rows.some(r=>r.event==='context'||r.event==='provider_request'))
'''
            env = {"PATH": os.environ["PATH"], "HOME": root, "HT_PI_QUALIFY_ROOT": root,
                   "HT_PI_QUALIFY_MODULES": str(MODULES), "HT_PI_FIXTURE": str(FIXTURE),
                   "HT_PI_QUALIFY_CLI": '["owned-cli","--json"]', "HT_PI_QUALIFY_LIVE": "1",
                   "HT_TEST_OWNER": str(os.getpid()), "HT_LEAK_RUN_ID": os.environ.get("HT_LEAK_RUN_ID", str(uuid.uuid4()))}
            result = subprocess.run(["node", "--input-type=module", "-e", code], env=env, cwd=REPO,
                                    capture_output=True, text=True, timeout=5)
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_notice_observer_sql_requires_current_connected_resolved_owner(self):
        # A synthetic schema tests read-only query scope, never native authority.
        with tempfile.TemporaryDirectory(prefix="ht-pi-notice-scope-") as root:
            path = Path(root) / "state/node/herdr-threads.db"; path.parent.mkdir(parents=True)
            db = sqlite3.connect(path)
            db.executescript("""
                CREATE TABLE receipt_state(message_id,seat_id,warning_message_id);
                CREATE TABLE receipts(message_id,seat_id,warning_message_id);
                CREATE TABLE digest_programmatic_warnings(warning_id,seat_id,thread_id,ordinal,event_seq);
                CREATE TABLE messages(id,sequence);
                CREATE TABLE occupant_bindings(binding_id,seat_id,generation,native_session,execution_id,harness,role,agent_id,target_id,target_generation,ended_at,liveness,registered_at,observation_provenance);
                CREATE TABLE seats(id,generation,state,instance_id,target_id,target_generation);
                CREATE TABLE digest_notice_offer(binding_id,offered_ordinal);
                CREATE TABLE invitation_presentation_consumers(binding_id,consumer,generation,execution,instance_id,seat_id,outbox_token,outbox_descriptors,outbox_retired_mask);
                INSERT INTO receipt_state VALUES('cause','sink','warning');
                INSERT INTO digest_programmatic_warnings VALUES('warning','primary','thread',4,3);
                INSERT INTO messages VALUES('warning',2);
                INSERT INTO occupant_bindings VALUES('binding','primary',1,'main','exec','pi','top_level','','pane',2,NULL,'connected',1,'cooperative_top_level');
                INSERT INTO seats VALUES('primary',1,'resolved','instance','pane',2);
                INSERT INTO invitation_presentation_consumers VALUES('binding','watch',1,'exec','instance','primary','token','[]',0);
            """)
            db.commit()
            run = Run.__new__(Run); run.root = Path(root); run.sink = {"seat": "sink"}; run.seat = "primary"; run.main_sid = "main"; run.recipient = "pane"
            self.assertTrue(run.notice_snapshot({"message": "cause"})["pending"])
            for table, field, value, original in (("seats", "state", "unresolved", "resolved"), ("occupant_bindings", "liveness", "disconnected", "connected"),
                                                ("occupant_bindings", "target_generation", 1, 2), ("occupant_bindings", "registered_at", None, 1)):
                db.execute(f"UPDATE {table} SET {field}=?", (value,)); db.commit()
                self.assertIsNone(run.notice_snapshot({"message": "cause"}), field)
                db.execute(f"UPDATE {table} SET {field}=?", (original,)); db.commit()
            db.execute("UPDATE invitation_presentation_consumers SET execution='stale'"); db.commit()
            with self.assertRaisesRegex(AssertionError, "stale"): run.notice_snapshot({"message": "cause"})
            db.close()

    def test_notice_send_requires_fresh_sink_guard_before_public_cause(self):
        with tempfile.TemporaryDirectory(prefix="ht-pi-cause-order-") as root:
            run = Run.__new__(Run); run.root = Path(root); run.main_sid = "main"; run.facts = {}; run.notice_causes = []
            run.sink = {"seat": "sink"}; observed, order = [], []
            starts = [0]
            run.rows = lambda: observed
            run.count = lambda _event: starts[0]
            run.observe_state = lambda *_args, **_kwargs: {}
            run.capture = lambda *_args, **_kwargs: {"styled": {"raw": "owned composer before trigger"}}
            run.start_notice_sink = lambda: order.append("admitted")
            run.verify_notice_sink = lambda: order.append("draft-verified")
            def probe(command, *_args, **_kwargs):
                self.assertEqual(order[-1], "draft-verified", "send preceded fresh sink draft witness")
                data = json.loads(command.split(" ", 1)[1]); order.append("public-send")
                observed.append({"event": "notice_cause", "state": {"sid": "main", "editor": data["hold"]}, "body": data["body"], "target": "sink", "result": {"result": {"data": "cause", "thread": "pair"}}})
            run.probe_command = probe
            run.wait = lambda _label, predicate, *_args: predicate()
            run.recipient_receipt = lambda _mid, _seat: {"seat": "sink", "effective_status": "pending", "deadline": 2000}
            with mock.patch.object(time, "time", return_value=1):
                cause = run.new_notice_cause("test")
            self.assertEqual(cause["message"], "cause"); self.assertEqual(order, ["admitted", "draft-verified", "public-send", "draft-verified"])
            run.recipient_receipt = lambda _mid, _seat: {"seat": "sink", "effective_status": "not_required", "deadline": None}
            with mock.patch.object(time, "time", return_value=1), self.assertRaisesRegex(AssertionError, "pending agent receipt"):
                run.new_notice_cause("invalid-person-like-result")
            run.recipient_receipt = lambda _mid, _seat: {"seat": "sink", "effective_status": "pending", "deadline": 2000}
            def synchronous_run(*args, **kwargs):
                probe(*args, **kwargs)
                starts[0] += 1
            run.probe_command = synchronous_run
            with mock.patch.object(time, "time", return_value=1), self.assertRaisesRegex(AssertionError, "unexpected native run"):
                run.new_notice_cause("synchronous-unsafe-run")

    def test_partial_second_recipient_is_cleaned_after_interruption(self):
        self.cleanup_failure_fixture(KeyboardInterrupt(), capture_failure=True, second_recipient=True)

    def test_notice_cause_requires_agent_binding_and_future_pending_receipt(self):
        binding = {"binding_id": "binding", "native_session": "sink-sid", "harness": "pi", "role": "top_level", "connected": True, "provenance": "cooperative_top_level"}
        inspection = {"summary": {"seat": "sink", "active": True, "availability": "available", "bindings": [binding]}}
        require_agent_sink(inspection, "sink", "sink-sid")
        row = {"seat": "sink", "effective_status": "pending", "deadline": 2000}
        self.assertEqual(require_pending_cause(row, "sink", 1000), 2000)
        for bad in ({**row, "effective_status": "not_required"}, {**row, "effective_status": "acknowledged"}, {**row, "deadline": None}, {**row, "deadline": 999}, {**row, "seat": "other"}):
            with self.assertRaises(AssertionError): require_pending_cause(bad, "sink", 1000)
        for change in ({"harness": "human"}, {"provenance": "managed_launch"}, {"connected": False}, {"binding_id": None}, {"agent_id": "child"}):
            bad = {"summary": {**inspection["summary"], "bindings": [{**binding, **change}]}}
            with self.assertRaises(AssertionError): require_agent_sink(bad, "sink", "sink-sid")
        with self.assertRaises(AssertionError): require_agent_sink(inspection, "sink", "foreign-sid")
        # All facts exist separately; no single live binding has them together.
        mixed = {**inspection, "open_binding": {"harness": "pi", "provenance": "cooperative_top_level"},
                 "history": {"items": [{"kind": "binding", "data": binding}]},
                 "summary": {**inspection["summary"], "bindings": [{**binding, "connected": False}, {**binding, "native_session": "other"}]}}
        with self.assertRaises(AssertionError): require_agent_sink(mixed, "sink", "sink-sid")

    def test_notice_snapshot_rejects_stale_consumer_and_untagged_or_changed_notice(self):
        descriptor = {"context_key": "dn1:2:3:4", "item": {"kind": "notice", "warning": "warning", "thread": "thread", "body": "FULL BODY"}}
        good = {"binding_id": "binding", "generation": 1, "native_session": "main", "execution_id": "exec", "warning_id": "warning", "thread_id": "thread", "ordinal": 4,
                "event_seq": 3, "sequence": 2, "frontier": 0, "outbox_token": "token", "outbox_descriptors": json.dumps([descriptor]), "outbox_retired_mask": 0,
                "consumer_generation": 1, "consumer_execution": "exec", "consumer_instance": "instance", "consumer_seat": "primary", "instance_id": "instance"}
        run = Run.__new__(Run); run.sink = {"seat": "sink"}; run.seat = "primary"; run.main_sid = "main"; run.recipient = "pane"
        run.canonical_read = lambda *_args: [dict(good)]
        cause = {"message": "cause", "body": "FULL BODY"}
        self.assertEqual(run.notice_snapshot(cause)["notice_descriptor"], descriptor)
        for field, value in (("consumer_generation", 0), ("consumer_execution", "old"), ("consumer_instance", "other"), ("consumer_seat", "other")):
            run.canonical_read = lambda *_args, field=field, value=value: [{**good, field: value}]
            with self.assertRaisesRegex(AssertionError, "stale"): run.notice_snapshot(cause)
        for change in ({"context_key": None}, {"context_key": "dn1:2:3:5"}, {"item": {**descriptor["item"], "body": "FULL"}}):
            run.canonical_read = lambda *_args, change=change: [{**good, "outbox_descriptors": json.dumps([{**descriptor, **change}])}]
            self.assertIsNone(run.notice_snapshot(cause)["notice_descriptor"])

    def test_notice_context_requires_exact_body_current_session_and_branch(self):
        text = '[herdr-threads] new messages:\n\n[herdr-threads] notice notice:w-target in thread from sender:\n  FULL NOTICE BODY'
        message = {"role": "custom", "customType": "herdr-threads:proposal", "content": text}
        entry = {"type": "custom_message", "customType": message["customType"], "content": text}
        good = {"at": 123, "event": "context", "state": {"sid": "main"}, "data": {"messages": [message]}, "branch": [entry]}
        run = Run.__new__(Run); run.main_sid = "main"; run.rows = lambda: [good]
        self.assertIsNotNone(run.notice_context("w-target", "FULL NOTICE BODY"))
        for change in ("branch", "context", "sid", "body", "id"):
            bad = json.loads(json.dumps(good))
            if change == "branch": bad["branch"] = []
            elif change == "context": bad["data"]["messages"] = []
            elif change == "sid": bad["state"]["sid"] = "other"
            else:
                old, new = ("FULL NOTICE BODY", "FULL NOTICE") if change == "body" else ("w-target", "w-other")
                bad["data"]["messages"][0]["content"] = text.replace(old, new)
                bad["branch"][0]["content"] = text.replace(old, new)
            run.rows = lambda: [bad]
            self.assertIsNone(run.notice_context("w-target", "FULL NOTICE BODY"), change)

    def test_canonical_observer_cannot_write_database(self):
        # Synthetic metadata exercises the observer only, never native/canonical notice proof.
        import sqlite3
        with tempfile.TemporaryDirectory(prefix="ht-pi-observer-selftest-") as root:
            path = Path(root) / "state/node/herdr-threads.db"; path.parent.mkdir(parents=True)
            db = sqlite3.connect(path); db.execute("CREATE TABLE fixture(value TEXT)"); db.execute("INSERT INTO fixture VALUES('unchanged')"); db.commit(); db.close()
            run = Run.__new__(Run); run.root = Path(root)
            self.assertEqual(run.canonical_read("SELECT value FROM fixture"), [{"value": "unchanged"}])
            with self.assertRaises(sqlite3.OperationalError): run.canonical_read("DELETE FROM fixture")
            self.assertEqual(run.canonical_read("SELECT value FROM fixture"), [{"value": "unchanged"}])

    def test_negative_case_rejects_late_watch_without_a_target_proposal(self):
        with tempfile.TemporaryDirectory(prefix="ht-pi-negative-selftest-") as root:
            run = Run.__new__(Run)
            run.root, run.facts, run.main_sid = Path(root), {}, "main"
            counts, statuses, sent, observed = {}, {}, [], []
            run.rows = lambda: observed
            run.count = lambda event: counts.get(event, 0)
            def prompt(text):
                if text.startswith("/probe-read-ack "):
                    statuses[text.split()[-1]] = "acknowledged"
                else:
                    counts["stream_held"] = counts.get("stream_held", 0) + 1
            run.prompt = prompt
            def send(body, **_kwargs):
                mid = "m-target" if not sent else "m-truncated"
                sent.append(body); statuses[mid] = "pending"
                return mid
            run.send = send
            run.receipt = lambda mid: {"effective_status": statuses[mid]}
            def wait(label, predicate, *_args):
                if label == "transformed run settles":
                    counts["agent_settled"] = counts.get("agent_settled", 0) + 1
                result = predicate()
                self.assertTrue(result, label)
                return result
            run.wait = wait
            def retry(_text):
                # Delayed watch delivery only arrives for the real retry. The
                # earlier turn never proposed or transformed the target.
                statuses["m-target"] = "acknowledged"
                observed.append({"event": "context", "data": {"messages": sent[:1]}})
            run.turn = retry
            run.model_saw = lambda body, *_args: body.startswith("TRUNCATED-BEGIN") or any(body in str(row) for row in observed)
            run.observe_state = lambda *_args: {"state": {"editor": ""}}
            with self.assertRaisesRegex(AssertionError, "target frame"):
                run.case_negative()

    def test_actual_fixture_transform_return_requires_exact_target_observation(self):
        with tempfile.TemporaryDirectory(prefix="ht-pi-transform-selftest-") as root:
            code = r'''
import assert from 'node:assert/strict'
import fs from 'node:fs'
import {pathToFileURL} from 'node:url'
const root=process.env.HT_PI_QUALIFY_ROOT, handlers=new Map()
const pi={on(name,fn){if(!handlers.has(name))handlers.set(name,[]);handlers.get(name).push(fn)},registerCommand(){},registerTool(){},registerProvider(){}}
const {default:fixture}=await import(pathToFileURL(process.env.HT_PI_FIXTURE));fixture(pi)
const ctx={isIdle:()=>false,ui:{getEditorText:()=>''},sessionManager:{getSessionId:()=> 'main',getBranch:()=>[]},model:{id:'scripted',provider:'fixture'}}
const target={mid:'m-target',body:'FULL TARGET BODY',sid:'main'}
fs.writeFileSync(root+'/transform',JSON.stringify(target))
const entry={type:'custom_message',customType:'herdr-threads:proposal',content:'[herdr-threads] message m-target in thread from sender:\n  FULL TARGET BODY'}
async function emit(entries){let event={outcome:'completed',entries,continue:true};for(const fn of handlers.get('turn_end')){const result=await fn(event,ctx);if(result)event={...event,...result}}return event}
const returned=await emit([entry]);assert.equal(returned.entries[0].content,'qualification replaced draft')
await emit([])
const observed=fs.readFileSync(root+'/native.jsonl','utf8').split('\n').filter(Boolean).map(JSON.parse).filter(row=>row.event==='transform_applied')
assert.deepEqual(observed[0].after,returned.entries);assert.deepEqual(observed[0].before,[entry]);assert.deepEqual(observed[1].before,[])
process.stdout.write(JSON.stringify(observed))
'''
            env = {"PATH": os.environ["PATH"], "HOME": root, "HT_PI_QUALIFY_ROOT": root,
                   "HT_PI_QUALIFY_MODULES": str(MODULES), "HT_PI_FIXTURE": str(FIXTURE),
                   "HT_PI_QUALIFY_LIVE": "1", "HT_TEST_OWNER": str(os.getpid()),
                   "HT_LEAK_RUN_ID": os.environ.get("HT_LEAK_RUN_ID", str(uuid.uuid4()))}
            result = subprocess.run(["node", "--input-type=module", "-e", code], env=env, cwd=REPO,
                                    capture_output=True, text=True, timeout=5)
            self.assertEqual(result.returncode, 0, result.stderr)
            observed = json.loads(result.stdout)
            run = Run.__new__(Run); run.main_sid = "main"
            run.rows = lambda: observed
            witness, custom_type = run.transformed_target("m-target", "FULL TARGET BODY", 0)
            self.assertEqual(witness["after"][0]["customType"], custom_type)
            with self.assertRaisesRegex(AssertionError, "target frame"):
                run.transformed_target("m-target", "FULL TARGET BODY", 1)
            for field, value in (("sid", "other"), ("mid", "m-other"), ("body", "FULL TARGET"), ("replacement", "unchanged")):
                row = json.loads(json.dumps(observed[0]))
                if field == "sid": row["state"]["sid"] = value
                elif field == "mid": row["before"][0]["content"] = row["before"][0]["content"].replace("m-target", value)
                elif field == "body": row["before"][0]["content"] = row["before"][0]["content"].replace("FULL TARGET BODY", value)
                else: row["after"][0]["content"] = row["before"][0]["content"]
                run.rows = lambda: [row]
                with self.subTest(field=field), self.assertRaisesRegex(AssertionError, "target frame"):
                    run.transformed_target("m-target", "FULL TARGET BODY", 0)

    def test_settle_fixture_barrier_precedes_send_and_follows_production_turn_handler(self):
        with tempfile.TemporaryDirectory(prefix="ht-pi-settle-selftest-") as root:
            code = r'''
import assert from 'node:assert/strict'
import fs from 'node:fs'
import { pathToFileURL } from 'node:url'
const root=process.env.HT_PI_QUALIFY_ROOT, handlers=new Map(), order=[]
let queued=false
const pi={on(name,fn){if(!handlers.has(name))handlers.set(name,[]);handlers.get(name).push(fn)},registerCommand(){},registerTool(){},registerProvider(){}}
pi.on('turn_end',()=>{assert.equal(queued,false);order.push('production-turn-end')})
pi.on('agent_before_settle',()=>{order.push('production-settle');return {entries:[{type:'custom_message',content:'CANCEL TARGET'}],continue:true}})
const {default:fixture}=await import(pathToFileURL(process.env.HT_PI_FIXTURE))
fixture(pi)
const ctx={isIdle:()=>false,ui:{getEditorText:()=> 'OWNED DRAFT'},sessionManager:{getSessionId:()=> 'test',getBranch:()=> []},model:{id:'scripted',provider:'fixture'},abort(){order.push('abort')}}
async function emit(name,event){for(const fn of handlers.get(name)??[]){const result=await fn(event,ctx);if(result)event={...event,...result}}return event}
fs.writeFileSync(root+'/settle-barrier','');fs.writeFileSync(root+'/cancel-settle','')
const pending=emit('turn_end',{type:'turn_end',outcome:'completed',entries:[],continue:false})
let seen
for(let i=0;i<200;i++){
 const text=fs.existsSync(root+'/native.jsonl')?fs.readFileSync(root+'/native.jsonl','utf8'):''
 seen=text.split('\n').filter(Boolean).map(JSON.parse).find(r=>r.event==='settle_barrier')
 if(seen)break
 await new Promise(resolve=>setTimeout(resolve,5))
}
assert.ok(seen,'completed-turn barrier was not reached')
assert.deepEqual(seen.entries,[]);assert.deepEqual(order,['production-turn-end'])
queued=true;order.push('send');fs.writeFileSync(root+'/release-settle','')
await pending
await emit('agent_before_settle',{type:'agent_before_settle',outcome:'completed',entries:[],continue:false})
assert.deepEqual(order,['production-turn-end','send','production-settle','abort'])
const logged=fs.readFileSync(root+'/native.jsonl','utf8').split('\n').filter(Boolean).map(JSON.parse)
assert.ok(logged.find(r=>r.event==='settle_abort'&&JSON.stringify(r.entries).includes('CANCEL TARGET')))
'''
            env = {"PATH": os.environ["PATH"], "HOME": root, "HT_PI_QUALIFY_ROOT": root,
                   "HT_PI_QUALIFY_MODULES": str(MODULES), "HT_PI_FIXTURE": str(FIXTURE),
                   "HT_PI_QUALIFY_LIVE": "1", "HT_TEST_OWNER": str(os.getpid()),
                   "HT_LEAK_RUN_ID": os.environ.get("HT_LEAK_RUN_ID", str(uuid.uuid4()))}
            result = subprocess.run(["node", "--input-type=module", "-e", code], env=env, cwd=REPO,
                                    capture_output=True, text=True, timeout=5)
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_passive_controls_are_cooldown_eligible_before_the_send(self):
        class Checked(Exception):
            pass
        for case in ("threshold", "live"):
            with self.subTest(case=case):
                run = Run.__new__(Run)
                now = [100.0]
                run.main_sid, run.facts = "main", {}
                run.rows = lambda: [{"event": "session_start", "state": {"sid": "main"}},
                                    {"event": "before_agent_start", "at": 100000, "state": {"sid": "main"}, "data": {"prompt": WAKE}}]
                run.count = lambda _event: 0
                run.prompt = lambda _text: None
                run.case_receipts = lambda: None
                def send(*_args, **_kwargs):
                    self.assertGreaterEqual(now[0], 132.0)
                    raise Checked()
                run.send = send
                with mock.patch.object(time, "time", side_effect=lambda: now[0]), mock.patch.object(time, "sleep", side_effect=lambda seconds: now.__setitem__(0, now[0] + seconds)), self.assertRaises(Checked):
                    getattr(run, "case_" + case)()

    def test_multiline_control_is_eligible_after_the_preceding_native_wake(self):
        class Checked(Exception):
            pass
        run = Run.__new__(Run)
        run.main_sid, run.facts, run.recipient = "main", {}, "owned-pane"
        now, settled = [100.0], [0]
        rows = [{"event": "session_start", "state": {"sid": "main"}}]
        run.rows = lambda: rows
        run.count = lambda event: settled[0] if event == "agent_settled" else sum(r["event"] == event for r in rows)
        run.capture = lambda _label: {"styled": {"raw": "equivalent controlled UI"}}
        run.herdr = lambda *_args, **_kwargs: None
        run.keys = lambda *_args: None
        run.wait = lambda _label, predicate, *_args: predicate()
        def send(*_args, **_kwargs):
            rows.append({"event": "before_agent_start", "at": now[0] * 1000, "state": {"sid": "main"}, "data": {"prompt": WAKE}})
            settled[0] += 1
        run.send = send
        run.model_saw = lambda *_args: True
        def blocked(label, *_args, **_kwargs):
            if label == "multiline-home":
                self.assertGreaterEqual(now[0], rows[-1]["at"] / 1000 + 32)
                raise Checked()
        run.blocked_send = blocked
        with mock.patch.object(time, "time", side_effect=lambda: now[0]), mock.patch.object(time, "sleep", side_effect=lambda seconds: now.__setitem__(0, now[0] + seconds)), self.assertRaises(Checked):
            run.case_safety()

    def test_named_control_uses_its_own_actual_native_start(self):
        class Checked(Exception):
            pass
        with tempfile.TemporaryDirectory(prefix="ht-pi-named-clock-selftest-") as root:
            run = Run.__new__(Run)
            run.root, run.main_sid, run.facts = Path(root), "main", {}
            run.env, run.provider, run.model = {"PATH": "/fake"}, "fixture", "model"
            run.shim, run.extension_args, run.sender, run.seat = Path("/fake/pi"), [], "sender", "main-seat"
            now = [100.0]
            rows = [{"event": "session_start", "state": {"sid": "main"}}]
            run.rows = lambda: rows
            run.count = lambda event: sum(r["event"] == event for r in rows)
            def command(argv, *_args, **_kwargs):
                if "new-session" in argv:
                    rows.append({"event": "session_start", "state": {"sid": "named"}})
                return 0, "", ""
            run.run = command
            run.ht = lambda *args, **_kwargs: {"result": {"data": {"items": [{"seat": "named-seat", "target": "pi-qualification-named", "active": True, "bindings": [{"harness": "pi", "connected": True}]}]}}}
            run.wait = lambda _label, predicate, *_args: predicate()
            run.receipt = lambda _mid: {"effective_status": "acknowledged"}
            def send(body, **_kwargs):
                if body == "NAMED-EXACT-WAKE":
                    rows.extend([{"event": "before_agent_start", "at": 100000, "state": {"sid": "named"}, "data": {"prompt": WAKE}},
                                 {"event": "agent_settled", "state": {"sid": "named"}},
                                 {"event": "before_agent_start", "at": 101000, "state": {"sid": "main"}, "data": {"prompt": WAKE}}])
                    now[0] = 101.0
                    return "message"
                self.assertEqual(body, "NAMED-PASSIVE-P1")
                self.assertEqual(now[0], 132.0, "only the named session's actual wake sets its lane")
                raise Checked()
            run.send = send
            with mock.patch.object(shutil, "which", return_value="/fake/tmux"), mock.patch.object(time, "time", side_effect=lambda: now[0]), mock.patch.object(time, "sleep", side_effect=lambda seconds: now.__setitem__(0, now[0] + seconds)), self.assertRaises(Checked):
                run.case_named()

    def cleanup_failure_fixture(self, interruption, ignore_term=False, capture_failure=False, second_recipient=False):
        with tempfile.TemporaryDirectory(prefix="ht-pi-driver-selftest-") as evidence:
            root = Path(tempfile.mkdtemp(prefix="ih."))
            args = argparse.Namespace(bin=sys.executable, modules=MODULES, pi=None,
                                      evidence_dir=evidence, live=False)
            run = Run(args)
            run.root = root
            (root / "owner").write_text(run.base["HT_TEST_OWNER"])
            (root / "provider.env").write_text("synthetic credential fixture")
            helper_code = '''import os,signal,subprocess,sys,time
from pathlib import Path
root=Path(sys.argv[1])
child=None if sys.argv[2]=='ignore' else subprocess.Popen([sys.executable,'-c','import time;time.sleep(120)',str(root)])
if child: (root/'child.pid').write_text(str(child.pid))
def stop(*_):
 if child: child.terminate();child.wait(timeout=3)
 raise SystemExit(143)
signal.signal(signal.SIGTERM,signal.SIG_IGN if sys.argv[2]=='ignore' else stop)
(root/'ready').touch()
while True: time.sleep(.05)
'''
            helper = subprocess.Popen([sys.executable, "-c", helper_code, str(root), "ignore" if ignore_term else "term"],
                                      env=run.base, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                      stderr=subprocess.PIPE, text=True, start_new_session=True)
            control = subprocess.Popen([sys.executable, "-c", "import time;time.sleep(120)"],
                                       env={k: v for k, v in run.base.items() if k not in ("HT_TEST_OWNER", "HT_LEAK_RUN_ID")},
                                       start_new_session=True)
            class InterruptedOnce:
                first = True
                def __getattr__(self, name):
                    return getattr(helper, name)
                def communicate(self, *args, **kwargs):
                    if self.first:
                        self.first = False
                        raise interruption
                    return helper.communicate(*args, **kwargs)
            run.host = InterruptedOnce()
            # Keep this injected-failure test fast without weakening its real,
            # exact-run process scan or replacing process cleanup with a mock.
            execute = run.run
            def short_scan(argv, *args, **kwargs):
                if str(argv[0]).endswith("check-no-leaked-processes"):
                    argv = [*argv, "--grace", "0"]
                return execute(argv, *args, **kwargs)
            run.run = short_scan
            run.start = lambda: None
            run.case_setup = lambda: None
            actions = []
            if second_recipient:
                probe = root / "sink"; probe.mkdir(); (probe / "native.jsonl").write_text('{"event":"fixture_loaded"}\n')
                run.sink = {"pane": "owned-sink-pane", "probe": probe}
            if capture_failure:
                run.recipient, run.workspace = "owned-fixture-pane", "owned-fixture-workspace"
                def fail_capture(_label):
                    raise RuntimeError("injected capture failure")
                run.capture = fail_capture
                run.keys = lambda *_args: None
                run.prompt = lambda *_args: None
                run.herdr = lambda *args, **_kwargs: actions.append(args) or {}
                run.ht = lambda *args, **_kwargs: actions.append(args) or {}
            try:
                run.wait("owned fake helper", lambda: (root / "ready").exists(), 5)
                child_pid = int((root / "child.pid").read_text()) if (root / "child.pid").exists() else None
                limits = {"HELPER_GRACE_TIMEOUT": .1, "HELPER_TERM_TIMEOUT": .2 if ignore_term else 3,
                          "HELPER_KILL_TIMEOUT": 3, "ROOT_CLEANUP_TIMEOUT": 8}
                with mock.patch.dict(globals(), limits), mock.patch.dict(main.__globals__, {"Run": lambda _args: run}), mock.patch("builtins.print"):
                    status = main(["--cases", "setup"])
                self.assertEqual(status, 1, "recovered teardown failure must still fail qualification")
                self.assertIsNotNone(helper.poll(), "owned helper was not terminated and reaped")
                self.assertFalse(root.exists(), "owned root survived failed graceful teardown")
                self.assertEqual(run.facts["cleanup"]["leak_rc"], 0, "owned descendants survived teardown")
                self.assertIsNone(control.poll(), "cleanup signalled an out-of-run control process")
                if child_pid:
                    with self.assertRaises(ProcessLookupError):
                        os.kill(child_pid, 0)
                if capture_failure:
                    self.assertIn(("workspace", "close", "owned-fixture-workspace"), actions)
                    self.assertIn(("daemon", "stop"), actions)
                if second_recipient:
                    self.assertIn(("pane", "send-keys", "owned-sink-pane", "ctrl+c"), actions)
                    self.assertIn(("pane", "send-text", "owned-sink-pane", "/quit"), actions)
                    self.assertTrue((run.ev / "native-sink.jsonl").exists())
                if ignore_term:
                    self.assertEqual(helper.returncode, -signal.SIGKILL)
            finally:
                if helper.poll() is None:
                    os.killpg(helper.pid, signal.SIGTERM)
                    try:
                        helper.communicate(timeout=4)
                    except subprocess.TimeoutExpired:
                        os.killpg(helper.pid, signal.SIGKILL)
                        helper.communicate(timeout=4)
                shutil.rmtree(root, ignore_errors=True)
                control.terminate()
                control.wait(timeout=4)

    def test_helper_timeout_terminates_reaps_descendants_and_removes_root(self):
        self.cleanup_failure_fixture(subprocess.TimeoutExpired("owned fake helper", .1))

    def test_helper_interruption_still_runs_independent_cleanup(self):
        self.cleanup_failure_fixture(KeyboardInterrupt(), capture_failure=True)

    def test_helper_that_ignores_term_is_killed_and_reaped(self):
        self.cleanup_failure_fixture(subprocess.TimeoutExpired("owned fake helper", .1), ignore_term=True)

    def test_passive_send_rejects_a_run_started_synchronously_inside_send(self):
        for case in ("threshold", "live"):
            with self.subTest(case=case):
                run = Run.__new__(Run)
                starts = [0]
                run.count = lambda _event: starts[0]
                run.main_sid = "main"
                run.rows = lambda: [{"event": "session_start", "state": {"sid": "main"}},
                                    {"event": "before_agent_start", "at": 0, "state": {"sid": "main"}, "data": {"prompt": WAKE}}]
                run.prompt = lambda _text: None
                run.case_receipts = lambda: None
                run.facts = {}
                def send(*_args, **_kwargs):
                    starts[0] += 1
                run.send = send
                def manual_turn(_text):
                    raise RuntimeError("negative assertion missed a synchronous wake")
                run.turn = manual_turn
                with mock.patch.object(time, "sleep"), self.assertRaisesRegex(AssertionError, "unexpected native run"):
                    getattr(run, "case_" + case)()

    def test_each_no_wake_guard_rejects_a_synchronous_send_start(self):
        for blocked in (False, True):
            with self.subTest(blocked=blocked):
                run = Run.__new__(Run)
                starts = [0]
                run.count = lambda _event: starts[0]
                run.capture = lambda _label: {"styled": {"raw": "unchanged"}}
                run.rows = lambda: [{"state": {"sid": "s1", "editor": "draft", "provider": "p1", "model": "m1"}}]
                run.observe_state = lambda _editor=None: run.rows()[-1]
                def send(*_args, **_kwargs):
                    starts[0] += 1
                run.send = send
                with mock.patch.object(time, "sleep"), self.assertRaisesRegex(AssertionError, "unexpected native run"):
                    if blocked:
                        run.blocked_send("draft-or-picker", "synthetic item")
                    else:
                        run.send_quiet("passive item")

    def test_blocked_send_rejects_ui_or_model_changes_without_a_run(self):
        for changed in ("screen", "model", "provider", "editor", "sid"):
            with self.subTest(changed=changed):
                run = Run.__new__(Run)
                state = {"sid": "s1", "editor": "draft", "provider": "fixture", "model": "m1"}
                screen = ["same selector and composer"]
                run.count = lambda _event: 0
                run.rows = lambda: [{"state": dict(state)}]
                run.observe_state = lambda _editor=None: run.rows()[-1]
                run.capture = lambda _label: {"styled": {"raw": screen[0]}}
                def send(*_args, **_kwargs):
                    if changed == "screen":
                        screen[0] = "picker consumed the wake marker"
                    else:
                        state[changed] = "changed"
                run.send = send
                with mock.patch.object(time, "sleep"), self.assertRaises(AssertionError):
                    run.blocked_send("modal", "synthetic item")

    def test_blocked_send_does_not_compare_a_stale_periodic_editor_sample(self):
        run = Run.__new__(Run)
        stale = {"sid": "s1", "editor": "    ", "provider": "p1", "model": "m1"}
        actual = {**stale, "editor": "first\nsecond"}
        samples = [stale, actual]
        run.rows = lambda: [{"state": samples.pop(0) if samples else actual}]
        run.observe_state = lambda _editor=None: {"state": dict(actual), "observation": "fresh"}
        run.count = lambda _event: 0
        run.capture = lambda _label: {"styled": {"raw": "identical rendered first/second draft"}}
        run.send = lambda *_args, **_kwargs: None
        with mock.patch.object(time, "sleep"):
            run.blocked_send("multiline-home", "synthetic item")

    def test_observer_waits_for_matching_nonce_session_and_intended_editor(self):
        with tempfile.TemporaryDirectory(prefix="ht-pi-observer-selftest-") as root:
            run = Run.__new__(Run)
            run.root = Path(root)
            stale = {"sid": "s1", "editor": "    ", "provider": "p1", "model": "m1"}
            ticks = [0]
            def rows():
                path = run.root / "observe-state.json"
                if not path.exists():
                    return [{"state": stale}]
                request = json.loads(path.read_text())
                ticks[0] += 1
                state = {**stale, "editor": "first\nsecond" if ticks[0] >= 3 else "    "}
                return [{"event": "state", "state": state,
                         "observation": request["nonce"] if ticks[0] >= 2 else "old-nonce"}]
            run.rows = rows
            with mock.patch.object(time, "sleep"):
                observed = run.observe_state("first\nsecond")
            self.assertEqual(ticks[0], 3)
            self.assertEqual(observed["state"]["editor"], "first\nsecond")
            self.assertEqual(json.loads((run.root / "observe-state.json").read_text())["sid"], "s1")

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
    print("cleanup: " + ("clean" if run.facts["cleanup"].get("ok") else "failed"), flush=True)
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
