"""Strict replay classifier for the Claude native probe, not an authorization API."""

FIELDS = frozenset({
    "harness", "version", "event", "tool_name", "session_id", "tool_use_id",
    "agent_id", "agent_type", "invocation_context", "context_received",
    "generation", "hook_before_tool", "hook_parent_pid", "host_session_id",
    "observer_status",
})
REFERENCE_FIELDS = frozenset({
    "session_id", "generation", "observed_after_request", "host_pid",
    "host_observed_in_hook", "context_expired", "known_replacement",
})


def classify(envelope, reference):
    if not isinstance(envelope, dict) or not isinstance(reference, dict):
        return "UNSUPPORTED"
    if set(envelope) - FIELDS or set(reference) - REFERENCE_FIELDS:
        return "UNSUPPORTED"
    if envelope.get("harness") != "claude" or envelope.get("version") != "2.1.283":
        return "UNSUPPORTED"
    if envelope.get("event") != "PreToolUse" or envelope.get("tool_name") != "Bash":
        return "UNSUPPORTED"
    if reference.get("observed_after_request") is not True:
        return "UNSUPPORTED"
    if reference.get("host_observed_in_hook") is not True:
        return "UNSUPPORTED"
    if reference.get("context_expired") is not False:
        return "UNSUPPORTED"
    if reference.get("known_replacement") is not False:
        return "UNSUPPORTED"
    if not isinstance(reference.get("session_id"), str) or not reference["session_id"]:
        return "UNSUPPORTED"
    if envelope.get("session_id") != reference["session_id"]:
        return "UNSUPPORTED"
    if envelope.get("observer_status") != "ok":
        return "UNSUPPORTED"
    if envelope.get("host_session_id") != reference["session_id"]:
        return "UNSUPPORTED"
    if type(reference.get("host_pid")) is not int or reference["host_pid"] <= 0:
        return "UNSUPPORTED"
    if envelope.get("hook_parent_pid") != reference["host_pid"]:
        return "UNSUPPORTED"
    if type(reference.get("generation")) is not int or envelope.get("generation") != reference["generation"]:
        return "UNSUPPORTED"
    if not isinstance(envelope.get("tool_use_id"), str) or not envelope["tool_use_id"]:
        return "UNSUPPORTED"
    if envelope.get("hook_before_tool") is not True:
        return "UNSUPPORTED"
    context = envelope.get("invocation_context")
    if not isinstance(context, str) or not context or envelope.get("context_received") != context:
        return "UNSUPPORTED"
    if "agent_id" in envelope:
        if not isinstance(envelope["agent_id"], str) or not envelope["agent_id"]:
            return "UNSUPPORTED"
        if not isinstance(envelope.get("agent_type"), str) or not envelope["agent_type"]:
            return "UNSUPPORTED"
        return "CHILD"
    if "agent_type" in envelope:
        return "UNSUPPORTED"
    return "ROOT_CANDIDATE"
