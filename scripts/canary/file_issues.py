#!/usr/bin/env python3
"""File deduplicated GitHub issues from a canary-report.json (nested spec §D10). Pure stdlib.

One issue per harness whose status is `break` or `inconclusive`. The exact title is the dedupe key.
An existing open issue gets a comment only when the harness block's digest changed; a new one gets
`gh label create harness-canary --force` then `gh issue create`. Never closes or edits issues and
never opens PRs. `--dry-run` prints the gh commands instead of running them.
"""
import sys
# scripts/canary/bisect.py would shadow the stdlib `bisect` (random imports it) when this directory
# is sys.path[0]; drop it before anything else is imported.
_here = __import__("os").path.dirname(__import__("os").path.abspath(__file__))
sys.path[:] = [p for p in sys.path if p not in ("", _here)]

import argparse, hashlib, json, os, re, shlex, subprocess

LABEL = "harness-canary"
ARTIFACT = "harness-canary-report"
MARKER = re.compile(r"<!-- canary-digest: ([0-9a-f]{64}) -->")
DETAIL_LINES = 40


def strip_timings(o):
    if isinstance(o, dict):
        return {k: strip_timings(v) for k, v in o.items() if k != "duration_ms"}
    if isinstance(o, list):
        return [strip_timings(v) for v in o]
    return o


def digest(block):
    """sha256 of the harness block minus timings (canonical JSON)."""
    data = json.dumps(strip_timings(block), sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    return hashlib.sha256(data.encode("utf-8")).hexdigest()


def title_for(block):
    h = block["harness"]
    if block["status"] == "break":
        return f"harness-canary: {h} {block['first_bad']} breaks herdr-threads"
    return f"harness-canary: {h} inconclusive above {block.get('last_good') or block['verified_max']}"


def excerpt(detail):
    lines = str(detail).splitlines() or [""]
    if len(lines) > DETAIL_LINES:
        lines = lines[:DETAIL_LINES] + [f"... ({len(lines) - DETAIL_LINES} more lines in the artifact)"]
    return "\n".join(lines)


def _cell(s):
    return str(s).replace("|", "\\|").replace("\n", " ")


def probe_table(block):
    ids = []
    for p in block["probes"]:
        for c in p["attempts"][-1]["checks"]:
            if c["id"] not in ids:
                ids.append(c["id"])
    if not block["probes"]:
        return ["(no probes were run)"]
    rows = ["| version | role | result | attempts | " + " | ".join(ids) + " |", "|---|---|---|---|" + "---|" * len(ids)]
    for p in block["probes"]:
        st = {c["id"]: c["status"] for c in p["attempts"][-1]["checks"]}
        res = p["result"] + (" (flaky)" if p["flaky"] else "")
        rows.append("| " + " | ".join([p["version"], p["role"], res, str(len(p["attempts"]))]
                                      + [_cell(st.get(i, "-")) for i in ids]) + " |")
    return rows


def body_for(block, report, run_url, artifact):
    out = [f"verdict: {block['status']}", ""]
    if block["status"] == "break":
        out += [f"Range: >= {block['first_bad']}", f"Last good version: {block['last_good']}", ""]
    else:
        out += [f"Last good version: {block.get('last_good') or 'none'} (verified_max {block['verified_max']})",
                "Observed results contradict monotonicity, or the baseline failed on confirmation; "
                "every probe is listed below.", ""]
    out += [f"Harness: {block['harness']} ({block['package']}), candidates: {', '.join(block['candidates']) or 'none'}",
            f"Canary commit {report['canary_commit']}, herdr-threads {report['herdr_threads']}, "
            f"runner {report['runner'].get('os')}/{report['runner'].get('arch')}", ""]
    if block["failing_checks"]:
        out += ["Failing checks:", ""]
        for c in block["failing_checks"]:
            out += [f"- `{c['id']}`", "", "```", excerpt(c["detail"]), "```", ""]
    out += ["Probes:", ""] + probe_table(block) + [""]
    out += [f"Run: {run_url or '(not given)'}", f"Artifact: `{artifact}` (canary-report.json, summary.md, probe logs)", ""]
    sa = block.get("suggested_action")
    if sa:
        out += [f"Suggested action: {sa['text']}", "", "```", sa["known_broken_snippet"], "```", ""]
    else:
        out += ["Suggested action: read the probe table, then add an adapter recipe for the versions above "
                "the last good one or mark them known_broken until one exists.", ""]
    out.append(f"<!-- canary-digest: {digest(block)} -->")
    return "\n".join(out) + "\n"


def comment_for(block, report, run_url, artifact):
    return ("The canary ran again and the result for this harness changed.\n\n"
            + body_for(block, report, run_url, artifact))


def latest_digest(issue):
    """Digest of the newest comment carrying one, else the body's."""
    for c in reversed(issue.get("comments") or []):
        m = MARKER.findall(c.get("body") or "")
        if m:
            return m[-1]
    m = MARKER.findall(issue.get("body") or "")
    return m[-1] if m else None


class Gh:
    def __init__(self, dry_run):
        self.dry_run = dry_run

    def show(self, argv, stdin=None):
        print("+ " + " ".join(shlex.quote(a) for a in argv))
        if stdin is not None:
            print("  --- body ---")
            print("\n".join("  " + l for l in stdin.rstrip("\n").splitlines()))
            print("  --- end body ---")

    def run(self, argv, stdin=None, capture=False):
        if self.dry_run:
            self.show(argv, stdin)
            return None
        p = subprocess.run(argv, input=stdin, capture_output=capture, text=True)
        if p.returncode != 0:
            sys.stderr.write(f"file_issues.py: {' '.join(argv[:4])} failed ({p.returncode}): {p.stderr or ''}\n")
            raise SystemExit(2)
        return p.stdout


def lookup(gh, repo, title, existing):
    """Open issues whose title equals `title` exactly (newest number first)."""
    if existing is None:
        argv = ["gh", "issue", "list", "--repo", repo, "--state", "open", "--label", LABEL,
                "--search", f"{title} in:title", "--json", "number,title,body,comments"]
        if gh.dry_run:
            gh.show(argv)
            return []
        issues = json.loads(gh.run(argv, capture=True) or "[]")
    else:
        issues = existing
    return sorted((i for i in issues if i.get("title") == title), key=lambda i: -i["number"])


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--report", required=True)
    ap.add_argument("--repo", required=True)
    ap.add_argument("--run-url", default="")
    ap.add_argument("--artifact-name", default=ARTIFACT)
    ap.add_argument("--dry-run", action="store_true", help="print the gh commands instead of running them")
    ap.add_argument("--existing-issues", help="JSON shaped like `gh issue list --json number,title,body` "
                    "(offline replacement for the open-issue lookup)")
    a = ap.parse_args(argv)
    if not a.dry_run and not (os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")):
        sys.stderr.write("file_issues.py: GH_TOKEN is not set (use --dry-run to print the commands)\n")
        return 2
    with open(a.report, encoding="utf-8") as f:
        report = json.load(f)
    existing = None
    if a.existing_issues:
        with open(a.existing_issues, encoding="utf-8") as f:
            existing = json.load(f)
    gh = Gh(a.dry_run)
    label_made = False
    for block in report["harnesses"]:
        if block["status"] not in ("break", "inconclusive"):
            continue
        title = title_for(block)
        found = lookup(gh, a.repo, title, existing)
        if found:
            issue = found[0]
            if latest_digest(issue) == digest(block):
                print(f"issue #{issue['number']} ({title}): unchanged, nothing to do")
                continue
            gh.run(["gh", "issue", "comment", str(issue["number"]), "--repo", a.repo, "--body-file", "-"],
                   stdin=comment_for(block, report, a.run_url, a.artifact_name))
            continue
        if not label_made:
            gh.run(["gh", "label", "create", LABEL, "--force", "--repo", a.repo,
                    "--description", "Harness version canary findings", "--color", "d93f0b"])
            label_made = True
        gh.run(["gh", "issue", "create", "--repo", a.repo, "--title", title, "--label", LABEL, "--body-file", "-"],
               stdin=body_for(block, report, a.run_url, a.artifact_name))
    return 0


if __name__ == "__main__":
    sys.exit(main())
