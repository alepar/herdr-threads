//! Version-pinned normalization of Herdr CLI responses.
//! A native session in these responses is display metadata, not current execution proof.

use crate::protocol::{
    ids::{HostTargetId, TerminalId},
    results::{ApiError, ErrorCode},
};
use serde_json::Value;
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativePane {
    pub target: HostTargetId,
    pub terminal_id: String,
    pub workspace_id: String,
    pub tab_id: String,
    pub revision: u64,
    pub status: String,
    /// Herdr's pane focus at read time (spec §10 skips pokes into a focused pane).
    pub focused: bool,
    pub agent: Option<String>,
    /// Herdr may retain this value after native execution replacement.
    pub cached_session: Option<String>,
    /// The pane shell's working directory as Herdr reports it (display
    /// metadata; absent when Herdr omits it).
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeSnapshot {
    pub panes: Vec<NativePane>,
    pub current_execution_proven: bool,
    pub incarnation_proven: bool,
    pub coherent_enumeration_proven: bool,
}

fn invalid(detail: impl Into<String>) -> ApiError {
    ApiError::stale_host_observation(detail)
}

fn field<'a>(value: &'a Value, name: &str) -> Result<&'a str, ApiError> {
    value
        .get(name)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid(format!("missing or invalid {name}")))
}

pub fn structured_host_error(value: &Value) -> Option<ApiError> {
    let host_error = value.get("error")?;
    let host_code = host_error
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let message = host_error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("host error");
    let code = match host_code {
        "permission_denied" | "unauthorized" | "access_denied" => ErrorCode::Unauthorized,
        "unsupported_version" | "version_mismatch" | "protocol_mismatch" => ErrorCode::Unsupported,
        "invalid_request" | "parse_error" | "invalid_json" => ErrorCode::InvalidRequest,
        "agent_pane_busy" | "agent_name_taken" | "agent_blocked" | "target_unsafe" => {
            ErrorCode::TargetUnsafe
        }
        // A definitive answer from a reachable server, not an outage.
        "pane_not_found" | "agent_not_found" => ErrorCode::NotFound,
        "host_unavailable" | "connection_failed" | "socket_unavailable" => {
            ErrorCode::HostUnavailable
        }
        _ => ErrorCode::HostUnavailable,
    };
    Some(ApiError::new(
        code,
        format!(
            "Herdr {}: {}",
            host_code.chars().take(64).collect::<String>(),
            message.chars().take(512).collect::<String>()
        ),
    ))
}

fn envelope(raw: &str, expected_type: &str) -> Result<Value, ApiError> {
    let value: Value = serde_json::from_str(raw).map_err(|_| invalid("invalid host JSON"))?;
    if let Some(error) = structured_host_error(&value) {
        return Err(error);
    }
    let result = value
        .get("result")
        .ok_or_else(|| invalid("missing host result"))?;
    if field(result, "type")? != expected_type {
        return Err(invalid("unexpected host result type"));
    }
    Ok(result.clone())
}

fn parse_pane(value: &Value) -> Result<NativePane, ApiError> {
    let target = HostTargetId::parse(field(value, "pane_id")?).map_err(invalid)?;
    let terminal_id = field(value, "terminal_id")?.to_owned();
    TerminalId::parse(terminal_id.clone()).map_err(invalid)?;
    let workspace_id = field(value, "workspace_id")?.to_owned();
    let tab_id = field(value, "tab_id")?.to_owned();
    let revision = value
        .get("revision")
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid("missing or invalid revision"))?;
    let status = field(value, "agent_status")?.to_owned();
    if !matches!(
        status.as_str(),
        "idle" | "working" | "blocked" | "done" | "unknown"
    ) {
        return Err(invalid("unknown agent status"));
    }
    let focused = value
        .get("focused")
        .and_then(Value::as_bool)
        .ok_or_else(|| invalid("missing focused field"))?;
    let agent = value
        .get("agent")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let cached_session = match value.get("agent_session") {
        None | Some(Value::Null) => None,
        Some(session) => {
            let kind = field(session, "kind")?;
            if !matches!(kind, "id" | "path") {
                return Err(invalid("unknown native session kind"));
            }
            field(session, "agent")?;
            field(session, "source")?;
            Some(field(session, "value")?.to_owned())
        }
    };
    let cwd = value
        .get("cwd")
        .and_then(Value::as_str)
        .filter(|cwd| !cwd.is_empty())
        .map(str::to_owned);
    Ok(NativePane {
        target,
        terminal_id,
        workspace_id,
        tab_id,
        revision,
        status,
        focused,
        agent,
        cached_session,
        cwd,
    })
}

pub fn normalize_snapshot(raw: &str) -> Result<NativeSnapshot, ApiError> {
    let result = envelope(raw, "session_snapshot")?;
    let snapshot = result
        .get("snapshot")
        .ok_or_else(|| invalid("missing snapshot"))?;
    if field(snapshot, "version")? != "0.9.1"
        || snapshot.get("protocol").and_then(Value::as_u64) != Some(22)
    {
        return Err(ApiError::unsupported(
            "unsupported Herdr snapshot version/protocol",
        ));
    }
    for collection in ["workspaces", "tabs", "agents", "layouts"] {
        if !snapshot.get(collection).is_some_and(Value::is_array) {
            return Err(invalid(format!("missing snapshot {collection}")));
        }
    }
    let values = snapshot
        .get("panes")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("missing snapshot panes"))?;
    let mut targets = HashSet::new();
    let mut terminals = HashSet::new();
    let mut panes = Vec::with_capacity(values.len());
    for value in values {
        let pane = parse_pane(value)?;
        if !targets.insert(pane.target.clone()) || !terminals.insert(pane.terminal_id.clone()) {
            return Err(invalid("duplicate pane or terminal identity"));
        }
        panes.push(pane);
    }
    Ok(NativeSnapshot {
        panes,
        current_execution_proven: false,
        incarnation_proven: false,
        coherent_enumeration_proven: false,
    })
}

pub fn normalize_pane(raw: &str, expected_target: &str) -> Result<NativePane, ApiError> {
    let result = envelope(raw, "pane_info")?;
    let pane = parse_pane(result.get("pane").ok_or_else(|| invalid("missing pane"))?)?;
    if pane.target.as_str() != expected_target {
        return Err(invalid("pane response target mismatch"));
    }
    Ok(pane)
}

/// Display names a person can type for a pane: its own label (`herdr pane
/// rename`) and its tab's label. Names are lookup aids only, never identity:
/// the CLI resolves a unique name to the pane ID once, before any request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneName {
    pub target: HostTargetId,
    pub label: Option<String>,
    pub tab_label: Option<String>,
    /// Panes in the same tab; a tab label names a pane only when it is 1.
    pub tab_pane_count: usize,
}

/// Pane IDs with their pane and tab labels from one validated snapshot.
pub fn normalize_pane_names(raw: &str) -> Result<Vec<PaneName>, ApiError> {
    let snapshot = normalize_snapshot(raw)?;
    let value: Value = serde_json::from_str(raw).map_err(|_| invalid("invalid host JSON"))?;
    let label = |item: &Value| {
        item.get("label")
            .and_then(Value::as_str)
            .filter(|label| !label.is_empty())
            .map(str::to_owned)
    };
    let tabs: Vec<(String, Option<String>)> = value
        .pointer("/result/snapshot/tabs")
        .and_then(Value::as_array)
        .map(|tabs| {
            tabs.iter()
                .filter_map(|tab| Some((tab.get("tab_id")?.as_str()?.to_owned(), label(tab))))
                .collect()
        })
        .unwrap_or_default();
    let raw_panes = value
        .pointer("/result/snapshot/panes")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("missing snapshot panes"))?;
    Ok(snapshot
        .panes
        .iter()
        .map(|pane| {
            let raw_pane = raw_panes.iter().find(|raw| {
                raw.get("pane_id").and_then(Value::as_str) == Some(pane.target.as_str())
            });
            PaneName {
                target: pane.target.clone(),
                label: raw_pane.and_then(label),
                tab_label: tabs
                    .iter()
                    .find(|(tab, _)| *tab == pane.tab_id)
                    .and_then(|(_, label)| label.clone()),
                tab_pane_count: snapshot
                    .panes
                    .iter()
                    .filter(|other| other.tab_id == pane.tab_id)
                    .count(),
            }
        })
        .collect())
}

/// Herdr `agent.get` for one pane. Detection-based kind and the
/// integration's agent session are diagnostic/best-effort evidence only.
pub fn normalize_pane_agent(
    raw: &str,
    target: &str,
) -> Result<Option<crate::ports::PaneAgentObservation>, ApiError> {
    let result = envelope(raw, "agent_info")?;
    let agent = result
        .get("agent")
        .ok_or_else(|| invalid("missing agent record"))?;
    if agent.get("pane_id").and_then(Value::as_str) != Some(target) {
        return Err(invalid("agent record names another pane"));
    }
    let kind = agent
        .get("agent")
        .and_then(Value::as_str)
        .filter(|kind| !kind.is_empty())
        .map(str::to_owned);
    let agent_session = match agent.get("agent_session") {
        None | Some(Value::Null) => None,
        Some(session) => Some(field(session, "value")?.to_owned()),
    };
    Ok(Some(crate::ports::PaneAgentObservation {
        kind,
        agent_session,
    }))
}

#[cfg(test)]
mod pane_agent_tests {
    use super::*;
    use crate::ports::PaneAgentObservation;

    fn raw(agent: &str, pane: &str, session: &str) -> String {
        format!(
            r#"{{"id":"x","result":{{"type":"agent_info","agent":{{{agent}"agent_status":"idle","pane_id":"{pane}","terminal_id":"term_1"{session}}}}}}}"#
        )
    }
    const SESSION: &str = r#","agent_session":{"agent":"claude","kind":"id","source":"herdr:claude","value":"sess-1"}"#;

    #[test]
    fn agent_info_with_session_maps_kind_and_session() {
        assert_eq!(
            normalize_pane_agent(&raw(r#""agent":"claude","#, "w4:p1", SESSION), "w4:p1").unwrap(),
            Some(PaneAgentObservation {
                kind: Some("claude".into()),
                agent_session: Some("sess-1".into()),
            })
        );
    }

    #[test]
    fn agent_info_without_session_has_none_session() {
        for session in ["", r#","agent_session":null"#] {
            let parsed =
                normalize_pane_agent(&raw(r#""agent":"claude","#, "w4:p1", session), "w4:p1")
                    .unwrap()
                    .unwrap();
            assert_eq!(parsed.kind.as_deref(), Some("claude"));
            assert_eq!(parsed.agent_session, None, "{session}");
        }
    }

    #[test]
    fn agent_info_without_kind_has_none_kind() {
        let parsed = normalize_pane_agent(&raw("", "w4:p1", SESSION), "w4:p1")
            .unwrap()
            .unwrap();
        assert_eq!(parsed.kind, None);
        assert_eq!(parsed.agent_session.as_deref(), Some("sess-1"));
    }

    #[test]
    fn agent_info_for_another_pane_is_an_error() {
        assert!(
            normalize_pane_agent(&raw(r#""agent":"claude","#, "w4:p2", SESSION), "w4:p1").is_err()
        );
    }

    #[test]
    fn wrong_result_type_is_an_error() {
        let wrong = raw(r#""agent":"claude","#, "w4:p1", "").replace("agent_info", "pane_info");
        assert!(normalize_pane_agent(&wrong, "w4:p1").is_err());
    }
}
