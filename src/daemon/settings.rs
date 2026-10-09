//! Per-instance operator settings: `<instance_dir>/settings.json`.
//!
//! Read once at daemon start (and by `doctor`). This is the single schema for
//! the file: the daemon's hardened read (`ServiceConfig::load`), the elected
//! daemon (`run_elected`) and `doctor` all parse it through [`parse`]. The
//! file rejects unknown keys (`deny_unknown_fields`), so a downgrade to a
//! release that does not know a key fails to start until the key is removed;
//! the error names the file.
//!
//! Keys (all optional):
//! - `"harness_manifest": "auto" | "off"` (default `auto`): `off` stops the
//!   daemon fetching the harness version manifest; the embedded copy is used.
//! - `"invitation_default_ms"` (default 300000) and `"receipt_default_ms"`
//!   (default 300000): default invitation and receipt deadlines.
//! - `"minimum_wake_delay_ms"` (default 30000, at least 30000): minimum
//!   spacing between wake attempts.
//! - `"wake_batch_delay_ms"` (default 0, zero disables): initial
//!   ordinary attention batching, separate from retry spacing.
//! - `"auto_archive_after_ms"` (default 3600000, zero disables): quiet-channel grace.
//! - `"mod_delivery": "on" | "off"` (default `on`): `off` refuses mod watch
//!   registrations and closes live channels (`Close{disabled}`); delivery stays
//!   on hooks plus native wake. Read by ht-j16.2.
//! - `"summary"`: thread summary, catch-up and soft-deadline poke settings
//!   (`crate::protocol::summary::SummarySettings`; every key optional,
//!   unknown keys rejected, validated by `ServiceConfig::from_settings`).
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// The settings file's name inside the instance directory.
pub const SETTINGS_FILE: &str = "settings.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HarnessManifestSetting {
    #[default]
    Auto,
    Off,
}

pub const DEFAULT_INVITATION_MS: u64 = 300_000;
pub const DEFAULT_RECEIPT_MS: u64 = 300_000;
pub const DEFAULT_MINIMUM_WAKE_DELAY_MS: u64 = 30_000;
pub const DEFAULT_WAKE_BATCH_DELAY_MS: u64 = 0;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct InstanceSettings {
    pub harness_manifest: HarnessManifestSetting,
    pub invitation_default_ms: u64,
    pub receipt_default_ms: u64,
    pub minimum_wake_delay_ms: u64,
    pub wake_batch_delay_ms: u64,
    #[serde(rename = "auto_archive_after_ms")]
    pub archive_after_ms: u64,
    pub mod_delivery: crate::protocol::watch::ModDeliverySetting,
    pub summary: crate::protocol::summary::SummarySettings,
}

impl Default for InstanceSettings {
    fn default() -> Self {
        Self {
            harness_manifest: HarnessManifestSetting::Auto,
            invitation_default_ms: DEFAULT_INVITATION_MS,
            receipt_default_ms: DEFAULT_RECEIPT_MS,
            minimum_wake_delay_ms: DEFAULT_MINIMUM_WAKE_DELAY_MS,
            wake_batch_delay_ms: DEFAULT_WAKE_BATCH_DELAY_MS,
            archive_after_ms: crate::store::archival::DEFAULT_AFTER_MS,
            mod_delivery: crate::protocol::watch::ModDeliverySetting::default(),
            summary: crate::protocol::summary::SummarySettings::default(),
        }
    }
}

/// Parse settings.json bytes against the one schema (no validation of values).
pub fn parse(bytes: &[u8]) -> Result<InstanceSettings, serde_json::Error> {
    serde_json::from_slice(bytes)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsError {
    pub path: PathBuf,
    pub detail: String,
}

impl std::fmt::Display for SettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: {} (fix or remove the file; unknown keys are rejected)",
            self.path.display(),
            self.detail
        )
    }
}

impl std::error::Error for SettingsError {}

/// Load `<instance_dir>/settings.json`. A missing file gives the defaults;
/// an unreadable file, invalid JSON, an unknown key or a bad value (including
/// an out-of-range timing value) is an error naming the file and the
/// complaint. A plain read: the permission checks live in
/// `ServiceConfig::load`.
pub fn load(instance_dir: &Path) -> Result<InstanceSettings, SettingsError> {
    let path = instance_dir.join(SETTINGS_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(InstanceSettings::default());
        }
        Err(error) => {
            return Err(SettingsError {
                path,
                detail: format!("unreadable: {error}"),
            });
        }
    };
    let settings = parse(text.as_bytes()).map_err(|error| SettingsError {
        path: path.clone(),
        detail: error.to_string(),
    })?;
    crate::service::config::ServiceConfig::from_settings(&settings).map_err(|text| {
        SettingsError {
            path,
            detail: text.to_owned(),
        }
    })?;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir_with(contents: Option<&str>) -> tempdir::Dir {
        tempdir::Dir::new(contents)
    }

    /// A throwaway directory under the process temp dir, removed on drop.
    mod tempdir {
        use std::path::{Path, PathBuf};
        pub struct Dir(PathBuf);
        impl Dir {
            pub fn new(contents: Option<&str>) -> Self {
                let path = std::env::temp_dir().join(format!(
                    "herdr-threads-settings-{}-{}",
                    std::process::id(),
                    uuid::Uuid::new_v4()
                ));
                std::fs::create_dir_all(&path).unwrap();
                if let Some(contents) = contents {
                    std::fs::write(path.join(super::super::SETTINGS_FILE), contents).unwrap();
                }
                Self(path)
            }
            pub fn path(&self) -> &Path {
                &self.0
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    #[test]
    fn wake_batch_delay_is_separate_and_zero_disables() {
        let defaults = load(dir_with(Some("{}")).path()).unwrap();
        assert_eq!(defaults.wake_batch_delay_ms, 0);
        assert_eq!(load(dir_with(None).path()).unwrap().wake_batch_delay_ms, 0);
        let batched = load(dir_with(Some(r#"{"wake_batch_delay_ms":15000}"#)).path()).unwrap();
        assert_eq!(batched.wake_batch_delay_ms, 15_000);
        let settings = load(dir_with(Some(r#"{"wake_batch_delay_ms":0}"#)).path()).unwrap();
        assert_eq!(settings.wake_batch_delay_ms, 0);
        assert_eq!(settings.minimum_wake_delay_ms, 30_000);
        let bad = dir_with(Some(r#"{"wake_batch_delay_ms":18446744073709551615}"#));
        assert!(
            load(bad.path())
                .unwrap_err()
                .detail
                .contains("invalid wake batch delay")
        );
    }

    #[test]
    fn mod_delivery_defaults_on_and_parses_off() {
        use crate::protocol::watch::ModDeliverySetting;
        let on = load(dir_with(Some("{}")).path()).unwrap();
        assert_eq!(on.mod_delivery, ModDeliverySetting::On);
        let off = load(dir_with(Some(r#"{"mod_delivery":"off"}"#)).path()).unwrap();
        assert_eq!(off.mod_delivery, ModDeliverySetting::Off);
        let bad = dir_with(Some(r#"{"mod_delivery":"maybe"}"#));
        let error = load(bad.path()).unwrap_err();
        assert!(error.to_string().contains(SETTINGS_FILE), "{error}");
    }

    #[test]
    fn missing_file_is_auto() {
        let dir = dir_with(None);
        assert_eq!(load(dir.path()).unwrap(), InstanceSettings::default());
        assert_eq!(
            InstanceSettings::default().harness_manifest,
            HarnessManifestSetting::Auto
        );
    }

    #[test]
    fn off_and_auto_parse() {
        let off = dir_with(Some(r#"{"harness_manifest":"off"}"#));
        assert_eq!(
            load(off.path()).unwrap().harness_manifest,
            HarnessManifestSetting::Off
        );
        let auto = dir_with(Some(r#"{"harness_manifest":"auto"}"#));
        assert_eq!(
            load(auto.path()).unwrap().harness_manifest,
            HarnessManifestSetting::Auto
        );
        let empty = dir_with(Some("{}"));
        assert_eq!(
            load(empty.path()).unwrap().harness_manifest,
            HarnessManifestSetting::Auto
        );
    }

    #[test]
    fn bad_value_unknown_key_and_invalid_json_name_the_file() {
        for bad in [
            r#"{"harness_manifest":"maybe"}"#,
            r#"{"other":1}"#,
            r#"{"harness_manifest":"off","other":1}"#,
            "not json",
        ] {
            let dir = dir_with(Some(bad));
            let error = load(dir.path()).unwrap_err();
            let text = error.to_string();
            assert!(text.contains(SETTINGS_FILE), "{bad}: {text}");
            assert!(text.contains(&dir.path().display().to_string()), "{text}");
        }
        let dir = dir_with(Some(r#"{"other":1}"#));
        assert!(
            load(dir.path())
                .unwrap_err()
                .detail
                .contains("unknown field `other`")
        );
    }

    #[test]
    fn timing_only_file_loads_with_auto_manifest() {
        let dir = dir_with(Some(
            r#"{"invitation_default_ms":120000,"receipt_default_ms":240000,"minimum_wake_delay_ms":45000}"#,
        ));
        let settings = load(dir.path()).unwrap();
        assert_eq!(settings.harness_manifest, HarnessManifestSetting::Auto);
        assert_eq!(settings.invitation_default_ms, 120_000);
        assert_eq!(settings.receipt_default_ms, 240_000);
        assert_eq!(settings.minimum_wake_delay_ms, 45_000);
    }

    #[test]
    fn mixed_file_loads_both_directions() {
        let dir = dir_with(Some(
            r#"{"harness_manifest":"off","minimum_wake_delay_ms":45000}"#,
        ));
        let settings = load(dir.path()).unwrap();
        assert_eq!(settings.harness_manifest, HarnessManifestSetting::Off);
        assert_eq!(settings.minimum_wake_delay_ms, 45_000);
        assert_eq!(settings.invitation_default_ms, DEFAULT_INVITATION_MS);
        let dir = dir_with(Some(
            r#"{"receipt_default_ms":240000,"harness_manifest":"auto"}"#,
        ));
        let settings = load(dir.path()).unwrap();
        assert_eq!(settings.harness_manifest, HarnessManifestSetting::Auto);
        assert_eq!(settings.receipt_default_ms, 240_000);
        assert_eq!(
            settings.minimum_wake_delay_ms,
            DEFAULT_MINIMUM_WAKE_DELAY_MS
        );
    }

    #[test]
    fn invalid_timing_value_is_a_settings_error() {
        let dir = dir_with(Some(r#"{"minimum_wake_delay_ms":1}"#));
        let text = load(dir.path()).unwrap_err().to_string();
        assert!(text.contains(SETTINGS_FILE), "{text}");
        assert!(text.contains("invalid minimum wake spacing"), "{text}");
    }
}

#[cfg(test)]
mod archival_tests {
    #[test]
    fn archival_setting_defaults_to_one_hour_zero_disables_and_overflow_refuses() {
        let defaults = super::parse(b"{}").unwrap();
        assert_eq!(
            crate::service::config::ServiceConfig::from_settings(&defaults)
                .unwrap()
                .archive_after_ms(),
            3_600_000
        );
        let off = super::parse(br#"{"auto_archive_after_ms":0}"#).unwrap();
        assert_eq!(
            crate::service::config::ServiceConfig::from_settings(&off)
                .unwrap()
                .archive_after_ms(),
            0
        );
        let invalid = super::parse(br#"{"auto_archive_after_ms":18446744073709551615}"#).unwrap();
        assert!(crate::service::config::ServiceConfig::from_settings(&invalid).is_err());
        assert!(super::parse(br#"{"auto_archive_after_ms":-1}"#).is_err());
    }
}
