#!/usr/bin/env python3
# usage: SCRATCH=<scratch dir> extract.py <evidence dir>
# Produces the redacted payloads/ and run*.json from the raw hook logs and Claude's own transcript.
import json, os, sys, glob
S = os.environ["SCRATCH"]
HOME = os.path.expanduser("~")
OUT = sys.argv[1]
os.makedirs(OUT + "/payloads", exist_ok=True)
def red(s):
    return s.replace(S, "<scratch>").replace(HOME, "~").replace(os.environ["USER"], "<user>")
def dump(obj, path):
    open(path, "w").write(red(json.dumps(obj, indent=1)) + "\n")
recs = [json.loads(l) for l in open(S + "/raw/hooks.jsonl")]
names = ["01-sessionstart-startup", "02-pretooluse-bash-root", "03-sessionstart-resume", "04-pretooluse-bash-subagent"]
assert len(recs) == 4
for n, r in zip(names, recs):
    open(f"{OUT}/payloads/{n}.json", "w").write(red(json.dumps(r["payload"], sort_keys=True, separators=(",", ":"))))
    open(f"{OUT}/payloads/{n}.env-names.txt", "w").write("\n".join(r["env_names"]) + "\n")
outs = [json.loads(l) for l in open(S + "/raw/hooks-out.jsonl")]
compact = [r for r in outs if r["payload"].get("source") == "compact"]
assert len(compact) == 1
open(f"{OUT}/payloads/05-sessionstart-compact.json", "w").write(red(json.dumps(compact[0]["payload"], sort_keys=True, separators=(",", ":"))))
dump([{"argv_tag": r["argv_tag"], "source": r["payload"].get("source"), "returned": r["returned"]} for r in outs],
     f"{OUT}/payloads/hook-out-returned.json")
runs = ["run1", "run2", "run3", "run4"]
ev = {}
for t in runs:
    d = json.load(open(f"{S}/raw/{t}.json"))
    open(f"{OUT}/{t}.json", "w").write(red(json.dumps(d, separators=(",", ":"))))
    leak = open(f"{S}/raw/{t}.parent-leak.txt").read().split()
    ev[t] = {"session_id": d["session_id"], "result": d.get("result"), "total_cost_usd": d.get("total_cost_usd"),
             "permission_denials": d.get("permission_denials"), "parent_claude_herdr_env_after_unset": leak}
sid = {json.load(open(f"{S}/raw/{t}.json"))["session_id"] for t in runs}
assert len(sid) == 1
tp = glob.glob(f"{HOME}/.claude/projects/*ht-claude287-compact-scratch-proj/{sid.copy().pop()}.jsonl")[0]
rows = []
for i, line in enumerate(open(tp), 1):
    j = json.loads(line)
    if j.get("type") == "system" and j.get("subtype") == "compact_boundary":
        rows.append({"line": i, "compact_boundary": True})
    if j.get("isCompactSummary"):
        rows.append({"line": i, "compact_summary_message": True})
    a = j.get("attachment")
    if a and a.get("type") in ("hook_success", "hook_additional_context"):
        rows.append({"line": i, "attachment": {k: a.get(k) for k in ("type", "hookName", "hookEvent", "content", "stdout", "exitCode") if k in a}})
ev["transcript_rows"] = rows
dump(ev, f"{OUT}/payloads/transcript-evidence.json")
