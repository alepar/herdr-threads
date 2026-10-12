#!/usr/bin/env python3
"""Parent-only V090 sealed exact-matrix enumeration and optional execution.

Preparation never invokes Cargo. Requires Python 3.11+, a fresh absolute evidence
directory, and an inventory refreshed to the final committed clean C source.
Default: enumerate only. --run: focused nextest, then six mandatory Cargo cases.
The frozen f330391 snapshot and explicit source-evidenced selection seal retain
all requirements. PREPARATION_ONLY metadata rejects Cargo before enumeration.
Only the parent may seal a clean committed successor after causal fix/review.
"""
from __future__ import annotations

import argparse
from collections import Counter
from contextlib import contextmanager
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import selectors
import signal
import subprocess
import sys
import time
import tomllib
import uuid

ALLOWED = {
    "herdr-threads": ("lib", "herdr_threads"),
    "herdr-threads::combined": ("test", "combined"),
    "herdr-threads::hook_entrypoint": ("test", "hook_entrypoint"),
    "herdr-threads::integration": ("test", "integration"),
    "herdr-threads::service": ("test", "service"),
}
TARGET_ARGS = [
    "--lib", "--test", "combined", "--test", "hook_entrypoint",
    "--test", "integration", "--test", "service",
]
BASELINE_SHA256 = "d568da428d23aaa0b517d0ab6a656e0f1684634fdf18f4281761b12a35546fad"
LOG_LIMIT = 32 * 1024 * 1024
LIST_LIMIT = 64 * 1024 * 1024
RECORD_LIMIT = 8 * 1024 * 1024


class TerminationRequested(BaseException):
    def __init__(self, signum: int):
        self.signum = signum
        super().__init__(f"termination signal {signum}")


class Cancellation:
    """Catchable termination enters owned cleanup; repeat signals never skip it."""
    def __init__(self):
        self.signum = None
        self.depth = 0

    def handle(self, signum, _frame):
        first = self.signum is None
        if first:
            self.signum = signum
        if first and not self.depth:
            raise TerminationRequested(signum)

    @contextmanager
    def defer(self, raise_after=True):
        self.depth += 1
        try:
            yield
        finally:
            self.depth -= 1
            if (
                raise_after and not self.depth and self.signum is not None
                and sys.exc_info()[0] is None
            ):
                raise TerminationRequested(self.signum)


def terminate_owned_group(child, grace_seconds=5.0) -> None:
    """Only a Popen created with start_new_session=True by this runner."""
    pgid = child.pid
    try:
        os.killpg(pgid, signal.SIGTERM)
    except ProcessLookupError:
        if child.poll() is None:
            child.wait(timeout=grace_seconds)
        return
    deadline = time.monotonic() + grace_seconds
    # Reap the leader, but keep checking its owned group: descendants can
    # outlive Cargo, and a leader exit alone does not prove group shutdown.
    while True:
        if child.poll() is None:
            try:
                child.wait(timeout=max(0.001, min(0.05, deadline - time.monotonic())))
            except subprocess.TimeoutExpired:
                pass
        try:
            os.killpg(pgid, 0)
        except ProcessLookupError:
            break
        if time.monotonic() >= deadline:
            try:
                os.killpg(pgid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            break
        time.sleep(0.01)
    if child.poll() is None:
        child.wait()


def mandatory_argv(row: dict) -> list[str]:
    if row["binary_id"] != "herdr-threads::combined":
        raise ValueError("mandatory target must be combined")
    return [
        "nice", "cargo", "test", "--locked", "--all-features",
        "--test", "combined", row["name"], "--", "--exact",
    ]


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def save(path: Path, value: object) -> None:
    encoded = (json.dumps(value, indent=2) + "\n").encode()
    if len(encoded) > RECORD_LIMIT:
        raise ValueError(f"record exceeds {RECORD_LIMIT} bytes: {path.name}")
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_bytes(encoded)
    temporary.replace(path)


def git(source: Path, *arguments: str) -> bytes:
    return subprocess.check_output(["git", *arguments], cwd=source)


def text_git(source: Path, *arguments: str) -> str:
    return git(source, *arguments).decode().strip()


def pair(row: dict) -> tuple[str, str]:
    return row["binary_id"], row["name"]


def predicate(row: dict) -> str:
    kind, binary_name = ALLOWED[row["binary_id"]]
    name = row["name"]
    # The refreshed preparation admits only Rust identifier module paths.
    if not re.fullmatch(r"[A-Za-z_][A-Za-z_0-9]*(?:::[A-Za-z_][A-Za-z_0-9]*)*", name):
        raise ValueError(f"unsafe or unsupported test name: {name}")
    return (
        f"(package(=herdr-threads) & kind({kind}) & "
        f"binary(={binary_name}) & test(={name}))"
    )


def selection_fingerprint(rows: list[dict]) -> str:
    fields = (
        "target", "binary_id", "binary_name", "target_kind", "name",
        "source_path", "source_line", "source_file_sha256",
        "source_function_sha256", "source_tree", "selection",
        "registration_chain", "test_attributes",
    )
    proof = [
        {key: row[key] for key in fields}
        for row in sorted(rows, key=pair)
    ]
    return hashlib.sha256(json.dumps(
        proof, sort_keys=True, separators=(",", ":")
    ).encode()).hexdigest()


def validate_inventory(
    inventory: dict, baseline: dict, seal: dict
) -> tuple[list[dict], list[dict]]:
    """Validate explicit source proof; pure function, including prototypes."""
    if seal.get("schema") != "herdr-threads.v090.selection-seal.v1":
        raise ValueError("wrong V090 selection seal schema")
    rows = inventory["expected_tests"]
    keys = [pair(row) for row in rows]
    if not rows or len(keys) != len(set(keys)):
        raise ValueError("empty or duplicate selection")
    original = {(row["target"], row["name"]): row for row in baseline["expected_tests"]}
    if len(original) != 391:
        raise ValueError("frozen original391 requirement set changed")
    dispositions = seal["reviewed_key_dispositions"]
    if len(dispositions) != 2:
        raise ValueError("both reviewed semantic dispositions must be retained")
    historical = [(row["target"], row["historical"]) for row in dispositions]
    if len(set(historical)) != len(historical):
        raise ValueError("duplicate semantic rename mapping")

    def normalize(key: tuple[str, str]) -> tuple[str, str]:
        for row in dispositions:
            if key == (row["target"], row["historical"]):
                return row["target"], row["canonical"]
        return key

    traces = seal["original391_requirement_map"]
    historical_traces = [(row["historical_target"], row["historical_name"]) for row in traces]
    if len(traces) != 391 or len(set(historical_traces)) != 391 or set(historical_traces) != set(original):
        raise ValueError("original391 traceability is incomplete or fabricated")
    canonical_by_target = {(row["target"], row["name"]) for row in rows}
    for trace in traces:
        old = trace["historical_target"], trace["historical_name"]
        actual = trace["canonical_target"], trace["canonical_name"]
        if actual != normalize(old) or actual not in canonical_by_target:
            raise ValueError(f"baseline requirement lost or incorrectly renamed: {old}")
        prior = original[old]
        if (
            trace["original_source_path"] != prior["source_path"]
            or trace["original_source_file_sha256"] != prior["source_file_sha256"]
            or trace["original_source_function_sha256"] != prior["source_function_sha256"]
        ):
            raise ValueError(f"historical baseline source fingerprints changed: {old}")
    for row in dispositions:
        old = row["target"], row["historical"]
        actual = row["target"], row["canonical"]
        if row["historical_was_in_original391"] != (old in original):
            raise ValueError("fabricated historical391 membership")
        if actual not in canonical_by_target:
            raise ValueError("canonical disposition requirement missing")
    allowed = seal["explicit_source_evidenced_allowlist"]
    allowed_keys = [pair(row) for row in allowed]
    if len(allowed_keys) != len(set(allowed_keys)) or set(keys) != set(allowed_keys):
        raise ValueError("selection differs from explicit frozen allowlist")
    if len(keys) < 391:
        raise ValueError("expanded V090 enumeration cannot shrink original391")
    if len(seal["published_Main244_keys"]) != 244:
        raise ValueError("Main concrete244 provenance is incomplete")
    for origin in seal["published_Main244_keys"] + seal["actual_composed_author_keys"]:
        if normalize((origin["target"], origin["name"])) not in canonical_by_target:
            raise ValueError("source-evidenced Main/author requirement missing")
    fingerprint = selection_fingerprint(rows)
    if fingerprint != seal["selection_fingerprint"] or fingerprint != inventory["selection_fingerprint"]:
        raise ValueError("source/registration/selection fingerprint mismatch")
    fields = (
        "target", "binary_id", "binary_name", "target_kind", "name",
        "source_path", "source_line", "source_file_sha256",
        "source_function_sha256", "source_tree", "selection",
        "registration_chain", "test_attributes",
    )
    proof = {pair(row): row for row in allowed}
    for row in rows:
        if any(row.get(key) != proof[pair(row)].get(key) for key in fields):
            raise ValueError(f"source proof changed: {pair(row)}")
        if row["binary_id"] not in ALLOWED:
            raise ValueError("unexpected target")
        kind, name = ALLOWED[row["binary_id"]]
        if (row["target_kind"], row["binary_name"]) != (kind, name):
            raise ValueError("wrong target kind/name")
        if not row.get("source_registered") or row["source_tree"] != inventory["source"]["tree"]:
            raise ValueError("unregistered or stale source row")
        if any(re.search(r"#\[\s*ignore", attr) for attr in row["test_attributes"]):
            raise ValueError("ignored source declaration cannot be selected")
        predicate(row)
    wanted_mandatory = {
        normalize((row["target"], row["name"])) for row in original.values()
        if row["selection"] == "mandatory_cargo_separate"
    }
    mandatory = [row for row in rows if row["selection"] == "mandatory_cargo_separate"]
    focused = [row for row in rows if row["selection"] == "focused_nextest"]
    if len(mandatory) != 6 or {(row["target"], row["name"]) for row in mandatory} != wanted_mandatory:
        raise ValueError("original mandatory six changed")
    if len(focused) + 6 != len(rows) or not focused:
        raise ValueError("unknown selection class or zero focused set")
    counts = inventory["counts"]
    if (counts["unique_expected_tests"], counts["mandatory_separate"], counts["focused_nextest"]) != (len(rows), 6, len(focused)):
        raise ValueError("static counts disagree with exact keys")
    if {row["binary_id"] for row in rows} != set(ALLOWED):
        raise ValueError("exact five targets required")
    return sorted(focused, key=pair), sorted(mandatory, key=pair)


def validate_execution_seal(inventory: dict, seal: dict) -> None:
    """Parent final metadata only; rejects prototype before any Cargo."""
    final = "sealed_parent_reviewed_final_source"
    if (
        inventory.get("phase") != final or seal.get("phase") != final
        or inventory.get("preparation_only") is not False
        or inventory["source"].get("identity_kind") != "committed_frozen"
    ):
        raise ValueError("PREPARATION_ONLY staged metadata cannot enumerate or execute; refresh/seal only after parent causal fix/review and clean commit")
    if inventory.get("pending_required") or seal.get("pending_required"):
        raise ValueError("pending required source rows prevent enumeration/execution")
    if (
        not inventory["source"].get("head")
        or inventory["source"]["head"] != seal.get("source_head")
        or inventory["source"]["tree"] != seal.get("source_tree")
    ):
        raise ValueError("final committed source seal identity mismatch")
    negative = (
        "herdr-threads",
        "cli::topology_handoff::tests::live::automatic_live::automatic_child_begin_expired_permit_has_no_effects_and_fresh_permit_succeeds",
    )
    if negative not in {pair(row) for row in inventory["expected_tests"]}:
        raise ValueError("required actual imported71bb negative control is absent")
    if len(inventory["expected_tests"]) < 391:
        raise ValueError("standalone selected enumeration must preserve original391")


def inspect_selection(document: dict, expected: list[dict]) -> dict:
    expected_keys = {pair(row) for row in expected}
    suites = document.get("rust-suites")
    if not isinstance(suites, dict):
        raise ValueError("nextest JSON has no rust-suites")
    actual = []
    errors = []
    binaries = []
    for suite_key, suite in suites.items():
        binary = suite.get("binary-id")
        if binary != suite_key:
            errors.append(f"suite key/ID mismatch: {suite_key}")
        if binary not in ALLOWED:
            errors.append(f"unexpected suite: {binary}")
        elif (suite.get("kind"), suite.get("binary-name")) != ALLOWED[binary]:
            errors.append(f"wrong binary kind/name: {binary}")
        if suite.get("package-name") != "herdr-threads" or suite.get("status") != "listed":
            errors.append(f"wrong package or unlisted suite: {binary}")
        cases = suite.get("testcases")
        if not isinstance(cases, dict):
            errors.append(f"missing testcases: {binary}")
            continue
        for name, case in cases.items():
            match = case.get("filter-match", {}).get("status")
            if match not in {"matches", "mismatch"}:
                errors.append(f"unknown filter status: {binary}/{name}: {match}")
            if match != "matches":
                continue
            actual.append((binary, name))
            if case.get("kind") != "test" or case.get("ignored") is not False:
                errors.append(f"ignored or wrong-kind selected case: {binary}/{name}")
        binaries.append({
            "binary_id": binary, "binary_name": suite.get("binary-name"),
            "kind": suite.get("kind"), "path": suite.get("binary-path"),
            "build_platform": suite.get("build-platform"), "cwd": suite.get("cwd"),
        })
    if set(suites) != set(ALLOWED):
        errors.append("listing did not contain exactly the five selected suites")
    missing = sorted(expected_keys - set(actual))
    extra = sorted(set(actual) - expected_keys)
    duplicate = sorted(key for key, count in Counter(actual).items() if count > 1)
    return {
        "status": "matched" if actual and not (errors or missing or extra or duplicate) else "rejected",
        "expected_count": len(expected_keys), "selected_count": len(actual),
        "selected_by_target": dict(Counter(binary for binary, _ in actual)),
        "expected_exact_pairs": sorted(expected_keys), "actual_selected_pairs": sorted(actual),
        "missing": missing, "extra": extra, "duplicates": duplicate, "errors": errors,
        "listed_binaries": binaries, "reported_total_test_count": document.get("test-count"),
        "count_policy": "Exact matching binary/name pairs, never aggregate test-count.",
    }


def parse_nextest_results(path: Path, expected: list[dict]) -> dict:
    # Pinned to the observed default nextest status format; unknown output fails
    # evidence validation instead of inventing executed/pass counts.
    pattern = re.compile(
        r"^\s*(PASS|FAIL|TIMEOUT|LEAK|EXECFAIL|SKIP)\s+\[[^\]]+\]\s+"
        r"\(\s*\d+\s*/\s*\d+\s*\)\s+(\S+)\s+(\S+)\s*$"
    )
    results = []
    for line in path.read_text(errors="replace").splitlines():
        match = pattern.match(line)
        if match:
            status, binary, name = match.groups()
            results.append({"binary_id": binary, "name": name, "status": status})
    keys = [pair(row) for row in results]
    expected_keys = {pair(row) for row in expected}
    return {
        "executed_cases": results, "executed_count": len(results),
        "passed_count": sum(row["status"] == "PASS" for row in results),
        "statuses": dict(Counter(row["status"] for row in results)),
        "missing": sorted(expected_keys - set(keys)),
        "extra": sorted(set(keys) - expected_keys),
        "duplicates": sorted(key for key, count in Counter(keys).items() if count > 1),
        "evidence_complete": bool(results) and set(keys) == expected_keys and len(keys) == len(set(keys)),
        "parser_contract": "Observed nextest PASS/FAIL status lines; never infer counts from exit alone.",
    }


def parse_recovery_results(paths: Path | list[Path], expected: list[dict]) -> dict:
    """Account for every libtest result name, including unrelated namespaces."""
    if isinstance(paths, Path):
        paths = [paths]
    pattern = re.compile(r"^test (\S+) \.\.\. (.+)$")
    results, summaries, errors = [], [], []
    for path in paths:
        for line in path.read_text(errors="replace").splitlines():
            stripped = line.strip()
            match = pattern.match(stripped)
            if match:
                name, status = match.groups()
                if status.startswith("ignored"):
                    status = "ignored"
                elif status not in {"ok", "FAILED"}:
                    errors.append(f"unknown test status in {path.name}: {name}: {status}")
                results.append({
                    "binary_id": "herdr-threads::combined", "name": name,
                    "status": status, "log": str(path),
                })
            if stripped.startswith("test result:"):
                counts = dict((kind, int(number)) for number, kind in re.findall(
                    r"\b(\d+) (passed|failed|ignored|measured)\b", stripped
                ))
                summaries.append({"log": str(path), "line": stripped, "counts": counts})
                if set(counts) != {"passed", "failed", "ignored", "measured"}:
                    errors.append(f"unrecognized summary in {path.name}")
        if sum(summary["log"] == str(path) for summary in summaries) != 1:
            errors.append(f"expected one test summary in {path.name}")
    keys = [pair(row) for row in results]
    expected_keys = {pair(row) for row in expected}
    missing = sorted(expected_keys - set(keys))
    extra = sorted(set(keys) - expected_keys)
    duplicates = sorted(key for key, count in Counter(keys).items() if count > 1)
    ignored = [row["name"] for row in results if row["status"] == "ignored"]
    totals = Counter()
    for summary in summaries:
        totals.update(summary["counts"])
    observed = {
        "passed": sum(row["status"] == "ok" for row in results),
        "failed": sum(row["status"] == "FAILED" for row in results),
        "ignored": len(ignored), "measured": 0,
    }
    if dict(totals) != observed:
        errors.append("summaries disagree with all actual test result lines")
    return {
        "executed_cases": results, "executed_count": len(results),
        "passed_count": observed["passed"], "missing": missing, "extra": extra,
        "duplicates": duplicates, "ignored": ignored, "errors": errors,
        "evidence_complete": bool(results) and not (
            missing or extra or duplicates or ignored or errors
        ),
        "summaries": summaries,
        "selection_policy": "Six independently exact Cargo calls; every result line accounted.",
    }


def check_source(source: Path, inventory: dict) -> dict:
    expected = inventory["source"]
    observed = {
        "head": text_git(source, "rev-parse", "HEAD"),
        "tree": text_git(source, "rev-parse", "HEAD^{tree}"),
        "tracked_status": text_git(source, "status", "--porcelain", "--untracked-files=no"),
        "unmerged": text_git(source, "ls-files", "-u"),
        "index_stage_sha256": hashlib.sha256(git(source, "ls-files", "--stage")).hexdigest(),
    }
    if observed["head"] != expected["head"] or observed["tree"] != expected["tree"]:
        raise ValueError("HEAD/tree differs from refreshed inventory")
    if observed["tracked_status"] or observed["unmerged"]:
        raise ValueError("source/index is not frozen clean")
    pending_merge = subprocess.run(["git", "rev-parse", "--verify", "MERGE_HEAD"], cwd=source, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    if pending_merge.returncode == 0:
        raise ValueError("final source still has a merge in progress")
    fingerprints = {}
    for relative, value in inventory["source_files"].items():
        path = (source / relative).resolve()
        if not path.is_relative_to(source) or not path.is_file():
            raise ValueError(f"invalid source fingerprint path: {relative}")
        actual = sha256(path)
        blob = text_git(source, "rev-parse", observed["tree"] + ":" + relative)
        if actual != value["sha256"] or blob != value["git_blob_id"]:
            raise ValueError(f"source/blob fingerprint mismatch: {relative}")
        fingerprints[relative] = actual
    for row in inventory["expected_tests"]:
        if fingerprints.get(row["source_path"]) != row["source_file_sha256"]:
            raise ValueError(f"row/file fingerprint mismatch: {pair(row)}")
    config = tomllib.loads((source / ".config/nextest.toml").read_text())
    if config["test-groups"]["automatic-envelope"]["max-threads"] != 1:
        raise ValueError("automatic-envelope max-threads changed")
    overrides = config["profile"]["default"]["overrides"]
    protected = [
        row for row in overrides if row.get("test-group") == "automatic-envelope"
        and row.get("threads-required") == "num-test-threads" and row.get("priority") == 100
        and "maximum_escaped_body_completes_and_recovers_exactly" in row.get("filter", "")
        and "actual_admission_boundary_completes_and_next_metadata_size_has_zero_effects" in row.get("filter", "")
    ]
    if len(protected) != 1:
        raise ValueError("both ALLSLOTS positives must retain the shared exact scheduling override")
    recovery = source / "tests/cli/lazy_recovery_context.rs"
    text = recovery.read_text()
    if "Duration::from_secs(30)" not in text or "Duration::from_millis(20)" not in text:
        raise ValueError("mandatory recovery30s/20ms projection waits changed")
    if sha256(recovery) != inventory["mandatory_recovery"]["source_file_sha256"]:
        raise ValueError("mandatory recovery file fingerprint changed")
    observed["source_file_sha256"] = fingerprints
    return observed


def binary_stamp(selection: dict, target: Path, required_executable: bool = True) -> dict:
    binaries = []
    for row in selection["listed_binaries"]:
        path = Path(row["path"]).resolve()
        if not path.is_relative_to(target) or not path.is_file():
            raise ValueError(f"binary lies outside the sole warmed target or is missing: {path}")
        binaries.append({**row, "sha256": sha256(path), "bytes": path.stat().st_size})
    executable = target / "debug" / "herdr-threads"
    if required_executable and not executable.is_file():
        raise ValueError("required built CLI executable is missing")
    return {
        "binaries": binaries,
        "executable": {"path": str(executable), "sha256": sha256(executable), "bytes": executable.stat().st_size}
        if executable.is_file() else None,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--inventory", type=Path, required=True)
    parser.add_argument("--selection-seal", type=Path, required=True)
    parser.add_argument("--evidence-dir", "--evidence", dest="evidence_dir", type=Path, required=True)
    parser.add_argument("--run", action="store_true")
    args = parser.parse_args()
    source = args.source.resolve()
    inventory_path = args.inventory.resolve()
    inventory = json.loads(inventory_path.read_text())
    seal_path = args.selection_seal.resolve()
    seal = json.loads(seal_path.read_text())
    seal_hash = sha256(seal_path)
    baseline_path = Path(__file__).with_name("resume-composed-matrix-preparation-f330bdab.json")
    if sha256(baseline_path) != BASELINE_SHA256:
        parser.error("immutable f330391 baseline snapshot changed")
    baseline = json.loads(baseline_path.read_text())
    if source != Path(inventory["source"]["path"]).resolve():
        parser.error("--source does not match inventory source")
    focused, mandatory = validate_inventory(inventory, baseline, seal)
    if not args.evidence_dir.is_absolute():
        parser.error("evidence directory must be absolute and fresh")
    evidence = args.evidence_dir.resolve()
    evidence.mkdir(parents=True, exist_ok=False)
    target = Path(inventory["parent_runner_requirements"]["cargo_target_dir"]).resolve()
    baseline_target = Path(baseline["parent_runner_requirements"]["cargo_target_dir"]).resolve()
    if target != baseline_target:
        parser.error("inventory must use the existing sole absolute warmed D target")
    run_id = str(uuid.uuid4())
    inventory_hash = sha256(inventory_path)
    record = {
        "schema": 1, "status": "started", "started_utc": datetime.now(timezone.utc).isoformat(),
        "run_requested": args.run, "focused_executed": False, "mandatory_executed": False,
        "source": str(source), "expected_head": inventory["source"]["head"],
        "expected_tree": inventory["source"]["tree"], "inventory": str(inventory_path),
        "inventory_sha256": inventory_hash, "selection_seal": str(seal_path),
        "selection_seal_sha256": seal_hash, "baseline_inventory": str(baseline_path),
        "baseline_sha256": BASELINE_SHA256, "runner_sha256": sha256(Path(__file__)),
        "ht_leak_run_id": run_id, "target": str(target),
        "expected_counts": {"all": len(focused) + 6, "focused": len(focused), "mandatory": 6},
        "commands": [], "artifact_limits": {"log_bytes": LOG_LIMIT, "list_bytes": LIST_LIMIT, "record_bytes": RECORD_LIMIT},
    }
    record_path = evidence / "execution.json"
    env = dict(os.environ)
    original_home = env.get("HOME")
    if original_home:
        env.setdefault("CARGO_HOME", str(Path(original_home) / ".cargo"))
        env.setdefault("RUSTUP_HOME", str(Path(original_home) / ".rustup"))
    for variable, suffix in {
        "HOME": "home", "CLAUDE_CONFIG_DIR": "claude", "CODEX_HOME": "codex",
        "GEMINI_CONFIG_DIR": "gemini", "GEMINI_CLI_HOME": "gemini-home",
    }.items():
        path = evidence / "isolated-config" / suffix
        path.mkdir(parents=True, exist_ok=True)
        env[variable] = str(path)
    env.update(CARGO_BUILD_JOBS="4", CARGO_TARGET_DIR=str(target), HT_LEAK_RUN_ID=run_id,
               HT_ACTIVATION_CAPTURE_PATH=str(evidence / "watch-activation.jsonl"),
               CARGO_TERM_COLOR="never")
    record["child_environment"] = {
        key: env[key] for key in [
            "CARGO_BUILD_JOBS", "CARGO_TARGET_DIR", "HT_LEAK_RUN_ID", "HT_ACTIVATION_CAPTURE_PATH",
            "HOME", "CLAUDE_CONFIG_DIR", "CODEX_HOME", "GEMINI_CONFIG_DIR", "GEMINI_CLI_HOME",
        ]
    }
    record["toolchain_environment"] = {key: env[key] for key in ("CARGO_HOME", "RUSTUP_HOME") if key in env}
    record["parent_home_modified"] = False
    save(record_path, record)
    leak_required = False
    outcome = 1
    cancellation = Cancellation()

    def command(stage: str, argv: list[str], list_json: bool = False) -> int:
        output = evidence / (stage + (".json" if list_json else ".log"))
        error_path = evidence / (stage + ".stderr.log") if list_json else None
        entry = {"stage": stage, "argv": argv, "cwd": str(source), "stdout": str(output)}
        if error_path:
            entry["stderr"] = str(error_path)
        record["commands"].append(entry)
        save(record_path, record)
        print(f"{stage}: {output}", flush=True)
        started = time.monotonic()
        child = None
        streams = []
        selector = selectors.DefaultSelector()
        errors = []
        ended_at = None
        try:
            stdout_file = output.open("wb")
            streams.append(stdout_file)
            stderr_file = error_path.open("wb") if error_path else None
            if stderr_file:
                streams.append(stderr_file)
            # Keep a created group owned even if termination arrives while
            # Popen waits for exec or immediately before assignment completes.
            with cancellation.defer():
                child = subprocess.Popen(argv, cwd=source, env=env, stdout=subprocess.PIPE,
                                         stderr=subprocess.PIPE if list_json else subprocess.STDOUT,
                                         start_new_session=True)
            selector.register(child.stdout, selectors.EVENT_READ, [stdout_file, LIST_LIMIT if list_json else LOG_LIMIT, 0, False])
            if child.stderr is not None:
                selector.register(child.stderr, selectors.EVENT_READ, [stderr_file, LOG_LIMIT, 0, False])
            while selector.get_map():
                for key, _ in selector.select(timeout=0.2):
                    chunk = os.read(key.fd, 64 * 1024)
                    if not chunk:
                        selector.unregister(key.fileobj)
                        key.fileobj.close()
                        continue
                    file, limit, count, overflow = key.data
                    remaining = max(0, limit - count)
                    file.write(chunk[:remaining])
                    key.data[2] += len(chunk)
                    if len(chunk) > remaining and not overflow:
                        key.data[3] = True
                        errors.append(f"output exceeded{limit} bytes: {file.name}; excess drained, never treated as complete evidence")
                if child.poll() is not None:
                    ended_at = ended_at or time.monotonic()
                    if selector.get_map() and time.monotonic() - ended_at > 2:
                        errors.append("owned command exited but inherited output pipe remained open; scoped leak check required")
                        break
            actual_exit = child.wait()
            entry["exit"] = actual_exit
            if errors:
                entry["capture_errors"] = errors
            return actual_exit if actual_exit else int(bool(errors))
        finally:
            with cancellation.defer():
                if child is not None and (
                    child.poll() is None or cancellation.signum is not None
                ):
                    terminate_owned_group(child)
                    entry["interrupted_child_exit"] = child.returncode
                    entry["owned_process_group"] = child.pid
                for key in list(selector.get_map().values()):
                    selector.unregister(key.fileobj)
                    key.fileobj.close()
                selector.close()
                for stream in streams:
                    stream.close()
                entry["seconds"] = time.monotonic() - started
                for label, path in (("stdout", output), ("stderr", error_path)):
                    if path is not None and path.exists():
                        entry[label + "_sha256"] = sha256(path)
                        entry[label + "_bytes"] = path.stat().st_size
                save(record_path, record)

    def frozen(stage: str, original: dict | None = None) -> dict:
        if sha256(inventory_path) != inventory_hash or sha256(seal_path) != seal_hash:
            raise ValueError("inventory/selection seal changed during parent sweep")
        observed = check_source(source, inventory)
        if original is not None and observed != original:
            raise ValueError(f"source/index changed during{stage}")
        record[stage] = observed
        save(record_path, record)
        return observed

    def leak_check(stage: str) -> int:
        with cancellation.defer():
            code = command(stage, ["scripts/check-no-leaked-processes", "--run-id", run_id])
            record[stage + "_exit"] = code
        return code

    previous_signals = {sig: signal.signal(sig, cancellation.handle) for sig in (signal.SIGTERM, signal.SIGINT)}
    try:
        validate_execution_seal(inventory, seal)
        before = frozen("source_before")
        leak_required = True
        all_rows = sorted(focused + mandatory, key=pair)
        full_expression = " | ".join(predicate(row) for row in all_rows)
        focused_expression = " | ".join(predicate(row) for row in focused)
        save(evidence / "selection-expressions.json", {
            "all": full_expression, "focused": focused_expression,
            "all_sha256": hashlib.sha256(full_expression.encode()).hexdigest(),
            "focused_sha256": hashlib.sha256(focused_expression.encode()).hexdigest(),
        })
        list_argv = ["nice", "cargo", "nextest", "list", "--locked", "--all-features",
                     *TARGET_ARGS, "--message-format", "json", "-E", full_expression]
        listed = command("nextest-list", list_argv, list_json=True)
        if listed:
            raise RuntimeError(f"nextest enumeration failed or incomplete capture: exit{listed}")
        selection = inspect_selection(json.loads((evidence / "nextest-list.json").read_text()), all_rows)
        save(evidence / "run-selection.json", selection)
        record["selection"] = {
            "status": selection["status"], "selected_count": selection["selected_count"],
            "selected_by_target": selection["selected_by_target"],
            "path": str(evidence / "run-selection.json"), "sha256": sha256(evidence / "run-selection.json"),
        }
        if selection["status"] != "matched":
            raise ValueError("exact selected names rejected; inspect run-selection.json")
        frozen("source_after_list", before)
        binaries = binary_stamp(selection, target)
        record["binaries_after_list"] = binaries
        record["status"] = "enumerated_not_run"
        outcome = 0
        if args.run:
            focused_code = 1
            focused_leaks = 1
            frozen("source_before_focused", before)
            if binary_stamp(selection, target) != binaries:
                raise ValueError("binaries/executable changed before focused run")
            record["binaries_before_focused"] = binaries
            try:
                record["focused_executed"] = True
                focused_code = command("nextest-run", [
                    "nice", "cargo", "nextest", "run", "--locked", "--all-features",
                    *TARGET_ARGS, "--status-level", "pass", "--final-status-level", "none",
                    "--no-fail-fast", "-E", focused_expression,
                ])
                results = parse_nextest_results(evidence / "nextest-run.log", focused)
                save(evidence / "focused-results.json", results)
                record["focused_results"] = results
                frozen("source_after_focused", before)
                record["binaries_after_focused"] = binary_stamp(selection, target)
                if record["binaries_after_focused"] != binaries:
                    raise ValueError("focused binary/executable hashes changed")
                if not results["evidence_complete"] or results["passed_count"] != len(focused):
                    focused_code = focused_code or 1
            finally:
                focused_leaks = leak_check("focused-scoped-leaks")
            if focused_leaks:
                raise RuntimeError("focused scoped leaks remain; mandatory stage not started")
            frozen("source_before_mandatory", before)
            record["binaries_before_mandatory"] = binary_stamp(selection, target)
            if record["binaries_before_mandatory"] != binaries:
                raise ValueError("binaries/executable changed before mandatory stage")
            mandatory_code = 1
            mandatory_leaks = 1
            try:
                record["mandatory_executed"] = True
                mandatory_code = 0
                mandatory_logs = []
                record["mandatory_exact_cases"] = []
                for index, row in enumerate(mandatory, 1):
                    stage = f"mandatory-recovery-{index:02}"
                    frozen(f"source_before_{stage}", before)
                    if binary_stamp(selection, target) != binaries:
                        raise ValueError("mandatory exact-case binaries changed")
                    case_exit = command(stage, mandatory_argv(row))
                    log = evidence / (stage + ".log")
                    case_results = parse_recovery_results(log, [row])
                    record["mandatory_exact_cases"].append({
                        "binary_id": row["binary_id"], "name": row["name"],
                        "exit": case_exit, "results": case_results,
                    })
                    mandatory_logs.append(log)
                    if case_exit or not case_results["evidence_complete"] or case_results["passed_count"] != 1:
                        mandatory_code = 1
                    frozen(f"source_after_{stage}", before)
                    if binary_stamp(selection, target) != binaries:
                        raise ValueError("mandatory exact-case binaries changed")
                results = parse_recovery_results(mandatory_logs, mandatory)
                save(evidence / "mandatory-results.json", results)
                record["mandatory_results"] = results
                frozen("source_after_mandatory", before)
                record["binaries_after_mandatory"] = binary_stamp(selection, target)
                if record["binaries_after_mandatory"] != binaries:
                    raise ValueError("mandatory stage binary/executable hashes changed")
                if not results["evidence_complete"] or results["ignored"] or results["passed_count"] != 6:
                    mandatory_code = mandatory_code or 1
            finally:
                mandatory_leaks = leak_check("mandatory-scoped-leaks")
            outcome = int(bool(focused_code or mandatory_code or mandatory_leaks))
            record["status"] = "run_failed" if outcome else "run_passed_cleanup_pending"
    except TerminationRequested as error:
        record["status"] = "interrupted"
        record["termination_signal"] = error.signum
        record["error"] = "Catchable termination; owned group cleanup and scoped scans attempted."
        outcome = 128 + error.signum
    except KeyboardInterrupt:
        record["status"] = "interrupted"
        record["error"] = "Parent interruption; only runner-owned command group stopped; scoped cleanup still attempted."
        outcome = 130
    except Exception as error:
        record["status"] = "failed"
        record["error"] = f"{type(error).__name__}: {error}"
        outcome = 1
    finally:
        with cancellation.defer(raise_after=False):
            if leak_required:
                try:
                    leak_exit = leak_check("final-scoped-leaks")
                    if leak_exit:
                        record["status_before_cleanup_failure"] = record["status"]
                        record["status"] = "cleanup_failed"
                        outcome = 1
                    elif record["status"] == "run_passed_cleanup_pending":
                        record["status"] = "passed"
                except Exception as error:
                    record["cleanup_error"] = f"{type(error).__name__}: {error}"
                    record["status"] = "cleanup_failed"
                    outcome = 1
            try:
                record["source_final"] = check_source(source, inventory)
                if "source_before" in record and record["source_final"] != record["source_before"]:
                    raise ValueError("final source/index differs")
                if sha256(inventory_path) != inventory_hash or sha256(seal_path) != seal_hash:
                    raise ValueError("final inventory/selection seal differs")
                if "binaries_after_list" in record:
                    record["binaries_final"] = binary_stamp(selection, target)
                    if record["binaries_final"] != record["binaries_after_list"]:
                        raise ValueError("final binary/executable differs")
            except Exception as error:
                record["final_provenance_error"] = f"{type(error).__name__}: {error}"
                record["status"] = "provenance_failed"
                outcome = outcome or 1
            capture = Path(env["HT_ACTIVATION_CAPTURE_PATH"])
            if capture.is_file():
                record["watch_capture"] = {"path": str(capture), "sha256": sha256(capture), "bytes": capture.stat().st_size}
                if capture.stat().st_size > LOG_LIMIT:
                    record["watch_capture_limit_warning"] = "External producer capture exceeds log budget; retained honestly, never truncated by this runner."
                    record["status"] = "capture_limit_exceeded"
                    outcome = outcome or 1
            if cancellation.signum is not None:
                record["termination_signal"] = cancellation.signum
                record["status_before_termination"] = record["status"]
                record["status"] = "interrupted"
                outcome = 128 + cancellation.signum
            record["finished_utc"] = datetime.now(timezone.utc).isoformat()
            record["exit"] = outcome
            save(record_path, record)
        for sig, previous in previous_signals.items():
            signal.signal(sig, previous)
    print(json.dumps({"status": record["status"], "evidence": str(record_path), "run_id": run_id}), flush=True)
    return outcome


if __name__ == "__main__":
    sys.exit(main())
