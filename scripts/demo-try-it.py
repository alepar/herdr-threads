#!/usr/bin/env python3
"""Type a command plan into a real shell and record its terminal as asciicast v2.

Run INSIDE the owned human demo pane, after private setup (docs/media/try-it.md).
No transcript output is synthesized. Input events document the per-character pacing.
The optional final follow is interrupted only after an operator's finish file, or
fails at the time limit. Failed commands retain their cast and stop the plan.
"""
import argparse
import codecs
import errno
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import signal
import struct
import sys
import tempfile
import termios
import time


def capture(args):
    commands = json.loads(args.commands.read_text())
    if not isinstance(commands, list) or not commands or any(
        not isinstance(command, str) or not command.strip() for command in commands
    ):
        raise ValueError("commands must be a nonempty JSON array of shell commands")
    if args.delay <= 0 or args.pause < 0 or args.timeout <= 0:
        raise ValueError("delay and timeout must be positive; pause must be nonnegative")
    if args.finish_file is not None and args.finish_file.exists():
        raise ValueError("finish marker already exists; use a fresh attempt")
    with tempfile.TemporaryDirectory(prefix="ht-typing-") as private:
        status = Path(private) / "status"
        rcfile = Path(private) / "bashrc"
        rcfile.write_text(
            "PS1='\\[\\e[1;36m\\]you $ \\[\\e[0m\\]'\nPS2='> '\n"
            "HISTFILE=/dev/null\nset +m\nHT_PROMPT_SEQ=0\n"
            "PROMPT_COMMAND='HT_LAST_RC=$?; set +m; HT_PROMPT_SEQ=$((HT_PROMPT_SEQ+1)); "
            "printf \"%s %s\\n\" \"$HT_PROMPT_SEQ\" \"$HT_LAST_RC\" > \"$HT_CAPTURE_STATUS.tmp\"; "
            "mv \"$HT_CAPTURE_STATUS.tmp\" \"$HT_CAPTURE_STATUS\"'\n"
        )
        # Open exclusively: retries must preserve their earlier attempt.
        with args.output.open("x") as cast:
            cast.write(json.dumps({"version": 2, "width": 104, "height": 34,
                                   "timestamp": int(time.time()),
                                   "title": "Herdr Threads: Try it yourself"}) + "\n")
            started = time.monotonic()
            decoder = codecs.getincrementaldecoder("utf-8")("replace")
            pid, master = pty.fork()
            if pid == 0:
                os.chdir(args.cwd)
                os.environ.update(HT_CAPTURE_STATUS=str(status), TERM="xterm-256color")
                for key in ("HERDR_AGENT", "CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT", "CODEX_THREAD_ID"):
                    os.environ.pop(key, None)
                os.execv("/bin/bash", ["bash", "--noprofile", "--rcfile", str(rcfile), "-i"])
            fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 34, 104, 0, 0))

            def event(kind, text):
                cast.write(json.dumps([round(time.monotonic() - started, 6), kind, text]) + "\n")
                cast.flush()

            def pump(duration):
                until = time.monotonic() + duration
                while time.monotonic() < until:
                    if select.select([master], [], [], max(0, until - time.monotonic()))[0]:
                        try:
                            data = os.read(master, 65536)
                        except OSError as error:
                            if error.errno == errno.EIO:
                                raise RuntimeError("recorded shell exited") from error
                            raise
                        if not data:
                            raise RuntimeError("recorded shell exited")
                        text = decoder.decode(data)
                        if text:
                            event("o", text)
                            sys.stdout.write(text)
                            sys.stdout.flush()

            def prompt(sequence):
                until = time.monotonic() + args.timeout
                while time.monotonic() < until:
                    pump(0.05)
                    if status.exists():
                        values = status.read_text().split()
                        if len(values) == 2 and int(values[0]) >= sequence:
                            return int(values[1])
                raise TimeoutError("command did not return to the prompt")

            def type_text(text):
                for char in text:
                    os.write(master, char.encode())
                    event("i", char)
                    pump(args.delay * (1.3 if char == " " else 1))
                # Multiline comments/commands may already have advanced the
                # counter. Freeze the boundary immediately before final Enter.
                sequence = int(status.read_text().split()[0])
                os.write(master, b"\n")
                event("i", "\n")
                return sequence + 1

            try:
                prompt(1)
                for index, command in enumerate(commands):
                    pump(args.pause)
                    sequence = type_text(command)
                    following = args.finish_file is not None and index == len(commands) - 1
                    if following:
                        until = time.monotonic() + args.timeout
                        while not args.finish_file.exists():
                            if int(status.read_text().split()[0]) >= sequence:
                                raise RuntimeError("follow returned before operator validation")
                            if time.monotonic() >= until:
                                raise TimeoutError("discussion deadline reached; capture is unsuccessful")
                            pump(0.2)
                        pump(8)
                        os.write(master, b"\x03")
                        event("i", "\x03")
                        result = prompt(sequence)
                        # follow handles an intentional Ctrl-C gracefully (0);
                        # ordinary Unix foreground tools commonly return 130.
                        if result not in (0, 130):
                            raise RuntimeError(f"follow exited unexpectedly (exit {result})")
                    else:
                        result = prompt(sequence)
                        if result:
                            raise RuntimeError(f"command failed (exit {result}); capture is unsuccessful")
                pump(3)
            finally:
                # Only the shell session we just created, including a live follow.
                try:
                    os.killpg(pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
                # On macOS an exiting PTY child can wait for terminal teardown;
                # close the master before the final wait, never after it.
                os.close(master)
                deadline = time.monotonic() + 2
                while time.monotonic() < deadline:
                    if os.waitpid(pid, os.WNOHANG)[0]:
                        break
                    time.sleep(0.05)
                else:
                    os.killpg(pid, signal.SIGKILL)
                    os.waitpid(pid, 0)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--commands", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--cwd", type=Path, default=Path.cwd())
    parser.add_argument("--delay", type=float, default=0.09, help="seconds per typed character")
    parser.add_argument("--pause", type=float, default=2, help="reading pause before each command")
    parser.add_argument("--timeout", type=float, default=600, help="maximum command/follow wait")
    parser.add_argument("--finish-file", type=Path, help="operator creates this after reviewing the live conclusion")
    args = parser.parse_args()
    def interrupted(signum, _frame):
        raise RuntimeError(f"capture interrupted by signal {signum}")
    for signum in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT):
        signal.signal(signum, interrupted)
    try:
        capture(args)
    except (OSError, ValueError, RuntimeError, TimeoutError) as error:
        print(f"demo-try-it: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
