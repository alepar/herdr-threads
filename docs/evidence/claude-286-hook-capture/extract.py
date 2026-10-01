import json, os, sys, glob
S = "<scratch>"
HOME = os.path.expanduser("~")
OUT = sys.argv[1]
os.makedirs(OUT + "/payloads", exist_ok=True)
def red(s):
    return s.replace(S, "<scratch>").replace(HOME, "~").replace(os.environ["USER"], "<user>")
def dump(obj, path, compact=False):
    txt = json.dumps(obj, sort_keys=compact, separators=(",", ":")) if compact else json.dumps(obj, indent=1)
    open(path, "w").write(red(txt) + ("\n" if not compact else ""))
recs = [json.loads(l) for l in open(S + "/raw/hooks.jsonl")]
names = ["01-sessionstart-startup", "02-pretooluse-bash-root", "03-sessionstart-resume", "04-pretooluse-bash-subagent"]
assert len(recs) == 4
for n, r in zip(names, recs):
    open(f"{OUT}/payloads/{n}.json", "w").write(red(json.dumps(r["payload"], sort_keys=True, separators=(",", ":"))))
    open(f"{OUT}/payloads/{n}.env-names.txt", "w").write("\n".join(r["env_names"]) + "\n")
by_session = {}
for t in ["run1", "run2", "run3", "run3b", "run3c"]:
    d = json.load(open(f"{S}/raw/{t}.json"))
    open(f"{OUT}/{t}.json", "w").write(red(json.dumps(d, separators=(",", ":"))))
    by_session.setdefault(d["session_id"], []).append(t)
r3 = [json.loads(l) for l in open(S + "/raw/hooks-run3.jsonl")]
runs3 = {}
for t in ["run3", "run3b", "run3c"]:
    runs3[json.load(open(f"{S}/raw/{t}.json"))["session_id"]] = t
for r in r3:
    t = runs3[r["payload"]["session_id"]]
    kind = "sessionstart-startup" if r["argv_tag"] == "SessionStart" else "pretooluse-bash-root"
    dump({"input": r["payload"], "returned": r["returned"]}, f"{OUT}/payloads/{t}-{kind}.returned.json")
ev = {}
for t in ["run1", "run2", "run3", "run3b", "run3c"]:
    d = json.load(open(f"{S}/raw/{t}.json"))
    leak = open(f"{S}/raw/{t}.parent-leak.txt").read().split()
    e = {"session_id": d["session_id"], "result": d.get("result"), "total_cost_usd": d.get("total_cost_usd"),
         "permission_denials": d.get("permission_denials"), "parent_claude_herdr_env_after_unset": leak}
    if t.startswith("run3"):
        tp = glob.glob(f"{HOME}/.claude/projects/-private-tmp-ht-claude286-scratch-proj/{d['session_id']}.jsonl")[0]
        rows = []
        for i, line in enumerate(open(tp), 1):
            j = json.loads(line)
            a = j.get("attachment")
            if a and a.get("type") in ("hook_success", "hook_additional_context"):
                rows.append({"line": i, "attachment": {k: a.get(k) for k in ("type", "hookName", "hookEvent", "content", "stdout", "exitCode") if k in a}})
            m = j.get("message") or {}
            c = m.get("content")
            if isinstance(c, list):
                for b in c:
                    if b.get("type") == "tool_use":
                        rows.append({"line": i, "tool_use_input_from_model": b.get("input")})
                    elif b.get("type") == "tool_result":
                        cc = b.get("content")
                        if isinstance(cc, list):
                            cc = "".join(x.get("text", "") for x in cc)
                        rows.append({"line": i, "tool_result": cc, "is_error": b.get("is_error", False)})
                    elif b.get("type") == "text" and m.get("role") == "assistant":
                        rows.append({"line": i, "assistant_text": b.get("text")})
        e["transcript_rows"] = rows
    ev[t] = e
dump(ev, f"{OUT}/payloads/transcript-evidence.json")
