"""Fail-closed replay of the Codex caller evidence contract.

This is a probe classifier, not a production authorization adapter. A native
capture may still be UNSUPPORTED even when these synthetic contract tests pass.
In particular, host.metadata_current is a fixture oracle; the native probe did
not establish a readable freshness signal for the interval after `/new` and
before its first turn.
"""

ALLOWED = frozenset({
    "harness", "version", "event", "session_id", "tool_use_id", "turn_id",
    "agent_id", "agent_type", "execution_id", "nonce", "transport",
    "transport_observed", "thread_source", "root_turn_id", "hook_utc",
    "command_utc", "herdr_env", "workspace_id", "tab_id", "pane_id",
    "transcript_turn_id", "host_session_id", "host_observed_utc",
    "hook_codex_pid", "host_codex_pid", "process_observed_utc", "source",
})


def sanitize(envelope):
    return {key: value for key, value in envelope.items() if key in ALLOWED
            and value is not None and isinstance(value, (str, int, float, bool))}


def classify(call, host):
    if not isinstance(call, dict) or not isinstance(host, dict):
        return "UNSUPPORTED"
    if call.get("harness") != "codex" or call.get("version") != "0.157.1":
        return "UNSUPPORTED"
    if call.get("event") != "PreToolUse" or not all(
        isinstance(call.get(key), str) and call[key]
        for key in ("session_id", "turn_id", "execution_id", "tool_use_id", "nonce")
    ):
        return "UNSUPPORTED"
    if call.get("transport") != "rewritten-command" or call.get("transport_observed") is not True:
        return "UNSUPPORTED"
    available = call.get("evidence_available_ms")
    hook_return = call.get("hook_return_ms")
    if (type(available) is not int or type(hook_return) is not int
            or available > hook_return):
        return "UNSUPPORTED"
    if call.get("transcript_turn_id") != call["turn_id"]:
        return "UNSUPPORTED"
    if call.get("thread_source") not in ("cli", "subagent"):
        return "UNSUPPORTED"
    if not isinstance(call.get("hook_codex_pid"), int) or call["hook_codex_pid"] <= 0:
        return "UNSUPPORTED"
    if call.get("hook_codex_pid") != call.get("host_codex_pid"):
        return "UNSUPPORTED"
    if not host.get("observation_after_request") or host.get("metadata_current") is not True:
        return "UNSUPPORTED"
    issued = host.get("context_issued_ms")
    expires = host.get("context_expires_ms")
    decision = host.get("decision_ms")
    if (any(type(value) is not int for value in (issued, expires, decision))
            or not 0 < expires - issued <= 250
            or not issued <= decision <= expires):
        return "UNSUPPORTED"
    generation = host.get("generation")
    if not isinstance(generation, int) or generation != host.get("observed_generation"):
        return "UNSUPPORTED"
    if call["session_id"] != host.get("session_id"):
        return "STALE"
    if call.get("host_session_id") != call["session_id"]:
        return "STALE"
    if call["hook_codex_pid"] != host.get("codex_pid"):
        return "STALE"
    if call.get("agent_id") or call["thread_source"] == "subagent":
        return "CHILD_OR_UNKNOWN"
    if call["execution_id"] != host.get("root_execution_id"):
        return "CHILD_OR_UNKNOWN"
    if call.get("root_turn_id") != call["turn_id"]:
        return "UNSUPPORTED"
    return "SUPPORTED_ROOT"
