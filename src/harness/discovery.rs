//! Same-binary metadata discovery. No installation, daemon, or native calls.
use super::{adapter::*, evidence::EvidenceOrigin, registry::Registry};
use serde::{Deserialize, Serialize};

pub const MAX_DISCOVERY_BYTES: usize = 65_536;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Discovery {
    pub schema_version: u8,
    pub adapters: Vec<AdapterEntry>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterEntry {
    pub id: String,
    pub display_name: String,
    pub host_kinds: Vec<String>,
    pub setup_scopes: Vec<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub legacy_contract_id: Option<String>,
    pub contracts: Vec<DomainContract>,
    #[serde(deserialize_with = "required_nullable")]
    pub canary_strategy: Option<CanaryDescriptor>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DomainContract {
    pub domain: String,
    pub origin: EvidenceOrigin,
    pub id: String,
    pub events: Vec<DomainEvent>,
    pub required_milestones: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DomainEvent {
    pub event: String,
    #[serde(deserialize_with = "required_nullable")]
    pub milestone: Option<String>,
    pub always_send: bool,
}
pub fn render(registry: &Registry) -> Result<String, String> {
    if registry.registrations().len() > 64 {
        return Err("discovery adapter limit exceeded".into());
    }
    let adapters = registry
        .registrations()
        .iter()
        .map(|registration| {
            let metadata = registration.metadata();
            if !super::runtime::printable(metadata.display_label, 128)
                || metadata.host_kinds.len() > 8
                || metadata.setup_scopes.len() > 2
                || metadata.host_kinds.iter().any(|s| !id(s, 64))
            {
                return Err("invalid adapter discovery metadata".into());
            }
            Ok(AdapterEntry {
                id: metadata.id.into(),
                display_name: metadata.display_label.into(),
                host_kinds: metadata.host_kinds.iter().map(|s| (*s).into()).collect(),
                setup_scopes: metadata
                    .setup_scopes
                    .iter()
                    .map(|s| match s {
                        SetupScopeKind::ConfigRoot => "config_root".into(),
                        SetupScopeKind::Profile => "profile".into(),
                    })
                    .collect(),
                legacy_contract_id: registration.legacy_contract_id(),
                contracts: registration
                    .contracts()
                    .iter()
                    .map(|d| {
                        Ok(DomainContract {
                            domain: d.domain_id.into(),
                            origin: d.origin,
                            id: d.contract_id_v2()?,
                            events: d
                                .events
                                .iter()
                                .map(|e| DomainEvent {
                                    event: e.native_event.into(),
                                    milestone: e.milestone.map(String::from),
                                    always_send: e.always_send,
                                })
                                .collect(),
                            required_milestones: d
                                .required_milestones
                                .iter()
                                .map(|s| (*s).into())
                                .collect(),
                        })
                    })
                    .collect::<Result<_, String>>()?,
                canary_strategy: registration.canary_strategy().map(|s| s.descriptor()),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let document = Discovery {
        schema_version: 1,
        adapters,
    };
    document.validate()?;
    Ok(serde_json::to_string(&document).map_err(|e| e.to_string())? + "\n")
}
impl Discovery {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_DISCOVERY_BYTES {
            return Err("discovery exceeds byte limit".into());
        }
        let document: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        document.validate()?;
        Ok(document)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 || self.adapters.len() > 64 {
            return Err("unsupported or unbounded discovery".into());
        }
        let mut ids = std::collections::HashSet::new();
        for adapter in &self.adapters {
            if !id(&adapter.id, 64)
                || !ids.insert(&adapter.id)
                || !super::runtime::printable(&adapter.display_name, 128)
                || adapter.host_kinds.len() > 8
                || !unique(&adapter.host_kinds)
                || adapter.host_kinds.iter().any(|s| !id(s, 64))
                || adapter.setup_scopes.len() > 2
                || !unique(&adapter.setup_scopes)
                || adapter
                    .setup_scopes
                    .iter()
                    .any(|s| !matches!(s.as_str(), "config_root" | "profile"))
                || adapter
                    .legacy_contract_id
                    .as_ref()
                    .is_some_and(|s| !hash(s))
                || adapter.contracts.len() > 8
            {
                return Err("invalid adapter discovery metadata".into());
            }
            let mut domains = std::collections::HashSet::new();
            for contract in &adapter.contracts {
                if !super::evidence::valid_name(&contract.domain)
                    || !domains.insert(&contract.domain)
                    || !hash(&contract.id)
                    || contract.events.len() > 32
                    || contract.required_milestones.len() > 8
                    || !unique(&contract.required_milestones)
                    || contract.required_milestones.iter().any(|m| {
                        !super::evidence::valid_name(m)
                            || !contract
                                .events
                                .iter()
                                .any(|e| e.milestone.as_ref() == Some(m))
                    })
                {
                    return Err("invalid discovery domain".into());
                }
                let mut events = std::collections::HashSet::new();
                for event in &contract.events {
                    if !event_name(&event.event)
                        || !events.insert(&event.event)
                        || event
                            .milestone
                            .as_ref()
                            .is_some_and(|m| !super::evidence::valid_name(m))
                    {
                        return Err("invalid discovery event".into());
                    }
                }
            }
            if let Some(strategy) = &adapter.canary_strategy {
                strategy.validate()?;
            }
        }
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        if bytes.len() + 1 > MAX_DISCOVERY_BYTES {
            return Err("discovery exceeds byte limit".into());
        }
        Ok(())
    }
}

fn required_nullable<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d)
}
fn unique(values: &[String]) -> bool {
    let mut seen = std::collections::HashSet::new();
    values.iter().all(|s| seen.insert(s))
}
fn id(s: &str, max: usize) -> bool {
    !s.is_empty()
        && s.len() <= max
        && s.as_bytes()[0].is_ascii_lowercase()
        && s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_-".contains(&c))
}
fn hash(s: &str) -> bool {
    s.len() == 16
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn event_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 63
        && s.as_bytes()[0].is_ascii_alphabetic()
        && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
}
impl CanaryDescriptor {
    pub fn validate(&self) -> Result<(), String> {
        let path = self.companion.strip_prefix("scripts/canary/adapters/");
        if self.artifact_schema_version != 1
            || self.companion.len() > 256
            || path.is_none_or(|tail| {
                tail.is_empty()
                    || tail.split('/').any(|part| {
                        part.is_empty()
                            || matches!(part, "." | "..")
                            || !part
                                .bytes()
                                .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
                    })
            })
            || self.model_key_env.as_ref().is_some_and(|s| {
                s.is_empty()
                    || s.len() > 64
                    || !s.as_bytes()[0].is_ascii_uppercase()
                    || !s
                        .bytes()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_')
            })
        {
            return Err("invalid canary strategy metadata".into());
        }
        match (&self.kind, &self.candidate_kind, &self.npm_package) {
            (CanaryKind::NpmRelease, CandidateKind::StableRelease, Some(package))
                if npm_package(package) =>
            {
                Ok(())
            }
            (CanaryKind::ExactRuntime, CandidateKind::ExactBuild, None) => Ok(()),
            _ => Err("inconsistent canary candidate strategy".into()),
        }
    }
}
fn npm_package(s: &str) -> bool {
    if s.len() > 128 {
        return false;
    }
    if let Some(scoped) = s.strip_prefix('@') {
        let parts: Vec<_> = scoped.split('/').collect();
        parts.len() == 2 && parts.iter().all(|s| id(s, 64))
    } else {
        id(s, 64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> serde_json::Value {
        serde_json::json!({"schema_version":1,"adapters":[{"id":"test","display_name":"Test",
            "host_kinds":[],"setup_scopes":["profile"],"legacy_contract_id":null,"contracts":[],
            "canary_strategy":{"kind":"exact_runtime","candidate_kind":"exact_build","npm_package":null,
                "model_key_env":null,"companion":"scripts/canary/adapters/test.py","artifact_schema_version":1}}]})
    }
    // Catches shell/traversal companions, strategy kind confusion, and unbounded discovery.
    #[test]
    fn discovery_refuses_invalid_paths_strategies_and_limits() {
        for path in [
            "/tmp/probe",
            "scripts/canary/adapters/../probe.py",
            "scripts/canary/adapters/a;echo.py",
            "scripts/canary/adapters//probe.py",
            "scripts/canary/adapters/a\\\\b.py",
            "scripts/canary/adapters/",
        ] {
            let mut value = fixture();
            value["adapters"][0]["canary_strategy"]["companion"] = path.into();
            assert!(
                Discovery::parse(value.to_string().as_bytes())
                    .and_then(|d| d.validate())
                    .is_err(),
                "{path}"
            );
        }
        let mut value = fixture();
        value["adapters"][0]["canary_strategy"]["candidate_kind"] = "stable_release".into();
        assert!(
            Discovery::parse(value.to_string().as_bytes())
                .and_then(|d| d.validate())
                .is_err()
        );
        let mut value = fixture();
        value["adapters"][0]["display_name"] = "a".repeat(129).into();
        assert!(
            Discovery::parse(value.to_string().as_bytes())
                .and_then(|d| d.validate())
                .is_err()
        );
        let mut value = fixture();
        let duplicate = value["adapters"][0].clone();
        value["adapters"].as_array_mut().unwrap().push(duplicate);
        assert!(
            Discovery::parse(value.to_string().as_bytes())
                .and_then(|d| d.validate())
                .is_err()
        );
        let mut bytes = fixture().to_string().into_bytes();
        bytes.extend(vec![b' '; MAX_DISCOVERY_BYTES]);
        assert!(Discovery::parse(&bytes).is_err());
    }
    #[test]
    fn discovery_parser_refuses_unknown_fields_and_accepts_explicit_no_provider() {
        let mut value = fixture();
        value["adapters"][0]["canary_strategy"] = serde_json::Value::Null;
        Discovery::parse(value.to_string().as_bytes())
            .unwrap()
            .validate()
            .unwrap();
        for extra in ["verified", "installed", "model_pass"] {
            let mut invalid = value.clone();
            invalid["adapters"][0][extra] = true.into();
            assert!(Discovery::parse(invalid.to_string().as_bytes()).is_err());
        }
        let mut invalid = value;
        invalid["adapters"][0]
            .as_object_mut()
            .unwrap()
            .remove("canary_strategy");
        assert!(Discovery::parse(invalid.to_string().as_bytes()).is_err());
    }
}
