#!/usr/bin/env python3
"""Observe exact root targeting from /side and unloaded embedded-session resume."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import pty
import signal
import struct
import termios
import time
import uuid

from spike import Spike


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--auth-file", type=Path, required=True)
    ap.add_argument("--output", type=Path, required=True)
    args = ap.parse_args()
    run = Spike(args)
    error = None
    try:
        run.start()
        root_session = run.session
        run.submit("/side Reply exactly SIDE_READY without tools.")
        run.wait(lambda: any(r["event"] == "SessionStart" and r["session_id"] != root_session for r in run.events()), label="side session registration")
        side_session = next(r["session_id"] for r in run.events() if r["event"] == "SessionStart" and r["session_id"] != root_session)
        run.wait(lambda: any(r["event"] == "Stop" and r["session_id"] == side_session for r in run.events()), label="side turn completion")
        before = len([r for r in run.events() if r["event"] == "UserPromptSubmit"])
        token = "ROOT_MAIL_" + uuid.uuid4().hex[:12]
        (run.root / "pending.json").write_text(json.dumps([token]))
        result = run.queue("herdr-threads: wake up")
        run.wait(lambda: any(token in r.get("injected", []) for r in run.events()), label="root queued wake with side displayed")
        run.wait(lambda: run.model_said(token), label="root model receives hook token")
        prompts = [r for r in run.events() if r["event"] == "UserPromptSubmit"][before:]
        run.record("root_queue_target_with_side_displayed", result["returncode"] == 0 and len(prompts) == 1 and prompts[0]["session_id"] == root_session, root_session=root_session, side_session=side_session, queue=result, new_prompts=prompts, model_token_seen=True)
        # Wait for the turn to finish before ending only our private foreground TUI.
        turn_id = prompts[0]["turn_id"]
        run.wait(lambda: any(r["event"] == "Stop" and r.get("turn_id") == turn_id for r in run.events()), label="root wake Stop")
        os.killpg(run.tui.pid, signal.SIGTERM)
        run.tui.wait(timeout=8)
        os.close(run.master)
        run.master = None
        token = "RESUME_MAIL_" + uuid.uuid4().hex[:12]
        (run.root / "pending.json").write_text(json.dumps([token]))
        before = run.counts()
        result = run.queue("herdr-threads: wake up")
        run.pump(15)
        held = bool(run.queue_rows()) and run.counts() == before
        run.record("unloaded_embedded_target_does_not_wake", result["returncode"] == 0 and held, queue=result, pending=run.queue_rows(), counts_before=before, counts_after=run.counts())
        run.master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 45, 160, 0, 0))
        run.tui = run.spawn(["codex", "--no-daemon", "--no-alt-screen", "--dangerously-bypass-hook-trust", "-s", "workspace-write", "-a", "never", "resume", root_session], stdin=slave, stdout=slave, stderr=slave)
        os.close(slave)
        run.wait(lambda: run.model_said(token), label="resumed embedded TUI consumes queued marker")
        run.record("explicit_resume_consumes_queued_marker", not run.queue_rows(), queued_session=root_session, counts_after=run.counts(), pending=run.queue_rows(), model_token_seen=True)
    except Exception as exc:
        error = exc
        run.log(type(exc).__name__ + ": " + str(exc))
    finally:
        run.cleanup()
        run.save(error)
    return int(error is not None or any(not r["passed"] for r in run.results))


if __name__ == "__main__":
    raise SystemExit(main())
