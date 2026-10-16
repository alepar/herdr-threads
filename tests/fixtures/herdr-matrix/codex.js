"""A stand-in Codex for the Herdr release matrix (no provider, model or credentials).

Started like npm's Codex, `<interpreter> .../codex.js` (the matrix's `codex` wrapper runs
`python3 -I .../codex.js`; the name is the shape both Herdr and herdr-threads recognize as Codex,
though the content is Python). It behaves like the parts of Codex that Herdr and herdr-threads observe:

- the screen ends in Codex's composer, `› Ask Codex to do anything` when empty and `› <text>` while
  typing; a running turn shows `• Working (… esc to interrupt)` above it;
- input is read raw: printable keys edit the composer, Enter submits, Ctrl+U clears it;
- the terminal title (OSC 0) is the working directory's name, as Codex sets it;
- `$CODEX_HOME/hooks.json` runs as Codex runs it (one JSON payload on stdin per command): SessionStart
  (`source: startup`) at start, UserPromptSubmit for each submitted line, PreToolUse before a tool,
  Stop at the end of every turn, Interrupt on Ctrl-C, SessionEnd at exit. That covers Herdr's own
  Codex integration too, whichever events a release's integration hooks. Each hook's `hookSpecificOutput.additionalContext` is the context the model
  would get.

A submitted line is a turn. `run: CMD` runs CMD as a Bash tool call (PreToolUse first, then CMD as
a child with CODEX_SESSION_ID set), and `run: A ;; B` runs two calls in the same turn; `exit` ends the session; anything else is just a prompt.
Every event is appended to `$FAKE_CODEX_LOG_DIR/<session>.jsonl` (default `.fake-codex` in the
working directory), so a test reads exactly what the agent saw.
"""
import json
import os
import select
import subprocess
import sys
import termios
import time
import tty
import uuid

HOME = os.environ.get("CODEX_HOME") or os.path.expanduser("~/.codex")
SESSION = str(uuid.uuid4())
LOG_DIR = os.environ.get("FAKE_CODEX_LOG_DIR") or os.path.join(os.getcwd(), ".fake-codex")
os.makedirs(LOG_DIR, exist_ok=True)
LOG = os.path.join(LOG_DIR, f"{SESSION}.jsonl")
transcript = [">_ OpenAI Codex (fake)", f"   session {SESSION}"]
buffer = ""
turn_started = None  # monotonic start of the running turn, for Codex's live working timer
queued = ""  # terminal input that arrived during a turn, processed after it (Codex queues it)
TTY = None


def log(kind, **fields):
    with open(LOG, "a") as f:
        f.write(json.dumps({"t": time.time(), "kind": kind, "pid": os.getpid(), "session": SESSION,
                            "pane": os.environ.get("HERDR_PANE_ID"), **fields}) + "\n")


def draw(working=False):
    rows = transcript[-30:] + [""]
    if working:
        # Codex's live timer ticks every second while a turn runs; Herdr's working rule needs it.
        elapsed = int(time.monotonic() - turn_started) if turn_started else 0
        rows += [f"• Working ({elapsed}s • esc to interrupt)", ""]
    rows.append("› " + (buffer if buffer else "Ask Codex to do anything"))
    # Codex's footer under the composer: Herdr's screen detection reads it.
    rows += ["", "  fake-model default · " + os.getcwd(), "  ? for shortcuts"]
    # Codex sets the terminal title (OSC 0) to its working directory's name.
    title = os.path.basename(os.getcwd()) or "codex"
    sys.stdout.write(f"\033]0;{title}\007\033[2J\033[H" + "\r\n".join(rows))
    sys.stdout.flush()


def hooks(event):
    try:
        with open(os.path.join(HOME, "hooks.json")) as f:
            return [h for group in json.load(f).get("hooks", {}).get(event, []) for h in group.get("hooks", [])]
    except (OSError, ValueError):
        return []


def run_hooks(event, **payload):
    contexts = []
    for hook in hooks(event):
        body = {"session_id": SESSION, "hook_event_name": event, "cwd": os.getcwd(),
                "transcript_path": None, "model": "fake", **payload}
        env = dict(os.environ, CODEX_SESSION_ID=SESSION)
        try:
            done = subprocess.run(hook["command"], shell=True, input=json.dumps(body), capture_output=True,
                                  text=True, timeout=hook.get("timeout", 10), env=env)
            out, rc, err = done.stdout, done.returncode, done.stderr
        except subprocess.TimeoutExpired:
            out, rc, err = "", "timeout", ""
        context = ""
        try:
            context = json.loads(out).get("hookSpecificOutput", {}).get("additionalContext", "") if out.strip() else ""
        except ValueError:
            context = out
        log("hook", event=event, rc=rc, context=context, stdout=out[-2000:], stderr=err[-2000:])
        if context:
            contexts.append(context)
            transcript.extend("  ctx " + line for line in context.splitlines()[:6])
    return contexts


def run_tool(command):
    """One Bash tool call: the screen keeps ticking and the terminal keeps being read while it runs.
    Input arriving now is logged (`input`, during_turn) and queued for after the turn."""
    global queued
    child = subprocess.Popen(command, shell=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                             env=dict(os.environ, CODEX_SESSION_ID=SESSION))
    deadline = time.monotonic() + 60
    while child.poll() is None and time.monotonic() < deadline:
        draw(working=True)
        ready, _, _ = select.select([TTY], [], [], 1.0)
        if ready:
            data = os.read(TTY, 1024).decode("utf-8", "replace")
            log("input", data=data, during_turn=True)
            queued += data
    if child.poll() is None:
        child.kill()
    stdout, stderr = child.communicate()
    return child.returncode, stdout, stderr


def turn(line):
    global turn_started
    turn_started = time.monotonic()
    transcript.append("› " + line)
    draw(working=True)
    log("prompt", text=line)
    turn_id = str(uuid.uuid4())  # one per turn, on every hook of the turn, as Codex sends it
    run_hooks("UserPromptSubmit", prompt=line, turn_id=turn_id)
    if line.startswith("run: "):
        # `run: A ;; B` is two Bash tool calls in one turn (PreToolUse before each).
        for command in line[5:].split(" ;; "):
            tool_id = "call_" + uuid.uuid4().hex[:12]
            run_hooks("PreToolUse", tool_name="Bash", tool_use_id=tool_id, tool_input={"command": command},
                      turn_id=turn_id)
            rc, stdout, stderr = run_tool(command)
            log("tool", command=command, rc=rc, stdout=stdout[-4000:], stderr=stderr[-2000:])
            transcript.append(f"• Ran {command[:60]} (rc={rc})")
            draw(working=True)
    transcript.append("• done")
    # Codex's end-of-turn summary: older Herdr releases read a finished turn from it.
    transcript.extend(["", "  Worked for 1s • " + time.strftime("%H:%M"), ""])
    run_hooks("Stop", stop_hook_active=False, last_assistant_message="done", turn_id=turn_id)
    turn_started = None
    log("turn_end", text=line)


def main():
    global buffer
    if len(sys.argv) > 1 and sys.argv[1] == "--version":
        print("codex-cli 0.0.0-fake")
        return
    log("start", argv=sys.argv[1:], cwd=os.getcwd())
    run_hooks("SessionStart", source="startup")
    global TTY, queued
    fd = TTY = sys.stdin.fileno()
    saved = termios.tcgetattr(fd)
    tty.setraw(fd)
    try:
        draw()
        while True:
            if queued:
                data, queued = queued, ""
            else:
                ready, _, _ = select.select([fd], [], [], 0.5)
                if not ready:
                    continue
                data = os.read(fd, 1024).decode("utf-8", "replace")
                if not data:
                    break
                log("input", data=data, during_turn=False)
            i = 0
            while i < len(data):
                ch = data[i]
                if ch == "\x1b":  # an escape sequence (arrows, bracketed paste markers): skipped
                    j = i + 1
                    if j < len(data) and data[j] == "[":
                        j += 1
                        while j < len(data) and not ("@" <= data[j] <= "~"):
                            j += 1
                    i = j + 1
                    continue
                if ch in "\r\n":
                    line, buffer = buffer, ""
                    if line == "exit":
                        run_hooks("SessionEnd", reason="exit")
                        log("exit")
                        return
                    if line:
                        turn(line)
                elif ch == "\x15":  # ctrl+u
                    buffer = ""
                elif ch == "\x03":  # ctrl+c
                    run_hooks("Interrupt")
                    log("interrupt")
                elif ch in "\x7f\b":
                    buffer = buffer[:-1]
                elif ch >= " ":
                    buffer += ch
                i += 1
            draw()
    finally:
        termios.tcsetattr(fd, termios.TCSADRAIN, saved)


if __name__ == "__main__":
    main()
