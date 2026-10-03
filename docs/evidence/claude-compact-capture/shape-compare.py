#!/usr/bin/env python3
# usage: shape-compare.py <new payload dir> <2.1.286 fixture dir>
# Flattens each payload to a path -> JSON type map and compares key paths and types per kind.
import json, sys
new, old = sys.argv[1], sys.argv[2]
pairs = [("01-sessionstart-startup", "01-sessionstart-startup"), ("02-pretooluse-bash-root", "02-pretooluse-bash-root"),
         ("03-sessionstart-resume", "03-sessionstart-resume"), ("04-pretooluse-bash-subagent", "04-pretooluse-bash-subagent")]
def flat(o, p=""):
    if isinstance(o, dict):
        out = {}
        for k, v in o.items():
            out.update(flat(v, f"{p}.{k}" if p else k))
        return out
    return {p: type(o).__name__}
ok = True
for n, o in pairs:
    a = flat(json.load(open(f"{new}/{n}.json")))
    b = flat(json.load(open(f"{old}/{o}.json")))
    diff = sorted(set(a.items()) ^ set(b.items()))
    print(f"{n}: {len(a)} paths vs {len(b)} paths; symmetric difference: {diff if diff else 'empty'}")
    ok = ok and not diff
print("RESULT:", "identical key paths and types" if ok else "DIFFERENT")
sys.exit(0 if ok else 1)
