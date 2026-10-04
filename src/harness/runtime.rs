//! Exact, bounded runtime attribution. Installation observations are not runtime evidence.
use super::recipe::Version;
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

pub const MAX_DISTANCE: u64 = u32::MAX as u64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeDescriptor {
    pub release_version: Option<String>,
    pub source: String,
    pub base_version: Option<String>,
    pub derived_version: Option<String>,
    pub commit: Option<String>,
    pub dirty: Option<bool>,
    pub distance: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeIdentity {
    pub key: String,
    #[serde(flatten)]
    pub descriptor: RuntimeDescriptor,
}

impl std::ops::Deref for RuntimeIdentity {
    type Target = RuntimeDescriptor;
    fn deref(&self) -> &Self::Target {
        &self.descriptor
    }
}

impl<'de> Deserialize<'de> for RuntimeIdentity {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Flatten with deny_unknown_fields on the descriptor is not supported by serde.
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            key: String,
            release_version: Option<String>,
            source: String,
            base_version: Option<String>,
            derived_version: Option<String>,
            commit: Option<String>,
            dirty: Option<bool>,
            distance: Option<u64>,
        }
        let w = Wire::deserialize(deserializer)?;
        let identity = Self {
            key: w.key,
            descriptor: RuntimeDescriptor {
                release_version: w.release_version,
                source: w.source,
                base_version: w.base_version,
                derived_version: w.derived_version,
                commit: w.commit,
                dirty: w.dirty,
                distance: w.distance,
            },
        };
        identity.validate().map_err(serde::de::Error::custom)?;
        Ok(identity)
    }
}

pub(crate) fn printable(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}
pub(crate) fn token(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}
impl RuntimeDescriptor {
    pub fn validate(&self) -> Result<(), String> {
        if !token(&self.source, 32) {
            return Err("invalid runtime source".into());
        }
        for value in [
            &self.release_version,
            &self.base_version,
            &self.derived_version,
            &self.commit,
        ]
        .into_iter()
        .flatten()
        {
            if !printable(value, 128) {
                return Err("invalid runtime descriptor string".into());
            }
        }
        if self.commit.as_ref().is_some_and(|v| {
            v.len() != 40
                || !v
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }) {
            return Err("invalid runtime commit".into());
        }
        if self.distance.is_some_and(|v| v > MAX_DISTANCE) {
            return Err("runtime distance exceeds bound".into());
        }
        Ok(())
    }
    /// Versioned JSON with byte-sorted keys and explicit nulls; key itself is excluded.
    pub fn canonical_json(&self) -> String {
        serde_json::json!({"schema_version":1,"release_version":self.release_version,"source":self.source,
            "base_version":self.base_version,"derived_version":self.derived_version,"commit":self.commit,
            "dirty":self.dirty,"distance":self.distance}).to_string()
    }
    fn build_key(&self) -> String {
        format!(
            "build:{:x}",
            Sha256::digest(self.canonical_json().as_bytes())
        )
    }
    fn stable(&self) -> bool {
        self.release_version
            .as_deref()
            .is_some_and(|v| Version::parse(v).is_some())
            && self.dirty != Some(true)
            && self.distance.unwrap_or(0) == 0
            && self
                .base_version
                .as_ref()
                .is_none_or(|v| Some(v) == self.release_version.as_ref())
            && self
                .derived_version
                .as_ref()
                .is_none_or(|v| Some(v) == self.release_version.as_ref())
            && self.commit.is_none()
    }
}
impl RuntimeIdentity {
    /// Only an adapter establishing stable attribution may call this constructor.
    pub fn stable_release(version: &str, source: &str) -> Result<Self, String> {
        let descriptor = RuntimeDescriptor {
            release_version: Some(version.into()),
            source: source.into(),
            base_version: None,
            derived_version: None,
            commit: None,
            dirty: None,
            distance: None,
        };
        let identity = Self {
            key: format!("release:{version}"),
            descriptor,
        };
        identity.validate()?;
        Ok(identity)
    }
    pub fn build(descriptor: RuntimeDescriptor) -> Result<Self, String> {
        descriptor.validate()?;
        Ok(Self {
            key: descriptor.build_key(),
            descriptor,
        })
    }
    pub fn validate(&self) -> Result<(), String> {
        self.descriptor.validate()?;
        if self.key.starts_with("release:") {
            if !self.descriptor.stable()
                || self.key != format!("release:{}", self.release_version.as_deref().unwrap_or(""))
            {
                return Err("runtime release key does not match stable descriptor".into());
            }
        } else if self.key != self.descriptor.build_key() {
            return Err("runtime build key does not match descriptor".into());
        }
        Ok(())
    }
    /// Build release text is informational and must never enter release ordering/lookup.
    pub fn release(&self) -> Option<&str> {
        self.validate().ok()?;
        self.key.strip_prefix("release:")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn development() -> RuntimeDescriptor {
        RuntimeDescriptor {
            release_version: Some("0.21.5".into()),
            source: "git".into(),
            base_version: Some("0.21.5".into()),
            derived_version: Some("0.21.5+3962.g37daf85".into()),
            commit: Some("37daf85b2ad0ee50ed45d7234dc47b7fa24cec09".into()),
            dirty: Some(false),
            distance: Some(3962),
        }
    }
    #[test]
    fn stable_release_keys_cannot_describe_development_builds() {
        let release = RuntimeIdentity::stable_release("0.21.5", "native_transcript").unwrap();
        assert_eq!(release.release(), Some("0.21.5"));
        let build = RuntimeIdentity::build(development()).unwrap();
        assert_eq!(build.release(), None);
        let mut forged = build.clone();
        forged.key = "release:0.21.5".into();
        assert!(forged.validate().is_err());
        assert!(
            serde_json::from_value::<RuntimeIdentity>(serde_json::to_value(&forged).unwrap())
                .is_err()
        );
        for text in ["0.21.5+dev", "01.2.3", "v1.2.3", "1.2", "1.2.3\n"] {
            assert!(
                RuntimeIdentity::stable_release(text, "native_transcript").is_err(),
                "{text:?}"
            );
        }
    }
    #[test]
    fn runtime_ingress_rejects_descriptor_key_mismatch_and_unbounded_fields() {
        let identity = RuntimeIdentity::build(development()).unwrap();
        let mut json = serde_json::to_value(&identity).unwrap();
        json["dirty"] = true.into();
        assert!(serde_json::from_value::<RuntimeIdentity>(json).is_err());
        for field in [
            "source",
            "commit",
            "derived_version",
            "base_version",
            "release_version",
        ] {
            for text in ["x".repeat(129), "bad\nvalue".into()] {
                let mut json = serde_json::to_value(&identity).unwrap();
                json[field] = text.into();
                assert!(serde_json::from_value::<RuntimeIdentity>(json).is_err());
            }
        }
        for commit in [
            "37daf85",
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            "gggggggggggggggggggggggggggggggggggggggg",
        ] {
            let mut descriptor = development();
            descriptor.commit = Some(commit.into());
            assert!(RuntimeIdentity::build(descriptor).is_err());
        }
        let mut source_boundary = development();
        source_boundary.source = "s".repeat(32);
        assert!(RuntimeIdentity::build(source_boundary.clone()).is_ok());
        source_boundary.source.push('s');
        assert!(RuntimeIdentity::build(source_boundary).is_err());
        let mut unicode_boundary = development();
        unicode_boundary.derived_version = Some("é".repeat(64));
        assert!(RuntimeIdentity::build(unicode_boundary.clone()).is_ok());
        unicode_boundary.derived_version = Some("é".repeat(65));
        assert!(RuntimeIdentity::build(unicode_boundary).is_err());
        let mut descriptor = development();
        descriptor.distance = Some(MAX_DISTANCE + 1);
        assert!(RuntimeIdentity::build(descriptor).is_err());
    }
    #[test]
    fn absent_fields_have_explicit_nulls_in_exact_build_hash() {
        let descriptor = RuntimeDescriptor {
            release_version: None,
            source: "runtime".into(),
            base_version: None,
            derived_version: None,
            commit: None,
            dirty: None,
            distance: None,
        };
        assert_eq!(
            descriptor.canonical_json(),
            r#"{"base_version":null,"commit":null,"derived_version":null,"dirty":null,"distance":null,"release_version":null,"schema_version":1,"source":"runtime"}"#
        );
        let identity = RuntimeIdentity::build(descriptor).unwrap();
        assert_eq!(
            serde_json::from_str::<RuntimeIdentity>(&serde_json::to_string(&identity).unwrap())
                .unwrap(),
            identity
        );
    }
}
