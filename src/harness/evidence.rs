//! Adapter-declared evidence domains. Observer claims never grant receipt authority.
use super::{
    adapter::ContractDescriptor,
    contract::{self, EventClass},
    runtime::{printable, token},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceOrigin {
    NativePayload,
    NativeShapeObservation,
    BridgeEnvelope,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributionHolding {
    Never,
    UntilAttributed,
    SuppressResumed,
}
#[derive(Debug, Clone, Copy)]
pub struct EvidenceEvent {
    pub native_event: &'static str,
    pub milestone: Option<&'static str>,
    pub always_send: bool,
}
pub const LEGACY_EVENTS: &[EvidenceEvent] = &[
    EvidenceEvent {
        native_event: "SessionStart",
        milestone: Some("lifecycle"),
        always_send: true,
    },
    EvidenceEvent {
        native_event: "PreToolUse",
        milestone: Some("tool"),
        always_send: false,
    },
];
pub fn valid_name(name: &str) -> bool {
    token(name, 32) && name.as_bytes()[0].is_ascii_lowercase()
}
fn valid_event(name: &str) -> bool {
    name.len() <= 63
        && name.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
fn unique<'a>(mut names: impl Iterator<Item = &'a str>) -> bool {
    let mut seen = std::collections::HashSet::new();
    names.all(|name| seen.insert(name))
}
impl ContractDescriptor {
    pub fn validate(&self) -> Result<(), String> {
        if self
            .resumed_unavailable_reason
            .is_some_and(|reason| !printable(reason, 128))
            || (self.holding == AttributionHolding::SuppressResumed)
                != self.resumed_unavailable_reason.is_some()
        {
            return Err("invalid resumed attribution policy".into());
        }
        if !valid_name(self.domain_id)
            || !printable(self.contract.harness, 64)
            || !printable(self.contract.discriminator, 128)
            || self.contract.events.len() > 32
            || self.events.len() != self.contract.events.len()
            || self.events.len() > 32
            || self.required_milestones.len() > 8
            || self.qualifications.len() > 8
        {
            return Err("unbounded or invalid domain declaration".into());
        }
        if !unique(self.contract.events.iter().map(|e| e.event))
            || !unique(self.events.iter().map(|e| e.native_event))
            || !unique(self.required_milestones.iter().copied())
            || !unique(self.qualifications.iter().copied())
        {
            return Err("duplicate domain declaration".into());
        }
        for event in self.contract.events {
            if !valid_event(event.event)
                || event.fields.len() > 64
                || !unique(event.fields.iter().map(|f| f.path))
                || event.fields.iter().any(|f| !printable(f.path, 128))
            {
                return Err("invalid contract event declaration".into());
            }
        }
        for event in self.events {
            if !valid_event(event.native_event)
                || self
                    .contract
                    .events
                    .iter()
                    .all(|e| e.event != event.native_event)
                || event.milestone.is_some_and(|m| !valid_name(m))
            {
                return Err("undeclared evidence event or milestone".into());
            }
        }
        if self.qualifications.iter().any(|q| !valid_name(q))
            || self
                .required_milestones
                .iter()
                .any(|m| !valid_name(m) || self.events.iter().all(|e| e.milestone != Some(m)))
        {
            return Err("invalid or undeclared required metadata".into());
        }
        Ok(())
    }
    pub fn event(&self, native_event: &str) -> Option<&EvidenceEvent> {
        self.events.iter().find(|e| e.native_event == native_event)
    }
    /// Empty required sets and unqualified observations can never verify a domain.
    pub fn verified(&self, successful_milestones: &[&str], qualifications: &[&str]) -> bool {
        self.validate().is_ok()
            && !self.required_milestones.is_empty()
            && self
                .required_milestones
                .iter()
                .all(|m| successful_milestones.contains(m))
            && self
                .qualifications
                .iter()
                .all(|q| qualifications.contains(q))
    }
    pub fn may_hold(&self, resumed: bool) -> bool {
        match self.holding {
            AttributionHolding::Never => false,
            AttributionHolding::UntilAttributed => true,
            AttributionHolding::SuppressResumed => !resumed,
        }
    }
    pub fn legacy_contract_id(&self) -> String {
        contract::contract_id(self.contract)
    }
    pub fn canonical_json_v2(&self) -> Result<String, String> {
        self.validate()?;
        let mut events: Vec<_> = self.events.iter().collect();
        events.sort_by_key(|e| e.native_event);
        let events: Vec<_> = events.into_iter().map(|e| {
            let declared = self.contract.events.iter().find(|d| d.event == e.native_event).expect("validated declaration");
            let class = match declared.class { EventClass::Lifecycle => "lifecycle", EventClass::Tool => "tool", EventClass::Other => "other" };
            serde_json::json!({"event": e.native_event, "class":class, "milestone":e.milestone,"always_send":e.always_send})
        }).collect();
        let mut milestones = self.required_milestones.to_vec();
        milestones.sort_unstable();
        let mut qualifications = self.qualifications.to_vec();
        qualifications.sort_unstable();
        let declaration: serde_json::Value =
            serde_json::from_str(&contract::canonical_json(self.contract))
                .expect("canonical contract JSON");
        Ok(serde_json::json!({"schema_version":2,"harness":self.contract.harness,"domain":self.domain_id,
            "origin":self.origin,"declaration":declaration,"events":events,"required_milestones":milestones,
            "qualifications":qualifications,"holding":self.holding,"resumed_unavailable_reason":self.resumed_unavailable_reason}).to_string())
    }
    pub fn contract_id_v2(&self) -> Result<String, String> {
        use sha2::{Digest, Sha256};
        Ok(format!("{:x}", Sha256::digest(self.canonical_json_v2()?.as_bytes()))[..16].into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::adapter::HarnessAdapter;
    #[test]
    fn codex_subagent_event_is_declared_without_verifying_a_milestone() {
        let descriptor = crate::harness::codex::CodexAdapter.contracts()[0];
        let event = descriptor
            .event("SubagentStart")
            .expect("known events need server descriptor metadata");
        assert_eq!(event.milestone, None);
        assert!(!descriptor.verified(&[], &[]));
    }
    #[test]
    fn domains_require_all_declared_milestones_and_qualifications() {
        let mut descriptor = crate::harness::claude::ClaudeAdapter.contracts()[0];
        assert!(!descriptor.verified(&["lifecycle"], &[]));
        assert!(!descriptor.verified(&["tool"], &[]));
        assert!(descriptor.verified(&["lifecycle", "tool"], &[]));
        descriptor.qualifications = &["same_runtime"];
        assert!(!descriptor.verified(&["lifecycle", "tool"], &[]));
        assert!(descriptor.verified(&["lifecycle", "tool"], &["same_runtime"]));
        descriptor.required_milestones = &[];
        assert!(!descriptor.verified(&["lifecycle", "tool"], &["same_runtime"]));
    }
    #[test]
    fn domain_metadata_rejects_unbounded_undeclared_or_duplicate_names() {
        let original = crate::harness::claude::ClaudeAdapter.contracts()[0];
        assert!(original.validate().is_ok());
        for name in ["", "Bad", "a-b", "abcdefghijklmnopqrstuvwxyzabcdefg", "a\n"] {
            let mut descriptor = original;
            descriptor.domain_id = name;
            assert!(descriptor.validate().is_err());
        }
        for milestones in [
            &["unknown"][..],
            &["lifecycle", "lifecycle"][..],
            &["tool"; 9][..],
        ] {
            let mut descriptor = original;
            descriptor.required_milestones = milestones;
            assert!(descriptor.validate().is_err());
        }
        let mut descriptor = original;
        descriptor.events = &[EvidenceEvent {
            native_event: "Unknown",
            milestone: Some("tool"),
            always_send: false,
        }];
        assert!(descriptor.validate().is_err());
    }
    #[test]
    fn ninth_distinct_declared_required_milestone_exceeds_the_bound() {
        use crate::harness::contract::{EventContract, HarnessContract};
        let names: &'static [&str] = &["a", "b", "c", "d", "e", "f", "g", "h", "i"];
        let events = Box::leak(
            names
                .iter()
                .map(|name| EventContract {
                    event: name,
                    class: EventClass::Other,
                    fields: &[],
                })
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        );
        let mut descriptor = crate::harness::claude::ClaudeAdapter.contracts()[0];
        descriptor.contract = Box::leak(Box::new(HarnessContract {
            harness: "claude",
            discriminator: "event",
            events,
        }));
        descriptor.events = Box::leak(
            names
                .iter()
                .map(|name| EvidenceEvent {
                    native_event: name,
                    milestone: Some(name),
                    always_send: false,
                })
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        );
        descriptor.required_milestones = &names[..8];
        assert!(descriptor.verified(&names[..8], &[]));
        descriptor.required_milestones = names;
        assert!(!descriptor.verified(names, &[]));
        assert!(descriptor.validate().is_err());
    }
    #[test]
    fn legacy_holding_policy_refuses_resumed_codex_credit() {
        let codex = crate::harness::codex::CodexAdapter.contracts()[0];
        let claude = crate::harness::claude::ClaudeAdapter.contracts()[0];
        assert!(codex.may_hold(false));
        assert!(!codex.may_hold(true));
        assert!(claude.may_hold(true));
    }
}
