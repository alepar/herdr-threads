#!/usr/bin/env python3
"""Isolated Herdr identity and recovery validation (ht-4is.11.5).

Drives the real herdr-threads CLI and daemon against a PRIVATE Herdr 0.9.1 server (private HOME,
XDG directories, config and API socket under one run directory) through these scenarios:

  R01 private host + test daemon bring-up           R07 missed events across daemon downtime
  R02 rename / tab + workspace reorder keep seats    R08 observed move of a registered seat
  R03 stand-in CLI exit retains the seat             R09 actual pane close retires
  R04 host socket denial (EACCES) + reconfirmation   R10 host stop/restore: holds, no auto-claim,
  R05 host socket unavailability (ENOENT)                operator repair labelled repair
  R06 test-daemon SIGKILL: deadlines never reset     R11 independent instance namespaces
  R13 cooperative idle wake: one overdue-warning    R12 shared server / foreign daemons untouched
      prompt into an idle stand-in agent, never a shell
Run order: R01 R02 R03 R13 R06 R07 R04 R05 R08 R09 R10 R11 R12 (sends before the host outages).

No model is launched: panes run stand-in shell commands (a script named `claude` that Herdr
classifies as an agent, then exits; R13's interactive variant echoes each input line it receives). Seat actions use the documented cooperative flags, so every
check-in/send/ACK here is a *stand-in cooperative* action, never model receipt evidence. The
shared Herdr server is never contacted, stopped or restarted; only daemons this run ensured and
the private servers it started are signalled.

Writes `results.json`, `report.md`, `commands.jsonl` and `evidence/*.json` into the run directory
and exits 0 only when every scenario PASSes.
"""

import argparse
import json
import os
from pathlib import Path
import platform
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
import traceback

sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "support"))
from private_host import (CommandLog, PrivateHerdr, SafetyError, TestDaemon, foreign_processes, pid_alive,
                          process_environment_mentions, sha256, utc_ms, wait_for)
from fixture import NativeFixture

RECONCILE_TIMEOUT = 40
# The daemon's fixed generic wake hint (src/notification/policy.rs MARKER).
WAKE_MARKER = "herdr-threads: attention pending; run herdr-threads inbox"
WAKE_RECEIVED = "HT-WAKE-RECEIVED: "
WAKE_DEADLINE_S = 5
# Quiet window after the first received prompt: shorter than the 30 s minimum wake spacing, so a second
# prompt inside it would be a storm, not a retry.
WAKE_QUIET_S = 20


class Scenario:
    def __init__(self, suite, sid, title, acceptance):
        self.suite, self.id, self.title, self.acceptance = suite, sid, title, acceptance
        self.checks, self.evidence, self.notes = [], [], []
        self.status = None
        self.error = None

    def check(self, name, condition, detail=""):
        ok = bool(condition)
        self.checks.append({"name": name, "ok": ok, "detail": detail if isinstance(detail, str) else json.dumps(detail, sort_keys=True)})
        return ok

    def note(self, text):
        self.notes.append(text)

    def save(self, name, data):
        path = self.suite.evidence_dir / f"{self.id}-{name}.json"
        path.write_text(json.dumps(data, indent=1, sort_keys=True) + "\n")
        self.evidence.append(str(path.relative_to(self.suite.run_dir)))
        return data

    def finish(self):
        if self.status is None:
            self.status = "PASS" if self.checks and all(c["ok"] for c in self.checks) else "FAIL"
        return {"id": self.id, "title": self.title, "acceptance": self.acceptance, "status": self.status,
                "checks": self.checks, "evidence": self.evidence, "notes": self.notes, "error": self.error}


class Suite:
    def __init__(self, args):
        self.args = args
        self.binary = Path(args.bin).resolve()
        # Short root: the private API socket must fit macOS's sockaddr_un limit.
        self.run_root = Path(args.run_root or "/private/tmp")
        if not str(self.run_root.resolve()).startswith("/private/tmp"):
            raise SafetyError("--run-root must be under /private/tmp")
        self.run_dir = Path(tempfile.mkdtemp(prefix="htr-", dir=str(self.run_root)))
        self.fixture = NativeFixture.create(self.run_dir / "fixture")
        self.evidence_dir = self.run_dir / "evidence"
        self.evidence_dir.mkdir(mode=0o700)
        self.log = CommandLog(self.run_dir / "commands.jsonl")
        self.state_dir = self.run_dir / "s"
        self.state_dir.mkdir(mode=0o700)
        self.host = PrivateHerdr(self.run_dir, self.log, "h")
        self.daemon = TestDaemon(self.binary, self.state_dir, self.host, self.log)
        self.second = None
        self.second_daemon = None
        self.bindir = self.run_dir / "bin"
        self.bindir.mkdir(mode=0o700)
        stub = self.bindir / "claude"
        # Stand-in only: Herdr classifies a foreground process named `claude` as an agent. It
        # prints a marker, sleeps and exits; no model or network is involved.
        stub.write_text("#!/bin/sh\necho HT-STANDIN-START\nsleep \"${1:-5}\"\necho HT-STANDIN-EXIT\n")
        stub.chmod(0o755)
        # R13's interactive stand-in (also named `claude`): echoes every line typed into it, so each Herdr prompt
        # the wake lane submits is visible exactly once in the pane. No model or network is involved.
        (self.bindir / "wake").mkdir(mode=0o700)
        self.wake_stub = self.bindir / "wake" / "claude"
        self.wake_stub.write_text("#!/bin/sh\necho HT-STANDIN-START\n"
                                  f"while IFS= read -r line; do echo \"{WAKE_RECEIVED}$line\"; done\n"
                                  "echo HT-STANDIN-EXIT\n")
        self.wake_stub.chmod(0o755)
        self.ids = {}
        self.results = []
        self.started = utc_ms()
        self.foreign_before = foreign_processes(self.run_dir)

    # ---------- helpers ----------
    def uid(self):
        return os.geteuid()

    def seat(self, seat_id):
        return self.daemon.seats().get(seat_id)

    def history(self, seat_id, kind=None):
        items = self.daemon.inspect(seat_id)["history"]["items"]
        return [item["data"] for item in items if kind is None or item["kind"] == kind]

    def reconciliations(self, count=2, since=None, timeout=RECONCILE_TIMEOUT):
        """Wait until `count` published reconciliations completed after `since` (UTC ms)."""
        since = utc_ms() if since is None else since
        seen = since
        for _ in range(count):
            marker = seen
            seen = wait_for(lambda: (lambda h: h and (h.get("last_reconciliation_at") or 0) > marker
                                     and h["last_reconciliation_at"])(self.daemon.health()),
                            timeout=timeout, description="a published host reconciliation")
        return seen

    def limitations(self):
        health = self.daemon.health() or {}
        return health.get("limitations") or []

    def reconciliation_failures(self):
        return [line for line in self.limitations() if "reconciliation failed" in line or "invalidated" in line]

    def checkin(self, seat_id, pane, tag):
        return self.daemon.run("check-in", "--lifecycle-event", f"hr-{tag}-{utc_ms()}", "--native-session", f"standin-{tag}",
                               pane=pane, coop=(seat_id, pane))

    def as_seat(self, seat_id, pane, *argv):
        return self.daemon.run(*argv, pane=pane, coop=(seat_id, pane))

    def pane_of(self, seat_id):
        return self.seat(seat_id)["target"]

    def split(self, pane, direction="right"):
        return self.host.api("pane.split", {"target_pane_id": pane, "direction": direction})["pane"]

    def move_to_workspace(self, pane, workspace_tab):
        moved = self.host.api("pane.move", {"pane_id": pane, "destination": {"type": "tab", "tab_id": workspace_tab, "split": "right"}})
        return moved["move_result"]

    def pending_for(self, seat_id, message):
        return next((item for item in self.daemon.pending(seat_id) if item["message"] == message), None)

    def scenario(self, sid, title, acceptance, body, needs=()):
        scenario = Scenario(self, sid, title, acceptance)
        blocked = [n for n in needs if not self.satisfied(n)]
        if blocked:
            scenario.status = "NOT_RUN"
            scenario.error = f"prerequisite scenario(s) did not complete: {', '.join(blocked)}"
        else:
            try:
                body(scenario)
            except Exception as error:  # record and continue; cleanup still runs
                scenario.error = f"{type(error).__name__}: {error}"
                scenario.check("scenario completed without exception", False, traceback.format_exc()[-2000:])
        result = scenario.finish()
        self.results.append(result)
        self.log.write({"kind": "scenario", "id": sid, "status": result["status"]})
        print(f"{sid} {result['status']}: {title}" + (f" ({result['error']})" if result["error"] else ""), flush=True)
        return result

    def satisfied(self, need):
        """R01 must PASS. Any other prerequisite that ran to completion (PASS, or FAIL on a checked
        expectation rather than an exception) leaves the state later scenarios build on."""
        result = next((r for r in self.results if r["id"] == need), None)
        if result is None or result["status"] == "NOT_RUN":
            return False
        return result["status"] == "PASS" or (need != "R01" and result["error"] is None)

    def latest_binding(self, seat_id):
        bindings = self.history(seat_id, "binding")
        return max(bindings, key=lambda b: b["ordinal"]) if bindings else None

    # ---------- scenarios ----------
    def r01(self, s):
        herdr = self.host.check_binary()
        s.save("herdr-binary", herdr)
        s.check("Herdr is the pinned 0.9.1 binary", herdr["pinned"], herdr)
        s.check("run directory is private under /private/tmp",
                str(self.run_dir).startswith(str(self.run_root / "htr-")) and str(self.run_dir).startswith("/private/tmp/")
                and (self.run_dir.stat().st_mode & 0o777) == 0o700)
        s.check("private server environment names only the private socket",
                self.host.environment()["HERDR_SOCKET_PATH"] == str(self.host.socket)
                and not any(k.startswith("HERDR_") and k not in ("HERDR_SOCKET_PATH", "HERDR_CONFIG_PATH")
                            for k in self.host.environment()))
        pid = self.host.start()
        self.fixture.record_owned("server", f"{self.host.name}:{pid}")
        pong = self.host.api("ping")
        s.check("private server speaks Herdr 0.9.1 protocol 22", pong.get("version") == "0.9.1" and pong.get("protocol") == 22, pong)
        workspace = self.host.api("workspace.create", {"label": "recovery", "cwd": str(self.run_dir), "focus": False,
                                                       "env": {"PATH": f"{self.bindir}:{os.environ.get('PATH', '')}"}})
        self.ids["w1"] = workspace["workspace"]["workspace_id"]
        self.ids["w1_tab"] = workspace["tab"]["tab_id"]
        self.ids["pA"] = workspace["root_pane"]["pane_id"]
        self.ids["pB"] = self.split(self.ids["pA"])["pane_id"]
        tab = self.host.api("tab.create", {"workspace_id": self.ids["w1"], "label": "c", "cwd": str(self.run_dir)})
        self.ids["t2"] = tab["tab"]["tab_id"]
        self.ids["pC"] = tab["root_pane"]["pane_id"]
        other = self.host.api("workspace.create", {"label": "other", "cwd": str(self.run_dir), "focus": False})
        self.ids["w2"] = other["workspace"]["workspace_id"]
        self.ids["w2_tab"] = other["tab"]["tab_id"]
        self.ids["w2_root"] = other["root_pane"]["pane_id"]
        health = self.daemon.ensure()
        self.fixture.record_owned("daemon", str(self.state_dir))
        s.save("health-initial", health)
        self.ids["instance"] = health["instance_id"]
        s.check("test daemon reachable with its own instance", bool(health.get("instance_id")))
        s.check("host reachability ready", health["host"]["reachability"] == "ready", health["host"])
        s.check("coherent enumeration supported (kernel peer-process incarnation witness)",
                health["host"]["coherent_enumeration"] == "supported", health["host"])
        endpoint = self.daemon.endpoint()
        s.check("daemon socket is private to this run's instance", endpoint["pid"] > 0)
        for label in ("A", "B", "C"):
            self.ids[label] = self.daemon.ok("seat", "resolve", "--pane", self.ids[f"p{label}"])
        s.check("ordinary resolve allocated three distinct seats", len({self.ids[k] for k in "ABC"}) == 3)
        for label in ("A", "B"):
            rc, data, out, err = self.checkin(self.ids[label], self.ids[f"p{label}"], f"{label}-initial")
            s.check(f"stand-in cooperative lifecycle check-in registers seat {label}", rc == 0 and data["context_disposition"] == "current",
                    err or out)
        self.ids["T"] = self.as_seat(self.ids["A"], self.ids["pA"], "thread", "create", "--topic", "host recovery", "--goal", "validation")[1]
        rc, invitation, _, err = self.as_seat(self.ids["A"], self.ids["pA"], "invite", self.ids["T"], "--seat", self.ids["B"], "--deadline", "900")
        s.check("A invites B", rc == 0, err)
        rc, _, _, err = self.as_seat(self.ids["B"], self.ids["pB"], "accept", self.ids["T"])
        s.check("B accepts (stand-in)", rc == 0, err)
        self.reconciliations(1)
        s.save("seats-initial", self.daemon.seats())
        s.save("ids", self.ids)

    def r02(self, s):
        before = self.daemon.seats()
        bindings = {k: self.history(self.ids[k], "binding") for k in "AB"}
        changed_at = utc_ms()
        self.host.api("pane.rename", {"pane_id": self.ids["pA"], "label": "renamed-a"})
        self.host.api("pane.rename", {"pane_id": self.ids["pB"], "label": "renamed-b"})
        self.host.api("tab.rename", {"tab_id": self.ids["w1_tab"], "label": "renamed-tab"})
        extra = self.host.api("tab.create", {"workspace_id": self.ids["w1"], "label": "reorder"})
        self.ids["t_reorder"] = extra["tab"]["tab_id"]
        tabs = self.host.api("tab.move", {"tab_id": self.ids["t_reorder"], "insert_index": 0})
        s.save("tab-order", tabs)
        workspaces = self.host.api("workspace.move", {"workspace_id": self.ids["w2"], "insert_index": 0})
        s.save("workspace-order", workspaces)
        panes = self.host.panes()
        s.check("Herdr kept pane addresses across rename/reorder",
                all(self.ids[f"p{k}"] in panes for k in "ABC"), sorted(panes))
        self.reconciliations(2, since=changed_at)
        after = s.save("seats-after", self.daemon.seats())
        for k in "ABC":
            seat = self.ids[k]
            s.check(f"seat {k} keeps its target", after[seat]["target"] == before[seat]["target"], [before[seat], after[seat]])
            s.check(f"seat {k} stays resolved at the same generation",
                    after[seat]["continuity"] == "resolved" and after[seat]["generation"] == before[seat]["generation"])
        for k in "AB":
            now = self.history(self.ids[k], "binding")
            s.check(f"seat {k} binding not ended by display changes", now == bindings[k], now)
        s.check("no reconciliation failure reported", not self.reconciliation_failures(), self.limitations())
        s.check("no seat retired", self.daemon.ok("seat", "retirements")["items"] == [])

    def r03(self, s):
        seat, pane = self.ids["A"], self.ids["pA"]
        before = self.seat(seat)
        self.host.run_in_pane(pane, "claude 6")
        present = wait_for(lambda: self.host.panes()[pane].get("agent") == "claude", timeout=20,
                           description="Herdr to observe the stand-in agent")
        s.check("Herdr observed the stand-in CLI as the pane's agent", present)
        observed_during = utc_ms()
        self.reconciliations(1, since=observed_during)
        s.save("seat-during-standin", self.seat(seat))
        wait_for(lambda: self.host.panes()[pane].get("agent") is None, timeout=30, description="the stand-in CLI to exit")
        text = self.host.pane_text(pane)
        s.check("stand-in CLI printed its exit marker (plain exit, pane stays)", "HT-STANDIN-EXIT" in text and pane in self.host.panes())
        exited_at = utc_ms()
        self.reconciliations(2, since=exited_at)
        after = s.save("seat-after-exit", self.seat(seat))
        s.check("CLI exit keeps the same seat on the same pane", after["target"] == before["target"] and after["retired_at"] is None,
                [before, after])
        s.check("CLI exit does not retire or unresolve the seat", after["continuity"] == "resolved", after)
        s.check("no retirement recorded", self.daemon.ok("seat", "retirements")["items"] == [])
        bindings = s.save("bindings-after-exit", self.history(seat, "binding"))
        s.note(f"latest binding after exit: ended_at={self.latest_binding(seat)['ended_at']} "
               "(a snapshot cannot see an occupant exit while its pane stays open; the next lifecycle hook, pane close or "
               "incarnation change does; the seat is retained either way)")
        rc, data, out, err = self.checkin(seat, pane, "A-after-exit")
        s.check("a fresh check-in in the same pane registers the retained seat", rc == 0, err or out)

    def agent_status(self, pane):
        result = self.host.api("agent.get", {"target": pane}, expect_ok=False)
        agent = result.get("agent") if isinstance(result, dict) else None
        return (agent.get("agent"), agent.get("agent_status")) if agent else (None, None)

    def received_prompts(self, pane):
        return [line[len(WAKE_RECEIVED):].strip() for line in self.host.pane_text(pane, lines=400).splitlines()
                if line.startswith(WAKE_RECEIVED)]

    def wake_rows(self, seat_id):
        """Read-only view of the seat's durable wake history and its overdue warnings."""
        path = self.daemon.instance_dir() / "threads.sqlite3"
        connection = sqlite3.connect(f"file:{path}?mode=ro", uri=True, timeout=5)
        connection.row_factory = sqlite3.Row
        try:
            wake = connection.execute("SELECT last_outcome,last_warning_seq,reservation_id FROM wake_work WHERE seat_id=?",
                                      (seat_id,)).fetchone()
            return dict(wake) if wake else None
        finally:
            connection.close()

    def r13(self, s):
        """Cooperative production wake (ht-4is.5.6): a stand-in agent seat misses a short receipt deadline; the one
        durable overdue warning is carried to the pane by exactly one prompt once Herdr reports the agent idle. While
        the pane is still a shell it is never prompted. The prompt settles nothing."""
        A = self.ids["A"]
        # A thread of its own (A and W only), so W's warnings are exactly this scenario's.
        T = self.ids["T_wake"] = self.as_seat(A, self.ids["pA"], "thread", "create", "--topic", "cooperative wake",
                                              "--goal", "validation")[1]
        tab = self.host.api("tab.create", {"workspace_id": self.ids["w1"], "label": "wake", "cwd": str(self.run_dir)})
        pane = self.ids["pW"] = tab["root_pane"]["pane_id"]
        W = self.ids["W"] = self.daemon.ok("seat", "resolve", "--pane", pane)
        rc, data, out, err = self.checkin(W, pane, "W-initial")
        s.check("stand-in cooperative check-in registers seat W", rc == 0 and data["context_disposition"] == "current",
                err or out)
        rc, _, _, err = self.as_seat(A, self.ids["pA"], "invite", T, "--seat", W, "--deadline", "900")
        s.check("A invites W", rc == 0, err)
        rc, _, _, err = self.as_seat(W, pane, "accept", T)
        s.check("W accepts (stand-in)", rc == 0, err)
        rc, message, _, err = self.as_seat(A, self.ids["pA"], "send", T, "--body", "wake probe", "--require-ack", W,
                                           "--deadline", str(WAKE_DEADLINE_S))
        s.check(f"A sends a {WAKE_DEADLINE_S}s required-ACK message to W", rc == 0, err)
        self.ids["M_wake"] = message
        time.sleep(3)  # the wake lane has attention for W while its pane is still a shell
        shell_text = s.save("pane-while-shell", {"text": self.host.pane_text(pane)})["text"]
        s.check("the shell pane was never prompted", WAKE_MARKER not in shell_text, shell_text[-600:])
        self.host.run_in_pane(pane, str(self.wake_stub))
        idle = wait_for(lambda: self.agent_status(pane) == ("claude", "idle"), timeout=30,
                        description="Herdr to report the stand-in agent idle")
        s.check("Herdr reports the interactive stand-in as an idle claude agent", idle, self.agent_status(pane))
        wait_for(lambda: self.daemon.warnings(W), timeout=WAKE_DEADLINE_S + 30, interval=0.5,
                 description="the overdue warning for W")
        warned_at = utc_ms()
        warnings = s.save("warnings-W", self.daemon.warnings(W))
        s.check("exactly one durable overdue warning for W", len(warnings) == 1, warnings)
        s.check("no prompt reached the pane before the warning", self.received_prompts(pane) == [],
                self.received_prompts(pane))
        wait_for(lambda: self.received_prompts(pane), timeout=120, interval=0.5,
                 description="the wake prompt in the idle stand-in pane")
        prompted_at = utc_ms()
        time.sleep(WAKE_QUIET_S)
        received = s.save("received-prompts", self.received_prompts(pane))
        wake = s.save("wake-work-W", self.wake_rows(W))
        s.note(f"warning observed at {warned_at}, first prompt observed at {prompted_at} "
               f"({(prompted_at - warned_at) / 1000:.1f}s later); quiet window {WAKE_QUIET_S}s")
        s.check(f"exactly one prompt reached the idle stand-in (no second prompt within {WAKE_QUIET_S}s)",
                len(received) == 1, received)
        s.check("the prompt is the fixed generic wake hint", received[:1] == [WAKE_MARKER], received)
        s.check("the durable attempt is recorded submitted and covered the warning",
                wake is not None and wake["last_outcome"] == "submitted" and wake["last_warning_seq"] is not None, wake)
        still = self.pending_for(W, message)
        s.check("the prompt settled nothing: the receipt is still pending and overdue",
                still is not None and still["overdue"] is True, still)
        rc, _, out, err = self.as_seat(W, pane, "ack", message)
        s.check("stand-in ACK settles the receipt (no further wake owed)", rc == 0 and self.pending_for(W, message) is None,
                err or out)
        s.check("the ACK adds no second warning", len(self.daemon.warnings(W)) == 1)

    def _socket_outage(self, s, mode):
        seats = {k: self.ids[k] for k in "ABC"}
        message = self.ids.get("M_outage")
        if message is None:
            rc, message, _, err = self.as_seat(self.ids["A"], self.ids["pA"], "send", self.ids["T"], "--body", "recovery probe",
                                               "--require-ack", self.ids["B"], "--deadline", "900")
            s.check("A sends a required-ACK message to B before the outage", rc == 0, err)
            self.ids["M_outage"] = message
        pending_before = self.pending_for(self.ids["B"], message)
        s.check("B has the pending receipt before the outage", pending_before is not None, pending_before)
        s.save("pending-before", pending_before)
        repairs_before = {k: self.history(v, "repair") for k, v in seats.items()}
        gens_before = {k: self.seat(v)["generation"] for k, v in seats.items()}
        away = self.host.socket.with_name(self.host.socket.name + ".away")
        if mode == "denied":
            os.chmod(self.host.socket, 0)
        else:
            os.rename(self.host.socket, away)
        outage_at = utc_ms()
        self.log.write({"kind": "host_socket_outage", "mode": mode})
        try:
            wait_for(lambda: (self.daemon.health() or {}).get("host", {}).get("reachability") == "unavailable", timeout=25,
                     description="the daemon to report the host unavailable")
            wait_for(lambda: all(self.seat(self.ids[k])["continuity"] == "unresolved" for k in "AB"), timeout=25,
                     description="registered seats to become unresolved")
            health = s.save("health-during", self.daemon.health())
            expected = ("Permission denied",) if mode == "denied" else ("No such file", "EndpointUnavailable")
            s.check(f"health names the host failure ({' or '.join(expected)})",
                    any(e in line for line in health["limitations"] for e in expected), health["limitations"])
            during = s.save("seats-during", self.daemon.seats())
            s.check("registered seats are unresolved, not retired, during the outage",
                    all(during[self.ids[k]]["continuity"] == "unresolved" and during[self.ids[k]]["retired_at"] is None for k in "AB"),
                    during)
            s.check("no seat is retired by a host error", all(v["retired_at"] is None for v in during.values())
                    and self.daemon.ok("seat", "retirements")["items"] == [])
            pending_during = self.pending_for(self.ids["B"], message)
            s.check("pending receipt kept with its deadline during the outage",
                    pending_during is not None and pending_during["deadline"] == pending_before["deadline"], pending_during)
            rc, _, out, err = self.checkin(self.ids["A"], self.ids["pA"], f"A-during-{mode}")
            s.check("check-in is refused while unresolved (nothing registered)", rc != 0 and "target_unresolved" in err, err)
            s.note(f"unregistered seat C during outage: {during[self.ids['C']]['continuity']}")
        finally:
            if mode == "denied":
                os.chmod(self.host.socket, 0o600)
            else:
                os.rename(away, self.host.socket)
            restored_at = utc_ms()
            self.log.write({"kind": "host_socket_restored", "mode": mode})
        wait_for(lambda: all(self.seat(self.ids[k])["continuity"] == "resolved" for k in "ABC"), timeout=RECONCILE_TIMEOUT,
                 description="structural reconfirmation of every seat")
        after = s.save("seats-after", self.daemon.seats())
        s.check("every seat reconfirmed on its original pane with no operator step",
                all(after[v]["target"] == self.ids[f"p{k}"] and after[v]["continuity"] == "resolved" for k, v in seats.items()), after)
        s.check("reconfirmation added no repair record", all(self.history(v, "repair") == repairs_before[k] for k, v in seats.items()))
        s.save("generations", {"before": gens_before, "after": {k: after[v]["generation"] for k, v in seats.items()}})
        health = s.save("health-after", self.daemon.health())
        s.check("host ready and no unresolved seats after restoration",
                health["host"]["reachability"] == "ready" and health["unresolved_seats"] == 0, health)
        pending_after = self.pending_for(self.ids["B"], message)
        s.check("pending receipt and deadline unchanged after restoration",
                pending_after is not None and pending_after["deadline"] == pending_before["deadline"], pending_after)
        for k in "AB":
            rc, _, out, err = self.checkin(self.ids[k], self.ids[f"p{k}"], f"{k}-after-{mode}")
            s.check(f"fresh check-in registers reconfirmed seat {k}", rc == 0, err or out)
        rc, _, out, err = self.as_seat(self.ids["A"], self.ids["pA"], "send", self.ids["T"], "--body", f"after {mode}")
        s.check("thread send works after reconfirmation and fresh check-in", rc == 0, err or out)
        rc, sent, out, err = self.as_seat(self.ids["A"], self.ids["pA"], "send", self.ids["T"], "--body", f"after {mode} ack",
                                          "--require-ack", self.ids["B"], "--deadline", "900")
        s.check("required-ACK send to the reconfirmed seat works", rc == 0, err or out)
        if rc != 0:
            s.save("send-failure-state", {"error": err.strip(), "seats": self.daemon.seats(),
                                          "bindings": {k: self.latest_binding(self.ids[k]) for k in "AB"}})
        s.note(f"outage lasted {restored_at - outage_at} ms")

    def r04(self, s):
        self._socket_outage(s, "denied")

    def r05(self, s):
        self._socket_outage(s, "missing")

    def r06(self, s):
        A, B = self.ids["A"], self.ids["B"]
        rc, message, _, err = self.as_seat(A, self.ids["pA"], "send", self.ids["T"], "--body", "deadline probe",
                                           "--require-ack", B, "--deadline", str(self.args.deadline_seconds))
        s.check("A sends a short-deadline required-ACK message to B", rc == 0, err)
        self.ids["M_deadline"] = message
        first = s.save("pending-initial", self.pending_for(B, message))
        rc, long_message, _, err = self.as_seat(A, self.ids["pA"], "send", self.ids["T"], "--body", "long deadline probe",
                                                "--require-ack", B, "--deadline", "900")
        s.check("A sends a long-deadline required-ACK message to B", rc == 0, err)
        long_before = self.pending_for(B, long_message)
        warnings_before = len(self.daemon.warnings(B))
        pid, boot = self.daemon.kill_daemon()
        s.check("only the test daemon was killed (argv names this run's state dir)",
                str(self.state_dir) in self.daemon.killed[-1]["command"])
        health = self.daemon.ensure()
        s.check("ensure starts a new daemon boot", health["boot_id"] != boot, [boot, health["boot_id"]])
        again = self.pending_for(B, message)
        s.check("deadline unchanged after daemon SIGKILL + ensure", again and again["deadline"] == first["deadline"]
                and again["decision_at"] == first["decision_at"], [first, again])
        long_again = self.pending_for(B, long_message)
        s.check("older receipt deadline unchanged too", long_again["deadline"] == long_before["deadline"])
        wait_for(lambda: utc_ms() > first["deadline"] + 1000, timeout=self.args.deadline_seconds + 30, interval=1,
                 description="the receipt deadline to pass")
        overdue_subject = f"overdue_receipt:{message}:{B}"
        wait_for(lambda: len(self.daemon.warnings(B)) > warnings_before, timeout=30, description="the overdue warning")
        warnings_after = s.save("warnings-after-deadline", self.daemon.warnings(B))
        s.check("exactly one overdue warning materialized", len(warnings_after) == warnings_before + 1, warnings_after)
        overdue = [item for item in self.daemon.ok("overdue", "--limit", "100")["items"] if item["subject"] == overdue_subject]
        s.check("receipt listed overdue once", len(overdue) == 1, overdue)
        pid2, boot2 = self.daemon.kill_daemon()
        health2 = self.daemon.ensure()
        s.check("second restart is a new boot", health2["boot_id"] != boot2, [boot2, health2["boot_id"]])
        time.sleep(8)  # several due-scan turns of the new daemon
        warnings_restart = s.save("warnings-after-second-restart", self.daemon.warnings(B))
        s.check("no duplicate warning after another restart", len(warnings_restart) == warnings_before + 1, warnings_restart)
        still = self.pending_for(B, message)
        s.check("receipt still pending (no ACK fabricated) with its original deadline",
                still is not None and still["deadline"] == first["deadline"] and still["overdue"] is True, still)
        rc, _, out, err = self.as_seat(B, self.ids["pB"], "ack", message)
        s.check("late stand-in ACK settles the receipt", rc == 0 and self.pending_for(B, message) is None, err or out)
        s.check("late settlement adds no second warning", len(self.daemon.warnings(B)) == warnings_before + 1)
        s.check("the killed pids were never foreign processes", all(str(k["pid"]) not in self.foreign_before for k in self.daemon.killed))

    def r07(self, s):
        C = self.ids["C"]
        before = self.seat(C)
        repairs = self.history(C, "repair")
        pid, boot = self.daemon.kill_daemon()
        moved = self.move_to_workspace(self.ids["pC"], self.ids["w2_tab"])
        s.save("move-result", moved)
        self.host.api("pane.rename", {"pane_id": moved["pane"]["pane_id"], "label": "moved-c"})
        s.check("Herdr gave the moved pane a new address and kept its terminal",
                moved["pane"]["pane_id"] != self.ids["pC"] and moved["previous_pane_id"] == self.ids["pC"], moved["pane"])
        self.ids["pC_old"], self.ids["pC"] = self.ids["pC"], moved["pane"]["pane_id"]
        time.sleep(2)  # the move and rename happen entirely while no daemon is listening
        self.daemon.ensure()
        followed = wait_for(lambda: self.seat(C)["target"] == self.ids["pC"], timeout=RECONCILE_TIMEOUT,
                            description="seat C to follow its moved pane")
        after = s.save("seat-after", self.seat(C))
        s.check("seat follows the missed move by snapshot after daemon reconnect", followed and after["continuity"] == "resolved")
        s.check("same seat and generation, no repair, not retired",
                after["seat"] == before["seat"] and after["generation"] == before["generation"]
                and after["retired_at"] is None and self.history(C, "repair") == repairs, [before, after])

    def r08(self, s):
        B = self.ids["B"]
        s.check("B is registered before its move", any(b.get("ended_at") is None for b in self.history(B, "binding")),
                self.history(B, "binding"))
        pane_d = self.split(self.ids["pA"], "down")["pane_id"]
        self.ids["pD"] = pane_d
        self.ids["D"] = self.daemon.ok("seat", "resolve", "--pane", pane_d)
        moved_b = self.move_to_workspace(self.ids["pB"], self.ids["w2_tab"])
        moved_d = self.move_to_workspace(pane_d, self.ids["w2_tab"])
        s.save("move-results", {"B": moved_b, "D": moved_d})
        self.ids["pB_old"], self.ids["pB"] = self.ids["pB"], moved_b["pane"]["pane_id"]
        self.ids["pD"] = moved_d["pane"]["pane_id"]
        moved_at = utc_ms()
        try:
            wait_for(lambda: self.seat(B)["target"] == self.ids["pB"] and self.seat(self.ids["D"])["target"] == self.ids["pD"],
                     timeout=RECONCILE_TIMEOUT, description="B and D to follow their moved panes")
        except TimeoutError:
            pass
        seats = s.save("seats-after-move", self.daemon.seats())
        health = s.save("health-after-move", self.daemon.health())
        s.check("registered seat B follows its observed move (same terminal, same verified host incarnation)",
                seats[B]["target"] == self.ids["pB"] and seats[B]["continuity"] == "resolved", seats[B])
        s.check("later seat D (unregistered) follows its observed move", seats[self.ids["D"]]["target"] == self.ids["pD"],
                seats[self.ids["D"]])
        failures = self.reconciliation_failures()
        s.check("no reconciliation failure reported after the move", not failures, failures)
        s.check("reconciliation keeps publishing after the move", (health.get("last_reconciliation_at") or 0) > moved_at,
                [moved_at, health.get("last_reconciliation_at")])
        s.check("B not retired or unresolved by its move", seats[B]["retired_at"] is None)

    def r09(self, s):
        B = self.ids["B"]
        pending_b = [item["message"] for item in self.daemon.pending(B)]
        s.save("pending-B-before-close", pending_b)
        live_b = self.host.pane_by_terminal(self.latest_binding(B)["terminal"])
        s.check("B's actual pane located by terminal", live_b is not None, live_b)
        close_target = live_b["pane_id"] if live_b else self.ids["pB"]
        self.host.api("pane.close", {"pane_id": close_target})
        self.log.write({"kind": "pane_closed", "pane": close_target, "seat": B})
        retired = wait_for(lambda: self.seat(B)["retired_at"] is not None, timeout=RECONCILE_TIMEOUT,
                           description="seat B retirement after pane close")
        s.check("actual pane close retires the seat", retired, self.seat(B))
        retirements = s.save("retirements", self.daemon.ok("seat", "retirements", "--limit", "100")["items"])
        s.check("retirement listed", any(item.get("seat") == B for item in retirements), retirements)
        s.save("inspect-B", self.daemon.inspect(B))
        s.check("retired seat's receipts leave pending without an ACK",
                wait_for(lambda: self.daemon.pending(B) == [], timeout=30, description="retirement cleanup"), self.daemon.pending(B))
        followed = False
        try:
            followed = wait_for(lambda: self.seat(self.ids["D"])["target"] == self.ids["pD"], timeout=RECONCILE_TIMEOUT,
                                description="seat D to follow its move once B is settled")
        except TimeoutError:
            pass
        s.note(f"seat D followed its earlier move after B retired: {bool(followed)}")
        self.host.api("pane.close", {"pane_id": self.ids["pD"]})
        wait_for(lambda: self.seat(self.ids["D"])["retired_at"] is not None, timeout=RECONCILE_TIMEOUT,
                 description="seat D retirement")
        s.check("closing an unregistered seat's pane retires it", self.seat(self.ids["D"])["retired_at"] is not None)
        fresh = self.split(self.ids["pA"], "down")["pane_id"]
        self.ids["pE"] = fresh
        rc, _, out, err = self.daemon.run("seat", "rebind", B, "--pane", fresh, "--operator")
        s.check("operator rebind cannot revive a retired seat", rc != 0, err or out)
        self.ids["E"] = self.daemon.ok("seat", "resolve", "--pane", fresh)
        s.check("a new pane gets a new seat, never the retired one", self.ids["E"] not in (B, self.ids["D"]))
        s.check("other seats unaffected by the closes",
                all(self.seat(self.ids[k])["continuity"] == "resolved" and self.seat(self.ids[k])["retired_at"] is None for k in "AC"))

    def r10(self, s):
        A, C, E = self.ids["A"], self.ids["C"], self.ids["E"]
        rc, _, _, err = self.checkin(E, self.ids["pE"], "E-initial")
        s.check("E checks in", rc == 0, err)
        rc, thread, _, err = self.as_seat(A, self.ids["pA"], "thread", "create", "--topic", "restore", "--goal", "validation")
        s.check("A creates a second thread", rc == 0, err)
        self.ids["T2"] = thread
        rc, _, _, err = self.as_seat(A, self.ids["pA"], "invite", thread, "--seat", E, "--deadline", "900")
        rc2, _, _, err2 = self.as_seat(E, self.ids["pE"], "accept", thread)
        s.check("E joins the second thread", rc == 0 and rc2 == 0, err or err2)
        rc, message, _, err = self.as_seat(A, self.ids["pA"], "send", thread, "--body", "restore probe",
                                           "--require-ack", E, "--deadline", "900")
        s.check("A sends a required-ACK message to E", rc == 0, err)
        pending_before = s.save("pending-E-before", self.pending_for(E, message))
        s.check("E's receipt timer started at send", pending_before is not None and pending_before["deadline"] is not None,
                pending_before)
        seats_before = s.save("seats-before", self.daemon.seats())
        live = [k for k in "ACE" if seats_before[self.ids[k]]["retired_at"] is None]
        retired = [k for k in ("B", "D")]
        old_pid = self.host.process.pid
        self.host.stop()
        new_pid = self.host.start()
        self.fixture.record_owned("server", f"{self.host.name}:{new_pid}")
        restored = s.save("restored-panes", {p: {"terminal": v["terminal_id"], "label": v.get("label")} for p, v in self.host.panes().items()})
        s.check("Herdr restored the saved addresses with fresh terminal IDs",
                all(self.ids[f"p{k}"] in restored for k in live)
                and all(restored[self.ids[f"p{k}"]]["terminal"] != self.latest_binding(self.ids[k])["terminal"]
                        for k in ("A", "E")), restored)
        wait_for(lambda: all(self.seat(self.ids[k])["continuity"] == "unresolved" for k in live), timeout=RECONCILE_TIMEOUT,
                 description="restored seats to become unresolved")
        after = s.save("seats-after-restore", self.daemon.seats())
        s.check("no restored seat retired", all(after[self.ids[k]]["retired_at"] is None for k in live))
        s.check("retired seats stay retired", all(after[self.ids[k]]["retired_at"] is not None for k in retired))
        s.check("no seat auto-claimed a restored pane", set(after) == set(seats_before)
                and all(after[self.ids[k]]["continuity"] == "unresolved" for k in live), after)
        rc, _, _, err = self.daemon.run("seat", "resolve", "--pane", self.ids["pA"])
        s.check("ordinary resolve of a restored saved address is refused", rc != 0 and "target_unresolved" in err, err)
        rc, _, _, err = self.daemon.run("seat", "resolve", "--pane", self.ids["w2_root"])
        s.check("ordinary resolve of a never-allocated restored pane is held too", rc != 0 and "target_unresolved" in err, err)
        rc, _, _, err = self.checkin(A, self.ids["pA"], "A-after-restore")
        s.check("startup check-in before repair is refused", rc != 0 and "target_unresolved" in err, err)
        pending_mid = self.pending_for(E, message)
        s.check("pending receipt deadline unchanged across host restore",
                pending_mid and pending_mid["deadline"] == pending_before["deadline"], pending_mid)
        self.daemon.kill_daemon()
        self.daemon.ensure()
        self.reconciliations(2)
        rc, _, _, err = self.daemon.run("seat", "resolve", "--pane", self.ids["pA"])
        s.check("holds persist across a daemon restart", rc != 0 and all(self.seat(self.ids[k])["continuity"] == "unresolved" for k in live),
                err)
        rc, _, _, err = self.daemon.run("seat", "rebind", A, "--pane", self.ids["pA"])
        s.check("repair without --operator is refused", rc != 0, err)
        rc, data, out, err = self.daemon.run("seat", "rebind", A, "--pane", self.ids["pA"], "--operator")
        s.check("operator rebind repairs A", rc == 0 and self.seat(A)["continuity"] == "resolved", err or out)
        repairs = s.save("repairs-A", self.history(A, "repair"))
        latest = max(repairs, key=lambda r: r["ordinal"]) if repairs else {}
        s.check("repair is labelled operator repair with the local-user actor and new host incarnation",
                latest.get("decision_kind") == "operator_rebind" and latest.get("operator_label") == f"operator:local-user:{self.uid()}"
                and f"pid={new_pid}:" in (latest.get("host_boot") or ""), latest)
        rc, _, _, err = self.as_seat(A, self.ids["pA"], "send", thread, "--body", "stale context")
        s.check("pre-repair context is fenced (no mutation before a fresh check-in)", rc != 0, err)
        rc, _, _, err = self.checkin(A, self.ids["pA"], "A-after-repair")
        s.check("fresh check-in after repair registers A", rc == 0, err)
        rc, _, _, err = self.daemon.run("seat", "rebind", E, "--pane", self.ids["pA"], "--operator")
        s.check("rebind onto an owned target is refused", rc != 0 and "target_already_owned" in err, err)
        rc, fresh_seat, out, err = self.daemon.run("seat", "resolve", "--pane", self.ids["pC"], "--new-seat", "--operator")
        s.check("explicit operator fresh role on a held target yields a new seat", rc == 0 and fresh_seat not in self.ids.values(), err or out)
        c_after = self.seat(C)
        s.check("the old seat keeps its unresolved state and history",
                c_after["seat"] == C and c_after["continuity"] == "unresolved" and c_after["retired_at"] is None, c_after)
        new_pane = self.split(self.ids["pA"], "down")["pane_id"]
        self.reconciliations(1)
        rc, new_seat, _, err = self.daemon.run("seat", "resolve", "--pane", new_pane)
        s.check("a pane created after the recovery baseline follows ordinary allocation", rc == 0 and new_seat, err)
        pending_end = s.save("pending-E-after", self.pending_for(E, message))
        s.check("E's receipt deadline still unchanged after the repair", pending_end is not None
                and pending_end["deadline"] == pending_before["deadline"], pending_end)
        s.note(f"restore: old server pid {old_pid}, new {new_pid}; E stays unresolved pending an operator choice")

    def r11(self, s):
        self.second = PrivateHerdr(self.run_dir, self.log, "g")
        pid = self.second.start()
        self.fixture.record_owned("server", f"g:{pid}")
        workspace = self.second.api("workspace.create", {"label": "second", "cwd": str(self.run_dir), "focus": False})
        pane = workspace["root_pane"]["pane_id"]
        self.second_daemon = TestDaemon(self.binary, self.state_dir, self.second, self.log)
        health = self.second_daemon.ensure()
        self.fixture.record_owned("daemon", f"{self.state_dir}#g")
        s.check("same state dir, second host: a distinct instance namespace", health["instance_id"] != self.ids["instance"],
                [self.ids["instance"], health["instance_id"]])
        s.check("second instance sees none of the first instance's seats", self.second_daemon.seats() == {})
        seat = self.second_daemon.ok("seat", "resolve", "--pane", pane)
        first = self.daemon.seats()
        s.check("same public address in the second host maps to a new seat", seat not in first, [pane, seat])
        s.check("first instance unchanged", all(self.ids[k] in first for k in "ACE") and seat not in first)
        s.save("instances", {"first": self.ids["instance"], "second": health["instance_id"], "second_pane": pane, "second_seat": seat})

    def r12(self, s):
        after = foreign_processes(self.run_dir)
        s.save("foreign-processes", {"before": self.foreign_before, "after": after})
        servers = {pid: info for pid, info in self.foreign_before.items() if "herdr server" in info["command"]}
        s.check("shared Herdr server(s) still running with the same pid and start time (never restarted)",
                servers and all(after.get(pid) == info for pid, info in servers.items()), [servers, after])
        killed = self.daemon.killed + (self.second_daemon.killed if self.second_daemon else [])
        s.save("killed-daemons", killed)
        gone = sorted(pid for pid in self.foreign_before if pid not in after)
        s.check("no foreign process was signalled by this run", not ({str(k["pid"]) for k in killed} & set(self.foreign_before)),
                {"killed": killed, "foreign_before": sorted(self.foreign_before)})
        if gone:
            s.note("foreign daemons that ended during the run under their owners' control (never signalled here): "
                   + ", ".join(f"{pid} {self.foreign_before[pid]['command'][:120]}" for pid in gone))
        s.check("every killed process was this run's test daemon",
                all(str(self.state_dir) in k["command"] and str(self.run_dir) in k["command"] for k in killed) and killed)
        commands = [json.loads(line) for line in (self.run_dir / "commands.jsonl").read_text().splitlines()]
        cli = [c for c in commands if c["kind"] == "cli"]
        s.check("every CLI call named a private host endpoint",
                cli and all(c["argv"][c["argv"].index("--host-endpoint") + 1].startswith(str(self.run_dir)) for c in cli))
        s.check("no model was launched (panes ran only the stand-in script)",
                not any(c.get("method") in ("agent.start",) for c in commands if c["kind"] == "herdr_api"))

    # ---------- run ----------
    def cleanup(self):
        problems = []
        for daemon in (self.second_daemon, self.daemon):
            if daemon is None:
                continue
            try:
                daemon.stop()
            except Exception as error:
                problems.append(f"daemon stop: {error}")
        for host in (self.second, self.host):
            if host is None:
                continue
            try:
                host.stop()
            except Exception as error:
                problems.append(f"server stop: {error}")
        try:
            self.fixture.cleanup(self)  # ledger: close_intent/closed for every recorded resource
        except Exception as error:
            problems.append(f"ledger cleanup: {error}")
        return problems

    def close_exact_owned_id(self, kind, resource_id):
        """Ledger closer: verify each recorded server/daemon is gone (they were stopped above)."""
        if kind == "server":
            pid = int(resource_id.split(":", 1)[1])
            if pid_alive(pid) and process_environment_mentions(pid, str(self.run_dir)):
                raise SafetyError(f"private server {resource_id} still running")
        elif kind == "daemon":
            daemon = self.second_daemon if resource_id.endswith("#g") else self.daemon
            if daemon is not None and daemon.health() is not None:
                raise SafetyError(f"test daemon {resource_id} still healthy")

    def run(self):
        self.log.write({"kind": "run_start", "bin": str(self.binary), "bin_sha256": sha256(self.binary), "run_dir": str(self.run_dir)})
        plan = [
            ("R01", "private Herdr host and test daemon", "isolated server/state; shared server never contacted", self.r01, ()),
            ("R02", "rename and reorder keep the seat", "pane/tab rename, tab and workspace reorder keep seat, target, generation, binding", self.r02, ("R01",)),
            ("R03", "CLI exit retains the seat", "plain CLI exit keeps the seat; a fresh check-in registers it", self.r03, ("R01",)),
            ("R13", "cooperative idle wake: one prompt for the overdue warning", "shell never prompted; idle stand-in agent receives exactly one generic hint after the durable warning; prompt settles nothing", self.r13, ("R01",)),
            ("R06", "daemon SIGKILL: deadlines never reset, one warning", "kill only the test daemon; ensure; deadline identical; exactly one overdue warning across restarts", self.r06, ("R01",)),
            ("R07", "missed events across daemon downtime recover by snapshot", "move+rename while no daemon runs; reconnect follows by snapshot", self.r07, ("R01",)),
            ("R04", "socket denial preserves unresolved state, then structural reconfirmation", "EACCES: unresolved not retired, receipts/deadlines kept; restored: reconfirmed without repair (crash fix3 real-host acceptance)", self.r04, ("R01",)),
            ("R05", "socket unavailability preserves unresolved state, then reconfirmation", "ENOENT outage: same as R04", self.r05, ("R01",)),
            ("R08", "observed move of a registered seat keeps the seat", "same terminal, same verified host incarnation: seat follows; later seats not blocked", self.r08, ("R01",)),
            ("R09", "pane close retires", "actual close retires; receipts leave pending without ACK; retired never revived", self.r09, ("R08",)),
            ("R10", "host stop/restore: no auto-claim, repair labelled repair", "fresh terminals at reused addresses held; holds survive daemon restart; operator rebind audited; owned target refused; fresh role; post-baseline allocation", self.r10, ("R09",)),
            ("R11", "independent instance namespaces", "same state dir, second private host: separate instance and seats", self.r11, ("R01",)),
            ("R12", "shared server and foreign daemons untouched", "only test daemons killed; shared server pid/start unchanged", self.r12, ()),
        ]
        only = set(self.args.only.split(",")) if self.args.only else None
        try:
            for sid, title, acceptance, body, needs in plan:
                if only and sid not in only and sid not in ("R01", "R12"):
                    continue
                if sid == "R12":
                    continue
                self.scenario(sid, title, acceptance, body, needs)
        finally:
            problems = self.cleanup()
            self.scenario("R12", plan[-1][1], plan[-1][2], lambda s: (self.r12(s), [s.check("cleanup completed", not problems, problems)]), ())
            self.write_report()
            self.log.close()
        return all(r["status"] == "PASS" for r in self.results)

    def write_report(self):
        summary = {
            "suite": "ht-4is.11.5 host identity and recovery", "run_dir": str(self.run_dir),
            "binary": {"path": str(self.binary), "sha256": sha256(self.binary)},
            "source_commit": subprocess.run(["git", "-C", str(Path(__file__).resolve().parents[3]), "rev-parse", "HEAD"],
                                            capture_output=True, text=True).stdout.strip(),
            "herdr": self.host.check_binary(), "platform": platform.platform(),
            "started_utc_ms": self.started, "finished_utc_ms": utc_ms(), "ids": self.ids,
            "counts": {status: sum(r["status"] == status for r in self.results) for status in ("PASS", "FAIL", "NOT_RUN")},
            "scenarios": self.results,
        }
        (self.run_dir / "results.json").write_text(json.dumps(summary, indent=1, sort_keys=True) + "\n")
        lines = [f"# Host recovery validation run {self.run_dir.name}", "",
                 f"Binary sha256 `{summary['binary']['sha256']}`, source `{summary['source_commit']}`, Herdr `{summary['herdr']['version']}` pinned={summary['herdr']['pinned']}.",
                 f"Counts: {summary['counts']}.", "", "| id | status | scenario | failed checks |", "|---|---|---|---|"]
        for r in self.results:
            failed = "; ".join(c["name"] for c in r["checks"] if not c["ok"]) or (r["error"] or "")
            lines.append(f"| {r['id']} | {r['status']} | {r['title']} | {failed} |")
        (self.run_dir / "report.md").write_text("\n".join(lines) + "\n")
        if self.args.evidence_out:
            out = Path(self.args.evidence_out) / self.run_dir.name
            out.mkdir(parents=True, exist_ok=True)
            for name in ("results.json", "report.md", "commands.jsonl"):
                shutil.copy2(self.run_dir / name, out / name)
            shutil.copytree(self.evidence_dir, out / "evidence", dirs_exist_ok=True)
            shutil.copy2(self.fixture.ledger_path, out / "owned-resources-ledger.jsonl")
            print(f"evidence copied to {out}")
        print(f"results: {self.run_dir / 'results.json'}")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--bin", required=True, help="herdr-threads executable under test")
    parser.add_argument("--evidence-out", help="directory to copy results, report, command log and evidence into")
    parser.add_argument("--deadline-seconds", type=int, default=20, help="R06 receipt deadline (default 20)")
    parser.add_argument("--only", help="comma-separated scenario ids (R01 and R12 always run)")
    parser.add_argument("--run-root", help="directory under /private/tmp for the private run directory (default /private/tmp)")
    args = parser.parse_args(argv)
    if platform.system() != "Darwin":
        print("UNSUPPORTED: the verified host incarnation witness exists only on macOS", file=sys.stderr)
        return 3
    suite = Suite(args)
    print(f"run dir {suite.run_dir}", flush=True)
    return 0 if suite.run() else 1


if __name__ == "__main__":
    sys.exit(main())
