//! Claude's prompt-suggestion setting ([`PROMPT_SUGGESTION_SETTING`]) as an
//! owned, backed-up edit of the user `settings.json` (ht-6jt).
//!
//! Claude Code draws a grayed-out prompt suggestion in the composer after
//! every turn (the default), and herdr-threads cannot tell it from typed text,
//! so a soft-deadline poke skips any Claude pane that shows one
//! (TRUST-POLICY A4). `setup claude` therefore offers to set the key `false`;
//! it never changes it without a yes or an explicit flag.
//!
//! The edit follows the hook installer's rules
//! ([`super::setup`]): a private manifest under `<state>/setup/` records the
//! key's previous value (absent or a JSON value) and the file's exact bytes
//! before and after the write; the file is replaced atomically after an
//! unchanged-baseline check. [`revert`] restores the key only when setup set
//! it and it still reads `false` (byte for byte when nothing else changed);
//! a value changed by hand since is left alone.
use super::{
    claude::PROMPT_SUGGESTION_SETTING,
    setup::{SetupError, config_bytes, publish_manifest, write_replacement},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{fs, path::Path};

/// The key as read from one settings file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestionState {
    /// `false`: suggestions are hidden.
    Disabled,
    /// `true`, set explicitly.
    Enabled,
    /// Absent (or no settings file): Claude's default, shown.
    Default,
    /// Present but not a boolean: Claude's reading is unknown.
    Other,
}

impl SuggestionState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Enabled => "enabled",
            Self::Default => "default (enabled)",
            Self::Other => "unrecognized value",
        }
    }

    pub fn of(value: &Value) -> Self {
        match value.get(PROMPT_SUGGESTION_SETTING) {
            None => Self::Default,
            Some(Value::Bool(false)) => Self::Disabled,
            Some(Value::Bool(true)) => Self::Enabled,
            Some(_) => Self::Other,
        }
    }
}

fn object(bytes: &[u8]) -> Result<Value, SetupError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| SetupError::Invalid)?;
    if !value.is_object() {
        return Err(SetupError::Invalid);
    }
    Ok(value)
}

/// The key's state in `config`; a missing file is Claude's default.
pub fn read_state(config: &Path) -> Result<SuggestionState, SetupError> {
    if !config.exists() && config.symlink_metadata().is_err() {
        return Ok(SuggestionState::Default);
    }
    Ok(SuggestionState::of(&object(&config_bytes(config)?)?))
}

/// What setup recorded when it set the key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuggestionManifest {
    pub version: u8,
    pub path: String,
    /// The key's value before setup set it; `None` when it was absent.
    pub previous: Option<Value>,
    /// The file's bytes before the write, restored exactly when the file
    /// still holds `written_bytes`.
    pub original_bytes: Vec<u8>,
    pub written_bytes: Vec<u8>,
}

/// Whether setup recorded setting the key (a manifest exists).
pub fn recorded(manifest_path: &Path) -> Result<Option<SuggestionManifest>, SetupError> {
    let Ok(meta) = manifest_path.symlink_metadata() else {
        return Ok(None);
    };
    if meta.file_type().is_symlink() {
        return Err(SetupError::Invalid);
    }
    // Two copies of a settings file of at most 1 MiB, as JSON byte arrays.
    if meta.len() > 16_777_216 {
        return Err(SetupError::TooLarge);
    }
    let bytes = fs::read(manifest_path).map_err(|_| SetupError::Io)?;
    let manifest: SuggestionManifest =
        serde_json::from_slice(&bytes).map_err(|_| SetupError::Invalid)?;
    if manifest.version != 1 {
        return Err(SetupError::Invalid);
    }
    Ok(Some(manifest))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisableOutcome {
    /// The key already read `false`; nothing was written or recorded.
    AlreadyDisabled,
    /// Setup set it `false` and recorded the previous value.
    Disabled,
}

/// Set the key `false` in `config` (which must exist), recording the
/// previous value in `manifest_path` first. A manifest left by an earlier
/// run keeps its original previous value.
pub fn disable(config: &Path, manifest_path: &Path) -> Result<DisableOutcome, SetupError> {
    let config_text = config.to_str().ok_or(SetupError::Invalid)?;
    let current = config_bytes(config)?;
    let mut value = object(&current)?;
    if SuggestionState::of(&value) == SuggestionState::Disabled {
        return Ok(DisableOutcome::AlreadyDisabled);
    }
    let earlier = recorded(manifest_path)?;
    if earlier
        .as_ref()
        .is_some_and(|manifest| manifest.path != config_text)
    {
        return Err(SetupError::Conflict);
    }
    let previous = value.get(PROMPT_SUGGESTION_SETTING).cloned();
    value
        .as_object_mut()
        .ok_or(SetupError::Invalid)?
        .insert(PROMPT_SUGGESTION_SETTING.into(), Value::Bool(false));
    let written = serde_json::to_vec(&value).map_err(|_| SetupError::Invalid)?;
    if written.len() > 1_048_576 {
        return Err(SetupError::TooLarge);
    }
    let manifest = SuggestionManifest {
        version: 1,
        path: config_text.into(),
        previous: match &earlier {
            Some(earlier) => earlier.previous.clone(),
            None => previous,
        },
        original_bytes: match &earlier {
            Some(earlier) => earlier.original_bytes.clone(),
            None => current.clone(),
        },
        written_bytes: written.clone(),
    };
    let manifest_bytes = serde_json::to_vec(&manifest).map_err(|_| SetupError::Invalid)?;
    if earlier.is_some() {
        write_replacement(manifest_path, &manifest_bytes, true)?;
    } else {
        publish_manifest(manifest_path, &manifest_bytes, false)?;
    }
    let published = (|| {
        if config_bytes(config)? != current {
            return Err(SetupError::Conflict);
        }
        write_replacement(config, &written, false)
    })();
    if let Err(error) = published {
        // Nothing was written: drop a record this run created, keep an earlier one.
        if earlier.is_none() {
            let _ = fs::remove_file(manifest_path);
        }
        return Err(error);
    }
    Ok(DisableOutcome::Disabled)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevertOutcome {
    /// Setup never set the key here.
    NotRecorded,
    /// The key was restored to its previous value (or removed).
    Reverted,
    /// The key no longer reads `false` (changed by hand) or the file is gone:
    /// left alone, record dropped.
    LeftChanged,
}

impl RevertOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotRecorded => "not_recorded",
            Self::Reverted => "reverted",
            Self::LeftChanged => "left_changed",
        }
    }
}

/// Undo [`disable`]: restore the previous value when the key still reads
/// `false`, then drop the record.
pub fn revert(config: &Path, manifest_path: &Path) -> Result<RevertOutcome, SetupError> {
    let Some(manifest) = recorded(manifest_path)? else {
        return Ok(RevertOutcome::NotRecorded);
    };
    if manifest.path != config.to_str().ok_or(SetupError::Invalid)? {
        return Err(SetupError::Conflict);
    }
    if config.symlink_metadata().is_err() {
        fs::remove_file(manifest_path).map_err(|_| SetupError::Io)?;
        return Ok(RevertOutcome::LeftChanged);
    }
    let current = config_bytes(config)?;
    let mut value = object(&current)?;
    if SuggestionState::of(&value) != SuggestionState::Disabled {
        fs::remove_file(manifest_path).map_err(|_| SetupError::Io)?;
        return Ok(RevertOutcome::LeftChanged);
    }
    let restored = if current == manifest.written_bytes {
        manifest.original_bytes.clone()
    } else {
        let map = value.as_object_mut().ok_or(SetupError::Invalid)?;
        match &manifest.previous {
            None => {
                map.remove(PROMPT_SUGGESTION_SETTING);
            }
            Some(previous) => {
                map.insert(PROMPT_SUGGESTION_SETTING.into(), previous.clone());
            }
        }
        serde_json::to_vec(&value).map_err(|_| SetupError::Invalid)?
    };
    if config_bytes(config)? != current {
        return Err(SetupError::Conflict);
    }
    write_replacement(config, &restored, false)?;
    fs::remove_file(manifest_path).map_err(|_| SetupError::Io)?;
    Ok(RevertOutcome::Reverted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Dir(PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn dir() -> Dir {
        let path = std::env::temp_dir().join(format!("prompt-suggestion-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        Dir(path)
    }

    #[test]
    fn states() {
        let of = |text: &str| SuggestionState::of(&serde_json::from_str(text).unwrap());
        assert_eq!(of("{}"), SuggestionState::Default);
        assert_eq!(
            of(r#"{"promptSuggestionEnabled":false}"#),
            SuggestionState::Disabled
        );
        assert_eq!(
            of(r#"{"promptSuggestionEnabled":true}"#),
            SuggestionState::Enabled
        );
        assert_eq!(
            of(r#"{"promptSuggestionEnabled":"no"}"#),
            SuggestionState::Other
        );
        let d = dir();
        assert_eq!(
            read_state(&d.0.join("settings.json")).unwrap(),
            SuggestionState::Default
        );
    }

    #[test]
    fn disable_then_revert_restores_bytes() {
        let d = dir();
        let (config, manifest) = (d.0.join("settings.json"), d.0.join("m.json"));
        let original = b"{\n  \"model\": \"x\"\n}\n";
        fs::write(&config, original).unwrap();
        assert_eq!(
            disable(&config, &manifest).unwrap(),
            DisableOutcome::Disabled
        );
        assert_eq!(read_state(&config).unwrap(), SuggestionState::Disabled);
        // Idempotent: already false, nothing recorded again.
        assert_eq!(
            disable(&config, &manifest).unwrap(),
            DisableOutcome::AlreadyDisabled
        );
        assert_eq!(revert(&config, &manifest).unwrap(), RevertOutcome::Reverted);
        assert_eq!(fs::read(&config).unwrap(), original);
        assert!(!manifest.exists());
        assert_eq!(
            revert(&config, &manifest).unwrap(),
            RevertOutcome::NotRecorded
        );
    }

    #[test]
    fn revert_restores_the_previous_value_beside_other_edits() {
        let d = dir();
        let (config, manifest) = (d.0.join("settings.json"), d.0.join("m.json"));
        fs::write(&config, br#"{"promptSuggestionEnabled":true}"#).unwrap();
        disable(&config, &manifest).unwrap();
        // Another key changed after setup: the key is restored structurally.
        fs::write(&config, br#"{"promptSuggestionEnabled":false,"model":"y"}"#).unwrap();
        assert_eq!(revert(&config, &manifest).unwrap(), RevertOutcome::Reverted);
        let value: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"promptSuggestionEnabled": true, "model": "y"})
        );
    }

    #[test]
    fn revert_leaves_a_hand_changed_value() {
        let d = dir();
        let (config, manifest) = (d.0.join("settings.json"), d.0.join("m.json"));
        fs::write(&config, b"{}").unwrap();
        disable(&config, &manifest).unwrap();
        fs::write(&config, br#"{"promptSuggestionEnabled":true}"#).unwrap();
        assert_eq!(
            revert(&config, &manifest).unwrap(),
            RevertOutcome::LeftChanged
        );
        assert_eq!(
            fs::read(&config).unwrap(),
            br#"{"promptSuggestionEnabled":true}"#
        );
        assert!(!manifest.exists());
    }

    #[test]
    fn already_false_is_never_recorded() {
        let d = dir();
        let (config, manifest) = (d.0.join("settings.json"), d.0.join("m.json"));
        fs::write(&config, br#"{"promptSuggestionEnabled":false}"#).unwrap();
        assert_eq!(
            disable(&config, &manifest).unwrap(),
            DisableOutcome::AlreadyDisabled
        );
        assert!(!manifest.exists());
        assert_eq!(
            revert(&config, &manifest).unwrap(),
            RevertOutcome::NotRecorded
        );
        assert_eq!(
            fs::read(&config).unwrap(),
            br#"{"promptSuggestionEnabled":false}"#
        );
    }
}
