//! Per-instance operator settings: `<instance_dir>/settings.json`.
//!
//! Read once at daemon start (and by `doctor`). The file rejects unknown keys
//! (`deny_unknown_fields`), so a downgrade to a release that does not know a
//! key fails to start until the key is removed; the error names the file.
//!
//! Keys:
//! - `"harness_manifest": "auto" | "off"` (default `auto`): `off` stops the
//!   daemon fetching the harness version manifest; the embedded copy is used.
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

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceSettings {
    #[serde(default)]
    pub harness_manifest: HarnessManifestSetting,
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
/// an unreadable file, invalid JSON, an unknown key or a bad value is an
/// error naming the file and the parser's complaint.
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
    serde_json::from_str(&text).map_err(|error| SettingsError {
        path,
        detail: error.to_string(),
    })
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
}
