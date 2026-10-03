//! Validated process timing shared by the store and scheduler.

use crate::protocol::summary::SummarySettings;
use crate::{
    daemon::paths::{InstancePaths, effective_uid},
    scheduler::config::SchedulerTiming,
    store::{StoreSettings, messages::MessageLimits},
};
use std::{
    fs::{self, OpenOptions},
    io::{self, Read},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
};
use uuid::Uuid;

const MAX_SETTINGS_BYTES: u64 = 4_096;
#[cfg(target_os = "macos")]
const O_NOFOLLOW: i32 = 0x0000_0100;
#[cfg(target_os = "linux")]
const O_NOFOLLOW: i32 = 0x0002_0000;

#[derive(Debug, Clone)]
pub struct ServiceConfig {
    timing: SchedulerTiming,
    minimum_wake_delay_ms: u64,
    summary: SummarySettings,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            timing: SchedulerTiming::default(),
            minimum_wake_delay_ms: crate::daemon::settings::DEFAULT_MINIMUM_WAKE_DELAY_MS,
            summary: SummarySettings::default(),
        }
    }
}

impl ServiceConfig {
    /// Load optional, private settings for one resolved host instance. A restart
    /// applies edits; the running owner keeps its validated in-memory values.
    pub fn load(paths: &InstancePaths) -> io::Result<Self> {
        let instance_dir = &paths.instance_dir;
        let instance = match fs::symlink_metadata(instance_dir) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error),
        };
        let instances = instance_dir.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "missing instances directory")
        })?;
        let state = instances.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "missing state directory")
        })?;
        // Herdr may supply a 0755 state root; only directories this plugin
        // creates below it must be exactly 0700.
        crate::daemon::paths::check_owned_state_root(state)?;
        let directories = [
            (instances, fs::symlink_metadata(instances)?),
            (instance_dir.as_path(), instance),
        ];
        for (_, metadata) in &directories {
            if !metadata.file_type().is_dir()
                || metadata.uid() != effective_uid()
                || metadata.permissions().mode() & 0o777 != 0o700
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "unsafe instance settings directory",
                ));
            }
        }
        let path = instance_dir.join("settings.json");
        let before = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error),
        };
        if !before.file_type().is_file()
            || before.uid() != effective_uid()
            || before.permissions().mode() & 0o777 != 0o600
            || before.len() > MAX_SETTINGS_BYTES
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "unsafe instance settings file",
            ));
        }
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(O_NOFOLLOW)
            .open(&path)?;
        let opened = file.metadata()?;
        let after = fs::symlink_metadata(&path)?;
        if !opened.file_type().is_file()
            || opened.uid() != effective_uid()
            || opened.permissions().mode() & 0o777 != 0o600
            || opened.len() > MAX_SETTINGS_BYTES
            || !after.file_type().is_file()
            || before.dev() != opened.dev()
            || before.ino() != opened.ino()
            || after.dev() != opened.dev()
            || after.ino() != opened.ino()
            || after.uid() != effective_uid()
            || after.permissions().mode() & 0o777 != 0o600
            || after.len() > MAX_SETTINGS_BYTES
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "instance settings file changed",
            ));
        }
        let mut bytes = Vec::new();
        file.by_ref()
            .take(MAX_SETTINGS_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_SETTINGS_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "instance settings file is too large",
            ));
        }
        let finished = file.metadata()?;
        if finished.len() != opened.len()
            || finished.mtime() != opened.mtime()
            || finished.mtime_nsec() != opened.mtime_nsec()
            || finished.ctime() != opened.ctime()
            || finished.ctime_nsec() != opened.ctime_nsec()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "instance settings changed during read",
            ));
        }
        for (path, before) in directories {
            let after = fs::symlink_metadata(path)?;
            if !after.file_type().is_dir()
                || after.uid() != effective_uid()
                || after.permissions().mode() & 0o777 != 0o700
                || after.dev() != before.dev()
                || after.ino() != before.ino()
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "instance settings directory changed",
                ));
            }
        }
        let settings = crate::daemon::settings::parse(&bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        Self::from_settings(&settings)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    /// Validate the timing keys of the one `settings.json` schema.
    pub fn from_settings(
        settings: &crate::daemon::settings::InstanceSettings,
    ) -> Result<Self, &'static str> {
        Self::new(
            settings.invitation_default_ms,
            settings.receipt_default_ms,
            settings.minimum_wake_delay_ms,
        )
        .and_then(|config| config.with_summary(settings.summary.clone()))
    }

    pub fn new(
        invitation_millis: u64,
        receipt_millis: u64,
        minimum_wake_delay_ms: u64,
    ) -> Result<Self, &'static str> {
        let timing = SchedulerTiming::new(invitation_millis, receipt_millis)?;
        if !(30_000..=i64::MAX as u64).contains(&minimum_wake_delay_ms) {
            return Err("invalid minimum wake spacing");
        }
        Ok(Self {
            timing,
            minimum_wake_delay_ms,
            summary: SummarySettings::default(),
        })
    }

    /// Replace the summary settings after validating them (spec §13).
    pub fn with_summary(mut self, summary: SummarySettings) -> Result<Self, &'static str> {
        summary.validate()?;
        self.summary = summary;
        Ok(self)
    }

    pub fn summary(&self) -> &SummarySettings {
        &self.summary
    }

    pub fn retry_config(&self) -> crate::notification::policy::RetryConfig {
        crate::notification::policy::RetryConfig::new(self.minimum_wake_delay_ms)
            .expect("validated service minimum wake spacing")
    }

    pub fn health_settings(&self) -> crate::protocol::results::HealthSettings {
        crate::protocol::results::HealthSettings {
            invitation_default_ms: self.timing.invitation.as_millis() as u64,
            receipt_default_ms: self.timing.receipt.as_millis() as u64,
            minimum_wake_delay_ms: self.minimum_wake_delay_ms,
        }
    }

    pub fn timing(&self) -> SchedulerTiming {
        self.timing
    }

    pub fn store_settings(&self, daemon_boot: Uuid) -> StoreSettings {
        StoreSettings {
            invitation_default_ms: Some(self.timing.invitation.as_millis() as u64),
            message_limits: MessageLimits {
                receipt_duration_ms: self.timing.receipt.as_millis(),
                ..MessageLimits::default()
            },
            daemon_boot: Some(daemon_boot),
            minimum_wake_delay_ms: self.minimum_wake_delay_ms,
            summary: self.summary.clone(),
        }
    }
}
