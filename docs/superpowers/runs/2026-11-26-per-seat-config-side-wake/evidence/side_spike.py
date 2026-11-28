#!/usr/bin/env python3
"""Throwaway ht-z41e.6 probe: Codex root vs /side identity (S1), concurrency (S2),
arrival (S3) and UI (S4). Reuses the 2026-11-14 queue-wake Spike harness (PTY-owned
Codex, scratch CODEX_HOME, private auth copy). Hooks here log the FULL payload."""
import argparse, glob, json, os, shlex, signal, sys, time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "2026-11-14-codex-queue-wake" / "evidence"))
import spike as base  # noqa: E402


def hook():
    root = Path(os.environ["SPIKE_ROOT"])
    event = json.load(sys.stdin)
    event["_at"] = time.time()
    event["_env"] = {k: os.environ.get(k) for k in ("CODEX_SESSION_ID", "CODEX_THREAD_ID", "CODEX_HOME")}
    with (root / "hooks.jsonl").open("a") as f:
        f.write(json.dumps(event) + "\n")
    print("{}")


class Side(base.Spike):
    def __init__(self, args):
        super().__init__(args)
        if os.environ.get("SPIKE_LEAK_ID"):
            self.env["HT_LEAK_RUN_ID"] = os.environ["SPIKE_LEAK_ID"]
        cmd = shlex.join([sys.executable, str(Path(__file__).resolve()), "--hook"])
        names = ("SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse", "Stop", "Interrupt", "SessionEnd")
        hooks = {n: [{"matcher": "*", "hooks": [{"type": "command", "command": cmd, "timeout": 10}]}] for n in names}
        (self.chome / "hooks.json").write_text(json.dumps({"hooks": hooks}))

    def events(self):
        rows = super().events()
        for r in rows:
            r["event"] = r.get("hook_event_name")
            r["session_id"] = r.get("session_id")
        return rows

    def start(self):  # lone root, full payload capture
        super().start()

    def rollouts(self):
        out = {}
        for p in sorted(glob.glob(str(self.chome / "sessions" / "**" / "*.jsonl"), recursive=True)):
            lines = Path(p).read_text(errors="replace").splitlines()
            meta = None
            for l in lines[:3]:
                try:
                    j = json.loads(l)
                except ValueError:
                    continue
                if j.get("type") == "session_meta":
                    pl = dict(j["payload"])
                    for k in ("instructions", "base_instructions", "dynamic_tools", "creator_user_id", "creator_account_id"):
                        pl.pop(k, None)
                    meta = pl
                    break
            out[os.path.basename(p)] = {"lines": len(lines), "session_meta": meta}
        return out

    def screen_tail(self, n=1800):
        t = self.raw[-n * 4:].decode(errors="replace")
        import re
        t = re.sub(r"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)", "", t)
        t = re.sub(r"\x1b\[\d*(?:;\d+)?[Hf]|\x1b\[\d*[EB]|\x1bM|\x1b\[\d*K", "\n", t)
        t = re.sub(r"\x1b\[[0-?]*[ -/]*[@-~]", "", t)
        return t[-n:]

    def root_prompts(self, root, after=0):
        return [r for r in self.events() if r["event"] == "UserPromptSubmit" and r["session_id"] == root and r["_at"] > after]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--auth-file", type=Path)
    ap.add_argument("--output", type=Path)
    ap.add_argument("--hook", action="store_true")
    a = ap.parse_args()
    if a.hook:
        hook()
        return 0
    run = Side(a)
    res = {}
    err = None
    try:
        run.start()
        root = run.session
        ev = run.events()
        res["lone_root"] = {"session_start": [r for r in ev if r["event"] == "SessionStart"], "rollouts": run.rollouts()}
        # open /side, register it
        run.submit("/side Reply exactly SIDE_READY without tools.")
        run.wait(lambda: any(r["event"] == "SessionStart" and r["session_id"] != root for r in run.events()), label="side start")
        side = next(r["session_id"] for r in run.events() if r["event"] == "SessionStart" and r["session_id"] != root)
        try:
            run.wait(lambda: any(r["event"] == "Stop" and r["session_id"] == side for r in run.events()), label="side stop", timeout=120)
        except TimeoutError:
            res["side_stop_missing"] = run.screen_tail(4000)
        res["root"], res["side"] = root, side
        res["after_side_open"] = {"rollouts": run.rollouts()}
        # S2a: side turn busy, queue root wake
        for n in ("entered", "release"):
            (run.cwd / n).unlink(missing_ok=True)
        (run.cwd / "barrier.py").write_text("import pathlib,time\np=pathlib.Path('.')\n(p/'entered').write_text('ready')\ns=time.monotonic()\nwhile not (p/'release').exists() and time.monotonic()-s<90: time.sleep(.1)\nprint('BARRIER_RELEASED')\n")
        run.submit("BUSY: execute exactly `python3 barrier.py` once with exec_command, yield_time_ms=1000, wait for completion with write_stdin as needed. No other command. Then reply BUSY_DONE.")
        run.wait(lambda: (run.cwd / "entered").exists(), label="side busy barrier")
        t0 = time.time()
        q = run.queue("herdr-threads: wake up")
        run.pump(25)
        started = run.root_prompts(root, t0)
        res["S2a_busy_side"] = {"queue": q, "root_prompts_while_side_busy": started, "side_stops_while_busy": [r for r in run.events() if r["event"] == "Stop" and r["session_id"] == side and r["_at"] > t0], "pending_rows": run.queue_rows()}
        (run.cwd / "release").write_text("go")
        run.wait(lambda: any(r["event"] == "Stop" and r["session_id"] == side and r["_at"] > t0 for r in run.events()), label="side busy stop", timeout=120)
        run.pump(20)
        res["S2a_after_release"] = {"root_prompts": run.root_prompts(root, t0), "pending_rows": run.queue_rows()}
        run.wait(lambda: not run.queue_rows(), timeout=60, label="queue drained")
        run.pump(8)
        # S2b/S3/S4: side displayed idle, typed draft, queue root wake
        before_roll = run.rollouts()
        os.write(run.master, b"SIDE_DRAFT_7731")
        run.pump(2)
        screen_before = run.screen_tail()
        t1 = time.time()
        n_ev = len(run.events())
        q2 = run.queue("herdr-threads: wake up")
        run.wait(lambda: run.root_prompts(root, t1), label="root wake prompt", timeout=60)
        rp = run.root_prompts(root, t1)
        tid = rp[0].get("turn_id")
        run.wait(lambda: any(r["event"] == "Stop" and r.get("turn_id") == tid for r in run.events()), label="root wake stop", timeout=90)
        run.pump(3)
        screen_after = run.screen_tail()
        new = run.events()[n_ev:]
        res["S2b_S3_S4"] = {
            "queue": q2,
            "events_since_queue": [{k: r.get(k) for k in ("event", "session_id", "turn_id", "prompt", "transcript_path", "_at")} for r in new],
            "side_events_since_queue": [r["event"] for r in new if r["session_id"] == side],
            "root_prompt": rp[0].get("prompt"),
            "rollouts_before": before_roll, "rollouts_after": run.rollouts(),
            "screen_before_tail": screen_before, "screen_after_tail": screen_after,
            "draft_in_screen_before": "SIDE_DRAFT_7731" in screen_before, "draft_in_screen_after": "SIDE_DRAFT_7731" in screen_after,
        }
        # submit the draft: must go to the side session
        t2 = time.time()
        os.write(run.master, b"\r")
        run.wait(lambda: any(r["event"] == "UserPromptSubmit" and r["_at"] > t2 for r in run.events()), label="draft submit", timeout=60)
        dp = [r for r in run.events() if r["event"] == "UserPromptSubmit" and r["_at"] > t2][0]
        res["S4_draft_submit"] = {"session_id": dp["session_id"], "is_side": dp["session_id"] == side, "prompt": dp.get("prompt")}
        run.wait(lambda: any(r["event"] == "Stop" and r["session_id"] == side and r["_at"] > t2 for r in run.events()), label="draft stop", timeout=90)
        # S1 resume stability
        os.killpg(run.tui.pid, signal.SIGTERM)
        run.tui.wait(timeout=8)
        os.close(run.master)
        run.master = None
        import fcntl, pty, struct, termios
        run.master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 45, 160, 0, 0))
        n2 = len(run.events())
        run.tui = run.spawn(["codex", "--no-daemon", "--no-alt-screen", "--dangerously-bypass-hook-trust", "-s", "workspace-write", "-a", "never", "resume", root], stdin=slave, stdout=slave, stderr=slave)
        os.close(slave)
        run.pump(10)
        res["S1_resume_early_events"] = [r["event"] for r in run.events()[n2:]]
        if not any(r["event"] == "SessionStart" for r in run.events()[n2:]):
            run.submit("Reply exactly RESUMED without tools.")
        run.wait(lambda: any(r["event"] == "SessionStart" for r in run.events()[n2:]), label="resume start", timeout=60)
        run.pump(3)
        res["S1_resume"] = {"events": run.events()[n2:], "rollouts": run.rollouts()}
        res["S1_all_events"] = run.events()
        res["terminal_tail"] = run.screen_tail(6000)
    except Exception as exc:
        err = exc
        run.log(type(exc).__name__ + ": " + str(exc))
        res["error"] = run.redact(str(exc))
        try:
            res["partial_events"] = run.events()
            res["screen_tail"] = run.screen_tail(3000)
        except Exception:
            pass
    finally:
        run.cleanup()
        a.output.mkdir(parents=True, exist_ok=True)
        res["version"] = os.popen("codex --version").read().strip()
        res["private_auth_refreshed"] = base.hashlib.sha256((run.chome / "auth.json").read_bytes()).hexdigest() != run.copy_digest
        res["source_auth_unchanged"] = base.hashlib.sha256(a.auth_file.read_bytes()).hexdigest() == run.source_digest
        res["cleanup_tagged_descendants"] = run.cleanup_pids
        (a.output / "side-results.json").write_text(run.redact(json.dumps(res, indent=2, default=str)) + "\n")
        base.shutil.rmtree(run.root, ignore_errors=True)
    return int(err is not None)


if __name__ == "__main__":
    sys.exit(main())
