"""Bounded read-only evidence for the existing wake-e2e driver.

Transport labels are compositional inferences from canonical state and the reviewed
no-fallback submission path, never observations of Herdr input requests. Native
proof also requires the caller's successful external-wake check. A qualified scope
is canonical activity evidence, not a claim about host focus, drafts or foreground.
"""
from __future__ import annotations

import contextlib
import json
from pathlib import Path
import sqlite3

MAX_BINDINGS = 64
MAX_INTENTS = 32
MAX_IDS = 128
MAX_SAMPLES = 128
MAX_BYTES = 65536


def _text(value, limit=128, optional=False):
    if value is None and optional:
        return None
    if not isinstance(value, str) or not 0 < len(value) <= limit or any(not 33 <= ord(c) <= 126 for c in value):
        raise ValueError("invalid identifier")
    return value


def _number(value, optional=False):
    if value is None and optional:
        return None
    if type(value) is not int or not 0 <= value <= (1 << 64) - 1:
        raise ValueError("invalid number")
    return value


def _json(value, limit):
    if not isinstance(value, (str, bytes)) or len(value.encode() if isinstance(value, str) else value) > limit:
        raise ValueError("invalid metadata size")
    return json.loads(value)


def _process(value):
    result = {"pid": _number(value["pid"]), "start": _number(value["start"])}
    if not 1 < result["pid"] <= (1 << 31) - 1:
        raise ValueError("invalid pid")
    return result


def _scope(value):
    return {"binding": _text(value["binding"]), "native_session": _text(value["native_session"]),
            "execution": _text(value["execution"]), "generation": _number(value["generation"]),
            "process": _process(value["process"]), "kernel_boot": _text(value["kernel_boot"], 64)}


def _helper(value):
    if value is None:
        return None
    value = _json(value, 2048)
    purpose = value["purpose"]
    if purpose not in ("add", "list", "delete"):
        raise ValueError("invalid helper purpose")
    witness = value.get("witness")
    if witness is not None:
        witness = {"process": _process(witness["process"]), "pgid": _number(witness["pgid"]),
                   "kernel_boot": _text(witness["kernel_boot"], 64)}
    return {"id": _text(value["id"]), "purpose": purpose,
            "subject": _text(value.get("subject"), optional=True),
            "owner_boot": _text(value["owner_boot"]), "kernel_boot": _text(value["kernel_boot"], 64),
            "witness": witness}


def _failure(error, truncated=False):
    # Never include exception text, SQL, database paths or raw metadata in evidence.
    return {"ok": False, "error": error, "truncated": truncated, "census_complete": False,
            "watermark": None, "bindings": [], "native": [], "intents": [], "wake_work": None}


def queue_snapshot(state: Path, seat: str, after: int = 0) -> dict:
    """One consistent live-WAL snapshot from the run's own daemon database.

    All open bindings count toward the 64-row canonical census, including children
    and disconnected rows. Only allowlisted scope/helper metadata is decoded;
    descriptors, diagnostics, credentials and message bodies are never selected.
    """
    try:
        _text(seat)
        _number(after)
        path = Path(state) / "node" / "herdr-threads.db"
        if not path.is_file():
            return _failure("database_missing")
        with contextlib.closing(sqlite3.connect(path.resolve().as_uri() + "?mode=ro", uri=True, timeout=.25)) as db:
            db.row_factory = sqlite3.Row
            db.execute("PRAGMA query_only=ON")
            db.execute("BEGIN")
            row = db.execute("SELECT instance_id,state,generation,target_id,target_generation FROM seats WHERE id=?", (seat,)).fetchone()
            if row is None:
                return _failure("seat_missing")
            seat_scope = {"instance": _text(row["instance_id"]), "state": _text(row["state"]),
                          "generation": _number(row["generation"]), "target": _text(row["target_id"], optional=True),
                          "target_generation": _number(row["target_generation"])}
            boot = db.execute("SELECT boot FROM node_kernel_boot WHERE id=1").fetchone()
            snapshot = {"ok": True, "seat": seat, "after": after, "truncated": False,
                        "seat_scope": seat_scope, "kernel_boot": _text(boot[0], 64) if boot else None,
                        "watermark": db.execute("SELECT COALESCE(MAX(ordinal),0) FROM wake_intents").fetchone()[0],
                        "bindings": [], "native": [], "intents": [], "wake_work": None}
            bindings = db.execute("SELECT binding_id,harness,native_session,execution_id,generation,liveness_pid,liveness_start,liveness_boot,liveness,role,agent_id,registered_at,observation_provenance,target_id,target_generation FROM occupant_bindings WHERE seat_id=? AND ended_at IS NULL ORDER BY ordinal DESC LIMIT 65", (seat,)).fetchall()
            snapshot["census_complete"] = len(bindings) <= MAX_BINDINGS
            snapshot["truncated"] = not snapshot["census_complete"]
            for binding in bindings:
                scope = None
                if all(binding[k] is not None for k in ("liveness_pid", "liveness_start", "liveness_boot")):
                    scope = _scope({"binding": binding["binding_id"], "native_session": binding["native_session"],
                                    "execution": binding["execution_id"], "generation": binding["generation"],
                                    "process": {"pid": binding["liveness_pid"], "start": binding["liveness_start"]},
                                    "kernel_boot": binding["liveness_boot"]})
                snapshot["bindings"].append({"adapter": _text(binding["harness"], 64), "scope": scope,
                    "connected": binding["liveness"] == "connected", "top_level": binding["role"] == "top_level",
                    "root_agent": binding["agent_id"] == "", "registered": binding["registered_at"] is not None and binding["observation_provenance"] == "cooperative_top_level",
                    "target": _text(binding["target_id"]), "target_generation": _number(binding["target_generation"])})
                native = db.execute("SELECT scope_json,turn_id,phase,observation_ordinal FROM native_turn_state WHERE instance_id=? AND binding_id=?", (seat_scope["instance"], binding["binding_id"])).fetchone()
                if native:
                    phase = native["phase"]
                    if phase not in ("started", "ended", "end_first", "unknown"):
                        raise ValueError("invalid native phase")
                    snapshot["native"].append({"scope": _scope(_json(native["scope_json"], 4096)),
                        "turn_id": _text(native["turn_id"]), "phase": phase, "ordinal": _number(native["observation_ordinal"])})
            work = db.execute("SELECT reservation_id,reservation_boot,last_reservation_id,last_reservation_boot,last_outcome,reserved_at_utc,last_reserved_at_utc,completed_at_utc,reason_bits,retry_step FROM wake_work WHERE seat_id=?", (seat,)).fetchone()
            if work:
                snapshot["wake_work"] = dict(zip(("active_attempt", "active_boot", "last_attempt", "last_boot", "last_outcome", "reserved_at_utc", "last_reserved_at_utc", "completed_at_utc", "reason_bits", "retry_step"), work))
                for key in ("active_attempt", "active_boot", "last_attempt", "last_boot", "last_outcome"):
                    snapshot["wake_work"][key] = _text(snapshot["wake_work"][key], optional=True)
            intents = db.execute("SELECT ordinal,attempt,adapter,target_json,phase,revision,owner_boot,arm_nonce,arm_native_id,arm_native_ordinal,logical_status,fulfillment_turn,fulfillment_ordinal,cleanup_quarantined,cleanup_list_attempted,native_no_add_exception,helper_json,ids_complete,created_at,updated_at FROM wake_intents WHERE instance_id=? AND origin_seat=? AND ordinal>? ORDER BY ordinal LIMIT 33", (seat_scope["instance"], seat, after)).fetchall()
            snapshot["truncated"] |= len(intents) > MAX_INTENTS
            for intent in intents[:MAX_INTENTS]:
                ids = db.execute("SELECT queue_id,cleanup_attempted FROM wake_intent_ids WHERE intent_ordinal=? ORDER BY queue_id LIMIT 129", (intent["ordinal"],)).fetchall()
                snapshot["truncated"] |= len(ids) > MAX_IDS
                phase, logical = intent["phase"], intent["logical_status"]
                if phase not in ("prepared", "armed", "accepted", "unknown", "no_effect") or logical not in ("active", "fulfilled", "scope_ended"):
                    raise ValueError("invalid intent state")
                snapshot["intents"].append({"ordinal": _number(intent["ordinal"]), "attempt": _text(intent["attempt"]),
                    "adapter": _text(intent["adapter"], 64), "scope": _scope(_json(intent["target_json"], 4096)),
                    "phase": phase, "revision": _number(intent["revision"]), "owner_boot": _text(intent["owner_boot"]),
                    "arm_nonce": _text(intent["arm_nonce"], optional=True),
                    "baseline": {"turn_id": _text(intent["arm_native_id"]), "ordinal": _number(intent["arm_native_ordinal"])} if intent["arm_native_id"] is not None else None,
                    "logical_status": logical,
                    "fulfillment": {"turn_id": _text(intent["fulfillment_turn"]), "ordinal": _number(intent["fulfillment_ordinal"])} if intent["fulfillment_turn"] is not None else None,
                    "cleanup_quarantined": bool(intent["cleanup_quarantined"]), "cleanup_list_attempted": bool(intent["cleanup_list_attempted"]),
                    "native_no_add_exception": bool(intent["native_no_add_exception"]), "helper": _helper(intent["helper_json"]),
                    "ids": [{"id": _text(item["queue_id"]), "cleanup_attempted": bool(item["cleanup_attempted"])} for item in ids[:MAX_IDS]],
                    "ids_complete": bool(intent["ids_complete"]), "created_at": intent["created_at"], "updated_at": intent["updated_at"]})
            db.execute("ROLLBACK")
        if len(json.dumps(snapshot, separators=(",", ":")).encode()) > MAX_BYTES:
            return _failure("snapshot_byte_limit", True)
        return snapshot
    except (sqlite3.Error, OSError):
        return _failure("database_or_schema_unavailable")
    except (ValueError, TypeError, KeyError, UnicodeError, RecursionError):
        return _failure("invalid_evidence")


def eligible_scope(snapshot: dict) -> dict | None:
    """Unique connected hooked Codex scope with canonical matched Start/End evidence."""
    if not snapshot.get("ok") or snapshot.get("truncated") or not snapshot.get("census_complete"):
        return None
    connected = [b for b in snapshot["bindings"] if b["connected"] and b["top_level"]]
    if len(connected) != 1:
        return None
    binding = connected[0]
    scope, seat = binding["scope"], snapshot["seat_scope"]
    if (not scope or binding["adapter"] != "codex" or not binding["registered"] or not binding["root_agent"]
            or seat["state"] != "resolved" or scope["generation"] != seat["generation"]
            or scope["kernel_boot"] != snapshot["kernel_boot"] or binding["target"] != seat["target"]
            or binding["target_generation"] != seat["target_generation"]):
        return None
    native = [n for n in snapshot["native"] if n["scope"] == scope and n["phase"] == "ended"]
    if len(native) != 1:
        return None
    return {"adapter": binding["adapter"], "scope": scope, "turn_id": native[0]["turn_id"], "ordinal": native[0]["ordinal"]}


def _add_witness(intent):
    helper = intent.get("helper")
    if not helper or helper["purpose"] != "add" or helper["subject"] is not None or helper["owner_boot"] != intent["owner_boot"]:
        return None
    witness = helper.get("witness")
    if (not witness or witness["kernel_boot"] != helper["kernel_boot"]
            or witness["kernel_boot"] != intent["scope"]["kernel_boot"]
            or witness["pgid"] != witness["process"]["pid"]):
        return None
    return helper


def queue_proof(before: dict, samples: list[dict], expectation: str) -> dict:
    """Judge only the measured interval. Unknown is never positive transport proof.

    The Accepted row, its pre-send matched Ended baseline and a committed Add witness
    may occur in different samples; cleanup replacing helper_json does not erase an
    earlier same-attempt observation. Fulfillment is activity, never a receipt.
    """
    result = {"passed": False, "expectation": expectation, "reasons": [], "attempts": [],
              "observed": {"samples": len(samples), "intents": 0, "armed": 0, "accepted": 0, "unknown": 0, "add_witnesses": 0},
              "transport_inference": "compositional; no native request audit"}
    if expectation not in ("accepted", "none", "native"):
        result["reasons"].append("invalid_expectation")
        return result
    if not samples or len(samples) > MAX_SAMPLES:
        result["reasons"].append("missing_or_excess_samples")
        return result
    if any(not s.get("ok") or s.get("truncated") or not s.get("census_complete") for s in [before, *samples]):
        result["reasons"].append("incomplete_evidence")
        return result
    if any(s.get("seat") != before.get("seat") or s.get("after", 0) > before["watermark"] for s in samples):
        result["reasons"].append("wrong_seat_or_interval_projection_gap")
        return result
    baseline = eligible_scope(before)
    intents = [i for sample in samples for i in sample["intents"] if i["ordinal"] > before["watermark"]]
    attempts = {i["attempt"] for i in intents}
    result["observed"]["intents"] = len(attempts)
    if len(attempts) > MAX_INTENTS:
        result["reasons"].append("attempt_evidence_limit")
        return result
    for phase in ("armed", "accepted", "unknown"):
        result["observed"][phase] = len({i["attempt"] for i in intents if i["phase"] == phase})
    result["observed"]["add_witnesses"] = len({i["attempt"] for i in intents if _add_witness(i)})
    possible = any(i["phase"] in ("armed", "accepted", "unknown") for i in intents)
    if expectation in ("none", "native"):
        if possible:
            result["reasons"].append("possible_effect_during_measured_interval")
        if expectation == "native" and baseline is not None:
            result["reasons"].append("before_scope_was_queue_qualified")
        result["passed"] = not result["reasons"]
        result["transport_inference"] = ("compositional native fallback; external wake check required" if expectation == "native"
                                         else "measured interval only; no lifetime absence claim")
        return result
    if result["observed"]["unknown"]:
        result["reasons"].append("unknown_is_not_positive_evidence")
        return result
    if baseline is None:
        result["reasons"].append("before_scope_not_matched_ended")
        return result
    for attempt in sorted(attempts):
        versions = [i for i in intents if i["attempt"] == attempt]
        accepted = [i for i in versions if i["phase"] == "accepted" and i["arm_nonce"] and i["ids"]
                    and i["adapter"] == baseline["adapter"] and i["scope"] == baseline["scope"]
                    and i["baseline"] == {"turn_id": baseline["turn_id"], "ordinal": baseline["ordinal"]}]
        for intent in reversed(accepted):
            helper = next((_add_witness(i) for i in versions if i["owner_boot"] == intent["owner_boot"]
                           and i["ordinal"] == intent["ordinal"] and i["scope"] == intent["scope"] and _add_witness(i)), None)
            if helper:
                result["attempts"].append({"attempt": attempt, "ordinal": intent["ordinal"], "phase": "accepted",
                    "owner_boot": intent["owner_boot"], "arm_nonce": intent["arm_nonce"], "baseline": intent["baseline"],
                    "owned_id_count": len(intent["ids"]), "owned_ids": [item["id"] for item in intent["ids"][:4]],
                    "add_helper": helper, "logical_status": intent["logical_status"], "fulfillment": intent["fulfillment"],
                    "cleanup_quarantined": intent["cleanup_quarantined"], "cleanup_list_attempted": intent["cleanup_list_attempted"]})
                break
    result["passed"] = bool(result["attempts"])
    if not result["passed"]:
        result["reasons"].append("no_correlated_accepted_with_baseline_and_add_witness")
    result["transport_inference"] = "compositional no-native-typing inference from reviewed Accepted/no-fallback path"
    if len(json.dumps(result, separators=(",", ":")).encode()) > MAX_BYTES:
        result.update(passed=False, attempts=[], reasons=["proof_byte_limit"])
    return result
