"""Allowlisted receipt evidence and independent pair reconciliation.

`source`: `synthetic` (no model ran), `native` (a live harness run whose accept/ACK rows all carry native
proof, `verified_current_target`), or `cooperative` (a live harness run whose rows carry the hook-established
cooperative claim, e.g. `cooperative_top_level`; model issuance is proven by the transcript join, not by the
DB class), or `none` (a live run that stored no accept/ACK provenance). A PASS needs live evidence (`native`
or `cooperative`); only `native` may be cited as native proof.
"""

import re

# `none`: a live run that stored no accept/ACK provenance at all (never a PASS source).
SOURCES = ("synthetic", "native", "cooperative", "none")
LIVE_SOURCES = ("native", "cooperative")
FIELDS = {"schema_version", "run_id", "status", "reason", "scenario", "source", "accepted_pairs",
          "model_calls", "db_receipts", "transport_hints", "retired_pairs", "artifacts"}
PAIR = {"message_id", "recipient_id"}
CALL = PAIR | {"call_id", "actor", "artifact"}
RECEIPT = CALL | {"receipt_id"}
IDENTIFIER = re.compile(r"[A-Za-z0-9][A-Za-z0-9:._/-]{0,127}\Z")
REASON = re.compile(r"[a-z][a-z0-9_]{0,63}\Z")
SUPPORT_FIELDS = {"artifact_type", "run_id", "status", "reason"}
SUPPORT_REASONS = {"interrupted", "host_unavailable", "fixture_error", "ambiguous_close", "unsupported_native"}


def validate_supporting_artifact(record):
    if (not isinstance(record, dict) or set(record) != SUPPORT_FIELDS
            or record["artifact_type"] != "fixture_event"
            or record["status"] not in ("FAIL", "UNSUPPORTED")
            or not isinstance(record["reason"], str) or record["reason"] not in SUPPORT_REASONS
            or not isinstance(record["run_id"], str) or not IDENTIFIER.fullmatch(record["run_id"])):
        raise ValueError("supporting evidence is not sanitized")


def _rows(rows, required):
    if not isinstance(rows, list):
        raise ValueError("evidence rows must be lists")
    for row in rows:
        if not isinstance(row, dict) or set(row) != required or any(not isinstance(value, str) or not IDENTIFIER.fullmatch(value) for value in row.values()):
            raise ValueError("malformed evidence row")


def reconcile(manifest):
    accepted = {(r["message_id"], r["recipient_id"]) for r in manifest["accepted_pairs"]}
    calls = {(r["call_id"], r["message_id"], r["recipient_id"], r["actor"]) for r in manifest["model_calls"]}
    acked = {(r["message_id"], r["recipient_id"]) for r in manifest["db_receipts"]
             if (r["call_id"], r["message_id"], r["recipient_id"], r["actor"]) in calls}
    retired = {(r["message_id"], r["recipient_id"]) for r in manifest["retired_pairs"]}
    def pairs(items):
        return [{"message_id": message, "recipient_id": recipient} for message, recipient in sorted(items)]
    return {"acked": pairs(acked), "retired": pairs(retired), "pending": pairs(accepted - acked - retired),
            "duplicate_logical_records": len(manifest["accepted_pairs"]) - len(accepted),
            "repeated_transport_hints": len(manifest["transport_hints"]) - len({(r["message_id"], r["recipient_id"]) for r in manifest["transport_hints"]})}


def validate_manifest(manifest):
    if not isinstance(manifest, dict) or set(manifest) != FIELDS:
        raise ValueError("manifest fields are not allowlisted")
    if manifest["schema_version"] != 1 or manifest["status"] not in ("PASS", "FAIL", "UNSUPPORTED") or manifest["source"] not in SOURCES:
        raise ValueError("invalid manifest metadata")
    if any(not isinstance(manifest[key], str) or not IDENTIFIER.fullmatch(manifest[key]) for key in ("run_id", "scenario")):
        raise ValueError("missing run metadata")
    if not isinstance(manifest["reason"], str) or (manifest["status"] != "PASS" and not REASON.fullmatch(manifest["reason"])) or (manifest["status"] == "PASS" and manifest["reason"]):
        raise ValueError("FAIL/UNSUPPORTED need reason; PASS has none")
    for name, fields in (("accepted_pairs", PAIR), ("model_calls", CALL), ("db_receipts", RECEIPT),
                         ("transport_hints", PAIR), ("retired_pairs", PAIR)):
        _rows(manifest[name], fields)
    if not isinstance(manifest["artifacts"], list) or any(not isinstance(path, str) or not IDENTIFIER.fullmatch(path) or path.startswith("/") or ".." in path.split("/") for path in manifest["artifacts"]):
        raise ValueError("artifacts must be relative private-run paths")
    if any(row["artifact"] not in manifest["artifacts"] for name in ("model_calls", "db_receipts") for row in manifest[name]):
        raise ValueError("model and SQLite evidence needs listed artifacts")
    accepted = {(r["message_id"], r["recipient_id"]) for r in manifest["accepted_pairs"]}
    if len(accepted) != len(manifest["accepted_pairs"]):
        raise ValueError("duplicate accepted pair")
    calls = {(r["call_id"], r["message_id"], r["recipient_id"], r["actor"]) for r in manifest["model_calls"]}
    if len(calls) != len(manifest["model_calls"]):
        raise ValueError("duplicate model call")
    for row in manifest["db_receipts"]:
        if (row["call_id"], row["message_id"], row["recipient_id"], row["actor"]) not in calls:
            raise ValueError("DB receipt lacks actual model call")
    if len({(r["message_id"], r["recipient_id"]) for r in manifest["db_receipts"]}) != len(manifest["db_receipts"]):
        raise ValueError("duplicate SQLite receipt for accepted pair")
    for name in ("model_calls", "db_receipts", "retired_pairs"):
        if any((r["message_id"], r["recipient_id"]) not in accepted for r in manifest[name]):
            raise ValueError("evidence references unaccepted pair")
    result = reconcile(manifest)
    if set((r["message_id"], r["recipient_id"]) for r in result["acked"]) & set((r["message_id"], r["recipient_id"]) for r in result["retired"]):
        raise ValueError("pair both ACKed and retired")
    if manifest["status"] == "PASS" and (not result["acked"] or result["pending"] or manifest["source"] not in LIVE_SOURCES):
        raise ValueError("PASS needs complete live (native or cooperative) receipt evidence")
    return result
