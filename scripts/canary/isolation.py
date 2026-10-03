#!/usr/bin/env python3
"""t0.isolation tripwire (nested spec §D2): the real ~/.claude/settings.json and ~/.codex/hooks.json
must not change while a probe runs.

    isolation.py snapshot OUT.json   record each watched file's mtime (null when absent)
    isolation.py check IN.json       compare against the snapshot; exit 0 pass, 2 infra (detail on stdout)

A file absent before and after passes; a file that appears, disappears or changes mtime during the probe
is an infra error (never a version break). Pure stdlib."""
import json, os, sys

WATCHED = (".claude/settings.json", ".codex/hooks.json")


def watched_paths(home=None):
    home = home or os.path.expanduser("~")
    return [os.path.join(home, rel) for rel in WATCHED]


def snapshot(paths):
    out = {}
    for p in paths:
        try:
            out[p] = os.stat(p).st_mtime_ns
        except FileNotFoundError:
            out[p] = None
    return out


def compare(before, after):
    """(ok, detail): ok is False when any watched file was created, removed or modified."""
    problems = []
    for p in sorted(set(before) | set(after)):
        b, a = before.get(p), after.get(p)
        if b == a:
            continue
        if b is None:
            problems.append(f"{p} was created during the probe")
        elif a is None:
            problems.append(f"{p} was removed during the probe")
        else:
            problems.append(f"{p} mtime changed during the probe")
    return (not problems, "; ".join(problems) or "real harness config untouched")


def main(argv):
    if len(argv) != 3 or argv[1] not in ("snapshot", "check"):
        print(__doc__, file=sys.stderr)
        return 64
    path = argv[2]
    if argv[1] == "snapshot":
        with open(path, "w", encoding="utf-8") as f:
            json.dump(snapshot(watched_paths()), f)
        return 0
    with open(path, encoding="utf-8") as f:
        before = json.load(f)
    ok, detail = compare(before, snapshot(before))
    print(detail)
    return 0 if ok else 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
