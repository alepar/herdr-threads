#!/usr/bin/env python3
"""Record an attached client of the rig's private Herdr server as asciicast v2.

  cam.py ROOT --cols N --rows N      (run inside the rig env: rig.py env ROOT -- python3 cam.py ROOT ...; needs nothing but the stdlib)

Control: append JSON lines to ROOT/control.jsonl
  {"keys": "..."}                 raw bytes to the client
  {"type": "...", "delay": 0.05}  typed one character at a time
  {"mark": "name"}                marker event in the cast
  {"start": true}                 start recording (after a forced redraw)
  {"stop": true}                  finish
"""
import argparse
import codecs
import fcntl
import json
import os
import pty
import select
import signal
import struct
import termios
import time
from pathlib import Path


def main():
    p = argparse.ArgumentParser()
    p.add_argument("root", type=Path)
    p.add_argument("--cols", type=int, default=100)
    p.add_argument("--rows", type=int, default=30)
    p.add_argument("--timeout", type=float, default=1500)
    a = p.parse_args()
    control = a.root / "control.jsonl"
    control.touch()
    pid, master = pty.fork()
    if pid == 0:
        fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", a.rows, a.cols, 0, 0))
        os.execvp("herdr", ["herdr"])
    size = lambda cols: fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", a.rows, cols, 0, 0))
    size(a.cols)
    decoder = codecs.getincrementaldecoder("utf-8")("replace")
    cast = None
    started = None
    queue = []  # (due, bytes)
    deadline = time.monotonic() + a.timeout
    raw = (a.root / "camera-raw.ansi").open("wb")
    try:
        with control.open() as lines:
            while time.monotonic() < deadline:
                now = time.monotonic()
                if not queue:
                    line = lines.readline()
                    if line.endswith("\n"):
                        cmd = json.loads(line)
                        if "keys" in cmd:
                            queue.append((now, cmd["keys"].encode()))
                        elif "type" in cmd:
                            due = now
                            for ch in cmd["type"]:
                                due += cmd.get("delay", 0.05) * (1.3 if ch == " " else 1)
                                queue.append((due, ch.encode()))
                        elif "mark" in cmd and cast:
                            cast.write(json.dumps([round(now - started, 6), "m", cmd["mark"]]) + "\n")
                        elif cmd.get("start"):
                            size(a.cols + 1)
                            time.sleep(0.3)
                            while select.select([master], [], [], 0.05)[0]:
                                data = os.read(master, 65536)
                                raw.write(data)
                                decoder.decode(data)
                            cast = (a.root / "camera.cast").open("w")
                            started = time.monotonic()
                            cast.write(json.dumps({"version": 2, "width": a.cols, "height": a.rows}) + "\n")
                            size(a.cols)
                        elif cmd.get("stop"):
                            return
                while queue and queue[0][0] <= time.monotonic():
                    os.write(master, queue.pop(0)[1])
                if select.select([master], [], [], 0.01)[0]:
                    data = os.read(master, 65536)
                    if not data:
                        raise RuntimeError("client exited")
                    raw.write(data)
                    raw.flush()
                    text = decoder.decode(data)
                    if cast and text:
                        cast.write(json.dumps([round(time.monotonic() - started, 6), "o", text]) + "\n")
                        cast.flush()
    finally:
        if cast:
            cast.close()
        raw.close()
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        os.close(master)
        try:
            os.waitpid(pid, 0)
        except ChildProcessError:
            pass


if __name__ == "__main__":
    main()
