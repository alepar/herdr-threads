#!/usr/bin/env python3
"""Count and consistency checker for findings-closure.md (ht-p03.37).

Usage: python3 findings-closure-check.py [--list-ids]

Finding-id rule (the findings doc is docs/history/remaining-findings-2026-10-01.md):
  * Sections B1..B10 are read, B5 is skipped (frozen; owned by another session).
  * Each `- ` bullet is one finding.  Its base id is the leading backticked token
    (`src/store/seats.rs:1217`), else the text before the first ` (` or `:`
    (`W6-R1`, `Wave 18`, `Token overhead`).
  * A bullet whose header carries a count, `Name (N)` or `Name (N, Severity)`,
    aggregates N sub-items and expands to rows `Name[1]` .. `Name[N]`.
  * Other bullets whose (bucket, base id) repeats inside a section are numbered
    in document order: `Wave 17#1`, `Wave 17#2`, ...
  * The ledger key of a finding is (bucket, id).

Ledger table columns (header row required):
  | id | bucket | severity | closing bead | commit | status | backing cells | finding |
  * closing bead: comma-separated bead ids; every bead must exist (`bd show`).
  * commit: comma-separated SHAs; every SHA must exist (`git cat-file -e`) and its
    subject must name the closing bead it stands for; there must be one SHA for every
    closing bead other than ht-p03.20.  A closing bead that is a seam integration
    bead (ht-p03.40/.42/.44/.46/.48, .12.11, .12.13, .14.9) therefore always carries
    its own commit next to the bucket bead's.
  * status: `closed`, `pending ht-p03.20` (needs >= 1 backing cell), `open` (needs
    >= 1 backing cell, which names the cell that leaves it open) or
    (after the ht-p03.20 rerun) a `closed` row naming ht-p03.20, whose backing cells must
    all PASS on the evidence SHA in native-rerun.json; an `open` row needs a non-PASS cell there; or
    `deferred: <reason>`; a deferral reason must cite a bd id that exists or a
    phrase in double square brackets that appears in the root spec's
    "Explicit deferrals" table.
  * severity must equal the severity word in the findings bullet ("-" when none).
Exit 0 only when every check passes.
"""
import re
import subprocess
import sys
from collections import Counter, OrderedDict
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = Path(subprocess.check_output(["git", "-C", str(HERE), "rev-parse", "--show-toplevel"], text=True).strip())
FINDINGS = REPO / "docs/history/remaining-findings-2026-10-01.md"
LEDGER = HERE / "findings-closure.md"
ROOT_SPEC = REPO / "docs/design/herdr-threads/remaining-findings/2026-10-01-remaining-herdr-threads-findings-design.md"
SKIP_BUCKETS = {"B5"}
SEAM_BEADS = {"ht-p03.40", "ht-p03.42", "ht-p03.44", "ht-p03.46", "ht-p03.48",
              "ht-p03.12.11", "ht-p03.12.13", "ht-p03.14.9"}
PENDING = "ht-p03.20"
SEVERITIES = ("Should-fix", "Nit", "Minor", "Gap", "UX", "accept", "Escalation", "Observation", "Blocker")


def parse_findings():
    """Return an ordered list of (bucket, id, severity) for every finding row owed."""
    rows = []
    bucket = None
    per_section = []  # (bucket, base, severity, aggregate_n, text)
    for line in FINDINGS.read_text().splitlines():
        m = re.match(r"^## (B\d+)\.", line)
        if m:
            bucket = m.group(1)
            continue
        if line.startswith("## "):
            bucket = None
            continue
        if not bucket or bucket in SKIP_BUCKETS or not line.startswith("- "):
            continue
        body = line[2:]
        m = re.match(r"`([^`]+)`", body)
        if m:
            base = m.group(1)
            rest = body[m.end():]
        else:
            m = re.match(r"(.+?)(?: \(|:)", body)
            if not m:
                raise SystemExit(f"cannot derive an id from findings bullet: {line}")
            base = m.group(1)
            rest = body[len(base):]
        agg = re.match(r"\s*\((\d+)(?:, ([A-Za-z-]+))?\)", rest)
        sev_m = re.match(r"\s*(?:`[^`]*`\s*)?\(([A-Za-z-]+)", rest) if not agg else None
        if agg:
            n = int(agg.group(1))
            sev = agg.group(2) or "-"
        else:
            n = 0
            sev = "-"
            for sm in re.finditer(r"\(([A-Za-z-]+)[,)]", rest[:80]):
                if sm.group(1) in SEVERITIES:
                    sev = sm.group(1)
                    break
        per_section.append((bucket, base, sev, n, body))
    counts = Counter((b, base) for b, base, _, n, _t in per_section if not n)
    seen = Counter()
    for bucket, base, sev, n, text in per_section:
        if n:
            for k in range(1, n + 1):
                rows.append((bucket, f"{base}[{k}]", sev, text))
        elif counts[(bucket, base)] > 1:
            seen[(bucket, base)] += 1
            rows.append((bucket, f"{base}#{seen[(bucket, base)]}", sev, text))
        else:
            rows.append((bucket, base, sev, text))
    return rows


def parse_ledger():
    text = LEDGER.read_text()
    rows = []
    header = None
    for line in text.splitlines():
        if not line.startswith("|"):
            if header is not None:
                break  # the ledger is the first table with the id/closing-bead header
            continue
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        if cells and cells[0] == "id" and "closing bead" in cells:
            header = cells
            continue
        if header is None or set(line.replace("|", "").strip()) <= set("-: "):
            continue
        if len(cells) != len(header):
            raise SystemExit(f"ledger row has {len(cells)} cells, header has {len(header)}: {line[:100]}")
        rows.append(dict(zip(header, cells)))
    if header is None:
        raise SystemExit("no ledger table header found in findings-closure.md")
    return rows


_bd_cache = {}


def bead_exists(bead):
    if bead not in _bd_cache:
        r = subprocess.run(["bd", "show", bead], capture_output=True, text=True, cwd=REPO)
        _bd_cache[bead] = r.returncode == 0 and bead in r.stdout
    return _bd_cache[bead]


def sha_info(sha):
    r = subprocess.run(["git", "-C", str(REPO), "cat-file", "-e", f"{sha}^{{commit}}"], capture_output=True)
    if r.returncode != 0:
        return None
    return subprocess.check_output(["git", "-C", str(REPO), "log", "-1", "--format=%s", sha], text=True).strip()


def deferral_table_text():
    text = ROOT_SPEC.read_text()
    start = text.index("## Explicit deferrals")
    end = text.index("## Follow-on", start)
    return text[start:end].lower()


RERUN = HERE / "native-rerun.json"
rerun = None


def backing_cells(text):
    import json as _json  # noqa: F401
    known = {c["cell"] for c in (rerun or {}).get("cells", [])}
    return [w for w in re.findall(r"[a-z0-9][a-z0-9.-]+", text) if w in known]


def rerun_outcome(cell):
    for c in (rerun or {}).get("cells", []):
        if c["cell"] == cell:
            return c["outcome"] if c.get("sha") == rerun.get("evidence_sha") else "stale"
    return "not run"


def rerun_pass(cell):
    return rerun_outcome(cell) in ("PASS", "PASS (flaky)")


def main():
    global rerun
    if RERUN.is_file():
        import json
        rerun = json.loads(RERUN.read_text())
    if "--list-ids" in sys.argv:
        for b, i, s, t in parse_findings():
            print(f"{b}\t{i}\t{s}\t{t}")
        return 0
    owed = parse_findings()
    ledger = parse_ledger()
    errors = []
    owed_keys = OrderedDict(((b, i), s) for b, i, s, _t in owed)
    if len(owed_keys) != len(owed):
        errors.append("findings doc yields duplicate (bucket, id) keys; fix the id rule")
    led_keys = Counter((r["bucket"], r["id"]) for r in ledger)
    for k, n in led_keys.items():
        if n > 1:
            errors.append(f"ledger has {n} rows for {k}")
        if k not in owed_keys:
            errors.append(f"ledger row without a finding: {k}")
    for k in owed_keys:
        if k not in led_keys:
            errors.append(f"finding without a ledger row: {k}")

    defer_text = deferral_table_text()
    stats = Counter()
    for r in ledger:
        key = (r["bucket"], r["id"])
        tag = f"{r['bucket']} {r['id']}"
        if key in owed_keys and r["severity"] != owed_keys[key]:
            errors.append(f"{tag}: severity {r['severity']!r}, findings doc says {owed_keys[key]!r}")
        beads = [b.strip() for b in r["closing bead"].split(",") if b.strip() and b.strip() != "-"]
        shas = [s.strip() for s in r["commit"].split(",") if s.strip() and s.strip() != "-"]
        status = r["status"]
        cells = r["backing cells"].strip()
        for b in beads:
            if not bead_exists(b):
                errors.append(f"{tag}: closing bead {b} does not exist")
        sha_subjects = []
        for s in shas:
            subj = sha_info(s)
            if subj is None:
                errors.append(f"{tag}: commit {s} does not exist")
            sha_subjects.append(subj or "")
        if status == "closed":
            stats["closed"] += 1
            need = [b for b in beads if b != PENDING]
            if PENDING in beads:
                # Closed by the final rerun: every backing cell must PASS on the evidence SHA in native-rerun.json.
                if rerun is None:
                    errors.append(f"{tag}: closed row names {PENDING} but native-rerun.json is absent")
                for c in backing_cells(cells):
                    if not rerun_pass(c):
                        errors.append(f"{tag}: closed by {PENDING} but backing cell {c} is {rerun_outcome(c)}")
                if not backing_cells(cells):
                    errors.append(f"{tag}: closed by {PENDING} but names no backing cell")
            elif not need:
                errors.append(f"{tag}: closed row names no closing bead")
            if len(shas) < len(need):
                errors.append(f"{tag}: {len(need)} closing beads but {len(shas)} commits (seam beads need their own commit)")
            for b in need:
                pat = re.compile(re.escape(b) + r"(?![0-9]|\.[0-9])")
                if shas and not any(pat.search(subj) for subj in sha_subjects):
                    errors.append(f"{tag}: no listed commit subject names {b}")
        elif status == f"pending {PENDING}":
            stats["pending"] += 1
            if PENDING not in beads:
                errors.append(f"{tag}: pending row must name {PENDING} as a closing bead")
            if not cells or cells == "-":
                errors.append(f"{tag}: pending row names no backing cell")
            need = [b for b in beads if b != PENDING]
            if len(shas) < len(need):
                errors.append(f"{tag}: {len(need)} non-{PENDING} beads but {len(shas)} commits")
        elif status == "open":
            stats["open"] += 1
            if not cells or cells == "-":
                errors.append(f"{tag}: open row must name the backing cell that leaves it open")
            elif rerun is not None and all(rerun_pass(c) for c in backing_cells(cells)):
                errors.append(f"{tag}: open row, but every named backing cell PASSes in native-rerun.json")
        elif status.startswith("deferred:"):
            stats["deferred"] += 1
            reason = status[len("deferred:"):].strip()
            ids = re.findall(r"\bht-[a-z0-9]+(?:\.[0-9]+)*\b", reason)
            phrases = re.findall(r"\[\[(.+?)\]\]", reason)
            if not ids and not phrases:
                errors.append(f"{tag}: deferral cites neither a bd id nor a [[spec deferral phrase]]")
            for i in ids:
                if not bead_exists(i):
                    errors.append(f"{tag}: deferral cites {i}, which does not exist")
            for p in phrases:
                if p.lower() not in defer_text:
                    errors.append(f"{tag}: deferral phrase {p!r} is not in the root spec's Explicit deferrals table")
        else:
            errors.append(f"{tag}: status {status!r} is not closed / pending {PENDING} / open / deferred: <reason>")
        if r["id"].startswith("Process debt[") and "owner:" not in r.get("finding", ""):
            errors.append(f"{tag}: process-debt row names no owner (needs 'owner: ...' in the finding column)")
        for b in beads:
            if b in SEAM_BEADS and status == "closed" and len(shas) < 2:
                errors.append(f"{tag}: seam-closed row names {b} but fewer than two commits")

    per_bucket = Counter(r["bucket"] for r in ledger)
    owed_bucket = Counter(b for b, _ in owed_keys)
    print("bucket  findings  ledger rows")
    for b in sorted(owed_bucket, key=lambda x: int(x[1:])):
        print(f"{b:6}  {owed_bucket[b]:8}  {per_bucket.get(b, 0):11}")
    print(f"total   {sum(owed_bucket.values()):8}  {len(ledger):11}")
    print("status: " + ", ".join(f"{k}={v}" for k, v in sorted(stats.items())))
    if errors:
        print(f"\nFAIL: {len(errors)} problem(s)")
        for e in errors:
            print("  - " + e)
        return 1
    print("\nOK: every finding has exactly one consistent ledger row")
    return 0


if __name__ == "__main__":
    sys.exit(main())
