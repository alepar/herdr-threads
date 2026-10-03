#!/usr/bin/env python3
"""Reconcile committed herdr-threads validation evidence into one report.

Reads the committed evidence folders of the herdr-native-mailbox-thread-plugin
run (native demo/matrix/TUI runs, the isolated host-recovery suite, the
package suite and the harness hook captures) and emits a scenario x harness
matrix with per-step verdicts plus receipt accounting.

Rules (validation design, "Reporting and completion criteria"):
- Only a live (non-dry-run, non-diagnostic) native run can decide a cell.
- NOT_EXERCISED, UNVERIFIED and UNSUPPORTED are reported as themselves and
  never counted as PASS.
- A required native cell without a passing live run makes the report FAIL.
- ACKed, pending, retired, duplicate logical pairs, duplicate receipt rows and
  repeated transport hints are counted separately; an ACK counts only when the
  model call and the SQLite receipt join on (call_id, message_id,
  recipient_id, actor).
- Every evidence row is stamped with the code SHA it was produced on. Later
  code changes invalidate the stamp.

Usage:
  scripts/reconcile-validation.py [--evidence-root DIR] [--output FILE]
                                  [--json] [--base REV] [--no-git] [--native-rerun JSON]
  scripts/reconcile-validation.py --collect-native-rerun MATRIX_LOG --write JSON

Evidence root: the full run evidence tree is not in the released tree; it is
archived in git tag `archive/herdr-threads-run-2026-09-26` at
`docs/superpowers/runs/2026-09-26-herdr-native-mailbox-thread-plugin/`.
Extract it, for example

  git archive archive/herdr-threads-run-2026-09-26 docs/superpowers/runs \
    | tar -x -C /tmp/ht-archive

and pass `--evidence-root /tmp/ht-archive/docs/superpowers/runs/2026-09-26-herdr-native-mailbox-thread-plugin`
(or set HERDR_THREADS_EVIDENCE_ROOT). Without either, the default is that path
inside this checkout, which exists only in a checkout of the archive tag.

Offline: reads only committed files (and, unless --no-git, `git diff
--name-only` to measure code drift since each evidence SHA). Starts no model,
daemon or Herdr server.
"""

import argparse
import json
import os
import pathlib
import re
import subprocess
import sys

REPO = pathlib.Path(__file__).resolve().parent.parent
RUN_REL = "docs/superpowers/runs/2026-09-26-herdr-native-mailbox-thread-plugin"
DEFAULT_OUTPUT = "docs/validation/report.md"
ARCHIVE_TAG = "archive/herdr-threads-run-2026-09-26"
EVIDENCE_ENV = "HERDR_THREADS_EVIDENCE_ROOT"
# Evidence folders also kept in the released tree, relative to docs/validation/.
IN_TREE_EVIDENCE = "../evidence"

NATIVE_FOLDERS = [
    "native-claude-demo-4",
    "native-codex-demo-3",
    "native-claude-matrix-1",
    "native-claude-matrix-2",
    "native-codex-matrix-1",
    "native-codex-matrix-2",
    "native-claude-tui-2",
    "native-claude-tui-3",
    "native-matrix-w11",
    "native-matrix-w12",
]
HOST_FOLDER = "host-recovery-validation-tip"
PACKAGE_FOLDER = "package-validation-tip"
CAPTURE_FOLDERS = {
    # folder: (harness, version token that must appear in report.md, summary)
    "claude-286-hook-capture": (
        "claude", "2.1.286",
        "live hook input/output capture; recipe widened to [2.1.283, 2.1.286]"),
    "codex-158-live-hook-capture": (
        "codex", "0.158.0",
        "live hook capture; SessionStart/PreToolUse additionalContext delivered on 0.158.0"),
}

HARNESSES = ("claude", "codex")
NON_PASS = ("FAIL", "BLOCKED", "UNSUPPORTED", "UNVERIFIED", "NOT_EXERCISED")
# Worst first. BLOCKED only follows an earlier FAIL in the driver.
SEVERITY = ["FAIL", "BLOCKED", "UNSUPPORTED", "UNVERIFIED", "NOT_EXERCISED", "PASS"]

# Scenario rows. `select(run)` decides whether a live run exercises the row;
# `steps` are the driver step IDs that decide it. `required` rows must PASS
# on both harnesses or the report FAILs. `gap` rows are known owed-but-open
# items: their NOT_EXERCISED/UNVERIFIED verdicts are listed as gaps (still
# never PASS); a FAIL there still fails the report.
ROWS = [
    {"id": "prelaunch-manual", "req": "R5, R24", "required": True,
     "title": "prelaunch handoff, manual launch (accept + exact ACK)",
     "steps": ["S16P", "S17", "S17H", "S18", "S18C"],
     "select": lambda r: r["launch"] != "managed" and not r["tokens"]},
    {"id": "prelaunch-managed", "req": "R5, R24", "required": True,
     "title": "prelaunch handoff, managed `herdr-threads launch`",
     "steps": ["S17M", "S17", "S17H", "S18", "S18C"],
     "select": lambda r: r["launch"] == "managed"},
    {"id": "lost-prompt", "req": "R5, F15", "required": True,
     "title": "lost initial prompt: idle recovery wake, then ACK",
     "steps": ["SL1", "SL2", "S18"],
     "select": lambda r: "lostprompt" in r["tokens"]},
    {"id": "restart-resume", "req": "R2", "required": True,
     "title": "restart + resume in the same pane keep membership/receipts",
     "steps": [],
     "select": lambda r: "restart" in r["phases"] or "resume" in r["phases"],
     "phases": ("restart", "resume")},
    {"id": "clear-new", "req": "R2", "required": True,
     "title": "/clear (Claude) or /new (Codex) in the same pane",
     "steps": ["S19X"],
     "select": lambda r: "clear" in r["phases"],
     "phases": ("clear",),
     "note": "S19X (conversation cleared before the clear-phase handoff is sent) exists only in "
             "drivers from `b428fdd` on; earlier TUI clear runs lack it and are UNVERIFIED, not PASS."},
    {"id": "deadline-warning", "req": "R11, R19", "required": True,
     "title": "missed deadline: one durable warning, late ACK kept",
     "steps": ["SW0", "SW1"],
     "select": lambda r: "warning" in r["tokens"]},
    {"id": "coalesced-warning-wake", "req": "R13", "required": False, "gap": True,
     "title": "warning-only coalesced wake (SW2)",
     "steps": ["SW2"],
     "select": lambda r: "warning" in r["tokens"],
     "note": "Not separately owed live: the warning was already offered at the woken "
             "agent's verified check-in. Covered deterministically by the isolated "
             "host stand-in scenario R13 (`tests/native/recovery/run_recovery.py`), "
             "not by a model."},
    {"id": "midturn", "req": "R19", "required": True,
     "title": "active tool-boundary arrival (PreToolUse) in the same turn",
     "steps": ["SM1", "SM2", "SM3"],
     "select": lambda r: "midturn" in r["tokens"]},
    {"id": "child", "req": "R4", "required": True,
     "title": "child read allowed, child ACK/accept absent, parent ACKs",
     "steps": ["SC1", "SC2", "SC3"],
     "select": lambda r: "child" in r["tokens"],
     "note": "Child ACK absence is proven at the transcript (sidechain) level. "
             "DB-level child separation is the accepted cooperative B1 limit: a child "
             "write would carry the root's seat/provenance/execution."},
    {"id": "concurrent-children", "req": "R4, R24", "required": False, "gap": True,
     "title": "two concurrent children (SK1)",
     "steps": ["SK1"],
     "select": lambda r: "children" in r["tokens"]},
    {"id": "children-write-absence", "req": "R4", "required": False, "gap": True,
     "title": "TUI children: no child write, top-level ACK (SK2, SK3)",
     "steps": ["SK2", "SK3"],
     "select": lambda r: "children" in r["tokens"]},
    {"id": "blocked-ui", "req": "R19", "required": True,
     "title": "approval UI untouched; wake refused `unsafe`; receipt stays pending",
     "steps": ["SU0", "SU1", "SU2"],
     "select": lambda r: "blockedui" in r["tokens"]},
    {"id": "burst", "req": "R14, R24", "required": True,
     "title": "burst > one inbox page: continuation + accept + exact ACK",
     "steps": ["SB0", "SB1", "SB2", "S18"],
     "select": lambda r: "burst" in r["tokens"]},
    {"id": "required-membership", "req": "R9", "required": True,
     "title": "service-required invitation: accept-required, leave refused",
     "steps": ["SR0", "SR1", "SR2", "SR3"],
     "select": lambda r: "required" in r["tokens"]},
]

HOME_RE = re.compile(r"/(?:Users|home)/[^/\s\"']+")
EMAIL_RE = re.compile(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}")


def redact(text):
    text = HOME_RE.sub("~", str(text))
    return EMAIL_RE.sub("<email>", text)


def load_json(path):
    with open(path, encoding="utf-8") as handle:
        return json.load(handle)


# ---------------------------------------------------------------- receipts

def _pair(record):
    return (record["message_id"], record["recipient_id"])


def reconcile_manifest(manifest):
    """Receipt accounting for one manifest (validation evidence schema v1)."""
    accepted_list = [_pair(r) for r in manifest.get("accepted_pairs", [])]
    accepted = set(accepted_list)
    calls = {(r["call_id"], r["message_id"], r["recipient_id"], r["actor"])
             for r in manifest.get("model_calls", [])}
    receipts = manifest.get("db_receipts", [])
    acked = {_pair(r) for r in receipts
             if (r["call_id"], r["message_id"], r["recipient_id"], r["actor"]) in calls
             and _pair(r) in accepted}
    unjoined = {_pair(r) for r in receipts} - acked
    retired = {_pair(r) for r in manifest.get("retired_pairs", [])} & accepted
    hints = [_pair(r) for r in manifest.get("transport_hints", [])]
    return {
        "accepted": len(accepted),
        "acked": len(acked),
        "pending": len(accepted - acked - retired),
        "retired": len(retired - acked),
        "acked_and_retired": len(acked & retired),
        "duplicate_logical_pairs": len(accepted_list) - len(accepted),
        "duplicate_receipt_rows": len(receipts) - len({_pair(r) for r in receipts}),
        "unjoined_receipts": len(unjoined),
        "transport_hints": len(hints),
        "repeated_transport_hints": len(hints) - len(set(hints)),
    }


STATE_RANK = {"acked": 3, "retired": 2, "pending": 1}


def sqlite_receipt_states(run_dir):
    """Final receipt state per (message, seat) across read-only SQLite snapshots."""
    states = {}
    for snap in sorted(pathlib.Path(run_dir).glob("sqlite-*.json")):
        try:
            data = load_json(snap)
        except (OSError, ValueError):
            continue
        if not isinstance(data, dict):
            continue
        for row in data.get("receipts") or []:
            key = (row.get("message_id"), row.get("seat_id"))
            state = row.get("state")
            if STATE_RANK.get(state, 0) >= STATE_RANK.get(states.get(key), 0):
                states[key] = state
    counts = {}
    for state in states.values():
        counts[state] = counts.get(state, 0) + 1
    return states, counts


# ---------------------------------------------------------------- native runs

def scenario_tokens(scenario):
    parts = scenario.split(".")
    return [p for p in parts[1:] if p and p != "managed"]


def phase_order(facts):
    started = facts.get("phase_started_utc") or {}
    names = list((facts.get("messages") or {}).keys())
    return sorted(names, key=lambda n: (n != "initial", started.get(n, "~"), n))


def load_native_run(folder, run_dir, root):
    manifest = load_json(run_dir / "manifest.json")
    summary = load_json(run_dir / "summary.json")
    facts = summary.get("facts", {}) or {}
    rel = run_dir.relative_to(root).as_posix()
    scenario = manifest.get("scenario", "")
    dry = bool(facts.get("dry_run")) or manifest.get("source") == "synthetic"
    diagnostic = "diag" in run_dir.name
    kind = "dry-run" if dry else ("diagnostic" if diagnostic else "live")
    launch = facts.get("launch") or summary.get("launch")
    if launch is None:
        launch = "managed" if ".managed" in scenario else "manual"
    steps = {}
    for step in summary.get("steps", []):
        steps.setdefault(step["step"], []).append(step)
    versions = facts.get("versions", {}) or {}
    harness_version = (facts.get("harness_version") or {}).get("installed")
    herdr = (versions.get("herdr") or {}).get("out", "")
    states, sqlite_counts = sqlite_receipt_states(run_dir)
    accounting = reconcile_manifest(manifest)
    acked_pairs = set()
    calls = {(r["call_id"], r["message_id"], r["recipient_id"], r["actor"])
             for r in manifest.get("model_calls", [])}
    for r in manifest.get("db_receipts", []):
        if (r["call_id"], r["message_id"], r["recipient_id"], r["actor"]) in calls:
            acked_pairs.add(_pair(r))
    sqlite_mismatch = sorted(p for p in acked_pairs if states and states.get(p) != "acked")
    accepted = {_pair(r) for r in manifest.get("accepted_pairs", [])}
    # Acked in the database but with no joined model call: issuance unverified,
    # so it stays pending in the accounting and is never a model receipt.
    accounting["db_acked_unjoined"] = len([p for p in accepted - acked_pairs if states.get(p) == "acked"])
    return {
        "folder": folder,
        "path": rel,
        "name": run_dir.relative_to(root / folder).as_posix(),
        "kind": kind,
        "harness": facts.get("harness") or scenario.split("-")[0],
        "mode": facts.get("mode") or ("tui" if "-tui-" in scenario else "print"),
        "launch": launch,
        "scenario": scenario,
        "tokens": scenario_tokens(scenario),
        "phases": phase_order(facts),
        "manifest_status": manifest.get("status"),
        "manifest_reason": manifest.get("reason", ""),
        "manifest_source": manifest.get("source"),
        "started": facts.get("started_utc") or "",
        "code_sha": versions.get("driver_repo_head", ""),
        "binary_sha256": versions.get("herdr_threads_sha256", ""),
        "harness_version": harness_version or "",
        "herdr_version": herdr.replace("herdr ", ""),
        "not_exercised": summary.get("not_exercised") or [],
        "unverified": summary.get("unverified") or [],
        "steps": steps,
        "accounting": accounting,
        "sqlite_counts": sqlite_counts,
        "sqlite_mismatch": sqlite_mismatch,
    }


def discover_native(root, folders):
    runs = []
    for folder in folders:
        base = root / folder
        if not base.is_dir():
            runs.append({"folder": folder, "missing": True})
            continue
        for manifest in sorted(base.rglob("manifest.json")):
            run_dir = manifest.parent
            if (run_dir / "summary.json").is_file():
                runs.append(load_native_run(folder, run_dir, root))
    return runs


def step_status(run, step_id, phase=None):
    entries = run["steps"].get(step_id, [])
    if not entries:
        return None
    statuses = [e["status"] for e in entries]
    for status in SEVERITY:
        if status in statuses:
            return status
    if all(s in ("SKIPPED", "INFO") for s in statuses):
        return statuses[0]
    return statuses[0]


# The driver numbers later phases by position, not by name: the second phase
# (restart, resume or clear) uses S19-S21C, the third S22-S24C.
PHASE_POSITION_STEPS = {1: ["S20", "S20H", "S21", "S21C"], 2: ["S23", "S23H", "S24", "S24C"]}


def row_steps(row, run):
    steps = list(row["steps"])
    for phase in row.get("phases", ()):
        if phase in run["phases"]:
            steps += PHASE_POSITION_STEPS.get(run["phases"].index(phase), [])
    if row["id"] == "deadline-warning" and run["mode"] != "tui":
        steps = [s for s in steps if s != "SW0"]  # SW0 is the TUI idle step only
    return steps


def run_row_verdict(row, run):
    statuses = {}
    for step in row_steps(row, run):
        status = step_status(run, step)
        statuses[step] = status or "MISSING"
    observed = [s for s in statuses.values() if s not in ("MISSING", "SKIPPED", "INFO")]
    if not observed:
        return "NOT_EXERCISED", statuses
    for status in SEVERITY:
        if status in observed:
            verdict = status
            break
    else:
        verdict = "UNVERIFIED"
    if verdict == "BLOCKED":
        verdict = "FAIL"
    if verdict == "PASS" and "MISSING" in statuses.values():
        verdict = "UNVERIFIED"
    return verdict, statuses


def build_matrix(runs):
    live = [r for r in runs if not r.get("missing") and r["kind"] == "live"]
    matrix = []
    for row in ROWS:
        cells = {}
        for harness in HARNESSES:
            candidates = [r for r in live if r["harness"] == harness and row["select"](r)]
            candidates.sort(key=lambda r: (r["started"], r["path"]))
            attempts = []
            for run in candidates:
                verdict, statuses = run_row_verdict(row, run)
                attempts.append({"run": run, "verdict": verdict, "steps": statuses})
            if attempts:
                deciding = attempts[-1]
                verdict = deciding["verdict"]
            else:
                deciding = None
                verdict = "NO_EVIDENCE"
            cells[harness] = {"verdict": verdict, "deciding": deciding, "attempts": attempts}
        matrix.append({"row": row, "cells": cells})
    return matrix


def overall(matrix, runs, host, package):
    failures, gaps = [], []
    for entry in matrix:
        row = entry["row"]
        for harness, cell in entry["cells"].items():
            verdict = cell["verdict"]
            label = f"{row['id']} / {harness}: {verdict}"
            if verdict == "PASS":
                continue
            if verdict == "FAIL":
                failures.append(label)
            elif row.get("required"):
                failures.append(label + " (required native support missing)")
            else:
                gaps.append(label)
    for run in runs:
        if run.get("missing"):
            failures.append(f"evidence folder {run['folder']} missing")
        elif run["kind"] == "live" and run["manifest_status"] == "PASS" and (
                run["accounting"]["pending"] or run["sqlite_mismatch"]):
            failures.append(f"{run['path']}: manifest PASS with pending or unmatched receipts")
    if host is None or host.get("status") != "PASS":
        failures.append("host-recovery suite not PASS")
    if package is None or package.get("status") != "PASS":
        failures.append("package suite not PASS")
    return ("FAIL" if failures else "PASS_WITH_GAPS" if gaps else "PASS"), failures, gaps


# ---------------------------------------------------------------- deterministic

def load_host(root):
    path = root / HOST_FOLDER
    results = sorted(path.glob("*/results.json"))
    if not results:
        return None
    data = load_json(results[0])
    scenarios = []
    for sc in data.get("scenarios", []):
        checks = sc.get("checks", [])
        ok = sum(1 for c in checks if c.get("ok"))
        scenarios.append({"id": sc.get("id"), "title": sc.get("title", ""),
                          "status": sc.get("status") or sc.get("result"),
                          "checks": f"{ok}/{len(checks)}"})
    counts = data.get("counts", {})
    status = "PASS" if counts.get("FAIL", 1) == 0 and counts.get("NOT_RUN", 1) == 0 else "FAIL"
    return {"path": results[0].relative_to(root).as_posix(), "status": status,
            "counts": counts, "code_sha": data.get("source_commit", ""),
            "binary_sha256": (data.get("binary") or {}).get("sha256", ""),
            "herdr": (data.get("herdr") or {}).get("version", "").replace("herdr ", ""),
            "platform": data.get("platform", ""), "scenarios": scenarios}


def load_package(root):
    path = root / PACKAGE_FOLDER
    report = path / "report.md"
    run_log = path / "run.log"
    if not report.is_file() or not run_log.is_file():
        return None
    text = report.read_text(encoding="utf-8")
    log = run_log.read_text(encoding="utf-8")
    sha = re.search(r"\b([0-9a-f]{40})\b", text)
    suite = path / "package-suite.log"
    suite_line = ""
    if suite.is_file():
        found = re.search(r"test result: .*", suite.read_text(encoding="utf-8"))
        suite_line = found.group(0) if found else ""
    gate_pass = "PACKAGE_VALIDATION_PASS" in log and re.search(r"^exit=0\s*$", log, re.M)
    checks = re.search(r"after (\d+) named checks", text)
    return {"path": PACKAGE_FOLDER, "status": "PASS" if gate_pass else "FAIL",
            "code_sha": sha.group(1) if sha else "", "checks": checks.group(1) if checks else "",
            "cargo_test": suite_line}


def load_captures(root):
    captures = []
    for folder, (harness, version, summary) in CAPTURE_FOLDERS.items():
        report = root / folder / "report.md"
        present = report.is_file() and version in report.read_text(encoding="utf-8")
        captures.append({"folder": folder, "harness": harness, "version": version,
                         "summary": summary, "status": "PRESENT" if present else "MISSING"})
    return captures


# ---------------------------------------------------------------- git drift

CODE_PATHS = ["src", "Cargo.toml", "Cargo.lock", "herdr-plugin.toml"]


def code_drift(sha, base, repo):
    if not sha:
        return None
    try:
        out = subprocess.run(["git", "-C", str(repo), "diff", "--name-only", sha, base, "--", *CODE_PATHS],
                             capture_output=True, text=True, timeout=30, check=True).stdout
    except (OSError, subprocess.SubprocessError):
        return None
    return len([line for line in out.splitlines() if line.strip()])


def resolve_base(base, repo):
    try:
        return subprocess.run(["git", "-C", str(repo), "rev-parse", base], capture_output=True,
                              text=True, timeout=10, check=True).stdout.strip()
    except (OSError, subprocess.SubprocessError):
        return None


# ---------------------------------------------------------------- report

def short(sha):
    return sha[:8] if sha else "n/a"


def render(result):
    runs, matrix = result["runs"], result["matrix"]
    host, package, captures = result["host"], result["package"], result["captures"]
    status, failures, gaps = result["overall"]
    drift = result["drift"]
    base = result["base"]
    ev = RUN_REL
    out = []
    w = out.append
    w("# herdr-threads validation report")
    w("")
    w("Generated by `scripts/reconcile-validation.py` from committed evidence under "
      f"`{ev}/`, archived in git tag `{ARCHIVE_TAG}` (path unchanged there). Evidence paths "
      "below are relative to that folder; the harness captures and the Codex write probe are also "
      f"kept in-tree under [`docs/evidence/`]({IN_TREE_EVIDENCE}/). Regenerate with "
      "`python3 scripts/reconcile-validation.py --evidence-root <archive>/" + ev + "` "
      "(see the script's usage). Do not edit by hand.")
    w("")
    w(f"**Report verdict: {status}.**")
    w("")
    if failures:
        w("Failing items:")
        w("")
        for item in failures:
            w(f"- {item}")
        w("")
    if gaps:
        w("Open gaps (owed, not exercised or unverified; never counted as PASS):")
        w("")
        for item in gaps:
            w(f"- {item}")
        w("")
    w("## Stamp")
    w("")
    w(f"- Report base: `{short(base) if base else 'unknown'}`. Each evidence row below carries the "
      "code SHA it was produced on. **Any later code change invalidates that row's stamp** and "
      "requires the relevant verification again; the `src drift` column counts files under "
      f"`{', '.join(CODE_PATHS)}` changed between the evidence SHA and the report base.")
    stale = sorted(sha for sha, n in drift.items() if n)
    if drift:
        w(f"- {len(stale)} of {len(drift)} evidence SHAs have code drift against the report base; their "
          "rows describe the code at the listed SHA, not the report base. Rerun the affected scenarios "
          "before claiming them for a later SHA.")
    else:
        w("- Code drift was not measured (`--no-git` or git unavailable); treat every stamp as unchecked.")
    w("- Versions observed: Claude Code 2.1.285 and 2.1.286 (doctor recipe "
      "`claude-hooks-2.1.283` [2.1.283, 2.1.286]); Codex 0.159.2, admitted by doctor as "
      "**schema-matched, live-unverified** under `codex-hooks-v1` (the dedicated live hook "
      "capture is 0.158.0; the native runs below ran 0.159.2); Herdr 0.9.1 (protocol 22); "
      f"macOS ({redact(host['platform']) if host else 'Darwin arm64'}).")
    w("- Model receipts are `cooperative_top_level` provenance throughout: a cooperative claim "
      "joined to a transcript root call, not kernel-verified native proof.")
    w("")
    if result.get("rerun"):
        out.extend(render_rerun(result["rerun"]))
        w("The sections below are the archived 2026-09-26 evidence. Where a rerun cell above covers the "
          "same scenario, the rerun cell is the current evidence and settles the matching open gap.")
        w("")
    w("## Evidence kinds")
    w("")
    w("| Kind | What it is | Sources |")
    w("|---|---|---|")
    w("| Deterministic | No model. `cargo test` suites and the isolated real-Herdr host suite whose panes "
      "run stand-in `claude` scripts; stand-in seat actions are not model receipts. | "
      f"`{HOST_FOLDER}/`, `{PACKAGE_FOLDER}/` |")
    w("| Native (live model) | A real Claude/Codex session in an owned Herdr pane; ACK counted only when a "
      "transcript root call joins a read-only SQLite receipt. | `native-*/` live runs |")
    w("| Driver dry-run | Same driver with captured hook payloads, no model; manifests are `synthetic` / "
      "`UNSUPPORTED dry_run_no_model` and can never PASS. | `native-*/dry*` |")
    w("| Diagnostic | Patched-driver investigation runs; excluded from verdicts. | `*diag*` |")
    w("| Harness capture | Hook payload shapes and output application for one harness version; not "
      "receipt evidence. | " + ", ".join(f"`{c['folder']}/`" for c in captures) + " |")
    w("")
    w("## Scenario x harness matrix (native, live)")
    w("")
    w("A cell's verdict comes from the most recent live run that exercised it; earlier attempts are "
      "listed under the matrix. Within a run the worst deciding step wins (BLOCKED counts as FAIL); a "
      "deciding step absent from that run makes an otherwise passing cell UNVERIFIED. Verdicts: PASS, "
      "FAIL, NOT_EXERCISED, UNVERIFIED, UNSUPPORTED, NO_EVIDENCE. Only PASS is a pass. A required row "
      "without PASS on both harnesses fails the report; `gap` rows list known owed items.")
    w("")
    w("| Scenario | Req | Required | Claude | Codex |")
    w("|---|---|---|---|---|")
    for entry in matrix:
        row = entry["row"]
        cols = []
        for harness in HARNESSES:
            cell = entry["cells"][harness]
            if cell["deciding"]:
                run = cell["deciding"]["run"]
                cols.append(f"**{cell['verdict']}** (`{run['path']}/`)")
            else:
                cols.append(f"**{cell['verdict']}**")
        req = "yes" if row.get("required") else "gap"
        w(f"| {row['title']} | {row['req']} | {req} | {cols[0]} | {cols[1]} |")
    w("")
    w("### Per-step verdicts of the deciding runs")
    w("")
    w("| Scenario | Harness | Run | Code SHA | Steps |")
    w("|---|---|---|---|---|")
    for entry in matrix:
        row = entry["row"]
        for harness in HARNESSES:
            cell = entry["cells"][harness]
            if not cell["deciding"]:
                w(f"| {row['id']} | {harness} | none | n/a | no live run exercised this |")
                continue
            run = cell["deciding"]["run"]
            steps = ", ".join(f"{k} {v}" for k, v in cell["deciding"]["steps"].items())
            w(f"| {row['id']} | {harness} | `{run['folder']}/{run['name']}` | `{short(run['code_sha'])}` | {steps} |")
    w("")
    w("### Superseded attempts")
    w("")
    w("Earlier live attempts stay on record. A later pass on newer code supersedes them; it does not "
      "turn the earlier failure into a pass.")
    w("")
    w("| Scenario | Harness | Run | Code SHA | Verdict |")
    w("|---|---|---|---|---|")
    any_superseded = False
    for entry in matrix:
        for harness in HARNESSES:
            for attempt in entry["cells"][harness]["attempts"][:-1]:
                run = attempt["run"]
                any_superseded = True
                w(f"| {entry['row']['id']} | {harness} | `{run['folder']}/{run['name']}` | "
                  f"`{short(run['code_sha'])}` | {attempt['verdict']} |")
    if not any_superseded:
        w("| none | | | | |")
    w("")
    w("### Scenario notes")
    w("")
    for entry in matrix:
        if entry["row"].get("note"):
            w(f"- **{entry['row']['id']}:** {entry['row']['note']}")
    w("")
    w("## Receipt accounting (native runs)")
    w("")
    w("Per manifest: accepted message-recipient pairs, ACKed (model call joined to a unique SQLite receipt "
      "on call/message/recipient/actor), pending, retired, duplicate logical pairs, duplicate receipt rows, "
      "and transport hints (total / repeated) counted separately. `SQLite final` is the final receipt "
      "state across the run's read-only snapshots (it also covers scenario messages outside the "
      "manifest, such as warning, mid-turn or blocked-UI sends). Manifests record phase handoffs; the "
      "driver records no transport hints in them, so hint counts are 0 by construction and wake prompts "
      "appear only in step evidence (SL2, SW0, SU2).")
    w("")
    w("| Run | Kind | Harness | Mode | Code SHA | Version | Manifest | Accepted | ACKed | Pending | Retired | DB-acked unjoined | Dup pairs | Dup rows | Hints (rep.) | SQLite final | Src drift |")
    w("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|")
    totals = {h: {"accepted": 0, "acked": 0, "pending": 0, "retired": 0, "db_acked_unjoined": 0,
                  "duplicate_logical_pairs": 0,
                  "duplicate_receipt_rows": 0, "transport_hints": 0, "repeated_transport_hints": 0}
              for h in HARNESSES}
    for run in runs:
        if run.get("missing"):
            w(f"| `{run['folder']}` | MISSING | | | | | | | | | | | | | | | |")
            continue
        acc = run["accounting"]
        if run["kind"] == "live" and run["harness"] in totals:
            for key in totals[run["harness"]]:
                totals[run["harness"]][key] += acc[key]
        manifest = run["manifest_status"] + (f" `{run['manifest_reason']}`" if run["manifest_reason"] else "")
        sqlite = ", ".join(f"{k} {v}" for k, v in sorted(run["sqlite_counts"].items())) or "n/a"
        if run["sqlite_mismatch"]:
            sqlite += f" (MISMATCH {len(run['sqlite_mismatch'])})"
        d = drift.get(run["code_sha"])
        w(f"| `{run['path']}/` | {run['kind']} | {run['harness']} | "
          f"{run['mode']} | `{short(run['code_sha'])}` | {run['harness_version']} | {manifest} | "
          f"{acc['accepted']} | {acc['acked']} | {acc['pending']} | {acc['retired']} | "
          f"{acc['db_acked_unjoined']} | {acc['duplicate_logical_pairs']} | {acc['duplicate_receipt_rows']} | "
          f"{acc['transport_hints']} ({acc['repeated_transport_hints']}) | {sqlite} | "
          f"{'n/a' if d is None else d} |")
    w("")
    w("Live-run totals (dry-run and diagnostic excluded):")
    w("")
    w("| Harness | Accepted | ACKed | Pending | Retired | DB-acked unjoined | Dup pairs | Dup rows | Hints | Repeated hints |")
    w("|---|---|---|---|---|---|---|---|---|---|")
    for harness, t in totals.items():
        w(f"| {harness} | {t['accepted']} | {t['acked']} | {t['pending']} | {t['retired']} | {t['db_acked_unjoined']} | "
          f"{t['duplicate_logical_pairs']} | {t['duplicate_receipt_rows']} | {t['transport_hints']} | "
          f"{t['repeated_transport_hints']} |")
    w("")
    pending_runs = [r for r in runs if not r.get("missing") and r["kind"] == "live" and r["accounting"]["pending"]]
    if pending_runs:
        w("Live runs with pending pairs (each stays pending and fails or leaves unverified that run; no "
          "coordinator ACK or receipt edit finished any scenario):")
        w("")
        for r in pending_runs:
            a = r["accounting"]
            extra = (f"; {a['db_acked_unjoined']} acked in SQLite without a joined model call "
                     "(issuance unverified, not counted)") if a["db_acked_unjoined"] else ""
            w(f"- `{r['folder']}/{r['name']}`: manifest {r['manifest_status']}, {a['pending']} pending{extra}.")
    else:
        w("No live run has a pending pair.")
    w("")
    w("## Non-pass steps in live runs")
    w("")
    w("| Run | Step | Status | Detail (redacted, truncated) |")
    w("|---|---|---|---|")
    for run in runs:
        if run.get("missing") or run["kind"] != "live":
            continue
        for step_id, entries in run["steps"].items():
            for e in entries:
                if e["status"] in NON_PASS:
                    detail = redact(e.get("detail", "")).replace("|", "\\|").replace("\n", " ")[:160]
                    w(f"| `{run['folder']}/{run['name']}` | {step_id} | {e['status']} | {detail} |")
    w("")
    w("## Deterministic evidence")
    w("")
    if host:
        w(f"### Isolated host recovery ([`{HOST_FOLDER}/`]({IN_TREE_EVIDENCE}/{HOST_FOLDER}/report.md))")
        w("")
        w(f"- Status **{host['status']}**, counts {host['counts']}; code `{short(host['code_sha'])}`, "
          f"binary sha256 `{short(host['binary_sha256'])}`, Herdr {host['herdr']}, src drift "
          f"{drift.get(host['code_sha'], 'n/a')}.")
        w("- Private Herdr server and test daemon only; stand-in agents, no model. The shared server was "
          "never contacted (R12).")
        w("- This tip run predates host scenario R13 (cooperative idle wake: one prompt for the overdue "
          "warning), which was added with the production safe wake. R13's real-host result (13/13 PASS on the safe-wake lane "
          "build 1256878) is committed in `host-recovery-validation-safe-wake/` (results.json, report.md).")
        w("")
        w("| Scenario | Status | Checks |")
        w("|---|---|---|")
        for sc in host["scenarios"]:
            w(f"| {sc['id']} | {sc['status']} | {sc['checks']} |")
        w("")
    else:
        w("- Host recovery evidence: MISSING (report FAIL).")
        w("")
    if package:
        w(f"### Package lifecycle ([`{PACKAGE_FOLDER}/`]({IN_TREE_EVIDENCE}/{PACKAGE_FOLDER}/report.md))")
        w("")
        w(f"- Gate status **{package['status']}** ({package['checks']} named checks), code "
          f"`{short(package['code_sha'])}`, src drift {drift.get(package['code_sha'], 'n/a')}.")
        w(f"- `cargo test --locked --all-features --test package`: `{package['cargo_test']}`.")
        w("")
    else:
        w("- Package evidence: MISSING (report FAIL).")
        w("")
    w("The composed-service fault matrix, scheduler, store and retirement regression suites are `cargo "
      "test` targets in this repository; this report does not re-run them and has no committed result "
      "manifest for them. Their pass state must be taken from a `cargo test` run at the stamped SHA.")
    w("")
    w("## Harness captures")
    w("")
    w("| Folder | Harness | Version | Status | Finding |")
    w("|---|---|---|---|---|")
    for c in captures:
        w(f"| [`{c['folder']}/`]({IN_TREE_EVIDENCE}/{c['folder']}/) | {c['harness']} | {c['version']} | "
          f"{c['status']} | {c['summary']} |")
    w("")
    w("## Honest gaps and accepted limits")
    w("")
    w("- **Concurrent children: NOT_EXERCISED on both harnesses.** Claude ran two subagents one after "
      "the other; Codex started none in the TUI children run. Child read/ACK absence is covered only for "
      "single (sequential) children.")
    w("- **SW2 warning-only coalesced wake: not separately owed live.** In every warning run the warning "
      "was already offered at a verified check-in, so no separate wake was due. It is covered by the "
      "deterministic host stand-in scenario R13, not by a live model; R13's real-host result (PASS, one prompt "
      "for the overdue warning, none within the spacing window) is committed in `host-recovery-validation-safe-wake/`.")
    w("- **DB-level child separation is the accepted cooperative B1 limit.** A child write would be "
      "stored with the root's seat, provenance and execution; child ACK absence is proven only at the "
      "transcript (sidechain) level.")
    w("- **The Codex sandbox write gap was missed by the native runs (ht-4is.8.20).** Every Codex native run "
      "used a state directory under `/private/tmp`, which workspace-write treats as writable. So no run "
      "exercised a real install, where the state directory is `~/.local/state/...` and every sandboxed "
      "mutation and check-in fails with `EPERM` writing its intent and context journals. Live tea party take "
      "8 found it. The fix adds the two client-journal directories to `writable_roots`, and the no-model "
      f"[write probe]({IN_TREE_EVIDENCE}/codex-sandbox-writes-probe/README.md) on 0.159.3 measures it with "
      "tmp excluded from the sandbox. Before the fix, send, check-in and accept fail with `EPERM`. After it, "
      "check-in, send, ACK, accept, leave and invite succeed, and the database, daemon files and state "
      "directory stay unwritable. No live model run has repeated this with a non-tmp state directory.")
    w("- **Codex 0.159.2 is admitted by schema fingerprint** (`schema-matched, live-unverified`). The "
      "dedicated live hook capture is for 0.158.0; the 0.159.2 native runs observed SessionStart "
      "context delivery but no dedicated 0.159.2 capture exists.")
    w("- **Token overhead is not compared.** There are no matched workloads; per-run usage is recorded "
      "in each run's `summary.json` but is not a controlled benchmark, and the earlier hcom/hmail totals "
      "are not comparable.")
    w("- **Claude managed launch** was last exercised on Claude 2.1.285 (`native-claude-matrix-1`); "
      "2.1.286 reruns covered the manual configurations.")
    w("- Raw native evidence (full transcripts, SQLite databases, `~/.claude.json` snapshots) stays "
      "private; committed folders hold redacted extracts. Home paths appear as `~`.")
    w("")
    return "\n".join(out) + "\n"


# ---------------------------------------------------------------- native rerun (ht-p03.20)

# The native matrix rerun of root spec section B8 Decision 3: `validate-native-demo.sh --matrix` writes
# ATTEMPT/MATRIX lines to $HT_MATRIX_LOG; `--collect-native-rerun LOG --write JSON` turns them (plus each
# final attempt's run-root manifest, summary and hook-context evidence) into a committed results file;
# `--native-rerun JSON` renders it into the report and lets its cells settle the matching gap rows.
RERUN_CELL_HARNESS = {
    "codex-manual": "codex", "codex-managed": "codex", "claude-manual": "claude", "claude-managed": "claude",
    "codex-no-initial-prompt": "codex", "claude-no-initial-prompt": "claude", "children-claude": "claude",
    "children-codex": "codex", "sw2-claude": "claude", "sw2-codex": "codex",
    "codex-tui-children-write-absence": "codex", "ht910-claude": "claude", "ht910-codex": "codex",
    "p40-crash-fix3": "codex", "codex-sandbox-xdg-state": "codex", "wave28-claude-wake-submission": "claude",
}
# Closures that depend on the rerun and their backing cells (root spec B8 Decision 3; findings-closure.md).
RERUN_CLOSURES = [
    ("R20 MET (W9-1)", ["sw2-claude", "sw2-codex", "wave28-claude-wake-submission"]),
    ("Wave 28 (wake left typed but unsent)", ["wave28-claude-wake-submission"]),
    ("B6 current-version evidence, Claude Code Listed row", ["claude-manual", "claude-managed"]),
    ("B6 current-version evidence, Codex Listed row", ["codex-manual", "codex-managed"]),
    ("B6 sandbox writes (non-tmp state dir)", ["codex-sandbox-xdg-state"]),
    ("ht-910 daemon restart, agent stays in its pane", ["ht910-claude", "ht910-codex"]),
    ("P40 crash-fix3 real-host acceptance", ["p40-crash-fix3"]),
    ("B3 concurrent children", ["children-claude", "children-codex"]),
    ("B3 SW2 coalesced warning wake", ["sw2-claude", "sw2-codex"]),
    ("B3 Codex TUI children write-absence", ["codex-tui-children-write-absence"]),
    ("R23 sweep status (crash/restart cells)", ["p40-crash-fix3", "ht910-claude", "ht910-codex"]),
    ("G0/B2 (every cell)", list(RERUN_CELL_HARNESS)),
]
# Archive gap rows (ROWS) a rerun cell settles: (row id, harness) -> cell.
RERUN_GAP_CELLS = {
    ("coalesced-warning-wake", "claude"): "sw2-claude", ("coalesced-warning-wake", "codex"): "sw2-codex",
    ("concurrent-children", "claude"): "children-claude", ("concurrent-children", "codex"): "children-codex",
    ("children-write-absence", "codex"): "codex-tui-children-write-absence",
}
_KV = re.compile(r"(\w+)=(\[[^\]]*\]|\S+)")


def _cell_fields(text):
    return {k: v.strip("[]") for k, v in _KV.findall(text)}


def rerun_passes(outcome):
    return outcome in ("PASS", "PASS (flaky)")


def _context_bytes(evidence):
    """Injected additionalContext bytes of the first turn (`initial` phase): SessionStart plus PreToolUse."""
    path = pathlib.Path(evidence) / "hook-context-initial.json"
    if not path.is_file():
        return None
    deliveries = load_json(path).get("deliveries", [])
    root = [d for d in deliveries if not d.get("sidechain")]
    return {"turn_bytes": sum(d.get("bytes", 0) for d in root), "deliveries": len(root),
            "session_start_bytes": sum(d.get("bytes", 0) for d in root if d.get("hook_event") == "SessionStart"),
            "pre_tool_use_bytes": sum(d.get("bytes", 0) for d in root if d.get("hook_event") == "PreToolUse")}


def collect_native_rerun(log_text, read_evidence=True):
    """Matrix log (ATTEMPT/MATRIX lines of one full-matrix run) -> results document."""
    attempts, cells = {}, []
    for line in log_text.splitlines():
        m = re.match(r"ATTEMPT (\d+) CELL (\S+) (.*)$", line)
        if m:
            fields = _cell_fields(m.group(3))
            attempts.setdefault(m.group(2), []).append({
                "attempt": int(m.group(1)), "sha": fields.get("sha"), "manifest_status": fields.get("manifest_status"),
                "harness_before": fields.get("harness_before"), "harness_after": fields.get("harness_after"),
                "harness_ran": fields.get("harness_ran"), "evidence": fields.get("evidence")})
            continue
        m = re.match(r"MATRIX cell=(\S+) attempts=(\d+) outcome=(.*?) :: ?(.*)$", line)
        if not m:
            continue
        name, outcome, tail = m.group(1), m.group(3).strip(), m.group(4)
        fields = _cell_fields(tail.split(" ", 2)[2] if tail.startswith("CELL ") else tail)
        verdict = outcome.split(" (")[0]
        if verdict.startswith("INVALID"):
            verdict = "NOT_EXERCISED"  # a version-moved attempt is never evidence
        if outcome == "PASS (flaky)":
            verdict = outcome
        cell = {"cell": name, "harness": RERUN_CELL_HARNESS.get(name, "?"), "attempts": int(m.group(2)),
                "outcome": verdict, "outcome_detail": outcome, "sha": fields.get("sha"),
                "harness_before": fields.get("harness_before"), "harness_after": fields.get("harness_after"),
                "harness_ran": fields.get("harness_ran"), "manifest_status": fields.get("manifest_status"), "reason": "", "steps": {},
                "evidence": fields.get("evidence"), "attempt_history": attempts.get(name, [])}
        evidence = fields.get("evidence")
        if read_evidence and evidence and evidence != "none" and pathlib.Path(evidence).is_dir():
            ev = pathlib.Path(evidence)
            if (ev / "manifest.json").is_file():
                cell["reason"] = load_json(ev / "manifest.json").get("reason", "")
            if (ev / "summary.json").is_file():
                summary = load_json(ev / "summary.json")
                cell["steps"] = {s["step"]: s["status"] for s in summary.get("steps", [])
                                 if s.get("status") not in ("INFO",)}
                bad = [f"{s['step']} {s['status']}: {s.get('detail', '')[:160]}" for s in summary.get("steps", [])
                       if s.get("status") in ("FAIL", "BLOCKED", "NOT_EXERCISED", "UNVERIFIED")]
                cell["non_pass_steps"] = bad
            cell["context"] = _context_bytes(ev)
            cell["evidence"] = ev.parent.name
        elif evidence and evidence != "none":
            cell["evidence"] = pathlib.Path(evidence).parent.name
        cells.append(cell)
    shas = sorted({c["sha"] for c in cells if c["sha"]})
    return {"cells": cells, "shas": shas, "evidence_sha": shas[0] if len(shas) == 1 else None}


def rerun_closures(rerun):
    by = {c["cell"]: c for c in rerun["cells"]}
    sha = rerun.get("evidence_sha")
    out = []
    for name, backing in RERUN_CLOSURES:
        missing = [c for c in backing if c not in by]
        bad = [f"{c}: {by[c]['outcome']}" for c in backing if c in by and not rerun_passes(by[c]["outcome"])]
        stale = [c for c in backing if c in by and by[c]["sha"] != sha]
        open_ = missing or bad or stale or not sha
        why = "; ".join(bad + [f"{c}: not run" for c in missing] + [f"{c}: stale" for c in stale])
        out.append({"closure": name, "backing": backing, "closed": not open_, "why": why})
    return out


def apply_rerun(result, rerun):
    """Rerun cells settle the archive gap rows they back; a non-PASS rerun cell keeps the gap open."""
    status, failures, gaps = result["overall"]
    by = {c["cell"]: c for c in rerun["cells"]}
    kept = []
    for label in gaps:
        row_id, rest = label.split(" / ", 1)
        cell = by.get(RERUN_GAP_CELLS.get((row_id, rest.split(":")[0])))
        if cell is None:
            kept.append(label)
        elif not rerun_passes(cell["outcome"]):
            kept.append(f"{row_id} / {rest.split(':')[0]}: {cell['outcome']} in the ht-p03.20 rerun ({cell['cell']})")
    failures = list(failures) + [f"rerun {c['cell']}: FAIL" for c in rerun["cells"] if c["outcome"] == "FAIL"]
    if not rerun.get("evidence_sha"):
        failures.append(f"rerun cells span {len(rerun['shas'])} SHAs; no single evidence SHA")
    result["overall"] = ("FAIL" if failures else "PASS_WITH_GAPS" if kept else "PASS"), failures, kept
    result["rerun"] = rerun


def render_rerun(rerun):
    out = []
    w = out.append
    sha = rerun.get("evidence_sha")
    w("## Native matrix rerun on the evidence SHA (ht-p03.20)")
    w("")
    w(f"Evidence SHA: `{sha or 'none (cells span several SHAs)'}`. Every cell below ran on it (root spec B8 "
      "Decision 3: one final SHA; later commits touch only evidence paths, checked with "
      f"`git diff --name-only {short(sha) if sha else '<sha>'}..HEAD`). Results file: "
      f"`{rerun.get('source', 'native-rerun.json')}`. Each cell ran once per attempt in its own isolated named "
      "Herdr session (`scripts/validate-native-demo.sh --matrix`), with the default Codex profile, setup into "
      "run-root copies of HOME, CLAUDE_CONFIG_DIR and CODEX_HOME, Claude from the fixed versioned binary "
      "`~/.local/share/claude/versions/<v>` on a run-root PATH entry and Codex from its fixed install path; "
      "`--version` was taken immediately before and after every cell, and the version the agent itself reported (Claude's transcript `claude_code_version` or TUI banner, Codex's rollout `cli_version` or TUI banner) had to equal it. Up to three attempts per cell on the "
      "same SHA; a PASS after a failed attempt is `PASS (flaky)`. NOT_EXERCISED is never PASS.")
    if rerun.get("full_matrix_runs"):
        w("")
        w(f"Full-matrix runs: {rerun['full_matrix_runs']}.")
    w("")
    w("| Cell | Harness | SHA | Version before | Version after | Version the agent reported | Attempts | Outcome | "
      "Reason / non-pass steps | Run |")
    w("|---|---|---|---|---|---|---|---|---|---|")
    for c in rerun["cells"]:
        notes = c.get("reason") or ""
        if not rerun_passes(c["outcome"]) and c.get("non_pass_steps"):
            notes = "; ".join(filter(None, [notes] + c["non_pass_steps"][:2]))
        if c.get("outcome_detail") and c["outcome_detail"] != c["outcome"]:
            notes = "; ".join(filter(None, [c["outcome_detail"], notes]))
        notes = notes.replace("|", "/").replace("\n", " ")
        w(f"| {c['cell']} | {c['harness']} | `{short(c['sha']) if c['sha'] else '-'}` | {c['harness_before'] or '-'} | "
          f"{c['harness_after'] or '-'} | {c.get('harness_ran') or '-'} | {c['attempts']} | **{c['outcome']}** | {notes or '-'} | "
          f"`{c.get('evidence') or '-'}` |")
    w("")
    versions = {}
    for c in rerun["cells"]:
        if c.get("harness_before"):
            versions.setdefault(c["harness"], set()).add(c["harness_before"])
    w("Harness versions across the matrix: " + "; ".join(
        f"{h}: {', '.join(sorted(v))}" for h, v in sorted(versions.items())) + " (one per harness required).")
    w("")
    w("### Per-turn injected context (measured, not a benchmark)")
    w("")
    w("Hook `additionalContext` bytes the harness recorded for the first turn of the manual core-flow cell "
      "(root session, SessionStart plus PreToolUse; Claude from the session transcript attachments, Codex from "
      "the rollout's herdr-threads developer messages). One measured number per harness; the token-overhead "
      "benchmark stays deferred.")
    w("")
    w("| Harness | Cell | Bytes injected in the turn | Deliveries | SessionStart bytes | PreToolUse bytes |")
    w("|---|---|---|---|---|---|")
    for name in ("claude-manual", "codex-manual"):
        c = next((x for x in rerun["cells"] if x["cell"] == name), None)
        ctx = c and c.get("context")
        if ctx:
            w(f"| {c['harness']} | {name} | {ctx['turn_bytes']} | {ctx['deliveries']} | "
              f"{ctx['session_start_bytes']} | {ctx['pre_tool_use_bytes']} |")
        else:
            w(f"| {RERUN_CELL_HARNESS[name]} | {name} | NOT_EXERCISED (no hook-context evidence) | - | - | - |")
    w("")
    w("### Rerun-dependent closures")
    w("")
    w("Each holds only if every backing cell PASSes on the evidence SHA; a FAIL, NOT_EXERCISED or stale "
      "backing cell leaves it open (root spec B8 Decision 3).")
    w("")
    w("| Closure | Backing cells | Status |")
    w("|---|---|---|")
    for item in rerun_closures(rerun):
        backing = ", ".join(item["backing"]) if len(item["backing"]) < 6 else "every cell above"
        status = "**closed**" if item["closed"] else f"**open**: {item['why']}"
        w(f"| {item['closure']} | {backing} | {status} |")
    w("")
    return out

def reconcile_all(root, base=None, repo=None, use_git=True):
    runs = discover_native(root, NATIVE_FOLDERS)
    matrix = build_matrix(runs)
    host = load_host(root)
    package = load_package(root)
    captures = load_captures(root)
    resolved = resolve_base(base, repo) if (use_git and base) else None
    drift = {}
    if use_git and resolved:
        shas = {r["code_sha"] for r in runs if not r.get("missing")}
        shas |= {x["code_sha"] for x in (host, package) if x}
        for sha in sorted(s for s in shas if s):
            drift[sha] = code_drift(sha, resolved, repo)
    result = {"runs": runs, "matrix": matrix, "host": host, "package": package,
              "captures": captures, "drift": drift, "base": resolved}
    result["overall"] = overall(matrix, runs, host, package)
    return result


def to_json(result):
    def cell(c):
        d = c["deciding"]
        return {"verdict": c["verdict"],
                "deciding": d and {"run": d["run"]["path"], "steps": d["steps"]},
                "attempts": [{"run": a["run"]["path"], "verdict": a["verdict"]} for a in c["attempts"]]}
    status, failures, gaps = result["overall"]
    return {
        "overall": status, "failures": failures, "gaps": gaps, "base": result["base"],
        "matrix": [{"row": e["row"]["id"], "required": e["row"].get("required", False),
                    "cells": {h: cell(c) for h, c in e["cells"].items()}} for e in result["matrix"]],
        "runs": [{k: v for k, v in r.items() if k != "steps"} for r in result["runs"]],
        "host": result["host"], "package": result["package"], "captures": result["captures"],
        "rerun_closures": rerun_closures(result["rerun"]) if result.get("rerun") else None,
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--evidence-root", default=os.environ.get(EVIDENCE_ENV) or str(REPO / RUN_REL),
                        help=f"run evidence folder (default: ${EVIDENCE_ENV}, else {RUN_REL} in this "
                             f"checkout, present only in a checkout of tag {ARCHIVE_TAG})")
    parser.add_argument("--output", default=str(REPO / DEFAULT_OUTPUT))
    parser.add_argument("--json", action="store_true", help="print the reconciliation as JSON instead")
    parser.add_argument("--base", default="99ea74c", help="code revision the stamp is measured against")
    parser.add_argument("--no-git", action="store_true", help="skip code-drift measurement")
    parser.add_argument("--native-rerun", metavar="JSON",
                        help="ht-p03.20 rerun results (from --collect-native-rerun): render them and let their "
                             "cells settle the matching gap rows")
    parser.add_argument("--collect-native-rerun", metavar="LOG",
                        help="read a `validate-native-demo.sh --matrix` log ($HT_MATRIX_LOG) and its run-root "
                             "evidence, write the results JSON to --write, and exit")
    parser.add_argument("--write", metavar="JSON", help="output path for --collect-native-rerun")
    args = parser.parse_args(argv)
    if args.collect_native_rerun:
        if not args.write:
            parser.error("--collect-native-rerun needs --write JSON")
        rerun = collect_native_rerun(pathlib.Path(args.collect_native_rerun).read_text(encoding="utf-8"))
        pathlib.Path(args.write).write_text(redact(json.dumps(rerun, indent=2, sort_keys=True)) + "\n", encoding="utf-8")
        print(f"collected {len(rerun['cells'])} cells, evidence SHA {rerun['evidence_sha']}: wrote {args.write}")
        return 0
    root = pathlib.Path(args.evidence_root).resolve()
    if not root.is_dir():
        parser.error(f"evidence root {root} not found; the run evidence is archived in git tag "
                     f"{ARCHIVE_TAG} (see usage: extract it and pass --evidence-root or set {EVIDENCE_ENV})")
    result = reconcile_all(root, base=args.base, repo=REPO, use_git=not args.no_git)
    if args.native_rerun:
        rerun = load_json(args.native_rerun)
        rerun.setdefault("source", os.path.relpath(pathlib.Path(args.native_rerun).resolve(), REPO))
        apply_rerun(result, rerun)
    if args.json:
        print(redact(json.dumps(to_json(result), indent=2, sort_keys=True, default=list)))
        return 0
    text = redact(render(result))
    pathlib.Path(args.output).parent.mkdir(parents=True, exist_ok=True)
    pathlib.Path(args.output).write_text(text, encoding="utf-8")
    status = result["overall"][0]
    print(f"{status}: wrote {os.path.relpath(args.output)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
