use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// Immutable directory and declared-environment selection retained across new-tab
/// handoff and retry. This protocol value owns the canonical requested profile
/// directory, bounded adapter-produced environment projection and selected cwd,
/// including shared validation of their bounds and agreement with bootstrap inputs.
/// It hides newly captured selection versus exact replay; absence preserves legacy
/// serialization and empty-env rules. It never chooses harness environment names,
/// resolves native named profiles, holds credentials, freezes configuration contents
/// or confers seat authority: adapters own profile semantics and daemon identity
/// validation owns authority. `LaunchPolicy` produces the projection; saved bootstrap
/// and derived launch inputs transport it unchanged for creation, preflight and retry.
/// Capture once before publication, retain in the durable plan and copy into its exact
/// child; no process-global state or external resources are owned. Structural errors
/// are deterministic and distinct from disappeared/non-directory selections, which
/// orchestration rechecks before unpublished create/start without canonicalizing again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchDirectoryContext {
    pub profile_dir: PathBuf,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
}

impl LaunchDirectoryContext {
    pub fn validate(&self) -> Result<(), &'static str> {
        let valid_path = |path: &Path| {
            super::frozen_absolute_path(path)
                && path
                    .to_str()
                    .is_some_and(|value| !value.chars().any(char::is_control))
        };
        if !valid_path(&self.profile_dir) || !valid_path(&self.cwd) {
            return Err("invalid retained launch directory");
        }
        if self.env.len() != 1 {
            return Err("invalid retained profile environment");
        }
        let (key, value) = self.env.first_key_value().unwrap();
        let mut bytes = key.bytes();
        if key.len() > 128
            || !bytes
                .next()
                .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
            || !bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            || Some(value.as_str()) != self.profile_dir.to_str()
        {
            return Err("invalid retained profile environment");
        }
        Ok(())
    }

    /// Pure agreement check; callers separately own filesystem and effect admission.
    pub fn validate_bootstrap(
        &self,
        cwd: &Path,
        env: &BTreeMap<String, String>,
    ) -> Result<(), &'static str> {
        self.validate()?;
        if self.cwd.as_os_str() != cwd.as_os_str() || &self.env != env {
            return Err("retained launch directory context differs from bootstrap");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> LaunchDirectoryContext {
        LaunchDirectoryContext {
            profile_dir: "/missing/profile ' with spaces".into(),
            cwd: "/missing/work \"dir\"".into(),
            env: [(
                "PROFILE_ROOT".into(),
                "/missing/profile ' with spaces".into(),
            )]
            .into(),
        }
    }

    #[test]
    fn launch_directories_legacy_launch_bytes_and_strict_retention() {
        let legacy = r#"{"target":"pane","harness":"Codex","harness_binary":null,"argv":[],"name":null,"pane_label":null}"#;
        let mut request: crate::cli::launch::LaunchRequest = serde_json::from_str(legacy).unwrap();
        assert!(request.directory_context.is_none());
        assert_eq!(serde_json::to_string(&request).unwrap(), legacy);
        request.directory_context = Some(context());
        let bytes = serde_json::to_vec(&request).unwrap();
        let restored: crate::cli::launch::LaunchRequest = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(restored.directory_context, Some(context()));
        let mut invalid = serde_json::to_value(request).unwrap();
        invalid["directory_context"]["extra"] = true.into();
        assert!(serde_json::from_value::<crate::cli::launch::LaunchRequest>(invalid).is_err());
    }

    #[test]
    fn launch_directories_validate_without_filesystem_access() {
        let value = context();
        assert_eq!(value.validate(), Ok(()));
        assert_eq!(value.validate_bootstrap(&value.cwd, &value.env), Ok(()));
        assert!(
            value
                .validate_bootstrap(std::path::Path::new("/different"), &value.env)
                .is_err()
        );
        assert!(
            value
                .validate_bootstrap(&value.cwd, &Default::default())
                .is_err()
        );
    }

    #[test]
    fn launch_directories_reject_invalid_paths() {
        for path in [
            "",
            "relative",
            "/a/../b",
            "/a/./b",
            "/a\nb",
            "/a\0b",
            &format!("/{}", "x".repeat(4096)),
        ] {
            let mut value = context();
            value.cwd = path.into();
            assert!(value.validate().is_err(), "cwd: {path:?}");
            value = context();
            value.profile_dir = path.into();
            value.env.insert("PROFILE_ROOT".into(), path.into());
            assert!(value.validate().is_err(), "profile: {path:?}");
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            let mut value = context();
            value.cwd = std::ffi::OsString::from_vec(vec![b'/', 255]).into();
            assert!(value.validate().is_err());
            value.profile_dir = value.cwd.clone();
            value.cwd = "/work".into();
            assert!(value.validate().is_err());
        }
    }

    #[test]
    fn launch_directories_bound_projection_and_require_exact_profile_value() {
        for key in ["", "1ROOT", "A-B", "A=B", "RÖÖT", "A\nB", &"A".repeat(129)] {
            let mut value = context();
            value.env = [(key.into(), value.profile_dir.to_str().unwrap().into())].into();
            assert!(value.validate().is_err(), "key: {key:?}");
        }
        let mut value = context();
        value.env.clear();
        assert!(value.validate().is_err());
        value = context();
        value
            .env
            .insert("SECOND".into(), value.profile_dir.to_str().unwrap().into());
        assert!(value.validate().is_err());
        value = context();
        value.env.insert("PROFILE_ROOT".into(), "/other".into());
        assert!(value.validate().is_err());
        let path = format!("/{}", "x".repeat(4095));
        value.profile_dir = path.clone().into();
        value.cwd = path.clone().into();
        value.env = [("A".repeat(128), path)].into();
        assert_eq!(value.validate(), Ok(()));
    }
}
