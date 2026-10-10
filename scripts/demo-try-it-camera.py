#!/usr/bin/env python3
"""Record an actual attached Herdr client; setup/navigation remain private.

Controller appends JSON strings containing literal terminal keys to control.jsonl.
After selecting the owned tab, create camera-start; create camera-stop to finish.
Private HOME/config/state and the existing socket must be supplied by the caller.
Only this process's client session is stopped; the shared server is never stopped.
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
import termios
import time
import sys

SCRATCH_ROOT = "/private/tmp" if sys.platform == "darwin" else "/tmp"


def capture(args):
    root = args.root.resolve()
    if root.parent != Path(SCRATCH_ROOT) or not root.name.startswith("ht-try-it."):
        raise ValueError("camera requires an owned private run directory")
    for name in ("camera-start", "camera-stop", "camera.cast", "camera-private.ansi", "control.jsonl"):
        if (root / name).exists():
            raise ValueError(f"fresh camera attempt required: {name} exists")
    control = root / "control.jsonl"
    control.touch(mode=0o600)
    started = None
    deadline = time.monotonic() + args.timeout
    decoder = codecs.getincrementaldecoder("utf-8")("replace")
    pid, master = pty.fork()
    if pid == 0:
        fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", args.rows, args.cols, 0, 0))
        os.chdir(root)
        os.execvp("herdr", ["herdr", "client"])
    try:
        (root / "camera.pid").write_text(str(pid) + "\n")
        def resize(cols):
            fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", args.rows, cols, 0, 0))
        resize(args.cols)
        with control.open() as keys, (root / "camera-private.ansi").open("xb") as private:
            def drain_private(duration):
                until = time.monotonic() + duration
                while time.monotonic() < until:
                    if select.select([master], [], [], 0.02)[0]:
                        data = os.read(master, 65536)
                        if not data:
                            raise RuntimeError("camera client exited during setup")
                        private.write(data)
                        decoder.decode(data)
                private.flush()
            cast = None
            try:
                while time.monotonic() < deadline:
                    if started is None and (root / "camera-start").exists():
                        drain_private(0.25)
                        resize(args.cols + 1)
                        drain_private(0.25)
                        cast = (root / "camera.cast").open("x")
                        started = time.monotonic()
                        cast.write(json.dumps({"version": 2, "width": args.cols, "height": args.rows,
                                               "title": "Alice, Bob and the human: a live Herdr walkthrough"}) + "\n")
                        # Actual client redraw after private navigation. No invented frame.
                        resize(args.cols)
                    line = keys.readline()
                    if line:
                        command = json.loads(line)
                        if not isinstance(command, str):
                            raise ValueError("control entries must be literal key strings")
                        os.write(master, command.encode())
                    if select.select([master], [], [], 0.02)[0]:
                        try:
                            data = os.read(master, 65536)
                        except OSError as error:
                            if error.errno == errno.EIO:
                                raise RuntimeError("camera client exited before stop") from error
                            raise
                        if not data:
                            raise RuntimeError("camera client exited before stop")
                        private.write(data)
                        private.flush()
                        text = decoder.decode(data)
                        if cast is not None and text:
                            cast.write(json.dumps([round(time.monotonic() - started, 6), "o", text]) + "\n")
                            cast.flush()
                    if (root / "camera-stop").exists():
                        if started is None:
                            raise RuntimeError("camera stopped before recording started")
                        return
                raise TimeoutError("camera deadline reached")
            finally:
                if cast is not None:
                    cast.close()
    finally:
        try:
            os.killpg(pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        os.close(master)
        until = time.monotonic() + 2
        while time.monotonic() < until:
            if os.waitpid(pid, os.WNOHANG)[0]:
                break
            time.sleep(0.05)
        else:
            os.killpg(pid, signal.SIGKILL)
            os.waitpid(pid, 0)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--cols", type=int, default=160)
    parser.add_argument("--rows", type=int, default=62)
    parser.add_argument("--timeout", type=float, default=1200)
    args = parser.parse_args()
    if not 1 <= args.cols < 65535 or not 1 <= args.rows <= 65535 or args.timeout <= 0:
        parser.error("positive terminal dimensions and timeout required")
    def interrupted(signum, _frame):
        raise RuntimeError(f"camera interrupted by signal {signum}")
    for signum in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
        signal.signal(signum, interrupted)
    capture(args)


if __name__ == "__main__":
    main()
