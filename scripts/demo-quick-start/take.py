#!/usr/bin/env python3
"""Drive one take: type the Quick start prompts into the rig's main tab through the recording client.

  take.py ROOT     (run inside the rig env; cam.py must be attached and idle, Claude up in w1:p1)
"""
import json
import subprocess
import sys
import time
from pathlib import Path

root = Path(sys.argv[1])
control = root / "control.jsonl"
MAIN, SHELL = "w1:p1", "w1:p2"
P1 = "/herdr-threads create a new weather channel"
P2 = "handoff to 5 new tabs, one per city, to find the weather in Tokyo, Paris, Cairo, Sydney and Rio"
P3 = "invite every weather tab to the channel and ask each to post a one-line update there every 5s for a minute"
P4 = "herdr-threads follow weather"
DELAY = 0.045


def ctl(**cmd):
    with control.open("a") as f:
        f.write(json.dumps(cmd) + "\n")


def herdr(*argv):
    out = subprocess.run(["herdr", *argv], capture_output=True, text=True, timeout=30)
    return json.loads(out.stdout) if out.stdout.strip().startswith("{") else {}


def tabs():
    return {t["label"]: t["agent_status"] for t in herdr("tab", "list")["result"]["tabs"]}


def typed(text, mark):
    ctl(mark=f"type:{mark}")
    ctl(type=text, delay=DELAY)
    time.sleep(len(text) * DELAY * 1.1 + 0.6)
    ctl(keys="\r")
    ctl(mark=f"sent:{mark}")


def wait_main(mark, timeout=240, extra=lambda: True):
    start = time.time()
    seen_working = False
    while time.time() - start < timeout:
        status = tabs().get("main")
        if status == "working":
            seen_working = True
        elif (seen_working or time.time() - start > 15) and extra():
            break
        time.sleep(0.5)
    else:
        print(f"timeout waiting for {mark}", flush=True)
    time.sleep(1.5)
    ctl(mark=f"done:{mark}")
    print(mark, round(time.time() - start, 1), tabs(), flush=True)


def cities_done():
    t = tabs()
    others = [s for label, s in t.items() if label != "main"]
    return len(others) >= 5 and all(s != "working" for s in others)


herdr("pane", "focus", "--pane", SHELL, "--direction", "up")
ctl(start=True)
time.sleep(1.5)
typed(P1, "p1")
wait_main("p1")
typed(P2, "p2")
wait_main("p2", extra=cities_done)
time.sleep(2)
typed(P3, "p3")
wait_main("p3")
time.sleep(15)
ctl(mark="gap:p3")
herdr("pane", "focus", "--pane", MAIN, "--direction", "down")
time.sleep(0.7)
herdr("pane", "resize", "--pane", SHELL, "--direction", "up", "--amount", "0.36")
time.sleep(0.9)
typed(P4, "p4")
time.sleep(45)
ctl(mark="end")
ctl(stop=True)
print("take done", flush=True)
