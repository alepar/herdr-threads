//! Validated process timing shared by the store and scheduler.

use crate::protocol::summary::SummarySettings;
use crate::sync::dialer::DialTemplate;
use crate::{
    daemon::paths::{NodePaths, effective_uid},
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

#[derive(Debug, Clone)]
pub struct ServiceConfig {
    timing: SchedulerTiming,
    minimum_wake_delay_ms: u64,
    wake_batch_delay_ms: u64,
    archive_after_ms: u64,
    watch_connection_cap: usize,
    summary: SummarySettings,
    dial_template: Option<DialTemplate>,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            timing: SchedulerTiming::default(),
            minimum_wake_delay_ms: crate::daemon::settings::DEFAULT_MINIMUM_WAKE_DELAY_MS,
            wake_batch_delay_ms: crate::daemon::settings::DEFAULT_WAKE_BATCH_DELAY_MS,
            archive_after_ms: crate::store::archival::DEFAULT_AFTER_MS,
            watch_connection_cap: crate::protocol::watch::MAX_WATCH_CONNECTIONS,
            summary: SummarySettings::default(),
            dial_template: None,
        }
    }
}

impl From<&ServiceConfig> for crate::sync::dialer::DialConfig {
    /// The Dialer's transport template; no fixed jitter seed.
    fn from(config: &ServiceConfig) -> Self {
        Self {
            template: config.dial_template.clone(),
            jitter_seed: None,
        }
    }
}

impl ServiceConfig {
    /// The validated `sync.dial_command`, or `None` for the ssh transport.
    pub fn dial_template(&self) -> Option<&DialTemplate> {
        self.dial_template.as_ref()
    }
    pub fn archive_after_ms(&self) -> u64 {
        self.archive_after_ms
    }
    /// The mod watch channel budget (spec D2): 64 in production, never read
    /// from settings.
    pub fn watch_connection_cap(&self) -> usize {
        self.watch_connection_cap
    }
    /// Lower the registry's watch budget so a test fills it with a few
    /// bindings instead of 64.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_watch_connection_cap(mut self, cap: usize) -> Result<Self, &'static str> {
        if cap == 0 || cap > crate::protocol::watch::MAX_WATCH_CONNECTIONS {
            return Err("invalid watch connection cap");
        }
        self.watch_connection_cap = cap;
        Ok(self)
    }
    pub fn with_archive_after(mut self, after_ms: u64) -> Result<Self, &'static str> {
        if after_ms > (i64::MAX - crate::store::archival::MAX_GAP_MS) as u64 {
            return Err("invalid archive grace");
        }
        self.archive_after_ms = after_ms;
        Ok(self)
    }

    /// Load optional, private settings for one resolved host instance. A restart
    /// applies edits; the running owner keeps its validated in-memory values.
    pub fn load(paths: &NodePaths) -> io::Result<Self> {
        let node_dir = &paths.node_dir;
        let node = match fs::symlink_metadata(node_dir) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error),
        };
        // Herdr may supply a 0755 state root; only directories this plugin
        // creates below it must be exactly 0700.
        crate::daemon::paths::check_owned_state_root(&paths.state_dir)?;
        if !node.file_type().is_dir()
            || node.uid() != effective_uid()
            || node.permissions().mode() & 0o777 != 0o700
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "unsafe node settings directory",
            ));
        }
        let directories = [(node_dir.as_path(), node)];
        let path = node_dir.join("settings.json");
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
            .custom_flags(libc::O_NOFOLLOW)
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
                    "node settings directory changed",
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
        .and_then(|config| config.with_wake_batch_delay(settings.wake_batch_delay_ms))
        .and_then(|config| config.with_summary(settings.summary.clone()))
        .and_then(|config| config.with_archive_after(settings.archive_after_ms))
        .and_then(|config| config.with_dial_command(settings.sync.dial_command.as_deref()))
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
            wake_batch_delay_ms: crate::daemon::settings::DEFAULT_WAKE_BATCH_DELAY_MS,
            archive_after_ms: crate::store::archival::DEFAULT_AFTER_MS,
            watch_connection_cap: crate::protocol::watch::MAX_WATCH_CONNECTIONS,
            summary: SummarySettings::default(),
            dial_template: None,
        })
    }

    /// Validate `sync.dial_command` into a `DialTemplate`.
    pub fn with_dial_command(mut self, command: Option<&str>) -> Result<Self, &'static str> {
        self.dial_template = match command {
            None => None,
            Some(text) => Some(DialTemplate::parse(text).map_err(|_| "invalid sync.dial_command")?),
        };
        Ok(self)
    }

    pub fn with_wake_batch_delay(mut self, delay_ms: u64) -> Result<Self, &'static str> {
        if delay_ms > i64::MAX as u64 {
            return Err("invalid wake batch delay");
        }
        self.wake_batch_delay_ms = delay_ms;
        Ok(self)
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
            wake_batch_delay_ms: self.wake_batch_delay_ms,
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
            wake_batch_delay_ms: self.wake_batch_delay_ms,
            summary: self.summary.clone(),
        }
    }
}
