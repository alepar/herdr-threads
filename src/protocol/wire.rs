use super::{
    commands::Command,
    output::OutputSpec,
    results::{ApiError, CommandResult},
};
use serde::{Deserialize, Deserializer, Serialize};
use std::io::{self, Write};

/// Wire protocol version. A daemon and a client speak only the same version.
/// 1: the original request/response envelope.
/// 2: `WireRequest.expected_boot`, the B5 commands `OperatorRetire`,
///    `OperatorReplace`, `OperatorCheckIn` and `ContinuityCheckIn`, and the
///    `DaemonBootChanged` refusal; the remaining-findings epic (ht-p03) added
///    the `Capabilities` and `HookParseFailure` commands, the `Capabilities`
///    result and `HistoryQuery.full_bodies` within version 2 (unreleased
///    then). A version-1 daemon rejects a version-2 request at decode, so the
///    descriptor check reports the skew first.
/// 3: the thread-summary commands `HotThreads`, `Summary`, `SummaryJob` and
///    `SummarySubmit`, the `relays_user` field on the `SendMessage` request (a
///    `deny_unknown_fields` struct) and their result shapes (epic ht-1ip). The
///    harness version evidence epic (ht-xoc) added the capability-gated
///    `HarnessEvidence` and `HarnessStates` commands and their results
///    (capabilities `hook.harness_evidence`, `harness.states`) within version
///    2 on main; version 3 carries both sets. A version-2 daemon rejects a
///    version-3 request at decode, so the descriptor check reports the skew
///    first.
/// 4: optional thread names, indexed selector resolution and name controls.
///    Shipped v0.2.1 result structs deny unknown fields: descriptor/version
///    fences refuse older peers before dispatch. Stored results remain readable.
/// 5: optional recorded user intent and rule-change evidence in messages and summaries.
/// 6: canonical compound handoff fences and the quiet-channel lifecycle lane.
pub const PROTOCOL_VERSION: u16 = 6;
pub const MAX_WIRE_FRAME_BYTES: usize = 1_048_576;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WireRequest {
    pub version: u16,
    pub request_id: String,
    pub expected_instance: String,
    /// The daemon boot the client read from the descriptor. A daemon running a
    /// different boot refuses the request before dispatch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_boot: Option<String>,
    /// Presentation only. Omitting it preserves the original JSON read mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<OutputSpec>,
    pub command: Command,
}

impl<'de> Deserialize<'de> for WireRequest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            version: u16,
            request_id: String,
            expected_instance: String,
            #[serde(default)]
            expected_boot: Option<String>,
            #[serde(default)]
            output: Option<OutputSpec>,
            command: serde_json::Value,
        }
        let raw = Raw::deserialize(deserializer)?;
        if raw.version != PROTOCOL_VERSION {
            return Err(serde::de::Error::custom("unknown wire version"));
        }
        if !valid_wire_id(&raw.request_id) || !valid_uuid(&raw.expected_instance) {
            return Err(serde::de::Error::custom("invalid request or instance id"));
        }
        if raw
            .expected_boot
            .as_deref()
            .is_some_and(|boot| !valid_uuid(boot))
        {
            return Err(serde::de::Error::custom("invalid expected boot"));
        }
        let fields = raw
            .command
            .as_object()
            .ok_or_else(|| serde::de::Error::custom("command must be an object"))?;
        if !fields.contains_key("kind") || fields.keys().any(|key| key != "kind" && key != "args") {
            return Err(serde::de::Error::custom("unknown command envelope field"));
        }
        if fields.get("kind").and_then(serde_json::Value::as_str) == Some("harness_health_v2")
            && fields.contains_key("args")
        {
            return Err(serde::de::Error::custom(
                "harness health v2 takes no arguments",
            ));
        }
        let command: Command =
            serde_json::from_value(raw.command).map_err(serde::de::Error::custom)?;
        command.validate().map_err(serde::de::Error::custom)?;
        if let Some(output) = &raw.output {
            output.validate().map_err(serde::de::Error::custom)?;
        }
        Ok(Self {
            version: raw.version,
            request_id: raw.request_id,
            expected_instance: raw.expected_instance,
            expected_boot: raw.expected_boot,
            output: raw.output,
            command,
        })
    }
}

fn valid_wire_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && value.bytes().all(|b| b.is_ascii_graphic())
}
fn valid_uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| id.hyphenated().to_string() == value)
}
impl WireRequest {
    pub fn decode(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WireResponse {
    pub version: u16,
    pub request_id: String,
    pub instance: String,
    pub daemon_boot: String,
    pub result: Result<CommandResult, ApiError>,
}
impl<'de> Deserialize<'de> for WireResponse {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            version: u16,
            request_id: String,
            instance: String,
            daemon_boot: String,
            result: Result<CommandResult, ApiError>,
        }
        let raw = Raw::deserialize(deserializer)?;
        if !valid_wire_id(&raw.request_id)
            || !valid_uuid(&raw.instance)
            || !valid_uuid(&raw.daemon_boot)
            || !result_identity_valid(&raw.result, &raw.instance, &raw.daemon_boot)
        {
            return Err(serde::de::Error::custom("invalid response identity"));
        }
        Ok(Self {
            version: raw.version,
            request_id: raw.request_id,
            instance: raw.instance,
            daemon_boot: raw.daemon_boot,
            result: raw.result,
        })
    }
}
impl WireResponse {
    pub fn correlates_to(&self, request: &WireRequest, expected_boot: Option<&str>) -> bool {
        self.version == PROTOCOL_VERSION
            && self.request_id == request.request_id
            && self.instance == request.expected_instance
            && expected_boot.is_none_or(|boot| self.daemon_boot == boot)
            && valid_wire_id(&self.request_id)
            && valid_uuid(&self.instance)
            && valid_uuid(&self.daemon_boot)
    }

    pub fn version_mismatch(
        request_id: String,
        instance: String,
        daemon_boot: String,
        daemon_version: &str,
    ) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            request_id,
            instance,
            daemon_boot,
            result: Err(ApiError::daemon_version_mismatch(daemon_version.to_owned())),
        }
    }
}

/// Four-byte big-endian length plus one bounded JSON response frame.
pub fn encode_wire_response(response: &WireResponse) -> Result<Vec<u8>, ApiError> {
    if !result_identity_valid(&response.result, &response.instance, &response.daemon_boot) {
        return Err(ApiError::invalid_request(
            "response result identity or bounds invalid",
        ));
    }
    let mut writer = BoundedJsonWriter::new(MAX_WIRE_FRAME_BYTES - 4);
    serde_json::to_writer(&mut writer, response)
        .map_err(|error| ApiError::invalid_request(format!("response encoding failed: {error}")))?;
    let total = writer
        .count
        .checked_add(4)
        .ok_or_else(|| ApiError::invalid_budget("wire frame size overflow"))?;
    if total > MAX_WIRE_FRAME_BYTES {
        let mut error = ApiError::invalid_budget("wire frame exceeds 1 MiB");
        error.required_minimum_bytes = u32::try_from(total).ok();
        return Err(error);
    }
    let mut frame = Vec::with_capacity(total);
    frame.extend_from_slice(&((total - 4) as u32).to_be_bytes());
    frame.extend_from_slice(&writer.bytes);
    Ok(frame)
}

fn result_identity_valid(
    result: &Result<CommandResult, ApiError>,
    instance: &str,
    boot: &str,
) -> bool {
    match result {
        Ok(CommandResult::Health(health)) => {
            health.instance_id == instance && health.boot_id == boot && health.validate().is_ok()
        }
        Ok(CommandResult::StopAccepted(accepted)) => accepted.boot_id == boot,
        Ok(CommandResult::ServiceInspection(inspection)) => {
            inspection.instance == instance
                && inspection.daemon_boot == boot
                && inspection.connection_generation.is_some() == inspection.registered_at.is_some()
                && inspection
                    .connection_generation
                    .is_none_or(|generation| generation > 0)
        }
        Ok(CommandResult::ServiceDisconnected(disconnect)) => {
            disconnect.instance == instance
                && disconnect.daemon_boot == boot
                && disconnect.connection_generation > 0
                && disconnect.disconnected
        }
        _ => true,
    }
}

struct BoundedJsonWriter {
    bytes: Vec<u8>,
    limit: usize,
    count: usize,
}
impl BoundedJsonWriter {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
            count: 0,
        }
    }
}
impl Write for BoundedJsonWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.count = self
            .count
            .checked_add(buf.len())
            .ok_or_else(|| io::Error::other("wire length overflow"))?;
        let retain = buf.len().min(self.limit.saturating_sub(self.bytes.len()));
        self.bytes.extend_from_slice(&buf[..retain]);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod thread_name_wire_tests {
    use super::*;
    #[test]
    fn channel_wire6_rejects_old_compound_steps_before_command_decode() {
        for version in [4, 5] {
            for kind in ["create_thread", "invite", "send_message"] {
                let error = serde_json::from_value::<WireRequest>(serde_json::json!({
                    "version":version,"request_id":"request","expected_instance":"00000000-0000-4000-8000-000000000001",
                    "command":{"kind":kind,"args":{"user_intent":"invalid"}}
                })).unwrap_err();
                assert!(
                    error.to_string().contains("unknown wire version"),
                    "{version}/{kind}: {error}"
                );
            }
        }
    }
}
