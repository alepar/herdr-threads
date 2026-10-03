"""canary-report.json assembly, summary.md rendering and exit code (nested spec §D8). Pure stdlib."""
import datetime, json, pathlib

PACKAGES = {"claude": "@anthropic-ai/claude-code", "codex": "@openai/codex"}
STATUSES = ("all_pass", "break", "inconclusive", "infra_error", "no_candidates", "known_broken_persists")


def _ver_args(v):
    return ", ".join(v.split("."))


def newest_failing(block):
    bad = [p["version"] for p in block.get("probes", []) if p["result"] == "fail"]
    if not bad:
        return None
    return max(bad, key=lambda v: tuple(int(x) for x in v.split(".")))


def suggested_action(block):
    """The §D8 suggested_action for a `break` block (None otherwise)."""
    if block.get("status") != "break" or not block.get("first_bad"):
        return None
    fb = block["first_bad"]
    hi = newest_failing(block) or fb
    snippet = ("known_broken: &[VersionSet::Interval { min: Some(Version::new(%s)), "
               "max: Some(Version::new(%s)) }]" % (_ver_args(fb), _ver_args(hi)))
    return {"kind": "adapter_or_known_broken", "range": f">= {fb}", "known_broken_snippet": snippet,
            "text": f"add a recipe for >= {fb} (adapter), or mark [{fb}, …) known_broken until one exists"}


def harness_block(harness, result, verified_max=None, package=None):
    """A §D8 harness block from a bisect.run result."""
    block = {"harness": harness, "package": package or PACKAGES.get(harness, harness),
             "verified_max": verified_max if verified_max is not None else result.get("baseline")}
    for k in ("candidates", "status", "first_bad", "last_good", "failing_checks", "signals"):
        block[k] = result[k]
    block["suggested_action"] = None
    for k in ("excluded", "probes", "assumes_monotone"):
        block[k] = result[k]
    return block


def exit_code(report):
    statuses = [h["status"] for h in report["harnesses"]]
    if "infra_error" in statuses:
        return 2
    if any(s in ("break", "inconclusive") for s in statuses):
        return 1
    return 0


def assemble(harness_blocks, inputs, runner, canary_commit, herdr_threads, generated_at=None):
    blocks = []
    for b in harness_blocks:
        b = dict(b)
        b["suggested_action"] = suggested_action(b)
        blocks.append(b)
    report = {"schema_version": 1,
              "generated_at": generated_at or datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
              "canary_commit": canary_commit, "herdr_threads": herdr_threads,
              "runner": runner, "inputs": inputs, "harnesses": blocks}
    report["exit_code"] = exit_code(report)
    return report


def _cell(s):
    return str(s).replace("|", "\\|").replace("\n", " ")


def render_summary(report):
    out = ["# Harness canary", "",
           f"canary commit `{report['canary_commit']}`, herdr-threads {report['herdr_threads']}, "
           f"runner {report['runner'].get('os')}/{report['runner'].get('arch')}", ""]
    for h in report["harnesses"]:
        out += [f"## {h['harness']} ({h['package']}) — verdict: {h['status']}", "",
                f"verified_max {h['verified_max']}; candidates: {', '.join(h['candidates']) or 'none'}", ""]
        if h["status"] == "break":
            out += [f"first bad: {h['first_bad']}, last good: {h['last_good']}", ""]
        if h["status"] == "known_broken_persists":
            out += ["The failure persists above a declared known_broken range; nothing to file.", ""]
        if h["excluded"]:
            out += [f"excluded by known_broken: {', '.join(h['excluded'])}", ""]
        ids = []
        for p in h["probes"]:
            for c in p["attempts"][-1]["checks"]:
                if c["id"] not in ids:
                    ids.append(c["id"])
        if h["probes"]:
            out += ["| version | role | result | attempts | " + " | ".join(ids) + " |",
                    "|---|---|---|---|" + "---|" * len(ids)]
            for p in h["probes"]:
                st = {c["id"]: c["status"] for c in p["attempts"][-1]["checks"]}
                res = p["result"] + (" (flaky)" if p["flaky"] else "")
                out.append("| " + " | ".join([p["version"], p["role"], res, str(len(p["attempts"]))]
                                             + [_cell(st.get(i, "-")) for i in ids]) + " |")
            out.append("")
        if h["failing_checks"]:
            out += ["failing checks:"] + [f"- {c['id']}: {c['detail']}" for c in h["failing_checks"]] + [""]
        if h["signals"]:
            out += ["signals:"] + [f"- {c['id']}: {c['detail']}" for c in h["signals"]] + [""]
        if h["suggested_action"]:
            sa = h["suggested_action"]
            out += [f"suggested action: {sa['text']}", "", "```", sa["known_broken_snippet"], "```", ""]
        if h["status"] == "inconclusive":
            out += ["Observed results contradict monotonicity; every probe is listed above.", ""]
        if h["assumes_monotone"]:
            out += ["Assumes a monotone break (versions never probed are not checked).", ""]
    out.append(f"exit code: {report['exit_code']}")
    return "\n".join(out) + "\n"


def write(report, out_dir):
    d = pathlib.Path(out_dir)
    d.mkdir(parents=True, exist_ok=True)
    (d / "canary-report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    (d / "summary.md").write_text(render_summary(report), encoding="utf-8")
