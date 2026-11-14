#!/usr/bin/env python3
"""warp.py IN.cast OUT.cast : compress the waits between markers; typing stays at recorded speed.

Segments are delimited by the take's marker events. A typing segment (type:X -> sent:X) plays at 1x.
Every other segment is sped up so it lasts at most its budget (seconds), never slowed down.
"""
import json
import sys

BUDGET = {"sent:p1": 3.5, "sent:p2": 11.0, "sent:p3": 5.0, "sent:p4": 16.0}
DEFAULT = 1.2
TAIL_HOLD = 2.5

events = [json.loads(line) for line in open(sys.argv[1])]
header, events = events[0], events[1:]
marks = [(e[0], e[2]) for e in events if e[1] == "m"]
bounds = [(0.0, "start")] + marks
segments = []  # (t0, t1, speed)
for (t0, name), (t1, _) in zip(bounds, bounds[1:] + [(events[-1][0], "eof")]):
    dur = t1 - t0
    if name.startswith("type:"):
        speed = 1.0
    else:
        budget = BUDGET.get(name, DEFAULT)
        speed = max(1.0, dur / budget) if dur > 0 else 1.0
    segments.append((t0, t1, speed))


def remap(t):
    out = 0.0
    for t0, t1, speed in segments:
        if t >= t1:
            out += (t1 - t0) / speed
        elif t > t0:
            out += (t - t0) / speed
    return out


with open(sys.argv[2], "w") as f:
    f.write(json.dumps(header) + "\n")
    last = 0.0
    for e in events:
        if e[1] != "o":
            continue
        last = round(remap(e[0]), 4)
        f.write(json.dumps([last, "o", e[2]]) + "\n")
    f.write(json.dumps([round(last + TAIL_HOLD, 4), "o", ""]) + "\n")
print("duration", round(last + TAIL_HOLD, 1), [(n, round(s, 1)) for (_, n), (_, _, s) in zip(bounds, segments)])
